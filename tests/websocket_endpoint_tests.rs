//! DD-041 Slice B: application WebSocket endpoints through `ntnt run`.
#[path = "support/system_fixture.rs"]
mod fixture;

use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

const APP: &str = r#"
import { text, status, ws_endpoint, websocket, ws_send, ws_close } from "std/http/server"
import { sleep_ms } from "std/concurrent"

fn opened(conn, state) {
    ws_send(conn, "welcome")
    return state
}

fn counter(conn, message, state) {
    if message["kind"] == "binary" {
        ws_send(conn, message["data"])
        return state
    }
    if message["data"] == "fail" {
        return Err("boom")
    }
    if message["data"] == "bye" {
        ws_send(conn, "closing")
        ws_close(conn, 4000, "done")
        return state
    }
    if message["data"] == "slow" {
        sleep_ms(1500)
    }
    let next = map { "count": state["count"] + 1 }
    ws_send(conn, message["data"] + ":" + str(next["count"]))
    return next
}

let counter_ws = ws_endpoint("counter", counter, map { "on_open": opened })

use_middleware(fn(req) {
    if req.path == "/private" {
        return status(401, "no")
    }
    return req
})

get("/ws", fn(req) { websocket(counter_ws, map { "state": map { "count": 0 } }) })
get("/private", fn(req) { websocket(counter_ws) })
get("/health", fn(req) { text("ok") })
get("/late", fn(req) {
    ws_endpoint("late", counter)
    return text("registered")
})
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

fn origin(addr: SocketAddr) -> String {
    format!("http://127.0.0.1:{}", addr.port())
}

#[allow(clippy::result_large_err)] // Test helper; the error is inspected, not propagated.
async fn connect(addr: SocketAddr, path: &str, origin: Option<&str>) -> Result<Socket, WsError> {
    let mut request = format!("ws://{addr}{path}").into_client_request().unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
    }
    tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .expect("handshake timed out")
    .map(|(socket, _)| socket)
}

/// Next data message, skipping pings.
async fn next_data(socket: &mut Socket) -> Message {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("no message within 10s")
            .expect("socket ended")
            .expect("socket error");
        if !matches!(message, Message::Ping(_) | Message::Pong(_)) {
            return message;
        }
    }
}

async fn close_frame(socket: &mut Socket) -> Option<(CloseCode, String)> {
    let mut frame = None;
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(Some(close)) = message {
                frame = Some((close.code, close.reason.to_string()));
            }
        }
    })
    .await;
    frame
}

fn start(env: &[(&str, &str)]) -> (fixture::Fixture, SocketAddr) {
    let mut server = fixture::Fixture::start(APP, &["run"], env);
    let addr = server.ready();
    (server, addr)
}

#[tokio::test]
async fn endpoint_keeps_state_in_order_across_two_workers() {
    let (_server, addr) = start(&[("NTNT_WORKERS", "2")]);
    let origin = origin(addr);
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    assert_eq!(
        next_data(&mut socket).await,
        Message::Text("welcome".into())
    );
    for i in 1..=12 {
        socket.send(Message::Text(format!("m{i}"))).await.unwrap();
        // Keep at most a few unanswered so the inbox limit is not the subject.
        if i % 3 == 0 {
            for j in i - 2..=i {
                assert_eq!(
                    next_data(&mut socket).await,
                    Message::Text(format!("m{j}:{j}"))
                );
            }
        }
    }
    socket.send(Message::Binary(vec![0, 7, 255])).await.unwrap();
    assert_eq!(
        next_data(&mut socket).await,
        Message::Binary(vec![0, 7, 255])
    );
    // A second connection has its own state.
    let mut other = connect(addr, "/ws", Some(&origin)).await.unwrap();
    assert_eq!(next_data(&mut other).await, Message::Text("welcome".into()));
    other.send(Message::Text("x".into())).await.unwrap();
    assert_eq!(next_data(&mut other).await, Message::Text("x:1".into()));
}

#[tokio::test]
async fn application_close_and_callback_error_close_the_socket() {
    let (_server, addr) = start(&[]);
    let origin = origin(addr);
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    assert_eq!(
        next_data(&mut socket).await,
        Message::Text("welcome".into())
    );
    socket.send(Message::Text("bye".into())).await.unwrap();
    // Queued messages are written before the application's close frame.
    assert_eq!(
        next_data(&mut socket).await,
        Message::Text("closing".into())
    );
    assert_eq!(
        close_frame(&mut socket).await,
        Some((CloseCode::from(4000), "done".to_string()))
    );

    let mut failing = connect(addr, "/ws", Some(&origin)).await.unwrap();
    assert_eq!(
        next_data(&mut failing).await,
        Message::Text("welcome".into())
    );
    failing.send(Message::Text("fail".into())).await.unwrap();
    assert_eq!(
        close_frame(&mut failing).await.map(|(code, _)| code),
        Some(CloseCode::Error)
    );
}

#[tokio::test]
async fn denials_foreign_origins_and_late_registration_open_nothing() {
    let (server, addr) = start(&[]);
    let origin = origin(addr);
    match connect(addr, "/private", Some(&origin)).await {
        Err(WsError::Http(response)) => assert_eq!(response.status().as_u16(), 401),
        other => panic!("expected 401, got {other:?}"),
    }
    for bad in [Some("http://evil.test"), Some("null"), None] {
        match connect(addr, "/ws", bad).await {
            Err(WsError::Http(response)) => {
                assert_eq!(response.status().as_u16(), 403, "{bad:?}")
            }
            other => panic!("expected 403 for {bad:?}, got {other:?}"),
        }
    }
    let late = reqwest::get(format!("http://{addr}/late")).await.unwrap();
    assert_eq!(late.status().as_u16(), 500);
    assert!(
        server
            .stderr()
            .contains("ws_endpoint() can only be called during application setup"),
        "{}",
        server.stderr()
    );
}

#[tokio::test]
async fn busy_callback_does_not_block_http_and_floods_are_refused() {
    let (_server, addr) = start(&[("NTNT_WORKERS", "2")]);
    let origin = origin(addr);
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    assert_eq!(
        next_data(&mut socket).await,
        Message::Text("welcome".into())
    );
    socket.send(Message::Text("slow".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    // One worker is always kept for HTTP while a callback runs.
    let started = std::time::Instant::now();
    let health = reqwest::get(format!("http://{addr}/health")).await.unwrap();
    assert_eq!(health.text().await.unwrap(), "ok");
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "HTTP waited {:?} behind a callback",
        started.elapsed()
    );
    // While that callback runs, at most 4 messages may wait.
    for i in 0..8 {
        if socket.send(Message::Text(format!("f{i}"))).await.is_err() {
            break;
        }
    }
    let mut code = None;
    let _ = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(Some(frame)) = message {
                code = Some(frame.code);
            }
        }
    })
    .await;
    assert_eq!(code, Some(CloseCode::Policy));
}

#[tokio::test]
async fn production_requires_trusted_origins() {
    let mut missing = fixture::Fixture::start(APP, &["run"], &[("NTNT_ENV", "production")]);
    assert!(!missing.wait().success());
    assert!(
        missing
            .stderr()
            .contains("set SITE_URL or pass the `origins` option"),
        "{}",
        missing.stderr()
    );

    let (_server, addr) = start(&[
        ("NTNT_ENV", "production"),
        ("NTNT_WORKERS", "2"),
        ("SITE_URL", "https://app.example.com/"),
    ]);
    let mut socket = connect(addr, "/ws", Some("https://app.example.com"))
        .await
        .unwrap();
    assert_eq!(
        next_data(&mut socket).await,
        Message::Text("welcome".into())
    );
    match connect(addr, "/ws", Some(&origin(addr))).await {
        Err(WsError::Http(response)) => assert_eq!(response.status().as_u16(), 403),
        other => panic!("loopback must not be trusted in production: {other:?}"),
    }
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_closes_open_sockets_with_going_away() {
    let (mut server, addr) = start(&[("NTNT_WORKERS", "2")]);
    let origin = origin(addr);
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    assert_eq!(
        next_data(&mut socket).await,
        Message::Text("welcome".into())
    );
    unsafe {
        libc::kill(server.child.id() as i32, libc::SIGTERM);
    }
    assert_eq!(
        close_frame(&mut socket).await.map(|(code, _)| code),
        Some(CloseCode::Away)
    );
    let status = tokio::task::spawn_blocking(move || {
        let status = server.wait();
        (status, server)
    })
    .await
    .unwrap()
    .0;
    assert!(status.success(), "server did not exit cleanly: {status:?}");
}

#[test]
fn ntnt_test_reports_websocket_as_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app.tnt");
    std::fs::write(
        &app,
        r#"
import { ws_endpoint, websocket } from "std/http/server"
let echo = ws_endpoint("echo", fn(conn, message, state) { state })
get("/ws", fn(req) { websocket(echo) })
listen(8080)
"#,
    )
    .unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["test"])
        .arg(&app)
        .args(["--get", "/ws"])
        .env_remove("NTNT_ENV")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("501"),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("websocket() requires the async HTTP server"),
        "{stdout}"
    );
}
