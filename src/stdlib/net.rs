//! std/net module - IPAM-grade IP/CIDR helpers and reachability probes.

use crate::error::IntentError;
use crate::interpreter::Value;
use std::cmp::{max, min};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

const DEFAULT_MAX_RESULTS: usize = 4096;
const HARD_MAX_RESULTS: usize = 65_536;
const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const MIN_TIMEOUT_MS: u64 = 50;
const MAX_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_TCP_PORTS: &[u16] = &[443, 80];
const MAX_TCP_PORTS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    V4,
    V6,
}

impl Family {
    fn bits(self) -> u8 {
        match self {
            Family::V4 => 32,
            Family::V6 => 128,
        }
    }

    fn version(self) -> i64 {
        match self {
            Family::V4 => 4,
            Family::V6 => 6,
        }
    }

    fn max_value(self) -> u128 {
        match self {
            Family::V4 => (1u128 << 32) - 1,
            Family::V6 => u128::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Network {
    family: Family,
    network: u128,
    prefix: u8,
}

impl Network {
    fn bits(&self) -> u8 {
        self.family.bits()
    }

    fn mask(&self) -> u128 {
        mask_for(self.family, self.prefix)
    }

    fn last(&self) -> u128 {
        self.network | (!self.mask() & self.family.max_value())
    }

    fn total_addresses_string(&self) -> String {
        pow2_decimal((self.bits() - self.prefix) as u32)
    }

    fn contains_network(&self, other: &Network) -> bool {
        self.family == other.family && self.network <= other.network && self.last() >= other.last()
    }

    fn overlaps(&self, other: &Network) -> bool {
        self.family == other.family && self.network <= other.last() && other.network <= self.last()
    }

    fn cidr_string(&self) -> String {
        format!(
            "{}/{}",
            ip_to_string(self.family, self.network),
            self.prefix
        )
    }
}

#[derive(Debug, Clone)]
struct ParsedInput {
    input: String,
    original_ip: u128,
    network: Network,
    kind: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReachabilityMethod {
    Auto,
    Icmp,
    Tcp,
}

impl ReachabilityMethod {
    fn parse(value: Option<&Value>) -> Result<Self, String> {
        match value {
            None => Ok(ReachabilityMethod::Auto),
            Some(Value::String(method)) => match method.as_str() {
                "auto" => Ok(ReachabilityMethod::Auto),
                "icmp" => Ok(ReachabilityMethod::Icmp),
                "tcp" => Ok(ReachabilityMethod::Tcp),
                other => Err(format!(
                    "ping() method must be 'auto', 'icmp', or 'tcp', got '{}'",
                    other
                )),
            },
            Some(other) => Err(format!(
                "ping() option 'method' must be String, got {}",
                other.type_name()
            )),
        }
    }
}

pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt ip_parse
    // @module std/net
    // @module_description Safe network primitives: IPAM-grade CIDR math and reachability probes
    // @signature ip_parse(ip_or_cidr: String) -> Result<Map, String>
    // Parses an IPv4/IPv6 address or CIDR and returns canonical IPAM fields.
    // @param ip_or_cidr Address or CIDR string, e.g. "192.168.1.0/24" or "2001:db8::/64"
    // @returns Result containing a map of canonical fields and classification booleans
    // @since v0.5.0
    // @tags #pure, #deterministic, #network
    // @example ip_parse("192.168.1.0/24") ~ "Parse an IPv4 subnet"
    module.insert(
        "ip_parse".to_string(),
        Value::NativeFunction {
            name: "ip_parse".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match string_arg(args, 0, "ip_parse") {
                Ok(input) => Ok(result_from(
                    parse_ip(input).map(|parsed| parsed_to_map(&parsed)),
                )),
                Err(e) => Err(e),
            },
        },
    );

    // @ntnt subnet_contains
    // @module std/net
    // @signature subnet_contains(cidr: String, ip_or_cidr: String) -> Result<Bool, String>
    // Returns true when the parent CIDR contains the entire child address or subnet.
    // @since v0.5.0
    // @tags #pure, #deterministic, #network
    module.insert(
        "subnet_contains".to_string(),
        Value::NativeFunction {
            name: "subnet_contains".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| binary_network_bool(args, "subnet_contains", |a, b| a.contains_network(b)),
        },
    );

    // @ntnt subnet_overlaps
    // @module std/net
    // @signature subnet_overlaps(a: String, b: String) -> Result<Bool, String>
    // Returns true when two IPv4 or IPv6 CIDRs overlap.
    // @since v0.5.0
    // @tags #pure, #deterministic, #network
    module.insert(
        "subnet_overlaps".to_string(),
        Value::NativeFunction {
            name: "subnet_overlaps".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| binary_network_bool(args, "subnet_overlaps", |a, b| a.overlaps(b)),
        },
    );

    // @ntnt subnet_split
    // @module std/net
    // @signature subnet_split(cidr: String, new_prefix: Int, opts?: Map) -> Result<Array<String>, String>
    // Splits a CIDR into child subnets with a longer prefix, enforcing result caps.
    // @since v0.5.0
    // @tags #pure, #deterministic, #network
    module.insert(
        "subnet_split".to_string(),
        Value::NativeFunction {
            name: "subnet_split".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: subnet_split_fn,
        },
    );

    // @ntnt subnet_supernet
    // @module std/net
    // @signature subnet_supernet(cidr: String, new_prefix?: Int) -> Result<String, String>
    // Returns the parent/supernet of a CIDR. Defaults to one bit shorter.
    // @since v0.5.0
    // @tags #pure, #deterministic, #network
    module.insert(
        "subnet_supernet".to_string(),
        Value::NativeFunction {
            name: "subnet_supernet".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: subnet_supernet_fn,
        },
    );

    // @ntnt subnet_summarize
    // @module std/net
    // @signature subnet_summarize(cidrs: Array<String>) -> Result<Array<String>, String>
    // Summarizes adjacent or overlapping CIDRs into the shortest equivalent route list.
    // @since v0.5.0
    // @tags #pure, #deterministic, #network
    module.insert(
        "subnet_summarize".to_string(),
        Value::NativeFunction {
            name: "subnet_summarize".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: subnet_summarize_fn,
        },
    );

    // @ntnt ip_range_to_cidrs
    // @module std/net
    // @signature ip_range_to_cidrs(start_ip: String, end_ip: String) -> Result<Array<String>, String>
    // Converts an inclusive IPv4/IPv6 range into the minimal CIDR cover.
    // @since v0.5.0
    // @tags #pure, #deterministic, #network
    module.insert(
        "ip_range_to_cidrs".to_string(),
        Value::NativeFunction {
            name: "ip_range_to_cidrs".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: ip_range_to_cidrs_fn,
        },
    );

    // @ntnt ping
    // @module std/net
    // @signature ping(host: String, opts?: Map) -> Result<Map, String>
    // Performs a first-shot host reachability probe. The default auto method uses
    // unprivileged TCP fallback when ICMP is unavailable.
    // @since v0.5.0
    // @tags #network
    module.insert(
        "ping".to_string(),
        Value::NativeFunction {
            name: "ping".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: ping_fn,
        },
    );

    module
}

fn string_arg<'a>(args: &'a [Value], index: usize, fn_name: &str) -> Result<&'a str, IntentError> {
    match &args[index] {
        Value::String(s) => Ok(s),
        other => Err(IntentError::type_error(format!(
            "{}() argument {} must be String, got {}",
            fn_name,
            index + 1,
            other.type_name()
        ))),
    }
}

fn int_arg(args: &[Value], index: usize, fn_name: &str) -> Result<i64, IntentError> {
    match &args[index] {
        Value::Int(i) => Ok(*i),
        other => Err(IntentError::type_error(format!(
            "{}() argument {} must be Int, got {}",
            fn_name,
            index + 1,
            other.type_name()
        ))),
    }
}

fn opts_arg<'a>(
    args: &'a [Value],
    index: usize,
    fn_name: &str,
) -> Result<Option<&'a HashMap<String, Value>>, IntentError> {
    if args.len() <= index {
        return Ok(None);
    }
    match &args[index] {
        Value::Map(m) => Ok(Some(m)),
        other => Err(IntentError::type_error(format!(
            "{}() options argument must be Map, got {}",
            fn_name,
            other.type_name()
        ))),
    }
}

fn result_from<T>(result: Result<T, String>) -> Value
where
    T: Into<Value>,
{
    match result {
        Ok(value) => Value::ok(value.into()),
        Err(err) => Value::err(Value::String(err)),
    }
}

impl From<HashMap<String, Value>> for Value {
    fn from(value: HashMap<String, Value>) -> Self {
        Value::Map(value)
    }
}

fn binary_network_bool(
    args: &[Value],
    fn_name: &str,
    op: fn(&Network, &Network) -> bool,
) -> Result<Value, IntentError> {
    let a = match string_arg(args, 0, fn_name) {
        Ok(s) => s,
        Err(e) => return Err(e),
    };
    let b = match string_arg(args, 1, fn_name) {
        Ok(s) => s,
        Err(e) => return Err(e),
    };

    Ok(result_from((|| {
        let left = parse_ip(a)?.network;
        let right = parse_ip(b)?.network;
        ensure_same_family(&left, &right)?;
        Ok(Value::Bool(op(&left, &right)))
    })()))
}

fn subnet_split_fn(args: &[Value]) -> Result<Value, IntentError> {
    let cidr = string_arg(args, 0, "subnet_split")?;
    let new_prefix = int_arg(args, 1, "subnet_split")?;
    let opts = opts_arg(args, 2, "subnet_split")?;

    Ok(result_from((|| {
        let network = parse_ip(cidr)?.network;
        let bits = network.bits();
        if new_prefix < 0 || new_prefix > bits as i64 {
            return Err(format!(
                "new_prefix must be between 0 and {} for IPv{}, got {}",
                bits,
                network.family.version(),
                new_prefix
            ));
        }
        let new_prefix = new_prefix as u8;
        if new_prefix <= network.prefix {
            return Err(format!(
                "new_prefix must be longer than input prefix /{}, got /{}",
                network.prefix, new_prefix
            ));
        }

        let max_results = parse_max_results(opts, "subnet_split.max_results")?;
        let count_exp = (new_prefix - network.prefix) as u32;
        if count_exp >= usize::BITS || (1usize << count_exp) > max_results {
            return Err(format!(
                "too many subnets: split would produce more than {} results",
                max_results
            ));
        }
        let count = 1usize << count_exp;
        let step = block_size(network.family, new_prefix);
        let mut subnets = Vec::with_capacity(count);
        for i in 0..count {
            let addr = network.network + step * i as u128;
            subnets.push(Value::String(
                Network {
                    family: network.family,
                    network: addr,
                    prefix: new_prefix,
                }
                .cidr_string(),
            ));
        }
        Ok(Value::Array(subnets))
    })()))
}

fn subnet_supernet_fn(args: &[Value]) -> Result<Value, IntentError> {
    let cidr = string_arg(args, 0, "subnet_supernet")?;
    Ok(result_from((|| {
        let network = parse_ip(cidr)?.network;
        let new_prefix = if args.len() > 1 {
            let explicit = int_arg(args, 1, "subnet_supernet").map_err(|e| e.to_string())?;
            if explicit < 0 || explicit > network.bits() as i64 {
                return Err(format!(
                    "new_prefix must be between 0 and {}, got {}",
                    network.bits(),
                    explicit
                ));
            }
            explicit as u8
        } else if network.prefix == 0 {
            return Err("cannot compute supernet for a /0 network".to_string());
        } else {
            network.prefix - 1
        };

        if new_prefix >= network.prefix {
            return Err(format!(
                "new_prefix must be shorter than input prefix /{}, got /{}",
                network.prefix, new_prefix
            ));
        }
        let mask = mask_for(network.family, new_prefix);
        Ok(Value::String(
            Network {
                family: network.family,
                network: network.network & mask,
                prefix: new_prefix,
            }
            .cidr_string(),
        ))
    })()))
}

fn subnet_summarize_fn(args: &[Value]) -> Result<Value, IntentError> {
    let values = match &args[0] {
        Value::Array(values) => values,
        other => {
            return Err(IntentError::type_error(format!(
                "subnet_summarize() argument 1 must be Array<String>, got {}",
                other.type_name()
            )))
        }
    };

    Ok(result_from((|| {
        if values.len() > DEFAULT_MAX_RESULTS {
            return Err(format!(
                "too many CIDRs: maximum is {}",
                DEFAULT_MAX_RESULTS
            ));
        }

        let mut ranges: Vec<(Family, u128, u128)> = Vec::with_capacity(values.len());
        for value in values {
            let cidr = match value {
                Value::String(s) => s,
                other => {
                    return Err(format!(
                        "subnet_summarize() expects Array<String>, got {} item",
                        other.type_name()
                    ))
                }
            };
            let network = parse_ip(cidr)?.network;
            ranges.push((network.family, network.network, network.last()));
        }

        if ranges.is_empty() {
            return Ok(Value::Array(vec![]));
        }

        let family = ranges[0].0;
        if ranges
            .iter()
            .any(|(item_family, _, _)| *item_family != family)
        {
            return Err("mixed IPv4/IPv6 families are not comparable".to_string());
        }

        ranges.sort_by_key(|(_, start, end)| (*start, *end));
        let mut merged: Vec<(u128, u128)> = Vec::new();
        for (_, start, end) in ranges {
            if let Some((_, last_end)) = merged.last_mut() {
                if start <= last_end.saturating_add(1) {
                    *last_end = max(*last_end, end);
                    continue;
                }
            }
            merged.push((start, end));
        }

        let mut output = Vec::new();
        for (start, end) in merged {
            for network in range_to_networks(family, start, end, HARD_MAX_RESULTS)? {
                output.push(Value::String(network.cidr_string()));
            }
        }
        Ok(Value::Array(output))
    })()))
}

fn ip_range_to_cidrs_fn(args: &[Value]) -> Result<Value, IntentError> {
    let start = string_arg(args, 0, "ip_range_to_cidrs")?;
    let end = string_arg(args, 1, "ip_range_to_cidrs")?;

    Ok(result_from((|| {
        let start = parse_ip(start)?;
        let end = parse_ip(end)?;
        if start.network.prefix != start.network.bits() || end.network.prefix != end.network.bits()
        {
            return Err("ip_range_to_cidrs() expects bare IP addresses, not CIDRs".to_string());
        }
        ensure_same_family(&start.network, &end.network)?;
        if start.original_ip > end.original_ip {
            return Err("start_ip must be less than or equal to end_ip".to_string());
        }
        let networks = range_to_networks(
            start.network.family,
            start.original_ip,
            end.original_ip,
            DEFAULT_MAX_RESULTS,
        )?;
        Ok(Value::Array(
            networks
                .into_iter()
                .map(|network| Value::String(network.cidr_string()))
                .collect(),
        ))
    })()))
}

fn ping_fn(args: &[Value]) -> Result<Value, IntentError> {
    let host = string_arg(args, 0, "ping")?;
    let opts = opts_arg(args, 1, "ping")?;

    Ok(result_from((|| {
        let method = ReachabilityMethod::parse(opts.and_then(|m| m.get("method")))?;
        if method == ReachabilityMethod::Icmp {
            return Err(
                "ICMP ping unavailable: std/net Phase 1 defaults to unprivileged TCP fallback; use method: 'auto' or 'tcp' unless the runtime adds ICMP capability support"
                    .to_string(),
            );
        }

        let allow_private = parse_bool_option(opts, "allow_private", false)?;
        let timeout_ms = parse_u64_option(opts, "timeout_ms", DEFAULT_TIMEOUT_MS)?
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
        let tcp_ports = parse_tcp_ports(opts)?;

        let probe = tcp_reachability(
            host,
            &tcp_ports,
            Duration::from_millis(timeout_ms),
            allow_private,
        )?;
        let mut result = HashMap::new();
        result.insert("host".to_string(), Value::String(host.to_string()));
        result.insert("reachable".to_string(), Value::Bool(probe.reachable));
        result.insert("method".to_string(), Value::String("tcp".to_string()));
        if method == ReachabilityMethod::Auto {
            result.insert(
                "fallback_from".to_string(),
                Value::String("icmp".to_string()),
            );
            result.insert("permission_limited".to_string(), Value::Bool(true));
        } else {
            result.insert("permission_limited".to_string(), Value::Bool(false));
        }
        result.insert(
            "ports_tried".to_string(),
            Value::Array(tcp_ports.iter().map(|p| Value::Int(*p as i64)).collect()),
        );
        if let Some(port) = probe.connected_port {
            result.insert("connected_port".to_string(), Value::Int(port as i64));
        } else {
            result.insert("connected_port".to_string(), Value::none());
            result.insert("reason".to_string(), Value::String(probe.reason));
        }
        if let Some(latency_ms) = probe.latency_ms {
            result.insert("latency_ms".to_string(), Value::Float(latency_ms));
        }
        Ok(Value::Map(result))
    })()))
}

#[derive(Debug)]
struct TcpProbeResult {
    reachable: bool,
    connected_port: Option<u16>,
    latency_ms: Option<f64>,
    reason: String,
}

fn tcp_reachability(
    host: &str,
    ports: &[u16],
    timeout: Duration,
    allow_private: bool,
) -> Result<TcpProbeResult, String> {
    let targets = resolve_tcp_targets(host, ports)?;
    if targets.is_empty() {
        return Ok(TcpProbeResult {
            reachable: false,
            connected_port: None,
            latency_ms: None,
            reason: "no resolved addresses".to_string(),
        });
    }
    enforce_resolved_target_policy(&targets, allow_private)?;

    let mut last_reason = "unreachable".to_string();
    for (port, addr) in targets {
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                drop(stream);
                return Ok(TcpProbeResult {
                    reachable: true,
                    connected_port: Some(port),
                    latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
                    reason: "connected".to_string(),
                });
            }
            Err(e) => {
                last_reason = e.kind().to_string();
            }
        }
    }
    Ok(TcpProbeResult {
        reachable: false,
        connected_port: None,
        latency_ms: None,
        reason: last_reason,
    })
}

fn resolve_tcp_targets(host: &str, ports: &[u16]) -> Result<Vec<(u16, SocketAddr)>, String> {
    let mut targets = Vec::new();
    for port in ports {
        let addrs: Vec<SocketAddr> = (host, *port)
            .to_socket_addrs()
            .map_err(|e| format!("failed to resolve {}:{}: {}", host, port, e))?
            .collect();
        targets.extend(addrs.into_iter().map(|addr| (*port, addr)));
    }
    Ok(targets)
}

fn enforce_resolved_target_policy(
    targets: &[(u16, SocketAddr)],
    allow_private: bool,
) -> Result<(), String> {
    for (_, addr) in targets {
        enforce_target_policy(addr.ip(), allow_private)?;
    }
    Ok(())
}

fn parse_ip(input: &str) -> Result<ParsedInput, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("IP/CIDR input cannot be empty".to_string());
    }

    let (ip_part, prefix_part) = match input.split_once('/') {
        Some((ip, prefix)) => (ip, Some(prefix)),
        None => (input, None),
    };

    let ip: IpAddr = ip_part
        .parse()
        .map_err(|_| format!("invalid IP address: {}", ip_part))?;
    let (family, original_ip) = ip_to_u128(ip);
    let bits = family.bits();
    let prefix = match prefix_part {
        Some(prefix) => {
            let parsed: u8 = prefix
                .parse()
                .map_err(|_| format!("invalid CIDR prefix: {}", prefix))?;
            if parsed > bits {
                return Err(format!(
                    "CIDR prefix /{} is invalid for IPv{}",
                    parsed,
                    family.version()
                ));
            }
            parsed
        }
        None => bits,
    };
    let mask = mask_for(family, prefix);

    Ok(ParsedInput {
        input: input.to_string(),
        original_ip,
        network: Network {
            family,
            network: original_ip & mask,
            prefix,
        },
        kind: if prefix_part.is_some() {
            "network"
        } else {
            "address"
        },
    })
}

fn parsed_to_map(parsed: &ParsedInput) -> HashMap<String, Value> {
    let network = &parsed.network;
    let ip = ip_to_string(network.family, parsed.original_ip);
    let network_ip = ip_to_string(network.family, network.network);
    let first = ip_to_string(network.family, network.network);
    let last = ip_to_string(network.family, network.last());

    let mut map = HashMap::new();
    map.insert("input".to_string(), Value::String(input_cidr_or_ip(parsed)));
    map.insert("kind".to_string(), Value::String(parsed.kind.to_string()));
    map.insert("version".to_string(), Value::Int(network.family.version()));
    map.insert("ip".to_string(), Value::String(ip));
    map.insert("prefix".to_string(), Value::Int(network.prefix as i64));
    map.insert("network".to_string(), Value::String(network_ip));
    map.insert("first".to_string(), Value::String(first));
    map.insert("last".to_string(), Value::String(last));
    map.insert(
        "total_addresses".to_string(),
        Value::String(network.total_addresses_string()),
    );
    map.insert(
        "reverse_zone".to_string(),
        reverse_zone(network).map_or_else(Value::none, Value::String),
    );

    match network.family {
        Family::V4 => add_ipv4_fields(&mut map, parsed),
        Family::V6 => add_ipv6_fields(&mut map, parsed),
    }

    map
}

fn input_cidr_or_ip(parsed: &ParsedInput) -> String {
    parsed.input.clone()
}

fn add_ipv4_fields(map: &mut HashMap<String, Value>, parsed: &ParsedInput) {
    let network = &parsed.network;
    let ip = Ipv4Addr::from(parsed.original_ip as u32);
    let mask = mask_for(Family::V4, network.prefix) as u32;
    let wildcard = !mask;
    let total = 1u128 << (32 - network.prefix);
    let usable = match network.prefix {
        0..=30 => total.saturating_sub(2),
        31 => 2,
        32 => 1,
        _ => 0,
    };

    map.insert(
        "broadcast".to_string(),
        Value::String(ip_to_string(Family::V4, network.last())),
    );
    map.insert(
        "netmask".to_string(),
        Value::String(Ipv4Addr::from(mask).to_string()),
    );
    map.insert(
        "wildcard_mask".to_string(),
        Value::String(Ipv4Addr::from(wildcard).to_string()),
    );
    map.insert(
        "usable_hosts".to_string(),
        Value::String(usable.to_string()),
    );
    map.insert("expanded".to_string(), Value::String(ip.to_string()));
    add_common_classification(map, IpAddr::V4(ip));
}

fn add_ipv6_fields(map: &mut HashMap<String, Value>, parsed: &ParsedInput) {
    let ip = Ipv6Addr::from(parsed.original_ip);
    map.insert("broadcast".to_string(), Value::none());
    map.insert("netmask".to_string(), Value::none());
    map.insert("wildcard_mask".to_string(), Value::none());
    map.insert("usable_hosts".to_string(), Value::none());
    map.insert("expanded".to_string(), Value::String(expand_ipv6(ip)));
    add_common_classification(map, IpAddr::V6(ip));
}

fn add_common_classification(map: &mut HashMap<String, Value>, ip: IpAddr) {
    let classification = classify_ip(ip);
    map.insert(
        "is_private".to_string(),
        Value::Bool(classification.is_private),
    );
    map.insert(
        "is_loopback".to_string(),
        Value::Bool(classification.is_loopback),
    );
    map.insert(
        "is_link_local".to_string(),
        Value::Bool(classification.is_link_local),
    );
    map.insert(
        "is_multicast".to_string(),
        Value::Bool(classification.is_multicast),
    );
    map.insert(
        "is_unspecified".to_string(),
        Value::Bool(classification.is_unspecified),
    );
    map.insert(
        "is_documentation".to_string(),
        Value::Bool(classification.is_documentation),
    );
    map.insert(
        "is_unique_local".to_string(),
        Value::Bool(classification.is_unique_local),
    );
}

#[derive(Debug)]
struct IpClassification {
    is_private: bool,
    is_loopback: bool,
    is_link_local: bool,
    is_multicast: bool,
    is_unspecified: bool,
    is_documentation: bool,
    is_unique_local: bool,
}

fn classify_ip(ip: IpAddr) -> IpClassification {
    match ip {
        IpAddr::V4(ip) => IpClassification {
            is_private: ip.is_private(),
            is_loopback: ip.is_loopback(),
            is_link_local: ip.is_link_local(),
            is_multicast: ip.is_multicast(),
            is_unspecified: ip.is_unspecified(),
            is_documentation: is_ipv4_documentation(ip),
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
                is_unique_local,
            }
        }
    }
}

fn is_ipv4_documentation(ip: Ipv4Addr) -> bool {
    let octets = ip.octets();
    matches!(
        octets,
        [192, 0, 2, _] | [198, 51, 100, _] | [203, 0, 113, _]
    )
}

fn enforce_target_policy(ip: IpAddr, allow_private: bool) -> Result<(), String> {
    let classification = classify_ip(ip);
    let denied_by_default = classification.is_private
        || classification.is_loopback
        || classification.is_link_local
        || classification.is_unspecified
        || classification.is_multicast
        || classification.is_documentation
        || matches!(ip, IpAddr::V4(ipv4) if ipv4.is_broadcast());
    if !denied_by_default {
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
    ) || matches!(
        std::env::var("NTNT_ALLOW_PRIVATE_IPS").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn ensure_same_family(a: &Network, b: &Network) -> Result<(), String> {
    if a.family != b.family {
        return Err("mixed IPv4/IPv6 families are not comparable".to_string());
    }
    Ok(())
}

fn parse_max_results(opts: Option<&HashMap<String, Value>>, key: &str) -> Result<usize, String> {
    let Some(opts) = opts else {
        return Ok(DEFAULT_MAX_RESULTS);
    };
    let Some(value) = opts.get("max_results") else {
        return Ok(DEFAULT_MAX_RESULTS);
    };
    let Value::Int(raw) = value else {
        return Err(format!("{} must be Int", key));
    };
    if *raw <= 0 {
        return Err(format!("{} must be positive", key));
    }
    let requested = usize::try_from(*raw).map_err(|_| format!("{} is too large", key))?;
    Ok(min(requested, HARD_MAX_RESULTS))
}

fn parse_bool_option(
    opts: Option<&HashMap<String, Value>>,
    key: &str,
    default: bool,
) -> Result<bool, String> {
    match opts.and_then(|m| m.get(key)) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(format!(
            "option '{}' must be Bool, got {}",
            key,
            other.type_name()
        )),
    }
}

fn parse_u64_option(
    opts: Option<&HashMap<String, Value>>,
    key: &str,
    default: u64,
) -> Result<u64, String> {
    match opts.and_then(|m| m.get(key)) {
        None => Ok(default),
        Some(Value::Int(value)) if *value >= 0 => Ok(*value as u64),
        Some(Value::Int(_)) => Err(format!("option '{}' must be non-negative", key)),
        Some(other) => Err(format!(
            "option '{}' must be Int, got {}",
            key,
            other.type_name()
        )),
    }
}

fn parse_tcp_ports(opts: Option<&HashMap<String, Value>>) -> Result<Vec<u16>, String> {
    let Some(value) = opts.and_then(|m| m.get("tcp_ports")) else {
        return Ok(DEFAULT_TCP_PORTS.to_vec());
    };
    let Value::Array(values) = value else {
        return Err("option 'tcp_ports' must be Array<Int>".to_string());
    };
    if values.is_empty() {
        return Err("option 'tcp_ports' cannot be empty".to_string());
    }
    if values.len() > MAX_TCP_PORTS {
        return Err(format!(
            "option 'tcp_ports' supports at most {} ports",
            MAX_TCP_PORTS
        ));
    }
    let mut ports = Vec::with_capacity(values.len());
    for value in values {
        let Value::Int(port) = value else {
            return Err("option 'tcp_ports' must be Array<Int>".to_string());
        };
        if *port < 1 || *port > 65_535 {
            return Err(format!("invalid TCP port: {}", port));
        }
        let port = *port as u16;
        if !ports.contains(&port) {
            ports.push(port);
        }
    }
    Ok(ports)
}

fn mask_for(family: Family, prefix: u8) -> u128 {
    let bits = family.bits();
    if prefix == 0 {
        0
    } else {
        (!0u128 << (bits - prefix)) & family.max_value()
    }
}

fn block_size(family: Family, prefix: u8) -> u128 {
    let host_bits = family.bits() - prefix;
    if host_bits == 128 {
        u128::MAX
    } else {
        1u128 << host_bits
    }
}

fn ip_to_u128(ip: IpAddr) -> (Family, u128) {
    match ip {
        IpAddr::V4(ip) => (Family::V4, u32::from(ip) as u128),
        IpAddr::V6(ip) => (Family::V6, u128::from(ip)),
    }
}

fn ip_to_string(family: Family, value: u128) -> String {
    match family {
        Family::V4 => Ipv4Addr::from(value as u32).to_string(),
        Family::V6 => Ipv6Addr::from(value).to_string(),
    }
}

fn expand_ipv6(ip: Ipv6Addr) -> String {
    ip.segments()
        .iter()
        .map(|segment| format!("{:04x}", segment))
        .collect::<Vec<_>>()
        .join(":")
}

fn pow2_decimal(exp: u32) -> String {
    match exp {
        0..=127 => (1u128 << exp).to_string(),
        128 => "340282366920938463463374607431768211456".to_string(),
        _ => unreachable!("IP address families only use up to 128 bits"),
    }
}

fn reverse_zone(network: &Network) -> Option<String> {
    match network.family {
        Family::V4 => {
            if network.prefix % 8 != 0 {
                return None;
            }
            let octets = Ipv4Addr::from(network.network as u32).octets();
            let count = (network.prefix / 8) as usize;
            let labels: Vec<String> = octets[..count]
                .iter()
                .rev()
                .map(|octet| octet.to_string())
                .collect();
            Some(if labels.is_empty() {
                "in-addr.arpa".to_string()
            } else {
                format!("{}.in-addr.arpa", labels.join("."))
            })
        }
        Family::V6 => {
            if network.prefix % 4 != 0 {
                return None;
            }
            let nibbles = (network.prefix / 4) as usize;
            let expanded = format!("{:032x}", network.network);
            let mut labels: Vec<String> = expanded
                .chars()
                .take(nibbles)
                .map(|ch| ch.to_string())
                .collect();
            labels.reverse();
            Some(if labels.is_empty() {
                "ip6.arpa".to_string()
            } else {
                format!("{}.ip6.arpa", labels.join("."))
            })
        }
    }
}

fn range_to_networks(
    family: Family,
    mut start: u128,
    end: u128,
    max_results: usize,
) -> Result<Vec<Network>, String> {
    if start > end {
        return Err("range start must be <= end".to_string());
    }
    if start == 0 && end == family.max_value() {
        return Ok(vec![Network {
            family,
            network: 0,
            prefix: 0,
        }]);
    }
    let bits = family.bits();
    let mut output = Vec::new();
    while start <= end {
        if output.len() >= max_results {
            return Err(format!(
                "too many CIDRs: range would produce more than {} results",
                max_results
            ));
        }

        let remaining = end - start + 1;
        let alignment_prefix = if start == 0 {
            0
        } else {
            bits - min(start.trailing_zeros() as u8, bits)
        };
        let remaining_prefix = bits - floor_log2(remaining) as u8;
        let prefix = max(alignment_prefix, remaining_prefix);
        let size = block_size(family, prefix);
        output.push(Network {
            family,
            network: start,
            prefix,
        });
        if size == 0 || start > u128::MAX - size {
            break;
        }
        start += size;
    }
    Ok(output)
}

fn floor_log2(value: u128) -> u32 {
    debug_assert!(value > 0);
    127 - value.leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_policy_classifies_ipv4_mapped_ipv6_by_embedded_address() {
        let mapped_loopback = "[::ffff:127.0.0.1]:80".parse::<SocketAddr>().unwrap();
        let mapped_metadata = "[::ffff:169.254.169.254]:80".parse::<SocketAddr>().unwrap();

        assert!(enforce_resolved_target_policy(&[(80, mapped_loopback)], false).is_err());
        assert!(enforce_resolved_target_policy(&[(80, mapped_metadata)], false).is_err());
    }

    #[test]
    fn target_policy_rejects_special_ranges_by_default() {
        let multicast = "224.0.0.1:80".parse::<SocketAddr>().unwrap();
        let documentation = "192.0.2.1:80".parse::<SocketAddr>().unwrap();
        let broadcast = "255.255.255.255:80".parse::<SocketAddr>().unwrap();

        assert!(enforce_resolved_target_policy(&[(80, multicast)], false).is_err());
        assert!(enforce_resolved_target_policy(&[(80, documentation)], false).is_err());
        assert!(enforce_resolved_target_policy(&[(80, broadcast)], false).is_err());
    }

    #[test]
    fn target_policy_checks_all_resolved_addresses_before_probe() {
        let public = "93.184.216.34:443".parse::<SocketAddr>().unwrap();
        let private = "127.0.0.1:443".parse::<SocketAddr>().unwrap();
        let targets = [(443, public), (443, private)];

        let err = enforce_resolved_target_policy(&targets, false).unwrap_err();
        assert!(err.contains("Network target denied by policy"));
    }
}
