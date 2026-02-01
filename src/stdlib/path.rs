//! std/path module - Path manipulation utilities

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Initialize the std/path module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt join
    // @module std/path
    // @module_description File path manipulation and resolution
    // @signature join(parts: Array<String>) -> String
    // Joins path segments into a single path string.
    // @param parts Array of path segments to join
    // @see_also dirname, basename, normalize
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example join(["src", "lib", "main.tnt"]) => "src/lib/main.tnt" ~ "Joins path segments"
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

    // @ntnt dirname
    // @module std/path
    // @signature dirname(path: String) -> Option<String>
    // Returns the directory portion of a path.
    // @param path The file path to extract the directory from
    // @see_also basename, join, extension, stem
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example dirname("src/lib/main.tnt") => Some("src/lib") ~ "Returns directory portion"
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

    // @ntnt basename
    // @module std/path
    // @signature basename(path: String) -> Option<String>
    // Returns the filename portion of a path.
    // @param path The file path to extract the filename from
    // @see_also dirname, extension, stem, join
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example basename("src/lib/main.tnt") => Some("main.tnt") ~ "Returns filename portion"
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

    // @ntnt extension
    // @module std/path
    // @signature extension(path: String) -> Option<String>
    // Returns the file extension without the leading dot.
    // @param path The file path to extract the extension from
    // @see_also stem, basename, with_extension
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example extension("main.tnt") => Some("tnt") ~ "Returns file extension without dot"
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

    // @ntnt stem
    // @module std/path
    // @signature stem(path: String) -> Option<String>
    // Returns the filename without its extension.
    // @param path The file path to extract the stem from
    // @see_also extension, basename, with_extension
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example stem("main.tnt") => Some("main") ~ "Returns filename without extension"
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

    // @ntnt resolve
    // @module std/path
    // @signature resolve(path: String) -> Result<String, String>
    // Resolves a path to an absolute path using filesystem canonicalize.
    // @param path The file path to resolve
    // @see_also is_absolute, normalize
    // @since v0.2.0
    // @tags #filesystem
    // @example resolve(".") => Ok("/Users/dev/project") ~ "Resolves current directory to absolute path"
    // @example resolve("nonexistent") => Err("No such file or directory") ~ "Returns Err for missing path"
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

    // @ntnt is_absolute
    // @module std/path
    // @signature is_absolute(path: String) -> Bool
    // Returns true if the path is absolute.
    // @param path The file path to check
    // @see_also is_relative
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example is_absolute("/usr/bin") => true ~ "Absolute path starts with /"
    // @example is_absolute("src/main.tnt") => false ~ "Relative path is not absolute"
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

    // @ntnt is_relative
    // @module std/path
    // @signature is_relative(path: String) -> Bool
    // Returns true if the path is relative.
    // @param path The file path to check
    // @see_also is_absolute
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example is_relative("src/main.tnt") => true ~ "Relative path without leading /"
    // @example is_relative("/usr/bin") => false ~ "Absolute path is not relative"
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

    // @ntnt with_extension
    // @module std/path
    // @signature with_extension(path: String, ext: String) -> String
    // Returns the path with its extension changed to the given extension.
    // @param path The original file path
    // @param ext The new extension (without leading dot)
    // @see_also extension, stem, basename
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example with_extension("file.txt", "md") => "file.md" ~ "Changes file extension"
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

    // @ntnt normalize
    // @module std/path
    // @signature normalize(path: String) -> String
    // Cleans up `..` and `.` path components without touching the filesystem.
    // @param path The file path to normalize
    // @see_also join, resolve, dirname
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example normalize("a/b/../c") => "a/c" ~ "Cleans up path components"
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
