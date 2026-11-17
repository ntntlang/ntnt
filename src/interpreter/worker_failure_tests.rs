//! Real HTTP/WebSocket worker-failure tests with an explicit native panic.
//! This entire module and its builtins are absent from normal library/CLI builds.
use super::{Interpreter, Value};
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

#[path = "../../tests/support/system_fixture.rs"]
mod fixture;
use fixture::HANG_GUARD;

/// How long a fixture waits for the test to remove one of its gate files. A
/// gate can stay closed across many test steps, so this is far longer than a
/// whole test: only a server left behind by a test that died reaches it.
const GATE_GUARD: Duration = Duration::from_secs(600);

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub(super) fn install(interpreter: &mut Interpreter) {
    let mut environment = interpreter.environment.borrow_mut();
    environment.define(
        "test_worker_panic".into(),
        Value::NativeFunction {
            name: "test_worker_panic".into(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_| panic!("injected HTTP worker failure"),
        },
    );
    environment.define(
        "test_admitted_callbacks".into(),
        Value::NativeFunction {
            name: "test_admitted_callbacks".into(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_| {
                let jobs = crate::stdlib::websocket::admitted_callback_jobs();
                Ok(Value::Int(jobs as i64))
            },
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
    // A request held by a gate must not time out before the gate opens.
    interpreter.set_request_timeout(GATE_GUARD.as_secs());
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
    tokio::time::timeout(HANG_GUARD, tokio_tungstenite::connect_async(request))
        .await
        .expect("handshake timed out")
        .map(|(socket, _)| socket)
}

/// Next data message, skipping pings.
async fn next_data(socket: &mut Socket) -> Message {
    loop {
        let message = tokio::time::timeout(HANG_GUARD, socket.next())
            .await
            .expect("no message within the hang guard")
            .expect("socket ended")
            .expect("socket error");
        if !matches!(message, Message::Ping(_) | Message::Pong(_)) {
            return message;
        }
    }
}

/// GET with one hang guard on the entire request and body.
async fn get_body(addr: SocketAddr, path: &str) -> String {
    tokio::time::timeout(HANG_GUARD, async {
        reqwest::get(format!("http://{addr}{path}"))
            .await?
            .text()
            .await
    })
    .await
    .expect("no response within the hang guard")
    .expect("request failed")
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
import { exists } from "std/fs"
import { monotonic_now } from "std/time"
fn wait_until_removed(path) {
    let deadline = monotonic_now() + GATE_GUARD_MS
    while exists(path) {
        assert(monotonic_now() < deadline)
        sleep_ms(10)
    }
}
// Absent during initial startup; a restart waits before becoming a live worker.
if exists(RESTART_BLOCK) {
    print("RESTART_WAIT")
    wait_until_removed(RESTART_BLOCK)
}
let mut primary = true
fn slow(conn, message, state) {
    print("CALLBACK_BEGIN")
    wait_until_removed(CALLBACK_BLOCK)
    ws_send(conn, "done:" + message["data"])
    return state
}
// The longest callback deadline: the test releases callbacks on its events.
let ep = ws_endpoint("slow", slow, map { "callback_timeout_ms": 300000 })
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
print("SECONDARY_SETUP")
"#;

#[tokio::test]
async fn worker_loss_keeps_a_worker_free_for_http() {
    // Three workers share two callback slots. Hold the restarted worker in
    // setup, even past its backoff, so only one of the two live workers may
    // run a callback. The other must answer HTTP while that callback is held.
    let gates = tempfile::tempdir().unwrap();
    let restart_block = gates.path().join("restart-block");
    let callback_block = gates.path().join("callback-block");
    std::fs::write(&callback_block, b"blocked").unwrap();
    let source = WORKER_LOSS_APP
        .replace("GATE_GUARD_MS", &GATE_GUARD.as_millis().to_string())
        .replace(
            "RESTART_BLOCK",
            &serde_json::to_string(restart_block.to_str().unwrap()).unwrap(),
        )
        .replace(
            "CALLBACK_BLOCK",
            &serde_json::to_string(callback_block.to_str().unwrap()).unwrap(),
        );
    let mut server = start(&source, &[("NTNT_WORKERS", "3")]);
    let addr = server.ready();
    // All initial workers have passed setup; only a restarted one sees this.
    std::fs::write(&restart_block, b"blocked").unwrap();
    let origin = origin(addr);
    // Read in the background, so the sockets answer server pings however long
    // the test takes.
    let (mut a, a_reader) = connect(addr, "/ws", Some(&origin)).await.unwrap().split();
    let mut a_replies = read_in_background(a_reader);
    let (mut b, b_reader) = connect(addr, "/ws", Some(&origin)).await.unwrap().split();
    let mut b_replies = read_in_background(b_reader);
    // Worker 0 refuses /crash with 409: retry until another worker takes it.
    let deadline = std::time::Instant::now() + HANG_GUARD;
    loop {
        let response =
            tokio::time::timeout(HANG_GUARD, reqwest::get(format!("http://{addr}/crash"))).await;
        if !matches!(response, Ok(Ok(ref r)) if r.status() == 409) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no other worker took /crash: {}",
            server.stderr()
        );
    }
    wait_for(&server, "no worker crashed", |s| {
        s.stderr().contains("crashed (panic)")
    })
    .await;
    // Observe the restart inside its setup gate, not just the panic log. This
    // guarantees the reduced live pool even if the test was descheduled for
    // longer than the supervisor's one-second restart backoff.
    wait_for(
        &server,
        "restarted worker did not reach restart gate",
        |s| s.stdout().contains("RESTART_WAIT"),
    )
    .await;
    a.send(Message::Text("a".into())).await.unwrap();
    b.send(Message::Text("b".into())).await.unwrap();
    wait_for(&server, "no callback reached its gate", |s| {
        s.stdout().contains("CALLBACK_BEGIN")
    })
    .await;
    // Neither callbacks nor the restarted worker can unblock HTTP by finishing:
    // the parent releases both gates only after this request succeeds.
    assert_eq!(get_body(addr, "/health").await, "ok");
    assert_eq!(
        server.stdout().matches("CALLBACK_BEGIN").count(),
        1,
        "more than one callback ran with two live workers: {}",
        server.stdout()
    );
    std::fs::remove_file(&callback_block).unwrap();
    std::fs::remove_file(&restart_block).unwrap();
    assert_eq!(
        next_reply(&mut a_replies).await,
        Message::Text("done:a".into())
    );
    assert_eq!(
        next_reply(&mut b_replies).await,
        Message::Text("done:b".into())
    );
    // Initial readiness required both secondary setups. A third proves the
    // supervisor's restarted interpreter completed setup after the release.
    wait_for(&server, "restarted worker did not finish setup", |s| {
        s.stdout().matches("SECONDARY_SETUP").count() >= 3
    })
    .await;
}

const SATURATION_APP: &str = r#"
import { text, status, ws_endpoint, websocket, ws_send } from "std/http/server"
import { sleep_ms } from "std/concurrent"
import { exists } from "std/fs"
import { monotonic_now } from "std/time"
fn wait_until_removed(path) {
    let deadline = monotonic_now() + GATE_GUARD_MS
    while exists(path) {
        assert(monotonic_now() < deadline)
        sleep_ms(10)
    }
}
// Absent during initial startup; a restart waits before becoming a live worker.
if exists(RESTART_BLOCK) {
    print("RESTART_WAIT")
    wait_until_removed(RESTART_BLOCK)
}
let mut primary = true
fn gated(conn, message, state) {
    let id = message["data"]
    print("CALLBACK_BEGIN " + id)
    wait_until_removed(GATES + "/" + id)
    ws_send(conn, "done:" + id)
    return state
}
// The longest callback deadline and lifetime: the test releases callbacks on
// its events, and the sockets live for the whole test.
let ep = ws_endpoint("gated", gated, map { "callback_timeout_ms": 300000, "max_lifetime_ms": 86400000 })
get("/ws", fn(req) { websocket(ep) })
get("/health", fn(req) { text("ok") })
// Admitted callback jobs. This builtin exists only in the Rust unit-test binary.
get("/admitted", fn(req) { text(str(test_admitted_callbacks())) })
// The same, for the one request that must wait for a worker.
get("/pending", fn(req) { text(str(test_admitted_callbacks())) })
get("/hold", fn(req) {
    if primary {
        return status(409, "primary worker")
    }
    print("HOLD_BEGIN")
    wait_until_removed(HOLD_BLOCK)
    return text("held")
})
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
print("SECONDARY_SETUP")
"#;

/// Messages from one socket, read in the background so it is always polled.
fn read_in_background(
    mut socket: futures_util::stream::SplitStream<Socket>,
) -> tokio::sync::mpsc::UnboundedReceiver<Message> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(Ok(message)) = socket.next().await {
            if !matches!(message, Message::Ping(_) | Message::Pong(_)) && tx.send(message).is_err()
            {
                break;
            }
        }
    });
    rx
}

async fn next_reply(messages: &mut tokio::sync::mpsc::UnboundedReceiver<Message>) -> Message {
    tokio::time::timeout(HANG_GUARD, messages.recv())
        .await
        .expect("no message within the hang guard")
        .expect("socket ended")
}

/// Ids of the callbacks that have begun, in order.
fn begun(server: &fixture::Fixture) -> Vec<String> {
    server
        .stdout()
        .lines()
        .filter_map(|line| line.strip_prefix("CALLBACK_BEGIN "))
        .map(str::to_string)
        .collect()
}

/// Wait until `done` holds for the server. Only a hang reaches the guard.
async fn wait_for(server: &fixture::Fixture, what: &str, done: impl Fn(&fixture::Fixture) -> bool) {
    let deadline = std::time::Instant::now() + HANG_GUARD;
    while !done(server) {
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: {}\n{}",
            server.stdout(),
            server.stderr()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn saturated_callbacks_recover_from_worker_loss_without_starving() {
    // Three workers share two callback slots, and every admission permit is
    // taken: two callbacks run while the rest wait in the queue. A worker
    // then dies. Callbacks are never preempted, so HTTP waits for one of the
    // running callbacks; the reduced pool then keeps a worker for HTTP, and
    // after the restart every queued callback runs once.
    let admitted = crate::stdlib::websocket::MAX_CALLBACK_JOBS;
    let gates = tempfile::tempdir().unwrap();
    let restart_block = gates.path().join("restart-block");
    let hold_block = gates.path().join("hold-block");
    let callback_gates = gates.path().join("callbacks");
    std::fs::create_dir(&callback_gates).unwrap();
    for i in 0..admitted {
        std::fs::write(callback_gates.join(format!("m{i}")), b"blocked").unwrap();
    }
    std::fs::write(&hold_block, b"blocked").unwrap();
    let literal = |path: &std::path::Path| serde_json::to_string(path.to_str().unwrap()).unwrap();
    let source = SATURATION_APP
        .replace("GATE_GUARD_MS", &GATE_GUARD.as_millis().to_string())
        .replace("RESTART_BLOCK", &literal(&restart_block))
        .replace("HOLD_BLOCK", &literal(&hold_block))
        .replace("GATES", &literal(&callback_gates));
    // The server reports each request it queues for the workers.
    let mut server = start(
        &source,
        &[("NTNT_WORKERS", "3"), ("NTNT_TEST_TRACE_QUEUE", "1")],
    );
    let addr = server.ready();
    // All initial workers have passed setup; only a restarted one sees this.
    std::fs::write(&restart_block, b"blocked").unwrap();
    let origin = origin(addr);
    // Handshakes need HTTP workers, so open every socket while all are free:
    // one per admission permit, and one more.
    let mut writers = Vec::new();
    let mut replies = Vec::new();
    for _ in 0..=admitted {
        let (writer, reader) = connect(addr, "/ws", Some(&origin)).await.unwrap().split();
        writers.push(writer);
        replies.push(read_in_background(reader));
    }
    // Keep one secondary worker in an HTTP request, so the first two
    // callbacks start on the other two workers. The primary refuses /hold,
    // so retry until a secondary takes it (bounded by the wait below).
    let hold = tokio::spawn(async move {
        loop {
            let body = get_body(addr, "/hold").await;
            if body != "primary worker" {
                return body;
            }
        }
    });
    wait_for(&server, "no secondary worker began /hold", |s| {
        s.stdout().contains("HOLD_BEGIN")
    })
    .await;
    for (i, writer) in writers.iter_mut().take(admitted).enumerate() {
        writer.send(Message::Text(format!("m{i}"))).await.unwrap();
    }
    wait_for(&server, "the callback slots were not used", |s| {
        begun(s).len() >= 2
    })
    .await;
    // Released, the held worker answers HTTP while both slots are busy.
    std::fs::remove_file(&hold_block).unwrap();
    let held = tokio::time::timeout(HANG_GUARD, hold)
        .await
        .expect("/hold did not finish within the hang guard")
        .unwrap();
    assert_eq!(held, "held");
    assert_eq!(get_body(addr, "/health").await, "ok");
    // A message is admitted when its connection owner reads it, which can be
    // after the send completes. While this worker is free for HTTP, wait until
    // every message holds an admission permit: none is returned before a
    // callback gate opens, so the overload check below cannot take one.
    let deadline = std::time::Instant::now() + HANG_GUARD;
    while get_body(addr, "/admitted").await != admitted.to_string() {
        assert!(
            std::time::Instant::now() < deadline,
            "not every message was admitted: {}",
            server.stdout()
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(begun(&server).len(), 2, "{}", server.stdout());
    // It is the only free worker, so it takes /crash.
    let crash =
        tokio::time::timeout(HANG_GUARD, reqwest::get(format!("http://{addr}/crash"))).await;
    assert!(
        !matches!(crash, Ok(Ok(ref r)) if r.status() == 409),
        "the primary worker took /crash"
    );
    wait_for(&server, "no worker crashed", |s| {
        s.stderr().contains("crashed (panic)")
    })
    .await;
    wait_for(&server, "the worker did not restart into its gate", |s| {
        s.stdout().contains("RESTART_WAIT")
    })
    .await;
    // Admission stays bounded during the loss: one more callback is refused.
    writers[admitted]
        .send(Message::Text("over".into()))
        .await
        .unwrap();
    match next_reply(&mut replies[admitted]).await {
        Message::Close(Some(frame)) => assert_eq!(frame.code, CloseCode::Again),
        other => panic!("expected a 1013 close, got {other:?}"),
    }
    // Both live workers run callbacks, which are not preempted: a request
    // waits in the queue until one finishes (or the worker restarts). Once
    // the request is queued, release one callback. The request reports the
    // admitted jobs when it is served: one fewer, so it was served only
    // after that callback had finished.
    let pending = tokio::spawn(get_body(addr, "/pending"));
    wait_for(&server, "the request was not queued", |s| {
        s.stdout().contains("HTTP_QUEUED /pending\n")
    })
    .await;
    let running = begun(&server);
    std::fs::remove_file(callback_gates.join(&running[0])).unwrap();
    assert_eq!(
        pending.await.unwrap(),
        (admitted - 1).to_string(),
        "HTTP was answered while every live worker ran a callback"
    );
    let finished: usize = running[0][1..].parse().unwrap();
    assert_eq!(
        next_reply(&mut replies[finished]).await,
        Message::Text(format!("done:{}", running[0]))
    );
    // Two live workers keep one callback slot: the worker that finished
    // stays free for HTTP, and no queued callback starts.
    assert_eq!(get_body(addr, "/health").await, "ok");
    assert_eq!(begun(&server).len(), 2, "{}", server.stdout());
    // The restarted worker brings the second slot back: a queued callback
    // starts beside the running one, and a worker is still free for HTTP.
    std::fs::remove_file(&restart_block).unwrap();
    wait_for(&server, "the restarted worker did not finish setup", |s| {
        s.stdout().matches("SECONDARY_SETUP").count() >= 3
    })
    .await;
    wait_for(&server, "the restored slot took no queued callback", |s| {
        begun(s).len() >= 3
    })
    .await;
    assert_eq!(get_body(addr, "/health").await, "ok");
    assert_eq!(begun(&server).len(), 3, "{}", server.stdout());
    // Released, every queued callback runs exactly once.
    for i in 0..admitted {
        let _ = std::fs::remove_file(callback_gates.join(format!("m{i}")));
    }
    for (i, reply) in replies.iter_mut().enumerate().take(admitted) {
        if i != finished {
            assert_eq!(next_reply(reply).await, Message::Text(format!("done:m{i}")));
        }
    }
    let mut ran = begun(&server);
    ran.sort();
    let mut expected: Vec<String> = (0..admitted).map(|i| format!("m{i}")).collect();
    expected.sort();
    assert_eq!(ran, expected, "every admitted callback runs once");
    // New connections are admitted and served.
    for name in ["fresh", "again"] {
        let mut socket = connect(addr, "/ws", Some(&origin)).await.unwrap();
        socket.send(Message::Text(name.into())).await.unwrap();
        assert_eq!(
            next_data(&mut socket).await,
            Message::Text(format!("done:{name}"))
        );
    }
}
