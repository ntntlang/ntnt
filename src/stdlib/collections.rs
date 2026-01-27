//! std/collections module - Array and collection manipulation functions

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Initialize the std/collections module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // push(arr, item) -> Array (returns new array with item added)
    module.insert(
        "push".to_string(),
        Value::NativeFunction {
            name: "push".to_string(),
            arity: 2,
            func: |args| match &args[0] {
                Value::Array(arr) => {
                    let mut new_arr = arr.clone();
                    new_arr.push(args[1].clone());
                    Ok(Value::Array(new_arr))
                }
                _ => Err(IntentError::TypeError(
                    "push() requires an array".to_string(),
                )),
            },
        },
    );

    // pop(arr) -> (Array, Option<Value>) - returns (new array without last, removed element)
    module.insert(
        "pop".to_string(),
        Value::NativeFunction {
            name: "pop".to_string(),
            arity: 1,
            func: |args| {
                match &args[0] {
                    Value::Array(arr) => {
                        let mut new_arr = arr.clone();
                        let popped = new_arr.pop();
                        let opt_val = match popped {
                            Some(v) => Value::EnumValue {
                                enum_name: "Option".to_string(),
                                variant: "Some".to_string(),
                                values: vec![v],
                            },
                            None => Value::EnumValue {
                                enum_name: "Option".to_string(),
                                variant: "None".to_string(),
                                values: vec![],
                            },
                        };
                        // Return tuple of (new array, popped value)
                        Ok(Value::Array(vec![Value::Array(new_arr), opt_val]))
                    }
                    _ => Err(IntentError::TypeError(
                        "pop() requires an array".to_string(),
                    )),
                }
            },
        },
    );

    // first(arr) -> Option<Value> or first(arr, default) -> Value
    module.insert(
        "first".to_string(),
        Value::NativeFunction {
            name: "first".to_string(),
            arity: 0, // Variable: 1 or 2 args
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "first() requires 1 or 2 arguments".to_string(),
                    ));
                }

                let arr = match &args[0] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(IntentError::TypeError(
                            "first() requires an array as first argument".to_string(),
                        ))
                    }
                };

                // If default provided, return value or default directly
                if args.len() == 2 {
                    return Ok(arr.first().cloned().unwrap_or_else(|| args[1].clone()));
                }

                // No default: return Option
                match arr.first() {
                    Some(v) => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: vec![v.clone()],
                    }),
                    None => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    }),
                }
            },
        },
    );

    // last(arr) -> Option<Value> or last(arr, default) -> Value
    module.insert(
        "last".to_string(),
        Value::NativeFunction {
            name: "last".to_string(),
            arity: 0, // Variable: 1 or 2 args
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "last() requires 1 or 2 arguments".to_string(),
                    ));
                }

                let arr = match &args[0] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(IntentError::TypeError(
                            "last() requires an array as first argument".to_string(),
                        ))
                    }
                };

                // If default provided, return value or default directly
                if args.len() == 2 {
                    return Ok(arr.last().cloned().unwrap_or_else(|| args[1].clone()));
                }

                // No default: return Option
                match arr.last() {
                    Some(v) => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: vec![v.clone()],
                    }),
                    None => Ok(Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    }),
                }
            },
        },
    );

    // reverse(arr) -> Array
    module.insert(
        "reverse".to_string(),
        Value::NativeFunction {
            name: "reverse".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Array(arr) => {
                    let mut new_arr = arr.clone();
                    new_arr.reverse();
                    Ok(Value::Array(new_arr))
                }
                _ => Err(IntentError::TypeError(
                    "reverse() requires an array".to_string(),
                )),
            },
        },
    );

    // slice(arr, start, end) -> Array
    module.insert(
        "slice".to_string(),
        Value::NativeFunction {
            name: "slice".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (Value::Array(arr), Value::Int(start), Value::Int(end)) => {
                    let start = *start as usize;
                    let end = (*end as usize).min(arr.len());
                    if start > arr.len() || start > end {
                        return Err(IntentError::RuntimeError("Invalid slice range".to_string()));
                    }
                    Ok(Value::Array(arr[start..end].to_vec()))
                }
                _ => Err(IntentError::TypeError(
                    "slice() requires array, int, int".to_string(),
                )),
            },
        },
    );

    // concat(arr1, arr2) -> Array
    module.insert(
        "concat".to_string(),
        Value::NativeFunction {
            name: "concat".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::Array(arr1), Value::Array(arr2)) => {
                    let mut new_arr = arr1.clone();
                    new_arr.extend(arr2.clone());
                    Ok(Value::Array(new_arr))
                }
                _ => Err(IntentError::TypeError(
                    "concat() requires two arrays".to_string(),
                )),
            },
        },
    );

    // is_empty(arr) -> Bool
    module.insert(
        "is_empty".to_string(),
        Value::NativeFunction {
            name: "is_empty".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Array(arr) => Ok(Value::Bool(arr.is_empty())),
                Value::String(s) => Ok(Value::Bool(s.is_empty())),
                _ => Err(IntentError::TypeError(
                    "is_empty() requires array or string".to_string(),
                )),
            },
        },
    );

    // ========== Map Iteration Functions ==========

    // keys(map) -> Array - Get all keys from a map as an array
    module.insert(
        "keys".to_string(),
        Value::NativeFunction {
            name: "keys".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Map(map) => {
                    let keys: Vec<Value> = map.keys().map(|k| Value::String(k.clone())).collect();
                    Ok(Value::Array(keys))
                }
                _ => Err(IntentError::TypeError("keys() requires a map".to_string())),
            },
        },
    );

    // values(map) -> Array - Get all values from a map as an array
    module.insert(
        "values".to_string(),
        Value::NativeFunction {
            name: "values".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Map(map) => {
                    let values: Vec<Value> = map.values().cloned().collect();
                    Ok(Value::Array(values))
                }
                _ => Err(IntentError::TypeError(
                    "values() requires a map".to_string(),
                )),
            },
        },
    );

    // entries(map) -> Array - Get all key-value pairs as array of [key, value] arrays
    module.insert(
        "entries".to_string(),
        Value::NativeFunction {
            name: "entries".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::Map(map) => {
                    let entries: Vec<Value> = map
                        .iter()
                        .map(|(k, v)| Value::Array(vec![Value::String(k.clone()), v.clone()]))
                        .collect();
                    Ok(Value::Array(entries))
                }
                _ => Err(IntentError::TypeError(
                    "entries() requires a map".to_string(),
                )),
            },
        },
    );

    // has_key(map, key) -> Bool - Check if a map contains a key
    module.insert(
        "has_key".to_string(),
        Value::NativeFunction {
            name: "has_key".to_string(),
            arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::Map(map), Value::String(key)) => Ok(Value::Bool(map.contains_key(key))),
                _ => Err(IntentError::TypeError(
                    "has_key() requires a map and string key".to_string(),
                )),
            },
        },
    );

    // get_key(map, key) -> Option<Value> - Safe map access, returns None if key missing
    // get_key(map, key, default) -> Value - Returns default if key missing
    module.insert("get_key".to_string(), Value::NativeFunction {
        name: "get_key".to_string(),
        arity: 0, // Variable arity: 2 or 3 arguments
        func: |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(IntentError::TypeError(
                    "get_key() requires 2 or 3 arguments: get_key(map, key) or get_key(map, key, default)".to_string()
                ));
            }

            match (&args[0], &args[1]) {
                (Value::Map(map), Value::String(key)) => {
                    match map.get(key) {
                        Some(value) => {
                            if args.len() == 3 {
                                // With default: return the value directly
                                Ok(value.clone())
                            } else {
                                // Without default: return Some(value)
                                Ok(Value::EnumValue {
                                    enum_name: "Option".to_string(),
                                    variant: "Some".to_string(),
                                    values: vec![value.clone()],
                                })
                            }
                        }
                        None => {
                            if args.len() == 3 {
                                // With default: return the default value
                                Ok(args[2].clone())
                            } else {
                                // Without default: return None
                                Ok(Value::EnumValue {
                                    enum_name: "Option".to_string(),
                                    variant: "None".to_string(),
                                    values: vec![],
                                })
                            }
                        }
                    }
                }
                _ => Err(IntentError::TypeError("get_key() requires a map and string key".to_string())),
            }
        },
    });

    module
}
