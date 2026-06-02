//! Integration tests for std/net Phase 1.
//!
//! These exercise the public ntnt stdlib surface. They intentionally start red
//! before the std/net module exists.

use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let temp_dir = std::env::temp_dir();
    temp_dir
        .join(format!(
            "ntnt_std_net_{}_{}_{}.tnt",
            prefix,
            std::process::id(),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

fn ntnt_binary() -> String {
    let exe = std::env::consts::EXE_SUFFIX;
    let debug_path = format!("./target/debug/ntnt{}", exe);
    let release_path = format!("./target/release/ntnt{}", exe);

    if std::path::Path::new(&debug_path).exists() {
        debug_path
    } else if std::path::Path::new(&release_path).exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
    }
}

fn run_ntnt_code(code: &str) -> (String, String, i32) {
    run_ntnt_code_with_env(code, &[])
}

fn run_ntnt_code_with_env(code: &str, envs: &[(&str, &str)]) -> (String, String, i32) {
    let test_file = unique_test_file("test");

    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");
    drop(file);

    let mut cmd = Command::new(ntnt_binary());
    cmd.args(["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("NTNT_ENV", "development");
    for (key, value) in envs {
        cmd.env(key, value);
    }

    let output = cmd.output().expect("Failed to execute ntnt");
    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn ip_parse_reports_ipv4_network_calculator_fields() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { ip_parse } from "std/net"

match ip_parse("192.168.1.0/24") {
    Ok(info) => {
        print(info["version"])
        print(info["network"])
        print(info["broadcast"])
        print(info["netmask"])
        print(info["wildcard_mask"])
        print(info["total_addresses"])
        print(info["usable_hosts"])
        print(info["is_private"])
    },
    Err(e) => print("ERR: " + e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    assert!(lines.contains(&"4"), "stdout: {stdout}");
    assert!(lines.contains(&"192.168.1.0"), "stdout: {stdout}");
    assert!(lines.contains(&"192.168.1.255"), "stdout: {stdout}");
    assert!(lines.contains(&"255.255.255.0"), "stdout: {stdout}");
    assert!(lines.contains(&"0.0.0.255"), "stdout: {stdout}");
    assert!(lines.contains(&"256"), "stdout: {stdout}");
    assert!(lines.contains(&"254"), "stdout: {stdout}");
    assert!(lines.contains(&"true"), "stdout: {stdout}");
}

#[test]
fn ip_parse_reports_ipv6_without_overflow() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { ip_parse } from "std/net"

match ip_parse("2001:db8::/64") {
    Ok(info) => {
        print(info["version"])
        print(info["ip"])
        print(info["network"])
        print(info["first"])
        print(info["last"])
        print(info["total_addresses"])
        print(info["is_documentation"])
    },
    Err(e) => print("ERR: " + e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("6"), "stdout: {stdout}");
    assert!(stdout.contains("2001:db8::"), "stdout: {stdout}");
    assert!(
        stdout.contains("2001:db8::ffff:ffff:ffff:ffff"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("18446744073709551616"), "stdout: {stdout}");
    assert!(stdout.contains("true"), "stdout: {stdout}");
}

#[test]
fn subnet_helpers_cover_contains_overlaps_split_supernet_and_summarize() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { join } from "std/string"
import { subnet_contains, subnet_overlaps, subnet_split, subnet_supernet, subnet_summarize, ip_range_to_cidrs } from "std/net"

match subnet_contains("10.0.0.0/8", "10.50.0.0/16") { Ok(v) => print("contains=" + str(v)), Err(e) => print("ERR " + e) }
match subnet_overlaps("10.0.0.0/24", "10.0.0.128/25") { Ok(v) => print("overlaps=" + str(v)), Err(e) => print("ERR " + e) }
match subnet_split("192.168.1.0/24", 26) { Ok(v) => print("split=" + join(v, ",")), Err(e) => print("ERR " + e) }
match subnet_supernet("192.168.1.0/24") { Ok(v) => print("supernet=" + v), Err(e) => print("ERR " + e) }
match subnet_summarize(["10.0.0.0/25", "10.0.0.128/25"]) { Ok(v) => print("summary=" + join(v, ",")), Err(e) => print("ERR " + e) }
match ip_range_to_cidrs("192.168.1.20", "192.168.1.31") { Ok(v) => print("range=" + join(v, ",")), Err(e) => print("ERR " + e) }
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("contains=true"), "stdout: {stdout}");
    assert!(stdout.contains("overlaps=true"), "stdout: {stdout}");
    assert!(
        stdout.contains("split=192.168.1.0/26,192.168.1.64/26,192.168.1.128/26,192.168.1.192/26"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("supernet=192.168.0.0/23"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("summary=10.0.0.0/24"), "stdout: {stdout}");
    assert!(
        stdout.contains("range=192.168.1.20/30,192.168.1.24/29"),
        "stdout: {stdout}"
    );
}

#[test]
fn subnet_split_rejects_explosive_results() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { subnet_split } from "std/net"

match subnet_split("2001:db8::/64", 128) {
    Ok(v) => print("unexpected"),
    Err(e) => print(e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("too many subnets"), "stdout: {stdout}");
}

#[test]
fn ping_auto_uses_unprivileged_tcp_fallback_for_private_monitoring() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let _ = listener.accept();
    });

    let code = format!(
        r#"
import {{ ping }} from "std/net"

match ping("127.0.0.1", map {{ "allow_private": true, "tcp_ports": [{port}], "timeout_ms": 1000 }}) {{
    Ok(info) => {{
        print(info["reachable"])
        print(info["method"])
        print(info["connected_port"])
    }},
    Err(e) => print("ERR: " + e)
}}
"#
    );

    let (stdout, stderr, exit_code) =
        run_ntnt_code_with_env(&code, &[("NTNT_NET_ALLOW_PRIVATE", "1")]);
    let _ = handle.join();

    assert_eq!(exit_code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("true"), "stdout: {stdout}");
    assert!(stdout.contains("tcp"), "stdout: {stdout}");
    assert!(stdout.contains(&port.to_string()), "stdout: {stdout}");
}

#[test]
fn ping_private_target_requires_process_level_opt_in() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { ping } from "std/net"

match ping("127.0.0.1", map { "allow_private": true, "method": "tcp", "tcp_ports": [9], "timeout_ms": 100 }) {
    Ok(info) => print("unexpected"),
    Err(e) => print(e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains("private targets require NTNT_NET_ALLOW_PRIVATE=1"),
        "stdout: {stdout}"
    );
}

#[test]
fn ping_rejects_ipv4_mapped_and_special_ranges_by_default() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { ping } from "std/net"

match ping("::ffff:127.0.0.1", map { "method": "tcp", "tcp_ports": [9], "timeout_ms": 100 }) {
    Ok(info) => print("unexpected mapped"),
    Err(e) => print("mapped=" + e)
}

match ping("224.0.0.1", map { "method": "tcp", "tcp_ports": [9], "timeout_ms": 100 }) {
    Ok(info) => print("unexpected multicast"),
    Err(e) => print("multicast=" + e)
}

match ping("192.0.2.1", map { "method": "tcp", "tcp_ports": [9], "timeout_ms": 100 }) {
    Ok(info) => print("unexpected documentation"),
    Err(e) => print("documentation=" + e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains("mapped=Network target denied by policy"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("multicast=Network target denied by policy"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("documentation=Network target denied by policy"),
        "stdout: {stdout}"
    );
}
