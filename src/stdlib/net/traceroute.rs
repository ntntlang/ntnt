//! Parallel TTL-stepped traceroute driver, generic over probe method.
//!
//! The driver emits a probe for every TTL `1..max_hops` up front, then collects
//! replies and demultiplexes each back to its hop by the per-hop token the
//! [`TraceProbe`] implementations carry (ICMP echo sequence, UDP destination
//! port, TCP source port). Sending the whole burst at once turns the slow
//! cases — silent hops and unreached destinations, which would otherwise each
//! cost a full per-hop timeout in series — into roughly a single timeout. The
//! ordered hop list is then assembled from the collected replies: silent hops
//! become `timed_out`, and hops beyond the one that reached the destination are
//! trimmed.
//!
//! Every method needs a raw ICMP receive socket for intermediate hops, so all
//! require CAP_NET_RAW; when it is unavailable the driver returns a clear
//! backend error. Apps can check `net_capabilities()` first.

use super::enforce_resolved_target_policy;
use super::icmp::{open_icmp_trace_probe, resolve_probe_targets, unique_target_ips};
use super::probe::{HopKind, HopReply, ProbeFailure, TraceProbe};
use super::transport::{open_tcp_trace_probe, open_udp_trace_probe};
use crate::interpreter::Value;
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};

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

    // Emit one probe per TTL up front, never past the global deadline so the
    // burst alone cannot blow the timeout budget. A send failure on the very
    // first hop is fatal (nothing usable was sent). A failure mid-burst bounds
    // the burst and is recorded as a terminal hop — so it is reported in the
    // result rather than silently dropped or counted as a normal timeout.
    let mut sent_hops = 0usize;
    let mut send_terminal: Option<(usize, String)> = None;
    for hop in 1..=options.max_hops {
        if deadline.checked_duration_since(Instant::now()).is_none() {
            break;
        }
        let ttl = hop.min(u8::MAX as usize) as u8;
        let seq = hop as u16;
        match probe.send(ttl, seq, deadline) {
            Ok(()) => sent_hops = hop,
            Err(failure) if sent_hops == 0 => return Err(failure),
            Err(failure) => {
                send_terminal = Some((hop, failure.into_message()));
                break;
            }
        }
    }

    // Collect replies until the deadline, or until everything up to the hop
    // that stops the trace (reached / terminal error) has answered.
    let mut replies: Vec<HopReply> = Vec::new();
    while let Some(reply) = probe.recv(deadline)? {
        replies.push(reply);
        if collection_complete(&replies, sent_hops) {
            break;
        }
    }

    Ok(assemble_trace(replies, sent_hops, send_terminal))
}

/// The hop a trace stops at: the lowest hop that reached the destination or
/// returned a terminal error. `None` while the trace is still open.
fn stop_hop(replies: &[HopReply]) -> Option<u16> {
    replies
        .iter()
        .filter(|r| matches!(r.kind, HopKind::Reached | HopKind::Terminal(_)))
        .map(|r| r.seq)
        .min()
}

/// Whether collection can stop early: a stopping hop is known and every hop
/// below it has already answered (nothing further can change the result).
fn collection_complete(replies: &[HopReply], sent_hops: usize) -> bool {
    let Some(stop) = stop_hop(replies) else {
        return sent_hops == 0;
    };
    (1..stop).all(|hop| replies.iter().any(|r| r.seq == hop))
}

/// Whether a reply kind stops the trace at its hop (reached the destination or
/// a terminal error), as opposed to an intermediate hop report.
fn is_stop_kind(kind: &HopKind) -> bool {
    matches!(kind, HopKind::Reached | HopKind::Terminal(_))
}

/// Builds the ordered hop list from the collected replies. Hops with no reply
/// are silent (`timed_out`); hops beyond the one that stops the trace are
/// trimmed. For a given hop the first reply is kept (its latency is the true
/// RTT), except that a later reached/terminal reply upgrades an earlier plain
/// hop — so a duplicate or stray hop report can never overwrite a stop.
///
/// `send_terminal` records a hop whose probe could not be sent (a mid-burst
/// target failure); it becomes a terminal hop unless an earlier reply already
/// stopped the trace, so the failure is reported rather than dropped.
fn assemble_trace(
    replies: Vec<HopReply>,
    sent_hops: usize,
    send_terminal: Option<(usize, String)>,
) -> TraceOutcome {
    let mut by_hop: HashMap<u16, HopReply> = HashMap::new();
    for reply in replies {
        let keep = match by_hop.get(&reply.seq) {
            None => true,
            Some(existing) => is_stop_kind(&reply.kind) && !is_stop_kind(&existing.kind),
        };
        if keep {
            by_hop.insert(reply.seq, reply);
        }
    }
    let reply_stop = by_hop
        .values()
        .filter(|r| matches!(r.kind, HopKind::Reached | HopKind::Terminal(_)))
        .map(|r| r.seq)
        .min();
    let reached = reply_stop
        .is_some_and(|s| matches!(by_hop.get(&s).map(|r| &r.kind), Some(HopKind::Reached)));

    // A send failure only matters if no earlier reply already stopped the trace.
    let send_stop = send_terminal.as_ref().and_then(|(hop, _)| {
        let within = reply_stop.is_none_or(|s| *hop < usize::from(s));
        within.then_some(*hop)
    });
    let upper = match (reply_stop.map(usize::from), send_stop) {
        (Some(r), Some(s)) => r.min(s),
        (Some(r), None) => r,
        (None, Some(s)) => s,
        (None, None) => sent_hops,
    };

    let mut hops = Vec::with_capacity(upper);
    for hop in 1..=upper {
        if let Some(reply) = by_hop.get(&(hop as u16)) {
            let (reached_here, error) = match &reply.kind {
                HopKind::Reached => (true, None),
                HopKind::Hop => (false, None),
                HopKind::Terminal(message) => (false, Some(message.clone())),
            };
            hops.push(TracerouteHop {
                hop,
                from: Some(reply.from.to_string()),
                latency_ms: Some(reply.latency_ms),
                reached: reached_here,
                timed_out: false,
                error,
            });
        } else if send_stop == Some(hop) {
            // The hop whose probe could not be sent.
            let message = send_terminal
                .as_ref()
                .map(|(_, m)| m.clone())
                .unwrap_or_default();
            hops.push(TracerouteHop {
                hop,
                from: None,
                latency_ms: None,
                reached: false,
                timed_out: false,
                error: Some(message),
            });
        } else {
            hops.push(TracerouteHop {
                hop,
                from: None,
                latency_ms: None,
                reached: false,
                timed_out: true,
                error: None,
            });
        }
    }

    TraceOutcome { hops, reached }
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

    // --- assembly logic (the part that can't run in CI without raw sockets) ---

    fn reply(seq: u16, last_octet: u8, kind: HopKind) -> HopReply {
        HopReply {
            seq,
            from: IpAddr::V4(Ipv4Addr::new(10, 0, 0, last_octet)),
            latency_ms: f64::from(seq),
            kind,
        }
    }

    #[test]
    fn assemble_orders_hops_and_marks_silent_ones_timed_out() {
        // Hops 1 and 3 answered, hop 2 silent, destination reached at hop 3.
        let replies = vec![
            reply(1, 1, HopKind::Hop),
            reply(3, 9, HopKind::Reached),
            // a stray reply for a hop beyond the destination — must be trimmed
            reply(4, 9, HopKind::Reached),
        ];
        let outcome = assemble_trace(replies, 30, None);
        assert!(outcome.reached);
        assert_eq!(outcome.hops.len(), 3, "trimmed at the reached hop");
        assert_eq!(outcome.hops[0].hop, 1);
        assert!(!outcome.hops[0].timed_out);
        // hop 2 was silent
        assert_eq!(outcome.hops[1].hop, 2);
        assert!(outcome.hops[1].timed_out);
        assert!(outcome.hops[1].from.is_none());
        // hop 3 reached
        assert!(outcome.hops[2].reached);
    }

    #[test]
    fn assemble_stops_at_earliest_of_reached_and_terminal() {
        // Terminal at hop 2 stops the trace even though hop 4 reached later.
        let replies = vec![
            reply(1, 1, HopKind::Hop),
            reply(2, 2, HopKind::Terminal("prohibited".to_string())),
            reply(4, 9, HopKind::Reached),
        ];
        let outcome = assemble_trace(replies, 30, None);
        assert!(!outcome.reached);
        assert_eq!(outcome.hops.len(), 2);
        assert_eq!(outcome.hops[1].error.as_deref(), Some("prohibited"));

        // Reached earlier than a later terminal: reached wins.
        let replies = vec![
            reply(3, 3, HopKind::Reached),
            reply(7, 7, HopKind::Terminal("x".to_string())),
        ];
        let outcome = assemble_trace(replies, 30, None);
        assert!(outcome.reached);
        assert_eq!(outcome.hops.len(), 3);
    }

    #[test]
    fn assemble_unreached_trace_runs_to_sent_hops() {
        // No reply reaches the destination: report all probed hops, silent ones
        // timed out.
        let replies = vec![reply(1, 1, HopKind::Hop), reply(2, 2, HopKind::Hop)];
        let outcome = assemble_trace(replies, 5, None);
        assert!(!outcome.reached);
        assert_eq!(outcome.hops.len(), 5);
        assert!(outcome.hops[2].timed_out);
        assert!(outcome.hops[4].timed_out);
    }

    #[test]
    fn assemble_does_not_let_a_stray_hop_overwrite_a_reached_or_terminal() {
        // A reached reply followed by a duplicate plain-hop reply for the same
        // hop must keep the reached classification.
        let replies = vec![reply(2, 5, HopKind::Reached), reply(2, 9, HopKind::Hop)];
        let outcome = assemble_trace(replies, 30, None);
        assert!(outcome.reached);
        assert!(outcome.hops[1].reached);
        // The first reply's latency is kept (the true RTT), not the later one.
        assert_eq!(outcome.hops[1].latency_ms, Some(2.0));

        // A terminal hop is likewise not downgraded by a later hop report.
        let replies = vec![
            reply(1, 1, HopKind::Terminal("blocked".to_string())),
            reply(1, 2, HopKind::Hop),
        ];
        let outcome = assemble_trace(replies, 30, None);
        assert_eq!(outcome.hops[0].error.as_deref(), Some("blocked"));
    }

    #[test]
    fn assemble_records_a_mid_burst_send_failure_as_a_terminal_hop() {
        // Hops 1 and 2 answered; the probe for hop 3 could not be sent.
        let replies = vec![reply(1, 1, HopKind::Hop), reply(2, 2, HopKind::Hop)];
        let outcome = assemble_trace(replies, 2, Some((3, "no route to host".to_string())));
        assert!(!outcome.reached);
        assert_eq!(outcome.hops.len(), 3);
        assert!(!outcome.hops[2].timed_out);
        assert_eq!(outcome.hops[2].error.as_deref(), Some("no route to host"));

        // If an earlier reply already reached, the later send failure is moot.
        let replies = vec![reply(1, 1, HopKind::Reached)];
        let outcome = assemble_trace(replies, 1, Some((3, "no route".to_string())));
        assert!(outcome.reached);
        assert_eq!(outcome.hops.len(), 1);
    }

    #[test]
    fn collection_completes_only_when_everything_below_the_stop_is_in() {
        let with_gap = vec![reply(1, 1, HopKind::Hop), reply(3, 9, HopKind::Reached)];
        // Hop 2 is still missing, so collection must keep waiting.
        assert!(!collection_complete(&with_gap, 30));

        let filled = vec![
            reply(1, 1, HopKind::Hop),
            reply(2, 2, HopKind::Hop),
            reply(3, 9, HopKind::Reached),
        ];
        assert!(collection_complete(&filled, 30));

        // No stop hop yet: keep collecting (unless nothing was sent).
        assert!(!collection_complete(&[reply(1, 1, HopKind::Hop)], 30));
        assert!(collection_complete(&[], 0));
    }
}
