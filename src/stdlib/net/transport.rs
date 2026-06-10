//! UDP and TCP traceroute probe methods.
//!
//! Both reuse the shared raw ICMP receive socket for intermediate hops — a
//! router whose TTL expires sends ICMP Time Exceeded regardless of what the
//! expiring packet carried — and differ only in how a probe is sent and how
//! the destination signals "reached":
//!
//! - **UDP**: a datagram to an unused port. Reached = ICMP Port Unreachable
//!   from the target (IPv4 type 3/code 3, IPv6 type 1/code 4). Send is
//!   unprivileged; only the raw ICMP receive needs CAP_NET_RAW.
//! - **TCP**: a raw SYN to a real port. Reached = SYN-ACK or RST from the
//!   target, which arrives as a TCP segment on a separate raw TCP socket — so
//!   the TCP method watches two sockets (raw ICMP for hops, raw TCP for the
//!   destination reply). Raw TCP send/receive needs CAP_NET_RAW and, in
//!   practice, Linux (BSD/Windows do not deliver TCP to raw sockets).

use super::icmp::{
    local_source_address_for, next_icmp_ident, open_raw_icmp_recv, parse_icmp_message,
    probe_io_failure, probe_socket_unavailable,
};
use super::probe::{
    quoted_inner_v4, quoted_inner_v6, set_socket_hop_limit, transport_checksum, HopProbe,
    ProbeFailure, TraceProbe,
};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::io::ErrorKind;
use std::mem::MaybeUninit;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::thread;
use std::time::{Duration, Instant};

const LABEL: &str = "traceroute";
const PAYLOAD: [u8; 32] = [0u8; 32];
/// How often the TCP method polls its two sockets while waiting for a reply.
const TCP_POLL_SLICE: Duration = Duration::from_millis(5);

// ---------------------------------------------------------------------------
// Shared ICMP-error matching for transport probes
// ---------------------------------------------------------------------------

/// If a datagram read from the raw ICMP socket is an ICMP error that quotes
/// one of our transport probes, returns the responder address and the ICMP
/// type/code. `protocol` is the quoted IP protocol (17 UDP, 6 TCP) and
/// `src_port`/`dst_port` identify our flow.
fn matched_icmp_error(
    bytes: &[u8],
    target_ip: IpAddr,
    fallback_source: IpAddr,
    protocol: u8,
    src_port: u16,
    dst_port: u16,
) -> Option<(IpAddr, u8, u8)> {
    let message = parse_icmp_message(bytes, target_ip, fallback_source)?;
    // Only ICMP error types carry a quoted packet: Time Exceeded and the
    // Destination Unreachable family (per IP version).
    let is_error = match target_ip {
        IpAddr::V4(_) => matches!(message.icmp_type, 3 | 11),
        IpAddr::V6(_) => matches!(message.icmp_type, 1..=4),
    };
    if !is_error {
        return None;
    }
    if !quoted_matches_transport(message.quoted, target_ip, protocol, src_port, dst_port) {
        return None;
    }
    Some((message.source, message.icmp_type, message.icmp_code))
}

/// True when the packet an ICMP error quotes is one of our transport probes,
/// matched by IP protocol and source/destination ports (the first four bytes
/// of both the UDP and TCP headers, always within the quoted 8 bytes).
fn quoted_matches_transport(
    quoted: &[u8],
    target_ip: IpAddr,
    protocol: u8,
    src_port: u16,
    dst_port: u16,
) -> bool {
    let inner = match target_ip {
        IpAddr::V4(_) => quoted_inner_v4(quoted),
        IpAddr::V6(_) => quoted_inner_v6(quoted),
    };
    let Some((proto, transport)) = inner else {
        return false;
    };
    proto == protocol
        && transport.len() >= 4
        && u16::from_be_bytes([transport[0], transport[1]]) == src_port
        && u16::from_be_bytes([transport[2], transport[3]]) == dst_port
}

/// ICMP Time Exceeded type per IP version (IPv4 11, IPv6 3).
fn is_time_exceeded(target_ip: IpAddr, icmp_type: u8) -> bool {
    match target_ip {
        IpAddr::V4(_) => icmp_type == 11,
        IpAddr::V6(_) => icmp_type == 3,
    }
}

/// True for an ICMP "port unreachable" — the UDP destination-reached signal
/// (IPv4 type 3/code 3, IPv6 type 1/code 4).
fn is_port_unreachable(target_ip: IpAddr, icmp_type: u8, icmp_code: u8) -> bool {
    match target_ip {
        IpAddr::V4(_) => icmp_type == 3 && icmp_code == 3,
        IpAddr::V6(_) => icmp_type == 1 && icmp_code == 4,
    }
}

fn icmp_error_message(source: IpAddr, icmp_type: u8, icmp_code: u8) -> String {
    format!("ICMP error from {source}: type {icmp_type} code {icmp_code}")
}

fn unspecified_for(target_ip: IpAddr) -> IpAddr {
    match target_ip {
        IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::UNSPECIFIED),
    }
}

/// Non-blocking `recv_from` into `buf`; `Ok(None)` when nothing is ready.
fn try_recv<'a>(
    socket: &Socket,
    buf: &'a mut [MaybeUninit<u8>],
) -> Result<Option<(&'a [u8], IpAddr)>, ProbeFailure> {
    match socket.recv_from(buf) {
        Ok((len, from)) => {
            let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
            let source = from.as_socket().map(|addr| addr.ip());
            Ok(Some((
                bytes,
                source.unwrap_or(Ipv4Addr::UNSPECIFIED.into()),
            )))
        }
        Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => Ok(None),
        Err(err) if err.kind() == ErrorKind::Interrupted => Ok(None),
        Err(err) => Err(probe_io_failure(LABEL, err)),
    }
}

// ---------------------------------------------------------------------------
// UDP method
// ---------------------------------------------------------------------------

pub(super) struct UdpTraceProbe {
    icmp: Socket,
    udp: Socket,
    target_ip: IpAddr,
    src_port: u16,
    dst_port: u16,
}

pub(super) fn open_udp_trace_probe(
    target_ip: IpAddr,
    dst_port: u16,
    timeout: Duration,
) -> Result<UdpTraceProbe, ProbeFailure> {
    let icmp = open_raw_icmp_recv(target_ip, timeout)?;
    let domain = match target_ip {
        IpAddr::V4(_) => Domain::IPV4,
        IpAddr::V6(_) => Domain::IPV6,
    };
    let udp = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP))
        .map_err(|err| probe_socket_unavailable(LABEL, err))?;
    udp.set_write_timeout(Some(timeout))
        .map_err(|err| probe_socket_unavailable(LABEL, err))?;
    // Bind to an ephemeral port so we know our source port for matching the
    // UDP header the ICMP errors quote back.
    udp.bind(&SockAddr::from(SocketAddr::new(
        unspecified_for(target_ip),
        0,
    )))
    .map_err(|err| probe_socket_unavailable(LABEL, err))?;
    let src_port = udp
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_socket())
        .map(|addr| addr.port())
        .ok_or_else(|| {
            ProbeFailure::Backend(format!(
                "{LABEL} failed: could not determine UDP source port"
            ))
        })?;
    Ok(UdpTraceProbe {
        icmp,
        udp,
        target_ip,
        src_port,
        dst_port,
    })
}

impl TraceProbe for UdpTraceProbe {
    fn probe(&mut self, ttl: u8, _seq: u16, deadline: Instant) -> Result<HopProbe, ProbeFailure> {
        set_socket_hop_limit(LABEL, &self.udp, self.target_ip, ttl)?;
        let sent_at = Instant::now();
        self.udp
            .send_to(
                &PAYLOAD,
                &SockAddr::from(SocketAddr::new(self.target_ip, self.dst_port)),
            )
            .map_err(|err| probe_io_failure(LABEL, err))?;

        loop {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Ok(HopProbe::TimedOut);
            };
            self.icmp
                .set_read_timeout(Some(remaining))
                .map_err(|err| probe_socket_unavailable(LABEL, err))?;
            let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
            let (bytes, fallback) = match self.icmp.recv_from(&mut buffer) {
                Ok((len, from)) => {
                    let bytes =
                        unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), len) };
                    let from = from.as_socket().map(|a| a.ip()).unwrap_or(self.target_ip);
                    (bytes, from)
                }
                Err(err) if err.kind() == ErrorKind::Interrupted => continue,
                Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    return Ok(HopProbe::TimedOut);
                }
                Err(err) => return Err(probe_io_failure(LABEL, err)),
            };
            let latency_ms = sent_at.elapsed().as_secs_f64() * 1000.0;
            if let Some((from, icmp_type, icmp_code)) = matched_icmp_error(
                bytes,
                self.target_ip,
                fallback,
                17,
                self.src_port,
                self.dst_port,
            ) {
                if is_time_exceeded(self.target_ip, icmp_type) {
                    return Ok(HopProbe::Hop { from, latency_ms });
                }
                if is_port_unreachable(self.target_ip, icmp_type, icmp_code) {
                    return Ok(HopProbe::Reached { from, latency_ms });
                }
                return Ok(HopProbe::Terminal {
                    from,
                    latency_ms,
                    message: icmp_error_message(from, icmp_type, icmp_code),
                });
            }
            // Not ours (stray ICMP): keep waiting within the budget.
        }
    }
}

// ---------------------------------------------------------------------------
// TCP method
// ---------------------------------------------------------------------------

pub(super) struct TcpTraceProbe {
    icmp: Socket,
    tcp: Socket,
    target_ip: IpAddr,
    local_ip: IpAddr,
    src_port: u16,
    dst_port: u16,
}

pub(super) fn open_tcp_trace_probe(
    target_ip: IpAddr,
    dst_port: u16,
    timeout: Duration,
) -> Result<TcpTraceProbe, ProbeFailure> {
    // Raw TCP reply capture only works on Linux; BSD/Windows do not deliver
    // TCP segments to raw sockets. Fail honestly rather than time out.
    if !cfg!(target_os = "linux") {
        return Err(ProbeFailure::Backend(format!(
            "{LABEL} unavailable: TCP traceroute requires Linux raw sockets"
        )));
    }
    let local_ip = local_source_address_for(target_ip).ok_or_else(|| {
        ProbeFailure::Backend(format!(
            "{LABEL} failed: could not determine local source address for TCP checksum"
        ))
    })?;
    let icmp = open_raw_icmp_recv(target_ip, timeout)?;
    icmp.set_nonblocking(true)
        .map_err(|err| probe_socket_unavailable(LABEL, err))?;
    let domain = match target_ip {
        IpAddr::V4(_) => Domain::IPV4,
        IpAddr::V6(_) => Domain::IPV6,
    };
    let tcp = Socket::new(domain, Type::RAW, Some(Protocol::TCP)).map_err(|err| {
        if err.kind() == ErrorKind::PermissionDenied {
            ProbeFailure::Backend(format!(
                "{LABEL} unavailable: raw TCP socket denied: {err}. \
                 Grant CAP_NET_RAW (Docker: cap_add: [NET_RAW]) or run with elevated privileges"
            ))
        } else {
            probe_socket_unavailable(LABEL, err)
        }
    })?;
    tcp.set_nonblocking(true)
        .map_err(|err| probe_socket_unavailable(LABEL, err))?;
    // Source port: a stable ephemeral value for this trace. The kernel has no
    // socket bound to it, so it will RST the target's SYN-ACK itself — which
    // also tears down the half-open connection we would otherwise leave.
    let src_port = 33000u16.wrapping_add(next_icmp_ident() % 20000);
    Ok(TcpTraceProbe {
        icmp,
        tcp,
        target_ip,
        local_ip,
        src_port,
        dst_port,
    })
}

impl TraceProbe for TcpTraceProbe {
    fn probe(&mut self, ttl: u8, seq: u16, deadline: Instant) -> Result<HopProbe, ProbeFailure> {
        set_socket_hop_limit(LABEL, &self.tcp, self.target_ip, ttl)?;
        // Encode the hop in the TCP sequence number so a SYN-ACK's ack number
        // (seq + 1) and the quoted TCP header self-identify the probe.
        let tcp_seq = u32::from(seq) << 8 | 0x53;
        let syn = build_tcp_syn(
            self.local_ip,
            self.target_ip,
            self.src_port,
            self.dst_port,
            tcp_seq,
        );
        let sent_at = Instant::now();
        self.tcp
            .send_to(
                &syn,
                &SockAddr::from(SocketAddr::new(self.target_ip, self.dst_port)),
            )
            .map_err(|err| probe_io_failure(LABEL, err))?;

        loop {
            if deadline.checked_duration_since(Instant::now()).is_none() {
                return Ok(HopProbe::TimedOut);
            }
            let latency_ms = sent_at.elapsed().as_secs_f64() * 1000.0;

            // Intermediate hops + terminal errors arrive on the raw ICMP socket.
            let mut icmp_buf = [MaybeUninit::<u8>::uninit(); 2048];
            if let Some((bytes, fallback)) = try_recv(&self.icmp, &mut icmp_buf)? {
                if let Some((from, icmp_type, icmp_code)) = matched_icmp_error(
                    bytes,
                    self.target_ip,
                    fallback,
                    6,
                    self.src_port,
                    self.dst_port,
                ) {
                    if is_time_exceeded(self.target_ip, icmp_type) {
                        return Ok(HopProbe::Hop { from, latency_ms });
                    }
                    return Ok(HopProbe::Terminal {
                        from,
                        latency_ms,
                        message: icmp_error_message(from, icmp_type, icmp_code),
                    });
                }
            }

            // The destination's SYN-ACK/RST arrives as a TCP segment.
            let mut tcp_buf = [MaybeUninit::<u8>::uninit(); 2048];
            if let Some((bytes, _)) = try_recv(&self.tcp, &mut tcp_buf)? {
                if tcp_reply_is_ours(bytes, self.target_ip, self.src_port, self.dst_port, tcp_seq) {
                    return Ok(HopProbe::Reached {
                        from: self.target_ip,
                        latency_ms,
                    });
                }
            }

            let remaining = match deadline.checked_duration_since(Instant::now()) {
                Some(remaining) => remaining,
                None => return Ok(HopProbe::TimedOut),
            };
            thread::sleep(TCP_POLL_SLICE.min(remaining));
        }
    }
}

/// Builds a TCP SYN segment with the pseudo-header checksum filled in. The
/// kernel adds the IP header (TTL set via the socket), so only the TCP
/// segment is crafted here.
fn build_tcp_syn(
    local_ip: IpAddr,
    target_ip: IpAddr,
    src_port: u16,
    dst_port: u16,
    seq: u32,
) -> Vec<u8> {
    let mut tcp = vec![0u8; 20];
    tcp[0..2].copy_from_slice(&src_port.to_be_bytes());
    tcp[2..4].copy_from_slice(&dst_port.to_be_bytes());
    tcp[4..8].copy_from_slice(&seq.to_be_bytes());
    // Data offset = 5 (20-byte header), no options.
    tcp[12] = 5 << 4;
    // Flags: SYN.
    tcp[13] = 0x02;
    // Advertised window.
    tcp[14..16].copy_from_slice(&64240u16.to_be_bytes());
    let checksum = transport_checksum(local_ip, target_ip, 6, &tcp);
    tcp[16..18].copy_from_slice(&checksum.to_be_bytes());
    tcp
}

/// True when a TCP segment is the destination's reply to our SYN: matching
/// ports and either SYN-ACK (acknowledging our sequence) or RST.
fn tcp_reply_is_ours(
    bytes: &[u8],
    target_ip: IpAddr,
    our_src_port: u16,
    target_port: u16,
    our_seq: u32,
) -> bool {
    // IPv4 raw sockets prepend the outer IP header; IPv6 raw sockets do not.
    let tcp = match target_ip {
        IpAddr::V4(_) if bytes.len() >= 20 && bytes[0] >> 4 == 4 => {
            let header_len = usize::from(bytes[0] & 0x0f) * 4;
            if header_len < 20 || bytes.len() < header_len + 20 {
                return false;
            }
            &bytes[header_len..]
        }
        IpAddr::V4(_) => return false,
        IpAddr::V6(_) => bytes,
    };
    if tcp.len() < 20 {
        return false;
    }
    let source_port = u16::from_be_bytes([tcp[0], tcp[1]]);
    let dest_port = u16::from_be_bytes([tcp[2], tcp[3]]);
    if source_port != target_port || dest_port != our_src_port {
        return false;
    }
    let flags = tcp[13];
    let syn = flags & 0x02 != 0;
    let ack = flags & 0x10 != 0;
    let rst = flags & 0x04 != 0;
    if rst {
        return true;
    }
    if syn && ack {
        let ack_num = u32::from_be_bytes([tcp[8], tcp[9], tcp[10], tcp[11]]);
        return ack_num == our_seq.wrapping_add(1);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v4(a: u8, b: u8, c: u8, d: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(a, b, c, d))
    }

    fn ipv4_packet(protocol: u8, transport: &[u8]) -> Vec<u8> {
        let mut pkt = vec![0u8; 20];
        pkt[0] = 0x45;
        pkt[9] = protocol;
        pkt.extend_from_slice(transport);
        pkt
    }

    /// A synthetic ICMPv4 error (type/code) quoting an inner IPv4 packet.
    fn icmpv4_error(icmp_type: u8, icmp_code: u8, quoted: &[u8]) -> Vec<u8> {
        let mut msg = vec![icmp_type, icmp_code, 0, 0, 0, 0, 0, 0];
        msg.extend_from_slice(quoted);
        msg
    }

    #[test]
    fn tcp_syn_has_syn_flag_and_valid_checksum() {
        let local = v4(192, 0, 2, 1);
        let target = v4(192, 0, 2, 2);
        let syn = build_tcp_syn(local, target, 40000, 80, 0x12345678);
        assert_eq!(u16::from_be_bytes([syn[0], syn[1]]), 40000);
        assert_eq!(u16::from_be_bytes([syn[2], syn[3]]), 80);
        assert_eq!(
            u32::from_be_bytes([syn[4], syn[5], syn[6], syn[7]]),
            0x12345678
        );
        assert_eq!(syn[13] & 0x02, 0x02, "SYN flag set");
        assert_eq!(syn[13] & 0x10, 0, "ACK flag clear");
        // Checksum over the segment + pseudo-header verifies to zero.
        assert_eq!(transport_checksum(local, target, 6, &syn), 0);
    }

    #[test]
    fn matched_icmp_error_identifies_our_udp_flow() {
        let inner_udp = {
            let mut u = vec![0u8; 8];
            u[0..2].copy_from_slice(&50000u16.to_be_bytes()); // src
            u[2..4].copy_from_slice(&33434u16.to_be_bytes()); // dst
            u
        };
        let quoted = ipv4_packet(17, &inner_udp);
        // Time Exceeded (type 11) quoting our UDP probe.
        let pkt = icmpv4_error(11, 0, &quoted);
        let target = v4(8, 8, 8, 8);
        let router = v4(10, 0, 0, 1);
        let got = matched_icmp_error(&pkt, target, router, 17, 50000, 33434);
        assert_eq!(got, Some((router, 11, 0)));
        assert!(is_time_exceeded(target, 11));
        // Wrong source port: not ours.
        assert!(matched_icmp_error(&pkt, target, router, 17, 9999, 33434).is_none());
        // Wrong protocol (TCP matcher against a UDP quote): not ours.
        assert!(matched_icmp_error(&pkt, target, router, 6, 50000, 33434).is_none());
    }

    #[test]
    fn udp_port_unreachable_is_the_reached_signal() {
        let inner_udp = {
            let mut u = vec![0u8; 8];
            u[0..2].copy_from_slice(&50000u16.to_be_bytes());
            u[2..4].copy_from_slice(&33434u16.to_be_bytes());
            u
        };
        let quoted = ipv4_packet(17, &inner_udp);
        let pkt = icmpv4_error(3, 3, &quoted); // dest unreachable / port unreachable
        let target = v4(8, 8, 8, 8);
        let (from, t, c) = matched_icmp_error(&pkt, target, target, 17, 50000, 33434).unwrap();
        assert!(!is_time_exceeded(target, t));
        assert!(is_port_unreachable(target, t, c));
        assert_eq!(from, target);
    }

    #[test]
    fn tcp_reply_detects_syn_ack_and_rst_for_our_flow() {
        let target = v4(93, 184, 216, 34);
        let our_src = 40000u16;
        let target_port = 80u16;
        let our_seq = 0x11223344u32;

        let mut synack = vec![0u8; 20];
        synack[0..2].copy_from_slice(&target_port.to_be_bytes());
        synack[2..4].copy_from_slice(&our_src.to_be_bytes());
        synack[8..12].copy_from_slice(&our_seq.wrapping_add(1).to_be_bytes()); // ack
        synack[13] = 0x12; // SYN+ACK
        let synack_pkt = ipv4_packet(6, &synack);
        assert!(tcp_reply_is_ours(
            &synack_pkt,
            target,
            our_src,
            target_port,
            our_seq
        ));

        // SYN-ACK with the wrong ack number is not ours.
        let mut wrong = synack.clone();
        wrong[8..12].copy_from_slice(&0u32.to_be_bytes());
        assert!(!tcp_reply_is_ours(
            &ipv4_packet(6, &wrong),
            target,
            our_src,
            target_port,
            our_seq
        ));

        // RST for our flow is "reached" regardless of sequence.
        let mut rst = vec![0u8; 20];
        rst[0..2].copy_from_slice(&target_port.to_be_bytes());
        rst[2..4].copy_from_slice(&our_src.to_be_bytes());
        rst[13] = 0x04; // RST
        assert!(tcp_reply_is_ours(
            &ipv4_packet(6, &rst),
            target,
            our_src,
            target_port,
            our_seq
        ));

        // A segment for a different port pair is ignored.
        let mut other = synack.clone();
        other[2..4].copy_from_slice(&12345u16.to_be_bytes());
        assert!(!tcp_reply_is_ours(
            &ipv4_packet(6, &other),
            target,
            our_src,
            target_port,
            our_seq
        ));
    }
}
