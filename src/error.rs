//! Error types for the Intent language

use thiserror::Error;

/// Result type alias for Intent operations
pub type Result<T> = std::result::Result<T, IntentError>;

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

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Contract violation: {0}")]
    ContractViolation(String),

    #[error("Runtime error: {0}")]
    RuntimeError(String),

    #[error("Undefined variable: {name}")]
    UndefinedVariable {
        name: String,
        suggestion: Option<String>,
    },

    #[error("Undefined function: {name}")]
    UndefinedFunction {
        name: String,
        suggestion: Option<String>,
    },

    #[error("Arity mismatch: function '{name}' expected {expected} arguments, got {got}")]
    ArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Index out of bounds: index {index}, length {length}")]
    IndexOutOfBounds { index: i64, length: usize },

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Requires human approval: {0}")]
    RequiresApproval(String),
}

impl IntentError {
    /// Return a unique error code for this error variant
    pub fn error_code(&self) -> &'static str {
        match self {
            IntentError::LexerError { .. } => "E001",
            IntentError::ParserError { .. } => "E002",
            IntentError::TypeError(_) => "E003",
            IntentError::ContractViolation(_) => "E004",
            IntentError::RuntimeError(_) => "E005",
            IntentError::UndefinedVariable { .. } => "E006",
            IntentError::UndefinedFunction { .. } => "E007",
            IntentError::ArityMismatch { .. } => "E008",
            IntentError::DivisionByZero => "E009",
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
}

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
            IntentError::TypeError(String::new()),
            IntentError::ContractViolation(String::new()),
            IntentError::RuntimeError(String::new()),
            IntentError::UndefinedVariable {
                name: String::new(),
                suggestion: None,
            },
            IntentError::UndefinedFunction {
                name: String::new(),
                suggestion: None,
            },
            IntentError::ArityMismatch {
                name: String::new(),
                expected: 0,
                got: 0,
            },
            IntentError::DivisionByZero,
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

        let e = IntentError::RuntimeError("test".into());
        assert_eq!(e.line(), None);
        assert_eq!(e.column(), None);
    }

    #[test]
    fn test_error_suggestion() {
        let e = IntentError::UndefinedVariable {
            name: "usres".into(),
            suggestion: Some("users".into()),
        };
        assert_eq!(e.suggestion(), Some("users"));

        let e = IntentError::UndefinedVariable {
            name: "xyz".into(),
            suggestion: None,
        };
        assert_eq!(e.suggestion(), None);
    }
}
