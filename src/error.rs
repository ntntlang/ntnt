//! Error types for the Intent language

use thiserror::Error;

/// Result type alias for Intent operations
pub type Result<T> = std::result::Result<T, IntentError>;

/// Rich context for type mismatch errors.
///
/// Provides structured expected/got information and optional hints,
/// inspired by Elm and Rust's error messages.
#[derive(Debug, Clone)]
pub struct TypeContext {
    /// What type was expected
    pub expected: String,
    /// What type was actually found
    pub got: String,
    /// Optional hint for how to fix
    pub hint: Option<String>,
}

impl TypeContext {
    pub fn new(expected: impl Into<String>, got: impl Into<String>) -> Self {
        Self {
            expected: expected.into(),
            got: got.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl std::fmt::Display for TypeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "expected {}, found {}", self.expected, self.got)?;
        if let Some(hint) = &self.hint {
            write!(f, "\n  hint: {}", hint)?;
        }
        Ok(())
    }
}

/// Main error type for Intent language operations
#[derive(Error, Debug)]
pub enum IntentError {
    #[error("Lexer error at line {line}, column {column}: {message}")]
    LexerError {
        line: usize,
        column: usize,
        message: String,
    },

    #[error("Parser error at line {line}, column {column}: {message}")]
    ParserError {
        line: usize,
        column: usize,
        message: String,
    },

    #[error("Type error: {message}")]
    TypeError {
        message: String,
        context: Option<TypeContext>,
        /// Source line (set by interpreter from current_line, 0 = unknown)
        line: usize,
    },

    #[error("Contract violation: {message}")]
    ContractViolation {
        message: String,
        /// 1-based line of the failing requires/ensures clause; 0 = unknown
        line: usize,
        /// 1-based line of the call site that triggered the check; 0 = unknown
        call_line: usize,
        /// Variable name/value pairs referenced by the failing clause
        values: Vec<(String, String)>,
    },

    #[error("Runtime error: {message}")]
    RuntimeError {
        message: String,
        context: Option<TypeContext>,
        /// Source line (set by interpreter from current_line, 0 = unknown)
        line: usize,
    },

    #[error("Undefined variable: {name}")]
    UndefinedVariable {
        name: String,
        suggestion: Option<String>,

        line: usize,
    },

    #[error("Undefined function: {name}")]
    UndefinedFunction {
        name: String,
        suggestion: Option<String>,
        /// Extra guidance beyond did-you-mean (e.g. the UFCS bridge hint)
        hint: Option<String>,

        line: usize,
    },

    #[error("Arity mismatch: function '{name}' expected {expected} arguments, got {got}")]
    ArityMismatch {
        name: String,
        expected: String,
        got: usize,

        line: usize,
    },

    #[error("Division by zero")]
    DivisionByZero { line: usize },

    #[error("Index out of bounds: index {index}, length {length}")]
    IndexOutOfBounds { index: i64, length: usize },

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Requires human approval: {0}")]
    RequiresApproval(String),
}

impl IntentError {
    /// Create a TypeError with just a message (backward compatible)
    pub fn type_error(message: impl Into<String>) -> Self {
        IntentError::TypeError {
            message: message.into(),
            context: None,
            line: 0,
        }
    }

    /// Create a TypeError with rich context (expected/got/hint)
    pub fn type_error_with_context(message: impl Into<String>, context: TypeContext) -> Self {
        IntentError::TypeError {
            message: message.into(),
            context: Some(context),
            line: 0,
        }
    }

    /// Create a ContractViolation with just a message (no location or values)
    pub fn contract_violation(message: impl Into<String>) -> Self {
        IntentError::ContractViolation {
            message: message.into(),
            line: 0,
            call_line: 0,
            values: Vec::new(),
        }
    }

    /// Create a RuntimeError with just a message (backward compatible)
    pub fn runtime_error(message: impl Into<String>) -> Self {
        IntentError::RuntimeError {
            message: message.into(),
            context: None,
            line: 0,
        }
    }

    /// Create a RuntimeError with rich context (expected/got/hint)
    pub fn runtime_error_with_context(message: impl Into<String>, context: TypeContext) -> Self {
        IntentError::RuntimeError {
            message: message.into(),
            context: Some(context),
            line: 0,
        }
    }

    /// Set the line number on a TypeError or RuntimeError (builder pattern).
    /// Usage: `IntentError::type_error("msg").at_line(42)`
    pub fn at_line(mut self, line: usize) -> Self {
        match &mut self {
            IntentError::TypeError { line: l, .. } => *l = line,
            IntentError::RuntimeError { line: l, .. } => *l = line,
            IntentError::UndefinedVariable { line: l, .. } => *l = line,
            IntentError::UndefinedFunction { line: l, .. } => *l = line,
            IntentError::ArityMismatch { line: l, .. } => *l = line,
            IntentError::DivisionByZero { line: l } => *l = line,
            // Backfill the call-site line from Located statement tracking,
            // but never overwrite a clause line already captured.
            IntentError::ContractViolation { line: l, .. } if *l == 0 => *l = line,
            _ => {}
        }
        self
    }

    /// Get the TypeContext if this error has one
    pub fn type_context(&self) -> Option<&TypeContext> {
        match self {
            IntentError::TypeError { context, .. } => context.as_ref(),
            IntentError::RuntimeError { context, .. } => context.as_ref(),
            _ => None,
        }
    }

    /// Format the error with rich context for terminal display
    pub fn rich_display(&self) -> String {
        let base = self.to_string();
        match self.type_context() {
            Some(ctx) => {
                let mut out = base;
                out.push_str(&format!("\n  │ expected: {}", ctx.expected));
                out.push_str(&format!("\n  │     found: {}", ctx.got));
                if let Some(hint) = &ctx.hint {
                    out.push_str(&format!("\n  │\n  └─ hint: {}", hint));
                }
                out
            }
            None => {
                // Add suggestion for known error types
                if let Some(suggestion) = self.suggestion() {
                    format!("{}\n  └─ did you mean: {}?", base, suggestion)
                } else {
                    base
                }
            }
        }
    }

    /// Return a unique error code for this error variant
    pub fn error_code(&self) -> &'static str {
        match self {
            IntentError::LexerError { .. } => "E001",
            IntentError::ParserError { .. } => "E002",
            IntentError::TypeError { .. } => "E003",
            IntentError::ContractViolation { .. } => "E004",
            IntentError::RuntimeError { .. } => "E005",
            IntentError::UndefinedVariable { .. } => "E006",
            IntentError::UndefinedFunction { .. } => "E007",
            IntentError::ArityMismatch { .. } => "E008",
            IntentError::DivisionByZero { .. } => "E009",
            IntentError::IndexOutOfBounds { .. } => "E010",
            IntentError::InvalidOperation(_) => "E011",
            IntentError::RequiresApproval(_) => "E012",
        }
    }

    /// Return the line number if this error has one
    pub fn line(&self) -> Option<usize> {
        match self {
            IntentError::LexerError { line, .. } => Some(*line),
            IntentError::ParserError { line, .. } => Some(*line),
            IntentError::TypeError { line, .. } if *line > 0 => Some(*line),
            IntentError::RuntimeError { line, .. } if *line > 0 => Some(*line),
            IntentError::UndefinedVariable { line, .. } if *line > 0 => Some(*line),
            IntentError::UndefinedFunction { line, .. } if *line > 0 => Some(*line),
            IntentError::ArityMismatch { line, .. } if *line > 0 => Some(*line),
            IntentError::DivisionByZero { line } if *line > 0 => Some(*line),
            IntentError::ContractViolation { line, .. } if *line > 0 => Some(*line),
            _ => None,
        }
    }

    /// Return the column number if this error has one
    pub fn column(&self) -> Option<usize> {
        match self {
            IntentError::LexerError { column, .. } => Some(*column),
            IntentError::ParserError { column, .. } => Some(*column),
            _ => None,
        }
    }

    /// Return the suggestion if this error has one
    pub fn suggestion(&self) -> Option<&str> {
        match self {
            IntentError::UndefinedVariable { suggestion, .. } => suggestion.as_deref(),
            IntentError::UndefinedFunction { suggestion, .. } => suggestion.as_deref(),
            _ => None,
        }
    }

    /// Return the extra hint if this error has one
    pub fn hint(&self) -> Option<&str> {
        match self {
            IntentError::UndefinedFunction { hint, .. } => hint.as_deref(),
            _ => None,
        }
    }
}

/// Method names agents commonly reach for, mapped to the NTNT function that
/// actually exists. Consulted before Levenshtein by both the runtime
/// method-miss error and the typechecker's unknown-method lint.
pub const METHOD_ALIAS_HINTS: &[(&str, &str)] = &[
    ("length", "len"),
    ("size", "len"),
    ("count", "len"),
    // no "map" entry: `map` is a keyword, so `.map(` is a parse error and
    // never reaches method dispatch
    ("to_string", "str"),
    ("to_str", "str"),
    ("append", "push"),
    ("upper", "to_upper"),
    ("lower", "to_lower"),
];

/// Compute the Levenshtein edit distance between two strings
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();

    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }

    // Use single-row optimization
    let mut prev_row: Vec<usize> = (0..=b_len).collect();
    let mut curr_row = vec![0; b_len + 1];

    for (i, a_char) in a.chars().enumerate() {
        curr_row[0] = i + 1;
        for (j, b_char) in b.chars().enumerate() {
            let cost = if a_char == b_char { 0 } else { 1 };
            curr_row[j + 1] = (prev_row[j + 1] + 1) // deletion
                .min(curr_row[j] + 1) // insertion
                .min(prev_row[j] + cost); // substitution
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }

    prev_row[b_len]
}

/// Find the best suggestion from a list of candidates for a misspelled name.
/// Returns None if no candidate is close enough.
pub fn find_suggestion(name: &str, candidates: &[String]) -> Option<String> {
    let name_lower = name.to_lowercase();
    let mut best: Option<(usize, &String)> = None;

    for candidate in candidates {
        // Skip internal/hidden names
        if candidate.starts_with('_') {
            continue;
        }

        let dist = levenshtein_distance(&name_lower, &candidate.to_lowercase());

        // Threshold: shorter names require closer matches
        let threshold = match name.len() {
            0..=2 => 1,
            3..=5 => 2,
            _ => 3,
        };

        if dist <= threshold {
            if let Some((best_dist, _)) = best {
                if dist < best_dist {
                    best = Some((dist, candidate));
                }
            } else {
                best = Some((dist, candidate));
            }
        }
    }

    best.map(|(_, s)| s.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance_identical() {
        assert_eq!(levenshtein_distance("hello", "hello"), 0);
    }

    #[test]
    fn test_levenshtein_distance_empty() {
        assert_eq!(levenshtein_distance("", "hello"), 5);
        assert_eq!(levenshtein_distance("hello", ""), 5);
        assert_eq!(levenshtein_distance("", ""), 0);
    }

    #[test]
    fn test_levenshtein_distance_single_edit() {
        assert_eq!(levenshtein_distance("cat", "car"), 1); // substitution
        assert_eq!(levenshtein_distance("cat", "cats"), 1); // insertion
        assert_eq!(levenshtein_distance("cats", "cat"), 1); // deletion
    }

    #[test]
    fn test_levenshtein_distance_multiple_edits() {
        assert_eq!(levenshtein_distance("kitten", "sitting"), 3);
        assert_eq!(levenshtein_distance("saturday", "sunday"), 3);
    }

    #[test]
    fn test_find_suggestion_close_match() {
        let candidates = vec![
            "users".to_string(),
            "items".to_string(),
            "count".to_string(),
        ];
        assert_eq!(
            find_suggestion("usres", &candidates),
            Some("users".to_string())
        );
        assert_eq!(
            find_suggestion("itmes", &candidates),
            Some("items".to_string())
        );
    }

    #[test]
    fn test_find_suggestion_no_match() {
        let candidates = vec![
            "users".to_string(),
            "items".to_string(),
            "count".to_string(),
        ];
        assert_eq!(
            find_suggestion("xyz_completely_different", &candidates),
            None
        );
    }

    #[test]
    fn test_find_suggestion_case_insensitive() {
        let candidates = vec!["Print".to_string()];
        assert_eq!(
            find_suggestion("prnt", &candidates),
            Some("Print".to_string())
        );
    }

    #[test]
    fn test_find_suggestion_skips_internal() {
        let candidates = vec!["_internal".to_string(), "public_fn".to_string()];
        assert_eq!(find_suggestion("_interanl", &candidates), None);
    }

    #[test]
    fn test_find_suggestion_picks_closest() {
        let candidates = vec![
            "print".to_string(),
            "printf".to_string(),
            "sprint".to_string(),
        ];
        assert_eq!(
            find_suggestion("prnt", &candidates),
            Some("print".to_string())
        );
    }

    #[test]
    fn contract_violation_carries_location_and_values() {
        let err = IntentError::ContractViolation {
            message: "Precondition failed in 'divide': b != 0".to_string(),
            line: 2,
            call_line: 7,
            values: vec![("b".to_string(), "0".to_string())],
        };

        assert_eq!(err.error_code(), "E004");
        assert_eq!(err.line(), Some(2));

        // at_line backfills only when the clause line is unknown
        let backfilled = IntentError::contract_violation("msg").at_line(9);
        assert_eq!(backfilled.line(), Some(9));
        let kept = err.at_line(99);
        assert_eq!(kept.line(), Some(2));
    }

    #[test]
    fn test_error_codes_unique() {
        let errors: Vec<IntentError> = vec![
            IntentError::LexerError {
                line: 0,
                column: 0,
                message: String::new(),
            },
            IntentError::ParserError {
                line: 0,
                column: 0,
                message: String::new(),
            },
            IntentError::type_error(""),
            IntentError::contract_violation(String::new()),
            IntentError::runtime_error(""),
            IntentError::UndefinedVariable {
                name: String::new(),
                suggestion: None,
                line: 0,
            },
            IntentError::UndefinedFunction {
                name: String::new(),
                suggestion: None,
                hint: None,
                line: 0,
            },
            IntentError::ArityMismatch {
                name: String::new(),
                expected: "0".to_string(),
                got: 0,
                line: 0,
            },
            IntentError::DivisionByZero { line: 0 },
            IntentError::IndexOutOfBounds {
                index: 0,
                length: 0,
            },
            IntentError::InvalidOperation(String::new()),
            IntentError::RequiresApproval(String::new()),
        ];

        let mut codes: Vec<&str> = errors.iter().map(|e| e.error_code()).collect();
        let total = codes.len();
        codes.sort();
        codes.dedup();
        assert_eq!(codes.len(), total, "Error codes must be unique");
    }

    #[test]
    fn test_error_line_info() {
        let e = IntentError::ParserError {
            line: 42,
            column: 10,
            message: "test".into(),
        };
        assert_eq!(e.line(), Some(42));
        assert_eq!(e.column(), Some(10));

        let e = IntentError::runtime_error("test");
        assert_eq!(e.line(), None);
        assert_eq!(e.column(), None);
    }

    #[test]
    fn test_error_suggestion() {
        let e = IntentError::UndefinedVariable {
            name: "usres".into(),
            suggestion: Some("users".into()),
            line: 0,
        };
        assert_eq!(e.suggestion(), Some("users"));

        let e = IntentError::UndefinedVariable {
            name: "xyz".into(),
            suggestion: None,
            line: 0,
        };
        assert_eq!(e.suggestion(), None);
    }

    #[test]
    fn test_type_context_display() {
        let ctx = TypeContext::new("Int", "String").with_hint("Use int(x) to convert");
        let display = format!("{}", ctx);
        assert!(display.contains("expected Int"));
        assert!(display.contains("found String"));
        assert!(display.contains("hint: Use int(x) to convert"));
    }

    #[test]
    fn test_rich_display_with_context() {
        let err = IntentError::type_error_with_context(
            "Cannot index Array with String",
            TypeContext::new("Int", "String").with_hint("Use int(key) or iterate instead"),
        );
        let display = err.rich_display();
        assert!(display.contains("Cannot index Array with String"));
        assert!(display.contains("expected: Int"));
        assert!(display.contains("found: String"));
        assert!(display.contains("hint: Use int(key)"));
    }

    #[test]
    fn test_rich_display_without_context() {
        let err = IntentError::type_error("simple error");
        let display = err.rich_display();
        assert_eq!(display, "Type error: simple error");
    }

    #[test]
    fn test_type_error_constructors() {
        let e1 = IntentError::type_error("msg");
        assert!(matches!(e1, IntentError::TypeError { context: None, .. }));

        let e2 = IntentError::type_error_with_context("msg", TypeContext::new("Int", "String"));
        assert!(matches!(
            e2,
            IntentError::TypeError {
                context: Some(_),
                ..
            }
        ));
        assert!(e2.type_context().is_some());
    }
}
