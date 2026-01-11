//! std/string module - String manipulation functions

use std::collections::HashMap;
use crate::interpreter::Value;
use crate::error::IntentError;

/// Initialize the std/string module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // split(str, delimiter) -> [String]
    module.insert("split".to_string(), Value::NativeFunction {
        name: "split".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(delim)) => {
                    let parts: Vec<Value> = s.split(delim.as_str())
                        .map(|p| Value::String(p.to_string()))
                        .collect();
                    Ok(Value::Array(parts))
                }
                _ => Err(IntentError::TypeError("split() requires two strings".to_string())),
            }
        },
    });
    
    // join(arr, delimiter) -> String
    module.insert("join".to_string(), Value::NativeFunction {
        name: "join".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Array(arr), Value::String(delim)) => {
                    let parts: Vec<String> = arr.iter()
                        .map(|v| v.to_string())
                        .collect();
                    Ok(Value::String(parts.join(delim)))
                }
                _ => Err(IntentError::TypeError("join() requires array and string".to_string())),
            }
        },
    });
    
    // trim(str) -> String
    module.insert("trim".to_string(), Value::NativeFunction {
        name: "trim".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                _ => Err(IntentError::TypeError("trim() requires a string".to_string())),
            }
        },
    });
    
    // replace(str, from, to) -> String
    module.insert("replace".to_string(), Value::NativeFunction {
        name: "replace".to_string(),
        arity: 3,
        func: |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(from), Value::String(to)) => {
                    Ok(Value::String(s.replace(from.as_str(), to.as_str())))
                }
                _ => Err(IntentError::TypeError("replace() requires three strings".to_string())),
            }
        },
    });
    
    // contains(str, substr) -> Bool
    module.insert("contains".to_string(), Value::NativeFunction {
        name: "contains".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(substr)) => {
                    Ok(Value::Bool(s.contains(substr.as_str())))
                }
                _ => Err(IntentError::TypeError("contains() requires two strings".to_string())),
            }
        },
    });
    
    // starts_with(str, prefix) -> Bool
    module.insert("starts_with".to_string(), Value::NativeFunction {
        name: "starts_with".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(prefix)) => {
                    Ok(Value::Bool(s.starts_with(prefix.as_str())))
                }
                _ => Err(IntentError::TypeError("starts_with() requires two strings".to_string())),
            }
        },
    });
    
    // ends_with(str, suffix) -> Bool
    module.insert("ends_with".to_string(), Value::NativeFunction {
        name: "ends_with".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::String(suffix)) => {
                    Ok(Value::Bool(s.ends_with(suffix.as_str())))
                }
                _ => Err(IntentError::TypeError("ends_with() requires two strings".to_string())),
            }
        },
    });
    
    // to_upper(str) -> String
    module.insert("to_upper".to_string(), Value::NativeFunction {
        name: "to_upper".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                _ => Err(IntentError::TypeError("to_upper() requires a string".to_string())),
            }
        },
    });
    
    // to_lower(str) -> String
    module.insert("to_lower".to_string(), Value::NativeFunction {
        name: "to_lower".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                _ => Err(IntentError::TypeError("to_lower() requires a string".to_string())),
            }
        },
    });
    
    // char_at(str, index) -> String
    module.insert("char_at".to_string(), Value::NativeFunction {
        name: "char_at".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(s), Value::Int(idx)) => {
                    let idx = *idx as usize;
                    s.chars().nth(idx)
                        .map(|c| Value::String(c.to_string()))
                        .ok_or_else(|| IntentError::RuntimeError(format!("Index {} out of bounds", idx)))
                }
                _ => Err(IntentError::TypeError("char_at() requires string and int".to_string())),
            }
        },
    });
    
    // substring(str, start, end) -> String
    module.insert("substring".to_string(), Value::NativeFunction {
        name: "substring".to_string(),
        arity: 3,
        func: |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(start), Value::Int(end)) => {
                    let start = *start as usize;
                    let end = *end as usize;
                    let chars: Vec<char> = s.chars().collect();
                    if start > chars.len() || end > chars.len() || start > end {
                        return Err(IntentError::RuntimeError("Invalid substring range".to_string()));
                    }
                    Ok(Value::String(chars[start..end].iter().collect()))
                }
                _ => Err(IntentError::TypeError("substring() requires string, int, int".to_string())),
            }
        },
    });
    
    module
}
