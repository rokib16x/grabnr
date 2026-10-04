//! SFTP over SSH, through sockets opened on a chosen network link (see `ftp::connect_bound`).
//!
//! Host keys are trusted on first use and remembered in a `known_hosts` file; a changed key is refused.

use std::net::{SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::stream::{FuturesOrdered, StreamExt};
use russh::client;
use russh::keys::{load_secret_key, HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh_sftp::client::RawSftpSession;
use russh_sftp::protocol::{FileAttributes, OpenFlags};
use tokio::io::{AsyncWriteExt, DuplexStream};

use crate::download::Route;
use crate::error::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub path: String,
}

pub fn looks_like(url: &str) -> bool {
    url.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("sftp://"))
}

/// `sftp://user[:password]@host[:port]/path/to/file`. A user name is required.
pub fn parse(url: &str) -> Result<Target> {
    let u = url::Url::parse(url).map_err(|_| Error::Other("that is not a valid SFTP link".into()))?;
    if u.scheme() != "sftp" {
        return Err(Error::Other("that is not an SFTP link".into()));
    }
    let host = u.host_str().ok_or_else(|| Error::Other("the SFTP link has no host".into()))?.to_string();
    if u.username().is_empty() {
        return Err(Error::Other("an SFTP link needs a user name: sftp://user@host/path".into()));
    }
    let dec = crate::ftp::percent_decode;
    let path = dec(u.path());
    if path.is_empty() || path.ends_with('/') {
        return Err(Error::Other("the SFTP link must point to a file, not a folder".into()));
    }
    Ok(Target { host, port: u.port().unwrap_or(22), user: dec(u.username()), pass: dec(u.password().unwrap_or("")), path })
}

/// Options for the SSH side that do not belong in the link.
#[derive(Debug, Clone, Default)]
pub struct SshOptions {
    /// File remembering the host keys seen so far. Without one, every host key is accepted (not recommended).
    pub known_hosts: Option<PathBuf>,
    /// Private key to sign in with instead of a password; the password in the link is then its passphrase.
    pub key: Option<PathBuf>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// First time we see this host: remembered now.
    New,
    Trusted,
    /// The host presents a different key than before.
    Changed {
        expected: String,
    },
}

/// The host keys seen so far, one `host:port fingerprint` per line.
pub fn check_known_host(file: &Path, host: &str, port: u16, fingerprint: &str) -> std::io::Result<Verdict> {
    let id = format!("{}:{port}", host.to_ascii_lowercase());
    let text = std::fs::read_to_string(file).unwrap_or_default();
    for line in text.lines() {
        if let Some((h, fp)) = line.split_once(' ') {
            if h == id {
                return Ok(if fp.trim() == fingerprint { Verdict::Trusted } else { Verdict::Changed { expected: fp.trim().to_string() } });
            }
        }
    }
    use std::io::Write;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(file)?;
    writeln!(f, "{id} {fingerprint}")?;
    Ok(Verdict::New)
}

struct Handler {
    host: String,
    port: u16,
    known: Option<PathBuf>,
    /// Why the key was refused, for the error message.
    refused: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Handler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> std::result::Result<bool, Self::Error> {
        let fp = match key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.fingerprint(HashAlg::Sha256).to_string(),
            PublicKeyOrCertificate::Certificate(c) => c.public_key().fingerprint(HashAlg::Sha256).to_string(),
        };
        let Some(file) = &self.known else { return Ok(true) };
        match check_known_host(file, &self.host, self.port, &fp) {
            Ok(Verdict::New | Verdict::Trusted) => Ok(true),
            Ok(Verdict::Changed { expected }) => {
                *self.refused.lock().unwrap() = Some(format!(
                    "the key of {} changed (was {expected}, now {fp}); this could be an attack, or the server was reinstalled",
                    self.host
                ));
                Ok(false)
            }
            Err(_) => Ok(true),
        }
    }
}

pub struct Session {
    sftp: Arc<RawSftpSession>,
    /// Keeps the SSH connection open.
    _ssh: client::Handle<Handler>,
}

fn err(msg: impl Into<String>) -> Error {
    Error::Other(format!("SFTP: {}", msg.into()))
}

fn not_found_or(e: impl std::fmt::Display) -> Error {
    let s = e.to_string();
    if s.to_lowercase().contains("no such") {
        Error::Status(404)
    } else {
        err(s)
    }
}

/// Bytes asked for in one SFTP read request (a packet is limited to 256 KiB).
const BLOCK: u64 = 128 * 1024;
/// Read requests kept in flight, to hide the round trip time.
const DEPTH: usize = 8;

impl Session {
    pub async fn connect(t: &Target, route: &Route, stall: Duration, ssh: &SshOptions) -> Result<Session> {
        let addr = tokio::net::lookup_host((t.host.as_str(), t.port))
            .await
            .map_err(|e| err(format!("cannot find {}: {e}", t.host)))?
            .find_map(|a| match a {
                SocketAddr::V4(v4) => Some(v4),
                _ => None,
            })
            .ok_or_else(|| err(format!("{} has no IPv4 address", t.host)))?;
        let sock = crate::ftp::connect_bound(SocketAddrV4::new(*addr.ip(), addr.port()), route, Duration::from_secs(8))
            .await
            .map_err(|e| err(format!("cannot connect to {}: {e}", t.host)))?;
        let refused = Arc::new(Mutex::new(None));
        let handler = Handler { host: t.host.clone(), port: t.port, known: ssh.known_hosts.clone(), refused: refused.clone() };
        let config = Arc::new(client::Config { inactivity_timeout: Some(stall.max(Duration::from_secs(30)) * 4), ..Default::default() });
        let mut session = match tokio::time::timeout(Duration::from_secs(20), client::connect_stream(config, sock, handler)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => return Err(refused.lock().unwrap().take().map(err).unwrap_or_else(|| err(e.to_string()))),
            Err(_) => return Err(err("the server did not answer in time")),
        };
        let ok = match &ssh.key {
            Some(path) => {
                let key = load_secret_key(path, Some(t.pass.as_str()).filter(|p| !p.is_empty()))
                    .map_err(|e| err(format!("cannot read the private key: {e}")))?;
                let hash = session.best_supported_rsa_hash().await.map_err(|e| err(e.to_string()))?.flatten();
                session
                    .authenticate_publickey(&t.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                    .await
                    .map_err(|e| err(e.to_string()))?
                    .success()
            }
            None => session.authenticate_password(&t.user, &t.pass).await.map_err(|e| err(e.to_string()))?.success(),
        };
        if !ok {
            return Err(Error::Status(401));
        }
        let channel = session.channel_open_session().await.map_err(|e| err(e.to_string()))?;
        channel.request_subsystem(true, "sftp").await.map_err(|e| err(e.to_string()))?;
        // The raw protocol session, not the file-like wrapper: the wrapper reads far ahead, which wastes bandwidth when a
        // file is fetched in small pieces.
        let raw = RawSftpSession::new(channel.into_stream());
        raw.set_timeout(stall.as_secs().max(5));
        raw.init().await.map_err(|e| err(e.to_string()))?;
        Ok(Session { sftp: Arc::new(raw), _ssh: session })
    }

    /// Size and modification time.
    pub async fn stat(&mut self, path: &str) -> Result<(u64, Option<String>)> {
        let a = self.sftp.stat(path).await.map_err(not_found_or)?.attrs;
        let size = a.size.ok_or_else(|| err("the server did not report the file size"))?;
        Ok((size, a.mtime.map(|t| t.to_string())))
    }

    /// Read exactly `len` bytes from `start`: requests are pipelined, never reach past the end, and stop when the
    /// returned reader is dropped.
    pub async fn open_range(&mut self, path: &str, start: u64, len: u64) -> Result<DuplexStream> {
        let handle = self.sftp.open(path, OpenFlags::READ, FileAttributes::default()).await.map_err(not_found_or)?.handle;
        let (mut tx, rx) = tokio::io::duplex(1 << 20);
        let (raw, h) = (self.sftp.clone(), handle);
        tokio::spawn(async move {
            let end = start + len;
            let mut next = start;
            let mut inflight = FuturesOrdered::new();
            'read: loop {
                while inflight.len() < DEPTH && next < end {
                    let (off, l) = (next, (end - next).min(BLOCK));
                    next += l;
                    let (raw, h) = (raw.clone(), h.clone());
                    inflight.push_back(async move { (off, l, raw.read(h, off, l as u32).await) });
                }
                let Some((off, l, res)) = inflight.next().await else { break };
                let Ok(data) = res else { break };
                if tx.write_all(&data.data).await.is_err() {
                    break; // the reader went away
                }
                // A short answer: fetch the rest of this block before the blocks that follow it.
                let mut got = data.data.len() as u64;
                while got < l {
                    match raw.read(h.clone(), off + got, (l - got) as u32).await {
                        Ok(more) if !more.data.is_empty() => {
                            got += more.data.len() as u64;
                            if tx.write_all(&more.data).await.is_err() {
                                break 'read;
                            }
                        }
                        _ => break 'read,
                    }
                }
            }
            drop(inflight);
            let _ = raw.close(h).await;
            let _ = tx.shutdown().await;
        });
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links() {
        let t = parse("sftp://me:p%40ss@host.example:2222/home/me/a%20b.iso").unwrap();
        assert_eq!(
            (t.user.as_str(), t.pass.as_str(), t.host.as_str(), t.port, t.path.as_str()),
            ("me", "p@ss", "host.example", 2222, "/home/me/a b.iso")
        );
        assert_eq!(parse("sftp://me@h/x").unwrap().port, 22);
        assert!(parse("sftp://h/x").is_err(), "a user name is required");
        assert!(parse("sftp://me@h/dir/").is_err());
        assert!(parse("ftp://me@h/x").is_err());
        assert!(looks_like("SFTP://a@b/c") && !looks_like("ftp://a/b"));
    }

    #[test]
    fn trusts_on_first_use_and_refuses_a_changed_key() {
        let dir = std::env::temp_dir().join(format!("grabnr-kh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("known_hosts");
        assert_eq!(check_known_host(&file, "Example.COM", 22, "SHA256:aaa").unwrap(), Verdict::New);
        assert_eq!(check_known_host(&file, "example.com", 22, "SHA256:aaa").unwrap(), Verdict::Trusted);
        assert_eq!(check_known_host(&file, "example.com", 22, "SHA256:bbb").unwrap(), Verdict::Changed { expected: "SHA256:aaa".into() });
        assert_eq!(check_known_host(&file, "example.com", 2222, "SHA256:bbb").unwrap(), Verdict::New, "another port is another server");
        let _ = std::fs::remove_dir_all(dir);
    }
}
