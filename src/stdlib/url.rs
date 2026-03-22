//! std/url module - URL parsing and encoding

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// URL encode a string (preserves some URL-safe characters)
pub fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z'
            | 'a'..='z'
            | '0'..='9'
            | '-'
            | '_'
            | '.'
            | '~'
            | '/'
            | ':'
            | '?'
            | '#'
            | '['
            | ']'
            | '@'
            | '!'
            | '$'
            | '&'
            | '\''
            | '('
            | ')'
            | '*'
            | '+'
            | ','
            | ';'
            | '=' => {
                result.push(c);
            }
            _ => {
                for byte in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}

/// URL encode a component (more aggressive, for query params)
pub fn url_encode_component(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                result.push(c);
            }
            _ => {
                for byte in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}

/// URL decode a string
pub fn url_decode(s: &str) -> std::result::Result<String, String> {
    let mut result = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if hex.len() == 2 {
                match u8::from_str_radix(&hex, 16) {
                    Ok(byte) => result.push(byte),
                    Err(_) => return Err(format!("Invalid percent encoding: %{}", hex)),
                }
            } else {
                return Err("Incomplete percent encoding".to_string());
            }
        } else if c == '+' {
            result.push(b' ');
        } else {
            for byte in c.to_string().as_bytes() {
                result.push(*byte);
            }
        }
    }

    String::from_utf8(result).map_err(|e| e.to_string())
}

/// Initialize the std/url module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt parse_url
    // @module std/url
    // @module_description URL parsing, encoding, and query string handling
    // @signature parse_url(url: String) -> Result<Map, String>
    // Parses a URL into its components: scheme, host, port, path, query, fragment.
    //
    // Also extracts username/password from auth URLs and parses query parameters
    // into a nested params map. The original URL is preserved as href.
    // @param url The URL string to parse
    // @returns Result containing a map of URL components
    // @see_also build_query, parse_query
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example parse_url("https://example.com/path?q=1") ~ "Parse URL into components map"
    // @error TypeError ~ "parse() requires a URL string" fix: "Pass a string"
    module.insert(
        "parse_url".to_string(),
        Value::NativeFunction {
            name: "parse_url".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                match &args[0] {
                    Value::String(url_str) => {
                        // Simple URL parser
                        let mut result = HashMap::new();
                        let url = url_str.as_str();

                        // Extract scheme
                        let (scheme, rest) = if let Some(pos) = url.find("://") {
                            (Some(&url[..pos]), &url[pos + 3..])
                        } else {
                            (None, url)
                        };

                        if let Some(s) = scheme {
                            result.insert("scheme".to_string(), Value::String(s.to_string()));
                        }

                        // Extract fragment
                        let (rest, fragment) = if let Some(pos) = rest.find('#') {
                            (&rest[..pos], Some(&rest[pos + 1..]))
                        } else {
                            (rest, None)
                        };

                        if let Some(f) = fragment {
                            result.insert("fragment".to_string(), Value::String(f.to_string()));
                        }

                        // Extract query
                        let (rest, query) = if let Some(pos) = rest.find('?') {
                            (&rest[..pos], Some(&rest[pos + 1..]))
                        } else {
                            (rest, None)
                        };

                        if let Some(q) = query {
                            result.insert("query".to_string(), Value::String(q.to_string()));

                            // Parse query parameters
                            let mut params = HashMap::new();
                            for pair in q.split('&') {
                                if let Some(eq_pos) = pair.find('=') {
                                    let key = &pair[..eq_pos];
                                    let value = &pair[eq_pos + 1..];
                                    params
                                        .insert(key.to_string(), Value::String(value.to_string()));
                                } else if !pair.is_empty() {
                                    params.insert(pair.to_string(), Value::String("".to_string()));
                                }
                            }
                            result.insert("params".to_string(), Value::Map(params));
                        }

                        // Extract host and path
                        let (host_part, path) = if let Some(pos) = rest.find('/') {
                            (&rest[..pos], &rest[pos..])
                        } else {
                            (rest, "")
                        };

                        // Extract port from host
                        let (host, port) = if let Some(pos) = host_part.rfind(':') {
                            let potential_port = &host_part[pos + 1..];
                            if potential_port.chars().all(|c| c.is_ascii_digit()) {
                                (&host_part[..pos], potential_port.parse::<i64>().ok())
                            } else {
                                (host_part, None)
                            }
                        } else {
                            (host_part, None)
                        };

                        // Extract username:password from host
                        let (auth, host) = if let Some(pos) = host.find('@') {
                            (Some(&host[..pos]), &host[pos + 1..])
                        } else {
                            (None, host)
                        };

                        if let Some(a) = auth {
                            if let Some(colon) = a.find(':') {
                                result.insert(
                                    "username".to_string(),
                                    Value::String(a[..colon].to_string()),
                                );
                                result.insert(
                                    "password".to_string(),
                                    Value::String(a[colon + 1..].to_string()),
                                );
                            } else {
                                result.insert("username".to_string(), Value::String(a.to_string()));
                            }
                        }

                        if !host.is_empty() {
                            result.insert("host".to_string(), Value::String(host.to_string()));
                        }

                        if let Some(p) = port {
                            result.insert("port".to_string(), Value::Int(p));
                        }

                        if !path.is_empty() {
                            result.insert("path".to_string(), Value::String(path.to_string()));
                        }

                        result.insert("href".to_string(), Value::String(url_str.clone()));

                        Ok(Value::ok(Value::Map(result)))
                    }
                    _ => Err(IntentError::type_error(
                        "parse() requires a URL string".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt encode
    // @module std/url
    // @signature encode(s: String) -> String
    // URL-encodes a string, preserving URL-safe characters.
    //
    // Preserves characters that are safe in URLs (slashes, colons, etc.)
    // while encoding spaces and other special characters. For encoding
    // query parameter values, use encode_component instead.
    // @param s The string to encode
    // @returns URL-encoded string
    // @see_also decode, encode_component
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example encode("hello world") => "hello%20world" ~ "Encode spaces"
    module.insert(
        "encode".to_string(),
        Value::NativeFunction {
            name: "encode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let encoded = url_encode(s);
                    Ok(Value::String(encoded))
                }
                _ => Err(IntentError::type_error(
                    "encode() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt encode_component
    // @module std/url
    // @signature encode_component(s: String) -> String
    // URL-encodes a string component aggressively, safe for query parameters.
    //
    // Unlike encode(), this encodes all non-alphanumeric characters except
    // hyphens, underscores, periods, and tildes. Use this for query parameter
    // keys and values.
    // @param s The string to encode
    // @returns Aggressively URL-encoded string
    // @see_also encode, decode, build_query
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example encode_component("a=b&c=d") => "a%3Db%26c%3Dd" ~ "Encode special chars"
    module.insert(
        "encode_component".to_string(),
        Value::NativeFunction {
            name: "encode_component".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(s) => {
                    let encoded = url_encode_component(s);
                    Ok(Value::String(encoded))
                }
                _ => Err(IntentError::type_error(
                    "encode_component() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt decode
    // @module std/url
    // @signature decode(s: String) -> Result<String, String>
    // URL-decodes a percent-encoded string.
    //
    // Converts %XX hex sequences back to characters and + signs to spaces.
    // Returns Err if the string contains invalid percent encoding.
    // @param s The URL-encoded string to decode
    // @returns Result containing the decoded string or an error
    // @see_also encode, encode_component
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example decode("hello%20world") => Ok("hello world") ~ "Decode percent-encoded spaces"
    module.insert(
        "decode".to_string(),
        Value::NativeFunction {
            name: "decode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(s) => match url_decode(s) {
                    Ok(decoded) => Ok(Value::ok(Value::String(decoded))),
                    Err(e) => Ok(Value::err(Value::String(e))),
                },
                _ => Err(IntentError::type_error(
                    "decode() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt build_query
    // @module std/url
    // @signature build_query(params: Map) -> String
    // Builds a URL query string from a map of key-value pairs.
    //
    // Keys and values are URL-encoded using component encoding. Pairs are
    // joined with & separators.
    // @param params Map of query parameter names to values
    // @returns Query string like "key1=value1&key2=value2"
    // @see_also parse_query, encode_component
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example build_query(map { "a": "1", "b": "2" }) => "a=1&b=2" ~ "Map to query string"
    module.insert(
        "build_query".to_string(),
        Value::NativeFunction {
            name: "build_query".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Map(params) => {
                    let pairs: Vec<String> = params
                        .iter()
                        .map(|(k, v)| {
                            let key = url_encode_component(k);
                            let value = url_encode_component(&v.to_string());
                            format!("{}={}", key, value)
                        })
                        .collect();
                    Ok(Value::String(pairs.join("&")))
                }
                _ => Err(IntentError::type_error(
                    "build_query() requires a map".to_string(),
                )),
            },
        },
    );

    // @ntnt parse_query
    // @module std/url
    // @signature parse_query(query: String) -> Map<String, String>
    // Parses a URL query string into a map of key-value pairs.
    //
    // Splits on & separators and = key-value delimiters. Both keys and values
    // are URL-decoded. Keys without values get empty string values.
    // @param query The query string to parse (without leading ?)
    // @returns Map of decoded query parameters
    // @see_also build_query, parse_url
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example parse_query("a=1&b=2") => map { "a": "1", "b": "2" } ~ "Query string to map"
    module.insert(
        "parse_query".to_string(),
        Value::NativeFunction {
            name: "parse_query".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                match &args[0] {
                    Value::String(query) => {
                        let mut result: HashMap<String, Value> = HashMap::new();

                        if !query.is_empty() {
                            for pair in query.split('&') {
                                if let Some((key, value)) = pair.split_once('=') {
                                    // URL decode both key and value
                                    let decoded_key =
                                        url_decode(key).unwrap_or_else(|_| key.to_string());
                                    let decoded_value =
                                        url_decode(value).unwrap_or_else(|_| value.to_string());
                                    result.insert(decoded_key, Value::String(decoded_value));
                                } else if !pair.is_empty() {
                                    // Handle keys without values (e.g., "flag" in "flag&name=value")
                                    let decoded_key =
                                        url_decode(pair).unwrap_or_else(|_| pair.to_string());
                                    result.insert(decoded_key, Value::String(String::new()));
                                }
                            }
                        }

                        Ok(Value::Map(result))
                    }
                    _ => Err(IntentError::type_error(
                        "parse_query() requires a string".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt join_url
    // @module std/url
    // @signature join_url(base: String, path: String) -> String
    // Joins a base URL with a path, handling trailing/leading slashes.
    //
    // Trims trailing slashes from the base and leading slashes from the path,
    // then joins them with a single slash. Renamed from join() to avoid
    // ambiguity with join() in std/string and std/path.
    // @param base The base URL
    // @param path The path to append
    // @returns Combined URL string
    // @since v0.4.0
    // @tags #pure, #deterministic
    // @example join_url("https://example.com", "/api/v1") => "https://example.com/api/v1" ~ "Join base and path"
    module.insert(
        "join_url".to_string(),
        Value::NativeFunction {
            name: "join_url".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(base), Value::String(path)) => {
                    let base = base.trim_end_matches('/');
                    let path = path.trim_start_matches('/');
                    Ok(Value::String(format!("{}/{}", base, path)))
                }
                _ => Err(IntentError::type_error(
                    "join_url() requires two strings".to_string(),
                )),
            },
        },
    );

    // @ntnt join
    // @module std/url
    // @signature join(base: String, path: String) -> String
    // Deprecated: use join_url() instead. Alias for backward compatibility.
    // @param base The base URL
    // @param path The path to append
    // @returns Combined URL string
    // @since v0.2.0
    // @tags #pure, #deterministic, #deprecated
    // @example join("https://example.com", "/api") => "https://example.com/api" ~ "Deprecated: use join_url()"
    module.insert(
        "join".to_string(),
        Value::NativeFunction {
            name: "join".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                eprintln!("[DEPRECATED] join() in std/url is deprecated. Use join_url() instead.");
                match (&args[0], &args[1]) {
                    (Value::String(base), Value::String(path)) => {
                        let base = base.trim_end_matches('/');
                        let path = path.trim_start_matches('/');
                        Ok(Value::String(format!("{}/{}", base, path)))
                    }
                    _ => Err(IntentError::type_error(
                        "join() requires two strings".to_string(),
                    )),
                }
            },
        },
    );

    module
}
