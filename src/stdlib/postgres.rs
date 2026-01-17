//! std/db/postgres module - PostgreSQL database driver
//!
//! Provides connection management, queries, and transactions for PostgreSQL.
//!
//! ```intent
//! import { connect, query, execute, transaction } from "std/db/postgres"
//!
//! let db = connect("postgres://user:pass@localhost/mydb")
//! let users = query(db, "SELECT * FROM users WHERE active = $1", [true])
//! ```

use crate::error::IntentError;
use crate::interpreter::Value;
use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use postgres::{types::ToSql, Client, NoTls, Row};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Result<T> = std::result::Result<T, IntentError>;

/// Thread-safe wrapper for PostgreSQL client
/// We use a unique ID to track connections and store them in a global registry
static CONNECTION_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, Arc<Mutex<Client>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CONNECTION_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Enum to hold different SQL parameter types
/// This allows us to properly serialize each type to PostgreSQL
#[derive(Debug)]
enum SqlParam {
    Int(i32),
    BigInt(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
    IntArray(Vec<i32>),
    StringArray(Vec<String>),
}

impl ToSql for SqlParam {
    fn to_sql(
        &self,
        ty: &postgres::types::Type,
        out: &mut postgres::types::private::BytesMut,
    ) -> std::result::Result<postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        match self {
            SqlParam::Int(v) => v.to_sql(ty, out),
            SqlParam::BigInt(v) => v.to_sql(ty, out),
            SqlParam::Float(v) => v.to_sql(ty, out),
            SqlParam::String(v) => v.to_sql(ty, out),
            SqlParam::Bool(v) => v.to_sql(ty, out),
            SqlParam::Null => Ok(postgres::types::IsNull::Yes),
            SqlParam::IntArray(v) => v.to_sql(ty, out),
            SqlParam::StringArray(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(ty: &postgres::types::Type) -> bool {
        <i32 as ToSql>::accepts(ty)
            || <i64 as ToSql>::accepts(ty)
            || <f64 as ToSql>::accepts(ty)
            || <String as ToSql>::accepts(ty)
            || <bool as ToSql>::accepts(ty)
            || <Vec<i32> as ToSql>::accepts(ty)
            || <Vec<String> as ToSql>::accepts(ty)
    }

    postgres::types::to_sql_checked!();
}

/// Convert an Intent Value to a SQL parameter
fn value_to_sql_param(value: &Value) -> SqlParam {
    match value {
        Value::Int(i) => {
            // Use i32 for smaller values, i64 for larger
            if *i >= i32::MIN as i64 && *i <= i32::MAX as i64 {
                SqlParam::Int(*i as i32)
            } else {
                SqlParam::BigInt(*i)
            }
        }
        Value::Float(f) => SqlParam::Float(*f),
        Value::String(s) => SqlParam::String(s.clone()),
        Value::Bool(b) => SqlParam::Bool(*b),
        Value::Unit => SqlParam::Null,
        Value::Array(arr) => {
            // Determine array type from first element
            if arr.is_empty() {
                SqlParam::StringArray(vec![])
            } else {
                match &arr[0] {
                    Value::Int(_) => {
                        let ints: Vec<i32> = arr
                            .iter()
                            .filter_map(|v| {
                                if let Value::Int(i) = v {
                                    Some(*i as i32)
                                } else {
                                    None
                                }
                            })
                            .collect();
                        SqlParam::IntArray(ints)
                    }
                    _ => {
                        let strings: Vec<String> = arr
                            .iter()
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                _ => format!("{}", v),
                            })
                            .collect();
                        SqlParam::StringArray(strings)
                    }
                }
            }
        }
        _ => SqlParam::String(format!("{}", value)), // Fallback to string representation
    }
}

/// Convert a PostgreSQL row to an Intent Map
fn row_to_value(row: &Row) -> Value {
    let mut map = HashMap::new();

    for (idx, column) in row.columns().iter().enumerate() {
        let name = column.name().to_string();
        let value = postgres_value_to_intent(row, idx, column.type_());
        map.insert(name, value);
    }

    Value::Map(map)
}

/// Convert a PostgreSQL value to an Intent Value based on type
fn postgres_value_to_intent(row: &Row, idx: usize, pg_type: &postgres::types::Type) -> Value {
    use postgres::types::Type;

    match *pg_type {
        // Boolean
        Type::BOOL => match row.try_get::<_, Option<bool>>(idx) {
            Ok(Some(v)) => Value::Bool(v),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Integers
        Type::INT2 => match row.try_get::<_, Option<i16>>(idx) {
            Ok(Some(v)) => Value::Int(v as i64),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },
        Type::INT4 => match row.try_get::<_, Option<i32>>(idx) {
            Ok(Some(v)) => Value::Int(v as i64),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },
        Type::INT8 => match row.try_get::<_, Option<i64>>(idx) {
            Ok(Some(v)) => Value::Int(v),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Floats
        Type::FLOAT4 => match row.try_get::<_, Option<f32>>(idx) {
            Ok(Some(v)) => Value::Float(v as f64),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },
        Type::FLOAT8 => match row.try_get::<_, Option<f64>>(idx) {
            Ok(Some(v)) => Value::Float(v),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // NUMERIC/DECIMAL - proper handling via rust_decimal
        Type::NUMERIC => {
            match row.try_get::<_, Option<Decimal>>(idx) {
                Ok(Some(v)) => {
                    // Convert Decimal to f64 for NTNT
                    use rust_decimal::prelude::ToPrimitive;
                    match v.to_f64() {
                        Some(f) => Value::Float(f),
                        None => Value::String(v.to_string()),
                    }
                }
                Ok(None) => Value::Unit,
                Err(_) => Value::Unit,
            }
        }

        // Strings
        Type::VARCHAR | Type::TEXT | Type::BPCHAR | Type::NAME => {
            match row.try_get::<_, Option<String>>(idx) {
                Ok(Some(v)) => Value::String(v),
                Ok(None) => Value::Unit,
                Err(_) => Value::Unit,
            }
        }

        // JSON/JSONB
        Type::JSON | Type::JSONB => match row.try_get::<_, Option<serde_json::Value>>(idx) {
            Ok(Some(v)) => json_to_intent_value(&v),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Date
        Type::DATE => match row.try_get::<_, Option<NaiveDate>>(idx) {
            Ok(Some(v)) => Value::String(v.format("%Y-%m-%d").to_string()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Time
        Type::TIME => match row.try_get::<_, Option<NaiveTime>>(idx) {
            Ok(Some(v)) => Value::String(v.format("%H:%M:%S").to_string()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Timestamp without timezone
        Type::TIMESTAMP => match row.try_get::<_, Option<NaiveDateTime>>(idx) {
            Ok(Some(v)) => Value::String(v.format("%Y-%m-%dT%H:%M:%S").to_string()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Timestamp with timezone
        Type::TIMESTAMPTZ => match row.try_get::<_, Option<DateTime<Utc>>>(idx) {
            Ok(Some(v)) => Value::String(v.to_rfc3339()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // UUID
        Type::UUID => match row.try_get::<_, Option<uuid::Uuid>>(idx) {
            Ok(Some(v)) => Value::String(v.to_string()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Integer arrays
        Type::INT4_ARRAY => match row.try_get::<_, Option<Vec<i32>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(|i| Value::Int(i as i64)).collect()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },
        Type::INT8_ARRAY => match row.try_get::<_, Option<Vec<i64>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::Int).collect()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // String arrays
        Type::TEXT_ARRAY | Type::VARCHAR_ARRAY => {
            match row.try_get::<_, Option<Vec<String>>>(idx) {
                Ok(Some(v)) => Value::Array(v.into_iter().map(Value::String).collect()),
                Ok(None) => Value::Unit,
                Err(_) => Value::Unit,
            }
        }

        // Float arrays
        Type::FLOAT8_ARRAY => match row.try_get::<_, Option<Vec<f64>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::Float).collect()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Boolean arrays
        Type::BOOL_ARRAY => match row.try_get::<_, Option<Vec<bool>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::Bool).collect()),
            Ok(None) => Value::Unit,
            Err(_) => Value::Unit,
        },

        // Fallback for unknown types
        _ => {
            // Try common types in order of likelihood
            if let Ok(Some(v)) = row.try_get::<_, Option<String>>(idx) {
                return Value::String(v);
            }
            if let Ok(Some(v)) = row.try_get::<_, Option<i64>>(idx) {
                return Value::Int(v);
            }
            if let Ok(Some(v)) = row.try_get::<_, Option<f64>>(idx) {
                return Value::Float(v);
            }
            if let Ok(Some(v)) = row.try_get::<_, Option<bool>>(idx) {
                return Value::Bool(v);
            }
            Value::Unit
        }
    }
}

/// Convert serde_json::Value to Intent Value (for JSONB support)
fn json_to_intent_value(json: &serde_json::Value) -> Value {
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

/// Connect to a PostgreSQL database
fn pg_connect(connection_string: &str) -> Result<Value> {
    match Client::connect(connection_string, NoTls) {
        Ok(client) => {
            let id = CONNECTION_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let wrapped = Arc::new(Mutex::new(client));

            // Store in registry
            if let Ok(mut registry) = CONNECTION_REGISTRY.lock() {
                registry.insert(id, wrapped);
            }

            // Return a connection handle as a map
            let mut handle = HashMap::new();
            handle.insert("_pg_connection_id".to_string(), Value::Int(id as i64));
            handle.insert("connected".to_string(), Value::Bool(true));

            Ok(Value::EnumValue {
                enum_name: "Result".to_string(),
                variant: "Ok".to_string(),
                values: vec![Value::Map(handle)],
            })
        }
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("Connection failed: {}", e))],
        }),
    }
}

/// Get a client from the connection handle
fn get_client(conn: &Value) -> Result<Arc<Mutex<Client>>> {
    match conn {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_pg_connection_id") {
                if let Ok(registry) = CONNECTION_REGISTRY.lock() {
                    if let Some(client) = registry.get(&(*id as u64)) {
                        return Ok(Arc::clone(client));
                    }
                }
                Err(IntentError::RuntimeError(
                    "Invalid or closed database connection".to_string(),
                ))
            } else {
                Err(IntentError::TypeError(
                    "Expected a database connection handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::TypeError(
            "Expected a database connection handle".to_string(),
        )),
    }
}

/// Execute a query and return rows
fn pg_query(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let client_arc = get_client(conn)?;
    let mut client = client_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    // Convert params to SqlParam
    let sql_params: Vec<SqlParam> = params.iter().map(value_to_sql_param).collect();

    // Create references for the query
    let param_refs: Vec<&(dyn ToSql + Sync)> = sql_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    match client.query(sql, &param_refs) {
        Ok(rows) => {
            let result: Vec<Value> = rows.iter().map(row_to_value).collect();
            Ok(Value::EnumValue {
                enum_name: "Result".to_string(),
                variant: "Ok".to_string(),
                values: vec![Value::Array(result)],
            })
        }
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("Query failed: {}", e))],
        }),
    }
}

/// Execute a query and return a single row (or null)
fn pg_query_one(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let client_arc = get_client(conn)?;
    let mut client = client_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    let sql_params: Vec<SqlParam> = params.iter().map(value_to_sql_param).collect();
    let param_refs: Vec<&(dyn ToSql + Sync)> = sql_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    match client.query_opt(sql, &param_refs) {
        Ok(Some(row)) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Ok".to_string(),
            values: vec![row_to_value(&row)],
        }),
        Ok(None) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Ok".to_string(),
            values: vec![Value::Unit],
        }),
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("Query failed: {}", e))],
        }),
    }
}

/// Execute a statement (INSERT/UPDATE/DELETE) and return affected row count
fn pg_execute(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let client_arc = get_client(conn)?;
    let mut client = client_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    let sql_params: Vec<SqlParam> = params.iter().map(value_to_sql_param).collect();
    let param_refs: Vec<&(dyn ToSql + Sync)> = sql_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    match client.execute(sql, &param_refs) {
        Ok(count) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Ok".to_string(),
            values: vec![Value::Int(count as i64)],
        }),
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("Execute failed: {}", e))],
        }),
    }
}

/// Close a database connection
fn pg_close(conn: &Value) -> Result<Value> {
    match conn {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_pg_connection_id") {
                if let Ok(mut registry) = CONNECTION_REGISTRY.lock() {
                    registry.remove(&(*id as u64));
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        _ => Err(IntentError::TypeError(
            "Expected a database connection handle".to_string(),
        )),
    }
}

/// Begin a transaction - returns a transaction handle
fn pg_begin(conn: &Value) -> Result<Value> {
    let client_arc = get_client(conn)?;
    let mut client = client_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    match client.execute("BEGIN", &[]) {
        Ok(_) => {
            // Return the same connection handle (transaction is implicit)
            Ok(Value::EnumValue {
                enum_name: "Result".to_string(),
                variant: "Ok".to_string(),
                values: vec![conn.clone()],
            })
        }
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("BEGIN failed: {}", e))],
        }),
    }
}

/// Commit a transaction
fn pg_commit(conn: &Value) -> Result<Value> {
    let client_arc = get_client(conn)?;
    let mut client = client_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    match client.execute("COMMIT", &[]) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("COMMIT failed: {}", e))],
        }),
    }
}

/// Rollback a transaction
fn pg_rollback(conn: &Value) -> Result<Value> {
    let client_arc = get_client(conn)?;
    let mut client = client_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    match client.execute("ROLLBACK", &[]) {
        Ok(_) => Ok(Value::Bool(true)),
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("ROLLBACK failed: {}", e))],
        }),
    }
}

/// Initialize the std/db/postgres module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // connect(connection_string) -> Result<Connection, Error>
    module.insert(
        "connect".to_string(),
        Value::NativeFunction {
            name: "connect".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(conn_str) => pg_connect(conn_str),
                _ => Err(IntentError::TypeError(
                    "connect() requires a connection string".to_string(),
                )),
            },
        },
    );

    // query(conn, sql, params?) -> Result<Array<Row>, Error>
    module.insert(
        "query".to_string(),
        Value::NativeFunction {
            name: "query".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => pg_query(conn, sql, params),
                (conn, Value::String(sql), Value::Unit) => pg_query(conn, sql, &[]),
                _ => Err(IntentError::TypeError(
                    "query() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // query_one(conn, sql, params?) -> Result<Row | null, Error>
    module.insert(
        "query_one".to_string(),
        Value::NativeFunction {
            name: "query_one".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => pg_query_one(conn, sql, params),
                (conn, Value::String(sql), Value::Unit) => pg_query_one(conn, sql, &[]),
                _ => Err(IntentError::TypeError(
                    "query_one() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // execute(conn, sql, params?) -> Result<int, Error> (returns affected row count)
    module.insert(
        "execute".to_string(),
        Value::NativeFunction {
            name: "execute".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => pg_execute(conn, sql, params),
                (conn, Value::String(sql), Value::Unit) => pg_execute(conn, sql, &[]),
                _ => Err(IntentError::TypeError(
                    "execute() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // close(conn) -> bool
    module.insert(
        "close".to_string(),
        Value::NativeFunction {
            name: "close".to_string(),
            arity: 1,
            func: |args| pg_close(&args[0]),
        },
    );

    // begin(conn) -> Result<Connection, Error> (starts transaction)
    module.insert(
        "begin".to_string(),
        Value::NativeFunction {
            name: "begin".to_string(),
            arity: 1,
            func: |args| pg_begin(&args[0]),
        },
    );

    // commit(conn) -> Result<bool, Error>
    module.insert(
        "commit".to_string(),
        Value::NativeFunction {
            name: "commit".to_string(),
            arity: 1,
            func: |args| pg_commit(&args[0]),
        },
    );

    // rollback(conn) -> Result<bool, Error>
    module.insert(
        "rollback".to_string(),
        Value::NativeFunction {
            name: "rollback".to_string(),
            arity: 1,
            func: |args| pg_rollback(&args[0]),
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_init() {
        let module = init();
        assert!(module.contains_key("connect"));
        assert!(module.contains_key("query"));
        assert!(module.contains_key("query_one"));
        assert!(module.contains_key("execute"));
        assert!(module.contains_key("close"));
        assert!(module.contains_key("begin"));
        assert!(module.contains_key("commit"));
        assert!(module.contains_key("rollback"));
    }

    #[test]
    fn test_json_to_intent_value() {
        let json = serde_json::json!({
            "name": "test",
            "count": 42,
            "active": true,
            "tags": ["a", "b"]
        });

        let value = json_to_intent_value(&json);
        match value {
            Value::Map(map) => {
                match map.get("name") {
                    Some(Value::String(s)) => assert_eq!(s, "test"),
                    _ => panic!("Expected name to be String"),
                }
                match map.get("count") {
                    Some(Value::Int(i)) => assert_eq!(*i, 42),
                    _ => panic!("Expected count to be Int"),
                }
                match map.get("active") {
                    Some(Value::Bool(b)) => assert!(*b),
                    _ => panic!("Expected active to be Bool"),
                }
                match map.get("tags") {
                    Some(Value::Array(arr)) => assert_eq!(arr.len(), 2),
                    _ => panic!("Expected tags to be Array"),
                }
            }
            _ => panic!("Expected Map"),
        }
    }

    #[test]
    fn test_value_to_sql_param() {
        // Test integer conversion
        let param = value_to_sql_param(&Value::Int(42));
        match param {
            SqlParam::Int(v) => assert_eq!(v, 42),
            _ => panic!("Expected Int"),
        }

        // Test boolean conversion
        let param = value_to_sql_param(&Value::Bool(true));
        match param {
            SqlParam::Bool(v) => assert!(v),
            _ => panic!("Expected Bool"),
        }

        // Test string conversion
        let param = value_to_sql_param(&Value::String("test".to_string()));
        match param {
            SqlParam::String(v) => assert_eq!(v, "test"),
            _ => panic!("Expected String"),
        }
    }
}
