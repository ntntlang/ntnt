use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AuthStorageBackend {
    Memory,
    Sqlite,
    Postgres,
    Redis,
}

pub(super) fn active_auth_storage_backend() -> AuthStorageBackend {
    let config = get_auth_config();
    match config.as_ref().map(|c| &c.session_store) {
        Some(SessionStore::Sqlite(_)) => AuthStorageBackend::Sqlite,
        Some(SessionStore::Postgres(_)) => AuthStorageBackend::Postgres,
        Some(SessionStore::Redis(_)) => AuthStorageBackend::Redis,
        _ => AuthStorageBackend::Memory,
    }
}

// ============================================================================
// Internal Auth Storage Contract (Phase 4.5C-1)
// ============================================================================
// These helpers are the contract boundary between auth lifecycle code and the
// active backend's native implementation. They intentionally cover all current
// auth record families:
// - sessions
// - staged auth challenges
// - OAuth states
// - session exchange tokens
//
// Higher-level helpers still own fallback/error behavior for now. This layer is
// responsible only for routing each operation through a consistent per-record
// contract while preserving backend-native atomicity and query shape.

pub(super) fn store_oauth_state_record(state: &OAuthState) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => store_oauth_state_sqlite(state),
        AuthStorageBackend::Postgres => store_oauth_state_postgres(state),
        AuthStorageBackend::Redis => store_oauth_state_redis(state),
        AuthStorageBackend::Memory => {
            SESSION_STORE.lock().unwrap().set_oauth_state(state.clone());
            Ok(())
        }
    }
}

pub(super) fn consume_oauth_state_record(
    state: &str,
) -> std::result::Result<Option<OAuthState>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => consume_oauth_state_sqlite(state),
        AuthStorageBackend::Postgres => consume_oauth_state_postgres(state),
        AuthStorageBackend::Redis => consume_oauth_state_redis(state),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            let oauth_state = store.get_oauth_state(state).cloned();
            if oauth_state.is_some() {
                store.delete_oauth_state(state);
            }
            Ok(oauth_state)
        }
    }
}

pub(super) fn store_exchange_token_record(
    token: &str,
    session_id: &str,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => store_exchange_token_sqlite(token, session_id),
        AuthStorageBackend::Postgres => store_exchange_token_postgres(token, session_id),
        AuthStorageBackend::Redis => store_exchange_token_redis(token, session_id),
        AuthStorageBackend::Memory => {
            SESSION_STORE
                .lock()
                .unwrap()
                .set_exchange_token(token.to_string(), session_id.to_string());
            Ok(())
        }
    }
}

pub(super) fn consume_exchange_token_record(
    token: &str,
) -> std::result::Result<Option<String>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => consume_exchange_token_sqlite(token),
        AuthStorageBackend::Postgres => consume_exchange_token_postgres(token),
        AuthStorageBackend::Redis => consume_exchange_token_redis(token),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            let session_id = store.get_exchange_token(token).cloned();
            if session_id.is_some() {
                store.delete_exchange_token(token);
            }
            Ok(session_id)
        }
    }
}

pub(super) fn store_auth_challenge_record(
    challenge: &AuthChallenge,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => store_auth_challenge_sqlite(challenge),
        AuthStorageBackend::Postgres => store_auth_challenge_postgres(challenge),
        AuthStorageBackend::Redis => store_auth_challenge_redis(challenge),
        AuthStorageBackend::Memory => {
            SESSION_STORE
                .lock()
                .unwrap()
                .set_auth_challenge(challenge.clone());
            Ok(())
        }
    }
}

pub(super) fn get_auth_challenge_record(
    id: &str,
) -> std::result::Result<Option<AuthChallenge>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => get_auth_challenge_sqlite(id),
        AuthStorageBackend::Postgres => get_auth_challenge_postgres(id),
        AuthStorageBackend::Redis => get_auth_challenge_redis(id),
        AuthStorageBackend::Memory => Ok(get_auth_challenge_memory(id)),
    }
}

pub(super) fn delete_auth_challenge_record(id: &str) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => delete_auth_challenge_sqlite(id),
        AuthStorageBackend::Postgres => delete_auth_challenge_postgres(id),
        AuthStorageBackend::Redis => delete_auth_challenge_redis(id),
        AuthStorageBackend::Memory => {
            SESSION_STORE.lock().unwrap().delete_auth_challenge(id);
            Ok(())
        }
    }
}

pub(super) fn consume_auth_challenge_record(
    id: &str,
) -> std::result::Result<Option<AuthChallenge>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => consume_auth_challenge_sqlite(id),
        AuthStorageBackend::Postgres => consume_auth_challenge_postgres(id),
        AuthStorageBackend::Redis => consume_auth_challenge_redis(id),
        AuthStorageBackend::Memory => Ok(SESSION_STORE.lock().unwrap().take_auth_challenge(id)),
    }
}

pub(super) fn cleanup_expired_auth_challenge_records(now: i64) -> std::result::Result<u64, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => cleanup_expired_auth_challenges_sqlite(now),
        AuthStorageBackend::Postgres => cleanup_expired_auth_challenges_postgres(now),
        AuthStorageBackend::Redis => cleanup_expired_auth_challenges_redis(now),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            Ok(store.cleanup_expired_auth_challenges(now) as u64)
        }
    }
}

pub(super) fn cleanup_expired_oauth_state_records(cutoff: i64) -> std::result::Result<u64, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => cleanup_expired_oauth_states_sqlite(cutoff),
        AuthStorageBackend::Postgres => cleanup_expired_oauth_states_postgres(cutoff),
        AuthStorageBackend::Redis => Ok(0),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            Ok(store.cleanup_expired_oauth_states(cutoff) as u64)
        }
    }
}

pub(super) fn cleanup_expired_exchange_token_records(now: i64) -> std::result::Result<u64, String> {
    let cutoff = now - EXCHANGE_TOKEN_TTL;
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => cleanup_expired_exchange_tokens_sqlite(cutoff),
        AuthStorageBackend::Postgres => cleanup_expired_exchange_tokens_postgres(cutoff),
        AuthStorageBackend::Redis => Ok(0),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            Ok(store.cleanup_expired_exchange_tokens(now) as u64)
        }
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

    let backend = active_auth_storage_backend();
    if let Err(e) = store_oauth_state_record(&oauth_state) {
        match backend {
            AuthStorageBackend::Sqlite => {
                eprintln!(
                    "[auth] SQLite oauth state store failed, using memory: {}",
                    e
                );
            }
            AuthStorageBackend::Postgres => {
                eprintln!(
                    "[auth] PostgreSQL oauth state store failed, using memory: {}",
                    e
                );
            }
            AuthStorageBackend::Redis => {
                eprintln!("[auth] Redis oauth state store failed, using memory: {}", e);
            }
            AuthStorageBackend::Memory => {}
        }
        SESSION_STORE.lock().unwrap().set_oauth_state(oauth_state);
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
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => consume_oauth_state_record(state).ok().flatten(),
        _ => consume_oauth_state_record(state)
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
    }
}

fn consume_oauth_state_sqlite(state: &str) -> std::result::Result<Option<OAuthState>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let now = chrono::Utc::now().timestamp();
    let min_created = now - 600; // 10 minutes

    let result = conn.query_row(
        "DELETE FROM auth_oauth_states
         WHERE state = ?1 AND created_at > ?2
         RETURNING state, nonce, pkce_verifier, provider, redirect_url, created_at",
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
            "DELETE FROM auth_oauth_states
         WHERE state = $1 AND created_at > $2
         RETURNING state, nonce, pkce_verifier, provider, redirect_url, created_at",
            &[&state, &min_created],
        )
        .map_err(|e| e.to_string())?;

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
    let backend = active_auth_storage_backend();
    if let Err(e) = store_exchange_token_record(token, session_id) {
        match backend {
            AuthStorageBackend::Sqlite => {
                eprintln!(
                    "[auth] SQLite exchange token store failed, using memory: {}",
                    e
                );
            }
            AuthStorageBackend::Postgres => {
                eprintln!(
                    "[auth] PostgreSQL exchange token store failed, using memory: {}",
                    e
                );
            }
            AuthStorageBackend::Redis => {
                eprintln!(
                    "[auth] Redis exchange token store failed, using memory: {}",
                    e
                );
            }
            AuthStorageBackend::Memory => {}
        }
        SESSION_STORE
            .lock()
            .unwrap()
            .set_exchange_token(token.to_string(), session_id.to_string());
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
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => consume_exchange_token_record(token).ok().flatten(),
        _ => consume_exchange_token_record(token)
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
    let backend = active_auth_storage_backend();
    if let Err(e) = store_auth_challenge_record(&challenge) {
        match backend {
            AuthStorageBackend::Sqlite => {
                eprintln!("[auth] WARNING: SQLite auth challenge store failed: {}", e);
            }
            AuthStorageBackend::Postgres => {
                eprintln!(
                    "[auth] WARNING: PostgreSQL auth challenge store failed: {}",
                    e
                );
            }
            AuthStorageBackend::Redis => {
                eprintln!("[auth] WARNING: Redis auth challenge store failed: {}", e);
            }
            AuthStorageBackend::Memory => {}
        }
        SESSION_STORE.lock().unwrap().set_auth_challenge(challenge);
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
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => get_auth_challenge_record(id),
        _ => match get_auth_challenge_record(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(get_auth_challenge_memory(id)),
            Err(e) => get_auth_challenge_memory(id).map(Some).ok_or(e),
        },
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
    let _ = delete_auth_challenge_record(id);
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
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => consume_auth_challenge_record(id),
        _ => match consume_auth_challenge_record(id) {
            Ok(Some(challenge)) => Ok(Some(challenge)),
            Ok(None) => Ok(SESSION_STORE.lock().unwrap().take_auth_challenge(id)),
            Err(e) => match SESSION_STORE.lock().unwrap().take_auth_challenge(id) {
                Some(challenge) => Ok(Some(challenge)),
                None => Err(e),
            },
        },
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

    // Expiry was already validated atomically inside the Lua script above.
    Ok(Some(challenge))
}

/// Cleanup expired auth challenges from the session store
fn cleanup_expired_auth_challenges_memory(now: i64) -> u64 {
    let mut store = SESSION_STORE.lock().unwrap();
    store.cleanup_expired_auth_challenges(now) as u64
}

pub fn cleanup_expired_auth_challenges() -> std::result::Result<u64, String> {
    let now = chrono::Utc::now().timestamp();
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => cleanup_expired_auth_challenge_records(now),
        _ => {
            let backend_count = cleanup_expired_auth_challenge_records(now)?;
            let memory_count = cleanup_expired_auth_challenges_memory(now);
            Ok(backend_count + memory_count)
        }
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
    let cutoff = now - 600;
    cleanup_expired_oauth_state_records(cutoff)
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
    cleanup_expired_exchange_token_records(now)
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

// Session-specific operations live in the same contract layer for now, even
// though session lifecycle orchestration still lives in `sessions.rs`.

pub(super) fn store_session_record(session: &Session) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => store_session_sqlite(session),
        AuthStorageBackend::Postgres => store_session_postgres(session),
        AuthStorageBackend::Redis => store_session_redis(session),
        AuthStorageBackend::Memory => {
            SESSION_STORE.lock().unwrap().set_session(session.clone());
            Ok(())
        }
    }
}

pub(super) fn get_session_record(id: &str) -> std::result::Result<Option<Session>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => get_session_sqlite(id),
        AuthStorageBackend::Postgres => get_session_postgres(id),
        AuthStorageBackend::Redis => get_session_redis(id),
        AuthStorageBackend::Memory => Ok(SESSION_STORE.lock().unwrap().get_session(id).cloned()),
    }
}

pub(super) fn get_refreshable_session_record(
    id: &str,
    refresh_ttl: i64,
) -> std::result::Result<Option<Session>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => get_expired_session_sqlite(id, refresh_ttl),
        AuthStorageBackend::Postgres => get_expired_session_postgres(id, refresh_ttl),
        AuthStorageBackend::Redis => get_expired_session_redis(id, refresh_ttl),
        AuthStorageBackend::Memory => Ok(None),
    }
}

pub(super) fn extend_session_record_expiry(
    id: &str,
    new_expires_at: i64,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => extend_session_expiry_sqlite(id, new_expires_at),
        AuthStorageBackend::Postgres => extend_session_expiry_postgres(id, new_expires_at),
        AuthStorageBackend::Redis => extend_session_expiry_redis(id, new_expires_at),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            if let Some(session) = store.get_session_mut(id) {
                session.expires_at = new_expires_at;
            }
            Ok(())
        }
    }
}

pub(super) fn update_session_record_tokens(
    id: &str,
    tokens: &TokenResponse,
    now: i64,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => update_session_tokens_sqlite(id, tokens, now),
        AuthStorageBackend::Postgres => update_session_tokens_postgres(id, tokens, now),
        AuthStorageBackend::Redis => update_session_tokens_redis(id, tokens, now),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            if let Some(session) = store.get_session_mut(id) {
                session.access_token = Some(tokens.access_token.clone());
                if let Some(ref rt) = tokens.refresh_token {
                    session.refresh_token = Some(rt.clone());
                }
                session.token_expires_at = tokens.expires_in.map(|e| now + e);
            }
            Ok(())
        }
    }
}

pub(super) fn update_session_record_data(
    id: &str,
    data_json: &str,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => update_session_data_sqlite(id, data_json),
        AuthStorageBackend::Postgres => update_session_data_postgres(id, data_json),
        AuthStorageBackend::Redis => update_session_data_redis(id, data_json),
        AuthStorageBackend::Memory => {
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

pub(super) fn delete_session_record(id: &str) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => delete_session_sqlite(id),
        AuthStorageBackend::Postgres => delete_session_postgres(id),
        AuthStorageBackend::Redis => delete_session_redis(id),
        AuthStorageBackend::Memory => {
            SESSION_STORE.lock().unwrap().delete_session(id);
            Ok(())
        }
    }
}

pub(super) fn migrate_session_record(
    old_id: &str,
    new_session: &Session,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => migrate_session_sqlite(old_id, new_session),
        AuthStorageBackend::Postgres => migrate_session_postgres(old_id, new_session),
        AuthStorageBackend::Redis => migrate_session_redis(old_id, new_session),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            if store.get_session(old_id).is_none() {
                return Err("Session not found".to_string());
            }
            store.delete_session(old_id);
            store.set_session(new_session.clone());
            Ok(())
        }
    }
}

pub(super) fn cleanup_expired_session_records(now: i64) -> std::result::Result<u64, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => cleanup_expired_sessions_sqlite(now),
        AuthStorageBackend::Postgres => cleanup_expired_sessions_postgres(now),
        AuthStorageBackend::Redis => cleanup_expired_sessions_redis(now),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            Ok(store.cleanup_expired(now) as u64)
        }
    }
}

pub(super) fn list_session_records_for_user(
    user_id: &str,
    current_session_id: Option<&str>,
    now: i64,
) -> std::result::Result<Vec<SessionInfo>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => {
            get_sessions_for_user_sqlite(user_id, current_session_id, now)
        }
        AuthStorageBackend::Postgres => {
            get_sessions_for_user_postgres(user_id, current_session_id, now)
        }
        AuthStorageBackend::Redis => get_sessions_for_user_redis(user_id, current_session_id, now),
        AuthStorageBackend::Memory => {
            let store = SESSION_STORE.lock().unwrap();
            Ok(store.get_sessions_for_user(user_id, current_session_id, now))
        }
    }
}

pub(super) fn delete_all_session_records_for_user(
    user_id: &str,
    keep_session_id: Option<&str>,
) -> std::result::Result<u64, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Sqlite => delete_all_sessions_for_user_sqlite(user_id, keep_session_id),
        AuthStorageBackend::Postgres => {
            delete_all_sessions_for_user_postgres(user_id, keep_session_id)
        }
        AuthStorageBackend::Redis => delete_all_sessions_for_user_redis(user_id, keep_session_id),
        AuthStorageBackend::Memory => {
            let mut store = SESSION_STORE.lock().unwrap();
            Ok(store.delete_all_sessions_for_user(user_id, keep_session_id) as u64)
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
        redis::cmd("SETEX")
            .arg(&key)
            .arg(ttl)
            .arg(&session_json)
            .query::<()>(&mut conn)
            .map_err(|e| format!("Redis SETEX error: {}", e))?;
    } else {
        return Ok(());
    }

    Ok(())
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

fn migrate_session_sqlite(old_id: &str, new_session: &Session) -> std::result::Result<(), String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let updated = conn
        .execute(
            "UPDATE auth_sessions
         SET id = ?1, user_id = ?2, provider = ?3, email = ?4, name = ?5, picture = ?6,
             raw_json = ?7, data_json = ?8, csrf_token = ?9, access_token = ?10,
             refresh_token = ?11, token_expires_at = ?12, created_at = ?13, expires_at = ?14
         WHERE id = ?15",
            rusqlite::params![
                new_session.id,
                new_session.user_id,
                new_session.provider,
                new_session.email,
                new_session.name,
                new_session.picture,
                new_session.raw_json,
                new_session.data_json,
                new_session.csrf_token,
                new_session.access_token,
                new_session.refresh_token,
                new_session.token_expires_at,
                new_session.created_at,
                new_session.expires_at,
                old_id,
            ],
        )
        .map_err(|e| e.to_string())?;

    if updated == 0 {
        return Err("Session not found".to_string());
    }

    Ok(())
}

fn migrate_session_postgres(
    old_id: &str,
    new_session: &Session,
) -> std::result::Result<(), String> {
    let url_guard = POSTGRES_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("PostgreSQL not initialized")?;

    let mut client = postgres::Client::connect(url, postgres::NoTls).map_err(|e| e.to_string())?;
    let updated = client
        .execute(
            "UPDATE auth_sessions
             SET id = $1, user_id = $2, provider = $3, email = $4, name = $5, picture = $6,
                 raw_json = $7, data_json = $8, csrf_token = $9, access_token = $10,
                 refresh_token = $11, token_expires_at = $12, created_at = $13, expires_at = $14
             WHERE id = $15",
            &[
                &new_session.id,
                &new_session.user_id,
                &new_session.provider,
                &new_session.email,
                &new_session.name,
                &new_session.picture,
                &new_session.raw_json,
                &new_session.data_json,
                &new_session.csrf_token,
                &new_session.access_token,
                &new_session.refresh_token,
                &new_session.token_expires_at,
                &new_session.created_at,
                &new_session.expires_at,
                &old_id,
            ],
        )
        .map_err(|e| e.to_string())?;

    if updated == 0 {
        return Err("Session not found".to_string());
    }

    Ok(())
}

fn migrate_session_redis(old_id: &str, new_session: &Session) -> std::result::Result<(), String> {
    let url_guard = REDIS_URL.lock().unwrap();
    let url = url_guard.as_ref().ok_or("Redis not initialized")?;

    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    let mut conn = client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))?;

    let old_key = format!("ntnt:session:{}", old_id);
    let new_key = format!("ntnt:session:{}", new_session.id);
    let ttl = (new_session.expires_at - chrono::Utc::now().timestamp()).max(0);
    let session_json = serde_json::json!({
        "id": new_session.id,
        "user_id": new_session.user_id,
        "provider": new_session.provider,
        "email": new_session.email,
        "name": new_session.name,
        "picture": new_session.picture,
        "raw_json": new_session.raw_json,
        "data_json": new_session.data_json,
        "csrf_token": new_session.csrf_token,
        "access_token": new_session.access_token,
        "refresh_token": new_session.refresh_token,
        "token_expires_at": new_session.token_expires_at,
        "created_at": new_session.created_at,
        "expires_at": new_session.expires_at,
    })
    .to_string();

    let lua_script = r#"
        local existing = redis.call('GET', KEYS[1])
        if not existing then return 0 end
        if tonumber(ARGV[1]) <= 0 then
            redis.call('DEL', KEYS[1])
            return 1
        end
        redis.call('SETEX', KEYS[2], tonumber(ARGV[1]), ARGV[2])
        redis.call('DEL', KEYS[1])
        return 1
    "#;

    let migrated: i32 = redis::Script::new(lua_script)
        .key(&old_key)
        .key(&new_key)
        .arg(ttl)
        .arg(&session_json)
        .invoke(&mut conn)
        .map_err(|e| e.to_string())?;

    if migrated == 0 {
        return Err("Session not found".to_string());
    }

    Ok(())
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

    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(sessions)
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
