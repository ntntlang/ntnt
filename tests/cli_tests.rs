//! Integration tests for NTNT CLI commands
//!
//! Tests the CLI commands: check, inspect, validate, parse, lex

use std::process::Command;

/// Helper to run ntnt command and capture output
fn run_ntnt(args: &[&str]) -> (String, String, i32) {
    // Try release binary first, fall back to debug
    let binary = if std::path::Path::new("./target/release/ntnt").exists() {
        "./target/release/ntnt"
    } else {
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

// ============================================================================
// ntnt check tests
// ============================================================================

#[test]
fn test_check_valid_file() {
    let (stdout, stderr, code) = run_ntnt(&["check", "examples/hello.tnt"]);
    assert_eq!(code, 0, "check should succeed for valid file");
    assert!(stdout.contains("No errors found") || stderr.contains("No errors found"));
}

#[test]
fn test_check_nonexistent_file() {
    let (_, _, code) = run_ntnt(&["check", "examples/nonexistent.tnt"]);
    assert_ne!(code, 0, "check should fail for nonexistent file");
}

// ============================================================================
// ntnt validate tests
// ============================================================================

#[test]
fn test_validate_valid_file() {
    let (stdout, _, code) = run_ntnt(&["validate", "examples/hello.tnt"]);
    assert_eq!(code, 0, "validate should succeed for valid file");

    // Should output JSON
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("validate should output valid JSON");

    assert!(json["summary"]["errors"].as_i64().unwrap() == 0);
    assert!(json["summary"]["valid"].as_i64().unwrap() == 1);
}

#[test]
fn test_validate_directory() {
    let (stdout, stderr, code) = run_ntnt(&["validate", "examples/"]);
    assert_eq!(code, 0, "validate should succeed when all files are valid");

    // Should output JSON with multiple files
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("validate should output valid JSON");

    let total = json["summary"]["total"].as_i64().unwrap();
    assert!(total > 10, "Should validate multiple files");

    // Check stderr has progress indicators
    assert!(stderr.contains("✓") || stderr.contains("⚠"));
}

#[test]
fn test_validate_shows_warnings() {
    let (stdout, _, _) = run_ntnt(&["validate", "examples/website.tnt"]);

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("validate should output valid JSON");

    // website.tnt has unused imports
    let warnings = json["summary"]["warnings"].as_i64().unwrap();
    assert!(warnings > 0, "Should detect unused import warnings");
}

// ============================================================================
// ntnt inspect tests
// ============================================================================

#[test]
fn test_inspect_outputs_json() {
    let (stdout, _, code) = run_ntnt(&["inspect", "examples/hello.tnt"]);
    assert_eq!(code, 0, "inspect should succeed");

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("inspect should output valid JSON");

    assert!(json["files"].is_array());
    assert!(json["functions"].is_array());
    assert!(json["routes"].is_array());
}

#[test]
fn test_inspect_detects_functions() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/hello.tnt"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let functions = json["functions"].as_array().unwrap();

    // hello.tnt has greet and factorial functions
    assert!(functions.len() >= 2, "Should detect functions");

    let func_names: Vec<&str> = functions
        .iter()
        .map(|f| f["name"].as_str().unwrap())
        .collect();

    assert!(func_names.contains(&"greet"), "Should find greet function");
    assert!(
        func_names.contains(&"factorial"),
        "Should find factorial function"
    );
}

#[test]
fn test_inspect_detects_routes() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/http_server.tnt"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let routes = json["routes"].as_array().unwrap();

    assert!(routes.len() > 0, "Should detect HTTP routes");

    // Check route structure
    let first_route = &routes[0];
    assert!(first_route["method"].is_string());
    assert!(first_route["path"].is_string());
    assert!(first_route["handler"].is_string());
    assert!(first_route["line"].is_number());
}

#[test]
fn test_inspect_detects_middleware() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/website.tnt"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let middleware = json["middleware"].as_array().unwrap();

    assert!(middleware.len() > 0, "Should detect middleware");
    assert_eq!(middleware[0]["handler"].as_str().unwrap(), "logger");
}

#[test]
fn test_inspect_detects_static_dirs() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/website.tnt"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let static_dirs = json["static"].as_array().unwrap();

    assert!(static_dirs.len() > 0, "Should detect static directories");
    assert_eq!(static_dirs[0]["prefix"].as_str().unwrap(), "/assets");
}

#[test]
fn test_inspect_includes_line_numbers() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/hello.tnt"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let functions = json["functions"].as_array().unwrap();

    // All functions should have line numbers
    for func in functions {
        assert!(
            func["line"].is_number(),
            "Functions should have line numbers"
        );
        let line = func["line"].as_i64().unwrap();
        assert!(line > 0, "Line numbers should be positive");
    }
}

#[test]
fn test_inspect_detects_contracts() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/contracts.tnt"]);

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let functions = json["functions"].as_array().unwrap();

    // Find the divide function which has contracts
    let divide_fn = functions
        .iter()
        .find(|f| f["name"].as_str().unwrap() == "divide")
        .expect("Should find divide function");

    let contracts = &divide_fn["contracts"];
    assert!(!contracts.is_null(), "divide should have contracts");
    assert!(contracts["requires"].as_array().unwrap().len() > 0);
}

#[test]
fn test_inspect_pretty_flag() {
    let (stdout_compact, _, _) = run_ntnt(&["inspect", "examples/hello.tnt"]);
    let (stdout_pretty, _, _) = run_ntnt(&["inspect", "examples/hello.tnt", "--pretty"]);

    // Pretty output should have newlines and indentation
    assert!(
        stdout_pretty.contains("\n  "),
        "Pretty output should be indented"
    );

    // Both should parse to the same JSON
    let json_compact: serde_json::Value = serde_json::from_str(&stdout_compact).unwrap();
    let json_pretty: serde_json::Value = serde_json::from_str(&stdout_pretty).unwrap();

    assert_eq!(json_compact["functions"], json_pretty["functions"]);
}

// ============================================================================
// ntnt parse tests
// ============================================================================

#[test]
fn test_parse_outputs_ast() {
    let (stdout, _, code) = run_ntnt(&["parse", "examples/hello.tnt"]);
    assert_eq!(code, 0, "parse should succeed");
    assert!(stdout.contains("Program") || stdout.contains("statements"));
}

#[test]
fn test_parse_json_flag() {
    let (stdout, _, code) = run_ntnt(&["parse", "examples/hello.tnt", "--json"]);
    assert_eq!(code, 0, "parse --json should succeed");

    // Should be valid JSON
    let _: serde_json::Value =
        serde_json::from_str(&stdout).expect("parse --json should output valid JSON");
}

// ============================================================================
// ntnt lex tests
// ============================================================================

#[test]
fn test_lex_outputs_tokens() {
    let (stdout, _, code) = run_ntnt(&["lex", "examples/hello.tnt"]);
    assert_eq!(code, 0, "lex should succeed");
    assert!(stdout.contains("Token"), "Should output tokens");
}

// ============================================================================
// Error handling tests
// ============================================================================

#[test]
fn test_validate_exits_nonzero_on_syntax_error() {
    use std::fs;
    use std::io::Write;

    // Create a file with a syntax error
    let test_file = "/tmp/ntnt_test_invalid.tnt";
    let mut file = fs::File::create(test_file).unwrap();
    writeln!(file, "fn broken(").unwrap();

    let (stdout, _, code) = run_ntnt(&["validate", test_file]);

    // Clean up
    fs::remove_file(test_file).ok();

    assert_ne!(
        code, 0,
        "validate should exit with error code on syntax error"
    );

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(json["summary"]["errors"].as_i64().unwrap() > 0);
}

#[test]
fn test_inspect_handles_invalid_file_gracefully() {
    use std::fs;
    use std::io::Write;

    // Create a file with a syntax error
    let test_file = "/tmp/ntnt_test_invalid2.tnt";
    let mut file = fs::File::create(test_file).unwrap();
    writeln!(file, "fn broken(").unwrap();

    let (stdout, stderr, code) = run_ntnt(&["inspect", test_file]);

    // Clean up
    fs::remove_file(test_file).ok();

    // Should still output JSON (with empty arrays) and warn
    assert_eq!(code, 0, "inspect should succeed even with parse errors");
    assert!(stderr.contains("Warning") || stderr.contains("Failed"));

    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(json["functions"].as_array().unwrap().is_empty());
}

// ============================================================================
// File-based routing detection tests
// ============================================================================

#[test]
fn test_inspect_detects_file_based_routes() {
    let (stdout, _, code) = run_ntnt(&["inspect", "examples/myapp"]);
    assert_eq!(code, 0);

    let json: serde_json::Value = serde_json::from_str(&stdout).expect("Should output valid JSON");

    let routes = json["routes"].as_array().expect("Should have routes array");

    // Should detect all file-based routes
    assert!(
        routes.len() >= 6,
        "Should detect at least 6 routes, found {}",
        routes.len()
    );

    // Check for specific routes
    let route_paths: Vec<&str> = routes.iter().filter_map(|r| r["path"].as_str()).collect();

    assert!(route_paths.contains(&"/"), "Should detect root route");
    assert!(
        route_paths.contains(&"/about"),
        "Should detect /about route"
    );
    assert!(
        route_paths.contains(&"/api/status"),
        "Should detect /api/status route"
    );
    assert!(
        route_paths.contains(&"/api/users"),
        "Should detect /api/users route"
    );
    assert!(
        route_paths
            .iter()
            .any(|p| p.contains("/api/users/") && p.contains("id")),
        "Should detect /api/users/{{id}} route"
    );
}

#[test]
fn test_inspect_file_based_routes_have_correct_methods() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/myapp"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let routes = json["routes"].as_array().unwrap();

    // Find routes for /api/users/{id} - should have GET, PUT, DELETE
    let user_routes: Vec<&serde_json::Value> = routes
        .iter()
        .filter(|r| {
            r["path"]
                .as_str()
                .map(|p| p.contains("/api/users/") && p.contains("id"))
                .unwrap_or(false)
        })
        .collect();

    let methods: Vec<&str> = user_routes
        .iter()
        .filter_map(|r| r["method"].as_str())
        .collect();

    assert!(methods.contains(&"GET"), "Should have GET method");
    assert!(methods.contains(&"PUT"), "Should have PUT method");
    assert!(methods.contains(&"DELETE"), "Should have DELETE method");
}

#[test]
fn test_inspect_file_based_routes_marked_correctly() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/myapp"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let routes = json["routes"].as_array().unwrap();

    // All routes in myapp should be marked as "file-based"
    for route in routes {
        assert_eq!(
            route["routing"].as_str(),
            Some("file-based"),
            "Route {} should be marked as file-based",
            route["path"]
        );
    }
}

#[test]
fn test_inspect_file_based_routes_have_line_numbers() {
    let (stdout, _, _) = run_ntnt(&["inspect", "examples/myapp"]);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let routes = json["routes"].as_array().unwrap();

    for route in routes {
        assert!(
            route["line"].is_number(),
            "Route {} should have a line number",
            route["path"]
        );
    }
}
