//! std/concurrent module — Structured concurrency primitives
//!
//! Provides channels, tasks (spawn/await), scheduled execution, and cooperative cancellation.
//!
//! ## Architecture
//!
//! A single `ConcurrencyRuntime` (via `LazyLock<ConcurrencyRuntime>`) owns all state:
//! - Tasks, channels, and schedules share one monotonic ID counter (`AtomicU64`).
//! - Lock discipline: acquire registry lock → clone Arcs → drop lock → operate. NEVER nest locks.
//! - All atomics use `Release`/`Acquire` pairing. No SeqCst, no Relaxed.
//!
//! ## Channels
//!
//! - `close()` removes from map (no `closed` flag). Dropping the `Sender` causes
//!   `recv()` to return `Unit` (disconnected).
//! - `send()` on a removed channel returns `false`.
//! - `recv()` is single-consumer — the receiver `MutexGuard` is held for the blocking duration.
//!   Uses 100ms timeout slices internally for cancellation awareness.
//!
//! ## Tasks
//!
//! - `await_task()` marks state as `Consumed` (the "I'm done with this handle" call).
//! - `try_await()` peeks without consuming — updates `last_checked_at`. Never errors for
//!   handles that existed; returns `{status: "consumed"}` or `{status: "expired"}` instead.
//! - `cancel_task()` calls `.cancel()` on the task's `CancelToken` (cooperative cancellation via yield points).
//! - All `eval_block` calls are wrapped in `catch_unwind(AssertUnwindSafe(...))`.
//! - Tasks auto-expire (marked `Expired`) after 5 minutes in terminal state (the reaper runs on
//!   `spawn()` and `after()` entry), but only if not recently `try_await()`'d.
//! - Expired entries are removed from the registry after 7 days (configurable via
//!   `NTNT_TASK_REMOVAL_TTL` env var in seconds) to prevent memory leaks in long-running servers.
//!
//! ## Schedules
//!
//! - `cancel_schedule()` sets flag AND removes from registry.
//! - Schedule sleep uses `CancelToken::wait_timeout()` for instant wakeup on cancellation.
//! - Zero-duration intervals are rejected.
//! - Tick execution spawns a thread with `catch_unwind` and overlap prevention via `AtomicBool`.
//!
//! ## Cancellation
//!
//! Thread-local `CURRENT_CANCEL_TOKEN` for cooperative cancellation.
//! Yield points: `recv()`, `recv_timeout()`, `sleep_ms()`, `fetch()`.
//! Note: `sleep()` from std/time is NOT cancellation-aware.

use crate::error::IntentError;
use crate::interpreter::{RuntimeCapability, Value};
use crossbeam_channel::{self as crossbeam};
use std::collections::{HashMap, HashSet};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, Weak};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

// =============================================================================
// CancelToken — cooperative cancellation with instant Condvar wakeup
// =============================================================================

/// Cooperative cancellation token. `cancel()` sets the flag and wakes all
/// threads blocked in `wait_timeout()` instantly.
pub struct CancelToken {
    inner: Mutex<bool>,
    condvar: Condvar,
}

impl CancelToken {
    pub fn new() -> Self {
        CancelToken {
            inner: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    /// Signal cancellation. Sets flag and wakes all waiting threads instantly.
    pub fn cancel(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *guard = true;
        self.condvar.notify_all();
    }

    /// Returns true if cancelled (or if mutex is poisoned).
    pub fn is_cancelled(&self) -> bool {
        self.inner.lock().map_or(true, |g| *g)
    }

    /// Sleep for up to `duration`, waking instantly if cancelled.
    /// Returns true if cancelled, false if timeout elapsed.
    pub fn wait_timeout(&self, duration: Duration) -> bool {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return true,
        };
        if *guard {
            return true;
        }
        self.condvar
            .wait_timeout_while(guard, duration, |cancelled| !*cancelled)
            .map_or(true, |(g, _)| *g)
    }
}

// =============================================================================
// Thread-local cancellation token (cooperative cancellation)
// =============================================================================

thread_local! {
    /// Set by the task's thread to point at the task's cancellation token.
    /// Yield-point functions check this to honour cooperative cancellation.
    pub static CURRENT_CANCEL_TOKEN: std::cell::RefCell<Option<Arc<CancelToken>>> =
        const { std::cell::RefCell::new(None) };
}

/// Check if the current thread's task has been cancelled.
/// Called at yield points: recv, recv_timeout, sleep_ms, http_fetch, http_get.
pub fn is_current_task_cancelled() -> bool {
    CURRENT_CANCEL_TOKEN.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|t| t.is_cancelled())
            .unwrap_or(false)
    })
}

/// Sleep for up to `duration`, waking instantly on cancellation.
/// Returns true if cancelled, false if duration elapsed.
pub fn sleep_cancellable(duration: Duration) -> bool {
    CURRENT_CANCEL_TOKEN.with(|cell| match cell.borrow().as_ref() {
        Some(token) => token.wait_timeout(duration),
        None => {
            thread::sleep(duration);
            false
        }
    })
}

/// Return Err if the current task is cancelled, for use at yield points.
fn check_cancellation() -> Result<()> {
    if is_current_task_cancelled() {
        Err(IntentError::runtime_error("Task cancelled".to_string()))
    } else {
        Ok(())
    }
}

// =============================================================================
// SerializedValue — thread-safe value transmission
// =============================================================================

/// Serialized value for thread-safe transmission across task/channel boundaries.
///
/// Each NTNT value type has a dedicated variant — no marker keys that could
/// collide with user data.
#[derive(Debug, Clone)]
pub(crate) enum SerializedValue {
    Unit,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<SerializedValue>),
    Map(HashMap<String, SerializedValue>),
    /// Struct with type name and serialized fields.
    Struct {
        name: String,
        fields: HashMap<String, SerializedValue>,
    },
    /// Enum variant with enum name, variant name, and associated values.
    EnumValue {
        enum_name: String,
        variant: String,
        values: Vec<SerializedValue>,
    },
    /// Task handle — just the ID.
    TaskHandle(u64),
    /// Sender handle — carries the Arc<Sender> so it disconnects naturally on drop.
    TxChannelHandle(u64, Arc<crossbeam::Sender<SerializedValue>>),
    /// Receiver handle — just the ID; receiver lives in the registry.
    RxChannelHandle(u64),
    /// Schedule handle — just the ID.
    ScheduleHandle(u64),
}

impl SerializedValue {
    /// Convert from Value to SerializedValue.
    /// Only serializable types are accepted; closures and NativeFunctions produce an error.
    pub(crate) fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Unit => Ok(SerializedValue::Unit),
            Value::Int(i) => Ok(SerializedValue::Int(*i)),
            Value::Float(f) => Ok(SerializedValue::Float(*f)),
            Value::Bool(b) => Ok(SerializedValue::Bool(*b)),
            Value::String(s) => Ok(SerializedValue::String(s.clone())),
            Value::Array(arr) => {
                let serialized: Result<Vec<_>> = arr.iter().map(Self::from_value).collect();
                Ok(SerializedValue::Array(serialized?))
            }
            Value::Map(map) => {
                let mut serialized = HashMap::new();
                for (k, v) in map {
                    serialized.insert(k.clone(), Self::from_value(v)?);
                }
                Ok(SerializedValue::Map(serialized))
            }
            Value::Struct { name, fields } => {
                let mut serialized = HashMap::new();
                for (k, v) in fields {
                    serialized.insert(k.clone(), Self::from_value(v)?);
                }
                Ok(SerializedValue::Struct {
                    name: name.clone(),
                    fields: serialized,
                })
            }
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } => {
                let vals: Result<Vec<_>> = values.iter().map(Self::from_value).collect();
                Ok(SerializedValue::EnumValue {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                    values: vals?,
                })
            }
            Value::TaskHandle(id) => Ok(SerializedValue::TaskHandle(*id)),
            Value::TxChannelHandle(id, cs) => {
                // Downcast the opaque Arc<dyn Any> back to Arc<Sender<SerializedValue>>.
                // This clone keeps the sender alive in the spawned task.
                let sender_arc = Arc::clone(&cs.0)
                    .downcast::<crossbeam::Sender<SerializedValue>>()
                    .map_err(|_| {
                        IntentError::runtime_error(
                            "Internal: TxChannelHandle contains unexpected sender type".to_string(),
                        )
                    })?;
                Ok(SerializedValue::TxChannelHandle(*id, sender_arc))
            }
            Value::RxChannelHandle(id) => Ok(SerializedValue::RxChannelHandle(*id)),
            Value::ScheduleHandle(id) => Ok(SerializedValue::ScheduleHandle(*id)),
            _ => Err(IntentError::type_error(
                "Only serializable types (Int, Float, String, Bool, Array, Map, Struct, Enum) can be sent across task boundaries".to_string(),
            )),
        }
    }

    /// Convert back to Value.
    pub(crate) fn to_value(&self) -> Value {
        match self {
            SerializedValue::Unit => Value::Unit,
            SerializedValue::Int(i) => Value::Int(*i),
            SerializedValue::Float(f) => Value::Float(*f),
            SerializedValue::Bool(b) => Value::Bool(*b),
            SerializedValue::String(s) => Value::String(s.clone()),
            SerializedValue::Array(arr) => Value::Array(arr.iter().map(|v| v.to_value()).collect()),
            SerializedValue::Map(map) => {
                let mut result = HashMap::new();
                for (k, v) in map {
                    result.insert(k.clone(), v.to_value());
                }
                Value::Map(result)
            }
            SerializedValue::Struct { name, fields } => {
                let mut result = HashMap::new();
                for (k, v) in fields {
                    result.insert(k.clone(), v.to_value());
                }
                Value::Struct {
                    name: name.clone(),
                    fields: result,
                }
            }
            SerializedValue::EnumValue {
                enum_name,
                variant,
                values,
            } => Value::EnumValue {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                values: values.iter().map(|v| v.to_value()).collect(),
            },
            SerializedValue::TaskHandle(id) => Value::TaskHandle(*id),
            SerializedValue::TxChannelHandle(id, sender_arc) => Value::TxChannelHandle(
                *id,
                crate::interpreter::ChannelSender(
                    Arc::clone(sender_arc) as Arc<dyn std::any::Any + Send + Sync>
                ),
            ),
            SerializedValue::RxChannelHandle(id) => Value::RxChannelHandle(*id),
            SerializedValue::ScheduleHandle(id) => Value::ScheduleHandle(*id),
        }
    }
}

// =============================================================================
// Task states
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskState {
    Running,
    Completed,
    Failed,
    Panicked,
    /// Result was consumed by await_task — handle is spent.
    Consumed,
    /// Reaper cleaned up the task after 5-minute TTL.
    Expired,
}

impl TaskState {
    fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed
                | TaskState::Failed
                | TaskState::Panicked
                | TaskState::Consumed
                | TaskState::Expired
        )
    }
}

// =============================================================================
// Task entry
// =============================================================================

/// Mutable task state protected by a single mutex. Updated atomically in `finalize_task()`.
pub(crate) struct TaskInner {
    pub(crate) state: TaskState,
    pub(crate) result: Option<SerializedValue>,
    pub(crate) error_msg: Option<String>,
    pub(crate) completed_at: Option<Instant>,
}

struct TaskEntry {
    /// Core mutable state — one lock instead of four.
    inner: Arc<Mutex<TaskInner>>,
    /// Cooperative cancellation token. `cancel_task()` calls `.cancel()` for instant wakeup.
    cancelled: Arc<CancelToken>,
    /// Last time `try_await()` checked this task (prevents reaping active handles).
    last_checked_at: Arc<Mutex<Option<Instant>>>,
    /// Condvar notified by `finalize_task()` when the task reaches a terminal state.
    /// `await_task()` waits on this instead of polling with `thread::sleep`.
    completed_notify: Arc<(Mutex<bool>, Condvar)>,
}

/// Cloned Arcs for operating on a task outside the registry lock.
/// Replaces the old 6-tuple return from `get_task_arcs()`.
pub(crate) struct TaskArcs {
    pub(crate) inner: Arc<Mutex<TaskInner>>,
    pub(crate) completed_notify: Arc<(Mutex<bool>, Condvar)>,
}

// =============================================================================
// Channel entry — no `closed` flag, close = remove from map
// =============================================================================

struct ChannelEntry {
    /// The receiver end. The sender lives entirely in TxChannelHandle values — when all
    /// TxChannelHandle clones for this channel are dropped, the Arc<Sender> refcount hits
    /// zero, the Sender drops, and recv() sees Disconnected → returns Unit automatically.
    /// No sentinel injection required.
    receiver: Arc<Mutex<crossbeam::Receiver<SerializedValue>>>,
    /// Weak probe to the sender Arc. When all TxChannelHandle clones are dropped, this
    /// becomes dead (upgrade() returns None). Used by the channel reaper to detect
    /// orphaned channels where close() was never called.
    sender_probe: Weak<dyn std::any::Any + Send + Sync>,
}

// =============================================================================
// Schedule entry
// =============================================================================

struct ScheduleEntry {
    cancelled: Arc<CancelToken>,
}

// =============================================================================
// ConcurrencyRuntime — single struct owns all state
// =============================================================================

/// The single concurrency runtime. One `static RUNTIME: LazyLock<ConcurrencyRuntime>`.
///
/// **Lock discipline (NEVER nest):**
/// Acquire registry lock → clone Arcs → drop lock → operate on cloned Arcs.
pub struct ConcurrencyRuntime {
    /// Monotonic ID counter shared by tasks, channels, and schedules.
    id_counter: AtomicU64,
    /// Number of currently active (Running) tasks. Incremented on spawn, decremented on finalize.
    pub(crate) active_tasks: AtomicU64,
    /// Task registry. Lock, clone Arcs, drop, then operate.
    tasks: Mutex<HashMap<u64, TaskEntry>>,
    /// Channel registry. close() = remove from this map.
    channels: Mutex<HashMap<u64, ChannelEntry>>,
    /// Schedule registry. cancel_schedule() = set flag + remove.
    schedules: Mutex<HashMap<u64, ScheduleEntry>>,
    /// Last time the inline reaper ran. Used to rate-limit reap_expired_tasks() calls on
    /// spawn()/after() to at most once per 10 seconds, avoiding O(n) Arc clones on every spawn.
    last_inline_reap: Mutex<Instant>,
}

impl ConcurrencyRuntime {
    fn new() -> Self {
        ConcurrencyRuntime {
            id_counter: AtomicU64::new(1),
            active_tasks: AtomicU64::new(0),
            tasks: Mutex::new(HashMap::new()),
            channels: Mutex::new(HashMap::new()),
            schedules: Mutex::new(HashMap::new()),
            last_inline_reap: Mutex::new(Instant::now()),
        }
    }

    fn next_id(&self) -> u64 {
        self.id_counter.fetch_add(1, AtomicOrdering::Release)
    }

    // -------------------------------------------------------------------------
    // Reaper — auto-expire terminal tasks older than 5 minutes, remove after 7 days
    // -------------------------------------------------------------------------

    /// Rate-limited wrapper around `reap_expired_tasks`. Only runs the full reap if at least
    /// 10 seconds have elapsed since the last inline reap. This avoids O(n) Arc clones on
    /// every `spawn()`/`after()` call when many tasks are in the registry.
    pub(crate) fn try_reap_expired_tasks(&self) {
        let should_reap = {
            let last = match self.last_inline_reap.lock() {
                Ok(l) => *l,
                Err(_) => return,
            };
            last.elapsed() >= Duration::from_secs(10)
        };
        if should_reap {
            self.reap_expired_tasks();
            if let Ok(mut last) = self.last_inline_reap.lock() {
                *last = Instant::now();
            }
        }
    }

    /// Reap tasks in two phases:
    /// 1. Mark terminal tasks as `Expired` after 5 minutes (unless recently `try_await()`'d).
    /// 2. Remove `Expired` entries from the registry after the removal TTL (default 7 days,
    ///    configurable via `NTNT_TASK_REMOVAL_TTL` in seconds).
    ///
    /// Called on `spawn()` and `after()` entry.
    fn reap_expired_tasks(&self) {
        let now = Instant::now();
        let expiry = Duration::from_secs(300); // 5 minutes
        let recent_check_window = Duration::from_secs(300); // 5 minutes
        let removal_ttl = task_removal_ttl();

        // Step 1: Acquire registry lock → clone Arcs → drop lock
        let task_arcs: Vec<(u64, Arc<Mutex<TaskInner>>, Arc<Mutex<Option<Instant>>>)> = {
            let tasks = match self.tasks.lock() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("[WARN] Task registry mutex poisoned during reap: {}", e);
                    return;
                }
            };
            tasks
                .iter()
                .map(|(id, entry)| {
                    (
                        *id,
                        Arc::clone(&entry.inner),
                        Arc::clone(&entry.last_checked_at),
                    )
                })
                .collect()
        };
        // Registry lock is dropped here

        // Step 2: Inspect per-task state outside registry lock
        let mut ids_to_expire: HashSet<u64> = HashSet::new();
        let mut ids_to_remove: Vec<u64> = Vec::new();
        for (id, inner_arc, last_checked_at_arc) in &task_arcs {
            let (state, completed_at) = match inner_arc.lock() {
                Ok(inner) => (inner.state, inner.completed_at),
                Err(e) => {
                    eprintln!(
                        "[WARN] Task inner mutex poisoned during reap (task {}): {}",
                        id, e
                    );
                    continue;
                }
            };

            // Phase 2: Remove entries that have been Expired for longer than removal_ttl
            if state == TaskState::Expired {
                if let Some(completed) = completed_at {
                    if now.duration_since(completed) >= removal_ttl {
                        ids_to_remove.push(*id);
                    }
                }
                continue;
            }

            // Phase 1: Only expire tasks in terminal states (not Running or already Expired)
            if !matches!(
                state,
                TaskState::Completed
                    | TaskState::Failed
                    | TaskState::Panicked
                    | TaskState::Consumed
            ) {
                continue;
            }
            let Some(completed) = completed_at else {
                continue; // no completion time recorded — skip
            };
            if now.duration_since(completed) < expiry {
                continue; // not old enough — skip
            }
            // Check if recently try_await'd
            let last_checked = match last_checked_at_arc.lock() {
                Ok(l) => *l,
                Err(e) => {
                    eprintln!(
                        "[WARN] Task last_checked_at mutex poisoned during reap (task {}): {}",
                        id, e
                    );
                    continue;
                }
            };
            if let Some(checked) = last_checked {
                if now.duration_since(checked) < recent_check_window {
                    continue; // recently checked — skip
                }
            }
            ids_to_expire.insert(*id);
        }

        // Step 3: Mark expired tasks (using already-cloned inner Arcs — no registry lock needed)
        for (id, inner_arc, _) in &task_arcs {
            if ids_to_expire.contains(id) {
                match inner_arc.lock() {
                    Ok(mut inner) => {
                        inner.state = TaskState::Expired;
                    }
                    Err(e) => {
                        eprintln!(
                            "[WARN] Task inner mutex poisoned during expire (task {}): {}",
                            id, e
                        );
                    }
                }
            }
        }

        // Step 4: Remove long-expired entries from the registry to free memory
        if !ids_to_remove.is_empty() {
            if let Ok(mut tasks) = self.tasks.lock() {
                for id in &ids_to_remove {
                    tasks.remove(id);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Channel reaper — remove orphaned channels
    // -------------------------------------------------------------------------

    /// Reap channels where all senders have been dropped and no messages remain.
    /// This prevents memory leaks from channels where `close()` is never called.
    ///
    /// A channel is considered dead when:
    /// 1. `sender_probe.upgrade()` returns None (all TxChannelHandle clones dropped), AND
    /// 2. The crossbeam receiver is empty (no buffered messages to consume).
    fn reap_disconnected_channels(&self) {
        // Step 1: Lock registry, find candidates where sender_probe is dead
        let candidates: Vec<(u64, Arc<Mutex<crossbeam::Receiver<SerializedValue>>>)> = {
            let channels = match self.channels.lock() {
                Ok(c) => c,
                Err(_) => return,
            };
            channels
                .iter()
                .filter(|(_, entry)| entry.sender_probe.upgrade().is_none())
                .map(|(id, entry)| (*id, Arc::clone(&entry.receiver)))
                .collect()
        };
        // Registry lock dropped here

        if candidates.is_empty() {
            return;
        }

        // Step 2: Check if channels are fully drained (sender dead + no messages remaining)
        let mut ids_to_remove = Vec::new();
        for (id, receiver) in &candidates {
            if let Ok(rx) = receiver.lock() {
                if rx.is_empty() {
                    ids_to_remove.push(*id);
                }
            }
        }

        // Step 3: Remove dead channels from registry
        if !ids_to_remove.is_empty() {
            if let Ok(mut channels) = self.channels.lock() {
                for id in &ids_to_remove {
                    channels.remove(id);
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Channels
    // -------------------------------------------------------------------------

    /// Create a new channel. Returns (tx_value, rx_value) — the sender and receiver handles.
    /// The sender Arc lives entirely in TxChannelHandle; the registry holds only the receiver.
    fn create_channel(&self) -> Result<(Value, Value)> {
        let id = self.next_id();
        let (tx, rx) = crossbeam::unbounded::<SerializedValue>();
        // Create the type-erased sender Arc and a Weak probe before moving into the entry.
        // The Weak lets the channel reaper detect when all TxChannelHandle clones have dropped.
        let sender_arc: Arc<dyn std::any::Any + Send + Sync> = Arc::new(tx);
        let sender_probe = Arc::downgrade(&sender_arc);
        let entry = ChannelEntry {
            receiver: Arc::new(Mutex::new(rx)),
            sender_probe,
        };
        let mut channels = self
            .channels
            .lock()
            .map_err(|_| IntentError::runtime_error("Channel registry poisoned".to_string()))?;
        channels.insert(id, entry);
        drop(channels);
        let tx_val = Value::TxChannelHandle(id, crate::interpreter::ChannelSender(sender_arc));
        let rx_val = Value::RxChannelHandle(id);
        Ok((tx_val, rx_val))
    }

    /// Clone the receiver Arc from the channel registry.
    /// Returns None if channel_id is not in the registry (closed/removed).
    fn get_receiver_arc(
        &self,
        channel_id: u64,
    ) -> Result<Option<Arc<Mutex<crossbeam::Receiver<SerializedValue>>>>> {
        let channels = self
            .channels
            .lock()
            .map_err(|_| IntentError::runtime_error("Channel registry poisoned".to_string()))?;
        Ok(channels.get(&channel_id).map(|e| Arc::clone(&e.receiver)))
    }

    /// Lock a receiver Arc, returning the guard.
    fn lock_receiver(
        receiver: &Arc<Mutex<crossbeam::Receiver<SerializedValue>>>,
    ) -> Result<MutexGuard<'_, crossbeam::Receiver<SerializedValue>>> {
        receiver
            .lock()
            .map_err(|_| IntentError::runtime_error("Channel receiver poisoned".to_string()))
    }

    fn recv(&self, channel_id: u64) -> Result<Value> {
        check_cancellation()?;
        let Some(receiver) = self.get_receiver_arc(channel_id)? else {
            return Ok(Value::Unit);
        };
        let rx = Self::lock_receiver(&receiver)?;
        // Use 100ms timeout slices so recv() is a true cancellation yield point.
        // Without this, a cancelled task blocked on recv() would hang forever
        // if the sender never sends or drops.
        loop {
            check_cancellation()?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(serialized) => return Ok(serialized.to_value()),
                Err(crossbeam::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam::RecvTimeoutError::Disconnected) => return Ok(Value::Unit),
            }
        }
    }

    fn recv_timeout(&self, channel_id: u64, timeout_ms: i64) -> Result<Value> {
        check_cancellation()?;
        let total_ms = if timeout_ms < 0 { 0 } else { timeout_ms as u64 };
        let Some(receiver) = self.get_receiver_arc(channel_id)? else {
            return Ok(Value::none());
        };
        let rx = Self::lock_receiver(&receiver)?;
        let deadline = Instant::now() + Duration::from_millis(total_ms);
        loop {
            check_cancellation()?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(Value::none());
            }
            let slice = remaining.min(Duration::from_millis(100));
            match rx.recv_timeout(slice) {
                Ok(serialized) => return Ok(Value::some(serialized.to_value())),
                Err(crossbeam::RecvTimeoutError::Timeout) => continue,
                Err(crossbeam::RecvTimeoutError::Disconnected) => return Ok(Value::none()),
            }
        }
    }

    fn try_recv(&self, channel_id: u64) -> Result<Value> {
        let Some(receiver) = self.get_receiver_arc(channel_id)? else {
            return Ok(Value::none());
        };
        let rx = Self::lock_receiver(&receiver)?;
        match rx.try_recv() {
            Ok(serialized) => Ok(Value::some(serialized.to_value())),
            Err(crossbeam::TryRecvError::Empty) => Ok(Value::none()),
            Err(crossbeam::TryRecvError::Disconnected) => Ok(Value::none()),
        }
    }

    fn close_channel(&self, channel_id: u64) -> bool {
        // close() = remove receiver from map. recv(rx) on a removed channel_id immediately
        // returns Unit (id not found in registry).
        //
        // Note: if an in-flight recv/recv_timeout/select already cloned the receiver Arc,
        // the crossbeam Receiver stays alive until those clones drop. send(tx) may still
        // succeed for those in-flight operations. Once all clones are dropped, the Receiver
        // drops and send() returns false (Disconnected). This is eventual, not immediate.
        let mut channels = match self.channels.lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        channels.remove(&channel_id).is_some()
    }

    /// Get a cloned crossbeam Receiver for use in select().
    /// crossbeam Receivers are Clone, so this doesn't need the Mutex.
    fn get_receiver_clone(&self, channel_id: u64) -> Option<crossbeam::Receiver<SerializedValue>> {
        let channels = self.channels.lock().ok()?;
        let entry = channels.get(&channel_id)?;
        // Lock the receiver Mutex just to clone the underlying crossbeam Receiver
        let rx = entry.receiver.lock().ok()?;
        Some(rx.clone())
    }

    // -------------------------------------------------------------------------
    // Tasks
    // -------------------------------------------------------------------------

    /// Register a new task and return its ID. The caller must spawn the thread.
    pub(crate) fn register_task(&self, cancelled: Arc<CancelToken>) -> Result<u64> {
        let id = self.next_id();
        let entry = TaskEntry {
            inner: Arc::new(Mutex::new(TaskInner {
                state: TaskState::Running,
                result: None,
                error_msg: None,
                completed_at: None,
            })),
            cancelled,
            last_checked_at: Arc::new(Mutex::new(None)),
            completed_notify: Arc::new((Mutex::new(false), Condvar::new())),
        };
        let mut tasks = self
            .tasks
            .lock()
            .map_err(|_| IntentError::runtime_error("Task registry poisoned".to_string()))?;
        tasks.insert(id, entry);
        Ok(id)
    }

    /// Get cloned Arcs for a task's core state.
    /// Returns Ok(None) if the task doesn't exist, Err if the registry mutex is poisoned.
    pub(crate) fn get_task_arcs(&self, task_id: u64) -> Result<Option<TaskArcs>> {
        let tasks = self
            .tasks
            .lock()
            .map_err(|_| IntentError::runtime_error("Task registry poisoned".to_string()))?;
        Ok(tasks.get(&task_id).map(|entry| TaskArcs {
            inner: Arc::clone(&entry.inner),
            completed_notify: Arc::clone(&entry.completed_notify),
        }))
    }

    /// `await_task(handle)` — blocks until task completes, then marks as Consumed.
    /// Returns `Result`: `Ok(value)` or `Err(message)`.
    fn await_task(&self, task_id: u64) -> Result<Value> {
        let arcs = match self.get_task_arcs(task_id)? {
            Some(arcs) => arcs,
            None => {
                return Err(IntentError::runtime_error(
                    "Invalid task handle".to_string(),
                ))
            }
        };

        // Check for already-consumed or expired handles
        {
            let inner = arcs
                .inner
                .lock()
                .map_err(|_| IntentError::runtime_error("Task inner mutex poisoned".to_string()))?;
            match inner.state {
                TaskState::Consumed => {
                    return Err(IntentError::runtime_error(
                        "Task result already consumed by await_task".to_string(),
                    ))
                }
                TaskState::Expired => {
                    return Err(IntentError::runtime_error(
                        "Task handle expired (cleaned up after 5 minutes)".to_string(),
                    ))
                }
                _ => {}
            }
        }

        // Wait for terminal state using Condvar (50ms timeout for cancellation checks).
        //
        // Lock ordering safety: `inner` and `completed_notify` are never held simultaneously.
        // Each loop iteration: check inner (acquire+release), then wait on condvar
        // (acquire+release). This matches finalize_task(), which also acquires them
        // sequentially (inner first, then completed_notify), preventing ABBA deadlocks.
        let (lock, cvar) = &*arcs.completed_notify;
        loop {
            check_cancellation()?;
            {
                let inner = arcs.inner.lock().map_err(|_| {
                    IntentError::runtime_error("Task inner mutex poisoned".to_string())
                })?;
                if inner.state.is_terminal() {
                    break;
                }
            }
            // inner lock is dropped before acquiring notify lock — no nesting
            let guard = lock.lock().map_err(|_| {
                IntentError::runtime_error("Task notify mutex poisoned".to_string())
            })?;
            let (_guard, _) = cvar
                .wait_timeout(guard, Duration::from_millis(50))
                .map_err(|_| {
                    IntentError::runtime_error("Task notify mutex poisoned during wait".to_string())
                })?;
            // notify lock dropped at end of scope before next iteration checks inner
        }

        // Read result and mark as Consumed — single lock acquisition
        let mut inner = arcs
            .inner
            .lock()
            .map_err(|_| IntentError::runtime_error("Task inner mutex poisoned".to_string()))?;
        let result_value = match inner.state {
            TaskState::Completed => {
                let val = inner
                    .result
                    .as_ref()
                    .map(|s| s.to_value())
                    .unwrap_or(Value::Unit);
                Value::ok(val)
            }
            TaskState::Failed | TaskState::Panicked => {
                let msg = inner
                    .error_msg
                    .clone()
                    .unwrap_or_else(|| "Task failed".to_string());
                Value::err(Value::String(msg))
            }
            TaskState::Running => {
                unreachable!("await_task: task still Running after condvar loop")
            }
            // Another concurrent await_task call may have consumed the result
            // between our condvar wakeup and re-acquiring the inner lock.
            TaskState::Consumed => {
                return Err(IntentError::runtime_error(
                    "Task result already consumed by a concurrent await_task call".to_string(),
                ))
            }
            TaskState::Expired => {
                return Err(IntentError::runtime_error(
                    "Task handle expired while awaiting".to_string(),
                ))
            }
        };

        // Mark as Consumed (preserves handle for try_await)
        inner.state = TaskState::Consumed;

        Ok(result_value)
    }

    /// `try_await(handle)` — peek at task state without removing. Updates `last_checked_at`.
    /// Returns a map: `{ "status": "running"|"completed"|"failed"|"panicked"|"consumed"|"expired", "result": ... }`
    /// NEVER returns an error for a handle that existed — returns status map instead.
    fn try_await(&self, task_id: u64) -> Result<Value> {
        // Get arcs (inner + last_checked_at)
        let (inner_arc, last_checked_arc) = {
            let tasks = match self.tasks.lock() {
                Ok(t) => t,
                Err(_) => {
                    return Err(IntentError::runtime_error(
                        "Task registry poisoned".to_string(),
                    ))
                }
            };
            match tasks.get(&task_id) {
                Some(entry) => (Arc::clone(&entry.inner), Arc::clone(&entry.last_checked_at)),
                None => {
                    return Err(IntentError::runtime_error(
                        "Invalid task handle".to_string(),
                    ))
                }
            }
        };

        // Update last_checked_at (rule 13: prevents reaper from invalidating active handles)
        if let Ok(mut last_checked) = last_checked_arc.lock() {
            *last_checked = Some(Instant::now());
        }

        let inner = inner_arc
            .lock()
            .map_err(|_| IntentError::runtime_error("Task inner mutex poisoned".to_string()))?;
        let mut result_map = HashMap::new();

        let status_str = match inner.state {
            TaskState::Running => "running",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
            TaskState::Panicked => "panicked",
            TaskState::Consumed => "consumed",
            TaskState::Expired => "expired",
        };
        result_map.insert("status".to_string(), Value::String(status_str.to_string()));

        match inner.state {
            TaskState::Completed => {
                let val = inner
                    .result
                    .as_ref()
                    .map(|s| s.to_value())
                    .unwrap_or(Value::Unit);
                result_map.insert("result".to_string(), Value::ok(val));
            }
            TaskState::Failed | TaskState::Panicked => {
                let msg = inner
                    .error_msg
                    .clone()
                    .unwrap_or_else(|| "Task error".to_string());
                result_map.insert("result".to_string(), Value::err(Value::String(msg)));
            }
            TaskState::Running | TaskState::Consumed | TaskState::Expired => {
                result_map.insert("result".to_string(), Value::none());
            }
        }

        Ok(Value::Map(result_map))
    }

    /// `cancel_task(handle)` — sets the cancellation flag only (cooperative).
    /// Does NOT force state to Cancelled. The task thread checks the flag at yield points.
    fn cancel_task(&self, task_id: u64) -> Result<Value> {
        // Lock → clone cancelled Arc → drop → set flag
        let cancelled_arc = {
            let tasks = match self.tasks.lock() {
                Ok(t) => t,
                Err(_) => {
                    return Err(IntentError::runtime_error(
                        "Task registry poisoned".to_string(),
                    ))
                }
            };
            match tasks.get(&task_id) {
                Some(entry) => Arc::clone(&entry.cancelled),
                None => return Ok(Value::Bool(false)),
            }
        };
        cancelled_arc.cancel();
        Ok(Value::Bool(true))
    }

    // -------------------------------------------------------------------------
    // Schedules
    // -------------------------------------------------------------------------

    fn register_schedule(&self) -> Result<(u64, Arc<CancelToken>, Arc<AtomicBool>)> {
        let id = self.next_id();
        let cancelled = Arc::new(CancelToken::new());
        let tick_running = Arc::new(AtomicBool::new(false));
        let entry = ScheduleEntry {
            cancelled: Arc::clone(&cancelled),
        };
        let mut schedules = self
            .schedules
            .lock()
            .map_err(|_| IntentError::runtime_error("Schedule registry poisoned".to_string()))?;
        schedules.insert(id, entry);
        Ok((id, cancelled, tick_running))
    }

    fn cancel_schedule(&self, schedule_id: u64) -> bool {
        // Set flag AND remove from registry (rule 14)
        let mut schedules = match self.schedules.lock() {
            Ok(s) => s,
            Err(_) => return false,
        };
        if let Some(entry) = schedules.remove(&schedule_id) {
            entry.cancelled.cancel();
            true
        } else {
            false
        }
    }

    // -------------------------------------------------------------------------
    // Shutdown — cancel all tasks and schedules
    // -------------------------------------------------------------------------

    pub fn shutdown(&self) {
        // Cancel all tasks
        if let Ok(tasks) = self.tasks.lock() {
            for (_id, entry) in tasks.iter() {
                entry.cancelled.cancel();
            }
        }
        // Cancel all schedules and remove them
        if let Ok(mut schedules) = self.schedules.lock() {
            for (_id, entry) in schedules.iter() {
                entry.cancelled.cancel();
            }
            schedules.clear();
        }
        // Close all channels (remove them)
        if let Ok(mut channels) = self.channels.lock() {
            channels.clear();
        }
    }
}

/// The single global concurrency runtime.
pub static RUNTIME: LazyLock<ConcurrencyRuntime> = LazyLock::new(ConcurrencyRuntime::new);

/// Read the reaper interval from NTNT_TASK_REAP_INTERVAL env var (seconds), default 300s (5 min).
fn reap_interval() -> Duration {
    std::env::var("NTNT_TASK_REAP_INTERVAL")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(300))
}

/// Read the task removal TTL from NTNT_TASK_REMOVAL_TTL env var (seconds).
/// Expired task entries are removed from the registry after this duration.
/// Default: 604800s (7 days).
fn task_removal_ttl() -> Duration {
    std::env::var("NTNT_TASK_REMOVAL_TTL")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(604800)) // 7 days
}

/// Maximum number of concurrent active tasks.
/// Configurable via NTNT_MAX_TASKS env var. Default: 1024.
fn max_tasks() -> u64 {
    std::env::var("NTNT_MAX_TASKS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1024)
}

/// Starts a dedicated reaper thread that periodically cleans up expired tasks.
/// Spawned exactly once via `LazyLock`.
static REAPER_STARTED: LazyLock<()> = LazyLock::new(|| {
    let interval = reap_interval();
    thread::spawn(move || loop {
        thread::sleep(interval);
        RUNTIME.reap_expired_tasks();
        RUNTIME.reap_disconnected_channels();
    });
});

// =============================================================================
// Handle value helpers
// =============================================================================

#[cfg(test)]
fn create_handle_value(kind: &str, id: u64) -> Value {
    match kind {
        "Task" => Value::TaskHandle(id),
        "Schedule" => Value::ScheduleHandle(id),
        // Channel handles are created via RUNTIME.create_channel() which returns (tx, rx) pair.
        // TxChannelHandle requires a live sender Arc — can't be manufactured from id alone.
        _ => unreachable!("Unknown handle kind: {}", kind),
    }
}

fn get_handle_id(handle: &Value, expected_type: &str) -> Result<u64> {
    match (handle, expected_type) {
        (Value::TaskHandle(id), "Task") => Ok(*id),
        (Value::TxChannelHandle(id, _), "TxChannel") => Ok(*id),
        (Value::RxChannelHandle(id), "RxChannel") => Ok(*id),
        (Value::ScheduleHandle(id), "Schedule") => Ok(*id),
        // Wrong handle type — helpful error messages
        (Value::TaskHandle(_), _) => Err(IntentError::type_error(format!(
            "Expected a {} handle, got a Task handle",
            expected_type
        ))),
        (Value::TxChannelHandle(_, _), _) => Err(IntentError::type_error(format!(
            "Expected a {} handle, got a TxChannel handle",
            expected_type
        ))),
        (Value::RxChannelHandle(_), _) => Err(IntentError::type_error(format!(
            "Expected a {} handle, got a RxChannel handle",
            expected_type
        ))),
        (Value::ScheduleHandle(_), _) => Err(IntentError::type_error(format!(
            "Expected a {} handle, got a Schedule handle",
            expected_type
        ))),
        _ => Err(IntentError::type_error(format!(
            "Expected a {} handle, got {}",
            expected_type,
            handle.type_name()
        ))),
    }
}

// =============================================================================
// Interval parsing
// =============================================================================

/// Parse a human-readable interval string like "5s", "1m", "500ms" into Duration.
fn parse_interval(s: &str) -> Result<Duration> {
    let s = s.trim();
    let err = |s| IntentError::runtime_error(format!("Invalid interval: {}", s));

    if let Some(num_str) = s.strip_suffix("ms") {
        let num: u64 = num_str.trim().parse().map_err(|_| err(s))?;
        Ok(Duration::from_millis(num))
    } else if let Some(num_str) = s.strip_suffix('s') {
        let num: u64 = num_str.trim().parse().map_err(|_| err(s))?;
        Ok(Duration::from_secs(num))
    } else if let Some(num_str) = s.strip_suffix('m') {
        let num: u64 = num_str.trim().parse().map_err(|_| err(s))?;
        Ok(Duration::from_secs(num * 60))
    } else if let Some(num_str) = s.strip_suffix('h') {
        let num: u64 = num_str.trim().parse().map_err(|_| err(s))?;
        Ok(Duration::from_secs(num * 3600))
    } else {
        let num: u64 = s.parse().map_err(|_| {
            IntentError::runtime_error(format!(
                "Invalid interval: {}. Use '5s', '1m', '500ms', or '1h'",
                s
            ))
        })?;
        Ok(Duration::from_millis(num))
    }
}

// =============================================================================
// Capture helpers — serialize environment bindings for cross-thread use
// =============================================================================

/// Captured bindings for cross-thread transfer.
#[derive(Clone)]
struct CapturedBindings {
    /// Serializable values (Int, Float, Bool, String, Array, Map, Struct, Enum)
    values: HashMap<String, SerializedValue>,
    /// Full NativeFunction identities — reconstructed directly in the child interpreter.
    native_fns: Vec<CapturedNativeFn>,
}

/// Snapshot of a NativeFunction — enough to reconstruct Value::NativeFunction
/// without ambiguous name-based module lookup.
#[derive(Clone)]
struct CapturedNativeFn {
    binding_name: String, // the variable name in user code (may be an alias)
    fn_name: String,      // the canonical function name
    arity: usize,
    max_arity: usize,
    requires: Option<crate::interpreter::RuntimeCapability>,
    func: fn(&[Value]) -> Result<Value>,
}

/// Capture all bindings from an environment for cross-thread use.
/// Returns Err with a list of non-serializable closure names if any are found.
fn capture_bindings(
    bindings: &HashMap<String, Value>,
) -> std::result::Result<CapturedBindings, Vec<String>> {
    let mut values = HashMap::new();
    let mut native_fns = Vec::new();
    let mut non_serializable_closures = Vec::new();

    for (key, value) in bindings {
        match value {
            Value::NativeFunction {
                name,
                arity,
                max_arity,
                func,
                requires,
            } => {
                native_fns.push(CapturedNativeFn {
                    binding_name: key.clone(),
                    fn_name: name.clone(),
                    arity: *arity,
                    max_arity: *max_arity,
                    requires: *requires,
                    func: *func,
                });
            }
            _ => match SerializedValue::from_value(value) {
                Ok(serialized) => {
                    values.insert(key.clone(), serialized);
                }
                Err(_) => {
                    // User-defined closures (Value::Function) cannot cross task boundaries.
                    // Track them so we can fail with a clear error listing all problematic captures.
                    if matches!(value, Value::Function { .. }) {
                        non_serializable_closures.push(key.clone());
                    } else {
                        eprintln!(
                            "[WARN] Cannot capture '{}' for concurrent task: value type '{}' is not serializable",
                            key,
                            value.type_name()
                        );
                    }
                }
            },
        }
    }

    if !non_serializable_closures.is_empty() {
        return Err(non_serializable_closures);
    }

    Ok(CapturedBindings { values, native_fns })
}

/// Inject captured bindings into a fresh interpreter.
fn inject_captured(interp: &mut crate::interpreter::Interpreter, captured: &CapturedBindings) {
    for (key, val) in &captured.values {
        interp.define_global(key.clone(), val.to_value());
    }

    for cap in &captured.native_fns {
        interp.define_global(
            cap.binding_name.clone(),
            Value::NativeFunction {
                name: cap.fn_name.clone(),
                arity: cap.arity,
                max_arity: cap.max_arity,
                requires: cap.requires,
                func: cap.func,
            },
        );
    }
}

// =============================================================================
// Shared helpers for spawn/after/schedule
// =============================================================================

fn free_variables(body: &crate::ast::Block) -> HashSet<String> {
    let mut referenced = HashSet::new();
    let mut bound = HashSet::new();
    collect_free_vars(&body.statements, &mut referenced, &mut bound);
    referenced
}

fn collect_free_vars(
    statements: &[crate::ast::Statement],
    referenced: &mut HashSet<String>,
    bound: &mut HashSet<String>,
) {
    use crate::ast::{ServerDirective, Statement};

    for stmt in statements {
        match stmt {
            Statement::Let {
                name,
                value,
                pattern,
                otherwise,
                ..
            } => {
                if let Some(value) = value {
                    collect_free_vars_expr(value, referenced, bound);
                }

                if let Some(otherwise) = otherwise {
                    let mut otherwise_bound = bound.clone();
                    otherwise_bound.insert("err".to_string());
                    collect_free_vars(&otherwise.statements, referenced, &mut otherwise_bound);
                }

                if let Some(pattern) = pattern {
                    bound.extend(names_bound_by_pattern(pattern));
                } else {
                    bound.insert(name.clone());
                }
            }
            Statement::Function {
                name,
                params,
                body,
                contract,
                ..
            } => {
                bound.insert(name.clone());

                let mut fn_bound = bound.clone();
                bind_parameter_defaults_and_names(params, referenced, &mut fn_bound);

                if let Some(contract) = contract {
                    for expr in &contract.requires {
                        collect_free_vars_expr(expr, referenced, &fn_bound);
                    }
                    for expr in &contract.ensures {
                        collect_free_vars_expr(expr, referenced, &fn_bound);
                    }
                }

                let mut body_bound = fn_bound.clone();
                collect_free_vars(&body.statements, referenced, &mut body_bound);
            }
            Statement::Expression(expr) => collect_free_vars_expr(expr, referenced, bound),
            Statement::Return(Some(expr)) => collect_free_vars_expr(expr, referenced, bound),
            Statement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                collect_free_vars_expr(condition, referenced, bound);

                let mut then_bound = bound.clone();
                collect_free_vars(&then_branch.statements, referenced, &mut then_bound);

                if let Some(else_branch) = else_branch {
                    let mut else_bound = bound.clone();
                    collect_free_vars(&else_branch.statements, referenced, &mut else_bound);
                }
            }
            Statement::While { condition, body } => {
                collect_free_vars_expr(condition, referenced, bound);
                let mut body_bound = bound.clone();
                collect_free_vars(&body.statements, referenced, &mut body_bound);
            }
            Statement::ForIn {
                variable,
                pattern,
                iterable,
                body,
            } => {
                collect_free_vars_expr(iterable, referenced, bound);
                let mut body_bound = bound.clone();
                if let Some(pattern) = pattern {
                    body_bound.extend(names_bound_by_pattern(pattern));
                } else {
                    body_bound.insert(variable.clone());
                }
                collect_free_vars(&body.statements, referenced, &mut body_bound);
            }
            Statement::Loop { body } => {
                let mut body_bound = bound.clone();
                collect_free_vars(&body.statements, referenced, &mut body_bound);
            }
            Statement::Defer(expr) => collect_free_vars_expr(expr, referenced, bound),
            Statement::Intent { target, .. } => {
                collect_free_vars(std::slice::from_ref(target.as_ref()), referenced, bound);
            }
            Statement::Impl {
                methods,
                invariants,
                ..
            } => {
                for method in methods {
                    collect_free_vars(std::slice::from_ref(method), referenced, bound);
                }
                for invariant in invariants {
                    collect_free_vars_expr(invariant, referenced, bound);
                }
            }
            Statement::Module { body, .. } => collect_free_vars(body, referenced, bound),
            Statement::Export { statement, .. } => {
                if let Some(statement) = statement {
                    collect_free_vars(std::slice::from_ref(statement.as_ref()), referenced, bound);
                }
            }
            Statement::Import { items, alias, .. } => {
                if let Some(alias) = alias {
                    bound.insert(alias.clone());
                } else {
                    for item in items {
                        bound.insert(item.alias.clone().unwrap_or_else(|| item.name.clone()));
                    }
                }
            }
            Statement::Server {
                port,
                directives,
                routes,
                groups,
            } => {
                collect_free_vars_expr(port, referenced, bound);
                for directive in directives {
                    match directive {
                        ServerDirective::Cors(expr) | ServerDirective::Middleware(expr) => {
                            collect_free_vars_expr(expr, referenced, bound);
                        }
                        ServerDirective::Static { .. } => {}
                    }
                }
                for route in routes {
                    collect_free_vars_expr(&route.handler, referenced, bound);
                }
                collect_free_vars_server_groups(groups, referenced, bound);
            }
            Statement::Job {
                options,
                perform_params,
                perform_body,
                on_failure,
                ..
            } => {
                for (_, expr) in options {
                    collect_free_vars_expr(expr, referenced, bound);
                }

                let mut perform_bound = bound.clone();
                bind_parameter_defaults_and_names(perform_params, referenced, &mut perform_bound);
                let mut perform_body_bound = perform_bound.clone();
                collect_free_vars(
                    &perform_body.statements,
                    referenced,
                    &mut perform_body_bound,
                );

                if let Some((failure_params, failure_body)) = on_failure {
                    let mut failure_bound = bound.clone();
                    bind_parameter_defaults_and_names(
                        failure_params,
                        referenced,
                        &mut failure_bound,
                    );
                    let mut failure_body_bound = failure_bound.clone();
                    collect_free_vars(
                        &failure_body.statements,
                        referenced,
                        &mut failure_body_bound,
                    );
                }
            }
            Statement::Located { stmt, .. } => {
                collect_free_vars(std::slice::from_ref(stmt.as_ref()), referenced, bound);
            }
            Statement::Enum { name, variants, .. } => {
                bound.insert(name.clone());
                for variant in variants {
                    bound.insert(variant.name.clone());
                }
            }
            Statement::Return(None)
            | Statement::Break
            | Statement::Continue
            | Statement::Use { .. }
            | Statement::TypeAlias { .. }
            | Statement::Struct { .. }
            | Statement::Trait { .. } => {}
        }
    }
}

fn collect_free_vars_expr(
    expr: &crate::ast::Expression,
    referenced: &mut HashSet<String>,
    bound: &HashSet<String>,
) {
    use crate::ast::{Expression, StringPart};

    match expr {
        Expression::Identifier(name) => {
            if !bound.contains(name) {
                referenced.insert(name.clone());
            }
        }
        Expression::Call {
            function,
            arguments,
        } => {
            collect_free_vars_expr(function, referenced, bound);
            for arg in arguments {
                collect_free_vars_expr(arg, referenced, bound);
            }
        }
        Expression::MethodCall {
            object,
            method,
            arguments,
        } => {
            collect_free_vars_expr(object, referenced, bound);
            if !bound.contains(method) {
                referenced.insert(method.clone());
            }
            for arg in arguments {
                collect_free_vars_expr(arg, referenced, bound);
            }
        }
        Expression::Binary { left, right, .. } => {
            collect_free_vars_expr(left, referenced, bound);
            collect_free_vars_expr(right, referenced, bound);
        }
        Expression::Unary { operand, .. } => {
            collect_free_vars_expr(operand, referenced, bound);
        }
        Expression::FieldAccess { object, .. } => {
            collect_free_vars_expr(object, referenced, bound);
        }
        Expression::Index { object, index } => {
            collect_free_vars_expr(object, referenced, bound);
            collect_free_vars_expr(index, referenced, bound);
        }
        Expression::Array(items) => {
            for item in items {
                collect_free_vars_expr(item, referenced, bound);
            }
        }
        Expression::MapLiteral(pairs) => {
            for (key, value) in pairs {
                collect_free_vars_expr(key, referenced, bound);
                collect_free_vars_expr(value, referenced, bound);
            }
        }
        Expression::Range { start, end, .. } => {
            collect_free_vars_expr(start, referenced, bound);
            collect_free_vars_expr(end, referenced, bound);
        }
        Expression::InterpolatedString(parts) => {
            for part in parts {
                if let StringPart::Expr(expr) = part {
                    collect_free_vars_expr(expr, referenced, bound);
                }
            }
        }
        Expression::TemplateString(parts) => {
            collect_free_vars_template_parts(parts, referenced, bound);
        }
        Expression::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_free_vars_expr(value, referenced, bound);
            }
        }
        Expression::EnumVariant { arguments, .. } => {
            for arg in arguments {
                collect_free_vars_expr(arg, referenced, bound);
            }
        }
        Expression::Lambda { params, body } => {
            let mut lambda_bound = bound.clone();
            bind_parameter_defaults_and_names(params, referenced, &mut lambda_bound);
            let mut body_bound = lambda_bound.clone();
            collect_free_vars(&body.statements, referenced, &mut body_bound);
        }
        Expression::Block(block) => {
            let mut block_bound = bound.clone();
            collect_free_vars(&block.statements, referenced, &mut block_bound);
        }
        Expression::IfExpr {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_free_vars_expr(condition, referenced, bound);
            collect_free_vars_expr(then_branch, referenced, bound);
            collect_free_vars_expr(else_branch, referenced, bound);
        }
        Expression::Match { scrutinee, arms } => {
            collect_free_vars_expr(scrutinee, referenced, bound);
            for arm in arms {
                let mut arm_bound = bound.clone();
                arm_bound.extend(names_bound_by_pattern(&arm.pattern));
                if let Some(guard) = &arm.guard {
                    collect_free_vars_expr(guard, referenced, &arm_bound);
                }
                collect_free_vars_expr(&arm.body, referenced, &arm_bound);
            }
        }
        Expression::Assign { target, value } => {
            collect_free_vars_expr(target, referenced, bound);
            collect_free_vars_expr(value, referenced, bound);
        }
        Expression::Await(inner) | Expression::Try(inner) => {
            collect_free_vars_expr(inner, referenced, bound);
        }
        Expression::TryCatch { body } => {
            let mut body_bound = bound.clone();
            collect_free_vars(&body.statements, referenced, &mut body_bound);
        }
        Expression::Integer(_)
        | Expression::Float(_)
        | Expression::String(_)
        | Expression::Bool(_)
        | Expression::Unit => {}
    }
}

fn collect_free_vars_template_parts(
    parts: &[crate::ast::TemplatePart],
    referenced: &mut HashSet<String>,
    bound: &HashSet<String>,
) {
    use crate::ast::TemplatePart;

    for part in parts {
        match part {
            TemplatePart::Literal(_) => {}
            TemplatePart::Expr(expr) | TemplatePart::RawExpr(expr) => {
                collect_free_vars_expr(expr, referenced, bound);
            }
            TemplatePart::FilteredExpr { expr, filters }
            | TemplatePart::RawFilteredExpr { expr, filters } => {
                collect_free_vars_expr(expr, referenced, bound);
                for filter in filters {
                    for arg in &filter.args {
                        collect_free_vars_expr(arg, referenced, bound);
                    }
                }
            }
            TemplatePart::ForLoop {
                var,
                iterable,
                body,
                empty_body,
            } => {
                collect_free_vars_expr(iterable, referenced, bound);

                let mut body_bound = bound.clone();
                body_bound.insert(var.clone());
                for implicit in [
                    "@index", "@index1", "@first", "@last", "@length", "@even", "@odd",
                ] {
                    body_bound.insert(implicit.to_string());
                }
                collect_free_vars_template_parts(body, referenced, &body_bound);

                let empty_bound = bound.clone();
                collect_free_vars_template_parts(empty_body, referenced, &empty_bound);
            }
            TemplatePart::IfBlock {
                condition,
                then_parts,
                elif_chains,
                else_parts,
            } => {
                collect_free_vars_expr(condition, referenced, bound);
                collect_free_vars_template_parts(then_parts, referenced, bound);
                for (elif_cond, elif_body) in elif_chains {
                    collect_free_vars_expr(elif_cond, referenced, bound);
                    collect_free_vars_template_parts(elif_body, referenced, bound);
                }
                collect_free_vars_template_parts(else_parts, referenced, bound);
            }
            TemplatePart::Partial { data_expr, .. } => {
                if let Some(data_expr) = data_expr {
                    collect_free_vars_expr(data_expr, referenced, bound);
                }
            }
        }
    }
}

fn collect_free_vars_server_groups(
    groups: &[crate::ast::ServerGroup],
    referenced: &mut HashSet<String>,
    bound: &HashSet<String>,
) {
    for group in groups {
        for middleware in &group.middleware {
            collect_free_vars_expr(middleware, referenced, bound);
        }
        for route in &group.routes {
            collect_free_vars_expr(&route.handler, referenced, bound);
        }
        collect_free_vars_server_groups(&group.groups, referenced, bound);
    }
}

fn bind_parameter_defaults_and_names(
    params: &[crate::ast::Parameter],
    referenced: &mut HashSet<String>,
    bound: &mut HashSet<String>,
) {
    for param in params {
        if let Some(default) = &param.default {
            collect_free_vars_expr(default, referenced, bound);
        }
        bound.extend(names_bound_by_parameter(param));
    }
}

fn names_bound_by_parameter(param: &crate::ast::Parameter) -> HashSet<String> {
    if let Some(pattern) = &param.pattern {
        names_bound_by_pattern(pattern)
    } else {
        HashSet::from([param.name.clone()])
    }
}

fn names_bound_by_pattern(pattern: &crate::ast::Pattern) -> HashSet<String> {
    use crate::ast::Pattern;

    match pattern {
        Pattern::Variable(name) => HashSet::from([name.clone()]),
        Pattern::Wildcard | Pattern::Literal(_) => HashSet::new(),
        Pattern::Array { elements, rest } => {
            let mut names: HashSet<String> =
                elements.iter().flat_map(names_bound_by_pattern).collect();
            if let Some(rest) = rest {
                names.insert(rest.clone());
            }
            names
        }
        Pattern::Map { fields, rest } => {
            let mut names: HashSet<String> = fields
                .iter()
                .flat_map(|(_, pattern)| names_bound_by_pattern(pattern))
                .collect();
            if let Some(rest) = rest {
                names.insert(rest.clone());
            }
            names
        }
        Pattern::Struct { fields, .. } => fields
            .iter()
            .flat_map(|(_, pattern)| names_bound_by_pattern(pattern))
            .collect(),
        Pattern::Variant { fields, .. } => fields
            .as_ref()
            .map(|fields| fields.iter().flat_map(names_bound_by_pattern).collect())
            .unwrap_or_default(),
        Pattern::Tuple(patterns) => patterns.iter().flat_map(names_bound_by_pattern).collect(),
    }
}

/// Validate a handler is a zero-parameter Function, capture its bindings, and return
/// the captured bindings + cloned body. Used by spawn(), after(), and schedule().
fn validate_and_capture(
    caller: &str,
    handler: &Value,
) -> Result<(CapturedBindings, crate::ast::Block)> {
    match handler {
        Value::Function {
            params,
            closure,
            body,
            ..
        } => {
            if !params.is_empty() {
                return Err(IntentError::runtime_error(format!(
                    "{}() handler must be a zero-parameter function",
                    caller
                )));
            }
            let free_vars = free_variables(body);
            let needed: HashMap<String, Value> = closure
                .borrow()
                .all_bindings()
                .into_iter()
                .filter(|(name, _)| free_vars.contains(name))
                .collect();
            let captured = capture_bindings(&needed).map_err(|names| {
                IntentError::runtime_error(format!(
                    "Cannot capture user-defined function(s) across task boundaries: {}. \
                     These functions use Rc<RefCell> which cannot be sent between threads. \
                     Inline the function body into the closure, or call a native function instead.",
                    names.join(", ")
                ))
            })?;
            Ok((captured, body.clone()))
        }
        _ => Err(IntentError::type_error(format!(
            "{}() requires a function",
            caller
        ))),
    }
}

/// Process the result of a catch_unwind(eval_block) and update task state atomically.
/// Shared by spawn() and after() thread bodies.
///
/// Locks `inner` ONCE and sets all fields (state, result/error_msg, completed_at) atomically.
/// This eliminates the possibility of inconsistent state (e.g., result stored but state not updated).
///
/// Lock ordering: acquires `inner` first, releases it, THEN acquires `completed_notify`.
/// This matches await_task() which also acquires them sequentially (inner, then notify).
/// Neither function holds both locks simultaneously — no ABBA deadlock possible.
pub(crate) fn finalize_task(
    result: std::result::Result<Result<Value>, Box<dyn std::any::Any + Send>>,
    inner_arc: &Arc<Mutex<TaskInner>>,
    completed_notify: &Arc<(Mutex<bool>, Condvar)>,
) {
    match inner_arc.lock() {
        Ok(mut inner) => {
            match result {
                Ok(Ok(value)) => match SerializedValue::from_value(&value) {
                    Ok(serialized) => {
                        inner.result = Some(serialized);
                        inner.state = TaskState::Completed;
                    }
                    Err(_) => {
                        inner.error_msg = Some(format!(
                                "Task returned a non-serializable value ({}). \
                                 Only Int, Float, Bool, String, Array, Map, Struct, Enum can cross task boundaries.",
                                value.type_name()
                            ));
                        inner.state = TaskState::Failed;
                    }
                },
                Ok(Err(e)) => {
                    inner.error_msg = Some(format!("{}", e));
                    inner.state = TaskState::Failed;
                }
                Err(panic_info) => {
                    let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                        s.clone()
                    } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                        s.to_string()
                    } else {
                        "Task panicked".to_string()
                    };
                    inner.error_msg = Some(msg);
                    inner.state = TaskState::Panicked;
                }
            }
            inner.completed_at = Some(Instant::now());
        }
        Err(_) => {
            // Even on poisoned mutex, we MUST still notify and decrement below.
            // Without notification, await_task() would hang forever on the condvar.
            eprintln!("[WARN] finalize_task: inner mutex poisoned, cannot update task state");
        }
    }
    // Always notify waiting await_task() calls, even if inner mutex was poisoned.
    // This prevents await_task from blocking forever on a poisoned task.
    if let Ok(mut done) = completed_notify.0.lock() {
        *done = true;
        completed_notify.1.notify_all();
    } else {
        eprintln!("[WARN] finalize_task: notify mutex poisoned, cannot wake await_task waiters");
    }
    // Always decrement active task counter
    RUNTIME.active_tasks.fetch_sub(1, AtomicOrdering::Release);
    CURRENT_CANCEL_TOKEN.with(|cell| {
        *cell.borrow_mut() = None;
    });
}

/// Run captured bindings in a fresh interpreter. Used inside catch_unwind in task threads.
fn run_in_fresh_interpreter(
    captured: &CapturedBindings,
    body: &crate::ast::Block,
) -> Result<Value> {
    let mut interp = crate::interpreter::Interpreter::new();
    inject_captured(&mut interp, captured);
    interp.eval_block(body)
}

// =============================================================================
// Public API functions (called from NativeFunction dispatch)
// =============================================================================

// --- Channels ---

fn concurrent_channel() -> Result<Value> {
    let (tx, rx) = RUNTIME.create_channel()?;
    Ok(Value::Array(vec![tx, rx]))
}

fn concurrent_send(ch: &Value, value: &Value) -> Result<Value> {
    // Only TxChannelHandle has the sender capability
    let sender_arc = match ch {
        Value::TxChannelHandle(_, cs) => cs
            .0
            .downcast_ref::<crossbeam::Sender<SerializedValue>>()
            .ok_or_else(|| {
                IntentError::runtime_error(
                    "Internal: TxChannelHandle contains unexpected sender type".to_string(),
                )
            })?,
        Value::RxChannelHandle(_) => {
            return Err(IntentError::type_error(
                "send() requires a TxChannel handle (the first element of channel()). Got RxChannel — you may have passed the wrong end of the channel.".to_string(),
            ))
        }
        other => {
            return Err(IntentError::type_error(format!(
                "send() requires a TxChannel handle, got {}",
                other.type_name()
            )))
        }
    };
    // Handles cannot be sent through channels (they're process-local references)
    if matches!(
        value,
        Value::TaskHandle(_)
            | Value::TxChannelHandle(_, _)
            | Value::RxChannelHandle(_)
            | Value::ScheduleHandle(_)
    ) {
        return Err(IntentError::type_error(
            "Handles (Task, TxChannel, RxChannel, Schedule) cannot be sent through channels"
                .to_string(),
        ));
    }
    let serialized = SerializedValue::from_value(value)?;
    Ok(Value::Bool(sender_arc.send(serialized).is_ok()))
}

fn concurrent_recv(ch: &Value) -> Result<Value> {
    let id = get_handle_id(ch, "RxChannel")?;
    RUNTIME.recv(id)
}

fn concurrent_recv_timeout(ch: &Value, timeout_ms: i64) -> Result<Value> {
    let id = get_handle_id(ch, "RxChannel")?;
    RUNTIME.recv_timeout(id, timeout_ms)
}

fn concurrent_try_recv(ch: &Value) -> Result<Value> {
    let id = get_handle_id(ch, "RxChannel")?;
    RUNTIME.try_recv(id)
}

fn concurrent_close(ch: &Value) -> Result<Value> {
    let id = get_handle_id(ch, "RxChannel")?;
    Ok(Value::Bool(RUNTIME.close_channel(id)))
}

// --- Tasks ---

pub(crate) fn check_task_limit() -> Result<()> {
    let active = RUNTIME.active_tasks.load(AtomicOrdering::Acquire);
    let limit = max_tasks();
    if active >= limit {
        return Err(IntentError::runtime_error(format!(
            "Maximum concurrent task limit reached ({}). \
             Set NTNT_MAX_TASKS to increase the limit.",
            limit
        )));
    }
    Ok(())
}

fn concurrent_spawn(handler: &Value) -> Result<Value> {
    RUNTIME.try_reap_expired_tasks();
    check_task_limit()?;
    let (captured, body) = validate_and_capture("spawn", handler)?;

    let cancelled = Arc::new(CancelToken::new());
    let task_id = RUNTIME.register_task(Arc::clone(&cancelled))?;
    RUNTIME.active_tasks.fetch_add(1, AtomicOrdering::Release);
    // Safe: task_id was just returned by register_task(), so it must exist in the registry
    let arcs = RUNTIME
        .get_task_arcs(task_id)?
        .expect("task just registered must exist");

    thread::spawn(move || {
        CURRENT_CANCEL_TOKEN.with(|cell| {
            *cell.borrow_mut() = Some(cancelled);
        });
        let result = catch_unwind(AssertUnwindSafe(|| {
            run_in_fresh_interpreter(&captured, &body)
        }));
        finalize_task(result, &arcs.inner, &arcs.completed_notify);
    });

    Ok(Value::TaskHandle(task_id))
}

fn concurrent_await_task(handle: &Value) -> Result<Value> {
    let id = get_handle_id(handle, "Task")?;
    RUNTIME.await_task(id)
}

fn concurrent_try_await(handle: &Value) -> Result<Value> {
    let id = get_handle_id(handle, "Task")?;
    RUNTIME.try_await(id)
}

fn concurrent_cancel_task(handle: &Value) -> Result<Value> {
    let id = get_handle_id(handle, "Task")?;
    RUNTIME.cancel_task(id)
}

/// Extract the inner Err from Ok(Err(msg)) → Some(Err(msg)), else None
fn extract_inner_err(val: &Value) -> Option<Value> {
    if let Value::EnumValue {
        enum_name,
        variant,
        values,
    } = val
    {
        if enum_name == "Result" && variant == "Ok" {
            if let Some(inner) = values.first() {
                if matches!(inner, Value::EnumValue { enum_name, variant, .. }
                    if enum_name == "Result" && variant == "Err")
                {
                    return Some(inner.clone());
                }
            }
        }
    }
    None
}

/// Check if a task result represents a failure — either a crash (outer Err from await_task)
/// or a returned error (Ok(Err(...)) from a task that completed but returned an Err value).
/// This matches user expectations: parallel cancels if fetch() fails, not just if the task panics.
fn is_task_failure(val: &Value) -> bool {
    match val {
        // Outer Err: task crashed/panicked
        Value::EnumValue {
            enum_name, variant, ..
        } if enum_name == "Result" && variant == "Err" => true,
        // Ok(Err(...)): task completed but returned an error
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Result" && variant == "Ok" => values.first().map_or(false, |inner| {
            matches!(inner, Value::EnumValue { enum_name, variant, .. }
                if enum_name == "Result" && variant == "Err")
        }),
        _ => false,
    }
}

// --- parallel/race ---

fn concurrent_parallel(fns_val: &Value) -> Result<Value> {
    let fns = match fns_val {
        Value::Array(arr) => arr,
        _ => {
            return Err(IntentError::type_error(
                "parallel() requires an array of functions".to_string(),
            ))
        }
    };

    if fns.is_empty() {
        return Ok(Value::Array(vec![]));
    }

    // Spawn all — cancel already-spawned if a later spawn fails
    let mut handles = Vec::with_capacity(fns.len());
    for f in fns {
        match concurrent_spawn(f) {
            Ok(handle) => handles.push(handle),
            Err(e) => {
                for h in &handles {
                    let _ = concurrent_cancel_task(h);
                }
                return Err(e);
            }
        }
    }

    // Await all in order — cancel siblings on any error (Rust-level or ntnt-level)
    let mut results = Vec::with_capacity(handles.len());
    for handle in &handles {
        match concurrent_await_task(handle) {
            Ok(result) => {
                if is_task_failure(&result) {
                    for h in &handles {
                        let _ = concurrent_cancel_task(h);
                    }
                    // Extract the inner error for a clean return
                    // Ok(Err(msg)) → return Err(msg), outer Err → return as-is
                    let err_val = extract_inner_err(&result).unwrap_or(result);
                    return Ok(err_val);
                }
                results.push(result);
            }
            Err(e) => {
                // Rust-level error (e.g. parent cancellation) — cancel all children
                for h in &handles {
                    let _ = concurrent_cancel_task(h);
                }
                return Err(e);
            }
        }
    }

    Ok(Value::Array(results))
}

fn concurrent_race(fns_val: &Value) -> Result<Value> {
    let fns = match fns_val {
        Value::Array(arr) => arr,
        _ => {
            return Err(IntentError::type_error(
                "race() requires an array of functions".to_string(),
            ))
        }
    };

    if fns.is_empty() {
        return Err(IntentError::runtime_error(
            "race() requires at least one function".to_string(),
        ));
    }

    // Spawn all — cancel already-spawned if a later spawn fails
    let mut handles = Vec::with_capacity(fns.len());
    for f in fns {
        match concurrent_spawn(f) {
            Ok(handle) => handles.push(handle),
            Err(e) => {
                for h in &handles {
                    let _ = concurrent_cancel_task(h);
                }
                return Err(e);
            }
        }
    }

    let cancel_all = |handles: &[Value]| {
        for h in handles {
            let _ = concurrent_cancel_task(h);
        }
    };

    let mut done = vec![false; handles.len()];
    let mut last_error: Option<Value> = None;

    loop {
        if let Err(e) = check_cancellation() {
            cancel_all(&handles);
            return Err(e);
        }

        for (i, handle) in handles.iter().enumerate() {
            if done[i] {
                continue;
            }

            let status_map = match concurrent_try_await(handle) {
                Ok(m) => m,
                Err(e) => {
                    cancel_all(&handles);
                    return Err(e);
                }
            };
            let (status, result) = match &status_map {
                Value::Map(m) => {
                    let status = match m.get("status") {
                        Some(Value::String(s)) => s.as_str(),
                        _ => "",
                    };
                    let result = m.get("result");
                    (status, result)
                }
                _ => ("", None),
            };

            match status {
                "completed" => {
                    // Get the actual result
                    let await_result = concurrent_await_task(handle);
                    match &await_result {
                        Ok(val) if is_task_failure(val) => {
                            // Task completed but returned Err — not a winner, treat as failure
                            done[i] = true;
                            last_error =
                                Some(extract_inner_err(val).unwrap_or_else(|| val.clone()));
                        }
                        _ => {
                            // Real success (or Rust-level error) — cancel all and return
                            cancel_all(&handles);
                            return await_result;
                        }
                    }
                }
                "failed" | "panicked" => {
                    done[i] = true;
                    if let Some(val) = result {
                        last_error = Some(val.clone());
                    }
                }
                "consumed" | "expired" => {
                    done[i] = true;
                }
                _ => {} // running
            }
        }

        if done.iter().all(|d| *d) {
            return Ok(last_error.unwrap_or_else(|| {
                Value::err(Value::String("All race participants failed".to_string()))
            }));
        }

        if let Err(e) = concurrent_sleep_ms(5) {
            cancel_all(&handles);
            return Err(e);
        }
    }
}

// --- sleep_ms (cancellation-aware) ---

/// sleep_ms(ms) — cancellation-aware sleep. Uses CancelToken::wait_timeout for instant wakeup.
fn concurrent_sleep_ms(ms: i64) -> Result<Value> {
    if ms <= 0 {
        return Ok(Value::Unit);
    }
    check_cancellation()?;
    if sleep_cancellable(Duration::from_millis(ms as u64)) {
        return Err(IntentError::runtime_error("Task cancelled".to_string()));
    }
    Ok(Value::Unit)
}

// --- after(delay, handler) ---

fn concurrent_after(delay: &Value, handler: &Value) -> Result<Value> {
    RUNTIME.try_reap_expired_tasks();
    check_task_limit()?;

    let delay_duration = match delay {
        Value::Int(ms) => {
            if *ms <= 0 {
                return Err(IntentError::runtime_error(
                    "after() delay must be positive".to_string(),
                ));
            }
            Duration::from_millis(*ms as u64)
        }
        Value::String(s) => parse_interval(s)?,
        _ => {
            return Err(IntentError::type_error(
                "after() delay must be an Int (milliseconds) or a String interval (e.g. '5s')"
                    .to_string(),
            ))
        }
    };

    let (captured, body) = validate_and_capture("after", handler)?;

    let cancelled = Arc::new(CancelToken::new());
    let task_id = RUNTIME.register_task(Arc::clone(&cancelled))?;
    RUNTIME.active_tasks.fetch_add(1, AtomicOrdering::Release);
    // Safe: task_id was just returned by register_task(), so it must exist in the registry
    let arcs = RUNTIME
        .get_task_arcs(task_id)?
        .expect("task just registered must exist");

    thread::spawn(move || {
        CURRENT_CANCEL_TOKEN.with(|cell| {
            *cell.borrow_mut() = Some(Arc::clone(&cancelled));
        });

        // Cancellation-aware delay — instant wakeup via CancelToken::wait_timeout
        let cancelled_during_delay = sleep_cancellable(delay_duration);

        let result = if cancelled_during_delay {
            Ok(Err(IntentError::runtime_error(
                "Task cancelled".to_string(),
            )))
        } else {
            catch_unwind(AssertUnwindSafe(|| {
                run_in_fresh_interpreter(&captured, &body)
            }))
        };
        finalize_task(result, &arcs.inner, &arcs.completed_notify);
    });

    Ok(Value::TaskHandle(task_id))
}

// --- schedule(interval, handler) ---

fn concurrent_schedule(interval: &Value, handler: &Value) -> Result<Value> {
    let interval_duration = match interval {
        Value::Int(ms) => {
            if *ms <= 0 {
                return Err(IntentError::runtime_error(
                    "schedule() interval must be positive and non-zero".to_string(),
                ));
            }
            Duration::from_millis(*ms as u64)
        }
        Value::String(s) => {
            let d = parse_interval(s)?;
            if d.is_zero() {
                return Err(IntentError::runtime_error(
                    "schedule() interval must be non-zero".to_string(),
                ));
            }
            d
        }
        _ => {
            return Err(IntentError::type_error(
                "schedule() interval must be an Int (milliseconds) or a String (e.g. '5s')"
                    .to_string(),
            ))
        }
    };

    let (captured, body) = validate_and_capture("schedule", handler)?;
    let (schedule_id, cancelled, tick_running) = RUNTIME.register_schedule()?;

    thread::spawn(move || {
        // Set cancel token so sleep_cancellable() works in this thread.
        CURRENT_CANCEL_TOKEN.with(|cell| {
            *cell.borrow_mut() = Some(Arc::clone(&cancelled));
        });

        loop {
            // Cancellation-aware interval sleep — instant wakeup via CancelToken::wait_timeout
            if sleep_cancellable(interval_duration) {
                return;
            }

            // Overlap prevention: skip if previous tick still running
            if tick_running
                .compare_exchange(false, true, AtomicOrdering::AcqRel, AtomicOrdering::Acquire)
                .is_err()
            {
                continue;
            }

            let tick_captured = captured.clone();
            let tick_body = body.clone();
            let tick_running_clone = Arc::clone(&tick_running);
            let tick_cancelled = Arc::clone(&cancelled);

            thread::spawn(move || {
                // Install cancel token so yield points (fetch, sleep_ms, recv)
                // in tick bodies respect schedule cancellation.
                CURRENT_CANCEL_TOKEN.with(|cell| {
                    *cell.borrow_mut() = Some(tick_cancelled);
                });
                let _result = catch_unwind(AssertUnwindSafe(|| {
                    let _ = run_in_fresh_interpreter(&tick_captured, &tick_body);
                }));
                CURRENT_CANCEL_TOKEN.with(|cell| {
                    *cell.borrow_mut() = None;
                });
                tick_running_clone.store(false, AtomicOrdering::Release);
            });
        }
    });

    Ok(Value::ScheduleHandle(schedule_id))
}

fn concurrent_cancel_schedule(handle: &Value) -> Result<Value> {
    let id = get_handle_id(handle, "Schedule")?;
    Ok(Value::Bool(RUNTIME.cancel_schedule(id)))
}

// --- select(channels, timeout_ms?) ---

/// `select(channels, timeout_ms?)` — wait for the first value from any of the given channels.
/// Returns `{status: "ok", channel: <handle>, value: <received>}` on success,
/// `{status: "timeout"}` on timeout, `{status: "closed"}` if all channels are closed.
/// All return shapes include a `status` key for consistent pattern matching.
/// This is a cancellation yield point.
fn concurrent_select(args: &[Value]) -> Result<Value> {
    // Yield point: check cancellation
    check_cancellation()?;

    // Parse arguments: first is Array of ChannelHandles, second is optional timeout
    if args.is_empty() {
        return Err(IntentError::runtime_error(
            "select() requires at least one argument: an array of channel handles".to_string(),
        ));
    }

    let channels_arr = match &args[0] {
        Value::Array(arr) => arr,
        _ => {
            return Err(IntentError::type_error(
                "select() first argument must be an array of channel handles".to_string(),
            ))
        }
    };

    if channels_arr.is_empty() {
        return Err(IntentError::runtime_error(
            "select() requires at least one channel".to_string(),
        ));
    }

    // Extract channel IDs and get receiver clones
    let mut channel_ids = Vec::with_capacity(channels_arr.len());
    let mut receivers = Vec::with_capacity(channels_arr.len());

    for (i, ch_val) in channels_arr.iter().enumerate() {
        let id = match ch_val {
            Value::RxChannelHandle(id) => *id,
            Value::TxChannelHandle(_, _) => {
                return Err(IntentError::type_error(format!(
                    "select() channel at index {} is a TxChannel (sender). Pass RxChannel handles (the second element of channel()) to select().",
                    i
                )))
            }
            _ => {
                return Err(IntentError::type_error(format!(
                    "select() channel at index {} is not an RxChannel handle, got {}",
                    i,
                    ch_val.type_name()
                )))
            }
        };
        match RUNTIME.get_receiver_clone(id) {
            Some(rx) => {
                channel_ids.push(id);
                receivers.push(rx);
            }
            None => {
                // Channel already closed/removed — skip it
                // If all channels are closed, we'll detect that below
            }
        }
    }

    if receivers.is_empty() {
        // All channels are closed
        let mut result = HashMap::new();
        result.insert("status".to_string(), Value::String("closed".to_string()));
        return Ok(Value::Map(result));
    }

    // Parse optional timeout
    let timeout = if args.len() > 1 {
        match &args[1] {
            Value::Int(ms) => {
                if *ms <= 0 {
                    Some(Duration::from_millis(0))
                } else {
                    Some(Duration::from_millis(*ms as u64))
                }
            }
            Value::String(s) => Some(parse_interval(s)?),
            _ => {
                return Err(IntentError::type_error(
                    "select() timeout must be an Int (milliseconds) or a String interval"
                        .to_string(),
                ))
            }
        }
    } else {
        None // No timeout — block indefinitely
    };

    // Track which channels are still alive (not disconnected)
    let mut alive = vec![true; receivers.len()];

    // Wait with optional timeout, using 100ms slices for cancellation checks
    let deadline = timeout.map(|t| Instant::now() + t);

    loop {
        check_cancellation()?;

        // Check if all channels are dead
        if alive.iter().all(|a| !a) {
            let mut result = HashMap::new();
            result.insert("status".to_string(), Value::String("closed".to_string()));
            return Ok(Value::Map(result));
        }

        let remaining = match deadline {
            Some(dl) => {
                let rem = dl.saturating_duration_since(Instant::now());
                if rem.is_zero() {
                    // Timeout expired
                    let mut result = HashMap::new();
                    result.insert("status".to_string(), Value::String("timeout".to_string()));
                    return Ok(Value::Map(result));
                }
                rem.min(Duration::from_millis(100))
            }
            None => Duration::from_millis(100), // Check cancellation every 100ms
        };

        // Rebuild Select each iteration, skipping dead channels
        // Maps select index back to original receiver index
        let mut sel = crossbeam::Select::new();
        let mut sel_to_orig: Vec<usize> = Vec::new();
        for (i, rx) in receivers.iter().enumerate() {
            if alive[i] {
                sel.recv(rx);
                sel_to_orig.push(i);
            }
        }

        match sel.ready_timeout(remaining) {
            Ok(sel_index) => {
                let orig_index = sel_to_orig[sel_index];
                // A receiver is ready — try to receive from it
                match receivers[orig_index].try_recv() {
                    Ok(serialized) => {
                        let mut result = HashMap::new();
                        result.insert("status".to_string(), Value::String("ok".to_string()));
                        result.insert(
                            "channel".to_string(),
                            Value::RxChannelHandle(channel_ids[orig_index]),
                        );
                        result.insert("value".to_string(), serialized.to_value());
                        return Ok(Value::Map(result));
                    }
                    Err(crossbeam::TryRecvError::Empty) => {
                        // Spurious wakeup — retry
                        continue;
                    }
                    Err(crossbeam::TryRecvError::Disconnected) => {
                        // Mark this channel as dead so we skip it in future iterations
                        alive[orig_index] = false;
                        continue;
                    }
                }
            }
            Err(crossbeam::ReadyTimeoutError) => {
                // Timeout on this slice — loop back to check cancellation/deadline
                continue;
            }
        }
    }
}

// --- thread_count ---

fn concurrent_thread_count() -> Result<Value> {
    Ok(Value::Int(
        thread::available_parallelism()
            .map(|n| n.get() as i64)
            .unwrap_or(1),
    ))
}

// =============================================================================
// Module initialization
// =============================================================================

pub fn init() -> HashMap<String, Value> {
    // Start the periodic reaper thread (runs exactly once via LazyLock)
    LazyLock::force(&REAPER_STARTED);

    let mut module = HashMap::new();

    // @ntnt channel
    // @module std/concurrent
    // @module_description Structured concurrency: tasks, channels, schedules, and cooperative cancellation
    // @signature channel() -> [TxChannel, RxChannel]
    // Creates a new unbounded channel and returns a [sender, receiver] pair.
    //
    // The sender (TxChannel) and receiver (RxChannel) are separate handles —
    // exactly like Rust's own channels. Pass the TxChannel to whoever should
    // send; keep (or pass) the RxChannel to whoever should recv.
    //
    // Ownership semantics: when ALL TxChannel clones for a channel are dropped
    // (e.g. a spawned task exits before or after calling send()), the receiver
    // automatically sees Disconnected and recv() returns Unit. No sentinel
    // injection required — this is structural, not approximate.
    //
    // Channels are single-consumer: only one task should call recv() at a time.
    // @returns Array containing [TxChannel, RxChannel]
    // @see_also send, recv, close, select
    // @since v0.4.6
    // @example let [tx, rx] = channel() ~ "Create a channel for inter-task communication"
    // @example ~ "Pass tx to a spawned task; recv on rx disconnects naturally if task fails"
    //   let [tx, rx] = channel()
    //   let task = spawn(fn() { send(tx, "hello") })
    //   let msg = recv(rx)
    // @expected "hello"
    module.insert(
        "channel".to_string(),
        Value::NativeFunction {
            name: "channel".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| concurrent_channel(),
        },
    );

    // @ntnt send
    // @module std/concurrent
    // @signature send(tx: TxChannel, value: Any) -> Bool
    // Sends a value through a channel using the sender handle (first element of channel()).
    // Returns false if the receiver has been closed (crossbeam Disconnected).
    // Serializable types: Int, Float, Bool, String, Array, Map, Struct, Enum.
    // @param tx The TxChannel sender handle (first element of channel())
    // @param value The value to send (must be serializable)
    // @see_also channel, recv, recv_timeout
    // @since v0.4.6
    // @example send(tx, "hello") => true ~ "Send a string through the channel"
    module.insert(
        "send".to_string(),
        Value::NativeFunction {
            name: "send".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| concurrent_send(&args[0], &args[1]),
        },
    );

    // @ntnt recv
    // @module std/concurrent
    // @signature recv(rx: RxChannel) -> Any
    // Receives a value from a channel. Blocks until a value is available.
    // Returns Unit if all senders have been dropped (Disconnected) or the receiver was closed.
    // This is a cancellation yield point: a cancelled task will exit here.
    // Single-consumer: the receiver lock is held for the blocking duration.
    // @param rx The RxChannel receiver handle (second element of channel())
    // @see_also channel, send, try_recv, recv_timeout
    // @since v0.4.6
    // @example let [tx, rx] = channel() ~ "Block until a value is received"
    // @example recv(rx)
    module.insert(
        "recv".to_string(),
        Value::NativeFunction {
            name: "recv".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| concurrent_recv(&args[0]),
        },
    );

    // @ntnt recv_timeout
    // @module std/concurrent
    // @signature recv_timeout(rx: RxChannel, millis: Int) -> Option<Any>
    // Receives with timeout. Returns None if timeout expires or all senders disconnected.
    // Loops in ≤100ms slices checking cancellation between iterations.
    // This is a cancellation yield point.
    // @param rx The RxChannel receiver handle (second element of channel())
    // @param millis Timeout in milliseconds (negative values clamped to 0)
    // @see_also recv, try_recv
    // @since v0.4.6
    // @example let [tx, rx] = channel() ~ "Wait up to 5 seconds for a value"
    // @example recv_timeout(rx, 5000)
    module.insert(
        "recv_timeout".to_string(),
        Value::NativeFunction {
            name: "recv_timeout".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match &args[1] {
                Value::Int(ms) => concurrent_recv_timeout(&args[0], *ms),
                _ => Err(IntentError::type_error(
                    "recv_timeout requires (channel, int_millis)".to_string(),
                )),
            },
        },
    );

    // @ntnt try_recv
    // @module std/concurrent
    // @signature try_recv(rx: RxChannel) -> Option<Any>
    // Non-blocking receive. Returns None if no value is available or all senders disconnected.
    // @param rx The RxChannel receiver handle (second element of channel())
    // @see_also recv, recv_timeout
    // @since v0.4.6
    // @example let [tx, rx] = channel() ~ "Check for a value without blocking"
    // @example try_recv(rx)
    module.insert(
        "try_recv".to_string(),
        Value::NativeFunction {
            name: "try_recv".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| concurrent_try_recv(&args[0]),
        },
    );

    // @ntnt close
    // @module std/concurrent
    // @signature close(rx: RxChannel) -> Bool
    // Closes a channel receiver by removing it from the registry. Once removed,
    // future send(tx, ...) returns false (crossbeam Disconnected). recv(rx) immediately
    // returns Unit since the id is no longer found. Returns true if existed, false otherwise.
    // @param rx The RxChannel receiver handle (second element of channel())
    // @see_also channel
    // @since v0.4.6
    // @example let [tx, rx] = channel() ~ "Close the receiver end"
    // @example close(rx) => true
    module.insert(
        "close".to_string(),
        Value::NativeFunction {
            name: "close".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| concurrent_close(&args[0]),
        },
    );

    // @ntnt select
    // @module std/concurrent
    // @signature select(channels: Array<RxChannel>, timeout_ms?: Int | String) -> Map
    // Waits for the first available value from any of the given receiver handles.
    // Returns a map with "status": "ok", "channel" (the RxChannel that fired), and "value" (the received value).
    // On timeout: returns {"status": "timeout"}.
    // If all channels are closed/disconnected: returns {"status": "closed"}.
    // All return shapes include a "status" key for consistent pattern matching.
    // This is a cancellation yield point.
    // @param channels Array of RxChannel handles to wait on
    // @param timeout_ms Optional timeout in milliseconds (Int) or as a string interval
    // @returns Map with channel/value on success, or status on timeout/closed
    // @see_also channel, recv, recv_timeout
    // @since v0.4.6
    // @example let [tx_a, rx_a] = channel() ~ "Wait for first value from either channel"
    // @example let [tx_b, rx_b] = channel()
    // @example select([rx_a, rx_b])
    // @example select([rx_a, rx_b], 5000) ~ "Wait up to 5 seconds"
    module.insert(
        "select".to_string(),
        Value::NativeFunction {
            name: "select".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: concurrent_select,
        },
    );

    // @ntnt spawn
    // @module std/concurrent
    // @signature spawn(handler: Function) -> Task
    // Spawns a zero-parameter function as a background task. Returns a Task handle.
    // The handler's closure environment is serialized for cross-thread use.
    // Serializable capture types: Int, Float, Bool, String, Array, Map, Struct, Enum.
    // The handler must have zero parameters (including no defaults).
    // @param handler A zero-parameter function to run in the background
    // @returns Task handle for use with await_task, try_await, cancel_task
    // @see_also await_task, try_await, cancel_task
    // @since v0.4.6
    // @example spawn(fn() { 42 }) ~ "Spawn a background task"
    module.insert(
        "spawn".to_string(),
        Value::NativeFunction {
            name: "spawn".to_string(),
            arity: 1,
            max_arity: 1,
            requires: Some(RuntimeCapability::TaskSpawning),
            func: |args| concurrent_spawn(&args[0]),
        },
    );

    // @ntnt await_task
    // @module std/concurrent
    // @signature await_task(task: Task) -> Result<Any, String>
    // Blocks until the task completes and returns its result. Marks the task as
    // consumed (the handle remains valid for try_await, which returns {status: "consumed"}).
    // Returns Ok(value) on success, Err(message) on failure or panic.
    // @param task The task handle from spawn() or after()
    // @returns Result containing the task's return value or error message
    // @see_also spawn, try_await, cancel_task
    // @since v0.4.6
    // @example await_task(task) => Ok(42) ~ "Wait for task result"
    module.insert(
        "await_task".to_string(),
        Value::NativeFunction {
            name: "await_task".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| concurrent_await_task(&args[0]),
        },
    );

    // @ntnt try_await
    // @module std/concurrent
    // @signature try_await(task: Task) -> Map
    // Non-blocking peek at task state. Does NOT remove the task from registry.
    // Returns a map with "status" ("running", "completed", "failed", "panicked", "consumed", "expired")
    // and "result" (Ok(value), Err(message), or None if still running/consumed/expired).
    // @param task The task handle
    // @returns Map with status and result fields
    // @see_also spawn, await_task, cancel_task
    // @since v0.4.6
    // @example try_await(task) => {"status": "running", "result": None} ~ "Check task status"
    module.insert(
        "try_await".to_string(),
        Value::NativeFunction {
            name: "try_await".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| concurrent_try_await(&args[0]),
        },
    );

    // @ntnt cancel_task
    // @module std/concurrent
    // @signature cancel_task(task: Task) -> Bool
    // Requests cooperative cancellation of a task. Sets the cancellation flag;
    // the task thread will exit at the next yield point (recv, recv_timeout,
    // sleep_ms, or fetch). Does NOT force immediate termination.
    // Returns true if the task existed, false otherwise.
    // @param task The task handle
    // @returns Bool indicating whether the cancellation was requested
    // @see_also spawn, await_task
    // @since v0.4.6
    // @example cancel_task(task) => true ~ "Cancel a running task"
    module.insert(
        "cancel_task".to_string(),
        Value::NativeFunction {
            name: "cancel_task".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| concurrent_cancel_task(&args[0]),
        },
    );

    // @ntnt parallel
    // @module std/concurrent
    // @signature parallel(fns: Array<Function>) -> Array | Err
    // Runs all functions concurrently and returns results in input order.
    // If any task fails (crash or returned Err), cancels all remaining tasks and returns that Err.
    // @param fns Array of zero-parameter functions to run
    // @returns Array of Ok values on success, or a single Err value on first failure
    // @see_also race, spawn, await_task
    // @since v0.4.6
    // @example let [a, b] = parallel([fn() { 1 }, fn() { 2 }])
    module.insert(
        "parallel".to_string(),
        Value::NativeFunction {
            name: "parallel".to_string(),
            arity: 1,
            max_arity: 1,
            requires: Some(RuntimeCapability::TaskSpawning),
            func: |args| concurrent_parallel(&args[0]),
        },
    );

    // @ntnt race
    // @module std/concurrent
    // @signature race(fns: Array<Function>) -> Result<Any, String>
    // Runs all functions concurrently and returns the first successful result.
    // Failed or panicked tasks are skipped; all remaining tasks are cancelled on success.
    // If all tasks fail, returns the last Err result.
    // @param fns Array of zero-parameter functions to race
    // @returns Result value from the first successful task
    // @see_also parallel, spawn, await_task, try_await
    // @since v0.4.6
    // @example race([fn() { primary() }, fn() { fallback() }])
    module.insert(
        "race".to_string(),
        Value::NativeFunction {
            name: "race".to_string(),
            arity: 1,
            max_arity: 1,
            requires: Some(RuntimeCapability::TaskSpawning),
            func: |args| concurrent_race(&args[0]),
        },
    );

    // @ntnt after
    // @module std/concurrent
    // @signature after(delay: Int | String, handler: Function) -> Task
    // Runs a zero-parameter handler function after a delay. Returns a Task handle.
    // Delay can be milliseconds (Int) or a human-readable string ("5s", "1m", "500ms").
    // The delay is cancellation-aware (50ms slices).
    // @param delay Delay in milliseconds (Int) or as a string interval
    // @param handler A zero-parameter function to run after the delay
    // @returns Task handle
    // @see_also spawn, await_task, schedule
    // @since v0.4.6
    // @example after(1000, fn() { print("delayed!") }) ~ "Run after 1 second"
    module.insert(
        "after".to_string(),
        Value::NativeFunction {
            name: "after".to_string(),
            arity: 2,
            max_arity: 2,
            requires: Some(RuntimeCapability::Scheduling),
            func: |args| concurrent_after(&args[0], &args[1]),
        },
    );

    // @ntnt schedule
    // @module std/concurrent
    // @signature schedule(interval: Int | String, handler: Function) -> Schedule
    // Runs a zero-parameter handler repeatedly at the given interval. Returns a Schedule handle.
    // Interval can be milliseconds (Int) or a string ("5s", "1m"). Zero intervals are rejected.
    // Each tick spawns a thread with catch_unwind; overlap prevention ensures a new tick
    // won't start until the previous one finishes. Panics in tick execution are caught
    // and logged — they don't kill the schedule.
    // @param interval Interval in milliseconds (Int) or as a string
    // @param handler A zero-parameter function to run on each tick
    // @returns Schedule handle for use with cancel_schedule
    // @see_also cancel_schedule, after
    // @since v0.4.6
    // @example schedule(5000, fn() { print("tick") }) ~ "Run every 5 seconds"
    module.insert(
        "schedule".to_string(),
        Value::NativeFunction {
            name: "schedule".to_string(),
            arity: 2,
            max_arity: 2,
            requires: Some(RuntimeCapability::Scheduling),
            func: |args| concurrent_schedule(&args[0], &args[1]),
        },
    );

    // @ntnt cancel_schedule
    // @module std/concurrent
    // @signature cancel_schedule(schedule: Schedule) -> Bool
    // Cancels a scheduled task. Sets the cancellation flag and removes from registry.
    // Returns true if the schedule existed, false otherwise.
    // @param schedule The schedule handle from schedule()
    // @returns Bool indicating whether the schedule was cancelled
    // @see_also schedule
    // @since v0.4.6
    // @example cancel_schedule(sched) => true ~ "Cancel a scheduled task"
    module.insert(
        "cancel_schedule".to_string(),
        Value::NativeFunction {
            name: "cancel_schedule".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| concurrent_cancel_schedule(&args[0]),
        },
    );

    // @ntnt sleep_ms
    // @module std/concurrent
    // @signature sleep_ms(ms: Int) -> Unit
    // Pauses execution for specified milliseconds. This is a cancellation yield point:
    // a cancelled task will exit during sleep_ms(). Uses 50ms slices internally.
    // Note: sleep() from std/time is NOT cancellation-aware — use this for spawned tasks.
    // @param ms Duration to sleep in milliseconds
    // @since v0.4.6
    // @example sleep_ms(1000) ~ "Sleep for 1 second (cancellation-aware)"
    module.insert(
        "sleep_ms".to_string(),
        Value::NativeFunction {
            name: "sleep_ms".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(ms) => concurrent_sleep_ms(*ms),
                _ => Err(IntentError::type_error(
                    "sleep_ms requires an integer".to_string(),
                )),
            },
        },
    );

    // @ntnt thread_count
    // @module std/concurrent
    // @signature thread_count() -> Int
    // Returns the number of available CPU threads. Useful for sizing parallel work.
    // @since v0.4.6
    // @example thread_count() => 8 ~ "Number of CPU threads"
    module.insert(
        "thread_count".to_string(),
        Value::NativeFunction {
            name: "thread_count".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| concurrent_thread_count(),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{
        BinaryOp, Block, Expression, MatchArm, Parameter, Pattern, Statement, StringPart,
        TemplateFilter, TemplatePart,
    };
    use std::cell::RefCell;
    use std::collections::HashSet;
    use std::rc::Rc;

    fn ids(names: &[&str]) -> HashSet<String> {
        names.iter().map(|name| (*name).to_string()).collect()
    }

    fn no_param_function(body: Block) -> Value {
        Value::Function {
            name: "handler".to_string(),
            params: vec![],
            body,
            closure: Rc::new(RefCell::new(crate::interpreter::Environment::new())),
            contract: None,
            type_params: vec![],
        }
    }

    #[test]
    fn test_module_init() {
        let module = init();
        assert!(module.contains_key("channel"));
        assert!(module.contains_key("send"));
        assert!(module.contains_key("recv"));
        assert!(module.contains_key("try_recv"));
        assert!(module.contains_key("recv_timeout"));
        assert!(module.contains_key("close"));
        assert!(module.contains_key("select"));
        assert!(module.contains_key("sleep_ms"));
        assert!(module.contains_key("thread_count"));
        // New functions
        assert!(module.contains_key("spawn"));
        assert!(module.contains_key("await_task"));
        assert!(module.contains_key("try_await"));
        assert!(module.contains_key("cancel_task"));
        assert!(module.contains_key("after"));
        assert!(module.contains_key("schedule"));
        assert!(module.contains_key("cancel_schedule"));
    }

    #[test]
    fn test_channel_creation() {
        let pair = concurrent_channel().unwrap();
        // channel() returns [tx, rx]
        assert!(matches!(pair, Value::Array(_)));
        if let Value::Array(ref v) = pair {
            assert_eq!(v.len(), 2);
            assert!(matches!(v[0], Value::TxChannelHandle(_, _)));
            assert!(matches!(v[1], Value::RxChannelHandle(_)));
        }
    }

    #[test]
    fn test_channel_send_recv() {
        let pair = concurrent_channel().unwrap();
        let (tx, rx) = if let Value::Array(ref v) = pair {
            (v[0].clone(), v[1].clone())
        } else {
            panic!("Expected array from channel()");
        };
        let sent = concurrent_send(&tx, &Value::String("hello".to_string())).unwrap();
        assert!(matches!(sent, Value::Bool(true)));
        let received = concurrent_recv(&rx).unwrap();
        assert!(matches!(received, Value::String(s) if s == "hello"));
    }

    #[test]
    fn test_try_recv_empty() {
        let pair = concurrent_channel().unwrap();
        let rx = if let Value::Array(ref v) = pair {
            v[1].clone()
        } else {
            panic!()
        };
        let result = concurrent_try_recv(&rx).unwrap();
        match result {
            Value::EnumValue { variant, .. } => assert_eq!(variant, "None"),
            _ => panic!("Expected Option::None"),
        }
    }

    #[test]
    fn test_channel_close_removes_from_registry() {
        let pair = concurrent_channel().unwrap();
        let (tx, rx) = if let Value::Array(ref v) = pair {
            (v[0].clone(), v[1].clone())
        } else {
            panic!("Expected array from channel()");
        };
        let id = get_handle_id(&rx, "RxChannel").unwrap();

        // Channel should exist
        assert!(RUNTIME.channels.lock().unwrap().contains_key(&id));

        // Close removes receiver from registry
        let closed = concurrent_close(&rx).unwrap();
        assert!(matches!(closed, Value::Bool(true)));

        // Channel should be gone from registry
        assert!(!RUNTIME.channels.lock().unwrap().contains_key(&id));

        // Send on closed channel returns false (crossbeam Disconnected)
        let sent = concurrent_send(&tx, &Value::Int(42)).unwrap();
        assert!(matches!(sent, Value::Bool(false)));

        // Close again returns false (not in registry)
        let closed2 = concurrent_close(&rx).unwrap();
        assert!(matches!(closed2, Value::Bool(false)));
    }

    #[test]
    fn test_serialization_round_trip() {
        let values = vec![
            Value::Int(42),
            Value::Float(3.14),
            Value::Bool(true),
            Value::String("test".to_string()),
            Value::Unit,
        ];

        for val in values {
            let serialized = SerializedValue::from_value(&val).unwrap();
            let deserialized = serialized.to_value();
            match (&val, &deserialized) {
                (Value::Int(a), Value::Int(b)) => assert_eq!(a, b),
                (Value::Float(a), Value::Float(b)) => assert_eq!(a, b),
                (Value::Bool(a), Value::Bool(b)) => assert_eq!(a, b),
                (Value::String(a), Value::String(b)) => assert_eq!(a, b),
                (Value::Unit, Value::Unit) => {}
                _ => panic!("Type mismatch"),
            }
        }
    }

    #[test]
    fn test_serialization_struct() {
        let mut fields = HashMap::new();
        fields.insert("x".to_string(), Value::Int(10));
        fields.insert("y".to_string(), Value::String("hello".to_string()));
        let val = Value::Struct {
            name: "Point".to_string(),
            fields,
        };
        let serialized = SerializedValue::from_value(&val).unwrap();
        let deserialized = serialized.to_value();
        match deserialized {
            Value::Struct { name, fields } => {
                assert_eq!(name, "Point");
                match fields.get("x") {
                    Some(Value::Int(10)) => {}
                    other => panic!("Expected Int(10), got {:?}", other),
                }
                match fields.get("y") {
                    Some(Value::String(s)) if s == "hello" => {}
                    other => panic!("Expected String(\"hello\"), got {:?}", other),
                }
            }
            _ => panic!("Expected Struct"),
        }
    }

    #[test]
    fn test_serialization_enum() {
        let val = Value::EnumValue {
            enum_name: "Color".to_string(),
            variant: "Red".to_string(),
            values: vec![Value::Int(255)],
        };
        let serialized = SerializedValue::from_value(&val).unwrap();
        let deserialized = serialized.to_value();
        match deserialized {
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } => {
                assert_eq!(enum_name, "Color");
                assert_eq!(variant, "Red");
                assert_eq!(values.len(), 1);
                assert!(matches!(&values[0], Value::Int(255)));
            }
            _ => panic!("Expected EnumValue"),
        }
    }

    #[test]
    fn test_thread_count() {
        let count = concurrent_thread_count().unwrap();
        match count {
            Value::Int(n) => assert!(n >= 1),
            _ => panic!("Expected Int"),
        }
    }

    #[test]
    fn test_recv_timeout_negative_clamped() {
        let pair = concurrent_channel().unwrap();
        let rx = if let Value::Array(ref v) = pair {
            v[1].clone()
        } else {
            panic!()
        };
        // Negative timeout should be clamped to 0 and return None immediately
        let result = concurrent_recv_timeout(&rx, -100).unwrap();
        match result {
            Value::EnumValue { variant, .. } => assert_eq!(variant, "None"),
            _ => panic!("Expected None for negative timeout"),
        }
    }

    #[test]
    fn test_parse_interval() {
        assert_eq!(parse_interval("100ms").unwrap(), Duration::from_millis(100));
        assert_eq!(parse_interval("5s").unwrap(), Duration::from_secs(5));
        assert_eq!(parse_interval("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_interval("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_interval("500").unwrap(), Duration::from_millis(500));
        assert!(parse_interval("abc").is_err());
    }

    #[test]
    fn test_handle_id_extraction() {
        let handle = create_handle_value("Task", 42);
        assert_eq!(get_handle_id(&handle, "Task").unwrap(), 42);
        assert!(get_handle_id(&handle, "RxChannel").is_err());
        assert!(get_handle_id(&handle, "TxChannel").is_err());
        assert!(get_handle_id(&Value::Int(42), "Task").is_err());
    }

    #[test]
    fn test_schedule_rejects_zero_interval() {
        // Int zero
        let handler = Value::Function {
            name: "test".to_string(),
            params: vec![],
            body: crate::ast::Block { statements: vec![] },
            closure: std::rc::Rc::new(std::cell::RefCell::new(
                crate::interpreter::Environment::new(),
            )),
            contract: None,
            type_params: vec![],
        };
        let result = concurrent_schedule(&Value::Int(0), &handler);
        assert!(result.is_err());

        // Negative
        let result = concurrent_schedule(&Value::Int(-1), &handler);
        assert!(result.is_err());
    }

    #[test]
    fn test_spawn_rejects_parameterized_function() {
        let handler = Value::Function {
            name: "test".to_string(),
            params: vec![crate::ast::Parameter {
                name: "x".to_string(),
                type_annotation: None,
                default: Some(crate::ast::Expression::Integer(42)),
                pattern: None,
            }],
            body: crate::ast::Block { statements: vec![] },
            closure: std::rc::Rc::new(std::cell::RefCell::new(
                crate::interpreter::Environment::new(),
            )),
            contract: None,
            type_params: vec![],
        };
        let result = concurrent_spawn(&handler);
        assert!(result.is_err());
    }

    #[test]
    fn test_cancel_task_sets_flag_only() {
        // Create a task handle with a known cancel token
        let cancelled = Arc::new(CancelToken::new());
        let task_id = RUNTIME.register_task(Arc::clone(&cancelled)).unwrap();
        let handle = create_handle_value("Task", task_id);

        // Cancel should set the flag
        let result = concurrent_cancel_task(&handle).unwrap();
        assert!(matches!(result, Value::Bool(true)));
        assert!(cancelled.is_cancelled());

        // Task state should still be Running (cooperative — not forced)
        let arcs = RUNTIME.get_task_arcs(task_id).unwrap().unwrap();
        assert_eq!(arcs.inner.lock().unwrap().state, TaskState::Running);
    }

    #[test]
    fn test_cancel_schedule_removes_from_registry() {
        let (schedule_id, cancelled, _tick_running) = RUNTIME.register_schedule().unwrap();
        let handle = create_handle_value("Schedule", schedule_id);

        // Should exist
        assert!(RUNTIME.schedules.lock().unwrap().contains_key(&schedule_id));

        // Cancel removes
        let result = concurrent_cancel_schedule(&handle).unwrap();
        assert!(matches!(result, Value::Bool(true)));

        // Flag set
        assert!(cancelled.is_cancelled());

        // Removed from registry
        assert!(!RUNTIME.schedules.lock().unwrap().contains_key(&schedule_id));

        // Second cancel returns false
        let result2 = concurrent_cancel_schedule(&handle).unwrap();
        assert!(matches!(result2, Value::Bool(false)));
    }

    #[test]
    fn test_capture_bindings_separates_native_fns() {
        let mut bindings = HashMap::new();
        bindings.insert("x".to_string(), Value::Int(42));
        bindings.insert(
            "native_fn".to_string(),
            Value::NativeFunction {
                name: "test".to_string(),
                arity: 0,
                max_arity: 0,
                requires: None,
                func: |_| Ok(Value::Unit),
            },
        );

        let captured = capture_bindings(&bindings).expect("should succeed with no closures");
        // Serializable values captured
        assert!(captured.values.contains_key("x"));
        assert!(!captured.values.contains_key("native_fn"));
        // Native function recorded with full identity for direct reconstruction
        assert!(captured
            .native_fns
            .iter()
            .any(|cap| cap.binding_name == "native_fn"
                && cap.fn_name == "test"
                && cap.arity == 0
                && cap.max_arity == 0));
    }

    #[test]
    fn test_free_variables_handles_expression_variants() {
        let body = Block {
            statements: vec![
                Statement::Let {
                    name: "bound".to_string(),
                    mutable: false,
                    type_annotation: None,
                    value: Some(Expression::Identifier("seed".to_string())),
                    pattern: None,
                    otherwise: Some(Block {
                        statements: vec![Statement::Return(Some(Expression::Binary {
                            left: Box::new(Expression::Identifier("err".to_string())),
                            operator: BinaryOp::Add,
                            right: Box::new(Expression::Identifier("fallback".to_string())),
                        }))],
                    }),
                },
                Statement::Expression(Expression::Call {
                    function: Box::new(Expression::Identifier("callee".to_string())),
                    arguments: vec![
                        Expression::MethodCall {
                            object: Box::new(Expression::Identifier("receiver".to_string())),
                            method: "method_lookup".to_string(),
                            arguments: vec![Expression::Identifier("method_arg".to_string())],
                        },
                        Expression::FieldAccess {
                            object: Box::new(Expression::Identifier("record".to_string())),
                            field: "name".to_string(),
                        },
                        Expression::Index {
                            object: Box::new(Expression::Identifier("items".to_string())),
                            index: Box::new(Expression::Identifier("idx".to_string())),
                        },
                        Expression::Array(vec![Expression::Identifier("array_item".to_string())]),
                        Expression::MapLiteral(vec![(
                            Expression::Identifier("map_key".to_string()),
                            Expression::Identifier("map_value".to_string()),
                        )]),
                        Expression::Range {
                            start: Box::new(Expression::Identifier("range_start".to_string())),
                            end: Box::new(Expression::Identifier("range_end".to_string())),
                            inclusive: true,
                        },
                        Expression::InterpolatedString(vec![
                            StringPart::Literal("hello ".to_string()),
                            StringPart::Expr(Expression::Identifier("interpolated".to_string())),
                        ]),
                        Expression::TemplateString(vec![
                            TemplatePart::Expr(Expression::Identifier("template_expr".to_string())),
                            TemplatePart::RawExpr(Expression::Identifier(
                                "template_raw".to_string(),
                            )),
                            TemplatePart::FilteredExpr {
                                expr: Expression::Identifier("template_filtered".to_string()),
                                filters: vec![TemplateFilter {
                                    name: "fmt".to_string(),
                                    args: vec![Expression::Identifier(
                                        "template_filter_arg".to_string(),
                                    )],
                                }],
                            },
                            TemplatePart::RawFilteredExpr {
                                expr: Expression::Identifier("template_raw_filtered".to_string()),
                                filters: vec![TemplateFilter {
                                    name: "raw".to_string(),
                                    args: vec![Expression::Identifier(
                                        "template_raw_filter_arg".to_string(),
                                    )],
                                }],
                            },
                            TemplatePart::ForLoop {
                                var: "item".to_string(),
                                iterable: Expression::Identifier("template_iter".to_string()),
                                body: vec![TemplatePart::Expr(Expression::Binary {
                                    left: Box::new(Expression::Identifier("item".to_string())),
                                    operator: BinaryOp::Add,
                                    right: Box::new(Expression::Binary {
                                        left: Box::new(Expression::Identifier(
                                            "@index".to_string(),
                                        )),
                                        operator: BinaryOp::Add,
                                        right: Box::new(Expression::Identifier(
                                            "template_outer".to_string(),
                                        )),
                                    }),
                                })],
                                empty_body: vec![TemplatePart::Expr(Expression::Identifier(
                                    "template_empty".to_string(),
                                ))],
                            },
                            TemplatePart::IfBlock {
                                condition: Expression::Identifier("template_if".to_string()),
                                then_parts: vec![TemplatePart::Expr(Expression::Identifier(
                                    "template_then".to_string(),
                                ))],
                                elif_chains: vec![(
                                    Expression::Identifier("template_elif".to_string()),
                                    vec![TemplatePart::Expr(Expression::Identifier(
                                        "template_elif_body".to_string(),
                                    ))],
                                )],
                                else_parts: vec![TemplatePart::Expr(Expression::Identifier(
                                    "template_else".to_string(),
                                ))],
                            },
                            TemplatePart::Partial {
                                name: "card".to_string(),
                                data_expr: Some(Expression::Identifier(
                                    "template_partial".to_string(),
                                )),
                            },
                        ]),
                        Expression::StructLiteral {
                            name: "Point".to_string(),
                            fields: vec![(
                                "x".to_string(),
                                Expression::Identifier("struct_value".to_string()),
                            )],
                        },
                        Expression::EnumVariant {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            arguments: vec![Expression::Identifier("enum_arg".to_string())],
                        },
                        Expression::Lambda {
                            params: vec![
                                Parameter {
                                    name: "first".to_string(),
                                    type_annotation: None,
                                    default: Some(Expression::Identifier(
                                        "lambda_default".to_string(),
                                    )),
                                    pattern: None,
                                },
                                Parameter {
                                    name: "_tuple".to_string(),
                                    type_annotation: None,
                                    default: Some(Expression::Identifier(
                                        "lambda_pattern_default".to_string(),
                                    )),
                                    pattern: Some(Pattern::Tuple(vec![
                                        Pattern::Variable("left".to_string()),
                                        Pattern::Variable("right".to_string()),
                                    ])),
                                },
                            ],
                            body: Block {
                                statements: vec![Statement::Expression(Expression::Binary {
                                    left: Box::new(Expression::Identifier("first".to_string())),
                                    operator: BinaryOp::Add,
                                    right: Box::new(Expression::Binary {
                                        left: Box::new(Expression::Identifier("right".to_string())),
                                        operator: BinaryOp::Add,
                                        right: Box::new(Expression::Identifier(
                                            "lambda_free".to_string(),
                                        )),
                                    }),
                                })],
                            },
                        },
                        Expression::Block(Block {
                            statements: vec![
                                Statement::Let {
                                    name: "block_local".to_string(),
                                    mutable: false,
                                    type_annotation: None,
                                    value: Some(Expression::Identifier("block_init".to_string())),
                                    pattern: None,
                                    otherwise: None,
                                },
                                Statement::Expression(Expression::Identifier(
                                    "block_free".to_string(),
                                )),
                            ],
                        }),
                        Expression::IfExpr {
                            condition: Box::new(Expression::Identifier("if_cond".to_string())),
                            then_branch: Box::new(Expression::Identifier("if_then".to_string())),
                            else_branch: Box::new(Expression::Identifier("if_else".to_string())),
                        },
                        Expression::Match {
                            scrutinee: Box::new(Expression::Identifier("match_input".to_string())),
                            arms: vec![MatchArm {
                                pattern: Pattern::Tuple(vec![
                                    Pattern::Variable("match_left".to_string()),
                                    Pattern::Variable("match_right".to_string()),
                                ]),
                                guard: Some(Expression::Identifier("match_guard".to_string())),
                                body: Expression::Binary {
                                    left: Box::new(Expression::Identifier(
                                        "match_left".to_string(),
                                    )),
                                    operator: BinaryOp::Add,
                                    right: Box::new(Expression::Identifier(
                                        "match_body_free".to_string(),
                                    )),
                                },
                            }],
                        },
                        Expression::Assign {
                            target: Box::new(Expression::Identifier("assign_target".to_string())),
                            value: Box::new(Expression::Identifier("assign_value".to_string())),
                        },
                        Expression::Await(Box::new(Expression::Identifier("awaited".to_string()))),
                        Expression::Try(Box::new(Expression::Identifier("tried".to_string()))),
                        Expression::TryCatch {
                            body: Block {
                                statements: vec![Statement::Expression(Expression::Identifier(
                                    "catch_free".to_string(),
                                ))],
                            },
                        },
                    ],
                }),
            ],
        };

        let free = free_variables(&body);
        let expected = ids(&[
            "array_item",
            "assign_target",
            "assign_value",
            "awaited",
            "block_free",
            "block_init",
            "callee",
            "catch_free",
            "enum_arg",
            "fallback",
            "if_cond",
            "if_else",
            "if_then",
            "idx",
            "interpolated",
            "items",
            "lambda_default",
            "lambda_free",
            "lambda_pattern_default",
            "map_key",
            "map_value",
            "match_body_free",
            "match_guard",
            "match_input",
            "method_arg",
            "method_lookup",
            "range_end",
            "range_start",
            "receiver",
            "record",
            "seed",
            "struct_value",
            "template_elif",
            "template_elif_body",
            "template_else",
            "template_empty",
            "template_expr",
            "template_filter_arg",
            "template_filtered",
            "template_if",
            "template_iter",
            "template_outer",
            "template_partial",
            "template_raw",
            "template_raw_filter_arg",
            "template_raw_filtered",
            "template_then",
            "tried",
        ]);

        assert_eq!(free, expected);
        assert!(!free.contains("bound"));
        assert!(!free.contains("err"));
        assert!(!free.contains("item"));
        assert!(!free.contains("@index"));
        assert!(!free.contains("first"));
        assert!(!free.contains("right"));
        assert!(!free.contains("match_left"));
        assert!(!free.contains("Point"));
        assert!(!free.contains("Result"));
    }

    #[test]
    fn test_free_variables_nested_closures_do_not_leak_inner_bindings() {
        let body = Block {
            statements: vec![
                Statement::Expression(Expression::Lambda {
                    params: vec![],
                    body: Block {
                        statements: vec![Statement::Let {
                            name: "inner_only".to_string(),
                            mutable: false,
                            type_annotation: None,
                            value: Some(Expression::Identifier("captured".to_string())),
                            pattern: None,
                            otherwise: None,
                        }],
                    },
                }),
                Statement::Expression(Expression::Identifier("inner_only".to_string())),
            ],
        };

        assert_eq!(free_variables(&body), ids(&["captured", "inner_only"]));
    }

    #[test]
    fn test_free_variables_destructuring_patterns_bind_names() {
        let body = Block {
            statements: vec![
                Statement::Let {
                    name: "_tuple".to_string(),
                    mutable: false,
                    type_annotation: None,
                    value: Some(Expression::Identifier("source".to_string())),
                    pattern: Some(Pattern::Tuple(vec![
                        Pattern::Variable("left".to_string()),
                        Pattern::Map {
                            fields: vec![(
                                "name".to_string(),
                                Pattern::Variable("right".to_string()),
                            )],
                            rest: Some("rest".to_string()),
                        },
                    ])),
                    otherwise: None,
                },
                Statement::Expression(Expression::Binary {
                    left: Box::new(Expression::Identifier("left".to_string())),
                    operator: BinaryOp::Add,
                    right: Box::new(Expression::Binary {
                        left: Box::new(Expression::Identifier("right".to_string())),
                        operator: BinaryOp::Add,
                        right: Box::new(Expression::Identifier("rest".to_string())),
                    }),
                }),
            ],
        };

        assert_eq!(free_variables(&body), ids(&["source"]));
    }

    #[test]
    fn test_free_variables_match_arm_bindings_scoped_correctly() {
        let body = Block {
            statements: vec![
                Statement::Expression(Expression::Match {
                    scrutinee: Box::new(Expression::Identifier("input".to_string())),
                    arms: vec![
                        MatchArm {
                            pattern: Pattern::Variable("value".to_string()),
                            guard: Some(Expression::Binary {
                                left: Box::new(Expression::Identifier("value".to_string())),
                                operator: BinaryOp::Add,
                                right: Box::new(Expression::Identifier("guard_free".to_string())),
                            }),
                            body: Expression::Binary {
                                left: Box::new(Expression::Identifier("value".to_string())),
                                operator: BinaryOp::Add,
                                right: Box::new(Expression::Identifier("body_free".to_string())),
                            },
                        },
                        MatchArm {
                            pattern: Pattern::Wildcard,
                            guard: None,
                            body: Expression::Identifier("fallback".to_string()),
                        },
                    ],
                }),
                Statement::Expression(Expression::Identifier("value".to_string())),
            ],
        };

        assert_eq!(
            free_variables(&body),
            ids(&["body_free", "fallback", "guard_free", "input", "value"])
        );
    }

    #[test]
    fn test_validate_and_capture_filters_unused_user_functions() {
        let mut env = crate::interpreter::Environment::new();
        env.define("used_data".to_string(), Value::Int(7));
        env.define("unused_data".to_string(), Value::Int(99));
        env.define(
            "unused_user_fn".to_string(),
            no_param_function(Block {
                statements: vec![Statement::Expression(Expression::Integer(1))],
            }),
        );

        let handler = Value::Function {
            name: "handler".to_string(),
            params: vec![],
            body: Block {
                statements: vec![Statement::Expression(Expression::Identifier(
                    "used_data".to_string(),
                ))],
            },
            closure: Rc::new(RefCell::new(env)),
            contract: None,
            type_params: vec![],
        };

        let (captured, _) = validate_and_capture("spawn", &handler).expect("capture should work");
        assert!(captured.values.contains_key("used_data"));
        assert!(!captured.values.contains_key("unused_data"));
        assert!(captured.native_fns.is_empty());
    }
}
