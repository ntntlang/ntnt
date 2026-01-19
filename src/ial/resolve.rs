//! Resolution: the heart of the term rewriting engine.
//!
//! This module contains the `resolve` function - a pure function that takes a term
//! and recursively rewrites it until reaching primitives. This is ~30 lines of core logic.

use std::collections::HashMap;

use super::primitives::{Primitive, Value};
use super::vocabulary::{Definition, Term, Vocabulary};

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
/// # Arguments
/// * `term` - The term to resolve
/// * `vocab` - The vocabulary containing all term definitions
///
/// # Returns
/// * `Ok(Vec<Primitive>)` - The resolved primitives
/// * `Err(ResolveError)` - If term not found or resolution fails
pub fn resolve(term: &Term, vocab: &Vocabulary) -> Result<Vec<Primitive>, ResolveError> {
    resolve_with_depth(term, vocab, 0)
}

/// Maximum recursion depth to prevent infinite loops
const MAX_DEPTH: usize = 100;

/// Internal resolve with depth tracking
fn resolve_with_depth(
    term: &Term,
    vocab: &Vocabulary,
    depth: usize,
) -> Result<Vec<Primitive>, ResolveError> {
    // Check for infinite recursion
    if depth > MAX_DEPTH {
        return Err(ResolveError {
            term: term.text.clone(),
            message: format!("Maximum recursion depth ({}) exceeded", MAX_DEPTH),
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
                    Ok(vec![substitute_primitive(primitive, &all_params)])
                }

                // Recursive case: more terms - substitute and resolve each
                Definition::Terms(sub_terms) => {
                    let mut results = Vec::new();

                    for sub_term in sub_terms {
                        // Substitute params from parent into sub-term
                        let substituted = sub_term.substitute(&all_params);

                        // Recurse
                        let resolved = resolve_with_depth(&substituted, vocab, depth + 1)?;
                        results.extend(resolved);
                    }

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
}
