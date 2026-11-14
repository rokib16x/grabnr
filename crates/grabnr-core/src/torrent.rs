//! BitTorrent and magnet links.
//!
//! The torrent engine (librqbit) binds a whole session to one network link. With several links, grabnr instead sends
//! every peer connection through a local SOCKS5 proxy ([`crate::spread`]) that opens each one over the least busy link,
//! so one session, and even a single file, downloads over all links at once: each peer is reached over one link, and
//! different peers over different links. Nothing is uploaded: the session stops when the files are complete.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, DhtSessionConfig, PeerConnectionOptions, Session,
    SessionOptions, TorrentStatsState,
};
use tokio_util::sync::CancellationToken;

use crate::download::{Emit, Event, Options, RouteStat, Snapshot};
use crate::error::{Error, Result};
use crate::spread::Spread;

/// A magnet link, or a link to a `.torrent` file.
pub fn looks_like(url: &str) -> bool {
    let u = url.trim();
    u.get(..7).is_some_and(|p| p.eq_ignore_ascii_case("magnet:"))
        || (url::Url::parse(u).is_ok_and(|p| matches!(p.scheme(), "http" | "https"))
            && u.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase().ends_with(".torrent"))
}

/// The display name inside a magnet link (`dn=`), if it has one.
pub fn magnet_name(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    u.query_pairs().find(|(k, _)| k == "dn").map(|(_, v)| v.into_owned()).filter(|v| !v.is_empty())
}

/// How long a peer may stay silent before its connection is dropped. A peer that is choking us sends nothing but a
/// keep-alive every two minutes, so librqbit's default of 10 s drops perfectly good peers over and over. Every drop
/// also pushes that peer's next attempt further out (10 s, 1 min, 6 min, 36 min, then hourly), which is what made
/// downloads run in bursts and then slow to a crawl.
const PEER_TIMEOUT: Duration = Duration::from_secs(150);
/// Send our own keep-alive well inside the two minutes other clients wait for.
const KEEP_ALIVE: Duration = Duration::from_secs(60);
/// With no data at all for this long, rebuild the peer list: ask the trackers and DHT again and retry every peer
/// right away instead of waiting out the backoff they piled up while the computer slept or the network was down.
const STALL_FIRST: Duration = Duration::from_secs(90);
/// The wait doubles after each rebuild that did not help, up to this, so a swarm with no seeders is not hammered.
const STALL_MAX: Duration = Duration::from_secs(600);

fn bt(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("torrent: {e}"))
}

/// How the session reaches peers: bound to one interface, or through a local proxy that spreads them over several links.
#[derive(Clone)]
enum Via {
    Device(Option<String>),
    Proxy(String),
}

async fn session(dir: &std::path::Path, via: &Via, opts: &Options) -> Result<Arc<Session>> {
    let mut so = SessionOptions {
        // Download only: no listening port, no remembered state between runs.
        persistence: None,
        fastresume: false,
        listen: None,
        disable_local_service_discovery: true,
        ipv4_only: true,
        ..Default::default()
    };
    let peer_opts =
        Some(PeerConnectionOptions { connect_timeout: None, read_write_timeout: Some(PEER_TIMEOUT), keep_alive_interval: Some(KEEP_ALIVE) });
    let proxy_url = match via {
        Via::Device(d) => {
            so.bind_device_name = d.clone();
            None
        }
        Via::Proxy(url) => Some(url.clone()),
    };
    so.connect = Some(ConnectionOptions { proxy_url, peer_opts, ..Default::default() });
    if opts.torrent_peers.is_empty() {
        so.dht = Some(DhtSessionConfig { persistence: None, ..Default::default() });
    } else {
        // Peers were given explicitly (tests, or a private tracker-less swarm): no need to look anywhere else.
        so.dht = None;
        so.disable_trackers = true;
    }
    Session::new_with_opts(dir.to_path_buf(), so).await.map_err(bt)
}

/// Download a torrent into `opts.dest_dir`. Returns the file, or the folder of a multi-file torrent.
pub async fn download(opts: &Options, cancel: CancellationToken, emit: Emit) -> Result<PathBuf> {
    if opts.routes.is_empty() {
        return Err(Error::Other("no network routes selected".into()));
    }
    std::fs::create_dir_all(&opts.dest_dir)?;
    let dir = opts.dest_dir.clone();

    // The torrent file: fetched over the web, or the magnet link itself (metadata then comes from the swarm).
    let source = if url::Url::parse(&opts.url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
        let (bytes, _) = crate::fetch::fetch_bytes(&opts.url, &opts.headers, opts.proxy.as_deref(), 8 << 20).await?;
        AddTorrent::from_bytes(bytes)
    } else {
        AddTorrent::from_url(opts.url.clone())
    };

    // Several links, or links that may change while it runs (Wi-Fi gone during sleep, a cable plugged in): every peer
    // connection goes through the spreading proxy, which follows the links and pulls even a single file over all of
    // them. One fixed link: bind the session to it.
    let spread = if opts.routes.len() > 1 || opts.link_watch.is_some() { Some(Spread::start(opts.routes.clone()).await?) } else { None };
    let via = match &spread {
        Some(s) => Via::Proxy(s.url()),
        None => Via::Device(opts.routes[0].link.as_ref().map(|l| l.name.clone())),
    };

    // 1. Learn what is in it.
    let lister = session(&dir, &via, opts).await?;
    let list_opts = AddTorrentOptions { list_only: true, initial_peers: some_peers(&opts.torrent_peers), ..Default::default() };
    let listed = tokio::select! {
        _ = cancel.cancelled() => { lister.stop().await; return Err(Error::Cancelled) }
        r = tokio::time::timeout(Duration::from_secs(90), lister.add_torrent(source, Some(list_opts))) => match r {
            Ok(r) => r.map_err(bt)?,
            Err(_) => { lister.stop().await; return Err(Error::Other("torrent: could not find the torrent's file list; no peers answered".into())) }
        },
    };
    let AddTorrentResponse::ListOnly(info) = listed else { return Err(bt("unexpected answer while listing the files")) };
    lister.stop().await;
    let name = info.info.name().map(|n| n.into_owned()).unwrap_or_else(|| "torrent".into());
    let files: Vec<(usize, u64)> = info.info.iter_file_lengths().enumerate().collect();
    let total: u64 = files.iter().map(|f| f.1).sum();
    let torrent_bytes = info.torrent_bytes.clone();
    let multi = files.len() > 1;
    // The torrent's own name is untrusted: keep it a plain file or folder name.
    let folder = crate::probe::sanitize(&name);
    crate::disk::ensure_space(&dir, total)?;

    // 2. Fetch it.
    emit(Event::Started { filename: name.clone(), total: Some(total), chunks: files.len(), ranges: true, resumed_chunks: 0 });
    let sess = session(&dir, &via, opts).await?;
    let add = AddTorrentOptions {
        overwrite: true,
        // A multi-file torrent goes into a folder of its own, named after the torrent.
        sub_folder: multi.then(|| folder.clone()),
        initial_peers: some_peers(&opts.torrent_peers),
        ..Default::default()
    };
    let resp = sess.add_torrent(AddTorrent::from_bytes(torrent_bytes), Some(add)).await.map_err(bt)?;
    let handle = resp.into_handle().ok_or_else(|| bt("the torrent was not started"))?;

    // 3. Report progress until everything is on disk.
    let mut announced = vec![false; files.len()];
    let mut rates: Vec<f64> = Vec::new();
    let mut last: Option<(Instant, Vec<u64>)> = None;
    // Stall watchdog: what was last seen arriving, since when nothing new has, and how long to allow before acting.
    let (mut seen, mut quiet_since, mut stall_after) = ((0u64, 0u64), Instant::now(), STALL_FIRST);
    let mut last_watch = Instant::now();
    let result = loop {
        let s = handle.stats();
        if let Some(e) = s.error.clone() {
            break Err(bt(e));
        }
        let live = matches!(s.state, TorrentStatsState::Live);
        let now_seen = (s.progress_bytes, s.live.as_ref().map(|l| l.snapshot.fetched_bytes).unwrap_or(0));
        if now_seen != seen || !live {
            if live && now_seen.0 > seen.0 {
                stall_after = STALL_FIRST; // pieces are arriving again
            }
            seen = now_seen;
            quiet_since = Instant::now();
        }
        let mut rebuild = live && !s.finished && quiet_since.elapsed() >= stall_after;
        if rebuild {
            stall_after = (stall_after * 2).min(STALL_MAX);
        }
        // Follow the links: one that vanished takes no new peers, one that came back (after sleep) or is new is used.
        if let (Some(sp), Some(watch)) = (&spread, &opts.link_watch) {
            if last_watch.elapsed() >= opts.watch_interval {
                last_watch = Instant::now();
                let w = watch.clone();
                if let Ok(desired) = tokio::task::spawn_blocking(move || w()).await {
                    // Back online, or on other links: the peers tried meanwhile all failed, so start over with them.
                    if sp.set_routes(&desired) && !desired.is_empty() {
                        rebuild = live && !s.finished;
                    }
                }
            }
        }
        if rebuild {
            // Pausing drops every peer with its backoff; resuming announces to trackers and DHT afresh. The pieces
            // already on disk are kept as they are, nothing is checked again.
            let _ = sess.pause(&handle).await;
            if let Err(e) = sess.unpause(&handle).await {
                break Err(bt(e));
            }
            quiet_since = Instant::now();
        }
        let done_bytes: u64 = files.iter().map(|&(f, _)| s.file_progress.get(f).copied().unwrap_or(0)).sum();
        let speed = s.live.as_ref().map(|l| l.download_speed.mbps * 1024.0 * 1024.0).unwrap_or(0.0);
        let routes: Vec<RouteStat> = match &spread {
            None => {
                vec![RouteStat { name: opts.routes[0].name.clone(), bytes: done_bytes, bytes_per_sec: speed, connections: 0, down: false }]
            }
            Some(sp) => {
                let usage = sp.usage();
                let now = Instant::now();
                let bytes: Vec<u64> = usage.iter().map(|u| u.bytes).collect();
                rates.resize(usage.len(), 0.0);
                if let Some((t, prev)) = &last {
                    let dt = now.duration_since(*t).as_secs_f64().max(0.05);
                    for (k, r) in rates.iter_mut().enumerate() {
                        // A link that joined since the last look has no earlier reading.
                        let inst = bytes[k].saturating_sub(prev.get(k).copied().unwrap_or(bytes[k])) as f64 / dt;
                        *r = 0.7 * *r + 0.3 * inst;
                    }
                }
                last = Some((now, bytes));
                usage
                    .into_iter()
                    .zip(&rates)
                    .map(|(u, &v)| RouteStat {
                        name: u.name,
                        bytes: u.bytes,
                        bytes_per_sec: v,
                        connections: u.connections,
                        down: u.down,
                    })
                    .collect()
            }
        };
        let busiest = (0..routes.len()).max_by_key(|&k| routes[k].bytes).unwrap_or(0);
        for &(f, len) in &files {
            if !announced[f] && s.file_progress.get(f).copied().unwrap_or(0) >= len {
                announced[f] = true;
                emit(Event::ChunkDone { idx: f, route: busiest });
            }
        }
        emit(Event::Progress(Snapshot { downloaded: done_bytes.min(total), total: Some(total), bytes_per_sec: speed, routes }));
        if s.finished {
            break Ok(());
        }
        tokio::select! {
            _ = cancel.cancelled() => break Err(Error::Cancelled),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    };
    sess.stop().await;
    drop(spread);
    result?;
    Ok(dir.join(&folder))
}

fn some_peers(p: &[SocketAddr]) -> Option<Vec<SocketAddr>> {
    (!p.is_empty()).then(|| p.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_torrent_links() {
        assert!(looks_like("magnet:?xt=urn:btih:abcdef&dn=Some+Show"));
        assert!(looks_like("MAGNET:?xt=urn:btih:abc"));
        assert!(looks_like("https://tracker.example/files/ubuntu.iso.torrent?passkey=1"));
        assert!(!looks_like("https://example.com/file.zip"));
        assert!(!looks_like("ftp://x/a.torrent"), "only web links to .torrent files");
    }

    #[test]
    fn reads_the_display_name_of_a_magnet() {
        assert_eq!(magnet_name("magnet:?xt=urn:btih:abc&dn=Linux+ISOs%20Pack").as_deref(), Some("Linux ISOs Pack"));
        assert_eq!(magnet_name("magnet:?xt=urn:btih:abc"), None);
    }
}
