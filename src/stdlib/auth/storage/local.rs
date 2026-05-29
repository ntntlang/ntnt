// Local-auth storage includes write-side helpers for credential lifecycle.
// DD-043 keeps app-specific extensions in metadata_json, while security-critical
// lifecycle state such as reset tokens may still require auth-owned helpers/storage.
#![cfg_attr(not(test), allow(dead_code))]

use super::*;
use rusqlite::OptionalExtension;
use std::collections::HashMap;

/// Durable local-auth record families (DD-043).
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::stdlib::auth) struct LocalPasswordResetToken {
    pub(in crate::stdlib::auth) selector: String,
    pub(in crate::stdlib::auth) local_user_id: String,
    pub(in crate::stdlib::auth) token_hash: String,
    pub(in crate::stdlib::auth) created_at: i64,
    pub(in crate::stdlib::auth) expires_at: i64,
}

pub(in crate::stdlib::auth) fn normalize_local_identifier(
    identifier_kind: &str,
    identifier: &str,
) -> std::result::Result<String, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    match kind.as_str() {
        "email" => normalize_email_identifier(identifier),
        "phone" => normalize_phone_identifier(identifier),
        "username" => normalize_username_identifier(identifier),
        "custom" => normalize_custom_identifier(identifier),
        other => Err(format!(
            "[auth] unsupported local identifier kind \"{}\". Supported kinds: email, phone, username, custom",
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

fn normalize_phone_identifier(identifier: &str) -> std::result::Result<String, String> {
    let trimmed = identifier.trim();
    if trimmed.is_empty() {
        return Err("[auth] local phone identifier must not be empty".to_string());
    }

    let mut normalized = String::with_capacity(trimmed.len());
    for (index, ch) in trimmed.chars().enumerate() {
        if ch.is_ascii_digit() {
            normalized.push(ch);
        } else if ch == '+' && index == 0 {
            normalized.push(ch);
        } else if matches!(ch, ' ' | '\t' | '\n' | '\r' | '-' | '.' | '(' | ')') {
            continue;
        } else {
            return Err("[auth] local phone identifier contains invalid characters".to_string());
        }
    }

    let digit_count = normalized.chars().filter(|ch| ch.is_ascii_digit()).count();
    if digit_count < 7 || digit_count > 15 {
        return Err("[auth] local phone identifier must contain 7 to 15 digits".to_string());
    }
    if normalized == "+" || normalized.is_empty() {
        return Err("[auth] local phone identifier must contain digits".to_string());
    }
    Ok(normalized)
}

fn normalize_username_identifier(identifier: &str) -> std::result::Result<String, String> {
    let normalized = identifier.trim().to_ascii_lowercase();
    let len = normalized.len();
    if !(3..=64).contains(&len) {
        return Err("[auth] local username identifier must be 3 to 64 characters".to_string());
    }
    if !normalized
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(
            "[auth] local username identifier may contain only letters, digits, underscore, hyphen, and dot"
                .to_string(),
        );
    }
    let starts_and_ends_with_alnum = normalized
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        && normalized
            .chars()
            .next_back()
            .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit());
    if !starts_and_ends_with_alnum {
        return Err(
            "[auth] local username identifier must start and end with a letter or digit"
                .to_string(),
        );
    }
    Ok(normalized)
}

fn normalize_custom_identifier(identifier: &str) -> std::result::Result<String, String> {
    let normalized = identifier.trim();
    if normalized.is_empty() {
        return Err("[auth] local custom identifier must not be empty".to_string());
    }
    if normalized.chars().any(|ch| ch.is_control()) {
        return Err(
            "[auth] local custom identifier must not contain control characters".to_string(),
        );
    }
    Ok(normalized.to_string())
}

#[derive(Debug, Default, Clone)]
pub(in crate::stdlib::auth) struct LocalAuthMemoryStore {
    identities_by_id: HashMap<String, LocalIdentity>,
    identity_id_by_lookup_key: HashMap<String, String>,
    credential_secrets_by_local_user_id: HashMap<String, LocalCredentialSecret>,
    password_reset_tokens_by_selector: HashMap<String, LocalPasswordResetToken>,
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

    pub(in crate::stdlib::auth) fn update_identity_by_identifier<F>(
        &mut self,
        identifier_kind: &str,
        identifier_normalized: &str,
        updater: F,
    ) -> std::result::Result<Option<LocalIdentity>, String>
    where
        F: FnOnce(&mut LocalIdentity) -> std::result::Result<(), String>,
    {
        let lookup_key = local_identity_lookup_key(identifier_kind, identifier_normalized)?;
        let Some(identity_id) = self.identity_id_by_lookup_key.get(&lookup_key).cloned() else {
            return Ok(None);
        };
        let mut identity = self
            .identities_by_id
            .get(&identity_id)
            .cloned()
            .ok_or_else(|| "[auth] local identity lookup index is corrupt".to_string())?;

        updater(&mut identity)?;
        self.store_identity(identity.clone())?;
        Ok(Some(identity))
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
        self.store_identity_and_credential_with_reset_revocation(identity, credential, false)
    }

    pub(in crate::stdlib::auth) fn store_identity_and_credential_with_reset_revocation(
        &mut self,
        identity: LocalIdentity,
        credential: LocalCredentialSecret,
        revoke_password_resets: bool,
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
            .insert(identity.id.clone(), credential);
        if revoke_password_resets {
            self.password_reset_tokens_by_selector
                .retain(|_, token| token.local_user_id != identity.id);
        }
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

    pub(in crate::stdlib::auth) fn store_password_reset_token(
        &mut self,
        token: LocalPasswordResetToken,
    ) -> std::result::Result<(), String> {
        validate_local_password_reset_token_for_storage(&token)?;
        if !self.identities_by_id.contains_key(&token.local_user_id) {
            return Err(
                "[auth] password reset token must reference an existing local identity".to_string(),
            );
        }
        self.password_reset_tokens_by_selector
            .insert(token.selector.clone(), token);
        Ok(())
    }

    pub(in crate::stdlib::auth) fn consume_password_reset_token_and_store_credential<F>(
        &mut self,
        selector: &str,
        token_hash: &str,
        now: i64,
        credential_builder: F,
    ) -> std::result::Result<Option<(LocalIdentity, LocalCredentialSecret)>, String>
    where
        F: FnOnce(&str) -> std::result::Result<LocalCredentialSecret, String>,
    {
        let Some(reset_token) = self
            .password_reset_tokens_by_selector
            .get(selector)
            .cloned()
        else {
            return Ok(None);
        };
        if reset_token.expires_at <= now {
            self.password_reset_tokens_by_selector.remove(selector);
            return Ok(None);
        }
        if !constant_time_compare(&reset_token.token_hash, token_hash) {
            return Ok(None);
        }
        let Some(identity) = self
            .identities_by_id
            .get(&reset_token.local_user_id)
            .cloned()
        else {
            return Ok(None);
        };
        if matches!(
            identity.state,
            LocalAccountState::Disabled | LocalAccountState::Locked
        ) {
            return Ok(None);
        }
        let credential = credential_builder(&reset_token.local_user_id)?;
        validate_local_credential_secret_for_storage(&credential)?;
        let identity = LocalIdentity {
            updated_at: now,
            state: LocalAccountState::Active,
            ..identity
        };
        self.store_identity_and_credential(identity.clone(), credential.clone())?;
        self.password_reset_tokens_by_selector
            .retain(|_, token| token.local_user_id != reset_token.local_user_id);
        Ok(Some((identity, credential)))
    }
}

fn local_identity_lookup_key(
    identifier_kind: &str,
    identifier_normalized: &str,
) -> std::result::Result<String, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let normalized = identifier_normalized.trim();
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

fn validate_local_password_reset_token_for_storage(
    token: &LocalPasswordResetToken,
) -> std::result::Result<(), String> {
    if token.selector.trim().is_empty() {
        return Err("[auth] password reset token selector must not be empty".to_string());
    }
    if token.local_user_id.trim().is_empty() {
        return Err("[auth] password reset token local_user_id must not be empty".to_string());
    }
    if token.token_hash.trim().is_empty() {
        return Err("[auth] password reset token hash must not be empty".to_string());
    }
    if token.expires_at <= token.created_at {
        return Err("[auth] password reset token expires_at must be after created_at".to_string());
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
        AuthStorageBackend::Postgres => store_local_identity_postgres(identity),
        AuthStorageBackend::Redis => store_local_identity_redis(identity),
    }
}

pub(in crate::stdlib::auth) fn store_local_identity_and_credential_record(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    store_local_identity_and_credential_record_inner(identity, credential, false)
}

pub(in crate::stdlib::auth) fn store_local_identity_and_credential_revoke_password_resets_record(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    store_local_identity_and_credential_record_inner(identity, credential, true)
}

fn store_local_identity_and_credential_record_inner(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
    revoke_password_resets: bool,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .store_identity_and_credential_with_reset_revocation(
                identity.clone(),
                credential.clone(),
                revoke_password_resets,
            ),
        AuthStorageBackend::Sqlite => {
            store_local_identity_and_credential_sqlite(identity, credential, revoke_password_resets)
        }
        AuthStorageBackend::Postgres => store_local_identity_and_credential_postgres(
            identity,
            credential,
            revoke_password_resets,
        ),
        AuthStorageBackend::Redis => {
            store_local_identity_and_credential_redis(identity, credential, revoke_password_resets)
        }
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
        AuthStorageBackend::Postgres => get_local_identity_by_id_postgres(id),
        AuthStorageBackend::Redis => get_local_identity_by_id_redis(id),
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
            get_local_identity_by_identifier_postgres(identifier_kind, identifier_normalized)
        }
        AuthStorageBackend::Redis => {
            get_local_identity_by_identifier_redis(identifier_kind, identifier_normalized)
        }
    }
}

pub(in crate::stdlib::auth) fn update_local_identity_by_identifier_record<F>(
    identifier_kind: &str,
    identifier_normalized: &str,
    updater: F,
) -> std::result::Result<Option<LocalIdentity>, String>
where
    F: FnOnce(&mut LocalIdentity) -> std::result::Result<(), String>,
{
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .update_identity_by_identifier(identifier_kind, identifier_normalized, updater),
        AuthStorageBackend::Sqlite => update_local_identity_by_identifier_sqlite(
            identifier_kind,
            identifier_normalized,
            updater,
        ),
        AuthStorageBackend::Postgres => update_local_identity_by_identifier_postgres(
            identifier_kind,
            identifier_normalized,
            updater,
        ),
        AuthStorageBackend::Redis => update_local_identity_by_identifier_redis(
            identifier_kind,
            identifier_normalized,
            updater,
        ),
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
        AuthStorageBackend::Postgres => store_local_credential_secret_postgres(credential),
        AuthStorageBackend::Redis => store_local_credential_secret_redis(credential),
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
        AuthStorageBackend::Postgres => get_local_credential_secret_postgres(local_user_id),
        AuthStorageBackend::Redis => get_local_credential_secret_redis(local_user_id),
    }
}

pub(in crate::stdlib::auth) fn store_local_password_reset_token_record(
    token: &LocalPasswordResetToken,
) -> std::result::Result<(), String> {
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .store_password_reset_token(token.clone()),
        AuthStorageBackend::Sqlite => store_local_password_reset_token_sqlite(token),
        AuthStorageBackend::Postgres => store_local_password_reset_token_postgres(token),
        AuthStorageBackend::Redis => store_local_password_reset_token_redis(token),
    }
}

pub(in crate::stdlib::auth) fn consume_local_password_reset_token_and_store_credential_record<F>(
    selector: &str,
    token_hash: &str,
    now: i64,
    credential_builder: F,
) -> std::result::Result<Option<(LocalIdentity, LocalCredentialSecret)>, String>
where
    F: FnOnce(&str) -> std::result::Result<LocalCredentialSecret, String>,
{
    match active_auth_storage_backend() {
        AuthStorageBackend::Memory => SESSION_STORE
            .lock()
            .unwrap()
            .local_auth
            .consume_password_reset_token_and_store_credential(
                selector,
                token_hash,
                now,
                credential_builder,
            ),
        AuthStorageBackend::Sqlite => {
            consume_local_password_reset_token_and_store_credential_sqlite(
                selector,
                token_hash,
                now,
                credential_builder,
            )
        }
        AuthStorageBackend::Postgres => {
            consume_local_password_reset_token_and_store_credential_postgres(
                selector,
                token_hash,
                now,
                credential_builder,
            )
        }
        AuthStorageBackend::Redis => consume_local_password_reset_token_and_store_credential_redis(
            selector,
            token_hash,
            now,
            credential_builder,
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
    revoke_password_resets: bool,
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

    if revoke_password_resets {
        tx.execute(
            "DELETE FROM auth_local_password_reset_tokens WHERE local_user_id = ?1",
            rusqlite::params![identity.id],
        )
        .map_err(|e| format!("[auth] failed to delete password reset tokens: {}", e))?;
    }

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
    let normalized = identifier_normalized.trim().to_string();
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

fn update_local_identity_by_identifier_sqlite<F>(
    identifier_kind: &str,
    identifier_normalized: &str,
    updater: F,
) -> std::result::Result<Option<LocalIdentity>, String>
where
    F: FnOnce(&mut LocalIdentity) -> std::result::Result<(), String>,
{
    let mut conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_mut().ok_or("SQLite not initialized")?;
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let normalized = identifier_normalized.trim().to_string();
    let tx = conn.transaction().map_err(|e| {
        format!(
            "[auth] failed to begin local identity update transaction: {}",
            e
        )
    })?;

    let Some(mut identity) = tx
        .query_row(
            "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
             FROM auth_local_identities
             WHERE identifier_kind = ?1 AND identifier_normalized = ?2",
            rusqlite::params![kind, normalized],
            local_identity_from_row,
        )
        .optional()
        .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))?
    else {
        tx.commit()
            .map_err(|e| format!("[auth] failed to commit local identity update transaction: {}", e))?;
        return Ok(None);
    };

    updater(&mut identity)?;
    let identity = normalize_local_identity_for_storage(identity)?;
    tx.execute(
        "UPDATE auth_local_identities
         SET identifier_kind = ?2,
             identifier = ?3,
             identifier_normalized = ?4,
             updated_at = ?5,
             state = ?6,
             metadata_json = ?7
         WHERE id = ?1",
        rusqlite::params![
            identity.id,
            identity.identifier_kind,
            identity.identifier,
            identity.identifier_normalized,
            identity.updated_at,
            identity.state.as_str(),
            identity.metadata_json,
        ],
    )
    .map_err(|e| format!("[auth] failed to update local identity: {}", e))?;
    tx.commit().map_err(|e| {
        format!(
            "[auth] failed to commit local identity update transaction: {}",
            e
        )
    })?;

    Ok(Some(identity))
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

fn store_local_password_reset_token_sqlite(
    token: &LocalPasswordResetToken,
) -> std::result::Result<(), String> {
    validate_local_password_reset_token_for_storage(token)?;
    let conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_ref().ok_or("SQLite not initialized")?;
    conn.execute(
        "INSERT INTO auth_local_password_reset_tokens
         (selector, local_user_id, token_hash, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(selector) DO UPDATE SET
            local_user_id = excluded.local_user_id,
            token_hash = excluded.token_hash,
            created_at = excluded.created_at,
            expires_at = excluded.expires_at",
        rusqlite::params![
            token.selector,
            token.local_user_id,
            token.token_hash,
            token.created_at,
            token.expires_at,
        ],
    )
    .map_err(|e| format!("[auth] failed to store password reset token: {}", e))?;
    Ok(())
}

fn consume_local_password_reset_token_and_store_credential_sqlite<F>(
    selector: &str,
    token_hash: &str,
    now: i64,
    credential_builder: F,
) -> std::result::Result<Option<(LocalIdentity, LocalCredentialSecret)>, String>
where
    F: FnOnce(&str) -> std::result::Result<LocalCredentialSecret, String>,
{
    let mut conn_guard = SQLITE_CONN.lock().unwrap();
    let conn = conn_guard.as_mut().ok_or("SQLite not initialized")?;
    let tx = conn.transaction().map_err(|e| {
        format!(
            "[auth] failed to begin password reset consume transaction: {}",
            e
        )
    })?;

    let Some(reset_token) = tx
        .query_row(
            "SELECT selector, local_user_id, token_hash, created_at, expires_at
             FROM auth_local_password_reset_tokens
             WHERE selector = ?1",
            rusqlite::params![selector],
            |row| {
                Ok(LocalPasswordResetToken {
                    selector: row.get(0)?,
                    local_user_id: row.get(1)?,
                    token_hash: row.get(2)?,
                    created_at: row.get(3)?,
                    expires_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(|e| format!("[auth] failed to lookup password reset token: {}", e))?
    else {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    };

    if reset_token.expires_at <= now {
        tx.execute(
            "DELETE FROM auth_local_password_reset_tokens WHERE selector = ?1",
            rusqlite::params![selector],
        )
        .map_err(|e| {
            format!(
                "[auth] failed to delete expired password reset token: {}",
                e
            )
        })?;
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    }
    if !constant_time_compare(&reset_token.token_hash, token_hash) {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    }
    let credential = credential_builder(&reset_token.local_user_id)?;
    validate_local_credential_secret_for_storage(&credential)?;

    let Some(identity) = tx
        .query_row(
            "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
             FROM auth_local_identities
             WHERE id = ?1",
            rusqlite::params![reset_token.local_user_id],
            local_identity_from_row,
        )
        .optional()
        .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))?
    else {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    };
    if matches!(
        identity.state,
        LocalAccountState::Disabled | LocalAccountState::Locked
    ) {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    }

    let identity = normalize_local_identity_for_storage(LocalIdentity {
        updated_at: now,
        state: LocalAccountState::Active,
        ..identity
    })?;
    tx.execute(
        "UPDATE auth_local_identities
         SET updated_at = ?2, state = ?3
         WHERE id = ?1",
        rusqlite::params![identity.id, identity.updated_at, identity.state.as_str()],
    )
    .map_err(|e| format!("[auth] failed to update local identity: {}", e))?;
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
    tx.execute(
        "DELETE FROM auth_local_password_reset_tokens WHERE local_user_id = ?1",
        rusqlite::params![reset_token.local_user_id],
    )
    .map_err(|e| format!("[auth] failed to delete password reset tokens: {}", e))?;
    tx.commit().map_err(|e| {
        format!(
            "[auth] failed to commit password reset consume transaction: {}",
            e
        )
    })?;

    Ok(Some((identity, credential.clone())))
}

fn postgres_url() -> std::result::Result<String, String> {
    POSTGRES_URL
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "PostgreSQL not initialized".to_string())
}

fn redis_connection() -> std::result::Result<redis::Connection, String> {
    let url = REDIS_URL
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "Redis not initialized".to_string())?;
    let client =
        redis::Client::open(url.as_str()).map_err(|e| format!("Redis client error: {}", e))?;
    client
        .get_connection()
        .map_err(|e| format!("Redis connection error: {}", e))
}

fn local_identity_from_postgres_row(
    row: &postgres::Row,
) -> std::result::Result<LocalIdentity, String> {
    let state: String = row.get(6);
    Ok(LocalIdentity {
        id: row.get(0),
        identifier_kind: row.get(1),
        identifier: row.get(2),
        identifier_normalized: row.get(3),
        created_at: row.get(4),
        updated_at: row.get(5),
        state: LocalAccountState::from_str(&state)?,
        metadata_json: row.get(7),
    })
}

fn store_local_identity_postgres(identity: &LocalIdentity) -> std::result::Result<(), String> {
    let identity = normalize_local_identity_for_storage(identity.clone())?;
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    client.execute(
        "INSERT INTO auth_local_identities
         (id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET
            identifier_kind = EXCLUDED.identifier_kind,
            identifier = EXCLUDED.identifier,
            identifier_normalized = EXCLUDED.identifier_normalized,
            updated_at = EXCLUDED.updated_at,
            state = EXCLUDED.state,
            metadata_json = EXCLUDED.metadata_json",
        &[
            &identity.id,
            &identity.identifier_kind,
            &identity.identifier,
            &identity.identifier_normalized,
            &identity.created_at,
            &identity.updated_at,
            &identity.state.as_str(),
            &identity.metadata_json,
        ],
    ).map_err(|e| format!("[auth] failed to store local identity: {}", e))?;
    Ok(())
}

fn store_local_identity_and_credential_postgres(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
    revoke_password_resets: bool,
) -> std::result::Result<(), String> {
    let identity = normalize_local_identity_for_storage(identity.clone())?;
    validate_local_credential_secret_for_storage(credential)?;
    validate_local_identity_credential_pair(&identity, credential)?;

    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    let mut tx = client
        .transaction()
        .map_err(|e| format!("[auth] failed to begin local bootstrap transaction: {}", e))?;

    tx.execute(
        "INSERT INTO auth_local_identities
         (id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (id) DO UPDATE SET
            identifier_kind = EXCLUDED.identifier_kind,
            identifier = EXCLUDED.identifier,
            identifier_normalized = EXCLUDED.identifier_normalized,
            updated_at = EXCLUDED.updated_at,
            state = EXCLUDED.state,
            metadata_json = EXCLUDED.metadata_json",
        &[
            &identity.id,
            &identity.identifier_kind,
            &identity.identifier,
            &identity.identifier_normalized,
            &identity.created_at,
            &identity.updated_at,
            &identity.state.as_str(),
            &identity.metadata_json,
        ],
    ).map_err(|e| format!("[auth] failed to store local identity: {}", e))?;

    tx.execute(
        "INSERT INTO auth_local_credentials
         (local_user_id, password_hash, password_hash_algorithm, password_hash_params_json, password_changed_at, must_change_password)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (local_user_id) DO UPDATE SET
            password_hash = EXCLUDED.password_hash,
            password_hash_algorithm = EXCLUDED.password_hash_algorithm,
            password_hash_params_json = EXCLUDED.password_hash_params_json,
            password_changed_at = EXCLUDED.password_changed_at,
            must_change_password = EXCLUDED.must_change_password",
        &[
            &credential.local_user_id,
            &credential.password_hash,
            &credential.password_hash_algorithm,
            &credential.password_hash_params_json,
            &credential.password_changed_at,
            &credential.must_change_password,
        ],
    ).map_err(|e| format!("[auth] failed to store local credential: {}", e))?;

    if revoke_password_resets {
        tx.execute(
            "DELETE FROM auth_local_password_reset_tokens WHERE local_user_id = $1",
            &[&identity.id],
        )
        .map_err(|e| format!("[auth] failed to delete password reset tokens: {}", e))?;
    }

    tx.commit()
        .map_err(|e| format!("[auth] failed to commit local bootstrap transaction: {}", e))
}

fn get_local_identity_by_id_postgres(
    id: &str,
) -> std::result::Result<Option<LocalIdentity>, String> {
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    let rows = client
        .query(
            "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
             FROM auth_local_identities WHERE id = $1",
            &[&id],
        )
        .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))?;
    rows.first()
        .map(local_identity_from_postgres_row)
        .transpose()
}

fn get_local_identity_by_identifier_postgres(
    identifier_kind: &str,
    identifier_normalized: &str,
) -> std::result::Result<Option<LocalIdentity>, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let normalized = identifier_normalized.trim().to_string();
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    let rows = client
        .query(
            "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
             FROM auth_local_identities WHERE identifier_kind = $1 AND identifier_normalized = $2",
            &[&kind, &normalized],
        )
        .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))?;
    rows.first()
        .map(local_identity_from_postgres_row)
        .transpose()
}

fn update_local_identity_by_identifier_postgres<F>(
    identifier_kind: &str,
    identifier_normalized: &str,
    updater: F,
) -> std::result::Result<Option<LocalIdentity>, String>
where
    F: FnOnce(&mut LocalIdentity) -> std::result::Result<(), String>,
{
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let normalized = identifier_normalized.trim().to_string();
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    let mut tx = client.transaction().map_err(|e| {
        format!(
            "[auth] failed to begin local identity update transaction: {}",
            e
        )
    })?;
    let rows = tx
        .query(
            "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
             FROM auth_local_identities WHERE identifier_kind = $1 AND identifier_normalized = $2 FOR UPDATE",
            &[&kind, &normalized],
        )
        .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))?;
    let Some(row) = rows.first() else {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit local identity update transaction: {}",
                e
            )
        })?;
        return Ok(None);
    };
    let mut identity = local_identity_from_postgres_row(row)?;
    updater(&mut identity)?;
    let identity = normalize_local_identity_for_storage(identity)?;
    tx.execute(
        "UPDATE auth_local_identities
         SET identifier_kind = $2, identifier = $3, identifier_normalized = $4,
             updated_at = $5, state = $6, metadata_json = $7
         WHERE id = $1",
        &[
            &identity.id,
            &identity.identifier_kind,
            &identity.identifier,
            &identity.identifier_normalized,
            &identity.updated_at,
            &identity.state.as_str(),
            &identity.metadata_json,
        ],
    )
    .map_err(|e| format!("[auth] failed to update local identity: {}", e))?;
    tx.commit().map_err(|e| {
        format!(
            "[auth] failed to commit local identity update transaction: {}",
            e
        )
    })?;
    Ok(Some(identity))
}

fn store_local_credential_secret_postgres(
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    validate_local_credential_secret_for_storage(credential)?;
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    client.execute(
        "INSERT INTO auth_local_credentials
         (local_user_id, password_hash, password_hash_algorithm, password_hash_params_json, password_changed_at, must_change_password)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (local_user_id) DO UPDATE SET
            password_hash = EXCLUDED.password_hash,
            password_hash_algorithm = EXCLUDED.password_hash_algorithm,
            password_hash_params_json = EXCLUDED.password_hash_params_json,
            password_changed_at = EXCLUDED.password_changed_at,
            must_change_password = EXCLUDED.must_change_password",
        &[
            &credential.local_user_id,
            &credential.password_hash,
            &credential.password_hash_algorithm,
            &credential.password_hash_params_json,
            &credential.password_changed_at,
            &credential.must_change_password,
        ],
    ).map_err(|e| format!("[auth] failed to store local credential: {}", e))?;
    Ok(())
}

fn get_local_credential_secret_postgres(
    local_user_id: &str,
) -> std::result::Result<Option<LocalCredentialSecret>, String> {
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    let rows = client
        .query(
            "SELECT local_user_id, password_hash, password_hash_algorithm, password_hash_params_json, password_changed_at, must_change_password
             FROM auth_local_credentials WHERE local_user_id = $1",
            &[&local_user_id],
        )
        .map_err(|e| format!("[auth] failed to lookup local credential: {}", e))?;
    Ok(rows.first().map(|row| LocalCredentialSecret {
        local_user_id: row.get(0),
        password_hash: row.get(1),
        password_hash_algorithm: row.get(2),
        password_hash_params_json: row.get(3),
        password_changed_at: row.get(4),
        must_change_password: row.get(5),
    }))
}

fn store_local_password_reset_token_postgres(
    token: &LocalPasswordResetToken,
) -> std::result::Result<(), String> {
    validate_local_password_reset_token_for_storage(token)?;
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    client
        .execute(
            "INSERT INTO auth_local_password_reset_tokens
         (selector, local_user_id, token_hash, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (selector) DO UPDATE SET
            local_user_id = EXCLUDED.local_user_id,
            token_hash = EXCLUDED.token_hash,
            created_at = EXCLUDED.created_at,
            expires_at = EXCLUDED.expires_at",
            &[
                &token.selector,
                &token.local_user_id,
                &token.token_hash,
                &token.created_at,
                &token.expires_at,
            ],
        )
        .map_err(|e| format!("[auth] failed to store password reset token: {}", e))?;
    Ok(())
}

fn consume_local_password_reset_token_and_store_credential_postgres<F>(
    selector: &str,
    token_hash: &str,
    now: i64,
    credential_builder: F,
) -> std::result::Result<Option<(LocalIdentity, LocalCredentialSecret)>, String>
where
    F: FnOnce(&str) -> std::result::Result<LocalCredentialSecret, String>,
{
    let url = postgres_url()?;
    let mut client = postgres::Client::connect(&url, postgres::NoTls).map_err(|e| e.to_string())?;
    let mut tx = client.transaction().map_err(|e| {
        format!(
            "[auth] failed to begin password reset consume transaction: {}",
            e
        )
    })?;

    let rows = tx
        .query(
            "SELECT selector, local_user_id, token_hash, created_at, expires_at
             FROM auth_local_password_reset_tokens WHERE selector = $1 FOR UPDATE",
            &[&selector],
        )
        .map_err(|e| format!("[auth] failed to lookup password reset token: {}", e))?;
    let Some(row) = rows.first() else {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    };
    let reset_token = LocalPasswordResetToken {
        selector: row.get(0),
        local_user_id: row.get(1),
        token_hash: row.get(2),
        created_at: row.get(3),
        expires_at: row.get(4),
    };
    if reset_token.expires_at <= now {
        tx.execute(
            "DELETE FROM auth_local_password_reset_tokens WHERE selector = $1",
            &[&selector],
        )
        .map_err(|e| {
            format!(
                "[auth] failed to delete expired password reset token: {}",
                e
            )
        })?;
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    }
    if !constant_time_compare(&reset_token.token_hash, token_hash) {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    }
    let credential = credential_builder(&reset_token.local_user_id)?;
    validate_local_credential_secret_for_storage(&credential)?;
    let rows = tx
        .query(
            "SELECT id, identifier_kind, identifier, identifier_normalized, created_at, updated_at, state, metadata_json
             FROM auth_local_identities WHERE id = $1 FOR UPDATE",
            &[&reset_token.local_user_id],
        )
        .map_err(|e| format!("[auth] failed to lookup local identity: {}", e))?;
    let Some(row) = rows.first() else {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    };
    let identity = local_identity_from_postgres_row(row)?;
    if matches!(
        identity.state,
        LocalAccountState::Disabled | LocalAccountState::Locked
    ) {
        tx.commit().map_err(|e| {
            format!(
                "[auth] failed to commit password reset consume transaction: {}",
                e
            )
        })?;
        return Ok(None);
    }
    let identity = normalize_local_identity_for_storage(LocalIdentity {
        updated_at: now,
        state: LocalAccountState::Active,
        ..identity
    })?;
    tx.execute(
        "UPDATE auth_local_identities SET updated_at = $2, state = $3 WHERE id = $1",
        &[&identity.id, &identity.updated_at, &identity.state.as_str()],
    )
    .map_err(|e| format!("[auth] failed to update local identity: {}", e))?;
    tx.execute(
        "INSERT INTO auth_local_credentials
         (local_user_id, password_hash, password_hash_algorithm, password_hash_params_json, password_changed_at, must_change_password)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (local_user_id) DO UPDATE SET
            password_hash = EXCLUDED.password_hash,
            password_hash_algorithm = EXCLUDED.password_hash_algorithm,
            password_hash_params_json = EXCLUDED.password_hash_params_json,
            password_changed_at = EXCLUDED.password_changed_at,
            must_change_password = EXCLUDED.must_change_password",
        &[
            &credential.local_user_id,
            &credential.password_hash,
            &credential.password_hash_algorithm,
            &credential.password_hash_params_json,
            &credential.password_changed_at,
            &credential.must_change_password,
        ],
    ).map_err(|e| format!("[auth] failed to store local credential: {}", e))?;
    tx.execute(
        "DELETE FROM auth_local_password_reset_tokens WHERE local_user_id = $1",
        &[&reset_token.local_user_id],
    )
    .map_err(|e| format!("[auth] failed to delete password reset tokens: {}", e))?;
    tx.commit().map_err(|e| {
        format!(
            "[auth] failed to commit password reset consume transaction: {}",
            e
        )
    })?;
    Ok(Some((identity, credential)))
}

fn redis_local_identity_key(id: &str) -> String {
    format!("ntnt:local_identity:{}", id)
}

fn redis_local_identity_lookup_key(identifier_kind: &str, identifier_normalized: &str) -> String {
    format!(
        "ntnt:local_identity_lookup:{}:{}",
        identifier_kind.trim().to_ascii_lowercase(),
        identifier_normalized.trim()
    )
}

fn redis_local_credential_key(local_user_id: &str) -> String {
    format!("ntnt:local_credential:{}", local_user_id)
}

fn redis_local_password_reset_key(selector: &str) -> String {
    format!("ntnt:local_password_reset:{}", selector)
}

fn redis_local_password_reset_user_set_key(local_user_id: &str) -> String {
    format!("ntnt:local_password_resets_for_user:{}", local_user_id)
}

fn local_identity_to_json(identity: &LocalIdentity) -> String {
    serde_json::json!({
        "id": identity.id,
        "identifier_kind": identity.identifier_kind,
        "identifier": identity.identifier,
        "identifier_normalized": identity.identifier_normalized,
        "created_at": identity.created_at,
        "updated_at": identity.updated_at,
        "state": identity.state.as_str(),
        "metadata_json": identity.metadata_json,
    })
    .to_string()
}

fn local_identity_from_json(json_str: &str) -> std::result::Result<LocalIdentity, String> {
    let json: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("[auth] failed to parse local identity JSON: {}", e))?;
    let state = json["state"]
        .as_str()
        .ok_or_else(|| "[auth] local identity JSON missing state".to_string())?;
    Ok(LocalIdentity {
        id: json["id"]
            .as_str()
            .ok_or_else(|| "[auth] local identity JSON missing id".to_string())?
            .to_string(),
        identifier_kind: json["identifier_kind"]
            .as_str()
            .ok_or_else(|| "[auth] local identity JSON missing identifier_kind".to_string())?
            .to_string(),
        identifier: json["identifier"]
            .as_str()
            .ok_or_else(|| "[auth] local identity JSON missing identifier".to_string())?
            .to_string(),
        identifier_normalized: json["identifier_normalized"]
            .as_str()
            .ok_or_else(|| "[auth] local identity JSON missing identifier_normalized".to_string())?
            .to_string(),
        created_at: json["created_at"]
            .as_i64()
            .ok_or_else(|| "[auth] local identity JSON missing created_at".to_string())?,
        updated_at: json["updated_at"]
            .as_i64()
            .ok_or_else(|| "[auth] local identity JSON missing updated_at".to_string())?,
        state: LocalAccountState::from_str(state)?,
        metadata_json: json["metadata_json"]
            .as_str()
            .ok_or_else(|| "[auth] local identity JSON missing metadata_json".to_string())?
            .to_string(),
    })
}

fn local_credential_to_json(credential: &LocalCredentialSecret) -> String {
    serde_json::json!({
        "local_user_id": credential.local_user_id,
        "password_hash": credential.password_hash,
        "password_hash_algorithm": credential.password_hash_algorithm,
        "password_hash_params_json": credential.password_hash_params_json,
        "password_changed_at": credential.password_changed_at,
        "must_change_password": credential.must_change_password,
    })
    .to_string()
}

fn local_credential_from_json(
    json_str: &str,
) -> std::result::Result<LocalCredentialSecret, String> {
    let json: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("[auth] failed to parse local credential JSON: {}", e))?;
    Ok(LocalCredentialSecret {
        local_user_id: json["local_user_id"]
            .as_str()
            .ok_or_else(|| "[auth] local credential JSON missing local_user_id".to_string())?
            .to_string(),
        password_hash: json["password_hash"]
            .as_str()
            .ok_or_else(|| "[auth] local credential JSON missing password_hash".to_string())?
            .to_string(),
        password_hash_algorithm: json["password_hash_algorithm"]
            .as_str()
            .ok_or_else(|| {
                "[auth] local credential JSON missing password_hash_algorithm".to_string()
            })?
            .to_string(),
        password_hash_params_json: json["password_hash_params_json"]
            .as_str()
            .ok_or_else(|| {
                "[auth] local credential JSON missing password_hash_params_json".to_string()
            })?
            .to_string(),
        password_changed_at: json["password_changed_at"].as_i64().ok_or_else(|| {
            "[auth] local credential JSON missing password_changed_at".to_string()
        })?,
        must_change_password: json["must_change_password"].as_bool().unwrap_or(false),
    })
}

fn local_password_reset_token_to_json(token: &LocalPasswordResetToken) -> String {
    serde_json::json!({
        "selector": token.selector,
        "local_user_id": token.local_user_id,
        "token_hash": token.token_hash,
        "created_at": token.created_at,
        "expires_at": token.expires_at,
    })
    .to_string()
}

fn local_password_reset_token_from_json(
    json_str: &str,
) -> std::result::Result<LocalPasswordResetToken, String> {
    let json: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("[auth] failed to parse password reset JSON: {}", e))?;
    Ok(LocalPasswordResetToken {
        selector: json["selector"]
            .as_str()
            .ok_or_else(|| "[auth] password reset JSON missing selector".to_string())?
            .to_string(),
        local_user_id: json["local_user_id"]
            .as_str()
            .ok_or_else(|| "[auth] password reset JSON missing local_user_id".to_string())?
            .to_string(),
        token_hash: json["token_hash"]
            .as_str()
            .ok_or_else(|| "[auth] password reset JSON missing token_hash".to_string())?
            .to_string(),
        created_at: json["created_at"]
            .as_i64()
            .ok_or_else(|| "[auth] password reset JSON missing created_at".to_string())?,
        expires_at: json["expires_at"]
            .as_i64()
            .ok_or_else(|| "[auth] password reset JSON missing expires_at".to_string())?,
    })
}

fn store_local_identity_redis(identity: &LocalIdentity) -> std::result::Result<(), String> {
    let identity = normalize_local_identity_for_storage(identity.clone())?;
    let mut conn = redis_connection()?;
    store_local_identity_redis_with_connection(&mut conn, &identity)
}

fn store_local_identity_redis_with_connection(
    conn: &mut redis::Connection,
    identity: &LocalIdentity,
) -> std::result::Result<(), String> {
    let lookup_key =
        redis_local_identity_lookup_key(&identity.identifier_kind, &identity.identifier_normalized);
    let identity_key = redis_local_identity_key(&identity.id);
    let stored: i64 = redis::Script::new(
        r#"
        local existing_id = redis.call('GET', KEYS[2])
        if existing_id and existing_id ~= ARGV[1] then
            return -1
        end

        local previous_identity = redis.call('GET', KEYS[1])
        if previous_identity then
            local previous = cjson.decode(previous_identity)
            local previous_lookup_key = 'ntnt:local_identity_lookup:' .. previous.identifier_kind .. ':' .. previous.identifier_normalized
            if previous_lookup_key ~= KEYS[2] then
                redis.call('DEL', previous_lookup_key)
            end
        end

        redis.call('SET', KEYS[1], ARGV[2])
        redis.call('SET', KEYS[2], ARGV[1])
        return 1
        "#,
    )
    .key(&identity_key)
    .key(&lookup_key)
    .arg(&identity.id)
    .arg(local_identity_to_json(identity))
    .invoke(conn)
    .map_err(|e| format!("Redis local identity store error: {}", e))?;

    if stored == -1 {
        return Err(format!(
            "[auth] local identity identifier already exists for {}",
            identity.identifier_kind
        ));
    }
    Ok(())
}

fn store_local_identity_and_credential_redis(
    identity: &LocalIdentity,
    credential: &LocalCredentialSecret,
    revoke_password_resets: bool,
) -> std::result::Result<(), String> {
    let identity = normalize_local_identity_for_storage(identity.clone())?;
    validate_local_credential_secret_for_storage(credential)?;
    validate_local_identity_credential_pair(&identity, credential)?;
    let mut conn = redis_connection()?;
    let identity_key = redis_local_identity_key(&identity.id);
    let lookup_key =
        redis_local_identity_lookup_key(&identity.identifier_kind, &identity.identifier_normalized);
    let credential_key = redis_local_credential_key(&credential.local_user_id);
    let reset_set_key = redis_local_password_reset_user_set_key(&identity.id);
    let stored: i64 = redis::Script::new(
        r#"
        local existing_id = redis.call('GET', KEYS[2])
        if existing_id and existing_id ~= ARGV[1] then
            return -1
        end

        local previous_identity = redis.call('GET', KEYS[1])
        if previous_identity then
            local previous = cjson.decode(previous_identity)
            local previous_lookup_key = 'ntnt:local_identity_lookup:' .. previous.identifier_kind .. ':' .. previous.identifier_normalized
            if previous_lookup_key ~= KEYS[2] then
                redis.call('DEL', previous_lookup_key)
            end
        end

        redis.call('SET', KEYS[1], ARGV[2])
        redis.call('SET', KEYS[2], ARGV[1])
        redis.call('SET', KEYS[3], ARGV[3])

        if ARGV[4] == '1' then
            local reset_keys = redis.call('SMEMBERS', KEYS[4])
            for _, key in ipairs(reset_keys) do
                redis.call('DEL', key)
            end
            redis.call('DEL', KEYS[4])
        end

        return 1
        "#,
    )
    .key(&identity_key)
    .key(&lookup_key)
    .key(&credential_key)
    .key(&reset_set_key)
    .arg(&identity.id)
    .arg(local_identity_to_json(&identity))
    .arg(local_credential_to_json(credential))
    .arg(if revoke_password_resets { "1" } else { "0" })
    .invoke(&mut conn)
    .map_err(|e| format!("Redis local identity+credential store error: {}", e))?;

    if stored == -1 {
        return Err(format!(
            "[auth] local identity identifier already exists for {}",
            identity.identifier_kind
        ));
    }
    Ok(())
}

fn get_local_identity_by_id_redis(id: &str) -> std::result::Result<Option<LocalIdentity>, String> {
    let mut conn = redis_connection()?;
    let json: Option<String> = redis::cmd("GET")
        .arg(redis_local_identity_key(id))
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;
    json.map(|json| local_identity_from_json(&json)).transpose()
}

fn get_local_identity_by_identifier_redis(
    identifier_kind: &str,
    identifier_normalized: &str,
) -> std::result::Result<Option<LocalIdentity>, String> {
    let mut conn = redis_connection()?;
    let lookup_key = redis_local_identity_lookup_key(identifier_kind, identifier_normalized);
    let id: Option<String> = redis::cmd("GET")
        .arg(lookup_key)
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;
    match id {
        Some(id) => get_local_identity_by_id_redis(&id),
        None => Ok(None),
    }
}

fn update_local_identity_by_identifier_redis<F>(
    identifier_kind: &str,
    identifier_normalized: &str,
    updater: F,
) -> std::result::Result<Option<LocalIdentity>, String>
where
    F: FnOnce(&mut LocalIdentity) -> std::result::Result<(), String>,
{
    let mut conn = redis_connection()?;
    let lookup_key = redis_local_identity_lookup_key(identifier_kind, identifier_normalized);
    let id: Option<String> = redis::cmd("GET")
        .arg(lookup_key)
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;
    let Some(id) = id else {
        return Ok(None);
    };
    let identity_json: Option<String> = redis::cmd("GET")
        .arg(redis_local_identity_key(&id))
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;
    let Some(identity_json) = identity_json else {
        return Ok(None);
    };
    let mut identity = local_identity_from_json(&identity_json)?;
    updater(&mut identity)?;
    let identity = normalize_local_identity_for_storage(identity)?;
    store_local_identity_redis_with_connection(&mut conn, &identity)?;
    Ok(Some(identity))
}

fn store_local_credential_secret_redis(
    credential: &LocalCredentialSecret,
) -> std::result::Result<(), String> {
    validate_local_credential_secret_for_storage(credential)?;
    if get_local_identity_by_id_redis(&credential.local_user_id)?.is_none() {
        return Err(
            "[auth] local credential must reference an existing local identity".to_string(),
        );
    }
    let mut conn = redis_connection()?;
    redis::cmd("SET")
        .arg(redis_local_credential_key(&credential.local_user_id))
        .arg(local_credential_to_json(credential))
        .query::<()>(&mut conn)
        .map_err(|e| format!("Redis local credential store error: {}", e))
}

fn get_local_credential_secret_redis(
    local_user_id: &str,
) -> std::result::Result<Option<LocalCredentialSecret>, String> {
    let mut conn = redis_connection()?;
    let json: Option<String> = redis::cmd("GET")
        .arg(redis_local_credential_key(local_user_id))
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;
    json.map(|json| local_credential_from_json(&json))
        .transpose()
}

fn store_local_password_reset_token_redis(
    token: &LocalPasswordResetToken,
) -> std::result::Result<(), String> {
    validate_local_password_reset_token_for_storage(token)?;
    if get_local_identity_by_id_redis(&token.local_user_id)?.is_none() {
        return Err(
            "[auth] password reset token must reference an existing local identity".to_string(),
        );
    }
    let mut conn = redis_connection()?;
    let now = chrono::Utc::now().timestamp();
    let ttl = token.expires_at.saturating_sub(now);
    if ttl <= 0 {
        return Err("[auth] password reset token expires before it can be stored".to_string());
    }
    let token_key = redis_local_password_reset_key(&token.selector);
    let user_set_key = redis_local_password_reset_user_set_key(&token.local_user_id);
    redis::pipe()
        .atomic()
        .cmd("SETEX")
        .arg(&token_key)
        .arg(ttl)
        .arg(local_password_reset_token_to_json(token))
        .ignore()
        .cmd("SADD")
        .arg(&user_set_key)
        .arg(&token_key)
        .ignore()
        .query::<()>(&mut conn)
        .map_err(|e| format!("Redis password reset token store error: {}", e))
}

fn consume_local_password_reset_token_and_store_credential_redis<F>(
    selector: &str,
    token_hash: &str,
    now: i64,
    credential_builder: F,
) -> std::result::Result<Option<(LocalIdentity, LocalCredentialSecret)>, String>
where
    F: FnOnce(&str) -> std::result::Result<LocalCredentialSecret, String>,
{
    let mut conn = redis_connection()?;
    let token_key = redis_local_password_reset_key(selector);
    let token_json: Option<String> = redis::cmd("GET")
        .arg(&token_key)
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;
    let Some(token_json) = token_json else {
        return Ok(None);
    };
    let reset_token = local_password_reset_token_from_json(&token_json)?;
    if reset_token.expires_at <= now {
        redis::pipe()
            .atomic()
            .cmd("DEL")
            .arg(&token_key)
            .ignore()
            .cmd("SREM")
            .arg(redis_local_password_reset_user_set_key(
                &reset_token.local_user_id,
            ))
            .arg(&token_key)
            .ignore()
            .query::<()>(&mut conn)
            .map_err(|e| format!("Redis expired password reset cleanup error: {}", e))?;
        return Ok(None);
    }
    if !constant_time_compare(&reset_token.token_hash, token_hash) {
        return Ok(None);
    }
    let identity_json: Option<String> = redis::cmd("GET")
        .arg(redis_local_identity_key(&reset_token.local_user_id))
        .query(&mut conn)
        .map_err(|e| format!("Redis GET error: {}", e))?;
    let Some(identity_json) = identity_json else {
        return Ok(None);
    };
    let identity = local_identity_from_json(&identity_json)?;
    if matches!(
        identity.state,
        LocalAccountState::Disabled | LocalAccountState::Locked
    ) {
        return Ok(None);
    }
    let credential = credential_builder(&reset_token.local_user_id)?;
    validate_local_credential_secret_for_storage(&credential)?;
    let identity = normalize_local_identity_for_storage(LocalIdentity {
        updated_at: now,
        state: LocalAccountState::Active,
        ..identity
    })?;

    let identity_key = redis_local_identity_key(&identity.id);
    let lookup_key =
        redis_local_identity_lookup_key(&identity.identifier_kind, &identity.identifier_normalized);
    let credential_key = redis_local_credential_key(&credential.local_user_id);
    let reset_set_key = redis_local_password_reset_user_set_key(&identity.id);
    let consumed: i64 = redis::Script::new(
        r#"
        local token_json = redis.call('GET', KEYS[1])
        if not token_json then
            return 0
        end

        local token = cjson.decode(token_json)
        if tonumber(token.expires_at) <= tonumber(ARGV[1]) then
            redis.call('DEL', KEYS[1])
            redis.call('SREM', KEYS[4], KEYS[1])
            return 0
        end

        if token.token_hash ~= ARGV[2] or token.local_user_id ~= ARGV[3] then
            return 0
        end

        local existing_id = redis.call('GET', KEYS[5])
        if existing_id and existing_id ~= ARGV[3] then
            return -1
        end

        redis.call('SET', KEYS[2], ARGV[4])
        redis.call('SET', KEYS[5], ARGV[3])
        redis.call('SET', KEYS[3], ARGV[5])

        local reset_keys = redis.call('SMEMBERS', KEYS[4])
        for _, key in ipairs(reset_keys) do
            redis.call('DEL', key)
        end
        redis.call('DEL', KEYS[4])
        return 1
        "#,
    )
    .key(&token_key)
    .key(&identity_key)
    .key(&credential_key)
    .key(&reset_set_key)
    .key(&lookup_key)
    .arg(now)
    .arg(token_hash)
    .arg(&identity.id)
    .arg(local_identity_to_json(&identity))
    .arg(local_credential_to_json(&credential))
    .invoke(&mut conn)
    .map_err(|e| format!("Redis password reset consume error: {}", e))?;

    if consumed == -1 {
        return Err(format!(
            "[auth] local identity identifier already exists for {}",
            identity.identifier_kind
        ));
    }
    if consumed != 1 {
        return Ok(None);
    }
    Ok(Some((identity, credential)))
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
    fn test_normalize_local_identifier_phone_username_and_custom() {
        assert_eq!(
            normalize_local_identifier("phone", " +1 (970) 444-8177 ").unwrap(),
            "+19704448177"
        );
        assert!(normalize_local_identifier("phone", "555").is_err());
        assert_eq!(
            normalize_local_identifier("username", "  Alice.Admin-1  ").unwrap(),
            "alice.admin-1"
        );
        assert!(normalize_local_identifier("username", "-alice").is_err());
        assert_eq!(
            normalize_local_identifier("custom", " External:ABC-123 ").unwrap(),
            "External:ABC-123"
        );
        assert!(normalize_local_identifier("custom", "\n").is_err());
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
