//! Integration tests for JavaScript-style `${...}` interpolation detection (DD-063 Rec 1).
//!
//! NTNT interpolation is `#{expr}`; a `${expr}` in a regular string is output
//! literally. These tests cover the scope-aware lint rule
//! `javascript_style_interpolation` and the deduplicated runtime [WARN].

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
    let release_path = manifest.join(format!("target/release/ntnt{}", exe));

    if debug_path.exists() {
        debug_path
    } else if release_path.exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
    }
}

/// Run the ntnt binary with the given subcommand args on a code snippet.
/// Returns (stdout, stderr, exit_code).
fn run_ntnt(args: &[&str], code: &str, env_vars: &[(&str, &str)]) -> (String, String, i32) {
    let test_file = unique_test_file("js_interp");
    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");

    let mut cmd = Command::new(ntnt_binary());
    cmd.args(args)
        .arg(&test_file)
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    for (k, v) in env_vars {
        cmd.env(k, v);
    }

    let output = cmd.output().expect("Failed to execute ntnt");
    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

fn lint(code: &str) -> (String, String, i32) {
    run_ntnt(&["lint"], code, &[])
}

fn lint_strict(code: &str) -> (String, String, i32) {
    run_ntnt(&["lint", "--strict"], code, &[])
}

fn run(code: &str, env_vars: &[(&str, &str)]) -> (String, String, i32) {
    run_ntnt(&["run"], code, env_vars)
}

const RULE: &str = "javascript_style_interpolation";

// ── Lint ───────────────────────────────────────────────────────────────

#[test]
fn lint_warns_when_interpolated_ident_is_in_scope() {
    let code = r#"let name = "world"
print("hello ${name}")"#;
    let (stdout, _, exit_code) = lint(code);
    assert!(
        stdout.contains(RULE),
        "expected {} finding, got: {}",
        RULE,
        stdout
    );
    assert!(
        stdout.contains("#{name}"),
        "hint should show the NTNT syntax #{{name}}: {}",
        stdout
    );
    assert!(
        stdout.contains("\"line\": 2"),
        "finding should point at line 2: {}",
        stdout
    );
    assert_eq!(exit_code, 0, "warnings must not fail default lint");
}

#[test]
fn lint_strict_promotes_to_error_and_fails() {
    let code = r#"fn greet(name: String) -> String {
    return "hello ${name}"
}
print(greet("world"))"#;
    let (stdout, _, exit_code) = lint_strict(code);
    assert!(stdout.contains(RULE), "expected {} in: {}", RULE, stdout);
    assert_eq!(
        exit_code, 1,
        "lint --strict must exit 1 on js-style interpolation (DD-063 acceptance criterion); stdout: {}",
        stdout
    );
}

#[test]
fn lint_ignores_out_of_scope_idents() {
    // Shell content: ${HOME} and ${1} are not NTNT variables in scope.
    let code = r#"let script = "echo ${HOME} ${1}"
print(script)"#;
    let (stdout, _, exit_code) = lint(code);
    assert!(
        !stdout.contains(RULE),
        "out-of-scope idents must not warn (shell false-positive guard): {}",
        stdout
    );
    assert_eq!(exit_code, 0);
}

#[test]
fn lint_ignores_template_strings() {
    // Template strings are for generated code; embedded JS `${...}` is legitimate.
    let code = r#"let b = 1
let s = """const x = `${b}`;"""
print("ok")"#;
    let (stdout, _, _) = lint(code);
    assert!(
        !stdout.contains(RULE),
        "template strings must never produce {}: {}",
        RULE,
        stdout
    );
}

#[test]
fn lint_mixed_interpolation_flags_only_js_part() {
    let code = r#"let a = 1
let b = 2
print("hi #{a} and ${b}")"#;
    let (stdout, _, _) = lint(code);
    let count = stdout.matches(RULE).count();
    assert_eq!(
        count, 1,
        "exactly one finding (for ${{b}} only), got {}: {}",
        count, stdout
    );
    assert!(stdout.contains("#{b}"), "hint should target b: {}", stdout);
}

#[test]
fn lint_reports_each_site_once() {
    // Expression statements are type-inferred twice internally
    // (check_statement + terminal-type inference) — findings must not double.
    let code = r#"let name = "world"
"hello ${name}"
print("ok")"#;
    let (stdout, _, _) = lint(code);
    let count = stdout.matches(RULE).count();
    assert_eq!(
        count, 1,
        "double-visited expression statement must report once, got {}: {}",
        count, stdout
    );
}

#[test]
fn lint_reports_each_site_once_inside_function_body() {
    // Block statements take the check_statement + infer_statement_terminal_type
    // double-visit path — findings must not double there either.
    let code = r#"fn f(name: String) -> String {
    "x ${name} y"
    return "done"
}
print(f("w"))"#;
    let (stdout, _, _) = lint(code);
    let count = stdout.matches(RULE).count();
    assert_eq!(
        count, 1,
        "double-visited block statement must report once, got {}: {}",
        count, stdout
    );
}

#[test]
fn lint_reports_distinct_sites_separately() {
    let code = r#"let name = "world"
let a = "first ${name}"
let b = "second ${name}"
print(a + b)"#;
    let (stdout, _, _) = lint(code);
    let count = stdout.matches(RULE).count();
    assert_eq!(
        count, 2,
        "two distinct sites of the same snippet must both report, got {}: {}",
        count, stdout
    );
    assert!(
        stdout.contains("\"line\": 2") && stdout.contains("\"line\": 3"),
        "findings should point at lines 2 and 3: {}",
        stdout
    );
}

#[test]
fn lint_supports_unicode_identifiers() {
    let code = "let naïve = \"x\"\nprint(\"h ${naïve}\")";
    let (stdout, _, _) = lint(code);
    assert!(
        stdout.contains(RULE) && stdout.contains("#{naïve}"),
        "unicode identifier in scope must be caught: {}",
        stdout
    );
}

#[test]
fn no_warn_for_builtin_and_function_names() {
    // Generated shell/JS content commonly references names that collide with
    // builtins, prelude, or user functions — those are not data variables and
    // must stay silent at lint AND runtime.
    let code = r#"fn helper(x: Int) -> Int { return x }
print("use ${len} and ${format} and ${helper}")"#;
    let (stdout, _, _) = lint(code);
    assert!(
        !stdout.contains(RULE),
        "function names must not trigger lint: {}",
        stdout
    );
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("use ${len} and ${format} and ${helper}"));
    assert!(
        !stderr.contains("[WARN]"),
        "function names must not trigger runtime warn: {}",
        stderr
    );
}

#[test]
fn run_warns_for_nested_inner_ident() {
    // `${PATH:-${name}}`: PATH is out of scope but the nested `${name}` is a
    // real in-scope reference and must still be caught.
    let code = r#"let name = "x"
print("echo ${PATH:-${name}} done")"#;
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("echo ${PATH:-${name}} done"));
    assert!(
        stderr.contains("[WARN]") && stderr.contains("#{name}"),
        "nested in-scope ident must warn: {}",
        stderr
    );
}

// ── Runtime ────────────────────────────────────────────────────────────

#[test]
fn run_warns_and_outputs_literally() {
    let code = r#"let name = "world"
print("hello ${name}")"#;
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("hello ${name}"),
        "the string is still output literally: {}",
        stdout
    );
    assert!(
        stderr.contains("[WARN]") && stderr.contains("#{name}"),
        "runtime should warn with the NTNT syntax: {}",
        stderr
    );
}

#[test]
fn run_warns_in_interpolated_string_literal_parts() {
    let code = r#"let a = 1
let b = 2
print("hi #{a} and ${b}")"#;
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("hi 1 and ${b}"), "stdout: {}", stdout);
    assert!(
        stderr.contains("[WARN]") && stderr.contains("#{b}"),
        "stderr: {}",
        stderr
    );
}

#[test]
fn run_warn_is_deduplicated_per_site() {
    let code = r#"let name = "world"
for i in 0..3 {
    print("hello ${name}")
}"#;
    let (_, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0);
    let count = stderr
        .lines()
        .filter(|l| l.contains("[WARN]") && l.contains("${name}"))
        .count();
    assert_eq!(
        count, 1,
        "one [WARN] per site, not per evaluation, got {}: {}",
        count, stderr
    );
}

#[test]
fn run_forgiving_mode_is_silent() {
    let code = r#"let name = "world"
print("hello ${name}")"#;
    let (stdout, stderr, exit_code) = run(code, &[("NTNT_TYPE_MODE", "forgiving")]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("hello ${name}"));
    assert!(
        !stderr.contains("[WARN]"),
        "forgiving mode must be silent: {}",
        stderr
    );
}

#[test]
fn run_no_warn_for_out_of_scope_idents() {
    let code = r#"print("path is ${UNDEFINED_THING}")"#;
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("path is ${UNDEFINED_THING}"));
    assert!(
        !stderr.contains("[WARN]"),
        "out-of-scope ident must not warn: {}",
        stderr
    );
}

#[test]
fn run_strict_type_mode_warns_but_still_executes() {
    // A literal string is type-valid: strict runtime must not turn the warn
    // into an error. The hard failure belongs to `lint --strict`.
    let code = r#"let name = "world"
print("hello ${name}")"#;
    let (stdout, stderr, exit_code) = run(code, &[("NTNT_TYPE_MODE", "strict")]);
    assert_eq!(exit_code, 0, "strict mode must not abort: {}", stderr);
    assert!(stdout.contains("hello ${name}"));
    assert!(stderr.contains("[WARN]"), "stderr: {}", stderr);
}

#[test]
fn run_strict_lint_mode_does_not_block_run_path() {
    let code = r#"let name = "world"
print("hello ${name}")"#;
    let (stdout, _, exit_code) = run(code, &[("NTNT_LINT_MODE", "strict")]);
    assert_eq!(
        exit_code, 0,
        "lint strict promotion applies to `ntnt lint`, not `ntnt run`"
    );
    assert!(stdout.contains("hello ${name}"));
}
