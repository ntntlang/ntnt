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
    pub async fn find_static_file(&self, path: &str) -> Option<(String, String)> {
        let dirs = self.static_dirs.read().await;
        for dir in dirs.iter() {
            if path.starts_with(&dir.url_prefix) {
                let relative = path.strip_prefix(&dir.url_prefix).unwrap_or("");
                let relative = relative.trim_start_matches('/');
                let file_path = PathBuf::from(&dir.fs_path).join(relative);
                if file_path.exists() && file_path.is_file() {
                    return Some((
                        file_path.to_string_lossy().to_string(),
                        dir.url_prefix.clone(),
                    ));
                }
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

    // Parse query params
    let mut query_params = HashMap::new();
    if !query.is_empty() {
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                query_params.insert(key.to_string(), value.to_string());
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

    response.body(Body::from(resp.body)).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::from("Failed to build response"))
            .unwrap()
    })
}

/// Serve a static file with proper headers
fn serve_static_file(file_path: &str) -> Response<Body> {
    use std::fs;

    match fs::read(file_path) {
        Ok(contents) => {
            let mime_type = guess_mime_type(file_path);
            let len = contents.len();

            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", mime_type)
                .header("content-length", len)
                .header("cache-control", "public, max-age=3600")
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

    // First, check for dynamic route match
    let route_match = state.routes.find_route(method.as_str(), &path).await;

    match route_match {
        Some((_handler_name, params)) => {
            // Convert request and send to interpreter
            match axum_to_bridge_request(req, params).await {
                Ok(bridge_req) => match state.interpreter.call(bridge_req).await {
                    Ok(response) => bridge_to_axum_response(response),
                    Err(e) => {
                        eprintln!("Handler error: {}", e);
                        bridge_to_axum_response(BridgeResponse::error(
                            500,
                            &format!("Internal Server Error: {}", e),
                        ))
                    }
                },
                Err(e) => {
                    eprintln!("Request parsing error: {}", e);
                    bridge_to_axum_response(BridgeResponse::error(400, "Bad Request"))
                }
            }
        }
        None => {
            // No dynamic route - check static files (GET only)
            if method == axum::http::Method::GET {
                if let Some((file_path, _prefix)) = state.routes.find_static_file(&path).await {
                    return serve_static_file(&file_path);
                }
            }

            // No route or static file - return 404
            bridge_to_axum_response(BridgeResponse::not_found())
        }
    }
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

    let state = AppState {
        interpreter: interpreter_handle,
        routes,
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

    println!("🚀 NTNT async server running on http://{}", addr);
    println!("   Using Axum + Tokio for high-concurrency");
    println!("   Routes registered: {}", route_count);
    if static_count > 0 {
        println!("   Static directories: {}", static_count);
    }
    println!("   Request timeout: {}s", config.request_timeout_secs);
    println!("   Max connections: {}", config.max_connections);
    println!();
    println!("Press Ctrl+C to stop");
    println!();

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
