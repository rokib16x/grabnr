//! FTP downloads against a small in-process FTP server.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use grabnr_core::{download, Error, Event, Options, Route, Store};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

struct Server {
    port: u16,
    /// Control connections accepted so far.
    sessions: Arc<AtomicUsize>,
    /// File bytes sent over data connections.
    sent: Arc<AtomicU64>,
    /// Commands received, in order, for every session.
    log: Arc<Mutex<Vec<String>>>,
    cfg: Arc<Mutex<Cfg>>,
}

#[derive(Default)]
struct Cfg {
    files: HashMap<String, Vec<u8>>,
    user: String,
    pass: String,
    no_rest: bool,
    /// Report a private, unreachable address in PASV replies (a server behind NAT).
    lie_in_pasv: bool,
    no_epsv: bool,
}

async fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (sessions, sent, log) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicU64::new(0)), Arc::new(Mutex::new(Vec::new())));
    let cfg = Arc::new(Mutex::new(Cfg::default()));
    let (s2, sn2, l2, c2) = (sessions.clone(), sent.clone(), log.clone(), cfg.clone());
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = listener.accept().await else { return };
            s2.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(session(sock, sn2.clone(), l2.clone(), c2.clone()));
        }
    });
    Server { port, sessions, sent, log, cfg }
}

async fn session(sock: TcpStream, sent: Arc<AtomicU64>, log: Arc<Mutex<Vec<String>>>, cfg: Arc<Mutex<Cfg>>) {
    let (rd, mut wr) = sock.into_split();
    let mut rd = BufReader::new(rd);
    let reply = |s: &str| format!("{s}\r\n");
    wr.write_all(reply("220 test ftp ready").as_bytes()).await.unwrap();
    let (mut user_ok, mut rest, mut pasv): (bool, u64, Option<TcpListener>) = (false, 0, None);
    let mut line = String::new();
    loop {
        line.clear();
        if rd.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let l = line.trim().to_string();
        log.lock().unwrap().push(l.clone());
        let (cmd, arg) = l.split_once(' ').map(|(a, b)| (a.to_uppercase(), b.to_string())).unwrap_or((l.to_uppercase(), String::new()));
        let out = match cmd.as_str() {
            "USER" => {
                user_ok = arg == cfg.lock().unwrap().user;
                reply("331 password please")
            }
            "PASS" => {
                if user_ok && arg == cfg.lock().unwrap().pass {
                    reply("230 welcome")
                } else {
                    reply("530 login incorrect")
                }
            }
            "TYPE" => reply("200 binary"),
            "SIZE" => match cfg.lock().unwrap().files.get(&arg) {
                Some(f) => reply(&format!("213 {}", f.len())),
                None => reply("550 no such file"),
            },
            "MDTM" => reply("213 20240101120000"),
            "REST" => {
                if cfg.lock().unwrap().no_rest {
                    reply("502 REST not implemented")
                } else {
                    rest = arg.parse().unwrap_or(0);
                    reply("350 restarting")
                }
            }
            "EPSV" | "PASV" => {
                if cmd == "EPSV" && cfg.lock().unwrap().no_epsv {
                    reply("500 EPSV not understood")
                } else {
                    let ln = TcpListener::bind("127.0.0.1:0").await.unwrap();
                    let p = ln.local_addr().unwrap().port();
                    pasv = Some(ln);
                    if cmd == "EPSV" {
                        reply(&format!("229 Entering Extended Passive Mode (|||{p}|)"))
                    } else {
                        let ip = if cfg.lock().unwrap().lie_in_pasv { "10,255,255,1" } else { "127,0,0,1" };
                        reply(&format!("227 Entering Passive Mode ({ip},{},{})", p / 256, p % 256))
                    }
                }
            }
            "RETR" => {
                let file = cfg.lock().unwrap().files.get(&arg).cloned();
                match (file, pasv.take()) {
                    (Some(f), Some(ln)) => {
                        wr.write_all(reply("150 opening data connection").as_bytes()).await.unwrap();
                        let (mut data, _) = ln.accept().await.unwrap();
                        let start = (rest as usize).min(f.len());
                        rest = 0;
                        let mut aborted = false;
                        for part in f[start..].chunks(32 * 1024) {
                            if data.write_all(part).await.is_err() {
                                aborted = true;
                                break;
                            }
                            sent.fetch_add(part.len() as u64, Ordering::Relaxed);
                            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                        }
                        drop(data);
                        reply(if aborted { "426 transfer aborted" } else { "226 transfer complete" })
                    }
                    (None, _) => reply("550 no such file"),
                    _ => reply("425 use PASV first"),
                }
            }
            "QUIT" => {
                let _ = wr.write_all(reply("221 bye").as_bytes()).await;
                return;
            }
            _ => reply("502 not implemented"),
        };
        wr.write_all(out.as_bytes()).await.unwrap();
    }
}

fn data(len: usize, seed: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 11 + seed) % 247) as u8).collect()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("grabnr-ftp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn silent() -> Arc<dyn Fn(Event) + Send + Sync> {
    Arc::new(|_| {})
}

fn two_routes() -> Vec<Route> {
    vec![Route::unbound("a"), Route::unbound("b")]
}

impl Server {
    fn with(&self, f: impl FnOnce(&mut Cfg)) {
        f(&mut self.cfg.lock().unwrap());
    }
    fn url(&self, path: &str) -> String {
        format!("ftp://tester:secret@127.0.0.1:{}{path}", self.port)
    }
    fn setup(&self, path: &str, body: Vec<u8>) {
        self.with(|c| {
            c.user = "tester".into();
            c.pass = "secret".into();
            c.files.insert(path.into(), body);
        });
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_a_file_in_pieces_over_both_links() {
    let s = serve().await;
    let body = data(5 * 1024 * 1024 + 321, 3);
    s.setup("/pub/big.bin", body.clone());
    let chunks = Arc::new(Mutex::new([0usize; 2]));
    let c = chunks.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ChunkDone { route, .. } = e {
            c.lock().unwrap()[route] += 1;
        }
    });
    let mut o = Options::new(s.url("/pub/big.bin"), tmp("basic"), two_routes());
    o.conns_per_route = 2;
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(path.file_name().unwrap(), "big.bin");
    assert_eq!(std::fs::read(path).unwrap(), body);
    let c = *chunks.lock().unwrap();
    assert!(c[0] > 0 && c[1] > 0, "both links should carry pieces: {c:?}");
    assert!(s.sessions.load(Ordering::Relaxed) >= 3, "pieces are fetched over several connections");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ignores_a_private_address_in_the_passive_reply_and_works_without_epsv() {
    let s = serve().await;
    let body = data(2 * 1024 * 1024, 9);
    s.setup("/f.bin", body.clone());
    s.with(|c| {
        c.lie_in_pasv = true;
        c.no_epsv = true;
    });
    let path = download(Options::new(s.url("/f.bin"), tmp("nat"), two_routes()), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
    assert!(s.log.lock().unwrap().iter().any(|l| l == "PASV"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_password_and_a_missing_file_fail_at_once_with_clear_errors() {
    let s = serve().await;
    s.setup("/f.bin", data(1000, 1));
    let bad = format!("ftp://tester:wrong@127.0.0.1:{}/f.bin", s.port);
    let err = download(Options::new(bad, tmp("badpass"), two_routes()), CancellationToken::new(), silent()).await.unwrap_err();
    assert!(matches!(err, Error::Status(401)), "{err}");
    let err =
        download(Options::new(s.url("/nope.bin"), tmp("nofile"), two_routes()), CancellationToken::new(), silent()).await.unwrap_err();
    assert!(matches!(err, Error::Status(404)), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn refuses_servers_that_cannot_start_part_way() {
    let s = serve().await;
    s.setup("/f.bin", data(2000, 2));
    s.with(|c| c.no_rest = true);
    let err = download(Options::new(s.url("/f.bin"), tmp("norest"), two_routes()), CancellationToken::new(), silent()).await.unwrap_err();
    assert!(err.to_string().contains("REST"), "{err}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumes_and_verifies_a_checksum() {
    let s = serve().await;
    let body = data(6 * 1024 * 1024, 5);
    s.setup("/r.bin", body.clone());
    let store = Arc::new(Store::in_memory().unwrap());
    let dir = tmp("resume");
    let mk = |sum: Option<String>| {
        let mut o = Options::new(s.url("/r.bin"), &dir, vec![Route::unbound("a")]);
        o.conns_per_route = 1;
        o.store = Some(store.clone());
        o.resume_key = Some("ftp-item".into());
        o.checksum = sum.map(|h| grabnr_core::Checksum::parse(&h).unwrap());
        o
    };
    let cancel = CancellationToken::new();
    let (c, n) = (cancel.clone(), Arc::new(AtomicUsize::new(0)));
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ChunkDone { .. } = e {
            if n.fetch_add(1, Ordering::Relaxed) + 1 == 2 {
                c.cancel();
            }
        }
    });
    assert!(matches!(download(mk(None), cancel, emit).await.unwrap_err(), Error::Cancelled));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let before = s.sent.load(Ordering::Relaxed);

    let sum = grabnr_core::checksum::compute_bytes(grabnr_core::Algo::Sha256, &body);
    let path = download(mk(Some(sum)), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
    let second = s.sent.load(Ordering::Relaxed) - before;
    assert!(second < body.len() as u64 - 1_500_000, "resume re-sent {second} of {} bytes", body.len());
}
