//! End-to-end tests for bounded SNMPv2c GETNEXT WALK.
//!
//! The UDP fixture hand-encodes responses and independently parses request BER;
//! it does not reuse the production codec.

use std::fs;
use std::io::Write;
use std::net::UdpSocket;
use std::ops::Range;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SECRET_CANARY: &str = "walk-community-canary-never-render";
static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_test_file() -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir()
        .join(format!(
            "ntnt_std_netmon_walk_{}_{}.tnt",
            std::process::id(),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

fn ntnt_binary() -> &'static str {
    env!("CARGO_BIN_EXE_ntnt")
}

fn run_ntnt_code(code: &str, envs: &[(&str, &str)]) -> (String, String, i32) {
    let test_file = unique_test_file();
    let mut file = fs::File::create(&test_file).expect("create test source");
    writeln!(file, "{code}").expect("write test source");
    drop(file);

    let mut command = Command::new(ntnt_binary());
    command
        .args(["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("NTNT_ENV", "development")
        .env("APP_ENV", "development")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .env_remove("NTNT_NETMON_ENABLE")
        .env_remove("NTNT_NET_ALLOW_PRIVATE")
        .env_remove("NTNT_TYPE_MODE")
        .env_remove("NTNT_LINT_MODE")
        .env_remove("NTNT_STRICT")
        .env_remove("NTNT_OOB_MODE");
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command.output().expect("run ntnt");
    let _ = fs::remove_file(test_file);
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

fn read_length(bytes: &[u8], offset: &mut usize) -> Option<usize> {
    let first = *bytes.get(*offset)?;
    *offset += 1;
    if first & 0x80 == 0 {
        return Some(usize::from(first));
    }
    let count = usize::from(first & 0x7f);
    if count == 0 || count > std::mem::size_of::<usize>() {
        return None;
    }
    let mut length = 0usize;
    for _ in 0..count {
        length = length.checked_mul(256)?;
        length = length.checked_add(usize::from(*bytes.get(*offset)?))?;
        *offset += 1;
    }
    Some(length)
}

fn read_tlv(bytes: &[u8], offset: &mut usize) -> Option<(u8, Range<usize>)> {
    let tag = *bytes.get(*offset)?;
    *offset += 1;
    let length = read_length(bytes, offset)?;
    let start = *offset;
    let end = start.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    *offset = end;
    Some((tag, start..end))
}

fn append_length(output: &mut Vec<u8>, length: usize) {
    if length < 128 {
        output.push(length as u8);
        return;
    }
    let bytes = length.to_be_bytes();
    let start = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len() - 1);
    output.push(0x80 | (bytes.len() - start) as u8);
    output.extend_from_slice(&bytes[start..]);
}

fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut output = vec![tag];
    append_length(&mut output, content.len());
    output.extend_from_slice(content);
    output
}

fn encode_base128(mut value: u64, output: &mut Vec<u8>) {
    let mut bytes = [0u8; 10];
    let mut index = bytes.len() - 1;
    bytes[index] = (value & 0x7f) as u8;
    value >>= 7;
    while value != 0 {
        index -= 1;
        bytes[index] = ((value & 0x7f) as u8) | 0x80;
        value >>= 7;
    }
    output.extend_from_slice(&bytes[index..]);
}

fn encode_oid(arcs: &[u32]) -> Vec<u8> {
    let mut output = Vec::new();
    encode_base128(u64::from(arcs[0]) * 40 + u64::from(arcs[1]), &mut output);
    for arc in &arcs[2..] {
        encode_base128(u64::from(*arc), &mut output);
    }
    output
}

fn decode_oid(bytes: &[u8]) -> Option<Vec<u32>> {
    let mut encoded_arcs = Vec::new();
    let mut value = 0u64;
    for byte in bytes {
        value = value.checked_shl(7)?.checked_add(u64::from(byte & 0x7f))?;
        if byte & 0x80 == 0 {
            encoded_arcs.push(u32::try_from(value).ok()?);
            value = 0;
        }
    }
    if value != 0 || encoded_arcs.is_empty() {
        return None;
    }
    let combined = encoded_arcs.remove(0);
    let (first, second) = if combined < 40 {
        (0, combined)
    } else if combined < 80 {
        (1, combined - 40)
    } else {
        (2, combined - 80)
    };
    let mut arcs = vec![first, second];
    arcs.extend(encoded_arcs);
    Some(arcs)
}

#[derive(Debug)]
struct ParsedRequest {
    request_id: Vec<u8>,
    community: Vec<u8>,
    cursor: Vec<u32>,
    pdu_tag: u8,
}

fn parse_request(bytes: &[u8]) -> Option<ParsedRequest> {
    let mut root_offset = 0;
    let (outer_tag, outer) = read_tlv(bytes, &mut root_offset)?;
    if outer_tag != 0x30 || root_offset != bytes.len() {
        return None;
    }

    let mut message_offset = outer.start;
    let (version_tag, version) = read_tlv(bytes, &mut message_offset)?;
    let (community_tag, community) = read_tlv(bytes, &mut message_offset)?;
    let (pdu_tag, pdu) = read_tlv(bytes, &mut message_offset)?;
    if version_tag != 0x02
        || bytes[version] != [1]
        || community_tag != 0x04
        || message_offset != outer.end
    {
        return None;
    }

    let mut pdu_offset = pdu.start;
    let (request_id_tag, request_id) = read_tlv(bytes, &mut pdu_offset)?;
    let (error_status_tag, error_status) = read_tlv(bytes, &mut pdu_offset)?;
    let (error_index_tag, error_index) = read_tlv(bytes, &mut pdu_offset)?;
    let (list_tag, list) = read_tlv(bytes, &mut pdu_offset)?;
    if request_id_tag != 0x02
        || request_id.is_empty()
        || error_status_tag != 0x02
        || bytes[error_status] != [0]
        || error_index_tag != 0x02
        || bytes[error_index] != [0]
        || list_tag != 0x30
        || pdu_offset != pdu.end
    {
        return None;
    }

    let mut list_offset = list.start;
    let (varbind_tag, varbind) = read_tlv(bytes, &mut list_offset)?;
    if varbind_tag != 0x30 || list_offset != list.end {
        return None;
    }
    let mut varbind_offset = varbind.start;
    let (oid_tag, oid) = read_tlv(bytes, &mut varbind_offset)?;
    let (null_tag, null) = read_tlv(bytes, &mut varbind_offset)?;
    if oid_tag != 0x06 || null_tag != 0x05 || !null.is_empty() || varbind_offset != varbind.end {
        return None;
    }

    Some(ParsedRequest {
        request_id: bytes[request_id].to_vec(),
        community: bytes[community].to_vec(),
        cursor: decode_oid(&bytes[oid])?,
        pdu_tag,
    })
}

#[derive(Clone)]
enum ValueSpec {
    Integer(u8),
    Octets(Vec<u8>),
    EndOfMibView,
    NoSuchObject,
    NoSuchInstance,
}

#[derive(Clone, Copy, Debug)]
enum Corruption {
    None,
    WrongVersion,
    WrongCommunity,
    WrongPdu,
    WrongRequestId,
    Trailing,
    Malformed,
    MultipleVarbinds,
    Oversized,
    AgentError,
    NonzeroSuccessIndex,
}

#[derive(Clone)]
enum Action {
    Respond {
        oid: Vec<u32>,
        value: ValueSpec,
        corruption: Corruption,
    },
    RememberResponse {
        oid: Vec<u32>,
        value: ValueSpec,
    },
    SendRememberedThenRespond {
        oid: Vec<u32>,
        value: ValueSpec,
    },
    ForgedThenRespond {
        oid: Vec<u32>,
        value: ValueSpec,
    },
    Drop,
}

fn encoded_value(value: &ValueSpec) -> Vec<u8> {
    match value {
        ValueSpec::Integer(value) => tlv(0x02, &[*value]),
        ValueSpec::Octets(value) => tlv(0x04, value),
        ValueSpec::EndOfMibView => tlv(0x82, &[]),
        ValueSpec::NoSuchObject => tlv(0x80, &[]),
        ValueSpec::NoSuchInstance => tlv(0x81, &[]),
    }
}

fn build_response(
    request: &ParsedRequest,
    oid: &[u32],
    value: &ValueSpec,
    corruption: Corruption,
) -> Vec<u8> {
    if matches!(corruption, Corruption::Oversized) {
        return vec![0; 8 * 1024 + 1];
    }

    let mut varbind_content = tlv(0x06, &encode_oid(oid));
    varbind_content.extend_from_slice(&encoded_value(value));
    let varbind = tlv(0x30, &varbind_content);
    let mut list_content = varbind.clone();
    if matches!(corruption, Corruption::MultipleVarbinds) {
        list_content.extend_from_slice(&varbind);
    }

    let request_id = if matches!(corruption, Corruption::WrongRequestId) {
        vec![0x7f]
    } else {
        request.request_id.clone()
    };
    let mut pdu_content = tlv(0x02, &request_id);
    let error_status = if matches!(corruption, Corruption::AgentError) {
        5
    } else {
        0
    };
    let error_index = if matches!(
        corruption,
        Corruption::AgentError | Corruption::NonzeroSuccessIndex
    ) {
        1
    } else {
        0
    };
    pdu_content.extend_from_slice(&tlv(0x02, &[error_status]));
    pdu_content.extend_from_slice(&tlv(0x02, &[error_index]));
    pdu_content.extend_from_slice(&tlv(0x30, &list_content));
    let pdu_tag = if matches!(corruption, Corruption::WrongPdu) {
        0xa0
    } else {
        0xa2
    };

    let version = if matches!(corruption, Corruption::WrongVersion) {
        0
    } else {
        1
    };
    let community = if matches!(corruption, Corruption::WrongCommunity) {
        b"wrong".as_slice()
    } else {
        request.community.as_slice()
    };
    let mut message = tlv(0x02, &[version]);
    message.extend_from_slice(&tlv(0x04, community));
    message.extend_from_slice(&tlv(pdu_tag, &pdu_content));
    let mut response = tlv(0x30, &message);
    if matches!(corruption, Corruption::Trailing) {
        response.push(0);
    }
    if matches!(corruption, Corruption::Malformed) {
        response[1] = response[1].saturating_add(1);
    }
    response
}

#[derive(Debug)]
struct AgentReport {
    requests: usize,
    cursors: Vec<Vec<u32>>,
    pdu_tags: Vec<u8>,
}

fn start_agent(actions: Vec<Action>) -> (u16, std::thread::JoinHandle<AgentReport>) {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind mock WALK agent");
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set fixture timeout");
    let port = socket.local_addr().expect("fixture address").port();
    let handle = std::thread::spawn(move || {
        let mut report = AgentReport {
            requests: 0,
            cursors: Vec::new(),
            pdu_tags: Vec::new(),
        };
        let mut packet = [0u8; 65_535];
        let mut remembered_response: Option<Vec<u8>> = None;
        for action in actions {
            let Ok((length, peer)) = socket.recv_from(&mut packet) else {
                break;
            };
            report.requests += 1;
            let Some(request) = parse_request(&packet[..length]) else {
                break;
            };
            assert_eq!(
                request.community.as_slice(),
                SECRET_CANARY.as_bytes(),
                "WALK request carried an unexpected community"
            );
            report.cursors.push(request.cursor.clone());
            report.pdu_tags.push(request.pdu_tag);
            match action {
                Action::Respond {
                    oid,
                    value,
                    corruption,
                } => {
                    let response = build_response(&request, &oid, &value, corruption);
                    let _ = socket.send_to(&response, peer);
                }
                Action::RememberResponse { oid, value } => {
                    remembered_response =
                        Some(build_response(&request, &oid, &value, Corruption::None));
                }
                Action::SendRememberedThenRespond { oid, value } => {
                    let old = remembered_response
                        .take()
                        .expect("fixture remembered response");
                    let current = build_response(&request, &oid, &value, Corruption::None);
                    let _ = socket.send_to(&old, peer);
                    let _ = socket.send_to(&current, peer);
                }
                Action::ForgedThenRespond { oid, value } => {
                    let response = build_response(&request, &oid, &value, Corruption::None);
                    let attacker = UdpSocket::bind("127.0.0.1:0").expect("bind forged source");
                    let _ = attacker.send_to(&response, peer);
                    let _ = socket.send_to(&response, peer);
                }
                Action::Drop => {}
            }
        }
        report
    });
    (port, handle)
}

fn enabled_env() -> [(&'static str, &'static str); 3] {
    [
        ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
        ("NTNT_NETMON_ENABLE", "1"),
        ("NTNT_NET_ALLOW_PRIVATE", "1"),
    ]
}

fn walk_source(port: u16, options: &str, body: &str) -> String {
    format!(
        r#"
import {{ require_secret }} from "std/secrets"
import {{ snmp_walk }} from "std/netmon"

let auth = map {{
    "version": "2c",
    "community": require_secret("SNMP_TEST_COMMUNITY")
}}
match snmp_walk(
    "127.0.0.1",
    auth,
    "1.3.6.1.2.1.2.2",
    map {{
        "port": {port},
        "timeout_ms": 1000,
        "allow_private": true,
        {options}
    }}
) {{
    Ok(result) => {{ {body} }},
    Err(error) => print("ERR: " + error)
}}
"#
    )
}

fn response(oid: &[u32], value: ValueSpec) -> Action {
    corrupted_response(oid, value, Corruption::None)
}

fn corrupted_response(oid: &[u32], value: ValueSpec, corruption: Corruption) -> Action {
    Action::Respond {
        oid: oid.to_vec(),
        value,
        corruption,
    }
}

#[test]
fn snmp_walk_collects_ordered_rows_and_stops_outside_the_root() {
    let root = vec![1, 3, 6, 1, 2, 1, 2, 2];
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 1, 1];
    let second = vec![1, 3, 6, 1, 2, 1, 2, 2, 1, 2];
    let outside = vec![1, 3, 6, 1, 2, 1, 3, 1];
    let (port, agent) = start_agent(vec![
        response(&first, ValueSpec::Integer(7)),
        response(&second, ValueSpec::Octets(b"eth0".to_vec())),
        response(&outside, ValueSpec::Integer(9)),
    ]);
    let source = walk_source(
        port,
        r#""max_results": 10, "retries": 0"#,
        r#"
let values = result["values"]
print(result["root_oid"])
print(result["requests"])
print(result["attempts"])
print(result["complete"])
print(result["stop_reason"])
print(len(values))
print(values[0]["oid"])
print(values[1]["value"])
"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(report.requests, 3);
    assert_eq!(report.pdu_tags, [0xa1, 0xa1, 0xa1]);
    assert_eq!(report.cursors, [root, first.clone(), second.clone()]);
    assert_eq!(
        stdout.lines().map(str::trim).collect::<Vec<_>>(),
        [
            "1.3.6.1.2.1.2.2",
            "3",
            "3",
            "true",
            "out_of_subtree",
            "2",
            "1.3.6.1.2.1.2.2.1.1",
            "eth0",
        ]
    );
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_walk_ignores_forged_source_and_accepts_connected_agent_response() {
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let outside = vec![1, 3, 6, 1, 2, 1, 3];
    let (port, agent) = start_agent(vec![
        Action::ForgedThenRespond {
            oid: first.clone(),
            value: ValueSpec::Integer(1),
        },
        response(&outside, ValueSpec::Integer(2)),
    ]);
    let source = walk_source(
        port,
        r#""max_results": 10, "retries": 0"#,
        r#"print(len(result["values"]))"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(report.requests, 2);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(stdout.trim(), "1");
}

#[test]
fn snmp_walk_ignores_delayed_old_attempt_before_current_response() {
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let outside = vec![1, 3, 6, 1, 2, 1, 3];
    let (port, agent) = start_agent(vec![
        Action::RememberResponse {
            oid: first.clone(),
            value: ValueSpec::Integer(1),
        },
        Action::SendRememberedThenRespond {
            oid: first,
            value: ValueSpec::Integer(1),
        },
        response(&outside, ValueSpec::Integer(2)),
    ]);
    let source = walk_source(
        port,
        r#""max_results": 10, "retries": 1, "timeout_ms": 1000"#,
        r#"
print(result["requests"])
print(result["attempts"])
print(len(result["values"]))
"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(report.requests, 3);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(
        stdout.lines().map(str::trim).collect::<Vec<_>>(),
        ["2", "3", "1"]
    );
}

#[test]
fn snmp_walk_omits_protocol_exceptions_and_completes_empty_walk() {
    let root = vec![1, 3, 6, 1, 2, 1, 2, 2];
    for (value, reason) in [
        (ValueSpec::EndOfMibView, "end_of_mib_view"),
        (ValueSpec::NoSuchObject, "no_such_object"),
        (ValueSpec::NoSuchInstance, "no_such_instance"),
    ] {
        let (port, agent) = start_agent(vec![response(&root, value)]);
        let source = walk_source(
            port,
            r#""max_results": 10, "retries": 0"#,
            r#"
print(result["complete"])
print(result["stop_reason"])
print(len(result["values"]))
"#,
        );
        let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
        let report = agent.join().expect("fixture thread");
        assert_eq!(report.requests, 1);
        assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
        assert_eq!(
            stdout.lines().map(str::trim).collect::<Vec<_>>(),
            ["true", reason, "0"]
        );
        assert!(!stdout.contains(SECRET_CANARY));
        assert!(!stderr.contains(SECRET_CANARY));
    }
}

#[test]
fn snmp_walk_rejects_terminal_exception_with_mismatched_cursor_oid() {
    let mismatched = vec![1, 3, 6, 1, 2, 1, 2, 2, 99];
    let (port, agent) = start_agent(vec![response(&mismatched, ValueSpec::EndOfMibView)]);
    let source = walk_source(
        port,
        r#""max_results": 10, "retries": 0"#,
        r#"print("unexpected success")"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(report.requests, 1);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("terminal exception OID"), "{stdout}");
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_walk_exact_max_uses_lookahead_terminal_reason() {
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let second = vec![1, 3, 6, 1, 2, 1, 2, 2, 2];
    let (port, agent) = start_agent(vec![
        response(&first, ValueSpec::Integer(1)),
        response(&second, ValueSpec::Integer(2)),
        response(&second, ValueSpec::EndOfMibView),
    ]);
    let source = walk_source(
        port,
        r#""max_results": 2, "retries": 0"#,
        r#"
print(result["complete"])
print(result["stop_reason"])
print(result["requests"])
print(len(result["values"]))
"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(report.requests, 3, "look-ahead request is mandatory");
    assert_eq!(
        stdout.lines().map(str::trim).collect::<Vec<_>>(),
        ["true", "end_of_mib_view", "3", "2"]
    );
}

#[test]
fn snmp_walk_partial_limit_omits_valid_lookahead() {
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let lookahead = vec![1, 3, 6, 1, 2, 1, 2, 2, 2];
    let (port, agent) = start_agent(vec![
        response(&first, ValueSpec::Integer(1)),
        response(&lookahead, ValueSpec::Integer(2)),
    ]);
    let source = walk_source(
        port,
        r#""max_results": 1, "on_limit": "partial", "retries": 0"#,
        r#"
print(result["complete"])
print(result["stop_reason"])
print(result["requests"])
print(len(result["values"]))
print(result["values"][0]["oid"])
"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(report.requests, 2);
    assert_eq!(
        stdout.lines().map(str::trim).collect::<Vec<_>>(),
        ["false", "max_results", "2", "1", "1.3.6.1.2.1.2.2.1",]
    );
}

#[test]
fn snmp_walk_limit_error_returns_no_partial_telemetry() {
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let lookahead = vec![1, 3, 6, 1, 2, 1, 2, 2, 2];
    let (port, agent) = start_agent(vec![
        response(&first, ValueSpec::Integer(1)),
        response(&lookahead, ValueSpec::Integer(2)),
    ]);
    let source = walk_source(
        port,
        r#""max_results": 1, "retries": 0"#,
        r#"print("unexpected success")"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(report.requests, 2);
    assert_eq!(stdout.trim(), "ERR: SNMP WALK exceeded max_results (1)");
    assert!(!stdout.contains("1.3.6.1.2.1.2.2.1"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_walk_rejects_equal_repeated_and_descending_oids() {
    let root = vec![1, 3, 6, 1, 2, 1, 2, 2];
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 2];
    let descending = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let cases = vec![
        vec![response(&root, ValueSpec::Integer(1))],
        vec![
            response(&first, ValueSpec::Integer(1)),
            response(&first, ValueSpec::Integer(2)),
        ],
        vec![
            response(&first, ValueSpec::Integer(1)),
            response(&descending, ValueSpec::Integer(2)),
        ],
    ];

    for actions in cases {
        let (port, agent) = start_agent(actions);
        let source = walk_source(
            port,
            r#""max_results": 10, "retries": 0"#,
            r#"print("unexpected success")"#,
        );
        let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
        let _ = agent.join().expect("fixture thread");
        assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
        assert!(
            stdout.contains("strictly increasing"),
            "unexpected ordering result: {stdout}"
        );
        assert!(!stdout.contains(SECRET_CANARY));
        assert!(!stderr.contains(SECRET_CANARY));
    }
}

#[test]
fn snmp_walk_rejects_malformed_mismatched_and_oversized_responses() {
    let row = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let cases = [
        (Corruption::WrongVersion, "version"),
        (Corruption::WrongCommunity, "community"),
        (Corruption::WrongPdu, "PDU type"),
        (Corruption::WrongRequestId, "failed after 1 bounded attempt"),
        (Corruption::Trailing, "datagram"),
        (Corruption::Malformed, "truncated"),
        (Corruption::MultipleVarbinds, "exactly 1"),
        (Corruption::Oversized, "exceeds 8192 bytes"),
        (Corruption::AgentError, "error status"),
        (Corruption::NonzeroSuccessIndex, "error index"),
    ];

    for (corruption, expected) in cases {
        let (port, agent) = start_agent(vec![corrupted_response(
            &row,
            ValueSpec::Integer(1),
            corruption,
        )]);
        let source = walk_source(
            port,
            r#""max_results": 10, "retries": 0"#,
            r#"print("unexpected success")"#,
        );
        let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
        let report = agent.join().expect("fixture thread");
        assert_eq!(report.requests, 1);
        assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
        assert!(
            stdout.contains(expected),
            "expected {expected:?} for {corruption:?}, got {stdout:?}"
        );
        assert!(!stdout.contains(SECRET_CANARY));
        assert!(!stderr.contains(SECRET_CANARY));
    }
}

#[test]
fn snmp_walk_discards_prior_rows_when_a_later_response_is_invalid() {
    let first = vec![1, 3, 6, 1, 2, 1, 2, 2, 1];
    let second = vec![1, 3, 6, 1, 2, 1, 2, 2, 2];
    let (port, agent) = start_agent(vec![
        response(&first, ValueSpec::Integer(1)),
        corrupted_response(&second, ValueSpec::Integer(2), Corruption::WrongCommunity),
    ]);
    let source = walk_source(
        port,
        r#""max_results": 10, "retries": 0"#,
        r#"print("unexpected success")"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let report = agent.join().expect("fixture thread");
    assert_eq!(report.requests, 2);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("community mismatch"), "{stdout}");
    assert!(!stdout.contains("1.3.6.1.2.1.2.2.1"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_walk_retry_budget_remains_inside_one_global_deadline() {
    let (port, agent) = start_agent(vec![Action::Drop, Action::Drop]);
    let source = walk_source(
        port,
        r#""max_results": 10, "retries": 1, "timeout_ms": 150"#,
        r#"print("unexpected success")"#,
    );
    let started = Instant::now();
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let elapsed = started.elapsed();
    let report = agent.join().expect("fixture thread");
    assert_eq!(report.requests, 2);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("global timeout") || stdout.contains("bounded attempt"));
    assert!(
        elapsed < Duration::from_millis(700),
        "deadline multiplied across retries: {elapsed:?}"
    );
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_walk_preflight_and_policy_fail_before_transport() {
    let source = walk_source(
        9,
        r#""max_results": 2048, "retries": 1"#,
        r#"print("unexpected success")"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("4098 possible attempts"), "{stdout}");

    let source = walk_source(
        9,
        r#""max_results": 10, "retries": 0, "unknown": true"#,
        r#"print("unexpected success")"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("unknown field"), "{stdout}");

    let source = walk_source(
        9,
        r#""max_results": 10, "retries": 0"#,
        r#"print("unexpected success")"#,
    );
    let env = [("SNMP_TEST_COMMUNITY", SECRET_CANARY)];
    let (stdout, stderr, code) = run_ntnt_code(&source, &env);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("std/netmon is disabled"), "{stdout}");

    let env = [
        ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
        ("NTNT_NETMON_ENABLE", "1"),
    ];
    let (stdout, stderr, code) = run_ntnt_code(&source, &env);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains("private") || stdout.contains("loopback"),
        "{stdout}"
    );

    for text in [stdout, stderr] {
        assert!(!text.contains(SECRET_CANARY));
    }
}

#[test]
fn snmp_walk_rejects_plaintext_community_without_rendering_it() {
    let source = r#"
import { snmp_walk } from "std/netmon"
let auth = map { "version": "2c", "community": "walk-community-canary-never-render" }
match snmp_walk("127.0.0.1", auth, "1.3.6.1", map { "allow_private": true }) {
    Ok(_) => print("unexpected success"),
    Err(error) => print("ERR: " + error)
}
"#;
    let env = [("NTNT_NETMON_ENABLE", "1"), ("NTNT_NET_ALLOW_PRIVATE", "1")];
    let (stdout, stderr, code) = run_ntnt_code(source, &env);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("must be Secret"), "{stdout}");
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}
