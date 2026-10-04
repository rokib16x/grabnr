//! HLS downloads against a local server that serves playlists and segments.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use grabnr_core::{download, Error, Event, Options, Quality, Route};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

struct Server {
    port: u16,
    files: Files,
    /// How many times each path was requested.
    hits: Arc<Mutex<HashMap<String, usize>>>,
    /// Milliseconds the server waits before answering each request.
    delay: Arc<AtomicU64>,
}

impl Server {
    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.port)
    }
    fn put(&self, path: &str, body: Vec<u8>) {
        self.files.lock().unwrap().insert(path.into(), body);
    }
    fn hits(&self, path: &str) -> usize {
        self.hits.lock().unwrap().get(path).copied().unwrap_or(0)
    }
}

async fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let files: Files = Arc::new(Mutex::new(HashMap::new()));
    let hits = Arc::new(Mutex::new(HashMap::new()));
    let delay = Arc::new(AtomicU64::new(15));
    let (f, h, d) = (files.clone(), hits.clone(), delay.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let (f, h, d) = (f.clone(), h.clone(), d.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 2048];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&buf).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap().to_string();
                *h.lock().unwrap().entry(path.clone()).or_insert(0) += 1;
                let range = req.to_lowercase().lines().find_map(|l| l.strip_prefix("range: bytes=").map(str::to_owned));
                let body = f.lock().unwrap().get(&path).cloned();
                let Some(body) = body else {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                    return;
                };
                tokio::time::sleep(std::time::Duration::from_millis(d.load(Ordering::Relaxed))).await;
                let (status, slice) = match range.and_then(|r| {
                    let (a, e) = r.split_once('-')?;
                    Some((a.parse::<usize>().ok()?, e.parse::<usize>().ok()?))
                }) {
                    Some((a, e)) => ("206 Partial Content", body[a..=e.min(body.len() - 1)].to_vec()),
                    None => ("200 OK", body),
                };
                let head = format!("HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", slice.len());
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&slice).await;
            });
        }
    });
    Server { port, files, hits, delay }
}

fn segment(i: usize) -> Vec<u8> {
    (0..40_000 + i * 700).map(|j| ((i * 13 + j * 7 + 5) % 251) as u8).collect()
}

fn playlist(n: usize, extra: &str, endlist: bool) -> String {
    let mut p = format!("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n{extra}");
    for i in 0..n {
        p.push_str(&format!("#EXTINF:4.0,\nseg{i}.ts\n"));
    }
    if endlist {
        p.push_str("#EXT-X-ENDLIST\n");
    }
    p
}

fn put_segments(s: &Server, prefix: &str, n: usize) -> Vec<u8> {
    let mut all = Vec::new();
    for i in 0..n {
        let seg = segment(i);
        s.put(&format!("{prefix}/seg{i}.ts"), seg.clone());
        all.extend(seg);
    }
    all
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("grabnr-hls-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn two_routes() -> Vec<Route> {
    vec![Route::unbound("a"), Route::unbound("b")]
}

fn silent() -> Arc<dyn Fn(Event) + Send + Sync> {
    Arc::new(|_| {})
}

fn options(url: String, name: &str) -> Options {
    let mut o = Options::new(url, tmp(name), two_routes());
    o.conns_per_route = 2;
    o
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_every_segment_in_order_over_both_links() {
    let s = serve().await;
    s.delay.store(150, Ordering::Relaxed); // slow enough for a progress report to arrive mid-download
    let all = put_segments(&s, "/v", 14);
    s.put("/v/index.m3u8", playlist(14, "", true).into_bytes());

    let seen = Arc::new(Mutex::new((Vec::<usize>::new(), [0usize; 2], 0u64, 0u64, 0usize, 0usize)));
    let sn = seen.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        let mut g = sn.lock().unwrap();
        match e {
            Event::Started { chunks, total, .. } => {
                g.4 = chunks;
                assert_eq!(total, None);
            }
            Event::ChunkDone { idx, route } => {
                g.0.push(idx);
                g.1[route] += 1;
            }
            Event::Progress(p) => {
                if let Some(t) = p.total {
                    g.2 = g.2.max(t);
                    assert!(p.downloaded <= t);
                    g.3 = g.3.max(p.downloaded);
                }
            }
            _ => {}
        }
    });
    let o = options(s.url("/v/index.m3u8"), "basic");
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(path.file_name().unwrap(), "index.ts");
    assert_eq!(std::fs::read(&path).unwrap(), all, "segments must be joined in playlist order");
    let g = seen.lock().unwrap();
    assert_eq!(g.4, 14);
    let mut done = g.0.clone();
    done.sort();
    assert_eq!(done, (0..14).collect::<Vec<_>>());
    assert!(g.1[0] > 0 && g.1[1] > 0, "both links should fetch segments: {:?}", g.1);
    assert!(g.2 > 0, "an estimated total should be reported once segments arrive");
    assert!(!path.with_file_name("index.ts.grabnr-hls").exists(), "working files are cleaned up");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_master_playlist_picks_the_requested_quality() {
    let s = serve().await;
    let _low = put_segments(&s, "/low", 4);
    // The "high" stream has different bytes.
    let mut high = Vec::new();
    for i in 0..4 {
        let seg: Vec<u8> = segment(i).iter().map(|b| b.wrapping_add(1)).collect();
        s.put(&format!("/high/seg{i}.ts"), seg.clone());
        high.extend(seg);
    }
    s.put("/low/index.m3u8", playlist(4, "", true).into_bytes());
    s.put("/high/index.m3u8", playlist(4, "", true).into_bytes());
    s.put(
        "/master.m3u8",
        b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=900000,RESOLUTION=640x360\nlow/index.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=3000000,RESOLUTION=1280x720\nhigh/index.m3u8\n".to_vec(),
    );

    let best = download(options(s.url("/master.m3u8"), "best"), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(best).unwrap(), high);

    let mut o = options(s.url("/master.m3u8"), "worst");
    o.hls_quality = Quality::Worst;
    let worst = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(worst).unwrap(), put_segments(&s, "/low", 4));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn decrypts_aes_128_segments() {
    type Enc = cbc::Encryptor<aes::Aes128>;
    let s = serve().await;
    let key = [3u8; 16];
    s.put("/enc/key.bin", key.to_vec());
    let mut plain_all = Vec::new();
    for i in 0..6usize {
        let plain = segment(i);
        // No IV in the playlist: the segment's media sequence number is the IV (sequence starts at 5 below).
        let mut iv = [0u8; 16];
        iv[8..].copy_from_slice(&(5 + i as u64).to_be_bytes());
        s.put(&format!("/enc/seg{i}.ts"), Enc::new(&key.into(), &iv.into()).encrypt_padded_vec_mut::<Pkcs7>(&plain));
        plain_all.extend(plain);
    }
    s.put("/enc/index.m3u8", playlist(6, "#EXT-X-MEDIA-SEQUENCE:5\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\"\n", true).into_bytes());
    let path = download(options(s.url("/enc/index.m3u8"), "aes"), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), plain_all);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn segments_that_are_slices_of_one_file() {
    let s = serve().await;
    let big: Vec<u8> = (0..30_000).map(|i| (i % 253) as u8).collect();
    s.put("/r/big.ts", big.clone());
    s.put(
        "/r/index.m3u8",
        b"#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\n#EXT-X-BYTERANGE:10000@0\nbig.ts\n#EXTINF:4,\n#EXT-X-BYTERANGE:10000\nbig.ts\n#EXTINF:4,\n#EXT-X-BYTERANGE:10000\nbig.ts\n#EXT-X-ENDLIST\n".to_vec(),
    );
    let path = download(options(s.url("/r/index.m3u8"), "range"), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), big);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fragmented_mp4_streams_start_with_their_init_section() {
    let s = serve().await;
    let init = b"INIT-SECTION".to_vec();
    s.put("/m/init.mp4", init.clone());
    let mut all = init;
    for i in 0..3 {
        let seg = segment(i);
        s.put(&format!("/m/seg{i}.ts"), seg.clone());
        all.extend(seg);
    }
    s.put("/m/index.m3u8", playlist(3, "#EXT-X-MAP:URI=\"init.mp4\"\n", true).into_bytes());
    let path = download(options(s.url("/m/index.m3u8"), "fmp4"), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(path.extension().unwrap(), "mp4");
    assert_eq!(std::fs::read(path).unwrap(), all);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumes_without_refetching_finished_segments() {
    let s = serve().await;
    let all = put_segments(&s, "/z", 20);
    s.put("/z/index.m3u8", playlist(20, "", true).into_bytes());
    let url = s.url("/z/index.m3u8");
    let dir = tmp("resume");
    let mk = || {
        let mut o = Options::new(url.clone(), &dir, vec![Route::unbound("a")]);
        o.conns_per_route = 1;
        o
    };
    let cancel = CancellationToken::new();
    let (c, n) = (cancel.clone(), Arc::new(AtomicUsize::new(0)));
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ChunkDone { .. } = e {
            if n.fetch_add(1, Ordering::Relaxed) + 1 == 8 {
                c.cancel();
            }
        }
    });
    assert!(matches!(download(mk(), cancel, emit).await.unwrap_err(), Error::Cancelled));
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let before: usize = (0..20).map(|i| s.hits(&format!("/z/seg{i}.ts"))).sum();
    assert!((8..=10).contains(&before), "fetched {before} segments before the cancel");

    let resumed = Arc::new(AtomicUsize::new(0));
    let r = resumed.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::Started { resumed_chunks, .. } = e {
            r.store(resumed_chunks, Ordering::Relaxed);
        }
    });
    let path = download(mk(), CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), all);
    assert!(resumed.load(Ordering::Relaxed) >= 8);
    let total: usize = (0..20).map(|i| s.hits(&format!("/z/seg{i}.ts"))).sum();
    assert!(total <= 20 + 2, "segments already on disk must not be fetched again ({total} requests for 20 segments)");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_streams_are_refused_with_a_clear_message() {
    let s = serve().await;
    put_segments(&s, "/live", 3);
    s.put("/live/index.m3u8", playlist(3, "", false).into_bytes());
    let err = download(options(s.url("/live/index.m3u8"), "live"), CancellationToken::new(), silent()).await.unwrap_err();
    assert!(err.to_string().contains("live stream"), "{err}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remuxes_to_mp4_when_ffmpeg_is_available() {
    use std::os::unix::fs::PermissionsExt;
    let s = serve().await;
    let all = put_segments(&s, "/f", 3);
    s.put("/f/index.m3u8", playlist(3, "", true).into_bytes());
    let dir = tmp("remux");
    std::fs::create_dir_all(&dir).unwrap();
    // A stand-in for ffmpeg that "converts" by copying: args are -y -loglevel error -i IN -c copy OUT.
    let fake = dir.join("fake-ffmpeg");
    std::fs::write(&fake, "#!/bin/sh\ncp \"$5\" \"$8\"\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut o = Options::new(s.url("/f/index.m3u8"), &dir, two_routes());
    o.ffmpeg = Some(fake.clone());
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(path.extension().unwrap(), "mp4");
    assert_eq!(std::fs::read(&path).unwrap(), all);
    assert!(!dir.join("index.ts").exists(), "the transport stream is replaced by the MP4");

    // A failing converter leaves the .ts in place.
    std::fs::write(&fake, "#!/bin/sh\nexit 1\n").unwrap();
    let mut o = Options::new(s.url("/f/index.m3u8"), tmp("remux-fail"), two_routes());
    o.ffmpeg = Some(fake);
    let path = download(o, CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(path.extension().unwrap(), "ts");
    assert_eq!(std::fs::read(path).unwrap(), all);
}
