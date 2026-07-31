//! Integration tests for Intent Studio features
//!
//! Tests the Intent Studio server, app auto-start, hot-reload, and live test execution
//!
//! ## Test Organization
//!
//! 1. **Unit tests** (fast, no server) - Test HTML content directly
//! 2. **Integration tests** (slower, need server) - Test server endpoints
//!
//! The HTML is embedded at compile time via include_str!() in intent_studio_server.rs.
//! Unit tests can check HTML content without starting a server.

use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

fn validate_report_schema(instance: &serde_json::Value) -> Result<(), String> {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/verification/reports/schema-v1.json"))
            .map_err(|error| error.to_string())?;
    validate_schema_node(&schema, instance, &schema, "$")
}

fn validate_schema_node(
    schema: &serde_json::Value,
    instance: &serde_json::Value,
    root: &serde_json::Value,
    path: &str,
) -> Result<(), String> {
    if let Some(reference) = schema.get("$ref").and_then(serde_json::Value::as_str) {
        let pointer = reference
            .strip_prefix('#')
            .ok_or_else(|| format!("unsupported external schema reference {reference}"))?;
        let target = root
            .pointer(pointer)
            .ok_or_else(|| format!("unknown schema reference {reference}"))?;
        return validate_schema_node(target, instance, root, path);
    }
    if let Some(expected) = schema.get("const") {
        if instance != expected {
            return Err(format!(
                "{path}: expected constant {expected}, got {instance}"
            ));
        }
    }
    if let Some(values) = schema.get("enum").and_then(serde_json::Value::as_array) {
        if !values.contains(instance) {
            return Err(format!("{path}: {instance} is not in enum {values:?}"));
        }
    }
    if let Some(expected) = schema.get("type") {
        let matches = match expected {
            serde_json::Value::String(kind) => json_type_matches(instance, kind),
            serde_json::Value::Array(kinds) => kinds
                .iter()
                .filter_map(serde_json::Value::as_str)
                .any(|kind| json_type_matches(instance, kind)),
            _ => false,
        };
        if !matches {
            return Err(format!(
                "{path}: value {instance} has wrong type for {expected}"
            ));
        }
    }
    if let Some(minimum) = schema.get("minimum").and_then(serde_json::Value::as_f64) {
        if instance.as_f64().is_some_and(|value| value < minimum) {
            return Err(format!("{path}: value is below {minimum}"));
        }
    }
    if let Some(maximum) = schema.get("maximum").and_then(serde_json::Value::as_f64) {
        if instance.as_f64().is_some_and(|value| value > maximum) {
            return Err(format!("{path}: value is above {maximum}"));
        }
    }
    if let Some(minimum) = schema.get("minLength").and_then(serde_json::Value::as_u64) {
        if instance
            .as_str()
            .is_some_and(|value| value.chars().count() < minimum as usize)
        {
            return Err(format!("{path}: string is shorter than {minimum}"));
        }
    }
    if let Some(pattern) = schema.get("pattern").and_then(serde_json::Value::as_str) {
        let regex = regex::Regex::new(pattern)
            .map_err(|error| format!("{path}: invalid schema pattern {pattern:?}: {error}"))?;
        if !instance.as_str().is_some_and(|value| regex.is_match(value)) {
            return Err(format!(
                "{path}: value does not match schema pattern {pattern:?}"
            ));
        }
    }
    if let Some(object) = instance.as_object() {
        if let Some(required) = schema.get("required").and_then(serde_json::Value::as_array) {
            for field in required.iter().filter_map(serde_json::Value::as_str) {
                if !object.contains_key(field) {
                    return Err(format!("{path}: missing required field {field}"));
                }
            }
        }
        if let Some(properties) = schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            if schema.get("additionalProperties") == Some(&serde_json::Value::Bool(false)) {
                for field in object.keys() {
                    if !properties.contains_key(field) {
                        return Err(format!("{path}: unexpected field {field}"));
                    }
                }
            }
            for (field, value) in object {
                if let Some(property_schema) = properties.get(field) {
                    validate_schema_node(property_schema, value, root, &format!("{path}.{field}"))?;
                }
            }
        }
    }
    if let (Some(items), Some(array)) = (schema.get("items"), instance.as_array()) {
        for (index, value) in array.iter().enumerate() {
            validate_schema_node(items, value, root, &format!("{path}[{index}]"))?;
        }
        if schema.get("uniqueItems") == Some(&serde_json::Value::Bool(true)) {
            let mut seen = std::collections::BTreeSet::new();
            for value in array {
                if !seen.insert(value.to_string()) {
                    return Err(format!("{path}: duplicate array item {value}"));
                }
            }
        }
    }
    Ok(())
}

fn json_type_matches(value: &serde_json::Value, kind: &str) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "null" => value.is_null(),
        _ => false,
    }
}

#[cfg(unix)]
use std::os::unix::process::CommandExt;

// ============================================================================
// Unit Tests - HTML Content (No Server Required)
// ============================================================================

/// Get the embedded Intent Studio HTML for testing
fn get_studio_html() -> &'static str {
    include_str!("../src/intent_studio_lite.html")
}

#[test]
fn test_html_has_intent_studio_title() {
    let html = get_studio_html();
    assert!(
        html.contains("Intent Studio"),
        "HTML should contain Intent Studio title"
    );
}

#[test]
fn test_html_has_logo_class() {
    let html = get_studio_html();
    assert!(
        html.contains("class=\"logo\""),
        "HTML should have logo class element"
    );
}

#[test]
fn test_html_has_open_app_button() {
    let html = get_studio_html();
    assert!(
        html.contains("Open App"),
        "HTML should have Open App button text"
    );
    assert!(
        html.contains("openApp()"),
        "HTML should have openApp() function call"
    );
}

#[test]
fn test_html_has_run_tests_button() {
    let html = get_studio_html();
    assert!(
        html.contains("Run Tests") || html.contains("runTests"),
        "HTML should have Run Tests button"
    );
}

#[test]
fn test_html_has_pass_fail_indicators() {
    let html = get_studio_html();
    assert!(
        html.contains("pass") || html.contains("Pass") || html.contains("Passing"),
        "HTML should have pass indicator"
    );
    assert!(
        html.contains("fail") || html.contains("Fail") || html.contains("Failing"),
        "HTML should have fail indicator"
    );
}

#[test]
fn test_html_has_filter_chips() {
    let html = get_studio_html();
    assert!(
        html.contains("filter-chip") || html.contains("filter"),
        "HTML should have filter chips"
    );
    assert!(
        html.contains("Failing") || html.contains("fail"),
        "HTML should have Failing filter"
    );
    assert!(
        html.contains("Warning") || html.contains("warn"),
        "HTML should have Warnings filter"
    );
}

#[test]
fn test_html_has_search_functionality() {
    let html = get_studio_html();
    assert!(
        html.contains("search") || html.contains("Search"),
        "HTML should have search functionality"
    );
    assert!(
        html.contains("handleSearch") || html.contains("search-input"),
        "HTML should have search handler"
    );
}

#[test]
fn test_html_has_summary_stats() {
    let html = get_studio_html();
    assert!(
        html.contains("summary") || html.contains("stat"),
        "HTML should have summary/stats section"
    );
    assert!(
        html.contains("pass-count") || html.contains("Passing"),
        "HTML should have pass count"
    );
    assert!(
        html.contains("fail-count") || html.contains("Failing"),
        "HTML should have fail count"
    );
}

#[test]
fn test_html_has_health_bar() {
    let html = get_studio_html();
    assert!(
        html.contains("health-bar") || html.contains("health"),
        "HTML should have health bar visualization"
    );
}

#[test]
fn test_html_has_glossary_panel() {
    let html = get_studio_html();
    assert!(
        html.contains("glossary") || html.contains("Glossary"),
        "HTML should have glossary panel"
    );
}

#[test]
fn test_html_has_auto_refresh_toggle() {
    let html = get_studio_html();
    assert!(
        html.contains("auto-refresh")
            || html.contains("Auto-refresh")
            || html.contains("toggleAutoRefresh"),
        "HTML should have auto-refresh toggle"
    );
}

#[test]
fn test_html_has_toast_notifications() {
    let html = get_studio_html();
    assert!(
        html.contains("toast") || html.contains("showToast"),
        "HTML should have toast notification support"
    );
}

#[test]
fn test_html_fetches_from_api() {
    let html = get_studio_html();
    assert!(
        html.contains("fetch(") || html.contains("/run-tests") || html.contains("/api/"),
        "HTML should fetch from API endpoints"
    );
}

#[test]
fn test_html_is_valid_structure() {
    let html = get_studio_html();
    assert!(
        html.contains("<!doctype html>") || html.contains("<!DOCTYPE html>"),
        "Should have doctype"
    );
    assert!(html.contains("<html"), "Should have html tag");
    assert!(html.contains("<head>"), "Should have head tag");
    assert!(html.contains("<body>"), "Should have body tag");
    assert!(html.contains("</html>"), "Should close html tag");
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Helper to run ntnt command and capture output
///
/// Prefers debug binary since that's what `cargo test` rebuilds.
/// Set NTNT_TEST_BINARY env var to override.
fn run_ntnt(args: &[&str]) -> (String, String, i32) {
    let binary = std::env::var("NTNT_TEST_BINARY").ok().unwrap_or_else(|| {
        // Account for .exe extension on Windows
        let exe = std::env::consts::EXE_SUFFIX;
        let debug_path = format!("./target/debug/ntnt{}", exe);
        let dev_release_path = format!("./target/dev-release/ntnt{}", exe);
        let release_path = format!("./target/release/ntnt{}", exe);

        // Prefer debug binary since cargo test rebuilds it
        if std::path::Path::new(&debug_path).exists() {
            debug_path
        } else if std::path::Path::new(&dev_release_path).exists() {
            dev_release_path
        } else {
            release_path
        }
    });

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

/// Helper to start Intent Studio as a background process
fn start_intent_studio(intent_file: &str, studio_port: u16, app_port: u16) -> Child {
    let binary = std::env::var("NTNT_TEST_BINARY").ok().unwrap_or_else(|| {
        // Account for .exe extension on Windows
        let exe = std::env::consts::EXE_SUFFIX;
        let debug_path = format!("./target/debug/ntnt{}", exe);
        let dev_release_path = format!("./target/dev-release/ntnt{}", exe);
        let release_path = format!("./target/release/ntnt{}", exe);

        if std::path::Path::new(&debug_path).exists() {
            debug_path
        } else if std::path::Path::new(&dev_release_path).exists() {
            dev_release_path
        } else {
            release_path
        }
    });

    let mut cmd = Command::new(binary);
    cmd.args(&[
        "intent",
        "studio",
        intent_file,
        "--port",
        &studio_port.to_string(),
        "--app-port",
        &app_port.to_string(),
        "--no-open", // Don't open browser during tests
    ])
    .current_dir(env!("CARGO_MANIFEST_DIR"))
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    cmd.spawn().expect("Failed to start Intent Studio")
}

/// Kill a child process and all its descendants
fn kill_process_tree(child: &mut Child) {
    #[cfg(unix)]
    {
        let pid = child.id() as i32;
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }

    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }

    let _ = child.wait();
}

/// Check if running in CI environment
fn is_ci() -> bool {
    std::env::var("CI").is_ok() || std::env::var("GITHUB_ACTIONS").is_ok()
}

/// Skip test if running in CI (for flaky network tests)
macro_rules! skip_on_ci {
    () => {
        if is_ci() {
            eprintln!("Skipping test on CI - run locally for full coverage");
            return;
        }
    };
}

/// Helper to wait for a server to be ready
fn wait_for_server(url: &str, timeout_secs: u64) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs() < timeout_secs {
        if let Ok(response) = reqwest::blocking::get(url) {
            if response.status().is_success() {
                return true;
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    false
}

/// Helper to make HTTP GET request
fn http_get(url: &str) -> Result<(u16, String), String> {
    reqwest::blocking::get(url)
        .map(|r| {
            let status = r.status().as_u16();
            let body = r.text().unwrap_or_default();
            (status, body)
        })
        .map_err(|e| e.to_string())
}

// ============================================================================
// Integration Tests - Server Endpoints (Network Required)
// ============================================================================

/// Combined server test - starts one server and tests multiple endpoints
/// This is more efficient than starting a new server for each test
#[test]
fn test_intent_studio_server_endpoints() {
    skip_on_ci!();

    let studio_port = 13010;
    let app_port = 18090;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    // Wait for server to start
    thread::sleep(Duration::from_secs(2));

    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if !wait_for_server(&studio_url, 5) {
        kill_process_tree(&mut child);
        panic!("Intent Studio failed to start");
    }

    // Test 1: Main page serves HTML
    if let Ok((status, body)) = http_get(&studio_url) {
        assert_eq!(status, 200, "Main page should return 200");
        assert!(body.contains("<!doctype html>"), "Should serve HTML");
        assert!(
            body.contains("Intent Studio"),
            "Should have Intent Studio title"
        );
    } else {
        kill_process_tree(&mut child);
        panic!("Failed to fetch main page");
    }

    // Test 2: app-status endpoint
    let status_url = format!("http://127.0.0.1:{}/app-status", studio_port);
    if let Ok((status, body)) = http_get(&status_url) {
        assert_eq!(status, 200, "app-status should return 200");
        let json: serde_json::Value = serde_json::from_str(&body).expect("Should return JSON");
        assert!(json["running"].is_boolean(), "Should have 'running' field");
    }

    // Test 3: run-tests endpoint
    thread::sleep(Duration::from_secs(1)); // Give app time to start
    let tests_url = format!("http://127.0.0.1:{}/run-tests", studio_port);
    if let Ok((status, body)) = http_get(&tests_url) {
        assert_eq!(status, 200, "run-tests should return 200");
        let json: serde_json::Value = serde_json::from_str(&body).expect("Should return JSON");
        assert!(json["features"].is_array(), "Should have features array");
        assert!(
            json["total_assertions"].is_number(),
            "Should have total_assertions"
        );
    }

    // Test 4: API endpoints (new style)
    let api_status_url = format!("http://127.0.0.1:{}/api/app-status", studio_port);
    if let Ok((status, _)) = http_get(&api_status_url) {
        assert_eq!(status, 200, "API app-status should return 200");
    }

    kill_process_tree(&mut child);
}

#[test]
fn test_intent_studio_custom_ports() {
    skip_on_ci!();

    let studio_port = 14001;
    let app_port = 19001;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(2));

    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    let studio_ready = wait_for_server(&studio_url, 5);

    kill_process_tree(&mut child);

    assert!(
        studio_ready,
        "Intent Studio should start on custom port {}",
        studio_port
    );
}

// ============================================================================
// Intent Check Tests (Regression Tests - Run on CI)
// ============================================================================

#[test]
fn test_intent_check_json_flag() {
    let (stdout, _stderr, code) = run_ntnt(&[
        "intent",
        "check",
        "tests/fixtures/simple_server/server.tnt",
        "--port",
        "18096",
        "--json",
    ]);

    assert!(
        stdout.trim_start().starts_with('{'),
        "JSON must be banner-free: {stdout}"
    );
    assert!(!stdout.contains("NTNT Intent Check"), "{stdout}");
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json should output valid JSON");
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/verification/reports/schema-v1.json"))
            .expect("committed report schema must be valid JSON");

    assert_eq!(json["schema"], schema["$id"]);
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["profile"], "legacy-live");
    assert_eq!(json["profile_qualification"], "profile:legacy-live");
    assert!(json["coverage"]["verified"]["covered"].is_number());
    assert!(json["coverage"]["required_bindings"]["total"].is_number());
    assert!(json["evidence"].is_array());
    assert_eq!(json["exit"]["code"], code);
    validate_report_schema(&json).expect("check JSON must validate against committed schema");
    assert_eq!(code, 0, "Simple server tests should pass: {stdout}");
}

#[test]
fn test_intent_check_actually_passes() {
    let (stdout, stderr, code) = run_ntnt(&[
        "intent",
        "check",
        "tests/fixtures/simple_server/server.tnt",
        "--port",
        "18091",
    ]);
    let output = format!("{}{}", stdout, stderr);

    assert_eq!(
        code, 0,
        "intent check should pass on simple_server fixture.\nOutput:\n{}",
        output
    );

    assert!(
        output.contains("passed") && output.contains("0 failed"),
        "Should show passing tests with 0 failures.\nOutput:\n{}",
        output
    );
}

#[test]
fn test_intent_check_fails_on_bad_assertions() {
    let temp_dir = std::env::temp_dir();
    let test_tnt = temp_dir
        .join("test_fail_check.tnt")
        .to_string_lossy()
        .to_string();
    let test_intent = temp_dir
        .join("test_fail_check.intent")
        .to_string_lossy()
        .to_string();

    fs::write(
        &test_tnt,
        r#"
import { html } from "std/http/server"

fn handler(req) {
    return html("<html><body>hello</body></html>")
}

get("/", handler)
listen(8080)
"#,
    )
    .unwrap();

    fs::write(
        &test_intent,
        r#"# Failing Test
## Overview
Test that should fail.

---

## Glossary

| Term | Means |
|------|-------|
| a user visits $path | GET $path |
| they see {text} | body contains {text} |

---

Feature: Bad Test
  id: feature.bad_test
  description: "This should fail"

  Scenario: Wrong content
    description: "Expects goodbye but gets hello"
    When a user visits /
    → status 200
    → they see "goodbye"
"#,
    )
    .unwrap();

    let (stdout, stderr, code) = run_ntnt(&["intent", "check", &test_tnt, "--port", "18092"]);

    fs::remove_file(&test_tnt).ok();
    fs::remove_file(&test_intent).ok();

    let output = format!("{}{}", stdout, stderr);

    assert_ne!(
        code, 0,
        "intent check should fail when assertions don't match.\nOutput:\n{}",
        output
    );
}

#[test]
fn test_intent_check_projects_legacy_test_blocks_into_the_report_ledger() {
    let fixture_stem = format!("ntnt_legacy_report_{}", std::process::id());
    let test_tnt = std::env::temp_dir().join(format!("{fixture_stem}.tnt"));
    let test_intent = std::env::temp_dir().join(format!("{fixture_stem}.intent"));
    fs::write(
        &test_tnt,
        r#"
import { html } from "std/http/server"

// @implements: feature.legacy-report
fn handler(req) {
    return html("<html><body>legacy ok</body></html>")
}

get("/", handler)
listen(8080)
"#,
    )
    .unwrap();
    let write_intent = |expected: &str| {
        fs::write(
            &test_intent,
            format!(
                r#"Feature: Legacy report
  id: feature.legacy-report
  test:
    - request: GET /
      assert:
        - status: 200
        - body contains "{expected}"
"#
            ),
        )
        .unwrap();
    };
    let tnt_path = test_tnt.to_string_lossy().to_string();

    write_intent("legacy ok");
    let (passing_stdout, passing_stderr, passing_code) =
        run_ntnt(&["intent", "check", &tnt_path, "--port", "18093", "--json"]);
    assert_eq!(
        passing_code, 0,
        "passing legacy test block must verify:\n{passing_stdout}\n{passing_stderr}"
    );
    let passing: serde_json::Value =
        serde_json::from_str(&passing_stdout).expect("passing report JSON");
    validate_report_schema(&passing).expect("passing compatibility report must match schema");
    assert_eq!(passing["coverage"]["required_bindings"]["total"], 2);
    assert_eq!(passing["coverage"]["required_bindings"]["covered"], 2);
    assert!(passing["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .all(|result| {
            result["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("binding.compat.feature.legacy-report.test.1."))
                && result["profile"] == "legacy-live"
        }));

    write_intent("never returned");
    let (failing_stdout, failing_stderr, failing_code) =
        run_ntnt(&["intent", "check", &tnt_path, "--port", "18094", "--json"]);
    fs::remove_file(&test_tnt).ok();
    fs::remove_file(&test_intent).ok();

    assert_ne!(
        failing_code, 0,
        "failing legacy test block must remain selected evidence:\n{failing_stdout}\n{failing_stderr}"
    );
    let failing: serde_json::Value =
        serde_json::from_str(&failing_stdout).expect("failing report JSON");
    validate_report_schema(&failing).expect("failing compatibility report must match schema");
    assert_eq!(failing["exit"]["code"], failing_code);
    assert!(failing["evidence"]
        .as_array()
        .unwrap()
        .iter()
        .any(|result| {
            result["disposition"] == "Failed"
                && result["obligation_id"].as_str().is_some_and(|id| {
                    id.starts_with("outcome.compat.feature.legacy-report.test.1.")
                })
        }));
}

#[test]
fn test_async_server_respects_listen_port_env_var() {
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir
        .join("test_port_env.tnt")
        .to_string_lossy()
        .to_string();

    fs::write(
        &test_file,
        r#"
import { json } from "std/http/server"

fn handler(req) {
    return json(map { "port_test": "ok" })
}

get("/", handler)
listen(8080)
"#,
    )
    .unwrap();

    let test_port = 19876;
    let binary = std::env::var("NTNT_TEST_BINARY").ok().unwrap_or_else(|| {
        // Account for .exe extension on Windows
        let exe = std::env::consts::EXE_SUFFIX;
        let debug_path = format!("./target/debug/ntnt{}", exe);
        let dev_release_path = format!("./target/dev-release/ntnt{}", exe);
        let release_path = format!("./target/release/ntnt{}", exe);

        if std::path::Path::new(&debug_path).exists() {
            debug_path
        } else if std::path::Path::new(&dev_release_path).exists() {
            dev_release_path
        } else {
            release_path
        }
    });

    let mut cmd = Command::new(&binary);
    cmd.args(&["run", &test_file])
        .env("NTNT_LISTEN_PORT", test_port.to_string())
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(unix)]
    cmd.process_group(0);

    let child = cmd.spawn();

    if let Ok(mut child) = child {
        thread::sleep(Duration::from_secs(2));

        let correct_url = format!("http://127.0.0.1:{}", test_port);
        let on_correct_port = wait_for_server(&correct_url, 3);

        kill_process_tree(&mut child);
        fs::remove_file(&test_file).ok();

        assert!(
            on_correct_port,
            "Server should listen on NTNT_LISTEN_PORT ({})",
            test_port
        );
    } else {
        fs::remove_file(&test_file).ok();
        panic!("Failed to start ntnt process");
    }
}

// ============================================================================
// CLI Help Tests
// ============================================================================

#[test]
fn test_intent_subcommand_help() {
    let (stdout, stderr, _) = run_ntnt(&["intent", "--help"]);
    let output = format!("{}{}", stdout, stderr);

    assert!(
        output.contains("studio") || output.contains("Studio"),
        "Help should mention studio"
    );
    assert!(
        output.contains("check") || output.contains("Check"),
        "Help should mention check"
    );
    assert!(
        output.contains("init") || output.contains("Init"),
        "Help should mention init"
    );
}

#[test]
fn test_intent_studio_help() {
    let (stdout, stderr, _) = run_ntnt(&["intent", "studio", "--help"]);
    let output = format!("{}{}", stdout, stderr);

    assert!(
        output.contains("port") || output.contains("PORT"),
        "Help should mention port option"
    );
    assert!(
        output.contains("app-port") || output.contains("APP"),
        "Help should mention app-port option"
    );
}

#[test]
fn test_default_ports_documented() {
    let (stdout, stderr, _) = run_ntnt(&["intent", "studio", "--help"]);
    let output = format!("{}{}", stdout, stderr);

    assert!(
        output.contains("3001") || output.contains("default"),
        "Should document default studio port"
    );
    assert!(
        output.contains("8081") || output.contains("default"),
        "Should document default app port"
    );
}

// ============================================================================
// Intent File Format Tests
// ============================================================================

#[test]
fn test_intent_init_generates_stub() {
    let temp_dir = std::env::temp_dir();
    let test_intent = temp_dir
        .join("test_init.intent")
        .to_string_lossy()
        .to_string();
    let test_tnt = temp_dir.join("test_init.tnt").to_string_lossy().to_string();

    fs::write(
        &test_intent,
        r#"# Test Project
# A simple test

## Overview
Test project for init command.

## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| they see {text} | body contains {text} |
| success response | status 200 |

---

Feature: Hello World
  id: feature.hello_world
  description: "Display hello world"

  Scenario: View hello world
    When a user visits /
    → success response
    → they see "Hello"
"#,
    )
    .unwrap();

    let (_stdout, _stderr, code) = run_ntnt(&["intent", "init", &test_intent, "-o", &test_tnt]);

    fs::remove_file(&test_intent).ok();

    if code == 0 {
        assert!(
            fs::metadata(&test_tnt).is_ok(),
            "Should create output .tnt file"
        );

        let content = fs::read_to_string(&test_tnt).unwrap_or_default();
        assert!(
            content.contains("Feature: Hello World") || content.contains("feature.hello_world"),
            "Generated file should reference the feature"
        );

        fs::remove_file(&test_tnt).ok();
    }
}

#[test]
fn test_intent_check_valid_file() {
    let (stdout, stderr, code) = run_ntnt(&["intent", "check", "examples/intent_demo/server.tnt"]);
    let output = format!("{}{}", stdout, stderr);

    assert!(
        code == 0 || code == 1 || output.contains("Intent file not found"),
        "intent check should work, have test failures, or report missing intent file"
    );
}

#[test]
fn test_intent_coverage_command() {
    let (stdout, stderr, code) = run_ntnt(&[
        "intent",
        "coverage",
        "examples/intent_demo/server.tnt",
        "--json",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.trim_start().starts_with('{'), "{stdout}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid report JSON");
    assert_eq!(json["schema_version"], 1);
    assert_eq!(json["profile"], "implementation");
    assert!(json["coverage"]["implementation"]["total"].is_number());
    assert!(json["coverage"]["executable"]["total"].is_number());
    assert!(json["coverage"]["verified"]["total"].is_number());
    validate_report_schema(&json).expect("coverage JSON must validate against committed schema");
}

#[test]
fn intent_coverage_threshold_flags_share_the_report_exit_decision() {
    let (stdout, stderr, code) = run_ntnt(&[
        "intent",
        "coverage",
        "examples/intent_demo/server.tnt",
        "--json",
        "--min-verified",
        "100",
    ]);
    assert_eq!(code, 1, "{stderr}");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid report JSON");
    assert_eq!(json["thresholds"]["verified"], 100.0);
    assert_eq!(json["exit"]["code"], code);
    assert_eq!(json["exit"]["reason"], "threshold-failed");
    validate_report_schema(&json).expect("threshold JSON must validate against committed schema");
}

// ============================================================================
// ntnt intent lint (DD-063 Rec 4b)
// ============================================================================

/// Write an intent file to a unique temp path, run `ntnt intent lint` on it
/// (plus extra args), clean up, and return (stdout, stderr, exit_code).
fn run_intent_lint(content: &str, extra_args: &[&str]) -> (String, String, i32) {
    use std::io::Write as _;
    static LINT_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let counter = LINT_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "ntnt_intent_lint_{}_{}.intent",
        std::process::id(),
        counter
    ));
    let mut file = std::fs::File::create(&path).expect("create intent file");
    write!(file, "{}", content).expect("write intent file");
    drop(file);

    let path_str = path.to_string_lossy().to_string();
    let mut args = vec!["intent", "lint", path_str.as_str()];
    args.extend_from_slice(extra_args);
    let result = run_ntnt(&args);

    std::fs::remove_file(&path).ok();
    result
}

const CLEAN_INTENT: &str = r#"## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the homepage | / |
| they see {text} | body contains {text} |
| success response | status 200 |

---

Feature: Home Page
  id: feature.home

  Scenario: First visit
    When a user visits the homepage
    → success response
    → they see "Welcome"
"#;

#[test]
fn intent_lint_clean_file_exits_zero() {
    let (stdout, _stderr, exit_code) = run_intent_lint(CLEAN_INTENT, &[]);
    assert_eq!(exit_code, 0, "clean intent should pass: {stdout}");
    assert!(stdout.contains("No issues found"), "{stdout}");
}

#[test]
fn intent_lint_unknown_term_exits_one_with_suggestion() {
    let content = CLEAN_INTENT.replace("→ success response", "→ succes response");
    let (stdout, _stderr, exit_code) = run_intent_lint(&content, &[]);
    assert_eq!(exit_code, 1, "unresolved term should fail: {stdout}");
    assert!(stdout.contains("unresolved_term"), "{stdout}");
    assert!(
        stdout.contains("did you mean") && stdout.contains("success response"),
        "should suggest the near-miss glossary term: {stdout}"
    );
}

#[test]
fn intent_lint_unresolved_when_clause_exits_one() {
    let content = CLEAN_INTENT.replace(
        "When a user visits the homepage",
        "When a user creates a task",
    );
    let (stdout, _stderr, exit_code) = run_intent_lint(&content, &[]);
    assert_eq!(exit_code, 1, "{stdout}");
    assert!(stdout.contains("unresolved_when"), "{stdout}");
}

#[test]
fn intent_lint_detects_glossary_cycle() {
    let content = r#"## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the homepage | / |
| term alpha | term beta |
| term beta | term alpha |

---

Feature: Home Page
  id: feature.home

  Scenario: First visit
    When a user visits the homepage
    → term alpha
"#;
    let (stdout, _stderr, exit_code) = run_intent_lint(content, &[]);
    assert_eq!(exit_code, 1, "{stdout}");
    assert!(stdout.contains("cycle"), "{stdout}");
    assert!(stdout.contains("term alpha"), "{stdout}");
}

#[test]
fn intent_lint_orphan_term_warns_but_exits_zero() {
    let content = CLEAN_INTENT.replace(
        "| success response | status 200 |",
        "| success response | status 200 |\n| never used term | status 418 |",
    );
    let (stdout, _stderr, exit_code) = run_intent_lint(&content, &[]);
    assert_eq!(exit_code, 0, "orphans are warnings only: {stdout}");
    assert!(stdout.contains("orphan_term"), "{stdout}");
    assert!(stdout.contains("never used term"), "{stdout}");
}

#[test]
fn intent_lint_json_output_shape() {
    let content = CLEAN_INTENT.replace("→ success response", "→ succes response");
    let (stdout, _stderr, exit_code) = run_intent_lint(&content, &["--json"]);
    assert_eq!(exit_code, 1);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert!(json["errors"].is_array());
    assert!(json["warnings"].is_array());
    assert_eq!(json["errors"][0]["kind"], "unresolved_term");
    assert!(json["errors"][0]["suggestions"].is_array());
    assert!(json["scenarios_checked"].is_number());
}

#[test]
fn intent_lint_reports_each_cycle_exactly_once() {
    // A two-term cycle must yield ONE finding, not one per participant —
    // and not once more via the scenario that references it
    let content = r#"## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the homepage | / |
| term alpha | term beta |
| term beta | term alpha |

---

Feature: Home Page
  id: feature.home

  Scenario: First visit
    When a user visits the homepage
    → term alpha
"#;
    let (stdout, _stderr, exit_code) = run_intent_lint(content, &["--json"]);
    assert_eq!(exit_code, 1);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let cycle_count = json["errors"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "cycle")
        .count();
    assert_eq!(
        cycle_count, 1,
        "cycle reported {cycle_count} times: {stdout}"
    );
}

#[test]
fn intent_lint_json_suggestions_is_empty_array_when_no_near_miss() {
    // Cycle findings carry no suggestions — the key must still be present
    let content = r#"## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the homepage | / |
| term alpha | term beta |
| term beta | term alpha |

---

Feature: Home Page
  id: feature.home

  Scenario: First visit
    When a user visits the homepage
    → term alpha
"#;
    let (stdout, _stderr, _exit) = run_intent_lint(content, &["--json"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let cycle = json["errors"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "cycle")
        .expect("cycle finding present");
    assert!(
        cycle["suggestions"].is_array() && cycle["suggestions"].as_array().unwrap().is_empty(),
        "suggestions must serialize as an empty array: {stdout}"
    );
}

#[test]
fn intent_lint_warns_on_vacuous_scenario() {
    // A scenario with no outcome lines verifies nothing — warn, don't fail
    let content = r#"## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the homepage | / |

---

Feature: Home Page
  id: feature.home

  Scenario: First visit
    When a user visits the homepage
"#;
    let (stdout, _stderr, exit_code) = run_intent_lint(content, &[]);
    assert_eq!(exit_code, 0, "vacuous scenarios are warnings: {stdout}");
    assert!(stdout.contains("vacuous_scenario"), "{stdout}");
    assert!(
        stdout.contains("passes vacuously") || stdout.contains("pass vacuously"),
        "{stdout}"
    );
}

#[test]
fn intent_lint_warns_on_vacuous_component_scenario() {
    let content = r#"## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |

---

Component: Error Popup
  id: component.error_popup

  Scenario: Popup appears
    When a user visits /error
"#;
    let (stdout, _stderr, exit_code) = run_intent_lint(content, &[]);
    assert_eq!(exit_code, 0, "{stdout}");
    assert!(
        stdout.contains("vacuous_scenario") && stdout.contains("a user visits /error"),
        "component scenarios need the same guard: {stdout}"
    );
    assert!(
        !stdout.contains("orphan_term"),
        "component-scenario clauses count as term usage: {stdout}"
    );
}

#[test]
fn intent_lint_accepts_tnt_path_with_paired_intent() {
    use std::io::Write as _;
    let dir = std::env::temp_dir().join(format!("ntnt_lint_pair_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let intent_path = dir.join("app.intent");
    let tnt_path = dir.join("app.tnt");
    write!(
        std::fs::File::create(&intent_path).unwrap(),
        "{}",
        CLEAN_INTENT
    )
    .unwrap();
    write!(
        std::fs::File::create(&tnt_path).unwrap(),
        "get(\"/\", fn(req) {{ \"Welcome\" }})\nlisten(8080)\n"
    )
    .unwrap();

    let tnt_str = tnt_path.to_string_lossy().to_string();
    let (stdout, _stderr, exit_code) = run_ntnt(&["intent", "lint", &tnt_str]);

    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(exit_code, 0, "should locate paired .intent: {stdout}");
    assert!(stdout.contains("app.intent"), "{stdout}");
}
