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
    /// (Host header, had credentials) for every non-probe request.
    seen: Arc<std::sync::Mutex<Vec<(String, bool)>>>,
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
    let seen: Arc<std::sync::Mutex<Vec<(String, bool)>>> = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (v, f, b, na, sn, sc) =
        (version.clone(), fail_first.clone(), bytes_served.clone(), need_auth.clone(), stall_next.clone(), seen.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (v, f, b, na, sn, sc) = (v.clone(), f.clone(), b.clone(), na.clone(), sn.clone(), sc.clone());
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
                if header("range").map(|r| r != "bytes=0-0").unwrap_or(false) {
                    sc.lock()
                        .unwrap()
                        .push((header("host").unwrap_or_default(), header("authorization").is_some() || header("cookie").is_some()));
                }
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
    Server { port, version, fail_first, bytes_served, need_auth, stall_next, seen }
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
        if (300..1000).contains(&ms) {
            vec![]
        } else {
            vec![Route::unbound("a")]
        }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn credentials_are_not_sent_to_a_mirror_on_another_host() {
    let s = serve(true).await;
    // Same server, reached under a different host name: to the client that is another origin.
    let mut o = Options::new(format!("http://127.0.0.1:{}/c.bin", s.port), tmp("leak"), two_routes());
    o.conns_per_route = 2;
    o.auth = Some(grabnr_core::Auth::Bearer("secret".into()));
    o.headers.push(("Cookie".into(), "session=1".into()));
    o.mirrors = vec![format!("http://localhost:{}/c.bin", s.port)];
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    let seen = s.seen.lock().unwrap().clone();
    assert!(seen.iter().any(|(h, auth)| h.starts_with("127.0.0.1") && *auth), "the original host should get the credentials: {seen:?}");
    let leaked: Vec<_> = seen.iter().filter(|(h, auth)| h.starts_with("localhost") && *auth).collect();
    assert!(leaked.is_empty(), "credentials leaked to the mirror host: {leaked:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_share_one_limiter() {
    let s = serve(true).await;
    let limiter = Arc::new(grabnr_core::limiter::Limiter::new(4 * 1024 * 1024));
    let mk = |n: &str| {
        let mut o = Options::new(format!("http://127.0.0.1:{}/{n}.bin", s.port), tmp(n), two_routes());
        o.shared_limit = Some(limiter.clone());
        o
    };
    let t = std::time::Instant::now();
    let (a, b) =
        tokio::join!(download(mk("sa"), CancellationToken::new(), silent()), download(mk("sb"), CancellationToken::new(), silent()));
    assert_eq!(std::fs::read(a.unwrap()).unwrap(), expected(0));
    assert_eq!(std::fs::read(b.unwrap()).unwrap(), expected(0));
    // 10 MB through a 4 MB/s budget; two independent limiters would finish in about 1.3 s.
    assert!(t.elapsed() >= std::time::Duration::from_millis(2000), "finished in {:?}", t.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_limit_can_be_lifted_while_downloading() {
    let s = serve(true).await;
    let limiter = Arc::new(grabnr_core::limiter::Limiter::new(1024 * 1024)); // 5 MB would take about 5 s
    let mut o = Options::new(format!("http://127.0.0.1:{}/lift.bin", s.port), tmp("lift"), two_routes());
    o.shared_limit = Some(limiter.clone());
    let l = limiter.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        l.set_rate(0);
    });
    let t = std::time::Instant::now();
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    assert!(t.elapsed() < std::time::Duration::from_millis(3000), "still throttled: {:?}", t.elapsed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn posts_json_to_a_webhook() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let got = Arc::new(std::sync::Mutex::new(String::new()));
    let g = got.clone();
    tokio::spawn(async move {
        let (mut sock, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 8192];
        let mut all = Vec::new();
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            all.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&all).to_string();
            if let Some((head, body)) = text.split_once("\r\n\r\n") {
                let len: usize = head
                    .to_lowercase()
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                if body.len() >= len {
                    *g.lock().unwrap() = text.clone();
                    break;
                }
            }
        }
        let _ = sock.write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n").await;
    });
    let status = grabnr_core::fetch::post_json(
        &format!("http://127.0.0.1:{port}/hook"),
        r#"{"event":"download.completed"}"#.into(),
        None,
        std::time::Duration::from_secs(5),
    )
    .await
    .unwrap();
    assert_eq!(status, 204);
    let req = got.lock().unwrap().to_lowercase();
    assert!(req.starts_with("post /hook"), "{req}");
    assert!(req.contains("content-type: application/json"));
    assert!(req.ends_with(r#"{"event":"download.completed"}"#));

    assert!(grabnr_core::fetch::post_json("ftp://x/y", "{}".into(), None, std::time::Duration::from_secs(1)).await.is_err());
}

fn resume_options(url: &str, dir: &std::path::Path, store: &Arc<Store>, key: Option<&str>) -> Options {
    let mut o = Options::new(url, dir, two_routes());
    o.conns_per_route = 1;
    o.store = Some(store.clone());
    o.resume_key = key.map(str::to_owned);
    o
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_damaged_finished_piece_is_downloaded_again_on_resume() {
    let s = serve(true).await;
    let dir = tmp("damaged");
    let store = Arc::new(Store::in_memory().unwrap());
    let url = format!("http://127.0.0.1:{}/d.bin", s.port);
    assert!(matches!(cancel_after_chunks(resume_options(&url, &dir, &store, None), 3).await, Error::Cancelled));

    // Simulate a crash that lost data the database already counted as done: zero the start of the file.
    let staging = dir.join("d.bin.grabnr");
    let mut bytes = std::fs::read(&staging).unwrap();
    bytes[..4096].fill(0);
    std::fs::write(&staging, bytes).unwrap();

    let seen = Arc::new(std::sync::Mutex::new((0usize, 0usize)));
    let sn = seen.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ResumeChecked { checked, redo } = e {
            *sn.lock().unwrap() = (checked, redo);
        }
    });
    let path = download(resume_options(&url, &dir, &store, None), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0), "the zeroed piece must not survive into the final file");
    let (checked, redo) = *seen.lock().unwrap();
    assert!(checked >= 3 && redo >= 1, "checked {checked}, redo {redo}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn untouched_pieces_pass_the_check_and_are_kept() {
    let s = serve(true).await;
    let dir = tmp("intact");
    let store = Arc::new(Store::in_memory().unwrap());
    let url = format!("http://127.0.0.1:{}/i.bin", s.port);
    assert!(matches!(cancel_after_chunks(resume_options(&url, &dir, &store, None), 2).await, Error::Cancelled));
    let redo = Arc::new(AtomicUsize::new(99));
    let r = redo.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ResumeChecked { redo, .. } = e {
            r.store(redo, Ordering::Relaxed);
        }
    });
    let path = download(resume_options(&url, &dir, &store, None), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    assert_eq!(redo.load(Ordering::Relaxed), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_replaced_link_resumes_the_same_partial_file() {
    let s = serve(true).await;
    let dir = tmp("newlink");
    let store = Arc::new(Store::in_memory().unwrap());
    let old = format!("http://127.0.0.1:{}/old-signed-url.bin?sig=expired", s.port);
    let new = format!("http://127.0.0.1:{}/old-signed-url.bin?sig=fresh", s.port);
    assert!(matches!(cancel_after_chunks(resume_options(&old, &dir, &store, Some("item-1")), 2).await, Error::Cancelled));

    let resumed = Arc::new(AtomicUsize::new(0));
    let r = resumed.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::Started { resumed_chunks, .. } = e {
            r.store(resumed_chunks, Ordering::Relaxed);
        }
    });
    let path = download(resume_options(&new, &dir, &store, Some("item-1")), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    assert!(resumed.load(Ordering::Relaxed) >= 2, "the new link should keep the chunks already on disk");
}

/// One link with one connection: pieces are fetched one at a time and nothing is raced, so byte counts are exact.
fn single_link(url: &str, dir: &std::path::Path, store: &Arc<Store>) -> Options {
    let mut o = Options::new(url, dir, vec![Route::unbound("a")]);
    o.conns_per_route = 1;
    o.store = Some(store.clone());
    o
}

fn key(url: &str, dir: &std::path::Path, name: &str) -> String {
    format!("{url}|{}", dir.join(name).display())
}

fn crc(data: &[u8]) -> u32 {
    let mut h = crc32fast::Hasher::new();
    h.update(data);
    h.finalize()
}

/// Leave the world as an interrupted download would: piece 0 finished, piece 1 written up to `kept` bytes and saved
/// as partial. Returns the size of a piece.
async fn interrupted_state(url: &str, dir: &std::path::Path, store: &Arc<Store>, name: &str, kept: u64) -> u64 {
    assert!(matches!(cancel_after_chunks(single_link(url, dir, store), 1).await, Error::Cancelled));
    let k = key(url, dir, name);
    let (rec, saved) = store.load(&k).unwrap().unwrap();
    let chunk = rec.chunk_size;
    // Whatever else the cancelled run managed to finish, forget it: the state below is the only progress.
    let others: Vec<usize> = saved.done.iter().map(|d| d.0).filter(|&i| i != 0).collect();
    store.forget(&k, &others).unwrap();
    store.forget_partial(&k, &[1]).unwrap();
    let staging = dir.join(name);
    let mut file = std::fs::read(&staging).unwrap();
    let want = expected(0);
    // Data after piece 0 is whatever the cancelled run left; make piece 1 hold exactly `kept` good bytes, the rest zero.
    file[chunk as usize..2 * chunk as usize].fill(0);
    file[chunk as usize..(chunk + kept) as usize].copy_from_slice(&want[chunk as usize..(chunk + kept) as usize]);
    std::fs::write(&staging, &file).unwrap();
    store.set_partial(&k, 1, kept, crc(&want[chunk as usize..(chunk + kept) as usize])).unwrap();
    chunk
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumes_inside_a_piece_instead_of_refetching_it() {
    let s = serve(true).await;
    let dir = tmp("inpiece");
    let store = Arc::new(Store::in_memory().unwrap());
    let url = format!("http://127.0.0.1:{}/p.bin", s.port);
    let kept = 600_000;
    let chunk = interrupted_state(&url, &dir, &store, "p.bin.grabnr", kept).await;
    let on_disk = chunk + kept; // piece 0 in full plus the saved part of piece 1

    tokio::time::sleep(std::time::Duration::from_millis(200)).await; // let the server finish what it had queued
    let before = s.bytes_served.load(Ordering::Relaxed);
    let (checked, redo) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(99)));
    let (max_shown, first_shown) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(u64::MAX)));
    let (c, r, m, f) = (checked.clone(), redo.clone(), max_shown.clone(), first_shown.clone());
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| match e {
        Event::ResumeChecked { checked, redo } => {
            c.store(checked, Ordering::Relaxed);
            r.store(redo, Ordering::Relaxed);
        }
        Event::Progress(p) => {
            m.fetch_max(p.downloaded, Ordering::Relaxed);
            f.fetch_min(p.downloaded, Ordering::Relaxed);
        }
        _ => {}
    });
    let path = download(single_link(&url, &dir, &store), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0));
    assert_eq!(redo.load(Ordering::Relaxed), 0, "intact pieces pass the check");
    assert_eq!(checked.load(Ordering::Relaxed), 2, "one finished piece and one partial");
    let second = s.bytes_served.load(Ordering::Relaxed) - before;
    assert!(second.abs_diff(SIZE - on_disk) <= 4096, "fetched {second} bytes, expected the {} that were missing", SIZE - on_disk);
    assert!(max_shown.load(Ordering::Relaxed) <= SIZE, "progress must never exceed the file size");
    assert!(first_shown.load(Ordering::Relaxed) >= on_disk, "progress starts from what is already on disk, not from zero");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_damaged_partial_piece_is_fetched_from_its_start() {
    let s = serve(true).await;
    let dir = tmp("badpartial");
    let store = Arc::new(Store::in_memory().unwrap());
    let url = format!("http://127.0.0.1:{}/q.bin", s.port);
    let chunk = interrupted_state(&url, &dir, &store, "q.bin.grabnr", 600_000).await;

    // The saved part of piece 1 is damaged.
    let staging = dir.join("q.bin.grabnr");
    let mut bytes = std::fs::read(&staging).unwrap();
    bytes[chunk as usize..chunk as usize + 4096].fill(0);
    std::fs::write(&staging, bytes).unwrap();

    let redo = Arc::new(AtomicUsize::new(0));
    let r = redo.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ResumeChecked { redo, .. } = e {
            r.store(redo, Ordering::Relaxed);
        }
    });
    let path = download(single_link(&url, &dir, &store), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), expected(0), "the damaged part must be repaired");
    assert_eq!(redo.load(Ordering::Relaxed), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_interrupted_fetch_saves_how_far_it_got() {
    // Real interruption: cancel while a piece is part-way. Whether a piece happens to be mid-way at that instant depends
    // on timing, so try a few times; the run must save progress inside a piece at least once.
    for attempt in 0..8 {
        let s = serve(true).await;
        let dir = tmp(&format!("realpartial{attempt}"));
        let store = Arc::new(Store::in_memory().unwrap());
        let url = format!("http://127.0.0.1:{}/r.bin", s.port);
        let mut o = single_link(&url, &dir, &store);
        o.speed_limit = Some(1024 * 1024);
        let cancel = CancellationToken::new();
        let c = cancel.clone();
        let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
            if let Event::Progress(p) = e {
                if p.downloaded >= 400_000 {
                    c.cancel(); // first piece (1 MB) is still in flight
                }
            }
        });
        let err = download(o, cancel, emit).await.unwrap_err();
        assert!(matches!(err, Error::Cancelled), "{err}");
        let (_, saved) = store.load(&key(&url, &dir, "r.bin.grabnr")).unwrap().unwrap();
        if let Some(&(idx, len, _)) = saved.partial.first() {
            assert_eq!(idx, 0);
            assert!(len > 0 && len < 1 << 20, "{len}");
            let path = download(single_link(&url, &dir, &store), CancellationToken::new(), silent()).await.unwrap();
            assert_eq!(std::fs::read(path).unwrap(), expected(0));
            return;
        }
    }
    panic!("no attempt saved progress inside a piece");
}
