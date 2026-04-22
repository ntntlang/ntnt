use super::*;

fn request_header(request: &Value, name: &str) -> Option<String> {
    if let Value::Map(req_map) = request {
        if let Some(Value::Map(headers)) = req_map.get("headers") {
            return headers.get(name).and_then(|value| match value {
                Value::String(s) => Some(s.clone()),
                _ => None,
            });
        }
    }
    None
}

pub(super) fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub(super) fn request_user_agent_hash(request: &Value) -> Option<String> {
    request_header(request, "user-agent")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(|value| sha256_hex(&value))
}

pub(super) fn request_ip_hash(request: &Value) -> Option<String> {
    let forwarded = request_header(request, "x-forwarded-for")
        .and_then(|value| value.split(',').next().map(|part| part.trim().to_string()))
        .filter(|value| !value.is_empty());
    let direct = request_header(request, "x-real-ip")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    forwarded.or(direct).map(|value| sha256_hex(&value))
}

pub(super) fn request_device_name(request: &Value) -> Option<String> {
    let ua = request_header(request, "user-agent")?;
    let lower = ua.to_ascii_lowercase();
    let device = if lower.contains("iphone") {
        "iPhone"
    } else if lower.contains("ipad") {
        "iPad"
    } else if lower.contains("android") && lower.contains("mobile") {
        "Android phone"
    } else if lower.contains("android") {
        "Android device"
    } else if lower.contains("macintosh") || lower.contains("mac os") {
        "Mac"
    } else if lower.contains("windows") {
        "Windows PC"
    } else if lower.contains("linux") {
        "Linux device"
    } else {
        "Unknown device"
    };

    let browser = if lower.contains("edg/") {
        "Edge"
    } else if lower.contains("chrome/") && !lower.contains("edg/") {
        "Chrome"
    } else if lower.contains("safari/") && !lower.contains("chrome/") {
        "Safari"
    } else if lower.contains("firefox/") {
        "Firefox"
    } else {
        "browser"
    };

    Some(format!("{} · {}", device, browser))
}

/// Get session ID from request cookies
pub fn get_session_id_from_request(request: &Value) -> Option<String> {
    let config = get_auth_config()?;
    let cookie_name = &config.cookie_name;

    if let Value::Map(req_map) = request {
        if let Some(Value::Map(headers)) = req_map.get("headers") {
            if let Some(Value::String(cookie_header)) = headers.get("cookie") {
                for cookie in cookie_header.split(';') {
                    let parts: Vec<&str> = cookie.trim().splitn(2, '=').collect();
                    if parts.len() == 2 && parts[0] == cookie_name {
                        let signed_token = parts[1];
                        // Verify HMAC signature and extract session ID
                        return verify_session_id(signed_token, &config.session_secret);
                    }
                }
            }
        }
    }
    None
}

pub fn get_auth_challenge_id_from_request(request: &Value) -> Option<String> {
    let config = get_auth_config()?;
    let cookie_name = auth_challenge_cookie_name(&config).ok()?;

    if let Value::Map(req_map) = request {
        if let Some(Value::Map(headers)) = req_map.get("headers") {
            if let Some(Value::String(cookie_header)) = headers.get("cookie") {
                for cookie in cookie_header.split(';') {
                    let parts: Vec<&str> = cookie.trim().splitn(2, '=').collect();
                    if parts.len() == 2 && parts[0] == cookie_name {
                        let signed_token = parts[1];
                        return verify_session_id(signed_token, &config.session_secret);
                    }
                }
            }
        }
    }

    None
}

/// Get user from request as HashMap (internal helper)
pub(super) fn get_user_from_request(request: &Value) -> Option<HashMap<String, Value>> {
    let session_id = get_session_id_from_request(request)?;
    let session = get_session_by_id(&session_id)?;
    if let Value::Map(m) = user_to_value(&session) {
        Some(m)
    } else {
        None
    }
}

/// Convert Session to User Value
pub fn user_to_value(session: &Session) -> Value {
    let mut map = HashMap::new();
    map.insert("id".to_string(), Value::String(session.user_id.clone()));
    map.insert(
        "provider".to_string(),
        Value::String(session.provider.clone()),
    );
    if let Some(email) = &session.email {
        map.insert("email".to_string(), Value::String(email.clone()));
    }
    if let Some(name) = &session.name {
        map.insert("name".to_string(), Value::String(name.clone()));
    }
    if let Some(picture) = &session.picture {
        map.insert("picture".to_string(), Value::String(picture.clone()));
    }
    map.insert(
        "raw".to_string(),
        Value::Map(json_string_to_value_map(&session.raw_json)),
    );
    // Include CSRF token so apps can embed it in forms: {{user.csrf_token}}
    if !session.csrf_token.is_empty() {
        map.insert(
            "csrf_token".to_string(),
            Value::String(session.csrf_token.clone()),
        );
    }
    Value::Map(map)
}

/// Convert Session to full Session Value
pub fn session_to_value(session: &Session) -> Value {
    let mut map = HashMap::new();
    map.insert("id".to_string(), Value::String(session.id.clone()));
    map.insert("user".to_string(), user_to_value(session));
    map.insert("created_at".to_string(), Value::Int(session.created_at));
    map.insert("expires_at".to_string(), Value::Int(session.expires_at));
    map.insert(
        "data".to_string(),
        Value::Map(json_string_to_value_map(&session.data_json)),
    );
    map.insert(
        "csrf_token".to_string(),
        Value::String(session.csrf_token.clone()),
    );

    // Include token info if available
    if let Some(ref at) = session.access_token {
        map.insert("access_token".to_string(), Value::String(at.clone()));
    }
    if let Some(ref rt) = session.refresh_token {
        map.insert("has_refresh_token".to_string(), Value::Bool(true));
        // Don't expose refresh token directly for security
        let _ = rt;
    }
    if let Some(exp) = session.token_expires_at {
        map.insert("token_expires_at".to_string(), Value::Int(exp));
    }

    Value::Map(map)
}

pub fn auth_challenge_to_value(challenge: &AuthChallenge) -> Value {
    let mut map = HashMap::new();
    let user_id = if challenge
        .subject_id
        .starts_with(&format!("{}:", challenge.provider))
    {
        challenge.subject_id.clone()
    } else {
        format!("{}:{}", challenge.provider, challenge.subject_id)
    };
    let user = HashMap::from([
        ("id".to_string(), Value::String(user_id.clone())),
        (
            "provider".to_string(),
            Value::String(challenge.provider.clone()),
        ),
    ]);

    map.insert("id".to_string(), Value::String(challenge.id.clone()));
    map.insert(
        "subject_id".to_string(),
        Value::String(challenge.subject_id.clone()),
    );
    map.insert("user_id".to_string(), Value::String(user_id));
    map.insert("user".to_string(), Value::Map(user));
    map.insert(
        "provider".to_string(),
        Value::String(challenge.provider.clone()),
    );
    map.insert("kind".to_string(), Value::String(challenge.kind.clone()));
    map.insert("created_at".to_string(), Value::Int(challenge.created_at));
    map.insert("expires_at".to_string(), Value::Int(challenge.expires_at));
    map.insert(
        "data".to_string(),
        Value::Map(json_string_to_value_map(&challenge.data_json)),
    );

    Value::Map(map)
}

// ============================================================================
// SECTION 8: Route Handlers
// ============================================================================

fn parse_canonical_site_url(site_url: &str) -> Option<(String, String)> {
    let trimmed = site_url.trim();
    let (proto, remainder) = if let Some(rest) = trimmed.strip_prefix("https://") {
        ("https".to_string(), rest)
    } else if let Some(rest) = trimmed.strip_prefix("http://") {
        ("http".to_string(), rest)
    } else {
        return None;
    };

    let host = remainder.split('/').next()?.trim();
    if host.is_empty()
        || !host
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']'))
    {
        return None;
    }

    Some((host.to_string(), proto))
}

fn normalized_proto(proto: &str) -> Option<String> {
    let trimmed = proto.trim();
    if trimmed.eq_ignore_ascii_case("https") {
        Some("https".to_string())
    } else if trimmed.eq_ignore_ascii_case("http") {
        Some("http".to_string())
    } else {
        None
    }
}

fn normalized_host(host: &str) -> Option<String> {
    let trimmed = host.trim();
    if trimmed.is_empty()
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

/// Helper to get host and protocol from request
pub(super) fn get_host_and_proto(req: &Value) -> (String, String) {
    if let Ok(site_url) = std::env::var("SITE_URL") {
        if let Some((host, proto)) = parse_canonical_site_url(&site_url) {
            return (host, proto);
        }
    }

    if let Value::Map(req_map) = req {
        let host = req_map
            .get("headers")
            .and_then(|h| {
                if let Value::Map(headers) = h {
                    headers.get("host").and_then(|v| {
                        if let Value::String(s) = v {
                            normalized_host(s)
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "localhost:8080".to_string());

        let proto = req_map
            .get("protocol")
            .and_then(|v| {
                if let Value::String(s) = v {
                    normalized_proto(s)
                } else {
                    None
                }
            })
            .or_else(|| {
                req_map.get("headers").and_then(|h| {
                    if let Value::Map(headers) = h {
                        headers.get("x-forwarded-proto").and_then(|v| {
                            if let Value::String(s) = v {
                                normalized_proto(s)
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_else(|| {
                if host.contains("localhost") || host.starts_with("127.") {
                    "http".to_string()
                } else {
                    "https".to_string()
                }
            });

        (host, proto)
    } else {
        ("localhost:8080".to_string(), "http".to_string())
    }
}
