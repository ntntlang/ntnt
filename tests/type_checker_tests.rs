//! Integration tests for NTNT type checker
//!
//! Tests the type checking diagnostics produced by `ntnt lint` and `ntnt validate`
//! CLI commands against .tnt files with various type annotation scenarios.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique test file path in the scratchpad directory
fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = format!("{:?}", std::thread::current().id());
    let temp_dir = std::env::temp_dir();
    temp_dir
        .join(format!(
            "ntnt_typecheck_{}_{}_{}_{}.tnt",
            prefix,
            std::process::id(),
            thread_id.replace(|c: char| !c.is_alphanumeric(), "_"),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

/// Helper to run ntnt command and capture output
fn run_ntnt(args: &[&str]) -> (String, String, i32) {
    // Prefer debug binary (matches cargo test profile), fall back to release
    let exe = std::env::consts::EXE_SUFFIX;
    let debug_path = format!("./target/debug/ntnt{}", exe);
    let release_path = format!("./target/release/ntnt{}", exe);

    let binary = if std::path::Path::new(&debug_path).exists() {
        debug_path
    } else if std::path::Path::new(&release_path).exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
    };

    let output = Command::new(binary)
        .args(args)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to execute ntnt");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    (stdout, stderr, code)
}

/// Helper to write a .tnt file with the given code, run `ntnt lint`, clean up, and return output
fn lint_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("lint");

    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    write!(file, "{}", code).expect("Failed to write test file");
    drop(file);

    let result = run_ntnt(&["lint", &test_file]);

    // Clean up
    fs::remove_file(&test_file).ok();

    result
}

/// Helper to write a .tnt file with the given code, run `ntnt lint --strict`, clean up, and return output
fn lint_strict_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("lint_strict");

    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    write!(file, "{}", code).expect("Failed to write test file");
    drop(file);

    let result = run_ntnt(&["lint", "--strict", &test_file]);

    // Clean up
    fs::remove_file(&test_file).ok();

    result
}

/// Helper to write a .tnt file with the given code, run `ntnt validate`, clean up, and return output
fn validate_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("validate");

    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    write!(file, "{}", code).expect("Failed to write test file");
    drop(file);

    let result = run_ntnt(&["validate", &test_file]);

    // Clean up
    fs::remove_file(&test_file).ok();

    result
}

// ============================================================================
// Type checker lint tests
// ============================================================================

#[test]
fn test_lint_allows_function_local_postgres_connect() {
    let code = r#"import { connect as pg_connect } from "std/db/postgres"

fn load_count() {
    let conn = pg_connect("postgres://example/app") otherwise { return None }
    return conn
}
"#;

    let (stdout, _stderr, code) = lint_code(code);
    assert_eq!(code, 0, "warning-only lint should exit zero");

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");
    let files = json["files"].as_array().unwrap();
    let has_stale_warning = files.iter().any(|file| {
        file["issues"]
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .any(|issue| issue["rule"].as_str() == Some("postgres_connect_in_function"))
    });
    assert!(
        !has_stale_warning,
        "function-local Postgres connect is shared-pool backed and should not warn: {stdout}"
    );
}

#[test]
fn test_lint_catches_arg_type_mismatch() {
    let code = r#"fn add(a: Int, b: Int) -> Int {
    return a + b
}

let result = add("hello", "world")
print(result)
"#;

    let (stdout, _stderr, code) = lint_code(code);

    // Lint should exit with a non-zero code when type errors are found
    assert_ne!(
        code, 0,
        "lint should report failure when type errors are present"
    );

    // Parse the JSON output to verify type_check errors are reported
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap();
    assert!(
        errors > 0,
        "Should report at least one type error, got {} errors",
        errors
    );

    // Verify the issues contain type_check rule errors about argument mismatches
    let files = json["files"].as_array().unwrap();
    assert!(!files.is_empty(), "Should have at least one file entry");

    let issues = files[0]["issues"].as_array().unwrap();
    let type_errors: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            issue["rule"].as_str() == Some("type_check")
                && issue["severity"].as_str() == Some("error")
        })
        .collect();

    assert!(
        !type_errors.is_empty(),
        "Should have type_check errors for argument type mismatches"
    );

    // At least one error should mention the argument type mismatch
    let has_arg_mismatch = type_errors.iter().any(|e| {
        let msg = e["message"].as_str().unwrap_or("");
        msg.contains("expected Int") && msg.contains("got String")
    });
    assert!(
        has_arg_mismatch,
        "Should mention expected Int but got String in at least one error. Errors: {:?}",
        type_errors
    );
}

#[test]
fn test_lint_catches_return_type_mismatch() {
    let code = r#"fn greet(name: String) -> Int {
    return "hello " + name
}

print(greet("world"))
"#;

    let (stdout, _stderr, code) = lint_code(code);

    // Lint should exit with a non-zero code for return type mismatch
    assert_ne!(
        code, 0,
        "lint should report failure when return type mismatches"
    );

    // Parse the JSON output
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap();
    assert!(
        errors > 0,
        "Should report at least one type error for return type mismatch"
    );

    // Verify type_check errors mention return type mismatch
    let files = json["files"].as_array().unwrap();
    let issues = files[0]["issues"].as_array().unwrap();
    let type_errors: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            issue["rule"].as_str() == Some("type_check")
                && issue["severity"].as_str() == Some("error")
        })
        .collect();

    assert!(
        !type_errors.is_empty(),
        "Should have type_check errors for return type mismatch"
    );

    // At least one error should describe the return type mismatch
    let has_return_mismatch = type_errors.iter().any(|e| {
        let msg = e["message"].as_str().unwrap_or("");
        msg.contains("Return type mismatch")
            || (msg.contains("expected Int") && msg.contains("String"))
    });
    assert!(
        has_return_mismatch,
        "Should mention return type mismatch (expected Int, returning String). Errors: {:?}",
        type_errors
    );
}

#[test]
fn test_lint_no_errors_on_untyped_code() {
    let code = r#"fn greet(name) {
    return "hello " + name
}

fn add(a, b) {
    return a + b
}

print(greet("world"))
print(add(1, 2))
"#;

    let (stdout, _stderr, code) = lint_code(code);

    // Lint should succeed (exit 0) for untyped code -- no type errors possible
    assert_eq!(
        code, 0,
        "lint should succeed for untyped code (no type annotations to violate)"
    );

    // Parse the JSON output and verify zero errors
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap();
    assert_eq!(errors, 0, "Should report zero errors for untyped code");

    // May have suggestions (e.g., missing contracts) but no type_check errors
    let files = json["files"].as_array().unwrap();
    if !files.is_empty() {
        let issues = files[0]["issues"].as_array().unwrap();
        let type_errors: Vec<&serde_json::Value> = issues
            .iter()
            .filter(|issue| {
                issue["rule"].as_str() == Some("type_check")
                    && issue["severity"].as_str() == Some("error")
            })
            .collect();

        assert!(
            type_errors.is_empty(),
            "Untyped code should have zero type_check errors, found: {:?}",
            type_errors
        );
    }
}

#[test]
fn test_lint_no_errors_on_correctly_typed_code() {
    let code = r#"fn add(a: Int, b: Int) -> Int {
    return a + b
}

fn greet(name: String) -> String {
    return "hello " + name
}

print(add(1, 2))
print(greet("world"))
"#;

    let (stdout, _stderr, code) = lint_code(code);

    // Lint should succeed for correctly typed code
    assert_eq!(code, 0, "lint should succeed for correctly typed code");

    // Parse the JSON output and verify zero errors
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap();
    assert_eq!(
        errors, 0,
        "Should report zero errors for correctly typed code"
    );

    // Verify no type_check error issues
    let files = json["files"].as_array().unwrap();
    if !files.is_empty() {
        let issues = files[0]["issues"].as_array().unwrap();
        let type_errors: Vec<&serde_json::Value> = issues
            .iter()
            .filter(|issue| {
                issue["rule"].as_str() == Some("type_check")
                    && issue["severity"].as_str() == Some("error")
            })
            .collect();

        assert!(
            type_errors.is_empty(),
            "Correctly typed code should have zero type_check errors, found: {:?}",
            type_errors
        );
    }
}

#[test]
fn test_lint_mixed_typed_untyped_no_false_positives() {
    let code = r#"fn typed_add(a: Int, b: Int) -> Int {
    return a + b
}

fn untyped_greet(name) {
    return "hello " + name
}

fn typed_concat(a: String, b: String) -> String {
    return a + b
}

fn untyped_identity(x) {
    return x
}

print(typed_add(1, 2))
print(untyped_greet("world"))
print(typed_concat("foo", "bar"))
print(untyped_identity(42))
"#;

    let (stdout, _stderr, code) = lint_code(code);

    // Lint should succeed -- all typed calls use correct types, untyped code is unchecked
    assert_eq!(
        code, 0,
        "lint should succeed for mixed typed/untyped code with no type violations"
    );

    // Parse the JSON output and verify zero errors
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap();
    assert_eq!(
        errors, 0,
        "Should report zero errors when typed code is correct and untyped code is present"
    );

    // Verify no type_check errors at all (no false positives from untyped code)
    let files = json["files"].as_array().unwrap();
    if !files.is_empty() {
        let issues = files[0]["issues"].as_array().unwrap();
        let type_errors: Vec<&serde_json::Value> = issues
            .iter()
            .filter(|issue| {
                issue["rule"].as_str() == Some("type_check")
                    && issue["severity"].as_str() == Some("error")
            })
            .collect();

        assert!(
            type_errors.is_empty(),
            "Mixed typed/untyped code should not produce false positive type errors, found: {:?}",
            type_errors
        );
    }

    // Suggestions (like missing contracts) are expected and acceptable
    let suggestions = json["summary"]["suggestions"].as_i64().unwrap_or(0);
    assert!(suggestions >= 0, "Suggestions count should be non-negative");
}

#[test]
fn test_validate_catches_type_errors() {
    let code = r#"fn add(a: Int, b: Int) -> Int {
    return a + b
}

let result = add("hello", "world")
print(result)
"#;

    let (_stdout, stderr, code) = validate_code(code);

    // Validate should exit with a non-zero code when type errors are present
    assert_ne!(
        code, 0,
        "validate should report failure when type errors exist"
    );

    // The stderr should mention type errors
    let combined = format!("{}{}", _stdout, stderr);
    let mentions_type_error = combined.contains("type error")
        || combined.contains("type_check")
        || combined.contains("Type error")
        || combined.contains("Errors:");

    assert!(
        mentions_type_error,
        "validate output should mention type errors. stdout: {}, stderr: {}",
        _stdout, stderr
    );
}

// ============================================================================
// Strict lint tests
// ============================================================================

#[test]
fn test_strict_lint_warns_untyped_params() {
    let source = r#"fn greet(name) {
    return "hello " + name
}

print(greet("world"))
"#;

    let (stdout, _stderr, exit_code) = lint_strict_code(source);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --strict should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert!(
        errors > 0,
        "Strict lint should produce errors for untyped params, got 0 errors"
    );

    // In strict mode, missing annotations are promoted to errors (not warnings)
    let files = json["files"].as_array().unwrap();
    let issues = files[0]["issues"].as_array().unwrap();
    let type_errors: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            issue["rule"].as_str() == Some("type_check")
                && issue["severity"].as_str() == Some("error")
        })
        .collect();

    assert!(
        !type_errors.is_empty(),
        "Should have type_check errors for untyped parameters in strict mode"
    );

    let has_param_error = type_errors.iter().any(|w| {
        let msg = w["message"].as_str().unwrap_or("");
        msg.contains("no type annotation") && msg.contains("name")
    });
    assert!(
        has_param_error,
        "Should error about untyped parameter 'name' in strict mode. Errors: {:?}",
        type_errors
    );

    // Strict lint with errors should exit non-zero
    assert!(
        exit_code != 0,
        "ntnt lint --strict should exit non-zero when annotation errors exist, got exit code {}",
        exit_code
    );
}

#[test]
fn test_strict_lint_warns_missing_return_type() {
    let source = r#"fn add(a: Int, b: Int) {
    return a + b
}

print(add(1, 2))
"#;

    let (stdout, _stderr, exit_code) = lint_strict_code(source);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --strict should output valid JSON");

    let files = json["files"].as_array().unwrap();
    let issues = files[0]["issues"].as_array().unwrap();
    // In strict mode, missing return type annotations are promoted to errors
    let return_errors: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            issue["rule"].as_str() == Some("type_check")
                && issue["severity"].as_str() == Some("error")
                && issue["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("no return type")
        })
        .collect();

    assert!(
        !return_errors.is_empty(),
        "Strict lint should error about missing return type on 'add'"
    );

    // Strict lint with errors should exit non-zero
    assert!(
        exit_code != 0,
        "ntnt lint --strict should exit non-zero when annotation errors exist, got exit code {}",
        exit_code
    );
}

#[test]
fn test_strict_lint_no_warnings_on_fully_typed() {
    let code = r#"fn add(a: Int, b: Int) -> Int {
    return a + b
}

fn greet(name: String) -> String {
    return "hello " + name
}

print(add(1, 2))
print(greet("world"))
"#;

    let (stdout, _stderr, _code) = lint_strict_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --strict should output valid JSON");

    let files = json["files"].as_array().unwrap();
    if !files.is_empty() {
        let issues = files[0]["issues"].as_array().unwrap();
        let type_warnings: Vec<&serde_json::Value> = issues
            .iter()
            .filter(|issue| {
                issue["rule"].as_str() == Some("type_check")
                    && issue["severity"].as_str() == Some("warning")
            })
            .collect();

        assert!(
            type_warnings.is_empty(),
            "Fully typed code should have zero strict lint warnings, found: {:?}",
            type_warnings
        );
    }
}

#[test]
fn test_strict_lint_not_triggered_without_flag() {
    // Same untyped code, but without --strict — should have zero type warnings
    let code = r#"fn greet(name) {
    return "hello " + name
}

print(greet("world"))
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let files = json["files"].as_array().unwrap();
    if !files.is_empty() {
        let issues = files[0]["issues"].as_array().unwrap();
        let type_warnings: Vec<&serde_json::Value> = issues
            .iter()
            .filter(|issue| {
                issue["rule"].as_str() == Some("type_check")
                    && issue["severity"].as_str() == Some("warning")
            })
            .collect();

        assert!(
            type_warnings.is_empty(),
            "Without --strict, untyped code should have zero type_check warnings, found: {:?}",
            type_warnings
        );
    }
}

// ============================================================================
// --warn-untyped integration tests (DD-009)
// ============================================================================

/// Helper to run `ntnt lint --warn-untyped` on code
fn lint_warn_untyped_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("lint_warn_untyped");
    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    write!(file, "{}", code).expect("Failed to write test file");
    drop(file);
    let result = run_ntnt(&["lint", "--warn-untyped", &test_file]);
    fs::remove_file(&test_file).ok();
    result
}

#[test]
fn test_warn_untyped_produces_warnings_not_errors() {
    let source = r#"fn greet(name) {
    return "hello " + name
}

print(greet("world"))
"#;

    let (stdout, _stderr, exit_code) = lint_warn_untyped_code(source);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --warn-untyped should output valid JSON");

    let warnings = json["summary"]["warnings"].as_i64().unwrap_or(0);
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);

    assert!(
        warnings > 0,
        "--warn-untyped should produce warnings for untyped params, got 0"
    );
    assert_eq!(
        errors, 0,
        "--warn-untyped should NOT produce errors (warnings are non-fatal), got {} errors",
        errors
    );

    // Exit code should be 0 (warnings are non-fatal)
    assert_eq!(
        exit_code, 0,
        "--warn-untyped should exit 0 (warnings are non-fatal), got exit code {}",
        exit_code
    );
}

#[test]
fn test_warn_untyped_no_warnings_on_fully_typed() {
    let code = r#"fn add(a: Int, b: Int) -> Int {
    return a + b
}

print(add(1, 2))
"#;

    let (stdout, _stderr, _code) = lint_warn_untyped_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --warn-untyped should output valid JSON");

    let files = json["files"].as_array().unwrap();
    if !files.is_empty() {
        let issues = files[0]["issues"].as_array().unwrap();
        let type_warnings: Vec<&serde_json::Value> = issues
            .iter()
            .filter(|issue| {
                issue["rule"].as_str() == Some("type_check")
                    && (issue["severity"].as_str() == Some("warning")
                        || issue["severity"].as_str() == Some("error"))
            })
            .collect();

        assert!(
            type_warnings.is_empty(),
            "Fully typed code with --warn-untyped should have zero type warnings, found: {:?}",
            type_warnings
        );
    }
}

// ============================================================================
// Contract type-checking integration tests
// ============================================================================

#[test]
fn test_lint_contracts_no_errors_on_valid() {
    let code = r#"fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}

print(divide(10, 2))
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(errors, 0, "Valid contracts should produce zero type errors");
}

#[test]
fn test_lint_contracts_ensures_with_len_result() {
    let code = r#"fn greet(name: String) -> String
    ensures len(result) > 0
{
    return "Hello, " + name
}

print(greet("world"))
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(
        errors, 0,
        "ensures len(result) > 0 should be valid when result is String"
    );
}

#[test]
fn test_lint_contracts_with_old() {
    let code = r#"fn increment(x: Int) -> Int
    ensures result == old(x) + 1
{
    return x + 1
}

print(increment(5))
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(
        errors, 0,
        "Contracts with old() should produce zero type errors"
    );
}

// ============================================================================
// Line number accuracy regression tests
// ============================================================================

#[test]
fn test_line_numbers_multiple_if_conditions() {
    // The third if-condition (line 11) has a type issue (Int instead of Bool).
    // The type checker should NOT report it on line 2 (the first if).
    let code = r#"
let x: Int = 10
if x > 5 {
    print("big")
}
let y: Bool = true
if y {
    print("yes")
}
let z: Int = 42
if z {
    print("truthy")
}
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let files = json["files"].as_array().unwrap();
    if files.is_empty() {
        return; // No file entries means no issues
    }

    let issues = files[0]["issues"].as_array().unwrap();
    let condition_warnings: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            let msg = issue["message"].as_str().unwrap_or("");
            msg.contains("Condition has type") && msg.contains("instead of Bool")
        })
        .collect();

    assert!(
        !condition_warnings.is_empty(),
        "Should have at least one condition type warning"
    );

    // The warning should be near line 11 (the `if z` line), not line 3 (the first `if`)
    for w in &condition_warnings {
        if let Some(line) = w["line"].as_i64() {
            assert!(
                line >= 10,
                "Condition warning should be near line 11 (the `if z`), not line {} (first `if`). Warning: {}",
                line,
                w["message"].as_str().unwrap_or("")
            );
        }
    }
}

#[test]
fn test_line_numbers_multiple_comparisons() {
    // Two == comparisons; only the second has incompatible types.
    // The warning should point to the second comparison, not the first.
    let code = r#"
fn check(a: Int, b: Int) -> Bool {
    return a == b
}

fn mixed(x: Int, y: String) -> Bool {
    return x == y
}
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let files = json["files"].as_array().unwrap();
    if files.is_empty() {
        return;
    }

    let issues = files[0]["issues"].as_array().unwrap();
    let comparison_warnings: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            let msg = issue["message"].as_str().unwrap_or("");
            msg.contains("Comparison") && msg.contains("incompatible types")
        })
        .collect();

    assert!(
        !comparison_warnings.is_empty(),
        "Should have at least one comparison type warning"
    );

    // The warning should be near line 7 (x == y), not line 3 (a == b)
    for w in &comparison_warnings {
        if let Some(line) = w["line"].as_i64() {
            assert!(
                line >= 6,
                "Comparison warning should be near line 7 (x == y), not line {} (a == b). Warning: {}",
                line,
                w["message"].as_str().unwrap_or("")
            );
        }
    }
}

#[test]
fn test_condition_type_hint_is_actionable() {
    // An if with an Int condition should produce a hint mentioning "!= 0"
    let code = r#"
let count: Int = 5
if count {
    print("has items")
}
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let files = json["files"].as_array().unwrap();
    if files.is_empty() {
        return;
    }

    let issues = files[0]["issues"].as_array().unwrap();
    let condition_warnings: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            let msg = issue["message"].as_str().unwrap_or("");
            msg.contains("Condition has type Int")
        })
        .collect();

    assert!(
        !condition_warnings.is_empty(),
        "Should have a condition type warning for Int"
    );

    // The hint should mention "!= 0" as an actionable suggestion
    let has_hint = condition_warnings.iter().any(|w| {
        let hint = w["hint"].as_str().unwrap_or("");
        hint.contains("!= 0")
    });
    assert!(
        has_hint,
        "Condition type hint for Int should suggest '!= 0'. Warnings: {:?}",
        condition_warnings
    );
}

#[test]
fn test_comparison_type_hint_suggests_conversion() {
    // Comparing Int with String should suggest int()/str() conversion
    let code = r#"
fn check(x: Int, y: String) -> Bool {
    return x == y
}
"#;

    let (stdout, _stderr, _code) = lint_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");

    let files = json["files"].as_array().unwrap();
    if files.is_empty() {
        return;
    }

    let issues = files[0]["issues"].as_array().unwrap();
    let comparison_warnings: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            let msg = issue["message"].as_str().unwrap_or("");
            msg.contains("Comparison") && msg.contains("incompatible types")
        })
        .collect();

    assert!(
        !comparison_warnings.is_empty(),
        "Should have a comparison type warning"
    );

    // The hint should mention handleable int() conversion or str() conversion
    let has_conversion_hint = comparison_warnings.iter().any(|w| {
        let hint = w["hint"].as_str().unwrap_or("");
        hint.contains("int(") && (hint.contains("??") || hint.contains("str("))
    });
    assert!(
        has_conversion_hint,
        "Comparison hint should suggest handleable int()/str() conversion. Warnings: {:?}",
        comparison_warnings
    );
}

#[test]
fn test_result_null_coalesce_fallback_type_is_checked() {
    let code = r#"
let x: Int = int("bad") ?? "not an int"
"#;

    let (stdout, _stderr, _code) = lint_code(code);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");
    let files = json["files"].as_array().unwrap();
    assert!(!files.is_empty(), "lint should report file results");
    let issues = files[0]["issues"].as_array().unwrap();
    assert!(
        issues.iter().any(|issue| {
            let msg = issue["message"].as_str().unwrap_or("");
            msg.contains("Type mismatch") && msg.contains("String") && msg.contains("Int")
        }),
        "Result ?? fallback with a String fallback should not typecheck as Int. Issues: {:?}",
        issues
    );
}

#[test]
fn test_known_success_null_coalesce_does_not_union_unreachable_fallback() {
    let code = r#"
let a: Int = Some(1) ?? "not an int"
let b: Int = Ok(2) ?? "also not an int"
let c: Int = Some(3) ?? len()
"#;

    let (stdout, _stderr, _code) = lint_code(code);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(
        errors, 0,
        "known Some/Ok coalesce should infer the success value type only. Output: {}",
        stdout
    );
}

#[test]
fn test_user_enum_variants_named_like_option_result_do_not_coalesce_as_builtins() {
    let code = r#"
enum MyEnum {
    Ok(Int),
    Err(String),
    Some(Int),
    None
}

let a: MyEnum = MyEnum::Ok(1) ?? len()
let b: MyEnum = MyEnum::Some(2) ?? len()
let c: MyEnum = MyEnum::Err("bad") ?? len()
let d: MyEnum = MyEnum::None ?? len()
"#;

    let (stdout, _stderr, _code) = lint_code(code);
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint should output valid JSON");
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(
        errors, 0,
        "user enum variants named Some/None/Ok/Err should stay user enum values under ??. Output: {}",
        stdout
    );
}

// ============================================================================
// Generic type parameter unification tests (DD-009 Phase 7.4)
// ============================================================================

#[test]
fn test_generic_identity_infers_return_type() {
    // identity<T>(42) should infer T=Int, return type Int
    // Assigning to String should produce a type error
    let source = r#"
fn identity<T>(x: T) -> T {
    return x
}
let result: String = identity(42)
"#;
    let (stdout, _stderr, exit_code) = lint_code(source);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert!(
        errors > 0,
        "identity<T>(42) assigned to String should produce a type error"
    );
    assert_ne!(exit_code, 0);
}

#[test]
fn test_generic_identity_correct_usage() {
    // identity<T>(42) assigned to Int should be fine
    let source = r#"
fn identity<T>(x: T) -> T {
    return x
}
let result: Int = identity(42)
"#;
    let (stdout, _stderr, _exit_code) = lint_code(source);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(
        errors, 0,
        "identity<T>(42) assigned to Int should not error"
    );
}

#[test]
fn test_generic_conflicting_type_params() {
    // fn f<T>(a: T, b: T) called with (Int, String) should error
    let source = r#"
fn merge<T>(a: T, b: T) -> T {
    return a
}
let result = merge(42, "hello")
"#;
    let (stdout, _stderr, _exit_code) = lint_code(source);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert!(
        errors > 0,
        "merge<T>(Int, String) should produce a type error for conflicting T"
    );
}

#[test]
fn test_generic_array_unification() {
    // fn first<T>(arr: [T]) -> T should unify T from array element type
    let source = r#"
fn first<T>(arr: [T]) -> T {
    return arr[0]
}
let x: Int = first([1, 2, 3])
"#;
    let (stdout, _stderr, _exit_code) = lint_code(source);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(
        errors, 0,
        "first<T>([Int]) assigned to Int should not error"
    );
}

#[test]
fn test_std_net_phase1_signatures_accept_valid_usage() {
    let source = r#"
import { ip_parse, subnet_contains, subnet_split, subnet_supernet, subnet_summarize, ip_range_to_cidrs, ping, tcp_connect, reachable, port_scan, dns_lookup, dns_reverse, tls_info } from "std/net"

let parsed = ip_parse("192.168.1.0/24")
let contains = subnet_contains("10.0.0.0/8", "10.1.0.0/16")
let split = subnet_split("192.168.1.0/24", 28)
let split_with_opts = subnet_split("192.168.1.0/24", 28, map { "max_results": 32 })
let parent = subnet_supernet("192.168.1.0/24")
let summary = subnet_summarize(["10.0.0.0/25", "10.0.0.128/25"])
let range = ip_range_to_cidrs("192.168.1.20", "192.168.1.31")
let icmp = ping("example.com", map { "method": "icmp", "timeout_ms": 1000 })
let tcp = tcp_connect("example.com", 443, map { "timeout_ms": 1000 })
let reachability = reachable("example.com", map { "tcp_ports": [443], "timeout_ms": 1000 })
let scan = port_scan("example.com", [80, 443], map { "timeout_ms": 1000, "concurrency": 20 })
let dns = dns_lookup("example.com", "A", map { "timeout_ms": 1000 })
let mx_dns = dns_lookup("example.com", "MX", map { "timeout_ms": 1000 })
let reverse_dns = dns_reverse("8.8.8.8", map { "timeout_ms": 1000 })
let cert = tls_info("example.com", map { "port": 443, "timeout_ms": 1000, "server_name": "example.com" })
"#;
    let (stdout, _stderr, _exit_code) = lint_code(source);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert_eq!(
        errors, 0,
        "std/net valid usage should not type error: {stdout}"
    );
}

#[test]
fn test_std_net_phase1_signatures_reject_wrong_argument_types() {
    let source = r#"
import { subnet_split, ping, port_scan, dns_lookup, tls_info } from "std/net"

let split = subnet_split("192.168.1.0/24", "28")
let reachability = ping(123)
let scan = port_scan("example.com", ["80"])
let dns = dns_lookup("example.com", 123)
let cert = tls_info(443)
"#;
    let (stdout, _stderr, exit_code) = lint_code(source);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let errors = json["summary"]["errors"].as_i64().unwrap_or(0);
    assert!(
        errors > 0,
        "wrong std/net argument types should error: {stdout}"
    );
    assert_ne!(exit_code, 0);
    assert!(
        stdout.contains("expected Int") || stdout.contains("expected String"),
        "type errors should mention expected argument types: {stdout}"
    );
}

// ===========================================================================
// Unknown-method lint (DD-063 Rec 4b)
// ===========================================================================

#[test]
fn test_unknown_method_warns_in_default_mode() {
    let source = r#"let s = "hi"
print(s.length())
"#;
    let (stdout, _stderr, exit_code) = lint_code(source);
    assert!(
        stdout.contains("\"rule\": \"unknown_method\""),
        "default-mode lint should flag s.length(): {stdout}"
    );
    assert!(
        stdout.contains("try len(s)"),
        "hint should bridge to the free function: {stdout}"
    );
    assert!(
        stdout.contains("\"severity\": \"warning\""),
        "should be a warning in default mode: {stdout}"
    );
    assert_eq!(exit_code, 0, "warnings must not fail default lint");
}

#[test]
fn test_unknown_method_errors_in_strict_mode() {
    let source = r#"let s = "hi"
print(s.length())
"#;
    let (stdout, _stderr, _exit_code) = lint_strict_code(source);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let found = json["files"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|f| f["issues"].as_array().cloned().unwrap_or_default())
        .any(|i| i["rule"] == "unknown_method" && i["severity"] == "error");
    assert!(found, "strict mode should promote to error: {stdout}");
}

#[test]
fn test_unknown_method_suppressed_for_untyped_receiver() {
    // Untyped param -> receiver is Any -> no warning (runtime hint catches it)
    let source = r#"fn shout(s) {
    return s.length()
}
"#;
    let (stdout, _stderr, _exit) = lint_code(source);
    assert!(
        !stdout.contains("unknown_method"),
        "Any receivers must not warn: {stdout}"
    );
}

#[test]
fn test_unknown_method_warns_for_non_callable_binding() {
    // A plain value binding named like the method must not silence the
    // lint — UFCS dispatch on it fails at runtime
    let source = r#"let length = 42
let s = "hi"
print(s.length())
"#;
    let (stdout, _stderr, _exit) = lint_code(source);
    assert!(
        stdout.contains("unknown_method"),
        "non-callable bindings must not suppress: {stdout}"
    );
}

#[test]
fn test_unknown_method_suppressed_for_let_bound_lambda() {
    let source = r#"let double = fn(x) { x * 2 }
let n = 5
print(n.double())
"#;
    let (stdout, _stderr, _exit) = lint_code(source);
    assert!(
        !stdout.contains("unknown_method"),
        "let-bound lambdas are UFCS-callable: {stdout}"
    );
}

#[test]
fn test_unknown_method_suppressed_with_unresolved_import() {
    let source = r#"import { mystery } from "./does_not_exist_anywhere.tnt"
let s = "hi"
print(s.definitely_not_real())
"#;
    let (stdout, _stderr, _exit) = lint_code(source);
    assert!(
        !stdout.contains("unknown_method"),
        "unresolved imports must suppress the check: {stdout}"
    );
}

#[test]
fn test_ufcs_known_functions_do_not_warn() {
    let source = r#"let s = "hi"
let arr = [1, 2, 3]
let x = Some(1)
print(s.len())
print(arr.push(4))
print(x.is_some())
print(x.unwrap_or(0))
"#;
    let (stdout, _stderr, _exit) = lint_code(source);
    assert!(
        !stdout.contains("unknown_method"),
        "real builtins/runtime globals must not warn: {stdout}"
    );
}

#[test]
fn test_option_helper_free_call_arity_checked() {
    // is_some is now in builtin_sigs, so wrong arity is caught statically
    let source = r#"let x = Some(1)
print(is_some(x, 99))
"#;
    let (stdout, _stderr, _exit) = lint_code(source);
    assert!(
        stdout.contains("is_some") && stdout.contains("argument"),
        "arity mismatch on is_some should be reported: {stdout}"
    );
}
