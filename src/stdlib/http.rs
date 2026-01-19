//! std/http module - HTTP client for making requests
//!
//! # API
//!
//! - `fetch(url)` - Simple GET request
//! - `fetch(options)` - Full request with options map
//! - `download(url, path)` - Download file to disk
//! - `Cache(ttl)` - Create a response cache
//!
//! # Options for fetch()
//!
//! - `url`: Request URL (required when using options map)
//! - `method`: HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD)
//! - `headers`: Map of headers
//! - `body`: Raw body string
//! - `json`: JSON body (auto-serializes and sets Content-Type)
//! - `form`: Form data (URL-encoded)
//! - `auth`: Map with `user` and `pass` for Basic auth
//! - `cookies`: Map of cookies to send
//! - `timeout`: Timeout in seconds (default: 30)

use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::json::intent_value_to_json;
use base64::Engine;
use reqwest::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

/// Cached raw response data (thread-safe, no Value references)
#[derive(Clone)]
struct CachedResponse {
    status: u16,
    status_text: String,
    headers: HashMap<String, String>,
    body: String,
    url: String,
    redirected: bool,
    cookies: HashMap<String, String>,
}

/// Cached response entry
struct CacheEntry {
    response: CachedResponse,
    expires_at: Instant,
}

/// Response cache with TTL
struct ResponseCache {
    entries: HashMap<String, CacheEntry>,
    default_ttl: Duration,
}

impl ResponseCache {
    fn new(ttl_seconds: u64) -> Self {
        ResponseCache {
            entries: HashMap::new(),
            default_ttl: Duration::from_secs(ttl_seconds),
        }
    }

    fn get(&mut self, key: &str) -> Option<CachedResponse> {
        if let Some(entry) = self.entries.get(key) {
            if Instant::now() < entry.expires_at {
                return Some(entry.response.clone());
            }
            // Expired - remove it
            self.entries.remove(key);
        }
        None
    }

    fn set(&mut self, key: String, response: CachedResponse, ttl: Option<Duration>) {
        let ttl = ttl.unwrap_or(self.default_ttl);
        self.entries.insert(
            key,
            CacheEntry {
                response,
                expires_at: Instant::now() + ttl,
            },
        );
    }

    fn delete(&mut self, key: &str) {
        self.entries.remove(key);
    }

    fn clear(&mut self) {
        self.entries.clear();
    }
}

// Global cache registry - stores caches by ID
lazy_static::lazy_static! {
    static ref CACHE_REGISTRY: Mutex<HashMap<u64, ResponseCache>> = Mutex::new(HashMap::new());
    static ref CACHE_COUNTER: Mutex<u64> = Mutex::new(0);
}

fn get_next_cache_id() -> u64 {
    let mut counter = CACHE_COUNTER.lock().unwrap();
    *counter += 1;
    *counter
}

/// Convert CachedResponse to Value
fn cached_response_to_value(resp: &CachedResponse) -> Value {
    let mut response_map = HashMap::new();

    response_map.insert("status".to_string(), Value::Int(resp.status as i64));
    response_map.insert(
        "status_text".to_string(),
        Value::String(resp.status_text.clone()),
    );

    let mut headers_map = HashMap::new();
    for (k, v) in &resp.headers {
        headers_map.insert(k.clone(), Value::String(v.clone()));
    }
    response_map.insert("headers".to_string(), Value::Map(headers_map));

    response_map.insert("body".to_string(), Value::String(resp.body.clone()));
    response_map.insert(
        "ok".to_string(),
        Value::Bool((200..300).contains(&resp.status)),
    );
    response_map.insert("url".to_string(), Value::String(resp.url.clone()));
    response_map.insert("redirected".to_string(), Value::Bool(resp.redirected));

    if !resp.cookies.is_empty() {
        let mut cookies_map = HashMap::new();
        for (k, v) in &resp.cookies {
            cookies_map.insert(k.clone(), Value::String(v.clone()));
        }
        response_map.insert("cookies".to_string(), Value::Map(cookies_map));
    }

    Value::Map(response_map)
}

fn cache_fetch(cache_id: u64, url: &str, opts: Option<&HashMap<String, Value>>) -> Result<Value> {
    // Check cache first
    {
        let mut registry = CACHE_REGISTRY.lock().unwrap();
        if let Some(cache) = registry.get_mut(&cache_id) {
            if let Some(cached) = cache.get(url) {
                let resp_value = cached_response_to_value(&cached);
                return Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![resp_value],
                });
            }
        }
    }

    // Fetch from network
    let result = match opts {
        Some(o) => http_fetch(o)?,
        None => http_get(url)?,
    };

    // Cache successful responses
    if let Value::EnumValue {
        variant, values, ..
    } = &result
    {
        if variant == "Ok" && !values.is_empty() {
            if let Value::Map(resp_map) = &values[0] {
                // Extract data from the response Value to create a CachedResponse
                let cached = CachedResponse {
                    status: resp_map
                        .get("status")
                        .and_then(|v| {
                            if let Value::Int(i) = v {
                                Some(*i as u16)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(0),
                    status_text: resp_map
                        .get("status_text")
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default(),
                    headers: resp_map
                        .get("headers")
                        .and_then(|v| {
                            if let Value::Map(m) = v {
                                Some(
                                    m.iter()
                                        .filter_map(|(k, v)| {
                                            if let Value::String(s) = v {
                                                Some((k.clone(), s.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                        .collect(),
                                )
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default(),
                    body: resp_map
                        .get("body")
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default(),
                    url: resp_map
                        .get("url")
                        .and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default(),
                    redirected: resp_map
                        .get("redirected")
                        .and_then(|v| {
                            if let Value::Bool(b) = v {
                                Some(*b)
                            } else {
                                None
                            }
                        })
                        .unwrap_or(false),
                    cookies: resp_map
                        .get("cookies")
                        .and_then(|v| {
                            if let Value::Map(m) = v {
                                Some(
                                    m.iter()
                                        .filter_map(|(k, v)| {
                                            if let Value::String(s) = v {
                                                Some((k.clone(), s.clone()))
                                            } else {
                                                None
                                            }
                                        })
                                        .collect(),
                                )
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default(),
                };

                let mut registry = CACHE_REGISTRY.lock().unwrap();
                if let Some(cache) = registry.get_mut(&cache_id) {
                    cache.set(url.to_string(), cached, None);
                }
            }
        }
    }

    Ok(result)
}

fn cache_delete(cache_id: u64, url: &str) {
    let mut registry = CACHE_REGISTRY.lock().unwrap();
    if let Some(cache) = registry.get_mut(&cache_id) {
        cache.delete(url);
    }
}

fn cache_clear(cache_id: u64) {
    let mut registry = CACHE_REGISTRY.lock().unwrap();
    if let Some(cache) = registry.get_mut(&cache_id) {
        cache.clear();
    }
}

/// Convert reqwest Response to Intent Value
fn response_to_value(
    status: u16,
    headers: &reqwest::header::HeaderMap,
    body: String,
    final_url: &str,
    original_url: &str,
) -> Value {
    let mut response_map = HashMap::new();

    // Status code
    response_map.insert("status".to_string(), Value::Int(status as i64));

    // Status text
    let status_text = match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    };
    response_map.insert(
        "status_text".to_string(),
        Value::String(status_text.to_string()),
    );

    // Headers as a map
    let mut headers_map = HashMap::new();
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            headers_map.insert(name.to_string(), Value::String(v.to_string()));
        }
    }
    response_map.insert("headers".to_string(), Value::Map(headers_map));

    // Body
    response_map.insert("body".to_string(), Value::String(body.clone()));

    // ok flag
    response_map.insert("ok".to_string(), Value::Bool((200..300).contains(&status)));

    // Final URL after redirects
    response_map.insert("url".to_string(), Value::String(final_url.to_string()));

    // Whether the request was redirected
    response_map.insert(
        "redirected".to_string(),
        Value::Bool(final_url != original_url),
    );

    Value::Map(response_map)
}

/// Simple HTTP GET request
fn http_get(url: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    match client.get(url).send() {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            match response.text() {
                Ok(body) => {
                    let resp_value = response_to_value(status, &headers, body, &final_url, url);
                    Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![resp_value],
                    })
                }
                Err(e) => Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Err".to_string(),
                    values: vec![Value::String(format!(
                        "Failed to read response body: {}",
                        e
                    ))],
                }),
            }
        }
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("HTTP request failed: {}", e))],
        }),
    }
}

/// Full HTTP request with all options
fn http_fetch(opts: &HashMap<String, Value>) -> Result<Value> {
    let url = match opts.get("url") {
        Some(Value::String(u)) => u.clone(),
        _ => {
            return Err(IntentError::TypeError(
                "fetch() requires 'url' option".to_string(),
            ))
        }
    };

    let method = match opts.get("method") {
        Some(Value::String(m)) => m.to_uppercase(),
        _ => "GET".to_string(),
    };

    // Build client with cookie store
    let client = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .build()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to create HTTP client: {}", e)))?;

    let mut request = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        "HEAD" => client.head(&url),
        _ => {
            return Err(IntentError::RuntimeError(format!(
                "Unsupported HTTP method: {}",
                method
            )))
        }
    };

    // Add headers
    if let Some(Value::Map(headers)) = opts.get("headers") {
        for (key, value) in headers {
            if let Value::String(v) = value {
                request = request.header(key.as_str(), v.as_str());
            }
        }
    }

    // Add cookies
    if let Some(Value::Map(cookies)) = opts.get("cookies") {
        let cookie_str: Vec<String> = cookies
            .iter()
            .filter_map(|(k, v)| {
                if let Value::String(val) = v {
                    Some(format!("{}={}", k, val))
                } else {
                    None
                }
            })
            .collect();
        if !cookie_str.is_empty() {
            request = request.header(COOKIE, cookie_str.join("; "));
        }
    }

    // Add basic auth
    if let Some(Value::Map(auth)) = opts.get("auth") {
        let username = match auth.get("user") {
            Some(Value::String(u)) => u.clone(),
            _ => String::new(),
        };
        let password = match auth.get("pass") {
            Some(Value::String(p)) => p.clone(),
            _ => String::new(),
        };
        if !username.is_empty() {
            let credentials = format!("{}:{}", username, password);
            let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
            request = request.header(AUTHORIZATION, format!("Basic {}", encoded));
        }
    }

    // Add raw body
    if let Some(Value::String(body)) = opts.get("body") {
        request = request.body(body.clone());
    }

    // Add JSON body
    if let Some(data) = opts.get("json") {
        let json_body = intent_value_to_json(data);
        request = request
            .header("Content-Type", "application/json")
            .body(json_body.to_string());
    }

    // Add form data
    if let Some(Value::Map(form_data)) = opts.get("form") {
        let mut form: Vec<(String, String)> = Vec::new();
        for (key, value) in form_data {
            let string_value = match value {
                Value::String(s) => s.clone(),
                Value::Int(i) => i.to_string(),
                Value::Float(f) => f.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => format!("{:?}", value),
            };
            form.push((key.clone(), string_value));
        }
        request = request.form(&form);
    }

    // Add timeout
    if let Some(Value::Int(timeout)) = opts.get("timeout") {
        request = request.timeout(Duration::from_secs(*timeout as u64));
    }

    // Execute request
    match request.send() {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();

            // Extract cookies from response
            let mut response_cookies = HashMap::new();
            for cookie_header in headers.get_all(SET_COOKIE) {
                if let Ok(cookie_str) = cookie_header.to_str() {
                    if let Some(equals_pos) = cookie_str.find('=') {
                        let name = cookie_str[..equals_pos].to_string();
                        let rest = &cookie_str[equals_pos + 1..];
                        let value = rest.split(';').next().unwrap_or("").to_string();
                        response_cookies.insert(name, Value::String(value));
                    }
                }
            }

            match response.text() {
                Ok(body) => {
                    let mut resp_value =
                        response_to_value(status, &headers, body, &final_url, &url);
                    // Add cookies to response
                    if let Value::Map(ref mut map) = resp_value {
                        if !response_cookies.is_empty() {
                            map.insert("cookies".to_string(), Value::Map(response_cookies));
                        }
                    }
                    Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![resp_value],
                    })
                }
                Err(e) => Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Err".to_string(),
                    values: vec![Value::String(format!(
                        "Failed to read response body: {}",
                        e
                    ))],
                }),
            }
        }
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("HTTP request failed: {}", e))],
        }),
    }
}

/// Download a file from URL
fn http_download(url: &str, file_path: &str) -> Result<Value> {
    let path = Path::new(file_path);

    // Create parent directories if they don't exist
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                IntentError::RuntimeError(format!("Failed to create directory: {}", e))
            })?;
        }
    }

    let client = reqwest::blocking::Client::new();
    match client.get(url).send() {
        Ok(response) => {
            let status = response.status().as_u16();
            if (200..300).contains(&status) {
                match response.bytes() {
                    Ok(bytes) => match File::create(path) {
                        Ok(mut file) => match file.write_all(&bytes) {
                            Ok(_) => {
                                let mut result_map = HashMap::new();
                                result_map.insert("status".to_string(), Value::Int(status as i64));
                                result_map.insert(
                                    "path".to_string(),
                                    Value::String(file_path.to_string()),
                                );
                                result_map
                                    .insert("size".to_string(), Value::Int(bytes.len() as i64));

                                Ok(Value::EnumValue {
                                    enum_name: "Result".to_string(),
                                    variant: "Ok".to_string(),
                                    values: vec![Value::Map(result_map)],
                                })
                            }
                            Err(e) => Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Err".to_string(),
                                values: vec![Value::String(format!("Failed to write file: {}", e))],
                            }),
                        },
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Failed to create file: {}", e))],
                        }),
                    },
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Failed to read response: {}", e))],
                    }),
                }
            } else {
                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Err".to_string(),
                    values: vec![Value::String(format!("HTTP error: status {}", status))],
                })
            }
        }
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("HTTP request failed: {}", e))],
        }),
    }
}

/// Initialize the std/http module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // fetch(url) or fetch(options) -> Result<Response, Error>
    module.insert(
        "fetch".to_string(),
        Value::NativeFunction {
            name: "fetch".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(url) => http_get(url),
                Value::Map(opts) => http_fetch(opts),
                _ => Err(IntentError::TypeError(
                    "fetch() requires a URL string or options map".to_string(),
                )),
            },
        },
    );

    // download(url, file_path) -> Result<{status, path, size}, Error>
    module.insert(
        "download".to_string(),
        Value::NativeFunction {
            name: "download".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(url), Value::String(file_path)) => http_download(url, file_path),
                _ => Err(IntentError::TypeError(
                    "download() requires URL string and file path string".to_string(),
                )),
            },
        },
    );

    // Cache(ttl_seconds) -> Cache object
    // Returns a map with cache_id and methods to call global cache functions
    module.insert(
        "Cache".to_string(),
        Value::NativeFunction {
            name: "Cache".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Int(ttl) => {
                    let cache_id = get_next_cache_id();

                    // Create and register the cache
                    {
                        let mut registry = CACHE_REGISTRY.lock().unwrap();
                        registry.insert(cache_id, ResponseCache::new(*ttl as u64));
                    }

                    // Return a map with the cache_id that methods can use
                    let mut cache_obj: HashMap<String, Value> = HashMap::new();
                    cache_obj.insert("_cache_id".to_string(), Value::Int(cache_id as i64));

                    Ok(Value::Map(cache_obj))
                }
                _ => Err(IntentError::TypeError(
                    "Cache() requires TTL in seconds (integer)".to_string(),
                )),
            },
        },
    );

    // cache_fetch(cache_obj, url_or_options) - internal function for cache.fetch()
    module.insert(
        "cache_fetch".to_string(),
        Value::NativeFunction {
            name: "cache_fetch".to_string(),
            arity: 2,
            func: |args| {
                let cache_id = match &args[0] {
                    Value::Map(m) => match m.get("_cache_id") {
                        Some(Value::Int(id)) => *id as u64,
                        _ => {
                            return Err(IntentError::TypeError("Invalid cache object".to_string()))
                        }
                    },
                    _ => return Err(IntentError::TypeError("Expected cache object".to_string())),
                };

                match &args[1] {
                    Value::String(url) => cache_fetch(cache_id, url, None),
                    Value::Map(opts) => {
                        let url = match opts.get("url") {
                            Some(Value::String(u)) => u.clone(),
                            _ => {
                                return Err(IntentError::TypeError(
                                    "Options must include 'url'".to_string(),
                                ))
                            }
                        };
                        cache_fetch(cache_id, &url, Some(opts))
                    }
                    _ => Err(IntentError::TypeError(
                        "cache.fetch() requires URL string or options map".to_string(),
                    )),
                }
            },
        },
    );

    // cache_delete(cache_obj, url) - internal function for cache.delete()
    module.insert(
        "cache_delete".to_string(),
        Value::NativeFunction {
            name: "cache_delete".to_string(),
            arity: 2,
            func: |args| {
                let cache_id = match &args[0] {
                    Value::Map(m) => match m.get("_cache_id") {
                        Some(Value::Int(id)) => *id as u64,
                        _ => {
                            return Err(IntentError::TypeError("Invalid cache object".to_string()))
                        }
                    },
                    _ => return Err(IntentError::TypeError("Expected cache object".to_string())),
                };

                if let Value::String(url) = &args[1] {
                    cache_delete(cache_id, url);
                    Ok(Value::Unit)
                } else {
                    Err(IntentError::TypeError(
                        "cache.delete() requires URL string".to_string(),
                    ))
                }
            },
        },
    );

    // cache_clear(cache_obj) - internal function for cache.clear()
    module.insert(
        "cache_clear".to_string(),
        Value::NativeFunction {
            name: "cache_clear".to_string(),
            arity: 1,
            func: |args| {
                let cache_id = match &args[0] {
                    Value::Map(m) => match m.get("_cache_id") {
                        Some(Value::Int(id)) => *id as u64,
                        _ => {
                            return Err(IntentError::TypeError("Invalid cache object".to_string()))
                        }
                    },
                    _ => return Err(IntentError::TypeError("Expected cache object".to_string())),
                };

                cache_clear(cache_id);
                Ok(Value::Unit)
            },
        },
    );

    module
}
