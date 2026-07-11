use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use base64::Engine;
use sha2::{Digest, Sha256};

use super::cookies::{normalize_cookie_same_site, validate_cookie_path};
use super::local::{
    consume_magic_link_record, discard_magic_link_record, issue_magic_link_record,
    local_identity_to_safe_value,
};
use super::request_helpers::security_signal_hash;
use super::storage::{
    create_manual_session, increment_auth_rate_limit_record, normalize_local_identifier,
};
use super::utils::{html_response, redirect_response};
use super::{
    get_auth_config, optional_string_option, persist_request_aware_manual_session,
    validate_http_request_arg, AuthConfig,
};

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static MISSING_MAGIC_LINK_CLIENT_ID_WARNED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone)]
struct MagicLinkFlowOptions {
    request_path: String,
    consume_path: String,
    base_url: String,
    success_url: String,
    failure_url: String,
    identifier_field: String,
    identifier_kind: String,
    link_ttl_seconds: i64,
    request_body_limit: usize,
    client_limit: i64,
    client_window_seconds: i64,
    identity_limit: i64,
    identity_window_seconds: i64,
    generic_response_floor_ms: u64,
    delivery_budget_hint_seconds: i64,
    trusted_client_ip_header: Option<String>,
    request_title: String,
    request_heading: String,
    request_copy: String,
    confirmation_title: String,
    confirmation_heading: String,
    confirmation_copy: String,
    eligible: Value,
    deliver: Value,
    authorize: Value,
    session_options: Option<HashMap<String, Value>>,
}

pub(crate) type MagicLinkFlowInvoke<'a> = dyn FnMut(&Value, Vec<Value>) -> Result<Value> + 'a;

fn value_is_callable(value: &Value) -> bool {
    matches!(value, Value::Function { .. } | Value::NativeFunction { .. })
}

fn required_callable_option(
    options: &HashMap<String, Value>,
    key: &str,
    function_name: &str,
) -> Result<Value> {
    match options.get(key) {
        Some(value) if value_is_callable(value) => Ok(value.clone()),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() {} must be a function, got {}",
            function_name,
            key,
            other.type_name()
        ))),
        None => Err(IntentError::type_error(format!(
            "[auth] {}() options.{} is required",
            function_name, key
        ))),
    }
}

fn int_option(
    options: &HashMap<String, Value>,
    key: &str,
    default: i64,
    function_name: &str,
) -> Result<i64> {
    match options.get(key) {
        Some(Value::Int(value)) => Ok(*value),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() {} must be an int, got {}",
            function_name,
            key,
            other.type_name()
        ))),
        None => Ok(default),
    }
}

fn positive_int_option(
    options: &HashMap<String, Value>,
    key: &str,
    default: i64,
    function_name: &str,
) -> Result<i64> {
    let value = int_option(options, key, default, function_name)?;
    if value <= 0 {
        return Err(IntentError::type_error(format!(
            "[auth] {}() {} must be > 0",
            function_name, key
        )));
    }
    Ok(value)
}

fn bool_option(
    options: &HashMap<String, Value>,
    key: &str,
    default: bool,
    function_name: &str,
) -> Result<bool> {
    match options.get(key) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() {} must be a bool, got {}",
            function_name,
            key,
            other.type_name()
        ))),
        None => Ok(default),
    }
}

fn string_option(
    options: &HashMap<String, Value>,
    key: &str,
    default: &str,
    function_name: &str,
) -> Result<String> {
    match options.get(key) {
        Some(Value::String(value)) => Ok(value.clone()),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() {} must be a string, got {}",
            function_name,
            key,
            other.type_name()
        ))),
        None => Ok(default.to_string()),
    }
}

fn optional_options_map(
    options: &HashMap<String, Value>,
    key: &str,
    function_name: &str,
) -> Result<Option<HashMap<String, Value>>> {
    match options.get(key) {
        Some(Value::Map(map)) => Ok(Some(map.clone())),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() {} must be a map, got {}",
            function_name,
            key,
            other.type_name()
        ))),
        None => Ok(None),
    }
}

fn validate_magic_link_flow_session_options(
    options: Option<HashMap<String, Value>>,
) -> Result<Option<HashMap<String, Value>>> {
    let Some(options) = options else {
        return Ok(None);
    };
    const KNOWN_OPTIONS: &[&str] = &[
        "session_ttl",
        "cookie_max_age",
        "cookie_path",
        "cookie_same_site",
        "cookie_secure",
        "cookie_http_only",
    ];
    if let Some(unknown) = options
        .keys()
        .find(|key| !KNOWN_OPTIONS.contains(&key.as_str()))
    {
        return Err(IntentError::type_error(format!(
            "[auth] magic_link_flow() unknown session option \"{}\"",
            unknown
        )));
    }
    for key in ["session_ttl", "cookie_max_age"] {
        match options.get(key) {
            Some(Value::Int(value)) if *value > 0 => {}
            Some(Value::Int(_)) => {
                return Err(IntentError::type_error(format!(
                    "[auth] magic_link_flow() session_options.{key} must be > 0"
                )))
            }
            Some(other) => {
                return Err(IntentError::type_error(format!(
                    "[auth] magic_link_flow() session_options.{key} must be an int, got {}",
                    other.type_name()
                )))
            }
            None => {}
        }
    }
    if let Some(value) = options.get("cookie_path") {
        let Value::String(path) = value else {
            return Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() session_options.cookie_path must be a string, got {}",
                value.type_name()
            )));
        };
        validate_cookie_path(path).map_err(IntentError::type_error)?;
    }
    if let Some(value) = options.get("cookie_same_site") {
        let Value::String(same_site) = value else {
            return Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() session_options.cookie_same_site must be a string, got {}",
                value.type_name()
            )));
        };
        normalize_cookie_same_site(same_site).map_err(IntentError::type_error)?;
    }
    for key in ["cookie_secure", "cookie_http_only"] {
        if let Some(value) = options.get(key) {
            if !matches!(value, Value::Bool(_)) {
                return Err(IntentError::type_error(format!(
                    "[auth] magic_link_flow() session_options.{key} must be a bool, got {}",
                    value.type_name()
                )));
            }
        }
    }
    Ok(Some(options))
}

fn parse_magic_link_flow_options(value: &Value) -> Result<MagicLinkFlowOptions> {
    let Value::Map(options) = value else {
        return Err(IntentError::type_error(format!(
            "[auth] magic_link_flow() options must be a map, got {}",
            value.type_name()
        )));
    };
    const KNOWN_OPTIONS: &[&str] = &[
        "request_path",
        "consume_path",
        "base_url",
        "success_url",
        "failure_url",
        "allow_external_redirects",
        "identifier_field",
        "identifier_kind",
        "ttl_seconds",
        "request_body_limit",
        "client_limit",
        "client_window_seconds",
        "identity_limit",
        "identity_window_seconds",
        "generic_response_floor_ms",
        "delivery_budget_hint_seconds",
        "trusted_client_ip_header",
        "request_title",
        "request_heading",
        "request_copy",
        "confirmation_title",
        "confirmation_heading",
        "confirmation_copy",
        "eligible",
        "deliver",
        "authorize",
        "session_options",
    ];
    if let Some(unknown) = options
        .keys()
        .find(|key| !KNOWN_OPTIONS.contains(&key.as_str()))
    {
        return Err(IntentError::type_error(format!(
            "[auth] magic_link_flow() unknown option \"{}\"",
            unknown
        )));
    }

    let allow_external_redirects = bool_option(
        options,
        "allow_external_redirects",
        false,
        "magic_link_flow",
    )?;
    let success_url = validate_magic_link_flow_redirect(
        &string_option(options, "success_url", "/", "magic_link_flow")?,
        allow_external_redirects,
    )?;
    let failure_url = validate_magic_link_flow_redirect(
        &string_option(options, "failure_url", "/login", "magic_link_flow")?,
        allow_external_redirects,
    )?;
    let request_body_limit = usize::try_from(positive_int_option(
        options,
        "request_body_limit",
        4096,
        "magic_link_flow",
    )?)
    .map_err(|_| {
        IntentError::type_error(
            "[auth] magic_link_flow() request_body_limit exceeds this platform's maximum"
                .to_string(),
        )
    })?;

    Ok(MagicLinkFlowOptions {
        request_path: validate_magic_link_flow_path(&string_option(
            options,
            "request_path",
            "/email-login",
            "magic_link_flow",
        )?)?,
        consume_path: validate_magic_link_flow_path(&string_option(
            options,
            "consume_path",
            "/email-login/consume",
            "magic_link_flow",
        )?)?,
        base_url: validate_magic_link_flow_base_url(
            &optional_string_option(options, "base_url", "magic_link_flow")?,
        )?,
        success_url,
        failure_url,
        identifier_field: string_option(options, "identifier_field", "email", "magic_link_flow")?,
        identifier_kind: string_option(options, "identifier_kind", "email", "magic_link_flow")?,
        link_ttl_seconds: positive_int_option(options, "ttl_seconds", 900, "magic_link_flow")?,
        request_body_limit,
        client_limit: positive_int_option(options, "client_limit", 10, "magic_link_flow")?,
        client_window_seconds: positive_int_option(
            options,
            "client_window_seconds",
            900,
            "magic_link_flow",
        )?,
        identity_limit: positive_int_option(options, "identity_limit", 3, "magic_link_flow")?,
        identity_window_seconds: positive_int_option(
            options,
            "identity_window_seconds",
            900,
            "magic_link_flow",
        )?,
        generic_response_floor_ms: positive_int_option(
            options,
            "generic_response_floor_ms",
            1200,
            "magic_link_flow",
        )? as u64,
        delivery_budget_hint_seconds: positive_int_option(
            options,
            "delivery_budget_hint_seconds",
            1,
            "magic_link_flow",
        )?,
        trusted_client_ip_header: optional_string_option(
            options,
            "trusted_client_ip_header",
            "magic_link_flow",
        )?,
        request_title: string_option(options, "request_title", "Email sign-in", "magic_link_flow")?,
        request_heading: string_option(
            options,
            "request_heading",
            "Email sign-in",
            "magic_link_flow",
        )?,
        request_copy: string_option(
            options,
            "request_copy",
            "Enter your email address and, if the account can use email sign-in, we will send a link.",
            "magic_link_flow",
        )?,
        confirmation_title: string_option(
            options,
            "confirmation_title",
            "Confirm sign-in",
            "magic_link_flow",
        )?,
        confirmation_heading: string_option(
            options,
            "confirmation_heading",
            "Confirm sign-in",
            "magic_link_flow",
        )?,
        confirmation_copy: string_option(
            options,
            "confirmation_copy",
            "Confirm this sign-in on the device where you requested the link.",
            "magic_link_flow",
        )?,
        eligible: required_callable_option(options, "eligible", "magic_link_flow")?,
        deliver: required_callable_option(options, "deliver", "magic_link_flow")?,
        authorize: required_callable_option(options, "authorize", "magic_link_flow")?,
        session_options: validate_magic_link_flow_session_options(optional_options_map(
            options,
            "session_options",
            "magic_link_flow",
        )?)?,
    })
}

fn validate_magic_link_flow_path(path: &str) -> Result<String> {
    if is_safe_local_redirect(path) {
        Ok(path.to_string())
    } else {
        Err(IntentError::type_error(
            "[auth] magic_link_flow() paths must be local absolute paths".to_string(),
        ))
    }
}

fn validate_magic_link_flow_redirect(url: &str, allow_external: bool) -> Result<String> {
    if is_safe_local_redirect(url) || (allow_external && is_http_url(url)) {
        Ok(url.to_string())
    } else {
        Err(IntentError::type_error(
            "[auth] magic_link_flow() redirects must be local absolute paths unless allow_external_redirects is true".to_string(),
        ))
    }
}

fn validate_magic_link_flow_base_url(configured: &Option<String>) -> Result<String> {
    let raw = match configured {
        Some(value) => value.clone(),
        None => std::env::var("SITE_URL").map_err(|_| {
            IntentError::type_error(
                "[auth] magic_link_flow() base_url is required unless SITE_URL is configured"
                    .to_string(),
            )
        })?,
    };
    normalize_magic_link_flow_base_url(&raw).ok_or_else(|| {
        IntentError::type_error(
            "[auth] magic_link_flow() base_url must be a trusted http(s) origin".to_string(),
        )
    })
}

fn normalize_magic_link_flow_base_url(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.chars().any(|ch| ch.is_control()) || trimmed.contains('\\') {
        return None;
    }
    let (scheme, host_port) = if let Some(rest) = trimmed.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        ("http", rest)
    } else {
        return None;
    };
    if host_port.is_empty()
        || host_port.contains('/')
        || host_port.contains('?')
        || host_port.contains('#')
        || host_port.contains('@')
    {
        return None;
    }
    if !host_port
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']'))
    {
        return None;
    }
    if scheme == "http" && !is_local_development_host(host_port) {
        return None;
    }
    Some(format!("{scheme}://{host_port}"))
}

fn is_local_development_host(host_port: &str) -> bool {
    let host = host_port
        .strip_prefix('[')
        .and_then(|value| value.split_once(']').map(|(host, _)| host))
        .unwrap_or_else(|| host_port.split(':').next().unwrap_or(host_port));
    host.eq_ignore_ascii_case("localhost")
        || host == "127.0.0.1"
        || host == "0.0.0.0"
        || host == "::1"
}

fn is_safe_local_redirect(value: &str) -> bool {
    value.starts_with('/')
        && !value.starts_with("//")
        && !value.chars().any(|ch| ch.is_control())
        && !value.contains('\\')
}

fn is_http_url(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://"))
        && !value.chars().any(|ch| ch.is_control())
}

fn request_method_path(req: &Value) -> Result<(String, String)> {
    validate_http_request_arg("magic_link_flow", req)?;
    let Value::Map(map) = req else {
        unreachable!("validate_http_request_arg already checked request map")
    };
    let method = match map.get("method") {
        Some(Value::String(method)) => method.to_ascii_uppercase(),
        _ => unreachable!("validate_http_request_arg already checked request method"),
    };
    let path = match map.get("path") {
        Some(Value::String(path)) => path.clone(),
        _ => unreachable!("validate_http_request_arg already checked request path"),
    };
    Ok((method, path))
}

fn bounded_form(req: &Value, limit: usize) -> Result<HashMap<String, String>> {
    let Value::Map(map) = req else {
        return Err(IntentError::type_error(
            "[auth] magic_link_flow() request must be an HTTP request map".to_string(),
        ));
    };
    let body = match map.get("body") {
        Some(Value::String(body)) => body,
        Some(other) => {
            return Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() request.body must be a string, got {}",
                other.type_name()
            )))
        }
        None => "",
    };
    if body.len() > limit {
        return Err(IntentError::runtime_error(
            "[auth] magic_link_flow() request body too large".to_string(),
        ));
    }

    let mut form = HashMap::new();
    for pair in body.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = urlencoding::decode(&key.replace('+', " "))
            .unwrap_or_else(|_| key.into())
            .to_string();
        let value = urlencoding::decode(&value.replace('+', " "))
            .unwrap_or_else(|_| value.into())
            .to_string();
        form.insert(key, value);
    }
    Ok(form)
}

fn html_escape_auth(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn magic_link_html_response(body: &str, script_hash: Option<&str>) -> Value {
    let mut response = html_response(body);
    if let Value::Map(response_map) = &mut response {
        if let Some(Value::Map(headers)) = response_map.get_mut("headers") {
            let script_policy = script_hash
                .map(|hash| format!("'sha256-{hash}'"))
                .unwrap_or_else(|| "'none'".to_string());
            headers.insert(
                "Content-Security-Policy".to_string(),
                Value::String(format!(
                    "default-src 'none'; script-src {script_policy}; form-action 'self'; base-uri 'none'; frame-ancestors 'none'; object-src 'none'"
                )),
            );
            headers.insert(
                "Referrer-Policy".to_string(),
                Value::String("no-referrer".to_string()),
            );
            headers.insert(
                "X-Frame-Options".to_string(),
                Value::String("DENY".to_string()),
            );
            headers.insert(
                "X-Content-Type-Options".to_string(),
                Value::String("nosniff".to_string()),
            );
            headers.insert(
                "Permissions-Policy".to_string(),
                Value::String("camera=(), microphone=(), geolocation=()".to_string()),
            );
        }
    }
    response
}

fn magic_link_request_page(options: &MagicLinkFlowOptions) -> Value {
    magic_link_html_response(
        &format!(
            r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title></head>
<body><main><h1>{}</h1><p>{}</p>
<form method="post" action="{}">
<label>Email <input type="email" name="{}" autocomplete="email" required></label>
<button type="submit">Send link</button>
</form></main></body></html>"#,
            html_escape_auth(&options.request_title),
            html_escape_auth(&options.request_heading),
            html_escape_auth(&options.request_copy),
            html_escape_auth(&options.request_path),
            html_escape_auth(&options.identifier_field)
        ),
        None,
    )
}

const MAGIC_LINK_CONFIRMATION_SCRIPT: &str = r##"(function(){
  var status = document.getElementById("magic-link-status");
  var input = document.getElementById("magic-link-token");
  var button = document.getElementById("magic-link-confirm");
  var raw = window.location.hash || "";
  history.replaceState(null, document.title, window.location.pathname + window.location.search);
  var token = "";
  try {
    if (raw.indexOf("#token=") === 0) token = decodeURIComponent(raw.slice(7));
  } catch (_) {
    token = "";
  }
  if (/^[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}$/.test(token)) {
    input.value = token;
    button.disabled = false;
  } else {
    status.textContent = "This sign-in link is invalid or expired.";
  }
})();"##;

fn magic_link_confirmation_page(options: &MagicLinkFlowOptions) -> Value {
    let script_hash = base64::engine::general_purpose::STANDARD
        .encode(Sha256::digest(MAGIC_LINK_CONFIRMATION_SCRIPT.as_bytes()));
    magic_link_html_response(
        &format!(
            r#"<!doctype html>
<html lang="en">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{}</title></head>
<body><main><h1>{}</h1><p>{}</p>
<form method="post" action="{}" id="magic-link-confirm-form">
<input type="hidden" name="token" id="magic-link-token">
<button type="submit" id="magic-link-confirm" disabled>Confirm sign-in</button>
</form><p id="magic-link-status"></p></main>
<script>{}</script></body></html>"#,
            html_escape_auth(&options.confirmation_title),
            html_escape_auth(&options.confirmation_heading),
            html_escape_auth(&options.confirmation_copy),
            html_escape_auth(&options.consume_path),
            MAGIC_LINK_CONFIRMATION_SCRIPT,
        ),
        Some(&script_hash),
    )
}

fn generic_magic_link_response_started(started: Instant, floor_ms: u64) -> Value {
    let floor = Duration::from_millis(floor_ms);
    let elapsed = started.elapsed();
    if floor > elapsed {
        std::thread::sleep(floor - elapsed);
    }
    magic_link_html_response(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Email sign-in</title></head><body><main><h1>Check your email</h1><p>If an account can use email sign-in, a link will arrive shortly.</p></main></body></html>",
        None,
    )
}

pub(super) fn auth_rate_limit_key(scope: &str, stable_id: &str) -> String {
    security_signal_hash(&format!("magic_link_flow:{scope}:{stable_id}"))
}

fn check_magic_link_flow_rate_limit(
    scope: &str,
    key_hash: &str,
    limit: i64,
    window_seconds: i64,
) -> std::result::Result<bool, String> {
    let now = chrono::Utc::now().timestamp();
    let bucket = increment_auth_rate_limit_record(scope, key_hash, window_seconds, now)?;
    Ok(bucket.count <= limit)
}

fn magic_link_flow_client_id(req: &Value, options: &MagicLinkFlowOptions) -> String {
    if let Some(header) = &options.trusted_client_ip_header {
        if let Value::Map(req_map) = req {
            if let Some(Value::Map(headers)) = req_map.get("headers") {
                if let Some(Value::String(value)) = headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(header))
                    .map(|(_, value)| value)
                {
                    let first = value.split(',').next().unwrap_or(value).trim();
                    if !first.is_empty() {
                        return first.to_string();
                    }
                }
            }
        }
    }
    if let Value::Map(req_map) = req {
        if let Some(Value::String(ip)) = req_map.get("peer_ip") {
            let ip = ip.trim();
            if !ip.is_empty() && ip != "unknown" {
                return ip.to_string();
            }
        }
    }
    if !MISSING_MAGIC_LINK_CLIENT_ID_WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "[auth] magic_link_flow() could not determine a client address; requests share a fail-closed rate-limit bucket. Configure a trusted client-IP header only when ingress strips spoofed values."
        );
    }
    "unknown".to_string()
}

fn eligible_result_allows(value: Value) -> Result<bool> {
    match value {
        Value::Bool(value) => Ok(value),
        Value::Map(_) => Ok(true),
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Result" && variant == "Ok" => match values.into_iter().next() {
            Some(Value::Bool(value)) => Ok(value),
            Some(Value::Map(_)) => Ok(true),
            Some(other) => Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() eligible Ok payload must be bool or map, got {}",
                other.type_name()
            ))),
            None => Ok(false),
        },
        Value::EnumValue {
            enum_name, variant, ..
        } if enum_name == "Result" && variant == "Err" => Ok(false),
        other => Err(IntentError::type_error(format!(
            "[auth] magic_link_flow() eligible must return bool, map, or Result, got {}",
            other.type_name()
        ))),
    }
}

fn delivery_result_is_ok(value: Value) -> bool {
    matches!(
        value,
        Value::EnumValue {
            enum_name,
            variant,
            ..
        } if enum_name == "Result" && variant == "Ok"
    )
}

fn result_map_or_error(
    value: Value,
    function_name: &str,
) -> Result<std::result::Result<HashMap<String, Value>, String>> {
    match value {
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Result" && variant == "Ok" => match values.into_iter().next() {
            Some(Value::Map(map)) => Ok(Ok(map)),
            Some(other) => Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() {function_name} Ok payload must be a map, got {}",
                other.type_name()
            ))),
            None => Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() {function_name} Ok payload must be a map"
            ))),
        },
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Result" && variant == "Err" => {
            let message = values
                .first()
                .and_then(|value| match value {
                    Value::String(value) => Some(value.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "rejected".to_string());
            Ok(Err(message))
        }
        other => Err(IntentError::type_error(format!(
            "[auth] magic_link_flow() {function_name} must return Result<Map, String>, got {}",
            other.type_name()
        ))),
    }
}

pub(super) fn token_from_issued(issued: &HashMap<String, Value>) -> Option<String> {
    match issued.get("token") {
        Some(Value::String(token)) if is_magic_link_token_shape(token) => Some(token.clone()),
        _ => None,
    }
}

fn is_magic_link_token_shape(token: &str) -> bool {
    let Some((selector, verifier)) = token.split_once('.') else {
        return false;
    };
    selector.len() == 22
        && verifier.len() == 43
        && selector
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
        && verifier
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

pub(crate) fn run_magic_link_flow(
    args: &[Value],
    invoke: &mut MagicLinkFlowInvoke<'_>,
) -> Result<Value> {
    if args.len() != 2 {
        return Err(IntentError::type_error(
            "[auth] magic_link_flow() requires request and options".to_string(),
        ));
    }
    let config = get_auth_config().ok_or_else(|| {
        IntentError::runtime_error(
            "[auth] Auth not initialized. Call enable_auth() before magic_link_flow().".to_string(),
        )
    })?;
    let req = &args[0];
    let options = parse_magic_link_flow_options(&args[1])?;
    let (method, path) = request_method_path(req)?;

    if method == "GET" && path == options.request_path {
        return Ok(magic_link_request_page(&options));
    }
    if method == "GET" && path == options.consume_path {
        return Ok(magic_link_confirmation_page(&options));
    }
    if method == "POST" && path == options.request_path {
        return run_magic_link_request_flow(req, &options, invoke);
    }
    if method == "POST" && path == options.consume_path {
        return run_magic_link_consume_flow(req, &options, &config, invoke);
    }

    Ok(redirect_response(&options.failure_url, None))
}

fn run_magic_link_request_flow(
    req: &Value,
    options: &MagicLinkFlowOptions,
    invoke: &mut MagicLinkFlowInvoke<'_>,
) -> Result<Value> {
    let started = Instant::now();

    let client_id = magic_link_flow_client_id(req, options);
    let client_key = auth_rate_limit_key("client", &client_id);
    let client_allowed = match check_magic_link_flow_rate_limit(
        "client",
        &client_key,
        options.client_limit,
        options.client_window_seconds,
    ) {
        Ok(allowed) => allowed,
        Err(_) => {
            eprintln!("[auth] magic-link client rate-limit backend unavailable");
            return Ok(generic_magic_link_response_started(
                started,
                options.generic_response_floor_ms,
            ));
        }
    };
    if !client_allowed {
        return Ok(generic_magic_link_response_started(
            started,
            options.generic_response_floor_ms,
        ));
    }

    let form = match bounded_form(req, options.request_body_limit) {
        Ok(form) => form,
        Err(_) => {
            return Ok(generic_magic_link_response_started(
                started,
                options.generic_response_floor_ms,
            ))
        }
    };

    let identifier_raw = form
        .get(&options.identifier_field)
        .map(|value| value.trim())
        .unwrap_or("");
    let identifier_normalized =
        match normalize_local_identifier(&options.identifier_kind, identifier_raw) {
            Ok(identifier) => identifier,
            Err(_) => {
                return Ok(generic_magic_link_response_started(
                    started,
                    options.generic_response_floor_ms,
                ))
            }
        };

    let eligible = match invoke(
        &options.eligible,
        vec![Value::String(identifier_normalized.clone())],
    )
    .and_then(eligible_result_allows)
    {
        Ok(eligible) => eligible,
        Err(_) => {
            eprintln!("[auth] magic-link eligibility callback failed");
            return Ok(generic_magic_link_response_started(
                started,
                options.generic_response_floor_ms,
            ));
        }
    };
    if !eligible {
        return Ok(generic_magic_link_response_started(
            started,
            options.generic_response_floor_ms,
        ));
    }

    let identity_key = auth_rate_limit_key("identity", &identifier_normalized);
    let identity_allowed = match check_magic_link_flow_rate_limit(
        "identity",
        &identity_key,
        options.identity_limit,
        options.identity_window_seconds,
    ) {
        Ok(allowed) => allowed,
        Err(_) => {
            eprintln!("[auth] magic-link identity rate-limit backend unavailable");
            return Ok(generic_magic_link_response_started(
                started,
                options.generic_response_floor_ms,
            ));
        }
    };
    if !identity_allowed {
        return Ok(generic_magic_link_response_started(
            started,
            options.generic_response_floor_ms,
        ));
    }

    let issued = match issue_magic_link_record(
        &options.identifier_kind,
        &identifier_normalized,
        Some(options.link_ttl_seconds),
    ) {
        Ok(issued) => issued,
        Err(_) => {
            return Ok(generic_magic_link_response_started(
                started,
                options.generic_response_floor_ms,
            ))
        }
    };
    let Some(token) = token_from_issued(&issued) else {
        return Ok(generic_magic_link_response_started(
            started,
            options.generic_response_floor_ms,
        ));
    };

    let url = format!(
        "{}{}#token={}",
        options.base_url,
        options.consume_path,
        urlencoding::encode(&token)
    );
    let delivery = invoke(
        &options.deliver,
        vec![Value::Map(HashMap::from([
            (
                "to".to_string(),
                Value::String(identifier_normalized.clone()),
            ),
            ("url".to_string(), Value::String(url)),
            (
                "expires_in".to_string(),
                Value::Int(options.link_ttl_seconds.min(3600).max(0)),
            ),
            (
                "budget_hint_seconds".to_string(),
                Value::Int(options.delivery_budget_hint_seconds),
            ),
        ]))],
    );
    if delivery
        .as_ref()
        .map_or(true, |value| !delivery_result_is_ok(value.clone()))
    {
        if discard_magic_link_record(&token).is_err() {
            eprintln!("[auth] magic-link delivery cleanup failed");
        }
    }

    Ok(generic_magic_link_response_started(
        started,
        options.generic_response_floor_ms,
    ))
}

fn run_magic_link_consume_flow(
    req: &Value,
    options: &MagicLinkFlowOptions,
    config: &AuthConfig,
    invoke: &mut MagicLinkFlowInvoke<'_>,
) -> Result<Value> {
    let form = match bounded_form(req, options.request_body_limit) {
        Ok(form) => form,
        Err(_) => return Ok(redirect_response(&options.failure_url, None)),
    };
    let token = form.get("token").map(|value| value.trim()).unwrap_or("");
    if !is_magic_link_token_shape(token) {
        return Ok(redirect_response(&options.failure_url, None));
    }

    let identity = match consume_magic_link_record(token) {
        Ok(identity) => identity,
        Err(_) => return Ok(redirect_response(&options.failure_url, None)),
    };
    let identity_value = match local_identity_to_safe_value(&identity) {
        Ok(identity) => identity,
        Err(_) => {
            eprintln!("[auth] magic-link identity conversion failed");
            return Ok(redirect_response(&options.failure_url, None));
        }
    };
    let authorized = match invoke(&options.authorize, vec![identity_value])
        .and_then(|value| result_map_or_error(value, "authorize"))
    {
        Ok(authorized) => authorized,
        Err(_) => {
            eprintln!("[auth] magic-link authorization callback failed");
            return Ok(redirect_response(&options.failure_url, None));
        }
    };
    let mut session_spec = match authorized {
        Ok(session_spec) => session_spec,
        Err(_) => return Ok(redirect_response(&options.failure_url, None)),
    };
    session_spec.insert("provider".to_string(), Value::String("local".to_string()));
    session_spec.insert("subject_id".to_string(), Value::String(identity.id.clone()));
    if identity.identifier_kind == "email" {
        session_spec.insert(
            "email".to_string(),
            Value::String(identity.identifier_normalized.clone()),
        );
    } else {
        session_spec.remove("email");
    }

    let session_ttl = match options
        .session_options
        .as_ref()
        .and_then(|map| map.get("session_ttl"))
    {
        Some(Value::Int(value)) if *value > 0 => *value,
        Some(Value::Int(_)) => {
            return Err(IntentError::type_error(
                "[auth] magic_link_flow() session_options.session_ttl must be > 0".to_string(),
            ))
        }
        Some(other) => {
            return Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() session_options.session_ttl must be an int, got {}",
                other.type_name()
            )))
        }
        None => config.session_ttl,
    };
    let effective_session_ttl = session_ttl.min(config.max_session_ttl.unwrap_or(session_ttl));
    let session = match create_manual_session(&session_spec, effective_session_ttl) {
        Ok(session) => session,
        Err(_) => {
            eprintln!("[auth] magic-link session specification rejected");
            return Ok(redirect_response(&options.failure_url, None));
        }
    };
    let session_options = match bounded_magic_link_flow_session_options(
        options.session_options.as_ref(),
        effective_session_ttl,
    ) {
        Ok(options) => options,
        Err(_) => {
            eprintln!("[auth] magic-link session options rejected");
            return Ok(redirect_response(&options.failure_url, None));
        }
    };
    match persist_request_aware_manual_session(
        &redirect_response(&options.success_url, None),
        req,
        session,
        Some(&session_options),
        config,
    ) {
        Ok(response) => Ok(response),
        Err(_) => {
            eprintln!("[auth] magic-link session persistence failed");
            Ok(redirect_response(&options.failure_url, None))
        }
    }
}

fn bounded_magic_link_flow_session_options(
    options: Option<&HashMap<String, Value>>,
    effective_session_ttl: i64,
) -> Result<HashMap<String, Value>> {
    let mut bounded = options.cloned().unwrap_or_default();
    let max_age = match bounded.get("cookie_max_age") {
        Some(Value::Int(value)) if *value > 0 => (*value).min(effective_session_ttl),
        Some(Value::Int(_)) => {
            return Err(IntentError::type_error(
                "[auth] magic_link_flow() session_options.cookie_max_age must be > 0".to_string(),
            ))
        }
        Some(other) => {
            return Err(IntentError::type_error(format!(
                "[auth] magic_link_flow() session_options.cookie_max_age must be an int, got {}",
                other.type_name()
            )))
        }
        None => effective_session_ttl,
    };
    bounded.insert("cookie_max_age".to_string(), Value::Int(max_age));
    Ok(bounded)
}
