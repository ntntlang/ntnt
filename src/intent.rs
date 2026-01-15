//! Intent-Driven Development (IDD) Module
//!
//! Parses `.intent` files and executes tests against NTNT programs.
//!
//! Intent files are YAML-based specifications that define both human-readable
//! requirements and machine-executable tests.
//!
//! # Example intent file:
//! ```yaml
//! # snowgauge.intent
//! Feature: Site Selection
//!   id: feature.site_selection
//!   description: "Users can select from available monitoring sites"
//!   test:
//!     - request: GET /
//!       assert:
//!         - status: 200
//!         - body contains "Bear Lake"
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use colored::*;

use crate::interpreter::Interpreter;
use crate::lexer::Lexer;
use crate::parser::Parser as IntentParser;
use crate::error::IntentError;

/// A single assertion within a test
#[derive(Debug, Clone)]
pub enum Assertion {
    /// Check HTTP status code: `status: 200`
    Status(u16),
    /// Check body contains text: `body contains "text"`
    BodyContains(String),
    /// Check body matches regex: `body matches r"pattern"`
    BodyMatches(String),
    /// Check body does not contain: `body not contains "error"`
    BodyNotContains(String),
    /// Check header value: `header "Content-Type" contains "text/html"`
    HeaderContains(String, String),
}

/// A single test case (HTTP request + assertions)
#[derive(Debug, Clone)]
pub struct TestCase {
    pub method: String,
    pub path: String,
    pub body: Option<String>,
    pub assertions: Vec<Assertion>,
}

/// A feature/requirement with tests
#[derive(Debug, Clone)]
pub struct Feature {
    pub id: Option<String>,
    pub name: String,
    pub description: Option<String>,
    pub tests: Vec<TestCase>,
}

/// Parsed intent file
#[derive(Debug)]
pub struct IntentFile {
    pub features: Vec<Feature>,
    pub source_path: String,
}

/// Result of running a single assertion
#[derive(Debug, Clone)]
pub struct AssertionResult {
    pub assertion: Assertion,
    pub passed: bool,
    pub actual: Option<String>,
    pub message: Option<String>,
}

/// Result of running a test case
#[derive(Debug, Clone)]
pub struct TestResult {
    pub test: TestCase,
    pub passed: bool,
    pub assertion_results: Vec<AssertionResult>,
    pub response_status: u16,
    pub response_body: String,
    pub response_headers: HashMap<String, String>,
}

/// Result of running a feature
#[derive(Debug)]
pub struct FeatureResult {
    pub feature: Feature,
    pub passed: bool,
    pub test_results: Vec<TestResult>,
}

/// Result of running all intent checks
#[derive(Debug)]
pub struct IntentCheckResult {
    pub intent_file: String,
    pub features_passed: usize,
    pub features_failed: usize,
    pub assertions_passed: usize,
    pub assertions_failed: usize,
    pub feature_results: Vec<FeatureResult>,
}

impl IntentFile {
    /// Parse an intent file from a path
    pub fn parse(path: &Path) -> Result<Self, IntentError> {
        let content = fs::read_to_string(path)
            .map_err(|e| IntentError::RuntimeError(format!("Failed to read intent file: {}", e)))?;
        
        Self::parse_content(&content, path.to_string_lossy().to_string())
    }
    
    /// Parse intent file content
    pub fn parse_content(content: &str, source_path: String) -> Result<Self, IntentError> {
        let mut features = Vec::new();
        let mut current_feature: Option<Feature> = None;
        let mut current_test: Option<TestCase> = None;
        let mut in_assertions = false;
        
        for line in content.lines() {
            let trimmed = line.trim();
            
            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            
            // Feature declaration
            if trimmed.starts_with("Feature:") {
                // Save previous feature
                if let Some(mut feat) = current_feature.take() {
                    if let Some(test) = current_test.take() {
                        feat.tests.push(test);
                    }
                    features.push(feat);
                }
                
                let name = trimmed.trim_start_matches("Feature:").trim().to_string();
                current_feature = Some(Feature {
                    id: None,
                    name,
                    description: None,
                    tests: Vec::new(),
                });
                current_test = None;
                in_assertions = false;
                continue;
            }
            
            // Inside a feature
            if let Some(ref mut feature) = current_feature {
                // Feature ID
                if trimmed.starts_with("id:") {
                    let id = trimmed.trim_start_matches("id:").trim();
                    feature.id = Some(id.to_string());
                    continue;
                }
                
                // Description
                if trimmed.starts_with("description:") {
                    let desc = trimmed.trim_start_matches("description:").trim();
                    // Remove surrounding quotes if present
                    let desc = desc.trim_matches('"').to_string();
                    feature.description = Some(desc);
                    continue;
                }
                
                // Test section start
                if trimmed == "test:" {
                    in_assertions = false;
                    continue;
                }
                
                // Request line (starts a new test case)
                if trimmed.starts_with("- request:") || trimmed.starts_with("request:") {
                    // Save previous test
                    if let Some(test) = current_test.take() {
                        feature.tests.push(test);
                    }
                    
                    let request_str = if trimmed.starts_with("- request:") {
                        trimmed.trim_start_matches("- request:").trim()
                    } else {
                        trimmed.trim_start_matches("request:").trim()
                    };
                    
                    // Parse "METHOD /path"
                    let parts: Vec<&str> = request_str.splitn(2, ' ').collect();
                    let method = parts.get(0).unwrap_or(&"GET").to_string();
                    let path = parts.get(1).unwrap_or(&"/").to_string();
                    
                    current_test = Some(TestCase {
                        method,
                        path,
                        body: None,
                        assertions: Vec::new(),
                    });
                    in_assertions = false;
                    continue;
                }
                
                // Assert section
                if trimmed == "assert:" {
                    in_assertions = true;
                    continue;
                }
                
                // Assertion lines
                if in_assertions {
                    if let Some(ref mut test) = current_test {
                        if let Some(assertion) = Self::parse_assertion(trimmed) {
                            test.assertions.push(assertion);
                        }
                    }
                    continue;
                }
                
                // Body for POST requests
                if trimmed.starts_with("body:") {
                    if let Some(ref mut test) = current_test {
                        let body = trimmed.trim_start_matches("body:").trim();
                        let body = body.trim_matches('"').to_string();
                        test.body = Some(body);
                    }
                    continue;
                }
            }
        }
        
        // Save final feature and test
        if let Some(mut feat) = current_feature {
            if let Some(test) = current_test {
                feat.tests.push(test);
            }
            features.push(feat);
        }
        
        Ok(IntentFile { features, source_path })
    }
    
    /// Parse a single assertion line
    fn parse_assertion(line: &str) -> Option<Assertion> {
        let line = line.trim().trim_start_matches('-').trim();
        
        // status: 200
        if line.starts_with("status:") {
            let code_str = line.trim_start_matches("status:").trim();
            if let Ok(code) = code_str.parse::<u16>() {
                return Some(Assertion::Status(code));
            }
        }
        
        // body contains "text"
        if line.starts_with("body contains") {
            let text = line.trim_start_matches("body contains").trim();
            let text = text.trim_matches('"').to_string();
            return Some(Assertion::BodyContains(text));
        }
        
        // body not contains "text"
        if line.starts_with("body not contains") {
            let text = line.trim_start_matches("body not contains").trim();
            let text = text.trim_matches('"').to_string();
            return Some(Assertion::BodyNotContains(text));
        }
        
        // body matches r"pattern" or body matches "pattern"
        if line.starts_with("body matches") {
            let pattern = line.trim_start_matches("body matches").trim();
            // Handle raw string r"..." or regular "..."
            let pattern = if pattern.starts_with("r\"") {
                pattern.trim_start_matches("r\"").trim_end_matches('"')
            } else {
                pattern.trim_matches('"')
            };
            return Some(Assertion::BodyMatches(pattern.to_string()));
        }
        
        // header "Name" contains "value"
        if line.starts_with("header") {
            // header "Content-Type" contains "text/html"
            let rest = line.trim_start_matches("header").trim();
            if let Some(idx) = rest.find("contains") {
                let header_name = rest[..idx].trim().trim_matches('"').to_string();
                let value = rest[idx..].trim_start_matches("contains").trim().trim_matches('"').to_string();
                return Some(Assertion::HeaderContains(header_name, value));
            }
        }
        
        None
    }
}

/// Run intent checks against an NTNT file
pub fn run_intent_check(
    ntnt_path: &Path,
    intent_path: &Path,
    port: u16,
    _verbose: bool,
) -> Result<IntentCheckResult, IntentError> {
    // Parse intent file
    let intent = IntentFile::parse(intent_path)?;
    
    // Read NTNT source
    let source = fs::read_to_string(ntnt_path)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to read NTNT file: {}", e)))?;
    
    // Count total tests
    let total_tests: usize = intent.features.iter().map(|f| f.tests.len()).sum();
    
    if total_tests == 0 {
        return Err(IntentError::RuntimeError("No tests found in intent file".to_string()));
    }
    
    // Setup for server
    let shutdown_flag = Arc::new(AtomicBool::new(false));
    let shutdown_flag_clone = shutdown_flag.clone();
    
    // Collect all tests to run
    let mut all_tests: Vec<(usize, usize, TestCase)> = Vec::new();
    for (fi, feature) in intent.features.iter().enumerate() {
        for (ti, test) in feature.tests.iter().enumerate() {
            all_tests.push((fi, ti, test.clone()));
        }
    }
    
    let all_tests_clone = all_tests.clone();
    let results: Arc<std::sync::Mutex<Vec<(usize, usize, TestResult)>>> = 
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let results_clone = results.clone();
    
    // Spawn thread to run tests
    let test_handle = thread::spawn(move || {
        // Wait for server to start
        thread::sleep(Duration::from_millis(300));
        
        for (fi, ti, test) in all_tests_clone {
            let result = run_single_test(&test, port);
            results_clone.lock().unwrap().push((fi, ti, result));
        }
        
        // Signal shutdown
        shutdown_flag_clone.store(true, Ordering::SeqCst);
    });
    
    // Start the server
    let mut interpreter = Interpreter::new();
    interpreter.set_test_mode(port, total_tests, shutdown_flag.clone());
    
    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();
    let mut parser = IntentParser::new(tokens);
    let ast = parser.parse()?;
    
    // Run (will exit when shutdown_flag is set)
    let _ = interpreter.eval(&ast);
    
    // Wait for test thread
    test_handle.join().ok();
    
    // Collect results
    let test_results = results.lock().unwrap();
    
    // Build feature results
    let mut feature_results: Vec<FeatureResult> = intent.features.iter().map(|f| {
        FeatureResult {
            feature: f.clone(),
            passed: true,
            test_results: Vec::new(),
        }
    }).collect();
    
    for (fi, _ti, result) in test_results.iter() {
        if !result.passed {
            feature_results[*fi].passed = false;
        }
        feature_results[*fi].test_results.push(result.clone());
    }
    
    // Calculate totals
    let mut features_passed = 0;
    let mut features_failed = 0;
    let mut assertions_passed = 0;
    let mut assertions_failed = 0;
    
    for fr in &feature_results {
        if fr.passed {
            features_passed += 1;
        } else {
            features_failed += 1;
        }
        for tr in &fr.test_results {
            for ar in &tr.assertion_results {
                if ar.passed {
                    assertions_passed += 1;
                } else {
                    assertions_failed += 1;
                }
            }
        }
    }
    
    Ok(IntentCheckResult {
        intent_file: intent.source_path,
        features_passed,
        features_failed,
        assertions_passed,
        assertions_failed,
        feature_results,
    })
}

/// Run a single test case against the server
fn run_single_test(test: &TestCase, port: u16) -> TestResult {
    let path = if test.path.starts_with('/') {
        test.path.clone()
    } else {
        format!("/{}", test.path)
    };
    
    let body_content = test.body.clone().unwrap_or_default();
    let request = if body_content.is_empty() {
        format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
            test.method, path, port
        )
    } else {
        format!(
            "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            test.method, path, port, body_content.len(), body_content
        )
    };
    
    // Try to connect
    let mut attempts = 0;
    let max_attempts = 20;
    
    while attempts < max_attempts {
        match TcpStream::connect(format!("127.0.0.1:{}", port)) {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
                stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
                
                if stream.write_all(request.as_bytes()).is_ok() {
                    let mut response = Vec::new();
                    let _ = stream.read_to_end(&mut response);
                    
                    if !response.is_empty() {
                        let response_str = String::from_utf8_lossy(&response).to_string();
                        let parts: Vec<&str> = response_str.splitn(2, "\r\n\r\n").collect();
                        let headers_str = parts.get(0).unwrap_or(&"");
                        let body = parts.get(1).unwrap_or(&"").to_string();
                        
                        // Parse status code
                        let status_code = headers_str
                            .lines()
                            .next()
                            .unwrap_or("")
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or("0")
                            .parse::<u16>()
                            .unwrap_or(0);
                        
                        // Parse headers
                        let mut headers = HashMap::new();
                        for line in headers_str.lines().skip(1) {
                            if let Some(idx) = line.find(':') {
                                let key = line[..idx].trim().to_lowercase();
                                let value = line[idx+1..].trim().to_string();
                                headers.insert(key, value);
                            }
                        }
                        
                        // Run assertions
                        let assertion_results = run_assertions(&test.assertions, status_code, &body, &headers);
                        let all_passed = assertion_results.iter().all(|r| r.passed);
                        
                        return TestResult {
                            test: test.clone(),
                            passed: all_passed,
                            assertion_results,
                            response_status: status_code,
                            response_body: body,
                            response_headers: headers,
                        };
                    }
                }
            }
            Err(_) => {}
        }
        attempts += 1;
        thread::sleep(Duration::from_millis(100));
    }
    
    // Connection failed
    TestResult {
        test: test.clone(),
        passed: false,
        assertion_results: vec![AssertionResult {
            assertion: Assertion::Status(0),
            passed: false,
            actual: None,
            message: Some("Connection failed".to_string()),
        }],
        response_status: 0,
        response_body: String::new(),
        response_headers: HashMap::new(),
    }
}

/// Run assertions against a response
fn run_assertions(
    assertions: &[Assertion],
    status: u16,
    body: &str,
    headers: &HashMap<String, String>,
) -> Vec<AssertionResult> {
    assertions.iter().map(|assertion| {
        match assertion {
            Assertion::Status(expected) => {
                let passed = status == *expected;
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(status.to_string()),
                    message: if passed { None } else { Some(format!("Expected status {}, got {}", expected, status)) },
                }
            }
            Assertion::BodyContains(text) => {
                let passed = body.contains(text);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(if body.len() > 100 { format!("{}...", &body[..100]) } else { body.to_string() }),
                    message: if passed { None } else { Some(format!("Body does not contain \"{}\"", text)) },
                }
            }
            Assertion::BodyNotContains(text) => {
                let passed = !body.contains(text);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(if body.len() > 100 { format!("{}...", &body[..100]) } else { body.to_string() }),
                    message: if passed { None } else { Some(format!("Body contains \"{}\" (should not)", text)) },
                }
            }
            Assertion::BodyMatches(pattern) => {
                let passed = match regex::Regex::new(pattern) {
                    Ok(re) => re.is_match(body),
                    Err(_) => false,
                };
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: Some(if body.len() > 100 { format!("{}...", &body[..100]) } else { body.to_string() }),
                    message: if passed { None } else { Some(format!("Body does not match pattern \"{}\"", pattern)) },
                }
            }
            Assertion::HeaderContains(header_name, expected_value) => {
                let actual = headers.get(&header_name.to_lowercase());
                let passed = actual.map(|v| v.contains(expected_value)).unwrap_or(false);
                AssertionResult {
                    assertion: assertion.clone(),
                    passed,
                    actual: actual.cloned(),
                    message: if passed { None } else { 
                        Some(format!("Header \"{}\" does not contain \"{}\"", header_name, expected_value)) 
                    },
                }
            }
        }
    }).collect()
}

/// Print intent check results
pub fn print_intent_results(result: &IntentCheckResult) {
    println!();
    
    for fr in &result.feature_results {
        // Feature header
        let status_icon = if fr.passed { "✓".green() } else { "✗".red() };
        println!("{} Feature: {}", status_icon, fr.feature.name.bold());
        
        if let Some(ref desc) = fr.feature.description {
            println!("  {}", desc.dimmed());
        }
        
        // Test results
        for tr in &fr.test_results {
            println!();
            let test_icon = if tr.passed { "✓".green() } else { "✗".red() };
            println!("  {} {} {}", test_icon, tr.test.method.cyan(), tr.test.path);
            
            // Assertion results
            for ar in &tr.assertion_results {
                let assertion_icon = if ar.passed { "✓".green() } else { "✗".red() };
                let assertion_desc = format_assertion(&ar.assertion);
                
                if ar.passed {
                    println!("    {} {}", assertion_icon, assertion_desc);
                } else {
                    println!("    {} {}", assertion_icon, assertion_desc.red());
                    if let Some(ref msg) = ar.message {
                        println!("      {}", msg.yellow());
                    }
                }
            }
        }
        println!();
    }
    
    // Summary
    let total_features = result.features_passed + result.features_failed;
    let total_assertions = result.assertions_passed + result.assertions_failed;
    
    let summary = format!(
        "{}/{} features passing ({}/{} assertions)",
        result.features_passed, total_features,
        result.assertions_passed, total_assertions
    );
    
    println!();
    if result.features_failed == 0 {
        println!("{}", summary.green().bold());
    } else {
        println!("{}", summary.red().bold());
    }
}

/// Format an assertion for display
fn format_assertion(assertion: &Assertion) -> String {
    match assertion {
        Assertion::Status(code) => format!("status: {}", code),
        Assertion::BodyContains(text) => format!("body contains \"{}\"", text),
        Assertion::BodyNotContains(text) => format!("body not contains \"{}\"", text),
        Assertion::BodyMatches(pattern) => format!("body matches \"{}\"", pattern),
        Assertion::HeaderContains(name, value) => format!("header \"{}\" contains \"{}\"", name, value),
    }
}

/// Find the intent file for a given NTNT file
/// Looks for: <name>.intent, <name>.tnt.intent, or intent.yaml in same directory
pub fn find_intent_file(ntnt_path: &Path) -> Option<std::path::PathBuf> {
    let parent = ntnt_path.parent()?;
    let stem = ntnt_path.file_stem()?.to_string_lossy();
    
    // Try <name>.intent
    let intent_path = parent.join(format!("{}.intent", stem));
    if intent_path.exists() {
        return Some(intent_path);
    }
    
    // Try <name>.tnt.intent
    let intent_path = parent.join(format!("{}.tnt.intent", stem));
    if intent_path.exists() {
        return Some(intent_path);
    }
    
    // Try intent.yaml in same directory
    let intent_path = parent.join("intent.yaml");
    if intent_path.exists() {
        return Some(intent_path);
    }
    
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_simple_intent() {
        let content = r#"
Feature: Site Selection
  id: feature.site_selection
  description: "Users can select sites"
  test:
    - request: GET /
      assert:
        - status: 200
        - body contains "Bear Lake"
"#;
        
        let intent = IntentFile::parse_content(content, "test.intent".to_string()).unwrap();
        assert_eq!(intent.features.len(), 1);
        
        let feature = &intent.features[0];
        assert_eq!(feature.name, "Site Selection");
        assert_eq!(feature.id, Some("feature.site_selection".to_string()));
        assert_eq!(feature.tests.len(), 1);
        
        let test = &feature.tests[0];
        assert_eq!(test.method, "GET");
        assert_eq!(test.path, "/");
        assert_eq!(test.assertions.len(), 2);
    }
    
    #[test]
    fn test_parse_assertions() {
        assert!(matches!(
            IntentFile::parse_assertion("status: 200"),
            Some(Assertion::Status(200))
        ));
        
        assert!(matches!(
            IntentFile::parse_assertion("body contains \"test\""),
            Some(Assertion::BodyContains(s)) if s == "test"
        ));
        
        assert!(matches!(
            IntentFile::parse_assertion("body not contains \"error\""),
            Some(Assertion::BodyNotContains(s)) if s == "error"
        ));
        
        assert!(matches!(
            IntentFile::parse_assertion("body matches r\"\\d+\""),
            Some(Assertion::BodyMatches(s)) if s == "\\d+"
        ));
    }
    
    #[test]
    fn test_multiple_features() {
        let content = r#"
Feature: Home Page
  test:
    - request: GET /
      assert:
        - status: 200

Feature: API
  test:
    - request: GET /api/status
      assert:
        - status: 200
"#;
        
        let intent = IntentFile::parse_content(content, "test.intent".to_string()).unwrap();
        assert_eq!(intent.features.len(), 2);
        assert_eq!(intent.features[0].name, "Home Page");
        assert_eq!(intent.features[1].name, "API");
    }
}
