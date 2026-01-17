//! std/crypto module - Cryptographic operations

use crate::error::IntentError;
use crate::interpreter::Value;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

/// Initialize the std/crypto module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // sha256(data) -> String - SHA-256 hash as hex string
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

    // sha256_bytes(data) -> [Int] - SHA-256 hash as byte array
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

    // hmac_sha256(key, data) -> String - HMAC-SHA256 as hex string
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

    // uuid() -> String - Generate a random UUID v4
    module.insert(
        "uuid".to_string(),
        Value::NativeFunction {
            name: "uuid".to_string(),
            arity: 0,
            func: |_args| Ok(Value::String(Uuid::new_v4().to_string())),
        },
    );

    // random_bytes(n) -> [Int] - Generate n cryptographically secure random bytes
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

    // random_hex(n) -> String - Generate n random bytes as hex string
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

    // hex_encode(bytes) -> String - Encode bytes as hex
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

    // hex_decode(hex_str) -> Result<[Int], Error> - Decode hex string to bytes
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

    module
}
