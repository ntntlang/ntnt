//! std/http module - HTTP client for making requests

use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::json::{intent_value_to_json, json_to_intent_value};
use base64::Engine;
use reqwest::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use std::collections::HashMap;
use std::fs::File;
use std::io::Write;
use std::path::Path;

type Result<T> = std::result::Result<T, IntentError>;

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
    response_map.insert("ok".to_string(), Value::Bool(status >= 200 && status < 300));

    // Final URL after redirects
    response_map.insert("url".to_string(), Value::String(final_url.to_string()));

    // Whether the request was redirected
    response_map.insert(
        "redirected".to_string(),
        Value::Bool(final_url != original_url),
    );

    Value::Map(response_map)
}

/// HTTP GET request
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

/// HTTP POST request
fn http_post(url: &str, body: &str, content_type: Option<&str>) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    let ct = content_type.unwrap_or("text/plain");
    match client
        .post(url)
        .header("Content-Type", ct)
        .body(body.to_string())
        .send()
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            match response.text() {
                Ok(resp_body) => {
                    let resp_value =
                        response_to_value(status, &headers, resp_body, &final_url, url);
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

/// HTTP PUT request
fn http_put(url: &str, body: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    match client
        .put(url)
        .header("Content-Type", "text/plain")
        .body(body.to_string())
        .send()
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            match response.text() {
                Ok(resp_body) => {
                    let resp_value =
                        response_to_value(status, &headers, resp_body, &final_url, url);
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

/// HTTP DELETE request
fn http_delete(url: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    match client.delete(url).send() {
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

/// HTTP PATCH request
fn http_patch(url: &str, body: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    match client
        .patch(url)
        .header("Content-Type", "text/plain")
        .body(body.to_string())
        .send()
    {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            match response.text() {
                Ok(resp_body) => {
                    let resp_value =
                        response_to_value(status, &headers, resp_body, &final_url, url);
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

/// HTTP HEAD request
fn http_head(url: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    match client.head(url).send() {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            // HEAD has no body
            let resp_value = response_to_value(status, &headers, String::new(), &final_url, url);
            Ok(Value::EnumValue {
                enum_name: "Result".to_string(),
                variant: "Ok".to_string(),
                values: vec![resp_value],
            })
        }
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("HTTP request failed: {}", e))],
        }),
    }
}

/// HTTP request with full options
fn http_request(opts: &HashMap<String, Value>) -> Result<Value> {
    let url = match opts.get("url") {
        Some(Value::String(u)) => u.clone(),
        _ => {
            return Err(IntentError::TypeError(
                "request() requires 'url' option".to_string(),
            ))
        }
    };

    let method = match opts.get("method") {
        Some(Value::String(m)) => m.to_uppercase(),
        _ => "GET".to_string(),
    };

    let client = reqwest::blocking::Client::builder()
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

    // Add body
    if let Some(Value::String(body)) = opts.get("body") {
        request = request.body(body.clone());
    }

    // Add timeout (in seconds)
    if let Some(Value::Int(timeout)) = opts.get("timeout") {
        request = request.timeout(std::time::Duration::from_secs(*timeout as u64));
    }

    // Add cache control
    if let Some(Value::String(cache)) = opts.get("cache") {
        request = match cache.as_str() {
            "no-store" => request.header("Cache-Control", "no-store"),
            "no-cache" => request
                .header("Cache-Control", "no-cache")
                .header("Pragma", "no-cache"),
            "reload" => request
                .header("Cache-Control", "no-cache")
                .header("Pragma", "no-cache"),
            "force-cache" => request.header("Cache-Control", "max-stale=31536000"),
            "only-if-cached" => request.header("Cache-Control", "only-if-cached"),
            _ => request, // "default" or unknown - no special headers
        };
    }

    // Add referrer and referrerPolicy
    let referrer_policy = opts
        .get("referrerPolicy")
        .and_then(|v| {
            if let Value::String(s) = v {
                Some(s.as_str())
            } else {
                None
            }
        })
        .unwrap_or("strict-origin-when-cross-origin");

    if let Some(Value::String(referrer)) = opts.get("referrer") {
        // Apply referrer based on policy
        let should_send = match referrer_policy {
            "no-referrer" => false,
            "origin" | "strict-origin" => true,
            "same-origin" => {
                // Check if same origin
                let ref_origin = referrer.split('/').take(3).collect::<Vec<_>>().join("/");
                let url_origin = url.split('/').take(3).collect::<Vec<_>>().join("/");
                ref_origin == url_origin
            }
            _ => true, // default, unsafe-url, origin-when-cross-origin, strict-origin-when-cross-origin
        };

        if should_send {
            let ref_value = match referrer_policy {
                "origin" | "strict-origin" => {
                    // Send only origin (scheme + host)
                    referrer.split('/').take(3).collect::<Vec<_>>().join("/") + "/"
                }
                "origin-when-cross-origin" | "strict-origin-when-cross-origin" => {
                    let ref_origin = referrer.split('/').take(3).collect::<Vec<_>>().join("/");
                    let url_origin = url.split('/').take(3).collect::<Vec<_>>().join("/");
                    if ref_origin == url_origin {
                        referrer.clone() // Same origin - send full URL
                    } else {
                        ref_origin + "/" // Cross-origin - send only origin
                    }
                }
                _ => referrer.clone(), // unsafe-url, same-origin - send full URL
            };
            request = request.header("Referer", ref_value);
        }
    }

    match request.send() {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            match response.text() {
                Ok(body) => {
                    let resp_value = response_to_value(status, &headers, body, &final_url, &url);
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

/// HTTP GET with JSON parsing
fn http_get_json(url: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::new();
    match client.get(url).header("Accept", "application/json").send() {
        Ok(response) => {
            let status = response.status().as_u16();
            if status >= 200 && status < 300 {
                match response.text() {
                    Ok(body) => match serde_json::from_str::<serde_json::Value>(&body) {
                        Ok(json) => {
                            let value = json_to_intent_value(&json);
                            Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Ok".to_string(),
                                values: vec![value],
                            })
                        }
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Failed to parse JSON: {}", e))],
                        }),
                    },
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!(
                            "Failed to read response body: {}",
                            e
                        ))],
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

/// HTTP POST JSON data
fn http_post_json(url: &str, data: &Value) -> Result<Value> {
    let json_body = intent_value_to_json(data);
    let json_str = json_body.to_string();

    let client = reqwest::blocking::Client::new();
    match client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .body(json_str)
        .send()
    {
        Ok(response) => {
            let status = response.status().as_u16();
            if status >= 200 && status < 300 {
                match response.text() {
                    Ok(body) => {
                        if body.is_empty() {
                            // Empty response body is OK for some POST requests
                            Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Ok".to_string(),
                                values: vec![Value::Unit],
                            })
                        } else {
                            match serde_json::from_str::<serde_json::Value>(&body) {
                                Ok(json) => {
                                    let value = json_to_intent_value(&json);
                                    Ok(Value::EnumValue {
                                        enum_name: "Result".to_string(),
                                        variant: "Ok".to_string(),
                                        values: vec![value],
                                    })
                                }
                                Err(e) => Ok(Value::EnumValue {
                                    enum_name: "Result".to_string(),
                                    variant: "Err".to_string(),
                                    values: vec![Value::String(format!(
                                        "Failed to parse JSON: {}",
                                        e
                                    ))],
                                }),
                            }
                        }
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

/// HTTP GET with Basic Authentication
fn http_basic_auth(url: &str, username: &str, password: &str) -> Result<Value> {
    let credentials = format!("{}:{}", username, password);
    let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
    let auth_header = format!("Basic {}", encoded);

    let client = reqwest::blocking::Client::new();
    match client.get(url).header(AUTHORIZATION, &auth_header).send() {
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

/// HTTP POST with form data (application/x-www-form-urlencoded)
fn http_post_form(url: &str, form_data: &HashMap<String, Value>) -> Result<Value> {
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

    let client = reqwest::blocking::Client::new();
    match client.post(url).form(&form).send() {
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
            if status >= 200 && status < 300 {
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

/// Upload a file to URL (multipart form data)
fn http_upload(url: &str, file_path: &str, field_name: &str) -> Result<Value> {
    use reqwest::blocking::multipart;

    let path = Path::new(file_path);
    if !path.exists() {
        return Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("File not found: {}", file_path))],
        });
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string();

    let form = multipart::Form::new()
        .file(field_name.to_string(), file_path)
        .map_err(|e| {
            IntentError::RuntimeError(format!("Failed to create form with file: {}", e))
        })?;

    let client = reqwest::blocking::Client::new();
    match client.post(url).multipart(form).send() {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();
            match response.text() {
                Ok(body) => {
                    let mut resp_value = response_to_value(status, &headers, body, &final_url, url);
                    // Add the filename to the response for convenience
                    if let Value::Map(ref mut map) = resp_value {
                        map.insert("filename".to_string(), Value::String(file_name));
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

/// HTTP request with cookies
fn http_request_with_cookies(opts: &HashMap<String, Value>) -> Result<Value> {
    let url = match opts.get("url") {
        Some(Value::String(u)) => u.clone(),
        _ => {
            return Err(IntentError::TypeError(
                "request() requires 'url' option".to_string(),
            ))
        }
    };

    let method = match opts.get("method") {
        Some(Value::String(m)) => m.to_uppercase(),
        _ => "GET".to_string(),
    };

    // Build client with cookie store if needed
    let client_builder = reqwest::blocking::Client::builder();
    let client = client_builder
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

    // Add cookies from map
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
    if let (Some(Value::String(username)), Some(Value::String(password))) =
        (opts.get("username"), opts.get("password"))
    {
        let credentials = format!("{}:{}", username, password);
        let encoded = base64::engine::general_purpose::STANDARD.encode(credentials.as_bytes());
        request = request.header(AUTHORIZATION, format!("Basic {}", encoded));
    }

    // Add body
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

    // Add timeout (in seconds)
    if let Some(Value::Int(timeout)) = opts.get("timeout") {
        request = request.timeout(std::time::Duration::from_secs(*timeout as u64));
    }

    // Add cache control
    if let Some(Value::String(cache)) = opts.get("cache") {
        request = match cache.as_str() {
            "no-store" => request.header("Cache-Control", "no-store"),
            "no-cache" => request
                .header("Cache-Control", "no-cache")
                .header("Pragma", "no-cache"),
            "reload" => request
                .header("Cache-Control", "no-cache")
                .header("Pragma", "no-cache"),
            "force-cache" => request.header("Cache-Control", "max-stale=31536000"),
            "only-if-cached" => request.header("Cache-Control", "only-if-cached"),
            _ => request, // "default" or unknown - no special headers
        };
    }

    // Add referrer and referrerPolicy
    let referrer_policy = opts
        .get("referrerPolicy")
        .and_then(|v| {
            if let Value::String(s) = v {
                Some(s.as_str())
            } else {
                None
            }
        })
        .unwrap_or("strict-origin-when-cross-origin");

    if let Some(Value::String(referrer)) = opts.get("referrer") {
        // Apply referrer based on policy
        let should_send = match referrer_policy {
            "no-referrer" => false,
            "origin" | "strict-origin" => true,
            "same-origin" => {
                // Check if same origin
                let ref_origin = referrer.split('/').take(3).collect::<Vec<_>>().join("/");
                let url_origin = url.split('/').take(3).collect::<Vec<_>>().join("/");
                ref_origin == url_origin
            }
            _ => true, // default, unsafe-url, origin-when-cross-origin, strict-origin-when-cross-origin
        };

        if should_send {
            let ref_value = match referrer_policy {
                "origin" | "strict-origin" => {
                    // Send only origin (scheme + host)
                    referrer.split('/').take(3).collect::<Vec<_>>().join("/") + "/"
                }
                "origin-when-cross-origin" | "strict-origin-when-cross-origin" => {
                    let ref_origin = referrer.split('/').take(3).collect::<Vec<_>>().join("/");
                    let url_origin = url.split('/').take(3).collect::<Vec<_>>().join("/");
                    if ref_origin == url_origin {
                        referrer.clone() // Same origin - send full URL
                    } else {
                        ref_origin + "/" // Cross-origin - send only origin
                    }
                }
                _ => referrer.clone(), // unsafe-url, same-origin - send full URL
            };
            request = request.header("Referer", ref_value);
        }
    }

    match request.send() {
        Ok(response) => {
            let status = response.status().as_u16();
            let headers = response.headers().clone();
            let final_url = response.url().to_string();

            // Extract cookies from response
            let mut response_cookies = HashMap::new();
            for cookie_header in headers.get_all(SET_COOKIE) {
                if let Ok(cookie_str) = cookie_header.to_str() {
                    // Parse simple cookie format: name=value; ...
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

/// Initialize the std/http module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // fetch(url) or fetch(options) -> Result<Response, Error>
    // - fetch("https://...") - Simple GET request (like browser fetch with just URL)
    // - fetch(map { "url": "...", "method": "POST", ... }) - Full request with cookies/headers
    module.insert(
        "fetch".to_string(),
        Value::NativeFunction {
            name: "fetch".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(url) => http_get(url),
                Value::Map(opts) => http_request_with_cookies(opts),
                _ => Err(IntentError::TypeError(
                    "fetch() requires a URL string or options map".to_string(),
                )),
            },
        },
    );

    // post(url, body) -> Result<Response, Error> - HTTP POST request
    module.insert(
        "post".to_string(),
        Value::NativeFunction {
            name: "post".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(url), Value::String(body)) => http_post(url, body, None),
                _ => Err(IntentError::TypeError(
                    "post() requires URL string and body string".to_string(),
                )),
            },
        },
    );

    // put(url, body) -> Result<Response, Error> - HTTP PUT request
    module.insert(
        "put".to_string(),
        Value::NativeFunction {
            name: "put".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(url), Value::String(body)) => http_put(url, body),
                _ => Err(IntentError::TypeError(
                    "put() requires URL string and body string".to_string(),
                )),
            },
        },
    );

    // delete(url) -> Result<Response, Error> - HTTP DELETE request
    module.insert(
        "delete".to_string(),
        Value::NativeFunction {
            name: "delete".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(url) => http_delete(url),
                _ => Err(IntentError::TypeError(
                    "delete() requires a URL string".to_string(),
                )),
            },
        },
    );

    // patch(url, body) -> Result<Response, Error> - HTTP PATCH request
    module.insert(
        "patch".to_string(),
        Value::NativeFunction {
            name: "patch".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(url), Value::String(body)) => http_patch(url, body),
                _ => Err(IntentError::TypeError(
                    "patch() requires URL string and body string".to_string(),
                )),
            },
        },
    );

    // head(url) -> Result<Response, Error> - HTTP HEAD request
    module.insert(
        "head".to_string(),
        Value::NativeFunction {
            name: "head".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(url) => http_head(url),
                _ => Err(IntentError::TypeError(
                    "head() requires a URL string".to_string(),
                )),
            },
        },
    );

    // request(options) -> Result<Response, Error> - Full HTTP request with options
    module.insert(
        "request".to_string(),
        Value::NativeFunction {
            name: "request".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Map(opts) => http_request(opts),
                _ => Err(IntentError::TypeError(
                    "request() requires an options map".to_string(),
                )),
            },
        },
    );

    // get_json(url) -> Result<Value, Error> - GET request that parses JSON response
    module.insert(
        "get_json".to_string(),
        Value::NativeFunction {
            name: "get_json".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(url) => http_get_json(url),
                _ => Err(IntentError::TypeError(
                    "get_json() requires a URL string".to_string(),
                )),
            },
        },
    );

    // post_json(url, data) -> Result<Value, Error> - POST JSON data and parse response
    module.insert(
        "post_json".to_string(),
        Value::NativeFunction {
            name: "post_json".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(url), data) => http_post_json(url, data),
                _ => Err(IntentError::TypeError(
                    "post_json() requires URL string and data".to_string(),
                )),
            },
        },
    );

    // basic_auth(url, username, password) -> Result<Response, Error> - GET with Basic auth
    module.insert(
        "basic_auth".to_string(),
        Value::NativeFunction {
            name: "basic_auth".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(url), Value::String(username), Value::String(password)) => {
                    http_basic_auth(url, username, password)
                }
                _ => Err(IntentError::TypeError(
                    "basic_auth() requires URL, username, and password strings".to_string(),
                )),
            },
        },
    );

    // post_form(url, form_data) -> Result<Response, Error> - POST with form encoding
    module.insert(
        "post_form".to_string(),
        Value::NativeFunction {
            name: "post_form".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(url), Value::Map(form_data)) => http_post_form(url, form_data),
                _ => Err(IntentError::TypeError(
                    "post_form() requires URL string and form data map".to_string(),
                )),
            },
        },
    );

    // download(url, file_path) -> Result<{status, path, size}, Error> - Download file
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

    // upload(url, file_path, field_name) -> Result<Response, Error> - Upload file
    module.insert(
        "upload".to_string(),
        Value::NativeFunction {
            name: "upload".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::String(url), Value::String(file_path), Value::String(field_name)) => {
                    http_upload(url, file_path, field_name)
                }
                _ => Err(IntentError::TypeError(
                    "upload() requires URL, file path, and field name strings".to_string(),
                )),
            },
        },
    );

    module
}
