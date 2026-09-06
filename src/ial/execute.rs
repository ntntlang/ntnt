//! Execution: running primitives against a context.
//!
//! This module executes primitives and verifies checks. The context holds
//! the results of actions (HTTP responses, CLI output, etc.) that checks verify.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use super::primitives::{CheckOp, Primitive, PropertyType, Value};
use std::path::Path;

/// Execution context - holds results of actions for checks to verify.
///
/// Uses dot-notation paths like:
/// - response.status
/// - response.body
/// - response.headers.content-type
/// - cli.exit_code
/// - cli.stdout
#[derive(Debug, Default)]
pub struct Context {
    values: HashMap<String, Value>,
}

impl Context {
    /// Create a new empty context
    pub fn new() -> Self {
        Context {
            values: HashMap::new(),
        }
    }

    /// Set a value at a path
    pub fn set(&mut self, path: impl Into<String>, value: Value) {
        self.values.insert(path.into(), value);
    }

    /// Get a value at a path
    pub fn get(&self, path: &str) -> Option<&Value> {
        // Direct lookup first
        if let Some(v) = self.values.get(path) {
            return Some(v);
        }

        // Try nested lookup (e.g., "response.headers.content-type")
        // This handles the case where we stored headers as a map
        let parts: Vec<&str> = path.split('.').collect();
        if parts.len() >= 2 {
            // Check if parent is a map
            let parent_path = parts[..parts.len() - 1].join(".");
            if let Some(Value::Map(map)) = self.values.get(&parent_path) {
                let key = parts.last().unwrap();
                return map.get(*key);
            }
        }

        None
    }

    /// Get a string value at a path
    pub fn get_string(&self, path: &str) -> Option<&str> {
        self.get(path).and_then(|v| v.as_str())
    }

    /// Get a number value at a path
    pub fn get_number(&self, path: &str) -> Option<f64> {
        self.get(path).and_then(|v| v.as_number())
    }
}

/// Result of executing a primitive
#[derive(Debug, Clone)]
pub struct ExecuteResult {
    /// Whether the primitive succeeded
    pub passed: bool,
    /// Description of what was checked/done
    pub description: String,
    /// Actual value (for checks)
    pub actual: Option<String>,
    /// Expected value (for checks)
    pub expected: Option<String>,
    /// Error message if failed
    pub message: Option<String>,
}

impl ExecuteResult {
    fn pass(description: impl Into<String>) -> Self {
        ExecuteResult {
            passed: true,
            description: description.into(),
            actual: None,
            expected: None,
            message: None,
        }
    }

    fn fail(description: impl Into<String>, message: impl Into<String>) -> Self {
        ExecuteResult {
            passed: false,
            description: description.into(),
            actual: None,
            expected: None,
            message: Some(message.into()),
        }
    }

    fn check_result(
        passed: bool,
        description: impl Into<String>,
        actual: impl Into<String>,
        expected: impl Into<String>,
    ) -> Self {
        let description = description.into();
        let actual = actual.into();
        let expected = expected.into();

        ExecuteResult {
            passed,
            description: description.clone(),
            actual: Some(actual.clone()),
            expected: Some(expected.clone()),
            message: if passed {
                None
            } else {
                Some(format!("Expected {}, got {}", expected, actual))
            },
        }
    }
}

/// Execute a single primitive against the context.
///
/// Actions (Http, Cli, ReadFile) populate the context.
/// Checks verify values in the context.
pub fn execute(primitive: &Primitive, ctx: &mut Context, port: u16) -> ExecuteResult {
    match primitive {
        Primitive::Http {
            method,
            path,
            body,
            headers: _,
        } => execute_http(method, path, body.as_deref(), ctx, port),

        Primitive::Cli { command, args } => execute_cli(command, args, ctx),

        Primitive::CodeQuality {
            file,
            lint,
            validate,
        } => execute_code_quality(file.as_deref(), *lint, *validate, ctx),

        Primitive::ReadFile { path } => execute_read_file(path, ctx),

        Primitive::FunctionCall {
            source_file,
            function_name,
            args,
        } => execute_function_call(source_file, function_name, args, ctx),

        Primitive::PropertyCheck {
            property,
            source_file,
            function_name,
            input,
        } => execute_property_check(property, source_file, function_name, input, ctx),

        Primitive::Check { op, path, expected } => do_execute_check(op, path, expected, ctx),
    }
}

/// Execute a Check primitive against a pre-populated context.
///
/// This is useful when the context has already been filled with HTTP response
/// data and you just need to verify assertions.
pub fn execute_check(primitive: &Primitive, ctx: &Context) -> ExecuteResult {
    match primitive {
        Primitive::Check { op, path, expected } => do_execute_check(op, path, expected, ctx),
        _ => ExecuteResult::fail(
            "Invalid primitive",
            "execute_check only handles Check primitives",
        ),
    }
}

/// Execute an HTTP request and populate context
fn execute_http(
    method: &str,
    path: &str,
    body: Option<&str>,
    ctx: &mut Context,
    port: u16,
) -> ExecuteResult {
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };

    let body_content = body.unwrap_or("");
    let request = if body_content.is_empty() {
        format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            method, path, port
        )
    } else {
        format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            method, path, port, body_content.len(), body_content
        )
    };

    let start = Instant::now();

    // Try to connect with retries
    let mut attempts = 0;
    let max_attempts = 20;

    while attempts < max_attempts {
        if let Ok(mut stream) = TcpStream::connect(format!("127.0.0.1:{}", port)) {
            stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
            stream.set_write_timeout(Some(Duration::from_secs(5))).ok();

            if stream.write_all(request.as_bytes()).is_ok() {
                let mut response = Vec::new();
                let _ = stream.read_to_end(&mut response);

                if !response.is_empty() {
                    let elapsed = start.elapsed();
                    let response_str = String::from_utf8_lossy(&response).to_string();

                    // Parse response
                    let parts: Vec<&str> = response_str.splitn(2, "\r\n\r\n").collect();
                    let headers_str = parts.first().unwrap_or(&"");
                    let response_body = parts.get(1).unwrap_or(&"").to_string();

                    // Parse status
                    let status = headers_str
                        .lines()
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("0")
                        .parse::<f64>()
                        .unwrap_or(0.0);

                    // Parse headers into a map
                    let mut headers_map = HashMap::new();
                    for line in headers_str.lines().skip(1) {
                        if let Some(idx) = line.find(':') {
                            let key = line[..idx].trim().to_lowercase();
                            let value = line[idx + 1..].trim().to_string();
                            headers_map.insert(key, Value::String(value));
                        }
                    }

                    // Populate context
                    ctx.set("response.status", Value::Number(status));
                    ctx.set("response.body", Value::String(response_body));
                    ctx.set("response.headers", Value::Map(headers_map));
                    ctx.set(
                        "response.time_ms",
                        Value::Number(elapsed.as_millis() as f64),
                    );

                    return ExecuteResult::pass(format!("{} {} → {}", method, path, status as u16));
                }
            }
        }

        attempts += 1;
        std::thread::sleep(Duration::from_millis(100));
    }

    ExecuteResult::fail(
        format!("{} {}", method, path),
        "Connection failed after retries",
    )
}

/// Execute a CLI command and populate context
fn execute_cli(command: &str, args: &[String], ctx: &mut Context) -> ExecuteResult {
    use std::process::Command;

    match Command::new(command).args(args).output() {
        Ok(output) => {
            ctx.set(
                "cli.exit_code",
                Value::Number(output.status.code().unwrap_or(-1) as f64),
            );
            ctx.set(
                "cli.stdout",
                Value::String(String::from_utf8_lossy(&output.stdout).to_string()),
            );
            ctx.set(
                "cli.stderr",
                Value::String(String::from_utf8_lossy(&output.stderr).to_string()),
            );

            ExecuteResult::pass(format!("{} {:?}", command, args))
        }
        Err(e) => ExecuteResult::fail(format!("{} {:?}", command, args), e.to_string()),
    }
}

/// Execute code quality checks (lint + validate) and populate context
///
/// This runs lint and validation checks on .tnt files without spawning external processes.
/// Results are stored in context:
/// - code.quality.passed: bool - overall pass/fail
/// - code.quality.error_count: number - count of errors
/// - code.quality.warning_count: number - count of warnings
/// - code.quality.files_checked: number - count of files checked
/// - code.quality.errors: array - list of error messages
fn execute_code_quality(
    file: Option<&str>,
    run_lint: bool,
    run_validate: bool,
    ctx: &mut Context,
) -> ExecuteResult {
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    let mut error_count = 0;
    let mut warning_count = 0;
    let mut files_checked = 0;
    let mut errors: Vec<Value> = Vec::new();

    // Collect files to check
    let files_to_check: Vec<std::path::PathBuf> = if let Some(path_str) = file {
        let path = Path::new(path_str);
        if path.is_dir() {
            // It's a directory - scan it for .tnt files
            collect_tnt_files_for_quality(path)
        } else if path.is_file() {
            // It's a specific file
            vec![path.to_path_buf()]
        } else {
            // Path doesn't exist - try as directory anyway (might be relative)
            collect_tnt_files_for_quality(path)
        }
    } else {
        // Find all .tnt files in current directory
        collect_tnt_files_for_quality(Path::new("."))
    };

    for file_path in &files_to_check {
        files_checked += 1;
        let file_name = file_path.to_string_lossy().to_string();

        // Read file
        let source = match std::fs::read_to_string(file_path) {
            Ok(s) => s,
            Err(e) => {
                error_count += 1;
                errors.push(Value::String(format!(
                    "{}: Could not read file: {}",
                    file_name, e
                )));
                continue;
            }
        };

        // Parse (this is the core validation)
        if run_validate || run_lint {
            let lexer = Lexer::new(&source);
            let tokens: Vec<_> = lexer.collect();
            let mut parser = Parser::new(tokens);

            match parser.parse() {
                Ok(ast) => {
                    // Parse succeeded - run lint checks if requested
                    if run_lint {
                        // Basic lint checks (can be expanded later)
                        let lint_results = run_basic_lint_checks(&ast, &source, &file_name);
                        warning_count += lint_results.warnings;
                        for warning in lint_results.messages {
                            errors.push(Value::String(warning));
                        }
                    }
                }
                Err(e) => {
                    // Parse failed - this is an error
                    error_count += 1;
                    errors.push(Value::String(format!("{}: {}", file_name, e)));
                }
            }
        }
    }

    let passed = error_count == 0;

    // Populate context
    ctx.set("code.quality.passed", Value::Bool(passed));
    ctx.set(
        "code.quality.error_count",
        Value::Number(error_count as f64),
    );
    ctx.set(
        "code.quality.warning_count",
        Value::Number(warning_count as f64),
    );
    ctx.set(
        "code.quality.files_checked",
        Value::Number(files_checked as f64),
    );
    ctx.set("code.quality.errors", Value::Array(errors.clone()));

    if passed {
        ExecuteResult::pass(format!(
            "Code quality: {} files checked, {} warnings",
            files_checked, warning_count
        ))
    } else {
        let error_summary = errors
            .iter()
            .take(3)
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        ExecuteResult::fail(
            format!(
                "Code quality: {} errors, {} warnings",
                error_count, warning_count
            ),
            error_summary,
        )
    }
}

/// Collect all .tnt files in a directory (recursively, but not too deep)
fn collect_tnt_files_for_quality(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "tnt" {
                        files.push(path);
                    }
                }
            } else if path.is_dir() {
                // Skip hidden directories and common non-source directories
                let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !dir_name.starts_with('.')
                    && dir_name != "node_modules"
                    && dir_name != "target"
                    && dir_name != "dist"
                {
                    // Only go one level deep to avoid huge scans
                    if let Ok(sub_entries) = std::fs::read_dir(&path) {
                        for sub_entry in sub_entries.flatten() {
                            let sub_path = sub_entry.path();
                            if sub_path.is_file() {
                                if let Some(ext) = sub_path.extension() {
                                    if ext == "tnt" {
                                        files.push(sub_path);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    files
}

/// Basic lint check results
struct LintResults {
    warnings: usize,
    messages: Vec<String>,
}

/// Run basic lint checks on a parsed AST
fn run_basic_lint_checks(
    _ast: &crate::ast::Program,
    _source: &str,
    _filename: &str,
) -> LintResults {
    // For now, return no warnings - the main lint is parse success
    // This can be expanded with more sophisticated checks later
    LintResults {
        warnings: 0,
        messages: Vec::new(),
    }
}

/// Read a file and populate context
fn execute_read_file(path: &str, ctx: &mut Context) -> ExecuteResult {
    match std::fs::read_to_string(path) {
        Ok(content) => {
            let file_path = format!("file.{}.content", path.replace(['/', '\\', '.'], "_"));
            ctx.set(&file_path, Value::String(content));
            ctx.set(
                &format!("file.{}.exists", path.replace(['/', '\\', '.'], "_")),
                Value::Bool(true),
            );
            ExecuteResult::pass(format!("Read file: {}", path))
        }
        Err(e) => {
            ctx.set(
                &format!("file.{}.exists", path.replace(['/', '\\', '.'], "_")),
                Value::Bool(false),
            );
            ExecuteResult::fail(format!("Read file: {}", path), e.to_string())
        }
    }
}

/// Execute a check against the context (internal implementation)
fn do_execute_check(op: &CheckOp, path: &str, expected: &Value, ctx: &Context) -> ExecuteResult {
    let actual = ctx.get(path);
    let description = format_check_description(op, path, expected);

    match op {
        CheckOp::Equals => {
            let passed = actual.is_some_and(|actual| native_equal(actual, expected));
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format_value(Some(expected)),
            )
        }

        CheckOp::NotEquals => {
            let passed = actual.is_some_and(|actual| !native_equal(actual, expected));
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("not {}", format_value(Some(expected))),
            )
        }

        CheckOp::Contains => {
            let passed = match (actual, expected) {
                (Some(Value::String(actual_str)), Value::String(expected_str)) => {
                    actual_str.contains(expected_str)
                }
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("contains {}", format_value(Some(expected))),
            )
        }

        CheckOp::NotContains => {
            let passed = match (actual, expected) {
                (Some(Value::String(actual_str)), Value::String(expected_str)) => {
                    !actual_str.contains(expected_str)
                }
                _ => true,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("not contains {}", format_value(Some(expected))),
            )
        }

        CheckOp::Matches => {
            let passed = match (actual, expected) {
                (Some(Value::String(actual_str)), Value::Regex(pattern)) => {
                    regex::Regex::new(pattern)
                        .map(|re| re.is_match(actual_str))
                        .unwrap_or(false)
                }
                (Some(Value::String(actual_str)), Value::String(pattern)) => {
                    regex::Regex::new(pattern)
                        .map(|re| re.is_match(actual_str))
                        .unwrap_or(false)
                }
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("matches {}", format_value(Some(expected))),
            )
        }

        CheckOp::Exists => {
            let passed = actual.is_some() && actual != Some(&Value::Null);
            ExecuteResult::check_result(passed, &description, format_value(actual), "exists")
        }

        CheckOp::NotExists => {
            let passed = actual.is_none() || actual == Some(&Value::Null);
            ExecuteResult::check_result(passed, &description, format_value(actual), "not exists")
        }

        CheckOp::LessThan => {
            let passed =
                actual.and_then(|a| a.numeric_cmp(expected)) == Some(std::cmp::Ordering::Less);
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("< {}", format_value(Some(expected))),
            )
        }

        CheckOp::GreaterThan => {
            let passed =
                actual.and_then(|a| a.numeric_cmp(expected)) == Some(std::cmp::Ordering::Greater);
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("> {}", format_value(Some(expected))),
            )
        }

        CheckOp::InRange => {
            let passed = match (actual, expected) {
                (Some(a), Value::Range(min, max)) => {
                    a.numeric_cmp(&Value::Number(*min))
                        .is_some_and(|order| order.is_ge())
                        && a.numeric_cmp(&Value::Number(*max))
                            .is_some_and(|order| order.is_le())
                }
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("in range {}", format_value(Some(expected))),
            )
        }

        CheckOp::StartsWith => {
            let passed = match (actual, expected) {
                (Some(Value::String(actual_str)), Value::String(prefix)) => {
                    actual_str.starts_with(prefix)
                }
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("starts with {}", format_value(Some(expected))),
            )
        }

        CheckOp::EndsWith => {
            let passed = match (actual, expected) {
                (Some(Value::String(actual_str)), Value::String(suffix)) => {
                    actual_str.ends_with(suffix)
                }
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("ends with {}", format_value(Some(expected))),
            )
        }

        CheckOp::IsType => {
            let passed = match (actual, expected) {
                (Some(value), Value::String(type_name)) => {
                    let actual_type = match value {
                        Value::Native(crate::native_test::NativeValue::Enum {
                            enum_name, ..
                        }) => enum_name.as_str(),
                        Value::Native(
                            crate::native_test::NativeValue::Int(_)
                            | crate::native_test::NativeValue::Float(_),
                        ) => "number",
                        Value::Native(_) => "native",
                        Value::String(_) => "string",
                        Value::Number(_) => "number",
                        Value::Bool(_) => "bool",
                        Value::Array(_) => "array",
                        Value::Map(_) => "map",
                        Value::Null => "null",
                        Value::Range(_, _) => "range",
                        Value::Regex(_) => "regex",
                    };
                    actual_type == type_name.as_str()
                }
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("is type {}", format_value(Some(expected))),
            )
        }

        CheckOp::HasLength => {
            let passed = match (actual, expected) {
                (Some(Value::String(s)), Value::Number(n)) => s.len() == *n as usize,
                (Some(Value::Array(arr)), Value::Number(n)) => arr.len() == *n as usize,
                (Some(Value::Map(map)), Value::Number(n)) => map.len() == *n as usize,
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("has length {}", format_value(Some(expected))),
            )
        }
    }
}

/// Format a check operation for display
fn format_check_description(op: &CheckOp, path: &str, expected: &Value) -> String {
    match op {
        CheckOp::Equals => format!("{} equals {}", path, format_value(Some(expected))),
        CheckOp::NotEquals => format!("{} not equals {}", path, format_value(Some(expected))),
        CheckOp::Contains => format!("{} contains {}", path, format_value(Some(expected))),
        CheckOp::NotContains => format!("{} not contains {}", path, format_value(Some(expected))),
        CheckOp::Matches => format!("{} matches {}", path, format_value(Some(expected))),
        CheckOp::Exists => format!("{} exists", path),
        CheckOp::NotExists => format!("{} not exists", path),
        CheckOp::LessThan => format!("{} < {}", path, format_value(Some(expected))),
        CheckOp::GreaterThan => format!("{} > {}", path, format_value(Some(expected))),
        CheckOp::InRange => format!("{} in {}", path, format_value(Some(expected))),
        CheckOp::StartsWith => format!("{} starts with {}", path, format_value(Some(expected))),
        CheckOp::EndsWith => format!("{} ends with {}", path, format_value(Some(expected))),
        CheckOp::IsType => format!("{} is type {}", path, format_value(Some(expected))),
        CheckOp::HasLength => format!("{} has length {}", path, format_value(Some(expected))),
    }
}

/// Format a value for display
fn format_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::Native(value)) => format!("{value:?}"),
        None => "null".to_string(),
        Some(Value::String(s)) => {
            if s.len() > 100 {
                format!("\"{}...\"", &s[..100])
            } else {
                format!("\"{}\"", s)
            }
        }
        Some(Value::Number(n)) => {
            if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            }
        }
        Some(Value::Bool(b)) => b.to_string(),
        Some(Value::Range(min, max)) => format!("[{}, {}]", min, max),
        Some(Value::Regex(r)) => format!("/{}/", r),
        Some(Value::Null) => "null".to_string(),
        Some(Value::Array(arr)) => format!("[{} items]", arr.len()),
        Some(Value::Map(map)) => format!("{{{} entries}}", map.len()),
    }
}

/// Execute a NTNT function call and store the result in context
///
/// This loads and parses the source file, then invokes the specified function
/// with the given arguments. The result is stored at "result" in the context.
fn execute_function_call(
    source_file: &str,
    function_name: &str,
    args: &[Value],
    ctx: &mut Context,
) -> ExecuteResult {
    use crate::native_test::{self, NativeRequest, NativeValue};
    let interpreter_args: Vec<_> = args.iter().map(ial_value_to_interpreter_value).collect();
    let native_args = match interpreter_args
        .iter()
        .map(NativeValue::from_runtime)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(args) => args,
        Err(error) => return ExecuteResult::fail(format!("Call {function_name}()"), error),
    };
    let response = native_test::execute(&NativeRequest {
        source: source_file.into(),
        function: function_name.into(),
        args: native_args,
    });
    if let Some(error) = response.error {
        return ExecuteResult::fail(format!("Call {function_name}()"), error);
    }
    if let Some(failed) = response
        .assertions
        .iter()
        .find(|assertion| !assertion.passed)
    {
        return ExecuteResult::fail(
            format!("Call {function_name}()"),
            format!("{}:{}: {}", failed.source, failed.line, failed.message),
        );
    }
    match response.value {
        Some(value) => {
            ctx.set("result", native_value_to_ial_value(&value));
            ExecuteResult::pass(format!("Call {function_name}()"))
        }
        None => ExecuteResult::fail(
            format!("Call {function_name}()"),
            "Native function returned no value",
        ),
    }
}

/// Convert IAL Value to interpreter Value
fn ial_value_to_interpreter_value(value: &Value) -> crate::interpreter::Value {
    match value {
        Value::Native(value) => value.to_runtime(),
        Value::String(s) => crate::interpreter::Value::String(s.clone()),
        Value::Number(n) => {
            if n.fract() == 0.0 && *n >= i64::MIN as f64 && *n < -(i64::MIN as f64) {
                crate::interpreter::Value::Int(*n as i64)
            } else {
                crate::interpreter::Value::Float(*n)
            }
        }
        Value::Bool(b) => crate::interpreter::Value::Bool(*b),
        Value::Null => crate::interpreter::Value::Unit,
        Value::Array(arr) => {
            let items: Vec<_> = arr.iter().map(ial_value_to_interpreter_value).collect();
            crate::interpreter::Value::Array(items)
        }
        Value::Map(map) => {
            let mut hm = std::collections::HashMap::new();
            for (k, v) in map {
                hm.insert(k.clone(), ial_value_to_interpreter_value(v));
            }
            crate::interpreter::Value::Map(hm)
        }
        Value::Range(min, max) => {
            // Represent range as an array [min, max]
            crate::interpreter::Value::Array(vec![
                crate::interpreter::Value::Float(*min),
                crate::interpreter::Value::Float(*max),
            ])
        }
        Value::Regex(pattern) => {
            // Represent regex as a string (the pattern)
            crate::interpreter::Value::String(pattern.clone())
        }
    }
}

/// Preserve native enum and integer identity rather than using display strings.
fn native_value_to_ial_value(value: &crate::native_test::NativeValue) -> Value {
    use crate::native_test::NativeValue as N;
    match value {
        N::String(s) => Value::String(s.clone()),

        N::Bool(b) => Value::Bool(*b),
        N::Unit => Value::Null,
        N::Array(items) => Value::Array(items.iter().map(native_value_to_ial_value).collect()),
        N::Map(items) => Value::Map(
            items
                .iter()
                .map(|(k, v)| (k.clone(), native_value_to_ial_value(v)))
                .collect(),
        ),
        N::Int(_) | N::Float(_) | N::Enum { .. } => Value::Native(value.clone()),
    }
}

fn native_equal(actual: &Value, expected: &Value) -> bool {
    fn contains_native(value: &Value) -> bool {
        match value {
            Value::Native(_) => true,
            Value::Array(items) => items.iter().any(contains_native),
            Value::Map(items) => items.values().any(contains_native),
            _ => false,
        }
    }
    if !contains_native(actual) && !contains_native(expected) {
        return actual == expected;
    }
    use crate::native_test::NativeValue;
    match (
        NativeValue::from_runtime(&ial_value_to_interpreter_value(actual)),
        NativeValue::from_runtime(&ial_value_to_interpreter_value(expected)),
    ) {
        (Ok(actual), Ok(expected)) => actual == expected,
        _ => false,
    }
}

/// Execute a property check (deterministic, idempotent, round-trips)
fn execute_property_check(
    property: &PropertyType,
    source_file: &str,
    function_name: &str,
    input: &Value,
    ctx: &mut Context,
) -> ExecuteResult {
    let mut first = Context::new();
    let initial = execute_function_call(
        source_file,
        function_name,
        std::slice::from_ref(input),
        &mut first,
    );
    if !initial.passed {
        return initial;
    }
    let Some(value) = first.get("result").cloned() else {
        return ExecuteResult::fail("Native property", "First call produced no value");
    };
    let (next_function, next_input, expected) = match property {
        PropertyType::Deterministic => (function_name, input, &value),
        PropertyType::Idempotent => (function_name, &value, &value),
        PropertyType::RoundTrips { inverse_function } => (inverse_function.as_str(), &value, input),
    };
    let mut second = Context::new();
    let repeated = execute_function_call(
        source_file,
        next_function,
        std::slice::from_ref(next_input),
        &mut second,
    );
    if !repeated.passed {
        return repeated;
    }
    let actual = second.get("result");
    let passed = actual.is_some_and(|actual| native_equal(actual, expected));
    if let Some(value) = actual {
        ctx.set("result", value.clone());
    }
    ExecuteResult::check_result(
        passed,
        format!("Native property {property:?}"),
        format_value(actual),
        format_value(Some(expected)),
    )
}

/// Execute multiple primitives and collect results
pub fn execute_all(primitives: &[Primitive], ctx: &mut Context, port: u16) -> Vec<ExecuteResult> {
    primitives.iter().map(|p| execute(p, ctx, port)).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_ial_results_keep_int_and_float_distinct() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("values.tnt");
        std::fs::write(
            &source,
            "fn integer() { return 42 }\nfn floating() { return 42.0 }",
        )
        .unwrap();
        let mut integer = Context::new();
        let mut floating = Context::new();
        assert!(
            execute_function_call(source.to_str().unwrap(), "integer", &[], &mut integer).passed
        );
        assert!(
            execute_function_call(source.to_str().unwrap(), "floating", &[], &mut floating).passed
        );
        assert_ne!(integer.get("result"), floating.get("result"));
        for ctx in [&integer, &floating] {
            assert_eq!(ctx.get_number("result"), Some(42.0));
            for (op, expected) in [
                (CheckOp::LessThan, Value::Number(100.0)),
                (CheckOp::GreaterThan, Value::Number(0.0)),
                (CheckOp::InRange, Value::Range(40.0, 50.0)),
                (CheckOp::IsType, Value::String("number".into())),
            ] {
                let result = execute_check(
                    &Primitive::Check {
                        op,
                        path: "result".into(),
                        expected,
                    },
                    ctx,
                );
                assert!(result.passed, "{result:?}");
            }
        }
    }

    #[test]
    fn native_ial_numeric_ordering_keeps_large_integers_exact() {
        use crate::native_test::NativeValue;
        use std::cmp::Ordering;
        for (integer, float, expected) in [
            (
                9_007_199_254_740_993,
                9_007_199_254_740_992.0,
                Ordering::Greater,
            ),
            (i64::MAX, 9_223_372_036_854_775_808.0, Ordering::Less),
            (i64::MIN, -9_223_372_036_854_775_808.0, Ordering::Equal),
            (0, -0.5, Ordering::Greater),
            (0, 0.5, Ordering::Less),
        ] {
            let a = Value::Native(NativeValue::Int(integer));
            let b = Value::Number(float);
            assert_eq!(a.numeric_cmp(&b), Some(expected));
            assert_eq!(b.numeric_cmp(&a), Some(expected.reverse()));
        }
        let number = Value::Native(NativeValue::Int(42));
        assert_eq!(number.numeric_cmp(&Value::String("42".into())), None);
        assert_eq!(number.numeric_cmp(&Value::Number(f64::NAN)), None);
    }

    #[test]
    fn native_result_identity_is_not_debug_text() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("domain.tnt");
        std::fs::write(&source, "fn sample() { return Err(\"expected\") }").unwrap();
        let mut ctx = super::Context::new();
        let result =
            super::execute_function_call(source.to_str().unwrap(), "sample", &[], &mut ctx);
        assert!(result.passed);
        assert!(
            !matches!(ctx.get("result"), Some(super::Value::String(_))),
            "Result identity was stringified"
        );
    }

    use super::*;

    #[test]
    fn test_context_set_get() {
        let mut ctx = Context::new();
        ctx.set("response.status", Value::Number(200.0));

        assert_eq!(ctx.get_number("response.status"), Some(200.0));
    }

    #[test]
    fn test_check_equals() {
        let ctx = {
            let mut c = Context::new();
            c.set("response.status", Value::Number(200.0));
            c
        };

        let result = do_execute_check(
            &CheckOp::Equals,
            "response.status",
            &Value::Number(200.0),
            &ctx,
        );
        assert!(result.passed);
    }

    #[test]
    fn test_check_contains() {
        let ctx = {
            let mut c = Context::new();
            c.set("response.body", Value::String("Hello, World!".to_string()));
            c
        };

        let result = do_execute_check(
            &CheckOp::Contains,
            "response.body",
            &Value::String("World".to_string()),
            &ctx,
        );
        assert!(result.passed);
    }

    #[test]
    fn test_check_in_range() {
        let mut ctx = Context::new();
        ctx.set("response.status", Value::Number(201.0));

        let result = do_execute_check(
            &CheckOp::InRange,
            "response.status",
            &Value::Range(200.0, 299.0),
            &ctx,
        );
        assert!(result.passed);

        ctx.set("response.status", Value::Number(404.0));
        let result = do_execute_check(
            &CheckOp::InRange,
            "response.status",
            &Value::Range(200.0, 299.0),
            &ctx,
        );
        assert!(!result.passed);
    }

    #[test]
    fn test_check_not_contains() {
        let ctx = {
            let mut c = Context::new();
            c.set("response.body", Value::String("Success".to_string()));
            c
        };

        let result = do_execute_check(
            &CheckOp::NotContains,
            "response.body",
            &Value::String("error".to_string()),
            &ctx,
        );
        assert!(result.passed);
    }
}
