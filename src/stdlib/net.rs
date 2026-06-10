//! std/net module - IPAM-grade IP/CIDR helpers and reachability probes.

use crate::error::IntentError;
use crate::interpreter::Value;
use chrono::{DateTime, SecondsFormat, Utc};
use hickory_resolver::config::{ResolverConfig, ResolverOpts};
use hickory_resolver::error::{ResolveError, ResolveErrorKind};
use hickory_resolver::proto::rr::RecordType;
use hickory_resolver::Resolver;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::{
    ClientConfig, ClientConnection, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::cmp::{max, min};
use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::mem::MaybeUninit;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream, ToSocketAddrs};
use std::str::FromStr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use x509_parser::extensions::GeneralName;
use x509_parser::prelude::*;

const DEFAULT_MAX_RESULTS: usize = 4096;
const HARD_MAX_RESULTS: usize = 65_536;
const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const MIN_TIMEOUT_MS: u64 = 50;
const MAX_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_PROBE_COUNT: usize = 1;
const MAX_PROBE_COUNT: usize = 10;
const DEFAULT_INTERVAL_MS: u64 = 0;
const MAX_INTERVAL_MS: u64 = 5_000;
const DEFAULT_REACHABLE_TCP_PORTS: [u16; 2] = [80, 443];
const MAX_TCP_PORTS: usize = 10;
const MAX_PORT_SCAN_PORTS: usize = 128;
static ICMP_IDENT_COUNTER: AtomicU16 = AtomicU16::new(1);
const DEFAULT_PORT_SCAN_CONCURRENCY: usize = 20;
const MAX_PORT_SCAN_CONCURRENCY: usize = 50;

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
    original_ip: u128,
    network: Network,
    kind: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DnsRecordType {
    name: String,
    record_type: RecordType,
}

impl DnsRecordType {
    fn parse(value: &str) -> Result<Self, String> {
        let name = value.trim().to_ascii_uppercase();
        let record_type = match RecordType::from_str(&name) {
            Ok(record_type) if SUPPORTED_DNS_RECORD_TYPES.contains(&name.as_str()) => record_type,
            _ => return Err(unsupported_dns_record_type(&name)),
        };
        Ok(Self { name, record_type })
    }

    fn as_str(&self) -> &str {
        &self.name
    }

    fn hickory_type(&self) -> RecordType {
        self.record_type
    }
}

const SUPPORTED_DNS_RECORD_TYPES: &[&str] = &[
    "A",
    "AAAA",
    "ANAME",
    "CAA",
    "CDNSKEY",
    "CDS",
    "CNAME",
    "CSYNC",
    "DNSKEY",
    "DS",
    "HINFO",
    "HTTPS",
    "KEY",
    "MX",
    "NAPTR",
    "NS",
    "NSEC",
    "NSEC3",
    "NSEC3PARAM",
    "NULL",
    "OPENPGPKEY",
    "PTR",
    "RRSIG",
    "SIG",
    "SOA",
    "SRV",
    "SSHFP",
    "SVCB",
    "TLSA",
    "TXT",
];

fn unsupported_dns_record_type(record_type: &str) -> String {
    format!(
        "dns_lookup() record_type must be one of {}, got '{}'",
        SUPPORTED_DNS_RECORD_TYPES.join(", "),
        record_type
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DnsAnswer {
    record_type: String,
    name: String,
    value: String,
    ttl: u32,
}

fn validate_ping_method(value: Option<&Value>) -> Result<(), String> {
    match value {
        None => Ok(()),
        Some(Value::String(method)) => match method.as_str() {
            "auto" | "icmp" => Ok(()),
            "tcp" => {
                Err("ping() only supports ICMP; use tcp_connect() for TCP port checks".to_string())
            }
            other => Err(format!(
                "ping() method must be 'auto' or 'icmp', got '{}'",
                other
            )),
        },
        Some(other) => Err(format!(
            "ping() option 'method' must be String, got {}",
            other.type_name()
        )),
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
    // @since v0.4.10
    // @tags #pure, #deterministic, #network
    // @example ip_parse("192.168.1.0/24") ~ "Parse an IPv4 subnet"
    module.insert(
        "ip_parse".to_string(),
        Value::NativeFunction {
            name: "ip_parse".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let input = string_arg(args, 0, "ip_parse")?;
                Ok(result_value(parse_ip(input).map(|parsed| {
                    Value::Map(parsed_to_map(input.trim(), &parsed))
                })))
            },
        },
    );

    // @ntnt subnet_contains
    // @module std/net
    // @signature subnet_contains(cidr: String, ip_or_cidr: String) -> Result<Bool, String>
    // Returns true when the parent CIDR contains the entire child address or subnet.
    // @since v0.4.10
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
    // @since v0.4.10
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
    // @since v0.4.10
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
    // @since v0.4.10
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
    // @since v0.4.10
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
    // @since v0.4.10
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
    // Performs an ICMP ping using native sockets (unprivileged datagram ICMP when
    // available, raw ICMP as fallback). Unreachable targets return Ok with failed
    // attempts; missing socket permissions and resolver/system failures return
    // Err(String). Apps that want TCP port checks should use tcp_connect();
    // high-level reachability checks can use reachable().
    // @param host Hostname or IP address to resolve and probe
    // @param opts Optional map with count (default 1, max 10), timeout_ms (default 2000), interval_ms (default 0, max 5000), and allow_private
    // @returns Result containing reachability status, latency summary, and per-attempt results
    // @since v0.4.10
    // @tags #network
    // @example ping("example.com", map { "count": 3 }) ~ "Send three ICMP echo requests"
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

    // @ntnt tcp_connect
    // @module std/net
    // @signature tcp_connect(host: String, port: Int, opts?: Map) -> Result<Map, String>
    // Performs a bounded TCP connect probe to one explicit port. Closed, refused,
    // or timed-out ports return Ok(map { "connected": false, ... }); invalid input,
    // policy denial, and resolver/system failures return Err(String).
    // @param host Hostname or IP address to resolve and probe
    // @param port TCP port from 1 to 65535
    // @param opts Optional map with timeout_ms, count, interval_ms, and allow_private
    // @returns Result containing connection status, latency summary, and per-attempt results
    // @since v0.4.10
    // @tags #network
    // @example tcp_connect("example.com", 443) ~ "Check whether TCP 443 accepts connections"
    module.insert(
        "tcp_connect".to_string(),
        Value::NativeFunction {
            name: "tcp_connect".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: tcp_connect_fn,
        },
    );

    // @ntnt reachable
    // @module std/net
    // @signature reachable(host: String, opts?: Map) -> Result<Map, String>
    // Performs a high-level reachability check using ICMP plus TCP ports 80 and 443
    // by default. Caller-provided tcp_ports add extra explicit TCP ports.
    // The result records the method that established reachability without pretending TCP is ping.
    // @param host Hostname or IP address to resolve and probe
    // @param opts Optional map with extra tcp_ports, timeout_ms, count, interval_ms, and allow_private
    // @returns Result containing reachability status, method used, fallback metadata, and attempt summary
    // @since v0.4.10
    // @tags #network
    // @example reachable("example.com", map { "tcp_ports": [8080], "count": 5 }) ~ "Check ICMP plus TCP 80, 443, and 8080"
    module.insert(
        "reachable".to_string(),
        Value::NativeFunction {
            name: "reachable".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: reachable_fn,
        },
    );

    // @ntnt port_scan
    // @module std/net
    // @signature port_scan(host: String, ports: Array<Int>, opts?: Map) -> Result<Array<Map>, String>
    // Performs a bounded TCP scan of explicit ports for one host. Only explicit port arrays
    // are accepted; ranges are intentionally not expanded. Results are sorted by port.
    // @param host Hostname or IP address to resolve and scan
    // @param ports Explicit array of TCP ports from 1 to 65535; duplicates are rejected
    // @param opts Optional map with timeout_ms, concurrency, and allow_private
    // @returns Result containing one map per port with open, latency_ms, and reason fields
    // @since v0.4.10
    // @tags #network
    // @example port_scan("example.com", [80, 443], map { "timeout_ms": 500 }) ~ "Scan explicit TCP ports"
    module.insert(
        "port_scan".to_string(),
        Value::NativeFunction {
            name: "port_scan".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: port_scan_fn,
        },
    );

    // @ntnt dns_lookup
    // @module std/net
    // @signature dns_lookup(name: String, record_type?: String, opts?: Map) -> Result<Array<Map>, String>
    // Looks up DNS records for a name. Supports A, AAAA, ANAME, CAA, CDNSKEY, CDS,
    // CNAME, CSYNC, DNSKEY, DS, HINFO, HTTPS, KEY, MX, NAPTR, NS, NSEC,
    // NSEC3, NSEC3PARAM, NULL, OPENPGPKEY, PTR, RRSIG, SIG, SOA, SRV,
    // SSHFP, SVCB, TLSA, and TXT records.
    // No-answer DNS responses return Ok([]); invalid input and resolver/system failures return Err(String).
    // @param name DNS name to query
    // @param record_type Optional supported DNS record type. Defaults to A.
    // @param opts Optional map with timeout_ms. When passing opts, include an explicit record_type such as "A".
    // @returns Result containing an array of records with actual returned type, name, value, and ttl
    // @since v0.4.10
    // @tags #network
    // @example dns_lookup("example.com", "A") ~ "Look up IPv4 DNS records"
    module.insert(
        "dns_lookup".to_string(),
        Value::NativeFunction {
            name: "dns_lookup".to_string(),
            arity: 1,
            max_arity: 3,
            requires: None,
            func: dns_lookup_fn,
        },
    );

    // @ntnt dns_reverse
    // @module std/net
    // @signature dns_reverse(ip: String, opts?: Map) -> Result<Array<String>, String>
    // Performs a reverse DNS/PTR lookup for an IPv4 or IPv6 address. No-answer
    // DNS responses return Ok([]); invalid input and resolver/system failures return Err(String).
    // @param ip IPv4 or IPv6 address
    // @param opts Optional map with timeout_ms
    // @returns Result containing zero or more PTR hostnames
    // @since v0.4.10
    // @tags #network
    // @example dns_reverse("8.8.8.8") ~ "Look up PTR hostnames for an IP address"
    module.insert(
        "dns_reverse".to_string(),
        Value::NativeFunction {
            name: "dns_reverse".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: dns_reverse_fn,
        },
    );

    // @ntnt tls_info
    // @module std/net
    // @signature tls_info(host: String, opts?: Map) -> Result<Map, String>
    // Opens a bounded TLS connection and returns certificate metadata. Validation
    // failures still return Ok(map { "valid": false, ... }) when a certificate is available.
    // @param host Hostname or IP address to connect to
    // @param opts Optional map with port, timeout_ms, server_name, and allow_private
    // @returns Result containing certificate subject, issuer, validity window, SANs, serial, protocol, and validation status
    // @since v0.4.10
    // @tags #network
    // @example tls_info("example.com") ~ "Inspect the HTTPS certificate for example.com"
    module.insert(
        "tls_info".to_string(),
        Value::NativeFunction {
            name: "tls_info".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: tls_info_fn,
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

fn result_value(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => Value::ok(value),
        Err(err) => Value::err(Value::String(err)),
    }
}

fn binary_network_bool(
    args: &[Value],
    fn_name: &str,
    op: fn(&Network, &Network) -> bool,
) -> Result<Value, IntentError> {
    let a = string_arg(args, 0, fn_name)?;
    let b = string_arg(args, 1, fn_name)?;

    Ok(result_value((|| {
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

    Ok(result_value((|| {
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
    Ok(result_value((|| {
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

    Ok(result_value((|| {
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

    Ok(result_value((|| {
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

    Ok(result_value((|| {
        validate_ping_method(opts.and_then(|m| m.get("method")))?;
        let options = parse_probe_options(opts)?;
        Ok(Value::Map(icmp_ping_for_host(host, &options)?))
    })()))
}

fn tcp_connect_fn(args: &[Value]) -> Result<Value, IntentError> {
    let host = string_arg(args, 0, "tcp_connect")?;
    let raw_port = int_arg(args, 1, "tcp_connect")?;
    let opts = opts_arg(args, 2, "tcp_connect")?;

    Ok(result_value((|| {
        let port = validate_tcp_port_arg(raw_port, "tcp_connect")?;
        let options = parse_probe_options(opts)?;
        let attempts = tcp_reachability_attempts_for_host(
            host,
            &[port],
            options.timeout,
            options.count,
            options.interval,
            options.allow_private,
        )?;
        Ok(Value::Map(tcp_connect_result_map(host, port, &attempts)))
    })()))
}

fn reachable_fn(args: &[Value]) -> Result<Value, IntentError> {
    let host = string_arg(args, 0, "reachable")?;
    let opts = opts_arg(args, 1, "reachable")?;

    Ok(result_value((|| {
        let options = parse_probe_options(opts)?;
        let icmp_result = icmp_ping_for_host(host, &options);
        let tcp_ports = parse_reachable_tcp_ports(opts, "reachable")?;
        let attempts = tcp_reachability_attempts_for_host(
            host,
            &tcp_ports,
            options.timeout,
            options.count,
            options.interval,
            options.allow_private,
        )?;
        Ok(Value::Map(reachable_result_map(
            host,
            &tcp_ports,
            &attempts,
            icmp_result,
        )))
    })()))
}

fn port_scan_fn(args: &[Value]) -> Result<Value, IntentError> {
    let host = string_arg(args, 0, "port_scan")?;
    let Value::Array(port_values) = &args[1] else {
        return Err(IntentError::type_error(format!(
            "port_scan() argument 2 must be Array<Int>, got {}",
            args[1].type_name()
        )));
    };
    let opts = opts_arg(args, 2, "port_scan")?;

    Ok(result_value((|| {
        let ports = parse_explicit_ports(port_values, "port_scan", "argument 2")?;
        let timeout_ms = parse_u64_option(opts, "timeout_ms", DEFAULT_TIMEOUT_MS)?
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
        let concurrency = parse_port_scan_concurrency(opts)?;
        let allow_private = parse_bool_option(opts, "allow_private", false)?;
        let results = port_scan_for_host(
            host,
            &ports,
            Duration::from_millis(timeout_ms),
            concurrency,
            allow_private,
        )?;
        Ok(Value::Array(
            results.into_iter().map(port_scan_result_to_value).collect(),
        ))
    })()))
}

fn dns_lookup_fn(args: &[Value]) -> Result<Value, IntentError> {
    let name = string_arg(args, 0, "dns_lookup")?;
    let (record_type, opts) = dns_lookup_args(args)?;

    Ok(result_value((|| {
        let record_type = DnsRecordType::parse(record_type)?;
        let resolver = dns_resolver(opts)?;
        let answers = dns_lookup_records(&resolver, name, record_type)?;
        Ok(dns_answers_to_value(answers))
    })()))
}

fn dns_reverse_fn(args: &[Value]) -> Result<Value, IntentError> {
    let ip = string_arg(args, 0, "dns_reverse")?;
    let opts = opts_arg(args, 1, "dns_reverse")?;

    Ok(result_value((|| {
        let ip: IpAddr = ip.parse().map_err(|_| {
            format!(
                "dns_reverse() argument 1 must be a valid IP address, got {}",
                ip
            )
        })?;
        let resolver = dns_resolver(opts)?;
        let names = dns_reverse_names(&resolver, ip)?;
        Ok(Value::Array(names.into_iter().map(Value::String).collect()))
    })()))
}

fn tls_info_fn(args: &[Value]) -> Result<Value, IntentError> {
    let host = string_arg(args, 0, "tls_info")?;
    let opts = opts_arg(args, 1, "tls_info")?;

    Ok(result_value((|| {
        let port = parse_tls_port(opts)?;
        let timeout_ms = parse_u64_option(opts, "timeout_ms", DEFAULT_TIMEOUT_MS)?
            .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
        let server_name = parse_string_option(opts, "server_name")?.unwrap_or(host);
        let allow_private = parse_bool_option(opts, "allow_private", false)?;
        let targets = resolve_tcp_targets(host, &[port])?;
        enforce_resolved_target_policy(&targets, allow_private)?;
        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let mut last_error = None;
        for (_, addr) in targets {
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                break;
            };
            if remaining.is_zero() {
                break;
            }
            match tls_info_for_addr(host, server_name, port, addr, remaining) {
                Ok(info) => return Ok(Value::Map(info)),
                Err(err) => last_error = Some(err),
            }
        }
        Err(last_error.unwrap_or_else(|| format!("TLS inspection timed out for {}:{}", host, port)))
    })()))
}

#[derive(Debug)]
struct MetadataVerifier;

impl ServerCertVerifier for MetadataVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        vec![
            SignatureScheme::ECDSA_NISTP256_SHA256,
            SignatureScheme::ECDSA_NISTP384_SHA384,
            SignatureScheme::ED25519,
            SignatureScheme::RSA_PSS_SHA256,
            SignatureScheme::RSA_PSS_SHA384,
            SignatureScheme::RSA_PSS_SHA512,
            SignatureScheme::RSA_PKCS1_SHA256,
            SignatureScheme::RSA_PKCS1_SHA384,
            SignatureScheme::RSA_PKCS1_SHA512,
        ]
    }
}

fn tls_info_for_addr(
    host: &str,
    server_name: &str,
    port: u16,
    addr: SocketAddr,
    timeout: Duration,
) -> Result<HashMap<String, Value>, String> {
    let metadata_config = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(MetadataVerifier))
        .with_no_client_auth();
    let metadata = connect_tls(
        host,
        server_name,
        port,
        addr,
        timeout,
        Arc::new(metadata_config),
    )?;

    let cert = metadata
        .certificates
        .first()
        .ok_or_else(|| format!("TLS peer {}:{} did not present a certificate", host, port))?;
    let mut result = tls_cert_to_map(cert.as_ref())?;
    result.insert("host".to_string(), Value::String(host.to_string()));
    result.insert(
        "server_name".to_string(),
        Value::String(server_name.to_string()),
    );
    result.insert("port".to_string(), Value::Int(port as i64));
    result.insert("remote_addr".to_string(), Value::String(addr.to_string()));
    result.insert("local_addr".to_string(), Value::String(metadata.local_addr));
    result.insert("protocol".to_string(), Value::String(metadata.protocol));
    result.insert("cipher".to_string(), Value::String(metadata.cipher));
    result.insert(
        "chain_len".to_string(),
        Value::Int(metadata.certificates.len() as i64),
    );

    let validation_error = verify_certificate_chain(server_name, &metadata.certificates)?;
    let valid = validation_error.is_none();
    result.insert("valid".to_string(), Value::Bool(valid));
    result.insert(
        "validation_error".to_string(),
        validation_error.map_or_else(Value::none, Value::String),
    );

    Ok(result)
}

struct TlsConnectionMetadata {
    local_addr: String,
    protocol: String,
    cipher: String,
    certificates: Vec<CertificateDer<'static>>,
}

fn tls_root_store() -> Result<RootCertStore, String> {
    let mut roots = RootCertStore::empty();
    let (added, _) =
        roots.add_parsable_certificates(webpki_root_certs::TLS_SERVER_ROOT_CERTS.iter().cloned());
    if added == 0 {
        return Err("no TLS trust roots available".to_string());
    }
    Ok(roots)
}

fn verify_certificate_chain(
    server_name: &str,
    certificates: &[CertificateDer<'static>],
) -> Result<Option<String>, String> {
    let Some((end_entity, intermediates)) = certificates.split_first() else {
        return Ok(Some(
            "certificate validation failed: peer did not present a certificate".to_string(),
        ));
    };
    let name = ServerName::try_from(server_name.to_string()).map_err(|_| {
        format!(
            "tls_info() server_name must be a valid DNS name or IP address, got {}",
            server_name
        )
    })?;
    let verifier = WebPkiServerVerifier::builder(Arc::new(tls_root_store()?))
        .build()
        .map_err(|e| format!("certificate validation unavailable: {}", e))?;
    Ok(verifier
        .verify_server_cert(end_entity, intermediates, &name, &[], UnixTime::now())
        .err()
        .map(|err| format!("certificate validation failed: {}", err)))
}

fn sleep_until_tls_deadline(deadline: Instant) {
    if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
        thread::sleep(min(remaining, Duration::from_millis(10)));
    }
}

fn connect_tls(
    host: &str,
    server_name: &str,
    port: u16,
    addr: SocketAddr,
    timeout: Duration,
    config: Arc<ClientConfig>,
) -> Result<TlsConnectionMetadata, String> {
    let name = ServerName::try_from(server_name.to_string()).map_err(|_| {
        format!(
            "tls_info() server_name must be a valid DNS name or IP address, got {}",
            server_name
        )
    })?;
    let deadline = Instant::now() + timeout;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|e| format!("TLS TCP connect failed for {}:{}: {}", host, port, e))?;
    if Instant::now() >= deadline {
        return Err(format!("TLS handshake timed out for {}:{}", host, port));
    }
    stream
        .set_nonblocking(true)
        .map_err(|e| format!("failed to set TLS socket nonblocking mode: {}", e))?;
    let local_addr = stream
        .local_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_default();
    let mut conn = ClientConnection::new(config, name)
        .map_err(|e| format!("failed to create TLS client: {}", e))?;
    while conn.is_handshaking() {
        let now = Instant::now();
        if now >= deadline {
            return Err(format!("TLS handshake timed out for {}:{}", host, port));
        }
        match conn.complete_io(&mut stream) {
            Ok((0, 0)) => sleep_until_tls_deadline(deadline),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                sleep_until_tls_deadline(deadline)
            }
            Err(e) => return Err(format!("TLS handshake failed for {}:{}: {}", host, port, e)),
        }
    }

    Ok(TlsConnectionMetadata {
        local_addr,
        protocol: conn
            .protocol_version()
            .map(|version| format!("{:?}", version))
            .unwrap_or_else(|| "unknown".to_string()),
        cipher: conn
            .negotiated_cipher_suite()
            .map(|suite| format!("{:?}", suite.suite()))
            .unwrap_or_else(|| "unknown".to_string()),
        certificates: conn.peer_certificates().unwrap_or(&[]).to_vec(),
    })
}

fn tls_cert_to_map(cert_der: &[u8]) -> Result<HashMap<String, Value>, String> {
    let (_, cert) = X509Certificate::from_der(cert_der)
        .map_err(|e| format!("failed to parse TLS certificate: {}", e))?;
    let mut map = HashMap::new();
    let subject = cert.subject().to_string();
    let issuer = cert.issuer().to_string();
    map.insert("subject".to_string(), Value::String(subject));
    map.insert("issuer".to_string(), Value::String(issuer));
    map.insert(
        "subject_common_name".to_string(),
        x509_common_name(cert.subject()).map_or_else(Value::none, Value::String),
    );
    map.insert(
        "issuer_common_name".to_string(),
        x509_common_name(cert.issuer()).map_or_else(Value::none, Value::String),
    );
    map.insert(
        "not_before".to_string(),
        Value::String(asn1_time_to_rfc3339(cert.validity().not_before.timestamp())),
    );
    map.insert(
        "not_after".to_string(),
        Value::String(asn1_time_to_rfc3339(cert.validity().not_after.timestamp())),
    );
    let now = Utc::now().timestamp();
    let days_left = ((cert.validity().not_after.timestamp() - now) / 86_400).max(0);
    map.insert("days_left".to_string(), Value::Int(days_left));
    map.insert(
        "serial".to_string(),
        Value::String(cert.raw_serial_as_string()),
    );
    map.insert(
        "signature_algorithm".to_string(),
        Value::String(cert.signature_algorithm.algorithm.to_string()),
    );
    map.insert(
        "san".to_string(),
        Value::Array(
            cert_subject_alt_names(&cert)
                .into_iter()
                .map(Value::String)
                .collect(),
        ),
    );
    Ok(map)
}

fn x509_common_name(name: &X509Name<'_>) -> Option<String> {
    name.iter_common_name()
        .next()
        .and_then(|cn| cn.as_str().ok())
        .map(str::to_string)
}

fn cert_subject_alt_names(cert: &X509Certificate<'_>) -> Vec<String> {
    let Ok(Some(san)) = cert.subject_alternative_name() else {
        return Vec::new();
    };
    san.value
        .general_names
        .iter()
        .filter_map(|name| match name {
            GeneralName::DNSName(value) => Some((*value).to_string()),
            GeneralName::IPAddress(bytes) => ip_address_from_san(bytes),
            _ => None,
        })
        .collect()
}

fn ip_address_from_san(bytes: &[u8]) -> Option<String> {
    match bytes.len() {
        4 => Some(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).to_string()),
        16 => {
            let mut octets = [0u8; 16];
            octets.copy_from_slice(bytes);
            Some(Ipv6Addr::from(octets).to_string())
        }
        _ => None,
    }
}

fn asn1_time_to_rfc3339(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp(timestamp, 0)
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
        .unwrap_or_else(|| timestamp.to_string())
}

fn dns_lookup_args(args: &[Value]) -> Result<(&str, Option<&HashMap<String, Value>>), IntentError> {
    if args.len() <= 1 {
        return Ok(("A", None));
    }

    let Value::String(record_type) = &args[1] else {
        return Err(IntentError::type_error(format!(
            "dns_lookup() argument 2 must be String record_type, got {}",
            args[1].type_name()
        )));
    };
    let opts = opts_arg(args, 2, "dns_lookup")?;
    Ok((record_type, opts))
}

fn dns_resolver(opts: Option<&HashMap<String, Value>>) -> Result<Resolver, String> {
    let timeout_ms = parse_u64_option(opts, "timeout_ms", DEFAULT_TIMEOUT_MS)?
        .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
    let (config, mut resolver_opts) = system_resolver_config()?;
    resolver_opts.timeout = Duration::from_millis(timeout_ms);
    resolver_opts.attempts = 1;
    Resolver::new(config, resolver_opts)
        .map_err(|e| format!("failed to initialize DNS resolver: {}", e))
}

fn system_resolver_config() -> Result<(ResolverConfig, ResolverOpts), String> {
    #[cfg(any(unix, target_os = "windows"))]
    {
        hickory_resolver::system_conf::read_system_conf()
            .map_err(|e| format!("failed to read system DNS configuration: {}", e))
    }
    #[cfg(not(any(unix, target_os = "windows")))]
    {
        Ok((ResolverConfig::default(), ResolverOpts::default()))
    }
}

fn dns_lookup_records(
    resolver: &Resolver,
    name: &str,
    record_type: DnsRecordType,
) -> Result<Vec<DnsAnswer>, String> {
    let lookup = match resolver.lookup(name, record_type.hickory_type()) {
        Ok(lookup) => lookup,
        Err(err) if is_dns_no_records(&err) => return Ok(vec![]),
        Err(err) => {
            return Err(format!(
                "DNS lookup failed for {} {}: {}",
                name,
                record_type.as_str(),
                err
            ))
        }
    };

    let mut answers = Vec::new();
    for record in lookup.record_iter() {
        let Some(data) = record.data() else {
            continue;
        };
        let value = data
            .ip_addr()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| data.to_string());
        answers.push(DnsAnswer {
            record_type: record.record_type().to_string(),
            name: record.name().to_utf8(),
            value,
            ttl: record.ttl(),
        });
    }
    Ok(answers)
}

fn dns_reverse_names(resolver: &Resolver, ip: IpAddr) -> Result<Vec<String>, String> {
    match resolver.reverse_lookup(ip) {
        Ok(lookup) => Ok(lookup.iter().map(|name| name.to_utf8()).collect()),
        Err(err) if is_dns_no_records(&err) => Ok(vec![]),
        Err(err) => Err(format!("reverse DNS lookup failed for {}: {}", ip, err)),
    }
}

fn is_dns_no_records(err: &ResolveError) -> bool {
    matches!(err.kind(), ResolveErrorKind::NoRecordsFound { .. })
}

fn dns_answers_to_value(answers: Vec<DnsAnswer>) -> Value {
    Value::Array(
        answers
            .into_iter()
            .map(|answer| {
                let mut map = HashMap::new();
                map.insert("type".to_string(), Value::String(answer.record_type));
                map.insert("name".to_string(), Value::String(answer.name));
                map.insert("value".to_string(), Value::String(answer.value));
                map.insert("ttl".to_string(), Value::Int(answer.ttl as i64));
                Value::Map(map)
            })
            .collect(),
    )
}

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

fn resolve_ping_targets(host: &str) -> Result<Vec<(u16, SocketAddr)>, String> {
    let resolved: Vec<SocketAddr> = (host, 0)
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve {}: {}", host, e))?
        .collect();
    Ok(resolved.into_iter().map(|addr| (0, addr)).collect())
}

fn icmp_ping(
    display_host: &str,
    target_ips: &[IpAddr],
    options: &ProbeOptions,
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
        );
        let result = match result {
            Ok(result) => result,
            Err(err) if icmp_probe_error_is_target_failure(&err) => failed_ping_result(
                display_host,
                *target_ip,
                format!("ICMP ping failed: {}", err),
                options.count,
            ),
            Err(err) => {
                first_fatal_error.get_or_insert(err);
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
) -> Result<IcmpPingResult, String> {
    let socket = match create_icmp_socket(target_ip, timeout) {
        Ok(socket) => socket,
        Err(err) if icmp_io_error_is_target_failure(&err) => {
            return Ok(failed_ping_result(
                display_host,
                target_ip,
                format!("ICMP ping failed: {}", err),
                count,
            ));
        }
        Err(err) => return Err(native_ping_socket_error(err)),
    };
    let local_ip = socket
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_socket())
        .map(|addr| addr.ip());
    let ident = icmp_ident_for_socket(&socket).unwrap_or_else(next_icmp_ident);
    let count = count.max(1);
    let deadline = Instant::now() + timeout;
    let payload = [0u8; 56];
    let mut attempts = Vec::new();
    let mut latencies = Vec::new();

    for seq in 1..=count {
        let sequence = (seq.min(u16::MAX as usize)) as u16;
        let remaining_attempts = count.saturating_sub(seq).saturating_add(1);
        let budget = match deadline.checked_duration_since(Instant::now()) {
            Some(remaining) => ping_attempt_budget(remaining, remaining_attempts, interval),
            None => Err(format!(
                "ICMP ping timed out before completing requested count: sent {} of {}",
                attempts.len(),
                count
            )),
        };
        let per_attempt_timeout = match budget {
            Ok(value) => value,
            // A budget failure before the first probe means timeout_ms cannot
            // fit the requested count/interval at all: surface it as an error.
            Err(err) if attempts.is_empty() => return Err(err),
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
        match send_ping_attempt(
            &socket,
            target_ip,
            local_ip,
            ident,
            sequence,
            &payload,
            per_attempt_timeout,
        ) {
            Ok(Some(reply)) => {
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
            Ok(None) => attempts.push(IcmpPingAttempt {
                seq: Some(sequence as i64),
                reachable: false,
                from: Some(target_ip.to_string()),
                ttl: None,
                latency_ms: None,
                error: Some("request timed out".to_string()),
            }),
            Err(err) if icmp_probe_error_is_target_failure(&err) => {
                attempts.push(IcmpPingAttempt {
                    seq: Some(sequence as i64),
                    reachable: false,
                    from: Some(target_ip.to_string()),
                    ttl: None,
                    latency_ms: None,
                    error: Some(err.to_string()),
                })
            }
            Err(err) => return Err(err),
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

fn ping_attempt_budget(
    remaining: Duration,
    remaining_attempts: usize,
    interval: Duration,
) -> Result<Duration, String> {
    let attempts = remaining_attempts.max(1);
    let future_intervals = interval
        .checked_mul(attempts.saturating_sub(1) as u32)
        .ok_or_else(|| "ICMP ping interval budget overflowed".to_string())?;
    if remaining <= future_intervals {
        return Err(format!(
            "ICMP ping timeout_ms is too small for count {} and interval_ms {}",
            remaining_attempts,
            interval.as_millis()
        ));
    }
    let probe_budget = (remaining - future_intervals) / (attempts as u32);
    if probe_budget.is_zero() {
        return Err("ICMP ping timeout_ms is too small to send requested probes".to_string());
    }
    Ok(probe_budget)
}

fn next_icmp_ident() -> u16 {
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
    if icmp_io_error_is_target_failure(&dgram_error) {
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

fn icmp_ident_for_socket(socket: &Socket) -> Option<u16> {
    socket
        .local_addr()
        .ok()
        .and_then(|addr| addr.as_socket())
        .map(|addr| addr.port())
        .filter(|port| *port != 0)
}

fn send_ping_attempt(
    socket: &Socket,
    target_ip: IpAddr,
    local_ip: Option<IpAddr>,
    ident: u16,
    sequence: u16,
    payload: &[u8],
    timeout: Duration,
) -> Result<Option<IcmpProbeReply>, String> {
    let deadline = Instant::now() + timeout;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(native_ping_socket_error)?;
    socket
        .set_write_timeout(Some(timeout))
        .map_err(native_ping_socket_error)?;
    let packet = build_icmp_echo_request(target_ip, local_ip, ident, sequence, payload)?;
    let sent_at = Instant::now();
    socket.send(&packet).map_err(native_ping_send_error)?;
    wait_for_icmp_reply(socket, target_ip, ident, sequence, deadline, sent_at)
}

#[derive(Debug, Clone)]
struct IcmpProbeReply {
    source: IpAddr,
    ttl: Option<i64>,
    sequence: u16,
    latency_ms: f64,
}

#[derive(Debug)]
enum IcmpProbeEvent {
    Reply(IcmpProbeReply),
    TargetFailure(String),
}

fn wait_for_icmp_reply(
    socket: &Socket,
    target_ip: IpAddr,
    ident: u16,
    sequence: u16,
    deadline: Instant,
    sent_at: Instant,
) -> Result<Option<IcmpProbeReply>, String> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Ok(None);
        };
        socket
            .set_read_timeout(Some(remaining))
            .map_err(native_ping_socket_error)?;
        let mut buffer = [MaybeUninit::<u8>::uninit(); 2048];
        let (len, from) = match socket.recv_from(&mut buffer) {
            Ok(received) => received,
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                if err.kind() == ErrorKind::Interrupted {
                    continue;
                }
                return Ok(None);
            }
            Err(err) if icmp_io_error_is_target_failure(&err) => {
                return Err(format!("ICMP ping failed: {}", err));
            }
            Err(err) => return Err(native_ping_error(err)),
        };
        let received_elapsed = sent_at.elapsed();
        let bytes = unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), len) };
        let source = from.as_socket().map(|addr| addr.ip()).unwrap_or(target_ip);
        match parse_icmp_probe_event(bytes, source, target_ip, ident, sequence, received_elapsed) {
            Some(IcmpProbeEvent::Reply(reply)) => return Ok(Some(reply)),
            Some(IcmpProbeEvent::TargetFailure(message)) => {
                return Err(message);
            }
            None => continue,
        }
    }
}

fn build_icmp_echo_request(
    target_ip: IpAddr,
    local_ip: Option<IpAddr>,
    ident: u16,
    sequence: u16,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
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
        (IpAddr::V6(dst), Some(IpAddr::V6(src))) => icmpv6_checksum(src, dst, &packet),
        (IpAddr::V6(_), _) => {
            return Err(
                "ICMP ping failed: could not determine local IPv6 address for checksum".to_string(),
            );
        }
    };
    packet[2..4].copy_from_slice(&checksum.to_be_bytes());
    Ok(packet)
}

fn parse_icmp_probe_event(
    bytes: &[u8],
    fallback_source: IpAddr,
    target_ip: IpAddr,
    ident: u16,
    sequence: u16,
    elapsed: Duration,
) -> Option<IcmpProbeEvent> {
    match target_ip {
        IpAddr::V4(_) => parse_icmpv4_probe_event(bytes, fallback_source, ident, sequence, elapsed),
        IpAddr::V6(_) => parse_icmpv6_probe_event(bytes, fallback_source, ident, sequence, elapsed),
    }
}

fn parse_icmpv4_probe_event(
    bytes: &[u8],
    fallback_source: IpAddr,
    ident: u16,
    sequence: u16,
    elapsed: Duration,
) -> Option<IcmpProbeEvent> {
    let (icmp, ttl, source) = if bytes.len() >= 20 && bytes[0] >> 4 == 4 {
        let header_len = usize::from(bytes[0] & 0x0f) * 4;
        if header_len < 20 || bytes.len() < header_len + 8 {
            return None;
        }
        let source = IpAddr::V4(Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]));
        (&bytes[header_len..], Some(i64::from(bytes[8])), source)
    } else {
        (bytes, None, fallback_source)
    };
    parse_icmp_payload(
        icmp,
        source,
        ttl,
        ident,
        sequence,
        elapsed,
        IcmpFamily::V4,
        0,
        &[3, 11],
    )
}

fn parse_icmpv6_probe_event(
    bytes: &[u8],
    fallback_source: IpAddr,
    ident: u16,
    sequence: u16,
    elapsed: Duration,
) -> Option<IcmpProbeEvent> {
    // Raw ICMPv6 sockets on mainstream platforms deliver the ICMPv6 payload
    // without the outer IPv6 header. Hop-limit is therefore unavailable here
    // unless this path grows recvmsg ancillary-data support later.
    parse_icmp_payload(
        bytes,
        fallback_source,
        None,
        ident,
        sequence,
        elapsed,
        IcmpFamily::V6,
        129,
        &[1, 2, 3, 4],
    )
}

#[derive(Clone, Copy)]
enum IcmpFamily {
    V4,
    V6,
}

fn parse_icmp_payload(
    icmp: &[u8],
    source: IpAddr,
    ttl: Option<i64>,
    ident: u16,
    sequence: u16,
    elapsed: Duration,
    family: IcmpFamily,
    reply_type: u8,
    failure_types: &[u8],
) -> Option<IcmpProbeEvent> {
    if icmp.len() < 8 {
        return None;
    }
    let icmp_type = icmp[0];
    let icmp_code = icmp[1];
    let packet_ident = u16::from_be_bytes([icmp[4], icmp[5]]);
    let packet_sequence = u16::from_be_bytes([icmp[6], icmp[7]]);
    if icmp_type == reply_type && packet_ident == ident && packet_sequence == sequence {
        return Some(IcmpProbeEvent::Reply(IcmpProbeReply {
            source,
            ttl,
            sequence: packet_sequence,
            latency_ms: elapsed.as_secs_f64() * 1000.0,
        }));
    }
    if failure_types.contains(&icmp_type) && icmp_error_quotes_probe(family, icmp, ident, sequence)
    {
        return Some(IcmpProbeEvent::TargetFailure(format!(
            "ICMP error from {}: type {} code {}",
            source, icmp_type, icmp_code
        )));
    }
    None
}

fn icmp_error_quotes_probe(family: IcmpFamily, icmp: &[u8], ident: u16, sequence: u16) -> bool {
    match family {
        IcmpFamily::V4 => icmpv4_error_quotes_probe(icmp, ident, sequence),
        IcmpFamily::V6 => icmpv6_error_quotes_probe(icmp, ident, sequence),
    }
}

fn icmpv4_error_quotes_probe(icmp: &[u8], ident: u16, sequence: u16) -> bool {
    let quoted = match icmp.get(8..) {
        Some(quoted) => quoted,
        None => return false,
    };
    if quoted.len() < 28 || quoted[0] >> 4 != 4 {
        return false;
    }
    let header_len = usize::from(quoted[0] & 0x0f) * 4;
    if header_len < 20 || quoted.len() < header_len + 8 || quoted[9] != 1 {
        return false;
    }
    let embedded_icmp = &quoted[header_len..];
    embedded_icmp[0] == 8
        && embedded_icmp[1] == 0
        && u16::from_be_bytes([embedded_icmp[4], embedded_icmp[5]]) == ident
        && u16::from_be_bytes([embedded_icmp[6], embedded_icmp[7]]) == sequence
}

fn icmpv6_error_quotes_probe(icmp: &[u8], ident: u16, sequence: u16) -> bool {
    let quoted = match icmp.get(8..) {
        Some(quoted) => quoted,
        None => return false,
    };
    if quoted.len() < 48 || quoted[0] >> 4 != 6 {
        return false;
    }
    let Some(embedded_icmp) = quoted_icmpv6_payload(quoted) else {
        return false;
    };
    embedded_icmp[0] == 128
        && embedded_icmp[1] == 0
        && u16::from_be_bytes([embedded_icmp[4], embedded_icmp[5]]) == ident
        && u16::from_be_bytes([embedded_icmp[6], embedded_icmp[7]]) == sequence
}

fn quoted_icmpv6_payload(quoted: &[u8]) -> Option<&[u8]> {
    let mut next_header = *quoted.get(6)?;
    let mut offset = 40usize;
    loop {
        if next_header == 58 {
            return quoted.get(offset..).filter(|payload| payload.len() >= 8);
        }
        if !matches!(next_header, 0 | 43 | 44 | 60) {
            return None;
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

fn internet_checksum(data: &[u8]) -> u16 {
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

fn icmpv6_checksum(source: Ipv6Addr, destination: Ipv6Addr, icmp: &[u8]) -> u16 {
    let mut pseudo_packet = Vec::with_capacity(40 + icmp.len());
    pseudo_packet.extend_from_slice(&source.octets());
    pseudo_packet.extend_from_slice(&destination.octets());
    pseudo_packet.extend_from_slice(&(icmp.len() as u32).to_be_bytes());
    pseudo_packet.extend_from_slice(&[0, 0, 0, 58]);
    pseudo_packet.extend_from_slice(icmp);
    internet_checksum(&pseudo_packet)
}

fn native_ping_socket_error(err: std::io::Error) -> String {
    format!("ICMP ping unavailable: native socket failed: {}", err)
}

fn native_ping_send_error(err: std::io::Error) -> String {
    if icmp_io_error_is_target_failure(&err) {
        format!("ICMP ping failed: {}", err)
    } else {
        native_ping_error(err)
    }
}

fn native_ping_error(err: std::io::Error) -> String {
    format!("ICMP ping failed: {}", err)
}

fn icmp_io_error_is_target_failure(err: &std::io::Error) -> bool {
    err.kind() == ErrorKind::ConnectionRefused || icmp_target_failure(&err.to_string())
}

fn icmp_probe_error_is_target_failure(err: &str) -> bool {
    icmp_target_failure(err)
}

fn icmp_target_failure(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    // "connection refused" is how Linux DGRAM ICMP sockets surface a pending
    // ICMP destination-unreachable error on recv; keep this list in sync with
    // icmp_io_error_is_target_failure, which checks the same condition by kind.
    lower.contains("connection refused")
        || lower.contains("network is unreachable")
        || lower.contains("no route to host")
        || lower.contains("destination host unreachable")
        || lower.contains("destination net unreachable")
        || lower.contains("host is down")
        || lower.contains("icmp error from")
}

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

fn icmp_ping_for_host(
    host: &str,
    options: &ProbeOptions,
) -> Result<HashMap<String, Value>, String> {
    let targets = resolve_ping_targets(host)?;
    enforce_resolved_target_policy(&targets, options.allow_private)?;
    let target_ips = unique_target_ips(&targets)?;
    let result = icmp_ping(host, &target_ips, options)?;
    Ok(icmp_ping_result_map(result))
}

fn unique_target_ips(targets: &[(u16, SocketAddr)]) -> Result<Vec<IpAddr>, String> {
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

struct ProbeOptions {
    timeout: Duration,
    count: usize,
    interval: Duration,
    allow_private: bool,
}

fn parse_probe_options(opts: Option<&HashMap<String, Value>>) -> Result<ProbeOptions, String> {
    let allow_private = parse_bool_option(opts, "allow_private", false)?;
    let timeout_ms = parse_u64_option(opts, "timeout_ms", DEFAULT_TIMEOUT_MS)?
        .clamp(MIN_TIMEOUT_MS, MAX_TIMEOUT_MS);
    let count = parse_count_option(opts)?;
    let interval_ms = parse_u64_option(opts, "interval_ms", DEFAULT_INTERVAL_MS)?
        .clamp(DEFAULT_INTERVAL_MS, MAX_INTERVAL_MS);
    Ok(ProbeOptions {
        timeout: Duration::from_millis(timeout_ms),
        count,
        interval: Duration::from_millis(interval_ms),
        allow_private,
    })
}

fn validate_tcp_port_arg(port: i64, fn_name: &str) -> Result<u16, String> {
    if !(1..=65_535).contains(&port) {
        return Err(format!(
            "{}() port must be between 1 and 65535, got {}",
            fn_name, port
        ));
    }
    Ok(port as u16)
}

#[derive(Debug)]
struct TcpProbeResult {
    reachable: bool,
    connected_port: Option<u16>,
    latency_ms: Option<f64>,
    remote_addr: Option<String>,
    local_addr: Option<String>,
    reason: String,
}

fn tcp_reachability_attempts_for_host(
    host: &str,
    ports: &[u16],
    timeout: Duration,
    count: usize,
    interval: Duration,
    allow_private: bool,
) -> Result<Vec<TcpProbeResult>, String> {
    let targets = resolve_tcp_targets(host, ports)?;
    enforce_resolved_target_policy(&targets, allow_private)?;
    Ok(tcp_reachability_attempts(
        &targets, timeout, count, interval,
    ))
}

#[derive(Debug)]
struct PortScanResult {
    port: u16,
    open: bool,
    latency_ms: Option<f64>,
    remote_addr: Option<String>,
    local_addr: Option<String>,
    reason: String,
}

fn port_scan_for_host(
    host: &str,
    ports: &[u16],
    timeout: Duration,
    concurrency: usize,
    allow_private: bool,
) -> Result<Vec<PortScanResult>, String> {
    let targets = resolve_port_scan_targets(host, ports)?;
    enforce_resolved_target_policy(&targets, allow_private)?;

    let mut by_port: HashMap<u16, Vec<SocketAddr>> = HashMap::new();
    for (port, addr) in targets {
        by_port.entry(port).or_default().push(addr);
    }

    let mut results = Vec::with_capacity(ports.len());
    for batch in ports.chunks(concurrency.max(1)) {
        let mut handles = Vec::with_capacity(batch.len());
        for port in batch {
            let port = *port;
            let addrs = by_port.remove(&port).unwrap_or_default();
            handles.push(thread::spawn(move || {
                scan_port_targets(port, addrs, timeout)
            }));
        }
        for handle in handles {
            results.push(
                handle
                    .join()
                    .map_err(|_| "port_scan() worker panicked".to_string())?,
            );
        }
    }

    results.sort_by_key(|result| result.port);
    Ok(results)
}

fn scan_port_targets(port: u16, targets: Vec<SocketAddr>, timeout: Duration) -> PortScanResult {
    if targets.is_empty() {
        return PortScanResult {
            port,
            open: false,
            latency_ms: None,
            remote_addr: None,
            local_addr: None,
            reason: "no resolved addresses".to_string(),
        };
    }

    let mut last_reason = "unreachable".to_string();
    for addr in targets {
        let start = Instant::now();
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => {
                let remote_addr = stream.peer_addr().ok().map(|addr| addr.to_string());
                let local_addr = stream.local_addr().ok().map(|addr| addr.to_string());
                drop(stream);
                return PortScanResult {
                    port,
                    open: true,
                    latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
                    remote_addr,
                    local_addr,
                    reason: "connected".to_string(),
                };
            }
            Err(e) => {
                last_reason = e.kind().to_string();
            }
        }
    }

    PortScanResult {
        port,
        open: false,
        latency_ms: None,
        remote_addr: None,
        local_addr: None,
        reason: last_reason,
    }
}

fn tcp_reachability_attempts(
    targets: &[(u16, SocketAddr)],
    timeout: Duration,
    count: usize,
    interval: Duration,
) -> Vec<TcpProbeResult> {
    let mut attempts = Vec::with_capacity(count);
    for attempt_index in 0..count {
        if attempt_index > 0 && !interval.is_zero() {
            thread::sleep(interval);
        }
        attempts.push(tcp_reachability_once(targets, timeout));
    }
    attempts
}

fn tcp_reachability_once(targets: &[(u16, SocketAddr)], timeout: Duration) -> TcpProbeResult {
    if targets.is_empty() {
        return TcpProbeResult {
            reachable: false,
            connected_port: None,
            latency_ms: None,
            remote_addr: None,
            local_addr: None,
            reason: "no resolved addresses".to_string(),
        };
    }

    let mut last_reason = "unreachable".to_string();
    for (port, addr) in targets {
        let start = Instant::now();
        match TcpStream::connect_timeout(addr, timeout) {
            Ok(stream) => {
                let remote_addr = stream.peer_addr().ok().map(|addr| addr.to_string());
                let local_addr = stream.local_addr().ok().map(|addr| addr.to_string());
                drop(stream);
                return TcpProbeResult {
                    reachable: true,
                    connected_port: Some(*port),
                    latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
                    remote_addr,
                    local_addr,
                    reason: "connected".to_string(),
                };
            }
            Err(e) => {
                last_reason = e.kind().to_string();
            }
        }
    }
    TcpProbeResult {
        reachable: false,
        connected_port: None,
        latency_ms: None,
        remote_addr: None,
        local_addr: None,
        reason: last_reason,
    }
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

fn resolve_port_scan_targets(host: &str, ports: &[u16]) -> Result<Vec<(u16, SocketAddr)>, String> {
    let Some(seed_port) = ports.first().copied() else {
        return Ok(Vec::new());
    };

    let resolved: Vec<SocketAddr> = (host, seed_port)
        .to_socket_addrs()
        .map_err(|e| format!("failed to resolve {}: {}", host, e))?
        .collect();

    let mut ips = Vec::with_capacity(resolved.len());
    let mut seen = HashSet::with_capacity(resolved.len());
    for addr in resolved {
        if seen.insert(addr.ip()) {
            ips.push(addr.ip());
        }
    }

    let mut targets = Vec::with_capacity(ports.len() * ips.len());
    for port in ports {
        targets.extend(ips.iter().map(|ip| (*port, SocketAddr::new(*ip, *port))));
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

fn tcp_reachability_result_map(
    host: &str,
    ports: &[u16],
    attempts: &[TcpProbeResult],
) -> HashMap<String, Value> {
    let sent = attempts.len();
    let received = attempts.iter().filter(|attempt| attempt.reachable).count();
    let failed = sent.saturating_sub(received);
    let mut latency_count = 0usize;
    let mut latency_sum = 0.0;
    let mut min_ms = f64::INFINITY;
    let mut max_ms = f64::NEG_INFINITY;
    for latency_ms in attempts.iter().filter_map(|attempt| attempt.latency_ms) {
        latency_count += 1;
        latency_sum += latency_ms;
        min_ms = min_ms.min(latency_ms);
        max_ms = max_ms.max(latency_ms);
    }

    let mut result = HashMap::new();
    result.insert("host".to_string(), Value::String(host.to_string()));
    result.insert("reachable".to_string(), Value::Bool(received > 0));
    result.insert("method".to_string(), Value::String("tcp".to_string()));
    result.insert("permission_limited".to_string(), Value::Bool(false));
    result.insert("sent".to_string(), Value::Int(sent as i64));
    result.insert("received".to_string(), Value::Int(received as i64));
    result.insert("failed".to_string(), Value::Int(failed as i64));
    result.insert(
        "loss_percent".to_string(),
        Value::Float(if sent == 0 {
            0.0
        } else {
            (failed as f64 / sent as f64) * 100.0
        }),
    );
    result.insert(
        "ports_tried".to_string(),
        Value::Array(ports.iter().map(|p| Value::Int(*p as i64)).collect()),
    );
    result.insert(
        "attempts".to_string(),
        Value::Array(attempts.iter().map(tcp_attempt_to_value).collect()),
    );

    if let Some(first_success) = attempts.iter().find(|attempt| attempt.reachable) {
        result.insert(
            "connected_port".to_string(),
            first_success
                .connected_port
                .map(|port| Value::Int(port as i64))
                .unwrap_or_else(Value::none),
        );
        result.insert(
            "remote_addr".to_string(),
            first_success
                .remote_addr
                .clone()
                .map_or_else(Value::none, Value::String),
        );
        result.insert(
            "local_addr".to_string(),
            first_success
                .local_addr
                .clone()
                .map_or_else(Value::none, Value::String),
        );
    } else {
        result.insert("connected_port".to_string(), Value::none());
        if let Some(last_failure) = attempts.last() {
            result.insert(
                "reason".to_string(),
                Value::String(last_failure.reason.clone()),
            );
        }
    }

    if latency_count > 0 {
        let avg_ms = latency_sum / latency_count as f64;
        result.insert("min_ms".to_string(), Value::Float(min_ms));
        result.insert("avg_ms".to_string(), Value::Float(avg_ms));
        result.insert("max_ms".to_string(), Value::Float(max_ms));
        if sent == 1 {
            result.insert("latency_ms".to_string(), Value::Float(avg_ms));
        }
    }

    result
}

fn tcp_connect_result_map(
    host: &str,
    port: u16,
    attempts: &[TcpProbeResult],
) -> HashMap<String, Value> {
    let mut result = tcp_reachability_result_map(host, &[port], attempts);
    let connected = attempts.iter().any(|attempt| attempt.reachable);
    result.insert("port".to_string(), Value::Int(port as i64));
    result.insert("connected".to_string(), Value::Bool(connected));
    result
}

fn reachable_result_map(
    host: &str,
    ports: &[u16],
    attempts: &[TcpProbeResult],
    icmp_result: Result<HashMap<String, Value>, String>,
) -> HashMap<String, Value> {
    let tcp_reachable = attempts.iter().any(|attempt| attempt.reachable);
    let mut tcp_result = tcp_reachability_result_map(host, ports, attempts);
    tcp_result.insert(
        "fallback_from".to_string(),
        Value::String("icmp".to_string()),
    );

    let tcp_ports = Value::Array(ports.iter().map(|p| Value::Int(*p as i64)).collect());
    let tcp_attempts = Value::Array(attempts.iter().map(tcp_attempt_to_value).collect());

    match icmp_result {
        Ok(mut icmp_map) => {
            let icmp_reachable = map_bool(&icmp_map, "reachable").unwrap_or(false);
            if let Some(Value::Array(icmp_attempts)) = icmp_map.get("attempts").cloned() {
                tcp_result.insert("icmp_attempts".to_string(), Value::Array(icmp_attempts));
            }
            tcp_result.insert("icmp_reachable".to_string(), Value::Bool(icmp_reachable));

            if icmp_reachable {
                icmp_map.insert("tcp_reachable".to_string(), Value::Bool(tcp_reachable));
                icmp_map.insert("tcp_ports_tried".to_string(), tcp_ports);
                icmp_map.insert("tcp_attempts".to_string(), tcp_attempts);
                if let Some(first_success) = attempts.iter().find(|attempt| attempt.reachable) {
                    icmp_map.insert(
                        "connected_port".to_string(),
                        first_success
                            .connected_port
                            .map(|port| Value::Int(port as i64))
                            .unwrap_or_else(Value::none),
                    );
                }
                icmp_map
            } else {
                tcp_result
            }
        }
        Err(error) => {
            tcp_result.insert("icmp_error".to_string(), Value::String(error));
            tcp_result.insert("icmp_reachable".to_string(), Value::Bool(false));
            tcp_result
        }
    }
}

fn map_bool(map: &HashMap<String, Value>, key: &str) -> Option<bool> {
    match map.get(key) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn port_scan_result_to_value(result: PortScanResult) -> Value {
    let mut map = HashMap::new();
    map.insert("port".to_string(), Value::Int(result.port as i64));
    map.insert("open".to_string(), Value::Bool(result.open));
    map.insert("method".to_string(), Value::String("tcp".to_string()));
    if let Some(latency_ms) = result.latency_ms {
        map.insert("latency_ms".to_string(), Value::Float(latency_ms));
    }
    if let Some(remote_addr) = result.remote_addr {
        map.insert("remote_addr".to_string(), Value::String(remote_addr));
    }
    if let Some(local_addr) = result.local_addr {
        map.insert("local_addr".to_string(), Value::String(local_addr));
    }
    map.insert("reason".to_string(), Value::String(result.reason));
    Value::Map(map)
}

fn tcp_attempt_to_value(attempt: &TcpProbeResult) -> Value {
    let mut map = HashMap::new();
    map.insert("reachable".to_string(), Value::Bool(attempt.reachable));
    map.insert("method".to_string(), Value::String("tcp".to_string()));
    map.insert(
        "connected_port".to_string(),
        attempt
            .connected_port
            .map(|port| Value::Int(port as i64))
            .unwrap_or_else(Value::none),
    );
    if let Some(latency_ms) = attempt.latency_ms {
        map.insert("latency_ms".to_string(), Value::Float(latency_ms));
    }
    if let Some(remote_addr) = &attempt.remote_addr {
        map.insert(
            "remote_addr".to_string(),
            Value::String(remote_addr.clone()),
        );
    }
    if let Some(local_addr) = &attempt.local_addr {
        map.insert("local_addr".to_string(), Value::String(local_addr.clone()));
    }
    if !attempt.reachable {
        map.insert("reason".to_string(), Value::String(attempt.reason.clone()));
    }
    Value::Map(map)
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

fn parsed_to_map(input: &str, parsed: &ParsedInput) -> HashMap<String, Value> {
    let network = &parsed.network;
    let ip = ip_to_string(network.family, parsed.original_ip);
    let network_ip = ip_to_string(network.family, network.network);
    let last = ip_to_string(network.family, network.last());

    let mut map = HashMap::new();
    map.insert("input".to_string(), Value::String(input.to_string()));
    map.insert("kind".to_string(), Value::String(parsed.kind.to_string()));
    map.insert("version".to_string(), Value::Int(network.family.version()));
    map.insert("ip".to_string(), Value::String(ip));
    map.insert("prefix".to_string(), Value::Int(network.prefix as i64));
    map.insert("network".to_string(), Value::String(network_ip.clone()));
    map.insert("first".to_string(), Value::String(network_ip));
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
    is_broadcast: bool,
    is_metadata_endpoint: bool,
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

fn parse_string_option<'a>(
    opts: Option<&'a HashMap<String, Value>>,
    key: &str,
) -> Result<Option<&'a str>, String> {
    match opts.and_then(|m| m.get(key)) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.as_str())),
        Some(other) => Err(format!(
            "option '{}' must be String, got {}",
            key,
            other.type_name()
        )),
    }
}

fn parse_tls_port(opts: Option<&HashMap<String, Value>>) -> Result<u16, String> {
    match opts.and_then(|m| m.get("port")) {
        None => Ok(443),
        Some(Value::Int(port)) => validate_tcp_port_arg(*port, "tls_info"),
        Some(other) => Err(format!(
            "option 'port' must be Int, got {}",
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

fn parse_count_option(opts: Option<&HashMap<String, Value>>) -> Result<usize, String> {
    match opts.and_then(|m| m.get("count")) {
        None => Ok(DEFAULT_PROBE_COUNT),
        Some(Value::Int(value)) if *value > 0 => {
            let count = usize::try_from(*value).map_err(|_| "option 'count' is too large")?;
            Ok(min(count, MAX_PROBE_COUNT))
        }
        Some(Value::Int(_)) => Err("option 'count' must be positive".to_string()),
        Some(other) => Err(format!(
            "option 'count' must be Int, got {}",
            other.type_name()
        )),
    }
}

fn parse_port_scan_concurrency(opts: Option<&HashMap<String, Value>>) -> Result<usize, String> {
    match opts.and_then(|m| m.get("concurrency")) {
        None => Ok(DEFAULT_PORT_SCAN_CONCURRENCY),
        Some(Value::Int(value)) if *value > 0 => {
            let concurrency = usize::try_from(*value)
                .map_err(|_| "option 'concurrency' is too large".to_string())?;
            Ok(min(concurrency, MAX_PORT_SCAN_CONCURRENCY))
        }
        Some(Value::Int(_)) => Err("option 'concurrency' must be positive".to_string()),
        Some(other) => Err(format!(
            "option 'concurrency' must be Int, got {}",
            other.type_name()
        )),
    }
}

fn parse_explicit_ports(values: &[Value], fn_name: &str, label: &str) -> Result<Vec<u16>, String> {
    if values.is_empty() {
        return Err(format!("{}() {} cannot be empty", fn_name, label));
    }
    if values.len() > MAX_PORT_SCAN_PORTS {
        return Err(format!(
            "{}() {} supports at most {} ports",
            fn_name, label, MAX_PORT_SCAN_PORTS
        ));
    }

    let mut ports = Vec::with_capacity(values.len());
    let mut seen = HashSet::with_capacity(values.len());
    for value in values {
        let Value::Int(port) = value else {
            return Err(format!("{}() {} must be Array<Int>", fn_name, label));
        };
        let port = validate_tcp_port_arg(*port, fn_name)?;
        if !seen.insert(port) {
            return Err(format!(
                "{}() duplicate port {} is not allowed",
                fn_name, port
            ));
        }
        ports.push(port);
    }
    Ok(ports)
}

fn parse_reachable_tcp_ports(
    opts: Option<&HashMap<String, Value>>,
    fn_name: &str,
) -> Result<Vec<u16>, String> {
    let extra_ports: &[Value] = match opts.and_then(|m| m.get("tcp_ports")) {
        Some(Value::Array(values)) => values.as_slice(),
        Some(_) => {
            return Err(format!(
                "{}() option 'tcp_ports' must be Array<Int>",
                fn_name
            ));
        }
        None => &[],
    };

    let mut ports = DEFAULT_REACHABLE_TCP_PORTS.to_vec();
    if ports.len() + extra_ports.len() > MAX_TCP_PORTS {
        return Err(format!(
            "{}() option 'tcp_ports' supports at most {} total ports including defaults 80 and 443",
            fn_name, MAX_TCP_PORTS
        ));
    }
    for value in extra_ports {
        let Value::Int(port) = value else {
            return Err(format!(
                "{}() option 'tcp_ports' must be Array<Int>",
                fn_name
            ));
        };
        let port = validate_tcp_port_arg(*port, fn_name)?;
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

    fn assert_result_err_contains(value: Value, expected: &str) {
        match value {
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } => {
                assert_eq!(enum_name, "Result");
                assert_eq!(variant, "Err");
                assert!(
                    matches!(values.as_slice(), [Value::String(message)] if message.contains(expected)),
                    "expected Result::Err message to contain {expected:?}, got {values:?}"
                );
            }
            other => panic!("expected Result::Err, got {other:?}"),
        }
    }

    #[test]
    fn icmp_echo_request_sets_checksum_and_identifiers() {
        let packet = build_icmp_echo_request(
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
        let err =
            build_icmp_echo_request(IpAddr::V6(Ipv6Addr::LOCALHOST), None, 1, 1, &[]).unwrap_err();
        assert!(err.contains("local IPv6 address"));
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
                Some(IcmpProbeEvent::TargetFailure(message)) => {
                    assert!(message.contains(&format!("type {icmp_type}")));
                }
                other => panic!("expected ICMPv6 type {icmp_type} target failure, got {other:?}"),
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

        let err = icmp_ping("example.com", &[], &options).unwrap_err();
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
    fn ping_attempt_budget_rejects_impossible_count_interval() {
        let err = ping_attempt_budget(Duration::from_millis(100), 3, Duration::from_millis(60))
            .unwrap_err();
        assert!(err.contains("timeout_ms is too small"));
    }

    #[test]
    fn icmp_connection_refused_is_target_failure() {
        // Linux DGRAM ICMP sockets surface a pending destination-unreachable
        // error on recv as ECONNREFUSED; it must classify as a target failure
        // both as a typed io::Error and after being stringified.
        let io_err = std::io::Error::from(ErrorKind::ConnectionRefused);
        assert!(icmp_io_error_is_target_failure(&io_err));
        assert!(icmp_probe_error_is_target_failure(&format!(
            "ICMP ping failed: {}",
            io_err
        )));
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
        let mapped_broadcast = "[::ffff:255.255.255.255]:80".parse::<SocketAddr>().unwrap();

        assert!(enforce_resolved_target_policy(&[(80, multicast)], false).is_err());
        assert!(enforce_resolved_target_policy(&[(80, documentation)], false).is_err());
        assert!(enforce_resolved_target_policy(&[(80, broadcast)], false).is_err());
        assert!(enforce_resolved_target_policy(&[(80, mapped_broadcast)], false).is_err());
    }

    #[test]
    fn target_policy_never_allows_metadata_or_special_ranges() {
        let metadata = "169.254.169.254:80".parse::<SocketAddr>().unwrap();
        let ecs_metadata = "169.254.170.2:80".parse::<SocketAddr>().unwrap();
        let mapped_metadata = "[::ffff:169.254.169.254]:80".parse::<SocketAddr>().unwrap();
        let mapped_ecs_metadata = "[::ffff:169.254.170.2]:80".parse::<SocketAddr>().unwrap();
        let multicast = "224.0.0.1:80".parse::<SocketAddr>().unwrap();
        let documentation = "192.0.2.1:80".parse::<SocketAddr>().unwrap();
        let broadcast = "255.255.255.255:80".parse::<SocketAddr>().unwrap();

        for target in [
            metadata,
            ecs_metadata,
            mapped_metadata,
            mapped_ecs_metadata,
            multicast,
            documentation,
            broadcast,
        ] {
            let err = enforce_resolved_target_policy(&[(80, target)], true).unwrap_err();
            assert!(err.contains("special-purpose targets are not allowed"));
        }
    }

    #[test]
    fn target_policy_checks_all_resolved_addresses_before_probe() {
        let public = "93.184.216.34:443".parse::<SocketAddr>().unwrap();
        let private = "127.0.0.1:443".parse::<SocketAddr>().unwrap();
        let targets = [(443, public), (443, private)];

        let err = enforce_resolved_target_policy(&targets, false).unwrap_err();
        assert!(err.contains("Network target denied by policy"));
    }

    #[test]
    fn resolve_ping_targets_accepts_ipv6_literals() {
        let targets = resolve_ping_targets("::1").unwrap();
        assert!(targets.iter().any(|(_, addr)| addr.ip().is_ipv6()));
    }

    #[test]
    fn ping_tcp_method_points_to_tcp_connect() {
        let result = ping_fn(&[
            Value::String("example.com".to_string()),
            Value::Map(HashMap::from([(
                "method".to_string(),
                Value::String("tcp".to_string()),
            )])),
        ])
        .unwrap();

        assert_result_err_contains(result, "use tcp_connect()");
    }

    #[test]
    fn ping_count_is_positive_and_clamped() {
        assert_eq!(parse_count_option(None).unwrap(), DEFAULT_PROBE_COUNT);
        assert_eq!(
            parse_count_option(Some(&HashMap::from([(
                "count".to_string(),
                Value::Int((MAX_PROBE_COUNT as i64) + 50),
            )])))
            .unwrap(),
            MAX_PROBE_COUNT
        );
        assert!(parse_count_option(Some(&HashMap::from([
            ("count".to_string(), Value::Int(0),)
        ])))
        .is_err());
    }

    #[test]
    fn reachable_tcp_ports_default_to_http_https_and_append_extras() {
        let defaults = parse_reachable_tcp_ports(None, "reachable").unwrap();
        assert_eq!(defaults, vec![80, 443]);

        let ports = parse_reachable_tcp_ports(
            Some(&HashMap::from([(
                "tcp_ports".to_string(),
                Value::Array(vec![Value::Int(8080), Value::Int(443), Value::Int(8443)]),
            )])),
            "reachable",
        )
        .unwrap();
        assert_eq!(ports, vec![80, 443, 8080, 8443]);
    }

    #[test]
    fn port_scan_concurrency_is_positive_and_clamped() {
        assert_eq!(
            parse_port_scan_concurrency(None).unwrap(),
            DEFAULT_PORT_SCAN_CONCURRENCY
        );
        assert_eq!(
            parse_port_scan_concurrency(Some(&HashMap::from([(
                "concurrency".to_string(),
                Value::Int((MAX_PORT_SCAN_CONCURRENCY as i64) + 99),
            )])))
            .unwrap(),
            MAX_PORT_SCAN_CONCURRENCY
        );
        assert!(parse_port_scan_concurrency(Some(&HashMap::from([(
            "concurrency".to_string(),
            Value::Int(0),
        )])))
        .is_err());
    }

    #[test]
    fn port_scan_ports_reject_duplicates_and_preserve_order() {
        let ports = parse_explicit_ports(
            &[Value::Int(443), Value::Int(80), Value::Int(22)],
            "port_scan",
            "argument 2",
        )
        .unwrap();
        assert_eq!(ports, vec![443, 80, 22]);

        let err =
            parse_explicit_ports(&[Value::Int(80), Value::Int(80)], "port_scan", "argument 2")
                .unwrap_err();
        assert!(err.contains("duplicate port 80"));
    }

    #[test]
    fn dns_record_type_parser_accepts_supported_lookup_types() {
        for record_type in [
            "a",
            "AAAA",
            "aname",
            "CAA",
            "CDNSKEY",
            "CDS",
            "CNAME",
            "CSYNC",
            "DNSKEY",
            "DS",
            "HINFO",
            "HTTPS",
            "KEY",
            "MX",
            "NAPTR",
            "NS",
            "NSEC",
            "NSEC3",
            "NSEC3PARAM",
            "NULL",
            "OPENPGPKEY",
            "PTR",
            "RRSIG",
            "SIG",
            "SOA",
            "SRV",
            "SSHFP",
            "SVCB",
            "TLSA",
            "TXT",
        ] {
            assert_eq!(
                DnsRecordType::parse(record_type).unwrap().as_str(),
                record_type.to_ascii_uppercase()
            );
        }
    }

    #[test]
    fn dns_record_type_parser_rejects_meta_and_unknown_types() {
        for record_type in ["ANY", "AXFR", "IXFR", "OPT", "TSIG", "ZERO", "BOGUS"] {
            assert!(DnsRecordType::parse(record_type)
                .unwrap_err()
                .contains("record_type must be one of"));
        }
    }

    #[test]
    fn dns_answers_render_stable_record_maps() {
        let value = dns_answers_to_value(vec![
            DnsAnswer {
                record_type: "A".to_string(),
                name: "example.com.".to_string(),
                value: "93.184.216.34".to_string(),
                ttl: 300,
            },
            DnsAnswer {
                record_type: "MX".to_string(),
                name: "example.com.".to_string(),
                value: "10 mail.example.com.".to_string(),
                ttl: 600,
            },
        ]);

        let Value::Array(records) = value else {
            panic!("expected DNS records array");
        };
        assert_eq!(records.len(), 2);
        let Value::Map(record) = &records[0] else {
            panic!("expected DNS record map");
        };
        assert!(matches!(record.get("type"), Some(Value::String(value)) if value == "A"));
        assert!(
            matches!(record.get("name"), Some(Value::String(value)) if value == "example.com.")
        );
        assert!(
            matches!(record.get("value"), Some(Value::String(value)) if value == "93.184.216.34")
        );
        assert!(matches!(record.get("ttl"), Some(Value::Int(300))));

        let Value::Map(record) = &records[1] else {
            panic!("expected DNS record map");
        };
        assert!(matches!(record.get("type"), Some(Value::String(value)) if value == "MX"));
        assert!(
            matches!(record.get("value"), Some(Value::String(value)) if value == "10 mail.example.com.")
        );
        assert!(matches!(record.get("ttl"), Some(Value::Int(600))));
    }

    #[test]
    fn tcp_reachability_attempts_return_per_attempt_summary() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let count = 3;
        let accept_thread = thread::spawn(move || {
            for _ in 0..count {
                let _ = listener.accept();
            }
        });

        let attempts = tcp_reachability_attempts(
            &[(addr.port(), addr)],
            Duration::from_millis(500),
            count,
            Duration::ZERO,
        );
        accept_thread.join().unwrap();

        assert_eq!(attempts.len(), count);
        assert!(attempts.iter().all(|attempt| attempt.reachable));

        let summary = tcp_reachability_result_map("127.0.0.1", &[addr.port()], &attempts);
        assert!(matches!(summary.get("sent"), Some(Value::Int(3))));
        assert!(matches!(summary.get("received"), Some(Value::Int(3))));
        assert!(matches!(summary.get("failed"), Some(Value::Int(0))));
        assert!(matches!(summary.get("loss_percent"), Some(Value::Float(value)) if *value == 0.0));
        assert!(matches!(summary.get("attempts"), Some(Value::Array(values)) if values.len() == 3));
    }
}
