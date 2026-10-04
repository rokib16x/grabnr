use std::net::{IpAddr, Ipv4Addr};

use netdev::interface::types::InterfaceType;
use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    WiFi,
    Ethernet,
    Cellular,
    Tunnel,
    Other,
}

/// One usable network link (an interface that is up and has an IPv4 address).
#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub name: String,
    pub index: u32,
    pub label: String,
    pub kind: LinkKind,
    pub ipv4: Ipv4Addr,
    pub gateway: Option<IpAddr>,
    pub is_default_route: bool,
    pub link_speed_mbps: Option<u64>,
}

fn classify(name: &str, ty: InterfaceType, label: &str) -> LinkKind {
    let l = label.to_ascii_lowercase();
    if name.starts_with("utun") || name.starts_with("ipsec") || name.starts_with("ppp") {
        return LinkKind::Tunnel;
    }
    if l.contains("wi-fi") || l.contains("wifi") || l.contains("wireless") || ty == InterfaceType::Wireless80211 {
        return LinkKind::WiFi;
    }
    if l.contains("iphone") || l.contains("android") || l.contains("cellular") || l.contains("modem") {
        return LinkKind::Cellular;
    }
    if ty == InterfaceType::Ethernet || l.contains("ethernet") || l.contains("lan") {
        return LinkKind::Ethernet;
    }
    LinkKind::Other
}

/// Interfaces that are up, non-loopback, and carry a routable IPv4 address.
pub fn list_links() -> Vec<Link> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|i| i.is_up() && !i.is_loopback())
        .filter_map(|i| {
            let ip = i.ipv4_addrs().into_iter().find(|a| !a.is_link_local() && !a.is_loopback())?;
            let label = i.friendly_name.clone().or(i.description.clone()).unwrap_or_else(|| i.name.clone());
            let kind = classify(&i.name, i.if_type, &label);
            let gateway = i
                .gateway
                .as_ref()
                .and_then(|g| g.ipv4.first().map(|a| IpAddr::V4(*a)));
            Some(Link {
                kind,
                label,
                ipv4: ip,
                gateway,
                is_default_route: i.default,
                link_speed_mbps: i.receive_speed.map(|b| b / 1_000_000),
                index: i.index,
                name: i.name,
            })
        })
        .collect()
}

/// Links that share a gateway cannot add bandwidth to one another.
pub fn shared_gateways(links: &[Link]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (i, a) in links.iter().enumerate() {
        for b in &links[i + 1..] {
            if a.gateway.is_some() && a.gateway == b.gateway {
                out.push((a.name.clone(), b.name.clone()));
            }
        }
    }
    out
}
