//! BitTorrent downloads against a local seeder, with the peer given explicitly (no DHT or tracker).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use grabnr_core::{download, Error, Event, Options, Route};
use librqbit::spawn_utils::BlockingSpawner;
use librqbit::{AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerMode, ListenerOptions, Session, SessionOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

fn data(len: usize, seed: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 31 + seed * 7 + (i >> 8)) % 251) as u8).collect()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("grabnr-bt-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Seed {
    addr: SocketAddr,
    torrent: Vec<u8>,
    _session: Arc<Session>,
}

/// Seed `content` (a file or folder under `parent`) from a local session and return the `.torrent` bytes.
async fn seed(parent: &Path, content: &str, sub: Option<&str>) -> Seed {
    let created = librqbit::create_torrent(
        &parent.join(content),
        CreateTorrentOptions { name: None, trackers: vec![], piece_length: Some(64 * 1024) },
        &BlockingSpawner::new(4),
    )
    .await
    .unwrap();
    let torrent = created.as_bytes().unwrap().to_vec();
    let session = Session::new_with_opts(
        parent.to_path_buf(),
        SessionOptions {
            dht: None,
            disable_trackers: true,
            persistence: None,
            disable_local_service_discovery: true,
            ipv4_only: true,
            listen: Some(ListenerOptions {
                mode: ListenerMode::TcpOnly,
                listen_addr: "127.0.0.1:0".parse().unwrap(),
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let handle = session
        .add_torrent(
            AddTorrent::from_bytes(torrent.clone()),
            Some(AddTorrentOptions { overwrite: true, sub_folder: sub.map(str::to_owned), ..Default::default() }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(30), handle.wait_until_completed())
        .await
        .expect("the seeder checks its files")
        .unwrap();
    let addr = session.listen_addr().expect("the seeder listens");
    Seed { addr: SocketAddr::new("127.0.0.1".parse().unwrap(), addr.port()), torrent, _session: session }
}

/// Serve one `.torrent` over HTTP.
async fn serve_torrent(bytes: Vec<u8>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let bytes = bytes.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/x-bittorrent\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    bytes.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&bytes).await;
            });
        }
    });
    format!("http://127.0.0.1:{port}/files.torrent")
}

fn two_routes() -> Vec<Route> {
    vec![Route::unbound("a"), Route::unbound("b")]
}

fn silent() -> Arc<dyn Fn(Event) + Send + Sync> {
    Arc::new(|_| {})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_multi_file_torrent_is_split_by_file_over_both_links() {
    let parent = tmp("seed-multi");
    std::fs::create_dir_all(parent.join("pack")).unwrap();
    let files: HashMap<&str, Vec<u8>> = HashMap::from([
        ("a.bin", data(1_500_000, 1)),
        ("b.bin", data(900_000, 2)),
        ("c.bin", data(700_000, 3)),
        ("d.bin", data(400_000, 4)),
    ]);
    for (n, d) in &files {
        std::fs::write(parent.join("pack").join(n), d).unwrap();
    }
    let s = seed(&parent, "pack", Some("pack")).await;
    let url = serve_torrent(s.torrent.clone()).await;

    let seen = Arc::new(Mutex::new((HashMap::<usize, usize>::new(), 0usize, 0u64)));
    let sn = seen.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| match e {
        Event::ChunkDone { idx, route } => {
            sn.lock().unwrap().0.insert(idx, route);
        }
        Event::Progress(p) => {
            let mut g = sn.lock().unwrap();
            g.1 = g.1.max(p.routes.len());
            g.2 = g.2.max(p.downloaded);
            assert!(p.downloaded <= p.total.unwrap());
        }
        _ => {}
    });
    let dest = tmp("leech-multi");
    let mut o = Options::new(url, &dest, two_routes());
    o.torrent_peers = vec![s.addr];
    let path = tokio::time::timeout(std::time::Duration::from_secs(120), download(o, CancellationToken::new(), emit))
        .await
        .expect("finishes in time")
        .unwrap();

    assert_eq!(path, dest.join("pack"));
    for (n, d) in &files {
        assert_eq!(&std::fs::read(path.join(n)).unwrap(), d, "{n} must arrive intact");
    }
    let g = seen.lock().unwrap();
    assert_eq!(g.0.len(), 4, "every file is reported as finished: {:?}", g.0);
    let routes: std::collections::HashSet<usize> = g.0.values().copied().collect();
    assert_eq!(routes.len(), 2, "both links should have downloaded some files: {:?}", g.0);
    assert_eq!(g.1, 2, "progress reports both links");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn a_single_file_torrent_uses_one_link_and_can_be_cancelled_and_resumed() {
    let parent = tmp("seed-single");
    let body = data(3_000_000, 9);
    std::fs::write(parent.join("movie.bin"), &body).unwrap();
    let s = seed(&parent, "movie.bin", None).await;
    let url = serve_torrent(s.torrent.clone()).await;
    let dest = tmp("leech-single");
    let mk = || {
        let mut o = Options::new(url.clone(), &dest, two_routes());
        o.torrent_peers = vec![s.addr];
        o
    };

    // Cancel as soon as some data has arrived.
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    let links = Arc::new(AtomicUsize::new(0));
    let l = links.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::Progress(p) = e {
            l.fetch_max(p.routes.len(), Ordering::Relaxed);
            if p.downloaded > 0 {
                c.cancel();
            }
        }
    });
    let first = tokio::time::timeout(std::time::Duration::from_secs(120), download(mk(), cancel, emit)).await.expect("stops in time");
    // It may also have finished before the cancel landed; both are fine as long as the second run ends with the right file.
    assert!(matches!(first, Err(Error::Cancelled)) || first.is_ok(), "{first:?}");
    assert_eq!(links.load(Ordering::Relaxed), 1, "a single file is fetched over a single link");

    let path = tokio::time::timeout(std::time::Duration::from_secs(120), download(mk(), CancellationToken::new(), silent()))
        .await
        .expect("finishes in time")
        .unwrap();
    assert_eq!(path, dest.join("movie.bin"));
    assert_eq!(std::fs::read(path).unwrap(), body);
}
