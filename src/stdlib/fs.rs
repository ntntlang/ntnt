//! std/fs module - File system operations

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::fs;

/// Initialize the std/fs module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

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
            func: |args| match &args[0] {
                Value::String(path) => match fs::read_to_string(path) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::read(path) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match (&args[0], &args[1]) {
                (Value::String(path), Value::String(content)) => match fs::write(path, content) {
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
                },
                _ => Err(IntentError::TypeError(
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
                    _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).exists())),
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).is_file())),
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => Ok(Value::Bool(std::path::Path::new(path).is_dir())),
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::create_dir(path) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::create_dir_all(path) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::read_dir(path) {
                    Ok(entries) => {
                        let files: Vec<Value> = entries
                            .flatten()
                            .map(|e| Value::String(e.path().to_string_lossy().to_string()))
                            .collect();
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::remove_file(path) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::remove_dir(path) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::remove_dir_all(path) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match (&args[0], &args[1]) {
                (Value::String(from), Value::String(to)) => match fs::rename(from, to) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match (&args[0], &args[1]) {
                (Value::String(from), Value::String(to)) => match fs::copy(from, to) {
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
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(path) => match fs::metadata(path) {
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
                },
                _ => Err(IntentError::TypeError(
                    "file_size() requires a string path".to_string(),
                )),
            },
        },
    );

    module
}
