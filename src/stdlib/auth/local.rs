use crate::interpreter::Value;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use std::collections::HashMap;

use super::storage::{
    get_local_credential_secret_record, get_local_identity_by_identifier_record,
    normalize_local_identifier, LocalAccountState, LocalCredentialSecret, LocalIdentity,
};

const INVALID_LOCAL_CREDENTIALS: &str = "Invalid local credentials";

pub(in crate::stdlib::auth) struct VerifiedLocalPassword {
    identity: LocalIdentity,
    credential: LocalCredentialSecret,
}

pub(in crate::stdlib::auth) fn verify_local_password_record(
    identifier_kind: &str,
    identifier: &str,
    password: &str,
) -> std::result::Result<VerifiedLocalPassword, String> {
    let identifier_normalized = normalize_local_identifier(identifier_kind, identifier)
        .map_err(|_| INVALID_LOCAL_CREDENTIALS.to_string())?;
    let identity =
        get_local_identity_by_identifier_record(identifier_kind, &identifier_normalized)?
            .ok_or_else(|| INVALID_LOCAL_CREDENTIALS.to_string())?;
    let credential = get_local_credential_secret_record(&identity.id)?
        .ok_or_else(|| INVALID_LOCAL_CREDENTIALS.to_string())?;

    if !verify_credential_password(&credential, password)? {
        return Err(INVALID_LOCAL_CREDENTIALS.to_string());
    }

    match identity.state {
        LocalAccountState::Disabled => Err("Local account is disabled".to_string()),
        LocalAccountState::Locked => Err("Local account is locked".to_string()),
        _ => Ok(VerifiedLocalPassword {
            identity,
            credential,
        }),
    }
}

fn verify_credential_password(
    credential: &LocalCredentialSecret,
    password: &str,
) -> std::result::Result<bool, String> {
    let algorithm = credential
        .password_hash_algorithm
        .trim()
        .to_ascii_lowercase();
    match algorithm.as_str() {
        "bcrypt" | "bcrypt2" => bcrypt::verify(password, &credential.password_hash)
            .map_err(|e| format!("[auth] local credential hash verify failed: {}", e)),
        "argon2" | "argon2id" => verify_argon2_password(password, &credential.password_hash),
        other => Err(format!(
            "[auth] unsupported local credential hash algorithm \"{}\"",
            other
        )),
    }
}

fn verify_argon2_password(
    password: &str,
    password_hash: &str,
) -> std::result::Result<bool, String> {
    let parsed_hash = PasswordHash::new(password_hash)
        .map_err(|e| format!("[auth] local argon2 password hash is invalid: {}", e))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
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
                    || verified.identity.state == LocalAccountState::PasswordChangeRequired,
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
