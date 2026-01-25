//! Resolution: the heart of the term rewriting engine.
//!
//! This module contains the `resolve` function - a pure function that takes a term
//! and recursively rewrites it until reaching primitives. This is ~30 lines of core logic.
//!
//! Includes cycle detection to prevent infinite loops in glossary definitions.
//!
//! # Resolution Traces
//!
//! For debugging and visualization, use `resolve_with_trace()` to get a step-by-step
//! record of how a term was resolved. This is used by Intent Studio to show
//! resolution chains in the UI.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use super::primitives::{Primitive, Value};
use super::vocabulary::{Definition, Term, Vocabulary};

// ============================================================================
// Resolution Traces
// ============================================================================

/// A single step in the resolution process
#[derive(Debug, Clone, Serialize)]
pub struct ResolutionStep {
    /// The term being resolved at this step
    pub term: String,
    /// Where this term was found: "glossary", "standard", "primitive"
    pub source: String,
    /// Nesting depth (0 = top level)
    pub depth: usize,
    /// What this term resolved to (term texts or primitive descriptions)
    pub resolved_to: Vec<String>,
}

/// Complete trace of a resolution from term to primitives
#[derive(Debug, Clone, Serialize)]
pub struct ResolutionTrace {
    /// The original term that was resolved
    pub original_term: String,
    /// Each step in the resolution process
    pub steps: Vec<ResolutionStep>,
    /// Human-readable descriptions of the final primitives
    pub final_primitives: Vec<String>,
    /// Whether resolution succeeded
    pub success: bool,
    /// Error message if resolution failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Error during resolution
#[derive(Debug, Clone)]
pub struct ResolveError {
    pub term: String,
    pub message: String,
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to resolve '{}': {}", self.term, self.message)
    }
}

impl std::error::Error for ResolveError {}

/// Resolve a term to primitives by recursive rewriting.
///
/// This is the core of the IAL engine. It:
/// 1. Looks up the term in the vocabulary
/// 2. If it's a primitive → return it (base case)
/// 3. If it's more terms → substitute params and recurse (recursive case)
///
/// Includes cycle detection: if the same term pattern appears twice in the
/// resolution path, an error is returned with the cycle shown.
///
/// # Arguments
/// * `term` - The term to resolve
/// * `vocab` - The vocabulary containing all term definitions
///
/// # Returns
/// * `Ok(Vec<Primitive>)` - The resolved primitives
/// * `Err(ResolveError)` - If term not found, cycle detected, or resolution fails
pub fn resolve(term: &Term, vocab: &Vocabulary) -> Result<Vec<Primitive>, ResolveError> {
    let mut path = Vec::new();
    let mut visited = HashSet::new();
    resolve_with_cycle_detection(term, vocab, 0, &mut path, &mut visited, None)
}

/// Resolve a term to primitives and collect a resolution trace.
///
/// This is the same as `resolve()` but also returns a step-by-step trace
/// of how the term was resolved. Use this for debugging or visualization.
///
/// # Arguments
/// * `term` - The term to resolve
/// * `vocab` - The vocabulary containing all term definitions
///
/// # Returns
/// A tuple of (primitives, trace) where trace shows each resolution step.
pub fn resolve_with_trace(
    term: &Term,
    vocab: &Vocabulary,
) -> (Result<Vec<Primitive>, ResolveError>, ResolutionTrace) {
    let mut path = Vec::new();
    let mut visited = HashSet::new();
    let mut steps = Vec::new();

    let result =
        resolve_with_cycle_detection(term, vocab, 0, &mut path, &mut visited, Some(&mut steps));

    let trace = ResolutionTrace {
        original_term: term.text.clone(),
        steps,
        final_primitives: match &result {
            Ok(primitives) => primitives.iter().map(primitive_to_string).collect(),
            Err(_) => vec![],
        },
        success: result.is_ok(),
        error: result.as_ref().err().map(|e| e.message.clone()),
    };

    (result, trace)
}

/// Convert a primitive to a human-readable string for traces
fn primitive_to_string(primitive: &Primitive) -> String {
    match primitive {
        Primitive::Check { op, path, expected } => {
            format!("check: {} {:?} {:?}", path, op, expected)
        }
        Primitive::Http { method, path, .. } => {
            format!("http: {} {}", method, path)
        }
        Primitive::Cli { command, args } => {
            format!("cli: {} {}", command, args.join(" "))
        }
        Primitive::Sql { query, .. } => {
            format!("sql: {}", query)
        }
        Primitive::ReadFile { path } => {
            format!("read_file: {}", path)
        }
        Primitive::CodeQuality {
            file,
            lint,
            validate,
        } => {
            format!(
                "code_quality: file={:?} lint={} validate={}",
                file, lint, validate
            )
        }
        Primitive::FunctionCall { function_name, .. } => {
            format!("function_call: {}", function_name)
        }
        Primitive::PropertyCheck { property, .. } => {
            format!("property_check: {:?}", property)
        }
        Primitive::InvariantCheck { invariant_id, .. } => {
            format!("invariant_check: {}", invariant_id)
        }
    }
}

/// Maximum recursion depth as a safety net (in addition to cycle detection)
const MAX_DEPTH: usize = 50;

/// Normalize a term text for cycle detection.
/// Strips parameters to detect cycles like: "a {x}" -> "b {x}" -> "a {y}"
fn normalize_term_for_cycle(text: &str) -> String {
    // Replace quoted strings and parameter-like values with placeholders
    let re = regex::Regex::new(r#"("[^"]*"|\{[^}]+\}|\S+)"#).unwrap();
    let mut normalized = String::new();
    let mut last_end = 0;

    for cap in re.captures_iter(text) {
        let m = cap.get(0).unwrap();
        normalized.push_str(&text[last_end..m.start()]);

        let matched = m.as_str();
        if matched.starts_with('"') || matched.starts_with('{') {
            // Replace quoted strings and {param} with a placeholder
            normalized.push_str("<P>");
        } else {
            // Keep literal words
            normalized.push_str(matched);
        }
        last_end = m.end();
    }
    normalized.push_str(&text[last_end..]);
    normalized.to_lowercase()
}

/// Internal resolve with cycle detection, depth tracking, and optional trace collection
fn resolve_with_cycle_detection(
    term: &Term,
    vocab: &Vocabulary,
    depth: usize,
    path: &mut Vec<String>,
    visited: &mut HashSet<String>,
    mut trace: Option<&mut Vec<ResolutionStep>>,
) -> Result<Vec<Primitive>, ResolveError> {
    // Check for maximum depth (safety net)
    if depth > MAX_DEPTH {
        return Err(ResolveError {
            term: term.text.clone(),
            message: format!(
                "Maximum recursion depth ({}) exceeded. Resolution path:\n  {}",
                MAX_DEPTH,
                path.join("\n  → ")
            ),
        });
    }

    // Normalize the term for cycle detection
    let normalized = normalize_term_for_cycle(&term.text);

    // Check for cycle
    if visited.contains(&normalized) {
        // Build a clear cycle visualization
        let cycle_start = path
            .iter()
            .position(|p| normalize_term_for_cycle(p) == normalized);
        let cycle_path = if let Some(start) = cycle_start {
            let mut cycle: Vec<_> = path[start..].to_vec();
            cycle.push(term.text.clone());
            cycle.join(" → ")
        } else {
            format!("... → {}", term.text)
        };

        return Err(ResolveError {
            term: term.text.clone(),
            message: format!(
                "Cycle detected in glossary definitions:\n  {}\n\nThis creates an infinite loop. \
                 Check your glossary for circular references.",
                cycle_path
            ),
        });
    }

    // Look up term in vocabulary
    match vocab.lookup(&term.text) {
        Some((captured_params, definition)) => {
            // Merge captured params with term's existing params
            let mut all_params = term.params.clone();
            for (k, v) in captured_params {
                all_params.insert(k, v);
            }

            match definition {
                // Base case: primitive - substitute params and return
                Definition::Primitive(primitive) => {
                    let result = substitute_primitive(primitive, &all_params);

                    // Record trace step if tracing
                    if let Some(steps) = trace {
                        steps.push(ResolutionStep {
                            term: term.text.clone(),
                            source: "primitive".to_string(),
                            depth,
                            resolved_to: vec![primitive_to_string(&result)],
                        });
                    }

                    Ok(vec![result])
                }

                // Recursive case: more terms - substitute and resolve each
                Definition::Terms(sub_terms) => {
                    // Add this term to the path for cycle detection
                    path.push(term.text.clone());
                    visited.insert(normalized.clone());

                    // Collect sub-term texts for trace
                    let sub_term_texts: Vec<String> = sub_terms
                        .iter()
                        .map(|t| t.substitute(&all_params).text)
                        .collect();

                    // Add the trace step before recursing
                    if let Some(ref mut steps) = trace {
                        steps.push(ResolutionStep {
                            term: term.text.clone(),
                            // Heuristic: depth 0-1 likely glossary, deeper is standard vocab
                            source: if depth <= 1 {
                                "glossary".to_string()
                            } else {
                                "standard".to_string()
                            },
                            depth,
                            resolved_to: sub_term_texts,
                        });
                    }

                    let mut results = Vec::new();

                    for sub_term in sub_terms {
                        // Substitute params from parent into sub-term
                        let substituted = sub_term.substitute(&all_params);

                        // Recurse with updated path (pass trace through)
                        let resolved = resolve_with_cycle_detection(
                            &substituted,
                            vocab,
                            depth + 1,
                            path,
                            visited,
                            trace.as_deref_mut().map(|s| s as &mut Vec<ResolutionStep>),
                        )?;
                        results.extend(resolved);
                    }

                    // Remove from path when done (backtrack)
                    path.pop();
                    visited.remove(&normalized);

                    Ok(results)
                }
            }
        }

        None => Err(ResolveError {
            term: term.text.clone(),
            message: "Unknown term - not found in vocabulary".to_string(),
        }),
    }
}

/// Substitute parameters into a primitive.
/// Replaces {param} placeholders in string values.
fn substitute_primitive(primitive: &Primitive, params: &HashMap<String, Value>) -> Primitive {
    match primitive {
        Primitive::Http {
            method,
            path,
            body,
            headers,
        } => Primitive::Http {
            method: substitute_string(method, params),
            path: substitute_string(path, params),
            body: body.as_ref().map(|b| substitute_string(b, params)),
            headers: headers.clone(),
        },

        Primitive::Cli { command, args } => Primitive::Cli {
            command: substitute_string(command, params),
            args: args.iter().map(|a| substitute_string(a, params)).collect(),
        },

        Primitive::Sql {
            query,
            params: sql_params,
        } => Primitive::Sql {
            query: substitute_string(query, params),
            params: sql_params.clone(),
        },

        Primitive::ReadFile { path } => Primitive::ReadFile {
            path: substitute_string(path, params),
        },

        Primitive::Check { op, path, expected } => Primitive::Check {
            op: op.clone(),
            path: substitute_string(path, params),
            expected: substitute_value(expected, params),
        },

        Primitive::CodeQuality {
            file,
            lint,
            validate,
        } => Primitive::CodeQuality {
            file: file.as_ref().map(|f| substitute_string(f, params)),
            lint: *lint,
            validate: *validate,
        },

        Primitive::FunctionCall {
            source_file,
            function_name,
            args,
        } => Primitive::FunctionCall {
            source_file: substitute_string(source_file, params),
            function_name: substitute_string(function_name, params),
            args: args.iter().map(|a| substitute_value(a, params)).collect(),
        },

        Primitive::PropertyCheck {
            property,
            source_file,
            function_name,
            input,
        } => Primitive::PropertyCheck {
            property: property.clone(),
            source_file: substitute_string(source_file, params),
            function_name: substitute_string(function_name, params),
            input: substitute_value(input, params),
        },

        Primitive::InvariantCheck {
            invariant_id,
            value,
        } => Primitive::InvariantCheck {
            invariant_id: substitute_string(invariant_id, params),
            value: substitute_value(value, params),
        },
    }
}

/// Substitute parameters into a string
fn substitute_string(s: &str, params: &HashMap<String, Value>) -> String {
    let mut result = s.to_string();

    for (name, value) in params {
        let placeholder1 = format!("{{{}}}", name);
        let placeholder2 = format!("${}", name);

        if let Value::String(v) = value {
            result = result.replace(&placeholder1, v);
            result = result.replace(&placeholder2, v);
        } else if let Value::Number(n) = value {
            let v = if n.fract() == 0.0 {
                format!("{}", *n as i64)
            } else {
                format!("{}", n)
            };
            result = result.replace(&placeholder1, &v);
            result = result.replace(&placeholder2, &v);
        }
    }

    result
}

/// Substitute parameters into a value
fn substitute_value(value: &Value, params: &HashMap<String, Value>) -> Value {
    match value {
        Value::String(s) => {
            let substituted = substitute_string(s, params);
            // Try to parse as number if it looks like one
            if let Ok(n) = substituted.parse::<f64>() {
                Value::Number(n)
            } else {
                Value::String(substituted)
            }
        }
        _ => value.clone(),
    }
}

/// Resolve multiple terms and collect all primitives
pub fn resolve_all(terms: &[Term], vocab: &Vocabulary) -> Result<Vec<Primitive>, ResolveError> {
    let mut results = Vec::new();
    for term in terms {
        results.extend(resolve(term, vocab)?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ial::primitives::CheckOp;

    fn test_vocab() -> Vocabulary {
        let mut vocab = Vocabulary::new();

        // Add some test terms
        vocab.add_primitive(
            "status is 2xx",
            Primitive::Check {
                op: CheckOp::InRange,
                path: "response.status".to_string(),
                expected: Value::Range(200.0, 299.0),
            },
        );

        vocab.add_primitive(
            "body contains {text}",
            Primitive::Check {
                op: CheckOp::Contains,
                path: "response.body".to_string(),
                expected: Value::String("{text}".to_string()),
            },
        );

        vocab.add_terms(
            "success response",
            vec![
                Term::new("status is 2xx"),
                Term::new("body contains \"ok\""),
            ],
        );

        vocab.add_terms(
            "success response with {message}",
            vec![
                Term::new("status is 2xx"),
                Term::new("body contains \"ok\""),
                Term::new("body contains {message}"),
            ],
        );

        vocab
    }

    #[test]
    fn test_resolve_primitive() {
        let vocab = test_vocab();
        let term = Term::new("status is 2xx");

        let result = resolve(&term, &vocab).unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], Primitive::Check { .. }));
    }

    #[test]
    fn test_resolve_with_param() {
        let vocab = test_vocab();
        let term = Term::new("body contains \"hello\"");

        let result = resolve(&term, &vocab).unwrap();
        assert_eq!(result.len(), 1);

        if let Primitive::Check { expected, .. } = &result[0] {
            assert_eq!(expected, &Value::String("hello".to_string()));
        } else {
            panic!("Expected Check primitive");
        }
    }

    #[test]
    fn test_resolve_terms() {
        let vocab = test_vocab();
        let term = Term::new("success response");

        let result = resolve(&term, &vocab).unwrap();
        assert_eq!(result.len(), 2); // status is 2xx + body contains "ok"
    }

    #[test]
    fn test_resolve_terms_with_param() {
        let vocab = test_vocab();
        let term = Term::new("success response with \"Task created\"");

        let result = resolve(&term, &vocab).unwrap();
        assert_eq!(result.len(), 3);

        // Third primitive should have substituted message
        if let Primitive::Check { expected, .. } = &result[2] {
            assert_eq!(expected, &Value::String("Task created".to_string()));
        } else {
            panic!("Expected Check primitive");
        }
    }

    #[test]
    fn test_resolve_unknown_term() {
        let vocab = test_vocab();
        let term = Term::new("unknown term here");

        let result = resolve(&term, &vocab);
        assert!(result.is_err());
    }

    #[test]
    fn test_cycle_detection_direct() {
        // Create a vocabulary with a direct cycle: A -> A
        let mut vocab = Vocabulary::new();
        vocab.add_terms(
            "self referencing term",
            vec![Term::new("self referencing term")],
        );

        let term = Term::new("self referencing term");
        let result = resolve(&term, &vocab);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("Cycle detected"),
            "Expected cycle detection error, got: {}",
            err.message
        );
    }

    #[test]
    fn test_cycle_detection_indirect() {
        // Create a vocabulary with an indirect cycle: A -> B -> C -> A
        let mut vocab = Vocabulary::new();
        vocab.add_terms("term A", vec![Term::new("term B")]);
        vocab.add_terms("term B", vec![Term::new("term C")]);
        vocab.add_terms("term C", vec![Term::new("term A")]);

        let term = Term::new("term A");
        let result = resolve(&term, &vocab);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("Cycle detected"),
            "Expected cycle detection error, got: {}",
            err.message
        );
        // The error should show the cycle path
        assert!(
            err.message.contains("term A") && err.message.contains("term B"),
            "Error should show cycle path, got: {}",
            err.message
        );
    }

    #[test]
    fn test_cycle_detection_with_params() {
        // Cycle with different parameter values should still be detected
        // "foo {x}" -> "bar {x}" -> "foo {y}" (cycle on normalized "foo <P>")
        let mut vocab = Vocabulary::new();
        vocab.add_terms("foo {x}", vec![Term::new("bar {x}")]);
        vocab.add_terms("bar {y}", vec![Term::new("foo {y}")]);

        let term = Term::new("foo \"test\"");
        let result = resolve(&term, &vocab);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("Cycle detected"),
            "Expected cycle detection error with params, got: {}",
            err.message
        );
    }

    #[test]
    fn test_no_false_positive_diamond() {
        // Diamond pattern should NOT be detected as a cycle:
        //       A
        //      / \
        //     B   C
        //      \ /
        //       D (primitive)
        //
        // A resolves to both B and C, both of which resolve to D
        // This is NOT a cycle because D is a primitive (base case)
        let mut vocab = Vocabulary::new();
        vocab.add_primitive(
            "primitive D",
            Primitive::Check {
                op: CheckOp::Equals,
                path: "test".to_string(),
                expected: Value::String("value".to_string()),
            },
        );
        vocab.add_terms("term B", vec![Term::new("primitive D")]);
        vocab.add_terms("term C", vec![Term::new("primitive D")]);
        vocab.add_terms("term A", vec![Term::new("term B"), Term::new("term C")]);

        let term = Term::new("term A");
        let result = resolve(&term, &vocab);

        assert!(result.is_ok(), "Diamond pattern should not be a cycle");
        let primitives = result.unwrap();
        assert_eq!(
            primitives.len(),
            2,
            "Should resolve to 2 primitives (D twice)"
        );
    }

    #[test]
    fn test_normalize_term_for_cycle() {
        // Test the normalization function
        // Quoted strings become <P>
        assert_eq!(
            normalize_term_for_cycle("body contains \"hello\""),
            "body contains <p>"
        );
        // {param} placeholders become <P>
        assert_eq!(
            normalize_term_for_cycle("body contains {text}"),
            "body contains <p>"
        );
        // Literal words stay as-is (lowercased)
        assert_eq!(normalize_term_for_cycle("status is 2xx"), "status is 2xx");
        assert_eq!(normalize_term_for_cycle("simple term"), "simple term");
        // Mixed: params replaced, literals kept
        assert_eq!(
            normalize_term_for_cycle("user {name} sees \"welcome\""),
            "user <p> sees <p>"
        );
    }

    #[test]
    fn test_resolve_with_trace() {
        let vocab = test_vocab();
        let term = Term::new("success response");

        let (result, trace) = resolve_with_trace(&term, &vocab);

        // Should succeed
        assert!(result.is_ok());
        assert!(trace.success);
        assert!(trace.error.is_none());

        // Should have steps
        assert!(!trace.steps.is_empty(), "Trace should have steps");

        // First step should be the original term
        assert_eq!(trace.steps[0].term, "success response");
        assert_eq!(trace.steps[0].depth, 0);

        // Should have final primitives
        assert_eq!(trace.final_primitives.len(), 2);
    }

    #[test]
    fn test_resolve_with_trace_error() {
        let vocab = test_vocab();
        let term = Term::new("unknown term");

        let (result, trace) = resolve_with_trace(&term, &vocab);

        // Should fail
        assert!(result.is_err());
        assert!(!trace.success);
        assert!(trace.error.is_some());
        assert!(trace.final_primitives.is_empty());
    }
}
