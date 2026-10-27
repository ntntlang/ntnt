//! Trusted client IP resolution for the HTTP servers (`req.client_ip`).
//!
//! `req.ip` keeps its historical behaviour (first `X-Forwarded-For` hop, which
//! any client can spoof). `req.client_ip` only believes a forwarding header
//! when the TCP peer is inside `NTNT_TRUSTED_PROXIES`; otherwise it is the peer.
//!
//! Configuration is read once at `listen()` (`init_from_env`). Invalid CIDRs
//! fail startup. With nothing configured the resolver is a single branch that
//! returns the peer, so the default costs nothing measurable.

use crate::error::{IntentError, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

/// One trusted network, stored as a 128-bit value so IPv4 and IPv6 share code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Cidr {
    v4: bool,
    network: u128,
    mask: u128,
}

impl Cidr {
    fn parse(text: &str) -> std::result::Result<Cidr, String> {
        let text = text.trim();
        let (addr, prefix) = match text.split_once('/') {
            Some((a, p)) => (a.trim(), Some(p.trim())),
            None => (text, None),
        };
        let ip: IpAddr = addr
            .parse()
            .map_err(|_| format!("invalid address in NTNT_TRUSTED_PROXIES: '{text}'"))?;
        let ip = canonical(ip);
        let bits: u8 = if ip.is_ipv4() { 32 } else { 128 };
        let prefix: u8 = match prefix {
            None => bits,
            Some(p) => p
                .parse::<u8>()
                .ok()
                .filter(|p| *p <= bits)
                .ok_or_else(|| format!("invalid prefix in NTNT_TRUSTED_PROXIES: '{text}'"))?,
        };
        let mask = prefix_mask(bits, prefix);
        let value = to_u128(ip);
        if value & !mask != 0 {
            return Err(format!(
                "NTNT_TRUSTED_PROXIES entry '{text}' has host bits set; use the network address"
            ));
        }
        Ok(Cidr {
            v4: ip.is_ipv4(),
            network: value,
            mask,
        })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        ip.is_ipv4() == self.v4 && (to_u128(ip) & self.mask) == self.network
    }
}

fn prefix_mask(bits: u8, prefix: u8) -> u128 {
    if prefix == 0 {
        return 0;
    }
    let full: u128 = if bits == 32 {
        u32::MAX as u128
    } else {
        u128::MAX
    };
    (full >> (bits - prefix)) << (bits - prefix)
}

fn to_u128(ip: IpAddr) -> u128 {
    match ip {
        IpAddr::V4(v4) => u32::from(v4) as u128,
        IpAddr::V6(v6) => u128::from(v6),
    }
}

/// IPv4-mapped IPv6 (`::ffff:a.b.c.d`) is treated as IPv4 so dual-stack
/// listeners match IPv4 CIDRs and report the familiar dotted form.
fn canonical(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(v6),
        },
        v4 => v4,
    }
}

/// Parse one forwarded value: a bare IP, `[v6]`, or `ip:port`/`[v6]:port`.
fn parse_hop(text: &str) -> Option<IpAddr> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if let Ok(ip) = text.parse::<IpAddr>() {
        return Some(canonical(ip));
    }
    if let Ok(sock) = text.parse::<SocketAddr>() {
        return Some(canonical(sock.ip()));
    }
    text.strip_prefix('[')
        .and_then(|t| t.strip_suffix(']'))
        .and_then(|t| t.parse::<Ipv6Addr>().ok())
        .map(|v6| canonical(IpAddr::V6(v6)))
}

/// Resolved process configuration.
#[derive(Debug, Clone, Default)]
pub struct ClientIpConfig {
    trusted: Vec<Cidr>,
    /// Lower-case header name read from trusted peers.
    header: String,
    /// `X-Forwarded-For` semantics (multi-hop, walk right to left).
    multi_hop: bool,
}

impl ClientIpConfig {
    /// Build from explicit values (used by `from_env` and tests).
    pub fn new(trusted_proxies: &str, header: Option<&str>) -> std::result::Result<Self, String> {
        let mut trusted = Vec::new();
        for entry in trusted_proxies.split(',') {
            if entry.trim().is_empty() {
                continue;
            }
            trusted.push(Cidr::parse(entry)?);
        }
        let header = header
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .unwrap_or_else(|| "x-forwarded-for".to_string());
        if !header
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(format!("invalid NTNT_CLIENT_IP_HEADER '{header}'"));
        }
        let multi_hop = header == "x-forwarded-for";
        Ok(ClientIpConfig {
            trusted,
            header,
            multi_hop,
        })
    }

    pub fn from_env() -> std::result::Result<Self, String> {
        let proxies = std::env::var("NTNT_TRUSTED_PROXIES").unwrap_or_default();
        let header = std::env::var("NTNT_CLIENT_IP_HEADER").ok();
        Self::new(&proxies, header.as_deref())
    }

    fn is_trusted(&self, ip: IpAddr) -> bool {
        self.trusted.iter().any(|c| c.contains(ip))
    }

    /// Resolve the client IP for one request. `header` looks up a header by
    /// lower-case name and returns its (possibly comma-joined) value.
    pub fn resolve<'a>(&self, peer: IpAddr, header: impl Fn(&str) -> Option<&'a str>) -> IpAddr {
        let peer = canonical(peer);
        // Fast path: nothing trusted, or the peer is not a trusted proxy.
        if self.trusted.is_empty() || !self.is_trusted(peer) {
            return peer;
        }
        let Some(value) = header(&self.header) else {
            return peer;
        };
        if !self.multi_hop {
            return match parse_hop(value) {
                Some(ip) => ip,
                None => {
                    warn_once();
                    peer
                }
            };
        }
        // X-Forwarded-For: right to left, skipping trusted hops; the first
        // untrusted address is the client. A malformed hop stops the walk.
        let mut leftmost_trusted = None;
        for hop in value.rsplit(',') {
            match parse_hop(hop) {
                Some(ip) if self.is_trusted(ip) => leftmost_trusted = Some(ip),
                Some(ip) => return ip,
                None => {
                    warn_once();
                    return peer;
                }
            }
        }
        // Every hop was a trusted proxy: the leftmost one is the best answer.
        leftmost_trusted.unwrap_or(peer)
    }
}

static WARNED: AtomicBool = AtomicBool::new(false);

fn warn_once() {
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[WARN] client_ip: a trusted proxy sent an unparseable client IP header; using the peer address (logged once)"
        );
    }
}

static CONFIG: OnceLock<ClientIpConfig> = OnceLock::new();

/// Read `NTNT_TRUSTED_PROXIES` / `NTNT_CLIENT_IP_HEADER` once. Called by
/// `listen()`; an invalid value fails startup with a clear error.
pub fn init_from_env() -> Result<()> {
    if CONFIG.get().is_some() {
        return Ok(());
    }
    let config = ClientIpConfig::from_env().map_err(IntentError::runtime_error)?;
    let _ = CONFIG.set(config);
    Ok(())
}

/// The process configuration (empty, i.e. "trust nobody", before `init_from_env`).
pub fn config() -> &'static ClientIpConfig {
    CONFIG.get_or_init(ClientIpConfig::default)
}

/// Convenience used by both servers.
pub fn resolve_string<'a>(peer: IpAddr, header: impl Fn(&str) -> Option<&'a str>) -> String {
    config().resolve(peer, header).to_string()
}

/// Peer fallback when the transport did not report an address.
pub fn unknown_peer() -> IpAddr {
    IpAddr::V4(Ipv4Addr::UNSPECIFIED)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn resolve(cfg: &ClientIpConfig, peer: &str, headers: &[(&str, &str)]) -> String {
        let map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        cfg.resolve(ip(peer), |name| map.get(name).map(|s| s.as_str()))
            .to_string()
    }

    #[test]
    fn untrusted_peer_ignores_forwarded_headers() {
        let cfg = ClientIpConfig::new("10.0.0.0/8", None).unwrap();
        assert_eq!(
            resolve(&cfg, "203.0.113.9", &[("x-forwarded-for", "1.2.3.4")]),
            "203.0.113.9"
        );
        let none = ClientIpConfig::default();
        assert_eq!(
            resolve(&none, "10.1.2.3", &[("x-forwarded-for", "1.2.3.4")]),
            "10.1.2.3"
        );
    }

    #[test]
    fn xff_walks_right_to_left_skipping_trusted_hops() {
        let cfg = ClientIpConfig::new("10.0.0.0/8, 172.16.0.0/12", None).unwrap();
        // Spoofed leftmost value is ignored; the first untrusted from the right wins.
        assert_eq!(
            resolve(
                &cfg,
                "10.0.0.5",
                &[("x-forwarded-for", "6.6.6.6, 198.51.100.7, 172.16.4.4")]
            ),
            "198.51.100.7"
        );
        assert_eq!(
            resolve(
                &cfg,
                "10.0.0.5",
                &[("x-forwarded-for", "10.9.9.9, 172.16.0.1")]
            ),
            "10.9.9.9"
        );
        assert_eq!(resolve(&cfg, "10.0.0.5", &[]), "10.0.0.5");
    }

    #[test]
    fn malformed_hop_falls_back_to_peer() {
        let cfg = ClientIpConfig::new("10.0.0.0/8", None).unwrap();
        assert_eq!(
            resolve(
                &cfg,
                "10.0.0.5",
                &[("x-forwarded-for", "1.2.3.4, not-an-ip")]
            ),
            "10.0.0.5"
        );
        assert_eq!(
            resolve(&cfg, "10.0.0.5", &[("x-forwarded-for", "")]),
            "10.0.0.5"
        );
    }

    #[test]
    fn single_value_header_and_ports_and_ipv6() {
        let cfg = ClientIpConfig::new("10.0.0.0/8,fd00::/8", Some("CF-Connecting-IP")).unwrap();
        assert_eq!(
            resolve(&cfg, "10.0.0.5", &[("cf-connecting-ip", "2001:db8::1")]),
            "2001:db8::1"
        );
        // The configured header is the only one read.
        assert_eq!(
            resolve(&cfg, "10.0.0.5", &[("x-forwarded-for", "1.2.3.4")]),
            "10.0.0.5"
        );
        assert_eq!(
            resolve(
                &cfg,
                "fd00::2",
                &[("cf-connecting-ip", "198.51.100.7:4433")]
            ),
            "198.51.100.7"
        );
        assert_eq!(
            resolve(&cfg, "10.0.0.5", &[("cf-connecting-ip", "[2001:db8::2]")]),
            "2001:db8::2"
        );
    }

    #[test]
    fn ipv4_mapped_peer_matches_ipv4_cidr() {
        let cfg = ClientIpConfig::new("10.0.0.0/8", None).unwrap();
        assert_eq!(
            resolve(
                &cfg,
                "::ffff:10.0.0.5",
                &[("x-forwarded-for", "198.51.100.7")]
            ),
            "198.51.100.7"
        );
        assert_eq!(
            resolve(&ClientIpConfig::default(), "::ffff:192.0.2.1", &[]),
            "192.0.2.1"
        );
    }

    #[test]
    fn invalid_configuration_is_rejected() {
        for bad in ["10.0.0.0/33", "not-a-cidr", "10.0.0.1/8", "fd00::/129"] {
            assert!(ClientIpConfig::new(bad, None).is_err(), "{bad}");
        }
        assert!(ClientIpConfig::new("", Some("bad header!")).is_err());
        assert!(ClientIpConfig::new(" , ", None).unwrap().trusted.is_empty());
        assert!(ClientIpConfig::new("0.0.0.0/0", None).is_ok());
        assert!(ClientIpConfig::new("192.0.2.7", None)
            .unwrap()
            .is_trusted(ip("192.0.2.7")));
    }
}
