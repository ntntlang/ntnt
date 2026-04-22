use super::storage::{
    active_auth_storage_backend, cleanup_expired_session_records,
    delete_all_session_records_for_user, delete_session_record, extend_session_record_expiry,
    get_refreshable_session_record_without_fallback, get_session_record_with_fallback,
    list_session_records_for_user, migrate_session_record, store_session_record,
    update_session_record_data, update_session_record_tokens, AuthStorageBackend,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionAccessEffect {
    Unchanged,
    ExpiryUpdated { expires_at: i64 },
}

pub(super) fn get_session_for_request(id: &str) -> (Option<Session>, SessionAccessEffect) {
    let config = get_auth_config();
    let mut session = get_session_record_with_fallback(id);

    if let Some(active_session) = session.as_mut() {
        let mut effect = SessionAccessEffect::Unchanged;
        if let Some(config) = &config {
            let now = chrono::Utc::now().timestamp();
            let capped_expires_at = capped_session_expiry(now, active_session.created_at, config);
            if capped_expires_at <= now {
                delete_session_by_id(&active_session.id);
                return (None, SessionAccessEffect::Unchanged);
            }
            effect = maybe_slide_session(active_session, config);
        }
        return (session, effect);
    }

    if let Some(config) = &config {
        if config.store_tokens {
            let expired_session =
                get_refreshable_session_record_without_fallback(id, config.refresh_ttl);

            if let Some(expired) = expired_session {
                if let Some(ref refresh_token) = expired.refresh_token {
                    if let Some(provider) =
                        config.providers.iter().find(|p| p.name == expired.provider)
                    {
                        match refresh_access_token(provider, refresh_token) {
                            Ok(tokens) => {
                                let now = chrono::Utc::now().timestamp();
                                let new_expires_at =
                                    capped_session_expiry(now, expired.created_at, config);

                                if new_expires_at <= now {
                                    delete_session_by_id(&expired.id);
                                    return (None, SessionAccessEffect::Unchanged);
                                }

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
                                return (
                                    Some(refreshed),
                                    SessionAccessEffect::ExpiryUpdated {
                                        expires_at: new_expires_at,
                                    },
                                );
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

    (None, SessionAccessEffect::Unchanged)
}

/// Get session by ID
pub fn get_session_by_id(id: &str) -> Option<Session> {
    get_session_for_request(id).0
}

pub(super) fn capped_session_expiry(now: i64, created_at: i64, config: &AuthConfig) -> i64 {
    let sliding_target = now + config.session_ttl;
    match config.max_session_ttl {
        Some(max_session_ttl) => sliding_target.min(created_at + max_session_ttl),
        None => sliding_target,
    }
}

fn maybe_slide_session(session: &mut Session, config: &AuthConfig) -> SessionAccessEffect {
    let now = chrono::Utc::now().timestamp();
    let target_expires_at = capped_session_expiry(now, session.created_at, config);

    if session.expires_at > target_expires_at {
        if extend_session_record_expiry(&session.id, target_expires_at).is_ok() {
            session.expires_at = target_expires_at;
            return SessionAccessEffect::ExpiryUpdated {
                expires_at: target_expires_at,
            };
        }
        return SessionAccessEffect::Unchanged;
    }

    if !config.sliding_sessions {
        return SessionAccessEffect::Unchanged;
    }

    if target_expires_at <= session.expires_at {
        return SessionAccessEffect::Unchanged;
    }

    let remaining = session.expires_at - now;
    if remaining > config.refresh_throttle {
        return SessionAccessEffect::Unchanged;
    }

    if extend_session_record_expiry(&session.id, target_expires_at).is_ok() {
        session.expires_at = target_expires_at;
        SessionAccessEffect::ExpiryUpdated {
            expires_at: target_expires_at,
        }
    } else {
        SessionAccessEffect::Unchanged
    }
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
    let had_fallback_session = store.get_session(old_id).is_some();
    store.delete_session(old_id);
    if had_fallback_session && active_auth_storage_backend() != AuthStorageBackend::Memory {
        store.set_session(new_session.clone());
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
