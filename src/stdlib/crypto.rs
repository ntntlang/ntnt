//! std/crypto module - Cryptographic operations

use crate::error::IntentError;
use crate::interpreter::Value;
use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit as AesKeyInit, Nonce};
use argon2::password_hash::SaltString;
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use regex::Regex;
use sha2::{Digest, Sha256, Sha384, Sha512};
use std::collections::HashMap;
use std::sync::OnceLock;
use uuid::Uuid;

/// Per-process secret for CSRF token generation/validation
static CSRF_SECRET: OnceLock<String> = OnceLock::new();

fn get_csrf_secret() -> &'static str {
    CSRF_SECRET.get_or_init(|| Uuid::new_v4().to_string())
}

/// Initialize the std/crypto module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt base64_decode_bytes
    // @module std/crypto
    // @signature base64_decode_bytes(encoded: String) -> Result<Array<Int>, String>
    // Decode standard padded Base64 without interpreting UTF-8.
    // Decoded payload is limited to 16 MiB; Value arrays use more heap memory.
    // @param encoded Standard Base64 text.
    // @since v0.5.4
    // @example base64_decode_bytes("AP8=") => Ok([0, 255]) ~ "Decode binary bytes"
    module.insert(
        "base64_decode_bytes".into(),
        Value::NativeFunction {
            name: "base64_decode_bytes".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| decode_bytes(&args[0], &STANDARD),
        },
    );

    // @ntnt base64url_encode_bytes
    // @module std/crypto
    // @signature base64url_encode_bytes(data: Array<Int>) -> String
    // Encode checked raw bytes as URL-safe Base64 without padding.
    // @since v0.5.4
    // @example base64url_encode_bytes([255]) ~ "Convert exact bytes"
    // @param data Checked integer bytes in 0..255.
    module.insert(
        "base64url_encode_bytes".into(),
        Value::NativeFunction {
            name: "base64url_encode_bytes".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(Value::String(
                    URL_SAFE_NO_PAD.encode(checked_crypto_bytes(&args[0], false)?),
                ))
            },
        },
    );

    // @ntnt base64url_decode_bytes
    // @module std/crypto
    // @signature base64url_decode_bytes(encoded: String) -> Result<Array<Int>, String>
    // Decode unpadded URL-safe Base64; decoded payload cap 16 MiB (Value arrays use more heap).
    // @since v0.5.4
    // @example base64url_decode_bytes("_w") ~ "Convert exact bytes"
    // @param encoded URL-safe unpadded Base64 text.
    module.insert(
        "base64url_decode_bytes".into(),
        Value::NativeFunction {
            name: "base64url_decode_bytes".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| decode_bytes(&args[0], &URL_SAFE_NO_PAD),
        },
    );

    // @ntnt utf8_encode
    // @module std/crypto
    // @signature utf8_encode(text: String) -> Array<Int>
    // Encode text as exact UTF-8 bytes.
    // @since v0.5.4
    // @example utf8_encode("hello") ~ "Convert exact bytes"
    // @param text Text to encode without normalization.
    module.insert(
        "utf8_encode".into(),
        Value::NativeFunction {
            name: "utf8_encode".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(s) => Ok(byte_values(s.as_bytes())),
                _ => Err(IntentError::type_error("utf8_encode expects String")),
            },
        },
    );

    // @ntnt utf8_decode
    // @module std/crypto
    // @signature utf8_decode(data: Array<Int>) -> Result<String, String>
    // Decode checked bytes as UTF-8; invalid UTF-8 returns Err without lossy replacement.
    // @since v0.5.4
    // @example utf8_decode([104, 105]) ~ "Convert exact bytes"
    // @param data Checked integer bytes in 0..255; invalid UTF-8 returns Err.
    module.insert(
        "utf8_decode".into(),
        Value::NativeFunction {
            name: "utf8_decode".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(match checked_crypto_bytes(&args[0], false) {
                    Ok(b) => match String::from_utf8(b) {
                        Ok(s) => Value::ok(Value::String(s)),
                        Err(e) => Value::err(Value::String(e.to_string())),
                    },
                    Err(e) => Value::err(Value::String(e.to_string())),
                })
            },
        },
    );

    // @ntnt sha512
    // @module std/crypto
    // @signature sha512(data: String | Array<Int>) -> String
    // SHA-512 lowercase hex of UTF-8 text or checked integer bytes in 0..255.
    // @since v0.5.4
    // @param data UTF-8 text or checked integer bytes in 0..255.
    // @example sha512("abc") ~ "Compute a 128-character lowercase digest"
    module.insert(
        "sha512".into(),
        Value::NativeFunction {
            name: "sha512".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(Value::String(hex::encode(Sha512::digest(
                    checked_crypto_bytes(&args[0], true)?,
                ))))
            },
        },
    );

    // @ntnt sha512_bytes
    // @module std/crypto
    // @signature sha512_bytes(data: String | Array<Int>) -> Array<Int>
    // SHA-512 as 64 raw digest bytes; input bytes must be integers in 0..255.
    // @since v0.5.4
    // @param data UTF-8 text or checked integer bytes in 0..255.
    // @example sha512_bytes([0, 255]) ~ "Compute 64 raw digest bytes"
    module.insert(
        "sha512_bytes".into(),
        Value::NativeFunction {
            name: "sha512_bytes".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(byte_values(&Sha512::digest(checked_crypto_bytes(
                    &args[0], true,
                )?)))
            },
        },
    );

    // @ntnt hmac_sha256_bytes
    // @module std/crypto
    // @signature hmac_sha256_bytes(key: String | Array<Int>, data: String | Array<Int>) -> Array<Int>
    // HMAC-SHA256 as 32 raw bytes. Text uses UTF-8; arrays require integers in 0..255.
    // @since v0.5.4
    // @param key UTF-8 text or checked raw key bytes.
    // @param data UTF-8 text or checked raw message bytes.
    // @example hmac_sha256_bytes([1, 2], [0, 255]) ~ "Authenticate binary data"
    module.insert(
        "hmac_sha256_bytes".into(),
        Value::NativeFunction {
            name: "hmac_sha256_bytes".into(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| Ok(byte_values(&checked_hmac(args)?.finalize().into_bytes())),
        },
    );

    // @ntnt hmac_sha256_verify
    // @module std/crypto
    // @signature hmac_sha256_verify(key: String | Array<Int>, data: String | Array<Int>, expected: String | Array<Int>) -> Result<Bool, String>
    // Verify with RustCrypto MAC verification. Malformed hex or non-32-byte tags return Err; a valid mismatch returns Ok(false).
    // @since v0.5.4
    // @param key UTF-8 text or checked raw key bytes.
    // @param data UTF-8 text or checked raw message bytes.
    // @param expected Exactly 32 checked bytes or 64 hexadecimal digits.
    // @example hmac_sha256_verify("key", "message", hmac_sha256("key", "message")) => Ok(true) ~ "Verify an authentic tag"
    module.insert(
        "hmac_sha256_verify".into(),
        Value::NativeFunction {
            name: "hmac_sha256_verify".into(),
            arity: 3,
            max_arity: 3,
            requires: None,
            func: |args| {
                Ok(match verify_hmac(args) {
                    Ok(v) => Value::ok(Value::Bool(v)),
                    Err(e) => Value::err(Value::String(e)),
                })
            },
        },
    );

    // @ntnt sha384
    // @module std/crypto
    // @signature sha384(data: String | Array<Int>) -> String
    // SHA-384 lowercase hex of exact UTF-8 or checked raw bytes.
    //
    // Rejects non-integers and bytes outside 0..255 with a type error.
    // @param data Exact input bytes; strings use UTF-8.
    // @since v0.5.4
    // @example sha384([97, 98, 99]) ~ "Encode exact bytes"
    module.insert(
        "sha384".into(),
        Value::NativeFunction {
            name: "sha384".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let data = checked_crypto_bytes(&args[0], true)?;
                Ok(Value::String(hex::encode(Sha384::digest(data))))
            },
        },
    );

    // @ntnt sha384_bytes
    // @module std/crypto
    // @signature sha384_bytes(data: String | Array<Int>) -> Array<Int>
    // SHA-384 as 48 raw digest bytes for SRI.
    //
    // Rejects non-integers and bytes outside 0..255 with a type error.
    // @param data Exact input bytes; strings use UTF-8.
    // @since v0.5.4
    // @example sha384_bytes([97, 98, 99]) ~ "Encode exact bytes"
    module.insert(
        "sha384_bytes".into(),
        Value::NativeFunction {
            name: "sha384_bytes".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let data = checked_crypto_bytes(&args[0], true)?;
                Ok(Value::Array(
                    Sha384::digest(data)
                        .iter()
                        .map(|b| Value::Int(i64::from(*b)))
                        .collect(),
                ))
            },
        },
    );

    // @ntnt base64_encode_bytes
    // @module std/crypto
    // @signature base64_encode_bytes(data: Array<Int>) -> String
    // RFC4648 standard padded base64 of checked raw bytes.
    //
    // Rejects non-integers and bytes outside 0..255 with a type error.
    // @param data Array of integer bytes in 0..255; strings are not accepted.
    // @since v0.5.4
    // @example base64_encode_bytes([97, 98, 99]) ~ "Encode exact bytes"
    module.insert(
        "base64_encode_bytes".into(),
        Value::NativeFunction {
            name: "base64_encode_bytes".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let data = checked_crypto_bytes(&args[0], false)?;
                Ok(Value::String(STANDARD.encode(data)))
            },
        },
    );

    // @ntnt sha256
    // @module std/crypto
    // @module_description Cryptographic hashing and random value generation
    // @signature sha256(data: String | Array<Int>) -> String
    // SHA-256 hash as hex string. Accepts text or checked integer bytes in 0..255.
    // @param data The input data to hash (string or byte array)
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example sha256("hello") => "2cf24dba..." ~ "Hash a string"
    module.insert(
        "sha256".into(),
        Value::NativeFunction {
            name: "sha256".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(Value::String(hex::encode(Sha256::digest(
                    checked_crypto_bytes(&args[0], true)?,
                ))))
            },
        },
    );

    // @ntnt sha256_bytes
    // @module std/crypto
    // @signature sha256_bytes(data: String | Array<Int>) -> Array<Int>
    // SHA-256 hash as byte array. Returns array of 32 integers (0-255).
    // @param data UTF-8 text or checked integer bytes in 0..255.
    // @see_also sha256
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example sha256_bytes("hello")[0] => 44 ~ "First byte of SHA-256 hash of 'hello'"
    module.insert(
        "sha256_bytes".into(),
        Value::NativeFunction {
            name: "sha256_bytes".into(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                Ok(byte_values(&Sha256::digest(checked_crypto_bytes(
                    &args[0], true,
                )?)))
            },
        },
    );

    // @ntnt hmac_sha256
    // @module std/crypto
    // @signature hmac_sha256(key: String | Array<Int>, data: String | Array<Int>) -> String
    // HMAC-SHA256 lowercase hex over UTF-8 text or checked integer bytes in 0..255.
    // @param key The secret key for HMAC
    // @param data The data to authenticate
    // @see_also sha256
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example hmac_sha256("secret", "message") ~ "Returns HMAC-SHA256 as 64-char hex string"
    module.insert(
        "hmac_sha256".into(),
        Value::NativeFunction {
            name: "hmac_sha256".into(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                Ok(Value::String(hex::encode(
                    checked_hmac(args)?.finalize().into_bytes(),
                )))
            },
        },
    );

    // @ntnt uuid
    // @module std/crypto
    // @signature uuid() -> String
    // Generates a random UUID v4 string.
    // @since v0.2.0
    // @example uuid() => "550e8400-e29b-41d4-a716-446655440000" ~ "Random UUID v4"
    module.insert(
        "uuid".to_string(),
        Value::NativeFunction {
            name: "uuid".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| Ok(Value::String(Uuid::new_v4().to_string())),
        },
    );

    // @ntnt random_bytes
    // @module std/crypto
    // @signature random_bytes(n: Int) -> Array<Int>
    // Generates n cryptographically secure random bytes. Size limit 0-1048576.
    // @param n Number of random bytes to generate
    // @since v0.2.0
    // @example random_bytes(16) ~ "Returns 16 random bytes as array of integers 0-255"
    // @error RuntimeError ~ "size must be 0-1048576" fix: "Reduce the requested byte count"
    module.insert(
        "random_bytes".to_string(),
        Value::NativeFunction {
            name: "random_bytes".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(n) => {
                    if *n < 0 || *n > 1024 * 1024 {
                        return Err(IntentError::runtime_error(
                            "random_bytes() size must be 0-1048576".to_string(),
                        ));
                    }
                    let mut bytes = vec![0u8; *n as usize];
                    rand::thread_rng().fill_bytes(&mut bytes);
                    let values: Vec<Value> = bytes.iter().map(|b| Value::Int(*b as i64)).collect();
                    Ok(Value::Array(values))
                }
                _ => Err(IntentError::type_error(
                    "random_bytes() requires an integer".to_string(),
                )),
            },
        },
    );

    // @ntnt random_hex
    // @module std/crypto
    // @signature random_hex(n: Int) -> String
    // Generates n random bytes as hex string (2n chars).
    // @param n Number of random bytes to generate
    // @see_also random_bytes
    // @since v0.2.0
    // @example random_hex(8) ~ "Returns 16-char hex string from 8 random bytes"
    module.insert(
        "random_hex".to_string(),
        Value::NativeFunction {
            name: "random_hex".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Int(n) => {
                    if *n < 0 || *n > 1024 * 1024 {
                        return Err(IntentError::runtime_error(
                            "random_hex() size must be 0-1048576".to_string(),
                        ));
                    }
                    let mut bytes = vec![0u8; *n as usize];
                    rand::thread_rng().fill_bytes(&mut bytes);
                    Ok(Value::String(hex::encode(bytes)))
                }
                _ => Err(IntentError::type_error(
                    "random_hex() requires an integer".to_string(),
                )),
            },
        },
    );

    // @ntnt hex_encode
    // @module std/crypto
    // @signature hex_encode(data: Array<Int> | String) -> String
    // Encodes bytes or string as hex.
    // @param data Byte array or string to encode
    // @see_also hex_decode
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example hex_encode("hi") => "6869"
    module.insert(
        "hex_encode".to_string(),
        Value::NativeFunction {
            name: "hex_encode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::Array(bytes) => {
                    let byte_vec: std::result::Result<Vec<u8>, _> = bytes
                        .iter()
                        .map(|v| match v {
                            Value::Int(i) => Ok(*i as u8),
                            _ => Err(IntentError::type_error(
                                "hex_encode() array must contain integers".to_string(),
                            )),
                        })
                        .collect();
                    Ok(Value::String(hex::encode(byte_vec?)))
                }
                Value::String(s) => Ok(Value::String(hex::encode(s.as_bytes()))),
                _ => Err(IntentError::type_error(
                    "hex_encode() requires array or string".to_string(),
                )),
            },
        },
    );

    // @ntnt hex_decode
    // @module std/crypto
    // @signature hex_decode(hex: String) -> Result<Array<Int>, String>
    // Decodes hex string to byte array. Returns Err for invalid hex.
    // @param hex The hex string to decode
    // @see_also hex_encode
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example hex_decode("6869") => Ok([104, 105]) ~ "Decode hex to bytes for 'hi'"
    // @example hex_decode("zz") => Err("...") ~ "Invalid hex returns Err"
    module.insert(
        "hex_decode".to_string(),
        Value::NativeFunction {
            name: "hex_decode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(hex_str) => match hex::decode(hex_str) {
                    Ok(bytes) => {
                        let values: Vec<Value> =
                            bytes.iter().map(|b| Value::Int(*b as i64)).collect();
                        Ok(Value::ok(Value::Array(values)))
                    }
                    Err(e) => Ok(Value::err(Value::String(e.to_string()))),
                },
                _ => Err(IntentError::type_error(
                    "hex_decode() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt hash_password
    // @module std/crypto
    // @signature hash_password(password: String, cost?: Int) -> Result<String, String>
    // Hash a password using bcrypt with configurable cost factor.
    //
    // Returns a bcrypt hash string that can be stored in the database.
    // The hash includes the salt, so no separate salt storage is needed.
    // The default cost of 12 provides good security for most applications.
    // Higher costs are more secure but slower — each increment doubles the time.
    // @param password The plaintext password to hash
    // @param cost Work factor (10-31). Default 12. Higher = slower but more secure.
    // @returns Ok(hash_string) on success, Err(message) on failure
    // @see_also verify_password, is_valid_hash
    // @since v0.4.0
    // @tags #io
    // @example hash_password("secret123") => Ok("$2b$12$...") ~ "Hash with default cost"
    // @example hash_password("secret123", 10) => Ok("$2b$10$...") ~ "Hash with minimum cost (faster but still secure)"
    // @example hash_password("secret123", 14) => Ok("$2b$14$...") ~ "Hash with higher cost (more secure)"
    // @error InvalidCost ~ "Cost must be between 10 and 31" fix: "Use a cost value of 10 or higher (OWASP minimum)"
    module.insert(
        "hash_password".to_string(),
        Value::NativeFunction {
            name: "hash_password".to_string(),
            arity: 0, // Variadic: 1-2 args
            max_arity: 0,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "hash_password() requires 1 or 2 arguments (password, optional cost)"
                            .to_string(),
                    ));
                }

                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "hash_password() requires a string password".to_string(),
                        ))
                    }
                };

                // Default cost is 12, which is a good balance of security and speed
                let cost: u32 = if args.len() == 2 {
                    match &args[1] {
                        Value::Int(c) => {
                            if *c < 10 || *c > 31 {
                                return Ok(Value::err(Value::String(
                                    "Cost must be between 10 and 31 (OWASP minimum)".to_string(),
                                )));
                            }
                            *c as u32
                        }
                        _ => {
                            return Err(IntentError::type_error(
                                "hash_password() cost must be an integer".to_string(),
                            ))
                        }
                    }
                } else {
                    12
                };

                match bcrypt::hash(&password, cost) {
                    Ok(hash) => Ok(Value::ok(Value::String(hash))),
                    Err(e) => Ok(Value::err(Value::String(format!("Hash error: {}", e)))),
                }
            },
        },
    );

    // @ntnt verify_password
    // @module std/crypto
    // @signature verify_password(password: String, hash: String) -> Result<Bool, String>
    // Verify a password against a bcrypt hash.
    //
    // Returns Ok(true) if the password matches, Ok(false) if it doesn't match,
    // or Err if the hash is malformed.
    // @param password The plaintext password to verify
    // @param hash The bcrypt hash to verify against
    // @returns Ok(true) if match, Ok(false) if no match, Err(message) if hash is invalid
    // @see_also hash_password, is_valid_hash
    // @since v0.4.0
    // @tags #io
    // @example verify_password("secret123", "$2b$12$...valid_hash...") => Ok(true) ~ "Correct password"
    // @example verify_password("wrong", "$2b$12$...valid_hash...") => Ok(false) ~ "Wrong password"
    // @example verify_password("secret", "not-a-hash") => Err("...") ~ "Invalid hash format"
    module.insert(
        "verify_password".to_string(),
        Value::NativeFunction {
            name: "verify_password".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "verify_password() requires a string password".to_string(),
                        ))
                    }
                };

                let hash = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "verify_password() requires a string hash".to_string(),
                        ))
                    }
                };

                match bcrypt::verify(&password, &hash) {
                    Ok(valid) => Ok(Value::ok(Value::Bool(valid))),
                    Err(e) => Ok(Value::err(Value::String(format!("Verify error: {}", e)))),
                }
            },
        },
    );

    // @ntnt is_valid_hash
    // @module std/crypto
    // @signature is_valid_hash(hash: String) -> Bool
    // Check if a string is a valid bcrypt hash format.
    //
    // This is useful for migrations or validating data before calling verify_password.
    // Does NOT verify the hash is correct — only that it has valid bcrypt structure.
    // @param hash The string to check
    // @returns true if the string matches bcrypt hash format, false otherwise
    // @see_also hash_password, verify_password
    // @since v0.4.0
    // @tags #pure, #deterministic
    // @example is_valid_hash("$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.V") => true ~ "Valid bcrypt hash"
    // @example is_valid_hash("not-a-hash") => false ~ "Plain string"
    // @example is_valid_hash("") => false ~ "Empty string"
    // @example is_valid_hash("$2a$10$N9qo8uLOickgx2ZMRZoMye") => false ~ "Truncated hash"
    module.insert(
        "is_valid_hash".to_string(),
        Value::NativeFunction {
            name: "is_valid_hash".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let hash = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "is_valid_hash() requires a string".to_string(),
                        ))
                    }
                };

                // Bcrypt hash format: $2[aby]$DD$[./A-Za-z0-9]{53}
                // Where DD is the cost factor (two digits)
                // The 53-character suffix is the salt (22 chars) + hash (31 chars) in base64
                let bcrypt_regex = Regex::new(r"^\$2[aby]?\$\d{2}\$[./A-Za-z0-9]{53}$").unwrap();

                Ok(Value::Bool(bcrypt_regex.is_match(&hash)))
            },
        },
    );

    // @ntnt base64_encode
    // @module std/crypto
    // @signature base64_encode(data: String) -> String
    // Encodes a string using standard Base64 encoding (RFC 4648).
    // @param data The string to encode
    // @returns Base64-encoded string
    // @see_also base64_decode, base64url_encode
    // @since v0.3.13
    // @tags #pure, #deterministic
    // @example base64_encode("Hello, World!") => "SGVsbG8sIFdvcmxkIQ==" ~ "Standard base64 encoding"
    module.insert(
        "base64_encode".to_string(),
        Value::NativeFunction {
            name: "base64_encode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(data) => Ok(Value::String(STANDARD.encode(data.as_bytes()))),
                _ => Err(IntentError::type_error(
                    "base64_encode() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt base64_decode
    // @module std/crypto
    // @signature base64_decode(encoded: String) -> Result<String, String>
    // Decodes a standard Base64-encoded string back to plaintext.
    // Returns Err if the input is not valid Base64 or not valid UTF-8.
    // @param encoded The Base64-encoded string to decode
    // @returns Ok(decoded_string) on success, Err(message) on failure
    // @see_also base64_encode, base64url_decode
    // @since v0.3.13
    // @tags #pure, #deterministic
    // @example base64_decode("SGVsbG8sIFdvcmxkIQ==") => Ok("Hello, World!") ~ "Decode base64 string"
    // @example base64_decode("!!!invalid!!!") => Err("...") ~ "Invalid base64 returns Err"
    module.insert(
        "base64_decode".to_string(),
        Value::NativeFunction {
            name: "base64_decode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(encoded) => match STANDARD.decode(encoded.as_bytes()) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(s) => Ok(Value::ok(Value::String(s))),
                        Err(e) => Ok(Value::err(Value::String(format!("UTF-8 error: {}", e)))),
                    },
                    Err(e) => Ok(Value::err(Value::String(format!(
                        "Base64 decode error: {}",
                        e
                    )))),
                },
                _ => Err(IntentError::type_error(
                    "base64_decode() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt base64url_encode
    // @module std/crypto
    // @signature base64url_encode(data: String) -> String
    // Encodes a string using URL-safe Base64 encoding (no padding).
    // Uses the URL_SAFE_NO_PAD alphabet, suitable for URLs and filenames.
    // @param data The string to encode
    // @returns URL-safe Base64-encoded string without padding
    // @see_also base64url_decode, base64_encode
    // @since v0.3.13
    // @tags #pure, #deterministic
    // @example base64url_encode("Hello, World!") => "SGVsbG8sIFdvcmxkIQ" ~ "URL-safe base64 (no padding)"
    module.insert(
        "base64url_encode".to_string(),
        Value::NativeFunction {
            name: "base64url_encode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(data) => Ok(Value::String(URL_SAFE_NO_PAD.encode(data.as_bytes()))),
                _ => Err(IntentError::type_error(
                    "base64url_encode() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt base64url_decode
    // @module std/crypto
    // @signature base64url_decode(encoded: String) -> Result<String, String>
    // Decodes a URL-safe Base64-encoded string (no padding) back to plaintext.
    // Returns Err if the input is not valid URL-safe Base64 or not valid UTF-8.
    // @param encoded The URL-safe Base64-encoded string to decode
    // @returns Ok(decoded_string) on success, Err(message) on failure
    // @see_also base64url_encode, base64_decode
    // @since v0.3.13
    // @tags #pure, #deterministic
    // @example base64url_decode("SGVsbG8sIFdvcmxkIQ") => Ok("Hello, World!") ~ "Decode URL-safe base64"
    // @example base64url_decode("!!!") => Err("...") ~ "Invalid input returns Err"
    module.insert(
        "base64url_decode".to_string(),
        Value::NativeFunction {
            name: "base64url_decode".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| match &args[0] {
                Value::String(encoded) => match URL_SAFE_NO_PAD.decode(encoded.as_bytes()) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(s) => Ok(Value::ok(Value::String(s))),
                        Err(e) => Ok(Value::err(Value::String(format!("UTF-8 error: {}", e)))),
                    },
                    Err(e) => Ok(Value::err(Value::String(format!(
                        "Base64url decode error: {}",
                        e
                    )))),
                },
                _ => Err(IntentError::type_error(
                    "base64url_decode() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt aes_generate_key
    // @module std/crypto
    // @signature aes_generate_key() -> String
    // Generates a random 256-bit AES key, returned as a 64-character hex string.
    // Use this key with aes_encrypt and aes_decrypt.
    // @returns 64-character hex string representing a 256-bit key
    // @see_also aes_encrypt, aes_decrypt
    // @since v0.3.13
    // @tags #io
    // @example aes_generate_key() ~ "Returns a 64-char hex string like 'a1b2c3d4...'"
    module.insert(
        "aes_generate_key".to_string(),
        Value::NativeFunction {
            name: "aes_generate_key".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| {
                let mut key = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut key);
                Ok(Value::String(hex::encode(key)))
            },
        },
    );

    // @ntnt aes_encrypt
    // @module std/crypto
    // @signature aes_encrypt(plaintext: String, key: String) -> Result<String, String>
    // Encrypts plaintext using AES-256-GCM authenticated encryption.
    // The key must be a 64-character hex string (32 bytes). A random 96-bit nonce
    // is generated for each call and prepended to the ciphertext before Base64 encoding.
    // @param plaintext The string to encrypt
    // @param key A 64-character hex string (256-bit key from aes_generate_key)
    // @returns Ok(base64_encoded_nonce_and_ciphertext) on success, Err(message) on failure
    // @see_also aes_decrypt, aes_generate_key
    // @since v0.3.13
    // @tags #io
    // @example aes_encrypt("secret data", aes_generate_key()) ~ "Returns Ok with base64 ciphertext"
    module.insert(
        "aes_encrypt".to_string(),
        Value::NativeFunction {
            name: "aes_encrypt".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let plaintext = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "aes_encrypt() requires a string plaintext".to_string(),
                        ))
                    }
                };
                let key_hex = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "aes_encrypt() requires a hex string key".to_string(),
                        ))
                    }
                };

                let key_bytes = match hex::decode(&key_hex) {
                    Ok(b) if b.len() == 32 => b,
                    Ok(b) => {
                        return Ok(Value::err(Value::String(format!(
                            "Key must be 32 bytes (64 hex chars), got {} bytes",
                            b.len()
                        ))))
                    }
                    Err(e) => {
                        return Ok(Value::err(Value::String(format!("Invalid hex key: {}", e))))
                    }
                };

                let cipher = Aes256Gcm::new_from_slice(&key_bytes).unwrap();
                let mut nonce_bytes = [0u8; 12];
                rand::thread_rng().fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);

                match cipher.encrypt(nonce, plaintext.as_bytes().as_ref()) {
                    Ok(ciphertext) => {
                        let mut combined = nonce_bytes.to_vec();
                        combined.extend_from_slice(&ciphertext);
                        Ok(Value::ok(Value::String(STANDARD.encode(&combined))))
                    }
                    Err(e) => Ok(Value::err(Value::String(format!(
                        "Encryption error: {}",
                        e
                    )))),
                }
            },
        },
    );

    // @ntnt aes_decrypt
    // @module std/crypto
    // @signature aes_decrypt(ciphertext: String, key: String) -> Result<String, String>
    // Decrypts AES-256-GCM encrypted data produced by aes_encrypt.
    // The input is a Base64-encoded string containing the nonce and ciphertext.
    // The key must be the same 64-character hex string used for encryption.
    // @param ciphertext The Base64-encoded string from aes_encrypt
    // @param key A 64-character hex string (256-bit key)
    // @returns Ok(plaintext) on success, Err(message) on failure (wrong key, tampered data, etc.)
    // @see_also aes_encrypt, aes_generate_key
    // @since v0.3.13
    // @tags #io
    // @example aes_decrypt(encrypted, key) ~ "Returns Ok with original plaintext"
    module.insert(
        "aes_decrypt".to_string(),
        Value::NativeFunction {
            name: "aes_decrypt".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let ciphertext_b64 = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "aes_decrypt() requires a string ciphertext".to_string(),
                        ))
                    }
                };
                let key_hex = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "aes_decrypt() requires a hex string key".to_string(),
                        ))
                    }
                };

                let key_bytes = match hex::decode(&key_hex) {
                    Ok(b) if b.len() == 32 => b,
                    Ok(b) => {
                        return Ok(Value::err(Value::String(format!(
                            "Key must be 32 bytes (64 hex chars), got {} bytes",
                            b.len()
                        ))))
                    }
                    Err(e) => {
                        return Ok(Value::err(Value::String(format!("Invalid hex key: {}", e))))
                    }
                };

                let combined = match STANDARD.decode(ciphertext_b64.as_bytes()) {
                    Ok(b) => b,
                    Err(e) => {
                        return Ok(Value::err(Value::String(format!(
                            "Base64 decode error: {}",
                            e
                        ))))
                    }
                };

                if combined.len() < 12 {
                    return Ok(Value::err(Value::String(
                        "Ciphertext too short (missing nonce)".to_string(),
                    )));
                }

                let (nonce_bytes, ciphertext) = combined.split_at(12);
                let nonce = Nonce::from_slice(nonce_bytes);
                let cipher = Aes256Gcm::new_from_slice(&key_bytes).unwrap();

                match cipher.decrypt(nonce, ciphertext.as_ref()) {
                    Ok(plaintext) => match String::from_utf8(plaintext) {
                        Ok(s) => Ok(Value::ok(Value::String(s))),
                        Err(e) => Ok(Value::err(Value::String(format!("UTF-8 error: {}", e)))),
                    },
                    Err(e) => Ok(Value::err(Value::String(format!(
                        "Decryption error: {}",
                        e
                    )))),
                }
            },
        },
    );

    // @ntnt argon2_hash
    // @module std/crypto
    // @signature argon2_hash(password: String) -> String
    // Hashes a password using Argon2id, the recommended password hashing algorithm.
    // Returns a PHC-format string that includes the salt and parameters.
    // Uses OWASP-recommended defaults: m=19456 KiB, t=2 iterations, p=1 parallelism.
    // @param password The plaintext password to hash
    // @returns PHC-format hash string starting with $argon2id$
    // @see_also argon2_verify, hash_password
    // @since v0.3.13
    // @tags #io
    // @example argon2_hash("my_password") ~ "Returns '$argon2id$v=19$m=19456,t=2,p=1$...'"
    module.insert(
        "argon2_hash".to_string(),
        Value::NativeFunction {
            name: "argon2_hash".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "argon2_hash() requires a string password".to_string(),
                        ))
                    }
                };

                let params = Params::new(19456, 2, 1, None).map_err(|e| {
                    IntentError::runtime_error(format!("Argon2 params error: {}", e))
                })?;
                let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
                let salt = SaltString::generate(&mut OsRng);

                match argon2.hash_password(password.as_bytes(), &salt) {
                    Ok(hash) => Ok(Value::String(hash.to_string())),
                    Err(e) => Err(IntentError::runtime_error(format!(
                        "Argon2 hash error: {}",
                        e
                    ))),
                }
            },
        },
    );

    // @ntnt argon2_verify
    // @module std/crypto
    // @signature argon2_verify(password: String, hash: String) -> Bool
    // Verifies a password against an Argon2 hash in PHC format.
    // Returns true if the password matches, false otherwise (including for invalid hashes).
    // @param password The plaintext password to verify
    // @param hash The Argon2 PHC-format hash string to verify against
    // @returns true if password matches, false otherwise
    // @see_also argon2_hash, verify_password
    // @since v0.3.13
    // @tags #io
    // @example argon2_verify("my_password", argon2_hash("my_password")) => true ~ "Correct password"
    // @example argon2_verify("wrong", argon2_hash("my_password")) => false ~ "Wrong password"
    module.insert(
        "argon2_verify".to_string(),
        Value::NativeFunction {
            name: "argon2_verify".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "argon2_verify() requires a string password".to_string(),
                        ))
                    }
                };
                let hash_str = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "argon2_verify() requires a string hash".to_string(),
                        ))
                    }
                };

                let parsed_hash = match PasswordHash::new(&hash_str) {
                    Ok(h) => h,
                    Err(_) => return Ok(Value::Bool(false)),
                };

                let argon2 = Argon2::default();
                Ok(Value::Bool(
                    argon2
                        .verify_password(password.as_bytes(), &parsed_hash)
                        .is_ok(),
                ))
            },
        },
    );

    // @ntnt csrf_generate
    // @module std/crypto
    // @signature csrf_generate() -> Map<String, String>
    // Generate a CSRF token and its HMAC signature for stateless CSRF protection.
    //
    // Returns a map with `token` (random value) and `hash` (HMAC-SHA256 signature).
    // Embed `token` in a hidden form field and `hash` in another hidden field or cookie.
    // Validate on POST with `csrf_validate(token, hash)`.
    // @returns A map with keys `"token"` and `"hash"`.
    // @see_also csrf_validate, hmac_sha256, uuid
    // @since v0.3.0
    // @tags #security
    // @example csrf_generate() ~ "Returns map { \"token\": \"...\", \"hash\": \"...\" }"
    module.insert(
        "csrf_generate".to_string(),
        Value::NativeFunction {
            name: "csrf_generate".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| {
                let token = Uuid::new_v4().to_string();
                let secret = get_csrf_secret();

                type HmacSha256 = Hmac<Sha256>;
                let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes())
                    .map_err(|e| IntentError::runtime_error(format!("HMAC error: {}", e)))?;
                mac.update(token.as_bytes());
                let hash = hex::encode(mac.finalize().into_bytes());

                let mut result = HashMap::new();
                result.insert("token".to_string(), Value::String(token));
                result.insert("hash".to_string(), Value::String(hash));
                Ok(Value::Map(result))
            },
        },
    );

    // @ntnt csrf_validate
    // @module std/crypto
    // @signature csrf_validate(token: String, hash: String) -> Bool
    // Validate a CSRF token against its HMAC hash.
    //
    // Compares the provided token's HMAC-SHA256 against the provided hash
    // using the same per-process secret used by `csrf_generate()`.
    // @param token The CSRF token from the form submission.
    // @param hash The HMAC hash from the form submission.
    // @returns `true` if the token is valid, `false` otherwise.
    // @see_also csrf_generate, hmac_sha256
    // @since v0.3.0
    // @tags #security
    // @example csrf_validate("some-token", "some-hash") => false ~ "Invalid token returns false"
    module.insert(
        "csrf_validate".to_string(),
        Value::NativeFunction {
            name: "csrf_validate".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(token), Value::String(hash)) => {
                    let secret = get_csrf_secret();

                    type HmacSha256 = Hmac<Sha256>;
                    let mut mac = <HmacSha256 as Mac>::new_from_slice(secret.as_bytes())
                        .map_err(|e| IntentError::runtime_error(format!("HMAC error: {}", e)))?;
                    mac.update(token.as_bytes());
                    let expected = hex::encode(mac.finalize().into_bytes());

                    Ok(Value::Bool(expected == *hash))
                }
                _ => Err(IntentError::type_error(
                    "csrf_validate() requires two string arguments (token, hash)".to_string(),
                )),
            },
        },
    );

    module
}

fn checked_crypto_bytes(value: &Value, allow_text: bool) -> Result<Vec<u8>, IntentError> {
    match value {
        Value::String(s) if allow_text => Ok(s.as_bytes().to_vec()),
        Value::Array(values) => values
            .iter()
            .map(|v| match v {
                Value::Int(n) if (0..=255).contains(n) => Ok(*n as u8),
                _ => Err(IntentError::type_error(
                    "expected integer bytes in 0..255".to_string(),
                )),
            })
            .collect(),
        _ => Err(IntentError::type_error(
            "expected raw byte array or permitted UTF-8 string".to_string(),
        )),
    }
}

fn decode_bytes(
    value: &Value,
    engine: &base64::engine::GeneralPurpose,
) -> Result<Value, IntentError> {
    let Value::String(encoded) = value else {
        return Err(IntentError::type_error("expected Base64 String"));
    };
    let unpadded = encoded.trim_end_matches('=');
    let estimated = unpadded.len().checked_mul(3).map(|n| n / 4);
    if encoded.len() > (16 * 1024 * 1024_usize).div_ceil(3) * 4
        || estimated.is_none_or(|n| n > 16 * 1024 * 1024)
    {
        return Ok(Value::err(Value::String(
            "capacity: decoded payload exceeds 16 MiB".into(),
        )));
    }
    Ok(match engine.decode(encoded) {
        Ok(bytes) => Value::ok(byte_values(&bytes)),
        Err(e) => Value::err(Value::String(format!("Base64 decode error: {e}"))),
    })
}

fn byte_values(bytes: &[u8]) -> Value {
    Value::Array(bytes.iter().map(|b| Value::Int(i64::from(*b))).collect())
}

fn checked_hmac(args: &[Value]) -> Result<Hmac<Sha256>, IntentError> {
    let key = checked_crypto_bytes(&args[0], true)?;
    let data = checked_crypto_bytes(&args[1], true)?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key)
        .map_err(|_| IntentError::type_error("invalid HMAC key"))?;
    mac.update(&data);
    Ok(mac)
}
fn verify_hmac(args: &[Value]) -> Result<bool, String> {
    let expected = match &args[2] {
        Value::String(s) if s.len() == 64 => hex::decode(s)
            .map_err(|_| "invalid_argument: expected 64 hexadecimal digits".to_string())?,
        Value::Array(a) if a.len() == 32 => {
            checked_crypto_bytes(&args[2], false).map_err(|e| e.to_string())?
        }
        _ => return Err("invalid_argument: expected exactly 32 tag bytes or 64 hex digits".into()),
    };
    let mac = checked_hmac(args).map_err(|e| e.to_string())?;
    Ok(mac.verify_slice(&expected).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(module: &HashMap<String, Value>, name: &str, args: Vec<Value>) -> Value {
        match module.get(name).unwrap() {
            Value::NativeFunction { func, .. } => func(&args).unwrap(),
            _ => panic!("not a function"),
        }
    }

    fn unwrap_string(v: Value) -> String {
        match v {
            Value::String(s) => s,
            _ => panic!("expected String"),
        }
    }

    fn unwrap_bool(v: Value) -> bool {
        match v {
            Value::Bool(b) => b,
            _ => panic!("expected Bool"),
        }
    }

    fn unwrap_result_ok_string(v: Value) -> String {
        match v {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                unwrap_string(values.into_iter().next().unwrap())
            }
            _ => panic!("expected EnumValue Ok"),
        }
    }

    fn assert_result_err(v: Value) {
        match v {
            Value::EnumValue { variant, .. } => assert_eq!(variant, "Err"),
            _ => panic!("expected EnumValue Err"),
        }
    }

    #[test]
    fn test_base64_encode_decode_roundtrip() {
        let m = init();
        let encoded = unwrap_string(call(
            &m,
            "base64_encode",
            vec![Value::String("Hello, World!".into())],
        ));
        assert_eq!(encoded, "SGVsbG8sIFdvcmxkIQ==");
        let decoded =
            unwrap_result_ok_string(call(&m, "base64_decode", vec![Value::String(encoded)]));
        assert_eq!(decoded, "Hello, World!");
    }

    #[test]
    fn test_base64url_encode_decode_roundtrip() {
        let m = init();
        let encoded = unwrap_string(call(
            &m,
            "base64url_encode",
            vec![Value::String("Hello, World!".into())],
        ));
        assert_eq!(encoded, "SGVsbG8sIFdvcmxkIQ");
        let decoded =
            unwrap_result_ok_string(call(&m, "base64url_decode", vec![Value::String(encoded)]));
        assert_eq!(decoded, "Hello, World!");
    }

    #[test]
    fn test_base64_decode_invalid() {
        let m = init();
        assert_result_err(call(
            &m,
            "base64_decode",
            vec![Value::String("!!!invalid!!!".into())],
        ));
    }

    #[test]
    fn test_aes_encrypt_decrypt_roundtrip() {
        let m = init();
        let key = call(&m, "aes_generate_key", vec![]);
        let encrypted = call(
            &m,
            "aes_encrypt",
            vec![Value::String("secret data".into()), key.clone()],
        );
        let ciphertext = match encrypted {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                values.into_iter().next().unwrap()
            }
            _ => panic!("expected Ok"),
        };
        let decrypted = unwrap_result_ok_string(call(&m, "aes_decrypt", vec![ciphertext, key]));
        assert_eq!(decrypted, "secret data");
    }

    #[test]
    fn test_aes_decrypt_wrong_key() {
        let m = init();
        let key1 = call(&m, "aes_generate_key", vec![]);
        let key2 = call(&m, "aes_generate_key", vec![]);
        let encrypted = call(
            &m,
            "aes_encrypt",
            vec![Value::String("secret".into()), key1],
        );
        let ciphertext = match encrypted {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("expected Ok"),
        };
        assert_result_err(call(&m, "aes_decrypt", vec![ciphertext, key2]));
    }

    #[test]
    fn test_aes_generate_key_length() {
        let m = init();
        let key = unwrap_string(call(&m, "aes_generate_key", vec![]));
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn test_argon2_hash_format() {
        let m = init();
        let hash = unwrap_string(call(
            &m,
            "argon2_hash",
            vec![Value::String("password123".into())],
        ));
        assert!(hash.starts_with("$argon2id$"), "got: {}", hash);
    }

    #[test]
    fn test_argon2_verify_correct() {
        let m = init();
        let hash = call(&m, "argon2_hash", vec![Value::String("password123".into())]);
        assert!(unwrap_bool(call(
            &m,
            "argon2_verify",
            vec![Value::String("password123".into()), hash],
        )));
    }

    #[test]
    fn test_argon2_verify_wrong_password() {
        let m = init();
        let hash = call(&m, "argon2_hash", vec![Value::String("password123".into())]);
        assert!(!unwrap_bool(call(
            &m,
            "argon2_verify",
            vec![Value::String("wrong".into()), hash],
        )));
    }

    #[test]
    fn test_argon2_verify_vs_bcrypt_hash() {
        let m = init();
        let bcrypt_hash =
            Value::String("$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.AWNgY0C1Dq/Cm".into());
        assert!(!unwrap_bool(call(
            &m,
            "argon2_verify",
            vec![Value::String("password".into()), bcrypt_hash],
        )));
    }
}
