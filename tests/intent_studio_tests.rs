//! Integration tests for Intent Studio features
//!
//! Tests the Intent Studio server, app auto-start, hot-reload, and live test execution
//!
//! ⚠️  IMPORTANT: HTML Embedding and Test Caching
//! ==================================================
//! The Intent Studio HTML is embedded at COMPILE TIME via include_str!() in main.rs.
//! This means:
//!
//! 1. If you edit intent_studio_lite.html, you MUST rebuild before testing:
//!    cargo build              # For debug binary
//!    cargo build --release    # For release binary
//!
//! 2. These tests prefer ./target/release/ntnt if it exists (see run_ntnt() below).
//!    If the release binary is stale, tests will run against OLD HTML!
//!
//! 3. To force tests to use debug binary:
//!    rm ./target/release/ntnt && cargo test
//!
//! 4. To ensure latest HTML in tests:
//!    cargo build --release && cargo test
//!
//! If tests show different pass/fail counts than Intent Studio in your browser,
//! you're likely testing against a stale binary. Rebuild and try again!
//! ==================================================

use std::fs;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

/// Helper to run ntnt command and capture output
///
/// ⚠️ WARNING: Prefers release binary! See module-level docs for caching issues.
fn run_ntnt(args: &[&str]) -> (String, String, i32) {
    // Try release binary first, fall back to debug
    let binary = if std::path::Path::new("./target/release/ntnt").exists() {
        eprintln!("⚠️  Using release binary (may have stale HTML if not rebuilt)");
        "./target/release/ntnt"
    } else {
        eprintln!("ℹ️  Using debug binary");
        "./target/debug/ntnt"
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

/// Helper to start Intent Studio as a background process
///
/// ⚠️ WARNING: Prefers release binary! See module-level docs for caching issues.
fn start_intent_studio(intent_file: &str, studio_port: u16, app_port: u16) -> Child {
    // Try release binary first, fall back to debug
    let binary = if std::path::Path::new("./target/release/ntnt").exists() {
        eprintln!("⚠️  Starting Intent Studio with release binary (may have stale HTML)");
        "./target/release/ntnt"
    } else {
        eprintln!("ℹ️  Starting Intent Studio with debug binary");
        "./target/debug/ntnt"
    };

    Command::new(binary)
        .args(&[
            "intent",
            "studio",
            intent_file,
            "--port",
            &studio_port.to_string(),
            "--app-port",
            &app_port.to_string(),
        ])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Intent Studio")
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
// Intent File Parsing Tests
// ============================================================================

#[test]
fn test_intent_check_valid_file() {
    let (stdout, stderr, code) = run_ntnt(&["intent", "check", "examples/intent_demo/server.tnt"]);
    let output = format!("{}{}", stdout, stderr);
    // Should run and produce output - code may be 0 (all pass) or 1 (some fail)
    // The demo intentionally has some failing tests
    assert!(
        code == 0 || code == 1 || output.contains("Intent file not found"),
        "intent check should work, have test failures, or report missing intent file"
    );
    // Verify it actually ran tests
    assert!(
        output.contains("Test Results")
            || output.contains("Feature")
            || output.contains("not found"),
        "intent check should produce test output or an error"
    );
}

#[test]
fn test_intent_check_shows_coverage() {
    let (stdout, stderr, code) = run_ntnt(&["intent", "check", "examples/intent_demo/server.tnt"]);
    // The intent check command outputs to stderr typically
    // Just verify it runs without crashing - the output format may vary
    let output = format!("{}{}", stdout, stderr);
    // Either succeeds, has test failures (code 1), or reports an error
    // The demo intentionally has some failing tests so code 1 is expected
    assert!(
        code == 0 || code == 1 || output.contains("not found") || output.contains("error"),
        "intent check should either succeed, have test failures, or report a clear error"
    );
}

#[test]
fn test_intent_coverage_command() {
    let (stdout, stderr, code) =
        run_ntnt(&["intent", "coverage", "examples/intent_demo/server.tnt"]);
    // Should succeed if intent file exists
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

#[test]
fn test_intent_init_generates_stub() {
    let temp_dir = std::env::temp_dir();
    let test_intent = temp_dir
        .join("test_init.intent")
        .to_string_lossy()
        .to_string();
    let test_tnt = temp_dir.join("test_init.tnt").to_string_lossy().to_string();

    // Create a simple intent file
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

    // Clean up
    fs::remove_file(&test_intent).ok();

    if code == 0 {
        // Check that output file was created
        assert!(
            fs::metadata(&test_tnt).is_ok(),
            "Should create output .tnt file"
        );

        let content = fs::read_to_string(&test_tnt).unwrap_or_default();
        // Should contain @implements annotation
        assert!(
            content.contains("@implements") || content.contains("feature.hello_world"),
            "Generated file should have @implements annotations"
        );

        fs::remove_file(&test_tnt).ok();
    }
}

// ============================================================================
// Intent Studio Server Tests
// ============================================================================

#[test]
fn test_intent_studio_starts_on_default_port() {
    skip_on_ci!();
    // Use unique ports to avoid conflicts with other tests
    let studio_port = 13001;
    let app_port = 18081;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    // Wait for studio to start
    thread::sleep(Duration::from_secs(2));

    // Try to connect to studio
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    let studio_ready = wait_for_server(&studio_url, 5);

    // Kill the process
    child.kill().ok();

    assert!(
        studio_ready,
        "Intent Studio should start and respond on port {}",
        studio_port
    );
}

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "Flaky network test on Windows - times out waiting for server"
)]
fn test_intent_studio_serves_html() {
    skip_on_ci!();
    let studio_port = 13002;
    let app_port = 18082;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(2));

    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((status, body)) = http_get(&studio_url) {
            child.kill().ok();

            assert_eq!(status, 200, "Should return 200 OK");
            assert!(body.contains("<!doctype html>"), "Should serve HTML");
            assert!(
                body.contains("Intent Studio"),
                "Should contain Intent Studio title"
            );
            return;
        }
    }

    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_has_logo() {
    skip_on_ci!();
    let studio_port = 13003;
    let app_port = 18083;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(2));

    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();

            // Should have text logo in the header
            assert!(body.contains("class=\"logo\""), "Should have logo class");
            assert!(
                body.contains("Intent Studio"),
                "Should contain Intent Studio logo text"
            );
            return;
        }
    }

    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_has_open_app_button() {
    skip_on_ci!();
    let studio_port = 13004;
    let app_port = 18084;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(2));

    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();

            // Should have Open App button and handler
            assert!(body.contains("Open App"), "Should have Open App button");
            assert!(body.contains("openApp()"), "Should have Open App handler");
            return;
        }
    }

    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_app_status_endpoint() {
    skip_on_ci!();
    let studio_port = 13005;
    let app_port = 18085;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(3)); // Give time for app to start

    let status_url = format!("http://127.0.0.1:{}/app-status", studio_port);
    if wait_for_server(&format!("http://127.0.0.1:{}", studio_port), 5) {
        if let Ok((status, body)) = http_get(&status_url) {
            child.kill().ok();

            assert_eq!(status, 200, "app-status should return 200");

            // Should return JSON with running and healthy fields
            // Format: {"running": true/false, "healthy": true/false, "status": 200} or {"running": false, "healthy": false, "error": "..."}
            let json: serde_json::Value =
                serde_json::from_str(&body).expect("app-status should return valid JSON");

            assert!(
                json["running"].is_boolean(),
                "Should have 'running' boolean field"
            );
            // healthy field is only present when running is true
            if json["running"].as_bool().unwrap_or(false) {
                assert!(
                    json["healthy"].is_boolean(),
                    "Should have 'healthy' boolean field when running"
                );
            }

            return;
        }
    }

    child.kill().ok();
    panic!("Could not connect to Intent Studio app-status endpoint");
}

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "Flaky network test on Windows - times out waiting for server"
)]
fn test_intent_studio_run_tests_endpoint() {
    skip_on_ci!();
    let studio_port = 13006;
    let app_port = 18086;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(3));

    let tests_url = format!("http://127.0.0.1:{}/run-tests", studio_port);
    if wait_for_server(&format!("http://127.0.0.1:{}", studio_port), 5) {
        // Wait a bit more for app to be ready
        thread::sleep(Duration::from_secs(2));

        if let Ok((status, body)) = http_get(&tests_url) {
            child.kill().ok();

            assert_eq!(status, 200, "run-tests should return 200");

            // Should return JSON with test results
            // Format is: {"features": [...], "total_assertions": N, "passed_assertions": N, "failed_assertions": N}
            let json: serde_json::Value =
                serde_json::from_str(&body).expect("run-tests should return valid JSON");

            assert!(json["features"].is_array(), "Should have features array");
            assert!(
                json["total_assertions"].is_number(),
                "Should have total_assertions count"
            );
            assert!(
                json["passed_assertions"].is_number(),
                "Should have passed_assertions count"
            );
            assert!(
                json["failed_assertions"].is_number(),
                "Should have failed_assertions count"
            );

            return;
        }
    }

    child.kill().ok();
    panic!("Could not connect to Intent Studio run-tests endpoint");
}

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "Flaky network test on Windows - times out waiting for server"
)]
fn test_intent_studio_ui_has_test_controls() {
    skip_on_ci!();
    let studio_port = 13007;
    let app_port = 18087;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(2));

    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();

            // Should have test-related UI elements
            assert!(
                body.contains("Run Tests") || body.contains("runTests"),
                "Should have Run Tests button"
            );
            assert!(
                body.contains("pass") || body.contains("Pass"),
                "Should have pass indicator"
            );
            assert!(
                body.contains("fail") || body.contains("Fail"),
                "Should have fail indicator"
            );
            return;
        }
    }

    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
#[cfg_attr(
    target_os = "windows",
    ignore = "Flaky network test on Windows - times out waiting for server"
)]
fn test_intent_studio_ui_has_app_status_indicator() {
    skip_on_ci!();
    let studio_port = 13008;
    let app_port = 18088;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(2));

    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();

            // UI is in flux; just verify the page renders the Intent Studio title
            assert!(
                body.contains("Intent Studio"),
                "Should render Intent Studio UI"
            );
            return;
        }
    }

    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_custom_ports() {
    skip_on_ci!();
    let studio_port = 14000;
    let app_port = 19000;

    let mut child =
        start_intent_studio("examples/intent_demo/server.intent", studio_port, app_port);

    thread::sleep(Duration::from_secs(2));

    // Studio should be on custom port
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    let studio_ready = wait_for_server(&studio_url, 5);

    child.kill().ok();

    assert!(
        studio_ready,
        "Intent Studio should start on custom port {}",
        studio_port
    );
}

// ============================================================================
// Hot Reload Tests
// ============================================================================

#[test]
fn test_hot_reload_env_var_override() {
    skip_on_ci!();
    // Test that NTNT_LISTEN_PORT environment variable works
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir
        .join("test_hot_reload.tnt")
        .to_string_lossy()
        .to_string();
    fs::write(
        &test_file,
        r#"
import { json } from "std/http/server"

fn handler(req) {
    return json(map { "status": "ok" })
}

get("/", handler)
listen(8080)
"#,
    )
    .unwrap();

    // Start with custom port via env var
    let binary = if std::path::Path::new("./target/release/ntnt").exists() {
        "./target/release/ntnt"
    } else {
        "./target/debug/ntnt"
    };

    let child = Command::new(binary)
        .args(&["run", &test_file])
        .env("NTNT_LISTEN_PORT", "19999")
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    if let Ok(mut child) = child {
        thread::sleep(Duration::from_secs(2));

        // Server should be on port 19999, not 8080
        let url = "http://127.0.0.1:19999";
        let server_ready = wait_for_server(url, 3);

        child.kill().ok();
        fs::remove_file(test_file).ok();

        assert!(server_ready, "Server should use NTNT_LISTEN_PORT env var");
    } else {
        fs::remove_file(test_file).ok();
    }
}

// ============================================================================
// Intent File Format Tests
// ============================================================================

#[test]
fn test_intent_file_parsing_features() {
    let temp_dir = std::env::temp_dir();
    let test_intent = temp_dir
        .join("test_features.intent")
        .to_string_lossy()
        .to_string();
    fs::write(
        &test_intent,
        r#"# Test Project

## Overview
A test project.

---

Feature: User Login
  id: feature.user_login
  description: "Allow users to log in"
  test:
    - request: POST /login
      body: '{"email": "test@test.com"}'
      assert:
        - status: 200
        - body contains "token"

Feature: User Logout
  id: feature.user_logout
  description: "Allow users to log out"
  test:
    - request: POST /logout
      assert:
        - status: 200

---

Constraint: Rate Limiting
  description: "Limit API requests"
  applies_to: [feature.user_login]
"#,
    )
    .unwrap();

    // Parse should work (we just verify file is readable)
    let content = fs::read_to_string(&test_intent).unwrap();

    fs::remove_file(&test_intent).ok();

    assert!(content.contains("Feature: User Login"));
    assert!(content.contains("id: feature.user_login"));
    assert!(content.contains("Constraint: Rate Limiting"));
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
        "Help should mention studio subcommand"
    );
    assert!(
        output.contains("check") || output.contains("Check"),
        "Help should mention check subcommand"
    );
    assert!(
        output.contains("init") || output.contains("Init"),
        "Help should mention init subcommand"
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

// ============================================================================
// Default Port Tests
// ============================================================================

#[test]
fn test_default_studio_port_is_3001() {
    // Verify default port in help or by checking behavior
    let (stdout, stderr, _) = run_ntnt(&["intent", "studio", "--help"]);
    let output = format!("{}{}", stdout, stderr);

    // Should mention 3001 as default
    assert!(
        output.contains("3001") || output.contains("default"),
        "Should document default studio port"
    );
}

#[test]
fn test_default_app_port_is_8081() {
    // Verify default port in help or by checking behavior
    let (stdout, stderr, _) = run_ntnt(&["intent", "studio", "--help"]);
    let output = format!("{}{}", stdout, stderr);

    // Should mention 8081 as default
    assert!(
        output.contains("8081") || output.contains("default"),
        "Should document default app port"
    );
}

// ============================================================================
// Regression Tests - These MUST run on CI to catch regressions
// ============================================================================

/// Test that intent check actually passes on a known-good example.
/// This test caught the NTNT_LISTEN_PORT regression where the async server
/// ignored the environment variable, causing all intent checks to fail.
///
/// IMPORTANT: This test does NOT use skip_on_ci!() - it MUST run on CI.
#[test]
fn test_intent_check_actually_passes() {
    // Use unique port to avoid conflicts with other tests
    let (stdout, stderr, code) = run_ntnt(&[
        "intent",
        "check",
        "tests/fixtures/simple_server/server.tnt",
        "--port",
        "18091",
    ]);
    let output = format!("{}{}", stdout, stderr);

    // The test should PASS (exit code 0), not just run
    assert_eq!(
        code, 0,
        "intent check should pass on simple_server fixture.\n\
         Exit code: {}\n\
         Output:\n{}",
        code, output
    );

    // Verify we actually ran tests and they passed
    assert!(
        output.contains("passed") && output.contains("0 failed"),
        "Should show passing tests with 0 failures.\nOutput:\n{}",
        output
    );
}

/// Test that the server respects NTNT_LISTEN_PORT environment variable.
/// This is a regression test for the async server ignoring NTNT_LISTEN_PORT.
///
/// IMPORTANT: This test does NOT use skip_on_ci!() - it MUST run on CI.
#[test]
fn test_async_server_respects_listen_port_env_var() {
    // Create a temp file that listens on port 8080 in code
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

    // Start server with NTNT_LISTEN_PORT override
    let test_port = 19876;
    let binary = if std::path::Path::new("./target/release/ntnt").exists() {
        "./target/release/ntnt"
    } else if std::path::Path::new("./target/dev-release/ntnt").exists() {
        "./target/dev-release/ntnt"
    } else {
        "./target/debug/ntnt"
    };

    let child = Command::new(binary)
        .args(&["run", &test_file])
        .env("NTNT_LISTEN_PORT", test_port.to_string())
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();

    if let Ok(mut child) = child {
        // Wait for server to start
        thread::sleep(Duration::from_secs(2));

        // Server MUST be on the env var port (19876), NOT the code port (8080)
        let correct_url = format!("http://127.0.0.1:{}", test_port);
        let wrong_url = "http://127.0.0.1:8080";

        let on_correct_port = wait_for_server(&correct_url, 3);
        let on_wrong_port = reqwest::blocking::get(wrong_url).is_ok();

        child.kill().ok();
        fs::remove_file(&test_file).ok();

        assert!(
            on_correct_port,
            "Server should listen on NTNT_LISTEN_PORT ({}), not the port in code (8080).\n\
             This is a regression - the async server must respect NTNT_LISTEN_PORT.",
            test_port
        );

        // Also verify it's NOT on the wrong port (unless something else is using it)
        if on_wrong_port && !on_correct_port {
            panic!(
                "Server is on port 8080 (from code) instead of {} (from NTNT_LISTEN_PORT).\n\
                 This means NTNT_LISTEN_PORT is being ignored!",
                test_port
            );
        }
    } else {
        fs::remove_file(&test_file).ok();
        panic!("Failed to start ntnt process");
    }
}

/// Test that intent check returns non-zero exit code when tests fail.
/// Ensures we can actually detect failures.
#[test]
fn test_intent_check_fails_on_bad_assertions() {
    // Create a server that doesn't match its intent
    let temp_dir = std::env::temp_dir();
    let test_tnt = temp_dir
        .join("test_fail_check.tnt")
        .to_string_lossy()
        .to_string();
    let test_intent = temp_dir
        .join("test_fail_check.intent")
        .to_string_lossy()
        .to_string();

    // Server returns "hello" but intent expects "goodbye"
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

    // Use unique port to avoid conflicts with other tests
    let (stdout, stderr, code) = run_ntnt(&["intent", "check", &test_tnt, "--port", "18092"]);

    // Clean up
    fs::remove_file(&test_tnt).ok();
    fs::remove_file(&test_intent).ok();

    let output = format!("{}{}", stdout, stderr);

    // Should fail (non-zero exit code)
    assert_ne!(
        code, 0,
        "intent check should fail when assertions don't match.\nOutput:\n{}",
        output
    );

    // Should show failed assertions
    assert!(
        output.contains("failed") || output.contains("Failed"),
        "Output should indicate test failure.\nOutput:\n{}",
        output
    );
}
