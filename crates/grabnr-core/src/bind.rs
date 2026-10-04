use std::net::IpAddr;
use std::time::Duration;

use serde::Serialize;

use crate::interfaces::Link;

/// How a connection is pinned to a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BindMode {
    /// Let the OS routing table decide.
    None,
    /// `bind()` to the interface's IP address. The route lookup can still pick another link.
    LocalAddr,
    /// `IP_BOUND_IF` / `SO_BINDTODEVICE`: forces egress through the interface.
    BoundIf,
}

impl BindMode {
    pub fn label(self) -> &'static str {
        match self {
            BindMode::None => "unbound",
            BindMode::LocalAddr => "local-addr",
            BindMode::BoundIf => "bound-if",
        }
    }
}

/// One-shot client (no connection reuse): used by probes and the spike.
pub fn client_for(link: Option<&Link>, mode: BindMode) -> Result<reqwest::Client, reqwest::Error> {
    build(link, mode, false, None)
}

/// Download client: keeps connections open from one chunk to the next.
/// HTTP/1.1 only, because HTTP/2 would multiplex every worker onto one TCP connection.
pub fn client_pooled(link: Option<&Link>, mode: BindMode) -> Result<reqwest::Client, reqwest::Error> {
    build(link, mode, true, None)
}

/// Like [`client_pooled`], going through an HTTP(S) or SOCKS5 proxy (`http://host:port`, `socks5://host:port`).
/// The connection to the proxy is the one pinned to the link.
pub fn client_via_proxy(link: Option<&Link>, mode: BindMode, proxy: Option<&str>) -> Result<reqwest::Client, reqwest::Error> {
    build(link, mode, true, proxy)
}

fn build(link: Option<&Link>, mode: BindMode, pooled: bool, proxy: Option<&str>) -> Result<reqwest::Client, reqwest::Error> {
    let mut b = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .user_agent(concat!("grabnr/", env!("CARGO_PKG_VERSION")));
    if let Some(p) = proxy.map(str::trim).filter(|p| !p.is_empty()) {
        b = b.proxy(reqwest::Proxy::all(p)?);
    }
    b = if pooled { b.http1_only().pool_idle_timeout(Duration::from_secs(30)) } else { b.pool_max_idle_per_host(0) };
    if let Some(link) = link {
        match mode {
            BindMode::None => {}
            BindMode::LocalAddr => b = b.local_address(IpAddr::V4(link.ipv4)),
            #[cfg(any(target_os = "macos", target_os = "linux", target_os = "android", target_os = "fuchsia"))]
            BindMode::BoundIf => b = b.interface(&link.name),
            #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "android", target_os = "fuchsia")))]
            BindMode::BoundIf => b = b.local_address(IpAddr::V4(link.ipv4)),
        }
    }
    b.build()
}
