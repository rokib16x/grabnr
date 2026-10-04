//! SFTP downloads against an in-process SSH/SFTP server.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use grabnr_core::sftp::SshOptions;
use grabnr_core::{download, Error, Event, Options, Route};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use russh_sftp::protocol::{Attrs, Data, FileAttributes, Handle, Status, StatusCode, Version};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

type Files = Arc<Mutex<HashMap<String, Vec<u8>>>>;

#[derive(Clone)]
struct Shared {
    files: Files,
    sessions: Arc<AtomicUsize>,
    sent: Arc<AtomicU64>,
}

#[derive(Clone)]
struct Srv(Shared);

impl russh::server::Server for Srv {
    type Handler = Ssh;
    fn new_client(&mut self, _: Option<SocketAddr>) -> Ssh {
        self.0.sessions.fetch_add(1, Ordering::Relaxed);
        Ssh { shared: self.0.clone(), channels: HashMap::new() }
    }
}

struct Ssh {
    shared: Shared,
    channels: HashMap<ChannelId, Channel<Msg>>,
}

impl russh::server::Handler for Ssh {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        Ok(if user == "tester" && password == "secret" { Auth::Accept } else { Auth::reject() })
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: russh::server::ChannelOpenHandle,
        _s: &mut Session,
    ) -> Result<(), Self::Error> {
        self.channels.insert(channel.id(), channel);
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(&mut self, id: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        if name == "sftp" {
            let channel = self.channels.remove(&id).unwrap();
            session.channel_success(id)?;
            russh_sftp::server::run(channel.into_stream(), Files2 { shared: self.shared.clone() }).await;
        } else {
            session.channel_failure(id)?;
        }
        Ok(())
    }
}

struct Files2 {
    shared: Shared,
}

fn ok(id: u32) -> Status {
    Status { id, status_code: StatusCode::Ok, error_message: "Ok".into(), language_tag: "en".into() }
}

impl russh_sftp::server::Handler for Files2 {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn init(&mut self, _v: u32, _e: HashMap<String, String>) -> Result<Version, StatusCode> {
        Ok(Version::new())
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        _f: russh_sftp::protocol::OpenFlags,
        _a: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        if self.shared.files.lock().unwrap().contains_key(&filename) {
            Ok(Handle { id, handle: filename })
        } else {
            Err(StatusCode::NoSuchFile)
        }
    }

    async fn close(&mut self, id: u32, _h: String) -> Result<Status, StatusCode> {
        Ok(ok(id))
    }

    async fn read(&mut self, id: u32, handle: String, offset: u64, len: u32) -> Result<Data, StatusCode> {
        let files = self.shared.files.lock().unwrap();
        let f = files.get(&handle).ok_or(StatusCode::NoSuchFile)?;
        let start = offset as usize;
        if start >= f.len() {
            return Err(StatusCode::Eof);
        }
        let end = (start + len as usize).min(f.len());
        self.shared.sent.fetch_add((end - start) as u64, Ordering::Relaxed);
        Ok(Data { id, data: f[start..end].to_vec() })
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let files = self.shared.files.lock().unwrap();
        let f = files.get(&path).ok_or(StatusCode::NoSuchFile)?;
        let attrs = FileAttributes { size: Some(f.len() as u64), mtime: Some(1_700_000_000), ..Default::default() };
        Ok(Attrs { id, attrs })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        self.stat(id, path).await
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, StatusCode> {
        self.stat(id, handle).await
    }
}

struct Server {
    port: u16,
    shared: Shared,
    task: tokio::task::JoinHandle<()>,
}

async fn serve_on(listener: TcpListener, shared: Shared) -> tokio::task::JoinHandle<()> {
    let config = Arc::new(russh::server::Config {
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        auth_rejection_time: std::time::Duration::from_millis(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_millis(0)),
        ..Default::default()
    });
    let mut srv = Srv(shared);
    tokio::spawn(async move {
        let _ = srv.run_on_socket(config, &listener).await;
    })
}

async fn serve() -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let shared =
        Shared { files: Arc::new(Mutex::new(HashMap::new())), sessions: Arc::new(AtomicUsize::new(0)), sent: Arc::new(AtomicU64::new(0)) };
    let task = serve_on(listener, shared.clone()).await;
    Server { port, shared, task }
}

impl Server {
    fn put(&self, path: &str, body: Vec<u8>) {
        self.shared.files.lock().unwrap().insert(path.into(), body);
    }
    fn url(&self, path: &str) -> String {
        format!("sftp://tester:secret@127.0.0.1:{}{path}", self.port)
    }
}

fn data(len: usize, seed: usize) -> Vec<u8> {
    (0..len).map(|i| ((i * 13 + seed) % 241) as u8).collect()
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("grabnr-sftp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn silent() -> Arc<dyn Fn(Event) + Send + Sync> {
    Arc::new(|_| {})
}

fn two_routes() -> Vec<Route> {
    vec![Route::unbound("a"), Route::unbound("b")]
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn downloads_in_pieces_over_both_links() {
    let s = serve().await;
    let body = data(4 * 1024 * 1024 + 99, 7);
    s.put("/srv/big.bin", body.clone());
    let chunks = Arc::new(Mutex::new([0usize; 2]));
    let c = chunks.clone();
    let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(move |e| {
        if let Event::ChunkDone { route, .. } = e {
            c.lock().unwrap()[route] += 1;
        }
    });
    let mut o = Options::new(s.url("/srv/big.bin"), tmp("basic"), two_routes());
    o.conns_per_route = 2;
    let path = download(o, CancellationToken::new(), emit).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
    let c = *chunks.lock().unwrap();
    assert!(c[0] > 0 && c[1] > 0, "both links should carry pieces: {c:?}");
    assert!(s.shared.sessions.load(Ordering::Relaxed) >= 2);
    s.task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wrong_password_and_a_missing_file_fail_with_clear_errors() {
    let s = serve().await;
    s.put("/f.bin", data(1000, 1));
    let bad = format!("sftp://tester:nope@127.0.0.1:{}/f.bin", s.port);
    let err = download(Options::new(bad, tmp("badpass"), two_routes()), CancellationToken::new(), silent()).await.unwrap_err();
    assert!(matches!(err, Error::Status(401)), "{err}");
    let err =
        download(Options::new(s.url("/missing.bin"), tmp("missing"), two_routes()), CancellationToken::new(), silent()).await.unwrap_err();
    assert!(matches!(err, Error::Status(404)), "{err}");
    s.task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remembers_the_host_key_and_refuses_a_different_one() {
    let s = serve().await;
    let body = data(1_500_000, 3);
    s.put("/k.bin", body.clone());
    let known = tmp("known").join("known_hosts");
    let mk = |name: &str| {
        let mut o = Options::new(s.url("/k.bin"), tmp(name), vec![Route::unbound("a")]);
        o.ssh = SshOptions { known_hosts: Some(known.clone()), key: None };
        o
    };
    // First visit: trusted and remembered. Second visit: same key, fine.
    let p = download(mk("tofu1"), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(p).unwrap(), body);
    assert!(std::fs::read_to_string(&known).unwrap().contains(&format!("127.0.0.1:{}", s.port)));
    let p = download(mk("tofu2"), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(p).unwrap(), body);

    // A different server (new host key) takes over the same address.
    s.task.abort();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let listener = TcpListener::bind(("127.0.0.1", s.port)).await.unwrap();
    let task2 = serve_on(listener, s.shared.clone()).await;
    let err = download(mk("tofu3"), CancellationToken::new(), silent()).await.unwrap_err();
    assert!(err.to_string().contains("changed"), "a changed host key must be refused: {err}");
    task2.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resumes_without_refetching_finished_pieces() {
    let s = serve().await;
    let body = data(6 * 1024 * 1024, 5);
    s.put("/r.bin", body.clone());
    let store = Arc::new(grabnr_core::Store::in_memory().unwrap());
    let dir = tmp("resume");
    let mk = || {
        let mut o = Options::new(s.url("/r.bin"), &dir, vec![Route::unbound("a")]);
        o.conns_per_route = 1;
        o.store = Some(store.clone());
        o.resume_key = Some("sftp-item".into());
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
    assert!(matches!(download(mk(), cancel, emit).await.unwrap_err(), Error::Cancelled));
    let before = s.shared.sent.load(Ordering::Relaxed);
    let path = download(mk(), CancellationToken::new(), silent()).await.unwrap();
    assert_eq!(std::fs::read(path).unwrap(), body);
    let second = s.shared.sent.load(Ordering::Relaxed) - before;
    // Two of six pieces (about 2 MB) were already on disk. Reads never run past the piece, so almost nothing is wasted.
    assert!(second <= body.len() as u64 - 1_900_000 + 300_000, "resume re-sent {second} of {} bytes", body.len());
    s.task.abort();
}
