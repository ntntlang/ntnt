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
//! # Quick Start
//! ```ntnt
//! import { oauth, enable_auth, get_user } from "std/auth"
//!
//! enable_auth(oauth("google", get_env("GOOGLE_CLIENT_ID"), get_env("GOOGLE_CLIENT_SECRET")))
//!
//! fn dashboard(req) {
//!     let user = get_user(req) otherwise return redirect("/login")
//!     return html("<h1>Hello, {user.name}!</h1>")
//! }
//! ```

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use base64::Engine;
use hmac::{Hmac, Mac};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use totp_rs::{Algorithm as TotpAlgorithm, Secret, TOTP};

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
    Value::EnumValue {
        enum_name: "Option".to_string(),
        variant: "None".to_string(),
        values: vec![],
    }
}

fn make_some(value: Value) -> Value {
    Value::EnumValue {
        enum_name: "Option".to_string(),
        variant: "Some".to_string(),
        values: vec![value],
    }
}

fn make_ok(value: Value) -> Value {
    Value::EnumValue {
        enum_name: "Result".to_string(),
        variant: "Ok".to_string(),
        values: vec![value],
    }
}

fn make_err(value: Value) -> Value {
    Value::EnumValue {
        enum_name: "Result".to_string(),
        variant: "Err".to_string(),
        values: vec![value],
    }
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

// ============================================================================
// SECTION 3: Built-in Provider Registry
// ============================================================================

/// Built-in provider template (without credentials)
struct BuiltinProvider {
    name: &'static str,
    authorize_url: &'static str,
    token_url: &'static str,
    userinfo_url: &'static str,
    issuer: Option<&'static str>,
    default_scopes: &'static [&'static str],
    supports_oidc: bool,
    supports_pkce: bool,
}

static BUILTIN_PROVIDERS: &[BuiltinProvider] = &[
    BuiltinProvider {
        name: "google",
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth",
        token_url: "https://oauth2.googleapis.com/token",
        userinfo_url: "https://www.googleapis.com/oauth2/v2/userinfo",
        issuer: Some("https://accounts.google.com"),
        default_scopes: &["openid", "email", "profile"],
        supports_oidc: true,
        supports_pkce: true,
    },
    BuiltinProvider {
        name: "github",
        authorize_url: "https://github.com/login/oauth/authorize",
        token_url: "https://github.com/login/oauth/access_token",
        userinfo_url: "https://api.github.com/user",
        issuer: None,
        default_scopes: &["read:user", "user:email"],
        supports_oidc: false,
        supports_pkce: false,
    },
    BuiltinProvider {
        name: "facebook",
        authorize_url: "https://www.facebook.com/v18.0/dialog/oauth",
        token_url: "https://graph.facebook.com/v18.0/oauth/access_token",
        userinfo_url: "https://graph.facebook.com/me?fields=id,name,email,picture",
        issuer: None,
        default_scopes: &["email", "public_profile"],
        supports_oidc: false,
        supports_pkce: true,
    },
    BuiltinProvider {
        name: "microsoft",
        authorize_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
        token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
        userinfo_url: "https://graph.microsoft.com/v1.0/me",
        issuer: Some("https://login.microsoftonline.com/common/v2.0"),
        default_scopes: &["openid", "email", "profile"],
        supports_oidc: true,
        supports_pkce: true,
    },
    BuiltinProvider {
        name: "discord",
        authorize_url: "https://discord.com/api/oauth2/authorize",
        token_url: "https://discord.com/api/oauth2/token",
        userinfo_url: "https://discord.com/api/users/@me",
        issuer: None,
        default_scopes: &["identify", "email"],
        supports_oidc: false,
        supports_pkce: true,
    },
    BuiltinProvider {
        name: "twitter",
        authorize_url: "https://twitter.com/i/oauth2/authorize",
        token_url: "https://api.twitter.com/2/oauth2/token",
        userinfo_url: "https://api.twitter.com/2/users/me?user.fields=profile_image_url",
        issuer: None,
        default_scopes: &["users.read", "tweet.read"],
        supports_oidc: false,
        supports_pkce: true, // Twitter OAuth 2.0 requires PKCE
    },
    BuiltinProvider {
        name: "linkedin",
        authorize_url: "https://www.linkedin.com/oauth/v2/authorization",
        token_url: "https://www.linkedin.com/oauth/v2/accessToken",
        userinfo_url: "https://api.linkedin.com/v2/userinfo",
        issuer: Some("https://www.linkedin.com"),
        default_scopes: &["openid", "profile", "email"],
        supports_oidc: true,
        supports_pkce: false,
    },
    BuiltinProvider {
        name: "apple",
        authorize_url: "https://appleid.apple.com/auth/authorize",
        token_url: "https://appleid.apple.com/auth/token",
        userinfo_url: "", // Apple uses ID token, no userinfo endpoint
        issuer: Some("https://appleid.apple.com"),
        default_scopes: &["name", "email"],
        supports_oidc: true,
        supports_pkce: false,
    },
    BuiltinProvider {
        name: "okta",
        authorize_url: "", // Requires tenant configuration
        token_url: "",
        userinfo_url: "",
        issuer: None, // Dynamic based on tenant
        default_scopes: &["openid", "email", "profile"],
        supports_oidc: true,
        supports_pkce: true,
    },
    BuiltinProvider {
        name: "auth0",
        authorize_url: "", // Requires domain configuration
        token_url: "",
        userinfo_url: "",
        issuer: None, // Dynamic based on domain
        default_scopes: &["openid", "email", "profile"],
        supports_oidc: true,
        supports_pkce: true,
    },
];

/// Get a built-in provider by name
fn get_builtin_provider(name: &str) -> Option<&'static BuiltinProvider> {
    BUILTIN_PROVIDERS.iter().find(|p| p.name == name)
}

/// Get list of available provider names (for error messages)
fn available_providers() -> String {
    BUILTIN_PROVIDERS
        .iter()
        .filter(|p| !p.authorize_url.is_empty()) // Only fully configured providers
        .map(|p| p.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Find similar provider name for typo suggestions
fn suggest_provider(name: &str) -> Option<&'static str> {
    BUILTIN_PROVIDERS
        .iter()
        .map(|p| p.name)
        .min_by_key(|p| levenshtein_distance(name, p))
        .filter(|p| levenshtein_distance(name, p) <= 2)
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut dp = vec![vec![0; b.len() + 1]; a.len() + 1];

    for (i, item) in dp.iter_mut().enumerate().take(a.len() + 1) {
        item[0] = i;
    }
    for j in 0..=b.len() {
        dp[0][j] = j;
    }

    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            dp[i][j] = (dp[i - 1][j] + 1)
                .min(dp[i][j - 1] + 1)
                .min(dp[i - 1][j - 1] + cost);
        }
    }
    dp[a.len()][b.len()]
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
        .map_err(|e| IntentError::RuntimeError(format!("[auth] OIDC discovery failed: {}", e)))?;

    let body = response.text().map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to read discovery response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to parse discovery document: {}", e))
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
            IntentError::RuntimeError("[auth] Discovery missing issuer".to_string())
        })?,
        authorization_endpoint: get_str("authorization_endpoint").ok_or_else(|| {
            IntentError::RuntimeError("[auth] Discovery missing authorization_endpoint".to_string())
        })?,
        token_endpoint: get_str("token_endpoint").ok_or_else(|| {
            IntentError::RuntimeError("[auth] Discovery missing token_endpoint".to_string())
        })?,
        userinfo_endpoint: get_str("userinfo_endpoint"),
        jwks_uri: get_str("jwks_uri").ok_or_else(|| {
            IntentError::RuntimeError("[auth] Discovery missing jwks_uri".to_string())
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
        .map_err(|e| IntentError::RuntimeError(format!("[auth] Token exchange failed: {}", e)))?;

    let body = response.text().map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to read token response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::RuntimeError(format!(
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
        return Err(IntentError::RuntimeError(format!(
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
            IntentError::RuntimeError(format!("[auth] No access_token in response: {}", body))
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
        .map_err(|e| IntentError::RuntimeError(format!("[auth] Token refresh failed: {}", e)))?;

    let body = response.text().map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to read refresh response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to parse refresh response: {}", e))
    })?;

    if let Some(error) = json.get("error") {
        let error_desc = json
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Err(IntentError::RuntimeError(format!(
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
            IntentError::RuntimeError("[auth] No access_token in refresh response".to_string())
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
            IntentError::RuntimeError(format!("[auth] Client credentials grant failed: {}", e))
        })?;

    let body = response.text().map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to read token response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to parse token response: {}", e))
    })?;

    if let Some(error) = json.get("error") {
        let error_desc = json
            .get("error_description")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Err(IntentError::RuntimeError(format!(
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
            IntentError::RuntimeError("[auth] No access_token in response".to_string())
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
        return Err(IntentError::RuntimeError(
            "[auth] Invalid ID token format".to_string(),
        ));
    }

    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(parts[1])
        .map_err(|e| IntentError::RuntimeError(format!("[auth] ID token decode error: {}", e)))?;

    let json: serde_json::Value = serde_json::from_slice(&payload)
        .map_err(|e| IntentError::RuntimeError(format!("[auth] ID token parse error: {}", e)))?;

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
                IntentError::RuntimeError("[auth] ID token missing issuer".to_string())
            })?;

        if iss != expected_iss {
            return Err(IntentError::RuntimeError(format!(
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
        return Err(IntentError::RuntimeError(
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
                IntentError::RuntimeError("[auth] ID token missing nonce".to_string())
            })?;

        if !constant_time_compare(nonce, expected_n) {
            return Err(IntentError::RuntimeError(
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
        .ok_or_else(|| IntentError::RuntimeError("[auth] ID token missing expiry".to_string()))?;

    let now = chrono::Utc::now().timestamp();
    if now > exp {
        return Err(IntentError::RuntimeError(
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
        .map_err(|e| IntentError::RuntimeError(format!("[auth] Userinfo request failed: {}", e)))?;

    let body = response.text().map_err(|e| {
        IntentError::RuntimeError(format!("[auth] Failed to read userinfo response: {}", e))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::RuntimeError(format!(
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
            IntentError::RuntimeError(format!("[auth] Token introspection failed: {}", e))
        })?;

    let body = response.text().map_err(|e| {
        IntentError::RuntimeError(format!(
            "[auth] Failed to read introspection response: {}",
            e
        ))
    })?;

    let json: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
        IntentError::RuntimeError(format!(
            "[auth] Failed to parse introspection response: {}",
            e
        ))
    })?;

    json_to_value_map(&json)
}

fn json_to_value_map(json: &serde_json::Value) -> Result<HashMap<String, Value>> {
    match json {
        serde_json::Value::Object(obj) => {
            let mut map = HashMap::new();
            for (key, val) in obj {
                map.insert(key.clone(), json_to_value(val));
            }
            Ok(map)
        }
        _ => Err(IntentError::TypeError("Expected JSON object".to_string())),
    }
}

fn json_map_to_value_map(
    obj: &serde_json::Map<String, serde_json::Value>,
) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    for (key, val) in obj {
        map.insert(key.clone(), json_to_value(val));
    }
    map
}

fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => make_none(),
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::String(n.to_string())
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => Value::Array(arr.iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => {
            let mut map = HashMap::new();
            for (key, val) in obj {
                map.insert(key.clone(), json_to_value(val));
            }
            Value::Map(map)
        }
    }
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Unit => serde_json::Value::Null,
        Value::EnumValue { variant, .. } if variant == "None" => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::json!(*f),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
        Value::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        _ => serde_json::Value::Null,
    }
}

fn value_map_to_json_string(map: &HashMap<String, Value>) -> String {
    let obj: serde_json::Map<String, serde_json::Value> = map
        .iter()
        .map(|(k, v)| (k.clone(), value_to_json(v)))
        .collect();
    serde_json::to_string(&serde_json::Value::Object(obj)).unwrap_or_else(|_| "{}".to_string())
}

fn json_string_to_value_map(json_str: &str) -> HashMap<String, Value> {
    if let Ok(json) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Ok(map) = json_to_value_map(&json) {
            return map;
        }
    }
    HashMap::new()
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
}

impl InMemoryStore {
    fn new() -> Self {
        InMemoryStore {
            sessions: HashMap::new(),
            oauth_states: HashMap::new(),
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

/// Initialize auth with config
pub fn init_auth(config: AuthConfig) {
    let is_prod = std::env::var("NTNT_ENV")
        .map(|v| v == "production" || v == "prod")
        .unwrap_or(false);

    // SECURITY: Require secure session_secret in production
    if is_prod && config.session_secret == DEFAULT_SESSION_SECRET_SENTINEL {
        eprintln!("┌─────────────────────────────────────────────────────────────────┐");
        eprintln!("│ FATAL: Cannot use default session_secret in production!        │");
        eprintln!("│                                                                 │");
        eprintln!("│ Set a secure random secret in enable_auth():                   │");
        eprintln!("│   enable_auth([...], map {{ \"session_secret\": get_env(\"SECRET\") }}) │");
        eprintln!("│                                                                 │");
        eprintln!("│ Generate a secret: openssl rand -base64 32                      │");
        eprintln!("└─────────────────────────────────────────────────────────────────┘");
        std::process::exit(1);
    }

    // In dev mode with no explicit secret, use auto-generated random secret
    let config = if !is_prod && config.session_secret == DEFAULT_SESSION_SECRET_SENTINEL {
        eprintln!(
            "[auth] Using auto-generated session secret (sessions won't persist across restarts)"
        );
        eprintln!("       Set session_secret in enable_auth() for production.");
        let mut config = config;
        config.session_secret = dev_session_secret().to_string();
        config
    } else {
        config
    };

    // Log session storage type
    match &config.session_store {
        SessionStore::Memory => {
            eprintln!("[auth] Using in-memory session storage");
            eprintln!("       Sessions will be lost on server restart.");
            if is_prod {
                eprintln!("       WARNING: Running in production without persistent storage!");
            }
        }
        SessionStore::Sqlite(path) => {
            eprintln!("[auth] Using SQLite session storage: {}", path);
        }
        SessionStore::Postgres(_url) => {
            eprintln!("[auth] Using PostgreSQL session storage");
            // Don't log connection URL (may contain password)
        }
        SessionStore::Redis(url) => {
            let backend = if url.starts_with("valkey://") {
                "Valkey"
            } else {
                "Redis"
            };
            eprintln!("[auth] Using {} session storage", backend);
            // Don't log connection URL (may contain password)
        }
    }

    let mut auth_config = AUTH_CONFIG.lock().unwrap();
    *auth_config = Some(config);
}

/// Initialize SQLite session storage
fn init_sqlite_sessions(path: &str) -> std::result::Result<(), String> {
    let conn =
        rusqlite::Connection::open(path).map_err(|e| format!("Failed to open SQLite: {}", e))?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS auth_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            email TEXT,
            name TEXT,
            picture TEXT,
            raw_json TEXT NOT NULL,
            data_json TEXT NOT NULL,
            csrf_token TEXT NOT NULL DEFAULT '',
            access_token TEXT,
            refresh_token TEXT,
            token_expires_at INTEGER,
            created_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("Failed to create sessions table: {}", e))?;

    // Index for cleanup queries
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON auth_sessions(expires_at)",
        [],
    )
    .map_err(|e| format!("Failed to create index: {}", e))?;

    // OAuth state table for CSRF protection
    conn.execute(
        "CREATE TABLE IF NOT EXISTS auth_oauth_states (
            state TEXT PRIMARY KEY,
            nonce TEXT,
            pkce_verifier TEXT,
            provider TEXT NOT NULL,
            redirect_url TEXT NOT NULL,
            created_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| format!("Failed to create oauth_states table: {}", e))?;

    let mut sqlite_conn = SQLITE_CONN.lock().unwrap();
    *sqlite_conn = Some(conn);
    Ok(())
}

/// Initialize PostgreSQL session storage
fn init_postgres_sessions(url: &str) -> std::result::Result<(), String> {
    // Test connection and create table
    let mut client = postgres::Client::connect(url, postgres::NoTls)
        .map_err(|e| format!("Failed to connect to PostgreSQL: {}", e))?;

    client
        .execute(
            "CREATE TABLE IF NOT EXISTS auth_sessions (
            id TEXT PRIMARY KEY,
            user_id TEXT NOT NULL,
            provider TEXT NOT NULL,
            email TEXT,
            name TEXT,
            picture TEXT,
            raw_json TEXT NOT NULL,
            data_json TEXT NOT NULL,
            csrf_token TEXT NOT NULL DEFAULT '',
            access_token TEXT,
            refresh_token TEXT,
            token_expires_at BIGINT,
            created_at BIGINT NOT NULL,
            expires_at BIGINT NOT NULL
        )",
            &[],
        )
        .map_err(|e| format!("Failed to create sessions table: {}", e))?;

    // Index for cleanup queries
    client
        .execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_expires ON auth_sessions(expires_at)",
            &[],
        )
        .ok(); // Ignore if already exists

    // OAuth state table for CSRF protection
    client
        .execute(
            "CREATE TABLE IF NOT EXISTS auth_oauth_states (
            state TEXT PRIMARY KEY,
            nonce TEXT,
            pkce_verifier TEXT,
            provider TEXT NOT NULL,
            redirect_url TEXT NOT NULL,
            created_at BIGINT NOT NULL
        )",
            &[],
        )
        .map_err(|e| format!("Failed to create oauth_states table: {}", e))?;

    // Store URL for later connections
    let mut pg_url = POSTGRES_URL.lock().unwrap();
    *pg_url = Some(url.to_string());
    Ok(())
}

/// Initialize Redis session storage
fn init_redis_sessions(url: &str) -> std::result::Result<(), String> {
    // Convert valkey:// to redis:// for the redis crate
    let redis_url = if url.starts_with("valkey://") {
        url.replacen("valkey://", "redis://", 1)
    } else {
        url.to_string()
    };

    // Test connection
    let client = redis::Client::open(redis_url.as_str())
        .map_err(|e| format!("Failed to create Redis client: {}", e))?;

    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Failed to connect to Redis: {}", e))?;

    // Test with a PING
    let _: String = redis::cmd("PING")
        .query(&mut conn)
        .map_err(|e| format!("Redis PING failed: {}", e))?;

    // Store URL for later connections
    let mut redis_url_store = REDIS_URL.lock().unwrap();
    *redis_url_store = Some(redis_url);
    Ok(())
}

/// Get auth config
pub fn get_auth_config() -> Option<AuthConfig> {
    AUTH_CONFIG.lock().unwrap().clone()
}

/// Generate a secure session ID
pub fn generate_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Generate OAuth state token
pub fn generate_oauth_state() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Generate OIDC nonce
pub fn generate_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Sign a session ID with HMAC-SHA256
/// Returns: "session_id.signature"
pub fn sign_session_id(session_id: &str, secret: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can accept key of any size");
    mac.update(session_id.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    format!("{}.{}", session_id, signature)
}

/// Verify and extract session ID from signed token
/// Returns Some(session_id) if valid, None if invalid signature
/// Uses constant-time comparison to prevent timing attacks
pub fn verify_session_id(signed_token: &str, secret: &str) -> Option<String> {
    // Split into id and signature
    let parts: Vec<&str> = signed_token.rsplitn(2, '.').collect();
    if parts.len() != 2 {
        return None;
    }
    let signature = parts[0];
    let session_id = parts[1];

    // Decode the provided signature from hex
    let signature_bytes = match hex::decode(signature) {
        Ok(bytes) => bytes,
        Err(_) => return None,
    };

    // Verify signature using constant-time comparison
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can accept key of any size");
    mac.update(session_id.as_bytes());

    // verify_slice uses constant-time comparison internally
    match mac.verify_slice(&signature_bytes) {
        Ok(_) => Some(session_id.to_string()),
        Err(_) => None,
    }
}

/// Store OAuth state
pub fn store_oauth_state(
    state: &str,
    provider: &str,
    redirect_url: &str,
    nonce: Option<&str>,
    pkce_verifier: Option<&str>,
) {
    let oauth_state = OAuthState {
        state: state.to_string(),
        nonce: nonce.map(|s| s.to_string()),
        pkce_verifier: pkce_verifier.map(|s| s.to_string()),
        provider: provider.to_string(),
        redirect_url: redirect_url.to_string(),
        created_at: chrono::Utc::now().timestamp(),
    };

    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            if let Err(e) = store_oauth_state_sqlite(&oauth_state) {
                eprintln!(
                    "[auth] SQLite oauth state store failed, using memory: {}",
                    e
                );
                SESSION_STORE.lock().unwrap().set_oauth_state(oauth_state);
            }
        }
        Some(SessionStore::Postgres(_)) => {
            if let Err(e) = store_oauth_state_postgres(&oauth_state) {
                eprintln!(
                    "[auth] PostgreSQL oauth state store failed, using memory: {}",
                    e
                );
                SESSION_STORE.lock().unwrap().set_oauth_state(oauth_state);
            }
        }
        Some(SessionStore::Redis(_)) => {
            if let Err(e) = store_oauth_state_redis(&oauth_state) {
                eprintln!("[auth] Redis oauth state store failed, using memory: {}", e);
                SESSION_STORE.lock().unwrap().set_oauth_state(oauth_state);
            }
        }
        _ => {
            SESSION_STORE.lock().unwrap().set_oauth_state(oauth_state);
        }
    }
}

fn store_oauth_state_sqlite(state: &OAuthState) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    conn.execute(
        "INSERT OR REPLACE INTO auth_oauth_states
         (state, nonce, pkce_verifier, provider, redirect_url, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            state.state,
            state.nonce,
            state.pkce_verifier,
            state.provider,
            state.redirect_url,
            state.created_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn store_oauth_state_postgres(state: &OAuthState) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    client
        .execute(
            "INSERT INTO auth_oauth_states
         (state, nonce, pkce_verifier, provider, redirect_url, created_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (state) DO UPDATE SET
            nonce = $2, pkce_verifier = $3, provider = $4, redirect_url = $5, created_at = $6",
            &[
                &state.state,
                &state.nonce,
                &state.pkce_verifier,
                &state.provider,
                &state.redirect_url,
                &state.created_at,
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn store_oauth_state_redis(state: &OAuthState) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let state_json = serde_json::json!({
        "state": state.state,
        "nonce": state.nonce,
        "pkce_verifier": state.pkce_verifier,
        "provider": state.provider,
        "redirect_url": state.redirect_url,
        "created_at": state.created_at,
    })
    .to_string();

    let key = format!("ntnt:oauth_state:{}", state.state);
    // OAuth state expires in 10 minutes
    redis::cmd("SETEX")
        .arg(&key)
        .arg(600)
        .arg(&state_json)
        .query::<()>(&mut conn)
        .map_err(|e| format!("Redis SETEX error: {}", e))?;

    Ok(())
}

/// Retrieve and consume OAuth state
pub fn consume_oauth_state(state: &str) -> Option<OAuthState> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            consume_oauth_state_sqlite(state)
                .ok()
                .flatten()
                .or_else(|| {
                    let mut store = SESSION_STORE.lock().unwrap();
                    let oauth_state = store.get_oauth_state(state).cloned();
                    if oauth_state.is_some() {
                        store.delete_oauth_state(state);
                    }
                    oauth_state
                })
        }
        Some(SessionStore::Postgres(_)) => consume_oauth_state_postgres(state)
            .ok()
            .flatten()
            .or_else(|| {
                let mut store = SESSION_STORE.lock().unwrap();
                let oauth_state = store.get_oauth_state(state).cloned();
                if oauth_state.is_some() {
                    store.delete_oauth_state(state);
                }
                oauth_state
            }),
        Some(SessionStore::Redis(_)) => {
            consume_oauth_state_redis(state).ok().flatten().or_else(|| {
                let mut store = SESSION_STORE.lock().unwrap();
                let oauth_state = store.get_oauth_state(state).cloned();
                if oauth_state.is_some() {
                    store.delete_oauth_state(state);
                }
                oauth_state
            })
        }
        _ => {
            let mut store = SESSION_STORE.lock().unwrap();
            let oauth_state = store.get_oauth_state(state).cloned();
            if oauth_state.is_some() {
                store.delete_oauth_state(state);
            }
            oauth_state
        }
    }
}

fn consume_oauth_state_sqlite(state: &str) -> std::result::Result<Option<OAuthState>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();
    let min_created = now - 600; // 10 minutes

    let result = conn.query_row(
        "SELECT state, nonce, pkce_verifier, provider, redirect_url, created_at
         FROM auth_oauth_states WHERE state = ?1 AND created_at > ?2",
        rusqlite::params![state, min_created],
        |row| {
            Ok(OAuthState {
                state: row.get(0)?,
                nonce: row.get(1)?,
                pkce_verifier: row.get(2)?,
                provider: row.get(3)?,
                redirect_url: row.get(4)?,
                created_at: row.get(5)?,
            })
        },
    );

    // Delete the state (consume it)
    let _ = conn.execute(
        "DELETE FROM auth_oauth_states WHERE state = ?1",
        rusqlite::params![state],
    );

    match result {
        Ok(oauth_state) => Ok(Some(oauth_state)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn consume_oauth_state_postgres(state: &str) -> std::result::Result<Option<OAuthState>, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let now = chrono::Utc::now().timestamp();
    let min_created = now - 600; // 10 minutes

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let rows = client
        .query(
            "SELECT state, nonce, pkce_verifier, provider, redirect_url, created_at
         FROM auth_oauth_states WHERE state = $1 AND created_at > $2",
            &[&state, &min_created],
        )
        .map_err(|e| e.to_string())?;

    // Delete the state (consume it)
    let _ = client.execute("DELETE FROM auth_oauth_states WHERE state = $1", &[&state]);

    if let Some(row) = rows.first() {
        Ok(Some(OAuthState {
            state: row.get(0),
            nonce: row.get(1),
            pkce_verifier: row.get(2),
            provider: row.get(3),
            redirect_url: row.get(4),
            created_at: row.get(5),
        }))
    } else {
        Ok(None)
    }
}

fn consume_oauth_state_redis(state: &str) -> std::result::Result<Option<OAuthState>, String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = format!("ntnt:oauth_state:{}", state);

    // Use GETDEL for atomic get-and-delete (Redis 6.2+)
    // This prevents race conditions where two concurrent requests could both consume the same state
    let result: Option<String> = redis::cmd("GETDEL")
        .arg(&key)
        .query(&mut conn)
        .or_else(|_| {
            // Fallback for Redis < 6.2: use Lua script for atomicity
            let lua_script = r#"
                local value = redis.call('GET', KEYS[1])
                if value then
                    redis.call('DEL', KEYS[1])
                end
                return value
            "#;
            redis::cmd("EVAL")
                .arg(lua_script)
                .arg(1)
                .arg(&key)
                .query(&mut conn)
        })
        .map_err(|e| format!("Redis GETDEL error: {}", e))?;

    match result {
        Some(json_str) => {
            let json: serde_json::Value =
                serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {}", e))?;

            Ok(Some(OAuthState {
                state: json["state"].as_str().unwrap_or("").to_string(),
                nonce: json["nonce"].as_str().map(|s| s.to_string()),
                pkce_verifier: json["pkce_verifier"].as_str().map(|s| s.to_string()),
                provider: json["provider"].as_str().unwrap_or("").to_string(),
                redirect_url: json["redirect_url"].as_str().unwrap_or("").to_string(),
                created_at: json["created_at"].as_i64().unwrap_or(0),
            }))
        }
        None => Ok(None),
    }
}

/// Create a session from OAuth user info
pub fn create_session(
    provider_name: &str,
    user_info: HashMap<String, Value>,
    tokens: Option<&TokenResponse>,
    ttl: i64,
) -> std::result::Result<Session, String> {
    let now = chrono::Utc::now().timestamp();
    let (user_id, email, name, picture) = extract_user_info(provider_name, &user_info);

    // Validate that we got a valid user ID from the provider
    if user_id.is_empty() {
        return Err(format!(
            "Provider '{}' returned no user ID. Response keys: {:?}",
            provider_name,
            user_info.keys().collect::<Vec<_>>()
        ));
    }

    let raw_json = value_map_to_json_string(&user_info);

    let (access_token, refresh_token, token_expires_at) = if let Some(t) = tokens {
        (
            Some(t.access_token.clone()),
            t.refresh_token.clone(),
            t.expires_in.map(|e| now + e),
        )
    } else {
        (None, None, None)
    };

    Ok(Session {
        id: generate_session_id(),
        user_id: format!("{}:{}", provider_name, user_id),
        provider: provider_name.to_string(),
        email,
        name,
        picture,
        raw_json,
        data_json: "{}".to_string(),
        csrf_token: uuid::Uuid::new_v4().to_string(),
        access_token,
        refresh_token,
        token_expires_at,
        created_at: now,
        expires_at: now + ttl,
    })
}

/// Store session
pub fn store_session(session: Session) {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            if let Err(e) = store_session_sqlite(&session) {
                eprintln!("[auth] WARNING: SQLite store failed: {}", e);
                eprintln!("[auth] Falling back to memory store - session will be lost on restart!");
                SESSION_STORE.lock().unwrap().set_session(session);
            }
        }
        Some(SessionStore::Postgres(_)) => {
            if let Err(e) = store_session_postgres(&session) {
                eprintln!("[auth] WARNING: PostgreSQL store failed: {}", e);
                eprintln!("[auth] Falling back to memory store - session will be lost on restart and not shared across instances!");
                SESSION_STORE.lock().unwrap().set_session(session);
            }
        }
        Some(SessionStore::Redis(_)) => {
            if let Err(e) = store_session_redis(&session) {
                eprintln!("[auth] WARNING: Redis store failed: {}", e);
                eprintln!("[auth] Falling back to memory store - session will be lost on restart and not shared across instances!");
                SESSION_STORE.lock().unwrap().set_session(session);
            }
        }
        _ => {
            SESSION_STORE.lock().unwrap().set_session(session);
        }
    }
}

fn store_session_sqlite(session: &Session) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    conn.execute(
        "INSERT OR REPLACE INTO auth_sessions
         (id, user_id, provider, email, name, picture, raw_json, data_json, csrf_token,
          access_token, refresh_token, token_expires_at, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        rusqlite::params![
            session.id,
            session.user_id,
            session.provider,
            session.email,
            session.name,
            session.picture,
            session.raw_json,
            session.data_json,
            session.csrf_token,
            session.access_token,
            session.refresh_token,
            session.token_expires_at,
            session.created_at,
            session.expires_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn store_session_postgres(session: &Session) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    client
        .execute(
            "INSERT INTO auth_sessions
         (id, user_id, provider, email, name, picture, raw_json, data_json, csrf_token,
          access_token, refresh_token, token_expires_at, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
         ON CONFLICT (id) DO UPDATE SET
            access_token = $10, refresh_token = $11, token_expires_at = $12",
            &[
                &session.id,
                &session.user_id,
                &session.provider,
                &session.email,
                &session.name,
                &session.picture,
                &session.raw_json,
                &session.data_json,
                &session.csrf_token,
                &session.access_token,
                &session.refresh_token,
                &session.token_expires_at,
                &session.created_at,
                &session.expires_at,
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn store_session_redis(session: &Session) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    // Serialize session to JSON
    let session_json = serde_json::json!({
        "id": session.id,
        "user_id": session.user_id,
        "provider": session.provider,
        "email": session.email,
        "name": session.name,
        "picture": session.picture,
        "raw_json": session.raw_json,
        "data_json": session.data_json,
        "csrf_token": session.csrf_token,
        "access_token": session.access_token,
        "refresh_token": session.refresh_token,
        "token_expires_at": session.token_expires_at,
        "created_at": session.created_at,
        "expires_at": session.expires_at,
    })
    .to_string();

    let key = format!("ntnt:session:{}", session.id);
    let ttl = session.expires_at - chrono::Utc::now().timestamp();

    if ttl > 0 {
        // SETEX: set with expiration
        redis::cmd("SETEX")
            .arg(&key)
            .arg(ttl)
            .arg(&session_json)
            .query::<()>(&mut conn)
            .map_err(|e| format!("Redis SETEX error: {}", e))?;
    } else {
        // Session already expired, don't store
        return Ok(());
    }

    Ok(())
}

/// Get session by ID
pub fn get_session_by_id(id: &str) -> Option<Session> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    let session = match store_type {
        Some(SessionStore::Sqlite(_)) => get_session_sqlite(id)
            .ok()
            .flatten()
            .or_else(|| SESSION_STORE.lock().unwrap().get_session(id).cloned()),
        Some(SessionStore::Postgres(_)) => get_session_postgres(id)
            .ok()
            .flatten()
            .or_else(|| SESSION_STORE.lock().unwrap().get_session(id).cloned()),
        Some(SessionStore::Redis(_)) => get_session_redis(id)
            .ok()
            .flatten()
            .or_else(|| SESSION_STORE.lock().unwrap().get_session(id).cloned()),
        _ => SESSION_STORE.lock().unwrap().get_session(id).cloned(),
    };

    // If we got a valid session, return it
    if session.is_some() {
        return session;
    }

    // Session not found (expired or missing). Try to find an expired-but-refreshable session.
    if let Some(config) = &config {
        if config.store_tokens {
            let expired_session = match &config.session_store {
                SessionStore::Sqlite(_) => get_expired_session_sqlite(id, config.refresh_ttl)
                    .ok()
                    .flatten(),
                SessionStore::Postgres(_) => get_expired_session_postgres(id, config.refresh_ttl)
                    .ok()
                    .flatten(),
                SessionStore::Redis(_) => get_expired_session_redis(id, config.refresh_ttl)
                    .ok()
                    .flatten(),
                _ => None, // Memory store already filters in get_session
            };

            if let Some(expired) = expired_session {
                if let Some(ref refresh_token) = expired.refresh_token {
                    // Find the provider config for this session
                    if let Some(provider) =
                        config.providers.iter().find(|p| p.name == expired.provider)
                    {
                        match refresh_access_token(provider, refresh_token) {
                            Ok(tokens) => {
                                // Extend session expiry by session_ttl
                                let now = chrono::Utc::now().timestamp();
                                let new_expires_at = now + config.session_ttl;

                                // Update tokens and extend session
                                update_session_tokens(&expired.id, &tokens);
                                extend_session_expiry(&expired.id, new_expires_at);

                                eprintln!(
                                    "[auth] Session {} auto-refreshed via refresh token",
                                    &expired.id[..8]
                                );

                                // Return the refreshed session
                                let mut refreshed = expired;
                                refreshed.access_token = Some(tokens.access_token);
                                if let Some(rt) = tokens.refresh_token {
                                    refreshed.refresh_token = Some(rt);
                                }
                                refreshed.token_expires_at = tokens.expires_in.map(|e| now + e);
                                refreshed.expires_at = new_expires_at;
                                return Some(refreshed);
                            }
                            Err(e) => {
                                eprintln!(
                                    "[auth] Auto-refresh failed for session {}: {}",
                                    &expired.id[..8],
                                    e
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Extend a session's expires_at timestamp (used after successful refresh)
fn extend_session_expiry(id: &str, new_expires_at: i64) {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            let _ = extend_session_expiry_sqlite(id, new_expires_at);
        }
        Some(SessionStore::Postgres(_)) => {
            let _ = extend_session_expiry_postgres(id, new_expires_at);
        }
        Some(SessionStore::Redis(_)) => {
            let _ = extend_session_expiry_redis(id, new_expires_at);
        }
        _ => {
            let mut store = SESSION_STORE.lock().unwrap();
            if let Some(session) = store.get_session_mut(id) {
                session.expires_at = new_expires_at;
            }
        }
    }
}

fn extend_session_expiry_sqlite(id: &str, new_expires_at: i64) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.execute(
        "UPDATE auth_sessions SET expires_at = ?1 WHERE id = ?2",
        rusqlite::params![new_expires_at, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn extend_session_expiry_postgres(
    id: &str,
    new_expires_at: i64,
) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    client
        .execute(
            "UPDATE auth_sessions SET expires_at = $1 WHERE id = $2",
            &[&new_expires_at, &id],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn extend_session_expiry_redis(id: &str, new_expires_at: i64) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;
    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;
    let key = format!("ntnt:session:{}", id);
    let now = chrono::Utc::now().timestamp();
    let new_ttl = new_expires_at - now;
    if new_ttl > 0 {
        // Read-modify-write with new expiry
        let lua_script = r#"
            local session = redis.call('GET', KEYS[1])
            if not session then return nil end
            local data = cjson.decode(session)
            data.expires_at = tonumber(ARGV[1])
            redis.call('SETEX', KEYS[1], tonumber(ARGV[2]), cjson.encode(data))
            return 1
        "#;
        let _: Option<i32> = redis::Script::new(lua_script)
            .key(&key)
            .arg(new_expires_at)
            .arg(new_ttl)
            .invoke(&mut conn)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn get_session_sqlite(id: &str) -> std::result::Result<Option<Session>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();

    let result = conn.query_row(
        "SELECT id, user_id, provider, email, name, picture, raw_json, data_json, csrf_token,
                access_token, refresh_token, token_expires_at, created_at, expires_at
         FROM auth_sessions WHERE id = ?1 AND expires_at > ?2",
        rusqlite::params![id, now],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                user_id: row.get(1)?,
                provider: row.get(2)?,
                email: row.get(3)?,
                name: row.get(4)?,
                picture: row.get(5)?,
                raw_json: row.get(6)?,
                data_json: row.get(7)?,
                csrf_token: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                access_token: row.get(9)?,
                refresh_token: row.get(10)?,
                token_expires_at: row.get(11)?,
                created_at: row.get(12)?,
                expires_at: row.get(13)?,
            })
        },
    );

    match result {
        Ok(session) => Ok(Some(session)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Retrieve an expired session that's still within the refresh window
fn get_expired_session_sqlite(
    id: &str,
    refresh_ttl: i64,
) -> std::result::Result<Option<Session>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();
    let refresh_cutoff = now - refresh_ttl;

    let result = conn.query_row(
        "SELECT id, user_id, provider, email, name, picture, raw_json, data_json, csrf_token,
                access_token, refresh_token, token_expires_at, created_at, expires_at
         FROM auth_sessions WHERE id = ?1 AND expires_at <= ?2 AND created_at > ?3 AND refresh_token IS NOT NULL",
        rusqlite::params![id, now, refresh_cutoff],
        |row| {
            Ok(Session {
                id: row.get(0)?, user_id: row.get(1)?, provider: row.get(2)?,
                email: row.get(3)?, name: row.get(4)?, picture: row.get(5)?,
                raw_json: row.get(6)?, data_json: row.get(7)?,
                csrf_token: row.get::<_, Option<String>>(8)?.unwrap_or_default(),
                access_token: row.get(9)?, refresh_token: row.get(10)?,
                token_expires_at: row.get(11)?, created_at: row.get(12)?, expires_at: row.get(13)?,
            })
        },
    );
    match result {
        Ok(session) => Ok(Some(session)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn get_expired_session_postgres(
    id: &str,
    refresh_ttl: i64,
) -> std::result::Result<Option<Session>, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let now = chrono::Utc::now().timestamp();
    let refresh_cutoff = now - refresh_ttl;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    let rows = client.query(
        "SELECT id, user_id, provider, email, name, picture, raw_json, data_json, csrf_token,
                access_token, refresh_token, token_expires_at, created_at, expires_at
         FROM auth_sessions WHERE id = $1 AND expires_at <= $2 AND created_at > $3 AND refresh_token IS NOT NULL",
        &[&id, &now, &refresh_cutoff],
    ).map_err(|e| e.to_string())?;

    if let Some(row) = rows.first() {
        Ok(Some(Session {
            id: row.get(0),
            user_id: row.get(1),
            provider: row.get(2),
            email: row.get(3),
            name: row.get(4),
            picture: row.get(5),
            raw_json: row.get(6),
            data_json: row.get(7),
            csrf_token: row.get::<_, Option<String>>(8).unwrap_or_default(),
            access_token: row.get(9),
            refresh_token: row.get(10),
            token_expires_at: row.get(11),
            created_at: row.get(12),
            expires_at: row.get(13),
        }))
    } else {
        Ok(None)
    }
}

fn get_expired_session_redis(
    id: &str,
    refresh_ttl: i64,
) -> std::result::Result<Option<Session>, String> {
    // Redis sessions use TTL-based expiry, so expired sessions are already deleted.
    // For Redis to support refresh, we'd need a separate refresh token key with longer TTL.
    // For now, return None — Redis users should set session_ttl = refresh_ttl.
    let _ = (id, refresh_ttl);
    Ok(None)
}

fn get_session_postgres(id: &str) -> std::result::Result<Option<Session>, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let now = chrono::Utc::now().timestamp();

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let rows = client
        .query(
            "SELECT id, user_id, provider, email, name, picture, raw_json, data_json, csrf_token,
                access_token, refresh_token, token_expires_at, created_at, expires_at
         FROM auth_sessions WHERE id = $1 AND expires_at > $2",
            &[&id, &now],
        )
        .map_err(|e| e.to_string())?;

    if let Some(row) = rows.first() {
        Ok(Some(Session {
            id: row.get(0),
            user_id: row.get(1),
            provider: row.get(2),
            email: row.get(3),
            name: row.get(4),
            picture: row.get(5),
            raw_json: row.get(6),
            data_json: row.get(7),
            csrf_token: row.get::<_, Option<String>>(8).unwrap_or_default(),
            access_token: row.get(9),
            refresh_token: row.get(10),
            token_expires_at: row.get(11),
            created_at: row.get(12),
            expires_at: row.get(13),
        }))
    } else {
        Ok(None)
    }
}

fn get_session_redis(id: &str) -> std::result::Result<Option<Session>, String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = format!("ntnt:session:{}", id);
    let result: Option<String> = redis::cmd("GET")
        .arg(&key)
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;

    match result {
        Some(json_str) => {
            let json: serde_json::Value =
                serde_json::from_str(&json_str).map_err(|e| format!("JSON parse error: {}", e))?;

            // Validate critical fields are present - don't silently accept empty/missing data
            let id = json["id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Session missing or empty 'id' field".to_string())?;
            let user_id = json["user_id"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Session missing or empty 'user_id' field".to_string())?;
            let provider = json["provider"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Session missing or empty 'provider' field".to_string())?;
            let csrf_token = json["csrf_token"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| "Session missing or empty 'csrf_token' field".to_string())?;
            let expires_at = json["expires_at"]
                .as_i64()
                .ok_or_else(|| "Session missing 'expires_at' field".to_string())?;
            let created_at = json["created_at"]
                .as_i64()
                .ok_or_else(|| "Session missing 'created_at' field".to_string())?;

            Ok(Some(Session {
                id: id.to_string(),
                user_id: user_id.to_string(),
                provider: provider.to_string(),
                email: json["email"].as_str().map(|s| s.to_string()),
                name: json["name"].as_str().map(|s| s.to_string()),
                picture: json["picture"].as_str().map(|s| s.to_string()),
                raw_json: json["raw_json"].as_str().unwrap_or("{}").to_string(),
                data_json: json["data_json"].as_str().unwrap_or("{}").to_string(),
                csrf_token: csrf_token.to_string(),
                access_token: json["access_token"].as_str().map(|s| s.to_string()),
                refresh_token: json["refresh_token"].as_str().map(|s| s.to_string()),
                token_expires_at: json["token_expires_at"].as_i64(),
                created_at,
                expires_at,
            }))
        }
        None => Ok(None),
    }
}

/// Update session tokens
pub fn update_session_tokens(id: &str, tokens: &TokenResponse) {
    let now = chrono::Utc::now().timestamp();
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            let _ = update_session_tokens_sqlite(id, tokens, now);
        }
        Some(SessionStore::Postgres(_)) => {
            let _ = update_session_tokens_postgres(id, tokens, now);
        }
        Some(SessionStore::Redis(_)) => {
            let _ = update_session_tokens_redis(id, tokens, now);
        }
        _ => {
            let mut store = SESSION_STORE.lock().unwrap();
            if let Some(session) = store.get_session_mut(id) {
                session.access_token = Some(tokens.access_token.clone());
                if let Some(ref rt) = tokens.refresh_token {
                    session.refresh_token = Some(rt.clone());
                }
                session.token_expires_at = tokens.expires_in.map(|e| now + e);
            }
        }
    }
}

fn update_session_tokens_sqlite(
    id: &str,
    tokens: &TokenResponse,
    now: i64,
) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let expires_at = tokens.expires_in.map(|e| now + e);

    conn.execute(
        "UPDATE auth_sessions SET access_token = ?1, refresh_token = COALESCE(?2, refresh_token),
         token_expires_at = ?3 WHERE id = ?4",
        rusqlite::params![tokens.access_token, tokens.refresh_token, expires_at, id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn update_session_tokens_postgres(
    id: &str,
    tokens: &TokenResponse,
    now: i64,
) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let expires_at = tokens.expires_in.map(|e| now + e);

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    client.execute(
        "UPDATE auth_sessions SET access_token = $1, refresh_token = COALESCE($2, refresh_token),
         token_expires_at = $3 WHERE id = $4",
        &[&tokens.access_token, &tokens.refresh_token, &expires_at, &id],
    ).map_err(|e| e.to_string())?;
    Ok(())
}

fn update_session_tokens_redis(
    id: &str,
    tokens: &TokenResponse,
    now: i64,
) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = format!("ntnt:session:{}", id);
    let expires_at = tokens.expires_in.map(|e| now + e);

    // Use Lua script for atomic read-modify-write to prevent race conditions
    let lua_script = r#"
        local session = redis.call('GET', KEYS[1])
        if not session then
            return nil
        end
        local data = cjson.decode(session)
        data.access_token = ARGV[1]
        if ARGV[2] ~= '' then
            data.refresh_token = ARGV[2]
        end
        if ARGV[3] ~= '' then
            data.token_expires_at = tonumber(ARGV[3])
        else
            data.token_expires_at = nil
        end
        local new_session = cjson.encode(data)
        local ttl = redis.call('TTL', KEYS[1])
        if ttl > 0 then
            redis.call('SETEX', KEYS[1], ttl, new_session)
        else
            redis.call('SET', KEYS[1], new_session)
        end
        return 'OK'
    "#;

    let refresh_token = tokens.refresh_token.as_deref().unwrap_or("");
    let expires_at_str = expires_at.map(|e| e.to_string()).unwrap_or_default();

    let result: Option<String> = redis::cmd("EVAL")
        .arg(lua_script)
        .arg(1)
        .arg(&key)
        .arg(&tokens.access_token)
        .arg(refresh_token)
        .arg(&expires_at_str)
        .query(&mut conn)
        .map_err(|e| format!("Redis EVAL error: {}", e))?;

    if result.is_none() {
        return Err("Session not found".to_string());
    }
    Ok(())
}

/// Update session custom data (for RBAC/claims)
pub fn update_session_data(id: &str, data_json: &str) -> std::result::Result<(), String> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => update_session_data_sqlite(id, data_json),
        Some(SessionStore::Postgres(_)) => update_session_data_postgres(id, data_json),
        Some(SessionStore::Redis(_)) => update_session_data_redis(id, data_json),
        _ => {
            // Memory backend
            let mut store = SESSION_STORE.lock().unwrap();
            if let Some(session) = store.get_session_mut(id) {
                session.data_json = data_json.to_string();
                Ok(())
            } else {
                Err("Session not found".to_string())
            }
        }
    }
}

fn update_session_data_sqlite(id: &str, data_json: &str) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    let rows = conn
        .execute(
            "UPDATE auth_sessions SET data_json = ?1 WHERE id = ?2",
            rusqlite::params![data_json, id],
        )
        .map_err(|e| e.to_string())?;

    if rows == 0 {
        Err("Session not found".to_string())
    } else {
        Ok(())
    }
}

fn update_session_data_postgres(id: &str, data_json: &str) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let rows = client
        .execute(
            "UPDATE auth_sessions SET data_json = $1 WHERE id = $2",
            &[&data_json, &id],
        )
        .map_err(|e| e.to_string())?;

    if rows == 0 {
        Err("Session not found".to_string())
    } else {
        Ok(())
    }
}

fn update_session_data_redis(id: &str, data_json: &str) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = format!("ntnt:session:{}", id);

    // Use Lua script for atomic read-modify-write to prevent race conditions
    let lua_script = r#"
        local session = redis.call('GET', KEYS[1])
        if not session then
            return nil
        end
        local data = cjson.decode(session)
        data.data_json = ARGV[1]
        local new_session = cjson.encode(data)
        local ttl = redis.call('TTL', KEYS[1])
        if ttl > 0 then
            redis.call('SETEX', KEYS[1], ttl, new_session)
        else
            redis.call('SET', KEYS[1], new_session)
        end
        return 'OK'
    "#;

    let result: Option<String> = redis::cmd("EVAL")
        .arg(lua_script)
        .arg(1)
        .arg(&key)
        .arg(data_json)
        .query(&mut conn)
        .map_err(|e| format!("Redis EVAL error: {}", e))?;

    if result.is_none() {
        Err("Session not found".to_string())
    } else {
        Ok(())
    }
}

/// Delete session by ID
pub fn delete_session_by_id(id: &str) {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            let _ = delete_session_sqlite(id);
        }
        Some(SessionStore::Postgres(_)) => {
            let _ = delete_session_postgres(id);
        }
        Some(SessionStore::Redis(_)) => {
            let _ = delete_session_redis(id);
        }
        _ => {}
    }
    // Always clean from memory too (fallback might have been used)
    SESSION_STORE.lock().unwrap().delete_session(id);
}

fn delete_session_sqlite(id: &str) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.execute(
        "DELETE FROM auth_sessions WHERE id = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_session_postgres(id: &str) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    client
        .execute("DELETE FROM auth_sessions WHERE id = $1", &[&id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_session_redis(id: &str) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = format!("ntnt:session:{}", id);
    redis::cmd("DEL")
        .arg(&key)
        .query::<()>(&mut conn)
        .map_err(|e| format!("Redis DEL error: {}", e))?;
    Ok(())
}

/// Cleanup expired sessions from the session store
/// Returns the number of sessions cleaned up
pub fn cleanup_expired_sessions() -> std::result::Result<u64, String> {
    let now = chrono::Utc::now().timestamp();
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => cleanup_expired_sessions_sqlite(now),
        Some(SessionStore::Postgres(_)) => cleanup_expired_sessions_postgres(now),
        Some(SessionStore::Redis(_)) => cleanup_expired_sessions_redis(now),
        _ => {
            // Memory backend - clean up in-memory store
            let mut store = SESSION_STORE.lock().unwrap();
            let count = store.cleanup_expired(now);
            Ok(count as u64)
        }
    }
}

fn cleanup_expired_sessions_sqlite(now: i64) -> std::result::Result<u64, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    let count = conn
        .execute(
            "DELETE FROM auth_sessions WHERE expires_at < ?1",
            rusqlite::params![now],
        )
        .map_err(|e| e.to_string())?;

    Ok(count as u64)
}

fn cleanup_expired_sessions_postgres(now: i64) -> std::result::Result<u64, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let count = client
        .execute("DELETE FROM auth_sessions WHERE expires_at < $1", &[&now])
        .map_err(|e| e.to_string())?;

    Ok(count)
}

fn cleanup_expired_sessions_redis(now: i64) -> std::result::Result<u64, String> {
    // Redis uses TTL for expiration, so keys expire automatically
    // But we can scan for any orphaned keys with expired sessions
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    // Scan for session keys and check their expires_at
    let mut count = 0u64;
    let mut cursor = 0u64;
    loop {
        let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:session:*")
            .arg("COUNT")
            .arg(100)
            .query(&mut conn)
            .map_err(|e| format!("Redis SCAN error: {}", e))?;

        for key in keys {
            let result: Option<String> = redis::cmd("GET")
                .arg(&key)
                .query(&mut conn)
                .map_err(|e| format!("Redis GET error: {}", e))?;

            if let Some(json_str) = result {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    if let Some(expires_at) = json["expires_at"].as_i64() {
                        if expires_at < now {
                            let _: () = redis::cmd("DEL")
                                .arg(&key)
                                .query(&mut conn)
                                .map_err(|e| format!("Redis DEL error: {}", e))?;
                            count += 1;
                        }
                    }
                }
            }
        }

        cursor = new_cursor;
        if cursor == 0 {
            break;
        }
    }

    Ok(count)
}

/// Cleanup expired OAuth states from the session store
/// OAuth states expire after 10 minutes
pub fn cleanup_expired_oauth_states() -> std::result::Result<u64, String> {
    let now = chrono::Utc::now().timestamp();
    let max_age = 600; // 10 minutes
    let cutoff = now - max_age;

    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => cleanup_expired_oauth_states_sqlite(cutoff),
        Some(SessionStore::Postgres(_)) => cleanup_expired_oauth_states_postgres(cutoff),
        Some(SessionStore::Redis(_)) => {
            // Redis OAuth states use TTL, so they expire automatically
            Ok(0)
        }
        _ => {
            // Memory backend - clean up in-memory store
            let mut store = SESSION_STORE.lock().unwrap();
            let count = store.cleanup_expired_oauth_states(cutoff);
            Ok(count as u64)
        }
    }
}

fn cleanup_expired_oauth_states_sqlite(cutoff: i64) -> std::result::Result<u64, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    let count = conn
        .execute(
            "DELETE FROM auth_oauth_states WHERE created_at < ?1",
            rusqlite::params![cutoff],
        )
        .map_err(|e| e.to_string())?;

    Ok(count as u64)
}

fn cleanup_expired_oauth_states_postgres(cutoff: i64) -> std::result::Result<u64, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let count = client
        .execute(
            "DELETE FROM auth_oauth_states WHERE created_at < $1",
            &[&cutoff],
        )
        .map_err(|e| e.to_string())?;

    Ok(count)
}

/// Get all sessions for a user
pub fn get_sessions_for_user(
    user_id: &str,
    current_session_id: Option<&str>,
) -> std::result::Result<Vec<SessionInfo>, String> {
    let now = chrono::Utc::now().timestamp();
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            get_sessions_for_user_sqlite(user_id, current_session_id, now)
        }
        Some(SessionStore::Postgres(_)) => {
            get_sessions_for_user_postgres(user_id, current_session_id, now)
        }
        Some(SessionStore::Redis(_)) => {
            get_sessions_for_user_redis(user_id, current_session_id, now)
        }
        _ => {
            // Memory backend
            let store = SESSION_STORE.lock().unwrap();
            Ok(store.get_sessions_for_user(user_id, current_session_id, now))
        }
    }
}

fn get_sessions_for_user_sqlite(
    user_id: &str,
    current_session_id: Option<&str>,
    now: i64,
) -> std::result::Result<Vec<SessionInfo>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    let mut stmt = conn
        .prepare(
            "SELECT id, user_id, provider, created_at, expires_at FROM auth_sessions
         WHERE user_id = ?1 AND expires_at > ?2 ORDER BY created_at DESC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![user_id, now], |row| {
            Ok(SessionInfo {
                id: row.get(0)?,
                user_id: row.get(1)?,
                provider: row.get(2)?,
                created_at: row.get(3)?,
                expires_at: row.get(4)?,
                is_current: false,
            })
        })
        .map_err(|e| e.to_string())?;

    let mut sessions: Vec<SessionInfo> = rows.filter_map(|r| r.ok()).collect();

    // Mark current session
    if let Some(current_id) = current_session_id {
        for session in &mut sessions {
            if session.id == current_id {
                session.is_current = true;
            }
        }
    }

    Ok(sessions)
}

fn get_sessions_for_user_postgres(
    user_id: &str,
    current_session_id: Option<&str>,
    now: i64,
) -> std::result::Result<Vec<SessionInfo>, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let rows = client
        .query(
            "SELECT id, user_id, provider, created_at, expires_at FROM auth_sessions
         WHERE user_id = $1 AND expires_at > $2 ORDER BY created_at DESC",
            &[&user_id, &now],
        )
        .map_err(|e| e.to_string())?;

    let mut sessions: Vec<SessionInfo> = rows
        .iter()
        .map(|row| SessionInfo {
            id: row.get(0),
            user_id: row.get(1),
            provider: row.get(2),
            created_at: row.get(3),
            expires_at: row.get(4),
            is_current: current_session_id
                .map(|c| c == row.get::<_, String>(0))
                .unwrap_or(false),
        })
        .collect();

    // Mark current session
    if let Some(current_id) = current_session_id {
        for session in &mut sessions {
            if session.id == current_id {
                session.is_current = true;
            }
        }
    }

    Ok(sessions)
}

fn get_sessions_for_user_redis(
    user_id: &str,
    current_session_id: Option<&str>,
    now: i64,
) -> std::result::Result<Vec<SessionInfo>, String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let mut sessions = Vec::new();
    let mut cursor = 0u64;

    loop {
        let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:session:*")
            .arg("COUNT")
            .arg(100)
            .query(&mut conn)
            .map_err(|e| format!("Redis SCAN error: {}", e))?;

        for key in keys {
            let result: Option<String> = redis::cmd("GET")
                .arg(&key)
                .query(&mut conn)
                .map_err(|e| format!("Redis GET error: {}", e))?;

            if let Some(json_str) = result {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    let session_user_id = json["user_id"].as_str().unwrap_or("");
                    let expires_at = json["expires_at"].as_i64().unwrap_or(0);

                    if session_user_id == user_id && expires_at > now {
                        let session_id = json["id"].as_str().unwrap_or("").to_string();
                        sessions.push(SessionInfo {
                            id: session_id.clone(),
                            user_id: session_user_id.to_string(),
                            provider: json["provider"].as_str().unwrap_or("").to_string(),
                            created_at: json["created_at"].as_i64().unwrap_or(0),
                            expires_at,
                            is_current: current_session_id
                                .map(|c| c == session_id)
                                .unwrap_or(false),
                        });
                    }
                }
            }
        }

        cursor = new_cursor;
        if cursor == 0 {
            break;
        }
    }

    // Sort by created_at descending
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(sessions)
}

/// Delete all sessions for a user, optionally keeping one session
/// Returns the number of sessions deleted
pub fn delete_all_sessions_for_user(
    user_id: &str,
    keep_session_id: Option<&str>,
) -> std::result::Result<u64, String> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            delete_all_sessions_for_user_sqlite(user_id, keep_session_id)
        }
        Some(SessionStore::Postgres(_)) => {
            delete_all_sessions_for_user_postgres(user_id, keep_session_id)
        }
        Some(SessionStore::Redis(_)) => {
            delete_all_sessions_for_user_redis(user_id, keep_session_id)
        }
        _ => {
            // Memory backend
            let mut store = SESSION_STORE.lock().unwrap();
            Ok(store.delete_all_sessions_for_user(user_id, keep_session_id) as u64)
        }
    }
}

fn delete_all_sessions_for_user_sqlite(
    user_id: &str,
    keep_session_id: Option<&str>,
) -> std::result::Result<u64, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    let count = if let Some(keep_id) = keep_session_id {
        conn.execute(
            "DELETE FROM auth_sessions WHERE user_id = ?1 AND id != ?2",
            rusqlite::params![user_id, keep_id],
        )
        .map_err(|e| e.to_string())?
    } else {
        conn.execute(
            "DELETE FROM auth_sessions WHERE user_id = ?1",
            rusqlite::params![user_id],
        )
        .map_err(|e| e.to_string())?
    };

    Ok(count as u64)
}

fn delete_all_sessions_for_user_postgres(
    user_id: &str,
    keep_session_id: Option<&str>,
) -> std::result::Result<u64, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let count = if let Some(keep_id) = keep_session_id {
        client
            .execute(
                "DELETE FROM auth_sessions WHERE user_id = $1 AND id != $2",
                &[&user_id, &keep_id],
            )
            .map_err(|e| e.to_string())?
    } else {
        client
            .execute("DELETE FROM auth_sessions WHERE user_id = $1", &[&user_id])
            .map_err(|e| e.to_string())?
    };

    Ok(count)
}

fn delete_all_sessions_for_user_redis(
    user_id: &str,
    keep_session_id: Option<&str>,
) -> std::result::Result<u64, String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let mut count = 0u64;
    let mut cursor = 0u64;

    loop {
        let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:session:*")
            .arg("COUNT")
            .arg(100)
            .query(&mut conn)
            .map_err(|e| format!("Redis SCAN error: {}", e))?;

        for key in keys {
            let result: Option<String> = redis::cmd("GET")
                .arg(&key)
                .query(&mut conn)
                .map_err(|e| format!("Redis GET error: {}", e))?;

            if let Some(json_str) = result {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&json_str) {
                    let session_user_id = json["user_id"].as_str().unwrap_or("");
                    let session_id = json["id"].as_str().unwrap_or("");

                    if session_user_id == user_id {
                        // Skip the session we want to keep
                        if keep_session_id.map(|k| k == session_id).unwrap_or(false) {
                            continue;
                        }

                        let _: () = redis::cmd("DEL")
                            .arg(&key)
                            .query(&mut conn)
                            .map_err(|e| format!("Redis DEL error: {}", e))?;
                        count += 1;
                    }
                }
            }
        }

        cursor = new_cursor;
        if cursor == 0 {
            break;
        }
    }

    Ok(count)
}

// ============================================================================
// SECTION 9: Password Utilities
// ============================================================================

/// Hash a password using bcrypt
pub fn hash_password(password: &str) -> std::result::Result<String, String> {
    bcrypt::hash(password, bcrypt::DEFAULT_COST)
        .map_err(|e| format!("Password hashing failed: {}", e))
}

/// Verify a password against a bcrypt hash
pub fn verify_password_hash(password: &str, hash: &str) -> bool {
    bcrypt::verify(password, hash).unwrap_or(false)
}

// ============================================================================
// SECTION 11: MFA/TOTP Functions
// ============================================================================

/// Generate a new TOTP secret
pub fn generate_totp_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

/// Create a TOTP instance from a secret
fn create_totp(secret: &str, email: &str, issuer: &str) -> std::result::Result<TOTP, String> {
    let secret = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|e| format!("Invalid secret: {}", e))?;

    TOTP::new(
        TotpAlgorithm::SHA1,
        6,  // 6 digits
        1,  // 1 step (30 seconds)
        30, // 30 second period
        secret,
        Some(issuer.to_string()),
        email.to_string(),
    )
    .map_err(|e| format!("Failed to create TOTP: {}", e))
}

/// Generate the otpauth:// URI for QR codes
pub fn get_totp_uri(
    secret: &str,
    email: &str,
    issuer: &str,
) -> std::result::Result<String, String> {
    let totp = create_totp(secret, email, issuer)?;
    Ok(totp.get_url())
}

/// Verify a TOTP code
pub fn verify_totp_code(secret: &str, code: &str, email: &str) -> bool {
    match create_totp(secret, email, "NTNT") {
        Ok(totp) => totp.check_current(code).unwrap_or(false),
        Err(_) => false,
    }
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

/// Get user from request as HashMap (internal helper)
fn get_user_from_request(request: &Value) -> Option<HashMap<String, Value>> {
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

// ============================================================================
// SECTION 8: Route Handlers
// ============================================================================

/// Helper to get host and protocol from request
fn get_host_and_proto(req: &Value) -> (String, String) {
    if let Value::Map(req_map) = req {
        let host = req_map
            .get("headers")
            .and_then(|h| {
                if let Value::Map(headers) = h {
                    headers.get("host").and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
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
            .get("headers")
            .and_then(|h| {
                if let Value::Map(headers) = h {
                    headers.get("x-forwarded-proto").and_then(|v| {
                        if let Value::String(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                } else {
                    None
                }
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

/// Helper to create redirect response
fn redirect_response(url: &str, cookies: Option<&str>) -> Value {
    let mut headers = HashMap::new();
    headers.insert("Location".to_string(), Value::String(url.to_string()));
    if let Some(cookie) = cookies {
        headers.insert("Set-Cookie".to_string(), Value::String(cookie.to_string()));
    }

    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(302));
    response.insert("headers".to_string(), Value::Map(headers));
    response.insert("body".to_string(), Value::String("".to_string()));

    Value::Map(response)
}

/// Helper to create JSON response
fn json_response(data: Value, status: i64) -> Value {
    let mut headers = HashMap::new();
    headers.insert(
        "Content-Type".to_string(),
        Value::String("application/json".to_string()),
    );

    let json_val = value_to_json(&data);
    let body = serde_json::to_string(&json_val).unwrap_or_else(|_| "{}".to_string());

    let mut response = HashMap::new();
    response.insert("status".to_string(), Value::Int(status));
    response.insert("headers".to_string(), Value::Map(headers));
    response.insert("body".to_string(), Value::String(body));

    Value::Map(response)
}

/// Convert a Value::Map (from oauth()) to a ProviderConfig
fn value_map_to_provider(
    map: &HashMap<String, Value>,
) -> std::result::Result<ProviderConfig, String> {
    let get_str = |key: &str| -> Option<String> {
        map.get(key).and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        })
    };

    let get_bool = |key: &str| -> bool {
        map.get(key)
            .and_then(|v| match v {
                Value::Bool(b) => Some(*b),
                _ => None,
            })
            .unwrap_or(false)
    };

    let get_str_array = |key: &str| -> Vec<String> {
        map.get(key)
            .and_then(|v| match v {
                Value::Array(arr) => Some(
                    arr.iter()
                        .filter_map(|v| match v {
                            Value::String(s) => Some(s.clone()),
                            _ => None,
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_default()
    };

    let name = get_str("name").ok_or("Provider must have a 'name'")?;
    let authorize_url = get_str("authorize_url").ok_or("Provider must have 'authorize_url'")?;
    let token_url = get_str("token_url").ok_or("Provider must have 'token_url'")?;
    let client_id = get_str("client_id").ok_or("Provider must have 'client_id'")?;

    Ok(ProviderConfig {
        name,
        client_id,
        client_secret: get_str("client_secret").unwrap_or_default(),
        authorize_url,
        token_url,
        userinfo_url: get_str("userinfo_url").unwrap_or_default(),
        scopes: get_str_array("scopes"),
        supports_oidc: get_bool("supports_oidc"),
        issuer: get_str("issuer"),
        jwks_uri: get_str("jwks_uri"),
        use_pkce: get_bool("use_pkce"),
        extra_params: {
            let mut params = HashMap::new();
            // Parse extra_params from config if provided
            if let Some(Value::Map(ep)) = map.get("extra_params") {
                for (k, v) in ep {
                    if let Value::String(s) = v {
                        params.insert(k.clone(), s.clone());
                    }
                }
            }
            params
        },
    })
}

/// Handle GET /auth/{provider} - Start OAuth flow
pub fn handle_auth_start(args: &[Value]) -> Result<Value> {
    let req = &args[0];

    // Get provider name from route params
    let provider_name = if let Value::Map(req_map) = req {
        req_map.get("params").and_then(|p| {
            if let Value::Map(params) = p {
                params.get("provider").and_then(|v| {
                    if let Value::String(s) = v {
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
    };

    let provider_name = provider_name.ok_or_else(|| {
        IntentError::RuntimeError("[auth] No provider specified in /auth/{provider}".to_string())
    })?;

    // Get auth config
    let config = get_auth_config().ok_or_else(|| {
        IntentError::RuntimeError(
            "[auth] Auth not configured - call enable_auth() first".to_string(),
        )
    })?;

    // Find the provider
    let provider = config
        .providers
        .iter()
        .find(|p| p.name == provider_name)
        .ok_or_else(|| {
            let msg = if let Some(suggestion) = suggest_provider(&provider_name) {
                format!(
                    "[auth] Unknown provider \"{}\"\n       Did you mean \"{}\"?\n       Available providers: {}",
                    provider_name, suggestion, available_providers()
                )
            } else {
                format!(
                    "[auth] Unknown provider \"{}\"\n       Available providers: {}",
                    provider_name, available_providers()
                )
            };
            IntentError::RuntimeError(msg)
        })?;

    // Generate state for CSRF protection
    let state = generate_oauth_state();

    // Generate nonce for OIDC
    let nonce = if provider.supports_oidc {
        Some(generate_nonce())
    } else {
        None
    };

    // Generate PKCE if enabled
    let (pkce_verifier, pkce_challenge) = if provider.use_pkce {
        let verifier = generate_pkce_verifier();
        let challenge = generate_pkce_challenge(&verifier);
        (Some(verifier), Some(challenge))
    } else {
        (None, None)
    };

    // Determine redirect URI (provider-specific for GitHub compatibility)
    let (host, proto) = get_host_and_proto(req);
    let redirect_uri = format!("{}://{}/auth/{}/callback", proto, host, provider.name);

    // Store OAuth state
    store_oauth_state(
        &state,
        &provider.name,
        &redirect_uri,
        nonce.as_deref(),
        pkce_verifier.as_deref(),
    );

    // When store_tokens is enabled and provider is Google, ensure we request refresh tokens
    let mut provider_for_url = provider.clone();
    if config.store_tokens {
        if !provider_for_url.extra_params.contains_key("access_type") {
            if provider_for_url.authorize_url.contains("google") {
                provider_for_url
                    .extra_params
                    .insert("access_type".to_string(), "offline".to_string());
                // prompt=consent forces Google to return a new refresh token every time
                if !provider_for_url.extra_params.contains_key("prompt") {
                    provider_for_url
                        .extra_params
                        .insert("prompt".to_string(), "consent".to_string());
                }
            }
        }
    }

    // Generate auth URL
    let auth_url = generate_auth_url(
        &provider_for_url,
        &redirect_uri,
        &state,
        nonce.as_deref(),
        pkce_challenge.as_deref(),
    );

    Ok(redirect_response(&auth_url, None))
}

/// Handle GET /auth/callback - OAuth callback
pub fn handle_auth_callback(args: &[Value]) -> Result<Value> {
    let req = &args[0];

    let config = get_auth_config()
        .ok_or_else(|| IntentError::RuntimeError("[auth] Auth not configured".to_string()))?;

    // Extract code and state from query params
    let (code, state, error) = if let Value::Map(req_map) = req {
        if let Some(Value::Map(query)) = req_map.get("query_params") {
            let code = query.get("code").and_then(|v| {
                if let Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            });
            let state = query.get("state").and_then(|v| {
                if let Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            });
            let error = query.get("error").and_then(|v| {
                if let Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            });
            (code, state, error)
        } else {
            (None, None, None)
        }
    } else {
        (None, None, None)
    };

    // Check for OAuth error
    if let Some(err) = error {
        eprintln!("[auth] OAuth error: {}", err);
        return Ok(redirect_response(&config.failure_url, None));
    }

    // Validate state
    let oauth_state = state.as_ref().and_then(|s| consume_oauth_state(s));

    if oauth_state.is_none() || code.is_none() {
        eprintln!("[auth] Invalid callback - missing code or state");
        return Ok(redirect_response(&config.failure_url, None));
    }

    let oauth_state = oauth_state.unwrap();
    let code = code.unwrap();

    // Find the provider
    let provider = config
        .providers
        .iter()
        .find(|p| p.name == oauth_state.provider);
    if provider.is_none() {
        eprintln!("[auth] Provider not found: {}", oauth_state.provider);
        return Ok(redirect_response(&config.failure_url, None));
    }
    let provider = provider.unwrap();

    // Exchange code for tokens
    let tokens = match exchange_code_for_tokens(
        provider,
        &code,
        &oauth_state.redirect_url,
        oauth_state.pkce_verifier.as_deref(),
    ) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("{}", e);
            return Ok(redirect_response(&config.failure_url, None));
        }
    };

    // Get user info - prefer ID token for OIDC providers
    let user_info = if let Some(ref id_token) = tokens.id_token {
        // Decode and validate ID token
        match decode_id_token(id_token) {
            Ok(claims) => {
                // Validate ID token claims
                if let Err(e) = validate_id_token_claims(
                    &claims,
                    provider.issuer.as_deref(),
                    &provider.client_id,
                    oauth_state.nonce.as_deref(),
                ) {
                    eprintln!("{}", e);
                    return Ok(redirect_response(&config.failure_url, None));
                }
                claims
            }
            Err(e) => {
                eprintln!(
                    "[auth] ID token decode failed, falling back to userinfo: {}",
                    e
                );
                // Fall back to userinfo endpoint
                match fetch_userinfo(provider, &tokens.access_token) {
                    Ok(u) => u,
                    Err(e) => {
                        eprintln!("{}", e);
                        return Ok(redirect_response(&config.failure_url, None));
                    }
                }
            }
        }
    } else {
        // No ID token, use userinfo endpoint
        match fetch_userinfo(provider, &tokens.access_token) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("{}", e);
                return Ok(redirect_response(&config.failure_url, None));
            }
        }
    };

    // Create session (optionally storing tokens)
    let session = create_session(
        &provider.name,
        user_info,
        if config.store_tokens {
            Some(&tokens)
        } else {
            None
        },
        config.session_ttl,
    )
    .map_err(|e| IntentError::RuntimeError(format!("[auth] Failed to create session: {}", e)))?;
    let session_id = session.id.clone();
    store_session(session);

    // Sign the session ID with HMAC for tamper protection
    let signed_session_id = sign_session_id(&session_id, &config.session_secret);

    // Create session cookie
    // Cookie Max-Age uses refresh_ttl (not session_ttl) so the browser retains the cookie
    // long enough for server-side auto-refresh to work when the session expires.
    let cookie_max_age = if config.store_tokens && config.refresh_ttl > config.session_ttl {
        config.refresh_ttl
    } else {
        config.session_ttl
    };
    let cookie = format!(
        "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite={}{}",
        config.cookie_name,
        signed_session_id,
        cookie_max_age,
        config.cookie_same_site,
        if config.cookie_secure { "; Secure" } else { "" }
    );

    Ok(redirect_response(&config.success_url, Some(&cookie)))
}

/// Handle POST /auth/logout - Clear session
pub fn handle_auth_logout(args: &[Value]) -> Result<Value> {
    let req = &args[0];
    let config = get_auth_config().unwrap_or_default();

    if let Some(session_id) = get_session_id_from_request(req) {
        delete_session_by_id(&session_id);
    }

    let cookie = format!(
        "{}=; Path=/; Max-Age=0; HttpOnly; SameSite={}{}",
        config.cookie_name,
        config.cookie_same_site,
        if config.cookie_secure { "; Secure" } else { "" }
    );

    Ok(redirect_response(&config.logout_url, Some(&cookie)))
}

// ============================================================================
// SECTION 9: Provider Config Helpers
// ============================================================================

/// Convert ProviderConfig to Value::Map
pub fn provider_to_value(provider: &ProviderConfig) -> Value {
    let mut map = HashMap::new();
    map.insert("_provider".to_string(), Value::Bool(true));
    map.insert("name".to_string(), Value::String(provider.name.clone()));
    map.insert(
        "client_id".to_string(),
        Value::String(provider.client_id.clone()),
    );
    map.insert(
        "client_secret".to_string(),
        Value::String(provider.client_secret.clone()),
    );
    map.insert(
        "authorize_url".to_string(),
        Value::String(provider.authorize_url.clone()),
    );
    map.insert(
        "token_url".to_string(),
        Value::String(provider.token_url.clone()),
    );
    map.insert(
        "userinfo_url".to_string(),
        Value::String(provider.userinfo_url.clone()),
    );
    map.insert(
        "scopes".to_string(),
        Value::Array(
            provider
                .scopes
                .iter()
                .map(|s| Value::String(s.clone()))
                .collect(),
        ),
    );
    map.insert("use_pkce".to_string(), Value::Bool(provider.use_pkce));
    map.insert(
        "supports_oidc".to_string(),
        Value::Bool(provider.supports_oidc),
    );

    if let Some(ref issuer) = provider.issuer {
        map.insert("issuer".to_string(), Value::String(issuer.clone()));
    }
    if !provider.extra_params.is_empty() {
        let mut extra = HashMap::new();
        for (k, v) in &provider.extra_params {
            extra.insert(k.clone(), Value::String(v.clone()));
        }
        map.insert("extra_params".to_string(), Value::Map(extra));
    }
    Value::Map(map)
}

/// Parse ProviderConfig from Value::Map
pub fn value_to_provider(value: &Value) -> Result<ProviderConfig> {
    match value {
        Value::Map(map) => {
            let get_str = |key: &str| -> Result<String> {
                match map.get(key) {
                    Some(Value::String(s)) => Ok(s.clone()),
                    _ => Err(IntentError::TypeError(format!(
                        "Provider {} must be a string",
                        key
                    ))),
                }
            };

            let get_bool = |key: &str, default: bool| -> bool {
                match map.get(key) {
                    Some(Value::Bool(b)) => *b,
                    _ => default,
                }
            };

            let name = get_str("name")?;
            let client_id = get_str("client_id")?;
            let client_secret = get_str("client_secret")?;
            let authorize_url = get_str("authorize_url")?;
            let token_url = get_str("token_url")?;
            let userinfo_url = map
                .get("userinfo_url")
                .and_then(|v| {
                    if let Value::String(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            let issuer = map.get("issuer").and_then(|v| {
                if let Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            });

            let jwks_uri = map.get("jwks_uri").and_then(|v| {
                if let Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            });

            let scopes = match map.get("scopes") {
                Some(Value::Array(arr)) => arr
                    .iter()
                    .filter_map(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            };

            let extra_params = match map.get("extra_params") {
                Some(Value::Map(m)) => m
                    .iter()
                    .filter_map(|(k, v)| match v {
                        Value::String(s) => Some((k.clone(), s.clone())),
                        _ => None,
                    })
                    .collect(),
                _ => HashMap::new(),
            };

            Ok(ProviderConfig {
                name,
                client_id,
                client_secret,
                authorize_url,
                token_url,
                userinfo_url,
                issuer,
                jwks_uri,
                scopes,
                extra_params,
                use_pkce: get_bool("use_pkce", false),
                supports_oidc: get_bool("supports_oidc", false),
            })
        }
        _ => Err(IntentError::TypeError("Provider must be a map".to_string())),
    }
}

// ============================================================================
// SECTION 11: Module Initialization
// ============================================================================

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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
                        "[auth] oauth() requires at least a provider name".to_string()
                    ));
                }

                let provider_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::TypeError(
                        "[auth] oauth() first argument must be a provider name string".to_string()
                    )),
                };

                // Check second argument type to determine signature
                match args.get(1) {
                    Some(Value::String(client_id)) => {
                        // Signature: oauth(provider, client_id, client_secret, options?)
                        let client_secret = match args.get(2) {
                            Some(Value::String(s)) => s.clone(),
                            Some(_) => return Err(IntentError::TypeError(
                                "[auth] oauth() client_secret must be a string".to_string()
                            )),
                            None => String::new(), // Allow empty for PKCE public clients
                        };

                        let options = match args.get(3) {
                            Some(Value::Map(m)) => Some(m.clone()),
                            Some(_) => return Err(IntentError::TypeError(
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
                                return Err(IntentError::RuntimeError(msg));
                            };

                        // Check if PKCE is explicitly requested or required
                        let use_pkce = options
                            .as_ref()
                            .and_then(|o| o.get("use_pkce"))
                            .and_then(|v| match v { Value::Bool(b) => Some(*b), _ => None })
                            .unwrap_or(provider_name == "twitter"); // Twitter requires PKCE

                        if use_pkce && !supports_pkce {
                            return Err(IntentError::RuntimeError(format!(
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
                            IntentError::TypeError(format!(
                                "[auth] Custom provider \"{}\" missing required field \"client_id\"",
                                provider_name
                            ))
                        })?;

                        let client_secret = get_str("client_secret").unwrap_or_default();

                        let authorize_url = get_str("authorize_url").ok_or_else(|| {
                            IntentError::TypeError(format!(
                                "[auth] Custom provider \"{}\" missing required field \"authorize_url\"",
                                provider_name
                            ))
                        })?;

                        let token_url = get_str("token_url").ok_or_else(|| {
                            IntentError::TypeError(format!(
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
                    Some(_) => Err(IntentError::TypeError(
                        "[auth] oauth() second argument must be client_id (string) or config (map)".to_string()
                    )),
                    None => Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] oauth_discover() requires issuer and client_id".to_string(),
                    ));
                }

                let issuer = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] oauth_discover() issuer must be a string".to_string(),
                        ))
                    }
                };

                let client_id = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 4 {
                    return Err(IntentError::TypeError(
                        "[auth] oauth_m2m() requires token_url, client_id, client_secret, scopes"
                            .to_string(),
                    ));
                }

                let token_url = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] token_url must be a string".to_string(),
                        ))
                    }
                };
                let client_id = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] client_id must be a string".to_string(),
                        ))
                    }
                };
                let client_secret = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
                        "[auth] oauth_refresh() requires a request".to_string(),
                    ));
                }

                let config = get_auth_config().ok_or_else(|| {
                    IntentError::RuntimeError("[auth] Auth not configured".to_string())
                })?;

                let session_id = get_session_id_from_request(&args[0]).ok_or_else(|| {
                    IntentError::RuntimeError("[auth] No session found".to_string())
                })?;

                let session = get_session_by_id(&session_id).ok_or_else(|| {
                    IntentError::RuntimeError("[auth] Session expired".to_string())
                })?;

                let refresh_token = session.refresh_token.as_ref().ok_or_else(|| {
                    IntentError::RuntimeError(
                        "[auth] No refresh token stored (enable store_tokens in auth config)"
                            .to_string(),
                    )
                })?;

                let provider = config
                    .providers
                    .iter()
                    .find(|p| p.name == session.provider)
                    .ok_or_else(|| {
                        IntentError::RuntimeError(format!(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
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
                        return Err(IntentError::TypeError(
                            "[auth] token must be a string".to_string(),
                        ))
                    }
                };

                let options = match &args[1] {
                    Value::Map(m) => m.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 4 {
                    return Err(IntentError::TypeError(
                        "[auth] oauth_introspect() requires introspection_url, token, client_id, client_secret".to_string()
                    ));
                }

                let introspection_url = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::TypeError("[auth] introspection_url must be a string".to_string())),
                };
                let token = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::TypeError("[auth] token must be a string".to_string())),
                };
                let client_id = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::TypeError("[auth] client_id must be a string".to_string())),
                };
                let client_secret = match &args[3] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::TypeError("[auth] client_secret must be a string".to_string())),
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] set_session() requires request and data".to_string(),
                    ));
                }

                let data_map = match &args[1] {
                    Value::Map(m) => m.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
    // Clean up expired sessions and OAuth states from the session store.
    //
    // Call this periodically (e.g., via a cron job or scheduled task) to remove
    // expired sessions and OAuth states from the database. For Redis, sessions
    // use TTL so they expire automatically, but this will scan for any orphaned entries.
    // @returns Result containing the number of expired sessions removed, or error
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] logout_all() requires request and keep_current".to_string(),
                    ));
                }

                let keep_current = match &args[1] {
                    Value::Bool(b) => *b,
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] verify_csrf() requires request and token".to_string(),
                    ));
                }

                let submitted_token = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] jwt_sign() requires claims and secret".to_string(),
                    ));
                }

                let claims = match &args[0] {
                    Value::Map(m) => m.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] claims must be a map".to_string(),
                        ))
                    }
                };

                let secret = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] jwt_verify() requires token and secret".to_string(),
                    ));
                }

                let token = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] token must be a string".to_string(),
                        ))
                    }
                };

                let secret = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
                        "[auth] jwt_decode() requires a token".to_string(),
                    ));
                }

                let token = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
                        "[auth] logout_user() requires a request".to_string(),
                    ));
                }

                let config = get_auth_config().unwrap_or_default();

                if let Some(session_id) = get_session_id_from_request(&args[0]) {
                    delete_session_by_id(&session_id);
                }

                let cookie = format!(
                    "{}=; Path=/; Max-Age=0; HttpOnly; SameSite={}{}",
                    config.cookie_name,
                    config.cookie_same_site,
                    if config.cookie_secure { "; Secure" } else { "" }
                );

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
    // @param options Optional map with keys: session_secret, session_ttl, after_login, after_logout, session_store
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
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::TypeError(
                        "[auth] enable_auth() requires a providers array".to_string(),
                    ));
                }

                // Parse providers array
                let providers_arr = match &args[0] {
                    Value::Array(arr) => arr.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] enable_auth() first argument must be an array of providers"
                                .to_string(),
                        ))
                    }
                };

                let options = match args.get(1) {
                    Some(Value::Map(m)) => Some(m.clone()),
                    Some(_) => {
                        return Err(IntentError::TypeError(
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
                                IntentError::TypeError(format!(
                                    "[auth] Invalid provider at index {}: {}",
                                    idx, e
                                ))
                            })?;
                            providers.push(provider);
                        }
                        _ => {
                            return Err(IntentError::TypeError(format!(
                                "[auth] Provider at index {} must be a map (use oauth() to create)",
                                idx
                            )));
                        }
                    }
                }

                // Parse options
                let session_secret = options
                    .as_ref()
                    .and_then(|o| o.get("session_secret"))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| DEFAULT_SESSION_SECRET_SENTINEL.to_string());

                let session_ttl = options
                    .as_ref()
                    .and_then(|o| o.get("session_ttl"))
                    .and_then(|v| match v {
                        Value::Int(n) => Some(*n),
                        _ => None,
                    })
                    .unwrap_or(86400);

                let success_url = options
                    .as_ref()
                    .and_then(|o| o.get("after_login"))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "/".to_string());

                let failure_url = options
                    .as_ref()
                    .and_then(|o| o.get("after_failure"))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "/".to_string());

                let logout_url = options
                    .as_ref()
                    .and_then(|o| o.get("after_logout"))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| "/".to_string());

                // Check if we should use secure cookies
                // Default: true (HTTPS required) - must explicitly set NTNT_ENV=dev to allow HTTP
                let is_dev = std::env::var("NTNT_ENV")
                    .map(|v| v == "dev" || v == "development")
                    .unwrap_or(false); // Default to secure (production) mode

                let cookie_secure = options
                    .as_ref()
                    .and_then(|o| o.get("cookie_secure"))
                    .and_then(|v| match v {
                        Value::Bool(b) => Some(*b),
                        _ => None,
                    })
                    .unwrap_or(!is_dev); // Secure by default unless explicitly in dev mode

                // Parse session storage backend
                // Format: "memory" (default), "sqlite:./path.db", "postgres://...", "redis://..."
                let session_store = options
                    .as_ref()
                    .and_then(|o| o.get("session_store"))
                    .and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .map(|s| {
                        if s == "memory" || s.is_empty() {
                            SessionStore::Memory
                        } else if s.starts_with("sqlite:") {
                            let path = s.strip_prefix("sqlite:").unwrap_or("./sessions.db");
                            SessionStore::Sqlite(path.to_string())
                        } else if s.starts_with("postgres://") || s.starts_with("postgresql://") {
                            SessionStore::Postgres(s)
                        } else if s.starts_with("redis://") || s.starts_with("valkey://") {
                            SessionStore::Redis(s)
                        } else {
                            eprintln!("[auth] Unknown session_store format '{}', using memory", s);
                            SessionStore::Memory
                        }
                    })
                    .unwrap_or(SessionStore::Memory);

                // Initialize database/cache if needed
                match &session_store {
                    SessionStore::Sqlite(path) => {
                        if let Err(e) = init_sqlite_sessions(path) {
                            eprintln!("[auth] Failed to initialize SQLite sessions: {}", e);
                            return Err(IntentError::RuntimeError(format!(
                                "Failed to initialize SQLite session store: {}",
                                e
                            )));
                        }
                    }
                    SessionStore::Postgres(url) => {
                        if let Err(e) = init_postgres_sessions(url) {
                            eprintln!("[auth] Failed to initialize PostgreSQL sessions: {}", e);
                            return Err(IntentError::RuntimeError(format!(
                                "Failed to initialize PostgreSQL session store: {}",
                                e
                            )));
                        }
                    }
                    SessionStore::Redis(url) => {
                        if let Err(e) = init_redis_sessions(url) {
                            eprintln!("[auth] Failed to initialize Redis sessions: {}", e);
                            return Err(IntentError::RuntimeError(format!(
                                "Failed to initialize Redis session store: {}",
                                e
                            )));
                        }
                    }
                    SessionStore::Memory => {}
                }

                // Create auth config
                let config = AuthConfig {
                    providers,
                    success_url,
                    failure_url,
                    logout_url,
                    cookie_name: "ntnt_session".to_string(),
                    cookie_secure,
                    cookie_same_site: "Lax".to_string(),
                    session_ttl,
                    store_tokens: options
                        .as_ref()
                        .and_then(|o| o.get("store_tokens"))
                        .and_then(|v| match v {
                            Value::Bool(b) => Some(*b),
                            _ => None,
                        })
                        .unwrap_or(false),
                    refresh_ttl: options
                        .as_ref()
                        .and_then(|o| o.get("refresh_ttl"))
                        .and_then(|v| match v {
                            Value::Int(n) => Some(*n),
                            _ => None,
                        })
                        .unwrap_or(86400 * 30), // 30 days default
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
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
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 4 {
                    return Err(IntentError::TypeError(
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
                        return Err(IntentError::TypeError(
                            "[auth] code must be a string".to_string(),
                        ))
                    }
                };

                let state = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] state must be a string".to_string(),
                        ))
                    }
                };

                let redirect_uri = match &args[3] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] create_session_from_oauth() requires (provider_name, user_info, tokens?)".to_string()
                    ));
                }

                let config = match get_auth_config() {
                    Some(c) => c,
                    None => return Ok(make_err(Value::String("Auth not initialized. Call enable_auth() first.".to_string()))),
                };

                let provider_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => return Err(IntentError::TypeError("[auth] provider_name must be a string".to_string())),
                };

                let user_info = match &args[1] {
                    Value::Map(m) => m.clone(),
                    _ => return Err(IntentError::TypeError("[auth] user_info must be a map".to_string())),
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

                // Build cookie — use refresh_ttl for Max-Age when refresh tokens are enabled
                let signed_session_id = sign_session_id(&session_id, &config.session_secret);
                let cookie_max_age = if config.store_tokens && config.refresh_ttl > config.session_ttl {
                    config.refresh_ttl
                } else {
                    config.session_ttl
                };
                let cookie = format!(
                    "{}={}; Path=/; Max-Age={}; HttpOnly; SameSite={}{}",
                    config.cookie_name,
                    signed_session_id,
                    cookie_max_age,
                    config.cookie_same_site,
                    if config.cookie_secure { "; Secure" } else { "" }
                );

                // Return result
                let mut result = HashMap::new();
                result.insert("session_id".to_string(), Value::String(session_id));
                result.insert("user_id".to_string(), Value::String(user_id));
                result.insert("cookie".to_string(), Value::String(cookie));

                Ok(make_ok(Value::Map(result)))
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

    // @ntnt hash_password
    // @module std/auth
    // @signature hash_password(password: String) -> Result<String, String>
    // Hash a password using bcrypt.
    //
    // Utility function to hash passwords for custom storage or verification.
    // Uses bcrypt with default cost factor.
    // @param password The password to hash
    // @returns Ok(hash) on success, Err(message) on failure
    // @see_also verify_password
    // @since v0.3.11
    // @tags #auth, #utility
    // @example hash_password("mypassword") => Ok("$2b$12$...") ~ "Hash a password"
    module.insert(
        "hash_password".to_string(),
        Value::NativeFunction {
            name: "hash_password".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                eprintln!("[DEPRECATED] hash_password() in std/auth is deprecated. Use hash_password() from std/crypto instead.");
                if args.is_empty() {
                    return Err(IntentError::TypeError(
                        "[auth] hash_password() requires a password".to_string(),
                    ));
                }

                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] password must be a string".to_string(),
                        ))
                    }
                };

                match hash_password(&password) {
                    Ok(hash) => Ok(make_ok(Value::String(hash))),
                    Err(e) => Ok(make_err(Value::String(e))),
                }
            },
        },
    );

    // @ntnt verify_password
    // @module std/auth
    // @signature verify_password(password: String, hash: String) -> Bool
    // Verify a password against a bcrypt hash.
    //
    // Utility function to verify passwords hashed with hash_password.
    // @param password The password to verify
    // @param hash The bcrypt hash to verify against
    // @returns true if password matches hash, false otherwise
    // @see_also hash_password
    // @since v0.3.11
    // @tags #auth, #utility
    // @example verify_password("mypassword", stored_hash) => true ~ "Verify password"
    module.insert(
        "verify_password".to_string(),
        Value::NativeFunction {
            name: "verify_password".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| {
                eprintln!("[DEPRECATED] verify_password() in std/auth is deprecated. Use verify_password() from std/crypto instead.");
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] verify_password() requires (password, hash)".to_string(),
                    ));
                }

                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] password must be a string".to_string(),
                        ))
                    }
                };

                let hash = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] hash must be a string".to_string(),
                        ))
                    }
                };

                Ok(Value::Bool(verify_password_hash(&password, &hash)))
            },
        },
    );

    // ==========================================================================
    // MFA/2FA Functions
    // ==========================================================================

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
            func: |args| {
                if args.len() < 3 {
                    return Err(IntentError::TypeError(
                        "[auth] totp_uri() requires (secret, email, issuer)".to_string(),
                    ));
                }

                let secret = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] secret must be a string".to_string(),
                        ))
                    }
                };

                let email = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] email must be a string".to_string(),
                        ))
                    }
                };

                let issuer = match &args[2] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| {
                if args.len() < 2 {
                    return Err(IntentError::TypeError(
                        "[auth] verify_totp() requires (secret, code)".to_string(),
                    ));
                }

                let secret = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "[auth] secret must be a string".to_string(),
                        ))
                    }
                };

                let code = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
