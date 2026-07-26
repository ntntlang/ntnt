//! Integration tests for the first std/netmon SNMP slice.

use std::fs;
use std::io::Write;
use std::net::UdpSocket;
use std::ops::Range;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SECRET_CANARY: &str = "netmon-community-canary-never-render";
static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    std::env::temp_dir()
        .join(format!(
            "ntnt_std_netmon_{prefix}_{}_{}.tnt",
            std::process::id(),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

fn ntnt_binary() -> String {
    let exe = std::env::consts::EXE_SUFFIX;
    for path in [
        format!("./target/debug/ntnt{exe}"),
        format!("./target/dev-release/ntnt{exe}"),
        format!("./target/release/ntnt{exe}"),
    ] {
        if std::path::Path::new(&path).exists() {
            return path;
        }
    }
    panic!("No ntnt binary found. Run 'cargo build' first.");
}

fn run_ntnt_code(code: &str, envs: &[(&str, &str)]) -> (String, String, i32) {
    let test_file = unique_test_file("test");
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

fn read_ber_length(bytes: &[u8], offset: &mut usize) -> Option<usize> {
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

fn read_tlv(bytes: &[u8], offset: &mut usize) -> Option<(u8, usize, Range<usize>)> {
    let tag_offset = *offset;
    let tag = *bytes.get(*offset)?;
    *offset += 1;
    let length = read_ber_length(bytes, offset)?;
    let start = *offset;
    let end = start.checked_add(length)?;
    if end > bytes.len() {
        return None;
    }
    *offset = end;
    Some((tag, tag_offset, start..end))
}

struct RequestLayout {
    pdu_tag_offset: usize,
    version_value_offset: usize,
}

fn inspect_get_request(bytes: &[u8]) -> Option<RequestLayout> {
    let mut root_offset = 0;
    let (outer_tag, _, outer) = read_tlv(bytes, &mut root_offset)?;
    if outer_tag != 0x30 || root_offset != bytes.len() {
        return None;
    }

    let mut message_offset = outer.start;
    let (version_tag, _, version) = read_tlv(bytes, &mut message_offset)?;
    if version_tag != 0x02 || &bytes[version.clone()] != [1] {
        return None;
    }
    let (community_tag, _, community) = read_tlv(bytes, &mut message_offset)?;
    if community_tag != 0x04 || &bytes[community] != SECRET_CANARY.as_bytes() {
        return None;
    }
    let (pdu_tag, pdu_tag_offset, pdu) = read_tlv(bytes, &mut message_offset)?;
    if pdu_tag != 0xa0 || message_offset != outer.end {
        return None;
    }

    let mut pdu_offset = pdu.start;
    let (request_id_tag, _, request_id) = read_tlv(bytes, &mut pdu_offset)?;
    let (error_status_tag, _, error_status) = read_tlv(bytes, &mut pdu_offset)?;
    let (error_index_tag, _, error_index) = read_tlv(bytes, &mut pdu_offset)?;
    let (varbind_list_tag, _, varbind_list) = read_tlv(bytes, &mut pdu_offset)?;
    if request_id_tag != 0x02
        || request_id.is_empty()
        || error_status_tag != 0x02
        || &bytes[error_status] != [0]
        || error_index_tag != 0x02
        || &bytes[error_index] != [0]
        || varbind_list_tag != 0x30
        || pdu_offset != pdu.end
    {
        return None;
    }

    let mut varbind_offset = varbind_list.start;
    let mut count = 0;
    while varbind_offset < varbind_list.end {
        let (varbind_tag, _, varbind) = read_tlv(bytes, &mut varbind_offset)?;
        if varbind_tag != 0x30 {
            return None;
        }
        let mut field_offset = varbind.start;
        let (oid_tag, _, oid) = read_tlv(bytes, &mut field_offset)?;
        let (value_tag, _, value) = read_tlv(bytes, &mut field_offset)?;
        if oid_tag != 0x06
            || oid.is_empty()
            || value_tag != 0x05
            || !value.is_empty()
            || field_offset != varbind.end
        {
            return None;
        }
        count += 1;
    }
    if count == 0 || varbind_offset != varbind_list.end {
        return None;
    }

    Some(RequestLayout {
        pdu_tag_offset,
        version_value_offset: version.start,
    })
}

fn validated_response(request: &[u8]) -> Option<Vec<u8>> {
    let layout = inspect_get_request(request)?;
    let mut response = request.to_vec();
    response[layout.pdu_tag_offset] = 0xa2;
    Some(response)
}

#[derive(Clone, Copy)]
enum AgentBehavior {
    Respond,
    DropFirst,
    Silent,
    Delayed(Duration),
    MismatchedOid,
    WrongVersion,
    TrailingBytes,
    OversizedResponse,
}

fn start_mock_snmp_agent(behavior: AgentBehavior) -> (u16, std::thread::JoinHandle<usize>) {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind mock SNMP agent");
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("set agent timeout");
    let port = socket.local_addr().expect("agent address").port();
    let handle = std::thread::spawn(move || {
        let request_limit = match behavior {
            AgentBehavior::DropFirst => 2,
            AgentBehavior::Silent => 4,
            _ => 1,
        };
        let mut received_requests = 0;
        let mut request = [0u8; 65_535];
        for index in 0..request_limit {
            let Ok((length, peer)) = socket.recv_from(&mut request) else {
                return received_requests;
            };
            received_requests += 1;
            let Some(mut response) = validated_response(&request[..length]) else {
                return received_requests;
            };
            if matches!(behavior, AgentBehavior::DropFirst) && index == 0 {
                continue;
            }
            if matches!(behavior, AgentBehavior::Silent) {
                continue;
            }
            match behavior {
                AgentBehavior::Delayed(delay) => std::thread::sleep(delay),
                AgentBehavior::MismatchedOid => {
                    let encoded_oid = [0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00];
                    let Some(offset) = response
                        .windows(encoded_oid.len())
                        .position(|window| window == encoded_oid)
                    else {
                        return received_requests;
                    };
                    response[offset + encoded_oid.len() - 1] = 0x01;
                }
                AgentBehavior::WrongVersion => {
                    let layout = inspect_get_request(&request[..length]).expect("validated layout");
                    response[layout.version_value_offset] = 0;
                }
                AgentBehavior::TrailingBytes => response.push(0),
                AgentBehavior::OversizedResponse => response = vec![0; 8 * 1024 + 1],
                AgentBehavior::Respond | AgentBehavior::DropFirst | AgentBehavior::Silent => {}
            }
            let _ = socket.send_to(&response, peer);
            return received_requests;
        }
        received_requests
    });
    (port, handle)
}

fn snmp_source(port: u16, timeout_ms: u64, retries: u64, body: &str) -> String {
    format!(
        r#"
import {{ require_secret }} from "std/secrets"
import {{ snmp_get }} from "std/netmon"

let auth = map {{
    "version": "2c",
    "community": require_secret("SNMP_TEST_COMMUNITY")
}}
match snmp_get(
    "127.0.0.1",
    auth,
    ["1.3.6.1.2.1.1.1.0", "1.3.6.1.2.1.1.5.0"],
    map {{
        "port": {port},
        "timeout_ms": {timeout_ms},
        "retries": {retries},
        "allow_private": true
    }}
) {{
    Ok(result) => {{ {body} }},
    Err(error) => print("ERR: " + error)
}}
"#
    )
}

fn enabled_env() -> [(&'static str, &'static str); 3] {
    [
        ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
        ("NTNT_NETMON_ENABLE", "1"),
        ("NTNT_NET_ALLOW_PRIVATE", "1"),
    ]
}

#[test]
fn snmp_get_uses_opaque_secret_and_normalizes_a_real_udp_response() {
    let (port, agent) = start_mock_snmp_agent(AgentBehavior::Respond);
    let source = snmp_source(
        port,
        1_000,
        0,
        r#"
let values = result["values"]
print(result["target"])
print(result["address"])
print(result["version"])
print(values[0]["oid"])
print(values[0]["type"])
print(values[1]["oid"])
"#,
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(agent.join().expect("mock agent thread"), 1);
    assert!(!stdout.contains("ERR:"), "stdout: {stdout}");
    assert!(!stdout.contains(SECRET_CANARY), "secret leaked to stdout");
    assert!(!stderr.contains(SECRET_CANARY), "secret leaked to stderr");
    let lines: Vec<_> = stdout.lines().map(str::trim).collect();
    assert_eq!(
        lines,
        [
            "127.0.0.1",
            "127.0.0.1",
            "2c",
            "1.3.6.1.2.1.1.1.0",
            "null",
            "1.3.6.1.2.1.1.5.0"
        ]
    );
}

#[test]
fn snmp_get_retries_within_one_global_deadline() {
    let (port, agent) = start_mock_snmp_agent(AgentBehavior::DropFirst);
    let source = snmp_source(port, 500, 1, r#"print(result["attempts"])"#);
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(agent.join().expect("retrying agent thread"), 2);
    assert_eq!(stdout.trim(), "2", "stdout: {stdout}");
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_get_silent_agent_uses_exact_attempts_inside_one_budget() {
    let (port, agent) = start_mock_snmp_agent(AgentBehavior::Silent);
    let source = snmp_source(port, 200, 3, r#"print("unexpected success")"#);
    let started = Instant::now();
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    let elapsed = started.elapsed();
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(agent.join().expect("silent agent thread"), 4);
    assert!(stdout.contains("failed after 4 bounded attempt(s)"));
    assert!(elapsed < Duration::from_secs(3), "elapsed: {elapsed:?}");
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_get_never_accepts_a_response_after_the_deadline() {
    let (port, agent) = start_mock_snmp_agent(AgentBehavior::Delayed(Duration::from_millis(200)));
    let source = snmp_source(port, 75, 0, r#"print("unexpected success")"#);
    let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(agent.join().expect("delayed agent thread"), 1);
    assert!(stdout.contains("ERR:"), "stdout: {stdout}");
    assert!(!stdout.contains("unexpected success"));
}

#[test]
fn snmp_get_rejects_unrequested_oid_wrong_version_and_trailing_bytes() {
    for (behavior, expected) in [
        (AgentBehavior::MismatchedOid, "OID mismatch"),
        (AgentBehavior::WrongVersion, "version mismatch"),
        (AgentBehavior::TrailingBytes, "trailing byte"),
        (
            AgentBehavior::OversizedResponse,
            "response exceeds 8192 bytes",
        ),
    ] {
        let (port, agent) = start_mock_snmp_agent(behavior);
        let source = snmp_source(port, 500, 0, r#"print("unexpected success")"#);
        let (stdout, stderr, code) = run_ntnt_code(&source, &enabled_env());
        assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
        assert_eq!(agent.join().expect("adversarial agent thread"), 1);
        assert!(stdout.contains(expected), "stdout: {stdout}");
        assert!(!stdout.contains(SECRET_CANARY));
        assert!(!stderr.contains(SECRET_CANARY));
    }
}

#[test]
fn snmp_get_requires_explicit_process_enablement() {
    let source = r#"
import { require_secret } from "std/secrets"
import { snmp_get } from "std/netmon"
let auth = map { "version": "2c", "community": require_secret("SNMP_TEST_COMMUNITY") }
match snmp_get("192.0.2.1", auth, ["1.3.6.1"]) {
    Ok(_) => print("unexpected success"),
    Err(error) => print(error)
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(source, &[("SNMP_TEST_COMMUNITY", SECRET_CANARY)]);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("std/netmon is disabled"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_get_private_target_requires_process_and_call_opt_in() {
    let source = r#"
import { require_secret } from "std/secrets"
import { snmp_get } from "std/netmon"
let auth = map { "version": "2c", "community": require_secret("SNMP_TEST_COMMUNITY") }
match snmp_get(
    "127.0.0.1",
    auth,
    ["1.3.6.1"],
    map { "port": 9, "timeout_ms": 50, "allow_private": true }
) {
    Ok(_) => print("unexpected success"),
    Err(error) => print(error)
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(
        source,
        &[
            ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
            ("NTNT_NETMON_ENABLE", "1"),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("private targets require NTNT_NET_ALLOW_PRIVATE=1"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_get_rejects_hostname_and_plaintext_community_without_echoing_secret() {
    let plaintext_source = format!(
        r#"
import {{ snmp_get }} from "std/netmon"
let auth = map {{ "version": "2c", "community": "{SECRET_CANARY}" }}
match snmp_get("192.0.2.1", auth, ["1.3.6.1"]) {{
    Ok(_) => print("unexpected success"),
    Err(error) => print(error)
}}
"#
    );
    let (stdout, stderr, code) = run_ntnt_code(&plaintext_source, &[("NTNT_NETMON_ENABLE", "1")]);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("auth.community must be Secret"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));

    let hostname_source = r#"
import { require_secret } from "std/secrets"
import { snmp_get } from "std/netmon"
let auth = map { "version": "2c", "community": require_secret("SNMP_TEST_COMMUNITY") }
match snmp_get("router.example.com", auth, ["1.3.6.1"]) {
    Ok(_) => print("unexpected success"),
    Err(error) => print(error)
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(
        hostname_source,
        &[
            ("NTNT_NETMON_ENABLE", "1"),
            ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("target must be an IPv4 or IPv6 literal"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}
