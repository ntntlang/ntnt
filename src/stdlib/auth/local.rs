use crate::interpreter::Value;
use argon2::password_hash::{Error as PasswordHashError, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use std::collections::HashMap;

use super::storage::{
    get_bootstrap_local_user_id_record, get_local_credential_secret_record,
    get_local_identity_by_id_record, get_local_identity_by_identifier_record,
    normalize_local_identifier, provision_local_user_record, store_bootstrap_local_user_id_record,
    LocalAccountState, LocalCredentialSecret, LocalIdentity,
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

pub(in crate::stdlib::auth) struct LocalUserCreation {
    identity: LocalIdentity,
    must_change_password: bool,
    created: bool,
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

pub(in crate::stdlib::auth) fn create_local_user_record(
    identifier_kind: &str,
    identifier: &str,
    password: &str,
    state: LocalAccountState,
    must_change_password: bool,
    metadata_json: String,
    id: Option<String>,
) -> std::result::Result<LocalUserCreation, String> {
    if password.is_empty() {
        return Err("[auth] local password must not be empty".to_string());
    }

    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = normalize_local_identifier(&kind, identifier)?;

    let local_user_id = match id {
        Some(id) => {
            let id = id.trim().to_string();
            if id.is_empty() {
                return Err("[auth] local identity id must not be empty".to_string());
            }
            id
        }
        None => uuid::Uuid::new_v4().to_string(),
    };

    let now = chrono::Utc::now().timestamp();
    let identity = LocalIdentity {
        id: local_user_id,
        identifier_kind: kind,
        identifier: identifier.to_string(),
        identifier_normalized,
        created_at: now,
        updated_at: now,
        state,
        metadata_json,
    };
    let credential =
        hash_local_password_for_identity(&identity.id, password, must_change_password, now)?;

    provision_local_user_record(&identity, &credential, false)?;

    Ok(LocalUserCreation {
        identity,
        must_change_password,
        created: true,
    })
}

pub(in crate::stdlib::auth) fn bootstrap_local_user_record(
    identifier_kind: &str,
    identifier: &str,
    password: &str,
) -> std::result::Result<LocalUserCreation, String> {
    if password.is_empty() {
        return Err("[auth] local password must not be empty".to_string());
    }

    let kind = identifier_kind.trim().to_ascii_lowercase();
    let identifier_normalized = normalize_local_identifier(&kind, identifier)?;

    if let Some(bootstrap_id) = get_bootstrap_local_user_id_record()? {
        if let Some(identity) = get_local_identity_by_id_record(&bootstrap_id)? {
            if identity.identifier_kind == kind
                && identity.identifier_normalized == identifier_normalized
            {
                return Ok(LocalUserCreation {
                    must_change_password: matches!(
                        identity.state,
                        LocalAccountState::Bootstrap
                            | LocalAccountState::PendingSetup
                            | LocalAccountState::PasswordChangeRequired
                    ),
                    identity,
                    created: false,
                });
            }
        }
        return Err("[auth] bootstrap local user already exists".to_string());
    }

    if let Some(identity) = get_local_identity_by_identifier_record(&kind, &identifier_normalized)?
    {
        store_bootstrap_local_user_id_record(&identity.id)?;
        return Ok(LocalUserCreation {
            must_change_password: matches!(
                identity.state,
                LocalAccountState::Bootstrap
                    | LocalAccountState::PendingSetup
                    | LocalAccountState::PasswordChangeRequired
            ),
            identity,
            created: false,
        });
    }

    let now = chrono::Utc::now().timestamp();
    let identity = LocalIdentity {
        id: uuid::Uuid::new_v4().to_string(),
        identifier_kind: kind,
        identifier: identifier.to_string(),
        identifier_normalized,
        created_at: now,
        updated_at: now,
        state: LocalAccountState::Bootstrap,
        metadata_json: "{}".to_string(),
    };
    let credential = hash_local_password_for_identity(&identity.id, password, true, now)?;
    provision_local_user_record(&identity, &credential, true)?;

    Ok(LocalUserCreation {
        identity,
        must_change_password: true,
        created: true,
    })
}

fn hash_local_password_for_identity(
    local_user_id: &str,
    password: &str,
    must_change_password: bool,
    now: i64,
) -> std::result::Result<LocalCredentialSecret, String> {
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|_| "[auth] failed to hash local password".to_string())?
        .to_string();

    Ok(LocalCredentialSecret {
        local_user_id: local_user_id.to_string(),
        password_hash,
        password_hash_algorithm: "argon2id".to_string(),
        password_hash_params_json: "{}".to_string(),
        password_changed_at: now,
        must_change_password,
    })
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
            if let Err(dummy_err) = verify_all_dummy_local_passwords(password) {
                return Err(dummy_err);
            }
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
    let (_password_matches, _algorithm) = verify_credential_password(&credential, password)?;
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

pub(in crate::stdlib::auth) fn local_user_creation_to_value(creation: LocalUserCreation) -> Value {
    let mut map = HashMap::from([
        (
            "subject_id".to_string(),
            Value::String(creation.identity.id.clone()),
        ),
        (
            "id".to_string(),
            Value::String(creation.identity.id.clone()),
        ),
        ("provider".to_string(), Value::String("local".to_string())),
        (
            "identifier_kind".to_string(),
            Value::String(creation.identity.identifier_kind.clone()),
        ),
        (
            "identifier".to_string(),
            Value::String(creation.identity.identifier.clone()),
        ),
        (
            "identifier_normalized".to_string(),
            Value::String(creation.identity.identifier_normalized.clone()),
        ),
        (
            "state".to_string(),
            Value::String(creation.identity.state.as_str().to_string()),
        ),
        (
            "must_change_password".to_string(),
            Value::Bool(creation.must_change_password),
        ),
        ("created".to_string(), Value::Bool(creation.created)),
    ]);

    if creation.identity.identifier_kind == "email" {
        map.insert(
            "email".to_string(),
            Value::String(creation.identity.identifier.clone()),
        );
    }

    Value::Map(map)
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
