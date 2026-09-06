//! Exercise native Intent through the actual command, without a paired server.
use std::process::{Command, Output};

fn check(source: &str, outcome: &str, json: bool) -> Output {
    check_call(source, "sample()", outcome, json)
}

fn check_call(source: &str, call: &str, outcome: &str, json: bool) -> Output {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("helpers.tnt"), source).unwrap();
    std::fs::write(
        root.path().join("library.intent"),
        format!(
            "# Native testing\n\n## Glossary\n\n| Term | Means |\n|------|-------|\n| exercising the helper | call: {call}, source: helpers.tnt |\n\nFeature: Native helper\n  id: feature.native\n\n  Scenario: Selected helper\n    When exercising the helper\n    → {outcome}\n"
        ),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    cmd.args(["intent", "check"])
        .arg(root.path().join("library.intent"))
        .arg("-v");
    if json {
        cmd.arg("--json");
    }
    cmd.output().unwrap()
}

#[test]
fn native_async_callbacks_fail_closed_even_when_caught_or_aliased() {
    for source in [
        "import { parallel } from \"std/concurrent\"\nfn sample() { let result = parallel([fn() { assert(false) }])\n assert(is_err(result)) }",
        "import { spawn, await_task } from \"std/concurrent\"\nfn sample() { let start = spawn\n let task = start(fn() { assert(false) })\n await_task(task)\n assert(true) }",
        "import { parallel } from \"std/concurrent\"\nfn attempt() { let ignored = parallel([fn() { assert(false) }]) otherwise { return 0 }\n return 0 }\nfn sample() { attempt()\n assert(true) }",
    ] {
        for json in [false, true] {
            let output = check(source, "native assertions pass", json);
            assert!(!output.status.success(), "unsupported callback went green: {}", String::from_utf8_lossy(&output.stdout));
            assert!(String::from_utf8_lossy(&output.stdout).contains("Unsupported native test capability"));
            if json {
                let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(report["passed"], false);
            }
        }
    }
}

#[test]
fn native_assertion_entries_report_actual_work() {
    for (source, success) in [
        ("fn sample() { assert(true) }", true),
        ("fn sample() { assert(false) }", false),
        (
            "fn sample() { assert(true)\nreturn Err(\"deliberate error\") }",
            false,
        ),
        ("fn sample() { return 42 }", false),
    ] {
        for json in [false, true] {
            let output = check(source, "native assertions pass", json);
            assert_eq!(
                output.status.success(),
                success,
                "stdout={}\nstderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            if json {
                let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(
                    report["features"][0]["scenarios"][0]["status"],
                    if success { "pass" } else { "fail" }
                );
            }
        }
    }
}

#[test]
fn native_values_round_trip_without_string_coercion() {
    for literal in [
        r#""42""#,
        r#""{label}""#,
        r#""$label""#,
        "42",
        "42.0",
        "true",
        "false",
        "None",
        "Some(42)",
        r#"Err("expected")"#,
        r#"Ok(map { "items": [1, true, "x,y", None], "nested": Some(false) })"#,
    ] {
        let output = check_call(
            "fn sample(value) { return value }",
            &format!("sample({literal})"),
            &format!("result is {literal}"),
            true,
        );
        assert!(
            output.status.success(),
            "literal={literal}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    for (source, expected) in [
        (r#"fn sample() { return "42" }"#, "42"),
        ("fn sample() { return 42 }", r#""42""#),
        ("fn sample() { return Some(42) }", "42"),
        ("fn sample() { return 42.0 }", "42"),
    ] {
        let output = check(source, &format!("result is {expected}"), true);
        assert!(
            !output.status.success(),
            "different native types compared equal: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
}

#[test]
fn native_unresolved_given_is_not_descriptive_success() {
    for json in [false, true] {
        let output = check(
            "fn sample() { assert(true) }",
            "native assertions pass\n    Given an unprovisioned database",
            json,
        );
        assert!(
            !output.status.success(),
            "ignored Given became green: {}",
            String::from_utf8_lossy(&output.stdout)
        );
        if json {
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["passed"], false);
            assert_eq!(
                report["passed_assertions"], 0,
                "must not run native entry with unexecuted setup"
            );
        }
    }
}

#[test]
fn native_http_predicate_is_not_function_evidence() {
    for outcome in [
        "status 200",
        "header \"X-Test\" contains \"yes\"",
        "ends with \"...\" or equals original",
        "",
    ] {
        for json in [false, true] {
            let output = check(r#"fn sample() { return "short" }"#, outcome, json);
            assert!(
                !output.status.success(),
                "unsupported/empty predicate went green ({outcome}): {}",
                String::from_utf8_lossy(&output.stdout)
            );
        }
    }
}

fn write_suite(root: &std::path::Path, outcome: &str) -> std::path::PathBuf {
    let path = root.join("library.intent");
    std::fs::write(&path, format!("# Native\n\n## Glossary\n\n| Term | Means |\n|------|-------|\n| exercising the helper | call: sample(), source: helpers.tnt |\n\nFeature: Native\n  id: feature.native\n  Scenario: Selected helper\n    When exercising the helper\n    → {outcome}\n")).unwrap();
    path
}

fn run_suite(path: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["intent", "check"])
        .arg(path)
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn native_relative_imports_and_assertion_source_locations() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("nested")).unwrap();
    std::fs::write(
        root.path().join("helpers.tnt"),
        "import { verify } from \"./nested/child.tnt\"\nfn sample() { verify() }",
    )
    .unwrap();
    let child = root.path().join("nested/child.tnt");
    let suite = write_suite(root.path(), "native assertions pass");
    for (condition, success) in [("true", true), ("false", false)] {
        std::fs::write(
            &child,
            format!("// Import-safe module\nfn verify() {{\n    assert({condition})\n}}\n"),
        )
        .unwrap();
        let output = run_suite(&suite, &["--json"]);
        assert_eq!(
            output.status.success(),
            success,
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(
            text.contains("child.tnt:3"),
            "missing actual assertion location: {text}"
        );
    }
}

#[test]
fn native_case_selection_and_empty_runs_agree_with_json() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("helpers.tnt"),
        "fn sample() { assert(true) }",
    )
    .unwrap();
    let suite = write_suite(root.path(), "native assertions pass");
    for (name, success) in [("Selected helper", true), ("not present", false)] {
        for json in [false, true] {
            let mut args = vec!["--case", name];
            if json {
                args.push("--json");
            }
            let output = run_suite(&suite, &args);
            assert_eq!(
                output.status.success(),
                success,
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
            if json {
                let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
                assert_eq!(report["passed"], success);
                if !success {
                    assert_eq!(report["total_assertions"], 0);
                }
            }
        }
    }
    std::fs::write(
        &suite,
        "# Empty\n\nFeature: Unimplemented\n  id: feature.empty\n",
    )
    .unwrap();
    let output = run_suite(&suite, &["--json"]);
    assert!(!output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["passed"], false);
    assert_eq!(report["features"][0]["passed"], false);
}

#[test]
fn native_clean_environment_and_failure_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let recorded = root.path().join("observed-root.txt");
    let record_literal = serde_json::to_string(recorded.to_str().unwrap()).unwrap();
    let suite = write_suite(root.path(), "native assertions pass");
    for ending in [
        "assert(true)",
        "assert(false)",
        "return Err(\"deliberate\")",
    ] {
        std::fs::write(root.path().join("helpers.tnt"), format!("import {{ get_env, cwd }} from \"std/env\"\nimport {{ write_file }} from \"std/fs\"\nfn sample() {{\n  assert(is_none(get_env(\"DATABASE_URL\")))\n  assert(is_none(get_env(\"NTNT_PROCESS_ENABLE\")))\n  unwrap(write_file({record_literal}, cwd()))\n  unwrap(write_file(\"fixture.txt\", \"disposable\"))\n  {ending}\n}}\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .args(["intent", "check"])
            .arg(&suite)
            .arg("--json")
            .env("DATABASE_URL", "must-not-inherit")
            .env("NTNT_PROCESS_ENABLE", "1")
            .output()
            .unwrap();
        assert_eq!(
            output.status.success(),
            ending == "assert(true)",
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let fixture_root = std::fs::read_to_string(&recorded).unwrap();
        assert!(
            !std::path::Path::new(&fixture_root).exists(),
            "owned fixture directory leaked: {fixture_root}"
        );
    }
}

#[test]
fn native_missing_binding_autorun_and_server_actions_fail() {
    for source in [
        "fn different() { assert(true) }",
        "fn sample() { assert(true) }\nsample()",
        "listen(19234)\nfn sample() { assert(true) }",
    ] {
        let output = check(source, "native assertions pass", true);
        assert!(
            !output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["passed"], false);
    }
}

#[test]
fn simple_http_intent_remains_executable() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/simple_server/server.intent");
    for json in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port().to_string();
        drop(listener);
        let mut args = vec!["--port", port.as_str()];
        if json {
            args.push("--json");
        }
        let output = run_suite(&fixture, &args);
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if json {
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["passed"], true);
        }
    }
}

#[test]
fn native_table_preserves_quoted_numeric_strings() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("helpers.tnt"),
        "fn sample(value) { assert(value == \"42\") }",
    )
    .unwrap();
    let suite = root.path().join("library.intent");
    std::fs::write(&suite, "# Typed rows\n\n## Glossary\n\n| Term | Means |\n|------|-------|\n| exercising rows | call: sample({input}), source: helpers.tnt, input: test_data.values |\n\nFeature: Rows\n  id: feature.rows\n  Scenario: Quoted input\n    When exercising rows\n    → native assertions pass\n\nTest Cases: Values\n  id: test_data.values\n  | input |\n  | \"42\" |\n").unwrap();
    let output = run_suite(&suite, &["--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn checked_in_native_sqlite_example_runs_through_cli() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/native_testing/library.intent");
    let output = run_suite(&fixture, &["--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["summary"]["total_scenarios"], 3);
    assert_eq!(report["passed"], true);
}

#[test]
fn native_missing_data_cannot_fall_back_to_one_successful_call() {
    let output = check_call(
        "fn sample() { assert(true) }",
        "sample(), input: test_data.missing",
        "native assertions pass",
        true,
    );
    assert!(
        !output.status.success(),
        "missing data executed a fallback case: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn native_deadline_cleans_the_owned_fixture_root() {
    let root = tempfile::tempdir().unwrap();
    let record = root.path().join("observed.txt");
    let literal = serde_json::to_string(record.to_str().unwrap()).unwrap();
    std::fs::write(root.path().join("helpers.tnt"), format!("import {{ cwd }} from \"std/env\"\nimport {{ write_file }} from \"std/fs\"\nimport {{ sleep_ms }} from \"std/concurrent\"\nfn sample() {{ unwrap(write_file({literal}, cwd()))\n sleep_ms(60000)\n assert(true) }}")).unwrap();
    let suite = write_suite(root.path(), "native assertions pass");
    let output = run_suite(&suite, &["--json"]);
    assert!(!output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("timed out"), "{text}");
    let fixture_root = std::fs::read_to_string(record).unwrap();
    assert!(!std::path::Path::new(&fixture_root).exists());
}

#[test]
fn native_cli_defaults_to_strict_runtime_checks() {
    let output = check(
        "fn sample() { let xs = [1]\n let ignored = xs[9]\n assert(true) }",
        "native assertions pass",
        true,
    );
    assert!(
        !output.status.success(),
        "native child silently used warn mode: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn native_library_value_call_needs_no_server() {
    for json in [false, true] {
        let output = check("fn sample() -> Int { return 42 }\n", "result is 42", json);
        assert!(
            output.status.success(),
            "stdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if json {
            let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["failed_assertions"], 0);
            assert_eq!(report["features"][0]["scenarios"][0]["status"], "pass");
        } else {
            assert!(!String::from_utf8_lossy(&output.stdout).contains("Starting server"));
        }
    }
}
