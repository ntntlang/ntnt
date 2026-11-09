//! Process shutdown closes open WebSockets with 1001 and refuses new ones.
//! Shutdown is process-wide and irreversible, so this runs in its own binary.

use futures_util::{SinkExt, StreamExt};
use ntnt::stdlib::http_bridge::{BridgeResponse, HandlerRequest, InterpreterHandle};
use ntnt::stdlib::http_server_async::{serve_on_listener, AsyncServerConfig, AsyncServerState};
use ntnt::stdlib::realtime;
use ntnt::stdlib::websocket::{OriginPolicy, WsSpec};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

#[tokio::test]
async fn shutdown_closes_open_sockets_with_going_away_and_refuses_new_ones() {
    let (tx, rx) = flume::bounded::<HandlerRequest>(8);
    tokio::spawn(async move {
        while let Ok(request) = rx.recv_async().await {
            let _ = request.reply_tx.send(BridgeResponse {
                status: 200,
                headers: Vec::new(),
                body: String::new(),
                sse: None,
                ws: Some(WsSpec::echo(OriginPolicy {
                    allowed: vec![],
                    allow_missing: true,
                    allow_loopback: false,
                })),
            });
        }
    });
    let routes = Arc::new(AsyncServerState::new());
    routes.register_route("GET", "/ws", "handler").await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(serve_on_listener(
        listener,
        AsyncServerConfig::default(),
        Arc::new(InterpreterHandle::new(tx)),
        routes,
    ));
    let url = || format!("ws://{addr}/ws").into_client_request().unwrap();

    let (mut socket, _) = tokio_tungstenite::connect_async(url()).await.unwrap();
    socket.send(Message::Text("before".into())).await.unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        Message::Text("before".into())
    );

    realtime::shutdown();
    let mut code = None;
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(Ok(message)) = socket.next().await {
            if let Message::Close(Some(frame)) = message {
                code = Some(frame.code);
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "socket stayed open after shutdown");
    assert_eq!(code, Some(CloseCode::Away));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while realtime::stats().0 != 0 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "permit not released"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    match tokio_tungstenite::connect_async(url()).await {
        Err(WsError::Http(response)) => assert_eq!(response.status().as_u16(), 503),
        other => panic!("expected 503 after shutdown, got {other:?}"),
    }
}
