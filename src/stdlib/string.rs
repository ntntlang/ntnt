//! std/string module - Comprehensive string manipulation functions
//!
//! Provides Go/JavaScript-quality string handling with case conversion,
//! trimming, searching, validation, and transformation utilities.

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Initialize the std/string module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // ========== Basic Operations ==========

    // split(str, delimiter) -> [String]
    module.insert(
        "split".to_string(),
        Value::NativeFunction {
            name: "split".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(delim)) => {
                    let parts: Vec<Value> = s
                        .split(delim.as_str())
                        .map(|p| Value::String(p.to_string()))
                        .collect();
                    Ok(Value::Array(parts))
                }
                _ => Err(IntentError::TypeError(
                    "split() requires two strings".to_string(),
                )),
            },
        },
    );

    // join(arr, delimiter) -> String
    module.insert(
        "join".to_string(),
        Value::NativeFunction {
            name: "join".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::Array(arr), Value::String(delim)) => {
                    let parts: Vec<String> = arr
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    Ok(Value::String(parts.join(delim)))
                }
                _ => Err(IntentError::TypeError(
                    "join() requires array and string".to_string(),
                )),
            },
        },
    );

    // concat(str1, str2, ...) -> String - Concatenate multiple strings
    // Note: In NTNT you can also use + operator, but this is useful for arrays
    module.insert(
        "concat".to_string(),
        Value::NativeFunction {
            name: "concat".to_string(),
            arity: 2, // At least 2, but handles variadic via array
            func: |args| match &args[0] {
                Value::Array(arr) => {
                    let result: String = arr
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    Ok(Value::String(result))
                }
                Value::String(s1) => match &args[1] {
                    Value::String(s2) => Ok(Value::String(format!("{}{}", s1, s2))),
                    _ => Err(IntentError::TypeError(
                        "concat() requires strings".to_string(),
                    )),
                },
                _ => Err(IntentError::TypeError(
                    "concat() requires strings or array of strings".to_string(),
                )),
            },
        },
    );

    // repeat(str, count) -> String
    module.insert(
        "repeat".to_string(),
        Value::NativeFunction {
            name: "repeat".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::Int(n)) => {
                    if *n < 0 {
                        return Err(IntentError::RuntimeError(
                            "repeat count must be non-negative".to_string(),
                        ));
                    }
                    Ok(Value::String(s.repeat(*n as usize)))
                }
                _ => Err(IntentError::TypeError(
                    "repeat() requires string and int".to_string(),
                )),
            },
        },
    );

    // reverse(str) -> String
    module.insert(
        "reverse".to_string(),
        Value::NativeFunction {
            name: "reverse".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.chars().rev().collect())),
                _ => Err(IntentError::TypeError(
                    "reverse() requires a string".to_string(),
                )),
            },
        },
    );

    // ========== Trimming ==========

    // trim(str) -> String - Trim both ends
    module.insert(
        "trim".to_string(),
        Value::NativeFunction {
            name: "trim".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim().to_string())),
                _ => Err(IntentError::TypeError(
                    "trim() requires a string".to_string(),
                )),
            },
        },
    );

    // trim_left(str) -> String - Trim start/left
    module.insert(
        "trim_left".to_string(),
        Value::NativeFunction {
            name: "trim_left".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim_start().to_string())),
                _ => Err(IntentError::TypeError(
                    "trim_left() requires a string".to_string(),
                )),
            },
        },
    );

    // trim_right(str) -> String - Trim end/right
    module.insert(
        "trim_right".to_string(),
        Value::NativeFunction {
            name: "trim_right".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim_end().to_string())),
                _ => Err(IntentError::TypeError(
                    "trim_right() requires a string".to_string(),
                )),
            },
        },
    );

    // trim_chars(str, chars) -> String - Trim specific characters
    module.insert(
        "trim_chars".to_string(),
        Value::NativeFunction {
            name: "trim_chars".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(chars)) => {
                    let char_set: Vec<char> = chars.chars().collect();
                    Ok(Value::String(
                        s.trim_matches(|c| char_set.contains(&c)).to_string(),
                    ))
                }
                _ => Err(IntentError::TypeError(
                    "trim_chars() requires two strings".to_string(),
                )),
            },
        },
    );

    // ========== Case Conversion ==========

    // to_upper(str) -> String
    module.insert(
        "to_upper".to_string(),
        Value::NativeFunction {
            name: "to_upper".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                _ => Err(IntentError::TypeError(
                    "to_upper() requires a string".to_string(),
                )),
            },
        },
    );

    // to_lower(str) -> String
    module.insert(
        "to_lower".to_string(),
        Value::NativeFunction {
            name: "to_lower".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                _ => Err(IntentError::TypeError(
                    "to_lower() requires a string".to_string(),
                )),
            },
        },
    );

    // capitalize(str) -> String - First letter uppercase
    module.insert(
        "capitalize".to_string(),
        Value::NativeFunction {
            name: "capitalize".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let mut chars = s.chars();
                    match chars.next() {
                        None => Ok(Value::String(String::new())),
                        Some(first) => {
                            let result: String = first
                                .to_uppercase()
                                .chain(chars.flat_map(|c| c.to_lowercase()))
                                .collect();
                            Ok(Value::String(result))
                        }
                    }
                }
                _ => Err(IntentError::TypeError(
                    "capitalize() requires a string".to_string(),
                )),
            },
        },
    );

    // title(str) -> String - Title Case Each Word
    module.insert(
        "title".to_string(),
        Value::NativeFunction {
            name: "title".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let result = s
                        .split_whitespace()
                        .map(|word| {
                            let mut chars = word.chars();
                            match chars.next() {
                                None => String::new(),
                                Some(first) => first
                                    .to_uppercase()
                                    .chain(chars.flat_map(|c| c.to_lowercase()))
                                    .collect(),
                            }
                        })
                        .collect::<Vec<String>>()
                        .join(" ");
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "title() requires a string".to_string(),
                )),
            },
        },
    );

    // to_snake_case(str) -> String - "helloWorld" -> "hello_world"
    module.insert(
        "to_snake_case".to_string(),
        Value::NativeFunction {
            name: "to_snake_case".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let mut result = String::new();
                    for (i, c) in s.chars().enumerate() {
                        if c.is_uppercase() {
                            if i > 0 {
                                result.push('_');
                            }
                            result.extend(c.to_lowercase());
                        } else if c == ' ' || c == '-' {
                            result.push('_');
                        } else {
                            result.push(c);
                        }
                    }
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "to_snake_case() requires a string".to_string(),
                )),
            },
        },
    );

    // to_camel_case(str) -> String - "hello_world" -> "helloWorld"
    module.insert(
        "to_camel_case".to_string(),
        Value::NativeFunction {
            name: "to_camel_case".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let mut result = String::new();
                    let mut capitalize_next = false;
                    for c in s.chars() {
                        if c == '_' || c == '-' || c == ' ' {
                            capitalize_next = true;
                        } else if capitalize_next {
                            result.extend(c.to_uppercase());
                            capitalize_next = false;
                        } else {
                            result.push(c);
                        }
                    }
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "to_camel_case() requires a string".to_string(),
                )),
            },
        },
    );

    // to_pascal_case(str) -> String - "hello_world" -> "HelloWorld"
    module.insert(
        "to_pascal_case".to_string(),
        Value::NativeFunction {
            name: "to_pascal_case".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let mut result = String::new();
                    let mut capitalize_next = true;
                    for c in s.chars() {
                        if c == '_' || c == '-' || c == ' ' {
                            capitalize_next = true;
                        } else if capitalize_next {
                            result.extend(c.to_uppercase());
                            capitalize_next = false;
                        } else {
                            result.push(c);
                        }
                    }
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "to_pascal_case() requires a string".to_string(),
                )),
            },
        },
    );

    // to_kebab_case(str) -> String - "hello_world" -> "hello-world"
    module.insert(
        "to_kebab_case".to_string(),
        Value::NativeFunction {
            name: "to_kebab_case".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let mut result = String::new();
                    for (i, c) in s.chars().enumerate() {
                        if c.is_uppercase() {
                            if i > 0 {
                                result.push('-');
                            }
                            result.extend(c.to_lowercase());
                        } else if c == '_' || c == ' ' {
                            result.push('-');
                        } else {
                            result.push(c);
                        }
                    }
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "to_kebab_case() requires a string".to_string(),
                )),
            },
        },
    );

    // slugify(str) -> String - "Hello World!" -> "hello-world"
    module.insert(
        "slugify".to_string(),
        Value::NativeFunction {
            name: "slugify".to_string(),
            arity: 1,
            func: |args| {
                match &args[0] {
                    Value::String(s) => {
                        let result: String = s
                            .to_lowercase()
                            .chars()
                            .map(|c| {
                                if c.is_alphanumeric() {
                                    c
                                } else if c.is_whitespace() || c == '_' {
                                    '-'
                                } else {
                                    '\0' // Mark for removal
                                }
                            })
                            .filter(|&c| c != '\0')
                            .collect();
                        // Remove consecutive dashes and trim
                        let mut prev_dash = false;
                        let result: String = result
                            .chars()
                            .filter(|&c| {
                                if c == '-' {
                                    if prev_dash {
                                        return false;
                                    }
                                    prev_dash = true;
                                } else {
                                    prev_dash = false;
                                }
                                true
                            })
                            .collect();
                        Ok(Value::String(result.trim_matches('-').to_string()))
                    }
                    _ => Err(IntentError::TypeError(
                        "slugify() requires a string".to_string(),
                    )),
                }
            },
        },
    );

    // ========== Search & Replace ==========

    // contains(str, substr) -> Bool
    module.insert(
        "contains".to_string(),
        Value::NativeFunction {
            name: "contains".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(substr)) => {
                    Ok(Value::Bool(s.contains(substr.as_str())))
                }
                _ => Err(IntentError::TypeError(
                    "contains() requires two strings".to_string(),
                )),
            },
        },
    );

    // starts_with(str, prefix) -> Bool
    module.insert(
        "starts_with".to_string(),
        Value::NativeFunction {
            name: "starts_with".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(prefix)) => {
                    Ok(Value::Bool(s.starts_with(prefix.as_str())))
                }
                _ => Err(IntentError::TypeError(
                    "starts_with() requires two strings".to_string(),
                )),
            },
        },
    );

    // ends_with(str, suffix) -> Bool
    module.insert(
        "ends_with".to_string(),
        Value::NativeFunction {
            name: "ends_with".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(suffix)) => {
                    Ok(Value::Bool(s.ends_with(suffix.as_str())))
                }
                _ => Err(IntentError::TypeError(
                    "ends_with() requires two strings".to_string(),
                )),
            },
        },
    );

    // index_of(str, substr) -> Int (-1 if not found)
    module.insert(
        "index_of".to_string(),
        Value::NativeFunction {
            name: "index_of".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(substr)) => match s.find(substr.as_str()) {
                    Some(idx) => Ok(Value::Int(idx as i64)),
                    None => Ok(Value::Int(-1)),
                },
                _ => Err(IntentError::TypeError(
                    "index_of() requires two strings".to_string(),
                )),
            },
        },
    );

    // last_index_of(str, substr) -> Int (-1 if not found)
    module.insert(
        "last_index_of".to_string(),
        Value::NativeFunction {
            name: "last_index_of".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(substr)) => match s.rfind(substr.as_str()) {
                    Some(idx) => Ok(Value::Int(idx as i64)),
                    None => Ok(Value::Int(-1)),
                },
                _ => Err(IntentError::TypeError(
                    "last_index_of() requires two strings".to_string(),
                )),
            },
        },
    );

    // count(str, substr) -> Int - Count occurrences
    module.insert(
        "count".to_string(),
        Value::NativeFunction {
            name: "count".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(substr)) => {
                    Ok(Value::Int(s.matches(substr.as_str()).count() as i64))
                }
                _ => Err(IntentError::TypeError(
                    "count() requires two strings".to_string(),
                )),
            },
        },
    );

    // replace(str, from, to) -> String - Replace all occurrences
    module.insert(
        "replace".to_string(),
        Value::NativeFunction {
            name: "replace".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(from), Value::String(to)) => {
                    Ok(Value::String(s.replace(from.as_str(), to.as_str())))
                }
                _ => Err(IntentError::TypeError(
                    "replace() requires three strings".to_string(),
                )),
            },
        },
    );

    // replace_first(str, from, to) -> String - Replace only first occurrence
    module.insert(
        "replace_first".to_string(),
        Value::NativeFunction {
            name: "replace_first".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(from), Value::String(to)) => {
                    Ok(Value::String(s.replacen(from.as_str(), to.as_str(), 1)))
                }
                _ => Err(IntentError::TypeError(
                    "replace_first() requires three strings".to_string(),
                )),
            },
        },
    );

    // replace_chars(str, chars, replacement) -> String - Replace any character in chars set
    module.insert(
        "replace_chars".to_string(),
        Value::NativeFunction {
            name: "replace_chars".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(chars), Value::String(replacement)) => {
                    let char_set: std::collections::HashSet<char> = chars.chars().collect();
                    let result: String = s
                        .chars()
                        .flat_map(|c| {
                            if char_set.contains(&c) {
                                replacement.chars().collect::<Vec<_>>()
                            } else {
                                vec![c]
                            }
                        })
                        .collect();
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "replace_chars() requires three strings".to_string(),
                )),
            },
        },
    );

    // remove_chars(str, chars) -> String - Remove all characters in chars set
    module.insert(
        "remove_chars".to_string(),
        Value::NativeFunction {
            name: "remove_chars".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(chars)) => {
                    let char_set: std::collections::HashSet<char> = chars.chars().collect();
                    let result: String = s.chars().filter(|c| !char_set.contains(c)).collect();
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "remove_chars() requires two strings".to_string(),
                )),
            },
        },
    );

    // keep_chars(str, allowed) -> String - Keep only characters in allowed set
    module.insert(
        "keep_chars".to_string(),
        Value::NativeFunction {
            name: "keep_chars".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(allowed)) => {
                    let char_set: std::collections::HashSet<char> = allowed.chars().collect();
                    let result: String = s.chars().filter(|c| char_set.contains(c)).collect();
                    Ok(Value::String(result))
                }
                _ => Err(IntentError::TypeError(
                    "keep_chars() requires two strings".to_string(),
                )),
            },
        },
    );

    // ========== Regex Operations ==========

    // replace_pattern(str, pattern, replacement) -> String - Replace all regex matches
    module.insert(
        "replace_pattern".to_string(),
        Value::NativeFunction {
            name: "replace_pattern".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(pattern), Value::String(replacement)) => {
                    match regex::Regex::new(pattern) {
                        Ok(re) => Ok(Value::String(
                            re.replace_all(s, replacement.as_str()).to_string(),
                        )),
                        Err(e) => Err(IntentError::RuntimeError(format!(
                            "Invalid regex pattern: {}",
                            e
                        ))),
                    }
                }
                _ => Err(IntentError::TypeError(
                    "replace_pattern() requires three strings".to_string(),
                )),
            },
        },
    );

    // matches_pattern(str, pattern) -> Bool - Check if string matches regex pattern
    module.insert(
        "matches_pattern".to_string(),
        Value::NativeFunction {
            name: "matches_pattern".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match regex::Regex::new(pattern) {
                    Ok(re) => Ok(Value::Bool(re.is_match(s))),
                    Err(e) => Err(IntentError::RuntimeError(format!(
                        "Invalid regex pattern: {}",
                        e
                    ))),
                },
                _ => Err(IntentError::TypeError(
                    "matches_pattern() requires two strings".to_string(),
                )),
            },
        },
    );

    // find_pattern(str, pattern) -> Option<String> - Find first regex match
    module.insert(
        "find_pattern".to_string(),
        Value::NativeFunction {
            name: "find_pattern".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match regex::Regex::new(pattern) {
                    Ok(re) => match re.find(s) {
                        Some(m) => Ok(Value::EnumValue {
                            enum_name: "Option".to_string(),
                            variant: "Some".to_string(),
                            values: vec![Value::String(m.as_str().to_string())],
                        }),
                        None => Ok(Value::EnumValue {
                            enum_name: "Option".to_string(),
                            variant: "None".to_string(),
                            values: vec![],
                        }),
                    },
                    Err(e) => Err(IntentError::RuntimeError(format!(
                        "Invalid regex pattern: {}",
                        e
                    ))),
                },
                _ => Err(IntentError::TypeError(
                    "find_pattern() requires two strings".to_string(),
                )),
            },
        },
    );

    // find_all_pattern(str, pattern) -> [String] - Find all regex matches
    module.insert(
        "find_all_pattern".to_string(),
        Value::NativeFunction {
            name: "find_all_pattern".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match regex::Regex::new(pattern) {
                    Ok(re) => {
                        let matches: Vec<Value> = re
                            .find_iter(s)
                            .map(|m| Value::String(m.as_str().to_string()))
                            .collect();
                        Ok(Value::Array(matches))
                    }
                    Err(e) => Err(IntentError::RuntimeError(format!(
                        "Invalid regex pattern: {}",
                        e
                    ))),
                },
                _ => Err(IntentError::TypeError(
                    "find_all_pattern() requires two strings".to_string(),
                )),
            },
        },
    );

    // split_pattern(str, pattern) -> [String] - Split by regex pattern
    module.insert(
        "split_pattern".to_string(),
        Value::NativeFunction {
            name: "split_pattern".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match regex::Regex::new(pattern) {
                    Ok(re) => {
                        let parts: Vec<Value> =
                            re.split(s).map(|p| Value::String(p.to_string())).collect();
                        Ok(Value::Array(parts))
                    }
                    Err(e) => Err(IntentError::RuntimeError(format!(
                        "Invalid regex pattern: {}",
                        e
                    ))),
                },
                _ => Err(IntentError::TypeError(
                    "split_pattern() requires two strings".to_string(),
                )),
            },
        },
    );

    // ========== Substring & Extraction ==========

    // char_at(str, index) -> String
    module.insert(
        "char_at".to_string(),
        Value::NativeFunction {
            name: "char_at".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::Int(idx)) => {
                    let idx = *idx as usize;
                    s.chars()
                        .nth(idx)
                        .map(|c| Value::String(c.to_string()))
                        .ok_or_else(|| {
                            IntentError::RuntimeError(format!("Index {} out of bounds", idx))
                        })
                }
                _ => Err(IntentError::TypeError(
                    "char_at() requires string and int".to_string(),
                )),
            },
        },
    );

    // substring(str, start, end) -> String
    module.insert(
        "substring".to_string(),
        Value::NativeFunction {
            name: "substring".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(start), Value::Int(end)) => {
                    let start = *start as usize;
                    let end = *end as usize;
                    let chars: Vec<char> = s.chars().collect();
                    if start > chars.len() || end > chars.len() || start > end {
                        return Err(IntentError::RuntimeError(
                            "Invalid substring range".to_string(),
                        ));
                    }
                    Ok(Value::String(chars[start..end].iter().collect()))
                }
                _ => Err(IntentError::TypeError(
                    "substring() requires string, int, int".to_string(),
                )),
            },
        },
    );

    // chars(str) -> [String] - Split into characters
    module.insert(
        "chars".to_string(),
        Value::NativeFunction {
            name: "chars".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let chars: Vec<Value> =
                        s.chars().map(|c| Value::String(c.to_string())).collect();
                    Ok(Value::Array(chars))
                }
                _ => Err(IntentError::TypeError(
                    "chars() requires a string".to_string(),
                )),
            },
        },
    );

    // lines(str) -> [String] - Split by newlines
    module.insert(
        "lines".to_string(),
        Value::NativeFunction {
            name: "lines".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let lines: Vec<Value> =
                        s.lines().map(|l| Value::String(l.to_string())).collect();
                    Ok(Value::Array(lines))
                }
                _ => Err(IntentError::TypeError(
                    "lines() requires a string".to_string(),
                )),
            },
        },
    );

    // words(str) -> [String] - Split by whitespace
    module.insert(
        "words".to_string(),
        Value::NativeFunction {
            name: "words".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let words: Vec<Value> = s
                        .split_whitespace()
                        .map(|w| Value::String(w.to_string()))
                        .collect();
                    Ok(Value::Array(words))
                }
                _ => Err(IntentError::TypeError(
                    "words() requires a string".to_string(),
                )),
            },
        },
    );

    // truncate(str, max_len, suffix) -> String - Truncate with suffix
    module.insert(
        "truncate".to_string(),
        Value::NativeFunction {
            name: "truncate".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(max_len), Value::String(suffix)) => {
                    let max = *max_len as usize;
                    let chars: Vec<char> = s.chars().collect();
                    if chars.len() <= max {
                        Ok(Value::String(s.clone()))
                    } else {
                        let truncate_at = max.saturating_sub(suffix.len());
                        let truncated: String = chars[..truncate_at].iter().collect();
                        Ok(Value::String(format!("{}{}", truncated, suffix)))
                    }
                }
                _ => Err(IntentError::TypeError(
                    "truncate() requires string, int, string".to_string(),
                )),
            },
        },
    );

    // ========== Padding ==========

    // pad_left(str, length, char) -> String
    module.insert(
        "pad_left".to_string(),
        Value::NativeFunction {
            name: "pad_left".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(len), Value::String(pad_char)) => {
                    let target_len = *len as usize;
                    let pad = pad_char.chars().next().unwrap_or(' ');
                    if s.len() >= target_len {
                        Ok(Value::String(s.clone()))
                    } else {
                        let padding =
                            std::iter::repeat_n(pad, target_len - s.len()).collect::<String>();
                        Ok(Value::String(format!("{}{}", padding, s)))
                    }
                }
                _ => Err(IntentError::TypeError(
                    "pad_left() requires string, int, string".to_string(),
                )),
            },
        },
    );

    // pad_right(str, length, char) -> String
    module.insert(
        "pad_right".to_string(),
        Value::NativeFunction {
            name: "pad_right".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(len), Value::String(pad_char)) => {
                    let target_len = *len as usize;
                    let pad = pad_char.chars().next().unwrap_or(' ');
                    if s.len() >= target_len {
                        Ok(Value::String(s.clone()))
                    } else {
                        let padding =
                            std::iter::repeat_n(pad, target_len - s.len()).collect::<String>();
                        Ok(Value::String(format!("{}{}", s, padding)))
                    }
                }
                _ => Err(IntentError::TypeError(
                    "pad_right() requires string, int, string".to_string(),
                )),
            },
        },
    );

    // center(str, length, char) -> String - Center with padding
    module.insert(
        "center".to_string(),
        Value::NativeFunction {
            name: "center".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::Int(len), Value::String(pad_char)) => {
                    let target_len = *len as usize;
                    let pad = pad_char.chars().next().unwrap_or(' ');
                    if s.len() >= target_len {
                        Ok(Value::String(s.clone()))
                    } else {
                        let total_padding = target_len - s.len();
                        let left_padding = total_padding / 2;
                        let right_padding = total_padding - left_padding;
                        let left: String = std::iter::repeat_n(pad, left_padding).collect();
                        let right: String = std::iter::repeat_n(pad, right_padding).collect();
                        Ok(Value::String(format!("{}{}{}", left, s, right)))
                    }
                }
                _ => Err(IntentError::TypeError(
                    "center() requires string, int, string".to_string(),
                )),
            },
        },
    );

    // ========== Validation ==========

    // is_empty(str) -> Bool
    module.insert(
        "is_empty".to_string(),
        Value::NativeFunction {
            name: "is_empty".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(s.is_empty())),
                _ => Err(IntentError::TypeError(
                    "is_empty() requires a string".to_string(),
                )),
            },
        },
    );

    // is_blank(str) -> Bool - Empty or only whitespace
    module.insert(
        "is_blank".to_string(),
        Value::NativeFunction {
            name: "is_blank".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(s.trim().is_empty())),
                _ => Err(IntentError::TypeError(
                    "is_blank() requires a string".to_string(),
                )),
            },
        },
    );

    // is_numeric(str) -> Bool - All digits
    module.insert(
        "is_numeric".to_string(),
        Value::NativeFunction {
            name: "is_numeric".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(
                    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()),
                )),
                _ => Err(IntentError::TypeError(
                    "is_numeric() requires a string".to_string(),
                )),
            },
        },
    );

    // is_alpha(str) -> Bool - All letters
    module.insert(
        "is_alpha".to_string(),
        Value::NativeFunction {
            name: "is_alpha".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(
                    !s.is_empty() && s.chars().all(|c| c.is_alphabetic()),
                )),
                _ => Err(IntentError::TypeError(
                    "is_alpha() requires a string".to_string(),
                )),
            },
        },
    );

    // is_alphanumeric(str) -> Bool - Letters and digits only
    module.insert(
        "is_alphanumeric".to_string(),
        Value::NativeFunction {
            name: "is_alphanumeric".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(
                    !s.is_empty() && s.chars().all(|c| c.is_alphanumeric()),
                )),
                _ => Err(IntentError::TypeError(
                    "is_alphanumeric() requires a string".to_string(),
                )),
            },
        },
    );

    // is_lowercase(str) -> Bool
    module.insert(
        "is_lowercase".to_string(),
        Value::NativeFunction {
            name: "is_lowercase".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(
                    !s.is_empty() && s.chars().all(|c| !c.is_alphabetic() || c.is_lowercase()),
                )),
                _ => Err(IntentError::TypeError(
                    "is_lowercase() requires a string".to_string(),
                )),
            },
        },
    );

    // is_uppercase(str) -> Bool
    module.insert(
        "is_uppercase".to_string(),
        Value::NativeFunction {
            name: "is_uppercase".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(
                    !s.is_empty() && s.chars().all(|c| !c.is_alphabetic() || c.is_uppercase()),
                )),
                _ => Err(IntentError::TypeError(
                    "is_uppercase() requires a string".to_string(),
                )),
            },
        },
    );

    // is_whitespace(str) -> Bool
    module.insert(
        "is_whitespace".to_string(),
        Value::NativeFunction {
            name: "is_whitespace".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::Bool(
                    !s.is_empty() && s.chars().all(|c| c.is_whitespace()),
                )),
                _ => Err(IntentError::TypeError(
                    "is_whitespace() requires a string".to_string(),
                )),
            },
        },
    );

    // matches(str, pattern) -> Bool - Simple glob-like matching (* and ?)
    module.insert(
        "matches".to_string(),
        Value::NativeFunction {
            name: "matches".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => {
                    Ok(Value::Bool(simple_glob_match(s, pattern)))
                }
                _ => Err(IntentError::TypeError(
                    "matches() requires two strings".to_string(),
                )),
            },
        },
    );

    // ========== Aliases for compatibility ==========

    // replace_all(str, from, to) -> String (alias for replace)
    module.insert(
        "replace_all".to_string(),
        Value::NativeFunction {
            name: "replace_all".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(s), Value::String(from), Value::String(to)) => {
                    Ok(Value::String(s.replace(from.as_str(), to.as_str())))
                }
                _ => Err(IntentError::TypeError(
                    "replace_all() requires three strings".to_string(),
                )),
            },
        },
    );

    // upper(str) -> String (alias for to_upper)
    module.insert(
        "upper".to_string(),
        Value::NativeFunction {
            name: "upper".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_uppercase())),
                _ => Err(IntentError::TypeError(
                    "upper() requires a string".to_string(),
                )),
            },
        },
    );

    // lower(str) -> String (alias for to_lower)
    module.insert(
        "lower".to_string(),
        Value::NativeFunction {
            name: "lower".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.to_lowercase())),
                _ => Err(IntentError::TypeError(
                    "lower() requires a string".to_string(),
                )),
            },
        },
    );

    // trim_start(str) -> String (alias for trim_left)
    module.insert(
        "trim_start".to_string(),
        Value::NativeFunction {
            name: "trim_start".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim_start().to_string())),
                _ => Err(IntentError::TypeError(
                    "trim_start() requires a string".to_string(),
                )),
            },
        },
    );

    // trim_end(str) -> String (alias for trim_right)
    module.insert(
        "trim_end".to_string(),
        Value::NativeFunction {
            name: "trim_end".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(s) => Ok(Value::String(s.trim_end().to_string())),
                _ => Err(IntentError::TypeError(
                    "trim_end() requires a string".to_string(),
                )),
            },
        },
    );

    module
}

/// Simple glob-like pattern matching (supports * and ?)
fn simple_glob_match(s: &str, pattern: &str) -> bool {
    let mut s_idx = 0;
    let mut p_idx = 0;
    let mut star_idx: Option<usize> = None;
    let mut match_idx = 0;

    let s_vec: Vec<char> = s.chars().collect();
    let p_vec: Vec<char> = pattern.chars().collect();

    while s_idx < s_vec.len() {
        if p_idx < p_vec.len() && (p_vec[p_idx] == '?' || p_vec[p_idx] == s_vec[s_idx]) {
            s_idx += 1;
            p_idx += 1;
        } else if p_idx < p_vec.len() && p_vec[p_idx] == '*' {
            star_idx = Some(p_idx);
            match_idx = s_idx;
            p_idx += 1;
        } else if let Some(star) = star_idx {
            p_idx = star + 1;
            match_idx += 1;
            s_idx = match_idx;
        } else {
            return false;
        }
    }

    while p_idx < p_vec.len() && p_vec[p_idx] == '*' {
        p_idx += 1;
    }

    p_idx == p_vec.len()
}
