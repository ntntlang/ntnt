//! Execution: running primitives against a context.
//!
//! This module executes primitives and verifies checks. The context holds
//! the results of actions (HTTP responses, CLI output, etc.) that checks verify.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use super::primitives::{CheckOp, Primitive, Value};
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
/// Actions (Http, Cli, Sql, ReadFile) populate the context.
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

        Primitive::Sql { query, params: _ } => {
            // SQL execution not yet implemented
            ExecuteResult::fail(
                format!("SQL: {}", query),
                "SQL execution not yet implemented",
            )
        }

        Primitive::ReadFile { path } => execute_read_file(path, ctx),

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
            let passed = actual == Some(expected);
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format_value(Some(expected)),
            )
        }

        CheckOp::NotEquals => {
            let passed = actual != Some(expected);
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
            let passed = match (actual, expected) {
                (Some(Value::Number(a)), Value::Number(e)) => a < e,
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("< {}", format_value(Some(expected))),
            )
        }

        CheckOp::GreaterThan => {
            let passed = match (actual, expected) {
                (Some(Value::Number(a)), Value::Number(e)) => a > e,
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("> {}", format_value(Some(expected))),
            )
        }

        CheckOp::InRange => {
            let passed = match (actual, expected) {
                (Some(Value::Number(a)), Value::Range(min, max)) => a >= min && a <= max,
                _ => false,
            };
            ExecuteResult::check_result(
                passed,
                &description,
                format_value(actual),
                format!("in range {}", format_value(Some(expected))),
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
    }
}

/// Format a value for display
fn format_value(value: Option<&Value>) -> String {
    match value {
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

/// Execute multiple primitives and collect results
pub fn execute_all(primitives: &[Primitive], ctx: &mut Context, port: u16) -> Vec<ExecuteResult> {
    primitives.iter().map(|p| execute(p, ctx, port)).collect()
}

#[cfg(test)]
mod tests {
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
