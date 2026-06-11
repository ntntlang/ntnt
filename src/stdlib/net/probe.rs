//! Shared probe substrate for `std/net` diagnostics.
//!
//! Probe drivers (ICMP ping today, traceroute and richer TCP probing later)
//! share two concerns that must behave identically everywhere:
//!
//! 1. **Failure classification.** A probe can fail because the *target* is
//!    unreachable (a valid diagnostic outcome, reported per-attempt) or
//!    because the *backend* could not probe at all (permissions, socket
//!    setup, impossible configuration — surfaced as `Err` to the caller).
//!    [`ProbeFailure`] carries that distinction as a type so it survives
//!    every layer; string sniffing on error messages is not allowed.
//!
//! 2. **Deadline budgeting.** A probe sequence shares one global deadline
//!    across attempts and inter-attempt intervals. [`probe_attempt_budget`]
//!    divides the remaining budget so a requested count either completes or
//!    fails loudly — it never silently shrinks.

use socket2::Socket;
use std::net::IpAddr;
use std::time::{Duration, Instant};

/// Why a probe failed, preserved as a type across all probe layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeFailure {
    /// The target itself is unreachable: no route, host down, or an ICMP
    /// error quoting our probe. This is a valid probe outcome — drivers
    /// record it as a failed attempt rather than aborting.
    Target(String),
    /// The probe machinery failed: permissions, socket errors, or a
    /// configuration that cannot be satisfied. Says nothing about the
    /// target and propagates as `Err` to the caller.
    Backend(String),
}

impl ProbeFailure {
    pub(crate) fn is_target(&self) -> bool {
        matches!(self, ProbeFailure::Target(_))
    }

    pub(crate) fn into_message(self) -> String {
        match self {
            ProbeFailure::Target(message) | ProbeFailure::Backend(message) => message,
        }
    }
}

impl std::fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProbeFailure::Target(message) | ProbeFailure::Backend(message) => f.write_str(message),
        }
    }
}

/// Splits the remaining global deadline across the outstanding attempts of a
/// probe sequence, reserving room for the intervals between them.
///
/// Returns the time budget for the next attempt, or a backend failure when
/// the remaining budget cannot fit the outstanding attempts plus intervals.
/// `label` names the calling probe (e.g. "ICMP ping") in error messages.
pub(crate) fn probe_attempt_budget(
    label: &str,
    remaining: Duration,
    remaining_attempts: usize,
    interval: Duration,
) -> Result<Duration, ProbeFailure> {
    let attempts = remaining_attempts.max(1);
    let future_intervals = interval
        .checked_mul(attempts.saturating_sub(1) as u32)
        .ok_or_else(|| ProbeFailure::Backend(format!("{label} interval budget overflowed")))?;
    if remaining <= future_intervals {
        return Err(ProbeFailure::Backend(format!(
            "{label} timeout_ms is too small for count {} and interval_ms {}",
            remaining_attempts,
            interval.as_millis()
        )));
    }
    let attempt_budget = (remaining - future_intervals) / (attempts as u32);
    if attempt_budget.is_zero() {
        return Err(ProbeFailure::Backend(format!(
            "{label} timeout_ms is too small to send requested probes"
        )));
    }
    Ok(attempt_budget)
}

// ---------------------------------------------------------------------------
// Checksums (shared by ICMP echo, ICMPv6, and TCP/UDP probe packets)
// ---------------------------------------------------------------------------

/// RFC 1071 one's-complement Internet checksum over `data`.
pub(super) fn internet_checksum(data: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut chunks = data.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    if let Some(&byte) = chunks.remainder().first() {
        sum += u16::from_be_bytes([byte, 0]) as u32;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Transport checksum over the IPv4/IPv6 pseudo-header followed by the
/// transport bytes. `protocol` is the IP protocol number (6 TCP, 17 UDP,
/// 58 ICMPv6). `src` and `dst` must be the same family; mismatches return 0
/// (callers guard family before calling).
pub(super) fn transport_checksum(src: IpAddr, dst: IpAddr, protocol: u8, transport: &[u8]) -> u16 {
    let mut pseudo = Vec::with_capacity(40 + transport.len());
    match (src, dst) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            pseudo.extend_from_slice(&s.octets());
            pseudo.extend_from_slice(&d.octets());
            pseudo.push(0);
            pseudo.push(protocol);
            pseudo.extend_from_slice(&(transport.len() as u16).to_be_bytes());
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            pseudo.extend_from_slice(&s.octets());
            pseudo.extend_from_slice(&d.octets());
            pseudo.extend_from_slice(&(transport.len() as u32).to_be_bytes());
            pseudo.extend_from_slice(&[0, 0, 0, protocol]);
        }
        _ => return 0,
    }
    pseudo.extend_from_slice(transport);
    internet_checksum(&pseudo)
}

// ---------------------------------------------------------------------------
// Quoted-packet extraction (the inner packet an ICMP error echoes back)
// ---------------------------------------------------------------------------

/// From the bytes an ICMP error quotes (an IPv4 packet: IP header + at least
/// 8 bytes of its payload), return the inner IP protocol number and the start
/// of the upper-layer header. Used to confirm an ICMP error refers to one of
/// our probes regardless of probe protocol (ICMP/UDP/TCP).
pub(super) fn quoted_inner_v4(quoted: &[u8]) -> Option<(u8, &[u8])> {
    if quoted.len() < 20 || quoted[0] >> 4 != 4 {
        return None;
    }
    let header_len = usize::from(quoted[0] & 0x0f) * 4;
    if header_len < 20 || quoted.len() < header_len + 8 {
        return None;
    }
    Some((quoted[9], &quoted[header_len..]))
}

/// IPv6 variant: walk hop-by-hop/routing/fragment/destination extension
/// headers to the upper-layer header, returning its protocol number and bytes.
pub(super) fn quoted_inner_v6(quoted: &[u8]) -> Option<(u8, &[u8])> {
    if quoted.len() < 40 || quoted[0] >> 4 != 6 {
        return None;
    }
    let mut next_header = quoted[6];
    let mut offset = 40usize;
    loop {
        // Not an extension header => this is the upper-layer protocol.
        if !matches!(next_header, 0 | 43 | 44 | 60) {
            let inner = quoted.get(offset..).filter(|payload| payload.len() >= 8)?;
            return Some((next_header, inner));
        }
        let ext_len = if next_header == 44 {
            8
        } else {
            (usize::from(*quoted.get(offset + 1)?) + 1) * 8
        };
        next_header = *quoted.get(offset)?;
        offset = offset.checked_add(ext_len)?;
        if quoted.len() < offset + 8 {
            return None;
        }
    }
}

// ---------------------------------------------------------------------------
// TTL / hop-limit
// ---------------------------------------------------------------------------

/// Sets the outgoing TTL (IPv4) or unicast hop limit (IPv6) on a probe socket.
pub(super) fn set_socket_hop_limit(
    label: &str,
    socket: &Socket,
    target_ip: IpAddr,
    hop: u8,
) -> Result<(), ProbeFailure> {
    let limit = u32::from(hop);
    match target_ip {
        IpAddr::V4(_) => socket.set_ttl_v4(limit),
        IpAddr::V6(_) => socket.set_unicast_hops_v6(limit),
    }
    .map_err(|err| ProbeFailure::Backend(format!("{label} failed: could not set hop limit: {err}")))
}

// ---------------------------------------------------------------------------
// Traceroute probe abstraction (one TraceProbe per protocol method)
// ---------------------------------------------------------------------------

/// What a received reply says about the hop it answers.
#[derive(Debug, PartialEq)]
pub(super) enum HopKind {
    /// A router reported TTL/hop-limit expiry: an intermediate hop.
    Hop,
    /// The destination responded (echo reply / port unreachable / SYN-ACK /
    /// RST depending on method): the trace reaches here.
    Reached,
    /// A terminal ICMP error other than TTL expiry (e.g. net/host
    /// unreachable, administratively prohibited): records the hop and stops.
    Terminal(String),
}

/// A reply matched back to the probe (hop) that elicited it. Returned by
/// [`TraceProbe::recv`]; `seq` is the per-hop correlation token the driver
/// sent, recovered from the reply so concurrent probes demultiplex correctly.
#[derive(Debug)]
pub(super) struct HopReply {
    pub(super) seq: u16,
    pub(super) from: IpAddr,
    pub(super) latency_ms: f64,
    pub(super) kind: HopKind,
}

/// A protocol-specific traceroute prober with send decoupled from receive so
/// the driver can emit a whole TTL burst before collecting replies.
/// Implementations own their sockets and track each probe's send time (keyed
/// by `seq`) for accurate per-hop latency.
pub(super) trait TraceProbe {
    /// Set the hop limit and send one probe carrying correlation token `seq`.
    /// Records the send time; does not wait for a reply. `deadline` bounds the
    /// send itself so a blocking send cannot push the trace past its timeout.
    fn send(&mut self, ttl: u8, seq: u16, deadline: Instant) -> Result<(), ProbeFailure>;

    /// Wait up to `deadline` for the next reply matching one of our
    /// outstanding probes, recovering which hop (`seq`) it answers.
    /// `Ok(None)` on timeout. Called repeatedly to drain a burst.
    fn recv(&mut self, deadline: Instant) -> Result<Option<HopReply>, ProbeFailure>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internet_checksum_of_self_checksummed_packet_is_zero() {
        // type 8 echo, ident 0x1234, seq 7, checksum field filled in.
        let mut pkt = vec![8u8, 0, 0, 0, 0x12, 0x34, 0, 7];
        let cksum = internet_checksum(&pkt);
        pkt[2..4].copy_from_slice(&cksum.to_be_bytes());
        assert_eq!(internet_checksum(&pkt), 0);
    }

    #[test]
    fn quoted_inner_v4_extracts_protocol_and_payload() {
        // Minimal IPv4 header (IHL=5) with protocol=6 (TCP) and 8 payload bytes.
        let mut quoted = vec![0u8; 20];
        quoted[0] = 0x45;
        quoted[9] = 6;
        quoted.extend_from_slice(&[0xAA; 8]);
        let (proto, inner) = quoted_inner_v4(&quoted).unwrap();
        assert_eq!(proto, 6);
        assert_eq!(inner, &[0xAA; 8]);
        // Truncated payload (<8 bytes after header) is rejected.
        assert!(quoted_inner_v4(&quoted[..24]).is_none());
    }

    #[test]
    fn quoted_inner_v6_walks_to_upper_layer() {
        // IPv6 header, next-header = 17 (UDP), 8 UDP bytes.
        let mut quoted = vec![0u8; 40];
        quoted[0] = 0x60;
        quoted[6] = 17;
        quoted.extend_from_slice(&[0xBB; 8]);
        let (proto, inner) = quoted_inner_v6(&quoted).unwrap();
        assert_eq!(proto, 17);
        assert_eq!(inner, &[0xBB; 8]);
    }

    #[test]
    fn transport_checksum_matches_known_pseudo_header() {
        use std::net::Ipv4Addr;
        let src = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let dst = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 2));
        let mut seg = vec![0u8; 20];
        // ports + seq so it isn't all zero
        seg[0..2].copy_from_slice(&12345u16.to_be_bytes());
        seg[2..4].copy_from_slice(&80u16.to_be_bytes());
        let cksum = transport_checksum(src, dst, 6, &seg);
        seg[16..18].copy_from_slice(&cksum.to_be_bytes());
        // Recomputing over the now-checksummed segment yields zero.
        assert_eq!(transport_checksum(src, dst, 6, &seg), 0);
    }

    #[test]
    fn probe_attempt_budget_rejects_impossible_count_interval() {
        let failure = probe_attempt_budget(
            "ICMP ping",
            Duration::from_millis(100),
            3,
            Duration::from_millis(60),
        )
        .unwrap_err();
        assert!(!failure.is_target());
        assert!(failure.into_message().contains("timeout_ms is too small"));
    }

    #[test]
    fn probe_attempt_budget_divides_remaining_across_attempts() {
        let budget = probe_attempt_budget(
            "ICMP ping",
            Duration::from_millis(900),
            3,
            Duration::from_millis(100),
        )
        .unwrap();
        // 900ms minus two 100ms intervals leaves 700ms across three probes.
        assert_eq!(budget, Duration::from_millis(700) / 3);
    }

    #[test]
    fn probe_failure_classification_is_typed() {
        let target = ProbeFailure::Target("host down".to_string());
        let backend = ProbeFailure::Backend("permission denied".to_string());
        assert!(target.is_target());
        assert!(!backend.is_target());
        assert_eq!(target.into_message(), "host down");
        assert_eq!(backend.to_string(), "permission denied");
    }
}
