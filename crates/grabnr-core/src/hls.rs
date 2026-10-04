//! HLS (`.m3u8`) playlists: variants, segments, byte ranges and AES-128 keys, and the download of a finished (VOD) stream.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, RANGE};
use reqwest::Client;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::bind::client_via_proxy;
use crate::download::{headers_for, max_conns, spawn_ticker, unique_path, Emit, Event, Options, RouteState, Shared};
use crate::error::{Error, Result};
use crate::limiter::Limiter;
use crate::probe::sanitize;

type Aes128Cbc = cbc::Decryptor<aes::Aes128>;

/// Which stream of a master playlist to take.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quality {
    #[default]
    Best,
    Worst,
    /// The variant closest to this height in pixels (for example 720).
    Height(u32),
}

impl Quality {
    /// `best`, `worst` or a height such as `720`.
    pub fn parse(s: &str) -> Quality {
        match s.trim().to_ascii_lowercase().as_str() {
            "worst" | "lowest" => Quality::Worst,
            "" | "best" | "highest" => Quality::Best,
            n => n.trim_end_matches('p').parse().map(Quality::Height).unwrap_or(Quality::Best),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Variant {
    pub url: String,
    pub bandwidth: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// For example `720p · 2.8 Mbps`.
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    pub uri: String,
    pub iv: Option<[u8; 16]>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub url: String,
    pub duration: f64,
    /// `(length, offset)` when the segment is a slice of a larger file.
    pub range: Option<(u64, u64)>,
    pub key: Option<Key>,
    /// Media sequence number; the default AES IV.
    pub seq: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Media {
    pub segments: Vec<Segment>,
    /// A finished (video on demand) stream. A playlist without it is live and keeps growing.
    pub endlist: bool,
    /// Initialization section of fragmented MP4 streams, written before the first segment.
    pub init: Option<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Playlist {
    Master(Vec<Variant>),
    Media(Media),
}

/// Is this URL a playlist? (By its path; the query string is ignored.)
pub fn looks_like(url: &str) -> bool {
    url.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase().ends_with(".m3u8")
}

fn attrs(line: &str) -> Vec<(String, String)> {
    // KEY=VALUE,KEY="quoted, value"
    let mut out = Vec::new();
    let (mut key, mut val, mut in_key, mut quoted) = (String::new(), String::new(), true, false);
    for ch in line.chars().chain(std::iter::once(',')) {
        match (ch, in_key, quoted) {
            ('=', true, _) => in_key = false,
            ('"', false, _) => quoted = !quoted,
            (',', false, false) => {
                out.push((key.trim().to_ascii_uppercase(), val.trim().to_string()));
                key.clear();
                val.clear();
                in_key = true;
            }
            (c, true, _) => key.push(c),
            (c, false, _) => val.push(c),
        }
    }
    out
}

fn attr<'a>(a: &'a [(String, String)], k: &str) -> Option<&'a str> {
    a.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn resolve(base: &Url, href: &str) -> Result<String> {
    base.join(href.trim()).map(|u| u.to_string()).map_err(|_| Error::Other(format!("bad link in the playlist: {href}")))
}

fn parse_iv(s: &str) -> Option<[u8; 16]> {
    let h = s.trim_start_matches("0x").trim_start_matches("0X");
    if h.len() != 32 {
        return None;
    }
    let mut iv = [0u8; 16];
    for (i, b) in iv.iter_mut().enumerate() {
        *b = u8::from_str_radix(&h[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(iv)
}

fn parse_range(s: &str, prev_end: u64) -> Option<(u64, u64)> {
    let (len, off) = match s.split_once('@') {
        Some((l, o)) => (l.parse().ok()?, o.parse().ok()?),
        None => (s.parse().ok()?, prev_end),
    };
    Some((len, off))
}

pub fn parse(text: &str, base_url: &str) -> Result<Playlist> {
    let base = Url::parse(base_url).map_err(|_| Error::Other("bad playlist link".into()))?;
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    if lines.next() != Some("#EXTM3U") {
        return Err(Error::Other("that is not an HLS playlist".into()));
    }
    let (mut variants, mut segments) = (Vec::new(), Vec::new());
    let (mut endlist, mut seq, mut pending_dur, mut key, mut init) = (false, 0u64, None::<f64>, None::<Key>, None::<Segment>);
    let (mut next_variant, mut pending_range, mut prev_range_end, mut seen_media_seq) = (None::<Variant>, None::<(u64, u64)>, 0u64, false);

    for line in lines {
        if let Some(rest) = line.strip_prefix("#EXT-X-STREAM-INF:") {
            let a = attrs(rest);
            let bandwidth = attr(&a, "BANDWIDTH").and_then(|b| b.parse().ok()).unwrap_or(0);
            let (w, h) = attr(&a, "RESOLUTION")
                .and_then(|r| r.split_once('x'))
                .map(|(w, h)| (w.parse().ok(), h.parse().ok()))
                .unwrap_or((None, None));
            let label = match h {
                Some(h) => format!("{h}p · {:.1} Mbps", bandwidth as f64 / 1e6),
                None => format!("{:.1} Mbps", bandwidth as f64 / 1e6),
            };
            next_variant = Some(Variant { url: String::new(), bandwidth, width: w, height: h, label });
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MEDIA-SEQUENCE:") {
            seq = rest.trim().parse().unwrap_or(0);
            seen_media_seq = true;
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            pending_dur = rest.split(',').next().and_then(|d| d.trim().parse().ok());
        } else if let Some(rest) = line.strip_prefix("#EXT-X-BYTERANGE:") {
            pending_range = parse_range(rest.trim(), prev_range_end);
        } else if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            let a = attrs(rest);
            match attr(&a, "METHOD").unwrap_or("NONE") {
                "NONE" => key = None,
                "AES-128" => {
                    let uri = attr(&a, "URI").ok_or_else(|| Error::Other("an encrypted stream has no key address".into()))?;
                    key = Some(Key { uri: resolve(&base, uri)?, iv: attr(&a, "IV").and_then(parse_iv) });
                }
                other => return Err(Error::Other(format!("this stream uses {other} encryption, which grabnr cannot decrypt"))),
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            let a = attrs(rest);
            if let Some(uri) = attr(&a, "URI") {
                let range = attr(&a, "BYTERANGE").and_then(|r| parse_range(r, 0));
                init = Some(Segment { url: resolve(&base, uri)?, duration: 0.0, range, key: None, seq: 0 });
            }
        } else if line == "#EXT-X-ENDLIST" {
            endlist = true;
        } else if !line.starts_with('#') {
            if let Some(mut v) = next_variant.take() {
                v.url = resolve(&base, line)?;
                variants.push(v);
            } else {
                let range = pending_range.take();
                if let Some((len, off)) = range {
                    prev_range_end = off + len;
                }
                segments.push(Segment {
                    url: resolve(&base, line)?,
                    duration: pending_dur.take().unwrap_or(0.0),
                    range,
                    key: key.clone(),
                    seq,
                });
                seq += 1;
            }
        }
    }
    let _ = seen_media_seq;
    if !variants.is_empty() {
        return Ok(Playlist::Master(variants));
    }
    if segments.is_empty() {
        return Err(Error::Other("the playlist has no video segments".into()));
    }
    Ok(Playlist::Media(Media { segments, endlist, init }))
}

/// The variant a quality setting asks for.
pub fn choose(variants: &[Variant], q: Quality) -> Option<&Variant> {
    match q {
        Quality::Best => variants.iter().max_by_key(|v| (v.height.unwrap_or(0), v.bandwidth)),
        Quality::Worst => variants.iter().min_by_key(|v| (v.height.unwrap_or(u32::MAX), v.bandwidth)),
        Quality::Height(h) => variants.iter().min_by_key(|v| (v.height.unwrap_or(0) as i64 - h as i64).abs()),
    }
}

/// Decrypt one AES-128-CBC segment. The IV is the one in the playlist, or the segment's sequence number.
pub fn decrypt(data: &[u8], key: &[u8], iv: Option<[u8; 16]>, seq: u64) -> Result<Vec<u8>> {
    let iv = iv.unwrap_or_else(|| {
        let mut b = [0u8; 16];
        b[8..].copy_from_slice(&seq.to_be_bytes());
        b
    });
    let key: [u8; 16] = key.try_into().map_err(|_| Error::Other("the stream's key is not 16 bytes".into()))?;
    Aes128Cbc::new(&key.into(), &iv.into())
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .map_err(|_| Error::Other("a video segment could not be decrypted".into()))
}

fn out_name(url: &str, ext: &str) -> String {
    let stem = Url::parse(url)
        .ok()
        .and_then(|u| u.path_segments().and_then(|mut s| s.rfind(|p| !p.is_empty()).map(str::to_owned)))
        .map(|f| f.trim_end_matches(".m3u8").trim_end_matches(".M3U8").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "video".into());
    sanitize(&format!("{stem}.{ext}"))
}

async fn get_bytes(
    client: &Client,
    url: &str,
    headers: HeaderMap,
    range: Option<(u64, u64)>,
    stall: Duration,
    stop: &CancellationToken,
) -> std::result::Result<Vec<u8>, SegErr> {
    let mut req = client.get(url).headers(headers);
    if let Some((len, off)) = range {
        req = req.header(RANGE, format!("bytes={}-{}", off, off + len - 1));
    }
    let resp = tokio::select! {
        _ = stop.cancelled() => return Err(SegErr::Cancelled),
        r = tokio::time::timeout(stall, req.send()) => match r {
            Ok(r) => r.map_err(|e| SegErr::Retry(e.to_string()))?,
            Err(_) => return Err(SegErr::Retry("no response from the server".into())),
        },
    };
    let st = resp.status();
    if !(st.is_success()) {
        return Err(if st.is_server_error() || st.as_u16() == 429 || st.as_u16() == 403 {
            SegErr::Retry(format!("HTTP {st}"))
        } else {
            SegErr::Fatal(Error::Status(st.as_u16()))
        });
    }
    let mut stream = resp.bytes_stream();
    let mut body = Vec::new();
    loop {
        let next = tokio::select! {
            _ = stop.cancelled() => return Err(SegErr::Cancelled),
            n = tokio::time::timeout(stall, stream.next()) => match n {
                Ok(n) => n,
                Err(_) => return Err(SegErr::Retry("the connection stalled".into())),
            },
        };
        match next {
            Some(Ok(b)) => body.extend_from_slice(&b),
            Some(Err(e)) => return Err(SegErr::Retry(e.to_string())),
            None => return Ok(body),
        }
    }
}

enum SegErr {
    Cancelled,
    Fatal(Error),
    Retry(String),
}

struct Job {
    segments: Vec<Segment>,
    dir: PathBuf,
    keys: std::collections::HashMap<String, Vec<u8>>,
    pending: Mutex<std::collections::VecDeque<usize>>,
    attempts: Vec<AtomicUsize>,
    done: AtomicUsize,
    /// Bytes of finished segments, to estimate the total size.
    bytes_done: AtomicU64,
    fatal: Mutex<Option<Error>>,
    headers: Vec<HeaderMap>,
    origin: String,
}

fn seg_path(dir: &Path, i: usize) -> PathBuf {
    dir.join(format!("seg-{i:06}"))
}

/// Download a finished HLS stream: pick a variant, fetch every segment over every link, decrypt, and join them.
pub async fn download(opts: &Options, cancel: CancellationToken, emit: Emit) -> Result<PathBuf> {
    let clients: Vec<Client> = opts
        .routes
        .iter()
        .map(|r| client_via_proxy(r.link.as_ref(), r.mode, opts.proxy.as_deref()))
        .collect::<std::result::Result<_, _>>()?;
    let first = clients.first().ok_or_else(|| Error::Other("no network routes selected".into()))?;
    let stop = cancel.child_token();
    let h0 = headers_for(&opts.headers, &opts.url, &opts.url);

    // Playlist: a master list points at variants; follow it to a media playlist.
    let mut url = opts.url.clone();
    let mut media = None;
    for _ in 0..3 {
        let text =
            get_bytes(first, &url, headers_for(&opts.headers, &opts.url, &url), None, opts.stall_timeout, &stop).await.map_err(seg_err)?;
        match parse(&String::from_utf8_lossy(&text), &url)? {
            Playlist::Media(m) => {
                media = Some(m);
                break;
            }
            Playlist::Master(vs) => {
                url = choose(&vs, opts.hls_quality).ok_or_else(|| Error::Other("the playlist lists no streams".into()))?.url.clone();
            }
        }
    }
    let _ = h0;
    let media = media.ok_or_else(|| Error::Other("the playlist points to more playlists than grabnr follows".into()))?;
    if !media.endlist {
        return Err(Error::Other("this is a live stream; grabnr can only download finished videos".into()));
    }

    std::fs::create_dir_all(&opts.dest_dir)?;
    let ext = if media.init.is_some() { "mp4" } else { "ts" };
    let filename = opts.filename.clone().map(|f| sanitize(&f)).unwrap_or_else(|| out_name(&opts.url, ext));
    let work = opts.dest_dir.join(format!("{filename}.grabnr-hls"));
    std::fs::create_dir_all(&work)?;

    // Keys are small; fetch each once up front.
    let mut keys = std::collections::HashMap::new();
    for s in media.segments.iter().chain(media.init.iter()) {
        if let Some(k) = &s.key {
            if !keys.contains_key(&k.uri) {
                let b = get_bytes(first, &k.uri, headers_for(&opts.headers, &opts.url, &k.uri), None, opts.stall_timeout, &stop)
                    .await
                    .map_err(seg_err)?;
                keys.insert(k.uri.clone(), b);
            }
        }
    }
    let init_bytes = match &media.init {
        Some(s) => Some(
            get_bytes(first, &s.url, headers_for(&opts.headers, &opts.url, &s.url), s.range, opts.stall_timeout, &stop)
                .await
                .map_err(seg_err)?,
        ),
        None => None,
    };

    let n = media.segments.len();
    let already: Vec<usize> = (0..n).filter(|&i| std::fs::metadata(seg_path(&work, i)).map(|m| m.len() > 0).unwrap_or(false)).collect();
    let bytes_done: u64 = already.iter().filter_map(|&i| std::fs::metadata(seg_path(&work, i)).ok()).map(|m| m.len()).sum();
    emit(Event::Started { filename: filename.clone(), total: None, chunks: n, ranges: true, resumed_chunks: already.len() });
    for &i in &already {
        emit(Event::ChunkDone { idx: i, route: 0 });
    }

    let shared = Arc::new(Shared::new(None, opts, &stop));
    shared.settled.store(bytes_done, Ordering::Relaxed);
    let job = Arc::new(Job {
        headers: media.segments.iter().map(|s| headers_for(&opts.headers, &opts.url, &s.url)).collect(),
        origin: opts.url.clone(),
        pending: Mutex::new((0..n).filter(|i| !already.contains(i)).collect()),
        attempts: (0..n).map(|_| AtomicUsize::new(0)).collect(),
        done: AtomicUsize::new(already.len()),
        bytes_done: AtomicU64::new(bytes_done),
        fatal: Mutex::new(None),
        segments: media.segments,
        dir: work.clone(),
        keys,
    });
    let _ = &job.origin;
    if already.len() == n {
        return finish(opts, &job, init_bytes, &filename, ext).await;
    }

    let ticker = spawn_ticker(shared.clone(), emit.clone());
    let mut set = JoinSet::new();
    for (r, client) in clients.iter().enumerate() {
        let rs = shared.route(r);
        for _ in 0..max_conns(opts, &opts.routes[r]) {
            let (job, shared, rs, client, stop, emit) =
                (job.clone(), shared.clone(), rs.clone(), client.clone(), stop.clone(), emit.clone());
            let (stall, max_attempts) = (opts.stall_timeout, opts.max_attempts);
            set.spawn(async move { worker(job, shared, rs, client, r, stop, emit, stall, max_attempts as usize).await });
        }
    }
    while set.join_next().await.is_some() {}
    ticker.abort();

    if let Some(e) = job.fatal.lock().unwrap().take() {
        return Err(e);
    }
    if cancel.is_cancelled() {
        return Err(Error::Cancelled);
    }
    if job.done.load(Ordering::Relaxed) < n {
        return Err(Error::AllFailed("some video segments could not be downloaded".into()));
    }
    finish(opts, &job, init_bytes, &filename, ext).await
}

fn seg_err(e: SegErr) -> Error {
    match e {
        SegErr::Cancelled => Error::Cancelled,
        SegErr::Fatal(e) => e,
        SegErr::Retry(m) => Error::AllFailed(m),
    }
}

#[allow(clippy::too_many_arguments)]
async fn worker(
    job: Arc<Job>,
    shared: Arc<Shared>,
    rs: Arc<RouteState>,
    client: Client,
    route: usize,
    stop: CancellationToken,
    emit: Emit,
    stall: Duration,
    max_attempts: usize,
) {
    loop {
        if stop.is_cancelled() || rs.stop.is_cancelled() || job.fatal.lock().unwrap().is_some() {
            return;
        }
        let Some(i) = job.pending.lock().unwrap().pop_front() else {
            // Nothing queued: finished, or another worker holds the last segments (and may hand one back).
            if job.done.load(Ordering::Relaxed) >= job.segments.len() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            continue;
        };
        rs.active.fetch_add(1, Ordering::Relaxed);
        let res = fetch_segment(&job, &shared, &rs, &client, i, &stop, stall).await;
        rs.active.fetch_sub(1, Ordering::Relaxed);
        match res {
            Ok(len) => {
                let done = job.done.fetch_add(1, Ordering::Relaxed) + 1;
                let total_bytes = job.bytes_done.fetch_add(len, Ordering::Relaxed) + len;
                // Estimate the whole size from the average so far, so progress and time left make sense.
                shared.total.store(total_bytes * job.segments.len() as u64 / done as u64, Ordering::Relaxed);
                emit(Event::ChunkDone { idx: i, route });
            }
            Err(SegErr::Cancelled) => {
                job.pending.lock().unwrap().push_front(i);
                return;
            }
            Err(SegErr::Fatal(e)) => {
                *job.fatal.lock().unwrap() = Some(e);
                stop.cancel();
                return;
            }
            Err(SegErr::Retry(msg)) => {
                let a = job.attempts[i].fetch_add(1, Ordering::Relaxed) + 1;
                if a >= max_attempts {
                    *job.fatal.lock().unwrap() = Some(Error::AllFailed(format!("segment {}: {msg}", i + 1)));
                    stop.cancel();
                    return;
                }
                job.pending.lock().unwrap().push_back(i);
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(250u64 << a.min(5)).min(Duration::from_secs(8))) => {}
                    _ = stop.cancelled() => return,
                }
            }
        }
    }
}

async fn fetch_segment(
    job: &Job,
    shared: &Shared,
    rs: &RouteState,
    client: &Client,
    i: usize,
    stop: &CancellationToken,
    stall: Duration,
) -> std::result::Result<u64, SegErr> {
    let seg = &job.segments[i];
    let mut data = get_bytes(client, &seg.url, job.headers[i].clone(), seg.range, stall, stop).await?;
    let wire = data.len() as u64;
    // Count the bytes against the speed limits, like any other transfer.
    let limiter: Option<&Arc<Limiter>> = shared.limit.as_ref();
    if let Some(l) = limiter {
        l.acquire(wire).await;
    }
    if let Some(l) = &rs.limit {
        l.acquire(wire).await;
    }
    rs.bytes.fetch_add(wire, Ordering::Relaxed);
    if let Some(k) = &seg.key {
        let key = job.keys.get(&k.uri).ok_or_else(|| SegErr::Fatal(Error::Other("a stream key is missing".into())))?;
        data = decrypt(&data, key, k.iv, seg.seq).map_err(SegErr::Fatal)?;
    }
    let (tmp, dest) = (job.dir.join(format!("seg-{i:06}.part")), seg_path(&job.dir, i));
    std::fs::write(&tmp, &data).map_err(|e| SegErr::Fatal(e.into()))?;
    std::fs::rename(&tmp, &dest).map_err(|e| SegErr::Fatal(e.into()))?;
    shared.settled.fetch_add(data.len() as u64, Ordering::Relaxed);
    Ok(data.len() as u64)
}

/// Join the segments in order (init section first), optionally remux with ffmpeg, and clean up.
async fn finish(opts: &Options, job: &Job, init: Option<Vec<u8>>, filename: &str, _ext: &str) -> Result<PathBuf> {
    let staging = opts.dest_dir.join(format!("{filename}.grabnr"));
    {
        use std::io::Write;
        let mut out = std::io::BufWriter::new(std::fs::File::create(&staging)?);
        if let Some(b) = &init {
            out.write_all(b)?;
        }
        for i in 0..job.segments.len() {
            let mut f = std::fs::File::open(seg_path(&job.dir, i))?;
            std::io::copy(&mut f, &mut out)?;
        }
        out.flush()?;
        out.get_ref().sync_all()?;
    }
    crate::download::verify_checksum_path(opts, &staging)?;
    let mut dest = unique_path(&opts.dest_dir, filename);
    std::fs::rename(&staging, &dest)?;
    let _ = std::fs::remove_dir_all(&job.dir);

    // Make a plain MP4 when ffmpeg is available: a transport stream (.ts) does not play in every player.
    if let (Some(ff), true) = (&opts.ffmpeg, dest.extension().is_some_and(|e| e == "ts")) {
        let mp4 = unique_path(
            &opts.dest_dir,
            &dest.with_extension("mp4").file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "video.mp4".into()),
        );
        let ok = tokio::process::Command::new(ff)
            .args(["-y", "-loglevel", "error", "-i"])
            .arg(&dest)
            .args(["-c", "copy"])
            .arg(&mp4)
            .stdin(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);
        if ok && std::fs::metadata(&mp4).map(|m| m.len() > 0).unwrap_or(false) {
            let _ = std::fs::remove_file(&dest);
            dest = mp4;
        } else {
            let _ = std::fs::remove_file(&mp4);
        }
    }
    let _ = Instant::now();
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=2800000,RESOLUTION=1280x720,CODECS=\"avc1.4d401f,mp4a.40.2\"\nhigh/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=1400000,RESOLUTION=854x480\nhttps://cdn.example/mid.m3u8\n";

    #[test]
    fn master_playlists_list_variants_and_pick_by_quality() {
        let Playlist::Master(v) = parse(MASTER, "https://v.example/a/master.m3u8").unwrap() else { panic!() };
        assert_eq!(v.len(), 3);
        assert_eq!(v[1].url, "https://v.example/a/high/index.m3u8");
        assert_eq!(v[2].url, "https://cdn.example/mid.m3u8");
        assert_eq!(v[1].label, "720p · 2.8 Mbps");
        assert_eq!(choose(&v, Quality::Best).unwrap().height, Some(720));
        assert_eq!(choose(&v, Quality::Worst).unwrap().height, Some(360));
        assert_eq!(choose(&v, Quality::Height(500)).unwrap().height, Some(480));
        assert_eq!(Quality::parse("720p"), Quality::Height(720));
        assert_eq!(Quality::parse("worst"), Quality::Worst);
        assert_eq!(Quality::parse("nonsense"), Quality::Best);
    }

    #[test]
    fn media_playlists_with_keys_ranges_and_init() {
        let text = "#EXTM3U\n#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-MAP:URI=\"init.mp4\",BYTERANGE=\"720@0\"\n#EXT-X-KEY:METHOD=AES-128,URI=\"keys/k.bin\",IV=0x000102030405060708090A0B0C0D0E0F\n#EXTINF:4.0,\nseg0.ts\n#EXT-X-KEY:METHOD=NONE\n#EXTINF:3.5,\n#EXT-X-BYTERANGE:1000@2000\nbig.ts\n#EXTINF:3.5,\n#EXT-X-BYTERANGE:500\nbig.ts\n#EXT-X-ENDLIST\n";
        let Playlist::Media(m) = parse(text, "https://v.example/p/list.m3u8").unwrap() else { panic!() };
        assert!(m.endlist);
        assert_eq!(m.init.as_ref().unwrap().range, Some((720, 0)));
        assert_eq!(m.segments.len(), 3);
        assert_eq!(m.segments[0].seq, 7);
        let k = m.segments[0].key.as_ref().unwrap();
        assert_eq!(k.uri, "https://v.example/p/keys/k.bin");
        assert_eq!(k.iv.unwrap()[15], 0x0F);
        assert!(m.segments[1].key.is_none(), "METHOD=NONE ends encryption");
        assert_eq!(m.segments[1].range, Some((1000, 2000)));
        assert_eq!(m.segments[2].range, Some((500, 3000)), "a range without an offset continues where the last one ended");
        assert_eq!(m.segments[0].duration, 4.0);
    }

    #[test]
    fn unsupported_and_invalid_playlists_are_refused_with_a_reason() {
        assert!(parse("<html>", "https://x/a.m3u8").is_err());
        let sample = "#EXTM3U\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"k\"\n#EXTINF:1,\na.ts\n#EXT-X-ENDLIST\n";
        let e = parse(sample, "https://x/a.m3u8").unwrap_err().to_string();
        assert!(e.contains("SAMPLE-AES"), "{e}");
        assert!(parse("#EXTM3U\n#EXT-X-ENDLIST\n", "https://x/a.m3u8").is_err());
        let live = parse("#EXTM3U\n#EXTINF:2,\na.ts\n", "https://x/a.m3u8").unwrap();
        assert!(matches!(live, Playlist::Media(m) if !m.endlist));
    }

    #[test]
    fn detects_playlist_links() {
        assert!(looks_like("https://x/live/stream.m3u8?token=1"));
        assert!(!looks_like("https://x/file.mp4"));
    }

    #[test]
    fn decrypts_aes_128_with_a_given_or_derived_iv() {
        use aes::cipher::BlockEncryptMut;
        type Enc = cbc::Encryptor<aes::Aes128>;
        let key = [7u8; 16];
        let plain = b"hello transport stream bytes, more than a block";
        let explicit = [9u8; 16];
        let ct = Enc::new(&key.into(), &explicit.into()).encrypt_padded_vec_mut::<Pkcs7>(plain);
        assert_eq!(decrypt(&ct, &key, Some(explicit), 0).unwrap(), plain);
        let mut derived = [0u8; 16];
        derived[8..].copy_from_slice(&42u64.to_be_bytes());
        let ct = Enc::new(&key.into(), &derived.into()).encrypt_padded_vec_mut::<Pkcs7>(plain);
        assert_eq!(decrypt(&ct, &key, None, 42).unwrap(), plain, "without an IV the sequence number is used");
        assert!(decrypt(&ct, &key, None, 43).is_err() || decrypt(&ct, &key, None, 43).unwrap() != plain);
    }

    #[test]
    fn output_names_come_from_the_playlist() {
        assert_eq!(out_name("https://x/path/My%20Show.m3u8?x=1", "ts"), "My%20Show.ts");
        assert_eq!(out_name("https://x/", "ts"), "video.ts");
    }
}
