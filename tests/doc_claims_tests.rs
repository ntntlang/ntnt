//! Doc-claim regression tests (DD-063 Rec 2).
//!
//! Each test locks a behavioral claim made in CLAUDE.md's "Critical Syntax
//! Rules" against the actual binary. If one of these fails, the language
//! changed — update CLAUDE.md and docs/AI_AGENT_GUIDE.md in the same PR,
//! never let the agent-facing docs drift from the implementation again.
//!
//! Assertions use error codes (E002/E005/E006/E007), lint rule ids, and short
//! stable fragments — never full message lines.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    let test_file = unique_test_file("doc_claim");
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

// ── Rule 1: maps require `map` keyword — bare `{}` is a block ──────────

#[test]
fn doc_rule01_bare_brace_map_is_parse_error() {
    let (_, stderr, exit_code) = run(r#"let user = { "name": "Alice" }
print(user)"#);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("E002") && stderr.contains("Expected expression"),
        "bare-brace map literal must be a loud parse error: {}",
        stderr
    );
}

#[test]
fn doc_rule01_bare_brace_is_block_expression() {
    let (stdout, _, exit_code) = run("let x = { 1 + 2 }\nprint(x)");
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("3"),
        "bare braces with an expression form a block: {}",
        stdout
    );
}

// ── Rule 2: interpolation is `#{expr}`, not `${expr}` ──────────────────

#[test]
fn doc_rule02_dollar_interpolation_is_literal() {
    let code = r#"let name = "World"
print("Hello, ${name}!")
print("Hello, #{name}!")"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("Hello, ${name}!"),
        "${{}} must not interpolate: {}",
        stdout
    );
    assert!(
        stdout.contains("Hello, World!"),
        "#{{}} must interpolate: {}",
        stdout
    );
}

// ── Rule 3: dot-call is UFCS sugar — x.f(a) calls f(x, a) ───────────────

#[test]
fn doc_rule03_ufcs_builtin() {
    let (stdout, _, exit_code) = run(r#"print("abc".len())"#);
    assert_eq!(exit_code, 0, "UFCS on builtins must work");
    assert!(stdout.contains("3"), "stdout: {}", stdout);
}

#[test]
fn doc_rule03_ufcs_user_function() {
    let code = r#"fn double(x: Int) -> Int { return x * 2 }
print(5.double())"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0, "UFCS on user functions must work");
    assert!(stdout.contains("10"), "stdout: {}", stdout);
}

#[test]
fn doc_rule03_ufcs_imported_function() {
    let code = r#"import { trim } from "std/string"
print("  hi  ".trim())"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0, "UFCS on imported functions must work");
    assert!(stdout.contains("hi"), "stdout: {}", stdout);
}

#[test]
fn doc_rule03_ufcs_stdlib_without_import() {
    let code = r#"let m = map { "a": 1 }
print(m.keys())"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0, "stdlib functions are globally registered");
    assert!(stdout.contains("a"), "stdout: {}", stdout);
}

#[test]
fn doc_rule03_parens_disambiguate_map_key_shadow() {
    // `m.keys` (no parens) reads the map key; `m.keys()` always calls keys(m).
    let code = r#"let m = map { "keys": 42, "a": 1 }
print(m.keys)
print(m.keys())"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.first().map(|l| l.contains("42")).unwrap_or(false),
        "m.keys must read the stored key value: {}",
        stdout
    );
    assert!(
        lines.get(1).map(|l| l.contains("a")).unwrap_or(false)
            && lines.get(1).map(|l| l.contains("keys")).unwrap_or(false),
        "m.keys() must call keys(m), ignoring the key: {}",
        stdout
    );
}

#[test]
fn doc_rule03_map_closure_not_dot_callable() {
    // Documented UFCS gap: closures stored in map values are not dot-callable.
    let code = r#"let m = map { "f": fn(x) { x + 1 } }
print(m.f(10))"#;
    let (_, stderr, exit_code) = run(code);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("E007") && stderr.contains("Undefined function: f"),
        "map-stored closures must fail with E007 on dot-call: {}",
        stderr
    );
}

#[test]
fn doc_rule03_lint_no_unused_import_for_ufcs() {
    // An import used only via dot-call must count as used.
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
fn doc_rule03_ufcs_typo_suggests_near_match() {
    // A misspelled dot-call resolves like a free-function miss and gets a hint.
    let code = r#"fn double(x: Int) -> Int { return x * 2 }
print(5.doubl())"#;
    let (_, stderr, exit_code) = run(code);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("double"),
        "a typo'd dot-call should suggest the near-match free function: {}",
        stderr
    );
}

#[test]
fn doc_rule03_ufcs_suggestion_skips_non_callable_binding() {
    // A variable named `doubl` is an exact match for the typo, but it is not
    // callable — the suggestion must point at the callable `double` instead.
    let code = r#"let doubl = 5
fn double(x: Int) -> Int { return x * 2 }
print(7.doubl())"#;
    let (_, stderr, exit_code) = run(code);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("double"),
        "suggestion must name the callable function, not the variable: {}",
        stderr
    );
}

// ── Rule 4: route functions are global builtins — never import them ────

#[test]
fn doc_rule04_route_fns_not_importable() {
    let code = r#"import { get, listen } from "std/http/server""#;
    let (_, stderr, exit_code) = run(code);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("is not exported from 'std/http/server'"),
        "importing route builtins must fail loudly: {}",
        stderr
    );
}

// ── Rule 5: semicolons are unnecessary (lint warning, not corruption) ──

#[test]
fn doc_rule05_semicolons_execute_correctly() {
    let (stdout, _, exit_code) = run("let x = 1; let y = 2; print(x + y);");
    assert_eq!(exit_code, 0, "semicolons must not corrupt parsing");
    assert!(stdout.contains("3"), "stdout: {}", stdout);
}

#[test]
fn doc_rule05_semicolons_lint_warns() {
    let (stdout, _, exit_code) = lint("let x = 1; print(x);");
    assert_eq!(exit_code, 0, "semicolon warnings must not fail lint");
    assert!(
        stdout.contains("unnecessary_semicolon"),
        "lint must flag semicolons: {}",
        stdout
    );
}

// ── Rule 6: `otherwise` blocks must diverge ────────────────────────────

#[test]
fn doc_rule06_otherwise_nondiverge_is_lint_error() {
    let code = r#"fn main() -> Int {
    let v = int("nope") otherwise { 0 }
    return v
}
print(main())"#;
    let (stdout, _, exit_code) = lint(code);
    assert_eq!(exit_code, 1, "non-diverging otherwise must fail lint");
    assert!(
        stdout.contains("otherwise block does not diverge"),
        "lint output: {}",
        stdout
    );
}

#[test]
fn doc_rule06_otherwise_nondiverge_runtime_error_on_error_path() {
    let code = r#"fn parse() -> Int {
    let v = int("notanum") otherwise { 0 }
    return v
}
print(parse())"#;
    let (_, stderr, exit_code) = run(code);
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("otherwise block must diverge"),
        "runtime must reject non-diverging otherwise on the error path: {}",
        stderr
    );
}

#[test]
fn doc_rule06_otherwise_diverging_recovers() {
    let code = r#"fn parse() -> Int {
    let v = int("notanum") otherwise { return 0 }
    return v
}
print(parse())"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("0"), "stdout: {}", stdout);
}

// ── Rule 7: ranges are `0..10` — range() doesn't exist ─────────────────

#[test]
fn doc_rule07_range_function_undefined() {
    let (_, stderr, exit_code) = run("for i in range(3) { print(i) }");
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("E006") && stderr.contains("range"),
        "range() must be a loud undefined error: {}",
        stderr
    );
}

#[test]
fn doc_rule07_range_syntax_works() {
    let (stdout, _, exit_code) = run("for i in 0..3 { print(i) }");
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("0") && stdout.contains("1") && stdout.contains("2"),
        "stdout: {}",
        stdout
    );
}

// ── Rule 8: `mut` is enforced for indexed mutation only ────────────────

#[test]
fn doc_rule08_plain_reassign_without_mut_succeeds() {
    // Locks ACTUAL current behavior: plain rebinding does not require `mut`.
    // If the maintainer later decides to enforce `mut` for plain reassignment
    // (a breaking change), INVERT this test in the same PR that changes it.
    let (stdout, _, exit_code) = run("let x = 0\nx = x + 1\nprint(x)");
    assert_eq!(
        exit_code, 0,
        "plain rebinding without mut currently succeeds"
    );
    assert!(stdout.contains("1"), "stdout: {}", stdout);
}

#[test]
fn doc_rule08_index_assign_requires_mut() {
    let (_, stderr, exit_code) = run("let arr = [1, 2, 3]\narr[0] = 99\nprint(arr)");
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("Cannot mutate 'arr'"),
        "indexed mutation without mut must fail: {}",
        stderr
    );
}

#[test]
fn doc_rule08_index_assign_with_mut_works() {
    let (stdout, _, exit_code) = run("let mut arr = [1, 2, 3]\narr[0] = 99\nprint(arr)");
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("99"), "stdout: {}", stdout);
}

// ── Rule 9: closures are `fn(x) { ... }` — pipe syntax doesn't exist ───

#[test]
fn doc_rule09_pipe_closure_is_parse_error() {
    let (_, stderr, exit_code) = run("let f = |x| x * 2\nprint(f(3))");
    assert_ne!(exit_code, 0);
    assert!(
        stderr.contains("E002"),
        "pipe-style closures must be a parse error: {}",
        stderr
    );
}

#[test]
fn doc_rule09_fn_closure_works() {
    let (stdout, _, exit_code) = run("let f = fn(x) { x * 2 }\nprint(f(3))");
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("6"), "stdout: {}", stdout);
}

// ── Rule 10 (replaced in docs): module-level `let` with `map {}` works ──

#[test]
fn doc_rule10_module_level_map_works() {
    // Locks the un-stale behavior: the old rule claimed module-level `let`
    // can't hold a map literal. It can.
    let code = r#"let config = map { "k": "v" }
print(config)"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0, "module-level map literals work");
    assert!(
        stdout.contains("k") && stdout.contains("v"),
        "stdout: {}",
        stdout
    );
}

// ── Rule 11: for..in on a string skips with a warning — use chars() ────

#[test]
fn doc_rule11_for_in_string_zero_iterations_with_warning() {
    let code = r#"for c in "ab" { print(c) }
print("done")"#;
    let (stdout, stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert_eq!(
        stdout.trim(),
        "done",
        "for..in on a string must not iterate: {}",
        stdout
    );
    assert!(
        stderr.contains("for..in on String"),
        "the skip must warn, not be silent: {}",
        stderr
    );
}

#[test]
fn doc_rule11_chars_iterates_string() {
    let code = r#"import { chars } from "std/string"
for c in chars("ab") { print(c) }"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("a") && stdout.contains("b"),
        "stdout: {}",
        stdout
    );
}

// ── Rule 12: template strings interpolate with {{expr}} ────────────────

#[test]
fn doc_rule12_template_double_braces() {
    let code = r#"let name = "World"
print("""Hello {{name}}""")
print("Hello {name}")"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("Hello World"),
        "{{{{expr}}}} must interpolate in template strings: {}",
        stdout
    );
    assert!(
        stdout.contains("Hello {name}"),
        "single braces stay literal in regular strings: {}",
        stdout
    );
}

// ── Rule 13: contracts go after return type, before body ───────────────

#[test]
fn doc_rule13_contract_placement_parses_and_runs() {
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

// ── Rule 14: 0 is truthy; falsy are false, "", None, [], map {} ────────

#[test]
fn doc_rule14_truthiness() {
    let code = r#"if 0 { print("zero-truthy") } else { print("zero-falsy") }
if "" { print("str-truthy") } else { print("str-falsy") }
if [] { print("arr-truthy") } else { print("arr-falsy") }
if map {} { print("map-truthy") } else { print("map-falsy") }"#;
    let (stdout, stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("zero-truthy"), "0 is truthy: {}", stdout);
    assert!(stdout.contains("str-falsy"), "\"\" is falsy: {}", stdout);
    assert!(stdout.contains("arr-falsy"), "[] is falsy: {}", stdout);
    assert!(
        stdout.contains("map-falsy"),
        "map {{}} is falsy: {}",
        stdout
    );
    assert!(
        stderr.contains("Non-boolean condition"),
        "non-boolean conditions warn: {}",
        stderr
    );
}

// ── Rule 15: missing map keys return None — use has_key() ──────────────

#[test]
fn doc_rule15_missing_map_key_returns_none() {
    let code = r#"let m = map { "a": 1 }
print(m["missing"])
print(has_key(m, "missing"))
print(m.missing)"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(
        lines.first().map(|l| l.contains("none")).unwrap_or(false),
        "m[\"missing\"] is none: {}",
        stdout
    );
    assert!(
        lines.get(1).map(|l| l.contains("false")).unwrap_or(false),
        "has_key is false: {}",
        stdout
    );
    assert!(
        lines.get(2).map(|l| l.contains("none")).unwrap_or(false),
        "m.missing is none: {}",
        stdout
    );
}

// ── Rule 16: `for k in map` iterates keys — entries() for pairs ────────

#[test]
fn doc_rule16_for_map_iterates_keys() {
    // Entry maps print with nondeterministic field order — always read
    // e.key / e.value, never assert on a printed entry map.
    let code = r#"let m = map { "a": 1, "b": 2 }
for k in m { print("key:" + k) }
for e in entries(m) { print(e.key + "=" + str(e.value)) }"#;
    let (stdout, _, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("key:a") && stdout.contains("key:b"),
        "for..in map yields keys: {}",
        stdout
    );
    assert!(
        stdout.contains("a=1") && stdout.contains("b=2"),
        "entries() yields key/value pairs: {}",
        stdout
    );
}
