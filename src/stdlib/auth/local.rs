use crate::interpreter::Value;
use argon2::password_hash::Error as PasswordHashError;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use std::collections::HashMap;
use std::sync::LazyLock;

use super::storage::{
    get_local_credential_secret_record, get_local_identity_by_identifier_record,
    normalize_local_identifier, LocalAccountState, LocalCredentialSecret, LocalIdentity,
};

const INVALID_LOCAL_CREDENTIALS: &str = "Invalid local credentials";
const INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH: &str =
    "[auth] local credential hash is invalid or unsupported";

static DUMMY_LOCAL_PASSWORD_HASH: LazyLock<String> = LazyLock::new(|| {
    bcrypt::hash("ntnt local auth dummy password", bcrypt::DEFAULT_COST)
        .expect("dummy local auth password hash must be constructible")
});

pub(in crate::stdlib::auth) struct VerifiedLocalPassword {
    identity: LocalIdentity,
    credential: LocalCredentialSecret,
}

pub(in crate::stdlib::auth) fn verify_local_password_record(
    identifier_kind: &str,
    identifier: &str,
    password: &str,
) -> std::result::Result<VerifiedLocalPassword, String> {
    let identifier_normalized = match normalize_local_identifier(identifier_kind, identifier) {
        Ok(identifier_normalized) => identifier_normalized,
        Err(_) => {
            verify_dummy_local_password(password)?;
            return Err(INVALID_LOCAL_CREDENTIALS.to_string());
        }
    };
    let Some(identity) =
        get_local_identity_by_identifier_record(identifier_kind, &identifier_normalized)?
    else {
        verify_dummy_local_password(password)?;
        return Err(INVALID_LOCAL_CREDENTIALS.to_string());
    };
    let Some(credential) = get_local_credential_secret_record(&identity.id)? else {
        verify_dummy_local_password(password)?;
        return Err(INVALID_LOCAL_CREDENTIALS.to_string());
    };

    let password_matches = verify_credential_password(&credential, password)?;
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

fn verify_dummy_local_password(password: &str) -> std::result::Result<(), String> {
    let credential = LocalCredentialSecret {
        local_user_id: "__dummy__".to_string(),
        password_hash: DUMMY_LOCAL_PASSWORD_HASH.clone(),
        password_hash_algorithm: "bcrypt".to_string(),
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
) -> std::result::Result<bool, String> {
    let algorithm = credential
        .password_hash_algorithm
        .trim()
        .to_ascii_lowercase();
    match algorithm.as_str() {
        "bcrypt" | "bcrypt2" => bcrypt::verify(password, &credential.password_hash)
            .map_err(|_| INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH.to_string()),
        "argon2" | "argon2id" => verify_argon2_password(password, &credential.password_hash),
        _ => Err(INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH.to_string()),
    }
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
