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
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

// =============================================================================
// SSRF Protection Configuration
// =============================================================================

/// SSRF protection configuration loaded from environment variables
#[derive(Debug, Clone)]
struct SsrfConfig {
    /// Whether SSRF protection is enabled (default: true)
    enabled: bool,
    /// Allow localhost requests (default: false in production, true in development)
    allow_localhost: bool,
    /// Allow private IP ranges (default: false)
    allow_private: bool,
    /// Additional blocked hosts (comma-separated in env var)
    blocked_hosts: Vec<String>,
}

impl Default for SsrfConfig {
    fn default() -> Self {
        let production_mode = std::env::var("NTNT_ENV")
            .map(|v| v == "production" || v == "prod")
            .unwrap_or(false);

        SsrfConfig {
            enabled: std::env::var("NTNT_SSRF_PROTECTION")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(true), // Enabled by default
            allow_localhost: std::env::var("NTNT_ALLOW_LOCALHOST")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(!production_mode), // Allow in dev, block in prod
            allow_private: std::env::var("NTNT_ALLOW_PRIVATE_IPS")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(false), // Blocked by default
            blocked_hosts: std::env::var("NTNT_BLOCKED_HOSTS")
                .map(|v| v.split(',').map(|s| s.trim().to_lowercase()).collect())
                .unwrap_or_default(),
        }
    }
}

/// Global SSRF configuration (loaded once from environment)
static SSRF_CONFIG: OnceLock<SsrfConfig> = OnceLock::new();

fn get_ssrf_config() -> &'static SsrfConfig {
    SSRF_CONFIG.get_or_init(SsrfConfig::default)
}

/// Check if an IP address is private/internal
fn is_private_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            // Loopback (127.0.0.0/8)
            ipv4.is_loopback()
            // Private ranges
            || ipv4.is_private()
            // Link-local (169.254.0.0/16) - includes AWS metadata endpoint
            || ipv4.is_link_local()
            // Documentation (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24)
            || is_documentation_ipv4(ipv4)
            // Broadcast
            || ipv4.is_broadcast()
            // Unspecified (0.0.0.0)
            || ipv4.is_unspecified()
        }
        IpAddr::V6(ipv6) => {
            // Loopback (::1)
            ipv6.is_loopback()
            // Unspecified (::)
            || ipv6.is_unspecified()
            // IPv4-mapped addresses (check the embedded IPv4)
            || ipv6.to_ipv4_mapped().map(|v4| is_private_ip(&IpAddr::V4(v4))).unwrap_or(false)
            // Unique local (fc00::/7)
            || is_unique_local_ipv6(ipv6)
            // Link-local (fe80::/10)
            || is_link_local_ipv6(ipv6)
        }
    }
}

fn is_documentation_ipv4(ip: &Ipv4Addr) -> bool {
    let octets = ip.octets();
    // TEST-NET-1: 192.0.2.0/24
    (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
    // TEST-NET-2: 198.51.100.0/24
    || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
    // TEST-NET-3: 203.0.113.0/24
    || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
}

fn is_unique_local_ipv6(ip: &Ipv6Addr) -> bool {
    // fc00::/7 - unique local addresses
    let segments = ip.segments();
    (segments[0] & 0xfe00) == 0xfc00
}

fn is_link_local_ipv6(ip: &Ipv6Addr) -> bool {
    // fe80::/10 - link-local addresses
    let segments = ip.segments();
    (segments[0] & 0xffc0) == 0xfe80
}

/// Validate a URL for SSRF protection
/// Returns Ok(()) if the URL is safe to fetch, or Err with a description if blocked
fn validate_url_for_ssrf(url: &str) -> std::result::Result<(), String> {
    let config = get_ssrf_config();

    // If protection is disabled, allow everything
    if !config.enabled {
        return Ok(());
    }

    // Parse the URL
    let parsed = match reqwest::Url::parse(url) {
        Ok(u) => u,
        Err(_) => return Err("Invalid URL format".to_string()),
    };

    // Only allow http and https schemes
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => return Err(format!("Blocked URL scheme: {}", scheme)),
    }

    // Get the host
    let host = match parsed.host_str() {
        Some(h) => h.to_lowercase(),
        None => return Err("URL has no host".to_string()),
    };

    // Check blocked hosts list
    if config
        .blocked_hosts
        .iter()
        .any(|blocked| host == *blocked || host.ends_with(&format!(".{}", blocked)))
    {
        return Err(format!("Host '{}' is blocked", host));
    }

    // Block cloud metadata endpoints explicitly (common SSRF targets)
    let metadata_hosts = [
        "169.254.169.254",          // AWS, GCP, Azure metadata
        "metadata.google.internal", // GCP
        "metadata.goog",            // GCP
        "169.254.170.2",            // AWS ECS task metadata
        "fd00:ec2::254",            // AWS IPv6 metadata
    ];
    if metadata_hosts.iter().any(|&m| host == m) {
        return Err("Cloud metadata endpoint blocked for security".to_string());
    }

    // Resolve the hostname to IP addresses and check each one
    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let socket_addrs = format!("{}:{}", host, port);

    // Try to resolve DNS
    let addrs: Vec<IpAddr> = match socket_addrs.to_socket_addrs() {
        Ok(iter) => iter.map(|s| s.ip()).collect(),
        Err(_) => {
            // DNS resolution failed - might be a blocked internal hostname
            // In production, block; in dev with allow_localhost, permit
            if config.allow_localhost {
                return Ok(());
            }
            return Err(format!("Could not resolve hostname: {}", host));
        }
    };

    // Check all resolved IPs
    for ip in &addrs {
        let is_loopback = ip.is_loopback();
        let is_private = is_private_ip(ip);

        // Check localhost
        if is_loopback && !config.allow_localhost {
            return Err(format!(
                "Localhost requests blocked ({}). Set NTNT_ALLOW_LOCALHOST=true to allow.",
                ip
            ));
        }

        // Check private IPs
        if is_private && !is_loopback && !config.allow_private {
            return Err(format!(
                "Private IP address blocked ({}). Set NTNT_ALLOW_PRIVATE_IPS=true to allow.",
                ip
            ));
        }
    }

    Ok(())
}

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
                return Ok(Value::ok(resp_value));
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

/// Maximum response body size (50MB default, configurable via NTNT_MAX_RESPONSE_SIZE)
/// @since v0.3.14
fn max_response_size() -> usize {
    static MAX_SIZE: OnceLock<usize> = OnceLock::new();
    *MAX_SIZE.get_or_init(|| {
        std::env::var("NTNT_MAX_RESPONSE_SIZE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50 * 1024 * 1024) // 50MB default
    })
}

/// Read response body with size limit to prevent memory exhaustion
/// @since v0.3.14
fn read_response_body_limited(
    response: reqwest::blocking::Response,
) -> std::result::Result<(String, u16, reqwest::header::HeaderMap, String), String> {
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let final_url = response.url().to_string();

    // Check Content-Length header first for early rejection
    if let Some(content_length) = response.content_length() {
        if content_length as usize > max_response_size() {
            return Err(format!(
                "Response too large: {} bytes (max: {} bytes)",
                content_length,
                max_response_size()
            ));
        }
    }

    match response.text() {
        Ok(body) => {
            if body.len() > max_response_size() {
                Err(format!(
                    "Response body too large: {} bytes (max: {} bytes)",
                    body.len(),
                    max_response_size()
                ))
            } else {
                Ok((body, status, headers, final_url))
            }
        }
        Err(e) => Err(format!("Failed to read response body: {}", e)),
    }
}

/// Simple HTTP GET request
fn http_get(url: &str) -> Result<Value> {
    // Cancellation yield point (rule 19): check before making the network request
    if crate::stdlib::concurrent::is_current_task_cancelled() {
        return Err(IntentError::runtime_error("Task cancelled".to_string()));
    }

    // SSRF protection: validate URL before making request
    if let Err(reason) = validate_url_for_ssrf(url) {
        return Ok(Value::err(Value::String(format!(
            "SSRF protection: {}",
            reason
        ))));
    }

    let client = reqwest::blocking::Client::new();
    match client.get(url).send() {
        Ok(response) => match read_response_body_limited(response) {
            Ok((body, status, headers, final_url)) => {
                let resp_value = response_to_value(status, &headers, body, &final_url, url);
                Ok(Value::ok(resp_value))
            }
            Err(e) => Ok(Value::err(Value::String(e))),
        },
        Err(e) => Ok(Value::err(Value::String(format!(
            "HTTP request failed: {}",
            e
        )))),
    }
}

/// Full HTTP request with all options
fn http_fetch(opts: &HashMap<String, Value>) -> Result<Value> {
    // Cancellation yield point (rule 19): check before making the network request
    if crate::stdlib::concurrent::is_current_task_cancelled() {
        return Err(IntentError::runtime_error("Task cancelled".to_string()));
    }

    let url = match opts.get("url") {
        Some(Value::String(u)) => u.clone(),
        _ => {
            return Err(IntentError::type_error(
                "fetch() requires 'url' option".to_string(),
            ))
        }
    };

    // SSRF protection: validate URL before making request
    if let Err(reason) = validate_url_for_ssrf(&url) {
        return Ok(Value::err(Value::String(format!(
            "SSRF protection: {}",
            reason
        ))));
    }

    let method = match opts.get("method") {
        Some(Value::String(m)) => m.to_uppercase(),
        _ => "GET".to_string(),
    };

    // Build client with cookie store
    let client = reqwest::blocking::Client::builder()
        .cookie_store(true)
        .build()
        .map_err(|e| IntentError::runtime_error(format!("Failed to create HTTP client: {}", e)))?;

    let mut request = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        "HEAD" => client.head(&url),
        _ => {
            return Err(IntentError::runtime_error(format!(
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
                    Ok(Value::ok(resp_value))
                }
                Err(e) => Ok(Value::err(Value::String(format!(
                    "Failed to read response body: {}",
                    e
                )))),
            }
        }
        Err(e) => Ok(Value::err(Value::String(format!(
            "HTTP request failed: {}",
            e
        )))),
    }
}

/// Download a file from URL
fn http_download(url: &str, file_path: &str) -> Result<Value> {
    // SSRF protection: validate URL before making request
    if let Err(reason) = validate_url_for_ssrf(url) {
        return Ok(Value::err(Value::String(format!(
            "SSRF protection: {}",
            reason
        ))));
    }

    let path = Path::new(file_path);

    // Create parent directories if they don't exist
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                IntentError::runtime_error(format!("Failed to create directory: {}", e))
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

                                Ok(Value::ok(Value::Map(result_map)))
                            }
                            Err(e) => Ok(Value::err(Value::String(format!(
                                "Failed to write file: {}",
                                e
                            )))),
                        },
                        Err(e) => Ok(Value::err(Value::String(format!(
                            "Failed to create file: {}",
                            e
                        )))),
                    },
                    Err(e) => Ok(Value::err(Value::String(format!(
                        "Failed to read response: {}",
                        e
                    )))),
                }
            } else {
                Ok(Value::err(Value::String(format!(
                    "HTTP error: status {}",
                    status
                ))))
            }
        }
        Err(e) => Ok(Value::err(Value::String(format!(
            "HTTP request failed: {}",
            e
        )))),
    }
}

/// Initialize the std/http module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt fetch
    // @module std/http
    // @module_description HTTP client for making requests to external services
    // @signature fetch(url_or_options: String | Map, options?: Map) -> Result<Response, String>
    // Make an HTTP request to a URL.
    //
    // Accepts one or two arguments:
    // - One argument: a URL string for a simple GET request, or an options map
    //   with full control over method, headers, body, authentication, cookies, and timeout.
    // - Two arguments: a URL string and an options map. The URL is merged into
    //   the options map automatically.
    // Options map keys: url (set automatically in 2-arg form), method, headers, body, json, form, auth, cookies, timeout.
    // @param url_or_options A URL string for GET, or a Map with request options
    // @param options (optional) A Map with request options when first argument is a URL string
    // @returns Result<Response, String> where Response is a Map with status, status_text, headers, body, ok, url, redirected, and cookies fields
    // @see_also download, cache_fetch
    // @since v0.1.0
    // @tags #network
    // @example fetch("https://api.example.com/data") => Ok({status: 200, body: "...", ...}) ~ "Simple GET request"
    // @example ~ "POST with JSON body (1-arg form)"
    //   let opts = map {
    //     "url": "https://api.example.com",
    //     "method": "POST",
    //     "json": map { "key": "value" }
    //   }
    //   fetch(opts)
    // @expected Ok({status: 201, ...})
    // @example ~ "POST with JSON body (2-arg form)"
    //   fetch("https://api.example.com", map {
    //     "method": "POST",
    //     "json": map { "key": "value" }
    //   })
    // @expected Ok({status: 201, ...})
    // @error TypeError ~ "fetch() requires a URL string or options map" fix: "Pass a String URL or a Map with request options"
    // @error TypeError ~ "fetch() requires 'url' option" fix: "Include 'url' key in the options map"
    // @error RuntimeError ~ "Unsupported HTTP method: ..." fix: "Use GET, POST, PUT, DELETE, PATCH, or HEAD"
    module.insert(
        "fetch".to_string(),
        Value::NativeFunction {
            name: "fetch".to_string(),
            arity: 1,
            max_arity: 2,
            func: |args| {
                if args.len() == 2 {
                    // Two-arg form: fetch(url, options)
                    match (&args[0], &args[1]) {
                        (Value::String(url), Value::Map(opts)) => {
                            let mut merged = opts.clone();
                            merged.insert("url".to_string(), Value::String(url.clone()));
                            http_fetch(&merged)
                        }
                        _ => Err(IntentError::type_error(
                            "fetch(url, options) requires a String URL and an options Map"
                                .to_string(),
                        )),
                    }
                } else {
                    // One-arg form: fetch(url) or fetch(options)
                    match &args[0] {
                        Value::String(url) => http_get(url),
                        Value::Map(opts) => http_fetch(opts),
                        _ => Err(IntentError::type_error(
                            "fetch() requires a URL string or options map".to_string(),
                        )),
                    }
                }
            },
        },
    );

    // @ntnt download
    // @module std/http
    // @signature download(url: String, file_path: String) -> Result<Map, String>
    // Download a file from a URL and save it to disk.
    //
    // Fetches the resource at the given URL and writes the response bytes to
    // the specified file path. Parent directories are created automatically
    // if they do not exist. Returns a map with status, path, and size on success.
    // @param url The URL of the file to download
    // @param file_path The local file path to save the downloaded content
    // @returns Result<Map{status: Int, path: String, size: Int}, String> on success; Err with message on failure
    // @see_also fetch
    // @since v0.1.0
    // @tags #network
    // @example download("https://example.com/file.zip", "./file.zip") => Ok({status: 200, path: "./file.zip", size: 1024}) ~ "Download a file"
    // @error TypeError ~ "download() requires URL string and file path string" fix: "Pass two String arguments: URL and file path"
    // @error RuntimeError ~ "Failed to create directory: ..." fix: "Ensure the parent directory path is valid and writable"
    // @error RuntimeError ~ "Failed to create file: ..." fix: "Ensure the file path is valid and writable"
    // @error RuntimeError ~ "HTTP error: status ..." fix: "Check the URL and server availability"
    module.insert(
        "download".to_string(),
        Value::NativeFunction {
            name: "download".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(url), Value::String(file_path)) => http_download(url, file_path),
                _ => Err(IntentError::type_error(
                    "download() requires URL string and file path string".to_string(),
                )),
            },
        },
    );

    // @ntnt Cache
    // @module std/http
    // @signature Cache(ttl_seconds: Int) -> Map
    // Create a response cache with a time-to-live (TTL) in seconds.
    //
    // Returns a cache object (Map) that can be used with cache_fetch, cache_delete,
    // and cache_clear to cache HTTP responses. Cached entries automatically expire
    // after the specified TTL. The cache object contains an internal _cache_id field.
    // @param ttl_seconds The default time-to-live for cached entries, in seconds
    // @returns Map containing a _cache_id field for use with cache helper functions
    // @see_also cache_fetch, cache_delete, cache_clear
    // @since v0.1.0
    // @tags #network
    // @example Cache(300) => {_cache_id: 1} ~ "Create a cache with 5-minute TTL"
    // @error TypeError ~ "Cache() requires TTL in seconds (integer)" fix: "Pass an Int value for the TTL"
    module.insert(
        "Cache".to_string(),
        Value::NativeFunction {
            name: "Cache".to_string(),
            arity: 1,
            max_arity: 1,
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
                _ => Err(IntentError::type_error(
                    "Cache() requires TTL in seconds (integer)".to_string(),
                )),
            },
        },
    );

    // @ntnt cache_fetch
    // @module std/http
    // @signature cache_fetch(cache_obj: Map, url_or_options: String | Map) -> Result<Response, String>
    // Fetch a URL using a cache, returning a cached response if available.
    //
    // Checks the cache for a previously stored response matching the URL.
    // On a cache miss, performs the HTTP request via fetch(), stores the
    // successful response in the cache, and returns it. This is the internal
    // function backing cache.fetch() method calls.
    // @param cache_obj A cache object created by Cache()
    // @param url_or_options A URL string or options Map (must include 'url' key)
    // @returns Result<Response, String> with the HTTP response (from cache or network)
    // @see_also Cache, cache_delete, cache_clear, fetch
    // @since v0.1.0
    // @tags #network
    // @example cache_fetch(my_cache, "https://api.example.com/data") => Ok({status: 200, ...}) ~ "Fetch with caching"
    // @error TypeError ~ "Invalid cache object" fix: "Pass a cache object created by Cache()"
    // @error TypeError ~ "Expected cache object" fix: "First argument must be a Map with _cache_id"
    // @error TypeError ~ "Options must include 'url'" fix: "Include 'url' key in the options map"
    // @error TypeError ~ "cache.fetch() requires URL string or options map" fix: "Pass a String URL or a Map with request options"
    module.insert(
        "cache_fetch".to_string(),
        Value::NativeFunction {
            name: "cache_fetch".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| {
                let cache_id = match &args[0] {
                    Value::Map(m) => match m.get("_cache_id") {
                        Some(Value::Int(id)) => *id as u64,
                        _ => {
                            return Err(IntentError::type_error("Invalid cache object".to_string()))
                        }
                    },
                    _ => return Err(IntentError::type_error("Expected cache object".to_string())),
                };

                match &args[1] {
                    Value::String(url) => cache_fetch(cache_id, url, None),
                    Value::Map(opts) => {
                        let url = match opts.get("url") {
                            Some(Value::String(u)) => u.clone(),
                            _ => {
                                return Err(IntentError::type_error(
                                    "Options must include 'url'".to_string(),
                                ))
                            }
                        };
                        cache_fetch(cache_id, &url, Some(opts))
                    }
                    _ => Err(IntentError::type_error(
                        "cache.fetch() requires URL string or options map".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt cache_delete
    // @module std/http
    // @signature cache_delete(cache_obj: Map, url: String) -> Unit
    // Remove a cached response for a specific URL.
    //
    // Evicts the cached entry for the given URL from the cache, if present.
    // This is the internal function backing cache.delete() method calls.
    // @param cache_obj A cache object created by Cache()
    // @param url The URL whose cached response should be removed
    // @returns Unit
    // @see_also Cache, cache_fetch, cache_clear
    // @since v0.1.0
    // @tags #network
    // @example cache_delete(my_cache, "https://api.example.com/data") => () ~ "Invalidate a cached URL"
    // @error TypeError ~ "Invalid cache object" fix: "Pass a cache object created by Cache()"
    // @error TypeError ~ "Expected cache object" fix: "First argument must be a Map with _cache_id"
    // @error TypeError ~ "cache.delete() requires URL string" fix: "Pass a String URL as the second argument"
    module.insert(
        "cache_delete".to_string(),
        Value::NativeFunction {
            name: "cache_delete".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| {
                let cache_id = match &args[0] {
                    Value::Map(m) => match m.get("_cache_id") {
                        Some(Value::Int(id)) => *id as u64,
                        _ => {
                            return Err(IntentError::type_error("Invalid cache object".to_string()))
                        }
                    },
                    _ => return Err(IntentError::type_error("Expected cache object".to_string())),
                };

                if let Value::String(url) = &args[1] {
                    cache_delete(cache_id, url);
                    Ok(Value::Unit)
                } else {
                    Err(IntentError::type_error(
                        "cache.delete() requires URL string".to_string(),
                    ))
                }
            },
        },
    );

    // @ntnt cache_clear
    // @module std/http
    // @signature cache_clear(cache_obj: Map) -> Unit
    // Remove all cached responses from a cache.
    //
    // Evicts every entry from the specified cache object, regardless of TTL.
    // This is the internal function backing cache.clear() method calls.
    // @param cache_obj A cache object created by Cache()
    // @returns Unit
    // @see_also Cache, cache_fetch, cache_delete
    // @since v0.1.0
    // @tags #network
    // @example cache_clear(my_cache) => () ~ "Clear all cached responses"
    // @error TypeError ~ "Invalid cache object" fix: "Pass a cache object created by Cache()"
    // @error TypeError ~ "Expected cache object" fix: "First argument must be a Map with _cache_id"
    module.insert(
        "cache_clear".to_string(),
        Value::NativeFunction {
            name: "cache_clear".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                let cache_id = match &args[0] {
                    Value::Map(m) => match m.get("_cache_id") {
                        Some(Value::Int(id)) => *id as u64,
                        _ => {
                            return Err(IntentError::type_error("Invalid cache object".to_string()))
                        }
                    },
                    _ => return Err(IntentError::type_error("Expected cache object".to_string())),
                };

                cache_clear(cache_id);
                Ok(Value::Unit)
            },
        },
    );

    module
}
