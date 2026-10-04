use clap::{Parser, Subcommand};
use grabnr_core::{interfaces::shared_gateways, list_links, spike};

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
