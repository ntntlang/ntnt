use super::storage::{
    active_auth_storage_backend, cleanup_expired_session_records,
    delete_all_session_records_for_user, delete_session_record, extend_session_record_expiry,
    get_refreshable_session_record, get_session_record, list_session_records_for_user,
    migrate_session_record, store_session_record, update_session_record_data,
    update_session_record_tokens, AuthStorageBackend,
};
use super::*;

/// Store session
pub fn store_session(session: Session) {
    let backend = active_auth_storage_backend();

    if let Err(e) = store_session_record(&session) {
        match backend {
            AuthStorageBackend::Sqlite => {
                eprintln!("[auth] WARNING: SQLite store failed: {}", e);
                eprintln!("[auth] Falling back to memory store - session will be lost on restart!");
            }
            AuthStorageBackend::Postgres => {
                eprintln!("[auth] WARNING: PostgreSQL store failed: {}", e);
                eprintln!("[auth] Falling back to memory store - session will be lost on restart and not shared across instances!");
            }
            AuthStorageBackend::Redis => {
                eprintln!("[auth] WARNING: Redis store failed: {}", e);
                eprintln!("[auth] Falling back to memory store - session will be lost on restart and not shared across instances!");
            }
            AuthStorageBackend::Memory => {}
        }
        SESSION_STORE.lock().unwrap().set_session(session);
    }
}

/// Get session by ID
pub fn get_session_by_id(id: &str) -> Option<Session> {
    let config = get_auth_config();
    let backend = active_auth_storage_backend();

    let session = match backend {
        AuthStorageBackend::Memory => get_session_record(id).ok().flatten(),
        _ => match get_session_record(id) {
            Ok(Some(session)) => Some(session),
            Ok(None) => SESSION_STORE.lock().unwrap().get_session(id).cloned(),
            Err(_) => SESSION_STORE.lock().unwrap().get_session(id).cloned(),
        },
    };

    if session.is_some() {
        return session;
    }

    if let Some(config) = &config {
        if config.store_tokens {
            let expired_session = match backend {
                AuthStorageBackend::Memory => None,
                _ => get_refreshable_session_record(id, config.refresh_ttl)
                    .ok()
                    .flatten(),
            };

            if let Some(expired) = expired_session {
                if let Some(ref refresh_token) = expired.refresh_token {
                    if let Some(provider) =
                        config.providers.iter().find(|p| p.name == expired.provider)
                    {
                        match refresh_access_token(provider, refresh_token) {
                            Ok(tokens) => {
                                let now = chrono::Utc::now().timestamp();
                                let new_expires_at = now + config.session_ttl;

                                update_session_tokens(&expired.id, &tokens);
                                extend_session_expiry(&expired.id, new_expires_at);

                                eprintln!(
                                    "[auth] Session {} auto-refreshed via refresh token",
                                    &expired.id[..8]
                                );

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
    let _ = extend_session_record_expiry(id, new_expires_at);
}

/// Update session tokens
pub fn update_session_tokens(id: &str, tokens: &TokenResponse) {
    let now = chrono::Utc::now().timestamp();
    let _ = update_session_record_tokens(id, tokens, now);
}

/// Update session custom data (for RBAC/claims)
pub fn update_session_data(id: &str, data_json: &str) -> std::result::Result<(), String> {
    update_session_record_data(id, data_json)
}

/// Delete session by ID
pub fn delete_session_by_id(id: &str) {
    let _ = delete_session_record(id);
    SESSION_STORE.lock().unwrap().delete_session(id);
}

pub(super) fn migrate_session(
    old_id: &str,
    new_session: &Session,
) -> std::result::Result<(), String> {
    migrate_session_record(old_id, new_session)?;

    let mut store = SESSION_STORE.lock().unwrap();
    store.delete_session(old_id);
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
    cleanup_expired_session_records(now)
}

/// Get all sessions for a user
pub fn get_sessions_for_user(
    user_id: &str,
    current_session_id: Option<&str>,
) -> std::result::Result<Vec<SessionInfo>, String> {
    let now = chrono::Utc::now().timestamp();
    list_session_records_for_user(user_id, current_session_id, now)
}

/// Delete all sessions for a user, optionally keeping one session
/// Returns the number of sessions deleted
pub fn delete_all_sessions_for_user(
    user_id: &str,
    keep_session_id: Option<&str>,
) -> std::result::Result<u64, String> {
    delete_all_session_records_for_user(user_id, keep_session_id)
}
