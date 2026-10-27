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
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
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

lazy_static::lazy_static! {
    static ref REGISTRY: Mutex<HashMap<String, Weak<Bus>>> = Mutex::new(HashMap::new());
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

fn reserve_payload(len: usize) -> bool {
    let mut used = PAYLOAD_USED.load(Ordering::Acquire);
    loop {
        let Some(next) = used.checked_add(len).filter(|n| *n <= MAX_PAYLOAD_BUDGET) else {
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
    if registry.len() >= MAX_TOPICS {
        return Err(IntentError::runtime_error(format!(
            "too many broadcast names (limit {MAX_TOPICS})"
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
    match encode_frame(value, event.as_deref(), id.as_deref()) {
        Ok(frame) => {
            if !reserve_payload(frame.len()) {
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
pub(crate) fn encode_frame(
    value: &Value,
    event: Option<&str>,
    id: Option<&str>,
) -> std::result::Result<Vec<u8>, String> {
    let mut out = Vec::with_capacity(256);
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
    write_json(value, &mut out, 0)?;
    out.extend_from_slice(b"\n\n");
    if out.len() > MAX_EVENT_BYTES {
        return Err(format!("event exceeds {MAX_EVENT_BYTES} bytes"));
    }
    Ok(out)
}

fn check_len(out: &[u8]) -> std::result::Result<(), String> {
    if out.len() > MAX_EVENT_BYTES {
        Err(format!("event exceeds {MAX_EVENT_BYTES} bytes"))
    } else {
        Ok(())
    }
}

fn write_str(s: &str, out: &mut Vec<u8>) -> std::result::Result<(), String> {
    if out.len() + s.len() > MAX_EVENT_BYTES {
        return Err(format!("event exceeds {MAX_EVENT_BYTES} bytes"));
    }
    serde_json::to_writer(&mut *out, s).map_err(|e| e.to_string())?;
    check_len(out)
}

/// Bounded JSON encoding of publishable values. Rejects secrets, functions
/// and runtime handles, including nested ones, even with zero subscribers.
fn write_json(value: &Value, out: &mut Vec<u8>, depth: usize) -> std::result::Result<(), String> {
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
        Value::String(s) => write_str(s, out)?,
        Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                write_json(item, out, depth + 1)?;
            }
            out.push(b']');
        }
        Value::Map(map) => write_object(map, out, depth)?,
        Value::Struct { fields, .. } => write_object(fields, out, depth)?,
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Option" => match (variant.as_str(), values.first()) {
            ("Some", Some(inner)) => write_json(inner, out, depth + 1)?,
            _ => out.extend_from_slice(b"null"),
        },
        Value::Secret(_) => return Err("Secret values cannot be published".into()),
        other => return Err(format!("cannot publish a {} value", other.type_name())),
    }
    check_len(out)
}

fn write_object(
    map: &HashMap<String, Value>,
    out: &mut Vec<u8>,
    depth: usize,
) -> std::result::Result<(), String> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    out.push(b'{');
    for (i, key) in keys.into_iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        write_str(key, out)?;
        out.push(b':');
        write_json(&map[key], out, depth + 1)?;
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

/// Held by an open stream; released when the HTTP body is dropped.
pub struct StreamPermit(());

impl Drop for StreamPermit {
    fn drop(&mut self) {
        OPEN_STREAMS.fetch_sub(1, Ordering::AcqRel);
    }
}

fn acquire_stream_permit() -> Option<StreamPermit> {
    let mut open = OPEN_STREAMS.load(Ordering::Acquire);
    loop {
        if open >= MAX_CONNECTIONS {
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
    rx: broadcast::Receiver<Arc<EncodedEvent>>,
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
    let permit = acquire_stream_permit().ok_or(OpenError::TooManyStreams)?;
    let mut heartbeat =
        tokio::time::interval_at(tokio::time::Instant::now() + spec.heartbeat, spec.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // A first frame flushes headers so clients see the stream open at once.
    let preamble = match spec.retry_ms {
        Some(retry) => format!("retry: {retry}\n\n"),
        None => ":\n\n".to_string(),
    };
    let state = StreamState {
        rx: spec.bus.tx.subscribe(),
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
    tokio::select! {
        biased;
        _ = s.shutdown.changed() => None,
        received = s.rx.recv() => match received {
            Ok(event) => Some((Ok(Bytes::from_owner(Frame(event))), s)),
            // A slow consumer loses its connection rather than silently
            // skipping events; the client reconnects and reconciles.
            Err(broadcast::error::RecvError::Lagged(_)) => {
                s.finished = true;
                Some((Ok(Bytes::from_static(b"event: ntnt.overflow\ndata: null\n\n")), s))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        },
        _ = s.heartbeat.tick() => Some((Ok(Bytes::from_static(b":\n\n")), s)),
    }
}

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
