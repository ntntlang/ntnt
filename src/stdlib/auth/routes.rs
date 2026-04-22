use super::cookies::{build_cleared_session_cookie, build_signed_session_cookie};
use super::guards::{encode_url_path_segment, escape_html, request_target};
use super::providers::{available_providers, suggest_provider};
use super::request_helpers::{request_device_name, request_ip_hash, request_user_agent_hash};
use super::*;

fn normalize_auth_route_prefix(prefix: &str) -> std::result::Result<String, String> {
    let trimmed = prefix.trim();
    if trimmed.is_empty() {
        return Err("[auth] route_prefix must not be empty".to_string());
    }

    let with_leading = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{}", trimmed.trim_start_matches('/'))
    };

    let normalized = if with_leading.len() > 1 {
        with_leading.trim_end_matches('/').to_string()
    } else {
        with_leading
    };

    if normalized.is_empty() || !normalized.starts_with('/') {
        return Err("[auth] route_prefix must start with /".to_string());
    }

    if normalized == "/" {
        return Err(
            "[auth] route_prefix must include at least one non-root path segment".to_string(),
        );
    }

    if normalized.chars().any(|ch| {
        ch.is_control()
            || ch.is_whitespace()
            || !matches!(ch,
                '/' | '-'
                    | '_'
                    | '~'
                    | '.'
                    | '0'..='9'
                    | 'A'..='Z'
                    | 'a'..='z'
            )
    }) {
        return Err(
            "[auth] route_prefix contains invalid characters; use URL-safe path segments only"
                .to_string(),
        );
    }

    Ok(normalized)
}

pub(super) fn normalize_auth_route_prefix_option(
    prefix: &str,
) -> std::result::Result<String, String> {
    normalize_auth_route_prefix(prefix)
}

pub(super) fn auth_route_prefix(config: &AuthConfig) -> String {
    normalize_auth_route_prefix(&config.route_prefix).unwrap_or_else(|_| "/auth".to_string())
}

pub(super) fn auth_route_path(config: &AuthConfig, suffix: &str) -> String {
    let prefix = auth_route_prefix(config);
    let trimmed = suffix.trim_start_matches('/');
    if trimmed.is_empty() {
        prefix
    } else {
        format!("{}/{}", prefix, trimmed)
    }
}

pub(super) fn auth_route_manifest(config: &AuthConfig) -> Vec<String> {
    vec![
        auth_route_path(config, ""),
        auth_route_path(config, "{provider}"),
        auth_route_path(config, "{provider}/callback"),
        auth_route_path(config, "logout"),
        auth_route_path(config, "health"),
    ]
}

pub(super) fn auth_route_collision_warnings(config: &AuthConfig) -> Vec<String> {
    let mut warnings = Vec::new();
    let prefix = auth_route_prefix(config);

    for protected in get_protected_paths() {
        let normalized = protected.trim();
        if normalized == "/*" || normalized == "/" {
            warnings.push(format!(
                "Protected path '{}' overlaps built-in auth routes under '{}'; auth routes remain exempt, but this catch-all may surprise app authors.",
                protected, prefix
            ));
            continue;
        }

        if let Some(base) = normalized.strip_suffix("/*") {
            let base = base.trim_end_matches('/');
            if prefix == base || prefix.starts_with(&format!("{}/", base)) {
                warnings.push(format!(
                    "Protected path '{}' overlaps built-in auth route prefix '{}'; auth routes stay exempt, but review middleware order and custom route wiring.",
                    protected, prefix
                ));
            }
        } else {
            let static_routes = [
                auth_route_path(config, ""),
                auth_route_path(config, "logout"),
                auth_route_path(config, "health"),
            ];
            if static_routes.iter().any(|route| route == normalized) {
                warnings.push(format!(
                    "Protected path '{}' exactly matches a built-in static auth route; auth routes stay exempt, but this pattern is likely accidental.",
                    protected
                ));
            }
        }
    }

    warnings
}

pub fn handle_auth_start(args: &[Value]) -> Result<Value> {
    let req = &args[0];

    let provider_name = if let Value::Map(req_map) = req {
        req_map.get("params").and_then(|params_value| {
            if let Value::Map(params) = params_value {
                params.get("provider").and_then(|provider_value| {
                    if let Value::String(provider) = provider_value {
                        Some(provider.clone())
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        })
    } else {
        None
    };

    let provider_name = provider_name.ok_or_else(|| {
        IntentError::runtime_error("[auth] No provider specified in /auth/{provider}".to_string())
    })?;

    let config = get_auth_config().ok_or_else(|| {
        IntentError::runtime_error(
            "[auth] Auth not configured - call enable_auth() first".to_string(),
        )
    })?;

    let provider = config
        .providers
        .iter()
        .find(|provider| provider.name == provider_name)
        .ok_or_else(|| {
            let msg = if let Some(suggestion) = suggest_provider(&provider_name) {
                format!(
                    "[auth] Unknown provider \"{}\"\n       Did you mean \"{}\"?\n       Available providers: {}",
                    provider_name,
                    suggestion,
                    available_providers()
                )
            } else {
                format!(
                    "[auth] Unknown provider \"{}\"\n       Available providers: {}",
                    provider_name,
                    available_providers()
                )
            };
            IntentError::runtime_error(msg)
        })?;

    let state = generate_oauth_state();
    let nonce = if provider.supports_oidc {
        Some(generate_nonce())
    } else {
        None
    };

    let (pkce_verifier, pkce_challenge) = if provider.use_pkce {
        let verifier = generate_pkce_verifier();
        let challenge = generate_pkce_challenge(&verifier);
        (Some(verifier), Some(challenge))
    } else {
        (None, None)
    };

    let (host, proto) = get_host_and_proto(req);
    let redirect_uri = format!(
        "{}://{}{}",
        proto,
        host,
        auth_route_path(&config, &format!("{}/callback", provider.name))
    );

    store_oauth_state(
        &state,
        &provider.name,
        &redirect_uri,
        nonce.as_deref(),
        pkce_verifier.as_deref(),
        false,
        request_device_name(req).as_deref(),
        request_user_agent_hash(req).as_deref(),
        request_ip_hash(req).as_deref(),
    );

    let mut provider_for_url = provider.clone();
    if config.store_tokens && !provider_for_url.extra_params.contains_key("access_type") {
        if provider_for_url.authorize_url.contains("google") {
            provider_for_url
                .extra_params
                .insert("access_type".to_string(), "offline".to_string());
            if !provider_for_url.extra_params.contains_key("prompt") {
                provider_for_url
                    .extra_params
                    .insert("prompt".to_string(), "consent".to_string());
            }
        }
    }

    let auth_url = generate_auth_url(
        &provider_for_url,
        &redirect_uri,
        &state,
        nonce.as_deref(),
        pkce_challenge.as_deref(),
    );

    Ok(redirect_response(&auth_url, None))
}

pub fn handle_auth_protect(args: &[Value]) -> Result<Value> {
    match enforce_auth_for_request(&args[0], false) {
        Ok(Some(cookie)) => {
            let mut response = match redirect_response(&request_target(&args[0]), Some(&cookie)) {
                Value::Map(map) => map,
                other => return Ok(other),
            };
            response.insert("status".to_string(), Value::Int(307));
            Ok(Value::Map(response))
        }
        Ok(None) => Ok(Value::Unit),
        Err(response) => Ok(response),
    }
}

pub fn handle_auth_index(_args: &[Value]) -> Result<Value> {
    let config = get_auth_config()
        .ok_or_else(|| IntentError::runtime_error("[auth] Auth not configured".to_string()))?;

    if !config.login_page_enabled {
        return Ok(json_response(
            Value::Map(HashMap::from([
                (
                    "error".to_string(),
                    Value::String(
                        "Built-in auth login page is disabled. Mount a custom route or enable login_page.".to_string(),
                    ),
                ),
                (
                    "routes".to_string(),
                    Value::Array(
                        auth_route_manifest(&config)
                            .into_iter()
                            .map(Value::String)
                            .collect(),
                    ),
                ),
            ])),
            404,
        ));
    }

    if config.providers.len() == 1 {
        let provider = &config.providers[0];
        let safe_provider = encode_url_path_segment(&provider.name);
        return Ok(redirect_response(
            &auth_route_path(&config, &safe_provider),
            None,
        ));
    }

    let provider_links = config
        .providers
        .iter()
        .map(|provider| {
            let label = escape_html(&provider.name);
            let safe_provider = encode_url_path_segment(&provider.name);
            format!(
                r#"<li><a class="ntnt-auth-provider" href="{href}">Sign in with {label}</a></li>"#,
                href = auth_route_path(&config, &safe_provider),
                label = label
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let title = escape_html(&config.login_page_title);
    let heading = escape_html(&config.login_page_heading);
    let copy = escape_html(&config.login_page_copy);
    let logo_html = config
        .login_page_logo_url
        .as_ref()
        .map(|logo| logo.trim())
        .filter(|logo| !logo.is_empty())
        .map(|logo| {
            format!(
                r#"<img class="ntnt-auth-logo" src="{}" alt="{} logo">"#,
                escape_html(logo),
                title
            )
        })
        .unwrap_or_default();

    Ok(html_response(&format!(
        r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>{title}</title>
    <style>
      :root {{ color-scheme: light dark; }}
      body {{ font-family: system-ui, sans-serif; margin: 0; background: #0b1020; color: #f5f7fb; }}
      main {{ max-width: 28rem; margin: 4rem auto; padding: 2rem; background: rgba(15, 23, 42, 0.92); border-radius: 1rem; box-shadow: 0 20px 40px rgba(0,0,0,0.25); }}
      h1 {{ margin-bottom: 0.5rem; }}
      p {{ color: #cbd5e1; line-height: 1.5; }}
      ul {{ list-style: none; padding: 0; margin: 1.5rem 0 0; display: grid; gap: 0.75rem; }}
      a.ntnt-auth-provider {{ display: block; padding: 0.85rem 1rem; border-radius: 0.75rem; text-decoration: none; background: #2563eb; color: white; font-weight: 600; text-align: center; }}
      .ntnt-auth-logo {{ max-width: 72px; max-height: 72px; display: block; margin-bottom: 1rem; border-radius: 0.75rem; }}
    </style>
  </head>
  <body>
    <main>
      {logo_html}
      <h1>{heading}</h1>
      <p>{copy}</p>
      <ul>
        {provider_links}
      </ul>
    </main>
  </body>
</html>"#,
        title = title,
        heading = heading,
        copy = copy,
        logo_html = logo_html,
        provider_links = provider_links,
    )))
}

pub fn handle_auth_callback(args: &[Value]) -> Result<Value> {
    let req = &args[0];

    let config = get_auth_config()
        .ok_or_else(|| IntentError::runtime_error("[auth] Auth not configured".to_string()))?;

    let (code, state, error, exchange) = if let Value::Map(req_map) = req {
        if let Some(Value::Map(query)) = req_map.get("query_params") {
            let code = query.get("code").and_then(|value| {
                if let Value::String(string) = value {
                    Some(string.clone())
                } else {
                    None
                }
            });
            let state = query.get("state").and_then(|value| {
                if let Value::String(string) = value {
                    Some(string.clone())
                } else {
                    None
                }
            });
            let error = query.get("error").and_then(|value| {
                if let Value::String(string) = value {
                    Some(string.clone())
                } else {
                    None
                }
            });
            let exchange = query.get("exchange").and_then(|value| {
                if let Value::String(string) = value {
                    Some(string.clone())
                } else {
                    None
                }
            });
            (code, state, error, exchange)
        } else {
            (None, None, None, None)
        }
    } else {
        (None, None, None, None)
    };

    if let Some(err) = error {
        eprintln!("[auth] OAuth error: {}", err);
        return Ok(redirect_response(&config.failure_url, None));
    }

    if let Some(exchange_token) = exchange {
        if code.is_some() {
            let _ = consume_exchange_token(&exchange_token);
            eprintln!(
                "[auth] Ignoring exchange token — OAuth code also present (crafted request?)"
            );
        } else if let Some(session_id) = consume_exchange_token(&exchange_token) {
            if get_session_by_id(&session_id).is_some() {
                eprintln!(
                    "[auth] Session exchange token consumed for session {}...",
                    &session_id[..8]
                );

                let cookie = build_signed_session_cookie(&config, &session_id, None)
                    .map_err(IntentError::runtime_error)?;

                return Ok(redirect_response(&config.success_url, Some(&cookie)));
            } else {
                eprintln!("[auth] Exchange token valid but session not found");
                return Ok(redirect_response(&config.failure_url, None));
            }
        } else {
            eprintln!("[auth] Invalid or expired exchange token");
            return Ok(redirect_response(&config.failure_url, None));
        }
    }

    let oauth_state = state
        .as_ref()
        .and_then(|state_value| consume_oauth_state(state_value));

    if oauth_state.is_none() || code.is_none() {
        eprintln!("[auth] Invalid callback - missing code or state");
        return Ok(redirect_response(&config.failure_url, None));
    }

    let oauth_state = oauth_state.unwrap();
    let code = code.unwrap();

    let provider = config
        .providers
        .iter()
        .find(|provider| provider.name == oauth_state.provider);
    if provider.is_none() {
        eprintln!("[auth] Provider not found: {}", oauth_state.provider);
        return Ok(redirect_response(&config.failure_url, None));
    }
    let provider = provider.unwrap();
    let provider_name = provider.name.clone();

    let tokens = match exchange_code_for_tokens(
        provider,
        &code,
        &oauth_state.redirect_url,
        oauth_state.pkce_verifier.as_deref(),
    ) {
        Ok(tokens) => tokens,
        Err(error) => {
            eprintln!("{}", error);
            return Ok(redirect_response(&config.failure_url, None));
        }
    };

    let user_info = if let Some(id_token) = &tokens.id_token {
        match decode_id_token(id_token) {
            Ok(claims) => {
                if let Err(error) = validate_id_token_claims(
                    &claims,
                    provider.issuer.as_deref(),
                    &provider.client_id,
                    oauth_state.nonce.as_deref(),
                ) {
                    eprintln!("{}", error);
                    return Ok(redirect_response(&config.failure_url, None));
                }
                claims
            }
            Err(error) => {
                eprintln!(
                    "[auth] ID token decode failed, falling back to userinfo: {}",
                    error
                );
                match fetch_userinfo(provider, &tokens.access_token) {
                    Ok(user_info) => user_info,
                    Err(error) => {
                        eprintln!("{}", error);
                        return Ok(redirect_response(&config.failure_url, None));
                    }
                }
            }
        }
    } else {
        match fetch_userinfo(provider, &tokens.access_token) {
            Ok(user_info) => user_info,
            Err(error) => {
                eprintln!("{}", error);
                return Ok(redirect_response(&config.failure_url, None));
            }
        }
    };

    let effective_session_ttl = config
        .session_ttl
        .min(config.max_session_ttl.unwrap_or(config.session_ttl));
    let session = create_session(
        &provider_name,
        user_info,
        if config.store_tokens {
            Some(&tokens)
        } else {
            None
        },
        effective_session_ttl,
    )
    .map_err(|error| {
        IntentError::runtime_error(format!("[auth] Failed to create session: {}", error))
    })?;
    let mut session = session;
    if let Some(existing_session_id) = get_session_id_from_request(req) {
        if let Some(existing_session) = get_session_by_id(&existing_session_id) {
            if existing_session_id != session.id {
                if session.data_json == "{}" && existing_session.data_json != "{}" {
                    session.data_json = existing_session.data_json.clone();
                }
                migrate_session(&existing_session_id, &session).map_err(|error| {
                    IntentError::runtime_error(format!(
                        "[auth] Failed to rotate session after OAuth callback: {}",
                        error
                    ))
                })?;
            } else {
                store_session(session.clone());
            }
        } else {
            store_session(session.clone());
        }
    } else {
        store_session(session.clone());
    }
    let session_id = session.id.clone();

    let exchange_token = generate_session_id();
    store_exchange_token(&exchange_token, &session_id);

    let safe_provider = encode_url_path_segment(&provider_name);
    let callback_url = format!(
        "{}?exchange={}",
        auth_route_path(&config, &format!("{}/callback", safe_provider)),
        exchange_token
    );
    let html = format!(
        r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>Completing login...</title>
<meta http-equiv="refresh" content="0;url={url}"></head>
<body><p>Completing login...</p>
<script>window.location.replace("{url}");</script>
<noscript><a href="{url}">Click here to continue</a></noscript>
</body></html>"#,
        url = callback_url
    );

    Ok(html_response(&html))
}

fn auth_health_enabled(config: &AuthConfig) -> bool {
    if config.health_endpoint {
        return true;
    }

    std::env::var("NTNT_ENV")
        .map(|v| v != "production" && v != "prod")
        .unwrap_or(true)
}

fn sanitize_session_store_label(store: &SessionStore) -> String {
    match store {
        SessionStore::Memory => "memory".to_string(),
        SessionStore::Sqlite(_) => "sqlite".to_string(),
        SessionStore::Postgres(_) => "postgres".to_string(),
        SessionStore::Redis(url) => {
            if url.starts_with("valkey://") {
                "valkey".to_string()
            } else {
                "redis".to_string()
            }
        }
    }
}

fn auth_health_warnings(config: &AuthConfig, request: Option<&Value>) -> Vec<String> {
    let mut warnings = Vec::new();
    let is_prod = std::env::var("NTNT_ENV")
        .map(|v| v == "production" || v == "prod")
        .unwrap_or(false);

    if matches!(config.session_store, SessionStore::Memory) && is_prod {
        warnings.push("Production is using in-memory session storage; sessions will be lost on restart and are not shared across instances.".to_string());
    }

    if is_prod && config.session_secret == DEFAULT_SESSION_SECRET_SENTINEL {
        warnings.push("Production is using the default session secret sentinel; auth should fail closed before this is reachable.".to_string());
    }

    if config.providers.is_empty() {
        warnings.push("No OAuth providers configured.".to_string());
    }

    let site_url_missing = std::env::var("SITE_URL").is_err();
    let callback_example = request.map(get_host_and_proto).and_then(|(host, proto)| {
        config.providers.first().map(|provider| {
            format!(
                "{}://{}{}",
                proto,
                host,
                auth_route_path(config, &format!("{}/callback", provider.name))
            )
        })
    });

    for provider in &config.providers {
        if provider.client_id.trim().is_empty() {
            warnings.push(format!(
                "Provider '{}' is missing client_id.",
                provider.name
            ));
        }
        if provider.client_secret.trim().is_empty() {
            warnings.push(format!(
                "Provider '{}' is missing client_secret.",
                provider.name
            ));
        }
    }

    if site_url_missing {
        if let Some(callback) = callback_example {
            warnings.push(format!(
                "SITE_URL is not set; OAuth callback URLs currently depend on request host headers (example callback: {}).",
                callback
            ));
        }
    }

    warnings
}

pub fn handle_auth_health(args: &[Value]) -> Result<Value> {
    let config = get_auth_config()
        .ok_or_else(|| IntentError::runtime_error("[auth] Auth not configured".to_string()))?;

    if !auth_health_enabled(&config) {
        return Ok(crate::stdlib::http_server::create_error_response(
            404,
            "Auth health endpoint is disabled in production.",
        ));
    }

    let request = args.first();
    let mut response = HashMap::new();
    response.insert("ok".to_string(), serde_json::Value::Bool(true));
    response.insert(
        "environment".to_string(),
        serde_json::Value::String(
            std::env::var("NTNT_ENV").unwrap_or_else(|_| "development".to_string()),
        ),
    );
    response.insert(
        "health_endpoint".to_string(),
        serde_json::Value::Bool(config.health_endpoint),
    );
    response.insert(
        "providers".to_string(),
        serde_json::Value::Array(
            config
                .providers
                .iter()
                .map(|provider| {
                    serde_json::json!({
                        "name": provider.name,
                        "supports_oidc": provider.supports_oidc,
                        "use_pkce": provider.use_pkce,
                        "has_client_id": !provider.client_id.trim().is_empty(),
                        "has_client_secret": !provider.client_secret.trim().is_empty(),
                        "authorize_url": provider.authorize_url,
                        "token_url": provider.token_url,
                    })
                })
                .collect(),
        ),
    );
    response.insert(
        "routes".to_string(),
        serde_json::json!({
            "prefix": auth_route_prefix(&config),
            "index": auth_route_path(&config, ""),
            "start": auth_route_path(&config, "{provider}"),
            "callback": auth_route_path(&config, "{provider}/callback"),
            "logout": auth_route_path(&config, "logout"),
            "health": auth_route_path(&config, "health"),
            "success_url": config.success_url,
            "failure_url": config.failure_url,
            "logout_url": config.logout_url,
        }),
    );
    response.insert(
        "cookie".to_string(),
        serde_json::json!({
            "name": config.cookie_name,
            "secure": config.cookie_secure,
            "same_site": config.cookie_same_site,
            "http_only": true,
        }),
    );
    response.insert(
        "session".to_string(),
        serde_json::json!({
            "store": sanitize_session_store_label(&config.session_store),
            "store_tokens": config.store_tokens,
            "session_ttl": config.session_ttl,
            "refresh_ttl": config.refresh_ttl,
            "sliding_sessions": config.sliding_sessions,
            "refresh_throttle": config.refresh_throttle,
            "max_session_ttl": config.max_session_ttl,
            "preset": config.auth_preset,
        }),
    );
    response.insert(
        "route_collision_warnings".to_string(),
        serde_json::Value::Array(
            auth_route_collision_warnings(&config)
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    response.insert(
        "protected_paths".to_string(),
        serde_json::Value::Array(
            get_protected_paths()
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );
    if let Some(req) = request {
        let (host, proto) = get_host_and_proto(req);
        response.insert(
            "request_context".to_string(),
            serde_json::json!({
                "host": host,
                "proto": proto,
            }),
        );
    }
    response.insert(
        "warnings".to_string(),
        serde_json::Value::Array(
            auth_health_warnings(&config, request)
                .into_iter()
                .map(serde_json::Value::String)
                .collect(),
        ),
    );

    let value_map = json_map_to_value_map(&response.into_iter().collect());
    Ok(json_response(Value::Map(value_map), 200))
}

pub fn handle_auth_logout(args: &[Value]) -> Result<Value> {
    let req = &args[0];
    let config = get_auth_config().unwrap_or_default();

    if let Some(session_id) = get_session_id_from_request(req) {
        delete_session_by_id(&session_id);
    }

    let cookie = build_cleared_session_cookie(&config, None).map_err(IntentError::runtime_error)?;

    Ok(redirect_response(&config.logout_url, Some(&cookie)))
}
