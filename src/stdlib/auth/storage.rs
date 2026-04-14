use super::*;

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

// ============================================================================
// Exchange Token Store/Consume (Safari ITP workaround)
// ============================================================================

/// Maximum lifetime of an exchange token in seconds.
/// Tokens older than this are considered expired and will not be consumed.
pub(super) const EXCHANGE_TOKEN_TTL: i64 = 60;
pub(super) const AUTH_CHALLENGE_TTL: i64 = 1800;

/// Store an exchange token mapping to a session ID.
/// Used to break the OAuth redirect chain for Safari ITP cookie persistence.
pub(super) fn store_exchange_token(token: &str, session_id: &str) {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            if let Err(e) = store_exchange_token_sqlite(token, session_id) {
                eprintln!(
                    "[auth] SQLite exchange token store failed, using memory: {}",
                    e
                );
                SESSION_STORE
                    .lock()
                    .unwrap()
                    .set_exchange_token(token.to_string(), session_id.to_string());
            }
        }
        Some(SessionStore::Postgres(_)) => {
            if let Err(e) = store_exchange_token_postgres(token, session_id) {
                eprintln!(
                    "[auth] PostgreSQL exchange token store failed, using memory: {}",
                    e
                );
                SESSION_STORE
                    .lock()
                    .unwrap()
                    .set_exchange_token(token.to_string(), session_id.to_string());
            }
        }
        Some(SessionStore::Redis(_)) => {
            if let Err(e) = store_exchange_token_redis(token, session_id) {
                eprintln!(
                    "[auth] Redis exchange token store failed, using memory: {}",
                    e
                );
                SESSION_STORE
                    .lock()
                    .unwrap()
                    .set_exchange_token(token.to_string(), session_id.to_string());
            }
        }
        _ => {
            SESSION_STORE
                .lock()
                .unwrap()
                .set_exchange_token(token.to_string(), session_id.to_string());
        }
    }
}

fn store_exchange_token_sqlite(token: &str, session_id: &str) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();

    conn.execute(
        "INSERT OR REPLACE INTO auth_exchange_tokens (token, session_id, created_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![token, session_id, now],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn store_exchange_token_postgres(token: &str, session_id: &str) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();

    client
        .execute(
            "INSERT INTO auth_exchange_tokens (token, session_id, created_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (token) DO UPDATE SET session_id = $2, created_at = $3",
            &[&token, &session_id, &now],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn store_exchange_token_redis(token: &str, session_id: &str) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = format!("ntnt:auth_exchange:{}", token);
    redis::cmd("SETEX")
        .arg(&key)
        .arg(EXCHANGE_TOKEN_TTL)
        .arg(session_id)
        .query::<()>(&mut conn)
        .map_err(|e| format!("Redis SETEX error: {}", e))?;

    Ok(())
}

/// Consume an exchange token, returning the associated session ID.
/// The token is deleted after retrieval (one-time use).
pub(super) fn consume_exchange_token(token: &str) -> Option<String> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => consume_exchange_token_sqlite(token)
            .ok()
            .flatten()
            .or_else(|| {
                let mut store = SESSION_STORE.lock().unwrap();
                let session_id = store.get_exchange_token(token).cloned();
                if session_id.is_some() {
                    store.delete_exchange_token(token);
                }
                session_id
            }),
        Some(SessionStore::Postgres(_)) => consume_exchange_token_postgres(token)
            .ok()
            .flatten()
            .or_else(|| {
                let mut store = SESSION_STORE.lock().unwrap();
                let session_id = store.get_exchange_token(token).cloned();
                if session_id.is_some() {
                    store.delete_exchange_token(token);
                }
                session_id
            }),
        Some(SessionStore::Redis(_)) => {
            consume_exchange_token_redis(token)
                .ok()
                .flatten()
                .or_else(|| {
                    let mut store = SESSION_STORE.lock().unwrap();
                    let session_id = store.get_exchange_token(token).cloned();
                    if session_id.is_some() {
                        store.delete_exchange_token(token);
                    }
                    session_id
                })
        }
        _ => {
            let mut store = SESSION_STORE.lock().unwrap();
            let session_id = store.get_exchange_token(token).cloned();
            if session_id.is_some() {
                store.delete_exchange_token(token);
            }
            session_id
        }
    }
}

fn consume_exchange_token_sqlite(token: &str) -> std::result::Result<Option<String>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();
    let min_created = now - EXCHANGE_TOKEN_TTL;

    // Atomic delete-and-return: DELETE the token and return session_id in one statement.
    // This prevents race conditions where two concurrent requests could both consume the same token.
    // Requires SQLite 3.35.0+ (2021-03-12) — always available since ntnt bundles SQLite via rusqlite.
    let result = conn.query_row(
        "DELETE FROM auth_exchange_tokens
         WHERE token = ?1 AND created_at > ?2
         RETURNING session_id",
        rusqlite::params![token, min_created],
        |row| row.get::<_, String>(0),
    );

    match result {
        Ok(session_id) => Ok(Some(session_id)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn consume_exchange_token_postgres(token: &str) -> std::result::Result<Option<String>, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let now = chrono::Utc::now().timestamp();
    let min_created = now - EXCHANGE_TOKEN_TTL;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    // Atomic delete-and-return: DELETE the token and return session_id in one statement.
    // This prevents race conditions where two concurrent requests could both consume the same token.
    let rows = client
        .query(
            "DELETE FROM auth_exchange_tokens
             WHERE token = $1 AND created_at > $2
             RETURNING session_id",
            &[&token, &min_created],
        )
        .map_err(|e| e.to_string())?;

    if let Some(row) = rows.first() {
        Ok(Some(row.get::<_, String>(0)))
    } else {
        Ok(None)
    }
}

fn consume_exchange_token_redis(token: &str) -> std::result::Result<Option<String>, String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = format!("ntnt:auth_exchange:{}", token);

    // Use GETDEL for atomic get-and-delete (Redis 6.2+)
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

    Ok(result)
}

/// Create a session from OAuth user info
pub(super) fn create_session(
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

pub(super) fn create_auth_challenge(
    challenge_spec: &HashMap<String, Value>,
) -> std::result::Result<AuthChallenge, String> {
    let subject_id = match challenge_spec.get("subject_id") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err("[auth] begin_auth_challenge() subject_id must not be empty".to_string())
        }
        Some(other) => {
            return Err(format!(
                "[auth] begin_auth_challenge() subject_id must be a string, got {}",
                other.type_name()
            ))
        }
        None => {
            return Err(
                "[auth] begin_auth_challenge() challenge.subject_id is required".to_string(),
            )
        }
    };

    let provider = match challenge_spec.get("provider") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err("[auth] begin_auth_challenge() provider must not be empty".to_string())
        }
        Some(other) => {
            return Err(format!(
                "[auth] begin_auth_challenge() provider must be a string, got {}",
                other.type_name()
            ))
        }
        None => "local".to_string(),
    };
    validate_provider_name(&provider)
        .map_err(|e| format!("[auth] begin_auth_challenge() {}", e))?;

    let kind = match challenge_spec.get("kind") {
        Some(Value::String(s)) => validate_auth_challenge_kind(s)?,
        Some(other) => {
            return Err(format!(
                "[auth] begin_auth_challenge() kind must be a string, got {}",
                other.type_name()
            ))
        }
        None => return Err("[auth] begin_auth_challenge() challenge.kind is required".to_string()),
    };

    let ttl = match challenge_spec.get("ttl") {
        Some(Value::Int(i)) if *i > 0 => *i,
        Some(Value::Int(_)) => {
            return Err("[auth] begin_auth_challenge() ttl must be greater than 0".to_string())
        }
        Some(other) => {
            return Err(format!(
                "[auth] begin_auth_challenge() ttl must be an int, got {}",
                other.type_name()
            ))
        }
        None => AUTH_CHALLENGE_TTL,
    };

    let data_map = match challenge_spec.get("data") {
        Some(Value::Map(map)) => map.clone(),
        Some(other) => {
            return Err(format!(
                "[auth] begin_auth_challenge() data must be a map, got {}",
                other.type_name()
            ))
        }
        None => HashMap::new(),
    };

    let now = chrono::Utc::now().timestamp();
    Ok(AuthChallenge {
        id: generate_session_id(),
        subject_id,
        provider,
        kind,
        data_json: value_map_to_json_string(&data_map),
        created_at: now,
        expires_at: now + ttl,
    })
}

pub(super) fn create_manual_session(
    session_spec: &HashMap<String, Value>,
    ttl: i64,
) -> std::result::Result<Session, String> {
    let subject_id = match session_spec.get("subject_id") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err("[auth] sign_in_session() subject_id must not be empty".to_string())
        }
        Some(other) => {
            return Err(format!(
                "[auth] sign_in_session() subject_id must be a string, got {}",
                other.type_name()
            ))
        }
        None => return Err("[auth] sign_in_session() session.subject_id is required".to_string()),
    };

    let provider = match session_spec.get("provider") {
        Some(Value::String(s)) if !s.is_empty() => s.clone(),
        Some(Value::String(_)) => {
            return Err("[auth] sign_in_session() provider must not be empty".to_string())
        }
        Some(other) => {
            return Err(format!(
                "[auth] sign_in_session() provider must be a string, got {}",
                other.type_name()
            ))
        }
        None => "local".to_string(),
    };
    validate_provider_name(&provider)?;

    let get_optional_string = |key: &str| -> std::result::Result<Option<String>, String> {
        match session_spec.get(key) {
            Some(Value::String(s)) => Ok(Some(s.clone())),
            Some(other) => Err(format!(
                "[auth] sign_in_session() {} must be a string, got {}",
                key,
                other.type_name()
            )),
            None => Ok(None),
        }
    };

    let mut data_map = HashMap::new();
    for key in ["claims", "data"] {
        match session_spec.get(key) {
            Some(Value::Map(map)) => data_map.extend(map.clone()),
            Some(other) => {
                return Err(format!(
                    "[auth] sign_in_session() {} must be a map, got {}",
                    key,
                    other.type_name()
                ))
            }
            None => {}
        }
    }

    let raw_map = match session_spec.get("raw") {
        Some(Value::Map(map)) => map.clone(),
        Some(other) => {
            return Err(format!(
                "[auth] sign_in_session() raw must be a map, got {}",
                other.type_name()
            ))
        }
        None => {
            let mut raw = HashMap::new();
            raw.insert("subject_id".to_string(), Value::String(subject_id.clone()));
            raw
        }
    };

    let user_id = if subject_id.starts_with(&format!("{}:", provider)) {
        subject_id.clone()
    } else {
        format!("{}:{}", provider, subject_id)
    };

    let now = chrono::Utc::now().timestamp();
    Ok(Session {
        id: generate_session_id(),
        user_id,
        provider,
        email: get_optional_string("email")?,
        name: get_optional_string("name")?,
        picture: get_optional_string("picture")?,
        raw_json: value_map_to_json_string(&raw_map),
        data_json: value_map_to_json_string(&data_map),
        csrf_token: uuid::Uuid::new_v4().to_string(),
        access_token: None,
        refresh_token: None,
        token_expires_at: None,
        created_at: now,
        expires_at: now + ttl,
    })
}

pub fn store_auth_challenge(challenge: AuthChallenge) {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            if let Err(e) = store_auth_challenge_sqlite(&challenge) {
                eprintln!("[auth] WARNING: SQLite auth challenge store failed: {}", e);
                SESSION_STORE.lock().unwrap().set_auth_challenge(challenge);
            }
        }
        Some(SessionStore::Postgres(_)) => {
            if let Err(e) = store_auth_challenge_postgres(&challenge) {
                eprintln!(
                    "[auth] WARNING: PostgreSQL auth challenge store failed: {}",
                    e
                );
                SESSION_STORE.lock().unwrap().set_auth_challenge(challenge);
            }
        }
        Some(SessionStore::Redis(_)) => {
            if let Err(e) = store_auth_challenge_redis(&challenge) {
                eprintln!("[auth] WARNING: Redis auth challenge store failed: {}", e);
                SESSION_STORE.lock().unwrap().set_auth_challenge(challenge);
            }
        }
        _ => {
            SESSION_STORE.lock().unwrap().set_auth_challenge(challenge);
        }
    }
}

pub(super) fn store_auth_challenge_sqlite(
    challenge: &AuthChallenge,
) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    conn.execute(
        "INSERT OR REPLACE INTO auth_challenges
         (id, subject_id, provider, kind, data_json, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            challenge.id,
            challenge.subject_id,
            challenge.provider,
            challenge.kind,
            challenge.data_json,
            challenge.created_at,
            challenge.expires_at,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn store_auth_challenge_postgres(challenge: &AuthChallenge) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    client
        .execute(
            "INSERT INTO auth_challenges
             (id, subject_id, provider, kind, data_json, created_at, expires_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (id) DO UPDATE SET
                subject_id = $2, provider = $3, kind = $4, data_json = $5, created_at = $6, expires_at = $7",
            &[
                &challenge.id,
                &challenge.subject_id,
                &challenge.provider,
                &challenge.kind,
                &challenge.data_json,
                &challenge.created_at,
                &challenge.expires_at,
            ],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn auth_challenge_redis_key(id: &str) -> String {
    format!("ntnt:auth_challenge:{}", id)
}

pub(super) fn auth_challenge_from_json_str(
    json_str: &str,
) -> std::result::Result<AuthChallenge, String> {
    let json: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Auth challenge JSON parse error: {}", e))?;

    let get_string = |field: &str| -> std::result::Result<String, String> {
        json[field]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| format!("Auth challenge JSON missing string field: {}", field))
    };

    let get_i64 = |field: &str| -> std::result::Result<i64, String> {
        json[field]
            .as_i64()
            .ok_or_else(|| format!("Auth challenge JSON missing integer field: {}", field))
    };

    Ok(AuthChallenge {
        id: get_string("id")?,
        subject_id: get_string("subject_id")?,
        provider: get_string("provider")?,
        kind: get_string("kind")?,
        data_json: get_string("data_json")?,
        created_at: get_i64("created_at")?,
        expires_at: get_i64("expires_at")?,
    })
}

fn store_auth_challenge_redis(challenge: &AuthChallenge) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let challenge_json = serde_json::json!({
        "id": challenge.id,
        "subject_id": challenge.subject_id,
        "provider": challenge.provider,
        "kind": challenge.kind,
        "data_json": challenge.data_json,
        "created_at": challenge.created_at,
        "expires_at": challenge.expires_at,
    })
    .to_string();

    let key = auth_challenge_redis_key(&challenge.id);
    let ttl = challenge.expires_at - chrono::Utc::now().timestamp();

    if ttl <= 0 {
        return Err(format!(
            "Redis auth challenge TTL already expired for {}",
            challenge.id
        ));
    }

    redis::cmd("SETEX")
        .arg(&key)
        .arg(ttl)
        .arg(&challenge_json)
        .query::<()>(&mut conn)
        .map_err(|e| format!("Redis SETEX error: {}", e))?;

    Ok(())
}

fn get_auth_challenge_memory(id: &str) -> Option<AuthChallenge> {
    let now = chrono::Utc::now().timestamp();
    let mut store = SESSION_STORE.lock().unwrap();

    match store.auth_challenges.get(id) {
        Some(challenge) if challenge.expires_at > now => Some(challenge.clone()),
        Some(_) => {
            store.auth_challenges.remove(id);
            None
        }
        None => None,
    }
}

pub fn get_auth_challenge_by_id(id: &str) -> std::result::Result<Option<AuthChallenge>, String> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => match get_auth_challenge_sqlite(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(get_auth_challenge_memory(id)),
            Err(e) => get_auth_challenge_memory(id).map(Some).ok_or(e),
        },
        Some(SessionStore::Postgres(_)) => match get_auth_challenge_postgres(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(get_auth_challenge_memory(id)),
            Err(e) => get_auth_challenge_memory(id).map(Some).ok_or(e),
        },
        Some(SessionStore::Redis(_)) => match get_auth_challenge_redis(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(get_auth_challenge_memory(id)),
            Err(e) => get_auth_challenge_memory(id).map(Some).ok_or(e),
        },
        _ => Ok(get_auth_challenge_memory(id)),
    }
}

fn get_auth_challenge_sqlite(id: &str) -> std::result::Result<Option<AuthChallenge>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();

    let result = conn.query_row(
        "SELECT id, subject_id, provider, kind, data_json, created_at, expires_at
         FROM auth_challenges WHERE id = ?1 AND expires_at > ?2",
        rusqlite::params![id, now],
        |row| {
            Ok(AuthChallenge {
                id: row.get(0)?,
                subject_id: row.get(1)?,
                provider: row.get(2)?,
                kind: row.get(3)?,
                data_json: row.get(4)?,
                created_at: row.get(5)?,
                expires_at: row.get(6)?,
            })
        },
    );

    match result {
        Ok(challenge) => Ok(Some(challenge)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn get_auth_challenge_postgres(id: &str) -> std::result::Result<Option<AuthChallenge>, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let now = chrono::Utc::now().timestamp();

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    let rows = client
        .query(
            "SELECT id, subject_id, provider, kind, data_json, created_at, expires_at
             FROM auth_challenges WHERE id = $1 AND expires_at > $2",
            &[&id, &now],
        )
        .map_err(|e| e.to_string())?;

    if let Some(row) = rows.first() {
        Ok(Some(AuthChallenge {
            id: row.get(0),
            subject_id: row.get(1),
            provider: row.get(2),
            kind: row.get(3),
            data_json: row.get(4),
            created_at: row.get(5),
            expires_at: row.get(6),
        }))
    } else {
        Ok(None)
    }
}

fn get_auth_challenge_redis(id: &str) -> std::result::Result<Option<AuthChallenge>, String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = auth_challenge_redis_key(id);
    let result: Option<String> = redis::cmd("GET")
        .arg(&key)
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;

    let Some(json_str) = result else {
        return Ok(None);
    };

    let challenge = auth_challenge_from_json_str(&json_str)?;

    if challenge.expires_at > chrono::Utc::now().timestamp() {
        Ok(Some(challenge))
    } else {
        Ok(None)
    }
}

pub fn delete_auth_challenge_by_id(id: &str) {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            let _ = delete_auth_challenge_sqlite(id);
        }
        Some(SessionStore::Postgres(_)) => {
            let _ = delete_auth_challenge_postgres(id);
        }
        Some(SessionStore::Redis(_)) => {
            let _ = delete_auth_challenge_redis(id);
        }
        _ => {}
    }

    SESSION_STORE.lock().unwrap().delete_auth_challenge(id);
}

fn delete_auth_challenge_sqlite(id: &str) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.execute(
        "DELETE FROM auth_challenges WHERE id = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_auth_challenge_postgres(id: &str) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    client
        .execute("DELETE FROM auth_challenges WHERE id = $1", &[&id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

fn delete_auth_challenge_redis(id: &str) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = auth_challenge_redis_key(id);
    redis::cmd("DEL")
        .arg(&key)
        .query::<()>(&mut conn)
        .map_err(|e| format!("Redis DEL error: {}", e))?;
    Ok(())
}

pub(super) fn consume_auth_challenge(
    id: &str,
) -> std::result::Result<Option<AuthChallenge>, String> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => match consume_auth_challenge_sqlite(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(SESSION_STORE.lock().unwrap().take_auth_challenge(id)),
            Err(e) => match SESSION_STORE.lock().unwrap().take_auth_challenge(id) {
                Some(challenge) => Ok(Some(challenge)),
                None => Err(e),
            },
        },
        Some(SessionStore::Postgres(_)) => match consume_auth_challenge_postgres(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(SESSION_STORE.lock().unwrap().take_auth_challenge(id)),
            Err(e) => match SESSION_STORE.lock().unwrap().take_auth_challenge(id) {
                Some(challenge) => Ok(Some(challenge)),
                None => Err(e),
            },
        },
        Some(SessionStore::Redis(_)) => match consume_auth_challenge_redis(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(SESSION_STORE.lock().unwrap().take_auth_challenge(id)),
            Err(e) => match SESSION_STORE.lock().unwrap().take_auth_challenge(id) {
                Some(challenge) => Ok(Some(challenge)),
                None => Err(e),
            },
        },
        _ => Ok(SESSION_STORE.lock().unwrap().take_auth_challenge(id)),
    }
}

fn consume_auth_challenge_sqlite(id: &str) -> std::result::Result<Option<AuthChallenge>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();

    let result = conn.query_row(
        "DELETE FROM auth_challenges
         WHERE id = ?1 AND expires_at > ?2
         RETURNING id, subject_id, provider, kind, data_json, created_at, expires_at",
        rusqlite::params![id, now],
        |row| {
            Ok(AuthChallenge {
                id: row.get(0)?,
                subject_id: row.get(1)?,
                provider: row.get(2)?,
                kind: row.get(3)?,
                data_json: row.get(4)?,
                created_at: row.get(5)?,
                expires_at: row.get(6)?,
            })
        },
    );

    match result {
        Ok(challenge) => Ok(Some(challenge)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

fn consume_auth_challenge_postgres(id: &str) -> std::result::Result<Option<AuthChallenge>, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;
    let now = chrono::Utc::now().timestamp();

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    let rows = client
        .query(
            "DELETE FROM auth_challenges
             WHERE id = $1 AND expires_at > $2
             RETURNING id, subject_id, provider, kind, data_json, created_at, expires_at",
            &[&id, &now],
        )
        .map_err(|e| e.to_string())?;

    if let Some(row) = rows.first() {
        Ok(Some(AuthChallenge {
            id: row.get(0),
            subject_id: row.get(1),
            provider: row.get(2),
            kind: row.get(3),
            data_json: row.get(4),
            created_at: row.get(5),
            expires_at: row.get(6),
        }))
    } else {
        Ok(None)
    }
}

fn consume_auth_challenge_redis(id: &str) -> std::result::Result<Option<AuthChallenge>, String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let key = auth_challenge_redis_key(id);
    let now = chrono::Utc::now().timestamp();
    let lua_script = r#"
        local existing = redis.call('GET', KEYS[1])
        if not existing then return nil end
        local ok, decoded = pcall(cjson.decode, existing)
        if not ok or not decoded then return nil end
        local expires_at = tonumber(decoded['expires_at'])
        local now = tonumber(ARGV[1])
        if not expires_at or expires_at <= now then return nil end
        redis.call('DEL', KEYS[1])
        return existing
    "#;

    let json_str: Option<String> = redis::Script::new(lua_script)
        .key(&key)
        .arg(now)
        .invoke(&mut conn)
        .map_err(|e| e.to_string())?;

    let Some(json_str) = json_str else {
        return Ok(None);
    };

    let challenge = auth_challenge_from_json_str(&json_str)?;

    if challenge.expires_at > chrono::Utc::now().timestamp() {
        Ok(Some(challenge))
    } else {
        Ok(None)
    }
}

/// Cleanup expired auth challenges from the session store
fn cleanup_expired_auth_challenges_memory(now: i64) -> u64 {
    let mut store = SESSION_STORE.lock().unwrap();
    store.cleanup_expired_auth_challenges(now) as u64
}

pub fn cleanup_expired_auth_challenges() -> std::result::Result<u64, String> {
    let now = chrono::Utc::now().timestamp();
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);
    let memory_count = cleanup_expired_auth_challenges_memory(now);

    match store_type {
        Some(SessionStore::Sqlite(_)) => {
            let sqlite_count = cleanup_expired_auth_challenges_sqlite(now)?;
            Ok(sqlite_count + memory_count)
        }
        Some(SessionStore::Postgres(_)) => {
            let postgres_count = cleanup_expired_auth_challenges_postgres(now)?;
            Ok(postgres_count + memory_count)
        }
        Some(SessionStore::Redis(_)) => {
            let redis_count = cleanup_expired_auth_challenges_redis(now)?;
            Ok(redis_count + memory_count)
        }
        _ => Ok(memory_count),
    }
}

fn cleanup_expired_auth_challenges_sqlite(now: i64) -> std::result::Result<u64, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    let count = conn
        .execute(
            "DELETE FROM auth_challenges WHERE expires_at < ?1",
            rusqlite::params![now],
        )
        .map_err(|e| e.to_string())?;

    Ok(count as u64)
}

fn cleanup_expired_auth_challenges_postgres(now: i64) -> std::result::Result<u64, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    let count = client
        .execute("DELETE FROM auth_challenges WHERE expires_at < $1", &[&now])
        .map_err(|e| e.to_string())?;

    Ok(count)
}

fn cleanup_expired_auth_challenges_redis(now: i64) -> std::result::Result<u64, String> {
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
            .arg("ntnt:auth_challenge:*")
            .arg("COUNT")
            .arg(100)
            .query(&mut conn)
            .map_err(|e| format!("Redis SCAN error: {}", e))?;

        for key in keys {
            let result: Option<String> = redis::cmd("GET")
                .arg(&key)
                .query(&mut conn)
                .map_err(|e| format!("Redis GET error: {}", e))?;

            let Some(json_str) = result else {
                continue;
            };

            if let Ok(challenge) = auth_challenge_from_json_str(&json_str) {
                if challenge.expires_at < now {
                    let _: () = redis::cmd("DEL")
                        .arg(&key)
                        .query(&mut conn)
                        .map_err(|e| format!("Redis DEL error: {}", e))?;
                    count += 1;
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

/// Clean up expired exchange tokens from all backends.
/// Called by `sessions_cleanup` and the background cleanup thread.
pub fn cleanup_expired_exchange_tokens() -> std::result::Result<u64, String> {
    let now = chrono::Utc::now().timestamp();
    let cutoff = now - EXCHANGE_TOKEN_TTL;

    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => cleanup_expired_exchange_tokens_sqlite(cutoff),
        Some(SessionStore::Postgres(_)) => cleanup_expired_exchange_tokens_postgres(cutoff),
        Some(SessionStore::Redis(_)) => {
            // Redis exchange tokens use SETEX TTL, so they expire automatically
            Ok(0)
        }
        _ => {
            // Memory backend
            let mut store = SESSION_STORE.lock().unwrap();
            let count = store.cleanup_expired_exchange_tokens(now);
            Ok(count as u64)
        }
    }
}

fn cleanup_expired_exchange_tokens_sqlite(cutoff: i64) -> std::result::Result<u64, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;

    let count = conn
        .execute(
            "DELETE FROM auth_exchange_tokens WHERE created_at < ?1",
            rusqlite::params![cutoff],
        )
        .map_err(|e| e.to_string())?;

    Ok(count as u64)
}

fn cleanup_expired_exchange_tokens_postgres(cutoff: i64) -> std::result::Result<u64, String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;

    let count = client
        .execute(
            "DELETE FROM auth_exchange_tokens WHERE created_at < $1",
            &[&cutoff],
        )
        .map_err(|e| e.to_string())?;

    Ok(count)
}
