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
    let (v, f, b) = (version.clone(), fail_first.clone(), bytes_served.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (v, f, b) = (v.clone(), f.clone(), b.clone());
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
    Server { port, version, fail_first, bytes_served }
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
