//! std/json module - JSON parsing and stringification

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Convert JSON value to Intent Value
pub fn json_to_intent_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Unit,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Unit
            }
        }
        serde_json::Value::String(s) => Value::String(s.clone()),
        serde_json::Value::Array(arr) => {
            Value::Array(arr.iter().map(json_to_intent_value).collect())
        }
        serde_json::Value::Object(obj) => {
            let mut map = HashMap::new();
            for (k, v) in obj {
                map.insert(k.clone(), json_to_intent_value(v));
            }
            Value::Map(map)
        }
    }
}

/// Convert Intent Value to JSON value
pub fn intent_value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Unit => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::Number(serde_json::Number::from(*i)),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(intent_value_to_json).collect())
        }
        Value::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), intent_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        Value::Struct { fields, .. } => {
            let obj: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .map(|(k, v)| (k.clone(), intent_value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        // For other types, convert to string representation
        _ => serde_json::Value::String(value.to_string()),
    }
}

/// Initialize the std/json module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // parse_json(json_str) -> Result<Value, Error>
    module.insert(
        "parse_json".to_string(),
        Value::NativeFunction {
            name: "parse_json".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(json_str) => {
                    match serde_json::from_str::<serde_json::Value>(json_str) {
                        Ok(json_val) => {
                            let intent_val = json_to_intent_value(&json_val);
                            Ok(Value::EnumValue {
                                enum_name: "Result".to_string(),
                                variant: "Ok".to_string(),
                                values: vec![intent_val],
                            })
                        }
                        Err(e) => Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(e.to_string())],
                        }),
                    }
                }
                _ => Err(IntentError::TypeError(
                    "parse_json() requires a JSON string".to_string(),
                )),
            },
        },
    );

    // stringify(value) -> String
    module.insert(
        "stringify".to_string(),
        Value::NativeFunction {
            name: "stringify".to_string(),
            arity: 1,
            func: |args| {
                let json_val = intent_value_to_json(&args[0]);
                Ok(Value::String(json_val.to_string()))
            },
        },
    );

    // stringify_pretty(value) -> String - With indentation
    module.insert(
        "stringify_pretty".to_string(),
        Value::NativeFunction {
            name: "stringify_pretty".to_string(),
            arity: 1,
            func: |args| {
                let json_val = intent_value_to_json(&args[0]);
                match serde_json::to_string_pretty(&json_val) {
                    Ok(s) => Ok(Value::String(s)),
                    Err(e) => Ok(Value::String(format!("{{\"error\": \"{}\"}}", e))),
                }
            },
        },
    );

    module
}
