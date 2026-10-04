use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

use grabnr_core::{download, Error, Event, Options, Route, Store};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

const SIZE: u64 = 5 * 1024 * 1024 + 123;

struct Server {
    port: u16,
    version: Arc<AtomicU64>,
    fail_first: Arc<AtomicUsize>,
    bytes_served: Arc<AtomicU64>,
    /// When set, requests must carry exactly this Authorization header (compared lowercased).
    need_auth: Arc<std::sync::Mutex<Option<String>>>,
    /// The next N range requests send headers and then go silent, like a dead connection after sleep.
    stall_next: Arc<AtomicUsize>,
}

fn byte(i: u64, version: u64) -> u8 {
    ((i * 7 + 3 + version) % 251) as u8
}

fn expected(version: u64) -> Vec<u8> {
    (0..SIZE).map(|i| byte(i, version)).collect()
}

async fn serve(ranges: bool) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let version = Arc::new(AtomicU64::new(0));
    let fail_first = Arc::new(AtomicUsize::new(0));
    let bytes_served = Arc::new(AtomicU64::new(0));
    let need_auth: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let stall_next = Arc::new(AtomicUsize::new(0));
    let (v, f, b, na, sn) = (version.clone(), fail_first.clone(), bytes_served.clone(), need_auth.clone(), stall_next.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (v, f, b, na, sn) = (v.clone(), f.clone(), b.clone(), na.clone(), sn.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&buf).to_lowercase();
                let header = |name: &str| req.lines().find_map(|l| l.strip_prefix(&format!("{name}: "))).map(str::to_owned);
                let want = na.lock().unwrap().clone();
                if let Some(want) = want {
                    if header("authorization").as_deref() != Some(want.to_lowercase().as_str()) {
                        let _ = sock.write_all(b"HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                        return;
                    }
                }
                let ver = v.load(Ordering::Relaxed);
                let etag = format!("\"v{ver}\"");

                if f.load(Ordering::Relaxed) > 0 && header("range").map(|r| r != "bytes=0-0").unwrap_or(false) {
                    f.fetch_sub(1, Ordering::Relaxed);
                    let _ = sock.write_all(b"HTTP/1.1 503 Service Unavailable\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                    return;
                }
                let range = if ranges { header("range") } else { None };
                let if_range_ok = header("if-range").map(|v| v == etag).unwrap_or(true);
                let (status, start, end, extra) = match range.filter(|_| if_range_ok).and_then(|r| {
                    let r = r.strip_prefix("bytes=")?.to_string();
                    let (a, b) = r.split_once('-')?;
                    Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok().unwrap_or(SIZE - 1).min(SIZE - 1)))
                }) {
                    Some((a, b)) => ("206 Partial Content", a, b, format!("content-range: bytes {a}-{b}/{SIZE}\r\n")),
                    None => ("200 OK", 0, SIZE - 1, String::new()),
                };
                let body: Vec<u8> = (start..=end).map(|i| byte(i, ver)).collect();
                let head = format!(
                    "HTTP/1.1 {status}\r\ncontent-length: {}\r\netag: {etag}\r\n{extra}{}connection: close\r\n\r\n",
                    body.len(),
                    if ranges { "accept-ranges: bytes\r\n" } else { "" }
                );
                let _ = sock.write_all(head.as_bytes()).await;
                if header("range").map(|r| r != "bytes=0-0").unwrap_or(false) && sn.load(Ordering::Relaxed) > 0 {
                    sn.fetch_sub(1, Ordering::Relaxed);
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    return;
                }
                // small writes so a cancel can land mid-chunk
                for part in body.chunks(64 * 1024) {
                    if sock.write_all(part).await.is_err() {
                        return;
                    }
                    b.fetch_add(part.len() as u64, Ordering::Relaxed);
                }
            });
        }
    });
    Server { port, version, fail_first, bytes_served, need_auth, stall_next }
}

fn silent() -> Arc<dyn Fn(Event) + Send + Sync> {
    Arc::new(|_| {})
}

fn two_routes() -> Vec<Route> {
    vec![Route::unbound("a"), Route::unbound("b")]
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("grabnr-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_a_file_across_two_routes() {
    let s = serve(true).await;
    let dir = tmp("basic");
    let seen_routes = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let sr = seen_routes.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ChunkDone { route, .. } = e {
            sr.lock().unwrap().insert(route);
        }
    });
    let mut o = Options::new(format!("http://127.0.0.1:{}/file.bin", s.port), &dir, two_routes());
    o.conns_per_route = 2;
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(path.file_name().unwrap(), "file.bin");
    assert_eq!(std::fs::read(&path).unwrap(), expected(0));
    assert!(!dir.join("file.bin.grabnr").exists(), "staging file must be renamed away");
    assert_eq!(seen_routes.lock().unwrap().len(), 2, "both routes should have fetched chunks");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retries_after_503() {
    let s = serve(true).await;
    s.fail_first.store(3, Ordering::Relaxed);
    let dir = tmp("retry");
    let mut o = Options::new(format!("http://127.0.0.1:{}/r.bin", s.port), &dir, two_routes());
    o.conns_per_route = 2;
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn falls_back_to_one_stream_without_range_support() {
    let s = serve(false).await;
    let dir = tmp("norange");
    let o = Options::new(format!("http://127.0.0.1:{}/n.bin", s.port), &dir, two_routes());
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
}

async fn cancel_after_chunks(o: Options, n: usize) -> Error {
    let cancel = CancellationToken::new();
    let (c, count) = (cancel.clone(), Arc::new(AtomicUsize::new(0)));
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ChunkDone { .. } = e {
            if count.fetch_add(1, Ordering::Relaxed) + 1 == n {
                c.cancel();
            }
        }
    });
    download(o, cancel, emit).await.unwrap_err()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumes_without_refetching_finished_chunks() {
    let s = serve(true).await;
    let dir = tmp("resume");
    let store = Arc::new(Store::in_memory().unwrap());
    let url = format!("http://127.0.0.1:{}/x.bin", s.port);
    let mk = || {
        let mut o = Options::new(url.clone(), &dir, two_routes());
        o.conns_per_route = 1;
        o.store = Some(store.clone());
        o
    };
    assert!(matches!(cancel_after_chunks(mk(), 2).await, Error::Cancelled));
    assert!(dir.join("x.bin.grabnr").exists(), "partial file must be kept for resume");
    let before = s.bytes_served.load(Ordering::Relaxed);

    let resumed = Arc::new(AtomicUsize::new(0));
    let r = resumed.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::Started { resumed_chunks, .. } = e {
            r.store(resumed_chunks, Ordering::Relaxed);
        }
    });
    let path = download(mk(), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    assert!(resumed.load(Ordering::Relaxed) >= 2, "should reuse the chunks finished before cancel");
    let second_run = s.bytes_served.load(Ordering::Relaxed) - before;
    assert!(second_run < SIZE, "resume must not download the whole file again ({second_run} bytes)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn discards_partial_file_when_server_file_changed() {
    let s = serve(true).await;
    let dir = tmp("changed");
    let store = Arc::new(Store::in_memory().unwrap());
    let url = format!("http://127.0.0.1:{}/c.bin", s.port);
    let mk = || {
        let mut o = Options::new(url.clone(), &dir, two_routes());
        o.conns_per_route = 1;
        o.store = Some(store.clone());
        o
    };
    assert!(matches!(cancel_after_chunks(mk(), 2).await, Error::Cancelled));
    s.version.store(1, Ordering::Relaxed); // new etag, different bytes

    let discarded = Arc::new(AtomicUsize::new(0));
    let d = discarded.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ResumeDiscarded { .. } = e {
            d.fetch_add(1, Ordering::Relaxed);
        }
    });
    let path = download(mk(), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(discarded.load(Ordering::Relaxed), 1);
    assert_eq!(std::fs::read(path).unwrap(), expected(1), "must not mix bytes from two versions");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn speed_limit_slows_the_download() {
    let s = serve(true).await;
    let dir = tmp("limit");
    let mut o = Options::new(format!("http://127.0.0.1:{}/l.bin", s.port), &dir, two_routes());
    o.speed_limit = Some(3 * 1024 * 1024); // 5 MB at 3 MB/s takes well over a second
    let t = std::time::Instant::now();
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert!(t.elapsed() >= std::time::Duration::from_millis(1200), "finished in {:?}", t.elapsed());
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_link_limit_pushes_work_to_the_other_link() {
    let s = serve(true).await;
    let dir = tmp("linklimit");
    let mut routes = two_routes();
    routes[0].speed_limit = Some(256 * 1024);
    let by_route = Arc::new(std::sync::Mutex::new([0usize; 2]));
    let br = by_route.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ChunkDone { route, .. } = e {
            br.lock().unwrap()[route] += 1;
        }
    });
    let mut o = Options::new(format!("http://127.0.0.1:{}/pl.bin", s.port), &dir, routes);
    o.conns_per_route = 2;
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    let c = *by_route.lock().unwrap();
    assert!(c[1] > c[0], "the unlimited link should carry most chunks: {c:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verifies_checksum_and_rejects_a_bad_one() {
    let s = serve(true).await;
    let url = format!("http://127.0.0.1:{}/h.bin", s.port);
    let good = grabnr_core::checksum::compute_bytes(grabnr_core::Algo::Sha256, &expected(0));

    let dir = tmp("sum-ok");
    let mut o = Options::new(&url, &dir, two_routes());
    o.checksum = Some(grabnr_core::Checksum::parse(&good).unwrap());
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert!(path.exists());

    let dir = tmp("sum-bad");
    let mut o = Options::new(&url, &dir, two_routes());
    o.checksum = Some(grabnr_core::Checksum::parse(&"0".repeat(64)).unwrap());
    let err = download(o, CancellationToken::new(), silent()).await.unwrap_err();
    assert!(matches!(err, Error::ChecksumMismatch { .. }), "{err}");
    assert!(!dir.join("h.bin").exists() && !dir.join("h.bin.grabnr").exists(), "a corrupt file must not be left behind");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sends_basic_and_bearer_credentials() {
    let s = serve(true).await;
    let url = format!("http://127.0.0.1:{}/a.bin", s.port);

    *s.need_auth.lock().unwrap() = Some("Basic dTpw".into());
    let mut o = Options::new(&url, tmp("auth-none"), two_routes());
    let err = download(o, CancellationToken::new(), silent()).await.unwrap_err();
    assert!(matches!(err, Error::Status(401)), "{err}");

    o = Options::new(&url, tmp("auth-basic"), two_routes());
    o.auth = Some(grabnr_core::Auth::Basic { user: "u".into(), pass: "p".into() });
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));

    *s.need_auth.lock().unwrap() = Some("Bearer tok".into());
    o = Options::new(&url, tmp("auth-bearer"), two_routes());
    o.auth = Some(grabnr_core::Auth::Bearer("tok".into()));
    assert!(download(o, CancellationToken::new(), silent()).await.is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_through_an_http_proxy() {
    // The test server answers any request line, so it doubles as a proxy: the target host below does not exist.
    let proxy = serve(true).await;
    let mut o = Options::new("http://files.invalid/p.bin", tmp("proxy"), two_routes());
    o.proxy = Some(format!("http://127.0.0.1:{}", proxy.port));
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));

    let direct = Options::new("http://files.invalid/p.bin", tmp("noproxy"), two_routes());
    assert!(download(direct, CancellationToken::new(), silent()).await.is_err(), "without the proxy the host cannot be reached");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_can_be_limited_to_fewer_connections() {
    let s = serve(true).await;
    let mut routes = two_routes();
    routes[0].max_conns = Some(1);
    let peak = Arc::new(AtomicUsize::new(0));
    let p = peak.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::Progress(snap) = e {
            p.fetch_max(snap.routes[0].connections, Ordering::Relaxed);
        }
    });
    let mut o = Options::new(format!("http://127.0.0.1:{}/m.bin", s.port), tmp("maxconns"), routes);
    o.conns_per_route = 6;
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    assert!(peak.load(Ordering::Relaxed) <= 1, "link a was capped at one connection");
}

fn watch(f: impl Fn(std::time::Duration) -> Vec<Route> + Send + Sync + 'static) -> grabnr_core::download::LinkWatch {
    let t0 = std::time::Instant::now();
    Arc::new(move || f(t0.elapsed()))
}

fn slow_options(url: String, name: &str, routes: Vec<Route>) -> Options {
    let mut o = Options::new(url, tmp(name), routes);
    o.conns_per_route = 1; // keep chunks queued so a joining link has something to take
    o.speed_limit = Some(2 * 1024 * 1024); // ~2.5 s for the file
    o.watch_interval = std::time::Duration::from_millis(100);
    o
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_that_appears_joins_the_running_download() {
    let s = serve(true).await;
    let mut o = slow_options(format!("http://127.0.0.1:{}/j.bin", s.port), "join", vec![Route::unbound("a")]);
    o.link_watch = Some(watch(|t| if t < std::time::Duration::from_millis(500) { vec![Route::unbound("a")] } else { two_routes() }));
    let seen = Arc::new(std::sync::Mutex::new((vec![0usize; 2], Vec::<String>::new())));
    let sn = seen.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| match e {
        Event::ChunkDone { route, .. } => sn.lock().unwrap().0[route] += 1,
        Event::RouteUp { name, .. } => sn.lock().unwrap().1.push(name),
        _ => {}
    });
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    let (chunks, ups) = seen.lock().unwrap().clone();
    assert_eq!(ups, vec!["b".to_string()]);
    assert!(chunks[1] >= 1, "the new link should have fetched chunks: {chunks:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_link_that_vanishes_hands_its_chunks_to_the_others() {
    let s = serve(true).await;
    let mut o = slow_options(format!("http://127.0.0.1:{}/v.bin", s.port), "vanish", two_routes());
    o.link_watch = Some(watch(|t| if t < std::time::Duration::from_millis(400) { two_routes() } else { vec![Route::unbound("a")] }));
    let downs = Arc::new(std::sync::Mutex::new(Vec::new()));
    let d = downs.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::RouteDown { route, .. } = e {
            d.lock().unwrap().push(route);
        }
    });
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0), "no bytes may be lost when a link drops mid-chunk");
    assert_eq!(*downs.lock().unwrap(), vec![1]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn survives_every_link_disappearing_for_a_while() {
    let s = serve(true).await;
    let mut o = slow_options(format!("http://127.0.0.1:{}/g.bin", s.port), "gap", vec![Route::unbound("a")]);
    o.link_watch = Some(watch(|t| {
        let ms = t.as_millis();
        if (300..1000).contains(&ms) { vec![] } else { vec![Route::unbound("a")] }
    }));
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gives_up_when_no_link_returns_within_the_grace_period() {
    let s = serve(true).await;
    let mut o = slow_options(format!("http://127.0.0.1:{}/x2.bin", s.port), "nolink", vec![Route::unbound("a")]);
    o.link_watch = Some(watch(|t| if t < std::time::Duration::from_millis(300) { vec![Route::unbound("a")] } else { vec![] }));
    o.no_link_grace = std::time::Duration::from_millis(800);
    let err = download(o, CancellationToken::new(), silent()).await.unwrap_err();
    assert!(matches!(err, Error::AllFailed(_)), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn recovers_from_connections_that_go_silent() {
    let s = serve(true).await;
    s.stall_next.store(2, Ordering::Relaxed);
    let mut o = Options::new(format!("http://127.0.0.1:{}/s.bin", s.port), tmp("stall"), two_routes());
    o.conns_per_route = 2;
    o.stall_timeout = std::time::Duration::from_millis(600);
    let t = std::time::Instant::now();
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    assert!(t.elapsed() < std::time::Duration::from_secs(20), "took {:?}", t.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spreads_chunks_over_mirrors_and_skips_dead_ones() {
    let a = serve(true).await;
    let b = serve(true).await;
    let mut o = Options::new(format!("http://127.0.0.1:{}/m.bin", a.port), tmp("mirrors"), two_routes());
    o.conns_per_route = 2;
    o.mirrors = vec![format!("http://127.0.0.1:{}/m.bin", b.port), "http://127.0.0.1:1/dead.bin".into()];
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    let (from_a, from_b) = (a.bytes_served.load(Ordering::Relaxed), b.bytes_served.load(Ordering::Relaxed));
    assert!(from_a > 0 && from_b > 0, "both servers should have served data: a={from_a} b={from_b}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_mirror_that_fails_midway_does_not_break_the_download() {
    let a = serve(true).await;
    let b = serve(true).await;
    b.fail_first.store(1000, Ordering::Relaxed); // every range request to the mirror gets a 503
    let mut o = Options::new(format!("http://127.0.0.1:{}/f.bin", a.port), tmp("badmirror"), two_routes());
    o.conns_per_route = 2;
    o.mirrors = vec![format!("http://127.0.0.1:{}/f.bin", b.port)];
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
}
