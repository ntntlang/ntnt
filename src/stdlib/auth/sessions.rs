use super::*;

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

pub(super) fn migrate_session(
    old_id: &str,
    new_session: &Session,
) -> std::result::Result<(), String> {
    let config = get_auth_config();
    let store_type = config.as_ref().map(|c| &c.session_store);

    match store_type {
        Some(SessionStore::Sqlite(_)) => migrate_session_sqlite(old_id, new_session)?,
        Some(SessionStore::Postgres(_)) => migrate_session_postgres(old_id, new_session)?,
        Some(SessionStore::Redis(_)) => migrate_session_redis(old_id, new_session)?,
        _ => {
            let mut store = SESSION_STORE.lock().unwrap();
            if store.get_session(old_id).is_none() {
                return Err("Session not found".to_string());
            }
            store.delete_session(old_id);
            store.set_session(new_session.clone());
            return Ok(());
        }
    }

    let mut store = SESSION_STORE.lock().unwrap();
    store.delete_session(old_id);
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

pub(super) fn build_rotated_session(session_id: &str) -> std::result::Result<Session, String> {
    let mut session = get_session_by_id(session_id).ok_or("No active session".to_string())?;
    session.id = generate_session_id();
    session.csrf_token = uuid::Uuid::new_v4().to_string();
    Ok(session)
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
