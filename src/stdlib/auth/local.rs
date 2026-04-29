use crate::interpreter::Value;
use argon2::password_hash::Error as PasswordHashError;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use std::collections::HashMap;

use super::storage::{
    get_local_credential_secret_record, get_local_identity_by_identifier_record,
    normalize_local_identifier, store_local_identity_and_credential_record,
    update_local_identity_by_identifier_record, LocalAccountState, LocalCredentialSecret,
    LocalIdentity,
};

const INVALID_LOCAL_CREDENTIALS: &str = "Invalid local credentials";
const INVALID_OR_UNSUPPORTED_LOCAL_CREDENTIAL_HASH: &str =
    "[auth] local credential hash is invalid or unsupported";
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
        return Err("[auth] local password must not be empty".to_string());
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

    store_local_identity_and_credential_record(&identity, &credential)?;

    Ok(VerifiedLocalPassword {
        identity,
        credential,
    })
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
