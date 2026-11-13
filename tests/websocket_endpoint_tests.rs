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

/// GET `path` with a deadline on the whole request and body.
async fn get_body(addr: SocketAddr, path: &'static str) -> String {
    tokio::time::timeout(Duration::from_secs(10), async move {
        reqwest::get(format!("http://{addr}{path}"))
            .await?
            .text()
            .await
    })
    .await
    .expect("no response within 10s")
    .expect("request failed")
}

const LIFETIME_APP: &str = r#"
import { text, ws_endpoint, websocket, ws_send } from "std/http/server"
fn echo(conn, message, state) {
    ws_send(conn, message["data"])
    return state
}
fn closed(state, info) {
    print("CLOSED " + str(info["code"]) + " " + info["reason"])
}
let short = ws_endpoint("short", echo, map { "max_lifetime_ms": 1200, "on_close": closed })
get("/ws", fn(req) { websocket(short) })
get("/health", fn(req) { text("ok") })
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

#[tokio::test]
async fn connection_closes_at_its_lifetime_limit() {
    let mut server = fixture::Fixture::start(LIFETIME_APP, &["run"], &[("NTNT_WORKERS", "2")]);
    let addr = server.ready();
    let origin = origin(addr);
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    socket.send(Message::Text("hi".into())).await.unwrap();
    assert_eq!(next_data(&mut socket).await, Message::Text("hi".into()));
    let started = std::time::Instant::now();
    assert_eq!(
        close_frame(&mut socket).await,
        Some((CloseCode::Away, "connection lifetime reached".to_string()))
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_millis(3000),
        "closed after {elapsed:?}"
    );
    // on_close sees why, and a reconnect goes through the HTTP pipeline again.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !server
        .stdout()
        .contains("CLOSED 1001 connection lifetime reached")
    {
        assert!(std::time::Instant::now() < deadline, "{}", server.stdout());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut again = connect(addr, "/ws", Some(&origin)).await.unwrap();
    again.send(Message::Text("back".into())).await.unwrap();
    assert_eq!(next_data(&mut again).await, Message::Text("back".into()));
}

#[test]
fn lifetime_must_be_finite_and_in_range() {
    let mut app = fixture::Fixture::start(
        r#"
import { text, ws_endpoint } from "std/http/server"
let forever = ws_endpoint("forever", fn(conn, message, state) { state }, map { "max_lifetime_ms": 0 })
get("/", fn(req) { text("ok") })
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#,
        &["run"],
        &[],
    );
    // wait() has a deadline: if validation accepted 0, the server would keep
    // serving, and the test fails (and the fixture kills it) instead of hanging.
    let status = app.wait();
    let stderr = app.stderr();
    assert!(!status.success(), "{stderr}");
    assert!(
        stderr.contains("connections are never unlimited"),
        "{stderr}"
    );
}

const RELOAD_APP_V1: &str = r#"
import { text, ws_endpoint, websocket, ws_send } from "std/http/server"
fn echo(conn, message, state) {
    ws_send(conn, "v1:" + message["data"])
    return state
}
let live = ws_endpoint("live", echo)
get("/ws", fn(req) { websocket(live) })
get("/health", fn(req) { text("ok") })
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

#[tokio::test]
async fn development_reload_closes_open_sockets_and_new_ones_use_new_code() {
    // Development mode, one worker: hot reload applies on the next request.
    let mut server = fixture::Fixture::start(RELOAD_APP_V1, &["run"], &[]);
    let addr = server.ready();
    let origin = origin(addr);
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    socket.send(Message::Text("a".into())).await.unwrap();
    assert_eq!(next_data(&mut socket).await, Message::Text("v1:a".into()));

    // Make the change visible to mtime checks on coarse-grained filesystems.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    std::fs::write(
        server.dir.path().join("fixture.tnt"),
        RELOAD_APP_V1.replace("\"v1:\"", "\"v2:\""),
    )
    .unwrap();
    assert_eq!(get_body(addr, "/health").await, "ok");

    assert_eq!(
        close_frame(&mut socket).await,
        Some((CloseCode::Restart, "server code reloaded".to_string())),
        "{}",
        server.stderr()
    );
    let mut fresh = connect(addr, "/ws", Some(&origin)).await.unwrap();
    fresh.send(Message::Text("b".into())).await.unwrap();
    assert_eq!(next_data(&mut fresh).await, Message::Text("v2:b".into()));
}

const STATE_RELOAD_APP: &str = r#"
import { text, ws_endpoint, websocket, ws_send } from "std/http/server"
import { sleep_ms } from "std/concurrent"
fn on_message(conn, message, state) {
    if message["data"] == "slow" {
        sleep_ms(1500)
    }
    print("MESSAGE v1 " + state["schema"] + " " + message["data"])
    ws_send(conn, "v1:" + message["data"])
    return state
}
fn on_close(state, info) {
    print("CLOSE v1 " + state["schema"] + " " + str(info["code"]))
}
let live = ws_endpoint("live", on_message, map { "on_close": on_close })
get("/ws", fn(req) { websocket(live, map { "state": map { "schema": "old" } }) })
get("/health", fn(req) { text("ok") })
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

#[tokio::test]
async fn development_reload_never_runs_new_callbacks_on_old_state() {
    // Development mode, one worker: the reload applies on the next request.
    let mut server = fixture::Fixture::start(STATE_RELOAD_APP, &["run"], &[]);
    let addr = server.ready();
    let origin = origin(addr);
    let mut old = connect(addr, "/ws", Some(&origin)).await.unwrap();
    old.send(Message::Text("a".into())).await.unwrap();
    assert_eq!(next_data(&mut old).await, Message::Text("v1:a".into()));

    // Make the change visible to mtime checks on coarse-grained filesystems.
    tokio::time::sleep(Duration::from_millis(1100)).await;
    // Hold the only worker in a slow callback and queue a message behind it.
    // Then change the callbacks and the state shape, and send the request
    // that reloads them: the worker takes that request first, so the queued
    // message and the old socket's on_close would meet the new code.
    old.send(Message::Text("slow".into())).await.unwrap();
    old.send(Message::Text("queued".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    std::fs::write(
        server.dir.path().join("fixture.tnt"),
        STATE_RELOAD_APP
            .replace("v1", "v2")
            .replace("\"old\"", "\"new\""),
    )
    .unwrap();
    let health = tokio::spawn(get_body(addr, "/health"));
    assert_eq!(
        close_frame(&mut old).await,
        Some((CloseCode::Restart, "server code reloaded".to_string())),
        "{}",
        server.stderr()
    );
    assert_eq!(health.await.unwrap(), "ok");

    // A new connection gets the new code and state, on_close included.
    let mut fresh = connect(addr, "/ws", Some(&origin)).await.unwrap();
    fresh.send(Message::Text("b".into())).await.unwrap();
    assert_eq!(next_data(&mut fresh).await, Message::Text("v2:b".into()));
    fresh.close(None).await.unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !server.stdout().contains("CLOSE v2 new 1000") {
        assert!(
            std::time::Instant::now() < deadline,
            "{}\n{}",
            server.stdout(),
            server.stderr()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The old connection's queued message and on_close were never run on the
    // new code; the old on_close cannot run either, its code is gone.
    let stdout = server.stdout();
    assert!(!stdout.contains("v2 old"), "{stdout}");
    assert!(!stdout.contains("CLOSE v1"), "{stdout}");
    assert!(
        server
            .stderr()
            .contains("on_close skipped: server code reloaded"),
        "{}",
        server.stderr()
    );
}

const ROUTE_RELOAD_APP: &str = r#"
import { text, ws_endpoint, websocket, ws_send } from "std/http/server"
fn echo(conn, message, state) {
    ws_send(conn, message["data"])
    return state
}
let live = ws_endpoint("live", echo)
get("/ws", fn(req) { websocket(live) })
routes("ROUTES_DIR")
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

#[tokio::test]
async fn editing_a_route_file_in_place_closes_open_sockets() {
    let routes = tempfile::tempdir().unwrap();
    let page = routes.path().join("page.tnt");
    // The explicit import keeps this test independent of how a reloaded route
    // gets the prelude (#357, fixed in #358).
    let route = |version: &str| {
        format!(
            "import {{ text }} from \"std/http/server\"\nexport fn get(req) {{\n    return text(\"{version}\")\n}}\n"
        )
    };
    std::fs::write(&page, route("route-v1")).unwrap();
    // Forward slashes keep a Windows path valid inside the string literal.
    let dir = routes.path().to_string_lossy().replace('\\', "/");
    let mut server =
        fixture::Fixture::start(&ROUTE_RELOAD_APP.replace("ROUTES_DIR", &dir), &["run"], &[]);
    let addr = server.ready();
    let origin = origin(addr);
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let get_page = || async {
        http.get(format!("http://{addr}/page"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    };
    assert_eq!(get_page().await, "route-v1", "{}", server.stderr());
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    socket.send(Message::Text("a".into())).await.unwrap();
    assert_eq!(next_data(&mut socket).await, Message::Text("a".into()));

    // Edit the existing route file in place (its directory does not change).
    tokio::time::sleep(Duration::from_millis(1100)).await;
    std::fs::write(&page, route("route-v2")).unwrap();
    assert_eq!(get_page().await, "route-v2", "{}", server.stderr());
    assert_eq!(
        close_frame(&mut socket).await,
        Some((CloseCode::Restart, "server code reloaded".to_string())),
        "{}",
        server.stderr()
    );
}

const STALE_WORKER_APP: &str = r#"
import { text, ws_endpoint, websocket, ws_send } from "std/http/server"
fn on_message(conn, message, state) {
    print("MESSAGE v1 " + state["schema"])
    ws_send(conn, "v1:" + state["schema"])
    return state
}
let live = ws_endpoint("live", on_message)
get("/ws", fn(req) { websocket(live, map { "state": map { "schema": "old" } }) })
get("/version", fn(req) { text("v1") })
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

#[tokio::test]
async fn stale_development_worker_never_mixes_old_and_new_code() {
    // Two development workers: only worker 0 reloads, so worker 1 keeps the
    // old code until it restarts. Old state must never reach the new
    // callbacks, and new state must never reach the old ones.
    let mut server = fixture::Fixture::start(STALE_WORKER_APP, &["run"], &[("NTNT_WORKERS", "2")]);
    let addr = server.ready();
    let origin = origin(addr);
    let app = server.dir.path().join("fixture.tnt");
    let previous = std::fs::metadata(&app).unwrap().modified().unwrap();
    std::fs::write(
        &app,
        STALE_WORKER_APP
            .replace("v1", "v2")
            .replace("\"old\"", "\"new\""),
    )
    .unwrap();
    std::fs::File::options()
        .write(true)
        .open(&app)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(previous + Duration::from_secs(2)))
        .unwrap();
    // Requests alternate between the workers; worker 0 reloads on its next one.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while get_body(addr, "/version").await != "v2" {
        assert!(std::time::Instant::now() < deadline, "{}", server.stderr());
    }
    let mut replies = Vec::new();
    for _ in 0..20 {
        let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
        let _ = socket.send(Message::Text("x".into())).await;
        let frame = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                match socket.next().await {
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                    other => return other,
                }
            }
        })
        .await
        .expect("no frame within 10s");
        match frame {
            Some(Ok(Message::Text(reply))) => replies.push(reply),
            // A socket that reached worker 1 closes before any callback runs.
            Some(Ok(Message::Close(Some(close)))) => {
                assert_eq!(close.code, CloseCode::Restart, "{}", close.reason)
            }
            other => panic!("unexpected {other:?}: {}", server.stderr()),
        }
        if replies.len() >= 4 {
            break;
        }
    }
    assert!(!replies.is_empty(), "no socket reached the reloaded worker");
    assert!(
        replies.iter().all(|reply| reply == "v2:new"),
        "{replies:?}\n{}",
        server.stdout()
    );
    let stdout = server.stdout();
    assert!(
        !stdout.contains("MESSAGE v2 old") && !stdout.contains("MESSAGE v1 new"),
        "{stdout}"
    );
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
        .args(["--get", "/ws", "--port", "18197"])
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

#[test]
fn ntnt_test_rejects_request_time_registration() {
    // Same setup-only rule as the async server, including through an alias.
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("app.tnt");
    std::fs::write(
        &app,
        r#"
import { text, ws_endpoint } from "std/http/server"
import { ws_endpoint as register } from "std/http/server"
get("/late", fn(req) {
    ws_endpoint("late", fn(conn, message, state) { state })
    return text("registered")
})
get("/alias", fn(req) {
    register("alias", fn(conn, message, state) { state })
    return text("registered")
})
listen(8080)
"#,
    )
    .unwrap();
    for path in ["/late", "/alias"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .args(["test"])
            .arg(&app)
            .args(["--get", path, "--port", "18198"])
            .env_remove("NTNT_ENV")
            .output()
            .unwrap();
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(all.contains("[RESPONSE] 500"), "{path}: {all}");
        assert!(
            all.contains("can only be called during application setup"),
            "{path}: {all}"
        );
    }
}

#[test]
fn production_set_by_load_env_after_caching_dev_mode_still_requires_origins() {
    // An `otherwise` that catches an error caches the mode before load_env
    // sets production.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("mode.env"), "NTNT_ENV=production\n").unwrap();
    std::fs::write(
        dir.path().join("app.tnt"),
        r#"
import { text, ws_endpoint } from "std/http/server"
import { load_env } from "std/env"
fn warm() {
    let z = (1 / 0) otherwise { return 0 }
    return z
}
let w = warm()
load_env("mode.env")
let chat = ws_endpoint("chat", fn(conn, message, state) { state })
get("/", fn(req) { text("ok") })
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#,
    )
    .unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .current_dir(dir.path())
        .args(["run", "app.tnt"])
        .env_remove("NTNT_ENV")
        .env_remove("SITE_URL")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // The fix makes startup fail at once; the bug lets the server run. Never
    // wait on a server that started.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("server started without trusted origins in production");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stderr = String::new();
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut stderr).unwrap();
    assert!(!status.success(), "{stderr}");
    assert!(
        stderr.contains("set SITE_URL or pass the `origins` option"),
        "{stderr}"
    );
}

const SCHEDULING_APP: &str = r#"
import { text, ws_endpoint, websocket, ws_send } from "std/http/server"
import { sleep_ms } from "std/concurrent"
fn echo(conn, message, state) {
    ws_send(conn, message["data"])
    return state
}
let fast = ws_endpoint("fast", echo)
get("/ws", fn(req) { websocket(fast) })
get("/slow", fn(req) {
    sleep_ms(1200)
    return text("slow done")
})
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

#[tokio::test]
async fn released_callback_slot_wakes_an_idle_worker() {
    // Two workers, one callback slot. When the slot holder takes a long HTTP
    // request it must hand the slot to the idle worker, so a callback runs
    // while that request is still in progress, without any other request.
    // Which worker takes the request is up to the scheduler, so repeat:
    // without the wakeup, each round has about an even chance of stalling.
    let mut server = fixture::Fixture::start(SCHEDULING_APP, &["run"], &[("NTNT_WORKERS", "2")]);
    let addr = server.ready();
    let origin = origin(addr);
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    socket.send(Message::Text("warm".into())).await.unwrap();
    assert_eq!(next_data(&mut socket).await, Message::Text("warm".into()));
    for round in 0..8 {
        let slow = tokio::spawn(async move {
            reqwest::get(format!("http://{addr}/slow"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        let started = std::time::Instant::now();
        socket
            .send(Message::Text(format!("r{round}")))
            .await
            .unwrap();
        assert_eq!(
            next_data(&mut socket).await,
            Message::Text(format!("r{round}"))
        );
        assert!(
            started.elapsed() < Duration::from_millis(600),
            "round {round}: callback waited {:?} behind the slow request",
            started.elapsed()
        );
        assert_eq!(slow.await.unwrap(), "slow done");
    }
}

const CLOSE_OVERLAP_APP: &str = r#"
import { ws_endpoint, websocket } from "std/http/server"
import { sleep_ms } from "std/concurrent"
fn slow_message(conn, message, state) {
    print("MESSAGE_BEGIN")
    sleep_ms(int(message["data"]) ?? 0)
    print("MESSAGE_END")
    return state
}
fn closed(state, info) {
    print("CLOSE_BEGIN")
}
let ep = ws_endpoint("slow", slow_message, map {
    "on_close": closed,
    "callback_timeout_ms": 400
})
get("/ws", fn(req) { websocket(ep) })
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

fn markers(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .filter(|l| matches!(*l, "MESSAGE_BEGIN" | "MESSAGE_END" | "CLOSE_BEGIN"))
        .collect()
}

#[tokio::test]
async fn on_close_never_overlaps_a_callback_that_outlived_its_deadline() {
    let mut server = fixture::Fixture::start(CLOSE_OVERLAP_APP, &["run"], &[("NTNT_WORKERS", "3")]);
    let addr = server.ready();
    let origin = origin(addr);

    // Past the deadline: the socket closes with 1011 promptly, and on_close
    // is skipped because the message callback is still running.
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    socket.send(Message::Text("1500".into())).await.unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        close_frame(&mut socket).await.map(|(code, _)| code),
        Some(CloseCode::Error)
    );
    assert!(
        started.elapsed() < Duration::from_millis(1200),
        "{:?}",
        started.elapsed()
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !server.stdout().contains("MESSAGE_END") {
        assert!(std::time::Instant::now() < deadline, "{}", server.stdout());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(markers(&server.stdout()), ["MESSAGE_BEGIN", "MESSAGE_END"]);
    assert!(
        server.stderr().contains("on_close skipped"),
        "{}",
        server.stderr()
    );

    // Peer closes while a callback runs within its deadline: on_close starts
    // only after the message callback finished.
    let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
    socket.send(Message::Text("250".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    socket.close(None).await.unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while markers(&server.stdout()).len() < 5 {
        assert!(std::time::Instant::now() < deadline, "{}", server.stdout());
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        markers(&server.stdout())[2..],
        ["MESSAGE_BEGIN", "MESSAGE_END", "CLOSE_BEGIN"]
    );
}

const BROADCAST_APP: &str = r#"
import { text, ws_endpoint, websocket, ws_send } from "std/http/server"
import { broadcast, publish } from "std/concurrent"
let feed = broadcast("feed", map { "capacity": 4 })
fn on_message(conn, message, state) {
    ws_send(conn, "echo:" + message["data"])
    return state
}
let live = ws_endpoint("live", on_message)
get("/ws", fn(req) { websocket(live, map { "source": feed }) })
get("/pub", fn(req) {
    let n = int(req.query_params["n"] ?? "0") ?? 0
    let pad = req.query_params["pad"] ?? ""
    let res = publish(feed, map { "n": n, "pad": pad }, map { "event": "tick", "id": n })
    return text(str(is_ok(res)))
})
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

async fn publish_n(addr: SocketAddr, n: usize, pad: usize) {
    let body = reqwest::get(format!("http://{addr}/pub?n={n}&pad={}", "x".repeat(pad)))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(body, "true");
}

#[tokio::test]
async fn broadcast_source_reaches_every_socket_across_workers() {
    let mut server = fixture::Fixture::start(BROADCAST_APP, &["run"], &[("NTNT_WORKERS", "2")]);
    let addr = server.ready();
    let origin = origin(addr);
    let mut sockets = Vec::new();
    for _ in 0..4 {
        sockets.push(connect(addr, "/ws", Some(&origin)).await.unwrap());
    }
    // A round trip proves each owner is running (and subscribed) first.
    for socket in &mut sockets {
        socket.send(Message::Text("hi".into())).await.unwrap();
        assert_eq!(next_data(socket).await, Message::Text("echo:hi".into()));
    }
    for n in 1..=3 {
        publish_n(addr, n, 0).await;
    }
    for socket in &mut sockets {
        for n in 1..=3 {
            // The published JSON value only; SSE event/id metadata is not added.
            assert_eq!(
                next_data(socket).await,
                Message::Text(format!(r#"{{"n":{n},"pad":""}}"#))
            );
        }
    }
    // Callbacks and the source share one ordered connection.
    sockets[0]
        .send(Message::Text("after".into()))
        .await
        .unwrap();
    assert_eq!(
        next_data(&mut sockets[0]).await,
        Message::Text("echo:after".into())
    );
}

#[tokio::test]
async fn socket_that_falls_behind_the_broadcast_is_closed_with_1013() {
    let mut server = fixture::Fixture::start(BROADCAST_APP, &["run"], &[("NTNT_WORKERS", "2")]);
    let addr = server.ready();
    let origin = origin(addr);
    let mut healthy = connect(addr, "/ws", Some(&origin)).await.unwrap();
    let mut slow = connect(addr, "/ws", Some(&origin)).await.unwrap();
    for socket in [&mut healthy, &mut slow] {
        socket.send(Message::Text("hi".into())).await.unwrap();
        assert_eq!(next_data(socket).await, Message::Text("echo:hi".into()));
    }
    // Keep the healthy socket read in the background.
    let (_tx, mut rx) = healthy.split();
    let (seen_tx, mut seen) = tokio::sync::mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        while let Some(Ok(message)) = rx.next().await {
            if let Message::Text(text) = message {
                let _ = seen_tx.send(text);
            }
        }
    });
    // The slow socket reads nothing while many large events are published, so
    // its owner blocks writing and the capacity-4 bus overtakes it.
    let total = 120;
    for n in 1..=total {
        publish_n(addr, n, 48 * 1024).await;
    }
    let mut code = None;
    let _ = tokio::time::timeout(Duration::from_secs(15), async {
        while let Some(Ok(message)) = slow.next().await {
            if let Message::Close(Some(frame)) = message {
                code = Some(frame.code);
            }
        }
    })
    .await;
    assert_eq!(code, Some(CloseCode::Again), "{}", server.stderr());
    // The healthy socket received every event in order.
    for n in 1..=total {
        let text = tokio::time::timeout(Duration::from_secs(10), seen.recv())
            .await
            .expect("healthy socket stalled")
            .expect("healthy socket closed");
        assert!(
            text.starts_with(&format!(r#"{{"n":{n},"#)),
            "{n}: {}",
            &text[..20]
        );
    }
    reader.abort();
}
