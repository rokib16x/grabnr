//! The download itself: probe, plan chunks, run workers on every link, resume.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, IF_RANGE, RANGE};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::adapt::{is_slow_tail, Ramp};
use crate::auth::Auth;
use crate::bind::{client_via_proxy, BindMode};
use crate::checksum::{self, Checksum};
use crate::disk;
use crate::error::{Error, Result};
use crate::interfaces::Link;
use crate::limiter::Limiter;
use crate::probe::{probe, sanitize, Probe};
use crate::scheduler::{chunk_size_for, plan, Lease, Scheduler};
use crate::store::{DoneChunk, PartialChunk, Record, Store};
use crate::writer::OffsetFile;

/// A path a worker can take to the internet: a bound link, or the OS default.
#[derive(Clone)]
pub struct Route {
    pub name: String,
    pub link: Option<Link>,
    pub mode: BindMode,
    /// Cap for this link in bytes per second, shared by all its connections.
    pub speed_limit: Option<u64>,
    /// Most connections on this link; falls back to `Options::conns_per_route`. Lets a weak link carry a smaller share.
    pub max_conns: Option<usize>,
}

impl Route {
    pub fn from_link(link: &Link) -> Self {
        Route { name: link.name.clone(), link: Some(link.clone()), mode: BindMode::BoundIf, speed_limit: None, max_conns: None }
    }

    /// Same link, same address: used to notice a link that came back with a different IP.
    pub fn key(&self) -> String {
        match &self.link {
            Some(l) => format!("{}@{}", l.name, l.ipv4),
            None => self.name.clone(),
        }
    }

    /// Let the OS routing table decide (used for single-link downloads and tests).
    pub fn unbound(name: &str) -> Self {
        Route { name: name.into(), link: None, mode: BindMode::None, speed_limit: None, max_conns: None }
    }
}

/// Reports the links that should be in use right now. Called every `watch_interval` during a download.
pub type LinkWatch = Arc<dyn Fn() -> Vec<Route> + Send + Sync>;

pub struct Options {
    pub url: String,
    pub dest_dir: PathBuf,
    /// Overrides the name from `Content-Disposition` / the URL.
    pub filename: Option<String>,
    pub routes: Vec<Route>,
    pub conns_per_route: usize,
    pub headers: Vec<(String, String)>,
    /// Without a store nothing is resumable.
    pub store: Option<Arc<Store>>,
    pub max_attempts: u32,
    /// Cap for the whole download in bytes per second.
    pub speed_limit: Option<u64>,
    /// A limiter shared with other downloads (and adjustable while running); used instead of `speed_limit` when set.
    pub shared_limit: Option<Arc<Limiter>>,
    /// Verified after the last chunk; a mismatch fails the download and discards the partial file.
    pub checksum: Option<Checksum>,
    /// Start each link with a few connections and add more while that pays off, up to `conns_per_route`.
    pub adaptive: bool,
    /// How often the adaptive controller looks at link speeds.
    pub ramp_interval: Duration,
    /// HTTP(S) or SOCKS5 proxy for every link, e.g. `socks5://127.0.0.1:1080`.
    pub proxy: Option<String>,
    /// Sent as the `Authorization` header (overrides one in `headers`).
    pub auth: Option<Auth>,
    /// When set, links that appear join the running download and links that vanish drain out.
    pub link_watch: Option<LinkWatch>,
    pub watch_interval: Duration,
    /// A connection that receives nothing for this long is dropped and the chunk retried (sleep/wake, dead Wi-Fi).
    pub stall_timeout: Duration,
    /// A link whose connections all gave up is tried again after this long.
    pub revive_after: Duration,
    /// With no working link at all, wait this long for one to come back before failing.
    pub no_link_grace: Duration,
    /// Stable name for the resume state. Without it the link and file name are used, so changing the link starts over;
    /// with it a replaced link (for example a new signed URL) can carry on with the same partial file.
    pub resume_key: Option<String>,
    /// Which stream of an HLS master playlist to download.
    pub hls_quality: crate::hls::Quality,
    /// ffmpeg to turn a downloaded HLS transport stream into an MP4 without re-encoding; none leaves a `.ts`.
    pub ffmpeg: Option<PathBuf>,
    /// Other URLs for the same file. Mirrors that do not report the same size (or lack range support) are ignored.
    pub mirrors: Vec<String>,
}

impl Options {
    pub fn new(url: impl Into<String>, dest_dir: impl Into<PathBuf>, routes: Vec<Route>) -> Self {
        Options {
            url: url.into(),
            dest_dir: dest_dir.into(),
            filename: None,
            routes,
            conns_per_route: 8,
            headers: Vec::new(),
            store: None,
            max_attempts: 6,
            speed_limit: None,
            shared_limit: None,
            checksum: None,
            adaptive: true,
            ramp_interval: Duration::from_secs(2),
            proxy: None,
            auth: None,
            link_watch: None,
            watch_interval: Duration::from_secs(3),
            stall_timeout: Duration::from_secs(30),
            revive_after: Duration::from_secs(10),
            no_link_grace: Duration::from_secs(30),
            mirrors: Vec::new(),
            resume_key: None,
            hls_quality: crate::hls::Quality::Best,
            ffmpeg: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RouteStat {
    pub name: String,
    /// Bytes this link put on the wire (includes raced duplicates).
    pub bytes: u64,
    pub bytes_per_sec: f64,
    pub connections: usize,
    /// The link is gone (unplugged, or its connections keep failing) and may come back.
    pub down: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub downloaded: u64,
    pub total: Option<u64>,
    pub bytes_per_sec: f64,
    pub routes: Vec<RouteStat>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Started {
        filename: String,
        total: Option<u64>,
        chunks: usize,
        ranges: bool,
        resumed_chunks: usize,
    },
    ResumeDiscarded {
        reason: String,
    },
    /// Finished pieces from an earlier run were re-read and checked; `redo` of them were damaged or unverifiable.
    ResumeChecked {
        checked: usize,
        redo: usize,
    },
    Progress(Snapshot),
    /// Which link fetched which chunk (drives the chunk grid).
    ChunkDone {
        idx: usize,
        route: usize,
    },
    RouteDown {
        route: usize,
        reason: String,
    },
    /// A link joined (or came back to) the running download.
    RouteUp {
        route: usize,
        name: String,
    },
    Finished {
        path: String,
    },
}

pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

const LINK_FAIL_LIMIT: u32 = 5;
/// How much a fetch writes before it saves its progress inside the chunk.
const CHECKPOINT: u64 = 1 << 20;

pub(crate) struct RouteState {
    name: String,
    /// Most connections this link may open.
    max: usize,
    pub(crate) bytes: AtomicU64,
    allowed: AtomicUsize,
    /// Most connections this link may use; lowered when the server throttles.
    ceiling: AtomicUsize,
    /// Smoothed bytes/s, written by the progress ticker.
    speed: AtomicU64,
    pub(crate) active: AtomicUsize,
    last_throttle: Mutex<Instant>,
    pub(crate) limit: Option<Limiter>,
    /// Running worker tasks for this link.
    workers: AtomicUsize,
    last_exit: Mutex<Instant>,
    /// Cancelled to make this link's workers drain; replaced when the link is restarted.
    pub(crate) stop: CancellationToken,
    down: std::sync::atomic::AtomicBool,
}

impl RouteState {
    fn new(r: &Route, opts: &Options, bytes: u64, stop: CancellationToken) -> Self {
        let max = max_conns(opts, r);
        RouteState {
            name: r.name.clone(),
            max,
            bytes: AtomicU64::new(bytes),
            allowed: AtomicUsize::new(Ramp::new(max, opts.adaptive).allowed),
            ceiling: AtomicUsize::new(max),
            speed: AtomicU64::new(0),
            active: AtomicUsize::new(0),
            last_throttle: Mutex::new(Instant::now() - Duration::from_secs(60)),
            limit: r.speed_limit.map(Limiter::new),
            workers: AtomicUsize::new(0),
            last_exit: Mutex::new(Instant::now()),
            stop,
            down: std::sync::atomic::AtomicBool::new(false),
        }
    }

    fn is_down(&self) -> bool {
        self.down.load(Ordering::Relaxed)
    }
}

/// Counts a worker as running until dropped, so the supervisor knows when a link has no workers left.
struct WorkerGuard(Arc<RouteState>);
impl Drop for WorkerGuard {
    fn drop(&mut self) {
        *self.0.last_exit.lock().unwrap() = Instant::now();
        self.0.workers.fetch_sub(1, Ordering::Relaxed);
    }
}

pub(crate) struct Shared {
    /// Size of the whole download in bytes; 0 while unknown (HLS refines an estimate as segments arrive).
    pub(crate) total: AtomicU64,
    /// Bytes safely on disk: finished chunks plus the saved part of unfinished ones.
    pub(crate) settled: AtomicU64,
    /// Bytes written by fetches still in progress, not yet settled.
    pub(crate) inflight: AtomicU64,
    /// Append-only: a link keeps its index for the whole download, so chunk and stat indices stay valid.
    routes: RwLock<Vec<Arc<RouteState>>>,
    pub(crate) limit: Option<Arc<Limiter>>,
}

impl Shared {
    pub(crate) fn new(total: Option<u64>, opts: &Options, stop: &CancellationToken) -> Self {
        Shared {
            total: AtomicU64::new(total.unwrap_or(0)),
            settled: AtomicU64::new(0),
            inflight: AtomicU64::new(0),
            routes: RwLock::new(opts.routes.iter().map(|r| Arc::new(RouteState::new(r, opts, 0, stop.child_token()))).collect()),
            limit: opts.shared_limit.clone().or_else(|| opts.speed_limit.map(|r| Arc::new(Limiter::new(r)))),
        }
    }

    pub(crate) fn route(&self, i: usize) -> Arc<RouteState> {
        self.routes.read().unwrap()[i].clone()
    }

    pub(crate) fn all(&self) -> Vec<Arc<RouteState>> {
        self.routes.read().unwrap().clone()
    }
}

struct Ctx {
    /// The file's URLs, primary first. Workers spread over them and move on when one fails.
    urls: Vec<String>,
    /// Request headers per URL (same order as `urls`): credentials only go to the host the user gave.
    headers: Vec<HeaderMap>,
    validator: Option<String>,
    sched: Scheduler,
    file: OffsetFile,
    store: Option<Arc<Store>>,
    id: String,
    shared: Arc<Shared>,
    abort: CancellationToken,
    fatal: Mutex<Option<Error>>,
    last_err: Mutex<String>,
    emit: Emit,
    max_attempts: u32,
    stall_timeout: Duration,
}

impl Ctx {
    fn fail(&self, e: Error) {
        let mut f = self.fatal.lock().unwrap();
        if f.is_none() {
            *f = Some(e);
        }
        self.abort.cancel();
    }
}

enum FetchErr {
    Cancelled,
    Fatal(Error),
    Throttled(u16),
    Retry(String),
}

enum Outcome {
    Won,
    Lost,
}

pub async fn download(mut opts: Options, cancel: CancellationToken, emit: Emit) -> Result<PathBuf> {
    if opts.routes.is_empty() {
        return Err(Error::Other("no network routes selected".into()));
    }
    if let Some(a) = &opts.auth {
        opts.headers.retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
        opts.headers.push(("Authorization".into(), a.header_value()));
    }
    // A playlist is not one file but many small ones: it has its own path through the engine.
    if crate::hls::looks_like(&opts.url) {
        let path = crate::hls::download(&opts, cancel, emit.clone()).await?;
        emit(Event::Finished { path: path.display().to_string() });
        return Ok(path);
    }
    let mut headers = HeaderMap::new();
    for (k, v) in &opts.headers {
        let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| Error::Other(format!("bad header {k}: {e}")))?;
        let val = HeaderValue::from_str(v).map_err(|e| Error::Other(format!("bad header value for {k}: {e}")))?;
        headers.insert(name, val);
    }
    let clients: Vec<Client> = opts
        .routes
        .iter()
        .map(|r| client_via_proxy(r.link.as_ref(), r.mode, opts.proxy.as_deref()))
        .collect::<std::result::Result<_, _>>()?;

    // Probe on the first route that answers; a dead link must not block the download.
    let mut probed: Option<(usize, Probe)> = None;
    let mut last: Option<Error> = None;
    for (i, c) in clients.iter().enumerate() {
        match probe(c, &opts.url, &headers).await {
            Ok(p) => {
                probed = Some((i, p));
                break;
            }
            Err(e) => last = Some(e),
        }
    }
    let Some((probe_route, probe)) = probed else {
        return Err(last.unwrap_or_else(|| Error::Other("probe failed".into())));
    };

    std::fs::create_dir_all(&opts.dest_dir)?;
    let filename = sanitize(opts.filename.as_deref().unwrap_or(&probe.filename));
    let staging = opts.dest_dir.join(format!("{filename}.grabnr"));

    let shared = Arc::new(Shared::new(probe.total, &opts, &cancel));

    let final_path = match (probe.ranges, probe.total) {
        (true, Some(total)) => ranged(&opts, &probe, clients, &filename, &staging, total, shared, cancel, emit.clone()).await?,
        _ => single(&opts, &probe, &clients[probe_route], &filename, &staging, shared, cancel, emit.clone()).await?,
    };
    emit(Event::Finished { path: final_path.display().to_string() });
    Ok(final_path)
}

#[allow(clippy::too_many_arguments)]
async fn ranged(
    opts: &Options,
    probe: &Probe,
    clients: Vec<Client>,
    filename: &str,
    staging: &Path,
    total: u64,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    emit: Emit,
) -> Result<PathBuf> {
    let id = opts.resume_key.clone().unwrap_or_else(|| format!("{}|{}", opts.url, staging.display()));
    let mut chunk_size = chunk_size_for(total, opts.routes.len());
    let mut done_raw: Vec<DoneChunk> = Vec::new();
    let mut partial_raw: Vec<PartialChunk> = Vec::new();

    let fresh = Record {
        id: id.clone(),
        url: opts.url.clone(),
        total,
        etag: probe.etag.clone(),
        last_modified: probe.last_modified.clone(),
        chunk_size,
    };
    match opts.store.as_ref().map(|s| s.load(&id)).transpose()?.flatten() {
        Some((rec, d))
            if rec.total == total
                && rec.etag == fresh.etag
                && rec.last_modified == fresh.last_modified
                && std::fs::metadata(staging).map(|m| m.len() == total).unwrap_or(false) =>
        {
            chunk_size = rec.chunk_size;
            done_raw = d.done;
            partial_raw = d.partial;
        }
        prev => {
            if prev.is_some() {
                emit(Event::ResumeDiscarded { reason: "the file on the server changed, or the partial file is gone".into() });
            }
            let _ = std::fs::remove_file(staging);
            if let Some(s) = &opts.store {
                s.begin(&Record { chunk_size, ..fresh })?;
            }
        }
    }

    let ranges = plan(total, chunk_size);
    // Trust nothing from the last run: re-read every finished piece and compare it with the checksum saved when it
    // completed. This catches a crash or power loss that left a piece marked done but never written, and any edit
    // to the partial file. Damaged pieces are simply downloaded again.
    let (done, partials): (Vec<usize>, Vec<(usize, u64, u32)>) = if done_raw.is_empty() && partial_raw.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        let (path, rs, raw_done, raw_part) = (staging.to_path_buf(), ranges.clone(), done_raw.clone(), partial_raw.clone());
        let checked = raw_done.len() + raw_part.len();
        let ((good, bad), (part_ok, part_bad)) =
            tokio::task::spawn_blocking(move || (verify_done(&path, &rs, &raw_done), verify_partial(&path, &rs, &raw_part)))
                .await
                .map_err(|e| Error::Other(e.to_string()))?;
        if let Some(s) = &opts.store {
            s.forget(&id, &bad)?;
            s.forget_partial(&id, &part_bad)?;
        }
        emit(Event::ResumeChecked { checked, redo: bad.len() + part_bad.len() });
        (good, part_ok)
    };
    let sched = Scheduler::new(ranges.clone(), &done).with_have(&partials);
    let resumed_bytes = sched.settled();
    shared.settled.store(resumed_bytes, Ordering::Relaxed);
    disk::ensure_space(&opts.dest_dir, total.saturating_sub(resumed_bytes))?;
    let file = OffsetFile::open_staging(staging, total)?;

    emit(Event::Started { filename: filename.into(), total: Some(total), chunks: ranges.len(), ranges: true, resumed_chunks: done.len() });

    let urls = usable_urls(opts, probe, &clients).await;
    let ctx = Arc::new(Ctx {
        headers: urls.iter().map(|u| headers_for(&opts.headers, &opts.url, u)).collect(),
        urls,
        validator: probe.validator().map(str::to_owned),
        sched,
        file,
        store: opts.store.clone(),
        id: id.clone(),
        shared: shared.clone(),
        abort: cancel.child_token(),
        fatal: Mutex::new(None),
        last_err: Mutex::new(String::new()),
        emit: emit.clone(),
        max_attempts: opts.max_attempts,
        stall_timeout: opts.stall_timeout,
    });

    let ticker = spawn_ticker(shared.clone(), emit.clone());
    let ramp = opts.adaptive.then(|| spawn_ramp(shared.clone(), opts.ramp_interval));
    supervise(&ctx, opts, clients).await;
    ticker.abort();
    if let Some(r) = ramp {
        r.abort();
    }
    let _ = ctx.file.sync();

    if let Some(e) = ctx.fatal.lock().unwrap().take() {
        return Err(e);
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if !ctx.sched.finished() {
        return Err(Error::AllFailed(ctx.last_err.lock().unwrap().clone()));
    }

    verify_checksum(opts, staging, ctx.store.as_deref().map(|s| (s, id.as_str())))?;
    let dest = unique_path(&opts.dest_dir, filename);
    std::fs::rename(staging, &dest)?;
    if let Some(s) = &opts.store {
        s.delete(&id)?;
    }
    Ok(dest)
}

/// Server without range support (or unknown size): one stream, no resume.
#[allow(clippy::too_many_arguments)]
async fn single(
    opts: &Options,
    probe: &Probe,
    client: &Client,
    filename: &str,
    staging: &Path,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    emit: Emit,
) -> Result<PathBuf> {
    if let Some(t) = probe.total {
        disk::ensure_space(&opts.dest_dir, t)?;
    }
    emit(Event::Started { filename: filename.into(), total: probe.total, chunks: 1, ranges: false, resumed_chunks: 0 });
    let resp = client.get(&probe.final_url).headers(headers_for(&opts.headers, &opts.url, &probe.final_url)).send().await?;
    if !resp.status().is_success() {
        return Err(Error::Status(resp.status().as_u16()));
    }
    let _ = std::fs::remove_file(staging);
    let file = OffsetFile::open_staging(staging, 0)?;
    let ticker = spawn_ticker(shared.clone(), emit);
    let mut stream = resp.bytes_stream();
    let mut pos = 0u64;
    let result: Result<()> = async {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Err(Error::Cancelled),
                next = stream.next() => match next {
                    Some(Ok(b)) => {
                        if let Some(l) = &shared.limit { l.acquire(b.len() as u64).await; }
                        if let Some(l) = &shared.route(0).limit {
                            l.acquire(b.len() as u64).await;
                        }
                        file.write_all_at(&b, pos)?;
                        pos += b.len() as u64;
                        shared.settled.fetch_add(b.len() as u64, Ordering::Relaxed);
                        shared.route(0).bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
                    }
                    Some(Err(e)) => return Err(e.into()),
                    None => return Ok(()),
                },
            }
        }
    }
    .await;
    ticker.abort();
    result?;
    file.sync()?;
    verify_checksum(opts, staging, None)?;
    let dest = unique_path(&opts.dest_dir, filename);
    std::fs::rename(staging, &dest)?;
    Ok(dest)
}

/// Runs the download: starts workers per link, watches for links coming and going, restarts links whose
/// connections all gave up, and returns when every chunk is done, the download is cancelled or it cannot continue.
async fn supervise(ctx: &Arc<Ctx>, opts: &Options, mut clients: Vec<Client>) {
    let mut defs: Vec<Route> = opts.routes.clone();
    let mut set: JoinSet<()> = JoinSet::new();
    for (i, client) in clients.iter().enumerate() {
        spawn_workers(ctx, &mut set, i, client);
    }
    let mut tick = tokio::time::interval(Duration::from_millis(250));
    let (mut last_watch, mut no_link_since) = (Instant::now(), None::<Instant>);
    loop {
        if ctx.sched.finished() || ctx.abort.is_cancelled() {
            break;
        }
        tokio::select! {
            _ = ctx.abort.cancelled() => break,
            Some(_) = set.join_next() => {}
            _ = tick.tick() => {}
        }
        if ctx.sched.finished() || ctx.abort.is_cancelled() {
            break;
        }

        if let Some(watch) = &opts.link_watch {
            if last_watch.elapsed() >= opts.watch_interval {
                last_watch = Instant::now();
                let w = watch.clone();
                if let Ok(desired) = tokio::task::spawn_blocking(move || w()).await {
                    reconcile(ctx, opts, &mut defs, &mut clients, &mut set, desired);
                }
            }
        }

        // A link whose connections all gave up is tried again after a pause (the network may be back).
        for (i, rs) in ctx.shared.all().iter().enumerate() {
            if !rs.is_down() && rs.workers.load(Ordering::Relaxed) == 0 && rs.last_exit.lock().unwrap().elapsed() >= opts.revive_after {
                restart_route(ctx, opts, &mut defs[i], &mut clients[i], &mut set, i, None);
            }
        }

        // Nothing is running: give links time to reappear before giving up.
        let running: usize = ctx.shared.all().iter().map(|r| r.workers.load(Ordering::Relaxed)).sum();
        if running == 0 {
            let since = *no_link_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= opts.no_link_grace {
                break;
            }
        } else {
            no_link_since = None;
        }
    }
    // Let the remaining workers see `finished` / the cancel and wind down.
    while set.join_next().await.is_some() {}
}

fn spawn_workers(ctx: &Arc<Ctx>, set: &mut JoinSet<()>, route: usize, client: &Client) {
    let rs = ctx.shared.route(route);
    for w in 0..rs.max {
        rs.workers.fetch_add(1, Ordering::Relaxed);
        let (ctx, rs, client) = (ctx.clone(), rs.clone(), client.clone());
        set.spawn(async move {
            let _guard = WorkerGuard(rs.clone());
            worker(ctx, route, w, client, rs).await;
        });
    }
}

/// Bring the running links in line with `desired`: new links join, vanished links drain, changed links restart.
fn reconcile(ctx: &Arc<Ctx>, opts: &Options, defs: &mut Vec<Route>, clients: &mut Vec<Client>, set: &mut JoinSet<()>, desired: Vec<Route>) {
    for d in &desired {
        match defs.iter().position(|r| r.name == d.name) {
            Some(i) => {
                let rs = ctx.shared.route(i);
                if rs.is_down() || defs[i].key() != d.key() {
                    let mut d = d.clone();
                    restart_route(ctx, opts, &mut d, &mut clients[i], set, i, Some(&mut defs[i]));
                }
            }
            None => {
                let Ok(client) = client_via_proxy(d.link.as_ref(), d.mode, opts.proxy.as_deref()) else { continue };
                let idx = defs.len();
                defs.push(d.clone());
                clients.push(client);
                let rs = Arc::new(RouteState::new(d, opts, 0, ctx.abort.child_token()));
                ctx.shared.routes.write().unwrap().push(rs);
                (ctx.emit)(Event::RouteUp { route: idx, name: d.name.clone() });
                spawn_workers(ctx, set, idx, &clients[idx]);
            }
        }
    }
    for (i, r) in defs.iter().enumerate() {
        let rs = ctx.shared.route(i);
        if !rs.is_down() && !desired.iter().any(|d| d.name == r.name) {
            rs.down.store(true, Ordering::Relaxed);
            rs.stop.cancel(); // its workers hand their chunks back and the other links take them
            (ctx.emit)(Event::RouteDown { route: i, reason: "the link went away".into() });
        }
    }
}

/// Replace a link's state with a fresh one (keeping its byte count) and start new workers on a new client.
/// `new_def` replaces the stored definition when the link changed (e.g. a new IP address).
fn restart_route(
    ctx: &Arc<Ctx>,
    opts: &Options,
    def: &mut Route,
    client: &mut Client,
    set: &mut JoinSet<()>,
    i: usize,
    stored: Option<&mut Route>,
) {
    let Ok(new_client) = client_via_proxy(def.link.as_ref(), def.mode, opts.proxy.as_deref()) else { return };
    let old = ctx.shared.route(i);
    old.stop.cancel();
    let fresh = Arc::new(RouteState::new(def, opts, old.bytes.load(Ordering::Relaxed), ctx.abort.child_token()));
    ctx.shared.routes.write().unwrap()[i] = fresh;
    *client = new_client;
    if let Some(s) = stored {
        *s = def.clone();
    }
    (ctx.emit)(Event::RouteUp { route: i, name: def.name.clone() });
    spawn_workers(ctx, set, i, client);
}

async fn worker(ctx: Arc<Ctx>, route: usize, wid: usize, client: Client, rs: Arc<RouteState>) {
    let mut fails = 0u32;
    // Spread connections over the mirrors; a failing URL passes the baton to the next one.
    let mut url_idx = (route + wid) % ctx.urls.len();
    loop {
        if ctx.abort.is_cancelled() || rs.stop.is_cancelled() || ctx.sched.finished() {
            break;
        }
        // Over this link's current connection budget: sit idle, the controller may raise it later.
        if wid >= rs.allowed.load(Ordering::Relaxed) || slow_tail(&ctx, &rs) {
            sleep_or_stop(&rs, Duration::from_millis(100)).await;
            continue;
        }
        let (idx, start, end) = match ctx.sched.lease() {
            Lease::Finished => break,
            Lease::Wait => {
                sleep_or_stop(&rs, Duration::from_millis(100)).await;
                continue;
            }
            Lease::Chunk { idx, start, end, .. } => (idx, start, end),
        };
        rs.active.fetch_add(1, Ordering::Relaxed);
        let r = fetch_chunk(&ctx, &client, route, &rs, url_idx, idx, start, end).await;
        rs.active.fetch_sub(1, Ordering::Relaxed);
        match r {
            Ok(Outcome::Won) => fails = 0,
            Ok(Outcome::Lost) => {
                ctx.sched.release(idx, false);
            }
            Err(FetchErr::Cancelled) => {
                ctx.sched.release(idx, false);
                break;
            }
            Err(FetchErr::Fatal(e)) => {
                ctx.sched.release(idx, false);
                ctx.fail(e);
                break;
            }
            Err(FetchErr::Throttled(code)) => {
                url_idx = (url_idx + 1) % ctx.urls.len();
                let attempts = ctx.sched.release(idx, true);
                *ctx.last_err.lock().unwrap() = format!("server throttled with HTTP {code}");
                if attempts >= ctx.max_attempts {
                    ctx.fail(Error::AllFailed(format!("HTTP {code} on chunk {idx}")));
                    break;
                }
                // Fewer parallel connections on this link, at most once per 2 s.
                {
                    let mut t = rs.last_throttle.lock().unwrap();
                    if t.elapsed() > Duration::from_secs(2) {
                        *t = Instant::now();
                        let now = rs.allowed.load(Ordering::Relaxed).saturating_sub(1).max(1);
                        rs.allowed.store(now, Ordering::Relaxed);
                        rs.ceiling.fetch_min(now, Ordering::Relaxed);
                    }
                }
                fails += 1;
                sleep_or_stop(&rs, backoff(fails)).await;
            }
            Err(FetchErr::Retry(msg)) => {
                url_idx = (url_idx + 1) % ctx.urls.len();
                let attempts = ctx.sched.release(idx, true);
                *ctx.last_err.lock().unwrap() = msg.clone();
                if attempts >= ctx.max_attempts {
                    ctx.fail(Error::AllFailed(format!("chunk {idx}: {msg}")));
                    break;
                }
                fails += 1;
                if fails >= LINK_FAIL_LIMIT {
                    // This link keeps failing: stop using it for now; the supervisor retries it later
                    // and the other links take over its chunks in the meantime.
                    (ctx.emit)(Event::RouteDown { route, reason: msg });
                    break;
                }
                sleep_or_stop(&rs, backoff(fails)).await;
            }
        }
    }
}

fn backoff(fails: u32) -> Duration {
    Duration::from_millis(250u64 << fails.min(5)).min(Duration::from_secs(8))
}

/// Sleep, but wake early when the download is cancelled or this link is told to stop.
async fn sleep_or_stop(rs: &RouteState, d: Duration) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = rs.stop.cancelled() => {}
    }
}

#[allow(clippy::too_many_arguments)]
async fn fetch_chunk(
    ctx: &Ctx,
    client: &Client,
    route: usize,
    rs: &RouteState,
    url_idx: usize,
    idx: usize,
    start: u64,
    end: u64,
) -> std::result::Result<Outcome, FetchErr> {
    let mirror = url_idx != 0;
    // Carry on from where an earlier attempt (or an earlier run) stopped inside this chunk.
    let (have, seed_crc) = ctx.sched.have(idx);
    let from = start + have;
    let mut req = client.get(&ctx.urls[url_idx]).headers(ctx.headers[url_idx].clone()).header(RANGE, format!("bytes={from}-{end}"));
    // The validator belongs to the primary URL; a mirror may have different ETags for identical bytes.
    if let (Some(v), false) = (&ctx.validator, mirror) {
        req = req.header(IF_RANGE, v);
    }
    let resp = tokio::select! {
        _ = rs.stop.cancelled() => return Err(FetchErr::Cancelled),
        r = tokio::time::timeout(ctx.stall_timeout, req.send()) => match r {
            Ok(r) => r.map_err(|e| FetchErr::Retry(e.to_string()))?,
            Err(_) => return Err(FetchErr::Retry("no response from the server".into())),
        },
    };
    match resp.status() {
        StatusCode::PARTIAL_CONTENT => {}
        // A mirror that ignores ranges is just a bad mirror; on the primary it means the file changed.
        StatusCode::OK if mirror => return Err(FetchErr::Retry("mirror ignored the range request".into())),
        StatusCode::OK => return Err(FetchErr::Fatal(Error::FileChanged)),
        s @ (StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE | StatusCode::FORBIDDEN) => {
            return Err(FetchErr::Throttled(s.as_u16()))
        }
        s if s.is_server_error() => return Err(FetchErr::Retry(format!("HTTP {s}"))),
        s if mirror => return Err(FetchErr::Retry(format!("mirror answered HTTP {s}"))),
        s => return Err(FetchErr::Fatal(Error::Status(s.as_u16()))),
    }

    let mut stream = resp.bytes_stream();
    let (mut pos, mut got) = (from, 0u64);
    let mut crc = crc32fast::Hasher::new_with_initial_len(seed_crc, have);
    let mut last_saved = 0u64;
    // A fetch that stops early keeps what it wrote when it is the only one on the chunk, so the next attempt (or run)
    // continues there. Otherwise its bytes are given up and the chunk is fetched in full by whoever wins.
    let stop = |got: u64, crc: &crc32fast::Hasher| {
        ctx.shared.inflight.fetch_sub(got, Ordering::Relaxed);
        if got == 0 {
            return;
        }
        let sum = crc.clone().finalize();
        if let Some(delta) = ctx.sched.set_have(idx, have + got, sum) {
            ctx.shared.settled.fetch_add(delta, Ordering::Relaxed);
            if let Some(s) = &ctx.store {
                let _ = s.set_partial(&ctx.id, idx, have + got, sum);
            }
        }
    };
    while pos <= end {
        let next = tokio::select! {
            _ = rs.stop.cancelled() => { stop(got, &crc); return Err(FetchErr::Cancelled) }
            n = tokio::time::timeout(ctx.stall_timeout, stream.next()) => match n {
                Ok(n) => n,
                Err(_) => { stop(got, &crc); return Err(FetchErr::Retry("the connection stalled".into())) }
            },
        };
        if ctx.sched.is_done(idx) {
            ctx.shared.inflight.fetch_sub(got, Ordering::Relaxed);
            return Ok(Outcome::Lost);
        }
        match next {
            Some(Ok(b)) => {
                let take = (b.len() as u64).min(end + 1 - pos) as usize;
                if let Some(l) = &ctx.shared.limit {
                    l.acquire(take as u64).await;
                }
                if let Some(l) = &rs.limit {
                    l.acquire(take as u64).await;
                }
                if let Err(e) = ctx.file.write_all_at(&b[..take], pos) {
                    ctx.shared.inflight.fetch_sub(got, Ordering::Relaxed);
                    return Err(FetchErr::Fatal(e.into()));
                }
                crc.update(&b[..take]);
                pos += take as u64;
                got += take as u64;
                rs.bytes.fetch_add(take as u64, Ordering::Relaxed);
                ctx.shared.inflight.fetch_add(take as u64, Ordering::Relaxed);
                // Checkpoint about every MiB so even a crash loses little; the saved part is re-checked on resume.
                if got - last_saved >= CHECKPOINT && pos <= end {
                    last_saved = got;
                    let sum = crc.clone().finalize();
                    if let Some(delta) = ctx.sched.set_have(idx, have + got, sum) {
                        ctx.shared.settled.fetch_add(delta, Ordering::Relaxed);
                        ctx.shared.inflight.fetch_sub(delta, Ordering::Relaxed);
                        if let Some(s) = &ctx.store {
                            let _ = s.set_partial(&ctx.id, idx, have + got, sum);
                        }
                    }
                }
            }
            Some(Err(e)) => {
                stop(got, &crc);
                return Err(FetchErr::Retry(e.to_string()));
            }
            None => {
                stop(got, &crc);
                return Err(FetchErr::Retry("connection closed before the chunk finished".into()));
            }
        }
    }

    let sum = crc.finalize();
    ctx.shared.inflight.fetch_sub(got, Ordering::Relaxed);
    match ctx.sched.complete(idx) {
        Some(delta) => {
            ctx.shared.settled.fetch_add(delta, Ordering::Relaxed);
            // The checksum covers the whole chunk, including the part fetched in an earlier attempt.
            if let Some(s) = &ctx.store {
                if let Err(e) = s.mark_done(&ctx.id, idx, sum) {
                    return Err(FetchErr::Fatal(e));
                }
            }
            (ctx.emit)(Event::ChunkDone { idx, route });
            Ok(Outcome::Won)
        }
        None => Ok(Outcome::Lost),
    }
}

pub(crate) fn spawn_ticker(shared: Arc<Shared>, emit: Emit) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let tick = Duration::from_millis(250);
        let (mut last_routes, mut route_ema): (Vec<u64>, Vec<f64>) = (Vec::new(), Vec::new());
        loop {
            tokio::time::sleep(tick).await;
            let secs = tick.as_secs_f64();
            let g = shared.settled.load(Ordering::Relaxed) + shared.inflight.load(Ordering::Relaxed);
            let states = shared.all();
            while last_routes.len() < states.len() {
                last_routes.push(states[last_routes.len()].bytes.load(Ordering::Relaxed));
                route_ema.push(0.0);
            }
            let routes = states
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let b = r.bytes.load(Ordering::Relaxed);
                    route_ema[i] = 0.5 * route_ema[i] + 0.5 * (b.saturating_sub(last_routes[i]) as f64 / secs);
                    last_routes[i] = b;
                    r.speed.store(route_ema[i].to_bits(), Ordering::Relaxed);
                    RouteStat {
                        name: r.name.clone(),
                        bytes: b,
                        bytes_per_sec: route_ema[i],
                        connections: r.active.load(Ordering::Relaxed),
                        down: r.is_down() || r.workers.load(Ordering::Relaxed) == 0,
                    }
                })
                .collect();
            let ema: f64 = route_ema.iter().sum();
            // Raced duplicates count while in flight; never report more than the file size.
            let total = Some(shared.total.load(Ordering::Relaxed)).filter(|t| *t > 0);
            let shown = total.map_or(g, |t| g.min(t));
            emit(Event::Progress(Snapshot { downloaded: shown, total, bytes_per_sec: ema, routes }));
        }
    })
}

pub(crate) fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.into());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..).map(|n| dir.join(format!("{stem} ({n}){ext}"))).find(|c| !c.exists()).expect("unbounded range")
}

/// Verify a finished file against the checksum in the options, if any.
pub(crate) fn verify_checksum_path(opts: &Options, path: &Path) -> Result<()> {
    verify_checksum(opts, path, None)
}

/// Check the finished staging file; on a mismatch drop it (and its resume record) so a retry starts clean.
fn verify_checksum(opts: &Options, staging: &Path, resume: Option<(&Store, &str)>) -> Result<()> {
    let Some(sum) = &opts.checksum else { return Ok(()) };
    let res = checksum::verify(sum, staging);
    if matches!(res, Err(Error::ChecksumMismatch { .. })) {
        let _ = std::fs::remove_file(staging);
        if let Some((s, id)) = resume {
            let _ = s.delete(id);
        }
    }
    res
}

fn slow_tail(ctx: &Ctx, me: &RouteState) -> bool {
    let routes = ctx.shared.all();
    if routes.len() < 2 {
        return false;
    }
    let speed = |r: &RouteState| f64::from_bits(r.speed.load(Ordering::Relaxed));
    let Some((best, best_state)) = routes.iter().filter(|r| !r.is_down()).map(|r| (speed(r), r)).max_by(|a, b| a.0.total_cmp(&b.0)) else {
        return false;
    };
    is_slow_tail(speed(me), best, ctx.sched.pending_len(), best_state.allowed.load(Ordering::Relaxed))
}

/// Every `interval`, let each link's connection count follow its measured speed.
fn spawn_ramp(shared: Arc<Shared>, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // One ramp per link state; a restarted link gets a new state and so a fresh ramp.
        let mut ramps: Vec<(Arc<RouteState>, Ramp)> = Vec::new();
        loop {
            tokio::time::sleep(interval).await;
            for (i, r) in shared.all().into_iter().enumerate() {
                if ramps.len() <= i {
                    ramps.push((r.clone(), Ramp::new(r.max, true)));
                } else if !Arc::ptr_eq(&ramps[i].0, &r) {
                    ramps[i] = (r.clone(), Ramp::new(r.max, true));
                }
                let ramp = &mut ramps[i].1;
                ramp.allowed = r.allowed.load(Ordering::Relaxed);
                // Throttling lowers the ceiling from the worker side.
                let ceiling = r.ceiling.load(Ordering::Relaxed);
                if ceiling < ramp.ceiling {
                    ramp.cap(ceiling);
                }
                let n = ramp.step(f64::from_bits(r.speed.load(Ordering::Relaxed)));
                r.allowed.store(n, Ordering::Relaxed);
            }
        }
    })
}

pub(crate) fn max_conns(opts: &Options, r: &Route) -> usize {
    r.max_conns.unwrap_or(opts.conns_per_route).max(1)
}

/// The primary URL plus every mirror that serves the same bytes (same size, range support).
async fn usable_urls(opts: &Options, primary: &Probe, clients: &[Client]) -> Vec<String> {
    let mut urls = vec![primary.final_url.clone()];
    let Some(client) = clients.first() else { return urls };
    for m in &opts.mirrors {
        if m == &opts.url || urls.contains(m) {
            continue;
        }
        let ok = tokio::time::timeout(Duration::from_secs(10), probe(client, m, &headers_for(&opts.headers, &opts.url, m))).await;
        if let Ok(Ok(p)) = ok {
            if p.ranges && p.total == primary.total {
                urls.push(p.final_url);
            }
        }
    }
    urls
}

const SENSITIVE: [&str; 4] = ["authorization", "cookie", "proxy-authorization", "www-authenticate"];

fn origin_of(url: &str) -> Option<(String, String, u16)> {
    let u = url::Url::parse(url).ok()?;
    Some((u.scheme().to_string(), u.host_str()?.to_ascii_lowercase(), u.port_or_known_default()?))
}

/// The request headers to send to `target`. Credentials (`Authorization`, `Cookie`, ...) are dropped when the target
/// is not the same scheme, host and port as the URL the user gave, which happens after a redirect to a CDN and for mirrors.
pub(crate) fn headers_for(headers: &[(String, String)], origin: &str, target: &str) -> HeaderMap {
    let same = origin == target || (origin_of(origin).is_some() && origin_of(origin) == origin_of(target));
    let mut m = HeaderMap::new();
    for (k, v) in headers {
        if !same && SENSITIVE.contains(&k.to_ascii_lowercase().as_str()) {
            continue;
        }
        if let (Ok(k), Ok(v)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
            m.insert(k, v);
        }
    }
    m
}

/// Split finished chunks into those whose bytes still match their saved checksum and those that do not.
fn verify_done(path: &Path, ranges: &[(u64, u64)], done: &[DoneChunk]) -> (Vec<usize>, Vec<usize>) {
    use std::io::{Read, Seek, SeekFrom};
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    let Ok(mut f) = std::fs::File::open(path) else { return (good, done.iter().map(|d| d.0).collect()) };
    let mut buf = vec![0u8; 1 << 20];
    for &(idx, crc) in done {
        let (Some(&(start, end)), Some(want)) = (ranges.get(idx), crc) else {
            bad.push(idx);
            continue;
        };
        let mut h = crc32fast::Hasher::new();
        let mut left = end - start + 1;
        let ok = f.seek(SeekFrom::Start(start)).is_ok()
            && loop {
                if left == 0 {
                    break true;
                }
                let n = (buf.len() as u64).min(left) as usize;
                if f.read_exact(&mut buf[..n]).is_err() {
                    break false;
                }
                h.update(&buf[..n]);
                left -= n as u64;
            };
        if ok && h.finalize() == want {
            good.push(idx);
        } else {
            bad.push(idx);
        }
    }
    (good, bad)
}

/// Check partly downloaded chunks against the checksum saved with them. Returns the ones still good as
/// `(index, bytes, crc)` and the indexes of the damaged ones.
fn verify_partial(path: &Path, ranges: &[(u64, u64)], partial: &[PartialChunk]) -> (Vec<(usize, u64, u32)>, Vec<usize>) {
    use std::io::{Read, Seek, SeekFrom};
    let (mut good, mut bad) = (Vec::new(), Vec::new());
    let Ok(mut f) = std::fs::File::open(path) else { return (good, partial.iter().map(|p| p.0).collect()) };
    let mut buf = vec![0u8; 1 << 20];
    for &(idx, len, want) in partial {
        let Some(&(start, end)) = ranges.get(idx) else {
            bad.push(idx);
            continue;
        };
        if len == 0 || len > end - start {
            bad.push(idx);
            continue;
        }
        let mut h = crc32fast::Hasher::new();
        let mut left = len;
        let ok = f.seek(SeekFrom::Start(start)).is_ok()
            && loop {
                if left == 0 {
                    break true;
                }
                let n = (buf.len() as u64).min(left) as usize;
                if f.read_exact(&mut buf[..n]).is_err() {
                    break false;
                }
                h.update(&buf[..n]);
                left -= n as u64;
            };
        if ok && h.finalize() == want {
            good.push((idx, len, want));
        } else {
            bad.push(idx);
        }
    }
    (good, bad)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(m: &HeaderMap) -> Vec<String> {
        let mut v: Vec<String> = m.keys().map(|k| k.as_str().to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn credentials_stay_on_the_original_host() {
        let h = vec![
            ("Authorization".to_string(), "Bearer x".to_string()),
            ("Cookie".to_string(), "a=b".to_string()),
            ("Referer".to_string(), "https://site/".to_string()),
        ];
        let all = ["authorization", "cookie", "referer"];
        assert_eq!(names(&headers_for(&h, "https://a.example/f", "https://a.example/other?x=1")), all);
        assert_eq!(names(&headers_for(&h, "https://a.example/f", "https://cdn.example/f")), ["referer"], "another host");
        assert_eq!(names(&headers_for(&h, "https://a.example/f", "http://a.example/f")), ["referer"], "downgrade to http");
        assert_eq!(names(&headers_for(&h, "https://a.example/f", "https://a.example:8443/f")), ["referer"], "another port");
    }
}
