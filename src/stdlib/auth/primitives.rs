use super::*;

pub fn generate_session_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Generate OAuth state token
pub fn generate_oauth_state() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Generate OIDC nonce
pub fn generate_nonce() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Sign a session ID with HMAC-SHA256
/// Returns: "session_id.signature"
pub fn sign_session_id(session_id: &str, secret: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can accept key of any size");
    mac.update(session_id.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    format!("{}.{}", session_id, signature)
}

/// Verify and extract session ID from signed token
/// Returns Some(session_id) if valid, None if invalid signature
/// Uses constant-time comparison to prevent timing attacks
pub fn verify_session_id(signed_token: &str, secret: &str) -> Option<String> {
    // Split into id and signature
    let parts: Vec<&str> = signed_token.rsplitn(2, '.').collect();
    if parts.len() != 2 {
        return None;
    }
    let signature = parts[0];
    let session_id = parts[1];

    // Decode the provided signature from hex
    let signature_bytes = match hex::decode(signature) {
        Ok(bytes) => bytes,
        Err(_) => return None,
    };

    // Verify signature using constant-time comparison
    type HmacSha256 = Hmac<Sha256>;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can accept key of any size");
    mac.update(session_id.as_bytes());

    // verify_slice uses constant-time comparison internally
    match mac.verify_slice(&signature_bytes) {
        Ok(_) => Some(session_id.to_string()),
        Err(_) => None,
    }
}

// ============================================================================
// SECTION 9: Password Utilities
// ============================================================================

// SECTION 9: MFA/TOTP Functions
// ============================================================================

/// Generate a new TOTP secret
pub fn generate_totp_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

/// Create a TOTP instance from a secret
fn create_totp(secret: &str, email: &str, issuer: &str) -> std::result::Result<TOTP, String> {
    let secret = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|e| format!("Invalid secret: {}", e))?;

    TOTP::new(
        TotpAlgorithm::SHA1,
        6,  // 6 digits
        1,  // 1 step (30 seconds)
        30, // 30 second period
        secret,
        Some(issuer.to_string()),
        email.to_string(),
    )
    .map_err(|e| format!("Failed to create TOTP: {}", e))
}

/// Generate the otpauth:// URI for QR codes
pub fn get_totp_uri(
    secret: &str,
    email: &str,
    issuer: &str,
) -> std::result::Result<String, String> {
    let totp = create_totp(secret, email, issuer)?;
    Ok(totp.get_url())
}

/// Verify a TOTP code
pub fn verify_totp_code(secret: &str, code: &str, email: &str) -> bool {
    match create_totp(secret, email, "NTNT") {
        Ok(totp) => totp.check_current(code).unwrap_or(false),
        Err(_) => false,
    }
}
