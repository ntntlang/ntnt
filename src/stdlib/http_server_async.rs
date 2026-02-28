//! Async HTTP Server module for NTNT
//!
//! High-concurrency HTTP server using Axum + Tokio for production workloads.
//!
//! ## Architecture
//!
//! The NTNT interpreter uses `Rc<RefCell<>>` for closures, which is not thread-safe.
//! This module bridges async Axum to the sync interpreter via message passing:
//!
//! 1. Async handlers receive HTTP requests
//! 2. Requests are converted to BridgeRequest and sent via channel
//! 3. Interpreter thread processes the request and sends response back
//! 4. Async handler receives BridgeResponse and converts to HTTP response
//!
//! ## Features
//!
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
use crate::stdlib::http_bridge::{BridgeRequest, BridgeResponse, SharedHandle};
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

use super::http_server::get_default_security_headers;

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

/// State shared between all request handlers
#[derive(Clone)]
pub struct AppState {
    /// Handle to send requests to the interpreter
    pub interpreter: SharedHandle,
    /// Route registry for matching requests
    pub routes: Arc<AsyncServerState>,
    /// Whether running in production mode (controls error page detail)
    pub is_production: bool,
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
        format!("\"{}_{}_{}\"", metadata.len(), duration.as_secs(), duration.subsec_nanos())
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

/// Main request handler - catches all requests and forwards to interpreter
async fn handle_request(State(state): State<AppState>, req: Request<Body>) -> impl IntoResponse {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    // Extract If-None-Match header for ETag/conditional request support (static files)
    let if_none_match = req
        .headers()
        .get("if-none-match")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // First, check for dynamic route match
    let route_match = state.routes.find_route(method.as_str(), &path).await;

    match route_match {
        Some((handler_name, params)) => {
            // Convert request and send to interpreter
            match axum_to_bridge_request(req, params).await {
                Ok(bridge_req) => match state.interpreter.call(bridge_req).await {
                    Ok(response) => bridge_to_axum_response(response),
                    Err(e) => {
                        let error_msg = format!("{}", e);
                        // Structured error log — always printed regardless of mode
                        eprintln!(
                            "[ERROR] {} {} | handler: {} | {}",
                            method, path, handler_name, error_msg
                        );
                        error_response(
                            500,
                            &error_msg,
                            &method.to_string(),
                            &path,
                            &handler_name,
                            state.is_production,
                        )
                    }
                },
                Err(e) => {
                    let error_msg = format!("{}", e);
                    eprintln!(
                        "[ERROR] {} {} | request parse | {}",
                        method, path, error_msg
                    );
                    error_response(
                        400,
                        &error_msg,
                        &method.to_string(),
                        &path,
                        "",
                        state.is_production,
                    )
                }
            }
        }
        None => {
            // No dynamic route - check static files (GET only)
            if method == axum::http::Method::GET {
                if let Some((file_path, _prefix)) = state.routes.find_static_file(&path).await {
                    return serve_static_file(&file_path, if_none_match.as_deref());
                }
            }

            // No route or static file - return 404
            error_response(
                404,
                "Not Found",
                &method.to_string(),
                &path,
                "",
                state.is_production,
            )
        }
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

/// Start the async HTTP server with interpreter bridge
///
/// This is the main entry point for production async servers.
pub async fn start_server_with_bridge(
    config: AsyncServerConfig,
    interpreter_handle: SharedHandle,
    routes: Arc<AsyncServerState>,
) -> Result<()> {
    let addr: SocketAddr = format!("{}:{}", config.host, config.port)
        .parse()
        .map_err(|e| IntentError::RuntimeError(format!("Invalid address: {}", e)))?;

    let route_count = routes.route_count().await;
    let static_count = routes.static_dir_count().await;

    let is_production = std::env::var("NTNT_ENV")
        .map(|v| v == "production" || v == "prod")
        .unwrap_or(false);

    let state = AppState {
        interpreter: interpreter_handle,
        routes,
        is_production,
    };

    // Build the router with catch-all handler
    let mut app = Router::new().fallback(handle_request).with_state(state);

    // Add middleware layers (order matters - applied bottom to top)
    // 1. Request timeout
    app = app.layer(TimeoutLayer::new(Duration::from_secs(
        config.request_timeout_secs,
    )));

    // 2. Compression
    if config.enable_compression {
        app = app.layer(CompressionLayer::new());
    }

    // 3. Tracing
    app = app.layer(TraceLayer::new_for_http());

    // Show user-friendly URL (0.0.0.0 means all interfaces, so use localhost for display)
    let display_url = if addr.ip().is_unspecified() {
        format!("http://localhost:{}", addr.port())
    } else {
        format!("http://{}", addr)
    };

    println!();
    println!("🚀 Server running — visit {}", display_url);
    println!(
        "   Routes: {}  |  Static: {}  |  Hot-reload: enabled",
        route_count, static_count
    );
    println!();
    println!("Press Ctrl+C to stop");

    // Create the listener
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| IntentError::RuntimeError(format!("Failed to bind: {}", e)))?;

    // Run the server with graceful shutdown
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| IntentError::RuntimeError(format!("Server error: {}", e)))
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
