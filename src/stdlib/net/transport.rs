//! UDP and TCP traceroute probe methods.
//!
//! Both reuse the shared raw ICMP receive socket for intermediate hops — a
//! router whose TTL expires sends ICMP Time Exceeded regardless of what the
//! expiring packet carried — and differ only in how a probe is sent and how
//! the destination signals "reached":
//!
//! - **UDP**: a datagram to a per-hop destination port. Reached = ICMP Port
//!   Unreachable from the target (IPv4 type 3/code 3, IPv6 type 1/code 4).
//!   Send is unprivileged; only the raw ICMP receive needs CAP_NET_RAW.
//! - **TCP**: a raw SYN to a real port. Reached = SYN-ACK or RST from the
//!   target, which arrives as a TCP segment on a separate raw TCP socket — so
//!   the TCP method watches two sockets (raw ICMP for hops, raw TCP for the
//!   destination reply). Raw TCP send/receive needs CAP_NET_RAW and Linux.
//!
//! Send is decoupled from receive (the [`TraceProbe`] trait) so the driver can
//! emit a whole TTL burst and then collect. Each reply is demultiplexed back
//! to its hop by a per-hop token: UDP encodes the hop in the destination port,
//! TCP in the sequence number.

use super::icmp::{
    local_source_address_for, next_icmp_ident, open_raw_icmp_recv, parse_icmp_message,
    probe_io_failure, probe_socket_unavailable,
};
use super::probe::{
    quoted_inner_v4, quoted_inner_v6, set_socket_hop_limit, transport_checksum, HopKind, HopReply,
    ProbeFailure, TraceProbe,
};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::collections::HashMap;
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
// Shared helpers
// ---------------------------------------------------------------------------

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

/// True for an ICMP error type that carries a quoted packet (Time Exceeded or
/// the Destination Unreachable family), per IP version.
fn is_quoting_error(target_ip: IpAddr, icmp_type: u8) -> bool {
    match target_ip {
        IpAddr::V4(_) => matches!(icmp_type, 3 | 11),
        IpAddr::V6(_) => matches!(icmp_type, 1..=4),
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

/// The transport header of the packet an ICMP error quotes, if it is ours by
/// protocol and source port. Returns `(dst_port, tcp_seq)`; `tcp_seq` is
/// present only for TCP (protocol 6), where it carries the per-hop token.
fn recover_quoted_transport(
    quoted: &[u8],
    target_ip: IpAddr,
    protocol: u8,
    our_src_port: u16,
) -> Option<(u16, Option<u32>)> {
    let (proto, transport) = match target_ip {
        IpAddr::V4(_) => quoted_inner_v4(quoted),
        IpAddr::V6(_) => quoted_inner_v6(quoted),
    }?;
    if proto != protocol
        || transport.len() < 4
        || u16::from_be_bytes([transport[0], transport[1]]) != our_src_port
    {
        return None;
    }
    let dst_port = u16::from_be_bytes([transport[2], transport[3]]);
    let tcp_seq = if protocol == 6 && transport.len() >= 8 {
        Some(u32::from_be_bytes([
            transport[4],
            transport[5],
            transport[6],
            transport[7],
        ]))
    } else {
        None
    };
    Some((dst_port, tcp_seq))
}

/// Reads one datagram from the raw ICMP socket with a deadline; `Ok(None)` on
/// timeout. Returns the bytes and the responder address (`recvfrom` source,
/// which IPv6 needs since it has no outer IP header).
fn recv_icmp_with_deadline<'a>(
    socket: &Socket,
    buf: &'a mut [MaybeUninit<u8>],
    deadline: Instant,
    target_ip: IpAddr,
) -> Result<Option<(&'a [u8], IpAddr)>, ProbeFailure> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        socket
            .set_read_timeout(Some(remaining))
            .map_err(|err| probe_socket_unavailable(LABEL, err))?;
        match socket.recv_from(buf) {
            Ok((len, from)) => {
                let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), len) };
                let source = from.as_socket().map(|a| a.ip()).unwrap_or(target_ip);
                return Ok(Some((bytes, source)));
            }
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Ok(None);
            }
            Err(err) => return Err(probe_io_failure(LABEL, err)),
        }
    }
}

/// Non-blocking `recv_from`; `Ok(None)` when nothing is ready.
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
    base_port: u16,
    sent_at: HashMap<u16, Instant>,
}

pub(super) fn open_udp_trace_probe(
    target_ip: IpAddr,
    base_port: u16,
    timeout: Duration,
) -> Result<UdpTraceProbe, ProbeFailure> {
    // UDP traceroute reply capture is validated on Linux; on other platforms
    // raw ICMP receive behaves differently (e.g. Windows rejects the probe
    // send with WSAEINVAL), so fail honestly rather than misreport hops.
    if !cfg!(target_os = "linux") {
        return Err(ProbeFailure::Backend(format!(
            "{LABEL} unavailable: UDP traceroute requires Linux raw sockets"
        )));
    }
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
        base_port,
        sent_at: HashMap::new(),
    })
}

impl UdpTraceProbe {
    /// Per-hop destination port: the base port plus the hop index, so each
    /// hop's quoted UDP header is distinguishable (classic Unix traceroute).
    /// The caller (`parse_traceroute_options`) guarantees no overflow.
    fn dst_port_for(&self, seq: u16) -> u16 {
        self.base_port.saturating_add(seq.saturating_sub(1))
    }

    /// Recovers the hop a quoted UDP destination port encodes.
    fn seq_for(&self, dst_port: u16) -> Option<u16> {
        dst_port.checked_sub(self.base_port)?.checked_add(1)
    }
}

impl TraceProbe for UdpTraceProbe {
    fn send(&mut self, ttl: u8, seq: u16) -> Result<(), ProbeFailure> {
        set_socket_hop_limit(LABEL, &self.udp, self.target_ip, ttl)?;
        let dst_port = self.dst_port_for(seq);
        self.sent_at.insert(seq, Instant::now());
        self.udp
            .send_to(
                &PAYLOAD,
                &SockAddr::from(SocketAddr::new(self.target_ip, dst_port)),
            )
            .map_err(|err| probe_io_failure(LABEL, err))?;
        Ok(())
    }

    fn recv(&mut self, deadline: Instant) -> Result<Option<HopReply>, ProbeFailure> {
        loop {
            let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
            let Some((bytes, fallback)) =
                recv_icmp_with_deadline(&self.icmp, &mut buffer, deadline, self.target_ip)?
            else {
                return Ok(None);
            };
            let received_at = Instant::now();
            let Some(message) = parse_icmp_message(bytes, self.target_ip, fallback) else {
                continue;
            };
            if !is_quoting_error(self.target_ip, message.icmp_type) {
                continue;
            }
            let Some((dst_port, _)) =
                recover_quoted_transport(message.quoted, self.target_ip, 17, self.src_port)
            else {
                continue;
            };
            let Some(seq) = self.seq_for(dst_port) else {
                continue;
            };
            let Some(sent) = self.sent_at.get(&seq) else {
                continue;
            };
            let latency_ms = received_at.saturating_duration_since(*sent).as_secs_f64() * 1000.0;
            let kind = if is_time_exceeded(self.target_ip, message.icmp_type) {
                HopKind::Hop
            } else if is_port_unreachable(self.target_ip, message.icmp_type, message.icmp_code)
                && message.source == self.target_ip
            {
                // Port Unreachable means "reached" only from the destination
                // itself; from an intermediate device it is a terminal error.
                HopKind::Reached
            } else {
                HopKind::Terminal(icmp_error_message(
                    message.source,
                    message.icmp_type,
                    message.icmp_code,
                ))
            };
            return Ok(Some(HopReply {
                seq,
                from: message.source,
                latency_ms,
                kind,
            }));
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
    sent_at: HashMap<u16, Instant>,
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
        sent_at: HashMap::new(),
    })
}

/// Encodes a hop into a TCP sequence number (low byte is a constant tag so a
/// stray segment is unlikely to decode), and back.
fn encode_tcp_seq(hop: u16) -> u32 {
    (u32::from(hop) << 8) | 0x53
}

fn decode_tcp_seq(seq: u32) -> Option<u16> {
    if seq & 0xff != 0x53 {
        return None;
    }
    u16::try_from(seq >> 8).ok()
}

impl TcpTraceProbe {
    /// Classifies an ICMP datagram (intermediate hop or terminal error),
    /// recovering the hop from the quoted TCP sequence number.
    fn classify_icmp(&self, bytes: &[u8], fallback: IpAddr) -> Option<(u16, IpAddr, HopKind)> {
        let message = parse_icmp_message(bytes, self.target_ip, fallback)?;
        if !is_quoting_error(self.target_ip, message.icmp_type) {
            return None;
        }
        let (_, tcp_seq) =
            recover_quoted_transport(message.quoted, self.target_ip, 6, self.src_port)?;
        let seq = decode_tcp_seq(tcp_seq?)?;
        let kind = if is_time_exceeded(self.target_ip, message.icmp_type) {
            HopKind::Hop
        } else {
            HopKind::Terminal(icmp_error_message(
                message.source,
                message.icmp_type,
                message.icmp_code,
            ))
        };
        Some((seq, message.source, kind))
    }
}

impl TraceProbe for TcpTraceProbe {
    fn send(&mut self, ttl: u8, seq: u16) -> Result<(), ProbeFailure> {
        set_socket_hop_limit(LABEL, &self.tcp, self.target_ip, ttl)?;
        let tcp_seq = encode_tcp_seq(seq);
        let syn = build_tcp_syn(
            self.local_ip,
            self.target_ip,
            self.src_port,
            self.dst_port,
            tcp_seq,
        );
        self.sent_at.insert(seq, Instant::now());
        self.tcp
            .send_to(
                &syn,
                &SockAddr::from(SocketAddr::new(self.target_ip, self.dst_port)),
            )
            .map_err(|err| probe_io_failure(LABEL, err))?;
        Ok(())
    }

    fn recv(&mut self, deadline: Instant) -> Result<Option<HopReply>, ProbeFailure> {
        loop {
            if deadline.checked_duration_since(Instant::now()).is_none() {
                return Ok(None);
            }

            // Intermediate hops + terminal errors arrive on the raw ICMP socket.
            let mut icmp_buf = [MaybeUninit::<u8>::uninit(); 2048];
            if let Some((bytes, fallback)) = try_recv(&self.icmp, &mut icmp_buf)? {
                let received_at = Instant::now();
                if let Some((seq, from, kind)) = self.classify_icmp(bytes, fallback) {
                    if let Some(sent) = self.sent_at.get(&seq) {
                        let latency_ms =
                            received_at.saturating_duration_since(*sent).as_secs_f64() * 1000.0;
                        return Ok(Some(HopReply {
                            seq,
                            from,
                            latency_ms,
                            kind,
                        }));
                    }
                }
            }

            // The destination's SYN-ACK/RST arrives as a TCP segment from the
            // target; the hop is recovered from its acknowledgement number.
            let mut tcp_buf = [MaybeUninit::<u8>::uninit(); 2048];
            if let Some((bytes, reply_source)) = try_recv(&self.tcp, &mut tcp_buf)? {
                let received_at = Instant::now();
                if reply_source == self.target_ip {
                    if let Some(seq) =
                        tcp_reply_hop(bytes, self.target_ip, self.src_port, self.dst_port)
                    {
                        if let Some(sent) = self.sent_at.get(&seq) {
                            let latency_ms =
                                received_at.saturating_duration_since(*sent).as_secs_f64() * 1000.0;
                            return Ok(Some(HopReply {
                                seq,
                                from: self.target_ip,
                                latency_ms,
                                kind: HopKind::Reached,
                            }));
                        }
                    }
                }
            }

            let remaining = match deadline.checked_duration_since(Instant::now()) {
                Some(remaining) => remaining,
                None => return Ok(None),
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

/// If a TCP segment is the destination's reply to one of our SYNs — matching
/// ports, an ACK flag, and SYN-ACK or RST — recovers the hop from its
/// acknowledgement number (= our sequence + 1). The RST a closed port sends in
/// response to a SYN is also RST+ACK acking our sequence, so both arrivals
/// decode the same way.
fn tcp_reply_hop(
    bytes: &[u8],
    target_ip: IpAddr,
    our_src_port: u16,
    target_port: u16,
) -> Option<u16> {
    // IPv4 raw sockets prepend the outer IP header; IPv6 raw sockets do not.
    let tcp = match target_ip {
        IpAddr::V4(_) if bytes.len() >= 20 && bytes[0] >> 4 == 4 => {
            let header_len = usize::from(bytes[0] & 0x0f) * 4;
            if header_len < 20 || bytes.len() < header_len + 20 {
                return None;
            }
            &bytes[header_len..]
        }
        IpAddr::V4(_) => return None,
        IpAddr::V6(_) => bytes,
    };
    if tcp.len() < 20 {
        return None;
    }
    let source_port = u16::from_be_bytes([tcp[0], tcp[1]]);
    let dest_port = u16::from_be_bytes([tcp[2], tcp[3]]);
    if source_port != target_port || dest_port != our_src_port {
        return None;
    }
    let flags = tcp[13];
    let syn = flags & 0x02 != 0;
    let ack = flags & 0x10 != 0;
    let rst = flags & 0x04 != 0;
    if !ack || !(rst || syn) {
        return None;
    }
    let ack_num = u32::from_be_bytes([tcp[8], tcp[9], tcp[10], tcp[11]]);
    decode_tcp_seq(ack_num.wrapping_sub(1))
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

    fn udp_quote(src_port: u16, dst_port: u16) -> Vec<u8> {
        let mut u = vec![0u8; 8];
        u[0..2].copy_from_slice(&src_port.to_be_bytes());
        u[2..4].copy_from_slice(&dst_port.to_be_bytes());
        u
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
        assert_eq!(transport_checksum(local, target, 6, &syn), 0);
    }

    #[test]
    fn tcp_seq_round_trips_through_encode_decode() {
        for hop in [1u16, 2, 30, 64, 255] {
            assert_eq!(decode_tcp_seq(encode_tcp_seq(hop)), Some(hop));
        }
        // A sequence without our tag does not decode to a hop.
        assert_eq!(decode_tcp_seq(0x12345678), None);
    }

    #[test]
    fn recover_quoted_transport_extracts_udp_dst_and_rejects_mismatches() {
        let quoted = ipv4_packet(17, &udp_quote(50000, 33436));
        let target = v4(8, 8, 8, 8);
        assert_eq!(
            recover_quoted_transport(&quoted, target, 17, 50000),
            Some((33436, None))
        );
        // Wrong source port: not ours.
        assert_eq!(recover_quoted_transport(&quoted, target, 17, 9999), None);
        // Wrong protocol (TCP against a UDP quote): not ours.
        assert_eq!(recover_quoted_transport(&quoted, target, 6, 50000), None);
    }

    #[test]
    fn recover_quoted_transport_extracts_tcp_sequence() {
        let mut tcp = vec![0u8; 8];
        tcp[0..2].copy_from_slice(&40000u16.to_be_bytes());
        tcp[2..4].copy_from_slice(&80u16.to_be_bytes());
        tcp[4..8].copy_from_slice(&encode_tcp_seq(7).to_be_bytes());
        let quoted = ipv4_packet(6, &tcp);
        let target = v4(8, 8, 8, 8);
        let (dst, seq) = recover_quoted_transport(&quoted, target, 6, 40000).unwrap();
        assert_eq!(dst, 80);
        assert_eq!(decode_tcp_seq(seq.unwrap()), Some(7));
    }

    #[test]
    fn tcp_reply_hop_recovers_hop_from_syn_ack_and_rst() {
        let target = v4(93, 184, 216, 34);
        let our_src = 40000u16;
        let target_port = 80u16;
        let hop = 5u16;
        let ack = encode_tcp_seq(hop).wrapping_add(1);

        let mut synack = vec![0u8; 20];
        synack[0..2].copy_from_slice(&target_port.to_be_bytes());
        synack[2..4].copy_from_slice(&our_src.to_be_bytes());
        synack[8..12].copy_from_slice(&ack.to_be_bytes());
        synack[13] = 0x12; // SYN+ACK
        assert_eq!(
            tcp_reply_hop(&ipv4_packet(6, &synack), target, our_src, target_port),
            Some(hop)
        );

        // RST+ACK acking our sequence decodes the same hop.
        let mut rst = synack.clone();
        rst[13] = 0x14; // RST+ACK
        assert_eq!(
            tcp_reply_hop(&ipv4_packet(6, &rst), target, our_src, target_port),
            Some(hop)
        );

        // Wrong port pair, or no ACK flag, is rejected.
        let mut wrong_port = synack.clone();
        wrong_port[2..4].copy_from_slice(&12345u16.to_be_bytes());
        assert_eq!(
            tcp_reply_hop(&ipv4_packet(6, &wrong_port), target, our_src, target_port),
            None
        );
        let mut no_ack = synack.clone();
        no_ack[13] = 0x02; // SYN only
        assert_eq!(
            tcp_reply_hop(&ipv4_packet(6, &no_ack), target, our_src, target_port),
            None
        );
    }
}
