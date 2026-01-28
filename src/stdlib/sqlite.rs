//! std/db/sqlite module - SQLite database driver
//!
//! Provides connection management, queries, and transactions for SQLite.
//! SQLite is bundled — no external dependencies needed.
//!
//! ```intent
//! import { connect, query, execute, close } from "std/db/sqlite"
//!
//! let db = connect("app.db")        // File-based database
//! let db = connect(":memory:")      // In-memory database
//! execute(db, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])
//! execute(db, "INSERT INTO users (name) VALUES (?)", ["Alice"])
//! let rows = query(db, "SELECT * FROM users", [])
//! close(db)
//! ```

use crate::error::IntentError;
use crate::interpreter::Value;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

type Result<T> = std::result::Result<T, IntentError>;

/// Thread-safe connection registry (same pattern as postgres.rs)
static CONNECTION_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, Arc<Mutex<Connection>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));
static CONNECTION_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Convert an Intent Value to a rusqlite Value
fn value_to_sqlite(value: &Value) -> rusqlite::types::Value {
    match value {
        Value::Int(i) => rusqlite::types::Value::Integer(*i),
        Value::Float(f) => rusqlite::types::Value::Real(*f),
        Value::String(s) => rusqlite::types::Value::Text(s.clone()),
        Value::Bool(b) => rusqlite::types::Value::Integer(if *b { 1 } else { 0 }),
        Value::Unit => rusqlite::types::Value::Null,
        _ => rusqlite::types::Value::Text(format!("{}", value)),
    }
}

/// Convert a SQLite ValueRef to an Intent Value
fn sqlite_to_value(val: ValueRef) -> Value {
    match val {
        ValueRef::Null => Value::Unit,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).to_string()),
        ValueRef::Blob(b) => Value::Array(b.iter().map(|byte| Value::Int(*byte as i64)).collect()),
    }
}

/// Connect to a SQLite database
fn sqlite_connect(path: &str) -> Result<Value> {
    let result = if path == ":memory:" {
        Connection::open_in_memory()
    } else {
        Connection::open(path)
    };

    match result {
        Ok(conn) => {
            // Enable WAL mode for better concurrent read performance
            let _ = conn.execute_batch("PRAGMA journal_mode=WAL;");
            // Enable foreign keys (off by default in SQLite)
            let _ = conn.execute_batch("PRAGMA foreign_keys=ON;");

            let id = CONNECTION_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let wrapped = Arc::new(Mutex::new(conn));

            if let Ok(mut registry) = CONNECTION_REGISTRY.lock() {
                registry.insert(id, wrapped);
            }

            let mut handle = HashMap::new();
            handle.insert("_sqlite_connection_id".to_string(), Value::Int(id as i64));
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

/// Get a connection from the registry by handle
fn get_connection(conn: &Value) -> Result<Arc<Mutex<Connection>>> {
    match conn {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_sqlite_connection_id") {
                if let Ok(registry) = CONNECTION_REGISTRY.lock() {
                    if let Some(client) = registry.get(&(*id as u64)) {
                        return Ok(Arc::clone(client));
                    }
                }
                Err(IntentError::RuntimeError(
                    "Invalid or closed SQLite connection".to_string(),
                ))
            } else {
                Err(IntentError::TypeError(
                    "Expected a SQLite connection handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::TypeError(
            "Expected a SQLite connection handle".to_string(),
        )),
    }
}

/// Execute a SELECT query and return all rows
fn sqlite_query(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let conn_arc = get_connection(conn)?;
    let conn_guard = conn_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    let sqlite_params: Vec<rusqlite::types::Value> = params.iter().map(value_to_sqlite).collect();

    let mut stmt = conn_guard
        .prepare(sql)
        .map_err(|e| IntentError::RuntimeError(format!("Query preparation failed: {}", e)))?;

    let column_count = stmt.column_count();
    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    let rows_result = stmt.query_map(rusqlite::params_from_iter(sqlite_params.iter()), |row| {
        let mut map = HashMap::new();
        for idx in 0..column_count {
            let name = column_names[idx].clone();
            let value = sqlite_to_value(row.get_ref(idx)?);
            map.insert(name, value);
        }
        Ok(Value::Map(map))
    });

    match rows_result {
        Ok(rows) => {
            let mut result = Vec::new();
            for row in rows {
                match row {
                    Ok(v) => result.push(v),
                    Err(e) => {
                        return Ok(Value::EnumValue {
                            enum_name: "Result".to_string(),
                            variant: "Err".to_string(),
                            values: vec![Value::String(format!("Row error: {}", e))],
                        })
                    }
                }
            }
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

/// Execute a SELECT query and return a single row (or Unit if no match)
fn sqlite_query_one(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let conn_arc = get_connection(conn)?;
    let conn_guard = conn_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    let sqlite_params: Vec<rusqlite::types::Value> = params.iter().map(value_to_sqlite).collect();

    let mut stmt = conn_guard
        .prepare(sql)
        .map_err(|e| IntentError::RuntimeError(format!("Query preparation failed: {}", e)))?;

    let column_count = stmt.column_count();
    let column_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

    match stmt.query_row(rusqlite::params_from_iter(sqlite_params.iter()), |row| {
        let mut map = HashMap::new();
        for idx in 0..column_count {
            let name = column_names[idx].clone();
            let value = sqlite_to_value(row.get_ref(idx)?);
            map.insert(name, value);
        }
        Ok(Value::Map(map))
    }) {
        Ok(row) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Ok".to_string(),
            values: vec![row],
        }),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Value::EnumValue {
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
fn sqlite_execute(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let conn_arc = get_connection(conn)?;
    let conn_guard = conn_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    let sqlite_params: Vec<rusqlite::types::Value> = params.iter().map(value_to_sqlite).collect();

    match conn_guard.execute(sql, rusqlite::params_from_iter(sqlite_params.iter())) {
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
fn sqlite_close(conn: &Value) -> Result<Value> {
    match conn {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_sqlite_connection_id") {
                if let Ok(mut registry) = CONNECTION_REGISTRY.lock() {
                    registry.remove(&(*id as u64));
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        _ => Err(IntentError::TypeError(
            "Expected a SQLite connection handle".to_string(),
        )),
    }
}

/// Begin a transaction
fn sqlite_begin(conn: &Value) -> Result<Value> {
    let conn_arc = get_connection(conn)?;
    let conn_guard = conn_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    match conn_guard.execute_batch("BEGIN") {
        Ok(_) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Ok".to_string(),
            values: vec![conn.clone()],
        }),
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("BEGIN failed: {}", e))],
        }),
    }
}

/// Commit a transaction
fn sqlite_commit(conn: &Value) -> Result<Value> {
    let conn_arc = get_connection(conn)?;
    let conn_guard = conn_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    match conn_guard.execute_batch("COMMIT") {
        Ok(_) => Ok(Value::Bool(true)),
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("COMMIT failed: {}", e))],
        }),
    }
}

/// Rollback a transaction
fn sqlite_rollback(conn: &Value) -> Result<Value> {
    let conn_arc = get_connection(conn)?;
    let conn_guard = conn_arc
        .lock()
        .map_err(|e| IntentError::RuntimeError(format!("Failed to lock connection: {}", e)))?;

    match conn_guard.execute_batch("ROLLBACK") {
        Ok(_) => Ok(Value::Bool(true)),
        Err(e) => Ok(Value::EnumValue {
            enum_name: "Result".to_string(),
            variant: "Err".to_string(),
            values: vec![Value::String(format!("ROLLBACK failed: {}", e))],
        }),
    }
}

/// Initialize the std/db/sqlite module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // connect(path) -> Result<Connection, Error>
    module.insert(
        "connect".to_string(),
        Value::NativeFunction {
            name: "connect".to_string(),
            arity: 1,
            func: |args| match &args[0] {
                Value::String(path) => sqlite_connect(path),
                _ => Err(IntentError::TypeError(
                    "connect() requires a database path string".to_string(),
                )),
            },
        },
    );

    // query(conn, sql, params) -> Result<Array<Row>, Error>
    module.insert(
        "query".to_string(),
        Value::NativeFunction {
            name: "query".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => sqlite_query(conn, sql, params),
                (conn, Value::String(sql), Value::Unit) => sqlite_query(conn, sql, &[]),
                _ => Err(IntentError::TypeError(
                    "query() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // query_one(conn, sql, params) -> Result<Row | Unit, Error>
    module.insert(
        "query_one".to_string(),
        Value::NativeFunction {
            name: "query_one".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => {
                    sqlite_query_one(conn, sql, params)
                }
                (conn, Value::String(sql), Value::Unit) => sqlite_query_one(conn, sql, &[]),
                _ => Err(IntentError::TypeError(
                    "query_one() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // execute(conn, sql, params) -> Result<Int, Error>
    module.insert(
        "execute".to_string(),
        Value::NativeFunction {
            name: "execute".to_string(),
            arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => {
                    sqlite_execute(conn, sql, params)
                }
                (conn, Value::String(sql), Value::Unit) => sqlite_execute(conn, sql, &[]),
                _ => Err(IntentError::TypeError(
                    "execute() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // close(conn) -> Bool
    module.insert(
        "close".to_string(),
        Value::NativeFunction {
            name: "close".to_string(),
            arity: 1,
            func: |args| sqlite_close(&args[0]),
        },
    );

    // begin(conn) -> Result<Connection, Error>
    module.insert(
        "begin".to_string(),
        Value::NativeFunction {
            name: "begin".to_string(),
            arity: 1,
            func: |args| sqlite_begin(&args[0]),
        },
    );

    // commit(conn) -> Result<Bool, Error>
    module.insert(
        "commit".to_string(),
        Value::NativeFunction {
            name: "commit".to_string(),
            arity: 1,
            func: |args| sqlite_commit(&args[0]),
        },
    );

    // rollback(conn) -> Result<Bool, Error>
    module.insert(
        "rollback".to_string(),
        Value::NativeFunction {
            name: "rollback".to_string(),
            arity: 1,
            func: |args| sqlite_rollback(&args[0]),
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
    fn test_in_memory_connect() {
        let result = sqlite_connect(":memory:");
        assert!(result.is_ok());
        let value = result.unwrap();
        match &value {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                match &values[0] {
                    Value::Map(map) => {
                        assert!(map.contains_key("_sqlite_connection_id"));
                        match map.get("connected") {
                            Some(Value::Bool(true)) => {}
                            _ => panic!("Expected connected=true"),
                        }
                    }
                    _ => panic!("Expected Map in Ok variant"),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        // Clean up
        if let Value::EnumValue { values, .. } = value {
            sqlite_close(&values[0]).unwrap();
        }
    }

    #[test]
    fn test_create_table_and_insert() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        // Create table
        let create_result = sqlite_execute(
            &conn,
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, score REAL)",
            &[],
        );
        assert!(create_result.is_ok());

        // Insert a row
        let insert_result = sqlite_execute(
            &conn,
            "INSERT INTO test (name, score) VALUES (?, ?)",
            &[Value::String("Alice".to_string()), Value::Float(95.5)],
        );
        match insert_result.unwrap() {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                match &values[0] {
                    Value::Int(1) => {}
                    other => panic!("Expected Int(1), got {:?}", other),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        sqlite_close(&conn).unwrap();
    }

    #[test]
    fn test_query_returns_rows() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        sqlite_execute(
            &conn,
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)",
            &[],
        )
        .unwrap();

        sqlite_execute(
            &conn,
            "INSERT INTO test (name) VALUES (?)",
            &[Value::String("Alice".to_string())],
        )
        .unwrap();

        sqlite_execute(
            &conn,
            "INSERT INTO test (name) VALUES (?)",
            &[Value::String("Bob".to_string())],
        )
        .unwrap();

        let result = sqlite_query(&conn, "SELECT * FROM test ORDER BY id", &[]).unwrap();
        match result {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                match &values[0] {
                    Value::Array(rows) => {
                        assert_eq!(rows.len(), 2);
                        // Check first row
                        match &rows[0] {
                            Value::Map(map) => {
                                match map.get("name") {
                                    Some(Value::String(s)) => assert_eq!(s, "Alice"),
                                    other => panic!("Expected name=Alice, got {:?}", other),
                                }
                                match map.get("id") {
                                    Some(Value::Int(1)) => {}
                                    other => panic!("Expected id=1, got {:?}", other),
                                }
                            }
                            _ => panic!("Expected Map row"),
                        }
                    }
                    _ => panic!("Expected Array"),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        sqlite_close(&conn).unwrap();
    }

    #[test]
    fn test_query_one() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        sqlite_execute(
            &conn,
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)",
            &[],
        )
        .unwrap();

        sqlite_execute(
            &conn,
            "INSERT INTO test (name) VALUES (?)",
            &[Value::String("Alice".to_string())],
        )
        .unwrap();

        // Query existing row
        let result =
            sqlite_query_one(&conn, "SELECT * FROM test WHERE id = ?", &[Value::Int(1)]).unwrap();
        match &result {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                match &values[0] {
                    Value::Map(map) => match map.get("name") {
                        Some(Value::String(s)) => assert_eq!(s, "Alice"),
                        other => panic!("Expected name=Alice, got {:?}", other),
                    },
                    _ => panic!("Expected Map"),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        // Query non-existing row returns Unit
        let result =
            sqlite_query_one(&conn, "SELECT * FROM test WHERE id = ?", &[Value::Int(999)]).unwrap();
        match result {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                match &values[0] {
                    Value::Unit => {}
                    other => panic!("Expected Unit, got {:?}", other),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        sqlite_close(&conn).unwrap();
    }

    #[test]
    fn test_parameterized_query() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        sqlite_execute(
            &conn,
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT, active INTEGER)",
            &[],
        )
        .unwrap();

        sqlite_execute(
            &conn,
            "INSERT INTO test (name, active) VALUES (?, ?)",
            &[Value::String("Alice".to_string()), Value::Bool(true)],
        )
        .unwrap();

        sqlite_execute(
            &conn,
            "INSERT INTO test (name, active) VALUES (?, ?)",
            &[Value::String("Bob".to_string()), Value::Bool(false)],
        )
        .unwrap();

        // Query with parameter
        let result = sqlite_query(
            &conn,
            "SELECT * FROM test WHERE active = ?",
            &[Value::Bool(true)],
        )
        .unwrap();
        match result {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                match &values[0] {
                    Value::Array(rows) => assert_eq!(rows.len(), 1),
                    _ => panic!("Expected Array"),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        sqlite_close(&conn).unwrap();
    }

    #[test]
    fn test_transaction() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        sqlite_execute(
            &conn,
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)",
            &[],
        )
        .unwrap();

        // Begin transaction
        let tx_result = sqlite_begin(&conn).unwrap();
        match &tx_result {
            Value::EnumValue { variant, .. } => assert_eq!(variant, "Ok"),
            _ => panic!("Expected EnumValue"),
        }

        // Insert within transaction
        sqlite_execute(
            &conn,
            "INSERT INTO test (name) VALUES (?)",
            &[Value::String("Alice".to_string())],
        )
        .unwrap();

        // Rollback
        let rollback_result = sqlite_rollback(&conn).unwrap();
        match rollback_result {
            Value::Bool(true) => {}
            other => panic!("Expected Bool(true), got {:?}", other),
        }

        // Verify row was rolled back
        let result = sqlite_query(&conn, "SELECT * FROM test", &[]).unwrap();
        match result {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant.as_str(), "Ok");
                match &values[0] {
                    Value::Array(rows) => {
                        assert_eq!(rows.len(), 0, "Rollback should remove inserted row")
                    }
                    _ => panic!("Expected Array"),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        // Now test commit
        sqlite_begin(&conn).unwrap();
        sqlite_execute(
            &conn,
            "INSERT INTO test (name) VALUES (?)",
            &[Value::String("Bob".to_string())],
        )
        .unwrap();
        sqlite_commit(&conn).unwrap();

        // Verify row persists
        let result = sqlite_query(&conn, "SELECT * FROM test", &[]).unwrap();
        match result {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant.as_str(), "Ok");
                match &values[0] {
                    Value::Array(rows) => assert_eq!(rows.len(), 1, "Committed row should persist"),
                    _ => panic!("Expected Array"),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        sqlite_close(&conn).unwrap();
    }

    #[test]
    fn test_null_handling() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        sqlite_execute(
            &conn,
            "CREATE TABLE test (id INTEGER PRIMARY KEY, name TEXT)",
            &[],
        )
        .unwrap();

        // Insert with NULL
        sqlite_execute(&conn, "INSERT INTO test (name) VALUES (?)", &[Value::Unit]).unwrap();

        let result = sqlite_query(&conn, "SELECT * FROM test", &[]).unwrap();
        match result {
            Value::EnumValue {
                variant, values, ..
            } => {
                assert_eq!(variant, "Ok");
                match &values[0] {
                    Value::Array(rows) => match &rows[0] {
                        Value::Map(map) => match map.get("name") {
                            Some(Value::Unit) => {}
                            other => panic!("Expected name=Unit, got {:?}", other),
                        },
                        _ => panic!("Expected Map"),
                    },
                    _ => panic!("Expected Array"),
                }
            }
            _ => panic!("Expected EnumValue"),
        }

        sqlite_close(&conn).unwrap();
    }

    #[test]
    fn test_close_connection() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        let result = sqlite_close(&conn).unwrap();
        match result {
            Value::Bool(true) => {}
            other => panic!("Expected Bool(true), got {:?}", other),
        }

        // Using closed connection should fail
        let query_result = sqlite_query(&conn, "SELECT 1", &[]);
        assert!(query_result.is_err());
    }

    #[test]
    fn test_value_to_sqlite_conversion() {
        match value_to_sqlite(&Value::Int(42)) {
            rusqlite::types::Value::Integer(v) => assert_eq!(v, 42),
            _ => panic!("Expected Integer"),
        }
        match value_to_sqlite(&Value::Float(3.14)) {
            rusqlite::types::Value::Real(v) => assert!((v - 3.14).abs() < f64::EPSILON),
            _ => panic!("Expected Real"),
        }
        match value_to_sqlite(&Value::String("hello".to_string())) {
            rusqlite::types::Value::Text(v) => assert_eq!(v, "hello"),
            _ => panic!("Expected Text"),
        }
        match value_to_sqlite(&Value::Bool(true)) {
            rusqlite::types::Value::Integer(v) => assert_eq!(v, 1),
            _ => panic!("Expected Integer for true"),
        }
        match value_to_sqlite(&Value::Bool(false)) {
            rusqlite::types::Value::Integer(v) => assert_eq!(v, 0),
            _ => panic!("Expected Integer for false"),
        }
        match value_to_sqlite(&Value::Unit) {
            rusqlite::types::Value::Null => {}
            _ => panic!("Expected Null"),
        }
    }

    #[test]
    fn test_error_on_invalid_sql() {
        let conn = match sqlite_connect(":memory:").unwrap() {
            Value::EnumValue { values, .. } => values.into_iter().next().unwrap(),
            _ => panic!("Expected EnumValue"),
        };

        let result = sqlite_execute(&conn, "INVALID SQL", &[]).unwrap();
        match result {
            Value::EnumValue { variant, .. } => assert_eq!(variant, "Err"),
            _ => panic!("Expected EnumValue"),
        }

        sqlite_close(&conn).unwrap();
    }
}
