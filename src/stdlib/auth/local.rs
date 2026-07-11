use crate::interpreter::Value;
use argon2::password_hash::Error as PasswordHashError;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

use super::storage::{
    consume_local_one_time_token_record,
    consume_local_password_reset_token_and_store_credential_record,
    delete_all_session_records_for_user, get_local_credential_secret_record,
    get_local_identity_by_identifier_record, normalize_local_identifier,
    store_local_identity_and_credential_record,
    store_local_identity_and_credential_revoke_password_resets_record,
    store_local_one_time_token_record, update_local_identity_by_identifier_record,
    LocalAccountState, LocalCredentialSecret, LocalIdentity, LocalOneTimeToken,
    LocalOneTimeTokenPurpose,
};

const INVALID_LOCAL_CREDENTIALS: &str = "Invalid local credentials";
const INVALID_LOCAL_TOTP_CODE: &str = "Invalid local TOTP code";
pub(in crate::stdlib::auth) const INVALID_PASSWORD_RESET_TOKEN: &str =
    "Invalid password reset token";
pub(in crate::stdlib::auth) const EMPTY_LOCAL_PASSWORD: &str =
    "[auth] local password must not be empty";
pub(in crate::stdlib::auth) const PASSWORD_RESET_ISSUANCE_UNAVAILABLE: &str =
    "Password reset issuance unavailable";
pub(in crate::stdlib::auth) const PASSWORD_RESET_VERIFICATION_UNAVAILABLE: &str =
    "Password reset verification unavailable";
pub(in crate::stdlib::auth) const INVALID_MAGIC_LINK_TOKEN: &str = "Invalid magic link token";
pub(in crate::stdlib::auth) const MAGIC_LINK_ISSUANCE_UNAVAILABLE: &str =
    "Magic link issuance unavailable";
pub(in crate::stdlib::auth) const MAGIC_LINK_VERIFICATION_UNAVAILABLE: &str =
    "Magic link verification unavailable";
const DEFAULT_PASSWORD_RESET_TTL_SECONDS: i64 = 60 * 60;
const DEFAULT_MAGIC_LINK_TTL_SECONDS: i64 = 15 * 60;
const MAX_MAGIC_LINK_TTL_SECONDS: i64 = 60 * 60;
const INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH: &str =
    "[auth] local credential hash is invalid or unsupported";
const DUMMY_LOCAL_TOTP_SECRET: &str = "JBSWY3DPEHPK3PXP";
const DUMMY_LOCAL_BCRYPT_PASSWORD_HASH: &str =
    "$2b$12$.yG5RREnsakkWw6jeYfJNOxZnY6SGO22Ce8jBqKkvXnbV/2Hm3h.y";
const DUMMY_LOCAL_ARGON2_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$bnRudCBsb2NhbCBhdXRoIGR1bW15IHNhbHQ$kWVUWPBuKgDKDzEhE8gQJdr9ig91IJGYCQ+HrISyEIs";

pub(in crate::stdlib::auth) struct VerifiedLocalPassword {
    identity: LocalIdentity,
    credential: LocalCredentialSecret,
}

pub(in crate::stdlib::auth) fn local_user_record(
    identifier_kind: &str,
    identifier: &str,
) -> std::result::Result<LocalIdentity, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = normalize_local_identifier(&kind, identifier.trim())?;
    get_local_identity_by_identifier_record(&kind, &identifier_normalized)?
        .ok_or_else(|| "[auth] local user not found".to_string())
}

pub(in crate::stdlib::auth) fn update_local_user_metadata_record(
    identifier_kind: &str,
    identifier: &str,
    metadata: &HashMap<String, Value>,
    replace: bool,
) -> std::result::Result<LocalIdentity, String> {
    reject_reserved_local_metadata_namespaces(metadata)?;
    if replace && metadata.is_empty() {
        return Err(
            "[auth] update_local_user_metadata() replace=true with empty metadata would clear all app metadata; pass at least one app metadata key"
                .to_string(),
        );
    }

    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = normalize_local_identifier(&kind, identifier.trim())?;
    update_local_identity_by_identifier_record(&kind, &identifier_normalized, |identity| {
        let mut updated_metadata = local_metadata_to_value_map(&identity.metadata_json)?;
        if replace {
            updated_metadata.retain(|key, _| key == "auth" || key.starts_with("auth."));
        }
        for (key, value) in metadata {
            updated_metadata.insert(key.clone(), value.clone());
        }

        identity.metadata_json = local_metadata_to_json_string(&updated_metadata)?;
        identity.updated_at = chrono::Utc::now().timestamp();
        Ok(())
    })?
    .ok_or_else(|| "[auth] local user not found".to_string())
}

pub(in crate::stdlib::auth) fn begin_totp_enrollment_record(
    identifier_kind: &str,
    identifier: &str,
    issuer: Option<&str>,
    label: Option<&str>,
) -> std::result::Result<HashMap<String, Value>, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = normalize_local_identifier(&kind, identifier.trim())?;
    let secret = super::generate_totp_secret();
    let issuer = clean_totp_display_value(issuer, "NTNT")?;
    let now = chrono::Utc::now().timestamp();

    let updated_identity =
        update_local_identity_by_identifier_record(&kind, &identifier_normalized, |identity| {
            ensure_totp_manageable_identity(identity)?;
            let label = clean_totp_display_value(label, identity.identifier.as_str())?;
            let mut metadata = local_metadata_to_value_map(&identity.metadata_json)?;
            let mut totp = auth_totp_metadata(&metadata)?;
            let enabled = bool_field(&totp, "enabled").unwrap_or(false);
            totp.insert("enabled".to_string(), Value::Bool(enabled));
            totp.insert("pending".to_string(), Value::Bool(true));
            totp.insert("pending_secret".to_string(), Value::String(secret.clone()));
            totp.insert("issuer".to_string(), Value::String(issuer.clone()));
            totp.insert("label".to_string(), Value::String(label));
            totp.insert("updated_at".to_string(), Value::Int(now));
            if !totp.contains_key("created_at") {
                totp.insert("created_at".to_string(), Value::Int(now));
            }
            set_auth_totp_metadata(&mut metadata, Some(totp))?;
            identity.metadata_json = local_metadata_to_json_string(&metadata)?;
            identity.updated_at = now;
            Ok(())
        })?
        .ok_or_else(|| "[auth] local user not found".to_string())?;

    let mut status = totp_status_from_identity(&updated_identity)?;
    let label =
        string_field(&status, "label").unwrap_or_else(|| updated_identity.identifier.clone());
    let uri = super::get_totp_uri(&secret, &label, &issuer)?;
    status.insert("uri".to_string(), Value::String(uri));
    Ok(status)
}

pub(in crate::stdlib::auth) fn confirm_totp_enrollment_record(
    identifier_kind: &str,
    identifier: &str,
    code: &str,
) -> std::result::Result<HashMap<String, Value>, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = normalize_local_identifier(&kind, identifier.trim())?;
    let now = chrono::Utc::now().timestamp();
    let updated_identity =
        update_local_identity_by_identifier_record(&kind, &identifier_normalized, |identity| {
            ensure_totp_manageable_identity(identity)?;
            let mut metadata = local_metadata_to_value_map(&identity.metadata_json)?;
            let totp = auth_totp_metadata(&metadata)?;
            let pending_secret = string_field(&totp, "pending_secret")
                .ok_or_else(|| "[auth] no pending local TOTP enrollment".to_string())?;
            let issuer = string_field(&totp, "issuer").unwrap_or_else(|| "NTNT".to_string());
            let label = string_field(&totp, "label").unwrap_or_else(|| identity.identifier.clone());
            let required = bool_field(&totp, "required").unwrap_or(false);
            let created_at = int_field(&totp, "created_at").unwrap_or(now);
            if !super::verify_totp_code(&pending_secret, code, &identity.identifier, &issuer) {
                return Err("Invalid local TOTP code".to_string());
            }
            set_auth_totp_metadata(
                &mut metadata,
                Some(HashMap::from([
                    ("enabled".to_string(), Value::Bool(true)),
                    ("pending".to_string(), Value::Bool(false)),
                    ("required".to_string(), Value::Bool(required)),
                    ("secret".to_string(), Value::String(pending_secret)),
                    ("issuer".to_string(), Value::String(issuer)),
                    ("label".to_string(), Value::String(label)),
                    ("created_at".to_string(), Value::Int(created_at)),
                    ("confirmed_at".to_string(), Value::Int(now)),
                    ("updated_at".to_string(), Value::Int(now)),
                ])),
            )?;
            identity.metadata_json = local_metadata_to_json_string(&metadata)?;
            identity.updated_at = now;
            Ok(())
        })?
        .ok_or_else(|| "[auth] local user not found".to_string())?;

    totp_status_from_identity(&updated_identity)
}

pub(in crate::stdlib::auth) fn verify_local_totp_record(
    identifier_kind: &str,
    identifier: &str,
    code: &str,
) -> std::result::Result<HashMap<String, Value>, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = match normalize_local_identifier(&kind, identifier.trim()) {
        Ok(identifier_normalized) => identifier_normalized,
        Err(_) => return invalid_local_totp_result(code),
    };

    let identity = match get_local_identity_by_identifier_record(&kind, &identifier_normalized) {
        Ok(Some(identity)) => identity,
        Ok(None) => return invalid_local_totp_result(code),
        Err(err) => {
            verify_dummy_local_totp(code);
            return Err(err);
        }
    };

    if matches!(
        identity.state,
        LocalAccountState::Disabled | LocalAccountState::Locked
    ) {
        return invalid_local_totp_result(code);
    }

    let metadata = match local_metadata_to_value_map(&identity.metadata_json) {
        Ok(metadata) => metadata,
        Err(_) => return invalid_local_totp_result(code),
    };
    let totp = match auth_totp_metadata(&metadata) {
        Ok(totp) => totp,
        Err(_) => return invalid_local_totp_result(code),
    };
    let secret = match string_field(&totp, "secret") {
        Some(secret) if bool_field(&totp, "enabled").unwrap_or(false) => secret,
        _ => return invalid_local_totp_result(code),
    };
    let issuer = string_field(&totp, "issuer").unwrap_or_else(|| "NTNT".to_string());
    if !super::verify_totp_code(&secret, code, &identity.identifier, &issuer) {
        return Err(INVALID_LOCAL_TOTP_CODE.to_string());
    }
    let mut status = totp_status_from_identity(&identity)?;
    status.insert("verified".to_string(), Value::Bool(true));
    Ok(status)
}

pub(in crate::stdlib::auth) fn totp_status_record(
    identifier_kind: &str,
    identifier: &str,
) -> std::result::Result<HashMap<String, Value>, String> {
    let identity = local_user_record(identifier_kind, identifier)?;
    ensure_totp_manageable_identity(&identity)?;
    totp_status_from_identity(&identity)
}

pub(in crate::stdlib::auth) fn reset_totp_record(
    identifier_kind: &str,
    identifier: &str,
) -> std::result::Result<HashMap<String, Value>, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = normalize_local_identifier(&kind, identifier.trim())?;
    let now = chrono::Utc::now().timestamp();
    let updated_identity =
        update_local_identity_by_identifier_record(&kind, &identifier_normalized, |identity| {
            ensure_totp_manageable_identity(identity)?;
            let mut metadata = local_metadata_to_value_map(&identity.metadata_json)?;
            set_auth_totp_metadata(&mut metadata, None)?;
            identity.metadata_json = local_metadata_to_json_string(&metadata)?;
            identity.updated_at = now;
            Ok(())
        })?
        .ok_or_else(|| "[auth] local user not found".to_string())?;

    totp_status_from_identity(&updated_identity)
}

fn reject_reserved_local_metadata_namespaces(
    metadata: &HashMap<String, Value>,
) -> std::result::Result<(), String> {
    for key in metadata.keys() {
        if key == "auth" || key.starts_with("auth.") {
            return Err(
                "[auth] local user metadata keys under auth.* are reserved for std/auth helpers"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn clean_totp_display_value(
    value: Option<&str>,
    default: &str,
) -> std::result::Result<String, String> {
    let cleaned = value.unwrap_or(default).trim();
    if cleaned.is_empty() {
        return Err("[auth] TOTP issuer/label must not be empty".to_string());
    }
    Ok(cleaned.to_string())
}

fn verify_dummy_local_totp(code: &str) {
    let _ = super::verify_totp_code(DUMMY_LOCAL_TOTP_SECRET, code, "__dummy__", "NTNT");
}

fn invalid_local_totp_result<T>(code: &str) -> std::result::Result<T, String> {
    verify_dummy_local_totp(code);
    Err(INVALID_LOCAL_TOTP_CODE.to_string())
}

fn ensure_totp_manageable_identity(identity: &LocalIdentity) -> std::result::Result<(), String> {
    if matches!(
        identity.state,
        LocalAccountState::Disabled | LocalAccountState::Locked
    ) {
        return Err("[auth] local user cannot manage TOTP in current state".to_string());
    }
    Ok(())
}

fn auth_totp_metadata(
    metadata: &HashMap<String, Value>,
) -> std::result::Result<HashMap<String, Value>, String> {
    match metadata.get("auth") {
        Some(Value::Map(auth)) => match auth.get("totp") {
            Some(Value::Map(totp)) => Ok(totp.clone()),
            Some(_) => Err("[auth] local TOTP metadata must be a map".to_string()),
            None => Ok(HashMap::new()),
        },
        Some(_) => Err("[auth] local auth metadata must be a map".to_string()),
        None => Ok(HashMap::new()),
    }
}

fn set_auth_totp_metadata(
    metadata: &mut HashMap<String, Value>,
    totp: Option<HashMap<String, Value>>,
) -> std::result::Result<(), String> {
    let mut auth = match metadata.remove("auth") {
        Some(Value::Map(auth)) => auth,
        Some(_) => return Err("[auth] local auth metadata must be a map".to_string()),
        None => HashMap::new(),
    };

    match totp {
        Some(totp) => {
            auth.insert("totp".to_string(), Value::Map(totp));
            metadata.insert("auth".to_string(), Value::Map(auth));
        }
        None => {
            auth.remove("totp");
            if !auth.is_empty() {
                metadata.insert("auth".to_string(), Value::Map(auth));
            }
        }
    }

    Ok(())
}

fn bool_field(map: &HashMap<String, Value>, key: &str) -> Option<bool> {
    match map.get(key) {
        Some(Value::Bool(value)) => Some(*value),
        _ => None,
    }
}

fn int_field(map: &HashMap<String, Value>, key: &str) -> Option<i64> {
    match map.get(key) {
        Some(Value::Int(value)) => Some(*value),
        _ => None,
    }
}

fn string_field(map: &HashMap<String, Value>, key: &str) -> Option<String> {
    match map.get(key) {
        Some(Value::String(value)) => Some(value.clone()),
        _ => None,
    }
}

fn totp_status_from_identity(
    identity: &LocalIdentity,
) -> std::result::Result<HashMap<String, Value>, String> {
    let metadata = local_metadata_to_value_map(&identity.metadata_json)?;
    let totp = auth_totp_metadata(&metadata)?;
    let enabled = bool_field(&totp, "enabled").unwrap_or(false);
    let pending = bool_field(&totp, "pending").unwrap_or(false);
    let required = bool_field(&totp, "required").unwrap_or(false);

    let mut status = HashMap::from([
        ("subject_id".to_string(), Value::String(identity.id.clone())),
        ("id".to_string(), Value::String(identity.id.clone())),
        (
            "local_user_id".to_string(),
            Value::String(identity.id.clone()),
        ),
        (
            "identifier_kind".to_string(),
            Value::String(identity.identifier_kind.clone()),
        ),
        (
            "identifier".to_string(),
            Value::String(identity.identifier.clone()),
        ),
        (
            "identifier_normalized".to_string(),
            Value::String(identity.identifier_normalized.clone()),
        ),
        (
            "state".to_string(),
            Value::String(identity.state.as_str().to_string()),
        ),
        ("enabled".to_string(), Value::Bool(enabled)),
        ("pending".to_string(), Value::Bool(pending)),
        ("required".to_string(), Value::Bool(required)),
    ]);

    if identity.identifier_kind == "email" {
        status.insert(
            "email".to_string(),
            Value::String(identity.identifier.clone()),
        );
    }
    if let Some(issuer) = string_field(&totp, "issuer") {
        status.insert("issuer".to_string(), Value::String(issuer));
    }
    if let Some(label) = string_field(&totp, "label") {
        status.insert("label".to_string(), Value::String(label));
    }
    for key in ["created_at", "updated_at", "confirmed_at"] {
        if let Some(value) = int_field(&totp, key) {
            status.insert(key.to_string(), Value::Int(value));
        }
    }

    Ok(status)
}

fn local_metadata_to_value_map(
    metadata_json: &str,
) -> std::result::Result<HashMap<String, Value>, String> {
    let json = serde_json::from_str::<serde_json::Value>(metadata_json)
        .map_err(|_| "[auth] local user metadata_json must be a JSON object".to_string())?;
    match crate::stdlib::json::json_to_intent_value(&json) {
        Value::Map(map) => Ok(map),
        _ => Err("[auth] local user metadata_json must be a JSON object".to_string()),
    }
}

fn local_metadata_to_json_string(
    metadata: &HashMap<String, Value>,
) -> std::result::Result<String, String> {
    let json = crate::stdlib::json::intent_value_to_json(&Value::Map(metadata.clone()));
    match json {
        serde_json::Value::Object(_) => serde_json::to_string(&json)
            .map_err(|_| "[auth] failed to encode local user metadata".to_string()),
        _ => Err("[auth] local user metadata must be a map".to_string()),
    }
}

fn safe_local_metadata_map(
    identity: &LocalIdentity,
) -> std::result::Result<HashMap<String, Value>, String> {
    let mut metadata = local_metadata_to_value_map(&identity.metadata_json)?;
    metadata.retain(|key, _| key != "auth" && !key.starts_with("auth."));
    Ok(metadata)
}

pub(in crate::stdlib::auth) fn local_identity_to_safe_value(
    identity: &LocalIdentity,
) -> std::result::Result<Value, String> {
    let mut map = HashMap::from([
        ("subject_id".to_string(), Value::String(identity.id.clone())),
        ("id".to_string(), Value::String(identity.id.clone())),
        (
            "local_user_id".to_string(),
            Value::String(identity.id.clone()),
        ),
        ("provider".to_string(), Value::String("local".to_string())),
        (
            "identifier_kind".to_string(),
            Value::String(identity.identifier_kind.clone()),
        ),
        (
            "identifier".to_string(),
            Value::String(identity.identifier.clone()),
        ),
        (
            "identifier_normalized".to_string(),
            Value::String(identity.identifier_normalized.clone()),
        ),
        (
            "state".to_string(),
            Value::String(identity.state.as_str().to_string()),
        ),
        (
            "metadata".to_string(),
            Value::Map(safe_local_metadata_map(identity)?),
        ),
    ]);

    if identity.identifier_kind == "email" {
        map.insert(
            "email".to_string(),
            Value::String(identity.identifier.clone()),
        );
    }

    Ok(Value::Map(map))
}

pub(in crate::stdlib::auth) fn bootstrap_local_user_record(
    identifier_kind: &str,
    identifier: &str,
    password: &str,
) -> std::result::Result<VerifiedLocalPassword, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier = identifier.trim();
    let identifier_normalized = normalize_local_identifier(&kind, identifier)?;

    if password.trim().is_empty() {
        return Err("[auth] local bootstrap password must not be empty".to_string());
    }

    if get_local_identity_by_identifier_record(&kind, &identifier_normalized)?.is_some() {
        return Err(format!("[auth] local identity already exists for {}", kind));
    }

    let now = chrono::Utc::now().timestamp();
    let identity = LocalIdentity {
        id: format!("local:{}", uuid::Uuid::new_v4()),
        identifier_kind: kind,
        identifier: identifier.to_string(),
        identifier_normalized,
        created_at: now,
        updated_at: now,
        state: LocalAccountState::Bootstrap,
        metadata_json: "{}".to_string(),
    };
    let credential = LocalCredentialSecret {
        local_user_id: identity.id.clone(),
        password_hash: bcrypt::hash(password, bcrypt::DEFAULT_COST)
            .map_err(|_| "[auth] failed to hash local bootstrap password".to_string())?,
        password_hash_algorithm: CredentialPasswordAlgorithm::Bcrypt
            .storage_name()
            .to_string(),
        password_hash_params_json: "{}".to_string(),
        password_changed_at: now,
        must_change_password: true,
    };

    store_local_identity_and_credential_record(&identity, &credential)?;

    Ok(VerifiedLocalPassword {
        identity,
        credential,
    })
}

pub(in crate::stdlib::auth) fn set_local_password_record(
    identifier_kind: &str,
    identifier: &str,
    current_password: &str,
    new_password: &str,
) -> std::result::Result<VerifiedLocalPassword, String> {
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier = identifier.trim();

    if new_password.trim().is_empty() {
        return Err(EMPTY_LOCAL_PASSWORD.to_string());
    }

    let verified = verify_local_password_record(&kind, identifier, current_password)?;
    if current_password == new_password {
        return Err("[auth] local password must differ from current password".to_string());
    }

    let now = chrono::Utc::now().timestamp();
    let identity = LocalIdentity {
        updated_at: now,
        state: LocalAccountState::Active,
        ..verified.identity
    };
    let credential = LocalCredentialSecret {
        local_user_id: identity.id.clone(),
        password_hash: bcrypt::hash(new_password, bcrypt::DEFAULT_COST)
            .map_err(|_| "[auth] failed to hash local password".to_string())?,
        password_hash_algorithm: CredentialPasswordAlgorithm::Bcrypt
            .storage_name()
            .to_string(),
        password_hash_params_json: "{}".to_string(),
        password_changed_at: now,
        must_change_password: false,
    };

    store_local_identity_and_credential_revoke_password_resets_record(&identity, &credential)?;

    Ok(VerifiedLocalPassword {
        identity,
        credential,
    })
}

pub(in crate::stdlib::auth) fn issue_password_reset_record(
    identifier_kind: &str,
    identifier: &str,
    ttl_seconds: Option<i64>,
) -> std::result::Result<HashMap<String, Value>, String> {
    let ttl_seconds = ttl_seconds.unwrap_or(DEFAULT_PASSWORD_RESET_TTL_SECONDS);
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = match normalize_local_identifier(&kind, identifier.trim()) {
        Ok(identifier_normalized) => identifier_normalized,
        Err(_) => return Ok(local_token_accepted_response()),
    };

    let now = chrono::Utc::now().timestamp();
    let expires_at = now.saturating_add(ttl_seconds.max(0));
    if expires_at <= now {
        return Ok(local_token_accepted_response());
    }

    let selector = random_urlsafe_token(16);
    let verifier = random_urlsafe_token(32);
    let token = format!("{selector}.{verifier}");

    if let Some(identity) = get_local_identity_by_identifier_record(&kind, &identifier_normalized)?
    {
        if !matches!(
            identity.state,
            LocalAccountState::Disabled | LocalAccountState::Locked
        ) {
            store_local_one_time_token_record(&LocalOneTimeToken {
                purpose: LocalOneTimeTokenPurpose::PasswordReset,
                selector: selector.clone(),
                local_user_id: identity.id.clone(),
                token_hash: hash_local_token_verifier(&verifier),
                created_at: now,
                expires_at,
            })?;
        }
    }

    Ok(local_token_response(selector, token, now, expires_at))
}

pub(in crate::stdlib::auth) fn consume_password_reset_record(
    token: &str,
    new_password: &str,
    revoke_sessions: bool,
) -> std::result::Result<(VerifiedLocalPassword, u64), String> {
    if new_password.trim().is_empty() {
        return Err(EMPTY_LOCAL_PASSWORD.to_string());
    }

    let Some((selector, verifier)) = token.split_once('.') else {
        return Err(INVALID_PASSWORD_RESET_TOKEN.to_string());
    };
    if selector.trim().is_empty() || verifier.trim().is_empty() || token.split('.').count() != 2 {
        return Err(INVALID_PASSWORD_RESET_TOKEN.to_string());
    }

    let now = chrono::Utc::now().timestamp();
    let submitted_hash = hash_local_token_verifier(verifier);
    let Some((identity, credential)) =
        consume_local_password_reset_token_and_store_credential_record(
            selector,
            &submitted_hash,
            now,
            |local_user_id| {
                Ok(LocalCredentialSecret {
                    local_user_id: local_user_id.to_string(),
                    password_hash: bcrypt::hash(new_password, bcrypt::DEFAULT_COST)
                        .map_err(|_| "[auth] failed to hash local password".to_string())?,
                    password_hash_algorithm: CredentialPasswordAlgorithm::Bcrypt
                        .storage_name()
                        .to_string(),
                    password_hash_params_json: "{}".to_string(),
                    password_changed_at: now,
                    must_change_password: false,
                })
            },
        )?
    else {
        return Err(INVALID_PASSWORD_RESET_TOKEN.to_string());
    };
    let revoked_sessions = if revoke_sessions {
        delete_all_session_records_for_user(&identity.id, None)?
    } else {
        0
    };
    Ok((
        VerifiedLocalPassword {
            identity,
            credential,
        },
        revoked_sessions,
    ))
}

pub(in crate::stdlib::auth) fn issue_magic_link_record(
    identifier_kind: &str,
    identifier: &str,
    ttl_seconds: Option<i64>,
) -> std::result::Result<HashMap<String, Value>, String> {
    let ttl_seconds = ttl_seconds
        .unwrap_or(DEFAULT_MAGIC_LINK_TTL_SECONDS)
        .min(MAX_MAGIC_LINK_TTL_SECONDS);
    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = match normalize_local_identifier(&kind, identifier.trim()) {
        Ok(identifier_normalized) => identifier_normalized,
        Err(_) => return Ok(local_token_accepted_response()),
    };

    let now = chrono::Utc::now().timestamp();
    let expires_at = now.saturating_add(ttl_seconds.max(0));
    if expires_at <= now {
        return Ok(local_token_accepted_response());
    }

    let selector = random_urlsafe_token(16);
    let verifier = random_urlsafe_token(32);
    let token = format!("{selector}.{verifier}");
    if let Some(identity) = get_local_identity_by_identifier_record(&kind, &identifier_normalized)?
    {
        if identity.state == LocalAccountState::Active {
            store_local_one_time_token_record(&LocalOneTimeToken {
                purpose: LocalOneTimeTokenPurpose::MagicLink,
                selector: selector.clone(),
                local_user_id: identity.id,
                token_hash: hash_local_token_verifier(&verifier),
                created_at: now,
                expires_at,
            })?;
        }
    }

    Ok(local_token_response(selector, token, now, expires_at))
}

pub(in crate::stdlib::auth) fn consume_magic_link_record(
    token: &str,
) -> std::result::Result<LocalIdentity, String> {
    let Some((selector, verifier)) = token.split_once('.') else {
        return Err(INVALID_MAGIC_LINK_TOKEN.to_string());
    };
    if !is_urlsafe_token_component(selector, 22)
        || !is_urlsafe_token_component(verifier, 43)
        || token.split('.').count() != 2
    {
        return Err(INVALID_MAGIC_LINK_TOKEN.to_string());
    }

    let now = chrono::Utc::now().timestamp();
    let submitted_hash = hash_local_token_verifier(verifier);
    consume_local_one_time_token_record(
        LocalOneTimeTokenPurpose::MagicLink,
        selector,
        &submitted_hash,
        now,
    )?
    .ok_or_else(|| INVALID_MAGIC_LINK_TOKEN.to_string())
}

fn local_token_accepted_response() -> HashMap<String, Value> {
    HashMap::from([("status".to_string(), Value::String("accepted".to_string()))])
}

fn local_token_response(
    selector: String,
    token: String,
    created_at: i64,
    expires_at: i64,
) -> HashMap<String, Value> {
    HashMap::from([
        ("status".to_string(), Value::String("accepted".to_string())),
        ("selector".to_string(), Value::String(selector)),
        ("token".to_string(), Value::String(token)),
        ("created_at".to_string(), Value::Int(created_at)),
        ("expires_at".to_string(), Value::Int(expires_at)),
    ])
}

fn is_urlsafe_token_component(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn random_urlsafe_token(byte_count: usize) -> String {
    let mut bytes = vec![0_u8; byte_count];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn hash_local_token_verifier(verifier: &str) -> String {
    hex::encode(Sha256::digest(verifier.as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialPasswordAlgorithm {
    Bcrypt,
    Argon2,
}

impl CredentialPasswordAlgorithm {
    fn parse(algorithm: &str) -> std::result::Result<Self, String> {
        match algorithm.trim().to_ascii_lowercase().as_str() {
            "bcrypt" | "bcrypt2" => Ok(Self::Bcrypt),
            "argon2" | "argon2id" => Ok(Self::Argon2),
            _ => Err(INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH.to_string()),
        }
    }

    fn storage_name(self) -> &'static str {
        match self {
            Self::Bcrypt => "bcrypt",
            Self::Argon2 => "argon2id",
        }
    }
}

pub(in crate::stdlib::auth) fn verify_local_password_record(
    identifier_kind: &str,
    identifier: &str,
    password: &str,
) -> std::result::Result<VerifiedLocalPassword, String> {
    let identifier_normalized = match normalize_local_identifier(identifier_kind, identifier) {
        Ok(identifier_normalized) => identifier_normalized,
        Err(_) => {
            verify_all_dummy_local_passwords(password)?;
            return Err(INVALID_LOCAL_CREDENTIALS.to_string());
        }
    };
    let identity =
        match get_local_identity_by_identifier_record(identifier_kind, &identifier_normalized) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                verify_all_dummy_local_passwords(password)?;
                return Err(INVALID_LOCAL_CREDENTIALS.to_string());
            }
            Err(err) => {
                verify_all_dummy_local_passwords(password)?;
                return Err(err);
            }
        };
    let credential = match get_local_credential_secret_record(&identity.id) {
        Ok(Some(credential)) => credential,
        Ok(None) => {
            verify_all_dummy_local_passwords(password)?;
            return Err(INVALID_LOCAL_CREDENTIALS.to_string());
        }
        Err(err) => {
            verify_all_dummy_local_passwords(password)?;
            return Err(err);
        }
    };

    let password_matches = match verify_credential_password(&credential, password) {
        Ok((password_matches, algorithm)) => {
            verify_complementary_dummy_local_password(password, algorithm)?;
            password_matches
        }
        Err(err) => {
            let _ = verify_all_dummy_local_passwords(password);
            return Err(err);
        }
    };
    let state_allows_password_login = !matches!(
        identity.state,
        LocalAccountState::Disabled | LocalAccountState::Locked
    );
    if !password_matches || !state_allows_password_login {
        return Err(INVALID_LOCAL_CREDENTIALS.to_string());
    }

    Ok(VerifiedLocalPassword {
        identity,
        credential,
    })
}

fn verify_all_dummy_local_passwords(password: &str) -> std::result::Result<(), String> {
    verify_dummy_local_password(password, CredentialPasswordAlgorithm::Bcrypt)?;
    verify_dummy_local_password(password, CredentialPasswordAlgorithm::Argon2)?;
    Ok(())
}

fn verify_complementary_dummy_local_password(
    password: &str,
    algorithm: CredentialPasswordAlgorithm,
) -> std::result::Result<(), String> {
    match algorithm {
        CredentialPasswordAlgorithm::Bcrypt => {
            verify_dummy_local_password(password, CredentialPasswordAlgorithm::Argon2)
        }
        CredentialPasswordAlgorithm::Argon2 => {
            verify_dummy_local_password(password, CredentialPasswordAlgorithm::Bcrypt)
        }
    }
}

fn verify_dummy_local_password(
    password: &str,
    algorithm: CredentialPasswordAlgorithm,
) -> std::result::Result<(), String> {
    let password_hash = match algorithm {
        CredentialPasswordAlgorithm::Bcrypt => DUMMY_LOCAL_BCRYPT_PASSWORD_HASH,
        CredentialPasswordAlgorithm::Argon2 => DUMMY_LOCAL_ARGON2_PASSWORD_HASH,
    };
    let credential = LocalCredentialSecret {
        local_user_id: "__dummy__".to_string(),
        password_hash: password_hash.to_string(),
        password_hash_algorithm: algorithm.storage_name().to_string(),
        password_hash_params_json: "{}".to_string(),
        password_changed_at: 0,
        must_change_password: false,
    };
    let _ = verify_credential_password(&credential, password)?;
    Ok(())
}

fn verify_credential_password(
    credential: &LocalCredentialSecret,
    password: &str,
) -> std::result::Result<(bool, CredentialPasswordAlgorithm), String> {
    let algorithm = CredentialPasswordAlgorithm::parse(&credential.password_hash_algorithm)?;
    let password_matches = match algorithm {
        CredentialPasswordAlgorithm::Bcrypt => bcrypt::verify(password, &credential.password_hash)
            .map_err(|_| INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH.to_string())?,
        CredentialPasswordAlgorithm::Argon2 => {
            verify_argon2_password(password, &credential.password_hash)?
        }
    };
    Ok((password_matches, algorithm))
}

fn verify_argon2_password(
    password: &str,
    password_hash: &str,
) -> std::result::Result<bool, String> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|_| INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH.to_string())?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed_hash) {
        Ok(()) => Ok(true),
        Err(PasswordHashError::Password) => Ok(false),
        Err(_) => Err(INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH.to_string()),
    }
}

pub(in crate::stdlib::auth) fn verified_local_password_to_value(
    verified: VerifiedLocalPassword,
) -> Value {
    let mut map = HashMap::from([
        (
            "subject_id".to_string(),
            Value::String(verified.identity.id.clone()),
        ),
        (
            "id".to_string(),
            Value::String(verified.identity.id.clone()),
        ),
        (
            "local_user_id".to_string(),
            Value::String(verified.identity.id.clone()),
        ),
        ("provider".to_string(), Value::String("local".to_string())),
        (
            "identifier_kind".to_string(),
            Value::String(verified.identity.identifier_kind.clone()),
        ),
        (
            "identifier".to_string(),
            Value::String(verified.identity.identifier.clone()),
        ),
        (
            "identifier_normalized".to_string(),
            Value::String(verified.identity.identifier_normalized.clone()),
        ),
        (
            "state".to_string(),
            Value::String(verified.identity.state.as_str().to_string()),
        ),
        (
            "must_change_password".to_string(),
            Value::Bool(
                verified.credential.must_change_password
                    || matches!(
                        verified.identity.state,
                        LocalAccountState::Bootstrap
                            | LocalAccountState::PendingSetup
                            | LocalAccountState::PasswordChangeRequired
                    ),
            ),
        ),
        (
            "password_changed_at".to_string(),
            Value::Int(verified.credential.password_changed_at),
        ),
    ]);

    if verified.identity.identifier_kind == "email" {
        map.insert(
            "email".to_string(),
            Value::String(verified.identity.identifier.clone()),
        );
    }

    Value::Map(map)
}
