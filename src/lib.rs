//! NTNT (Intent) Programming Language
//!
//! A programming language designed for AI-driven development with
//! first-class contracts, typed effects, and human-in-the-loop governance.

pub mod lexer;
pub mod parser;
pub mod ast;
pub mod interpreter;
pub mod types;
pub mod contracts;
pub mod error;
pub mod stdlib;

pub use error::{IntentError, Result};
