//! std/http module - HTTP client for making requests
//!
//! # API
//!
//! - `fetch(url)` - Simple GET request
//! - `fetch(options)` - Full request with options map
//! - `download(url, path)` - Download file to disk
//! - `Cache(ttl)` - Create a response cache
//!
//! Requests containing `Secret` values require HTTPS. `APP_ENV=development`
//! permits direct plain HTTP only for `localhost` and loopback IP addresses.
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
//! - `follow_redirects`: Explicit safe manual following (default: false)
//! - `max_redirects`: Maximum followed edges (default: 5, allowed: 1..10)

use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::json::intent_value_to_json_expose;
use base64::Engine;
use reqwest::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
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

#[derive(Debug)]
struct ValidatedHttpTarget {
    host: String,
    addresses: Vec<SocketAddr>,
}

/// Validate and resolve a URL for SSRF protection.
/// The returned addresses are safe to pin into the HTTP client, closing the gap
/// between validation-time DNS and connection-time DNS.
fn validated_http_target(url: &str) -> std::result::Result<Option<ValidatedHttpTarget>, String> {
    let config = get_ssrf_config();

    // If protection is disabled, allow everything without overriding resolution.
    if !config.enabled {
        return Ok(None);
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

    // Resolve once, validate every address, and return this exact set to the
    // client builder so connection-time DNS cannot rebind the target.
    let addrs: Vec<SocketAddr> = match socket_addrs.to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(_) => return Err(format!("Could not resolve hostname: {}", host)),
    };
    validate_resolved_addresses(config, host, addrs).map(Some)
}

fn validate_resolved_addresses(
    config: &SsrfConfig,
    host: String,
    addrs: Vec<SocketAddr>,
) -> std::result::Result<ValidatedHttpTarget, String> {
    if addrs.is_empty() {
        return Err(format!("Could not resolve hostname: {}", host));
    }

    // Check all resolved IPs.
    for address in &addrs {
        let ip = address.ip();
        let is_loopback = ip.is_loopback();
        let is_private = is_private_ip(&ip);

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

    Ok(ValidatedHttpTarget {
        host,
        addresses: addrs,
    })
}

/// Validate a URL for SSRF protection without retaining its resolved target.
fn validate_url_for_ssrf(url: &str) -> std::result::Result<(), String> {
    validated_http_target(url).map(|_| ())
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

#[derive(Clone, Hash, PartialEq, Eq)]
enum CacheKey {
    Url(String),
    Follow { url: String, limit: usize },
}

/// Response cache with TTL
struct ResponseCache {
    entries: HashMap<CacheKey, CacheEntry>,
    default_ttl: Duration,
}

impl ResponseCache {
    fn new(ttl_seconds: u64) -> Self {
        ResponseCache {
            entries: HashMap::new(),
            default_ttl: Duration::from_secs(ttl_seconds),
        }
    }

    fn get(&mut self, key: &CacheKey) -> Option<CachedResponse> {
        if let Some(entry) = self.entries.get(key) {
            if Instant::now() < entry.expires_at {
                return Some(entry.response.clone());
            }
            // Expired - remove it
            self.entries.remove(key);
        }
        None
    }

    fn set(&mut self, key: CacheKey, response: CachedResponse, ttl: Option<Duration>) {
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
        self.entries.retain(|identity, _| match identity {
            CacheKey::Url(url) | CacheKey::Follow { url, .. } => url != key,
        });
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
    if opts.is_some_and(|options| options.values().any(Value::contains_secret)) {
        return Err(IntentError::type_error(
            "cache_fetch() cannot cache responses for secret-bearing requests; use fetch() directly"
                .to_string(),
        ));
    }

    let policy = opts.map(RedirectPolicy::parse).transpose()?;
    if let Some(policy) = policy {
        if policy.expired()? {
            return Ok(Value::err(Value::String("HTTP chain timeout".into())));
        }
        if policy.enabled {
            if let Err(error) = redirect_url(url) {
                return Ok(Value::err(Value::String(error)));
            }
        }
    }
    let key = match policy {
        Some(policy) if policy.enabled => CacheKey::Follow {
            url: url.to_string(),
            limit: policy.limit,
        },
        _ => CacheKey::Url(url.to_string()),
    };
    // Check cache first
    {
        let mut registry = CACHE_REGISTRY.lock().unwrap();
        if let Some(cache) = registry.get_mut(&cache_id) {
            if let Some(cached) = cache.get(&key) {
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
                    cache.set(key, cached, None);
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

fn build_http_client(
    cookie_store: bool,
    direct_loopback_http: bool,
    url: &str,
) -> Result<reqwest::blocking::Client> {
    let target = validated_http_target(url).map_err(IntentError::runtime_error)?;
    build_http_client_for_target(cookie_store, direct_loopback_http, target)
}

fn build_http_client_for_target(
    cookie_store: bool,
    direct_loopback_http: bool,
    target: Option<ValidatedHttpTarget>,
) -> Result<reqwest::blocking::Client> {
    let mut builder = reqwest::blocking::Client::builder()
        .cookie_store(cookie_store)
        .redirect(reqwest::redirect::Policy::none());

    if let Some(target) = target {
        // A configured proxy would resolve the hostname independently and reopen the
        // validation-to-connection gap, so protected requests always connect directly.
        builder = builder
            .no_proxy()
            .resolve_to_addrs(&target.host, &target.addresses);
    }

    if direct_loopback_http {
        // Plaintext development traffic must remain on loopback even when the process
        // has system proxy settings.
        builder = builder.no_proxy();
    }
    builder.build().map_err(|error| {
        IntentError::runtime_error(format!("Failed to create HTTP client: {error}"))
    })
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

    let client = build_http_client(false, false, url)?;
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

fn secret_or_string<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    match value {
        Value::String(value) => Ok(value),
        Value::Secret(secret) => Ok(secret.expose()),
        other => Err(IntentError::type_error(format!(
            "fetch() {field} must be a String or Secret, got {}",
            other.type_name()
        ))),
    }
}

fn form_scalar(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        Value::Secret(secret) => Ok(secret.expose().to_string()),
        Value::Int(value) => Ok(value.to_string()),
        Value::Float(value) => Ok(value.to_string()),
        Value::Bool(value) => Ok(value.to_string()),
        other => Err(IntentError::type_error(format!(
            "fetch() form values must be scalar or Secret, got {}",
            other.type_name()
        ))),
    }
}

fn is_loopback_url(url: &reqwest::Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    let host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);

    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn validate_secret_transport(
    url: &str,
    contains_secret: bool,
    app_env: Option<&str>,
) -> Result<bool> {
    if !contains_secret {
        return Ok(false);
    }

    let parsed = reqwest::Url::parse(url).map_err(|_| {
        IntentError::type_error("Secret-bearing HTTP requests require a valid URL".to_string())
    })?;

    if parsed.scheme() == "https" {
        return Ok(false);
    }

    let development = app_env.is_some_and(|value| value.eq_ignore_ascii_case("development"));
    if development && parsed.scheme() == "http" && is_loopback_url(&parsed) {
        return Ok(true);
    }

    Err(IntentError::type_error(
        "Secret-bearing HTTP requests require HTTPS; APP_ENV=development permits HTTP only for localhost or loopback IPs"
            .to_string(),
    ))
}

fn format_ssrf_error(reason: &str, direct_loopback_http: bool) -> String {
    if direct_loopback_http {
        format!(
            "SSRF protection: {reason}. APP_ENV=development permits plaintext loopback transport, but NTNT's SSRF policy remains independent"
        )
    } else {
        format!("SSRF protection: {reason}")
    }
}

struct PreparedHttpRequest {
    request: reqwest::blocking::RequestBuilder,
}

fn prepare_http_request(
    opts: &HashMap<String, Value>,
    app_env: Option<&str>,
    cookie_store: bool,
) -> Result<std::result::Result<PreparedHttpRequest, String>> {
    if crate::stdlib::concurrent::is_current_task_cancelled() {
        return Err(IntentError::runtime_error("Task cancelled".to_string()));
    }

    let url = match opts.get("url") {
        Some(Value::String(url)) => url.clone(),
        _ => {
            return Err(IntentError::type_error(
                "fetch() requires 'url' option".to_string(),
            ))
        }
    };

    let contains_secret = opts.values().any(Value::contains_secret);
    let direct_loopback_http = validate_secret_transport(&url, contains_secret, app_env)?;
    let target = match validated_http_target(&url) {
        Ok(target) => target,
        Err(reason) => return Ok(Err(format_ssrf_error(&reason, direct_loopback_http))),
    };

    let method = match opts.get("method") {
        Some(Value::String(method)) => method.to_uppercase(),
        None => "GET".to_string(),
        Some(other) => {
            return Err(IntentError::type_error(format!(
                "fetch() method must be a String, got {}",
                other.type_name()
            )))
        }
    };
    let client = match build_http_client_for_target(cookie_store, direct_loopback_http, target) {
        Ok(client) => client,
        Err(error) => return Ok(Err(error.to_string())),
    };
    let mut request = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "DELETE" => client.delete(&url),
        "PATCH" => client.patch(&url),
        "HEAD" => client.head(&url),
        _ => {
            return Err(IntentError::runtime_error(format!(
                "Unsupported HTTP method: {method}"
            )))
        }
    };

    if let Some(Value::Map(headers)) = opts.get("headers") {
        for (key, value) in headers {
            request = request.header(key.as_str(), secret_or_string(value, "header value")?);
        }
    } else if let Some(other) = opts.get("headers") {
        return Err(IntentError::type_error(format!(
            "fetch() headers must be a Map, got {}",
            other.type_name()
        )));
    }

    if let Some(Value::Map(cookies)) = opts.get("cookies") {
        let cookie_values: Result<Vec<String>> = cookies
            .iter()
            .map(|(key, value)| {
                secret_or_string(value, "cookie value").map(|value| format!("{key}={value}"))
            })
            .collect();
        let cookie_values = cookie_values?;
        if !cookie_values.is_empty() {
            request = request.header(COOKIE, cookie_values.join("; "));
        }
    } else if let Some(other) = opts.get("cookies") {
        return Err(IntentError::type_error(format!(
            "fetch() cookies must be a Map, got {}",
            other.type_name()
        )));
    }

    if let Some(Value::Map(auth)) = opts.get("auth") {
        let username = match auth.get("user") {
            Some(value) => secret_or_string(value, "basic-auth user")?,
            None => "",
        };
        let password = match auth.get("pass") {
            Some(value) => secret_or_string(value, "basic-auth password")?,
            None => "",
        };
        if !username.is_empty() {
            let encoded = base64::engine::general_purpose::STANDARD
                .encode(format!("{username}:{password}").as_bytes());
            request = request.header(AUTHORIZATION, format!("Basic {encoded}"));
        }
    } else if let Some(other) = opts.get("auth") {
        return Err(IntentError::type_error(format!(
            "fetch() auth must be a Map, got {}",
            other.type_name()
        )));
    }

    if let Some(body) = opts.get("body") {
        request = request.body(secret_or_string(body, "body")?.to_string());
    }
    if let Some(data) = opts.get("json") {
        let json_body = intent_value_to_json_expose(data)?;
        request = request
            .header("Content-Type", "application/json")
            .body(json_body.to_string());
    }
    if let Some(Value::Map(form_data)) = opts.get("form") {
        let form: Result<Vec<(String, String)>> = form_data
            .iter()
            .map(|(key, value)| form_scalar(value).map(|value| (key.clone(), value)))
            .collect();
        request = request.form(&form?);
    } else if let Some(other) = opts.get("form") {
        return Err(IntentError::type_error(format!(
            "fetch() form must be a Map, got {}",
            other.type_name()
        )));
    }

    match opts.get("timeout") {
        Some(Value::Int(timeout)) if *timeout >= 0 => {
            request = request.timeout(Duration::from_secs(*timeout as u64));
        }
        Some(Value::Int(_)) => {
            return Err(IntentError::type_error(
                "fetch() timeout must be non-negative".to_string(),
            ))
        }
        Some(other) => {
            return Err(IntentError::type_error(format!(
                "fetch() timeout must be an Int, got {}",
                other.type_name()
            )))
        }
        None => {}
    }

    Ok(Ok(PreparedHttpRequest { request }))
}

#[derive(Clone, Copy)]
struct RedirectPolicy {
    enabled: bool,
    limit: usize,
    deadline: Option<Instant>,
}

impl RedirectPolicy {
    fn parse(opts: &HashMap<String, Value>) -> Result<Self> {
        let enabled = match opts.get("follow_redirects") {
            None | Some(Value::Bool(false)) => false,
            Some(Value::Bool(true)) => true,
            _ => {
                return Err(IntentError::type_error(
                    "fetch() follow_redirects must be a Bool".to_string(),
                ))
            }
        };
        let limit = match opts.get("max_redirects") {
            None => 5,
            Some(Value::Int(n)) if (1..=10).contains(n) => *n as usize,
            _ => {
                return Err(IntentError::type_error(
                    "fetch() max_redirects must be an Int in 1..10".to_string(),
                ))
            }
        };
        let deadline = if enabled {
            if opts.values().any(Value::contains_secret) {
                return Err(IntentError::type_error(
                    "Secret-bearing requests cannot follow redirects".to_string(),
                ));
            }
            if let Some(Value::Map(headers)) = opts.get("headers") {
                if headers.keys().any(|key| key.eq_ignore_ascii_case("host")) {
                    return Err(IntentError::type_error(
                        "Explicit Host is forbidden with follow_redirects".to_string(),
                    ));
                }
            }
            let seconds = match opts.get("timeout") {
                None => 30,
                Some(Value::Int(n)) if *n >= 0 => *n as u64,
                _ => {
                    return Err(IntentError::type_error(
                        "fetch() timeout must be a non-negative Int".to_string(),
                    ))
                }
            };
            Some(
                Instant::now()
                    .checked_add(Duration::from_secs(seconds))
                    .ok_or_else(|| {
                        IntentError::type_error("fetch() timeout is too large".to_string())
                    })?,
            )
        } else {
            None
        };
        Ok(Self {
            enabled,
            limit,
            deadline,
        })
    }

    fn expired(self) -> Result<bool> {
        if crate::stdlib::concurrent::is_current_task_cancelled() {
            return Err(IntentError::runtime_error("Task cancelled".to_string()));
        }
        Ok(self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline))
    }
}

fn url_has_userinfo(url: &str) -> bool {
    url.strip_prefix("//")
        .or_else(|| url.split_once("://").map(|(_, authority)| authority))
        .is_some_and(|authority| {
            authority
                .split(['/', '?', '#'])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
}

fn redirect_url(url: &str) -> std::result::Result<reqwest::Url, String> {
    if url_has_userinfo(url) {
        return Err("Redirect URL userinfo is forbidden".into());
    }
    let mut url = reqwest::Url::parse(url).map_err(|_| "Invalid redirect URL".to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Redirect URL requires HTTP or HTTPS".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("Redirect URL userinfo is forbidden".into());
    }
    url.set_fragment(None);
    Ok(url)
}

fn redirect_options(
    opts: &mut HashMap<String, Value>,
    status: u16,
    origin_changed: bool,
) -> std::result::Result<(), String> {
    let method = match opts.get("method") {
        Some(Value::String(method)) => method.to_uppercase(),
        _ => "GET".to_string(),
    };
    let to_get =
        (status == 303 && method != "HEAD") || (matches!(status, 301 | 302) && method == "POST");
    if to_get {
        opts.insert("method".into(), Value::String("GET".into()));
        for key in ["body", "json", "form"] {
            opts.remove(key);
        }
        if let Some(Value::Map(headers)) = opts.get_mut("headers") {
            headers.retain(|key, _| {
                let key = key.to_ascii_lowercase();
                !key.starts_with("content-")
                    && !matches!(
                        key.as_str(),
                        "transfer-encoding" | "trailer" | "expect" | "digest"
                    )
            });
        }
    }
    if origin_changed {
        if ["body", "json", "form"]
            .iter()
            .any(|key| opts.contains_key(*key))
        {
            return Err("Cross-origin redirect would replay a request body".into());
        }
        opts.remove("auth");
        opts.remove("cookies");
        if let Some(Value::Map(headers)) = opts.get_mut("headers") {
            headers.retain(|key, _| {
                matches!(
                    key.to_ascii_lowercase().as_str(),
                    "accept" | "accept-language" | "user-agent"
                )
            });
        }
    }
    Ok(())
}

struct HttpResponse {
    response: reqwest::blocking::Response,
    policy: RedirectPolicy,
    hops: usize,
}

/// Manual redirect transport shared by request-map consumers. Automatic following
/// remains disabled; each preparation resolves, validates and pins its own target.
fn send_http_request(
    opts: &HashMap<String, Value>,
    app_env: Option<&str>,
    cookie_store: bool,
) -> Result<std::result::Result<HttpResponse, String>> {
    let policy = RedirectPolicy::parse(opts)?;
    let mut effective = opts.clone();
    let mut visited = std::collections::HashSet::new();
    for hops in 0..=policy.limit {
        if policy.expired()? {
            return Ok(Err("HTTP chain timeout".into()));
        }
        if policy.enabled {
            let Some(Value::String(url)) = effective.get("url") else {
                return Err(IntentError::type_error(
                    "fetch() requires 'url' option".to_string(),
                ));
            };
            let url = match redirect_url(url) {
                Ok(url) => url,
                Err(e) => return Ok(Err(e)),
            };
            if !visited.insert(url.to_string()) {
                return Ok(Err("Redirect cycle detected".into()));
            }
            effective.insert("url".into(), Value::String(url.to_string()));
        }
        let prepared =
            match prepare_http_request(&effective, app_env, cookie_store && !policy.enabled)? {
                Ok(prepared) => prepared,
                Err(error) => return Ok(Err(error)),
            };
        if policy.expired()? {
            return Ok(Err("HTTP chain timeout".into()));
        }
        let request = if let Some(deadline) = policy.deadline {
            prepared
                .request
                .timeout(deadline.saturating_duration_since(Instant::now()))
        } else {
            prepared.request
        };
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => return Ok(Err(format!("HTTP request failed: {error}"))),
        };
        if policy.expired()? {
            return Ok(Err("HTTP chain timeout".into()));
        }
        if !policy.enabled || !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
            return Ok(Ok(HttpResponse {
                response,
                policy,
                hops,
            }));
        }
        let mut locations = response.headers().get_all(reqwest::header::LOCATION).iter();
        let Some(location) = locations.next() else {
            return Ok(Ok(HttpResponse {
                response,
                policy,
                hops,
            }));
        };
        if locations.next().is_some() || location.as_bytes().len() > 8192 {
            return Ok(Err("Invalid redirect Location count or size".into()));
        }
        let location = match location.to_str() {
            Ok(location)
                if !location.trim().is_empty() && !location.chars().any(char::is_control) =>
            {
                location
            }
            _ => return Ok(Err("Invalid redirect Location".into())),
        };
        if url_has_userinfo(location) {
            return Ok(Err("Redirect URL userinfo is forbidden".into()));
        }
        let next = response.url().join(location).ok();
        let Some(next) = next else {
            return Ok(Err("Invalid redirect Location".into()));
        };
        let next = match redirect_url(next.as_str()) {
            Ok(next) => next,
            Err(e) => return Ok(Err(e)),
        };
        if response.url().scheme() == "https" && next.scheme() == "http" {
            return Ok(Err("HTTPS redirect downgrade forbidden".into()));
        }
        if hops == policy.limit {
            return Ok(Err("Redirect limit exceeded".into()));
        }
        if let Err(error) = redirect_options(
            &mut effective,
            response.status().as_u16(),
            response.url().origin() != next.origin(),
        ) {
            return Ok(Err(error));
        }
        effective.insert("url".to_string(), Value::String(next.to_string()));
        // Drop the intermediate body without consuming it.
    }
    unreachable!()
}

/// Opted-in text is UTF-8 with replacement, independent of Content-Type charset.
/// Validate decoded expansion before allocating the returned String.
fn read_redirect_body(
    mut response: reqwest::blocking::Response,
    policy: RedirectPolicy,
) -> Result<std::result::Result<String, String>> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        if policy.expired()? {
            return Ok(Err("HTTP chain timeout".into()));
        }
        let read = match response.read(&mut buffer) {
            Ok(n) => n,
            Err(e) => return Ok(Err(format!("Failed to read response body: {e}"))),
        };
        if read == 0 {
            break;
        }
        if read > max_response_size().saturating_sub(bytes.len()) {
            return Ok(Err("Response body too large".into()));
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let mut decoded_len = 0usize;
    for chunk in bytes.utf8_chunks() {
        let len = chunk.valid().len() + if chunk.invalid().is_empty() { 0 } else { 3 };
        if len > max_response_size().saturating_sub(decoded_len) {
            return Ok(Err("Decoded response body too large".into()));
        }
        decoded_len += len;
    }
    let body = String::from_utf8_lossy(&bytes).into_owned();
    if policy.expired()? {
        return Ok(Err("HTTP chain timeout".into()));
    }
    Ok(Ok(body))
}

/// Full HTTP request with all options
fn http_fetch(opts: &HashMap<String, Value>) -> Result<Value> {
    let app_env = std::env::var("APP_ENV").ok();
    http_fetch_with_app_env(opts, app_env.as_deref())
}

pub(crate) fn http_fetch_with_app_env(
    opts: &HashMap<String, Value>,
    app_env: Option<&str>,
) -> Result<Value> {
    let url = match opts.get("url") {
        Some(Value::String(url)) => url.clone(),
        _ => {
            return Err(IntentError::type_error(
                "fetch() requires 'url' option".to_string(),
            ))
        }
    };
    let response = send_http_request(opts, app_env, true)?;
    match response {
        Ok(chain) => {
            let response = chain.response;
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

            let body = if chain.policy.enabled {
                read_redirect_body(response, chain.policy)?
            } else {
                response.text().map_err(|e| e.to_string())
            };
            match body {
                Ok(body) => {
                    let mut resp_value =
                        response_to_value(status, &headers, body, &final_url, &url);
                    if let Value::Map(ref mut map) = resp_value {
                        map.insert("redirected".into(), Value::Bool(chain.hops > 0));
                    }
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

#[derive(Clone, Copy)]
struct DownloadOptions {
    overwrite: bool,
    create_parent: bool,
}

fn download_temp_path(path: &Path) -> Result<PathBuf> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    let directory = parent.unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| IntentError::type_error("download() path must name a file".to_string()))?;
    Ok(directory.join(format!(
        ".{file_name}.ntnt-download-{}",
        uuid::Uuid::new_v4()
    )))
}

fn response_headers_value(headers: &reqwest::header::HeaderMap) -> Value {
    Value::Map(
        headers
            .iter()
            .filter_map(|(name, value)| {
                value
                    .to_str()
                    .ok()
                    .map(|value| (name.to_string(), Value::String(value.to_string())))
            })
            .collect(),
    )
}

fn promote_download(
    temporary_path: &Path,
    destination: &Path,
    overwrite: bool,
) -> std::io::Result<()> {
    if !overwrite {
        std::fs::hard_link(temporary_path, destination)?;
        return std::fs::remove_file(temporary_path);
    }

    prepare_overwrite_permissions(temporary_path, destination)?;
    replace_download(temporary_path, destination)
}

fn prepare_overwrite_permissions(temporary_path: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    match std::fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_file() => {
            std::fs::set_permissions(temporary_path, metadata.permissions())?;
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    #[cfg(not(unix))]
    let _ = (temporary_path, destination);
    Ok(())
}

fn replace_download(temporary_path: &Path, destination: &Path) -> std::io::Result<()> {
    #[cfg(not(windows))]
    {
        std::fs::rename(temporary_path, destination)
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let source = temporary_path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let result = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

fn commit_redirect_download(
    policy: RedirectPolicy,
    temporary_path: &Path,
    destination: &Path,
    overwrite: bool,
    cleanup: impl FnOnce(&Path) -> std::io::Result<()>,
) -> Result<std::result::Result<Option<String>, String>> {
    if policy.expired()? {
        return Ok(Err("HTTP chain timeout".into()));
    }
    if overwrite {
        if let Err(error) = prepare_overwrite_permissions(temporary_path, destination) {
            return Ok(Err(format!(
                "Failed to prepare download permissions: {error}"
            )));
        }
        // Permission preparation can block too. Recheck immediately before the
        // replacing filesystem operation, not merely before its preparation.
        if policy.expired()? {
            return Ok(Err("HTTP chain timeout".into()));
        }
        return Ok(replace_download(temporary_path, destination)
            .map(|_| None)
            .map_err(|e| format!("Failed to promote download: {e}")));
    }
    if let Err(error) = std::fs::hard_link(temporary_path, destination) {
        return Ok(Err(format!("Failed to promote download: {error}")));
    }
    // A successful link is the commit point. Cleanup cannot undo success.
    Ok(Ok(cleanup(temporary_path).err().map(|error| {
        format!("Download committed; temporary cleanup failed: {error}")
    })))
}

fn http_download(
    request_options: &HashMap<String, Value>,
    file_path: &str,
    file_options: DownloadOptions,
) -> Result<Value> {
    let path = Path::new(file_path);
    if path.exists() && !file_options.overwrite {
        return Ok(Value::err(Value::String(format!(
            "download() destination already exists: {file_path}"
        ))));
    }

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if !parent.exists() {
            if !file_options.create_parent {
                return Ok(Value::err(Value::String(format!(
                    "download() parent directory does not exist: {}",
                    parent.display()
                ))));
            }
            std::fs::create_dir_all(parent).map_err(|error| {
                IntentError::runtime_error(format!("Failed to create directory: {error}"))
            })?;
        }
    }

    let app_env = std::env::var("APP_ENV").ok();
    let chain = match send_http_request(request_options, app_env.as_deref(), false)? {
        Ok(chain) => chain,
        Err(error) => return Ok(Value::err(Value::String(error))),
    };
    let policy = chain.policy;
    let mut response = chain.response;
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    if !(200..300).contains(&status) {
        let mut diagnostic = Vec::new();
        let _ = response.take(8192).read_to_end(&mut diagnostic);
        let diagnostic = String::from_utf8_lossy(&diagnostic);
        let suffix = if diagnostic.trim().is_empty() {
            String::new()
        } else {
            format!(": {}", diagnostic.trim())
        };
        return Ok(Value::err(Value::String(format!(
            "HTTP error: status {status}{suffix}"
        ))));
    }

    let temporary_path = download_temp_path(path)?;
    let mut temporary = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_path)
    {
        Ok(file) => file,
        Err(error) => {
            return Ok(Value::err(Value::String(format!(
                "Failed to create temporary download: {error}"
            ))))
        }
    };

    let transfer = (|| -> Result<std::result::Result<i64, String>> {
        let mut buffer = [0_u8; 64 * 1024];
        let mut bytes_written = 0_i64;
        loop {
            if crate::stdlib::concurrent::is_current_task_cancelled() {
                return Err(IntentError::runtime_error("download cancelled".to_string()));
            }
            if policy.expired()? {
                return Ok(Err("HTTP chain timeout".into()));
            }
            let read = match response.read(&mut buffer) {
                Ok(read) => read,
                Err(error) => return Ok(Err(format!("Failed to read response: {error}"))),
            };
            if read == 0 {
                break;
            }
            if policy.enabled && read > max_response_size().saturating_sub(bytes_written as usize) {
                return Ok(Err("Response body too large".into()));
            }
            if let Err(error) = temporary.write_all(&buffer[..read]) {
                return Ok(Err(format!("Failed to write file: {error}")));
            }
            bytes_written += read as i64;
        }
        if policy.enabled && policy.expired()? {
            return Ok(Err("HTTP chain timeout".into()));
        }
        if let Err(error) = temporary.flush().and_then(|_| temporary.sync_all()) {
            return Ok(Err(format!("Failed to flush file: {error}")));
        }
        if policy.enabled && policy.expired()? {
            return Ok(Err("HTTP chain timeout".into()));
        }
        Ok(Ok(bytes_written))
    })();
    drop(temporary);
    let bytes_written = match transfer {
        Ok(Ok(bytes)) => bytes,
        other => {
            let _ = std::fs::remove_file(&temporary_path);
            return match other {
                Err(error) => Err(error),
                Ok(Err(error)) => Ok(Value::err(Value::String(error))),
                _ => unreachable!(),
            };
        }
    };
    let promotion = if policy.enabled {
        commit_redirect_download(
            policy,
            &temporary_path,
            path,
            file_options.overwrite,
            |path| std::fs::remove_file(path),
        )
    } else {
        Ok(
            promote_download(&temporary_path, path, file_options.overwrite)
                .map(|_| None)
                .map_err(|e| format!("Failed to promote download: {e}")),
        )
    };
    let cleanup_warning = match promotion {
        Ok(Ok(warning)) => warning,
        other => {
            let _ = std::fs::remove_file(&temporary_path);
            return match other {
                Err(error) => Err(error),
                Ok(Err(error)) => Ok(Value::err(Value::String(error))),
                _ => unreachable!(),
            };
        }
    };

    let mut info = HashMap::from([
        ("status".to_string(), Value::Int(status as i64)),
        ("path".to_string(), Value::String(file_path.to_string())),
        ("size".to_string(), Value::Int(bytes_written)),
        ("bytes_written".to_string(), Value::Int(bytes_written)),
        ("headers".to_string(), response_headers_value(&headers)),
    ]);
    if let Some(warning) = cleanup_warning {
        info.insert("cleanup_warning".into(), Value::String(warning));
        info.insert(
            "temporary_path".into(),
            Value::String(temporary_path.display().to_string()),
        );
    }
    Ok(Value::ok(Value::Map(info)))
}

fn download_file_options(
    value: Option<&Value>,
    defaults: DownloadOptions,
) -> Result<DownloadOptions> {
    let Some(value) = value else {
        return Ok(defaults);
    };
    let Value::Map(options) = value else {
        return Err(IntentError::type_error(
            "download() file options must be a Map".to_string(),
        ));
    };
    let boolean = |name: &str, fallback: bool| match options.get(name) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(IntentError::type_error(format!(
            "download() {name} must be a Bool, got {}",
            other.type_name()
        ))),
        None => Ok(fallback),
    };
    Ok(DownloadOptions {
        overwrite: boolean("overwrite", defaults.overwrite)?,
        create_parent: boolean("create_parent", defaults.create_parent)?,
    })
}

fn download_from_args(args: &[Value]) -> Result<Value> {
    let Value::String(file_path) = &args[1] else {
        return Err(IntentError::type_error(
            "download() file path must be a String".to_string(),
        ));
    };
    let (request_options, defaults) = match &args[0] {
        Value::String(url) => (
            HashMap::from([("url".to_string(), Value::String(url.clone()))]),
            DownloadOptions {
                overwrite: true,
                create_parent: true,
            },
        ),
        Value::Map(options) => (
            options.clone(),
            DownloadOptions {
                overwrite: false,
                create_parent: false,
            },
        ),
        other => {
            return Err(IntentError::type_error(format!(
                "download() requires a URL String or request Map, got {}",
                other.type_name()
            )))
        }
    };
    let file_options = download_file_options(args.get(2), defaults)?;
    http_download(&request_options, file_path, file_options)
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
    // Options map keys: url (set automatically in 2-arg form), method, headers, body, json, form, auth, cookies, timeout, follow_redirects, max_redirects.
    // Redirects default to terminal 3xx responses. follow_redirects:true manually follows
    // 301/302/303/307/308, validating and pinning each protected hop without proxies.
    // max_redirects defaults to 5 (allowed 1..10); it never enables following by itself.
    // Opt-in rejects Secret values, URL userinfo, explicit Host, HTTPS downgrades,
    // cycles, and malformed/multiple/oversized Location values. Missing Location is terminal.
    // Origins compare scheme, normalized hostname and effective port. Origin changes
    // permanently strip auth/cookies and caller headers except Accept, Accept-Language,
    // User-Agent; response cookies are never forwarded. Body replay across origins fails.
    // POST 301/302 and non-HEAD 303 become GET, clearing bodies and entity headers.
    // Opt-in uses one timeout budget (default 30 seconds); zero expires before contact.
    // Blocking system DNS and filesystem operations cannot be interrupted by this budget.
    // Final opt-in bodies use bounded UTF-8 decoding with replacement (charset ignored),
    // limited by NTNT_MAX_RESPONSE_SIZE in both received bytes and decoded UTF-8 bytes.
    // URL-only cycle detection also rejects POST->303->GET at the same normalized URL.
    // Opaque Secret values are accepted only in header values, cookie values, basic-auth
    // fields, raw bodies, JSON leaves, and form values. Secret-bearing requests require
    // HTTPS; APP_ENV=development permits direct HTTP only for localhost and loopback IPs,
    // bypassing system proxies.
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
    // @error TypeError ~ "fetch() follow_redirects must be a Bool" fix: "Pass true or false for follow_redirects"
    // @error TypeError ~ "Secret-bearing requests cannot follow redirects" fix: "Use explicit independently authorized requests for Secret-bearing traffic"
    // @error TypeError ~ "Secret-bearing HTTP requests require HTTPS" fix: "Use HTTPS, or set APP_ENV=development for localhost/loopback HTTP"
    // @error RuntimeError ~ "Unsupported HTTP method: ..." fix: "Use GET, POST, PUT, DELETE, PATCH, or HEAD"
    module.insert(
        "fetch".to_string(),
        Value::NativeFunction {
            name: "fetch".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
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
    // @signature download(url_or_options: String | Map, file_path: String, file_options?: Map) -> Result<Map, String>
    // Stream an HTTP response to a file and promote it atomically.
    //
    // A String performs the legacy GET behavior, including parent creation, overwrite,
    // and preservation of Unix regular-file permissions when replacing a destination.
    // A request Map accepts the same request fields and safety rules as fetch(). Its safe
    // file defaults reject overwrite and missing parents. file_options can set overwrite
    // and create_parent. Failed requests leave an existing destination unchanged.
    // Request maps support the same explicit redirect policy as fetch(). Opt-in streamed
    // bytes are bounded by NTNT_MAX_RESPONSE_SIZE. Failures before promotion preserve
    // the destination and remove the temporary file. Promotion is the commit point;
    // committed no-overwrite cleanup failure returns Ok with cleanup_warning and temporary_path.
    // @param url_or_options A URL String for GET or a fetch-compatible request Map
    // @param file_path The local file path to save the downloaded content
    // @param file_options Optional Map with overwrite and create_parent Bool fields
    // @returns Result<Map{status: Int, path: String, size: Int, bytes_written: Int, headers: Map}, String>
    // @see_also fetch
    // @since v0.1.0
    // @tags #network
    // @example download("https://example.com/file.zip", "./file.zip") => Ok({status: 200, path: "./file.zip", size: 1024, bytes_written: 1024}) ~ "Legacy GET download"
    // @example download(map { "url": "https://example.com/audio", "method": "POST", "json": map { "text": "Hello" } }, "./take.wav", map { "create_parent": true }) => Ok({status: 200, path: "./take.wav", ...}) ~ "Binary POST download"
    // @error TypeError ~ "download() requires a URL String or request Map" fix: "Pass a URL String or fetch-compatible request Map"
    // @error RuntimeError ~ "Failed to create directory: ..." fix: "Ensure the parent directory path is valid and writable"
    // @error RuntimeError ~ "Failed to create file: ..." fix: "Ensure the file path is valid and writable"
    // @error RuntimeError ~ "HTTP error: status ..." fix: "Check the URL and server availability"
    module.insert(
        "download".to_string(),
        Value::NativeFunction {
            name: "download".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: download_from_args,
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
            requires: None,
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
    // Checks the cache for a response matching the URL and redirect policy/hop limit.
    // Redirect policy and opt-in timeout are validated even on a cache hit.
    // Secret-bearing request options are rejected because cache keys do not include credentials.
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
            requires: None,
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
    // Evicts all redirect-policy variants for the given URL, if present.
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
            requires: None,
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
            requires: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;

    static DOWNLOAD_COUNTER: AtomicUsize = AtomicUsize::new(0);

    // Production configuration is process-global and cached in OnceLocks. Keep
    // environment-dependent cases isolated under both nextest and plain libtest.
    fn run_http_environment_case(name: &str) -> bool {
        let test = format!("stdlib::http::tests::{name}");
        const GUARD: &str = "NTNT_HTTP_ISOLATED_TEST";
        if std::env::var(GUARD).ok().as_deref() == Some(test.as_str()) {
            return false;
        }
        let mut output = tempfile::tempfile().expect("create isolated test log");
        let mut command =
            std::process::Command::new(std::env::current_exe().expect("current test executable"));
        command.args(["--exact", &test, "--nocapture", "--test-threads=1"]);
        for (key, _) in std::env::vars_os() {
            let text = key.to_string_lossy();
            if text.starts_with("NTNT_")
                || text == "APP_ENV"
                || text.to_ascii_lowercase().ends_with("_proxy")
            {
                command.env_remove(key);
            }
        }
        command
            .env(GUARD, &test)
            .env("NTNT_ENV", "development")
            .env("NTNT_SSRF_PROTECTION", "true")
            .env("NTNT_ALLOW_LOCALHOST", "true")
            .env("NTNT_ALLOW_PRIVATE_IPS", "false")
            .stdout(output.try_clone().expect("clone test stdout"))
            .stderr(output.try_clone().expect("clone test stderr"));
        let mut child = command.spawn().expect("spawn isolated HTTP test");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().expect("poll isolated HTTP test") {
                break status;
            }
            if started.elapsed() >= Duration::from_secs(30) {
                let _ = child.kill();
                let _ = child.wait();
                panic!("isolated HTTP test exceeded 30 seconds: {test}");
            }
            thread::sleep(Duration::from_millis(10));
        };
        std::io::Seek::rewind(&mut output).expect("rewind test log");
        let mut log = String::new();
        output.read_to_string(&mut log).expect("read test log");
        assert!(status.success(), "isolated HTTP test failed: {test}\n{log}");
        assert!(
            log.contains(&test) && log.contains("test result: ok. 1 passed;"),
            "isolated HTTP test did not execute exactly one passing case: {test}\n{log}"
        );
        true
    }

    fn response_status(value: Value) -> i64 {
        let Value::EnumValue {
            enum_name,
            variant,
            values,
        } = value
        else {
            panic!("fetch returned a non-Result value");
        };
        assert_eq!(enum_name, "Result");
        assert_eq!(variant, "Ok");
        let Value::Map(response) = &values[0] else {
            panic!("fetch returned a non-map response");
        };
        let Some(Value::Int(status)) = response.get("status") else {
            panic!("fetch response omitted integer status");
        };
        *status
    }

    fn result_map(value: Value) -> HashMap<String, Value> {
        let Value::EnumValue {
            enum_name,
            variant,
            mut values,
        } = value
        else {
            panic!("operation returned a non-Result value");
        };
        assert_eq!(enum_name, "Result");
        assert_eq!(variant, "Ok");
        let Value::Map(map) = values.remove(0) else {
            panic!("operation returned a non-map value");
        };
        map
    }

    fn result_error(value: Value) -> String {
        let Value::EnumValue {
            enum_name,
            variant,
            mut values,
        } = value
        else {
            panic!("operation returned a non-Result value");
        };
        assert_eq!(enum_name, "Result");
        assert_eq!(variant, "Err");
        let Value::String(error) = values.remove(0) else {
            panic!("operation returned a non-string error");
        };
        error
    }

    fn download_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "ntnt-{name}-{}-{}",
            std::process::id(),
            DOWNLOAD_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn assert_no_download_temporary_file(directory: &Path) {
        let entries = std::fs::read_dir(directory)
            .expect("read download directory")
            .map(|entry| entry.expect("read directory entry").file_name())
            .collect::<Vec<_>>();
        assert!(
            entries
                .iter()
                .all(|name| !name.to_string_lossy().contains(".ntnt-download-")),
            "download left a temporary file behind: {entries:?}"
        );
    }

    fn raw_response_fixture(response: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind response fixture");
        let url = format!("http://{}/audio", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept response request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set response request timeout");
            let mut buffer = [0_u8; 1024];
            let _ = stream.read(&mut buffer).expect("read response request");
            stream.write_all(&response).expect("write raw response");
        });
        (url, server)
    }

    fn binary_post_fixture(
        expected_body: &'static str,
        bytes: Vec<u8>,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind binary fixture");
        let url = format!("http://{}/speech", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept binary request");
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .expect("set binary request timeout");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).expect("read binary request");
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= header_end + 4 + length {
                    break;
                }
            }

            let request_text = String::from_utf8_lossy(&request).to_string();
            assert!(request_text.starts_with("POST /speech HTTP/1.1"));
            assert!(request_text
                .to_ascii_lowercase()
                .contains("x-studio: sound-stage"));
            assert!(request_text.ends_with(expected_body));
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .expect("write binary headers");
            let split = bytes.len() / 2;
            stream
                .write_all(&bytes[..split])
                .expect("write first binary chunk");
            stream.flush().expect("flush first binary chunk");
            thread::sleep(Duration::from_millis(10));
            stream
                .write_all(&bytes[split..])
                .expect("write second binary chunk");
            request_text
        });
        (url, server)
    }

    struct RedirectFixture {
        url: String,
        destination_hits: Arc<AtomicUsize>,
        destination_done: mpsc::Sender<()>,
        redirect_thread: thread::JoinHandle<()>,
        destination_thread: thread::JoinHandle<()>,
    }

    impl RedirectFixture {
        fn finish(self) -> usize {
            self.redirect_thread.join().expect("redirect thread");
            self.destination_done
                .send(())
                .expect("stop destination listener");
            self.destination_thread.join().expect("destination thread");
            self.destination_hits.load(Ordering::SeqCst)
        }
    }

    fn drain_http_request(stream: &mut std::net::TcpStream) {
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("set request read timeout");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = match stream.read(&mut buffer) {
                Ok(0) => return,
                Ok(read) => read,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ) =>
                {
                    return;
                }
                Err(error) => panic!("read request: {error}"),
            };
            request.extend_from_slice(&buffer[..read]);

            let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length").then(|| {
                        value
                            .trim()
                            .parse::<usize>()
                            .expect("integer content length")
                    })
                })
                .unwrap_or(0);
            if request.len() >= header_end + 4 + content_length {
                return;
            }
        }
    }

    #[test]
    fn drain_http_request_treats_an_incomplete_request_timeout_as_completion() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind timeout fixture");
        let mut client = std::net::TcpStream::connect(listener.local_addr().unwrap())
            .expect("connect timeout fixture");
        client
            .write_all(b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\n\r\n")
            .expect("write incomplete request");
        let (mut server, _) = listener.accept().expect("accept timeout fixture");

        drain_http_request(&mut server);
    }

    fn redirect_fixture(status: u16) -> RedirectFixture {
        let destination = TcpListener::bind("127.0.0.1:0").expect("bind destination");
        destination
            .set_nonblocking(true)
            .expect("set destination nonblocking");
        let destination_url = format!("http://{}/private", destination.local_addr().unwrap());
        let destination_hits = Arc::new(AtomicUsize::new(0));
        let hits = Arc::clone(&destination_hits);
        let (destination_done, destination_done_rx) = mpsc::channel();
        let destination_thread = thread::spawn(move || loop {
            if destination_done_rx.try_recv().is_ok() {
                break;
            }
            match destination.accept() {
                Ok((mut stream, _)) => {
                    hits.fetch_add(1, Ordering::SeqCst);
                    drain_http_request(&mut stream);
                    stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 11\r\nConnection: close\r\n\r\ndestination",
                        )
                        .expect("write destination response");
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(error) => panic!("destination accept failed: {error}"),
            }
        });

        let redirector = TcpListener::bind("127.0.0.1:0").expect("bind redirector");
        let url = format!("http://{}/redirect", redirector.local_addr().unwrap());
        let redirect_thread = thread::spawn(move || {
            let (mut stream, _) = redirector.accept().expect("accept redirect request");
            drain_http_request(&mut stream);
            let response = format!(
                "HTTP/1.1 {status} Redirect\r\nLocation: {destination_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            stream
                .write_all(response.as_bytes())
                .expect("write redirect response");
        });

        RedirectFixture {
            url,
            destination_hits,
            destination_done,
            redirect_thread,
            destination_thread,
        }
    }

    #[test]
    fn fetch_options_do_not_follow_redirects_by_default() {
        for status in [301, 302, 303, 307, 308] {
            let fixture = redirect_fixture(status);
            let mut options =
                HashMap::from([("url".to_string(), Value::String(fixture.url.clone()))]);
            if status == 307 || status == 308 {
                options.insert("method".to_string(), Value::String("POST".to_string()));
                options.insert(
                    "body".to_string(),
                    Value::String("redirect-body-canary".to_string()),
                );
            }
            let result = http_fetch(&options).expect("fetch should return a Result value");

            assert_eq!(response_status(result), status as i64);
            assert_eq!(fixture.finish(), 0, "must not follow {status} redirect");
        }
    }

    #[test]
    fn fetch_string_does_not_follow_redirects_by_default() {
        for status in [301, 302, 303, 307, 308] {
            let fixture = redirect_fixture(status);
            let result = http_get(&fixture.url).expect("fetch should return a Result value");

            assert_eq!(response_status(result), status as i64);
            assert_eq!(fixture.finish(), 0, "must not follow {status} redirect");
        }
    }

    #[test]
    fn cache_misses_do_not_follow_redirects_by_default() {
        for status in [301, 302, 303, 307, 308] {
            let fixture = redirect_fixture(status);
            let cache_id = get_next_cache_id();
            CACHE_REGISTRY
                .lock()
                .expect("cache registry")
                .insert(cache_id, ResponseCache::new(60));

            let result = cache_fetch(cache_id, &fixture.url, None)
                .expect("cache fetch should return a Result value");

            CACHE_REGISTRY
                .lock()
                .expect("cache registry")
                .remove(&cache_id);
            assert_eq!(response_status(result), status as i64);
            assert_eq!(fixture.finish(), 0, "must not follow {status} redirect");
        }
    }

    struct ChainFixture {
        url: String,
        requests: Arc<Mutex<Vec<String>>>,
        stop: mpsc::Sender<()>,
        server: thread::JoinHandle<()>,
    }

    impl ChainFixture {
        fn new(respond: impl Fn(usize, &str) -> String + Send + 'static) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/start", listener.local_addr().unwrap());
            let requests = Arc::new(Mutex::new(Vec::new()));
            let captured = requests.clone();
            let (stop, done) = mpsc::channel();
            let server = thread::spawn(move || {
                while done.try_recv().is_err() {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            stream
                                .set_write_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut bytes = Vec::new();
                            let mut buffer = [0; 4096];
                            loop {
                                let Ok(n) = stream.read(&mut buffer) else {
                                    break;
                                };
                                if n == 0 {
                                    break;
                                }
                                bytes.extend_from_slice(&buffer[..n]);
                                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                                    let headers = String::from_utf8_lossy(&bytes[..end]);
                                    let len = headers
                                        .lines()
                                        .find_map(|line| {
                                            let (k, v) = line.split_once(':')?;
                                            k.eq_ignore_ascii_case("content-length")
                                                .then(|| v.trim().parse::<usize>().unwrap())
                                        })
                                        .unwrap_or(0);
                                    if bytes.len() >= end + 4 + len {
                                        break;
                                    }
                                }
                            }
                            let request = String::from_utf8_lossy(&bytes).to_string();
                            let mut requests = captured.lock().unwrap();
                            let n = requests.len();
                            requests.push(request.clone());
                            drop(requests);
                            let _ = stream.write_all(respond(n, &request).as_bytes());
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(2))
                        }
                        Err(e) => panic!("{e}"),
                    }
                }
            });
            Self {
                url,
                requests,
                stop,
                server,
            }
        }
        fn finish(self) -> Vec<String> {
            self.stop.send(()).unwrap();
            self.server.join().unwrap();
            Arc::try_unwrap(self.requests)
                .unwrap()
                .into_inner()
                .unwrap()
        }
        fn options(&self) -> HashMap<String, Value> {
            HashMap::from([
                ("url".into(), Value::String(self.url.clone())),
                ("follow_redirects".into(), Value::Bool(true)),
            ])
        }
    }

    fn redirect_response(status: u16, location: &str) -> String {
        format!("HTTP/1.1 {status} Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    }

    #[test]
    fn safe_redirect_limits_and_cycles() {
        let fixture = ChainFixture::new(|n, _| redirect_response(302, &format!("/hop{n}")));
        let mut opts = fixture.options();
        opts.insert("max_redirects".into(), Value::Int(1));
        let result = http_fetch(&opts);
        let requests = fixture.finish();
        assert!(result_error(result.unwrap()).contains("limit"));
        assert_eq!(requests.len(), 2);
        let fixture = ChainFixture::new(|_, _| redirect_response(303, "/start#fragment"));
        let result = http_fetch(&fixture.options());
        let requests = fixture.finish();
        assert!(result_error(result.unwrap()).contains("cycle"));
        assert_eq!(requests.len(), 1);
    }

    #[test]
    fn safe_redirect_rejects_invalid_locations_before_destination() {
        for location in [
            "".to_string(),
            "x".repeat(8193),
            "/final\r\nLocation: /other".into(),
            "file:///tmp/blocked".into(),
            "http://user:pass@127.0.0.1:1/".into(),
        ] {
            let fixture = ChainFixture::new(move |_, _| redirect_response(302, &location));
            let result = http_fetch(&fixture.options());
            let requests = fixture.finish();
            assert!(!result_error(result.unwrap()).is_empty());
            assert_eq!(requests.len(), 1);
        }
    }

    #[test]
    fn safe_redirect_option_validation_before_contact() {
        let fixture =
            ChainFixture::new(|_, _| "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into());
        for value in [Value::Int(0), Value::Int(11), Value::String("5".into())] {
            let mut opts = fixture.options();
            opts.insert("max_redirects".into(), value);
            assert!(http_fetch(&opts).is_err());
        }
        for value in [
            Value::Int(-1),
            Value::String("1".into()),
            Value::Int(i64::MAX),
        ] {
            let mut opts = fixture.options();
            opts.insert("timeout".into(), value);
            assert!(http_fetch(&opts).is_err());
        }
        let mut opts = fixture.options();
        opts.insert("timeout".into(), Value::Int(0));
        assert!(result_error(http_fetch(&opts).unwrap()).contains("timeout"));
        opts.remove("timeout");
        opts.insert(
            "headers".into(),
            Value::Map(HashMap::from([(
                "hOsT".into(),
                Value::String("other".into()),
            )])),
        );
        assert!(http_fetch(&opts).is_err());
        opts.remove("headers");
        opts.insert(
            "ignored".into(),
            Value::Array(vec![Value::Secret(
                crate::interpreter::SecretValue::new("TEST", "canary").unwrap(),
            )]),
        );
        assert!(http_fetch(&opts).is_err());
        assert_eq!(fixture.finish().len(), 0);
    }

    #[test]
    fn safe_redirect_method_body_matrix() {
        for status in [301, 302, 303, 307, 308] {
            for method in ["POST", "PUT", "HEAD"] {
                let fixture = ChainFixture::new(move |n, _| {
                    if n == 0 {
                        redirect_response(status, "/final")
                    } else {
                        "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
                    }
                });
                let mut opts = fixture.options();
                opts.insert("method".into(), Value::String(method.into()));
                if method != "HEAD" {
                    opts.insert("body".into(), Value::String("payload".into()));
                    // All body sources coexist; GET conversion must remove even
                    // the sources shadowed by the final form serialization.
                    opts.insert(
                        "json".into(),
                        Value::Map(HashMap::from([(
                            "json".into(),
                            Value::String("payload".into()),
                        )])),
                    );
                    opts.insert(
                        "form".into(),
                        Value::Map(HashMap::from([(
                            "form".into(),
                            Value::String("payload".into()),
                        )])),
                    );
                    opts.insert(
                        "headers".into(),
                        Value::Map(HashMap::from([
                            ("Content-Type".into(), Value::String("text/plain".into())),
                            ("Content-Encoding".into(), Value::String("identity".into())),
                        ])),
                    );
                }
                let result = http_fetch(&opts);
                let requests = fixture.finish();
                assert_eq!(response_status(result.unwrap()), 200);
                let converts = method != "HEAD"
                    && (status == 303 || (method == "POST" && matches!(status, 301 | 302)));
                assert!(
                    requests[1].starts_with(&format!(
                        "{} /final ",
                        if converts { "GET" } else { method }
                    )),
                    "status={status} method={method}: {}",
                    requests[1]
                );
                if converts {
                    assert!(!requests[1].contains("payload"));
                    assert!(!requests[1].to_lowercase().contains("content-"));
                } else if method != "HEAD" {
                    assert!(requests[1].ends_with("payload"));
                }
            }
        }
    }

    #[test]
    fn safe_redirect_cross_origin_strips_permanently() {
        let return_url = Arc::new(Mutex::new(String::new()));
        let back = return_url.clone();
        let destination =
            ChainFixture::new(move |_, _| redirect_response(302, &back.lock().unwrap()));
        let target = destination.url.clone();
        let source = ChainFixture::new(move |n, _| {
            if n < 2 {
                redirect_response(302, if n == 0 { "/same" } else { &target })
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
            }
        });
        *return_url.lock().unwrap() = source.url.replace("/start", "/returned");
        let mut opts = source.options();
        opts.insert(
            "auth".into(),
            Value::Map(HashMap::from([
                ("user".into(), Value::String("alice".into())),
                ("pass".into(), Value::String("password".into())),
            ])),
        );
        opts.insert(
            "cookies".into(),
            Value::Map(HashMap::from([(
                "session".into(),
                Value::String("cookie-canary".into()),
            )])),
        );
        opts.insert(
            "headers".into(),
            Value::Map(HashMap::from([
                ("X-Custom".into(), Value::String("custom-canary".into())),
                ("aCcEpT".into(), Value::String("text/plain".into())),
            ])),
        );
        let result = http_fetch(&opts);
        let source_requests = source.finish();
        let target_requests = destination.finish();
        assert_eq!(response_status(result.unwrap()), 200);
        assert_eq!(source_requests.len(), 3);
        assert_eq!(target_requests.len(), 1);
        assert!(source_requests[1].to_lowercase().contains("authorization:"));
        assert!(source_requests[1].contains("cookie-canary"));
        for request in [&source_requests[2], &target_requests[0]] {
            assert!(!request.to_lowercase().contains("authorization:"));
            assert!(!request.contains("cookie-canary"));
            assert!(!request.contains("custom-canary"));
            assert!(request.to_lowercase().contains("accept: text/plain"));
        }
    }

    #[test]
    fn safe_redirect_cross_origin_body_denied_without_contact() {
        for status in [301, 302, 303, 307, 308] {
            let destination =
                ChainFixture::new(|_, _| "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into());
            let target = destination.url.clone();
            let source = ChainFixture::new(move |_, _| redirect_response(status, &target));
            let mut opts = source.options();
            opts.insert("method".into(), Value::String("PUT".into()));
            opts.insert("body".into(), Value::String("private-body".into()));
            let result = http_fetch(&opts);
            assert_eq!(source.finish().len(), 1);
            let requests = destination.finish();
            if status == 303 {
                assert_eq!(response_status(result.unwrap()), 200);
                assert_eq!(requests.len(), 1);
                assert!(!requests[0].contains("private-body"));
            } else {
                assert!(result_error(result.unwrap()).contains("body"));
                assert_eq!(requests.len(), 0);
            }
        }
    }

    #[test]
    fn safe_redirect_fetch_bounds_received_and_decoded_body() {
        if run_http_environment_case("safe_redirect_fetch_bounds_received_and_decoded_body") {
            return;
        }
        std::env::set_var("NTNT_MAX_RESPONSE_SIZE", "8");
        for body in ["123456789", "ééééé"] {
            let fixture = ChainFixture::new(move |_, _| {
                format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n", body.len())
            });
            let result = http_fetch(&fixture.options());
            assert_eq!(fixture.finish().len(), 1);
            assert!(result_error(result.unwrap()).contains("large"));
        }
        // Three invalid bytes expand to nine UTF-8 replacement bytes: received
        // bytes fit, but the decoded result must be rejected before allocation.
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n".to_vec();
        response.extend_from_slice(&[0xff, 0xff, 0xff]);
        let (url, server) = raw_response_fixture(response);
        let result = http_fetch(&HashMap::from([
            ("url".into(), Value::String(url)),
            ("follow_redirects".into(), Value::Bool(true)),
        ]));
        server.join().unwrap();
        assert!(result_error(result.unwrap()).contains("Decoded response body too large"));
        std::env::remove_var("NTNT_MAX_RESPONSE_SIZE");
    }

    #[test]
    fn safe_redirect_download_follows_and_bounds_bytes() {
        if run_http_environment_case("safe_redirect_download_follows_and_bounds_bytes") {
            return;
        }
        std::env::set_var("NTNT_MAX_RESPONSE_SIZE", "8");
        let directory = download_directory("safe-download");
        std::fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("file");
        for body in ["done", "123456789"] {
            std::fs::write(&destination, b"old").unwrap();
            let fixture = ChainFixture::new(move |n, _| {
                if n == 0 {
                    redirect_response(302, "/file")
                } else {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                }
            });
            let result = http_download(
                &fixture.options(),
                destination.to_str().unwrap(),
                DownloadOptions {
                    overwrite: true,
                    create_parent: false,
                },
            );
            assert_eq!(fixture.finish().len(), 2);
            if body == "done" {
                assert_eq!(response_status(result.unwrap()), 200);
                assert_eq!(std::fs::read(&destination).unwrap(), b"done");
            } else {
                assert!(result_error(result.unwrap()).contains("large"));
                assert_eq!(std::fs::read(&destination).unwrap(), b"old");
            }
            assert_no_download_temporary_file(&directory);
        }
        std::fs::remove_dir_all(directory).unwrap();
        std::env::remove_var("NTNT_MAX_RESPONSE_SIZE");
    }

    #[test]
    fn safe_redirect_cache_policy_and_hit_validation() {
        let fixture = ChainFixture::new(|_, request| {
            if request.starts_with("GET /start ") {
                redirect_response(302, "/final")
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone".into()
            }
        });
        let id = get_next_cache_id();
        CACHE_REGISTRY
            .lock()
            .unwrap()
            .insert(id, ResponseCache::new(60));
        let mut opts = fixture.options();
        assert_eq!(
            response_status(cache_fetch(id, &fixture.url, None).unwrap()),
            302
        );
        assert_eq!(
            response_status(cache_fetch(id, &fixture.url, Some(&opts)).unwrap()),
            200
        );
        assert_eq!(
            response_status(cache_fetch(id, &fixture.url, None).unwrap()),
            302
        );
        assert_eq!(
            response_status(cache_fetch(id, &fixture.url, Some(&opts)).unwrap()),
            200
        );
        opts.insert("timeout".into(), Value::Int(0));
        assert!(
            result_error(cache_fetch(id, &fixture.url, Some(&opts)).unwrap()).contains("timeout")
        );
        opts.insert("timeout".into(), Value::Int(-1));
        assert!(cache_fetch(id, &fixture.url, Some(&opts)).is_err());
        opts.remove("timeout");
        opts.insert("max_redirects".into(), Value::Int(0));
        assert!(cache_fetch(id, &fixture.url, Some(&opts)).is_err());
        cache_delete(id, &fixture.url);
        assert!(CACHE_REGISTRY
            .lock()
            .unwrap()
            .get(&id)
            .unwrap()
            .entries
            .is_empty());
        CACHE_REGISTRY.lock().unwrap().remove(&id);
        assert_eq!(fixture.finish().len(), 3);
    }

    #[test]
    fn safe_redirect_download_commit_cleanup_failure_is_success() {
        let directory = download_directory("commit-warning");
        std::fs::create_dir_all(&directory).unwrap();
        let temp = directory.join("temporary");
        let dest = directory.join("destination");
        std::fs::write(&temp, b"committed").unwrap();
        let policy = RedirectPolicy::parse(&HashMap::from([(
            "follow_redirects".into(),
            Value::Bool(true),
        )]))
        .unwrap();
        let result = commit_redirect_download(policy, &temp, &dest, false, |_| {
            Err(std::io::Error::other("injected cleanup failure"))
        });
        assert_eq!(std::fs::read(&dest).unwrap(), b"committed");
        assert!(result
            .unwrap()
            .expect("successful link is committed")
            .unwrap()
            .contains("cleanup"));
        assert!(temp.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn safe_redirect_download_precommit_expiry_and_cancellation() {
        let directory = download_directory("precommit");
        std::fs::create_dir_all(&directory).unwrap();
        let temp = directory.join("temporary");
        let dest = directory.join("destination");
        std::fs::write(&temp, b"new").unwrap();
        std::fs::write(&dest, b"old").unwrap();
        let policy = RedirectPolicy {
            enabled: true,
            limit: 5,
            deadline: Some(Instant::now()),
        };
        assert!(
            commit_redirect_download(policy, &temp, &dest, true, |_| panic!("not committed"))
                .unwrap()
                .unwrap_err()
                .contains("timeout")
        );
        let token = Arc::new(crate::stdlib::concurrent::CancelToken::new());
        token.cancel();
        crate::stdlib::concurrent::CURRENT_CANCEL_TOKEN
            .with(|current| *current.borrow_mut() = Some(token));
        let result =
            commit_redirect_download(policy, &temp, &dest, true, |_| panic!("not committed"));
        crate::stdlib::concurrent::CURRENT_CANCEL_TOKEN
            .with(|current| *current.borrow_mut() = None);
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
        assert_eq!(std::fs::read(&temp).unwrap(), b"new");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn safe_redirect_empty_userinfo_denied() {
        let destination =
            ChainFixture::new(|_, _| "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into());
        let target = destination.url.replace("http://", "http://@");
        let source = ChainFixture::new(move |_, _| redirect_response(302, &target));
        let result = http_fetch(&source.options());
        assert_eq!(source.finish().len(), 1);
        let contacts = destination.finish();
        assert!(result_error(result.unwrap()).contains("userinfo"));
        assert!(contacts.is_empty());
    }

    #[test]
    fn safe_redirect_public_dispatch_variants() {
        let fixture = ChainFixture::new(|_, request| {
            if request.starts_with("GET /start ") {
                redirect_response(302, "/final")
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone".into()
            }
        });
        let module = init();
        let call = |name: &str, args: &[Value]| {
            let Value::NativeFunction { func, .. } = &module[name] else {
                panic!("native function");
            };
            func(args).unwrap()
        };
        assert_eq!(
            response_status(call("fetch", &[Value::Map(fixture.options())])),
            200
        );
        assert_eq!(
            response_status(call(
                "fetch",
                &[
                    Value::String(fixture.url.clone()),
                    Value::Map(fixture.options())
                ]
            )),
            200
        );
        let cache = call("Cache", &[Value::Int(60)]);
        assert_eq!(
            response_status(call("cache_fetch", &[cache, Value::Map(fixture.options())])),
            200
        );
        let directory = download_directory("public-dispatch");
        let path = directory.join("file");
        let options = Value::Map(HashMap::from([("create_parent".into(), Value::Bool(true))]));
        assert_eq!(
            response_status(call(
                "download",
                &[
                    Value::Map(fixture.options()),
                    Value::String(path.display().to_string()),
                    options
                ]
            )),
            200
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"done");
        std::fs::remove_dir_all(directory).unwrap();
        let mut opts = fixture.options();
        opts.insert("follow_redirects".into(), Value::Bool(false));
        assert_eq!(response_status(call("fetch", &[Value::Map(opts)])), 302);
        assert_eq!(fixture.finish().len(), 9);
    }

    #[test]
    fn safe_redirect_protected_proxy_bypass_and_blocked_hop() {
        if run_http_environment_case("safe_redirect_protected_proxy_bypass_and_blocked_hop") {
            return;
        }
        std::env::set_var("NTNT_BLOCKED_HOSTS", "localhost");
        let proxy = ChainFixture::new(|_, _| panic!("protected request reached proxy"));
        std::env::set_var("HTTP_PROXY", &proxy.url);
        std::env::set_var("http_proxy", &proxy.url);
        std::env::set_var("ALL_PROXY", &proxy.url);
        std::env::set_var("NO_PROXY", "");
        std::env::set_var("no_proxy", "");
        let destination =
            ChainFixture::new(|_, _| "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into());
        let target = destination.url.replace("127.0.0.1", "localhost");
        let source = ChainFixture::new(move |_, _| redirect_response(302, &target));
        let result = http_fetch(&source.options());
        assert_eq!(source.finish().len(), 1);
        assert!(destination.finish().is_empty());
        assert!(proxy.finish().is_empty());
        assert!(result_error(result.unwrap()).contains("blocked"));
    }

    #[test]
    fn safe_redirect_resolution_set_is_all_validated() {
        let config = SsrfConfig {
            enabled: true,
            allow_localhost: false,
            allow_private: false,
            blocked_hosts: Vec::new(),
        };
        let public = "8.8.8.8:443".parse().unwrap();
        let blocked = "127.0.0.1:443".parse().unwrap();
        assert!(
            validate_resolved_addresses(&config, "example.test".into(), vec![public, blocked])
                .is_err()
        );
        assert!(validate_resolved_addresses(&config, "example.test".into(), vec![]).is_err());
        let approved = vec![public, "1.1.1.1:443".parse().unwrap()];
        let target =
            validate_resolved_addresses(&config, "example.test".into(), approved.clone()).unwrap();
        assert_eq!(target.addresses, approved);
        assert_eq!(target.host, "example.test");
    }

    #[test]
    fn safe_redirect_whole_chain_timeout() {
        let source = ChainFixture::new(|n, _| {
            thread::sleep(Duration::from_millis(650));
            if n == 0 {
                redirect_response(302, "/final")
            } else {
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n".into()
            }
        });
        let mut opts = source.options();
        opts.insert("timeout".into(), Value::Int(1));
        let result = http_fetch(&opts);
        assert!(!result_error(result.unwrap()).is_empty());
        assert_eq!(source.finish().len(), 2);
    }

    #[test]
    fn safe_redirect_relative_success() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/start", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let count = hits.clone();
        let (stop, done) = mpsc::channel();
        let server = thread::spawn(move || {
            while done.try_recv().is_err() {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        drain_http_request(&mut stream);
                        let n = count.fetch_add(1, Ordering::SeqCst);
                        let response = if n == 0 {
                            "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        } else {
                            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndone"
                        };
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
        });
        let result = http_fetch(&HashMap::from([
            ("url".to_string(), Value::String(url.clone())),
            ("follow_redirects".to_string(), Value::Bool(true)),
        ]));
        stop.send(()).unwrap();
        server.join().unwrap();
        let map = result_map(result.expect("safe opt-in should be accepted"));
        assert!(matches!(map.get("status"), Some(Value::Int(200))));
        assert!(matches!(map.get("redirected"), Some(Value::Bool(true))));
        assert!(matches!(map.get("body"), Some(Value::String(s)) if s == "done"));
        assert!(
            matches!(map.get("url"), Some(Value::String(s)) if s == &url.replace("/start", "/final"))
        );
        assert_eq!(hits.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn secret_bearing_fetch_never_follows_redirects() {
        let fixture = redirect_fixture(307);
        let result = http_fetch_with_app_env(
            &HashMap::from([
                ("url".to_string(), Value::String(fixture.url.clone())),
                ("method".to_string(), Value::String("POST".to_string())),
                ("follow_redirects".to_string(), Value::Bool(false)),
                (
                    "body".to_string(),
                    Value::Secret(
                        crate::interpreter::SecretValue::new(
                            "REDIRECT_SECRET",
                            "secret-body-canary",
                        )
                        .expect("valid secret"),
                    ),
                ),
            ]),
            Some("development"),
        )
        .expect("fetch should return a Result value");

        assert_eq!(response_status(result), 307);
        assert_eq!(fixture.finish(), 0, "secret request must not follow");
    }

    #[test]
    fn fetch_rejects_non_boolean_redirect_policy() {
        let error = http_fetch(&HashMap::from([
            (
                "url".to_string(),
                Value::String("http://127.0.0.1:1/not-requested".to_string()),
            ),
            (
                "follow_redirects".to_string(),
                Value::String("yes".to_string()),
            ),
        ]))
        .expect_err("redirect policy must be typed");

        assert!(error
            .to_string()
            .contains("follow_redirects must be a Bool"));
    }

    #[test]
    fn download_does_not_follow_redirects_or_create_a_file() {
        for status in [301, 302, 303, 307, 308] {
            let fixture = redirect_fixture(status);
            let path = std::env::temp_dir().join(format!(
                "ntnt-redirect-download-{}-{status}",
                std::process::id()
            ));
            let _ = std::fs::remove_file(&path);
            let result = http_download(
                &HashMap::from([("url".to_string(), Value::String(fixture.url.clone()))]),
                path.to_str().expect("UTF-8 temp path"),
                DownloadOptions {
                    overwrite: true,
                    create_parent: true,
                },
            )
            .expect("download should return a Result value");

            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Err"),
                "download must return the {status} response as an error"
            );
            assert!(
                !path.exists(),
                "download must not create a file for {status}"
            );
            assert_eq!(fixture.finish(), 0, "must not follow {status} redirect");
        }
    }

    #[test]
    fn download_posts_request_options_and_streams_binary_bytes() {
        let expected = vec![b'R', b'I', b'F', b'F', 0, 255, 128, b'W', b'A', b'V', b'E'];
        let expected_body = "{\"cfg_scale\":4,\"input\":\"Hello\"}";
        let (url, server) = binary_post_fixture(expected_body, expected.clone());
        let directory = std::env::temp_dir().join(format!(
            "ntnt-binary-download-{}-{}",
            std::process::id(),
            DOWNLOAD_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let destination = directory.join("take.wav");
        let request = HashMap::from([
            ("url".to_string(), Value::String(url)),
            ("method".to_string(), Value::String("POST".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "x-studio".to_string(),
                    Value::String("sound-stage".to_string()),
                )])),
            ),
            (
                "json".to_string(),
                Value::Map(HashMap::from([
                    ("input".to_string(), Value::String("Hello".to_string())),
                    ("cfg_scale".to_string(), Value::Int(4)),
                ])),
            ),
        ]);

        let result = http_download(
            &request,
            destination.to_str().expect("UTF-8 destination"),
            DownloadOptions {
                overwrite: false,
                create_parent: true,
            },
        )
        .expect("download result");

        server.join().expect("binary fixture");
        assert_eq!(std::fs::read(&destination).unwrap(), expected);
        let result = result_map(result);
        assert!(matches!(result.get("status"), Some(Value::Int(200))));
        assert!(matches!(result.get("size"), Some(Value::Int(11))));
        assert!(matches!(result.get("bytes_written"), Some(Value::Int(11))));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn download_non_success_preserves_destination_and_removes_temporary_file() {
        let response = b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 12\r\nConnection: close\r\n\r\ntry tomorrow".to_vec();
        let (url, server) = raw_response_fixture(response);
        let directory = download_directory("failed-download");
        let destination = directory.join("take.wav");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&destination, b"approved take").unwrap();

        let result = http_download(
            &HashMap::from([("url".to_string(), Value::String(url))]),
            destination.to_str().unwrap(),
            DownloadOptions {
                overwrite: true,
                create_parent: false,
            },
        )
        .expect("download result");

        server.join().unwrap();
        assert!(result_error(result).contains("status 503: try tomorrow"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"approved take");
        assert_no_download_temporary_file(&directory);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn download_truncated_body_preserves_destination_and_removes_temporary_file() {
        let response =
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\npartial".to_vec();
        let (url, server) = raw_response_fixture(response);
        let directory = download_directory("truncated-download");
        let destination = directory.join("take.wav");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&destination, b"approved take").unwrap();

        let result = http_download(
            &HashMap::from([("url".to_string(), Value::String(url))]),
            destination.to_str().unwrap(),
            DownloadOptions {
                overwrite: true,
                create_parent: false,
            },
        )
        .expect("download result");

        server.join().unwrap();
        assert!(result_error(result).contains("Failed to read response"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"approved take");
        assert_no_download_temporary_file(&directory);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn download_cancellation_preserves_destination_and_removes_temporary_file() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind cancellation fixture");
        let url = format!("http://{}/audio", listener.local_addr().unwrap());
        let (chunk_sent, chunk_received) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept cancellation request");
            let mut request = [0_u8; 1024];
            let _ = stream
                .read(&mut request)
                .expect("read cancellation request");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                1024 * 1024
            )
            .unwrap();
            stream.write_all(&vec![1_u8; 64 * 1024]).unwrap();
            stream.flush().unwrap();
            chunk_sent.send(()).unwrap();
            thread::sleep(Duration::from_millis(30));
            let _ = stream.write_all(&vec![2_u8; 64 * 1024]);
        });

        let directory = download_directory("cancelled-download");
        let destination = directory.join("take.wav");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&destination, b"approved take").unwrap();
        let token = Arc::new(crate::stdlib::concurrent::CancelToken::new());
        crate::stdlib::concurrent::CURRENT_CANCEL_TOKEN.with(|current| {
            *current.borrow_mut() = Some(Arc::clone(&token));
        });
        let canceller = thread::spawn(move || {
            chunk_received.recv().unwrap();
            token.cancel();
        });

        let result = http_download(
            &HashMap::from([("url".to_string(), Value::String(url))]),
            destination.to_str().unwrap(),
            DownloadOptions {
                overwrite: true,
                create_parent: false,
            },
        );
        crate::stdlib::concurrent::CURRENT_CANCEL_TOKEN.with(|current| {
            *current.borrow_mut() = None;
        });

        canceller.join().unwrap();
        server.join().unwrap();
        assert!(result
            .expect_err("cancelled download must return a runtime error")
            .to_string()
            .contains("download cancelled"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"approved take");
        assert_no_download_temporary_file(&directory);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn request_map_download_defaults_reject_overwrite_and_missing_parent() {
        let request = Value::Map(HashMap::from([(
            "url".to_string(),
            Value::String("http://127.0.0.1:1/audio".to_string()),
        )]));
        let directory = download_directory("safe-download-defaults");
        let destination = directory.join("take.wav");

        let missing_parent = download_from_args(&[
            request.clone(),
            Value::String(destination.to_string_lossy().into_owned()),
        ])
        .expect("download result");
        assert!(result_error(missing_parent).contains("parent directory does not exist"));

        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(&destination, b"approved take").unwrap();
        let existing = download_from_args(&[
            request,
            Value::String(destination.to_string_lossy().into_owned()),
        ])
        .expect("download result");
        assert!(result_error(existing).contains("destination already exists"));
        assert_eq!(std::fs::read(&destination).unwrap(), b"approved take");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_download_creates_parent_and_overwrites_destination() {
        let expected = b"new take".to_vec();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            expected.len()
        )
        .into_bytes()
        .into_iter()
        .chain(expected.clone())
        .collect();
        let (url, server) = raw_response_fixture(response);
        let directory = download_directory("legacy-download");
        let destination = directory.join("nested/take.wav");

        let result = download_from_args(&[
            Value::String(url),
            Value::String(destination.to_string_lossy().into_owned()),
        ])
        .expect("download result");

        server.join().unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), expected);
        assert!(matches!(
            result_map(result).get("bytes_written"),
            Some(Value::Int(8))
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn overwrite_preserves_existing_destination_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let expected = b"replacement".to_vec();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            expected.len()
        )
        .into_bytes()
        .into_iter()
        .chain(expected.clone())
        .collect();
        let (url, server) = raw_response_fixture(response);
        let directory = download_directory("overwrite-permissions");
        std::fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("artifact");
        std::fs::write(&destination, b"original").unwrap();
        std::fs::set_permissions(&destination, std::fs::Permissions::from_mode(0o751)).unwrap();

        let result = download_from_args(&[
            Value::String(url),
            Value::String(destination.to_string_lossy().into_owned()),
        ])
        .expect("download result");

        server.join().unwrap();
        assert!(matches!(
            result_map(result).get("bytes_written"),
            Some(Value::Int(11))
        ));
        assert_eq!(std::fs::read(&destination).unwrap(), expected);
        assert_eq!(
            std::fs::metadata(&destination)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o751
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn secret_transport_requires_https_by_default() {
        let error = validate_secret_transport("http://127.0.0.1:8080/api", true, None)
            .expect_err("secret-bearing HTTP must fail closed without APP_ENV=development");
        assert!(error.to_string().contains("HTTPS"));

        assert!(validate_secret_transport("https://api.example.com/v1", true, None).is_ok());
        assert!(validate_secret_transport("http://api.example.com/v1", false, None).is_ok());
    }

    #[test]
    fn development_allows_secret_http_only_to_loopback_hosts() {
        for url in [
            "http://localhost:8080/api",
            "http://127.0.0.1:8080/api",
            "http://127.42.0.9:8080/api",
            "http://[::1]:8080/api",
        ] {
            assert!(
                validate_secret_transport(url, true, Some("development")).is_ok(),
                "development should allow loopback URL: {url}"
            );
        }

        for url in [
            "http://example.com/api",
            "http://localhost.example.com/api",
            "http://localhost./api",
            "http://10.0.0.8/api",
            "http://192.168.1.10/api",
            "http://0.0.0.0:8080/api",
            "http://[::]:8080/api",
        ] {
            assert!(
                validate_secret_transport(url, true, Some("development")).is_err(),
                "development must reject non-loopback URL: {url}"
            );
        }

        assert!(
            validate_secret_transport("http://localhost:8080/api", true, Some("production"))
                .is_err()
        );
        assert!(validate_secret_transport("http://localhost:8080/api", true, Some("dev")).is_err());
        let invalid = validate_secret_transport("not a URL", true, Some("development"))
            .expect_err("malformed secret-bearing URL must fail");
        assert!(invalid.to_string().contains("valid URL"));

        let ssrf_error = format_ssrf_error("Localhost requests blocked", true);
        assert!(ssrf_error.contains("APP_ENV=development"));
        assert!(ssrf_error.contains("SSRF policy remains independent"));
    }

    #[test]
    fn cache_fetch_rejects_secret_bearing_options_before_cache_lookup() {
        let canary = "cache-secret-canary";
        let options = HashMap::from([
            (
                "url".to_string(),
                Value::String("http://127.0.0.1:1/never-requested".to_string()),
            ),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "Authorization".to_string(),
                    Value::Secret(
                        crate::interpreter::SecretValue::new("CACHE_SECRET", canary)
                            .expect("valid secret"),
                    ),
                )])),
            ),
        ]);

        let error = cache_fetch(0, "http://127.0.0.1:1/never-requested", Some(&options))
            .expect_err("secret-bearing requests must not use the response cache");
        assert!(!error.to_string().contains(canary));
    }
}
