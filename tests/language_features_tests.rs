//! Integration tests for NTNT language features
//!
//! Tests new language features including:
//! - Map iteration functions (keys, values, entries, has_key)
//! - Nested map inference
//! - CSV parsing

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

/// Helper to run ntnt with a code string
fn run_ntnt_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("feature_test");

    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");
    drop(file);

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
        .args(&["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to execute ntnt");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    // Clean up
    fs::remove_file(&test_file).ok();

    (stdout, stderr, exit_code)
}

/// Helper to run ntnt parse on code
fn run_ntnt_parse(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("parse_test");

    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");
    drop(file);

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
        .args(&["parse", &test_file, "--json"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to execute ntnt");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    // Clean up
    fs::remove_file(&test_file).ok();

    (stdout, stderr, exit_code)
}

// ============================================================================
// Map Iteration Functions (keys, values, entries, has_key)
// ============================================================================

#[test]
fn test_keys_function() {
    let code = r#"
import { keys } from "std/collections"
let data = map { "a": 1, "b": 2, "c": 3 }
let k = keys(data)
print(len(k))
for key in k {
    print(key)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "keys() should work");
    assert!(stdout.contains("3"), "Should have 3 keys");
    assert!(stdout.contains("a"), "Should contain key 'a'");
    assert!(stdout.contains("b"), "Should contain key 'b'");
    assert!(stdout.contains("c"), "Should contain key 'c'");
}

#[test]
fn test_values_function() {
    let code = r#"
import { values } from "std/collections"
let data = map { "x": 10, "y": 20, "z": 30 }
let v = values(data)
print(len(v))
let mut sum = 0
for val in v {
    sum = sum + val
}
print(sum)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "values() should work");
    assert!(stdout.contains("3"), "Should have 3 values");
    assert!(stdout.contains("60"), "Sum should be 60");
}

#[test]
fn test_entries_function() {
    let code = r#"
import { entries } from "std/collections"
let data = map { "name": "Alice", "age": 30 }
let e = entries(data)
print(len(e))
for entry in e {
    print("{entry[0]}: {entry[1]}")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "entries() should work");
    assert!(stdout.contains("2"), "Should have 2 entries");
    assert!(
        stdout.contains("name: Alice") || stdout.contains("age: 30"),
        "Should contain entry data"
    );
}

#[test]
fn test_has_key_function() {
    let code = r#"
import { has_key } from "std/collections"
let data = map { "present": 1, "also_here": 2 }
print(has_key(data, "present"))
print(has_key(data, "missing"))
print(has_key(data, "also_here"))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "has_key() should work");

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines.len(), 3, "Should have 3 output lines");
    assert_eq!(lines[0], "true", "has_key for 'present' should be true");
    assert_eq!(lines[1], "false", "has_key for 'missing' should be false");
    assert_eq!(lines[2], "true", "has_key for 'also_here' should be true");
}

#[test]
fn test_keys_empty_map() {
    let code = r#"
import { keys } from "std/collections"
let empty = map {}
print(len(keys(empty)))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "keys() on empty map should work");
    assert!(stdout.contains("0"), "Empty map should have 0 keys");
}

#[test]
fn test_has_key_empty_map() {
    let code = r#"
import { has_key } from "std/collections"
let empty = map {}
print(has_key(empty, "anything"))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "has_key() on empty map should work");
    assert!(
        stdout.contains("false"),
        "Empty map should not have any key"
    );
}

#[test]
fn test_iterate_over_keys() {
    let code = r#"
import { keys } from "std/collections"
let scores = map { "alice": 100, "bob": 85, "charlie": 92 }
let mut total = 0
for name in keys(scores) {
    total = total + scores[name]
}
print(total)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Iterating over keys should work");
    assert!(stdout.contains("277"), "Total should be 277");
}

// ============================================================================
// Nested Map Inference
// ============================================================================

#[test]
fn test_nested_map_inference_basic() {
    let code = r#"
let data = map {
    "user": { "name": "Alice", "age": 30 }
}
print(data["user"]["name"])
print(data["user"]["age"])
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Nested map inference should work");
    assert!(stdout.contains("Alice"), "Should access nested name");
    assert!(stdout.contains("30"), "Should access nested age");
}

#[test]
fn test_nested_map_inference_deep() {
    let code = r#"
let config = map {
    "level1": {
        "level2": {
            "level3": { "value": 42 }
        }
    }
}
print(config["level1"]["level2"]["level3"]["value"])
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Deep nested map inference should work");
    assert!(stdout.contains("42"), "Should access deeply nested value");
}

#[test]
fn test_nested_map_inference_mixed() {
    let code = r#"
let mixed = map {
    "explicit": map { "a": 1 },
    "inferred": { "b": 2 }
}
print(mixed["explicit"]["a"])
print(mixed["inferred"]["b"])
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Mixed explicit and inferred maps should work");
    assert!(stdout.contains("1"), "Explicit map should work");
    assert!(stdout.contains("2"), "Inferred map should work");
}

#[test]
fn test_nested_map_inference_empty() {
    let code = r#"
import { keys } from "std/collections"
let data = map {
    "empty": {},
    "nested_empty": { "inner": {} }
}
print(len(keys(data["empty"])))
print(len(keys(data["nested_empty"]["inner"])))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Empty nested maps should work");

    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "0", "Empty map should have 0 keys");
    assert_eq!(lines[1], "0", "Nested empty map should have 0 keys");
}

#[test]
fn test_nested_map_with_iteration() {
    let code = r#"
import { keys, has_key } from "std/collections"
let users = map {
    "alice": { "score": 100, "level": 5 },
    "bob": { "score": 85, "level": 3 }
}

let mut total_score = 0
for name in keys(users) {
    let user = users[name]
    total_score = total_score + user["score"]
}
print(total_score)
print(has_key(users["alice"], "score"))
print(has_key(users["alice"], "missing"))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Nested maps with iteration should work");
    assert!(stdout.contains("185"), "Total score should be 185");
    assert!(stdout.contains("true"), "has_key should find 'score'");
    assert!(
        stdout.contains("false"),
        "has_key should not find 'missing'"
    );
}

#[test]
fn test_block_expression_not_affected() {
    let code = r#"
let result = {
    let x = 10
    let y = 20
    x + y
}
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Block expressions should still work");
    assert!(stdout.contains("30"), "Block should evaluate to 30");
}

#[test]
fn test_nested_map_parses_correctly() {
    let code = r#"
let data = map {
    "a": { "x": 1 },
    "b": { "y": 2 }
}
"#;
    let (stdout, _, exit_code) = run_ntnt_parse(code);
    assert_eq!(exit_code, 0, "Nested map should parse");

    // The AST should contain MapLiteral nodes
    assert!(stdout.contains("MapLiteral"), "Should parse as MapLiteral");
}

// ============================================================================
// Backwards Compatibility
// ============================================================================

#[test]
fn test_explicit_nested_map_still_works() {
    let code = r#"
let old_style = map {
    "a": map { "x": 1, "y": 2 },
    "b": map { "z": 3 }
}
print(old_style["a"]["x"])
print(old_style["b"]["z"])
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Explicit map syntax should still work");
    assert!(stdout.contains("1"), "Should access a.x");
    assert!(stdout.contains("3"), "Should access b.z");
}

#[test]
fn test_top_level_map_requires_keyword() {
    let code = r#"
let data = { "name": "Alice" }
print(data)
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    // This should either fail to parse or produce unexpected behavior
    // because {} at top level is a block, not a map
    assert!(
        exit_code != 0 || stderr.contains("error") || stderr.contains("Error"),
        "Top-level {{}} without map keyword should not work as a map"
    );
}

// ============================================================================
// CSV Parsing (if not already tested)
// ============================================================================

#[test]
fn test_csv_parse_basic() {
    let code = r#"
import { parse_csv } from "std/csv"
let csv_data = "name,age,city
Alice,30,NYC
Bob,25,LA"
let rows = parse_csv(csv_data)
print(len(rows))
print(rows[0][0])
print(rows[2][1])
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "CSV parsing should work");
    assert!(stdout.contains("3"), "Should have 3 rows (header + 2 data)");
    assert!(stdout.contains("name"), "First cell should be 'name'");
    assert!(
        stdout.contains("25"),
        "Should access Bob's age (row 2, col 1)"
    );
}
// ============================================================================
// Truthy/Falsy Values
// ============================================================================

#[test]
fn test_truthy_numbers_including_zero() {
    let code = r#"
if 0 { print("zero-truthy") } else { print("zero-falsy") }
if 1 { print("one-truthy") }
if -1 { print("neg-truthy") }
if 0.0 { print("float-zero-truthy") } else { print("float-zero-falsy") }
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Number truthiness should work");
    assert!(stdout.contains("zero-truthy"), "0 should be truthy");
    assert!(stdout.contains("one-truthy"), "1 should be truthy");
    assert!(stdout.contains("neg-truthy"), "-1 should be truthy");
    assert!(stdout.contains("float-zero-truthy"), "0.0 should be truthy");
}

#[test]
fn test_truthy_empty_string_is_falsy() {
    let code = r#"
let empty = ""
let full = "hello"
if empty { print("empty-truthy") } else { print("empty-falsy") }
if full { print("full-truthy") }
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "String truthiness should work");
    assert!(
        stdout.contains("empty-falsy"),
        "Empty string should be falsy"
    );
    assert!(
        stdout.contains("full-truthy"),
        "Non-empty string should be truthy"
    );
}

#[test]
fn test_truthy_empty_array_is_falsy() {
    let code = r#"
let empty = []
let full = [1, 2, 3]
if empty { print("empty-truthy") } else { print("empty-falsy") }
if full { print("full-truthy") }
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Array truthiness should work");
    assert!(
        stdout.contains("empty-falsy"),
        "Empty array should be falsy"
    );
    assert!(
        stdout.contains("full-truthy"),
        "Non-empty array should be truthy"
    );
}

#[test]
fn test_truthy_empty_map_is_falsy() {
    let code = r#"
let empty = map {}
let full = map { "a": 1 }
if empty { print("empty-truthy") } else { print("empty-falsy") }
if full { print("full-truthy") }
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Map truthiness should work");
    assert!(stdout.contains("empty-falsy"), "Empty map should be falsy");
    assert!(
        stdout.contains("full-truthy"),
        "Non-empty map should be truthy"
    );
}

#[test]
fn test_truthy_none_is_falsy() {
    let code = r#"
let none_val = None
let some_val = Some(42)
if none_val { print("none-truthy") } else { print("none-falsy") }
if some_val { print("some-truthy") }
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Option truthiness should work");
    assert!(stdout.contains("none-falsy"), "None should be falsy");
    assert!(stdout.contains("some-truthy"), "Some should be truthy");
}

#[test]
fn test_truthy_in_conditionals() {
    let code = r#"
let query = "site=bear_lake"
let empty = ""

// Short-circuit with truthy check
if query && true {
    print("query-present")
}

if !empty {
    print("empty-absent")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Truthy conditionals should work");
    assert!(
        stdout.contains("query-present"),
        "Non-empty string in && should work"
    );
    assert!(
        stdout.contains("empty-absent"),
        "!empty_string should be true"
    );
}

// ==========================================================================
// Template String Tests (triple-quoted strings with {{}} interpolation)
// ==========================================================================

#[test]
fn test_template_string_basic_interpolation() {
    let code = r#"
let name = "World"
let greeting = """Hello, {{name}}!"""
print(greeting)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Template string basic interpolation should work"
    );
    assert!(
        stdout.contains("Hello, World!"),
        "Should interpolate {{name}}"
    );
}

#[test]
fn test_template_string_css_passthrough() {
    let code = r#"
let css = """h1 { color: blue; }"""
print(css)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "CSS in template string should work");
    assert!(
        stdout.contains("h1 { color: blue; }"),
        "Single braces should pass through unchanged"
    );
}

#[test]
fn test_template_string_for_loop() {
    let code = r#"
let items = ["a", "b", "c"]
let out = """{{#for x in items}}[{{x}}]{{/for}}"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Template for loop should work");
    assert!(stdout.contains("[a]"), "Should iterate first item");
    assert!(stdout.contains("[b]"), "Should iterate second item");
    assert!(stdout.contains("[c]"), "Should iterate third item");
}

#[test]
fn test_template_string_if_condition() {
    let code = r#"
let show = true
let out = """{{#if show}}visible{{/if}}"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Template if condition should work");
    assert!(
        stdout.contains("visible"),
        "Should show content when condition is true"
    );
}

#[test]
fn test_template_string_if_else() {
    let code = r#"
let logged_in = false
let nav = """{{#if logged_in}}profile{{#else}}login{{/if}}"""
print(nav)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Template if-else should work");
    assert!(
        stdout.contains("login"),
        "Should show else branch when condition is false"
    );
    assert!(
        !stdout.contains("profile"),
        "Should not show then branch when condition is false"
    );
}

#[test]
fn test_template_string_escaped_braces() {
    let code = r#"
let out = """Use \{{ and \}} for literal braces"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Template escaped braces should work");
    assert!(
        stdout.contains("{{ and }}"),
        "Should output literal double braces"
    );
}

#[test]
fn test_template_string_complex_expressions() {
    let code = r#"
let items = [
    map { "name": "Widget", "price": 99 }
]
let out = """{{#for item in items}}{{item["name"]}}: ${{item["price"]}}{{/for}}"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Template complex expressions should work");
    assert!(
        stdout.contains("Widget: $99"),
        "Should interpolate map access expressions"
    );
}

#[test]
fn test_template_string_multiline() {
    let code = r#"
let name = "Test"
let page = """
<html>
<body>
    <h1>{{name}}</h1>
</body>
</html>
"""
print(page)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Multiline template string should work");
    assert!(
        stdout.contains("<h1>Test</h1>"),
        "Should interpolate in multiline content"
    );
    assert!(stdout.contains("<html>"), "Should preserve HTML tags");
}

/// @since v0.3.13
#[test]
fn test_template_nested_if_both_true() {
    let code = r#"
let outer = true
let inner = true
let out = """{{#if outer}}{{#if inner}}yes{{/if}}{{/if}}"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Nested if blocks should work");
    assert_eq!(
        stdout.trim(),
        "yes",
        "Both conditions true should render inner content"
    );
}

/// @since v0.3.13
#[test]
fn test_template_nested_if_inner_false() {
    let code = r#"
let outer = true
let inner = false
let out = """{{#if outer}}{{#if inner}}yes{{/if}}no{{/if}}"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Nested if with inner false should work");
    assert_eq!(
        stdout.trim(),
        "no",
        "Inner false should skip inner block but keep outer content"
    );
}

/// @since v0.3.13
#[test]
fn test_template_triple_nested_if() {
    let code = r#"
let a = true
let b = true
let c = true
let out = """{{#if a}}{{#if b}}{{#if c}}deep{{/if}}{{/if}}{{/if}}"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Triple nested if should work");
    assert_eq!(stdout.trim(), "deep", "Three levels deep should render");
}

/// @since v0.3.13
#[test]
fn test_template_nested_if_with_else() {
    let code = r#"
let a = true
let b = false
let out = """{{#if a}}{{#if b}}both{{#else}}only a{{/if}}{{#else}}neither{{/if}}"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Nested if with else should work");
    assert_eq!(stdout.trim(), "only a", "Should render else of inner block");
}

/// @since v0.3.13
#[test]
fn test_template_nested_if_mixed_content() {
    let code = r#"
let show = true
let detail = true
let out = """before{{#if show}} outer {{#if detail}}(detail){{/if}} end{{/if}} after"""
print(out)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Mixed content around nested blocks should work"
    );
    assert_eq!(stdout.trim(), "before outer (detail) end after");
}

#[test]
fn test_get_key_with_two_args() {
    let code = r#"
import { get_key } from "std/collections"
let data = map { "name": "Alice", "age": 30 }

// Get existing key - returns Some
let name = get_key(data, "name")
print(name)

// Get missing key - returns None
let missing = get_key(data, "missing")
print(missing)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "get_key() with 2 args should work");
    assert!(
        stdout.contains("Some(Alice)") || stdout.contains("Option::Some(Alice)"),
        "Should return Some for existing key"
    );
    assert!(
        stdout.contains("None") || stdout.contains("Option::None"),
        "Should return None for missing key"
    );
}

#[test]
fn test_get_key_with_default() {
    let code = r#"
import { get_key } from "std/collections"
let data = map { "name": "Alice" }

// Get existing key with default - returns value
let name = get_key(data, "name", "Unknown")
print(name)

// Get missing key with default - returns default
let city = get_key(data, "city", "Boston")
print(city)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "get_key() with default should work");
    assert!(
        stdout.contains("Alice"),
        "Should return value for existing key"
    );
    assert!(
        stdout.contains("Boston"),
        "Should return default for missing key"
    );
}

#[test]
fn test_null_coalesce_operator() {
    let code = r#"
import { get_key } from "std/collections"
let data = map { "name": "Alice" }

// ?? unwraps Some value
let name = get_key(data, "name") ?? "Default"
print(name)

// ?? returns right side for None
let city = get_key(data, "city") ?? "Unknown"
print(city)

// ?? with built-in None
let x = None
let result = x ?? "Fallback"
print(result)

// ?? with Some
let y = Some(42)
let val = y ?? 0
print(val)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "?? operator should work");
    assert!(stdout.contains("Alice"), "Should unwrap Some to Alice");
    assert!(stdout.contains("Unknown"), "Should return default for None");
    assert!(
        stdout.contains("Fallback"),
        "Should return fallback for explicit None"
    );
    assert!(stdout.contains("42"), "Should unwrap Some(42)");
}

// ============================================================================
// String Functions: replace_chars, remove_chars, keep_chars
// ============================================================================

#[test]
fn test_replace_chars_basic() {
    let code = r#"
import { replace_chars } from "std/string"
let result = replace_chars("hello world", " ", "-")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "replace_chars should work");
    assert!(
        stdout.contains("hello-world"),
        "Should replace space with hyphen"
    );
}

#[test]
fn test_replace_chars_multiple() {
    let code = r#"
import { replace_chars } from "std/string"
let result = replace_chars("a.b,c;d", ".,;", "-")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "replace_chars with multiple chars should work"
    );
    assert!(
        stdout.contains("a-b-c-d"),
        "Should replace all specified chars"
    );
}

#[test]
fn test_replace_chars_empty_replacement() {
    let code = r#"
import { replace_chars } from "std/string"
let result = replace_chars("a1b2c3", "123", "")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "replace_chars with empty replacement should work"
    );
    assert!(stdout.contains("abc"), "Should remove digits");
}

#[test]
fn test_remove_chars_basic() {
    let code = r#"
import { remove_chars } from "std/string"
let result = remove_chars("hello123world", "0123456789")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "remove_chars should work");
    assert!(stdout.contains("helloworld"), "Should remove all digits");
}

#[test]
fn test_remove_chars_punctuation() {
    let code = r#"
import { remove_chars } from "std/string"
let result = remove_chars("Hello, World!", ",.! ")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "remove_chars with punctuation should work");
    assert!(
        stdout.contains("HelloWorld"),
        "Should remove punctuation and spaces"
    );
}

#[test]
fn test_keep_chars_basic() {
    let code = r#"
import { keep_chars } from "std/string"
let result = keep_chars("abc123def456", "0123456789")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "keep_chars should work");
    assert!(stdout.contains("123456"), "Should keep only digits");
}

#[test]
fn test_keep_chars_letters() {
    let code = r#"
import { keep_chars } from "std/string"
let result = keep_chars("H3ll0 W0rld!", "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "keep_chars with letters should work");
    assert!(stdout.contains("HllWrld"), "Should keep only letters");
}

#[test]
fn test_replace_all_function() {
    let code = r#"
import { replace_all } from "std/string"
let result = replace_all("foo bar foo baz foo", "foo", "qux")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "replace_all should work");
    assert!(
        stdout.contains("qux bar qux baz qux"),
        "Should replace all occurrences"
    );
}

// ============================================================================
// Regex Functions: replace_pattern, matches_pattern, find_pattern, etc.
// ============================================================================

#[test]
fn test_replace_pattern_basic() {
    let code = r#"
import { replace_pattern } from "std/string"
let result = replace_pattern("hello 123 world 456", r"[0-9]+", "NUM")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "replace_pattern should work");
    assert!(
        stdout.contains("hello NUM world NUM"),
        "Should replace all number sequences"
    );
}

#[test]
fn test_replace_pattern_whitespace() {
    let code = r#"
import { replace_pattern } from "std/string"
let result = replace_pattern("hello   world  test", r"\s+", " ")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "replace_pattern with whitespace should work");
    assert!(
        stdout.contains("hello world test"),
        "Should normalize whitespace"
    );
}

#[test]
fn test_replace_pattern_slugify() {
    let code = r#"
import { replace_pattern, to_lower, trim_chars } from "std/string"
let title = "Hello, World! (2024)"
let slug = to_lower(title)
let slug = replace_pattern(slug, r"[^a-z0-9]+", "-")
let slug = trim_chars(slug, "-")
print(slug)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "replace_pattern for slugify should work");
    assert!(
        stdout.contains("hello-world-2024"),
        "Should create a proper slug"
    );
}

#[test]
fn test_matches_pattern_basic() {
    let code = r#"
import { matches_pattern } from "std/string"
print(matches_pattern("hello123", r"[0-9]+"))
print(matches_pattern("hello", r"[0-9]+"))
print(matches_pattern("test@example.com", r"@"))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "matches_pattern should work");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true", "Should match digits");
    assert_eq!(lines[1], "false", "Should not match when no digits");
    assert_eq!(lines[2], "true", "Should match @ symbol");
}

#[test]
fn test_matches_pattern_email() {
    let code = r#"
import { matches_pattern } from "std/string"
let email_pattern = r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$"
print(matches_pattern("test@example.com", email_pattern))
print(matches_pattern("invalid-email", email_pattern))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "matches_pattern for email should work");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true", "Should match valid email");
    assert_eq!(lines[1], "false", "Should not match invalid email");
}

#[test]
fn test_find_pattern_basic() {
    let code = r#"
import { find_pattern } from "std/string"
let result = find_pattern("hello 123 world", r"[0-9]+")
match result {
    Some(m) => print("found: " + m),
    None => print("not found")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "find_pattern should work");
    assert!(stdout.contains("found: 123"), "Should find the number");
}

#[test]
fn test_find_pattern_no_match() {
    let code = r#"
import { find_pattern } from "std/string"
let result = find_pattern("hello world", r"[0-9]+")
match result {
    Some(m) => print("found: " + m),
    None => print("not found")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "find_pattern with no match should work");
    assert!(stdout.contains("not found"), "Should return None");
}

#[test]
fn test_find_all_pattern_basic() {
    let code = r#"
import { find_all_pattern } from "std/string"
let matches = find_all_pattern("a1b2c3d4", r"[0-9]")
print(len(matches))
for m in matches {
    print(m)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "find_all_pattern should work");
    assert!(stdout.contains("4"), "Should find 4 matches");
    assert!(stdout.contains("1"), "Should find 1");
    assert!(stdout.contains("2"), "Should find 2");
    assert!(stdout.contains("3"), "Should find 3");
    assert!(stdout.contains("4"), "Should find 4");
}

#[test]
fn test_find_all_pattern_words() {
    let code = r#"
import { find_all_pattern } from "std/string"
let matches = find_all_pattern("foo bar baz foo qux foo", r"foo")
print(len(matches))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "find_all_pattern for words should work");
    assert!(stdout.contains("3"), "Should find 3 occurrences of foo");
}

#[test]
fn test_split_pattern_basic() {
    let code = r#"
import { split_pattern } from "std/string"
let parts = split_pattern("a1b2c3d", r"[0-9]")
print(len(parts))
for p in parts {
    print(p)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "split_pattern should work");
    assert!(stdout.contains("4"), "Should have 4 parts");
    assert!(stdout.contains("a"), "Should contain a");
    assert!(stdout.contains("b"), "Should contain b");
    assert!(stdout.contains("c"), "Should contain c");
    assert!(stdout.contains("d"), "Should contain d");
}

#[test]
fn test_split_pattern_whitespace() {
    let code = r#"
import { split_pattern } from "std/string"
let parts = split_pattern("hello   world  test", r"\s+")
print(len(parts))
for p in parts {
    print("[" + p + "]")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "split_pattern with whitespace should work");
    assert!(stdout.contains("3"), "Should have 3 parts");
    assert!(stdout.contains("[hello]"), "Should contain hello");
    assert!(stdout.contains("[world]"), "Should contain world");
    assert!(stdout.contains("[test]"), "Should contain test");
}

// ============================================================================
// Regex Capture Groups
// ============================================================================

#[test]
fn test_capture_pattern_basic() {
    let code = r#"
import { capture_pattern } from "std/string"
let result = capture_pattern("Bear Lake (1042)", r"(.+) \((\d+)\)")
match result {
    Some(groups) => {
        print(groups[0])
        print(groups[1])
        print(groups[2])
    },
    None => print("no match")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "capture_pattern should work");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "Bear Lake (1042)");
    assert_eq!(lines[1], "Bear Lake");
    assert_eq!(lines[2], "1042");
}

#[test]
fn test_capture_pattern_no_match() {
    let code = r#"
import { capture_pattern } from "std/string"
let result = capture_pattern("no numbers here", r"(\d+)-(\d+)")
match result {
    Some(groups) => print("matched"),
    None => print("none")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("none"));
}

#[test]
fn test_capture_pattern_date() {
    let code = r#"
import { capture_pattern } from "std/string"
let result = capture_pattern("2024-01-15", r"(\d{4})-(\d{2})-(\d{2})")
match result {
    Some(groups) => {
        print(groups[1])
        print(groups[2])
        print(groups[3])
    },
    None => print("no match")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "2024");
    assert_eq!(lines[1], "01");
    assert_eq!(lines[2], "15");
}

#[test]
fn test_capture_pattern_optional_group() {
    let code = r#"
import { capture_pattern } from "std/string"
let result = capture_pattern("foo123", r"(\w+?)(\d+)(\s+)?")
match result {
    Some(groups) => {
        print(len(groups))
        print(groups[1])
        print(groups[2])
        print("[" + groups[3] + "]")
    },
    None => print("no match")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "4");
    assert_eq!(lines[1], "foo");
    assert_eq!(lines[2], "123");
    assert_eq!(
        lines[3], "[]",
        "Unmatched optional group should be empty string"
    );
}

#[test]
fn test_capture_all_pattern_basic() {
    let code = r#"
import { capture_all_pattern } from "std/string"
let results = capture_all_pattern("2024-01 and 2025-02", r"(\d{4})-(\d{2})")
print(len(results))
for groups in results {
    print(groups[1] + "/" + groups[2])
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "2");
    assert_eq!(lines[1], "2024/01");
    assert_eq!(lines[2], "2025/02");
}

#[test]
fn test_capture_all_pattern_no_match() {
    let code = r#"
import { capture_all_pattern } from "std/string"
let results = capture_all_pattern("no matches", r"(\d+)-(\d+)")
print(len(results))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("0"));
}

#[test]
fn test_capture_named_pattern_basic() {
    let code = r#"
import { capture_named_pattern } from "std/string"
let result = capture_named_pattern("2024-01-15", r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})")
match result {
    Some(m) => {
        print(m["year"])
        print(m["month"])
        print(m["day"])
    },
    None => print("no match")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "2024");
    assert_eq!(lines[1], "01");
    assert_eq!(lines[2], "15");
}

#[test]
fn test_capture_named_pattern_no_match() {
    let code = r#"
import { capture_named_pattern } from "std/string"
let result = capture_named_pattern("no digits", r"(?P<num>\d+)")
match result {
    Some(m) => print("matched"),
    None => print("none")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("none"));
}

#[test]
fn test_capture_named_pattern_mixed() {
    let code = r#"
import { capture_named_pattern } from "std/string"
let result = capture_named_pattern("hello 42 world", r"(\w+) (?P<num>\d+) (\w+)")
match result {
    Some(m) => {
        print(m["0"])
        print(m["1"])
        print(m["num"])
        print(m["3"])
    },
    None => print("no match")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "hello 42 world");
    assert_eq!(lines[1], "hello");
    assert_eq!(lines[2], "42");
    assert_eq!(lines[3], "world");
}

// ============================================================================
// Map Destructuring
// ============================================================================

#[test]
fn test_map_destructuring_basic() {
    let code = r#"
let data = map { "name": "Alice", "age": 30 }
let { name, age } = data
print("{name} is {age}")
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Map destructuring should work");
    assert!(stdout.contains("Alice is 30"));
}

#[test]
fn test_map_destructuring_rename() {
    let code = r#"
let data = map { "name": "Alice" }
let { name: n } = data
print(n)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Alice"));
}

#[test]
fn test_map_destructuring_nested() {
    let code = r#"
let data = map { "user": { "name": "Bob" } }
let { user: { name } } = data
print(name)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Bob"));
}

#[test]
fn test_map_destructuring_struct() {
    let code = r#"
struct User { name: String }
let u = User { name: "Eve" }
let { name } = u
print(name)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Eve"));
}

#[test]
fn test_map_destructuring_missing_key() {
    let code = r#"
let data = map { "name": "Alice" }
let { missing } = data
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail on missing key");
    assert!(
        stderr.contains("Pattern destructuring failed")
            || stderr.contains("error")
            || stderr.contains("Error"),
        "Should report a destructuring error: {stderr}"
    );
}

#[test]
fn test_map_destructuring_in_match() {
    let code = r#"
let data = map { "x": 1, "y": 2 }
match data {
    { x, y } => print("{x},{y}"),
    _ => print("no match")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("1,2"));
}

// ============================================================================
// Rest/Spread Patterns
// ============================================================================

#[test]
fn test_array_exact_destructuring() {
    let code = r#"
let [a, b, c] = [1, 2, 3]
print(b)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("2"));
}

#[test]
fn test_array_rest_pattern() {
    let code = r#"
let [first, ...rest] = [1, 2, 3, 4]
print(first)
print(len(rest))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Array rest pattern should work");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "3");
}

#[test]
fn test_array_rest_empty() {
    let code = r#"
let [a, ...rest] = [1]
print(a)
print(len(rest))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "1");
    assert_eq!(lines[1], "0");
}

#[test]
fn test_map_rest_pattern() {
    let code = r#"
let { name, ...other } = map { "name": "Alice", "age": 30, "city": "NYC" }
print(name)
print(other["age"])
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Map rest pattern should work");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "Alice");
    assert_eq!(lines[1], "30");
}

#[test]
fn test_array_rest_with_multiple_leading() {
    let code = r#"
let [a, b, ...rest] = [10, 20, 30, 40, 50]
print(a)
print(b)
print(len(rest))
print(rest[0])
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
    assert_eq!(lines[2], "3");
    assert_eq!(lines[3], "30");
}

// ============================================================================
// For-Loop Destructuring
// ============================================================================

#[test]
fn test_for_loop_array_destructuring() {
    let code = r#"
import { entries } from "std/collections"
let data = map { "a": 1, "b": 2 }
for [k, v] in entries(data) {
    print("{k}={v}")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "For-loop array destructuring should work");
    // Map order is not guaranteed, so check both are present
    assert!(stdout.contains("a=1"));
    assert!(stdout.contains("b=2"));
}

#[test]
fn test_for_loop_map_destructuring() {
    let code = r#"
let users = [map { "name": "Alice" }, map { "name": "Bob" }]
for { name } in users {
    print(name)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "For-loop map destructuring should work");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "Alice");
    assert_eq!(lines[1], "Bob");
}

#[test]
fn test_for_loop_tuple_destructuring() {
    let code = r#"
let pairs = [[1, "one"], [2, "two"], [3, "three"]]
for [num, word] in pairs {
    print("{num}: {word}")
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "1: one");
    assert_eq!(lines[1], "2: two");
    assert_eq!(lines[2], "3: three");
}

// ============================================================================
// Higher-Order Functions: filter, transform
// ============================================================================

#[test]
fn test_filter_basic() {
    let code = r#"
fn is_even(x) {
    return x % 2 == 0
}

let nums = [1, 2, 3, 4, 5, 6]
let evens = filter(nums, is_even)
print(len(evens))
for n in evens {
    print(n)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "filter should work");
    assert!(stdout.contains("3"), "Should have 3 even numbers");
    assert!(stdout.contains("2"), "Should contain 2");
    assert!(stdout.contains("4"), "Should contain 4");
    assert!(stdout.contains("6"), "Should contain 6");
}

#[test]
fn test_filter_empty_result() {
    let code = r#"
fn is_negative(x) {
    return x < 0
}

let nums = [1, 2, 3]
let negatives = filter(nums, is_negative)
print(len(negatives))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "filter with no matches should work");
    assert!(stdout.contains("0"), "Should have 0 elements");
}

#[test]
fn test_filter_all_match() {
    let code = r#"
fn is_positive(x) {
    return x > 0
}

let nums = [1, 2, 3]
let positives = filter(nums, is_positive)
print(len(positives))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "filter with all matches should work");
    assert!(stdout.contains("3"), "Should have 3 elements");
}

#[test]
fn test_transform_basic() {
    let code = r#"
fn double(x) {
    return x * 2
}

let nums = [1, 2, 3]
let doubled = transform(nums, double)
for n in doubled {
    print(n)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "transform should work");
    assert!(stdout.contains("2"), "Should contain 2");
    assert!(stdout.contains("4"), "Should contain 4");
    assert!(stdout.contains("6"), "Should contain 6");
}

#[test]
fn test_transform_to_string() {
    let code = r#"
fn to_greeting(name) {
    return "Hello, " + name + "!"
}

let names = ["Alice", "Bob"]
let greetings = transform(names, to_greeting)
for g in greetings {
    print(g)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "transform with strings should work");
    assert!(
        stdout.contains("Hello, Alice!"),
        "Should contain Alice greeting"
    );
    assert!(
        stdout.contains("Hello, Bob!"),
        "Should contain Bob greeting"
    );
}

#[test]
fn test_transform_empty_array() {
    let code = r#"
fn double(x) {
    return x * 2
}

let nums = []
let doubled = transform(nums, double)
print(len(doubled))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "transform on empty array should work");
    assert!(stdout.contains("0"), "Should have 0 elements");
}

#[test]
fn test_filter_and_transform_chained() {
    let code = r#"
fn is_even(x) {
    return x % 2 == 0
}

fn double(x) {
    return x * 2
}

let nums = [1, 2, 3, 4, 5]
let evens = filter(nums, is_even)
let doubled = transform(evens, double)
for n in doubled {
    print(n)
}
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "filter then transform should work");
    assert!(stdout.contains("4"), "Should contain 4 (2*2)");
    assert!(stdout.contains("8"), "Should contain 8 (4*2)");
}

// ============================================================================
// Round with Decimals
// ============================================================================

#[test]
fn test_round_without_decimals() {
    let code = r#"
print(round(3.7))
print(round(3.2))
print(round(-2.5))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "round without decimals should work");
    assert!(stdout.contains("4"), "Should round 3.7 to 4");
    assert!(stdout.contains("3"), "Should round 3.2 to 3");
    assert!(
        stdout.contains("-2") || stdout.contains("-3"),
        "Should round -2.5"
    );
}

#[test]
fn test_round_with_decimals() {
    let code = r#"
print(round(3.14159, 2))
print(round(3.14159, 4))
print(round(2.5, 0))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "round with decimals should work");
    assert!(stdout.contains("3.14"), "Should round to 3.14");
    assert!(stdout.contains("3.1416"), "Should round to 3.1416");
}

#[test]
fn test_round_with_zero_decimals() {
    let code = r#"
let result = round(3.7, 0)
print(result)
// Verify it's a float by doing float math
let check = result + 0.5
print(check)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "round with 0 decimals should work");
    assert!(stdout.contains("4"), "Should round to 4");
    assert!(
        stdout.contains("4.5"),
        "Should be able to do float math (4 + 0.5 = 4.5)"
    );
}

// ============================================================================
// First/Last with Default
// ============================================================================

#[test]
fn test_first_without_default() {
    let code = r#"
import { first } from "std/collections"
let nums = [10, 20, 30]
let result = first(nums)
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "first without default should work");
    assert!(stdout.contains("Some(10)"), "Should return Some(10)");
}

#[test]
fn test_first_with_default_value_exists() {
    let code = r#"
import { first } from "std/collections"
let nums = [10, 20, 30]
let result = first(nums, -1)
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "first with default should return value");
    assert!(stdout.contains("10"), "Should return 10 directly");
    assert!(!stdout.contains("Some"), "Should not return Option");
}

#[test]
fn test_first_with_default_empty_array() {
    let code = r#"
import { first } from "std/collections"
let nums = []
let result = first(nums, -1)
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "first with default on empty array should return default"
    );
    assert!(stdout.contains("-1"), "Should return default -1");
}

#[test]
fn test_last_without_default() {
    let code = r#"
import { last } from "std/collections"
let nums = [10, 20, 30]
let result = last(nums)
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "last without default should work");
    assert!(stdout.contains("Some(30)"), "Should return Some(30)");
}

#[test]
fn test_last_with_default_value_exists() {
    let code = r#"
import { last } from "std/collections"
let nums = [10, 20, 30]
let result = last(nums, -1)
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "last with default should return value");
    assert!(stdout.contains("30"), "Should return 30 directly");
    assert!(!stdout.contains("Some"), "Should not return Option");
}

#[test]
fn test_last_with_default_empty_array() {
    let code = r#"
import { last } from "std/collections"
let nums = []
let result = last(nums, -1)
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "last with default on empty array should return default"
    );
    assert!(stdout.contains("-1"), "Should return default -1");
}

// ============================================================================
// Pipe Operator (|>) Tests
// ============================================================================

#[test]
fn test_pipe_basic_builtin() {
    let code = r#"
let x = 5 |> str
print(x)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "pipe with builtin should succeed");
    assert!(stdout.contains("5"), "Should convert 5 to string");
}

#[test]
fn test_pipe_with_args() {
    let code = r#"
import { split } from "std/string"
let parts = "a,b,c" |> split(",")
print(parts)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "pipe with extra args should succeed");
    assert!(
        stdout.contains("a") && stdout.contains("b") && stdout.contains("c"),
        "Should split into parts: {}",
        stdout
    );
}

#[test]
fn test_pipe_chained() {
    let code = r#"
import { trim, to_lower } from "std/string"
let result = "  Hello World  " |> trim |> to_lower
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "chained pipes should succeed");
    assert!(
        stdout.contains("hello world"),
        "Should trim and lowercase: {}",
        stdout
    );
}

#[test]
fn test_pipe_multi_arg_chain() {
    let code = r#"
import { split, join } from "std/string"
let result = "a,b,c" |> split(",") |> join("-")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "multi-arg chain should succeed");
    assert!(
        stdout.contains("a-b-c"),
        "Should split then rejoin: {}",
        stdout
    );
}

#[test]
fn test_pipe_user_defined_function() {
    let code = r#"
fn double(x) { return x * 2 }
let result = 5 |> double
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "pipe with user function should succeed");
    assert!(stdout.contains("10"), "Should double 5 to 10: {}", stdout);
}

#[test]
fn test_pipe_long_chain() {
    let code = r#"
import { split, join, trim, to_lower } from "std/string"
let result = "  Hello, World  " |> trim |> to_lower |> split(",") |> join(" and")
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "long pipe chain should succeed");
    assert!(
        stdout.contains("hello") && stdout.contains("and"),
        "Should process through full chain: {}",
        stdout
    );
}

#[test]
fn test_pipe_with_len() {
    let code = r#"
let result = "hello" |> len
print(result)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "pipe with len should succeed");
    assert!(stdout.contains("5"), "Should get length 5: {}", stdout);
}

#[test]
fn test_pipe_error_invalid_rhs() {
    let code = r#"
let result = 5 |> 3
"#;
    let (_, _, exit_code) = run_ntnt_code(code);
    assert_ne!(
        exit_code, 0,
        "pipe with non-function RHS should produce a parse error"
    );
}

// ============================================================================
// Route Pattern Auto-Detection Tests
// ============================================================================

#[test]
fn test_route_auto_detect_single_param() {
    let code = r#"
import { json } from "std/http/server"
fn get_user(req) { return json(map { "ok": true }) }

// {id} should be auto-detected as a route parameter, not interpolated
get("/users/{id}", get_user)
print("registered")
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "route auto-detection should handle {{id}} without raw string.\nstderr: {}",
        stderr
    );
    assert!(
        stdout.contains("registered"),
        "Should register route successfully: {}",
        stdout
    );
}

#[test]
fn test_route_auto_detect_multiple_params() {
    let code = r#"
import { json } from "std/http/server"
fn handler(req) { return json(map { "ok": true }) }

// Multiple {param} placeholders in one route
post("/api/{category}/items/{id}", handler)
print("registered")
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "route auto-detection should handle multiple params.\nstderr: {}",
        stderr
    );
    assert!(stdout.contains("registered"));
}

#[test]
fn test_route_auto_detect_all_http_methods() {
    let code = r#"
import { json } from "std/http/server"
fn handler(req) { return json(map { "ok": true }) }

get("/items/{id}", handler)
post("/items/{id}", handler)
put("/items/{id}", handler)
delete("/items/{id}", handler)
patch("/items/{id}", handler)
print("all methods registered")
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "all HTTP methods should support auto-detection.\nstderr: {}",
        stderr
    );
    assert!(stdout.contains("all methods registered"));
}

#[test]
fn test_route_auto_detect_with_raw_string_still_works() {
    let code = r#"
import { json } from "std/http/server"
fn handler(req) { return json(map { "ok": true }) }

// Raw strings should still work (backward compatibility)
get(r"/users/{id}", handler)
print("raw string route registered")
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "raw string routes should still work");
    assert!(stdout.contains("raw string route registered"));
}

#[test]
fn test_route_auto_detect_plain_string_no_params() {
    let code = r#"
import { json } from "std/http/server"
fn handler(req) { return json(map { "ok": true }) }

// Plain strings without params should work as before
get("/", handler)
get("/about", handler)
print("plain routes registered")
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "plain string routes should work");
    assert!(stdout.contains("plain routes registered"));
}

#[test]
fn test_route_auto_detect_no_false_positive_outside_route() {
    // {name} outside a route call should still be interpolation
    let code = r#"
let name = "world"
let greeting = "hello {name}"
print(greeting)
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "interpolation outside routes should work normally"
    );
    assert!(
        stdout.contains("hello world"),
        "String interpolation should still work outside route calls: {}",
        stdout
    );
}

// ============================================================================
// SQLite integration tests
// ============================================================================

#[test]
fn test_sqlite_connect_and_query() {
    let code = r#"
import { connect, query, execute, close } from "std/db/sqlite"

match connect(":memory:") {
    Ok(db) => {
        execute(db, "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)", [])
        execute(db, "INSERT INTO test (name) VALUES (?)", ["Alice"])
        execute(db, "INSERT INTO test (name) VALUES (?)", ["Bob"])
        match query(db, "SELECT * FROM test ORDER BY id", []) {
            Ok(rows) => {
                print(len(rows))
                print(rows[0]["name"])
                print(rows[1]["name"])
            },
            Err(e) => print("ERROR: " + e)
        }
        close(db)
    },
    Err(e) => print("CONNECT ERROR: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "stderr: {}", stderr);
    assert!(stdout.contains("2"), "Should have 2 rows: {}", stdout);
    assert!(stdout.contains("Alice"), "Should contain Alice: {}", stdout);
    assert!(stdout.contains("Bob"), "Should contain Bob: {}", stdout);
}

#[test]
fn test_sqlite_query_one() {
    let code = r#"
import { connect, query_one, execute, close } from "std/db/sqlite"

match connect(":memory:") {
    Ok(db) => {
        execute(db, "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)", [])
        execute(db, "INSERT INTO test (name) VALUES (?)", ["Alice"])

        match query_one(db, "SELECT * FROM test WHERE id = ?", [1]) {
            Ok(row) => print(row["name"]),
            Err(e) => print("ERROR: " + e)
        }

        match query_one(db, "SELECT * FROM test WHERE id = ?", [999]) {
            Ok(row) => print("missing:" + str(row)),
            Err(e) => print("ERROR: " + e)
        }

        close(db)
    },
    Err(e) => print("CONNECT ERROR: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "stderr: {}", stderr);
    assert!(stdout.contains("Alice"), "Should find Alice: {}", stdout);
    assert!(
        stdout.contains("missing:"),
        "Missing row should return value: {}",
        stdout
    );
}

#[test]
fn test_sqlite_transaction() {
    let code = r#"
import { connect, query, execute, close, begin, commit, rollback } from "std/db/sqlite"

match connect(":memory:") {
    Ok(db) => {
        execute(db, "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)", [])

        // Insert and rollback
        begin(db)
        execute(db, "INSERT INTO test (name) VALUES (?)", ["Rolled Back"])
        rollback(db)

        match query(db, "SELECT * FROM test", []) {
            Ok(rows) => print("after_rollback:" + str(len(rows))),
            Err(e) => print("ERROR: " + e)
        }

        // Insert and commit
        begin(db)
        execute(db, "INSERT INTO test (name) VALUES (?)", ["Committed"])
        commit(db)

        match query(db, "SELECT * FROM test", []) {
            Ok(rows) => print("after_commit:" + str(len(rows))),
            Err(e) => print("ERROR: " + e)
        }

        close(db)
    },
    Err(e) => print("CONNECT ERROR: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "stderr: {}", stderr);
    assert!(
        stdout.contains("after_rollback:0"),
        "Rollback should remove row: {}",
        stdout
    );
    assert!(
        stdout.contains("after_commit:1"),
        "Commit should keep row: {}",
        stdout
    );
}

#[test]
fn test_sqlite_parameterized_types() {
    let code = r#"
import { connect, query, execute, close } from "std/db/sqlite"

match connect(":memory:") {
    Ok(db) => {
        execute(db, "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, score REAL, active INTEGER)", [])
        execute(db, "INSERT INTO test (name, score, active) VALUES (?, ?, ?)", ["Alice", 95.5, true])

        match query(db, "SELECT * FROM test", []) {
            Ok(rows) => {
                let row = rows[0]
                print(row["name"])
                print(row["score"])
            },
            Err(e) => print("ERROR: " + e)
        }

        close(db)
    },
    Err(e) => print("CONNECT ERROR: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "stderr: {}", stderr);
    assert!(stdout.contains("Alice"), "name should be Alice: {}", stdout);
    assert!(stdout.contains("95.5"), "score should be 95.5: {}", stdout);
}

// ============================================================================
// Error Message Tests (Phase 7.6)
// ============================================================================

#[test]
fn test_undefined_variable_suggestion() {
    let code = r#"
let users = [1, 2, 3]
print(usres)
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail with undefined variable");
    assert!(
        stderr.contains("E006"),
        "Should contain error code E006: {}",
        stderr
    );
    assert!(
        stderr.contains("usres"),
        "Should mention the misspelled name: {}",
        stderr
    );
    assert!(
        stderr.contains("users"),
        "Should suggest 'users': {}",
        stderr
    );
}

#[test]
fn test_undefined_function_suggestion() {
    let code = r#"
prnt("hello")
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail with undefined variable");
    assert!(
        stderr.contains("E006") || stderr.contains("E007"),
        "Should contain error code: {}",
        stderr
    );
    assert!(
        stderr.contains("prnt"),
        "Should mention the misspelled name: {}",
        stderr
    );
    assert!(
        stderr.contains("print"),
        "Should suggest 'print': {}",
        stderr
    );
}

#[test]
fn test_arity_mismatch_shows_function_name() {
    let code = r#"
fn add(a, b) {
    return a + b
}
print(add(1, 2, 3))
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail with arity mismatch");
    assert!(
        stderr.contains("E008"),
        "Should contain error code E008: {}",
        stderr
    );
    assert!(
        stderr.contains("add"),
        "Should mention function name 'add': {}",
        stderr
    );
    assert!(
        stderr.contains("expected 2"),
        "Should show expected count: {}",
        stderr
    );
    assert!(
        stderr.contains("got 3"),
        "Should show actual count: {}",
        stderr
    );
}

#[test]
fn test_parser_error_has_error_code() {
    let code = r#"
let x = 10
if x > 5 {
    print("hello"
}
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail with parser error");
    assert!(
        stderr.contains("E002"),
        "Should contain error code E002: {}",
        stderr
    );
}

#[test]
fn test_no_suggestion_for_distant_name() {
    let code = r#"
let users = [1, 2, 3]
print(completely_wrong_name)
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail with undefined variable");
    assert!(
        !stderr.contains("Did you mean"),
        "Should NOT suggest anything for distant names: {}",
        stderr
    );
}

// ===========================================================================
// ? operator tests
// ===========================================================================

#[test]
fn test_try_operator_unwraps_ok() {
    let code = r#"
fn get_value() {
    return Ok(42)
}

fn main() {
    let val = get_value()?
    print(val)
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("42"), "Should print 42: stdout={}", stdout);
}

#[test]
fn test_try_operator_unwraps_some() {
    let code = r#"
fn find_item() {
    return Some(99)
}

fn main() {
    let val = find_item()?
    print(val)
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("99"), "Should print 99: stdout={}", stdout);
}

#[test]
fn test_try_operator_early_returns_err() {
    let code = r#"
fn failing() {
    return Err("something went wrong")
}

fn main() {
    let val = failing()?
    print("should not reach here")
    return Ok("done")
}

let result = main()
print(result)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        !stdout.contains("should not reach here"),
        "Should have early-returned: stdout={}",
        stdout
    );
    assert!(
        stdout.contains("Err"),
        "Should print the Err value: stdout={}",
        stdout
    );
}

#[test]
fn test_try_operator_early_returns_none() {
    let code = r#"
fn find_nothing() {
    return None
}

fn main() {
    let val = find_nothing()?
    print("should not reach here")
    return Some("done")
}

let result = main()
print(result)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        !stdout.contains("should not reach here"),
        "Should have early-returned: stdout={}",
        stdout
    );
    assert!(
        stdout.contains("None"),
        "Should print None: stdout={}",
        stdout
    );
}

#[test]
fn test_try_operator_passthrough_non_result() {
    let code = r#"
fn get_number() {
    return 42
}

fn main() {
    let val = get_number()?
    print(val)
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Should succeed (passthrough): stderr={}",
        stderr
    );
    assert!(stdout.contains("42"), "Should print 42: stdout={}", stdout);
}

#[test]
fn test_try_operator_chained() {
    let code = r#"
fn step1() {
    return Ok(Ok(10))
}

fn main() {
    let inner = step1()?
    let val = inner?
    print(val)
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("10"), "Should print 10: stdout={}", stdout);
}

// ===========================================================================
// otherwise keyword tests
// ===========================================================================

#[test]
fn test_otherwise_unwraps_ok() {
    let code = r#"
fn main() {
    let x = Ok(42) otherwise { return "fallback" }
    print(x)
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("42"), "Should print 42: stdout={}", stdout);
}

#[test]
fn test_otherwise_unwraps_some() {
    let code = r#"
fn main() {
    let x = Some("hello") otherwise { return "fallback" }
    print(x)
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("hello"),
        "Should print hello: stdout={}",
        stdout
    );
}

#[test]
fn test_otherwise_executes_on_err() {
    let code = r#"
fn main() {
    let x = Err("fail") otherwise {
        print("error handled: {err}")
        return "default"
    }
    print("should not reach here")
}

let result = main()
print("result: {result}")
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("error handled: fail"),
        "Should print error message with err bound: stdout={}",
        stdout
    );
    assert!(
        !stdout.contains("should not reach here"),
        "Should have diverged: stdout={}",
        stdout
    );
    assert!(
        stdout.contains("result: default"),
        "Should return default: stdout={}",
        stdout
    );
}

#[test]
fn test_otherwise_executes_on_none() {
    let code = r#"
fn main() {
    let x = None otherwise {
        return "nothing found"
    }
    print("should not reach here")
}

let result = main()
print(result)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("nothing found"),
        "Should print nothing found: stdout={}",
        stdout
    );
    assert!(
        !stdout.contains("should not reach here"),
        "Should have diverged: stdout={}",
        stdout
    );
}

#[test]
fn test_otherwise_err_is_bound() {
    let code = r#"
fn main() {
    let x = Err("db connection failed") otherwise {
        print("caught: {err}")
        return -1
    }
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("caught: db connection failed"),
        "Should have err bound to error value: stdout={}",
        stdout
    );
}

#[test]
fn test_otherwise_non_diverging_block_errors() {
    let code = r#"
fn main() {
    let x = Err("fail") otherwise {
        print("error")
    }
}

main()
"#;
    let (_, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail: non-diverging otherwise");
    assert!(
        stderr.contains("otherwise block must diverge"),
        "Should say must diverge: stderr={}",
        stderr
    );
}

#[test]
fn test_otherwise_single_expression_form() {
    let code = r#"
fn main() {
    let x = Err("fail") otherwise return "handled"
    print("should not reach here")
}

let result = main()
print(result)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("handled"),
        "Should print handled: stdout={}",
        stdout
    );
    assert!(
        !stdout.contains("should not reach here"),
        "Should have diverged: stdout={}",
        stdout
    );
}

#[test]
fn test_otherwise_with_break_in_loop() {
    let code = r#"
let items = [Ok(1), Ok(2), Err("bad"), Ok(4)]
let mut sum = 0

for item in items {
    let val = item otherwise {
        print("breaking on error: {err}")
        break
    }
    sum = sum + val
}

print("sum: {sum}")
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("breaking on error: bad"),
        "Should print break message: stdout={}",
        stdout
    );
    assert!(
        stdout.contains("sum: 3"),
        "Should sum first two values: stdout={}",
        stdout
    );
}

#[test]
fn test_otherwise_with_continue_in_loop() {
    let code = r#"
let items = [Ok(1), Err("skip"), Ok(3), Err("skip2"), Ok(5)]
let mut sum = 0

for item in items {
    let val = item otherwise {
        continue
    }
    sum = sum + val
}

print("sum: {sum}")
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("sum: 9"),
        "Should sum Ok values (1+3+5=9): stdout={}",
        stdout
    );
}

#[test]
fn test_otherwise_passthrough_non_result() {
    let code = r#"
fn main() {
    let x = 42 otherwise { return "fallback" }
    print(x)
}

main()
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Should succeed (passthrough): stderr={}",
        stderr
    );
    assert!(stdout.contains("42"), "Should print 42: stdout={}", stdout);
}

// ============================================================================
// Anonymous Functions / Closures (Phase 7.3)
// ============================================================================

#[test]
fn test_closure_single_expression() {
    let code = r#"
let double = fn(x) { x * 2 }
print(double(5))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("10"),
        "double(5) should be 10: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_multi_statement() {
    let code = r#"
let process = fn(x) {
    let y = x + 10
    return y * 2
}
print(process(5))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("30"),
        "process(5) should be 30: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_no_params() {
    let code = r#"
let greet = fn() { "hello" }
print(greet())
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("hello"),
        "greet() should return hello: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_multiple_params() {
    let code = r#"
let add = fn(a, b) { a + b }
print(add(3, 4))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("7"),
        "add(3,4) should be 7: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_typed_params() {
    let code = r#"
let multiply = fn(a: Int, b: Int) -> Int { a * b }
print(multiply(6, 7))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("42"),
        "multiply(6,7) should be 42: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_capture() {
    let code = r#"
let n = 10
let add_n = fn(x) { x + n }
print(add_n(5))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("15"),
        "add_n(5) should be 15: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_nested() {
    let code = r#"
let make_adder = fn(x) {
    return fn(y) { x + y }
}
let add5 = make_adder(5)
print(add5(10))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("15"),
        "add5(10) should be 15: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_with_filter() {
    let code = r#"
let nums = [1, -2, 3, -4, 5]
let positives = filter(nums, fn(x) { x > 0 })
print(positives)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("1"), "Should contain 1: stdout={}", stdout);
    assert!(stdout.contains("3"), "Should contain 3: stdout={}", stdout);
    assert!(stdout.contains("5"), "Should contain 5: stdout={}", stdout);
    assert!(
        !stdout.contains("-2"),
        "Should not contain -2: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_with_transform() {
    let code = r#"
let nums = [1, 2, 3]
let doubled = transform(nums, fn(x) { x * 2 })
print(doubled)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("2"), "Should contain 2: stdout={}", stdout);
    assert!(stdout.contains("4"), "Should contain 4: stdout={}", stdout);
    assert!(stdout.contains("6"), "Should contain 6: stdout={}", stdout);
}

#[test]
fn test_closure_immediate_invocation() {
    let code = r#"
let result = fn(x) { x + 1 }(5)
print(result)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("6"),
        "fn(x){{x+1}}(5) should be 6: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_as_callback() {
    let code = r#"
fn run_callback(cb) {
    cb()
}
run_callback(fn() { print("callback executed") })
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("callback executed"),
        "Should print callback message: stdout={}",
        stdout
    );
}

#[test]
fn test_closure_stored_then_used() {
    let code = r#"
let is_positive = fn(x) { x > 0 }
let data = [-3, -1, 0, 2, 4]
let result = filter(data, is_positive)
print(result)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("2"), "Should contain 2: stdout={}", stdout);
    assert!(stdout.contains("4"), "Should contain 4: stdout={}", stdout);
}

// ============================================================================
// get_index — Safe Array Index Access
// ============================================================================

#[test]
fn test_get_index_returns_option() {
    let code = r#"
import { get_index } from "std/collections"
let arr = [10, 20, 30]
print(get_index(arr, 0))
print(get_index(arr, 2))
print(get_index(arr, 5))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("Some(10)"),
        "Index 0 should be Some(10): stdout={}",
        stdout
    );
    assert!(
        stdout.contains("Some(30)"),
        "Index 2 should be Some(30): stdout={}",
        stdout
    );
    assert!(
        stdout.contains("None"),
        "Index 5 should be None: stdout={}",
        stdout
    );
}

#[test]
fn test_get_index_with_default() {
    let code = r#"
import { get_index } from "std/collections"
let arr = [10, 20, 30]
print(get_index(arr, 1, 0))
print(get_index(arr, 10, 0))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("20"),
        "Index 1 with default should be 20: stdout={}",
        stdout
    );
    assert!(
        stdout.contains("0"),
        "Index 10 with default should be 0: stdout={}",
        stdout
    );
}

#[test]
fn test_get_index_negative() {
    let code = r#"
import { get_index } from "std/collections"
let arr = [10, 20, 30]
print(get_index(arr, -1))
print(get_index(arr, -3))
print(get_index(arr, -5))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("Some(30)"),
        "-1 should be Some(30): stdout={}",
        stdout
    );
    assert!(
        stdout.contains("Some(10)"),
        "-3 should be Some(10): stdout={}",
        stdout
    );
    assert!(
        stdout.contains("None"),
        "-5 should be None: stdout={}",
        stdout
    );
}

#[test]
fn test_get_index_with_otherwise() {
    let code = r#"
import { get_index } from "std/collections"

fn extract(arr) {
    let a = get_index(arr, 0) otherwise return "empty"
    let b = get_index(arr, 1) otherwise return "only one: {a}"
    return "{a} and {b}"
}

print(extract([]))
print(extract(["hello"]))
print(extract(["hello", "world"]))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("empty"), "Empty array: stdout={}", stdout);
    assert!(
        stdout.contains("only one: hello"),
        "Single element: stdout={}",
        stdout
    );
    assert!(
        stdout.contains("hello and world"),
        "Two elements: stdout={}",
        stdout
    );
}

#[test]
fn test_get_index_empty_array() {
    let code = r#"
import { get_index } from "std/collections"
print(get_index([], 0))
print(get_index([], 0, "default"))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("None"),
        "Empty array index 0 should be None: stdout={}",
        stdout
    );
    assert!(
        stdout.contains("default"),
        "Empty array with default: stdout={}",
        stdout
    );
}

// ============================================================================
// If-Expressions (Phase 7.14)
// ============================================================================

#[test]
fn test_if_expr_true_branch() {
    let code = r#"
let x = if true { 1 } else { 2 }
print(x)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("1"),
        "True branch should return 1: stdout={}",
        stdout
    );
}

#[test]
fn test_if_expr_false_branch() {
    let code = r#"
let x = if false { 1 } else { 2 }
print(x)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("2"),
        "False branch should return 2: stdout={}",
        stdout
    );
}

#[test]
fn test_if_expr_with_comparison() {
    let code = r#"
let x = if 5 > 3 { "yes" } else { "no" }
print(x)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("yes"),
        "5 > 3 should select 'yes': stdout={}",
        stdout
    );
}

#[test]
fn test_if_expr_else_if_chain() {
    let code = r#"
let x = if 0 > 0 { "positive" } else if 0 == 0 { "zero" } else { "negative" }
print(x)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("zero"),
        "0 == 0 should select 'zero': stdout={}",
        stdout
    );
}

#[test]
fn test_if_expr_in_function_call() {
    let code = r#"
print(if true { "a" } else { "b" })
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(stdout.contains("a"), "Should print 'a': stdout={}", stdout);
}

#[test]
fn test_if_expr_nested() {
    let code = r#"
let x = if true { if false { 1 } else { 2 } } else { 3 }
print(x)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("2"),
        "Nested if should return 2: stdout={}",
        stdout
    );
}

#[test]
fn test_if_expr_with_variables() {
    let code = r#"
let n = 10
let x = if n > 5 { n * 2 } else { n }
print(x)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("20"),
        "10 > 5 so n*2 = 20: stdout={}",
        stdout
    );
}

#[test]
fn test_if_expr_string_result() {
    let code = r#"
let s = if len("hi") > 0 { "non-empty" } else { "empty" }
print(s)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Should succeed: stderr={}", stderr);
    assert!(
        stdout.contains("non-empty"),
        "len('hi') > 0 should select 'non-empty': stdout={}",
        stdout
    );
}

// ============================================================================
// None/Option Comparison Safety
// ============================================================================

#[test]
fn test_none_equality_with_none() {
    let code = r#"
print(None == None)
print(None != None)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "None == None should work: stderr={}", stderr);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true", "None == None should be true");
    assert_eq!(lines[1], "false", "None != None should be false");
}

#[test]
fn test_none_equality_cross_type() {
    let code = r#"
print(42 == None)
print(42 != None)
print("hello" == None)
print(true != None)
print(3.14 == None)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Cross-type None comparison should work: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "false", "42 == None should be false");
    assert_eq!(lines[1], "true", "42 != None should be true");
    assert_eq!(lines[2], "false", "\"hello\" == None should be false");
    assert_eq!(lines[3], "true", "true != None should be true");
    assert_eq!(lines[4], "false", "3.14 == None should be false");
}

#[test]
fn test_some_equality() {
    let code = r#"
print(Some(1) == Some(1))
print(Some(1) == Some(2))
print(Some(1) == None)
print(Some(1) != None)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Some equality should work: stderr={}", stderr);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true", "Some(1) == Some(1) should be true");
    assert_eq!(lines[1], "false", "Some(1) == Some(2) should be false");
    assert_eq!(lines[2], "false", "Some(1) == None should be false");
    assert_eq!(lines[3], "true", "Some(1) != None should be true");
}

#[test]
fn test_result_equality() {
    let code = r#"
print(Ok("x") == Ok("x"))
print(Ok("x") == Ok("y"))
print(Ok(1) == Err("fail"))
print(Err("a") == Err("a"))
print(Err("a") != Err("b"))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Result equality should work: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true", "Ok(\"x\") == Ok(\"x\") should be true");
    assert_eq!(lines[1], "false", "Ok(\"x\") == Ok(\"y\") should be false");
    assert_eq!(lines[2], "false", "Ok(1) == Err(\"fail\") should be false");
    assert_eq!(lines[3], "true", "Err(\"a\") == Err(\"a\") should be true");
    assert_eq!(lines[4], "true", "Err(\"a\") != Err(\"b\") should be true");
}

#[test]
fn test_none_comparison_in_loop() {
    // Simulates the snowgauge pattern: checking array values against None
    let code = r#"
let temps = [Some(32.5), None, Some(28.0), None]
let mut last_valid = None
for i in 0..len(temps) {
    if temps[i] != None {
        last_valid = temps[i]
    }
}
print(last_valid)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "None comparison in loop should work: stderr={}",
        stderr
    );
    assert!(
        stdout.contains("28"),
        "Last valid temp should be Some(28.0): stdout={}",
        stdout
    );
}

#[test]
fn test_null_coalesce_with_option() {
    let code = r#"
let a = Some(42) ?? 0
let b = None ?? 99
print(a)
print(b)
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Null coalesce should work: stderr={}", stderr);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "42", "Some(42) ?? 0 should be 42");
    assert_eq!(lines[1], "99", "None ?? 99 should be 99");
}

// ============================================================================
// JSON Option Serialization
// ============================================================================

#[test]
fn test_json_stringify_none() {
    let code = r#"
import { stringify } from "std/json"
print(stringify(None))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "stringify(None) should work: stderr={}",
        stderr
    );
    assert_eq!(stdout.trim(), "null", "None should serialize to null");
}

#[test]
fn test_json_stringify_some() {
    let code = r#"
import { stringify } from "std/json"
print(stringify(Some(42)))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "stringify(Some(42)) should work: stderr={}",
        stderr
    );
    assert_eq!(stdout.trim(), "42", "Some(42) should serialize to 42");
}

#[test]
fn test_json_stringify_option_array() {
    let code = r#"
import { stringify } from "std/json"
let arr = [Some(1), None, Some(3)]
print(stringify(arr))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "stringify option array should work: stderr={}",
        stderr
    );
    assert_eq!(
        stdout.trim(),
        "[1,null,3]",
        "Mixed Option array should serialize correctly"
    );
}

// ── Feature 7.16: None/null JSON Serialization ──

#[test]
fn test_parse_json_null_is_none() {
    let code = r#"
import { parse_json } from "std/json"
let result = parse_json("null")
match result {
    Ok(val) => {
        match val {
            None => print("GOT_NONE"),
            _ => print("NOT_NONE")
        }
    },
    Err(e) => print("ERROR: {e}")
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "parse_json null should work: stderr={}",
        stderr
    );
    assert_eq!(
        stdout.trim(),
        "GOT_NONE",
        "parse_json(\"null\") should return None"
    );
}

#[test]
fn test_parse_json_object_with_null() {
    let code = r#"
import { parse_json, stringify } from "std/json"
// Build a JSON string with null value via stringify, then re-parse
let json_str = stringify(map { "key": None, "val": 42 })
let result = parse_json(json_str)
match result {
    Ok(data) => {
        match data["key"] {
            None => print("KEY_IS_NONE"),
            _ => print("KEY_NOT_NONE")
        }
    },
    Err(e) => print("ERROR: {e}")
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "parse_json object with null should work: stderr={}",
        stderr
    );
    assert_eq!(
        stdout.trim(),
        "KEY_IS_NONE",
        "null value in JSON object should be None"
    );
}

#[test]
fn test_json_null_roundtrip() {
    let code = r#"
import { parse_json, stringify } from "std/json"
let original = map { "x": None, "y": 42 }
let json_str = stringify(original)
let parsed = parse_json(json_str)
match parsed {
    Ok(data) => {
        match data["x"] {
            None => print("ROUNDTRIP_OK"),
            _ => print("ROUNDTRIP_FAIL")
        }
    },
    Err(e) => print("ERROR: {e}")
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "JSON null roundtrip should work: stderr={}",
        stderr
    );
    assert_eq!(
        stdout.trim(),
        "ROUNDTRIP_OK",
        "None should round-trip through JSON"
    );
}

// ── Feature 7.9: Default Parameter Values ──

#[test]
fn test_default_param_basic() {
    let code = r#"
fn greet(name = "World") {
    return "Hello, {name}!"
}
print(greet())
print(greet("Alice"))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "default param should work: stderr={}", stderr);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "Hello, World!");
    assert_eq!(lines[1], "Hello, Alice!");
}

#[test]
fn test_default_param_multiple() {
    let code = r#"
fn paginate(items, page = 1, per_page = 25) {
    return "items={items} page={page} per_page={per_page}"
}
print(paginate("users"))
print(paginate("users", 2))
print(paginate("users", 3, 10))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "multiple default params should work: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "items=users page=1 per_page=25");
    assert_eq!(lines[1], "items=users page=2 per_page=25");
    assert_eq!(lines[2], "items=users page=3 per_page=10");
}

#[test]
fn test_default_param_prior_param_reference() {
    let code = r#"
fn foo(a = 1, b = a + 1) {
    return "{a},{b}"
}
print(foo())
print(foo(10))
print(foo(10, 20))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "default referencing prior param should work: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "1,2");
    assert_eq!(lines[1], "10,11");
    assert_eq!(lines[2], "10,20");
}

#[test]
fn test_default_param_ordering_error() {
    let code = r#"
fn bad(a = 1, b) {
    return a + b
}
"#;
    let (_stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "required param after default should fail");
    assert!(
        stderr.contains("cannot follow a parameter with a default value"),
        "Should report ordering error: stderr={}",
        stderr
    );
}

#[test]
fn test_default_param_too_many_args() {
    let code = r#"
fn greet(name = "World") {
    return "Hello, {name}!"
}
greet("a", "b")
"#;
    let (_stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "too many args should fail");
    assert!(
        stderr.contains("expected") && stderr.contains("got"),
        "Should report arity error: stderr={}",
        stderr
    );
}

#[test]
fn test_default_param_with_contract() {
    let code = r#"
fn divide(a, b = 1)
    requires b != 0
{
    return a / b
}
print(divide(10))
print(divide(10, 2))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "default param with contract should work: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "5");
}

#[test]
fn test_default_param_lambda() {
    let code = r#"
let f = fn(x = 5) { x * 2 }
print(f())
print(f(10))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "lambda default param should work: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "10");
    assert_eq!(lines[1], "20");
}

#[test]
fn test_default_param_with_type_annotation() {
    let code = r#"
fn add(a: Int, b: Int = 10) -> Int {
    return a + b
}
print(add(5))
print(add(5, 20))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "typed default param should work: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "15");
    assert_eq!(lines[1], "25");
}

// ===== Deep Mutation Tests =====

#[test]
fn test_deep_mutation_array_index() {
    let code = r#"
let mut arr = [1, 2, 3]
arr[0] = 10
print(arr[0])
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "10");
}

#[test]
fn test_deep_mutation_map_key() {
    let code = r#"
let mut m = map { "a": 1 }
m["a"] = 2
print(m["a"])
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "2");
}

#[test]
fn test_deep_mutation_nested_array_of_maps() {
    let code = r#"
let mut users = [map { "name": "Alice", "role": "user" }]
users[0]["role"] = "admin"
print(users[0]["role"])
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "admin");
}

#[test]
fn test_deep_mutation_nested_map_of_arrays() {
    let code = r#"
let mut data = map { "items": [1, 2, 3] }
data["items"][1] = 20
print(data["items"][1])
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "20");
}

#[test]
fn test_deep_mutation_triple_nesting() {
    let code = r#"
let mut deep = map { "a": map { "b": [10, 20] } }
deep["a"]["b"][0] = 99
print(deep["a"]["b"][0])
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "99");
}

#[test]
fn test_deep_mutation_immutable_fails() {
    let code = r#"
let users = [map { "name": "Alice" }]
users[0]["name"] = "Bob"
"#;
    let (_stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail: immutable variable");
}

#[test]
fn test_deep_mutation_out_of_bounds() {
    let code = r#"
let mut arr = [1, 2]
arr[5] = 10
"#;
    let (_stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_ne!(exit_code, 0, "Should fail: index out of bounds");
}

#[test]
fn test_deep_mutation_new_map_key() {
    let code = r#"
let mut m = map { "a": 1 }
m["b"] = 2
print(m["b"])
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "2");
}

#[test]
fn test_crypto_base64_roundtrip() {
    let code = r#"
import { base64_encode, base64_decode } from "std/crypto"

let encoded = base64_encode("Hello NTNT!")
print(encoded)
let decoded = base64_decode(encoded)
match decoded {
    Ok(val) => print(val)
    Err(e) => print("ERROR: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "base64 roundtrip failed: stderr={}", stderr);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "SGVsbG8gTlROVCE=");
    assert_eq!(lines[1], "Hello NTNT!");
}

#[test]
fn test_crypto_base64url_roundtrip() {
    let code = r#"
import { base64url_encode, base64url_decode } from "std/crypto"

let encoded = base64url_encode("Hello NTNT!")
print(encoded)
let decoded = base64url_decode(encoded)
match decoded {
    Ok(val) => print(val)
    Err(e) => print("ERROR: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "base64url roundtrip failed: stderr={}",
        stderr
    );
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "SGVsbG8gTlROVCE");
    assert_eq!(lines[1], "Hello NTNT!");
}

#[test]
fn test_crypto_aes_roundtrip() {
    let code = r#"
import { aes_generate_key, aes_encrypt, aes_decrypt } from "std/crypto"

let key = aes_generate_key()
let encrypted = aes_encrypt("secret message", key)
match encrypted {
    Ok(ct) => {
        let decrypted = aes_decrypt(ct, key)
        match decrypted {
            Ok(pt) => print(pt)
            Err(e) => print("DECRYPT ERROR: " + e)
        }
    }
    Err(e) => print("ENCRYPT ERROR: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "aes roundtrip failed: stderr={}", stderr);
    assert_eq!(stdout.trim(), "secret message");
}

#[test]
fn test_crypto_argon2_hash_verify() {
    let code = r#"
import { argon2_hash, argon2_verify } from "std/crypto"

let hash = argon2_hash("mypassword")
print(argon2_verify("mypassword", hash))
print(argon2_verify("wrongpassword", hash))
"#;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "argon2 failed: stderr={}", stderr);
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
}

#[test]
fn test_collections_has_value_strings() {
    let code = r#"
import { has_value } from "std/collections"
let colors = ["red", "green", "blue"]
print(has_value(colors, "green"))
print(has_value(colors, "purple"))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "contains() should work with strings");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
}

#[test]
fn test_collections_has_value_ints() {
    let code = r#"
import { has_value } from "std/collections"
let nums = [1, 2, 3]
print(has_value(nums, 2))
print(has_value(nums, 99))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "contains() should work with ints");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
}

#[test]
fn test_collections_has_value_empty_array() {
    let code = r#"
import { has_value } from "std/collections"
print(has_value([], "anything"))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "contains() should work with empty array");
    assert_eq!(stdout.trim(), "false");
}

#[test]
fn test_collections_has_value_nested() {
    let code = r#"
import { has_value } from "std/collections"
let nested = [[1, 2], [3, 4]]
print(has_value(nested, [1, 2]))
print(has_value(nested, [5, 6]))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "contains() should work with nested arrays");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true");
    assert_eq!(lines[1], "false");
}

#[test]
fn test_collections_has_value_bools() {
    let code = r#"
import { has_value } from "std/collections"
let flags = [true, false]
print(has_value(flags, true))
print(has_value(flags, false))
"#;
    let (stdout, _, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "contains() should work with bools");
    let lines: Vec<&str> = stdout.trim().lines().collect();
    assert_eq!(lines[0], "true", "true is in [true, false]");
    assert_eq!(lines[1], "true", "false is in [true, false]");
}

// =============================================================================
// Raw string smart delimiter tests (fixes #1.3, #1.4)
// =============================================================================

#[test]
fn test_raw_string_with_href_hash() {
    // Issue 1.4: r#"..."# should handle href="#" without premature termination
    let code = r##"
let html = r#"<a href="#">Link</a>"#
print(html)
"##;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "r#\"...\"# with href=\"#\" should parse. stderr: {}",
        stderr
    );
    assert_eq!(stdout.trim(), r##"<a href="#">Link</a>"##);
}

#[test]
fn test_raw_string_with_hex_colors() {
    // Issue 1.4: r#"..."# should handle CSS hex colors
    let code = r##"
let css = r#"<div style="color: #fff; background: #333">"#
print(css)
"##;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "r#\"...\"# with hex colors should parse. stderr: {}",
        stderr
    );
    assert!(stdout.contains("#fff"));
    assert!(stdout.contains("#333"));
}

#[test]
fn test_raw_string_with_svg_paths() {
    // Issue 1.3: SVG paths with decimals in raw strings
    let code = r##"
let svg = r#"<path d="M10.5 20.3 L30.7 40.1"/>"#
print(svg)
"##;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "SVG paths in raw strings should work. stderr: {}",
        stderr
    );
    assert!(stdout.contains("M10.5 20.3"));
}

#[test]
fn test_raw_string_multi_hash_still_works() {
    // r##"..."## should continue to work
    let code = r###"
let s = r##"contains "# and more"##
print(s)
"###;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "r##\"...\"## should work. stderr: {}", stderr);
    assert!(stdout.contains(r##"contains "# and more"##));
}

#[test]
fn test_raw_string_multiple_hash_sequences() {
    // Multiple "# sequences in one r#"..."# string
    let code = r##"
let html = r#"<a href="#">one</a> <a href="#">two</a>"#
print(html)
"##;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Multiple href=\"#\" in one raw string. stderr: {}",
        stderr
    );
    assert!(stdout.contains(r##"<a href="#">one</a>"##));
    assert!(stdout.contains(r##"<a href="#">two</a>"##));
}

#[test]
fn test_raw_string_normal_close_still_works() {
    // Regular r#"..."# without problematic content
    let code = r##"
let s = r#"just a normal string"#
print(s)
"##;
    let (stdout, stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(
        exit_code, 0,
        "Normal raw string should still close properly. stderr: {}",
        stderr
    );
    assert_eq!(stdout.trim(), "just a normal string");
}
