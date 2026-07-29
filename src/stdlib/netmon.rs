//! Bounded network-monitoring primitives.
//!
//! `std/netmon` builds monitoring-oriented protocol support on top of the
//! outbound target policy shared with `std/net`. Credentials remain opaque
//! `Secret` values and are exposed only to the protocol sink that needs them.

#[path = "netmon_codec.rs"]
mod codec;

use self::codec::{
    decode_response, decode_response_allow_stale, encode_get_next_request, encode_get_request,
    DecodedValue,
};
use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use crate::stdlib::net::enforce_resolved_target_policy;
use std::collections::{HashMap, HashSet};
use std::env;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

const DEFAULT_SNMP_PORT: u16 = 161;
const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const MIN_TIMEOUT_MS: u64 = 50;
const MAX_TIMEOUT_MS: u64 = 30_000;
const MAX_RETRIES: u64 = 3;
const MAX_ATTEMPTS: usize = 4;
const MAX_TARGET_BYTES: usize = 64;
const MAX_COMMUNITY_BYTES: usize = 255;
const MAX_OIDS: usize = 64;
const MAX_OID_BYTES: usize = 255;
const MAX_OID_SEGMENTS: usize = 128;
const MAX_REQUEST_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 8 * 1024;
const DEFAULT_MAX_RESULTS: usize = 256;
const MAX_WALK_RESULTS: usize = 2_048;
const MAX_WALK_OPERATIONS: usize = 4_096;
const MAX_WALK_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MAX_WALK_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
struct SnmpOptions {
    port: u16,
    timeout: Duration,
    retries: usize,
    allow_private: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnLimit {
    Error,
    Partial,
}

#[derive(Debug, Clone, Copy)]
struct SnmpWalkOptions {
    common: SnmpOptions,
    max_results: usize,
    on_limit: OnLimit,
}

struct SnmpV2cAuth<'a> {
    community: &'a str,
}

#[derive(Debug)]
struct ParsedOid {
    arcs: Vec<u32>,
    canonical: String,
}

pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt snmp_get
    // @module std/netmon
    // @module_description Bounded network-monitoring protocols with opaque credentials and normalized results
    // @signature snmp_get(target: String, auth: Map, oids: Array<String>, opts?: Map) -> Result<Map, String>
    // Reads one bounded set of numeric OIDs from an SNMP agent. Slice 1 supports
    // SNMPv2c only. The strict auth map must contain `version: "2c"` and a
    // `community` Secret, normally returned by `require_secret()`; plaintext
    // community strings are rejected. Unknown auth and option keys are rejected.
    // SNMPv2c does not encrypt or authenticate its community or payload; use this
    // slice only on trusted management networks or protected tunnels.
    //
    // `NTNT_NETMON_ENABLE=1` is required for every call. Slice 1 accepts literal
    // IPv4 and IPv6 targets only, eliminating unbounded hostname resolution.
    // Private/internal targets additionally require `NTNT_NET_ALLOW_PRIVATE=1`
    // and `allow_private: true`; special-purpose targets remain denied.
    // `timeout_ms` is one global budget across request encoding, UDP send/receive,
    // and retries. Encoded requests are capped at 8 KiB; responses are capped at
    // 8 KiB and must be complete, strict SNMPv2c BER with exactly the requested
    // OIDs in order.
    // @param target Literal IPv4 or IPv6 address without a port
    // @param auth Strict map with version (`"2c"`) and community (Secret)
    // @param oids One to 64 unique numeric OIDs
    // @param opts Optional strict map with port (default 161), timeout_ms (default 2000), retries (default 0, max 3), and allow_private
    // @returns Result containing target, checked address, port, version, duration_ms, attempts, and normalized values
    // @error TypeError ~ "snmp_get() argument 1 must be String" fix: "Pass an IPv4 or IPv6 literal"
    // @error RuntimeError ~ "std/netmon is disabled" fix: "Set NTNT_NETMON_ENABLE=1 for the process"
    // @error RuntimeError ~ "snmp_get() auth.community must be Secret" fix: "Load the community with std/secrets.require_secret()"
    // @see_also require_secret, net_capabilities
    // @since v0.5.2
    // @tags #network, #monitoring, #snmp, #security
    // @example snmp_get("10.0.50.1", map { "version": "2c", "community": require_secret("SNMP_COMMUNITY") }, ["1.3.6.1.2.1.1.1.0"], map { "allow_private": true }) ~ "Read sysDescr using an opaque community"
    module.insert(
        "snmp_get".to_string(),
        Value::NativeFunction {
            name: "snmp_get".to_string(),
            arity: 3,
            max_arity: 4,
            requires: None,
            func: snmp_get_fn,
        },
    );

    // @ntnt snmp_walk
    // @module std/netmon
    // @signature snmp_walk(target: String, auth: Map, oid: String, opts?: Map) -> Result<Map, String>
    // Walks one numeric OID subtree with bounded SNMPv2c GETNEXT requests. The
    // community must be an opaque Secret. SNMPv2c is plaintext; use a trusted
    // management network or protected tunnel. Every call requires
    // `NTNT_NETMON_ENABLE=1`; private targets also require the shared process
    // opt-in and `allow_private: true`.
    //
    // The closed options map accepts the common SNMP options plus `max_results`
    // (default 256, hard maximum 2048) and `on_limit` (`"error"`, the default,
    // or `"partial"`). One global deadline covers all cursors, retries, response
    // validation, the mandatory limit look-ahead, normalization, and result
    // construction. Requests and responses are capped at 8 KiB, cumulative
    // received bytes at 8 MiB, and conservative normalized output at 4 MiB.
    //
    // A successful map has exactly: `target: String`, `address: String`,
    // `port: Int`, `version: String` (`"2c"`), `root_oid: String`,
    // `duration_ms: Int`, `requests: Int`, `attempts: Int`, `complete: Bool`,
    // `stop_reason: String`, and `values: Array<Map>`. Each value uses the same
    // normalized `oid`, `type`, `value`, and optional `encoding` fields as
    // `snmp_get`. `stop_reason` is one of `out_of_subtree`, `end_of_mib_view`,
    // `no_such_object`, `no_such_instance`, or `max_results`. Only
    // `max_results` has `complete: false`, and it is returned only when
    // `on_limit: "partial"`; all transport, protocol, ordering, deadline, and
    // byte-budget failures are `Err(String)` without prior-row telemetry.
    // @param target Literal IPv4 or IPv6 address without a port
    // @param auth Strict map with version (`"2c"`) and community (Secret)
    // @param oid Numeric root OID in dotted notation
    // @param opts Optional strict map with port, timeout_ms, retries, allow_private, max_results, and on_limit
    // @returns Result whose Ok map has exactly target, address, port, version, root_oid, duration_ms, requests, attempts, complete, stop_reason, and values
    // @error RuntimeError ~ "std/netmon is disabled" fix: "Set NTNT_NETMON_ENABLE=1 for the process"
    // @error RuntimeError ~ "snmp_walk() auth.community must be Secret" fix: "Load it with std/secrets.require_secret()"
    // @see_also snmp_get, require_secret, net_capabilities
    // @since v0.5.3
    // @tags #network, #monitoring, #snmp, #security
    // @example snmp_walk("10.0.50.1", map { "version": "2c", "community": require_secret("SNMP_COMMUNITY") }, "1.3.6.1.2.1.2.2", map { "allow_private": true, "max_results": 128 }) ~ "Walk a bounded interface table"
    module.insert(
        "snmp_walk".to_string(),
        Value::NativeFunction {
            name: "snmp_walk".to_string(),
            arity: 3,
            max_arity: 4,
            requires: None,
            func: snmp_walk_fn,
        },
    );

    module
}

fn snmp_walk_fn(args: &[Value]) -> Result<Value> {
    let target = expect_string(&args[0], "snmp_walk() argument 1")?;
    let auth = expect_map(&args[1], "snmp_walk() argument 2")?;
    let oid = expect_string(&args[2], "snmp_walk() argument 3")?;
    let opts = match args.get(3) {
        Some(value) => Some(expect_map(value, "snmp_walk() argument 4")?),
        None => None,
    };

    Ok(match snmp_walk(target, auth, oid, opts) {
        Ok(result) => Value::ok(Value::Map(result)),
        Err(error) => Value::err(Value::String(error)),
    })
}

struct WalkReply {
    oid: Vec<u32>,
    value: Value,
    terminal_reason: Option<&'static str>,
    normalized_size: usize,
}

fn snmp_walk(
    target: &str,
    auth: &HashMap<String, Value>,
    oid: &str,
    opts: Option<&HashMap<String, Value>>,
) -> std::result::Result<HashMap<String, Value>, String> {
    require_netmon_enabled()?;
    let target_ip = parse_target(target).map_err(as_walk_error)?;
    let auth = parse_v2c_auth(auth).map_err(as_walk_error)?;
    let options = parse_walk_options(opts).map_err(as_walk_error)?;
    let root = parse_single_oid(oid).map_err(as_walk_error)?;
    let address = SocketAddr::new(target_ip, options.common.port);
    enforce_resolved_target_policy(
        &[(options.common.port, address)],
        options.common.allow_private,
    )?;

    let started = Instant::now();
    let deadline = started + options.common.timeout;
    let mut cursor = root.arcs.clone();
    let mut values = Vec::with_capacity(options.max_results);
    let mut requests = 0usize;
    let mut attempts = 0usize;
    let mut received_bytes = 0usize;
    let mut output_bytes = conservative_walk_envelope_size(target, address, &root.canonical);

    loop {
        remaining_until(deadline, "SNMP WALK global timeout expired")?;
        requests += 1;
        let reply = walk_cursor_request(
            address,
            auth.community.as_bytes(),
            &cursor,
            options.common.retries,
            deadline,
            &mut attempts,
            &mut received_bytes,
        )?;

        if let Some(reason) = reply.terminal_reason {
            return finish_walk(
                target,
                address,
                &root.canonical,
                started,
                deadline,
                requests,
                attempts,
                true,
                reason,
                values,
            );
        }
        if !oid_is_in_subtree(&reply.oid, &root.arcs) {
            return finish_walk(
                target,
                address,
                &root.canonical,
                started,
                deadline,
                requests,
                attempts,
                true,
                "out_of_subtree",
                values,
            );
        }
        if values.len() == options.max_results {
            return match options.on_limit {
                OnLimit::Error => Err(format!(
                    "SNMP WALK exceeded max_results ({})",
                    options.max_results
                )),
                OnLimit::Partial => finish_walk(
                    target,
                    address,
                    &root.canonical,
                    started,
                    deadline,
                    requests,
                    attempts,
                    false,
                    "max_results",
                    values,
                ),
            };
        }
        output_bytes = add_walk_budget(
            output_bytes,
            reply.normalized_size,
            MAX_WALK_OUTPUT_BYTES,
            "normalized output",
        )?;
        values.push(reply.value);
        cursor = reply.oid;
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_cursor_request(
    address: SocketAddr,
    community: &[u8],
    cursor: &[u32],
    retries: usize,
    deadline: Instant,
    attempts: &mut usize,
    received_bytes: &mut usize,
) -> std::result::Result<WalkReply, String> {
    let attempt_limit = retries.saturating_add(1).clamp(1, MAX_ATTEMPTS);
    let mut last_error = "request failed".to_string();

    for attempt in 0..attempt_limit {
        let remaining = remaining_until(deadline, "SNMP WALK global timeout expired")?;
        let remaining_attempts = attempt_limit - attempt;
        let attempt_budget = remaining / remaining_attempts as u32;
        let attempt_deadline = Instant::now()
            .checked_add(attempt_budget)
            .map(|candidate| candidate.min(deadline))
            .unwrap_or(deadline);
        let request_id = rand::random::<i32>() & i32::MAX;
        let request = encode_bounded_get_next_request(request_id, community, cursor)?;
        *attempts += 1;

        let packet = match send_walk_and_receive(
            address,
            request,
            attempt_deadline,
            request_id,
            community,
            received_bytes,
        ) {
            Ok(packet) => packet,
            Err(error) => {
                if error.starts_with("SNMP WALK cumulative responses") {
                    return Err(error);
                }
                last_error = error;
                continue;
            }
        };
        let response = match decode_response(packet.as_slice(), request_id, community) {
            Ok(response) => response,
            Err(error) => {
                last_error = error;
                continue;
            }
        };
        if response.error_status != 0 {
            return Err(format!(
                "SNMP agent returned error status {} at varbind index {}",
                response.error_status, response.error_index
            ));
        }
        if response.varbinds.len() != 1 {
            return Err(format!(
                "SNMP WALK response returned {} varbind(s), expected exactly 1",
                response.varbinds.len()
            ));
        }
        let varbind = response
            .varbinds
            .into_iter()
            .next()
            .expect("checked one varbind");
        let terminal_reason = match &varbind.value {
            DecodedValue::EndOfMibView => Some("end_of_mib_view"),
            DecodedValue::NoSuchObject => Some("no_such_object"),
            DecodedValue::NoSuchInstance => Some("no_such_instance"),
            _ => None,
        };
        if terminal_reason.is_some() && varbind.oid.as_slice() != cursor {
            return Err(format!(
                "SNMP WALK terminal exception OID {} must match cursor {}",
                format_oid(&varbind.oid),
                format_oid(cursor)
            ));
        }
        if terminal_reason.is_none() && varbind.oid.as_slice() <= cursor {
            return Err(format!(
                "SNMP WALK response OID {} must be strictly increasing after {}",
                format_oid(&varbind.oid),
                format_oid(cursor)
            ));
        }
        let canonical = format_oid(&varbind.oid);
        let value = normalize_varbind(&canonical, varbind.value)?;
        let normalized_size = conservative_value_size(&value);
        remaining_until(
            deadline,
            "SNMP WALK global timeout expired while decoding response",
        )?;
        return Ok(WalkReply {
            oid: varbind.oid,
            value,
            terminal_reason,
            normalized_size,
        });
    }

    Err(format!(
        "SNMP WALK request failed after {attempt_limit} bounded attempt(s): {last_error}"
    ))
}

fn encode_bounded_get_next_request(
    request_id: i32,
    community: &[u8],
    oid: &[u32],
) -> std::result::Result<Zeroizing<Vec<u8>>, String> {
    let request = encode_get_next_request(request_id, community, oid)?;
    if request.len() > MAX_REQUEST_BYTES {
        return Err(format!(
            "snmp_walk() encoded request exceeds {MAX_REQUEST_BYTES} bytes"
        ));
    }
    Ok(request)
}

#[allow(clippy::too_many_arguments)]
fn finish_walk(
    target: &str,
    address: SocketAddr,
    root_oid: &str,
    started: Instant,
    deadline: Instant,
    requests: usize,
    attempts: usize,
    complete: bool,
    stop_reason: &str,
    values: Vec<Value>,
) -> std::result::Result<HashMap<String, Value>, String> {
    remaining_until(
        deadline,
        "SNMP WALK global timeout expired before result construction",
    )?;
    let mut result = HashMap::new();
    result.insert("target".to_string(), Value::String(target.to_string()));
    result.insert(
        "address".to_string(),
        Value::String(address.ip().to_string()),
    );
    result.insert("port".to_string(), Value::Int(i64::from(address.port())));
    result.insert("version".to_string(), Value::String("2c".to_string()));
    result.insert("root_oid".to_string(), Value::String(root_oid.to_string()));
    result.insert("duration_ms".to_string(), Value::Int(elapsed_ms(started)));
    result.insert("requests".to_string(), Value::Int(requests as i64));
    result.insert("attempts".to_string(), Value::Int(attempts as i64));
    result.insert("complete".to_string(), Value::Bool(complete));
    result.insert(
        "stop_reason".to_string(),
        Value::String(stop_reason.to_string()),
    );
    result.insert("values".to_string(), Value::Array(values));
    let final_output_size = conservative_map_size(&result);
    if final_output_size > MAX_WALK_OUTPUT_BYTES {
        return Err(format!(
            "SNMP WALK normalized output exceeds {MAX_WALK_OUTPUT_BYTES} bytes"
        ));
    }
    remaining_until(
        deadline,
        "SNMP WALK global timeout expired while constructing result",
    )?;
    Ok(result)
}

fn conservative_walk_envelope_size(target: &str, address: SocketAddr, root_oid: &str) -> usize {
    let envelope = HashMap::from([
        ("target".to_string(), Value::String(target.to_string())),
        (
            "address".to_string(),
            Value::String(address.ip().to_string()),
        ),
        ("port".to_string(), Value::Int(i64::from(address.port()))),
        ("version".to_string(), Value::String("2c".to_string())),
        ("root_oid".to_string(), Value::String(root_oid.to_string())),
        ("duration_ms".to_string(), Value::Int(0)),
        ("requests".to_string(), Value::Int(0)),
        ("attempts".to_string(), Value::Int(0)),
        ("complete".to_string(), Value::Bool(false)),
        (
            "stop_reason".to_string(),
            Value::String("no_such_instance".to_string()),
        ),
        ("values".to_string(), Value::Array(Vec::new())),
    ]);
    conservative_map_size(&envelope)
}

fn oid_is_in_subtree(oid: &[u32], root: &[u32]) -> bool {
    oid.starts_with(root)
}

fn conservative_value_size(value: &Value) -> usize {
    match value {
        Value::String(value) => value.len().saturating_add(32),
        Value::Array(values) => values.iter().fold(32usize, |size, value| {
            size.saturating_add(conservative_value_size(value))
        }),
        Value::Map(values) => conservative_map_size(values),
        _ => 64,
    }
}

fn conservative_map_size(values: &HashMap<String, Value>) -> usize {
    values.iter().fold(64usize, |size, (key, value)| {
        size.saturating_add(key.len())
            .saturating_add(conservative_value_size(value))
            .saturating_add(32)
    })
}

fn add_walk_budget(
    current: usize,
    amount: usize,
    maximum: usize,
    label: &str,
) -> std::result::Result<usize, String> {
    let total = current
        .checked_add(amount)
        .ok_or_else(|| format!("SNMP WALK {label} budget overflow"))?;
    if total > maximum {
        return Err(format!("SNMP WALK {label} exceed {maximum} bytes"));
    }
    Ok(total)
}

fn parse_walk_options(
    opts: Option<&HashMap<String, Value>>,
) -> std::result::Result<SnmpWalkOptions, String> {
    let empty = HashMap::new();
    let opts = opts.unwrap_or(&empty);
    reject_unknown_keys(
        opts,
        &[
            "port",
            "timeout_ms",
            "retries",
            "allow_private",
            "max_results",
            "on_limit",
        ],
        "options",
    )?;
    let port = parse_bounded_int(
        opts,
        "port",
        i64::from(DEFAULT_SNMP_PORT),
        1,
        i64::from(u16::MAX),
    )? as u16;
    let timeout_ms = parse_bounded_int(
        opts,
        "timeout_ms",
        DEFAULT_TIMEOUT_MS as i64,
        MIN_TIMEOUT_MS as i64,
        MAX_TIMEOUT_MS as i64,
    )? as u64;
    let retries = parse_bounded_int(opts, "retries", 0, 0, MAX_RETRIES as i64)? as usize;
    let allow_private = match opts.get("allow_private") {
        Some(Value::Bool(value)) => *value,
        Some(value) => {
            return Err(format!(
                "snmp_walk() options.allow_private must be Bool, got {}",
                value.type_name()
            ))
        }
        None => false,
    };
    let max_results = parse_bounded_int(
        opts,
        "max_results",
        DEFAULT_MAX_RESULTS as i64,
        1,
        MAX_WALK_RESULTS as i64,
    )? as usize;
    let on_limit = match opts.get("on_limit") {
        Some(Value::String(value)) if value == "error" => OnLimit::Error,
        Some(Value::String(value)) if value == "partial" => OnLimit::Partial,
        Some(Value::String(_)) => {
            return Err("snmp_walk() options.on_limit must be 'error' or 'partial'".to_string())
        }
        Some(value) => {
            return Err(format!(
                "snmp_walk() options.on_limit must be String, got {}",
                value.type_name()
            ))
        }
        None => OnLimit::Error,
    };
    let operations = max_results
        .checked_add(1)
        .and_then(|requests| requests.checked_mul(retries + 1))
        .ok_or_else(|| "snmp_walk() operation budget overflow".to_string())?;
    if operations > MAX_WALK_OPERATIONS {
        return Err(format!(
            "snmp_walk() requires {operations} possible attempts; maximum is {MAX_WALK_OPERATIONS}"
        ));
    }
    Ok(SnmpWalkOptions {
        common: SnmpOptions {
            port,
            timeout: Duration::from_millis(timeout_ms),
            retries,
            allow_private,
        },
        max_results,
        on_limit,
    })
}

fn parse_single_oid(raw: &str) -> std::result::Result<ParsedOid, String> {
    parse_oids(&[Value::String(raw.to_string())]).and_then(|mut parsed| {
        parsed
            .pop()
            .ok_or_else(|| "snmp_walk() oid is required".to_string())
    })
}

fn as_walk_error(error: String) -> String {
    error.replace("snmp_get()", "snmp_walk()")
}

fn snmp_get_fn(args: &[Value]) -> Result<Value> {
    let target = expect_string(&args[0], "snmp_get() argument 1")?;
    let auth = expect_map(&args[1], "snmp_get() argument 2")?;
    let oid_values = expect_array(&args[2], "snmp_get() argument 3")?;
    let opts = match args.get(3) {
        Some(value) => Some(expect_map(value, "snmp_get() argument 4")?),
        None => None,
    };

    Ok(match snmp_get(target, auth, oid_values, opts) {
        Ok(result) => Value::ok(Value::Map(result)),
        Err(error) => Value::err(Value::String(error)),
    })
}

fn snmp_get(
    target: &str,
    auth: &HashMap<String, Value>,
    oid_values: &[Value],
    opts: Option<&HashMap<String, Value>>,
) -> std::result::Result<HashMap<String, Value>, String> {
    require_netmon_enabled()?;
    let target_ip = parse_target(target)?;
    let auth = parse_v2c_auth(auth)?;
    let options = parse_options(opts)?;
    let parsed_oids = parse_oids(oid_values)?;
    let address = SocketAddr::new(target_ip, options.port);
    enforce_resolved_target_policy(&[(options.port, address)], options.allow_private)?;

    let started = Instant::now();
    let deadline = started + options.timeout;
    let attempt_limit = options.retries.saturating_add(1).clamp(1, MAX_ATTEMPTS);
    let oid_arcs: Vec<Vec<u32>> = parsed_oids.iter().map(|oid| oid.arcs.clone()).collect();
    let mut last_error = "request failed".to_string();
    let mut attempts_made = 0usize;

    for attempt in 0..attempt_limit {
        let remaining = remaining_until(deadline, "global timeout expired")?;
        let remaining_attempts = attempt_limit - attempt;
        let attempt_budget = remaining / remaining_attempts as u32;
        let attempt_deadline = Instant::now()
            .checked_add(attempt_budget)
            .map(|candidate| candidate.min(deadline))
            .unwrap_or(deadline);
        let request_id = rand::random::<i32>() & i32::MAX;
        let request = encode_bounded_request(request_id, auth.community.as_bytes(), &oid_arcs)?;

        attempts_made += 1;
        match send_and_receive(address, request, attempt_deadline) {
            Ok(packet) => {
                if Instant::now() >= deadline {
                    last_error = "global timeout expired after receiving response".to_string();
                    break;
                }
                let response =
                    match decode_response(packet.as_slice(), request_id, auth.community.as_bytes())
                    {
                        Ok(response) => response,
                        Err(error) => {
                            last_error = error;
                            continue;
                        }
                    };
                if response.error_status != 0 {
                    return Err(format!(
                        "SNMP agent returned error status {} at varbind index {}",
                        response.error_status, response.error_index
                    ));
                }
                if response.varbinds.len() != parsed_oids.len() {
                    return Err(format!(
                        "SNMP response returned {} varbind(s), expected {}",
                        response.varbinds.len(),
                        parsed_oids.len()
                    ));
                }

                let mut values = Vec::with_capacity(response.varbinds.len());
                for (index, (varbind, expected)) in response
                    .varbinds
                    .into_iter()
                    .zip(parsed_oids.iter())
                    .enumerate()
                {
                    if varbind.oid != expected.arcs {
                        return Err(format!(
                            "SNMP response OID mismatch at item {}: expected {}, got {}",
                            index + 1,
                            expected.canonical,
                            format_oid(&varbind.oid)
                        ));
                    }
                    values.push(normalize_varbind(&expected.canonical, varbind.value)?);
                }
                if Instant::now() >= deadline {
                    return Err(
                        "SNMP global timeout expired while normalizing response".to_string()
                    );
                }

                let mut result = HashMap::new();
                result.insert("target".to_string(), Value::String(target.to_string()));
                result.insert(
                    "address".to_string(),
                    Value::String(address.ip().to_string()),
                );
                result.insert("port".to_string(), Value::Int(i64::from(options.port)));
                result.insert("version".to_string(), Value::String("2c".to_string()));
                result.insert("duration_ms".to_string(), Value::Int(elapsed_ms(started)));
                result.insert("attempts".to_string(), Value::Int(attempts_made as i64));
                result.insert("values".to_string(), Value::Array(values));
                return Ok(result);
            }
            Err(error) => last_error = error,
        }
    }

    Err(format!(
        "SNMP request failed after {attempts_made} bounded attempt(s): {last_error}"
    ))
}

fn encode_bounded_request(
    request_id: i32,
    community: &[u8],
    oids: &[Vec<u32>],
) -> std::result::Result<Zeroizing<Vec<u8>>, String> {
    let request = encode_get_request(request_id, community, oids)?;
    if request.len() > MAX_REQUEST_BYTES {
        return Err(format!(
            "snmp_get() encoded request exceeds {MAX_REQUEST_BYTES} bytes"
        ));
    }
    Ok(request)
}

fn send_walk_and_receive(
    address: SocketAddr,
    request: Zeroizing<Vec<u8>>,
    attempt_deadline: Instant,
    expected_request_id: i32,
    expected_community: &[u8],
    received_bytes: &mut usize,
) -> std::result::Result<Zeroizing<Vec<u8>>, String> {
    let bind_address = match address {
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket =
        UdpSocket::bind(bind_address).map_err(|error| format!("SNMP UDP bind failed: {error}"))?;
    socket
        .connect(address)
        .map_err(|error| format!("SNMP UDP connect failed: {error}"))?;
    socket
        .set_write_timeout(Some(remaining_until(
            attempt_deadline,
            "attempt timeout expired before send",
        )?))
        .map_err(|error| format!("SNMP UDP write-timeout setup failed: {error}"))?;
    let sent = socket
        .send(request.as_slice())
        .map_err(|error| format!("SNMP UDP send failed: {error}"))?;
    if sent != request.len() {
        return Err(format!(
            "SNMP UDP send wrote {sent} bytes, expected {}",
            request.len()
        ));
    }
    drop(request);

    let mut packet = Zeroizing::new(vec![0_u8; MAX_RESPONSE_BYTES + 1]);
    loop {
        socket
            .set_read_timeout(Some(remaining_until(
                attempt_deadline,
                "attempt timeout expired after send",
            )?))
            .map_err(|error| format!("SNMP UDP read-timeout setup failed: {error}"))?;
        packet.resize(MAX_RESPONSE_BYTES + 1, 0);
        let received = match socket.recv(packet.as_mut_slice()) {
            Ok(received) => received,
            Err(error) if is_oversized_datagram_error(&error) => {
                *received_bytes = add_walk_budget(
                    *received_bytes,
                    MAX_RESPONSE_BYTES + 1,
                    MAX_WALK_RESPONSE_BYTES,
                    "cumulative responses",
                )?;
                return Err(format!("SNMP response exceeds {MAX_RESPONSE_BYTES} bytes"));
            }
            Err(error) => return Err(format!("SNMP UDP receive failed: {error}")),
        };
        *received_bytes = add_walk_budget(
            *received_bytes,
            received,
            MAX_WALK_RESPONSE_BYTES,
            "cumulative responses",
        )?;
        if Instant::now() >= attempt_deadline {
            return Err("SNMP attempt timeout expired while receiving response".to_string());
        }
        if received > MAX_RESPONSE_BYTES {
            return Err(format!("SNMP response exceeds {MAX_RESPONSE_BYTES} bytes"));
        }
        packet.truncate(received);
        match decode_response_allow_stale(
            packet.as_slice(),
            expected_request_id,
            expected_community,
        ) {
            Ok(Some(_)) => return Ok(packet),
            Ok(None) => continue,
            Err(error) => return Err(error),
        }
    }
}

fn is_oversized_datagram_error(error: &std::io::Error) -> bool {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        error.raw_os_error() == Some(90)
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd"
    ))]
    {
        error.raw_os_error() == Some(40)
    }
    #[cfg(windows)]
    {
        error.raw_os_error() == Some(10_040)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "openbsd",
        target_os = "netbsd",
        windows
    )))]
    {
        let _ = error;
        false
    }
}

fn send_and_receive(
    address: SocketAddr,
    request: Zeroizing<Vec<u8>>,
    attempt_deadline: Instant,
) -> std::result::Result<Zeroizing<Vec<u8>>, String> {
    let bind_address = match address {
        SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
        SocketAddr::V6(_) => SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0),
    };
    let socket =
        UdpSocket::bind(bind_address).map_err(|error| format!("SNMP UDP bind failed: {error}"))?;
    socket
        .connect(address)
        .map_err(|error| format!("SNMP UDP connect failed: {error}"))?;

    socket
        .set_write_timeout(Some(remaining_until(
            attempt_deadline,
            "attempt timeout expired before send",
        )?))
        .map_err(|error| format!("SNMP UDP write-timeout setup failed: {error}"))?;
    let sent = socket
        .send(request.as_slice())
        .map_err(|error| format!("SNMP UDP send failed: {error}"))?;
    if sent != request.len() {
        return Err(format!(
            "SNMP UDP send wrote {sent} bytes, expected {}",
            request.len()
        ));
    }
    drop(request);

    socket
        .set_read_timeout(Some(remaining_until(
            attempt_deadline,
            "attempt timeout expired after send",
        )?))
        .map_err(|error| format!("SNMP UDP read-timeout setup failed: {error}"))?;
    let mut packet = Zeroizing::new(vec![0_u8; MAX_RESPONSE_BYTES + 1]);
    let received = socket
        .recv(packet.as_mut_slice())
        .map_err(|error| format!("SNMP UDP receive failed: {error}"))?;
    if Instant::now() >= attempt_deadline {
        return Err("SNMP attempt timeout expired while receiving response".to_string());
    }
    if received > MAX_RESPONSE_BYTES {
        return Err(format!("SNMP response exceeds {MAX_RESPONSE_BYTES} bytes"));
    }
    packet.truncate(received);
    Ok(packet)
}

fn remaining_until(deadline: Instant, message: &str) -> std::result::Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| message.to_string())
}

fn require_netmon_enabled() -> std::result::Result<(), String> {
    let enabled = env::var("NTNT_NETMON_ENABLE")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false);
    if enabled {
        Ok(())
    } else {
        Err("std/netmon is disabled; set NTNT_NETMON_ENABLE=1 for this process".to_string())
    }
}

fn parse_target(target: &str) -> std::result::Result<IpAddr, String> {
    if target.is_empty() {
        return Err("snmp_get() target must not be empty".to_string());
    }
    if target.trim() != target {
        return Err("snmp_get() target must not have leading or trailing whitespace".to_string());
    }
    if target.len() > MAX_TARGET_BYTES || target.chars().any(char::is_control) {
        return Err(format!(
            "snmp_get() target must contain at most {MAX_TARGET_BYTES} non-control bytes"
        ));
    }
    target.parse::<IpAddr>().map_err(|_| {
        "snmp_get() target must be an IPv4 or IPv6 literal; hostnames are deferred".to_string()
    })
}

fn parse_v2c_auth(auth: &HashMap<String, Value>) -> std::result::Result<SnmpV2cAuth<'_>, String> {
    reject_unknown_keys(auth, &["version", "community"], "auth")?;
    match auth.get("version") {
        Some(Value::String(version)) if version == "2c" => {}
        Some(Value::String(_)) => {
            return Err("snmp_get() auth.version must be exactly '2c' in Slice 1".to_string())
        }
        Some(value) => {
            return Err(format!(
                "snmp_get() auth.version must be String, got {}",
                value.type_name()
            ))
        }
        None => return Err("snmp_get() auth.version is required".to_string()),
    }

    let community = match auth.get("community") {
        Some(Value::Secret(secret)) => secret.expose(),
        Some(value) => {
            return Err(format!(
                "snmp_get() auth.community must be Secret, got {}",
                value.type_name()
            ))
        }
        None => return Err("snmp_get() auth.community is required".to_string()),
    };
    if community.is_empty() || community.len() > MAX_COMMUNITY_BYTES {
        return Err(format!(
            "snmp_get() auth.community must contain 1 to {MAX_COMMUNITY_BYTES} bytes"
        ));
    }
    Ok(SnmpV2cAuth { community })
}

fn parse_options(
    opts: Option<&HashMap<String, Value>>,
) -> std::result::Result<SnmpOptions, String> {
    let Some(opts) = opts else {
        return Ok(SnmpOptions {
            port: DEFAULT_SNMP_PORT,
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
            retries: 0,
            allow_private: false,
        });
    };
    reject_unknown_keys(
        opts,
        &["port", "timeout_ms", "retries", "allow_private"],
        "options",
    )?;
    let port = parse_bounded_int(
        opts,
        "port",
        i64::from(DEFAULT_SNMP_PORT),
        1,
        i64::from(u16::MAX),
    )? as u16;
    let timeout_ms = parse_bounded_int(
        opts,
        "timeout_ms",
        DEFAULT_TIMEOUT_MS as i64,
        MIN_TIMEOUT_MS as i64,
        MAX_TIMEOUT_MS as i64,
    )? as u64;
    let retries = parse_bounded_int(opts, "retries", 0, 0, MAX_RETRIES as i64)? as usize;
    let allow_private = match opts.get("allow_private") {
        Some(Value::Bool(value)) => *value,
        Some(value) => {
            return Err(format!(
                "snmp_get() options.allow_private must be Bool, got {}",
                value.type_name()
            ))
        }
        None => false,
    };
    Ok(SnmpOptions {
        port,
        timeout: Duration::from_millis(timeout_ms),
        retries,
        allow_private,
    })
}

fn parse_bounded_int(
    opts: &HashMap<String, Value>,
    key: &str,
    default: i64,
    minimum: i64,
    maximum: i64,
) -> std::result::Result<i64, String> {
    let value = match opts.get(key) {
        Some(Value::Int(value)) => *value,
        Some(value) => {
            return Err(format!(
                "snmp_get() options.{key} must be Int, got {}",
                value.type_name()
            ))
        }
        None => default,
    };
    if !(minimum..=maximum).contains(&value) {
        return Err(format!(
            "snmp_get() options.{key} must be between {minimum} and {maximum}"
        ));
    }
    Ok(value)
}

fn reject_unknown_keys(
    map: &HashMap<String, Value>,
    allowed: &[&str],
    label: &str,
) -> std::result::Result<(), String> {
    let mut unknown: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|key| !allowed.contains(key))
        .collect();
    unknown.sort_unstable();
    if unknown.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "snmp_get() {label} contains unknown field(s): {}",
            unknown.join(", ")
        ))
    }
}

fn parse_oids(values: &[Value]) -> std::result::Result<Vec<ParsedOid>, String> {
    if values.is_empty() || values.len() > MAX_OIDS {
        return Err(format!(
            "snmp_get() oids must contain 1 to {MAX_OIDS} items"
        ));
    }
    let mut parsed = Vec::with_capacity(values.len());
    let mut seen = HashSet::new();
    for (index, value) in values.iter().enumerate() {
        let Value::String(raw) = value else {
            return Err(format!(
                "snmp_get() OID at item {} must be String, got {}",
                index + 1,
                value.type_name()
            ));
        };
        let canonical_input = raw.strip_prefix('.').unwrap_or(raw);
        if canonical_input.is_empty() || canonical_input.len() > MAX_OID_BYTES {
            return Err(format!(
                "snmp_get() OID at item {} must contain 1 to {MAX_OID_BYTES} bytes",
                index + 1
            ));
        }
        let segments: Vec<u32> = canonical_input
            .split('.')
            .map(|segment| {
                segment.parse::<u32>().map_err(|_| {
                    format!(
                        "snmp_get() OID at item {} must use numeric dotted notation",
                        index + 1
                    )
                })
            })
            .collect::<std::result::Result<_, _>>()?;
        if !(2..=MAX_OID_SEGMENTS).contains(&segments.len()) {
            return Err(format!(
                "snmp_get() OID at item {} must contain 2 to {MAX_OID_SEGMENTS} arcs",
                index + 1
            ));
        }
        if segments[0] > 2 || (segments[0] < 2 && segments[1] > 39) {
            return Err(format!(
                "snmp_get() OID at item {} has invalid root arcs",
                index + 1
            ));
        }
        let canonical = format_oid(&segments);
        if !seen.insert(canonical.clone()) {
            return Err(format!("snmp_get() duplicate OID '{canonical}'"));
        }
        parsed.push(ParsedOid {
            arcs: segments,
            canonical,
        });
    }
    Ok(parsed)
}

fn normalize_varbind(oid: &str, value: DecodedValue<'_>) -> std::result::Result<Value, String> {
    let mut map = HashMap::new();
    map.insert("oid".to_string(), Value::String(oid.to_string()));
    match value {
        DecodedValue::Boolean(value) => {
            map.insert("type".to_string(), Value::String("boolean".to_string()));
            map.insert("value".to_string(), Value::Bool(value));
        }
        DecodedValue::Integer(value) => insert_int_value(&mut map, "integer", value),
        DecodedValue::OctetString(bytes) => {
            map.insert(
                "type".to_string(),
                Value::String("octet_string".to_string()),
            );
            insert_bytes(&mut map, bytes);
        }
        DecodedValue::Null => insert_none_value(&mut map, "null"),
        DecodedValue::ObjectIdentifier(value) => {
            map.insert(
                "type".to_string(),
                Value::String("object_identifier".to_string()),
            );
            map.insert("value".to_string(), Value::String(format_oid(&value)));
        }
        DecodedValue::IpAddress(value) => {
            map.insert("type".to_string(), Value::String("ip_address".to_string()));
            map.insert(
                "value".to_string(),
                Value::String(Ipv4Addr::from(value).to_string()),
            );
        }
        DecodedValue::Counter32(value) => insert_int_value(&mut map, "counter32", i64::from(value)),
        DecodedValue::Unsigned32(value) => {
            insert_int_value(&mut map, "unsigned32", i64::from(value))
        }
        DecodedValue::Timeticks(value) => insert_int_value(&mut map, "timeticks", i64::from(value)),
        DecodedValue::Opaque(bytes) => {
            map.insert("type".to_string(), Value::String("opaque".to_string()));
            map.insert("encoding".to_string(), Value::String("hex".to_string()));
            map.insert("value".to_string(), Value::String(hex::encode(bytes)));
        }
        DecodedValue::Counter64(value) => {
            map.insert("type".to_string(), Value::String("counter64".to_string()));
            map.insert("encoding".to_string(), Value::String("decimal".to_string()));
            map.insert("value".to_string(), Value::String(value.to_string()));
        }
        DecodedValue::NoSuchObject => insert_none_value(&mut map, "no_such_object"),
        DecodedValue::NoSuchInstance => insert_none_value(&mut map, "no_such_instance"),
        DecodedValue::EndOfMibView => insert_none_value(&mut map, "end_of_mib_view"),
    }
    Ok(Value::Map(map))
}

fn insert_int_value(map: &mut HashMap<String, Value>, kind: &str, value: i64) {
    map.insert("type".to_string(), Value::String(kind.to_string()));
    map.insert("value".to_string(), Value::Int(value));
}

fn insert_none_value(map: &mut HashMap<String, Value>, kind: &str) {
    map.insert("type".to_string(), Value::String(kind.to_string()));
    map.insert("value".to_string(), Value::none());
}

fn insert_bytes(map: &mut HashMap<String, Value>, bytes: &[u8]) {
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            map.insert("encoding".to_string(), Value::String("utf8".to_string()));
            map.insert("value".to_string(), Value::String(text.to_string()));
        }
        Err(_) => {
            map.insert("encoding".to_string(), Value::String("hex".to_string()));
            map.insert("value".to_string(), Value::String(hex::encode(bytes)));
        }
    }
}

fn format_oid(arcs: &[u32]) -> String {
    arcs.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".")
}

fn elapsed_ms(started: Instant) -> i64 {
    i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX)
}

fn expect_string<'a>(value: &'a Value, label: &str) -> Result<&'a str> {
    match value {
        Value::String(value) => Ok(value),
        other => Err(IntentError::type_error(format!(
            "{label} must be String, got {}",
            other.type_name()
        ))),
    }
}

fn expect_map<'a>(value: &'a Value, label: &str) -> Result<&'a HashMap<String, Value>> {
    match value {
        Value::Map(value) => Ok(value),
        other => Err(IntentError::type_error(format!(
            "{label} must be Map, got {}",
            other.type_name()
        ))),
    }
}

fn expect_array<'a>(value: &'a Value, label: &str) -> Result<&'a [Value]> {
    match value {
        Value::Array(value) => Ok(value),
        other => Err(IntentError::type_error(format!(
            "{label} must be Array, got {}",
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::SecretValue;

    #[test]
    fn v2c_auth_requires_opaque_community_and_never_renders_plaintext() {
        let canary = "netmon-community-plain-canary";
        let plaintext = HashMap::from([
            ("version".to_string(), Value::String("2c".to_string())),
            ("community".to_string(), Value::String(canary.to_string())),
        ]);
        let error = parse_v2c_auth(&plaintext)
            .err()
            .expect("plaintext community must fail");
        assert!(!error.contains(canary));

        let secret = SecretValue::new("SNMP_COMMUNITY", canary).expect("valid secret");
        let opaque = HashMap::from([
            ("version".to_string(), Value::String("2c".to_string())),
            ("community".to_string(), Value::Secret(secret)),
        ]);
        assert_eq!(
            parse_v2c_auth(&opaque).expect("opaque community").community,
            canary
        );
    }

    #[test]
    fn auth_and_options_reject_unknown_keys_deterministically() {
        let auth = HashMap::from([
            ("version".to_string(), Value::String("2c".to_string())),
            (
                "community".to_string(),
                Value::Secret(SecretValue::new("SNMP_COMMUNITY", "secret").expect("valid secret")),
            ),
            ("zeta".to_string(), Value::Int(1)),
            ("alpha".to_string(), Value::Int(1)),
        ]);
        assert_eq!(
            parse_v2c_auth(&auth)
                .err()
                .expect("unknown auth key must fail"),
            "snmp_get() auth contains unknown field(s): alpha, zeta"
        );

        let options = HashMap::from([("timeout".to_string(), Value::Int(1))]);
        assert_eq!(
            parse_options(Some(&options)).expect_err("unknown option must fail"),
            "snmp_get() options contains unknown field(s): timeout"
        );
    }

    #[test]
    fn oid_and_target_parsers_are_strict_and_bounded() {
        let parsed = parse_oids(&[
            Value::String(".1.3.6.1.2.1.1.1.0".to_string()),
            Value::String("1.3.6.1.2.1.1.3.0".to_string()),
        ])
        .expect("valid OIDs");
        assert_eq!(parsed[0].canonical, "1.3.6.1.2.1.1.1.0");

        let duplicate = parse_oids(&[
            Value::String(".1.3.6.1".to_string()),
            Value::String("1.3.6.1".to_string()),
        ])
        .expect_err("canonical duplicate must fail");
        assert!(duplicate.contains("duplicate OID"));
        assert!(parse_oids(&[Value::String("1.3.bad".to_string())]).is_err());
        assert!(parse_oids(&[Value::String(" 1.3.6.1".to_string())]).is_err());

        assert!(parse_target("192.0.2.1").is_ok());
        assert!(parse_target("2001:db8::1").is_ok());
        assert!(parse_target("router.example.com").is_err());
        assert!(parse_target(" 10.0.0.1").is_err());
    }

    #[test]
    fn value_normalization_preserves_large_counters_boolean_and_binary_octets() {
        let counter = normalize_varbind("1.3.6.1", DecodedValue::Counter64(u64::MAX))
            .expect("counter normalizes");
        let Value::Map(counter) = counter else {
            panic!("expected counter map");
        };
        assert!(
            matches!(counter.get("value"), Some(Value::String(value)) if value == &u64::MAX.to_string())
        );

        let boolean =
            normalize_varbind("1.3.6.2", DecodedValue::Boolean(true)).expect("boolean normalizes");
        let Value::Map(boolean) = boolean else {
            panic!("expected boolean map");
        };
        assert!(matches!(boolean.get("value"), Some(Value::Bool(true))));

        let binary = normalize_varbind("1.3.6.3", DecodedValue::OctetString(&[0xff, 0x00]))
            .expect("binary octets normalize");
        let Value::Map(binary) = binary else {
            panic!("expected binary map");
        };
        assert!(matches!(binary.get("encoding"), Some(Value::String(value)) if value == "hex"));

        let missing = normalize_varbind("1.3.6.4", DecodedValue::NoSuchObject)
            .expect("protocol exception normalizes");
        let Value::Map(missing) = missing else {
            panic!("expected missing map");
        };
        assert!(
            matches!(missing.get("type"), Some(Value::String(value)) if value == "no_such_object")
        );
        assert!(matches!(
            missing.get("value"),
            Some(Value::EnumValue { enum_name, variant, .. })
                if enum_name == "Option" && variant == "None"
        ));
    }

    #[test]
    fn encoded_request_size_is_bounded_before_transport() {
        let oversized_oids = (0..MAX_OIDS)
            .map(|index| {
                let mut oid = vec![2, index as u32];
                oid.extend(vec![u32::MAX; MAX_OID_SEGMENTS - 2]);
                oid
            })
            .collect::<Vec<_>>();
        let error = encode_bounded_request(7, b"secret", &oversized_oids)
            .expect_err("oversized request must fail before transport");
        assert!(error.contains("encoded request exceeds"));
    }

    #[test]
    fn walk_cumulative_receive_and_output_budgets_are_exact_and_overflow_safe() {
        assert_eq!(
            add_walk_budget(
                MAX_WALK_RESPONSE_BYTES - 1,
                1,
                MAX_WALK_RESPONSE_BYTES,
                "cumulative responses",
            )
            .expect("exact receive limit is allowed"),
            MAX_WALK_RESPONSE_BYTES
        );
        assert!(add_walk_budget(
            MAX_WALK_RESPONSE_BYTES,
            1,
            MAX_WALK_RESPONSE_BYTES,
            "cumulative responses",
        )
        .expect_err("receive limit plus one must fail")
        .contains("cumulative responses"));
        assert_eq!(
            add_walk_budget(
                MAX_WALK_OUTPUT_BYTES - 1,
                1,
                MAX_WALK_OUTPUT_BYTES,
                "normalized output",
            )
            .expect("exact output limit is allowed"),
            MAX_WALK_OUTPUT_BYTES
        );
        assert!(
            add_walk_budget(usize::MAX, 1, MAX_WALK_OUTPUT_BYTES, "normalized output",)
                .expect_err("counter overflow must fail")
                .contains("overflow")
        );
    }

    #[test]
    fn walk_receive_budget_charges_oversized_datagram_before_rejection() {
        let server = UdpSocket::bind("127.0.0.1:0").expect("bind fixture");
        let address = server.local_addr().expect("fixture address");
        let fixture = std::thread::spawn(move || {
            let mut request = [0u8; 16];
            let (_, peer) = server.recv_from(&mut request).expect("receive request");
            server
                .send_to(&vec![0u8; MAX_RESPONSE_BYTES + 1], peer)
                .expect("send oversized response");
        });
        let mut received_bytes = MAX_WALK_RESPONSE_BYTES - MAX_RESPONSE_BYTES;
        let error = send_walk_and_receive(
            address,
            Zeroizing::new(vec![0]),
            Instant::now() + Duration::from_secs(1),
            7,
            b"secret",
            &mut received_bytes,
        )
        .expect_err("oversized datagram must consume the cumulative budget first");
        fixture.join().expect("fixture thread");
        assert!(error.contains("cumulative responses"), "{error}");
    }

    #[test]
    fn finish_walk_enforces_the_exact_conservative_output_boundary() {
        let address: SocketAddr = "127.0.0.1:161".parse().expect("socket address");
        let finish = |payload_len: usize| {
            let started = Instant::now();
            finish_walk(
                "127.0.0.1",
                address,
                "1.3.6.1.2.1",
                started,
                started + Duration::from_secs(5),
                1,
                1,
                true,
                "end_of_mib_view",
                vec![Value::String("x".repeat(payload_len))],
            )
        };

        let mut accepted = 0usize;
        let mut rejected = MAX_WALK_OUTPUT_BYTES + 1;
        while accepted + 1 < rejected {
            let midpoint = accepted + (rejected - accepted) / 2;
            if finish(midpoint).is_ok() {
                accepted = midpoint;
            } else {
                rejected = midpoint;
            }
        }

        let exact = finish(accepted).expect("exact conservative output limit must pass");
        assert_eq!(conservative_map_size(&exact), MAX_WALK_OUTPUT_BYTES);
        assert!(finish(accepted + 1)
            .expect_err("one byte beyond conservative output limit must fail")
            .contains("normalized output exceeds"));
    }
}
