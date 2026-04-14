use super::ProviderConfig;
use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use std::collections::HashMap;

/// Built-in provider template (without credentials)
pub(super) struct BuiltinProvider {
    pub(super) name: &'static str,
    pub(super) authorize_url: &'static str,
    pub(super) token_url: &'static str,
    pub(super) userinfo_url: &'static str,
    pub(super) issuer: Option<&'static str>,
    pub(super) default_scopes: &'static [&'static str],
    pub(super) supports_oidc: bool,
    pub(super) supports_pkce: bool,
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
        supports_pkce: true,
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
        userinfo_url: "",
        issuer: Some("https://appleid.apple.com"),
        default_scopes: &["name", "email"],
        supports_oidc: true,
        supports_pkce: false,
    },
    BuiltinProvider {
        name: "okta",
        authorize_url: "",
        token_url: "",
        userinfo_url: "",
        issuer: None,
        default_scopes: &["openid", "email", "profile"],
        supports_oidc: true,
        supports_pkce: true,
    },
    BuiltinProvider {
        name: "auth0",
        authorize_url: "",
        token_url: "",
        userinfo_url: "",
        issuer: None,
        default_scopes: &["openid", "email", "profile"],
        supports_oidc: true,
        supports_pkce: true,
    },
];

pub(super) fn get_builtin_provider(name: &str) -> Option<&'static BuiltinProvider> {
    BUILTIN_PROVIDERS
        .iter()
        .find(|provider| provider.name == name)
}

pub(super) fn available_providers() -> String {
    BUILTIN_PROVIDERS
        .iter()
        .filter(|provider| !provider.authorize_url.is_empty())
        .map(|provider| provider.name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn suggest_provider(name: &str) -> Option<&'static str> {
    BUILTIN_PROVIDERS
        .iter()
        .map(|provider| provider.name)
        .min_by_key(|provider| levenshtein_distance(name, provider))
        .filter(|provider| levenshtein_distance(name, provider) <= 2)
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

pub(super) fn is_safe_provider_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == '.')
}

pub(super) fn validate_provider_name(name: &str) -> std::result::Result<(), String> {
    if is_safe_provider_name(name) {
        Ok(())
    } else {
        Err(
            "provider name must use only ASCII letters, numbers, periods, underscores, or hyphens"
                .to_string(),
        )
    }
}

fn get_str(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    map.get(key).and_then(|value| match value {
        Value::String(string) => Some(string.clone()),
        _ => None,
    })
}

fn get_bool(map: &HashMap<String, Value>, key: &str, default: bool) -> bool {
    match map.get(key) {
        Some(Value::Bool(value)) => *value,
        _ => default,
    }
}

fn get_str_array(map: &HashMap<String, Value>, key: &str) -> Vec<String> {
    map.get(key)
        .and_then(|value| match value {
            Value::Array(values) => Some(
                values
                    .iter()
                    .filter_map(|value| match value {
                        Value::String(string) => Some(string.clone()),
                        _ => None,
                    })
                    .collect(),
            ),
            _ => None,
        })
        .unwrap_or_default()
}

pub(super) fn value_map_to_provider(
    map: &HashMap<String, Value>,
) -> std::result::Result<ProviderConfig, String> {
    let name = get_str(map, "name").ok_or("Provider must have a 'name'")?;
    let authorize_url =
        get_str(map, "authorize_url").ok_or("Provider must have 'authorize_url'")?;
    let token_url = get_str(map, "token_url").ok_or("Provider must have 'token_url'")?;
    let client_id = get_str(map, "client_id").ok_or("Provider must have 'client_id'")?;

    validate_provider_name(&name)?;

    Ok(ProviderConfig {
        name,
        client_id,
        client_secret: get_str(map, "client_secret").unwrap_or_default(),
        authorize_url,
        token_url,
        userinfo_url: get_str(map, "userinfo_url").unwrap_or_default(),
        scopes: get_str_array(map, "scopes"),
        supports_oidc: get_bool(map, "supports_oidc", false),
        issuer: get_str(map, "issuer"),
        jwks_uri: get_str(map, "jwks_uri"),
        use_pkce: get_bool(map, "use_pkce", false),
        extra_params: {
            let mut params = HashMap::new();
            if let Some(Value::Map(extra_params)) = map.get("extra_params") {
                for (key, value) in extra_params {
                    if let Value::String(string) = value {
                        params.insert(key.clone(), string.clone());
                    }
                }
            }
            params
        },
    })
}

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
                .map(|scope| Value::String(scope.clone()))
                .collect(),
        ),
    );
    map.insert("use_pkce".to_string(), Value::Bool(provider.use_pkce));
    map.insert(
        "supports_oidc".to_string(),
        Value::Bool(provider.supports_oidc),
    );

    if let Some(issuer) = &provider.issuer {
        map.insert("issuer".to_string(), Value::String(issuer.clone()));
    }
    if !provider.extra_params.is_empty() {
        let mut extra = HashMap::new();
        for (key, value) in &provider.extra_params {
            extra.insert(key.clone(), Value::String(value.clone()));
        }
        map.insert("extra_params".to_string(), Value::Map(extra));
    }

    Value::Map(map)
}

pub fn value_to_provider(value: &Value) -> Result<ProviderConfig> {
    match value {
        Value::Map(map) => {
            let get_required_str = |key: &str| -> Result<String> {
                match map.get(key) {
                    Some(Value::String(string)) => Ok(string.clone()),
                    _ => Err(IntentError::type_error(format!(
                        "Provider {} must be a string",
                        key
                    ))),
                }
            };

            let name = get_required_str("name")?;
            validate_provider_name(&name).map_err(IntentError::type_error)?;
            let client_id = get_required_str("client_id")?;
            let client_secret = get_required_str("client_secret")?;
            let authorize_url = get_required_str("authorize_url")?;
            let token_url = get_required_str("token_url")?;
            let userinfo_url = get_str(map, "userinfo_url").unwrap_or_default();
            let issuer = get_str(map, "issuer");
            let jwks_uri = get_str(map, "jwks_uri");
            let scopes = match map.get("scopes") {
                Some(Value::Array(values)) => values
                    .iter()
                    .filter_map(|value| match value {
                        Value::String(string) => Some(string.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => vec![],
            };
            let extra_params = match map.get("extra_params") {
                Some(Value::Map(extra)) => extra
                    .iter()
                    .filter_map(|(key, value)| match value {
                        Value::String(string) => Some((key.clone(), string.clone())),
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
                use_pkce: get_bool(map, "use_pkce", false),
                supports_oidc: get_bool(map, "supports_oidc", false),
            })
        }
        _ => Err(IntentError::type_error(
            "Provider must be a map".to_string(),
        )),
    }
}
