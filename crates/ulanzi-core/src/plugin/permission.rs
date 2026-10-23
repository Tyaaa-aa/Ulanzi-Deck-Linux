use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use super::Permissions;

/// May a plugin with these permissions connect to `host`? `resolved` is what the name resolved to
/// (empty when not resolved yet; a name only counts as local-network when it resolved to local
/// addresses alone).
pub fn host_allowed(p: &Permissions, host: &str, resolved: &[IpAddr]) -> bool {
    let host = norm(host);
    let listed = p.network.iter().any(|entry| {
        let entry = norm(entry);
        match entry.strip_prefix("*.") {
            Some(suffix) => host.len() > suffix.len() + 1 && host.ends_with(&format!(".{suffix}")),
            None => entry == host,
        }
    });
    if listed {
        return true;
    }
    if !p.lan {
        return false;
    }
    if host == "localhost" || host.ends_with(".local") {
        return true;
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_lan(ip);
    }
    !resolved.is_empty() && resolved.iter().all(|ip| is_lan(*ip))
}

/// Lowercase and drop one trailing dot (`localhost.` is `localhost`).
fn norm(s: &str) -> String {
    let s = s.to_ascii_lowercase();
    s.strip_suffix('.').map_or(s.clone(), str::to_string)
}

/// Loopback, private (10/8, 172.16/12, 192.168/16), link-local (169.254/16), IPv6 unique-local
/// (fc00::/7) and link-local (fe80::/10).
pub fn is_lan(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(a) => is_lan_v4(a),
        IpAddr::V6(a) => is_lan_v6(a),
    }
}

fn is_lan_v4(a: Ipv4Addr) -> bool {
    a.is_loopback() || a.is_private() || a.is_link_local()
}

fn is_lan_v6(a: Ipv6Addr) -> bool {
    if let Some(v4) = a.to_ipv4_mapped() {
        return is_lan_v4(v4);
    }
    let first = a.segments()[0];
    a.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80
}
