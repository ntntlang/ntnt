//! TTL-stepped traceroute driver on the shared ICMP substrate.
//!
//! Each hop sends one echo request with an incremented TTL/hop-limit and
//! interprets the resulting [`IcmpProbeEvent`]: a Time Exceeded error names
//! the router at that hop, an echo reply means the destination was reached,
//! and any other ICMP error (destination unreachable, administratively
//! prohibited, ...) terminates the trace with that hop recorded.
//!
//! Traceroute requires a RAW ICMP socket: only raw sockets deliver Time
//! Exceeded packets with the reporting router's source address. Datagram
//! ICMP sockets surface those errors as bare errno values without the
//! offender, which would produce a hop list with no addresses. When raw
//! sockets are unavailable the driver returns a clear backend error; apps
//! can check `net_capabilities().traceroute` first.

use super::enforce_resolved_target_policy;
use super::icmp::{
    create_raw_icmp_socket, next_icmp_ident, probe_socket_unavailable, resolve_probe_targets,
    send_echo_probe, unique_target_ips, IcmpProbeEvent, ProbeDelivery,
};
use super::probe::{probe_attempt_budget, ProbeFailure};
use crate::interpreter::Value;
use socket2::Socket;
use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::time::{Duration, Instant};

const TRACEROUTE_LABEL: &str = "traceroute";

pub(super) struct TracerouteOptions {
    pub(super) timeout: Duration,
    pub(super) max_hops: usize,
    pub(super) allow_private: bool,
}

#[derive(Debug, Clone)]
struct TracerouteHop {
    hop: usize,
    from: Option<String>,
    latency_ms: Option<f64>,
    reached: bool,
    timed_out: bool,
    error: Option<String>,
}

#[derive(Debug)]
struct TraceOutcome {
    hops: Vec<TracerouteHop>,
    reached: bool,
}

pub(super) fn traceroute_for_host(
    host: &str,
    options: &TracerouteOptions,
) -> Result<HashMap<String, Value>, String> {
    let targets = resolve_probe_targets(host)?;
    enforce_resolved_target_policy(&targets, options.allow_private)?;
    let target_ips = unique_target_ips(&targets)?;
    // Traceroute traces a single path: use the first resolved address, in
    // resolver preference order.
    let target_ip = target_ips[0];
    let outcome =
        run_traceroute(target_ip, options).map_err(super::probe::ProbeFailure::into_message)?;
    Ok(traceroute_result_map(host, target_ip, options, outcome))
}

fn run_traceroute(
    target_ip: IpAddr,
    options: &TracerouteOptions,
) -> Result<TraceOutcome, ProbeFailure> {
    let socket = create_raw_icmp_socket(target_ip, options.timeout).map_err(|err| {
        if err.kind() == ErrorKind::PermissionDenied {
            ProbeFailure::Backend(format!(
                "traceroute unavailable: raw ICMP socket denied: {err}. \
                 Grant CAP_NET_RAW (Docker: cap_add: [NET_RAW]) or run with elevated privileges"
            ))
        } else {
            probe_socket_unavailable(TRACEROUTE_LABEL, err)
        }
    })?;
    let local_ip = socket
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_socket())
        .map(|addr| addr.ip());
    let ident = next_icmp_ident();
    let deadline = Instant::now() + options.timeout;
    let payload = [0u8; 32];
    let mut hops: Vec<TracerouteHop> = Vec::new();
    let mut reached = false;

    for hop in 1..=options.max_hops {
        let remaining_hops = options.max_hops.saturating_sub(hop).saturating_add(1);
        let budget = match deadline.checked_duration_since(Instant::now()) {
            Some(remaining) => {
                probe_attempt_budget(TRACEROUTE_LABEL, remaining, remaining_hops, Duration::ZERO)
            }
            None => Err(ProbeFailure::Backend(format!(
                "traceroute timed out after {} of {} hops",
                hops.len(),
                options.max_hops
            ))),
        };
        let per_hop_timeout = match budget {
            Ok(value) => value,
            // No budget before the first hop means timeout_ms cannot fit the
            // requested max_hops at all: surface it as an error.
            Err(failure) if hops.is_empty() => return Err(failure),
            // Mid-trace exhaustion keeps the hops already discovered; the
            // result reports reached=false with the partial path.
            Err(_) => break,
        };
        // A setsockopt failure mid-trace must not discard the path already
        // discovered: before the first hop it is a hard error, otherwise it
        // ends the trace with the partial result (mirrors the budget arms).
        if let Err(failure) = set_hop_limit(&socket, target_ip, hop) {
            if hops.is_empty() {
                return Err(failure);
            }
            break;
        }
        let sequence = hop.min(u16::MAX as usize) as u16;
        match send_echo_probe(
            TRACEROUTE_LABEL,
            &socket,
            target_ip,
            local_ip,
            ident,
            sequence,
            &payload,
            per_hop_timeout,
            // Unconnected raw socket: receive Time Exceeded from every hop.
            ProbeDelivery::Unconnected,
        ) {
            Ok(Some(IcmpProbeEvent::Reply(reply))) => {
                hops.push(TracerouteHop {
                    hop,
                    from: Some(reply.source.to_string()),
                    latency_ms: Some(reply.latency_ms),
                    reached: true,
                    timed_out: false,
                    error: None,
                });
                reached = true;
                break;
            }
            Ok(Some(IcmpProbeEvent::Error(error))) if error.is_time_exceeded() => {
                // TTL expired in transit: this is the router at the current hop.
                hops.push(TracerouteHop {
                    hop,
                    from: Some(error.source.to_string()),
                    latency_ms: Some(error.latency_ms),
                    reached: false,
                    timed_out: false,
                    error: None,
                });
            }
            Ok(Some(IcmpProbeEvent::Error(error))) => {
                // Destination unreachable and friends terminate the trace.
                hops.push(TracerouteHop {
                    hop,
                    from: Some(error.source.to_string()),
                    latency_ms: Some(error.latency_ms),
                    reached: false,
                    timed_out: false,
                    error: Some(error.message()),
                });
                break;
            }
            Ok(None) => hops.push(TracerouteHop {
                hop,
                from: None,
                latency_ms: None,
                reached: false,
                timed_out: true,
                error: None,
            }),
            Err(failure) if failure.is_target() => {
                hops.push(TracerouteHop {
                    hop,
                    from: None,
                    latency_ms: None,
                    reached: false,
                    timed_out: false,
                    error: Some(failure.into_message()),
                });
                break;
            }
            Err(failure) if hops.is_empty() => return Err(failure),
            // Mid-trace backend failure: keep the hops already discovered and
            // record the failure on the hop where it happened.
            Err(failure) => {
                hops.push(TracerouteHop {
                    hop,
                    from: None,
                    latency_ms: None,
                    reached: false,
                    timed_out: false,
                    error: Some(failure.into_message()),
                });
                break;
            }
        }
    }

    Ok(TraceOutcome { hops, reached })
}

fn set_hop_limit(socket: &Socket, target_ip: IpAddr, hop: usize) -> Result<(), ProbeFailure> {
    let limit = hop.min(255) as u32;
    match target_ip {
        IpAddr::V4(_) => socket.set_ttl_v4(limit),
        IpAddr::V6(_) => socket.set_unicast_hops_v6(limit),
    }
    .map_err(|err| probe_socket_unavailable(TRACEROUTE_LABEL, err))
}

fn traceroute_result_map(
    host: &str,
    target_ip: IpAddr,
    options: &TracerouteOptions,
    outcome: TraceOutcome,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    map.insert("host".to_string(), Value::String(host.to_string()));
    map.insert(
        "target_addr".to_string(),
        Value::String(target_ip.to_string()),
    );
    map.insert("method".to_string(), Value::String("icmp".to_string()));
    map.insert("reached".to_string(), Value::Bool(outcome.reached));
    map.insert("max_hops".to_string(), Value::Int(options.max_hops as i64));
    map.insert(
        "hop_count".to_string(),
        Value::Int(outcome.hops.len() as i64),
    );
    map.insert(
        "hops".to_string(),
        Value::Array(outcome.hops.iter().map(hop_to_value).collect()),
    );
    map
}

fn hop_to_value(hop: &TracerouteHop) -> Value {
    let mut map = HashMap::new();
    map.insert("hop".to_string(), Value::Int(hop.hop as i64));
    map.insert("reached".to_string(), Value::Bool(hop.reached));
    map.insert("timed_out".to_string(), Value::Bool(hop.timed_out));
    if let Some(from) = &hop.from {
        map.insert("from".to_string(), Value::String(from.clone()));
    }
    if let Some(latency_ms) = hop.latency_ms {
        map.insert("latency_ms".to_string(), Value::Float(latency_ms));
    }
    if let Some(error) = &hop.error {
        map.insert("error".to_string(), Value::String(error.clone()));
    }
    Value::Map(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn sample_outcome() -> TraceOutcome {
        TraceOutcome {
            hops: vec![
                TracerouteHop {
                    hop: 1,
                    from: Some("192.0.2.1".to_string()),
                    latency_ms: Some(1.2),
                    reached: false,
                    timed_out: false,
                    error: None,
                },
                TracerouteHop {
                    hop: 2,
                    from: None,
                    latency_ms: None,
                    reached: false,
                    timed_out: true,
                    error: None,
                },
                TracerouteHop {
                    hop: 3,
                    from: Some("93.184.216.34".to_string()),
                    latency_ms: Some(10.5),
                    reached: true,
                    timed_out: false,
                    error: None,
                },
            ],
            reached: true,
        }
    }

    #[test]
    fn traceroute_result_map_reports_path_and_reachability() {
        let options = TracerouteOptions {
            timeout: Duration::from_millis(1000),
            max_hops: 30,
            allow_private: false,
        };
        let map = traceroute_result_map(
            "example.com",
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
            &options,
            sample_outcome(),
        );

        assert!(matches!(map.get("reached"), Some(Value::Bool(true))));
        assert!(matches!(map.get("hop_count"), Some(Value::Int(3))));
        assert!(matches!(map.get("max_hops"), Some(Value::Int(30))));
        assert!(matches!(map.get("method"), Some(Value::String(m)) if m == "icmp"));
        let Some(Value::Array(hops)) = map.get("hops") else {
            panic!("expected hops array");
        };
        assert_eq!(hops.len(), 3);
        let Value::Map(first) = &hops[0] else {
            panic!("expected hop map");
        };
        assert!(matches!(first.get("from"), Some(Value::String(f)) if f == "192.0.2.1"));
        assert!(matches!(first.get("reached"), Some(Value::Bool(false))));
        let Value::Map(second) = &hops[1] else {
            panic!("expected hop map");
        };
        assert!(matches!(second.get("timed_out"), Some(Value::Bool(true))));
        assert!(!second.contains_key("from"));
        let Value::Map(last) = &hops[2] else {
            panic!("expected hop map");
        };
        assert!(matches!(last.get("reached"), Some(Value::Bool(true))));
    }

    #[test]
    fn traceroute_without_raw_socket_capability_is_backend_failure() {
        // Only meaningful where raw ICMP is unavailable (default CI and dev
        // machines); where raw sockets work this asserts the success path
        // classification instead.
        let options = TracerouteOptions {
            timeout: Duration::from_millis(200),
            max_hops: 1,
            allow_private: true,
        };
        match run_traceroute(IpAddr::V4(Ipv4Addr::LOCALHOST), &options) {
            Ok(outcome) => assert!(!outcome.hops.is_empty()),
            Err(failure) => {
                assert!(!failure.is_target());
                let message = failure.into_message();
                assert!(
                    message.contains("traceroute unavailable"),
                    "unexpected message: {message}"
                );
            }
        }
    }
}
