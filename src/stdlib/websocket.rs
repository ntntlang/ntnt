//! Native WebSocket transport (DD-041 Slice B).
//!
//! The async server takes the HTTP upgrade before the request is converted
//! for the interpreter, then runs the normal pipeline. The connection is
//! upgraded only when the final response is successful and carries a
//! [`WsSpec`]; any other response (auth denial, redirect, error) is sent as
//! ordinary HTTP and the held upgrade is dropped.
//!
//! After the 101 response one task owns the socket: reads, writes, ping/pong,
//! close and shutdown. Senders hold weak [`WsHandle`]s with bounded queues, so
//! no caller can block on a socket or keep a closed connection alive. The
//! connection permit is released only when that task ends.
//!
//! No NTNT API produces a `WsSpec` yet. The internal echo handler exercises
//! the transport until endpoint registration lands.

use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::StreamExt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use crate::stdlib::realtime::{PayloadCharge, StreamPermit};

/// Most messages queued for one connection.
pub const OUTBOX_MESSAGES: usize = 16;
/// Most payload bytes queued for one connection.
pub const OUTBOX_BYTES: usize = 256 * 1024;

/// Live connection owners, including admitted upgrades not yet completed.
/// Server exit waits for these so close frames are sent before the runtime
/// is dropped.
static OWNERS: AtomicUsize = AtomicUsize::new(0);
static OWNERS_IDLE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Held by one admitted connection until its owner task ends.
pub(crate) struct OwnerGuard(());

impl OwnerGuard {
    pub(crate) fn new() -> Self {
        OWNERS.fetch_add(1, Ordering::AcqRel);
        Self(())
    }
}

impl Drop for OwnerGuard {
    fn drop(&mut self) {
        if OWNERS.fetch_sub(1, Ordering::AcqRel) == 1 {
            OWNERS_IDLE.notify_waiters();
        }
    }
}

/// Wait until every connection owner has finished. Callers bound this.
pub(crate) async fn wait_for_owners() {
    loop {
        let idle = OWNERS_IDLE.notified();
        tokio::pin!(idle);
        idle.as_mut().enable();
        if OWNERS.load(Ordering::Acquire) == 0 {
            return;
        }
        idle.await;
    }
}

/// Largest message one connection can queue; inbound messages above it are
/// refused by the codec with 1009 instead of failing later as overload.
pub(crate) fn effective_max_message(max_event_bytes: usize) -> usize {
    max_event_bytes.min(OUTBOX_BYTES)
}

/// Close codes used by the runtime (RFC 6455 §7.4.1).
pub mod close_code {
    pub const NORMAL: u16 = 1000;
    pub const GOING_AWAY: u16 = 1001;
    pub const PROTOCOL: u16 = 1002;
    pub const INVALID_DATA: u16 = 1007;
    pub const TOO_BIG: u16 = 1009;
    pub const TRY_AGAIN_LATER: u16 = 1013;
}

/// Liveness and teardown deadlines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timings {
    /// Interval between server pings.
    pub ping_interval: Duration,
    /// Time allowed for a pong after a ping.
    pub pong_timeout: Duration,
    /// Time allowed for one write to complete.
    pub write_timeout: Duration,
    /// Time allowed for the peer to finish the close handshake.
    pub close_drain: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            ping_interval: Duration::from_secs(30),
            pong_timeout: Duration::from_secs(10),
            write_timeout: Duration::from_secs(10),
            close_drain: Duration::from_secs(5),
        }
    }
}

/// Exact-match `Origin` policy. CORS does not authorize a WebSocket handshake.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginPolicy {
    /// Allowed origins, compared exactly (e.g. `https://app.example.com`).
    pub allowed: Vec<String>,
    /// Allow requests without an `Origin` header (non-browser clients).
    pub allow_missing: bool,
}

impl OriginPolicy {
    /// `None` means the request had no `Origin` header. `"null"` is never allowed.
    pub fn allows(&self, origin: Option<&str>) -> bool {
        match origin {
            None => self.allow_missing,
            Some("null") => false,
            Some(origin) => self.allowed.iter().any(|allowed| allowed == origin),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Handler {
    /// Internal transport check: every data message is sent back.
    Echo,
}

/// A WebSocket accepted by the final response. Only a description: the
/// connection is admitted and upgraded on the async side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WsSpec {
    handler: Handler,
    origins: OriginPolicy,
    timings: Timings,
}

impl WsSpec {
    /// Internal echo endpoint used to test the transport.
    #[doc(hidden)]
    pub fn echo(origins: OriginPolicy) -> Self {
        Self {
            handler: Handler::Echo,
            origins,
            timings: Timings::default(),
        }
    }

    #[doc(hidden)]
    pub fn with_timings(mut self, timings: Timings) -> Self {
        self.timings = timings;
        self
    }

    pub fn origins(&self) -> &OriginPolicy {
        &self.origins
    }
}

/// One application message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WsData {
    Text(String),
    Binary(Vec<u8>),
}

impl WsData {
    fn len(&self) -> usize {
        match self {
            WsData::Text(text) => text.len(),
            WsData::Binary(bytes) => bytes.len(),
        }
    }

    fn into_message(self) -> Message {
        match self {
            WsData::Text(text) => Message::Text(text),
            WsData::Binary(bytes) => Message::Binary(bytes),
        }
    }
}

/// Why a message was not queued. Success only ever means "queued".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The connection has ended.
    Closed,
    /// The connection's queue is full.
    QueueFull,
    /// The message exceeds the per-message limit.
    TooLarge,
    /// The process-wide realtime payload budget is exhausted.
    OverBudget,
}

/// Bytes charged to one connection's queue and to the process payload
/// budget. Released when the message is written or the queue is dropped.
struct Charge {
    queued: Arc<AtomicUsize>,
    len: usize,
    _payload: PayloadCharge,
}

impl Drop for Charge {
    fn drop(&mut self) {
        self.queued.fetch_sub(self.len, Ordering::AcqRel);
    }
}

struct Queued {
    data: WsData,
    _charge: Charge,
}

/// Weak send authority for one connection. It never keeps the connection,
/// its queue or its socket alive, and it never waits.
#[derive(Clone)]
pub struct WsHandle {
    tx: mpsc::WeakSender<Queued>,
    queued: Weak<AtomicUsize>,
    max_message: usize,
}

impl WsHandle {
    pub fn send(&self, data: WsData) -> Result<(), SendError> {
        let len = data.len();
        if len > self.max_message {
            return Err(SendError::TooLarge);
        }
        let tx = self.tx.upgrade().ok_or(SendError::Closed)?;
        let queued = self.queued.upgrade().ok_or(SendError::Closed)?;
        let payload = crate::stdlib::realtime::charge_payload(len).ok_or(SendError::OverBudget)?;
        let mut used = queued.load(Ordering::Acquire);
        loop {
            let Some(next) = used.checked_add(len).filter(|n| *n <= OUTBOX_BYTES) else {
                return Err(SendError::QueueFull);
            };
            match queued.compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break,
                Err(actual) => used = actual,
            }
        }
        let message = Queued {
            data,
            _charge: Charge {
                queued,
                len,
                _payload: payload,
            },
        };
        // A rejected message is dropped here, releasing its charge.
        tx.try_send(message).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => SendError::QueueFull,
            mpsc::error::TrySendError::Closed(_) => SendError::Closed,
        })
    }
}

/// How the connection loop ended.
enum End {
    /// Send this close frame, then wait briefly for the peer's reply.
    Close(u16, &'static str),
    /// The peer started the close handshake; flush the automatic reply.
    PeerClosed,
    /// The transport is gone or unresponsive; drop it.
    Drop,
}

fn error_end(error: axum::Error) -> End {
    use tokio_tungstenite::tungstenite::Error;
    match error.into_inner().downcast::<Error>().map(|error| *error) {
        Ok(Error::Capacity(_)) => End::Close(close_code::TOO_BIG, "message too large"),
        Ok(Error::Utf8) => End::Close(close_code::INVALID_DATA, "invalid UTF-8"),
        Ok(Error::Protocol(_)) => End::Close(close_code::PROTOCOL, "protocol error"),
        _ => End::Drop,
    }
}

async fn write(socket: &mut WebSocket, message: Message, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, socket.send(message)).await,
        Ok(Ok(()))
    )
}

fn dispatch(handler: Handler, handle: &WsHandle, data: WsData) -> Result<(), End> {
    match handler {
        Handler::Echo => handle
            .send(data)
            .map_err(|_| End::Close(close_code::TRY_AGAIN_LATER, "server is overloaded")),
    }
}

/// Own one upgraded connection until it ends. The permit is released last.
pub(crate) async fn run(
    mut socket: WebSocket,
    spec: WsSpec,
    permit: StreamPermit,
    owner: OwnerGuard,
    mut shutdown: watch::Receiver<bool>,
    max_message: usize,
) {
    let timings = spec.timings;
    let (tx, mut rx) = mpsc::channel::<Queued>(OUTBOX_MESSAGES);
    let queued = Arc::new(AtomicUsize::new(0));
    let handle = WsHandle {
        tx: tx.downgrade(),
        queued: Arc::downgrade(&queued),
        max_message,
    };
    let mut ping = tokio::time::interval_at(
        Instant::now() + timings.ping_interval,
        timings.ping_interval,
    );
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pong_deadline: Option<Instant> = None;

    let end = loop {
        let pong_wait = pong_deadline;
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break End::Close(close_code::GOING_AWAY, "server shutting down");
                }
            }
            _ = tokio::time::sleep_until(pong_wait.unwrap_or_else(Instant::now)), if pong_wait.is_some() => {
                break End::Drop;
            }
            // Write queued messages before reading more, so one connection's
            // echo cannot outrun its own socket.
            Some(out) = rx.recv() => {
                let Queued { data, _charge } = out;
                if !write(&mut socket, data.into_message(), timings.write_timeout).await {
                    break End::Drop;
                }
            }
            _ = ping.tick() => {
                if pong_deadline.is_none() {
                    pong_deadline = Some(Instant::now() + timings.pong_timeout);
                }
                if !write(&mut socket, Message::Ping(Vec::new()), timings.write_timeout).await {
                    break End::Drop;
                }
            }
            incoming = socket.next() => match incoming {
                None => break End::Drop,
                Some(Ok(Message::Text(text))) => {
                    if let Err(end) = dispatch(spec.handler, &handle, WsData::Text(text)) {
                        break end;
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if let Err(end) = dispatch(spec.handler, &handle, WsData::Binary(bytes)) {
                        break end;
                    }
                }
                // The protocol library queues the pong reply itself.
                Some(Ok(Message::Ping(_))) => {}
                Some(Ok(Message::Pong(_))) => pong_deadline = None,
                Some(Ok(Message::Close(_))) => break End::PeerClosed,
                Some(Err(error)) => break error_end(error),
            },
        }
    };

    // Stop accepting new messages before the close handshake.
    drop(handle);
    drop(tx);
    rx.close();
    while rx.try_recv().is_ok() {}

    match end {
        End::Close(code, reason) => {
            let frame = Message::Close(Some(CloseFrame {
                code,
                reason: reason.into(),
            }));
            if write(&mut socket, frame, timings.write_timeout).await {
                drain(&mut socket, timings.close_drain).await;
            }
        }
        End::PeerClosed => drain(&mut socket, timings.close_drain).await,
        End::Drop => {}
    }
    drop(socket);
    drop(rx);
    drop(permit);
    drop(owner);
}

/// Poll until the peer finishes closing, which also flushes an automatic
/// close reply, bounded by `limit`.
async fn drain(socket: &mut WebSocket, limit: Duration) {
    let _ = tokio::time::timeout(limit, async {
        while let Some(Ok(_)) = socket.next().await {}
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_policy_is_exact_and_rejects_null() {
        let policy = OriginPolicy {
            allowed: vec!["https://app.example.com".into()],
            allow_missing: false,
        };
        assert!(policy.allows(Some("https://app.example.com")));
        assert!(!policy.allows(Some("https://app.example.com.evil.test")));
        assert!(!policy.allows(Some("http://app.example.com")));
        assert!(!policy.allows(Some("null")));
        assert!(!policy.allows(None));
        let open = OriginPolicy {
            allow_missing: true,
            ..policy
        };
        assert!(open.allows(None));
        assert!(!open.allows(Some("null")));
    }

    #[tokio::test]
    async fn owner_wait_returns_when_the_last_owner_ends() {
        let guard = OwnerGuard::new();
        let waiter = tokio::spawn(wait_for_owners());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiter.is_finished(), "returned with an owner still live");
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("owner wait did not wake")
            .unwrap();
    }

    #[test]
    fn effective_message_limit_never_exceeds_the_outbox() {
        assert_eq!(effective_max_message(1024), 1024);
        assert_eq!(effective_max_message(32 * 1024 * 1024), OUTBOX_BYTES);
    }

    #[test]
    fn failed_payload_charge_leaves_the_budget_unchanged() {
        let before = crate::stdlib::realtime::stats().1;
        assert!(crate::stdlib::realtime::charge_payload(usize::MAX).is_none());
        assert_eq!(crate::stdlib::realtime::stats().1, before);
    }

    #[tokio::test]
    async fn handle_bounds_queue_and_fails_closed_after_owner_drops() {
        let (tx, mut rx) = mpsc::channel::<Queued>(OUTBOX_MESSAGES);
        let queued = Arc::new(AtomicUsize::new(0));
        let handle = WsHandle {
            tx: tx.downgrade(),
            queued: Arc::downgrade(&queued),
            max_message: 64 * 1024,
        };
        assert_eq!(
            handle.send(WsData::Binary(vec![0; 64 * 1024 + 1])),
            Err(SendError::TooLarge)
        );
        // Byte bound: four 64 KiB messages fill 256 KiB.
        for _ in 0..4 {
            handle.send(WsData::Binary(vec![0; 64 * 1024])).unwrap();
        }
        assert_eq!(
            handle.send(WsData::Text("x".into())),
            Err(SendError::QueueFull)
        );
        assert_eq!(queued.load(Ordering::Acquire), OUTBOX_BYTES);
        // Writing a message releases its bytes.
        drop(rx.recv().await.unwrap());
        assert_eq!(queued.load(Ordering::Acquire), OUTBOX_BYTES - 64 * 1024);
        // Count bound.
        while rx.try_recv().is_ok() {}
        for _ in 0..OUTBOX_MESSAGES {
            handle.send(WsData::Text("x".into())).unwrap();
        }
        assert_eq!(
            handle.send(WsData::Text("x".into())),
            Err(SendError::QueueFull)
        );
        // The owner ending invalidates every handle and releases queued bytes.
        let alias = handle.clone();
        drop(tx);
        drop(rx);
        drop(queued);
        assert_eq!(alias.send(WsData::Text("x".into())), Err(SendError::Closed));
    }
}
