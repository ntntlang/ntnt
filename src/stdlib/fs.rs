//! std/fs module - File system operations

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::fs;
#[path = "fs_owned.rs"]
pub mod owned;

/// Initialize the std/fs module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt write_file_atomic
    // @module std/fs
    // @signature write_file_atomic(path: String, content: String | Array<Int>, options?: Map<String, Any>) -> Result<Unit, String>
    // Atomically replace a file using an owned same-parent temporary inode and rename.
    // Default sync:true syncs file and parent on Unix. Non-Unix requires sync:false and no mode.
    // Unix mode defaults to 0600, restricted by umask at creation; explicit broader mode also exposes staging.
    // Replaces terminal symlinks, uses a new inode, and does not preserve ownership or ACLs.
    // Trusted ancestors and filesystem rename guarantees are required; this is not race-free path authorization.
    // Err distinguishes unpublished failure (including cleanup failure) from published durability_uncertain.
    // @since v0.5.4
    // @param path Destination in a trusted existing parent directory.
    // @param content UTF-8 text or checked integer bytes in 0..255.
    // @param options Optional mode 0..511 and sync Bool (default true); non-Unix requires sync:false without mode.
    // @example write_file_atomic("example.bin", [0, 255], map { "sync": false }) ~ "Publish bytes with portable atomic visibility"
    module.insert(
        "write_file_atomic".into(),
        Value::NativeFunction {
            name: "write_file_atomic".into(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                Ok(match owned::write_atomic(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt temp_file
    // @module std/fs
    // @signature temp_file(options?: Map<String, Any>) -> Result<TempFile, String>
    // Create an owned private file. Options: trusted parent and separator/NUL-free prefix. Unix mode 0600 under umask; other platforms use OS ACL rules.
    // At most 128 live resources. Aliases share identity; JSON/task/channel transfer is rejected.
    // Explicit close is recommended; last-owner Drop and runtime shutdown are best-effort safety nets, not crash guarantees.
    // Callers must not replace owned paths or their ancestors; recursive cleanup does not follow interior symlinks.
    // @since v0.5.4
    // @param options Optional trusted parent String and separator/NUL-free prefix String.
    // @example temp_file() ~ "Create an owned file; use temp_path and explicitly temp_close"
    module.insert(
        "temp_file".into(),
        Value::NativeFunction {
            name: "temp_file".into(),
            arity: 0,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match owned::create_temp(args, false) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt temp_dir
    // @module std/fs
    // @signature temp_dir(options?: Map<String, Any>) -> Result<TempDir, String>
    // Create an owned directory. Options: trusted parent and separator/NUL-free prefix. Unix mode 0700 under umask; other platforms use OS ACL rules.
    // At most 128 live resources. Aliases share identity; JSON/task/channel transfer is rejected.
    // Explicit close is recommended; last-owner Drop and runtime shutdown are best-effort safety nets, not crash guarantees.
    // Callers must not replace owned paths or their ancestors; recursive cleanup does not follow interior symlinks.
    // @since v0.5.4
    // @param options Optional trusted parent String and separator/NUL-free prefix String.
    // @example temp_dir() ~ "Create an owned directory; explicitly temp_close after use"
    module.insert(
        "temp_dir".into(),
        Value::NativeFunction {
            name: "temp_dir".into(),
            arity: 0,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match owned::create_temp(args, true) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt temp_path
    // @module std/fs
    // @signature temp_path(resource: TempFile | TempDir) -> Result<String, String>
    // Expose the open owned path for filesystem APIs. Closed and non-UTF8 paths return Err.
    // At most 128 live resources. Aliases share identity; JSON/task/channel transfer is rejected.
    // Explicit close is recommended; last-owner Drop and runtime shutdown are best-effort safety nets, not crash guarantees.
    // Callers must not replace owned paths or their ancestors; recursive cleanup does not follow interior symlinks.
    // @since v0.5.4
    // @param resource An open native TempFile or TempDir.
    // @example temp_path(resource) ~ "Get the owned UTF-8 path"
    module.insert(
        "temp_path".into(),
        Value::NativeFunction {
            name: "temp_path".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match owned::temp_path(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt temp_close
    // @module std/fs
    // @signature temp_close(resource: TempFile | TempDir) -> Result<Unit, String>
    // Close all aliases and remove the owned path. Success is idempotent; cleanup failure remains a terminal Err on repeated close, with no automatic retry.
    // At most 128 live resources. Aliases share identity; JSON/task/channel transfer is rejected.
    // Explicit close is recommended; last-owner Drop and runtime shutdown are best-effort safety nets, not crash guarantees.
    // Callers must not replace owned paths or their ancestors; recursive cleanup does not follow interior symlinks.
    // @since v0.5.4
    // @param resource A native TempFile or TempDir, including a closed alias.
    // @example temp_close(resource) ~ "Invalidate aliases and clean up the owned path"
    module.insert(
        "temp_close".into(),
        Value::NativeFunction {
            name: "temp_close".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match owned::temp_close(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt lstat
    // @module std/fs
    // @signature lstat(path: String) -> Result<Map<String, Any>, String>
    // Inspect without following the terminal symlink: size, is_file, is_dir, is_symlink, modified and created (Unix seconds, 0 if unavailable, matching file_stat).
    // @since v0.5.4
    // @param path Path whose terminal entry is inspected without following it.
    // @example lstat("example.bin") ~ "Inspect file or symlink metadata"
    module.insert(
        "lstat".into(),
        Value::NativeFunction {
            name: "lstat".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match owned::lstat(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt read_link
    // @module std/fs
    // @signature read_link(path: String) -> Result<String, String>
    // Return the exact symlink target without canonicalization. Non-UTF8 targets and non-symlinks return Err.
    // @since v0.5.4
    // @param path Symlink whose exact target should be read.
    // @example read_link("example-link") ~ "Read the target without canonicalizing it"
    module.insert(
        "read_link".into(),
        Value::NativeFunction {
            name: "read_link".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match owned::read_link(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt symlink
    // @module std/fs
    // @signature symlink(target: String, path: String, kind?: String) -> Result<Unit, String>
    // Create a symlink without replacement. Kind defaults to file; file or dir is required on Windows. Permission or Windows privilege denial returns Err.
    // @since v0.5.4
    // @param target Target string stored by the symlink.
    // @param path New link path in a trusted parent; existing entries are never replaced.
    // @param kind Optional file (default) or dir, used for Windows symlink creation.
    // @example symlink("example.bin", "example-link", "file") ~ "Create a file symlink; handle OS privilege errors"
    module.insert(
        "symlink".into(),
        Value::NativeFunction {
            name: "symlink".into(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                Ok(match owned::symlink(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt read_file
    // @module std/fs
    // @module_description File system operations: reading, writing, and directory management
    // @signature read_file(path: String) -> Result<String, String>
    // Read the entire contents of a file as a UTF-8 string.
    //
    // Opens the file at the given path and returns its contents. The file must
    // be valid UTF-8. Returns a Result wrapping the file content on success or
    // an error message on failure.
    // @param path The filesystem path to the file to read.
    // @returns Result<String, String> Ok with file contents, or Err with error message.
    // @see_also read_bytes, write_file, exists
    // @since v0.1.0
    // @tags #filesystem
    // @example read_file("hello.txt") => Ok("Hello, world!") ~ "Read file contents"
    // @error TypeError ~ "read_file() requires a string path" fix: "Pass a String argument"
    module.insert(
        "read_file".to_string(),
        Value::NativeFunction {
            name: "read_file".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::read_to_string(path) {
                    Ok(content) => Ok(Value::ok(Value::String(content))),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "read_file() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt read_bytes
    // @module std/fs
    // @signature read_bytes(path: String) -> Result<Array<Int>, String>
    // Read the entire contents of a file as raw bytes.
    //
    // Opens the file at the given path and returns its contents as an array of
    // integers (0-255), one per byte. Useful for binary files that are not valid
    // UTF-8.
    // @param path The filesystem path to the file to read.
    // @returns Result<Array<Int>, String> Ok with array of byte values, or Err with error message.
    // @see_also read_file, write_file, file_size
    // @since v0.1.0
    // @tags #filesystem
    // @example read_bytes("data.bin") => Ok([72, 101, 108, 108, 111]) ~ "Read binary file as byte array"
    // @error TypeError ~ "read_bytes() requires a string path" fix: "Pass a String argument"
    module.insert(
        "read_bytes".to_string(),
        Value::NativeFunction {
            name: "read_bytes".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::read(path) {
                    Ok(bytes) => {
                        let arr: Vec<Value> = bytes.iter().map(|b| Value::Int(*b as i64)).collect();
                        Ok(Value::ok(Value::Array(arr)))
                    }
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "read_bytes() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt write_file
    // @module std/fs
    // @signature write_file(path: String, content: String) -> Result<Unit, String>
    // Write a string to a file, creating or overwriting it.
    //
    // Writes the given content to the file at path. If the file already exists
    // it is truncated and overwritten. If it does not exist it is created.
    // Parent directories must already exist.
    // @param path The filesystem path to write to.
    // @param content The string content to write.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also read_file, append_file, copy
    // @since v0.1.0
    // @tags #filesystem
    // @example write_file("out.txt", "hello") => Ok(()) ~ "Write string to file"
    // @error TypeError ~ "write_file() requires path and content strings" fix: "Pass two String arguments"
    module.insert(
        "write_file".to_string(),
        Value::NativeFunction {
            name: "write_file".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(path), Value::String(content)) => match fs::write(path, content) {
                    Ok(()) => Ok(Value::ok(Value::Unit)),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "write_file() requires path and content strings".to_string(),
                )),
            },
        },
    );

    // @ntnt append_file
    // @module std/fs
    // @signature append_file(path: String, content: String) -> Result<Unit, String>
    // Append a string to the end of a file, creating it if it does not exist.
    //
    // Opens the file in append mode and writes the content at the end. If the
    // file does not exist it is created. Existing content is preserved.
    // @param path The filesystem path to append to.
    // @param content The string content to append.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also write_file, read_file
    // @since v0.1.0
    // @tags #filesystem
    // @example append_file("log.txt", "new line\n") => Ok(()) ~ "Append to file"
    // @error TypeError ~ "append_file() requires path and content strings" fix: "Pass two String arguments"
    module.insert(
        "append_file".to_string(),
        Value::NativeFunction {
            name: "append_file".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
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
                            Ok(()) => Ok(Value::ok(Value::Unit)),
                            Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                        }
                    }
                    _ => Err(IntentError::type_error(
                        "append_file() requires path and content strings".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt exists
    // @module std/fs
    // @signature exists(path: String) -> Bool
    // Check whether a file or directory exists at the given path.
    //
    // Returns true if a filesystem entry (file, directory, or symlink) exists
    // at the specified path, false otherwise.
    // @param path The filesystem path to check.
    // @returns Bool True if the path exists, false otherwise.
    // @see_also is_file, is_dir
    // @since v0.1.0
    // @tags #filesystem
    // @example exists("/tmp") => true ~ "Check path existence"
    // @error TypeError ~ "exists() requires a string path" fix: "Pass a String argument"
    module.insert(
        "exists".to_string(),
        Value::NativeFunction {
            name: "exists".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).exists())),
                _ => Err(IntentError::type_error(
                    "exists() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt is_file
    // @module std/fs
    // @signature is_file(path: String) -> Bool
    // Check whether the path points to a regular file.
    //
    // Returns true only if the path exists and is a regular file (not a
    // directory or symlink to a directory).
    // @param path The filesystem path to check.
    // @returns Bool True if the path is a regular file, false otherwise.
    // @see_also is_dir, exists
    // @since v0.1.0
    // @tags #filesystem
    // @example is_file("config.tnt") => true ~ "Check if path is a file"
    // @error TypeError ~ "is_file() requires a string path" fix: "Pass a String argument"
    module.insert(
        "is_file".to_string(),
        Value::NativeFunction {
            name: "is_file".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).is_file())),
                _ => Err(IntentError::type_error(
                    "is_file() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt is_dir
    // @module std/fs
    // @signature is_dir(path: String) -> Bool
    // Check whether the path points to a directory.
    //
    // Returns true only if the path exists and is a directory.
    // @param path The filesystem path to check.
    // @returns Bool True if the path is a directory, false otherwise.
    // @see_also is_file, exists
    // @since v0.1.0
    // @tags #filesystem
    // @example is_dir("/tmp") => true ~ "Check if path is a directory"
    // @error TypeError ~ "is_dir() requires a string path" fix: "Pass a String argument"
    module.insert(
        "is_dir".to_string(),
        Value::NativeFunction {
            name: "is_dir".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).is_dir())),
                _ => Err(IntentError::type_error(
                    "is_dir() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt mkdir
    // @module std/fs
    // @signature mkdir(path: String) -> Result<Unit, String>
    // Create a single directory.
    //
    // Creates the directory at the given path. The parent directory must already
    // exist. Fails if the directory already exists or if the parent is missing.
    // Use mkdir_all to create intermediate directories automatically.
    // @param path The filesystem path for the new directory.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also mkdir_all, remove_dir, readdir
    // @since v0.1.0
    // @tags #filesystem
    // @example mkdir("new_dir") => Ok(()) ~ "Create a directory"
    // @error TypeError ~ "mkdir() requires a string path" fix: "Pass a String argument"
    module.insert(
        "mkdir".to_string(),
        Value::NativeFunction {
            name: "mkdir".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::create_dir(path) {
                    Ok(()) => Ok(Value::ok(Value::Unit)),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "mkdir() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt mkdir_all
    // @module std/fs
    // @signature mkdir_all(path: String) -> Result<Unit, String>
    // Create a directory and all missing parent directories.
    //
    // Recursively creates directories along the given path. If the directory
    // already exists, this is not an error. Equivalent to `mkdir -p` on Unix.
    // @param path The filesystem path for the new directory tree.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also mkdir, remove_dir_all, readdir
    // @since v0.1.0
    // @tags #filesystem
    // @example mkdir_all("a/b/c") => Ok(()) ~ "Create nested directories"
    // @error TypeError ~ "mkdir_all() requires a string path" fix: "Pass a String argument"
    module.insert(
        "mkdir_all".to_string(),
        Value::NativeFunction {
            name: "mkdir_all".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::create_dir_all(path) {
                    Ok(()) => Ok(Value::ok(Value::Unit)),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "mkdir_all() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt readdir
    // @module std/fs
    // @signature readdir(path: String) -> Result<Array<String>, String>
    // List the entries of a directory.
    //
    // Returns an array of full path strings for every entry in the directory.
    // The order is filesystem-dependent and not guaranteed to be sorted.
    // Entries that cannot be read are silently skipped.
    // @param path The filesystem path to the directory to list.
    // @returns Result<Array<String>, String> Ok with array of entry paths, or Err with error message.
    // @see_also mkdir, is_dir, exists
    // @since v0.1.0
    // @tags #filesystem
    // @example readdir(".") => Ok(["./file.tnt", "./lib"]) ~ "List directory entries"
    // @error TypeError ~ "readdir() requires a string path" fix: "Pass a String argument"
    module.insert(
        "readdir".to_string(),
        Value::NativeFunction {
            name: "readdir".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::read_dir(path) {
                    Ok(entries) => {
                        let files: Vec<Value> = entries
                            .flatten()
                            .map(|e| Value::String(e.path().to_string_lossy().to_string()))
                            .collect();
                        Ok(Value::ok(Value::Array(files)))
                    }
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "readdir() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt remove
    // @module std/fs
    // @signature remove(path: String) -> Result<Unit, String>
    // Remove a file from the filesystem.
    //
    // Deletes the file at the given path. Fails if the path does not exist or
    // points to a directory. Use remove_dir or remove_dir_all for directories.
    // @param path The filesystem path to the file to remove.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also remove_dir, remove_dir_all, exists
    // @since v0.1.0
    // @tags #filesystem
    // @example remove("temp.txt") => Ok(()) ~ "Delete a file"
    // @error TypeError ~ "remove() requires a string path" fix: "Pass a String argument"
    module.insert(
        "remove".to_string(),
        Value::NativeFunction {
            name: "remove".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::remove_file(path) {
                    Ok(()) => Ok(Value::ok(Value::Unit)),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "remove() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt remove_dir
    // @module std/fs
    // @signature remove_dir(path: String) -> Result<Unit, String>
    // Remove an empty directory.
    //
    // Deletes the directory at the given path. The directory must be empty;
    // if it contains any entries the operation will fail. Use remove_dir_all
    // to recursively remove a directory and its contents.
    // @param path The filesystem path to the empty directory to remove.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also remove_dir_all, remove, mkdir
    // @since v0.1.0
    // @tags #filesystem
    // @example remove_dir("empty_dir") => Ok(()) ~ "Remove an empty directory"
    // @error TypeError ~ "remove_dir() requires a string path" fix: "Pass a String argument"
    module.insert(
        "remove_dir".to_string(),
        Value::NativeFunction {
            name: "remove_dir".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::remove_dir(path) {
                    Ok(()) => Ok(Value::ok(Value::Unit)),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "remove_dir() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt remove_dir_all
    // @module std/fs
    // @signature remove_dir_all(path: String) -> Result<Unit, String>
    // Recursively remove a directory and all of its contents.
    //
    // Deletes the directory at the given path along with every file and
    // subdirectory it contains. Use with caution as this operation is
    // irreversible.
    // @param path The filesystem path to the directory to remove recursively.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also remove_dir, remove, mkdir_all
    // @since v0.1.0
    // @tags #filesystem
    // @example remove_dir_all("build") => Ok(()) ~ "Remove directory tree"
    // @error TypeError ~ "remove_dir_all() requires a string path" fix: "Pass a String argument"
    module.insert(
        "remove_dir_all".to_string(),
        Value::NativeFunction {
            name: "remove_dir_all".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::remove_dir_all(path) {
                    Ok(()) => Ok(Value::ok(Value::Unit)),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "remove_dir_all() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt rename
    // @module std/fs
    // @signature rename(from: String, to: String) -> Result<Unit, String>
    // Rename or move a file or directory.
    //
    // Renames the filesystem entry at `from` to the path `to`. This can also
    // be used to move entries across directories on the same filesystem. Fails
    // if the source does not exist or the destination's parent directory is
    // missing.
    // @param from The current filesystem path.
    // @param to The desired new filesystem path.
    // @returns Result<Unit, String> Ok on success, or Err with error message.
    // @see_also copy, remove, exists
    // @since v0.1.0
    // @tags #filesystem
    // @example rename("old.txt", "new.txt") => Ok(()) ~ "Rename a file"
    // @error TypeError ~ "rename() requires two string paths" fix: "Pass two String arguments"
    module.insert(
        "rename".to_string(),
        Value::NativeFunction {
            name: "rename".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(from), Value::String(to)) => match fs::rename(from, to) {
                    Ok(()) => Ok(Value::ok(Value::Unit)),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "rename() requires two string paths".to_string(),
                )),
            },
        },
    );

    // @ntnt copy
    // @module std/fs
    // @signature copy(from: String, to: String) -> Result<Int, String>
    // Copy a file to a new location, returning the number of bytes copied.
    //
    // Copies the file at `from` to the path `to`. If the destination file
    // already exists it is overwritten. The source must be a regular file.
    // On success the Result contains the number of bytes written.
    // @param from The filesystem path of the source file.
    // @param to The filesystem path for the destination copy.
    // @returns Result<Int, String> Ok with byte count copied, or Err with error message.
    // @see_also rename, write_file, read_file
    // @since v0.1.0
    // @tags #filesystem
    // @example copy("src.txt", "dst.txt") => Ok(1024) ~ "Copy file and get byte count"
    // @error TypeError ~ "copy() requires two string paths" fix: "Pass two String arguments"
    module.insert(
        "copy".to_string(),
        Value::NativeFunction {
            name: "copy".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(from), Value::String(to)) => match fs::copy(from, to) {
                    Ok(bytes) => Ok(Value::ok(Value::Int(bytes as i64))),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "copy() requires two string paths".to_string(),
                )),
            },
        },
    );

    // @ntnt file_size
    // @module std/fs
    // @signature file_size(path: String) -> Result<Int, String>
    // Get the size of a file in bytes.
    //
    // Returns the length in bytes of the file at the given path by reading its
    // filesystem metadata. Fails if the path does not exist or is not accessible.
    // @param path The filesystem path to query.
    // @returns Result<Int, String> Ok with file size in bytes, or Err with error message.
    // @see_also exists, is_file, read_file
    // @since v0.1.0
    // @tags #filesystem
    // @example file_size("data.txt") => Ok(256) ~ "Get file size in bytes"
    // @error TypeError ~ "file_size() requires a string path" fix: "Pass a String argument"
    module.insert(
        "file_size".to_string(),
        Value::NativeFunction {
            name: "file_size".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::metadata(path) {
                    Ok(meta) => Ok(Value::ok(Value::Int(meta.len() as i64))),
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "file_size() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt file_stat
    // @module std/fs
    // @signature file_stat(path: String) -> Result<Map, String>
    // Get filesystem metadata for a file or directory.
    //
    // Returns a map with size (bytes), modified (unix timestamp, 0 if unavailable),
    // created (unix timestamp, 0 if unavailable), is_file, and is_dir fields.
    // Useful for cache busting, conditional processing, and file management.
    // @param path The filesystem path to query.
    // @returns Result<Map, String> Ok with metadata map, or Err with error message.
    // @see_also file_size, exists, is_file, is_dir
    // @since v0.4.6
    // @tags #filesystem
    // @example file_stat("styles.css") => Ok(map { "size": 1234, "modified": 1773882626, "created": 1773800000, "is_file": true, "is_dir": false }) ~ "Get file metadata"
    // @error TypeError ~ "file_stat() requires a string path" fix: "Pass a String argument"
    module.insert(
        "file_stat".to_string(),
        Value::NativeFunction {
            name: "file_stat".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(path) => match fs::metadata(path) {
                    Ok(meta) => {
                        let mut map = HashMap::new();
                        map.insert(
                            "size".to_string(),
                            Value::Int(meta.len().min(i64::MAX as u64) as i64),
                        );
                        map.insert("is_file".to_string(), Value::Bool(meta.is_file()));
                        map.insert("is_dir".to_string(), Value::Bool(meta.is_dir()));

                        // modified/created: 0 if unavailable (some platforms don't support these)
                        let modified = meta
                            .modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
                            .unwrap_or(0);
                        map.insert("modified".to_string(), Value::Int(modified));

                        let created = meta
                            .created()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs().min(i64::MAX as u64) as i64)
                            .unwrap_or(0);
                        map.insert("created".to_string(), Value::Int(created));

                        Ok(Value::ok(Value::Map(map)))
                    }
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "file_stat() requires a string path".to_string(),
                )),
            },
        },
    );

    // @ntnt write_bytes
    // @module std/fs
    // @signature write_bytes(path: String, content: Array<Int>) -> Result<Unit, String>
    // Write up to 16 MiB of validated bytes, creating or truncating with ordinary symlink semantics.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @param content Integer bytes 0..255, maximum 16 MiB; validated before opening.
    // @since v0.5.4
    // @example write_bytes("local-data", [0, 255]) ~ "Inspect Result before continuing"
    module.insert(
        "write_bytes".into(),
        Value::NativeFunction {
            name: "write_bytes".into(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                Ok(match system_write_bytes(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt write_file_exclusive
    // @module std/fs
    // @signature write_file_exclusive(path: String, content: String | Array<Int>, options?: Map<String, Any>) -> Result<Unit, String>
    // Unix exclusive creation with initial mode (default 384, restricted by umask) and sync (default true). Options are mode and sync only. Trusted ancestors required. Never removes a partially written file.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @param content Exact UTF-8 String or checked integer bytes; maximum 16 MiB.
    // @param options Optional mode (0..511, default 384) and sync (Bool, default true).
    // @since v0.5.4
    // @example write_file_exclusive("local-data", [0, 255]) ~ "Inspect Result before continuing"
    module.insert(
        "write_file_exclusive".into(),
        Value::NativeFunction {
            name: "write_file_exclusive".into(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                Ok(match system_write_file_exclusive(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt mkdir_private
    // @module std/fs
    // @signature mkdir_private(path: String, mode?: Int) -> Result<Unit, String>
    // Unix single directory creation, default mode 448 restricted by umask; existing entries fail.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @param mode Optional integer 0..511, default 448; restricted by inherited umask.
    // @since v0.5.4
    // @example mkdir_private("local-data") ~ "Inspect Result before continuing"
    module.insert(
        "mkdir_private".into(),
        Value::NativeFunction {
            name: "mkdir_private".into(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                Ok(match system_mkdir_private(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt sync_file
    // @module std/fs
    // @signature sync_file(path: String) -> Result<Unit, String>
    // Sync an existing regular file descriptor. Unix rejects terminal symlinks and special files; non-Unix terminal links follow OS open semantics.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @since v0.5.4
    // @example sync_file("local-data") ~ "Inspect Result before continuing"
    module.insert(
        "sync_file".into(),
        Value::NativeFunction {
            name: "sync_file".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match system_sync_file(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt sync_dir
    // @module std/fs
    // @signature sync_dir(path: String) -> Result<Unit, String>
    // Unix directory descriptor sync. OS/filesystem durability semantics apply; no physical-media guarantee.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @since v0.5.4
    // @example sync_dir("local-data") ~ "Inspect Result before continuing"
    module.insert(
        "sync_dir".into(),
        Value::NativeFunction {
            name: "sync_dir".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match system_sync_dir(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt file_permissions
    // @module std/fs
    // @signature file_permissions(path: String) -> Result<Map<String, Any>, String>
    // Unix lstat returns mode (including special bits), uid, gid, is_symlink, is_file and is_dir.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @since v0.5.4
    // @example file_permissions("local-data") ~ "Inspect Result before continuing"
    module.insert(
        "file_permissions".into(),
        Value::NativeFunction {
            name: "file_permissions".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match system_file_permissions(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt chmod
    // @module std/fs
    // @signature chmod(path: String, mode: Int) -> Result<Unit, String>
    // Unix chmod follows terminal symlinks, accepts 0..511. Not a race-free authorization operation.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @param mode Integer 0..511; special bits rejected.
    // @since v0.5.4
    // @example chmod("local-data", 384) ~ "Inspect Result before continuing"
    module.insert(
        "chmod".into(),
        Value::NativeFunction {
            name: "chmod".into(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                Ok(match system_chmod(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt chown
    // @module std/fs
    // @signature chown(path: String, uid: Int, gid: Int) -> Result<Unit, String>
    // Unix chown follows terminal symlinks. IDs exclude negative values and all-ones sentinel. OS may clear set-ID bits.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @param uid Nonnegative OS user ID excluding the all-ones sentinel.
    // @param gid Nonnegative OS group ID excluding the all-ones sentinel.
    // @since v0.5.4
    // @example chown("local-data", 1000, 1000) ~ "Inspect Result before continuing"
    module.insert(
        "chown".into(),
        Value::NativeFunction {
            name: "chown".into(),
            arity: 3,
            max_arity: 3,
            requires: None,
            func: |args| {
                Ok(match system_chown(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );
    // @ntnt access
    // @module std/fs
    // @signature access(path: String, mode: String) -> Result<Bool, String>
    // Unix real-ID access check: empty mode checks existence; nonrepeating r/w/x combinations check access. Advisory and TOCTOU-prone, not permission to open.
    //
    // Errors use invalid_argument:, already_exists:, unsupported: or io: classes.
    // Unix-only operations return unsupported before mutation on other platforms.
    // @param path Filesystem path with trusted ancestors; NUL is rejected.
    // @param mode Empty for existence or a nonrepeating r/w/x combination; real-ID semantics.
    // @since v0.5.4
    // @example access("local-data", "r") ~ "Inspect Result before continuing"
    module.insert(
        "access".into(),
        Value::NativeFunction {
            name: "access".into(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                Ok(match system_access(args) {
                    Ok(v) => Value::ok(v),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    module
}

const MAX_WRITE_BYTES: usize = 16 * 1024 * 1024;
type SystemResult = std::result::Result<Value, String>;
fn system_path(args: &[Value]) -> std::result::Result<&str, String> {
    match args.first() {
        Some(Value::String(s)) if !s.contains('\0') => Ok(s),
        _ => Err("invalid_argument: expected path without NUL".into()),
    }
}
fn system_mode(value: Option<&Value>, default: u32) -> std::result::Result<u32, String> {
    match value {
        None => Ok(default),
        Some(Value::Int(n)) if (0..=511).contains(n) => Ok(*n as u32),
        _ => Err("invalid_argument: mode must be an integer in 0..511".into()),
    }
}
fn system_io(e: std::io::Error) -> String {
    format!(
        "{}: {e}",
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            "already_exists"
        } else {
            "io"
        }
    )
}
fn system_bytes(value: &Value, text: bool) -> std::result::Result<Vec<u8>, String> {
    match value {
        Value::String(s) if text && s.len() <= MAX_WRITE_BYTES => Ok(s.as_bytes().to_vec()),
        Value::Array(a) if a.len() <= MAX_WRITE_BYTES => {
            // Validate the whole input before allocating a conversion buffer or opening a file.
            if a.iter()
                .any(|v| !matches!(v, Value::Int(n) if (0..=255).contains(n)))
            {
                return Err("invalid_argument: bytes must be integers in 0..255".into());
            }
            Ok(a.iter()
                .map(|v| {
                    if let Value::Int(n) = v {
                        *n as u8
                    } else {
                        unreachable!()
                    }
                })
                .collect())
        }
        _ => Err("invalid_argument: expected bytes (maximum 16 MiB)".into()),
    }
}
fn system_write_bytes(args: &[Value]) -> SystemResult {
    let path = system_path(args)?;
    let bytes = system_bytes(&args[1], false)?;
    fs::write(path, bytes).map_err(system_io)?;
    Ok(Value::Unit)
}
fn system_write_file_exclusive(args: &[Value]) -> SystemResult {
    let path = system_path(args)?;
    let mut mode = 384;
    let mut sync = true;
    if let Some(options) = args.get(2) {
        let Value::Map(options) = options else {
            return Err("invalid_argument: options must be a map".into());
        };
        for (key, value) in options {
            match key.as_str() {
                "mode" => mode = system_mode(Some(value), 384)?,
                "sync" => {
                    if let Value::Bool(b) = value {
                        sync = *b
                    } else {
                        return Err("invalid_argument: sync must be Bool".into());
                    }
                }
                _ => return Err("invalid_argument: unknown exclusive-write option".into()),
            }
        }
    }
    let bytes = system_bytes(&args[1], true)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(path)
            .map_err(system_io)?;
        write_and_sync(&mut file, &bytes, sync, |file| file.sync_all())?;
        Ok(Value::Unit)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode, sync, bytes);
        Err("unsupported: secure initial-mode exclusive creation requires Unix".into())
    }
}
#[cfg(unix)]
fn write_and_sync<W: std::io::Write>(
    file: &mut W,
    bytes: &[u8],
    sync: bool,
    sync_all: impl FnOnce(&mut W) -> std::io::Result<()>,
) -> std::result::Result<(), String> {
    file.write_all(bytes).map_err(|e| {
        format!("write_failed: file_created=true; content_may_be_partial=true; {e}")
    })?;
    if sync {
        sync_all(file).map_err(|e| {
            format!("durability_uncertain: file_created=true; write_completed=true; {e}")
        })?;
    }
    Ok(())
}
fn system_mkdir_private(args: &[Value]) -> SystemResult {
    let path = system_path(args)?;
    let mode = system_mode(args.get(1), 448)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .mode(mode)
            .create(path)
            .map_err(system_io)?;
        Ok(Value::Unit)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err("unsupported: initial-mode directory creation requires Unix".into())
    }
}
fn system_sync_file(args: &[Value]) -> SystemResult {
    system_sync(args, false)
}
fn system_sync_dir(args: &[Value]) -> SystemResult {
    system_sync(args, true)
}
fn system_sync(args: &[Value], directory: bool) -> SystemResult {
    let path = system_path(args)?;
    #[cfg(not(unix))]
    if directory {
        return Err("unsupported: directory sync requires Unix".into());
    }
    let mut options = fs::OpenOptions::new();
    options.read(true);
    // Windows FlushFileBuffers requires GENERIC_WRITE on an existing handle.
    #[cfg(windows)]
    options.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(system_io)?;
    let metadata = file.metadata().map_err(system_io)?;
    if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
        return Err("invalid_argument: wrong file kind for sync".into());
    }
    file.sync_all().map_err(system_io)?;
    Ok(Value::Unit)
}
fn system_file_permissions(args: &[Value]) -> SystemResult {
    let path = system_path(args)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = fs::symlink_metadata(path).map_err(system_io)?;
        Ok(Value::Map(HashMap::from([
            ("mode".into(), Value::Int(i64::from(m.mode() & 0o7777))),
            ("uid".into(), Value::Int(i64::from(m.uid()))),
            ("gid".into(), Value::Int(i64::from(m.gid()))),
            ("is_symlink".into(), Value::Bool(m.is_symlink())),
            ("is_file".into(), Value::Bool(m.is_file())),
            ("is_dir".into(), Value::Bool(m.is_dir())),
        ])))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("unsupported: POSIX metadata requires Unix".into())
    }
}
fn system_chmod(args: &[Value]) -> SystemResult {
    let path = system_path(args)?;
    let mode = system_mode(args.get(1), 384)?;
    #[cfg(unix)]
    {
        let path = std::ffi::CString::new(path).map_err(|_| "invalid_argument: NUL".to_string())?;
        // SAFETY: live NUL-terminated path, validated permission bits.
        if unsafe { libc::chmod(path.as_ptr(), mode as libc::mode_t) } != 0 {
            return Err(system_io(std::io::Error::last_os_error()));
        }
        Ok(Value::Unit)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Err("unsupported: chmod requires Unix".into())
    }
}
fn system_id(value: &Value) -> std::result::Result<u32, String> {
    match value {
        Value::Int(n) if (0..i64::from(u32::MAX)).contains(n) => Ok(*n as u32),
        _ => Err("invalid_argument: ID must fit OS type and exclude all-ones sentinel".into()),
    }
}
fn system_chown(args: &[Value]) -> SystemResult {
    let path = system_path(args)?;
    let uid = system_id(&args[1])?;
    let gid = system_id(&args[2])?;
    #[cfg(unix)]
    {
        let uid = libc::uid_t::try_from(uid)
            .map_err(|_| "invalid_argument: uid out of range".to_string())?;
        let gid = libc::gid_t::try_from(gid)
            .map_err(|_| "invalid_argument: gid out of range".to_string())?;
        if uid == libc::uid_t::MAX || gid == libc::gid_t::MAX {
            return Err("invalid_argument: sentinel ID".into());
        }
        let path = std::ffi::CString::new(path).map_err(|_| "invalid_argument: NUL".to_string())?;
        // SAFETY: live NUL-terminated path and checked OS ID types.
        if unsafe { libc::chown(path.as_ptr(), uid, gid) } != 0 {
            return Err(system_io(std::io::Error::last_os_error()));
        }
        Ok(Value::Unit)
    }
    #[cfg(not(unix))]
    {
        let _ = (path, uid, gid);
        Err("unsupported: chown requires Unix".into())
    }
}
fn system_access(args: &[Value]) -> SystemResult {
    let path = system_path(args)?;
    let Value::String(mode) = &args[1] else {
        return Err("invalid_argument: access mode must be String".into());
    };
    let mut bits = 0;
    for c in mode.chars() {
        let bit = match c {
            'r' => 4,
            'w' => 2,
            'x' => 1,
            _ => return Err("invalid_argument: access mode must contain only r/w/x".into()),
        };
        if bits & bit != 0 {
            return Err("invalid_argument: repeated access mode".into());
        }
        bits |= bit;
    }
    #[cfg(unix)]
    {
        let flags = if bits == 0 {
            libc::F_OK
        } else {
            (if bits & 4 != 0 { libc::R_OK } else { 0 })
                | (if bits & 2 != 0 { libc::W_OK } else { 0 })
                | (if bits & 1 != 0 { libc::X_OK } else { 0 })
        };
        let path = std::ffi::CString::new(path).map_err(|_| "invalid_argument: NUL".to_string())?;
        // SAFETY: live NUL-terminated path and access(2) flags; no credential mutation.
        if unsafe { libc::access(path.as_ptr(), flags) } == 0 {
            return Ok(Value::Bool(true));
        }
        let e = std::io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::ENOENT | libc::ENOTDIR | libc::EACCES | libc::EPERM | libc::EROFS) => {
                Ok(Value::Bool(false))
            }
            _ => Err(system_io(e)),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, bits);
        Err("unsupported: real-ID access requires Unix".into())
    }
}

#[cfg(all(test, unix))]
mod system_tests {
    use super::*;
    use std::io::{self, Write};
    struct ShortWriter {
        bytes: Vec<u8>,
        fail: bool,
    }
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if self.fail && self.bytes.len() >= 2 {
                return Err(io::Error::other("injected write failure"));
            }
            let n = bytes.len().min(2);
            self.bytes.extend_from_slice(&bytes[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn short_write_and_sync_failures_report_committed_state() {
        let mut writer = ShortWriter {
            bytes: vec![],
            fail: false,
        };
        write_and_sync(&mut writer, b"abcdef", true, |w| {
            assert_eq!(w.bytes, b"abcdef");
            Ok(())
        })
        .unwrap();
        let mut writer = ShortWriter {
            bytes: vec![],
            fail: true,
        };
        let e = write_and_sync(&mut writer, b"abcdef", true, |_| {
            panic!("must not sync partial write")
        })
        .unwrap_err();
        assert!(e.starts_with("write_failed: file_created=true; content_may_be_partial=true"));
        assert_eq!(writer.bytes, b"ab");
        let mut writer = ShortWriter {
            bytes: vec![],
            fail: false,
        };
        let e = write_and_sync(&mut writer, b"abcdef", true, |_| {
            Err(io::Error::other("injected sync failure"))
        })
        .unwrap_err();
        assert!(e.starts_with("durability_uncertain: file_created=true; write_completed=true"));
        assert_eq!(writer.bytes, b"abcdef");
    }
}
