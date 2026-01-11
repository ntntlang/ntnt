//! std/time module - Time and date operations

use std::collections::HashMap;
use std::time::{SystemTime, Duration, UNIX_EPOCH};
use crate::interpreter::Value;
use crate::error::IntentError;

/// Convert days since Unix epoch to year, month, day
fn days_to_ymd(days: i64) -> (i64, i64, i64) {
    // Algorithm adapted from Howard Hinnant's date algorithms
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m as i64, d as i64)
}

/// Initialize the std/time module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // now() -> Int - Current Unix timestamp in seconds
    module.insert("now".to_string(), Value::NativeFunction {
        name: "now".to_string(),
        arity: 0,
        func: |_args| {
            match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => Ok(Value::Int(duration.as_secs() as i64)),
                Err(_) => Err(IntentError::RuntimeError("System time before Unix epoch".to_string())),
            }
        },
    });
    
    // now_millis() -> Int - Current Unix timestamp in milliseconds
    module.insert("now_millis".to_string(), Value::NativeFunction {
        name: "now_millis".to_string(),
        arity: 0,
        func: |_args| {
            match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => Ok(Value::Int(duration.as_millis() as i64)),
                Err(_) => Err(IntentError::RuntimeError("System time before Unix epoch".to_string())),
            }
        },
    });
    
    // now_nanos() -> Int - Current Unix timestamp in nanoseconds
    module.insert("now_nanos".to_string(), Value::NativeFunction {
        name: "now_nanos".to_string(),
        arity: 0,
        func: |_args| {
            match SystemTime::now().duration_since(UNIX_EPOCH) {
                Ok(duration) => Ok(Value::Int(duration.as_nanos() as i64)),
                Err(_) => Err(IntentError::RuntimeError("System time before Unix epoch".to_string())),
            }
        },
    });
    
    // sleep(millis) -> Unit - Sleep for milliseconds
    module.insert("sleep".to_string(), Value::NativeFunction {
        name: "sleep".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ms) => {
                    if *ms < 0 {
                        return Err(IntentError::RuntimeError("sleep() requires non-negative milliseconds".to_string()));
                    }
                    std::thread::sleep(Duration::from_millis(*ms as u64));
                    Ok(Value::Unit)
                }
                _ => Err(IntentError::TypeError("sleep() requires an integer (milliseconds)".to_string())),
            }
        },
    });
    
    // elapsed(start_millis) -> Int - Milliseconds since start
    module.insert("elapsed".to_string(), Value::NativeFunction {
        name: "elapsed".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(start) => {
                    match SystemTime::now().duration_since(UNIX_EPOCH) {
                        Ok(duration) => {
                            let now = duration.as_millis() as i64;
                            Ok(Value::Int(now - start))
                        }
                        Err(_) => Err(IntentError::RuntimeError("System time error".to_string())),
                    }
                }
                _ => Err(IntentError::TypeError("elapsed() requires a start timestamp".to_string())),
            }
        },
    });
    
    // format_timestamp(timestamp, format) -> String
    // Simple format: %Y-%m-%d %H:%M:%S
    module.insert("format_timestamp".to_string(), Value::NativeFunction {
        name: "format_timestamp".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(timestamp), Value::String(format)) => {
                    // Convert Unix timestamp to broken-down time
                    let ts = *timestamp;
                    
                    // Calculate date/time components (basic UTC conversion)
                    let days_since_epoch = ts / 86400;
                    let time_of_day = ts % 86400;
                    
                    let hours = time_of_day / 3600;
                    let minutes = (time_of_day % 3600) / 60;
                    let seconds = time_of_day % 60;
                    
                    // Calculate year, month, day from days since epoch
                    let (year, month, day) = days_to_ymd(days_since_epoch);
                    
                    let result = format
                        .replace("%Y", &format!("{:04}", year))
                        .replace("%m", &format!("{:02}", month))
                        .replace("%d", &format!("{:02}", day))
                        .replace("%H", &format!("{:02}", hours))
                        .replace("%M", &format!("{:02}", minutes))
                        .replace("%S", &format!("{:02}", seconds));
                    
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError("format_timestamp() requires int and format string".to_string())),
            }
        },
    });
    
    // duration_secs(seconds) -> Map { secs, millis, nanos }
    module.insert("duration_secs".to_string(), Value::NativeFunction {
        name: "duration_secs".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(secs) => {
                    let mut map = HashMap::new();
                    map.insert("secs".to_string(), Value::Int(*secs));
                    map.insert("millis".to_string(), Value::Int(*secs * 1000));
                    map.insert("nanos".to_string(), Value::Int(*secs * 1_000_000_000));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::TypeError("duration_secs() requires an integer".to_string())),
            }
        },
    });
    
    // duration_millis(millis) -> Map { secs, millis, nanos }
    module.insert("duration_millis".to_string(), Value::NativeFunction {
        name: "duration_millis".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Int(ms) => {
                    let mut map = HashMap::new();
                    map.insert("secs".to_string(), Value::Int(*ms / 1000));
                    map.insert("millis".to_string(), Value::Int(*ms));
                    map.insert("nanos".to_string(), Value::Int(*ms * 1_000_000));
                    Ok(Value::Map(map))
                }
                _ => Err(IntentError::TypeError("duration_millis() requires an integer".to_string())),
            }
        },
    });
    
    module
}
