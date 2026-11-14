//! A local SOCKS5 proxy that spreads outgoing TCP connections over several links.
//!
//! The torrent engine binds a whole session to one interface, but it can send every peer connection through a SOCKS5
//! proxy. Pointing it at this proxy lets one session (and so one file) pull from peers over all links at once: each new
//! connection leaves through the link that carries the fewest connections for its share, and falls over to the next
//! link when that one cannot reach the peer.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

use crate::download::Route;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
struct Counter {
    active: AtomicUsize,
    /// Bytes received from the far end over this link.
    bytes: AtomicU64,
}

/// One link the proxy can send connections over. Slots are never removed, so a link keeps its index (and its totals)
/// when it goes away and comes back.
struct Slot {
    route: Route,
    /// Relative share of new connections (from `Route::max_conns`; equal when unset).
    weight: usize,
    /// The link is gone (unplugged, Wi-Fi off, asleep): no new connections go out over it.
    down: bool,
    counter: Arc<Counter>,
}

impl Slot {
    fn new(route: Route) -> Self {
        Slot { weight: weight_of(&route), route, down: false, counter: Arc::default() }
    }
}

fn weight_of(r: &Route) -> usize {
    r.max_conns.unwrap_or(100).max(1)
}

struct Shared {
    slots: RwLock<Vec<Slot>>,
    next: AtomicUsize,
}

impl Shared {
    /// Links in the order a new connection should try them: least loaded for its weight first, ties taken in turn.
    fn order(&self) -> Vec<(Route, Arc<Counter>)> {
        let slots = self.slots.read().unwrap();
        let live: Vec<usize> = (0..slots.len()).filter(|&k| !slots[k].down).collect();
        if live.is_empty() {
            return Vec::new();
        }
        let n = live.len();
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        let mut idx: Vec<usize> = (0..n).map(|k| live[(start + k) % n]).collect();
        // active / weight, compared without division: a/wa < b/wb  <=>  a*wb < b*wa. Stable sort keeps the rotation for ties.
        idx.sort_by(|&a, &b| {
            let la = slots[a].counter.active.load(Ordering::Relaxed) * slots[b].weight;
            let lb = slots[b].counter.active.load(Ordering::Relaxed) * slots[a].weight;
            la.cmp(&lb)
        });
        idx.into_iter().map(|k| (slots[k].route.clone(), slots[k].counter.clone())).collect()
    }
}

/// Per-link totals of a running [`Spread`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkUse {
    pub name: String,
    pub connections: usize,
    pub bytes: u64,
    pub down: bool,
}

/// The running proxy. It stops when dropped.
pub struct Spread {
    addr: SocketAddr,
    shared: Arc<Shared>,
    cancel: CancellationToken,
}

impl Spread {
    /// Listen on a free loopback port and spread connections over `routes`.
    pub async fn start(routes: Vec<Route>) -> std::io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared { slots: RwLock::new(routes.into_iter().map(Slot::new).collect()), next: AtomicUsize::new(0) });
        let cancel = CancellationToken::new();
        let (sh, c) = (shared.clone(), cancel.clone());
        tokio::spawn(async move {
            loop {
                let sock = tokio::select! {
                    _ = c.cancelled() => return,
                    r = listener.accept() => match r {
                        Ok((s, _)) => s,
                        Err(_) => continue,
                    },
                };
                let (sh, c) = (sh.clone(), c.clone());
                tokio::spawn(async move {
                    tokio::select! {
                        _ = c.cancelled() => {}
                        _ = serve(sock, sh) => {}
                    }
                });
            }
        });
        Ok(Spread { addr, shared, cancel })
    }

    /// `socks5://127.0.0.1:<port>`, for `librqbit::ConnectionOptions::proxy_url`.
    pub fn url(&self) -> String {
        format!("socks5://{}", self.addr)
    }

    /// Bring the links in line with the ones usable now: new links join, links that vanished stop taking new
    /// connections, and a link that came back (possibly with a new address) is used again. Returns whether anything
    /// changed.
    pub fn set_routes(&self, desired: &[Route]) -> bool {
        let mut slots = self.shared.slots.write().unwrap();
        let mut changed = false;
        for d in desired {
            match slots.iter_mut().find(|s| s.route.name == d.name) {
                Some(s) => {
                    if s.down || s.route.key() != d.key() {
                        changed = true;
                    }
                    s.route = d.clone();
                    s.weight = weight_of(d);
                    s.down = false;
                }
                None => {
                    slots.push(Slot::new(d.clone()));
                    changed = true;
                }
            }
        }
        for s in slots.iter_mut().filter(|s| !s.down && !desired.iter().any(|d| d.name == s.route.name)) {
            s.down = true;
            changed = true;
        }
        changed
    }

    /// Open connections and bytes received so far, one entry per link, in the order the links were first seen.
    pub fn usage(&self) -> Vec<LinkUse> {
        self.shared
            .slots
            .read()
            .unwrap()
            .iter()
            .map(|s| LinkUse {
                name: s.route.name.clone(),
                connections: s.counter.active.load(Ordering::Relaxed),
                bytes: s.counter.bytes.load(Ordering::Relaxed),
                down: s.down,
            })
            .collect()
    }
}

impl Drop for Spread {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Where the client asked to go.
#[derive(Debug, PartialEq, Eq)]
enum Target {
    Ip(SocketAddrV4),
    Name(String, u16),
    Unsupported,
}

/// Read the SOCKS5 greeting and CONNECT request (no authentication). `None` for anything else.
async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(s: &mut S) -> std::io::Result<Option<Target>> {
    let mut head = [0u8; 2];
    s.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Ok(None);
    }
    let mut methods = vec![0u8; head[1] as usize];
    s.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        s.write_all(&[5, 0xff]).await?;
        return Ok(None);
    }
    s.write_all(&[5, 0]).await?;

    let mut req = [0u8; 4];
    s.read_exact(&mut req).await?;
    if req[0] != 5 {
        return Ok(None);
    }
    let target = match req[3] {
        1 => {
            let mut a = [0u8; 6];
            s.read_exact(&mut a).await?;
            Target::Ip(SocketAddrV4::new(Ipv4Addr::new(a[0], a[1], a[2], a[3]), u16::from_be_bytes([a[4], a[5]])))
        }
        3 => {
            let mut len = [0u8; 1];
            s.read_exact(&mut len).await?;
            let mut name = vec![0u8; len[0] as usize + 2];
            s.read_exact(&mut name).await?;
            let port = u16::from_be_bytes([name[name.len() - 2], name[name.len() - 1]]);
            name.truncate(name.len() - 2);
            Target::Name(String::from_utf8_lossy(&name).into_owned(), port)
        }
        4 => {
            let mut a = [0u8; 18];
            s.read_exact(&mut a).await?;
            Target::Unsupported
        }
        _ => return Ok(None),
    };
    if req[1] != 1 {
        reply(s, 7).await?; // command not supported
        return Ok(None);
    }
    Ok(Some(target))
}

async fn reply<S: AsyncWrite + Unpin>(s: &mut S, code: u8) -> std::io::Result<()> {
    s.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0]).await
}

async fn serve(mut client: TcpStream, sh: Arc<Shared>) -> std::io::Result<()> {
    let Some(target) = handshake(&mut client).await? else { return Ok(()) };
    let addrs: Vec<SocketAddrV4> = match target {
        Target::Ip(a) => vec![a],
        Target::Name(host, port) => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map(|it| {
                it.filter_map(|a| match a {
                    SocketAddr::V4(v4) => Some(v4),
                    SocketAddr::V6(_) => None,
                })
                .collect()
            })
            .unwrap_or_default(),
        Target::Unsupported => vec![],
    };
    let Some(&addr) = addrs.first() else {
        return reply(&mut client, 8).await; // address type not supported / unresolvable
    };

    let mut upstream = None;
    for (route, counter) in sh.order() {
        // Count the attempt so connections opened at the same moment spread out instead of all picking the same link.
        counter.active.fetch_add(1, Ordering::Relaxed);
        match crate::ftp::connect_bound(addr, &route, CONNECT_TIMEOUT).await {
            Ok(s) => {
                upstream = Some((counter, s));
                break;
            }
            Err(_) => {
                counter.active.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
    let Some((counter, upstream)) = upstream else {
        return reply(&mut client, 5).await; // connection refused
    };
    let _done = ActiveGuard(counter.clone());
    reply(&mut client, 0).await?;

    let (mut cr, mut cw) = client.into_split();
    let (mut ur, mut uw) = upstream.into_split();
    let up = async {
        let r = tokio::io::copy(&mut cr, &mut uw).await;
        let _ = uw.shutdown().await;
        r.map(|_| ())
    };
    let down = async {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            let n = ur.read(&mut buf).await?;
            if n == 0 {
                let _ = cw.shutdown().await;
                return Ok::<_, std::io::Error>(());
            }
            cw.write_all(&buf[..n]).await?;
            counter.bytes.fetch_add(n as u64, Ordering::Relaxed);
        }
    };
    // Peer connections are not half-closed: once either side is done, close both. Waiting for both directions would
    // keep a connection whose peer vanished (sleep, dropped Wi-Fi) open and counted forever, because nothing ever
    // arrives on it again to say so.
    tokio::select! {
        _ = up => {}
        _ = down => {}
    }
    Ok(())
}

struct ActiveGuard(Arc<Counter>);

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn echo_server() -> SocketAddrV4 {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let SocketAddr::V4(addr) = l.local_addr().unwrap() else { unreachable!() };
        tokio::spawn(async move {
            loop {
                let (mut s, _) = l.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        let n = s.read(&mut buf).await.unwrap_or(0);
                        if n == 0 || s.write_all(&buf[..n]).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        addr
    }

    async fn socks_connect(proxy: SocketAddr, to: SocketAddrV4) -> TcpStream {
        let mut s = TcpStream::connect(proxy).await.unwrap();
        s.write_all(&[5, 1, 0]).await.unwrap();
        let mut r = [0u8; 2];
        s.read_exact(&mut r).await.unwrap();
        assert_eq!(r, [5, 0]);
        let mut req = vec![5, 1, 0, 1];
        req.extend_from_slice(&to.ip().octets());
        req.extend_from_slice(&to.port().to_be_bytes());
        s.write_all(&req).await.unwrap();
        let mut rep = [0u8; 10];
        s.read_exact(&mut rep).await.unwrap();
        assert_eq!(rep[1], 0, "the proxy connects");
        s
    }

    #[tokio::test]
    async fn connections_alternate_over_links_and_bytes_are_counted() {
        let echo = echo_server().await;
        let spread = Spread::start(vec![Route::unbound("a"), Route::unbound("b")]).await.unwrap();
        let mut a = socks_connect(spread.addr, echo).await;
        let mut b = socks_connect(spread.addr, echo).await;
        let usage = spread.usage();
        assert_eq!(usage.iter().map(|u| u.connections).collect::<Vec<_>>(), vec![1, 1], "one connection on each link");

        for s in [&mut a, &mut b] {
            s.write_all(b"hello").await.unwrap();
            let mut got = [0u8; 5];
            s.read_exact(&mut got).await.unwrap();
            assert_eq!(&got, b"hello");
        }
        assert_eq!(spread.usage().iter().map(|u| u.bytes).collect::<Vec<_>>(), vec![5, 5]);

        drop(a);
        // The first link frees up, so the next connection goes there.
        for _ in 0..50 {
            if spread.usage()[0].connections == 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let _c = socks_connect(spread.addr, echo).await;
        assert_eq!(spread.usage().iter().map(|u| u.connections).collect::<Vec<_>>(), vec![1, 1]);
    }

    #[tokio::test]
    async fn a_heavier_link_takes_more_connections() {
        let echo = echo_server().await;
        let mut heavy = Route::unbound("heavy");
        heavy.max_conns = Some(3);
        let mut light = Route::unbound("light");
        light.max_conns = Some(1);
        let spread = Spread::start(vec![heavy, light]).await.unwrap();
        let mut keep = Vec::new();
        for _ in 0..4 {
            keep.push(socks_connect(spread.addr, echo).await);
        }
        assert_eq!(spread.usage().iter().map(|u| u.connections).collect::<Vec<_>>(), vec![3, 1]);
    }

    /// A peer that accepts and then never says or reads anything again (it went to sleep with the Wi-Fi).
    async fn silent_server() -> SocketAddrV4 {
        let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let SocketAddr::V4(addr) = l.local_addr().unwrap() else { unreachable!() };
        tokio::spawn(async move {
            let mut keep = Vec::new();
            loop {
                let (s, _) = l.accept().await.unwrap();
                keep.push(s);
            }
        });
        addr
    }

    #[tokio::test]
    async fn closing_the_client_frees_a_link_even_when_the_peer_is_silent() {
        let silent = silent_server().await;
        let spread = Spread::start(vec![Route::unbound("a")]).await.unwrap();
        let c = socks_connect(spread.addr, silent).await;
        assert_eq!(spread.usage()[0].connections, 1);
        drop(c);
        for _ in 0..100 {
            if spread.usage()[0].connections == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the connection to a silent peer stayed counted after the client closed it");
    }

    #[tokio::test]
    async fn a_vanished_link_takes_no_new_connections_until_it_returns() {
        let echo = echo_server().await;
        let spread = Spread::start(vec![Route::unbound("a"), Route::unbound("b")]).await.unwrap();
        assert!(spread.set_routes(&[Route::unbound("b")]), "a went away");
        assert!(!spread.set_routes(&[Route::unbound("b")]), "nothing changed");
        let mut keep = Vec::new();
        for _ in 0..3 {
            keep.push(socks_connect(spread.addr, echo).await);
        }
        let usage = spread.usage();
        assert_eq!(usage.iter().map(|u| (u.name.as_str(), u.connections, u.down)).collect::<Vec<_>>(), vec![("a", 0, true), ("b", 3, false)]);

        assert!(spread.set_routes(&[Route::unbound("a"), Route::unbound("b"), Route::unbound("c")]), "a is back, c is new");
        keep.push(socks_connect(spread.addr, echo).await);
        let usage = spread.usage();
        assert_eq!(usage.len(), 3, "links keep their place");
        assert!(!usage[0].down);
        assert_eq!(usage[1].connections, 3, "the new connection went to a less loaded link");
    }
}
