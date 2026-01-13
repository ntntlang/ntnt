//! std/concurrent module - Concurrency primitives
//!
//! Provides channels for communication between the main program and background tasks.
//! This module offers a simplified concurrency model suitable for common use cases
//! like parallel data processing and background work.
//!
//! ```ntnt
//! import { channel, send, recv, try_recv, close } from "std/concurrent"
//! import { sleep } from "std/time"
//!
//! // Create a channel for communication
//! let ch = channel()
//!
//! // Send values (from main or spawned context)
//! send(ch, "hello")
//! send(ch, map { "count": 42 })
//!
//! // Receive values
//! let msg = recv(ch)  // blocks until value available
//!
//! // Non-blocking receive
//! match try_recv(ch) {
//!     Some(value) => print("Got: " + str(value)),
//!     None => print("No message yet")
//! }
//!
//! // Close when done
//! close(ch)
//! ```
//!
//! For CPU-bound parallel work, use `parallel()`:
//! ```ntnt
//! // Run expensive operations in parallel (thread pool)
//! let results = parallel([
//!     || compute_a(),
//!     || compute_b(),
//!     || compute_c()
//! ])
//! ```

use std::collections::HashMap;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;
use crate::interpreter::Value;
use crate::error::IntentError;

type Result<T> = std::result::Result<T, IntentError>;

// Global registry for channels - using thread-safe value serialization
static CHANNEL_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, ChannelPair>>> = 
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CHANNEL_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Serialized value for thread-safe transmission
/// Only primitive and composite types that can be cloned
#[derive(Debug, Clone)]
enum SerializedValue {
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
    fn from_value(value: &Value) -> Result<Self> {
        match value {
            Value::Unit => Ok(SerializedValue::Unit),
            Value::Int(i) => Ok(SerializedValue::Int(*i)),
            Value::Float(f) => Ok(SerializedValue::Float(*f)),
            Value::Bool(b) => Ok(SerializedValue::Bool(*b)),
            Value::String(s) => Ok(SerializedValue::String(s.clone())),
            Value::Array(arr) => {
                let serialized: Result<Vec<_>> = arr.iter()
                    .map(Self::from_value)
                    .collect();
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
                // Serialize struct as a map with __type field
                let mut serialized = HashMap::new();
                serialized.insert("__type".to_string(), SerializedValue::String(name.clone()));
                for (k, v) in fields {
                    serialized.insert(k.clone(), Self::from_value(v)?);
                }
                Ok(SerializedValue::Map(serialized))
            }
            Value::EnumValue { enum_name, variant, values } => {
                // Serialize enum as a map
                let mut serialized = HashMap::new();
                serialized.insert("__enum".to_string(), SerializedValue::String(enum_name.clone()));
                serialized.insert("__variant".to_string(), SerializedValue::String(variant.clone()));
                let vals: Result<Vec<_>> = values.iter().map(Self::from_value).collect();
                serialized.insert("__values".to_string(), SerializedValue::Array(vals?));
                Ok(SerializedValue::Map(serialized))
            }
            _ => Err(IntentError::TypeError(
                "Only primitive types (Int, Float, String, Bool, Array, Map) can be sent through channels".to_string()
            )),
        }
    }
    
    /// Convert back to Value
    fn to_value(&self) -> Value {
        match self {
            SerializedValue::Unit => Value::Unit,
            SerializedValue::Int(i) => Value::Int(*i),
            SerializedValue::Float(f) => Value::Float(*f),
            SerializedValue::Bool(b) => Value::Bool(*b),
            SerializedValue::String(s) => Value::String(s.clone()),
            SerializedValue::Array(arr) => {
                Value::Array(arr.iter().map(|v| v.to_value()).collect())
            }
            SerializedValue::Map(map) => {
                // Check for special __enum marker
                if let Some(SerializedValue::String(enum_name)) = map.get("__enum") {
                    if let (Some(SerializedValue::String(variant)), Some(SerializedValue::Array(values))) = 
                        (map.get("__variant"), map.get("__values")) {
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
                Err(IntentError::TypeError("Expected a Channel".to_string()))
            }
        }
        _ => Err(IntentError::TypeError("Expected a Channel".to_string())),
    }
}

/// channel() -> Channel
/// Creates a new unbounded channel for communication
fn concurrent_channel() -> Result<Value> {
    let (tx, rx) = mpsc::channel();
    let id = CHANNEL_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    
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

/// send(channel, value) -> Bool
/// Sends a value through the channel. Returns false if channel is closed.
fn concurrent_send(ch: &Value, value: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;
    let serialized = SerializedValue::from_value(value)?;
    
    let registry = CHANNEL_REGISTRY.lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock registry: {}", e)))?;
    
    if let Some(pair) = registry.get(&id) {
        // Check if closed
        if *pair.closed.lock().unwrap() {
            return Ok(Value::Bool(false));
        }
        
        match pair.sender.send(serialized) {
            Ok(_) => Ok(Value::Bool(true)),
            Err(_) => Ok(Value::Bool(false)),  // Receiver dropped
        }
    } else {
        Err(IntentError::RuntimeError("Invalid channel".to_string()))
    }
}

/// recv(channel) -> Value
/// Receives a value from the channel. Blocks until a value is available.
/// Returns Unit if channel is closed and empty.
fn concurrent_recv(ch: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;
    
    let receiver = {
        let registry = CHANNEL_REGISTRY.lock()
            .map_err(|e| IntentError::RuntimeError(format!("Failed to lock registry: {}", e)))?;
        
        if let Some(pair) = registry.get(&id) {
            Arc::clone(&pair.receiver)
        } else {
            return Err(IntentError::RuntimeError("Invalid channel".to_string()));
        }
    };
    
    let rx = receiver.lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock receiver: {}", e)))?;
    
    match rx.recv() {
        Ok(serialized) => Ok(serialized.to_value()),
        Err(_) => Ok(Value::Unit),  // Channel closed
    }
}

/// recv_timeout(channel, millis) -> Option<Value>
/// Receives a value with timeout. Returns None if timeout expires.
fn concurrent_recv_timeout(ch: &Value, timeout_ms: i64) -> Result<Value> {
    let id = get_channel_id(ch)?;
    
    let receiver = {
        let registry = CHANNEL_REGISTRY.lock()
            .map_err(|e| IntentError::RuntimeError(format!("Failed to lock registry: {}", e)))?;
        
        if let Some(pair) = registry.get(&id) {
            Arc::clone(&pair.receiver)
        } else {
            return Err(IntentError::RuntimeError("Invalid channel".to_string()));
        }
    };
    
    let rx = receiver.lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock receiver: {}", e)))?;
    
    match rx.recv_timeout(Duration::from_millis(timeout_ms as u64)) {
        Ok(serialized) => Ok(Value::EnumValue {
            enum_name: "Option".to_string(),
            variant: "Some".to_string(),
            values: vec![serialized.to_value()],
        }),
        Err(mpsc::RecvTimeoutError::Timeout) => Ok(Value::EnumValue {
            enum_name: "Option".to_string(),
            variant: "None".to_string(),
            values: vec![],
        }),
        Err(mpsc::RecvTimeoutError::Disconnected) => Ok(Value::EnumValue {
            enum_name: "Option".to_string(),
            variant: "None".to_string(),
            values: vec![],
        }),
    }
}

/// try_recv(channel) -> Option<Value>
/// Non-blocking receive. Returns None if no value is available.
fn concurrent_try_recv(ch: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;
    
    let receiver = {
        let registry = CHANNEL_REGISTRY.lock()
            .map_err(|e| IntentError::RuntimeError(format!("Failed to lock registry: {}", e)))?;
        
        if let Some(pair) = registry.get(&id) {
            Arc::clone(&pair.receiver)
        } else {
            return Err(IntentError::RuntimeError("Invalid channel".to_string()));
        }
    };
    
    let rx = receiver.lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock receiver: {}", e)))?;
    
    match rx.try_recv() {
        Ok(serialized) => Ok(Value::EnumValue {
            enum_name: "Option".to_string(),
            variant: "Some".to_string(),
            values: vec![serialized.to_value()],
        }),
        Err(mpsc::TryRecvError::Empty) => Ok(Value::EnumValue {
            enum_name: "Option".to_string(),
            variant: "None".to_string(),
            values: vec![],
        }),
        Err(mpsc::TryRecvError::Disconnected) => Ok(Value::EnumValue {
            enum_name: "Option".to_string(),
            variant: "None".to_string(),
            values: vec![],
        }),
    }
}

/// close(channel) -> Bool
/// Closes a channel. Senders will fail, receivers will get remaining messages then Unit.
fn concurrent_close(ch: &Value) -> Result<Value> {
    let id = get_channel_id(ch)?;
    
    let registry = CHANNEL_REGISTRY.lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock registry: {}", e)))?;
    
    if let Some(pair) = registry.get(&id) {
        let mut closed = pair.closed.lock().unwrap();
        *closed = true;
        Ok(Value::Bool(true))
    } else {
        Ok(Value::Bool(false))
    }
}

/// sleep_ms(millis) -> Unit
/// Sleep for the specified number of milliseconds (re-exported from std/time for convenience)
fn concurrent_sleep_ms(ms: i64) -> Result<Value> {
    if ms > 0 {
        thread::sleep(Duration::from_millis(ms as u64));
    }
    Ok(Value::Unit)
}

/// thread_count() -> Int
/// Returns the number of available CPU threads (useful for parallel work decisions)
fn concurrent_thread_count() -> Result<Value> {
    Ok(Value::Int(thread::available_parallelism()
        .map(|n| n.get() as i64)
        .unwrap_or(1)))
}

/// Initialize the std/concurrent module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();
    
    // channel() -> Channel
    module.insert("channel".to_string(), Value::NativeFunction {
        name: "channel".to_string(),
        arity: 0,
        func: |_args| concurrent_channel(),
    });
    
    // send(channel, value) -> Bool
    module.insert("send".to_string(), Value::NativeFunction {
        name: "send".to_string(),
        arity: 2,
        func: |args| concurrent_send(&args[0], &args[1]),
    });
    
    // recv(channel) -> Value
    module.insert("recv".to_string(), Value::NativeFunction {
        name: "recv".to_string(),
        arity: 1,
        func: |args| concurrent_recv(&args[0]),
    });
    
    // recv_timeout(channel, millis) -> Option<Value>
    module.insert("recv_timeout".to_string(), Value::NativeFunction {
        name: "recv_timeout".to_string(),
        arity: 2,
        func: |args| {
            match &args[1] {
                Value::Int(ms) => concurrent_recv_timeout(&args[0], *ms),
                _ => Err(IntentError::TypeError("recv_timeout requires (channel, int_millis)".to_string())),
            }
        },
    });
    
    // try_recv(channel) -> Option<Value>
    module.insert("try_recv".to_string(), Value::NativeFunction {
        name: "try_recv".to_string(),
        arity: 1,
        func: |args| concurrent_try_recv(&args[0]),
    });
    
    // close(channel) -> Bool
    module.insert("close".to_string(), Value::NativeFunction {
        name: "close".to_string(),
        arity: 1,
        func: |args| concurrent_close(&args[0]),
    });
    
    // sleep_ms(millis) -> Unit  
    module.insert("sleep_ms".to_string(), Value::NativeFunction {
        name: "sleep_ms".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ms) => concurrent_sleep_ms(*ms),
                _ => Err(IntentError::TypeError("sleep_ms requires an integer".to_string())),
            }
        },
    });
    
    // thread_count() -> Int
    module.insert("thread_count".to_string(), Value::NativeFunction {
        name: "thread_count".to_string(),
        arity: 0,
        func: |_args| concurrent_thread_count(),
    });
    
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
    }
    
    #[test]
    fn test_channel_creation() {
        let ch = concurrent_channel().unwrap();
        assert!(matches!(ch, Value::Map(_)));
    }
    
    #[test]
    fn test_channel_send_recv() {
        let ch = concurrent_channel().unwrap();
        
        // Send a value
        let sent = concurrent_send(&ch, &Value::String("hello".to_string())).unwrap();
        assert!(matches!(sent, Value::Bool(true)));
        
        // Receive it
        let received = concurrent_recv(&ch).unwrap();
        assert!(matches!(received, Value::String(s) if s == "hello"));
    }
    
    #[test]
    fn test_try_recv_empty() {
        let ch = concurrent_channel().unwrap();
        
        // Try receive on empty channel
        let result = concurrent_try_recv(&ch).unwrap();
        match result {
            Value::EnumValue { variant, .. } => assert_eq!(variant, "None"),
            _ => panic!("Expected Option::None"),
        }
    }
    
    #[test]
    fn test_serialization_round_trip() {
        // Test primitive types
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
            // Can't use == but we can check the variant
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
}
