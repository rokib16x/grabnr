use clap::{Parser, Subcommand};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use grabnr_core::{interfaces::shared_gateways, list_links, spike, Error, Event, Options, Route, Store};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(name = "grabnr", about = "Combine every network connection into one fast download")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List usable network links
    Links,
    /// Download a URL over every active link (Ctrl-C pauses; run again to resume)
    Get {
        url: String,
        /// Output folder
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
        /// Interfaces to use (default: all usable links), e.g. en0,en5
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Connections per link
        #[arg(long, default_value_t = 8)]
        conns: usize,
        /// Extra request header, "Name: value" (repeatable)
        #[arg(short = 'H', long = "header")]
        headers: Vec<String>,
        /// Cap the total speed, in MB/s (fractions allowed)
        #[arg(long)]
        limit: Option<f64>,
        /// Verify the file against this hash (`sha256:<hex>`, `sha1:<hex>`, `md5:<hex>`, or bare hex)
        #[arg(long)]
        checksum: Option<String>,
        /// Proxy for every link: http://host:port or socks5://host:port
        #[arg(long)]
        proxy: Option<String>,
        /// Basic auth as user:password
        #[arg(long)]
        user: Option<String>,
        /// Bearer token
        #[arg(long)]
        bearer: Option<String>,
    },
    /// Check that interface binding works and that links add up
    Spike {
        /// Interfaces to use (default: all usable links), e.g. en0,en5
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// Seconds per throughput test
        #[arg(long, default_value_t = 8)]
        secs: u64,
        #[arg(long, default_value = spike::DEFAULT_TEST_URL)]
        url: String,
        /// Print JSON instead of text
        #[arg(long)]
        json: bool,
    },
}

#[tokio::main]
async fn main() {
    match Cli::parse().cmd {
        Cmd::Links => {
            let links = list_links();
            for l in &links {
                println!(
                    "{:<8} {:<10} {:<15} gw={:<15} {}{}",
                    l.name,
                    format!("{:?}", l.kind),
                    l.ipv4,
                    l.gateway.map(|g| g.to_string()).unwrap_or_else(|| "-".into()),
                    l.label,
                    if l.is_default_route { "  [default route]" } else { "" }
                );
            }
            for (a, b) in shared_gateways(&links) {
                println!("warning: {a} and {b} share a gateway and will not add bandwidth");
            }
        }
        Cmd::Get { url, out, only, conns, headers, limit, checksum, proxy, user, bearer } => {
            let mut links = list_links();
            if !only.is_empty() {
                links.retain(|l| only.contains(&l.name));
            }
            if links.is_empty() {
                eprintln!("no usable links found");
                std::process::exit(1);
            }
            let routes: Vec<Route> = links.iter().map(Route::from_link).collect();
            eprintln!("using: {}", links.iter().map(|l| format!("{} ({})", l.name, l.label)).collect::<Vec<_>>().join(", "));

            let db = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".grabnr");
            std::fs::create_dir_all(&db).ok();
            let mut opts = Options::new(url, out, routes);
            opts.conns_per_route = conns;
            opts.proxy = proxy;
            opts.auth = match (bearer, user) {
                (Some(t), _) => Some(grabnr_core::Auth::Bearer(t)),
                (None, Some(u)) => {
                    let (user, pass) = u.split_once(':').unwrap_or((u.as_str(), ""));
                    Some(grabnr_core::Auth::Basic { user: user.into(), pass: pass.into() })
                }
                _ => None,
            };
            opts.speed_limit = limit.filter(|l| *l > 0.0).map(|l| (l * 1024.0 * 1024.0) as u64);
            if let Some(c) = checksum {
                opts.checksum = Some(grabnr_core::Checksum::parse(&c).unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(2);
                }));
            }
            opts.store = Some(Arc::new(Store::open(&db.join("state.db")).expect("open state db")));
            opts.headers = headers
                .iter()
                .filter_map(|h| h.split_once(':').map(|(k, v)| (k.trim().to_string(), v.trim().to_string())))
                .collect();

            let cancel = CancellationToken::new();
            let c = cancel.clone();
            tokio::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                eprintln!("\npausing...");
                c.cancel();
            });
            let emit: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(|e| match e {
                Event::Started { filename, total, chunks, ranges, resumed_chunks } => eprintln!(
                    "{filename}: {} in {chunks} chunk(s){}{}",
                    total.map(human).unwrap_or_else(|| "unknown size".into()),
                    if ranges { "" } else { " (server has no range support: single stream)" },
                    if resumed_chunks > 0 { format!(", resuming with {resumed_chunks} done") } else { String::new() }
                ),
                Event::ResumeDiscarded { reason } => eprintln!("starting over: {reason}"),
                Event::RouteDown { route, reason } => eprintln!("\nlink #{route} dropped out: {reason}"),
                Event::Progress(s) => {
                    let pct = s.total.filter(|t| *t > 0).map(|t| format!("{:>3}%", s.downloaded * 100 / t)).unwrap_or_default();
                    let per: Vec<String> = s.routes.iter().map(|r| format!("{} {}/s", r.name, human(r.bytes_per_sec as u64))).collect();
                    eprint!("\r{pct} {} at {}/s  [{}]   ", human(s.downloaded), human(s.bytes_per_sec as u64), per.join("  "));
                    let _ = std::io::stderr().flush();
                }
                Event::Finished { path } => eprintln!("\nsaved {path}"),
                Event::ChunkDone { .. } => {}
            });
            match grabnr_core::download(opts, cancel, emit).await {
                Ok(_) => {}
                Err(Error::Cancelled) => {
                    eprintln!("paused; run the same command to resume");
                    std::process::exit(130);
                }
                Err(e) => {
                    eprintln!("\nerror: {e}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::Spike { only, secs, url, json } => {
            let mut links = list_links();
            if !only.is_empty() {
                links.retain(|l| only.contains(&l.name));
            }
            if links.is_empty() {
                eprintln!("no usable links found");
                std::process::exit(1);
            }
            eprintln!("testing {} link(s), {secs}s each...", links.len());
            let report = spike::run(&links, &url, secs).await;
            if json {
                println!("{}", serde_json::to_string_pretty(&report).unwrap());
                return;
            }
            println!("baseline public IP (OS routing): {}", report.baseline_ip.as_deref().unwrap_or("?"));
            for r in &report.links {
                println!("\n{}", r.link);
                for m in &r.modes {
                    println!(
                        "  {:<11} ip={:<16} {:?}{}",
                        m.mode.label(),
                        m.public_ip.as_deref().unwrap_or("-"),
                        m.verdict,
                        m.error.as_ref().map(|e| format!("  ({e})")).unwrap_or_default()
                    );
                }
                println!("  solo: {:.1} Mbps{}", r.solo.mbps, r.solo.error.as_ref().map(|e| format!("  ({e})")).unwrap_or_default());
            }
            println!("\ncombined: {:.1} Mbps  (best single link: {:.1} Mbps)", report.combined_mbps, report.best_solo_mbps);
        }
    }
}

fn human(b: u64) -> String {
    const U: [&str; 4] = ["B", "KB", "MB", "GB"];
    let (mut v, mut i) = (b as f64, 0);
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    format!("{v:.1} {}", U[i])
}
