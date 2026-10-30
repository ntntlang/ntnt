//! Realtime startup uses the real async server, not the synchronous test server.
#[path = "support/system_fixture.rs"]
mod fixture;

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

fn app(setup: &str, realtime: &str) -> String {
    format!(
        r#"
import {{ text }} from "std/http/server"
import {{ broadcast, sleep_ms }} from "std/concurrent"
{setup}
print("SETUP_DONE")
get("/health", fn(req) {{ text("ready") }})
listen(0, map {{ "host": "127.0.0.1", "readiness": "json" {realtime} }})
"#
    )
}

fn setups_before_readiness(server: &fixture::Fixture) -> usize {
    let stdout = server.stdout();
    let before = stdout
        .split_once("NTNT_READY ")
        .expect("missing readiness")
        .0;
    before.matches("SETUP_DONE").count()
}

#[test]
fn realtime_readiness_waits_for_every_configured_worker() {
    for workers in ["1", "3"] {
        let source = app("sleep_ms(150)\nlet bus = broadcast(\"ready\")", "");
        let mut server = fixture::Fixture::start(
            &source,
            &["run"],
            &[("NTNT_ENV", "production"), ("NTNT_WORKERS", workers)],
        );
        let addr = server.ready();
        assert_eq!(
            setups_before_readiness(&server),
            workers.parse::<usize>().unwrap(),
            "{}",
            server.stdout()
        );
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            stream,
            "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"), "{response}");
        assert!(response.contains("ready"), "{response}");
    }
}

#[test]
fn explicit_realtime_options_wait_even_with_only_request_time_buses() {
    let source = app(
        "sleep_ms(150)\nget(\"/events\", fn(req) { text(str(broadcast(\"lazy\"))) })",
        ", \"realtime\": map {}",
    );
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "3")],
    );
    server.ready();
    assert_eq!(setups_before_readiness(&server), 3, "{}", server.stdout());
}

#[test]
fn realtime_listener_is_not_bound_while_a_worker_is_still_initializing() {
    let gate = tempfile::tempdir().unwrap();
    let gate_file = gate.path().join("continue");
    let gate_literal = serde_json::to_string(gate_file.to_str().unwrap()).unwrap();
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = reservation.local_addr().unwrap();
    drop(reservation);
    let source = app(
        &format!(
            r#"
import {{ exists }} from "std/fs"
fn warmup() {{
    let temporary = broadcast("transient") otherwise {{
        print("WORKER_WAITING")
        while !exists({gate_literal}) {{ sleep_ms(10) }}
        return 0
    }}
    return 0
}}
warmup()
let bus = broadcast("ready")
"#
        ),
        ", \"realtime\": map { \"max_topics\": 1 }",
    )
    .replace("listen(0,", &format!("listen({},", addr.port()));
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !server.stdout().contains("WORKER_WAITING") {
        assert!(
            server.child.try_wait().unwrap().is_none(),
            "{}",
            server.stderr()
        );
        assert!(
            std::time::Instant::now() < deadline,
            "worker did not reach setup gate"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!server.stdout().contains("NTNT_READY"));
    assert!(
        TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_err(),
        "listener admitted a connection before worker initialization"
    );
    std::fs::write(gate_file, b"continue").unwrap();
    assert_eq!(server.ready(), addr);
    assert_eq!(setups_before_readiness(&server), 2);
}

#[test]
fn buffered_only_readiness_retains_the_existing_startup_path() {
    let source = app("sleep_ms(150)", "");
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "3")],
    );
    server.ready();
    assert_eq!(setups_before_readiness(&server), 1, "{}", server.stdout());
}

#[test]
fn realtime_worker_setup_error_is_fatal_without_readiness_or_retry_loop() {
    let source = app(
        r#"
fn warmup() {
    let temporary = broadcast("transient")
    return 0
}
warmup()
let bus = broadcast("ready")
"#,
        ", \"realtime\": map { \"max_topics\": 1 }",
    );
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "3")],
    );
    assert!(!server.wait().success());
    assert!(!server.stdout().contains("NTNT_READY"));
    let error = server.stderr();
    assert!(error.contains("realtime worker"), "{error}");
    assert!(error.contains("too many broadcast names"), "{error}");
    assert!(
        !error.contains("retrying"),
        "initial realtime setup errors are fatal: {error}"
    );
}

#[test]
fn stalled_realtime_worker_setup_has_a_bounded_startup_decision() {
    let source = app(
        r#"
fn warmup() {
    let temporary = broadcast("transient") otherwise {
        sleep_ms(12000)
        return 0
    }
    return 0
}
warmup()
let bus = broadcast("ready")
"#,
        ", \"realtime\": map { \"max_topics\": 1 }",
    );
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
    );
    // Fixture::wait has a real child-exit deadline; Drop killing a child does not count.
    assert!(!server.wait().success());
    assert!(!server.stdout().contains("NTNT_READY"));
    assert!(
        server
            .stderr()
            .contains("realtime worker initialization timed out"),
        "{}",
        server.stderr()
    );
}

#[test]
fn bind_failure_after_realtime_worker_initialization_exits_without_readiness() {
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    let source =
        app("let bus = broadcast(\"ready\")", "").replace("listen(0,", &format!("listen({port},"));
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "3")],
    );
    assert!(!server.wait().success());
    assert!(!server.stdout().contains("NTNT_READY"));
    assert_eq!(
        server.stdout().matches("SETUP_DONE").count(),
        3,
        "{}",
        server.stdout()
    );
    assert!(
        server.stderr().contains("Failed to bind"),
        "{}",
        server.stderr()
    );
}
