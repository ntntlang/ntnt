//! Finite WebSocket refusals must keep explicit framing through the full async stack.
#[path = "support/system_fixture.rs"]
mod fixture;

use axum::http::{HeaderMap, HeaderName, HeaderValue};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

const ORIGIN: &str = "https://app.example.com";
const HANDSHAKE: &str =
    "Sec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n";
const APP: &str = r#"
import { ws_endpoint, websocket, ws_send } from "std/http/server"
fn echo(conn, message, state) {
    ws_send(conn, message["data"])
    return state
}
let endpoint = ws_endpoint("echo", echo, map { "origins": ["https://app.example.com"] })
enable_csp(map { "default-src": "'none'" })
get("/ws", fn(req) { websocket(endpoint) })
listen(0, map {
    "fixture": true, "readiness": "json",
    "realtime": map { "max_connections": 1 }
})
"#;

fn start() -> (fixture::Fixture, SocketAddr) {
    let mut server = fixture::Fixture::start(
        APP,
        &["run"],
        &[
            ("NTNT_ENV", "production"),
            ("NTNT_WORKERS", "2"),
            ("NTNT_SECURITY_HEADERS", "1"),
        ],
    );
    let addr = server.ready();
    (server, addr)
}

/// Read the actual HTTP/1 bytes without decoding chunked framing or forcing
/// connection-close framing. Bound both the response size and the whole read.
async fn refusal(addr: SocketAddr, headers: &str, encoding: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                format!(
                    "GET /ws HTTP/1.1\r\nHost: {addr}\r\nConnection: upgrade\r\nUpgrade: websocket\r\nAccept-Encoding: {encoding}\r\n{headers}\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut bytes = Vec::new();
        let mut chunk = [0; 1024];
        loop {
            let n = stream.read(&mut chunk).await.unwrap();
            assert!(n > 0, "refusal closed early: {bytes:?}");
            bytes.extend_from_slice(&chunk[..n]);
            assert!(bytes.len() < 16 * 1024, "refusal exceeded response cap");
            let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
                continue;
            };
            let head = std::str::from_utf8(&bytes[..end]).unwrap();
            let length = head.lines().skip(1).find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap())
            });
            let body = &bytes[end + 4..];
            // Retain the raw chunk framing on the baseline so the assertion
            // below diagnoses the missing length, not a keep-alive timeout.
            if length.is_some_and(|n| body.len() >= n) || body.ends_with(b"\r\n0\r\n\r\n") {
                break;
            }
        }
        String::from_utf8(bytes).unwrap()
    })
    .await
    .expect("refusal did not complete within 5s")
}

fn assert_refusal(wire: &str, status: u16, expected_body: &str) {
    let (head, body) = wire.split_once("\r\n\r\n").expect("HTTP response");
    let mut lines = head.lines();
    let status_line = lines.next().unwrap();
    assert!(
        status_line.starts_with(&format!("HTTP/1.1 {status} ")),
        "{wire}"
    );
    let mut headers = HeaderMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').unwrap();
        headers.append(
            HeaderName::from_bytes(name.as_bytes()).unwrap(),
            HeaderValue::from_str(value.trim()).unwrap(),
        );
    }
    assert_eq!(
        headers.get("content-length").and_then(|v| v.to_str().ok()),
        Some(expected_body.len().to_string().as_str()),
        "finite refusal must advertise its byte length: {wire}"
    );
    assert_eq!(headers.get_all("content-length").iter().count(), 1);
    assert_eq!(body, expected_body, "body must be complete and unchunked");
    for name in [
        "transfer-encoding",
        "content-encoding",
        "upgrade",
        "sec-websocket-accept",
        "sec-websocket-protocol",
        "sec-websocket-extensions",
    ] {
        assert!(!headers.contains_key(name), "unexpected {name}: {wire}");
    }
    assert_eq!(headers["content-type"], "text/plain; charset=utf-8");
    assert_eq!(headers["cache-control"], "no-store");
    assert_eq!(headers["server"], "ntnt-async");
    assert_eq!(headers["x-content-type-options"], "nosniff");
    assert_eq!(headers["x-frame-options"], "DENY");
    assert_eq!(
        headers["referrer-policy"],
        "strict-origin-when-cross-origin"
    );
    assert!(headers["content-security-policy"]
        .to_str()
        .unwrap()
        .contains("default-src 'none'"));
    if status == 503 {
        assert_eq!(headers["retry-after"], "5");
    } else {
        assert!(!headers.contains_key("retry-after"));
    }
}

#[tokio::test]
async fn denied_origins_have_complete_fixed_length_http1_bodies() {
    let (_server, addr) = start();
    for origin in [
        Some("https://evil.example"),
        Some("http://app.example.com"),
        Some("null"),
        None,
    ] {
        let origin_header = origin
            .map(|origin| format!("Origin: {origin}\r\n"))
            .unwrap_or_default();
        // The compression layer also wraps identity/small responses. Neither
        // identity nor accepting gzip may erase this small body's framing.
        for encoding in ["identity", "gzip"] {
            let wire = refusal(addr, &format!("{HANDSHAKE}{origin_header}"), encoding).await;
            assert_refusal(&wire, 403, "Origin not allowed");
        }
    }
}

#[tokio::test]
async fn malformed_upgrade_has_a_complete_fixed_length_http1_body() {
    let (_server, addr) = start();
    // Missing both the key and version, then missing just the key.
    for handshake in ["", "Sec-WebSocket-Version: 13\r\n"] {
        let wire = refusal(
            addr,
            &format!("{handshake}Origin: {ORIGIN}\r\n"),
            "identity",
        )
        .await;
        assert_refusal(&wire, 400, "WebSocket upgrade required");
    }
}

#[tokio::test]
async fn admission_refusal_has_fixed_length_without_changing_successful_upgrades() {
    let (_server, addr) = start();
    let mut request = format!("ws://{addr}/ws").into_client_request().unwrap();
    request
        .headers_mut()
        .insert("origin", ORIGIN.parse().unwrap());
    let (mut socket, handshake) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .expect("upgrade timed out")
    .unwrap();
    assert_eq!(handshake.status().as_u16(), 101);
    assert_eq!(handshake.headers()["upgrade"], "websocket");
    assert!(handshake.headers().contains_key("sec-websocket-accept"));
    for name in ["content-length", "transfer-encoding", "content-encoding"] {
        assert!(!handshake.headers().contains_key(name), "{name} on 101");
    }
    // A round trip proves the admitted connection owns the only permit.
    tokio::time::timeout(Duration::from_secs(5), async {
        socket
            .send(Message::Text("still open".into()))
            .await
            .unwrap();
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Text("still open".into())
        );
    })
    .await
    .expect("echo timed out");
    let wire = refusal(
        addr,
        &format!("{HANDSHAKE}Origin: {ORIGIN}\r\n"),
        "identity",
    )
    .await;
    assert_refusal(&wire, 503, "Too many open realtime connections");
    tokio::time::timeout(Duration::from_secs(5), socket.close(None))
        .await
        .expect("close timed out")
        .unwrap();
}
