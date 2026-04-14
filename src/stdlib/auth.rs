//! Auth module for NTNT — Full OAuth 2.0 and OIDC support
//!
//! Progressive disclosure design:
//! - One line for common cases: `enable_auth(oauth("google", id, secret))`
//! - Customization when needed: scopes, PKCE, refresh tokens
//! - Full control for enterprise: custom providers, M2M auth, token validation
//!
//! # Supported Flows
//! - Authorization Code (server-side apps)
//! - Authorization Code + PKCE (SPAs, mobile, CLI)
//! - Client Credentials (machine-to-machine)
//! - Refresh Token (long-lived sessions)
//!
//! # OIDC Support
//! - ID token extraction and validation
//! - Nonce for replay attack protection
//! - OIDC Discovery (auto-configure from issuer)
//! - ID token claims as user info source
//!
//! # Boundary
//! - `std/auth` is for auth flows, sessions, CSRF, current-user helpers, and TOTP
//! - generic crypto helpers like `uuid`, `hash_password`, and `verify_password` belong in `std/crypto`
//!
//! # Quick Start
//! ```ntnt
//! import { oauth, enable_auth, get_user } from "std/auth"
//!
//! enable_auth(oauth("google", get_env("GOOGLE_CLIENT_ID"), get_env("GOOGLE_CLIENT_SECRET")))
//!
//! fn dashboard(req) {
//!     let user = get_user(req) otherwise return redirect("/login")
//!     return html("<h1>Hello, #{user.name}!</h1>")
//! }
//! ```

use crate::error::{IntentError, Result};
use crate::interpreter::{RuntimeCapability, Value};
use base64::Engine;
use hmac::{Hmac, Mac};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use totp_rs::{Algorithm as TotpAlgorithm, Secret, TOTP};

mod config;
mod cookies;
mod guards;
mod primitives;
mod providers;
mod request_helpers;
mod routes;
mod sessions;
mod storage;
mod utils;

use config::{auth_option_suggestion, initialize_session_store};
pub use config::{ensure_auth_session_store, init_auth, parse_auth_session_store};
pub use cookies::default_auth_cookie_secure_env;
use cookies::{
    add_set_cookie_header, auth_challenge_cookie_name, build_cleared_auth_challenge_cookie,
    build_cleared_session_cookie, build_signed_auth_challenge_cookie, build_signed_session_cookie,
};
#[cfg(test)]
use guards::path_matches_protected_pattern;
use guards::{encode_url_path_segment, escape_html, validate_auth_challenge_kind};
pub use guards::{
    enforce_auth_for_request, get_protected_paths, register_protected_paths, reset_protected_paths,
};
pub use primitives::{
    generate_nonce, generate_oauth_state, generate_session_id, generate_totp_secret, get_totp_uri,
    sign_session_id, verify_session_id, verify_totp_code,
};
use providers::{
    available_providers, get_builtin_provider, is_safe_provider_name, suggest_provider,
    validate_provider_name, value_map_to_provider,
};
pub use providers::{provider_to_value, value_to_provider};
pub use request_helpers::{
    auth_challenge_to_value, get_auth_challenge_id_from_request, get_session_id_from_request,
    session_to_value, user_to_value,
};
use request_helpers::{get_host_and_proto, get_user_from_request};
pub use routes::{
    handle_auth_callback, handle_auth_index, handle_auth_logout, handle_auth_protect,
    handle_auth_start,
};
use sessions::{
    build_rotated_session, get_session_by_id, migrate_session, update_session_data,
    update_session_tokens,
};
pub use sessions::{
    cleanup_expired_sessions, delete_all_sessions_for_user, delete_session_by_id,
    get_sessions_for_user, store_session,
};
#[cfg(test)]
use storage::store_auth_challenge_sqlite;
pub use storage::{
    cleanup_expired_auth_challenges, cleanup_expired_exchange_tokens, cleanup_expired_oauth_states,
    consume_oauth_state, delete_auth_challenge_by_id, get_auth_challenge_by_id,
    store_auth_challenge, store_oauth_state,
};
use storage::{
    consume_auth_challenge, consume_exchange_token, create_auth_challenge, create_manual_session,
    create_session, store_exchange_token, EXCHANGE_TOKEN_TTL,
};
use utils::{
    html_response, json_map_to_value_map, json_response, json_string_to_value_map,
    json_to_value_map, redirect_response, value_map_to_json_string, value_to_json,
};

// Internal architecture note:
// Phase 4.5 is being executed in small, behavior-preserving slices. `auth.rs` remains the
// public std/auth surface for now, while lower-risk internals move into focused submodules.
// During this cleanup, prefer explicit semantics over clever abstraction, keep fallback/error
// behavior intentional, and avoid mixing new auth features into refactor-only slices.

// ============================================================================
// SECURITY HELPERS
// ============================================================================

/// Constant-time string comparison to prevent timing attacks.
/// Returns true if both strings are equal, performing the same number of operations
/// regardless of where the first difference occurs.
fn constant_time_compare(a: &str, b: &str) -> bool {
    let a_bytes = a.as_bytes();
    let b_bytes = b.as_bytes();

    // Length check - still leaks length info but that's acceptable for most security tokens
    if a_bytes.len() != b_bytes.len() {
        return false;
    }

    // XOR all bytes and accumulate - runs in constant time
    let mut result: u8 = 0;
    for (x, y) in a_bytes.iter().zip(b_bytes.iter()) {
        result |= x ^ y;
    }
    result == 0
}

// ============================================================================
// SECTION 1: Value Helpers for Option/Result types
// ============================================================================

fn make_none() -> Value {
    Value::none()
}

fn make_some(value: Value) -> Value {
    Value::some(value)
}

fn make_ok(value: Value) -> Value {
    Value::ok(value)
}

fn make_err(value: Value) -> Value {
    Value::err(value)
}

// ============================================================================
// SECTION 2: Types
// ============================================================================

/// Configuration for an OAuth provider
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
    pub userinfo_url: String,
    pub issuer: Option<String>,   // OIDC issuer for token validation
    pub jwks_uri: Option<String>, // JWKS endpoint for RS256 validation
    pub scopes: Vec<String>,
    pub extra_params: HashMap<String, String>,
    pub use_pkce: bool,      // Enable PKCE
    pub supports_oidc: bool, // Provider supports OIDC
}

impl Default for ProviderConfig {
    fn default() -> Self {
        ProviderConfig {
            name: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            authorize_url: String::new(),
            token_url: String::new(),
            userinfo_url: String::new(),
            issuer: None,
            jwks_uri: None,
            scopes: Vec::new(),
            extra_params: HashMap::new(),
            use_pkce: false,
            supports_oidc: false,
        }
    }
}

/// Token response from OAuth provider
#[derive(Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<i64>,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub scope: Option<String>,
}

/// Session data stored in memory
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub picture: Option<String>,
    pub raw_json: String,
    pub data_json: String,
    pub csrf_token: String, // CSRF protection token
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
    pub token_expires_at: Option<i64>,
    pub created_at: i64,
    pub expires_at: i64,
}

/// Pending auth challenge stored separately from authenticated sessions
#[derive(Debug, Clone)]
pub struct AuthChallenge {
    pub id: String,
    pub subject_id: String,
    pub provider: String,
    pub kind: String,
    pub data_json: String,
    pub created_at: i64,
    pub expires_at: i64,
}

/// OAuth state for CSRF protection
#[derive(Debug, Clone)]
pub struct OAuthState {
    pub state: String,
    pub nonce: Option<String>,         // OIDC nonce for replay protection
    pub pkce_verifier: Option<String>, // PKCE code verifier
    pub provider: String,
    pub redirect_url: String,
    pub created_at: i64,
}

/// Session storage backend type
#[derive(Debug, Clone)]
pub enum SessionStore {
    Memory,
    Sqlite(String),   // Path to SQLite database
    Postgres(String), // Connection URL
    Redis(String),    // Redis/Valkey URL (redis:// or valkey://)
}

impl Default for SessionStore {
    fn default() -> Self {
        SessionStore::Memory
    }
}

/// Full auth configuration
#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub providers: Vec<ProviderConfig>,
    pub success_url: String,
    pub failure_url: String,
    pub logout_url: String,
    pub protected_paths: Vec<String>,
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub cookie_same_site: String,
    pub session_ttl: i64,
    pub refresh_ttl: i64, // How long refresh tokens can extend sessions (default: 30 days)
    pub store_tokens: bool, // Store access/refresh tokens in session
    pub session_secret: String, // Secret for session signing
    pub session_store: SessionStore, // Session storage backend
}

impl Default for AuthConfig {
    fn default() -> Self {
        AuthConfig {
            providers: Vec::new(),
            success_url: "/".to_string(),
            failure_url: "/".to_string(),
            logout_url: "/".to_string(),
            protected_paths: Vec::new(),
            cookie_name: "ntnt_session".to_string(),
            cookie_secure: true,
            cookie_same_site: "lax".to_string(),
            session_ttl: 86400 * 7,  // 7 days
            refresh_ttl: 86400 * 30, // 30 days — how long refresh tokens can extend sessions
            store_tokens: false,
            session_secret: DEFAULT_SESSION_SECRET_SENTINEL.to_string(),
            session_store: SessionStore::Memory,
        }
    }
}

/// Sentinel value used to detect when user hasn't set a session secret.
/// Not used as an actual secret — dev mode generates a random one.
const DEFAULT_SESSION_SECRET_SENTINEL: &str = "ntnt-dev-secret-change-in-production";

/// Auto-generated random session secret for dev mode.
/// Generated once at startup; sessions won't persist across restarts.
fn dev_session_secret() -> &'static str {
    use rand::RngCore;
    static SECRET: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        hex::encode(bytes)
    });
    &SECRET
}

/// OIDC Discovery document
#[derive(Debug, Clone)]
pub struct OidcDiscovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub userinfo_endpoint: Option<String>,
    pub jwks_uri: String,
    pub scopes_supported: Vec<String>,
    pub response_types_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
}

/// Extract user info from provider response or ID token
fn extract_user_info(
    provider: &str,
    info: &HashMap<String, Value>,
) -> (String, Option<String>, Option<String>, Option<String>) {
    let get_string = |key: &str| -> Option<String> {
        info.get(key).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            Value::Int(i) => Some(i.to_string()),
            _ => None,
        })
    };

    match provider {
        "google" => (
            get_string("id")
                .or_else(|| get_string("sub"))
                .unwrap_or_default(),
            get_string("email"),
            get_string("name"),
            get_string("picture"),
        ),
        "github" => (
            get_string("id").unwrap_or_default(),
            get_string("email"),
            get_string("name").or_else(|| get_string("login")),
            get_string("avatar_url"),
        ),
        "facebook" => {
            let picture = info.get("picture").and_then(|p| {
                if let Value::Map(pic) = p {
                    pic.get("data").and_then(|d| {
                        if let Value::Map(data) = d {
                            data.get("url").and_then(|u| {
                                if let Value::String(s) = u {
                                    Some(s.clone())
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
                }
            });
            (
                get_string("id").unwrap_or_default(),
                get_string("email"),
                get_string("name"),
                picture,
            )
        }
        "microsoft" => (
            get_string("id")
                .or_else(|| get_string("sub"))
                .unwrap_or_default(),
            get_string("mail")
                .or_else(|| get_string("email"))
                .or_else(|| get_string("userPrincipalName")),
            get_string("displayName").or_else(|| get_string("name")),
            None, // Microsoft Graph requires separate call for photo
        ),
        "discord" => {
            let avatar = get_string("avatar").map(|a| {
                let id = get_string("id").unwrap_or_default();
                format!("https://cdn.discordapp.com/avatars/{}/{}.png", id, a)
            });
            (
                get_string("id").unwrap_or_default(),
                get_string("email"),
                get_string("username"),
                avatar,
            )
        }
        "twitter" => {
            // Twitter v2 API nests user data
            let data = info.get("data").and_then(|d| {
                if let Value::Map(m) = d {
                    Some(m.clone())
                } else {
                    None
                }
            });
            if let Some(d) = data {
                let get_from_data = |key: &str| -> Option<String> {
                    d.get(key).and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                };
                (
                    get_from_data("id").unwrap_or_default(),
                    None, // Twitter doesn't provide email easily
                    get_from_data("name"),
                    get_from_data("profile_image_url"),
                )
            } else {
                (
                    get_string("id").unwrap_or_default(),
                    None,
                    get_string("name"),
                    None,
                )
            }
        }
        "linkedin" => (
            get_string("sub").unwrap_or_default(),
            get_string("email"),
            get_string("name"),
            get_string("picture"),
        ),
        "apple" => (
            get_string("sub").unwrap_or_default(),
            get_string("email"),
            // Apple sends name in a nested object on first auth only
            get_string("name").or_else(|| {
                info.get("name").and_then(|n| {
                    if let Value::Map(name_map) = n {
                        let first = name_map.get("firstName").and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        let last = name_map.get("lastName").and_then(|v| {
                            if let Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        });
                        match (first, last) {
                            (Some(f), Some(l)) => Some(format!("{} {}", f, l)),
                            (Some(f), None) => Some(f),
                            (None, Some(l)) => Some(l),
                            _ => None,
                        }
                    } else {
                        None
                    }
                })
            }),
            None,
        ),
        _ => (
            get_string("id")
                .or_else(|| get_string("sub"))
                .unwrap_or_default(),
            get_string("email"),
            get_string("name"),
            get_string("picture").or_else(|| get_string("avatar")),
        ),
    }
}

// ============================================================================
// SECTION 4: PKCE Support
// ============================================================================

/// Generate PKCE code verifier (43-128 character random string)
pub fn generate_pkce_verifier() -> String {
    let bytes: Vec<u8> = (0..32).map(|_| rand::random::<u8>()).collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
}

/// Generate PKCE code challenge from verifier (S256 method)
pub fn generate_pkce_challenge(verifier: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&hash)
}

// ============================================================================
// SECTION 5: OIDC Discovery
// ============================================================================

/// Fetch OIDC discovery document from issuer
pub fn fetch_oidc_discovery(issuer: &str) -> Result<OidcDiscovery> {
    let discovery_url = format!(
        "{}/.well-known/openid-configuration",
        issuer.trim_end_matches('/')
    );

    let client = reqwest::blocking::Client::new();
    let response = client
        .get(&discovery_url)
        .header("Accept", "application/json")
        .send()
        .map_err(|e| IntentError::runtime_error(format!("[auth] OIDC discovery failed: {}", e)))?;

    let body = response.text().map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to read discovery response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to parse discovery document: {}", e))
    })?;

    let get_str = |key: &str| -> Option<String> {
        json.get(key)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };

    let get_arr = |key: &str| -> Vec<String> {
        json.get(key)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default()
    };

    Ok(OidcDiscovery {
        issuer: get_str("issuer").ok_or_else(|| {
            IntentError::runtime_error("[auth] Discovery missing issuer".to_string())
        })?,
        authorization_endpoint: get_str("authorization_endpoint").ok_or_else(|| {
            IntentError::runtime_error(
                "[auth] Discovery missing authorization_endpoint".to_string(),
            )
        })?,
        token_endpoint: get_str("token_endpoint").ok_or_else(|| {
            IntentError::runtime_error("[auth] Discovery missing token_endpoint".to_string())
        })?,
        userinfo_endpoint: get_str("userinfo_endpoint"),
        jwks_uri: get_str("jwks_uri").ok_or_else(|| {
            IntentError::runtime_error("[auth] Discovery missing jwks_uri".to_string())
        })?,
        scopes_supported: get_arr("scopes_supported"),
        response_types_supported: get_arr("response_types_supported"),
        token_endpoint_auth_methods_supported: get_arr("token_endpoint_auth_methods_supported"),
    })
}

// ============================================================================
// SECTION 6: OAuth Flow
// ============================================================================

/// Generate OAuth authorization URL
pub fn generate_auth_url(
    provider: &ProviderConfig,
    redirect_uri: &str,
    state: &str,
    nonce: Option<&str>,
    pkce_challenge: Option<&str>,
) -> String {
    let scopes = provider.scopes.join(" ");
    let mut url = format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        provider.authorize_url,
        urlencoding::encode(&provider.client_id),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(&scopes),
        urlencoding::encode(state)
    );

    // Add nonce for OIDC
    if let Some(n) = nonce {
        url.push_str(&format!("&nonce={}", urlencoding::encode(n)));
    }

    // Add PKCE challenge
    if let Some(challenge) = pkce_challenge {
        url.push_str(&format!(
            "&code_challenge={}&code_challenge_method=S256",
            urlencoding::encode(challenge)
        ));
    }

    // Add extra params
    for (key, value) in &provider.extra_params {
        url.push_str(&format!(
            "&{}={}",
            urlencoding::encode(key),
            urlencoding::encode(value)
        ));
    }

    url
}

/// Exchange authorization code for tokens
pub fn exchange_code_for_tokens(
    provider: &ProviderConfig,
    code: &str,
    redirect_uri: &str,
    pkce_verifier: Option<&str>,
) -> Result<TokenResponse> {
    let client = reqwest::blocking::Client::new();

    let mut params = vec![
        ("client_id", provider.client_id.as_str()),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
    ];

    // Don't send client_secret for public clients (PKCE)
    if !provider.client_secret.is_empty() {
        params.push(("client_secret", provider.client_secret.as_str()));
    }

    // Add PKCE verifier
    let verifier_owned;
    if let Some(v) = pkce_verifier {
        verifier_owned = v.to_string();
        params.push(("code_verifier", verifier_owned.as_str()));
    }

    let response = client
        .post(&provider.token_url)
        .header("Accept", "application/json")
        .header("User-Agent", "NTNT/0.3.13") // Required by GitHub
        .form(&params)
        .send()
        .map_err(|e| IntentError::runtime_error(format!("[auth] Token exchange failed: {}", e)))?;

    let body = response.text().map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to read token response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::runtime_error(format!(
            "[auth] Failed to parse token response: {} - Body: {}",
            e, body
        ))
    })?;

    // Check for error in response
    if let Some(error) = json.get("error") {
        let error_desc = json
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Err(IntentError::runtime_error(format!(
            "[auth] OAuth error: {} - {}",
            error.as_str().unwrap_or("unknown"),
            error_desc
        )));
    }

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            IntentError::runtime_error(format!("[auth] No access_token in response: {}", body))
        })?;

    // Default expires_in to 1 hour if not provided (security: don't allow infinite-lived tokens)
    let expires_in = json
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .or(Some(3600)); // Default 1 hour

    Ok(TokenResponse {
        access_token,
        token_type: json
            .get("token_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Bearer")
            .to_string(),
        expires_in,
        refresh_token: json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        id_token: json
            .get("id_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        scope: json
            .get("scope")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Refresh access token using refresh token
pub fn refresh_access_token(
    provider: &ProviderConfig,
    refresh_token: &str,
) -> Result<TokenResponse> {
    let client = reqwest::blocking::Client::new();

    let params = [
        ("client_id", provider.client_id.as_str()),
        ("client_secret", provider.client_secret.as_str()),
        ("refresh_token", refresh_token),
        ("grant_type", "refresh_token"),
    ];

    let response = client
        .post(&provider.token_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .map_err(|e| IntentError::runtime_error(format!("[auth] Token refresh failed: {}", e)))?;

    let body = response.text().map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to read refresh response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to parse refresh response: {}", e))
    })?;

    if let Some(error) = json.get("error") {
        let error_desc = json
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Err(IntentError::runtime_error(format!(
            "[auth] Refresh error: {} - {}",
            error.as_str().unwrap_or("unknown"),
            error_desc
        )));
    }

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            IntentError::runtime_error("[auth] No access_token in refresh response".to_string())
        })?;

    // Default expires_in to 1 hour if not provided
    let expires_in = json
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .or(Some(3600));

    Ok(TokenResponse {
        access_token,
        token_type: json
            .get("token_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Bearer")
            .to_string(),
        expires_in,
        refresh_token: json
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .or_else(|| Some(refresh_token.to_string())), // Keep old refresh token if not returned
        id_token: json
            .get("id_token")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        scope: json
            .get("scope")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Client credentials flow for M2M authentication
pub fn client_credentials_grant(
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    scopes: &[String],
) -> Result<TokenResponse> {
    let client = reqwest::blocking::Client::new();

    let scope = scopes.join(" ");
    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("grant_type", "client_credentials"),
        ("scope", &scope),
    ];

    let response = client
        .post(token_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .map_err(|e| {
            IntentError::runtime_error(format!("[auth] Client credentials grant failed: {}", e))
        })?;

    let body = response.text().map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to read token response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to parse token response: {}", e))
    })?;

    if let Some(error) = json.get("error") {
        let error_desc = json
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Err(IntentError::runtime_error(format!(
            "[auth] Client credentials error: {} - {}",
            error.as_str().unwrap_or("unknown"),
            error_desc
        )));
    }

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| {
            IntentError::runtime_error("[auth] No access_token in response".to_string())
        })?;

    // Default expires_in to 1 hour if not provided
    let expires_in = json
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .or(Some(3600));

    Ok(TokenResponse {
        access_token,
        token_type: json
            .get("token_type")
            .and_then(|v| v.as_str())
            .unwrap_or("Bearer")
            .to_string(),
        expires_in,
        refresh_token: None,
        id_token: None,
        scope: json
            .get("scope")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
    })
}

/// Decode ID token claims (without full signature verification)
pub fn decode_id_token(id_token: &str) -> Result<HashMap<String, Value>> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 {
        return Err(IntentError::runtime_error(
            "[auth] Invalid ID token format".to_string(),
        ));
    }

    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| IntentError::runtime_error(format!("[auth] ID token decode error: {}", e)))?;

    let json: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|e| IntentError::runtime_error(format!("[auth] ID token parse error: {}", e)))?;

    json_to_value_map(&json)
}

/// Validate ID token claims (basic validation without signature verification)
pub fn validate_id_token_claims(
    claims: &HashMap<String, Value>,
    expected_issuer: Option<&str>,
    expected_audience: &str,
    expected_nonce: Option<&str>,
) -> Result<()> {
    // Validate issuer
    if let Some(expected_iss) = expected_issuer {
        let iss = claims
            .get("iss")
            .and_then(|v| {
                if let Value::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                IntentError::runtime_error("[auth] ID token missing issuer".to_string())
            })?;

        if iss != expected_iss {
            return Err(IntentError::runtime_error(format!(
                "[auth] ID token issuer mismatch: expected {}, got {}",
                expected_iss, iss
            )));
        }
    }

    // Validate audience
    let aud = claims.get("aud");
    let aud_valid = match aud {
        Some(Value::String(s)) => s == expected_audience,
        Some(Value::Array(arr)) => arr.iter().any(|v| {
            if let Value::String(s) = v {
                s == expected_audience
            } else {
                false
            }
        }),
        _ => false,
    };
    if !aud_valid {
        return Err(IntentError::runtime_error(
            "[auth] ID token audience mismatch".to_string(),
        ));
    }

    // Validate nonce using constant-time comparison to prevent timing attacks
    if let Some(expected_n) = expected_nonce {
        let nonce = claims
            .get("nonce")
            .and_then(|v| {
                if let Value::String(s) = v {
                    Some(s.as_str())
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                IntentError::runtime_error("[auth] ID token missing nonce".to_string())
            })?;

        if !constant_time_compare(nonce, expected_n) {
            return Err(IntentError::runtime_error(
                "[auth] ID token nonce mismatch (possible replay attack)".to_string(),
            ));
        }
    }

    // Validate expiry
    let exp = claims
        .get("exp")
        .and_then(|v| {
            if let Value::Int(i) = v {
                Some(*i)
            } else {
                None
            }
        })
        .ok_or_else(|| IntentError::runtime_error("[auth] ID token missing expiry".to_string()))?;

    let now = chrono::Utc::now().timestamp();
    if now > exp {
        return Err(IntentError::runtime_error(
            "[auth] ID token expired".to_string(),
        ));
    }

    Ok(())
}

/// Fetch user info from provider
pub fn fetch_userinfo(
    provider: &ProviderConfig,
    access_token: &str,
) -> Result<HashMap<String, Value>> {
    // Apple doesn't have a userinfo endpoint - user info is in the ID token
    if provider.userinfo_url.is_empty() {
        return Ok(HashMap::new());
    }

    let client = reqwest::blocking::Client::new();

    let response = client
        .get(&provider.userinfo_url)
        .header("Authorization", format!("Bearer {}", access_token))
        .header("Accept", "application/json")
        .header("User-Agent", "NTNT/0.3.13") // Required by GitHub
        .send()
        .map_err(|e| {
            IntentError::runtime_error(format!("[auth] Userinfo request failed: {}", e))
        })?;

    let body = response.text().map_err(|e| {
        IntentError::runtime_error(format!("[auth] Failed to read userinfo response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::runtime_error(format!(
            "[auth] Failed to parse userinfo: {} - Body: {}",
            e, body
        ))
    })?;

    json_to_value_map(&json)
}

/// Token introspection (RFC 7662)
pub fn introspect_token(
    introspection_url: &str,
    token: &str,
    client_id: &str,
    client_secret: &str,
) -> Result<HashMap<String, Value>> {
    let client = reqwest::blocking::Client::new();

    let params = [
        ("token", token),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ];

    let response = client
        .post(introspection_url)
        .header("Accept", "application/json")
        .form(&params)
        .send()
        .map_err(|e| {
            IntentError::runtime_error(format!("[auth] Token introspection failed: {}", e))
        })?;

    let body = response.text().map_err(|e| {
        IntentError::runtime_error(format!(
            "[auth] Failed to read introspection response: {}",
            e
        ))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::runtime_error(format!(
            "[auth] Failed to parse introspection response: {}",
            e
        ))
    })?;

    json_to_value_map(&json)
}

// ============================================================================
// SECTION 7: Session Management
// ============================================================================

/// Session info for listing (excludes sensitive token data)
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub is_current: bool,
}

/// In-memory session store (used as fallback or when session_store = Memory)
#[allow(dead_code)]
struct InMemoryStore {
    sessions: HashMap<String, Session>,
    oauth_states: HashMap<String, OAuthState>,
    exchange_tokens: HashMap<String, (String, i64)>, // token → (session_id, created_at)
    auth_challenges: HashMap<String, AuthChallenge>,
}

impl InMemoryStore {
    fn new() -> Self {
        InMemoryStore {
            sessions: HashMap::new(),
            oauth_states: HashMap::new(),
            exchange_tokens: HashMap::new(),
            auth_challenges: HashMap::new(),
        }
    }

    fn get_session(&self, id: &str) -> Option<&Session> {
        self.sessions.get(id).filter(|s| {
            let now = chrono::Utc::now().timestamp();
            s.expires_at > now
        })
    }

    fn get_session_mut(&mut self, id: &str) -> Option<&mut Session> {
        let now = chrono::Utc::now().timestamp();
        self.sessions.get_mut(id).filter(|s| s.expires_at > now)
    }

    fn set_session(&mut self, session: Session) {
        self.sessions.insert(session.id.clone(), session);
    }

    fn delete_session(&mut self, id: &str) {
        self.sessions.remove(id);
    }

    fn get_oauth_state(&self, state: &str) -> Option<&OAuthState> {
        self.oauth_states.get(state).filter(|s| {
            let now = chrono::Utc::now().timestamp();
            now - s.created_at < 600 // 10 minutes
        })
    }

    fn set_oauth_state(&mut self, state: OAuthState) {
        self.oauth_states.insert(state.state.clone(), state);
    }

    fn delete_oauth_state(&mut self, state: &str) {
        self.oauth_states.remove(state);
    }

    fn set_exchange_token(&mut self, token: String, session_id: String) {
        let now = chrono::Utc::now().timestamp();
        self.exchange_tokens.insert(token, (session_id, now));
    }

    fn get_exchange_token(&self, token: &str) -> Option<&String> {
        self.exchange_tokens
            .get(token)
            .and_then(|(session_id, created_at)| {
                let now = chrono::Utc::now().timestamp();
                if now - created_at < EXCHANGE_TOKEN_TTL {
                    Some(session_id)
                } else {
                    None
                }
            })
    }

    fn delete_exchange_token(&mut self, token: &str) {
        self.exchange_tokens.remove(token);
    }

    fn set_auth_challenge(&mut self, challenge: AuthChallenge) {
        self.auth_challenges.insert(challenge.id.clone(), challenge);
    }

    fn delete_auth_challenge(&mut self, id: &str) {
        self.auth_challenges.remove(id);
    }

    fn take_auth_challenge(&mut self, id: &str) -> Option<AuthChallenge> {
        let now = chrono::Utc::now().timestamp();
        match self.auth_challenges.get(id) {
            Some(challenge) if challenge.expires_at > now => self.auth_challenges.remove(id),
            Some(_) => {
                self.auth_challenges.remove(id);
                None
            }
            _ => None,
        }
    }

    fn cleanup_expired_exchange_tokens(&mut self, now: i64) -> usize {
        let cutoff = now - EXCHANGE_TOKEN_TTL;
        let before = self.exchange_tokens.len();
        self.exchange_tokens
            .retain(|_, (_, created_at)| *created_at >= cutoff);
        before - self.exchange_tokens.len()
    }

    fn cleanup_expired_auth_challenges(&mut self, now: i64) -> usize {
        let before = self.auth_challenges.len();
        self.auth_challenges
            .retain(|_, challenge| challenge.expires_at >= now);
        before - self.auth_challenges.len()
    }

    fn cleanup_expired(&mut self, now: i64) -> usize {
        let before = self.sessions.len();
        self.sessions.retain(|_, s| s.expires_at >= now);
        before - self.sessions.len()
    }

    fn cleanup_expired_oauth_states(&mut self, cutoff: i64) -> usize {
        let before = self.oauth_states.len();
        self.oauth_states.retain(|_, s| s.created_at >= cutoff);
        before - self.oauth_states.len()
    }

    fn get_sessions_for_user(
        &self,
        user_id: &str,
        current_session_id: Option<&str>,
        now: i64,
    ) -> Vec<SessionInfo> {
        let mut sessions: Vec<SessionInfo> = self
            .sessions
            .values()
            .filter(|s| s.user_id == user_id && s.expires_at > now)
            .map(|s| SessionInfo {
                id: s.id.clone(),
                user_id: s.user_id.clone(),
                provider: s.provider.clone(),
                created_at: s.created_at,
                expires_at: s.expires_at,
                is_current: current_session_id.map(|c| c == s.id).unwrap_or(false),
            })
            .collect();

        // Sort by created_at descending
        sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        sessions
    }

    fn delete_all_sessions_for_user(
        &mut self,
        user_id: &str,
        keep_session_id: Option<&str>,
    ) -> usize {
        let before = self.sessions.len();
        self.sessions.retain(|id, s| {
            // Keep if not this user, or if it's the session we want to keep
            s.user_id != user_id || keep_session_id.map(|k| k == id).unwrap_or(false)
        });
        before - self.sessions.len()
    }
}

// Global in-memory session store (always available as fallback)
static SESSION_STORE: std::sync::LazyLock<Arc<Mutex<InMemoryStore>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(InMemoryStore::new())));

// Database connections for session storage
static SQLITE_CONN: std::sync::LazyLock<Arc<Mutex<Option<rusqlite::Connection>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

// Note: Postgres uses connection pooling, so we store the URL and connect per-request
static POSTGRES_URL: std::sync::LazyLock<Arc<Mutex<Option<String>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

// Redis URL for session storage
static REDIS_URL: std::sync::LazyLock<Arc<Mutex<Option<String>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

// Global auth config
static AUTH_CONFIG: std::sync::LazyLock<Arc<Mutex<Option<AuthConfig>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(None)));

// Protected path patterns registered via enable_auth(..., map { "protected_paths": [...] })
// and require_auth("/admin/*") helper calls.
static AUTH_PROTECTED_PATHS: std::sync::LazyLock<Arc<Mutex<Vec<String>>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));

pub fn get_auth_config() -> Option<AuthConfig> {
    AUTH_CONFIG.lock().unwrap().clone()
}

/// Generate a secure session ID
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt oauth
    // @module std/auth
    // @module_description Full OAuth 2.0 and OIDC authentication with JWT support
    // @signature oauth(provider: String, client_id: String, client_secret: String, options?: Map) -> Provider
    // Create an OAuth provider configuration.
    //
    // Supports built-in providers (google, github, facebook, microsoft, discord,
    // twitter, linkedin, apple) with sensible defaults, or custom providers
    // with full configuration. Supports OIDC (ID tokens, nonce validation) and PKCE.
    // @param provider Provider name (e.g., "google", "github") or custom name
    // @param client_id OAuth client ID (or config map for custom providers)
    // @param client_secret OAuth client secret (omit for PKCE public clients)
    // @param options Optional map: scopes, use_pkce, access_type, prompt
    // @returns Provider configuration to pass to enable_auth()
    // @see_also enable_auth, get_user, oauth_pkce
    // @since v0.3.11
    // @tags #auth, #oauth, #oidc
    // @example oauth("google", "client_id", "client_secret") => Provider ~ "Google OAuth with defaults"
    // @example oauth("github", "id", "secret", map { "scopes": ["repo"] }) => Provider ~ "GitHub with custom scopes"
    // @example oauth("google", "id", "secret", map { "use_pkce": true }) => Provider ~ "Google with PKCE"
    module.insert(
        "oauth".to_string(),
        Value::NativeFunction {
            name: "oauth".to_string(),
            arity: 0,  // Variadic: 2-4 args (provider, client_id, client_secret?, options?)
            max_arity: 0,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] oauth() requires at least a provider name".to_string()
                    ));
                }

                let provider_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::type_error(
                        "[auth] oauth() first argument must be a provider name string".to_string()
                    )),
                };

                // Check second argument type to determine signature
                match args.get(1) {
                    Some(Value::String(client_id)) => {
                        // Signature: oauth(provider, client_id, client_secret, options?)
                        let client_secret = match args.get(2) {
                            Some(Value::String(s)) => s.clone(),
                            Some(_) => return Err(IntentError::type_error(
                                "[auth] oauth() client_secret must be a string".to_string()
                            )),
                            None => String::new(), // Allow empty for PKCE public clients
                        };

                        let options = match args.get(3) {
                            Some(Value::Map(m)) => Some(m.clone()),
                            Some(_) => return Err(IntentError::type_error(
                                "[auth] oauth() options must be a map".to_string()
                            )),
                            None => None,
                        };

                        // Look up built-in provider
                        let builtin = get_builtin_provider(&provider_name);

                        let (authorize_url, token_url, userinfo_url, default_scopes, issuer, supports_oidc, supports_pkce) =
                            if let Some(b) = builtin {
                                (
                                    b.authorize_url.to_string(),
                                    b.token_url.to_string(),
                                    b.userinfo_url.to_string(),
                                    b.default_scopes.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
                                    b.issuer.map(|s| s.to_string()),
                                    b.supports_oidc,
                                    b.supports_pkce,
                                )
                            } else {
                                // Unknown provider - require full config
                                let msg = if let Some(suggestion) = suggest_provider(&provider_name) {
                                    format!(
                                        "[auth] Unknown provider \"{}\"\n       Did you mean \"{}\"?\n       For custom providers, use: oauth(\"{}\", map {{ \"client_id\": ..., \"authorize_url\": ... }})",
                                        provider_name, suggestion, provider_name
                                    )
                                } else {
                                    format!(
                                        "[auth] Unknown provider \"{}\"\n       Available: {}\n       For custom providers, use: oauth(\"{}\", map {{ \"client_id\": ..., \"authorize_url\": ... }})",
                                        provider_name, available_providers(), provider_name
                                    )
                                };
                                return Err(IntentError::runtime_error(msg));
                            };

                        // Check if PKCE is explicitly requested or required
                        let use_pkce = options
                            .as_ref()
                            .and_then(|o| o.get("use_pkce"))
                            .and_then(|v| match v { Value::Bool(b) => Some(*b), _ => None })
                            .unwrap_or(provider_name == "twitter"); // Twitter requires PKCE

                        if use_pkce && !supports_pkce {
                            return Err(IntentError::runtime_error(format!(
                                "[auth] Provider \"{}\" does not support PKCE",
                                provider_name
                            )));
                        }

                        // Override scopes if provided in options
                        let scopes = options
                            .as_ref()
                            .and_then(|o| o.get("scopes"))
                            .and_then(|v| match v {
                                Value::Array(arr) => Some(
                                    arr.iter()
                                        .filter_map(|v| match v {
                                            Value::String(s) => Some(s.clone()),
                                            _ => None,
                                        })
                                        .collect()
                                ),
                                _ => None,
                            })
                            .unwrap_or(default_scopes);

                        // Extract extra params from options
                        let mut extra_params = HashMap::new();
                        if let Some(opts) = &options {
                            for (key, value) in opts {
                                if key != "scopes" && key != "use_pkce" {
                                    if let Value::String(v) = value {
                                        extra_params.insert(key.clone(), v.clone());
                                    }
                                }
                            }
                        }

                        let config = ProviderConfig {
                            name: provider_name,
                            client_id: client_id.clone(),
                            client_secret,
                            authorize_url,
                            token_url,
                            userinfo_url,
                            issuer,
                            jwks_uri: None,
                            scopes,
                            extra_params,
                            use_pkce,
                            supports_oidc,
                        };

                        Ok(provider_to_value(&config))
                    }
                    Some(Value::Map(config_map)) => {
                        // Signature: oauth(name, config_map) - custom provider
                        let get_str = |key: &str| -> Option<String> {
                            config_map.get(key).and_then(|v| {
                                if let Value::String(s) = v { Some(s.clone()) } else { None }
                            })
                        };

                        let get_bool = |key: &str, default: bool| -> bool {
                            config_map.get(key).and_then(|v| {
                                if let Value::Bool(b) = v { Some(*b) } else { None }
                            }).unwrap_or(default)
                        };

                        let client_id = get_str("client_id").ok_or_else(|| {
                            IntentError::type_error(format!(
                                "[auth] Custom provider \"{}\" missing required field \"client_id\"",
                                provider_name
                            ))
                        })?;

                        let client_secret = get_str("client_secret").unwrap_or_default();

                        let authorize_url = get_str("authorize_url").ok_or_else(|| {
                            IntentError::type_error(format!(
                                "[auth] Custom provider \"{}\" missing required field \"authorize_url\"",
                                provider_name
                            ))
                        })?;

                        let token_url = get_str("token_url").ok_or_else(|| {
                            IntentError::type_error(format!(
                                "[auth] Custom provider \"{}\" missing required field \"token_url\"",
                                provider_name
                            ))
                        })?;

                        let userinfo_url = get_str("userinfo_url").unwrap_or_default();
                        let issuer = get_str("issuer");
                        let jwks_uri = get_str("jwks_uri");

                        let scopes = config_map
                            .get("scopes")
                            .and_then(|v| match v {
                                Value::Array(arr) => Some(
                                    arr.iter()
                                        .filter_map(|v| match v {
                                            Value::String(s) => Some(s.clone()),
                                            _ => None,
                                        })
                                        .collect()
                                ),
                                _ => None,
                            })
                            .unwrap_or_else(|| vec!["openid".to_string(), "email".to_string(), "profile".to_string()]);

                        let supports_oidc = get_bool("supports_oidc", issuer.is_some());
                        let config = ProviderConfig {
                            name: provider_name,
                            client_id,
                            client_secret,
                            authorize_url,
                            token_url,
                            userinfo_url,
                            issuer,
                            jwks_uri,
                            scopes,
                            extra_params: HashMap::new(),
                            use_pkce: get_bool("use_pkce", false),
                            supports_oidc,
                        };

                        Ok(provider_to_value(&config))
                    }
                    Some(_) => Err(IntentError::type_error(
                        "[auth] oauth() second argument must be client_id (string) or config (map)".to_string()
                    )),
                    None => Err(IntentError::type_error(
                        "[auth] oauth() requires credentials or configuration".to_string()
                    )),
                }
            },
        },
    );

    // @ntnt oauth_discover
    // @module std/auth
    // @signature oauth_discover(issuer: String, client_id: String, client_secret?: String, options?: Map) -> Result<Provider, String>
    // Create an OAuth provider using OIDC Discovery.
    //
    // Automatically fetches configuration from the issuer's .well-known/openid-configuration
    // endpoint. Useful for Okta, Auth0, Keycloak, and other OIDC providers.
    // @param issuer The OIDC issuer URL (e.g., "https://mycompany.okta.com")
    // @param client_id OAuth client ID
    // @param client_secret OAuth client secret (optional for PKCE)
    // @param options Optional map: scopes, use_pkce
    // @returns Result containing Provider or error message
    // @see_also oauth, enable_auth
    // @since v0.3.11
    // @tags #auth, #oidc, #discovery
    // @example oauth_discover("https://mycompany.okta.com", "client_id", "secret") => Ok(Provider) ~ "Okta with auto-discovery"
    module.insert(
        "oauth_discover".to_string(),
        Value::NativeFunction {
            name: "oauth_discover".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] oauth_discover() requires issuer and client_id".to_string(),
                    ));
                }

                let issuer = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] oauth_discover() issuer must be a string".to_string(),
                        ))
                    }
                };

                let client_id = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] oauth_discover() client_id must be a string".to_string(),
                        ))
                    }
                };

                let client_secret = args
                    .get(2)
                    .and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                let options = args.get(3).and_then(|v| {
                    if let Value::Map(m) = v {
                        Some(m.clone())
                    } else {
                        None
                    }
                });

                // Fetch OIDC discovery
                let discovery = match fetch_oidc_discovery(&issuer) {
                    Ok(d) => d,
                    Err(e) => return Ok(make_err(Value::String(e.to_string()))),
                };

                let use_pkce = options
                    .as_ref()
                    .and_then(|o| o.get("use_pkce"))
                    .and_then(|v| {
                        if let Value::Bool(b) = v {
                            Some(*b)
                        } else {
                            None
                        }
                    })
                    .unwrap_or(false);

                let scopes = options
                    .as_ref()
                    .and_then(|o| o.get("scopes"))
                    .and_then(|v| match v {
                        Value::Array(arr) => Some(
                            arr.iter()
                                .filter_map(|v| {
                                    if let Value::String(s) = v {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_else(|| {
                        vec![
                            "openid".to_string(),
                            "email".to_string(),
                            "profile".to_string(),
                        ]
                    });

                let provider_name = issuer
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('.')
                    .next()
                    .unwrap_or("custom")
                    .to_string();

                let config = ProviderConfig {
                    name: provider_name,
                    client_id,
                    client_secret,
                    authorize_url: discovery.authorization_endpoint,
                    token_url: discovery.token_endpoint,
                    userinfo_url: discovery.userinfo_endpoint.unwrap_or_default(),
                    issuer: Some(discovery.issuer),
                    jwks_uri: Some(discovery.jwks_uri),
                    scopes,
                    extra_params: HashMap::new(),
                    use_pkce,
                    supports_oidc: true,
                };

                Ok(make_ok(provider_to_value(&config)))
            },
        },
    );

    // @ntnt oauth_m2m
    // @module std/auth
    // @signature oauth_m2m(token_url: String, client_id: String, client_secret: String, scopes: [String]) -> Result<Map, String>
    // Get an access token using client credentials grant (M2M authentication).
    //
    // Used for server-to-server API calls where no user is involved.
    // @param token_url The token endpoint URL
    // @param client_id OAuth client ID
    // @param client_secret OAuth client secret
    // @param scopes Array of scopes to request
    // @returns Result containing token response map or error
    // @see_also oauth, oauth_refresh
    // @since v0.3.11
    // @tags #auth, #oauth, #m2m
    // @example oauth_m2m("https://oauth.example.com/token", "id", "secret", ["api.read"]) => Ok({access_token: "...", ...}) ~ "Get M2M token"
    module.insert(
        "oauth_m2m".to_string(),
        Value::NativeFunction {
            name: "oauth_m2m".to_string(),
            arity: 4,
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 4 {
                    return Err(IntentError::type_error(
                        "[auth] oauth_m2m() requires token_url, client_id, client_secret, scopes"
                            .to_string(),
                    ));
                }

                let token_url = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] token_url must be a string".to_string(),
                        ))
                    }
                };
                let client_id = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] client_id must be a string".to_string(),
                        ))
                    }
                };
                let client_secret = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] client_secret must be a string".to_string(),
                        ))
                    }
                };
                let scopes: Vec<String> = match &args[3] {
                    Value::Array(arr) => arr
                        .iter()
                        .filter_map(|v| {
                            if let Value::String(s) = v {
                                Some(s.clone())
                            } else {
                                None
                            }
                        })
                        .collect(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] scopes must be an array".to_string(),
                        ))
                    }
                };

                match client_credentials_grant(&token_url, &client_id, &client_secret, &scopes) {
                    Ok(tokens) => {
                        let mut map = HashMap::new();
                        map.insert(
                            "access_token".to_string(),
                            Value::String(tokens.access_token),
                        );
                        map.insert("token_type".to_string(), Value::String(tokens.token_type));
                        if let Some(exp) = tokens.expires_in {
                            map.insert("expires_in".to_string(), Value::Int(exp));
                        }
                        if let Some(scope) = tokens.scope {
                            map.insert("scope".to_string(), Value::String(scope));
                        }
                        Ok(make_ok(Value::Map(map)))
                    }
                    Err(e) => Ok(make_err(Value::String(e.to_string()))),
                }
            },
        },
    );

    // @ntnt oauth_refresh
    // @module std/auth
    // @signature oauth_refresh(req: Request) -> Result<Map, String>
    // Refresh the access token for the current session.
    //
    // Uses the stored refresh token to get a new access token. Updates the session
    // with new tokens. Requires enable_auth() with store_tokens: true.
    // @param req The HTTP request object
    // @returns Result containing new token info or error
    // @see_also get_session, oauth
    // @since v0.3.11
    // @tags #auth, #oauth
    // @example oauth_refresh(req) => Ok({access_token: "...", expires_in: 3600}) ~ "Refresh tokens"
    module.insert(
        "oauth_refresh".to_string(),
        Value::NativeFunction {
            name: "oauth_refresh".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] oauth_refresh() requires a request".to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error("[auth] Auth not configured".to_string())
                })?;

                let session_id = get_session_id_from_request(&args[0]).ok_or_else(|| {
                    IntentError::runtime_error("[auth] No session found".to_string())
                })?;

                let session = get_session_by_id(&session_id).ok_or_else(|| {
                    IntentError::runtime_error("[auth] Session expired".to_string())
                })?;

                let refresh_token = session.refresh_token.as_ref().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] No refresh token stored (enable store_tokens in auth config)"
                            .to_string(),
                    )
                })?;

                let provider = config
                    .providers
                    .iter()
                    .find(|p| p.name == session.provider)
                    .ok_or_else(|| {
                        IntentError::runtime_error(format!(
                            "[auth] Provider {} not found",
                            session.provider
                        ))
                    })?;

                match refresh_access_token(provider, refresh_token) {
                    Ok(tokens) => {
                        // Update session with new tokens
                        update_session_tokens(&session_id, &tokens);

                        let mut map = HashMap::new();
                        map.insert(
                            "access_token".to_string(),
                            Value::String(tokens.access_token),
                        );
                        map.insert("token_type".to_string(), Value::String(tokens.token_type));
                        if let Some(exp) = tokens.expires_in {
                            map.insert("expires_in".to_string(), Value::Int(exp));
                        }
                        Ok(make_ok(Value::Map(map)))
                    }
                    Err(e) => Ok(make_err(Value::String(e.to_string()))),
                }
            },
        },
    );

    // @ntnt oauth_validate
    // @module std/auth
    // @signature oauth_validate(token: String, options: Map) -> Result<Map, String>
    // Validate an incoming bearer token (for APIs acting as resource servers).
    //
    // Decodes and validates the token claims without calling the provider.
    // For full validation, use oauth_introspect().
    // @param token The bearer token to validate
    // @param options Map with issuer, audience for validation
    // @returns Result containing token claims or error
    // @see_also oauth_introspect, jwt_verify
    // @since v0.3.11
    // @tags #auth, #oauth, #validation
    // @example oauth_validate(token, map { "issuer": "https://...", "audience": "my-api" }) ~ "Validate bearer token"
    module.insert(
        "oauth_validate".to_string(),
        Value::NativeFunction {
            name: "oauth_validate".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] oauth_validate() requires token and options".to_string(),
                    ));
                }

                let token = match &args[0] {
                    Value::String(s) => {
                        // Handle "Bearer <token>" format
                        if s.to_lowercase().starts_with("bearer ") {
                            s[7..].to_string()
                        } else {
                            s.clone()
                        }
                    }
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] token must be a string".to_string(),
                        ))
                    }
                };

                let options = match &args[1] {
                    Value::Map(m) => m.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] options must be a map".to_string(),
                        ))
                    }
                };

                // Decode the token
                let claims = match decode_id_token(&token) {
                    Ok(c) => c,
                    Err(e) => return Ok(make_err(Value::String(e.to_string()))),
                };

                let issuer = options.get("issuer").and_then(|v| {
                    if let Value::String(s) = v {
                        Some(s.as_str())
                    } else {
                        None
                    }
                });
                let audience = options
                    .get("audience")
                    .and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                // Validate claims
                if let Err(e) = validate_id_token_claims(&claims, issuer, &audience, None) {
                    return Ok(make_err(Value::String(e.to_string())));
                }

                Ok(make_ok(Value::Map(claims)))
            },
        },
    );

    // @ntnt oauth_introspect
    // @module std/auth
    // @signature oauth_introspect(introspection_url: String, token: String, client_id: String, client_secret: String) -> Result<Map, String>
    // Introspect a token using the provider's introspection endpoint (RFC 7662).
    //
    // Calls the authorization server to validate the token. More reliable than
    // local validation but adds network latency.
    // @param introspection_url The introspection endpoint URL
    // @param token The token to introspect
    // @param client_id OAuth client ID
    // @param client_secret OAuth client secret
    // @returns Result containing introspection response or error
    // @see_also oauth_validate
    // @since v0.3.11
    // @tags #auth, #oauth, #introspection
    // @example oauth_introspect("https://auth.example.com/introspect", token, "id", "secret") ~ "Introspect token"
    module.insert(
        "oauth_introspect".to_string(),
        Value::NativeFunction {
            name: "oauth_introspect".to_string(),
            arity: 4,
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 4 {
                    return Err(IntentError::type_error(
                        "[auth] oauth_introspect() requires introspection_url, token, client_id, client_secret".to_string()
                    ));
                }

                let introspection_url = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::type_error("[auth] introspection_url must be a string".to_string())),
                };
                let token = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::type_error("[auth] token must be a string".to_string())),
                };
                let client_id = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::type_error("[auth] client_id must be a string".to_string())),
                };
                let client_secret = match &args[3] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::type_error("[auth] client_secret must be a string".to_string())),
                };

                match introspect_token(&introspection_url, &token, &client_id, &client_secret) {
                    Ok(result) => Ok(make_ok(Value::Map(result))),
                    Err(e) => Ok(make_err(Value::String(e.to_string()))),
                }
            },
        },
    );

    // @ntnt get_user
    // @module std/auth
    // @signature get_user(req: Request) -> Option<User>
    // Get the current authenticated user from the request.
    //
    // Returns Some(user) if authenticated, None if not. Use with `otherwise`
    // for concise auth checks in handlers.
    // @param req The HTTP request object
    // @returns Option containing the User map or None
    // @see_also get_session, logout_user
    // @since v0.3.11
    // @tags #auth
    // @example get_user(req) otherwise return redirect("/login") ~ "Require auth"
    module.insert(
        "get_user".to_string(),
        Value::NativeFunction {
            name: "get_user".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] get_user() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        return Ok(make_some(user_to_value(&session)));
                    }
                }

                Ok(make_none())
            },
        },
    );

    // @ntnt get_session
    // @module std/auth
    // @signature get_session(req: Request) -> Option<Session>
    // Get the current session from the request.
    //
    // Returns the full session object including user, timestamps, tokens, and custom data.
    // @param req The HTTP request object
    // @returns Option containing the Session map or None
    // @see_also get_user, logout_user, oauth_refresh
    // @since v0.3.11
    // @tags #auth
    // @example get_session(req) ~ "Get full session data"
    module.insert(
        "get_session".to_string(),
        Value::NativeFunction {
            name: "get_session".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] get_session() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        return Ok(make_some(session_to_value(&session)));
                    }
                }

                Ok(make_none())
            },
        },
    );

    // @ntnt current_user
    // @module std/auth
    // @signature current_user(req: Request) -> Option<User>
    // Get the current authenticated user from the request.
    //
    // Alias for `get_user(req)` with a clearer request-time name for login/session flows.
    // @param req The HTTP request object
    // @returns Option containing the User map or None
    // @see_also get_user, current_session, sign_in_session
    // @since v0.4.9
    // @tags #auth, #session
    // @example current_user(req) otherwise return redirect("/login") ~ "Require a current user"
    module.insert(
        "current_user".to_string(),
        Value::NativeFunction {
            name: "current_user".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] current_user() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        return Ok(make_some(user_to_value(&session)));
                    }
                }

                Ok(make_none())
            },
        },
    );

    // @ntnt current_session
    // @module std/auth
    // @signature current_session(req: Request) -> Option<Session>
    // Get the current session from the request.
    //
    // Alias for `get_session(req)` with a clearer request-time name for session-driven flows.
    // @param req The HTTP request object
    // @returns Option containing the Session map or None
    // @see_also get_session, current_user, rotate_session
    // @since v0.4.9
    // @tags #auth, #session
    // @example current_session(req) ~ "Read the current session"
    module.insert(
        "current_session".to_string(),
        Value::NativeFunction {
            name: "current_session".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] current_session() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        return Ok(make_some(session_to_value(&session)));
                    }
                }

                Ok(make_none())
            },
        },
    );

    // @ntnt current_auth_challenge
    // @module std/auth
    // @signature current_auth_challenge(req: Request) -> Option<AuthChallenge>
    // Get the current staged auth challenge from the request.
    //
    // Use this during multi-step auth flows like password -> TOTP or first-login
    // setup. Challenges are distinct from authenticated sessions and do not grant
    // protected-route access on their own.
    // @param req The HTTP request object
    // @returns Option containing the active auth challenge or None
    // @see_also begin_auth_challenge, complete_auth_challenge, cancel_auth_challenge
    // @since v0.4.9
    // @tags #auth, #session, #mfa
    // @example current_auth_challenge(req) ~ "Read staged auth state"
    module.insert(
        "current_auth_challenge".to_string(),
        Value::NativeFunction {
            name: "current_auth_challenge".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] current_auth_challenge() requires a request".to_string(),
                    ));
                }

                let challenge_id = get_auth_challenge_id_from_request(&args[0]);
                if let Some(id) = challenge_id {
                    if let Some(challenge) =
                        get_auth_challenge_by_id(&id).map_err(IntentError::runtime_error)?
                    {
                        return Ok(make_some(auth_challenge_to_value(&challenge)));
                    }
                }

                Ok(make_none())
            },
        },
    );

    // @ntnt begin_auth_challenge
    // @module std/auth
    // @signature begin_auth_challenge(response: Response, challenge: Map) -> Response
    // Persist a pending auth challenge and attach the challenge cookie.
    //
    // Use this for staged auth flows like MFA verification, first-login setup,
    // or password reset completion. Challenges are separate from full sessions.
    // @param response The Response map to attach the challenge cookie to
    // @param challenge Challenge data map, including required `subject_id` and `kind`
    // @returns Response with a persisted auth challenge and Set-Cookie header
    // @see_also current_auth_challenge, complete_auth_challenge, cancel_auth_challenge
    // @since v0.4.9
    // @tags #auth, #session, #mfa
    // @example begin_auth_challenge(redirect("/admin/verify"), map { "subject_id": user.id, "kind": "mfa_pending", "ttl": 1800 }) ~ "Begin a staged auth flow"
    module.insert(
        "begin_auth_challenge".to_string(),
        Value::NativeFunction {
            name: "begin_auth_challenge".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "[auth] begin_auth_challenge() requires response and challenge"
                            .to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before begin_auth_challenge()."
                            .to_string(),
                    )
                })?;

                let challenge_spec = match &args[1] {
                    Value::Map(map) => map.clone(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] begin_auth_challenge() challenge must be a map, got {}",
                            other.type_name()
                        )))
                    }
                };

                let challenge =
                    create_auth_challenge(&challenge_spec).map_err(IntentError::type_error)?;
                let ttl = challenge.expires_at - chrono::Utc::now().timestamp();
                let cookie = build_signed_auth_challenge_cookie(&config, &challenge.id, ttl)
                    .map_err(IntentError::type_error)?;
                let response =
                    add_set_cookie_header(&args[0], &cookie).map_err(IntentError::type_error)?;

                store_auth_challenge(challenge);
                Ok(response)
            },
        },
    );

    // @ntnt complete_auth_challenge
    // @module std/auth
    // @signature complete_auth_challenge(response: Response, req: Request, session?: Map, options?: Map) -> Response
    // Upgrade the current auth challenge into a full authenticated session.
    //
    // This consumes the active challenge, creates a real session, attaches the
    // normal auth cookie, and clears the challenge cookie in the same response.
    // @param response The Response map to attach cookies to
    // @param req The current HTTP request
    // @param session Optional session data map merged onto the completed session
    // @param options Optional session options like `session_ttl` and cookie overrides
    // @returns Response with the auth challenge consumed and the session cookie attached
    // @see_also begin_auth_challenge, current_auth_challenge, cancel_auth_challenge, sign_in_session
    // @since v0.4.9
    // @tags #auth, #session, #mfa
    // @example complete_auth_challenge(redirect("/admin"), req, map { "claims": map { "role": "admin" } }) ~ "Upgrade staged auth into a session"
    module.insert(
        "complete_auth_challenge".to_string(),
        Value::NativeFunction {
            name: "complete_auth_challenge".to_string(),
            arity: 2,
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 4 {
                    return Err(IntentError::type_error(
                        "[auth] complete_auth_challenge() requires response, request, and optional session/options"
                            .to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before complete_auth_challenge()."
                            .to_string(),
                    )
                })?;

                let session_spec = match args.get(2) {
                    Some(Value::Map(map)) => map.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] complete_auth_challenge() session must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => HashMap::new(),
                };

                let options = match args.get(3) {
                    Some(Value::Map(map)) => Some(map.clone()),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] complete_auth_challenge() options must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => None,
                };

                let challenge_id = get_auth_challenge_id_from_request(&args[1]).ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] complete_auth_challenge() requires an active auth challenge"
                            .to_string(),
                    )
                })?;

                // Intentionally read before consume so validation failures do not burn the
                // staged auth challenge. A later consume can still fail if another request
                // cancels or completes the flow first.
                let challenge = get_auth_challenge_by_id(&challenge_id)
                    .map_err(IntentError::runtime_error)?
                    .ok_or_else(|| {
                        IntentError::runtime_error(
                            "[auth] complete_auth_challenge() requires an active auth challenge"
                                .to_string(),
                        )
                    })?;

                let mut merged_session = session_spec.clone();
                match merged_session.get("subject_id") {
                    Some(Value::String(subject_id)) if subject_id == &challenge.subject_id => {}
                    Some(Value::String(_)) => {
                        return Err(IntentError::type_error(
                            "[auth] complete_auth_challenge() session.subject_id must match the active auth challenge"
                                .to_string(),
                        ))
                    }
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] complete_auth_challenge() session.subject_id must be a string, got {}",
                            other.type_name()
                        )))
                    }
                    None => {
                        merged_session.insert(
                            "subject_id".to_string(),
                            Value::String(challenge.subject_id.clone()),
                        );
                    }
                }

                match merged_session.get("provider") {
                    Some(Value::String(provider)) if provider == &challenge.provider => {}
                    Some(Value::String(_)) => {
                        return Err(IntentError::type_error(
                            "[auth] complete_auth_challenge() session.provider must match the active auth challenge"
                                .to_string(),
                        ))
                    }
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] complete_auth_challenge() session.provider must be a string, got {}",
                            other.type_name()
                        )))
                    }
                    None => {
                        merged_session.insert(
                            "provider".to_string(),
                            Value::String(challenge.provider.clone()),
                        );
                    }
                }

                let session_ttl = match options.as_ref().and_then(|m| m.get("session_ttl")) {
                    Some(Value::Int(i)) => *i,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] complete_auth_challenge() session_ttl must be an int, got {}",
                            other.type_name()
                        )))
                    }
                    None => config.session_ttl,
                };

                let session =
                    create_manual_session(&merged_session, session_ttl).map_err(IntentError::type_error)?;
                let session_cookie =
                    build_signed_session_cookie(&config, &session.id, options.as_ref())
                        .map_err(IntentError::type_error)?;
                let cleared_challenge_cookie =
                    build_cleared_auth_challenge_cookie(&config).map_err(IntentError::type_error)?;
                let response =
                    add_set_cookie_header(&args[0], &session_cookie).map_err(IntentError::type_error)?;
                let response = add_set_cookie_header(&response, &cleared_challenge_cookie)
                    .map_err(IntentError::type_error)?;

                let consumed = consume_auth_challenge(&challenge_id)
                    .map_err(IntentError::runtime_error)?;
                if consumed.is_none() {
                    return Err(IntentError::runtime_error(
                        "[auth] complete_auth_challenge() auth challenge expired, was cancelled, or was already consumed"
                            .to_string(),
                    ));
                }

                store_session(session);
                Ok(response)
            },
        },
    );

    // @ntnt cancel_auth_challenge
    // @module std/auth
    // @signature cancel_auth_challenge(response: Response, req: Request) -> Response
    // Cancel the current auth challenge and clear the challenge cookie.
    //
    // Use this when a staged auth flow is abandoned, fails, or needs to be reset.
    // @param response The Response map to attach the clearing cookie to
    // @param req The current HTTP request
    // @returns Response with the challenge cookie cleared
    // @see_also begin_auth_challenge, current_auth_challenge, complete_auth_challenge
    // @since v0.4.9
    // @tags #auth, #session, #mfa
    // @example cancel_auth_challenge(redirect("/login"), req) ~ "Cancel staged auth"
    module.insert(
        "cancel_auth_challenge".to_string(),
        Value::NativeFunction {
            name: "cancel_auth_challenge".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "[auth] cancel_auth_challenge() requires response and request".to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before cancel_auth_challenge()."
                            .to_string(),
                    )
                })?;

                let cleared_cookie =
                    build_cleared_auth_challenge_cookie(&config).map_err(IntentError::type_error)?;
                let response =
                    add_set_cookie_header(&args[0], &cleared_cookie).map_err(IntentError::type_error)?;

                if let Some(challenge_id) = get_auth_challenge_id_from_request(&args[1]) {
                    delete_auth_challenge_by_id(&challenge_id);
                }

                Ok(response)
            },
        },
    );

    // @ntnt session_data
    // @module std/auth
    // @signature session_data(req: Request) -> Option<Map>
    // Get custom data stored in the current session.
    //
    // Returns the custom data map stored via set_session, or None if no session
    // or no custom data. Use this to store and retrieve user roles, permissions,
    // preferences, or other application-specific data.
    // @param req The HTTP request object
    // @returns Option containing the custom data Map or None
    // @see_also set_session, get_session, get_user
    // @since v0.3.11
    // @tags #auth, #rbac
    // @example session_data(req) ~ "Get user roles and preferences"
    module.insert(
        "session_data".to_string(),
        Value::NativeFunction {
            name: "session_data".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] session_data() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        let data = json_string_to_value_map(&session.data_json);
                        if data.is_empty() {
                            return Ok(make_none());
                        }
                        return Ok(make_some(Value::Map(data)));
                    }
                }

                Ok(make_none())
            },
        },
    );

    // @ntnt validate_csrf
    // @module std/auth
    // @signature validate_csrf(req: Request) -> Result<Bool, Map>
    // Validate CSRF token on state-changing requests (POST, PUT, DELETE, PATCH).
    //
    // Compares the CSRF token from the request (form field `_csrf_token` or header
    // `X-CSRF-Token`) against the token stored in the session. Returns `true` if
    // valid. Returns an error response map (403) if invalid, which can be returned
    // directly from a route handler.
    //
    // Skips validation for:
    // - GET, HEAD, OPTIONS requests (safe methods)
    // - API key auth (Bearer token) — CSRF only applies to cookie-based sessions
    // - Requests with no session (will fail auth check separately)
    //
    // Usage in middleware:
    // ```ntnt
    // let csrf_ok = validate_csrf(req)
    // if typeof(csrf_ok) == "Map" { return csrf_ok }  // Return 403 response
    // ```
    //
    // Usage in forms:
    // ```html
    // <input type="hidden" name="_csrf_token" value="{{user.csrf_token}}">
    // ```
    // @param req The HTTP request object
    // @returns true if valid or safe method; a 403 error response Map if invalid
    // @see_also get_user, get_session
    // @since v0.4.0
    // @tags #auth, #csrf, #security
    // @example validate_csrf(req) ~ "Check CSRF token on POST"
    module.insert(
        "validate_csrf".to_string(),
        Value::NativeFunction {
            name: "validate_csrf".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] validate_csrf() requires a request".to_string(),
                    ));
                }

                let req = &args[0];
                let req_map = match req {
                    Value::Map(m) => m,
                    _ => return Ok(Value::Bool(true)), // Not a valid request, let other checks handle it
                };

                // Skip safe methods (GET, HEAD, OPTIONS)
                let method = match req_map.get("method") {
                    Some(Value::String(m)) => m.to_uppercase(),
                    _ => return Ok(Value::Bool(true)),
                };
                if method == "GET" || method == "HEAD" || method == "OPTIONS" {
                    return Ok(Value::Bool(true));
                }

                // Skip if request has Bearer token (API key auth, not session-based)
                if let Some(Value::Map(headers)) = req_map.get("headers") {
                    if let Some(Value::String(auth_header)) = headers.get("authorization") {
                        if auth_header.starts_with("Bearer ") {
                            return Ok(Value::Bool(true));
                        }
                    }
                }

                // Get session CSRF token
                let session_id = get_session_id_from_request(req);
                let session_csrf = match session_id {
                    Some(ref id) => match get_session_by_id(id) {
                        Some(session) if !session.csrf_token.is_empty() => {
                            session.csrf_token.clone()
                        }
                        _ => return Ok(Value::Bool(true)), // No session = no CSRF to validate
                    },
                    None => return Ok(Value::Bool(true)), // No cookie = not a browser session
                };

                // Extract CSRF token from request:
                // 1. Form field: _csrf_token
                // 2. Header: X-CSRF-Token
                let mut request_csrf = None;

                // Check form body (URL-encoded or JSON)
                if let Some(Value::String(body)) = req_map.get("body") {
                    // Try URL-encoded form: _csrf_token=value
                    for param in body.split('&') {
                        let parts: Vec<&str> = param.splitn(2, '=').collect();
                        if parts.len() == 2 && parts[0] == "_csrf_token" {
                            request_csrf = Some(
                                urlencoding::decode(parts[1])
                                    .unwrap_or_default()
                                    .to_string(),
                            );
                            break;
                        }
                    }
                    // Try JSON body: {"_csrf_token": "value"}
                    if request_csrf.is_none() {
                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(body) {
                            if let Some(token) = json.get("_csrf_token").and_then(|v| v.as_str()) {
                                request_csrf = Some(token.to_string());
                            }
                        }
                    }
                }

                // Check X-CSRF-Token header (for AJAX requests)
                if request_csrf.is_none() {
                    if let Some(Value::Map(headers)) = req_map.get("headers") {
                        if let Some(Value::String(token)) = headers.get("x-csrf-token") {
                            request_csrf = Some(token.clone());
                        }
                    }
                }

                // Validate
                match request_csrf {
                    Some(token) if token == session_csrf => Ok(Value::Bool(true)),
                    _ => {
                        eprintln!(
                            "[auth] CSRF validation failed for {} {}",
                            method,
                            req_map
                                .get("path")
                                .and_then(|v| if let Value::String(s) = v {
                                    Some(s.as_str())
                                } else {
                                    None
                                })
                                .unwrap_or("?")
                        );
                        // Return a 403 response map that can be returned directly from handlers
                        let mut response = HashMap::new();
                        response.insert("status".to_string(), Value::Int(403));
                        let mut headers = HashMap::new();
                        headers.insert(
                            "content-type".to_string(),
                            Value::String("text/html; charset=utf-8".to_string()),
                        );
                        response.insert("headers".to_string(), Value::Map(headers));
                        response.insert(
                            "body".to_string(),
                            Value::String("CSRF token missing or invalid".to_string()),
                        );
                        Ok(Value::Map(response))
                    }
                }
            },
        },
    );

    // @ntnt set_session
    // @module std/auth
    // @signature set_session(req: Request, data: Map) -> Result<Unit, String>
    // Store custom data in the current session.
    //
    // Use this to store user roles, permissions, preferences, or other application-specific
    // data that should persist across requests. Data is stored as JSON in the session.
    // @param req The HTTP request object
    // @param data The custom data map to store
    // @returns Result indicating success or error message
    // @see_also session_data, get_session
    // @since v0.3.11
    // @tags #auth, #rbac
    // @example set_session(req, map { "roles": ["admin"], "theme": "dark" }) ~ "Store user preferences"
    module.insert(
        "set_session".to_string(),
        Value::NativeFunction {
            name: "set_session".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] set_session() requires request and data".to_string(),
                    ));
                }

                let data_map = match &args[1] {
                    Value::Map(m) => m.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] set_session() data must be a map".to_string(),
                        ))
                    }
                };

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    let data_json = value_map_to_json_string(&data_map);
                    if let Err(e) = update_session_data(&id, &data_json) {
                        return Ok(make_err(Value::String(e)));
                    }
                    return Ok(make_ok(Value::Unit));
                }

                Ok(make_err(Value::String("No active session".to_string())))
            },
        },
    );

    // @ntnt sessions_cleanup
    // @module std/auth
    // @signature sessions_cleanup() -> Result<Int, String>
    // Clean up expired sessions, auth challenges, OAuth states, and exchange tokens from the session store.
    //
    // Call this periodically (e.g., via a cron job or scheduled task) to remove
    // expired sessions, auth challenges, OAuth states, and exchange tokens from the database. For Redis,
    // these use TTL so they expire automatically, but this will scan for any orphaned entries.
    // @returns Result containing the number of expired entries removed, or error
    // @see_also enable_auth
    // @since v0.3.11
    // @tags #auth, #maintenance
    // @example sessions_cleanup() ~ "Remove expired sessions"
    module.insert(
        "sessions_cleanup".to_string(),
        Value::NativeFunction {
            name: "sessions_cleanup".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| {
                let mut total = 0u64;

                // Clean up expired sessions
                match cleanup_expired_sessions() {
                    Ok(count) => total += count,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!(
                            "Session cleanup failed: {}",
                            e
                        ))))
                    }
                }

                // Clean up expired auth challenges
                match cleanup_expired_auth_challenges() {
                    Ok(count) => total += count,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!(
                            "Auth challenge cleanup failed: {}",
                            e
                        ))))
                    }
                }

                // Clean up expired OAuth states
                match cleanup_expired_oauth_states() {
                    Ok(count) => total += count,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!(
                            "OAuth state cleanup failed: {}",
                            e
                        ))))
                    }
                }

                // Clean up expired exchange tokens
                match cleanup_expired_exchange_tokens() {
                    Ok(count) => total += count,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!(
                            "Exchange token cleanup failed: {}",
                            e
                        ))))
                    }
                }

                Ok(make_ok(Value::Int(total as i64)))
            },
        },
    );

    // @ntnt user_sessions
    // @module std/auth
    // @signature user_sessions(req: Request) -> Result<Array<SessionInfo>, String>
    // Get all active sessions for the current user.
    //
    // Returns an array of session info objects, each containing id, provider,
    // created_at, expires_at, and is_current (boolean indicating if it's the
    // current session). Useful for "manage your sessions" UI.
    // @param req The HTTP request object
    // @returns Result containing array of session info, or error
    // @see_also logout_all, get_session
    // @since v0.3.11
    // @tags #auth, #security
    // @example user_sessions(req) ~ "List all user's active sessions"
    module.insert(
        "user_sessions".to_string(),
        Value::NativeFunction {
            name: "user_sessions".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] user_sessions() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                let session = session_id.as_ref().and_then(|id| get_session_by_id(id));

                if let Some(s) = session {
                    match get_sessions_for_user(&s.user_id, session_id.as_deref()) {
                        Ok(sessions) => {
                            let session_values: Vec<Value> = sessions
                                .iter()
                                .map(|si| {
                                    let mut map = HashMap::new();
                                    map.insert("id".to_string(), Value::String(si.id.clone()));
                                    map.insert(
                                        "provider".to_string(),
                                        Value::String(si.provider.clone()),
                                    );
                                    map.insert("created_at".to_string(), Value::Int(si.created_at));
                                    map.insert("expires_at".to_string(), Value::Int(si.expires_at));
                                    map.insert(
                                        "is_current".to_string(),
                                        Value::Bool(si.is_current),
                                    );
                                    Value::Map(map)
                                })
                                .collect();
                            Ok(make_ok(Value::Array(session_values)))
                        }
                        Err(e) => Ok(make_err(Value::String(e))),
                    }
                } else {
                    Ok(make_err(Value::String("Not authenticated".to_string())))
                }
            },
        },
    );

    // @ntnt logout_all
    // @module std/auth
    // @signature logout_all(req: Request, keep_current: Bool) -> Result<Int, String>
    // Log out all sessions for the current user.
    //
    // Deletes all sessions for the user. If keep_current is true, keeps the
    // current session active (useful for "log out everywhere else"). Returns
    // the number of sessions that were deleted.
    // @param req The HTTP request object
    // @param keep_current If true, keep the current session active
    // @returns Result containing number of sessions deleted, or error
    // @see_also user_sessions, logout_user
    // @since v0.3.11
    // @tags #auth, #security
    // @example logout_all(req, true) ~ "Log out everywhere except here"
    // @example logout_all(req, false) ~ "Log out from all devices"
    module.insert(
        "logout_all".to_string(),
        Value::NativeFunction {
            name: "logout_all".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] logout_all() requires request and keep_current".to_string(),
                    ));
                }

                let keep_current = match &args[1] {
                    Value::Bool(b) => *b,
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] keep_current must be a boolean".to_string(),
                        ))
                    }
                };

                let session_id = get_session_id_from_request(&args[0]);
                let session = session_id.as_ref().and_then(|id| get_session_by_id(id));

                if let Some(s) = session {
                    let keep_id = if keep_current {
                        session_id.as_deref()
                    } else {
                        None
                    };
                    match delete_all_sessions_for_user(&s.user_id, keep_id) {
                        Ok(count) => Ok(make_ok(Value::Int(count as i64))),
                        Err(e) => Ok(make_err(Value::String(e))),
                    }
                } else {
                    Ok(make_err(Value::String("Not authenticated".to_string())))
                }
            },
        },
    );

    // @ntnt csrf_token
    // @module std/auth
    // @signature csrf_token(req: Request) -> Option<String>
    // Get the CSRF token for the current session.
    //
    // Use this token in forms to protect against Cross-Site Request Forgery.
    // Include the token as a hidden field named "_csrf" and verify it with verify_csrf().
    // @param req The HTTP request object
    // @returns Option containing the CSRF token string, or None if not authenticated
    // @see_also verify_csrf, csrf_field
    // @since v0.3.11
    // @tags #auth, #security
    // @example csrf_token(req) ~ "Get token for form"
    module.insert(
        "csrf_token".to_string(),
        Value::NativeFunction {
            name: "csrf_token".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] csrf_token() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        return Ok(make_some(Value::String(session.csrf_token)));
                    }
                }

                Ok(make_none())
            },
        },
    );

    // @ntnt csrf_field
    // @module std/auth
    // @signature csrf_field(req: Request) -> String
    // Get an HTML hidden input field with the CSRF token.
    //
    // Returns a ready-to-use hidden input element for forms. Use this to
    // easily include CSRF protection in your forms without manual formatting.
    // @param req The HTTP request object
    // @returns HTML string like `<input type="hidden" name="_csrf" value="..."/>`
    // @see_also csrf_token, verify_csrf
    // @since v0.3.11
    // @tags #auth, #security
    // @example csrf_field(req) ~ "Get hidden input for form"
    module.insert(
        "csrf_field".to_string(),
        Value::NativeFunction {
            name: "csrf_field".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] csrf_field() requires a request".to_string(),
                    ));
                }

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        let field = format!(
                            r#"<input type="hidden" name="_csrf" value="{}"/>"#,
                            session.csrf_token
                        );
                        return Ok(Value::String(field));
                    }
                }

                // Return empty string if not authenticated
                Ok(Value::String(String::new()))
            },
        },
    );

    // @ntnt verify_csrf
    // @module std/auth
    // @signature verify_csrf(req: Request, token: String) -> Bool
    // Verify a CSRF token against the session's token.
    //
    // Returns true if the token matches the session's CSRF token, false otherwise.
    // Use this in POST/PUT/DELETE handlers to validate the "_csrf" form field.
    // @param req The HTTP request object
    // @param token The CSRF token from the form submission
    // @returns true if valid, false if invalid or not authenticated
    // @see_also csrf_token, csrf_field
    // @since v0.3.11
    // @tags #auth, #security
    // @example verify_csrf(req, form["_csrf"]) ~ "Validate form submission"
    module.insert(
        "verify_csrf".to_string(),
        Value::NativeFunction {
            name: "verify_csrf".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] verify_csrf() requires request and token".to_string(),
                    ));
                }

                let submitted_token = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] token must be a string".to_string(),
                        ))
                    }
                };

                let session_id = get_session_id_from_request(&args[0]);
                if let Some(id) = session_id {
                    if let Some(session) = get_session_by_id(&id) {
                        // Use proper constant-time comparison to prevent timing attacks
                        let valid = constant_time_compare(&session.csrf_token, &submitted_token);
                        return Ok(Value::Bool(valid));
                    }
                }

                Ok(Value::Bool(false))
            },
        },
    );

    // @ntnt jwt_sign
    // @module std/auth
    // @signature jwt_sign(claims: Map, secret: String, options?: Map) -> Result<String, String>
    // Create a signed JWT token from claims.
    //
    // Signs the claims using HS256 algorithm and returns the JWT string.
    // Optional options map can include: exp (expiration as unix timestamp),
    // iat (issued-at, defaults to now), sub (subject), iss (issuer), aud (audience).
    // @param claims The payload claims as a map
    // @param secret The signing secret (should be at least 32 bytes)
    // @param options Optional map with exp, iat, sub, iss, aud
    // @returns Result containing the JWT string, or error message
    // @see_also jwt_verify, jwt_decode
    // @since v0.3.11
    // @tags #auth, #jwt
    // @example jwt_sign(map { "user_id": 123 }, secret) ~ "Create a token"
    // @example jwt_sign(map { "user_id": 123 }, secret, map { "exp": now() + 3600 }) ~ "Token with 1hr expiry"
    module.insert(
        "jwt_sign".to_string(),
        Value::NativeFunction {
            name: "jwt_sign".to_string(),
            arity: 2, // 2-3 args (options is optional)
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] jwt_sign() requires claims and secret".to_string(),
                    ));
                }

                let claims = match &args[0] {
                    Value::Map(m) => m.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] claims must be a map".to_string(),
                        ))
                    }
                };

                let secret = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] secret must be a string".to_string(),
                        ))
                    }
                };

                let options = if args.len() > 2 {
                    match &args[2] {
                        Value::Map(m) => Some(m.clone()),
                        _ => None,
                    }
                } else {
                    None
                };

                // Build JWT claims
                let mut jwt_claims = serde_json::Map::new();

                // Add user claims
                for (k, v) in &claims {
                    jwt_claims.insert(k.clone(), value_to_json(v));
                }

                // Add standard claims from options
                let now = chrono::Utc::now().timestamp();
                if let Some(opts) = &options {
                    if let Some(Value::Int(exp)) = opts.get("exp") {
                        jwt_claims.insert("exp".to_string(), serde_json::json!(*exp));
                    }
                    if let Some(Value::Int(iat)) = opts.get("iat") {
                        jwt_claims.insert("iat".to_string(), serde_json::json!(*iat));
                    } else {
                        jwt_claims.insert("iat".to_string(), serde_json::json!(now));
                    }
                    if let Some(Value::String(sub)) = opts.get("sub") {
                        jwt_claims.insert("sub".to_string(), serde_json::json!(sub));
                    }
                    if let Some(Value::String(iss)) = opts.get("iss") {
                        jwt_claims.insert("iss".to_string(), serde_json::json!(iss));
                    }
                    if let Some(Value::String(aud)) = opts.get("aud") {
                        jwt_claims.insert("aud".to_string(), serde_json::json!(aud));
                    }
                } else {
                    jwt_claims.insert("iat".to_string(), serde_json::json!(now));
                }

                let encoding_key = EncodingKey::from_secret(secret.as_bytes());
                match encode(&Header::default(), &jwt_claims, &encoding_key) {
                    Ok(token) => Ok(make_ok(Value::String(token))),
                    Err(e) => Ok(make_err(Value::String(format!(
                        "JWT encoding error: {}",
                        e
                    )))),
                }
            },
        },
    );

    // @ntnt jwt_verify
    // @module std/auth
    // @signature jwt_verify(token: String, secret: String) -> Result<Map, String>
    // Verify a JWT token and return its claims.
    //
    // Validates the signature and expiration, then returns the claims as a map.
    // Returns Err if the token is invalid, expired, or has wrong signature.
    // @param token The JWT token string
    // @param secret The signing secret used to create the token
    // @returns Result containing the claims map, or error message
    // @see_also jwt_sign, jwt_decode
    // @since v0.3.11
    // @tags #auth, #jwt
    // @example jwt_verify(token, secret) ~ "Verify and get claims"
    module.insert(
        "jwt_verify".to_string(),
        Value::NativeFunction {
            name: "jwt_verify".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] jwt_verify() requires token and secret".to_string(),
                    ));
                }

                let token = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] token must be a string".to_string(),
                        ))
                    }
                };

                let secret = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] secret must be a string".to_string(),
                        ))
                    }
                };

                let decoding_key = DecodingKey::from_secret(secret.as_bytes());
                let validation = Validation::new(Algorithm::HS256);

                match decode::<serde_json::Map<String, serde_json::Value>>(
                    &token,
                    &decoding_key,
                    &validation,
                ) {
                    Ok(token_data) => {
                        let claims = json_map_to_value_map(&token_data.claims);
                        Ok(make_ok(Value::Map(claims)))
                    }
                    Err(e) => Ok(make_err(Value::String(format!(
                        "JWT verification failed: {}",
                        e
                    )))),
                }
            },
        },
    );

    // @ntnt jwt_decode
    // @module std/auth
    // @signature jwt_decode(token: String) -> Result<Map, String>
    // Decode a JWT token WITHOUT verifying the signature.
    //
    // Use this only for debugging or when you need to inspect token contents
    // before verification. Never trust the claims from this function for auth.
    // @param token The JWT token string
    // @returns Result containing map with "header" and "payload" keys, or error
    // @see_also jwt_sign, jwt_verify
    // @since v0.3.11
    // @tags #auth, #jwt
    // @example jwt_decode(token) ~ "Inspect token without verification"
    module.insert(
        "jwt_decode".to_string(),
        Value::NativeFunction {
            name: "jwt_decode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] jwt_decode() requires a token".to_string(),
                    ));
                }

                let token = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] token must be a string".to_string(),
                        ))
                    }
                };

                // Split the token into parts
                let parts: Vec<&str> = token.split('.').collect();
                if parts.len() != 3 {
                    return Ok(make_err(Value::String("Invalid JWT format".to_string())));
                }

                // Decode header
                let header_json =
                    match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[0]) {
                        Ok(bytes) => match String::from_utf8(bytes) {
                            Ok(s) => s,
                            Err(_) => {
                                return Ok(make_err(Value::String(
                                    "Invalid header encoding".to_string(),
                                )))
                            }
                        },
                        Err(_) => {
                            return Ok(make_err(Value::String("Invalid header base64".to_string())))
                        }
                    };

                // Decode payload
                let payload_json =
                    match base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]) {
                        Ok(bytes) => match String::from_utf8(bytes) {
                            Ok(s) => s,
                            Err(_) => {
                                return Ok(make_err(Value::String(
                                    "Invalid payload encoding".to_string(),
                                )))
                            }
                        },
                        Err(_) => {
                            return Ok(make_err(Value::String(
                                "Invalid payload base64".to_string(),
                            )))
                        }
                    };

                // Parse JSON
                let header: serde_json::Map<String, serde_json::Value> =
                    match serde_json::from_str(&header_json) {
                        Ok(h) => h,
                        Err(_) => {
                            return Ok(make_err(Value::String("Invalid header JSON".to_string())))
                        }
                    };

                let payload: serde_json::Map<String, serde_json::Value> =
                    match serde_json::from_str(&payload_json) {
                        Ok(p) => p,
                        Err(_) => {
                            return Ok(make_err(Value::String("Invalid payload JSON".to_string())))
                        }
                    };

                let mut result = HashMap::new();
                result.insert(
                    "header".to_string(),
                    Value::Map(json_map_to_value_map(&header)),
                );
                result.insert(
                    "payload".to_string(),
                    Value::Map(json_map_to_value_map(&payload)),
                );

                Ok(make_ok(Value::Map(result)))
            },
        },
    );

    // @ntnt logout_user
    // @module std/auth
    // @signature logout_user(req: Request) -> Response
    // Log out the current user and return a redirect response.
    //
    // Clears the session and returns a redirect to the configured logout_url
    // (default: "/") with the session cookie cleared.
    // @param req The HTTP request object
    // @returns Redirect response with session cookie cleared
    // @see_also get_user, get_session
    // @since v0.3.11
    // @tags #auth
    // @example logout_user(req) ~ "Log out and redirect to home"
    module.insert(
        "logout_user".to_string(),
        Value::NativeFunction {
            name: "logout_user".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "[auth] logout_user() requires a request".to_string(),
                    ));
                }

                let config = get_auth_config().unwrap_or_default();

                if let Some(session_id) = get_session_id_from_request(&args[0]) {
                    delete_session_by_id(&session_id);
                }

                let cookie = build_cleared_session_cookie(&config, None)
                    .map_err(IntentError::runtime_error)?;

                Ok(redirect_response(&config.logout_url, Some(&cookie)))
            },
        },
    );

    // @ntnt enable_auth
    // @module std/auth
    // @signature enable_auth(providers: [Provider], options?: Map) -> Unit
    // Initialize the authentication system with OAuth providers.
    //
    // Stores provider configurations for use by auth handlers. After calling this,
    // you can use auth_start, auth_callback, and auth_logout
    // with routes to enable OAuth login.
    //
    // Session storage options: "memory" (default), "sqlite:./path.db", "postgres://url", or "redis://url".
    // @param providers Array of provider configs created by oauth() or oauth_discover()
    // @param options Optional map with keys: session_secret, session_ttl, refresh_ttl, success_url/after_login, failure_url/after_failure, logout_url/after_logout, protected_paths, cookie_name, cookie_secure, session_store, store_tokens
    // @returns Unit
    // @see_also oauth, oauth_discover, auth_start
    // @since v0.3.11
    // @tags #auth, #oauth
    // @example ~ "Initialize auth with GitHub"
    //   let github = oauth("github", get_env("GITHUB_ID"), get_env("GITHUB_SECRET"))
    //   enable_auth([github], map { "session_secret": "my-secret" })
    // @example enable_auth([github], map { "session_store": "sqlite:./sessions.db" }) ~ "SQLite sessions"
    // @example enable_auth([github], map { "session_store": "redis://localhost:6379" }) ~ "Redis sessions"
    module.insert(
        "enable_auth".to_string(),
        Value::NativeFunction {
            name: "enable_auth".to_string(),
            arity: 0, // Variadic: 1-2 args (providers, options?)
            max_arity: 0,
            requires: Some(RuntimeCapability::HttpConfig),
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] enable_auth() requires 1 or 2 arguments (providers, optional config)"
                            .to_string(),
                    ));
                }

                // Parse providers array
                let providers_arr = match &args[0] {
                    Value::Array(arr) => arr.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] enable_auth() first argument must be an array of providers"
                                .to_string(),
                        ))
                    }
                };

                let options = match args.get(1) {
                    Some(Value::Map(m)) => Some(m.clone()),
                    Some(_) => {
                        return Err(IntentError::type_error(
                            "[auth] enable_auth() second argument must be an options map"
                                .to_string(),
                        ))
                    }
                    None => None,
                };

                // Parse providers
                let mut providers = Vec::new();
                for (idx, pval) in providers_arr.iter().enumerate() {
                    match pval {
                        Value::Map(pmap) => {
                            let provider = value_map_to_provider(pmap).map_err(|e| {
                                IntentError::type_error(format!(
                                    "[auth] Invalid provider at index {}: {}",
                                    idx, e
                                ))
                            })?;
                            providers.push(provider);
                        }
                        _ => {
                            return Err(IntentError::type_error(format!(
                                "[auth] Provider at index {} must be a map (use oauth() to create)",
                                idx
                            )));
                        }
                    }
                }

                if let Some(opts) = &options {
                    for key in opts.keys() {
                        let known = matches!(
                            key.as_str(),
                            "session_secret"
                                | "session_ttl"
                                | "refresh_ttl"
                                | "success_url"
                                | "after_login"
                                | "failure_url"
                                | "after_failure"
                                | "logout_url"
                                | "after_logout"
                                | "cookie_name"
                                | "cookie_secure"
                                | "session_store"
                                | "store_tokens"
                                | "protected_paths"
                        );
                        if !known {
                            let suggestion = auth_option_suggestion(key)
                                .map(|s| format!(" Did you mean \"{}\"?", s))
                                .unwrap_or_default();
                            return Err(IntentError::type_error(format!(
                                "[auth] enable_auth() unknown option \"{}\".{}",
                                key, suggestion
                            )));
                        }
                    }
                }

                let get_option = |keys: &[&str]| {
                    options
                        .as_ref()
                        .and_then(|opts| keys.iter().find_map(|key| opts.get(*key)))
                };

                let session_secret = match get_option(&["session_secret"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"session_secret\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => DEFAULT_SESSION_SECRET_SENTINEL.to_string(),
                };

                let session_ttl = match get_option(&["session_ttl"]) {
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"session_ttl\" must be an int, got {}",
                            other.type_name()
                        )));
                    }
                    None => 86400,
                };

                let success_url = match get_option(&["success_url", "after_login"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"success_url\"/\"after_login\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => "/".to_string(),
                };

                let failure_url = match get_option(&["failure_url", "after_failure"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"failure_url\"/\"after_failure\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => "/".to_string(),
                };

                let logout_url = match get_option(&["logout_url", "after_logout"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"logout_url\"/\"after_logout\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => "/".to_string(),
                };

                let protected_paths = match get_option(&["protected_paths"]) {
                    Some(Value::String(s)) => vec![s.clone()],
                    Some(Value::Array(arr)) => {
                        let mut paths = Vec::new();
                        for value in arr {
                            match value {
                                Value::String(path) => paths.push(path.clone()),
                                other => {
                                    return Err(IntentError::type_error(format!(
                                        "[auth] enable_auth() option \"protected_paths\" entries must be strings, got {}",
                                        other.type_name()
                                    )));
                                }
                            }
                        }
                        paths
                    }
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"protected_paths\" must be a string or array of strings, got {}",
                            other.type_name()
                        )));
                    }
                    None => Vec::new(),
                };

                let cookie_name = match get_option(&["cookie_name"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"cookie_name\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => "ntnt_session".to_string(),
                };

                let cookie_secure = match get_option(&["cookie_secure"]) {
                    Some(Value::Bool(b)) => *b,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"cookie_secure\" must be a bool, got {}",
                            other.type_name()
                        )));
                    }
                    None => default_auth_cookie_secure_env(),
                };

                let session_store = match get_option(&["session_store"]) {
                    Some(Value::String(s)) => parse_auth_session_store(s)
                        .map_err(IntentError::type_error)?,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"session_store\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => SessionStore::Memory,
                };

                // Initialize database/cache if needed
                if let Err(e) = initialize_session_store(&session_store) {
                    eprintln!("[auth] Failed to initialize session store: {}", e);
                    return Err(IntentError::runtime_error(format!(
                        "Failed to initialize session store: {}",
                        e
                    )));
                }

                let store_tokens = match get_option(&["store_tokens"]) {
                    Some(Value::Bool(b)) => *b,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"store_tokens\" must be a bool, got {}",
                            other.type_name()
                        )));
                    }
                    None => false,
                };

                let refresh_ttl = match get_option(&["refresh_ttl"]) {
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"refresh_ttl\" must be an int, got {}",
                            other.type_name()
                        )));
                    }
                    None => 86400 * 30,
                };

                // Create auth config
                let config = AuthConfig {
                    providers,
                    success_url,
                    failure_url,
                    logout_url,
                    protected_paths,
                    cookie_name,
                    cookie_secure,
                    cookie_same_site: "Lax".to_string(),
                    session_ttl,
                    store_tokens,
                    refresh_ttl,
                    session_secret,
                    session_store,
                };

                // Initialize auth
                init_auth(config);

                Ok(Value::Unit)
            },
        },
    );

    // ==========================================================================
    // OAuth Primitives (for manual flow control)
    // ==========================================================================

    // @ntnt require_auth
    // @module std/auth
    // @signature require_auth(target?: Request | String | [String]) -> Function | Unit | Response
    // Protect routes with the configured auth session.
    //
    // Usage patterns:
    //
    // - `use_middleware(require_auth())` protects every request that reaches that middleware.
    //
    // - `require_auth("/admin/*")` registers protected path patterns for file-routed apps.
    //
    // - `require_auth(req)` may be called directly inside custom middleware.
    //
    // @param target Optional request object, single path pattern, or array of path patterns
    // @returns Middleware function, Unit for path registration, or a redirect/401 response when called with a request
    // @see_also enable_auth, get_user, get_session
    // @since v0.4.9
    // @tags #auth, #middleware
    // @example use_middleware(require_auth()) ~ "Protect every request"
    // @example require_auth("/admin/*") ~ "Protect all admin file routes"
    module.insert(
        "require_auth".to_string(),
        Value::NativeFunction {
            name: "require_auth".to_string(),
            arity: 0,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.len() == 1 {
                    if let Value::Map(map) = &args[0] {
                        if map.contains_key("method") && map.contains_key("path") {
                            return match enforce_auth_for_request(&args[0], true) {
                                Ok(()) => Ok(Value::Unit),
                                Err(response) => Ok(response),
                            };
                        }
                    }
                }

                if !args.is_empty() {
                    if get_auth_config().is_none() {
                        return Err(IntentError::runtime_error(
                            "[auth] require_auth(path) requires enable_auth() to be called first"
                                .to_string(),
                        ));
                    }

                    let mut paths = Vec::new();
                    for arg in args {
                        match arg {
                            Value::String(path) => paths.push(path.clone()),
                            Value::Array(items) => {
                                for item in items {
                                    match item {
                                        Value::String(path) => paths.push(path.clone()),
                                        other => {
                                            return Err(IntentError::type_error(format!(
                                                "[auth] require_auth() path entries must be strings, got {}",
                                                other.type_name()
                                            )));
                                        }
                                    }
                                }
                            }
                            other => {
                                return Err(IntentError::type_error(format!(
                                    "[auth] require_auth() expects a request, string path, or array of string paths, got {}",
                                    other.type_name()
                                )));
                            }
                        }
                    }

                    register_protected_paths(&paths);
                    return Ok(Value::Unit);
                }

                Ok(Value::NativeFunction {
                    name: "require_auth_middleware".to_string(),
                    arity: 1,
                    max_arity: 1,
                    requires: None,
                    func: |mw_args| match enforce_auth_for_request(&mw_args[0], true) {
                        Ok(()) => Ok(Value::Unit),
                        Err(response) => Ok(response),
                    },
                })
            },
        },
    );

    // @ntnt oauth_start
    // @module std/auth
    // @signature oauth_start(provider: Map, redirect_uri: String) -> Result<String, String>
    // Generate an OAuth authorization URL for manual flow control.
    //
    // Use this when you want to control the OAuth flow manually instead of using
    // auth_start. Returns the authorization URL with state parameter
    // for CSRF protection.
    // @param provider Provider config from oauth()
    // @param redirect_uri Your callback URL
    // @returns Ok(auth_url) to redirect user to, Err on failure
    // @see_also oauth_exchange, oauth
    // @since v0.3.11
    // @tags #auth, #oauth, #primitive
    // @example oauth_start(github, "https://myapp.com/callback") => Ok("https://github.com/...") ~ "Get auth URL"
    module.insert(
        "oauth_start".to_string(),
        Value::NativeFunction {
            name: "oauth_start".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] oauth_start() requires (provider, redirect_uri)".to_string(),
                    ));
                }

                // Parse provider config
                let provider = match value_to_provider(&args[0]) {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!("Invalid provider: {}", e))))
                    }
                };

                let redirect_uri = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] redirect_uri must be a string".to_string(),
                        ))
                    }
                };

                // Generate state for CSRF protection
                let state = uuid::Uuid::new_v4().to_string();

                // Generate PKCE if enabled
                let (pkce_verifier, pkce_challenge) = if provider.use_pkce {
                    let verifier = generate_pkce_verifier();
                    let challenge = generate_pkce_challenge(&verifier);
                    (Some(verifier), Some(challenge))
                } else {
                    (None, None)
                };

                // Generate nonce for OIDC
                let nonce = if provider.supports_oidc {
                    Some(uuid::Uuid::new_v4().to_string())
                } else {
                    None
                };

                // Store OAuth state for validation
                store_oauth_state(
                    &state,
                    &provider.name,
                    &redirect_uri,
                    nonce.as_deref(),
                    pkce_verifier.as_deref(),
                );

                // Generate the authorization URL
                let auth_url = generate_auth_url(
                    &provider,
                    &redirect_uri,
                    &state,
                    nonce.as_deref(),
                    pkce_challenge.as_deref(),
                );

                Ok(make_ok(Value::String(auth_url)))
            },
        },
    );

    // @ntnt oauth_exchange
    // @module std/auth
    // @signature oauth_exchange(provider: Map, code: String, state: String, redirect_uri: String) -> Result<Map, String>
    // Exchange OAuth authorization code for tokens and user info.
    //
    // Use this after receiving the callback with code and state parameters.
    // Returns tokens and user info - you decide what to do with them (create session, etc).
    // @param provider Provider config from oauth()
    // @param code Authorization code from callback
    // @param state State parameter from callback (for CSRF validation)
    // @param redirect_uri Same redirect_uri used in oauth_start
    // @returns Ok(map with tokens and user_info) or Err on failure
    // @see_also oauth_start, create_session_from_oauth
    // @since v0.3.11
    // @tags #auth, #oauth, #primitive
    // @example oauth_exchange(github, code, state, redirect_uri) => Ok({access_token: "...", user_info: {...}}) ~ "Exchange code"
    module.insert(
        "oauth_exchange".to_string(),
        Value::NativeFunction {
            name: "oauth_exchange".to_string(),
            arity: 4,
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 4 {
                    return Err(IntentError::type_error(
                        "[auth] oauth_exchange() requires (provider, code, state, redirect_uri)"
                            .to_string(),
                    ));
                }

                // Parse provider config
                let provider = match value_to_provider(&args[0]) {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!("Invalid provider: {}", e))))
                    }
                };

                let code = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] code must be a string".to_string(),
                        ))
                    }
                };

                let state = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] state must be a string".to_string(),
                        ))
                    }
                };

                let redirect_uri = match &args[3] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] redirect_uri must be a string".to_string(),
                        ))
                    }
                };

                // Validate state (CSRF protection) - consume_oauth_state retrieves and deletes
                let oauth_state = match consume_oauth_state(&state) {
                    Some(s) => s,
                    None => {
                        return Ok(make_err(Value::String(
                            "Invalid or expired state parameter (CSRF check failed)".to_string(),
                        )))
                    }
                };

                // Exchange code for tokens
                let tokens = match exchange_code_for_tokens(
                    &provider,
                    &code,
                    &redirect_uri,
                    oauth_state.pkce_verifier.as_deref(),
                ) {
                    Ok(t) => t,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!(
                            "Token exchange failed: {}",
                            e
                        ))))
                    }
                };

                // Fetch user info
                let user_info = match fetch_userinfo(&provider, &tokens.access_token) {
                    Ok(info) => info,
                    Err(e) => {
                        return Ok(make_err(Value::String(format!(
                            "Failed to fetch user info: {}",
                            e
                        ))))
                    }
                };

                // Build result map
                let mut result = HashMap::new();
                result.insert(
                    "access_token".to_string(),
                    Value::String(tokens.access_token.clone()),
                );
                if let Some(ref rt) = tokens.refresh_token {
                    result.insert("refresh_token".to_string(), Value::String(rt.clone()));
                }
                if let Some(exp) = tokens.expires_in {
                    result.insert("expires_in".to_string(), Value::Int(exp));
                }
                if let Some(ref scope) = tokens.scope {
                    result.insert("scope".to_string(), Value::String(scope.clone()));
                }
                result.insert("user_info".to_string(), Value::Map(user_info));

                Ok(make_ok(Value::Map(result)))
            },
        },
    );

    // @ntnt create_session_from_oauth
    // @module std/auth
    // @signature create_session_from_oauth(provider_name: String, user_info: Map, tokens?: Map) -> Result<Map, String>
    // Create a session from OAuth user info and tokens.
    //
    // Use this after oauth_exchange to create a session. Returns the session info
    // and Set-Cookie header value.
    // @param provider_name Name of the provider (for user_id prefix)
    // @param user_info User info map from oauth_exchange
    // @param tokens Optional tokens map from oauth_exchange
    // @returns Ok(map with session_id, user_id, cookie) or Err on failure
    // @see_also oauth_exchange, get_session
    // @since v0.3.11
    // @tags #auth, #oauth, #session
    // @example create_session_from_oauth("github", user_info, tokens) => Ok({session_id: "...", cookie: "..."}) ~ "Create session"
    module.insert(
        "create_session_from_oauth".to_string(),
        Value::NativeFunction {
            name: "create_session_from_oauth".to_string(),
            arity: 0, // Variadic: 2-3 args
            max_arity: 0,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] create_session_from_oauth() requires (provider_name, user_info, tokens?)".to_string()
                    ));
                }

                let config = match get_auth_config() {
                    Some(c) => c,
                    None => return Ok(make_err(Value::String("Auth not initialized. Call enable_auth() first.".to_string()))),
                };

                let provider_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::type_error("[auth] provider_name must be a string".to_string())),
                };

                let user_info = match &args[1] {
                    Value::Map(m) => m.clone(),
                    _ => return Err(IntentError::type_error("[auth] user_info must be a map".to_string())),
                };

                // Parse optional tokens
                let tokens = if let Some(Value::Map(t)) = args.get(2) {
                    let access_token = t.get("access_token")
                        .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                        .unwrap_or_default();
                    let refresh_token = t.get("refresh_token")
                        .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None });
                    let expires_in = t.get("expires_in")
                        .and_then(|v| if let Value::Int(i) = v { Some(*i) } else { None });
                    let scope = t.get("scope")
                        .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None });
                    let token_type = t.get("token_type")
                        .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None })
                        .unwrap_or_else(|| "Bearer".to_string());
                    let id_token = t.get("id_token")
                        .and_then(|v| if let Value::String(s) = v { Some(s.clone()) } else { None });

                    Some(TokenResponse {
                        access_token,
                        refresh_token,
                        expires_in,
                        token_type,
                        scope,
                        id_token,
                    })
                } else {
                    None
                };

                // Create session
                let session = match create_session(&provider_name, user_info, tokens.as_ref(), config.session_ttl) {
                    Ok(s) => s,
                    Err(e) => return Ok(make_err(Value::String(e))),
                };

                // Store session
                let session_id = session.id.clone();
                let user_id = session.user_id.clone();
                store_session(session);

                let cookie = build_signed_session_cookie(&config, &session_id, None)
                    .map_err(IntentError::type_error)?;

                // Return result
                let mut result = HashMap::new();
                result.insert("session_id".to_string(), Value::String(session_id));
                result.insert("user_id".to_string(), Value::String(user_id));
                result.insert("cookie".to_string(), Value::String(cookie));

                Ok(make_ok(Value::Map(result)))
            },
        },
    );

    // @ntnt sign_in_session
    // @module std/auth
    // @signature sign_in_session(response: Response, session: Map, options?: Map) -> Response
    // Persist a session and attach the auth cookie to an existing response.
    //
    // Use this after password, magic-link, or other non-OAuth login flows. The
    // session map must include `subject_id`, and may optionally include `provider`,
    // `email`, `name`, `picture`, `claims`, `data`, or `raw`.
    // @param response The Response map to attach the session cookie to
    // @param session Session data map, including required `subject_id`
    // @param options Optional map with `session_ttl` and cookie override keys (`cookie_path`, `cookie_same_site`, `cookie_secure`, `cookie_http_only`, `cookie_max_age`)
    // @returns Response with a persisted session and Set-Cookie header
    // @see_also sign_out_session, current_session, rotate_session
    // @since v0.4.9
    // @tags #auth, #session
    // @example sign_in_session(redirect("/admin"), map { "subject_id": user.id, "claims": map { "role": "admin" } }) ~ "Sign in and redirect"
    module.insert(
        "sign_in_session".to_string(),
        Value::NativeFunction {
            name: "sign_in_session".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] sign_in_session() requires response, session, and optional options"
                            .to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before sign_in_session()."
                            .to_string(),
                    )
                })?;

                let session_spec = match &args[1] {
                    Value::Map(map) => map.clone(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] sign_in_session() session must be a map, got {}",
                            other.type_name()
                        )))
                    }
                };

                let options = match args.get(2) {
                    Some(Value::Map(map)) => Some(map.clone()),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] sign_in_session() options must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => None,
                };

                let session_ttl = match options.as_ref().and_then(|m| m.get("session_ttl")) {
                    Some(Value::Int(i)) => *i,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] sign_in_session() session_ttl must be an int, got {}",
                            other.type_name()
                        )))
                    }
                    None => config.session_ttl,
                };

                let session = create_manual_session(&session_spec, session_ttl)
                    .map_err(IntentError::type_error)?;
                let session_id = session.id.clone();

                let cookie = build_signed_session_cookie(&config, &session_id, options.as_ref())
                    .map_err(IntentError::type_error)?;
                let response =
                    add_set_cookie_header(&args[0], &cookie).map_err(IntentError::type_error)?;

                store_session(session);
                Ok(response)
            },
        },
    );

    // @ntnt rotate_session
    // @module std/auth
    // @signature rotate_session(response: Response, req: Request, options?: Map) -> Response
    // Rotate the current session ID and attach the new auth cookie to a response.
    //
    // Use this after privilege changes or sensitive login completion to prevent
    // session fixation while preserving the existing session payload.
    // @param response The Response map to attach the rotated cookie to
    // @param req The current HTTP request
    // @param options Optional cookie override keys (`cookie_path`, `cookie_same_site`, `cookie_secure`, `cookie_http_only`, `cookie_max_age`)
    // @returns Response with the rotated session cookie
    // @see_also sign_in_session, sign_out_session, current_session
    // @since v0.4.9
    // @tags #auth, #session, #security
    // @example rotate_session(redirect("/admin"), req) ~ "Rotate session after elevated auth"
    module.insert(
        "rotate_session".to_string(),
        Value::NativeFunction {
            name: "rotate_session".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] rotate_session() requires response, request, and optional options"
                            .to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before rotate_session()."
                            .to_string(),
                    )
                })?;

                let options = match args.get(2) {
                    Some(Value::Map(map)) => Some(map.clone()),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] rotate_session() options must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => None,
                };

                let session_id = get_session_id_from_request(&args[1]).ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] rotate_session() requires an active session".to_string(),
                    )
                })?;
                let rotated =
                    build_rotated_session(&session_id).map_err(IntentError::runtime_error)?;

                let cookie = build_signed_session_cookie(&config, &rotated.id, options.as_ref())
                    .map_err(IntentError::type_error)?;
                let response =
                    add_set_cookie_header(&args[0], &cookie).map_err(IntentError::type_error)?;

                migrate_session(&session_id, &rotated).map_err(IntentError::runtime_error)?;
                Ok(response)
            },
        },
    );

    // @ntnt sign_out_session
    // @module std/auth
    // @signature sign_out_session(response: Response, req: Request, options?: Map) -> Response
    // Revoke the current session and attach a clearing auth cookie to a response.
    //
    // Use this when your app wants logout behavior without being forced into the
    // built-in redirect handler.
    // @param response The Response map to attach the clearing cookie to
    // @param req The current HTTP request
    // @param options Optional cookie override keys (`cookie_path`, `cookie_same_site`, `cookie_secure`, `cookie_http_only`)
    // @returns Response with the auth cookie cleared
    // @see_also sign_in_session, rotate_session, current_user
    // @since v0.4.9
    // @tags #auth, #session
    // @example sign_out_session(redirect("/login"), req) ~ "Sign out and redirect"
    module.insert(
        "sign_out_session".to_string(),
        Value::NativeFunction {
            name: "sign_out_session".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] sign_out_session() requires response, request, and optional options"
                            .to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before sign_out_session()."
                            .to_string(),
                    )
                })?;

                let options = match args.get(2) {
                    Some(Value::Map(map)) => Some(map.clone()),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] sign_out_session() options must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => None,
                };

                let cookie = build_cleared_session_cookie(&config, options.as_ref())
                    .map_err(IntentError::type_error)?;
                let response =
                    add_set_cookie_header(&args[0], &cookie).map_err(IntentError::type_error)?;

                if let Some(session_id) = get_session_id_from_request(&args[1]) {
                    delete_session_by_id(&session_id);
                }

                Ok(response)
            },
        },
    );

    // ==========================================================================
    // Convenience Handlers (optional, use primitives for more control)
    // ==========================================================================

    // @ntnt auth_start
    // @module std/auth
    // @signature auth_start(req: Request) -> Response
    // Handle OAuth login start - redirects to the provider's authorization page.
    //
    // Use with a route like GET /auth/{provider}. Reads the provider name from
    // req.params.provider and generates the OAuth authorization URL with PKCE/nonce.
    // @param req The HTTP request with route param {provider}
    // @returns Redirect response to OAuth provider
    // @see_also enable_auth, auth_callback
    // @since v0.3.11
    // @tags #auth, #oauth, #handler
    // @example get("/auth/{provider}", auth_start) ~ "Wire up login routes"
    module.insert(
        "auth_start".to_string(),
        Value::NativeFunction {
            name: "auth_start".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| handle_auth_start(&args),
        },
    );

    // @ntnt auth_callback
    // @module std/auth
    // @signature auth_callback(req: Request) -> Response
    // Handle OAuth callback - exchanges code for tokens, creates session.
    //
    // Use with a route like GET /auth/{provider}/callback. Reads state and code from query params,
    // validates CSRF, exchanges code for tokens, and creates a user session.
    // @param req The HTTP request with query params state and code
    // @returns Redirect response to after_login URL with session cookie
    // @see_also enable_auth, auth_start
    // @since v0.3.11
    // @tags #auth, #oauth, #handler
    // @example get("/auth/{provider}/callback", auth_callback) ~ "Wire up callback route"
    module.insert(
        "auth_callback".to_string(),
        Value::NativeFunction {
            name: "auth_callback".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| handle_auth_callback(&args),
        },
    );

    // @ntnt auth_logout
    // @module std/auth
    // @signature auth_logout(req: Request) -> Response
    // Handle logout - clears the session and redirects.
    //
    // Use with a route like POST /auth/logout. Clears the session cookie and
    // redirects to after_logout URL.
    // @param req The HTTP request
    // @returns Redirect response to after_logout URL
    // @see_also enable_auth, get_user
    // @since v0.3.11
    // @tags #auth, #handler
    // @example post("/auth/logout", auth_logout) ~ "Wire up logout route"
    module.insert(
        "auth_logout".to_string(),
        Value::NativeFunction {
            name: "auth_logout".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| handle_auth_logout(&args),
        },
    );

    // @ntnt auth_me
    // @module std/auth
    // @signature auth_me(req: Request) -> Response
    // Return current user as JSON for SPAs.
    //
    // Use with a route like GET /auth/me. Returns the current user's session
    // data as JSON, or 401 if not authenticated.
    // @param req The HTTP request
    // @returns JSON response with user data or 401
    // @see_also get_user, enable_auth
    // @since v0.3.11
    // @tags #auth, #handler, #api
    // @example get("/auth/me", auth_me) ~ "Wire up user endpoint"
    module.insert(
        "auth_me".to_string(),
        Value::NativeFunction {
            name: "auth_me".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let user = get_user_from_request(&args[0]);
                match user {
                    Some(u) => Ok(json_response(Value::Map(u), 200)),
                    None => Ok(json_response(
                        Value::Map({
                            let mut m = HashMap::new();
                            m.insert(
                                "error".to_string(),
                                Value::String("Not authenticated".to_string()),
                            );
                            m
                        }),
                        401,
                    )),
                }
            },
        },
    );

    // @ntnt totp_secret
    // @module std/auth
    // @signature totp_secret() -> String
    // Generate a new TOTP secret for MFA setup.
    //
    // Creates a random base32-encoded secret suitable for TOTP authentication.
    // Use this secret with totp_uri() to generate a QR code for authenticator apps.
    // @returns Base32-encoded TOTP secret
    // @see_also totp_uri, verify_totp
    // @since v0.3.11
    // @tags #auth, #mfa, #totp
    // @example totp_secret() => "JBSWY3DPEHPK3PXP..." ~ "Generate secret"
    module.insert(
        "totp_secret".to_string(),
        Value::NativeFunction {
            name: "totp_secret".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| Ok(Value::String(generate_totp_secret())),
        },
    );

    // @ntnt totp_uri
    // @module std/auth
    // @signature totp_uri(secret: String, email: String, issuer: String) -> Result<String, String>
    // Generate an otpauth:// URI for QR codes.
    //
    // Creates a URI that can be encoded as a QR code for authenticator apps
    // like Google Authenticator or Authy.
    // @param secret TOTP secret (base32 encoded)
    // @param email User's email for the account label
    // @param issuer App name shown in authenticator
    // @returns Ok(uri) on success, Err(message) on failure
    // @see_also totp_secret, verify_totp
    // @since v0.3.11
    // @tags #auth, #mfa, #totp
    // @example totp_uri(secret, "user@example.com", "MyApp") => Ok("otpauth://...") ~ "Get URI for QR"
    module.insert(
        "totp_uri".to_string(),
        Value::NativeFunction {
            name: "totp_uri".to_string(),
            arity: 3,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 3 {
                    return Err(IntentError::type_error(
                        "[auth] totp_uri() requires (secret, email, issuer)".to_string(),
                    ));
                }

                let secret = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] secret must be a string".to_string(),
                        ))
                    }
                };

                let email = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] email must be a string".to_string(),
                        ))
                    }
                };

                let issuer = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] issuer must be a string".to_string(),
                        ))
                    }
                };

                match get_totp_uri(&secret, &email, &issuer) {
                    Ok(uri) => Ok(make_ok(Value::String(uri))),
                    Err(e) => Ok(make_err(Value::String(e))),
                }
            },
        },
    );

    // @ntnt verify_totp
    // @module std/auth
    // @signature verify_totp(secret: String, code: String) -> Bool
    // Verify a TOTP code against a secret.
    //
    // Checks if the provided 6-digit code is valid for the given secret.
    // Allows for 30-second time window drift.
    // @param secret TOTP secret (base32 encoded)
    // @param code 6-digit code from authenticator app
    // @returns true if code is valid, false otherwise
    // @see_also totp_secret, totp_uri
    // @since v0.3.11
    // @tags #auth, #mfa, #totp
    // @example verify_totp(secret, "123456") => true ~ "Verify 2FA code"
    module.insert(
        "verify_totp".to_string(),
        Value::NativeFunction {
            name: "verify_totp".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::type_error(
                        "[auth] verify_totp() requires (secret, code)".to_string(),
                    ));
                }

                let secret = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] secret must be a string".to_string(),
                        ))
                    }
                };

                let code = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "[auth] code must be a string".to_string(),
                        ))
                    }
                };

                Ok(Value::Bool(verify_totp_code(&secret, &code, "")))
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    static AUTH_TEST_MUTEX: std::sync::LazyLock<std::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

    fn reset_auth_test_state() {
        let mut store = SESSION_STORE.lock().unwrap();
        store.sessions.clear();
        store.oauth_states.clear();
        store.exchange_tokens.clear();
        store.auth_challenges.clear();
        drop(store);
        *AUTH_CONFIG.lock().unwrap() = None;
        *SQLITE_CONN.lock().unwrap() = None;
        *POSTGRES_URL.lock().unwrap() = None;
        *REDIS_URL.lock().unwrap() = None;
        reset_protected_paths();
    }

    fn module_fn(module: &HashMap<String, Value>, name: &str) -> fn(&[Value]) -> Result<Value> {
        match module.get(name) {
            Some(Value::NativeFunction { func, .. }) => *func,
            other => panic!("expected native function {} got {:?}", name, other),
        }
    }

    fn cookie_header_from_response(response: &Value) -> String {
        let cookies = cookie_headers_from_response(response);
        cookies
            .first()
            .cloned()
            .unwrap_or_else(|| panic!("missing Set-Cookie header"))
    }

    fn cookie_headers_from_response(response: &Value) -> Vec<String> {
        let headers = match response {
            Value::Map(map) => match map.get("headers") {
                Some(Value::Map(headers)) => headers,
                other => panic!("expected headers map, got {:?}", other),
            },
            other => panic!("expected response map, got {:?}", other),
        };

        let cookie = headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
            .map(|(_, v)| v)
            .unwrap_or_else(|| panic!("missing Set-Cookie header"));

        match cookie {
            Value::String(s) => vec![s.split(';').next().unwrap().to_string()],
            Value::Array(values) => values
                .iter()
                .map(|value| match value {
                    Value::String(s) => s.split(';').next().unwrap().to_string(),
                    other => panic!("expected cookie string in array, got {:?}", other),
                })
                .collect(),
            other => panic!("expected Set-Cookie string/array, got {:?}", other),
        }
    }

    fn request_with_cookie(cookie: &str) -> Value {
        request_with_cookies(&[cookie])
    }

    fn request_with_cookies(cookies: &[&str]) -> Value {
        Value::Map(HashMap::from([(
            "headers".to_string(),
            Value::Map(HashMap::from([(
                "cookie".to_string(),
                Value::String(cookies.join("; ")),
            )])),
        )]))
    }

    fn init_test_auth(session_store: SessionStore) {
        let config = AuthConfig {
            session_secret: "test-secret".to_string(),
            cookie_secure: false,
            session_store,
            ..AuthConfig::default()
        };
        ensure_auth_session_store(&config).unwrap();
        init_auth(config);
    }

    #[test]
    fn test_exchange_token_store_consume_memory() {
        let mut store = InMemoryStore::new();

        store.set_exchange_token("test-token-123".to_string(), "session-abc".to_string());

        // Should return the session_id
        let result = store.get_exchange_token("test-token-123");
        assert_eq!(result.map(|s| s.as_str()), Some("session-abc"));

        // Delete it (simulating consume)
        store.delete_exchange_token("test-token-123");

        // Second consume — should return None (one-time use)
        assert_eq!(store.get_exchange_token("test-token-123"), None);
    }

    #[test]
    fn test_exchange_token_expired() {
        let mut store = InMemoryStore::new();

        // Insert with created_at=0 (far in the past, well beyond EXCHANGE_TOKEN_TTL)
        store
            .exchange_tokens
            .insert("expired-token".to_string(), ("session-xyz".to_string(), 0));

        assert_eq!(store.get_exchange_token("expired-token"), None);
    }

    #[test]
    fn test_exchange_token_not_found() {
        let store = InMemoryStore::new();
        assert_eq!(store.get_exchange_token("nonexistent"), None);
    }

    #[test]
    fn test_exchange_token_cleanup() {
        let mut store = InMemoryStore::new();

        // One fresh token, one expired
        store.set_exchange_token("fresh".to_string(), "session-1".to_string());
        store
            .exchange_tokens
            .insert("stale".to_string(), ("session-2".to_string(), 0));

        let now = chrono::Utc::now().timestamp();
        let removed = store.cleanup_expired_exchange_tokens(now);

        assert_eq!(removed, 1);
        assert_eq!(
            store.get_exchange_token("fresh").map(|s| s.as_str()),
            Some("session-1")
        );
        assert_eq!(store.get_exchange_token("stale"), None);
    }

    #[test]
    fn test_html_response_structure() {
        let resp = html_response("<p>Hello</p>");

        if let Value::Map(map) = resp {
            assert!(matches!(map.get("status"), Some(Value::Int(200))));

            match map.get("body") {
                Some(Value::String(s)) => assert_eq!(s, "<p>Hello</p>"),
                other => panic!("Expected body string, got {:?}", other),
            }

            if let Some(Value::Map(headers)) = map.get("headers") {
                match headers.get("Content-Type") {
                    Some(Value::String(s)) => assert_eq!(s, "text/html; charset=utf-8"),
                    other => panic!("Expected Content-Type header, got {:?}", other),
                }
                match headers.get("Cache-Control") {
                    Some(Value::String(s)) => assert_eq!(s, "no-store"),
                    other => panic!("Expected Cache-Control: no-store, got {:?}", other),
                }
            } else {
                panic!("Expected headers map");
            }
        } else {
            panic!("Expected response map");
        }
    }

    #[test]
    fn test_exchange_token_ttl_constant() {
        // Ensure the TTL constant is sensible (between 10s and 300s)
        assert!(EXCHANGE_TOKEN_TTL >= 10);
        assert!(EXCHANGE_TOKEN_TTL <= 300);
    }

    #[test]
    fn test_path_matches_protected_pattern_exact_and_subtree() {
        assert!(path_matches_protected_pattern("/admin", "/admin"));
        assert!(path_matches_protected_pattern("/admin", "/admin/*"));
        assert!(path_matches_protected_pattern("/admin/users", "/admin/*"));
        assert!(path_matches_protected_pattern("/admin/users", "admin/*"));
        assert!(!path_matches_protected_pattern("/adminish", "/admin/*"));
        assert!(!path_matches_protected_pattern(
            "/settings/profile",
            "/settings"
        ));
    }

    #[test]
    fn test_validate_provider_name_rejects_unsafe_names() {
        assert!(validate_provider_name("google").is_ok());
        assert!(validate_provider_name("google.workspace").is_ok());
        assert!(validate_provider_name("foo_bar-123").is_ok());
        assert!(validate_provider_name("google<script>").is_err());
        assert!(validate_provider_name("bad name").is_err());
    }

    #[test]
    fn test_begin_auth_challenge_sets_cookie_and_current_helper_reads_it() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let current_auth_challenge = module_fn(&module, "current_auth_challenge");
        let current_session = module_fn(&module, "current_session");

        let response = redirect_response("/admin/verify", None);
        let challenge_response = begin_auth_challenge(&[
            response,
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("kind".to_string(), Value::String("mfa_pending".to_string())),
                (
                    "data".to_string(),
                    Value::Map(HashMap::from([(
                        "next".to_string(),
                        Value::String("/admin".to_string()),
                    )])),
                ),
            ])),
        ])
        .unwrap();

        let cookie = cookie_header_from_response(&challenge_response);
        assert!(cookie.starts_with("ntnt_session_challenge="));
        let req = request_with_cookie(&cookie);

        let challenge = current_auth_challenge(&[req.clone()]).unwrap();
        match challenge {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Some");
                let challenge_map = match values.first() {
                    Some(Value::Map(map)) => map,
                    other => panic!("expected challenge map, got {:?}", other),
                };
                assert!(
                    matches!(challenge_map.get("kind"), Some(Value::String(kind)) if kind == "mfa_pending")
                );
                match challenge_map.get("data") {
                    Some(Value::Map(data)) => {
                        assert!(
                            matches!(data.get("next"), Some(Value::String(next)) if next == "/admin")
                        );
                    }
                    other => panic!("expected challenge data map, got {:?}", other),
                }
            }
            other => panic!("expected Some(challenge), got {:?}", other),
        }

        assert!(
            matches!(current_session(&[req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "None")
        );
    }

    #[test]
    fn test_begin_auth_challenge_rejects_missing_kind() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let err = begin_auth_challenge(&[
            redirect_response("/admin/verify", None),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap_err();

        assert!(
            format!("{}", err).contains("[auth] begin_auth_challenge() challenge.kind is required")
        );
        assert!(SESSION_STORE.lock().unwrap().auth_challenges.is_empty());
    }

    #[test]
    fn test_complete_auth_challenge_creates_session_and_consumes_challenge() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let complete_auth_challenge = module_fn(&module, "complete_auth_challenge");
        let current_auth_challenge = module_fn(&module, "current_auth_challenge");
        let current_session = module_fn(&module, "current_session");

        let started = begin_auth_challenge(&[
            redirect_response("/admin/verify", None),
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("kind".to_string(), Value::String("mfa_pending".to_string())),
            ])),
        ])
        .unwrap();
        let challenge_cookie = cookie_header_from_response(&started);
        let req = request_with_cookie(&challenge_cookie);

        let completed = complete_auth_challenge(&[
            redirect_response("/admin", None),
            req.clone(),
            Value::Map(HashMap::from([(
                "claims".to_string(),
                Value::Map(HashMap::from([(
                    "role".to_string(),
                    Value::String("admin".to_string()),
                )])),
            )])),
        ])
        .unwrap();

        let cookies = cookie_headers_from_response(&completed);
        let session_cookie = cookies
            .iter()
            .find(|cookie| cookie.starts_with("ntnt_session="))
            .cloned()
            .unwrap_or_else(|| panic!("missing session cookie"));
        let cleared_challenge_cookie = cookies
            .iter()
            .find(|cookie| cookie.starts_with("ntnt_session_challenge="))
            .cloned()
            .unwrap_or_else(|| panic!("missing cleared challenge cookie"));
        assert_eq!(cleared_challenge_cookie, "ntnt_session_challenge=");

        let session_req = request_with_cookie(&session_cookie);
        let session = current_session(&[session_req]).unwrap();
        match session {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Some");
                let session_map = match values.first() {
                    Some(Value::Map(map)) => map,
                    other => panic!("expected session map, got {:?}", other),
                };
                match session_map.get("data") {
                    Some(Value::Map(data)) => {
                        assert!(
                            matches!(data.get("role"), Some(Value::String(role)) if role == "admin")
                        );
                    }
                    other => panic!("expected session data map, got {:?}", other),
                }
            }
            other => panic!("expected Some(session), got {:?}", other),
        }

        assert!(
            matches!(current_auth_challenge(&[req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "None")
        );
    }

    #[test]
    fn test_complete_auth_challenge_invalid_response_keeps_challenge_active() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let complete_auth_challenge = module_fn(&module, "complete_auth_challenge");
        let current_auth_challenge = module_fn(&module, "current_auth_challenge");

        let started = begin_auth_challenge(&[
            redirect_response("/admin/verify", None),
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("kind".to_string(), Value::String("mfa_pending".to_string())),
            ])),
        ])
        .unwrap();
        let challenge_cookie = cookie_header_from_response(&started);
        let req = request_with_cookie(&challenge_cookie);

        let err =
            complete_auth_challenge(&[Value::String("not-a-response".to_string()), req.clone()])
                .unwrap_err();
        assert!(format!("{}", err).contains("[auth] response must be a map"));
        assert!(
            matches!(current_auth_challenge(&[req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "Some")
        );
    }

    #[test]
    fn test_cancel_auth_challenge_clears_cookie_and_removes_challenge() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let cancel_auth_challenge = module_fn(&module, "cancel_auth_challenge");
        let current_auth_challenge = module_fn(&module, "current_auth_challenge");

        let started = begin_auth_challenge(&[
            redirect_response("/admin/verify", None),
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("kind".to_string(), Value::String("mfa_pending".to_string())),
            ])),
        ])
        .unwrap();
        let challenge_cookie = cookie_header_from_response(&started);
        let req = request_with_cookie(&challenge_cookie);

        let cancelled =
            cancel_auth_challenge(&[redirect_response("/login", None), req.clone()]).unwrap();
        let cleared = cookie_header_from_response(&cancelled);
        assert_eq!(cleared, "ntnt_session_challenge=");
        assert!(
            matches!(current_auth_challenge(&[req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "None")
        );
    }

    #[test]
    fn test_auth_challenge_sqlite_store_get_consume_and_cleanup() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        let active = AuthChallenge {
            id: "challenge-active".to_string(),
            subject_id: "user-123".to_string(),
            provider: "local".to_string(),
            kind: "mfa_pending".to_string(),
            data_json: value_map_to_json_string(&HashMap::from([(
                "next".to_string(),
                Value::String("/admin".to_string()),
            )])),
            created_at: now,
            expires_at: now + 60,
        };

        store_auth_challenge(active.clone());
        let fetched = get_auth_challenge_by_id(&active.id)
            .expect("sqlite lookup should succeed")
            .expect("sqlite challenge should persist");
        assert_eq!(fetched.id, active.id);
        assert_eq!(fetched.subject_id, active.subject_id);
        assert_eq!(fetched.kind, active.kind);

        let consumed = consume_auth_challenge(&active.id)
            .expect("sqlite consume should succeed")
            .expect("sqlite challenge should be returned exactly once");
        assert_eq!(consumed.id, active.id);
        assert!(get_auth_challenge_by_id(&active.id)
            .expect("sqlite lookup after consume should succeed")
            .is_none());

        let expired = AuthChallenge {
            id: "challenge-expired".to_string(),
            subject_id: "user-456".to_string(),
            provider: "local".to_string(),
            kind: "password_reset".to_string(),
            data_json: "{}".to_string(),
            created_at: now - 120,
            expires_at: now - 60,
        };
        store_auth_challenge_sqlite(&expired).expect("sqlite expired insert should succeed");

        let removed = cleanup_expired_auth_challenges().expect("sqlite cleanup should succeed");
        assert_eq!(removed, 1);
        assert!(get_auth_challenge_by_id(&expired.id)
            .expect("sqlite lookup after cleanup should succeed")
            .is_none());
    }

    #[test]
    fn test_complete_auth_challenge_sqlite_backend_round_trip() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let complete_auth_challenge = module_fn(&module, "complete_auth_challenge");
        let current_auth_challenge = module_fn(&module, "current_auth_challenge");
        let current_session = module_fn(&module, "current_session");

        let started = begin_auth_challenge(&[
            redirect_response("/admin/verify", None),
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("provider".to_string(), Value::String("local".to_string())),
                ("kind".to_string(), Value::String("mfa_pending".to_string())),
            ])),
        ])
        .unwrap();
        let challenge_cookie = cookie_header_from_response(&started);
        let req = request_with_cookie(&challenge_cookie);

        assert!(
            matches!(current_auth_challenge(&[req.clone()]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "Some")
        );

        let completed = complete_auth_challenge(&[
            redirect_response("/admin", None),
            req.clone(),
            Value::Map(HashMap::from([(
                "claims".to_string(),
                Value::Map(HashMap::from([(
                    "role".to_string(),
                    Value::String("admin".to_string()),
                )])),
            )])),
        ])
        .unwrap();

        let session_cookie = cookie_headers_from_response(&completed)
            .into_iter()
            .find(|cookie| cookie.starts_with("ntnt_session="))
            .expect("session cookie should be set");
        let session_req = request_with_cookie(&session_cookie);

        assert!(
            matches!(current_auth_challenge(&[req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "None")
        );
        assert!(
            matches!(current_session(&[session_req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "Some")
        );
    }

    #[test]
    fn test_take_auth_challenge_removes_expired_entry() {
        let now = chrono::Utc::now().timestamp();
        let mut store = InMemoryStore::new();
        store.set_auth_challenge(AuthChallenge {
            id: "challenge-expired-memory".to_string(),
            subject_id: "user-123".to_string(),
            provider: "local".to_string(),
            kind: "mfa_pending".to_string(),
            data_json: "{}".to_string(),
            created_at: now - 120,
            expires_at: now - 60,
        });

        assert!(store
            .take_auth_challenge("challenge-expired-memory")
            .is_none());
        assert!(!store
            .auth_challenges
            .contains_key("challenge-expired-memory"));
    }

    #[test]
    fn test_cleanup_expired_auth_challenges_sqlite_also_cleans_memory_fallback() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE
            .lock()
            .unwrap()
            .set_auth_challenge(AuthChallenge {
                id: "challenge-memory-fallback".to_string(),
                subject_id: "user-123".to_string(),
                provider: "local".to_string(),
                kind: "mfa_pending".to_string(),
                data_json: "{}".to_string(),
                created_at: now - 120,
                expires_at: now - 60,
            });

        let removed = cleanup_expired_auth_challenges().expect("cleanup should succeed");
        assert_eq!(removed, 1);
        assert!(!SESSION_STORE
            .lock()
            .unwrap()
            .auth_challenges
            .contains_key("challenge-memory-fallback"));
    }

    #[test]
    fn test_consume_auth_challenge_uses_memory_fallback_when_backend_unavailable() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        let challenge = AuthChallenge {
            id: "challenge-memory-only".to_string(),
            subject_id: "user-123".to_string(),
            provider: "local".to_string(),
            kind: "mfa_pending".to_string(),
            data_json: "{}".to_string(),
            created_at: now,
            expires_at: now + 60,
        };
        SESSION_STORE
            .lock()
            .unwrap()
            .set_auth_challenge(challenge.clone());
        *SQLITE_CONN.lock().unwrap() = None;

        let consumed = consume_auth_challenge(&challenge.id)
            .expect("memory fallback consume should succeed")
            .expect("memory fallback challenge should be returned");
        assert_eq!(consumed.id, challenge.id);
        assert!(!SESSION_STORE
            .lock()
            .unwrap()
            .auth_challenges
            .contains_key(&challenge.id));
    }

    #[test]
    fn test_consume_auth_challenge_propagates_backend_error_without_memory_fallback() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));
        *SQLITE_CONN.lock().unwrap() = None;

        let err = consume_auth_challenge("missing-challenge")
            .expect_err("backend failure without fallback should surface error");
        assert!(err.contains("SQLite not initialized"));
    }

    #[test]
    fn test_get_auth_challenge_by_id_propagates_backend_error_without_memory_fallback() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));
        *SQLITE_CONN.lock().unwrap() = None;

        let err = get_auth_challenge_by_id("missing-challenge")
            .expect_err("backend lookup failure without fallback should surface error");
        assert!(err.contains("SQLite not initialized"));
    }

    #[test]
    fn test_sign_in_session_persists_claims_and_current_helpers_read_them() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let current_session = module_fn(&module, "current_session");
        let current_user = module_fn(&module, "current_user");

        let response = redirect_response("/admin", None);
        let signed_in = sign_in_session(&[
            response,
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("name".to_string(), Value::String("Alice".to_string())),
                (
                    "claims".to_string(),
                    Value::Map(HashMap::from([(
                        "role".to_string(),
                        Value::String("admin".to_string()),
                    )])),
                ),
            ])),
        ])
        .unwrap();

        let cookie = cookie_header_from_response(&signed_in);
        let req = request_with_cookie(&cookie);

        let session = current_session(&[req.clone()]).unwrap();
        let user = current_user(&[req]).unwrap();

        match session {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Some");
                let session_map = match values.first() {
                    Some(Value::Map(map)) => map,
                    other => panic!("expected session map, got {:?}", other),
                };
                match session_map.get("data") {
                    Some(Value::Map(data)) => match data.get("role") {
                        Some(Value::String(role)) => assert_eq!(role, "admin"),
                        other => panic!("expected role string, got {:?}", other),
                    },
                    other => panic!("expected session data map, got {:?}", other),
                }
            }
            other => panic!("expected Some(session), got {:?}", other),
        }

        match user {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Some");
                let user_map = match values.first() {
                    Some(Value::Map(map)) => map,
                    other => panic!("expected user map, got {:?}", other),
                };
                match user_map.get("name") {
                    Some(Value::String(name)) => assert_eq!(name, "Alice"),
                    other => panic!("expected user name, got {:?}", other),
                }
            }
            other => panic!("expected Some(user), got {:?}", other),
        }
    }

    #[test]
    fn test_sign_in_session_rejects_missing_subject_id() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let err = sign_in_session(&[
            redirect_response("/admin", None),
            Value::Map(HashMap::from([(
                "provider".to_string(),
                Value::String("local".to_string()),
            )])),
        ])
        .unwrap_err();

        assert!(
            format!("{}", err).contains("[auth] sign_in_session() session.subject_id is required")
        );
    }

    #[test]
    fn test_sign_in_session_does_not_persist_when_response_invalid() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let err = sign_in_session(&[
            Value::String("not-a-response".to_string()),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap_err();

        assert!(format!("{}", err).contains("[auth] response must be a map"));
        assert!(SESSION_STORE.lock().unwrap().sessions.is_empty());
    }

    #[test]
    fn test_sign_in_session_rejects_cookie_name_override() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let err = sign_in_session(&[
            redirect_response("/admin", None),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
            Value::Map(HashMap::from([(
                "cookie_name".to_string(),
                Value::String("other_cookie".to_string()),
            )])),
        ])
        .unwrap_err();

        assert!(format!("{}", err).contains("cookie_name override is not supported"));
        assert!(SESSION_STORE.lock().unwrap().sessions.is_empty());
    }

    #[test]
    fn test_rotate_session_changes_id_and_invalidates_old_cookie() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let rotate_session = module_fn(&module, "rotate_session");
        let current_session = module_fn(&module, "current_session");

        let signed_in = sign_in_session(&[
            redirect_response("/admin", None),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap();
        let old_cookie = cookie_header_from_response(&signed_in);
        let old_req = request_with_cookie(&old_cookie);
        let old_session = current_session(&[old_req.clone()]).unwrap();
        let old_id = match old_session {
            Value::EnumValue { values, .. } => match values.first() {
                Some(Value::Map(map)) => match map.get("id") {
                    Some(Value::String(id)) => id.clone(),
                    other => panic!("expected session id, got {:?}", other),
                },
                other => panic!("expected session map, got {:?}", other),
            },
            other => panic!("expected Some(session), got {:?}", other),
        };

        let rotated =
            rotate_session(&[redirect_response("/admin", None), old_req.clone()]).unwrap();
        let new_cookie = cookie_header_from_response(&rotated);
        let new_req = request_with_cookie(&new_cookie);
        let new_session = current_session(&[new_req]).unwrap();
        let new_id = match new_session {
            Value::EnumValue { values, .. } => match values.first() {
                Some(Value::Map(map)) => match map.get("id") {
                    Some(Value::String(id)) => id.clone(),
                    other => panic!("expected session id, got {:?}", other),
                },
                other => panic!("expected session map, got {:?}", other),
            },
            other => panic!("expected Some(session), got {:?}", other),
        };

        assert_ne!(old_id, new_id);
        assert!(get_session_by_id(&old_id).is_none());
    }

    #[test]
    fn test_rotate_session_requires_active_session() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let rotate_session = module_fn(&module, "rotate_session");
        let err = rotate_session(&[
            redirect_response("/admin", None),
            request_with_cookie("ntnt_session=invalid"),
        ])
        .unwrap_err();

        assert!(format!("{}", err).contains("[auth] rotate_session() requires an active session"));
    }

    #[test]
    fn test_rotate_session_invalid_response_keeps_existing_session() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let rotate_session = module_fn(&module, "rotate_session");
        let current_session = module_fn(&module, "current_session");

        let signed_in = sign_in_session(&[
            redirect_response("/admin", None),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap();
        let cookie = cookie_header_from_response(&signed_in);
        let req = request_with_cookie(&cookie);
        let before = current_session(&[req.clone()]).unwrap();

        let err = rotate_session(&[Value::String("not-a-response".to_string()), req.clone()])
            .unwrap_err();
        assert!(format!("{}", err).contains("[auth] response must be a map"));

        let after = current_session(&[req]).unwrap();
        match (before, after) {
            (
                Value::EnumValue {
                    values: before_values,
                    ..
                },
                Value::EnumValue {
                    values: after_values,
                    ..
                },
            ) => {
                let before_id = match before_values.first() {
                    Some(Value::Map(map)) => match map.get("id") {
                        Some(Value::String(id)) => id.clone(),
                        other => panic!("expected session id, got {:?}", other),
                    },
                    other => panic!("expected session map, got {:?}", other),
                };
                let after_id = match after_values.first() {
                    Some(Value::Map(map)) => match map.get("id") {
                        Some(Value::String(id)) => id.clone(),
                        other => panic!("expected session id, got {:?}", other),
                    },
                    other => panic!("expected session map, got {:?}", other),
                };
                assert_eq!(before_id, after_id);
            }
            other => panic!("expected session before/after, got {:?}", other),
        }
    }

    #[test]
    fn test_sign_out_session_clears_cookie_and_removes_session() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let sign_out_session = module_fn(&module, "sign_out_session");
        let current_session = module_fn(&module, "current_session");

        let signed_in = sign_in_session(&[
            redirect_response("/admin", None),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap();
        let cookie = cookie_header_from_response(&signed_in);
        let req = request_with_cookie(&cookie);

        let signed_out = sign_out_session(&[
            redirect_response("/login", None),
            req.clone(),
            Value::Map(HashMap::from([(
                "cookie_max_age".to_string(),
                Value::Int(3600),
            )])),
        ])
        .unwrap();
        let set_cookie = match signed_out {
            Value::Map(map) => match map.get("headers") {
                Some(Value::Map(headers)) => headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case("set-cookie"))
                    .map(|(_, v)| v.clone())
                    .unwrap_or_else(|| panic!("missing Set-Cookie header")),
                other => panic!("expected headers map, got {:?}", other),
            },
            other => panic!("expected response map, got {:?}", other),
        };
        match set_cookie {
            Value::String(s) => assert!(s.contains("Max-Age=0")),
            other => panic!("expected cookie string, got {:?}", other),
        }

        assert!(
            matches!(current_session(&[req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "None")
        );
    }

    #[test]
    fn test_sign_out_session_invalid_response_keeps_session_active() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let sign_out_session = module_fn(&module, "sign_out_session");
        let current_session = module_fn(&module, "current_session");

        let signed_in = sign_in_session(&[
            redirect_response("/admin", None),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap();
        let cookie = cookie_header_from_response(&signed_in);
        let req = request_with_cookie(&cookie);

        let err = sign_out_session(&[Value::String("not-a-response".to_string()), req.clone()])
            .unwrap_err();
        assert!(format!("{}", err).contains("[auth] response must be a map"));
        assert!(
            matches!(current_session(&[req]).unwrap(), Value::EnumValue { ref variant, .. } if variant == "Some")
        );
    }

    #[test]
    fn test_value_to_provider_rejects_unsafe_names() {
        let provider = Value::Map(HashMap::from([
            (
                "name".to_string(),
                Value::String("google<script>".to_string()),
            ),
            ("client_id".to_string(), Value::String("id".to_string())),
            (
                "client_secret".to_string(),
                Value::String("secret".to_string()),
            ),
            (
                "authorize_url".to_string(),
                Value::String("https://example.com/auth".to_string()),
            ),
            (
                "token_url".to_string(),
                Value::String("https://example.com/token".to_string()),
            ),
        ]));

        assert!(value_to_provider(&provider).is_err());
    }

    #[test]
    fn test_handle_auth_index_encodes_provider_links() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        init_auth(AuthConfig {
            providers: vec![
                ProviderConfig {
                    name: "good".to_string(),
                    ..ProviderConfig::default()
                },
                ProviderConfig {
                    name: "bad\" onclick=\"alert(1)".to_string(),
                    ..ProviderConfig::default()
                },
            ],
            ..AuthConfig::default()
        });

        let response = handle_auth_index(&[]).unwrap();
        let body = match response {
            Value::Map(map) => match map.get("body") {
                Some(Value::String(body)) => body.clone(),
                other => panic!("expected body string, got {:?}", other),
            },
            other => panic!("expected response map, got {:?}", other),
        };

        assert!(body.contains("/auth/bad%22%20onclick%3D%22alert%281%29"));
        assert!(body.contains("Sign in with bad&quot; onclick=&quot;alert(1)"));
        assert!(!body.contains("href=\"/auth/bad\" onclick"));
    }

    #[test]
    fn test_register_protected_paths_normalizes_missing_leading_slash() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_protected_paths();
        register_protected_paths(&["admin/*".to_string()]);
        assert_eq!(get_protected_paths(), vec!["/admin/*".to_string()]);
    }

    #[test]
    fn test_enforce_auth_for_request_redirects_html_routes() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_protected_paths();
        init_auth(AuthConfig {
            providers: vec![ProviderConfig {
                name: "google".to_string(),
                ..ProviderConfig::default()
            }],
            protected_paths: vec!["/admin/*".to_string()],
            ..AuthConfig::default()
        });

        let req = Value::Map(HashMap::from([
            ("path".to_string(), Value::String("/admin".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "accept".to_string(),
                    Value::String("text/html".to_string()),
                )])),
            ),
        ]));

        let response = enforce_auth_for_request(&req, false).unwrap_err();
        if let Value::Map(map) = response {
            assert!(matches!(map.get("status"), Some(Value::Int(302))));
        } else {
            panic!("expected response map");
        }
    }

    #[test]
    fn test_enforce_auth_for_request_returns_json_for_api_routes() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_protected_paths();
        init_auth(AuthConfig {
            providers: vec![ProviderConfig {
                name: "google".to_string(),
                ..ProviderConfig::default()
            }],
            protected_paths: vec!["/admin/*".to_string()],
            ..AuthConfig::default()
        });

        let req = Value::Map(HashMap::from([
            ("path".to_string(), Value::String("/admin/data".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "accept".to_string(),
                    Value::String("application/json".to_string()),
                )])),
            ),
        ]));

        let response = enforce_auth_for_request(&req, false).unwrap_err();
        if let Value::Map(map) = response {
            assert!(matches!(map.get("status"), Some(Value::Int(401))));
            match map.get("body") {
                Some(Value::String(body)) => assert!(body.contains("unauthorized")),
                other => panic!("expected json body, got {:?}", other),
            }
        } else {
            panic!("expected response map");
        }
    }

    #[test]
    fn test_enforce_auth_for_request_skips_auth_routes_even_when_forced() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_protected_paths();
        init_auth(AuthConfig {
            providers: vec![ProviderConfig {
                name: "google".to_string(),
                ..ProviderConfig::default()
            }],
            protected_paths: vec!["/admin/*".to_string()],
            ..AuthConfig::default()
        });

        let req = Value::Map(HashMap::from([(
            "path".to_string(),
            Value::String("/auth/google".to_string()),
        )]));

        assert!(enforce_auth_for_request(&req, true).is_ok());
    }
}
