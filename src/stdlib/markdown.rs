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
    // Does NOT sanitize HTML — embedded HTML tags, heading attributes and link or
    // image URLs of any scheme (including javascript:) pass through as-is.
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
    // Convert user-supplied or untrusted Markdown to HTML without raw HTML, heading
    // attributes or link and image URLs of unsafe schemes.
    //
    // Embedded HTML tags, HTML blocks and comments are dropped. Heading attribute
    // syntax (`{#id .class key=value}`) stays literal text. Every href and src is
    // relative (including `//host/path`, `?query` and `#fragment`) or uses http,
    // https or mailto: a link with any other scheme (javascript:, data:, vbscript:,
    // file:, ...) renders as its plain text, and such an image as its alt text.
    // Schemes are compared the way browsers read them: after entity decoding,
    // ignoring case, tabs, line breaks and leading or trailing spaces and control
    // characters. Everything else matches to_html(). The guarantee covers the
    // returned HTML only; escape anything you add around it.
    // @param markdown Markdown source, possibly user-supplied.
    // @returns HTML without raw HTML, heading attributes or unsafe link and image URLs.
    // @see_also to_html, parse_blocks
    // @tags #text #markdown #security
    // @example to_html_safe("[x](javascript:alert(1)) <b>hi</b>") => "<p>x hi</p>\n" ~ "Unsafe links keep only their text"
    // @example to_html_safe("[docs](/guide) ![logo](https://example.com/logo.png)") => "<p><a href=\"/guide\">docs</a> <img src=\"https://example.com/logo.png\" alt=\"logo\" /></p>\n" ~ "Relative and https URLs are kept"
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

/// URL schemes to_html_safe() keeps in link and image destinations. Destinations
/// without a scheme (relative paths, `//host/path`, `?query`, `#fragment`) are kept too.
const SAFE_URL_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// Convert markdown to HTML; `safe` selects the to_html_safe() rendering.
fn markdown_to_html(input: &str, safe: bool) -> Value {
    let mut html_output = String::new();
    if safe {
        // `{onclick=...}` after a heading would become an attribute; keep it as text.
        let mut options = markdown_options();
        options.remove(Options::ENABLE_HEADING_ATTRIBUTES);
        html::push_html(
            &mut html_output,
            safe_events(Parser::new_ext(input, options)),
        );
    } else {
        html::push_html(&mut html_output, Parser::new_ext(input, markdown_options()));
    }
    Value::String(html_output)
}

/// The to_html_safe() event stream: raw HTML is dropped, and a link or image whose
/// destination is not allowed loses its start and end tags, so a link renders as
/// its text and an image as its alt text.
fn safe_events<'a>(events: impl Iterator<Item = Event<'a>>) -> impl Iterator<Item = Event<'a>> {
    // Whether each open link or image kept its start tag, so its end tag matches.
    let mut kept = Vec::new();
    events.filter(move |event| match event {
        Event::Html(_) | Event::InlineHtml(_) => false,
        Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => {
            let keep = is_allowed_url(dest_url);
            kept.push(keep);
            keep
        }
        Event::End(TagEnd::Link | TagEnd::Image) => kept.pop().unwrap_or(true),
        _ => true,
    })
}

/// Whether to_html_safe() may emit `url` as an href or src.
fn is_allowed_url(url: &str) -> bool {
    url_scheme(url).is_none_or(|scheme| SAFE_URL_SCHEMES.contains(&scheme.as_str()))
}

/// The lowercase scheme a browser's URL parser reads from `url`, or `None` for a
/// reference without one. Like the WHATWG URL parser, ignores leading and trailing
/// C0 controls and spaces and every tab, line feed and carriage return, so
/// `" JAVA\tSCRIPT:"` is `javascript`. pulldown-cmark has already decoded entity
/// references such as `jav&#x61;script:` in `url`.
fn url_scheme(url: &str) -> Option<String> {
    let mut chars = url
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'));
    let first = chars.next().filter(char::is_ascii_alphabetic)?;
    let mut scheme = String::from(first.to_ascii_lowercase());
    for c in chars {
        match c {
            ':' => return Some(scheme),
            c if c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.') => {
                scheme.push(c.to_ascii_lowercase())
            }
            _ => return None,
        }
    }
    None
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
    fn wikilinks_with_empty_labels_have_bounded_event_growth() {
        // RUSTSEC-2026-0414: 16 empty-label wikilinks yielded 327,676 events
        // before pulldown-cmark 0.13.5. Exercise the opt-in dependency path;
        // NTNT's markdown_options() does not enable wikilinks.
        let input = "[[a|]] ".repeat(16);
        // Stop just beyond the expected count so a regression stays cheap.
        let events = Parser::new_ext(&input, Options::ENABLE_WIKILINKS)
            .take(66)
            .count();
        assert_eq!(events, 65);
    }

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

    fn render(input: &str, safe: bool) -> String {
        match markdown_to_html(input, safe) {
            Value::String(html) => html,
            other => panic!("expected string, got {other:?}"),
        }
    }

    /// (case, Markdown, exact to_html_safe() output). A link with a disallowed
    /// scheme keeps only its text, an image only its alt text, and heading
    /// attribute syntax stays literal text.
    const UNSAFE_PAYLOADS: &[(&str, &str, &str)] = &[
        ("javascript", "[x](javascript:alert(1))", "<p>x</p>\n"),
        ("uppercase", "[x](JAVASCRIPT:alert(1))", "<p>x</p>\n"),
        ("mixed case", "[x](JaVaScRiPt:alert(1))", "<p>x</p>\n"),
        ("tab", "[x](<java\tscript:alert(1)>)", "<p>x</p>\n"),
        ("entity tab", "[x](java&#9;script:alert(1))", "<p>x</p>\n"),
        (
            "entity newline",
            "[x](java&#10;script:alert(1))",
            "<p>x</p>\n",
        ),
        (
            "entity return",
            "[x](java&#13;script:alert(1))",
            "<p>x</p>\n",
        ),
        ("leading space", "[x](< javascript:alert(1)>)", "<p>x</p>\n"),
        (
            "trailing space",
            "[x](<javascript:alert(1) >)",
            "<p>x</p>\n",
        ),
        (
            "leading control",
            "[x](&#1;javascript:alert(1))",
            "<p>x</p>\n",
        ),
        (
            "entity letter",
            "[x](jav&#x61;script:alert(1))",
            "<p>x</p>\n",
        ),
        (
            "entity colon",
            "[x](javascript&colon;alert(1))",
            "<p>x</p>\n",
        ),
        (
            "data",
            "[x](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)",
            "<p>x</p>\n",
        ),
        ("vbscript", "[x](vbscript:msgbox(1))", "<p>x</p>\n"),
        ("title", "[x](javascript:alert(1) \"t\")", "<p>x</p>\n"),
        (
            "formatted text",
            "[**x**](javascript:alert(1))",
            "<p><strong>x</strong></p>\n",
        ),
        (
            "autolink",
            "<javascript:alert(1)>",
            "<p>javascript:alert(1)</p>\n",
        ),
        (
            "reference",
            "[x][r]\n\n[r]: javascript:alert(1)",
            "<p>x</p>\n",
        ),
        (
            "collapsed reference",
            "[r][]\n\n[r]: javascript:alert(1)",
            "<p>r</p>\n",
        ),
        (
            "shortcut reference",
            "[r]\n\n[r]: javascript:alert(1)",
            "<p>r</p>\n",
        ),
        ("image", "![alt](javascript:alert(1))", "<p>alt</p>\n"),
        (
            "data image",
            "![alt](data:image/svg+xml;base64,PHN2Zz4=)",
            "<p>alt</p>\n",
        ),
        (
            "reference image",
            "![alt][r]\n\n[r]: javascript:alert(1)",
            "<p>alt</p>\n",
        ),
        (
            "safe image in unsafe link",
            "[![ok](https://example.com/a.png)](javascript:alert(1))",
            "<p><img src=\"https://example.com/a.png\" alt=\"ok\" /></p>\n",
        ),
        (
            "unsafe image in safe link",
            "[![ok](javascript:alert(1))](https://example.com)",
            "<p><a href=\"https://example.com\">ok</a></p>\n",
        ),
        (
            "heading event handler",
            "# Title {onclick=alert(1)}",
            "<h1>Title {onclick=alert(1)}</h1>\n",
        ),
        (
            "heading id and class",
            "## Title {#top .big}",
            "<h2>Title {#top .big}</h2>\n",
        ),
    ];

    #[test]
    fn safe_mode_neutralizes_unsafe_urls_and_heading_attributes() {
        let failures = UNSAFE_PAYLOADS
            .iter()
            .filter_map(|(case, input, expected)| {
                let html = render(input, true);
                (html != *expected).then(|| format!("{case}: {input:?} rendered {html:?}"))
            })
            .collect::<Vec<_>>();
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[test]
    fn safe_mode_renders_allowed_destinations_like_trusted_mode() {
        let inputs = [
            "[x](https://example.com/a?b=1#c)",
            "[x](http://example.com)",
            "[x](HTTPS://EXAMPLE.COM)",
            "[x](mailto:someone@example.com)",
            "[x](/docs/page)",
            "[x](docs/page.html)",
            "[x](./notes:2026.md)",
            "[x](//example.com/path)",
            "[x](?page=2)",
            "[x](#section)",
            "[x](<docs/a file.md>)",
            "[x](https://example.com \"Title\")",
            "<https://example.com>",
            "<someone@example.com>",
            "[x][r]\n\n[r]: https://example.com",
            "![alt](https://example.com/a.png \"Title\")",
            "![alt](images/a.png)",
            "![a [b](javascript:alert(1))](https://example.com/i.png)",
            "Text[^1]\n\n[^1]: Footnote.",
            "# Title\n\n- [x] done\n\n| A |\n|---|\n| 1 |",
        ];
        for input in inputs {
            assert_eq!(render(input, true), render(input, false), "input {input:?}");
        }
        assert_eq!(
            render("[x](https://example.com)", true),
            "<p><a href=\"https://example.com\">x</a></p>\n"
        );
        assert_eq!(
            render("<someone@example.com>", true),
            "<p><a href=\"mailto:someone@example.com\">someone@example.com</a></p>\n"
        );
        assert_eq!(
            render("![alt](images/a.png)", true),
            "<p><img src=\"images/a.png\" alt=\"alt\" /></p>\n"
        );
    }

    #[test]
    fn url_scheme_follows_browser_normalization() {
        let cases = [
            ("javascript:alert(1)", Some("javascript")),
            (" \u{1}JaVa\tSc\nRi\rPt:x \u{1f}", Some("javascript")),
            ("HTTPS://example.com", Some("https")),
            ("web+app.v-1:x", Some("web+app.v-1")),
            ("mailto:", Some("mailto")),
            ("/path:x", None),
            ("./a:b", None),
            ("a/b:c", None),
            ("1http:x", None),
            ("#frag:x", None),
            ("?q=a:b", None),
            ("//host/a:b", None),
            ("java script:x", None),
            ("java\u{1}script:x", None),
            ("\u{a0}javascript:x", None),
            ("javascript", None),
            ("", None),
        ];
        for (url, expected) in cases {
            assert_eq!(url_scheme(url).as_deref(), expected, "url {url:?}");
        }
    }

    #[test]
    fn to_html_safe_doc_examples_hold() {
        assert_eq!(
            render("[x](javascript:alert(1)) <b>hi</b>", true),
            "<p>x hi</p>\n"
        );
        assert_eq!(
            render(
                "[docs](/guide) ![logo](https://example.com/logo.png)",
                true
            ),
            "<p><a href=\"/guide\">docs</a> <img src=\"https://example.com/logo.png\" alt=\"logo\" /></p>\n"
        );
    }

    #[test]
    fn trusted_mode_keeps_destinations_and_heading_attributes() {
        assert_eq!(
            render("[x](javascript:alert(1))", false),
            "<p><a href=\"javascript:alert(1)\">x</a></p>\n"
        );
        assert_eq!(
            render("# Title {#top .big}", false),
            "<h1 id=\"top\" class=\"big\">Title</h1>\n"
        );
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
