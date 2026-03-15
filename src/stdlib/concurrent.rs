//! std/concurrent module - Concurrency primitives
//!
//! All mutable concurrency state lives in a single `ConcurrencyRuntime` struct,
//! accessed through `static RUNTIME`. Public free functions are thin wrappers.
//!
//! ## Lock discipline (invariant)
//!
//! **No method on `ConcurrencyRuntime` ever holds two locks simultaneously.**
//! Every method: acquire one registry lock → extract Arcs/clones → drop lock → operate.
//! This eliminates deadlocks by construction.
//!
//! ## Cleanup model
//!
//! - **Tasks:** `await_task()` removes from map (consume semantics). `try_await()` peeks.
//!   Fire-and-forget tasks are reaped by `reap_stale_tasks()` on each `spawn_task()`/`after()`.
//!   Stale = terminal state + older than 5 minutes.
//! - **Channels:** `close()` = `remove()`. Dropping the Sender causes recv → Disconnected.
//!   The Receiver lives in an Arc held by any active recv() caller.
//! - **Schedules:** `cancel_schedule()` sets the AtomicBool. Thread exits on next check.
//!
//! ## Atomic ordering
//!
//! ALL atomics: `Release` for stores, `Acquire` for loads. No SeqCst, no Relaxed.
//!
//! ```ntnt
//! import { channel, send, recv, try_recv, close, spawn, await_task,
//!          try_await, cancel_task } from "std/concurrent"
//!
//! // Create a channel for communication
//! let ch = channel()
//!
//! // Spawn a background task
//! let task = spawn(fn() { return 42 })
//! let result = await_task(task)  // result == Ok(42)
//!
//! // Spawn with channel communication
//! let ch = channel()
//! spawn(fn() { send(ch, "hello from task") })
//! let msg = recv(ch)  // "hello from task"
//! ```

use crate::ast::Block;
use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, LazyLock, Mutex};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

// ============================================================
// Serialized Values (thread-safe value transport)
// ============================================================

/// Serialized value for thread-safe transmission.
/// Only primitive and composite types that can be cloned.
#[derive(Debug, Clone)]
pub enum SerializedValue {
    Unit,
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    Array(Vec<SerializedValue>),
    Map(HashMap<String, SerializedValue>),
}

impl SerializedValue {
    /// Convert from Value to SerializedValue (only safe types)
    pub fn from_value(value: &Value) -> Result<Self> {
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
                serialized.insert("__type".to_string(), SerializedValue::String(name.clone()));
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
            _ => Err(IntentError::type_error(
                "Only primitive types (Int, Float, String, Bool, Array, Map) can be sent through channels".to_string()
            )),
        }
    }

    /// Convert back to Value
    pub fn to_value(&self) -> Value {
        match self {
            SerializedValue::Unit => Value::Unit,
            SerializedValue::Int(i) => Value::Int(*i),
            SerializedValue::Float(f) => Value::Float(*f),
            SerializedValue::Bool(b) => Value::Bool(*b),
            SerializedValue::String(s) => Value::String(s.clone()),
            SerializedValue::Array(arr) => Value::Array(arr.iter().map(|v| v.to_value()).collect()),
            SerializedValue::Map(map) => {
                // Check for special __enum marker
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
                // Check for special __type marker (struct)
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
        }
    }
}

// ============================================================
// Cooperative Cancellation (thread-local)
// ============================================================

thread_local! {
    /// Thread-local cancellation flag for the current spawned task.
    /// Set when a task thread starts, checked at yield points.
    static CURRENT_TASK_CANCELLED: std::cell::RefCell<Option<Arc<AtomicBool>>> =
        const { std::cell::RefCell::new(None) };
}

/// Set the cancellation flag for the current thread (used when a task starts running).
fn set_current_task_cancelled(flag: Arc<AtomicBool>) {
    CURRENT_TASK_CANCELLED.with(|f| {
        *f.borrow_mut() = Some(flag);
    });
}

/// Check if the current task context is cancelled.
/// Called from cooperative cancellation points (recv, sleep_ms, fetch).
/// Returns Err if cancelled, Ok(()) if not.
pub fn check_cancellation() -> Result<()> {
    CURRENT_TASK_CANCELLED.with(|flag| {
        if let Some(ref cancelled) = *flag.borrow() {
            if cancelled.load(Ordering::Acquire) {
                return Err(IntentError::runtime_error("Task cancelled".to_string()));
            }
        }
        Ok(())
    })
}

// ============================================================
// ConcurrencyRuntime — single owner of all concurrency state
// ============================================================

/// Task lifecycle state.
#[derive(Debug, Clone)]
pub enum TaskState {
    Pending,
    Running,
    Completed(SerializedValue),
    Failed(String),
    Cancelled,
}

/// One entry per spawned task (spawn + after).
struct TaskEntry {
    state: Arc<(Mutex<TaskState>, Condvar)>,
    cancelled: Arc<AtomicBool>,
    created_at: Instant,
}

/// One entry per channel.
struct ChannelEntry {
    sender: mpsc::Sender<SerializedValue>,
    receiver: Arc<Mutex<mpsc::Receiver<SerializedValue>>>,
}

/// One entry per schedule.
struct ScheduleEntry {
    cancelled: Arc<AtomicBool>,
}

/// Single owner of all concurrency state. RAII cleanup.
///
/// ## Lock discipline (invariant — enforced by code review)
///
/// No method on this struct ever holds two of {tasks, channels, schedules} simultaneously.
/// Pattern: lock → extract Arcs → drop lock → operate on Arcs.
pub struct ConcurrencyRuntime {
    /// Single monotonic ID counter for tasks, channels, and schedules.
    /// No collisions because they live in separate HashMaps.
    next_id: AtomicU64,

    /// All spawned tasks (spawn + after).
    tasks: Mutex<HashMap<u64, TaskEntry>>,

    /// All channels.
    channels: Mutex<HashMap<u64, ChannelEntry>>,

    /// All schedules.
    schedules: Mutex<HashMap<u64, ScheduleEntry>>,

    /// Global shutdown flag — set by `shutdown()`, checked by schedule loops.
    shutdown: AtomicBool,
}

/// Max age for stale task reaping (fire-and-forget tasks in terminal state).
const STALE_TASK_AGE: Duration = Duration::from_secs(300); // 5 minutes

impl ConcurrencyRuntime {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            tasks: Mutex::new(HashMap::new()),
            channels: Mutex::new(HashMap::new()),
            schedules: Mutex::new(HashMap::new()),
            shutdown: AtomicBool::new(false),
        }
    }

    /// Allocate a fresh ID (used for tasks, channels, and schedules).
    fn next_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Release)
    }

    // --------------------------------------------------------
    // Task methods
    // --------------------------------------------------------

    /// Spawn a function as a background task.
    /// The function's captured variables are serialized (snapshot at spawn time).
    /// The task runs on a new thread with its own Interpreter instance.
    pub fn spawn_task(&self, func: &Value) -> Result<Value> {
        // Extract function components
        let (params, body, closure_bindings) = extract_function_for_task(func, "spawn")?;

        // Validate: spawned functions must take no arguments
        if !params.is_empty() {
            return Err(IntentError::runtime_error(
                "spawn() function must take no arguments (use closure capture for data)"
                    .to_string(),
            ));
        }

        // Reap stale completed tasks before inserting new ones
        self.reap_stale_tasks();

        let task_id = self.next_id();
        let state = Arc::new((Mutex::new(TaskState::Pending), Condvar::new()));
        let cancelled = Arc::new(AtomicBool::new(false));

        // Insert into registry — lock, insert, drop.
        {
            let mut tasks = self.tasks.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
            })?;
            tasks.insert(
                task_id,
                TaskEntry {
                    state: Arc::clone(&state),
                    cancelled: Arc::clone(&cancelled),
                    created_at: Instant::now(),
                },
            );
        } // lock dropped

        // Spawn on a new thread
        let task_state = Arc::clone(&state);
        let task_cancelled = Arc::clone(&cancelled);

        thread::spawn(move || {
            run_task_thread(task_state, task_cancelled, body, closure_bindings);
        });

        Ok(create_task_value(task_id))
    }

    /// Block until task completes. Consumes the task from the registry.
    /// Returns Ok(value) on success, Err(message) on failure/cancellation.
    pub fn await_task(&self, task: &Value) -> Result<Value> {
        let id = get_task_id(task)?;

        // Extract the state Arc, then drop the registry lock.
        let state_arc = {
            let tasks = self.tasks.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
            })?;
            if let Some(entry) = tasks.get(&id) {
                Arc::clone(&entry.state)
            } else {
                return Err(IntentError::runtime_error(format!(
                    "Invalid task (id={})",
                    id
                )));
            }
        }; // lock dropped

        // Wait for task completion using condvar
        let (lock, cvar) = &*state_arc;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        let result = loop {
            match &*state {
                TaskState::Completed(sv) => {
                    break Ok(Value::ok(sv.to_value()));
                }
                TaskState::Failed(msg) => {
                    break Ok(Value::err(Value::String(msg.clone())));
                }
                TaskState::Cancelled => {
                    break Ok(Value::err(Value::String("Task cancelled".to_string())));
                }
                TaskState::Pending | TaskState::Running => {
                    state = cvar.wait(state).unwrap_or_else(|e| e.into_inner());
                }
            }
        };

        // Consume: remove completed task from registry
        drop(state); // release condvar lock first
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.remove(&id);
        }

        result
    }

    /// Non-blocking peek at task status. Does NOT consume from registry.
    /// Returns None if still running, Some(Result) if done.
    pub fn try_await_task(&self, task: &Value) -> Result<Value> {
        let id = get_task_id(task)?;

        let state_arc = {
            let tasks = self.tasks.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
            })?;
            if let Some(entry) = tasks.get(&id) {
                Arc::clone(&entry.state)
            } else {
                return Err(IntentError::runtime_error(format!(
                    "Invalid task (id={})",
                    id
                )));
            }
        }; // lock dropped

        let (lock, _cvar) = &*state_arc;
        let state = lock.lock().unwrap_or_else(|e| e.into_inner());

        match &*state {
            TaskState::Completed(sv) => Ok(Value::some(Value::ok(sv.to_value()))),
            TaskState::Failed(msg) => Ok(Value::some(Value::err(Value::String(msg.clone())))),
            TaskState::Cancelled => Ok(Value::some(Value::err(Value::String(
                "Task cancelled".to_string(),
            )))),
            TaskState::Pending | TaskState::Running => Ok(Value::none()),
        }
    }

    /// Set the cancellation flag. Cooperative — task checks at yield points.
    pub fn cancel_task(&self, task: &Value) -> Result<Value> {
        let id = get_task_id(task)?;

        // Extract Arcs, drop lock, then operate.
        let (cancelled, state_arc) = {
            let tasks = self.tasks.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
            })?;
            if let Some(entry) = tasks.get(&id) {
                (Arc::clone(&entry.cancelled), Arc::clone(&entry.state))
            } else {
                return Ok(Value::Bool(false));
            }
        }; // lock dropped

        cancelled.store(true, Ordering::Release);

        // Notify condvar so await_task can re-evaluate
        let (_lock, cvar) = &*state_arc;
        cvar.notify_all();

        Ok(Value::Bool(true))
    }

    /// Reap tasks in terminal state older than STALE_TASK_AGE.
    /// Called internally by spawn_task() and after() to bound registry size.
    fn reap_stale_tasks(&self) {
        let now = Instant::now();
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.retain(|_id, entry| {
                // Keep if not stale
                if now.duration_since(entry.created_at) < STALE_TASK_AGE {
                    return true;
                }
                // Keep if not in terminal state
                let (lock, _) = &*entry.state;
                if let Ok(state) = lock.try_lock() {
                    !matches!(
                        *state,
                        TaskState::Completed(_) | TaskState::Failed(_) | TaskState::Cancelled
                    )
                } else {
                    true // keep if we can't check (lock contended)
                }
            });
        }
    }

    // --------------------------------------------------------
    // Channel methods
    // --------------------------------------------------------

    /// Create a new unbounded channel.
    pub fn create_channel(&self) -> Result<Value> {
        let (tx, rx) = mpsc::channel();
        let id = self.next_id();

        {
            let mut channels = self.channels.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock channel registry: {}", e))
            })?;
            channels.insert(
                id,
                ChannelEntry {
                    sender: tx,
                    receiver: Arc::new(Mutex::new(rx)),
                },
            );
        } // lock dropped

        Ok(create_channel_value(id))
    }

    /// Send a value through a channel. Returns false if channel is closed/removed.
    pub fn send(&self, ch: &Value, value: &Value) -> Result<Value> {
        let id = get_channel_id(ch)?;
        let serialized = SerializedValue::from_value(value)?;

        // Extract sender clone, drop registry lock, then send.
        let sender = {
            let channels = self.channels.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock channel registry: {}", e))
            })?;
            if let Some(entry) = channels.get(&id) {
                entry.sender.clone()
            } else {
                // Channel was closed/removed — send fails gracefully
                return Ok(Value::Bool(false));
            }
        }; // lock dropped

        match sender.send(serialized) {
            Ok(_) => Ok(Value::Bool(true)),
            Err(_) => Ok(Value::Bool(false)),
        }
    }

    /// Receive a value, blocking until available.
    ///
    /// Note: channels are single-consumer — the receiver mutex is held for the
    /// full blocking duration, so only one active `recv()` caller per channel
    /// at a time. This matches Go's channel semantics.
    pub fn recv(&self, ch: &Value) -> Result<Value> {
        let id = get_channel_id(ch)?;

        check_cancellation()?;

        // Extract receiver Arc, drop registry lock.
        let receiver = {
            let channels = self.channels.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock channel registry: {}", e))
            })?;
            if let Some(entry) = channels.get(&id) {
                Arc::clone(&entry.receiver)
            } else {
                return Err(IntentError::runtime_error("Invalid channel".to_string()));
            }
        }; // lock dropped

        let rx = receiver
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock receiver: {}", e)))?;

        // Loop with timeout slices to allow cooperative cancellation
        loop {
            check_cancellation()?;
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(serialized) => return Ok(serialized.to_value()),
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Value::Unit),
            }
        }
    }

    /// Receive with timeout. Returns None on timeout or disconnect.
    pub fn recv_timeout(&self, ch: &Value, timeout_ms: i64) -> Result<Value> {
        let timeout_ms = timeout_ms.max(0) as u64;
        let id = get_channel_id(ch)?;

        check_cancellation()?;

        let receiver = {
            let channels = self.channels.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock channel registry: {}", e))
            })?;
            if let Some(entry) = channels.get(&id) {
                Arc::clone(&entry.receiver)
            } else {
                return Err(IntentError::runtime_error("Invalid channel".to_string()));
            }
        }; // lock dropped

        let rx = receiver
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock receiver: {}", e)))?;

        let mut remaining = timeout_ms;
        loop {
            check_cancellation()?;
            let slice = remaining.min(100);
            match rx.recv_timeout(Duration::from_millis(slice)) {
                Ok(serialized) => return Ok(Value::some(serialized.to_value())),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    remaining = remaining.saturating_sub(slice);
                    if remaining == 0 {
                        return Ok(Value::none());
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Value::none()),
            }
        }
    }

    /// Non-blocking receive. Returns None if empty.
    pub fn try_recv(&self, ch: &Value) -> Result<Value> {
        let id = get_channel_id(ch)?;

        let receiver = {
            let channels = self.channels.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock channel registry: {}", e))
            })?;
            if let Some(entry) = channels.get(&id) {
                Arc::clone(&entry.receiver)
            } else {
                return Err(IntentError::runtime_error("Invalid channel".to_string()));
            }
        }; // lock dropped

        let rx = receiver
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock receiver: {}", e)))?;

        match rx.try_recv() {
            Ok(serialized) => Ok(Value::some(serialized.to_value())),
            Err(mpsc::TryRecvError::Empty) => Ok(Value::none()),
            Err(mpsc::TryRecvError::Disconnected) => Ok(Value::none()),
        }
    }

    /// Close a channel by removing it from the map.
    /// Dropping the Sender causes recv → Disconnected. The Receiver lives in
    /// its Arc, so active recv() callers can still drain buffered messages.
    pub fn close_channel(&self, ch: &Value) -> Result<Value> {
        let id = get_channel_id(ch)?;

        let mut channels = self.channels.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock channel registry: {}", e))
        })?;

        if channels.remove(&id).is_some() {
            Ok(Value::Bool(true))
        } else {
            Ok(Value::Bool(false))
        }
    }

    // --------------------------------------------------------
    // Schedule methods
    // --------------------------------------------------------

    /// Schedule a function to run at a fixed interval.
    /// Overlap prevention: skips tick if previous execution still running.
    /// catch_unwind wraps every tick.
    pub fn schedule(
        &self,
        interval_str: &str,
        body: Block,
        closure_bindings: HashMap<String, SerializedValue>,
    ) -> Result<Value> {
        let duration = parse_interval(interval_str)?;

        if duration.is_zero() {
            return Err(IntentError::runtime_error(
                "schedule() interval must be greater than zero".to_string(),
            ));
        }

        let schedule_id = self.next_id();
        let cancelled = Arc::new(AtomicBool::new(false));

        {
            let mut schedules = self.schedules.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock schedule registry: {}", e))
            })?;
            schedules.insert(
                schedule_id,
                ScheduleEntry {
                    cancelled: Arc::clone(&cancelled),
                },
            );
        } // lock dropped

        let schedule_cancelled = Arc::clone(&cancelled);

        thread::spawn(move || {
            let executing = Arc::new(AtomicBool::new(false));

            loop {
                // Sleep in 50ms slices for cancellation responsiveness
                let total_ms = duration.as_millis() as u64;
                let mut slept = 0u64;
                while slept < total_ms {
                    if schedule_cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let slice = 50u64.min(total_ms - slept);
                    thread::sleep(Duration::from_millis(slice));
                    slept += slice;
                }

                if schedule_cancelled.load(Ordering::Acquire) {
                    break;
                }

                // Overlap prevention
                if executing.load(Ordering::Acquire) {
                    eprintln!("[schedule] Skipping tick — previous execution still running");
                    continue;
                }

                executing.store(true, Ordering::Release);
                let exec_executing = Arc::clone(&executing);
                let exec_bindings = closure_bindings.clone();
                let exec_body = body.clone();
                let exec_cancelled = Arc::clone(&schedule_cancelled);

                thread::spawn(move || {
                    set_current_task_cancelled(exec_cancelled);

                    let mut interpreter = crate::interpreter::Interpreter::new();
                    interpreter.define_all_stdlib_as_globals();
                    for (name, sv) in &exec_bindings {
                        interpreter.define_variable(name.clone(), sv.to_value());
                    }

                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        interpreter.eval_block(&exec_body)
                    }));

                    match result {
                        Ok(Err(e)) => eprintln!("[schedule] Error: {}", e),
                        Err(panic) => {
                            let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                                s.to_string()
                            } else if let Some(s) = panic.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic".to_string()
                            };
                            eprintln!("[schedule] Panic: {}", msg);
                        }
                        Ok(Ok(_)) => {}
                    }

                    exec_executing.store(false, Ordering::Release);
                });
            }
        });

        let mut sched = HashMap::new();
        sched.insert("_schedule_id".to_string(), Value::Int(schedule_id as i64));
        sched.insert("type".to_string(), Value::String("Schedule".to_string()));
        Ok(Value::Map(sched))
    }

    /// Cancel a schedule. Returns true if found.
    pub fn cancel_schedule(&self, schedule: &Value) -> Result<Value> {
        let id = get_schedule_id(schedule)?;

        let cancelled = {
            let schedules = self.schedules.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock schedule registry: {}", e))
            })?;
            if let Some(entry) = schedules.get(&id) {
                Arc::clone(&entry.cancelled)
            } else {
                return Ok(Value::Bool(false));
            }
        }; // lock dropped

        cancelled.store(true, Ordering::Release);
        Ok(Value::Bool(true))
    }

    // --------------------------------------------------------
    // After (delayed one-shot task)
    // --------------------------------------------------------

    /// Execute a function once after a delay (ms).
    /// Uses the task system — it's spawn + sleep.
    pub fn after(
        &self,
        delay_ms: i64,
        body: Block,
        closure_bindings: HashMap<String, SerializedValue>,
    ) -> Result<Value> {
        self.reap_stale_tasks();

        let task_id = self.next_id();
        let state = Arc::new((Mutex::new(TaskState::Pending), Condvar::new()));
        let cancelled = Arc::new(AtomicBool::new(false));

        {
            let mut tasks = self.tasks.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
            })?;
            tasks.insert(
                task_id,
                TaskEntry {
                    state: Arc::clone(&state),
                    cancelled: Arc::clone(&cancelled),
                    created_at: Instant::now(),
                },
            );
        } // lock dropped

        let task_state = Arc::clone(&state);
        let task_cancelled = Arc::clone(&cancelled);

        thread::spawn(move || {
            set_current_task_cancelled(Arc::clone(&task_cancelled));

            // Update state to Running
            {
                let (lock, cvar) = &*task_state;
                let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
                *s = TaskState::Running;
                cvar.notify_all();
            }

            // Sleep for the delay in 50ms cancellation-aware slices
            if delay_ms > 0 {
                let total = delay_ms as u64;
                let mut slept = 0u64;
                let increment = 50u64.min(total);
                while slept < total {
                    if task_cancelled.load(Ordering::Acquire) {
                        let (lock, cvar) = &*task_state;
                        let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
                        *s = TaskState::Cancelled;
                        cvar.notify_all();
                        return;
                    }
                    let sleep_for = increment.min(total - slept);
                    thread::sleep(Duration::from_millis(sleep_for));
                    slept += sleep_for;
                }
            }

            // Check cancellation after sleep
            if task_cancelled.load(Ordering::Acquire) {
                let (lock, cvar) = &*task_state;
                let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
                *s = TaskState::Cancelled;
                cvar.notify_all();
                return;
            }

            // Execute with catch_unwind
            let mut interpreter = crate::interpreter::Interpreter::new();
            interpreter.define_all_stdlib_as_globals();
            for (name, sv) in &closure_bindings {
                interpreter.define_variable(name.clone(), sv.to_value());
            }

            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                interpreter.eval_block(&body)
            }));

            let (lock, cvar) = &*task_state;
            let mut s = lock.lock().unwrap_or_else(|e| e.into_inner());
            if task_cancelled.load(Ordering::Acquire) {
                *s = TaskState::Cancelled;
            } else {
                match result {
                    Ok(Ok(value)) => {
                        let final_value = match value {
                            Value::Return(inner) => *inner,
                            other => other,
                        };
                        match SerializedValue::from_value(&final_value) {
                            Ok(sv) => *s = TaskState::Completed(sv),
                            Err(e) => {
                                *s = TaskState::Failed(format!("Cannot serialize result: {}", e))
                            }
                        }
                    }
                    Ok(Err(e)) => {
                        *s = TaskState::Failed(format!("{}", e));
                    }
                    Err(panic) => {
                        let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "unknown panic".to_string()
                        };
                        *s = TaskState::Failed(format!("panic: {}", msg));
                    }
                }
            }
            cvar.notify_all();
        });

        Ok(create_task_value(task_id))
    }

    // --------------------------------------------------------
    // Shutdown
    // --------------------------------------------------------

    /// Cancel everything: all tasks, all schedules. Called from interpreter shutdown.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::Release);

        // Cancel all tasks — extract Arcs first, drop lock, then operate.
        let task_handles: Vec<(Arc<AtomicBool>, Arc<(Mutex<TaskState>, Condvar)>)> = {
            if let Ok(tasks) = self.tasks.lock() {
                tasks
                    .iter()
                    .map(|(_id, entry)| (Arc::clone(&entry.cancelled), Arc::clone(&entry.state)))
                    .collect()
            } else {
                Vec::new()
            }
        };
        for (cancelled, state_arc) in task_handles {
            cancelled.store(true, Ordering::Release);
            let (_lock, cvar) = &*state_arc;
            cvar.notify_all();
        }

        // Cancel all schedules — extract flags first, drop lock, then operate.
        let schedule_flags: Vec<Arc<AtomicBool>> = {
            if let Ok(schedules) = self.schedules.lock() {
                schedules
                    .iter()
                    .map(|(_id, entry)| Arc::clone(&entry.cancelled))
                    .collect()
            } else {
                Vec::new()
            }
        };
        for flag in schedule_flags {
            flag.store(true, Ordering::Release);
        }
    }
}

// ============================================================
// Static runtime instance
// ============================================================

static RUNTIME: LazyLock<ConcurrencyRuntime> = LazyLock::new(ConcurrencyRuntime::new);

// ============================================================
// Helper functions (pure, no state)
// ============================================================

/// Create a Task value handle (represented as a Map with _task_id).
fn create_task_value(id: u64) -> Value {
    let mut task = HashMap::new();
    task.insert("_task_id".to_string(), Value::Int(id as i64));
    task.insert("type".to_string(), Value::String("Task".to_string()));
    Value::Map(task)
}

/// Get task ID from a Task value.
fn get_task_id(task: &Value) -> Result<u64> {
    match task {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_task_id") {
                Ok(*id as u64)
            } else {
                Err(IntentError::type_error("Expected a Task".to_string()))
            }
        }
        _ => Err(IntentError::type_error("Expected a Task".to_string())),
    }
}

/// Create a Channel value handle.
fn create_channel_value(id: u64) -> Value {
    let mut ch = HashMap::new();
    ch.insert("_channel_id".to_string(), Value::Int(id as i64));
    ch.insert("type".to_string(), Value::String("Channel".to_string()));
    Value::Map(ch)
}

/// Get channel ID from a channel value.
fn get_channel_id(ch: &Value) -> Result<u64> {
    match ch {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_channel_id") {
                Ok(*id as u64)
            } else {
                Err(IntentError::type_error("Expected a Channel".to_string()))
            }
        }
        _ => Err(IntentError::type_error("Expected a Channel".to_string())),
    }
}

/// Get schedule ID from a schedule handle value.
fn get_schedule_id(schedule: &Value) -> Result<u64> {
    match schedule {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_schedule_id") {
                Ok(*id as u64)
            } else {
                Err(IntentError::type_error(
                    "Expected a Schedule handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::type_error(
            "Expected a Schedule handle".to_string(),
        )),
    }
}

/// Extract function params, body, and serialized closure bindings for task spawning.
/// Used by `spawn_task`. Shared logic for extracting and serializing a function value.
fn extract_function_for_task(
    func: &Value,
    caller: &str,
) -> Result<(
    Vec<crate::ast::Parameter>,
    Block,
    HashMap<String, SerializedValue>,
)> {
    match func {
        Value::Function {
            params,
            body,
            closure,
            ..
        } => {
            let bindings = closure.borrow().all_bindings();
            let mut serialized_bindings = HashMap::new();
            let mut user_skipped = Vec::new();
            for (name, value) in &bindings {
                match SerializedValue::from_value(value) {
                    Ok(sv) => {
                        serialized_bindings.insert(name.clone(), sv);
                    }
                    Err(_) => {
                        // Only warn for user-defined non-serializable values,
                        // not stdlib/native functions (available via builtins).
                        if !matches!(value, Value::NativeFunction { .. }) {
                            user_skipped.push(name.clone());
                        }
                    }
                }
            }
            if !user_skipped.is_empty() {
                eprintln!(
                    "[{}] Warning: cannot capture non-serializable values across tasks: {}. \
                     These variables will not be available in the spawned task. \
                     Only primitive types (Int, Float, String, Bool, Array, Map) can cross task boundaries.",
                    caller,
                    user_skipped.join(", ")
                );
            }
            Ok((params.clone(), body.clone(), serialized_bindings))
        }
        _ => Err(IntentError::type_error(format!(
            "{}() requires a function argument",
            caller
        ))),
    }
}

/// Common task thread body: set cancellation context, transition Running → terminal.
/// Wraps eval_block in catch_unwind.
fn run_task_thread(
    task_state: Arc<(Mutex<TaskState>, Condvar)>,
    task_cancelled: Arc<AtomicBool>,
    body: Block,
    closure_bindings: HashMap<String, SerializedValue>,
) {
    set_current_task_cancelled(Arc::clone(&task_cancelled));

    // Transition to Running
    {
        let (lock, cvar) = &*task_state;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        *state = TaskState::Running;
        cvar.notify_all();
    }

    // Check cancellation before starting
    if task_cancelled.load(Ordering::Acquire) {
        let (lock, cvar) = &*task_state;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        *state = TaskState::Cancelled;
        cvar.notify_all();
        return;
    }

    // Create interpreter with all stdlib
    let mut interpreter = crate::interpreter::Interpreter::new();
    interpreter.define_all_stdlib_as_globals();
    for (name, sv) in &closure_bindings {
        interpreter.define_variable(name.clone(), sv.to_value());
    }

    // Execute with catch_unwind — panics don't poison shared state
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        interpreter.eval_block(&body)
    }));

    // Transition to terminal state
    let (lock, cvar) = &*task_state;
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());

    if task_cancelled.load(Ordering::Acquire) {
        *state = TaskState::Cancelled;
    } else {
        match result {
            Ok(Ok(value)) => {
                let final_value = match value {
                    Value::Return(inner) => *inner,
                    other => other,
                };
                match SerializedValue::from_value(&final_value) {
                    Ok(sv) => *state = TaskState::Completed(sv),
                    Err(e) => *state = TaskState::Failed(format!("Cannot serialize result: {}", e)),
                }
            }
            Ok(Err(e)) => {
                *state = TaskState::Failed(format!("{}", e));
            }
            Err(panic_info) => {
                let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                    format!("Task panicked: {}", s)
                } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                    format!("Task panicked: {}", s)
                } else {
                    "Task panicked (unknown cause)".to_string()
                };
                *state = TaskState::Failed(msg);
            }
        }
    }

    cvar.notify_all();
}

// ============================================================
// Public free functions — thin wrappers around RUNTIME
// ============================================================

/// Create a new channel.
pub fn concurrent_channel() -> Result<Value> {
    RUNTIME.create_channel()
}

/// Send a value through a channel.
pub fn concurrent_send(ch: &Value, value: &Value) -> Result<Value> {
    RUNTIME.send(ch, value)
}

/// Receive a value, blocking.
pub fn concurrent_recv(ch: &Value) -> Result<Value> {
    RUNTIME.recv(ch)
}

/// Receive with timeout.
pub fn concurrent_recv_timeout(ch: &Value, timeout_ms: i64) -> Result<Value> {
    RUNTIME.recv_timeout(ch, timeout_ms)
}

/// Non-blocking receive.
pub fn concurrent_try_recv(ch: &Value) -> Result<Value> {
    RUNTIME.try_recv(ch)
}

/// Close a channel.
pub fn concurrent_close(ch: &Value) -> Result<Value> {
    RUNTIME.close_channel(ch)
}

/// Sleep with cooperative cancellation.
fn concurrent_sleep_ms(ms: i64) -> Result<Value> {
    if ms > 0 {
        let total = ms as u64;
        let mut slept = 0u64;
        let increment = 50u64.min(total);
        while slept < total {
            check_cancellation()?;
            let sleep_for = increment.min(total - slept);
            thread::sleep(Duration::from_millis(sleep_for));
            slept += sleep_for;
        }
    }
    Ok(Value::Unit)
}

/// Get available thread count.
fn concurrent_thread_count() -> Result<Value> {
    Ok(Value::Int(
        thread::available_parallelism()
            .map(|n| n.get() as i64)
            .unwrap_or(1),
    ))
}

/// Spawn a background task.
pub fn concurrent_spawn(func: &Value) -> Result<Value> {
    RUNTIME.spawn_task(func)
}

/// Block until task completes (consume semantics).
pub fn concurrent_await_task(task: &Value) -> Result<Value> {
    RUNTIME.await_task(task)
}

/// Non-blocking peek at task status.
pub fn concurrent_try_await(task: &Value) -> Result<Value> {
    RUNTIME.try_await_task(task)
}

/// Cancel a task cooperatively.
pub fn concurrent_cancel_task(task: &Value) -> Result<Value> {
    RUNTIME.cancel_task(task)
}

/// Schedule a periodic function.
pub fn concurrent_schedule(
    interval_str: &str,
    body: Block,
    closure_bindings: HashMap<String, SerializedValue>,
) -> Result<Value> {
    RUNTIME.schedule(interval_str, body, closure_bindings)
}

/// Execute a function once after a delay.
pub fn concurrent_after(
    delay_ms: i64,
    body: Block,
    closure_bindings: HashMap<String, SerializedValue>,
) -> Result<Value> {
    RUNTIME.after(delay_ms, body, closure_bindings)
}

/// Cancel a schedule.
pub fn concurrent_cancel_schedule(schedule: &Value) -> Result<Value> {
    RUNTIME.cancel_schedule(schedule)
}

/// Cancel all tasks (called from interpreter shutdown).
pub fn cancel_all_tasks() {
    RUNTIME.shutdown();
}

/// Cancel all schedules (called from interpreter shutdown).
/// Now a no-op since `shutdown()` cancels everything. Kept for API compatibility.
pub fn cancel_all_schedules() {
    // shutdown() already cancelled all schedules. This is kept so callers
    // don't need to change. The first call to cancel_all_tasks() via
    // RUNTIME.shutdown() handles everything.
}

// ============================================================
// Pure functions — interval parsing
// ============================================================

/// Parse an interval string like "every 5s", "every 2m", "every 1h"
pub fn parse_interval(interval: &str) -> Result<Duration> {
    let s = interval.trim().to_lowercase();
    let s = s.strip_prefix("every ").unwrap_or(&s);
    let s = s.trim();

    if let Some(num_str) = s.strip_suffix("ms") {
        let n: u64 = num_str
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", interval)))?;
        Ok(Duration::from_millis(n))
    } else if let Some(num_str) = s.strip_suffix('s') {
        let n: u64 = num_str
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", interval)))?;
        Ok(Duration::from_secs(n))
    } else if let Some(num_str) = s.strip_suffix('m') {
        let n: u64 = num_str
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", interval)))?;
        Ok(Duration::from_secs(n * 60))
    } else if let Some(num_str) = s.strip_suffix('h') {
        let n: u64 = num_str
            .trim()
            .parse()
            .map_err(|_| IntentError::runtime_error(format!("Invalid interval: {}", interval)))?;
        Ok(Duration::from_secs(n * 3600))
    } else {
        Err(IntentError::runtime_error(format!(
            "Invalid interval format: '{}'. Use 'every Ns', 'every Nm', 'every Nh', or 'every Nms'",
            interval
        )))
    }
}

// ============================================================
// Module Initialization
// ============================================================

/// Initialize the std/concurrent module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt channel
    // @module std/concurrent
    // @module_description Concurrent execution with channels, tasks, and scheduling
    // @signature channel() -> Channel
    // Creates a new unbounded channel for inter-task communication.
    // @returns Channel handle (Map with _channel_id)
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
    // Sends a value through a channel. Returns false if channel is closed.
    // @param ch The channel to send on
    // @param value The value to send (only primitive types: Int, Float, String, Bool, Array, Map)
    // @see_also channel, recv
    // @since v0.2.0
    // @example send(ch, "hello") ~ "Send a string through the channel"
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
    // Receives a value from a channel. Blocks until a value is available. Returns Unit if channel is closed and empty.
    // @param ch The channel to receive from
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
    // @param ch The channel to receive from
    // @param millis Timeout in milliseconds
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
    // Non-blocking receive. Returns None if no value is available.
    // @param ch The channel to receive from
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
    // Closes a channel. Senders will fail, receivers get remaining messages then Unit.
    // @param ch The channel to close
    // @see_also channel
    // @since v0.2.0
    // @example close(ch) ~ "Close the channel when done"
    module.insert(
        "close".to_string(),
        Value::NativeFunction {
            name: "close".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_close(&args[0]),
        },
    );

    // @ntnt sleep_ms
    // @module std/concurrent
    // @signature sleep_ms(ms: Int) -> Unit
    // Pauses execution for specified milliseconds.
    // @param ms Duration to sleep in milliseconds
    // @since v0.2.0
    // @example sleep_ms(1000) ~ "Sleep for 1 second"
    module.insert(
        "sleep_ms".to_string(),
        Value::NativeFunction {
            name: "sleep_ms".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                eprintln!(
                    "[DEPRECATED] sleep_ms() is deprecated. Use sleep() from std/time instead."
                );
                match &args[0] {
                    Value::Int(ms) => concurrent_sleep_ms(*ms),
                    _ => Err(IntentError::type_error(
                        "sleep_ms requires an integer".to_string(),
                    )),
                }
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

    // @ntnt spawn
    // @module std/concurrent
    // @signature spawn(fn: Function) -> Task
    // Spawns a function as a background task on a new thread. The function's captured
    // variables are serialized (snapshot at spawn time) — mutations after spawn are not visible.
    // Returns a Task handle for await_task/try_await/cancel_task.
    // @param fn A zero-argument function to execute in the background
    // @returns Task handle (Map with _task_id)
    // @see_also await_task, try_await, cancel_task
    // @since v0.5.0
    // @example spawn(fn() { return 42 }) ~ "Spawn a background task that returns 42"
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
    // Blocks until the task completes. Returns Ok(value) on success, Err(message) on failure.
    // Integrates with `otherwise` for error handling.
    // @param task The Task handle from spawn()
    // @returns Result with the task's return value or error message
    // @see_also spawn, try_await, cancel_task
    // @since v0.5.0
    // @example await_task(task) ~ "Wait for task to complete and get result"
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
    // @signature try_await(task: Task) -> Option<Result<Any, String>>
    // Non-blocking check on a task. Returns None if still running, Some(Result) if done.
    // @param task The Task handle from spawn()
    // @returns None if pending/running, Some(Ok(value)) or Some(Err(msg)) if completed
    // @see_also spawn, await_task, cancel_task
    // @since v0.5.0
    // @example try_await(task) ~ "Check if task is done without blocking"
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
    // Requests cancellation of a task. Cancellation is cooperative — checked at yield points
    // (recv, sleep, fetch). Returns true if the cancellation was set, false if task not found.
    // @param task The Task handle from spawn()
    // @returns true if cancellation flag was set
    // @see_also spawn, await_task
    // @since v0.5.0
    // @example cancel_task(task) ~ "Request task cancellation"
    module.insert(
        "cancel_task".to_string(),
        Value::NativeFunction {
            name: "cancel_task".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_cancel_task(&args[0]),
        },
    );

    // @ntnt cancel_schedule
    // @module std/concurrent
    // @signature cancel_schedule(schedule: Schedule) -> Bool
    // Cancels a scheduled task. Returns true if the schedule was found and cancelled,
    // false if the schedule handle was not found.
    // @param schedule The Schedule handle from schedule()
    // @returns true if cancellation flag was set
    // @see_also schedule
    // @since v0.5.0
    // @example cancel_schedule(handle) ~ "Cancel a scheduled task"
    module.insert(
        "cancel_schedule".to_string(),
        Value::NativeFunction {
            name: "cancel_schedule".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| concurrent_cancel_schedule(&args[0]),
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
        assert!(module.contains_key("sleep_ms"));
        assert!(module.contains_key("thread_count"));
        assert!(module.contains_key("spawn"));
        assert!(module.contains_key("await_task"));
        assert!(module.contains_key("try_await"));
        assert!(module.contains_key("cancel_task"));
        assert!(module.contains_key("cancel_schedule"));
    }

    #[test]
    fn test_channel_creation() {
        let ch = concurrent_channel().unwrap();
        assert!(matches!(ch, Value::Map(_)));
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
    fn test_thread_count() {
        let count = concurrent_thread_count().unwrap();
        match count {
            Value::Int(n) => assert!(n >= 1),
            _ => panic!("Expected Int"),
        }
    }

    #[test]
    fn test_task_value_creation() {
        let task = create_task_value(42);
        assert_eq!(get_task_id(&task).unwrap(), 42);
    }

    #[test]
    fn test_parse_interval() {
        assert_eq!(parse_interval("every 5s").unwrap(), Duration::from_secs(5));
        assert_eq!(
            parse_interval("every 2m").unwrap(),
            Duration::from_secs(120)
        );
        assert_eq!(
            parse_interval("every 1h").unwrap(),
            Duration::from_secs(3600)
        );
        assert_eq!(
            parse_interval("every 100ms").unwrap(),
            Duration::from_millis(100)
        );
        assert_eq!(parse_interval("5s").unwrap(), Duration::from_secs(5));
        assert!(parse_interval("invalid").is_err());
    }

    #[test]
    fn test_cancel_task_invalid() {
        let fake_task = create_task_value(999999);
        let result = concurrent_cancel_task(&fake_task).unwrap();
        assert!(matches!(result, Value::Bool(false)));
    }
}
