//! Integration tests for NTNT language features
//!
//! Tests new language features including:
//! - Map iteration functions (keys, values, entries, has_key)
//! - Nested map inference
//! - CSV parsing

use std::process::Command;
use std::fs;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique test file path
fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = format!("{:?}", std::thread::current().id());
    format!("/tmp/ntnt_{}_{}_{}_{}.tnt", prefix, std::process::id(), thread_id.replace(|c: char| !c.is_alphanumeric(), "_"), counter)
}

/// Helper to run ntnt with a code string
fn run_ntnt_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("feature_test");
    
    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");
    drop(file);
    
    let output = Command::new("./target/release/ntnt")
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
    
    let output = Command::new("./target/release/ntnt")
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
    assert!(stdout.contains("name: Alice") || stdout.contains("age: 30"), "Should contain entry data");
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
    assert!(stdout.contains("false"), "Empty map should not have any key");
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
    assert!(stdout.contains("false"), "has_key should not find 'missing'");
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
    assert!(exit_code != 0 || stderr.contains("error") || stderr.contains("Error"), 
        "Top-level {{}} without map keyword should not work as a map");
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
    assert!(stdout.contains("25"), "Should access Bob's age (row 2, col 1)");
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
    assert!(stdout.contains("empty-falsy"), "Empty string should be falsy");
    assert!(stdout.contains("full-truthy"), "Non-empty string should be truthy");
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
    assert!(stdout.contains("empty-falsy"), "Empty array should be falsy");
    assert!(stdout.contains("full-truthy"), "Non-empty array should be truthy");
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
    assert!(stdout.contains("full-truthy"), "Non-empty map should be truthy");
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
    assert!(stdout.contains("query-present"), "Non-empty string in && should work");
    assert!(stdout.contains("empty-absent"), "!empty_string should be true");
}