//! std/json module - JSON parsing and stringification

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Convert JSON value to Intent Value
pub fn json_to_intent_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::none(),
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
        Value::EnumValue {
            enum_name,
            variant,
            values,
        } if enum_name == "Option" => match variant.as_str() {
            "None" => serde_json::Value::Null,
            "Some" => values
                .first()
                .map(intent_value_to_json)
                .unwrap_or(serde_json::Value::Null),
            _ => serde_json::Value::String(value.to_string()),
        },
        // For other types, convert to string representation
        _ => serde_json::Value::String(value.to_string()),
    }
}

/// Initialize the std/json module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt parse_json
    // @module std/json
    // @module_description JSON parsing and serialization
    // @signature parse_json(json_str: String) -> Result<Any, String>
    // Parses a JSON string into a value.
    //
    // Returns Ok with the parsed value on success, or Err with a descriptive
    // parse error message. Supports all JSON types: objects become Maps,
    // arrays become Arrays, numbers become Int or Float, and null becomes None.
    // @param json_str The JSON string to parse
    // @returns Result containing the parsed value or an error message
    // @see_also stringify, stringify_pretty
    // @since v0.1.0
    // @tags #pure
    // @example parse_json("{\"key\": \"value\"}") => Ok(map { "key": "value" }) ~ "Parse JSON object"
    // @example parse_json("null") => Ok(None) ~ "JSON null becomes None"
    // @error TypeError ~ "parse_json() requires a JSON string" fix: "Pass a string argument"
    // @gotcha JSON null is parsed as None (not Unit), enabling round-trip with stringify(None) → "null"
    module.insert(
        "parse_json".to_string(),
        Value::NativeFunction {
            name: "parse_json".to_string(),
            arity: 1,
            max_arity: 1,
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

    // @ntnt stringify
    // @module std/json
    // @signature stringify(value: Any) -> String
    // Converts a value to a compact JSON string.
    //
    // Maps, arrays, strings, numbers, booleans, and Unit are serialized to
    // their JSON equivalents. Structs are serialized as JSON objects.
    // Option values are unwrapped: None becomes null, Some(v) becomes v.
    // @param value The value to serialize
    // @returns Compact JSON string with no extra whitespace
    // @see_also stringify_pretty, parse_json
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example stringify(map { "key": "value" }) => "{\"key\":\"value\"}" ~ "Compact JSON"
    // @example stringify(None) => "null" ~ "None serializes to null"
    // @example stringify(Some(42)) => "42" ~ "Some unwraps to inner value"
    // @gotcha Both None and Unit serialize to JSON null
    module.insert(
        "stringify".to_string(),
        Value::NativeFunction {
            name: "stringify".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                let json_val = intent_value_to_json(&args[0]);
                Ok(Value::String(json_val.to_string()))
            },
        },
    );

    // @ntnt stringify_pretty
    // @module std/json
    // @signature stringify_pretty(value: Any) -> String
    // Converts a value to a pretty-printed JSON string with indentation.
    //
    // Behaves identically to stringify() but formats the output with newlines
    // and 2-space indentation for readability. None becomes null, Some(v) becomes v.
    // @param value The value to serialize
    // @returns Indented JSON string for human readability
    // @see_also stringify, parse_json
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example stringify_pretty(map { "a": 1 }) ~ "Pretty-printed with newlines and indentation"
    module.insert(
        "stringify_pretty".to_string(),
        Value::NativeFunction {
            name: "stringify_pretty".to_string(),
            arity: 1,
            max_arity: 1,
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
