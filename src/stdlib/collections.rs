//! std/collections module - Array and collection manipulation functions

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Initialize the std/collections module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt push
    // @module std/collections
    // @module_description Higher-order collection operations: transform, filter, reduce, sort, and group
    // @signature push(arr: Array, item: Any) -> Array
    // Returns a new array with the item appended.
    //
    // Does not mutate the original array. The new element is added
    // at the end of the returned array.
    // @param arr The source array
    // @param item The element to append
    // @returns A new array containing all original elements plus the new item
    // @see_also pop, concat, first, last
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example push([1, 2], 3) => [1, 2, 3] ~ "Append element to array"
    // @error TypeError ~ "push() requires an array" fix: "Ensure first argument is an array"
    module.insert(
        "push".to_string(),
        Value::NativeFunction {
            name: "push".to_string(),
            arity: 2,
            max_arity: 2,
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

    // @ntnt pop
    // @module std/collections
    // @signature pop(arr: Array) -> Array<[Array, Option<Any>]>
    // Returns a tuple of [new array without last element, popped element as Option].
    //
    // Does not mutate the original array. Returns a two-element array where the
    // first element is the new array and the second is the popped value wrapped
    // in an Option (Some(value) if the array was non-empty, None if empty).
    // @param arr The source array
    // @returns A two-element array: [remaining array, Option of popped element]
    // @see_also push, first, last, slice
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example pop([1, 2, 3]) => [[1, 2], Some(3)] ~ "Pop last element from array"
    // @error TypeError ~ "pop() requires an array" fix: "Ensure argument is an array"
    module.insert(
        "pop".to_string(),
        Value::NativeFunction {
            name: "pop".to_string(),
            arity: 1,
            max_arity: 1,
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
                            None => Value::none(),
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

    // @ntnt first
    // @module std/collections
    // @signature first(arr: Array, default?: Any) -> Option<Any> | Any
    // Returns the first element of an array.
    //
    // Without a default, returns Option: Some(value) if the array is non-empty,
    // None if empty. With a default, returns the first element directly or the
    // default value if the array is empty.
    // @param arr The source array
    // @param default (optional) Value to return if the array is empty
    // @returns The first element as Option, or the value/default directly
    // @see_also last, push, pop, slice
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example first([1, 2, 3]) => Some(1) ~ "First element wrapped in Option"
    // @example first([], 0) => 0 ~ "Default returned for empty array"
    // @error TypeError ~ "first() requires an array as first argument" fix: "Ensure first argument is an array"
    module.insert(
        "first".to_string(),
        Value::NativeFunction {
            name: "first".to_string(),
            arity: 0, // Variable: 1 or 2 args
            max_arity: 0,
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
                    None => Ok(Value::none()),
                }
            },
        },
    );

    // @ntnt last
    // @module std/collections
    // @signature last(arr: Array, default?: Any) -> Option<Any> | Any
    // Returns the last element of an array.
    //
    // Without a default, returns Option: Some(value) if the array is non-empty,
    // None if empty. With a default, returns the last element directly or the
    // default value if the array is empty.
    // @param arr The source array
    // @param default (optional) Value to return if the array is empty
    // @returns The last element as Option, or the value/default directly
    // @see_also first, push, pop, slice
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example last([1, 2, 3]) => Some(3) ~ "Last element wrapped in Option"
    // @example last([], 0) => 0 ~ "Default returned for empty array"
    // @error TypeError ~ "last() requires an array as first argument" fix: "Ensure first argument is an array"
    module.insert(
        "last".to_string(),
        Value::NativeFunction {
            name: "last".to_string(),
            arity: 0, // Variable: 1 or 2 args
            max_arity: 0,
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
                    None => Ok(Value::none()),
                }
            },
        },
    );

    // @ntnt reverse
    // @module std/collections
    // @signature reverse(arr: Array) -> Array
    // Returns a new array with elements in reverse order.
    //
    // Does not mutate the original array.
    // @param arr The source array
    // @returns A new array with elements reversed
    // @see_also slice, concat, push, first, last
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example reverse([1, 2, 3]) => [3, 2, 1] ~ "Reverse array order"
    // @error TypeError ~ "reverse() requires an array" fix: "Ensure argument is an array"
    module.insert(
        "reverse".to_string(),
        Value::NativeFunction {
            name: "reverse".to_string(),
            arity: 1,
            max_arity: 1,
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

    // @ntnt slice
    // @module std/collections
    // @signature slice(arr: Array, start: Int, end: Int) -> Array
    // Extracts a section of an array from start to end (exclusive).
    //
    // Returns a new array containing elements from index start up to but not
    // including index end. The end index is clamped to the array length.
    // @param arr The source array
    // @param start The starting index (inclusive)
    // @param end The ending index (exclusive)
    // @returns A new array containing the sliced elements
    // @see_also concat, reverse, first, last
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example slice([1, 2, 3, 4], 1, 3) => [2, 3] ~ "Slice from index 1 to 3"
    // @error RuntimeError ~ "Invalid slice range" fix: "Ensure start <= end and start <= array length"
    // @error TypeError ~ "slice() requires array, int, int" fix: "Pass an array and two integer indices"
    module.insert(
        "slice".to_string(),
        Value::NativeFunction {
            name: "slice".to_string(),
            arity: 3,
            max_arity: 3,
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

    // @ntnt concat
    // @module std/collections
    // @signature concat(arr1: Array, arr2: Array) -> Array
    // Concatenates two arrays into a new array.
    //
    // Does not mutate either input array. Returns a new array containing
    // all elements of arr1 followed by all elements of arr2.
    // @param arr1 The first array
    // @param arr2 The second array to append
    // @returns A new array containing elements from both arrays
    // @see_also push, slice, reverse
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example concat([1, 2], [3, 4]) => [1, 2, 3, 4] ~ "Concatenate two arrays"
    // @error TypeError ~ "concat() requires two arrays" fix: "Ensure both arguments are arrays"
    module.insert(
        "concat".to_string(),
        Value::NativeFunction {
            name: "concat".to_string(),
            arity: 2,
            max_arity: 2,
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

    // @ntnt is_empty
    // @module std/collections
    // @signature is_empty(x: Array | String) -> Bool
    // Returns true if the array or string is empty.
    //
    // Works with both Array and String types. For arrays, checks if the
    // length is zero. For strings, checks if the string has no characters.
    // @param x An array or string to check
    // @returns true if the collection has no elements/characters, false otherwise
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example is_empty([]) => true ~ "Empty array"
    // @example is_empty([1]) => false ~ "Non-empty array"
    // @example is_empty("") => true ~ "Empty string"
    // @error TypeError ~ "is_empty() requires array or string" fix: "Pass an array or string"
    module.insert(
        "is_empty".to_string(),
        Value::NativeFunction {
            name: "is_empty".to_string(),
            arity: 1,
            max_arity: 1,
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

    // @ntnt keys
    // @module std/collections
    // @signature keys(m: Map) -> Array<String>
    // Returns an array of all keys in the map.
    //
    // The order of keys is not guaranteed to be consistent.
    // @param m The source map
    // @returns An array of string keys
    // @see_also values, entries, has_key, get_key
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example keys(map { "a": 1, "b": 2 }) => ["a", "b"] ~ "Get map keys"
    // @error TypeError ~ "keys() requires a map" fix: "Ensure argument is a map"
    module.insert(
        "keys".to_string(),
        Value::NativeFunction {
            name: "keys".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| match &args[0] {
                Value::Map(map) => {
                    let keys: Vec<Value> = map.keys().map(|k| Value::String(k.clone())).collect();
                    Ok(Value::Array(keys))
                }
                _ => Err(IntentError::TypeError("keys() requires a map".to_string())),
            },
        },
    );

    // @ntnt values
    // @module std/collections
    // @signature values(m: Map) -> Array<Any>
    // Returns an array of all values in the map.
    //
    // The order of values corresponds to the order of keys, which is
    // not guaranteed to be consistent.
    // @param m The source map
    // @returns An array of map values
    // @see_also keys, entries, has_key, get_key
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example values(map { "a": 1, "b": 2 }) => [1, 2] ~ "Get map values"
    // @error TypeError ~ "values() requires a map" fix: "Ensure argument is a map"
    module.insert(
        "values".to_string(),
        Value::NativeFunction {
            name: "values".to_string(),
            arity: 1,
            max_arity: 1,
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

    // @ntnt entries
    // @module std/collections
    // @signature entries(m: Map) -> Array<Map>
    // Returns an array of {key, value} maps from the map.
    //
    // Each entry is a map with a "key" field (the string key) and a
    // "value" field (the corresponding value). Access them as entry["key"]
    // and entry["value"].
    // @param m The source map
    // @returns An array of maps, each with "key" and "value" fields
    // @see_also keys, values, has_key, get_key
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example entries(map { "a": 1 }) => [map { "key": "a", "value": 1 }] ~ "Get map entries as {key, value} maps"
    // @error TypeError ~ "entries() requires a map" fix: "Ensure argument is a map"
    module.insert(
        "entries".to_string(),
        Value::NativeFunction {
            name: "entries".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| match &args[0] {
                Value::Map(map) => {
                    let entries: Vec<Value> = map
                        .iter()
                        .map(|(k, v)| {
                            let mut entry = HashMap::new();
                            entry.insert("key".to_string(), Value::String(k.clone()));
                            entry.insert("value".to_string(), v.clone());
                            Value::Map(entry)
                        })
                        .collect();
                    Ok(Value::Array(entries))
                }
                _ => Err(IntentError::TypeError(
                    "entries() requires a map".to_string(),
                )),
            },
        },
    );

    // @ntnt has_key
    // @module std/collections
    // @signature has_key(m: Map, key: String) -> Bool
    // Returns true if the map contains the specified key.
    // @param m The map to search
    // @param key The key to look for
    // @returns true if the key exists in the map, false otherwise
    // @see_also get_key, keys, values, entries
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example has_key(map { "a": 1 }, "a") => true ~ "Key exists"
    // @example has_key(map { "a": 1 }, "b") => false ~ "Key does not exist"
    // @error TypeError ~ "has_key() requires a map and string key" fix: "Pass a map and a string key"
    module.insert(
        "has_key".to_string(),
        Value::NativeFunction {
            name: "has_key".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::Map(map), Value::String(key)) => Ok(Value::Bool(map.contains_key(key))),
                // Non-map first argument: return false instead of crashing.
                // has_key() is a check — it should be safe to call on anything.
                (_, Value::String(_)) => Ok(Value::Bool(false)),
                _ => Err(IntentError::TypeError(
                    "has_key() requires a map and string key".to_string(),
                )),
            },
        },
    );

    // @ntnt get_key
    // @module std/collections
    // @signature get_key(m: Map, key: String, default?: Any) -> Option<Any> | Any
    // Gets a value from a map by key with safe access.
    //
    // Without a default, returns Option: Some(value) if the key exists,
    // None if missing. With a default, returns the value directly or the
    // default value if the key is not found.
    // @param m The source map
    // @param key The key to look up
    // @param default (optional) Value to return if the key is not found
    // @returns The value as Option, or the value/default directly
    // @see_also has_key, keys, values, entries
    // @since v0.1.0
    // @tags #pure, #deterministic
    // @example get_key(map { "a": 1 }, "a") => Some(1) ~ "Key found, wrapped in Option"
    // @example get_key(map { "a": 1 }, "b", 0) => 0 ~ "Key missing, default returned"
    // @error TypeError ~ "get_key() requires a map and string key" fix: "Pass a map and a string key"
    module.insert("get_key".to_string(), Value::NativeFunction {
        name: "get_key".to_string(),
        arity: 2,
        max_arity: 3,
        func: |args| {
            eprintln!("[DEPRECATED] get_key() is deprecated. Use get_or() instead.");
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
                                Ok(Value::none())
                            }
                        }
                    }
                }
                _ => Err(IntentError::TypeError("get_key() requires a map and string key".to_string())),
            }
        },
    });

    // @ntnt get_index
    // @module std/collections
    // @signature get_index(arr: Array, index: Int, default?: Any) -> Option<Any> | Any
    // Gets an element from an array by index with safe access.
    //
    // Without a default, returns Option: Some(value) if the index is valid,
    // None if out of bounds. With a default, returns the element directly or
    // the default value if the index is out of bounds. Supports negative
    // indexing: -1 is the last element, -2 is second-to-last, etc.
    // @param arr The source array
    // @param index The index to access (supports negative indexing)
    // @param default (optional) Value to return if the index is out of bounds
    // @returns The element as Option, or the value/default directly
    // @see_also first, last, get_key, slice
    // @since v0.4.0
    // @tags #pure, #deterministic
    // @example get_index([10, 20, 30], 1) => Some(20) ~ "Index found, wrapped in Option"
    // @example get_index([10, 20, 30], 5) => None ~ "Out of bounds returns None"
    // @example get_index([10, 20, 30], 1, 0) => 20 ~ "With default, returns value directly"
    // @example get_index([10, 20, 30], 5, 0) => 0 ~ "Out of bounds returns default"
    // @example get_index([10, 20, 30], -1) => Some(30) ~ "Negative index from end"
    // @error TypeError ~ "get_index() requires an array and integer index" fix: "Pass an array and an integer index"
    module.insert("get_index".to_string(), Value::NativeFunction {
        name: "get_index".to_string(),
        arity: 0, // Variable arity: 2 or 3 arguments
        max_arity: 0,
        func: |args| {
            if args.len() < 2 || args.len() > 3 {
                return Err(IntentError::TypeError(
                    "get_index() requires 2 or 3 arguments: get_index(arr, index) or get_index(arr, index, default)".to_string()
                ));
            }

            let arr = match &args[0] {
                Value::Array(arr) => arr,
                _ => {
                    return Err(IntentError::TypeError(
                        "get_index() requires an array as first argument".to_string(),
                    ))
                }
            };

            let index = match &args[1] {
                Value::Int(i) => *i,
                _ => {
                    return Err(IntentError::TypeError(
                        "get_index() requires an integer as second argument".to_string(),
                    ))
                }
            };

            // Resolve negative indices
            let resolved = if index < 0 {
                let pos = arr.len() as i64 + index;
                if pos < 0 { None } else { Some(pos as usize) }
            } else {
                Some(index as usize)
            };

            let element = resolved.and_then(|i| arr.get(i));

            match element {
                Some(value) => {
                    if args.len() == 3 {
                        Ok(value.clone())
                    } else {
                        Ok(Value::EnumValue {
                            enum_name: "Option".to_string(),
                            variant: "Some".to_string(),
                            values: vec![value.clone()],
                        })
                    }
                }
                None => {
                    if args.len() == 3 {
                        Ok(args[2].clone())
                    } else {
                        Ok(Value::none())
                    }
                }
            }
        },
    });

    // @ntnt merge
    // @module std/collections
    // @signature merge(map1: Map, map2: Map) -> Map
    // Shallow-merges two maps. Values from map2 win on key conflicts.
    //
    // Returns a new map containing all key-value pairs from both maps.
    // If both maps contain the same key, the value from map2 is used.
    // @param map1 The base map
    // @param map2 The map whose values take priority on conflict
    // @returns A new map with merged key-value pairs
    // @see_also get_key, get_or, has_key, keys, values
    // @since v0.3.13
    // @tags #pure, #deterministic
    // @example merge(map { "a": 1, "b": 2 }, map { "b": 3, "c": 4 }) => map { "a": 1, "b": 3, "c": 4 } ~ "Merge with conflict resolution"
    // @error TypeError ~ "merge() requires two maps" fix: "Ensure both arguments are maps"
    module.insert(
        "merge".to_string(),
        Value::NativeFunction {
            name: "merge".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| match (&args[0], &args[1]) {
                (Value::Map(map1), Value::Map(map2)) => {
                    let mut result = map1.clone();
                    for (k, v) in map2.iter() {
                        result.insert(k.clone(), v.clone());
                    }
                    Ok(Value::Map(result))
                }
                _ => Err(IntentError::TypeError(
                    "merge() requires two maps".to_string(),
                )),
            },
        },
    );

    // @ntnt get_or
    // @module std/collections
    // @signature get_or(m: Map, key: String, default: Any) -> Any
    // Gets a value from a map by key, returning a default if the key is missing.
    //
    // Unlike get_key which returns an Option, get_or always returns a plain value.
    // If the key exists, returns its value. If not, returns the default.
    // @param m The source map
    // @param key The key to look up
    // @param default The value to return if the key is not found
    // @returns The value for the key, or the default
    // @see_also get_key, has_key, merge, keys
    // @since v0.3.13
    // @tags #pure, #deterministic
    // @example get_or(map { "name": "Alice" }, "name", "Anonymous") => "Alice" ~ "Key exists"
    // @example get_or(map { "name": "Alice" }, "age", 0) => 0 ~ "Key missing, returns default"
    // @error TypeError ~ "get_or() requires a map, string key, and default value" fix: "Pass a map, string key, and default value"
    module.insert(
        "get_or".to_string(),
        Value::NativeFunction {
            name: "get_or".to_string(),
            arity: 3,
            max_arity: 3,
            func: |args| match (&args[0], &args[1]) {
                (Value::Map(map), Value::String(key)) => {
                    Ok(map.get(key).cloned().unwrap_or_else(|| args[2].clone()))
                }
                // Non-map first argument: return the default value instead of crashing.
                // get_or() exists for defensive access — it should never be the thing that crashes.
                (_, Value::String(_)) => Ok(args[2].clone()),
                _ => Err(IntentError::TypeError(
                    "get_or() requires a map (or any value), string key, and default value"
                        .to_string(),
                )),
            },
        },
    );

    // @ntnt includes
    // @module std/collections
    // @signature includes(arr: Array, value: Any) -> Bool
    // Returns true if the array includes the specified value (deep equality).
    //
    // Iterates through the array and checks each element for equality with the
    // given value. Supports all value types including nested arrays and enums.
    // Named `includes` (not `contains`) to avoid collision with contains() in
    // std/string which checks substrings. Follows JavaScript convention.
    // @param arr The array to search
    // @param value The value to look for
    // @returns true if the value is found in the array, false otherwise
    // @see_also has_key, first, last, is_empty
    // @since v0.4.0
    // @tags #pure, #deterministic
    // @example includes(["red", "green", "blue"], "green") => true ~ "Value found"
    // @example includes([1, 2, 3], 5) => false ~ "Value not found"
    // @error TypeError ~ "includes() requires an array as first argument" fix: "Ensure first argument is an array"
    module.insert(
        "includes".to_string(),
        Value::NativeFunction {
            name: "includes".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| match &args[0] {
                Value::Array(arr) => {
                    let needle = &args[1];
                    let found = arr.iter().any(|item| values_equal(item, needle));
                    Ok(Value::Bool(found))
                }
                _ => Err(IntentError::TypeError(
                    "includes() requires an array as first argument".to_string(),
                )),
            },
        },
    );

    // @ntnt has_value
    // @module std/collections
    // @signature has_value(arr: Array, value: Any) -> Bool
    // Deprecated: use includes() instead. Alias for backward compatibility.
    // @param arr The array to search
    // @param value The value to look for
    // @returns true if the value is found in the array, false otherwise
    // @see_also includes
    // @since v0.4.0
    // @tags #pure, #deterministic, #deprecated
    // @example has_value([1, 2, 3], 2) => true ~ "Deprecated alias for includes"
    module.insert(
        "has_value".to_string(),
        Value::NativeFunction {
            name: "has_value".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| {
                eprintln!("[DEPRECATED] has_value() is deprecated. Use includes() from std/collections instead.");
                match &args[0] {
                    Value::Array(arr) => {
                        let needle = &args[1];
                        let found = arr.iter().any(|item| values_equal(item, needle));
                        Ok(Value::Bool(found))
                    }
                    _ => Err(IntentError::TypeError(
                        "has_value() requires an array as first argument".to_string(),
                    )),
                }
            },
        },
    );

    module
}

/// Deep equality comparison for Values (used by contains)
fn values_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::String(a), Value::String(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Unit, Value::Unit) => true,
        (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
        (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| values_equal(x, y))
        }
        (
            Value::EnumValue {
                enum_name: en1,
                variant: v1,
                values: vals1,
            },
            Value::EnumValue {
                enum_name: en2,
                variant: v2,
                values: vals2,
            },
        ) => {
            en1 == en2
                && v1 == v2
                && vals1.len() == vals2.len()
                && vals1
                    .iter()
                    .zip(vals2.iter())
                    .all(|(x, y)| values_equal(x, y))
        }
        _ => false,
    }
}
