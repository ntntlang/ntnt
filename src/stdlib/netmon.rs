//! Bounded network-monitoring primitives.
//!
//! `std/netmon` builds monitoring-oriented protocol support on top of the
//! outbound target policy shared with `std/net`. Credentials remain opaque
//! `Secret` values and are exposed only to the protocol sink that needs them.

#[path = "netmon_codec.rs"]
mod codec;

use self::codec::{decode_response, encode_get_request, DecodedValue};
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
const MAX_RESPONSE_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy)]
struct SnmpOptions {
    port: u16,
    timeout: Duration,
    retries: usize,
    allow_private: bool,
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
    // 16 KiB and must be complete, strict SNMPv2c BER with exactly the requested
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

    module
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
}
