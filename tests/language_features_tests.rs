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

    // Try release binary first, fall back to debug
    let binary = if std::path::Path::new("./target/release/ntnt").exists() {
        "./target/release/ntnt"
    } else {
        "./target/debug/ntnt"
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

    // Try release binary first, fall back to debug
    let binary = if std::path::Path::new("./target/release/ntnt").exists() {
        "./target/release/ntnt"
    } else {
        "./target/debug/ntnt"
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
import { parse } from "std/csv"
let csv_data = "name,age,city
Alice,30,NYC
Bob,25,LA"
let rows = parse(csv_data)
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
