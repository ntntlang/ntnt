//! build.rs — Scans Rust source for `// @ntnt` doc comment blocks,
//! validates coverage against NativeFunction inserts, and generates
//! `doc_data.json` embedded in the binary.

use serde::Serialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Data structures — KEEP IN SYNC with src/main.rs mod docs {}
//
// This struct uses Serialize (build.rs writes JSON).
// The mirror in src/main.rs uses Deserialize (binary reads JSON).
// If you add/remove/rename a field here, update src/main.rs too.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct DocEntry {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    module: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    params: Vec<ParamDoc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    returns: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    examples: Vec<ExampleDoc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    see_also: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    errors: Vec<ErrorDoc>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    gotchas: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    module_description: Option<String>,
    source_file: String,
    source_line: usize,
}

#[derive(Debug, Serialize)]
struct ExampleDoc {
    code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

#[derive(Debug, Serialize)]
struct ParamDoc {
    name: String,
    description: String,
}

#[derive(Debug, Serialize)]
struct ErrorDoc {
    error_type: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<String>,
}

// ---------------------------------------------------------------------------
// Source scanner
// ---------------------------------------------------------------------------

/// Discover source files to scan for `// @ntnt` blocks.
/// Automatically finds all .rs files in src/stdlib/ plus src/interpreter.rs.
fn discover_source_files() -> Vec<String> {
    let mut files: Vec<String> = Vec::new();

    // Scan all .rs files in src/stdlib/
    let stdlib_dir = Path::new("src/stdlib");
    if let Ok(entries) = fs::read_dir(stdlib_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "rs").unwrap_or(false) {
                files.push(path.to_string_lossy().to_string());
            }
        }
    }

    // Always include interpreter.rs (global builtins)
    files.push("src/interpreter.rs".to_string());

    files.sort();
    files
}

fn main() {
    // Windows debug builds have large unoptimized stack frames.
    // Increase the default stack size from 1MB to 8MB to prevent
    // overflow in the tree-walking interpreter's deep call chains.
    #[cfg(target_os = "windows")]
    println!("cargo:rustc-link-arg=/STACK:8388608");

    let source_files = discover_source_files();

    // Re-run when scanned source files change or new files are added
    for file in &source_files {
        println!("cargo:rerun-if-changed={}", file);
    }
    // Re-run when the stdlib directory itself changes (new files added/removed)
    println!("cargo:rerun-if-changed=src/stdlib");

    let mut all_docs: Vec<DocEntry> = Vec::new();
    let mut had_errors = false;

    for file in &source_files {
        let content = match fs::read_to_string(file) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("cargo:warning=build.rs: cannot read {}: {}", file, e);
                continue;
            }
        };

        let docs = extract_ntnt_docs(&content, file);
        let native_fns = extract_native_functions(&content);

        // Only validate coverage if the file has at least one @ntnt block
        if !docs.is_empty() {
            if !validate_coverage(&docs, &native_fns, file) {
                had_errors = true;
            }
        }

        all_docs.extend(docs);
    }

    if had_errors {
        std::process::exit(1);
    }

    // Write doc_data.json to OUT_DIR
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let dest = Path::new(&out_dir).join("doc_data.json");
    let json = serde_json::to_string(&all_docs).expect("Failed to serialize doc data");
    fs::write(&dest, json).expect("Failed to write doc_data.json");

    eprintln!(
        "cargo:warning=build.rs: extracted {} doc entries from {} files",
        all_docs.len(),
        source_files.len()
    );
}

// ---------------------------------------------------------------------------
// Parsing `// @ntnt` blocks
// ---------------------------------------------------------------------------

fn extract_ntnt_docs(content: &str, source_file: &str) -> Vec<DocEntry> {
    let lines: Vec<&str> = content.lines().collect();
    let mut entries: Vec<DocEntry> = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        // Look for `// @ntnt <name>`
        if let Some(rest) = trimmed.strip_prefix("// @ntnt ") {
            let name = rest.trim().to_string();
            let block_start = i + 1; // line number (1-indexed)

            // Collect all consecutive `//` lines after the header
            let mut block_lines: Vec<&str> = Vec::new();
            i += 1;
            while i < lines.len() {
                let t = lines[i].trim();
                if t.starts_with("//") && !t.starts_with("//!") {
                    // Strip the `// ` prefix (or just `//` for empty lines)
                    let line_content = t
                        .strip_prefix("// ")
                        .unwrap_or_else(|| t.strip_prefix("//").unwrap_or(""));
                    block_lines.push(line_content);
                    i += 1;
                } else {
                    break;
                }
            }

            let entry = parse_doc_block(name, &block_lines, source_file, block_start);
            entries.push(entry);
        } else {
            i += 1;
        }
    }

    entries
}

fn parse_doc_block(
    name: String,
    lines: &[&str],
    source_file: &str,
    source_line: usize,
) -> DocEntry {
    let mut module: Option<String> = None;
    let mut module_description: Option<String> = None;
    let mut signature: Option<String> = None;
    let mut summary_lines: Vec<String> = Vec::new();
    let mut desc_lines: Vec<String> = Vec::new();
    let mut params: Vec<ParamDoc> = Vec::new();
    let mut returns: Option<String> = None;
    let mut examples: Vec<ExampleDoc> = Vec::new();
    let mut see_also: Vec<String> = Vec::new();
    let mut since: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut errors: Vec<ErrorDoc> = Vec::new();
    let mut gotchas: Vec<String> = Vec::new();

    let mut found_summary = false;
    let mut in_description = false;

    // Multi-line example state
    let mut in_multiline_example = false;
    let mut ml_example_lines: Vec<String> = Vec::new();
    let mut ml_example_desc: Option<String> = None;
    let mut ml_example_expected: Option<String> = None;

    // Helper closure to flush a multi-line example into the examples vec
    macro_rules! flush_multiline_example {
        () => {
            if in_multiline_example && !ml_example_lines.is_empty() {
                examples.push(ExampleDoc {
                    code: ml_example_lines.join("\n"),
                    expected: ml_example_expected.take(),
                    description: ml_example_desc.take(),
                });
                ml_example_lines.clear();
            }
            in_multiline_example = false;
            ml_example_desc = None;
            ml_example_expected = None;
        };
    }

    for line in lines {
        let line = *line;

        // Multi-line example: collect indented continuation lines
        if in_multiline_example {
            if let Some(val) = line.strip_prefix("@expected ") {
                ml_example_expected = Some(val.trim().to_string());
                continue;
            }
            if line.starts_with("  ") {
                ml_example_lines.push(line[2..].to_string());
                continue;
            }
            // Non-indented, non-@expected line ends the multi-line block
            flush_multiline_example!();
            // Fall through to process this line normally
        }

        // @field directives
        if let Some(val) = line.strip_prefix("@module ") {
            module = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("@module_description ") {
            module_description = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("@signature ") {
            let sig_line = val.trim().to_string();
            signature = Some(match signature {
                Some(existing) => format!("{}\n{}", existing, sig_line),
                None => sig_line,
            });
        } else if let Some(val) = line.strip_prefix("@param ") {
            // @param name description
            let val = val.trim();
            if let Some(space_idx) = val.find(' ') {
                params.push(ParamDoc {
                    name: val[..space_idx].to_string(),
                    description: val[space_idx + 1..].trim().to_string(),
                });
            }
        } else if let Some(val) = line.strip_prefix("@returns ") {
            returns = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("@see_also ") {
            see_also = val.split(',').map(|s| s.trim().to_string()).collect();
        } else if let Some(val) = line.strip_prefix("@since ") {
            since = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("@tags ") {
            tags = val
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        } else if line == "@example" {
            // Bare @example — start multi-line mode
            in_multiline_example = true;
        } else if let Some(val) = line.strip_prefix("@example ") {
            let val = val.trim();
            if val.starts_with("~ \"") {
                // @example ~ "description" — multi-line with description
                if let Some(end) = val[3..].rfind('"') {
                    ml_example_desc = Some(val[3..3 + end].to_string());
                }
                in_multiline_example = true;
            } else {
                // Single-line example (existing behavior)
                examples.push(parse_example(val));
            }
        } else if let Some(val) = line.strip_prefix("@error ") {
            errors.push(parse_error(val.trim()));
        } else if let Some(val) = line.strip_prefix("@gotcha ") {
            gotchas.push(val.trim().to_string());
        } else if line.starts_with('@') {
            // Unknown @ directive — skip with a warning
            eprintln!(
                "cargo:warning=build.rs: unknown directive '{}' in @ntnt {} ({})",
                line, name, source_file
            );
        } else if !found_summary {
            // First non-@ line(s) before an empty line = summary
            if line.is_empty() {
                if !summary_lines.is_empty() {
                    found_summary = true;
                    in_description = true;
                }
            } else {
                summary_lines.push(line.to_string());
            }
        } else if in_description {
            desc_lines.push(line.to_string());
        }
    }

    // Flush any trailing multi-line example at end of block
    if in_multiline_example && !ml_example_lines.is_empty() {
        examples.push(ExampleDoc {
            code: ml_example_lines.join("\n"),
            expected: ml_example_expected,
            description: ml_example_desc,
        });
    }

    // If we never hit an empty line, summary_lines is everything
    if !found_summary && !summary_lines.is_empty() {
        // summary_lines collected — no empty line separator needed
    }

    let summary = summary_lines.join(" ");
    let description = if desc_lines.is_empty() {
        None
    } else {
        // Trim trailing empty lines
        while desc_lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            desc_lines.pop();
        }
        if desc_lines.is_empty() {
            None
        } else {
            // Join consecutive non-empty lines into paragraphs with spaces;
            // empty lines become paragraph separators (\n\n).
            let mut paragraphs: Vec<String> = Vec::new();
            let mut current: Vec<String> = Vec::new();
            for line in &desc_lines {
                if line.is_empty() {
                    if !current.is_empty() {
                        paragraphs.push(current.join(" "));
                        current.clear();
                    }
                } else {
                    current.push(line.clone());
                }
            }
            if !current.is_empty() {
                paragraphs.push(current.join(" "));
            }
            if paragraphs.is_empty() {
                None
            } else {
                Some(paragraphs.join("\n\n"))
            }
        }
    };

    if summary.is_empty() {
        eprintln!(
            "cargo:warning=build.rs: @ntnt {} in {} has no summary line",
            name, source_file
        );
    }

    DocEntry {
        name,
        module,
        module_description,
        signature,
        summary,
        description,
        params,
        returns,
        examples,
        see_also,
        since,
        tags,
        errors,
        gotchas,
        source_file: source_file.to_string(),
        source_line,
    }
}

/// Parse `@example <code> => <expected> ~ "<description>"`
fn parse_example(raw: &str) -> ExampleDoc {
    let mut code = raw.to_string();
    let mut expected: Option<String> = None;
    let mut description: Option<String> = None;

    // Extract description: ` ~ "..."`
    if let Some(tilde_idx) = raw.rfind(" ~ \"") {
        let desc_part = &raw[tilde_idx + 4..];
        if let Some(end_quote) = desc_part.rfind('"') {
            description = Some(desc_part[..end_quote].to_string());
            code = raw[..tilde_idx].to_string();
        }
    }

    // Extract expected: ` => ...` (rfind so arrows in code like match don't break parsing)
    if let Some(arrow_idx) = code.rfind(" => ") {
        expected = Some(code[arrow_idx + 4..].trim().to_string());
        code = code[..arrow_idx].trim().to_string();
    }

    ExampleDoc {
        code,
        expected,
        description,
    }
}

/// Parse `@error Type ~ "message" fix: "fix text"`
fn parse_error(raw: &str) -> ErrorDoc {
    let mut error_type = raw.to_string();
    let mut message = String::new();
    let mut fix: Option<String> = None;

    // Extract error type and message: `Type ~ "message"`
    if let Some(tilde_idx) = raw.find(" ~ \"") {
        error_type = raw[..tilde_idx].trim().to_string();
        let rest = &raw[tilde_idx + 4..];

        // Find the closing quote for message
        if let Some(quote_end) = rest.find('"') {
            message = rest[..quote_end].to_string();
            let after_msg = &rest[quote_end + 1..];

            // Extract fix: ` fix: "fix text"`
            if let Some(fix_idx) = after_msg.find("fix: \"") {
                let fix_start = &after_msg[fix_idx + 6..];
                if let Some(fix_end) = fix_start.rfind('"') {
                    fix = Some(fix_start[..fix_end].to_string());
                }
            }
        }
    }

    ErrorDoc {
        error_type,
        message,
        fix,
    }
}

// ---------------------------------------------------------------------------
// Detecting NativeFunction inserts
// ---------------------------------------------------------------------------

/// Extracts function names from `module.insert("NAME"` and `.define("NAME"` patterns.
/// Handles both multi-line and single-line registration styles.
fn extract_native_functions(content: &str) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    let mut names: Vec<String> = Vec::new();

    for i in 0..lines.len() {
        let trimmed = lines[i].trim();

        // Pattern 1 (multi-line): `module.insert(` or `....define(` on its own line,
        // followed by `"NAME".to_string(),` then `Value::NativeFunction {`
        if trimmed == "module.insert(" || trimmed.ends_with(".define(") {
            if i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if let Some(name) = extract_quoted_string(next) {
                    if i + 2 < lines.len() {
                        let third = lines[i + 2].trim();
                        if third.contains("Value::NativeFunction") {
                            names.push(name);
                        }
                    }
                }
            }
            continue;
        }

        // Pattern 2 (single-line): `module.insert("NAME".to_string(), Value::NativeFunction {`
        if (trimmed.starts_with("module.insert(\"") || trimmed.contains(".define(\""))
            && trimmed.contains("Value::NativeFunction")
        {
            if let Some(name) = extract_quoted_string(trimmed) {
                names.push(name);
            }
        }
    }

    names
}

/// Extracts the first quoted string from a line like `"split".to_string(),`
fn extract_quoted_string(line: &str) -> Option<String> {
    let start = line.find('"')?;
    let rest = &line[start + 1..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

// ---------------------------------------------------------------------------
// Coverage validation
// ---------------------------------------------------------------------------

/// Returns true if coverage is complete, false if errors were found.
fn validate_coverage(docs: &[DocEntry], native_fns: &[String], source_file: &str) -> bool {
    let doc_names: HashMap<&str, &DocEntry> = docs.iter().map(|d| (d.name.as_str(), d)).collect();
    let fn_set: std::collections::HashSet<&str> = native_fns.iter().map(|s| s.as_str()).collect();

    let mut ok = true;

    // Check for undocumented NativeFunctions
    for name in native_fns {
        if !doc_names.contains_key(name.as_str()) {
            eprintln!(
                "cargo:warning=build.rs: UNDOCUMENTED function '{}' in {} — add a // @ntnt {} block above its module.insert()",
                name, source_file, name
            );
            ok = false;
        }
    }

    // Check for orphaned doc blocks (stale docs with no matching function)
    for doc in docs {
        if !fn_set.contains(doc.name.as_str()) {
            eprintln!(
                "cargo:warning=build.rs: ORPHANED doc block @ntnt '{}' in {} — no matching NativeFunction insert found. Remove or rename the @ntnt block.",
                doc.name, source_file
            );
            ok = false;
        }
    }

    if !ok {
        eprintln!(
            "cargo:warning=build.rs: {} has documentation errors. Build will fail.",
            source_file
        );
    }

    ok
}
