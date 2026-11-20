//! Client address for rate limits and audit, SPEC 16.
//!
//! `X-Forwarded-For` is honoured only when the TCP peer is a trusted proxy (Traefik), and then
//! only up to the first hop that is not itself trusted. Anything a client writes into the
//! header to the left of that hop is ignored, so a client cannot pick its own rate limit key.

use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

use axum::{extract::ConnectInfo, http::request::Parts};

/// An IP network such as `10.42.0.0/16` or `fd00::/8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cidr {
    addr: IpAddr,
    prefix: u8,
}

impl Cidr {
    pub fn contains(&self, ip: IpAddr) -> bool {
        // An IPv4 peer may arrive as an IPv4-mapped IPv6 address on a dual-stack socket.
        let ip = match ip {
            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),
            v4 @ IpAddr::V4(_) => v4,
        };
        match (self.addr, ip) {
            (IpAddr::V4(net), IpAddr::V4(ip)) => {
                let mask = u32::MAX.checked_shl(32 - u32::from(self.prefix)).unwrap_or(0);
                u32::from(net) & mask == u32::from(ip) & mask
            }
            (IpAddr::V6(net), IpAddr::V6(ip)) => {
                let mask = u128::MAX.checked_shl(128 - u32::from(self.prefix)).unwrap_or(0);
                u128::from(net) & mask == u128::from(ip) & mask
            }
            _ => false,
        }
    }
}

impl FromStr for Cidr {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, ()> {
        let (addr, prefix) = match s.split_once('/') {
            Some((a, p)) => (a, Some(p)),
            None => (s, None),
        };
        let addr: IpAddr = addr.trim().parse().map_err(|_| ())?;
        let max = if addr.is_ipv4() { 32 } else { 128 };
        let prefix = match prefix {
            Some(p) => p.trim().parse::<u8>().map_err(|_| ())?,
            None => max,
        };
        if prefix > max {
            return Err(());
        }
        Ok(Self { addr, prefix })
    }
}

/// Resolves the client address of a request.
pub fn client_ip(parts: &Parts, trusted: &[Cidr]) -> IpAddr {
    let peer = parts
        .extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(IpAddr::from([0, 0, 0, 0]), |c| c.0.ip());
    resolve(peer, forwarded_for(parts).as_deref(), trusted)
}

fn forwarded_for(parts: &Parts) -> Option<String> {
    let values: Vec<&str> = parts
        .headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    (!values.is_empty()).then(|| values.join(","))
}

/// Walks the forwarded chain from the right while each hop is trusted and returns the first
/// untrusted address. Malformed entries stop the walk at the last trusted hop.
fn resolve(peer: IpAddr, xff: Option<&str>, trusted: &[Cidr]) -> IpAddr {
    let is_trusted = |ip: IpAddr| trusted.iter().any(|c| c.contains(ip));
    if !is_trusted(peer) {
        return peer;
    }
    let mut current = peer;
    if let Some(chain) = xff {
        for hop in chain.rsplit(',') {
            let Ok(ip) = hop.trim().parse::<IpAddr>() else {
                break;
            };
            current = ip;
            if !is_trusted(ip) {
                break;
            }
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cidrs(list: &[&str]) -> Vec<Cidr> {
        list.iter()
            .map(|s| s.parse().unwrap_or_else(|()| panic!("{s}")))
            .collect()
    }

    #[test]
    fn parses_and_matches_networks() {
        let c: Cidr = "10.42.0.0/16".parse().expect("v4");
        assert!(c.contains("10.42.7.9".parse().expect("ip")));
        assert!(!c.contains("10.43.0.1".parse().expect("ip")));
        assert!(c.contains("::ffff:10.42.0.1".parse().expect("mapped")));
        let v6: Cidr = "fd00::/8".parse().expect("v6");
        assert!(v6.contains("fd12::1".parse().expect("ip")));
        let all: Cidr = "0.0.0.0/0".parse().expect("all");
        assert!(all.contains("192.0.2.1".parse().expect("ip")));
        for bad in ["10.0.0.0/33", "nonsense", "10.0.0.0/x", "::/129"] {
            assert!(bad.parse::<Cidr>().is_err(), "{bad}");
        }
    }

    #[test]
    fn forwarded_header_is_ignored_from_untrusted_peers() {
        let peer = "203.0.113.5".parse().expect("ip");
        let got = resolve(peer, Some("198.51.100.1"), &cidrs(&["10.0.0.0/8"]));
        assert_eq!(got, peer);
    }

    #[test]
    fn client_cannot_spoof_through_a_trusted_proxy() {
        // The client wrote 1.2.3.4 itself, Traefik appended the real address.
        let peer = "10.0.0.2".parse().expect("ip");
        let got = resolve(peer, Some("1.2.3.4, 198.51.100.7"), &cidrs(&["10.0.0.0/8"]));
        assert_eq!(got, "198.51.100.7".parse::<IpAddr>().expect("ip"));
    }

    #[test]
    fn chained_trusted_proxies_are_skipped() {
        let peer = "10.0.0.2".parse().expect("ip");
        let got = resolve(peer, Some("198.51.100.7, 10.0.0.9"), &cidrs(&["10.0.0.0/8"]));
        assert_eq!(got, "198.51.100.7".parse::<IpAddr>().expect("ip"));
    }

    #[test]
    fn garbage_in_the_header_falls_back_to_the_last_trusted_hop() {
        let peer = "10.0.0.2".parse().expect("ip");
        let got = resolve(peer, Some("not-an-ip"), &cidrs(&["10.0.0.0/8"]));
        assert_eq!(got, peer);
    }
}
