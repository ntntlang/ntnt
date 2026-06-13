//! Intentional syntax contract tests (DD-063 Rec 2).
//!
//! These are not a snapshot of every current CLAUDE.md behavior. They only
//! cover syntax rules we want to treat as stable language/agent contracts.
//! If one fails, either the language changed intentionally and docs must be
//! updated in the same PR, or we regressed a contract agents should rely on.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = format!("{:?}", std::thread::current().id());
    std::env::temp_dir()
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
    let release_path = manifest.join(format!("target/release/ntnt{}", exe));

    if debug_path.exists() {
        debug_path
    } else if release_path.exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
    }
}

/// Run `ntnt <subcommand> <file>` on a code snippet.
/// Returns (stdout, stderr, exit_code).
fn run_ntnt(subcommand: &str, code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("syntax_contract");
    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");

    let output = Command::new(ntnt_binary())
        .arg(subcommand)
        .arg(&test_file)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to execute ntnt");
    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

fn run(code: &str) -> (String, String, i32) {
    run_ntnt("run", code)
}

fn lint(code: &str) -> (String, String, i32) {
    run_ntnt("lint", code)
}

#[test]
fn map_literals_require_map_keyword() {
    let (_, stderr, exit_code) = run(r#"let user = { "name": "Alice" }
print(user)"#);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("E002") && stderr.contains("Expected expression"),
        "bare-brace map literals must be loud parse errors: {}",
        stderr
    );
}

#[test]
fn string_interpolation_uses_hash_braces() {
    let (stdout, _, exit_code) = run(r#"let name = "World"
print("Hello, #{name}!")"#);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Hello, World!"), "stdout: {}", stdout);
}

#[test]
fn javascript_style_interpolation_is_diagnosed() {
    let (stdout, _, _) = lint(
        r#"let name = "World"
print("Hello, ${name}!")"#,
    );
    assert!(
        stdout.contains("javascript_style_interpolation") && stdout.contains("#{name}"),
        "${{}} interpolation should be diagnosed with a #{{}} repair hint: {}",
        stdout
    );
}

#[test]
fn dot_call_ufcs_works_for_builtin_imported_and_user_functions() {
    let code = r#"import { trim } from "std/string"
fn double(x: Int) -> Int { return x * 2 }
print("abc".len())
print("  hi  ".trim())
print(5.double())"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("3"), "stdout: {}", stdout);
    assert!(stdout.contains("hi"), "stdout: {}", stdout);
    assert!(stdout.contains("10"), "stdout: {}", stdout);
}

#[test]
fn imported_function_used_by_dot_call_is_not_unused() {
    let code = r#"import { trim } from "std/string"
print("  hi  ".trim())"#;
    let (stdout, _, exit_code) = lint(code);
    assert_eq!(exit_code, 0);
    assert!(
        !stdout.contains("unused_import"),
        "UFCS usage must count as import usage: {}",
        stdout
    );
}

#[test]
fn otherwise_recovery_blocks_must_diverge() {
    let code = r#"fn main() -> Int {
    let v = int("nope") otherwise { 0 }
    return v
}
print(main())"#;
    let (stdout, _, exit_code) = lint(code);
    assert_eq!(exit_code, 1);
    assert!(
        stdout.contains("otherwise block does not diverge"),
        "lint output: {}",
        stdout
    );
}

#[test]
fn template_strings_interpolate_double_braces() {
    let code = r#"let name = "World"
print("""Hello {{name}}""")"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Hello World"), "stdout: {}", stdout);
}

#[test]
fn contract_clauses_go_after_return_type_before_body() {
    let code = r#"fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}
print(divide(10, 2))"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("5"), "stdout: {}", stdout);
}
