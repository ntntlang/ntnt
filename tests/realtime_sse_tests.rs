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

#[test]
fn configured_stream_and_topic_limits_apply_across_workers_and_release_on_disconnect() {
    let source = APP.replace(
        "listen(0, map {",
        "get(\"/new-topic\", fn(req) {\n    let other = broadcast(\"other\") otherwise { return text(err) }\n    return text(\"unexpected\")\n})\nlisten(0, map { \"realtime\": map { \"max_connections\": 1, \"max_topics\": 1 },",
    );
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "4")],
    );
    let addr = server.ready();
    // A denial and HEAD must not consume the sole stream permit.
    assert!(get_body(addr, "/private").starts_with("HTTP/1.1 401"));
    let mut head = open(addr, "/events", "HEAD");
    read_until(&mut head, &mut Vec::new(), b"\r\n\r\n", 1);
    let mut first = open(addr, "/events", "GET");
    read_until(&mut first, &mut Vec::new(), b"retry: 1000\n\n", 1);
    for _ in 0..8 {
        let rejected = get_body(addr, "/events");
        assert!(rejected.starts_with("HTTP/1.1 503"), "{rejected}");
        let topic = get_body(addr, "/new-topic");
        assert!(
            topic.contains("too many broadcast names (limit 1)"),
            "{topic}"
        );
        assert!(get_body(addr, "/pub?n=7").ends_with("true"));
    }
    drop(first);
    // Transport drop is asynchronous: retry only until its permit is released.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let mut next = open(addr, "/events", "GET");
        let mut response = Vec::new();
        read_until(&mut next, &mut response, b"\r\n\r\n", 1);
        if response.starts_with(b"HTTP/1.1 200") {
            read_until(&mut next, &mut response, b"retry: 1000\n\n", 1);
            break;
        }
        assert!(response.starts_with(b"HTTP/1.1 503"));
        assert!(Instant::now() < deadline, "stream permit leaked");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn configured_event_size_is_used_even_with_no_subscribers_and_can_be_raised() {
    for (limit, payload, accepted) in [(32, 64, false), (100_000, 70_000, true)] {
        let source = format!(
            r#"
import {{ text }} from "std/http/server"
import {{ broadcast, publish, spawn, await_task }} from "std/concurrent"
let bus = broadcast("size")
get("/publish", fn(req) {{
    let task = spawn(fn() {{ is_ok(publish(bus, "{}")) }})
    return text(str(await_task(task) ?? false))
}})
listen(0, map {{ "readiness": "json", "host": "127.0.0.1", "realtime": map {{ "max_event_bytes": {limit} }} }})
"#,
            "x".repeat(payload)
        );
        let mut server = fixture::Fixture::start(
            &source,
            &["run"],
            &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
        );
        let addr = server.ready();
        for _ in 0..4 {
            let response = get_body(addr, "/publish");
            assert!(response.ends_with(&accepted.to_string()), "{response}");
        }
    }
}

#[test]
fn configured_payload_budget_rejects_a_burst_and_is_released_on_disconnect() {
    // The probe frame needs the entire byte budget, not leftover space that
    // would allow a small publish to succeed even with leaked earlier frames.
    let probe = "x".repeat(512 - b"data: \"\"\n\n".len());
    let source = APP.replace(
        "listen(0, map {",
        &format!(
            r#"
get("/fill", fn(req) {{
    for n in 0..100000 {{
        let queued = publish(bus, "{}") otherwise {{ return text(err) }}
    }}
    return text("budget was not enforced")
}})
get("/probe", fn(req) {{ text(str(is_ok(publish(bus, "{}")))) }})
listen(0, map {{ "realtime": map {{ "max_event_bytes": 512, "max_payload_bytes": 512 }},
"#,
            "x".repeat(200),
            probe
        ),
    );
    let mut server = fixture::Fixture::start(
        &source,
        &["run"],
        &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
    );
    let addr = server.ready();
    let mut stream = open(addr, "/events", "GET");
    read_until(&mut stream, &mut Vec::new(), b"retry: 1000\n\n", 1);
    let response = get_body(addr, "/fill");
    assert!(
        response.contains("realtime payload budget exhausted"),
        "{response}"
    );
    drop(stream);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if get_body(addr, "/probe").ends_with("true") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "payload budget leaked after disconnect"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn invalid_realtime_options_and_incompatible_setup_fail_before_readiness() {
    for realtime in [
        "true",
        "map { \"unknown\": 1 }",
        "map { \"max_connections\": 0 }",
        "map { \"max_topics\": -1 }",
        "map { \"max_event_bytes\": 1.5 }",
        "map { \"max_payload_bytes\": \"100\" }",
        "map { \"max_payload_bytes\": 0 }",
        "map { \"max_payload_bytes\": 32, \"max_event_bytes\": 64 }",
    ] {
        let source = APP.replace(
            "listen(0, map {",
            &format!("listen(0, map {{ \"realtime\": {realtime},"),
        );
        let mut failed = fixture::Fixture::start(&source, &["run"], &[]);
        assert!(!failed.wait().success(), "{realtime}");
        assert!(!failed.stdout().contains("NTNT_READY"));
        assert!(failed.stderr().contains("realtime"), "{}", failed.stderr());
    }
    let source = APP.replace(
        "listen(0, map {",
        "let other = broadcast(\"setup-other\")\nlisten(0, map { \"realtime\": map { \"max_topics\": 1 },",
    );
    let mut failed = fixture::Fixture::start(&source, &["run"], &[]);
    assert!(!failed.wait().success());
    assert!(!failed.stdout().contains("NTNT_READY"));
    assert!(
        failed.stderr().contains("max_topics"),
        "{}",
        failed.stderr()
    );
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

/// SSE over HTTP/2 with prior knowledge (h2c) through the real `ntnt run`
/// listener, which multiplexes streams under explicit HTTP/2 limits.
mod h2c {
    use super::fixture;
    use axum::body::Body;
    use futures_util::{Stream, StreamExt};
    use hyper::body::{Bytes, Incoming};
    use hyper::client::conn::{http1, http2};
    use hyper::{Request, Response, StatusCode, Version};
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use std::net::SocketAddr;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::sync::mpsc;

    /// Every wait in these tests is bounded by this.
    const WAIT: Duration = Duration::from_secs(10);
    /// The HTTP/2 default stream window, advertised explicitly.
    const STREAM_WINDOW: u32 = 65_535;

    /// Whether a client connection reads its socket. Closing the gate stalls
    /// the whole connection at the transport, like an unresponsive peer.
    #[derive(Default)]
    struct Gate {
        open: AtomicBool,
        waker: Mutex<Option<Waker>>,
    }

    impl Gate {
        fn opened() -> Arc<Self> {
            let gate = Arc::new(Self::default());
            gate.set(true);
            gate
        }

        fn set(&self, open: bool) {
            self.open.store(open, Ordering::Release);
            if open {
                if let Some(waker) = self.waker.lock().unwrap().take() {
                    waker.wake();
                }
            }
        }
    }

    struct GatedIo {
        socket: tokio::net::TcpStream,
        gate: Arc<Gate>,
    }

    impl AsyncRead for GatedIo {
        fn poll_read(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if !self.gate.open.load(Ordering::Acquire) {
                *self.gate.waker.lock().unwrap() = Some(cx.waker().clone());
                // The gate may have opened before the waker was stored.
                if !self.gate.open.load(Ordering::Acquire) {
                    return Poll::Pending;
                }
            }
            Pin::new(&mut self.socket).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for GatedIo {
        fn poll_write(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            Pin::new(&mut self.socket).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.socket).poll_flush(cx)
        }

        fn poll_shutdown(
            mut self: Pin<&mut Self>,
            cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            Pin::new(&mut self.socket).poll_shutdown(cx)
        }
    }

    /// An HTTP/2 prior-knowledge connection whose streams each advertise
    /// `stream_window` bytes, released only as a stream is read. The
    /// connection window is large, so an unread stream cannot exhaust it.
    async fn h2_connect(
        addr: SocketAddr,
        stream_window: u32,
        gate: Arc<Gate>,
    ) -> http2::SendRequest<Body> {
        let (sender, connection) = tokio::time::timeout(WAIT, async {
            let socket = tokio::net::TcpStream::connect(addr).await.unwrap();
            socket.set_nodelay(true).unwrap();
            http2::Builder::new(TokioExecutor::new())
                .initial_stream_window_size(stream_window)
                .initial_connection_window_size(64 * 1024 * 1024)
                .handshake(TokioIo::new(GatedIo { socket, gate }))
                .await
                .unwrap()
        })
        .await
        .expect("no HTTP/2 connection");
        tokio::spawn(connection);
        sender
    }

    /// GET `path` as a new stream on an existing HTTP/2 connection.
    async fn h2_get(
        connection: &mut http2::SendRequest<Body>,
        addr: SocketAddr,
        path: &str,
    ) -> Response<Incoming> {
        let request = Request::get(format!("http://{addr}{path}"))
            .body(Body::empty())
            .unwrap();
        let response = tokio::time::timeout(WAIT, async {
            connection.ready().await.unwrap();
            connection.send_request(request).await.unwrap()
        })
        .await
        .expect("no HTTP/2 response");
        assert_eq!(response.version(), Version::HTTP_2);
        response
    }

    /// GET `path` on a new HTTP/1.1 connection.
    async fn h1_get(addr: SocketAddr, path: &str) -> Response<Incoming> {
        let request = Request::get(path)
            .header("host", addr.to_string())
            .body(Body::empty())
            .unwrap();
        let response = tokio::time::timeout(WAIT, async {
            let socket = tokio::net::TcpStream::connect(addr).await.unwrap();
            let (mut sender, connection) = http1::handshake(TokioIo::new(socket)).await.unwrap();
            tokio::spawn(connection);
            sender.send_request(request).await.unwrap()
        })
        .await
        .expect("no HTTP/1.1 response");
        assert_eq!(response.version(), Version::HTTP_11);
        response
    }

    async fn get_text(addr: SocketAddr, path: &str) -> String {
        let response = h1_get(addr, path).await;
        let body = tokio::time::timeout(
            WAIT,
            axum::body::to_bytes(Body::new(response.into_body()), 1024 * 1024),
        )
        .await
        .expect("no HTTP/1.1 body")
        .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    /// One SSE frame as these fixtures produce them.
    #[derive(Clone, Debug, PartialEq)]
    enum Frame {
        /// The `retry:` field that opens every stream here.
        Retry,
        Heartbeat,
        /// A published event, by `id`.
        Event(u64),
        /// The marker that ends a stream which fell behind its bus.
        Overflow,
        Other(String),
    }

    fn classify(frame: &str) -> Frame {
        if frame.starts_with("retry: ") {
            Frame::Retry
        } else if frame == ":" {
            Frame::Heartbeat
        } else if frame.starts_with("event: ntnt.overflow\n") {
            Frame::Overflow
        } else if let Some(id) = frame
            .strip_prefix("id: ")
            .and_then(|rest| rest.split('\n').next())
            .and_then(|id| id.parse().ok())
        {
            Frame::Event(id)
        } else {
            Frame::Other(frame.chars().take(80).collect())
        }
    }

    type Chunks = Pin<Box<dyn Stream<Item = Result<Bytes, axum::Error>> + Send>>;

    /// An event stream response, read frame by frame.
    struct Sse {
        chunks: Chunks,
        buffer: Vec<u8>,
    }

    impl Sse {
        fn new(response: Response<Incoming>) -> Self {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()["content-type"],
                "text/event-stream; charset=utf-8"
            );
            Self {
                chunks: Box::pin(Body::new(response.into_body()).into_data_stream()),
                buffer: Vec::new(),
            }
        }

        /// The next frame, or `None` once the stream has ended.
        async fn read(&mut self) -> Option<Frame> {
            loop {
                if let Some(end) = self.buffer.windows(2).position(|w| w == b"\n\n") {
                    let frame: Vec<u8> = self.buffer.drain(..end + 2).collect();
                    return Some(classify(std::str::from_utf8(&frame[..end]).unwrap()));
                }
                match self.chunks.next().await {
                    Some(Ok(chunk)) => self.buffer.extend_from_slice(&chunk),
                    Some(Err(error)) => panic!("stream failed: {error}"),
                    None => {
                        assert!(self.buffer.is_empty(), "stream ended inside a frame");
                        return None;
                    }
                }
            }
        }

        async fn next(&mut self) -> Option<Frame> {
            tokio::time::timeout(WAIT, self.read())
                .await
                .expect("no frame within the deadline")
        }

        /// Events up to the frame that ended them, skipping heartbeats.
        async fn events_until_end(&mut self) -> (Vec<u64>, Option<Frame>) {
            let mut events = Vec::new();
            let ending = tokio::time::timeout(WAIT, async {
                loop {
                    match self.read().await {
                        Some(Frame::Event(id)) => events.push(id),
                        Some(Frame::Heartbeat) => {}
                        other => return other,
                    }
                }
            })
            .await;
            match ending {
                Ok(ending) => (events, ending),
                Err(_) => panic!("stream did not end; it received {events:?}"),
            }
        }

        /// Keep reading in the background, so the stream never stalls.
        fn spawn(mut self) -> mpsc::UnboundedReceiver<Frame> {
            let (tx, rx) = mpsc::unbounded_channel();
            tokio::spawn(async move {
                while let Some(frame) = self.read().await {
                    if tx.send(frame).is_err() {
                        break;
                    }
                }
            });
            rx
        }
    }

    /// Wait for the event `id`, skipping heartbeats. Anything else fails.
    async fn expect_event(frames: &mut mpsc::UnboundedReceiver<Frame>, id: u64, name: &str) {
        let next = tokio::time::timeout(WAIT, async {
            loop {
                match frames.recv().await {
                    Some(Frame::Heartbeat) => {}
                    other => return other,
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("{name}: no event {id}"));
        assert_eq!(next, Some(Frame::Event(id)), "{name}");
    }

    fn start(source: &str) -> (fixture::Fixture, SocketAddr) {
        let mut server = fixture::Fixture::start(
            source,
            &["run"],
            &[("NTNT_ENV", "production"), ("NTNT_WORKERS", "2")],
        );
        let addr = server.ready();
        (server, addr)
    }

    const SHARED_APP: &str = r#"
import { text, sse } from "std/http/server"
import { broadcast, publish } from "std/concurrent"
import { repeat } from "std/string"
let bus = broadcast("demo", map { "capacity": 8 })
get("/events", fn(req) { sse(bus, map { "retry_ms": 1000, "heartbeat_ms": 1000 }) })
get("/pub", fn(req) {
    let n = int(req.query_params["n"] ?? "0") ?? 0
    let queued = publish(bus, map { "n": n, "pad": repeat("x", 8000) }, map { "id": n }) otherwise {
        return text(err)
    }
    return text("queued")
})
listen(0, map { "readiness": "json", "host": "127.0.0.1" })
"#;

    #[tokio::test]
    async fn streams_sharing_a_connection_get_events_and_a_stalled_one_holds_back_nothing() {
        let (server, addr) = start(SHARED_APP);
        // Three event streams on one HTTP/2 connection, one on HTTP/1.1.
        let mut connection = h2_connect(addr, STREAM_WINDOW, Gate::opened()).await;
        let mut first = Sse::new(h2_get(&mut connection, addr, "/events").await);
        let mut second = Sse::new(h2_get(&mut connection, addr, "/events").await);
        let mut stalled = Sse::new(h2_get(&mut connection, addr, "/events").await);
        let mut http1 = Sse::new(h1_get(addr, "/events").await);
        for stream in [&mut first, &mut second, &mut stalled, &mut http1] {
            assert_eq!(stream.next().await, Some(Frame::Retry));
        }
        // Heartbeats reach each multiplexed stream while nothing is published.
        assert_eq!(first.next().await, Some(Frame::Heartbeat));
        assert_eq!(second.next().await, Some(Frame::Heartbeat));
        // `stalled` is not read again, so its window is never released and the
        // server can send it at most STREAM_WINDOW bytes. Every other reader
        // receives event n before n + 1 is published, so none of them can
        // fall behind the 8-event bus, while the stalled stream is overtaken.
        let mut readers = [
            ("first h2 stream", first.spawn()),
            ("second h2 stream", second.spawn()),
            ("HTTP/1.1 stream", http1.spawn()),
        ];
        let total = 48;
        for n in 1..=total {
            let queued = get_text(addr, &format!("/pub?n={n}")).await;
            assert_eq!(queued, "queued", "{}", server.stderr());
            for (name, frames) in &mut readers {
                expect_event(frames, n, name).await;
            }
        }
        // The stalled stream holds only what fit its window, plus the frame
        // the server was sending and the next one it had in hand. It then
        // ends with the overflow event instead of having the rest buffered.
        let (events, ending) = stalled.events_until_end().await;
        assert_eq!(ending, Some(Frame::Overflow), "{events:?}");
        assert_eq!(stalled.next().await, None);
        let held = STREAM_WINDOW as usize / 8000 + 2;
        assert!(
            !events.is_empty() && events.len() <= held,
            "a stalled stream got {} events; at most {held} fit its window: {events:?}",
            events.len()
        );
        assert_eq!(events, (1..=events.len() as u64).collect::<Vec<_>>());
    }

    const ROOMS: usize = 8;

    const ROOMS_APP: &str = r#"
import { text, sse } from "std/http/server"
import { broadcast, publish } from "std/concurrent"
import { repeat } from "std/string"
let demo = broadcast("demo", map { "capacity": 8 })
get("/events", fn(req) { sse(demo, map { "retry_ms": 1000 }) })
get("/room/{id}", fn(req) {
    return sse(broadcast("room-" + req.params["id"], map { "capacity": 1 }), map { "retry_ms": 1000 })
})
get("/round", fn(req) {
    let n = int(req.query_params["n"] ?? "0") ?? 0
    for i in 0..ROOMS {
        let room = broadcast("room-" + str(i), map { "capacity": 1 })
        let queued = publish(room, map { "n": n, "pad": repeat("x", 16000) }, map { "id": n }) otherwise {
            return text(err)
        }
    }
    let queued = publish(demo, map { "n": n }, map { "id": n }) otherwise { return text(err) }
    return text("queued")
})
listen(0, map { "readiness": "json", "host": "127.0.0.1", "realtime": map { "max_payload_bytes": 2097152 } })
"#;

    #[tokio::test]
    async fn a_stalled_connection_keeps_each_stream_within_its_send_buffer() {
        let (server, addr) = start(&ROOMS_APP.replace("ROOMS", &ROOMS.to_string()));
        // Each stream follows its own bus, so what the server holds for one
        // stream is not shared with another and the payload budget sees the
        // sum. The client then stops reading the connection's socket. Its
        // windows are large: only the server's per-stream send buffer limits
        // what it takes from each bus once the socket buffers are full.
        let gate = Gate::opened();
        let mut connection = h2_connect(addr, 8 * 1024 * 1024, Arc::clone(&gate)).await;
        let mut stalled = Vec::new();
        for room in 0..ROOMS {
            let path = format!("/room/{room}");
            let mut stream = Sse::new(h2_get(&mut connection, addr, &path).await);
            assert_eq!(stream.next().await, Some(Frame::Retry));
            stalled.push(stream);
        }
        gate.set(false);
        // Another HTTP/2 connection and an HTTP/1.1 client keep reading.
        let mut other = h2_connect(addr, STREAM_WINDOW, Gate::opened()).await;
        let mut h2 = Sse::new(h2_get(&mut other, addr, "/events").await);
        let mut http1 = Sse::new(h1_get(addr, "/events").await);
        for stream in [&mut h2, &mut http1] {
            assert_eq!(stream.next().await, Some(Frame::Retry));
        }
        let mut readers = [
            ("other HTTP/2 connection", h2.spawn()),
            ("HTTP/1.1 stream", http1.spawn()),
        ];
        // Each round publishes a 16 KB event to every room. Once the socket
        // buffers are full, each stalled stream may hold 64 KiB plus two
        // events, under 1 MiB for all eight, so the 2 MiB budget never runs
        // out. With hyper's 400 KB default the eight hold about 3.5 MiB.
        let rounds = 128;
        for n in 1..=rounds {
            let queued = get_text(addr, &format!("/round?n={n}")).await;
            assert_eq!(
                queued,
                "queued",
                "round {n}: the stalled streams hold more than their send buffers: {}",
                server.stderr()
            );
            for (name, frames) in &mut readers {
                expect_event(frames, n, name).await;
            }
        }
        // Reading again, each stalled stream receives what was buffered for
        // it, then the overflow event: it fell behind its bus.
        gate.set(true);
        for (room, stream) in stalled.iter_mut().enumerate() {
            let (events, ending) = stream.events_until_end().await;
            assert_eq!(ending, Some(Frame::Overflow), "room {room}: {events:?}");
            assert_eq!(stream.next().await, None);
            assert!(events.len() < rounds as usize, "room {room}: {events:?}");
            assert_eq!(events, (1..=events.len() as u64).collect::<Vec<_>>());
        }
    }

    const RESET_APP: &str = r#"
import { text, sse } from "std/http/server"
import { broadcast, publish } from "std/concurrent"
import { repeat } from "std/string"
let demo = broadcast("demo")
let unheard = broadcast("unheard")
get("/events", fn(req) { sse(demo, map { "retry_ms": 1000 }) })
get("/room/{id}", fn(req) { sse(broadcast("room-" + req.params["id"]), map { "retry_ms": 1000 }) })
get("/fill/{id}", fn(req) {
    let room = broadcast("room-" + req.params["id"])
    for n in 0..100000 {
        let queued = publish(room, repeat("x", 16000)) otherwise { return text(err) }
    }
    return text("budget was not enforced")
})
// An event that needs the whole budget, on a bus nobody follows.
get("/probe", fn(req) {
    let queued = publish(unheard, repeat("x", 65536 - 10)) otherwise { return text(err) }
    return text("budget free")
})
get("/topic/{id}", fn(req) {
    let created = broadcast("probe-" + req.params["id"]) otherwise { return text(err) }
    return text("created")
})
get("/pub", fn(req) {
    let queued = publish(demo, "after", map { "id": 1 }) otherwise { return text(err) }
    return text("queued")
})
listen(0, map { "readiness": "json", "host": "127.0.0.1", "realtime": map {
    "max_connections": 2, "max_topics": 3, "max_event_bytes": 65536, "max_payload_bytes": 65536
} })
"#;

    /// Repeat `attempt` until it returns true, within the deadline.
    async fn eventually(what: &str, mut attempt: impl AsyncFnMut() -> bool) {
        let deadline = Instant::now() + WAIT;
        while !attempt().await {
            assert!(Instant::now() < deadline, "{what}");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn resetting_a_stream_releases_its_permit_subscription_and_buffered_events() {
        let (server, addr) = start(RESET_APP);
        let mut connection = h2_connect(addr, STREAM_WINDOW, Gate::opened()).await;
        let mut sibling = Sse::new(h2_get(&mut connection, addr, "/events").await);
        assert_eq!(sibling.next().await, Some(Frame::Retry));
        let mut sibling = sibling.spawn();
        // The second and last permitted stream follows a bus its own request
        // created, taking the last topic, and is never read.
        let mut reset = Sse::new(h2_get(&mut connection, addr, "/room/r").await);
        assert_eq!(reset.next().await, Some(Frame::Retry));
        // Once its window is full the server holds that stream's next events
        // and the bus keeps those it has not read, until the budget runs out.
        // Events hyper has not yet written can still fit the window, which
        // frees their budget, so fill again until the budget stays exhausted.
        let budget = "realtime payload budget exhausted";
        eventually("the stalled stream never held the budget", async || {
            get_text(addr, "/fill/r").await == budget && get_text(addr, "/probe").await == budget
        })
        .await;
        // Both stream permits and every topic are in use.
        let refused = h2_get(&mut connection, addr, "/events").await;
        assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
        let refused = h1_get(addr, "/events").await;
        assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
        let topic = get_text(addr, "/topic/x").await;
        assert!(
            topic.contains("too many broadcast names (limit 3)"),
            "{topic}"
        );

        // Reset that stream only: its connection stays open.
        drop(reset);
        eventually(
            "the reset stream kept events charged to the budget",
            async || get_text(addr, "/probe").await == "budget free",
        )
        .await;
        eventually("the reset stream kept its bus alive", async || {
            get_text(addr, "/topic/x").await == "created"
        })
        .await;
        let mut admitted = None;
        eventually("the reset stream kept its permit", async || {
            let response = h2_get(&mut connection, addr, "/events").await;
            let ok = response.status() == StatusCode::OK;
            if ok {
                admitted = Some(Sse::new(response));
            }
            ok
        })
        .await;
        let mut admitted = admitted.unwrap();
        assert_eq!(admitted.next().await, Some(Frame::Retry));
        // The sibling on the same connection was not disturbed.
        assert_eq!(
            get_text(addr, "/pub").await,
            "queued",
            "{}",
            server.stderr()
        );
        expect_event(&mut sibling, 1, "sibling").await;
        assert_eq!(admitted.next().await, Some(Frame::Event(1)));
    }
}
