//! std/path module - Path manipulation utilities

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Initialize the std/path module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // join(parts...) -> String - Join path components (takes array)
    module.insert(
        "join".to_string(),
        Value::NativeFunction {
            name: "join".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Array(parts) => {
                    let mut path = PathBuf::new();
                    for part in parts {
                        match part {
                            Value::String(s) => path.push(s),
                            _ => {
                                return Err(IntentError::TypeError(
                                    "join() requires array of strings".to_string(),
                                ))
                            }
                        }
                    }
                    Ok(Value::String(path.to_string_lossy().to_string()))
                }
                _ => Err(IntentError::TypeError(
                    "join() requires an array of path parts".to_string(),
                )),
            },
        },
    );

    // dirname(path) -> Option<String>
    module.insert(
        "dirname".to_string(),
        Value::NativeFunction {
            name: "dirname".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).parent() {
                    Some(p) => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: vec![Value::String(p.to_string_lossy().to_string())],
                    }),
                    None => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    }),
                },
                _ => Err(IntentError::TypeError(
                    "dirname() requires a string path".to_string(),
                )),
            },
        },
    );

    // basename(path) -> Option<String>
    module.insert(
        "basename".to_string(),
        Value::NativeFunction {
            name: "basename".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).file_name() {
                    Some(name) => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: vec![Value::String(name.to_string_lossy().to_string())],
                    }),
                    None => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    }),
                },
                _ => Err(IntentError::TypeError(
                    "basename() requires a string path".to_string(),
                )),
            },
        },
    );

    // extension(path) -> Option<String>
    module.insert(
        "extension".to_string(),
        Value::NativeFunction {
            name: "extension".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).extension() {
                    Some(ext) => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: vec![Value::String(ext.to_string_lossy().to_string())],
                    }),
                    None => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    }),
                },
                _ => Err(IntentError::TypeError(
                    "extension() requires a string path".to_string(),
                )),
            },
        },
    );

    // stem(path) -> Option<String> - Filename without extension
    module.insert(
        "stem".to_string(),
        Value::NativeFunction {
            name: "stem".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).file_stem() {
                    Some(stem) => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: vec![Value::String(stem.to_string_lossy().to_string())],
                    }),
                    None => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    }),
                },
                _ => Err(IntentError::TypeError(
                    "stem() requires a string path".to_string(),
                )),
            },
        },
    );

    // resolve(path) -> Result<String, Error> - Absolute path
    module.insert(
        "resolve".to_string(),
        Value::NativeFunction {
            name: "resolve".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => match std::fs::canonicalize(path) {
                    Ok(abs) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![Value::String(abs.to_string_lossy().to_string())],
                    }),
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(e.to_string())],
                    }),
                },
                _ => Err(IntentError::TypeError(
                    "resolve() requires a string path".to_string(),
                )),
            },
        },
    );

    // is_absolute(path) -> Bool
    module.insert(
        "is_absolute".to_string(),
        Value::NativeFunction {
            name: "is_absolute".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(Path::new(path).is_absolute())),
                _ => Err(IntentError::TypeError(
                    "is_absolute() requires a string path".to_string(),
                )),
            },
        },
    );

    // is_relative(path) -> Bool
    module.insert(
        "is_relative".to_string(),
        Value::NativeFunction {
            name: "is_relative".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(Path::new(path).is_relative())),
                _ => Err(IntentError::TypeError(
                    "is_relative() requires a string path".to_string(),
                )),
            },
        },
    );

    // with_extension(path, ext) -> String
    module.insert(
        "with_extension".to_string(),
        Value::NativeFunction {
            name: "with_extension".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(path), Value::String(ext)) => {
                    let new_path = Path::new(path).with_extension(ext);
                    Ok(Value::String(new_path.to_string_lossy().to_string()))
                }
                _ => Err(IntentError::TypeError(
                    "with_extension() requires two strings".to_string(),
                )),
            },
        },
    );

    // normalize(path) -> String - Cleans up .. and . components
    module.insert(
        "normalize".to_string(),
        Value::NativeFunction {
            name: "normalize".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => {
                    let p = Path::new(path);
                    let mut normalized = PathBuf::new();
                    for component in p.components() {
                        use std::path::Component;
                        match component {
                            Component::ParentDir => {
                                if !normalized.pop() {
                                    normalized.push("..");
                                }
                            }
                            Component::CurDir => {}
                            c => normalized.push(c.as_os_str()),
                        }
                    }
                    Ok(Value::String(normalized.to_string_lossy().to_string()))
                }
                _ => Err(IntentError::TypeError(
                    "normalize() requires a string path".to_string(),
                )),
            },
        },
    );

    module
}
