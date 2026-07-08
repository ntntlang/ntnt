//! Integration tests for std/validate (DD-058 item 1)
//!
//! Runs the compiled binary on .tnt programs exercising schema validation:
//! coercion, defaults, custom predicates, error maps, and shadowing.

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

fn run(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("validate");
    let mut file = fs::File::create(&test_file).expect("create test file");
    writeln!(file, "{}", code).expect("write test file");
    drop(file);

    let output = Command::new(ntnt_binary())
        .args(["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("execute ntnt");
    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn validate_coerces_and_cleans() {
    let code = r#"
import { schema, validate, required, email, min_value } from "std/validate"

let s = schema(map {
    "email": [required, email],
    "age": [required, int, min_value(13)]
})

match validate(s, map { "email": "a@b.co", "age": "25", "extra": "dropped" }) {
    Ok(clean) => {
        print(clean["age"] + 1)
        print(has_key(clean, "extra"))
    },
    Err(e) => print("unexpected: " + str(e))
}
"#;
    let (stdout, stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    // "25" + 1 would be a string concat; 26 proves int coercion
    assert!(
        stdout.contains("26"),
        "coerced int should be usable: {stdout}"
    );
    assert!(stdout.contains("false"), "unknown keys dropped: {stdout}");
}

#[test]
fn validate_reports_field_errors() {
    let code = r#"
import { schema, validate, required, email, min_value } from "std/validate"

let s = schema(map {
    "email": [required, email],
    "age": [required, int, min_value(13)]
})

match validate(s, map { "email": "nope", "age": "12" }) {
    Ok(_) => print("unexpected ok"),
    Err(errors) => {
        print(errors["email"])
        print(errors["age"])
    }
}
"#;
    let (stdout, stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("Must be a valid email address"), "{stdout}");
    assert!(stdout.contains("Must be at least 13"), "{stdout}");
}

#[test]
fn validate_missing_required_field() {
    let code = r#"
import { schema, validate, required } from "std/validate"

let s = schema(map { "name": [required] })
match validate(s, map {}) {
    Ok(_) => print("unexpected ok"),
    Err(errors) => print(errors["name"])
}
"#;
    let (stdout, _stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Required"), "{stdout}");
}

#[test]
fn validate_optional_and_default() {
    let code = r#"
import { schema, validate, optional, default, one_of } from "std/validate"

let s = schema(map {
    "nickname": [optional],
    "role": [default("user"), one_of(["admin", "user"])]
})
match validate(s, map {}) {
    Ok(clean) => {
        print(has_key(clean, "nickname"))
        print(clean["role"])
    },
    Err(e) => print("unexpected: " + str(e))
}
"#;
    let (stdout, _stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("false"),
        "optional absent is omitted: {stdout}"
    );
    assert!(stdout.contains("user"), "default applied: {stdout}");
}

#[test]
fn validate_custom_closure_predicate() {
    let code = r#"
import { schema, validate, required } from "std/validate"

let s = schema(map {
    "even": [required, fn(v) { v % 2 == 0 }],
    "named": [required, fn(v) { if v == "x" { return "No x allowed" } return true }]
})
match validate(s, map { "even": 3, "named": "x" }) {
    Ok(_) => print("unexpected ok"),
    Err(errors) => {
        print(errors["even"])
        print(errors["named"])
    }
}
match validate(s, map { "even": 4, "named": "y" }) {
    Ok(clean) => print("clean: " + str(clean["even"])),
    Err(e) => print("unexpected err: " + str(e))
}
"#;
    let (stdout, stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("Invalid value"), "{stdout}");
    assert!(stdout.contains("No x allowed"), "custom message: {stdout}");
    assert!(stdout.contains("clean: 4"), "{stdout}");
}

#[test]
fn validate_trim_transform_applies() {
    let code = r#"
import { schema, validate, required, min_length } from "std/validate"
import { trim } from "std/string"

let s = schema(map { "name": [required, trim, min_length(1)] })
match validate(s, map { "name": "  Alice  " }) {
    Ok(clean) => print("[" + clean["name"] + "]"),
    Err(e) => print("unexpected: " + str(e))
}
match validate(s, map { "name": "   " }) {
    Ok(_) => print("unexpected ok"),
    Err(errors) => print(errors["name"])
}
"#;
    let (stdout, _stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("[Alice]"), "trimmed: {stdout}");
    assert!(
        stdout.contains("Must be at least 1 characters"),
        "whitespace-only fails min_length after trim: {stdout}"
    );
}

#[test]
fn user_defined_validate_still_shadows_builtin() {
    let code = r#"
fn validate(a, b) {
    return "shadowed"
}
print(validate(1, 2))
"#;
    let (stdout, stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("shadowed"), "{stdout}");
}

#[test]
fn validate_works_with_raw_map_schema() {
    // schema() is recommended but validate accepts a raw rules map too
    let code = r#"
import { validate, required } from "std/validate"

match validate(map { "name": [required] }, map { "name": "ok" }) {
    Ok(clean) => print(clean["name"]),
    Err(e) => print("unexpected: " + str(e))
}
"#;
    let (stdout, _stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("ok"), "{stdout}");
}

#[test]
fn aliased_import_keeps_closure_support() {
    // Dispatch is on the resolved value, not the call-site name
    let code = r#"
import { validate as check, schema, required } from "std/validate"

let s = schema(map { "even": [required, fn(v) { v % 2 == 0 }] })
match check(s, map { "even": 4 }) {
    Ok(clean) => print("ok: " + str(clean["even"])),
    Err(e) => print("unexpected err: " + str(e))
}
let indirect = check
match indirect(s, map { "even": 3 }) {
    Ok(_) => print("unexpected ok"),
    Err(errors) => print(errors["even"])
}
"#;
    let (stdout, stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("ok: 4"), "aliased call: {stdout}");
    assert!(
        stdout.contains("Invalid value"),
        "let-bound call with closure rule: {stdout}"
    );
}

#[test]
fn matches_on_non_string_says_must_be_string() {
    let code = r#"
import { schema, validate, matches } from "std/validate"

let s = schema(map { "code": [matches("^[A-Z]+$")] })
match validate(s, map { "code": 42 }) {
    Ok(_) => print("unexpected ok"),
    Err(errors) => print(errors["code"])
}
"#;
    let (stdout, _stderr, exit_code) = run(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Must be a string"), "{stdout}");
}
