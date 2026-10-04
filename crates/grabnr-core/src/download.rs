//! The download itself: probe, plan chunks, run workers on every link, resume.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, IF_RANGE, RANGE};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::adapt::{is_slow_tail, Ramp};
use crate::auth::Auth;
use crate::bind::{client_via_proxy, BindMode};
use crate::checksum::{self, Checksum};
use crate::disk;
use crate::limiter::Limiter;
use crate::error::{Error, Result};
use crate::interfaces::Link;
use crate::probe::{probe, sanitize, Probe};
use crate::scheduler::{chunk_size_for, plan, Lease, Scheduler};
use crate::store::{Record, Store};
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

    /// Let the OS routing table decide (used for single-link downloads and tests).
    pub fn unbound(name: &str) -> Self {
        Route { name: name.into(), link: None, mode: BindMode::None, speed_limit: None, max_conns: None }
    }
}

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
            checksum: None,
            adaptive: true,
            ramp_interval: Duration::from_secs(2),
            proxy: None,
            auth: None,
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
    Started { filename: String, total: Option<u64>, chunks: usize, ranges: bool, resumed_chunks: usize },
    ResumeDiscarded { reason: String },
    Progress(Snapshot),
    /// Which link fetched which chunk (drives the chunk grid).
    ChunkDone { idx: usize, route: usize },
    RouteDown { route: usize, reason: String },
    Finished { path: String },
}

pub type Emit = Arc<dyn Fn(Event) + Send + Sync>;

const LINK_FAIL_LIMIT: u32 = 5;

struct RouteState {
    name: String,
    bytes: AtomicU64,
    allowed: AtomicUsize,
    /// Most connections this link may use; lowered when the server throttles.
    ceiling: AtomicUsize,
    /// Smoothed bytes/s, written by the progress ticker.
    speed: AtomicU64,
    active: AtomicUsize,
    last_throttle: Mutex<Instant>,
    limit: Option<Limiter>,
}

struct Shared {
    total: Option<u64>,
    global: AtomicU64,
    routes: Vec<RouteState>,
    limit: Option<Limiter>,
}

struct Ctx {
    url: String,
    headers: HeaderMap,
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
    let mut headers = HeaderMap::new();
    for (k, v) in &opts.headers {
        let name = HeaderName::from_bytes(k.as_bytes()).map_err(|e| Error::Other(format!("bad header {k}: {e}")))?;
        let val = HeaderValue::from_str(v).map_err(|e| Error::Other(format!("bad header value for {k}: {e}")))?;
        headers.insert(name, val);
    }
    let clients: Vec<Client> =
        opts.routes.iter().map(|r| client_via_proxy(r.link.as_ref(), r.mode, opts.proxy.as_deref())).collect::<std::result::Result<_, _>>()?;

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

    let shared = Arc::new(Shared {
        total: probe.total,
        global: AtomicU64::new(0),
        routes: opts
            .routes
            .iter()
            .map(|r| RouteState {
                name: r.name.clone(),
                bytes: AtomicU64::new(0),
                allowed: AtomicUsize::new(Ramp::new(max_conns(&opts, r), opts.adaptive).allowed),
                ceiling: AtomicUsize::new(max_conns(&opts, r)),
                speed: AtomicU64::new(0),
                active: AtomicUsize::new(0),
                last_throttle: Mutex::new(Instant::now() - Duration::from_secs(60)),
                limit: r.speed_limit.map(Limiter::new),
            })
            .collect(),
        limit: opts.speed_limit.map(Limiter::new),
    });

    let final_path = match (probe.ranges, probe.total) {
        (true, Some(total)) => {
            ranged(&opts, &probe, &clients, &filename, &staging, total, shared, cancel, emit.clone()).await?
        }
        _ => single(&opts, &probe, &clients[probe_route], &filename, &staging, shared, cancel, emit.clone()).await?,
    };
    emit(Event::Finished { path: final_path.display().to_string() });
    Ok(final_path)
}

#[allow(clippy::too_many_arguments)]
async fn ranged(
    opts: &Options,
    probe: &Probe,
    clients: &[Client],
    filename: &str,
    staging: &Path,
    total: u64,
    shared: Arc<Shared>,
    cancel: CancellationToken,
    emit: Emit,
) -> Result<PathBuf> {
    let id = format!("{}|{}", opts.url, staging.display());
    let mut chunk_size = chunk_size_for(total, opts.routes.len());
    let mut done: Vec<usize> = Vec::new();

    let fresh = Record {
        id: id.clone(),
        url: opts.url.clone(),
        total,
        etag: probe.etag.clone(),
        last_modified: probe.last_modified.clone(),
        chunk_size,
    };
    match opts.store.as_ref().map(|s| s.load(&id)).transpose()?.flatten() {
        Some((rec, d)) if rec.total == total && rec.etag == fresh.etag && rec.last_modified == fresh.last_modified
            && std::fs::metadata(staging).map(|m| m.len() == total).unwrap_or(false) =>
        {
            chunk_size = rec.chunk_size;
            done = d;
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
    let resumed_bytes: u64 = done.iter().filter_map(|&i| ranges.get(i)).map(|&(s, e)| e - s + 1).sum();
    shared.global.store(resumed_bytes, Ordering::Relaxed);
    disk::ensure_space(&opts.dest_dir, total.saturating_sub(resumed_bytes))?;
    let file = OffsetFile::open_staging(staging, total)?;

    emit(Event::Started { filename: filename.into(), total: Some(total), chunks: ranges.len(), ranges: true, resumed_chunks: done.len() });

    let ctx = Arc::new(Ctx {
        url: probe.final_url.clone(),
        headers: opts.headers.iter().fold(HeaderMap::new(), |mut m, (k, v)| {
            if let (Ok(k), Ok(v)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
                m.insert(k, v);
            }
            m
        }),
        validator: probe.validator().map(str::to_owned),
        sched: Scheduler::new(ranges, &done),
        file,
        store: opts.store.clone(),
        id: id.clone(),
        shared: shared.clone(),
        abort: cancel.child_token(),
        fatal: Mutex::new(None),
        last_err: Mutex::new(String::new()),
        emit: emit.clone(),
        max_attempts: opts.max_attempts,
    });

    let ticker = spawn_ticker(shared.clone(), emit.clone());
    let ramp = opts.adaptive.then(|| spawn_ramp(shared.clone(), opts.routes.iter().map(|r| max_conns(opts, r)).collect(), opts.ramp_interval));
    let mut workers = Vec::new();
    for (r, client) in clients.iter().enumerate() {
        for w in 0..max_conns(opts, &opts.routes[r]) {
            workers.push(tokio::spawn(worker(ctx.clone(), r, w, client.clone())));
        }
    }
    for w in workers {
        let _ = w.await;
    }
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
    let mut req = client.get(&probe.final_url);
    for (k, v) in &opts.headers {
        req = req.header(k, v);
    }
    let resp = req.send().await?;
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
                        if let Some(l) = &shared.routes[0].limit { l.acquire(b.len() as u64).await; }
                        file.write_all_at(&b, pos)?;
                        pos += b.len() as u64;
                        shared.global.fetch_add(b.len() as u64, Ordering::Relaxed);
                        shared.routes[0].bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
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

async fn worker(ctx: Arc<Ctx>, route: usize, wid: usize, client: Client) {
    let rs = &ctx.shared.routes[route];
    let mut fails = 0u32;
    loop {
        if ctx.abort.is_cancelled() || ctx.sched.finished() {
            break;
        }
        // Over this link's current connection budget: sit idle, the controller may raise it later.
        if wid >= rs.allowed.load(Ordering::Relaxed) || slow_tail(&ctx, route) {
            sleep_or_abort(&ctx, Duration::from_millis(100)).await;
            continue;
        }
        let (idx, start, end, _raced) = match ctx.sched.lease() {
            Lease::Finished => break,
            Lease::Wait => {
                sleep_or_abort(&ctx, Duration::from_millis(100)).await;
                continue;
            }
            Lease::Chunk { idx, start, end, raced } => (idx, start, end, raced),
        };
        rs.active.fetch_add(1, Ordering::Relaxed);
        let r = fetch_chunk(&ctx, &client, route, idx, start, end).await;
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
                sleep_or_abort(&ctx, backoff(fails)).await;
            }
            Err(FetchErr::Retry(msg)) => {
                let attempts = ctx.sched.release(idx, true);
                *ctx.last_err.lock().unwrap() = msg.clone();
                if attempts >= ctx.max_attempts {
                    ctx.fail(Error::AllFailed(format!("chunk {idx}: {msg}")));
                    break;
                }
                fails += 1;
                if fails >= LINK_FAIL_LIMIT {
                    // This link keeps failing: stop using it, the others take over its chunks.
                    (ctx.emit)(Event::RouteDown { route, reason: msg });
                    break;
                }
                sleep_or_abort(&ctx, backoff(fails)).await;
            }
        }
    }
}

fn backoff(fails: u32) -> Duration {
    Duration::from_millis(250u64 << fails.min(5)).min(Duration::from_secs(8))
}

async fn sleep_or_abort(ctx: &Ctx, d: Duration) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = ctx.abort.cancelled() => {}
    }
}

async fn fetch_chunk(ctx: &Ctx, client: &Client, route: usize, idx: usize, start: u64, end: u64) -> std::result::Result<Outcome, FetchErr> {
    let mut req = client.get(&ctx.url).headers(ctx.headers.clone()).header(RANGE, format!("bytes={start}-{end}"));
    if let Some(v) = &ctx.validator {
        req = req.header(IF_RANGE, v);
    }
    let resp = tokio::select! {
        _ = ctx.abort.cancelled() => return Err(FetchErr::Cancelled),
        r = req.send() => r.map_err(|e| FetchErr::Retry(e.to_string()))?,
    };
    match resp.status() {
        StatusCode::PARTIAL_CONTENT => {}
        // We asked for a range and got the whole file: If-Range failed, so the file changed.
        StatusCode::OK => return Err(FetchErr::Fatal(Error::FileChanged)),
        s @ (StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE | StatusCode::FORBIDDEN) => {
            return Err(FetchErr::Throttled(s.as_u16()))
        }
        s if s.is_server_error() => return Err(FetchErr::Retry(format!("HTTP {s}"))),
        s => return Err(FetchErr::Fatal(Error::Status(s.as_u16()))),
    }

    let rs = &ctx.shared.routes[route];
    let mut stream = resp.bytes_stream();
    let (mut pos, mut got) = (start, 0u64);
    let rollback = |got: u64| {
        ctx.shared.global.fetch_sub(got, Ordering::Relaxed);
    };
    while pos <= end {
        let next = tokio::select! {
            _ = ctx.abort.cancelled() => { rollback(got); return Err(FetchErr::Cancelled) }
            n = stream.next() => n,
        };
        if ctx.sched.is_done(idx) {
            rollback(got);
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
                    rollback(got);
                    return Err(FetchErr::Fatal(e.into()));
                }
                pos += take as u64;
                got += take as u64;
                rs.bytes.fetch_add(take as u64, Ordering::Relaxed);
                ctx.shared.global.fetch_add(take as u64, Ordering::Relaxed);
            }
            Some(Err(e)) => {
                rollback(got);
                return Err(FetchErr::Retry(e.to_string()));
            }
            None => {
                rollback(got);
                return Err(FetchErr::Retry("connection closed before the chunk finished".into()));
            }
        }
    }

    if ctx.sched.complete(idx) {
        if let Some(s) = &ctx.store {
            if let Err(e) = s.mark_done(&ctx.id, idx) {
                return Err(FetchErr::Fatal(e));
            }
        }
        (ctx.emit)(Event::ChunkDone { idx, route });
        Ok(Outcome::Won)
    } else {
        rollback(got);
        Ok(Outcome::Lost)
    }
}

fn spawn_ticker(shared: Arc<Shared>, emit: Emit) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let tick = Duration::from_millis(250);
        let mut last_global = shared.global.load(Ordering::Relaxed);
        let mut last_routes: Vec<u64> = shared.routes.iter().map(|r| r.bytes.load(Ordering::Relaxed)).collect();
        let (mut ema, mut route_ema) = (0.0f64, vec![0.0f64; shared.routes.len()]);
        loop {
            tokio::time::sleep(tick).await;
            let secs = tick.as_secs_f64();
            let g = shared.global.load(Ordering::Relaxed);
            ema = 0.5 * ema + 0.5 * (g.saturating_sub(last_global) as f64 / secs);
            last_global = g;
            let routes = shared
                .routes
                .iter()
                .enumerate()
                .map(|(i, r)| {
                    let b = r.bytes.load(Ordering::Relaxed);
                    route_ema[i] = 0.5 * route_ema[i] + 0.5 * (b.saturating_sub(last_routes[i]) as f64 / secs);
                    last_routes[i] = b;
                    r.speed.store(route_ema[i].to_bits(), Ordering::Relaxed);
                    RouteStat { name: r.name.clone(), bytes: b, bytes_per_sec: route_ema[i], connections: r.active.load(Ordering::Relaxed) }
                })
                .collect();
            // A raced duplicate counts until the losing copy aborts; never report more than the file size.
            let shown = shared.total.map_or(g, |t| g.min(t));
            emit(Event::Progress(Snapshot { downloaded: shown, total: shared.total, bytes_per_sec: ema, routes }));
        }
    })
}

fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(name);
    let stem = p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| name.into());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|c| !c.exists())
        .expect("unbounded range")
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

fn slow_tail(ctx: &Ctx, route: usize) -> bool {
    let routes = &ctx.shared.routes;
    if routes.len() < 2 {
        return false;
    }
    let speed = |r: &RouteState| f64::from_bits(r.speed.load(Ordering::Relaxed));
    let Some((best, best_state)) = routes.iter().map(|r| (speed(r), r)).max_by(|a, b| a.0.total_cmp(&b.0)) else { return false };
    is_slow_tail(speed(&routes[route]), best, ctx.sched.pending_len(), best_state.allowed.load(Ordering::Relaxed))
}

/// Every `interval`, let each link's connection count follow its measured speed.
fn spawn_ramp(shared: Arc<Shared>, max_per_route: Vec<usize>, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut ramps: Vec<Ramp> = max_per_route.iter().map(|&m| Ramp::new(m, true)).collect();
        loop {
            tokio::time::sleep(interval).await;
            for (r, ramp) in shared.routes.iter().zip(ramps.iter_mut()) {
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

fn max_conns(opts: &Options, r: &Route) -> usize {
    r.max_conns.unwrap_or(opts.conns_per_route).max(1)
}
