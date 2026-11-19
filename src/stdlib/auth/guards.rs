use super::cookies::build_signed_session_cookie_with_max_age;
use super::sessions::{get_session_for_request, SessionAccessEffect};
use super::*;
use crate::stdlib::http_server::canonical_route_path;
use std::cell::Cell;

thread_local! {
    /// Whether the request this thread is serving was routed to one of
    /// std/auth's public handlers. Only [`RouteDispatch`] changes it.
    static SERVING_PUBLIC_AUTH_HANDLER: Cell<bool> = const { Cell::new(false) };
}

/// Records, for one request, whether the router selected one of std/auth's
/// public handlers (`super::routes::PUBLIC_AUTH_HANDLERS`). The interpreter
/// holds it while middleware and the handler run, so the guard can exempt
/// std/auth's own routes without exempting an app handler that shares their
/// prefix. Dropping it restores the previous state.
#[must_use = "the selected handler is recorded only while this value is alive"]
pub struct RouteDispatch {
    previous: bool,
}

impl RouteDispatch {
    pub fn enter(handler: &Value) -> Self {
        let public = matches!(
            handler,
            Value::NativeFunction { name, .. }
                if super::routes::PUBLIC_AUTH_HANDLERS.contains(&name.as_str())
        );
        Self {
            previous: SERVING_PUBLIC_AUTH_HANDLER.with(|serving| serving.replace(public)),
        }
    }
}

impl Drop for RouteDispatch {
    fn drop(&mut self) {
        SERVING_PUBLIC_AUTH_HANDLER.with(|serving| serving.set(self.previous));
    }
}

/// A protected path pattern in the router's canonical form (see
/// `canonical_route_path`), keeping a trailing `/*` subtree marker.
fn normalize_protected_path(pattern: &str) -> String {
    let trimmed = pattern.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let rooted = format!("/{trimmed}");
    match rooted.strip_suffix("/*") {
        Some(base) => match canonical_route_path(base).as_str() {
            "/" => "/*".to_string(),
            base => format!("{base}/*"),
        },
        None => canonical_route_path(&rooted),
    }
}

fn dedupe_protected_paths(paths: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    paths.retain(|path| seen.insert(path.clone()));
}

/// The protection rules every worker enforces: `enable_auth()`'s
/// `protected_paths` and `require_auth(path)` patterns, normalized.
struct ProtectedPaths {
    patterns: Vec<String>,
    /// Live [`ServingScope`]s.
    serving: usize,
}

static PROTECTED_PATHS: std::sync::Mutex<ProtectedPaths> = std::sync::Mutex::new(ProtectedPaths {
    patterns: Vec::new(),
    serving: 0,
});

impl ProtectedPaths {
    fn add(&mut self, paths: &[String]) {
        for path in paths {
            let normalized = normalize_protected_path(path);
            if !normalized.is_empty() {
                self.patterns.push(normalized);
            }
        }
        dedupe_protected_paths(&mut self.patterns);
    }
}

/// Held by a server for as long as it may serve requests. Meanwhile setup can
/// run again (secondary workers start or restart, a development reload runs)
/// while other workers serve, so protection rules are only added; see
/// [`replace_protected_paths_unless_serving`].
#[must_use = "protection rules are kept only while the scope is alive"]
pub struct ServingScope(());

impl ServingScope {
    pub fn start() -> Self {
        PROTECTED_PATHS.lock().unwrap().serving += 1;
        Self(())
    }
}

impl Drop for ServingScope {
    fn drop(&mut self) {
        // May run while unwinding, where a second panic would abort.
        let mut protected = PROTECTED_PATHS
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        protected.serving -= 1;
    }
}

fn with_protected_paths<T>(f: impl FnOnce(&[String]) -> T) -> T {
    let protected = PROTECTED_PATHS.lock().unwrap();
    f(&protected.patterns)
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

/// Remove every protection rule, whether or not a server is serving.
pub fn reset_protected_paths() {
    PROTECTED_PATHS.lock().unwrap().patterns.clear();
}

pub fn register_protected_paths(paths: &[String]) {
    PROTECTED_PATHS.lock().unwrap().add(paths);
}

/// `enable_auth()`'s `protected_paths`. Before any server serves they replace
/// the rules. While one serves they are added to the rules instead: the setup
/// running again re-registers its `require_auth(path)` rules only when it
/// reaches them, other workers serve meanwhile, and a worker that has not
/// reloaded still runs code that relies on the old rules. Removing a rule
/// takes a restart.
pub(super) fn replace_protected_paths_unless_serving(paths: &[String]) {
    let mut protected = PROTECTED_PATHS.lock().unwrap();
    if protected.serving == 0 {
        protected.patterns.clear();
    }
    protected.add(paths);
}

pub fn get_protected_paths() -> Vec<String> {
    PROTECTED_PATHS.lock().unwrap().patterns.clone()
}

#[cfg(test)]
pub(super) fn path_matches_protected_pattern(path: &str, pattern: &str) -> bool {
    let normalized_path = canonical_route_path(path);
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

/// std/auth's public routes (login page, OAuth start and callback, logout,
/// health) need no session. The exemption follows the handler the router
/// selected, so an app handler under the same prefix keeps the protection
/// the app gave it with `require_auth()` or a protected path.
fn is_auth_exempt(path: &str) -> bool {
    SERVING_PUBLIC_AUTH_HANDLER.with(Cell::get) && is_under_auth_prefix(path)
}

fn is_under_auth_prefix(path: &str) -> bool {
    let prefix = get_auth_config()
        .map(|config| super::routes::auth_route_prefix(&config))
        .unwrap_or_else(|| "/auth".to_string());
    path == prefix || path.starts_with(&format!("{}/", prefix))
}

/// The request path in the canonical form the router matched (repeated and
/// trailing slashes removed). Auth decisions and redirects use only this, so a
/// different spelling of a protected route cannot miss its rule.
pub(super) fn request_path(request: &Value) -> String {
    if let Value::Map(req_map) = request {
        if let Some(Value::String(path)) = req_map.get("path") {
            return canonical_route_path(path);
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

    let login_path = get_auth_config()
        .map(|config| super::routes::auth_route_path(&config, ""))
        .unwrap_or_else(|| "/auth".to_string());
    redirect_response(&login_path, None)
}

/// `path` is canonical (see `request_path`), like the stored patterns.
fn path_requires_auth(path: &str) -> bool {
    if is_auth_exempt(path) {
        return false;
    }

    with_protected_paths(|patterns| {
        patterns
            .iter()
            .any(|pattern| path_matches_normalized_protected_pattern(path, pattern))
    })
}

/// `Err` carries the ready-made denial response, boxed to keep the result small.
pub fn enforce_auth_for_request(
    request: &Value,
    force_auth: bool,
) -> std::result::Result<Option<String>, Box<Value>> {
    let path = request_path(request);

    if is_auth_exempt(&path) {
        return Ok(None);
    }

    if !force_auth && !path_requires_auth(&path) {
        return Ok(None);
    }

    let Some(_config) = get_auth_config() else {
        return Err(Box::new(crate::stdlib::http_server::create_error_response(
            500,
            "Auth not initialized. Call enable_auth() before require_auth().",
        )));
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

    Err(Box::new(auth_required_response(request)))
}
