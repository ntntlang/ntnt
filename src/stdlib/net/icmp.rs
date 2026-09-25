//! Native ICMP probe substrate and the `ping()` driver.
//!
//! The substrate (socket creation with datagram-first/raw-fallback, echo
//! packet construction, checksums, reply/error parsing, capability
//! detection) is deliberately separated from the ping driver so future
//! probes — traceroute in particular — can reuse it: traceroute is the same
//! echo machinery with a stepped TTL and Time Exceeded treated as a hop
//! report instead of a failure.
//!
//! All fallible paths classify failures through
//! [`ProbeFailure`](super::probe::ProbeFailure): `Target` failures are valid
//! probe outcomes recorded per-attempt, `Backend` failures abort and surface
//! as `Err` to the caller. Classification happens once, at the io boundary —
//! never by sniffing composed message strings.

use super::probe::{
    internet_checksum, probe_attempt_budget, quoted_inner_v4, quoted_inner_v6,
    set_socket_hop_limit, transport_checksum, HopProbe, ProbeFailure, TraceProbe,
};
use super::{policy::enforce_resolved_target_policy, ProbeOptions};
use crate::interpreter::Value;
use crate::stdlib::send_deadline::SendDeadline;
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::collections::HashMap;
use std::io::ErrorKind;
use std::mem::MaybeUninit;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicU16, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const PING_LABEL: &str = "ICMP ping";

static ICMP_IDENT_COUNTER: AtomicU16 = AtomicU16::new(1);

// ---------------------------------------------------------------------------
// Capability detection
// ---------------------------------------------------------------------------

/// Which ICMP socket types the current process can create, per family.
pub(super) struct IcmpCapabilities {
    pub(super) v4_datagram: bool,
    pub(super) v4_raw: bool,
    pub(super) v6_datagram: bool,
    pub(super) v6_raw: bool,
}

impl IcmpCapabilities {
    pub(super) fn ping_available(&self) -> bool {
        self.v4_datagram || self.v4_raw || self.v6_datagram || self.v6_raw
    }

    /// ICMP-echo traceroute needs raw ICMP: only raw sockets deliver Time
    /// Exceeded packets with the reporting router's address.
    pub(super) fn traceroute_available(&self) -> bool {
        self.v4_raw || self.v6_raw
    }

    /// UDP traceroute also relies on the raw ICMP receive path, but its probe
    /// send + reply handling are validated only on Linux (Windows rejects the
    /// send with WSAEINVAL), so report it there.
    pub(super) fn traceroute_udp_available(&self) -> bool {
        cfg!(target_os = "linux") && self.traceroute_available()
    }

    /// TCP traceroute additionally needs a raw TCP socket, and raw TCP reply
    /// capture only works on Linux (BSD/Windows do not deliver TCP to raw
    /// sockets), so report it only where it can actually function.
    pub(super) fn traceroute_tcp_available(&self) -> bool {
        cfg!(target_os = "linux") && self.traceroute_available() && raw_tcp_socket_creatable()
    }
}

/// Whether a raw TCP socket can be created by this process (creation only —
/// no traffic). Used to keep the `traceroute_tcp` capability flag honest.
fn raw_tcp_socket_creatable() -> bool {
    let creatable = |domain| Socket::new(domain, Type::RAW, Some(Protocol::TCP)).is_ok();
    creatable(Domain::IPV4) || creatable(Domain::IPV6)
}

pub(super) fn detect_icmp_capabilities() -> IcmpCapabilities {
    IcmpCapabilities {
        v4_datagram: icmp_path_available(Domain::IPV4, Type::DGRAM, Protocol::ICMPV4),
        v4_raw: icmp_path_available(Domain::IPV4, Type::RAW, Protocol::ICMPV4),
        v6_datagram: icmp_path_available(Domain::IPV6, Type::DGRAM, Protocol::ICMPV6),
        v6_raw: icmp_path_available(Domain::IPV6, Type::RAW, Protocol::ICMPV6),
    }
}

const CAPABILITY_PROBE_TIMEOUT: Duration = Duration::from_millis(50);

/// Capability detection exercises the same socket setup path `ping()` uses —
/// create, set timeouts, connect — against loopback, so a reported capability
/// means an echo request could actually be built and sent. `connect()` on a
/// datagram/raw socket only sets the default destination locally; detection
/// never sends any traffic.
fn icmp_path_available(domain: Domain, socket_type: Type, protocol: Protocol) -> bool {
    let loopback = match domain {
        Domain::IPV6 => IpAddr::V6(Ipv6Addr::LOCALHOST),
        _ => IpAddr::V4(Ipv4Addr::LOCALHOST),
    };
    let Ok(socket) = open_connected_icmp_socket(
        domain,
        socket_type,
        protocol,
        loopback,
        CAPABILITY_PROBE_TIMEOUT,
    ) else {
        return false;
    };
    match loopback {
        // ICMPv6 echo construction needs a local IPv6 address for the
        // pseudo-header checksum; require it just like build_icmp_echo_request.
        IpAddr::V6(_) => socket
            .local_addr()
            .ok()
            .and_then(|addr| addr.as_socket())
            .map(|addr| addr.ip().is_ipv6())
            .unwrap_or(false),
        IpAddr::V4(_) => true,
    }
}

// ---------------------------------------------------------------------------
// Failure classification (io boundary only)
// ---------------------------------------------------------------------------

pub(super) fn probe_socket_unavailable(label: &str, err: std::io::Error) -> ProbeFailure {
    ProbeFailure::Backend(format!("{label} unavailable: native socket failed: {err}"))
}

pub(super) fn probe_io_failure(label: &str, err: std::io::Error) -> ProbeFailure {
    let message = format!("{label} failed: {err}");
    if io_error_indicates_target_failure(&err) {
        ProbeFailure::Target(message)
    } else {
        ProbeFailure::Backend(message)
    }
}

pub(super) fn io_error_indicates_target_failure(err: &std::io::Error) -> bool {
    // ECONNREFUSED is how Linux DGRAM ICMP sockets surface a pending ICMP
    // destination-unreachable error on send/recv.
    if err.kind() == ErrorKind::ConnectionRefused {
        return true;
    }
    let lower = err.to_string().to_ascii_lowercase();
    lower.contains("connection refused")
        || lower.contains("network is unreachable")
        || lower.contains("no route to host")
        || lower.contains("destination host unreachable")
        || lower.contains("destination net unreachable")
        || lower.contains("host is down")
}

// ---------------------------------------------------------------------------
// Socket substrate
// ---------------------------------------------------------------------------

pub(super) fn next_icmp_ident() -> u16 {
    let counter = ICMP_IDENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    (std::process::id() as u16) ^ counter.wrapping_mul(0x9e37)
}

fn create_icmp_socket(target_ip: IpAddr, timeout: Duration) -> std::io::Result<Socket> {
    let (domain, protocol) = match target_ip {
        IpAddr::V4(_) => (Domain::IPV4, Protocol::ICMPV4),
        IpAddr::V6(_) => (Domain::IPV6, Protocol::ICMPV6),
    };
    let dgram_error =
        match open_connected_icmp_socket(domain, Type::DGRAM, protocol, target_ip, timeout) {
            Ok(socket) => return Ok(socket),
            Err(err) => err,
        };
    if io_error_indicates_target_failure(&dgram_error) {
        return Err(dgram_error);
    }
    match open_connected_icmp_socket(domain, Type::RAW, protocol, target_ip, timeout) {
        Ok(socket) => Ok(socket),
        Err(raw_error) => {
            if dgram_error.kind() == ErrorKind::PermissionDenied {
                Err(dgram_error)
            } else {
                Err(raw_error)
            }
        }
    }
}

/// Opens an UNCONNECTED raw ICMP socket for the target's family. Traceroute
/// requires raw sockets (only they deliver Time Exceeded with the reporting
/// router's address) and must NOT connect: a connected raw socket would make
/// the kernel drop replies from intermediate routers, which are exactly the
/// hops traceroute needs. The caller sends with `ProbeDelivery::Unconnected`.
/// Usually needs CAP_NET_RAW.
pub(super) fn create_raw_icmp_socket(
    target_ip: IpAddr,
    timeout: Duration,
) -> std::io::Result<Socket> {
    let (domain, protocol) = match target_ip {
        IpAddr::V4(_) => (Domain::IPV4, Protocol::ICMPV4),
        IpAddr::V6(_) => (Domain::IPV6, Protocol::ICMPV6),
    };
    let socket = Socket::new(domain, Type::RAW, Some(protocol))?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    Ok(socket)
}

fn open_connected_icmp_socket(
    domain: Domain,
    socket_type: Type,
    protocol: Protocol,
    target_ip: IpAddr,
    timeout: Duration,
) -> std::io::Result<Socket> {
    let socket = Socket::new(domain, socket_type, Some(protocol))?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    socket.connect(&SockAddr::from(SocketAddr::new(target_ip, 0)))?;
    Ok(socket)
}

/// Determines the local source address the kernel would use to reach
/// `target_ip`, by connecting a throwaway UDP socket. A UDP `connect()` sends
/// no packets — it only makes the kernel select a route and source address,
/// which we then read back. Ping does not need this (its ICMP socket is
/// connected, so its own `local_addr()` is populated), but traceroute's raw
/// socket is intentionally unconnected and would otherwise report the
/// unspecified address, producing a wrong ICMPv6 pseudo-header checksum.
pub(super) fn local_source_address_for(target_ip: IpAddr) -> Option<IpAddr> {
    let domain = match target_ip {
        IpAddr::V4(_) => Domain::IPV4,
        IpAddr::V6(_) => Domain::IPV6,
    };
    let socket = Socket::new(domain, Type::DGRAM, Some(Protocol::UDP)).ok()?;
    // Port is arbitrary and unused; UDP connect does not transmit.
    socket
        .connect(&SockAddr::from(SocketAddr::new(target_ip, 53)))
        .ok()?;
    let ip = socket
        .local_addr()
        .ok()?
        .as_socket()
        .map(|addr| addr.ip())?;
    if ip.is_unspecified() {
        None
    } else {
        Some(ip)
    }
}

fn icmp_ident_for_socket(socket: &Socket) -> Option<u16> {
    socket
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_socket())
        .map(|addr| addr.port())
        .filter(|port| *port != 0)
}

/// Shared connected echo socket setup for finite ping batches and persistent probes.
pub(super) struct EchoSocket {
    pub(super) socket: Socket,
    pub(super) target: IpAddr,
    local: Option<IpAddr>,
    ident: u16,
}
impl EchoSocket {
    #[cfg(test)]
    pub(super) fn test_udp(socket: std::net::UdpSocket) -> Self {
        Self {
            socket: socket.into(),
            target: IpAddr::V4(Ipv4Addr::LOCALHOST),
            local: None,
            ident: 0x1234,
        }
    }

    pub(super) fn open(target: IpAddr, timeout: Duration) -> std::io::Result<Self> {
        #[cfg(test)]
        if let Some(socket) = tests::TEST_ECHO_SOCKET.with(|slot| slot.borrow_mut().take()) {
            return Ok(socket);
        }
        let socket = create_icmp_socket(target, timeout)?;
        let local = socket
            .local_addr()
            .ok()
            .and_then(|a| a.as_socket())
            .map(|a| a.ip());
        let ident = icmp_ident_for_socket(&socket).unwrap_or_else(next_icmp_ident);
        Ok(Self {
            socket,
            target,
            local,
            ident,
        })
    }
    pub(super) fn send_persistent(
        &self,
        sequence: u16,
        payload: &[u8],
    ) -> Result<(), ProbeFailure> {
        self.socket
            .set_nonblocking(true)
            .map_err(|e| probe_socket_unavailable(PING_LABEL, e))?;
        let packet = build_icmp_echo_request(
            PING_LABEL,
            self.target,
            self.local,
            self.ident,
            sequence,
            payload,
        )?;
        // A local send failure (including EAGAIN/ENETUNREACH) is never a
        // target timeout. No send retries or synthetic sent counts.
        let sent = self
            .socket
            .send(&packet)
            .map_err(|e| probe_socket_unavailable(PING_LABEL, e))?;
        if sent != packet.len() {
            return Err(ProbeFailure::Backend(
                "incomplete ICMP datagram send".into(),
            ));
        }
        Ok(())
    }
    pub(super) fn receive_persistent(
        &self,
        sequence: u16,
        payload: &[u8],
        sent_at: Instant,
        wait: Duration,
    ) -> Result<Option<IcmpProbeEvent>, ProbeFailure> {
        // Bounded blocking receive wakes on arrival rather than inflating RTT
        // with sleep polling. The owner protects the sole descriptor throughout.
        // socket2 truncates timeouts to OS units (microseconds on Unix,
        // milliseconds on Windows). A tiny positive remainder must not become
        // zero, which disables the timeout. The caller still checks its absolute
        // deadline and cancellation after this bounded receive.
        self.socket
            .set_read_timeout(Some(wait.max(Duration::from_millis(1))))
            .and_then(|_| self.socket.set_nonblocking(false))
            .map_err(|e| probe_socket_unavailable(PING_LABEL, e))?;
        let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
        let (len, from) = match self.socket.recv_from(&mut buffer) {
            Ok(v) => v,
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                return Ok(None)
            }
            // Kernel errors lack a quoted generation. Treat them as backend
            // failures, never attribute them to the current target measurement.
            Err(e) => return Err(probe_socket_unavailable(PING_LABEL, e)),
        };
        // socket2 initialized exactly len bytes.
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), len) };
        let source = from.as_socket().map(|a| a.ip()).unwrap_or(self.target);
        if !persistent_packet_matches(bytes, source, self.target, payload) {
            return Ok(None);
        }
        Ok(parse_icmp_probe_event(
            bytes,
            source,
            self.target,
            self.ident,
            sequence,
            sent_at.elapsed(),
        ))
    }
}

/// Full nonce + logical counter required even before first wire rollover.
/// RFC-minimum error quotes with only the echo header are deliberately ignored.
fn persistent_packet_matches(bytes: &[u8], source: IpAddr, target: IpAddr, payload: &[u8]) -> bool {
    let Some(message) = parse_icmp_message(bytes, target, source) else {
        return false;
    };
    let reply_type = if target.is_ipv4() { 0 } else { 129 };
    if message.icmp_type == reply_type {
        return message.source == target && message.icmp_code == 0 && message.quoted == payload;
    }
    let quoted = message.quoted;
    let inner = match target {
        IpAddr::V4(ip) => {
            if quoted.get(16..20) != Some(ip.octets().as_slice()) {
                return false;
            }
            quoted_inner_v4(quoted).filter(|(proto, _)| *proto == 1)
        }
        IpAddr::V6(ip) => {
            if quoted.get(24..40) != Some(ip.octets().as_slice()) {
                return false;
            }
            quoted_inner_v6(quoted).filter(|(proto, _)| *proto == 58)
        }
    };
    inner.is_some_and(|(_, inner)| inner.get(8..8 + payload.len()) == Some(payload))
}

// ---------------------------------------------------------------------------
// Packet substrate
// ---------------------------------------------------------------------------

fn build_icmp_echo_request(
    label: &str,
    target_ip: IpAddr,
    local_ip: Option<IpAddr>,
    ident: u16,
    sequence: u16,
    payload: &[u8],
) -> Result<Vec<u8>, ProbeFailure> {
    let request_type = match target_ip {
        IpAddr::V4(_) => 8,
        IpAddr::V6(_) => 128,
    };
    let mut packet = Vec::with_capacity(8 + payload.len());
    packet.extend_from_slice(&[request_type, 0, 0, 0]);
    packet.extend_from_slice(&ident.to_be_bytes());
    packet.extend_from_slice(&sequence.to_be_bytes());
    packet.extend_from_slice(payload);

    let checksum = match (target_ip, local_ip) {
        (IpAddr::V4(_), _) => internet_checksum(&packet),
        // ICMPv6 uses protocol 58 in its pseudo-header checksum.
        (IpAddr::V6(_), Some(src @ IpAddr::V6(_))) => {
            transport_checksum(src, target_ip, 58, &packet)
        }
        (IpAddr::V6(_), _) => {
            return Err(ProbeFailure::Backend(format!(
                "{label} failed: could not determine local IPv6 address for checksum"
            )));
        }
    };
    packet[2..4].copy_from_slice(&checksum.to_be_bytes());
    Ok(packet)
}

// ---------------------------------------------------------------------------
// Reply parsing substrate
// ---------------------------------------------------------------------------

/// An ICMP message lifted out of a raw-socket datagram: the outer IPv4 header
/// (if present) is stripped, leaving the responder's address, the ICMP
/// type/code, and the bytes the ICMP error quotes from the original packet.
/// Shared by every probe method that listens on the raw ICMP socket.
pub(super) struct IcmpMessage<'a> {
    pub(super) source: IpAddr,
    pub(super) ttl: Option<i64>,
    pub(super) icmp_type: u8,
    pub(super) icmp_code: u8,
    /// The 4 bytes after type/code/checksum — ident+sequence for an echo reply.
    pub(super) rest: [u8; 4],
    /// Bytes after the 8-byte ICMP header — for an error, the quoted packet.
    pub(super) quoted: &'a [u8],
}

/// Parses a datagram read from a raw ICMP socket. IPv4 raw sockets prepend the
/// outer IP header (giving the responder address and TTL); IPv6 raw sockets do
/// not, so the responder address comes from `recvfrom` via `fallback_source`.
pub(super) fn parse_icmp_message(
    bytes: &[u8],
    target_ip: IpAddr,
    fallback_source: IpAddr,
) -> Option<IcmpMessage<'_>> {
    let (icmp, ttl, source) = match target_ip {
        IpAddr::V4(_) if bytes.len() >= 20 && bytes[0] >> 4 == 4 => {
            let header_len = usize::from(bytes[0] & 0x0f) * 4;
            if header_len < 20 || bytes.len() < header_len + 8 {
                return None;
            }
            let source = IpAddr::V4(Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]));
            (&bytes[header_len..], Some(i64::from(bytes[8])), source)
        }
        _ => (bytes, None, fallback_source),
    };
    if icmp.len() < 8 {
        return None;
    }
    Some(IcmpMessage {
        source,
        ttl,
        icmp_type: icmp[0],
        icmp_code: icmp[1],
        rest: [icmp[4], icmp[5], icmp[6], icmp[7]],
        quoted: &icmp[8..],
    })
}

#[derive(Debug, Clone)]
pub(super) struct IcmpProbeReply {
    pub(super) source: IpAddr,
    pub(super) ttl: Option<i64>,
    pub(super) sequence: u16,
    pub(super) latency_ms: f64,
}

/// An ICMP error that quotes our probe (destination unreachable, time
/// exceeded, ...). Carried structurally so each driver can interpret it:
/// ping records every variant as a failed attempt, while traceroute treats
/// Time Exceeded as a hop report.
#[derive(Debug, Clone)]
pub(super) struct IcmpProbeError {
    pub(super) source: IpAddr,
    pub(super) icmp_type: u8,
    pub(super) icmp_code: u8,
    pub(super) latency_ms: f64,
}

impl IcmpProbeError {
    pub(super) fn message(&self) -> String {
        format!(
            "ICMP error from {}: type {} code {}",
            self.source, self.icmp_type, self.icmp_code
        )
    }

    /// Time Exceeded is ICMPv4 type 11 / ICMPv6 type 3 — the signal a
    /// TTL-stepped probe uses to identify an intermediate hop.
    pub(super) fn is_time_exceeded(&self) -> bool {
        match self.source {
            IpAddr::V4(_) => self.icmp_type == 11,
            IpAddr::V6(_) => self.icmp_type == 3,
        }
    }
}

#[derive(Debug)]
pub(super) enum IcmpProbeEvent {
    Reply(IcmpProbeReply),
    Error(IcmpProbeError),
}

fn parse_icmp_probe_event(
    bytes: &[u8],
    fallback_source: IpAddr,
    target_ip: IpAddr,
    ident: u16,
    sequence: u16,
    elapsed: Duration,
) -> Option<IcmpProbeEvent> {
    let message = parse_icmp_message(bytes, target_ip, fallback_source)?;
    let (reply_type, failure_types): (u8, &[u8]) = match target_ip {
        IpAddr::V4(_) => (0, &[3, 11]),
        IpAddr::V6(_) => (129, &[1, 2, 3, 4]),
    };
    let latency_ms = elapsed.as_secs_f64() * 1000.0;

    if message.icmp_type == reply_type
        && u16::from_be_bytes([message.rest[0], message.rest[1]]) == ident
        && u16::from_be_bytes([message.rest[2], message.rest[3]]) == sequence
    {
        return Some(IcmpProbeEvent::Reply(IcmpProbeReply {
            source: message.source,
            ttl: message.ttl,
            sequence,
            latency_ms,
        }));
    }

    if failure_types.contains(&message.icmp_type)
        && echo_error_quotes_probe(message.quoted, target_ip, ident, sequence)
    {
        return Some(IcmpProbeEvent::Error(IcmpProbeError {
            source: message.source,
            icmp_type: message.icmp_type,
            icmp_code: message.icmp_code,
            latency_ms,
        }));
    }
    None
}

/// True when the packet an ICMP error quotes is one of our echo requests.
fn echo_error_quotes_probe(quoted: &[u8], target_ip: IpAddr, ident: u16, sequence: u16) -> bool {
    let (icmp_protocol, echo_type, inner) = match target_ip {
        IpAddr::V4(_) => match quoted_inner_v4(quoted) {
            Some((proto, inner)) => (1u8, 8u8, (proto, inner)),
            None => return false,
        },
        IpAddr::V6(_) => match quoted_inner_v6(quoted) {
            Some((proto, inner)) => (58u8, 128u8, (proto, inner)),
            None => return false,
        },
    };
    let (proto, inner) = inner;
    proto == icmp_protocol
        && inner.len() >= 8
        && inner[0] == echo_type
        && u16::from_be_bytes([inner[4], inner[5]]) == ident
        && u16::from_be_bytes([inner[6], inner[7]]) == sequence
}

// ---------------------------------------------------------------------------
// Probe send/receive
// ---------------------------------------------------------------------------

/// How an echo probe reaches the target.
///
/// Ping connects its socket and uses `send()`. Traceroute must NOT connect:
/// a connected raw ICMP socket makes the kernel drop datagrams whose source
/// is not the connected peer, which is exactly the intermediate-router Time
/// Exceeded replies traceroute depends on. It uses `send_to` on an
/// unconnected socket so replies from any hop are received.
pub(super) enum ProbeDelivery {
    /// Socket is connected to the target; send with `send()`.
    Connected,
    /// Socket is unconnected; send to the target with `send_to()`.
    Unconnected,
}

/// Sends one ICMP echo request and waits for the matching event: a reply,
/// an ICMP error quoting the probe, or `None` on timeout. Shared by ping
/// and traceroute; `label` names the calling probe in failure messages.
#[allow(clippy::too_many_arguments)]
pub(super) fn send_echo_probe(
    label: &str,
    socket: &Socket,
    target_ip: IpAddr,
    local_ip: Option<IpAddr>,
    ident: u16,
    sequence: u16,
    payload: &[u8],
    timeout: Duration,
    delivery: ProbeDelivery,
) -> Result<Option<IcmpProbeEvent>, ProbeFailure> {
    send_echo_probe_guarded(
        label, socket, target_ip, local_ip, ident, sequence, payload, timeout, delivery, &mut None,
    )
}

#[allow(clippy::too_many_arguments)]
fn send_echo_probe_guarded(
    label: &str,
    socket: &Socket,
    target_ip: IpAddr,
    local_ip: Option<IpAddr>,
    ident: u16,
    sequence: u16,
    payload: &[u8],
    timeout: Duration,
    delivery: ProbeDelivery,
    start_deadline: &mut Option<&SendDeadline>,
) -> Result<Option<IcmpProbeEvent>, ProbeFailure> {
    let deadline = Instant::now() + timeout;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|err| probe_socket_unavailable(label, err))?;
    socket
        .set_write_timeout(Some(timeout))
        .map_err(|err| probe_socket_unavailable(label, err))?;
    let packet = build_icmp_echo_request(label, target_ip, local_ip, ident, sequence, payload)?;
    #[cfg(test)]
    tests::delay_before_send();
    let sent_at = Instant::now();
    // Userspace authorization adjacent to the OS send, after packet/socket setup.
    // Preemption can still occur before the syscall: this is not a NIC wire-time guarantee.
    if let Some(guard) = start_deadline.as_ref() {
        guard.check().map_err(ProbeFailure::Backend)?;
    }
    // An attempted send commits this finite sample, even if the syscall fails.
    // Requested later packets keep their original global timeout, not this start guard.
    *start_deadline = None;
    let send_result = match delivery {
        ProbeDelivery::Connected => socket.send(&packet),
        ProbeDelivery::Unconnected => {
            socket.send_to(&packet, &SockAddr::from(SocketAddr::new(target_ip, 0)))
        }
    };
    send_result.map_err(|err| probe_io_failure(label, err))?;
    wait_for_icmp_event(label, socket, target_ip, ident, sequence, deadline, sent_at)
}

#[allow(clippy::too_many_arguments)]
fn wait_for_icmp_event(
    label: &str,
    socket: &Socket,
    target_ip: IpAddr,
    ident: u16,
    sequence: u16,
    deadline: Instant,
    sent_at: Instant,
) -> Result<Option<IcmpProbeEvent>, ProbeFailure> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        socket
            .set_read_timeout(Some(remaining))
            .map_err(|err| probe_socket_unavailable(label, err))?;
        let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
        let (len, from) = match socket.recv_from(&mut buffer) {
            Ok(received) => received,
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                return Ok(None);
            }
            Err(err) => return Err(probe_io_failure(label, err)),
        };
        let received_elapsed = sent_at.elapsed();
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), len) };
        let source = from.as_socket().map(|addr| addr.ip()).unwrap_or(target_ip);
        match parse_icmp_probe_event(bytes, source, target_ip, ident, sequence, received_elapsed) {
            Some(event) => return Ok(Some(event)),
            None => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Traceroute: shared raw-ICMP receive socket + ICMP echo probe method
// ---------------------------------------------------------------------------

/// Maps a raw-ICMP-socket creation error to traceroute's user-facing failure,
/// turning a permission error into the CAP_NET_RAW guidance every method
/// shares (all traceroute methods need the raw ICMP receive path for hops).
pub(super) fn raw_icmp_socket_denied(err: std::io::Error) -> ProbeFailure {
    if err.kind() == ErrorKind::PermissionDenied {
        ProbeFailure::Backend(format!(
            "traceroute unavailable: raw ICMP socket denied: {err}. \
             Grant CAP_NET_RAW (Docker: cap_add: [NET_RAW]) or run with elevated privileges"
        ))
    } else {
        probe_socket_unavailable("traceroute", err)
    }
}

/// Opens the unconnected raw ICMP socket every traceroute method listens on
/// for Time Exceeded (intermediate hops) and ICMP errors from the target.
pub(super) fn open_raw_icmp_recv(
    target_ip: IpAddr,
    timeout: Duration,
) -> Result<Socket, ProbeFailure> {
    create_raw_icmp_socket(target_ip, timeout).map_err(raw_icmp_socket_denied)
}

/// ICMP echo traceroute method: send + receive both happen on one raw ICMP
/// socket, so this simply wraps the shared echo probe.
pub(super) struct IcmpTraceProbe {
    socket: Socket,
    target_ip: IpAddr,
    local_ip: Option<IpAddr>,
    ident: u16,
}

pub(super) fn open_icmp_trace_probe(
    target_ip: IpAddr,
    timeout: Duration,
) -> Result<IcmpTraceProbe, ProbeFailure> {
    let socket = open_raw_icmp_recv(target_ip, timeout)?;
    let local_ip = local_source_address_for(target_ip);
    let ident = next_icmp_ident();
    Ok(IcmpTraceProbe {
        socket,
        target_ip,
        local_ip,
        ident,
    })
}

impl TraceProbe for IcmpTraceProbe {
    fn probe(&mut self, ttl: u8, seq: u16, deadline: Instant) -> Result<HopProbe, ProbeFailure> {
        set_socket_hop_limit("traceroute", &self.socket, self.target_ip, ttl)?;
        let Some(timeout) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(HopProbe::TimedOut);
        };
        let payload = [0u8; 32];
        let event = send_echo_probe(
            "traceroute",
            &self.socket,
            self.target_ip,
            self.local_ip,
            self.ident,
            seq,
            &payload,
            timeout,
            ProbeDelivery::Unconnected,
        )?;
        Ok(match event {
            Some(IcmpProbeEvent::Reply(reply)) => HopProbe::Reached {
                from: reply.source,
                latency_ms: reply.latency_ms,
            },
            Some(IcmpProbeEvent::Error(error)) if error.is_time_exceeded() => HopProbe::Hop {
                from: error.source,
                latency_ms: error.latency_ms,
            },
            Some(IcmpProbeEvent::Error(error)) => HopProbe::Terminal {
                from: error.source,
                latency_ms: error.latency_ms,
                message: error.message(),
            },
            None => HopProbe::TimedOut,
        })
    }
}

// ---------------------------------------------------------------------------
// Ping driver
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct IcmpPingAttempt {
    seq: Option<i64>,
    reachable: bool,
    from: Option<String>,
    ttl: Option<i64>,
    latency_ms: Option<f64>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct IcmpPingResult {
    host: String,
    target_addr: Option<String>,
    sent: usize,
    received: usize,
    loss_percent: f64,
    min_ms: Option<f64>,
    avg_ms: Option<f64>,
    max_ms: Option<f64>,
    attempts: Vec<IcmpPingAttempt>,
}

pub(super) fn icmp_ping_for_host(
    host: &str,
    options: &ProbeOptions,
) -> Result<HashMap<String, Value>, String> {
    icmp_ping_for_host_guarded(host, options, None)
}

pub(super) fn icmp_ping_for_host_guarded(
    host: &str,
    options: &ProbeOptions,
    start_deadline: Option<&SendDeadline>,
) -> Result<HashMap<String, Value>, String> {
    let targets = resolve_probe_targets(host)?;
    enforce_resolved_target_policy(&targets, options.allow_private)?;
    let target_ips = unique_target_ips(&targets)?;
    let result = icmp_ping(host, &target_ips, options, start_deadline)?;
    Ok(icmp_ping_result_map(result))
}

pub(super) fn resolve_probe_targets(host: &str) -> Result<Vec<(u16, SocketAddr)>, String> {
    let resolved: Vec<SocketAddr> = (host, 0)
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve {}: {}", host, e))?
        .collect();
    Ok(resolved.into_iter().map(|addr| (0, addr)).collect())
}

pub(super) fn unique_target_ips(targets: &[(u16, SocketAddr)]) -> Result<Vec<IpAddr>, String> {
    let mut ips = Vec::new();
    for (_, addr) in targets {
        let ip = addr.ip();
        if !ips.contains(&ip) {
            ips.push(ip);
        }
    }
    if ips.is_empty() {
        return Err("failed to resolve target: resolver returned no usable addresses".to_string());
    }
    Ok(ips)
}

fn icmp_ping(
    display_host: &str,
    target_ips: &[IpAddr],
    options: &ProbeOptions,
    mut start_deadline: Option<&SendDeadline>,
) -> Result<IcmpPingResult, String> {
    let deadline = Instant::now() + options.timeout;
    let mut results = Vec::new();
    let mut first_fatal_error = None;

    for target_ip in target_ips {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        if remaining.is_zero() {
            break;
        }
        let result = run_native_ping(
            display_host,
            *target_ip,
            remaining,
            options.count,
            options.interval,
            &mut start_deadline,
        );
        let result = match result {
            Ok(result) => result,
            Err(failure) if failure.is_target() => failed_ping_result(
                display_host,
                *target_ip,
                failure.into_message(),
                options.count,
            ),
            Err(ProbeFailure::Backend(error)) if error == "start_deadline_expired" => {
                return Err(error);
            }
            Err(failure) => {
                first_fatal_error.get_or_insert(failure.into_message());
                continue;
            }
        };
        let reachable = result.received > 0;
        results.push(result);
        if reachable {
            break;
        }
    }

    if results.is_empty() {
        return Err(first_fatal_error.unwrap_or_else(|| {
            format!("ICMP ping unavailable: no probe could be sent for {display_host}")
        }));
    }

    Ok(combine_icmp_ping_attempts(display_host, results))
}

fn run_native_ping(
    display_host: &str,
    target_ip: IpAddr,
    timeout: Duration,
    count: usize,
    interval: Duration,
    start_deadline: &mut Option<&SendDeadline>,
) -> Result<IcmpPingResult, ProbeFailure> {
    let core = match EchoSocket::open(target_ip, timeout) {
        Ok(socket) => socket,
        Err(err) if io_error_indicates_target_failure(&err) => {
            return Ok(failed_ping_result(
                display_host,
                target_ip,
                format!("ICMP ping failed: {}", err),
                count,
            ));
        }
        Err(err) => return Err(probe_socket_unavailable(PING_LABEL, err)),
    };
    let socket = core.socket;
    let local_ip = core.local;
    let ident = core.ident;
    let count = count.max(1);
    let deadline = Instant::now() + timeout;
    let payload = [0u8; 56];
    let mut attempts = Vec::new();
    let mut latencies = Vec::new();

    for seq in 1..=count {
        let sequence = (seq.min(u16::MAX as usize)) as u16;
        let remaining_attempts = count.saturating_sub(seq).saturating_add(1);
        let budget = match deadline.checked_duration_since(Instant::now()) {
            Some(remaining) => {
                probe_attempt_budget(PING_LABEL, remaining, remaining_attempts, interval)
            }
            None => Err(ProbeFailure::Backend(format!(
                "ICMP ping timed out before completing requested count: sent {} of {}",
                attempts.len(),
                count
            ))),
        };
        let per_attempt_timeout = match budget {
            Ok(value) => value,
            // A budget failure before the first probe means timeout_ms cannot
            // fit the requested count/interval at all: surface it as an error.
            Err(failure) if attempts.is_empty() => return Err(failure),
            // Mid-sequence exhaustion keeps the probes already completed and
            // records the unsent remainder as failed attempts so the caller
            // still sees the full requested count.
            Err(_) => {
                push_unsent_ping_attempts(
                    &mut attempts,
                    target_ip,
                    seq,
                    count,
                    "ICMP ping deadline exhausted before probe could be sent",
                );
                break;
            }
        };
        match send_echo_probe_guarded(
            PING_LABEL,
            &socket,
            target_ip,
            local_ip,
            ident,
            sequence,
            &payload,
            per_attempt_timeout,
            // Ping's socket (datagram or raw) is connected to the target.
            ProbeDelivery::Connected,
            start_deadline,
        ) {
            Ok(Some(IcmpProbeEvent::Reply(reply))) => {
                latencies.push(reply.latency_ms);
                attempts.push(IcmpPingAttempt {
                    seq: Some(reply.sequence as i64),
                    reachable: true,
                    from: Some(reply.source.to_string()),
                    ttl: reply.ttl,
                    latency_ms: Some(reply.latency_ms),
                    error: None,
                });
            }
            // Any ICMP error quoting our probe is a target failure for ping.
            Ok(Some(IcmpProbeEvent::Error(error))) => attempts.push(IcmpPingAttempt {
                seq: Some(sequence as i64),
                reachable: false,
                from: Some(target_ip.to_string()),
                ttl: None,
                latency_ms: None,
                error: Some(error.message()),
            }),
            Ok(None) => attempts.push(IcmpPingAttempt {
                seq: Some(sequence as i64),
                reachable: false,
                from: Some(target_ip.to_string()),
                ttl: None,
                latency_ms: None,
                error: Some("request timed out".to_string()),
            }),
            Err(failure) if failure.is_target() => {
                attempts.push(IcmpPingAttempt {
                    seq: Some(sequence as i64),
                    reachable: false,
                    from: Some(target_ip.to_string()),
                    ttl: None,
                    latency_ms: None,
                    error: Some(failure.into_message()),
                });
            }
            Err(failure) => return Err(failure),
        }

        if seq < count && !interval.is_zero() {
            let interval_fits = deadline
                .checked_duration_since(Instant::now())
                .is_some_and(|remaining| remaining >= interval);
            if !interval_fits {
                push_unsent_ping_attempts(
                    &mut attempts,
                    target_ip,
                    seq + 1,
                    count,
                    "ICMP ping deadline exhausted before probe could be sent",
                );
                break;
            }
            thread::sleep(interval);
        }
    }

    Ok(finish_icmp_ping_result(
        display_host,
        target_ip,
        attempts,
        latencies,
    ))
}

// ---------------------------------------------------------------------------
// Result assembly
// ---------------------------------------------------------------------------

fn push_unsent_ping_attempts(
    attempts: &mut Vec<IcmpPingAttempt>,
    target_ip: IpAddr,
    from_seq: usize,
    count: usize,
    message: &str,
) {
    for seq in from_seq..=count {
        attempts.push(IcmpPingAttempt {
            seq: Some(seq as i64),
            reachable: false,
            from: Some(target_ip.to_string()),
            ttl: None,
            latency_ms: None,
            error: Some(message.to_string()),
        });
    }
}

fn failed_ping_result(
    host: &str,
    target_ip: IpAddr,
    message: String,
    count: usize,
) -> IcmpPingResult {
    let count = count.max(1);
    let attempts = (1..=count)
        .map(|seq| IcmpPingAttempt {
            seq: Some(seq as i64),
            reachable: false,
            from: Some(target_ip.to_string()),
            ttl: None,
            latency_ms: None,
            error: Some(message.clone()),
        })
        .collect();
    IcmpPingResult {
        host: host.to_string(),
        target_addr: Some(target_ip.to_string()),
        sent: count,
        received: 0,
        loss_percent: 100.0,
        min_ms: None,
        avg_ms: None,
        max_ms: None,
        attempts,
    }
}

fn finish_icmp_ping_result(
    host: &str,
    target_ip: IpAddr,
    attempts: Vec<IcmpPingAttempt>,
    latencies: Vec<f64>,
) -> IcmpPingResult {
    let sent = attempts.len();
    let received = attempts.iter().filter(|attempt| attempt.reachable).count();
    let loss_percent = if sent == 0 {
        100.0
    } else {
        ((sent.saturating_sub(received)) as f64 / sent as f64) * 100.0
    };
    let min_ms = latencies.iter().copied().reduce(f64::min);
    let max_ms = latencies.iter().copied().reduce(f64::max);
    let avg_ms = if latencies.is_empty() {
        None
    } else {
        Some(latencies.iter().sum::<f64>() / latencies.len() as f64)
    };

    IcmpPingResult {
        host: host.to_string(),
        target_addr: Some(target_ip.to_string()),
        sent,
        received,
        loss_percent,
        min_ms,
        avg_ms,
        max_ms,
        attempts,
    }
}

fn combine_icmp_ping_attempts(host: &str, results: Vec<IcmpPingResult>) -> IcmpPingResult {
    let mut sent = 0usize;
    let mut received = 0usize;
    let mut target_addr = results
        .iter()
        .find(|result| result.received > 0)
        .and_then(|result| result.target_addr.clone())
        .or_else(|| {
            results
                .first()
                .and_then(|result| result.target_addr.clone())
        });
    let mut attempts = Vec::new();
    for mut result in results {
        sent = sent.saturating_add(result.sent);
        received = received.saturating_add(result.received);
        attempts.append(&mut result.attempts);
        if target_addr.is_none() {
            target_addr = result.target_addr;
        }
    }

    let mut min_ms: Option<f64> = None;
    let mut max_ms: Option<f64> = None;
    let mut latency_total = 0.0;
    let mut latency_count = 0usize;
    for latency_ms in attempts.iter().filter_map(|attempt| attempt.latency_ms) {
        min_ms = Some(min_ms.map_or(latency_ms, |current| current.min(latency_ms)));
        max_ms = Some(max_ms.map_or(latency_ms, |current| current.max(latency_ms)));
        latency_total += latency_ms;
        latency_count += 1;
    }
    let avg_ms = (latency_count > 0).then(|| latency_total / latency_count as f64);
    let loss_percent = if sent == 0 {
        100.0
    } else {
        ((sent.saturating_sub(received)) as f64 / sent as f64) * 100.0
    };

    IcmpPingResult {
        host: host.to_string(),
        target_addr,
        sent,
        received,
        loss_percent,
        min_ms,
        avg_ms,
        max_ms,
        attempts,
    }
}

fn icmp_ping_result_map(result: IcmpPingResult) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert("host".to_string(), Value::String(result.host));
    map.insert("reachable".to_string(), Value::Bool(result.received > 0));
    map.insert("method".to_string(), Value::String("icmp".to_string()));
    map.insert("permission_limited".to_string(), Value::Bool(false));
    map.insert("sent".to_string(), Value::Int(result.sent as i64));
    map.insert("received".to_string(), Value::Int(result.received as i64));
    map.insert(
        "failed".to_string(),
        Value::Int(result.sent.saturating_sub(result.received) as i64),
    );
    map.insert(
        "loss_percent".to_string(),
        Value::Float(result.loss_percent),
    );
    map.insert(
        "target_addr".to_string(),
        result.target_addr.map_or_else(Value::none, Value::String),
    );
    map.insert(
        "attempts".to_string(),
        Value::Array(result.attempts.iter().map(icmp_attempt_to_value).collect()),
    );
    if let Some(min_ms) = result.min_ms {
        map.insert("min_ms".to_string(), Value::Float(min_ms));
    }
    if let Some(avg_ms) = result.avg_ms {
        map.insert("avg_ms".to_string(), Value::Float(avg_ms));
        if result.sent == 1 {
            map.insert("latency_ms".to_string(), Value::Float(avg_ms));
        }
    }
    if let Some(max_ms) = result.max_ms {
        map.insert("max_ms".to_string(), Value::Float(max_ms));
    }
    map
}

fn icmp_attempt_to_value(attempt: &IcmpPingAttempt) -> Value {
    let mut map = HashMap::new();
    map.insert("reachable".to_string(), Value::Bool(attempt.reachable));
    map.insert("method".to_string(), Value::String("icmp".to_string()));
    if let Some(seq) = attempt.seq {
        map.insert("seq".to_string(), Value::Int(seq));
    }
    if let Some(from) = &attempt.from {
        map.insert("from".to_string(), Value::String(from.clone()));
    }
    if let Some(ttl) = attempt.ttl {
        map.insert("ttl".to_string(), Value::Int(ttl));
    }
    if let Some(latency_ms) = attempt.latency_ms {
        map.insert("latency_ms".to_string(), Value::Float(latency_ms));
    }
    if let Some(error) = &attempt.error {
        map.insert("error".to_string(), Value::String(error.clone()));
    }
    Value::Map(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        pub(super) static TEST_ECHO_SOCKET: std::cell::RefCell<Option<EchoSocket>> = const { std::cell::RefCell::new(None) };
        static SETUP_EXPIRY: std::cell::RefCell<Option<SendDeadline>> = const { std::cell::RefCell::new(None) };
    }

    pub(super) fn delay_before_send() {
        if let Some(guard) = SETUP_EXPIRY.with(|slot| slot.borrow_mut().take()) {
            // Controlled last-setup barrier: proceed only once this exact guard expires.
            while guard.check().is_ok() {
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn wall_deadline_after(ms: i64) -> SendDeadline {
        SendDeadline::parse(Some(&HashMap::from([(
            "start_deadline_ms".into(),
            Value::Int(chrono::Utc::now().timestamp_millis() + ms),
        )])))
        .unwrap()
    }

    fn udp_pair() -> (Socket, std::net::UdpSocket) {
        let peer = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        peer.set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let client = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        client.connect(peer.local_addr().unwrap()).unwrap();
        (client.into(), peer)
    }

    #[test]
    fn ping_start_deadline_delayed_setup_sends_zero_datagrams() {
        let (socket, peer) = udp_pair();
        let guard = wall_deadline_after(1000);
        guard.check().unwrap();
        SETUP_EXPIRY.with(|slot| *slot.borrow_mut() = Some(guard.clone()));
        let mut pending = Some(&guard);
        let result = send_echo_probe_guarded(
            PING_LABEL,
            &socket,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            None,
            7,
            1,
            b"fixture",
            Duration::from_millis(300),
            ProbeDelivery::Connected,
            &mut pending,
        );
        let mut packet = [0u8; 128];
        let received = peer.recv_from(&mut packet);
        assert!(
            matches!(received, Err(ref e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut)),
            "unexpected datagram: {received:?}"
        );
        assert_eq!(result.unwrap_err().into_message(), "start_deadline_expired");
        assert!(
            pending.is_some(),
            "no attempted send must leave start uncommitted"
        );
    }

    #[test]
    fn ping_start_deadline_monotonic_expiry_stops_driver_before_send() {
        let (socket, peer) = udp_pair();
        TEST_ECHO_SOCKET.with(|slot| {
            *slot.borrow_mut() = Some(EchoSocket {
                socket,
                target: IpAddr::V4(Ipv4Addr::LOCALHOST),
                local: None,
                ident: 7,
            })
        });
        let guard = SendDeadline::parse(Some(&HashMap::from([
            (
                "start_monotonic_deadline_ms".into(),
                Value::Int(crate::stdlib::time::monotonic_millis() + 1000),
            ),
            ("start_deadline_ms".into(), Value::Int(i64::MAX)),
        ])))
        .unwrap();
        guard.check().unwrap();
        SETUP_EXPIRY.with(|slot| *slot.borrow_mut() = Some(guard.clone()));
        let options = ProbeOptions {
            timeout: Duration::from_secs(1),
            count: 2,
            interval: Duration::ZERO,
            allow_private: true,
        };
        let result = icmp_ping(
            "loopback fixture",
            &[IpAddr::V4(Ipv4Addr::LOCALHOST)],
            &options,
            Some(&guard),
        );
        assert_eq!(result.unwrap_err(), "start_deadline_expired");
        let mut packet = [0u8; 128];
        assert!(
            matches!(peer.recv_from(&mut packet), Err(ref e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
        );
    }

    #[test]
    fn ping_start_deadline_in_budget_burst_completes_after_expiry() {
        let (socket, peer) = udp_pair();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        TEST_ECHO_SOCKET.with(|slot| {
            *slot.borrow_mut() = Some(EchoSocket {
                socket,
                target: IpAddr::V4(Ipv4Addr::LOCALHOST),
                local: None,
                ident: 7,
            })
        });
        let guard = wall_deadline_after(1000);
        let reply_guard = guard.clone();
        let fixture = thread::spawn(move || {
            for sequence in 1..=2 {
                let mut packet = [0u8; 128];
                let (len, from) = peer.recv_from(&mut packet).unwrap();
                assert_eq!(u16::from_be_bytes([packet[6], packet[7]]), sequence);
                if sequence == 1 {
                    while reply_guard.check().is_ok() {
                        thread::sleep(Duration::from_millis(1));
                    }
                }
                packet[0] = 0;
                peer.send_to(&packet[..len], from).unwrap();
            }
        });
        let mut pending = Some(&guard);
        let result = run_native_ping(
            "loopback fixture",
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Duration::from_secs(4),
            2,
            Duration::from_millis(10),
            &mut pending,
        )
        .unwrap();
        fixture.join().unwrap();
        assert_eq!(guard.check(), Err("start_deadline_expired".into()));
        assert_eq!(result.sent, 2);
        assert_eq!(result.received, 2);
        assert!(pending.is_none());
    }

    #[test]
    fn ping_start_deadline_real_icmp_loopback_optional() {
        let target = IpAddr::V4(Ipv4Addr::LOCALHOST);
        let core = match EchoSocket::open(target, Duration::from_secs(4)) {
            Ok(core) => core,
            Err(error) if error.kind() == ErrorKind::PermissionDenied => {
                eprintln!("SKIP real first-send ICMP fixture: {error}; UDP-backed boundary assertions run separately");
                return;
            }
            Err(error) => panic!("native loopback ICMP setup failed: {error}"),
        };
        // This descriptor is a real native ICMP socket, not the UDP fixture.
        TEST_ECHO_SOCKET.with(|slot| *slot.borrow_mut() = Some(core));
        let guard = wall_deadline_after(1000);
        let mut pending = Some(&guard);
        let result = run_native_ping(
            "127.0.0.1",
            target,
            Duration::from_secs(4),
            2,
            Duration::from_millis(1200),
            &mut pending,
        )
        .unwrap();
        assert_eq!(result.sent, 2);
        assert_eq!(result.received, 2);
        assert!(pending.is_none());
        assert_eq!(guard.check(), Err("start_deadline_expired".into()));
    }

    #[test]
    fn ping_start_deadline_does_not_extend_global_timeout() {
        let (socket, peer) = udp_pair();
        TEST_ECHO_SOCKET.with(|slot| {
            *slot.borrow_mut() = Some(EchoSocket {
                socket,
                target: IpAddr::V4(Ipv4Addr::LOCALHOST),
                local: None,
                ident: 7,
            })
        });
        let guard = wall_deadline_after(1000);
        let mut pending = Some(&guard);
        let started = Instant::now();
        let result = run_native_ping(
            "silent loopback fixture",
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            Duration::from_millis(150),
            1,
            Duration::ZERO,
            &mut pending,
        )
        .unwrap();
        assert!(started.elapsed() < Duration::from_millis(750));
        assert_eq!(result.received, 0);
        assert_eq!(result.sent, 1);
        assert!(pending.is_none());
        for _ in 0..1 {
            let mut packet = [0u8; 128];
            assert!(peer.recv_from(&mut packet).is_ok());
        }
        let mut packet = [0u8; 128];
        assert!(
            matches!(peer.recv_from(&mut packet), Err(ref e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
        );
    }

    #[test]
    fn ping_start_deadline_malformed_fails_closed() {
        for key in ["start_deadline_ms", "start_monotonic_deadline_ms"] {
            let result = super::super::ping_fn(&[
                Value::String("127.0.0.1".into()),
                Value::Map(HashMap::from([
                    ("allow_private".into(), Value::Bool(true)),
                    ("count".into(), Value::Int(1)),
                    (key.into(), Value::String("bad".into())),
                ])),
            ])
            .unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, ref values, .. }
            if variant == "Err" && matches!(values.first(), Some(Value::String(s)) if s.contains(key))),
                "{result:?}"
            );
        }
    }

    #[test]
    fn persistent_receive_tiny_wait_never_disables_socket_timeout() {
        // Blocking UDP exercises the same socket2 receive path without ICMP
        // privileges. Existing nonblocking UDP fixtures cannot catch this bug.
        for wait in [
            Duration::from_nanos(1),
            Duration::from_nanos(500),
            Duration::from_nanos(999),
            Duration::from_micros(1),
            Duration::from_millis(1),
        ] {
            let udp = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
            let address = udp.local_addr().unwrap();
            let echo = EchoSocket::test_udp(udp);
            // Rescue an accidentally infinite receive so the red test is bounded,
            // but cancel the pending rescue as soon as the receive returns.
            let (cancel, cancelled) = std::sync::mpsc::channel();
            let rescue = std::thread::spawn(move || {
                if matches!(
                    cancelled.recv_timeout(Duration::from_millis(150)),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    let sender = std::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
                    sender
                        .send_to(b"timeout regression rescue", address)
                        .unwrap();
                }
            });
            let result = echo.receive_persistent(1, b"expected payload", Instant::now(), wait);
            // The receiver is already gone if the rescue fired; that is harmless.
            let _ = cancel.send(());
            rescue.join().unwrap();
            let installed = echo.socket.read_timeout().unwrap();
            assert!(result.unwrap().is_none());
            assert!(
                installed.is_some_and(|timeout| !timeout.is_zero()),
                "positive wait {wait:?} disabled the OS receive timeout: {installed:?}"
            );
        }
    }

    #[test]
    fn real_persistent_echo_datagram_and_raw_paths() {
        let mut exercised = 0;
        for target in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let (domain, protocol) = if target.is_ipv4() {
                (Domain::IPV4, Protocol::ICMPV4)
            } else {
                (Domain::IPV6, Protocol::ICMPV6)
            };
            for kind in [Type::DGRAM, Type::RAW] {
                let socket = match open_connected_icmp_socket(
                    domain,
                    kind,
                    protocol,
                    target,
                    Duration::from_secs(1),
                ) {
                    Ok(socket) => socket,
                    Err(e) => {
                        eprintln!("SKIP real ICMP {target} {kind:?}: {e}; no probes asserted for this path");
                        continue;
                    }
                };
                let local = socket.local_addr().unwrap().as_socket().map(|a| a.ip());
                let ident = icmp_ident_for_socket(&socket).unwrap_or_else(next_icmp_ident);
                socket.set_nonblocking(true).unwrap();
                let core = EchoSocket {
                    socket,
                    target,
                    local,
                    ident,
                };
                let nonce: [u8; 16] = rand::random();
                for logical in [65_535u64, 65_536] {
                    let mut payload = nonce.to_vec();
                    payload.extend_from_slice(&logical.to_be_bytes());
                    let start = Instant::now();
                    core.send_persistent(logical as u16, &payload).unwrap();
                    loop {
                        assert!(
                            start.elapsed() < Duration::from_secs(1),
                            "real ICMP {target} {kind:?} did not reply"
                        );
                        match core
                            .receive_persistent(
                                logical as u16,
                                &payload,
                                start,
                                Duration::from_millis(5),
                            )
                            .unwrap()
                        {
                            Some(IcmpProbeEvent::Reply(r)) => {
                                assert_eq!(r.source, target);
                                break;
                            }
                            Some(event) => panic!("unexpected real loopback event {event:?}"),
                            None => std::thread::sleep(Duration::from_millis(5)),
                        }
                    }
                }
                exercised += 1;
                eprintln!("REAL ICMP {target} {kind:?}: same socket received two correlated replies across forced wire rollover");
            }
        }
        if std::env::var("NTNT_ICMP_REQUIRE").as_deref() == Ok("1") {
            assert!(exercised > 0, "required real ICMP, but all paths skipped");
        }
    }

    #[test]
    fn persistent_correlation_rejects_stale_malformed_and_truncated_v4_v6() {
        let payload = [7u8; 24];
        for target in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ] {
            let (reply_type, echo_type, error_type) = if target.is_ipv4() {
                (0, 8, 3)
            } else {
                (129, 128, 1)
            };
            let mut reply = vec![reply_type, 0, 0, 0, 0x12, 0x34, 0, 0];
            reply.extend_from_slice(&payload);
            assert!(persistent_packet_matches(&reply, target, target, &payload));
            assert!(
                parse_icmp_probe_event(&reply, target, target, 0x1234, 0, Duration::ZERO).is_some()
            );
            assert!(
                parse_icmp_probe_event(&reply, target, target, 0x1235, 0, Duration::ZERO).is_none()
            );
            assert!(
                parse_icmp_probe_event(&reply, target, target, 0x1234, 1, Duration::ZERO).is_none()
            );
            let mut stale = payload;
            stale[23] ^= 1;
            assert!(!persistent_packet_matches(&reply, target, target, &stale));
            for n in 0..reply.len() {
                assert!(!persistent_packet_matches(
                    &reply[..n],
                    target,
                    target,
                    &payload
                ));
            }
            let other = if target.is_ipv4() {
                "127.0.0.2".parse().unwrap()
            } else {
                "::2".parse().unwrap()
            };
            assert!(!persistent_packet_matches(&reply, other, target, &payload));
            let mut quoted = if let IpAddr::V4(ip) = target {
                let mut q = vec![0; 20];
                q[0] = 0x45;
                q[9] = 1;
                q[16..20].copy_from_slice(&ip.octets());
                q
            } else if let IpAddr::V6(ip) = target {
                let mut q = vec![0; 40];
                q[0] = 0x60;
                q[6] = 58;
                q[24..40].copy_from_slice(&ip.octets());
                q
            } else {
                unreachable!()
            };
            reply[0] = echo_type;
            quoted.extend_from_slice(&reply);
            let mut error = vec![error_type, 0, 0, 0, 0, 0, 0, 0];
            error.extend_from_slice(&quoted);
            assert!(persistent_packet_matches(&error, other, target, &payload));
            assert!(matches!(
                parse_icmp_probe_event(&error, other, target, 0x1234, 0, Duration::ZERO),
                Some(IcmpProbeEvent::Error(_))
            ));
            assert!(!persistent_packet_matches(&error, other, target, &stale));
            for n in 0..error.len() {
                assert!(!persistent_packet_matches(
                    &error[..n],
                    other,
                    target,
                    &payload
                ));
            }
            // A different quoted destination with matching identifier/sequence/payload cannot be ours.
            error[if target.is_ipv4() { 24 } else { 32 }] ^= 1;
            assert!(!persistent_packet_matches(&error, other, target, &payload));
        }
    }

    #[test]
    fn icmp_echo_request_sets_checksum_and_identifiers() {
        let packet = build_icmp_echo_request(
            PING_LABEL,
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            None,
            0x1234,
            7,
            &[1, 2, 3, 4],
        )
        .unwrap();

        assert_eq!(packet[0], 8);
        assert_eq!(u16::from_be_bytes([packet[4], packet[5]]), 0x1234);
        assert_eq!(u16::from_be_bytes([packet[6], packet[7]]), 7);
        assert_eq!(internet_checksum(&packet), 0);
    }

    #[test]
    fn icmpv6_echo_request_requires_local_address_for_checksum() {
        let failure =
            build_icmp_echo_request(PING_LABEL, IpAddr::V6(Ipv6Addr::LOCALHOST), None, 1, 1, &[])
                .unwrap_err();
        assert!(!failure.is_target());
        assert!(failure.into_message().contains("local IPv6 address"));
    }

    #[test]
    fn parses_icmpv6_error_types_that_quote_probe() {
        let ident: u16 = 0x5555;
        let sequence: u16 = 9;
        let mut quoted = vec![0u8; 40];
        quoted[0] = 0x60;
        quoted[6] = 58;
        quoted.extend_from_slice(&[128, 0, 0, 0]);
        quoted.extend_from_slice(&ident.to_be_bytes());
        quoted.extend_from_slice(&sequence.to_be_bytes());

        for icmp_type in [1, 2, 3, 4] {
            let mut packet = vec![icmp_type, 0, 0, 0, 0, 0, 0, 0];
            packet.extend_from_slice(&quoted);
            let event = parse_icmp_probe_event(
                &packet,
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                IpAddr::V6(Ipv6Addr::LOCALHOST),
                ident,
                sequence,
                Duration::from_millis(12),
            );

            match event {
                Some(IcmpProbeEvent::Error(error)) => {
                    assert_eq!(error.icmp_type, icmp_type);
                    assert_eq!(error.source, IpAddr::V6(Ipv6Addr::LOCALHOST));
                    assert!(error.message().contains(&format!("type {icmp_type}")));
                    // ICMPv6 Time Exceeded is type 3 — the traceroute hop signal.
                    assert_eq!(error.is_time_exceeded(), icmp_type == 3);
                }
                other => panic!("expected ICMPv6 type {icmp_type} probe error, got {other:?}"),
            }
        }
    }

    #[test]
    fn icmp_ping_errors_when_no_probe_can_be_sent() {
        let options = ProbeOptions {
            timeout: Duration::from_millis(50),
            count: 1,
            interval: Duration::ZERO,
            allow_private: false,
        };

        let err = icmp_ping("example.com", &[], &options, None).unwrap_err();
        assert!(err.contains("no probe could be sent"));
    }

    #[test]
    fn parses_icmpv4_reply_with_ip_header() {
        let ident: u16 = 0x4444;
        let sequence: u16 = 3;
        let mut packet = vec![0u8; 20];
        packet[0] = 0x45;
        packet[8] = 61;
        packet[12..16].copy_from_slice(&[198, 51, 100, 9]);
        packet.extend_from_slice(&[0, 0, 0, 0]);
        packet.extend_from_slice(&ident.to_be_bytes());
        packet.extend_from_slice(&sequence.to_be_bytes());

        let event = parse_icmp_probe_event(
            &packet,
            IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)),
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 10)),
            ident,
            sequence,
            Duration::from_millis(12),
        );

        match event {
            Some(IcmpProbeEvent::Reply(reply)) => {
                assert_eq!(reply.source, IpAddr::V4(Ipv4Addr::new(198, 51, 100, 9)));
                assert_eq!(reply.ttl, Some(61));
                assert_eq!(reply.sequence, sequence);
            }
            other => panic!("expected matching ICMPv4 reply, got {other:?}"),
        }
    }

    #[test]
    fn unique_target_ips_preserves_resolver_order_without_duplicates() {
        let public_v6 = "[2001:4860:4860::8888]:0".parse::<SocketAddr>().unwrap();
        let public_v4 = "93.184.216.34:0".parse::<SocketAddr>().unwrap();
        let targets = [(0, public_v6), (0, public_v4), (0, public_v6)];

        assert_eq!(
            unique_target_ips(&targets).unwrap(),
            vec![public_v6.ip(), public_v4.ip()]
        );
    }

    #[test]
    fn failed_ping_result_preserves_requested_count() {
        let result = failed_ping_result(
            "example.com",
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            "ICMP ping failed: Network is unreachable".to_string(),
            3,
        );

        assert_eq!(result.sent, 3);
        assert_eq!(result.received, 0);
        assert_eq!(result.attempts.len(), 3);
        assert_eq!(result.attempts[2].seq, Some(3));
    }

    #[test]
    fn icmp_connection_refused_is_target_failure() {
        // Linux DGRAM ICMP sockets surface a pending destination-unreachable
        // error on recv as ECONNREFUSED; it must classify as a target failure.
        let io_err = std::io::Error::from(ErrorKind::ConnectionRefused);
        assert!(io_error_indicates_target_failure(&io_err));
        assert!(probe_io_failure(PING_LABEL, io_err).is_target());
    }

    #[test]
    fn icmp_permission_denied_is_backend_failure() {
        let io_err = std::io::Error::from(ErrorKind::PermissionDenied);
        assert!(!io_error_indicates_target_failure(&io_err));
        let failure = probe_io_failure(PING_LABEL, io_err);
        assert!(!failure.is_target());
        assert!(failure.into_message().starts_with("ICMP ping failed:"));
    }

    #[test]
    fn push_unsent_ping_attempts_fills_remaining_count() {
        let target_ip = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1));
        let mut attempts = vec![IcmpPingAttempt {
            seq: Some(1),
            reachable: true,
            from: Some(target_ip.to_string()),
            ttl: Some(56),
            latency_ms: Some(10.0),
            error: None,
        }];

        push_unsent_ping_attempts(&mut attempts, target_ip, 2, 4, "deadline exhausted");

        assert_eq!(attempts.len(), 4);
        assert_eq!(attempts[1].seq, Some(2));
        assert_eq!(attempts[3].seq, Some(4));
        assert!(attempts[1..]
            .iter()
            .all(|a| !a.reachable && a.error.as_deref() == Some("deadline exhausted")));

        let result = finish_icmp_ping_result("example.com", target_ip, attempts, vec![10.0]);
        assert_eq!(result.sent, 4);
        assert_eq!(result.received, 1);
    }

    #[test]
    fn target_ping_failure_can_be_aggregated_with_later_success() {
        let failed = failed_ping_result(
            "example.com",
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            "ICMP ping failed: Network is unreachable".to_string(),
            1,
        );
        let success = finish_icmp_ping_result(
            "example.com",
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
            vec![IcmpPingAttempt {
                seq: Some(1),
                reachable: true,
                from: Some("93.184.216.34".to_string()),
                ttl: Some(56),
                latency_ms: Some(10.0),
                error: None,
            }],
            vec![10.0],
        );

        let combined = combine_icmp_ping_attempts("example.com", vec![failed, success]);
        assert_eq!(combined.received, 1);
        assert_eq!(combined.target_addr.as_deref(), Some("93.184.216.34"));
        assert_eq!(combined.attempts.len(), 2);
    }

    #[test]
    fn resolve_probe_targets_accepts_ipv6_literals() {
        let targets = resolve_probe_targets("::1").unwrap();
        assert!(targets.iter().any(|(_, addr)| addr.ip().is_ipv6()));
    }

    #[test]
    fn capability_detection_is_consistent_and_does_not_panic() {
        let caps = detect_icmp_capabilities();
        assert_eq!(
            caps.ping_available(),
            caps.v4_datagram || caps.v4_raw || caps.v6_datagram || caps.v6_raw
        );
    }

    #[test]
    fn capability_detection_matches_ping_socket_setup_path() {
        // Whatever this environment permits, the reported IPv4 capability must
        // agree with what create_icmp_socket (the path ping() actually takes)
        // can do against loopback — capabilities must not overstate ping.
        let caps = detect_icmp_capabilities();
        let socket_path_works =
            create_icmp_socket(IpAddr::V4(Ipv4Addr::LOCALHOST), Duration::from_millis(50)).is_ok();
        assert_eq!(caps.v4_datagram || caps.v4_raw, socket_path_works);
    }

    #[test]
    fn local_source_address_for_returns_concrete_non_unspecified_address() {
        // For loopback the kernel selects loopback as the source; the helper
        // must never hand back the unspecified address, which would corrupt
        // the ICMPv6 checksum for traceroute's unconnected socket.
        let v4 = local_source_address_for(IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert!(matches!(v4, Some(ip) if ip.is_ipv4() && !ip.is_unspecified()));

        // IPv6 loopback may be unavailable in some sandboxes; when present the
        // source must be a concrete IPv6 address suitable for the checksum.
        if let Some(ip) = local_source_address_for(IpAddr::V6(Ipv6Addr::LOCALHOST)) {
            assert!(ip.is_ipv6() && !ip.is_unspecified());
        }
    }
}
