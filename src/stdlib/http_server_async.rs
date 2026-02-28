//! Async HTTP Server module for NTNT
//!
//! High-concurrency HTTP server using Axum + Tokio for production workloads.
//!
//! ## Architecture (DD-006: Per-Request Interpreter)
//!
//! Each HTTP request gets its own `Interpreter` instance via `spawn_blocking`.
//! No bridge channel, no single interpreter thread — true parallel execution.
//!
//! ## Features
//!
//! - Per-request interpreter instances for true parallelism
//! - High-concurrency via Tokio async runtime
//! - Static file serving with caching headers
//! - Request timeouts
//! - Gzip compression
//! - Graceful shutdown
//!
//! ## Usage
//!
//! ```bash
//! ntnt run server.tnt
//! ```

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use axum::{
    body::Body,
    extract::State,
    http::{header, Request, StatusCode},
    response::{IntoResponse, Response},
    Router,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tower_http::{compression::CompressionLayer, timeout::TimeoutLayer, trace::TraceLayer};

use super::http_server::{get_default_security_headers, SharedState};

/// A serializable HTTP request that can be sent across thread boundaries
#[derive(Debug, Clone)]
pub struct BridgeRequest {
    pub method: String,
    pub path: String,
    pub url: String,
    pub query: String,
    pub query_params: HashMap<String, String>,
    pub params: HashMap<String, String>,
    pub headers: HashMap<String, String>,
    pub body: String,
    pub id: String,
    pub ip: String,
    pub protocol: String,
}

impl BridgeRequest {
    /// Convert to NTNT Value for handler invocation
    pub fn to_value(&self) -> Value {
        let mut map: HashMap<String, Value> = HashMap::new();

        map.insert("method".to_string(), Value::String(self.method.clone()));
        map.insert("path".to_string(), Value::String(self.path.clone()));
        map.insert("url".to_string(), Value::String(self.url.clone()));
        map.insert("query".to_string(), Value::String(self.query.clone()));
        map.insert("body".to_string(), Value::String(self.body.clone()));
        map.insert("id".to_string(), Value::String(self.id.clone()));
        map.insert("ip".to_string(), Value::String(self.ip.clone()));
        map.insert("protocol".to_string(), Value::String(self.protocol.clone()));

        let query_params: HashMap<String, Value> = self
            .query_params
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        map.insert("query_params".to_string(), Value::Map(query_params));

        let params: HashMap<String, Value> = self
            .params
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        map.insert("params".to_string(), Value::Map(params));

        let headers: HashMap<String, Value> = self
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        map.insert("headers".to_string(), Value::Map(headers));

        map.insert("context".to_string(), Value::Map(HashMap::new()));

        Value::Map(map)
    }
}

/// A serializable HTTP response that can be sent back from the interpreter
#[derive(Debug, Clone)]
pub struct BridgeResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl BridgeResponse {
    /// Create from NTNT Value (handler response)
    pub fn from_value(value: &Value) -> Self {
        match value {
            Value::Map(map) => {
                let status = match map.get("status") {
                    Some(Value::Int(s)) => *s as u16,
                    _ => 200,
                };

                let body = match map.get("body") {
                    Some(Value::String(b)) => b.clone(),
                    _ => String::new(),
                };

                let mut headers = Vec::new();
                if let Some(Value::Map(h)) = map.get("headers") {
                    for (k, v) in h {
                        match v {
                            Value::String(val) => {
                                headers.push((k.clone(), val.clone()));
                            }
                            Value::Array(arr) => {
                                for item in arr {
                                    if let Value::String(val) = item {
                                        headers.push((k.clone(), val.clone()));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                BridgeResponse {
                    status,
                    headers,
                    body,
                }
            }
            _ => BridgeResponse {
                status: 500,
                headers: Vec::new(),
                body: "Handler did not return a valid response".to_string(),
            },
        }
    }

    /// Create an error response
    pub fn error(status: u16, message: &str) -> Self {
        BridgeResponse {
            status,
            headers: vec![(
                "content-type".to_string(),
                "text/plain; charset=utf-8".to_string(),
            )],
            body: message.to_string(),
        }
    }

    /// Create a not found response
    pub fn not_found() -> Self {
        Self::error(404, "Not Found")
    }
}

/// Apply security headers to a built Response<Body>.
/// Only adds headers not already set by the application (app can override).
fn apply_async_security_headers(response: &mut Response<Body>) {
    let disabled = std::env::var("NTNT_SECURITY_HEADERS")
        .map(|v| v == "0" || v.to_lowercase() == "false")
        .unwrap_or(false);
    if disabled {
        return;
    }
    let security_headers = get_default_security_headers();
    let headers = response.headers_mut();
    for (key, value) in security_headers {
        if let Ok(name) = header::HeaderName::try_from(key.as_str()) {
            if !headers.contains_key(&name) {
                if let Value::String(val) = value {
                    if let Ok(hv) = header::HeaderValue::from_str(&val) {
                        headers.insert(name, hv);
                    }
                }
            }
        }
    }
}

/// Route segment for pattern matching (mirrors sync version)
#[derive(Debug, Clone)]
pub enum RouteSegment {
    Static(String),
    Param(String),
}

/// Compiled route with parsed pattern
#[derive(Debug, Clone)]
pub struct Route {
    pub method: String,
    pub pattern: String,
    pub segments: Vec<RouteSegment>,
}

/// Route registration - stores pattern info only (not Values)
#[derive(Debug, Clone)]
pub struct RouteInfo {
    pub route: Route,
    pub handler_name: String,
}

/// Static directory configuration
#[derive(Debug, Clone)]
pub struct StaticDir {
    pub url_prefix: String,
    pub fs_path: String,
}

/// Async server state - thread-safe route registry
pub struct AsyncServerState {
    /// Routes with handler names (not actual handlers)
    pub routes: RwLock<Vec<RouteInfo>>,
    /// Static file directories (url_prefix, filesystem_path)
    pub static_dirs: RwLock<Vec<StaticDir>>,
}

impl AsyncServerState {
    pub fn new() -> Self {
        AsyncServerState {
            routes: RwLock::new(Vec::new()),
            static_dirs: RwLock::new(Vec::new()),
        }
    }

    /// Register a route pattern
    pub async fn register_route(&self, method: &str, pattern: &str, handler_name: &str) {
        let route = Route {
            method: method.to_string(),
            pattern: pattern.to_string(),
            segments: parse_route_pattern(pattern),
        };
        let info = RouteInfo {
            route,
            handler_name: handler_name.to_string(),
        };
        let mut routes = self.routes.write().await;
        routes.push(info);
    }

    /// Register a static directory
    pub async fn register_static_dir(&self, url_prefix: &str, fs_path: &str) {
        let mut dirs = self.static_dirs.write().await;
        dirs.push(StaticDir {
            url_prefix: url_prefix.to_string(),
            fs_path: fs_path.to_string(),
        });
    }

    /// Find a matching route and return handler name + params
    pub async fn find_route(
        &self,
        method: &str,
        path: &str,
    ) -> Option<(String, HashMap<String, String>)> {
        let routes = self.routes.read().await;
        for info in routes.iter() {
            if info.route.method == method {
                if let Some(params) = match_route(path, &info.route) {
                    return Some((info.handler_name.clone(), params));
                }
            }
        }
        None
    }

    /// Check if path matches a static directory
    ///
    /// Security: validates against path traversal attacks by rejecting ".." segments,
    /// null bytes, and encoded traversal patterns, then verifying the canonical path
    /// stays within the base directory.
    /// @since v0.3.14
    pub async fn find_static_file(&self, path: &str) -> Option<(String, String)> {
        let dirs = self.static_dirs.read().await;
        for dir in dirs.iter() {
            if path.starts_with(&dir.url_prefix) {
                let relative = path.strip_prefix(&dir.url_prefix).unwrap_or("");
                let relative = relative.trim_start_matches('/');

                // Security: reject path traversal attempts
                if relative.contains("..") || relative.contains('\0') {
                    return None;
                }

                // Check for encoded traversal patterns
                let decoded = urlencoding::decode(relative).unwrap_or_else(|_| relative.into());
                if decoded.contains("..") {
                    return None;
                }

                let file_path = PathBuf::from(&dir.fs_path).join(relative);

                // Security: verify canonical path stays within base directory
                if let Ok(canonical) = file_path.canonicalize() {
                    if let Ok(base_canonical) = std::path::Path::new(&dir.fs_path).canonicalize() {
                        if canonical.starts_with(&base_canonical) && canonical.is_file() {
                            return Some((
                                canonical.to_string_lossy().to_string(),
                                dir.url_prefix.clone(),
                            ));
                        }
                    }
                    // File exists but outside base directory — reject
                    return None;
                }

                // File doesn't exist — no match
            }
        }
        None
    }

    /// Get route count
    pub async fn route_count(&self) -> usize {
        self.routes.read().await.len()
    }

    /// Get static dir count
    pub async fn static_dir_count(&self) -> usize {
        self.static_dirs.read().await.len()
    }

    /// Clear all routes (for hot-reload)
    pub async fn clear_routes(&self) {
        let mut routes = self.routes.write().await;
        routes.clear();
    }

    /// Clear all static directories (for hot-reload)
    pub async fn clear_static_dirs(&self) {
        let mut dirs = self.static_dirs.write().await;
        dirs.clear();
    }

    /// Clear all state (routes and static dirs) for hot-reload
    pub async fn clear(&self) {
        self.clear_routes().await;
        self.clear_static_dirs().await;
    }

    /// Synchronous version of clear for use from non-async context
    pub fn clear_blocking(&self, rt: &tokio::runtime::Runtime) {
        rt.block_on(self.clear());
    }

    /// Synchronous version of register_route for use from non-async context
    pub fn register_route_blocking(
        &self,
        rt: &tokio::runtime::Runtime,
        method: &str,
        pattern: &str,
        handler_name: &str,
    ) {
        rt.block_on(self.register_route(method, pattern, handler_name));
    }

    /// Synchronous version of register_static_dir for use from non-async context
    pub fn register_static_dir_blocking(
        &self,
        rt: &tokio::runtime::Runtime,
        url_prefix: &str,
        fs_path: &str,
    ) {
        rt.block_on(self.register_static_dir(url_prefix, fs_path));
    }
}

impl Default for AsyncServerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a route pattern into segments
pub fn parse_route_pattern(pattern: &str) -> Vec<RouteSegment> {
    pattern
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                RouteSegment::Param(segment[1..segment.len() - 1].to_string())
            } else {
                RouteSegment::Static(segment.to_string())
            }
        })
        .collect()
}

/// Match a URL path against a route
pub fn match_route(path: &str, route: &Route) -> Option<HashMap<String, String>> {
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let route_segments = &route.segments;

    // Handle root path specially
    if path == "/" && route_segments.is_empty() {
        return Some(HashMap::new());
    }

    if path_segments.len() != route_segments.len() {
        return None;
    }

    let mut params = HashMap::new();

    for (path_seg, route_seg) in path_segments.iter().zip(route_segments.iter()) {
        match route_seg {
            RouteSegment::Static(expected) => {
                if path_seg != expected {
                    return None;
                }
            }
            RouteSegment::Param(name) => {
                params.insert(name.clone(), path_seg.to_string());
            }
        }
    }

    Some(params)
}

/// Convert Axum request to BridgeRequest
async fn axum_to_bridge_request(
    req: Request<Body>,
    params: HashMap<String, String>,
) -> Result<BridgeRequest> {
    let method = req.method().to_string();
    let uri = req.uri();
    let path = uri.path().to_string();
    let url = uri.to_string();
    let query = uri.query().unwrap_or("").to_string();

    // Parse query params (URL-decode values)
    let mut query_params = HashMap::new();
    if !query.is_empty() {
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                // URL-decode the value to handle encoded characters like %2F -> /
                let decoded_value = urlencoding::decode(value)
                    .unwrap_or_else(|_| value.into())
                    .to_string();
                query_params.insert(key.to_string(), decoded_value);
            }
        }
    }

    // Extract headers
    let mut headers = HashMap::new();
    let mut client_ip = None;
    for (name, value) in req.headers() {
        if let Ok(v) = value.to_str() {
            let key = name.to_string().to_lowercase();
            if key == "x-forwarded-for" {
                client_ip = Some(v.split(',').next().unwrap_or(v).trim().to_string());
            }
            headers.insert(key, v.to_string());
        }
    }

    // Read body
    let body_bytes = axum::body::to_bytes(req.into_body(), 10 * 1024 * 1024)
        .await
        .map_err(|e| IntentError::RuntimeError(format!("Failed to read body: {}", e)))?;
    let body = String::from_utf8_lossy(&body_bytes).to_string();

    Ok(BridgeRequest {
        method,
        path,
        url,
        query,
        query_params,
        params,
        headers,
        body,
        id: uuid::Uuid::new_v4().to_string(),
        ip: client_ip.unwrap_or_else(|| "unknown".to_string()),
        protocol: "http".to_string(),
    })
}

/// Convert BridgeResponse to Axum response
fn bridge_to_axum_response(resp: BridgeResponse) -> Response<Body> {
    let status = StatusCode::from_u16(resp.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);

    let mut response = Response::builder().status(status);

    for (key, value) in resp.headers {
        if let Ok(name) = header::HeaderName::try_from(key.as_str()) {
            if let Ok(val) = header::HeaderValue::from_str(&value) {
                response = response.header(name, val);
            }
        }
    }

    // Add server header
    response = response.header("server", "ntnt-async");

    let mut resp = response.body(Body::from(resp.body)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("Failed to build response"))
            .unwrap()
    });
    apply_async_security_headers(&mut resp);
    resp
}

/// Serve a static file with proper headers
fn serve_static_file(file_path: &str, if_none_match: Option<&str>) -> Response<Body> {
    use std::fs;

    let path = std::path::Path::new(file_path);
    let metadata = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header("content-type", "text/plain")
                .body(Body::from("File not found"))
                .unwrap();
        }
    };

    // Generate ETag from file size + modification time
    let etag = if let Ok(mtime) = metadata.modified() {
        let duration = mtime
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        format!(
            "\"{}_{}_{}\"",
            metadata.len(),
            duration.as_secs(),
            duration.subsec_nanos()
        )
    } else {
        format!("\"size-{}\"", metadata.len())
    };

    // Check If-None-Match — return 304 if ETag matches
    if let Some(client_etag) = if_none_match {
        if client_etag == etag || client_etag == "*" {
            let mut resp = Response::builder()
                .status(StatusCode::NOT_MODIFIED)
                .header("etag", &etag)
                .header("cache-control", cache_control_for(file_path))
                .header("server", "ntnt-async")
                .body(Body::empty())
                .unwrap();
            apply_async_security_headers(&mut resp);
            return resp;
        }
    }

    let mut resp = match fs::read(file_path) {
        Ok(contents) => {
            let mime_type = guess_mime_type(file_path);
            let len = contents.len();

            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", mime_type)
                .header("content-length", len)
                .header("etag", &etag)
                .header("cache-control", cache_control_for(file_path))
                .header("server", "ntnt-async")
                .body(Body::from(contents))
                .unwrap_or_else(|_| {
                    Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Body::from("Failed to read file"))
                        .unwrap()
                })
        }
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "text/plain")
            .body(Body::from("File not found"))
            .unwrap(),
    };
    apply_async_security_headers(&mut resp);
    resp
}

/// Returns appropriate Cache-Control header based on file type.
/// Images/fonts get long cache (1 year, immutable). CSS/JS get 1 day. HTML gets no-cache.
fn cache_control_for(file_path: &str) -> &'static str {
    let ext = file_path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "svg" | "ico" | "webp" | "avif" | "woff" | "woff2"
        | "ttf" | "eot" => "public, max-age=31536000, immutable",
        "css" | "js" => "public, max-age=86400",
        "html" | "htm" => "no-cache",
        _ => "public, max-age=3600",
    }
}

/// Guess MIME type from file extension
fn guess_mime_type(path: &str) -> &'static str {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "pdf" => "application/pdf",
        "txt" => "text/plain; charset=utf-8",
        "xml" => "application/xml",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Generate an error response with proper HTML page.
/// In dev mode: shows full error details for debugging.
/// In prod mode: shows a clean, user-friendly page without internals.
/// Always logs structured error to stderr regardless of mode.
fn error_response(
    status: u16,
    error: &str,
    method: &str,
    path: &str,
    handler: &str,
    is_production: bool,
) -> Response<Body> {
    let status_text = match status {
        400 => "Bad Request",
        404 => "Not Found",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Error",
    };

    let body = if is_production {
        // Production: clean page, no internals leaked
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{status} {status_text}</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:system-ui,-apple-system,sans-serif;background:#09090b;color:#fafafa;display:flex;align-items:center;justify-content:center;min-height:100vh;padding:2rem}}
.err{{text-align:center;max-width:480px}}
.code{{font-size:6rem;font-weight:800;color:#c084fc;line-height:1}}
.msg{{font-size:1.25rem;color:#a1a1aa;margin:1rem 0 2rem}}
a{{color:#c084fc;text-decoration:none}}a:hover{{text-decoration:underline}}
</style>
</head>
<body><div class="err">
<div class="code">{status}</div>
<div class="msg">{status_text}</div>
<p style="color:#52525b;font-size:0.85rem;margin-bottom:1.5rem">Something went wrong processing your request.</p>
<a href="/">← Back to Home</a>
</div></body></html>"#,
            status = status,
            status_text = status_text,
        )
    } else {
        // Dev mode: full error details for debugging
        let handler_info = if handler.is_empty() {
            String::new()
        } else {
            format!(
                r#"<div class="detail"><span class="label">Handler</span><span class="value">{}</span></div>"#,
                html_escape_str(handler)
            )
        };

        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>{status} {status_text} — ntnt</title>
<style>
*{{margin:0;padding:0;box-sizing:border-box}}
body{{font-family:system-ui,-apple-system,sans-serif;background:#09090b;color:#fafafa;padding:2rem}}
.container{{max-width:800px;margin:0 auto}}
.header{{display:flex;align-items:baseline;gap:1rem;margin-bottom:1.5rem;padding-bottom:1rem;border-bottom:1px solid #27272a}}
.code{{font-size:3rem;font-weight:800;color:#ef4444;line-height:1}}
.title{{font-size:1.5rem;color:#a1a1aa}}
.error-box{{background:#1c1c1e;border:1px solid #ef4444;border-radius:12px;padding:1.25rem;margin-bottom:1.5rem;font-family:'JetBrains Mono',monospace;font-size:0.9rem;line-height:1.6;color:#fca5a5;white-space:pre-wrap;word-break:break-word;overflow-x:auto}}
.details{{background:#18181b;border:1px solid #27272a;border-radius:12px;padding:1.25rem;margin-bottom:1.5rem}}
.detail{{display:flex;gap:1rem;padding:0.5rem 0;border-bottom:1px solid #1e1e21}}
.detail:last-child{{border-bottom:none}}
.label{{color:#71717a;font-size:0.8rem;text-transform:uppercase;letter-spacing:0.05em;min-width:80px;flex-shrink:0}}
.value{{font-family:'JetBrains Mono',monospace;font-size:0.85rem;color:#e4e4e7}}
.hint{{background:#1a1a2e;border:1px solid #312e81;border-radius:12px;padding:1.25rem;color:#a5b4fc;font-size:0.85rem;line-height:1.6}}
.hint strong{{color:#c084fc}}
.footer{{margin-top:2rem;color:#3f3f46;font-size:0.75rem;text-align:center}}
</style>
</head>
<body><div class="container">
<div class="header"><div class="code">{status}</div><div class="title">{status_text}</div></div>
<div class="error-box">{error}</div>
<div class="details">
<div class="detail"><span class="label">Route</span><span class="value">{method} {path}</span></div>
{handler_info}
</div>
<div class="hint">
<strong>💡 Dev Mode</strong> — This page is only shown when <code>NTNT_ENV</code> is not set to <code>production</code>.<br>
Set <code>NTNT_ENV=production</code> to show a clean error page to users.
</div>
<div class="footer">ntnt error handler · <a href="/" style="color:#52525b">home</a></div>
</div></body></html>"#,
            status = status,
            status_text = status_text,
            error = html_escape_str(error),
            method = method,
            path = html_escape_str(path),
            handler_info = handler_info,
        )
    };

    let status_code = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut resp = Response::builder()
        .status(status_code)
        .header("content-type", "text/html; charset=utf-8")
        .header("server", "ntnt-async")
        .body(Body::from(body))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::from("Internal Server Error"))
                .unwrap()
        });
    apply_async_security_headers(&mut resp);
    resp
}

/// Escape HTML special characters in error messages
fn html_escape_str(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Configuration for the async server
#[derive(Clone)]
pub struct AsyncServerConfig {
    pub port: u16,
    pub host: String,
    pub enable_compression: bool,
    pub request_timeout_secs: u64,
    pub max_connections: usize,
}

impl Default for AsyncServerConfig {
    fn default() -> Self {
        AsyncServerConfig {
            port: 8080,
            host: "0.0.0.0".to_string(),
            enable_compression: true,
            request_timeout_secs: 30,
            max_connections: 10_000,
        }
    }
}

/// State for the per-request server (DD-006 Phase 4)
#[derive(Clone)]
pub struct PerRequestState {
    pub shared: Arc<std::sync::RwLock<super::http_server::SharedState>>,
    pub is_production: bool,
    pub request_timeout_secs: u64,
}

/// Start the per-request HTTP server (DD-006 Phase 4).
///
/// Each request gets its own `Interpreter` via `spawn_blocking`.
/// No bridge channel — true parallel execution.
pub async fn start_per_request_server(
    config: AsyncServerConfig,
    shared: Arc<std::sync::RwLock<super::http_server::SharedState>>,
    test_shutdown_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|e| IntentError::RuntimeError(format!("Invalid address: {}", e)))?;

    let route_count = shared.read().unwrap().route_count();
    let static_count = shared.read().unwrap().static_dirs.len();

    let is_production = std::env::var("NTNT_ENV")
        .map(|v| v == "production" || v == "prod")
        .unwrap_or(false);

    // Spawn hot-reload watcher if not in production
    if !is_production {
        let source_file = shared.read().unwrap().main_source_file.clone();
        if let Some(source_file) = source_file {
            let poll_ms: u64 = std::env::var("NTNT_HOT_RELOAD_INTERVAL_MS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(500);
            let shared_clone = shared.clone();
            tokio::spawn(hot_reload_watcher(
                shared_clone,
                source_file,
                Duration::from_millis(poll_ms),
            ));
        }
    }

    let state = PerRequestState {
        shared: shared.clone(),
        is_production,
        request_timeout_secs: config.request_timeout_secs,
    };

    // Build the router with catch-all handler
    let mut app = Router::new().fallback(handle_per_request).with_state(state);

    // Add middleware layers
    app = app.layer(TimeoutLayer::new(Duration::from_secs(
        config.request_timeout_secs,
    )));
    if config.enable_compression {
        app = app.layer(CompressionLayer::new());
    }
    app = app.layer(TraceLayer::new_for_http());

    let display_url = if addr.ip().is_unspecified() {
        format!("http://localhost:{}", addr.port())
    } else {
        format!("http://{}", addr)
    };

    println!();
    println!("🚀 Server running — visit {}", display_url);
    println!(
        "   Routes: {}  |  Static: {}  |  Mode: per-request",
        route_count, static_count
    );
    if !is_production {
        println!("   Hot-reload: enabled (background watcher)");
    }
    println!();
    println!("Press Ctrl+C to stop");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| IntentError::RuntimeError(format!("Failed to bind: {}", e)))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal_with_handlers(shared, test_shutdown_flag))
        .await
        .map_err(|e| IntentError::RuntimeError(format!("Server error: {}", e)))
}

/// Handle a request using per-request interpreter (DD-006 Phase 4).
///
/// Route lookup happens under a read lock, then lock is released.
/// Handler execution happens in spawn_blocking with a fresh Interpreter.
async fn handle_per_request(
    State(state): State<PerRequestState>,
    req: Request<Body>,
) -> impl IntoResponse {
    use crate::interpreter::Interpreter;

    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let if_none_match = req
        .headers()
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // Read lock to find route + get handler/middleware
    let lookup_result = {
        let shared = state.shared.read().unwrap_or_else(|e| e.into_inner());

        // Check static files first for GET requests
        if method == axum::http::Method::GET {
            for (url_prefix, fs_path) in &shared.static_dirs {
                if path.starts_with(url_prefix) {
                    let relative = path.strip_prefix(url_prefix).unwrap_or("");
                    let relative = relative.trim_start_matches('/');
                    if !relative.contains("..") && !relative.contains('\0') {
                        let decoded =
                            urlencoding::decode(relative).unwrap_or_else(|_| relative.into());
                        if !decoded.contains("..") {
                            let file_path = std::path::PathBuf::from(fs_path).join(relative);
                            if let Ok(canonical) = file_path.canonicalize() {
                                if let Ok(base_canonical) =
                                    std::path::Path::new(fs_path).canonicalize()
                                {
                                    if canonical.starts_with(&base_canonical) && canonical.is_file()
                                    {
                                        return serve_static_file(
                                            &canonical.to_string_lossy(),
                                            if_none_match.as_deref(),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Get request origin for CORS
        let request_origin = req
            .headers()
            .get("origin")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        // Handle CORS preflight
        if method == axum::http::Method::OPTIONS {
            if let Some(cors_config) = shared.get_cors_config() {
                let preflight = cors_config.create_preflight_response(request_origin.as_deref());
                let bridge_resp = BridgeResponse::from_value(&preflight);
                return bridge_to_axum_response(bridge_resp);
            }
        }

        // Find route
        let route_result = shared.find_route_typed(method.as_str(), &path);

        match route_result {
            super::http_server::RouteMatchResult::Matched {
                handler, params, ..
            } => {
                let middleware = shared.get_middleware().to_vec();
                let cors_config = shared.get_cors_config().cloned();
                Some((handler, params, middleware, cors_config, request_origin))
            }
            super::http_server::RouteMatchResult::TypeMismatch {
                param_name,
                expected,
                got,
            } => {
                let error_msg = format!(
                    "Bad Request: Parameter '{}' must be type {}, got '{}'",
                    param_name, expected, got
                );
                let mut bad_request = super::http_server::create_error_response(400, &error_msg);
                if let Some(cors_config) = shared.get_cors_config() {
                    if let Value::Map(ref mut resp_map) = bad_request {
                        cors_config.apply_to_response(resp_map, request_origin.as_deref());
                    }
                }
                let bridge_resp = BridgeResponse::from_value(&bad_request);
                return bridge_to_axum_response(bridge_resp);
            }
            super::http_server::RouteMatchResult::NotFound => {
                // Check static files for non-GET too (already checked GET above)
                let cors_config = shared.get_cors_config().cloned();
                let mut not_found = super::http_server::create_error_response(
                    404,
                    &format!("Not Found: {} {}", method, path),
                );
                if let Some(ref cc) = cors_config {
                    if let Value::Map(ref mut resp_map) = not_found {
                        cc.apply_to_response(resp_map, request_origin.as_deref());
                    }
                }
                let bridge_resp = BridgeResponse::from_value(&not_found);
                return bridge_to_axum_response(bridge_resp);
            }
        }
    };
    // Lock released here

    let (handler, route_params, middleware, cors_config, request_origin) = lookup_result.unwrap();

    // Convert Axum request to BridgeRequest
    let bridge_req = match axum_to_bridge_request(req, route_params).await {
        Ok(r) => r,
        Err(e) => {
            let error_msg = format!("{}", e);
            eprintln!(
                "[ERROR] {} {} | request parse | {}",
                method, path, error_msg
            );
            return error_response(
                400,
                &error_msg,
                &method.to_string(),
                &path,
                "",
                state.is_production,
            );
        }
    };

    // Execute in spawn_blocking — fresh interpreter per request
    let shared_clone = state.shared.clone();
    let method_str = method.to_string();
    let path_clone = path.clone();

    // Convert CORS config to Send-safe data before moving into spawn_blocking
    let cors_origin = request_origin.clone();
    let cors_config_clone = cors_config.clone();

    let join_result = tokio::task::spawn_blocking(move || {
        // Convert to Value inside spawn_blocking (Value is !Send, can't cross boundary)
        let req_value = bridge_req.to_value();

        // Read shared state to seed interpreter (brief read lock)
        let shared_guard = shared_clone.read().unwrap_or_else(|e| e.into_inner());
        let result = Interpreter::execute_request(&shared_guard, &handler, &middleware, req_value);
        drop(shared_guard);

        // Convert to BridgeResponse inside spawn_blocking so Result<Value> (which is !Send)
        // never crosses the thread boundary. Only BridgeResponse (Send) is returned.
        match result {
            Ok(mut response) => {
                // Apply CORS headers if enabled
                if let Some(ref cc) = cors_config_clone {
                    if let Value::Map(ref mut resp_map) = response {
                        cc.apply_to_response(resp_map, cors_origin.as_deref());
                    }
                }
                BridgeResponse::from_value(&response)
            }
            Err(e) => {
                let error_msg = e.to_string();
                eprintln!(
                    "[ERROR] {} {} | handler | {}",
                    method_str, path_clone, error_msg
                );
                BridgeResponse::error(500, &error_msg)
            }
        }
    })
    .await;

    match join_result {
        Ok(bridge_resp) => bridge_to_axum_response(bridge_resp),
        Err(join_err) => {
            // Handler panicked — return 500 instead of crashing
            eprintln!(
                "[ERROR] {} {} | handler panicked: {}",
                method, path, join_err
            );
            let bridge_resp = BridgeResponse::error(500, "Internal Server Error: handler panicked");
            bridge_to_axum_response(bridge_resp)
        }
    }
}

/// Shutdown signal handler that runs shutdown handlers before exiting.
async fn shutdown_signal_with_handlers(
    shared: Arc<std::sync::RwLock<super::http_server::SharedState>>,
    test_shutdown_flag: Option<Arc<std::sync::atomic::AtomicBool>>,
) {
    // Wait for either Ctrl-C/SIGTERM or the test-mode shutdown flag
    if let Some(flag) = test_shutdown_flag {
        loop {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    } else {
        shutdown_signal().await;
    }

    // Run shutdown handlers in a blocking task
    let _ = tokio::task::spawn_blocking(move || {
        crate::interpreter::Interpreter::run_shutdown_handlers(
            &shared.read().unwrap_or_else(|e| e.into_inner()),
        );
    })
    .await;
}

/// Signal handler for graceful shutdown
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    println!("\n🛑 Shutdown signal received, stopping server...");
}

// === Helper functions for creating NTNT response Values ===

/// Create a JSON response Value
pub fn create_json_response(data: &Value, status: i64) -> Value {
    let json_value = crate::stdlib::json::intent_value_to_json(data);
    let json_string = json_value.to_string();
    let mut headers = HashMap::new();
    // Default security headers
    headers.insert(
        "x-content-type-options".to_string(),
        Value::String("nosniff".to_string()),
    );
    headers.insert(
        "x-frame-options".to_string(),
        Value::String("DENY".to_string()),
    );
    headers.insert(
        "referrer-policy".to_string(),
        Value::String("strict-origin-when-cross-origin".to_string()),
    );
    headers.insert(
        "content-type".to_string(),
        Value::String("application/json".to_string()),
    );
    headers.insert(
        "cache-control".to_string(),
        Value::String("no-cache, no-store, must-revalidate".to_string()),
    );

    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(status));
    response.insert("headers".to_string(), Value::Map(headers));
    response.insert("body".to_string(), Value::String(json_string));
    Value::Map(response)
}

/// Create an error response Value
pub fn create_error_response(status: i64, message: &str) -> Value {
    let mut headers = HashMap::new();
    headers.insert(
        "content-type".to_string(),
        Value::String("text/plain; charset=utf-8".to_string()),
    );

    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(status));
    response.insert("headers".to_string(), Value::Map(headers));
    response.insert("body".to_string(), Value::String(message.to_string()));
    Value::Map(response)
}

/// Rebuild SharedState from a source file by re-parsing and re-evaluating everything.
///
/// Creates a fresh Interpreter, evaluates the .tnt source (which registers routes,
/// middleware, etc.), then extracts the resulting SharedState including type context.
/// This is a full startup re-run — no diffing of old state.
pub fn rebuild_shared_state(source_file: &str) -> Result<SharedState> {
    use crate::interpreter::Interpreter;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    let source = std::fs::read_to_string(source_file).map_err(|e| {
        IntentError::RuntimeError(format!(
            "Failed to read source file '{}': {}",
            source_file, e
        ))
    })?;

    let mut interp = Interpreter::new();
    interp.set_main_source_file(source_file);

    // Set test_mode so listen() captures SharedState instead of starting a server
    let shutdown_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    interp.set_test_mode(0, 0, shutdown_flag);

    let lexer = Lexer::new(&source);
    let tokens: Vec<_> = lexer.collect();
    let mut parser = Parser::new(tokens);
    let ast = parser
        .parse()
        .map_err(|e| IntentError::RuntimeError(format!("Parse error during hot-reload: {}", e)))?;

    // Eval registers routes/middleware/etc. into server_state.
    // listen() in test_mode returns immediately without starting a server.
    let _ = interp.eval(&ast);

    // Copy type context from interpreter to server_state
    interp.server_state.structs = interp.structs.clone();
    interp.server_state.enums = interp.enums.clone();
    interp.server_state.type_aliases = interp.type_aliases.clone();
    interp.server_state.trait_definitions = interp.trait_definitions.clone();
    interp.server_state.trait_implementations = interp.trait_implementations.clone();
    interp.server_state.main_source_file = Some(source_file.to_string());

    Ok(std::mem::take(&mut interp.server_state))
}

/// Collect modification times for all .tnt files in a directory tree.
fn collect_file_mtimes(
    source_file: &str,
    shared: &SharedState,
) -> HashMap<PathBuf, std::time::SystemTime> {
    let mut mtimes = HashMap::new();

    // Track the main source file
    let main_path = PathBuf::from(source_file);
    if let Ok(meta) = std::fs::metadata(&main_path) {
        if let Ok(mtime) = meta.modified() {
            mtimes.insert(main_path, mtime);
        }
    }

    // Track routes directory
    if let Some(ref routes_dir) = shared.routes_dir {
        collect_tnt_mtimes_recursive(&PathBuf::from(routes_dir), &mut mtimes);
    }

    // Track lib modules
    for lib_path in &shared.lib_modules {
        let p = PathBuf::from(lib_path);
        if let Ok(meta) = std::fs::metadata(&p) {
            if let Ok(mtime) = meta.modified() {
                mtimes.insert(p, mtime);
            }
        }
    }

    // Track middleware files
    for mw_path in &shared.middleware_files {
        let p = PathBuf::from(mw_path);
        if let Ok(meta) = std::fs::metadata(&p) {
            if let Ok(mtime) = meta.modified() {
                mtimes.insert(p, mtime);
            }
        }
    }

    mtimes
}

/// Recursively collect .tnt file mtimes from a directory.
fn collect_tnt_mtimes_recursive(
    dir: &PathBuf,
    mtimes: &mut HashMap<PathBuf, std::time::SystemTime>,
) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_tnt_mtimes_recursive(&path, mtimes);
            } else if path.extension().and_then(|e| e.to_str()) == Some("tnt") {
                if let Ok(meta) = std::fs::metadata(&path) {
                    if let Ok(mtime) = meta.modified() {
                        mtimes.insert(path, mtime);
                    }
                }
            }
        }
    }
}

/// Background hot-reload watcher task.
///
/// Polls for .tnt file changes at `poll_interval` and atomically swaps the
/// SharedState when changes are detected. Failed reloads log an error and
/// keep the old state running. In-flight requests complete with their cloned
/// StoredHandler — the write lock is only held during the swap.
pub async fn hot_reload_watcher(
    shared: Arc<std::sync::RwLock<SharedState>>,
    source_file: String,
    poll_interval: Duration,
) {
    let mut last_mtimes = {
        let s = shared.read().unwrap_or_else(|e| e.into_inner());
        collect_file_mtimes(&source_file, &s)
    };

    loop {
        tokio::time::sleep(poll_interval).await;

        let current_mtimes = {
            let s = shared.read().unwrap_or_else(|e| e.into_inner());
            collect_file_mtimes(&source_file, &s)
        };

        if current_mtimes != last_mtimes {
            match rebuild_shared_state(&source_file) {
                Ok(new_state) => {
                    let route_count = new_state.route_count();
                    // Write lock only during swap — in-flight requests have their cloned handlers
                    {
                        let mut guard = shared.write().unwrap_or_else(|e| e.into_inner());
                        *guard = new_state;
                    }
                    eprintln!("[hot-reload] Reloaded — {} routes", route_count);
                }
                Err(e) => {
                    eprintln!("[hot-reload] Failed: {} — keeping old state", e);
                }
            }
            last_mtimes = current_mtimes;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_route_pattern() {
        let segments = parse_route_pattern("/users/{id}");
        assert_eq!(segments.len(), 2);
        match &segments[0] {
            RouteSegment::Static(s) => assert_eq!(s, "users"),
            _ => panic!("Expected static segment"),
        }
        match &segments[1] {
            RouteSegment::Param(p) => assert_eq!(p, "id"),
            _ => panic!("Expected param segment"),
        }
    }

    #[test]
    fn test_match_route_basic() {
        let route = Route {
            method: "GET".to_string(),
            pattern: "/users/{id}".to_string(),
            segments: parse_route_pattern("/users/{id}"),
        };
        let result = match_route("/users/123", &route);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_match_route_root() {
        let route = Route {
            method: "GET".to_string(),
            pattern: "/".to_string(),
            segments: parse_route_pattern("/"),
        };
        let result = match_route("/", &route);
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_guess_mime_type() {
        assert_eq!(guess_mime_type("style.css"), "text/css; charset=utf-8");
        assert_eq!(
            guess_mime_type("app.js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(guess_mime_type("image.png"), "image/png");
        assert_eq!(
            guess_mime_type("data.json"),
            "application/json; charset=utf-8"
        );
        assert_eq!(guess_mime_type("unknown"), "application/octet-stream");
    }

    #[test]
    fn test_create_error_response() {
        let resp = create_error_response(404, "Not Found");
        if let Value::Map(map) = resp {
            match map.get("status") {
                Some(Value::Int(404)) => {}
                other => panic!("Expected status 404, got {:?}", other),
            }
            match map.get("body") {
                Some(Value::String(s)) if s == "Not Found" => {}
                other => panic!("Expected body 'Not Found', got {:?}", other),
            }
        } else {
            panic!("Expected Map");
        }
    }

    #[tokio::test]
    async fn test_async_server_state() {
        let state = AsyncServerState::new();
        assert_eq!(state.route_count().await, 0);

        state.register_route("GET", "/test", "test_handler").await;
        assert_eq!(state.route_count().await, 1);

        let found = state.find_route("GET", "/test").await;
        assert!(found.is_some());
        let (handler_name, params) = found.unwrap();
        assert_eq!(handler_name, "test_handler");
        assert!(params.is_empty());

        let not_found = state.find_route("POST", "/test").await;
        assert!(not_found.is_none());
    }

    #[tokio::test]
    async fn test_route_with_params() {
        let state = AsyncServerState::new();
        state.register_route("GET", "/users/{id}", "get_user").await;

        let found = state.find_route("GET", "/users/42").await;
        assert!(found.is_some());
        let (handler_name, params) = found.unwrap();
        assert_eq!(handler_name, "get_user");
        assert_eq!(params.get("id"), Some(&"42".to_string()));
    }

    #[tokio::test]
    async fn test_static_dir_registration() {
        let state = AsyncServerState::new();
        assert_eq!(state.static_dir_count().await, 0);

        state.register_static_dir("/assets", "./public").await;
        assert_eq!(state.static_dir_count().await, 1);
    }
}
