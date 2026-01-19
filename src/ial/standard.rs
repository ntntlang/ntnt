//! Standard vocabulary definitions for IAL.
//!
//! This module provides the built-in vocabulary that ships with IAL:
//! - HTTP assertion terms (status, body contains, etc.)
//! - Response time assertions
//! - CLI assertions
//!
//! Users can extend this with glossary terms from their .intent files.

use super::primitives::{CheckOp, Primitive, Value};
use super::vocabulary::{Term, Vocabulary};

/// Build the standard vocabulary with all built-in assertion patterns.
pub fn standard_vocabulary() -> Vocabulary {
    let mut vocab = Vocabulary::new();

    // ========================================
    // HTTP Status Assertions
    // ========================================

    // "status: 200" → Check response.status equals 200
    // Uses string placeholder that will be substituted and parsed as number
    vocab.add_primitive(
        "status: {code}",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "response.status".to_string(),
            expected: Value::String("{code}".to_string()), // Placeholder - resolved from {code}
        },
    );

    // "status 2xx" → Check response.status in range 200-299
    vocab.add_primitive(
        "status 2xx",
        Primitive::Check {
            op: CheckOp::InRange,
            path: "response.status".to_string(),
            expected: Value::Range(200.0, 299.0),
        },
    );

    // "status 4xx" → Check response.status in range 400-499
    vocab.add_primitive(
        "status 4xx",
        Primitive::Check {
            op: CheckOp::InRange,
            path: "response.status".to_string(),
            expected: Value::Range(400.0, 499.0),
        },
    );

    // "status 5xx" → Check response.status in range 500-599
    vocab.add_primitive(
        "status 5xx",
        Primitive::Check {
            op: CheckOp::InRange,
            path: "response.status".to_string(),
            expected: Value::Range(500.0, 599.0),
        },
    );

    // ========================================
    // HTTP Body Assertions
    // ========================================

    // "body contains {text}" → Check response.body contains text
    vocab.add_primitive(
        "body contains {text}",
        Primitive::Check {
            op: CheckOp::Contains,
            path: "response.body".to_string(),
            expected: Value::String("{text}".to_string()), // Placeholder - resolved from {text}
        },
    );

    // "body not contains {text}" → Check response.body not contains text
    vocab.add_primitive(
        "body not contains {text}",
        Primitive::Check {
            op: CheckOp::NotContains,
            path: "response.body".to_string(),
            expected: Value::String("{text}".to_string()),
        },
    );

    // "body matches {pattern}" → Check response.body matches regex
    vocab.add_primitive(
        "body matches {pattern}",
        Primitive::Check {
            op: CheckOp::Matches,
            path: "response.body".to_string(),
            expected: Value::Regex("{pattern}".to_string()),
        },
    );

    // "body is empty" → Check response.body equals ""
    vocab.add_primitive(
        "body is empty",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "response.body".to_string(),
            expected: Value::String(String::new()),
        },
    );

    // "body is not empty" → Check response.body not equals ""
    vocab.add_primitive(
        "body is not empty",
        Primitive::Check {
            op: CheckOp::NotEquals,
            path: "response.body".to_string(),
            expected: Value::String(String::new()),
        },
    );

    // ========================================
    // JSON Body Assertions (semantic aliases)
    // ========================================

    // "body has field {field}" → body contains the field name
    vocab.add_terms(
        "body has field {field}",
        vec![Term::new("body contains \"{field}\"")],
    );

    // "response is valid JSON" → body matches JSON pattern
    vocab.add_primitive(
        "response is valid JSON",
        Primitive::Check {
            op: CheckOp::Matches,
            path: "response.body".to_string(),
            expected: Value::Regex(r"^\s*[\{\[].*[\}\]]\s*$".to_string()),
        },
    );

    // ========================================
    // Response Time Assertions
    // ========================================

    // "response time < {ms}ms" → Check response time
    vocab.add_primitive(
        "response time < {ms}ms",
        Primitive::Check {
            op: CheckOp::LessThan,
            path: "response.time_ms".to_string(),
            expected: Value::Number(0.0), // Placeholder
        },
    );

    // "response time < {seconds}s" → Check response time (seconds)
    vocab.add_primitive(
        "response time < {seconds}s",
        Primitive::Check {
            op: CheckOp::LessThan,
            path: "response.time_ms".to_string(),
            expected: Value::Number(0.0), // Placeholder, will be multiplied by 1000
        },
    );

    // ========================================
    // Header Assertions
    // ========================================

    // "header {name} exists" → Check header exists
    vocab.add_primitive(
        "header {name} exists",
        Primitive::Check {
            op: CheckOp::Exists,
            path: "response.headers".to_string(), // Will be suffixed with .{name}
            expected: Value::Null,
        },
    );

    // "header {name} equals {value}" → Check header value
    vocab.add_primitive(
        "header {name} equals {value}",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "response.headers.{name}".to_string(),
            expected: Value::String("{value}".to_string()),
        },
    );

    // "header {name} contains {value}" → Check header contains value
    vocab.add_primitive(
        "header {name} contains {value}",
        Primitive::Check {
            op: CheckOp::Contains,
            path: "response.headers.{name}".to_string(),
            expected: Value::String("{value}".to_string()),
        },
    );

    // ========================================
    // Content-Type shortcuts
    // ========================================

    // "content-type is json" → header content-type contains application/json
    vocab.add_terms(
        "content-type is json",
        vec![Term::new(
            "header content-type contains \"application/json\"",
        )],
    );

    // "content-type is html" → header content-type contains text/html
    vocab.add_terms(
        "content-type is html",
        vec![Term::new("header content-type contains \"text/html\"")],
    );

    // "content-type is text" → header content-type contains text/plain
    vocab.add_terms(
        "content-type is text",
        vec![Term::new("header content-type contains \"text/plain\"")],
    );

    // ========================================
    // CLI Assertions
    // ========================================

    // "exit code is {code}" → Check CLI exit code
    vocab.add_primitive(
        "exit code is {code}",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "cli.exit_code".to_string(),
            expected: Value::Number(0.0),
        },
    );

    // "stdout contains {text}" → Check CLI stdout
    vocab.add_primitive(
        "stdout contains {text}",
        Primitive::Check {
            op: CheckOp::Contains,
            path: "cli.stdout".to_string(),
            expected: Value::String("{text}".to_string()),
        },
    );

    // "stderr contains {text}" → Check CLI stderr
    vocab.add_primitive(
        "stderr contains {text}",
        Primitive::Check {
            op: CheckOp::Contains,
            path: "cli.stderr".to_string(),
            expected: Value::String("{text}".to_string()),
        },
    );

    // "stderr is empty" → Check CLI stderr is empty
    vocab.add_primitive(
        "stderr is empty",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "cli.stderr".to_string(),
            expected: Value::String(String::new()),
        },
    );

    // ========================================
    // Code Quality Assertions
    // ========================================

    // "code passes lint" → Run lint and check passed
    vocab.add_primitive(
        "code passes lint",
        Primitive::CodeQuality {
            file: None,
            lint: true,
            validate: true,
        },
    );

    // "code quality passes" → synonym for code passes lint
    vocab.add_primitive(
        "code quality passes",
        Primitive::CodeQuality {
            file: None,
            lint: true,
            validate: true,
        },
    );

    // "no syntax errors" → Check code.quality.error_count equals 0
    vocab.add_primitive(
        "no syntax errors",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "code.quality.error_count".to_string(),
            expected: Value::Number(0.0),
        },
    );

    // "no lint errors" → Check code.quality.error_count equals 0
    vocab.add_terms("no lint errors", vec![Term::new("no syntax errors")]);

    // "no lint warnings" → Check code.quality.warning_count equals 0
    vocab.add_primitive(
        "no lint warnings",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "code.quality.warning_count".to_string(),
            expected: Value::Number(0.0),
        },
    );

    // "code is valid" → Check code.quality.passed is true
    vocab.add_primitive(
        "code is valid",
        Primitive::Check {
            op: CheckOp::Equals,
            path: "code.quality.passed".to_string(),
            expected: Value::Bool(true),
        },
    );

    vocab
}

/// Parse a glossary section from an intent file and add to vocabulary.
///
/// Glossary format:
/// ```text
/// ## Glossary
///
/// valid user id:
///   - body contains "id"
///   - body contains "name"
///
/// authenticated response:
///   - status: 200
///   - header authorization exists
/// ```
pub fn parse_glossary(content: &str, vocab: &mut Vocabulary) -> Result<(), String> {
    let mut in_glossary = false;
    let mut current_term: Option<String> = None;
    let mut current_definitions: Vec<Term> = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim();

        // Check for glossary section
        if trimmed.starts_with("## Glossary") || trimmed.starts_with("## glossary") {
            in_glossary = true;
            continue;
        }

        // Check for end of glossary (next section)
        if in_glossary && trimmed.starts_with("## ") && !trimmed.to_lowercase().contains("glossary")
        {
            // Save current term if any
            if let Some(term_name) = current_term.take() {
                if !current_definitions.is_empty() {
                    vocab.add_terms(&term_name, std::mem::take(&mut current_definitions));
                }
            }
            in_glossary = false;
            continue;
        }

        if !in_glossary {
            continue;
        }

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Check for term definition (ends with colon, not indented)
        if trimmed.ends_with(':') && !line.starts_with(' ') && !line.starts_with('\t') {
            // Save previous term if any
            if let Some(term_name) = current_term.take() {
                if !current_definitions.is_empty() {
                    vocab.add_terms(&term_name, std::mem::take(&mut current_definitions));
                }
            }

            // Start new term
            current_term = Some(trimmed.trim_end_matches(':').to_string());
            current_definitions.clear();
        }
        // Check for definition item (starts with -)
        else if trimmed.starts_with("- ") && current_term.is_some() {
            let def_text = trimmed.trim_start_matches("- ").trim();
            current_definitions.push(Term::new(def_text));
        }
    }

    // Save last term if any
    if let Some(term_name) = current_term {
        if !current_definitions.is_empty() {
            vocab.add_terms(&term_name, current_definitions);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_standard_vocabulary_has_status() {
        let vocab = standard_vocabulary();

        // Should find status: pattern
        let result = vocab.lookup("status: 200");
        assert!(result.is_some(), "Should find 'status: 200' pattern");
    }

    #[test]
    fn test_standard_vocabulary_has_body_contains() {
        let vocab = standard_vocabulary();

        let result = vocab.lookup("body contains \"hello\"");
        assert!(result.is_some(), "Should find 'body contains' pattern");
    }

    #[test]
    fn test_standard_vocabulary_has_2xx() {
        let vocab = standard_vocabulary();

        let result = vocab.lookup("status 2xx");
        assert!(result.is_some(), "Should find 'status 2xx' pattern");
    }

    #[test]
    fn test_parse_glossary() {
        let content = r#"
# My App

## Glossary

valid user:
  - body contains "id"
  - body contains "name"

authenticated:
  - status: 200
  - header authorization exists

## Features
"#;

        let mut vocab = standard_vocabulary();
        parse_glossary(content, &mut vocab).unwrap();

        // Should find custom term
        let result = vocab.lookup("valid user");
        assert!(result.is_some(), "Should find 'valid user' glossary term");
    }

    #[test]
    fn test_content_type_shortcuts() {
        let vocab = standard_vocabulary();

        // These should exist as terms that expand to header checks
        assert!(vocab.lookup("content-type is json").is_some());
        assert!(vocab.lookup("content-type is html").is_some());
        assert!(vocab.lookup("content-type is text").is_some());
    }
}
