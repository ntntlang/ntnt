//! TTL-stepped traceroute driver, generic over probe method.
//!
//! The driver steps the TTL/hop-limit and interprets each [`HopProbe`]: a
//! `Hop` names the router at that distance, `Reached` means the destination
//! answered, `Terminal` records a terminal ICMP error and stops, and
//! `TimedOut` records a silent hop. How a probe is sent and how "reached" is
//! detected is owned by the per-method [`TraceProbe`] implementations
//! (ICMP echo in `icmp`, UDP and TCP in `transport`).
//!
//! Every method needs a raw ICMP receive socket for intermediate hops, so all
//! require CAP_NET_RAW; when it is unavailable the driver returns a clear
//! backend error. Apps can check `net_capabilities()` first.

use super::icmp::{open_icmp_trace_probe, resolve_probe_targets, unique_target_ips};
use super::policy::enforce_resolved_target_policy;
use super::probe::{probe_attempt_budget, HopProbe, ProbeFailure, TraceProbe};
use super::transport::{open_tcp_trace_probe, open_udp_trace_probe};
use crate::interpreter::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

const TRACEROUTE_LABEL: &str = "traceroute";

/// Which probe protocol a traceroute uses.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum ProbeMethod {
    Icmp,
    Udp,
    Tcp,
}

impl ProbeMethod {
    pub(super) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "icmp" => Ok(ProbeMethod::Icmp),
            "udp" => Ok(ProbeMethod::Udp),
            "tcp" => Ok(ProbeMethod::Tcp),
            other => Err(format!(
                "option 'method' must be \"icmp\", \"udp\", or \"tcp\", got {other:?}"
            )),
        }
    }

    fn label(self) -> &'static str {
        match self {
            ProbeMethod::Icmp => "icmp",
            ProbeMethod::Udp => "udp",
            ProbeMethod::Tcp => "tcp",
        }
    }

    /// Whether this method targets a destination port (UDP/TCP) or not (ICMP).
    fn uses_port(self) -> bool {
        matches!(self, ProbeMethod::Udp | ProbeMethod::Tcp)
    }
}

pub(super) struct TracerouteOptions {
    pub(super) timeout: Duration,
    pub(super) max_hops: usize,
    pub(super) allow_private: bool,
    pub(super) method: ProbeMethod,
    pub(super) port: u16,
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
    let outcome = run_traceroute(target_ip, options).map_err(ProbeFailure::into_message)?;
    Ok(traceroute_result_map(host, target_ip, options, outcome))
}

fn open_method_probe(
    target_ip: IpAddr,
    options: &TracerouteOptions,
) -> Result<Box<dyn TraceProbe>, ProbeFailure> {
    Ok(match options.method {
        ProbeMethod::Icmp => Box::new(open_icmp_trace_probe(target_ip, options.timeout)?),
        ProbeMethod::Udp => Box::new(open_udp_trace_probe(
            target_ip,
            options.port,
            options.timeout,
        )?),
        ProbeMethod::Tcp => Box::new(open_tcp_trace_probe(
            target_ip,
            options.port,
            options.timeout,
        )?),
    })
}

fn run_traceroute(
    target_ip: IpAddr,
    options: &TracerouteOptions,
) -> Result<TraceOutcome, ProbeFailure> {
    let mut probe = open_method_probe(target_ip, options)?;
    let deadline = Instant::now() + options.timeout;
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
        let hop_deadline = match budget {
            Ok(value) => Instant::now() + value,
            // No budget before the first hop means timeout_ms cannot fit the
            // requested max_hops at all: surface it as an error.
            Err(failure) if hops.is_empty() => return Err(failure),
            // Mid-trace exhaustion keeps the hops already discovered.
            Err(_) => break,
        };

        let ttl = hop.min(u8::MAX as usize) as u8;
        let seq = hop.min(u16::MAX as usize) as u16;
        match probe.probe(ttl, seq, hop_deadline) {
            Ok(HopProbe::Reached { from, latency_ms }) => {
                hops.push(TracerouteHop {
                    hop,
                    from: Some(from.to_string()),
                    latency_ms: Some(latency_ms),
                    reached: true,
                    timed_out: false,
                    error: None,
                });
                reached = true;
                break;
            }
            Ok(HopProbe::Hop { from, latency_ms }) => hops.push(TracerouteHop {
                hop,
                from: Some(from.to_string()),
                latency_ms: Some(latency_ms),
                reached: false,
                timed_out: false,
                error: None,
            }),
            Ok(HopProbe::Terminal {
                from,
                latency_ms,
                message,
            }) => {
                hops.push(TracerouteHop {
                    hop,
                    from: Some(from.to_string()),
                    latency_ms: Some(latency_ms),
                    reached: false,
                    timed_out: false,
                    error: Some(message),
                });
                break;
            }
            Ok(HopProbe::TimedOut) => hops.push(TracerouteHop {
                hop,
                from: None,
                latency_ms: None,
                reached: false,
                timed_out: true,
                error: None,
            }),
            // A backend failure before the first hop is fatal; mid-trace it
            // records the hop where it happened and keeps the partial path.
            Err(failure) if hops.is_empty() => return Err(failure),
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
    map.insert(
        "method".to_string(),
        Value::String(options.method.label().to_string()),
    );
    if options.method.uses_port() {
        map.insert("port".to_string(), Value::Int(i64::from(options.port)));
    }
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

    fn options(method: ProbeMethod, port: u16) -> TracerouteOptions {
        TracerouteOptions {
            timeout: Duration::from_millis(1000),
            max_hops: 30,
            allow_private: false,
            method,
            port,
        }
    }

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
    fn method_parse_accepts_known_and_rejects_unknown() {
        assert_eq!(ProbeMethod::parse("icmp"), Ok(ProbeMethod::Icmp));
        assert_eq!(ProbeMethod::parse("udp"), Ok(ProbeMethod::Udp));
        assert_eq!(ProbeMethod::parse("tcp"), Ok(ProbeMethod::Tcp));
        assert!(ProbeMethod::parse("bogus").is_err());
    }

    #[test]
    fn traceroute_result_map_reports_path_and_reachability() {
        let map = traceroute_result_map(
            "example.com",
            IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)),
            &options(ProbeMethod::Icmp, 0),
            sample_outcome(),
        );

        assert!(matches!(map.get("reached"), Some(Value::Bool(true))));
        assert!(matches!(map.get("hop_count"), Some(Value::Int(3))));
        assert!(matches!(map.get("max_hops"), Some(Value::Int(30))));
        assert!(matches!(map.get("method"), Some(Value::String(m)) if m == "icmp"));
        // ICMP traceroute carries no port field.
        assert!(!map.contains_key("port"));
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
    }

    #[test]
    fn tcp_and_udp_result_maps_echo_method_and_port() {
        let target = IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34));
        let tcp = traceroute_result_map(
            "example.com",
            target,
            &options(ProbeMethod::Tcp, 443),
            sample_outcome(),
        );
        assert!(matches!(tcp.get("method"), Some(Value::String(m)) if m == "tcp"));
        assert!(matches!(tcp.get("port"), Some(Value::Int(443))));
        let udp = traceroute_result_map(
            "example.com",
            target,
            &options(ProbeMethod::Udp, 33434),
            sample_outcome(),
        );
        assert!(matches!(udp.get("method"), Some(Value::String(m)) if m == "udp"));
        assert!(matches!(udp.get("port"), Some(Value::Int(33434))));
    }

    #[test]
    fn traceroute_without_raw_socket_capability_is_backend_failure() {
        // Only meaningful where raw ICMP is unavailable (default CI and dev
        // machines); where raw sockets work this asserts the success path
        // classification instead.
        let opts = TracerouteOptions {
            timeout: Duration::from_millis(200),
            max_hops: 1,
            allow_private: true,
            method: ProbeMethod::Icmp,
            port: 0,
        };
        match run_traceroute(IpAddr::V4(Ipv4Addr::LOCALHOST), &opts) {
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
