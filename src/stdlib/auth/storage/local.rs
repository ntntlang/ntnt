// Local-auth storage intentionally includes write-side helpers before the
// public create/reset/bootstrap flows land in later DD-062 slices.
#![cfg_attr(not(test), allow(dead_code))]

use super::*;
use rusqlite::OptionalExtension;
use std::collections::HashMap;

/// Durable local-auth record families planned by DD-062.
///
/// These are deliberately modeled before implementation so credential-related
/// state does not inherit the softer memory fallback semantics used by some
/// transient session/OAuth/challenge paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::stdlib::auth) enum LocalAuthRecordKind {
    Identity,
    CredentialSecret,
    TotpEnrollment,
    PasswordResetToken,
    BootstrapState,
}

/// Fallback contract for a local-auth record family.
///
/// Durable local-auth state is security-critical account state. In production,
/// backend failures must fail closed instead of silently degrading to process
/// memory. This policy is the scaffold future local-auth storage code must wire
/// into real store/get/update/consume helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::stdlib::auth) struct LocalAuthFallbackPolicy {
    pub(in crate::stdlib::auth) store_failure_fails_closed: bool,
    pub(in crate::stdlib::auth) lookup_failure_fails_closed: bool,
    pub(in crate::stdlib::auth) update_failure_fails_closed: bool,
    pub(in crate::stdlib::auth) production_memory_fallback_allowed: bool,
}

pub(in crate::stdlib::auth) fn local_auth_record_fallback_policy(
    _record_kind: LocalAuthRecordKind,
) -> LocalAuthFallbackPolicy {
    LocalAuthFallbackPolicy {
        store_failure_fails_closed: true,
        lookup_failure_fails_closed: true,
        update_failure_fails_closed: true,
        production_memory_fallback_allowed: false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::stdlib::auth) enum LocalAccountState {
    Bootstrap,
    PendingSetup,
    Active,
    Disabled,
    Locked,
    PasswordChangeRequired,
}

impl LocalAccountState {
    pub(in crate::stdlib::auth) fn as_str(self) -> &'static str {
        match self {
            LocalAccountState::Bootstrap => "bootstrap",
            LocalAccountState::PendingSetup => "pending_setup",
            LocalAccountState::Active => "active",
            LocalAccountState::Disabled => "disabled",
            LocalAccountState::Locked => "locked",
            LocalAccountState::PasswordChangeRequired => "password_change_required",
        }
    }

    pub(in crate::stdlib::auth) fn from_str(value: &str) -> std::result::Result<Self, String> {
        match value {
            "bootstrap" => Ok(LocalAccountState::Bootstrap),
            "pending_setup" => Ok(LocalAccountState::PendingSetup),
            "active" => Ok(LocalAccountState::Active),
            "disabled" => Ok(LocalAccountState::Disabled),
            "locked" => Ok(LocalAccountState::Locked),
            "password_change_required" => Ok(LocalAccountState::PasswordChangeRequired),
            other => Err(format!(
                "[auth] unknown local account state \"{}\". Expected one of: bootstrap, pending_setup, active, disabled, locked, password_change_required",
                other
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::stdlib::auth) struct LocalIdentity {
    pub(in crate::stdlib::auth) id: String,
    pub(in crate::stdlib::auth) identifier_kind: String,
    pub(in crate::stdlib::auth) identifier: String,
    pub(in crate::stdlib::auth) identifier_normalized: String,
    pub(in crate::stdlib::auth) created_at: i64,
    pub(in crate::stdlib::auth) updated_at: i64,
    pub(in crate::stdlib::auth) state: LocalAccountState,
    pub(in crate::stdlib::auth) metadata_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::stdlib::auth) struct LocalCredentialSecret {
    pub(in crate::stdlib::auth) local_user_id: String,
    pub(in crate::stdlib::auth) password_hash: String,
    pub(in crate::stdlib::auth) password_hash_algorithm: String,
    pub(in crate::stdlib::auth) password_hash_params_json: String,
    pub(in crate::stdlib::auth) password_changed_at: i64,
    pub(in crate::stdlib::auth) must_change_password: bool,
}

pub(in crate::stdlib::auth) fn normalize_local_identifier(
    identifier_kind: &str,
    identifier: &str,
) -> std::result::Result<String, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    match kind.as_str() {
        "email" => normalize_email_identifier(identifier),
        other => Err(format!(
            "[auth] unsupported local identifier kind \"{}\". Supported kinds: email",
            other
        )),
    }
}

fn normalize_email_identifier(identifier: &str) -> std::result::Result<String, String> {
    let normalized = identifier.trim().to_ascii_lowercase();
    let Some((local, domain)) = normalized.split_once('@') else {
        return Err("[auth] local email identifier must contain @".to_string());
    };
    if local.is_empty() || domain.is_empty() || domain.starts_with('.') || domain.ends_with('.') {
        return Err("[auth] local email identifier must have a local part and domain".to_string());
    }
    if domain.split('.').any(|part| part.is_empty()) {
        return Err("[auth] local email identifier domain is invalid".to_string());
    }
    Ok(normalized)
}

#[derive(Debug, Default, Clone)]
pub(in crate::stdlib::auth) struct LocalAuthMemoryStore {
    identities_by_id: HashMap<String, LocalIdentity>,
    identity_id_by_lookup_key: HashMap<String, String>,
    credential_secrets_by_local_user_id: HashMap<String, LocalCredentialSecret>,
}

impl LocalAuthMemoryStore {
    pub(in crate::stdlib::auth) fn store_identity(
        &mut self,
        identity: LocalIdentity,
    ) -> std::result::Result<(), String> {
        let identity = normalize_local_identity_for_storage(identity)?;
        if identity.id.trim().is_empty() {
            return Err("[auth] local identity id must not be empty".to_string());
        }
        let lookup_key =
            local_identity_lookup_key(&identity.identifier_kind, &identity.identifier_normalized)?;
        if let Some(existing_id) = self.identity_id_by_lookup_key.get(&lookup_key) {
            if existing_id != &identity.id {
                return Err(format!(
                    "[auth] local identity identifier already exists for {}",
                    identity.identifier_kind
                ));
            }
        }

        if let Some(previous) = self.identities_by_id.get(&identity.id) {
            let previous_lookup_key = local_identity_lookup_key(
                &previous.identifier_kind,
                &previous.identifier_normalized,
            )?;
            if previous_lookup_key != lookup_key {
                self.identity_id_by_lookup_key.remove(&previous_lookup_key);
            }
        }

        self.identity_id_by_lookup_key
            .insert(lookup_key, identity.id.clone());
        self.identities_by_id.insert(identity.id.clone(), identity);
        Ok(())
    }

    pub(in crate::stdlib::auth) fn get_identity_by_id(
        &self,
        id: &str,
    ) -> std::result::Result<Option<LocalIdentity>, String> {
        Ok(self.identities_by_id.get(id).cloned())
    }

    pub(in crate::stdlib::auth) fn get_identity_by_identifier(
        &self,
        identifier_kind: &str,
        identifier_normalized: &str,
    ) -> std::result::Result<Option<LocalIdentity>, String> {
        let lookup_key = local_identity_lookup_key(identifier_kind, identifier_normalized)?;
        Ok(self
            .identity_id_by_lookup_key
            .get(&lookup_key)
            .and_then(|id| self.identities_by_id.get(id))
            .cloned())
    }

    pub(in crate::stdlib::auth) fn store_credential_secret(
        &mut self,
        credential: LocalCredentialSecret,
    ) -> std::result::Result<(), String> {
        validate_local_credential_secret_for_storage(&credential)?;
        if !self
            .identities_by_id
            .contains_key(&credential.local_user_id)
        {
            return Err(
                "[auth] local credential must reference an existing local identity".to_string(),
            );
        }
        self.credential_secrets_by_local_user_id
            .insert(credential.local_user_id.clone(), credential);
        Ok(())
    }

    pub(in crate::stdlib::auth) fn store_identity_and_credential(
        &mut self,
        identity: LocalIdentity,
        credential: LocalCredentialSecret,
    ) -> std::result::Result<(), String> {
        let identity = normalize_local_identity_for_storage(identity)?;
        validate_local_credential_secret_for_storage(&credential)?;
        validate_local_identity_credential_pair(&identity, &credential)?;

        let lookup_key =
            local_identity_lookup_key(&identity.identifier_kind, &identity.identifier_normalized)?;
        if let Some(existing_id) = self.identity_id_by_lookup_key.get(&lookup_key) {
            if existing_id != &identity.id {
                return Err(format!(
                    "[auth] local identity identifier already exists for {}",
                    identity.identifier_kind
                ));
            }
        }

        if let Some(previous) = self.identities_by_id.get(&identity.id) {
            let previous_lookup_key = local_identity_lookup_key(
                &previous.identifier_kind,
                &previous.identifier_normalized,
            )?;
            if previous_lookup_key != lookup_key {
                self.identity_id_by_lookup_key.remove(&previous_lookup_key);
            }
        }

        self.identity_id_by_lookup_key
            .insert(lookup_key, identity.id.clone());
        self.identities_by_id
            .insert(identity.id.clone(), identity.clone());
        self.credential_secrets_by_local_user_id
            .insert(identity.id, credential);
        Ok(())
    }

    pub(in crate::stdlib::auth) fn get_credential_secret(
        &self,
        local_user_id: &str,
    ) -> std::result::Result<Option<LocalCredentialSecret>, String> {
        Ok(self
            .credential_secrets_by_local_user_id
            .get(local_user_id)
            .cloned())
    }
}

fn local_identity_lookup_key(
    identifier_kind: &str,
    identifier_normalized: &str,
) -> std::result::Result<String, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let normalized = identifier_normalized.trim().to_ascii_lowercase();
    if kind.is_empty() || normalized.is_empty() {
        return Err("[auth] local identity identifier kind/value must not be empty".to_string());
    }
    Ok(format!("{}:{}", kind, normalized))
}

fn normalize_local_identity_for_storage(
    mut identity: LocalIdentity,
) -> std::result::Result<LocalIdentity, String> {
    let kind = identity.identifier_kind.trim().to_ascii_lowercase();
    let normalized = normalize_local_identifier(&kind, &identity.identifier)?;
    identity.identifier_kind = kind;
    identity.identifier_normalized = normalized;
    Ok(identity)
}

fn validate_local_credential_secret_for_storage(
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    if credential.local_user_id.trim().is_empty() {
        return Err("[auth] local credential local_user_id must not be empty".to_string());
    }
    if credential.password_hash.trim().is_empty() {
        return Err("[auth] local credential password_hash must not be empty".to_string());
    }
    if credential.password_hash_algorithm.trim().is_empty() {
        return Err(
            "[auth] local credential password_hash_algorithm must not be empty".to_string(),
        );
    }
    Ok(())
}

fn validate_local_identity_credential_pair(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    if credential.local_user_id != identity.id {
        return Err("[auth] local credential must reference the paired local identity".to_string());
    }
    Ok(())
}

pub(in crate::stdlib::auth) fn store_local_identity_record(
    identity: &LocalIdentity,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .store_identity(identity.clone()),
        AuthStorageBackend::Sqlite => store_local_identity_sqlite(identity),
        AuthStorageBackend::Postgres => {
            Err("[auth] local identity storage is not implemented for PostgreSQL yet".to_string())
        }
        AuthStorageBackend::Redis => {
            Err("[auth] local identity storage is not implemented for Redis/Valkey yet".to_string())
        }
    }
}

pub(in crate::stdlib::auth) fn store_local_identity_and_credential_record(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .store_identity_and_credential(identity.clone(), credential.clone()),
        AuthStorageBackend::Sqlite => {
            store_local_identity_and_credential_sqlite(identity, credential)
        }
        AuthStorageBackend::Postgres => Err(
            "[auth] local identity and credential storage is not implemented for PostgreSQL yet"
                .to_string(),
        ),
        AuthStorageBackend::Redis => Err(
            "[auth] local identity and credential storage is not implemented for Redis/Valkey yet"
                .to_string(),
        ),
    }
}

pub(in crate::stdlib::auth) fn get_local_identity_by_id_record(
    id: &str,
) -> std::result::Result<Option<LocalIdentity>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .get_identity_by_id(id),
        AuthStorageBackend::Sqlite => get_local_identity_by_id_sqlite(id),
        AuthStorageBackend::Postgres => {
            Err("[auth] local identity lookup is not implemented for PostgreSQL yet".to_string())
        }
        AuthStorageBackend::Redis => {
            Err("[auth] local identity lookup is not implemented for Redis/Valkey yet".to_string())
        }
    }
}

pub(in crate::stdlib::auth) fn get_local_identity_by_identifier_record(
    identifier_kind: &str,
    identifier_normalized: &str,
) -> std::result::Result<Option<LocalIdentity>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .get_identity_by_identifier(identifier_kind, identifier_normalized),
        AuthStorageBackend::Sqlite => {
            get_local_identity_by_identifier_sqlite(identifier_kind, identifier_normalized)
        }
        AuthStorageBackend::Postgres => {
            Err("[auth] local identity lookup is not implemented for PostgreSQL yet".to_string())
        }
        AuthStorageBackend::Redis => {
            Err("[auth] local identity lookup is not implemented for Redis/Valkey yet".to_string())
        }
    }
}

pub(in crate::stdlib::auth) fn store_local_credential_secret_record(
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .store_credential_secret(credential.clone()),
        AuthStorageBackend::Sqlite => store_local_credential_secret_sqlite(credential),
        AuthStorageBackend::Postgres => {
            Err("[auth] local credential storage is not implemented for PostgreSQL yet".to_string())
        }
        AuthStorageBackend::Redis => Err(
            "[auth] local credential storage is not implemented for Redis/Valkey yet".to_string(),
        ),
    }
}

pub(in crate::stdlib::auth) fn get_local_credential_secret_record(
    local_user_id: &str,
) -> std::result::Result<Option<LocalCredentialSecret>, String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .get_credential_secret(local_user_id),
        AuthStorageBackend::Sqlite => get_local_credential_secret_sqlite(local_user_id),
        AuthStorageBackend::Postgres => {
            Err("[auth] local credential lookup is not implemented for PostgreSQL yet".to_string())
        }
        AuthStorageBackend::Redis => Err(
            "[auth] local credential lookup is not implemented for Redis/Valkey yet".to_string(),
        ),
    }
}

fn store_local_identity_sqlite(identity: &LocalIdentity) -> std::result::Result<(), String> {
    let identity = normalize_local_identity_for_storage(identity.clone())?;
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.execute(
        "INSERT INTO auth_local_identities
         (id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            identifier_kind = excluded.identifier_kind,
            identifier = excluded.identifier,
            identifier_normalized = excluded.identifier_normalized,
            updated_at = excluded.updated_at,
            state = excluded.state,
            metadata_json = excluded.metadata_json",
        rusqlite::params![
            identity.id,
            identity.identifier_kind,
            identity.identifier,
            identity.identifier_normalized,
            identity.created_at,
            identity.updated_at,
            identity.state.as_str(),
            identity.metadata_json,
        ],
    )
    .map_err(|e| format!("[auth] failed to store local identity: {}", e))?;
    Ok(())
}

fn store_local_identity_and_credential_sqlite(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    let identity = normalize_local_identity_for_storage(identity.clone())?;
    validate_local_credential_secret_for_storage(credential)?;
    validate_local_identity_credential_pair(&identity, credential)?;

    let mut conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_mut().ok_or("SQLite not initialized")?;
    let tx = conn
        .transaction()
        .map_err(|e| format!("[auth] failed to begin local bootstrap transaction: {}", e))?;

    tx.execute(
        "INSERT INTO auth_local_identities
         (id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(id) DO UPDATE SET
            identifier_kind = excluded.identifier_kind,
            identifier = excluded.identifier,
            identifier_normalized = excluded.identifier_normalized,
            updated_at = excluded.updated_at,
            state = excluded.state,
            metadata_json = excluded.metadata_json",
        rusqlite::params![
            identity.id,
            identity.identifier_kind,
            identity.identifier,
            identity.identifier_normalized,
            identity.created_at,
            identity.updated_at,
            identity.state.as_str(),
            identity.metadata_json,
        ],
    )
    .map_err(|e| format!("[auth] failed to store local identity: {}", e))?;

    tx.execute(
        "INSERT INTO auth_local_credentials
         (local_user_id, password_hash, password_hash_algorithm, password_hash_params_json, password_changed_at, must_change_password)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(local_user_id) DO UPDATE SET
            password_hash = excluded.password_hash,
            password_hash_algorithm = excluded.password_hash_algorithm,
            password_hash_params_json = excluded.password_hash_params_json,
            password_changed_at = excluded.password_changed_at,
            must_change_password = excluded.must_change_password",
        rusqlite::params![
            credential.local_user_id,
            credential.password_hash,
            credential.password_hash_algorithm,
            credential.password_hash_params_json,
            credential.password_changed_at,
            if credential.must_change_password { 1 } else { 0 },
        ],
    )
    .map_err(|e| format!("[auth] failed to store local credential: {}", e))?;

    tx.commit()
        .map_err(|e| format!("[auth] failed to commit local bootstrap transaction: {}", e))
}

fn local_identity_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LocalIdentity> {
    let state: String = row.get(6)?;
    let state = LocalAccountState::from_str(&state).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(
            6,
            rusqlite::types::Type::Text,
            Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
        )
    })?;
    Ok(LocalIdentity {
        id: row.get(0)?,
        identifier_kind: row.get(1)?,
        identifier: row.get(2)?,
        identifier_normalized: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
        state,
        metadata_json: row.get(7)?,
    })
}

fn get_local_identity_by_id_sqlite(id: &str) -> std::result::Result<Option<LocalIdentity>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.query_row(
        "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
         FROM auth_local_identities
         WHERE id = ?1",
        rusqlite::params![id],
        local_identity_from_row,
    )
    .optional()
    .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))
}

fn get_local_identity_by_identifier_sqlite(
    identifier_kind: &str,
    identifier_normalized: &str,
) -> std::result::Result<Option<LocalIdentity>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let normalized = identifier_normalized.trim().to_ascii_lowercase();
    conn.query_row(
        "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
         FROM auth_local_identities
         WHERE identifier_kind = ?1 AND identifier_normalized = ?2",
        rusqlite::params![kind, normalized],
        local_identity_from_row,
    )
    .optional()
    .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))
}

fn store_local_credential_secret_sqlite(
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    validate_local_credential_secret_for_storage(credential)?;
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.execute(
        "INSERT INTO auth_local_credentials
         (local_user_id, password_hash, password_hash_algorithm, password_hash_params_json, password_changed_at, must_change_password)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(local_user_id) DO UPDATE SET
            password_hash = excluded.password_hash,
            password_hash_algorithm = excluded.password_hash_algorithm,
            password_hash_params_json = excluded.password_hash_params_json,
            password_changed_at = excluded.password_changed_at,
            must_change_password = excluded.must_change_password",
        rusqlite::params![
            credential.local_user_id,
            credential.password_hash,
            credential.password_hash_algorithm,
            credential.password_hash_params_json,
            credential.password_changed_at,
            if credential.must_change_password { 1 } else { 0 },
        ],
    )
    .map_err(|e| format!("[auth] failed to store local credential: {}", e))?;
    Ok(())
}

fn get_local_credential_secret_sqlite(
    local_user_id: &str,
) -> std::result::Result<Option<LocalCredentialSecret>, String> {
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.query_row(
        "SELECT local_user_id, password_hash, password_hash_algorithm, password_hash_params_json, password_changed_at, must_change_password
         FROM auth_local_credentials
         WHERE local_user_id = ?1",
        rusqlite::params![local_user_id],
        |row| {
            let must_change_password: i64 = row.get(5)?;
            Ok(LocalCredentialSecret {
                local_user_id: row.get(0)?,
                password_hash: row.get(1)?,
                password_hash_algorithm: row.get(2)?,
                password_hash_params_json: row.get(3)?,
                password_changed_at: row.get(4)?,
                must_change_password: must_change_password != 0,
            })
        },
    )
    .optional()
    .map_err(|e| format!("[auth] failed to lookup local credential: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_local_identifier_email() {
        assert_eq!(
            normalize_local_identifier("email", "  Alice@Example.COM  ").unwrap(),
            "alice@example.com"
        );
        assert!(normalize_local_identifier("email", "not-an-email").is_err());
    }

    #[test]
    fn test_memory_local_identity_and_credential_round_trip() {
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

        let mut store = LocalAuthMemoryStore::default();
        store.store_identity(identity.clone()).unwrap();
        store.store_credential_secret(credential.clone()).unwrap();

        assert_eq!(
            store
                .get_identity_by_identifier("email", "alice@example.com")
                .unwrap(),
            Some(identity.clone())
        );
        assert_eq!(
            store.get_credential_secret("local-user-1").unwrap(),
            Some(credential)
        );
    }

    #[test]
    fn test_memory_local_credential_requires_identity() {
        let mut store = LocalAuthMemoryStore::default();
        let credential = LocalCredentialSecret {
            local_user_id: "missing-user".to_string(),
            password_hash: "argon2$hash".to_string(),
            password_hash_algorithm: "argon2id".to_string(),
            password_hash_params_json: "{}".to_string(),
            password_changed_at: 101,
            must_change_password: false,
        };

        assert!(store.store_credential_secret(credential).is_err());
    }

    #[test]
    fn test_memory_local_identity_rejects_identifier_collision() {
        let mut store = LocalAuthMemoryStore::default();
        let first = LocalIdentity {
            id: "local-user-1".to_string(),
            identifier_kind: "email".to_string(),
            identifier: "alice@example.com".to_string(),
            identifier_normalized: "alice@example.com".to_string(),
            created_at: 100,
            updated_at: 100,
            state: LocalAccountState::Active,
            metadata_json: "{}".to_string(),
        };
        let mut second = first.clone();
        second.id = "local-user-2".to_string();

        store.store_identity(first).unwrap();
        assert!(store.store_identity(second).is_err());
    }
}
