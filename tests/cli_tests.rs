//! Integration tests for NTNT CLI commands
//!
//! Tests the CLI commands: check, inspect, validate, parse, lex

use std::process::Command;

/// Helper to run ntnt command and capture output
fn run_ntnt(args: &[&str]) -> (String, String, i32) {
    // Prefer debug binary (matches cargo test profile), fall back to release
    // This ensures we test the freshly built binary, not a cached release build
    // Account for .exe extension on Windows
    let exe = std::env::consts::EXE_SUFFIX;
    let debug_path = format!("./target/debug/ntnt{}", exe);
    let release_path = format!("./target/release/ntnt{}", exe);

    let binary = if std::path::Path::new(&debug_path).exists() {
        debug_path
    } else if std::path::Path::new(&release_path).exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
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
    assert_eq!(
        code, 0,
        "validate should succeed when all files are valid.\nstderr:\n{}\nstdout:\n{}",
        stderr, stdout
    );

    // Should output JSON with multiple files
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("validate should output valid JSON");

    let total = json["summary"]["total"].as_i64().unwrap();
    assert!(total > 10, "Should validate multiple files");

    // Check stderr has progress indicators
    assert!(stderr.contains("✓") || stderr.contains("⚠"));
}

#[test]
fn test_validate_outputs_structured_json() {
    let (stdout, _, code) = run_ntnt(&["validate", "examples/http_server.tnt"]);

    assert_eq!(code, 0, "validate should succeed on a valid file");

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("validate should output valid JSON");

    // Verify expected structure
    assert!(
        json["summary"]["errors"].is_number(),
        "summary should have errors count"
    );
    assert!(
        json["summary"]["warnings"].is_number(),
        "summary should have warnings count"
    );
    assert!(json["files"].is_array(), "should have files array");

    let files = json["files"].as_array().unwrap();
    assert!(!files.is_empty(), "should have at least one file entry");
    assert!(
        files[0]["valid"].as_bool().unwrap(),
        "http_server.tnt should be valid"
    );
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

    // Create a file with a syntax error in temp directory
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("ntnt_test_invalid.tnt");
    let mut file = fs::File::create(&test_file).unwrap();
    writeln!(file, "fn broken(").unwrap();

    let test_path = test_file.to_str().unwrap();
    let (stdout, _, code) = run_ntnt(&["validate", test_path]);

    // Clean up
    fs::remove_file(&test_file).ok();

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

    // Create a file with a syntax error in temp directory
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("ntnt_test_invalid2.tnt");
    let mut file = fs::File::create(&test_file).unwrap();
    writeln!(file, "fn broken(").unwrap();

    let test_path = test_file.to_str().unwrap();
    let (stdout, stderr, code) = run_ntnt(&["inspect", test_path]);

    // Clean up
    fs::remove_file(&test_file).ok();

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
#[cfg_attr(
    target_os = "windows",
    ignore = "File-based routing detection has Windows path issues"
)]
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
#[cfg_attr(
    target_os = "windows",
    ignore = "File-based routing detection has Windows path issues"
)]
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

// ============================================================================
// Route pattern auto-detection tests (inspect)
// ============================================================================

#[test]
fn test_inspect_detects_auto_detected_route_params() {
    use std::fs;
    use std::io::Write;

    // Create a file with routes using regular strings (no raw strings)
    let temp_dir = std::env::temp_dir();
    let test_file = temp_dir.join("ntnt_test_route_autodetect.tnt");
    let mut file = fs::File::create(&test_file).unwrap();
    writeln!(
        file,
        r#"import {{ json }} from "std/http/server"
fn get_user(req) {{ return json(map {{ "ok": true }}) }}
fn list_items(req) {{ return json(map {{ "ok": true }}) }}

get("/users/{{id}}", get_user)
post("/api/{{category}}/items/{{id}}", list_items)
get("/", get_user)
listen(8080)"#
    )
    .unwrap();

    let test_path = test_file.to_str().unwrap();
    let (stdout, _, code) = run_ntnt(&["inspect", test_path]);

    // Clean up
    fs::remove_file(&test_file).ok();

    assert_eq!(code, 0, "inspect should succeed");

    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("inspect should output valid JSON");
    let routes = json["routes"].as_array().expect("Should have routes array");

    // Should detect all 3 routes including those with auto-detected params
    assert!(
        routes.len() >= 3,
        "Should detect at least 3 routes, found {}",
        routes.len()
    );

    let route_paths: Vec<&str> = routes.iter().filter_map(|r| r["path"].as_str()).collect();

    assert!(
        route_paths.contains(&"/users/{id}"),
        "Should detect /users/{{id}} route from regular string. Found: {:?}",
        route_paths
    );
    assert!(
        route_paths.contains(&"/api/{category}/items/{id}"),
        "Should detect /api/{{category}}/items/{{id}} route. Found: {:?}",
        route_paths
    );
    assert!(route_paths.contains(&"/"), "Should detect / route");
}

// =========================================================================
// ntnt learn tests
// =========================================================================

#[test]
fn test_learn_stdout_outputs_rules() {
    let (stdout, _, exit_code) = run_ntnt(&["learn"]);
    assert_eq!(exit_code, 0, "ntnt learn should exit 0");
    assert!(
        stdout.contains("Critical Syntax Rules"),
        "Should contain critical rules header"
    );
    assert!(stdout.contains("#{"), "Should mention interpolation syntax");
}

#[test]
fn test_learn_unknown_platform_exits_nonzero() {
    let (_, stderr, exit_code) = run_ntnt(&["learn", "nonexistent"]);
    assert_ne!(exit_code, 0, "Unknown platform should exit non-zero");
    assert!(
        stderr.contains("Unknown platform"),
        "Should report unknown platform"
    );
}

#[test]
fn test_learn_check_no_files() {
    // Run in a temp dir with no learn files — should exit 0 (nothing to check)
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_1", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["learn", "--check"])
        .current_dir(&dir_path)
        .output()
        .expect("Failed to run ntnt learn --check");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code().unwrap(), 0);
    assert!(
        stdout.contains("No ntnt learn files found"),
        "Should report no files found"
    );
}

#[test]
fn test_learn_claude_code_creates_files() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_2", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["learn", "claude-code"])
        .current_dir(&dir_path)
        .output()
        .expect("Failed to run ntnt learn claude-code");
    assert_eq!(output.status.code().unwrap(), 0);
    assert!(
        &dir_path.join(".claude/CLAUDE.md").exists(),
        "Should create .claude/CLAUDE.md"
    );
    assert!(
        &dir_path.join(".claude/rules/ntnt.md").exists(),
        "Should create .claude/rules/ntnt.md"
    );

    // Verify content
    let claude_md = std::fs::read_to_string(&dir_path.join(".claude/CLAUDE.md")).unwrap();
    assert!(
        claude_md.contains("Generated by ntnt v"),
        "Should have version header"
    );
    assert!(
        claude_md.contains("Critical Syntax Rules"),
        "Should have rules content"
    );
}

#[test]
fn test_learn_check_detects_stale_legacy_cursorrules() {
    let dir_path =
        std::env::temp_dir().join(format!("ntnt_test_{}_legacy_check", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let exe = find_ntnt_binary();

    std::fs::write(
        dir_path.join(".cursorrules"),
        "# Generated by ntnt v0.0.1 — do not edit manually\nold cursor rules",
    )
    .unwrap();

    let output = Command::new(&exe)
        .args(&["learn", "--check"])
        .current_dir(&dir_path)
        .output()
        .expect("Failed to run ntnt learn --check");

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Stale legacy .cursorrules should exit 1"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(".cursorrules (legacy cursor)"),
        "Should mention legacy cursor file"
    );
    assert!(
        stdout.contains("stale"),
        "Should report stale legacy cursor file"
    );
}

#[test]
fn test_learn_check_detects_stale() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_3", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let exe = find_ntnt_binary();

    // Create a stale file with an old version
    let claude_dir = &dir_path.join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::write(
        claude_dir.join("CLAUDE.md"),
        "# Generated by ntnt v0.0.1 — do not edit manually\nold content",
    )
    .unwrap();

    let output = Command::new(&exe)
        .args(&["learn", "--check"])
        .current_dir(&dir_path)
        .output()
        .expect("Failed to run ntnt learn --check");
    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Stale files should exit 1"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("stale"), "Should report stale file");
}

#[test]
fn test_learn_update_migrates_managed_legacy_cursorrules() {
    let dir_path =
        std::env::temp_dir().join(format!("ntnt_test_{}_legacy_cursor", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let exe = find_ntnt_binary();

    std::fs::write(
        dir_path.join(".cursorrules"),
        "# Generated by ntnt v0.0.1 — do not edit manually\nold cursor rules",
    )
    .unwrap();

    let output = Command::new(&exe)
        .args(&["learn", "--update"])
        .current_dir(&dir_path)
        .output()
        .expect("Failed to run ntnt learn --update");

    assert_eq!(
        output.status.code().unwrap(),
        0,
        "learn --update should succeed"
    );
    assert!(
        dir_path.join(".cursor/rules/ntnt-primer.mdc").exists(),
        "Should create the Cursor primer file"
    );
    assert!(
        dir_path.join(".cursor/rules/ntnt-reference.mdc").exists(),
        "Should create the Cursor reference file"
    );
    assert!(
        !dir_path.join(".cursorrules").exists(),
        "Managed legacy .cursorrules should be removed after migration"
    );
}

#[test]
fn test_learn_cursor_preserves_user_managed_legacy_cursorrules() {
    let dir_path =
        std::env::temp_dir().join(format!("ntnt_test_{}_user_cursor", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let exe = find_ntnt_binary();

    std::fs::write(
        dir_path.join(".cursorrules"),
        "# user-managed cursor rules\nkeep me",
    )
    .unwrap();

    let output = Command::new(&exe)
        .args(&["learn", "cursor"])
        .current_dir(&dir_path)
        .output()
        .expect("Failed to run ntnt learn cursor");

    assert_eq!(
        output.status.code().unwrap(),
        0,
        "learn cursor should succeed"
    );
    assert!(
        dir_path.join(".cursorrules").exists(),
        "User-managed legacy .cursorrules should not be deleted"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("skipping removal"),
        "Should explain why the legacy file was preserved"
    );
}

// =========================================================================
// ntnt migrate tests
// =========================================================================

#[test]
fn test_migrate_dry_run_no_writes() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_4", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    let original = r#"let x = "Hello {name}""#;
    std::fs::write(&test_file, original).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap(), "--dry-run"])
        .output()
        .expect("Failed to run ntnt migrate --dry-run");
    assert_eq!(output.status.code().unwrap(), 0);

    // File should NOT be modified
    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(after, original, "Dry run should not modify files");
}

#[test]
fn test_migrate_applies_changes() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_5", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"let x = "Hello {name}""#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"let x = "Hello #{name}""#,
        "Should migrate old interpolation to new"
    );
}

#[test]
fn test_migrate_preserves_route_params() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_6", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"get("/users/{id}", handler)"#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"get("/users/{id}", handler)"#,
        "Route params should not be migrated"
    );
}

#[test]
fn test_migrate_skips_template_strings() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_7", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"let x = """hello {{name}}""""#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"let x = """hello {{name}}""""#,
        "Template strings should not be modified"
    );
}

#[test]
fn test_migrate_already_migrated_idempotent() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_8", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    let content = r#"let x = "Hello #{name}""#;
    std::fs::write(&test_file, content).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(after, content, "Already-migrated files should be untouched");
}

#[test]
fn test_migrate_redirect_urls_are_migrated() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_9", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"let url = "/project/{slug}/""#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"let url = "/project/#{slug}/""#,
        "Redirect URLs should have interpolation migrated"
    );
}

#[test]
fn test_migrate_api_urls_are_migrated() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_10", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"let url = "https://{domain}/health""#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"let url = "https://#{domain}/health""#,
        "API URLs with interpolation should be migrated"
    );
}

#[test]
fn test_migrate_file_paths_are_migrated() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_11", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"let path = "projects/{slug}/{path}/file""#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"let path = "projects/#{slug}/#{path}/file""#,
        "File path interpolation should be migrated"
    );
}

#[test]
fn test_migrate_route_params_in_post() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_12", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"post("/api/domains/{slug}/add", handler)"#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"post("/api/domains/{slug}/add", handler)"#,
        "Route params in post() should not be migrated"
    );
}

#[test]
fn test_migrate_route_params_with_multiple_params() {
    let dir_path = std::env::temp_dir().join(format!("ntnt_test_{}_13", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir_path);
    std::fs::create_dir_all(&dir_path).unwrap();
    let test_file = &dir_path.join("test.tnt");
    std::fs::write(&test_file, r#"get("/project/{slug}/{path}/", handler)"#).unwrap();

    let exe = find_ntnt_binary();
    let output = Command::new(&exe)
        .args(&["migrate", test_file.to_str().unwrap()])
        .output()
        .expect("Failed to run ntnt migrate");
    assert_eq!(output.status.code().unwrap(), 0);

    let after = std::fs::read_to_string(&test_file).unwrap();
    assert_eq!(
        after, r#"get("/project/{slug}/{path}/", handler)"#,
        "Multiple route params in get() should not be migrated"
    );
}

/// Helper to find the ntnt binary (works from any current_dir)
fn find_ntnt_binary() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let exe = std::env::consts::EXE_SUFFIX;
    let debug_path = format!("{}/target/debug/ntnt{}", manifest_dir, exe);
    let release_path = format!("{}/target/release/ntnt{}", manifest_dir, exe);
    let dev_release_path = format!("{}/target/dev-release/ntnt{}", manifest_dir, exe);
    if std::path::Path::new(&debug_path).exists() {
        debug_path
    } else if std::path::Path::new(&dev_release_path).exists() {
        dev_release_path
    } else if std::path::Path::new(&release_path).exists() {
        release_path
    } else {
        panic!("No ntnt binary found in {}/target/", manifest_dir);
    }
}
