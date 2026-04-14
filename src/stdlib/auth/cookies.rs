use super::{sign_session_id, AuthConfig};
use crate::interpreter::Value;
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(super) struct AuthCookieSettings {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) same_site: String,
    pub(super) secure: bool,
    pub(super) http_only: bool,
    pub(super) max_age: i64,
}

pub fn default_auth_cookie_secure_env() -> bool {
    if let Ok(env) = std::env::var("NTNT_ENV") {
        if env.eq_ignore_ascii_case("development") || env.eq_ignore_ascii_case("dev") {
            return false;
        }
    }

    if let Ok(site_url) = std::env::var("SITE_URL") {
        let lower = site_url.to_ascii_lowercase();
        if lower.starts_with("http://localhost")
            || lower.starts_with("http://127.0.0.1")
            || lower.starts_with("http://0.0.0.0")
        {
            return false;
        }
    }

    true
}

pub(super) fn auth_challenge_cookie_name(
    config: &AuthConfig,
) -> std::result::Result<String, String> {
    validate_cookie_name(&format!("{}_challenge", config.cookie_name))
}

fn default_auth_cookie_max_age(config: &AuthConfig) -> i64 {
    if config.store_tokens && config.refresh_ttl > config.session_ttl {
        config.refresh_ttl
    } else {
        config.session_ttl
    }
}

fn validate_cookie_name(name: &str) -> std::result::Result<String, String> {
    if name.is_empty() {
        return Err("[auth] cookie_name must not be empty".to_string());
    }

    if name.chars().all(|ch| {
        ch.is_ascii_alphanumeric()
            || matches!(
                ch,
                '!' | '#'
                    | '$'
                    | '%'
                    | '&'
                    | '\''
                    | '*'
                    | '+'
                    | '-'
                    | '.'
                    | '^'
                    | '_'
                    | '`'
                    | '|'
                    | '~'
            )
    }) {
        Ok(name.to_string())
    } else {
        Err("[auth] cookie_name contains invalid characters".to_string())
    }
}

fn validate_cookie_path(path: &str) -> std::result::Result<String, String> {
    if path.is_empty() {
        return Err("[auth] cookie_path must not be empty".to_string());
    }
    if !path.starts_with('/') {
        return Err("[auth] cookie_path must start with /".to_string());
    }
    if path.chars().any(|ch| ch.is_control() || ch == ';') {
        return Err("[auth] cookie_path contains invalid characters".to_string());
    }

    Ok(path.to_string())
}

fn normalize_cookie_same_site(value: &str) -> std::result::Result<String, String> {
    match value.to_ascii_lowercase().as_str() {
        "lax" => Ok("Lax".to_string()),
        "strict" => Ok("Strict".to_string()),
        "none" => Ok("None".to_string()),
        _ => Err("[auth] cookie_same_site must be one of: Lax, Strict, None".to_string()),
    }
}

fn auth_cookie_settings(
    config: &AuthConfig,
    max_age: i64,
    overrides: Option<&HashMap<String, Value>>,
) -> std::result::Result<AuthCookieSettings, String> {
    if overrides.and_then(|m| m.get("cookie_name")).is_some() {
        return Err(
            "[auth] cookie_name override is not supported; configure it via enable_auth()"
                .to_string(),
        );
    }

    let get_string = |key: &str| -> std::result::Result<Option<String>, String> {
        match overrides.and_then(|m| m.get(key)) {
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(other) => Err(format!(
                "[auth] {} must be a string, got {}",
                key,
                other.type_name()
            )),
            None => Ok(None),
        }
    };

    let get_bool = |key: &str| -> std::result::Result<Option<bool>, String> {
        match overrides.and_then(|m| m.get(key)) {
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(other) => Err(format!(
                "[auth] {} must be a bool, got {}",
                key,
                other.type_name()
            )),
            None => Ok(None),
        }
    };

    let cookie_max_age = match overrides.and_then(|m| m.get("cookie_max_age")) {
        Some(Value::Int(i)) => *i,
        Some(other) => {
            return Err(format!(
                "[auth] cookie_max_age must be an int, got {}",
                other.type_name()
            ))
        }
        None => max_age,
    };

    Ok(AuthCookieSettings {
        name: validate_cookie_name(&config.cookie_name)?,
        path: validate_cookie_path(&get_string("cookie_path")?.unwrap_or_else(|| "/".to_string()))?,
        same_site: normalize_cookie_same_site(
            &get_string("cookie_same_site")?.unwrap_or_else(|| config.cookie_same_site.clone()),
        )?,
        secure: get_bool("cookie_secure")?.unwrap_or(config.cookie_secure),
        http_only: get_bool("cookie_http_only")?.unwrap_or(true),
        max_age: cookie_max_age,
    })
}

fn build_auth_cookie_string(value: &str, settings: &AuthCookieSettings) -> String {
    let mut cookie = format!(
        "{}={}; Path={}; Max-Age={}; SameSite={}",
        settings.name, value, settings.path, settings.max_age, settings.same_site
    );

    if settings.http_only {
        cookie.push_str("; HttpOnly");
    }
    if settings.secure {
        cookie.push_str("; Secure");
    }

    cookie
}

pub(super) fn build_signed_session_cookie(
    config: &AuthConfig,
    session_id: &str,
    overrides: Option<&HashMap<String, Value>>,
) -> std::result::Result<String, String> {
    let settings = auth_cookie_settings(config, default_auth_cookie_max_age(config), overrides)?;
    let signed_session_id = sign_session_id(session_id, &config.session_secret);
    Ok(build_auth_cookie_string(&signed_session_id, &settings))
}

pub(super) fn build_cleared_session_cookie(
    config: &AuthConfig,
    overrides: Option<&HashMap<String, Value>>,
) -> std::result::Result<String, String> {
    let sanitized_overrides = overrides.map(|map| {
        let mut sanitized = map.clone();
        sanitized.remove("cookie_max_age");
        sanitized
    });
    let settings = auth_cookie_settings(config, 0, sanitized_overrides.as_ref())?;
    Ok(build_auth_cookie_string("", &settings))
}

pub(super) fn build_signed_auth_challenge_cookie(
    config: &AuthConfig,
    challenge_id: &str,
    ttl: i64,
) -> std::result::Result<String, String> {
    let mut settings = auth_cookie_settings(config, ttl.max(0), None)?;
    settings.name = auth_challenge_cookie_name(config)?;
    let signed_challenge_id = sign_session_id(challenge_id, &config.session_secret);
    Ok(build_auth_cookie_string(&signed_challenge_id, &settings))
}

pub(super) fn build_cleared_auth_challenge_cookie(
    config: &AuthConfig,
) -> std::result::Result<String, String> {
    let mut settings = auth_cookie_settings(config, 0, None)?;
    settings.name = auth_challenge_cookie_name(config)?;
    Ok(build_auth_cookie_string("", &settings))
}

pub(super) fn add_set_cookie_header(
    response: &Value,
    cookie: &str,
) -> std::result::Result<Value, String> {
    let mut response = match response {
        Value::Map(map) => map.clone(),
        other => {
            return Err(format!(
                "[auth] response must be a map, got {}",
                other.type_name()
            ))
        }
    };

    let mut headers = match response.get("headers") {
        Some(Value::Map(headers)) => headers.clone(),
        Some(other) => {
            return Err(format!(
                "[auth] response headers must be a map, got {}",
                other.type_name()
            ))
        }
        None => HashMap::new(),
    };

    let set_cookie_key = headers
        .keys()
        .find(|key| key.eq_ignore_ascii_case("set-cookie"))
        .cloned()
        .unwrap_or_else(|| "Set-Cookie".to_string());

    match headers.get(&set_cookie_key) {
        Some(Value::Array(existing)) => {
            let mut cookies = existing.clone();
            cookies.push(Value::String(cookie.to_string()));
            headers.insert(set_cookie_key, Value::Array(cookies));
        }
        Some(Value::String(existing)) => {
            headers.insert(
                set_cookie_key,
                Value::Array(vec![
                    Value::String(existing.clone()),
                    Value::String(cookie.to_string()),
                ]),
            );
        }
        Some(other) => {
            return Err(format!(
                "[auth] response Set-Cookie header must be a string or array, got {}",
                other.type_name()
            ))
        }
        None => {
            headers.insert(set_cookie_key, Value::String(cookie.to_string()));
        }
    }

    response.insert("headers".to_string(), Value::Map(headers));
    Ok(Value::Map(response))
}
