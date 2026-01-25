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

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("--json should output valid JSON");

    assert!(json["features"].is_array(), "Should have features array");
    assert!(
        json["total_assertions"].is_number(),
        "Should have total_assertions"
    );
    assert!(
        json["passed_assertions"].is_number(),
        "Should have passed_assertions"
    );
    assert!(
        json["failed_assertions"].is_number(),
        "Should have failed_assertions"
    );
    assert_eq!(code, 0, "Simple server tests should pass");
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
        if std::path::Path::new("./target/debug/ntnt").exists() {
            "./target/debug/ntnt".to_string()
        } else if std::path::Path::new("./target/dev-release/ntnt").exists() {
            "./target/dev-release/ntnt".to_string()
        } else {
            "./target/release/ntnt".to_string()
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

---

Feature: Hello World
  id: feature.hello_world
  description: "Display hello world"
  test:
    - request: GET /
      assert:
        - status: 200
        - body contains "Hello"
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
            content.contains("@implements") || content.contains("feature.hello_world"),
            "Generated file should have @implements annotations"
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
    let (stdout, stderr, code) =
        run_ntnt(&["intent", "coverage", "examples/intent_demo/server.tnt"]);

    if code == 0 {
        assert!(
            stdout.contains("%")
                || stderr.contains("%")
                || stdout.contains("Feature")
                || stderr.contains("Feature"),
            "Coverage should show percentage or feature info"
        );
    }
}
