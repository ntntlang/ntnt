//! WebSocket transport through the real async server and bridge.
//!
//! A stand-in interpreter answers bridge requests so each test controls the
//! final response; everything from the socket to the bridge is production
//! code. No NTNT API opens a WebSocket yet.

use futures_util::{SinkExt, StreamExt};
use ntnt::stdlib::http_bridge::{BridgeResponse, HandlerRequest, InterpreterHandle};
use ntnt::stdlib::http_server_async::{serve_on_listener, AsyncServerConfig, AsyncServerState};
use ntnt::stdlib::realtime;
use ntnt::stdlib::websocket::{OriginPolicy, Timings, WsSpec};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

/// The connection counter is process-wide; tests that read it run one at a time.
static COUNTER: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const ORIGIN: &str = "http://app.test";

fn fast() -> Timings {
    Timings {
        ping_interval: Duration::from_millis(200),
        pong_timeout: Duration::from_millis(300),
        write_timeout: Duration::from_secs(2),
        close_drain: Duration::from_millis(500),
    }
}

fn echo() -> WsSpec {
    WsSpec::echo(OriginPolicy {
        allowed: vec![ORIGIN.into()],
        allow_missing: false,
    })
    .with_timings(fast())
}

fn response(status: u16, headers: &[(&str, &str)], ws: Option<WsSpec>) -> BridgeResponse {
    BridgeResponse {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: "http body".into(),
        sse: None,
        ws,
    }
}

/// Stand-in for the interpreter pipeline's final response.
fn final_response(path: &str) -> BridgeResponse {
    match path {
        "/ws" => response(
            200,
            &[
                ("set-cookie", "a=1; Path=/"),
                ("set-cookie", "b=2; Path=/"),
                ("x-app", "yes"),
            ],
            Some(echo()),
        ),
        // Middleware denial: no WebSocket in the response.
        "/denied" => response(401, &[("set-cookie", "session=; Max-Age=0")], None),
        // A non-success response never upgrades, even if it carries a spec.
        "/refresh" => response(
            307,
            &[("location", "/login"), ("set-cookie", "refresh=1; Path=/")],
            Some(echo()),
        ),
        // A handler tries to override handshake headers.
        "/override" => response(
            200,
            &[("sec-websocket-accept", "forged"), ("connection", "close")],
            Some(echo()),
        ),
        "/missing-origin-ok" => response(
            200,
            &[],
            Some(
                WsSpec::echo(OriginPolicy {
                    allowed: vec![],
                    allow_missing: true,
                })
                .with_timings(fast()),
            ),
        ),
        _ => response(200, &[], None),
    }
}

async fn start() -> SocketAddr {
    let (tx, rx) = flume::bounded::<HandlerRequest>(64);
    tokio::spawn(async move {
        while let Ok(request) = rx.recv_async().await {
            let _ = request.reply_tx.send(final_response(&request.request.path));
        }
    });
    let routes = Arc::new(AsyncServerState::new());
    for path in [
        "/ws",
        "/denied",
        "/refresh",
        "/override",
        "/missing-origin-ok",
        "/plain",
    ] {
        routes.register_route("GET", path, "handler").await;
        routes.register_route("POST", path, "handler").await;
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let config = AsyncServerConfig {
        enable_compression: true,
        ..AsyncServerConfig::default()
    };
    tokio::spawn(serve_on_listener(
        listener,
        config,
        Arc::new(InterpreterHandle::new(tx)),
        routes,
    ));
    addr
}

fn request(
    addr: SocketAddr,
    path: &str,
    origin: Option<&str>,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("ws://{addr}{path}").into_client_request().unwrap();
    if let Some(origin) = origin {
        request
            .headers_mut()
            .insert("origin", origin.parse().unwrap());
    }
    request
}

async fn open_count_is(expected: usize) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while realtime::stats().0 != expected {
        assert!(
            tokio::time::Instant::now() < deadline,
            "open connections {} != {expected}",
            realtime::stats().0
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn rejected_status(error: WsError) -> (u16, Vec<String>) {
    match error {
        WsError::Http(response) => (
            response.status().as_u16(),
            response
                .headers()
                .get_all("set-cookie")
                .iter()
                .map(|v| v.to_str().unwrap().to_string())
                .collect(),
        ),
        other => panic!("expected an HTTP refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn upgrade_keeps_application_headers_and_echoes_in_order() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    let payload_base = realtime::stats().1;
    let (mut socket, handshake) =
        tokio_tungstenite::connect_async(request(addr, "/ws", Some(ORIGIN)))
            .await
            .unwrap();
    assert_eq!(handshake.status().as_u16(), 101);
    let cookies: Vec<_> = handshake
        .headers()
        .get_all("set-cookie")
        .iter()
        .map(|v| v.to_str().unwrap())
        .collect();
    assert_eq!(cookies, ["a=1; Path=/", "b=2; Path=/"]);
    assert_eq!(handshake.headers()["x-app"], "yes");
    assert!(handshake.headers().get("content-encoding").is_none());
    assert_eq!(realtime::stats().0, base + 1);

    for i in 0..20 {
        socket.send(Message::Text(format!("m{i}"))).await.unwrap();
    }
    socket.send(Message::Binary(vec![0, 1, 255])).await.unwrap();
    for i in 0..20 {
        assert_eq!(
            socket.next().await.unwrap().unwrap(),
            Message::Text(format!("m{i}"))
        );
    }
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Binary(vec![0, 1, 255])
    );

    socket.close(None).await.unwrap();
    while socket.next().await.is_some() {}
    open_count_is(base).await;
    // Queued echo bytes were charged to the shared budget and all released.
    assert_eq!(realtime::stats().1, payload_base);
}

#[tokio::test]
async fn non_success_responses_never_upgrade() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    let denied = tokio_tungstenite::connect_async(request(addr, "/denied", Some(ORIGIN)))
        .await
        .unwrap_err();
    assert_eq!(
        rejected_status(denied),
        (401, vec!["session=; Max-Age=0".to_string()])
    );
    let refresh = tokio_tungstenite::connect_async(request(addr, "/refresh", Some(ORIGIN)))
        .await
        .unwrap_err();
    assert_eq!(
        rejected_status(refresh),
        (307, vec!["refresh=1; Path=/".to_string()])
    );
    // A route that does not ask for a WebSocket answers as plain HTTP.
    let plain = tokio_tungstenite::connect_async(request(addr, "/plain", Some(ORIGIN)))
        .await
        .unwrap_err();
    assert_eq!(rejected_status(plain).0, 200);
    assert_eq!(realtime::stats().0, base);
}

#[tokio::test]
async fn origin_policy_refuses_foreign_null_and_missing_origins() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    for origin in [Some("http://evil.test"), Some("null"), None] {
        let error = tokio_tungstenite::connect_async(request(addr, "/ws", origin))
            .await
            .unwrap_err();
        assert_eq!(rejected_status(error).0, 403, "origin {origin:?}");
    }
    // Explicit opt-in for clients that send no Origin.
    let (mut socket, _) =
        tokio_tungstenite::connect_async(request(addr, "/missing-origin-ok", None))
            .await
            .unwrap();
    socket.close(None).await.unwrap();
    while socket.next().await.is_some() {}
    open_count_is(base).await;
}

#[tokio::test]
async fn malformed_and_plain_requests_to_a_websocket_route_open_nothing() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    let client = reqwest::Client::new();
    // Plain GET: the response wants a WebSocket but the request is not one.
    let plain = client
        .get(format!("http://{addr}/ws"))
        .header("origin", ORIGIN)
        .send()
        .await
        .unwrap();
    assert_eq!(plain.status().as_u16(), 400);
    // Upgrade header without a key or version.
    let malformed = client
        .get(format!("http://{addr}/ws"))
        .header("origin", ORIGIN)
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .send()
        .await
        .unwrap();
    assert_eq!(malformed.status().as_u16(), 400);
    // A POST is never a WebSocket handshake.
    let post = client
        .post(format!("http://{addr}/ws"))
        .header("origin", ORIGIN)
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    assert_eq!(post.status().as_u16(), 400);
    assert_eq!(realtime::stats().0, base);
}

#[tokio::test]
async fn application_cannot_override_handshake_headers() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    let (mut socket, handshake) =
        tokio_tungstenite::connect_async(request(addr, "/override", Some(ORIGIN)))
            .await
            .expect("forged handshake headers must not break the upgrade");
    assert_ne!(handshake.headers()["sec-websocket-accept"], "forged");
    assert!(handshake.headers()["connection"]
        .to_str()
        .unwrap()
        .eq_ignore_ascii_case("upgrade"));
    socket.send(Message::Text("ok".into())).await.unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Text("ok".into())
    );
    socket.close(None).await.unwrap();
    while socket.next().await.is_some() {}
    open_count_is(base).await;
}

#[tokio::test]
async fn oversized_message_closes_with_1009_and_releases_the_connection() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    let (mut socket, _) = tokio_tungstenite::connect_async(request(addr, "/ws", Some(ORIGIN)))
        .await
        .unwrap();
    socket
        .send(Message::Binary(vec![7; 64 * 1024 + 1]))
        .await
        .unwrap();
    let mut code = None;
    while let Some(Ok(message)) = socket.next().await {
        if let Message::Close(Some(frame)) = message {
            code = Some(frame.code);
        }
    }
    assert_eq!(code, Some(CloseCode::Size));
    open_count_is(base).await;
}

#[tokio::test]
async fn server_pings_and_drops_a_peer_that_never_answers() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    // A polled client answers pings automatically and stays open.
    let (mut live, _) = tokio_tungstenite::connect_async(request(addr, "/ws", Some(ORIGIN)))
        .await
        .unwrap();
    let mut pings = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1200);
    while let Ok(Some(message)) = tokio::time::timeout_at(deadline, live.next()).await {
        if let Message::Ping(_) = message.unwrap() {
            pings += 1;
        }
    }
    assert!(pings >= 3, "expected repeated pings, saw {pings}");
    live.send(Message::Text("still open".into())).await.unwrap();
    // A client that never reads never sends a pong.
    let (silent, _) = tokio_tungstenite::connect_async(request(addr, "/ws", Some(ORIGIN)))
        .await
        .unwrap();
    open_count_is(base + 2).await;
    open_count_is(base + 1).await;
    drop(silent);
    loop {
        match live.next().await.unwrap().unwrap() {
            Message::Text(text) => {
                assert_eq!(text, "still open");
                break;
            }
            Message::Ping(_) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    drop(live);
    open_count_is(base).await;
}

#[tokio::test]
async fn abrupt_disconnect_releases_the_connection() {
    let _serial = COUNTER.lock().await;
    let addr = start().await;
    let base = realtime::stats().0;
    let (socket, _) = tokio_tungstenite::connect_async(request(addr, "/ws", Some(ORIGIN)))
        .await
        .unwrap();
    open_count_is(base + 1).await;
    drop(socket);
    open_count_is(base).await;
}
