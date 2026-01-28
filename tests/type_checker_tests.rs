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
    let code = r#"fn greet(name) {
    return "hello " + name
}

print(greet("world"))
"#;

    let (stdout, _stderr, _code) = lint_strict_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --strict should output valid JSON");

    let warnings = json["summary"]["warnings"].as_i64().unwrap_or(0);
    assert!(
        warnings > 0,
        "Strict lint should produce warnings for untyped params, got 0 warnings"
    );

    // Verify the warnings mention the untyped parameter
    let files = json["files"].as_array().unwrap();
    let issues = files[0]["issues"].as_array().unwrap();
    let type_warnings: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            issue["rule"].as_str() == Some("type_check")
                && issue["severity"].as_str() == Some("warning")
        })
        .collect();

    assert!(
        !type_warnings.is_empty(),
        "Should have type_check warnings for untyped parameters"
    );

    let has_param_warning = type_warnings.iter().any(|w| {
        let msg = w["message"].as_str().unwrap_or("");
        msg.contains("no type annotation") && msg.contains("name")
    });
    assert!(
        has_param_warning,
        "Should warn about untyped parameter 'name'. Warnings: {:?}",
        type_warnings
    );
}

#[test]
fn test_strict_lint_warns_missing_return_type() {
    let code = r#"fn add(a: Int, b: Int) {
    return a + b
}

print(add(1, 2))
"#;

    let (stdout, _stderr, _code) = lint_strict_code(code);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("lint --strict should output valid JSON");

    let files = json["files"].as_array().unwrap();
    let issues = files[0]["issues"].as_array().unwrap();
    let return_warnings: Vec<&serde_json::Value> = issues
        .iter()
        .filter(|issue| {
            issue["rule"].as_str() == Some("type_check")
                && issue["severity"].as_str() == Some("warning")
                && issue["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("no return type")
        })
        .collect();

    assert!(
        !return_warnings.is_empty(),
        "Strict lint should warn about missing return type on 'add'"
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
