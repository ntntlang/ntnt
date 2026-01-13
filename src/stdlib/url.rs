//! std/url module - URL parsing and encoding

use std::collections::HashMap;
use crate::interpreter::Value;
use crate::error::IntentError;

/// URL encode a string (preserves some URL-safe characters)
pub fn url_encode(s: &str) -> String {
    let mut result = String::new();
    for c in s.chars() {
        match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' | '/' | ':' | '?' | '#' | '[' | ']' | '@' | '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '=' => {
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
    
    // parse(url) -> Result<Map, Error> - Parse URL into components
    module.insert("parse".to_string(), Value::NativeFunction {
        name: "parse".to_string(),
        arity: 1,
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
                                params.insert(key.to_string(), Value::String(value.to_string()));
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
                            result.insert("username".to_string(), Value::String(a[..colon].to_string()));
                            result.insert("password".to_string(), Value::String(a[colon + 1..].to_string()));
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
                    
                    Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![Value::Map(result)],
                    })
                }
                _ => Err(IntentError::TypeError("parse() requires a URL string".to_string())),
            }
        },
    });
    
    // encode(str) -> String - URL encode a string
    module.insert("encode".to_string(), Value::NativeFunction {
        name: "encode".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(s) => {
                    let encoded = url_encode(s);
                    Ok(Value::String(encoded))
                }
                _ => Err(IntentError::TypeError("encode() requires a string".to_string())),
            }
        },
    });
    
    // encode_component(str) -> String - URL encode a component (more aggressive encoding)
    module.insert("encode_component".to_string(), Value::NativeFunction {
        name: "encode_component".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(s) => {
                    let encoded = url_encode_component(s);
                    Ok(Value::String(encoded))
                }
                _ => Err(IntentError::TypeError("encode_component() requires a string".to_string())),
            }
        },
    });
    
    // decode(str) -> Result<String, Error> - URL decode a string
    module.insert("decode".to_string(), Value::NativeFunction {
        name: "decode".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(s) => {
                    match url_decode(s) {
                        Ok(decoded) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::String(decoded)],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e)],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError("decode() requires a string".to_string())),
            }
        },
    });
    
    // build_query(params) -> String - Build query string from map
    module.insert("build_query".to_string(), Value::NativeFunction {
        name: "build_query".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Map(params) => {
                    let pairs: Vec<String> = params.iter()
                        .map(|(k, v)| {
                            let key = url_encode_component(k);
                            let value = url_encode_component(&v.to_string());
                            format!("{}={}", key, value)
                        })
                        .collect();
                    Ok(Value::String(pairs.join("&")))
                }
                _ => Err(IntentError::TypeError("build_query() requires a map".to_string())),
            }
        },
    });
    
    // parse_query(query_string) -> Map - Parse query/form string to map (inverse of build_query)
    module.insert("parse_query".to_string(), Value::NativeFunction {
        name: "parse_query".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(query) => {
                    let mut result: HashMap<String, Value> = HashMap::new();
                    
                    if !query.is_empty() {
                        for pair in query.split('&') {
                            if let Some((key, value)) = pair.split_once('=') {
                                // URL decode both key and value
                                let decoded_key = url_decode(key).unwrap_or_else(|_| key.to_string());
                                let decoded_value = url_decode(value).unwrap_or_else(|_| value.to_string());
                                result.insert(decoded_key, Value::String(decoded_value));
                            } else if !pair.is_empty() {
                                // Handle keys without values (e.g., "flag" in "flag&name=value")
                                let decoded_key = url_decode(pair).unwrap_or_else(|_| pair.to_string());
                                result.insert(decoded_key, Value::String(String::new()));
                            }
                        }
                    }
                    
                    Ok(Value::Map(result))
                }
                _ => Err(IntentError::TypeError("parse_query() requires a string".to_string())),
            }
        },
    });
    
    // join(base, path) -> String - Join base URL with path
    module.insert("join".to_string(), Value::NativeFunction {
        name: "join".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::String(base), Value::String(path)) => {
                    let base = base.trim_end_matches('/');
                    let path = path.trim_start_matches('/');
                    Ok(Value::String(format!("{}/{}", base, path)))
                }
                _ => Err(IntentError::TypeError("join() requires two strings".to_string())),
            }
        },
    });
    
    module
}
