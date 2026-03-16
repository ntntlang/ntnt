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
//!
//! ## Tasks
//!
//! - `await_task()` marks state as `Consumed` (the "I'm done with this handle" call).
//! - `try_await()` peeks without consuming — updates `last_checked_at`. Never errors for
//!   handles that existed; returns `{status: "consumed"}` or `{status: "expired"}` instead.
//! - `cancel_task()` only sets the `AtomicBool` flag (cooperative cancellation via yield points).
//! - All `eval_block` calls are wrapped in `catch_unwind(AssertUnwindSafe(...))`.
//! - Tasks auto-expire (marked `Expired`) after 5 minutes in terminal state (the reaper runs on
//!   `spawn()` and `after()` entry), but only if not recently `try_await()`'d.
//!
//! ## Schedules
//!
//! - `cancel_schedule()` sets flag AND removes from registry.
//! - Schedule sleep uses 50ms cancellation-aware slices.
//! - Zero-duration intervals are rejected.
//! - Tick execution spawns a thread with `catch_unwind` and overlap prevention via `AtomicBool`.
//!
//! ## Cancellation
//!
//! Thread-local `CURRENT_TASK_CANCELLED` for cooperative cancellation.
//! Yield points: `recv()`, `recv_timeout()`, `sleep_ms()`, `http_fetch()`, `http_get()`.
//! Note: `sleep()` from std/time is NOT cancellation-aware.

use crate::error::IntentError;
use crate::interpreter::Value;
use crossbeam_channel::{self as crossbeam};
use std::collections::HashMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

// =============================================================================
// Thread-local cancellation flag (cooperative cancellation)
// =============================================================================

thread_local! {
    /// Set by the task's thread to point at the task's cancellation flag.
    /// Yield-point functions check this to honour cooperative cancellation.
    pub static CURRENT_TASK_CANCELLED: std::cell::RefCell<Option<Arc<AtomicBool>>> =
        const { std::cell::RefCell::new(None) };
}

/// Check if the current thread's task has been cancelled.
/// Called at yield points: recv, recv_timeout, sleep_ms, http_fetch, http_get.
pub fn is_current_task_cancelled() -> bool {
    CURRENT_TASK_CANCELLED.with(|cell| {
        cell.borrow()
            .as_ref()
            .map(|flag| flag.load(AtomicOrdering::Acquire))
            .unwrap_or(false)
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
/// Handles: Unit, Int, Float, Bool, String, Array, Map, Struct (via `__type` marker),
/// EnumValue (via `__enum` marker).
#[derive(Debug, Clone)]
pub(crate) enum SerializedValue {
    Unit,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<SerializedValue>),
    Map(HashMap<String, SerializedValue>),
    /// Handle types — just the ID, reconstructed to the proper Value variant on deserialization.
    TaskHandle(u64),
    ChannelHandle(u64),
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
                serialized.insert(
                    "__type".to_string(),
                    SerializedValue::String(name.clone()),
                );
                for (k, v) in fields {
                    serialized.insert(k.clone(), Self::from_value(v)?);
                }
                Ok(SerializedValue::Map(serialized))
            }
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } => {
                let mut serialized = HashMap::new();
                serialized.insert(
                    "__enum".to_string(),
                    SerializedValue::String(enum_name.clone()),
                );
                serialized.insert(
                    "__variant".to_string(),
                    SerializedValue::String(variant.clone()),
                );
                let vals: Result<Vec<_>> = values.iter().map(Self::from_value).collect();
                serialized.insert("__values".to_string(), SerializedValue::Array(vals?));
                Ok(SerializedValue::Map(serialized))
            }
            Value::TaskHandle(id) => Ok(SerializedValue::TaskHandle(*id)),
            Value::ChannelHandle(id) => Ok(SerializedValue::ChannelHandle(*id)),
            Value::ScheduleHandle(id) => Ok(SerializedValue::ScheduleHandle(*id)),
            _ => Err(IntentError::type_error(
                "Only serializable types (Int, Float, String, Bool, Array, Map, Struct, Enum) can be sent across task boundaries".to_string(),
            )),
        }
    }

    /// Convert back to Value, reconstructing Struct and EnumValue from markers.
    pub(crate) fn to_value(&self) -> Value {
        match self {
            SerializedValue::Unit => Value::Unit,
            SerializedValue::Int(i) => Value::Int(*i),
            SerializedValue::Float(f) => Value::Float(*f),
            SerializedValue::Bool(b) => Value::Bool(*b),
            SerializedValue::String(s) => Value::String(s.clone()),
            SerializedValue::Array(arr) => Value::Array(arr.iter().map(|v| v.to_value()).collect()),
            SerializedValue::Map(map) => {
                // Check for __enum marker first
                if let Some(SerializedValue::String(enum_name)) = map.get("__enum") {
                    if let (
                        Some(SerializedValue::String(variant)),
                        Some(SerializedValue::Array(values)),
                    ) = (map.get("__variant"), map.get("__values"))
                    {
                        return Value::EnumValue {
                            enum_name: enum_name.clone(),
                            variant: variant.clone(),
                            values: values.iter().map(|v| v.to_value()).collect(),
                        };
                    }
                }
                // Check for __type marker (struct)
                if let Some(SerializedValue::String(type_name)) = map.get("__type") {
                    let mut fields = HashMap::new();
                    for (k, v) in map {
                        if k != "__type" {
                            fields.insert(k.clone(), v.to_value());
                        }
                    }
                    return Value::Struct {
                        name: type_name.clone(),
                        fields,
                    };
                }
                // Regular map
                let mut result = HashMap::new();
                for (k, v) in map {
                    result.insert(k.clone(), v.to_value());
                }
                Value::Map(result)
            }
            SerializedValue::TaskHandle(id) => Value::TaskHandle(*id),
            SerializedValue::ChannelHandle(id) => Value::ChannelHandle(*id),
            SerializedValue::ScheduleHandle(id) => Value::ScheduleHandle(*id),
        }
    }
}

// =============================================================================
// Task states
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskState {
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

struct TaskEntry {
    /// Current state of the task. Written by task thread, read by main thread.
    state: Arc<Mutex<TaskState>>,
    /// The result once the task reaches a terminal state.
    result: Arc<Mutex<Option<SerializedValue>>>,
    /// The error message if the task failed/panicked.
    error_msg: Arc<Mutex<Option<String>>>,
    /// Cooperative cancellation flag. Set by `cancel_task()` with Release ordering.
    cancelled: Arc<AtomicBool>,
    /// When the task entered a terminal state (for reaper).
    completed_at: Arc<Mutex<Option<Instant>>>,
    /// Last time `try_await()` checked this task (prevents reaping active handles).
    last_checked_at: Arc<Mutex<Option<Instant>>>,
}

// =============================================================================
// Channel entry — no `closed` flag, close = remove from map
// =============================================================================

struct ChannelEntry {
    sender: crossbeam::Sender<SerializedValue>,
    /// Arc<Mutex<Receiver>> — single-consumer. The MutexGuard is held for the
    /// blocking duration of recv(). A second recv() caller blocks until the first finishes.
    /// Note: crossbeam Receiver is Clone, but we keep Mutex for single-consumer semantics.
    receiver: Arc<Mutex<crossbeam::Receiver<SerializedValue>>>,
}

// =============================================================================
// Schedule entry
// =============================================================================

struct ScheduleEntry {
    cancelled: Arc<AtomicBool>,
    /// Overlap prevention: true while a tick is executing.
    /// Used by the schedule thread (not read from ScheduleEntry directly,
    /// but cloned out during register_schedule).
    #[allow(dead_code)]
    tick_running: Arc<AtomicBool>,
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
    /// Task registry. Lock, clone Arcs, drop, then operate.
    tasks: Mutex<HashMap<u64, TaskEntry>>,
    /// Channel registry. close() = remove from this map.
    channels: Mutex<HashMap<u64, ChannelEntry>>,
    /// Schedule registry. cancel_schedule() = set flag + remove.
    schedules: Mutex<HashMap<u64, ScheduleEntry>>,
}

impl ConcurrencyRuntime {
    fn new() -> Self {
        ConcurrencyRuntime {
            id_counter: AtomicU64::new(1),
            tasks: Mutex::new(HashMap::new()),
            channels: Mutex::new(HashMap::new()),
            schedules: Mutex::new(HashMap::new()),
        }
    }

    fn next_id(&self) -> u64 {
        self.id_counter.fetch_add(1, AtomicOrdering::Release)
    }

    // -------------------------------------------------------------------------
    // Reaper — auto-expire terminal tasks older than 5 minutes
    // -------------------------------------------------------------------------

    /// Reap tasks that have been in terminal state for >5 min AND haven't been
    /// `try_await()`'d recently. Called on `spawn()` and `after()` entry.
    ///
    /// Instead of removing from registry, marks state as `Expired` so that
    /// `try_await()` can return `{status: "expired"}` instead of erroring.
    fn reap_expired_tasks(&self) {
        let now = Instant::now();
        let expiry = Duration::from_secs(300); // 5 minutes
        let recent_check_window = Duration::from_secs(300); // 5 minutes

        // Step 1: Acquire registry lock → clone Arcs → drop lock
        #[allow(clippy::type_complexity)]
        let task_arcs: Vec<(
            u64,
            Arc<Mutex<TaskState>>,
            Arc<Mutex<Option<Instant>>>,
            Arc<Mutex<Option<Instant>>>,
        )> = {
            let tasks = match self.tasks.lock() {
                Ok(t) => t,
                Err(_) => return, // poisoned — skip
            };
            tasks
                .iter()
                .map(|(id, entry)| {
                    (
                        *id,
                        Arc::clone(&entry.state),
                        Arc::clone(&entry.completed_at),
                        Arc::clone(&entry.last_checked_at),
                    )
                })
                .collect()
        };
        // Registry lock is dropped here

        // Step 2: Inspect per-task state outside registry lock
        let mut ids_to_expire: Vec<u64> = Vec::new();
        for (id, state_arc, completed_at_arc, last_checked_at_arc) in &task_arcs {
            let state = match state_arc.lock() {
                Ok(s) => *s,
                Err(_) => continue, // poisoned — skip
            };
            // Only expire tasks in terminal states (not Running or already Expired)
            if !matches!(
                state,
                TaskState::Completed
                    | TaskState::Failed
                    | TaskState::Panicked
                    | TaskState::Consumed
            ) {
                continue;
            }
            let completed_at = match completed_at_arc.lock() {
                Ok(c) => *c,
                Err(_) => continue,
            };
            let Some(completed) = completed_at else {
                continue; // no completion time recorded — skip
            };
            if now.duration_since(completed) < expiry {
                continue; // not old enough — skip
            }
            // Check if recently try_await'd
            let last_checked = match last_checked_at_arc.lock() {
                Ok(l) => *l,
                Err(_) => continue,
            };
            if let Some(checked) = last_checked {
                if now.duration_since(checked) < recent_check_window {
                    continue; // recently checked — skip
                }
            }
            ids_to_expire.push(*id);
        }

        // Step 3: Mark expired tasks (using already-cloned state Arcs — no registry lock needed)
        for (id, state_arc, _, _) in &task_arcs {
            if ids_to_expire.contains(id) {
                if let Ok(mut s) = state_arc.lock() {
                    *s = TaskState::Expired;
                }
            }
        }
    }

    // -------------------------------------------------------------------------
    // Channels
    // -------------------------------------------------------------------------

    fn create_channel(&self) -> u64 {
        let id = self.next_id();
        let (tx, rx) = crossbeam::unbounded();
        let entry = ChannelEntry {
            sender: tx,
            receiver: Arc::new(Mutex::new(rx)),
        };
        // Lock, insert, drop
        if let Ok(mut channels) = self.channels.lock() {
            channels.insert(id, entry);
        }
        id
    }

    fn send(&self, channel_id: u64, value: SerializedValue) -> bool {
        // Lock → clone sender → drop lock → send
        let sender = {
            let channels = match self.channels.lock() {
                Ok(c) => c,
                Err(_) => return false,
            };
            match channels.get(&channel_id) {
                Some(entry) => entry.sender.clone(),
                None => return false, // channel removed (closed)
            }
        };
        // Sender cloned, lock dropped — safe to send
        sender.send(value).is_ok()
    }

    fn recv(&self, channel_id: u64) -> Result<Value> {
        // Yield point: check cancellation before blocking
        check_cancellation()?;

        // Lock → clone receiver Arc → drop lock → lock receiver → recv
        let receiver = {
            let channels = match self.channels.lock() {
                Ok(c) => c,
                Err(_) => {
                    return Err(IntentError::runtime_error(
                        "Channel registry poisoned".to_string(),
                    ))
                }
            };
            match channels.get(&channel_id) {
                Some(entry) => Arc::clone(&entry.receiver),
                None => return Ok(Value::Unit), // channel removed → disconnected → Unit
            }
        };
        // Channel registry lock dropped. Now lock the receiver (single-consumer).
        let rx = match receiver.lock() {
            Ok(r) => r,
            Err(_) => {
                return Err(IntentError::runtime_error(
                    "Channel receiver poisoned".to_string(),
                ))
            }
        };
        match rx.recv() {
            Ok(serialized) => Ok(serialized.to_value()),
            Err(_) => Ok(Value::Unit), // sender dropped → disconnected → Unit
        }
    }

    fn recv_timeout(&self, channel_id: u64, timeout_ms: i64) -> Result<Value> {
        // Yield point: check cancellation
        check_cancellation()?;

        // Clamp negative to 0
        let total_ms = if timeout_ms < 0 { 0 } else { timeout_ms as u64 };

        // Lock → clone receiver Arc → drop lock
        let receiver = {
            let channels = match self.channels.lock() {
                Ok(c) => c,
                Err(_) => {
                    return Err(IntentError::runtime_error(
                        "Channel registry poisoned".to_string(),
                    ))
                }
            };
            match channels.get(&channel_id) {
                Some(entry) => Arc::clone(&entry.receiver),
                None => return Ok(Value::none()), // channel removed → None
            }
        };

        let rx = match receiver.lock() {
            Ok(r) => r,
            Err(_) => {
                return Err(IntentError::runtime_error(
                    "Channel receiver poisoned".to_string(),
                ))
            }
        };

        // Loop in ≤100ms slices, checking cancellation between iterations
        let deadline = Instant::now() + Duration::from_millis(total_ms);
        loop {
            check_cancellation()?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Ok(Value::none()); // timeout
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
        // Lock → clone receiver Arc → drop lock
        let receiver = {
            let channels = match self.channels.lock() {
                Ok(c) => c,
                Err(_) => {
                    return Err(IntentError::runtime_error(
                        "Channel registry poisoned".to_string(),
                    ))
                }
            };
            match channels.get(&channel_id) {
                Some(entry) => Arc::clone(&entry.receiver),
                None => return Ok(Value::none()), // removed
            }
        };
        let rx = match receiver.lock() {
            Ok(r) => r,
            Err(_) => {
                return Err(IntentError::runtime_error(
                    "Channel receiver poisoned".to_string(),
                ))
            }
        };
        match rx.try_recv() {
            Ok(serialized) => Ok(Value::some(serialized.to_value())),
            Err(crossbeam::TryRecvError::Empty) => Ok(Value::none()),
            Err(crossbeam::TryRecvError::Disconnected) => Ok(Value::none()),
        }
    }

    fn close_channel(&self, channel_id: u64) -> bool {
        // close() = remove from map. Dropping the Sender causes recv() → Disconnected → Unit.
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
    fn register_task(&self, cancelled: Arc<AtomicBool>) -> u64 {
        let id = self.next_id();
        let entry = TaskEntry {
            state: Arc::new(Mutex::new(TaskState::Running)),
            result: Arc::new(Mutex::new(None)),
            error_msg: Arc::new(Mutex::new(None)),
            cancelled,
            completed_at: Arc::new(Mutex::new(None)),
            last_checked_at: Arc::new(Mutex::new(None)),
        };
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.insert(id, entry);
        }
        id
    }

    /// Get cloned Arcs for a task (state, result, error_msg, cancelled, completed_at).
    /// Returns None if the task doesn't exist.
    fn get_task_arcs(
        &self,
        task_id: u64,
    ) -> Option<(
        Arc<Mutex<TaskState>>,
        Arc<Mutex<Option<SerializedValue>>>,
        Arc<Mutex<Option<String>>>,
        Arc<AtomicBool>,
        Arc<Mutex<Option<Instant>>>,
    )> {
        let tasks = self.tasks.lock().ok()?;
        let entry = tasks.get(&task_id)?;
        Some((
            Arc::clone(&entry.state),
            Arc::clone(&entry.result),
            Arc::clone(&entry.error_msg),
            Arc::clone(&entry.cancelled),
            Arc::clone(&entry.completed_at),
        ))
    }

    /// `await_task(handle)` — blocks until task completes, then marks as Consumed.
    /// Returns `Result`: `Ok(value)` or `Err(message)`.
    fn await_task(&self, task_id: u64) -> Result<Value> {
        // Get Arcs (lock → clone → drop)
        let (state_arc, result_arc, error_arc, _cancelled, _completed_at) =
            match self.get_task_arcs(task_id) {
                Some(arcs) => arcs,
                None => {
                    return Err(IntentError::runtime_error(
                        "Invalid task handle".to_string(),
                    ))
                }
            };

        // Check for already-consumed or expired handles
        {
            let state = state_arc.lock().unwrap();
            match *state {
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

        // Spin-wait for terminal state (10ms slices)
        loop {
            {
                let state = state_arc.lock().unwrap();
                if state.is_terminal() {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }

        // Read result
        let state = *state_arc.lock().unwrap();
        let result_value = match state {
            TaskState::Completed => {
                let result = result_arc.lock().unwrap();
                let val = result.as_ref().map(|s| s.to_value()).unwrap_or(Value::Unit);
                Value::ok(val)
            }
            TaskState::Failed => {
                let err = error_arc.lock().unwrap();
                let msg = err.clone().unwrap_or_else(|| "Task failed".to_string());
                Value::err(Value::String(msg))
            }
            TaskState::Panicked => {
                let err = error_arc.lock().unwrap();
                let msg = err.clone().unwrap_or_else(|| "Task panicked".to_string());
                Value::err(Value::String(msg))
            }
            TaskState::Running => unreachable!(),
            // These are caught above, but handle for exhaustiveness
            TaskState::Consumed | TaskState::Expired => unreachable!(),
        };

        // Mark as Consumed instead of removing (preserves handle for try_await)
        *state_arc.lock().unwrap() = TaskState::Consumed;

        Ok(result_value)
    }

    /// `try_await(handle)` — peek at task state without removing. Updates `last_checked_at`.
    /// Returns a map: `{ "status": "running"|"completed"|"failed"|"panicked"|"consumed"|"expired", "result": ... }`
    /// NEVER returns an error for a handle that existed — returns status map instead.
    fn try_await(&self, task_id: u64) -> Result<Value> {
        // Get arcs
        let arcs = {
            let tasks = match self.tasks.lock() {
                Ok(t) => t,
                Err(_) => {
                    return Err(IntentError::runtime_error(
                        "Task registry poisoned".to_string(),
                    ))
                }
            };
            match tasks.get(&task_id) {
                Some(entry) => (
                    Arc::clone(&entry.state),
                    Arc::clone(&entry.result),
                    Arc::clone(&entry.error_msg),
                    Arc::clone(&entry.last_checked_at),
                ),
                None => {
                    return Err(IntentError::runtime_error(
                        "Invalid task handle".to_string(),
                    ))
                }
            }
        };
        let (state_arc, result_arc, error_arc, last_checked_arc) = arcs;

        // Update last_checked_at (rule 13: prevents reaper from invalidating active handles)
        if let Ok(mut last_checked) = last_checked_arc.lock() {
            *last_checked = Some(Instant::now());
        }

        let state = *state_arc.lock().unwrap();
        let mut result_map = HashMap::new();

        let status_str = match state {
            TaskState::Running => "running",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
            TaskState::Panicked => "panicked",
            TaskState::Consumed => "consumed",
            TaskState::Expired => "expired",
        };
        result_map.insert("status".to_string(), Value::String(status_str.to_string()));

        match state {
            TaskState::Completed => {
                let result = result_arc.lock().unwrap();
                let val = result.as_ref().map(|s| s.to_value()).unwrap_or(Value::Unit);
                result_map.insert("result".to_string(), Value::ok(val));
            }
            TaskState::Failed | TaskState::Panicked => {
                let err = error_arc.lock().unwrap();
                let msg = err.clone().unwrap_or_else(|| "Task error".to_string());
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
        let cancelled = {
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
        cancelled.store(true, AtomicOrdering::Release);
        Ok(Value::Bool(true))
    }

    // -------------------------------------------------------------------------
    // Schedules
    // -------------------------------------------------------------------------

    fn register_schedule(&self) -> (u64, Arc<AtomicBool>, Arc<AtomicBool>) {
        let id = self.next_id();
        let cancelled = Arc::new(AtomicBool::new(false));
        let tick_running = Arc::new(AtomicBool::new(false));
        let entry = ScheduleEntry {
            cancelled: Arc::clone(&cancelled),
            tick_running: Arc::clone(&tick_running),
        };
        if let Ok(mut schedules) = self.schedules.lock() {
            schedules.insert(id, entry);
        }
        (id, cancelled, tick_running)
    }

    fn cancel_schedule(&self, schedule_id: u64) -> bool {
        // Set flag AND remove from registry (rule 14)
        let mut schedules = match self.schedules.lock() {
            Ok(s) => s,
            Err(_) => return false,
        };
        if let Some(entry) = schedules.remove(&schedule_id) {
            entry.cancelled.store(true, AtomicOrdering::Release);
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
                entry.cancelled.store(true, AtomicOrdering::Release);
            }
        }
        // Cancel all schedules and remove them
        if let Ok(mut schedules) = self.schedules.lock() {
            for (_id, entry) in schedules.iter() {
                entry.cancelled.store(true, AtomicOrdering::Release);
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

// =============================================================================
// Handle value helpers
// =============================================================================

fn create_handle_value(kind: &str, id: u64) -> Value {
    match kind {
        "Task" => Value::TaskHandle(id),
        "Channel" => Value::ChannelHandle(id),
        "Schedule" => Value::ScheduleHandle(id),
        _ => unreachable!("Unknown handle kind: {}", kind),
    }
}

fn get_handle_id(handle: &Value, expected_type: &str) -> Result<u64> {
    match (handle, expected_type) {
        (Value::TaskHandle(id), "Task") => Ok(*id),
        (Value::ChannelHandle(id), "Channel") => Ok(*id),
        (Value::ScheduleHandle(id), "Schedule") => Ok(*id),
        // Wrong handle type
        (Value::TaskHandle(_), _) => Err(IntentError::type_error(format!(
            "Expected a {} handle, got a Task handle",
            expected_type
        ))),
        (Value::ChannelHandle(_), _) => Err(IntentError::type_error(format!(
            "Expected a {} handle, got a Channel handle",
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
    if s.ends_with("ms") {
        let num: u64 = s[..s.len() - 2]
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", s)))?;
        Ok(Duration::from_millis(num))
    } else if s.ends_with('s') {
        let num: u64 = s[..s.len() - 1]
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", s)))?;
        Ok(Duration::from_secs(num))
    } else if s.ends_with('m') {
        let num: u64 = s[..s.len() - 1]
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", s)))?;
        Ok(Duration::from_secs(num * 60))
    } else if s.ends_with('h') {
        let num: u64 = s[..s.len() - 1]
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", s)))?;
        Ok(Duration::from_secs(num * 3600))
    } else {
        // Try as plain milliseconds
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
/// Separates serializable values from native function names.
#[derive(Clone)]
struct CapturedBindings {
    /// Serializable values (Int, Float, Bool, String, Array, Map, Struct, Enum)
    values: HashMap<String, SerializedValue>,
    /// NativeFunction bindings — stored with arity for disambiguation when
    /// multiple modules export the same function name.
    native_fn_names: Vec<CapturedNativeFn>,
}

#[derive(Clone)]
struct CapturedNativeFn {
    binding_name: String, // the variable name in user code (may be an alias)
    fn_name: String,      // the canonical function name
    arity: usize,         // for disambiguation when names collide
    max_arity: usize,
}

/// Capture all bindings from an environment for cross-thread use.
/// Suppresses warnings for NativeFunction (expected, not user error).
/// Only warns for user-defined non-serializable values (closures).
fn capture_bindings(
    bindings: &HashMap<String, Value>,
) -> std::result::Result<CapturedBindings, Vec<String>> {
    let mut values = HashMap::new();
    let mut native_fn_names = Vec::new();
    let mut non_serializable_closures = Vec::new();

    for (key, value) in bindings {
        match value {
            Value::NativeFunction {
                name,
                arity,
                max_arity,
                ..
            } => {
                // Record function name + arity for re-lookup in child interpreter.
                // Arity disambiguates when modules share names (e.g., connect, query).
                native_fn_names.push(CapturedNativeFn {
                    binding_name: key.clone(),
                    fn_name: name.clone(),
                    arity: *arity,
                    max_arity: *max_arity,
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

    Ok(CapturedBindings {
        values,
        native_fn_names,
    })
}

/// Inject captured bindings into a fresh interpreter.
/// - Serializable values are defined directly.
/// - NativeFunction names are looked up from stdlib modules AND builtins,
///   with arity used for disambiguation when multiple modules share a name.
fn inject_captured(interp: &mut crate::interpreter::Interpreter, captured: &CapturedBindings) {
    // Inject serializable values
    for (key, val) in &captured.values {
        interp.define_global(key.clone(), val.to_value());
    }

    // Re-inject native functions
    for cap in &captured.native_fn_names {
        // First: search loaded modules for this function name
        let all_matches = interp.find_all_in_loaded_modules(&cap.fn_name);

        // Filter to matches with same arity
        let arity_matches: Vec<_> = all_matches
            .iter()
            .filter(|(_, value)| {
                if let Value::NativeFunction {
                    arity, max_arity, ..
                } = value
                {
                    *arity == cap.arity && *max_arity == cap.max_arity
                } else {
                    false
                }
            })
            .collect();

        if arity_matches.len() > 1 {
            let module_names: Vec<&str> = arity_matches.iter().map(|(m, _)| m.as_str()).collect();
            eprintln!(
                "[ERROR] Ambiguous native function capture: '{}' found in multiple modules ({}). Import the specific function to disambiguate.",
                cap.fn_name,
                module_names.join(", ")
            );
            continue;
        }

        if let Some((_, value)) = arity_matches.into_iter().next() {
            interp.define_global(cap.binding_name.clone(), value.clone());
            continue;
        }

        // Fallback: check if it's a builtin already in the global environment
        // (builtins like len, print, str are defined by Interpreter::new())
        if let Some(value) = interp.get_global(&cap.fn_name) {
            if let Value::NativeFunction {
                arity, max_arity, ..
            } = &value
            {
                if *arity == cap.arity && *max_arity == cap.max_arity {
                    interp.define_global(cap.binding_name.clone(), value);
                }
            }
        }
    }
}

// =============================================================================
// Public API functions (called from NativeFunction dispatch)
// =============================================================================

// --- Channels ---

fn concurrent_channel() -> Result<Value> {
    let id = RUNTIME.create_channel();
    Ok(create_handle_value("Channel", id))
}

fn concurrent_send(ch: &Value, value: &Value) -> Result<Value> {
    let id = get_handle_id(ch, "Channel")?;
    // Handles cannot be sent through channels (they're process-local references)
    if matches!(
        value,
        Value::TaskHandle(_) | Value::ChannelHandle(_) | Value::ScheduleHandle(_)
    ) {
        return Err(IntentError::type_error(
            "Handles (Task, Channel, Schedule) cannot be sent through channels".to_string(),
        ));
    }
    let serialized = SerializedValue::from_value(value)?;
    Ok(Value::Bool(RUNTIME.send(id, serialized)))
}

fn concurrent_recv(ch: &Value) -> Result<Value> {
    let id = get_handle_id(ch, "Channel")?;
    RUNTIME.recv(id)
}

fn concurrent_recv_timeout(ch: &Value, timeout_ms: i64) -> Result<Value> {
    let id = get_handle_id(ch, "Channel")?;
    RUNTIME.recv_timeout(id, timeout_ms)
}

fn concurrent_try_recv(ch: &Value) -> Result<Value> {
    let id = get_handle_id(ch, "Channel")?;
    RUNTIME.try_recv(id)
}

fn concurrent_close(ch: &Value) -> Result<Value> {
    let id = get_handle_id(ch, "Channel")?;
    Ok(Value::Bool(RUNTIME.close_channel(id)))
}

// --- Tasks ---

/// `spawn(handler)` — spawn a zero-parameter function as a background task.
/// The handler's closure environment is serialized for cross-thread use.
/// All eval_block calls are wrapped in catch_unwind.
fn concurrent_spawn(handler: &Value) -> Result<Value> {
    // Reap expired tasks before spawning (rule 12)
    RUNTIME.reap_expired_tasks();

    // Validate: must be a Function with no parameters (including defaults) (rule 27)
    match handler {
        Value::Function {
            params,
            closure,
            body,
            ..
        } => {
            if !params.is_empty() {
                return Err(IntentError::runtime_error(
                    "spawn() handler must be a zero-parameter function. Got a function with parameters.".to_string(),
                ));
            }

            // Capture environment bindings
            let bindings = closure.borrow().all_bindings();
            let captured = match capture_bindings(&bindings) {
                Ok(c) => c,
                Err(names) => {
                    return Err(IntentError::runtime_error(format!(
                        "Cannot capture user-defined function(s) across task boundaries: {}.                          Use closure capture for data, not function references.",
                        names.join(", ")
                    )));
                }
            };
            let body_clone = body.clone();

            // Create cancellation flag and register task
            let cancelled = Arc::new(AtomicBool::new(false));
            let task_id = RUNTIME.register_task(Arc::clone(&cancelled));

            // Get Arcs for the task thread (lock → clone → drop)
            let (state_arc, result_arc, error_arc, _cancelled_arc, completed_at_arc) =
                RUNTIME.get_task_arcs(task_id).unwrap();

            // Spawn thread
            thread::spawn(move || {
                // Install cancellation flag in thread-local
                CURRENT_TASK_CANCELLED.with(|cell| {
                    *cell.borrow_mut() = Some(cancelled);
                });

                // Create a fresh interpreter and inject captured bindings.
                // Interpreter::new() gives us builtins + stdlib modules.
                // inject_captured adds serializable values + re-injects native functions.
                let result = catch_unwind(AssertUnwindSafe(|| {
                    use crate::interpreter::Interpreter;

                    let mut interp = Interpreter::new();
                    inject_captured(&mut interp, &captured);
                    interp.eval_block(&body_clone)
                }));

                // Process result and update task state
                match result {
                    Ok(Ok(value)) => {
                        // Successful completion
                        match SerializedValue::from_value(&value) {
                            Ok(serialized) => {
                                *result_arc.lock().unwrap() = Some(serialized);
                            }
                            Err(_) => {
                                // Result not serializable — store Unit
                                *result_arc.lock().unwrap() = Some(SerializedValue::Unit);
                            }
                        }
                        *state_arc.lock().unwrap() = TaskState::Completed;
                    }
                    Ok(Err(e)) => {
                        // Runtime error (including cancellation)
                        *error_arc.lock().unwrap() = Some(format!("{}", e));
                        *state_arc.lock().unwrap() = TaskState::Failed;
                    }
                    Err(panic_info) => {
                        // Panic
                        let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else {
                            "Task panicked".to_string()
                        };
                        *error_arc.lock().unwrap() = Some(msg);
                        *state_arc.lock().unwrap() = TaskState::Panicked;
                    }
                }

                // Record completion time for reaper
                *completed_at_arc.lock().unwrap() = Some(Instant::now());

                // Clear thread-local
                CURRENT_TASK_CANCELLED.with(|cell| {
                    *cell.borrow_mut() = None;
                });
            });

            Ok(create_handle_value("Task", task_id))
        }
        _ => Err(IntentError::type_error(
            "spawn() requires a function".to_string(),
        )),
    }
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

// --- sleep_ms (cancellation-aware) ---

/// sleep_ms(ms) — cancellation-aware sleep. Loops in 50ms slices checking cancellation.
fn concurrent_sleep_ms(ms: i64) -> Result<Value> {
    if ms <= 0 {
        return Ok(Value::Unit);
    }

    let deadline = Instant::now() + Duration::from_millis(ms as u64);
    loop {
        check_cancellation()?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(Value::Unit);
        }
        let slice = remaining.min(Duration::from_millis(50));
        thread::sleep(slice);
    }
}

// --- after(delay, handler) ---

/// `after(delay, handler)` — run handler after a delay. Returns a Task handle.
fn concurrent_after(delay: &Value, handler: &Value) -> Result<Value> {
    // Reap expired tasks (rule 12)
    RUNTIME.reap_expired_tasks();

    // Parse delay
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

    // Validate: must be a Function with no parameters (rule 27)
    match handler {
        Value::Function {
            params,
            closure,
            body,
            ..
        } => {
            if !params.is_empty() {
                return Err(IntentError::runtime_error(
                    "after() handler must be a zero-parameter function. Got a function with parameters.".to_string(),
                ));
            }

            let bindings = closure.borrow().all_bindings();
            let captured = match capture_bindings(&bindings) {
                Ok(c) => c,
                Err(names) => {
                    return Err(IntentError::runtime_error(format!(
                        "Cannot capture user-defined function(s) across task boundaries: {}.                          Use closure capture for data, not function references.",
                        names.join(", ")
                    )));
                }
            };
            let body_clone = body.clone();

            let cancelled = Arc::new(AtomicBool::new(false));
            let task_id = RUNTIME.register_task(Arc::clone(&cancelled));
            let (state_arc, result_arc, error_arc, _cancelled_arc, completed_at_arc) =
                RUNTIME.get_task_arcs(task_id).unwrap();

            let cancelled_for_thread = Arc::clone(&cancelled);

            thread::spawn(move || {
                // Install cancellation flag
                CURRENT_TASK_CANCELLED.with(|cell| {
                    *cell.borrow_mut() = Some(Arc::clone(&cancelled_for_thread));
                });

                // Cancellation-aware delay (50ms slices)
                let deadline = Instant::now() + delay_duration;
                loop {
                    if cancelled_for_thread.load(AtomicOrdering::Acquire) {
                        *error_arc.lock().unwrap() = Some("Task cancelled".to_string());
                        *state_arc.lock().unwrap() = TaskState::Failed;
                        *completed_at_arc.lock().unwrap() = Some(Instant::now());
                        return;
                    }
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        break;
                    }
                    thread::sleep(remaining.min(Duration::from_millis(50)));
                }

                // Execute handler with catch_unwind
                let result = catch_unwind(AssertUnwindSafe(|| {
                    use crate::interpreter::Interpreter;

                    let mut interp = Interpreter::new();
                    inject_captured(&mut interp, &captured);
                    interp.eval_block(&body_clone)
                }));

                match result {
                    Ok(Ok(value)) => {
                        match SerializedValue::from_value(&value) {
                            Ok(serialized) => {
                                *result_arc.lock().unwrap() = Some(serialized);
                            }
                            Err(_) => {
                                *result_arc.lock().unwrap() = Some(SerializedValue::Unit);
                            }
                        }
                        *state_arc.lock().unwrap() = TaskState::Completed;
                    }
                    Ok(Err(e)) => {
                        *error_arc.lock().unwrap() = Some(format!("{}", e));
                        *state_arc.lock().unwrap() = TaskState::Failed;
                    }
                    Err(panic_info) => {
                        let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else {
                            "Task panicked".to_string()
                        };
                        *error_arc.lock().unwrap() = Some(msg);
                        *state_arc.lock().unwrap() = TaskState::Panicked;
                    }
                }

                *completed_at_arc.lock().unwrap() = Some(Instant::now());

                CURRENT_TASK_CANCELLED.with(|cell| {
                    *cell.borrow_mut() = None;
                });
            });

            Ok(create_handle_value("Task", task_id))
        }
        _ => Err(IntentError::type_error(
            "after() requires a function".to_string(),
        )),
    }
}

// --- schedule(interval, handler) ---

/// `schedule(interval, handler)` — run handler repeatedly at the given interval.
/// Returns a Schedule handle. Zero-duration intervals are rejected.
fn concurrent_schedule(interval: &Value, handler: &Value) -> Result<Value> {
    // Parse interval
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

    // Validate: must be a Function with no parameters (rule 27)
    match handler {
        Value::Function {
            params,
            closure,
            body,
            ..
        } => {
            if !params.is_empty() {
                return Err(IntentError::runtime_error(
                    "schedule() handler must be a zero-parameter function. Got a function with parameters.".to_string(),
                ));
            }

            let bindings = closure.borrow().all_bindings();
            let captured = match capture_bindings(&bindings) {
                Ok(c) => c,
                Err(names) => {
                    return Err(IntentError::runtime_error(format!(
                        "Cannot capture user-defined function(s) across task boundaries: {}.                          Use closure capture for data, not function references.",
                        names.join(", ")
                    )));
                }
            };
            let body_clone = body.clone();

            let (schedule_id, cancelled, tick_running) = RUNTIME.register_schedule();

            thread::spawn(move || {
                loop {
                    // Cancellation-aware sleep (50ms slices) — rule 15
                    let deadline = Instant::now() + interval_duration;
                    loop {
                        if cancelled.load(AtomicOrdering::Acquire) {
                            return; // schedule cancelled
                        }
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        thread::sleep(remaining.min(Duration::from_millis(50)));
                    }

                    // Check cancellation again after sleep
                    if cancelled.load(AtomicOrdering::Acquire) {
                        return;
                    }

                    // Overlap prevention (rule 17): skip if previous tick still running
                    if tick_running
                        .compare_exchange(
                            false,
                            true,
                            AtomicOrdering::Acquire,
                            AtomicOrdering::Acquire,
                        )
                        .is_err()
                    {
                        continue; // previous tick still running, skip this one
                    }

                    // Clone what the tick thread needs
                    let tick_captured = captured.clone();
                    let tick_body = body_clone.clone();
                    let tick_running_clone = Arc::clone(&tick_running);

                    // Spawn tick execution in a separate thread with catch_unwind (rule 17)
                    thread::spawn(move || {
                        let _result = catch_unwind(AssertUnwindSafe(|| {
                            use crate::interpreter::Interpreter;

                            let mut interp = Interpreter::new();
                            inject_captured(&mut interp, &tick_captured);
                            let _ = interp.eval_block(&tick_body);
                        }));

                        // Reset overlap flag — even on panic (catch_unwind ensures this runs)
                        tick_running_clone.store(false, AtomicOrdering::Release);
                    });
                }
            });

            Ok(create_handle_value("Schedule", schedule_id))
        }
        _ => Err(IntentError::type_error(
            "schedule() requires a function".to_string(),
        )),
    }
}

fn concurrent_cancel_schedule(handle: &Value) -> Result<Value> {
    let id = get_handle_id(handle, "Schedule")?;
    Ok(Value::Bool(RUNTIME.cancel_schedule(id)))
}

// --- select(channels, timeout_ms?) ---

/// `select(channels, timeout_ms?)` — wait for the first value from any of the given channels.
/// Returns `{channel: <handle>, value: <received>}` on success,
/// `{status: "timeout"}` on timeout, `{status: "closed"}` if all channels are closed.
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
            Value::ChannelHandle(id) => *id,
            _ => {
                return Err(IntentError::type_error(format!(
                    "select() channel at index {} is not a Channel handle, got {}",
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
                        result.insert(
                            "channel".to_string(),
                            Value::ChannelHandle(channel_ids[orig_index]),
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
    let mut module = HashMap::new();

    // @ntnt channel
    // @module std/concurrent
    // @module_description Structured concurrency: tasks, channels, schedules, and cooperative cancellation
    // @signature channel() -> Channel
    // Creates a new unbounded channel for inter-task communication.
    // Channels are single-consumer: only one task should call recv() at a time.
    // @returns Channel handle
    // @see_also send, recv, close
    // @since v0.2.0
    // @example channel() ~ "Create a channel for inter-task communication"
    module.insert(
        "channel".to_string(),
        Value::NativeFunction {
            name: "channel".to_string(),
            arity: 0,
            max_arity: 0,
            func: |_args| concurrent_channel(),
        },
    );

    // @ntnt send
    // @module std/concurrent
    // @signature send(ch: Channel, value: Any) -> Bool
    // Sends a value through a channel. Returns false if the channel has been closed.
    // Serializable types: Int, Float, Bool, String, Array, Map, Struct, Enum.
    // @param ch The channel handle
    // @param value The value to send (must be serializable)
    // @see_also channel, recv
    // @since v0.2.0
    // @example send(ch, "hello") => true ~ "Send a string through the channel"
    module.insert(
        "send".to_string(),
        Value::NativeFunction {
            name: "send".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| concurrent_send(&args[0], &args[1]),
        },
    );

    // @ntnt recv
    // @module std/concurrent
    // @signature recv(ch: Channel) -> Any
    // Receives a value from a channel. Blocks until a value is available.
    // Returns Unit if the channel is closed and empty (sender dropped).
    // This is a cancellation yield point: a cancelled task will exit here.
    // Single-consumer: the receiver lock is held for the blocking duration.
    // @param ch The channel handle
    // @see_also channel, send, try_recv, recv_timeout
    // @since v0.2.0
    // @example recv(ch) ~ "Block until a value is received"
    module.insert(
        "recv".to_string(),
        Value::NativeFunction {
            name: "recv".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_recv(&args[0]),
        },
    );

    // @ntnt recv_timeout
    // @module std/concurrent
    // @signature recv_timeout(ch: Channel, millis: Int) -> Option<Any>
    // Receives with timeout. Returns None if timeout expires or channel disconnected.
    // Loops in ≤100ms slices checking cancellation between iterations.
    // This is a cancellation yield point.
    // @param ch The channel handle
    // @param millis Timeout in milliseconds (negative values clamped to 0)
    // @see_also recv, try_recv
    // @since v0.2.0
    // @example recv_timeout(ch, 5000) ~ "Wait up to 5 seconds for a value"
    module.insert(
        "recv_timeout".to_string(),
        Value::NativeFunction {
            name: "recv_timeout".to_string(),
            arity: 2,
            max_arity: 2,
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
    // @signature try_recv(ch: Channel) -> Option<Any>
    // Non-blocking receive. Returns None if no value is available or channel is closed.
    // @param ch The channel handle
    // @see_also recv, recv_timeout
    // @since v0.2.0
    // @example try_recv(ch) ~ "Check for a value without blocking"
    module.insert(
        "try_recv".to_string(),
        Value::NativeFunction {
            name: "try_recv".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_try_recv(&args[0]),
        },
    );

    // @ntnt close
    // @module std/concurrent
    // @signature close(ch: Channel) -> Bool
    // Closes a channel by removing it from the registry. The sender is dropped,
    // causing any blocking recv() to return Unit. Returns true if the channel existed.
    // @param ch The channel handle
    // @see_also channel
    // @since v0.2.0
    // @example close(ch) => true ~ "Close the channel"
    module.insert(
        "close".to_string(),
        Value::NativeFunction {
            name: "close".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_close(&args[0]),
        },
    );

    // @ntnt select
    // @module std/concurrent
    // @signature select(channels: Array<Channel>, timeout_ms?: Int | String) -> Map
    // Waits for the first available value from any of the given channels.
    // Returns a map with "channel" (the handle that fired) and "value" (the received value).
    // On timeout: returns {"status": "timeout"}.
    // If all channels are closed: returns {"status": "closed"}.
    // This is a cancellation yield point.
    // @param channels Array of Channel handles to wait on
    // @param timeout_ms Optional timeout in milliseconds (Int) or as a string interval
    // @returns Map with channel/value on success, or status on timeout/closed
    // @see_also channel, recv, recv_timeout
    // @since v0.5.0
    // @example select([ch_a, ch_b]) ~ "Wait for first value from either channel"
    // @example select([ch_a, ch_b], 5000) ~ "Wait up to 5 seconds"
    module.insert(
        "select".to_string(),
        Value::NativeFunction {
            name: "select".to_string(),
            arity: 1,
            max_arity: 2,
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
    // @since v0.5.0
    // @example spawn(fn() { 42 }) ~ "Spawn a background task"
    module.insert(
        "spawn".to_string(),
        Value::NativeFunction {
            name: "spawn".to_string(),
            arity: 1,
            max_arity: 1,
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
    // @since v0.5.0
    // @example await_task(task) => Ok(42) ~ "Wait for task result"
    module.insert(
        "await_task".to_string(),
        Value::NativeFunction {
            name: "await_task".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_await_task(&args[0]),
        },
    );

    // @ntnt try_await
    // @module std/concurrent
    // @signature try_await(task: Task) -> Map
    // Non-blocking peek at task state. Does NOT remove the task from registry.
    // Returns a map with "status" ("running", "completed", "failed", "panicked")
    // and "result" (Ok(value), Err(message), or None if still running).
    // @param task The task handle
    // @returns Map with status and result fields
    // @see_also spawn, await_task, cancel_task
    // @since v0.5.0
    // @example try_await(task) => {"status": "running", "result": None} ~ "Check task status"
    module.insert(
        "try_await".to_string(),
        Value::NativeFunction {
            name: "try_await".to_string(),
            arity: 1,
            max_arity: 1,
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
    // @since v0.5.0
    // @example cancel_task(task) => true ~ "Cancel a running task"
    module.insert(
        "cancel_task".to_string(),
        Value::NativeFunction {
            name: "cancel_task".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_cancel_task(&args[0]),
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
    // @since v0.5.0
    // @example after(1000, fn() { print("delayed!") }) ~ "Run after 1 second"
    module.insert(
        "after".to_string(),
        Value::NativeFunction {
            name: "after".to_string(),
            arity: 2,
            max_arity: 2,
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
    // @since v0.5.0
    // @example schedule(5000, fn() { print("tick") }) ~ "Run every 5 seconds"
    module.insert(
        "schedule".to_string(),
        Value::NativeFunction {
            name: "schedule".to_string(),
            arity: 2,
            max_arity: 2,
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
    // @since v0.5.0
    // @example cancel_schedule(sched) => true ~ "Cancel a scheduled task"
    module.insert(
        "cancel_schedule".to_string(),
        Value::NativeFunction {
            name: "cancel_schedule".to_string(),
            arity: 1,
            max_arity: 1,
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
    // @since v0.5.0
    // @example sleep_ms(1000) ~ "Sleep for 1 second (cancellation-aware)"
    module.insert(
        "sleep_ms".to_string(),
        Value::NativeFunction {
            name: "sleep_ms".to_string(),
            arity: 1,
            max_arity: 1,
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
    // @since v0.2.0
    // @example thread_count() => 8 ~ "Number of CPU threads"
    module.insert(
        "thread_count".to_string(),
        Value::NativeFunction {
            name: "thread_count".to_string(),
            arity: 0,
            max_arity: 0,
            func: |_args| concurrent_thread_count(),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let ch = concurrent_channel().unwrap();
        assert!(matches!(ch, Value::ChannelHandle(_)));
    }

    #[test]
    fn test_channel_send_recv() {
        let ch = concurrent_channel().unwrap();
        let sent = concurrent_send(&ch, &Value::String("hello".to_string())).unwrap();
        assert!(matches!(sent, Value::Bool(true)));
        let received = concurrent_recv(&ch).unwrap();
        assert!(matches!(received, Value::String(s) if s == "hello"));
    }

    #[test]
    fn test_try_recv_empty() {
        let ch = concurrent_channel().unwrap();
        let result = concurrent_try_recv(&ch).unwrap();
        match result {
            Value::EnumValue { variant, .. } => assert_eq!(variant, "None"),
            _ => panic!("Expected Option::None"),
        }
    }

    #[test]
    fn test_channel_close_removes_from_registry() {
        let ch = concurrent_channel().unwrap();
        let id = get_handle_id(&ch, "Channel").unwrap();

        // Channel should exist
        assert!(RUNTIME.channels.lock().unwrap().contains_key(&id));

        // Close removes it
        let closed = concurrent_close(&ch).unwrap();
        assert!(matches!(closed, Value::Bool(true)));

        // Channel should be gone
        assert!(!RUNTIME.channels.lock().unwrap().contains_key(&id));

        // Send on closed channel returns false
        let sent = concurrent_send(&ch, &Value::Int(42)).unwrap();
        assert!(matches!(sent, Value::Bool(false)));

        // Close again returns false
        let closed2 = concurrent_close(&ch).unwrap();
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
        let ch = concurrent_channel().unwrap();
        // Negative timeout should be clamped to 0 and return None immediately
        let result = concurrent_recv_timeout(&ch, -100).unwrap();
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
        assert!(get_handle_id(&handle, "Channel").is_err());
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
        // Create a task handle with a known cancelled flag
        let cancelled = Arc::new(AtomicBool::new(false));
        let task_id = RUNTIME.register_task(Arc::clone(&cancelled));
        let handle = create_handle_value("Task", task_id);

        // Cancel should set the flag
        let result = concurrent_cancel_task(&handle).unwrap();
        assert!(matches!(result, Value::Bool(true)));
        assert!(cancelled.load(AtomicOrdering::Acquire));

        // Task state should still be Running (cooperative — not forced)
        let (state_arc, _, _, _, _) = RUNTIME.get_task_arcs(task_id).unwrap();
        assert_eq!(*state_arc.lock().unwrap(), TaskState::Running);
    }

    #[test]
    fn test_cancel_schedule_removes_from_registry() {
        let (schedule_id, cancelled, _tick_running) = RUNTIME.register_schedule();
        let handle = create_handle_value("Schedule", schedule_id);

        // Should exist
        assert!(RUNTIME.schedules.lock().unwrap().contains_key(&schedule_id));

        // Cancel removes
        let result = concurrent_cancel_schedule(&handle).unwrap();
        assert!(matches!(result, Value::Bool(true)));

        // Flag set
        assert!(cancelled.load(AtomicOrdering::Acquire));

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
                func: |_| Ok(Value::Unit),
            },
        );

        let captured = capture_bindings(&bindings).expect("should succeed with no closures");
        // Serializable values captured
        assert!(captured.values.contains_key("x"));
        assert!(!captured.values.contains_key("native_fn"));
        // Native function recorded for re-lookup with arity
        assert!(captured
            .native_fn_names
            .iter()
            .any(|cap| cap.binding_name == "native_fn" && cap.fn_name == "test"));
    }
}
