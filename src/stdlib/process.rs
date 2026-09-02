//! Supervised native process execution.

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

const DEFAULT_MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_TERMINATION_GRACE_MS: u64 = 5_000;

#[derive(Clone)]
enum StdinMode {
    Null,
    Inherit,
    File(PathBuf),
    Data(Vec<u8>),
}

#[derive(Clone)]
enum OutputMode {
    Capture,
    Inherit,
    Null,
    File { path: PathBuf, append: bool },
}

#[derive(Clone)]
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
}

struct SpawnedProcess {
    child: Child,
    stdout_reader: Option<JoinHandle<std::result::Result<Vec<u8>, String>>>,
    stderr_reader: Option<JoinHandle<std::result::Result<Vec<u8>, String>>>,
    stdin_writer: Option<JoinHandle<std::result::Result<(), String>>>,
    output_error: Arc<Mutex<Option<String>>>,
    started_at: Instant,
    options: ProcessOptions,
}

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

fn parse_options(value: Option<&Value>) -> Result<ProcessOptions> {
    let mut parsed = ProcessOptions::run_defaults();
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
    parsed.stdin = parse_stdin(options.get("stdin"))?;
    parsed.stdout = parse_output(options.get("stdout"), OutputMode::Capture, "stdout")?;
    parsed.stderr = parse_output(options.get("stderr"), OutputMode::Capture, "stderr")?;
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

fn parse_command(args: &[Value]) -> Result<(String, Vec<String>, ProcessOptions)> {
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
    Ok((program.clone(), arguments, parse_options(args.get(2))?))
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
        for extension in ["exe", "cmd", "bat", "com"] {
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
    if std::env::var("NTNT_PROCESS_ENABLE").as_deref() != Ok("1") {
        return Err("process execution is disabled; set NTNT_PROCESS_ENABLE=1".to_string());
    }
    let resolved = resolve_program(program)?;
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

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    stream: &'static str,
    max_bytes: usize,
    output_error: Arc<Mutex<Option<String>>>,
) -> JoinHandle<std::result::Result<Vec<u8>, String>> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 16 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .map_err(|error| format!("failed reading process {stream}: {error}"))?;
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
) -> std::result::Result<SpawnedProcess, String> {
    let mut command = Command::new(executable);
    command.args(arguments);
    if let Some(cwd) = &options.cwd {
        command.current_dir(cwd);
    }
    if options.clear_env {
        command.env_clear();
    }
    command.envs(&options.env);
    command.stdout(output_stdio(&options.stdout)?);
    command.stderr(output_stdio(&options.stderr)?);
    match &options.stdin {
        StdinMode::Null => command.stdin(Stdio::null()),
        StdinMode::Inherit => command.stdin(Stdio::inherit()),
        StdinMode::File(path) => {
            let file = File::open(path).map_err(|error| {
                format!("cannot open process input '{}': {error}", path.display())
            })?;
            command.stdin(Stdio::from(file))
        }
        StdinMode::Data(_) => command.stdin(Stdio::piped()),
    };

    let started_at = Instant::now();
    let mut child = command.spawn().map_err(|error| {
        format!(
            "failed to launch process '{}': {error}",
            executable.display()
        )
    })?;
    let output_error = Arc::new(Mutex::new(None));
    let stdout_reader = child.stdout.take().map(|stdout| {
        spawn_reader(
            stdout,
            "stdout",
            options.max_output_bytes,
            Arc::clone(&output_error),
        )
    });
    let stderr_reader = child.stderr.take().map(|stderr| {
        spawn_reader(
            stderr,
            "stderr",
            options.max_output_bytes,
            Arc::clone(&output_error),
        )
    });
    let stdin_writer = match (&options.stdin, child.stdin.take()) {
        (StdinMode::Data(data), Some(mut stdin)) => {
            let data = data.clone();
            Some(std::thread::spawn(move || {
                stdin
                    .write_all(&data)
                    .map_err(|error| format!("failed writing process stdin: {error}"))
            }))
        }
        _ => None,
    };
    Ok(SpawnedProcess {
        child,
        stdout_reader,
        stderr_reader,
        stdin_writer,
        output_error,
        started_at,
        options,
    })
}

#[cfg(unix)]
fn terminate_child(child: &mut Child) -> std::io::Result<()> {
    let result = unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGTERM) };
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

#[cfg(windows)]
fn terminate_child(child: &mut Child) -> std::io::Result<()> {
    child.kill()
}

fn stop_and_reap(process: &mut SpawnedProcess) -> std::result::Result<ExitStatus, String> {
    terminate_child(&mut process.child)
        .map_err(|error| format!("failed to terminate process: {error}"))?;
    let deadline = Instant::now() + process.options.termination_grace;
    loop {
        if let Some(status) = process
            .child
            .try_wait()
            .map_err(|error| format!("failed to monitor process: {error}"))?
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            process
                .child
                .kill()
                .map_err(|error| format!("failed to kill process: {error}"))?;
            return process
                .child
                .wait()
                .map_err(|error| format!("failed to reap process: {error}"));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn join_reader(
    reader: Option<JoinHandle<std::result::Result<Vec<u8>, String>>>,
    stream: &str,
) -> std::result::Result<Vec<u8>, String> {
    match reader {
        Some(reader) => reader
            .join()
            .map_err(|_| format!("process {stream} reader panicked"))?,
        None => Ok(Vec::new()),
    }
}

fn exit_signal(status: &ExitStatus) -> Value {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        status
            .signal()
            .map(|signal| Value::Int(signal as i64))
            .map(Value::some)
            .unwrap_or_else(Value::none)
    }
    #[cfg(not(unix))]
    {
        let _ = status;
        Value::none()
    }
}

fn result_map(
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
    duration: Duration,
) -> Value {
    let exit_code = status
        .code()
        .map(|code| Value::some(Value::Int(code as i64)))
        .unwrap_or_else(Value::none);
    Value::Map(HashMap::from([
        (
            "success".to_string(),
            Value::Bool(status.success() && !timed_out),
        ),
        ("exit_code".to_string(), exit_code),
        ("signal".to_string(), exit_signal(&status)),
        (
            "stdout".to_string(),
            Value::String(String::from_utf8_lossy(&stdout).into_owned()),
        ),
        (
            "stderr".to_string(),
            Value::String(String::from_utf8_lossy(&stderr).into_owned()),
        ),
        ("timed_out".to_string(), Value::Bool(timed_out)),
        (
            "duration_ms".to_string(),
            Value::Int(duration.as_millis().min(i64::MAX as u128) as i64),
        ),
    ]))
}

fn wait_for_run(mut process: SpawnedProcess) -> std::result::Result<Value, String> {
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        let output_error = {
            process
                .output_error
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone()
        };
        if let Some(error) = output_error {
            let _ = stop_and_reap(&mut process);
            return Err(error);
        }
        if crate::stdlib::concurrent::is_current_task_cancelled() {
            cancelled = true;
            break stop_and_reap(&mut process)?;
        }
        if process
            .options
            .timeout
            .is_some_and(|timeout| process.started_at.elapsed() >= timeout)
        {
            timed_out = true;
            break stop_and_reap(&mut process)?;
        }
        if let Some(status) = process
            .child
            .try_wait()
            .map_err(|error| format!("failed to monitor process: {error}"))?
        {
            break status;
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    if let Some(writer) = process.stdin_writer {
        writer
            .join()
            .map_err(|_| "process stdin writer panicked".to_string())??;
    }
    let stdout = join_reader(process.stdout_reader, "stdout")?;
    let stderr = join_reader(process.stderr_reader, "stderr")?;
    if cancelled {
        return Err("process cancelled".to_string());
    }
    Ok(result_map(
        status,
        stdout,
        stderr,
        timed_out,
        process.started_at.elapsed(),
    ))
}

fn run_from_args(args: &[Value]) -> Result<Value> {
    let (program, arguments, options) = parse_command(args)?;
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
    match wait_for_run(process) {
        Ok(result) => Ok(Value::ok(result)),
        Err(error) => Ok(Value::err(Value::String(error))),
    }
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
    // @param program Executable path or name resolved through PATH
    // @param args Literal arguments passed directly to the executable
    // @param options Optional cwd, env, stdio, timeout, grace, and output-limit settings
    // @returns Ok with exit, output, timeout, and duration fields; Err for capability or monitoring failures
    // @since v0.5.3
    // @tags #process #system
    // @example run("/usr/bin/ffmpeg", ["-version"]) => Ok({...}) ~ "Run without a shell"
    // @error RuntimeError ~ "process execution is disabled" fix: "Set NTNT_PROCESS_ENABLE=1 for a trusted application"
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

    module
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interpreter::Value;
    use std::sync::{LazyLock, Mutex};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

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

    #[test]
    fn run_honors_timeout_and_reaps_child() {
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
        assert!(error
            .to_string()
            .contains("stdout exceeded max_output_bytes limit of 64"));
    }

    #[test]
    fn run_applies_cwd_environment_and_string_stdin() {
        let (program, args) = current_test_command("fixture_context", &[]);
        let directory =
            std::env::temp_dir().join(format!("ntnt-process-context-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let expected_cwd = std::fs::canonicalize(&directory).unwrap();
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
        assert!(
            stdout.contains(&format!("cwd={}", expected_cwd.display())),
            "stdout={stdout}"
        );
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
    fn fixture_large_output() {
        println!("{}", "x".repeat(32 * 1024));
        std::thread::sleep(Duration::from_secs(10));
    }

    #[test]
    #[ignore]
    fn fixture_context() {
        let mut stdin = String::new();
        std::io::stdin().read_to_string(&mut stdin).unwrap();
        println!("cwd={}", std::env::current_dir().unwrap().display());
        println!(
            "env={}",
            std::env::var("NTNT_PROCESS_FIXTURE").unwrap_or_default()
        );
        println!("stdin={stdin}");
    }
}
