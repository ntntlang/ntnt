//! # Markdown Module (`std/markdown`)
//!
//! Convert Markdown to HTML using pulldown-cmark.
//!
//! ```ntnt
//! import { to_html, to_html_safe } from "std/markdown"
//!
//! let html = to_html("# Hello\n\nThis is **bold**.")
//! let safe = to_html_safe("# Hello\n\n<script>alert('xss')</script>")
//! ```

use crate::error::IntentError;
use crate::interpreter::Value;
use pulldown_cmark::{html, Options, Parser};
use std::collections::HashMap;

/// Initialize the std/markdown module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt to_html
    // @module std/markdown
    // @signature to_html(markdown: String) -> String
    // Convert a Markdown string to HTML. Supports GitHub Flavored Markdown:
    // tables, strikethrough, task lists, footnotes, heading attributes.
    // Does NOT sanitize HTML — embedded HTML tags pass through as-is.
    // Use to_html_safe() if the input is untrusted.
    module.insert(
        "to_html".to_string(),
        Value::NativeFunction {
            name: "to_html".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(md) => Ok(markdown_to_html(md, false)),
                _ => Err(IntentError::type_error(
                    "to_html() requires a String argument".to_string(),
                )),
            },
        },
    );

    // @ntnt to_html_safe
    // @module std/markdown
    // @signature to_html_safe(markdown: String) -> String
    // Convert a Markdown string to HTML with embedded HTML tags stripped.
    // Use this when rendering user-supplied or untrusted Markdown content.
    module.insert(
        "to_html_safe".to_string(),
        Value::NativeFunction {
            name: "to_html_safe".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(md) => Ok(markdown_to_html(md, true)),
                _ => Err(IntentError::type_error(
                    "to_html_safe() requires a String argument".to_string(),
                )),
            },
        },
    );

    module
}

/// Convert markdown to HTML
fn markdown_to_html(input: &str, strip_raw_html: bool) -> Value {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);

    if strip_raw_html {
        // Filter out raw HTML events for safe rendering
        let parser = Parser::new_ext(input, options);
        let filtered = parser.filter(|event| {
            !matches!(
                event,
                pulldown_cmark::Event::Html(_) | pulldown_cmark::Event::InlineHtml(_)
            )
        });
        let mut html_output = String::new();
        html::push_html(&mut html_output, filtered);
        Value::String(html_output)
    } else {
        let parser = Parser::new_ext(input, options);
        let mut html_output = String::new();
        html::push_html(&mut html_output, parser);
        Value::String(html_output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_markdown() {
        match markdown_to_html("# Hello", false) {
            Value::String(s) => assert!(s.contains("<h1>Hello</h1>")),
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_bold_italic() {
        match markdown_to_html("**bold** and *italic*", false) {
            Value::String(s) => {
                assert!(s.contains("<strong>bold</strong>"));
                assert!(s.contains("<em>italic</em>"));
            }
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_code_block() {
        match markdown_to_html("```rust\nfn main() {}\n```", false) {
            Value::String(s) => {
                assert!(s.contains("<code"));
                assert!(s.contains("fn main()"));
            }
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_table() {
        let md = "| A | B |\n|---|---|\n| 1 | 2 |";
        match markdown_to_html(md, false) {
            Value::String(s) => {
                assert!(s.contains("<table>"));
                assert!(s.contains("<th>A</th>"));
                assert!(s.contains("<td>1</td>"));
            }
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_links() {
        match markdown_to_html("[click](https://example.com)", false) {
            Value::String(s) => assert!(s.contains("<a href=\"https://example.com\">click</a>")),
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_safe_strips_html() {
        match markdown_to_html("<script>alert('xss')</script>", true) {
            Value::String(s) => assert!(!s.contains("<script>")),
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_safe_keeps_markdown() {
        match markdown_to_html("# Title\n\n**bold** text", true) {
            Value::String(s) => {
                assert!(s.contains("<h1>Title</h1>"));
                assert!(s.contains("<strong>bold</strong>"));
            }
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_strikethrough() {
        match markdown_to_html("~~deleted~~", false) {
            Value::String(s) => assert!(s.contains("<del>deleted</del>")),
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_task_list() {
        match markdown_to_html("- [x] Done\n- [ ] Todo", false) {
            Value::String(s) => {
                assert!(s.contains("checked"));
                assert!(s.contains("type=\"checkbox\""));
            }
            _ => panic!("Expected string"),
        }
    }

    #[test]
    fn test_empty_string() {
        match markdown_to_html("", false) {
            Value::String(s) => assert_eq!(s, ""),
            _ => panic!("Expected string"),
        }
    }
}
