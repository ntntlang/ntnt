//! DD-041 Slice A: Server-Sent Events through the real async server.
#[path = "support/system_fixture.rs"]
mod fixture;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

const APP: &str = r#"
import { text, status, sse } from "std/http/server"
import { broadcast, publish } from "std/concurrent"
let bus = broadcast("demo")
use_middleware(fn(req) {
    if req.path == "/private" {
        return status(401, "no")
    }
    return req
})
get("/events", fn(req) {
    let mut r = sse(bus, map { "retry_ms": 1000 })
    r["headers"]["set-cookie"] = "seen=1; Path=/"
    r["headers"]["content-length"] = "5"
    r["headers"]["content-encoding"] = "gzip"
    return r
})
get("/private", fn(req) { sse(bus) })
get("/pub", fn(req) {
    let n = int(req.query_params["n"] ?? "0") ?? 0
    let res = publish(bus, map { "n": n, "msg": "a\nb" }, map { "event": "tick", "id": n })
    return text(str(is_ok(res)))
})
get("/bad", fn(req) {
    let res = publish(bus, fn() { 1 })
    return text(str(is_err(res)))
})
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

fn open(addr: SocketAddr, path: &str, method: &str) -> TcpStream {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    s.set_write_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(s, "{method} {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
    s
}

fn read_until(s: &mut TcpStream, buf: &mut Vec<u8>, needle: &[u8], count: usize) {
    let end = Instant::now() + Duration::from_secs(5);
    let mut chunk = [0u8; 8192];
    while buf.windows(needle.len()).filter(|w| *w == needle).count() < count {
        assert!(
            Instant::now() < end,
            "timed out: {}",
            String::from_utf8_lossy(buf)
        );
        let n = s.read(&mut chunk).unwrap();
        assert!(n > 0, "closed early: {}", String::from_utf8_lossy(buf));
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn get_body(addr: SocketAddr, path: &str) -> String {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    write!(
        s,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut raw = String::new();
    s.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    if !head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return raw;
    }
    // Decode chunked framing so callers can assert on the body text.
    let mut decoded = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let len = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if len == 0 {
            break;
        }
        decoded.push_str(&after[..len]);
        rest = &after[len + 2..];
    }
    format!("{head}\r\n\r\n{decoded}")
}

#[test]
fn sse_delivers_cross_worker_events_with_fixed_headers_and_denials_open_nothing() {
    for workers in ["1", "4"] {
        let mut server = fixture::Fixture::start(
            APP,
            &["run"],
            &[("NTNT_ENV", "production"), ("NTNT_WORKERS", workers)],
        );
        let addr = server.ready();
        // Give secondary workers time to load before publishing from them.
        std::thread::sleep(Duration::from_millis(500));
        let mut clients: Vec<(TcpStream, Vec<u8>)> = (0..3)
            .map(|_| {
                let mut s = open(addr, "/events", "GET");
                let mut buf = Vec::new();
                read_until(&mut s, &mut buf, b"retry: 1000\n\n", 1);
                (s, buf)
            })
            .collect();
        let head = String::from_utf8_lossy(&clients[0].1).to_ascii_lowercase();
        assert!(head.starts_with("http/1.1 200"), "{head}");
        assert!(head.contains("content-type: text/event-stream"), "{head}");
        assert!(
            head.contains("cache-control: no-store, no-transform"),
            "{head}"
        );
        assert!(head.contains("x-accel-buffering: no"), "{head}");
        assert!(head.contains("set-cookie: seen=1"), "{head}");
        assert!(
            !head.contains("content-length"),
            "app cannot fix length: {head}"
        );

        for n in 1..=4 {
            let body = get_body(addr, &format!("/pub?n={n}"));
            assert!(body.ends_with("true"), "{body} {}", server.stderr());
        }
        for (s, buf) in &mut clients {
            read_until(s, buf, b"event: tick\n", 4);
            let text = String::from_utf8_lossy(buf);
            assert!(
                text.contains("event: tick\nid: 1\ndata: {\"msg\":\"a\\nb\",\"n\":1}\n\n"),
                "workers={workers}: {text}"
            );
        }
        assert!(
            get_body(addr, "/bad").ends_with("true"),
            "functions are rejected"
        );

        let denied = get_body(addr, "/private");
        assert!(denied.starts_with("HTTP/1.1 401"), "{denied}");
        assert!(!denied.contains("text/event-stream"), "{denied}");

        let mut head_req = open(addr, "/events", "HEAD");
        let mut head_resp = Vec::new();
        read_until(&mut head_req, &mut head_resp, b"\r\n\r\n", 1);
        assert!(head_resp.starts_with(b"HTTP/1.1 200"));
        assert!(
            head_resp.ends_with(b"\r\n\r\n"),
            "HEAD must not stream a body"
        );
    }
}

#[cfg(unix)]
#[test]
fn sigterm_ends_open_and_stalled_streams_promptly() {
    let mut server = fixture::Fixture::start(
        APP,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
    );
    let addr = server.ready();
    let mut reading = open(addr, "/events", "GET");
    let mut buf = Vec::new();
    read_until(&mut reading, &mut buf, b"retry: 1000\n\n", 1);
    // A stalled client: never reads after the request.
    let _stalled = open(addr, "/events", "GET");
    std::thread::sleep(Duration::from_millis(200));

    let signalled = Instant::now();
    unsafe {
        libc::kill(server.child.id() as libc::pid_t, libc::SIGTERM);
    }
    let status = server.wait();
    assert!(status.success(), "{status:?} {}", server.stderr());
    assert!(
        signalled.elapsed() < Duration::from_secs(5),
        "streams must end on shutdown, not wait for the drain limit: {:?}",
        signalled.elapsed()
    );
    let mut rest = Vec::new();
    let _ = reading.read_to_end(&mut rest);
}

#[test]
fn sync_test_server_rejects_sse_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.tnt");
    std::fs::write(&file, APP).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["test"])
        .arg(&file)
        .args(["--get", "/events"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("501"), "{stdout}");
    assert!(
        stdout.contains("requires the async HTTP server"),
        "{stdout}"
    );
}

#[test]
fn broadcast_handles_are_rejected_by_channels_even_when_nested() {
    let source = r#"
import { broadcast, channel, send } from "std/concurrent"
let bus = broadcast("nested")
let tx = channel()[0]
fn attempt(label, payload) {
    let r = send(tx, payload) otherwise {
        print("rejected #{label}")
        return false
    }
    print("accepted #{label}")
    return true
}
attempt("bare", bus)
attempt("array", [bus])
attempt("map", map { "b": bus })
attempt("plain", [1])
"#;
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("app.tnt");
    std::fs::write(&file, source).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .arg("run")
        .arg(&file)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    for label in ["bare", "array", "map"] {
        assert!(stdout.contains(&format!("rejected {label}")), "{stdout}");
    }
    assert!(stdout.contains("accepted plain"), "control case: {stdout}");
}
