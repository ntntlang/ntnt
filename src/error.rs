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

    #[error("Parser error at line {line}: {message}")]
    ParserError { line: usize, message: String },

    #[error("Type error: {0}")]
    TypeError(String),

    #[error("Contract violation: {0}")]
    ContractViolation(String),

    #[error("Runtime error: {0}")]
    RuntimeError(String),

    #[error("Undefined variable: {0}")]
    UndefinedVariable(String),

    #[error("Undefined function: {0}")]
    UndefinedFunction(String),

    #[error("Arity mismatch: expected {expected} arguments, got {got}")]
    ArityMismatch { expected: usize, got: usize },

    #[error("Division by zero")]
    DivisionByZero,

    #[error("Index out of bounds: index {index}, length {length}")]
    IndexOutOfBounds { index: i64, length: usize },

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Requires human approval: {0}")]
    RequiresApproval(String),
}
