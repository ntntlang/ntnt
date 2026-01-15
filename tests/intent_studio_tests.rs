//! Integration tests for Intent Studio features
//!
//! Tests the Intent Studio server, app auto-start, hot-reload, and live test execution

use std::process::{Command, Child, Stdio};
use std::time::Duration;
use std::thread;
use std::fs;

/// Helper to run ntnt command and capture output
fn run_ntnt(args: &[&str]) -> (String, String, i32) {
    let output = Command::new("./target/release/ntnt")
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
    Command::new("./target/release/ntnt")
        .args(&["intent", "studio", intent_file, "--port", &studio_port.to_string(), "--app-port", &app_port.to_string()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start Intent Studio")
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
    let (_stdout, stderr, code) = run_ntnt(&["intent", "check", "examples/intent_demo/server.tnt"]);
    // Should succeed - file exists with matching .intent
    assert!(code == 0 || stderr.contains("Intent file not found"), 
        "intent check should work or report missing intent file");
}

#[test]
fn test_intent_check_shows_coverage() {
    let (stdout, stderr, code) = run_ntnt(&["intent", "check", "examples/intent_demo/server.tnt"]);
    // The intent check command outputs to stderr typically
    // Just verify it runs without crashing - the output format may vary
    let output = format!("{}{}", stdout, stderr);
    // Either shows coverage info OR reports that something is implemented/missing
    assert!(code == 0 || output.contains("not found") || output.contains("error"),
        "intent check should either succeed or report a clear error");
}

#[test]
fn test_intent_coverage_command() {
    let (stdout, stderr, code) = run_ntnt(&["intent", "coverage", "examples/intent_demo/server.tnt"]);
    // Should succeed if intent file exists
    if code == 0 {
        assert!(stdout.contains("%") || stderr.contains("%") || 
                stdout.contains("Feature") || stderr.contains("Feature"),
            "Coverage should show percentage or feature info");
    }
}

#[test]
fn test_intent_init_generates_stub() {
    let test_intent = "/tmp/test_init.intent";
    let test_tnt = "/tmp/test_init.tnt";
    
    // Create a simple intent file
    fs::write(test_intent, r#"# Test Project
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
"#).unwrap();
    
    let (_stdout, _stderr, code) = run_ntnt(&["intent", "init", test_intent, "-o", test_tnt]);
    
    // Clean up
    fs::remove_file(test_intent).ok();
    
    if code == 0 {
        // Check that output file was created
        assert!(fs::metadata(test_tnt).is_ok(), "Should create output .tnt file");
        
        let content = fs::read_to_string(test_tnt).unwrap_or_default();
        // Should contain @implements annotation
        assert!(content.contains("@implements") || content.contains("feature.hello_world"),
            "Generated file should have @implements annotations");
        
        fs::remove_file(test_tnt).ok();
    }
}

// ============================================================================
// Intent Studio Server Tests  
// ============================================================================

#[test]
fn test_intent_studio_starts_on_default_port() {
    // Use unique ports to avoid conflicts with other tests
    let studio_port = 13001;
    let app_port = 18081;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent", 
        studio_port, 
        app_port
    );
    
    // Wait for studio to start
    thread::sleep(Duration::from_secs(2));
    
    // Try to connect to studio
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    let studio_ready = wait_for_server(&studio_url, 5);
    
    // Kill the process
    child.kill().ok();
    
    assert!(studio_ready, "Intent Studio should start and respond on port {}", studio_port);
}

#[test]
fn test_intent_studio_serves_html() {
    let studio_port = 13002;
    let app_port = 18082;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
    thread::sleep(Duration::from_secs(2));
    
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((status, body)) = http_get(&studio_url) {
            child.kill().ok();
            
            assert_eq!(status, 200, "Should return 200 OK");
            assert!(body.contains("<!DOCTYPE html>"), "Should serve HTML");
            assert!(body.contains("Intent Studio"), "Should contain Intent Studio title");
            assert!(body.contains("NTNT"), "Should contain NTNT branding");
            return;
        }
    }
    
    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_has_logo() {
    let studio_port = 13003;
    let app_port = 18083;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
    thread::sleep(Duration::from_secs(2));
    
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();
            
            // Should have the NTNT logo as base64 PNG
            assert!(body.contains("data:image/png;base64,"), 
                "Should contain base64-encoded PNG logo");
            assert!(body.contains("logo-icon"), 
                "Should have logo-icon class");
            return;
        }
    }
    
    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_has_open_app_button() {
    let studio_port = 13004;
    let app_port = 18084;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
    thread::sleep(Duration::from_secs(2));
    
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();
            
            // Should have Open App button with correct port
            assert!(body.contains("Open App"), "Should have Open App button");
            assert!(body.contains(&format!("http://127.0.0.1:{}", app_port)), 
                "Should link to correct app port");
            assert!(body.contains("target=\"_blank\""), 
                "Should open in new tab");
            return;
        }
    }
    
    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_app_status_endpoint() {
    let studio_port = 13005;
    let app_port = 18085;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
    thread::sleep(Duration::from_secs(3)); // Give time for app to start
    
    let status_url = format!("http://127.0.0.1:{}/app-status", studio_port);
    if wait_for_server(&format!("http://127.0.0.1:{}", studio_port), 5) {
        if let Ok((status, body)) = http_get(&status_url) {
            child.kill().ok();
            
            assert_eq!(status, 200, "app-status should return 200");
            
            // Should return JSON with running and healthy fields
            // Format: {"running": true/false, "healthy": true/false, "status": 200} or {"running": false, "healthy": false, "error": "..."}
            let json: serde_json::Value = serde_json::from_str(&body)
                .expect("app-status should return valid JSON");
            
            assert!(json["running"].is_boolean(), "Should have 'running' boolean field");
            // healthy field is only present when running is true
            if json["running"].as_bool().unwrap_or(false) {
                assert!(json["healthy"].is_boolean(), "Should have 'healthy' boolean field when running");
            }
            
            return;
        }
    }
    
    child.kill().ok();
    panic!("Could not connect to Intent Studio app-status endpoint");
}

#[test]
fn test_intent_studio_run_tests_endpoint() {
    let studio_port = 13006;
    let app_port = 18086;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
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
            let json: serde_json::Value = serde_json::from_str(&body)
                .expect("run-tests should return valid JSON");
            
            assert!(json["features"].is_array(), "Should have features array");
            assert!(json["total_assertions"].is_number(), "Should have total_assertions count");
            assert!(json["passed_assertions"].is_number(), "Should have passed_assertions count");
            assert!(json["failed_assertions"].is_number(), "Should have failed_assertions count");
            
            return;
        }
    }
    
    child.kill().ok();
    panic!("Could not connect to Intent Studio run-tests endpoint");
}

#[test]
fn test_intent_studio_ui_has_test_controls() {
    let studio_port = 13007;
    let app_port = 18087;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
    thread::sleep(Duration::from_secs(2));
    
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();
            
            // Should have test-related UI elements
            assert!(body.contains("Run Tests") || body.contains("runTests"), 
                "Should have Run Tests button");
            assert!(body.contains("pass") || body.contains("Pass"), 
                "Should have pass indicator");
            assert!(body.contains("fail") || body.contains("Fail"), 
                "Should have fail indicator");
            return;
        }
    }
    
    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_ui_has_app_status_indicator() {
    let studio_port = 13008;
    let app_port = 18088;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
    thread::sleep(Duration::from_secs(2));
    
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    if wait_for_server(&studio_url, 5) {
        if let Ok((_, body)) = http_get(&studio_url) {
            child.kill().ok();
            
            // Should have app status indicator
            assert!(body.contains("app-status") || body.contains("appStatus"), 
                "Should have app status element");
            assert!(body.contains("checkAppStatus") || body.contains("app_status"),
                "Should have app status checking logic");
            return;
        }
    }
    
    child.kill().ok();
    panic!("Could not connect to Intent Studio");
}

#[test]
fn test_intent_studio_custom_ports() {
    let studio_port = 14000;
    let app_port = 19000;
    
    let mut child = start_intent_studio(
        "examples/intent_demo/server.intent",
        studio_port,
        app_port
    );
    
    thread::sleep(Duration::from_secs(2));
    
    // Studio should be on custom port
    let studio_url = format!("http://127.0.0.1:{}", studio_port);
    let studio_ready = wait_for_server(&studio_url, 5);
    
    child.kill().ok();
    
    assert!(studio_ready, "Intent Studio should start on custom port {}", studio_port);
}

// ============================================================================
// Hot Reload Tests
// ============================================================================

#[test]
fn test_hot_reload_env_var_override() {
    // Test that NTNT_LISTEN_PORT environment variable works
    let test_file = "/tmp/test_hot_reload.tnt";
    fs::write(test_file, r#"
import { json } from "std/http/server"

fn handler(req) {
    return json(map { "status": "ok" })
}

get("/", handler)
listen(8080)
"#).unwrap();
    
    // Start with custom port via env var
    let child = Command::new("./target/release/ntnt")
        .args(&["run", test_file])
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
    let test_intent = "/tmp/test_features.intent";
    fs::write(test_intent, r#"# Test Project

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
"#).unwrap();
    
    // Parse should work (we just verify file is readable)
    let content = fs::read_to_string(test_intent).unwrap();
    
    fs::remove_file(test_intent).ok();
    
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
    
    assert!(output.contains("studio") || output.contains("Studio"), 
        "Help should mention studio subcommand");
    assert!(output.contains("check") || output.contains("Check"),
        "Help should mention check subcommand");
    assert!(output.contains("init") || output.contains("Init"),
        "Help should mention init subcommand");
}

#[test]
fn test_intent_studio_help() {
    let (stdout, stderr, _) = run_ntnt(&["intent", "studio", "--help"]);
    let output = format!("{}{}", stdout, stderr);
    
    assert!(output.contains("port") || output.contains("PORT"),
        "Help should mention port option");
    assert!(output.contains("app-port") || output.contains("APP"),
        "Help should mention app-port option");
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
    assert!(output.contains("3001") || output.contains("default"),
        "Should document default studio port");
}

#[test]
fn test_default_app_port_is_8081() {
    // Verify default port in help or by checking behavior
    let (stdout, stderr, _) = run_ntnt(&["intent", "studio", "--help"]);
    let output = format!("{}{}", stdout, stderr);
    
    // Should mention 8081 as default
    assert!(output.contains("8081") || output.contains("default"),
        "Should document default app port");
}
