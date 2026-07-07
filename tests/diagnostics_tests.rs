//! Integration tests for DD-063 Rec 4a diagnostics:
//! multi-error parser recovery in lint/validate, and enriched E004 contract
//! violations (clause line, source frame, call site, runtime values).

use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique test file path
fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = format!("{:?}", std::thread::current().id());
    let temp_dir = std::env::temp_dir();
    temp_dir
        .join(format!(
            "ntnt_{}_{}_{}_{}.tnt",
            prefix,
            std::process::id(),
            thread_id.replace(|c: char| !c.is_alphanumeric(), "_"),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

fn ntnt_binary() -> std::path::PathBuf {
    let exe = std::env::consts::EXE_SUFFIX;
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let debug_path = manifest.join(format!("target/debug/ntnt{}", exe));
    let dev_release_path = manifest.join(format!("target/dev-release/ntnt{}", exe));
    let release_path = manifest.join(format!("target/release/ntnt{}", exe));

    if debug_path.exists() {
        debug_path
    } else if dev_release_path.exists() {
        dev_release_path
    } else if release_path.exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
    }
}

/// Run the ntnt binary with the given subcommand args on a code snippet.
/// Returns (stdout, stderr, exit_code).
fn run_ntnt(args: &[&str], code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("diagnostics");
    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");

    let mut cmd = Command::new(ntnt_binary());
    cmd.args(args)
        .arg(&test_file)
        .current_dir(env!("CARGO_MANIFEST_DIR"));

    let output = cmd.output().expect("Failed to execute ntnt");
    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

fn lint(code: &str) -> (String, String, i32) {
    run_ntnt(&["lint"], code)
}

fn run(code: &str) -> (String, String, i32) {
    run_ntnt(&["run"], code)
}

fn count_parse_errors(lint_stdout: &str) -> usize {
    lint_stdout.matches("\"rule\": \"parse_error\"").count()
}

// ── Parser error recovery (lint) ───────────────────────────────────────

#[test]
fn lint_reports_multiple_parse_errors_with_distinct_lines() {
    let code = "fn one() {\n    let 1 = 2\n}\n\nfn two() {\n    let 3 = 4\n}\n\nfn three() {\n    let 5 = 6\n}";
    let (stdout, _stderr, _) = lint(code);

    // Three errors, one per broken function, each with a severity of error
    assert_eq!(
        count_parse_errors(&stdout),
        3,
        "expected 3 parse_error issues in: {stdout}"
    );
    assert!(stdout.contains("\"line\": 2"), "missing line 2: {stdout}");
    assert!(stdout.contains("\"line\": 6"), "missing line 6: {stdout}");
    assert!(stdout.contains("\"line\": 10"), "missing line 10: {stdout}");
}

#[test]
fn lint_caps_parse_errors_and_notes_suppression() {
    let mut code = String::new();
    for i in 0..8 {
        code.push_str(&format!("let {} = {}\n", i * 2, i * 2 + 1));
    }
    let (stdout, _stderr, _) = lint(&code);

    assert_eq!(
        count_parse_errors(&stdout),
        // 5 real errors + 1 informational cap notice (same rule id)
        6,
        "expected capped errors plus notice in: {stdout}"
    );
    assert!(
        stdout.contains("additional parse errors may be suppressed"),
        "missing cap notice: {stdout}"
    );
}

#[test]
fn lint_does_not_cascade_from_a_single_broken_expression() {
    let code = "let x = 1 +\nlet y = 2\nlet z = 3";
    let (stdout, _stderr, _) = lint(code);

    let count = count_parse_errors(&stdout);
    assert!(
        (1..=2).contains(&count),
        "one broken expression should yield 1-2 errors, got {count}: {stdout}"
    );
}

#[test]
fn lint_skips_semantic_checks_when_parse_errors_exist() {
    // The undefined variable would normally produce a type_check issue;
    // with a parse error present, semantic checks must be suppressed.
    let code = "let 1 = 2\nprint(definitely_undefined_variable)";
    let (stdout, _stderr, _) = lint(code);

    assert!(count_parse_errors(&stdout) >= 1);
    assert!(
        !stdout.contains("\"rule\": \"type_check\""),
        "semantic issues reported on a partial AST: {stdout}"
    );
}

#[test]
fn run_still_aborts_on_first_parse_error_only() {
    let code = "fn one() {\n    let 1 = 2\n}\n\nfn two() {\n    let 3 = 4\n}";
    let (_stdout, stderr, exit_code) = run(code);

    assert_ne!(exit_code, 0);
    assert!(stderr.contains("error[E002]"), "stderr: {stderr}");
    assert!(stderr.contains("line 2"), "stderr: {stderr}");
    // The second error is never reached on the run path
    assert!(
        !stderr.contains("line 6"),
        "run path reported more than the first error: {stderr}"
    );
}

// ── Enriched E004 contract violations ──────────────────────────────────

#[test]
fn e004_precondition_shows_clause_line_frame_and_values() {
    let code = "fn divide(a: Int, b: Int) -> Int\n    requires b != 0\n{\n    return a / b\n}\n\nlet boom = divide(10, 0)";
    let (_stdout, stderr, exit_code) = run(code);

    assert_ne!(exit_code, 0);
    assert!(stderr.contains("error[E004]"), "stderr: {stderr}");
    assert!(
        stderr.contains("Precondition failed in 'divide': b != 0"),
        "stderr: {stderr}"
    );
    // Clause line (the `requires` on line 2) drives the location + frame
    assert!(stderr.contains(":2"), "missing clause line: {stderr}");
    assert!(
        stderr.contains("requires b != 0"),
        "missing source frame: {stderr}"
    );
    assert!(stderr.contains("where: b = 0"), "stderr: {stderr}");
    assert!(
        stderr.contains("call at line 7"),
        "missing call-site note: {stderr}"
    );
}

#[test]
fn e004_postcondition_shows_result_value() {
    let code = "fn buggy_add(a: Int, b: Int) -> Int\n    ensures result == a + b\n{\n    return a + b + 1\n}\n\nlet x = buggy_add(2, 3)";
    let (_stdout, stderr, exit_code) = run(code);

    assert_ne!(exit_code, 0);
    assert!(stderr.contains("error[E004]"), "stderr: {stderr}");
    assert!(
        stderr.contains("where: result = 6, a = 2, b = 3"),
        "stderr: {stderr}"
    );
}

#[test]
fn e004_multi_param_clause_lists_all_values_in_order() {
    let code = "fn withdraw(from_balance: Int, amount: Int) -> Int\n    requires from_balance >= amount\n{\n    return from_balance - amount\n}\n\nlet x = withdraw(50, 100)";
    let (_stdout, stderr, exit_code) = run(code);

    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("where: from_balance = 50, amount = 100"),
        "stderr: {stderr}"
    );
}

#[test]
fn e004_ufcs_receiver_appears_in_where_values() {
    // `s.len()` is a MethodCall — the receiver must still be collected
    let code = "fn shout(s: String) -> String\n    requires s.len() > 0\n{\n    return s\n}\n\nlet x = shout(\"\")";
    let (_stdout, stderr, exit_code) = run(code);

    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("where: s = \"\""),
        "UFCS receiver missing from where values: {stderr}"
    );
}

#[test]
fn e004_message_text_is_unchanged_for_existing_consumers() {
    // HTTP mapping and user tests match on this exact Display prefix
    let code = "fn divide(a: Int, b: Int) -> Int\n    requires b != 0\n{\n    return a / b\n}\n\nlet boom = divide(10, 0)";
    let (_stdout, stderr, _) = run(code);

    assert!(
        stderr.contains("Contract violation: Precondition failed in 'divide': b != 0"),
        "Display text drifted: {stderr}"
    );
}
