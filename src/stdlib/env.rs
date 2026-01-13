//! std/env module - Environment variable access

use std::collections::HashMap;
use crate::interpreter::Value;
use crate::error::IntentError;

/// Initialize the std/env module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // get_env(name) -> Option<String>
    module.insert("get_env".to_string(), Value::NativeFunction {
        name: "get_env".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(name) => {
                    match std::env::var(name) {
                        Ok(val) => Ok(Value::EnumValue {
                            enum_name: "Option".to_string(),
                            variant: "Some".to_string(),
                            values: vec![Value::String(val)],
                        }),
                        Err(_) => Ok(Value::EnumValue {
                            enum_name: "Option".to_string(),
                            variant: "None".to_string(),
                            values: vec![],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("get_env() requires a string".to_string())),
            }
        },
    });
    
    // args() -> [String] - command line arguments
    module.insert("args".to_string(), Value::NativeFunction {
        name: "args".to_string(),
        arity: 0,
        func: |_args| {
            let args: Vec<Value> = std::env::args()
                .map(Value::String)
                .collect();
            Ok(Value::Array(args))
        },
    });
    
    // cwd() -> String - current working directory
    module.insert("cwd".to_string(), Value::NativeFunction {
        name: "cwd".to_string(),
        arity: 0,
        func: |_args| {
            match std::env::current_dir() {
                Ok(path) => Ok(Value::String(path.to_string_lossy().to_string())),
                Err(e) => Err(IntentError::RuntimeError(format!("Failed to get cwd: {}", e))),
            }
        },
    });
    
    // load_env(path) -> Result<(), String> - Load .env file
    module.insert("load_env".to_string(), Value::NativeFunction {
        name: "load_env".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match std::fs::read_to_string(path) {
                        Ok(content) => {
                            for line in content.lines() {
                                let line = line.trim();
                                // Skip comments and empty lines
                                if line.is_empty() || line.starts_with('#') {
                                    continue;
                                }
                                // Parse KEY=VALUE
                                if let Some((key, value)) = line.split_once('=') {
                                    std::env::set_var(key.trim(), value.trim());
                                }
                            }
                            Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Ok".to_string(),
                                values: vec![Value::Unit],
                            })
                        }
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Failed to load .env: {}", e))],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("load_env() requires a string path".to_string())),
            }
        },
    });
    
    module
}
