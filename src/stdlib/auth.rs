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
mod local;
mod oauth;
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
    normalize_cookie_same_site,
};
#[cfg(test)]
use guards::path_matches_protected_pattern;
use guards::validate_auth_challenge_kind;
pub use guards::{
    enforce_auth_for_request, get_protected_paths, register_protected_paths, reset_protected_paths,
};
use local::{
    begin_totp_enrollment_record, bootstrap_local_user_record, confirm_totp_enrollment_record,
    consume_password_reset_record, issue_password_reset_record, local_identity_to_safe_value,
    local_user_record, reset_totp_record, set_local_password_record, totp_status_record,
    update_local_user_metadata_record, verified_local_password_to_value,
    verify_local_password_record, verify_local_totp_record,
};
use oauth::extract_user_info;
pub use oauth::{
    client_credentials_grant, decode_id_token, exchange_code_for_tokens, fetch_oidc_discovery,
    fetch_userinfo, generate_auth_url, generate_pkce_challenge, generate_pkce_verifier,
    introspect_token, refresh_access_token, validate_id_token_claims,
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
use request_helpers::{
    get_host_and_proto, get_user_from_request, request_device_name, request_ip_hash,
    request_user_agent_hash,
};
pub use routes::{
    handle_auth_callback, handle_auth_health, handle_auth_index, handle_auth_logout,
    handle_auth_protect, handle_auth_start,
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

const AUTH_CHALLENGE_CSRF_DATA_KEY: &str = "auth.challenge_csrf";

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
    pub device_name: Option<String>,
    pub user_agent_hash: Option<String>,
    pub last_ip_hash: Option<String>,
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
    /// Server-side nonce for pre-session staged auth forms. This must not be
    /// exposed through the app-owned `data` map returned by current_auth_challenge().
    pub csrf_token: String,
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
    pub remember_me: bool,
    pub device_name: Option<String>,
    pub user_agent_hash: Option<String>,
    pub last_ip_hash: Option<String>,
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
    pub route_prefix: String,
    pub login_page_enabled: bool,
    pub login_page_title: String,
    pub login_page_logo_url: Option<String>,
    pub login_page_heading: String,
    pub login_page_copy: String,
    pub cookie_name: String,
    pub cookie_secure: bool,
    pub cookie_same_site: String,
    pub session_ttl: i64,
    pub refresh_ttl: i64, // How long refresh tokens can extend sessions (default: 30 days)
    pub sliding_sessions: bool, // Extend active sessions on authenticated reads
    pub refresh_throttle: i64, // Only extend sliding sessions when remaining TTL is at or below this many seconds
    pub max_session_ttl: Option<i64>, // Absolute max lifetime from session creation
    pub auth_preset: Option<String>, // Lifecycle/security preset name, if selected
    pub store_tokens: bool,    // Store access/refresh tokens in session
    pub health_endpoint: bool, // Expose /auth/health diagnostics (dev-only default unless explicitly enabled)
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
            route_prefix: "/auth".to_string(),
            login_page_enabled: true,
            login_page_title: "Sign in".to_string(),
            login_page_logo_url: None,
            login_page_heading: "Sign in".to_string(),
            login_page_copy: "Choose a provider to continue.".to_string(),
            cookie_name: "ntnt_session".to_string(),
            cookie_secure: true,
            cookie_same_site: "lax".to_string(),
            session_ttl: 86400 * 7,  // 7 days
            refresh_ttl: 86400 * 30, // 30 days — how long refresh tokens can extend sessions
            sliding_sessions: false,
            refresh_throttle: 300,
            max_session_ttl: None,
            auth_preset: None,
            store_tokens: false,
            health_endpoint: false,
            session_secret: DEFAULT_SESSION_SECRET_SENTINEL.to_string(),
            session_store: SessionStore::Memory,
        }
    }
}

fn auth_preset_config(name: &str) -> std::result::Result<AuthConfig, String> {
    let preset = name.trim().to_ascii_lowercase();
    let mut config = AuthConfig::default();
    config.auth_preset = Some(preset.clone());

    match preset.as_str() {
        "consumer" => {
            config.session_ttl = 86400 * 14;
            config.refresh_ttl = 86400 * 30;
            config.sliding_sessions = true;
            config.refresh_throttle = 1800;
            config.max_session_ttl = Some(86400 * 30);
            config.cookie_same_site = "Lax".to_string();
        }
        "admin" => {
            config.session_ttl = 3600;
            config.refresh_ttl = 86400;
            config.sliding_sessions = true;
            config.refresh_throttle = 300;
            config.max_session_ttl = Some(86400);
            config.cookie_same_site = "Strict".to_string();
        }
        "internal" => {
            config.session_ttl = 86400;
            config.refresh_ttl = 86400 * 7;
            config.sliding_sessions = true;
            config.refresh_throttle = 900;
            config.max_session_ttl = Some(86400 * 7);
            config.cookie_same_site = "Lax".to_string();
        }
        "strict" => {
            config.session_ttl = 1800;
            config.refresh_ttl = 1800;
            config.sliding_sessions = false;
            config.refresh_throttle = 0;
            config.max_session_ttl = Some(1800);
            config.cookie_same_site = "Strict".to_string();
        }
        _ => {
            return Err(format!(
                "[auth] unknown preset \"{}\". Expected one of: consumer, admin, internal, strict",
                name
            ))
        }
    }

    Ok(config)
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

// ============================================================================
// SECTION 7: Session Management
// ============================================================================

/// Session info for listing (excludes sensitive token data)
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub user_id: String,
    pub provider: String,
    pub device_name: Option<String>,
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
    local_auth: storage::LocalAuthMemoryStore,
}

impl InMemoryStore {
    fn new() -> Self {
        InMemoryStore {
            sessions: HashMap::new(),
            oauth_states: HashMap::new(),
            exchange_tokens: HashMap::new(),
            auth_challenges: HashMap::new(),
            local_auth: storage::LocalAuthMemoryStore::default(),
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

    #[allow(dead_code)]
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

    #[allow(dead_code)]
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
                device_name: s.device_name.clone(),
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

fn validate_http_request_arg(function_name: &str, req: &Value) -> Result<()> {
    let req_map = match req {
        Value::Map(map) => map,
        other => {
            return Err(IntentError::type_error(format!(
                "[auth] {}() request must be an HTTP request map, got {}",
                function_name,
                other.type_name()
            )))
        }
    };

    for key in ["method", "path"] {
        match req_map.get(key) {
            Some(Value::String(_)) => {}
            Some(other) => {
                return Err(IntentError::type_error(format!(
                    "[auth] {}() request.{} must be a string, got {}",
                    function_name,
                    key,
                    other.type_name()
                )))
            }
            None => {
                return Err(IntentError::type_error(format!(
                    "[auth] {}() request must include {}",
                    function_name, key
                )))
            }
        }
    }

    match req_map.get("headers") {
        Some(Value::Map(_)) | None => Ok(()),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() request.headers must be a map, got {}",
            function_name,
            other.type_name()
        ))),
    }
}

fn optional_kind_arg(function_name: &str, args: &[Value], index: usize) -> Result<Option<String>> {
    match args.get(index) {
        Some(Value::String(kind)) => {
            Ok(Some(validate_auth_challenge_kind(kind).map_err(|err| {
                IntentError::type_error(err.replace("begin_auth_challenge", function_name))
            })?))
        }
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() kind must be a string, got {}",
            function_name,
            other.type_name()
        ))),
        None => Ok(None),
    }
}

fn active_auth_challenge_for_request(
    function_name: &str,
    req: &Value,
    expected_kind: Option<&str>,
) -> Result<Option<AuthChallenge>> {
    validate_http_request_arg(function_name, req)?;

    let Some(challenge_id) = get_auth_challenge_id_from_request(req) else {
        return Ok(None);
    };
    let Some(challenge) =
        get_auth_challenge_by_id(&challenge_id).map_err(IntentError::runtime_error)?
    else {
        return Ok(None);
    };
    if let Some(kind) = expected_kind {
        if challenge.kind != kind {
            return Ok(None);
        }
    }
    Ok(Some(challenge))
}

fn auth_challenge_csrf_token(challenge: &AuthChallenge) -> Option<String> {
    if !challenge.csrf_token.is_empty() {
        return Some(challenge.csrf_token.clone());
    }

    // Compatibility for challenges created by the pre-review implementation.
    // New challenges store this nonce in the dedicated csrf_token field so app-owned
    // continuation data cannot leak or be overwritten.
    let data = json_string_to_value_map(&challenge.data_json);
    match data.get(AUTH_CHALLENGE_CSRF_DATA_KEY) {
        Some(Value::String(token)) if !token.is_empty() => Some(token.clone()),
        _ => None,
    }
}

fn parse_local_identifier_options(function_name: &str, options: Option<&Value>) -> Result<String> {
    match options {
        Some(Value::Map(map)) => match map.get("identifier_kind") {
            Some(Value::String(kind)) => Ok(kind.clone()),
            Some(other) => Err(IntentError::type_error(format!(
                "[auth] {}() identifier_kind must be a string, got {}",
                function_name,
                other.type_name()
            ))),
            None => Ok("email".to_string()),
        },
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() options must be a map, got {}",
            function_name,
            other.type_name()
        ))),
        None => Ok("email".to_string()),
    }
}

fn parse_local_metadata_update_options(options: Option<&Value>) -> Result<(String, bool)> {
    match options {
        Some(Value::Map(map)) => {
            let identifier_kind = match map.get("identifier_kind") {
                Some(Value::String(kind)) => kind.clone(),
                Some(other) => {
                    let message = format!(
                        "[auth] update_local_user_metadata() identifier_kind must be a string, got {}",
                        other.type_name()
                    );
                    return Err(IntentError::type_error(message));
                }
                None => "email".to_string(),
            };
            let replace = match map.get("replace") {
                Some(Value::Bool(value)) => *value,
                Some(other) => {
                    return Err(IntentError::type_error(format!(
                        "[auth] update_local_user_metadata() replace must be a bool, got {}",
                        other.type_name()
                    )))
                }
                None => false,
            };
            Ok((identifier_kind, replace))
        }
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] update_local_user_metadata() options must be a map, got {}",
            other.type_name()
        ))),
        None => Ok(("email".to_string(), false)),
    }
}

fn require_auth_initialized_for(function_name: &str) -> Result<()> {
    get_auth_config().map(|_| ()).ok_or_else(|| {
        IntentError::runtime_error(format!(
            "[auth] Auth not initialized. Call enable_auth() before {}().",
            function_name
        ))
    })
}

fn string_arg<'a>(
    function_name: &str,
    args: &'a [Value],
    index: usize,
    name: &str,
) -> Result<&'a str> {
    match args.get(index) {
        Some(Value::String(value)) => Ok(value.as_str()),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() {} must be a string, got {}",
            function_name,
            name,
            other.type_name()
        ))),
        None => Err(IntentError::type_error(format!(
            "[auth] {}() missing required {}",
            function_name, name
        ))),
    }
}

fn parse_password_reset_issue_options(
    function_name: &str,
    options: Option<&Value>,
) -> Result<(String, Option<i64>)> {
    match options {
        Some(Value::Map(map)) => {
            let identifier_kind = optional_string_option(map, "identifier_kind", function_name)?
                .unwrap_or_else(|| "email".to_string());
            let ttl_seconds = match map.get("ttl_seconds") {
                Some(Value::Int(value)) => Some(*value),
                Some(other) => {
                    return Err(IntentError::type_error(format!(
                        "[auth] {}() ttl_seconds must be an int, got {}",
                        function_name,
                        other.type_name()
                    )))
                }
                None => None,
            };
            Ok((identifier_kind, ttl_seconds))
        }
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() options must be a map, got {}",
            function_name,
            other.type_name()
        ))),
        None => Ok(("email".to_string(), None)),
    }
}

fn parse_password_reset_consume_options(
    function_name: &str,
    options: Option<&Value>,
) -> Result<bool> {
    match options {
        Some(Value::Map(map)) => match map.get("revoke_sessions") {
            Some(Value::Bool(value)) => Ok(*value),
            Some(other) => Err(IntentError::type_error(format!(
                "[auth] {}() revoke_sessions must be a bool, got {}",
                function_name,
                other.type_name()
            ))),
            None => Ok(false),
        },
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() options must be a map, got {}",
            function_name,
            other.type_name()
        ))),
        None => Ok(false),
    }
}

fn parse_totp_options(
    function_name: &str,
    options: Option<&Value>,
) -> Result<(String, Option<String>, Option<String>)> {
    match options {
        Some(Value::Map(map)) => {
            let identifier_kind = optional_string_option(map, "identifier_kind", function_name)?
                .unwrap_or_else(|| "email".to_string());
            let issuer = optional_string_option(map, "issuer", function_name)?;
            let label = optional_string_option(map, "label", function_name)?;
            Ok((identifier_kind, issuer, label))
        }
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() options must be a map, got {}",
            function_name,
            other.type_name()
        ))),
        None => Ok(("email".to_string(), None, None)),
    }
}

fn optional_string_option(
    map: &HashMap<String, Value>,
    key: &str,
    function_name: &str,
) -> Result<Option<String>> {
    match map.get(key) {
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(other) => Err(IntentError::type_error(format!(
            "[auth] {}() {} must be a string, got {}",
            function_name,
            key,
            other.type_name()
        ))),
        None => Ok(None),
    }
}

fn requested_group_ids(value: &Value) -> Result<Vec<String>> {
    match value {
        Value::String(group_id) => Ok(vec![group_id.clone()]),
        Value::Array(items) => items
            .iter()
            .map(|item| match item {
                Value::String(group_id) => Ok(group_id.clone()),
                other => Err(IntentError::type_error(format!(
                    "[auth] has_group() group IDs must be strings, got {}",
                    other.type_name()
                ))),
            })
            .collect(),
        other => Err(IntentError::type_error(format!(
            "[auth] has_group() expects a group id string or array of strings, got {}",
            other.type_name()
        ))),
    }
}

fn is_http_request_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Map(map)
            if matches!(map.get("method"), Some(Value::String(_)))
                && matches!(map.get("path"), Some(Value::String(_)))
    )
}

fn is_auth_session_value(value: &Value) -> bool {
    let Value::Map(map) = value else {
        return false;
    };
    let has_session_id = matches!(map.get("id"), Some(Value::String(id)) if !id.trim().is_empty());
    let has_user = matches!(map.get("user"), Some(Value::Map(user)) if matches!(user.get("id"), Some(Value::String(id)) if !id.trim().is_empty()));
    let has_data = matches!(map.get("data"), Some(Value::Map(_)));
    has_session_id && has_user && has_data
}

fn session_value_for_auth_subject(value: &Value) -> Result<Option<Value>> {
    if is_http_request_value(value) {
        validate_http_request_arg("has_group", value)?;
        let Some(session_id) = get_session_id_from_request(value) else {
            return Ok(None);
        };
        return Ok(get_session_by_id(&session_id).map(|session| session_to_value(&session)));
    }

    match value {
        Value::Map(_) if is_auth_session_value(value) => Ok(Some(value.clone())),
        Value::Map(_) => Err(IntentError::type_error(
            "[auth] has_group() expects a request map or auth session map with id, user.id, and data",
        )),
        other => Err(IntentError::type_error(format!(
            "[auth] has_group() expects a request or session map, got {}",
            other.type_name()
        ))),
    }
}

fn session_value_has_group(session_value: &Value, group_ids: &[String]) -> bool {
    let Value::Map(session) = session_value else {
        return false;
    };
    let Some(Value::Map(data)) = session.get("data") else {
        return false;
    };
    value_map_has_group(data, group_ids)
}

fn value_map_has_group(map: &HashMap<String, Value>, group_ids: &[String]) -> bool {
    let direct_match = match map.get("group_ids") {
        Some(Value::Array(values)) => values.iter().any(|value| match value {
            Value::String(candidate) => group_ids.iter().any(|expected| expected == candidate),
            _ => false,
        }),
        _ => false,
    };
    if direct_match {
        return true;
    }

    match map.get("claims") {
        Some(Value::Map(claims)) => value_map_has_group(claims, group_ids),
        _ => false,
    }
}

fn prepare_request_aware_manual_session(req: &Value, mut session: Session) -> Session {
    session.device_name = request_device_name(req);
    session.user_agent_hash = request_user_agent_hash(req);
    session.last_ip_hash = request_ip_hash(req);
    session
}

fn persist_manual_session_record(req: &Value, mut session: Session) -> Result<()> {
    if let Some(existing_session_id) = get_session_id_from_request(req) {
        if let Some(existing_session) = get_session_by_id(&existing_session_id) {
            if existing_session_id != session.id {
                if existing_session.user_id == session.user_id
                    && session.data_json == "{}"
                    && existing_session.data_json != "{}"
                {
                    session.data_json = existing_session.data_json.clone();
                }
                migrate_session(&existing_session_id, &session)
                    .map_err(IntentError::runtime_error)?;
            } else {
                store_session(session);
            }
        } else {
            store_session(session);
        }
    } else {
        store_session(session);
    }

    Ok(())
}

fn persist_request_aware_manual_session(
    response: &Value,
    req: &Value,
    session: Session,
    options: Option<&HashMap<String, Value>>,
    config: &AuthConfig,
) -> Result<Value> {
    let session = prepare_request_aware_manual_session(req, session);
    let cookie = build_signed_session_cookie(config, &session.id, options)
        .map_err(IntentError::type_error)?;
    let response = add_set_cookie_header(response, &cookie).map_err(IntentError::type_error)?;
    persist_manual_session_record(req, session)?;
    Ok(response)
}

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

    // @ntnt auth_challenge_csrf_token
    // @module std/auth
    // @signature auth_challenge_csrf_token(req: Request, kind?: String) -> Option<String>
    // Get the CSRF token bound to the current staged auth challenge.
    //
    // Use this only for pre-session staged forms such as first-login password
    // rotation or password -> TOTP verification. Signed-in forms should use
    // `csrf_field(req)` and `verify_csrf(req, token)` instead.
    // @param req The HTTP request object
    // @param kind Optional challenge kind to require before returning a token
    // @returns Option containing the challenge CSRF token, or None when no matching challenge is active
    // @see_also auth_challenge_csrf_field, verify_auth_challenge_csrf, begin_auth_challenge
    // @since v0.4.9
    // @tags #auth, #security, #csrf
    // @example auth_challenge_csrf_token(req, "local.totp") ~ "Read staged challenge CSRF token"
    module.insert(
        "auth_challenge_csrf_token".to_string(),
        Value::NativeFunction {
            name: "auth_challenge_csrf_token".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] auth_challenge_csrf_token() requires request and optional kind"
                            .to_string(),
                    ));
                }
                let kind = optional_kind_arg("auth_challenge_csrf_token", args, 1)?;
                let Some(challenge) = active_auth_challenge_for_request(
                    "auth_challenge_csrf_token",
                    &args[0],
                    kind.as_deref(),
                )?
                else {
                    return Ok(make_none());
                };
                match auth_challenge_csrf_token(&challenge) {
                    Some(token) => Ok(make_some(Value::String(token))),
                    None => Ok(make_none()),
                }
            },
        },
    );

    // @ntnt auth_challenge_csrf_field
    // @module std/auth
    // @signature auth_challenge_csrf_field(req: Request, kind?: String) -> String
    // Get an HTML hidden input field for the current staged auth challenge CSRF token.
    //
    // `begin_auth_challenge()` creates a server-side challenge CSRF nonce automatically.
    // Render this helper in pre-session staged forms, then verify the submitted value
    // with `verify_auth_challenge_csrf(req, form["_csrf"], kind)` before mutating
    // credentials, MFA state, or sessions.
    // @param req The HTTP request object
    // @param kind Optional challenge kind to require before rendering a field
    // @returns HTML string like `<input type="hidden" name="_csrf" value="..."/>`, or an empty string when no matching challenge is active
    // @see_also verify_auth_challenge_csrf, auth_challenge_csrf_token, csrf_field
    // @since v0.4.9
    // @tags #auth, #security, #csrf
    // @example auth_challenge_csrf_field(req, "local.password_change") ~ "Render hidden CSRF input for staged password change"
    module.insert(
        "auth_challenge_csrf_field".to_string(),
        Value::NativeFunction {
            name: "auth_challenge_csrf_field".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] auth_challenge_csrf_field() requires request and optional kind"
                            .to_string(),
                    ));
                }
                let kind = optional_kind_arg("auth_challenge_csrf_field", args, 1)?;
                let Some(challenge) = active_auth_challenge_for_request(
                    "auth_challenge_csrf_field",
                    &args[0],
                    kind.as_deref(),
                )?
                else {
                    return Ok(Value::String(String::new()));
                };
                let Some(token) = auth_challenge_csrf_token(&challenge) else {
                    return Ok(Value::String(String::new()));
                };
                Ok(Value::String(format!(
                    r#"<input type="hidden" name="_csrf" value="{}"/>"#,
                    token
                )))
            },
        },
    );

    // @ntnt verify_auth_challenge_csrf
    // @module std/auth
    // @signature verify_auth_challenge_csrf(req: Request, token: String, kind?: String) -> Bool
    // Verify a submitted CSRF token against the current staged auth challenge.
    //
    // This is for pre-session challenge forms. It validates against the active
    // auth-challenge cookie rather than the authenticated-session cookie used by
    // `verify_csrf()`.
    // @param req The HTTP request object
    // @param token The submitted `_csrf` form value
    // @param kind Optional challenge kind to require before accepting the token
    // @returns true when the token matches the active staged challenge, false otherwise
    // @see_also auth_challenge_csrf_field, auth_challenge_csrf_token, verify_csrf
    // @since v0.4.9
    // @tags #auth, #security, #csrf
    // @example verify_auth_challenge_csrf(req, form["_csrf"], "local.totp") ~ "Validate staged challenge form"
    module.insert(
        "verify_auth_challenge_csrf".to_string(),
        Value::NativeFunction {
            name: "verify_auth_challenge_csrf".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] verify_auth_challenge_csrf() requires request, token, and optional kind"
                            .to_string(),
                    ));
                }
                let submitted_token = string_arg("verify_auth_challenge_csrf", args, 1, "token")?;
                let kind = optional_kind_arg("verify_auth_challenge_csrf", args, 2)?;
                let Some(challenge) = active_auth_challenge_for_request(
                    "verify_auth_challenge_csrf",
                    &args[0],
                    kind.as_deref(),
                )? else {
                    return Ok(Value::Bool(false));
                };
                let Some(expected_token) = auth_challenge_csrf_token(&challenge) else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(constant_time_compare(
                    &expected_token,
                    submitted_token,
                )))
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
    // @example complete_auth_challenge(redirect("/admin"), req, map { "claims": app_claims_for_user(user) }) ~ "Upgrade staged auth into a session"
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

                validate_http_request_arg("complete_auth_challenge", &args[1])?;

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

                let effective_session_ttl = session_ttl
                    .min(config.max_session_ttl.unwrap_or(session_ttl));
                let session = create_manual_session(&merged_session, effective_session_ttl)
                    .map_err(IntentError::type_error)?;
                let session = prepare_request_aware_manual_session(&args[1], session);
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

                persist_manual_session_record(&args[1], session)?;
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
    // device_name (when captured), created_at, expires_at, and is_current
    // (boolean indicating if it's the current session). Sensitive raw hashes are
    // never exposed. Useful for "manage your sessions" UI.
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
                                    if let Some(device_name) = &si.device_name {
                                        map.insert(
                                            "device_name".to_string(),
                                            Value::String(device_name.clone()),
                                        );
                                    }
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
    // @signature enable_auth(providers: [Provider], preset_or_options?: String | Map, overrides?: Map) -> Unit
    // Initialize the authentication system with OAuth providers.
    //
    // Stores provider configurations for use by auth handlers. After calling this,
    // you can use auth_start, auth_callback, and auth_logout
    // with routes to enable OAuth login.
    //
    // Session storage options: "memory" (default), "sqlite:./path.db", "postgres://url", or "redis://url".
    // Built-in presets: "consumer", "admin", "internal", "strict".
    // Supported option keys: session_secret, session_ttl, refresh_ttl, sliding_sessions,
    // refresh_throttle, max_session_ttl, success_url/after_login, failure_url/after_failure,
    // logout_url/after_logout, protected_paths, route_prefix, login_page, login_page_title,
    // login_page_logo_url, login_page_heading, login_page_copy, cookie_name, cookie_secure,
    // cookie_same_site, session_store, store_tokens, health_endpoint.
    // When a preset string is used, the overrides map is applied on top of the preset.
    // @param providers Array of provider configs created by oauth() or oauth_discover()
    // @param preset_or_options Optional preset string or options map
    // @param overrides Optional overrides map applied on top of a preset
    // @returns Unit
    // @see_also oauth, oauth_discover, auth_start
    // @since v0.3.11
    // @tags #auth, #oauth
    // @example ~ "Initialize auth with GitHub and explicit options"
    //   let github = oauth("github", get_env("GITHUB_ID"), get_env("GITHUB_SECRET"))
    //   enable_auth([github], map { "session_secret": "my-secret" })
    // @example enable_auth([github], "admin") ~ "Admin preset"
    // @example enable_auth([github], "consumer", map { "session_store": "sqlite:./sessions.db" }) ~ "Preset plus overrides"
    // @example enable_auth([github], map { "session_store": "redis://localhost:6379" }) ~ "Redis sessions"
    module.insert(
        "enable_auth".to_string(),
        Value::NativeFunction {
            name: "enable_auth".to_string(),
            arity: 0, // Variadic: 1-2 args (providers, options?)
            max_arity: 0,
            requires: Some(RuntimeCapability::HttpConfig),
            func: |args| {
                if args.is_empty() || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] enable_auth() requires 1 to 3 arguments (providers, optional preset/options, optional overrides)"
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

                let mut preset_name: Option<String> = None;
                let mut options: Option<HashMap<String, Value>> = None;

                match args.get(1) {
                    Some(Value::Map(m)) => options = Some(m.clone()),
                    Some(Value::String(s)) => preset_name = Some(s.clone()),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() second argument must be a preset string or options map, got {}",
                            other.type_name()
                        )))
                    }
                    None => {}
                }

                if let Some(arg3) = args.get(2) {
                    match arg3 {
                        Value::Map(m) => {
                            if options.is_some() {
                                return Err(IntentError::type_error(
                                    "[auth] enable_auth() accepts at most one options/overrides map"
                                        .to_string(),
                                ));
                            }
                            options = Some(m.clone());
                        }
                        other => {
                            return Err(IntentError::type_error(format!(
                                "[auth] enable_auth() third argument must be an overrides map, got {}",
                                other.type_name()
                            )))
                        }
                    }
                }

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
                                | "sliding_sessions"
                                | "refresh_throttle"
                                | "max_session_ttl"
                                | "success_url"
                                | "after_login"
                                | "failure_url"
                                | "after_failure"
                                | "logout_url"
                                | "after_logout"
                                | "route_prefix"
                                | "login_page"
                                | "login_page_title"
                                | "login_page_logo_url"
                                | "login_page_heading"
                                | "login_page_copy"
                                | "cookie_name"
                                | "cookie_secure"
                                | "cookie_same_site"
                                | "session_store"
                                | "store_tokens"
                                | "health_endpoint"
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

                let mut base_config = match preset_name.as_ref() {
                    Some(name) => auth_preset_config(name).map_err(IntentError::type_error)?,
                    None => AuthConfig::default(),
                };

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
                    None => base_config.session_secret.clone(),
                };

                let session_ttl = match get_option(&["session_ttl"]) {
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"session_ttl\" must be an int, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.session_ttl,
                };

                let success_url = match get_option(&["success_url", "after_login"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"success_url\"/\"after_login\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.success_url.clone(),
                };

                let failure_url = match get_option(&["failure_url", "after_failure"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"failure_url\"/\"after_failure\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.failure_url.clone(),
                };

                let logout_url = match get_option(&["logout_url", "after_logout"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"logout_url\"/\"after_logout\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.logout_url.clone(),
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
                    None => base_config.protected_paths.clone(),
                };

                let route_prefix = match get_option(&["route_prefix"]) {
                    Some(Value::String(s)) => routes::normalize_auth_route_prefix_option(s)
                        .map_err(IntentError::type_error)?,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"route_prefix\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.route_prefix.clone(),
                };

                let login_page_enabled = match get_option(&["login_page"]) {
                    Some(Value::Bool(b)) => *b,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"login_page\" must be a bool, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.login_page_enabled,
                };

                let login_page_title = match get_option(&["login_page_title"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"login_page_title\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.login_page_title.clone(),
                };

                let login_page_logo_url = match get_option(&["login_page_logo_url"]) {
                    Some(Value::String(s)) => {
                        let trimmed = s.trim();
                        if trimmed.is_empty() {
                            None
                        } else {
                            Some(trimmed.to_string())
                        }
                    }
                    Some(Value::EnumValue {
                        enum_name,
                        variant,
                        ..
                    }) if enum_name == "Option" && variant == "None" => None,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"login_page_logo_url\" must be a string or None, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.login_page_logo_url.clone(),
                };

                let login_page_heading = match get_option(&["login_page_heading"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"login_page_heading\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.login_page_heading.clone(),
                };

                let login_page_copy = match get_option(&["login_page_copy"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"login_page_copy\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.login_page_copy.clone(),
                };

                let cookie_name = match get_option(&["cookie_name"]) {
                    Some(Value::String(s)) => s.clone(),
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"cookie_name\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.cookie_name.clone(),
                };

                let cookie_secure = match get_option(&["cookie_secure"]) {
                    Some(Value::Bool(b)) => *b,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"cookie_secure\" must be a bool, got {}",
                            other.type_name()
                        )));
                    }
                    None => default_auth_cookie_secure_env()
                };

                let cookie_same_site = match get_option(&["cookie_same_site"]) {
                    Some(Value::String(s)) => normalize_cookie_same_site(s)
                        .map_err(IntentError::type_error)?,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"cookie_same_site\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.cookie_same_site.clone(),
                };

                let session_store = match get_option(&["session_store"]) {
                    Some(Value::String(s)) => {
                        parse_auth_session_store(s).map_err(IntentError::type_error)?
                    }
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"session_store\" must be a string, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.session_store.clone(),
                };

                // Initialize database/cache if needed
                if let Err(e) = initialize_session_store(&session_store) {
                    eprintln!("[auth] Failed to initialize session store: {}", e);
                    return Err(IntentError::runtime_error(format!(
                        "Failed to initialize session store: {}",
                        e
                    )));
                }

                let health_endpoint = match get_option(&["health_endpoint"]) {
                    Some(Value::Bool(b)) => *b,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"health_endpoint\" must be a bool, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.health_endpoint,
                };

                let store_tokens = match get_option(&["store_tokens"]) {
                    Some(Value::Bool(b)) => *b,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"store_tokens\" must be a bool, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.store_tokens,
                };

                let refresh_ttl = match get_option(&["refresh_ttl"]) {
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"refresh_ttl\" must be an int, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.refresh_ttl,
                };

                let sliding_sessions = match get_option(&["sliding_sessions"]) {
                    Some(Value::Bool(b)) => *b,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"sliding_sessions\" must be a bool, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.sliding_sessions,
                };

                let refresh_throttle = match get_option(&["refresh_throttle"]) {
                    Some(Value::Int(n)) => *n,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"refresh_throttle\" must be an int, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.refresh_throttle,
                };

                if refresh_throttle < 0 {
                    return Err(IntentError::type_error(
                        "[auth] enable_auth() option \"refresh_throttle\" must be >= 0"
                            .to_string(),
                    ));
                }

                let max_session_ttl = match get_option(&["max_session_ttl"]) {
                    Some(Value::Int(n)) => Some(*n),
                    Some(Value::EnumValue {
                        enum_name,
                        variant,
                        ..
                    }) if enum_name == "Option" && variant == "None" => None,
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] enable_auth() option \"max_session_ttl\" must be an int or None, got {}",
                            other.type_name()
                        )));
                    }
                    None => base_config.max_session_ttl,
                };

                if let Some(max_session_ttl) = max_session_ttl {
                    if max_session_ttl <= 0 {
                        return Err(IntentError::type_error(
                            "[auth] enable_auth() option \"max_session_ttl\" must be > 0"
                                .to_string(),
                        ));
                    }
                }

                base_config.providers = providers;
                base_config.success_url = success_url;
                base_config.failure_url = failure_url;
                base_config.logout_url = logout_url;
                base_config.protected_paths = protected_paths;
                base_config.route_prefix = route_prefix;
                base_config.login_page_enabled = login_page_enabled;
                base_config.login_page_title = login_page_title;
                base_config.login_page_logo_url = login_page_logo_url;
                base_config.login_page_heading = login_page_heading;
                base_config.login_page_copy = login_page_copy;
                base_config.cookie_name = cookie_name;
                base_config.cookie_secure = cookie_secure;
                base_config.cookie_same_site = cookie_same_site;
                base_config.session_ttl = session_ttl;
                base_config.refresh_ttl = refresh_ttl;
                base_config.sliding_sessions = sliding_sessions;
                base_config.refresh_throttle = refresh_throttle;
                base_config.max_session_ttl = max_session_ttl;
                base_config.store_tokens = store_tokens;
                base_config.health_endpoint = health_endpoint;
                base_config.session_secret = session_secret;
                base_config.session_store = session_store;
                let config = base_config;

                // Initialize auth
                init_auth(config.clone());

                eprintln!(
                    "[auth] Built-in auth route manifest: {}",
                    routes::auth_route_manifest(&config).join(", ")
                );
                for warning in routes::auth_route_collision_warnings(&config) {
                    eprintln!("[auth] Warning: {}", warning);
                }

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
                                Ok(Some(cookie)) => {
                                    let mut response = match redirect_response(
                                        &guards::request_target(&args[0]),
                                        Some(&cookie),
                                    ) {
                                        Value::Map(map) => map,
                                        other => return Ok(other),
                                    };
                                    response.insert("status".to_string(), Value::Int(307));
                                    Ok(Value::Map(response))
                                },
                                Ok(None) => Ok(Value::Unit),
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
                        Ok(Some(cookie)) => {
                            let mut response = match redirect_response(
                                &guards::request_target(&mw_args[0]),
                                Some(&cookie),
                            ) {
                                Value::Map(map) => map,
                                other => return Ok(other),
                            };
                            response.insert("status".to_string(), Value::Int(307));
                            Ok(Value::Map(response))
                        },
                        Ok(None) => Ok(Value::Unit),
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
                    false,
                    None,
                    None,
                    None,
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
                let session = match create_session(
                    &provider_name,
                    user_info,
                    tokens.as_ref(),
                    config.session_ttl.min(config.max_session_ttl.unwrap_or(config.session_ttl)),
                ) {
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

    // @ntnt local_user
    // @module std/auth
    // @signature local_user(identifier: String, options?: Map) -> Result<Map, String>
    // Load a safe local identity payload, including non-secret extension metadata.
    //
    // Use this from trusted server-side setup/admin code when an app needs the
    // local identity record and app-owned metadata without verifying a password.
    // Reserved `auth.*` metadata is kept server-side and omitted from the returned
    // payload; use dedicated std/auth helpers for stdlib-managed lifecycle state.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param options Optional map with `identifier_kind` (`"email"`, `"phone"`, `"username"`, or `"custom"`; default `"email"`)
    // @returns Ok(map) with safe local user fields and `metadata`; Err(message) when missing or unsupported
    // @see_also update_local_user_metadata, verify_local_password
    // @since v0.4.9
    // @tags #auth, #local-auth, #metadata
    // @example let user = local_user("admin@example.com")? ~ "Load a safe local user payload"
    module.insert(
        "local_user".to_string(),
        Value::NativeFunction {
            name: "local_user".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] local_user() requires identifier and optional options".to_string(),
                    ));
                }

                let _config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before local_user()."
                            .to_string(),
                    )
                })?;

                let identifier = match &args[0] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] local_user() identifier must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let identifier_kind = parse_local_identifier_options("local_user", args.get(1))?;

                match local_user_record(&identifier_kind, identifier) {
                    Ok(identity) => match local_identity_to_safe_value(&identity) {
                        Ok(value) => Ok(Value::ok(value)),
                        Err(message) => Ok(Value::err(Value::String(message))),
                    },
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt update_local_user_metadata
    // @module std/auth
    // @signature update_local_user_metadata(identifier: String, metadata: Map, options?: Map) -> Result<Map, String>
    // Merge or replace app-owned local identity metadata and return a safe payload.
    //
    // By default this helper performs a top-level merge into `metadata_json` and
    // preserves reserved `auth.*` namespaces for std/auth-managed lifecycle state.
    // Pass `map { "replace": true }` to replace app-visible metadata. Inputs may
    // not write `auth` or `auth.*` keys directly.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param metadata App-owned metadata map to merge or replace
    // @param options Optional map with `identifier_kind` and `replace`
    // @returns Ok(map) with safe local user fields and metadata; Err(message) on missing user, reserved namespace, or unsupported backend
    // @see_also local_user, verify_local_password
    // @since v0.4.9
    // @tags #auth, #local-auth, #metadata
    // @example update_local_user_metadata(user.email, map { "app": map { "group_ids": ["admins"] } })? ~ "Attach app authorization context"
    module.insert(
        "update_local_user_metadata".to_string(),
        Value::NativeFunction {
            name: "update_local_user_metadata".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] update_local_user_metadata() requires identifier, metadata, and optional options"
                            .to_string(),
                    ));
                }

                let _config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before update_local_user_metadata()."
                            .to_string(),
                    )
                })?;

                let identifier = match &args[0] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] update_local_user_metadata() identifier must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let metadata = match &args[1] {
                    Value::Map(map) => map,
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] update_local_user_metadata() metadata must be a map, got {}",
                            other.type_name()
                        )))
                    }
                };
                let (identifier_kind, replace) = parse_local_metadata_update_options(args.get(2))?;

                match update_local_user_metadata_record(
                    &identifier_kind,
                    identifier,
                    metadata,
                    replace,
                ) {
                    Ok(identity) => match local_identity_to_safe_value(&identity) {
                        Ok(value) => Ok(Value::ok(value)),
                        Err(message) => Ok(Value::err(Value::String(message))),
                    },
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt has_group
    // @module std/auth
    // @signature has_group(subject: Request | Session, group_ids: String | [String]) -> Bool
    // Check app-owned group IDs from authenticated session data.
    //
    // This is a thin authorization helper, not an RBAC system. Apps decide what
    // group IDs mean and attach them during `sign_in_session(...)` as
    // `data.group_ids` or `data.claims.group_ids`. The helper accepts either a
    // request with an auth cookie or a session map from `current_session(req)`.
    // @param subject Request or Session map
    // @param group_ids Required group ID or any-of list
    // @returns true if the active session contains any requested group ID
    // @see_also require_auth, current_session, sign_in_session
    // @since v0.4.9
    // @tags #auth, #authorization, #rbac
    // @example has_group(req, "admins") ~ "Check an API/page request for admin group membership"
    // @example has_group(current_session(req)?, ["admins", "owners"]) ~ "Check any accepted group"
    module.insert(
        "has_group".to_string(),
        Value::NativeFunction {
            name: "has_group".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "[auth] has_group() requires a request/session and group id(s)".to_string(),
                    ));
                }

                let group_ids = requested_group_ids(&args[1])?;
                let Some(session_value) = session_value_for_auth_subject(&args[0])? else {
                    return Ok(Value::Bool(false));
                };
                Ok(Value::Bool(session_value_has_group(
                    &session_value,
                    &group_ids,
                )))
            },
        },
    );

    // @ntnt bootstrap_local_user
    // @module std/auth
    // @signature bootstrap_local_user(identifier: String, password: String, options?: Map) -> Result<Map, String>
    // Provision an initial local credential record for app setup flows.
    //
    // The helper normalizes the identifier (email by default), rejects an existing
    // local identity with the same normalized identifier, stores an auth-owned
    // local identity plus password credential, and returns the same safe local
    // user payload shape as `verify_local_password(...)`. The bootstrapped
    // account starts in `bootstrap` state with `must_change_password: true` so
    // app-owned setup code can force rotation before granting regular access.
    // It never exposes passwords, password hashes, hash parameters, credentials,
    // secrets, or tokens.
    // @param identifier The local setup identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param password The temporary plaintext password to hash and store
    // @param options Optional map with `identifier_kind` (`"email"`, `"phone"`, `"username"`, or `"custom"`; default `"email"`)
    // @returns Ok(map) with safe local user fields; Err(message) on duplicate, invalid input, or storage backend failure
    // @error RuntimeError ~ "Auth not initialized" fix: "Call enable_auth(...) during app startup before bootstrapping local credentials"
    // @error TypeError ~ "identifier must be a string" fix: "Pass the setup email/identifier as a string"
    // @see_also verify_local_password, sign_in_session
    // @since v0.4.9
    // @tags #auth, #local-auth, #security
    // @example let user = bootstrap_local_user("admin@example.com", setup_password)? ~ "Provision a first setup user"
    // @example sign_in_session(redirect("/setup"), req, map { "subject_id": user["subject_id"], "email": user["email"] }) ~ "Sign in the setup user with request-aware session handling"
    module.insert(
        "bootstrap_local_user".to_string(),
        Value::NativeFunction {
            name: "bootstrap_local_user".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] bootstrap_local_user() requires identifier, password, and optional options"
                            .to_string(),
                    ));
                }

                let _config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before bootstrap_local_user()."
                            .to_string(),
                    )
                })?;

                let identifier = match &args[0] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] bootstrap_local_user() identifier must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let password = match &args[1] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] bootstrap_local_user() password must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let identifier_kind = match args.get(2) {
                    Some(Value::Map(map)) => match map.get("identifier_kind") {
                        Some(Value::String(kind)) => kind.as_str(),
                        Some(other) => {
                            return Err(IntentError::type_error(format!(
                                "[auth] bootstrap_local_user() identifier_kind must be a string, got {}",
                                other.type_name()
                            )))
                        }
                        None => "email",
                    },
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] bootstrap_local_user() options must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => "email",
                };

                match bootstrap_local_user_record(identifier_kind, identifier, password) {
                    Ok(verified) => Ok(Value::ok(verified_local_password_to_value(verified))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt set_local_password
    // @module std/auth
    // @signature set_local_password(identifier: String, current_password: String, new_password: String, options?: Map) -> Result<Map, String>
    // Verify the current local credential, rotate the user's password, and clear setup-required local account state.
    //
    // The helper normalizes the identifier (email by default), loads the auth-owned
    // local identity, writes a replacement password credential, transitions the
    // identity to `active`, clears `must_change_password`, and returns the same
    // safe local user payload shape as `verify_local_password(...)`. It requires
    // the current setup/forced-change password before rotation, then callers can
    // compose the resulting user through request-aware `sign_in_session(...)`. It
    // never exposes passwords, password hashes, hash parameters, credentials,
    // secrets, or tokens.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param current_password The current setup, forced-change, or active local password to verify before rotation
    // @param new_password The replacement plaintext password to hash and store; it must differ from the current password
    // @param options Optional map with `identifier_kind` (`"email"`, `"phone"`, `"username"`, or `"custom"`; default `"email"`)
    // @returns Ok(map) with safe local user fields; Err(message) on invalid credentials, invalid input, or storage backend failure
    // @error RuntimeError ~ "Auth not initialized" fix: "Call enable_auth(...) during app startup before rotating local credentials"
    // @error TypeError ~ "identifier must be a string" fix: "Pass the setup email/identifier as a string"
    // @see_also bootstrap_local_user, verify_local_password, sign_in_session
    // @since v0.4.9
    // @tags #auth, #local-auth, #security
    // @example let user = set_local_password(form["email"] ?? "", form["setup_password"] ?? "", form["new_password"] ?? "")? ~ "Complete local setup by verifying and rotating the bootstrap password"
    // @example sign_in_session(redirect("/admin"), req, map { "subject_id": user["subject_id"], "email": user["email"] }) ~ "Sign in after setup completion with request-aware session handling"
    module.insert(
        "set_local_password".to_string(),
        Value::NativeFunction {
            name: "set_local_password".to_string(),
            arity: 3,
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 3 || args.len() > 4 {
                    return Err(IntentError::type_error(
                        "[auth] set_local_password() requires identifier, current_password, new_password, and optional options"
                            .to_string(),
                    ));
                }

                let _config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before set_local_password()."
                            .to_string(),
                    )
                })?;

                let identifier = match &args[0] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] set_local_password() identifier must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let current_password = match &args[1] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] set_local_password() current_password must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let new_password = match &args[2] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] set_local_password() new_password must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let identifier_kind = match args.get(3) {
                    Some(Value::Map(map)) => match map.get("identifier_kind") {
                        Some(Value::String(kind)) => kind.as_str(),
                        Some(other) => {
                            return Err(IntentError::type_error(format!(
                                "[auth] set_local_password() identifier_kind must be a string, got {}",
                                other.type_name()
                            )))
                        }
                        None => "email",
                    },
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] set_local_password() options must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => "email",
                };

                match set_local_password_record(
                    identifier_kind,
                    identifier,
                    current_password,
                    new_password,
                ) {
                    Ok(verified) => Ok(Value::ok(verified_local_password_to_value(verified))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt issue_password_reset
    // @module std/auth
    // @signature issue_password_reset(identifier: String, options?: Map) -> Result<Map, String>
    // Issue a one-time password reset token for a local identity.
    //
    // The helper normalizes the identifier (email by default), stores only a hashed
    // verifier with an opaque selector, and returns syntactically valid token material
    // for valid-shaped requests so response shape does not reveal account existence.
    // Only resettable local identities have the selector/verifier persisted; dummy
    // token material for missing, disabled, or locked identities later fails with the
    // same generic consume error. Malformed identifiers or non-positive TTLs return
    // a generic accepted payload without token material. Store or send the returned
    // `token` out-of-band; std/auth never stores the raw token.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param options Optional map with `identifier_kind` (`"email"`, `"phone"`, `"username"`, or `"custom"`; default `"email"`) and `ttl_seconds` (default 3600)
    // @returns Ok(map) with `status: "accepted"`; syntactically valid reset requests also include `token`, `selector`, `created_at`, and `expires_at` without revealing whether a matching account exists
    // @see_also consume_password_reset, verify_local_password, set_local_password
    // @since v0.4.9
    // @tags #auth, #local-auth, #password-reset, #security
    // @example let reset = issue_password_reset(form["email"] ?? "")? ~ "Begin password reset without account enumeration"
    module.insert(
        "issue_password_reset".to_string(),
        Value::NativeFunction {
            name: "issue_password_reset".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] issue_password_reset() requires identifier and optional options"
                            .to_string(),
                    ));
                }
                require_auth_initialized_for("issue_password_reset")?;
                let identifier = string_arg("issue_password_reset", args, 0, "identifier")?;
                let (identifier_kind, ttl_seconds) =
                    parse_password_reset_issue_options("issue_password_reset", args.get(1))?;
                match issue_password_reset_record(&identifier_kind, identifier, ttl_seconds) {
                    Ok(payload) => Ok(Value::ok(Value::Map(payload))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt consume_password_reset
    // @module std/auth
    // @signature consume_password_reset(token: String, new_password: String, options?: Map) -> Result<Map, String>
    // Consume a one-time password reset token and rotate the local password.
    //
    // Valid tokens are consumed atomically, verified against the stored hash, and
    // then used to replace the local credential, transition the identity to `active`,
    // and clear `must_change_password`. Missing, malformed, expired, replayed, and
    // wrong-verifier tokens all return the same generic error. Returned payloads are
    // safe local auth user maps and never expose password hashes, token hashes, raw
    // token material, credentials, or secrets. Pass `map { "revoke_sessions": true }`
    // to explicitly revoke that local user's existing sessions after a successful reset;
    // by default, existing sessions are left active.
    // @param token The `selector.verifier` token returned by `issue_password_reset(...)`
    // @param new_password Replacement plaintext password to hash and store
    // @param options Optional map with `revoke_sessions` (default false)
    // @returns Ok(map) with safe local user fields and `revoked_sessions`; Err(message) for invalid/expired/replayed tokens or storage failure
    // @see_also issue_password_reset, verify_local_password, logout_all, sign_in_session
    // @since v0.4.9
    // @tags #auth, #local-auth, #password-reset, #security
    // @example let user = consume_password_reset(form["token"] ?? "", form["new_password"] ?? "")? ~ "Finish a password reset without revoking sessions"
    // @example let user = consume_password_reset(form["token"] ?? "", form["new_password"] ?? "", map { "revoke_sessions": form["logout_all"] == "on" })? ~ "Finish a reset and explicitly revoke existing sessions from a checkbox"
    module.insert(
        "consume_password_reset".to_string(),
        Value::NativeFunction {
            name: "consume_password_reset".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] consume_password_reset() requires token, new_password, and optional options"
                            .to_string(),
                    ));
                }
                require_auth_initialized_for("consume_password_reset")?;
                let token = string_arg("consume_password_reset", args, 0, "token")?;
                let new_password = string_arg("consume_password_reset", args, 1, "new_password")?;
                let revoke_sessions =
                    parse_password_reset_consume_options("consume_password_reset", args.get(2))?;
                match consume_password_reset_record(token, new_password, revoke_sessions) {
                    Ok((verified, revoked_sessions)) => {
                        let mut value = verified_local_password_to_value(verified);
                        if let Value::Map(ref mut map) = value {
                            map.insert(
                                "revoked_sessions".to_string(),
                                Value::Int(revoked_sessions as i64),
                            );
                        }
                        Ok(Value::ok(value))
                    }
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt verify_local_password
    // @module std/auth
    // @signature verify_local_password(identifier: String, password: String, options?: Map) -> Result<Map, String>
    // Verify a local credential record and return a safe auth user payload.
    //
    // The helper normalizes the identifier (email by default), loads the auth-owned
    // local identity and credential secret, verifies the password hash, and returns
    // a map suitable for app-specific session claim derivation and
    // `sign_in_session(...)`. It never exposes password hashes or hash parameters.
    // Missing identities, missing credential secrets, bad passwords, disabled accounts,
    // and locked accounts all return the same invalid-credentials error to avoid
    // account-state or identity enumeration. Corrupted or unsupported stored hashes
    // return a generic operational auth error without backend parser details after
    // running the same dummy verification work used for absent credentials. Bootstrap,
    // pending-setup, and password-change-required identities can verify credentials,
    // but the returned payload forces `must_change_password: true` so callers do not
    // accidentally treat setup-required accounts as fully active sessions.
    // @param identifier The local login identifier. Email is the default identifier kind.
    // @param password The plaintext password from the login form
    // @param options Optional map with `identifier_kind` (`"email"`, `"phone"`, `"username"`, or `"custom"`; default `"email"`)
    // @returns Ok(map) with `subject_id`, `provider`, identifier fields, account `state`, and password-change metadata; Err(message) on invalid credentials or operational credential errors
    // @error RuntimeError ~ "Auth not initialized" fix: "Call enable_auth(...) during app startup before verifying local credentials"
    // @error TypeError ~ "identifier must be a string" fix: "Pass the submitted email/identifier as a string"
    // @see_also sign_in_session, current_session
    // @since v0.4.9
    // @tags #auth, #local-auth, #security
    // @example let verified = verify_local_password(form["email"] ?? "", form["password"] ?? "")? ~ "Verify email/password credentials"
    // @example sign_in_session(redirect("/admin"), req, map { "subject_id": verified["subject_id"], "email": verified["email"] }) ~ "Complete request-aware local sign-in after verification"
    module.insert(
        "verify_local_password".to_string(),
        Value::NativeFunction {
            name: "verify_local_password".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] verify_local_password() requires identifier, password, and optional options"
                            .to_string(),
                    ));
                }

                let _config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before verify_local_password()."
                            .to_string(),
                    )
                })?;

                let identifier = match &args[0] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] verify_local_password() identifier must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let password = match &args[1] {
                    Value::String(s) => s.as_str(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] verify_local_password() password must be a string, got {}",
                            other.type_name()
                        )))
                    }
                };
                let identifier_kind = match args.get(2) {
                    Some(Value::Map(map)) => match map.get("identifier_kind") {
                        Some(Value::String(kind)) => kind.as_str(),
                        Some(other) => {
                            return Err(IntentError::type_error(format!(
                                "[auth] verify_local_password() identifier_kind must be a string, got {}",
                                other.type_name()
                            )))
                        }
                        None => "email",
                    },
                    Some(other) => {
                        return Err(IntentError::type_error(format!(
                            "[auth] verify_local_password() options must be a map, got {}",
                            other.type_name()
                        )))
                    }
                    None => "email",
                };

                match verify_local_password_record(identifier_kind, identifier, password) {
                    Ok(verified) => Ok(Value::ok(verified_local_password_to_value(verified))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt begin_totp_enrollment
    // @module std/auth
    // @signature begin_totp_enrollment(identifier: String, options?: Map) -> Result<Map, String>
    // Start TOTP enrollment for a local identity and return one-time setup material.
    //
    // Generates a new TOTP secret, stores it under std/auth-owned local identity metadata (`auth.totp.pending_secret`),
    // and returns safe status fields plus an `otpauth://` URI for QR-code setup. The raw secret is not returned as a
    // standalone field and is never exposed through `local_user(...)`, `current_user(...)`, or `totp_status(...)`.
    // The setup URI itself is secret-bearing; render it only in the setup response and do not log, cache, or persist it.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param options Optional map with `identifier_kind`, `issuer`, and `label`
    // @returns Ok(map) with pending TOTP status and setup `uri`; Err(message) on invalid identity/state/storage
    // @see_also confirm_totp_enrollment, totp_status, verify_local_totp, reset_totp
    // @since v0.4.9
    // @tags #auth, #local-auth, #mfa, #totp
    // @example begin_totp_enrollment("admin@example.com", map { "issuer": "Admin" })? ~ "Create TOTP setup URI"
    module.insert(
        "begin_totp_enrollment".to_string(),
        Value::NativeFunction {
            name: "begin_totp_enrollment".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] begin_totp_enrollment() requires identifier and optional options"
                            .to_string(),
                    ));
                }
                require_auth_initialized_for("begin_totp_enrollment")?;
                let identifier = string_arg("begin_totp_enrollment", args, 0, "identifier")?;
                let (identifier_kind, issuer, label) =
                    parse_totp_options("begin_totp_enrollment", args.get(1))?;
                match begin_totp_enrollment_record(
                    &identifier_kind,
                    identifier,
                    issuer.as_deref(),
                    label.as_deref(),
                ) {
                    Ok(status) => Ok(Value::ok(Value::Map(status))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt confirm_totp_enrollment
    // @module std/auth
    // @signature confirm_totp_enrollment(identifier: String, code: String, options?: Map) -> Result<Map, String>
    // Confirm a pending local TOTP enrollment using a code from the authenticator app.
    //
    // Moves `auth.totp.pending_secret` to std/auth-owned confirmed secret metadata only after the submitted code
    // verifies. Returned status is secret-free and safe to use in setup flows.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param code The 6-digit TOTP code from the authenticator app
    // @param options Optional map with `identifier_kind`
    // @returns Ok(map) with confirmed TOTP status; Err(message) on missing pending setup or invalid code
    // @see_also begin_totp_enrollment, verify_local_totp, totp_status, reset_totp
    // @since v0.4.9
    // @tags #auth, #local-auth, #mfa, #totp
    // @example confirm_totp_enrollment("admin@example.com", form["code"] ?? "")? ~ "Finish TOTP setup"
    module.insert(
        "confirm_totp_enrollment".to_string(),
        Value::NativeFunction {
            name: "confirm_totp_enrollment".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] confirm_totp_enrollment() requires identifier, code, and optional options"
                            .to_string(),
                    ));
                }
                require_auth_initialized_for("confirm_totp_enrollment")?;
                let identifier = string_arg("confirm_totp_enrollment", args, 0, "identifier")?;
                let code = string_arg("confirm_totp_enrollment", args, 1, "code")?;
                let (identifier_kind, _, _) =
                    parse_totp_options("confirm_totp_enrollment", args.get(2))?;
                match confirm_totp_enrollment_record(&identifier_kind, identifier, code) {
                    Ok(status) => Ok(Value::ok(Value::Map(status))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt verify_local_totp
    // @module std/auth
    // @signature verify_local_totp(identifier: String, code: String, options?: Map) -> Result<Map, String>
    // Verify a local user's confirmed TOTP code without exposing the stored secret.
    //
    // Use after `verify_local_password(...)` in staged login flows. Apps should keep the user in an auth challenge
    // until this helper succeeds, then complete the challenge with `complete_auth_challenge(...)` or sign in through
    // `sign_in_session(...)`. Pair TOTP endpoints with app rate limiting/backoff; this helper verifies codes but does
    // not own account lockout policy.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param code The 6-digit TOTP code from the authenticator app
    // @param options Optional map with `identifier_kind`
    // @returns Ok(map) with `verified: true` and safe TOTP status; Err(message) on invalid code or unavailable TOTP
    // @see_also begin_auth_challenge, complete_auth_challenge, totp_status
    // @since v0.4.9
    // @tags #auth, #local-auth, #mfa, #totp
    // @example verify_local_totp("admin@example.com", form["code"] ?? "")? ~ "Verify second factor"
    module.insert(
        "verify_local_totp".to_string(),
        Value::NativeFunction {
            name: "verify_local_totp".to_string(),
            arity: 2,
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() < 2 || args.len() > 3 {
                    return Err(IntentError::type_error(
                        "[auth] verify_local_totp() requires identifier, code, and optional options"
                            .to_string(),
                    ));
                }
                require_auth_initialized_for("verify_local_totp")?;
                let identifier = string_arg("verify_local_totp", args, 0, "identifier")?;
                let code = string_arg("verify_local_totp", args, 1, "code")?;
                let (identifier_kind, _, _) = parse_totp_options("verify_local_totp", args.get(2))?;
                match verify_local_totp_record(&identifier_kind, identifier, code) {
                    Ok(status) => Ok(Value::ok(Value::Map(status))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt totp_status
    // @module std/auth
    // @signature totp_status(identifier: String, options?: Map) -> Result<Map, String>
    // Read a local user's safe TOTP enrollment status.
    //
    // Returns status booleans and display metadata only. It never includes pending or confirmed TOTP secret material.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param options Optional map with `identifier_kind`
    // @returns Ok(map) with safe TOTP status; Err(message) on invalid identity/storage
    // @see_also begin_totp_enrollment, confirm_totp_enrollment, verify_local_totp, reset_totp
    // @since v0.4.9
    // @tags #auth, #local-auth, #mfa, #totp
    // @example totp_status("admin@example.com")? ~ "Check whether TOTP is enabled"
    module.insert(
        "totp_status".to_string(),
        Value::NativeFunction {
            name: "totp_status".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] totp_status() requires identifier and optional options".to_string(),
                    ));
                }
                require_auth_initialized_for("totp_status")?;
                let identifier = string_arg("totp_status", args, 0, "identifier")?;
                let (identifier_kind, _, _) = parse_totp_options("totp_status", args.get(1))?;
                match totp_status_record(&identifier_kind, identifier) {
                    Ok(status) => Ok(Value::ok(Value::Map(status))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt reset_totp
    // @module std/auth
    // @signature reset_totp(identifier: String, options?: Map) -> Result<Map, String>
    // Clear a local user's pending or confirmed TOTP enrollment.
    //
    // Removes only std/auth-owned `auth.totp` metadata and preserves app-owned metadata namespaces.
    // @param identifier The local user identifier. Supported kinds are `email` (default), `phone`, `username`, and `custom`.
    // @param options Optional map with `identifier_kind`
    // @returns Ok(map) with disabled TOTP status; Err(message) on invalid identity/state/storage
    // @see_also begin_totp_enrollment, confirm_totp_enrollment, totp_status
    // @since v0.4.9
    // @tags #auth, #local-auth, #mfa, #totp
    // @example reset_totp("admin@example.com")? ~ "Remove TOTP enrollment"
    module.insert(
        "reset_totp".to_string(),
        Value::NativeFunction {
            name: "reset_totp".to_string(),
            arity: 1,
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "[auth] reset_totp() requires identifier and optional options".to_string(),
                    ));
                }
                require_auth_initialized_for("reset_totp")?;
                let identifier = string_arg("reset_totp", args, 0, "identifier")?;
                let (identifier_kind, _, _) = parse_totp_options("reset_totp", args.get(1))?;
                match reset_totp_record(&identifier_kind, identifier) {
                    Ok(status) => Ok(Value::ok(Value::Map(status))),
                    Err(message) => Ok(Value::err(Value::String(message))),
                }
            },
        },
    );

    // @ntnt sign_in_session
    // @module std/auth
    // @signature sign_in_session(response: Response, req: Request, session: Map, options?: Map) -> Response
    // Persist a request-aware session and attach the auth cookie to an existing response.
    //
    // Use this after password, magic-link, or other non-OAuth login flows. The
    // request argument lets `std/auth` rotate/migrate any existing session and
    // capture the same device/IP/user-agent metadata used by OAuth callbacks.
    // If an existing session for the same user is rotated and the new session
    // has no explicit `claims`/`data`, its session data is preserved. Cross-user
    // sign-in always starts with only the provided session data.
    // Migration note for 0.4.9 pre-release callers: the old
    // `sign_in_session(response, session, options?)` shape is intentionally not
    // supported; pass the current request as the second argument so metadata and
    // session rotation are not silently skipped.
    // The session map must include `subject_id`, and may optionally include
    // `provider`, `email`, `name`, `picture`, `claims`, `data`, or `raw`.
    // @param response The Response map to attach the session cookie to
    // @param req The current HTTP request
    // @param session Session data map, including required `subject_id`
    // @param options Optional map with `session_ttl` and cookie override keys (`cookie_path`, `cookie_same_site`, `cookie_secure`, `cookie_http_only`, `cookie_max_age`)
    // @returns Response with a persisted session and Set-Cookie header
    // @error TypeError ~ "request must be an HTTP request map" fix: "Call sign_in_session(response, req, session, options?) from a route handler and pass the current req"
    // @see_also sign_out_session, current_session, rotate_session
    // @since v0.4.9
    // @tags #auth, #session
    // @example sign_in_session(redirect("/admin"), req, map { "subject_id": user.id, "claims": app_claims_for_user(user) }) ~ "Sign in and redirect"
    module.insert(
        "sign_in_session".to_string(),
        Value::NativeFunction {
            name: "sign_in_session".to_string(),
            arity: 3,
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 3 || args.len() > 4 {
                    return Err(IntentError::type_error(
                        "[auth] sign_in_session() requires response, request, session, and optional options"
                            .to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::runtime_error(
                        "[auth] Auth not initialized. Call enable_auth() before sign_in_session()."
                            .to_string(),
                    )
                })?;

                validate_http_request_arg("sign_in_session", &args[1])?;

                let session_spec = match &args[2] {
                    Value::Map(map) => map.clone(),
                    other => {
                        return Err(IntentError::type_error(format!(
                            "[auth] sign_in_session() session must be a map, got {}",
                            other.type_name()
                        )))
                    }
                };

                let options = match args.get(3) {
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

                let effective_session_ttl =
                    session_ttl.min(config.max_session_ttl.unwrap_or(session_ttl));
                let session = create_manual_session(&session_spec, effective_session_ttl)
                    .map_err(IntentError::type_error)?;

                persist_request_aware_manual_session(
                    &args[0],
                    &args[1],
                    session,
                    options.as_ref(),
                    &config,
                )
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

    // @ntnt auth_health
    // @module std/auth
    // @signature auth_health(req: Request) -> Response
    // Return auth diagnostics for the built-in `/auth/health` route.
    //
    // The response includes safe config state, provider posture, cookie/session settings,
    // and warnings for common auth misconfigurations without leaking secrets.
    // In production, this route is disabled unless `health_endpoint: true` is set.
    // @param req The current HTTP request object
    // @returns JSON response with auth diagnostics
    // @see_also enable_auth, auth_start, auth_callback
    // @since v0.4.9
    // @tags #auth, #observability
    // @example get("/auth/health", auth_health) ~ "Wire up auth diagnostics"
    module.insert(
        "auth_health".to_string(),
        Value::NativeFunction {
            name: "auth_health".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| handle_auth_health(args),
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

                Ok(Value::Bool(verify_totp_code(&secret, &code, "", "NTNT")))
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::storage::{
        cleanup_expired_oauth_state_records, consume_auth_challenge_record,
        consume_exchange_token_record,
        consume_local_password_reset_token_and_store_credential_record, consume_oauth_state_record,
        delete_all_session_records_for_user, delete_session_record, extend_session_record_expiry,
        get_auth_challenge_record, get_local_credential_secret_record,
        get_local_identity_by_identifier_record, get_refreshable_session_record,
        get_session_record, list_session_records_for_user, migrate_session_record,
        store_auth_challenge_record, store_exchange_token_record,
        store_local_identity_and_credential_record, store_local_password_reset_token_record,
        store_oauth_state_record, store_session_record, update_session_record_data,
        update_session_record_tokens, LocalAccountState, LocalCredentialSecret, LocalIdentity,
        LocalPasswordResetToken, OAUTH_STATE_TTL,
    };
    use super::*;

    static AUTH_TEST_MUTEX: std::sync::LazyLock<std::sync::Mutex<()>> =
        std::sync::LazyLock::new(|| std::sync::Mutex::new(()));

    fn reset_auth_test_state() {
        let mut store = SESSION_STORE.lock().unwrap();
        store.sessions.clear();
        store.oauth_states.clear();
        store.exchange_tokens.clear();
        store.auth_challenges.clear();
        store.local_auth = storage::LocalAuthMemoryStore::default();
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

    fn result_err_string(value: Value) -> String {
        let Value::EnumValue {
            enum_name,
            variant,
            values,
        } = value
        else {
            panic!("expected Result::Err, got {value:?}");
        };
        assert_eq!(enum_name, "Result");
        assert_eq!(variant, "Err");
        match values.first() {
            Some(Value::String(message)) => message.clone(),
            other => panic!("unexpected Result::Err payload: {other:?}"),
        }
    }

    fn result_ok_map(value: Value) -> HashMap<String, Value> {
        let Value::EnumValue {
            enum_name,
            variant,
            values,
        } = value
        else {
            panic!("expected Result::Ok, got {value:?}");
        };
        assert_eq!(enum_name, "Result");
        assert_eq!(variant, "Ok");
        match values.first() {
            Some(Value::Map(map)) => map.clone(),
            other => panic!("unexpected Result::Ok payload: {other:?}"),
        }
    }

    fn assert_map_bool(map: &HashMap<String, Value>, key: &str, expected: bool) {
        match map.get(key) {
            Some(Value::Bool(value)) => assert_eq!(*value, expected, "unexpected {key}"),
            other => panic!("expected {key} bool, got {other:?}"),
        }
    }

    fn map_int(map: &HashMap<String, Value>, key: &str) -> i64 {
        match map.get(key) {
            Some(Value::Int(value)) => *value,
            other => panic!("expected {key} int, got {other:?}"),
        }
    }

    fn map_string(map: &HashMap<String, Value>, key: &str) -> String {
        match map.get(key) {
            Some(Value::String(value)) => value.clone(),
            other => panic!("expected {key} string, got {other:?}"),
        }
    }

    fn assert_no_totp_secret_material(value: &Value, secret: &str) {
        match value {
            Value::String(s) => assert!(
                !s.contains(secret),
                "string value unexpectedly exposed TOTP secret material"
            ),
            Value::Array(values) => {
                for item in values {
                    assert_no_totp_secret_material(item, secret);
                }
            }
            Value::Map(map) => {
                for (key, item) in map {
                    assert!(
                        key != "secret" && key != "pending_secret",
                        "safe payload unexpectedly exposed TOTP secret key {key}"
                    );
                    assert_no_totp_secret_material(item, secret);
                }
            }
            Value::EnumValue { values, .. } => {
                for item in values {
                    assert_no_totp_secret_material(item, secret);
                }
            }
            _ => {}
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
        Value::Map(HashMap::from([
            ("method".to_string(), Value::String("GET".to_string())),
            ("path".to_string(), Value::String("/admin".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "cookie".to_string(),
                    Value::String(cookies.join("; ")),
                )])),
            ),
        ]))
    }

    fn request_with_cookie_and_security_headers(cookie: &str) -> Value {
        Value::Map(HashMap::from([
            ("method".to_string(), Value::String("GET".to_string())),
            ("path".to_string(), Value::String("/admin".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([
                    ("cookie".to_string(), Value::String(cookie.to_string())),
                    (
                        "user-agent".to_string(),
                        Value::String("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/17.0 Safari/605.1.15".to_string()),
                    ),
                    (
                        "x-forwarded-for".to_string(),
                        Value::String("203.0.113.10".to_string()),
                    ),
                ])),
            ),
            ("ip".to_string(), Value::String("198.51.100.20".to_string())),
        ]))
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

    fn auth_test_postgres_store() -> Option<SessionStore> {
        std::env::var("NTNT_AUTH_TEST_POSTGRES_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .map(SessionStore::Postgres)
    }

    fn auth_test_redis_store() -> Option<SessionStore> {
        std::env::var("NTNT_AUTH_TEST_REDIS_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .map(SessionStore::Redis)
    }

    fn run_auth_storage_contract_round_trip(store_kind: SessionStore, label: &str) {
        reset_auth_test_state();
        init_test_auth(store_kind.clone());

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: format!("session-{}-1", label),
            user_id: format!("user-{}", label),
            provider: "local".to_string(),
            email: Some(format!("{}@example.com", label)),
            name: Some(format!("{} User", label)),
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: format!("csrf-{}-1", label),
            access_token: Some(format!("access-{}-1", label)),
            refresh_token: Some(format!("refresh-{}-1", label)),
            token_expires_at: Some(now + 60),
            device_name: Some("SQLite Mac".to_string()),
            user_agent_hash: Some("ua-sqlite-1".to_string()),
            last_ip_hash: Some("ip-sqlite-1".to_string()),
            created_at: now,
            expires_at: now + 300,
        };
        store_session_record(&session)
            .unwrap_or_else(|e| panic!("{} session store should succeed: {}", label, e));
        let stored_session = get_session_record(&session.id)
            .unwrap_or_else(|e| panic!("{} session lookup should succeed: {}", label, e))
            .unwrap_or_else(|| panic!("{} session should exist", label));
        assert_eq!(stored_session.id, session.id);
        assert_eq!(stored_session.device_name.as_deref(), Some("SQLite Mac"));
        assert_eq!(
            stored_session.user_agent_hash.as_deref(),
            Some("ua-sqlite-1")
        );
        assert_eq!(stored_session.last_ip_hash.as_deref(), Some("ip-sqlite-1"));
        update_session_record_data(&session.id, r#"{"role":"admin"}"#)
            .unwrap_or_else(|e| panic!("{} session data update should succeed: {}", label, e));
        update_session_record_tokens(
            &session.id,
            &TokenResponse {
                access_token: format!("access-{}-2", label),
                token_type: "Bearer".to_string(),
                expires_in: Some(120),
                refresh_token: Some(format!("refresh-{}-2", label)),
                id_token: None,
                scope: None,
            },
            now,
        )
        .unwrap_or_else(|e| panic!("{} session token update should succeed: {}", label, e));
        extend_session_record_expiry(&session.id, now + 600)
            .unwrap_or_else(|e| panic!("{} session expiry extension should succeed: {}", label, e));
        let updated_session = get_session_record(&session.id)
            .unwrap_or_else(|e| panic!("{} updated session lookup should succeed: {}", label, e))
            .unwrap_or_else(|| panic!("{} updated session should exist", label));
        assert_eq!(updated_session.data_json, r#"{"role":"admin"}"#);
        assert_eq!(
            updated_session.access_token.as_deref(),
            Some(format!("access-{}-2", label).as_str())
        );
        assert_eq!(
            updated_session.refresh_token.as_deref(),
            Some(format!("refresh-{}-2", label).as_str())
        );
        assert_eq!(updated_session.token_expires_at, Some(now + 120));
        assert_eq!(updated_session.expires_at, now + 600);
        assert_eq!(updated_session.device_name.as_deref(), Some("SQLite Mac"));
        assert_eq!(
            updated_session.user_agent_hash.as_deref(),
            Some("ua-sqlite-1")
        );
        assert_eq!(updated_session.last_ip_hash.as_deref(), Some("ip-sqlite-1"));

        let refreshable_session = Session {
            id: format!("session-{}-refreshable", label),
            user_id: format!("user-{}", label),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: format!("csrf-{}-refreshable", label),
            access_token: Some(format!("access-{}-refreshable", label)),
            refresh_token: Some(format!("refresh-{}-refreshable", label)),
            token_expires_at: Some(now - 30),
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 30,
            expires_at: now - 5,
        };
        store_session_record(&refreshable_session).unwrap_or_else(|e| {
            panic!("{} refreshable session store should succeed: {}", label, e)
        });
        let refreshable = get_refreshable_session_record(&refreshable_session.id, 3600)
            .unwrap_or_else(|e| panic!("{} refreshable lookup should succeed: {}", label, e));
        match store_kind {
            SessionStore::Memory => {
                assert!(
                    refreshable.is_none(),
                    "{} memory store should not surface expired sessions as refreshable",
                    label
                );
            }
            _ => {
                assert_eq!(
                    refreshable
                        .unwrap_or_else(|| panic!("{} refreshable session should exist", label))
                        .id,
                    refreshable_session.id
                );
            }
        }

        let rotated_session = Session {
            id: format!("session-{}-rotated", label),
            csrf_token: format!("csrf-{}-rotated", label),
            ..updated_session.clone()
        };
        migrate_session_record(&session.id, &rotated_session)
            .unwrap_or_else(|e| panic!("{} session migration should succeed: {}", label, e));
        assert!(get_session_record(&session.id)
            .unwrap_or_else(|e| panic!("{} old session lookup should succeed: {}", label, e))
            .is_none());
        let rotated_lookup = get_session_record(&rotated_session.id)
            .unwrap_or_else(|e| panic!("{} rotated session lookup should succeed: {}", label, e))
            .unwrap_or_else(|| panic!("{} rotated session should exist", label));
        assert_eq!(rotated_lookup.csrf_token, format!("csrf-{}-rotated", label));
        assert_eq!(rotated_lookup.device_name.as_deref(), Some("SQLite Mac"));
        assert_eq!(
            rotated_lookup.user_agent_hash.as_deref(),
            Some("ua-sqlite-1")
        );
        assert_eq!(rotated_lookup.last_ip_hash.as_deref(), Some("ip-sqlite-1"));
        let listed_sessions = list_session_records_for_user(
            &format!("user-{}", label),
            Some(&rotated_session.id),
            now,
        )
        .unwrap_or_else(|e| panic!("{} session listing should succeed: {}", label, e));
        assert_eq!(listed_sessions.len(), 1);
        let listed_current = listed_sessions
            .iter()
            .find(|session| session.id == rotated_session.id && session.is_current)
            .unwrap_or_else(|| panic!("{} current session should be listed", label));
        assert_eq!(listed_current.device_name.as_deref(), Some("SQLite Mac"));
        assert_eq!(
            delete_all_session_records_for_user(
                &format!("user-{}", label),
                Some(&rotated_session.id),
            )
            .unwrap_or_else(|e| panic!("{} delete-all sessions should succeed: {}", label, e)),
            1
        );
        delete_session_record(&rotated_session.id)
            .unwrap_or_else(|e| panic!("{} session delete should succeed: {}", label, e));
        assert!(get_session_record(&rotated_session.id)
            .unwrap_or_else(|e| panic!("{} deleted session lookup should succeed: {}", label, e))
            .is_none());

        let local_identity = LocalIdentity {
            id: format!("local-user-{}", label),
            identifier_kind: "username".to_string(),
            identifier: format!("{}User", label),
            identifier_normalized: format!("{}user", label),
            created_at: now,
            updated_at: now,
            state: LocalAccountState::Active,
            metadata_json: r#"{"app":{"group_ids":["admins"]}}"#.to_string(),
        };
        let local_credential = LocalCredentialSecret {
            local_user_id: local_identity.id.clone(),
            password_hash: format!("hash-{}", label),
            password_hash_algorithm: "bcrypt".to_string(),
            password_hash_params_json: "{}".to_string(),
            password_changed_at: now,
            must_change_password: false,
        };
        store_local_identity_and_credential_record(&local_identity, &local_credential)
            .unwrap_or_else(|e| {
                panic!(
                    "{} local identity+credential store should succeed: {}",
                    label, e
                )
            });
        let stored_local_identity =
            get_local_identity_by_identifier_record("username", &format!("{}user", label))
                .unwrap_or_else(|e| panic!("{} local identity lookup should succeed: {}", label, e))
                .unwrap_or_else(|| panic!("{} local identity should exist", label));
        assert_eq!(stored_local_identity.id, local_identity.id);
        assert_eq!(
            stored_local_identity.metadata_json,
            local_identity.metadata_json
        );
        let stored_local_credential = get_local_credential_secret_record(&local_identity.id)
            .unwrap_or_else(|e| panic!("{} local credential lookup should succeed: {}", label, e))
            .unwrap_or_else(|| panic!("{} local credential should exist", label));
        assert_eq!(
            stored_local_credential.password_hash,
            local_credential.password_hash
        );
        let reset_token = LocalPasswordResetToken {
            selector: format!("reset-selector-{}", label),
            local_user_id: local_identity.id.clone(),
            token_hash: format!("reset-hash-{}", label),
            created_at: now,
            expires_at: now + 3600,
        };
        store_local_password_reset_token_record(&reset_token).unwrap_or_else(|e| {
            panic!(
                "{} local password reset token store should succeed: {}",
                label, e
            )
        });
        let consumed_reset = consume_local_password_reset_token_and_store_credential_record(
            &reset_token.selector,
            &reset_token.token_hash,
            now,
            |local_user_id| {
                Ok(LocalCredentialSecret {
                    local_user_id: local_user_id.to_string(),
                    password_hash: format!("hash-{}-rotated", label),
                    password_hash_algorithm: "bcrypt".to_string(),
                    password_hash_params_json: "{}".to_string(),
                    password_changed_at: now + 1,
                    must_change_password: false,
                })
            },
        )
        .unwrap_or_else(|e| {
            panic!(
                "{} local password reset consume should succeed: {}",
                label, e
            )
        })
        .unwrap_or_else(|| panic!("{} local password reset token should consume", label));
        assert_eq!(consumed_reset.0.id, local_identity.id);
        assert_eq!(
            consumed_reset.1.password_hash,
            format!("hash-{}-rotated", label)
        );
        assert!(
            consume_local_password_reset_token_and_store_credential_record(
                &reset_token.selector,
                &reset_token.token_hash,
                now,
                |local_user_id| {
                    Ok(LocalCredentialSecret {
                        local_user_id: local_user_id.to_string(),
                        password_hash: "unused".to_string(),
                        password_hash_algorithm: "bcrypt".to_string(),
                        password_hash_params_json: "{}".to_string(),
                        password_changed_at: now + 2,
                        must_change_password: false,
                    })
                },
            )
            .unwrap_or_else(|e| panic!(
                "{} local password reset replay lookup should succeed: {}",
                label, e
            ))
            .is_none()
        );

        let oauth_state = OAuthState {
            state: format!("oauth-{}-active", label),
            nonce: Some(format!("nonce-{}", label)),
            pkce_verifier: Some(format!("pkce-{}", label)),
            provider: "github".to_string(),
            redirect_url: "/auth/callback".to_string(),
            remember_me: true,
            device_name: Some(format!("{} Browser", label)),
            user_agent_hash: Some(format!("ua-{}-oauth", label)),
            last_ip_hash: Some(format!("ip-{}-oauth", label)),
            created_at: now,
        };
        store_oauth_state_record(&oauth_state)
            .unwrap_or_else(|e| panic!("{} oauth state store should succeed: {}", label, e));
        let consumed_oauth_state = consume_oauth_state_record(&oauth_state.state)
            .unwrap_or_else(|e| panic!("{} oauth state consume should succeed: {}", label, e))
            .unwrap_or_else(|| panic!("{} oauth state should exist", label));
        assert_eq!(consumed_oauth_state.state, oauth_state.state);
        assert!(consumed_oauth_state.remember_me);
        assert_eq!(
            consumed_oauth_state.device_name.as_deref(),
            Some(format!("{} Browser", label).as_str())
        );
        assert_eq!(
            consumed_oauth_state.user_agent_hash.as_deref(),
            Some(format!("ua-{}-oauth", label).as_str())
        );
        assert_eq!(
            consumed_oauth_state.last_ip_hash.as_deref(),
            Some(format!("ip-{}-oauth", label).as_str())
        );
        assert!(
            consume_oauth_state_record(&oauth_state.state)
                .unwrap_or_else(|e| panic!(
                    "{} oauth state second consume should succeed: {}",
                    label, e
                ))
                .is_none(),
            "{} oauth state should be one-time use",
            label
        );
        let expired_oauth_state = OAuthState {
            state: format!("oauth-{}-expired", label),
            nonce: None,
            pkce_verifier: None,
            provider: "github".to_string(),
            redirect_url: "/auth/callback".to_string(),
            remember_me: false,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - OAUTH_STATE_TTL - 100,
        };
        store_oauth_state_record(&expired_oauth_state).unwrap_or_else(|e| {
            panic!("{} expired oauth state store should succeed: {}", label, e)
        });
        let cleaned_oauth_states = cleanup_expired_oauth_state_records(now - OAUTH_STATE_TTL)
            .unwrap_or_else(|e| panic!("{} oauth cleanup should succeed: {}", label, e));
        match store_kind {
            SessionStore::Redis(_) => {
                assert_eq!(
                    cleaned_oauth_states, 0,
                    "{} redis oauth cleanup should only scrub memory fallback entries",
                    label
                );
                assert!(
                    consume_oauth_state_record(&expired_oauth_state.state)
                        .unwrap_or_else(|e| panic!(
                            "{} expired oauth state consume should succeed: {}",
                            label, e
                        ))
                        .is_some(),
                    "{} redis oauth state should remain consumable until Redis TTL expires",
                    label
                );
            }
            _ => {
                assert!(
                    cleaned_oauth_states >= 1,
                    "{} oauth cleanup should remove at least the expired state",
                    label
                );
                assert!(
                    consume_oauth_state_record(&expired_oauth_state.state)
                        .unwrap_or_else(|e| panic!(
                            "{} expired oauth state consume after cleanup should succeed: {}",
                            label, e
                        ))
                        .is_none(),
                    "{} expired oauth state should not be consumable after cleanup",
                    label
                );
            }
        }

        store_exchange_token_record(&format!("exchange-{}-active", label), &session.id)
            .unwrap_or_else(|e| panic!("{} exchange token store should succeed: {}", label, e));
        let active_exchange_token = format!("exchange-{}-active", label);
        assert_eq!(
            consume_exchange_token_record(&active_exchange_token)
                .unwrap_or_else(|e| panic!(
                    "{} exchange token consume should succeed: {}",
                    label, e
                ))
                .as_deref(),
            Some(session.id.as_str())
        );
        assert!(
            consume_exchange_token_record(&active_exchange_token)
                .unwrap_or_else(|e| panic!(
                    "{} exchange token second consume should succeed: {}",
                    label, e
                ))
                .is_none(),
            "{} exchange token should be one-time use",
            label
        );

        let challenge = AuthChallenge {
            id: format!("challenge-{}-active", label),
            subject_id: format!("user-{}", label),
            provider: "local".to_string(),
            kind: "mfa_pending".to_string(),
            csrf_token: "test-csrf".to_string(),
            data_json: "{}".to_string(),
            created_at: now,
            expires_at: now + 60,
        };
        store_auth_challenge_record(&challenge)
            .unwrap_or_else(|e| panic!("{} auth challenge store should succeed: {}", label, e));
        assert_eq!(
            get_auth_challenge_record(&challenge.id)
                .unwrap_or_else(|e| panic!("{} auth challenge lookup should succeed: {}", label, e))
                .unwrap_or_else(|| panic!("{} auth challenge should exist", label))
                .id,
            challenge.id
        );
        assert_eq!(
            consume_auth_challenge_record(&challenge.id)
                .unwrap_or_else(|e| panic!(
                    "{} auth challenge consume should succeed: {}",
                    label, e
                ))
                .unwrap_or_else(|| panic!("{} auth challenge should exist", label))
                .id,
            challenge.id
        );
        assert!(
            consume_auth_challenge_record(&challenge.id)
                .unwrap_or_else(|e| panic!(
                    "{} auth challenge second consume should succeed: {}",
                    label, e
                ))
                .is_none(),
            "{} auth challenge should be one-time use",
            label
        );
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
                assert!(
                    matches!(challenge_map.get("user_id"), Some(Value::String(user_id)) if user_id == "local:user-123")
                );
                match challenge_map.get("user") {
                    Some(Value::Map(user)) => {
                        assert!(
                            matches!(user.get("id"), Some(Value::String(id)) if id == "local:user-123")
                        );
                        assert!(
                            matches!(user.get("provider"), Some(Value::String(provider)) if provider == "local")
                        );
                    }
                    other => panic!("expected challenge user map, got {:?}", other),
                }
                match challenge_map.get("data") {
                    Some(Value::Map(data)) => {
                        assert!(
                            matches!(data.get("next"), Some(Value::String(next)) if next == "/admin")
                        );
                        assert!(
                            !data.contains_key(AUTH_CHALLENGE_CSRF_DATA_KEY),
                            "current_auth_challenge() must not expose the private challenge csrf nonce"
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
    fn test_current_auth_challenge_hides_legacy_data_embedded_csrf_token() {
        let now = chrono::Utc::now().timestamp();
        let challenge = AuthChallenge {
            id: "challenge-legacy-csrf".to_string(),
            subject_id: "user-123".to_string(),
            provider: "local".to_string(),
            kind: "mfa_pending".to_string(),
            csrf_token: String::new(),
            data_json: value_map_to_json_string(&HashMap::from([
                (
                    AUTH_CHALLENGE_CSRF_DATA_KEY.to_string(),
                    Value::String("legacy-secret".to_string()),
                ),
                ("next".to_string(), Value::String("/admin".to_string())),
            ])),
            created_at: now,
            expires_at: now + 60,
        };

        let Value::Map(challenge_map) = auth_challenge_to_value(&challenge) else {
            panic!("expected challenge map");
        };
        let Some(Value::Map(data)) = challenge_map.get("data") else {
            panic!("expected data map");
        };
        assert!(
            !data.contains_key(AUTH_CHALLENGE_CSRF_DATA_KEY),
            "legacy data-embedded csrf tokens must stay private"
        );
        assert!(matches!(data.get("next"), Some(Value::String(next)) if next == "/admin"));
    }

    #[test]
    fn test_begin_auth_challenge_preserves_app_data_named_like_internal_csrf_key() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let current_auth_challenge = module_fn(&module, "current_auth_challenge");
        let csrf_token = module_fn(&module, "auth_challenge_csrf_token");
        let verify_csrf = module_fn(&module, "verify_auth_challenge_csrf");

        let app_value = "app-owned-value".to_string();
        let started = begin_auth_challenge(&[
            redirect_response("/local/totp", None),
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("kind".to_string(), Value::String("local.totp".to_string())),
                (
                    "data".to_string(),
                    Value::Map(HashMap::from([(
                        AUTH_CHALLENGE_CSRF_DATA_KEY.to_string(),
                        Value::String(app_value.clone()),
                    )])),
                ),
            ])),
        ])
        .unwrap();
        let req = request_with_cookie(&cookie_header_from_response(&started));

        let challenge = current_auth_challenge(&[req.clone()]).unwrap();
        let Value::EnumValue {
            variant, values, ..
        } = challenge
        else {
            panic!("expected Some(challenge)");
        };
        assert_eq!(variant, "Some");
        let Some(Value::Map(challenge_map)) = values.first() else {
            panic!("expected challenge map, got {:?}", values.first());
        };
        let Some(Value::Map(data)) = challenge_map.get("data") else {
            panic!("expected challenge data map");
        };
        assert!(
            matches!(data.get(AUTH_CHALLENGE_CSRF_DATA_KEY), Some(Value::String(value)) if value == &app_value)
        );

        let token_value =
            csrf_token(&[req.clone(), Value::String("local.totp".to_string())]).unwrap();
        let Value::EnumValue {
            variant,
            mut values,
            ..
        } = token_value
        else {
            panic!("expected Some(token)");
        };
        assert_eq!(variant, "Some");
        let Value::String(generated_token) = values.remove(0) else {
            panic!("expected generated challenge csrf token");
        };
        assert_ne!(generated_token, app_value);
        assert!(matches!(
            verify_csrf(&[
                req.clone(),
                Value::String(generated_token),
                Value::String("local.totp".to_string()),
            ])
            .unwrap(),
            Value::Bool(true)
        ));
        assert!(matches!(
            verify_csrf(&[
                req,
                Value::String(app_value),
                Value::String("local.totp".to_string()),
            ])
            .unwrap(),
            Value::Bool(false)
        ));
    }

    #[test]
    fn test_auth_challenge_csrf_helpers_validate_active_challenge_token() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let csrf_token = module_fn(&module, "auth_challenge_csrf_token");
        let csrf_field = module_fn(&module, "auth_challenge_csrf_field");
        let verify_csrf = module_fn(&module, "verify_auth_challenge_csrf");

        let started = begin_auth_challenge(&[
            redirect_response("/local/totp", None),
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("user-123".to_string()),
                ),
                ("kind".to_string(), Value::String("local.totp".to_string())),
            ])),
        ])
        .unwrap();
        let cookie = cookie_header_from_response(&started);
        let req = request_with_cookie(&cookie);

        let token_value =
            csrf_token(&[req.clone(), Value::String("local.totp".to_string())]).unwrap();
        let token = match token_value {
            Value::EnumValue {
                variant,
                mut values,
                ..
            } => {
                assert_eq!(variant, "Some");
                match values.remove(0) {
                    Value::String(token) => token,
                    other => panic!("expected challenge csrf token string, got {:?}", other),
                }
            }
            other => panic!("expected Some(token), got {:?}", other),
        };
        assert!(!token.is_empty());

        let field = csrf_field(&[req.clone(), Value::String("local.totp".to_string())]).unwrap();
        match field {
            Value::String(field) => {
                assert!(field.contains(r#"name="_csrf""#));
                assert!(field.contains(&token));
            }
            other => panic!("expected challenge csrf field string, got {:?}", other),
        }

        assert!(matches!(
            verify_csrf(&[
                req.clone(),
                Value::String(token.clone()),
                Value::String("local.totp".to_string()),
            ])
            .unwrap(),
            Value::Bool(true)
        ));
        assert!(matches!(
            verify_csrf(&[
                req.clone(),
                Value::String("wrong".to_string()),
                Value::String("local.totp".to_string()),
            ])
            .unwrap(),
            Value::Bool(false)
        ));
        assert!(matches!(
            verify_csrf(&[
                req,
                Value::String(token),
                Value::String("local.password_change".to_string()),
            ])
            .unwrap(),
            Value::Bool(false)
        ));
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
    fn test_session_backend_error_uses_memory_fallback() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-memory-fallback-active".to_string(),
            user_id: "user-memory-fallback".to_string(),
            provider: "local".to_string(),
            email: Some("fallback@example.com".to_string()),
            name: Some("Fallback User".to_string()),
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-memory-fallback".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now,
            expires_at: now + 300,
        };
        SESSION_STORE.lock().unwrap().set_session(session.clone());
        *SQLITE_CONN.lock().unwrap() = None;

        let fetched =
            get_session_by_id(&session.id).expect("memory fallback session should be returned");
        assert_eq!(fetched.id, session.id);
        assert_eq!(fetched.user_id, session.user_id);
    }

    #[test]
    fn test_refreshable_session_lookup_does_not_fallback_to_memory() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE.lock().unwrap().set_session(Session {
            id: "session-memory-refresh-fallback".to_string(),
            user_id: "user-memory-refresh".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-memory-refresh".to_string(),
            access_token: Some("access-memory-refresh".to_string()),
            refresh_token: Some("refresh-memory-refresh".to_string()),
            token_expires_at: Some(now - 30),
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 30,
            expires_at: now - 5,
        });
        *SQLITE_CONN.lock().unwrap() = None;

        assert!(get_session_by_id("session-memory-refresh-fallback").is_none());
    }

    #[test]
    fn test_oauth_state_backend_error_uses_ttl_checked_memory_fallback() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE.lock().unwrap().set_oauth_state(OAuthState {
            state: "oauth-memory-fallback-active".to_string(),
            nonce: None,
            pkce_verifier: None,
            provider: "github".to_string(),
            redirect_url: "/auth/callback".to_string(),
            remember_me: false,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now,
        });
        *SQLITE_CONN.lock().unwrap() = None;

        let consumed = consume_oauth_state("oauth-memory-fallback-active")
            .expect("memory fallback oauth state should be returned");
        assert_eq!(consumed.state, "oauth-memory-fallback-active");
        assert!(consume_oauth_state("oauth-memory-fallback-active").is_none());

        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));
        SESSION_STORE.lock().unwrap().set_oauth_state(OAuthState {
            state: "oauth-memory-fallback-expired".to_string(),
            nonce: None,
            pkce_verifier: None,
            provider: "github".to_string(),
            redirect_url: "/auth/callback".to_string(),
            remember_me: false,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 700,
        });
        *SQLITE_CONN.lock().unwrap() = None;

        assert!(consume_oauth_state("oauth-memory-fallback-expired").is_none());
    }

    #[test]
    fn test_exchange_token_backend_error_uses_ttl_checked_memory_fallback() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE.lock().unwrap().set_exchange_token(
            "exchange-memory-fallback-active".to_string(),
            "session-fallback-active".to_string(),
        );
        *SQLITE_CONN.lock().unwrap() = None;

        assert_eq!(
            consume_exchange_token("exchange-memory-fallback-active").as_deref(),
            Some("session-fallback-active")
        );
        assert!(consume_exchange_token("exchange-memory-fallback-active").is_none());

        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));
        SESSION_STORE.lock().unwrap().exchange_tokens.insert(
            "exchange-memory-fallback-expired".to_string(),
            (
                "session-fallback-expired".to_string(),
                now - EXCHANGE_TOKEN_TTL - 1,
            ),
        );
        *SQLITE_CONN.lock().unwrap() = None;

        assert!(consume_exchange_token("exchange-memory-fallback-expired").is_none());
    }

    #[test]
    fn test_oauth_and_exchange_cleanup_also_scrub_memory_fallback_for_non_memory_backends() {
        use super::storage::{
            cleanup_expired_exchange_token_records, cleanup_expired_oauth_state_records,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        let cutoff = now - 600;
        let mut store = SESSION_STORE.lock().unwrap();
        store.set_oauth_state(OAuthState {
            state: "oauth-memory-cleanup".to_string(),
            nonce: None,
            pkce_verifier: None,
            provider: "github".to_string(),
            redirect_url: "/auth/callback".to_string(),
            remember_me: false,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 700,
        });
        store.exchange_tokens.insert(
            "exchange-memory-cleanup".to_string(),
            ("session-cleanup".to_string(), now - EXCHANGE_TOKEN_TTL - 1),
        );
        drop(store);

        assert_eq!(
            cleanup_expired_oauth_state_records(cutoff)
                .expect("oauth state cleanup should succeed"),
            1
        );
        assert_eq!(
            cleanup_expired_exchange_token_records(now)
                .expect("exchange token cleanup should succeed"),
            1
        );
        assert!(SESSION_STORE
            .lock()
            .unwrap()
            .get_oauth_state("oauth-memory-cleanup")
            .is_none());
        assert!(SESSION_STORE
            .lock()
            .unwrap()
            .get_exchange_token("exchange-memory-cleanup")
            .is_none());
    }

    #[test]
    fn test_auth_storage_contract_memory_round_trip_all_record_types() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        run_auth_storage_contract_round_trip(SessionStore::Memory, "memory");
    }

    #[test]
    fn test_local_auth_durable_record_families_fail_closed_by_policy() {
        use super::storage::{local_auth_record_fallback_policy, LocalAuthRecordKind};

        for record_kind in [
            LocalAuthRecordKind::Identity,
            LocalAuthRecordKind::CredentialSecret,
            LocalAuthRecordKind::TotpEnrollment,
            LocalAuthRecordKind::PasswordResetToken,
            LocalAuthRecordKind::BootstrapState,
        ] {
            let policy = local_auth_record_fallback_policy(record_kind);
            assert!(
                policy.store_failure_fails_closed,
                "{record_kind:?} store failures must fail closed"
            );
            assert!(
                policy.lookup_failure_fails_closed,
                "{record_kind:?} lookup failures must fail closed"
            );
            assert!(
                policy.update_failure_fails_closed,
                "{record_kind:?} update/consume failures must fail closed"
            );
            assert!(
                !policy.production_memory_fallback_allowed,
                "{record_kind:?} must not allow production memory fallback"
            );
        }
    }

    #[test]
    fn test_local_identity_and_credential_store_round_trip_memory_and_sqlite() {
        use super::storage::{
            get_local_credential_secret_record, get_local_identity_by_id_record,
            get_local_identity_by_identifier_record, store_local_credential_secret_record,
            store_local_identity_record, LocalAccountState, LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();

        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let identity = LocalIdentity {
                id: "local-user-1".to_string(),
                identifier_kind: "email".to_string(),
                identifier: "Alice@Example.COM".to_string(),
                identifier_normalized: "alice@example.com".to_string(),
                created_at: 100,
                updated_at: 100,
                state: LocalAccountState::Active,
                metadata_json: "{}".to_string(),
            };
            let credential = LocalCredentialSecret {
                local_user_id: identity.id.clone(),
                password_hash: "argon2$hash".to_string(),
                password_hash_algorithm: "argon2id".to_string(),
                password_hash_params_json: "{}".to_string(),
                password_changed_at: 101,
                must_change_password: false,
            };

            store_local_identity_record(&identity).unwrap();
            store_local_credential_secret_record(&credential).unwrap();

            assert_eq!(
                get_local_identity_by_identifier_record("email", "alice@example.com").unwrap(),
                Some(identity.clone())
            );
            assert_eq!(
                get_local_identity_by_id_record("local-user-1").unwrap(),
                Some(identity.clone())
            );
            assert_eq!(
                get_local_credential_secret_record("local-user-1").unwrap(),
                Some(credential)
            );
        }
    }

    #[test]
    fn test_local_identity_and_credential_atomic_store_rejects_invalid_credential_without_orphaning_memory_and_sqlite(
    ) {
        use super::storage::{
            get_local_credential_secret_record, get_local_identity_by_identifier_record,
            store_local_identity_and_credential_record, LocalAccountState, LocalCredentialSecret,
            LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();

        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let identity = LocalIdentity {
                id: "local-user-atomic-failure".to_string(),
                identifier_kind: "email".to_string(),
                identifier: "atomic@example.com".to_string(),
                identifier_normalized: "atomic@example.com".to_string(),
                created_at: 100,
                updated_at: 100,
                state: LocalAccountState::Bootstrap,
                metadata_json: "{}".to_string(),
            };
            let invalid_credential = LocalCredentialSecret {
                local_user_id: identity.id.clone(),
                password_hash: "".to_string(),
                password_hash_algorithm: "bcrypt".to_string(),
                password_hash_params_json: "{}".to_string(),
                password_changed_at: 101,
                must_change_password: true,
            };

            let err = store_local_identity_and_credential_record(&identity, &invalid_credential)
                .expect_err("atomic store should reject invalid credentials");
            assert!(
                err.contains("password_hash"),
                "unexpected invalid credential error: {err}"
            );
            assert_eq!(
                get_local_identity_by_identifier_record("email", "atomic@example.com").unwrap(),
                None,
                "failed atomic store must not leave an orphaned identity"
            );
            assert_eq!(
                get_local_credential_secret_record("local-user-atomic-failure").unwrap(),
                None,
                "failed atomic store must not leave a credential"
            );
        }
    }

    #[test]
    fn test_sqlite_atomic_local_identity_and_credential_store_rolls_back_identity_on_credential_failure(
    ) {
        use super::storage::{
            get_local_identity_by_identifier_record, store_local_identity_and_credential_record,
            LocalAccountState, LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        {
            let conn_guard = SQLITE_CONN.lock().unwrap();
            let conn = conn_guard.as_ref().expect("SQLite should be initialized");
            conn.execute(
                "CREATE TRIGGER fail_local_credential_insert
                 BEFORE INSERT ON auth_local_credentials
                 BEGIN
                     SELECT RAISE(ABORT, 'forced local credential failure');
                 END",
                [],
            )
            .expect("test trigger should be installed");
        }

        let identity = LocalIdentity {
            id: "local-user-sqlite-atomic".to_string(),
            identifier_kind: "email".to_string(),
            identifier: "sqlite-atomic@example.com".to_string(),
            identifier_normalized: "sqlite-atomic@example.com".to_string(),
            created_at: 100,
            updated_at: 100,
            state: LocalAccountState::Bootstrap,
            metadata_json: "{}".to_string(),
        };
        let credential = LocalCredentialSecret {
            local_user_id: identity.id.clone(),
            password_hash: "bcrypt$hash".to_string(),
            password_hash_algorithm: "bcrypt".to_string(),
            password_hash_params_json: "{}".to_string(),
            password_changed_at: 101,
            must_change_password: true,
        };

        let err = store_local_identity_and_credential_record(&identity, &credential)
            .expect_err("credential failure should abort the atomic store");
        assert!(
            err.contains("forced local credential failure"),
            "unexpected SQLite credential failure: {err}"
        );
        assert_eq!(
            get_local_identity_by_identifier_record("email", "sqlite-atomic@example.com").unwrap(),
            None,
            "SQLite transaction failure must roll back the identity insert"
        );
    }

    #[test]
    fn test_local_credential_store_rejects_missing_identity_memory_and_sqlite() {
        use super::storage::{store_local_credential_secret_record, LocalCredentialSecret};

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();

        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let credential = LocalCredentialSecret {
                local_user_id: "missing-local-user".to_string(),
                password_hash: "argon2$hash".to_string(),
                password_hash_algorithm: "argon2id".to_string(),
                password_hash_params_json: "{}".to_string(),
                password_changed_at: 101,
                must_change_password: false,
            };

            let err = store_local_credential_secret_record(&credential)
                .expect_err("orphaned local credentials must be rejected");
            assert!(
                err.contains("local credential") || err.contains("FOREIGN KEY"),
                "unexpected orphan credential error: {err}"
            );
        }
    }

    #[test]
    fn test_sqlite_local_credentials_cascade_when_identity_deleted() {
        use super::storage::{
            get_local_credential_secret_record, store_local_credential_secret_record,
            store_local_identity_record, LocalAccountState, LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        store_local_identity_record(&LocalIdentity {
            id: "local-user-delete-cascade".to_string(),
            identifier_kind: "email".to_string(),
            identifier: "delete@example.com".to_string(),
            identifier_normalized: "delete@example.com".to_string(),
            created_at: 100,
            updated_at: 100,
            state: LocalAccountState::Active,
            metadata_json: "{}".to_string(),
        })
        .unwrap();
        store_local_credential_secret_record(&LocalCredentialSecret {
            local_user_id: "local-user-delete-cascade".to_string(),
            password_hash: "argon2$hash".to_string(),
            password_hash_algorithm: "argon2id".to_string(),
            password_hash_params_json: "{}".to_string(),
            password_changed_at: 101,
            must_change_password: false,
        })
        .unwrap();

        {
            let conn_guard = SQLITE_CONN.lock().unwrap();
            let conn = conn_guard.as_ref().expect("SQLite should be initialized");
            conn.execute(
                "DELETE FROM auth_local_identities WHERE id = ?1",
                rusqlite::params!["local-user-delete-cascade"],
            )
            .expect("deleting a local identity should cascade credential cleanup");
        }

        assert_eq!(
            get_local_credential_secret_record("local-user-delete-cascade").unwrap(),
            None
        );
    }

    #[test]
    fn test_local_identity_store_normalizes_lookup_fields_on_write_memory_and_sqlite() {
        use super::storage::{
            get_local_identity_by_identifier_record, store_local_identity_record,
            LocalAccountState, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();

        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            store_local_identity_record(&LocalIdentity {
                id: "local-user-normalized-write".to_string(),
                identifier_kind: " Email ".to_string(),
                identifier: "Admin@Example.COM".to_string(),
                identifier_normalized: " Admin@Example.COM ".to_string(),
                created_at: 100,
                updated_at: 100,
                state: LocalAccountState::Active,
                metadata_json: "{}".to_string(),
            })
            .unwrap();

            let fetched = get_local_identity_by_identifier_record("email", "admin@example.com")
                .unwrap()
                .expect("stored local identity should be queryable by canonical lookup");
            assert_eq!(fetched.identifier_kind, "email");
            assert_eq!(fetched.identifier_normalized, "admin@example.com");
        }
    }

    #[test]
    fn test_verify_local_password_native_helper_returns_auth_safe_user() {
        use super::storage::{
            store_local_credential_secret_record, store_local_identity_record, LocalAccountState,
            LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let password_hash = bcrypt::hash("correct horse battery staple", 4).unwrap();
            store_local_identity_record(&LocalIdentity {
                id: "local-user-verify".to_string(),
                identifier_kind: "email".to_string(),
                identifier: "Admin@Example.COM".to_string(),
                identifier_normalized: "admin@example.com".to_string(),
                created_at: 100,
                updated_at: 100,
                state: LocalAccountState::Active,
                metadata_json: "{}".to_string(),
            })
            .unwrap();
            store_local_credential_secret_record(&LocalCredentialSecret {
                local_user_id: "local-user-verify".to_string(),
                password_hash,
                password_hash_algorithm: "bcrypt".to_string(),
                password_hash_params_json: "{}".to_string(),
                password_changed_at: 101,
                must_change_password: false,
            })
            .unwrap();

            let module = init();
            let verify_local_password = module_fn(&module, "verify_local_password");
            let verified = verify_local_password(&[
                Value::String(" admin@example.com ".to_string()),
                Value::String("correct horse battery staple".to_string()),
            ])
            .unwrap();

            let Value::EnumValue {
                enum_name,
                variant,
                values,
            } = verified
            else {
                panic!("verify_local_password should return Result");
            };
            assert_eq!(enum_name, "Result");
            assert_eq!(variant, "Ok");
            let Value::Map(user) = &values[0] else {
                panic!("verify_local_password Ok payload should be a map");
            };
            match user.get("subject_id") {
                Some(Value::String(subject_id)) => assert_eq!(subject_id, "local-user-verify"),
                other => panic!("unexpected subject_id payload: {other:?}"),
            }
            match user.get("email") {
                Some(Value::String(email)) => assert_eq!(email, "Admin@Example.COM"),
                other => panic!("unexpected email payload: {other:?}"),
            }
            match user.get("state") {
                Some(Value::String(state)) => assert_eq!(state, "active"),
                other => panic!("unexpected state payload: {other:?}"),
            }
            assert!(
                !user.contains_key("password_hash"),
                "verified local user payload must not expose password hashes"
            );

            let wrong_password = verify_local_password(&[
                Value::String("admin@example.com".to_string()),
                Value::String("wrong".to_string()),
            ])
            .unwrap();
            let Value::EnumValue {
                enum_name,
                variant,
                values,
            } = wrong_password
            else {
                panic!("wrong password should return Result");
            };
            assert_eq!(enum_name, "Result");
            assert_eq!(variant, "Err");
            match values.first() {
                Some(Value::String(message)) => assert_eq!(message, "Invalid local credentials"),
                other => panic!("unexpected wrong-password error payload: {other:?}"),
            }
        }
    }

    #[test]
    fn test_verify_local_password_marks_setup_states_must_change_password() {
        use super::storage::{
            store_local_credential_secret_record, store_local_identity_record, LocalAccountState,
            LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let module = init();
            let verify_local_password = module_fn(&module, "verify_local_password");
            for (state, must_change_password) in [
                (LocalAccountState::Bootstrap, true),
                (LocalAccountState::PendingSetup, true),
                (LocalAccountState::PasswordChangeRequired, true),
                (LocalAccountState::Active, false),
            ] {
                let email = format!("{}@example.com", state.as_str());
                let local_user_id = format!("local-user-{}", state.as_str());
                store_local_identity_record(&LocalIdentity {
                    id: local_user_id.clone(),
                    identifier_kind: "email".to_string(),
                    identifier: email.clone(),
                    identifier_normalized: email.clone(),
                    created_at: 100,
                    updated_at: 100,
                    state,
                    metadata_json: "{}".to_string(),
                })
                .unwrap();
                store_local_credential_secret_record(&LocalCredentialSecret {
                    local_user_id,
                    password_hash: bcrypt::hash("state setup password", 4).unwrap(),
                    password_hash_algorithm: "bcrypt".to_string(),
                    password_hash_params_json: "{}".to_string(),
                    password_changed_at: 101,
                    must_change_password: false,
                })
                .unwrap();

                let verified = verify_local_password(&[
                    Value::String(email),
                    Value::String("state setup password".to_string()),
                ])
                .unwrap();
                let Value::EnumValue {
                    enum_name,
                    variant,
                    values,
                } = verified
                else {
                    panic!("verify_local_password should return Result");
                };
                assert_eq!(enum_name, "Result");
                assert_eq!(variant, "Ok");
                let Value::Map(user) = &values[0] else {
                    panic!("verify_local_password Ok payload should be a map");
                };
                match user.get("must_change_password") {
                    Some(Value::Bool(value)) => assert_eq!(*value, must_change_password),
                    other => panic!("unexpected must_change_password payload: {other:?}"),
                }
            }
        }
    }

    #[test]
    fn test_verify_local_password_rejects_enumeration_states_with_generic_error() {
        use super::storage::{
            store_local_credential_secret_record, store_local_identity_record, LocalAccountState,
            LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let password_hash = bcrypt::hash("valid local password", 4).unwrap();
            for (id, email, state, with_credential) in [
                (
                    "local-user-active",
                    "active@example.com",
                    LocalAccountState::Active,
                    true,
                ),
                (
                    "local-user-disabled",
                    "disabled@example.com",
                    LocalAccountState::Disabled,
                    true,
                ),
                (
                    "local-user-locked",
                    "locked@example.com",
                    LocalAccountState::Locked,
                    true,
                ),
                (
                    "local-user-no-credential",
                    "no-credential@example.com",
                    LocalAccountState::Active,
                    false,
                ),
            ] {
                store_local_identity_record(&LocalIdentity {
                    id: id.to_string(),
                    identifier_kind: "email".to_string(),
                    identifier: email.to_string(),
                    identifier_normalized: email.to_string(),
                    created_at: 100,
                    updated_at: 100,
                    state,
                    metadata_json: "{}".to_string(),
                })
                .unwrap();
                if with_credential {
                    store_local_credential_secret_record(&LocalCredentialSecret {
                        local_user_id: id.to_string(),
                        password_hash: password_hash.clone(),
                        password_hash_algorithm: "bcrypt".to_string(),
                        password_hash_params_json: "{}".to_string(),
                        password_changed_at: 101,
                        must_change_password: false,
                    })
                    .unwrap();
                }
            }

            let module = init();
            let verify_local_password = module_fn(&module, "verify_local_password");
            for (email, password) in [
                ("active@example.com", "wrong local password"),
                ("missing@example.com", "valid local password"),
                ("no-credential@example.com", "valid local password"),
                ("disabled@example.com", "valid local password"),
                ("locked@example.com", "valid local password"),
            ] {
                let message = result_err_string(
                    verify_local_password(&[
                        Value::String(email.to_string()),
                        Value::String(password.to_string()),
                    ])
                    .unwrap(),
                );
                assert_eq!(message, "Invalid local credentials");
            }
        }
    }

    #[test]
    fn test_verify_local_password_reports_corrupted_hashes_as_safe_operational_errors() {
        use super::storage::{
            store_local_credential_secret_record, store_local_identity_record, LocalAccountState,
            LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);
            store_local_identity_record(&LocalIdentity {
                id: "local-user-corrupted-hash".to_string(),
                identifier_kind: "email".to_string(),
                identifier: "corrupt@example.com".to_string(),
                identifier_normalized: "corrupt@example.com".to_string(),
                created_at: 100,
                updated_at: 100,
                state: LocalAccountState::Active,
                metadata_json: "{}".to_string(),
            })
            .unwrap();

            let module = init();
            let verify_local_password = module_fn(&module, "verify_local_password");
            for (password_hash, algorithm) in [
                ("not-a-valid-password-hash", "bcrypt"),
                ("not-a-valid-password-hash", "argon2id"),
                ("not-a-valid-password-hash", "scrypt"),
            ] {
                store_local_credential_secret_record(&LocalCredentialSecret {
                    local_user_id: "local-user-corrupted-hash".to_string(),
                    password_hash: password_hash.to_string(),
                    password_hash_algorithm: algorithm.to_string(),
                    password_hash_params_json: "{}".to_string(),
                    password_changed_at: 101,
                    must_change_password: false,
                })
                .unwrap();

                let message = result_err_string(
                    verify_local_password(&[
                        Value::String("corrupt@example.com".to_string()),
                        Value::String("valid local password".to_string()),
                    ])
                    .unwrap(),
                );
                assert_eq!(
                    message,
                    "[auth] local credential hash is invalid or unsupported"
                );
                assert!(!message.contains(algorithm));
                assert!(!message.contains("password hash"));
            }
        }
    }

    #[test]
    fn test_bootstrap_local_user_native_helper_provisions_safe_setup_user() {
        use super::storage::{
            get_local_credential_secret_record, get_local_identity_by_identifier_record,
            LocalAccountState,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let module = init();
            let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
            let bootstrapped = result_ok_map(
                bootstrap_local_user(&[
                    Value::String(" Bootstrap@Example.COM ".to_string()),
                    Value::String("temporary bootstrap password".to_string()),
                ])
                .unwrap(),
            );

            let local_user_id = match bootstrapped.get("local_user_id") {
                Some(Value::String(id)) => id.clone(),
                other => panic!("unexpected local_user_id payload: {other:?}"),
            };
            match bootstrapped.get("subject_id") {
                Some(Value::String(subject_id)) => assert_eq!(subject_id, &local_user_id),
                other => panic!("unexpected subject_id payload: {other:?}"),
            }
            match bootstrapped.get("provider") {
                Some(Value::String(provider)) => assert_eq!(provider, "local"),
                other => panic!("unexpected provider payload: {other:?}"),
            }
            match bootstrapped.get("email") {
                Some(Value::String(email)) => assert_eq!(email, "Bootstrap@Example.COM"),
                other => panic!("unexpected email payload: {other:?}"),
            }
            match bootstrapped.get("identifier_normalized") {
                Some(Value::String(identifier)) => assert_eq!(identifier, "bootstrap@example.com"),
                other => panic!("unexpected identifier_normalized payload: {other:?}"),
            }
            match bootstrapped.get("state") {
                Some(Value::String(state)) => assert_eq!(state, "bootstrap"),
                other => panic!("unexpected state payload: {other:?}"),
            }
            match bootstrapped.get("must_change_password") {
                Some(Value::Bool(value)) => assert!(*value),
                other => panic!("unexpected must_change_password payload: {other:?}"),
            }
            for secret_key in [
                "password",
                "password_hash",
                "password_hash_algorithm",
                "password_hash_params_json",
                "credential",
                "secret",
                "token",
            ] {
                assert!(
                    !bootstrapped.contains_key(secret_key),
                    "bootstrap payload must not expose {secret_key}"
                );
            }

            let stored_identity =
                get_local_identity_by_identifier_record("email", "bootstrap@example.com")
                    .unwrap()
                    .expect("bootstrap should store a local identity");
            assert_eq!(stored_identity.id, local_user_id);
            assert_eq!(stored_identity.state, LocalAccountState::Bootstrap);
            let credential = get_local_credential_secret_record(&stored_identity.id)
                .unwrap()
                .expect("bootstrap should store a local credential secret");
            assert_ne!(credential.password_hash, "temporary bootstrap password");
            assert!(!credential.password_hash.trim().is_empty());
            assert!(credential.must_change_password);

            let verify_local_password = module_fn(&module, "verify_local_password");
            let verified = result_ok_map(
                verify_local_password(&[
                    Value::String("bootstrap@example.com".to_string()),
                    Value::String("temporary bootstrap password".to_string()),
                ])
                .unwrap(),
            );
            match verified.get("local_user_id") {
                Some(Value::String(id)) => assert_eq!(id, &stored_identity.id),
                other => panic!("unexpected verified local_user_id payload: {other:?}"),
            }
        }
    }

    #[test]
    fn test_set_local_password_native_helper_completes_setup_and_rotates_secret() {
        use super::storage::{
            get_local_credential_secret_record, get_local_identity_by_identifier_record,
            LocalAccountState,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let module = init();
            let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
            let set_local_password = module_fn(&module, "set_local_password");
            let verify_local_password = module_fn(&module, "verify_local_password");

            let bootstrapped = result_ok_map(
                bootstrap_local_user(&[
                    Value::String(" Rotate@Example.COM ".to_string()),
                    Value::String("temporary setup password".to_string()),
                ])
                .unwrap(),
            );
            match bootstrapped.get("must_change_password") {
                Some(Value::Bool(value)) => assert!(*value),
                other => panic!("unexpected bootstrap must_change_password payload: {other:?}"),
            }

            let missing_current_password = set_local_password(&[
                Value::String(" rotate@example.com ".to_string()),
                Value::String("rotated local password".to_string()),
            ])
            .expect_err("set_local_password should require the current password");
            assert!(
                missing_current_password
                    .to_string()
                    .contains("requires identifier, current_password, new_password"),
                "unexpected missing-current-password error: {missing_current_password}"
            );

            let wrong_current_password = result_err_string(
                set_local_password(&[
                    Value::String(" rotate@example.com ".to_string()),
                    Value::String("wrong setup password".to_string()),
                    Value::String("rotated local password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(wrong_current_password, "Invalid local credentials");

            let same_password = result_err_string(
                set_local_password(&[
                    Value::String(" rotate@example.com ".to_string()),
                    Value::String("temporary setup password".to_string()),
                    Value::String("temporary setup password".to_string()),
                ])
                .unwrap(),
            );
            assert!(
                same_password.contains("must differ from current password"),
                "unexpected same-password error: {same_password}"
            );
            let setup_identity =
                get_local_identity_by_identifier_record("email", "rotate@example.com")
                    .unwrap()
                    .expect("setup local identity should still be stored");
            assert_eq!(setup_identity.state, LocalAccountState::Bootstrap);
            let setup_credential = get_local_credential_secret_record(&setup_identity.id)
                .unwrap()
                .expect("setup local credential should still be stored");
            assert!(setup_credential.must_change_password);

            let updated = result_ok_map(
                set_local_password(&[
                    Value::String(" rotate@example.com ".to_string()),
                    Value::String("temporary setup password".to_string()),
                    Value::String("rotated local password".to_string()),
                ])
                .unwrap(),
            );
            let local_user_id = match updated.get("local_user_id") {
                Some(Value::String(id)) => id.clone(),
                other => panic!("unexpected local_user_id payload: {other:?}"),
            };
            match updated.get("state") {
                Some(Value::String(state)) => assert_eq!(state, "active"),
                other => panic!("unexpected state payload: {other:?}"),
            }
            match updated.get("must_change_password") {
                Some(Value::Bool(value)) => assert!(!*value),
                other => panic!("unexpected must_change_password payload: {other:?}"),
            }
            for secret_key in [
                "password",
                "password_hash",
                "password_hash_algorithm",
                "password_hash_params_json",
                "credential",
                "secret",
                "token",
            ] {
                assert!(
                    !updated.contains_key(secret_key),
                    "set_local_password payload must not expose {secret_key}"
                );
            }

            let stored_identity =
                get_local_identity_by_identifier_record("email", "rotate@example.com")
                    .unwrap()
                    .expect("rotated local identity should be stored");
            assert_eq!(stored_identity.id, local_user_id);
            assert_eq!(stored_identity.state, LocalAccountState::Active);
            let credential = get_local_credential_secret_record(&stored_identity.id)
                .unwrap()
                .expect("rotated local credential should be stored");
            assert!(!credential.must_change_password);
            assert_ne!(credential.password_hash, "rotated local password");

            let old_password = result_err_string(
                verify_local_password(&[
                    Value::String("rotate@example.com".to_string()),
                    Value::String("temporary setup password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(old_password, "Invalid local credentials");

            let verified = result_ok_map(
                verify_local_password(&[
                    Value::String("rotate@example.com".to_string()),
                    Value::String("rotated local password".to_string()),
                ])
                .unwrap(),
            );
            match verified.get("state") {
                Some(Value::String(state)) => assert_eq!(state, "active"),
                other => panic!("unexpected verified state payload: {other:?}"),
            }
            match verified.get("must_change_password") {
                Some(Value::Bool(value)) => assert!(!*value),
                other => panic!("unexpected verified must_change_password payload: {other:?}"),
            }
        }
    }

    #[test]
    fn test_password_reset_helpers_issue_consume_and_reject_replay_memory_and_sqlite() {
        use super::storage::{
            get_local_credential_secret_record, get_local_identity_by_identifier_record,
            LocalAccountState,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let module = init();
            let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
            let issue_password_reset = module_fn(&module, "issue_password_reset");
            let consume_password_reset = module_fn(&module, "consume_password_reset");
            let verify_local_password = module_fn(&module, "verify_local_password");

            result_ok_map(
                bootstrap_local_user(&[
                    Value::String(" Reset@Example.COM ".to_string()),
                    Value::String("temporary reset password".to_string()),
                ])
                .unwrap(),
            );

            let issued = result_ok_map(
                issue_password_reset(&[Value::String(" reset@example.com ".to_string())]).unwrap(),
            );
            let token = map_string(&issued, "token");
            let selector = map_string(&issued, "selector");
            assert!(
                token.len() >= 64,
                "reset token should have high-entropy material"
            );
            assert!(
                token.starts_with(&format!("{selector}.")),
                "token should include its selector prefix for lookup"
            );
            assert!(map_int(&issued, "expires_at") > map_int(&issued, "created_at"));
            let sibling_issued = result_ok_map(
                issue_password_reset(&[Value::String("reset@example.com".to_string())]).unwrap(),
            );
            let sibling_token = map_string(&sibling_issued, "token");
            for secret_key in [
                "password",
                "password_hash",
                "password_hash_algorithm",
                "password_hash_params_json",
                "credential",
                "secret",
                "token_hash",
                "local_user_id",
            ] {
                assert!(
                    !issued.contains_key(secret_key),
                    "issue_password_reset payload must not expose {secret_key}"
                );
            }

            let stored_identity =
                get_local_identity_by_identifier_record("email", "reset@example.com")
                    .unwrap()
                    .expect("reset identity should be stored");
            assert_eq!(stored_identity.state, LocalAccountState::Bootstrap);
            assert!(
                !stored_identity.metadata_json.contains(&token),
                "raw reset token must not be stored in local identity metadata"
            );
            assert!(
                !stored_identity.metadata_json.contains("token_hash"),
                "reset token hashes belong in reset-token storage, not metadata"
            );

            store_session_record(&Session {
                id: "reset-default-keeps-session".to_string(),
                user_id: stored_identity.id.clone(),
                provider: "local".to_string(),
                email: Some("reset@example.com".to_string()),
                name: None,
                picture: None,
                raw_json: "{}".to_string(),
                data_json: "{}".to_string(),
                csrf_token: "csrf-reset-default-keeps-session".to_string(),
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                device_name: None,
                user_agent_hash: None,
                last_ip_hash: None,
                created_at: chrono::Utc::now().timestamp(),
                expires_at: chrono::Utc::now().timestamp() + 300,
            })
            .unwrap();
            let consumed = result_ok_map(
                consume_password_reset(&[
                    Value::String(token.clone()),
                    Value::String("rotated by reset".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(map_int(&consumed, "revoked_sessions"), 0);
            assert!(
                get_session_record("reset-default-keeps-session")
                    .unwrap()
                    .is_some(),
                "password reset must keep existing sessions unless explicitly asked to revoke"
            );
            delete_session_record("reset-default-keeps-session").unwrap();
            assert_eq!(map_string(&consumed, "local_user_id"), stored_identity.id);
            match consumed.get("state") {
                Some(Value::String(state)) => assert_eq!(state, "active"),
                other => panic!("unexpected reset state payload: {other:?}"),
            }
            match consumed.get("must_change_password") {
                Some(Value::Bool(value)) => assert!(!*value),
                other => panic!("unexpected reset must_change_password payload: {other:?}"),
            }
            for secret_key in [
                "password",
                "password_hash",
                "password_hash_algorithm",
                "password_hash_params_json",
                "credential",
                "secret",
                "token",
                "selector",
                "token_hash",
            ] {
                assert!(
                    !consumed.contains_key(secret_key),
                    "consume_password_reset payload must not expose {secret_key}"
                );
            }

            let old_password = result_err_string(
                verify_local_password(&[
                    Value::String("reset@example.com".to_string()),
                    Value::String("temporary reset password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(old_password, "Invalid local credentials");

            let verified = result_ok_map(
                verify_local_password(&[
                    Value::String("reset@example.com".to_string()),
                    Value::String("rotated by reset".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(map_string(&verified, "local_user_id"), stored_identity.id);
            let rotated_credential = get_local_credential_secret_record(&stored_identity.id)
                .unwrap()
                .expect("reset credential should be stored");
            assert!(!rotated_credential.must_change_password);

            let replay = result_err_string(
                consume_password_reset(&[
                    Value::String(token),
                    Value::String("replay reset password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(replay, "Invalid password reset token");

            let sibling_replay = result_err_string(
                consume_password_reset(&[
                    Value::String(sibling_token),
                    Value::String("sibling reset password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(sibling_replay, "Invalid password reset token");

            let revoke_issued = result_ok_map(
                issue_password_reset(&[Value::String("reset@example.com".to_string())]).unwrap(),
            );
            let revoke_token = map_string(&revoke_issued, "token");
            store_session_record(&Session {
                id: "reset-option-revokes-session".to_string(),
                user_id: stored_identity.id.clone(),
                provider: "local".to_string(),
                email: Some("reset@example.com".to_string()),
                name: None,
                picture: None,
                raw_json: "{}".to_string(),
                data_json: "{}".to_string(),
                csrf_token: "csrf-reset-option-revokes-session".to_string(),
                access_token: None,
                refresh_token: None,
                token_expires_at: None,
                device_name: None,
                user_agent_hash: None,
                last_ip_hash: None,
                created_at: chrono::Utc::now().timestamp(),
                expires_at: chrono::Utc::now().timestamp() + 300,
            })
            .unwrap();
            let consumed_with_revoke = result_ok_map(
                consume_password_reset(&[
                    Value::String(revoke_token),
                    Value::String("rotated and revoked sessions".to_string()),
                    Value::Map(HashMap::from([(
                        "revoke_sessions".to_string(),
                        Value::Bool(true),
                    )])),
                ])
                .unwrap(),
            );
            assert_eq!(map_int(&consumed_with_revoke, "revoked_sessions"), 1);
            assert!(
                get_session_record("reset-option-revokes-session")
                    .unwrap()
                    .is_none(),
                "explicit revoke_sessions option should revoke existing sessions"
            );
        }
    }

    #[test]
    fn test_set_local_password_revokes_outstanding_password_reset_tokens() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let module = init();
            let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
            let issue_password_reset = module_fn(&module, "issue_password_reset");
            let consume_password_reset = module_fn(&module, "consume_password_reset");
            let set_local_password = module_fn(&module, "set_local_password");
            let verify_local_password = module_fn(&module, "verify_local_password");

            result_ok_map(
                bootstrap_local_user(&[
                    Value::String("manual-rotate@example.com".to_string()),
                    Value::String("temporary reset password".to_string()),
                ])
                .unwrap(),
            );
            let issued = result_ok_map(
                issue_password_reset(&[Value::String("manual-rotate@example.com".to_string())])
                    .unwrap(),
            );
            let token = map_string(&issued, "token");

            let rotated = result_ok_map(
                set_local_password(&[
                    Value::String("manual-rotate@example.com".to_string()),
                    Value::String("temporary reset password".to_string()),
                    Value::String("manually rotated password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(map_string(&rotated, "state"), "active");

            let stale_reset = result_err_string(
                consume_password_reset(&[
                    Value::String(token),
                    Value::String("attacker chosen reset password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(stale_reset, "Invalid password reset token");

            let current_password_still_valid = result_ok_map(
                verify_local_password(&[
                    Value::String("manual-rotate@example.com".to_string()),
                    Value::String("manually rotated password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(map_string(&current_password_still_valid, "state"), "active");
        }
    }

    #[test]
    fn test_password_reset_consume_is_atomic_when_sqlite_credential_write_fails() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let module = init();
        let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
        let issue_password_reset = module_fn(&module, "issue_password_reset");
        let consume_password_reset = module_fn(&module, "consume_password_reset");
        let verify_local_password = module_fn(&module, "verify_local_password");

        result_ok_map(
            bootstrap_local_user(&[
                Value::String("atomic-reset@example.com".to_string()),
                Value::String("temporary reset password".to_string()),
            ])
            .unwrap(),
        );
        let issued = result_ok_map(
            issue_password_reset(&[Value::String("atomic-reset@example.com".to_string())]).unwrap(),
        );
        let token = map_string(&issued, "token");

        {
            let conn_guard = SQLITE_CONN.lock().unwrap();
            let conn = conn_guard.as_ref().expect("SQLite should be initialized");
            conn.execute(
                "CREATE TRIGGER fail_password_reset_credential_update
                 BEFORE UPDATE ON auth_local_credentials
                 BEGIN
                     SELECT RAISE(ABORT, 'forced password reset credential failure');
                 END",
                [],
            )
            .expect("test trigger should be installed");
        }

        let failed = result_err_string(
            consume_password_reset(&[
                Value::String(token.clone()),
                Value::String("rotated by reset".to_string()),
            ])
            .unwrap(),
        );
        assert!(
            failed.contains("forced password reset credential failure"),
            "unexpected storage failure message: {failed}"
        );

        let old_password_still_valid = result_ok_map(
            verify_local_password(&[
                Value::String("atomic-reset@example.com".to_string()),
                Value::String("temporary reset password".to_string()),
            ])
            .unwrap(),
        );
        assert_eq!(map_string(&old_password_still_valid, "state"), "bootstrap");

        {
            let conn_guard = SQLITE_CONN.lock().unwrap();
            let conn = conn_guard.as_ref().expect("SQLite should be initialized");
            conn.execute("DROP TRIGGER fail_password_reset_credential_update", [])
                .expect("test trigger should be dropped");
        }

        let consumed = result_ok_map(
            consume_password_reset(&[
                Value::String(token),
                Value::String("rotated by reset".to_string()),
            ])
            .unwrap(),
        );
        assert_eq!(map_string(&consumed, "state"), "active");

        let new_password = result_ok_map(
            verify_local_password(&[
                Value::String("atomic-reset@example.com".to_string()),
                Value::String("rotated by reset".to_string()),
            ])
            .unwrap(),
        );
        assert_eq!(map_string(&new_password, "state"), "active");
    }

    #[test]
    fn test_password_reset_helpers_use_generic_responses_for_missing_expired_and_malformed_tokens()
    {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            let module = init();
            let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
            let issue_password_reset = module_fn(&module, "issue_password_reset");
            let consume_password_reset = module_fn(&module, "consume_password_reset");
            let verify_local_password = module_fn(&module, "verify_local_password");

            let missing_user = result_ok_map(
                issue_password_reset(&[Value::String("missing@example.com".to_string())]).unwrap(),
            );
            assert_eq!(map_string(&missing_user, "status"), "accepted");
            let missing_token = map_string(&missing_user, "token");
            let missing_selector = map_string(&missing_user, "selector");
            assert!(
                missing_token.starts_with(&format!("{missing_selector}.")),
                "missing-account issuance should preserve response shape without storing a usable token"
            );
            let missing_consume = result_err_string(
                consume_password_reset(&[
                    Value::String(missing_token),
                    Value::String("missing account reset password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(missing_consume, "Invalid password reset token");

            result_ok_map(
                bootstrap_local_user(&[
                    Value::String("expire@example.com".to_string()),
                    Value::String("temporary reset password".to_string()),
                ])
                .unwrap(),
            );
            let expired_issue = result_ok_map(
                issue_password_reset(&[
                    Value::String("expire@example.com".to_string()),
                    Value::Map(HashMap::from([("ttl_seconds".to_string(), Value::Int(0))])),
                ])
                .unwrap(),
            );
            assert_eq!(map_string(&expired_issue, "status"), "accepted");
            assert!(
                !expired_issue.contains_key("token") && !expired_issue.contains_key("selector"),
                "zero-ttl reset issuance must not return unstored token material"
            );

            let malformed = result_err_string(
                consume_password_reset(&[
                    Value::String("not-a-valid-reset-token".to_string()),
                    Value::String("malformed reset password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(malformed, "Invalid password reset token");

            let wrong_verifier_issue = result_ok_map(
                issue_password_reset(&[Value::String("expire@example.com".to_string())]).unwrap(),
            );
            let wrong_selector = map_string(&wrong_verifier_issue, "selector");
            let wrong_verifier_token = format!("{wrong_selector}.definitely-wrong-verifier");
            let wrong_verifier = result_err_string(
                consume_password_reset(&[
                    Value::String(wrong_verifier_token),
                    Value::String("wrong verifier password".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(wrong_verifier, "Invalid password reset token");

            let valid_after_wrong_verifier = result_ok_map(
                consume_password_reset(&[
                    Value::String(map_string(&wrong_verifier_issue, "token")),
                    Value::String("valid after wrong verifier".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(map_string(&valid_after_wrong_verifier, "state"), "active");

            let rotated_password = result_ok_map(
                verify_local_password(&[
                    Value::String("expire@example.com".to_string()),
                    Value::String("valid after wrong verifier".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(map_string(&rotated_password, "state"), "active");
        }
    }

    #[test]
    fn test_local_user_metadata_updates_merge_atomically_in_memory() {
        use super::storage::{store_local_identity_record, LocalAccountState, LocalIdentity};

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        store_local_identity_record(&LocalIdentity {
            id: "local-user-atomic-metadata".to_string(),
            identifier_kind: "email".to_string(),
            identifier: "Atomic@Example.COM".to_string(),
            identifier_normalized: "atomic@example.com".to_string(),
            created_at: 100,
            updated_at: 100,
            state: LocalAccountState::Active,
            metadata_json: "{}".to_string(),
        })
        .unwrap();

        let thread_count = 24;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(thread_count));
        let handles: Vec<_> = (0..thread_count)
            .map(|index| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    super::local::update_local_user_metadata_record(
                        "email",
                        "atomic@example.com",
                        &HashMap::from([(
                            format!("flag_{index}"),
                            Value::String(format!("value_{index}")),
                        )]),
                        false,
                    )
                    .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let module = init();
        let local_user = module_fn(&module, "local_user");
        let user =
            result_ok_map(local_user(&[Value::String("atomic@example.com".to_string())]).unwrap());
        let Some(Value::Map(metadata)) = user.get("metadata") else {
            panic!("expected metadata map after concurrent updates, got {user:?}");
        };
        for index in 0..thread_count {
            match metadata.get(&format!("flag_{index}")) {
                Some(Value::String(value)) => assert_eq!(value, &format!("value_{index}")),
                other => panic!("metadata update for flag_{index} was lost: {other:?}"),
            }
        }
    }

    #[test]
    fn test_local_user_metadata_helpers_round_trip_memory_and_sqlite() {
        use super::storage::{
            get_local_identity_by_identifier_record, store_local_credential_secret_record,
            store_local_identity_record, LocalAccountState, LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            store_local_identity_record(&LocalIdentity {
                id: "local-user-metadata".to_string(),
                identifier_kind: "email".to_string(),
                identifier: "Meta@Example.COM".to_string(),
                identifier_normalized: "meta@example.com".to_string(),
                created_at: 100,
                updated_at: 100,
                state: LocalAccountState::Active,
                metadata_json: r#"{"app":{"theme":"dark"},"auth":{"totp":{"enabled":true,"secret":"server-only"}}}"#.to_string(),
            })
            .unwrap();
            store_local_credential_secret_record(&LocalCredentialSecret {
                local_user_id: "local-user-metadata".to_string(),
                password_hash: bcrypt::hash("metadata password", 4).unwrap(),
                password_hash_algorithm: "bcrypt".to_string(),
                password_hash_params_json: "{}".to_string(),
                password_changed_at: 101,
                must_change_password: false,
            })
            .unwrap();

            let module = init();
            let local_user = module_fn(&module, "local_user");
            let update_local_user_metadata = module_fn(&module, "update_local_user_metadata");
            let verify_local_password = module_fn(&module, "verify_local_password");

            let user = result_ok_map(
                local_user(&[Value::String(" meta@example.com ".to_string())]).unwrap(),
            );
            match user.get("metadata") {
                Some(Value::Map(metadata)) => {
                    assert!(metadata.contains_key("app"));
                    assert!(
                        !format!("{metadata:?}").contains("server-only"),
                        "safe local_user metadata must not expose std/auth secret material"
                    );
                }
                other => panic!("expected safe metadata map, got {other:?}"),
            }

            let updated = result_ok_map(
                update_local_user_metadata(&[
                    Value::String("meta@example.com".to_string()),
                    Value::Map(HashMap::from([(
                        "app".to_string(),
                        Value::Map(HashMap::from([
                            ("theme".to_string(), Value::String("light".to_string())),
                            (
                                "group_ids".to_string(),
                                Value::Array(vec![Value::String("admins".to_string())]),
                            ),
                        ])),
                    )])),
                ])
                .unwrap(),
            );
            match updated.get("metadata") {
                Some(Value::Map(metadata)) => match metadata.get("app") {
                    Some(Value::Map(app)) => {
                        match app.get("theme") {
                            Some(Value::String(theme)) => assert_eq!(theme, "light"),
                            other => panic!("expected theme metadata string, got {other:?}"),
                        }
                        match app.get("group_ids") {
                            Some(Value::Array(group_ids)) => match group_ids.first() {
                                Some(Value::String(group_id)) => assert_eq!(group_id, "admins"),
                                other => panic!("expected group id string, got {other:?}"),
                            },
                            other => panic!("expected group_ids metadata array, got {other:?}"),
                        }
                    }
                    other => panic!("expected app metadata map, got {other:?}"),
                },
                other => panic!("expected safe metadata map, got {other:?}"),
            }

            let verified = result_ok_map(
                verify_local_password(&[
                    Value::String("meta@example.com".to_string()),
                    Value::String("metadata password".to_string()),
                ])
                .unwrap(),
            );
            assert!(
                !verified.contains_key("metadata"),
                "verify_local_password must stay auth-safe and not expose metadata by default"
            );

            let reserved_update = result_err_string(
                update_local_user_metadata(&[
                    Value::String("meta@example.com".to_string()),
                    Value::Map(HashMap::from([(
                        "auth".to_string(),
                        Value::Map(HashMap::from([(
                            "totp".to_string(),
                            Value::String("client-controlled".to_string()),
                        )])),
                    )])),
                ])
                .unwrap(),
            );
            assert!(reserved_update.contains("reserved"));

            let empty_replace = result_err_string(
                update_local_user_metadata(&[
                    Value::String("meta@example.com".to_string()),
                    Value::Map(HashMap::new()),
                    Value::Map(HashMap::from([("replace".to_string(), Value::Bool(true))])),
                ])
                .unwrap(),
            );
            assert!(empty_replace.contains("replace=true with empty metadata"));

            let stored = get_local_identity_by_identifier_record("email", "meta@example.com")
                .unwrap()
                .unwrap();
            assert!(stored.metadata_json.contains("\"theme\":\"light\""));
            assert!(stored.metadata_json.contains("server-only"));
        }
    }

    #[test]
    fn test_totp_enrollment_helpers_round_trip_memory_and_sqlite_without_secret_leakage() {
        use super::storage::{
            get_local_identity_by_identifier_record, store_local_credential_secret_record,
            store_local_identity_record, LocalAccountState, LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        for store in [
            SessionStore::Memory,
            SessionStore::Sqlite(":memory:".to_string()),
        ] {
            reset_auth_test_state();
            init_test_auth(store);

            store_local_identity_record(&LocalIdentity {
                id: "local-user-totp".to_string(),
                identifier_kind: "email".to_string(),
                identifier: "Totp@Example.COM".to_string(),
                identifier_normalized: "totp@example.com".to_string(),
                created_at: 100,
                updated_at: 100,
                state: LocalAccountState::Active,
                metadata_json: r#"{"app":{"theme":"dark"}}"#.to_string(),
            })
            .unwrap();
            store_local_credential_secret_record(&LocalCredentialSecret {
                local_user_id: "local-user-totp".to_string(),
                password_hash: bcrypt::hash("totp password", 4).unwrap(),
                password_hash_algorithm: "bcrypt".to_string(),
                password_hash_params_json: "{}".to_string(),
                password_changed_at: 101,
                must_change_password: false,
            })
            .unwrap();

            let module = init();
            let begin_totp_enrollment = module_fn(&module, "begin_totp_enrollment");
            let confirm_totp_enrollment = module_fn(&module, "confirm_totp_enrollment");
            let verify_local_totp = module_fn(&module, "verify_local_totp");
            let totp_status = module_fn(&module, "totp_status");
            let reset_totp = module_fn(&module, "reset_totp");
            let local_user = module_fn(&module, "local_user");
            let sign_in_session = module_fn(&module, "sign_in_session");
            let current_user = module_fn(&module, "current_user");

            let enrollment = result_ok_map(
                begin_totp_enrollment(&[
                    Value::String("totp@example.com".to_string()),
                    Value::Map(HashMap::from([(
                        "issuer".to_string(),
                        Value::String("Example Admin".to_string()),
                    )])),
                ])
                .unwrap(),
            );
            assert_map_bool(&enrollment, "pending", true);
            assert_map_bool(&enrollment, "enabled", false);
            let enrollment_created_at = map_int(&enrollment, "created_at");
            assert!(enrollment_created_at > 0);
            assert!(
                !enrollment.contains_key("secret"),
                "begin_totp_enrollment must not expose the raw secret field"
            );
            match enrollment.get("uri") {
                Some(Value::String(uri)) => assert!(uri.starts_with("otpauth://totp/")),
                other => panic!("expected setup uri, got {other:?}"),
            }

            let safe_user = result_ok_map(
                local_user(&[Value::String("totp@example.com".to_string())]).unwrap(),
            );

            let stored = get_local_identity_by_identifier_record("email", "totp@example.com")
                .unwrap()
                .unwrap();
            assert!(stored.metadata_json.contains("pending_secret"));
            let pending_secret = serde_json::from_str::<serde_json::Value>(&stored.metadata_json)
                .unwrap()["auth"]["totp"]["pending_secret"]
                .as_str()
                .unwrap()
                .to_string();
            assert_no_totp_secret_material(&Value::Map(safe_user.clone()), &pending_secret);

            let bad_code = result_err_string(
                confirm_totp_enrollment(&[
                    Value::String("totp@example.com".to_string()),
                    Value::String("000000".to_string()),
                ])
                .unwrap(),
            );
            assert!(bad_code.contains("Invalid local TOTP code"));

            let secret_bytes = Secret::Encoded(pending_secret.clone())
                .to_bytes()
                .expect("test secret should decode");
            let totp = TOTP::new(
                TotpAlgorithm::SHA1,
                6,
                1,
                30,
                secret_bytes,
                Some("Example Admin".to_string()),
                "totp@example.com".to_string(),
            )
            .expect("test TOTP should construct");
            let valid_code = totp.generate_current().expect("test code should generate");

            let confirmed = result_ok_map(
                confirm_totp_enrollment(&[
                    Value::String("totp@example.com".to_string()),
                    Value::String(valid_code.clone()),
                ])
                .unwrap(),
            );
            assert_map_bool(&confirmed, "enabled", true);
            assert_map_bool(&confirmed, "pending", false);
            assert_eq!(map_int(&confirmed, "created_at"), enrollment_created_at);
            assert_no_totp_secret_material(&Value::Map(confirmed.clone()), &pending_secret);

            let verified = result_ok_map(
                verify_local_totp(&[
                    Value::String("totp@example.com".to_string()),
                    Value::String(valid_code),
                ])
                .unwrap(),
            );
            assert_map_bool(&verified, "verified", true);
            assert_map_bool(&verified, "enabled", true);

            let status = result_ok_map(
                totp_status(&[Value::String("totp@example.com".to_string())]).unwrap(),
            );
            assert_map_bool(&status, "enabled", true);
            assert_eq!(map_int(&status, "created_at"), enrollment_created_at);
            assert_no_totp_secret_material(&Value::Map(status.clone()), &pending_secret);

            let signed_in = sign_in_session(&[
                redirect_response("/admin", None),
                request_with_cookie(""),
                Value::Map(HashMap::from([
                    (
                        "subject_id".to_string(),
                        Value::String("local-user-totp".to_string()),
                    ),
                    (
                        "email".to_string(),
                        Value::String("totp@example.com".to_string()),
                    ),
                ])),
            ])
            .unwrap();
            let cookie = cookie_header_from_response(&signed_in);
            let current_user_value = current_user(&[request_with_cookie(&cookie)]).unwrap();
            assert_no_totp_secret_material(&current_user_value, &pending_secret);

            let active_reenrollment = result_ok_map(
                begin_totp_enrollment(&[Value::String("totp@example.com".to_string())]).unwrap(),
            );
            assert_map_bool(&active_reenrollment, "enabled", true);
            assert_map_bool(&active_reenrollment, "pending", true);
            assert_eq!(
                map_int(&active_reenrollment, "created_at"),
                enrollment_created_at
            );

            let reset = result_ok_map(
                reset_totp(&[Value::String("totp@example.com".to_string())]).unwrap(),
            );
            assert_map_bool(&reset, "enabled", false);
            assert_map_bool(&reset, "pending", false);

            let after_reset = get_local_identity_by_identifier_record("email", "totp@example.com")
                .unwrap()
                .unwrap();
            assert!(!after_reset.metadata_json.contains("totp"));
            assert!(after_reset.metadata_json.contains("theme"));

            let reenrollment = result_ok_map(
                begin_totp_enrollment(&[Value::String("totp@example.com".to_string())]).unwrap(),
            );
            assert_map_bool(&reenrollment, "pending", true);
            assert_map_bool(&reenrollment, "enabled", false);
        }
    }

    #[test]
    fn test_totp_helpers_reject_disabled_and_locked_local_accounts() {
        use super::storage::{store_local_identity_record, LocalAccountState, LocalIdentity};

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        for (email, state) in [
            ("disabled@example.com", LocalAccountState::Disabled),
            ("locked@example.com", LocalAccountState::Locked),
        ] {
            store_local_identity_record(&LocalIdentity {
                id: format!("local-user-{email}"),
                identifier_kind: "email".to_string(),
                identifier: email.to_string(),
                identifier_normalized: email.to_string(),
                created_at: 100,
                updated_at: 100,
                state,
                metadata_json: "{}".to_string(),
            })
            .unwrap();
        }

        let module = init();
        let begin_totp_enrollment = module_fn(&module, "begin_totp_enrollment");
        let confirm_totp_enrollment = module_fn(&module, "confirm_totp_enrollment");
        let verify_local_totp = module_fn(&module, "verify_local_totp");
        let totp_status = module_fn(&module, "totp_status");
        let reset_totp = module_fn(&module, "reset_totp");

        for email in ["disabled@example.com", "locked@example.com"] {
            let begin_error = result_err_string(
                begin_totp_enrollment(&[Value::String(email.to_string())]).unwrap(),
            );
            assert!(begin_error.contains("cannot manage TOTP"));

            let confirm_error = result_err_string(
                confirm_totp_enrollment(&[
                    Value::String(email.to_string()),
                    Value::String("000000".to_string()),
                ])
                .unwrap(),
            );
            assert!(confirm_error.contains("cannot manage TOTP"));

            let verify_error = result_err_string(
                verify_local_totp(&[
                    Value::String(email.to_string()),
                    Value::String("000000".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(verify_error, "Invalid local TOTP code");

            let status_error =
                result_err_string(totp_status(&[Value::String(email.to_string())]).unwrap());
            assert!(status_error.contains("cannot manage TOTP"));

            let reset_error =
                result_err_string(reset_totp(&[Value::String(email.to_string())]).unwrap());
            assert!(reset_error.contains("cannot manage TOTP"));
        }
    }

    #[test]
    fn test_verify_local_totp_uses_generic_errors_for_unavailable_or_disabled_mfa() {
        use super::storage::{store_local_identity_record, LocalAccountState, LocalIdentity};

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        for (id, email, state, metadata_json) in [
            (
                "local-user-no-totp",
                "no-totp@example.com",
                LocalAccountState::Active,
                "{}",
            ),
            (
                "local-user-missing-secret",
                "missing-secret@example.com",
                LocalAccountState::Active,
                "{\"auth\":{\"totp\":{\"enabled\":true,\"issuer\":\"Admin\"}}}",
            ),
            (
                "local-user-malformed-totp",
                "malformed-totp@example.com",
                LocalAccountState::Active,
                "{\"auth\":{\"totp\":\"not-a-map\"}}",
            ),
            (
                "local-user-disabled-totp",
                "disabled-totp@example.com",
                LocalAccountState::Disabled,
                "{}",
            ),
            (
                "local-user-locked-totp",
                "locked-totp@example.com",
                LocalAccountState::Locked,
                "{}",
            ),
        ] {
            store_local_identity_record(&LocalIdentity {
                id: id.to_string(),
                identifier_kind: "email".to_string(),
                identifier: email.to_string(),
                identifier_normalized: email.to_string(),
                created_at: 100,
                updated_at: 100,
                state,
                metadata_json: metadata_json.to_string(),
            })
            .unwrap();
        }

        let module = init();
        let verify_local_totp = module_fn(&module, "verify_local_totp");

        for email in [
            "unknown@example.com",
            "no-totp@example.com",
            "missing-secret@example.com",
            "malformed-totp@example.com",
            "disabled-totp@example.com",
            "locked-totp@example.com",
        ] {
            let message = result_err_string(
                verify_local_totp(&[
                    Value::String(email.to_string()),
                    Value::String("000000".to_string()),
                ])
                .unwrap(),
            );
            assert_eq!(
                message, "Invalid local TOTP code",
                "unexpected error for {email}"
            );
        }
    }

    #[test]
    fn test_local_password_login_sign_in_captures_request_metadata_and_groups() {
        use super::storage::{
            store_local_credential_secret_record, store_local_identity_record, LocalAccountState,
            LocalCredentialSecret, LocalIdentity,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        store_local_identity_record(&LocalIdentity {
            id: "local-user-login".to_string(),
            identifier_kind: "email".to_string(),
            identifier: "Login@Example.COM".to_string(),
            identifier_normalized: "login@example.com".to_string(),
            created_at: 100,
            updated_at: 100,
            state: LocalAccountState::Active,
            metadata_json: "{}".to_string(),
        })
        .unwrap();
        store_local_credential_secret_record(&LocalCredentialSecret {
            local_user_id: "local-user-login".to_string(),
            password_hash: bcrypt::hash("login password", 4).unwrap(),
            password_hash_algorithm: "bcrypt".to_string(),
            password_hash_params_json: "{}".to_string(),
            password_changed_at: 101,
            must_change_password: false,
        })
        .unwrap();

        let module = init();
        let verify_local_password = module_fn(&module, "verify_local_password");
        let sign_in_session = module_fn(&module, "sign_in_session");
        let has_group = module_fn(&module, "has_group");
        let verified = result_ok_map(
            verify_local_password(&[
                Value::String("login@example.com".to_string()),
                Value::String("login password".to_string()),
            ])
            .unwrap(),
        );
        let request = request_with_cookie_and_security_headers("");

        let signed_in = sign_in_session(&[
            redirect_response("/admin", None),
            request,
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    verified.get("subject_id").cloned().unwrap(),
                ),
                ("email".to_string(), verified.get("email").cloned().unwrap()),
                (
                    "data".to_string(),
                    Value::Map(HashMap::from([(
                        "group_ids".to_string(),
                        Value::Array(vec![Value::String("admins".to_string())]),
                    )])),
                ),
            ])),
        ])
        .unwrap();

        let cookie = cookie_header_from_response(&signed_in);
        let req = request_with_cookie(&cookie);
        let session_id = get_session_id_from_request(&req).expect("session cookie should verify");
        let session = get_session_by_id(&session_id).expect("session should be persisted");
        assert_eq!(session.device_name.as_deref(), Some("Mac · Safari"));
        assert!(session.user_agent_hash.is_some());
        assert!(session.last_ip_hash.is_some());
        match has_group(&[req, Value::String("admins".to_string())]).unwrap() {
            Value::Bool(true) => {}
            other => panic!("expected local login session to carry admin group, got {other:?}"),
        }
    }

    #[test]
    fn test_has_group_checks_session_data_for_pages_and_api_requests() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let has_group = module_fn(&module, "has_group");
        let session = Session {
            id: "session-groups".to_string(),
            user_id: "local-user-groups".to_string(),
            provider: "local".to_string(),
            email: Some("groups@example.com".to_string()),
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: r#"{"group_ids":["admins","billing"],"claims":{"scope":"admin"}}"#
                .to_string(),
            csrf_token: "csrf-groups".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: Some("Test Device".to_string()),
            user_agent_hash: Some("ua-hash".to_string()),
            last_ip_hash: Some("ip-hash".to_string()),
            created_at: 100,
            expires_at: chrono::Utc::now().timestamp() + 300,
        };
        store_session(session.clone());
        let cookie =
            build_signed_session_cookie(get_auth_config().as_ref().unwrap(), &session.id, None)
                .unwrap();
        let req = request_with_cookie(&cookie);

        match has_group(&[req.clone(), Value::String("admins".to_string())]).unwrap() {
            Value::Bool(true) => {}
            other => panic!("expected admins membership to be true, got {other:?}"),
        }
        match has_group(&[req.clone(), Value::String("operators".to_string())]).unwrap() {
            Value::Bool(false) => {}
            other => panic!("expected operators membership to be false, got {other:?}"),
        }
        match has_group(&[
            session_to_value(&session),
            Value::Array(vec![
                Value::String("operators".to_string()),
                Value::String("billing".to_string()),
            ]),
        ])
        .unwrap()
        {
            Value::Bool(true) => {}
            other => panic!("expected any-group membership to be true, got {other:?}"),
        }

        let arbitrary_map = HashMap::from([(
            "data".to_string(),
            Value::Map(HashMap::from([(
                "group_ids".to_string(),
                Value::Array(vec![Value::String("admins".to_string())]),
            )])),
        )]);
        let err = has_group(&[
            Value::Map(arbitrary_map),
            Value::String("admins".to_string()),
        ])
        .unwrap_err();
        assert!(err.to_string().contains("auth session map"));
    }

    #[test]
    fn test_bootstrap_local_user_rejects_duplicate_and_invalid_inputs() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
        bootstrap_local_user(&[
            Value::String("admin@example.com".to_string()),
            Value::String("temporary bootstrap password".to_string()),
        ])
        .unwrap();

        let duplicate = result_err_string(
            bootstrap_local_user(&[
                Value::String(" Admin@Example.com ".to_string()),
                Value::String("temporary bootstrap password".to_string()),
            ])
            .unwrap(),
        );
        assert!(
            duplicate.contains("already exists"),
            "unexpected duplicate error: {duplicate}"
        );

        let invalid_email = result_err_string(
            bootstrap_local_user(&[
                Value::String("not-an-email".to_string()),
                Value::String("temporary bootstrap password".to_string()),
            ])
            .unwrap(),
        );
        assert!(
            invalid_email.contains("must contain @"),
            "unexpected invalid-email error: {invalid_email}"
        );

        let empty_password = result_err_string(
            bootstrap_local_user(&[
                Value::String("new@example.com".to_string()),
                Value::String("   ".to_string()),
            ])
            .unwrap(),
        );
        assert!(
            empty_password.contains("password must not be empty"),
            "unexpected empty-password error: {empty_password}"
        );
    }

    #[test]
    fn test_bootstrap_local_user_requires_auth_initialization() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let bootstrap_local_user = module_fn(&module, "bootstrap_local_user");
        let err = bootstrap_local_user(&[
            Value::String("admin@example.com".to_string()),
            Value::String("password".to_string()),
        ])
        .expect_err("bootstrap_local_user should require initialized auth");
        assert!(
            err.to_string().contains("enable_auth"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_verify_local_password_requires_auth_initialization() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let verify_local_password = module_fn(&module, "verify_local_password");
        let err = verify_local_password(&[
            Value::String("admin@example.com".to_string()),
            Value::String("password".to_string()),
        ])
        .expect_err("verify_local_password should require initialized auth");
        assert!(
            err.to_string().contains("enable_auth"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_auth_storage_contract_postgres_round_trip_all_record_types() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        let Some(store) = auth_test_postgres_store() else {
            eprintln!("[auth-test] skipping Postgres auth contract test — set NTNT_AUTH_TEST_POSTGRES_URL");
            return;
        };
        run_auth_storage_contract_round_trip(store, "postgres");
    }

    #[test]
    fn test_auth_storage_contract_redis_round_trip_all_record_types() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        let Some(store) = auth_test_redis_store() else {
            eprintln!(
                "[auth-test] skipping Redis auth contract test — set NTNT_AUTH_TEST_REDIS_URL"
            );
            return;
        };
        run_auth_storage_contract_round_trip(store, "redis");
    }

    #[test]
    fn test_redis_password_reset_consume_is_one_time_under_concurrency() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        let Some(store) = auth_test_redis_store() else {
            eprintln!(
                "[auth-test] skipping Redis password reset concurrency test — set NTNT_AUTH_TEST_REDIS_URL"
            );
            return;
        };
        reset_auth_test_state();
        init_test_auth(store);

        let now = chrono::Utc::now().timestamp();
        let identity = LocalIdentity {
            id: format!("local-user-redis-concurrent-{now}"),
            identifier_kind: "email".to_string(),
            identifier: format!("redis-concurrent-{now}@example.com"),
            identifier_normalized: format!("redis-concurrent-{now}@example.com"),
            created_at: now,
            updated_at: now,
            state: LocalAccountState::Active,
            metadata_json: "{}".to_string(),
        };
        let credential = LocalCredentialSecret {
            local_user_id: identity.id.clone(),
            password_hash: "hash-before-concurrent-reset".to_string(),
            password_hash_algorithm: "bcrypt".to_string(),
            password_hash_params_json: "{}".to_string(),
            password_changed_at: now,
            must_change_password: false,
        };
        store_local_identity_and_credential_record(&identity, &credential).unwrap();
        let reset_token = LocalPasswordResetToken {
            selector: format!("redis-concurrent-selector-{now}"),
            local_user_id: identity.id.clone(),
            token_hash: format!("redis-concurrent-token-hash-{now}"),
            created_at: now,
            expires_at: now + 3600,
        };
        store_local_password_reset_token_record(&reset_token).unwrap();

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for index in 0..2 {
            let barrier = barrier.clone();
            let selector = reset_token.selector.clone();
            let token_hash = reset_token.token_hash.clone();
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                consume_local_password_reset_token_and_store_credential_record(
                    &selector,
                    &token_hash,
                    now,
                    |local_user_id| {
                        Ok(LocalCredentialSecret {
                            local_user_id: local_user_id.to_string(),
                            password_hash: format!("hash-after-concurrent-reset-{index}"),
                            password_hash_algorithm: "bcrypt".to_string(),
                            password_hash_params_json: "{}".to_string(),
                            password_changed_at: now + index,
                            must_change_password: false,
                        })
                    },
                )
            }));
        }

        let successes = handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .expect("redis consume thread should not panic")
            })
            .map(|result| result.expect("redis consume should not return storage error"))
            .filter(|result| result.is_some())
            .count();
        assert_eq!(
            successes, 1,
            "exactly one concurrent Redis password reset consume should succeed"
        );
        assert!(
            consume_local_password_reset_token_and_store_credential_record(
                &reset_token.selector,
                &reset_token.token_hash,
                now,
                |local_user_id| {
                    Ok(LocalCredentialSecret {
                        local_user_id: local_user_id.to_string(),
                        password_hash: "unused-replay-hash".to_string(),
                        password_hash_algorithm: "bcrypt".to_string(),
                        password_hash_params_json: "{}".to_string(),
                        password_changed_at: now + 10,
                        must_change_password: false,
                    })
                },
            )
            .unwrap()
            .is_none(),
            "Redis password reset token should be gone after the successful consume"
        );
    }

    #[test]
    fn test_auth_storage_contract_sqlite_round_trip_all_record_types() {
        use super::storage::{
            cleanup_expired_auth_challenge_records, cleanup_expired_exchange_token_records,
            cleanup_expired_oauth_state_records, delete_all_session_records_for_user,
            delete_session_record, extend_session_record_expiry, get_auth_challenge_record,
            get_refreshable_session_record, get_session_record, list_session_records_for_user,
            migrate_session_record, store_auth_challenge_record, store_exchange_token_record,
            store_oauth_state_record, store_session_record, update_session_record_data,
            update_session_record_tokens,
        };

        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Sqlite(":memory:".to_string()));

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-sqlite-1".to_string(),
            user_id: "user-sqlite".to_string(),
            provider: "local".to_string(),
            email: Some("sqlite@example.com".to_string()),
            name: Some("SQLite User".to_string()),
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-sqlite-1".to_string(),
            access_token: Some("access-sqlite-1".to_string()),
            refresh_token: Some("refresh-sqlite-1".to_string()),
            token_expires_at: Some(now + 60),
            device_name: Some("SQLite Mac".to_string()),
            user_agent_hash: Some("ua-sqlite-1".to_string()),
            last_ip_hash: Some("ip-sqlite-1".to_string()),
            created_at: now,
            expires_at: now + 300,
        };
        store_session_record(&session).expect("sqlite session store should succeed");
        assert_eq!(
            get_session_record(&session.id)
                .expect("sqlite session lookup should succeed")
                .expect("sqlite session should exist")
                .id,
            session.id
        );
        update_session_record_data(&session.id, r#"{"role":"admin"}"#)
            .expect("sqlite session data update should succeed");
        update_session_record_tokens(
            &session.id,
            &TokenResponse {
                access_token: "access-sqlite-2".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: Some(120),
                refresh_token: Some("refresh-sqlite-2".to_string()),
                id_token: None,
                scope: None,
            },
            now,
        )
        .expect("sqlite session token update should succeed");
        extend_session_record_expiry(&session.id, now + 600)
            .expect("sqlite session expiry extension should succeed");
        let updated_session = get_session_record(&session.id)
            .expect("sqlite updated session lookup should succeed")
            .expect("sqlite updated session should exist");
        assert_eq!(updated_session.data_json, r#"{"role":"admin"}"#);
        assert_eq!(
            updated_session.access_token.as_deref(),
            Some("access-sqlite-2")
        );
        assert_eq!(
            updated_session.refresh_token.as_deref(),
            Some("refresh-sqlite-2")
        );
        assert_eq!(updated_session.token_expires_at, Some(now + 120));
        assert_eq!(updated_session.expires_at, now + 600);

        let refreshable_session = Session {
            id: "session-sqlite-refreshable".to_string(),
            user_id: "user-sqlite".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-sqlite-refreshable".to_string(),
            access_token: Some("access-refreshable".to_string()),
            refresh_token: Some("refresh-refreshable".to_string()),
            token_expires_at: Some(now - 30),
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 30,
            expires_at: now - 5,
        };
        store_session_record(&refreshable_session)
            .expect("sqlite refreshable session store should succeed");
        assert_eq!(
            get_refreshable_session_record(&refreshable_session.id, 3600)
                .expect("sqlite refreshable lookup should succeed")
                .expect("sqlite refreshable session should exist")
                .id,
            refreshable_session.id
        );

        let rotated_session = Session {
            id: "session-sqlite-rotated".to_string(),
            csrf_token: "csrf-sqlite-rotated".to_string(),
            ..updated_session.clone()
        };
        migrate_session_record(&session.id, &rotated_session)
            .expect("sqlite session migration should succeed");
        assert!(get_session_record(&session.id)
            .expect("sqlite old session lookup should succeed")
            .is_none());
        assert_eq!(
            get_session_record(&rotated_session.id)
                .expect("sqlite rotated session lookup should succeed")
                .expect("sqlite rotated session should exist")
                .csrf_token,
            "csrf-sqlite-rotated"
        );
        let listed_sessions =
            list_session_records_for_user("user-sqlite", Some(&rotated_session.id), now)
                .expect("sqlite session listing should succeed");
        assert_eq!(listed_sessions.len(), 1);
        assert_eq!(
            listed_sessions
                .iter()
                .filter(|session| session.is_current)
                .count(),
            1
        );
        assert!(listed_sessions
            .iter()
            .any(|session| session.id == rotated_session.id && session.is_current));
        assert_eq!(
            delete_all_session_records_for_user("user-sqlite", Some(&rotated_session.id))
                .expect("sqlite delete-all sessions should succeed"),
            1
        );
        let remaining_sessions =
            list_session_records_for_user("user-sqlite", Some(&rotated_session.id), now)
                .expect("sqlite remaining session listing should succeed");
        assert_eq!(remaining_sessions.len(), 1);
        assert_eq!(remaining_sessions[0].id, rotated_session.id);
        delete_session_record(&rotated_session.id).expect("sqlite session delete should succeed");
        assert!(get_session_record(&rotated_session.id)
            .expect("sqlite deleted session lookup should succeed")
            .is_none());

        let oauth_state = OAuthState {
            state: "oauth-sqlite-active".to_string(),
            nonce: Some("nonce-sqlite".to_string()),
            pkce_verifier: Some("pkce-sqlite".to_string()),
            provider: "github".to_string(),
            redirect_url: "/auth/callback".to_string(),
            remember_me: true,
            device_name: Some("SQLite Browser".to_string()),
            user_agent_hash: Some("ua-sqlite-oauth".to_string()),
            last_ip_hash: Some("ip-sqlite-oauth".to_string()),
            created_at: now,
        };
        store_oauth_state_record(&oauth_state).expect("sqlite oauth state store should succeed");
        assert_eq!(
            super::storage::consume_oauth_state_record(&oauth_state.state)
                .expect("sqlite oauth state consume should succeed")
                .expect("sqlite oauth state should exist")
                .state,
            oauth_state.state
        );
        let expired_oauth_state = OAuthState {
            state: "oauth-sqlite-expired".to_string(),
            nonce: None,
            pkce_verifier: None,
            provider: "github".to_string(),
            redirect_url: "/auth/callback".to_string(),
            remember_me: false,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 700,
        };
        store_oauth_state_record(&expired_oauth_state)
            .expect("expired sqlite oauth state store should succeed");
        assert_eq!(
            cleanup_expired_oauth_state_records(now - 600)
                .expect("sqlite oauth cleanup should succeed"),
            1
        );

        store_exchange_token_record("exchange-sqlite-active", &session.id)
            .expect("sqlite exchange token store should succeed");
        assert_eq!(
            super::storage::consume_exchange_token_record("exchange-sqlite-active")
                .expect("sqlite exchange token consume should succeed")
                .as_deref(),
            Some(session.id.as_str())
        );
        let stale_exchange_created_at = now - EXCHANGE_TOKEN_TTL - 1;
        SQLITE_CONN
            .lock()
            .unwrap()
            .as_ref()
            .expect("sqlite connection should be initialized")
            .execute(
                "INSERT INTO auth_exchange_tokens (token, session_id, created_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![
                    "exchange-sqlite-expired",
                    "session-sqlite-stale",
                    stale_exchange_created_at,
                ],
            )
            .expect("sqlite expired exchange token insert should succeed");
        assert_eq!(
            cleanup_expired_exchange_token_records(now)
                .expect("sqlite exchange cleanup should succeed"),
            1
        );

        let challenge = AuthChallenge {
            id: "challenge-sqlite-active".to_string(),
            subject_id: "user-sqlite".to_string(),
            provider: "local".to_string(),
            kind: "mfa_pending".to_string(),
            csrf_token: "test-csrf".to_string(),
            data_json: "{}".to_string(),
            created_at: now,
            expires_at: now + 60,
        };
        store_auth_challenge_record(&challenge)
            .expect("sqlite auth challenge store should succeed");
        assert_eq!(
            get_auth_challenge_record(&challenge.id)
                .expect("sqlite auth challenge lookup should succeed")
                .expect("sqlite auth challenge should exist")
                .id,
            challenge.id
        );
        assert_eq!(
            super::storage::consume_auth_challenge_record(&challenge.id)
                .expect("sqlite auth challenge consume should succeed")
                .expect("sqlite auth challenge should exist")
                .id,
            challenge.id
        );
        let expired_challenge = AuthChallenge {
            id: "challenge-sqlite-expired".to_string(),
            subject_id: "user-sqlite".to_string(),
            provider: "local".to_string(),
            kind: "password_reset".to_string(),
            csrf_token: "test-csrf".to_string(),
            data_json: "{}".to_string(),
            created_at: now - 120,
            expires_at: now - 60,
        };
        store_auth_challenge_sqlite(&expired_challenge)
            .expect("sqlite expired challenge insert should succeed");
        assert_eq!(
            cleanup_expired_auth_challenge_records(now)
                .expect("sqlite auth challenge cleanup should succeed"),
            1
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
            csrf_token: "test-csrf".to_string(),
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
            csrf_token: "test-csrf".to_string(),
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
            csrf_token: "test-csrf".to_string(),
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
                csrf_token: "test-csrf".to_string(),
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
            csrf_token: "test-csrf".to_string(),
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

    fn github_test_provider_value() -> Value {
        Value::Map(HashMap::from([
            ("name".to_string(), Value::String("github".to_string())),
            (
                "client_id".to_string(),
                Value::String("test-client".to_string()),
            ),
            (
                "client_secret".to_string(),
                Value::String("test-secret".to_string()),
            ),
            (
                "authorize_url".to_string(),
                Value::String("https://github.com/login/oauth/authorize".to_string()),
            ),
            (
                "token_url".to_string(),
                Value::String("https://github.com/login/oauth/access_token".to_string()),
            ),
            (
                "userinfo_url".to_string(),
                Value::String("https://api.github.com/user".to_string()),
            ),
        ]))
    }

    #[test]
    fn test_enable_auth_accepts_preset_string() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        enable_auth(&[
            Value::Array(vec![provider]),
            Value::String("admin".to_string()),
        ])
        .expect("enable_auth should accept preset string");

        let config = get_auth_config().expect("auth config should be initialized");
        assert_eq!(config.auth_preset.as_deref(), Some("admin"));
        assert_eq!(config.session_ttl, 3600);
        assert_eq!(config.max_session_ttl, Some(86400));
        assert_eq!(config.cookie_same_site, "Strict");
    }

    #[test]
    fn test_enable_auth_accepts_preset_plus_overrides() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        enable_auth(&[
            Value::Array(vec![provider]),
            Value::String("consumer".to_string()),
            Value::Map(HashMap::from([
                ("session_ttl".to_string(), Value::Int(7200)),
                ("cookie_secure".to_string(), Value::Bool(false)),
                (
                    "cookie_same_site".to_string(),
                    Value::String("Strict".to_string()),
                ),
            ])),
        ])
        .expect("enable_auth should accept preset plus overrides");

        let config = get_auth_config().expect("auth config should be initialized");
        assert_eq!(config.auth_preset.as_deref(), Some("consumer"));
        assert_eq!(config.session_ttl, 7200);
        assert!(!config.cookie_secure);
        assert_eq!(config.cookie_same_site, "Strict");
        assert!(config.sliding_sessions);
    }

    #[test]
    fn test_enable_auth_rejects_unknown_preset() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        let err = enable_auth(&[
            Value::Array(vec![provider]),
            Value::String("wizard-mode".to_string()),
        ])
        .expect_err("enable_auth should reject unknown preset");
        assert!(format!("{}", err).contains("unknown preset"));
    }

    #[test]
    fn test_enable_auth_accepts_session_lifecycle_options() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        enable_auth(&[
            Value::Array(vec![provider]),
            Value::Map(HashMap::from([
                ("sliding_sessions".to_string(), Value::Bool(true)),
                ("refresh_throttle".to_string(), Value::Int(120)),
                ("max_session_ttl".to_string(), Value::Int(3600)),
            ])),
        ])
        .expect("enable_auth should accept session lifecycle options");

        let config = get_auth_config().expect("auth config should be initialized");
        assert!(config.sliding_sessions);
        assert_eq!(config.refresh_throttle, 120);
        assert_eq!(config.max_session_ttl, Some(3600));
    }

    #[test]
    fn test_enable_auth_rejects_invalid_session_lifecycle_option_types() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        let err = enable_auth(&[
            Value::Array(vec![provider]),
            Value::Map(HashMap::from([(
                "sliding_sessions".to_string(),
                Value::String("yes".to_string()),
            )])),
        ])
        .expect_err("enable_auth should reject invalid sliding_sessions type");
        assert!(format!("{}", err).contains("option \"sliding_sessions\" must be a bool"));
    }

    #[test]
    fn test_enable_auth_rejects_negative_refresh_throttle() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        let err = enable_auth(&[
            Value::Array(vec![provider]),
            Value::Map(HashMap::from([(
                "refresh_throttle".to_string(),
                Value::Int(-1),
            )])),
        ])
        .expect_err("enable_auth should reject negative refresh_throttle");
        assert!(format!("{}", err).contains("option \"refresh_throttle\" must be >= 0"));
    }

    #[test]
    fn test_enable_auth_rejects_non_positive_max_session_ttl() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        let err = enable_auth(&[
            Value::Array(vec![provider]),
            Value::Map(HashMap::from([(
                "max_session_ttl".to_string(),
                Value::Int(0),
            )])),
        ])
        .expect_err("enable_auth should reject non-positive max_session_ttl");
        assert!(format!("{}", err).contains("option \"max_session_ttl\" must be > 0"));
    }

    #[test]
    fn test_enable_auth_accepts_none_for_max_session_ttl() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();

        let module = init();
        let enable_auth = module_fn(&module, "enable_auth");
        let provider = github_test_provider_value();

        enable_auth(&[
            Value::Array(vec![provider]),
            Value::Map(HashMap::from([(
                "max_session_ttl".to_string(),
                Value::EnumValue {
                    enum_name: "Option".to_string(),
                    variant: "None".to_string(),
                    values: vec![],
                },
            )])),
        ])
        .expect("enable_auth should accept None for max_session_ttl");

        let config = get_auth_config().expect("auth config should be initialized");
        assert_eq!(config.max_session_ttl, None);
    }

    #[test]
    fn test_sign_in_session_caps_initial_expiry_to_max_session_ttl() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            session_ttl: 86400,
            max_session_ttl: Some(1800),
            ..AuthConfig::default()
        });

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let current_session = module_fn(&module, "current_session");
        let before = chrono::Utc::now().timestamp();

        let signed_in = sign_in_session(&[
            redirect_response("/admin", None),
            request_with_cookie(""),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap();
        let cookie = cookie_header_from_response(&signed_in);
        let req = request_with_cookie(&cookie);
        let after = chrono::Utc::now().timestamp();

        let session = current_session(&[req]).unwrap();
        match session {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Some");
                let session_map = match values.first() {
                    Some(Value::Map(map)) => map,
                    other => panic!("expected session map, got {:?}", other),
                };
                match session_map.get("expires_at") {
                    Some(Value::Int(expires_at)) => {
                        assert!(*expires_at >= before + 1800);
                        assert!(*expires_at <= after + 1800);
                    }
                    other => panic!("expected expires_at int, got {:?}", other),
                }
            }
            other => panic!("expected Some(session), got {:?}", other),
        }
    }

    #[test]
    fn test_get_session_by_id_slides_active_session_expiry() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: true,
            refresh_throttle: 300,
            session_ttl: 3600,
            max_session_ttl: Some(7200),
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-sliding-active".to_string(),
            user_id: "user-sliding".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-sliding-active".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 600,
            expires_at: now + 60,
        };
        SESSION_STORE.lock().unwrap().set_session(session.clone());

        let fetched = get_session_by_id(&session.id).expect("session should exist");
        assert!(fetched.expires_at > session.expires_at);
        assert_eq!(
            SESSION_STORE
                .lock()
                .unwrap()
                .get_session(&session.id)
                .expect("session should remain stored")
                .expires_at,
            fetched.expires_at
        );
    }

    #[test]
    fn test_get_session_by_id_respects_max_session_ttl_when_sliding() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: true,
            refresh_throttle: 300,
            session_ttl: 3600,
            max_session_ttl: Some(1800),
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-sliding-capped".to_string(),
            user_id: "user-sliding-cap".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-sliding-capped".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 1200,
            expires_at: now + 30,
        };
        SESSION_STORE.lock().unwrap().set_session(session.clone());

        let fetched = get_session_by_id(&session.id).expect("session should exist");
        assert_eq!(fetched.expires_at, session.created_at + 1800);
    }

    #[test]
    fn test_get_session_by_id_clamps_existing_session_to_absolute_cap() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: false,
            max_session_ttl: Some(1800),
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-cap-clamp".to_string(),
            user_id: "user-cap-clamp".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-cap-clamp".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 1200,
            expires_at: now + 7200,
        };
        SESSION_STORE.lock().unwrap().set_session(session.clone());

        let fetched = get_session_by_id(&session.id).expect("session should exist");
        assert_eq!(fetched.expires_at, session.created_at + 1800);
    }

    #[test]
    fn test_get_session_by_id_invalidates_session_past_absolute_cap() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: false,
            max_session_ttl: Some(1800),
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-cap-expired".to_string(),
            user_id: "user-cap-expired".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-cap-expired".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 3600,
            expires_at: now + 60,
        };
        SESSION_STORE.lock().unwrap().set_session(session.clone());

        assert!(get_session_by_id(&session.id).is_none());
        assert!(SESSION_STORE
            .lock()
            .unwrap()
            .get_session(&session.id)
            .is_none());
    }

    #[test]
    fn test_complete_auth_challenge_caps_initial_expiry_to_max_session_ttl() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            max_session_ttl: Some(1800),
            session_ttl: 86400,
            ..AuthConfig::default()
        });

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let complete_auth_challenge = module_fn(&module, "complete_auth_challenge");
        let current_session = module_fn(&module, "current_session");
        let before = chrono::Utc::now().timestamp();

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
            req,
            Value::Map(HashMap::new()),
        ])
        .unwrap();
        let session_cookie = cookie_headers_from_response(&completed)
            .into_iter()
            .find(|cookie| cookie.starts_with("ntnt_session="))
            .expect("missing session cookie");
        let session_req = request_with_cookie(&session_cookie);
        let after = chrono::Utc::now().timestamp();

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
                match session_map.get("expires_at") {
                    Some(Value::Int(expires_at)) => {
                        assert!(*expires_at >= before + 1800);
                        assert!(*expires_at <= after + 1800);
                    }
                    other => panic!("expected expires_at int, got {:?}", other),
                }
            }
            other => panic!("expected Some(session), got {:?}", other),
        }
    }

    #[test]
    fn test_get_session_by_id_skips_sliding_refresh_when_throttle_not_met() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: true,
            refresh_throttle: 300,
            session_ttl: 3600,
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-sliding-throttle".to_string(),
            user_id: "user-sliding-throttle".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-sliding-throttle".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 60,
            expires_at: now + 1200,
        };
        SESSION_STORE.lock().unwrap().set_session(session.clone());

        let fetched = get_session_by_id(&session.id).expect("session should exist");
        assert_eq!(fetched.expires_at, session.expires_at);
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
            request_with_cookie(""),
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
    fn test_sign_in_session_captures_request_metadata() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let request = request_with_cookie_and_security_headers("");

        let signed_in = sign_in_session(&[
            redirect_response("/admin", None),
            request.clone(),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap();

        let cookie = cookie_header_from_response(&signed_in);
        let session_id = get_session_id_from_request(&request_with_cookie(&cookie))
            .expect("session cookie should verify");
        let session = get_session_by_id(&session_id).expect("session should be persisted");
        assert_eq!(session.device_name.as_deref(), Some("Mac · Safari"));
        assert!(session.user_agent_hash.is_some());
        assert!(session.last_ip_hash.is_some());
    }

    #[test]
    fn test_sign_in_session_rotates_existing_session() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");

        let first = sign_in_session(&[
            redirect_response("/admin", None),
            request_with_cookie(""),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("first-user".to_string()),
            )])),
        ])
        .unwrap();
        let old_cookie = cookie_header_from_response(&first);
        let old_id = get_session_id_from_request(&request_with_cookie(&old_cookie))
            .expect("old session cookie should verify");

        let second_req = request_with_cookie_and_security_headers(&old_cookie);
        let second = sign_in_session(&[
            redirect_response("/admin", None),
            second_req,
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("second-user".to_string()),
            )])),
        ])
        .unwrap();
        let new_cookie = cookie_header_from_response(&second);
        let new_id = get_session_id_from_request(&request_with_cookie(&new_cookie))
            .expect("new session cookie should verify");

        assert_ne!(old_id, new_id);
        assert!(get_session_by_id(&old_id).is_none());
        let new_session = get_session_by_id(&new_id).expect("new session should be persisted");
        assert_eq!(new_session.user_id, "local:second-user");
        assert_eq!(new_session.device_name.as_deref(), Some("Mac · Safari"));
    }

    #[test]
    fn test_sign_in_session_preserves_existing_data_only_for_same_user() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");

        let first = sign_in_session(&[
            redirect_response("/admin", None),
            request_with_cookie(""),
            Value::Map(HashMap::from([
                (
                    "subject_id".to_string(),
                    Value::String("same-user".to_string()),
                ),
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
        let first_cookie = cookie_header_from_response(&first);

        let same_user = sign_in_session(&[
            redirect_response("/admin", None),
            request_with_cookie(&first_cookie),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("same-user".to_string()),
            )])),
        ])
        .unwrap();
        let same_user_cookie = cookie_header_from_response(&same_user);
        let same_user_id = get_session_id_from_request(&request_with_cookie(&same_user_cookie))
            .expect("same-user session cookie should verify");
        let same_user_session = get_session_by_id(&same_user_id).expect("session should exist");
        assert_eq!(same_user_session.data_json, r#"{"role":"admin"}"#);

        let different_user = sign_in_session(&[
            redirect_response("/admin", None),
            request_with_cookie(&same_user_cookie),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("different-user".to_string()),
            )])),
        ])
        .unwrap();
        let different_user_cookie = cookie_header_from_response(&different_user);
        let different_user_id =
            get_session_id_from_request(&request_with_cookie(&different_user_cookie))
                .expect("different-user session cookie should verify");
        let different_user_session =
            get_session_by_id(&different_user_id).expect("session should exist");
        assert_eq!(different_user_session.user_id, "local:different-user");
        assert_eq!(different_user_session.data_json, "{}");
    }

    #[test]
    fn test_sign_in_session_rejects_invalid_request_argument() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let sign_in_session = module_fn(&module, "sign_in_session");
        let err = sign_in_session(&[
            redirect_response("/admin", None),
            Value::String("not-a-request".to_string()),
            Value::Map(HashMap::from([(
                "subject_id".to_string(),
                Value::String("user-123".to_string()),
            )])),
        ])
        .unwrap_err();

        assert!(format!("{}", err)
            .contains("[auth] sign_in_session() request must be an HTTP request map"));
    }

    #[test]
    fn test_complete_auth_challenge_rejects_invalid_request_argument() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let complete_auth_challenge = module_fn(&module, "complete_auth_challenge");
        let err = complete_auth_challenge(&[
            redirect_response("/admin", None),
            Value::String("not-a-request".to_string()),
        ])
        .unwrap_err();

        assert!(format!("{}", err)
            .contains("[auth] complete_auth_challenge() request must be an HTTP request map"));
    }

    #[test]
    fn test_complete_auth_challenge_captures_request_metadata() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let module = init();
        let begin_auth_challenge = module_fn(&module, "begin_auth_challenge");
        let complete_auth_challenge = module_fn(&module, "complete_auth_challenge");

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
        let req = request_with_cookie_and_security_headers(&challenge_cookie);

        let completed = complete_auth_challenge(&[
            redirect_response("/admin", None),
            req,
            Value::Map(HashMap::new()),
        ])
        .unwrap();
        let session_cookie = cookie_headers_from_response(&completed)
            .into_iter()
            .find(|cookie| cookie.starts_with("ntnt_session="))
            .expect("missing session cookie");
        let session_id = get_session_id_from_request(&request_with_cookie(&session_cookie))
            .expect("session cookie should verify");
        let session = get_session_by_id(&session_id).expect("session should be persisted");
        assert_eq!(session.device_name.as_deref(), Some("Mac · Safari"));
        assert!(session.user_agent_hash.is_some());
        assert!(session.last_ip_hash.is_some());
    }

    #[test]
    fn test_user_sessions_exposes_safe_device_name_metadata() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-device-name".to_string(),
            user_id: "local:user-123".to_string(),
            provider: "local".to_string(),
            email: Some("alice@example.com".to_string()),
            name: Some("Alice".to_string()),
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-device-name".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: Some("Mac · Safari".to_string()),
            user_agent_hash: Some("ua-hash-must-stay-private".to_string()),
            last_ip_hash: Some("ip-hash-must-stay-private".to_string()),
            created_at: now,
            expires_at: now + 300,
        };
        store_session_record(&session).expect("session store should succeed");

        let config = get_auth_config().expect("auth config should be initialized");
        let cookie = build_signed_session_cookie(&config, &session.id, None)
            .expect("session cookie should build");
        let req = request_with_cookie(cookie.split(';').next().unwrap());

        let module = init();
        let user_sessions = module_fn(&module, "user_sessions");
        let listed = user_sessions(&[req]).expect("user_sessions should run");

        let sessions = match listed {
            Value::EnumValue {
                enum_name,
                variant,
                values,
            } => {
                assert_eq!(enum_name, "Result");
                assert_eq!(variant, "Ok");
                match values.into_iter().next() {
                    Some(Value::Array(sessions)) => sessions,
                    other => panic!("expected Ok(Array), got {:?}", other),
                }
            }
            other => panic!("expected Result::Ok, got {:?}", other),
        };
        assert_eq!(sessions.len(), 1);
        let session_info = match sessions.first() {
            Some(Value::Map(map)) => map,
            other => panic!("expected session info map, got {:?}", other),
        };
        match session_info.get("device_name") {
            Some(Value::String(device_name)) => assert_eq!(device_name, "Mac · Safari"),
            other => panic!("expected public device_name string, got {:?}", other),
        }
        assert!(
            !session_info.contains_key("user_agent_hash"),
            "user_sessions() must not expose raw user-agent hash"
        );
        assert!(
            !session_info.contains_key("last_ip_hash"),
            "user_sessions() must not expose raw IP hash"
        );
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
            request_with_cookie(""),
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
            request_with_cookie(""),
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
            request_with_cookie(""),
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
            request_with_cookie(""),
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
            request_with_cookie(""),
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
            request_with_cookie(""),
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
            request_with_cookie(""),
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
    fn test_refresh_access_token_does_not_force_old_refresh_token_into_response() {
        let parsed = serde_json::json!({
            "access_token": "new-access",
            "token_type": "Bearer",
            "expires_in": 3600
        });

        let tokens = TokenResponse {
            access_token: parsed["access_token"].as_str().unwrap().to_string(),
            token_type: parsed["token_type"].as_str().unwrap().to_string(),
            expires_in: parsed["expires_in"].as_i64(),
            refresh_token: parsed
                .get("refresh_token")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            id_token: None,
            scope: None,
        };

        assert_eq!(tokens.refresh_token, None);
    }

    #[test]
    fn test_update_session_record_tokens_preserves_refresh_token_when_provider_omits_one() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_test_auth(SessionStore::Memory);

        let now = chrono::Utc::now().timestamp();
        let session = Session {
            id: "session-refresh-preserve".to_string(),
            user_id: "user-refresh-preserve".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-refresh-preserve".to_string(),
            access_token: Some("access-old".to_string()),
            refresh_token: Some("refresh-old".to_string()),
            token_expires_at: Some(now + 60),
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now,
            expires_at: now + 600,
        };
        store_session_record(&session).expect("session should store");

        update_session_record_tokens(
            &session.id,
            &TokenResponse {
                access_token: "access-new".to_string(),
                token_type: "Bearer".to_string(),
                expires_in: Some(120),
                refresh_token: None,
                id_token: None,
                scope: None,
            },
            now,
        )
        .expect("token update should succeed");

        let updated = get_session_record(&session.id)
            .expect("lookup should succeed")
            .expect("session should exist");
        assert_eq!(updated.access_token.as_deref(), Some("access-new"));
        assert_eq!(updated.refresh_token.as_deref(), Some("refresh-old"));
    }

    #[test]
    fn test_handle_auth_health_reports_safe_diagnostics_in_dev() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        let previous_env = std::env::var("NTNT_ENV").ok();
        let previous_site = std::env::var("SITE_URL").ok();
        unsafe {
            std::env::remove_var("SITE_URL");
            std::env::set_var("NTNT_ENV", "development");
        }

        init_auth(AuthConfig {
            providers: vec![ProviderConfig {
                name: "google".to_string(),
                client_id: "client-id".to_string(),
                client_secret: "super-secret".to_string(),
                authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
                token_url: "https://oauth2.googleapis.com/token".to_string(),
                userinfo_url: "https://openidconnect.googleapis.com/v1/userinfo".to_string(),
                supports_oidc: true,
                use_pkce: true,
                ..ProviderConfig::default()
            }],
            protected_paths: vec!["/admin/*".to_string()],
            store_tokens: true,
            session_secret: "dev-secret-value".to_string(),
            ..AuthConfig::default()
        });

        let req = Value::Map(HashMap::from([(
            "headers".to_string(),
            Value::Map(HashMap::from([(
                "host".to_string(),
                Value::String("example.com".to_string()),
            )])),
        )]));

        let response = handle_auth_health(&[req]).expect("health should succeed");
        let body = match response {
            Value::Map(map) => match map.get("body") {
                Some(Value::String(body)) => body.clone(),
                other => panic!("expected body string, got {:?}", other),
            },
            other => panic!("expected response map, got {:?}", other),
        };

        assert!(body.contains("\"providers\""));
        assert!(body.contains("\"google\""));
        assert!(body.contains("\"warnings\""));
        assert!(body.contains("SITE_URL is not set"));
        assert!(!body.contains("super-secret"));
        assert!(!body.contains("dev-secret-value"));

        match previous_env {
            Some(val) => unsafe { std::env::set_var("NTNT_ENV", val) },
            None => unsafe { std::env::remove_var("NTNT_ENV") },
        }
        match previous_site {
            Some(val) => unsafe { std::env::set_var("SITE_URL", val) },
            None => unsafe { std::env::remove_var("SITE_URL") },
        }
    }

    #[test]
    fn test_handle_auth_health_disabled_in_production_without_opt_in() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        let previous_env = std::env::var("NTNT_ENV").ok();
        unsafe {
            std::env::set_var("NTNT_ENV", "production");
        }

        init_auth(AuthConfig {
            session_secret: "prod-secret-value".to_string(),
            ..AuthConfig::default()
        });

        let response = handle_auth_health(&[]).expect("health should return response");
        match response {
            Value::Map(map) => {
                assert!(matches!(map.get("status"), Some(Value::Int(404))));
            }
            other => panic!("expected response map, got {:?}", other),
        }

        match previous_env {
            Some(val) => unsafe { std::env::set_var("NTNT_ENV", val) },
            None => unsafe { std::env::remove_var("NTNT_ENV") },
        }
    }

    #[test]
    fn test_handle_auth_health_enabled_in_production_with_opt_in() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        let previous_env = std::env::var("NTNT_ENV").ok();
        unsafe {
            std::env::set_var("NTNT_ENV", "production");
        }

        init_auth(AuthConfig {
            health_endpoint: true,
            session_secret: "prod-secret-value".to_string(),
            session_store: SessionStore::Memory,
            ..AuthConfig::default()
        });

        let response = handle_auth_health(&[]).expect("health should succeed");
        let body = match response {
            Value::Map(map) => match map.get("body") {
                Some(Value::String(body)) => body.clone(),
                other => panic!("expected body string, got {:?}", other),
            },
            other => panic!("expected response map, got {:?}", other),
        };
        assert!(body.contains("Production is using in-memory session storage"));

        match previous_env {
            Some(val) => unsafe { std::env::set_var("NTNT_ENV", val) },
            None => unsafe { std::env::remove_var("NTNT_ENV") },
        }
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
    fn test_handle_auth_index_uses_custom_route_prefix_and_login_copy() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        init_auth(AuthConfig {
            providers: vec![
                ProviderConfig {
                    name: "github".to_string(),
                    ..ProviderConfig::default()
                },
                ProviderConfig {
                    name: "google".to_string(),
                    ..ProviderConfig::default()
                },
            ],
            route_prefix: "/signin".to_string(),
            login_page_title: "Welcome back".to_string(),
            login_page_heading: "Continue to dashboard".to_string(),
            login_page_copy: "Pick a provider to keep going.".to_string(),
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
        assert!(body.contains("Welcome back"));
        assert!(body.contains("Continue to dashboard"));
        assert!(body.contains("Pick a provider to keep going."));
        assert!(body.contains("/signin/github"));
        assert!(body.contains("/signin/google"));
    }

    #[test]
    fn test_handle_auth_index_returns_json_when_login_page_disabled() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        init_auth(AuthConfig {
            providers: vec![
                ProviderConfig {
                    name: "google".to_string(),
                    ..ProviderConfig::default()
                },
                ProviderConfig {
                    name: "github".to_string(),
                    ..ProviderConfig::default()
                },
            ],
            login_page_enabled: false,
            route_prefix: "/signin".to_string(),
            ..AuthConfig::default()
        });

        let response = handle_auth_index(&[]).unwrap();
        let map = match response {
            Value::Map(map) => map,
            other => panic!("expected response map, got {:?}", other),
        };
        assert!(matches!(map.get("status"), Some(Value::Int(404))));
        match map.get("body") {
            Some(Value::String(body)) => {
                assert!(body.contains("Built-in auth login page is disabled"));
                assert!(body.contains("/signin/{provider}"));
            }
            other => panic!("expected body string, got {:?}", other),
        }
    }

    #[test]
    fn test_handle_auth_health_reports_custom_prefix_and_collision_warning() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            providers: vec![ProviderConfig {
                name: "google".to_string(),
                ..ProviderConfig::default()
            }],
            route_prefix: "/signin".to_string(),
            protected_paths: vec!["/signin/*".to_string()],
            health_endpoint: true,
            session_secret: "dev-secret-value".to_string(),
            ..AuthConfig::default()
        });

        let response = handle_auth_health(&[]).expect("health should succeed");
        let body = match response {
            Value::Map(map) => match map.get("body") {
                Some(Value::String(body)) => body.clone(),
                other => panic!("expected body string, got {:?}", other),
            },
            other => panic!("expected response map, got {:?}", other),
        };

        assert!(body.contains("\"prefix\":\"/signin\""));
        assert!(body.contains("/signin/{provider}/callback"));
        assert!(body.contains("route_collision_warnings"));
        assert!(body.contains("overlaps built-in auth route prefix"));
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
    fn test_value_to_json_uses_canonical_option_and_struct_conversion() {
        let some = Value::some(Value::String("hi".to_string()));
        assert_eq!(
            value_to_json(&some),
            serde_json::Value::String("hi".to_string())
        );

        let structured = Value::Struct {
            name: "User".to_string(),
            fields: HashMap::from([(
                "email".to_string(),
                Value::String("alice@example.com".to_string()),
            )]),
        };
        assert_eq!(
            value_to_json(&structured),
            serde_json::json!({ "email": "alice@example.com" })
        );
    }

    #[test]
    fn test_get_host_and_proto_prefers_request_protocol_field() {
        let req = Value::Map(HashMap::from([
            ("protocol".to_string(), Value::String("https".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([
                    ("host".to_string(), Value::String("example.com".to_string())),
                    (
                        "x-forwarded-proto".to_string(),
                        Value::String("http".to_string()),
                    ),
                ])),
            ),
        ]));

        assert_eq!(
            get_host_and_proto(&req),
            ("example.com".to_string(), "https".to_string())
        );
    }

    #[test]
    fn test_get_host_and_proto_prefers_site_url_when_present() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        let previous = std::env::var("SITE_URL").ok();
        unsafe {
            std::env::set_var("SITE_URL", "https://canonical.example.com/app");
        }

        let req = Value::Map(HashMap::from([(
            "headers".to_string(),
            Value::Map(HashMap::from([(
                "host".to_string(),
                Value::String("attacker.example.com".to_string()),
            )])),
        )]));

        assert_eq!(
            get_host_and_proto(&req),
            ("canonical.example.com".to_string(), "https".to_string())
        );

        match previous {
            Some(val) => unsafe { std::env::set_var("SITE_URL", val) },
            None => unsafe { std::env::remove_var("SITE_URL") },
        }
    }

    #[test]
    fn test_handle_auth_protect_skips_cookie_refresh_redirect_for_api_post() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: true,
            refresh_throttle: 300,
            session_ttl: 3600,
            protected_paths: vec!["/admin".to_string()],
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE.lock().unwrap().set_session(Session {
            id: "session-api-refresh".to_string(),
            user_id: "user-api-refresh".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-api-refresh".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 600,
            expires_at: now + 60,
        });

        let config = get_auth_config().expect("auth config should be initialized");
        let cookie = build_signed_session_cookie(&config, "session-api-refresh", None)
            .expect("cookie should build");
        let req = Value::Map(HashMap::from([
            ("method".to_string(), Value::String("POST".to_string())),
            ("path".to_string(), Value::String("/admin".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([
                    ("cookie".to_string(), Value::String(cookie)),
                    (
                        "accept".to_string(),
                        Value::String("application/json".to_string()),
                    ),
                ])),
            ),
        ]));

        let response = handle_auth_protect(&[req]).expect("auth protect should succeed");
        assert!(matches!(response, Value::Unit));
    }

    #[test]
    fn test_handle_auth_protect_uses_307_for_cookie_refresh_redirect() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: true,
            refresh_throttle: 300,
            session_ttl: 3600,
            protected_paths: vec!["/admin".to_string()],
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE.lock().unwrap().set_session(Session {
            id: "session-protect-refresh".to_string(),
            user_id: "user-protect-refresh".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-protect-refresh".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 600,
            expires_at: now + 60,
        });

        let config = get_auth_config().expect("auth config should be initialized");
        let cookie = build_signed_session_cookie(&config, "session-protect-refresh", None)
            .expect("cookie should build");
        let req = Value::Map(HashMap::from([
            ("method".to_string(), Value::String("POST".to_string())),
            ("path".to_string(), Value::String("/admin".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "cookie".to_string(),
                    Value::String(cookie),
                )])),
            ),
        ]));

        let response = handle_auth_protect(&[req]).expect("auth protect should succeed");
        assert!(matches!(response, Value::Unit));
    }

    #[test]
    fn test_handle_auth_protect_preserves_query_string_on_cookie_refresh_redirect() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: true,
            refresh_throttle: 300,
            session_ttl: 3600,
            protected_paths: vec!["/admin".to_string()],
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE.lock().unwrap().set_session(Session {
            id: "session-protect-query-refresh".to_string(),
            user_id: "user-protect-query-refresh".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-protect-query-refresh".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 600,
            expires_at: now + 60,
        });

        let config = get_auth_config().expect("auth config should be initialized");
        let cookie = build_signed_session_cookie(&config, "session-protect-query-refresh", None)
            .expect("cookie should build");
        let req = Value::Map(HashMap::from([
            ("method".to_string(), Value::String("GET".to_string())),
            ("path".to_string(), Value::String("/admin".to_string())),
            (
                "query".to_string(),
                Value::String("page=2&sort=asc".to_string()),
            ),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "cookie".to_string(),
                    Value::String(cookie),
                )])),
            ),
        ]));

        let response = handle_auth_protect(&[req]).expect("auth protect should succeed");
        match response {
            Value::Map(map) => match map.get("headers") {
                Some(Value::Map(headers)) => {
                    assert!(
                        matches!(headers.get("Location"), Some(Value::String(loc)) if loc == "/admin?page=2&sort=asc")
                    );
                }
                other => panic!("expected headers map, got {:?}", other),
            },
            other => panic!("expected response map, got {:?}", other),
        }
    }

    #[test]
    fn test_handle_auth_protect_uses_307_for_cookie_refresh_redirect_on_browser_get() {
        let _guard = AUTH_TEST_MUTEX.lock().unwrap();
        reset_auth_test_state();
        init_auth(AuthConfig {
            sliding_sessions: true,
            refresh_throttle: 300,
            session_ttl: 3600,
            protected_paths: vec!["/admin".to_string()],
            ..AuthConfig::default()
        });

        let now = chrono::Utc::now().timestamp();
        SESSION_STORE.lock().unwrap().set_session(Session {
            id: "session-protect-browser-refresh".to_string(),
            user_id: "user-protect-browser-refresh".to_string(),
            provider: "local".to_string(),
            email: None,
            name: None,
            picture: None,
            raw_json: "{}".to_string(),
            data_json: "{}".to_string(),
            csrf_token: "csrf-protect-browser-refresh".to_string(),
            access_token: None,
            refresh_token: None,
            token_expires_at: None,
            device_name: None,
            user_agent_hash: None,
            last_ip_hash: None,
            created_at: now - 600,
            expires_at: now + 60,
        });

        let config = get_auth_config().expect("auth config should be initialized");
        let cookie = build_signed_session_cookie(&config, "session-protect-browser-refresh", None)
            .expect("cookie should build");
        let req = Value::Map(HashMap::from([
            ("method".to_string(), Value::String("GET".to_string())),
            ("path".to_string(), Value::String("/admin".to_string())),
            (
                "headers".to_string(),
                Value::Map(HashMap::from([(
                    "cookie".to_string(),
                    Value::String(cookie),
                )])),
            ),
        ]));

        let response = handle_auth_protect(&[req]).expect("auth protect should succeed");
        match response {
            Value::Map(map) => {
                assert!(matches!(map.get("status"), Some(Value::Int(307))));
            }
            other => panic!("expected response map, got {:?}", other),
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
