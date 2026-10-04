//! BitTorrent and magnet links.
//!
//! The torrent engine (librqbit) binds a whole session to one network link and cannot hand out pieces across sessions.
//! grabnr therefore splits a multi-file torrent by file: one session per link, each downloading its own share of the
//! files, largest first to the least loaded link. A single-file torrent runs on one link (the fastest). Nothing is
//! uploaded: the sessions stop when the files are complete.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use librqbit::{AddTorrent, AddTorrentOptions, AddTorrentResponse, DhtSessionConfig, ManagedTorrent, Session, SessionOptions};
use tokio_util::sync::CancellationToken;

use crate::download::{Emit, Event, Options, RouteStat, Snapshot};
use crate::error::{Error, Result};

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

/// Split files over `n` links: largest first, each to the link with the least bytes so far. Returns the file indexes per
/// link; a link with nothing to do gets an empty list.
pub fn partition(files: &[(usize, u64)], n: usize) -> Vec<Vec<usize>> {
    let n = n.max(1);
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut load = vec![0u64; n];
    let mut order: Vec<&(usize, u64)> = files.iter().collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for &(idx, len) in order {
        let k = (0..n).min_by_key(|&k| (load[k], k)).unwrap_or(0);
        out[k].push(idx);
        load[k] += len;
    }
    for v in &mut out {
        v.sort_unstable();
    }
    out
}

fn bt(e: impl std::fmt::Display) -> Error {
    Error::Other(format!("torrent: {e}"))
}

async fn session(dir: &std::path::Path, device: Option<String>, opts: &Options) -> Result<Arc<Session>> {
    let mut so = SessionOptions {
        bind_device_name: device,
        // Download only: no listening port, no remembered state between runs.
        persistence: None,
        fastresume: false,
        listen: None,
        disable_local_service_discovery: true,
        ipv4_only: true,
        ..Default::default()
    };
    if opts.torrent_peers.is_empty() {
        so.dht = Some(DhtSessionConfig { persistence: None, ..Default::default() });
    } else {
        // Peers were given explicitly (tests, or a private tracker-less swarm): no need to look anywhere else.
        so.dht = None;
        so.disable_trackers = true;
    }
    Session::new_with_opts(dir.to_path_buf(), so).await.map_err(bt)
}

struct Part {
    session: Arc<Session>,
    handle: Arc<ManagedTorrent>,
    link: String,
    files: Vec<usize>,
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

    // 1. Learn what is in it (on the first link).
    let device_of = |i: usize| opts.routes[i].link.as_ref().map(|l| l.name.clone());
    let lister = session(&dir, device_of(0), opts).await?;
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

    // 2. One session per link that has files to fetch. A single-file torrent uses the fastest link only.
    let usable: Vec<usize> = if multi {
        (0..opts.routes.len()).collect()
    } else {
        let best =
            (0..opts.routes.len()).max_by_key(|&i| opts.routes[i].link.as_ref().and_then(|l| l.link_speed_mbps).unwrap_or(0)).unwrap_or(0);
        vec![best]
    };
    let shares = partition(&files, usable.len());
    emit(Event::Started { filename: name.clone(), total: Some(total), chunks: files.len(), ranges: true, resumed_chunks: 0 });

    let mut parts: Vec<Part> = Vec::new();
    for (k, share) in shares.iter().enumerate().filter(|(_, s)| !s.is_empty()) {
        let route = usable[k];
        let sess = session(&dir, device_of(route), opts).await?;
        let add = AddTorrentOptions {
            only_files: Some(share.clone()),
            overwrite: true,
            // A multi-file torrent goes into a folder of its own, named after the torrent.
            sub_folder: multi.then(|| folder.clone()),
            initial_peers: some_peers(&opts.torrent_peers),
            ..Default::default()
        };
        let resp = sess.add_torrent(AddTorrent::from_bytes(torrent_bytes.clone()), Some(add)).await.map_err(bt)?;
        let handle = resp.into_handle().ok_or_else(|| bt("the torrent was not started"))?;
        parts.push(Part { session: sess, handle, link: opts.routes[route].name.clone(), files: share.clone() });
    }

    // 3. Report progress until everything is on disk.
    let mut announced = vec![false; files.len()];
    let result = loop {
        let mut done_bytes = 0u64;
        let mut stats = Vec::new();
        let mut all_done = true;
        for p in &parts {
            let s = p.handle.stats();
            done_bytes += p.files.iter().map(|&f| s.file_progress.get(f).copied().unwrap_or(0)).sum::<u64>();
            all_done &= s.finished;
            for &f in &p.files {
                if !announced[f] && s.file_progress.get(f).copied().unwrap_or(0) >= files[f].1 {
                    announced[f] = true;
                    let route = parts.iter().position(|q| q.link == p.link).unwrap_or(0);
                    emit(Event::ChunkDone { idx: f, route });
                }
            }
            let bps = s.live.as_ref().map(|l| l.download_speed.mbps * 1024.0 * 1024.0).unwrap_or(0.0);
            stats.push((
                p.link.clone(),
                p.files.iter().map(|&f| s.file_progress.get(f).copied().unwrap_or(0)).sum::<u64>(),
                bps,
                s.error.clone(),
            ));
        }
        if let Some(e) = stats.iter().find_map(|s| s.3.clone()) {
            break Err(bt(e));
        }
        let speed: f64 = stats.iter().map(|s| s.2).sum();
        emit(Event::Progress(Snapshot {
            downloaded: done_bytes.min(total),
            total: Some(total),
            bytes_per_sec: speed,
            routes: stats
                .iter()
                .map(|(n, b, v, _)| RouteStat { name: n.clone(), bytes: *b, bytes_per_sec: *v, connections: 0, down: false })
                .collect(),
        }));
        if all_done {
            break Ok(());
        }
        tokio::select! {
            _ = cancel.cancelled() => break Err(Error::Cancelled),
            _ = tokio::time::sleep(Duration::from_millis(250)) => {}
        }
    };
    for p in &parts {
        p.session.stop().await;
    }
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

    #[test]
    fn partitions_files_by_size_over_links() {
        let files = vec![(0, 10), (1, 40), (2, 30), (3, 20)];
        let p = partition(&files, 2);
        let sum = |v: &Vec<usize>| v.iter().map(|&i| files[i].1).sum::<u64>();
        // 40 -> link 0, 30 -> link 1, 20 -> link 1 (30 < 40), 10 -> link 0: both links end up with 50 bytes.
        assert_eq!(p, vec![vec![0, 1], vec![2, 3]]);
        assert_eq!(sum(&p[0]), sum(&p[1]));
        // every file is assigned exactly once
        let mut all: Vec<usize> = p.concat();
        all.sort();
        assert_eq!(all, vec![0, 1, 2, 3]);
        assert_eq!(partition(&files, 1), vec![vec![0, 1, 2, 3]]);
        let few = partition(&[(0, 5)], 3);
        assert_eq!(few, vec![vec![0], vec![], vec![]], "more links than files: the extra links stay idle");
    }
}
