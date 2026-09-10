#[path = "support/system_fixture.rs"]
mod fixture;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;
fn request(addr: SocketAddr, path: &str, method: &str) -> Vec<u8> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = Vec::new();
    stream.take(1024 * 1024).read_to_end(&mut response).unwrap();
    response
}
fn headers(response: &[u8]) -> String {
    String::from_utf8_lossy(response.split(|b| *b == 0).next().unwrap())
        .split("\r\n\r\n")
        .next()
        .unwrap()
        .to_ascii_lowercase()
}
const ROUTES: &str = r#"
import { text, json, redirect, status } from "std/http/server"
enable_csp()
get("/", fn(req) { text("hello") })
get("/json", fn(req) { json(map { "version": 1, "data": map { "key": "value" } }) })
get("/redirect", fn(req) { redirect("/json") })
get("/empty", fn(req) { status(204, "") })
get("/headers", fn(req) { map { "status": 200, "headers": map { "sErVeR": "fixture", "CACHE-control": "max-age=20", "X-Keep": "yes" }, "body": "body" } })
"#;
#[test]
fn async_readiness_headers_port_zero_and_production_workers() {
    for env in [
        vec![],
        vec![("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
    ] {
        let source=format!("{ROUTES}\nlisten(0, map {{ \"fixture\": true, \"readiness\": \"json\", \"suppress_server_header\": true, \"suppress_cache_control\": true }})");
        let mut server = fixture::Fixture::start(&source, &["run"], &env);
        let addr = server.ready();
        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0);
        for path in ["/", "/json", "/redirect", "/missing", "/headers", "/empty"] {
            let response = request(addr, path, "GET");
            let h = headers(&response);
            assert!(!h.contains("\r\nserver:"), "{path}: {h}");
            assert!(!h.contains("\r\ncache-control:"), "{path}: {h}");
            assert!(h.contains("x-content-type-options:"), "{path}: {h}");
            assert!(h.contains("content-security-policy:"), "{path}: {h}");
            if path == "/headers" {
                assert!(h.contains("x-keep: yes"));
            }
            if path == "/redirect" {
                assert!(h.contains("location: /json"));
            }
        }
        let response = request(addr, "/", "HEAD");
        assert!(response.ends_with(b"\r\n\r\n"));
        assert_eq!(server.stdout().matches("NTNT_READY ").count(), 1);
    }
}
#[test]
fn async_default_headers_and_failed_startup_never_ready() {
    let source =
        format!("{ROUTES}\nlisten(0, map {{ \"fixture\": true, \"readiness\": \"json\" }})");
    let mut server = fixture::Fixture::start(&source, &["run"], &[]);
    let addr = server.ready();
    let h = headers(&request(addr, "/headers", "GET"));
    assert!(h.contains("\r\nserver:"));
    assert!(h.contains("\r\ncache-control:"));
    let source = format!(
        "{ROUTES}\nlisten({}, map {{ \"fixture\": true, \"readiness\": \"json\" }})",
        addr.port()
    );
    let mut failed = fixture::Fixture::start(&source, &["run"], &[]);
    assert!(
        !failed.wait().success(),
        "{} {}",
        failed.stdout(),
        failed.stderr()
    );
    assert!(!failed.stdout().contains("NTNT_READY"));
    assert!(failed.stderr().contains("Failed to bind"));
    for call in [
        "listen(-1)",
        "listen(65536)",
        "listen(0, map { \"readiness\": \"bad\" })",
        "listen(0, map { \"fixture\": true, \"host\": \"0.0.0.0\" })",
        "listen(0, map { \"suppress_server_header\": true })",
        "listen(0, map { \"unknown\": true })",
    ] {
        let mut failed = fixture::Fixture::start(&format!("{ROUTES}\n{call}"), &["run"], &[]);
        assert!(!failed.wait().success(), "{call}");
        assert!(!failed.stdout().contains("NTNT_READY"));
    }
}
#[test]
fn sync_cli_old_listen_and_options() {
    for listen in ["listen(0)","listen(0, map { \"fixture\": true, \"readiness\": \"json\", \"suppress_server_header\": true, \"suppress_cache_control\": true })"] {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port().to_string();
        drop(reservation);
        let mut child=fixture::Fixture::start(&format!("{ROUTES}\n{listen}"),&["test","--port",&port,"--get","/json"],&[]);
        assert!(child.wait().success(),"{} {}",child.stdout(),child.stderr());assert!(child.stdout().contains("version"),"{}",child.stdout());
        assert_eq!(child.stdout().matches("NTNT_READY ").count(),usize::from(listen.contains("readiness")));
    }
}
#[test]
fn sync_wire_filter_preserves_binary_head_and_dependency_framing() {
    use ntnt::stdlib::http_server::{send_static_response_with_options, FixtureHeaders};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("asset.bin");
    std::fs::write(&path, [0, 255, 13, 10, 1]).unwrap();
    for method in ["GET", "HEAD"] {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let addr = server.server_addr().to_ip().unwrap();
        let path = path.clone();
        let worker = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .expect("client request");
            send_static_response_with_options(
                request,
                path.to_str().unwrap(),
                FixtureHeaders {
                    suppress_server: true,
                    suppress_cache_control: true,
                },
            )
            .unwrap();
        });
        let response = request(addr, "/asset.bin", method);
        worker.join().unwrap();
        let h = headers(&response);
        assert!(!h.contains("\r\nserver:"));
        assert!(!h.contains("\r\ncache-control:"));
        assert!(h.contains("etag:"));
        if method == "GET" {
            assert!(response.ends_with(&[0, 255, 13, 10, 1]));
        } else {
            assert!(response.ends_with(b"\r\n\r\n"));
        }
    }
}

#[test]
fn native_http_json_cas_and_no_send_email_capture() {
    let source = include_str!("../examples/system-primitives/http-fixture.tnt");
    let mut lint = fixture::Fixture::start(source, &["lint", "--strict"], &[]);
    assert!(lint.wait().success(), "{} {}", lint.stdout(), lint.stderr());
    let mut server = fixture::Fixture::start(source, &["run"], &[("NTNT_WORKERS", "1")]);
    let addr = server.ready();
    fn post(addr: SocketAddr, path: &str, body: &str) -> Vec<u8> {
        let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
        s.set_nonblocking(false).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        s.set_write_timeout(Some(Duration::from_secs(3))).unwrap();
        write!(s,"POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
        let mut response = Vec::new();
        s.take(65536).read_to_end(&mut response).unwrap();
        response
    }
    for body in [
        "[]",
        "null",
        "true",
        "1",
        "\"not an object\"",
        "{}",
        r#"{"cas":"0","data":{}}"#,
        r#"{"cas":0}"#,
        r#"{"cas":0,"data":[]}"#,
        r#"{"cas":0,"data":null}"#,
    ] {
        let invalid = post(addr, "/state", body);
        assert!(
            headers(&invalid).contains("400"),
            "invalid body {body}: {}",
            String::from_utf8_lossy(&invalid)
        );
    }
    let untouched = request(addr, "/state", "GET");
    assert!(String::from_utf8_lossy(&untouched).contains("\"version\":0"));
    let good = post(addr, "/state", r#"{"cas":0,"data":{"value":"local"}}"#);
    assert!(
        headers(&good).contains("200 ok"),
        "{}",
        String::from_utf8_lossy(&good)
    );
    let stale = post(addr, "/state", r#"{"cas":0,"data":{}}"#);
    assert!(headers(&stale).contains("409"));
    let state = request(addr, "/state", "GET");
    let state = String::from_utf8(state).unwrap();
    assert!(state.contains("\"version\":1"));
    assert!(state.contains("\"value\":\"local\""));
    let email = post(
        addr,
        "/emails",
        r#"{"to":"test@example.invalid","text":"captured only"}"#,
    );
    assert!(String::from_utf8_lossy(&email).contains("local-capture"));
    assert!(server.stdout().contains("CAPTURE "));
}

#[test]
fn readiness_once_across_hot_reload_and_port_override() {
    let source =
        format!("{ROUTES}\nlisten(1, map {{ \"fixture\": true, \"readiness\": \"json\" }})");
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_LISTEN_PORT", "0"), ("NTNT_WORKERS", "1")],
    );
    let addr = server.ready();
    assert_ne!(addr.port(), 1);
    assert!(String::from_utf8_lossy(&request(addr, "/", "GET")).contains("hello"));
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(
        server.dir.path().join("fixture.tnt"),
        source.replace("hello", "reloaded"),
    )
    .unwrap();
    let end = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        if String::from_utf8_lossy(&request(addr, "/", "GET")).contains("reloaded") {
            break;
        }
        assert!(
            std::time::Instant::now() < end,
            "hot reload deadline: {} {}",
            server.stdout(),
            server.stderr()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(server.stdout().matches("NTNT_READY ").count(), 1);
    let mut failed = fixture::Fixture::start(
        "listen(0, map { \"fixture\": true, \"readiness\": \"json\" })",
        &["run"],
        &[],
    );
    assert!(!failed.wait().success());
    assert!(!failed.stdout().contains("NTNT_READY"));
}
#[test]
fn sync_test_port_precedes_environment_override() {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let source =
        format!("{ROUTES}\nlisten(0, map {{ \"readiness\": \"json\", \"host\": \"127.0.0.1\" }})");
    let mut child = fixture::Fixture::start(
        &source,
        &["test", "--port", &port.to_string(), "--get", "/json"],
        &[("NTNT_LISTEN_PORT", "0")],
    );
    assert!(
        child.wait().success(),
        "{} {}",
        child.stdout(),
        child.stderr()
    );
    let out = child.stdout();
    let ready = out
        .lines()
        .find_map(|s| s.strip_prefix("NTNT_READY "))
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(ready).unwrap();
    assert_eq!(value["port"], port);
    assert_eq!(value["host"], "127.0.0.1");
}
