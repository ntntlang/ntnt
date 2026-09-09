//! Supervised native process execution.
//!
//! `run()` waits for one command; `start()` returns an opaque `Process` handle for
//! `wait()`, `try_wait()`, `terminate()`, and `kill()`. Both launch executables
//! directly, require `NTNT_PROCESS_ENABLE`, and honor an optional exact-path
//! `NTNT_PROCESS_ALLOW` allowlist. All active commands are registered for runtime
//! shutdown, started processes are monitored autonomously, captured pipes use
//! cancellable workers that are joined after a bounded drain, and completed results
//! remain in a bounded cache for repeated handle operations.

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;
type OutputReadResult = std::result::Result<Vec<u8>, String>;
type OutputReader = JoinHandle<OutputReadResult>;

const DEFAULT_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_TERMINATION_GRACE_MS: u64 = 5_000;
const IO_THREAD_DRAIN_GRACE_MS: u64 = 500;
const MAX_RETAINED_TERMINAL_PROCESSES: usize = 64;
const MAX_RETAINED_TERMINAL_BYTES: usize = 64 * 1024 * 1024;

#[cfg(test)]
static ACTIVE_OUTPUT_READERS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(test)]
struct ActiveOutputReader;

#[cfg(test)]
impl Drop for ActiveOutputReader {
    fn drop(&mut self) {
        ACTIVE_OUTPUT_READERS.fetch_sub(1, Ordering::AcqRel);
    }
}

trait ProcessPipe: Read + Send + 'static {
    fn prepare(&self) -> std::io::Result<()>;
    fn read_available(
        &mut self,
        buffer: &mut [u8],
        io_cancel: &AtomicBool,
    ) -> std::io::Result<Option<usize>>;
}

#[cfg(unix)]
fn set_pipe_nonblocking(pipe: &impl std::os::fd::AsRawFd) -> std::io::Result<()> {
    let descriptor = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
macro_rules! impl_process_pipe {
    ($pipe:ty) => {
        impl ProcessPipe for $pipe {
            fn prepare(&self) -> std::io::Result<()> {
                set_pipe_nonblocking(self)
            }

            fn read_available(
                &mut self,
                buffer: &mut [u8],
                _io_cancel: &AtomicBool,
            ) -> std::io::Result<Option<usize>> {
                match self.read(buffer) {
                    Ok(read) => Ok(Some(read)),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => Ok(None),
                    Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(Some(0)),
                    Err(error) => Err(error),
                }
            }
        }
    };
}

#[cfg(windows)]
fn read_pipe_cancellable(
    pipe: &impl std::os::windows::io::AsRawHandle,
    buffer: &mut [u8],
    io_cancel: &AtomicBool,
) -> std::io::Result<Option<usize>> {
    use windows_sys::Win32::Foundation::{
        ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::Storage::FileSystem::ReadFile;
    use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResultEx, OVERLAPPED};

    // Rust's std::process creates the parent ChildStdout/ChildStderr ends as
    // overlapped handles (std/sys/process/windows/child_pipe.rs::anon_pipe).
    let handle = pipe.as_raw_handle() as windows_sys::Win32::Foundation::HANDLE;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let started = unsafe {
        ReadFile(
            handle,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            std::ptr::null_mut(),
            &mut overlapped,
        )
    };
    if started == 0 {
        let error = std::io::Error::last_os_error();
        match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_BROKEN_PIPE) => return Ok(Some(0)),
            Some(ERROR_IO_PENDING) => {}
            _ => return Err(error),
        }
    }

    loop {
        let mut read = 0_u32;
        let completed = unsafe { GetOverlappedResultEx(handle, &overlapped, &mut read, 5, 0) };
        if completed != 0 {
            return Ok(Some(read as usize));
        }

        let error = std::io::Error::last_os_error();
        match error.raw_os_error().map(|code| code as u32) {
            Some(ERROR_BROKEN_PIPE) => return Ok(Some(0)),
            Some(WAIT_TIMEOUT) if !io_cancel.load(Ordering::Acquire) => continue,
            Some(ERROR_OPERATION_ABORTED) if io_cancel.load(Ordering::Acquire) => return Ok(None),
            Some(WAIT_TIMEOUT) => {
                let cancelled = unsafe { CancelIoEx(handle, &overlapped) };
                let cancel_error = if cancelled == 0 {
                    let error = std::io::Error::last_os_error();
                    (error.raw_os_error().map(|code| code as u32) != Some(ERROR_NOT_FOUND))
                        .then_some(error)
                } else {
                    None
                };

                // CancelIoEx only requests cancellation. Even when it fails, the read may
                // still be pending, so always observe terminal completion before returning;
                // the OVERLAPPED and caller buffer must remain alive until then.
                let mut final_read = 0_u32;
                let final_result = unsafe {
                    GetOverlappedResultEx(handle, &overlapped, &mut final_read, u32::MAX, 0)
                };
                if final_result != 0 {
                    return Ok(Some(final_read as usize));
                }
                let final_error = std::io::Error::last_os_error();
                return match final_error.raw_os_error().map(|code| code as u32) {
                    Some(ERROR_OPERATION_ABORTED) => Ok(None),
                    Some(ERROR_BROKEN_PIPE) => Ok(Some(0)),
                    _ => Err(cancel_error.unwrap_or(final_error)),
                };
            }
            _ => return Err(error),
        }
    }
}

#[cfg(windows)]
macro_rules! impl_process_pipe {
    ($pipe:ty) => {
        impl ProcessPipe for $pipe {
            fn prepare(&self) -> std::io::Result<()> {
                Ok(())
            }

            fn read_available(
                &mut self,
                buffer: &mut [u8],
                io_cancel: &AtomicBool,
            ) -> std::io::Result<Option<usize>> {
                read_pipe_cancellable(self, buffer, io_cancel)
            }
        }
    };
}

impl_process_pipe!(ChildStdout);
impl_process_pipe!(ChildStderr);

enum StdinMode {
    Null,
    Inherit,
    File(PathBuf),
    Data(Vec<u8>),
}

enum OutputMode {
    Capture,
    Inherit,
    Null,
    File { path: PathBuf, append: bool },
}

struct ProcessOptions {
    cwd: Option<PathBuf>,
    env: HashMap<String, String>,
    clear_env: bool,
    stdin: StdinMode,
    stdout: OutputMode,
    stderr: OutputMode,
    timeout: Option<Duration>,
    termination_grace: Duration,
    max_output_bytes: usize,
}

impl ProcessOptions {
    fn run_defaults() -> Self {
        Self {
            cwd: None,
            env: HashMap::new(),
            clear_env: false,
            stdin: StdinMode::Null,
            stdout: OutputMode::Capture,
            stderr: OutputMode::Capture,
            timeout: None,
            termination_grace: Duration::from_millis(DEFAULT_TERMINATION_GRACE_MS),
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
        }
    }

    fn start_defaults() -> Self {
        Self {
            stdout: OutputMode::Inherit,
            stderr: OutputMode::Inherit,
            ..Self::run_defaults()
        }
    }
}

#[cfg(target_os = "macos")]
struct MacExitObserver {
    queue: std::os::fd::OwnedFd,
    exited: AtomicBool,
}

#[cfg(target_os = "macos")]
impl MacExitObserver {
    fn new(child: &Child) -> std::io::Result<Self> {
        use std::os::fd::{AsRawFd, FromRawFd};

        let raw_queue = unsafe { libc::kqueue() };
        if raw_queue == -1 {
            return Err(std::io::Error::last_os_error());
        }
        let queue = unsafe { std::os::fd::OwnedFd::from_raw_fd(raw_queue) };
        // EVFILT_PROC reports an already-pending NOTE_EXIT without reaping the child,
        // so registration remains safe even when a short-lived child exited first.
        let change = libc::kevent {
            ident: child.id() as libc::uintptr_t,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE,
            fflags: libc::NOTE_EXIT,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        let mut initial_event: libc::kevent = unsafe { std::mem::zeroed() };
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                &change,
                1,
                &mut initial_event,
                1,
                &timeout,
            )
        };
        if result == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if result == 1 && initial_event.flags & libc::EV_ERROR != 0 {
            return Err(std::io::Error::from_raw_os_error(initial_event.data as i32));
        }
        let exited = result == 1
            && initial_event.ident == child.id() as libc::uintptr_t
            && initial_event.fflags & libc::NOTE_EXIT != 0;
        Ok(Self {
            queue,
            exited: AtomicBool::new(exited),
        })
    }

    fn is_exited(&self, child: &Child) -> std::io::Result<bool> {
        use std::os::fd::AsRawFd;

        if self.exited.load(Ordering::Acquire) {
            return Ok(true);
        }
        let mut event: libc::kevent = unsafe { std::mem::zeroed() };
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let result = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                &timeout,
            )
        };
        if result == -1 {
            return Err(std::io::Error::last_os_error());
        }
        if result == 0 {
            return Ok(false);
        }
        if event.flags & libc::EV_ERROR != 0 {
            return Err(std::io::Error::from_raw_os_error(event.data as i32));
        }
        let exited =
            event.ident == child.id() as libc::uintptr_t && event.fflags & libc::NOTE_EXIT != 0;
        if exited {
            self.exited.store(true, Ordering::Release);
        }
        Ok(exited)
    }
}

struct ProcessEntry {
    child: Mutex<Child>,
    #[cfg(target_os = "macos")]
    exit_observer: MacExitObserver,
    stdout_reader: Mutex<Option<OutputReader>>,
    stderr_reader: Mutex<Option<OutputReader>>,
    io_cancel: Arc<AtomicBool>,
    output_error: Arc<Mutex<Option<String>>>,
    started_at: Instant,
    timeout: Option<Duration>,
    termination_grace: Duration,
    reaped: AtomicBool,
    monitor: Mutex<()>,
    terminal_at: Mutex<Option<Instant>>,
    terminal: Mutex<Option<std::result::Result<ProcessSummary, String>>>,
}

#[derive(Clone)]
struct ProcessSummary {
    success: bool,
    exit_code: Option<i32>,
    signal: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    duration_ms: i64,
}

impl ProcessSummary {
    fn into_value(self) -> Value {
        Value::Map(HashMap::from([
            ("success".to_string(), Value::Bool(self.success)),
            (
                "exit_code".to_string(),
                self.exit_code
                    .map(|code| Value::some(Value::Int(code as i64)))
                    .unwrap_or_else(Value::none),
            ),
            (
                "signal".to_string(),
                self.signal
                    .map(|signal| Value::some(Value::Int(signal as i64)))
                    .unwrap_or_else(Value::none),
            ),
            ("stdout".to_string(), Value::String(self.stdout)),
            ("stderr".to_string(), Value::String(self.stderr)),
            ("timed_out".to_string(), Value::Bool(self.timed_out)),
            ("duration_ms".to_string(), Value::Int(self.duration_ms)),
        ]))
    }
}

pub struct ProcessRuntime {
    next_id: AtomicU64,
    shutting_down: AtomicBool,
    entries: Mutex<HashMap<u64, Arc<ProcessEntry>>>,
}

pub static RUNTIME: LazyLock<ProcessRuntime> = LazyLock::new(ProcessRuntime::new);

fn type_error(message: impl Into<String>) -> IntentError {
    IntentError::type_error(message.into())
}

fn option_bool(options: &HashMap<String, Value>, name: &str, default: bool) -> Result<bool> {
    match options.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(type_error(format!(
            "process option '{name}' must be Bool, got {}",
            other.type_name()
        ))),
        None => Ok(default),
    }
}

fn option_nonnegative_u64(
    options: &HashMap<String, Value>,
    name: &str,
    default: Option<u64>,
) -> Result<Option<u64>> {
    match options.get(name) {
        Some(Value::Int(value)) if *value >= 0 => Ok(Some(*value as u64)),
        Some(Value::Int(_)) => Err(type_error(format!(
            "process option '{name}' must be non-negative"
        ))),
        Some(other) => Err(type_error(format!(
            "process option '{name}' must be Int, got {}",
            other.type_name()
        ))),
        None => Ok(default),
    }
}

fn option_string(options: &HashMap<String, Value>, name: &str) -> Result<Option<String>> {
    match options.get(name) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(other) => Err(type_error(format!(
            "process option '{name}' must be String, got {}",
            other.type_name()
        ))),
        None => Ok(None),
    }
}

fn parse_bytes(value: &Value, context: &str) -> Result<Vec<u8>> {
    let Value::Array(values) = value else {
        return Err(type_error(format!("{context} must be an Array<Int>")));
    };
    values
        .iter()
        .map(|value| match value {
            Value::Int(byte) if (0..=255).contains(byte) => Ok(*byte as u8),
            _ => Err(type_error(format!(
                "{context} must contain only byte integers from 0 through 255"
            ))),
        })
        .collect()
}

fn parse_stdin(value: Option<&Value>) -> Result<StdinMode> {
    let Some(value) = value else {
        return Ok(StdinMode::Null);
    };
    let Value::Map(options) = value else {
        return Err(type_error("process stdin option must be a Map"));
    };
    let mode = option_string(options, "mode")?
        .ok_or_else(|| type_error("process stdin option requires a 'mode'"))?;
    match mode.as_str() {
        "null" => Ok(StdinMode::Null),
        "inherit" => Ok(StdinMode::Inherit),
        "file" => option_string(options, "path")?
            .map(PathBuf::from)
            .map(StdinMode::File)
            .ok_or_else(|| type_error("process file stdin requires a 'path'")),
        "string" => option_string(options, "data")?
            .map(|data| StdinMode::Data(data.into_bytes()))
            .ok_or_else(|| type_error("process string stdin requires 'data'")),
        "bytes" => options
            .get("data")
            .ok_or_else(|| type_error("process bytes stdin requires 'data'"))
            .and_then(|data| parse_bytes(data, "process bytes stdin data"))
            .map(StdinMode::Data),
        _ => Err(type_error(format!(
            "unsupported process stdin mode '{mode}'"
        ))),
    }
}

fn parse_output(value: Option<&Value>, default: OutputMode, stream: &str) -> Result<OutputMode> {
    let Some(value) = value else {
        return Ok(default);
    };
    let Value::Map(options) = value else {
        return Err(type_error(format!("process {stream} option must be a Map")));
    };
    let mode = option_string(options, "mode")?
        .ok_or_else(|| type_error(format!("process {stream} option requires a 'mode'")))?;
    match mode.as_str() {
        "capture" => Ok(OutputMode::Capture),
        "inherit" => Ok(OutputMode::Inherit),
        "null" => Ok(OutputMode::Null),
        "file" => {
            let path = option_string(options, "path")?
                .ok_or_else(|| type_error(format!("process file {stream} requires a 'path'")))?;
            Ok(OutputMode::File {
                path: PathBuf::from(path),
                append: option_bool(options, "append", false)?,
            })
        }
        _ => Err(type_error(format!(
            "unsupported process {stream} mode '{mode}'"
        ))),
    }
}

fn parse_options(value: Option<&Value>, mut parsed: ProcessOptions) -> Result<ProcessOptions> {
    let Some(value) = value else {
        return Ok(parsed);
    };
    let Value::Map(options) = value else {
        return Err(type_error("process options must be a Map"));
    };
    let allowed: HashSet<&str> = [
        "cwd",
        "env",
        "clear_env",
        "stdin",
        "stdout",
        "stderr",
        "timeout_ms",
        "termination_grace_ms",
        "max_output_bytes",
    ]
    .into_iter()
    .collect();
    if let Some(unknown) = options.keys().find(|key| !allowed.contains(key.as_str())) {
        return Err(type_error(format!("unknown process option '{unknown}'")));
    }

    parsed.cwd = option_string(options, "cwd")?.map(PathBuf::from);
    parsed.clear_env = option_bool(options, "clear_env", false)?;
    parsed.stdin = match options.get("stdin") {
        Some(value) => parse_stdin(Some(value))?,
        None => parsed.stdin,
    };
    parsed.stdout = parse_output(options.get("stdout"), parsed.stdout, "stdout")?;
    parsed.stderr = parse_output(options.get("stderr"), parsed.stderr, "stderr")?;
    parsed.timeout =
        option_nonnegative_u64(options, "timeout_ms", None)?.map(Duration::from_millis);
    parsed.termination_grace = Duration::from_millis(
        option_nonnegative_u64(
            options,
            "termination_grace_ms",
            Some(DEFAULT_TERMINATION_GRACE_MS),
        )?
        .expect("termination grace has a default"),
    );
    let max_output = option_nonnegative_u64(
        options,
        "max_output_bytes",
        Some(DEFAULT_MAX_OUTPUT_BYTES as u64),
    )?
    .expect("max output has a default");
    parsed.max_output_bytes = usize::try_from(max_output)
        .map_err(|_| type_error("process max_output_bytes is too large for this platform"))?;

    if let Some(value) = options.get("env") {
        let Value::Map(environment) = value else {
            return Err(type_error("process option 'env' must be a Map"));
        };
        for (name, value) in environment {
            let value = match value {
                Value::String(value) => value.clone(),
                Value::Secret(secret) => secret.expose().to_string(),
                other => {
                    return Err(type_error(format!(
                        "process environment value '{name}' must be String or Secret, got {}",
                        other.type_name()
                    )))
                }
            };
            parsed.env.insert(name.clone(), value);
        }
    }
    Ok(parsed)
}

fn parse_command(
    args: &[Value],
    defaults: ProcessOptions,
) -> Result<(String, Vec<String>, ProcessOptions)> {
    let Value::String(program) = &args[0] else {
        return Err(type_error("process program must be a String"));
    };
    let Value::Array(arguments) = &args[1] else {
        return Err(type_error("process arguments must be an Array<String>"));
    };
    let arguments = arguments
        .iter()
        .map(|argument| match argument {
            Value::String(argument) => Ok(argument.clone()),
            other => Err(type_error(format!(
                "process arguments must be Strings, got {}",
                other.type_name()
            ))),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok((
        program.clone(),
        arguments,
        parse_options(args.get(2), defaults)?,
    ))
}

fn resolve_program(program: &str) -> std::result::Result<PathBuf, String> {
    let path = Path::new(program);
    if path.is_absolute() || path.components().count() > 1 {
        return std::fs::canonicalize(path)
            .map_err(|error| format!("cannot resolve process executable '{program}': {error}"));
    }

    let search_path = std::env::var_os("PATH")
        .ok_or_else(|| format!("cannot resolve process executable '{program}': PATH is not set"))?;
    for directory in std::env::split_paths(&search_path) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return std::fs::canonicalize(&candidate).map_err(|error| {
                format!("cannot canonicalize process executable '{program}': {error}")
            });
        }
        #[cfg(windows)]
        for extension in ["exe", "com"] {
            let candidate = directory.join(format!("{program}.{extension}"));
            if candidate.is_file() {
                return std::fs::canonicalize(&candidate).map_err(|error| {
                    format!("cannot canonicalize process executable '{program}': {error}")
                });
            }
        }
    }
    Err(format!("cannot resolve process executable '{program}'"))
}

fn authorize_program(program: &str) -> std::result::Result<PathBuf, String> {
    let enabled = std::env::var("NTNT_PROCESS_ENABLE")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        });
    if !enabled {
        return Err("process execution is disabled; set NTNT_PROCESS_ENABLE=1".to_string());
    }
    let resolved = resolve_program(program)?;
    validate_windows_program(&resolved)?;
    let Some(allowlist) = std::env::var_os("NTNT_PROCESS_ALLOW") else {
        return Ok(resolved);
    };
    let allowed = std::env::split_paths(&allowlist)
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .any(|path| path == resolved);
    if !allowed {
        return Err(format!(
            "process executable is not allowed: {}",
            resolved.display()
        ));
    }
    Ok(resolved)
}

#[cfg(windows)]
fn validate_windows_program(program: &Path) -> std::result::Result<(), String> {
    let extension = program
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd") {
        return Err(format!(
            "Windows batch scripts are not direct executables: {}; invoke an explicitly allowlisted cmd.exe only when shell authority is intentional",
            program.display()
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn validate_windows_program(_program: &Path) -> std::result::Result<(), String> {
    Ok(())
}

fn output_stdio(mode: &OutputMode) -> std::result::Result<Stdio, String> {
    match mode {
        OutputMode::Capture => Ok(Stdio::piped()),
        OutputMode::Inherit => Ok(Stdio::inherit()),
        OutputMode::Null => Ok(Stdio::null()),
        OutputMode::File { path, append } => {
            let file = OpenOptions::new()
                .create(true)
                .write(true)
                .append(*append)
                .truncate(!append)
                .open(path)
                .map_err(|error| {
                    format!("cannot open process output '{}': {error}", path.display())
                })?;
            Ok(Stdio::from(file))
        }
    }
}

fn input_stdio(mode: &StdinMode) -> std::result::Result<Stdio, String> {
    match mode {
        StdinMode::Null => Ok(Stdio::null()),
        StdinMode::Inherit => Ok(Stdio::inherit()),
        StdinMode::File(path) => File::open(path)
            .map(Stdio::from)
            .map_err(|error| format!("cannot open process input '{}': {error}", path.display())),
        StdinMode::Data(data) => {
            let mut file = tempfile::tempfile()
                .map_err(|error| format!("cannot create process stdin buffer: {error}"))?;
            file.write_all(data)
                .map_err(|error| format!("cannot write process stdin buffer: {error}"))?;
            file.seek(SeekFrom::Start(0))
                .map_err(|error| format!("cannot rewind process stdin buffer: {error}"))?;
            Ok(Stdio::from(file))
        }
    }
}

fn spawn_reader<R: ProcessPipe>(
    mut reader: R,
    stream: &'static str,
    max_bytes: usize,
    io_cancel: Arc<AtomicBool>,
    output_error: Arc<Mutex<Option<String>>>,
) -> OutputReader {
    std::thread::spawn(move || {
        #[cfg(test)]
        let _active_reader = {
            ACTIVE_OUTPUT_READERS.fetch_add(1, Ordering::AcqRel);
            ActiveOutputReader
        };
        let mut output = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            if io_cancel.load(Ordering::Acquire) {
                return Ok(output);
            }
            let read = match reader.read_available(&mut buffer, &io_cancel) {
                Ok(Some(read)) => read,
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => {
                    return Err(format!("failed reading process {stream}: {error}"));
                }
            };
            if read == 0 {
                return Ok(output);
            }
            if output.len().saturating_add(read) > max_bytes {
                let message =
                    format!("process {stream} exceeded max_output_bytes limit of {max_bytes}");
                *output_error
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(message.clone());
                return Err(message);
            }
            output.extend_from_slice(&buffer[..read]);
        }
    })
}

fn spawn_process(
    executable: &Path,
    arguments: &[String],
    options: ProcessOptions,
) -> std::result::Result<Arc<ProcessEntry>, String> {
    let mut command = Command::new(executable);
    command.args(arguments);
    if let Some(cwd) = &options.cwd {
        command.current_dir(cwd);
    }
    if options.clear_env {
        command.env_clear();
    }
    command.envs(&options.env);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    command.stdout(output_stdio(&options.stdout)?);
    command.stderr(output_stdio(&options.stderr)?);
    command.stdin(input_stdio(&options.stdin)?);

    let started_at = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        format!(
            "failed to launch process '{}': {error}",
            executable.display()
        )
    })?;
    #[cfg(target_os = "macos")]
    let exit_observer = match MacExitObserver::new(&child) {
        Ok(observer) => observer,
        Err(error) => {
            let _ = kill_child(&mut child);
            let _ = child.wait();
            return Err(format!("failed to configure process monitoring: {error}"));
        }
    };
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let capture_setup = stdout
        .as_ref()
        .map(ProcessPipe::prepare)
        .transpose()
        .and_then(|_| stderr.as_ref().map(ProcessPipe::prepare).transpose());
    if let Err(error) = capture_setup {
        let _ = kill_child(&mut child);
        let _ = child.wait();
        return Err(format!("failed to configure process capture: {error}"));
    }

    let io_cancel = Arc::new(AtomicBool::new(false));
    let output_error = Arc::new(Mutex::new(None));
    let stdout_reader = stdout.map(|stdout| {
        spawn_reader(
            stdout,
            "stdout",
            options.max_output_bytes,
            Arc::clone(&io_cancel),
            Arc::clone(&output_error),
        )
    });
    let stderr_reader = stderr.map(|stderr| {
        spawn_reader(
            stderr,
            "stderr",
            options.max_output_bytes,
            Arc::clone(&io_cancel),
            Arc::clone(&output_error),
        )
    });
    let timeout = options.timeout;
    let termination_grace = options.termination_grace;
    Ok(Arc::new(ProcessEntry {
        child: Mutex::new(child),
        #[cfg(target_os = "macos")]
        exit_observer,
        stdout_reader: Mutex::new(stdout_reader),
        stderr_reader: Mutex::new(stderr_reader),
        io_cancel,
        output_error,
        started_at,
        timeout,
        termination_grace,
        reaped: AtomicBool::new(false),
        monitor: Mutex::new(()),
        terminal_at: Mutex::new(None),
        terminal: Mutex::new(None),
    }))
}

#[cfg(unix)]
fn terminate_child(child: &mut Child) -> std::io::Result<()> {
    signal_process_group(child, libc::SIGTERM)
}

#[cfg(unix)]
fn kill_child(child: &mut Child) -> std::io::Result<()> {
    signal_process_group(child, libc::SIGKILL)
}

#[cfg(unix)]
fn signal_process_group(child: &Child, signal: libc::c_int) -> std::io::Result<()> {
    let result = unsafe { libc::kill(-(child.id() as libc::pid_t), signal) };
    if result == 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(target_os = "macos")]
fn macos_process_is_live(pid: libc::pid_t) -> std::io::Result<bool> {
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let bytes = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            (&mut info as *mut libc::proc_bsdinfo).cast(),
            std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int,
        )
    };
    if bytes as usize == std::mem::size_of::<libc::proc_bsdinfo>() {
        return Ok(info.pbi_status != libc::SZOMB);
    }
    if unsafe { libc::kill(pid, 0) } == -1
        && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
    {
        return Ok(false);
    }
    Err(std::io::Error::other(format!(
        "failed to inspect macOS process {pid}"
    )))
}

#[cfg(target_os = "macos")]
fn exited_group_has_live_descendants(child: &Child) -> std::io::Result<bool> {
    const MAX_INSPECTED_GROUP_PROCESSES: usize = 65_536;

    let leader = child.id() as libc::pid_t;
    let capacity = unsafe { libc::proc_listpgrppids(leader, std::ptr::null_mut(), 0) };
    if capacity <= 0 || capacity as usize > MAX_INSPECTED_GROUP_PROCESSES {
        return Err(std::io::Error::other(
            "failed to size exited macOS process group",
        ));
    }
    let mut pids = vec![0 as libc::pid_t; capacity as usize];
    let listed = unsafe {
        libc::proc_listpgrppids(
            leader,
            pids.as_mut_ptr().cast(),
            std::mem::size_of_val(pids.as_slice()) as libc::c_int,
        )
    };
    if listed <= 0 || listed as usize > pids.len() {
        return Err(std::io::Error::other(
            "failed to inspect exited macOS process group",
        ));
    }
    for pid in pids.into_iter().take(listed as usize) {
        if pid != 0 && pid != leader && macos_process_is_live(pid)? {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(target_os = "macos")]
fn clean_up_exited_process_group(child: &Child) -> std::io::Result<()> {
    match signal_process_group(child, libc::SIGKILL) {
        Err(error)
            if error.raw_os_error() == Some(libc::EPERM)
                && !exited_group_has_live_descendants(child)? =>
        {
            // Darwin reports EPERM when an exited group contains no live members.
            Ok(())
        }
        result => result,
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn clean_up_exited_process_group(child: &Child) -> std::io::Result<()> {
    signal_process_group(child, libc::SIGKILL)
}

#[cfg(windows)]
fn terminate_child(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

#[cfg(windows)]
fn kill_child(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn child_exit_is_pending(_process: &ProcessEntry, child: &Child) -> std::io::Result<bool> {
    loop {
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                child.id() as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == 0 {
            return Ok(info.si_signo != 0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(target_os = "macos")]
fn child_exit_is_pending(process: &ProcessEntry, child: &Child) -> std::io::Result<bool> {
    process.exit_observer.is_exited(child)
}

#[cfg(unix)]
fn reap_if_exited(
    process: &ProcessEntry,
    child: &mut Child,
) -> std::result::Result<Option<ExitStatus>, String> {
    if !child_exit_is_pending(process, child)
        .map_err(|error| format!("failed to monitor process: {error}"))?
    {
        return Ok(None);
    }

    // Keep the exited leader unreaped until its process group is signalled. A zombie
    // still owns its PID, so the negative ID cannot be redirected to a reused PID.
    clean_up_exited_process_group(child)
        .map_err(|error| format!("failed to clean up process descendants: {error}"))?;
    child
        .wait()
        .map(Some)
        .map_err(|error| format!("failed to reap process: {error}"))
}

#[cfg(windows)]
fn reap_if_exited(
    _process: &ProcessEntry,
    child: &mut Child,
) -> std::result::Result<Option<ExitStatus>, String> {
    child
        .try_wait()
        .map_err(|error| format!("failed to monitor process: {error}"))
}

#[cfg(unix)]
fn child_has_exited(
    process: &ProcessEntry,
    child: &mut Child,
) -> std::result::Result<bool, String> {
    child_exit_is_pending(process, child)
        .map_err(|error| format!("failed to monitor process: {error}"))
}

#[cfg(windows)]
fn child_has_exited(
    _process: &ProcessEntry,
    child: &mut Child,
) -> std::result::Result<bool, String> {
    child
        .try_wait()
        .map(|status| status.is_some())
        .map_err(|error| format!("failed to monitor process: {error}"))
}

fn cached_reaped_status(
    process: &ProcessEntry,
    child: &mut Child,
) -> std::result::Result<Option<ExitStatus>, String> {
    if !process.reaped.load(Ordering::Acquire) {
        return Ok(None);
    }
    child
        .wait()
        .map(Some)
        .map_err(|error| format!("failed to read reaped process status: {error}"))
}

fn try_status(process: &ProcessEntry) -> std::result::Result<Option<ExitStatus>, String> {
    let mut child = process
        .child
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(status) = cached_reaped_status(process, &mut child)? {
        return Ok(Some(status));
    }
    let status = reap_if_exited(process, &mut child)?;
    if status.is_some() {
        process.reaped.store(true, Ordering::Release);
    }
    Ok(status)
}

fn stop_and_reap(process: &ProcessEntry) -> std::result::Result<ExitStatus, String> {
    {
        let mut child = process
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(status) = cached_reaped_status(process, &mut child)? {
            return Ok(status);
        }
        if let Some(status) = reap_if_exited(process, &mut child)? {
            process.reaped.store(true, Ordering::Release);
            return Ok(status);
        }
        terminate_child(&mut child)
            .map_err(|error| format!("failed to terminate process: {error}"))?;
    }
    let deadline = Instant::now() + process.termination_grace;
    loop {
        if let Some(status) = try_status(process)? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let mut child = process
                .child
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // Another monitor can reap between the preceding status probe and this lock.
            // Reuse its cached status before signalling any numeric process-group ID.
            if let Some(status) = cached_reaped_status(process, &mut child)? {
                return Ok(status);
            }
            kill_child(&mut child).map_err(|error| format!("failed to kill process: {error}"))?;
            let status = child
                .wait()
                .map_err(|error| format!("failed to reap process: {error}"))?;
            process.reaped.store(true, Ordering::Release);
            return Ok(status);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn join_reader(reader: Option<OutputReader>, stream: &str) -> std::result::Result<Vec<u8>, String> {
    match reader {
        Some(reader) => reader
            .join()
            .map_err(|_| format!("process {stream} reader panicked"))?,
        None => Ok(Vec::new()),
    }
}

fn exit_signal(status: &ExitStatus) -> Option<i32> {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status.signal()
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        None
    }
}

fn result_map(
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
    duration: Duration,
) -> ProcessSummary {
    ProcessSummary {
        success: status.success() && !timed_out,
        exit_code: status.code(),
        signal: exit_signal(&status),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
        timed_out,
        duration_ms: duration.as_millis().min(i64::MAX as u128) as i64,
    }
}

fn finalize_process(
    process: &ProcessEntry,
    status: ExitStatus,
    timed_out: bool,
) -> std::result::Result<ProcessSummary, String> {
    let stdout_reader = process
        .stdout_reader
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    let stderr_reader = process
        .stderr_reader
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();

    let deadline = Instant::now() + Duration::from_millis(IO_THREAD_DRAIN_GRACE_MS);
    while stdout_reader
        .as_ref()
        .is_some_and(|reader| !reader.is_finished())
        || stderr_reader
            .as_ref()
            .is_some_and(|reader| !reader.is_finished())
    {
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let stdout_lingering = stdout_reader
        .as_ref()
        .is_some_and(|reader| !reader.is_finished());
    let stderr_lingering = stderr_reader
        .as_ref()
        .is_some_and(|reader| !reader.is_finished());
    if stdout_lingering || stderr_lingering {
        process.io_cancel.store(true, Ordering::Release);
    }

    let stdout = join_reader(stdout_reader, "stdout")?;
    let stderr = join_reader(stderr_reader, "stderr")?;
    if stdout_lingering {
        return Err("process stdout remained open after the child exited".to_string());
    }
    if stderr_lingering {
        return Err("process stderr remained open after the child exited".to_string());
    }
    Ok(result_map(
        status,
        stdout,
        stderr,
        timed_out,
        process.started_at.elapsed(),
    ))
}

fn monitor_process_uncached(
    process: &ProcessEntry,
    block: bool,
) -> std::result::Result<Option<ProcessSummary>, String> {
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        let output_error = process
            .output_error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        if let Some(error) = output_error {
            return match stop_and_reap(process) {
                Ok(status) => {
                    let _ = finalize_process(process, status, false);
                    Err(error)
                }
                Err(cleanup_error) => Err(format!("{error}; cleanup failed: {cleanup_error}")),
            };
        }
        if crate::stdlib::concurrent::is_current_task_cancelled() {
            cancelled = true;
            break stop_and_reap(process)?;
        }
        if process
            .timeout
            .is_some_and(|timeout| process.started_at.elapsed() >= timeout)
        {
            timed_out = true;
            break stop_and_reap(process)?;
        }
        if let Some(status) = try_status(process)? {
            break status;
        }
        if !block {
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    let finalized = finalize_process(process, status, timed_out);
    if cancelled {
        match finalized {
            Ok(_) => Err("process cancelled".to_string()),
            Err(error) => Err(format!("process cancelled; cleanup failed: {error}")),
        }
    } else {
        finalized.map(Some)
    }
}

fn monitor_process(
    process: &ProcessEntry,
    block: bool,
) -> std::result::Result<Option<ProcessSummary>, String> {
    let _monitor = process
        .monitor
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if let Some(result) = process
        .terminal
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
    {
        return result.map(Some);
    }

    let result = match monitor_process_uncached(process, block) {
        Ok(None) => return Ok(None),
        Ok(Some(summary)) => {
            *process
                .terminal_at
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(Instant::now());
            *process
                .terminal
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(Ok(summary.clone()));
            Ok(Some(summary))
        }
        Err(error) => {
            *process
                .terminal_at
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(Instant::now());
            *process
                .terminal
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(Err(error.clone()));
            Err(error)
        }
    };
    RUNTIME.prune_terminal_entries();
    result
}

fn supervise_process(process: Arc<ProcessEntry>) {
    std::thread::spawn(move || loop {
        match monitor_process(&process, false) {
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Ok(Some(_)) | Err(_) => return,
        }
    });
}

/// Launch only the native test child protocol using the existing supervisor.
/// This Rust-only entry does not authorize any std/process operation in .tnt.
pub(crate) fn run_native_test_child(
    executable: &Path,
    cwd: &Path,
    request_path: &Path,
    response_path: &Path,
) -> std::result::Result<(), String> {
    let options = ProcessOptions {
        cwd: Some(cwd.to_path_buf()),
        clear_env: true,
        env: native_test_launch_env(),
        timeout: Some(Duration::from_secs(30)),
        max_output_bytes: 512 * 1024, // one MiB total across the two captured streams
        ..ProcessOptions::run_defaults()
    };
    let arguments = vec![
        "__native-test".to_string(),
        request_path
            .to_str()
            .ok_or("Native request path is not UTF-8")?
            .to_string(),
        response_path
            .to_str()
            .ok_or("Native response path is not UTF-8")?
            .to_string(),
    ];
    let process = spawn_process(executable, &arguments, options)?;
    let id = register_process(&process)?;
    let result = monitor_process(&process, true);
    // The normal monitor already kills/reaps on timeout and output failure.
    // If OS observation itself failed, make a final cleanup attempt before
    // releasing the fixture owner; retain every failure as non-green.
    let result = match result {
        Err(error) => match stop_and_reap(&process)
            .and_then(|status| finalize_process(&process, status, false).map(|_| ()))
        {
            Ok(()) => Err(error),
            Err(cleanup) => Err(format!("{error}; cleanup failed: {cleanup}")),
        },
        Ok(Some(summary)) if summary.timed_out => {
            Err("Native test child timed out after 30 seconds".into())
        }
        Ok(Some(summary)) if !summary.success => Err(format!(
            "Native test child failed (exit {:?}, signal {:?}): {}",
            summary.exit_code, summary.signal, summary.stderr
        )),
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err("Native child monitor unexpectedly returned pending".into()),
    };
    RUNTIME.remove(id);
    result
}

fn native_test_launch_env() -> HashMap<String, String> {
    #[cfg(windows)]
    {
        // No HOME, PATH, application credentials, dotenv or capability flags.
        ["SystemRoot", "WINDIR"]
            .into_iter()
            .filter_map(|name| {
                std::env::var(name)
                    .ok()
                    .map(|value| (name.to_string(), value))
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        HashMap::new()
    }
}

fn run_from_args(args: &[Value]) -> Result<Value> {
    let (program, arguments, options) = parse_command(args, ProcessOptions::run_defaults())?;
    if crate::stdlib::concurrent::is_current_task_cancelled() {
        return Ok(Value::err(Value::String("process cancelled".to_string())));
    }
    let executable = match authorize_program(&program) {
        Ok(executable) => executable,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    let process = match spawn_process(&executable, &arguments, options) {
        Ok(process) => process,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    let process_id = match register_process(&process) {
        Ok(process_id) => process_id,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    let result = monitor_process(&process, true);
    RUNTIME.remove(process_id);
    match result {
        Ok(Some(result)) => Ok(Value::ok(result.into_value())),
        Ok(None) => unreachable!("blocking process monitor returned pending"),
        Err(error) => Ok(Value::err(Value::String(error))),
    }
}

impl ProcessRuntime {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            shutting_down: AtomicBool::new(false),
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn insert(&self, process: Arc<ProcessEntry>) -> std::result::Result<u64, String> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("process runtime is shutting down".to_string());
        }
        let id = self.next_id.fetch_add(1, Ordering::AcqRel);
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.shutting_down.load(Ordering::Acquire) {
            return Err("process runtime is shutting down".to_string());
        }
        entries.insert(id, process);
        Ok(id)
    }

    fn get(&self, id: u64) -> Option<Arc<ProcessEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&id)
            .cloned()
    }

    fn remove(&self, id: u64) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(&id);
    }

    fn prune_terminal_entries(&self) {
        self.prune_terminal_entries_to(
            MAX_RETAINED_TERMINAL_PROCESSES,
            MAX_RETAINED_TERMINAL_BYTES,
        );
    }

    fn prune_terminal_entries_to(&self, max_count: usize, max_bytes: usize) {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut terminal = entries
            .iter()
            .filter_map(|(id, process)| {
                let retained_bytes = process
                    .terminal
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .map(|result| {
                        result.as_ref().map_or(0, |summary| {
                            summary.stdout.len().saturating_add(summary.stderr.len())
                        })
                    })?;
                let terminal_at = process
                    .terminal_at
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .unwrap_or(process.started_at);
                Some((*id, terminal_at, retained_bytes))
            })
            .collect::<Vec<_>>();
        terminal.sort_unstable_by_key(|(_, completed_at, _)| *completed_at);

        let mut retained_count = terminal.len();
        let mut retained_bytes = terminal
            .iter()
            .fold(0_usize, |total, (_, _, bytes)| total.saturating_add(*bytes));
        for (id, _, bytes) in terminal {
            if retained_count <= max_count && retained_bytes <= max_bytes {
                break;
            }
            entries.remove(&id);
            retained_count -= 1;
            retained_bytes = retained_bytes.saturating_sub(bytes);
        }
    }

    pub fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        let processes = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for process in processes {
            let already_terminal = process
                .terminal
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_some();
            if already_terminal {
                continue;
            }
            let status = stop_and_reap(&process);
            let _monitor = process
                .monitor
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let mut terminal = process
                .terminal
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if terminal.is_none() {
                *process
                    .terminal_at
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = Some(Instant::now());
                *terminal =
                    Some(status.and_then(|status| finalize_process(&process, status, false)));
            }
        }
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        #[cfg(test)]
        self.shutting_down.store(false, Ordering::Release);
    }
}

fn register_process(process: &Arc<ProcessEntry>) -> std::result::Result<u64, String> {
    match RUNTIME.insert(Arc::clone(process)) {
        Ok(process_id) => Ok(process_id),
        Err(error) => {
            let cleanup = stop_and_reap(process)
                .and_then(|status| finalize_process(process, status, false).map(|_| ()));
            match cleanup {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!("{error}; cleanup failed: {cleanup_error}")),
            }
        }
    }
}

fn process_handle(value: &Value) -> Result<u64> {
    match value {
        Value::ProcessHandle(id) => Ok(*id),
        other => Err(type_error(format!(
            "expected Process handle, got {}",
            other.type_name()
        ))),
    }
}

fn registered_process(value: &Value) -> Result<std::result::Result<Arc<ProcessEntry>, String>> {
    let id = process_handle(value)?;
    Ok(RUNTIME
        .get(id)
        .ok_or_else(|| format!("unknown process handle: {id}")))
}

fn start_from_args(args: &[Value]) -> Result<Value> {
    let (program, arguments, options) = parse_command(args, ProcessOptions::start_defaults())?;
    if crate::stdlib::concurrent::is_current_task_cancelled() {
        return Ok(Value::err(Value::String("process cancelled".to_string())));
    }
    let executable = match authorize_program(&program) {
        Ok(executable) => executable,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    let process = match spawn_process(&executable, &arguments, options) {
        Ok(process) => process,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    let id = match register_process(&process) {
        Ok(process_id) => process_id,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    supervise_process(process);
    Ok(Value::ok(Value::ProcessHandle(id)))
}

fn wait_from_args(args: &[Value]) -> Result<Value> {
    let process = match registered_process(&args[0])? {
        Ok(process) => process,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    match monitor_process(&process, true) {
        Ok(Some(result)) => Ok(Value::ok(result.into_value())),
        Ok(None) => unreachable!("blocking process monitor returned pending"),
        Err(error) => Ok(Value::err(Value::String(error))),
    }
}

fn try_wait_from_args(args: &[Value]) -> Result<Value> {
    let process = match registered_process(&args[0])? {
        Ok(process) => process,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    match monitor_process(&process, false) {
        Ok(Some(result)) => Ok(Value::ok(Value::some(result.into_value()))),
        Ok(None) => Ok(Value::ok(Value::none())),
        Err(error) => Ok(Value::err(Value::String(error))),
    }
}

fn signal_from_args(args: &[Value], force: bool) -> Result<Value> {
    let process = match registered_process(&args[0])? {
        Ok(process) => process,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    if process
        .terminal
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_some()
    {
        return Ok(Value::ok(Value::Bool(false)));
    }
    let mut child = process
        .child
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if process.reaped.load(Ordering::Acquire) {
        return Ok(Value::ok(Value::Bool(false)));
    }
    match child_has_exited(&process, &mut child) {
        Ok(true) => {
            #[cfg(windows)]
            process.reaped.store(true, Ordering::Release);
            Ok(Value::ok(Value::Bool(false)))
        }
        Ok(false) => {
            let result = if force {
                kill_child(&mut child)
            } else {
                terminate_child(&mut child)
            };
            match result {
                Ok(()) => Ok(Value::ok(Value::Bool(true))),
                Err(error) => Ok(Value::err(Value::String(format!(
                    "failed to {} process: {error}",
                    if force { "kill" } else { "terminate" }
                )))),
            }
        }
        Err(error) => Ok(Value::err(Value::String(error))),
    }
}

fn terminate_from_args(args: &[Value]) -> Result<Value> {
    signal_from_args(args, false)
}

fn kill_from_args(args: &[Value]) -> Result<Value> {
    signal_from_args(args, true)
}

pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt run
    // @module std/process
    // @signature run(program: String, args: Array<String>, options?: Map) -> Result<Map, String>
    // Run a native program directly and wait for its bounded result.
    //
    // The operating-system process API receives every argument literally; no shell is
    // invoked. Execution requires NTNT_PROCESS_ENABLE=1 and respects NTNT_PROCESS_ALLOW.
    // Options: cwd (String), env (Map<String, String | Secret>), clear_env (Bool),
    // stdin/stdout/stderr mode maps, timeout_ms, termination_grace_ms, and
    // max_output_bytes. stdin modes are null, inherit, file, string, and bytes;
    // stdout/stderr modes are capture, inherit, null, and file. File modes require path,
    // output files optionally accept append, and string/bytes input requires data.
    // run() has no default timeout; set timeout_ms whenever execution must be time-bounded.
    // @param program Executable path or name resolved through PATH
    // @param args Literal arguments passed directly to the executable
    // @param options Optional cwd, env, stdio, timeout, grace, and output-limit settings
    // @returns Ok with exit, output, timeout, and duration fields; Err for capability or monitoring failures
    // @since v0.5.3
    // @tags #process #system
    // @example run("/usr/bin/ffmpeg", ["-version"]) => Ok({...}) ~ "Run without a shell"
    // @error RuntimeError ~ "process execution is disabled" fix: "Set NTNT_PROCESS_ENABLE=1 for a trusted application"
    // @error RuntimeError ~ "Windows batch scripts are not direct executables" fix: "Invoke an explicitly allowlisted cmd.exe only when shell authority is intentional"
    module.insert(
        "run".to_string(),
        Value::NativeFunction {
            name: "run".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: run_from_args,
        },
    );

    // @ntnt start
    // @module std/process
    // @signature start(program: String, args: Array<String>, options?: Map) -> Result<Process, String>
    // Start a supervised native process and return an opaque handle.
    // The runtime monitors exit, timeout, and captured-output limits without caller polling.
    // On Unix, the child leads a process group so timeout and shutdown also terminate
    // descendants. start() accepts the same options as run(), but stdout/stderr default to
    // inherit instead of capture.
    // @param program Executable path or name resolved through PATH
    // @param args Literal arguments passed directly to the executable
    // @param options Optional cwd, env, stdio, timeout, grace, and output-limit settings
    // @returns Ok(Process) or an execution/capability error
    // @example start("mlx_audio.server", []) => Ok(Process(1)) ~ "Start a supervised service"
    // @gotcha start defaults stdout and stderr to inherit; select capture only when output is bounded and will be collected
    // @gotcha Completed handles share a 64-entry, 64-MiB terminal-result cache; persist needed results instead of retaining handles indefinitely
    // @see_also run, wait, try_wait, terminate, kill
    // @since v0.5.3
    // @tags #process #system
    module.insert(
        "start".to_string(),
        Value::NativeFunction {
            name: "start".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: start_from_args,
        },
    );

    // @ntnt wait
    // @module std/process
    // @signature wait(process: Process) -> Result<Map, String>
    // Wait for a supervised process and return its cached final result. The runtime
    // retains at most 64 terminal handles and 64 MiB of terminal output; older
    // handles return an invalid-handle error after eviction.
    // @param process Opaque handle returned by start
    // @returns Ok with the final process result or Err for an invalid handle
    // @example wait(process) => Ok({success: true, exit_code: Some(0), ...}) ~ "Wait and cache the final result"
    // @since v0.5.3
    // @tags #process #system
    module.insert(
        "wait".to_string(),
        Value::NativeFunction {
            name: "wait".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: wait_from_args,
        },
    );

    // @ntnt try_wait
    // @module std/process
    // @signature try_wait(process: Process) -> Result<Option<Map>, String>
    // Inspect a supervised process without blocking.
    // @param process Opaque handle returned by start
    // @returns Ok(None) while running, Ok(Some(result)) after exit, or Err for an invalid or evicted handle
    // @example try_wait(process) => Ok(None) ~ "Poll without blocking"
    // @since v0.5.3
    // @tags #process #system
    module.insert(
        "try_wait".to_string(),
        Value::NativeFunction {
            name: "try_wait".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: try_wait_from_args,
        },
    );

    // @ntnt terminate
    // @module std/process
    // @signature terminate(process: Process) -> Result<Bool, String>
    // Request graceful termination of a supervised process.
    // @param process Opaque handle returned by start
    // @returns Ok(true) when requested or Ok(false) when already exited
    // @example terminate(process) => Ok(true) ~ "Request graceful termination"
    // @gotcha Unix sends SIGTERM; Windows uses the supported child termination operation
    // @since v0.5.3
    // @tags #process #system
    module.insert(
        "terminate".to_string(),
        Value::NativeFunction {
            name: "terminate".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: terminate_from_args,
        },
    );

    // @ntnt kill
    // @module std/process
    // @signature kill(process: Process) -> Result<Bool, String>
    // Force termination of a supervised process.
    // @param process Opaque handle returned by start
    // @returns Ok(true) when requested or Ok(false) when already exited
    // @example kill(process) => Ok(true) ~ "Force child termination"
    // @since v0.5.3
    // @tags #process #system
    module.insert(
        "kill".to_string(),
        Value::NativeFunction {
            name: "kill".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: kill_from_args,
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::Value;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
    static RUNTIME_TEST_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    fn current_test_command(fixture: &str, trailing: &[&str]) -> (String, Vec<Value>) {
        let executable = std::env::current_exe().unwrap();
        let mut args = vec![
            Value::String("--exact".to_string()),
            Value::String(format!("stdlib::process::tests::{fixture}")),
            Value::String("--ignored".to_string()),
            Value::String("--nocapture".to_string()),
            Value::String("--".to_string()),
        ];
        args.extend(trailing.iter().map(|arg| Value::String((*arg).to_string())));
        (executable.to_string_lossy().into_owned(), args)
    }

    fn with_process_capability<T>(executable: &str, action: impl FnOnce() -> T) -> T {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_enable = std::env::var_os("NTNT_PROCESS_ENABLE");
        let old_allow = std::env::var_os("NTNT_PROCESS_ALLOW");
        std::env::set_var("NTNT_PROCESS_ENABLE", "1");
        std::env::set_var("NTNT_PROCESS_ALLOW", executable);
        let result = action();
        match old_enable {
            Some(value) => std::env::set_var("NTNT_PROCESS_ENABLE", value),
            None => std::env::remove_var("NTNT_PROCESS_ENABLE"),
        }
        match old_allow {
            Some(value) => std::env::set_var("NTNT_PROCESS_ALLOW", value),
            None => std::env::remove_var("NTNT_PROCESS_ALLOW"),
        }
        result
    }

    fn result_variant(value: Value) -> (String, Value) {
        let Value::EnumValue {
            enum_name,
            variant,
            mut values,
        } = value
        else {
            panic!("expected Result value");
        };
        assert_eq!(enum_name, "Result");
        (variant, values.remove(0))
    }

    fn run_args(program: String, args: Vec<Value>) -> Vec<Value> {
        vec![Value::String(program), Value::Array(args)]
    }

    fn run_args_with_options(
        program: String,
        args: Vec<Value>,
        options: HashMap<String, Value>,
    ) -> Vec<Value> {
        vec![
            Value::String(program),
            Value::Array(args),
            Value::Map(options),
        ]
    }

    #[test]
    fn run_is_disabled_without_explicit_capability() {
        let _guard = ENV_LOCK.lock().unwrap();
        let old_enable = std::env::var_os("NTNT_PROCESS_ENABLE");
        std::env::remove_var("NTNT_PROCESS_ENABLE");
        let (program, args) = current_test_command("fixture_print_args", &[]);

        let result = run_from_args(&run_args(program, args)).expect("run result");

        if let Some(value) = old_enable {
            std::env::set_var("NTNT_PROCESS_ENABLE", value);
        }
        let (variant, error) = result_variant(result);
        assert_eq!(variant, "Err");
        assert!(error.to_string().contains("NTNT_PROCESS_ENABLE=1"));
    }

    #[test]
    fn run_passes_metacharacters_as_literal_arguments() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command(
            "fixture_print_args",
            &["hello; echo injected", "$(touch nope)", "$HOME"],
        );
        let result = with_process_capability(&program, || {
            run_from_args(&run_args(program.clone(), args)).expect("run result")
        });
        let (variant, value) = result_variant(result);
        assert_eq!(variant, "Ok");
        let Value::Map(result) = value else {
            panic!("expected process result map");
        };
        assert!(matches!(result.get("success"), Some(Value::Bool(true))));
        let Some(Value::String(stdout)) = result.get("stdout") else {
            panic!("expected captured stdout");
        };
        assert!(stdout.contains("hello; echo injected"));
        assert!(stdout.contains("$(touch nope)"));
        assert!(stdout.contains("$HOME"));
    }

    #[test]
    fn run_returns_nonzero_exit_as_ok_result() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command("fixture_nonzero", &[]);
        let result = with_process_capability(&program, || {
            run_from_args(&run_args(program.clone(), args)).expect("run result")
        });
        let (variant, value) = result_variant(result);
        assert_eq!(variant, "Ok");
        let Value::Map(result) = value else {
            panic!("expected process result map");
        };
        assert!(matches!(result.get("success"), Some(Value::Bool(false))));
        assert!(
            matches!(result.get("exit_code"), Some(Value::EnumValue { variant, .. }) if variant == "Some")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_capture_observes_eof_for_quiet_child() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let command = std::env::var_os("COMSPEC").expect("Windows command interpreter path");
        let arguments = ["/D", "/C", "exit", "0"].map(str::to_string);
        let process = spawn_process(
            Path::new(&command),
            &arguments,
            ProcessOptions::run_defaults(),
        )
        .expect("spawn quiet child");

        let result = monitor_process_uncached(&process, true)
            .expect("quiet capture must observe EOF")
            .expect("blocking monitor returns a result");

        assert!(result.success);
        assert!(result.stdout.is_empty());
        assert!(result.stderr.is_empty());
        assert_eq!(ACTIVE_OUTPUT_READERS.load(Ordering::Acquire), 0);
    }

    #[cfg(windows)]
    #[test]
    fn windows_capture_cancellation_completes_pending_read() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let program = std::env::var_os("COMSPEC").expect("Windows command interpreter path");
        let arguments = ["/D", "/Q", "/C", "ping -n 6 127.0.0.1 >nul"].map(str::to_string);
        let mut options = ProcessOptions::run_defaults();
        options.stderr = OutputMode::Null;
        let process = spawn_process(Path::new(&program), &arguments, options)
            .expect("spawn pending-read cancellation fixture");
        let reader_start_deadline = Instant::now() + Duration::from_secs(1);
        while ACTIVE_OUTPUT_READERS.load(Ordering::Acquire) == 0
            && Instant::now() < reader_start_deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(ACTIVE_OUTPUT_READERS.load(Ordering::Acquire), 1);
        std::thread::sleep(Duration::from_millis(25));

        let reader = process
            .stdout_reader
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
            .expect("stdout capture reader");
        process.io_cancel.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !reader.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        let completed_before_child_exit = reader.is_finished();

        if !completed_before_child_exit {
            let mut child = process
                .child
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let _ = kill_child(&mut child);
        }
        let result = reader.join().expect("capture reader must not panic");
        assert!(
            completed_before_child_exit,
            "cancellation must complete the pending read before its buffer is released"
        );
        assert!(result.is_ok(), "capture cancellation must be graceful");

        let mut child = process
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if completed_before_child_exit {
            let _ = kill_child(&mut child);
        }
        let _ = child.wait();
        assert_eq!(ACTIVE_OUTPUT_READERS.load(Ordering::Acquire), 0);
    }

    #[cfg(unix)]
    #[test]
    fn observing_exit_preserves_pid_until_group_cleanup() {
        let program = std::env::current_exe().expect("current test executable");
        let arguments = [
            "--exact",
            "stdlib::process::tests::fixture_nonzero",
            "--ignored",
            "--nocapture",
        ]
        .map(str::to_string);
        let mut options = ProcessOptions::run_defaults();
        options.stdout = OutputMode::Null;
        options.stderr = OutputMode::Null;
        let process =
            spawn_process(&program, &arguments, options).expect("spawn exit-observation fixture");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let child = process
                .child
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if child_exit_is_pending(&process, &child).expect("observe child exit") {
                break;
            }
            drop(child);
            assert!(Instant::now() < deadline, "child did not exit in time");
            std::thread::sleep(Duration::from_millis(5));
        }

        let mut child = process
            .child
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        assert!(
            child_exit_is_pending(&process, &child).expect("observe the same unreaped child again"),
            "exit observation must preserve the PID through group cleanup"
        );
        let status = reap_if_exited(&process, &mut child)
            .expect("clean up exited process group")
            .expect("child remains available to reap");
        assert!(!status.success());
    }

    #[test]
    fn run_honors_timeout_and_reaps_child() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command("fixture_sleep", &[]);
        let options = HashMap::from([
            ("timeout_ms".to_string(), Value::Int(20)),
            ("termination_grace_ms".to_string(), Value::Int(20)),
        ]);
        let started = Instant::now();
        let result = with_process_capability(&program, || {
            run_from_args(&run_args_with_options(program.clone(), args, options))
                .expect("run result")
        });
        assert!(started.elapsed() < Duration::from_secs(2));
        let (variant, value) = result_variant(result);
        assert_eq!(variant, "Ok");
        let Value::Map(result) = value else {
            panic!("expected process result map");
        };
        assert!(matches!(result.get("success"), Some(Value::Bool(false))));
        assert!(matches!(result.get("timed_out"), Some(Value::Bool(true))));
    }

    #[test]
    fn run_stops_child_when_captured_output_exceeds_limit() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command("fixture_large_output", &[]);
        let options = HashMap::from([
            ("max_output_bytes".to_string(), Value::Int(64)),
            ("termination_grace_ms".to_string(), Value::Int(20)),
        ]);
        let result = with_process_capability(&program, || {
            run_from_args(&run_args_with_options(program.clone(), args, options))
                .expect("run result")
        });
        let (variant, error) = result_variant(result);
        assert_eq!(variant, "Err");
        let error = error.to_string();
        assert!(
            error.contains("exceeded max_output_bytes limit of 64"),
            "unexpected process error: {error}"
        );
    }

    #[test]
    fn run_applies_cwd_environment_and_string_stdin() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command("fixture_context", &[]);
        let directory =
            std::env::temp_dir().join(format!("ntnt-process-context-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("cwd-marker"), b"expected directory").unwrap();
        let options = HashMap::from([
            (
                "cwd".to_string(),
                Value::String(directory.to_string_lossy().into_owned()),
            ),
            (
                "env".to_string(),
                Value::Map(HashMap::from([(
                    "NTNT_PROCESS_FIXTURE".to_string(),
                    Value::String("violet".to_string()),
                )])),
            ),
            (
                "stdin".to_string(),
                Value::Map(HashMap::from([
                    ("mode".to_string(), Value::String("string".to_string())),
                    ("data".to_string(), Value::String("read this".to_string())),
                ])),
            ),
        ]);
        let result = with_process_capability(&program, || {
            run_from_args(&run_args_with_options(program.clone(), args, options))
                .expect("run result")
        });
        std::fs::remove_dir_all(&directory).unwrap();
        let (variant, value) = result_variant(result);
        assert_eq!(variant, "Ok");
        let Value::Map(result) = value else {
            panic!("expected process result map");
        };
        let Some(Value::String(stdout)) = result.get("stdout") else {
            panic!("expected captured stdout");
        };
        assert!(stdout.contains("env=violet"), "stdout={stdout}");
        assert!(stdout.contains("stdin=read this"), "stdout={stdout}");
        assert!(stdout.contains("cwd_marker=true"), "stdout={stdout}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_batch_scripts_are_rejected_as_implicit_shells() {
        let error = validate_windows_program(Path::new("tool.cmd"))
            .expect_err("cmd files must require an explicit shell");
        assert!(error.contains("not direct executables"));
        let error = validate_windows_program(Path::new("tool.BAT"))
            .expect_err("bat files must require an explicit shell");
        assert!(error.contains("not direct executables"));
        assert!(validate_windows_program(Path::new("tool.exe")).is_ok());
    }

    #[test]
    fn run_rejects_executable_outside_allowlist() {
        let (program, args) = current_test_command("fixture_print_args", &[]);
        let _guard = ENV_LOCK.lock().unwrap();
        let old_enable = std::env::var_os("NTNT_PROCESS_ENABLE");
        let old_allow = std::env::var_os("NTNT_PROCESS_ALLOW");
        std::env::set_var("NTNT_PROCESS_ENABLE", "1");
        std::env::set_var(
            "NTNT_PROCESS_ALLOW",
            std::env::temp_dir().join("not-allowed"),
        );
        let result = run_from_args(&run_args(program, args)).expect("run result");
        match old_enable {
            Some(value) => std::env::set_var("NTNT_PROCESS_ENABLE", value),
            None => std::env::remove_var("NTNT_PROCESS_ENABLE"),
        }
        match old_allow {
            Some(value) => std::env::set_var("NTNT_PROCESS_ALLOW", value),
            None => std::env::remove_var("NTNT_PROCESS_ALLOW"),
        }
        let (variant, error) = result_variant(result);
        assert_eq!(variant, "Err");
        assert!(error.to_string().contains("is not allowed"));
    }

    fn supervised_args(program: String, args: Vec<Value>) -> Vec<Value> {
        run_args_with_options(
            program,
            args,
            HashMap::from([
                (
                    "stdout".to_string(),
                    Value::Map(HashMap::from([(
                        "mode".to_string(),
                        Value::String("capture".to_string()),
                    )])),
                ),
                (
                    "stderr".to_string(),
                    Value::Map(HashMap::from([(
                        "mode".to_string(),
                        Value::String("capture".to_string()),
                    )])),
                ),
                ("termination_grace_ms".to_string(), Value::Int(20)),
            ]),
        )
    }

    fn start_supervised_fixture(fixture: &str) -> (String, Value) {
        let (program, args) = current_test_command(fixture, &[]);
        let value = with_process_capability(&program, || {
            start_from_args(&supervised_args(program.clone(), args)).expect("start result")
        });
        let (variant, handle) = result_variant(value);
        assert_eq!(variant, "Ok");
        (program, handle)
    }

    #[test]
    fn lifecycle_signal_after_exit_preserves_result_for_wait() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        RUNTIME.shutdown();
        let (program, argument_values) = current_test_command("fixture_nonzero", &[]);
        let arguments = argument_values
            .into_iter()
            .map(|argument| match argument {
                Value::String(argument) => argument,
                _ => unreachable!("test arguments are strings"),
            })
            .collect::<Vec<_>>();
        let mut options = ProcessOptions::start_defaults();
        options.stdout = OutputMode::Null;
        options.stderr = OutputMode::Null;
        let process = spawn_process(Path::new(&program), &arguments, options)
            .expect("spawn completed-process signal fixture");
        let id = RUNTIME
            .insert(Arc::clone(&process))
            .expect("register process");
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let exited = child_has_exited(
                &process,
                &mut process
                    .child
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
            )
            .expect("observe child exit");
            if exited {
                break;
            }
            assert!(Instant::now() < deadline, "child did not exit in time");
            std::thread::sleep(Duration::from_millis(5));
        }

        let signalled = terminate_from_args(&[Value::ProcessHandle(id)]).expect("signal result");
        let (variant, value) = result_variant(signalled);
        assert_eq!(variant, "Ok");
        assert!(matches!(value, Value::Bool(false)));

        let waited = wait_from_args(&[Value::ProcessHandle(id)]).expect("wait result");
        let (variant, value) = result_variant(waited);
        assert_eq!(variant, "Ok");
        let Value::Map(result) = value else {
            panic!("expected process result map");
        };
        assert!(matches!(result.get("success"), Some(Value::Bool(false))));
        RUNTIME.shutdown();
    }

    #[test]
    fn lifecycle_signal_during_finalization_is_a_noop() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        RUNTIME.shutdown();
        let (program, argument_values) = current_test_command("fixture_nonzero", &[]);
        let arguments = argument_values
            .into_iter()
            .map(|argument| match argument {
                Value::String(argument) => argument,
                _ => unreachable!("test arguments are strings"),
            })
            .collect::<Vec<_>>();
        let mut options = ProcessOptions::start_defaults();
        options.stdout = OutputMode::Null;
        options.stderr = OutputMode::Null;
        let process = spawn_process(Path::new(&program), &arguments, options)
            .expect("spawn finalization-gap fixture");
        let id = RUNTIME
            .insert(Arc::clone(&process))
            .expect("register process");
        let deadline = Instant::now() + Duration::from_secs(2);
        let status = loop {
            if let Some(status) = try_status(&process).expect("observe and reap child") {
                break status;
            }
            assert!(Instant::now() < deadline, "child did not exit in time");
            std::thread::sleep(Duration::from_millis(5));
        };
        assert!(process.terminal.lock().unwrap().is_none());

        let signalled = terminate_from_args(&[Value::ProcessHandle(id)]).expect("signal result");
        let (variant, value) = result_variant(signalled);
        assert_eq!(variant, "Ok");
        assert!(matches!(value, Value::Bool(false)));

        let repeated_status = stop_and_reap(&process).expect("reuse cached reaped status");
        assert_eq!(status.code(), repeated_status.code());
        let result = finalize_process(&process, status, false).expect("finalize process");
        assert!(!result.success);
        RUNTIME.remove(id);
        RUNTIME.shutdown();
    }

    #[test]
    fn lifecycle_start_try_wait_and_cached_wait() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, handle) = start_supervised_fixture("fixture_sleep_short");
        assert!(matches!(handle, Value::ProcessHandle(_)));
        let first = with_process_capability(&program, || {
            try_wait_from_args(std::slice::from_ref(&handle)).expect("try_wait result")
        });
        let (variant, pending) = result_variant(first);
        assert_eq!(variant, "Ok");
        assert!(matches!(pending, Value::EnumValue { variant, .. } if variant == "None"));

        let final_value = with_process_capability(&program, || {
            wait_from_args(std::slice::from_ref(&handle)).expect("wait result")
        });
        let repeated = with_process_capability(&program, || {
            wait_from_args(std::slice::from_ref(&handle)).expect("cached wait result")
        });
        let (variant, value) = result_variant(final_value);
        assert_eq!(variant, "Ok");
        let Value::Map(result) = value else {
            panic!("expected process result map");
        };
        let (repeated_variant, repeated_value) = result_variant(repeated);
        assert_eq!(repeated_variant, "Ok");
        let Value::Map(repeated_result) = repeated_value else {
            panic!("expected cached process result map");
        };
        assert_eq!(
            result.get("exit_code").unwrap().to_string(),
            repeated_result.get("exit_code").unwrap().to_string()
        );
        assert_eq!(
            result.get("stdout").unwrap().to_string(),
            repeated_result.get("stdout").unwrap().to_string()
        );
        assert_eq!(
            result.get("duration_ms").unwrap().to_string(),
            repeated_result.get("duration_ms").unwrap().to_string()
        );
        assert!(matches!(result.get("success"), Some(Value::Bool(true))));
    }

    #[test]
    fn completed_process_registry_is_bounded() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        RUNTIME.shutdown();
        let mut handles = Vec::new();
        for _ in 0..(MAX_RETAINED_TERMINAL_PROCESSES + 4) {
            let (program, handle) = start_supervised_fixture("fixture_nonzero");
            let waited = with_process_capability(&program, || {
                wait_from_args(std::slice::from_ref(&handle)).expect("wait result")
            });
            let (variant, value) = result_variant(waited);
            assert_eq!(variant, "Ok", "unexpected process result: {value}");
            handles.push(handle);
        }

        let (terminal_count, retained_bytes) = {
            let entries = RUNTIME.entries.lock().unwrap();
            let terminal_count = entries
                .values()
                .filter(|process| process.terminal.lock().unwrap().is_some())
                .count();
            let retained_bytes = entries.values().fold(0_usize, |total, process| {
                let bytes = process
                    .terminal
                    .lock()
                    .unwrap()
                    .as_ref()
                    .and_then(|result| result.as_ref().ok())
                    .map_or(0, |summary| summary.stdout.len() + summary.stderr.len());
                total.saturating_add(bytes)
            });
            (terminal_count, retained_bytes)
        };
        assert!(terminal_count <= MAX_RETAINED_TERMINAL_PROCESSES);
        assert!(retained_bytes <= MAX_RETAINED_TERMINAL_BYTES);
        assert!(retained_bytes > 0);

        let expired = wait_from_args(std::slice::from_ref(&handles[0])).unwrap();
        let retained = wait_from_args(std::slice::from_ref(handles.last().unwrap())).unwrap();
        assert_eq!(result_variant(expired).0, "Err");
        assert_eq!(result_variant(retained).0, "Ok");

        RUNTIME.prune_terminal_entries_to(MAX_RETAINED_TERMINAL_PROCESSES, 0);
        let output_evicted = wait_from_args(std::slice::from_ref(handles.last().unwrap())).unwrap();
        assert_eq!(result_variant(output_evicted).0, "Err");
        RUNTIME.shutdown();
    }

    #[test]
    fn lifecycle_terminate_and_kill_active_processes() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, terminate_handle) = start_supervised_fixture("fixture_sleep");
        let terminated = with_process_capability(&program, || {
            terminate_from_args(std::slice::from_ref(&terminate_handle)).unwrap()
        });
        let (variant, value) = result_variant(terminated);
        assert_eq!(variant, "Ok");
        assert!(matches!(value, Value::Bool(true)));
        with_process_capability(&program, || {
            wait_from_args(std::slice::from_ref(&terminate_handle)).unwrap()
        });
        let terminated_again = with_process_capability(&program, || {
            terminate_from_args(std::slice::from_ref(&terminate_handle)).unwrap()
        });
        assert!(
            matches!(result_variant(terminated_again), (variant, Value::Bool(false)) if variant == "Ok")
        );

        let (_, kill_handle) = start_supervised_fixture("fixture_sleep");
        let killed = with_process_capability(&program, || {
            kill_from_args(std::slice::from_ref(&kill_handle)).unwrap()
        });
        assert!(matches!(result_variant(killed), (variant, Value::Bool(true)) if variant == "Ok"));
        with_process_capability(&program, || {
            wait_from_args(std::slice::from_ref(&kill_handle)).unwrap()
        });
    }

    #[test]
    fn lifecycle_runtime_shutdown_reaps_children() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, handle) = start_supervised_fixture("fixture_sleep");
        RUNTIME.shutdown();
        let waited = with_process_capability(&program, || {
            wait_from_args(std::slice::from_ref(&handle)).unwrap()
        });
        let (variant, error) = result_variant(waited);
        assert_eq!(variant, "Err");
        assert!(error.to_string().contains("unknown process handle"));
    }

    #[test]
    fn lifecycle_terminate_remains_responsive_while_waiting() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (_, handle) = start_supervised_fixture("fixture_sleep");
        let Value::ProcessHandle(id) = handle else {
            panic!("expected process handle");
        };
        let process = RUNTIME.get(id).unwrap();
        let waiter = std::thread::spawn(move || monitor_process(&process, true));
        std::thread::sleep(Duration::from_millis(30));
        let started = Instant::now();
        let terminated = terminate_from_args(&[Value::ProcessHandle(id)]).unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(
            matches!(result_variant(terminated), (variant, Value::Bool(true)) if variant == "Ok")
        );
        let waited = waiter.join().unwrap().expect("wait result");
        assert!(waited.is_some());
    }

    #[test]
    fn lifecycle_timeout_is_enforced_without_polling() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command("fixture_sleep", &[]);
        let options = HashMap::from([
            ("timeout_ms".to_string(), Value::Int(20)),
            ("termination_grace_ms".to_string(), Value::Int(20)),
        ]);
        let value = with_process_capability(&program, || {
            start_from_args(&run_args_with_options(program.clone(), args, options))
                .expect("start result")
        });
        let (variant, handle) = result_variant(value);
        assert_eq!(variant, "Ok");
        let Value::ProcessHandle(id) = handle else {
            panic!("expected process handle");
        };

        let process = RUNTIME.get(id).expect("registered process");
        let deadline = Instant::now() + Duration::from_secs(2);
        let terminal = loop {
            let terminal = process.terminal.lock().unwrap().clone();
            if terminal.is_some() || Instant::now() >= deadline {
                break terminal;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            terminal.is_some(),
            "timeout must be enforced without wait/try_wait"
        );
        let summary = terminal.unwrap().expect("timeout result");
        assert!(summary.timed_out);
        RUNTIME.shutdown();
    }

    #[test]
    fn lifecycle_output_limit_is_enforced_without_polling() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command("fixture_large_output", &[]);
        let options = HashMap::from([
            (
                "stdout".to_string(),
                Value::Map(HashMap::from([(
                    "mode".to_string(),
                    Value::String("capture".to_string()),
                )])),
            ),
            ("max_output_bytes".to_string(), Value::Int(64)),
            ("termination_grace_ms".to_string(), Value::Int(20)),
        ]);
        let value = with_process_capability(&program, || {
            start_from_args(&run_args_with_options(program.clone(), args, options))
                .expect("start result")
        });
        let (variant, handle) = result_variant(value);
        assert_eq!(variant, "Ok");
        let Value::ProcessHandle(id) = handle else {
            panic!("expected process handle");
        };

        let process = RUNTIME.get(id).expect("registered process");
        let deadline = Instant::now() + Duration::from_secs(2);
        let terminal = loop {
            let terminal = process.terminal.lock().unwrap().clone();
            if terminal.is_some() || Instant::now() >= deadline {
                break terminal;
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(
            terminal.is_some(),
            "output limits must be enforced without polling"
        );
        let result = terminal.unwrap();
        match result {
            Err(error) => assert!(error.contains("stdout exceeded max_output_bytes")),
            Ok(_) => panic!("output overflow must fail"),
        }
        RUNTIME.shutdown();
    }

    #[cfg(unix)]
    #[test]
    fn run_timeout_terminates_descendants_that_inherit_capture_pipes() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let (program, args) = current_test_command("fixture_spawn_descendant", &[]);
        let options = HashMap::from([
            ("timeout_ms".to_string(), Value::Int(20)),
            ("termination_grace_ms".to_string(), Value::Int(20)),
        ]);
        let started = Instant::now();
        let result = with_process_capability(&program, || {
            run_from_args(&run_args_with_options(program.clone(), args, options))
                .expect("run result")
        });

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "descendant-held pipes must not defeat the timeout"
        );
        let (variant, value) = result_variant(result);
        assert_eq!(variant, "Ok");
        let Value::Map(result) = value else {
            panic!("expected process result map");
        };
        assert!(matches!(result.get("timed_out"), Some(Value::Bool(true))));
    }

    #[cfg(unix)]
    #[test]
    fn detached_descendant_cannot_block_autonomous_finalization() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        let marker =
            std::env::temp_dir().join(format!("ntnt-process-detached-{}", std::process::id()));
        std::fs::remove_file(&marker).ok();
        let marker_text = marker.to_string_lossy().into_owned();
        let (program, args) =
            current_test_command("fixture_spawn_detached_descendant", &[&marker_text]);
        let options = HashMap::from([
            (
                "stdout".to_string(),
                Value::Map(HashMap::from([(
                    "mode".to_string(),
                    Value::String("capture".to_string()),
                )])),
            ),
            (
                "stderr".to_string(),
                Value::Map(HashMap::from([(
                    "mode".to_string(),
                    Value::String("capture".to_string()),
                )])),
            ),
        ]);

        let started = with_process_capability(&program, || {
            start_from_args(&run_args_with_options(program.clone(), args, options))
                .expect("start result")
        });
        let (variant, handle) = result_variant(started);
        assert_eq!(variant, "Ok");
        let Value::ProcessHandle(id) = handle else {
            panic!("expected process handle");
        };
        let process = RUNTIME.get(id).expect("registered process");
        let deadline = Instant::now() + Duration::from_secs(2);
        let terminal = loop {
            if let Some(terminal) = process
                .terminal
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
            {
                break Some(terminal);
            }
            if Instant::now() >= deadline {
                break None;
            }
            std::thread::sleep(Duration::from_millis(5));
        };

        let detached_pid: libc::pid_t = std::fs::read_to_string(&marker)
            .expect("detached fixture wrote pid")
            .parse()
            .expect("valid detached fixture pid");
        unsafe {
            libc::kill(-detached_pid, libc::SIGKILL);
        }
        std::fs::remove_file(marker).ok();

        let wait_started = Instant::now();
        let waited = wait_from_args(&[Value::ProcessHandle(id)]).expect("wait result");
        let wait_elapsed = wait_started.elapsed();
        RUNTIME.shutdown();

        let terminal = terminal.expect("autonomous finalization must be bounded");
        let error = match terminal {
            Err(error) => error,
            Ok(_) => panic!("detached captured pipe must fail clearly"),
        };
        assert!(
            error.contains("remained open after the child exited"),
            "unexpected process error: {error}"
        );
        assert!(wait_elapsed < Duration::from_secs(1));
        assert_eq!(result_variant(waited).0, "Err");
        assert_eq!(ACTIVE_OUTPUT_READERS.load(Ordering::Acquire), 0);
    }

    #[test]
    fn runtime_shutdown_interrupts_blocking_run_processes() {
        let _runtime_guard = RUNTIME_TEST_LOCK.lock().unwrap();
        RUNTIME.shutdown();
        let runner = std::thread::spawn(|| {
            let (program, args) = current_test_command("fixture_sleep", &[]);
            let options = HashMap::from([
                ("timeout_ms".to_string(), Value::Int(5_000)),
                ("termination_grace_ms".to_string(), Value::Int(20)),
            ]);
            let result = with_process_capability(&program, || {
                run_from_args(&run_args_with_options(program.clone(), args, options))
                    .expect("run result")
            });
            let (variant, value) = result_variant(result);
            assert_eq!(variant, "Ok");
            let Value::Map(result) = value else {
                panic!("expected process result map");
            };
            matches!(result.get("success"), Some(Value::Bool(false)))
        });

        let registration_deadline = Instant::now() + Duration::from_secs(2);
        while RUNTIME
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_empty()
            && Instant::now() < registration_deadline
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !RUNTIME
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .is_empty(),
            "blocking run() processes must be registered for runtime shutdown"
        );

        let started = Instant::now();
        RUNTIME.shutdown();
        assert!(runner.join().expect("run thread"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    #[ignore]
    fn fixture_print_args() {
        let args = std::env::args().skip_while(|arg| arg != "--").skip(1);
        for arg in args {
            println!("{arg}");
        }
    }

    #[test]
    #[ignore]
    fn fixture_nonzero() {
        panic!("intentional nonzero fixture");
    }

    #[test]
    #[ignore]
    fn fixture_sleep() {
        std::thread::sleep(Duration::from_secs(10));
    }

    #[test]
    #[ignore]
    fn fixture_sleep_short() {
        std::thread::sleep(Duration::from_millis(100));
    }

    #[test]
    #[ignore]
    fn fixture_large_output() {
        println!("{}", "x".repeat(32 * 1024));
        std::thread::sleep(Duration::from_secs(10));
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn fixture_spawn_descendant() {
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "sleep 2"])
            .spawn()
            .expect("spawn descendant fixture");
        std::thread::sleep(Duration::from_secs(10));
        let _ = child.wait();
    }

    #[cfg(unix)]
    #[test]
    #[ignore]
    fn fixture_spawn_detached_descendant() {
        use std::os::unix::process::CommandExt;

        let marker = std::env::args()
            .skip_while(|argument| argument != "--")
            .nth(1)
            .expect("pid marker argument");
        let executable = std::env::current_exe().unwrap();
        let mut command = std::process::Command::new(executable);
        command.args([
            "--exact",
            "stdlib::process::tests::fixture_write_pid_and_sleep",
            "--ignored",
            "--nocapture",
            "--",
            &marker,
        ]);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let _detached = command.spawn().expect("spawn detached descendant fixture");
        let deadline = Instant::now() + Duration::from_secs(1);
        while !Path::new(&marker).is_file() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            Path::new(&marker).is_file(),
            "detached fixture did not start"
        );
    }

    #[test]
    #[ignore]
    fn fixture_write_pid_and_sleep() {
        let marker = std::env::args()
            .skip_while(|argument| argument != "--")
            .nth(1)
            .expect("pid marker argument");
        std::fs::write(marker, std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(10));
    }

    #[test]
    #[ignore]
    fn fixture_context() {
        let mut stdin = String::new();
        std::io::stdin().read_to_string(&mut stdin).unwrap();
        println!("cwd={}", std::env::current_dir().unwrap().display());
        println!("cwd_marker={}", Path::new("cwd-marker").is_file());
        println!(
            "env={}",
            std::env::var("NTNT_PROCESS_FIXTURE").unwrap_or_default()
        );
        println!("stdin={stdin}");
    }
}
