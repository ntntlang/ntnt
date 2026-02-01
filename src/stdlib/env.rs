//! std/env module - Environment variable access

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Initialize the std/env module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt get_env
    // @module std/env
    // @module_description Environment variable access
    // @signature get_env(name: String) -> Option<String>
    // Gets an environment variable by name.
    //
    // Returns Some(value) if the variable is set, or None if it is not defined
    // in the current process environment.
    // @param name The environment variable name
    // @returns Option containing the value or None
    // @see_also load_env
    // @since v0.1.0
    // @example get_env("HOME") => Some("/Users/...") ~ "Get home directory"
    // @example get_env("UNDEFINED_VAR") => None ~ "Missing variable"
    module.insert(
        "get_env".to_string(),
        Value::NativeFunction {
            name: "get_env".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(name) => match std::env::var(name) {
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
                },
                _ => Err(IntentError::TypeError(
                    "get_env() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt args
    // @module std/env
    // @signature args() -> Array<String>
    // Returns command-line arguments as an array of strings.
    //
    // The first element is the program name, followed by any arguments
    // passed on the command line.
    // @returns Array of argument strings
    // @since v0.1.0
    // @example args() => ["ntnt", "script.tnt", "--flag"] ~ "Program name and arguments"
    module.insert(
        "args".to_string(),
        Value::NativeFunction {
            name: "args".to_string(),
            arity: 0,
            func: |_args| {
                let args: Vec<Value> = std::env::args().map(Value::String).collect();
                Ok(Value::Array(args))
            },
        },
    );

    // @ntnt cwd
    // @module std/env
    // @signature cwd() -> String
    // Returns the current working directory as a string.
    // @returns Absolute path of the current working directory
    // @since v0.1.0
    // @tags #deterministic
    // @example cwd() => "/Users/dev/project" ~ "Current directory path"
    // @error RuntimeError ~ "Failed to get cwd" fix: "Ensure the process has filesystem access"
    module.insert(
        "cwd".to_string(),
        Value::NativeFunction {
            name: "cwd".to_string(),
            arity: 0,
            func: |_args| match std::env::current_dir() {
                Ok(path) => Ok(Value::String(path.to_string_lossy().to_string())),
                Err(e) => Err(IntentError::RuntimeError(format!(
                    "Failed to get cwd: {}",
                    e
                ))),
            },
        },
    );

    // @ntnt load_env
    // @module std/env
    // @signature load_env(path: String) -> Result<Unit, String>
    // Loads environment variables from a .env file.
    //
    // Reads the file line by line, parsing KEY=VALUE pairs. Lines starting
    // with # are treated as comments and skipped. Variables are set in
    // the current process environment.
    // @param path Path to the .env file
    // @returns Result indicating success or file read error
    // @see_also get_env
    // @since v0.2.0
    // @example load_env(".env") ~ "Load default .env file"
    // @example load_env(".env.local") ~ "Load environment-specific file"
    module.insert(
        "load_env".to_string(),
        Value::NativeFunction {
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
                    _ => Err(IntentError::TypeError(
                        "load_env() requires a string path".to_string(),
                    )),
                }
            },
        },
    );

    module
}
