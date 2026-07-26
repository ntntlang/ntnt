//! Bounded network-monitoring primitives.
//!
//! `std/netmon` builds monitoring-oriented protocol support on top of the
//! outbound target policy shared with `std/net`. Credentials remain opaque
//! `Secret` values and are exposed only to the protocol sink that needs them.

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use crate::stdlib::net::enforce_resolved_target_policy;
use snmp2::{Oid, SyncSession, Value as SnmpValue};
use std::collections::{HashMap, HashSet};
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

const DEFAULT_SNMP_PORT: u16 = 161;
const DEFAULT_TIMEOUT_MS: u64 = 2_000;
const MIN_TIMEOUT_MS: u64 = 50;
const MAX_TIMEOUT_MS: u64 = 30_000;
const MAX_RETRIES: u64 = 3;
const MAX_ATTEMPTS: usize = 4;
const MAX_TARGET_BYTES: usize = 253;
const MAX_COMMUNITY_BYTES: usize = 255;
const MAX_OIDS: usize = 64;
const MAX_OID_BYTES: usize = 255;
const MAX_OID_SEGMENTS: usize = 128;

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
    // SNMPv2c does not encrypt its community or payload; use this slice only on
    // trusted management networks or protected tunnels while SNMPv3 is deferred.
    //
    // Public targets are allowed by default. Private/internal targets require
    // both `NTNT_NET_ALLOW_PRIVATE=1` and `allow_private: true`; metadata,
    // multicast, broadcast, unspecified, and documentation targets remain denied.
    // The timeout is a global budget across address fallback and retries.
    // @param target DNS hostname or IPv4/IPv6 address without a port
    // @param auth Strict map with version (`"2c"`) and community (Secret)
    // @param oids One to 64 numeric OIDs
    // @param opts Optional strict map with port (default 161), timeout_ms (default 2000), retries (default 0, max 3), and allow_private
    // @returns Result containing target, checked address, port, version, duration_ms, attempts, and normalized values
    // @error TypeError ~ "snmp_get() argument 1 must be String" fix: "Pass a hostname or IP address"
    // @error RuntimeError ~ "snmp_get() auth.community must be Secret" fix: "Load the community with std/secrets.require_secret()"
    // @see_also require_secret, net_capabilities
    // @since v0.5.2
    // @tags #network, #monitoring, #snmp, #security
    // @example snmp_get("router.example.com", map { "version": "2c", "community": require_secret("SNMP_COMMUNITY") }, ["1.3.6.1.2.1.1.1.0"]) ~ "Read sysDescr using an opaque community"
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
    let target = string_arg(args, 0, "snmp_get")?;
    let auth = map_arg(args, 1, "snmp_get", "auth")?;
    let oid_values = array_arg(args, 2, "snmp_get", "oids")?;
    let opts = optional_map_arg(args, 3, "snmp_get", "options")?;

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
    let target = validate_target(target)?;
    let auth = parse_v2c_auth(auth)?;
    let options = parse_options(opts)?;
    let parsed_oids = parse_oids(oid_values)?;
    let addresses = resolve_targets(target, options.port)?;
    let policy_targets: Vec<_> = addresses
        .iter()
        .copied()
        .map(|address| (options.port, address))
        .collect();
    enforce_resolved_target_policy(&policy_targets, options.allow_private)?;

    let oid_refs: Vec<&Oid<'_>> = parsed_oids.iter().collect();
    let attempt_limit = addresses
        .len()
        .saturating_add(options.retries)
        .clamp(1, MAX_ATTEMPTS);
    let started = Instant::now();
    let deadline = started + options.timeout;
    let mut last_error = "request failed".to_string();
    let mut attempts_made = 0usize;

    for attempt in 0..attempt_limit {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            last_error = "global timeout expired".to_string();
            break;
        };
        if remaining.is_zero() {
            last_error = "global timeout expired".to_string();
            break;
        }
        let remaining_attempts = attempt_limit - attempt;
        let attempt_timeout = remaining / remaining_attempts as u32;
        let address = addresses[attempt % addresses.len()];
        attempts_made += 1;
        let request_id = rand::random::<i32>();
        let mut session = match SyncSession::new_v2c(
            address,
            auth.community.as_bytes(),
            Some(attempt_timeout),
            request_id,
        ) {
            Ok(session) => session,
            Err(error) => {
                last_error = error.to_string();
                continue;
            }
        };

        match session.get_many(&oid_refs) {
            Ok(response) => {
                if response.error_status != 0 {
                    return Err(format!(
                        "SNMP agent returned error status {} at varbind index {}",
                        response.error_status, response.error_index
                    ));
                }
                let response_varbinds: Vec<_> = response.varbinds.collect();
                if response_varbinds.len() != parsed_oids.len() {
                    return Err(format!(
                        "SNMP response returned {} varbind(s), expected {}",
                        response_varbinds.len(),
                        parsed_oids.len()
                    ));
                }
                let mut values = Vec::with_capacity(response_varbinds.len());
                for (index, ((oid, value), expected)) in response_varbinds
                    .into_iter()
                    .zip(parsed_oids.iter())
                    .enumerate()
                {
                    let actual_oid = oid.to_string();
                    let expected_oid = expected.to_string();
                    if actual_oid != expected_oid {
                        return Err(format!(
                            "SNMP response OID mismatch at item {}: expected {}, got {}",
                            index + 1,
                            expected_oid,
                            actual_oid
                        ));
                    }
                    values.push(normalize_varbind(&actual_oid, value)?);
                }
                let mut result = HashMap::new();
                result.insert("target".to_string(), Value::String(target.to_string()));
                result.insert(
                    "address".to_string(),
                    Value::String(address.ip().to_string()),
                );
                result.insert("port".to_string(), Value::Int(i64::from(options.port)));
                result.insert("version".to_string(), Value::String("2c".to_string()));
                result.insert(
                    "duration_ms".to_string(),
                    Value::Int(started.elapsed().as_millis().min(i64::MAX as u128) as i64),
                );
                result.insert("attempts".to_string(), Value::Int(attempts_made as i64));
                result.insert("values".to_string(), Value::Array(values));
                return Ok(result);
            }
            Err(error) => last_error = error.to_string(),
        }
    }

    Err(format!(
        "SNMP request failed after {attempts_made} bounded attempt(s): {last_error}"
    ))
}

fn validate_target(target: &str) -> std::result::Result<&str, String> {
    if target.is_empty() {
        return Err("snmp_get() target must not be empty".to_string());
    }
    if target.trim() != target {
        return Err("snmp_get() target must not have leading or trailing whitespace".to_string());
    }
    if target.len() > MAX_TARGET_BYTES || target.chars().any(char::is_control) {
        return Err(format!(
            "snmp_get() target must be at most {MAX_TARGET_BYTES} bytes and contain no control characters"
        ));
    }
    Ok(target)
}

fn parse_v2c_auth(auth: &HashMap<String, Value>) -> std::result::Result<SnmpV2cAuth<'_>, String> {
    reject_unknown_keys(auth, &["version", "community"], "snmp_get() auth")?;
    match auth.get("version") {
        Some(Value::String(version)) if version == "2c" => {}
        Some(Value::String(_)) => {
            return Err("snmp_get() auth.version currently supports only '2c'".to_string())
        }
        Some(other) => {
            return Err(format!(
                "snmp_get() auth.version must be String, got {}",
                other.type_name()
            ))
        }
        None => return Err("snmp_get() auth.version is required".to_string()),
    }
    let community = match auth.get("community") {
        Some(Value::Secret(secret)) => secret.expose(),
        Some(other) => {
            return Err(format!(
                "snmp_get() auth.community must be Secret, got {}",
                other.type_name()
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
        "snmp_get() options",
    )?;
    let port = parse_bounded_int_option(
        opts,
        "port",
        i64::from(DEFAULT_SNMP_PORT),
        1,
        i64::from(u16::MAX),
    )? as u16;
    let timeout_ms = parse_bounded_int_option(
        opts,
        "timeout_ms",
        DEFAULT_TIMEOUT_MS as i64,
        MIN_TIMEOUT_MS as i64,
        MAX_TIMEOUT_MS as i64,
    )? as u64;
    let retries = parse_bounded_int_option(opts, "retries", 0, 0, MAX_RETRIES as i64)? as usize;
    let allow_private = match opts.get("allow_private") {
        None => false,
        Some(Value::Bool(value)) => *value,
        Some(other) => {
            return Err(format!(
                "snmp_get() option 'allow_private' must be Bool, got {}",
                other.type_name()
            ))
        }
    };
    Ok(SnmpOptions {
        port,
        timeout: Duration::from_millis(timeout_ms),
        retries,
        allow_private,
    })
}

fn parse_bounded_int_option(
    opts: &HashMap<String, Value>,
    name: &str,
    default: i64,
    min: i64,
    max: i64,
) -> std::result::Result<i64, String> {
    match opts.get(name) {
        None => Ok(default),
        Some(Value::Int(value)) if (min..=max).contains(value) => Ok(*value),
        Some(Value::Int(value)) => Err(format!(
            "snmp_get() option '{name}' must be between {min} and {max}, got {value}"
        )),
        Some(other) => Err(format!(
            "snmp_get() option '{name}' must be Int, got {}",
            other.type_name()
        )),
    }
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
            "{label} contains unknown key(s): {}",
            unknown.join(", ")
        ))
    }
}

fn parse_oids(values: &[Value]) -> std::result::Result<Vec<Oid<'static>>, String> {
    if values.is_empty() || values.len() > MAX_OIDS {
        return Err(format!(
            "snmp_get() oids must contain 1 to {MAX_OIDS} items"
        ));
    }
    let mut seen = HashSet::with_capacity(values.len());
    let mut parsed = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        let Value::String(raw) = value else {
            return Err(format!(
                "snmp_get() oids item {} must be String, got {}",
                index + 1,
                value.type_name()
            ));
        };
        let canonical = raw.strip_prefix('.').unwrap_or(raw);
        if canonical.is_empty() || canonical.len() > MAX_OID_BYTES {
            return Err(format!(
                "snmp_get() OID at item {} must contain 1 to {MAX_OID_BYTES} bytes",
                index + 1
            ));
        }
        let segments = canonical
            .split('.')
            .map(|segment| {
                if segment.is_empty() || !segment.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(format!(
                        "snmp_get() OID at item {} must be numeric dotted notation",
                        index + 1
                    ));
                }
                segment.parse::<u64>().map_err(|_| {
                    format!(
                        "snmp_get() OID at item {} contains an out-of-range segment",
                        index + 1
                    )
                })
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if segments.len() < 2 || segments.len() > MAX_OID_SEGMENTS {
            return Err(format!(
                "snmp_get() OID at item {} must contain 2 to {MAX_OID_SEGMENTS} segments",
                index + 1
            ));
        }
        if segments[0] > 2 || (segments[0] < 2 && segments[1] > 39) {
            return Err(format!(
                "snmp_get() OID at item {} has an invalid root arc",
                index + 1
            ));
        }
        let canonical = segments
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(".");
        if !seen.insert(canonical.clone()) {
            return Err(format!("snmp_get() duplicate OID '{canonical}'"));
        }
        let oid = Oid::from(&segments)
            .map_err(|_| format!("snmp_get() invalid OID at item {}", index + 1))?;
        parsed.push(oid);
    }
    Ok(parsed)
}

fn resolve_targets(target: &str, port: u16) -> std::result::Result<Vec<SocketAddr>, String> {
    let mut addresses: Vec<_> = (target, port)
        .to_socket_addrs()
        .map_err(|_| "snmp_get() target could not be resolved".to_string())?
        .collect();
    addresses.sort_unstable();
    addresses.dedup();
    if addresses.is_empty() {
        Err("snmp_get() target resolved to no addresses".to_string())
    } else {
        Ok(addresses)
    }
}

fn normalize_varbind(oid: &str, value: SnmpValue<'_>) -> std::result::Result<Value, String> {
    let mut map = HashMap::new();
    map.insert("oid".to_string(), Value::String(oid.to_string()));
    match value {
        SnmpValue::Boolean(value) => normalized(&mut map, "boolean", Value::Bool(value)),
        SnmpValue::Null => normalized(&mut map, "null", Value::none()),
        SnmpValue::Integer(value) => normalized(&mut map, "integer", Value::Int(value)),
        SnmpValue::OctetString(bytes) => match std::str::from_utf8(bytes) {
            Ok(text) => {
                map.insert("encoding".to_string(), Value::String("utf8".to_string()));
                normalized(&mut map, "octet_string", Value::String(text.to_string()));
            }
            Err(_) => {
                map.insert("encoding".to_string(), Value::String("hex".to_string()));
                normalized(&mut map, "octet_string", Value::String(hex::encode(bytes)));
            }
        },
        SnmpValue::ObjectIdentifier(value) => normalized(
            &mut map,
            "object_identifier",
            Value::String(value.to_string()),
        ),
        SnmpValue::IpAddress(value) => normalized(
            &mut map,
            "ip_address",
            Value::String(format!(
                "{}.{}.{}.{}",
                value[0], value[1], value[2], value[3]
            )),
        ),
        SnmpValue::Counter32(value) => {
            normalized(&mut map, "counter32", Value::Int(i64::from(value)))
        }
        SnmpValue::Unsigned32(value) => {
            normalized(&mut map, "unsigned32", Value::Int(i64::from(value)))
        }
        SnmpValue::Timeticks(value) => {
            normalized(&mut map, "timeticks", Value::Int(i64::from(value)))
        }
        SnmpValue::Opaque(bytes) => {
            map.insert("encoding".to_string(), Value::String("hex".to_string()));
            normalized(&mut map, "opaque", Value::String(hex::encode(bytes)));
        }
        SnmpValue::Counter64(value) => {
            map.insert("encoding".to_string(), Value::String("decimal".to_string()));
            normalized(&mut map, "counter64", Value::String(value.to_string()));
        }
        SnmpValue::EndOfMibView => normalized(&mut map, "end_of_mib_view", Value::none()),
        SnmpValue::NoSuchObject => normalized(&mut map, "no_such_object", Value::none()),
        SnmpValue::NoSuchInstance => normalized(&mut map, "no_such_instance", Value::none()),
        SnmpValue::Sequence(_)
        | SnmpValue::Set(_)
        | SnmpValue::Constructed(_, _)
        | SnmpValue::GetRequest(_)
        | SnmpValue::GetNextRequest(_)
        | SnmpValue::GetBulkRequest(_)
        | SnmpValue::Response(_)
        | SnmpValue::SetRequest(_)
        | SnmpValue::InformRequest(_)
        | SnmpValue::Trap(_)
        | SnmpValue::Report(_) => {
            return Err(format!(
                "SNMP response for OID {oid} contained an unsupported constructed value"
            ))
        }
    }
    Ok(Value::Map(map))
}

fn normalized(map: &mut HashMap<String, Value>, kind: &str, value: Value) {
    map.insert("type".to_string(), Value::String(kind.to_string()));
    map.insert("value".to_string(), value);
}

fn string_arg<'a>(args: &'a [Value], index: usize, fn_name: &str) -> Result<&'a str> {
    match &args[index] {
        Value::String(value) => Ok(value),
        other => Err(IntentError::type_error(format!(
            "{fn_name}() argument {} must be String, got {}",
            index + 1,
            other.type_name()
        ))),
    }
}

fn map_arg<'a>(
    args: &'a [Value],
    index: usize,
    fn_name: &str,
    label: &str,
) -> Result<&'a HashMap<String, Value>> {
    match &args[index] {
        Value::Map(value) => Ok(value),
        other => Err(IntentError::type_error(format!(
            "{fn_name}() {label} argument must be Map, got {}",
            other.type_name()
        ))),
    }
}

fn optional_map_arg<'a>(
    args: &'a [Value],
    index: usize,
    fn_name: &str,
    label: &str,
) -> Result<Option<&'a HashMap<String, Value>>> {
    if args.len() <= index {
        return Ok(None);
    }
    map_arg(args, index, fn_name, label).map(Some)
}

fn array_arg<'a>(
    args: &'a [Value],
    index: usize,
    fn_name: &str,
    label: &str,
) -> Result<&'a [Value]> {
    match &args[index] {
        Value::Array(value) => Ok(value),
        other => Err(IntentError::type_error(format!(
            "{fn_name}() {label} argument must be Array<String>, got {}",
            other.type_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::SecretValue;

    fn secret(value: &str) -> Value {
        Value::Secret(SecretValue::new("SNMP_COMMUNITY", value).expect("valid secret"))
    }

    #[test]
    fn v2c_auth_requires_opaque_community_and_never_renders_plaintext() {
        let canary = "community-canary-never-render";
        let plaintext = HashMap::from([
            ("version".to_string(), Value::String("2c".to_string())),
            ("community".to_string(), Value::String(canary.to_string())),
        ]);
        let error = parse_v2c_auth(&plaintext)
            .err()
            .expect("plaintext community must fail");
        assert!(error.contains("must be Secret"));
        assert!(!error.contains(canary));

        let opaque = HashMap::from([
            ("version".to_string(), Value::String("2c".to_string())),
            ("community".to_string(), secret(canary)),
        ]);
        let parsed = parse_v2c_auth(&opaque).expect("opaque community accepted");
        assert_eq!(parsed.community, canary);
    }

    #[test]
    fn auth_and_options_reject_unknown_keys_deterministically() {
        let auth = HashMap::from([
            ("version".to_string(), Value::String("2c".to_string())),
            ("community".to_string(), secret("test")),
            ("username".to_string(), Value::String("not-v2c".to_string())),
        ]);
        assert_eq!(
            parse_v2c_auth(&auth)
                .err()
                .expect("unknown auth key must fail"),
            "snmp_get() auth contains unknown key(s): username"
        );

        let opts = HashMap::from([
            ("zeta".to_string(), Value::Bool(true)),
            ("alpha".to_string(), Value::Bool(true)),
        ]);
        assert_eq!(
            parse_options(Some(&opts)).expect_err("unknown option keys must fail"),
            "snmp_get() options contains unknown key(s): alpha, zeta"
        );
    }

    #[test]
    fn oid_parser_canonicalizes_and_rejects_invalid_or_duplicate_values() {
        let parsed = parse_oids(&[
            Value::String(".1.3.6.1.2.1.1.1.0".to_string()),
            Value::String("1.3.6.1.2.1.1.3.0".to_string()),
        ])
        .expect("valid OIDs");
        assert_eq!(parsed[0].to_string(), "1.3.6.1.2.1.1.1.0");

        let duplicate = parse_oids(&[
            Value::String(".1.3.6.1".to_string()),
            Value::String("1.3.6.1".to_string()),
        ])
        .err()
        .expect("canonical duplicate must fail");
        assert!(duplicate.contains("duplicate OID"));

        let invalid = parse_oids(&[Value::String("1.3.bad".to_string())])
            .err()
            .expect("named MIB form is not accepted in slice 1");
        assert!(invalid.contains("numeric dotted notation"));

        assert!(parse_oids(&[Value::String(" 1.3.6.1".to_string())]).is_err());
        assert!(validate_target(" router.example.com").is_err());
    }

    #[test]
    fn value_normalization_preserves_large_counters_and_binary_octets() {
        let counter = normalize_varbind("1.3.6.1", SnmpValue::Counter64(u64::MAX))
            .expect("counter normalizes");
        let Value::Map(counter) = counter else {
            panic!("expected map");
        };
        assert!(matches!(counter.get("type"), Some(Value::String(value)) if value == "counter64"));
        assert!(
            matches!(counter.get("value"), Some(Value::String(value)) if value == &u64::MAX.to_string())
        );
        assert!(
            matches!(counter.get("encoding"), Some(Value::String(value)) if value == "decimal")
        );

        let binary = normalize_varbind("1.3.6.2", SnmpValue::OctetString(&[0xff, 0x00]))
            .expect("binary octets normalize");
        let Value::Map(binary) = binary else {
            panic!("expected map");
        };
        assert!(matches!(binary.get("value"), Some(Value::String(value)) if value == "ff00"));
        assert!(matches!(binary.get("encoding"), Some(Value::String(value)) if value == "hex"));

        let missing = normalize_varbind("1.3.6.3", SnmpValue::NoSuchObject)
            .expect("protocol exception normalizes");
        let Value::Map(missing) = missing else {
            panic!("expected map");
        };
        assert!(
            matches!(missing.get("type"), Some(Value::String(value)) if value == "no_such_object")
        );
        assert!(matches!(
            missing.get("value"),
            Some(Value::EnumValue {
                enum_name,
                variant,
                values
            }) if enum_name == "Option" && variant == "None" && values.is_empty()
        ));
    }
}
