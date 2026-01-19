//! NTNT (Intent) Programming Language
//!
//! A programming language designed for AI-driven development with
//! first-class contracts, typed effects, and human-in-the-loop governance.

pub mod ast;
pub mod contracts;
pub mod error;
pub mod ial;
pub mod intent;
pub mod interpreter;
pub mod lexer;
pub mod parser;
pub mod stdlib;
pub mod types;

pub use error::{IntentError, Result};
