//! std/concurrent module - Concurrency primitives
//!
//! Provides channels for communication between the main program and background tasks,
//! plus structured task spawning with spawn/await/cancel semantics.
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
//! let result = await_task(task)  // result == 42
//!
//! // Spawn with channel communication
//! let ch = channel()
//! spawn(fn() { send(ch, "hello from task") })
//! let msg = recv(ch)  // "hello from task"
//! ```

use crate::ast::{Block, Parameter};
use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

type Result<T> = std::result::Result<T, IntentError>;

// ============================================================
// Serialized Values (thread-safe value transport)
// ============================================================

/// Global registry for channels
static CHANNEL_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, ChannelPair>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CHANNEL_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

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
// Channel Implementation
// ============================================================

/// A channel pair (sender + receiver) using serialized values
struct ChannelPair {
    sender: mpsc::Sender<SerializedValue>,
    receiver: Arc<Mutex<mpsc::Receiver<SerializedValue>>>,
    closed: Arc<Mutex<bool>>,
}

/// Create a channel value handle
fn create_channel_value(id: u64) -> Value {
    let mut ch = HashMap::new();
    ch.insert("_channel_id".to_string(), Value::Int(id as i64));
    ch.insert("type".to_string(), Value::String("Channel".to_string()));
    Value::Map(ch)
}

/// Get channel ID from a channel value
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

fn concurrent_channel() -> Result<Value> {
    let (tx, rx) = mpsc::channel();
    let id = CHANNEL_ID_COUNTER.fetch_add(1, Ordering::SeqCst);

    let pair = ChannelPair {
        sender: tx,
        receiver: Arc::new(Mutex::new(rx)),
        closed: Arc::new(Mutex::new(false)),
    };

    if let Ok(mut registry) = CHANNEL_REGISTRY.lock() {
        registry.insert(id, pair);
    }

    Ok(create_channel_value(id))
}

fn concurrent_send(ch: &Value, value: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;
    let serialized = SerializedValue::from_value(value)?;

    // Extract sender and closed flag, then drop registry lock before sending.
    // This prevents blocking all channel operations if send ever blocks.
    let (sender, closed) = {
        let registry = CHANNEL_REGISTRY
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock registry: {}", e)))?;
        if let Some(pair) = registry.get(&id) {
            (pair.sender.clone(), Arc::clone(&pair.closed))
        } else {
            return Err(IntentError::runtime_error("Invalid channel".to_string()));
        }
    }; // registry lock dropped here

    if *closed.lock().unwrap() {
        return Ok(Value::Bool(false));
    }
    match sender.send(serialized) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(_) => Ok(Value::Bool(false)),
    }
}

fn concurrent_recv(ch: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;

    // Cooperative cancellation check
    check_cancellation()?;

    let receiver = {
        let registry = CHANNEL_REGISTRY
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock registry: {}", e)))?;
        if let Some(pair) = registry.get(&id) {
            Arc::clone(&pair.receiver)
        } else {
            return Err(IntentError::runtime_error("Invalid channel".to_string()));
        }
    };

    let rx = receiver
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock receiver: {}", e)))?;

    // Use recv_timeout in a loop to allow cancellation checks for spawned tasks
    loop {
        check_cancellation()?;
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(serialized) => return Ok(serialized.to_value()),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(Value::Unit),
        }
    }
}

fn concurrent_recv_timeout(ch: &Value, timeout_ms: i64) -> Result<Value> {
    let id = get_channel_id(ch)?;

    let receiver = {
        let registry = CHANNEL_REGISTRY
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock registry: {}", e)))?;
        if let Some(pair) = registry.get(&id) {
            Arc::clone(&pair.receiver)
        } else {
            return Err(IntentError::runtime_error("Invalid channel".to_string()));
        }
    };

    let rx = receiver
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock receiver: {}", e)))?;

    match rx.recv_timeout(Duration::from_millis(timeout_ms as u64)) {
        Ok(serialized) => Ok(Value::some(serialized.to_value())),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(Value::none()),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Value::none()),
    }
}

fn concurrent_try_recv(ch: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;

    let receiver = {
        let registry = CHANNEL_REGISTRY
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock registry: {}", e)))?;
        if let Some(pair) = registry.get(&id) {
            Arc::clone(&pair.receiver)
        } else {
            return Err(IntentError::runtime_error("Invalid channel".to_string()));
        }
    };

    let rx = receiver
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock receiver: {}", e)))?;

    match rx.try_recv() {
        Ok(serialized) => Ok(Value::some(serialized.to_value())),
        Err(mpsc::TryRecvError::Empty) => Ok(Value::none()),
        Err(mpsc::TryRecvError::Disconnected) => Ok(Value::none()),
    }
}

fn concurrent_close(ch: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;

    let mut registry = CHANNEL_REGISTRY
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock registry: {}", e)))?;

    if let Some(pair) = registry.get(&id) {
        let mut closed = pair.closed.lock().unwrap();
        *closed = true;
        drop(closed);
        // Remove from registry to free memory. Any pending recv() calls will
        // get Disconnected on their next attempt since the sender is dropped.
        registry.remove(&id);
        Ok(Value::Bool(true))
    } else {
        Ok(Value::Bool(false))
    }
}

fn concurrent_sleep_ms(ms: i64) -> Result<Value> {
    if ms > 0 {
        // Sleep in small increments to allow cooperative cancellation
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

fn concurrent_thread_count() -> Result<Value> {
    Ok(Value::Int(
        thread::available_parallelism()
            .map(|n| n.get() as i64)
            .unwrap_or(1),
    ))
}

// ============================================================
// Task System - Structured Concurrency
// ============================================================

/// Task state representing the lifecycle of a spawned task
#[derive(Debug, Clone)]
pub enum TaskState {
    /// Task has been created but not yet started
    Pending,
    /// Task is actively running
    Running,
    /// Task completed successfully with a result
    Completed(SerializedValue),
    /// Task failed with an error message
    Failed(String),
    /// Task was cancelled
    Cancelled,
}

/// Internal task handle shared between the task registry and spawned threads
pub struct TaskHandle {
    /// Current state of the task
    pub state: Arc<(Mutex<TaskState>, Condvar)>,
    /// Cancellation flag — checked cooperatively by long-running operations
    pub cancelled: Arc<AtomicBool>,
}

/// Global task registry
static TASK_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, TaskHandle>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static TASK_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Check if the current task context is cancelled.
/// Called from cooperative cancellation points (recv, sleep_ms, fetch).
/// Returns Err if cancelled, Ok(()) if not.
pub fn check_cancellation() -> Result<()> {
    // Use thread-local to store the current task's cancellation flag
    CURRENT_TASK_CANCELLED.with(|flag| {
        if let Some(ref cancelled) = *flag.borrow() {
            if cancelled.load(Ordering::Relaxed) {
                return Err(IntentError::runtime_error("Task cancelled".to_string()));
            }
        }
        Ok(())
    })
}

thread_local! {
    /// Thread-local cancellation flag for the current spawned task
    static CURRENT_TASK_CANCELLED: std::cell::RefCell<Option<Arc<AtomicBool>>> =
        const { std::cell::RefCell::new(None) };
}

/// Set the cancellation flag for the current thread (used when a task starts running)
fn set_current_task_cancelled(flag: Arc<AtomicBool>) {
    CURRENT_TASK_CANCELLED.with(|f| {
        *f.borrow_mut() = Some(flag);
    });
}

/// Create a Task value handle (represented as a Map with _task_id)
fn create_task_value(id: u64) -> Value {
    let mut task = HashMap::new();
    task.insert("_task_id".to_string(), Value::Int(id as i64));
    task.insert("type".to_string(), Value::String("Task".to_string()));
    Value::Map(task)
}

/// Get task ID from a Task value
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

/// Spawn a function as a background task.
/// The function's captured variables are serialized (snapshot at spawn time).
/// The task runs on a new thread with its own Interpreter instance.
///
/// Arguments: the function Value (must be Value::Function)
/// Returns: Task handle value
pub fn concurrent_spawn(func: &Value) -> Result<Value> {
    // Extract function components
    let (params, body, closure_bindings) = match func {
        Value::Function {
            params,
            body,
            closure,
            ..
        } => {
            // Snapshot all variables from the closure environment chain
            let bindings = closure.borrow().all_bindings();
            // Serialize only the serializable bindings (skip functions, native fns, etc.)
            let mut serialized_bindings = HashMap::new();
            let mut skipped_names = Vec::new();
            for (name, value) in &bindings {
                match SerializedValue::from_value(value) {
                    Ok(sv) => {
                        serialized_bindings.insert(name.clone(), sv);
                    }
                    Err(_) => {
                        // Track non-serializable captures — they may be available
                        // through the spawned interpreter's stdlib, but user-defined
                        // functions will NOT be available in the spawned task.
                        skipped_names.push(name.clone());
                    }
                }
            }
            if !skipped_names.is_empty() {
                eprintln!(
                    "[spawn] Warning: cannot capture non-serializable values across tasks: {}. \
                     These variables will not be available in the spawned task. \
                     Only primitive types (Int, Float, String, Bool, Array, Map) can cross task boundaries.",
                    skipped_names.join(", ")
                );
            }
            (params.clone(), body.clone(), serialized_bindings)
        }
        _ => {
            return Err(IntentError::type_error(
                "spawn() requires a function argument".to_string(),
            ));
        }
    };

    // Validate: spawned functions should take no arguments
    let required_params = params.iter().filter(|p| p.default.is_none()).count();
    if required_params > 0 {
        return Err(IntentError::runtime_error(
            "spawn() function must take no arguments (captured variables are serialized automatically)".to_string(),
        ));
    }

    // Create task handle
    let task_id = TASK_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let state = Arc::new((Mutex::new(TaskState::Pending), Condvar::new()));
    let cancelled = Arc::new(AtomicBool::new(false));

    let handle = TaskHandle {
        state: Arc::clone(&state),
        cancelled: Arc::clone(&cancelled),
    };

    // Register in global task registry
    {
        let mut registry = TASK_REGISTRY.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
        })?;
        registry.insert(task_id, handle);
    }

    // Spawn the task on a new thread
    let task_state = Arc::clone(&state);
    let task_cancelled = Arc::clone(&cancelled);

    thread::spawn(move || {
        // Set up cancellation context for this thread
        set_current_task_cancelled(Arc::clone(&task_cancelled));

        // Update state to Running
        {
            let (lock, cvar) = &*task_state;
            let mut state = lock.lock().unwrap();
            *state = TaskState::Running;
            cvar.notify_all();
        }

        // Check cancellation before starting
        if task_cancelled.load(Ordering::Relaxed) {
            let (lock, cvar) = &*task_state;
            let mut state = lock.lock().unwrap();
            *state = TaskState::Cancelled;
            cvar.notify_all();
            return;
        }

        // Create a new Interpreter for this task with all stdlib available
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.define_all_stdlib_as_globals();

        // Inject captured variables into the interpreter's environment
        // (these override any stdlib names that conflict, which is correct
        // since the user's captures take precedence)
        for (name, sv) in &closure_bindings {
            interpreter.define_variable(name.clone(), sv.to_value());
        }

        // Execute the function body
        let result = interpreter.eval_block(&body);

        // Update task state with result
        let (lock, cvar) = &*task_state;
        let mut state = lock.lock().unwrap();

        if task_cancelled.load(Ordering::Relaxed) {
            *state = TaskState::Cancelled;
        } else {
            match result {
                Ok(value) => {
                    // Handle Return values (unwrap the inner value)
                    let final_value = match value {
                        Value::Return(inner) => *inner,
                        other => other,
                    };
                    match SerializedValue::from_value(&final_value) {
                        Ok(sv) => *state = TaskState::Completed(sv),
                        Err(e) => {
                            *state = TaskState::Failed(format!("Cannot serialize result: {}", e))
                        }
                    }
                }
                Err(e) => {
                    *state = TaskState::Failed(format!("{}", e));
                }
            }
        }

        cvar.notify_all();
    });

    Ok(create_task_value(task_id))
}

/// await_task(task) -> Result
/// Blocks until the task completes. Returns Ok(value) or Err(error_string).
pub fn concurrent_await_task(task: &Value) -> Result<Value> {
    let id = get_task_id(task)?;

    let state_arc = {
        let registry = TASK_REGISTRY.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
        })?;
        if let Some(handle) = registry.get(&id) {
            Arc::clone(&handle.state)
        } else {
            return Err(IntentError::runtime_error(format!(
                "Invalid task (id={})",
                id
            )));
        }
    };

    // Wait for task completion using condvar
    let (lock, cvar) = &*state_arc;
    let mut state = lock.lock().unwrap();
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
                // Wait for state change
                state = cvar.wait(state).unwrap();
            }
        }
    };

    // Clean up: remove completed task from registry to prevent memory leaks
    drop(state); // release the condvar lock first
    if let Ok(mut registry) = TASK_REGISTRY.lock() {
        registry.remove(&id);
    }

    result
}

/// try_await(task) -> Option<Result>
/// Non-blocking check. Returns None if still running, Some(Result) if done.
pub fn concurrent_try_await(task: &Value) -> Result<Value> {
    let id = get_task_id(task)?;

    let state_arc = {
        let registry = TASK_REGISTRY.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
        })?;
        if let Some(handle) = registry.get(&id) {
            Arc::clone(&handle.state)
        } else {
            return Err(IntentError::runtime_error(format!(
                "Invalid task (id={})",
                id
            )));
        }
    };

    let (lock, _cvar) = &*state_arc;
    let state = lock.lock().unwrap();

    let is_done = !matches!(&*state, TaskState::Pending | TaskState::Running);
    let result = match &*state {
        TaskState::Completed(sv) => Ok(Value::some(Value::ok(sv.to_value()))),
        TaskState::Failed(msg) => Ok(Value::some(Value::err(Value::String(msg.clone())))),
        TaskState::Cancelled => Ok(Value::some(Value::err(Value::String(
            "Task cancelled".to_string(),
        )))),
        TaskState::Pending | TaskState::Running => Ok(Value::none()),
    };

    // Clean up completed tasks from registry to prevent memory leaks
    if is_done {
        drop(state);
        if let Ok(mut registry) = TASK_REGISTRY.lock() {
            registry.remove(&id);
        }
    }

    result
}

/// cancel_task(task) -> Bool
/// Sets the cancellation flag on a task. Cancellation is cooperative.
pub fn concurrent_cancel_task(task: &Value) -> Result<Value> {
    let id = get_task_id(task)?;

    let registry = TASK_REGISTRY
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock task registry: {}", e)))?;

    if let Some(handle) = registry.get(&id) {
        handle.cancelled.store(true, Ordering::SeqCst);
        // Wake up anyone waiting on the condvar
        let (lock, cvar) = &*handle.state;
        let mut state = lock.lock().unwrap();
        // Only change state if still Pending or Running
        match *state {
            TaskState::Pending | TaskState::Running => {
                *state = TaskState::Cancelled;
                cvar.notify_all();
            }
            _ => {} // Already completed/failed/cancelled
        }
        Ok(Value::Bool(true))
    } else {
        Ok(Value::Bool(false))
    }
}

// ============================================================
// Schedule System - Periodic Task Execution
// ============================================================

/// Global schedule registry
static SCHEDULE_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, ScheduleHandle>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static SCHEDULE_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Handle for a scheduled task
struct ScheduleHandle {
    /// Cancellation flag
    cancelled: Arc<AtomicBool>,
}

/// Parse an interval string like "every 5s", "every 2m", "every 1h"
pub fn parse_interval(interval: &str) -> Result<Duration> {
    let s = interval.trim().to_lowercase();
    let s = s.strip_prefix("every ").unwrap_or(&s);
    let s = s.trim();

    // Try to parse as "Nms", "Ns", "Nm", "Nh" (check "ms" before "s")
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

/// Schedule a function to run at a fixed interval.
/// Returns a schedule ID. Overlap prevention: skips tick if previous execution still running.
pub fn concurrent_schedule(
    interval_str: &str,
    params: Vec<Parameter>,
    body: Block,
    closure_bindings: HashMap<String, SerializedValue>,
) -> Result<Value> {
    let duration = parse_interval(interval_str)?;

    let schedule_id = SCHEDULE_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));

    let handle = ScheduleHandle {
        cancelled: Arc::clone(&cancelled),
    };

    {
        let mut registry = SCHEDULE_REGISTRY.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock schedule registry: {}", e))
        })?;
        registry.insert(schedule_id, handle);
    }

    // Spawn a thread that periodically executes the function
    let schedule_cancelled = Arc::clone(&cancelled);
    let _params = params;

    thread::spawn(move || {
        // Flag for overlap prevention
        let executing = Arc::new(AtomicBool::new(false));

        loop {
            // Sleep for the interval
            thread::sleep(duration);

            // Check cancellation
            if schedule_cancelled.load(Ordering::Relaxed) {
                break;
            }

            // Overlap prevention: skip if previous execution still running
            if executing.load(Ordering::Relaxed) {
                eprintln!("[schedule] Skipping tick — previous execution still running");
                continue;
            }

            // Execute the function
            executing.store(true, Ordering::Relaxed);
            let exec_executing = Arc::clone(&executing);
            let exec_bindings = closure_bindings.clone();
            let exec_body = body.clone();
            let exec_cancelled = Arc::clone(&schedule_cancelled);

            thread::spawn(move || {
                // Set up cancellation context
                set_current_task_cancelled(exec_cancelled);

                let mut interpreter = crate::interpreter::Interpreter::new();
                interpreter.define_all_stdlib_as_globals();
                for (name, sv) in &exec_bindings {
                    interpreter.define_variable(name.clone(), sv.to_value());
                }

                if let Err(e) = interpreter.eval_block(&exec_body) {
                    eprintln!("[schedule] Error: {}", e);
                }

                exec_executing.store(false, Ordering::Relaxed);
            });
        }
    });

    // Return a schedule handle value
    let mut sched = HashMap::new();
    sched.insert("_schedule_id".to_string(), Value::Int(schedule_id as i64));
    sched.insert("type".to_string(), Value::String("Schedule".to_string()));
    Ok(Value::Map(sched))
}

/// Cancel all scheduled tasks (called on shutdown)
pub fn cancel_all_schedules() {
    if let Ok(registry) = SCHEDULE_REGISTRY.lock() {
        for (_id, handle) in registry.iter() {
            handle.cancelled.store(true, Ordering::SeqCst);
        }
    }
}

/// Cancel all spawned tasks (called on shutdown)
pub fn cancel_all_tasks() {
    if let Ok(registry) = TASK_REGISTRY.lock() {
        for (_id, handle) in registry.iter() {
            handle.cancelled.store(true, Ordering::SeqCst);
            let (lock, cvar) = &*handle.state;
            if let Ok(mut state) = lock.lock() {
                match *state {
                    TaskState::Pending | TaskState::Running => {
                        *state = TaskState::Cancelled;
                        cvar.notify_all();
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Execute a function once after a delay (ms).
/// Lifecycle-aware: cancelled on shutdown.
pub fn concurrent_after(
    delay_ms: i64,
    params: Vec<Parameter>,
    body: Block,
    closure_bindings: HashMap<String, SerializedValue>,
) -> Result<Value> {
    // Use the task system for after() — it's basically spawn + sleep
    let task_id = TASK_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let state = Arc::new((Mutex::new(TaskState::Pending), Condvar::new()));
    let cancelled = Arc::new(AtomicBool::new(false));

    let handle = TaskHandle {
        state: Arc::clone(&state),
        cancelled: Arc::clone(&cancelled),
    };

    {
        let mut registry = TASK_REGISTRY.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock task registry: {}", e))
        })?;
        registry.insert(task_id, handle);
    }

    let task_state = Arc::clone(&state);
    let task_cancelled = Arc::clone(&cancelled);
    let _params = params;

    thread::spawn(move || {
        set_current_task_cancelled(Arc::clone(&task_cancelled));

        // Update state to Running
        {
            let (lock, cvar) = &*task_state;
            let mut s = lock.lock().unwrap();
            *s = TaskState::Running;
            cvar.notify_all();
        }

        // Sleep for the delay
        if delay_ms > 0 {
            // Sleep in small increments to check cancellation
            let total = delay_ms as u64;
            let mut slept = 0u64;
            let increment = 50u64.min(total); // Check every 50ms
            while slept < total {
                if task_cancelled.load(Ordering::Relaxed) {
                    let (lock, cvar) = &*task_state;
                    let mut s = lock.lock().unwrap();
                    *s = TaskState::Cancelled;
                    cvar.notify_all();
                    return;
                }
                let sleep_for = increment.min(total - slept);
                thread::sleep(Duration::from_millis(sleep_for));
                slept += sleep_for;
            }
        }

        // Check cancellation again
        if task_cancelled.load(Ordering::Relaxed) {
            let (lock, cvar) = &*task_state;
            let mut s = lock.lock().unwrap();
            *s = TaskState::Cancelled;
            cvar.notify_all();
            return;
        }

        // Execute the function
        let mut interpreter = crate::interpreter::Interpreter::new();
        interpreter.define_all_stdlib_as_globals();
        for (name, sv) in &closure_bindings {
            interpreter.define_variable(name.clone(), sv.to_value());
        }

        let result = interpreter.eval_block(&body);

        let (lock, cvar) = &*task_state;
        let mut s = lock.lock().unwrap();
        if task_cancelled.load(Ordering::Relaxed) {
            *s = TaskState::Cancelled;
        } else {
            match result {
                Ok(value) => {
                    let final_value = match value {
                        Value::Return(inner) => *inner,
                        other => other,
                    };
                    match SerializedValue::from_value(&final_value) {
                        Ok(sv) => *s = TaskState::Completed(sv),
                        Err(e) => *s = TaskState::Failed(format!("Cannot serialize result: {}", e)),
                    }
                }
                Err(e) => {
                    *s = TaskState::Failed(format!("{}", e));
                }
            }
        }
        cvar.notify_all();
    });

    Ok(create_task_value(task_id))
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
