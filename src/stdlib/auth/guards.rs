use super::cookies::build_signed_session_cookie_with_max_age;
use super::sessions::{get_session_for_request, SessionAccessEffect};
use super::*;

fn normalize_protected_path(pattern: &str) -> String {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let normalized_input = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed.trim_start_matches('/'))
    };

    if normalized_input == "/" || normalized_input == "/*" {
        return normalized_input;
    }

    if let Some(base) = normalized_input.strip_suffix("/*") {
        let normalized = if base.len() > 1 {
            base.trim_end_matches('/')
        } else {
            base
        };
        return format!("{}/*", normalized);
    }

    if normalized_input.len() > 1 {
        normalized_input.trim_end_matches('/').to_string()
    } else {
        normalized_input
    }
}

fn dedupe_protected_paths(paths: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

fn with_protected_paths<T>(f: impl FnOnce(&[String]) -> T) -> T {
    let protected = AUTH_PROTECTED_PATHS.lock().unwrap();
    f(&protected)
}

pub(super) fn validate_auth_challenge_kind(kind: &str) -> std::result::Result<String, String> {
    if kind.is_empty() {
        return Err("[auth] begin_auth_challenge() kind must not be empty".to_string());
    }

    if is_safe_provider_name(kind) {
        Ok(kind.to_string())
    } else {
        Err("[auth] begin_auth_challenge() kind must use only ASCII letters, numbers, periods, underscores, or hyphens".to_string())
    }
}

pub(super) fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub(super) fn encode_url_path_segment(text: &str) -> String {
    let mut encoded = String::new();

    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{:02X}", byte));
        }
    }

    encoded
}

pub fn reset_protected_paths() {
    let mut protected = AUTH_PROTECTED_PATHS.lock().unwrap();
    protected.clear();
}

pub fn register_protected_paths(paths: &[String]) {
    let mut protected = AUTH_PROTECTED_PATHS.lock().unwrap();
    for path in paths {
        let normalized = normalize_protected_path(path);
        if !normalized.is_empty() {
            protected.push(normalized);
        }
    }
    dedupe_protected_paths(&mut protected);
}

pub fn get_protected_paths() -> Vec<String> {
    AUTH_PROTECTED_PATHS.lock().unwrap().clone()
}

#[cfg(test)]
pub(super) fn path_matches_protected_pattern(path: &str, pattern: &str) -> bool {
    let normalized_path = normalize_protected_path(path);
    let normalized_pattern = normalize_protected_path(pattern);

    path_matches_normalized_protected_pattern(&normalized_path, &normalized_pattern)
}

fn path_matches_normalized_protected_pattern(path: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }

    if pattern == "/*" {
        return true;
    }

    if let Some(base) = pattern.strip_suffix("/*") {
        if base.is_empty() || base == "/" {
            return true;
        }
        return path == base
            || path
                .strip_prefix(base)
                .map(|rest| rest.starts_with('/'))
                .unwrap_or(false);
    }

    path == pattern
}

fn is_auth_exempt_path(path: &str) -> bool {
    let normalized = normalize_protected_path(path);
    let prefix = get_auth_config()
        .map(|config| super::routes::auth_route_prefix(&config))
        .unwrap_or_else(|| "/auth".to_string());
    normalized == prefix || normalized.starts_with(&format!("{}/", prefix))
}

pub(super) fn request_path(request: &Value) -> String {
    if let Value::Map(req_map) = request {
        if let Some(Value::String(path)) = req_map.get("path") {
            return path.clone();
        }
    }
    "/".to_string()
}

pub(super) fn request_target(request: &Value) -> String {
    let path = request_path(request);
    let Value::Map(req_map) = request else {
        return path;
    };

    match req_map.get("query") {
        Some(Value::String(query)) if !query.is_empty() => {
            if query.starts_with('?') {
                format!("{}{}", path, query)
            } else {
                format!("{}?{}", path, query)
            }
        }
        Some(Value::Map(map)) if !map.is_empty() => {
            let mut pairs = Vec::new();
            for (key, value) in map {
                let encoded_key = encode_url_path_segment(key);
                let encoded_value = match value {
                    Value::String(s) => encode_url_path_segment(s),
                    Value::Int(i) => encode_url_path_segment(&i.to_string()),
                    Value::Float(f) => encode_url_path_segment(&f.to_string()),
                    Value::Bool(b) => encode_url_path_segment(&b.to_string()),
                    _ => continue,
                };
                pairs.push(format!("{}={}", encoded_key, encoded_value));
            }
            if pairs.is_empty() {
                path
            } else {
                format!("{}?{}", path, pairs.join("&"))
            }
        }
        _ => path,
    }
}

fn request_prefers_api(request: &Value) -> bool {
    let path = request_path(request);
    if path == "/api" || path.starts_with("/api/") {
        return true;
    }

    let Value::Map(req_map) = request else {
        return false;
    };
    let Some(Value::Map(headers)) = req_map.get("headers") else {
        return false;
    };

    if headers
        .get("accept")
        .and_then(|v| match v {
            Value::String(s) => Some(s.to_ascii_lowercase()),
            _ => None,
        })
        .map(|accept| accept.contains("application/json"))
        .unwrap_or(false)
    {
        return true;
    }

    if headers
        .get("content-type")
        .and_then(|v| match v {
            Value::String(s) => Some(s.to_ascii_lowercase()),
            _ => None,
        })
        .map(|content_type| content_type.contains("application/json"))
        .unwrap_or(false)
    {
        return true;
    }

    headers
        .get("x-requested-with")
        .and_then(|v| match v {
            Value::String(s) => Some(s.eq_ignore_ascii_case("xmlhttprequest")),
            _ => None,
        })
        .unwrap_or(false)
}

fn auth_required_response(request: &Value) -> Value {
    if request_prefers_api(request) {
        let mut headers = HashMap::new();
        headers.insert(
            "Content-Type".to_string(),
            Value::String("application/json".to_string()),
        );

        let mut response = HashMap::new();
        response.insert("status".to_string(), Value::Int(401));
        response.insert("headers".to_string(), Value::Map(headers));
        response.insert(
            "body".to_string(),
            Value::String(
                serde_json::json!({
                    "error": "unauthorized",
                    "message": "Authentication required"
                })
                .to_string(),
            ),
        );
        return Value::Map(response);
    }

    redirect_response("/auth", None)
}

fn path_requires_auth(path: &str) -> bool {
    if is_auth_exempt_path(path) {
        return false;
    }

    let normalized_path = normalize_protected_path(path);

    with_protected_paths(|patterns| {
        patterns
            .iter()
            .any(|pattern| path_matches_normalized_protected_pattern(&normalized_path, pattern))
    })
}

pub fn enforce_auth_for_request(
    request: &Value,
    force_auth: bool,
) -> std::result::Result<Option<String>, Value> {
    let path = request_path(request);

    if is_auth_exempt_path(&path) {
        return Ok(None);
    }

    if !force_auth && !path_requires_auth(&path) {
        return Ok(None);
    }

    let Some(_config) = get_auth_config() else {
        return Err(crate::stdlib::http_server::create_error_response(
            500,
            "Auth not initialized. Call enable_auth() before require_auth().",
        ));
    };

    if let Some(session_id) = get_session_id_from_request(request) {
        let (session, effect) = get_session_for_request(&session_id);
        if session.is_some() {
            let refreshed_cookie = match effect {
                SessionAccessEffect::ExpiryUpdated { expires_at } => {
                    let method = if let Value::Map(req_map) = request {
                        req_map.get("method").and_then(|v| match v {
                            Value::String(s) => Some(s.to_ascii_uppercase()),
                            _ => None,
                        })
                    } else {
                        None
                    }
                    .unwrap_or_else(|| "GET".to_string());

                    let safe_navigation =
                        (method == "GET" || method == "HEAD") && !request_prefers_api(request);

                    if !safe_navigation {
                        None
                    } else {
                        let now = chrono::Utc::now().timestamp();
                        get_auth_config().and_then(|config| {
                            build_signed_session_cookie_with_max_age(
                                &config,
                                &session_id,
                                expires_at - now,
                                None,
                            )
                            .ok()
                        })
                    }
                }
                SessionAccessEffect::TokensRefreshed {
                    expires_at,
                    refresh_rotated,
                } => {
                    let method = if let Value::Map(req_map) = request {
                        req_map.get("method").and_then(|v| match v {
                            Value::String(s) => Some(s.to_ascii_uppercase()),
                            _ => None,
                        })
                    } else {
                        None
                    }
                    .unwrap_or_else(|| "GET".to_string());

                    let safe_navigation =
                        (method == "GET" || method == "HEAD") && !request_prefers_api(request);

                    if !safe_navigation {
                        None
                    } else {
                        let now = chrono::Utc::now().timestamp();
                        let cookie = get_auth_config().and_then(|config| {
                            build_signed_session_cookie_with_max_age(
                                &config,
                                &session_id,
                                expires_at - now,
                                None,
                            )
                            .ok()
                        });

                        if refresh_rotated {
                            eprintln!(
                                "[auth] Refreshed session cookie after refresh-token rotation for session {}",
                                &session_id[..8]
                            );
                        }

                        cookie
                    }
                }
                SessionAccessEffect::Unchanged => None,
            };
            return Ok(refreshed_cookie);
        }
    }

    Err(auth_required_response(request))
}
