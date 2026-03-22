//! std/csv module - CSV parsing and stringification

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Initialize the std/csv module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // @ntnt parse_csv
    // @module std/csv
    // @module_description CSV parsing and generation
    // @signature parse_csv(csv: String) -> Array<Array<String>>
    // Parses a CSV string into an array of rows, where each row is an array of strings.
    //
    // Handles quoted fields with commas and escaped double-quotes. Empty rows
    // are automatically skipped. Supports both LF and CRLF line endings.
    // @param csv The CSV string to parse
    // @returns Array of rows, each row an array of field strings
    // @see_also parse_with_headers, stringify
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example parse_csv("a,b\n1,2") => [["a", "b"], ["1", "2"]] ~ "Basic CSV parsing"
    // @error TypeError ~ "parse_csv() requires a string" fix: "Pass a CSV string"
    module.insert(
        "parse_csv".to_string(),
        Value::NativeFunction {
            name: "parse_csv".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let csv_string = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "parse_csv() requires a string".to_string(),
                        ))
                    }
                };

                let delimiter = if args.len() > 1 {
                    match &args[1] {
                        Value::String(d) => d.chars().next().unwrap_or(','),
                        _ => ',',
                    }
                } else {
                    ','
                };

                let rows = parse_csv_string(&csv_string, delimiter)?;

                let result: Vec<Value> = rows
                    .into_iter()
                    .map(|row| Value::Array(row.into_iter().map(Value::String).collect()))
                    .collect();

                Ok(Value::Array(result))
            },
        },
    );

    // @ntnt parse_with_headers
    // @module std/csv
    // @signature parse_with_headers(csv: String) -> Array<Map<String, String>>
    // Parses CSV into an array of maps using the first row as column headers.
    //
    // The first row defines the map keys. Each subsequent row becomes a map
    // with those keys mapped to the corresponding field values.
    // @param csv The CSV string with a header row
    // @returns Array of maps, one per data row
    // @see_also parse_csv, stringify_with_headers
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example parse_with_headers("name,age\nAlice,30") => [map { "name": "Alice", "age": "30" }] ~ "Headers become map keys"
    // @error TypeError ~ "csv.parse_with_headers() requires a string" fix: "Pass a CSV string"
    module.insert(
        "parse_with_headers".to_string(),
        Value::NativeFunction {
            name: "csv_parse_with_headers".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let csv_string = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "csv.parse_with_headers() requires a string".to_string(),
                        ))
                    }
                };

                let delimiter = if args.len() > 1 {
                    match &args[1] {
                        Value::String(d) => d.chars().next().unwrap_or(','),
                        _ => ',',
                    }
                } else {
                    ','
                };

                let rows = parse_csv_string(&csv_string, delimiter)?;

                if rows.is_empty() {
                    return Ok(Value::Array(vec![]));
                }

                let headers = &rows[0];
                let data_rows = &rows[1..];

                let result: Vec<Value> = data_rows
                    .iter()
                    .map(|row| {
                        let mut map = HashMap::new();
                        for (i, header) in headers.iter().enumerate() {
                            let value = row.get(i).cloned().unwrap_or_default();
                            map.insert(header.clone(), Value::String(value));
                        }
                        Value::Map(map)
                    })
                    .collect();

                Ok(Value::Array(result))
            },
        },
    );

    // @ntnt stringify
    // @module std/csv
    // @signature stringify(rows: Array<Array<Any>>) -> String
    // Converts an array of rows to a CSV string.
    //
    // Values are converted to strings. Fields containing commas, quotes,
    // or newlines are automatically quoted with escaped double-quotes.
    // @param rows Array of arrays, each inner array is a row
    // @returns CSV-formatted string
    // @see_also stringify_with_headers, parse_csv
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example stringify([["a", "b"], [1, 2]]) => "a,b\n1,2" ~ "Array rows to CSV"
    // @error TypeError ~ "csv.stringify() requires an array" fix: "Pass an array of row arrays"
    module.insert(
        "stringify".to_string(),
        Value::NativeFunction {
            name: "csv_stringify".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                let rows = match &args[0] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(IntentError::type_error(
                            "csv.stringify() requires an array".to_string(),
                        ))
                    }
                };

                let delimiter = if args.len() > 1 {
                    match &args[1] {
                        Value::String(d) => d.chars().next().unwrap_or(','),
                        _ => ',',
                    }
                } else {
                    ','
                };

                let mut result = String::new();

                for (i, row_value) in rows.iter().enumerate() {
                    if i > 0 {
                        result.push('\n');
                    }

                    let row = match row_value {
                        Value::Array(arr) => arr,
                        _ => {
                            return Err(IntentError::type_error(
                                "csv.stringify: each row must be an array".to_string(),
                            ))
                        }
                    };

                    let line = row
                        .iter()
                        .map(|v| escape_csv_field(&value_to_string(v), delimiter))
                        .collect::<Vec<_>>()
                        .join(&delimiter.to_string());

                    result.push_str(&line);
                }

                Ok(Value::String(result))
            },
        },
    );

    // @ntnt stringify_with_headers
    // @module std/csv
    // @signature stringify_with_headers(rows: Array<Map>, headers: Array<String>) -> String
    // Converts an array of maps to a CSV string with a header row.
    //
    // The headers array defines both the column order and the header row.
    // Each map row is serialized by looking up the header keys.
    // @param rows Array of maps, one per data row
    // @param headers Array of column names (also used as map keys)
    // @returns CSV string with header row followed by data rows
    // @see_also stringify, parse_with_headers
    // @since v0.2.0
    // @tags #pure, #deterministic
    // @example stringify_with_headers([map { "name": "Alice" }], ["name"]) => "name\nAlice" ~ "Maps to CSV with headers"
    // @error TypeError ~ "csv.stringify_with_headers() first arg must be array" fix: "Pass array of maps"
    module.insert(
        "stringify_with_headers".to_string(),
        Value::NativeFunction {
            name: "csv_stringify_with_headers".to_string(),
            arity: 2,
            max_arity: 2,
            requires: None,
            func: |args| {
                let rows = match &args[0] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(IntentError::type_error(
                            "csv.stringify_with_headers() first arg must be array".to_string(),
                        ))
                    }
                };

                let headers: Vec<String> = match &args[1] {
                    Value::Array(arr) => arr
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => Ok(s.clone()),
                            _ => Err(IntentError::type_error(
                                "Headers must be strings".to_string(),
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => {
                        return Err(IntentError::type_error(
                            "csv.stringify_with_headers() second arg must be headers array"
                                .to_string(),
                        ))
                    }
                };

                let delimiter = if args.len() > 2 {
                    match &args[2] {
                        Value::String(d) => d.chars().next().unwrap_or(','),
                        _ => ',',
                    }
                } else {
                    ','
                };

                let mut result = String::new();

                // Write header row
                let header_line = headers
                    .iter()
                    .map(|h| escape_csv_field(h, delimiter))
                    .collect::<Vec<_>>()
                    .join(&delimiter.to_string());
                result.push_str(&header_line);

                // Write data rows
                for row_value in rows.iter() {
                    result.push('\n');

                    let row = match row_value {
                        Value::Map(m) => m,
                        _ => {
                            return Err(IntentError::type_error(
                                "csv.stringify_with_headers: each row must be a map".to_string(),
                            ))
                        }
                    };

                    let line = headers
                        .iter()
                        .map(|h| {
                            let value = row.get(h).cloned().unwrap_or(Value::String(String::new()));
                            escape_csv_field(&value_to_string(&value), delimiter)
                        })
                        .collect::<Vec<_>>()
                        .join(&delimiter.to_string());

                    result.push_str(&line);
                }

                Ok(Value::String(result))
            },
        },
    );

    module
}

/// Parse CSV string into rows of fields, handling quoted fields
fn parse_csv_string(input: &str, delimiter: char) -> Result<Vec<Vec<String>>, IntentError> {
    let mut rows = Vec::new();
    let mut current_row = Vec::new();
    let mut current_field = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                // Check for escaped quote (double quote)
                if chars.peek() == Some(&'"') {
                    chars.next();
                    current_field.push('"');
                } else {
                    // End of quoted field
                    in_quotes = false;
                }
            } else {
                current_field.push(ch);
            }
        } else if ch == '"' {
            // Start of quoted field
            in_quotes = true;
        } else if ch == delimiter {
            // End of field
            current_row.push(current_field.trim().to_string());
            current_field = String::new();
        } else if ch == '\n' {
            // End of row
            current_row.push(current_field.trim().to_string());
            // Only add non-empty rows
            if !current_row.iter().all(|f| f.is_empty()) || current_row.len() > 1 {
                rows.push(current_row);
            }
            current_row = Vec::new();
            current_field = String::new();
        } else if ch == '\r' {
            // Handle \r\n line endings
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            current_row.push(current_field.trim().to_string());
            if !current_row.iter().all(|f| f.is_empty()) || current_row.len() > 1 {
                rows.push(current_row);
            }
            current_row = Vec::new();
            current_field = String::new();
        } else {
            current_field.push(ch);
        }
    }

    // Don't forget the last field/row
    if !current_field.is_empty() || !current_row.is_empty() {
        current_row.push(current_field.trim().to_string());
        if !current_row.iter().all(|f| f.is_empty()) || current_row.len() > 1 {
            rows.push(current_row);
        }
    }

    Ok(rows)
}

/// Escape a CSV field if it contains special characters
fn escape_csv_field(field: &str, delimiter: char) -> String {
    if field.contains(delimiter)
        || field.contains('"')
        || field.contains('\n')
        || field.contains('\r')
    {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_string()
    }
}

/// Convert Value to string for CSV output
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Unit => String::new(),
        _ => format!("{:?}", v),
    }
}
