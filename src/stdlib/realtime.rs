//! Native realtime core (DD-041 Slice A): process-local broadcast buses,
//! bounded event encoding, and Server-Sent Event streams.
//!
//! Ownership model:
//! - A `Bus` is reachable from NTNT only through `Value::Broadcast(Arc<Bus>)`.
//!   The name registry holds `Weak` references, so unused buses are freed and
//!   pruned; a browser-supplied string never grants publish/subscribe authority.
//! - `publish` validates and encodes the SSE frame once into an
//!   `EncodedEvent` that owns a payload-budget permit. Subscribers share it via
//!   `Arc`, and the permit is released only when the last reference (ring slot
//!   or bytes held by the HTTP body) drops.
//! - No subscription exists before transport commitment: `sse()` only
//!   describes a stream. The receiver and a connection permit are created on
//!   the async side when the final response is converted, so middleware
//!   replacement, denial, HEAD or reply failure never leave a live subscription.
//! - Streams produce frames only when the HTTP body is polled, and end
//!   cooperatively when the process shutdown signal is raised.

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use bytes::Bytes;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

/// Maximum distinct live bus names per process.
pub const MAX_TOPICS: usize = 256;
/// Default and maximum retained events per bus (tokio may round capacity up).
pub const DEFAULT_CAPACITY: usize = 64;
pub const MAX_CAPACITY: usize = 1024;
/// Maximum encoded size of one event frame.
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
/// Maximum total bytes of live encoded events across all buses.
pub const MAX_PAYLOAD_BUDGET: usize = 32 * 1024 * 1024;
/// Maximum concurrently open SSE streams per process.
pub const MAX_CONNECTIONS: usize = 1024;
/// Default heartbeat (comment frame) interval.
pub const DEFAULT_HEARTBEAT_MS: u64 = 15_000;
const MAX_DEPTH: usize = 64;

/// Process-wide limits, fixed by the first listener. Scripts use the same
/// defaults before listen(); HTTP workers and task captures share this policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limits {
    max_topics: usize,
    max_connections: usize,
    max_event_bytes: usize,
    max_payload_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_topics: MAX_TOPICS,
            max_connections: MAX_CONNECTIONS,
            max_event_bytes: MAX_EVENT_BYTES,
            max_payload_bytes: MAX_PAYLOAD_BUDGET,
        }
    }
}

impl Limits {
    pub(crate) fn parse(value: &Value) -> Result<Self> {
        let Value::Map(map) = value else {
            return Err(IntentError::type_error("listen realtime must be a Map"));
        };
        let mut limits = Self::default();
        for (key, value) in map {
            let target = match key.as_str() {
                "max_topics" => &mut limits.max_topics,
                "max_connections" => &mut limits.max_connections,
                "max_event_bytes" => &mut limits.max_event_bytes,
                "max_payload_bytes" => &mut limits.max_payload_bytes,
                _ => {
                    return Err(IntentError::type_error(format!(
                        "unknown realtime option: {key}"
                    )))
                }
            };
            let number = match value {
                Value::Int(n) if *n > 0 => usize::try_from(*n).ok(),
                _ => None,
            };
            *target = number.ok_or_else(|| {
                IntentError::type_error(format!(
                    "realtime {key} must be a positive Int that fits this platform"
                ))
            })?;
        }
        if limits.max_event_bytes > limits.max_payload_bytes {
            return Err(IntentError::type_error(
                "realtime max_event_bytes must not exceed max_payload_bytes",
            ));
        }
        Ok(limits)
    }

    fn check_usage(self, topics: usize, payload_bytes: usize, connections: usize) -> Result<()> {
        for (name, limit, used) in [
            ("max_topics", self.max_topics, topics),
            ("max_payload_bytes", self.max_payload_bytes, payload_bytes),
            ("max_connections", self.max_connections, connections),
        ] {
            if used > limit {
                return Err(IntentError::runtime_error(format!(
                    "realtime {name} ({limit}) is below resources already in use ({used})"
                )));
            }
        }
        Ok(())
    }
}

// Operations hold a read guard until admission/publication finishes. The
// startup write guard prevents a task from admitting under the old defaults
// concurrently with a lower limit being installed. No guard crosses await.
static LIMITS: RwLock<Option<Limits>> = RwLock::new(None);

/// Setup-time registration opts an application into the realtime startup path.
pub(crate) fn has_live_buses() -> bool {
    REGISTRY
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .any(|bus| bus.strong_count() > 0)
}

pub(crate) fn configure(limits: Limits) -> Result<()> {
    let mut configured = LIMITS.write().unwrap_or_else(|e| e.into_inner());
    if let Some(current) = *configured {
        return if current == limits {
            Ok(())
        } else {
            Err(IntentError::runtime_error(
                "realtime limits are already fixed for this process",
            ))
        };
    }
    let mut registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    registry.retain(|_, weak| weak.strong_count() > 0);
    limits.check_usage(
        registry.len(),
        PAYLOAD_USED.load(Ordering::Acquire),
        OPEN_STREAMS.load(Ordering::Acquire),
    )?;
    *configured = Some(limits);
    Ok(())
}

/// A stream's receiver plus when it last delivered a frame. The receiver is
/// `None` once evicted.
struct Slot {
    rx: tokio::sync::Mutex<Option<broadcast::Receiver<Arc<EncodedEvent>>>>,
    last_progress_ms: AtomicU64,
}

fn monotonic_ms() -> u64 {
    lazy_static::lazy_static! {
        static ref START: std::time::Instant = std::time::Instant::now();
    }
    START.elapsed().as_millis() as u64
}

lazy_static::lazy_static! {
    static ref REGISTRY: Mutex<HashMap<String, Weak<Bus>>> = Mutex::new(HashMap::new());
    /// Every open stream's receiver slot, so a publisher under budget
    /// pressure can drop stalled receivers that are not being polled.
    static ref SUBSCRIBERS: Mutex<Vec<Weak<Slot>>> = Mutex::new(Vec::new());
    static ref SHUTDOWN: watch::Sender<bool> = watch::channel(false).0;
}
static PAYLOAD_USED: AtomicUsize = AtomicUsize::new(0);
static OPEN_STREAMS: AtomicUsize = AtomicUsize::new(0);

/// A named, process-local fan-out source.
pub struct Bus {
    name: String,
    capacity: usize,
    tx: broadcast::Sender<Arc<EncodedEvent>>,
}

impl std::fmt::Debug for Bus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Broadcast({})", self.name)
    }
}

impl Bus {
    pub fn name(&self) -> &str {
        &self.name
    }
    fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}

/// One immutable, encoded SSE frame plus its budget charge.
pub struct EncodedEvent {
    bytes: Vec<u8>,
}

impl Drop for EncodedEvent {
    fn drop(&mut self) {
        PAYLOAD_USED.fetch_sub(self.bytes.len(), Ordering::AcqRel);
    }
}

/// Shares an `EncodedEvent` with Hyper without copying its bytes.
struct Frame(Arc<EncodedEvent>);

impl AsRef<[u8]> for Frame {
    fn as_ref(&self) -> &[u8] {
        &self.0.bytes
    }
}

fn reserve_payload(len: usize, max_bytes: usize) -> bool {
    let mut used = PAYLOAD_USED.load(Ordering::Acquire);
    loop {
        let Some(next) = used.checked_add(len).filter(|n| *n <= max_bytes) else {
            return false;
        };
        match PAYLOAD_USED.compare_exchange_weak(used, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return true,
            Err(actual) => used = actual,
        }
    }
}

fn valid_bus_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b':'))
}

/// `broadcast(name, options?)`: get or create the named bus.
pub fn broadcast_value(args: &[Value]) -> Result<Value> {
    let name = match args.first() {
        Some(Value::String(s)) if valid_bus_name(s) => s.clone(),
        Some(Value::String(_)) => {
            return Err(IntentError::type_error(
                "broadcast() name must be 1-128 characters of [A-Za-z0-9._:-]",
            ))
        }
        _ => {
            return Err(IntentError::type_error(
                "broadcast() requires a name string",
            ))
        }
    };
    let mut requested: Option<usize> = None;
    if let Some(opts) = args.get(1) {
        let Value::Map(map) = opts else {
            return Err(IntentError::type_error("broadcast() options must be a map"));
        };
        for (key, value) in map {
            match (key.as_str(), value) {
                ("capacity", Value::Int(n)) if (1..=MAX_CAPACITY as i64).contains(n) => {
                    requested = Some(*n as usize)
                }
                ("capacity", _) => {
                    return Err(IntentError::type_error(format!(
                        "broadcast() capacity must be an Int from 1 to {MAX_CAPACITY}"
                    )))
                }
                _ => {
                    return Err(IntentError::type_error(format!(
                        "unknown broadcast() option: {key}"
                    )))
                }
            }
        }
    }
    let configuration = LIMITS.read().unwrap_or_else(|e| e.into_inner());
    let limits = configuration.unwrap_or_default();
    let mut registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(bus) = registry.get(&name).and_then(Weak::upgrade) {
        if requested.is_some_and(|c| c != bus.capacity) {
            return Err(IntentError::runtime_error(format!(
                "broadcast '{name}' already exists with capacity {}",
                bus.capacity
            )));
        }
        return Ok(Value::Broadcast(bus));
    }
    registry.retain(|_, weak| weak.strong_count() > 0);
    if registry.len() >= limits.max_topics {
        return Err(IntentError::runtime_error(format!(
            "too many broadcast names (limit {})",
            limits.max_topics
        )));
    }
    let capacity = requested.unwrap_or(DEFAULT_CAPACITY);
    let (tx, _) = broadcast::channel(capacity);
    let bus = Arc::new(Bus {
        name: name.clone(),
        capacity,
        tx,
    });
    registry.insert(name, Arc::downgrade(&bus));
    Ok(Value::Broadcast(bus))
}

/// `publish(bus, value, options?)`: validate, encode once, fan out.
/// Returns `Ok(())` even with zero subscribers; validation still runs.
pub fn publish_value(args: &[Value]) -> Result<Value> {
    let Some(Value::Broadcast(bus)) = args.first() else {
        return Err(IntentError::type_error(
            "publish() requires a Broadcast from broadcast() as its first argument",
        ));
    };
    let Some(value) = args.get(1) else {
        return Err(IntentError::type_error("publish() requires a value"));
    };
    let (event, id) = match args.get(2) {
        None => (None, None),
        Some(Value::Map(map)) => event_metadata(map)?,
        Some(_) => return Err(IntentError::type_error("publish() options must be a map")),
    };
    let configuration = LIMITS.read().unwrap_or_else(|e| e.into_inner());
    let limits = configuration.unwrap_or_default();
    match encode_frame_with_limit(
        value,
        event.as_deref(),
        id.as_deref(),
        limits.max_event_bytes,
    ) {
        Ok(frame) => {
            let len = frame.len();
            let fits = || {
                PAYLOAD_USED
                    .load(Ordering::Acquire)
                    .checked_add(len)
                    .is_some_and(|n| n <= limits.max_payload_bytes)
            };
            if !reserve_payload(len, limits.max_payload_bytes)
                && !(evict_stalled(fits) > 0 && reserve_payload(len, limits.max_payload_bytes))
            {
                return Ok(Value::err(Value::String(
                    "realtime payload budget exhausted".into(),
                )));
            }
            let event = Arc::new(EncodedEvent { bytes: frame });
            // No subscribers is a successful no-op, not replay storage.
            let _ = bus.tx.send(event);
            Ok(Value::ok(Value::Unit))
        }
        Err(message) => Ok(Value::err(Value::String(message))),
    }
}

fn event_metadata(map: &HashMap<String, Value>) -> Result<(Option<String>, Option<String>)> {
    let mut event = None;
    let mut id = None;
    for (key, value) in map {
        match (key.as_str(), value) {
            ("event", Value::String(s)) => event = Some(s.clone()),
            ("id", Value::String(s)) => id = Some(s.clone()),
            ("id", Value::Int(n)) => id = Some(n.to_string()),
            ("event" | "id", _) => {
                return Err(IntentError::type_error(format!(
                    "publish() option '{key}' must be a String"
                )))
            }
            _ => {
                return Err(IntentError::type_error(format!(
                    "unknown publish() option: {key}"
                )))
            }
        }
    }
    Ok((event, id))
}

/// Encode one SSE frame: optional `event:`/`id:` lines and one `data:` line of
/// compact JSON. Size is bounded while encoding.
#[cfg(test)]
fn encode_frame(
    value: &Value,
    event: Option<&str>,
    id: Option<&str>,
) -> std::result::Result<Vec<u8>, String> {
    encode_frame_with_limit(value, event, id, MAX_EVENT_BYTES)
}

fn encode_frame_with_limit(
    value: &Value,
    event: Option<&str>,
    id: Option<&str>,
    max_bytes: usize,
) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(256.min(max_bytes));
    if let Some(event) = event {
        if event.is_empty() || event.len() > 128 || event.contains(['\r', '\n']) {
            return Err("event name must be 1-128 characters without line breaks".into());
        }
        out.extend_from_slice(b"event: ");
        out.extend_from_slice(event.as_bytes());
        out.push(b'\n');
    }
    if let Some(id) = id {
        if id.len() > 256 || id.contains(['\r', '\n', '\0']) {
            return Err(
                "event id must be at most 256 characters without line breaks or NUL".into(),
            );
        }
        out.extend_from_slice(b"id: ");
        out.extend_from_slice(id.as_bytes());
        out.push(b'\n');
    }
    out.extend_from_slice(b"data: ");
    write_json(value, &mut out, 0, max_bytes)?;
    out.extend_from_slice(b"\n\n");
    check_len(&out, max_bytes)?;
    Ok(out)
}

fn check_len(out: &[u8], max_bytes: usize) -> std::result::Result<(), String> {
    if out.len() > max_bytes {
        Err(format!("event exceeds {max_bytes} bytes"))
    } else {
        Ok(())
    }
}

fn write_str(s: &str, out: &mut Vec<u8>, max_bytes: usize) -> std::result::Result<(), String> {
    if s.len() > max_bytes.saturating_sub(out.len()) {
        return Err(format!("event exceeds {max_bytes} bytes"));
    }
    // Escaping can expand a string sixfold. Stop the serializer before growing
    // beyond the configured frame size, not after an unbounded allocation.
    struct Bounded<'a>(&'a mut Vec<u8>, usize);
    impl std::io::Write for Bounded<'_> {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.1.saturating_sub(self.0.len()) {
                return Err(std::io::Error::other("event size limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    serde_json::to_writer(Bounded(out, max_bytes), s)
        .map_err(|_| format!("event exceeds {max_bytes} bytes"))
}

/// Bounded JSON encoding of publishable values. Rejects secrets, functions
/// and runtime handles, including nested ones, even with zero subscribers.
fn write_json(
    value: &Value,
    out: &mut Vec<u8>,
    depth: usize,
    max_bytes: usize,
) -> std::result::Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("event nesting exceeds {MAX_DEPTH} levels"));
    }
    match value {
        Value::Unit => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Int(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::Float(f) if f.is_finite() => {
            serde_json::to_writer(&mut *out, f).map_err(|e| e.to_string())?
        }
        Value::Float(_) => return Err("cannot publish NaN or infinite numbers".into()),
        Value::String(s) => write_str(s, out, max_bytes)?,
        // Each element needs at least 2 bytes ("0,"): reject impossible sizes
        // before walking the whole input.
        Value::Array(items)
            if items.len() > max_bytes.saturating_sub(out.len()).saturating_add(1) / 2 =>
        {
            return Err(format!("event exceeds {max_bytes} bytes"))
        }
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_json(item, out, depth + 1, max_bytes)?;
            }
            out.push(b']');
        }
        Value::Map(map) => write_object(map, out, depth, max_bytes)?,
        Value::Struct { fields, .. } => write_object(fields, out, depth, max_bytes)?,
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Option" => match (variant.as_str(), values.first()) {
            ("Some", Some(inner)) => write_json(inner, out, depth + 1, max_bytes)?,
            _ => out.extend_from_slice(b"null"),
        },
        Value::Secret(_) => return Err("Secret values cannot be published".into()),
        other => return Err(format!("cannot publish a {} value", other.type_name())),
    }
    check_len(out, max_bytes)
}

fn write_object(
    map: &HashMap<String, Value>,
    out: &mut Vec<u8>,
    depth: usize,
    max_bytes: usize,
) -> std::result::Result<(), String> {
    // Each entry needs at least 5 bytes (`"":0,`): reject before sorting keys.
    if map.len() > max_bytes.saturating_sub(out.len()).saturating_add(1) / 5 {
        return Err(format!("event exceeds {max_bytes} bytes"));
    }
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    out.push(b'{');
    for (i, key) in keys.into_iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        write_str(key, out, max_bytes)?;
        out.push(b':');
        write_json(&map[key], out, depth + 1, max_bytes)?;
    }
    out.push(b'}');
    Ok(())
}

/// Parsed `sse()` request carried by the response envelope to the async side.
#[derive(Clone, Debug)]
pub struct SseSpec {
    pub bus: Arc<Bus>,
    pub heartbeat: Duration,
    pub retry_ms: Option<u64>,
}

/// `sse(bus, options?)`: build the response envelope. Opens nothing.
pub fn sse_response(args: &[Value]) -> Result<Value> {
    let Some(bus @ Value::Broadcast(_)) = args.first() else {
        return Err(IntentError::type_error(
            "sse() requires a Broadcast from broadcast() as its first argument",
        ));
    };
    let mut heartbeat_ms = DEFAULT_HEARTBEAT_MS as i64;
    let mut retry_ms: Option<i64> = None;
    if let Some(opts) = args.get(1) {
        let Value::Map(map) = opts else {
            return Err(IntentError::type_error("sse() options must be a map"));
        };
        for (key, value) in map {
            match (key.as_str(), value) {
                ("heartbeat_ms", Value::Int(n)) if (1_000..=300_000).contains(n) => {
                    heartbeat_ms = *n
                }
                ("retry_ms", Value::Int(n)) if (0..=86_400_000).contains(n) => retry_ms = Some(*n),
                ("heartbeat_ms", _) => {
                    return Err(IntentError::type_error(
                        "sse() heartbeat_ms must be an Int from 1000 to 300000",
                    ))
                }
                ("retry_ms", _) => {
                    return Err(IntentError::type_error(
                        "sse() retry_ms must be an Int from 0 to 86400000",
                    ))
                }
                _ => {
                    return Err(IntentError::type_error(format!(
                        "unknown sse() option: {key}"
                    )))
                }
            }
        }
    }
    let mut stream = HashMap::new();
    stream.insert("bus".to_string(), bus.clone());
    stream.insert("heartbeat_ms".to_string(), Value::Int(heartbeat_ms));
    if let Some(retry) = retry_ms {
        stream.insert("retry_ms".to_string(), Value::Int(retry));
    }
    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(200));
    response.insert("headers".to_string(), Value::Map(HashMap::new()));
    response.insert("body".to_string(), Value::String(String::new()));
    response.insert(SSE_KEY.to_string(), Value::Map(stream));
    Ok(Value::Map(response))
}

/// Response-envelope key carrying an `sse()` stream description.
pub const SSE_KEY: &str = "_sse";

/// Extract a stream description from a response map, if present.
pub fn spec_from_response(map: &HashMap<String, Value>) -> Option<SseSpec> {
    let Some(Value::Map(stream)) = map.get(SSE_KEY) else {
        return None;
    };
    let Some(Value::Broadcast(bus)) = stream.get("bus") else {
        return None;
    };
    let heartbeat_ms = match stream.get("heartbeat_ms") {
        Some(Value::Int(n)) if (1_000..=300_000).contains(n) => *n as u64,
        _ => DEFAULT_HEARTBEAT_MS,
    };
    let retry_ms = match stream.get("retry_ms") {
        Some(Value::Int(n)) if (0..=86_400_000).contains(n) => Some(*n as u64),
        _ => None,
    };
    Some(SseSpec {
        bus: Arc::clone(bus),
        heartbeat: Duration::from_millis(heartbeat_ms),
        retry_ms,
    })
}

/// A stream is stalled if it has unread events and has delivered nothing for
/// this long. A healthy client keeps Hyper polling, so it keeps progressing.
#[cfg(not(test))]
const STALL_AGE_MS: u64 = 2_000;
#[cfg(test)]
const STALL_AGE_MS: u64 = 100;

/// Under budget pressure, drop the receivers of stalled streams, oldest
/// progress first, until `enough()` reports the pending publish fits. A
/// stalled client stops Hyper from polling its body, so the stream cannot
/// evict itself; dropping its receiver releases the frames only it retained.
/// Evicted streams end with an overflow event when next polled. Streams that
/// are currently polled (receiver locked) or recently progressing are never
/// touched. Returns the number evicted.
fn evict_stalled(mut enough: impl FnMut() -> bool) -> usize {
    let now = monotonic_ms();
    let mut candidates: Vec<(u64, Arc<Slot>)> = {
        let mut subscribers = SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner());
        subscribers.retain(|weak| weak.strong_count() > 0);
        subscribers
            .iter()
            .filter_map(Weak::upgrade)
            .filter_map(|slot| {
                let last = slot.last_progress_ms.load(Ordering::Acquire);
                (now.saturating_sub(last) >= STALL_AGE_MS).then_some((last, slot))
            })
            .collect()
    };
    candidates.sort_by_key(|(last, _)| *last);
    let mut evicted = 0;
    for (_, slot) in candidates {
        if enough() {
            break;
        }
        if let Ok(mut guard) = slot.rx.try_lock() {
            if guard.as_ref().is_some_and(|rx| !rx.is_empty()) {
                *guard = None;
                evicted += 1;
            }
        }
    }
    evicted
}

/// Held by an open stream; released when the HTTP body is dropped.
pub struct StreamPermit(());

impl Drop for StreamPermit {
    fn drop(&mut self) {
        OPEN_STREAMS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn acquire_stream_permit(max_connections: usize) -> Option<StreamPermit> {
    let mut open = OPEN_STREAMS.load(Ordering::Acquire);
    loop {
        if open >= max_connections {
            return None;
        }
        match OPEN_STREAMS.compare_exchange_weak(
            open,
            open + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(StreamPermit(())),
            Err(actual) => open = actual,
        }
    }
}

struct StreamState {
    /// Keeps the bus alive while the stream is open, so a bus created inside
    /// the request (`sse(broadcast("x"))`) is not freed when the handler returns.
    _bus: Arc<Bus>,
    rx: Arc<Slot>,
    shutdown: watch::Receiver<bool>,
    heartbeat: tokio::time::Interval,
    preamble: Option<Bytes>,
    finished: bool,
    _permit: StreamPermit,
}

/// Why a stream could not be opened.
#[derive(Debug, PartialEq, Eq)]
pub enum OpenError {
    ShuttingDown,
    TooManyStreams,
}

/// Subscribe and build the body stream. Must run inside the Tokio runtime.
pub fn open_stream(
    spec: &SseSpec,
) -> std::result::Result<
    impl futures_util::Stream<Item = std::result::Result<Bytes, Infallible>> + Send + 'static,
    OpenError,
> {
    let shutdown = SHUTDOWN.subscribe();
    if *shutdown.borrow() {
        return Err(OpenError::ShuttingDown);
    }
    let configuration = LIMITS.read().unwrap_or_else(|e| e.into_inner());
    let limits = configuration.unwrap_or_default();
    let permit = acquire_stream_permit(limits.max_connections).ok_or(OpenError::TooManyStreams)?;
    let mut heartbeat =
        tokio::time::interval_at(tokio::time::Instant::now() + spec.heartbeat, spec.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // A first frame flushes headers so clients see the stream open at once.
    let preamble = match spec.retry_ms {
        Some(retry) => format!("retry: {retry}\n\n"),
        None => ":\n\n".to_string(),
    };
    let slot = Arc::new(Slot {
        rx: tokio::sync::Mutex::new(Some(spec.bus.tx.subscribe())),
        last_progress_ms: AtomicU64::new(monotonic_ms()),
    });
    {
        let mut subscribers = SUBSCRIBERS.lock().unwrap_or_else(|e| e.into_inner());
        if subscribers.len() >= limits.max_connections.saturating_mul(2) {
            subscribers.retain(|weak| weak.strong_count() > 0);
        }
        subscribers.push(Arc::downgrade(&slot));
    }
    let state = StreamState {
        _bus: Arc::clone(&spec.bus),
        rx: slot,
        shutdown,
        heartbeat,
        preamble: Some(Bytes::from(preamble)),
        finished: false,
        _permit: permit,
    };
    Ok(futures_util::stream::unfold(state, next_frame))
}

async fn next_frame(
    mut s: StreamState,
) -> Option<(std::result::Result<Bytes, Infallible>, StreamState)> {
    if s.finished {
        return None;
    }
    if let Some(preamble) = s.preamble.take() {
        return Some((Ok(preamble), s));
    }
    if *s.shutdown.borrow() {
        return None;
    }
    let slot = Arc::clone(&s.rx);
    slot.last_progress_ms
        .store(monotonic_ms(), Ordering::Release);
    let mut guard = slot.rx.lock().await;
    let Some(rx) = guard.as_mut() else {
        // Evicted while stalled: tell the client, then end.
        s.finished = true;
        return Some((Ok(Bytes::from_static(OVERFLOW)), s));
    };
    tokio::select! {
        biased;
        _ = s.shutdown.changed() => None,
        received = rx.recv() => match received {
            Ok(event) => {
                // Delivered: the client is progressing even if it waited idle
                // for a long time before this event.
                slot.last_progress_ms.store(monotonic_ms(), Ordering::Release);
                Some((Ok(Bytes::from_owner(Frame(event))), s))
            }
            // A slow consumer loses its connection rather than silently
            // skipping events; the client reconnects and reconciles.
            Err(broadcast::error::RecvError::Lagged(_)) => {
                s.finished = true;
                Some((Ok(Bytes::from_static(OVERFLOW)), s))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        },
        _ = s.heartbeat.tick() => Some((Ok(Bytes::from_static(b":\n\n")), s)),
    }
}

const OVERFLOW: &[u8] = b"event: ntnt.overflow\ndata: null\n\n";

/// End all open streams and refuse new ones. Called on process shutdown.
pub fn shutdown() {
    SHUTDOWN.send_replace(true);
}

/// Live counters, for tests and diagnostics.
pub fn stats() -> (usize, usize) {
    (
        OPEN_STREAMS.load(Ordering::Acquire),
        PAYLOAD_USED.load(Ordering::Acquire),
    )
}

/// Number of receivers currently attached to a bus.
pub fn subscribers(bus: &Bus) -> usize {
    bus.subscriber_count()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stream and budget counters are process-global; tests that assert on
    /// them run one at a time.
    static COUNTERS: Mutex<()> = Mutex::new(());

    fn map(entries: &[(&str, Value)]) -> Value {
        Value::Map(
            entries
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        )
    }

    fn bus(name: &str) -> Arc<Bus> {
        match broadcast_value(&[Value::String(name.into())]).unwrap() {
            Value::Broadcast(bus) => bus,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn realtime_limits_parse_defaults_and_reject_invalid_values() {
        assert_eq!(Limits::parse(&map(&[])).unwrap(), Limits::default());
        for key in [
            "max_connections",
            "max_topics",
            "max_event_bytes",
            "max_payload_bytes",
        ] {
            for value in [
                Value::Int(0),
                Value::Int(-1),
                Value::Float(1.0),
                Value::Bool(true),
                Value::Unit,
            ] {
                assert!(Limits::parse(&map(&[(key, value)])).is_err(), "{key}");
            }
            let mut values = vec![(key, Value::Int(i64::MAX))];
            if key == "max_event_bytes" {
                values.push(("max_payload_bytes", Value::Int(i64::MAX)));
            }
            assert_eq!(
                Limits::parse(&map(&values)).is_ok(),
                usize::BITS == 64,
                "{key}"
            );
        }
        let limits = Limits::parse(&map(&[
            ("max_topics", Value::Int(3)),
            ("max_connections", Value::Int(2)),
            ("max_event_bytes", Value::Int(16)),
            ("max_payload_bytes", Value::Int(32)),
        ]))
        .unwrap();
        assert_eq!(
            limits,
            Limits {
                max_topics: 3,
                max_connections: 2,
                max_event_bytes: 16,
                max_payload_bytes: 32
            }
        );
        assert!(Limits::parse(&map(&[("max_payload_bytes", Value::Int(1))])).is_err());
        assert!(Limits::parse(&map(&[("typo", Value::Int(1))])).is_err());
    }

    #[test]
    fn startup_usage_errors_report_the_specific_limit_and_both_values() {
        let limits = Limits {
            max_topics: 1,
            max_connections: 1,
            max_event_bytes: 1,
            max_payload_bytes: 1,
        };
        assert!(limits.check_usage(1, 1, 1).is_ok());
        for (name, topics, payload, connections) in [
            ("max_topics", 2, 0, 0),
            ("max_payload_bytes", 0, 2, 0),
            ("max_connections", 0, 0, 2),
        ] {
            let error = limits
                .check_usage(topics, payload, connections)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(&format!(
                    "realtime {name} (1) is below resources already in use (2)"
                )),
                "{error}"
            );
        }
    }

    #[test]
    fn configured_encoding_bound_includes_framing_metadata_and_escaping() {
        let value = Value::String("x".into());
        let frame = encode_frame(&value, Some("tick"), Some("1")).unwrap();
        assert!(encode_frame_with_limit(&value, Some("tick"), Some("1"), frame.len()).is_ok());
        assert!(encode_frame_with_limit(&value, Some("tick"), Some("1"), frame.len() - 1).is_err());
        let mut out = Vec::new();
        assert!(write_str(&"\0".repeat(32), &mut out, 32).is_err());
        assert!(
            out.len() <= 32,
            "escaping must not first allocate the expanded string"
        );
    }

    #[test]
    fn configured_budget_stays_charged_until_the_last_frame_reference_drops() {
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let before = stats().1;
        assert!(reserve_payload(32, before + 32));
        let event = Arc::new(EncodedEvent { bytes: vec![0; 32] });
        let bytes = Bytes::from_owner(Frame(event.clone()));
        assert!(!reserve_payload(1, before + 32));
        drop(event);
        assert_eq!(stats().1, before + 32);
        drop(bytes);
        assert_eq!(stats().1, before);
    }

    #[test]
    fn frames_are_compact_sorted_json_with_validated_metadata() {
        let value = map(&[
            ("b", Value::String("line1\nline2 ✓".into())),
            (
                "a",
                Value::Array(vec![Value::Int(1), Value::Float(1.5), Value::Unit]),
            ),
        ]);
        let frame = encode_frame(&value, Some("tick"), Some("7")).unwrap();
        assert_eq!(
            String::from_utf8(frame).unwrap(),
            "event: tick\nid: 7\ndata: {\"a\":[1,1.5,null],\"b\":\"line1\\nline2 ✓\"}\n\n"
        );
        assert!(encode_frame(&Value::Int(1), Some("a\nb"), None).is_err());
        assert!(encode_frame(&Value::Int(1), None, Some("x\ry")).is_err());
        assert!(encode_frame(&Value::Float(f64::NAN), None, None).is_err());
    }

    #[test]
    fn secrets_handles_oversize_and_deep_values_are_rejected() {
        let secret = Value::Secret(crate::secret::SecretValue::new("TEST", "s").unwrap());
        let nested = map(&[("inner", Value::Array(vec![secret]))]);
        assert!(encode_frame(&nested, None, None)
            .unwrap_err()
            .contains("Secret"));
        assert!(encode_frame(&Value::TaskHandle(1), None, None).is_err());
        let big = Value::String("x".repeat(MAX_EVENT_BYTES));
        assert!(encode_frame(&big, None, None)
            .unwrap_err()
            .contains("exceeds"));
        let mut deep = Value::Int(0);
        for _ in 0..=MAX_DEPTH + 1 {
            deep = Value::Array(vec![deep]);
        }
        assert!(encode_frame(&deep, None, None).is_err());
    }

    #[test]
    fn registry_returns_the_same_bus_and_frees_unused_names() {
        let a = bus("test.same");
        let b = bus("test.same");
        assert!(Arc::ptr_eq(&a, &b));
        let conflict = broadcast_value(&[
            Value::String("test.same".into()),
            map(&[("capacity", Value::Int(DEFAULT_CAPACITY as i64 + 1))]),
        ]);
        assert!(conflict.is_err());
        let weak = Arc::downgrade(&a);
        drop((a, b));
        assert!(weak.upgrade().is_none());
        assert!(broadcast_value(&[Value::String("bad name!".into())]).is_err());
    }

    #[test]
    fn publish_validates_with_zero_subscribers_and_budget_is_released() {
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let bus = bus("test.publish");
        let (_, before) = stats();
        let ok = publish_value(&[Value::Broadcast(bus.clone()), Value::Int(1)]).unwrap();
        assert!(matches!(ok, Value::EnumValue { ref variant, .. } if variant == "Ok"));
        // With no receivers tokio drops the message immediately.
        assert_eq!(stats().1, before);
        let secret = Value::Secret(crate::secret::SecretValue::new("TEST", "s").unwrap());
        let err = publish_value(&[Value::Broadcast(bus), secret]).unwrap();
        assert!(matches!(err, Value::EnumValue { ref variant, .. } if variant == "Err"));
    }

    #[tokio::test]
    async fn stream_yields_shared_frames_and_releases_permit_and_budget_on_drop() {
        use futures_util::StreamExt;
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let bus = bus("test.stream");
        let spec = SseSpec {
            bus: bus.clone(),
            heartbeat: Duration::from_secs(60),
            retry_ms: Some(1500),
        };
        let (open_before, _) = stats();
        let mut stream = Box::pin(open_stream(&spec).unwrap());
        assert_eq!(stats().0, open_before + 1);
        assert_eq!(subscribers(&bus), 1);
        assert_eq!(
            &stream.next().await.unwrap().unwrap()[..],
            b"retry: 1500\n\n"
        );
        publish_value(&[
            Value::Broadcast(bus.clone()),
            map(&[("n", Value::Int(1))]),
            map(&[("event", Value::String("tick".into()))]),
        ])
        .unwrap();
        let frame = stream.next().await.unwrap().unwrap();
        assert_eq!(&frame[..], b"event: tick\ndata: {\"n\":1}\n\n");
        let charged = stats().1;
        assert!(charged >= frame.len());
        drop(stream);
        assert_eq!(stats().0, open_before);
        assert_eq!(subscribers(&bus), 0);
        // The frame still held by the body keeps its charge until dropped.
        assert_eq!(stats().1, charged);
        drop(frame);
        assert!(stats().1 < charged);
    }

    #[tokio::test]
    async fn stream_keeps_a_request_created_bus_alive() {
        use futures_util::StreamExt;
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let spec = SseSpec {
            bus: bus("test.ephemeral"),
            heartbeat: Duration::from_secs(60),
            retry_ms: None,
        };
        let mut stream = Box::pin(open_stream(&spec).unwrap());
        drop(spec);
        stream.next().await.unwrap().unwrap();
        let again = bus("test.ephemeral");
        assert_eq!(subscribers(&again), 1, "same bus while the stream is open");
        publish_value(&[Value::Broadcast(again), Value::Int(5)]).unwrap();
        assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"data: 5\n\n");
    }

    #[tokio::test]
    async fn budget_pressure_evicts_stalled_subscribers_on_any_bus() {
        use futures_util::StreamExt;
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let stalled_bus = match broadcast_value(&[
            Value::String("test.stalled".into()),
            map(&[("capacity", Value::Int(1024))]),
        ])
        .unwrap()
        {
            Value::Broadcast(bus) => bus,
            _ => unreachable!(),
        };
        let spec = SseSpec {
            bus: stalled_bus.clone(),
            heartbeat: Duration::from_secs(60),
            retry_ms: None,
        };
        let mut stalled = Box::pin(open_stream(&spec).unwrap());
        stalled.next().await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(STALL_AGE_MS * 2)).await;
        // Never poll `stalled` again: fill the budget through its bus.
        let blob = Value::String("x".repeat(MAX_EVENT_BYTES - 64));
        let mut published = 0;
        for _ in 0..1024 {
            match publish_value(&[Value::Broadcast(stalled_bus.clone()), blob.clone()]).unwrap() {
                Value::EnumValue { variant, .. } if variant == "Ok" => published += 1,
                other => panic!("publish failed under pressure: {other:?} after {published}"),
            }
        }
        // Another bus can still publish, and the stalled stream was evicted.
        let other = bus("test.other");
        let ok = publish_value(&[Value::Broadcast(other), Value::Int(1)]).unwrap();
        assert!(matches!(ok, Value::EnumValue { ref variant, .. } if variant == "Ok"));
        assert!(stats().1 <= MAX_PAYLOAD_BUDGET);
        let mut ended = false;
        while let Some(frame) = stalled.next().await {
            if frame.unwrap().starts_with(b"event: ntnt.overflow") {
                ended = true;
            }
        }
        assert!(ended, "evicted stream ends with an overflow event");
    }

    #[tokio::test]
    async fn eviction_spares_progressing_streams_and_stops_when_enough() {
        use futures_util::StreamExt;
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let open = |name: &str| {
            let spec = SseSpec {
                bus: bus(name),
                heartbeat: Duration::from_secs(60),
                retry_ms: None,
            };
            (spec.bus.clone(), Box::pin(open_stream(&spec).unwrap()))
        };
        let (old_bus, mut oldest) = open("test.evict.oldest");
        oldest.next().await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let (newer_bus, mut newer) = open("test.evict.newer");
        newer.next().await.unwrap().unwrap();
        tokio::time::sleep(Duration::from_millis(STALL_AGE_MS * 2)).await;
        let (fresh_bus, mut fresh) = open("test.evict.fresh");
        fresh.next().await.unwrap().unwrap();
        for b in [&old_bus, &newer_bus, &fresh_bus] {
            for n in 0..2 {
                publish_value(&[Value::Broadcast(b.clone()), Value::Int(n)]).unwrap();
            }
        }
        // Enough after one eviction: only the oldest stalled stream goes.
        let mut checks = 0;
        assert_eq!(
            evict_stalled(|| {
                checks += 1;
                checks > 1
            }),
            1
        );
        let ended = |frames: Vec<Bytes>| {
            frames
                .iter()
                .any(|f| f.starts_with(b"event: ntnt.overflow"))
        };
        assert!(ended(oldest.by_ref().map(|f| f.unwrap()).collect().await));
        // The newer stalled stream and the recently progressing one are intact.
        assert_eq!(&newer.next().await.unwrap().unwrap()[..], b"data: 0\n\n");
        assert_eq!(&fresh.next().await.unwrap().unwrap()[..], b"data: 0\n\n");
        // Never enough: every stalled stream with a backlog is a candidate,
        // but a stream that just progressed is not.
        assert_eq!(evict_stalled(|| false), 0);
    }

    #[tokio::test]
    async fn long_idle_stream_that_just_received_is_not_stalled() {
        use futures_util::StreamExt;
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let spec = SseSpec {
            bus: bus("test.idle"),
            heartbeat: Duration::from_secs(60),
            retry_ms: None,
        };
        let mut stream = Box::pin(open_stream(&spec).unwrap());
        stream.next().await.unwrap().unwrap();
        // Waits idle past the stall age, then a burst arrives.
        let waiting = tokio::spawn(async move {
            let first = stream.next().await.unwrap().unwrap();
            (stream, first)
        });
        tokio::time::sleep(Duration::from_millis(STALL_AGE_MS * 2)).await;
        for n in 0..3 {
            publish_value(&[Value::Broadcast(spec.bus.clone()), Value::Int(n)]).unwrap();
        }
        let (mut stream, first) = waiting.await.unwrap();
        assert_eq!(&first[..], b"data: 0\n\n");
        // Between polls, with unread events, it must not look stalled.
        assert_eq!(evict_stalled(|| false), 0);
        assert_eq!(&stream.next().await.unwrap().unwrap()[..], b"data: 1\n\n");
    }

    #[test]
    fn oversized_collections_are_rejected_before_full_traversal() {
        let wide: HashMap<String, Value> = (0..20_000)
            .map(|i| (i.to_string(), Value::Int(0)))
            .collect();
        assert!(encode_frame(&Value::Map(wide), None, None).is_err());
        let long = Value::Array(vec![Value::Int(0); 40_000]);
        assert!(encode_frame(&long, None, None).is_err());
        let fits = Value::Array(vec![Value::Int(0); 1000]);
        assert!(encode_frame(&fits, None, None).is_ok());
    }

    #[tokio::test]
    async fn slow_subscriber_is_closed_after_lag() {
        use futures_util::StreamExt;
        let _serial = COUNTERS.lock().unwrap_or_else(|e| e.into_inner());
        let bus = match broadcast_value(&[
            Value::String("test.lag".into()),
            map(&[("capacity", Value::Int(2))]),
        ])
        .unwrap()
        {
            Value::Broadcast(bus) => bus,
            _ => unreachable!(),
        };
        let spec = SseSpec {
            bus: bus.clone(),
            heartbeat: Duration::from_secs(60),
            retry_ms: None,
        };
        let mut stream = Box::pin(open_stream(&spec).unwrap());
        stream.next().await.unwrap().unwrap();
        for n in 0..5 {
            publish_value(&[Value::Broadcast(bus.clone()), Value::Int(n)]).unwrap();
        }
        let overflow = stream.next().await.unwrap().unwrap();
        assert!(overflow.starts_with(b"event: ntnt.overflow"));
        assert!(stream.next().await.is_none());
    }
}
