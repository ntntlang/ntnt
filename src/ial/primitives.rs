//! Core primitives for the IAL term rewriting engine.
//!
//! Primitives are the base cases - the ONLY things that actually execute.
//! Everything else (glossary terms, components, standard assertions) rewrites to these.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The primitives - the ONLY things that actually DO anything.
/// This is a small, fixed set. New capabilities come from vocabulary, not new primitives.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Primitive {
    // === Actions (produce results stored in Context) ===
    /// HTTP request: method, path, optional body, optional headers
    Http {
        method: String,
        path: String,
        body: Option<String>,
        headers: Option<HashMap<String, String>>,
    },

    /// CLI command execution (restricted to safe commands)
    Cli { command: String, args: Vec<String> },

    /// Code quality checks (lint + validate) - runs on all .tnt files in project
    /// This is a safe, built-in check that doesn't execute arbitrary code.
    CodeQuality {
        /// Optional specific file to check (if None, checks all .tnt files)
        file: Option<String>,
        /// Whether to run lint checks
        lint: bool,
        /// Whether to run validation checks
        validate: bool,
    },

    /// SQL query execution
    Sql { query: String, params: Vec<Value> },

    /// Read file contents
    ReadFile { path: String },

    // === Checks (verify values in Context) ===
    /// Universal check: operation on a path with expected value
    Check {
        op: CheckOp,
        path: String,
        expected: Value,
    },
}

/// Check operations - the universal set of comparisons.
/// These cover all assertion types across all domains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CheckOp {
    /// Exact equality
    Equals,
    /// Not equal
    NotEquals,
    /// String/array contains
    Contains,
    /// String/array does not contain
    NotContains,
    /// Regex match
    Matches,
    /// Value exists (not null/missing)
    Exists,
    /// Value does not exist
    NotExists,
    /// Numeric less than
    LessThan,
    /// Numeric greater than
    GreaterThan,
    /// Numeric in range (inclusive)
    InRange,
}

/// Values in the system - what primitives operate on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// String value
    String(String),
    /// Numeric value (integers and floats unified)
    Number(f64),
    /// Boolean value
    Bool(bool),
    /// Range for InRange checks (min, max inclusive)
    Range(f64, f64),
    /// Regex pattern for Matches checks
    Regex(String),
    /// Null/missing value
    Null,
    /// Array of values
    Array(Vec<Value>),
    /// Map of string keys to values
    Map(HashMap<String, Value>),
}

impl Value {
    /// Create a string value
    pub fn string(s: impl Into<String>) -> Self {
        Value::String(s.into())
    }

    /// Create a number value
    pub fn number(n: impl Into<f64>) -> Self {
        Value::Number(n.into())
    }

    /// Create a range value (for status codes like 2xx)
    pub fn range(min: impl Into<f64>, max: impl Into<f64>) -> Self {
        Value::Range(min.into(), max.into())
    }

    /// Create a regex value
    pub fn regex(pattern: impl Into<String>) -> Self {
        Value::Regex(pattern.into())
    }

    /// Try to get as string
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// Try to get as number
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    /// Try to get as bool
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::String(s)
    }
}

impl From<i32> for Value {
    fn from(n: i32) -> Self {
        Value::Number(n as f64)
    }
}

impl From<u16> for Value {
    fn from(n: u16) -> Self {
        Value::Number(n as f64)
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Bool(b)
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::String(s) => write!(f, "{}", s),
            Value::Number(n) => {
                // Display integers without decimal point
                if n.fract() == 0.0 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{}", n)
                }
            }
            Value::Bool(b) => write!(f, "{}", b),
            Value::Range(min, max) => write!(f, "{}-{}", min, max),
            Value::Regex(pattern) => write!(f, "/{}/", pattern),
            Value::Null => write!(f, "null"),
            Value::Array(items) => {
                let items_str: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                write!(f, "[{}]", items_str.join(", "))
            }
            Value::Map(map) => {
                let pairs: Vec<String> = map.iter().map(|(k, v)| format!("{}: {}", k, v)).collect();
                write!(f, "{{{}}}", pairs.join(", "))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_value_creation() {
        assert_eq!(Value::string("hello"), Value::String("hello".to_string()));
        assert_eq!(Value::number(42), Value::Number(42.0));
        assert_eq!(Value::range(200, 299), Value::Range(200.0, 299.0));
    }

    #[test]
    fn test_value_from() {
        let v: Value = "test".into();
        assert_eq!(v, Value::String("test".to_string()));

        let v: Value = 42.into();
        assert_eq!(v, Value::Number(42.0));
    }

    #[test]
    fn test_primitive_http() {
        let p = Primitive::Http {
            method: "GET".to_string(),
            path: "/api/tasks".to_string(),
            body: None,
            headers: None,
        };
        assert!(matches!(p, Primitive::Http { .. }));
    }

    #[test]
    fn test_primitive_check() {
        let p = Primitive::Check {
            op: CheckOp::Contains,
            path: "response.body".to_string(),
            expected: Value::string("hello"),
        };
        assert!(matches!(p, Primitive::Check { .. }));
    }
}
