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
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

/// Initialize the std/crypto module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt sha256
    // @module std/crypto
    // @module_description Cryptographic hashing and random value generation
    // @signature sha256(data: String | Array<Int>) -> String
    // SHA-256 hash as hex string. Accepts string or byte array.
    // @param data The input data to hash (string or byte array)
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example sha256("hello") => "2cf24dba..." ~ "Hash a string"
    module.insert(
        "sha256".to_string(),
        Value::NativeFunction {
            name: "sha256".to_string(),
            arity: 1,
            func: |args| {
                match &args[0] {
                    Value::String(data) => {
                        let mut hasher = Sha256::new();
                        hasher.update(data.as_bytes());
                        let result = hasher.finalize();
                        Ok(Value::String(hex::encode(result)))
                    }
                    Value::Array(bytes) => {
                        // Handle array of bytes
                        let byte_vec: std::result::Result<Vec<u8>, _> = bytes
                            .iter()
                            .map(|v| match v {
                                Value::Int(i) => Ok(*i as u8),
                                _ => Err(IntentError::TypeError(
                                    "sha256() array must contain integers".to_string(),
                                )),
                            })
                            .collect();
                        let byte_vec = byte_vec?;
                        let mut hasher = Sha256::new();
                        hasher.update(&byte_vec);
                        let result = hasher.finalize();
                        Ok(Value::String(hex::encode(result)))
                    }
                    _ => Err(IntentError::TypeError(
                        "sha256() requires a string or byte array".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt sha256_bytes
    // @module std/crypto
    // @signature sha256_bytes(data: String) -> Array<Int>
    // SHA-256 hash as byte array. Returns array of 32 integers (0-255).
    // @param data The input string to hash
    // @see_also sha256
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example sha256_bytes("hello")[0] => 44 ~ "First byte of SHA-256 hash of 'hello'"
    module.insert(
        "sha256_bytes".to_string(),
        Value::NativeFunction {
            name: "sha256_bytes".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(data) => {
                    let mut hasher = Sha256::new();
                    hasher.update(data.as_bytes());
                    let result = hasher.finalize();
                    let bytes: Vec<Value> = result.iter().map(|b| Value::Int(*b as i64)).collect();
                    Ok(Value::Array(bytes))
                }
                _ => Err(IntentError::TypeError(
                    "sha256_bytes() requires a string".to_string(),
                )),
            },
        },
    );

    // @ntnt hmac_sha256
    // @module std/crypto
    // @signature hmac_sha256(key: String, data: String) -> String
    // HMAC-SHA256 message authentication code as hex string.
    // @param key The secret key for HMAC
    // @param data The data to authenticate
    // @see_also sha256
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example hmac_sha256("secret", "message") ~ "Returns HMAC-SHA256 as 64-char hex string"
    module.insert(
        "hmac_sha256".to_string(),
        Value::NativeFunction {
            name: "hmac_sha256".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::String(key), Value::String(data)) => {
                    type HmacSha256 = Hmac<Sha256>;
                    let mut mac = <HmacSha256 as Mac>::new_from_slice(key.as_bytes())
                        .map_err(|e| IntentError::RuntimeError(format!("HMAC error: {}", e)))?;
                    mac.update(data.as_bytes());
                    let result = mac.finalize();
                    Ok(Value::String(hex::encode(result.into_bytes())))
                }
                _ => Err(IntentError::TypeError(
                    "hmac_sha256() requires two strings (key, data)".to_string(),
                )),
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
            func: |args| match &args[0] {
                Value::Int(n) => {
                    if *n < 0 || *n > 1024 * 1024 {
                        return Err(IntentError::RuntimeError(
                            "random_bytes() size must be 0-1048576".to_string(),
                        ));
                    }
                    let mut bytes = vec![0u8; *n as usize];
                    rand::thread_rng().fill_bytes(&mut bytes);
                    let values: Vec<Value> = bytes.iter().map(|b| Value::Int(*b as i64)).collect();
                    Ok(Value::Array(values))
                }
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::Int(n) => {
                    if *n < 0 || *n > 1024 * 1024 {
                        return Err(IntentError::RuntimeError(
                            "random_hex() size must be 0-1048576".to_string(),
                        ));
                    }
                    let mut bytes = vec![0u8; *n as usize];
                    rand::thread_rng().fill_bytes(&mut bytes);
                    Ok(Value::String(hex::encode(bytes)))
                }
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::Array(bytes) => {
                    let byte_vec: std::result::Result<Vec<u8>, _> = bytes
                        .iter()
                        .map(|v| match v {
                            Value::Int(i) => Ok(*i as u8),
                            _ => Err(IntentError::TypeError(
                                "hex_encode() array must contain integers".to_string(),
                            )),
                        })
                        .collect();
                    Ok(Value::String(hex::encode(byte_vec?)))
                }
                Value::String(s) => Ok(Value::String(hex::encode(s.as_bytes()))),
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(hex_str) => match hex::decode(hex_str) {
                    Ok(bytes) => {
                        let values: Vec<Value> =
                            bytes.iter().map(|b| Value::Int(*b as i64)).collect();
                        Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::Array(values)],
                        })
                    }
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(e.to_string())],
                    }),
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "hash_password() requires 1 or 2 arguments (password, optional cost)"
                            .to_string(),
                    ));
                }

                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "hash_password() requires a string password".to_string(),
                        ))
                    }
                };

                // Default cost is 12, which is a good balance of security and speed
                let cost: u32 = if args.len() == 2 {
                    match &args[1] {
                        Value::Int(c) => {
                            if *c < 10 || *c > 31 {
                                return Ok(Value::EnumValue {
                                    enum_name: "Result".to_string(),
                                    variant: "Err".to_string(),
                                    values: vec![Value::String(
                                        "Cost must be between 10 and 31 (OWASP minimum)"
                                            .to_string(),
                                    )],
                                });
                            }
                            *c as u32
                        }
                        _ => {
                            return Err(IntentError::TypeError(
                                "hash_password() cost must be an integer".to_string(),
                            ))
                        }
                    }
                } else {
                    12
                };

                match bcrypt::hash(&password, cost) {
                    Ok(hash) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![Value::String(hash)],
                    }),
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Hash error: {}", e))],
                    }),
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
            func: |args| {
                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "verify_password() requires a string password".to_string(),
                        ))
                    }
                };

                let hash = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "verify_password() requires a string hash".to_string(),
                        ))
                    }
                };

                match bcrypt::verify(&password, &hash) {
                    Ok(valid) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![Value::Bool(valid)],
                    }),
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Verify error: {}", e))],
                    }),
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
            func: |args| {
                let hash = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(data) => Ok(Value::String(STANDARD.encode(data.as_bytes()))),
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(encoded) => match STANDARD.decode(encoded.as_bytes()) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(s) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::String(s)],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("UTF-8 error: {}", e))],
                        }),
                    },
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Base64 decode error: {}", e))],
                    }),
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(data) => Ok(Value::String(URL_SAFE_NO_PAD.encode(data.as_bytes()))),
                _ => Err(IntentError::TypeError(
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
            func: |args| match &args[0] {
                Value::String(encoded) => match URL_SAFE_NO_PAD.decode(encoded.as_bytes()) {
                    Ok(bytes) => match String::from_utf8(bytes) {
                        Ok(s) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::String(s)],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("UTF-8 error: {}", e))],
                        }),
                    },
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Base64url decode error: {}", e))],
                    }),
                },
                _ => Err(IntentError::TypeError(
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
            func: |args| {
                let plaintext = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "aes_encrypt() requires a string plaintext".to_string(),
                        ))
                    }
                };
                let key_hex = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "aes_encrypt() requires a hex string key".to_string(),
                        ))
                    }
                };

                let key_bytes = match hex::decode(&key_hex) {
                    Ok(b) if b.len() == 32 => b,
                    Ok(b) => {
                        return Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!(
                                "Key must be 32 bytes (64 hex chars), got {} bytes",
                                b.len()
                            ))],
                        })
                    }
                    Err(e) => {
                        return Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Invalid hex key: {}", e))],
                        })
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
                        Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::String(STANDARD.encode(&combined))],
                        })
                    }
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Encryption error: {}", e))],
                    }),
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
            func: |args| {
                let ciphertext_b64 = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "aes_decrypt() requires a string ciphertext".to_string(),
                        ))
                    }
                };
                let key_hex = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "aes_decrypt() requires a hex string key".to_string(),
                        ))
                    }
                };

                let key_bytes = match hex::decode(&key_hex) {
                    Ok(b) if b.len() == 32 => b,
                    Ok(b) => {
                        return Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!(
                                "Key must be 32 bytes (64 hex chars), got {} bytes",
                                b.len()
                            ))],
                        })
                    }
                    Err(e) => {
                        return Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Invalid hex key: {}", e))],
                        })
                    }
                };

                let combined = match STANDARD.decode(ciphertext_b64.as_bytes()) {
                    Ok(b) => b,
                    Err(e) => {
                        return Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Base64 decode error: {}", e))],
                        })
                    }
                };

                if combined.len() < 12 {
                    return Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(
                            "Ciphertext too short (missing nonce)".to_string(),
                        )],
                    });
                }

                let (nonce_bytes, ciphertext) = combined.split_at(12);
                let nonce = Nonce::from_slice(nonce_bytes);
                let cipher = Aes256Gcm::new_from_slice(&key_bytes).unwrap();

                match cipher.decrypt(nonce, ciphertext.as_ref()) {
                    Ok(plaintext) => match String::from_utf8(plaintext) {
                        Ok(s) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Ok".to_string(),
                            values: vec![Value::String(s)],
                        }),
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("UTF-8 error: {}", e))],
                        }),
                    },
                    Err(e) => Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Err".to_string(),
                        values: vec![Value::String(format!("Decryption error: {}", e))],
                    }),
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
            func: |args| {
                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "argon2_hash() requires a string password".to_string(),
                        ))
                    }
                };

                let params = Params::new(19456, 2, 1, None).map_err(|e| {
                    IntentError::RuntimeError(format!("Argon2 params error: {}", e))
                })?;
                let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
                let salt = SaltString::generate(&mut OsRng);

                match argon2.hash_password(password.as_bytes(), &salt) {
                    Ok(hash) => Ok(Value::String(hash.to_string())),
                    Err(e) => Err(IntentError::RuntimeError(format!(
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
            func: |args| {
                let password = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "argon2_verify() requires a string password".to_string(),
                        ))
                    }
                };
                let hash_str = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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

    module
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
