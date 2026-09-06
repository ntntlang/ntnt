//! # Markdown Module (`std/markdown`)
//!
//! Convert Markdown to HTML using pulldown-cmark.
//!
//! ```ntnt
//! import { parse_blocks, to_html, to_html_safe } from "std/markdown"
//!
//! let html = to_html("# Hello\n\nThis is **bold**.")
//! let safe = to_html_safe("# Hello\n\n<script>alert('xss')</script>")
//! ```

use crate::error::IntentError;
use crate::interpreter::Value;
use pulldown_cmark::{html, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::collections::HashMap;

#[derive(Debug)]
struct MarkdownBlock {
    kind: String,
    start: usize,
    end: usize,
    source: String,
    text: String,
    meta: HashMap<String, Value>,
}

struct PendingBlock {
    kind: String,
    start: usize,
    end: usize,
    text: String,
    meta: HashMap<String, Value>,
    depth: usize,
}

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

    // @ntnt parse_blocks
    // @module std/markdown
    // @signature parse_blocks(markdown: String) -> Array<Map>
    // Parse Markdown into ordered, non-overlapping top-level blocks.
    //
    // Each block contains kind, inclusive start and exclusive end UTF-8 byte offsets,
    // the exact source slice, plain text, and block-specific metadata. Bytes between
    // blocks remain only in the original input so callers can replace one range without
    // rewriting unrelated whitespace or formatting.
    // @param markdown Markdown source to parse.
    // @returns Ordered maps with kind, start, end, source, text, and meta fields.
    // @see_also to_html, to_html_safe
    // @since v0.5.3
    // @tags #text #markdown
    // @example parse_blocks("# Hello\n\nWorld") => [{kind: "heading", start: 0, end: 7, source: "# Hello", text: "Hello", meta: {level: 1}}, ...] ~ "Retain source ranges"
    // @error TypeError ~ "parse_blocks() requires a String argument" fix: "Pass Markdown source as a String"
    module.insert(
        "parse_blocks".to_string(),
        Value::NativeFunction {
            name: "parse_blocks".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(markdown) => Ok(markdown_blocks_to_value(markdown)),
                _ => Err(IntentError::type_error(
                    "parse_blocks() requires a String argument".to_string(),
                )),
            },
        },
    );

    // @ntnt replace_source_range
    // @module std/markdown
    // @signature replace_source_range(markdown: String, start: Int, end: Int, replacement: String) -> String
    // Replace an inclusive-start, exclusive-end UTF-8 byte range in Markdown source.
    // This pairs with parse_blocks() offsets while preserving every byte outside the range.
    // @param markdown Original Markdown source.
    // @param start Inclusive UTF-8 byte offset.
    // @param end Exclusive UTF-8 byte offset.
    // @param replacement Replacement source text.
    // @returns Markdown with exactly the requested source range replaced.
    // @see_also parse_blocks
    // @since v0.5.3
    // @tags #text #markdown
    // @example replace_source_range("# Chaptér\n\nOld", 12, 15, "New") => "# Chaptér\n\nNew" ~ "Splice a parsed source range"
    // @error RuntimeError ~ "Invalid Markdown source range" fix: "Use UTF-8 byte offsets returned by parse_blocks()"
    module.insert(
        "replace_source_range".to_string(),
        Value::NativeFunction {
            name: "replace_source_range".to_string(),
            arity: 4,
            max_arity: 4,
            requires: None,
            func: |args| match (&args[0], &args[1], &args[2], &args[3]) {
                (
                    Value::String(markdown),
                    Value::Int(start),
                    Value::Int(end),
                    Value::String(replacement),
                ) => {
                    let start = usize::try_from(*start).map_err(|_| {
                        IntentError::runtime_error("Invalid Markdown source range".to_string())
                    })?;
                    let end = usize::try_from(*end).map_err(|_| {
                        IntentError::runtime_error("Invalid Markdown source range".to_string())
                    })?;
                    replace_markdown_source_range(markdown, start, end, replacement)
                        .map(Value::String)
                }
                _ => Err(IntentError::type_error(
                    "replace_source_range() requires string, int, int, string".to_string(),
                )),
            },
        },
    );

    module
}

fn replace_markdown_source_range(
    markdown: &str,
    start: usize,
    end: usize,
    replacement: &str,
) -> std::result::Result<String, IntentError> {
    if start > end
        || end > markdown.len()
        || !markdown.is_char_boundary(start)
        || !markdown.is_char_boundary(end)
    {
        return Err(IntentError::runtime_error(
            "Invalid Markdown source range".to_string(),
        ));
    }

    let mut edited = String::with_capacity(markdown.len() - (end - start) + replacement.len());
    edited.push_str(&markdown[..start]);
    edited.push_str(replacement);
    edited.push_str(&markdown[end..]);
    Ok(edited)
}

/// Convert markdown to HTML
fn markdown_to_html(input: &str, strip_raw_html: bool) -> Value {
    let options = markdown_options();

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

fn markdown_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_HEADING_ATTRIBUTES);
    options
}

fn block_kind_and_meta(tag: &Tag<'_>) -> Option<(String, HashMap<String, Value>)> {
    let mut meta = HashMap::new();
    let kind = match tag {
        Tag::Paragraph => "paragraph",
        Tag::Heading { level, .. } => {
            let level = match level {
                HeadingLevel::H1 => 1,
                HeadingLevel::H2 => 2,
                HeadingLevel::H3 => 3,
                HeadingLevel::H4 => 4,
                HeadingLevel::H5 => 5,
                HeadingLevel::H6 => 6,
            };
            meta.insert("level".to_string(), Value::Int(level));
            "heading"
        }
        Tag::BlockQuote(_) => "blockquote",
        Tag::CodeBlock(code_kind) => {
            if let CodeBlockKind::Fenced(language) = code_kind {
                meta.insert("language".to_string(), Value::String(language.to_string()));
            }
            "code_block"
        }
        Tag::HtmlBlock => "html",
        Tag::List(start) => {
            meta.insert("ordered".to_string(), Value::Bool(start.is_some()));
            if let Some(start) = start {
                meta.insert("start".to_string(), Value::Int(*start as i64));
            }
            "list"
        }
        Tag::FootnoteDefinition(label) => {
            meta.insert("label".to_string(), Value::String(label.to_string()));
            "footnote_definition"
        }
        Tag::Table(_) => "table",
        Tag::MetadataBlock(_) => "unknown",
        Tag::DefinitionList => "unknown",
        _ => return None,
    };
    Some((kind.to_string(), meta))
}

fn push_separator(text: &mut String, separator: char) {
    if !text.is_empty() && !text.ends_with(separator) {
        text.push(separator);
    }
}

fn record_inline_destination(meta: &mut HashMap<String, Value>, tag: &Tag<'_>) {
    let (kind, destination, title) = match tag {
        Tag::Link {
            dest_url, title, ..
        } => ("link", dest_url, title),
        Tag::Image {
            dest_url, title, ..
        } => ("image", dest_url, title),
        _ => return,
    };
    let entry = Value::Map(HashMap::from([
        ("kind".to_string(), Value::String(kind.to_string())),
        (
            "destination".to_string(),
            Value::String(destination.to_string()),
        ),
        ("title".to_string(), Value::String(title.to_string())),
    ]));
    match meta.entry("links".to_string()) {
        std::collections::hash_map::Entry::Vacant(vacant) => {
            vacant.insert(Value::Array(vec![entry]));
        }
        std::collections::hash_map::Entry::Occupied(mut occupied) => {
            if let Value::Array(links) = occupied.get_mut() {
                links.push(entry);
            }
        }
    }
}

fn finish_block(input: &str, pending: PendingBlock) -> MarkdownBlock {
    let end = if input[..pending.end].ends_with("\r\n") {
        pending.end - 2
    } else if input[..pending.end].ends_with('\n') {
        pending.end - 1
    } else {
        pending.end
    };
    let text = if pending.kind == "code_block" {
        pending.text
    } else {
        pending.text.trim_end_matches(['\n', '\t']).to_string()
    };
    MarkdownBlock {
        kind: pending.kind,
        start: pending.start,
        end,
        source: input[pending.start..end].to_string(),
        text,
        meta: pending.meta,
    }
}

fn parse_markdown_blocks(input: &str) -> Vec<MarkdownBlock> {
    let mut blocks = Vec::new();
    let mut pending: Option<PendingBlock> = None;

    for (event, range) in Parser::new_ext(input, markdown_options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                if let Some(active) = pending.as_mut() {
                    record_inline_destination(&mut active.meta, &tag);
                    active.depth += 1;
                    active.end = active.end.max(range.end);
                } else if let Some((kind, meta)) = block_kind_and_meta(&tag) {
                    pending = Some(PendingBlock {
                        kind,
                        start: range.start,
                        end: range.end,
                        text: String::new(),
                        meta,
                        depth: 1,
                    });
                }
            }
            Event::End(tag) => {
                if let Some(active) = pending.as_mut() {
                    active.end = active.end.max(range.end);
                    if matches!(tag, TagEnd::Item | TagEnd::TableRow) {
                        push_separator(&mut active.text, '\n');
                    } else if matches!(tag, TagEnd::TableCell) {
                        push_separator(&mut active.text, '\t');
                    }
                    active.depth -= 1;
                    if active.depth == 0 {
                        let completed = pending.take().expect("active block");
                        blocks.push(finish_block(input, completed));
                    }
                }
            }
            Event::Text(value)
            | Event::Code(value)
            | Event::InlineMath(value)
            | Event::DisplayMath(value)
            | Event::Html(value)
            | Event::InlineHtml(value) => {
                if let Some(active) = pending.as_mut() {
                    active.text.push_str(&value);
                    active.end = active.end.max(range.end);
                }
            }
            Event::FootnoteReference(label) => {
                if let Some(active) = pending.as_mut() {
                    active.text.push_str(&label);
                    active.end = active.end.max(range.end);
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some(active) = pending.as_mut() {
                    push_separator(&mut active.text, '\n');
                    active.end = active.end.max(range.end);
                }
            }
            Event::Rule => {
                if let Some(active) = pending.as_mut() {
                    active.end = active.end.max(range.end);
                } else {
                    blocks.push(MarkdownBlock {
                        kind: "thematic_break".to_string(),
                        start: range.start,
                        end: range.end,
                        source: input[range.clone()].to_string(),
                        text: String::new(),
                        meta: HashMap::new(),
                    });
                }
            }
            Event::TaskListMarker(checked) => {
                if let Some(active) = pending.as_mut() {
                    active.text.push_str(if checked { "[x] " } else { "[ ] " });
                    active.end = active.end.max(range.end);
                }
            }
        }
    }

    blocks
}

fn markdown_blocks_to_value(input: &str) -> Value {
    Value::Array(
        parse_markdown_blocks(input)
            .into_iter()
            .map(|block| {
                Value::Map(HashMap::from([
                    ("kind".to_string(), Value::String(block.kind)),
                    ("start".to_string(), Value::Int(block.start as i64)),
                    ("end".to_string(), Value::Int(block.end as i64)),
                    ("source".to_string(), Value::String(block.source)),
                    ("text".to_string(), Value::String(block.text)),
                    ("meta".to_string(), Value::Map(block.meta)),
                ]))
            })
            .collect(),
    )
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

    #[test]
    fn parse_blocks_preserves_unicode_byte_ranges_and_gaps() {
        let input = "# Chaptér\n\nMara *paused*.\n";
        let blocks = parse_markdown_blocks(input);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "heading");
        assert_eq!(&input[blocks[0].start..blocks[0].end], "# Chaptér");
        assert_eq!(blocks[0].source, "# Chaptér");
        assert_eq!(blocks[0].text, "Chaptér");
        assert!(matches!(blocks[0].meta.get("level"), Some(Value::Int(1))));
        assert_eq!(&input[blocks[1].start..blocks[1].end], "Mara *paused*.");
        assert_eq!(blocks[1].source, "Mara *paused*.");
        assert_eq!(blocks[1].text, "Mara paused.");
        assert!(blocks[0].end < blocks[1].start);
    }

    #[test]
    fn replace_source_range_uses_utf8_byte_offsets() {
        let input = "# Chaptér\n\nMara *paused*.\n";
        let blocks = parse_markdown_blocks(input);
        let edited =
            replace_markdown_source_range(input, blocks[1].start, blocks[1].end, "Mara listened.")
                .unwrap();

        assert_eq!(edited, "# Chaptér\n\nMara listened.\n");
        assert!(replace_markdown_source_range(input, 0, 8, "bad").is_err());
    }

    #[test]
    fn parse_blocks_returns_empty_array_for_empty_input() {
        assert!(parse_markdown_blocks("").is_empty());
    }

    #[test]
    fn parse_blocks_reports_code_and_list_metadata() {
        let input = "```rust\nfn main() {}\n```\n\n3. third\n4. fourth\n";
        let blocks = parse_markdown_blocks(input);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "code_block");
        assert!(matches!(
            blocks[0].meta.get("language"),
            Some(Value::String(language)) if language == "rust"
        ));
        assert_eq!(blocks[0].text, "fn main() {}\n");
        assert_eq!(blocks[1].kind, "list");
        assert!(matches!(
            blocks[1].meta.get("ordered"),
            Some(Value::Bool(true))
        ));
        assert!(matches!(blocks[1].meta.get("start"), Some(Value::Int(3))));
        assert_eq!(blocks[1].text, "third\nfourth");
    }

    #[test]
    fn parse_blocks_groups_nested_blocks_without_overlap() {
        let input = "> A **quoted** line.\n\n| A | B |\n|---|---|\n| 1 | 2 |\n";
        let blocks = parse_markdown_blocks(input);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "blockquote");
        assert_eq!(blocks[0].text, "A quoted line.");
        assert_eq!(blocks[1].kind, "table");
        assert!(blocks.windows(2).all(|pair| pair[0].end <= pair[1].start));
        for block in blocks {
            assert_eq!(block.source, input[block.start..block.end]);
        }
    }

    #[test]
    fn parse_blocks_keeps_nested_rules_inside_their_container() {
        let input = "> ---\n\n- before\n  ---\n  after\n";
        let blocks = parse_markdown_blocks(input);

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "blockquote");
        assert_eq!(blocks[0].source, "> ---");
        assert_eq!(blocks[1].kind, "list");
        assert!(blocks.windows(2).all(|pair| pair[0].end <= pair[1].start));
        for block in blocks {
            assert_eq!(block.source, input[block.start..block.end]);
        }
    }

    #[test]
    fn parse_blocks_reports_standalone_and_footnote_kinds() {
        let input = "---\n\n<div>raw</div>\n\n[^note]: Footnote text.\n";
        let blocks = parse_markdown_blocks(input);

        assert_eq!(
            blocks
                .iter()
                .map(|block| block.kind.as_str())
                .collect::<Vec<_>>(),
            ["thematic_break", "html", "footnote_definition"]
        );
        assert!(matches!(
            blocks[2].meta.get("label"),
            Some(Value::String(label)) if label == "note"
        ));
        assert_eq!(blocks[2].text, "Footnote text.");
    }

    #[test]
    fn parse_blocks_records_link_and_image_destinations() {
        let input = "Read [the site](https://example.com \"Docs\") and ![cover](cover.jpg).";
        let blocks = parse_markdown_blocks(input);
        let links = match blocks[0].meta.get("links") {
            Some(Value::Array(links)) => links,
            other => panic!("expected links metadata, got {other:?}"),
        };

        assert_eq!(links.len(), 2);
        let first = match &links[0] {
            Value::Map(link) => link,
            other => panic!("expected link map, got {other:?}"),
        };
        assert!(matches!(
            first.get("kind"),
            Some(Value::String(kind)) if kind == "link"
        ));
        assert!(matches!(
            first.get("destination"),
            Some(Value::String(destination)) if destination == "https://example.com"
        ));
        assert!(matches!(
            first.get("title"),
            Some(Value::String(title)) if title == "Docs"
        ));
        let second = match &links[1] {
            Value::Map(link) => link,
            other => panic!("expected image map, got {other:?}"),
        };
        assert!(matches!(
            second.get("kind"),
            Some(Value::String(kind)) if kind == "image"
        ));
        assert!(matches!(
            second.get("destination"),
            Some(Value::String(destination)) if destination == "cover.jpg"
        ));
    }
}
