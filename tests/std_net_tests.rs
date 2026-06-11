//! Integration tests for std/net Phase 1 public behavior.

use rcgen::generate_simple_self_signed;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ServerConfig, ServerConnection};
use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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

fn start_local_tls_server(expected_connections: usize) -> (u16, std::thread::JoinHandle<usize>) {
    let certified = generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("generate local TLS certificate");
    let cert = CertificateDer::from(certified.cert.der().to_vec());
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(certified.key_pair.serialize_der()));
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![cert], key)
            .expect("build local TLS server config"),
    );
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local TLS listener");
    listener
        .set_nonblocking(true)
        .expect("make local TLS listener nonblocking");
    let port = listener.local_addr().unwrap().port();

    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut accepted = 0;
        while accepted < expected_connections && Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    accepted += 1;
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                    let Ok(mut conn) = ServerConnection::new(config.clone()) else {
                        continue;
                    };
                    while conn.is_handshaking() && Instant::now() < deadline {
                        match conn.complete_io(&mut stream) {
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                    if !conn.is_handshaking() {
                        let _ = conn.writer().write_all(b"ok");
                        let _ = conn.complete_io(&mut stream);
                        std::thread::sleep(Duration::from_millis(50));
                    }
                }
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        accepted
    });

    (port, handle)
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
fn dns_lookup_rejects_unknown_record_types_as_result_error() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { dns_lookup } from "std/net"

match dns_lookup("example.com", "BOGUS", map { "timeout_ms": 100 }) {
    Ok(records) => print("unexpected"),
    Err(e) => print(e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains("record_type must be one of"),
        "stdout: {stdout}"
    );
}

#[test]
fn dns_lookup_rejects_operational_meta_record_types_as_result_error() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { dns_lookup } from "std/net"

for record_type in ["ANY", "AXFR", "IXFR", "OPT", "TSIG", "ZERO"] {
    match dns_lookup("example.com", record_type, map { "timeout_ms": 100 }) {
        Ok(records) => print("unexpected"),
        Err(e) => print(contains(e, "record_type must be one of"))
    }
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(stdout.matches("true").count(), 6, "stdout: {stdout}");
}

#[test]
fn dns_reverse_rejects_invalid_ip_as_result_error() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { dns_reverse } from "std/net"

match dns_reverse("not-an-ip", map { "timeout_ms": 100 }) {
    Ok(records) => print("unexpected"),
    Err(e) => print(e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("valid IP address"), "stdout: {stdout}");
}

#[test]
fn dns_lookup_external_smoke_is_opt_in() {
    if std::env::var("NTNT_NET_EXTERNAL_DNS_TESTS").as_deref() != Ok("1") {
        return;
    }

    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { dns_lookup, dns_reverse } from "std/net"

match dns_lookup("example.com", "A", map { "timeout_ms": 2000 }) {
    Ok(records) => print(len(records) >= 0),
    Err(e) => print("lookup=" + e)
}

match dns_reverse("8.8.8.8", map { "timeout_ms": 2000 }) {
    Ok(names) => print(len(names) >= 0),
    Err(e) => print("reverse=" + e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(!stdout.contains("lookup="), "stdout: {stdout}");
    assert!(!stdout.contains("reverse="), "stdout: {stdout}");
}

#[test]
fn tcp_connect_uses_explicit_port_and_multiple_attempts() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
    let port = listener.local_addr().unwrap().port();
    let count = 3;
    let handle = std::thread::spawn(move || {
        for _ in 0..count {
            let _ = listener.accept();
        }
    });

    let code = format!(
        r#"
import {{ tcp_connect }} from "std/net"

match tcp_connect("127.0.0.1", {port}, map {{ "allow_private": true, "count": {count}, "timeout_ms": 1000 }}) {{
    Ok(info) => {{
        print(info["connected"])
        print(info["method"])
        print(info["port"])
        print(info["connected_port"])
        print(info["remote_addr"])
        print(info["local_addr"])
        print(info["sent"])
        print(info["received"])
        print(info["failed"])
        print(info["loss_percent"])
        print(len(info["attempts"]))
    }},
    Err(e) => print("ERR: " + e)
}}
"#
    );

    let (stdout, stderr, exit_code) =
        run_ntnt_code_with_env(&code, &[("NTNT_NET_ALLOW_PRIVATE", "1")]);
    let _ = handle.join();

    assert_eq!(
        exit_code, 0,
        "stderr: {stderr}
stdout: {stdout}"
    );
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    assert!(lines.contains(&"true"), "stdout: {stdout}");
    assert!(lines.contains(&"tcp"), "stdout: {stdout}");
    let port_string = port.to_string();
    assert!(lines.contains(&port_string.as_str()), "stdout: {stdout}");
    assert!(
        stdout.contains(&format!("127.0.0.1:{port}")),
        "stdout: {stdout}"
    );
    assert!(
        lines.iter().filter(|line| **line == "3").count() >= 3,
        "stdout: {stdout}"
    );
    assert!(lines.contains(&"0"), "stdout: {stdout}");
}

#[test]
fn tcp_connect_closed_port_returns_connected_false() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let code = format!(
        r#"
import {{ tcp_connect }} from "std/net"

match tcp_connect("127.0.0.1", {port}, map {{ "allow_private": true, "timeout_ms": 100 }}) {{
    Ok(info) => {{
        print(info["connected"])
        print(info["reachable"])
        print(info["reason"])
    }},
    Err(e) => print("ERR: " + e)
}}
"#
    );

    let (stdout, stderr, exit_code) =
        run_ntnt_code_with_env(&code, &[("NTNT_NET_ALLOW_PRIVATE", "1")]);

    assert_eq!(exit_code, 0, "stderr: {stderr}\nstdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    assert!(lines.contains(&"false"), "stdout: {stdout}");
    assert!(!stdout.contains("ERR:"), "stdout: {stdout}");
}

#[test]
fn reachable_uses_icmp_and_default_plus_extra_tcp_ports() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let code = format!(
        r#"
import {{ reachable }} from "std/net"

match reachable("127.0.0.1", map {{ "allow_private": true, "tcp_ports": [{port}], "timeout_ms": 1000 }}) {{
    Ok(info) => {{
        let ports = info["tcp_ports_tried"] ?? info["ports_tried"]
        print(info["reachable"])
        print(info["method"])
        print(ports[0])
        print(ports[1])
        print(ports[2])
        print(info["connected_port"])
    }},
    Err(e) => print("ERR: " + e)
}}
"#
    );

    let (stdout, stderr, exit_code) =
        run_ntnt_code_with_env(&code, &[("NTNT_NET_ALLOW_PRIVATE", "1")]);

    assert_eq!(
        exit_code, 0,
        "stderr: {stderr}
stdout: {stdout}"
    );
    assert!(!stdout.contains("ERR:"), "stdout: {stdout}");
    assert!(stdout.contains("80"), "stdout: {stdout}");
    assert!(stdout.contains("443"), "stdout: {stdout}");
    assert!(stdout.contains(&port.to_string()), "stdout: {stdout}");
}

#[cfg(target_os = "linux")]
#[test]
fn ping_auto_uses_icmp_without_tcp_fallback() {
    if std::env::var("NTNT_TEST_ICMP_RAW").ok().as_deref() != Some("1") {
        eprintln!("skipping raw ICMP integration test; set NTNT_TEST_ICMP_RAW=1 to run");
        return;
    }

    let (stdout, stderr, code) = run_ntnt_code_with_env(
        r#"
import { ping } from "std/net"

match ping("127.0.0.1", map { "count": 1, "timeout_ms": 1000, "allow_private": true }) {
    Ok(info) => {
        print(info["reachable"])
        print(info["method"])
        print(info["received"])
    },
    Err(e) => print(e)
}
"#,
        &[("NTNT_NET_ALLOW_PRIVATE", "1")],
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("true"), "stdout: {stdout}");
    assert!(stdout.contains("icmp"), "stdout: {stdout}");
    assert!(stdout.contains("1"), "stdout: {stdout}");
}

#[test]
fn tcp_connect_private_target_requires_process_level_opt_in() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { tcp_connect } from "std/net"

match tcp_connect("127.0.0.1", 9, map { "allow_private": true, "timeout_ms": 100 }) {
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
fn tcp_connect_rejects_ipv4_mapped_and_special_ranges_by_default() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { tcp_connect } from "std/net"

match tcp_connect("::ffff:127.0.0.1", 9, map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected mapped"),
    Err(e) => print("mapped=" + e)
}

match tcp_connect("224.0.0.1", 9, map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected multicast"),
    Err(e) => print("multicast=" + e)
}

match tcp_connect("192.0.2.1", 9, map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected documentation"),
    Err(e) => print("documentation=" + e)
}

match tcp_connect("255.255.255.255", 9, map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected broadcast"),
    Err(e) => print("broadcast=" + e)
}

match tcp_connect("::ffff:255.255.255.255", 9, map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected mapped broadcast"),
    Err(e) => print("mapped_broadcast=" + e)
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
    assert!(
        stdout.contains("broadcast=Network target denied by policy"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("mapped_broadcast=Network target denied by policy"),
        "stdout: {stdout}"
    );
}

#[test]
fn tcp_connect_rejects_never_allowed_targets_even_with_private_opt_in() {
    let (stdout, stderr, code) = run_ntnt_code_with_env(
        r#"
import { tcp_connect } from "std/net"

match tcp_connect("169.254.169.254", 80, map { "allow_private": true, "timeout_ms": 100 }) {
    Ok(info) => print("unexpected metadata"),
    Err(e) => print("metadata=" + e)
}

match tcp_connect("169.254.170.2", 80, map { "allow_private": true, "timeout_ms": 100 }) {
    Ok(info) => print("unexpected ecs metadata"),
    Err(e) => print("ecs_metadata=" + e)
}

match tcp_connect("::ffff:169.254.169.254", 80, map { "allow_private": true, "timeout_ms": 100 }) {
    Ok(info) => print("unexpected mapped metadata"),
    Err(e) => print("mapped_metadata=" + e)
}

match tcp_connect("::ffff:169.254.170.2", 80, map { "allow_private": true, "timeout_ms": 100 }) {
    Ok(info) => print("unexpected mapped ecs metadata"),
    Err(e) => print("mapped_ecs_metadata=" + e)
}

match tcp_connect("224.0.0.1", 9, map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected multicast"),
    Err(e) => print("multicast=" + e)
}

match tcp_connect("255.255.255.255", 9, map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected broadcast"),
    Err(e) => print("broadcast=" + e)
}
"#,
        &[("NTNT_NET_ALLOW_PRIVATE", "1")],
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains(
            "metadata=Network target denied by policy: special-purpose targets are not allowed"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(
            "ecs_metadata=Network target denied by policy: special-purpose targets are not allowed"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("mapped_metadata=Network target denied by policy: special-purpose targets are not allowed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("mapped_ecs_metadata=Network target denied by policy: special-purpose targets are not allowed"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(
            "multicast=Network target denied by policy: special-purpose targets are not allowed"
        ),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains(
            "broadcast=Network target denied by policy: special-purpose targets are not allowed"
        ),
        "stdout: {stdout}"
    );
}

#[test]
fn port_scan_reports_open_and_closed_ports_sorted() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind local listener");
    let open_port = listener.local_addr().unwrap().port();
    let closed_listener = TcpListener::bind("127.0.0.1:0").expect("bind closed port");
    let closed_port = closed_listener.local_addr().unwrap().port();
    drop(closed_listener);

    listener
        .set_nonblocking(true)
        .expect("make listener nonblocking");
    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match listener.accept() {
                Ok(_) => return true,
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return false,
            }
        }
    });

    let code = format!(
        r#"
import {{ port_scan }} from "std/net"

match port_scan("127.0.0.1", [{closed_port}, {open_port}], map {{ "allow_private": true, "timeout_ms": 1000, "concurrency": 200 }}) {{
    Ok(results) => {{
        for result in results {{
            print(result["port"])
            print(result["open"])
            print(result["method"])
            print(result["reason"])
        }}
    }},
    Err(e) => print("ERR: " + e)
}}
"#
    );

    let (stdout, stderr, exit_code) =
        run_ntnt_code_with_env(&code, &[("NTNT_NET_ALLOW_PRIVATE", "1")]);
    let accepted_open_connection = handle.join().expect("accept helper should not panic");

    assert_eq!(exit_code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        accepted_open_connection,
        "accept helper timed out before port_scan connected; stdout: {stdout}"
    );
    assert!(!stdout.contains("ERR:"), "stdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().map(str::trim).collect();
    let open_port_line = open_port.to_string();
    let closed_port_line = closed_port.to_string();
    let port_lines: Vec<String> = lines
        .iter()
        .copied()
        .filter(|line| *line == open_port_line || *line == closed_port_line)
        .map(str::to_string)
        .collect();
    let mut expected_ports = [open_port, closed_port];
    expected_ports.sort();
    assert_eq!(
        port_lines,
        expected_ports
            .iter()
            .map(|port| port.to_string())
            .collect::<Vec<_>>(),
        "port_scan results must be sorted by port: {stdout}"
    );
    assert!(lines.contains(&"true"), "stdout: {stdout}");
    assert!(lines.contains(&"connected"), "stdout: {stdout}");
    assert!(lines.contains(&"false"), "stdout: {stdout}");
    assert!(
        lines.iter().filter(|line| **line == "tcp").count() >= 2,
        "stdout: {stdout}"
    );
}

#[test]
fn port_scan_rejects_duplicate_invalid_and_too_many_ports() {
    let too_many = (1..=129)
        .map(|port| port.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let code = format!(
        r#"
import {{ port_scan }} from "std/net"

match port_scan("example.com", [80, 80]) {{
    Ok(info) => print("unexpected duplicate"),
    Err(e) => print("duplicate=" + e)
}}

match port_scan("example.com", [0]) {{
    Ok(info) => print("unexpected invalid"),
    Err(e) => print("invalid=" + e)
}}

match port_scan("example.com", [{too_many}]) {{
    Ok(info) => print("unexpected too many"),
    Err(e) => print("too_many=" + e)
}}
"#
    );

    let (stdout, stderr, exit_code) = run_ntnt_code(&code);

    assert_eq!(exit_code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(stdout.contains("duplicate port 80"), "stdout: {stdout}");
    assert!(
        stdout.contains("port must be between 1 and 65535"),
        "stdout: {stdout}"
    );
    assert!(
        stdout.contains("supports at most 128 ports"),
        "stdout: {stdout}"
    );
}

#[test]
fn port_scan_unresolved_host_error_does_not_blame_first_port() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { port_scan } from "std/net"

match port_scan("missing.invalid", [21, 80], map { "timeout_ms": 100 }) {
    Ok(info) => print("unexpected"),
    Err(e) => print(e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains("failed to resolve missing.invalid:"),
        "stdout: {stdout}"
    );
    assert!(
        !stdout.contains("missing.invalid:21"),
        "error should not look like it tried to resolve host:port as a hostname: {stdout}"
    );
}

#[test]
fn port_scan_private_target_requires_process_level_opt_in() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { port_scan } from "std/net"

match port_scan("127.0.0.1", [9], map { "allow_private": true, "timeout_ms": 100 }) {
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
fn tls_info_returns_certificate_metadata_when_validation_fails() {
    let (port, handle) = start_local_tls_server(1);
    let code = format!(
        r#"
import {{ join }} from "std/string"
import {{ tls_info }} from "std/net"

match tls_info("127.0.0.1", map {{ "port": {port}, "server_name": "localhost", "allow_private": true, "timeout_ms": 2000 }}) {{
    Ok(info) => {{
        print(info["subject"])
        print(info["issuer"])
        print(info["subject_common_name"])
        print(info["not_before"])
        print(info["not_after"])
        print(join(info["san"], ","))
        print(info["valid"])
        print(info["validation_error"])
        print(info["protocol"])
        print(info["cipher"])
    }},
    Err(e) => print("ERR:" + e)
}}
"#
    );

    let (stdout, stderr, exit_code) =
        run_ntnt_code_with_env(&code, &[("NTNT_NET_ALLOW_PRIVATE", "1")]);
    let accepted = handle.join().expect("TLS helper should not panic");

    assert_eq!(exit_code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert_eq!(
        accepted, 1,
        "TLS helper did not accept the metadata connection; stdout: {stdout}"
    );
    assert!(!stdout.contains("ERR:"), "stdout: {stdout}");
    assert!(stdout.contains("localhost"), "stdout: {stdout}");
    assert!(stdout.contains("T"), "stdout: {stdout}");
    assert!(stdout.contains("Z"), "stdout: {stdout}");
    assert!(stdout.contains("false"), "stdout: {stdout}");
    assert!(
        stdout.contains("certificate validation failed"),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("TLS"), "stdout: {stdout}");
}

#[test]
fn tls_info_private_target_requires_process_level_opt_in() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { tls_info } from "std/net"

match tls_info("127.0.0.1", map { "allow_private": true, "timeout_ms": 100 }) {
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
fn net_capabilities_reports_probe_support_without_traffic() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { net_capabilities } from "std/net"
import { has_key } from "std/collections"

let caps = net_capabilities()
print(has_key(caps, "ping"))
print(has_key(caps, "traceroute"))
print(has_key(caps, "traceroute_udp"))
print(has_key(caps, "traceroute_tcp"))
print(has_key(caps, "icmpv4_datagram"))
print(has_key(caps, "icmpv4_raw"))
print(has_key(caps, "icmpv6_datagram"))
print(has_key(caps, "icmpv6_raw"))
print(caps.tcp)
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines,
        vec!["true", "true", "true", "true", "true", "true", "true", "true", "true"],
        "stdout: {stdout}"
    );
}

#[test]
fn traceroute_rejects_unknown_method() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { traceroute } from "std/net"
match traceroute("example.com", map { "method": "carrier-pigeon" }) {
    Ok(_) => print("unexpected-ok"),
    Err(e) => print(e)
}
"#,
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    assert!(
        stdout.contains("option 'method' must be"),
        "stdout: {stdout}"
    );
}

#[test]
fn traceroute_tcp_and_udp_are_honest_about_capability() {
    // CI-safe on any host: for each method the call's outcome must match the
    // matching per-method capability flag. Where the raw socket is
    // unavailable, the call returns a clear Err; where it works, hops result.
    for method in ["udp", "tcp"] {
        let code_src = format!(
            r#"
import {{ net_capabilities, traceroute }} from "std/net"
import {{ contains }} from "std/string"

let caps = net_capabilities()
let cap = if "{method}" == "tcp" {{ caps.traceroute_tcp }} else {{ caps.traceroute_udp }}
match traceroute("127.0.0.1", map {{ "method": "{method}", "max_hops": 2, "timeout_ms": 500, "allow_private": true }}) {{
    Ok(result) => {{
        if cap {{ print("ok-with-capability") }} else {{ print("unexpected-ok-without-capability") }}
    }},
    Err(e) => {{
        if cap {{
            print("unexpected-err-with-capability: #{{e}}")
        }} else {{
            if contains(e, "unavailable") {{ print("err-without-capability") }} else {{ print("unexpected-err-message: #{{e}}") }}
        }}
    }}
}}
"#
        );
        let (stdout, stderr, code) =
            run_ntnt_code_with_env(&code_src, &[("NTNT_NET_ALLOW_PRIVATE", "1")]);
        assert_eq!(
            code, 0,
            "method {method} stderr: {stderr}\nstdout: {stdout}"
        );
        let line = stdout.trim();
        assert!(
            line == "ok-with-capability" || line == "err-without-capability",
            "method {method} stdout: {stdout}"
        );
    }
}

#[test]
fn traceroute_is_honest_about_raw_socket_capability() {
    // CI-safe on any host: where raw ICMP is unavailable traceroute must
    // return a clear Err naming the capability, and where it is available a
    // loopback trace must produce hops. The branch taken always matches what
    // net_capabilities() reported.
    let (stdout, stderr, code) = run_ntnt_code_with_env(
        r#"
import { net_capabilities, traceroute } from "std/net"
import { contains } from "std/string"

let caps = net_capabilities()
match traceroute("127.0.0.1", map { "max_hops": 2, "timeout_ms": 500, "allow_private": true }) {
    Ok(result) => {
        if caps.traceroute {
            if result.hop_count > 0 {
                print("ok-with-capability")
            } else {
                print("ok-but-empty")
            }
        } else {
            print("unexpected-ok-without-capability")
        }
    },
    Err(e) => {
        if caps.traceroute {
            print("unexpected-err-with-capability: #{e}")
        } else {
            if contains(e, "traceroute unavailable") {
                print("err-without-capability")
            } else {
                print("unexpected-err-message: #{e}")
            }
        }
    }
}
"#,
        &[("NTNT_NET_ALLOW_PRIVATE", "1")],
    );

    assert_eq!(code, 0, "stderr: {stderr}\nstdout: {stdout}");
    let line = stdout.trim();
    assert!(
        line == "ok-with-capability" || line == "err-without-capability",
        "stdout: {stdout}"
    );
}
