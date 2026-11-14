//! Who is really asking, when the site sits behind the Caddy proxy container.
//!
//! The proxy's `X-Forwarded-For` is believed only when the TCP peer is the proxy
//! itself (its address is looked up by name, so it follows the container when it
//! is recreated). Every other peer, including a stranger on the direct port who
//! sends the header by hand, is taken at its own address. From a trusted peer the
//! right-most entry is used: that is the one the proxy appended, and anything
//! to its left was written by the client.

use axum::http::{header, HeaderMap};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::RwLock;

/// The proxy containers this site trusts: names or addresses from `WEB_TRUSTED_PROXY`.
pub struct Proxies {
    names: Vec<String>,
    ips: RwLock<HashSet<IpAddr>>,
}

impl Proxies {
    pub fn new(spec: &str) -> Proxies {
        let names: Vec<String> = spec.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        let fixed = names.iter().filter_map(|n| n.parse::<IpAddr>().ok()).map(|ip| ip.to_canonical()).collect();
        Proxies { names, ips: RwLock::new(fixed) }
    }

    /// Looks the names up again. A name that doesn't resolve (the proxy isn't running, or the
    /// `public` profile is off) trusts nothing, so the header is ignored.
    pub async fn refresh(&self) {
        let mut found: HashSet<IpAddr> = HashSet::new();
        for n in &self.names {
            if let Ok(ip) = n.parse::<IpAddr>() {
                found.insert(ip.to_canonical());
            } else {
                match tokio::net::lookup_host((n.as_str(), 0)).await {
                    // Only a private address can be the container beside us: a name that fell
                    // through to the host's resolver and came back public is not trusted.
                    Ok(addrs) => found.extend(addrs.map(|a| a.ip().to_canonical()).filter(|i| is_private_client(&i.to_string()))),
                    // A lookup that fails (resolver hiccup) leaves what we knew; a name that
                    // doesn't exist at all yields no addresses above and so clears it.
                    Err(e) if !gone(&e) => return,
                    Err(_) => {}
                }
            }
        }
        let mut cur = self.ips.write().unwrap();
        if *cur != found {
            tracing::info!("trusted proxy addresses: {found:?}");
            *cur = found;
        }
    }

    pub fn is_proxy(&self, ip: IpAddr) -> bool {
        self.ips.read().unwrap().contains(&ip.to_canonical())
    }

    /// For tests: replace the trusted addresses.
    pub fn set(&self, ips: impl IntoIterator<Item = IpAddr>) {
        *self.ips.write().unwrap() = ips.into_iter().map(|i| i.to_canonical()).collect();
    }
}

/// Does this lookup error mean the name doesn't exist (the proxy isn't running), as opposed to the
/// resolver being briefly unreachable?
fn gone(e: &std::io::Error) -> bool {
    let m = e.to_string();
    m.contains("not known") || m.contains("does not resolve") || m.contains("No address associated") || m.contains("NXDOMAIN") || m.contains("No such host")
}

/// Where a request really came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The client's address as text, `unknown` when a proxy sent nothing usable.
    pub ip: String,
    /// The TCP peer was the proxy, so TLS ended there.
    pub via_proxy: bool,
}

pub fn resolve(peer: Option<IpAddr>, headers: &HeaderMap, proxies: &Proxies) -> Source {
    let Some(peer) = peer else { return Source { ip: "unknown".into(), via_proxy: false } };
    if !proxies.is_proxy(peer) {
        return Source { ip: peer.to_canonical().to_string(), via_proxy: false };
    }
    let ip = rightmost_forwarded(headers).map(|i| i.to_string()).unwrap_or_else(|| "unknown".into());
    Source { ip, via_proxy: true }
}

/// The last address of the last `X-Forwarded-For` line; `None` when it isn't an address.
fn rightmost_forwarded(headers: &HeaderMap) -> Option<IpAddr> {
    let line = headers.get_all("x-forwarded-for").iter().next_back()?.to_str().ok()?;
    let last = line.rsplit(',').next()?.trim();
    // Caddy writes a bare address; tolerate `[::1]` and `1.2.3.4:5` from other proxies.
    let bare = last.trim_start_matches('[').split(']').next().unwrap_or(last);
    let bare = if bare.matches(':').count() == 1 { bare.split(':').next().unwrap_or(bare) } else { bare };
    bare.parse::<IpAddr>().ok().map(|i| i.to_canonical())
}

/// An address on this computer's own network or a private one: loopback, RFC 1918,
/// Tailscale's 100.64.0.0/10, and the IPv6 equivalents.
pub fn is_private_client(ip: &str) -> bool {
    match ip.parse::<IpAddr>().map(|i| i.to_canonical()) {
        Ok(IpAddr::V4(v)) => v.is_loopback() || v.is_private() || (v.octets()[0] == 100 && (64..128).contains(&v.octets()[1])),
        Ok(IpAddr::V6(v)) => v.is_loopback() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

/// The host of `WEB_PUBLIC_URL`, without scheme, port or path, lower case.
pub fn public_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map(|(_, r)| r)?;
    let auth = rest.split('/').next()?;
    host_only(auth)
}

/// A `Host` header's name without its port, lower case.
pub fn host_only(h: &str) -> Option<String> {
    let h = h.trim();
    let name = if let Some(r) = h.strip_prefix('[') { r.split(']').next()? } else { h.rsplit_once(':').map_or(h, |(n, _)| n) };
    (!name.is_empty()).then(|| name.trim_end_matches('.').to_ascii_lowercase())
}

pub fn host_of(headers: &HeaderMap) -> Option<String> {
    host_only(headers.get(header::HOST)?.to_str().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hm(lines: &[&str]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for l in lines {
            h.append("x-forwarded-for", l.parse().unwrap());
        }
        h
    }
    fn proxies() -> Proxies {
        let p = Proxies::new("172.20.0.5");
        p.set(["172.20.0.5".parse().unwrap()]);
        p
    }

    #[test]
    fn a_header_from_any_other_peer_is_ignored() {
        let o = resolve(Some("203.0.113.9".parse().unwrap()), &hm(&["1.2.3.4"]), &proxies());
        assert_eq!(o, Source { ip: "203.0.113.9".into(), via_proxy: false });
    }

    #[test]
    fn from_the_proxy_the_rightmost_entry_wins() {
        let o = resolve(Some("172.20.0.5".parse().unwrap()), &hm(&["6.6.6.6, 7.7.7.7, 198.51.100.4"]), &proxies());
        assert_eq!(o, Source { ip: "198.51.100.4".into(), via_proxy: true });
        // Several header lines: the last line is the proxy's.
        let o = resolve(Some("172.20.0.5".parse().unwrap()), &hm(&["6.6.6.6", "198.51.100.4"]), &proxies());
        assert_eq!(o.ip, "198.51.100.4");
    }

    #[test]
    fn a_proxy_that_sends_nothing_usable_names_no_one() {
        for bad in [vec![], vec!["not-an-address"], vec![" "], vec!["1.2.3.4,"]] {
            let o = resolve(Some("172.20.0.5".parse().unwrap()), &hm(&bad), &proxies());
            assert_eq!(o, Source { ip: "unknown".into(), via_proxy: true }, "{bad:?}");
        }
    }

    #[test]
    fn no_trusted_proxy_means_the_header_never_counts() {
        let p = Proxies::new("");
        let o = resolve(Some("172.20.0.5".parse().unwrap()), &hm(&["9.9.9.9"]), &p);
        assert_eq!(o, Source { ip: "172.20.0.5".into(), via_proxy: false });
    }

    #[test]
    fn address_forms() {
        let o = resolve(Some("172.20.0.5".parse().unwrap()), &hm(&["[2001:db8::1]"]), &proxies());
        assert_eq!(o.ip, "2001:db8::1");
        let o = resolve(Some("172.20.0.5".parse().unwrap()), &hm(&["::ffff:198.51.100.4"]), &proxies());
        assert_eq!(o.ip, "198.51.100.4");
        let o = resolve(Some("::ffff:172.20.0.5".parse().unwrap()), &hm(&["198.51.100.4"]), &proxies());
        assert!(o.via_proxy);
    }

    #[test]
    fn a_missing_name_is_told_from_a_resolver_hiccup() {
        let e = |m: &str| std::io::Error::other(m.to_string());
        for gone_msg in ["failed to lookup address information: Name or service not known", "failed to lookup address information: Name does not resolve", "No address associated with hostname"] {
            assert!(gone(&e(gone_msg)), "{gone_msg}");
        }
        assert!(!gone(&e("failed to lookup address information: Temporary failure in name resolution")));
    }

    #[test]
    fn private_addresses() {
        for ip in ["127.0.0.1", "::1", "10.1.2.3", "172.16.0.9", "192.168.1.5", "100.64.0.1", "100.127.255.255", "fd00::1", "fe80::1"] {
            assert!(is_private_client(ip), "{ip}");
        }
        for ip in ["8.8.8.8", "100.63.0.1", "100.128.0.1", "172.32.0.1", "2001:db8::1", "unknown", ""] {
            assert!(!is_private_client(ip), "{ip}");
        }
    }

    #[test]
    fn hosts() {
        assert_eq!(public_host("https://Ffxi.Example.com/").as_deref(), Some("ffxi.example.com"));
        assert_eq!(public_host("https://ffxi.example.com:8443").as_deref(), Some("ffxi.example.com"));
        assert_eq!(host_only("FFXI.example.com:443").as_deref(), Some("ffxi.example.com"));
        assert_eq!(host_only("[::1]:8080").as_deref(), Some("::1"));
        assert_eq!(public_host("nonsense"), None);
    }
}
