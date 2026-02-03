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
use std::sync::OnceLock;
use std::time::SystemTime;

// =============================================================================
// Security Configuration
// =============================================================================

/// Security configuration loaded from environment variables
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// Maximum request body size in bytes (default: 10MB)
    pub max_body_size: usize,
    /// Whether to add security headers to all responses (default: true)
    pub security_headers: bool,
    /// Whether running in production mode (default: false)
    pub production_mode: bool,
    /// Whether to show detailed error messages (default: true in dev, false in prod)
    pub detailed_errors: bool,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        let production_mode = std::env::var("NTNT_ENV")
            .map(|v| v == "production" || v == "prod")
            .unwrap_or(false);

        SecurityConfig {
            max_body_size: std::env::var("NTNT_MAX_BODY_SIZE")
                .ok()
                .and_then(|s| parse_size(&s))
                .unwrap_or(10 * 1024 * 1024), // 10MB default
            security_headers: std::env::var("NTNT_SECURITY_HEADERS")
                .map(|v| v != "0" && v.to_lowercase() != "false")
                .unwrap_or(true), // Enabled by default
            production_mode,
            detailed_errors: std::env::var("NTNT_DETAILED_ERRORS")
                .map(|v| v == "1" || v.to_lowercase() == "true")
                .unwrap_or(!production_mode), // Detailed in dev, generic in prod
        }
    }
}

/// Parse a size string like "10MB", "1GB", "500KB" into bytes
fn parse_size(s: &str) -> Option<usize> {
    let s = s.trim().to_uppercase();
    if let Some(num) = s.strip_suffix("GB") {
        num.trim()
            .parse::<usize>()
            .ok()
            .map(|n| n * 1024 * 1024 * 1024)
    } else if let Some(num) = s.strip_suffix("MB") {
        num.trim().parse::<usize>().ok().map(|n| n * 1024 * 1024)
    } else if let Some(num) = s.strip_suffix("KB") {
        num.trim().parse::<usize>().ok().map(|n| n * 1024)
    } else if let Some(num) = s.strip_suffix('B') {
        num.trim().parse::<usize>().ok()
    } else {
        s.parse::<usize>().ok()
    }
}

/// Global security configuration (loaded once from environment)
static SECURITY_CONFIG: OnceLock<SecurityConfig> = OnceLock::new();

/// Get the global security configuration
pub fn get_security_config() -> &'static SecurityConfig {
    SECURITY_CONFIG.get_or_init(SecurityConfig::default)
}

/// Default security headers added to all responses
pub fn get_default_security_headers() -> HashMap<String, Value> {
    let mut headers = HashMap::new();

    // Prevent MIME type sniffing
    headers.insert(
        "x-content-type-options".to_string(),
        Value::String("nosniff".to_string()),
    );

    // Prevent clickjacking (can be overridden by app if needed for iframes)
    headers.insert(
        "x-frame-options".to_string(),
        Value::String("DENY".to_string()),
    );

    // Control referrer information
    headers.insert(
        "referrer-policy".to_string(),
        Value::String("strict-origin-when-cross-origin".to_string()),
    );

    // Prevent XSS in older browsers
    headers.insert(
        "x-xss-protection".to_string(),
        Value::String("1; mode=block".to_string()),
    );

    // Don't expose server software in production (overridden below)
    // Note: We don't add Server header here - let tiny_http's default or none

    headers
}

/// Apply security headers to a response map
pub fn apply_security_headers(response: &mut HashMap<String, Value>) {
    let config = get_security_config();
    if !config.security_headers {
        return;
    }

    let security_headers = get_default_security_headers();

    // Get existing headers or create new map
    let headers = match response.get_mut("headers") {
        Some(Value::Map(h)) => h,
        _ => {
            response.insert("headers".to_string(), Value::Map(HashMap::new()));
            match response.get_mut("headers") {
                Some(Value::Map(h)) => h,
                _ => return,
            }
        }
    };

    // Add security headers only if not already set (allow app to override)
    for (key, value) in security_headers {
        if !headers.contains_key(&key) {
            headers.insert(key, value);
        }
    }
}

/// Represents a route segment - either static text or a parameter
#[derive(Debug, Clone)]
pub enum RouteSegment {
    Static(String),
    Param {
        name: String,
        /// Type constraint: "Int", "Float", or None (defaults to String)
        param_type: Option<String>,
    },
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

/// CORS (Cross-Origin Resource Sharing) configuration
#[derive(Debug, Clone)]
pub struct CorsConfig {
    pub origins: Vec<String>, // Allowed origins (["*"] for wildcard)
    pub methods: Vec<String>, // Allowed HTTP methods
    pub headers: Vec<String>, // Allowed request headers
    pub credentials: bool,    // Whether to allow credentials
    pub max_age: i64,         // Preflight cache duration in seconds
}

impl Default for CorsConfig {
    fn default() -> Self {
        CorsConfig {
            origins: vec!["*".to_string()],
            methods: vec![
                "GET".to_string(),
                "POST".to_string(),
                "PUT".to_string(),
                "DELETE".to_string(),
                "PATCH".to_string(),
                "OPTIONS".to_string(),
            ],
            headers: vec![
                "Content-Type".to_string(),
                "Authorization".to_string(),
                "Accept".to_string(),
            ],
            credentials: false,
            max_age: 86400,
        }
    }
}

impl CorsConfig {
    /// Create CORS config from an options map
    pub fn from_value(options: &HashMap<String, Value>) -> Self {
        let mut config = CorsConfig::default();

        // Parse origins
        if let Some(origins) = options.get("origins") {
            match origins {
                Value::String(s) => {
                    config.origins = vec![s.clone()];
                }
                Value::Array(arr) => {
                    config.origins = arr
                        .iter()
                        .filter_map(|v| {
                            if let Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .collect();
                }
                _ => {}
            }
        }

        // Parse methods
        if let Some(Value::Array(methods)) = options.get("methods") {
            config.methods = methods
                .iter()
                .filter_map(|v| {
                    if let Value::String(s) = v {
                        Some(s.to_uppercase())
                    } else {
                        None
                    }
                })
                .collect();
        }

        // Parse headers
        if let Some(Value::Array(headers)) = options.get("headers") {
            config.headers = headers
                .iter()
                .filter_map(|v| {
                    if let Value::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .collect();
        }

        // Parse credentials
        if let Some(Value::Bool(creds)) = options.get("credentials") {
            config.credentials = *creds;
        }

        // Parse max_age
        if let Some(Value::Int(age)) = options.get("max_age") {
            config.max_age = *age;
        }

        config
    }

    /// Check if the given origin is allowed
    pub fn is_origin_allowed(&self, origin: &str) -> bool {
        if self.origins.iter().any(|o| o == "*") {
            return true;
        }
        self.origins.iter().any(|o| o == origin)
    }

    /// Get the Access-Control-Allow-Origin header value for the given request origin
    pub fn get_allow_origin(&self, request_origin: Option<&str>) -> Option<String> {
        match request_origin {
            Some(origin) if self.is_origin_allowed(origin) => {
                // If credentials are enabled, we must return the specific origin, not *
                if self.credentials {
                    Some(origin.to_string())
                } else if self.origins.iter().any(|o| o == "*") {
                    Some("*".to_string())
                } else {
                    Some(origin.to_string())
                }
            }
            None if !self.credentials && self.origins.iter().any(|o| o == "*") => {
                Some("*".to_string())
            }
            _ => None,
        }
    }

    /// Apply CORS headers to a response Value
    pub fn apply_to_response(
        &self,
        response: &mut HashMap<String, Value>,
        request_origin: Option<&str>,
    ) {
        let mut headers = match response.get("headers") {
            Some(Value::Map(h)) => h.clone(),
            _ => HashMap::new(),
        };

        // Access-Control-Allow-Origin
        if let Some(allow_origin) = self.get_allow_origin(request_origin) {
            headers.insert(
                "access-control-allow-origin".to_string(),
                Value::String(allow_origin),
            );
        }

        // Access-Control-Allow-Methods
        headers.insert(
            "access-control-allow-methods".to_string(),
            Value::String(self.methods.join(", ")),
        );

        // Access-Control-Allow-Headers
        headers.insert(
            "access-control-allow-headers".to_string(),
            Value::String(self.headers.join(", ")),
        );

        // Access-Control-Allow-Credentials
        if self.credentials {
            headers.insert(
                "access-control-allow-credentials".to_string(),
                Value::String("true".to_string()),
            );
        }

        // Access-Control-Max-Age
        headers.insert(
            "access-control-max-age".to_string(),
            Value::String(self.max_age.to_string()),
        );

        response.insert("headers".to_string(), Value::Map(headers));
    }

    /// Create a preflight (OPTIONS) response
    pub fn create_preflight_response(&self, request_origin: Option<&str>) -> Value {
        let mut headers = HashMap::new();

        // Access-Control-Allow-Origin
        if let Some(allow_origin) = self.get_allow_origin(request_origin) {
            headers.insert(
                "access-control-allow-origin".to_string(),
                Value::String(allow_origin),
            );
        }

        // Access-Control-Allow-Methods
        headers.insert(
            "access-control-allow-methods".to_string(),
            Value::String(self.methods.join(", ")),
        );

        // Access-Control-Allow-Headers
        headers.insert(
            "access-control-allow-headers".to_string(),
            Value::String(self.headers.join(", ")),
        );

        // Access-Control-Allow-Credentials
        if self.credentials {
            headers.insert(
                "access-control-allow-credentials".to_string(),
                Value::String("true".to_string()),
            );
        }

        // Access-Control-Max-Age
        headers.insert(
            "access-control-max-age".to_string(),
            Value::String(self.max_age.to_string()),
        );

        create_response_value(204, headers, String::new())
    }
}

/// Result of route lookup with type information
#[derive(Debug, Clone)]
pub enum RouteMatchResult {
    /// Route matched successfully
    Matched {
        handler: Value,
        params: HashMap<String, String>,
        route_index: usize,
    },
    /// Route pattern matched but typed param validation failed
    TypeMismatch {
        param_name: String,
        expected: String,
        got: String,
    },
    /// No route matched
    NotFound,
}

/// Server state stored in the interpreter
#[derive(Debug, Clone)]
pub struct ServerState {
    pub routes: Vec<(Route, Value, RouteSource)>, // Routes with handlers and source info
    /// Route index for O(1) lookup by (method, segment_count) -> route indices
    route_index: HashMap<(String, usize), Vec<usize>>,
    pub static_dirs: Vec<(String, String)>, // (url_prefix, filesystem_path)
    pub middleware: Vec<Value>,             // Middleware functions to run before handlers
    pub hot_reload: bool,                   // Whether hot-reload is enabled
    pub shutdown_handlers: Vec<Value>,      // Functions to call on server shutdown
    pub cors_config: Option<CorsConfig>,    // Optional CORS configuration
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            routes: Vec::new(),
            route_index: HashMap::new(),
            static_dirs: Vec::new(),
            middleware: Vec::new(),
            hot_reload: true, // Enable hot-reload by default in dev
            shutdown_handlers: Vec::new(),
            cors_config: None,
        }
    }

    pub fn clear(&mut self) {
        self.routes.clear();
        self.route_index.clear();
        self.static_dirs.clear();
        self.middleware.clear();
        self.shutdown_handlers.clear();
        // Note: cors_config is NOT cleared - it's typically configured once at startup
    }

    /// Enable CORS with the given configuration
    pub fn enable_cors(&mut self, config: CorsConfig) {
        self.cors_config = Some(config);
    }

    /// Get the CORS configuration if enabled
    pub fn get_cors_config(&self) -> Option<&CorsConfig> {
        self.cors_config.as_ref()
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

    /// Detect if a new route would conflict with existing routes
    ///
    /// Two routes conflict if:
    /// - Same method
    /// - Same number of segments
    /// - At each position: either both are params (ambiguous) or static segments match
    ///
    /// Returns Some(conflicting_pattern) if a conflict is detected, None otherwise.
    pub fn detect_route_conflict(
        &self,
        method: &str,
        new_segments: &[RouteSegment],
    ) -> Option<String> {
        for (route, _, _) in &self.routes {
            if route.method != method {
                continue;
            }

            if route.segments.len() != new_segments.len() {
                continue;
            }

            let mut all_match = true;
            let mut has_ambiguous_params = false;

            for (existing, new) in route.segments.iter().zip(new_segments.iter()) {
                match (existing, new) {
                    // Two statics - must be equal
                    (RouteSegment::Static(a), RouteSegment::Static(b)) => {
                        if a != b {
                            all_match = false;
                            break;
                        }
                    }
                    // Both params - ambiguous!
                    (RouteSegment::Param { .. }, RouteSegment::Param { .. }) => {
                        has_ambiguous_params = true;
                    }
                    // One static, one param - OK, static takes priority
                    _ => {
                        all_match = false;
                        break;
                    }
                }
            }

            if all_match && has_ambiguous_params {
                return Some(route.pattern.clone());
            }
        }

        None
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
        let segments = parse_route_pattern(pattern);
        let segment_count = segments.len();
        let route = Route {
            method: method.to_string(),
            pattern: pattern.to_string(),
            segments,
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

        // Add to flat routes list (index is the position)
        let route_idx = self.routes.len();
        self.routes.push((route, handler, source));

        // Add to route index for fast lookup by (method, segment_count)
        let key = (method.to_string(), segment_count);
        self.route_index.entry(key).or_default().push(route_idx);
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
        match self.find_route_typed(method, path) {
            RouteMatchResult::Matched {
                handler,
                params,
                route_index,
            } => Some((handler, params, route_index)),
            _ => None,
        }
    }

    /// Find a route with detailed type validation result
    ///
    /// Uses indexed lookup by (method, segment_count) for O(1) partitioning,
    /// then linear search within the partition (typically 1-5 routes).
    pub fn find_route_typed(&self, method: &str, path: &str) -> RouteMatchResult {
        // Count path segments to narrow down candidates
        let segment_count = path.split('/').filter(|s| !s.is_empty()).count();
        let key = (method.to_string(), segment_count);

        // Look up candidate routes by (method, segment_count)
        let candidate_indices = match self.route_index.get(&key) {
            Some(indices) => indices,
            None => return RouteMatchResult::NotFound,
        };

        // Search only the candidates (typically 1-5 routes vs all routes)
        for &index in candidate_indices {
            let (route, handler, _source) = &self.routes[index];
            match match_route_typed(path, route) {
                MatchResult::Matched(params) => {
                    return RouteMatchResult::Matched {
                        handler: handler.clone(),
                        params,
                        route_index: index,
                    };
                }
                MatchResult::TypeMismatch {
                    param_name,
                    expected,
                    got,
                } => {
                    return RouteMatchResult::TypeMismatch {
                        param_name,
                        expected,
                        got,
                    };
                }
                MatchResult::NoMatch => {
                    // Continue looking for a match
                }
            }
        }
        RouteMatchResult::NotFound
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

                // Security: reject any path traversal attempts before even constructing the path
                // Check for ".." in the relative path (could be encoded or normalized)
                if relative.contains("..") || relative.contains('\0') {
                    return None; // Path traversal attempt - reject
                }

                // Also check for encoded traversal patterns
                let decoded =
                    urlencoding::decode(&relative).unwrap_or_else(|_| relative.clone().into());
                if decoded.contains("..") {
                    return None; // Encoded path traversal attempt - reject
                }

                // Construct full filesystem path
                let full_path = std::path::Path::new(directory).join(&relative);

                // Security: ensure we're not escaping the directory (path traversal)
                // Use canonicalize when file exists for the strongest guarantee
                if let Ok(canonical) = full_path.canonicalize() {
                    if let Ok(base_canonical) = std::path::Path::new(directory).canonicalize() {
                        if canonical.starts_with(&base_canonical) {
                            return Some((canonical.to_string_lossy().to_string(), relative));
                        }
                    }
                    // File exists but is outside the base directory - reject
                    return None;
                }

                // File doesn't exist - still return the path for 404 handling
                // but only if we've already validated no traversal patterns above
                // The path is safe because we rejected ".." and null bytes
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
                let inner = &segment[1..segment.len() - 1];
                // Check for type annotation: {name: Type}
                if let Some(colon_pos) = inner.find(':') {
                    let name = inner[..colon_pos].trim().to_string();
                    let param_type = Some(inner[colon_pos + 1..].trim().to_string());
                    RouteSegment::Param { name, param_type }
                } else {
                    RouteSegment::Param {
                        name: inner.to_string(),
                        param_type: None,
                    }
                }
            } else {
                RouteSegment::Static(segment.to_string())
            }
        })
        .collect()
}

/// Parse a route pattern with typed parameter info from AST
pub fn parse_pattern_with_types(
    pattern: &str,
    typed_params: &[crate::ast::TypedRouteParam],
) -> Vec<RouteSegment> {
    use std::collections::HashMap;

    // Build a map of param name -> type for lookup
    let type_map: HashMap<&str, Option<&str>> = typed_params
        .iter()
        .map(|p| (p.name.as_str(), p.param_type.as_deref()))
        .collect();

    pattern
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                let inner = &segment[1..segment.len() - 1];
                // Extract just the name (strip any inline type annotation)
                let name = if let Some(colon_pos) = inner.find(':') {
                    inner[..colon_pos].trim()
                } else {
                    inner.trim()
                };

                // Look up type from the typed_params or use inline annotation
                let param_type = if let Some(Some(t)) = type_map.get(name) {
                    Some((*t).to_string())
                } else if let Some(colon_pos) = inner.find(':') {
                    Some(inner[colon_pos + 1..].trim().to_string())
                } else {
                    None
                };

                RouteSegment::Param {
                    name: name.to_string(),
                    param_type,
                }
            } else {
                RouteSegment::Static(segment.to_string())
            }
        })
        .collect()
}

/// Match result indicating success or typed parameter failure
pub enum MatchResult {
    /// Route matched, returns extracted parameters
    Matched(HashMap<String, String>),
    /// Route pattern matched but typed param validation failed (return 400)
    TypeMismatch {
        param_name: String,
        expected: String,
        got: String,
    },
    /// Route pattern did not match
    NoMatch,
}

/// Match a URL path with type validation, returning detailed result
pub fn match_route_typed(path: &str, route: &Route) -> MatchResult {
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if path_segments.len() != route.segments.len() {
        return MatchResult::NoMatch;
    }

    let mut params = HashMap::new();

    for (path_seg, route_seg) in path_segments.iter().zip(route.segments.iter()) {
        match route_seg {
            RouteSegment::Static(expected) => {
                if path_seg != expected {
                    return MatchResult::NoMatch;
                }
            }
            RouteSegment::Param { name, param_type } => {
                // Validate type if specified
                if let Some(type_name) = param_type {
                    match type_name.as_str() {
                        "Int" => {
                            if path_seg.parse::<i64>().is_err() {
                                return MatchResult::TypeMismatch {
                                    param_name: name.clone(),
                                    expected: "Int".to_string(),
                                    got: path_seg.to_string(),
                                };
                            }
                        }
                        "Float" => {
                            if path_seg.parse::<f64>().is_err() {
                                return MatchResult::TypeMismatch {
                                    param_name: name.clone(),
                                    expected: "Float".to_string(),
                                    got: path_seg.to_string(),
                                };
                            }
                        }
                        // String type (or unknown) - always matches
                        _ => {}
                    }
                }
                params.insert(name.clone(), path_seg.to_string());
            }
        }
    }

    MatchResult::Matched(params)
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
    crate::stdlib::json::intent_value_to_json(value)
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

    // @ntnt text
    // @module std/http/server
    // @module_description HTTP response builders for server route handlers
    // @signature text(body: String) -> Response
    // Create a plain-text HTTP response with status 200.
    //
    // Wraps the given string in a Response map with Content-Type set to
    // `text/plain; charset=utf-8` and cache-control headers that prevent
    // browser caching of dynamic content.
    // @param body The plain-text string to send as the response body.
    // @returns A Response map with status 200, text/plain content-type, and no-cache headers.
    // @see_also html, json, status, redirect, response
    // @since v0.1.0
    // @tags #http, #server
    // @example text("Hello, World!") => Response { status: 200, body: "Hello, World!" } ~ "Plain text response"
    // @error TypeError ~ "text() requires a string" fix: "Pass a String value as the argument"
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

    // @ntnt html
    // @module std/http/server
    // @signature html(body: String, status_code?: Int) -> Response
    // Create an HTML HTTP response.
    //
    // Returns a Response map with Content-Type `text/html; charset=utf-8`.
    // Accepts an optional second argument to override the default 200 status code.
    // Includes cache-control and pragma headers to prevent browser caching of
    // dynamic HTML content.
    // @param body The HTML string to send as the response body.
    // @param status_code Optional HTTP status code (defaults to 200).
    // @returns A Response map with the given status, text/html content-type, and no-cache headers.
    // @see_also text, json, status, redirect, response
    // @since v0.1.0
    // @tags #http, #server
    // @example html("<h1>Hello</h1>") => Response { status: 200, body: "<h1>Hello</h1>" } ~ "HTML response"
    // @example html("<h1>Not Found</h1>", 404) => Response { status: 404 } ~ "HTML with custom status"
    // @error TypeError ~ "html() requires 1 or 2 arguments (body, optional status_code)" fix: "Pass 1 or 2 arguments"
    // @error TypeError ~ "html() body must be a string" fix: "Ensure the first argument is a String"
    // @error TypeError ~ "html() status code must be an integer" fix: "Pass an Int as the second argument"
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

    // @ntnt json
    // @module std/http/server
    // @signature json(data: Any, status_code?: Int) -> Response
    // Create a JSON HTTP response.
    //
    // Serializes the given value (typically a Map or Array) to a JSON string
    // and returns a Response with Content-Type `application/json`. Accepts an
    // optional second argument to override the default 200 status code. Includes
    // cache-control headers to prevent browser caching of API responses.
    // @param data The value to serialize as JSON (Map, Array, String, Int, Float, Bool, or Unit).
    // @param status_code Optional HTTP status code (defaults to 200).
    // @returns A Response map with the given status, application/json content-type, and no-cache headers.
    // @see_also text, html, status, redirect, response, parse_json
    // @since v0.1.0
    // @tags #http, #server
    // @example json(map { "ok": true }) => Response { status: 200, body: "{\"ok\":true}" } ~ "JSON response"
    // @example json(map { "error": "not found" }, 404) => Response { status: 404 } ~ "JSON with custom status"
    // @error TypeError ~ "json() requires 1 or 2 arguments (data, optional status_code)" fix: "Pass 1 or 2 arguments"
    // @error TypeError ~ "json() status code must be an integer" fix: "Pass an Int as the second argument"
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

    // @ntnt status
    // @module std/http/server
    // @signature status(code: Int, body: String) -> Response
    // Create a plain-text HTTP response with an explicit status code.
    //
    // Returns a Response map with the specified status code, Content-Type
    // `text/plain; charset=utf-8`, and the provided body string.
    // @param code The HTTP status code (e.g., 201, 400, 503).
    // @param body The plain-text body string.
    // @returns A Response map with the given status and text/plain content-type.
    // @see_also text, html, json, redirect, error, response
    // @since v0.1.0
    // @tags #http, #server
    // @example status(201, "Created") => Response { status: 201, body: "Created" } ~ "Custom status response"
    // @error TypeError ~ "status() requires int and string" fix: "Pass an Int status code and a String body"
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

    // @ntnt redirect
    // @module std/http/server
    // @signature redirect(url: String) -> Response
    // Create an HTTP 302 redirect response.
    //
    // Returns a Response map with status 302 and a `Location` header set to
    // the provided URL. The body is empty.
    //
    // WARNING: This function does NOT validate the URL. If user input flows into
    // this function, attackers can redirect users to malicious sites (open redirect).
    // Use `redirect_safe()` instead when the URL comes from user input.
    // @param url The URL to redirect the client to (absolute or relative path).
    // @returns A Response map with status 302, a Location header, and an empty body.
    // @see_also redirect_safe, text, html, json, status, response
    // @since v0.1.0
    // @tags #http, #server
    // @example redirect("/dashboard") => Response { status: 302, headers: { "location": "/dashboard" } } ~ "Redirect response"
    // @error TypeError ~ "redirect() requires a URL string" fix: "Pass a String URL as the argument"
    // @gotcha Does not validate URLs - use redirect_safe() for user-provided URLs
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

    // @ntnt redirect_safe
    // @module std/http/server
    // @signature redirect_safe(url: String, fallback?: String) -> Response
    // Create a safe HTTP 302 redirect response that prevents open redirect attacks.
    //
    // Only allows redirects to relative paths (e.g., /dashboard, ./page, ../back).
    // Rejects absolute URLs, protocol-relative URLs (//evil.com), and dangerous
    // schemes (javascript:, data:, etc.). If the URL is unsafe, redirects to the
    // fallback URL (default: "/").
    //
    // Use this function instead of `redirect()` when the URL comes from user input
    // (e.g., query parameters, form fields, database values).
    // @param url The URL to redirect to (must be a relative path for safety).
    // @param fallback Optional fallback URL if the provided URL is unsafe (default: "/").
    // @returns A Response map with status 302, a Location header, and an empty body.
    // @see_also redirect, text, html, json, status, response
    // @since v0.3.11
    // @tags #http, #server, #security
    // @example redirect_safe("/dashboard") => Response { status: 302, headers: { "location": "/dashboard" } } ~ "Safe relative redirect"
    // @example redirect_safe("https://evil.com") => Response { status: 302, headers: { "location": "/" } } ~ "Unsafe URL redirects to fallback"
    // @example redirect_safe("//evil.com/path", "/home") => Response { status: 302, headers: { "location": "/home" } } ~ "Protocol-relative URL rejected"
    // @error TypeError ~ "redirect_safe() requires a URL string" fix: "Pass a String URL as the first argument"
    module.insert(
        "redirect_safe".to_string(),
        Value::NativeFunction {
            name: "redirect_safe".to_string(),
            arity: 0, // Variadic: 1 or 2 args
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "redirect_safe() requires 1 or 2 arguments (url, optional fallback)"
                            .to_string(),
                    ));
                }

                let url = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "redirect_safe() requires a URL string".to_string(),
                        ))
                    }
                };

                let fallback = if args.len() == 2 {
                    match &args[1] {
                        Value::String(s) => s.clone(),
                        _ => "/".to_string(),
                    }
                } else {
                    "/".to_string()
                };

                // Use the safe URL or fallback
                let safe_url = if is_safe_redirect_url(&url) {
                    url
                } else {
                    fallback
                };

                let mut headers = HashMap::new();
                headers.insert("location".to_string(), Value::String(safe_url));
                Ok(create_response_value(302, headers, String::new()))
            },
        },
    );

    // @ntnt not_found
    // @module std/http/server
    // @signature not_found() -> Response
    // Create an HTTP 404 Not Found response.
    //
    // Returns a Response map with status 404, Content-Type `text/plain; charset=utf-8`,
    // and body "Not Found". Takes no arguments.
    // @returns A Response map with status 404, text/plain content-type, and body "Not Found".
    // @see_also error, status, text, html, json, response
    // @since v0.1.0
    // @tags #http, #server
    // @example not_found() => Response { status: 404, body: "Not Found" } ~ "404 response"
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

    // @ntnt error
    // @module std/http/server
    // @signature error(message: String) -> Response
    // Create an HTTP 500 Internal Server Error response.
    //
    // Returns a Response map with status 500, Content-Type `text/plain; charset=utf-8`,
    // and the provided message as the body.
    // @param message The error message to send as the response body.
    // @returns A Response map with status 500, text/plain content-type, and the error message body.
    // @see_also not_found, status, text, html, json, response
    // @since v0.1.0
    // @tags #http, #server
    // @example error("Something went wrong") => Response { status: 500, body: "Something went wrong" } ~ "500 error response"
    // @error TypeError ~ "error() requires a string" fix: "Pass a String message as the argument"
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

    // @ntnt static_file
    // @module std/http/server
    // @signature static_file(content: String, content_type: String, max_age?: Int) -> Response
    // Create a cacheable HTTP response for static assets.
    //
    // Returns a Response map with status 200, the specified Content-Type, and a
    // `Cache-Control: public, max-age=<seconds>` header. The optional `max_age`
    // parameter controls how long browsers cache the asset (defaults to 3600
    // seconds / 1 hour).
    // @param content The file content as a string.
    // @param content_type The MIME type for the Content-Type header (e.g., "text/css", "image/png").
    // @param max_age Optional cache duration in seconds (defaults to 3600).
    // @returns A Response map with status 200, the given content-type, and public cache-control headers.
    // @see_also text, html, json, response
    // @since v0.1.0
    // @tags #http, #server
    // @example static_file(css, "text/css") => Response { status: 200 } ~ "Static CSS with default 1h cache"
    // @example static_file(js, "application/javascript", 86400) => Response { status: 200 } ~ "Static JS with 24h cache"
    // @error TypeError ~ "static_file() requires 2-3 arguments (content, content_type, optional max_age)" fix: "Pass 2 or 3 arguments"
    // @error TypeError ~ "static_file() content must be a string" fix: "Ensure the first argument is a String"
    // @error TypeError ~ "static_file() content_type must be a string" fix: "Ensure the second argument is a String"
    // @error TypeError ~ "static_file() max_age must be an integer" fix: "Pass an Int as the third argument"
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

    // @ntnt response
    // @module std/http/server
    // @signature response(status: Int, headers: Map<String, String>, body: String) -> Response
    // Create a fully custom HTTP response.
    //
    // Provides complete control over status code, headers, and body. Header keys
    // are lowercased automatically. Use this when the convenience builders
    // (text, html, json) do not offer enough flexibility.
    // @param status The HTTP status code.
    // @param headers A Map of header names to header values.
    // @param body The response body string.
    // @returns A Response map with the given status, headers (lowercased keys), and body.
    // @see_also text, html, json, status, redirect, static_file
    // @since v0.1.0
    // @tags #http, #server
    // @example response(200, map { "X-Custom": "value" }, "OK") => Response { status: 200 } ~ "Custom response"
    // @error TypeError ~ "response() status must be an integer" fix: "Pass an Int as the first argument"
    // @error TypeError ~ "response() headers must be a map" fix: "Pass a Map as the second argument"
    // @error TypeError ~ "response() body must be a string" fix: "Pass a String as the third argument"
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

    // @ntnt parse_json
    // @module std/http/server
    // @signature parse_json(req: Request | String) -> Result<Map<String, Any>, String>
    // Parse a request body (or raw string) as JSON.
    //
    // Accepts either a Request map (extracts the `body` field) or a plain String.
    // Returns a Result enum: `Ok(value)` on success with the parsed data, or
    // `Err(message)` if the JSON is malformed. JSON null values become None.
    // @param req A Request map with a `body` field, or a raw JSON string.
    // @returns Result<Map<String, Any>, String> -- Ok with parsed value, or Err with parse error message.
    // @see_also json, parse_form
    // @since v0.1.0
    // @tags #http, #server
    // @example parse_json("{\"key\": \"value\"}") => Ok(map { "key": "value" }) ~ "Parse JSON string"
    // @example parse_json("not json") => Err("expected ...") ~ "Returns Err on invalid JSON"
    // @gotcha JSON null values are parsed as None (not Unit), matching std/json behavior
    // @error TypeError ~ "parse_json() requires a request with body" fix: "Pass a Request map that contains a body field"
    // @error TypeError ~ "parse_json() requires a request map or body string" fix: "Pass a Request map or a String"
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

    // @ntnt parse_form
    // @module std/http/server
    // @signature parse_form(req: Request | String) -> Map<String, String>
    // Parse a request body (or raw string) as URL-encoded form data.
    //
    // Accepts either a Request map (extracts the `body` field) or a plain String.
    // Splits the body on `&` and `=` to produce key-value pairs. Keys and values
    // are URL-decoded automatically. Keys without a value are mapped to an empty
    // string.
    // @param req A Request map with a `body` field, or a raw URL-encoded form string.
    // @returns A Map<String, String> of decoded form field names to values.
    // @see_also parse_json, json
    // @since v0.1.0
    // @tags #http, #server
    // @example parse_form("name=Alice&age=30") => map { "name": "Alice", "age": "30" } ~ "Parse form data"
    // @example parse_form("q=hello+world") => map { "q": "hello world" } ~ "URL-decoded values"
    // @error TypeError ~ "parse_form() requires a request with body" fix: "Pass a Request map that contains a body field"
    // @error TypeError ~ "parse_form() requires a request map or body string" fix: "Pass a Request map or a String"
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

    // ============================================================================
    // Cookie Management Functions
    // ============================================================================

    // @ntnt set_cookie
    // @module std/http/server
    // @signature set_cookie(name: String, value: String, options?: Map) -> String
    // Build a Set-Cookie header value string.
    //
    // Constructs a properly formatted Set-Cookie header value with the given
    // name, value, and optional attributes. The returned string can be used
    // as a header value directly or with the `with_cookie` helper.
    //
    // Options map supports:
    // - `path` (String): Cookie path scope (default: "/")
    // - `domain` (String): Cookie domain scope
    // - `max_age` (Int): Max age in seconds
    // - `secure` (Bool): Only send over HTTPS
    // - `http_only` (Bool): Not accessible via JavaScript
    // - `same_site` (String): "Strict", "Lax", or "None"
    // - `expires` (String): Expiration date (RFC 7231 format)
    // - `partitioned` (Bool): CHIPS partitioned cookie
    // @param name The cookie name.
    // @param value The cookie value.
    // @param options Optional map of cookie attributes.
    // @returns A Set-Cookie header value string.
    // @see_also get_cookie, get_cookies, delete_cookie, with_cookie
    // @since v0.3.11
    // @tags #http, #server, #cookies
    // @example set_cookie("session", "abc123") => "session=abc123; Path=/" ~ "Basic cookie"
    // @example set_cookie("token", "xyz", map { "http_only": true, "secure": true }) => "token=xyz; Path=/; HttpOnly; Secure" ~ "Secure cookie"
    // @error TypeError ~ "set_cookie() requires 2 or 3 arguments" fix: "Pass name, value, and optional options map"
    module.insert(
        "set_cookie".to_string(),
        Value::NativeFunction {
            name: "set_cookie".to_string(),
            arity: 0, // Variadic: 2-3 args
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::TypeError(
                        "set_cookie() requires 2 or 3 arguments (name, value, optional options)"
                            .to_string(),
                    ));
                }

                let name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "set_cookie() name must be a string".to_string(),
                        ))
                    }
                };

                // Validate cookie name (RFC 6265)
                if !is_valid_cookie_name(&name) {
                    return Err(IntentError::TypeError(
                        "set_cookie() name contains invalid characters (must be alphanumeric, -, _, or .)".to_string(),
                    ));
                }

                let value = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "set_cookie() value must be a string".to_string(),
                        ))
                    }
                };

                let options = if args.len() == 3 {
                    match &args[2] {
                        Value::Map(m) => m.clone(),
                        _ => {
                            return Err(IntentError::TypeError(
                                "set_cookie() options must be a map".to_string(),
                            ))
                        }
                    }
                } else {
                    HashMap::new()
                };

                let cookie_str = build_cookie_string(&name, &value, &options);
                Ok(Value::String(cookie_str))
            },
        },
    );

    // @ntnt get_cookie
    // @module std/http/server
    // @signature get_cookie(req: Request, name: String) -> Option<String>
    // Get a specific cookie value from a request.
    //
    // Parses the request's Cookie header and returns the value of the named
    // cookie wrapped in Some, or None if the cookie is not present.
    // @param req The Request map containing headers.
    // @param name The name of the cookie to retrieve.
    // @returns Some(value) if the cookie exists, None otherwise.
    // @see_also get_cookies, set_cookie, with_cookie
    // @since v0.3.11
    // @tags #http, #server, #cookies
    // @example get_cookie(req, "session") => Some("abc123") ~ "Get existing cookie"
    // @example get_cookie(req, "missing") => None ~ "Cookie not found"
    // @error TypeError ~ "get_cookie() requires a request map and cookie name" fix: "Pass a Request and String"
    module.insert(
        "get_cookie".to_string(),
        Value::NativeFunction {
            name: "get_cookie".to_string(),
            arity: 2,
            func: |args| {
                let headers = match &args[0] {
                    Value::Map(map) => match map.get("headers") {
                        Some(Value::Map(h)) => h.clone(),
                        _ => HashMap::new(),
                    },
                    _ => {
                        return Err(IntentError::TypeError(
                            "get_cookie() requires a request map".to_string(),
                        ))
                    }
                };

                let name = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "get_cookie() name must be a string".to_string(),
                        ))
                    }
                };

                // Get cookie header (case-insensitive)
                let cookie_header = headers
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == "cookie")
                    .and_then(|(_, v)| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    });

                match cookie_header {
                    Some(header) => {
                        let cookies = parse_cookie_header(&header);
                        match cookies.get(&name) {
                            Some(value) => Ok(Value::EnumValue {
                                enum_name: "Option".to_string(),
                                variant: "Some".to_string(),
                                values: vec![Value::String(value.clone())],
                            }),
                            None => Ok(Value::EnumValue {
                                enum_name: "Option".to_string(),
                                variant: "None".to_string(),
                                values: vec![],
                            }),
                        }
                    }
                    None => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    }),
                }
            },
        },
    );

    // @ntnt get_cookies
    // @module std/http/server
    // @signature get_cookies(req: Request) -> Map<String, String>
    // Get all cookies from a request as a map.
    //
    // Parses the request's Cookie header and returns all cookie name-value
    // pairs as a Map. Returns an empty map if no cookies are present.
    // @param req The Request map containing headers.
    // @returns A Map<String, String> of cookie names to values.
    // @see_also get_cookie, set_cookie, with_cookie
    // @since v0.3.11
    // @tags #http, #server, #cookies
    // @example get_cookies(req) => map { "session": "abc", "theme": "dark" } ~ "All cookies"
    // @error TypeError ~ "get_cookies() requires a request map" fix: "Pass a Request map"
    module.insert(
        "get_cookies".to_string(),
        Value::NativeFunction {
            name: "get_cookies".to_string(),
            arity: 1,
            func: |args| {
                let headers = match &args[0] {
                    Value::Map(map) => match map.get("headers") {
                        Some(Value::Map(h)) => h.clone(),
                        _ => HashMap::new(),
                    },
                    _ => {
                        return Err(IntentError::TypeError(
                            "get_cookies() requires a request map".to_string(),
                        ))
                    }
                };

                // Get cookie header (case-insensitive)
                let cookie_header = headers
                    .iter()
                    .find(|(k, _)| k.to_lowercase() == "cookie")
                    .and_then(|(_, v)| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    });

                let cookies: HashMap<String, Value> = match cookie_header {
                    Some(header) => parse_cookie_header(&header)
                        .into_iter()
                        .map(|(k, v)| (k, Value::String(v)))
                        .collect(),
                    None => HashMap::new(),
                };

                Ok(Value::Map(cookies))
            },
        },
    );

    // @ntnt delete_cookie
    // @module std/http/server
    // @signature delete_cookie(name: String, options?: Map) -> String
    // Build a Set-Cookie header value that deletes a cookie.
    //
    // Returns a Set-Cookie header string with Max-Age=0 to instruct the browser
    // to delete the cookie. The options map can specify `path` and `domain` to
    // ensure the correct cookie is deleted.
    // @param name The name of the cookie to delete.
    // @param options Optional map with `path` and `domain` to match the original cookie.
    // @returns A Set-Cookie header value string that deletes the cookie.
    // @see_also set_cookie, with_cookie
    // @since v0.3.11
    // @tags #http, #server, #cookies
    // @example delete_cookie("session") => "session=; Path=/; Max-Age=0" ~ "Delete cookie"
    // @error TypeError ~ "delete_cookie() requires 1 or 2 arguments" fix: "Pass cookie name and optional options"
    module.insert(
        "delete_cookie".to_string(),
        Value::NativeFunction {
            name: "delete_cookie".to_string(),
            arity: 0, // Variadic: 1-2 args
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "delete_cookie() requires 1 or 2 arguments (name, optional options)"
                            .to_string(),
                    ));
                }

                let name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "delete_cookie() name must be a string".to_string(),
                        ))
                    }
                };

                let mut options = if args.len() == 2 {
                    match &args[1] {
                        Value::Map(m) => m.clone(),
                        _ => {
                            return Err(IntentError::TypeError(
                                "delete_cookie() options must be a map".to_string(),
                            ))
                        }
                    }
                } else {
                    HashMap::new()
                };

                // Set max_age to 0 to delete the cookie
                options.insert("max_age".to_string(), Value::Int(0));

                let cookie_str = build_cookie_string(&name, "", &options);
                Ok(Value::String(cookie_str))
            },
        },
    );

    // @ntnt with_cookie
    // @module std/http/server
    // @signature with_cookie(response: Response, name: String, value: String, options?: Map) -> Response
    // Add a Set-Cookie header to a response.
    //
    // Returns a new Response with the Set-Cookie header added. If the response
    // already has Set-Cookie headers, the new cookie is appended (using an array
    // for multiple Set-Cookie headers). This is the ergonomic way to set cookies
    // without manually building headers.
    // @param response The Response map to add the cookie to.
    // @param name The cookie name.
    // @param value The cookie value.
    // @param options Optional map of cookie attributes (same as set_cookie).
    // @returns A new Response map with the Set-Cookie header added.
    // @see_also set_cookie, delete_cookie, get_cookie
    // @since v0.3.11
    // @tags #http, #server, #cookies
    // @example with_cookie(json(data), "session", "abc123") ~ "Add cookie to JSON response"
    // @example with_cookie(html(page), "theme", "dark", map { "max_age": 86400 }) ~ "Cookie with options"
    // @error TypeError ~ "with_cookie() requires 3 or 4 arguments" fix: "Pass response, name, value, and optional options"
    module.insert(
        "with_cookie".to_string(),
        Value::NativeFunction {
            name: "with_cookie".to_string(),
            arity: 0, // Variadic: 3-4 args
            func: |args| {
                if args.len() < 3 || args.len() > 4 {
                    return Err(IntentError::TypeError(
                        "with_cookie() requires 3 or 4 arguments (response, name, value, optional options)"
                            .to_string(),
                    ));
                }

                let mut response = match &args[0] {
                    Value::Map(m) => m.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "with_cookie() response must be a map".to_string(),
                        ))
                    }
                };

                let name = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "with_cookie() name must be a string".to_string(),
                        ))
                    }
                };

                let value = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "with_cookie() value must be a string".to_string(),
                        ))
                    }
                };

                let options = if args.len() == 4 {
                    match &args[3] {
                        Value::Map(m) => m.clone(),
                        _ => {
                            return Err(IntentError::TypeError(
                                "with_cookie() options must be a map".to_string(),
                            ))
                        }
                    }
                } else {
                    HashMap::new()
                };

                let cookie_str = build_cookie_string(&name, &value, &options);

                // Get or create headers map
                let mut headers = match response.get("headers") {
                    Some(Value::Map(h)) => h.clone(),
                    _ => HashMap::new(),
                };

                // Handle existing set-cookie header (may need to convert to array)
                let set_cookie_key = "set-cookie".to_string();
                match headers.get(&set_cookie_key) {
                    Some(Value::Array(arr)) => {
                        // Already an array, append to it
                        let mut new_arr = arr.clone();
                        new_arr.push(Value::String(cookie_str));
                        headers.insert(set_cookie_key, Value::Array(new_arr));
                    }
                    Some(Value::String(existing)) => {
                        // Convert single value to array
                        headers.insert(
                            set_cookie_key,
                            Value::Array(vec![
                                Value::String(existing.clone()),
                                Value::String(cookie_str),
                            ]),
                        );
                    }
                    _ => {
                        // No existing cookie, just set as string
                        headers.insert(set_cookie_key, Value::String(cookie_str));
                    }
                }

                response.insert("headers".to_string(), Value::Map(headers));
                Ok(Value::Map(response))
            },
        },
    );

    // ============================================================================
    // Multipart Form Parsing Functions
    // ============================================================================

    // @ntnt parse_multipart
    // @module std/http/server
    // @signature parse_multipart(req: Request) -> Result<Map<String, Any>, String>
    // Parse a multipart/form-data request body.
    //
    // Extracts fields and files from a multipart request. Text fields are returned
    // as String values. File fields are returned as Maps with: `filename` (String),
    // `content_type` (String), `size` (Int), and `data` (String - may be lossy for
    // binary files).
    //
    // Note: Binary file data passes through String conversion and may be lossy.
    // For binary files, use `save_upload()` to write directly to disk.
    // @param req The Request map with Content-Type header and body.
    // @returns Ok(Map) with field names as keys, or Err(String) on parse failure.
    // @see_also save_upload, parse_form
    // @since v0.3.11
    // @tags #http, #server, #file-upload
    // @example
    //   let fields = parse_multipart(req)?
    //   let name = fields["name"]
    //   let file = fields["document"]
    //   print("Uploaded: {file[\"filename\"]}, {file[\"size\"]} bytes")
    // @error TypeError ~ "parse_multipart() requires a request map" fix: "Pass a Request map"
    // @error ParseError ~ "Invalid multipart boundary" fix: "Ensure Content-Type header includes boundary"
    module.insert(
        "parse_multipart".to_string(),
        Value::NativeFunction {
            name: "parse_multipart".to_string(),
            arity: 1,
            func: |args| {
                let (content_type, body) = match &args[0] {
                    Value::Map(map) => {
                        let body = match map.get("body") {
                            Some(Value::String(b)) => b.clone(),
                            _ => {
                                return Ok(Value::EnumValue {
                                    enum_name: "Result".to_string(),
                                    variant: "Err".to_string(),
                                    values: vec![Value::String("Request has no body".to_string())],
                                })
                            }
                        };

                        let content_type = match map.get("headers") {
                            Some(Value::Map(headers)) => {
                                // Look for content-type header (case-insensitive)
                                headers
                                    .iter()
                                    .find(|(k, _)| k.to_lowercase() == "content-type")
                                    .and_then(|(_, v)| {
                                        if let Value::String(s) = v {
                                            Some(s.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .unwrap_or_default()
                            }
                            _ => String::new(),
                        };

                        (content_type, body)
                    }
                    _ => {
                        return Err(IntentError::TypeError(
                            "parse_multipart() requires a request map".to_string(),
                        ))
                    }
                };

                // Extract boundary from Content-Type
                let boundary = content_type.split(';').find_map(|part| {
                    let trimmed = part.trim();
                    if trimmed.starts_with("boundary=") {
                        Some(
                            trimmed
                                .trim_start_matches("boundary=")
                                .trim_matches('"')
                                .to_string(),
                        )
                    } else {
                        None
                    }
                });

                let boundary = match boundary {
                    Some(b) => b,
                    None => {
                        return Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(
                                "Invalid multipart: no boundary found in Content-Type".to_string(),
                            )],
                        })
                    }
                };

                // Parse multipart body
                match parse_multipart_body(&body, &boundary) {
                    Ok(fields) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![Value::Map(fields)],
                    }),
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(e)],
                    }),
                }
            },
        },
    );

    // @ntnt save_upload
    // @module std/http/server
    // @signature save_upload(file_field: Map, path: String) -> Result<Int, String>
    // Save an uploaded file to disk.
    //
    // Writes the file data from a parsed multipart field to the specified path.
    // Returns the number of bytes written on success. Parent directories are
    // created automatically if they don't exist.
    //
    // Security: Paths are validated to prevent directory traversal attacks.
    // Relative paths are resolved from the current working directory.
    // Paths containing `..` are rejected for security.
    // @param file_field The file field Map from parse_multipart() with a `data` key.
    // @param path The filesystem path to save the file to (relative or absolute).
    // @returns Ok(Int) bytes written, or Err(String) on failure.
    // @see_also parse_multipart
    // @since v0.3.11
    // @tags #http, #server, #file-upload, #filesystem
    // @example save_upload(fields["photo"], "uploads/photo.jpg") => Ok(1024) ~ "Save to relative path"
    // @error TypeError ~ "save_upload() requires a file map and path" fix: "Pass a file field and String path"
    // @error SecurityError ~ "Path traversal not allowed" fix: "Use a path without '..' components"
    module.insert(
        "save_upload".to_string(),
        Value::NativeFunction {
            name: "save_upload".to_string(),
            arity: 2,
            func: |args| {
                let data = match &args[0] {
                    Value::Map(map) => match map.get("data") {
                        Some(Value::String(d)) => d.clone(),
                        _ => {
                            return Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Err".to_string(),
                                values: vec![Value::String("File field has no data".to_string())],
                            })
                        }
                    },
                    _ => {
                        return Err(IntentError::TypeError(
                            "save_upload() first argument must be a file map".to_string(),
                        ))
                    }
                };

                let path = match &args[1] {
                    Value::String(p) => p.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "save_upload() second argument must be a path string".to_string(),
                        ))
                    }
                };

                // Security: Validate path to prevent directory traversal
                if let Err(e) = validate_upload_path(&path) {
                    return Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(e)],
                    });
                }

                // Create parent directories if they don't exist (convenience)
                let path_obj = std::path::Path::new(&path);
                if let Some(parent) = path_obj.parent() {
                    if !parent.as_os_str().is_empty() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            return Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Err".to_string(),
                                values: vec![Value::String(format!(
                                    "Failed to create directory: {}",
                                    e
                                ))],
                            });
                        }
                    }
                }

                // Write file to disk
                match std::fs::write(&path, data.as_bytes()) {
                    Ok(()) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![Value::Int(data.len() as i64)],
                    }),
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Failed to save file: {}", e))],
                    }),
                }
            },
        },
    );

    module
}

/// Parse a multipart/form-data body into fields
fn parse_multipart_body(
    body: &str,
    boundary: &str,
) -> std::result::Result<HashMap<String, Value>, String> {
    let mut fields = HashMap::new();
    let delimiter = format!("--{}", boundary);
    let end_delimiter = format!("--{}--", boundary);

    // Split body by boundary
    let parts: Vec<&str> = body.split(&delimiter).collect();

    for part in parts {
        let part = part.trim();

        // Skip empty parts and the final delimiter
        if part.is_empty() || part == "--" || part.starts_with("--") {
            continue;
        }

        // Split headers from content (separated by \r\n\r\n or \n\n)
        let (headers_section, content) = if let Some(pos) = part.find("\r\n\r\n") {
            (&part[..pos], &part[pos + 4..])
        } else if let Some(pos) = part.find("\n\n") {
            (&part[..pos], &part[pos + 2..])
        } else {
            continue;
        };

        // Remove trailing boundary marker from content
        let content = content
            .trim_end_matches(&end_delimiter)
            .trim_end_matches("\r\n")
            .trim_end_matches('\n');

        // Parse Content-Disposition header
        let mut name: Option<String> = None;
        let mut filename: Option<String> = None;
        let mut content_type: Option<String> = None;

        for line in headers_section.lines() {
            let line = line.trim();
            if line.to_lowercase().starts_with("content-disposition:") {
                let disposition = &line["content-disposition:".len()..];
                // Parse name="value" pairs
                for part in disposition.split(';') {
                    let part = part.trim();
                    if part.starts_with("name=") {
                        name = Some(part["name=".len()..].trim_matches('"').to_string());
                    } else if part.starts_with("filename=") {
                        // Sanitize filename to prevent path traversal and injection
                        let raw_filename = part["filename=".len()..].trim_matches('"');
                        filename = Some(sanitize_filename(raw_filename));
                    }
                }
            } else if line.to_lowercase().starts_with("content-type:") {
                content_type = Some(line["content-type:".len()..].trim().to_string());
            }
        }

        // Add field to result
        if let Some(field_name) = name {
            if let Some(fname) = filename {
                // File field
                let mut file_map = HashMap::new();
                file_map.insert("filename".to_string(), Value::String(fname));
                file_map.insert(
                    "content_type".to_string(),
                    Value::String(
                        content_type.unwrap_or_else(|| "application/octet-stream".to_string()),
                    ),
                );
                file_map.insert("size".to_string(), Value::Int(content.len() as i64));
                file_map.insert("data".to_string(), Value::String(content.to_string()));
                fields.insert(field_name, Value::Map(file_map));
            } else {
                // Text field
                fields.insert(field_name, Value::String(content.to_string()));
            }
        }
    }

    Ok(fields)
}

/// Validate an upload path to prevent directory traversal attacks
/// Returns Ok(()) if the path is safe, Err(message) if not
fn validate_upload_path(path: &str) -> std::result::Result<(), String> {
    // Reject empty paths
    if path.is_empty() {
        return Err("Path cannot be empty".to_string());
    }

    // Reject paths containing ".." (directory traversal)
    // Check both raw string and normalized path components
    if path.contains("..") {
        return Err("Path traversal ('..') not allowed for security".to_string());
    }

    // Reject null bytes (could truncate path in some systems)
    if path.contains('\0') {
        return Err("Path contains null byte".to_string());
    }

    // Normalize the path and check for traversal attempts
    let path_obj = std::path::Path::new(path);
    for component in path_obj.components() {
        match component {
            std::path::Component::ParentDir => {
                return Err("Path traversal ('..') not allowed for security".to_string());
            }
            _ => {}
        }
    }

    Ok(())
}

/// Sanitize a filename from multipart upload
/// Removes path components and dangerous characters for security
fn sanitize_filename(filename: &str) -> String {
    // Extract just the filename (no path components)
    let name = std::path::Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);

    // Remove null bytes and other dangerous characters
    let sanitized: String = name
        .chars()
        .filter(|&c| {
            c != '\0'
                && c != '/'
                && c != '\\'
                && c != ':'
                && c != '*'
                && c != '?'
                && c != '"'
                && c != '<'
                && c != '>'
                && c != '|'
        })
        .collect();

    // If empty after sanitization, use a default name
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "unnamed_file".to_string()
    } else {
        sanitized
    }
}

/// Validate a cookie name per RFC 6265
/// Cookie names must be tokens: no CTLs, spaces, or separators
fn is_valid_cookie_name(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // RFC 6265 token: exclude CTLs (0-31, 127) and separators
    const SEPARATORS: &[char] = &[
        '(', ')', '<', '>', '@', ',', ';', ':', '\\', '"', '/', '[', ']', '?', '=', '{', '}', ' ',
        '\t',
    ];
    name.chars().all(|c| {
        let code = c as u32;
        code > 31 && code < 127 && !SEPARATORS.contains(&c)
    })
}

/// Sanitize a cookie value by URL-encoding unsafe characters
/// Also strips CR/LF to prevent header injection
fn sanitize_cookie_value(value: &str) -> String {
    // First, strip any CR or LF characters (header injection prevention)
    let stripped: String = value.chars().filter(|&c| c != '\r' && c != '\n').collect();

    // URL-encode characters that are unsafe in cookie values
    // Per RFC 6265, cookie values should exclude CTLs, whitespace, quotes, comma, semicolon, backslash
    let mut result = String::with_capacity(stripped.len());
    for c in stripped.chars() {
        match c {
            // Safe characters - pass through
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '.' | '_' | '~' => result.push(c),
            // Encode everything else
            _ => {
                for byte in c.to_string().as_bytes() {
                    result.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    result
}

/// Build a Set-Cookie header string from name, value, and options
///
/// In production mode (NTNT_ENV=production), cookies get secure defaults:
/// - Secure: true (unless explicitly set to false)
/// - SameSite: Lax (unless explicitly set)
/// - HttpOnly: true for session/auth cookies (unless explicitly set to false)
///
/// These defaults can be overridden by explicitly setting the options.
fn build_cookie_string(name: &str, value: &str, options: &HashMap<String, Value>) -> String {
    let config = get_security_config();

    // Sanitize the cookie value to prevent injection attacks
    let safe_value = sanitize_cookie_value(value);
    let mut parts = vec![format!("{}={}", name, safe_value)];

    // Path (default: "/")
    let path = match options.get("path") {
        Some(Value::String(p)) => p.clone(),
        _ => "/".to_string(),
    };
    parts.push(format!("Path={}", path));

    // Domain
    if let Some(Value::String(domain)) = options.get("domain") {
        parts.push(format!("Domain={}", domain));
    }

    // Max-Age
    if let Some(Value::Int(max_age)) = options.get("max_age") {
        parts.push(format!("Max-Age={}", max_age));
    }

    // Expires
    if let Some(Value::String(expires)) = options.get("expires") {
        parts.push(format!("Expires={}", expires));
    }

    // HttpOnly - default to true in production for session/auth-related cookies
    let http_only = match options.get("http_only") {
        Some(Value::Bool(b)) => *b,
        Some(_) => false, // Non-bool value treated as not set
        None => {
            // In production, default HttpOnly to true for session-like cookie names
            if config.production_mode {
                let lower_name = name.to_lowercase();
                lower_name.contains("session")
                    || lower_name.contains("token")
                    || lower_name.contains("auth")
                    || lower_name.contains("csrf")
                    || lower_name.contains("jwt")
            } else {
                false
            }
        }
    };
    if http_only {
        parts.push("HttpOnly".to_string());
    }

    // Secure - default to true in production (unless explicitly false)
    let secure = match options.get("secure") {
        Some(Value::Bool(b)) => *b,
        Some(_) => false,               // Non-bool value treated as not set
        None => config.production_mode, // Default true in production
    };
    if secure {
        parts.push("Secure".to_string());
    }

    // SameSite - default to Lax in production
    match options.get("same_site") {
        Some(Value::String(same_site)) => {
            parts.push(format!("SameSite={}", same_site));
        }
        Some(_) => {
            // Non-string value: use production default if applicable
            if config.production_mode {
                parts.push("SameSite=Lax".to_string());
            }
        }
        None => {
            // In production, default to Lax for CSRF protection
            if config.production_mode {
                parts.push("SameSite=Lax".to_string());
            }
        }
    }

    // Partitioned (CHIPS)
    if let Some(Value::Bool(true)) = options.get("partitioned") {
        parts.push("Partitioned".to_string());
    }

    parts.join("; ")
}

/// Parse a Cookie header string into a map of name-value pairs
fn parse_cookie_header(header: &str) -> HashMap<String, String> {
    let mut cookies = HashMap::new();
    for part in header.split(';') {
        let trimmed = part.trim();
        if let Some((name, value)) = trimmed.split_once('=') {
            cookies.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    cookies
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
/// Enforces the configured max body size limit (NTNT_MAX_BODY_SIZE, default 10MB)
pub fn process_request(
    mut request: tiny_http::Request,
    params: HashMap<String, String>,
) -> Result<(Value, tiny_http::Request)> {
    let config = get_security_config();
    let max_size = config.max_body_size;

    // Check Content-Length header first for early rejection
    if let Some(content_length) = request
        .headers()
        .iter()
        .find(|h| h.field.as_str().to_ascii_lowercase() == "content-length")
        .and_then(|h| h.value.as_str().parse::<usize>().ok())
    {
        if content_length > max_size {
            return Err(IntentError::RuntimeError(format!(
                "Request body too large: {} bytes exceeds limit of {} bytes. \
                 Configure with NTNT_MAX_BODY_SIZE environment variable.",
                content_length, max_size
            )));
        }
    }

    // Read the request body with size limit using take()
    use std::io::Read;
    let mut body_string = String::new();
    let mut limited_reader = request.as_reader().take((max_size + 1) as u64);

    match limited_reader.read_to_string(&mut body_string) {
        Ok(n) => {
            if n > max_size {
                return Err(IntentError::RuntimeError(format!(
                    "Request body too large: {} bytes exceeds limit of {} bytes. \
                     Configure with NTNT_MAX_BODY_SIZE environment variable.",
                    n, max_size
                )));
            }
        }
        Err(e) => {
            return Err(IntentError::RuntimeError(format!(
                "Failed to read request body: {}",
                e
            )));
        }
    }

    // Create request value
    let req_value = request_to_value(&request, params, body_string);

    Ok((req_value, request))
}

/// Send a response back to the client
/// Automatically adds security headers (configurable via NTNT_SECURITY_HEADERS=false)
pub fn send_response(request: tiny_http::Request, response: &Value) -> Result<()> {
    let config = get_security_config();

    let (status, mut headers, body) = match response {
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

    // Add security headers if enabled (apps can override by setting headers explicitly)
    if config.security_headers {
        let security_headers = get_default_security_headers();
        for (key, value) in security_headers {
            // Only add if not already set by the application
            if !headers.contains_key(&key) {
                headers.insert(key, value);
            }
        }
    }

    // Build tiny_http response
    let mut response_builder = tiny_http::Response::from_string(body).with_status_code(status);

    // Add headers (arrays emit multiple headers with same name, e.g., Set-Cookie)
    for (key, value) in headers {
        match value {
            Value::String(v) => {
                if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), v.as_bytes()) {
                    response_builder = response_builder.with_header(header);
                }
            }
            Value::Array(arr) => {
                // Array values emit multiple headers with same key
                for item in arr {
                    if let Value::String(v) = item {
                        if let Ok(header) =
                            tiny_http::Header::from_bytes(key.as_bytes(), v.as_bytes())
                        {
                            response_builder = response_builder.with_header(header);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Force connection close to prevent stale responses on keep-alive connections
    if let Ok(header) = tiny_http::Header::from_bytes("Connection".as_bytes(), "close".as_bytes()) {
        response_builder = response_builder.with_header(header);
    }

    // Only add server identifier in development mode
    if !config.production_mode {
        if let Ok(header) =
            tiny_http::Header::from_bytes("Server".as_bytes(), "ntnt-http".as_bytes())
        {
            response_builder = response_builder.with_header(header);
        }
    }

    request
        .respond(response_builder)
        .map_err(|e| IntentError::RuntimeError(format!("Failed to send response: {}", e)))
}

/// Create an error response
/// In production mode (NTNT_ENV=production), error details are hidden unless
/// NTNT_DETAILED_ERRORS=true is explicitly set.
pub fn create_error_response(status: i64, message: &str) -> Value {
    let config = get_security_config();

    // In production, hide error details unless explicitly enabled
    let body = if config.detailed_errors {
        message.to_string()
    } else {
        // Generic messages for common status codes
        match status {
            400 => "Bad Request".to_string(),
            401 => "Unauthorized".to_string(),
            403 => "Forbidden".to_string(),
            404 => "Not Found".to_string(),
            405 => "Method Not Allowed".to_string(),
            413 => "Payload Too Large".to_string(),
            429 => "Too Many Requests".to_string(),
            500 => "Internal Server Error".to_string(),
            502 => "Bad Gateway".to_string(),
            503 => "Service Unavailable".to_string(),
            _ => format!("Error {}", status),
        }
    };

    let mut headers = HashMap::new();
    headers.insert(
        "content-type".to_string(),
        Value::String("text/plain; charset=utf-8".to_string()),
    );
    create_response_value(status, headers, body)
}

/// Check if a URL is safe for redirects (prevents open redirect vulnerabilities)
/// Returns true for:
/// - Relative paths (/foo, ./bar, ../baz)
/// - Same-origin URLs (protocol-relative URLs are rejected for safety)
/// Returns false for:
/// - Absolute URLs to external domains
/// - Protocol-relative URLs (//evil.com)
/// - javascript:, data:, and other dangerous schemes
pub fn is_safe_redirect_url(url: &str) -> bool {
    let url = url.trim();

    // Empty URL is safe (will redirect to current page)
    if url.is_empty() {
        return true;
    }

    // Reject protocol-relative URLs (//evil.com/path)
    if url.starts_with("//") {
        return false;
    }

    // Reject dangerous schemes
    let lower = url.to_lowercase();
    if lower.starts_with("javascript:")
        || lower.starts_with("data:")
        || lower.starts_with("vbscript:")
        || lower.starts_with("file:")
    {
        return false;
    }

    // If it starts with a scheme (http://, https://, etc.), it's absolute
    if url.contains("://") {
        return false;
    }

    // Relative paths are safe (/path, ./path, ../path, path)
    true
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

    // Helper to wrap match_route_typed for backward-compatible test assertions
    fn match_route(path: &str, route: &Route) -> Option<HashMap<String, String>> {
        match match_route_typed(path, route) {
            MatchResult::Matched(params) => Some(params),
            _ => None,
        }
    }

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
            RouteSegment::Param { name, .. } => assert_eq!(name, "id"),
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
            RouteSegment::Param { name, .. } => assert_eq!(name, "user_id"),
            _ => panic!("Expected param segment"),
        }
        match &segments[2] {
            RouteSegment::Static(s) => assert_eq!(s, "posts"),
            _ => panic!("Expected static segment"),
        }
        match &segments[3] {
            RouteSegment::Param { name, .. } => assert_eq!(name, "post_id"),
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
