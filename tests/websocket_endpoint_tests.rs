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

/// Runs the shipped example on a free port, in development mode.
struct Example {
    child: std::process::Child,
    addr: SocketAddr,
    _log: tempfile::TempDir,
}

impl Example {
    fn start() -> Self {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let log = tempfile::tempdir().unwrap();
        let out = std::fs::File::create(log.path().join("out")).unwrap();
        let child = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(["run", "examples/websocket_demo/server.tnt"])
            .env_remove("NTNT_ENV")
            .env_remove("SITE_URL")
            .env_remove("NTNT_WORKERS")
            .env("NTNT_LISTEN_PORT", port.to_string())
            .stdout(std::process::Stdio::from(out.try_clone().unwrap()))
            .stderr(std::process::Stdio::from(out))
            .spawn()
            .unwrap();
        let mut example = Self {
            child,
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            _log: log,
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            assert!(
                example.child.try_wait().unwrap().is_none(),
                "example exited: {}",
                example.log()
            );
            if std::net::TcpStream::connect(example.addr).is_ok() {
                return example;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "example not listening: {}",
                example.log()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self._log.path().join("out")).unwrap_or_default()
    }
}

impl Drop for Example {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn websocket_example_echo_auth_and_feed_work_end_to_end() {
    let example = Example::start();
    // Each request and handshake has its own limit, and the whole flow has a
    // deadline, so a server that accepts connections and then stops answering
    // fails the test instead of hanging it.
    if tokio::time::timeout(Duration::from_secs(90), example_flow(&example))
        .await
        .is_err()
    {
        panic!("example flow exceeded 90s: {}", example.log());
    }
}

async fn example_flow(example: &Example) {
    let addr = example.addr;
    let origin = origin(addr);
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    // The page and its script are served.
    let page = http.get(format!("http://{addr}/")).send().await.unwrap();
    assert_eq!(page.status().as_u16(), 200, "{}", example.log());
    let script = http
        .get(format!("http://{addr}/static/app.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(script.status().as_u16(), 200);
    assert!(script.text().await.unwrap().contains("new WebSocket"));

    // Public echo keeps per-connection state.
    let mut echo = connect(addr, "/ws/echo", Some(&origin)).await.unwrap();
    echo.send(Message::Text("hi".into())).await.unwrap();
    assert_eq!(next_data(&mut echo).await, Message::Text("#1 hi".into()));
    echo.send(Message::Text("again".into())).await.unwrap();
    assert_eq!(next_data(&mut echo).await, Message::Text("#2 again".into()));
    // A maximum-size message is accepted, but "#3 " pushes the echo over the
    // send limit: the example refuses it out loud and keeps the count.
    echo.send(Message::Text("x".repeat(64 * 1024)))
        .await
        .unwrap();
    assert_eq!(
        next_data(&mut echo).await,
        Message::Text("not echoed: message too large".into())
    );
    echo.send(Message::Text("after".into())).await.unwrap();
    assert_eq!(next_data(&mut echo).await, Message::Text("#3 after".into()));

    // The feed refuses anonymous clients with ordinary HTTP.
    match connect(addr, "/ws/feed", Some(&origin)).await {
        Err(WsError::Http(response)) => assert_eq!(response.status().as_u16(), 401),
        other => panic!("expected 401, got {other:?}"),
    }

    // Sign in and keep the session cookie.
    let login = http
        .post(format!("http://{addr}/login"))
        .form(&[("email", "demo@example.com"), ("password", "demo-password")])
        .send()
        .await
        .unwrap();
    // Success and failure both redirect; only success goes to "/".
    assert!(login.status().is_redirection(), "{}", example.log());
    assert_eq!(
        login
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/"),
        "login failed: {}",
        example.log()
    );
    let cookie: String = login
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    assert!(!cookie.is_empty(), "no session cookie");

    let mut request = format!("ws://{addr}/ws/feed")
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("origin", origin.parse().unwrap());
    request
        .headers_mut()
        .insert("cookie", cookie.parse().unwrap());
    let (mut feed, _) = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_tungstenite::connect_async(request),
    )
    .await
    .expect("feed handshake timed out")
    .unwrap();
    assert_eq!(
        next_data(&mut feed).await,
        Message::Text("welcome demo@example.com".into())
    );
    feed.send(Message::Text("ping".into())).await.unwrap();
    assert_eq!(
        next_data(&mut feed).await,
        Message::Text("pong for demo@example.com".into())
    );

    // Publish through the CSRF-protected form; the feed receives the JSON.
    let signed_in_page = http
        .get(format!("http://{addr}/"))
        .header("cookie", &cookie)
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let csrf = signed_in_page
        .split("name=\"_csrf\" value=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("csrf field on the signed-in page")
        .to_string();
    let bad = http
        .post(format!("http://{addr}/announce"))
        .header("cookie", &cookie)
        .form(&[("_csrf", "wrong"), ("text", "nope")])
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status().as_u16(), 403);
    let ok = http
        .post(format!("http://{addr}/announce"))
        .header("cookie", &cookie)
        .form(&[("_csrf", csrf.as_str()), ("text", "hello all")])
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200, "{}", example.log());
    let Message::Text(json) = next_data(&mut feed).await else {
        panic!("expected a text announcement")
    };
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        value,
        serde_json::json!({"from": "demo@example.com", "text": "hello all"})
    );
}
