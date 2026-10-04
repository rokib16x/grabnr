//! A text report for bug reports: versions, links and settings, with nothing private in it.

use crate::manager::Settings;

pub struct Counts {
    pub queued: usize,
    pub downloading: usize,
    pub paused: usize,
    pub done: usize,
    pub error: usize,
}

/// Just the host of a URL: file names, paths and query strings can be private.
pub fn host_only(url: &str) -> String {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| "(unreadable link)".into())
}

fn yes(b: bool) -> &'static str {
    if b {
        "on"
    } else {
        "off"
    }
}

pub struct Input<'a> {
    pub version: &'a str,
    pub os: &'a str,
    pub links: &'a [(String, String, String)],
    pub settings: &'a Settings,
    pub counts: Counts,
    /// (link, message) of recent failures.
    pub errors: &'a [(String, String)],
    pub webhook_error: Option<&'a str>,
}

pub fn build(i: &Input) -> String {
    let s = i.settings;
    let mut out = String::new();
    out.push_str(&format!("grabnr {}\n{}\n\n", i.version, i.os));
    out.push_str("Connections\n");
    if i.links.is_empty() {
        out.push_str("  none found\n");
    }
    for (name, label, kind) in i.links {
        let enabled = s.enabled_links.as_ref().is_none_or(|l| l.contains(name));
        out.push_str(&format!("  {name}  {label}  {kind}  {}\n", if enabled { "enabled" } else { "disabled" }));
    }
    out.push_str(&format!(
        "\nDownloads: {} downloading, {} queued, {} paused, {} finished, {} failed\n",
        i.counts.downloading, i.counts.queued, i.counts.paused, i.counts.done, i.counts.error
    ));
    out.push_str(&format!(
        "\nSettings\n  max connections per link: {}\n  simultaneous downloads: {}\n  speed limit: {}\n  automatic retries: {}\n  schedule rules: {}\n  link rules: {}\n  skip cellular: {}\n  proxy: {}\n  webhook: {}\n  quarantine flag: {}\n  run command when finished: {}\n",
        s.conns_per_route,
        s.max_active,
        if s.speed_limit_kbps == 0 { "none".to_string() } else { format!("{} KB/s", s.speed_limit_kbps) },
        s.auto_retry,
        s.schedule.len(),
        s.link_rules.len(),
        yes(s.skip_cellular),
        if s.proxy.is_empty() { "none" } else { "set" },
        if s.webhook_url.is_empty() { "none" } else { "set" },
        yes(s.quarantine),
        if s.after_command.is_empty() { "none" } else { "set" },
    ));
    if !i.errors.is_empty() {
        out.push_str("\nRecent failures (host only)\n");
        for (host, msg) in i.errors {
            out.push_str(&format!("  {host}: {msg}\n"));
        }
    }
    if let Some(e) = i.webhook_error {
        out.push_str(&format!("\nLast webhook error: {e}\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_only_the_host() {
        assert_eq!(host_only("https://files.example.com/private/name.zip?token=abc"), "files.example.com");
        assert_eq!(host_only("nonsense"), "(unreadable link)");
    }
}
