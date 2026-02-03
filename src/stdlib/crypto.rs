//! std/crypto module - Cryptographic operations

use crate::error::IntentError;
use crate::interpreter::Value;
use hmac::{Hmac, Mac};
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
                    let mut mac = HmacSha256::new_from_slice(key.as_bytes())
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

    module
}
