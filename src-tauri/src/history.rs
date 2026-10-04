//! Per-download statistics and the completed-downloads history (with CSV export).

use std::collections::{BTreeMap, HashMap};

use grabnr_core::download::RouteStat;
use serde::{Deserialize, Serialize};

/// What a download has used so far, summed over every run (pause/resume, app restarts).
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Stats {
    /// Bytes each link put on the wire, by interface name.
    pub link_bytes: BTreeMap<String, u64>,
    /// Seconds the download was actually running.
    pub active_secs: f64,
    /// Fastest speed any single link reached.
    pub peak_bps: f64,
}

impl Stats {
    /// Fold in a progress snapshot. `seen` holds each link's byte counter from the previous snapshot of this run,
    /// because the engine's counters restart from zero every run.
    pub fn add_progress(&mut self, seen: &mut HashMap<String, u64>, routes: &[RouteStat]) {
        for r in routes {
            let before = seen.insert(r.name.clone(), r.bytes).unwrap_or(0);
            *self.link_bytes.entry(r.name.clone()).or_default() += r.bytes.saturating_sub(before);
            if r.bytes_per_sec > self.peak_bps {
                self.peak_bps = r.bytes_per_sec;
            }
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryRec {
    pub id: String,
    pub name: String,
    pub url: String,
    pub bytes: u64,
    /// Unix seconds.
    pub finished: u64,
    pub active_secs: f64,
    pub link_bytes: BTreeMap<String, u64>,
    /// Estimated seconds saved compared with using only the fastest link.
    pub saved_secs: f64,
}

/// Estimate: how long the fastest single link alone would have taken, minus how long it took.
/// Zero unless more than one link carried data, because one link saves nothing.
pub fn time_saved(bytes: u64, stats: &Stats) -> f64 {
    let used = stats.link_bytes.values().filter(|b| **b > bytes / 50).count();
    if used < 2 || stats.peak_bps <= 0.0 || stats.active_secs <= 0.0 {
        return 0.0;
    }
    (bytes as f64 / stats.peak_bps - stats.active_secs).max(0.0)
}

pub fn record(id: &str, name: &str, url: &str, bytes: u64, finished: u64, stats: &Stats) -> HistoryRec {
    HistoryRec {
        id: id.into(),
        name: name.into(),
        url: url.into(),
        bytes,
        finished,
        active_secs: stats.active_secs,
        link_bytes: stats.link_bytes.clone(),
        saved_secs: time_saved(bytes, stats),
    }
}

fn csv_field(s: &str) -> String {
    // Quote when needed, and defuse spreadsheet formulas in names or URLs.
    let s = if s.starts_with(['=', '+', '-', '@']) { format!("'{s}") } else { s.to_string() };
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

pub fn to_csv(recs: &[HistoryRec]) -> String {
    let mut out = String::from("finished_unix,name,url,bytes,active_seconds,saved_seconds_estimate,bytes_per_link\n");
    for r in recs {
        let links = r.link_bytes.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(";");
        out.push_str(&format!(
            "{},{},{},{},{:.1},{:.1},{}\n",
            r.finished,
            csv_field(&r.name),
            csv_field(&r.url),
            r.bytes,
            r.active_secs,
            r.saved_secs,
            csv_field(&links)
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn route(name: &str, bytes: u64, bps: f64) -> RouteStat {
        RouteStat { name: name.into(), bytes, bytes_per_sec: bps, connections: 1, down: false }
    }

    #[test]
    fn progress_counts_each_run_from_zero() {
        let mut s = Stats::default();
        let mut seen = HashMap::new();
        s.add_progress(&mut seen, &[route("en0", 100, 10.0), route("en5", 50, 30.0)]);
        s.add_progress(&mut seen, &[route("en0", 250, 12.0), route("en5", 50, 5.0)]);
        // second run: engine counters start over
        let mut seen = HashMap::new();
        s.add_progress(&mut seen, &[route("en0", 40, 8.0)]);
        assert_eq!(s.link_bytes["en0"], 290);
        assert_eq!(s.link_bytes["en5"], 50);
        assert_eq!(s.peak_bps, 30.0);
    }

    #[test]
    fn time_saved_needs_two_links_and_is_never_negative() {
        let mut s = Stats { active_secs: 10.0, peak_bps: 50.0, ..Default::default() };
        s.link_bytes.insert("a".into(), 600);
        assert_eq!(time_saved(1000, &s), 0.0, "a single link saves nothing");
        s.link_bytes.insert("b".into(), 400);
        assert_eq!(time_saved(1000, &s), 10.0); // 1000 / 50 = 20 s alone vs 10 s
        s.active_secs = 30.0;
        assert_eq!(time_saved(1000, &s), 0.0);
    }

    #[test]
    fn csv_quotes_and_defuses_formulas() {
        let mut s = Stats { active_secs: 2.0, peak_bps: 1.0, ..Default::default() };
        s.link_bytes.insert("en0".into(), 5);
        let r = record("1", "=cmd,\"x\".zip", "https://a/b", 5, 1700000000, &s);
        let csv = to_csv(&[r]);
        let row = csv.lines().nth(1).unwrap();
        assert!(row.starts_with("1700000000,\"'=cmd,\"\"x\"\".zip\",https://a/b,5,2.0,0.0,en0=5"), "{row}");
    }
}
