//! std/csv module - CSV parsing and stringification

use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;

/// Initialize the std/csv module
pub fn init() -> HashMap<String, Value> {
    let mut module: HashMap<String, Value> = HashMap::new();

    // parse(csv_str, delimiter?) -> Array of Arrays
    // parse("a,b,c\n1,2,3") -> [["a","b","c"], ["1","2","3"]]
    module.insert(
        "parse".to_string(),
        Value::NativeFunction {
            name: "csv_parse".to_string(),
            arity: 1,
            func: |args| {
                let csv_string = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "csv.parse() requires a string".to_string(),
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

    // parse_with_headers(csv_str, delimiter?) -> Array of Maps
    // parse_with_headers("name,age\nAlice,30") -> [{"name": "Alice", "age": "30"}]
    module.insert(
        "parse_with_headers".to_string(),
        Value::NativeFunction {
            name: "csv_parse_with_headers".to_string(),
            arity: 1,
            func: |args| {
                let csv_string = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
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

    // stringify(rows, delimiter?) -> String
    // stringify([["a","b"], ["1","2"]]) -> "a,b\n1,2"
    module.insert(
        "stringify".to_string(),
        Value::NativeFunction {
            name: "csv_stringify".to_string(),
            arity: 1,
            func: |args| {
                let rows = match &args[0] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(IntentError::TypeError(
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
                            return Err(IntentError::TypeError(
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

    // stringify_with_headers(data, headers, delimiter?) -> String
    // stringify_with_headers([{"name": "Alice"}], ["name"]) -> "name\nAlice"
    module.insert(
        "stringify_with_headers".to_string(),
        Value::NativeFunction {
            name: "csv_stringify_with_headers".to_string(),
            arity: 2,
            func: |args| {
                let rows = match &args[0] {
                    Value::Array(arr) => arr,
                    _ => {
                        return Err(IntentError::TypeError(
                            "csv.stringify_with_headers() first arg must be array".to_string(),
                        ))
                    }
                };

                let headers: Vec<String> = match &args[1] {
                    Value::Array(arr) => arr
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => Ok(s.clone()),
                            _ => Err(IntentError::TypeError(
                                "Headers must be strings".to_string(),
                            )),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    _ => {
                        return Err(IntentError::TypeError(
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
                            return Err(IntentError::TypeError(
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
