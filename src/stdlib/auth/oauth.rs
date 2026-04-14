use super::*;

const NTNT_USER_AGENT: &str = concat!("NTNT/", env!("CARGO_PKG_VERSION"));

/// Extract user info from provider response or ID token
pub(super) fn extract_user_info(
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
        .header("User-Agent", NTNT_USER_AGENT) // Required by GitHub
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
        .header("User-Agent", NTNT_USER_AGENT) // Required by GitHub
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
