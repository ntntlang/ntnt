//! HTTP Bridge - Connects async Axum handlers to the sync NTNT interpreter
//!
//! This module provides the communication layer between the async HTTP server
//! and the synchronous NTNT interpreter worker pool. Each worker has its own
//! Interpreter instance (not thread-safe due to Rc<RefCell<>>), and workers
//! share a multi-consumer channel (flume) for load balancing.
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                     Tokio Async Runtime                         │
//! │  ┌─────────┐  ┌─────────┐  ┌─────────┐                         │
//! │  │ Task 1  │  │ Task 2  │  │ Task N  │  ... (async handlers)   │
//! │  └────┬────┘  └────┬────┘  └────┬────┘                         │
//! │       │            │            │                               │
//! │       └────────────┼────────────┘                               │
//! │                    │                                            │
//! │              ┌─────▼─────┐                                      │
//! │              │  Channel  │  (flume MPMC + oneshot reply)        │
//! │              └─────┬─────┘                                      │
//! └────────────────────┼────────────────────────────────────────────┘
//!                      │
//! ┌────────────────────▼────────────────────────────────────────────┐
//! │                  Worker Pool                                     │
//! │  ┌────────────┐ ┌────────────┐ ┌────────────┐                   │
//! │  │  Worker 1  │ │  Worker 2  │ │  Worker N  │                   │
//! │  │  (own      │ │  (own      │ │  (own      │                   │
//! │  │   Interp.) │ │   Interp.) │ │   Interp.) │                   │
//! │  └────────────┘ └────────────┘ └────────────┘                   │
//! └─────────────────────────────────────────────────────────────────┘
//! ```

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::oneshot;

/// A serializable HTTP request that can be sent across thread boundaries
#[derive(Debug, Clone)]
pub struct BridgeRequest {
    /// HTTP method (GET, POST, etc.)
    pub method: String,
    /// Request path (e.g., "/users/123")
    pub path: String,
    /// Full URL including query string
    pub url: String,
    /// Query string (after ?)
    pub query: String,
    /// Parsed query parameters
    pub query_params: HashMap<String, String>,
    /// Route parameters extracted from path (e.g., {id} -> "123")
    pub params: HashMap<String, String>,
    /// HTTP headers (lowercase keys)
    pub headers: HashMap<String, String>,
    /// Request body as string
    pub body: String,
    /// Unique request ID
    pub id: String,
    /// Client IP address
    pub ip: String,
    /// Protocol (http/https)
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

        // Query params
        let query_params: HashMap<String, Value> = self
            .query_params
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        map.insert("query_params".to_string(), Value::Map(query_params));

        // Route params
        let params: HashMap<String, Value> = self
            .params
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        map.insert("params".to_string(), Value::Map(params));

        // Headers
        let headers: HashMap<String, Value> = self
            .headers
            .iter()
            .map(|(k, v)| (k.clone(), Value::String(v.clone())))
            .collect();
        map.insert("headers".to_string(), Value::Map(headers));

        // Empty context map for middleware to populate (e.g., authenticated user, feature flags)
        map.insert("context".to_string(), Value::Map(HashMap::new()));

        Value::Map(map)
    }
}

/// A serializable HTTP response that can be sent back from the interpreter
#[derive(Debug, Clone)]
pub struct BridgeResponse {
    /// HTTP status code
    pub status: u16,
    /// Response headers (Vec allows multiple headers with same name, e.g., Set-Cookie)
    pub headers: Vec<(String, String)>,
    /// Response body
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

                // Flatten headers - arrays become multiple entries with same key (e.g., Set-Cookie)
                let mut headers = Vec::new();
                if let Some(Value::Map(h)) = map.get("headers") {
                    for (k, v) in h {
                        match v {
                            Value::String(val) => {
                                headers.push((k.clone(), val.clone()));
                            }
                            Value::Array(arr) => {
                                // Array values emit multiple headers with same key
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

/// Message sent from async handlers to the interpreter thread
pub struct HandlerRequest {
    /// The HTTP request data
    pub request: BridgeRequest,
    /// Channel to send the response back
    pub reply_tx: oneshot::Sender<BridgeResponse>,
}

/// Handle to send requests to the interpreter worker pool
#[derive(Clone)]
pub struct InterpreterHandle {
    tx: flume::Sender<HandlerRequest>,
}

impl InterpreterHandle {
    /// Create a new handle with the given sender
    pub fn new(tx: flume::Sender<HandlerRequest>) -> Self {
        InterpreterHandle { tx }
    }

    /// Send a request to the interpreter and wait for response
    pub async fn call(&self, request: BridgeRequest) -> Result<BridgeResponse> {
        let (reply_tx, reply_rx) = oneshot::channel();

        let handler_request = HandlerRequest { request, reply_tx };

        self.tx
            .send_async(handler_request)
            .await
            .map_err(|_| IntentError::runtime_error("Interpreter channel closed".to_string()))?;

        reply_rx
            .await
            .map_err(|_| IntentError::runtime_error("Interpreter did not respond".to_string()))
    }
}

/// Configuration for the interpreter bridge
pub struct BridgeConfig {
    /// Channel buffer size (number of pending requests)
    pub channel_buffer: usize,
}

impl Default for BridgeConfig {
    fn default() -> Self {
        BridgeConfig {
            channel_buffer: 1024,
        }
    }
}

/// Create a channel pair for interpreter communication (MPMC via flume)
pub fn create_channel(
    config: &BridgeConfig,
) -> (
    flume::Sender<HandlerRequest>,
    flume::Receiver<HandlerRequest>,
) {
    flume::bounded(config.channel_buffer)
}

/// Wrapper to make InterpreterHandle work with Axum's State extractor
pub type SharedHandle = Arc<InterpreterHandle>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_request_to_value() {
        let req = BridgeRequest {
            method: "GET".to_string(),
            path: "/users/42".to_string(),
            url: "/users/42?foo=bar".to_string(),
            query: "foo=bar".to_string(),
            query_params: [("foo".to_string(), "bar".to_string())]
                .into_iter()
                .collect(),
            params: [("id".to_string(), "42".to_string())].into_iter().collect(),
            headers: [("content-type".to_string(), "application/json".to_string())]
                .into_iter()
                .collect(),
            body: "".to_string(),
            id: "req-123".to_string(),
            ip: "127.0.0.1".to_string(),
            protocol: "http".to_string(),
        };

        let value = req.to_value();

        if let Value::Map(map) = value {
            match map.get("method") {
                Some(Value::String(m)) => assert_eq!(m, "GET"),
                _ => panic!("Expected method"),
            }
            match map.get("path") {
                Some(Value::String(p)) => assert_eq!(p, "/users/42"),
                _ => panic!("Expected path"),
            }
            if let Some(Value::Map(params)) = map.get("params") {
                match params.get("id") {
                    Some(Value::String(id)) => assert_eq!(id, "42"),
                    _ => panic!("Expected id param"),
                }
            }
        } else {
            panic!("Expected Map");
        }
    }

    #[test]
    fn test_bridge_response_from_value() {
        let mut headers = HashMap::new();
        headers.insert(
            "content-type".to_string(),
            Value::String("application/json".to_string()),
        );

        let mut map = HashMap::new();
        map.insert("status".to_string(), Value::Int(201));
        map.insert("body".to_string(), Value::String("{\"id\":1}".to_string()));
        map.insert("headers".to_string(), Value::Map(headers));

        let value = Value::Map(map);
        let response = BridgeResponse::from_value(&value);

        assert_eq!(response.status, 201);
        assert_eq!(response.body, "{\"id\":1}");
        assert!(response
            .headers
            .contains(&("content-type".to_string(), "application/json".to_string())));
    }

    #[test]
    fn test_bridge_response_array_headers() {
        // Test that array header values become multiple headers (for Set-Cookie)
        let mut headers = HashMap::new();
        headers.insert(
            "set-cookie".to_string(),
            Value::Array(vec![
                Value::String("session=abc123; Path=/".to_string()),
                Value::String("theme=dark; Path=/".to_string()),
            ]),
        );

        let mut map = HashMap::new();
        map.insert("status".to_string(), Value::Int(200));
        map.insert("body".to_string(), Value::String("OK".to_string()));
        map.insert("headers".to_string(), Value::Map(headers));

        let value = Value::Map(map);
        let response = BridgeResponse::from_value(&value);

        // Should have 2 set-cookie headers
        let set_cookie_count = response
            .headers
            .iter()
            .filter(|(k, _)| k == "set-cookie")
            .count();
        assert_eq!(set_cookie_count, 2);
        assert!(response.headers.contains(&(
            "set-cookie".to_string(),
            "session=abc123; Path=/".to_string()
        )));
        assert!(response
            .headers
            .contains(&("set-cookie".to_string(), "theme=dark; Path=/".to_string())));
    }

    #[test]
    fn test_bridge_response_error() {
        let response = BridgeResponse::error(500, "Internal Server Error");
        assert_eq!(response.status, 500);
        assert_eq!(response.body, "Internal Server Error");
    }

    #[tokio::test]
    async fn test_channel_creation() {
        let config = BridgeConfig::default();
        let (tx, rx) = create_channel(&config);

        // Spawn a mock "interpreter" that echoes back
        tokio::spawn(async move {
            if let Ok(req) = rx.recv_async().await {
                let response = BridgeResponse {
                    status: 200,
                    headers: Vec::new(),
                    body: format!("Echo: {}", req.request.path),
                };
                let _ = req.reply_tx.send(response);
            }
        });

        let handle = InterpreterHandle::new(tx);
        let request = BridgeRequest {
            method: "GET".to_string(),
            path: "/test".to_string(),
            url: "/test".to_string(),
            query: "".to_string(),
            query_params: HashMap::new(),
            params: HashMap::new(),
            headers: HashMap::new(),
            body: "".to_string(),
            id: "1".to_string(),
            ip: "127.0.0.1".to_string(),
            protocol: "http".to_string(),
        };

        let response = handle.call(request).await.unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, "Echo: /test");
    }
}
