//! Integration tests for the first std/netmon SNMP slice.

use snmp2::{MessageType, Pdu};
use std::fs;
use std::io::Write;
use std::net::UdpSocket;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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

fn skip_tlv(bytes: &[u8], offset: &mut usize, expected_tag: u8) -> Option<()> {
    if *bytes.get(*offset)? != expected_tag {
        return None;
    }
    *offset += 1;
    let length = read_ber_length(bytes, offset)?;
    *offset = offset.checked_add(length)?;
    (*offset <= bytes.len()).then_some(())
}

fn request_pdu_tag_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 0usize;
    if *bytes.get(offset)? != 0x30 {
        return None;
    }
    offset += 1;
    let outer_length = read_ber_length(bytes, &mut offset)?;
    let outer_end = offset.checked_add(outer_length)?;
    if outer_end != bytes.len() {
        return None;
    }
    skip_tlv(bytes, &mut offset, 0x02)?;
    skip_tlv(bytes, &mut offset, 0x04)?;
    (offset < outer_end).then_some(offset)
}

fn validated_response(request: &[u8]) -> Option<Vec<u8>> {
    let pdu = Pdu::from_bytes(request).ok()?;
    if pdu.community != SECRET_CANARY.as_bytes()
        || pdu.message_type != MessageType::GetRequest
        || pdu.varbinds.clone().count() == 0
    {
        return None;
    }
    let tag_offset = request_pdu_tag_offset(request)?;
    let mut response = request.to_vec();
    response[tag_offset] = 0xa2; // GetResponse, preserving request ID and varbinds.
    Some(response)
}

#[derive(Clone, Copy)]
enum AgentBehavior {
    Respond,
    DropFirst,
    MismatchedOid,
}

fn start_mock_snmp_agent(behavior: AgentBehavior) -> (u16, std::thread::JoinHandle<bool>) {
    let socket = UdpSocket::bind("127.0.0.1:0").expect("bind mock SNMP agent");
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("set agent timeout");
    let port = socket.local_addr().expect("agent address").port();
    let handle = std::thread::spawn(move || {
        let request_count = if matches!(behavior, AgentBehavior::DropFirst) {
            2
        } else {
            1
        };
        let mut request = [0u8; 65_507];
        for index in 0..request_count {
            let Ok((length, peer)) = socket.recv_from(&mut request) else {
                return false;
            };
            if matches!(behavior, AgentBehavior::DropFirst) && index == 0 {
                continue;
            }
            let Some(mut response) = validated_response(&request[..length]) else {
                return false;
            };
            if matches!(behavior, AgentBehavior::MismatchedOid) {
                // BER payload for 1.3.6.1.2.1.1.1.0. Change the instance arc to .1.
                let encoded_oid = [0x2b, 0x06, 0x01, 0x02, 0x01, 0x01, 0x01, 0x00];
                let Some(offset) = response
                    .windows(encoded_oid.len())
                    .position(|window| window == encoded_oid)
                else {
                    return false;
                };
                response[offset + encoded_oid.len() - 1] = 0x01;
            }
            return socket.send_to(&response, peer).is_ok();
        }
        false
    });
    (port, handle)
}

#[test]
fn snmp_get_uses_opaque_secret_and_normalizes_a_real_udp_response() {
    let (port, agent) = start_mock_snmp_agent(AgentBehavior::Respond);
    let source = format!(
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
    map {{ "port": {port}, "timeout_ms": 1000, "allow_private": true }}
) {{
    Ok(result) => {{
        let values = result["values"]
        print(result["target"])
        print(result["address"])
        print(result["version"])
        print(values[0]["oid"])
        print(values[0]["type"])
        print(values[1]["oid"])
    }},
    Err(error) => print("ERR: " + error)
}}
"#
    );
    let (stdout, stderr, code) = run_ntnt_code(
        &source,
        &[
            ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
            ("NTNT_NET_ALLOW_PRIVATE", "1"),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        agent.join().expect("mock agent thread"),
        "agent rejected request"
    );
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
    let source = format!(
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
    ["1.3.6.1.2.1.1.1.0"],
    map {{
        "port": {port},
        "timeout_ms": 500,
        "retries": 1,
        "allow_private": true
    }}
) {{
    Ok(result) => print(result["attempts"]),
    Err(error) => print("ERR: " + error)
}}
"#
    );
    let (stdout, stderr, code) = run_ntnt_code(
        &source,
        &[
            ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
            ("NTNT_NET_ALLOW_PRIVATE", "1"),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        agent.join().expect("retrying agent thread"),
        "agent rejected retry"
    );
    assert_eq!(stdout.trim(), "2", "stdout: {stdout}");
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_get_rejects_a_response_for_an_unrequested_oid() {
    let (port, agent) = start_mock_snmp_agent(AgentBehavior::MismatchedOid);
    let source = format!(
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
    ["1.3.6.1.2.1.1.1.0"],
    map {{ "port": {port}, "timeout_ms": 500, "allow_private": true }}
) {{
    Ok(_) => print("unexpected success"),
    Err(error) => print(error)
}}
"#
    );
    let (stdout, stderr, code) = run_ntnt_code(
        &source,
        &[
            ("SNMP_TEST_COMMUNITY", SECRET_CANARY),
            ("NTNT_NET_ALLOW_PRIVATE", "1"),
        ],
    );
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        agent.join().expect("mismatched agent thread"),
        "agent failed to return mismatch"
    );
    assert!(
        stdout.contains("SNMP response OID mismatch at item 1"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_get_private_target_requires_process_and_call_opt_in() {
    let source = r#"
import { require_secret } from "std/secrets"
import { snmp_get } from "std/netmon"

let auth = map {
    "version": "2c",
    "community": require_secret("SNMP_TEST_COMMUNITY")
}
match snmp_get(
    "127.0.0.1",
    auth,
    ["1.3.6.1.2.1.1.1.0"],
    map { "port": 9, "timeout_ms": 50, "allow_private": true }
) {
    Ok(_) => print("unexpected success"),
    Err(error) => print(error)
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(source, &[("SNMP_TEST_COMMUNITY", SECRET_CANARY)]);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("private targets require NTNT_NET_ALLOW_PRIVATE=1"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}

#[test]
fn snmp_get_rejects_plaintext_community_without_echoing_it() {
    let source = format!(
        r#"
import {{ snmp_get }} from "std/netmon"

let auth = map {{ "version": "2c", "community": "{SECRET_CANARY}" }}
match snmp_get("router.example.com", auth, ["1.3.6.1.2.1.1.1.0"]) {{
    Ok(_) => print("unexpected success"),
    Err(error) => print(error)
}}
"#
    );
    let (stdout, stderr, code) = run_ntnt_code(&source, &[]);
    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("auth.community must be Secret"));
    assert!(!stdout.contains(SECRET_CANARY));
    assert!(!stderr.contains(SECRET_CANARY));
}
