//! IAL Term Rewriting Engine
//!
//! This module implements Intent Assertion Language (IAL) as a term rewriting system.
//!
//! # Architecture
//!
//! The engine has three core functions:
//!
//! 1. **parse()** - Convert assertion text into Terms
//! 2. **resolve()** - Recursively rewrite Terms into Primitives using the Vocabulary
//! 3. **execute()** - Run Primitives against a Context
//!
//! # Vocabulary
//!
//! Everything is vocabulary - glossary terms, standard assertions, and components
//! are all entries that rewrite to other terms or primitives.
//!
//! # Example
//!
//! ```text
//! // Assertion text
//! "valid user response"
//!
//! // Glossary lookup rewrites to:
//! ["status 2xx", "body has user fields"]
//!
//! // Standard lookup rewrites "body has user fields" to:
//! ["body contains \"id\"", "body contains \"name\""]
//!
//! // Standard lookup rewrites each to primitives:
//! [Check { op: InRange, path: "response.status", expected: Range(200, 299) },
//!  Check { op: Contains, path: "response.body", expected: "id" },
//!  Check { op: Contains, path: "response.body", expected: "name" }]
//! ```

pub mod execute;
pub mod primitives;
pub mod resolve;
pub mod standard;
pub mod vocabulary;

// Re-export main types
pub use execute::{execute, execute_all, Context, ExecuteResult};
pub use primitives::{CheckOp, Primitive, Value};
pub use resolve::{resolve, resolve_all, ResolveError};
pub use standard::{parse_glossary, standard_vocabulary};
pub use vocabulary::{Definition, Pattern, Term, Vocabulary};

/// Result type for IAL operations
pub type IalResult<T> = Result<T, IalError>;

/// Unified error type for IAL
#[derive(Debug, Clone)]
pub enum IalError {
    /// Failed to parse a pattern
    PatternError(String),
    /// Failed to resolve a term
    ResolveError(String),
    /// Failed to execute a primitive
    ExecuteError(String),
    /// Generic error
    Other(String),
}

impl std::fmt::Display for IalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IalError::PatternError(msg) => write!(f, "Pattern error: {}", msg),
            IalError::ResolveError(msg) => write!(f, "Resolve error: {}", msg),
            IalError::ExecuteError(msg) => write!(f, "Execute error: {}", msg),
            IalError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for IalError {}

impl From<ResolveError> for IalError {
    fn from(err: ResolveError) -> Self {
        IalError::ResolveError(err.to_string())
    }
}

/// High-level API: Run assertions against a running server.
///
/// This is the main entry point for running IAL tests.
///
/// # Arguments
///
/// * `assertions` - List of assertion strings to evaluate
/// * `vocab` - The vocabulary to use for resolution
/// * `port` - The port where the server is running
///
/// # Returns
///
/// A vector of execution results, one per primitive generated.
pub fn run_assertions(
    assertions: &[String],
    vocab: &Vocabulary,
    port: u16,
) -> IalResult<Vec<ExecuteResult>> {
    let mut ctx = Context::new();
    let mut all_results = Vec::new();

    for assertion in assertions {
        let term = Term::new(assertion);
        let primitives = resolve(&term, vocab)?;

        let results = execute_all(&primitives, &mut ctx, port);
        all_results.extend(results);
    }

    Ok(all_results)
}

/// High-level API: Run a single test scenario.
///
/// A scenario consists of an HTTP request followed by assertions.
///
/// # Arguments
///
/// * `method` - HTTP method (GET, POST, etc.)
/// * `path` - Request path
/// * `body` - Optional request body
/// * `assertions` - List of assertions to verify
/// * `vocab` - The vocabulary to use
/// * `port` - The server port
///
/// # Returns
///
/// A tuple of (passed, results) where passed is true if all assertions passed.
pub fn run_scenario(
    method: &str,
    path: &str,
    body: Option<&str>,
    assertions: &[String],
    vocab: &Vocabulary,
    port: u16,
) -> IalResult<(bool, Vec<ExecuteResult>)> {
    let mut ctx = Context::new();
    let mut all_results = Vec::new();

    // Execute the HTTP request first
    let request = Primitive::Http {
        method: method.to_string(),
        path: path.to_string(),
        body: body.map(String::from),
        headers: None,
    };

    let request_result = execute::execute(&request, &mut ctx, port);
    let request_passed = request_result.passed;
    all_results.push(request_result);

    // Only run assertions if request succeeded
    if request_passed {
        for assertion in assertions {
            let term = Term::new(assertion);
            let primitives = resolve(&term, vocab)?;

            for primitive in primitives {
                let result = execute::execute(&primitive, &mut ctx, port);
                all_results.push(result);
            }
        }
    }

    let all_passed = all_results.iter().all(|r| r.passed);
    Ok((all_passed, all_results))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_term_creation() {
        let term = Term::new("status: 200");
        assert_eq!(term.text, "status: 200");
    }

    #[test]
    fn test_vocabulary_with_standard() {
        let vocab = standard_vocabulary();

        // Should be able to look up standard terms
        assert!(vocab.lookup("status: 200").is_some());
        assert!(vocab.lookup("body contains \"test\"").is_some());
        assert!(vocab.lookup("status 2xx").is_some());
    }

    #[test]
    fn test_resolve_simple_status() {
        let vocab = standard_vocabulary();
        let term = Term::new("status 2xx");

        let primitives = resolve(&term, &vocab).unwrap();
        assert_eq!(primitives.len(), 1);

        match &primitives[0] {
            Primitive::Check { op, path, expected } => {
                assert!(matches!(op, CheckOp::InRange));
                assert_eq!(path, "response.status");
                assert!(matches!(expected, Value::Range(200.0, 299.0)));
            }
            _ => panic!("Expected Check primitive"),
        }
    }

    #[test]
    fn test_context_and_execute() {
        let mut ctx = Context::new();
        ctx.set("response.status", Value::Number(200.0));
        ctx.set("response.body", Value::String("Hello, World!".to_string()));

        // Test status check
        let check = Primitive::Check {
            op: CheckOp::Equals,
            path: "response.status".to_string(),
            expected: Value::Number(200.0),
        };

        let result = execute::execute(&check, &mut ctx, 8080);
        assert!(result.passed);

        // Test body contains check
        let check = Primitive::Check {
            op: CheckOp::Contains,
            path: "response.body".to_string(),
            expected: Value::String("World".to_string()),
        };

        let result = execute::execute(&check, &mut ctx, 8080);
        assert!(result.passed);
    }
}
