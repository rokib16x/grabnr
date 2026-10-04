//! A small FTP client that opens its sockets through a chosen network link, so each connection of a download can use a
//! different link (the point of grabnr). Plain FTP only: no TLS, and passwords travel unencrypted.

use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::bind::BindMode;
use crate::download::Route;
use crate::error::{Error, Result};

/// Where a file is and how to sign in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub pass: String,
    pub path: String,
}

pub fn looks_like(url: &str) -> bool {
    url.get(..6).is_some_and(|p| p.eq_ignore_ascii_case("ftp://"))
}

/// `ftp://[user[:password]@]host[:port]/path/to/file`. Without a user the sign-in is anonymous.
pub fn parse(url: &str) -> Result<Target> {
    let u = url::Url::parse(url).map_err(|_| Error::Other("that is not a valid FTP link".into()))?;
    if u.scheme() != "ftp" {
        return Err(Error::Other("that is not an FTP link".into()));
    }
    let host = u.host_str().ok_or_else(|| Error::Other("the FTP link has no host".into()))?.to_string();
    let dec = |s: &str| percent_decode(s);
    let (user, pass) = if u.username().is_empty() {
        ("anonymous".to_string(), "grabnr@".to_string())
    } else {
        (dec(u.username()), dec(u.password().unwrap_or("")))
    };
    let path = dec(u.path());
    if path.is_empty() || path.ends_with('/') {
        return Err(Error::Other("the FTP link must point to a file, not a folder".into()));
    }
    Ok(Target { host, port: u.port().unwrap_or(21), user, pass, path })
}

pub(crate) fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Open a TCP connection to `addr` that leaves through `route`'s link.
pub async fn connect_bound(addr: SocketAddrV4, route: &Route, timeout: Duration) -> std::io::Result<TcpStream> {
    let (link, mode) = (route.link.clone(), route.mode);
    let std_stream = tokio::task::spawn_blocking(move || -> std::io::Result<std::net::TcpStream> {
        use socket2::{Domain, Protocol, Socket, Type};
        let s = Socket::new(Domain::IPV4, Type::STREAM, Some(Protocol::TCP))?;
        if let Some(link) = &link {
            match mode {
                BindMode::None => {}
                BindMode::LocalAddr => s.bind(&SocketAddr::V4(SocketAddrV4::new(link.ipv4, 0)).into())?,
                BindMode::BoundIf => bind_to_interface(&s, link)?,
            }
        }
        s.connect_timeout(&SocketAddr::V4(addr).into(), timeout)?;
        s.set_nonblocking(true)?;
        s.set_tcp_nodelay(true).ok();
        Ok(s.into())
    })
    .await
    .map_err(|e| std::io::Error::other(e.to_string()))??;
    TcpStream::from_std(std_stream)
}

#[cfg(any(target_os = "macos", target_os = "ios", target_os = "tvos", target_os = "watchos", target_os = "visionos"))]
fn bind_to_interface(s: &socket2::Socket, link: &crate::interfaces::Link) -> std::io::Result<()> {
    s.bind_device_by_index_v4(std::num::NonZeroU32::new(link.index))
}

#[cfg(any(target_os = "linux", target_os = "android", target_os = "fuchsia"))]
fn bind_to_interface(s: &socket2::Socket, link: &crate::interfaces::Link) -> std::io::Result<()> {
    s.bind_device(Some(link.name.as_bytes()))
}

// Elsewhere (Windows): bind the source address; the OS then routes by it.
#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "tvos",
    target_os = "watchos",
    target_os = "visionos",
    target_os = "linux",
    target_os = "android",
    target_os = "fuchsia"
)))]
fn bind_to_interface(s: &socket2::Socket, link: &crate::interfaces::Link) -> std::io::Result<()> {
    s.bind(&SocketAddr::V4(SocketAddrV4::new(link.ipv4, 0)).into())
}

/// One signed-in FTP control connection.
pub struct Session {
    ctrl: BufReader<TcpStream>,
    peer: Ipv4Addr,
    route: Route,
    stall: Duration,
    in_transfer: bool,
}

fn proto(msg: impl Into<String>) -> Error {
    Error::Other(format!("FTP: {}", msg.into()))
}

impl Session {
    pub async fn connect(t: &Target, route: &Route, stall: Duration) -> Result<Session> {
        let addr = tokio::net::lookup_host((t.host.as_str(), t.port))
            .await
            .map_err(|e| proto(format!("cannot find {}: {e}", t.host)))?
            .find_map(|a| match a {
                SocketAddr::V4(v4) => Some(v4),
                _ => None,
            })
            .ok_or_else(|| proto(format!("{} has no IPv4 address", t.host)))?;
        let sock =
            connect_bound(addr, route, Duration::from_secs(8)).await.map_err(|e| proto(format!("cannot connect to {}: {e}", t.host)))?;
        let mut s = Session { ctrl: BufReader::new(sock), peer: *addr.ip(), route: route.clone(), stall, in_transfer: false };
        let (code, text) = s.reply().await?;
        if code != 220 {
            return Err(proto(format!("the server did not welcome us: {code} {text}")));
        }
        let (code, text) = s.cmd(&format!("USER {}", t.user)).await?;
        let (code, text) = match code {
            331 | 332 => s.cmd(&format!("PASS {}", t.pass)).await?,
            _ => (code, text),
        };
        if code != 230 && code != 202 {
            return Err(if code == 530 { Error::Status(401) } else { proto(format!("sign-in failed: {code} {text}")) });
        }
        let (code, text) = s.cmd("TYPE I").await?;
        if code != 200 {
            return Err(proto(format!("could not switch to binary mode: {code} {text}")));
        }
        Ok(s)
    }

    /// Read one reply, which may span several lines (`123-...` until `123 ...`).
    async fn reply(&mut self) -> Result<(u16, String)> {
        let mut first: Option<(u16, String)> = None;
        let mut text = String::new();
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout(self.stall, self.ctrl.read_line(&mut line))
                .await
                .map_err(|_| proto("the server stopped answering"))?
                .map_err(|e| proto(e.to_string()))?;
            if n == 0 {
                return Err(proto("the server closed the connection"));
            }
            let line = line.trim_end();
            text.push_str(line);
            text.push('\n');
            let code: Option<u16> = line.get(..3).and_then(|c| c.parse().ok());
            match (&first, code) {
                (None, Some(c)) => {
                    if line.as_bytes().get(3) == Some(&b'-') {
                        first = Some((c, String::new()));
                    } else {
                        return Ok((c, line.get(4..).unwrap_or("").to_string()));
                    }
                }
                (Some((c, _)), Some(k)) if *c == k && line.as_bytes().get(3) == Some(&b' ') => return Ok((k, text.trim().to_string())),
                _ => {}
            }
        }
    }

    async fn cmd(&mut self, line: &str) -> Result<(u16, String)> {
        self.ctrl.get_mut().write_all(format!("{line}\r\n").as_bytes()).await.map_err(|e| proto(e.to_string()))?;
        self.reply().await
    }

    /// Size in bytes.
    pub async fn size(&mut self, path: &str) -> Result<u64> {
        let (code, text) = self.cmd(&format!("SIZE {path}")).await?;
        match code {
            213 => text.trim().parse().map_err(|_| proto(format!("unreadable size: {text}"))),
            550 => Err(Error::Status(404)),
            _ => Err(proto(format!("the server cannot tell the file size ({code} {text})"))),
        }
    }

    /// Modification time as the server reports it, used to notice a changed file.
    pub async fn mdtm(&mut self, path: &str) -> Option<String> {
        match self.cmd(&format!("MDTM {path}")).await {
            Ok((213, t)) => Some(t.trim().to_string()),
            _ => None,
        }
    }

    /// Does the server let us start a transfer part-way (`REST`)? Needed to fetch pieces in parallel.
    pub async fn supports_rest(&mut self) -> bool {
        matches!(self.cmd("REST 1").await, Ok((350, _))) && self.cmd("REST 0").await.is_ok()
    }

    /// Start `RETR` at `start` and return the data connection. The caller reads the bytes it wants and then calls
    /// [`Session::end_transfer`].
    pub async fn open_range(&mut self, path: &str, start: u64) -> Result<TcpStream> {
        let port = self.passive_port().await?;
        let data = connect_bound(SocketAddrV4::new(self.peer, port), &self.route, Duration::from_secs(8))
            .await
            .map_err(|e| proto(format!("data connection failed: {e}")))?;
        if start > 0 {
            let (code, text) = self.cmd(&format!("REST {start}")).await?;
            if code != 350 {
                return Err(proto(format!("the server cannot resume at {start}: {code} {text}")));
            }
        }
        let (code, text) = self.cmd(&format!("RETR {path}")).await?;
        match code {
            125 | 150 => {
                self.in_transfer = true;
                Ok(data)
            }
            550 => Err(Error::Status(404)),
            _ => Err(proto(format!("the server refused the download: {code} {text}"))),
        }
    }

    /// Port for the next data connection. The address in the reply is ignored on purpose: servers behind NAT often send
    /// their private address, and it is always the same host.
    async fn passive_port(&mut self) -> Result<u16> {
        let (code, text) = self.cmd("EPSV").await?;
        if code == 229 {
            if let Some(p) = text.split('|').nth(3).and_then(|p| p.parse().ok()) {
                return Ok(p);
            }
        }
        let (code, text) = self.cmd("PASV").await?;
        if code != 227 {
            return Err(proto(format!("passive mode failed: {code} {text}")));
        }
        let inner = text.split('(').nth(1).and_then(|r| r.split(')').next()).ok_or_else(|| proto("unreadable passive reply"))?;
        let n: Vec<u16> = inner.split(',').filter_map(|v| v.trim().parse().ok()).collect();
        if n.len() != 6 {
            return Err(proto("unreadable passive reply"));
        }
        Ok(n[4] * 256 + n[5])
    }

    /// After the data connection is closed, read the server's final reply for the transfer so the control connection is
    /// ready for the next command. Returns false when it is not (the caller then drops the session).
    pub async fn end_transfer(&mut self) -> bool {
        if !self.in_transfer {
            return true;
        }
        self.in_transfer = false;
        // A 226 (done) or 426 (we closed early) is expected; some servers send both.
        matches!(tokio::time::timeout(Duration::from_secs(3), self.reply()).await, Ok(Ok((226 | 426 | 451 | 250, _))))
    }

    pub async fn quit(mut self) {
        let _ = self.ctrl.get_mut().write_all(b"QUIT\r\n").await;
    }
}

/// The address this host resolves to, for callers that need to know whether it is reachable at all.
pub fn host_ip(t: &Target) -> Option<IpAddr> {
    t.host.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_links_with_and_without_credentials() {
        let t = parse("ftp://files.example.com/pub/a%20b.iso").unwrap();
        assert_eq!(
            (t.host.as_str(), t.port, t.user.as_str(), t.pass.as_str(), t.path.as_str()),
            ("files.example.com", 21, "anonymous", "grabnr@", "/pub/a b.iso")
        );
        let t = parse("ftp://me:p%40ss@host:2121/x/y.zip").unwrap();
        assert_eq!((t.user.as_str(), t.pass.as_str(), t.port, t.path.as_str()), ("me", "p@ss", 2121, "/x/y.zip"));
        assert!(parse("ftp://host/dir/").is_err(), "folders are not files");
        assert!(parse("http://host/a").is_err());
        assert!(looks_like("FTP://x/y") && !looks_like("https://x/y"));
    }

    #[test]
    fn percent_decoding_leaves_bad_escapes_alone() {
        assert_eq!(percent_decode("a%2Fb%zz%"), "a/b%zz%");
        assert_eq!(percent_decode("%41"), "A");
    }
}
