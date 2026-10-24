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

/// Render a TOTP enrollment URI as local QR geometry, never as XML content.
pub(super) fn render_totp_qr_svg(uri: &str) -> std::result::Result<String, String> {
    use qrcode::{Color, EcLevel, QrCode};
    use std::fmt::Write;

    const INVALID_URI: &str = "Invalid TOTP enrollment URI";
    if uri.len() > 2048 {
        return Err("TOTP URI must be at most 2048 UTF-8 bytes".to_string());
    }
    // The URL parser accepts whitespace and malformed percent escapes, and
    // totp-rs 5.7 can panic on a hostless URL. Reject these before its parser.
    let bytes = uri.as_bytes();
    if uri.chars().any(|c| c.is_whitespace() || c.is_control())
        || bytes.iter().enumerate().any(|(i, byte)| {
            *byte == b'%'
                && (bytes.get(i + 1).is_none_or(|b| !b.is_ascii_hexdigit())
                    || bytes.get(i + 2).is_none_or(|b| !b.is_ascii_hexdigit()))
        })
    {
        return Err(INVALID_URI.to_string());
    }
    let url = reqwest::Url::parse(uri).map_err(|_| INVALID_URI.to_string())?;
    if url.scheme() != "otpauth"
        || url.host_str() != Some("totp")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(INVALID_URI.to_string());
    }
    let totp = TOTP::from_url(uri).map_err(|_| INVALID_URI.to_string())?;
    if totp.account_name.is_empty() || totp.step == 0 {
        return Err(INVALID_URI.to_string());
    }

    let code = QrCode::with_error_correction_level(uri.as_bytes(), EcLevel::Q)
        .map_err(|_| "TOTP URI exceeds QR code capacity".to_string())?;
    let side = code.width() + 8; // Four light modules on every edge.
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {side} {side}\" width=\"{}\" height=\"{}\" shape-rendering=\"crispEdges\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/><path fill=\"black\" d=\"",
        side * 4, side * 4,
    );
    for y in 0..code.width() {
        for x in 0..code.width() {
            if code[(x, y)] == Color::Dark {
                // Writing to a String is infallible. Only integer geometry is emitted.
                write!(svg, "M{} {}h1v1h-1z", x + 4, y + 4).unwrap();
            }
        }
    }
    svg.push_str("\"/></svg>");
    Ok(svg)
}

/// Verify a TOTP code
pub fn verify_totp_code(secret: &str, code: &str, email: &str, issuer: &str) -> bool {
    match create_totp(secret, email, issuer) {
        Ok(totp) => totp.check_current(code).unwrap_or(false),
        Err(_) => false,
    }
}
