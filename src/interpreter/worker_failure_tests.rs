//! Real HTTP/WebSocket worker-failure tests with an explicit native panic.
//! This entire module and its builtin are absent from normal library/CLI builds.
use super::{Interpreter, Value};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

#[path = "../../tests/support/system_fixture.rs"]
mod fixture;

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) fn install(interpreter: &mut Interpreter) {
    interpreter.environment.borrow_mut().define(
        "test_worker_panic".into(),
        Value::NativeFunction {
            name: "test_worker_panic".into(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_| panic!("injected HTTP worker failure"),
        },
    );
}

fn start(source: &str, env: &[(&str, &str)]) -> fixture::Fixture {
    fixture::Fixture::start_with_command(source, env, |file| {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "interpreter::worker_failure_tests::serve_fixture",
                "--nocapture",
                "--test-threads=1",
                "--format=terse",
            ])
            .env("NTNT_WORKER_FAULT_SOURCE", file);
        command
    })
}

#[test]
fn serve_fixture() {
    let Some(file) = std::env::var_os("NTNT_WORKER_FAULT_SOURCE") else {
        return;
    };
    let source = crate::source::SourceFile::read(&file).unwrap();
    let parsed = source.parse().unwrap();
    let mut interpreter = Interpreter::new();
    interpreter.set_main_source_file(file.to_str().unwrap());
    interpreter.eval_source(&parsed).unwrap();
}

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

/// GET with a deadline on the entire request and body.
async fn timed_get(addr: SocketAddr, path: &str) -> (String, Duration) {
    let started = std::time::Instant::now();
    let body = tokio::time::timeout(Duration::from_secs(10), async {
        reqwest::get(format!("http://{addr}{path}"))
            .await?
            .text()
            .await
    })
    .await
    .expect("no response within 10s")
    .expect("request failed");
    (body, started.elapsed())
}

async fn get_body(addr: SocketAddr, path: &str) -> String {
    timed_get(addr, path).await.0
}

const RESTART_APP: &str = r#"
import { text, status, ws_endpoint, websocket, ws_send } from "std/http/server"
import { sleep_ms } from "std/concurrent"
print("LOADED v1")
let mut primary = true
fn on_message(conn, message, state) {
    print("MESSAGE v1 " + state["schema"])
    ws_send(conn, "v1:" + state["schema"])
    return state
}
let live = ws_endpoint("live", on_message)
get("/ws", fn(req) { websocket(live, map { "state": map { "schema": "old" } }) })
get("/version", fn(req) { text("v1") })
get("/hold", fn(req) {
    if !primary {
        return status(409, "not worker 0")
    }
    print("HOLD_BEGIN")
    sleep_ms(6000)
    return text("held")
})
get("/crash", fn(req) {
    // Worker 0 serves on the main thread: a crash there ends the process.
    if primary {
        return status(409, "worker 0")
    }
    // This builtin exists only in the Rust unit-test binary.
    test_worker_panic()
})
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
// listen() serves until shutdown on worker 0 and returns in the others.
primary = false
"#;

#[tokio::test]
async fn worker_restarted_after_reload_takes_no_websocket_work() {
    // Reload v1 to v2 on worker 0 and keep a v2 socket open. Then, with
    // worker 0 busy, revert the file to v1 and crash worker 1: its restart
    // loads v1, which still matches the startup source. It must not run the
    // v1 callback on the v2 socket's state.
    let mut server = start(RESTART_APP, &[("NTNT_WORKERS", "2")]);
    let addr = server.ready();
    let origin = origin(addr);
    let app = server.dir.path().join("fixture.tnt");
    let started = std::fs::metadata(&app).unwrap().modified().unwrap();
    let write = |source: &str, ahead: u64| {
        std::fs::write(&app, source).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&app)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(started + Duration::from_secs(ahead)))
            .unwrap();
    };
    // A hot reload re-runs the file and listen() returns there too, so the
    // reloaded worker 0 must keep `primary`.
    write(
        &RESTART_APP
            .replace("v1", "v2")
            .replace("\"old\"", "\"new\"")
            .replace("\nprimary = false\n", "\n"),
        2,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while get_body(addr, "/version").await != "v2" {
        assert!(std::time::Instant::now() < deadline, "{}", server.stderr());
    }
    // A v2 socket on worker 0 (sockets that reach worker 1 close with 1012).
    let mut fresh = None;
    for _ in 0..20 {
        let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
        let _ = socket.send(Message::Text("x".into())).await;
        if let Message::Text(reply) = next_data(&mut socket).await {
            assert_eq!(reply, "v2:new");
            fresh = Some(socket);
            break;
        }
    }
    let mut fresh = fresh.expect("no socket reached worker 0");
    // Hold worker 0 (worker 1 refuses /hold), so later requests reach worker 1.
    let hold = tokio::spawn(async move {
        for _ in 0..20 {
            if get_body(addr, "/hold").await == "held" {
                return;
            }
        }
        panic!("worker 0 never took /hold");
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !server.stdout().contains("HOLD_BEGIN") {
        assert!(std::time::Instant::now() < deadline, "{}", server.stderr());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    write(RESTART_APP, 4);
    let _ = tokio::time::timeout(
        Duration::from_secs(10),
        reqwest::get(format!("http://{addr}/crash")),
    )
    .await;
    // Worker 1 restarts after a second and loads v1 again.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while server.stdout().matches("LOADED v1").count() < 3 {
        assert!(
            std::time::Instant::now() < deadline,
            "{}\n{}",
            server.stdout(),
            server.stderr()
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    // Only worker 0 has the code that built this state; it answers once free.
    fresh.send(Message::Text("y".into())).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(15), next_data(&mut fresh))
        .await
        .expect("no reply within 15s");
    assert_eq!(reply, Message::Text("v2:new".into()), "{}", server.stdout());
    assert!(
        !server.stdout().contains("MESSAGE v1 new"),
        "{}",
        server.stdout()
    );
    tokio::time::timeout(Duration::from_secs(15), hold)
        .await
        .expect("hold did not finish")
        .unwrap();
}

const WORKER_LOSS_APP: &str = r#"
import { text, status, ws_endpoint, websocket, ws_send } from "std/http/server"
import { sleep_ms } from "std/concurrent"
let mut primary = true
fn slow(conn, message, state) {
    sleep_ms(1500)
    ws_send(conn, "done:" + message["data"])
    return state
}
let ep = ws_endpoint("slow", slow)
get("/ws", fn(req) { websocket(ep) })
get("/health", fn(req) { text("ok") })
get("/crash", fn(req) {
    // Worker 0 serves on the main thread: a crash there ends the process.
    if primary {
        return status(409, "primary worker")
    }
    // This builtin exists only in the Rust unit-test binary.
    test_worker_panic()
})
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
// listen() serves until shutdown on worker 0 and returns in the others.
primary = false
"#;

#[tokio::test]
async fn worker_loss_keeps_a_worker_free_for_http() {
    // Three workers share two callback slots, and idle workers hold them while
    // they wait. After one worker crashes, only one callback may run until it
    // restarts (after a second), so HTTP is still answered at once.
    let mut server = start(WORKER_LOSS_APP, &[("NTNT_WORKERS", "3")]);
    let addr = server.ready();
    let origin = origin(addr);
    let mut a = connect(addr, "/ws", Some(&origin)).await.unwrap();
    let mut b = connect(addr, "/ws", Some(&origin)).await.unwrap();
    // Requests rotate between workers, and worker 0 refuses with 409.
    let mut crashed = false;
    for _ in 0..10 {
        let response = tokio::time::timeout(
            Duration::from_secs(10),
            reqwest::get(format!("http://{addr}/crash")),
        )
        .await;
        if !matches!(response, Ok(Ok(ref r)) if r.status() == 409) {
            crashed = true;
            break;
        }
    }
    assert!(crashed, "no other worker took /crash: {}", server.stderr());
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !server.stderr().contains("crashed (panic)") {
        assert!(std::time::Instant::now() < deadline, "{}", server.stderr());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    a.send(Message::Text("a".into())).await.unwrap();
    b.send(Message::Text("b".into())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let (body, waited) = timed_get(addr, "/health").await;
    assert_eq!(body, "ok");
    assert!(
        waited < Duration::from_millis(500),
        "HTTP waited {waited:?} for the crashed worker to restart"
    );
    assert_eq!(next_data(&mut a).await, Message::Text("done:a".into()));
    assert_eq!(next_data(&mut b).await, Message::Text("done:b".into()));
}
