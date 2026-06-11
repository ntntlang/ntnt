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
//! to its hop by a per-hop port token: UDP varies the destination port, TCP
//! the source port (recovered from the quoted header and the target's reply).

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

/// The (source, destination) ports of the transport header the packet an ICMP
/// error quotes, if its protocol matches. The caller decides which port is the
/// per-hop token: UDP varies the destination port, TCP the source port.
fn recover_quoted_ports(quoted: &[u8], target_ip: IpAddr, protocol: u8) -> Option<(u16, u16)> {
    let (proto, transport) = match target_ip {
        IpAddr::V4(_) => quoted_inner_v4(quoted),
        IpAddr::V6(_) => quoted_inner_v6(quoted),
    }?;
    if proto != protocol || transport.len() < 4 {
        return None;
    }
    Some((
        u16::from_be_bytes([transport[0], transport[1]]),
        u16::from_be_bytes([transport[2], transport[3]]),
    ))
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
            let Some((quoted_src, quoted_dst)) =
                recover_quoted_ports(message.quoted, self.target_ip, 17)
            else {
                continue;
            };
            // UDP varies the destination port per hop; the source port is ours.
            if quoted_src != self.src_port {
                continue;
            }
            let Some(seq) = self.seq_for(quoted_dst) else {
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
    base_src_port: u16,
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
    // Base source port: each hop sends from base + (hop - 1) so the target's
    // reply (whose destination port echoes our source) identifies the hop,
    // robustly for SYN-ACK and every RST variant. The base is chosen with
    // headroom for max_hops (<=64) below 65535. The kernel has no socket bound
    // to these ports, so it RSTs the target's SYN-ACK itself, tearing down the
    // half-open connection we would otherwise leave.
    let base_src_port = 33000u16.wrapping_add(next_icmp_ident() % 20000);
    Ok(TcpTraceProbe {
        icmp,
        tcp,
        target_ip,
        local_ip,
        base_src_port,
        dst_port,
        sent_at: HashMap::new(),
    })
}

impl TcpTraceProbe {
    /// Per-hop source port (base + hop index) and its inverse.
    fn src_port_for(&self, seq: u16) -> u16 {
        self.base_src_port.saturating_add(seq.saturating_sub(1))
    }

    fn seq_for_src(&self, src_port: u16) -> Option<u16> {
        src_port.checked_sub(self.base_src_port)?.checked_add(1)
    }

    /// Classifies an ICMP datagram (intermediate hop or terminal error),
    /// recovering the hop from the quoted TCP source port. The quoted
    /// destination port must be our fixed target port.
    fn classify_icmp(&self, bytes: &[u8], fallback: IpAddr) -> Option<(u16, IpAddr, HopKind)> {
        let message = parse_icmp_message(bytes, self.target_ip, fallback)?;
        if !is_quoting_error(self.target_ip, message.icmp_type) {
            return None;
        }
        let (quoted_src, quoted_dst) = recover_quoted_ports(message.quoted, self.target_ip, 6)?;
        if quoted_dst != self.dst_port {
            return None;
        }
        let seq = self.seq_for_src(quoted_src)?;
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
        let src_port = self.src_port_for(seq);
        let syn = build_tcp_syn(
            self.local_ip,
            self.target_ip,
            src_port,
            self.dst_port,
            // The sequence number is not used for correlation (the source port
            // is), so a fixed value suffices.
            0x5354_5230,
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

            // The destination's SYN-ACK or RST arrives as a TCP segment from
            // the target; its destination port is the per-hop source port we
            // sent from, which recovers the hop for any reply flag combination.
            let mut tcp_buf = [MaybeUninit::<u8>::uninit(); 2048];
            if let Some((bytes, reply_source)) = try_recv(&self.tcp, &mut tcp_buf)? {
                let received_at = Instant::now();
                if reply_source == self.target_ip {
                    if let Some(our_src_port) =
                        tcp_reply_src_port(bytes, self.target_ip, self.dst_port)
                    {
                        if let Some(seq) = self.seq_for_src(our_src_port) {
                            if let Some(sent) = self.sent_at.get(&seq) {
                                let latency_ms =
                                    received_at.saturating_duration_since(*sent).as_secs_f64()
                                        * 1000.0;
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

/// If a TCP segment is the destination's reply to one of our SYNs — coming
/// from the target port and carrying SYN-ACK or RST — returns the segment's
/// destination port, which is the per-hop source port we sent from. Unlike an
/// acknowledgement-number scheme this accepts every RST variant (with or
/// without ACK), which firewalls and closed ports both produce.
fn tcp_reply_src_port(bytes: &[u8], target_ip: IpAddr, target_port: u16) -> Option<u16> {
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
    if source_port != target_port {
        return None;
    }
    let flags = tcp[13];
    let syn = flags & 0x02 != 0;
    let ack = flags & 0x10 != 0;
    let rst = flags & 0x04 != 0;
    // A SYN-ACK (open) or any RST (closed/filtered) means the target answered.
    if rst || (syn && ack) {
        Some(dest_port)
    } else {
        None
    }
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
    fn recover_quoted_ports_extracts_both_ports_and_rejects_protocol_mismatch() {
        let quoted = ipv4_packet(17, &udp_quote(50000, 33436));
        let target = v4(8, 8, 8, 8);
        assert_eq!(
            recover_quoted_ports(&quoted, target, 17),
            Some((50000, 33436))
        );
        // Wrong protocol (TCP against a UDP quote): not ours.
        assert_eq!(recover_quoted_ports(&quoted, target, 6), None);
    }

    #[test]
    fn tcp_reply_src_port_accepts_syn_ack_and_every_rst_variant() {
        let target = v4(93, 184, 216, 34);
        let our_src = 40005u16; // the per-hop source port we sent from
        let target_port = 80u16;

        let mut reply = vec![0u8; 20];
        reply[0..2].copy_from_slice(&target_port.to_be_bytes()); // from target port
        reply[2..4].copy_from_slice(&our_src.to_be_bytes()); // to our source port

        // SYN-ACK (open).
        reply[13] = 0x12;
        assert_eq!(
            tcp_reply_src_port(&ipv4_packet(6, &reply), target, target_port),
            Some(our_src)
        );
        // RST+ACK (closed port, RFC response).
        reply[13] = 0x14;
        assert_eq!(
            tcp_reply_src_port(&ipv4_packet(6, &reply), target, target_port),
            Some(our_src)
        );
        // Bare RST (some firewalls) is still accepted — the prior ack-number
        // scheme would have missed it.
        reply[13] = 0x04;
        assert_eq!(
            tcp_reply_src_port(&ipv4_packet(6, &reply), target, target_port),
            Some(our_src)
        );

        // A SYN with no ACK is not a destination reply.
        reply[13] = 0x02;
        assert_eq!(
            tcp_reply_src_port(&ipv4_packet(6, &reply), target, target_port),
            None
        );
        // A segment not from the target port is ignored.
        reply[13] = 0x12;
        reply[0..2].copy_from_slice(&12345u16.to_be_bytes());
        assert_eq!(
            tcp_reply_src_port(&ipv4_packet(6, &reply), target, target_port),
            None
        );
    }
}
