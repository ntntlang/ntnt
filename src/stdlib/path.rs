//! std/path module - Path manipulation utilities

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Initialize the std/path module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt join_path
    // @module std/path
    // @module_description File path manipulation and resolution
    // @signature join_path(parts: Array<String>) -> String
    // Joins path segments into a single path string.
    //
    // Renamed from join() to avoid ambiguity with join() in std/string and std/url.
    // @param parts Array of path segments to join
    // @see_also dirname, basename, normalize
    // @since v0.4.0
    // @tags #pure, #deterministic
    // @example join_path(["src", "lib", "main.tnt"]) => "src/lib/main.tnt" ~ "Joins path segments"
    module.insert(
        "join_path".to_string(),
        Value::NativeFunction {
            name: "join_path".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Array(parts) => {
                    let mut path = PathBuf::new();
                    for part in parts {
                        match part {
                            Value::String(s) => path.push(s),
                            _ => {
                                return Err(IntentError::type_error(
                                    "join() requires array of strings".to_string(),
                                ))
                            }
                        }
                    }
                    Ok(Value::String(path.to_string_lossy().to_string()))
                }
                _ => Err(IntentError::type_error(
                    "join() requires an array of path parts".to_string(),
                )),
            },
        },
    );

    // @ntnt join
    // @module std/path
    // @signature join(parts: Array<String>) -> String
    // Deprecated: use join_path() instead. Alias for backward compatibility.
    // @param parts Array of path segments to join
    // @see_also join_path
    // @since v0.1.0
    // @tags #pure, #deterministic, #deprecated
    // @example join(["src", "lib"]) => "src/lib" ~ "Deprecated: use join_path()"
    module.insert(
        "join".to_string(),
        Value::NativeFunction {
            name: "join".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                eprintln!(
                    "[DEPRECATED] join() in std/path is deprecated. Use join_path() instead."
                );
                match &args[0] {
                    Value::Array(parts) => {
                        let mut path = std::path::PathBuf::new();
                        for part in parts {
                            match part {
                                Value::String(s) => path.push(s),
                                _ => {
                                    return Err(IntentError::type_error(
                                        "join() requires array of strings".to_string(),
                                    ))
                                }
                            }
                        }
                        Ok(Value::String(path.to_string_lossy().to_string()))
                    }
                    _ => Err(IntentError::type_error(
                        "join() requires an array of path parts".to_string(),
                    )),
                }
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
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).parent() {
                    Some(p) => Ok(Value::some(Value::String(p.to_string_lossy().to_string()))),
                    None => Ok(Value::none()),
                },
                _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).file_name() {
                    Some(name) => Ok(Value::some(Value::String(
                        name.to_string_lossy().to_string(),
                    ))),
                    None => Ok(Value::none()),
                },
                _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).extension() {
                    Some(ext) => Ok(Value::some(Value::String(
                        ext.to_string_lossy().to_string(),
                    ))),
                    None => Ok(Value::none()),
                },
                _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match Path::new(path).file_stem() {
                    Some(stem) => Ok(Value::some(Value::String(
                        stem.to_string_lossy().to_string(),
                    ))),
                    None => Ok(Value::none()),
                },
                _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match std::fs::canonicalize(path) {
                    Ok(abs) => Ok(Value::ok(Value::String(abs.to_string_lossy().to_string()))),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(Path::new(path).is_absolute())),
                _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(Path::new(path).is_relative())),
                _ => Err(IntentError::type_error(
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
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(path), Value::String(ext)) => {
                    let new_path = Path::new(path).with_extension(ext);
                    Ok(Value::String(new_path.to_string_lossy().to_string()))
                }
                _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
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
                _ => Err(IntentError::type_error(
                    "normalize() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt resolve_missing
    // @module std/path
    // @signature resolve_missing(path: String) -> Result<String, String>
    // Resolve existing symlinks in order, allowing genuinely missing suffixes.
    //
    // Processes dotdot after symlinks; caps expansions at 40 and path data at 64 KiB.
    // Errors on loops, non-directories, invalid prefixes, NUL and non-UTF-8 paths.
    // Best-effort identity only; concurrent ancestor replacement is not prevented.
    // @param path Relative or absolute filesystem path.
    // @since v0.5.4
    // @example resolve_missing("new/output.bin") ~ "Resolve an unpublished destination"
    module.insert(
        "resolve_missing".into(),
        Value::NativeFunction {
            name: "resolve_missing".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match &args[0] {
                    Value::String(path) => match resolve_missing_path(path) {
                        Ok(path) => Value::ok(Value::String(path)),
                        Err(e) => Value::err(Value::String(e)),
                    },
                    _ => Value::err(Value::String(
                        "invalid_argument: path must be String".into(),
                    )),
                })
            },
        },
    );
    module
}

// Shared by the original path and absolute symlink targets. A Windows UNC
// prefix has an implicit RootDir even when no separator follows the share.
// Count bytes from the original spelling, never from the reconstructed root.
fn split_absolute_root(input: &str) -> Result<(PathBuf, &str), String> {
    use std::path::Component;
    let mut root = PathBuf::new();
    let mut consumed = 0;
    for component in Path::new(input).components() {
        match component {
            Component::Prefix(prefix) => {
                root.push(prefix.as_os_str());
                consumed = prefix.as_os_str().len();
            }
            Component::RootDir => {
                root.push(component.as_os_str());
                while input
                    .as_bytes()
                    .get(consumed)
                    .is_some_and(|b| *b == b'/' || (cfg!(windows) && *b == b'\\'))
                {
                    consumed += 1;
                }
            }
            _ => break,
        }
    }
    if !root.is_absolute() {
        return Err("invalid_argument: ambiguous path prefix".into());
    }
    // Path::strip_prefix would normalize file/. and lose non-directory errors.
    Ok((root, &input[consumed..]))
}

fn resolve_missing_path(input: &str) -> Result<String, String> {
    use std::collections::VecDeque;
    use std::path::Component;
    const LIMIT: usize = 65536;
    fn tokens(path: &str) -> VecDeque<String> {
        path.split(|c| c == '/' || (cfg!(windows) && c == '\\'))
            .map(str::to_owned)
            .collect()
    }
    if input.contains('\0') || input.len() > LIMIT || input.is_empty() {
        return Err("invalid_argument: empty, NUL or oversized path".into());
    }
    let path = Path::new(input);
    if cfg!(windows)
        && (path.has_root() != path.is_absolute()
            || matches!(path.components().next(), Some(Component::Prefix(_)))
                && !path.is_absolute())
    {
        return Err("invalid_argument: ambiguous Windows prefix".into());
    }
    let (mut resolved, relative) = if path.is_absolute() {
        split_absolute_root(input)?
    } else {
        (
            std::env::current_dir().map_err(|e| format!("io: {e}"))?,
            input,
        )
    };
    let mut pending = tokens(relative);
    let mut expansions = 0;
    while let Some(part) = pending.pop_front() {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            resolved.pop();
            continue;
        }
        resolved.push(&part);
        match std::fs::symlink_metadata(&resolved) {
            Ok(meta) if meta.is_symlink() => {
                expansions += 1;
                if expansions > 40 {
                    return Err("io: symlink expansion limit (40)".into());
                }
                let target = std::fs::read_link(&resolved).map_err(|e| format!("io: {e}"))?;
                resolved.pop();
                let text = target
                    .to_str()
                    .ok_or("invalid_argument: non-UTF-8 symlink")?;
                let text = if target.is_absolute() {
                    let (root, suffix) = split_absolute_root(text)?;
                    resolved = root;
                    suffix
                } else {
                    if target.has_root()
                        || matches!(target.components().next(), Some(Component::Prefix(_)))
                    {
                        return Err("invalid_argument: ambiguous symlink prefix".into());
                    }
                    text
                };
                let size = pending
                    .iter()
                    .try_fold(text.len(), |n, p| n.checked_add(p.len() + 1))
                    .ok_or("invalid_argument: expanded path overflow")?;
                if size > LIMIT {
                    return Err("invalid_argument: expanded path exceeds 64 KiB".into());
                }
                let mut expanded = tokens(text);
                expanded.append(&mut pending);
                pending = expanded;
            }
            Ok(meta) => {
                if !meta.is_dir() && !pending.is_empty() {
                    return Err("io: path component is not a directory".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("io: {e}")),
        }
        if resolved.as_os_str().len() > LIMIT {
            return Err("invalid_argument: resolved path exceeds 64 KiB".into());
        }
    }
    resolved
        .into_os_string()
        .into_string()
        .map_err(|_| "invalid_argument: non-UTF-8 result".into())
}

#[cfg(all(test, windows))]
mod windows_root_tests {
    use super::*;

    #[test]
    fn unc_root_and_absolute_link_target_share_raw_suffix_handling() {
        for spelling in [
            r"\\server\share",
            r"\\server\share\",
            r"\\?\UNC\server\share",
        ] {
            let (root, suffix) = split_absolute_root(spelling).unwrap();
            assert!(root.is_absolute());
            assert_eq!(suffix, "");
            // No filesystem access or network share is needed for a root alone.
            assert_eq!(PathBuf::from(resolve_missing_path(spelling).unwrap()), root);
            // read_link supplies a PathBuf; exercise that exact conversion too.
            let target = PathBuf::from(spelling);
            let (link_root, link_suffix) = split_absolute_root(target.to_str().unwrap()).unwrap();
            assert_eq!(link_root, root);
            assert_eq!(link_suffix, "");
        }
        for spelling in [r"\\server\share\file\.", r"C:\file\."] {
            assert_eq!(split_absolute_root(spelling).unwrap().1, r"file\.");
        }
    }
}
