//! std/fs module - File system operations

use std::collections::HashMap;
use std::fs;
use crate::interpreter::Value;
use crate::error::IntentError;

/// Initialize the std/fs module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // read_file(path) -> Result<String, Error>
    module.insert("read_file".to_string(), Value::NativeFunction {
        name: "read_file".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::read_to_string(path) {
                        Ok(content) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::String(content)],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("read_file() requires a string path".to_string())),
            }
        },
    });
    
    // read_bytes(path) -> Result<[Int], Error>
    module.insert("read_bytes".to_string(), Value::NativeFunction {
        name: "read_bytes".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::read(path) {
                        Ok(bytes) => {
                            let arr: Vec<Value> = bytes.iter().map(|b| Value::Int(*b as i64)).collect();
                            Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Ok".to_string(),
                                values: vec![Value::Array(arr)],
                            })
                        }
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("read_bytes() requires a string path".to_string())),
            }
        },
    });
    
    // write_file(path, content) -> Result<Unit, Error>
    module.insert("write_file".to_string(), Value::NativeFunction {
        name: "write_file".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(path), Value::String(content)) => {
                    match fs::write(path, content) {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("write_file() requires path and content strings".to_string())),
            }
        },
    });
    
    // append_file(path, content) -> Result<Unit, Error>
    module.insert("append_file".to_string(), Value::NativeFunction {
        name: "append_file".to_string(),
        arity: 2,
        func: |args| {
            use std::fs::OpenOptions;
            use std::io::Write;
            
            match (&args[0], &args[1]) {
                (Value::String(path), Value::String(content)) => {
                    let result = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .and_then(|mut f| f.write_all(content.as_bytes()));
                    
                    match result {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("append_file() requires path and content strings".to_string())),
            }
        },
    });
    
    // exists(path) -> Bool
    module.insert("exists".to_string(), Value::NativeFunction {
        name: "exists".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).exists())),
                _ => Err(IntentError::TypeError("exists() requires a string path".to_string())),
            }
        },
    });
    
    // is_file(path) -> Bool
    module.insert("is_file".to_string(), Value::NativeFunction {
        name: "is_file".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).is_file())),
                _ => Err(IntentError::TypeError("is_file() requires a string path".to_string())),
            }
        },
    });
    
    // is_dir(path) -> Bool
    module.insert("is_dir".to_string(), Value::NativeFunction {
        name: "is_dir".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).is_dir())),
                _ => Err(IntentError::TypeError("is_dir() requires a string path".to_string())),
            }
        },
    });
    
    // mkdir(path) -> Result<Unit, Error>
    module.insert("mkdir".to_string(), Value::NativeFunction {
        name: "mkdir".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::create_dir(path) {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("mkdir() requires a string path".to_string())),
            }
        },
    });
    
    // mkdir_all(path) -> Result<Unit, Error> - Creates dir and all parents
    module.insert("mkdir_all".to_string(), Value::NativeFunction {
        name: "mkdir_all".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::create_dir_all(path) {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("mkdir_all() requires a string path".to_string())),
            }
        },
    });
    
    // readdir(path) -> Result<[String], Error>
    module.insert("readdir".to_string(), Value::NativeFunction {
        name: "readdir".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::read_dir(path) {
                        Ok(entries) => {
                            let mut files: Vec<Value> = Vec::new();
                            for entry in entries {
                                if let Ok(e) = entry {
                                    files.push(Value::String(e.path().to_string_lossy().to_string()));
                                }
                            }
                            Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Ok".to_string(),
                                values: vec![Value::Array(files)],
                            })
                        }
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("readdir() requires a string path".to_string())),
            }
        },
    });
    
    // remove(path) -> Result<Unit, Error> - Removes file
    module.insert("remove".to_string(), Value::NativeFunction {
        name: "remove".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::remove_file(path) {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("remove() requires a string path".to_string())),
            }
        },
    });
    
    // remove_dir(path) -> Result<Unit, Error> - Removes empty directory
    module.insert("remove_dir".to_string(), Value::NativeFunction {
        name: "remove_dir".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::remove_dir(path) {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("remove_dir() requires a string path".to_string())),
            }
        },
    });
    
    // remove_dir_all(path) -> Result<Unit, Error> - Removes directory and contents
    module.insert("remove_dir_all".to_string(), Value::NativeFunction {
        name: "remove_dir_all".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::remove_dir_all(path) {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("remove_dir_all() requires a string path".to_string())),
            }
        },
    });
    
    // rename(from, to) -> Result<Unit, Error>
    module.insert("rename".to_string(), Value::NativeFunction {
        name: "rename".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(from), Value::String(to)) => {
                    match fs::rename(from, to) {
                        Ok(()) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Unit],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("rename() requires two string paths".to_string())),
            }
        },
    });
    
    // copy(from, to) -> Result<Int, Error> - Returns bytes copied
    module.insert("copy".to_string(), Value::NativeFunction {
        name: "copy".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(from), Value::String(to)) => {
                    match fs::copy(from, to) {
                        Ok(bytes) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Int(bytes as i64)],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("copy() requires two string paths".to_string())),
            }
        },
    });
    
    // file_size(path) -> Result<Int, Error>
    module.insert("file_size".to_string(), Value::NativeFunction {
        name: "file_size".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(path) => {
                    match fs::metadata(path) {
                        Ok(meta) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Int(meta.len() as i64)],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("file_size() requires a string path".to_string())),
            }
        },
    });
    
    module
}
