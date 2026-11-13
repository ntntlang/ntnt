//! Native WebSocket transport and application endpoints (DD-041 Slice B).
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
//! Applications register endpoints during setup with `ws_endpoint`; the
//! callback functions stay in each worker's interpreter. The owner sends one
//! callback job at a time per connection to the shared worker pool, carrying
//! explicit JSON state and a weak send handle, and commits the returned state.
//! An internal echo handler tests the transport without an interpreter.

use axum::extract::ws::{CloseFrame, Message};
use futures_util::StreamExt;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, watch, Notify, Semaphore};
use tokio::time::Instant;

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use crate::stdlib::realtime::{PayloadCharge, StreamPermit};

/// Most messages queued for one connection.
pub const OUTBOX_MESSAGES: usize = 16;
/// Most payload bytes queued for one connection.
pub const OUTBOX_BYTES: usize = 256 * 1024;
/// Most messages waiting for a callback on one connection.
pub const INBOX_MESSAGES: usize = 4;
/// Most bytes waiting for a callback on one connection.
pub const INBOX_BYTES: usize = 256 * 1024;
/// Largest encoded per-connection state.
pub const MAX_STATE_BYTES: usize = 16 * 1024;
/// Callback jobs admitted (queued or running) across the process.
pub const MAX_CALLBACK_JOBS: usize = 64;
/// Default longest a connection may stay open (`max_lifetime_ms` overrides
/// it per endpoint, never unlimited). The client then reconnects through the
/// normal HTTP and auth pipeline.
pub const DEFAULT_LIFETIME: Duration = Duration::from_secs(300);
/// Allowed `max_lifetime_ms` range.
pub const LIFETIME_RANGE_MS: std::ops::RangeInclusive<i64> = 1_000..=86_400_000;

lazy_static::lazy_static! {
    /// Bumped by each successful development reload; open connections close.
    static ref CODE_GENERATION: watch::Sender<u64> = watch::channel(0).0;
}

/// Close every open WebSocket because the application code was reloaded.
/// Returns the new generation, which the reloading interpreter now runs.
pub(crate) fn code_reloaded() -> u64 {
    let mut reloaded = 0;
    CODE_GENERATION.send_modify(|generation| {
        *generation += 1;
        reloaded = *generation;
    });
    reloaded
}

/// The current code generation. A connection keeps the generation its
/// handshake ran under; its callbacks never run under another one.
pub(crate) fn code_generation() -> u64 {
    *CODE_GENERATION.borrow()
}

/// Default time a connection waits for one callback before closing
/// (`callback_timeout_ms` overrides it per endpoint).
pub const CALLBACK_DEADLINE: Duration = Duration::from_secs(30);

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
    /// Reported to `on_close` when the connection ended without a close frame.
    pub const ABNORMAL: u16 = 1006;
    pub const INVALID_DATA: u16 = 1007;
    pub const POLICY: u16 = 1008;
    pub const TOO_BIG: u16 = 1009;
    pub const INTERNAL: u16 = 1011;
    /// Development reload: the code serving this connection changed.
    pub const SERVICE_RESTART: u16 = 1012;
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
    /// Development default: any `http(s)://localhost|127.0.0.1|[::1]` port.
    pub allow_loopback: bool,
}

impl OriginPolicy {
    /// `None` means the request had no `Origin` header. `"null"` is never allowed.
    pub fn allows(&self, origin: Option<&str>) -> bool {
        match origin {
            None => self.allow_missing,
            Some("null") => false,
            Some(origin) => {
                let origin = origin.to_ascii_lowercase();
                self.allowed.contains(&origin)
                    || (self.allow_loopback && is_loopback_origin(&origin))
            }
        }
    }

    /// Policy from endpoint options, else `SITE_URL`, else (development
    /// only) loopback. Never derived from request Host or forwarded headers.
    pub(crate) fn configure(
        origins: Option<Vec<String>>,
        allow_missing: bool,
        site_url: Option<&str>,
        production: bool,
    ) -> Result<Self> {
        let allowed = match origins {
            Some(list) => list
                .iter()
                .map(|origin| canonical_origin(origin))
                .collect::<Result<Vec<_>>>()?,
            None => match site_url.filter(|url| !url.trim().is_empty()) {
                Some(url) => vec![canonical_origin(site_origin(url))?],
                None if production => {
                    return Err(IntentError::runtime_error(
                        "ws_endpoint needs trusted origins in production: set SITE_URL or pass the `origins` option",
                    ))
                }
                None => {
                    return Ok(Self {
                        allowed: Vec::new(),
                        allow_missing,
                        allow_loopback: true,
                    })
                }
            },
        };
        Ok(Self {
            allowed,
            allow_missing,
            allow_loopback: false,
        })
    }
}

/// `https://app.example.com/path` -> `https://app.example.com`.
fn site_origin(url: &str) -> &str {
    let url = url.trim();
    match url.find("://") {
        Some(i) => {
            let rest = &url[i + 3..];
            let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
            &url[..i + 3 + end]
        }
        None => url,
    }
}

/// Validate and lowercase an origin: `scheme://host[:port]`, http(s) only.
fn canonical_origin(origin: &str) -> Result<String> {
    let invalid = || {
        IntentError::type_error(format!(
            "invalid WebSocket origin {origin:?}: expected scheme://host[:port], e.g. https://app.example.com"
        ))
    };
    let origin = origin.trim();
    let (scheme, rest) = origin.split_once("://").ok_or_else(invalid)?;
    let scheme = scheme.to_ascii_lowercase();
    if !(scheme == "http" || scheme == "https")
        || rest.is_empty()
        || rest.contains(['/', '?', '#', '@', ' '])
    {
        return Err(invalid());
    }
    let rest = rest.to_ascii_lowercase();
    // Split off a port after the host (an IPv6 host is bracketed).
    let host_end = if rest.starts_with('[') {
        rest.find(']').map(|i| i + 1).ok_or_else(invalid)?
    } else {
        rest.find(':').unwrap_or(rest.len())
    };
    let (host, port) = rest.split_at(host_end);
    if host.is_empty() || host == "[]" {
        return Err(invalid());
    }
    match port {
        "" => Ok(format!("{scheme}://{host}")),
        port => {
            let number = port
                .strip_prefix(':')
                .filter(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|p| p.parse::<u16>().ok())
                .ok_or_else(invalid)?;
            // Browsers omit the default port from Origin; so must we.
            let default = if scheme == "https" { 443 } else { 80 };
            if number == default {
                Ok(format!("{scheme}://{host}"))
            } else {
                Ok(format!("{scheme}://{host}:{number}"))
            }
        }
    }
}

fn is_loopback_origin(origin: &str) -> bool {
    let Some((scheme, rest)) = origin.split_once("://") else {
        return false;
    };
    if !(scheme == "http" || scheme == "https") {
        return false;
    }
    let (host, port) = if let Some(v6) = rest.strip_prefix('[') {
        match v6.split_once(']') {
            Some((host, port)) => (host, port),
            None => return false,
        }
    } else {
        match rest.split_once(':') {
            Some((host, port)) => (host, &rest[host.len()..][..port.len() + 1]),
            None => (rest, ""),
        }
    };
    let port_ok = port.is_empty()
        || port
            .strip_prefix(':')
            .is_some_and(|p| !p.is_empty() && p.parse::<u16>().is_ok());
    port_ok && matches!(host, "localhost" | "127.0.0.1" | "::1")
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Handler {
    /// Internal transport check: every data message is sent back.
    Echo,
    /// An application endpoint registered with `ws_endpoint`.
    Endpoint(String),
}

/// Setup-time endpoint configuration, identical in every worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EndpointConfig {
    pub origins: OriginPolicy,
    pub has_open: bool,
    pub has_close: bool,
    /// How long the connection waits for one callback.
    pub callback_deadline: Duration,
    /// Longest the connection may stay open.
    pub lifetime: Duration,
}

lazy_static::lazy_static! {
    static ref ENDPOINTS: Mutex<HashMap<String, EndpointConfig>> = Mutex::new(HashMap::new());
    static ref ADMISSION: Arc<Semaphore> = Arc::new(Semaphore::new(MAX_CALLBACK_JOBS));
    static ref CALLBACKS: Mutex<Option<CallbackSender>> = Mutex::new(None);
}

/// Register an endpoint's configuration. Every worker registers the same
/// name; a different configuration is a startup error. `replace` is for
/// development hot reload, which re-evaluates setup with new code.
pub(crate) fn register_endpoint(name: &str, config: EndpointConfig, replace: bool) -> Result<()> {
    let mut endpoints = ENDPOINTS.lock().unwrap_or_else(|e| e.into_inner());
    match endpoints.get(name) {
        Some(existing) if *existing != config && !replace => Err(IntentError::runtime_error(
            format!("ws_endpoint {name:?} is already registered with a different configuration"),
        )),
        _ => {
            endpoints.insert(name.to_string(), config);
            Ok(())
        }
    }
}

/// Whether any endpoint is registered; such apps use the realtime startup path.
pub(crate) fn has_endpoints() -> bool {
    !ENDPOINTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .is_empty()
}

fn endpoint_config(name: &str) -> Option<EndpointConfig> {
    ENDPOINTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(name)
        .cloned()
}

lazy_static::lazy_static! {
    /// Callback slots as tokens. A worker may take or run a callback only
    /// while it holds one. Returning a token wakes a worker waiting for one,
    /// so released capacity is never missed by an idle worker.
    static ref SLOT_TOKENS: (flume::Sender<()>, flume::Receiver<()>) = flume::unbounded();
}

/// With more than one worker, one is always left for HTTP so callbacks
/// cannot occupy every worker.
pub(crate) fn set_callback_workers(workers: usize) {
    let (tx, rx) = &*SLOT_TOKENS;
    while rx.try_recv().is_ok() {}
    for _ in 0..workers.saturating_sub(1).max(1) {
        let _ = tx.send(());
    }
}

/// Held by a worker while it may take or run a callback; dropping it
/// returns the token.
pub(crate) struct CallbackSlot(());

impl Drop for CallbackSlot {
    fn drop(&mut self) {
        let _ = SLOT_TOKENS.0.send(());
    }
}

pub(crate) fn try_callback_slot() -> Option<CallbackSlot> {
    SLOT_TOKENS.1.try_recv().ok().map(|()| CallbackSlot(()))
}

/// Receiver workers select on to wait for a slot.
pub(crate) fn slot_tokens() -> &'static flume::Receiver<()> {
    &SLOT_TOKENS.1
}

/// Wrap a token already received from [`slot_tokens`].
pub(crate) fn claimed_slot() -> CallbackSlot {
    CallbackSlot(())
}

/// Install the queue that connection owners send callback jobs to.
pub(crate) fn install_callbacks(tx: CallbackSender) {
    *CALLBACKS.lock().unwrap_or_else(|e| e.into_inner()) = Some(tx);
}

fn callbacks() -> Option<CallbackSender> {
    CALLBACKS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

/// A WebSocket accepted by the final response. Only a description: the
/// connection is admitted and upgraded on the async side.
#[derive(Clone, Debug)]
pub struct WsSpec {
    handler: Handler,
    origins: OriginPolicy,
    timings: Timings,
    /// Initial state, encoded JSON.
    state: String,
    has_open: bool,
    has_close: bool,
    callback_deadline: Duration,
    lifetime: Duration,
    /// Code generation the handshake ran under, which built `state`.
    generation: u64,
    /// Broadcast forwarded to this connection as text messages.
    source: Option<Arc<crate::stdlib::realtime::Bus>>,
}

impl WsSpec {
    /// Internal echo endpoint used to test the transport.
    #[doc(hidden)]
    pub fn echo(origins: OriginPolicy) -> Self {
        Self {
            handler: Handler::Echo,
            origins,
            timings: Timings::default(),
            state: "null".into(),
            has_open: false,
            has_close: false,
            callback_deadline: CALLBACK_DEADLINE,
            lifetime: DEFAULT_LIFETIME,
            generation: code_generation(),
            source: None,
        }
    }

    /// Override the connection lifetime (internal tests).
    #[doc(hidden)]
    pub fn with_lifetime(mut self, lifetime: Duration) -> Self {
        self.lifetime = lifetime;
        self
    }

    /// Record the code generation of the interpreter whose handler built
    /// `state`.
    pub(crate) fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }

    /// Forward a broadcast to the connection (internal tests).
    #[doc(hidden)]
    pub fn with_source(mut self, bus: Arc<crate::stdlib::realtime::Bus>) -> Self {
        self.source = Some(bus);
        self
    }

    /// Callback endpoints need workers to run them; refuse the upgrade
    /// rather than accept a socket nothing will serve.
    pub(crate) fn ready(&self) -> bool {
        match self.handler {
            Handler::Echo => true,
            Handler::Endpoint(_) => callbacks().is_some(),
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

/// Callback work sent from a connection owner to the worker pool.
pub struct CallbackJob {
    pub endpoint: String,
    pub conn: WsHandle,
    /// Current state, encoded JSON.
    pub state: String,
    /// Code generation that built the state. A worker never runs a callback
    /// from another generation: after a development reload the endpoint's
    /// callbacks are the new code.
    pub generation: u64,
    pub event: CallbackEvent,
    /// Where the outcome goes; dropped unanswered if the owner has gone.
    pub reply: Reply,
    _admission: tokio::sync::OwnedSemaphorePermit,
}

/// Sends a callback's outcome. Also marks the callback finished, which a
/// reply deadline cannot do: interpreter code is not preempted.
pub struct Reply {
    tx: oneshot::Sender<CallbackOutcome>,
    running: Running,
}

impl Reply {
    /// Mark the callback finished, then deliver the outcome, so the owner
    /// never sees a reply while the callback still counts as running.
    pub fn send(self, outcome: CallbackOutcome) {
        let Reply { tx, running } = self;
        drop(running);
        let _ = tx.send(outcome);
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

/// Marks a callback as still executing until it is dropped.
struct Running(Arc<std::sync::atomic::AtomicBool>);

impl Drop for Running {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

impl CallbackJob {
    /// Skip work for a connection that has already ended.
    pub fn is_stale(&self) -> bool {
        self.reply.is_closed()
    }

    /// Put the job back on the queue for a worker that may run it. The queue
    /// has room (admission bounds the jobs in existence); if it has closed
    /// because the server is stopping, the job is dropped and its connection
    /// reports the callback as not run.
    pub(crate) fn requeue(self) {
        if let Some(tx) = callbacks() {
            let _ = tx.try_send(self);
        }
    }
}

pub enum CallbackEvent {
    Open,
    Message(WsData),
    Close { code: u16, reason: String },
}

/// `Ok(state)`: commit this encoded state. `Err`: the callback failed.
pub type CallbackOutcome = std::result::Result<String, String>;
pub type CallbackSender = flume::Sender<CallbackJob>;
pub type CallbackReceiver = flume::Receiver<CallbackJob>;

/// The callback queue shared by the async server and every worker. Bounded
/// by admission permits, so sends never wait.
pub fn callback_channel() -> (CallbackSender, CallbackReceiver) {
    flume::bounded(MAX_CALLBACK_JOBS)
}

/// Encode application state: JSON data only (no functions, handles or
/// secrets), at most `MAX_STATE_BYTES`.
pub fn encode_state(value: &Value) -> std::result::Result<String, String> {
    fn data_only(value: &Value, depth: usize) -> std::result::Result<(), String> {
        if depth > 64 {
            return Err("WebSocket state is nested too deeply".into());
        }
        match value {
            Value::Unit | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::String(_) => {
                Ok(())
            }
            Value::Array(items) => items.iter().try_for_each(|v| data_only(v, depth + 1)),
            Value::Map(map) => map.values().try_for_each(|v| data_only(v, depth + 1)),
            Value::EnumValue {
                enum_name, values, ..
            } if enum_name == "Option" => values.iter().try_for_each(|v| data_only(v, depth + 1)),
            Value::Secret(_) => Err("WebSocket state cannot contain secrets".into()),
            other => Err(format!(
                "WebSocket state must be JSON data (Map, Array, String, Int, Float, Bool, None); found {}",
                other.type_name()
            )),
        }
    }
    data_only(value, 0)?;
    let json =
        crate::stdlib::json::intent_value_to_json_reject(value).map_err(|e| e.to_string())?;
    let text = json.to_string();
    if text.len() > MAX_STATE_BYTES {
        return Err(format!(
            "WebSocket state is {} bytes; the limit is {MAX_STATE_BYTES}",
            text.len()
        ));
    }
    Ok(text)
}

pub fn decode_state(text: &str) -> Value {
    serde_json::from_str::<serde_json::Value>(text)
        .map(|json| crate::stdlib::json::json_to_intent_value(&json))
        .unwrap_or(Value::Unit)
}

/// NTNT values for the WebSocket API. Opaque: no ID or socket is exposed.
#[derive(Clone, Debug)]
pub enum WsValue {
    Endpoint(String),
    Conn(WsHandle),
}

impl WsValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            WsValue::Endpoint(_) => "WebSocketEndpoint",
            WsValue::Conn(_) => "WebSocket",
        }
    }

    pub fn same(&self, other: &WsValue) -> bool {
        match (self, other) {
            (WsValue::Endpoint(a), WsValue::Endpoint(b)) => a == b,
            (WsValue::Conn(a), WsValue::Conn(b)) => Weak::ptr_eq(&a.control, &b.control),
            _ => false,
        }
    }
}

impl std::fmt::Display for WsValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WsValue::Endpoint(name) => write!(f, "<websocket endpoint {name}>"),
            WsValue::Conn(_) => f.write_str("<websocket>"),
        }
    }
}

/// Response-envelope key carrying a `websocket()` description.
pub const WS_KEY: &str = "_ws";

/// Build a `WsSpec` from a response map. `Some(Err)` means the map asked for
/// a WebSocket but the description is invalid.
pub fn spec_from_response(
    map: &HashMap<String, Value>,
) -> Option<std::result::Result<WsSpec, String>> {
    let invalid = || Err("invalid websocket() response".to_string());
    let Value::Map(ws) = map.get(WS_KEY)? else {
        return Some(invalid());
    };
    let Some(Value::WebSocket(value)) = ws.get("endpoint") else {
        return Some(invalid());
    };
    let WsValue::Endpoint(name) = value.as_ref() else {
        return Some(invalid());
    };
    let Some(config) = endpoint_config(name) else {
        return Some(Err(format!("ws_endpoint {name:?} is not registered")));
    };
    let state = match encode_state(ws.get("state").unwrap_or(&Value::Unit)) {
        Ok(state) => state,
        Err(error) => return Some(Err(error)),
    };
    let source = match ws.get("source") {
        None | Some(Value::Unit) => None,
        Some(Value::Broadcast(bus)) => Some(Arc::clone(bus)),
        Some(_) => return Some(invalid()),
    };
    Some(Ok(WsSpec {
        handler: Handler::Endpoint(name.clone()),
        origins: config.origins,
        timings: Timings::default(),
        state,
        has_open: config.has_open,
        has_close: config.has_close,
        callback_deadline: config.callback_deadline,
        lifetime: config.lifetime,
        // The handler that built `state` has just run on this worker.
        generation: code_generation(),
        source,
    }))
}

/// `websocket(endpoint, options?)`: upgrade response builder. Opens nothing.
pub fn websocket_response(args: &[Value]) -> Result<Value> {
    let endpoint = match args.first() {
        Some(endpoint @ Value::WebSocket(value))
            if matches!(value.as_ref(), WsValue::Endpoint(_)) =>
        {
            endpoint.clone()
        }
        _ => {
            return Err(IntentError::type_error(
                "websocket() requires an endpoint from ws_endpoint() as its first argument",
            ))
        }
    };
    let mut state = Value::Unit;
    let mut source = Value::Unit;
    match args.get(1) {
        None | Some(Value::Unit) => {}
        Some(Value::Map(options)) => {
            for (key, value) in options {
                match key.as_str() {
                    "state" => {
                        encode_state(value).map_err(IntentError::type_error)?;
                        state = value.clone();
                    }
                    "source" => match value {
                        Value::Broadcast(_) => source = value.clone(),
                        _ => {
                            return Err(IntentError::type_error(
                                "websocket() source must be a Broadcast from broadcast()",
                            ))
                        }
                    },
                    _ => {
                        return Err(IntentError::type_error(format!(
                            "unknown websocket() option: {key} (supported: state, source)"
                        )))
                    }
                }
            }
        }
        Some(_) => return Err(IntentError::type_error("websocket() options must be a Map")),
    }
    let mut ws = HashMap::new();
    ws.insert("endpoint".to_string(), endpoint);
    ws.insert("state".to_string(), state);
    ws.insert("source".to_string(), source);
    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(200));
    response.insert("headers".to_string(), Value::Map(HashMap::new()));
    response.insert("body".to_string(), Value::String(String::new()));
    response.insert(WS_KEY.to_string(), Value::Map(ws));
    Ok(Value::Map(response))
}

fn conn_arg<'a>(name: &str, args: &'a [Value]) -> Result<&'a WsHandle> {
    match args.first() {
        Some(Value::WebSocket(value)) => match value.as_ref() {
            WsValue::Conn(handle) => Ok(handle),
            WsValue::Endpoint(_) => Err(IntentError::type_error(format!(
                "{name}() requires a connection (the conn callback argument), not an endpoint"
            ))),
        },
        _ => Err(IntentError::type_error(format!(
            "{name}() requires a WebSocket connection as its first argument"
        ))),
    }
}

fn send_error(error: SendError) -> Value {
    let message = match error {
        SendError::Closed => "connection closed",
        SendError::QueueFull => "send queue full",
        SendError::TooLarge => "message too large",
        SendError::OverBudget => "realtime payload budget exhausted",
    };
    Value::err(Value::String(message.into()))
}

/// `ws_send(conn, data)`: queue one text (String) or binary (Array of 0-255)
/// message. `Ok` means queued, not delivered.
pub fn ws_send_value(args: &[Value]) -> Result<Value> {
    let handle = conn_arg("ws_send", args)?;
    let data = match args.get(1) {
        Some(Value::String(text)) => WsData::Text(text.clone()),
        Some(Value::Array(items)) => {
            let mut bytes = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    Value::Int(n) if (0..=255).contains(n) => bytes.push(*n as u8),
                    _ => {
                        return Err(IntentError::type_error(
                            "ws_send() binary data must be an Array of Ints from 0 to 255",
                        ))
                    }
                }
            }
            WsData::Binary(bytes)
        }
        Some(Value::Secret(_)) => {
            return Err(IntentError::type_error(
                "ws_send() cannot send a Secret value",
            ))
        }
        _ => {
            return Err(IntentError::type_error(
                "ws_send() data must be a String (text) or Array of 0-255 Ints (binary)",
            ))
        }
    };
    Ok(match handle.send(data) {
        Ok(()) => Value::ok(Value::Unit),
        Err(error) => send_error(error),
    })
}

/// `ws_close(conn, code?, reason?)`: close after queued messages are written.
pub fn ws_close_value(args: &[Value]) -> Result<Value> {
    let handle = conn_arg("ws_close", args)?;
    let code = match args.get(1) {
        None | Some(Value::Unit) => close_code::NORMAL,
        Some(Value::Int(n)) if *n == 1000 || (3000..=4999).contains(n) => *n as u16,
        Some(_) => {
            return Err(IntentError::type_error(
                "ws_close() code must be 1000 or an application code from 3000 to 4999",
            ))
        }
    };
    let reason = match args.get(2) {
        None | Some(Value::Unit) => String::new(),
        Some(Value::String(reason)) if reason.len() <= 123 => reason.clone(),
        Some(Value::String(_)) => {
            return Err(IntentError::type_error(
                "ws_close() reason must be at most 123 bytes",
            ))
        }
        Some(_) => {
            return Err(IntentError::type_error(
                "ws_close() reason must be a String",
            ))
        }
    };
    Ok(match handle.close(code, reason) {
        Ok(()) => Value::ok(Value::Unit),
        Err(error) => send_error(error),
    })
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

/// Close requests from `ws_close`, read by the owner.
#[derive(Default)]
struct Control {
    close: Mutex<Option<(u16, String)>>,
    notify: Notify,
}

/// Weak send authority for one connection. It never keeps the connection,
/// its queue or its socket alive, and it never waits.
#[derive(Clone)]
pub struct WsHandle {
    tx: mpsc::WeakSender<Queued>,
    queued: Weak<AtomicUsize>,
    control: Weak<Control>,
    max_message: usize,
}

impl std::fmt::Debug for WsHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WsHandle")
    }
}

impl WsHandle {
    /// Ask the owner to close after queued messages are written. Idempotent;
    /// the first request wins.
    pub fn close(&self, code: u16, reason: String) -> std::result::Result<(), SendError> {
        let control = self.control.upgrade().ok_or(SendError::Closed)?;
        let mut close = control.close.lock().unwrap_or_else(|e| e.into_inner());
        if close.is_none() {
            *close = Some((code, reason));
        }
        drop(close);
        control.notify.notify_one();
        Ok(())
    }

    pub fn send(&self, data: WsData) -> std::result::Result<(), SendError> {
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
    Close(u16, String),
    /// The peer started the close handshake; flush the automatic reply.
    PeerClosed(u16, String),
    /// The transport is gone or unresponsive; drop it.
    Drop,
}

fn error_end(error: axum::Error) -> End {
    use tokio_tungstenite::tungstenite::Error;
    match error.into_inner().downcast::<Error>().map(|error| *error) {
        Ok(Error::Capacity(_)) => End::Close(close_code::TOO_BIG, "message too large".into()),
        Ok(Error::Utf8) => End::Close(close_code::INVALID_DATA, "invalid UTF-8".into()),
        Ok(Error::Protocol(_)) => End::Close(close_code::PROTOCOL, "protocol error".into()),
        _ => End::Drop,
    }
}

/// The owner works on any message stream and sink: the upgraded socket in
/// production, a stand-in in tests that need exact scheduling.
pub(crate) trait Socket:
    futures_util::Stream<Item = std::result::Result<Message, axum::Error>>
    + futures_util::Sink<Message, Error = axum::Error>
    + Unpin
    + Send
{
}

impl<S> Socket for S where
    S: futures_util::Stream<Item = std::result::Result<Message, axum::Error>>
        + futures_util::Sink<Message, Error = axum::Error>
        + Unpin
        + Send
{
}

async fn write(socket: &mut impl Socket, message: Message, timeout: Duration) -> bool {
    use futures_util::SinkExt;
    matches!(
        tokio::time::timeout(timeout, socket.send(message)).await,
        Ok(Ok(()))
    )
}

fn overloaded() -> End {
    End::Close(close_code::TRY_AGAIN_LATER, "server is overloaded".into())
}

/// Messages waiting for the connection's callback, bounded by count and bytes.
#[derive(Default)]
struct Inbox {
    items: VecDeque<WsData>,
    bytes: usize,
}

impl Inbox {
    fn push(&mut self, data: WsData) -> bool {
        let len = data.len();
        if self.items.len() >= INBOX_MESSAGES || self.bytes + len > INBOX_BYTES {
            return false;
        }
        self.bytes += len;
        self.items.push_back(data);
        true
    }

    fn pop(&mut self) -> Option<WsData> {
        let data = self.items.pop_front()?;
        self.bytes -= data.len();
        Some(data)
    }
}

/// Per-connection application state and the one callback in flight.
struct Session {
    endpoint: String,
    state: String,
    has_close: bool,
    inbox: Inbox,
    pending: Option<(oneshot::Receiver<CallbackOutcome>, Instant)>,
    /// True while the last started callback may still be executing.
    running: Arc<std::sync::atomic::AtomicBool>,
    deadline: Duration,
    /// Code generation that built `state`.
    generation: u64,
}

impl Session {
    /// Admit one callback job without waiting. `false`: the pool is full.
    fn start(&mut self, handle: &WsHandle, event: CallbackEvent) -> bool {
        let Some(tx) = callbacks() else {
            return false;
        };
        let Ok(admission) = ADMISSION.clone().try_acquire_owned() else {
            return false;
        };
        let (reply, rx) = oneshot::channel();
        self.running.store(true, Ordering::Release);
        let job = CallbackJob {
            endpoint: self.endpoint.clone(),
            conn: handle.clone(),
            state: self.state.clone(),
            generation: self.generation,
            event,
            reply: Reply {
                tx: reply,
                running: Running(Arc::clone(&self.running)),
            },
            _admission: admission,
        };
        if tx.try_send(job).is_err() {
            return false;
        }
        self.pending = Some((rx, Instant::now() + self.deadline));
        true
    }

    /// Start the next queued message if idle.
    fn advance(&mut self, handle: &WsHandle) -> std::result::Result<(), End> {
        if self.pending.is_none() {
            if let Some(data) = self.inbox.pop() {
                if !self.start(handle, CallbackEvent::Message(data)) {
                    return Err(overloaded());
                }
            }
        }
        Ok(())
    }
}

async fn wait_pending(
    pending: &mut Option<(oneshot::Receiver<CallbackOutcome>, Instant)>,
) -> Option<CallbackOutcome> {
    match pending {
        Some((rx, deadline)) => match tokio::time::timeout_at(*deadline, rx).await {
            Ok(Ok(outcome)) => Some(outcome),
            // The worker dropped the job (shutdown or a crash).
            Ok(Err(_)) => Some(Err("callback was not run".into())),
            Err(_) => None,
        },
        None => std::future::pending().await,
    }
}

/// Own one upgraded connection until it ends. The permit is released last.
pub(crate) async fn run(
    mut socket: impl Socket,
    spec: WsSpec,
    permit: StreamPermit,
    owner: OwnerGuard,
    mut shutdown: watch::Receiver<bool>,
    max_message: usize,
) {
    let timings = spec.timings;
    let (tx, mut rx) = mpsc::channel::<Queued>(OUTBOX_MESSAGES);
    let queued = Arc::new(AtomicUsize::new(0));
    let control = Arc::new(Control::default());
    let handle = WsHandle {
        tx: tx.downgrade(),
        queued: Arc::downgrade(&queued),
        control: Arc::downgrade(&control),
        max_message,
    };
    let mut session = match &spec.handler {
        Handler::Echo => None,
        Handler::Endpoint(name) => Some(Session {
            endpoint: name.clone(),
            state: spec.state.clone(),
            has_close: spec.has_close,
            inbox: Inbox::default(),
            pending: None,
            running: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            deadline: spec.callback_deadline,
            generation: spec.generation,
        }),
    };
    let mut ping = tokio::time::interval_at(
        Instant::now() + timings.ping_interval,
        timings.ping_interval,
    );
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut pong_deadline: Option<Instant> = None;
    // Finite by design: handshake authorization is not permanent, so the
    // client must come back through the HTTP pipeline.
    let expires = tokio::time::sleep(spec.lifetime);
    tokio::pin!(expires);
    let mut code_changes = CODE_GENERATION.subscribe();
    // An upgrade that completes just after a reload carries state built by
    // the old code: close it before any callback can run.
    let reloaded_since_handshake = *code_changes.borrow_and_update() != spec.generation;
    // Subscribe before the first message is read: only events published after
    // the upgrade are delivered, as with SSE. The receiver keeps the bus alive.
    let mut source = spec.source.as_ref().map(|bus| bus.subscribe());
    // A close requested by the application, applied once queued messages are written.
    let mut requested_close: Option<(u16, String)> = None;

    let end = 'run: {
        if reloaded_since_handshake {
            break 'run End::Close(close_code::SERVICE_RESTART, "server code reloaded".into());
        }
        if let Some(session) = session.as_mut() {
            if spec.has_open && !session.start(&handle, CallbackEvent::Open) {
                break 'run overloaded();
            }
        }
        loop {
            if let Some((code, reason)) = requested_close.take() {
                if rx.is_empty() {
                    break 'run End::Close(code, reason);
                }
                requested_close = Some((code, reason));
            }
            let pong_wait = pong_deadline;
            let pending = session.as_mut().map(|s| &mut s.pending);
            let has_pending = pending.as_ref().is_some_and(|p| p.is_some());
            let mut no_pending = None;
            let pending = pending.unwrap_or(&mut no_pending);
            tokio::select! {
                biased;
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break 'run End::Close(close_code::GOING_AWAY, "server shutting down".into());
                    }
                }
                Ok(()) = code_changes.changed() => {
                    break 'run End::Close(close_code::SERVICE_RESTART, "server code reloaded".into());
                }
                _ = &mut expires => {
                    break 'run End::Close(close_code::GOING_AWAY, "connection lifetime reached".into());
                }
                _ = tokio::time::sleep_until(pong_wait.unwrap_or_else(Instant::now)), if pong_wait.is_some() => {
                    break 'run End::Drop;
                }
                _ = control.notify.notified(), if requested_close.is_none() => {
                    let close = control.close.lock().unwrap_or_else(|e| e.into_inner()).clone();
                    if let Some(close) = close {
                        requested_close = Some(close);
                    }
                }
                // Write queued messages before reading more, so one connection
                // cannot outrun its own socket.
                Some(out) = rx.recv() => {
                    let Queued { data, _charge } = out;
                    if !write(&mut socket, data.into_message(), timings.write_timeout).await {
                        break 'run End::Drop;
                    }
                }
                outcome = wait_pending(pending), if has_pending => {
                    let session = session.as_mut().expect("pending implies a session");
                    session.pending = None;
                    match outcome {
                        Some(Ok(state)) => session.state = state,
                        Some(Err(error)) => {
                            eprintln!("[websocket {}] callback failed: {error}", session.endpoint);
                            break 'run End::Close(close_code::INTERNAL, "callback failed".into());
                        }
                        None => {
                            eprintln!(
                                "[websocket {}] callback exceeded {}ms",
                                session.endpoint,
                                session.deadline.as_millis()
                            );
                            break 'run End::Close(close_code::INTERNAL, "callback timed out".into());
                        }
                    }
                    if let Err(end) = session.advance(&handle) {
                        break 'run end;
                    }
                }
                _ = ping.tick() => {
                    if pong_deadline.is_none() {
                        pong_deadline = Some(Instant::now() + timings.pong_timeout);
                    }
                    if !write(&mut socket, Message::Ping(Vec::new()), timings.write_timeout).await {
                        break 'run End::Drop;
                    }
                }
                incoming = socket.next(), if requested_close.is_none() => {
                    let data = match incoming {
                        None => break 'run End::Drop,
                        Some(Ok(Message::Text(text))) => WsData::Text(text),
                        Some(Ok(Message::Binary(bytes))) => WsData::Binary(bytes),
                        // The protocol library queues the pong reply itself.
                        Some(Ok(Message::Ping(_))) => continue,
                        Some(Ok(Message::Pong(_))) => {
                            pong_deadline = None;
                            continue;
                        }
                        Some(Ok(Message::Close(frame))) => {
                            let (code, reason) = frame
                                .map(|f| (f.code, f.reason.into_owned()))
                                .unwrap_or((close_code::NORMAL, String::new()));
                            break 'run End::PeerClosed(code, reason);
                        }
                        Some(Err(error)) => break 'run error_end(error),
                    };
                    match session.as_mut() {
                        None => {
                            if handle.send(data).is_err() {
                                break 'run overloaded();
                            }
                        }
                        Some(session) => {
                            if !session.inbox.push(data) {
                                break 'run End::Close(
                                    close_code::POLICY,
                                    "too many unprocessed messages".into(),
                                );
                            }
                            if let Err(end) = session.advance(&handle) {
                                break 'run end;
                            }
                        }
                    }
                }
                // Last in the biased order: a busy broadcast must not starve
                // client input or pongs (a pong left unread would time out a
                // healthy client). Client input cannot starve this branch: it
                // is bounded by the inbox/outbox limits, which close the
                // connection when exceeded.
                // One encoding per publish; each connection writes the shared
                // JSON as a text message. A connection that falls behind the
                // bus capacity is closed rather than skipping events.
                received = async { source.as_mut().expect("guarded").recv().await }, if source.is_some() => {
                    use tokio::sync::broadcast::error::RecvError;
                    match received {
                        Ok(event) => {
                            let text = event.json().to_owned();
                            drop(event);
                            if !write(&mut socket, Message::Text(text), timings.write_timeout).await {
                                break 'run End::Drop;
                            }
                        }
                        Err(RecvError::Lagged(_)) => {
                            break 'run End::Close(
                                close_code::TRY_AGAIN_LATER,
                                "fell behind the broadcast".into(),
                            );
                        }
                        // Every handle to the bus is gone: nothing more will arrive.
                        Err(RecvError::Closed) => source = None,
                    }
                }
            }
        }
    };

    // Stop accepting new messages before the close handshake.
    drop(handle);
    drop(tx);
    rx.close();
    while rx.try_recv().is_ok() {}

    let info = match &end {
        End::Close(code, reason) | End::PeerClosed(code, reason) => (*code, reason.clone()),
        End::Drop => (close_code::ABNORMAL, String::new()),
    };
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
        End::PeerClosed(..) => drain(&mut socket, timings.close_drain).await,
        End::Drop => {}
    }
    drop(socket);
    drop(rx);
    drop(permit);

    // `on_close` is best effort after the socket is gone; it never holds the
    // connection permit and is skipped if the pool is full or shutting down.
    // It never runs alongside an earlier callback for the same connection: a
    // callback past its deadline may still be executing, so then it is skipped.
    if let Some(mut session) = session {
        if session.has_close && !*shutdown.borrow() {
            if let Some((rx, deadline)) = session.pending.take() {
                let _ = tokio::time::timeout_at(deadline, rx).await;
            }
            if session.running.load(Ordering::Acquire) {
                eprintln!(
                    "[websocket {}] on_close skipped: an earlier callback is still running",
                    session.endpoint
                );
                drop(owner);
                return;
            }
            // After a development reload the endpoint's on_close is the new
            // code; it must never receive state built by the old code.
            if session.generation != code_generation() {
                eprintln!(
                    "[websocket {}] on_close skipped: server code reloaded",
                    session.endpoint
                );
                drop(owner);
                return;
            }
            let dead = WsHandle {
                tx: mpsc::channel::<Queued>(1).0.downgrade(),
                queued: Weak::new(),
                control: Weak::new(),
                max_message,
            };
            let (code, reason) = info;
            if session.start(&dead, CallbackEvent::Close { code, reason }) {
                if let Some((rx, deadline)) = session.pending.take() {
                    let _ = tokio::time::timeout_at(deadline, rx).await;
                }
            }
        }
    }
    drop(owner);
}

/// Poll until the peer finishes closing, which also flushes an automatic
/// close reply, bounded by `limit`.
async fn drain(socket: &mut impl Socket, limit: Duration) {
    let _ = tokio::time::timeout(limit, async {
        while let Some(Ok(_)) = socket.next().await {}
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stand-in socket: messages the test sends arrive as client input, and
    /// every write completes at once and is recorded.
    struct FakeSocket {
        incoming: mpsc::UnboundedReceiver<Message>,
        outgoing: mpsc::UnboundedSender<Message>,
    }

    impl futures_util::Stream for FakeSocket {
        type Item = std::result::Result<Message, axum::Error>;
        fn poll_next(
            mut self: std::pin::Pin<&mut Self>,
            cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            self.incoming.poll_recv(cx).map(|m| m.map(Ok))
        }
    }

    impl futures_util::Sink<Message> for FakeSocket {
        type Error = axum::Error;
        fn poll_ready(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), axum::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn start_send(
            self: std::pin::Pin<&mut Self>,
            item: Message,
        ) -> std::result::Result<(), axum::Error> {
            let _ = self.outgoing.send(item);
            Ok(())
        }
        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), axum::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
        fn poll_close(
            self: std::pin::Pin<&mut Self>,
            _: &mut std::task::Context<'_>,
        ) -> std::task::Poll<std::result::Result<(), axum::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn client_input_is_served_before_a_ready_broadcast_backlog() {
        // A broadcast backlog and a client message become ready together. The
        // owner must read the client first (the same branch reads pongs), so
        // a busy broadcast cannot delay pongs into a false pong timeout.
        // Single-threaded runtime: the owner only runs when this test awaits.
        let _serial = crate::stdlib::realtime::tests::COUNTERS.lock().await;
        let mut options = HashMap::new();
        options.insert("capacity".to_string(), Value::Int(1024));
        let bus = match crate::stdlib::realtime::broadcast_value(&[
            Value::String("ws-order-test".into()),
            Value::Map(options),
        ])
        .unwrap()
        {
            Value::Broadcast(bus) => bus,
            other => panic!("{other:?}"),
        };
        let (in_tx, in_rx) = mpsc::unbounded_channel();
        let (out_tx, mut out_rx) = mpsc::unbounded_channel();
        let (permit, shutdown, max_event) = crate::stdlib::realtime::admit_connection().unwrap();
        let spec = WsSpec::echo(OriginPolicy::default()).with_source(bus.clone());
        let owner = tokio::spawn(run(
            FakeSocket {
                incoming: in_rx,
                outgoing: out_tx,
            },
            spec,
            permit,
            OwnerGuard::new(),
            shutdown,
            effective_max_message(max_event),
        ));
        // Let the owner subscribe and park with nothing ready.
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
        let value = Value::Broadcast(bus);
        for n in 0..200 {
            crate::stdlib::realtime::publish_value(&[value.clone(), Value::Int(n)]).unwrap();
        }
        in_tx.send(Message::Text("client".into())).unwrap();
        let mut position = None;
        for i in 0..=200 {
            let message = tokio::time::timeout(Duration::from_secs(2), out_rx.recv())
                .await
                .expect("owner stopped writing")
                .expect("owner ended");
            if message == Message::Text("client".into()) {
                position = Some(i);
                break;
            }
        }
        assert!(
            position.is_some_and(|p| p < 2),
            "client echo written at position {position:?}, behind the broadcast backlog"
        );
        drop(in_tx);
        owner.await.unwrap();
    }

    #[test]
    fn origin_policy_is_exact_and_rejects_null() {
        let policy = OriginPolicy {
            allowed: vec!["https://app.example.com".into()],
            allow_missing: false,
            allow_loopback: false,
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

    #[test]
    fn origin_configuration_prefers_options_then_site_url_and_fails_closed() {
        let explicit = OriginPolicy::configure(
            Some(vec!["HTTPS://App.Example.com".into()]),
            false,
            Some("https://ignored.test"),
            true,
        )
        .unwrap();
        assert!(explicit.allows(Some("https://app.example.com")));
        assert!(!explicit.allows(Some("https://ignored.test")));
        let site =
            OriginPolicy::configure(None, false, Some("https://site.test/app?x"), true).unwrap();
        assert_eq!(site.allowed, vec!["https://site.test".to_string()]);
        let default_port =
            OriginPolicy::configure(None, false, Some("https://app.example.com:443/"), true)
                .unwrap();
        assert!(default_port.allows(Some("https://app.example.com")));
        let http_default =
            OriginPolicy::configure(Some(vec!["http://x.test:80".into()]), false, None, true)
                .unwrap();
        assert!(http_default.allows(Some("http://x.test")));
        let custom =
            OriginPolicy::configure(Some(vec!["https://x.test:8443".into()]), false, None, true)
                .unwrap();
        assert!(custom.allows(Some("https://x.test:8443")));
        assert!(!custom.allows(Some("https://x.test")));
        let v6 = OriginPolicy::configure(Some(vec!["http://[::1]:80".into()]), false, None, true)
            .unwrap();
        assert!(v6.allows(Some("http://[::1]")));
        for bad in [
            "https://a.test:",
            "https://a.test:abc",
            "https://a.test:99999",
            "https://[::1",
        ] {
            assert!(
                OriginPolicy::configure(Some(vec![bad.into()]), false, None, true).is_err(),
                "{bad}"
            );
        }
        assert!(OriginPolicy::configure(None, false, None, true).is_err());
        assert!(OriginPolicy::configure(None, false, Some("  "), true).is_err());
        let dev = OriginPolicy::configure(None, false, None, false).unwrap();
        for origin in [
            "http://localhost:3000",
            "http://127.0.0.1:8080",
            "http://[::1]:9000",
            "https://localhost",
        ] {
            assert!(dev.allows(Some(origin)), "{origin}");
        }
        for origin in [
            "http://localhost.evil.test",
            "http://evil.test",
            "http://127.0.0.1.evil.test:80",
            "http://localhost:99999",
            "ftp://localhost",
        ] {
            assert!(!dev.allows(Some(origin)), "{origin}");
        }
        for bad in [
            "app.example.com",
            "https://a.test/path",
            "javascript://x",
            "https://",
        ] {
            assert!(
                OriginPolicy::configure(Some(vec![bad.into()]), false, None, false).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn state_must_be_bounded_json_data() {
        let mut map = HashMap::new();
        map.insert("n".to_string(), Value::Int(1));
        assert_eq!(encode_state(&Value::Map(map)).unwrap(), r#"{"n":1}"#);
        assert!(encode_state(&Value::String("x".repeat(MAX_STATE_BYTES))).is_err());
        let handle = Value::ScheduleHandle(1);
        assert!(encode_state(&Value::Array(vec![handle])).is_err());
        assert_eq!(
            decode_state(r#"{"n":1}"#).to_string(),
            Value::Map({
                let mut m = HashMap::new();
                m.insert("n".to_string(), Value::Int(1));
                m
            })
            .to_string()
        );
    }

    #[test]
    fn inbox_is_bounded_by_count_and_bytes() {
        let mut inbox = Inbox::default();
        for _ in 0..INBOX_MESSAGES {
            assert!(inbox.push(WsData::Text("x".into())));
        }
        assert!(!inbox.push(WsData::Text("x".into())));
        let mut big = Inbox::default();
        assert!(big.push(WsData::Binary(vec![0; INBOX_BYTES])));
        assert!(!big.push(WsData::Binary(vec![0; 1])));
        assert_eq!(big.pop().map(|d| d.len()), Some(INBOX_BYTES));
        assert!(big.push(WsData::Binary(vec![0; 1])));
    }

    #[test]
    fn endpoint_registration_rejects_conflicting_configuration() {
        let config = EndpointConfig {
            origins: OriginPolicy::default(),
            has_open: false,
            has_close: false,
            callback_deadline: CALLBACK_DEADLINE,
            lifetime: DEFAULT_LIFETIME,
        };
        register_endpoint("unit-conflict", config.clone(), false).unwrap();
        register_endpoint("unit-conflict", config.clone(), false).unwrap();
        let other = EndpointConfig {
            has_open: true,
            ..config.clone()
        };
        assert!(register_endpoint("unit-conflict", other.clone(), false).is_err());
        register_endpoint("unit-conflict", other, true).unwrap();
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
        let _serial = crate::stdlib::realtime::tests::COUNTERS.blocking_lock();
        let before = crate::stdlib::realtime::stats().1;
        assert!(crate::stdlib::realtime::charge_payload(usize::MAX).is_none());
        assert_eq!(crate::stdlib::realtime::stats().1, before);
    }

    #[tokio::test]
    async fn handle_bounds_queue_and_fails_closed_after_owner_drops() {
        // Sends charge the shared payload budget.
        let _serial = crate::stdlib::realtime::tests::COUNTERS.lock().await;
        let (tx, mut rx) = mpsc::channel::<Queued>(OUTBOX_MESSAGES);
        let queued = Arc::new(AtomicUsize::new(0));
        let control = Arc::new(Control::default());
        let handle = WsHandle {
            tx: tx.downgrade(),
            queued: Arc::downgrade(&queued),
            control: Arc::downgrade(&control),
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
