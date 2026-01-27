//! HTTP Server module for NTNT
//!
//! Provides a simple HTTP server with routing support for building web applications.
//!
//! Example usage:
//! ```ntnt
//! use "std/http/server"
//!
//! fn home(req) {
//!     return text("Hello, World!")
//! }
//!
//! fn get_user(req) {
//!     let id = req.params.id
//!     return json({ "id": id, "name": "User " + id })
//! }
//!
//! get("/", home)
//! get("/users/{id}", get_user)
//! listen(8080)
//! ```

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use crate::stdlib::json::json_to_intent_value;
use std::collections::HashMap;
use std::time::SystemTime;

/// Represents a route segment - either static text or a parameter
#[derive(Debug, Clone)]
pub enum RouteSegment {
    Static(String),
    Param(String),
}

/// A compiled route with its pattern parsed into segments
#[derive(Debug, Clone)]
pub struct Route {
    pub method: String,
    pub pattern: String,
    pub segments: Vec<RouteSegment>,
}

/// Information about a route's source file for hot-reload
#[derive(Debug, Clone)]
pub struct RouteSource {
    pub file_path: Option<String>, // None for inline routes
    pub mtime: Option<SystemTime>, // Last modification time
    pub imported_files: HashMap<String, SystemTime>, // Tracked imports for this route
}

/// Server state stored in the interpreter
#[derive(Debug, Clone)]
pub struct ServerState {
    pub routes: Vec<(Route, Value, RouteSource)>, // Routes with handlers and source info
    pub static_dirs: Vec<(String, String)>,       // (url_prefix, filesystem_path)
    pub middleware: Vec<Value>,                   // Middleware functions to run before handlers
    pub hot_reload: bool,                         // Whether hot-reload is enabled
    pub shutdown_handlers: Vec<Value>,            // Functions to call on server shutdown
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            routes: Vec::new(),
            static_dirs: Vec::new(),
            middleware: Vec::new(),
            hot_reload: true, // Enable hot-reload by default in dev
            shutdown_handlers: Vec::new(),
        }
    }

    pub fn clear(&mut self) {
        self.routes.clear();
        self.static_dirs.clear();
        self.middleware.clear();
        self.shutdown_handlers.clear();
    }

    pub fn add_shutdown_handler(&mut self, handler: Value) {
        self.shutdown_handlers.push(handler);
    }

    pub fn get_shutdown_handlers(&self) -> &[Value] {
        &self.shutdown_handlers
    }

    /// Add a route without source file info (inline routes)
    pub fn add_route(&mut self, method: &str, pattern: &str, handler: Value) {
        self.add_route_with_source(method, pattern, handler, None, HashMap::new());
    }

    /// Add a route with source file info for hot-reload
    pub fn add_route_with_source(
        &mut self,
        method: &str,
        pattern: &str,
        handler: Value,
        file_path: Option<String>,
        imported_files: HashMap<String, SystemTime>,
    ) {
        let route = Route {
            method: method.to_string(),
            pattern: pattern.to_string(),
            segments: parse_route_pattern(pattern),
        };

        // Get file mtime if path provided
        let mtime = file_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok().and_then(|m| m.modified().ok()));

        let source = RouteSource {
            file_path,
            mtime,
            imported_files,
        };
        self.routes.push((route, handler, source));
    }

    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    /// Find a route and return its index for potential hot-reload
    pub fn find_route(
        &self,
        method: &str,
        path: &str,
    ) -> Option<(Value, HashMap<String, String>, usize)> {
        for (index, (route, handler, _source)) in self.routes.iter().enumerate() {
            if route.method == method {
                if let Some(params) = match_route(path, route) {
                    return Some((handler.clone(), params, index));
                }
            }
        }
        None
    }

    /// Check if a route needs reloading based on file mtime or imported files
    pub fn needs_reload(&self, route_index: usize) -> bool {
        if !self.hot_reload {
            return false;
        }

        if let Some((_, _, source)) = self.routes.get(route_index) {
            // Check main route file
            if let (Some(file_path), Some(cached_mtime)) = (&source.file_path, &source.mtime) {
                if let Ok(metadata) = std::fs::metadata(file_path) {
                    if let Ok(current_mtime) = metadata.modified() {
                        if current_mtime > *cached_mtime {
                            return true;
                        }
                    }
                }
            }

            // Check imported files
            for (import_path, import_mtime) in &source.imported_files {
                if let Ok(metadata) = std::fs::metadata(import_path) {
                    if let Ok(current_mtime) = metadata.modified() {
                        if current_mtime > *import_mtime {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// Update a route's handler after hot-reload
    pub fn update_route_handler(
        &mut self,
        route_index: usize,
        new_handler: Value,
        new_imported_files: HashMap<String, SystemTime>,
    ) {
        if let Some((_, handler, source)) = self.routes.get_mut(route_index) {
            *handler = new_handler;
            // Update mtime
            if let Some(file_path) = &source.file_path {
                source.mtime = std::fs::metadata(file_path)
                    .ok()
                    .and_then(|m| m.modified().ok());
            }
            // Update imported files
            source.imported_files = new_imported_files;
        }
    }

    /// Get the source info for a route
    pub fn get_route_source(&self, route_index: usize) -> Option<&RouteSource> {
        self.routes.get(route_index).map(|(_, _, source)| source)
    }

    pub fn add_static_dir(&mut self, prefix: String, directory: String) {
        self.static_dirs.push((prefix, directory));
    }

    pub fn add_middleware(&mut self, handler: Value) {
        self.middleware.push(handler);
    }

    pub fn find_static_file(&self, path: &str) -> Option<(String, String)> {
        for (prefix, directory) in &self.static_dirs {
            // Check if path starts with prefix
            let prefix_path = if prefix.ends_with('/') {
                prefix.clone()
            } else {
                format!("{}/", prefix)
            };
            if path.starts_with(&prefix_path) || path == prefix.trim_end_matches('/') {
                // Get the relative path after the prefix
                let relative = if path == prefix.trim_end_matches('/') {
                    "index.html".to_string()
                } else {
                    path.strip_prefix(&prefix_path).unwrap_or("").to_string()
                };

                // Handle empty relative path (root of static dir)
                let relative = if relative.is_empty() {
                    "index.html".to_string()
                } else {
                    relative
                };

                // Construct full filesystem path
                let full_path = std::path::Path::new(directory).join(&relative);

                // Security: ensure we're not escaping the directory (path traversal)
                if let Ok(canonical) = full_path.canonicalize() {
                    if let Ok(base_canonical) = std::path::Path::new(directory).canonicalize() {
                        if canonical.starts_with(&base_canonical) {
                            return Some((canonical.to_string_lossy().to_string(), relative));
                        }
                    }
                }

                // If canonicalize fails (file doesn't exist), try the raw path
                return Some((full_path.to_string_lossy().to_string(), relative));
            }
        }
        None
    }

    pub fn get_middleware(&self) -> &[Value] {
        &self.middleware
    }
}

impl Default for ServerState {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a route pattern into segments
/// e.g., "/users/{id}/posts/{post_id}" -> [Static("users"), Param("id"), Static("posts"), Param("post_id")]
fn parse_route_pattern(pattern: &str) -> Vec<RouteSegment> {
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

/// Match a URL path against a route, returning extracted parameters if matched
fn match_route(path: &str, route: &Route) -> Option<HashMap<String, String>> {
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if path_segments.len() != route.segments.len() {
        return None;
    }

    let mut params = HashMap::new();

    for (path_seg, route_seg) in path_segments.iter().zip(route.segments.iter()) {
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

/// Convert a tiny_http Request to an Intent Value
pub fn request_to_value(
    request: &tiny_http::Request,
    params: HashMap<String, String>,
    body: String,
) -> Value {
    let mut req_map: HashMap<String, Value> = HashMap::new();

    // Method
    req_map.insert(
        "method".to_string(),
        Value::String(request.method().to_string()),
    );

    // URL and path
    let url = request.url().to_string();
    let path = url.split('?').next().unwrap_or(&url).to_string();
    req_map.insert("url".to_string(), Value::String(url.clone()));
    req_map.insert("path".to_string(), Value::String(path));

    // Query string
    let query = if url.contains('?') {
        url.split('?').nth(1).unwrap_or("").to_string()
    } else {
        String::new()
    };
    req_map.insert("query".to_string(), Value::String(query.clone()));

    // Parse query params into a map
    let mut query_params: HashMap<String, Value> = HashMap::new();
    if !query.is_empty() {
        for pair in query.split('&') {
            if let Some((key, value)) = pair.split_once('=') {
                query_params.insert(key.to_string(), Value::String(value.to_string()));
            }
        }
    }
    req_map.insert("query_params".to_string(), Value::Map(query_params));

    // Route params (from path like /users/{id})
    let param_map: HashMap<String, Value> = params
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    req_map.insert("params".to_string(), Value::Map(param_map));

    // Headers
    let mut headers: HashMap<String, Value> = HashMap::new();
    let mut client_ip: Option<String> = None;
    let mut request_id: Option<String> = None;

    for header in request.headers() {
        let field_lower = header.field.to_string().to_lowercase();
        let value = header.value.to_string();

        // Extract proxy headers
        if field_lower == "x-forwarded-for" {
            // X-Forwarded-For can be comma-separated, take the first (original client)
            client_ip = Some(value.split(',').next().unwrap_or(&value).trim().to_string());
        } else if field_lower == "x-real-ip" && client_ip.is_none() {
            client_ip = Some(value.clone());
        } else if field_lower == "x-request-id" {
            request_id = Some(value.clone());
        }

        headers.insert(field_lower, Value::String(value));
    }
    req_map.insert("headers".to_string(), Value::Map(headers));

    // Body
    req_map.insert("body".to_string(), Value::String(body));

    // Client IP (from proxy headers or remote address)
    let ip = client_ip.unwrap_or_else(|| {
        request
            .remote_addr()
            .map(|addr| addr.ip().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    });
    req_map.insert("ip".to_string(), Value::String(ip));

    // Request ID (from header or generate one)
    let id = request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    req_map.insert("id".to_string(), Value::String(id));

    // Protocol (assume HTTP unless X-Forwarded-Proto says HTTPS)
    let protocol =
        headers_get_string(&req_map, "x-forwarded-proto").unwrap_or_else(|| "http".to_string());
    req_map.insert("protocol".to_string(), Value::String(protocol));

    Value::Map(req_map)
}

/// Helper to get a string from the headers map
fn headers_get_string(req_map: &HashMap<String, Value>, key: &str) -> Option<String> {
    if let Some(Value::Map(headers)) = req_map.get("headers") {
        if let Some(Value::String(value)) = headers.get(key) {
            return Some(value.clone());
        }
    }
    None
}

/// Convert Intent Value to JSON for response serialization
fn intent_value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Int(n) => serde_json::Value::Number((*n).into()),
        Value::Float(f) => serde_json::Value::Number(
            serde_json::Number::from_f64(*f).unwrap_or_else(|| serde_json::Number::from(0)),
        ),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(intent_value_to_json).collect())
        }
        Value::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), intent_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Value::Unit => serde_json::Value::Null,
        _ => serde_json::Value::String(format!("{:?}", value)),
    }
}

/// Create a response Value with given status, headers, and body
fn create_response_value(status: i64, headers: HashMap<String, Value>, body: String) -> Value {
    let mut response_map: HashMap<String, Value> = HashMap::new();
    response_map.insert("status".to_string(), Value::Int(status));
    response_map.insert("headers".to_string(), Value::Map(headers));
    response_map.insert("body".to_string(), Value::String(body));
    Value::Map(response_map)
}

/// Initialize the std/http/server module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // Response.text(body) -> Response - Create a text response
    module.insert(
        "text".to_string(),
        Value::NativeFunction {
            name: "text".to_string(),
            arity: 1,
            func: |args| {
                match &args[0] {
                    Value::String(body) => {
                        let mut headers = HashMap::new();
                        headers.insert(
                            "content-type".to_string(),
                            Value::String("text/plain; charset=utf-8".to_string()),
                        );
                        // Prevent caching for dynamic text content
                        headers.insert(
                            "cache-control".to_string(),
                            Value::String("no-cache, no-store, must-revalidate".to_string()),
                        );
                        Ok(create_response_value(200, headers, body.clone()))
                    }
                    _ => Err(IntentError::TypeError(
                        "text() requires a string".to_string(),
                    )),
                }
            },
        },
    );

    // Response.html(body, status_code?) -> Response - Create an HTML response
    // Includes cache-control headers to prevent browser caching
    module.insert(
        "html".to_string(),
        Value::NativeFunction {
            name: "html".to_string(),
            arity: 0, // Accepts 1 or 2 arguments (0 = variadic)
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "html() requires 1 or 2 arguments (body, optional status_code)".to_string(),
                    ));
                }

                let body = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "html() body must be a string".to_string(),
                        ))
                    }
                };

                let status_code = if args.len() == 2 {
                    match &args[1] {
                        Value::Int(code) => *code,
                        _ => {
                            return Err(IntentError::TypeError(
                                "html() status code must be an integer".to_string(),
                            ))
                        }
                    }
                } else {
                    200
                };

                let mut headers = HashMap::new();
                headers.insert(
                    "content-type".to_string(),
                    Value::String("text/html; charset=utf-8".to_string()),
                );
                // Prevent browser caching of dynamic HTML content
                headers.insert(
                    "cache-control".to_string(),
                    Value::String("no-cache, no-store, must-revalidate".to_string()),
                );
                headers.insert("pragma".to_string(), Value::String("no-cache".to_string()));
                Ok(create_response_value(status_code, headers, body))
            },
        },
    );

    // Response.json(data, status_code?) -> Response - Create a JSON response
    // If status_code is provided, use it; otherwise default to 200
    // Includes cache-control headers to prevent browser caching
    module.insert(
        "json".to_string(),
        Value::NativeFunction {
            name: "json".to_string(),
            arity: 0, // Accepts 1 or 2 arguments (0 = variadic)
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "json() requires 1 or 2 arguments (data, optional status_code)".to_string(),
                    ));
                }

                let status_code = if args.len() == 2 {
                    match &args[1] {
                        Value::Int(code) => *code,
                        _ => {
                            return Err(IntentError::TypeError(
                                "json() status code must be an integer".to_string(),
                            ))
                        }
                    }
                } else {
                    200
                };

                let json_value = intent_value_to_json(&args[0]);
                let body = json_value.to_string();
                let mut headers = HashMap::new();
                headers.insert(
                    "content-type".to_string(),
                    Value::String("application/json".to_string()),
                );
                // Prevent browser caching of API responses
                headers.insert(
                    "cache-control".to_string(),
                    Value::String("no-cache, no-store, must-revalidate".to_string()),
                );
                Ok(create_response_value(status_code, headers, body))
            },
        },
    );

    // Response.status(code, body) -> Response - Create response with status code
    module.insert(
        "status".to_string(),
        Value::NativeFunction {
            name: "status".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::Int(code), Value::String(body)) => {
                    let mut headers = HashMap::new();
                    headers.insert(
                        "content-type".to_string(),
                        Value::String("text/plain; charset=utf-8".to_string()),
                    );
                    Ok(create_response_value(*code, headers, body.clone()))
                }
                _ => Err(IntentError::TypeError(
                    "status() requires int and string".to_string(),
                )),
            },
        },
    );

    // Response.redirect(url) -> Response - Create a redirect response
    module.insert(
        "redirect".to_string(),
        Value::NativeFunction {
            name: "redirect".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(url) => {
                    let mut headers = HashMap::new();
                    headers.insert("location".to_string(), Value::String(url.clone()));
                    Ok(create_response_value(302, headers, String::new()))
                }
                _ => Err(IntentError::TypeError(
                    "redirect() requires a URL string".to_string(),
                )),
            },
        },
    );

    // Response.not_found() -> Response - Create a 404 response
    module.insert(
        "not_found".to_string(),
        Value::NativeFunction {
            name: "not_found".to_string(),
            arity: 0,
            func: |_args| {
                let mut headers = HashMap::new();
                headers.insert(
                    "content-type".to_string(),
                    Value::String("text/plain; charset=utf-8".to_string()),
                );
                Ok(create_response_value(404, headers, "Not Found".to_string()))
            },
        },
    );

    // Response.error(message) -> Response - Create a 500 error response
    module.insert(
        "error".to_string(),
        Value::NativeFunction {
            name: "error".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(msg) => {
                    let mut headers = HashMap::new();
                    headers.insert(
                        "content-type".to_string(),
                        Value::String("text/plain; charset=utf-8".to_string()),
                    );
                    Ok(create_response_value(500, headers, msg.clone()))
                }
                _ => Err(IntentError::TypeError(
                    "error() requires a string".to_string(),
                )),
            },
        },
    );

    // Response.static_file(content, content_type, max_age?) -> Response
    // Create a response with caching enabled for static assets
    // max_age is in seconds, defaults to 3600 (1 hour)
    module.insert("static_file".to_string(), Value::NativeFunction {
        name: "static_file".to_string(),
        arity: 0, // Accepts 2 or 3 arguments
        func: |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(IntentError::TypeError(
                    "static_file() requires 2-3 arguments (content, content_type, optional max_age)".to_string()
                ));
            }

            let content = match &args[0] {
                Value::String(s) => s.clone(),
                _ => return Err(IntentError::TypeError("static_file() content must be a string".to_string())),
            };

            let content_type = match &args[1] {
                Value::String(s) => s.clone(),
                _ => return Err(IntentError::TypeError("static_file() content_type must be a string".to_string())),
            };

            let max_age = if args.len() == 3 {
                match &args[2] {
                    Value::Int(n) => *n,
                    _ => return Err(IntentError::TypeError("static_file() max_age must be an integer".to_string())),
                }
            } else {
                3600 // Default 1 hour
            };

            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                Value::String(content_type),
            );
            headers.insert(
                "cache-control".to_string(),
                Value::String(format!("public, max-age={}", max_age)),
            );
            Ok(create_response_value(200, headers, content))
        },
    });

    // Response.response(status, headers, body) -> Response
    // Create a fully custom response with complete control over headers
    module.insert(
        "response".to_string(),
        Value::NativeFunction {
            name: "response".to_string(),
            arity: 3,
            func: |args| {
                let status = match &args[0] {
                    Value::Int(code) => *code,
                    _ => {
                        return Err(IntentError::TypeError(
                            "response() status must be an integer".to_string(),
                        ))
                    }
                };

                let custom_headers = match &args[1] {
                    Value::Map(map) => map.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "response() headers must be a map".to_string(),
                        ))
                    }
                };

                let body = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "response() body must be a string".to_string(),
                        ))
                    }
                };

                let mut headers = HashMap::new();
                for (key, value) in custom_headers {
                    headers.insert(key.to_lowercase(), value);
                }

                Ok(create_response_value(status, headers, body))
            },
        },
    );

    // parse_json(req) -> Result<Value, Error> - Parse request body as JSON
    module.insert(
        "parse_json".to_string(),
        Value::NativeFunction {
            name: "parse_json".to_string(),
            arity: 1,
            func: |args| {
                let body = match &args[0] {
                    Value::Map(map) => match map.get("body") {
                        Some(Value::String(b)) => b.clone(),
                        _ => {
                            return Err(IntentError::TypeError(
                                "parse_json() requires a request with body".to_string(),
                            ))
                        }
                    },
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "parse_json() requires a request map or body string".to_string(),
                        ))
                    }
                };

                match serde_json::from_str::<serde_json::Value>(&body) {
                    Ok(json_val) => {
                        let intent_val = json_to_intent_value(&json_val);
                        Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![intent_val],
                        })
                    }
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(e.to_string())],
                    }),
                }
            },
        },
    );

    // parse_form(req) -> Map - Parse request body as URL-encoded form data
    module.insert(
        "parse_form".to_string(),
        Value::NativeFunction {
            name: "parse_form".to_string(),
            arity: 1,
            func: |args| {
                let body = match &args[0] {
                    Value::Map(map) => match map.get("body") {
                        Some(Value::String(b)) => b.clone(),
                        _ => {
                            return Err(IntentError::TypeError(
                                "parse_form() requires a request with body".to_string(),
                            ))
                        }
                    },
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "parse_form() requires a request map or body string".to_string(),
                        ))
                    }
                };

                let mut form_data: HashMap<String, Value> = HashMap::new();
                for pair in body.split('&') {
                    if pair.is_empty() {
                        continue;
                    }
                    if let Some((key, value)) = pair.split_once('=') {
                        // URL decode the key and value
                        let decoded_key = urlencoding::decode(key)
                            .unwrap_or_else(|_| key.into())
                            .to_string();
                        let decoded_value = urlencoding::decode(value)
                            .unwrap_or_else(|_| value.into())
                            .to_string();
                        form_data.insert(decoded_key, Value::String(decoded_value));
                    } else {
                        // Key with no value
                        let decoded_key = urlencoding::decode(pair)
                            .unwrap_or_else(|_| pair.into())
                            .to_string();
                        form_data.insert(decoded_key, Value::String(String::new()));
                    }
                }
                Ok(Value::Map(form_data))
            },
        },
    );

    // Note: new_server, get, post, put, delete, patch, and listen are handled
    // specially in the interpreter because they need access to interpreter state

    module
}

/// Start the HTTP server - this is called from the interpreter
pub fn start_server(port: u16) -> Result<tiny_http::Server> {
    let addr = format!("0.0.0.0:{}", port);
    tiny_http::Server::http(&addr)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to start server: {}", e)))
}

/// Start the HTTP server with timeout support (for test mode)
/// Binds to 127.0.0.1 only for security in test mode
pub fn start_server_with_timeout(
    port: u16,
    _timeout: std::time::Duration,
) -> Result<tiny_http::Server> {
    let addr = format!("127.0.0.1:{}", port);
    tiny_http::Server::http(&addr)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to start test server: {}", e)))
}

/// Read request body and create request Value
pub fn process_request(
    mut request: tiny_http::Request,
    params: HashMap<String, String>,
) -> Result<(Value, tiny_http::Request)> {
    // Read the request body
    let mut body = String::new();
    if let Err(e) = request.as_reader().read_to_string(&mut body) {
        return Err(IntentError::RuntimeError(format!(
            "Failed to read request body: {}",
            e
        )));
    }

    // Create request value
    let req_value = request_to_value(&request, params, body);

    Ok((req_value, request))
}

/// Send a response back to the client
pub fn send_response(request: tiny_http::Request, response: &Value) -> Result<()> {
    let (status, headers, body) = match response {
        Value::Map(map) => {
            let status = match map.get("status") {
                Some(Value::Int(s)) => *s as u16,
                _ => 200,
            };

            let headers = match map.get("headers") {
                Some(Value::Map(h)) => h.clone(),
                _ => HashMap::new(),
            };

            let body = match map.get("body") {
                Some(Value::String(b)) => b.clone(),
                _ => String::new(),
            };

            (status, headers, body)
        }
        _ => return Err(IntentError::TypeError("Response must be a map".to_string())),
    };

    // Build tiny_http response
    let mut response_builder = tiny_http::Response::from_string(body).with_status_code(status);

    // Add headers
    for (key, value) in headers {
        if let Value::String(v) = value {
            if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), v.as_bytes()) {
                response_builder = response_builder.with_header(header);
            }
        }
    }

    // Force connection close to prevent stale responses on keep-alive connections
    if let Ok(header) = tiny_http::Header::from_bytes("Connection".as_bytes(), "close".as_bytes()) {
        response_builder = response_builder.with_header(header);
    }

    // Add server identifier
    if let Ok(header) = tiny_http::Header::from_bytes("Server".as_bytes(), "ntnt-http".as_bytes()) {
        response_builder = response_builder.with_header(header);
    }

    request
        .respond(response_builder)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to send response: {}", e)))
}

/// Create an error response
pub fn create_error_response(status: i64, message: &str) -> Value {
    let mut headers = HashMap::new();
    headers.insert(
        "content-type".to_string(),
        Value::String("text/plain; charset=utf-8".to_string()),
    );
    create_response_value(status, headers, message.to_string())
}

/// Get MIME type based on file extension
pub fn get_mime_type(path: &str) -> &'static str {
    let extension = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        // HTML/Web
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "xml" => "application/xml; charset=utf-8",

        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "ico" => "image/x-icon",
        "webp" => "image/webp",

        // Fonts
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "eot" => "application/vnd.ms-fontobject",

        // Documents
        "pdf" => "application/pdf",
        "txt" => "text/plain; charset=utf-8",
        "md" => "text/markdown; charset=utf-8",

        // Data
        "csv" => "text/csv; charset=utf-8",
        "yaml" | "yml" => "application/x-yaml; charset=utf-8",

        // Archives
        "zip" => "application/zip",
        "gz" | "gzip" => "application/gzip",
        "tar" => "application/x-tar",

        // Media
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",

        // Catch-all
        _ => "application/octet-stream",
    }
}

/// Serve a static file from the filesystem
pub fn serve_static_file(file_path: &str) -> Result<Value> {
    use std::fs;
    use std::io::Read;

    let path = std::path::Path::new(file_path);

    // Check if file exists
    if !path.exists() {
        return Ok(create_error_response(404, "File not found"));
    }

    // Check if it's a file (not directory)
    if !path.is_file() {
        // If it's a directory, try index.html
        let index_path = path.join("index.html");
        if index_path.is_file() {
            return serve_static_file(&index_path.to_string_lossy());
        }
        return Ok(create_error_response(404, "Not a file"));
    }

    // Get MIME type
    let mime_type = get_mime_type(file_path);

    // Read file content
    let content = if mime_type.starts_with("text/")
        || mime_type.contains("javascript")
        || mime_type.contains("json")
        || mime_type.contains("xml")
        || mime_type.contains("yaml")
    {
        // Text files - read as string
        fs::read_to_string(path)
            .map_err(|e| IntentError::RuntimeError(format!("Failed to read file: {}", e)))?
    } else {
        // Binary files - read as bytes and encode as base64 or raw
        // For now, we'll read as lossy UTF-8 (works for most text, not ideal for binary)
        // A proper solution would need binary response support
        let mut file = fs::File::open(path)
            .map_err(|e| IntentError::RuntimeError(format!("Failed to open file: {}", e)))?;
        let mut buffer = Vec::new();
        file.read_to_end(&mut buffer)
            .map_err(|e| IntentError::RuntimeError(format!("Failed to read file: {}", e)))?;

        // For binary files, we need to handle them differently
        // For now, return raw bytes (this works with tiny_http's response)
        String::from_utf8_lossy(&buffer).to_string()
    };

    let mut headers = HashMap::new();
    headers.insert(
        "content-type".to_string(),
        Value::String(mime_type.to_string()),
    );

    // Add cache control for static files
    headers.insert(
        "cache-control".to_string(),
        Value::String("public, max-age=3600".to_string()),
    );

    Ok(create_response_value(200, headers, content))
}

/// Send a binary response (for static files)
pub fn send_static_response(request: tiny_http::Request, file_path: &str) -> Result<()> {
    use std::fs::File;
    use std::io::Read;

    let path = std::path::Path::new(file_path);

    // Check if file exists and is a file
    if !path.exists() || !path.is_file() {
        let not_found = create_error_response(404, "File not found");
        return send_response(request, &not_found);
    }

    // Get MIME type
    let mime_type = get_mime_type(file_path);

    // Open and read the file
    let mut file = File::open(path)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to open file: {}", e)))?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to read file: {}", e)))?;

    // Build response with proper headers
    let content_type = tiny_http::Header::from_bytes(b"Content-Type", mime_type.as_bytes())
        .map_err(|_| IntentError::RuntimeError("Invalid header".to_string()))?;
    let cache_control = tiny_http::Header::from_bytes(b"Cache-Control", b"public, max-age=3600")
        .map_err(|_| IntentError::RuntimeError("Invalid header".to_string()))?;
    let connection_close = tiny_http::Header::from_bytes(b"Connection", b"close")
        .map_err(|_| IntentError::RuntimeError("Invalid header".to_string()))?;
    let server_header = tiny_http::Header::from_bytes(b"Server", b"ntnt-http")
        .map_err(|_| IntentError::RuntimeError("Invalid header".to_string()))?;

    let response = tiny_http::Response::from_data(buffer)
        .with_status_code(200)
        .with_header(content_type)
        .with_header(cache_control)
        .with_header(connection_close)
        .with_header(server_header);

    request
        .respond(response)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to send response: {}", e)))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper functions to check Value types without PartialEq
    fn assert_value_string(v: &Value, expected: &str) {
        match v {
            Value::String(s) => assert_eq!(s, expected),
            _ => panic!("Expected String(\"{}\"), got {:?}", expected, v),
        }
    }

    fn get_map_int(map: &HashMap<String, Value>, key: &str) -> i64 {
        match map.get(key) {
            Some(Value::Int(n)) => *n,
            other => panic!("Expected Int at key '{}', got {:?}", key, other),
        }
    }

    fn get_map_string(map: &HashMap<String, Value>, key: &str) -> String {
        match map.get(key) {
            Some(Value::String(s)) => s.clone(),
            other => panic!("Expected String at key '{}', got {:?}", key, other),
        }
    }

    fn get_map_map(map: &HashMap<String, Value>, key: &str) -> HashMap<String, Value> {
        match map.get(key) {
            Some(Value::Map(m)) => m.clone(),
            other => panic!("Expected Map at key '{}', got {:?}", key, other),
        }
    }

    // ===========================================
    // Route Pattern Parsing Tests
    // ===========================================

    #[test]
    fn test_parse_route_pattern_static() {
        let segments = parse_route_pattern("/users");
        assert_eq!(segments.len(), 1);
        match &segments[0] {
            RouteSegment::Static(s) => assert_eq!(s, "users"),
            _ => panic!("Expected static segment"),
        }
    }

    #[test]
    fn test_parse_route_pattern_static_nested() {
        let segments = parse_route_pattern("/api/v1/users");
        assert_eq!(segments.len(), 3);
        match &segments[0] {
            RouteSegment::Static(s) => assert_eq!(s, "api"),
            _ => panic!("Expected static segment"),
        }
        match &segments[1] {
            RouteSegment::Static(s) => assert_eq!(s, "v1"),
            _ => panic!("Expected static segment"),
        }
        match &segments[2] {
            RouteSegment::Static(s) => assert_eq!(s, "users"),
            _ => panic!("Expected static segment"),
        }
    }

    #[test]
    fn test_parse_route_pattern_single_param() {
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
    fn test_parse_route_pattern_multiple_params() {
        let segments = parse_route_pattern("/users/{user_id}/posts/{post_id}");
        assert_eq!(segments.len(), 4);
        match &segments[0] {
            RouteSegment::Static(s) => assert_eq!(s, "users"),
            _ => panic!("Expected static segment"),
        }
        match &segments[1] {
            RouteSegment::Param(p) => assert_eq!(p, "user_id"),
            _ => panic!("Expected param segment"),
        }
        match &segments[2] {
            RouteSegment::Static(s) => assert_eq!(s, "posts"),
            _ => panic!("Expected static segment"),
        }
        match &segments[3] {
            RouteSegment::Param(p) => assert_eq!(p, "post_id"),
            _ => panic!("Expected param segment"),
        }
    }

    #[test]
    fn test_parse_route_pattern_root() {
        let segments = parse_route_pattern("/");
        assert_eq!(segments.len(), 0);
    }

    // ===========================================
    // Route Matching Tests
    // ===========================================

    #[test]
    fn test_match_route_static_exact() {
        let route = Route {
            method: "GET".to_string(),
            pattern: "/users".to_string(),
            segments: parse_route_pattern("/users"),
        };
        let result = match_route("/users", &route);
        assert!(result.is_some());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_match_route_static_no_match() {
        let route = Route {
            method: "GET".to_string(),
            pattern: "/users".to_string(),
            segments: parse_route_pattern("/users"),
        };
        let result = match_route("/posts", &route);
        assert!(result.is_none());
    }

    #[test]
    fn test_match_route_with_param() {
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
    fn test_match_route_with_multiple_params() {
        let route = Route {
            method: "GET".to_string(),
            pattern: "/users/{user_id}/posts/{post_id}".to_string(),
            segments: parse_route_pattern("/users/{user_id}/posts/{post_id}"),
        };
        let result = match_route("/users/42/posts/99", &route);
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.get("user_id"), Some(&"42".to_string()));
        assert_eq!(params.get("post_id"), Some(&"99".to_string()));
    }

    #[test]
    fn test_match_route_wrong_segment_count() {
        let route = Route {
            method: "GET".to_string(),
            pattern: "/users/{id}".to_string(),
            segments: parse_route_pattern("/users/{id}"),
        };
        // Too few segments
        let result = match_route("/users", &route);
        assert!(result.is_none());

        // Too many segments
        let result = match_route("/users/123/extra", &route);
        assert!(result.is_none());
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

    // ===========================================
    // ServerState Tests
    // ===========================================

    #[test]
    fn test_server_state_new() {
        let state = ServerState::new();
        assert_eq!(state.route_count(), 0);
    }

    #[test]
    fn test_server_state_add_route() {
        let mut state = ServerState::new();
        state.add_route("GET", "/users", Value::Unit);
        assert_eq!(state.route_count(), 1);
    }

    #[test]
    fn test_server_state_find_route() {
        let mut state = ServerState::new();
        state.add_route("GET", "/users/{id}", Value::String("handler".to_string()));

        let result = state.find_route("GET", "/users/123");
        assert!(result.is_some());
        let (handler, params, _index) = result.unwrap();
        assert_value_string(&handler, "handler");
        assert_eq!(params.get("id"), Some(&"123".to_string()));
    }

    #[test]
    fn test_server_state_find_route_wrong_method() {
        let mut state = ServerState::new();
        state.add_route("GET", "/users", Value::Unit);

        let result = state.find_route("POST", "/users");
        assert!(result.is_none());
    }

    #[test]
    fn test_server_state_find_route_no_match() {
        let mut state = ServerState::new();
        state.add_route("GET", "/users", Value::Unit);

        let result = state.find_route("GET", "/posts");
        assert!(result.is_none());
    }

    #[test]
    fn test_server_state_clear() {
        let mut state = ServerState::new();
        state.add_route("GET", "/users", Value::Unit);
        state.add_route("POST", "/users", Value::Unit);
        assert_eq!(state.route_count(), 2);

        state.clear();
        assert_eq!(state.route_count(), 0);
    }

    #[test]
    fn test_server_state_multiple_routes() {
        let mut state = ServerState::new();
        state.add_route("GET", "/", Value::String("home".to_string()));
        state.add_route("GET", "/users", Value::String("list_users".to_string()));
        state.add_route("GET", "/users/{id}", Value::String("get_user".to_string()));
        state.add_route("POST", "/users", Value::String("create_user".to_string()));

        assert_eq!(state.route_count(), 4);

        // Test finding each route
        let (handler, _, _) = state.find_route("GET", "/").unwrap();
        assert_value_string(&handler, "home");

        let (handler, _, _) = state.find_route("GET", "/users").unwrap();
        assert_value_string(&handler, "list_users");

        let (handler, params, _) = state.find_route("GET", "/users/42").unwrap();
        assert_value_string(&handler, "get_user");
        assert_eq!(params.get("id"), Some(&"42".to_string()));

        let (handler, _, _) = state.find_route("POST", "/users").unwrap();
        assert_value_string(&handler, "create_user");
    }

    // ===========================================
    // JSON Conversion Tests
    // ===========================================

    #[test]
    fn test_intent_value_to_json_int() {
        let value = Value::Int(42);
        let json = intent_value_to_json(&value);
        assert_eq!(json, serde_json::json!(42));
    }

    #[test]
    fn test_intent_value_to_json_float() {
        let value = Value::Float(3.14);
        let json = intent_value_to_json(&value);
        assert_eq!(json, serde_json::json!(3.14));
    }

    #[test]
    fn test_intent_value_to_json_string() {
        let value = Value::String("hello".to_string());
        let json = intent_value_to_json(&value);
        assert_eq!(json, serde_json::json!("hello"));
    }

    #[test]
    fn test_intent_value_to_json_bool() {
        let value = Value::Bool(true);
        let json = intent_value_to_json(&value);
        assert_eq!(json, serde_json::json!(true));
    }

    #[test]
    fn test_intent_value_to_json_array() {
        let value = Value::Array(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let json = intent_value_to_json(&value);
        assert_eq!(json, serde_json::json!([1, 2, 3]));
    }

    #[test]
    fn test_intent_value_to_json_map() {
        let mut map = HashMap::new();
        map.insert("name".to_string(), Value::String("Alice".to_string()));
        map.insert("age".to_string(), Value::Int(30));
        let value = Value::Map(map);
        let json = intent_value_to_json(&value);

        assert_eq!(json["name"], serde_json::json!("Alice"));
        assert_eq!(json["age"], serde_json::json!(30));
    }

    #[test]
    fn test_intent_value_to_json_unit() {
        let value = Value::Unit;
        let json = intent_value_to_json(&value);
        assert_eq!(json, serde_json::Value::Null);
    }

    #[test]
    fn test_intent_value_to_json_nested() {
        let mut inner_map = HashMap::new();
        inner_map.insert("city".to_string(), Value::String("NYC".to_string()));

        let mut map = HashMap::new();
        map.insert("user".to_string(), Value::String("Bob".to_string()));
        map.insert("address".to_string(), Value::Map(inner_map));
        map.insert(
            "scores".to_string(),
            Value::Array(vec![Value::Int(100), Value::Int(95)]),
        );

        let value = Value::Map(map);
        let json = intent_value_to_json(&value);

        assert_eq!(json["user"], serde_json::json!("Bob"));
        assert_eq!(json["address"]["city"], serde_json::json!("NYC"));
        assert_eq!(json["scores"], serde_json::json!([100, 95]));
    }

    // ===========================================
    // Response Helper Tests
    // ===========================================

    #[test]
    fn test_create_response_value() {
        let mut headers = HashMap::new();
        headers.insert("x-custom".to_string(), Value::String("test".to_string()));

        let response = create_response_value(201, headers, "Created".to_string());

        match response {
            Value::Map(map) => {
                assert_eq!(get_map_int(&map, "status"), 201);
                assert_eq!(get_map_string(&map, "body"), "Created");

                let hdrs = get_map_map(&map, "headers");
                assert_eq!(get_map_string(&hdrs, "x-custom"), "test");
            }
            _ => panic!("Expected response to be a map"),
        }
    }

    #[test]
    fn test_create_error_response() {
        let response = create_error_response(500, "Internal Server Error");

        match response {
            Value::Map(map) => {
                assert_eq!(get_map_int(&map, "status"), 500);
                assert_eq!(get_map_string(&map, "body"), "Internal Server Error");
            }
            _ => panic!("Expected response to be a map"),
        }
    }

    // ===========================================
    // Module Init Tests
    // ===========================================

    #[test]
    fn test_init_module_has_all_functions() {
        let module = init();

        // Check all response helper functions exist
        assert!(module.contains_key("text"));
        assert!(module.contains_key("html"));
        assert!(module.contains_key("json"));
        assert!(module.contains_key("status"));
        assert!(module.contains_key("redirect"));
        assert!(module.contains_key("not_found"));
        assert!(module.contains_key("error"));
    }

    #[test]
    fn test_text_function() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("text") {
            let args = vec![Value::String("Hello".to_string())];
            let result = func(&args);
            assert!(result.is_ok());

            if let Value::Map(map) = result.unwrap() {
                assert_eq!(get_map_int(&map, "status"), 200);
                assert_eq!(get_map_string(&map, "body"), "Hello");

                let headers = get_map_map(&map, "headers");
                assert_eq!(
                    get_map_string(&headers, "content-type"),
                    "text/plain; charset=utf-8"
                );
            } else {
                panic!("Expected Map response");
            }
        } else {
            panic!("text function not found");
        }
    }

    #[test]
    fn test_html_function() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("html") {
            let args = vec![Value::String("<h1>Test</h1>".to_string())];
            let result = func(&args);
            assert!(result.is_ok());

            if let Value::Map(map) = result.unwrap() {
                assert_eq!(get_map_int(&map, "status"), 200);
                assert_eq!(get_map_string(&map, "body"), "<h1>Test</h1>");

                let headers = get_map_map(&map, "headers");
                assert_eq!(
                    get_map_string(&headers, "content-type"),
                    "text/html; charset=utf-8"
                );
            } else {
                panic!("Expected Map response");
            }
        } else {
            panic!("html function not found");
        }
    }

    #[test]
    fn test_json_function() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("json") {
            let mut map = HashMap::new();
            map.insert("key".to_string(), Value::String("value".to_string()));

            let args = vec![Value::Map(map)];
            let result = func(&args);
            assert!(result.is_ok());

            if let Value::Map(resp) = result.unwrap() {
                assert_eq!(get_map_int(&resp, "status"), 200);

                let headers = get_map_map(&resp, "headers");
                assert_eq!(get_map_string(&headers, "content-type"), "application/json");

                // Verify body is valid JSON
                let body = get_map_string(&resp, "body");
                let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_eq!(parsed["key"], "value");
            } else {
                panic!("Expected Map response");
            }
        } else {
            panic!("json function not found");
        }
    }

    #[test]
    fn test_status_function() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("status") {
            let args = vec![Value::Int(404), Value::String("Not Found".to_string())];
            let result = func(&args);
            assert!(result.is_ok());

            if let Value::Map(map) = result.unwrap() {
                assert_eq!(get_map_int(&map, "status"), 404);
                assert_eq!(get_map_string(&map, "body"), "Not Found");
            } else {
                panic!("Expected Map response");
            }
        } else {
            panic!("status function not found");
        }
    }

    #[test]
    fn test_redirect_function() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("redirect") {
            let args = vec![Value::String("/new-location".to_string())];
            let result = func(&args);
            assert!(result.is_ok());

            if let Value::Map(map) = result.unwrap() {
                assert_eq!(get_map_int(&map, "status"), 302);

                let headers = get_map_map(&map, "headers");
                assert_eq!(get_map_string(&headers, "location"), "/new-location");
            } else {
                panic!("Expected Map response");
            }
        } else {
            panic!("redirect function not found");
        }
    }

    #[test]
    fn test_not_found_function() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("not_found") {
            let args: Vec<Value> = vec![];
            let result = func(&args);
            assert!(result.is_ok());

            if let Value::Map(map) = result.unwrap() {
                assert_eq!(get_map_int(&map, "status"), 404);
                assert_eq!(get_map_string(&map, "body"), "Not Found");
            } else {
                panic!("Expected Map response");
            }
        } else {
            panic!("not_found function not found");
        }
    }

    #[test]
    fn test_error_function() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("error") {
            let args = vec![Value::String("Something went wrong".to_string())];
            let result = func(&args);
            assert!(result.is_ok());

            if let Value::Map(map) = result.unwrap() {
                assert_eq!(get_map_int(&map, "status"), 500);
                assert_eq!(get_map_string(&map, "body"), "Something went wrong");
            } else {
                panic!("Expected Map response");
            }
        } else {
            panic!("error function not found");
        }
    }

    // ===========================================
    // Error Handling Tests
    // ===========================================

    #[test]
    fn test_text_wrong_type() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("text") {
            let args = vec![Value::Int(42)];
            let result = func(&args);
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_html_wrong_type() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("html") {
            let args = vec![Value::Int(42)];
            let result = func(&args);
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_status_wrong_type() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("status") {
            // Wrong first arg type
            let args = vec![
                Value::String("404".to_string()),
                Value::String("Not Found".to_string()),
            ];
            let result = func(&args);
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_redirect_wrong_type() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("redirect") {
            let args = vec![Value::Int(302)];
            let result = func(&args);
            assert!(result.is_err());
        }
    }

    #[test]
    fn test_error_wrong_type() {
        let module = init();
        if let Some(Value::NativeFunction { func, .. }) = module.get("error") {
            let args = vec![Value::Int(500)];
            let result = func(&args);
            assert!(result.is_err());
        }
    }

    // ===========================================
    // MIME Type Detection Tests
    // ===========================================

    #[test]
    fn test_mime_type_html() {
        assert_eq!(get_mime_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(get_mime_type("page.htm"), "text/html; charset=utf-8");
    }

    #[test]
    fn test_mime_type_css() {
        assert_eq!(get_mime_type("styles.css"), "text/css; charset=utf-8");
    }

    #[test]
    fn test_mime_type_javascript() {
        assert_eq!(
            get_mime_type("app.js"),
            "application/javascript; charset=utf-8"
        );
        assert_eq!(
            get_mime_type("module.mjs"),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn test_mime_type_json() {
        assert_eq!(
            get_mime_type("data.json"),
            "application/json; charset=utf-8"
        );
    }

    #[test]
    fn test_mime_type_images() {
        assert_eq!(get_mime_type("photo.png"), "image/png");
        assert_eq!(get_mime_type("photo.jpg"), "image/jpeg");
        assert_eq!(get_mime_type("photo.jpeg"), "image/jpeg");
        assert_eq!(get_mime_type("logo.gif"), "image/gif");
        assert_eq!(get_mime_type("icon.svg"), "image/svg+xml");
        assert_eq!(get_mime_type("favicon.ico"), "image/x-icon");
        assert_eq!(get_mime_type("image.webp"), "image/webp");
    }

    #[test]
    fn test_mime_type_fonts() {
        assert_eq!(get_mime_type("font.woff"), "font/woff");
        assert_eq!(get_mime_type("font.woff2"), "font/woff2");
        assert_eq!(get_mime_type("font.ttf"), "font/ttf");
        assert_eq!(get_mime_type("font.otf"), "font/otf");
    }

    #[test]
    fn test_mime_type_unknown() {
        assert_eq!(get_mime_type("file.xyz"), "application/octet-stream");
        assert_eq!(get_mime_type("noextension"), "application/octet-stream");
    }

    #[test]
    fn test_mime_type_case_insensitive() {
        assert_eq!(get_mime_type("index.HTML"), "text/html; charset=utf-8");
        assert_eq!(get_mime_type("styles.CSS"), "text/css; charset=utf-8");
        assert_eq!(get_mime_type("image.PNG"), "image/png");
    }

    // ===========================================
    // ServerState Static Directory Tests
    // ===========================================

    #[test]
    fn test_server_state_add_static_dir() {
        let mut state = ServerState::new();
        state.add_static_dir("/static".to_string(), "./public".to_string());
        assert_eq!(state.static_dirs.len(), 1);
    }

    #[test]
    fn test_server_state_multiple_static_dirs() {
        let mut state = ServerState::new();
        state.add_static_dir("/static".to_string(), "./public".to_string());
        state.add_static_dir("/assets".to_string(), "./assets".to_string());
        assert_eq!(state.static_dirs.len(), 2);
    }

    #[test]
    fn test_server_state_clear_includes_static_dirs() {
        let mut state = ServerState::new();
        state.add_route("GET", "/", Value::Unit);
        state.add_static_dir("/static".to_string(), "./public".to_string());
        state.add_middleware(Value::Unit);

        state.clear();

        assert_eq!(state.route_count(), 0);
        assert_eq!(state.static_dirs.len(), 0);
        assert_eq!(state.middleware.len(), 0);
    }

    // ===========================================
    // ServerState Middleware Tests
    // ===========================================

    #[test]
    fn test_server_state_add_middleware() {
        let mut state = ServerState::new();
        state.add_middleware(Value::String("logger".to_string()));
        assert_eq!(state.middleware.len(), 1);
    }

    #[test]
    fn test_server_state_multiple_middleware() {
        let mut state = ServerState::new();
        state.add_middleware(Value::String("logger".to_string()));
        state.add_middleware(Value::String("auth".to_string()));
        state.add_middleware(Value::String("cors".to_string()));
        assert_eq!(state.middleware.len(), 3);
    }

    #[test]
    fn test_server_state_get_middleware() {
        let mut state = ServerState::new();
        state.add_middleware(Value::String("logger".to_string()));
        state.add_middleware(Value::String("auth".to_string()));

        let middleware = state.get_middleware();
        assert_eq!(middleware.len(), 2);
    }

    // ===========================================
    // Static File Path Matching Tests
    // ===========================================

    #[test]
    fn test_find_static_file_basic() {
        let mut state = ServerState::new();
        // Use temp directory for testing
        let temp_dir = std::env::temp_dir();
        let test_dir = temp_dir.join("intent_test_static");
        let _ = std::fs::create_dir_all(&test_dir);
        let test_file = test_dir.join("test.txt");
        let _ = std::fs::write(&test_file, "test content");

        state.add_static_dir(
            "/static".to_string(),
            test_dir.to_string_lossy().to_string(),
        );

        let result = state.find_static_file("/static/test.txt");
        assert!(result.is_some());

        // Cleanup
        let _ = std::fs::remove_file(&test_file);
        let _ = std::fs::remove_dir(&test_dir);
    }

    #[test]
    fn test_find_static_file_no_match() {
        let mut state = ServerState::new();
        state.add_static_dir("/static".to_string(), "./nonexistent".to_string());

        // Path doesn't match prefix
        let result = state.find_static_file("/other/file.txt");
        assert!(result.is_none());
    }

    // ===========================================
    // Error Response Tests (for contract validation)
    // ===========================================

    #[test]
    fn test_create_error_response_400_bad_request() {
        let resp = create_error_response(400, "Bad Request: Precondition failed");
        if let Value::Map(map) = resp {
            assert_eq!(get_map_int(&map, "status"), 400);
            assert_eq!(
                get_map_string(&map, "body"),
                "Bad Request: Precondition failed"
            );
            // Content-type is in the headers sub-map
            let headers = get_map_map(&map, "headers");
            assert_eq!(
                get_map_string(&headers, "content-type"),
                "text/plain; charset=utf-8"
            );
        } else {
            panic!("Expected Map response");
        }
    }

    #[test]
    fn test_create_error_response_500_server_error() {
        let resp = create_error_response(500, "Internal Error: Postcondition failed");
        if let Value::Map(map) = resp {
            assert_eq!(get_map_int(&map, "status"), 500);
            assert_eq!(
                get_map_string(&map, "body"),
                "Internal Error: Postcondition failed"
            );
        } else {
            panic!("Expected Map response");
        }
    }

    #[test]
    fn test_create_error_response_404_not_found() {
        let resp = create_error_response(404, "Not Found: /api/missing");
        if let Value::Map(map) = resp {
            assert_eq!(get_map_int(&map, "status"), 404);
            assert_eq!(get_map_string(&map, "body"), "Not Found: /api/missing");
        } else {
            panic!("Expected Map response");
        }
    }

    #[test]
    fn test_create_error_response_custom_status() {
        let resp = create_error_response(503, "Service Unavailable");
        if let Value::Map(map) = resp {
            assert_eq!(get_map_int(&map, "status"), 503);
            assert_eq!(get_map_string(&map, "body"), "Service Unavailable");
        } else {
            panic!("Expected Map response");
        }
    }

    // ===========================================
    // Contract Error Message Format Tests
    // ===========================================

    #[test]
    fn test_error_response_contains_contract_message() {
        // Simulate a contract violation error message
        let msg = "Precondition failed in 'create_user': req.body != \"\"";
        let resp = create_error_response(400, &format!("Bad Request: {}", msg));
        if let Value::Map(map) = resp {
            let body = get_map_string(&map, "body");
            assert!(body.contains("Precondition failed"));
            assert!(body.contains("create_user"));
            assert!(body.contains("req.body"));
        } else {
            panic!("Expected Map response");
        }
    }

    #[test]
    fn test_error_response_postcondition_message() {
        let msg = "Postcondition failed in 'divide': result * b == a";
        let resp = create_error_response(500, &format!("Internal Error: {}", msg));
        if let Value::Map(map) = resp {
            let body = get_map_string(&map, "body");
            assert!(body.contains("Postcondition failed"));
            assert!(body.contains("divide"));
        } else {
            panic!("Expected Map response");
        }
    }
}
