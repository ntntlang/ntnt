//! HTTP Server module for Intent
//! 
//! Provides a simple HTTP server with routing support for building web applications.
//! 
//! Example usage:
//! ```intent
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

use std::collections::HashMap;
use crate::error::{IntentError, Result};
use crate::interpreter::Value;

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

/// Server state stored in the interpreter
#[derive(Debug, Clone)]
pub struct ServerState {
    pub routes: Vec<(Route, Value)>,  // Routes with their handlers
}

impl ServerState {
    pub fn new() -> Self {
        ServerState {
            routes: Vec::new(),
        }
    }
    
    pub fn clear(&mut self) {
        self.routes.clear();
    }
    
    pub fn add_route(&mut self, method: &str, pattern: &str, handler: Value) {
        let route = Route {
            method: method.to_string(),
            pattern: pattern.to_string(),
            segments: parse_route_pattern(pattern),
        };
        self.routes.push((route, handler));
    }
    
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }
    
    pub fn find_route(&self, method: &str, path: &str) -> Option<(Value, HashMap<String, String>)> {
        for (route, handler) in &self.routes {
            if route.method == method {
                if let Some(params) = match_route(path, route) {
                    return Some((handler.clone(), params));
                }
            }
        }
        None
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
                RouteSegment::Param(segment[1..segment.len()-1].to_string())
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
    req_map.insert("method".to_string(), Value::String(request.method().to_string()));
    
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
    for header in request.headers() {
        headers.insert(
            header.field.to_string().to_lowercase(),
            Value::String(header.value.to_string()),
        );
    }
    req_map.insert("headers".to_string(), Value::Map(headers));
    
    // Body
    req_map.insert("body".to_string(), Value::String(body));
    
    Value::Map(req_map)
}

/// Convert Intent Value to JSON for response serialization
fn intent_value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Int(n) => serde_json::Value::Number((*n).into()),
        Value::Float(f) => serde_json::Value::Number(
            serde_json::Number::from_f64(*f).unwrap_or_else(|| serde_json::Number::from(0))
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
    module.insert("text".to_string(), Value::NativeFunction {
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
                    Ok(create_response_value(200, headers, body.clone()))
                }
                _ => Err(IntentError::TypeError("text() requires a string".to_string())),
            }
        },
    });
    
    // Response.html(body) -> Response - Create an HTML response
    module.insert("html".to_string(), Value::NativeFunction {
        name: "html".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(body) => {
                    let mut headers = HashMap::new();
                    headers.insert(
                        "content-type".to_string(),
                        Value::String("text/html; charset=utf-8".to_string()),
                    );
                    Ok(create_response_value(200, headers, body.clone()))
                }
                _ => Err(IntentError::TypeError("html() requires a string".to_string())),
            }
        },
    });
    
    // Response.json(data) -> Response - Create a JSON response
    module.insert("json".to_string(), Value::NativeFunction {
        name: "json".to_string(),
        arity: 1,
        func: |args| {
            let json_value = intent_value_to_json(&args[0]);
            let body = json_value.to_string();
            let mut headers = HashMap::new();
            headers.insert(
                "content-type".to_string(),
                Value::String("application/json".to_string()),
            );
            Ok(create_response_value(200, headers, body))
        },
    });
    
    // Response.status(code, body) -> Response - Create response with status code
    module.insert("status".to_string(), Value::NativeFunction {
        name: "status".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Int(code), Value::String(body)) => {
                    let mut headers = HashMap::new();
                    headers.insert(
                        "content-type".to_string(),
                        Value::String("text/plain; charset=utf-8".to_string()),
                    );
                    Ok(create_response_value(*code, headers, body.clone()))
                }
                _ => Err(IntentError::TypeError("status() requires int and string".to_string())),
            }
        },
    });
    
    // Response.redirect(url) -> Response - Create a redirect response
    module.insert("redirect".to_string(), Value::NativeFunction {
        name: "redirect".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(url) => {
                    let mut headers = HashMap::new();
                    headers.insert(
                        "location".to_string(),
                        Value::String(url.clone()),
                    );
                    Ok(create_response_value(302, headers, String::new()))
                }
                _ => Err(IntentError::TypeError("redirect() requires a URL string".to_string())),
            }
        },
    });
    
    // Response.not_found() -> Response - Create a 404 response
    module.insert("not_found".to_string(), Value::NativeFunction {
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
    });
    
    // Response.error(message) -> Response - Create a 500 error response
    module.insert("error".to_string(), Value::NativeFunction {
        name: "error".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::String(msg) => {
                    let mut headers = HashMap::new();
                    headers.insert(
                        "content-type".to_string(),
                        Value::String("text/plain; charset=utf-8".to_string()),
                    );
                    Ok(create_response_value(500, headers, msg.clone()))
                }
                _ => Err(IntentError::TypeError("error() requires a string".to_string())),
            }
        },
    });
    
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

/// Read request body and create request Value
pub fn process_request(
    mut request: tiny_http::Request,
    params: HashMap<String, String>,
) -> Result<(Value, tiny_http::Request)> {
    // Read the request body
    let mut body = String::new();
    if let Err(e) = request.as_reader().read_to_string(&mut body) {
        return Err(IntentError::RuntimeError(format!("Failed to read request body: {}", e)));
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
    let mut response_builder = tiny_http::Response::from_string(body)
        .with_status_code(status);
    
    // Add headers
    for (key, value) in headers {
        if let Value::String(v) = value {
            if let Ok(header) = tiny_http::Header::from_bytes(key.as_bytes(), v.as_bytes()) {
                response_builder = response_builder.with_header(header);
            }
        }
    }
    
    request.respond(response_builder)
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
        let (handler, params) = result.unwrap();
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
        let (handler, _) = state.find_route("GET", "/").unwrap();
        assert_value_string(&handler, "home");
        
        let (handler, _) = state.find_route("GET", "/users").unwrap();
        assert_value_string(&handler, "list_users");
        
        let (handler, params) = state.find_route("GET", "/users/42").unwrap();
        assert_value_string(&handler, "get_user");
        assert_eq!(params.get("id"), Some(&"42".to_string()));
        
        let (handler, _) = state.find_route("POST", "/users").unwrap();
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
        let value = Value::Array(vec![
            Value::Int(1),
            Value::Int(2),
            Value::Int(3),
        ]);
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
        map.insert("scores".to_string(), Value::Array(vec![
            Value::Int(100),
            Value::Int(95),
        ]));
        
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
                assert_eq!(get_map_string(&headers, "content-type"), "text/plain; charset=utf-8");
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
                assert_eq!(get_map_string(&headers, "content-type"), "text/html; charset=utf-8");
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
            let args = vec![
                Value::Int(404),
                Value::String("Not Found".to_string()),
            ];
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
}
