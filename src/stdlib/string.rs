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

    // @ntnt split
    // @module std/string
    // @module_description String manipulation: splitting, joining, trimming, searching, and transforming text
    // @signature split(s: String, delim: String) -> Array<String>
    // @param s The string to split
    // @param delim The delimiter to split on
    // Splits a string into an array of substrings.
    //
    // When the delimiter is not found, returns a single-element array
    // containing the original string. An empty delimiter splits into
    // individual characters.
    // @see_also join, chars, split_pattern
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example split("a,b,c", ",") => ["a", "b", "c"] ~ "Basic comma-separated split"
    // @example split("no-match", ",") => ["no-match"] ~ "No delimiter found returns original in array"
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

    // @ntnt join
    // @module std/string
    // @signature join(arr: Array, delim: String) -> String
    // @param arr The array of elements to join
    // @param delim The delimiter to insert between elements
    // Joins array elements into a string with a delimiter.
    //
    // Non-string elements are converted to their string representation.
    // @see_also split, concat
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example join(["a", "b"], ",") => "a,b" ~ "Join with comma"
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

    // @ntnt concat
    // @module std/string
    // @signature concat(a: String, b: String) -> String
    // @param a The first string, or an array of strings to concatenate
    // @param b The second string to append
    // Concatenates two strings, or joins an array of strings.
    //
    // Also accepts an array as the first argument, in which case
    // all elements are concatenated without a delimiter.
    // @see_also join, split
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example concat("hello", " world") => "hello world" ~ "Concatenate two strings"
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

    // @ntnt repeat
    // @module std/string
    // @signature repeat(s: String, n: Int) -> String
    // @param s The string to repeat
    // @param n The number of times to repeat the string
    // Repeats a string n times.
    // @see_also pad_left, pad_right
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example repeat("ab", 3) => "ababab" ~ "Repeat string three times"
    // @error RuntimeError ~ "repeat count must be non-negative" fix: "Ensure n >= 0"
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

    // @ntnt reverse
    // @module std/string
    // @signature reverse(s: String) -> String
    // @param s The string to reverse
    // Reverses a string.
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example reverse("abc") => "cba" ~ "Reverse characters"
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

    // @ntnt trim
    // @module std/string
    // @signature trim(s: String) -> String
    // @param s The string to trim
    // Removes leading and trailing whitespace.
    // @see_also trim_left, trim_right, trim_chars
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example trim("  hello  ") => "hello" ~ "Trim both ends"
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

    // @ntnt trim_left
    // @module std/string
    // @signature trim_left(s: String) -> String
    // @param s The string to trim from the left
    // Removes leading whitespace.
    // @see_also trim, trim_right, trim_start
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example trim_left("  hello") => "hello" ~ "Remove leading spaces"
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

    // @ntnt trim_right
    // @module std/string
    // @signature trim_right(s: String) -> String
    // @param s The string to trim from the right
    // Removes trailing whitespace.
    // @see_also trim, trim_left, trim_end
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example trim_right("hello  ") => "hello" ~ "Remove trailing spaces"
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

    // @ntnt trim_chars
    // @module std/string
    // @signature trim_chars(s: String, chars: String) -> String
    // @param s The string to trim
    // @param chars The set of characters to remove from both ends
    // Removes specified characters from both ends.
    // @see_also trim, remove_chars
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example trim_chars("-hello-", "-") => "hello" ~ "Trim specific character"
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

    // @ntnt to_upper
    // @module std/string
    // @signature to_upper(s: String) -> String
    // @param s The string to convert to uppercase
    // Converts string to uppercase.
    // @see_also to_lower, capitalize, upper
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example to_upper("hello") => "HELLO" ~ "Convert to uppercase"
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

    // @ntnt to_lower
    // @module std/string
    // @signature to_lower(s: String) -> String
    // @param s The string to convert to lowercase
    // Converts string to lowercase.
    // @see_also to_upper, capitalize, lower
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example to_lower("HELLO") => "hello" ~ "Convert to lowercase"
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

    // @ntnt capitalize
    // @module std/string
    // @signature capitalize(s: String) -> String
    // @param s The string to capitalize
    // Capitalizes the first character, lowercases the rest.
    // @see_also title, to_upper
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example capitalize("hello") => "Hello" ~ "Capitalize first letter"
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

    // @ntnt title
    // @module std/string
    // @signature title(s: String) -> String
    // @param s The string to convert to title case
    // Capitalizes the first letter of each word.
    // @see_also capitalize, to_upper
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example title("hello world") => "Hello World" ~ "Title case each word"
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

    // @ntnt to_snake_case
    // @module std/string
    // @signature to_snake_case(s: String) -> String
    // @param s The string to convert to snake_case
    // Converts to snake_case.
    // @see_also to_camel_case, to_pascal_case, to_kebab_case
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example to_snake_case("helloWorld") => "hello_world" ~ "camelCase to snake_case"
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

    // @ntnt to_camel_case
    // @module std/string
    // @signature to_camel_case(s: String) -> String
    // @param s The string to convert to camelCase
    // Converts to camelCase.
    // @see_also to_snake_case, to_pascal_case, to_kebab_case
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example to_camel_case("hello_world") => "helloWorld" ~ "snake_case to camelCase"
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

    // @ntnt to_pascal_case
    // @module std/string
    // @signature to_pascal_case(s: String) -> String
    // @param s The string to convert to PascalCase
    // Converts to PascalCase.
    // @see_also to_snake_case, to_camel_case, to_kebab_case
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example to_pascal_case("hello_world") => "HelloWorld" ~ "snake_case to PascalCase"
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

    // @ntnt to_kebab_case
    // @module std/string
    // @signature to_kebab_case(s: String) -> String
    // @param s The string to convert to kebab-case
    // Converts to kebab-case.
    // @see_also to_snake_case, to_camel_case, slugify
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example to_kebab_case("helloWorld") => "hello-world" ~ "camelCase to kebab-case"
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

    // @ntnt slugify
    // @module std/string
    // @signature slugify(s: String) -> String
    // @param s The string to convert into a URL-friendly slug
    // Converts to a URL-friendly slug.
    //
    // Lowercases, replaces spaces and underscores with hyphens,
    // removes non-alphanumeric characters, and collapses consecutive hyphens.
    // @see_also to_kebab_case, to_snake_case
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example slugify("Hello World!") => "hello-world" ~ "URL-friendly slug"
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

    // @ntnt contains
    // @module std/string
    // @signature contains(s: String, substr: String) -> Bool
    // @param s The string to search within
    // @param substr The substring to search for
    // Checks if string contains substring.
    // @see_also starts_with, ends_with, index_of
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example contains("hello", "ell") => true ~ "Substring found"
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

    // @ntnt starts_with
    // @module std/string
    // @signature starts_with(s: String, prefix: String) -> Bool
    // @param s The string to check
    // @param prefix The prefix to test for
    // Checks if string starts with prefix.
    // @see_also ends_with, contains
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example starts_with("hello", "he") => true ~ "Prefix match"
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

    // @ntnt ends_with
    // @module std/string
    // @signature ends_with(s: String, suffix: String) -> Bool
    // @param s The string to check
    // @param suffix The suffix to look for at the end of the string
    // Checks if string ends with suffix.
    // @see_also starts_with, contains
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example ends_with("hello", "lo") => true ~ "Suffix match"
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

    // @ntnt index_of
    // @module std/string
    // @signature index_of(s: String, substr: String) -> Int
    // @param s The string to search within
    // @param substr The substring to find the first occurrence of
    // Returns index of first occurrence, or -1 if not found.
    // @see_also last_index_of, contains
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example index_of("hello", "l") => 2 ~ "First occurrence index"
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

    // @ntnt last_index_of
    // @module std/string
    // @signature last_index_of(s: String, substr: String) -> Int
    // @param s The string to search within
    // @param substr The substring to find the last occurrence of
    // Returns index of last occurrence, or -1 if not found.
    // @see_also index_of, contains
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example last_index_of("hello", "l") => 3 ~ "Last occurrence index"
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

    // @ntnt count
    // @module std/string
    // @signature count(s: String, substr: String) -> Int
    // @param s The string to search within
    // @param substr The substring to count occurrences of
    // Counts occurrences of substring.
    // @see_also contains, index_of
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example count("ababa", "a") => 3 ~ "Count character occurrences"
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

    // @ntnt replace
    // @module std/string
    // @signature replace(s: String, from: String, to: String) -> String
    // @param s The input string to perform replacements on
    // @param from The substring to search for
    // @param to The replacement string for each occurrence
    // Replaces all occurrences of from with to.
    // @see_also replace_first, replace_chars, replace_pattern, replace_all
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example replace("hello", "l", "L") => "heLLo" ~ "Replace all occurrences"
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

    // @ntnt replace_first
    // @module std/string
    // @signature replace_first(s: String, from: String, to: String) -> String
    // @param s The input string to perform the replacement on
    // @param from The substring to search for
    // @param to The replacement string for the first match
    // Replaces first occurrence of from with to.
    // @see_also replace, replace_pattern
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example replace_first("abab", "a", "X") => "Xbab" ~ "Replace first only"
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

    // @ntnt replace_chars
    // @module std/string
    // @signature replace_chars(s: String, chars: String, repl: String) -> String
    // @param s The input string to perform replacements on
    // @param chars The set of characters to replace (each character is matched individually)
    // @param repl The replacement string for each matched character
    // Replaces any character in the chars set with replacement.
    // @see_also remove_chars, keep_chars, replace
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example replace_chars("a.b_c", "._", "-") => "a-b-c" ~ "Replace characters from set"
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

    // @ntnt remove_chars
    // @module std/string
    // @signature remove_chars(s: String, chars: String) -> String
    // @param s The input string to remove characters from
    // @param chars The set of characters to remove (each character is matched individually)
    // Removes all characters in the chars set.
    // @see_also replace_chars, keep_chars
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example remove_chars("a!b?", "!?") => "ab" ~ "Remove punctuation"
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

    // @ntnt keep_chars
    // @module std/string
    // @signature keep_chars(s: String, allowed: String) -> String
    // @param s The input string to filter
    // @param allowed The set of characters to keep (all others are removed)
    // Keeps only characters in the allowed set.
    // @see_also remove_chars, replace_chars
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example keep_chars("a1b2", "ab") => "ab" ~ "Keep only letters"
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

    // @ntnt replace_pattern
    // @module std/string
    // @signature replace_pattern(s: String, pattern: String, repl: String) -> String
    // @param s The input string to search within
    // @param pattern A regular expression pattern to match against
    // @param repl The replacement string for each match
    // Replaces all regex matches with replacement.
    // @see_also matches_pattern, find_pattern, split_pattern, capture_pattern, replace
    // @since v0.2.0
    // @tags #pure
    // @example replace_pattern("a1b2", "\\d", "X") => "aXbX" ~ "Replace digits with X"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
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

    // @ntnt matches_pattern
    // @module std/string
    // @signature matches_pattern(s: String, pattern: String) -> Bool
    // @param s The string to test
    // @param pattern A regular expression pattern to match against
    // Checks if string matches regex pattern.
    // @see_also find_pattern, replace_pattern, matches
    // @since v0.2.0
    // @tags #pure
    // @example matches_pattern("test123", "\\d+") => true ~ "Contains digits"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
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

    // @ntnt find_pattern
    // @module std/string
    // @signature find_pattern(s: String, pattern: String) -> Option<String>
    // @param s The string to search within
    // @param pattern A regular expression pattern to find
    // Returns first regex match or None.
    // @see_also find_all_pattern, matches_pattern, capture_pattern
    // @since v0.2.0
    // @tags #pure
    // @example find_pattern("ab12cd", "\\d+") => Some("12") ~ "Find first digit sequence"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
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

    // @ntnt find_all_pattern
    // @module std/string
    // @signature find_all_pattern(s: String, pattern: String) -> Array<String>
    // @param s The string to search within
    // @param pattern A regular expression pattern to find all occurrences of
    // Returns all regex matches.
    // @see_also find_pattern, matches_pattern, capture_all_pattern
    // @since v0.2.0
    // @tags #pure
    // @example find_all_pattern("a1b2", "\\d") => ["1", "2"] ~ "Find all digits"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
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

    // @ntnt split_pattern
    // @module std/string
    // @signature split_pattern(s: String, pattern: String) -> Array<String>
    // @param s The string to split
    // @param pattern A regular expression pattern to split on
    // Splits string by regex pattern.
    // @see_also split, find_all_pattern
    // @since v0.2.0
    // @tags #pure
    // @example split_pattern("a1b2c", "\\d") => ["a", "b", "c"] ~ "Split on digits"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
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

    // @ntnt capture_pattern
    // @module std/string
    // @signature capture_pattern(s: String, pattern: String) -> Option<Array<String>>
    // @param s The string to search within
    // @param pattern A regular expression pattern with capture groups
    // Returns first match with capture groups, or None if no match.
    //
    // Returns an array where index 0 is the full match and subsequent indices
    // are the capture groups. Unmatched optional groups produce empty strings.
    // @returns Option containing array of [full_match, group1, group2, ...] or None
    // @see_also capture_all_pattern, capture_named_pattern, find_pattern
    // @since v0.3.10
    // @tags #pure
    // @example capture_pattern("2024-01-15", r"(\d{4})-(\d{2})-(\d{2})") => Some(["2024-01-15", "2024", "01", "15"]) ~ "Extract date parts"
    // @example capture_pattern("no match", r"(\d+)") => None ~ "Returns None when no match"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
    module.insert(
        "capture_pattern".to_string(),
        Value::NativeFunction {
            name: "capture_pattern".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match regex::Regex::new(pattern) {
                    Ok(re) => match re.captures(s) {
                        Some(caps) => {
                            let groups: Vec<Value> = caps
                                .iter()
                                .map(|m| {
                                    Value::String(
                                        m.map(|m| m.as_str().to_string()).unwrap_or_default(),
                                    )
                                })
                                .collect();
                            Ok(Value::EnumValue {
                                enum_name: "Option".to_string(),
                                variant: "Some".to_string(),
                                values: vec![Value::Array(groups)],
                            })
                        }
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
                    "capture_pattern() requires two strings".to_string(),
                )),
            },
        },
    );

    // @ntnt capture_all_pattern
    // @module std/string
    // @signature capture_all_pattern(s: String, pattern: String) -> Array<Array<String>>
    // @param s The string to search within
    // @param pattern A regular expression pattern with capture groups
    // Returns all matches with capture groups.
    //
    // Each inner array has the full match at index 0 followed by capture groups.
    // Returns an empty array if there are no matches.
    // @returns Array of arrays, each containing [full_match, group1, group2, ...]
    // @see_also capture_pattern, capture_named_pattern, find_all_pattern
    // @since v0.3.10
    // @tags #pure
    // @example capture_all_pattern("2024-01 and 2025-02", r"(\d{4})-(\d{2})") => [["2024-01", "2024", "01"], ["2025-02", "2025", "02"]] ~ "Extract multiple date pairs"
    // @example capture_all_pattern("no match", r"(\d+)") => [] ~ "Returns empty array when no matches"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
    module.insert(
        "capture_all_pattern".to_string(),
        Value::NativeFunction {
            name: "capture_all_pattern".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match regex::Regex::new(pattern) {
                    Ok(re) => {
                        let all_matches: Vec<Value> = re
                            .captures_iter(s)
                            .map(|caps| {
                                let groups: Vec<Value> = caps
                                    .iter()
                                    .map(|m| {
                                        Value::String(
                                            m.map(|m| m.as_str().to_string()).unwrap_or_default(),
                                        )
                                    })
                                    .collect();
                                Value::Array(groups)
                            })
                            .collect();
                        Ok(Value::Array(all_matches))
                    }
                    Err(e) => Err(IntentError::RuntimeError(format!(
                        "Invalid regex pattern: {}",
                        e
                    ))),
                },
                _ => Err(IntentError::TypeError(
                    "capture_all_pattern() requires two strings".to_string(),
                )),
            },
        },
    );

    // @ntnt capture_named_pattern
    // @module std/string
    // @signature capture_named_pattern(s: String, pattern: String) -> Option<Map<String, String>>
    // @param s The string to search within
    // @param pattern A regular expression pattern with named capture groups using (?P<name>...) syntax
    // Returns first match with named capture groups as a map, or None if no match.
    //
    // Named groups become map keys. Unnamed groups use their numeric index as
    // a string key ("0" for full match, "1", "2", etc.). Unmatched optional
    // groups produce empty string values.
    // @returns Option containing map of group names to matched strings, or None
    // @see_also capture_pattern, capture_all_pattern, find_pattern
    // @since v0.3.10
    // @tags #pure
    // @example capture_named_pattern("2024-01-15", r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})") => Some({"0": "2024-01-15", "year": "2024", "month": "01", "day": "15"}) ~ "Named groups as map keys"
    // @example capture_named_pattern("no match", r"(?P<num>\d+)") => None ~ "Returns None when no match"
    // @error RuntimeError ~ "Invalid regex pattern" fix: "Check regex syntax"
    module.insert(
        "capture_named_pattern".to_string(),
        Value::NativeFunction {
            name: "capture_named_pattern".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(s), Value::String(pattern)) => match regex::Regex::new(pattern) {
                    Ok(re) => match re.captures(s) {
                        Some(caps) => {
                            let mut map = std::collections::HashMap::new();
                            for (i, name) in re.capture_names().enumerate() {
                                let value = caps
                                    .get(i)
                                    .map(|m| m.as_str().to_string())
                                    .unwrap_or_default();
                                let key = match name {
                                    Some(n) => n.to_string(),
                                    None => i.to_string(),
                                };
                                map.insert(key, Value::String(value));
                            }
                            Ok(Value::EnumValue {
                                enum_name: "Option".to_string(),
                                variant: "Some".to_string(),
                                values: vec![Value::Map(map)],
                            })
                        }
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
                    "capture_named_pattern() requires two strings".to_string(),
                )),
            },
        },
    );

    // ========== Substring & Extraction ==========

    // @ntnt char_at
    // @module std/string
    // @signature char_at(s: String, index: Int) -> String
    // @param s The string to index into
    // @param index Zero-based position of the character to retrieve
    // Returns character at index.
    // @see_also substring, chars
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example char_at("hello", 0) => "h" ~ "First character"
    // @error RuntimeError ~ "Index out of bounds" fix: "Check string length with len() first"
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

    // @ntnt substring
    // @module std/string
    // @signature substring(s: String, start: Int, end: Int) -> String
    // @param s The source string to extract from
    // @param start The zero-based starting index (inclusive)
    // @param end The zero-based ending index (exclusive)
    // Extracts substring from start to end (exclusive).
    // @see_also char_at, chars
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example substring("hello", 1, 4) => "ell" ~ "Extract middle portion"
    // @error RuntimeError ~ "Invalid substring range" fix: "Ensure 0 <= start <= end <= len(s)"
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

    // @ntnt chars
    // @module std/string
    // @signature chars(s: String) -> Array<String>
    // @param s The string to split into individual characters
    // Splits string into array of characters.
    // @see_also split, char_at
    // @since v0.1.0
    // @tags #pure, #deterministic, #allocates
    // @example chars("hi") => ["h", "i"] ~ "Split into characters"
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

    // @ntnt lines
    // @module std/string
    // @signature lines(s: String) -> Array<String>
    // @param s The string to split into newline-delimited lines
    // Splits string by newlines.
    // @see_also split, words
    // @since v0.1.0
    // @tags #pure, #deterministic, #allocates
    // @example lines("a\nb") => ["a", "b"] ~ "Split by newline"
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

    // @ntnt words
    // @module std/string
    // @signature words(s: String) -> Array<String>
    // @param s The string to split into whitespace-delimited words
    // Splits string by whitespace.
    // @see_also split, lines
    // @since v0.1.0
    // @tags #pure, #deterministic, #allocates
    // @example words("hello world") => ["hello", "world"] ~ "Split by whitespace"
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

    // @ntnt truncate
    // @module std/string
    // @signature truncate(s: String, max_len: Int, suffix: String) -> String
    // @param s The string to truncate
    // @param max_len The maximum length of the resulting string (including suffix)
    // @param suffix The string to append when truncation occurs (e.g. "...")
    // Truncates string to max length with suffix.
    //
    // If the string is shorter than max_len, it is returned unchanged.
    // @see_also substring
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example truncate("hello world", 8, "...") => "hello..." ~ "Truncate with ellipsis"
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

    // @ntnt pad_left
    // @module std/string
    // @signature pad_left(s: String, len: Int, char: String) -> String
    // @param s The string to pad
    // @param len The desired total length of the result
    // @param char The character to use for padding (first char used if multi-char)
    // Pads string on the left to reach target length.
    // @see_also pad_right, center
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example pad_left("5", 3, "0") => "005" ~ "Zero-pad a number"
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

    // @ntnt pad_right
    // @module std/string
    // @signature pad_right(s: String, len: Int, char: String) -> String
    // @param s The string to pad
    // @param len The desired total length of the result
    // @param char The character to use for padding (first char used if multi-char)
    // Pads string on the right to reach target length.
    // @see_also pad_left, center
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example pad_right("5", 3, "0") => "500" ~ "Pad right with zeros"
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

    // @ntnt center
    // @module std/string
    // @signature center(s: String, len: Int, char: String) -> String
    // @param s The string to center
    // @param len The desired total length of the result
    // @param char The character to use for padding on both sides
    // Centers string with padding on both sides.
    // @see_also pad_left, pad_right
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example center("hi", 6, "-") => "--hi--" ~ "Center with dashes"
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

    // @ntnt is_empty
    // @module std/string
    // @signature is_empty(s: String) -> Bool
    // @param s The string to check for emptiness
    // Returns true if string is empty.
    // @see_also is_blank
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example is_empty("") => true ~ "Empty string"
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

    // @ntnt is_blank
    // @module std/string
    // @signature is_blank(s: String) -> Bool
    // @param s The string to check for blankness (empty or whitespace-only)
    // Returns true if string is empty or only whitespace.
    // @see_also is_empty, is_whitespace
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example is_blank("  ") => true ~ "Whitespace-only is blank"
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

    // @ntnt is_numeric
    // @module std/string
    // @signature is_numeric(s: String) -> Bool
    // @param s The string to check for numeric-only content
    // Returns true if string contains only digits.
    // @see_also is_alpha, is_alphanumeric
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example is_numeric("123") => true ~ "All digits"
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

    // @ntnt is_alpha
    // @module std/string
    // @signature is_alpha(s: String) -> Bool
    // @param s The string to check for alphabetic-only content
    // Returns true if string contains only letters.
    // @see_also is_numeric, is_alphanumeric
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example is_alpha("abc") => true ~ "All letters"
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

    // @ntnt is_alphanumeric
    // @module std/string
    // @signature is_alphanumeric(s: String) -> Bool
    // @param s The string to check for alphanumeric-only content
    // Returns true if string contains only letters and digits.
    // @see_also is_alpha, is_numeric
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example is_alphanumeric("abc123") => true ~ "Letters and digits"
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

    // @ntnt is_lowercase
    // @module std/string
    // @signature is_lowercase(s: String) -> Bool
    // @param s The string to check for lowercase letters
    // Returns true if all letters are lowercase.
    // @see_also is_uppercase, to_lower
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example is_lowercase("hello") => true ~ "All lowercase"
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

    // @ntnt is_uppercase
    // @module std/string
    // @signature is_uppercase(s: String) -> Bool
    // @param s The string to check for uppercase letters
    // Returns true if all letters are uppercase.
    // @see_also is_lowercase, to_upper
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example is_uppercase("HELLO") => true ~ "All uppercase"
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

    // @ntnt is_whitespace
    // @module std/string
    // @signature is_whitespace(s: String) -> Bool
    // @param s The string to check for whitespace-only content
    // Returns true if string contains only whitespace.
    // @see_also is_blank, is_empty
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example is_whitespace("  \t") => true ~ "Spaces and tabs"
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

    // @ntnt matches
    // @module std/string
    // @signature matches(s: String, pattern: String) -> Bool
    // @param s The string to test against the pattern
    // @param pattern A glob pattern using * (any chars) and ? (single char) wildcards
    // Simple glob matching with * and ? wildcards.
    //
    // Uses glob-style patterns, not regular expressions.
    // Use matches_pattern() for regex matching.
    // @see_also matches_pattern, contains
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example matches("hello.txt", "*.txt") => true ~ "Glob wildcard match"
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

    // @ntnt replace_all
    // @module std/string
    // @signature replace_all(s: String, from: String, to: String) -> String
    // @param s The input string to perform replacements on
    // @param from The substring to search for
    // @param to The replacement string for each occurrence
    // Alias for replace. Replaces all occurrences of from with to.
    // @see_also replace
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example replace_all("hello", "l", "L") => "heLLo" ~ "Replace all occurrences"
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

    // @ntnt upper
    // @module std/string
    // @signature upper(s: String) -> String
    // @param s The string to convert to uppercase
    // Alias for to_upper. Converts string to uppercase.
    // @see_also to_upper, lower
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example upper("hello") => "HELLO" ~ "Convert to uppercase"
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

    // @ntnt lower
    // @module std/string
    // @signature lower(s: String) -> String
    // @param s The string to convert to lowercase
    // Alias for to_lower. Converts string to lowercase.
    // @see_also to_lower, upper
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example lower("HELLO") => "hello" ~ "Convert to lowercase"
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

    // @ntnt trim_start
    // @module std/string
    // @signature trim_start(s: String) -> String
    // @param s The string to trim from the left
    // Alias for trim_left. Removes leading whitespace.
    // @see_also trim_left, trim_end
    // @since v0.3.0
    // @tags #pure, #deterministic
    // @example trim_start("  hello") => "hello" ~ "Remove leading spaces"
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

    // @ntnt trim_end
    // @module std/string
    // @signature trim_end(s: String) -> String
    // @param s The string to trim from the right
    // Alias for trim_right. Removes trailing whitespace.
    // @see_also trim_right, trim_start
    // @since v0.3.0
    // @tags #pure, #deterministic
    // @example trim_end("hello  ") => "hello" ~ "Remove trailing spaces"
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
