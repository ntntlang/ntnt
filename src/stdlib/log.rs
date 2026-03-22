//! Structured Logging Module (std/log)
//!
//! Provides structured logging with configurable log levels and optional
//! JSON-serialized context data. All log output goes to stderr to keep
//! stdout clean for application output.

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};

/// Global log level: 0=debug, 1=info, 2=warn, 3=error
/// Default is INFO (1)
static LOG_LEVEL: AtomicU8 = AtomicU8::new(1);

/// Log level constants
const LEVEL_DEBUG: u8 = 0;
const LEVEL_INFO: u8 = 1;
const LEVEL_WARN: u8 = 2;
const LEVEL_ERROR: u8 = 3;

/// Convert log level name to numeric value
fn parse_log_level(level: &str) -> Option<u8> {
    match level.to_lowercase().as_str() {
        "debug" => Some(LEVEL_DEBUG),
        "info" => Some(LEVEL_INFO),
        "warn" | "warning" => Some(LEVEL_WARN),
        "error" => Some(LEVEL_ERROR),
        _ => None,
    }
}

/// Get current timestamp in ISO 8601 format
fn timestamp() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Convert NTNT Value to JSON for log context
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Unit => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
        Value::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Value::EnumValue {
            variant, values, ..
        } => {
            if values.is_empty() {
                serde_json::Value::String(variant.clone())
            } else if values.len() == 1 {
                let mut obj = serde_json::Map::new();
                obj.insert(variant.clone(), value_to_json(&values[0]));
                serde_json::Value::Object(obj)
            } else {
                let mut obj = serde_json::Map::new();
                obj.insert(
                    variant.clone(),
                    serde_json::Value::Array(values.iter().map(value_to_json).collect()),
                );
                serde_json::Value::Object(obj)
            }
        }
        _ => serde_json::Value::String(format!("{:?}", value)),
    }
}

/// Write a log entry to stderr
fn write_log(level: u8, level_name: &str, message: &str, data: Option<&Value>) {
    let current_level = LOG_LEVEL.load(Ordering::Relaxed);
    if level < current_level {
        return;
    }

    let ts = timestamp();
    let data_str = match data {
        Some(val) => {
            let json = value_to_json(val);
            format!(" {}", json)
        }
        None => String::new(),
    };

    eprintln!("{} [{}] {}{}", ts, level_name, message, data_str);
}

/// Initialize the std/log module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt log_debug
    // @module std/log
    // @module_description Structured logging with configurable levels and JSON context
    // @signature log_debug(message: String, data?: Any) -> Unit
    // Log a message at DEBUG level.
    //
    // Debug logs are for detailed diagnostic information during development.
    // They are hidden by default (log level is INFO). Use `set_log_level("debug")`
    // to enable. Output goes to stderr in the format:
    // `2026-02-02T10:30:00Z [DEBUG] message {"context":"data"}`
    // @param message The log message.
    // @param data Optional context data to serialize as JSON.
    // @returns Unit
    // @see_also log_info, log_warn, log_error, set_log_level
    // @since v0.3.11
    // @tags #logging, #debug
    // @example log_debug("Processing request", map { "id": 123 }) ~ "Debug with context"
    // @example log_debug("Checkpoint reached") ~ "Simple debug message"
    module.insert(
        "log_debug".to_string(),
        Value::NativeFunction {
            name: "log_debug".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "log_debug() requires 1 or 2 arguments (message, optional data)"
                            .to_string(),
                    ));
                }

                let message = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "log_debug() message must be a string".to_string(),
                        ))
                    }
                };

                let data = if args.len() == 2 {
                    Some(&args[1])
                } else {
                    None
                };

                write_log(LEVEL_DEBUG, "DEBUG", &message, data);
                Ok(Value::Unit)
            },
        },
    );

    // @ntnt log_info
    // @module std/log
    // @signature log_info(message: String, data?: Any) -> Unit
    // Log a message at INFO level.
    //
    // Info logs are for general operational information. This is the default
    // log level. Output goes to stderr in the format:
    // `2026-02-02T10:30:00Z [INFO] message {"context":"data"}`
    // @param message The log message.
    // @param data Optional context data to serialize as JSON.
    // @returns Unit
    // @see_also log_debug, log_warn, log_error, set_log_level
    // @since v0.3.11
    // @tags #logging
    // @example log_info("Server started", map { "port": 8080 }) ~ "Info with context"
    // @example log_info("User logged in") ~ "Simple info message"
    module.insert(
        "log_info".to_string(),
        Value::NativeFunction {
            name: "log_info".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "log_info() requires 1 or 2 arguments (message, optional data)".to_string(),
                    ));
                }

                let message = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "log_info() message must be a string".to_string(),
                        ))
                    }
                };

                let data = if args.len() == 2 {
                    Some(&args[1])
                } else {
                    None
                };

                write_log(LEVEL_INFO, "INFO", &message, data);
                Ok(Value::Unit)
            },
        },
    );

    // @ntnt log_warn
    // @module std/log
    // @signature log_warn(message: String, data?: Any) -> Unit
    // Log a message at WARN level.
    //
    // Warn logs are for potentially harmful situations or unexpected behavior
    // that doesn't prevent operation. Output goes to stderr in the format:
    // `2026-02-02T10:30:00Z [WARN] message {"context":"data"}`
    // @param message The log message.
    // @param data Optional context data to serialize as JSON.
    // @returns Unit
    // @see_also log_debug, log_info, log_error, set_log_level
    // @since v0.3.11
    // @tags #logging
    // @example log_warn("Rate limit approaching", map { "current": 95, "max": 100 }) ~ "Warning with context"
    // @example log_warn("Deprecated API called") ~ "Simple warning"
    module.insert(
        "log_warn".to_string(),
        Value::NativeFunction {
            name: "log_warn".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "log_warn() requires 1 or 2 arguments (message, optional data)".to_string(),
                    ));
                }

                let message = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "log_warn() message must be a string".to_string(),
                        ))
                    }
                };

                let data = if args.len() == 2 {
                    Some(&args[1])
                } else {
                    None
                };

                write_log(LEVEL_WARN, "WARN", &message, data);
                Ok(Value::Unit)
            },
        },
    );

    // @ntnt log_error
    // @module std/log
    // @signature log_error(message: String, data?: Any) -> Unit
    // Log a message at ERROR level.
    //
    // Error logs are for serious problems that may prevent normal operation.
    // Output goes to stderr in the format:
    // `2026-02-02T10:30:00Z [ERROR] message {"context":"data"}`
    // @param message The log message.
    // @param data Optional context data to serialize as JSON.
    // @returns Unit
    // @see_also log_debug, log_info, log_warn, set_log_level
    // @since v0.3.11
    // @tags #logging
    // @example log_error("Database connection failed", map { "host": "db.example.com" }) ~ "Error with context"
    // @example log_error("Critical failure") ~ "Simple error message"
    module.insert(
        "log_error".to_string(),
        Value::NativeFunction {
            name: "log_error".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "log_error() requires 1 or 2 arguments (message, optional data)"
                            .to_string(),
                    ));
                }

                let message = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "log_error() message must be a string".to_string(),
                        ))
                    }
                };

                let data = if args.len() == 2 {
                    Some(&args[1])
                } else {
                    None
                };

                write_log(LEVEL_ERROR, "ERROR", &message, data);
                Ok(Value::Unit)
            },
        },
    );

    // @ntnt set_log_level
    // @module std/log
    // @signature set_log_level(level: String) -> Unit
    // Set the global log level.
    //
    // Controls which log messages are output. Messages below the set level
    // are silently ignored. Valid levels: "debug", "info", "warn", "error".
    // Default is "info".
    // @param level The log level name: "debug", "info", "warn", or "error".
    // @returns Unit
    // @see_also log_debug, log_info, log_warn, log_error
    // @since v0.3.11
    // @tags #logging, #configuration
    // @example set_log_level("debug") ~ "Enable debug logging"
    // @example set_log_level("error") ~ "Only show errors"
    // @error TypeError ~ "Invalid log level" fix: "Use 'debug', 'info', 'warn', or 'error'"
    module.insert(
        "set_log_level".to_string(),
        Value::NativeFunction {
            name: "set_log_level".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let level_str = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "set_log_level() requires a string".to_string(),
                        ))
                    }
                };

                match parse_log_level(&level_str) {
                    Some(level) => {
                        LOG_LEVEL.store(level, Ordering::Relaxed);
                        Ok(Value::Unit)
                    }
                    None => Err(IntentError::type_error(format!(
                        "Invalid log level '{}'. Use 'debug', 'info', 'warn', or 'error'",
                        level_str
                    ))),
                }
            },
        },
    );

    // @ntnt request_logger
    // @module std/log
    // @signature request_logger() -> Function
    // Create a request logging middleware function.
    //
    // Returns a function suitable for use with `use_middleware()` that logs
    // incoming HTTP requests at INFO level. Logs the HTTP method and path
    // for each request.
    // @returns A middleware function that logs requests.
    // @see_also log_info, use_middleware
    // @since v0.3.11
    // @tags #logging, #http, #middleware
    // @example use_middleware(request_logger()) ~ "Log all incoming requests"
    module.insert(
        "request_logger".to_string(),
        Value::NativeFunction {
            name: "request_logger".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| {
                // Return a native function that can be used as middleware
                Ok(Value::NativeFunction {
                    name: "request_logger_middleware".to_string(),
                    arity: 1,
                    max_arity: 1,
                    requires: None,
                    func: |args| {
                        let (method, path) = match &args[0] {
                            Value::Map(map) => {
                                let method = match map.get("method") {
                                    Some(Value::String(m)) => m.clone(),
                                    _ => "UNKNOWN".to_string(),
                                };
                                let path = match map.get("path") {
                                    Some(Value::String(p)) => p.clone(),
                                    _ => "/".to_string(),
                                };
                                (method, path)
                            }
                            _ => ("UNKNOWN".to_string(), "/".to_string()),
                        };

                        write_log(LEVEL_INFO, "INFO", &format!("{} {}", method, path), None);
                        Ok(Value::Unit)
                    },
                })
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_log_level() {
        assert_eq!(parse_log_level("debug"), Some(LEVEL_DEBUG));
        assert_eq!(parse_log_level("DEBUG"), Some(LEVEL_DEBUG));
        assert_eq!(parse_log_level("info"), Some(LEVEL_INFO));
        assert_eq!(parse_log_level("INFO"), Some(LEVEL_INFO));
        assert_eq!(parse_log_level("warn"), Some(LEVEL_WARN));
        assert_eq!(parse_log_level("warning"), Some(LEVEL_WARN));
        assert_eq!(parse_log_level("error"), Some(LEVEL_ERROR));
        assert_eq!(parse_log_level("ERROR"), Some(LEVEL_ERROR));
        assert_eq!(parse_log_level("invalid"), None);
    }

    #[test]
    fn test_value_to_json() {
        assert_eq!(value_to_json(&Value::Unit), serde_json::Value::Null);
        assert_eq!(
            value_to_json(&Value::Bool(true)),
            serde_json::Value::Bool(true)
        );
        assert_eq!(
            value_to_json(&Value::Int(42)),
            serde_json::Value::Number(42.into())
        );
        assert_eq!(
            value_to_json(&Value::String("test".to_string())),
            serde_json::Value::String("test".to_string())
        );
    }

    #[test]
    fn test_log_level_filtering() {
        // Set to ERROR level
        LOG_LEVEL.store(LEVEL_ERROR, Ordering::Relaxed);

        // This should be filtered out (level too low)
        // In a real test we'd capture stderr, but here we just verify no panic
        write_log(LEVEL_INFO, "INFO", "This should be filtered", None);
        write_log(LEVEL_ERROR, "ERROR", "This should appear", None);

        // Reset to default
        LOG_LEVEL.store(LEVEL_INFO, Ordering::Relaxed);
    }
}
