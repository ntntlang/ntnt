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

    /// Convert to serde_json::Value for storage in JSONB columns
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            SerializedValue::Unit => serde_json::Value::Null,
            SerializedValue::Int(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
            SerializedValue::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            SerializedValue::Bool(b) => serde_json::Value::Bool(*b),
            SerializedValue::String(s) => serde_json::Value::String(s.clone()),
            SerializedValue::Array(arr) => {
                serde_json::Value::Array(arr.iter().map(|v| v.to_json()).collect())
            }
            SerializedValue::Map(map) => {
                let obj: serde_json::Map<std::string::String, serde_json::Value> =
                    map.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
                serde_json::Value::Object(obj)
            }
        }
    }

    /// Convert from serde_json::Value back to SerializedValue
    pub fn from_json(json: &serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => SerializedValue::Unit,
            serde_json::Value::Bool(b) => SerializedValue::Bool(*b),
            serde_json::Value::Number(n) => {
                if let Some(i) = n.as_i64() {
                    SerializedValue::Int(i)
                } else if let Some(f) = n.as_f64() {
                    SerializedValue::Float(f)
                } else {
                    SerializedValue::Unit
                }
            }
            serde_json::Value::String(s) => SerializedValue::String(s.clone()),
            serde_json::Value::Array(arr) => {
                SerializedValue::Array(arr.iter().map(Self::from_json).collect())
            }
            serde_json::Value::Object(obj) => {
                let mut map = HashMap::new();
                for (k, v) in obj {
                    map.insert(k.clone(), Self::from_json(v));
                }
                SerializedValue::Map(map)
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

    let registry = CHANNEL_REGISTRY
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock registry: {}", e)))?;

    if let Some(pair) = registry.get(&id) {
        if *pair.closed.lock().unwrap() {
            return Ok(Value::Bool(false));
        }
        match pair.sender.send(serialized) {
            Ok(_) => Ok(Value::Bool(true)),
            Err(_) => Ok(Value::Bool(false)),
        }
    } else {
        Err(IntentError::runtime_error("Invalid channel".to_string()))
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

    let registry = CHANNEL_REGISTRY
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock registry: {}", e)))?;

    if let Some(pair) = registry.get(&id) {
        let mut closed = pair.closed.lock().unwrap();
        *closed = true;
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
            for (name, value) in &bindings {
                match SerializedValue::from_value(value) {
                    Ok(sv) => {
                        serialized_bindings.insert(name.clone(), sv);
                    }
                    Err(_) => {
                        // Skip non-serializable values (functions, native functions, etc.)
                        // They'll be available through the interpreter's builtins
                    }
                }
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
    loop {
        match &*state {
            TaskState::Completed(sv) => {
                return Ok(Value::ok(sv.to_value()));
            }
            TaskState::Failed(msg) => {
                return Ok(Value::err(Value::String(msg.clone())));
            }
            TaskState::Cancelled => {
                return Ok(Value::err(Value::String("Task cancelled".to_string())));
            }
            TaskState::Pending | TaskState::Running => {
                // Wait for state change
                state = cvar.wait(state).unwrap();
            }
        }
    }
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

    match &*state {
        TaskState::Completed(sv) => Ok(Value::some(Value::ok(sv.to_value()))),
        TaskState::Failed(msg) => Ok(Value::some(Value::err(Value::String(msg.clone())))),
        TaskState::Cancelled => Ok(Value::some(Value::err(Value::String(
            "Task cancelled".to_string(),
        )))),
        TaskState::Pending | TaskState::Running => Ok(Value::none()),
    }
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

// ============================================================
// Cron Expression Parser
// ============================================================

/// A parsed cron field that can match values
#[derive(Debug, Clone)]
enum CronField {
    /// Match any value
    Any,
    /// Match specific values
    Values(Vec<u32>),
}

impl CronField {
    fn matches(&self, value: u32) -> bool {
        match self {
            CronField::Any => true,
            CronField::Values(vals) => vals.contains(&value),
        }
    }
}

/// A parsed 5-field cron expression: minute hour day_of_month month day_of_week
#[derive(Debug, Clone)]
pub struct CronExpression {
    minute: CronField,
    hour: CronField,
    day_of_month: CronField,
    month: CronField,
    day_of_week: CronField,
}

/// Parse a single cron field (e.g., "*/15", "1-5", "MON-FRI", "1,3,5")
fn parse_cron_field(
    field: &str,
    min: u32,
    max: u32,
    names: &[(&str, u32)],
) -> std::result::Result<CronField, String> {
    let field = field.trim();

    if field == "*" {
        return Ok(CronField::Any);
    }

    // Handle step: */N or range/N
    if let Some(step_parts) = field.split_once('/') {
        let step: u32 = step_parts
            .1
            .parse()
            .map_err(|_| format!("Invalid step value: {}", step_parts.1))?;
        if step == 0 {
            return Err("Step value cannot be 0".to_string());
        }

        let (range_start, range_end) = if step_parts.0 == "*" {
            (min, max)
        } else if let Some((s, e)) = step_parts.0.split_once('-') {
            let s = resolve_cron_value(s, names)?;
            let e = resolve_cron_value(e, names)?;
            (s, e)
        } else {
            let s = resolve_cron_value(step_parts.0, names)?;
            (s, max)
        };

        let mut values = Vec::new();
        let mut v = range_start;
        while v <= range_end {
            values.push(v);
            v += step;
        }
        return Ok(CronField::Values(values));
    }

    // Handle comma-separated list
    let mut values = Vec::new();
    for part in field.split(',') {
        let part = part.trim();
        if let Some((s, e)) = part.split_once('-') {
            let start = resolve_cron_value(s.trim(), names)?;
            let end = resolve_cron_value(e.trim(), names)?;
            for v in start..=end {
                if !values.contains(&v) {
                    values.push(v);
                }
            }
        } else {
            let v = resolve_cron_value(part, names)?;
            if !values.contains(&v) {
                values.push(v);
            }
        }
    }

    values.sort();
    Ok(CronField::Values(values))
}

/// Resolve a cron value — either a number or a named value (MON, JAN, etc.)
fn resolve_cron_value(s: &str, names: &[(&str, u32)]) -> std::result::Result<u32, String> {
    let s_upper = s.to_uppercase();
    for (name, val) in names {
        if s_upper == *name {
            return Ok(*val);
        }
    }
    s.parse::<u32>()
        .map_err(|_| format!("Invalid cron value: {}", s))
}

/// Day-of-week names
const DOW_NAMES: &[(&str, u32)] = &[
    ("SUN", 0),
    ("MON", 1),
    ("TUE", 2),
    ("WED", 3),
    ("THU", 4),
    ("FRI", 5),
    ("SAT", 6),
];

/// Month names
const MONTH_NAMES: &[(&str, u32)] = &[
    ("JAN", 1),
    ("FEB", 2),
    ("MAR", 3),
    ("APR", 4),
    ("MAY", 5),
    ("JUN", 6),
    ("JUL", 7),
    ("AUG", 8),
    ("SEP", 9),
    ("OCT", 10),
    ("NOV", 11),
    ("DEC", 12),
];

/// Parse a 5-field cron expression string
pub fn parse_cron(expr: &str) -> std::result::Result<CronExpression, String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "Cron expression must have 5 fields (minute hour day month dow), got {}",
            fields.len()
        ));
    }

    Ok(CronExpression {
        minute: parse_cron_field(fields[0], 0, 59, &[])?,
        hour: parse_cron_field(fields[1], 0, 23, &[])?,
        day_of_month: parse_cron_field(fields[2], 1, 31, &[])?,
        month: parse_cron_field(fields[3], 1, 12, MONTH_NAMES)?,
        day_of_week: parse_cron_field(fields[4], 0, 6, DOW_NAMES)?,
    })
}

/// Calculate the next run time from `after` that matches the cron expression.
/// Returns the next matching UTC timestamp as SystemTime.
pub fn cron_next_run(cron: &CronExpression, after: std::time::SystemTime) -> std::time::SystemTime {
    use std::time::UNIX_EPOCH;

    let since_epoch = after.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = since_epoch.as_secs();

    // Convert to broken-down time components (UTC)
    let mut minute = ((total_secs / 60) % 60) as u32;
    let mut hour = ((total_secs / 3600) % 24) as u32;

    // Days since epoch
    let mut days = (total_secs / 86400) as i64;

    // Start from the next minute
    minute += 1;
    if minute >= 60 {
        minute = 0;
        hour += 1;
        if hour >= 24 {
            hour = 0;
            days += 1;
        }
    }

    // Search for the next matching time (max ~4 years ahead to avoid infinite loop)
    let max_days = days + 366 * 4;
    while days <= max_days {
        // Convert days since epoch to year/month/day
        let (_year, month, day, dow) = days_to_ymd(days);

        if cron.month.matches(month)
            && cron.day_of_month.matches(day)
            && cron.day_of_week.matches(dow)
        {
            while hour < 24 {
                if cron.hour.matches(hour) {
                    while minute < 60 {
                        if cron.minute.matches(minute) {
                            // Found a match
                            let day_secs = days as u64 * 86400;
                            let time_secs = hour as u64 * 3600 + minute as u64 * 60;
                            return UNIX_EPOCH
                                + std::time::Duration::from_secs(day_secs + time_secs);
                        }
                        minute += 1;
                    }
                }
                minute = 0;
                hour += 1;
            }
        }
        hour = 0;
        minute = 0;
        days += 1;
    }

    // Fallback (should not happen for valid cron expressions)
    after + std::time::Duration::from_secs(3600)
}

/// Convert days since Unix epoch to (year, month, day, day_of_week)
fn days_to_ymd(days: i64) -> (i32, u32, u32, u32) {
    // Unix epoch was Thursday (dow=4)
    let dow = ((days % 7 + 4) % 7) as u32; // 0=Sun, 1=Mon, ...6=Sat

    // Civil calendar calculation (from Howard Hinnant's algorithm)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i32 + (era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };

    (year, m, d, dow)
}

/// Check if a schedule string looks like a cron expression (5 space-separated fields)
fn is_cron_expression(s: &str) -> bool {
    let trimmed = s.trim();
    // Must not start with "every " (interval syntax)
    if trimmed.to_lowercase().starts_with("every ") {
        return false;
    }
    // Check if it has 5 whitespace-separated fields
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    fields.len() == 5
}

/// Schedule a function to run at a fixed interval or cron expression.
/// Returns a schedule ID. Overlap prevention: skips tick if previous execution still running.
pub fn concurrent_schedule(
    interval_str: &str,
    params: Vec<Parameter>,
    body: Block,
    closure_bindings: HashMap<String, SerializedValue>,
) -> Result<Value> {
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

    let schedule_cancelled = Arc::clone(&cancelled);
    let _params = params;
    let interval_owned = interval_str.to_string();

    if is_cron_expression(interval_str) {
        // Cron mode
        let cron = parse_cron(interval_str)
            .map_err(|e| IntentError::runtime_error(format!("Invalid cron expression: {}", e)))?;
        let cron_expr_str = interval_owned.clone();

        thread::spawn(move || {
            let executing = Arc::new(AtomicBool::new(false));

            loop {
                if schedule_cancelled.load(Ordering::Relaxed) {
                    break;
                }

                // Calculate next run time
                let now = std::time::SystemTime::now();
                let next_run = cron_next_run(&cron, now);

                let wait = next_run
                    .duration_since(now)
                    .unwrap_or(Duration::from_secs(60));

                // Sleep in small increments so we can check cancellation
                let sleep_end = std::time::Instant::now() + wait;
                while std::time::Instant::now() < sleep_end {
                    if schedule_cancelled.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(
                        Duration::from_millis(500)
                            .min(sleep_end.saturating_duration_since(std::time::Instant::now())),
                    );
                }

                if schedule_cancelled.load(Ordering::Relaxed) {
                    break;
                }

                // Overlap prevention
                if executing.load(Ordering::Relaxed) {
                    eprintln!("[schedule/cron] Skipping tick — previous execution still running");
                    continue;
                }

                // Cluster safety: for PostgreSQL backend, use advisory lock
                let should_run = {
                    use crate::stdlib::jobs::{get_backend, BackendKind};
                    match get_backend() {
                        Ok(BackendKind::Postgres(_)) => {
                            // Try to acquire advisory lock
                            cron_pg_try_advisory_lock(&cron_expr_str)
                        }
                        _ => true,
                    }
                };

                if !should_run {
                    continue;
                }

                executing.store(true, Ordering::Relaxed);
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

                    if let Err(e) = interpreter.eval_block(&exec_body) {
                        eprintln!("[schedule/cron] Error: {}", e);
                    }

                    exec_executing.store(false, Ordering::Relaxed);
                });
            }
        });
    } else {
        // Interval mode (existing behavior)
        let duration = parse_interval(interval_str)?;

        thread::spawn(move || {
            let executing = Arc::new(AtomicBool::new(false));

            loop {
                thread::sleep(duration);

                if schedule_cancelled.load(Ordering::Relaxed) {
                    break;
                }

                if executing.load(Ordering::Relaxed) {
                    eprintln!("[schedule] Skipping tick — previous execution still running");
                    continue;
                }

                executing.store(true, Ordering::Relaxed);
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

                    if let Err(e) = interpreter.eval_block(&exec_body) {
                        eprintln!("[schedule] Error: {}", e);
                    }

                    exec_executing.store(false, Ordering::Relaxed);
                });
            }
        });
    }

    let mut sched = HashMap::new();
    sched.insert("_schedule_id".to_string(), Value::Int(schedule_id as i64));
    sched.insert("type".to_string(), Value::String("Schedule".to_string()));
    Ok(Value::Map(sched))
}

/// Try to acquire a PostgreSQL advisory lock for cron cluster safety
fn cron_pg_try_advisory_lock(cron_expr: &str) -> bool {
    use std::hash::{Hash, Hasher};
    let key = format!("cron:{}", cron_expr);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    key.hash(&mut hasher);
    let hash = hasher.finish() as i64;

    // Try to get a connection from the job pool
    let pool = match crate::stdlib::jobs::get_job_pool() {
        Ok(p) => p,
        Err(_) => return true, // If no pool, run anyway
    };

    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;
    db_rt.block_on(async {
        let client = match pool.get().await {
            Ok(c) => c,
            Err(_) => return true,
        };
        let row = client
            .query_one("SELECT pg_try_advisory_lock($1)", &[&hash])
            .await;
        match row {
            Ok(r) => r.get::<_, bool>(0),
            Err(_) => true,
        }
    })
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
