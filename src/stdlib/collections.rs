//! std/collections module - Array and collection manipulation functions

use std::collections::HashMap;
use crate::interpreter::Value;
use crate::error::IntentError;

/// Initialize the std/collections module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();
    
    // push(arr, item) -> Array (returns new array with item added)
    module.insert("push".to_string(), Value::NativeFunction {
        name: "push".to_string(),
        arity: 2,
        func: |args| {
            match &args[0] {
                Value::Array(arr) => {
                    let mut new_arr = arr.clone();
                    new_arr.push(args[1].clone());
                    Ok(Value::Array(new_arr))
                }
                _ => Err(IntentError::TypeError("push() requires an array".to_string())),
            }
        },
    });
    
    // pop(arr) -> (Array, Option<Value>) - returns (new array without last, removed element)
    module.insert("pop".to_string(), Value::NativeFunction {
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
                _ => Err(IntentError::TypeError("pop() requires an array".to_string())),
            }
        },
    });
    
    // first(arr) -> Option<Value>
    module.insert("first".to_string(), Value::NativeFunction {
        name: "first".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Array(arr) => {
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
                }
                _ => Err(IntentError::TypeError("first() requires an array".to_string())),
            }
        },
    });
    
    // last(arr) -> Option<Value>
    module.insert("last".to_string(), Value::NativeFunction {
        name: "last".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Array(arr) => {
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
                }
                _ => Err(IntentError::TypeError("last() requires an array".to_string())),
            }
        },
    });
    
    // reverse(arr) -> Array
    module.insert("reverse".to_string(), Value::NativeFunction {
        name: "reverse".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Array(arr) => {
                    let mut new_arr = arr.clone();
                    new_arr.reverse();
                    Ok(Value::Array(new_arr))
                }
                _ => Err(IntentError::TypeError("reverse() requires an array".to_string())),
            }
        },
    });
    
    // slice(arr, start, end) -> Array
    module.insert("slice".to_string(), Value::NativeFunction {
        name: "slice".to_string(),
        arity: 3,
        func: |args| {
            match (&args[0], &args[1], &args[2]) {
                (Value::Array(arr), Value::Int(start), Value::Int(end)) => {
                    let start = *start as usize;
                    let end = (*end as usize).min(arr.len());
                    if start > arr.len() || start > end {
                        return Err(IntentError::RuntimeError("Invalid slice range".to_string()));
                    }
                    Ok(Value::Array(arr[start..end].to_vec()))
                }
                _ => Err(IntentError::TypeError("slice() requires array, int, int".to_string())),
            }
        },
    });
    
    // concat(arr1, arr2) -> Array
    module.insert("concat".to_string(), Value::NativeFunction {
        name: "concat".to_string(),
        arity: 2,
        func: |args| {
            match (&args[0], &args[1]) {
                (Value::Array(arr1), Value::Array(arr2)) => {
                    let mut new_arr = arr1.clone();
                    new_arr.extend(arr2.clone());
                    Ok(Value::Array(new_arr))
                }
                _ => Err(IntentError::TypeError("concat() requires two arrays".to_string())),
            }
        },
    });
    
    // is_empty(arr) -> Bool
    module.insert("is_empty".to_string(), Value::NativeFunction {
        name: "is_empty".to_string(),
        arity: 1,
        func: |args| {
            match &args[0] {
                Value::Array(arr) => Ok(Value::Bool(arr.is_empty())),
                Value::String(s) => Ok(Value::Bool(s.is_empty())),
                _ => Err(IntentError::TypeError("is_empty() requires array or string".to_string())),
            }
        },
    });
    
    module
}
