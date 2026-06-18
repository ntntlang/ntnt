//! Target classification and outbound safety policy for `std/net`.
//!
//! `std/net` can be used from public web apps, so hostname/IP-taking probes
//! must not become SSRF/scanning shortcuts by default. Public targets are
//! allowed; private/internal targets require both process-level deployment
//! intent (`NTNT_NET_ALLOW_PRIVATE=1`) and per-call `allow_private: true`;
//! cloud metadata and other special-purpose targets remain denied.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

#[derive(Debug)]
pub(super) struct IpClassification {
    pub(super) is_private: bool,
    pub(super) is_loopback: bool,
    pub(super) is_link_local: bool,
    pub(super) is_multicast: bool,
    pub(super) is_unspecified: bool,
    pub(super) is_documentation: bool,
    pub(super) is_broadcast: bool,
    pub(super) is_metadata_endpoint: bool,
    pub(super) is_unique_local: bool,
}

pub(super) fn classify_ip(ip: IpAddr) -> IpClassification {
    match ip {
        IpAddr::V4(ip) => IpClassification {
            is_private: ip.is_private(),
            is_loopback: ip.is_loopback(),
            is_link_local: ip.is_link_local(),
            is_multicast: ip.is_multicast(),
            is_unspecified: ip.is_unspecified(),
            is_documentation: is_ipv4_documentation(ip),
            is_broadcast: ip.is_broadcast(),
            is_metadata_endpoint: is_ipv4_metadata_endpoint(ip),
            is_unique_local: false,
        },
        IpAddr::V6(ip) => {
            if let Some(mapped) = ip.to_ipv4_mapped() {
                return classify_ip(IpAddr::V4(mapped));
            }
            let first_segment = ip.segments()[0];
            let is_unique_local = (first_segment & 0xfe00) == 0xfc00;
            let is_link_local = (first_segment & 0xffc0) == 0xfe80;
            let is_documentation = ip.segments()[0] == 0x2001 && ip.segments()[1] == 0x0db8;
            IpClassification {
                is_private: is_unique_local,
                is_loopback: ip.is_loopback(),
                is_link_local,
                is_multicast: ip.is_multicast(),
                is_unspecified: ip.is_unspecified(),
                is_documentation,
                is_broadcast: false,
                is_metadata_endpoint: is_ipv6_metadata_endpoint(ip),
                is_unique_local,
            }
        }
    }
}

pub(in crate::stdlib::net) fn enforce_resolved_target_policy(
    targets: &[(u16, SocketAddr)],
    allow_private: bool,
) -> Result<(), String> {
    for (_, addr) in targets {
        enforce_target_policy(addr.ip(), allow_private)?;
    }
    Ok(())
}

fn enforce_target_policy(ip: IpAddr, allow_private: bool) -> Result<(), String> {
    let classification = classify_ip(ip);
    let never_allowed = classification.is_metadata_endpoint
        || classification.is_unspecified
        || classification.is_multicast
        || classification.is_documentation
        || classification.is_broadcast;
    if never_allowed {
        return Err(
            "Network target denied by policy: special-purpose targets are not allowed".to_string(),
        );
    }

    let private_target = classification.is_private
        || classification.is_loopback
        || classification.is_link_local
        || classification.is_unique_local;
    if !private_target {
        return Ok(());
    }
    if !allow_private || !process_allows_private_targets() {
        return Err(
            "Network target denied by policy: private targets require NTNT_NET_ALLOW_PRIVATE=1"
                .to_string(),
        );
    }
    Ok(())
}

fn process_allows_private_targets() -> bool {
    matches!(
        std::env::var("NTNT_NET_ALLOW_PRIVATE").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn is_ipv4_documentation(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    matches!(
        octets,
        [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
    )
}

fn is_ipv4_metadata_endpoint(ip: Ipv4Addr) -> bool {
    matches!(ip.octets(), [169, 254, 169, 254] | [169, 254, 170, 2])
}

fn is_ipv6_metadata_endpoint(ip: Ipv6Addr) -> bool {
    ip.segments() == [0xfd00, 0x0ec2, 0, 0, 0, 0, 0, 0x0254]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_ipv4_mapped_ipv6_by_embedded_address() {
        let mapped_loopback = "::ffff:127.0.0.1".parse::<IpAddr>().unwrap();
        let mapped_metadata = "::ffff:169.254.169.254".parse::<IpAddr>().unwrap();

        assert!(classify_ip(mapped_loopback).is_loopback);
        assert!(classify_ip(mapped_metadata).is_metadata_endpoint);
    }

    #[test]
    fn policy_rejects_special_ranges_even_when_private_targets_are_allowed() {
        for target in [
            "169.254.169.254:80",
            "169.254.170.2:80",
            "[::ffff:169.254.169.254]:80",
            "[::ffff:169.254.170.2]:80",
            "224.0.0.1:80",
            "192.0.2.1:80",
            "255.255.255.255:80",
        ] {
            let addr = target.parse::<SocketAddr>().unwrap();
            let err = enforce_resolved_target_policy(&[(80, addr)], true).unwrap_err();
            assert!(err.contains("special-purpose targets are not allowed"));
        }
    }

    #[test]
    fn policy_checks_all_resolved_addresses_before_probe() {
        let public = "93.184.216.34:443".parse::<SocketAddr>().unwrap();
        let private = "127.0.0.1:443".parse::<SocketAddr>().unwrap();
        let err =
            enforce_resolved_target_policy(&[(443, public), (443, private)], false).unwrap_err();
        assert!(err.contains("Network target denied by policy"));
    }
}
