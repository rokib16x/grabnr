//! Phase-0 spike: prove that we can pin traffic to a specific interface and
//! that two interfaces download at the same time with summed throughput.

use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::Serialize;

use crate::bind::{client_for, BindMode};
use crate::interfaces::Link;

pub const IP_ECHO_URL: &str = "https://api.ipify.org";
pub const DEFAULT_TEST_URL: &str = "https://proof.ovh.net/files/100Mb.dat";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Egress IP differs from the default route: the bind demonstrably worked.
    Bound,
    /// Same egress IP as the default route: shared ISP, or the bind was ignored.
    SameEgress,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModeResult {
    pub mode: BindMode,
    pub public_ip: Option<String>,
    pub verdict: Verdict,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Throughput {
    pub link: String,
    pub bytes: u64,
    pub secs: f64,
    pub mbps: f64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkReport {
    pub link: String,
    pub modes: Vec<ModeResult>,
    pub solo: Throughput,
}

#[derive(Debug, Clone, Serialize)]
pub struct SpikeReport {
    pub baseline_ip: Option<String>,
    pub links: Vec<LinkReport>,
    pub combined: Vec<Throughput>,
    pub combined_mbps: f64,
    pub best_solo_mbps: f64,
}

async fn public_ip(link: Option<&Link>, mode: BindMode) -> Result<String, String> {
    let client = client_for(link, mode).map_err(|e| e.to_string())?;
    let text = client
        .get(IP_ECHO_URL)
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .text()
        .await
        .map_err(|e| e.to_string())?;
    Ok(text.trim().to_string())
}

/// Stream `url` through `link` for at most `secs` seconds and report throughput.
pub async fn measure(link: &Link, mode: BindMode, url: &str, secs: u64) -> Throughput {
    let name = link.name.clone();
    let fail = |e: String| Throughput { link: name.clone(), bytes: 0, secs: 0.0, mbps: 0.0, error: Some(e) };
    let client = match client_for(Some(link), mode) {
        Ok(c) => c,
        Err(e) => return fail(e.to_string()),
    };
    let start = Instant::now();
    let resp = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => return fail(e.to_string()),
    };
    if !resp.status().is_success() {
        return fail(format!("HTTP {}", resp.status()));
    }
    let mut stream = resp.bytes_stream();
    let mut bytes = 0u64;
    let deadline = Duration::from_secs(secs);
    while start.elapsed() < deadline {
        match tokio::time::timeout(deadline.saturating_sub(start.elapsed()), stream.next()).await {
            Ok(Some(Ok(chunk))) => bytes += chunk.len() as u64,
            Ok(Some(Err(e))) => return fail(e.to_string()),
            Ok(None) | Err(_) => break,
        }
    }
    let elapsed = start.elapsed().as_secs_f64().max(0.001);
    Throughput { link: name, bytes, secs: elapsed, mbps: bytes as f64 * 8.0 / elapsed / 1e6, error: None }
}

pub async fn run(links: &[Link], url: &str, secs: u64) -> SpikeReport {
    let baseline_ip = public_ip(None, BindMode::None).await.ok();

    let mut reports = Vec::new();
    for link in links {
        let mut modes = Vec::new();
        for mode in [BindMode::LocalAddr, BindMode::BoundIf] {
            let r = public_ip(Some(link), mode).await;
            modes.push(match r {
                Ok(ip) => {
                    let verdict = if Some(&ip) == baseline_ip.as_ref() { Verdict::SameEgress } else { Verdict::Bound };
                    ModeResult { mode, public_ip: Some(ip), verdict, error: None }
                }
                Err(e) => ModeResult { mode, public_ip: None, verdict: Verdict::Failed, error: Some(e) },
            });
        }
        // Throughput uses the strongest mode available on this platform.
        let solo = measure(link, BindMode::BoundIf, url, secs).await;
        reports.push(LinkReport { link: link.name.clone(), modes, solo });
    }

    let usable: Vec<&Link> = links
        .iter()
        .filter(|l| reports.iter().any(|r| r.link == l.name && r.solo.error.is_none()))
        .collect();
    let combined = futures_util::future::join_all(
        usable.iter().map(|l| measure(l, BindMode::BoundIf, url, secs)),
    )
    .await;
    let combined_mbps = combined.iter().map(|t| t.mbps).sum();
    let best_solo_mbps = reports.iter().map(|r| r.solo.mbps).fold(0.0, f64::max);

    SpikeReport { baseline_ip, links: reports, combined, combined_mbps, best_solo_mbps }
}
