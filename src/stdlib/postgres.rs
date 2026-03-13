//! std/db/postgres module - PostgreSQL database driver
//!
//! Provides connection pooling, queries, and transactions for PostgreSQL.
//! Uses deadpool-postgres for async connection pooling with a dedicated Tokio runtime.
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
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio_postgres::types::ToSql;
use tokio_postgres::NoTls;

type Result<T> = std::result::Result<T, IntentError>;

/// Dedicated Tokio runtime for all database operations.
/// The interpreter thread is not inside a Tokio runtime, so we need our own.
static DB_RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> = std::sync::LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("Failed to create DB runtime")
});

/// Pool registry — maps connection IDs to deadpool-postgres pools
static POOL_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, Pool>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Transaction registry — maps connection handle IDs to dedicated client objects.
/// Transactions must pin to a single connection, so we check out a client
/// at BEGIN and hold it until COMMIT/ROLLBACK.
static TXN_REGISTRY: std::sync::LazyLock<
    Mutex<HashMap<u64, std::sync::Arc<tokio::sync::Mutex<deadpool_postgres::Client>>>>,
> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

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
        ty: &tokio_postgres::types::Type,
        out: &mut tokio_postgres::types::private::BytesMut,
    ) -> std::result::Result<tokio_postgres::types::IsNull, Box<dyn std::error::Error + Sync + Send>>
    {
        use tokio_postgres::types::Type;
        match self {
            SqlParam::Int(v) => v.to_sql(ty, out),
            SqlParam::BigInt(v) => v.to_sql(ty, out),
            SqlParam::Float(v) => v.to_sql(ty, out),
            // Auto-coerce strings to target column type (#32)
            SqlParam::String(v) => match *ty {
                Type::INT2 => {
                    let parsed: i16 = v
                        .parse()
                        .map_err(|_| format!("Cannot coerce string \"{}\" to SMALLINT", v))?;
                    parsed.to_sql(ty, out)
                }
                Type::INT4 => {
                    let parsed: i32 = v
                        .parse()
                        .map_err(|_| format!("Cannot coerce string \"{}\" to INTEGER", v))?;
                    parsed.to_sql(ty, out)
                }
                Type::INT8 => {
                    let parsed: i64 = v
                        .parse()
                        .map_err(|_| format!("Cannot coerce string \"{}\" to BIGINT", v))?;
                    parsed.to_sql(ty, out)
                }
                Type::FLOAT4 | Type::FLOAT8 => {
                    let parsed: f64 = v
                        .parse()
                        .map_err(|_| format!("Cannot coerce string \"{}\" to FLOAT", v))?;
                    parsed.to_sql(ty, out)
                }
                Type::BOOL => {
                    let parsed = match v.to_lowercase().as_str() {
                        "true" | "t" | "1" | "yes" => true,
                        "false" | "f" | "0" | "no" => false,
                        _ => {
                            return Err(format!("Cannot coerce string \"{}\" to BOOLEAN", v).into())
                        }
                    };
                    parsed.to_sql(ty, out)
                }
                _ => v.to_sql(ty, out),
            },
            SqlParam::Bool(v) => v.to_sql(ty, out),
            SqlParam::Null => Ok(tokio_postgres::types::IsNull::Yes),
            SqlParam::IntArray(v) => v.to_sql(ty, out),
            SqlParam::StringArray(v) => v.to_sql(ty, out),
        }
    }

    fn accepts(_ty: &tokio_postgres::types::Type) -> bool {
        // Accept all types — we handle coercion in to_sql
        true
    }

    tokio_postgres::types::to_sql_checked!();
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
        Value::EnumValue {
            enum_name, variant, ..
        } if enum_name == "Option" && variant == "None" => SqlParam::Null,
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

/// Convert a tokio-postgres row to an Intent Map
fn row_to_value(row: &tokio_postgres::Row) -> Value {
    let mut map = HashMap::new();

    for (idx, column) in row.columns().iter().enumerate() {
        let name = column.name().to_string();
        let value = postgres_value_to_intent(row, idx, column.type_());
        map.insert(name, value);
    }

    Value::Map(map)
}

/// Construct an Option::None value (represents SQL NULL)
fn sql_none() -> Value {
    Value::none()
}

/// Convert a PostgreSQL value to an Intent Value based on type
fn postgres_value_to_intent(
    row: &tokio_postgres::Row,
    idx: usize,
    pg_type: &tokio_postgres::types::Type,
) -> Value {
    use tokio_postgres::types::Type;

    match *pg_type {
        // Boolean
        Type::BOOL => match row.try_get::<_, Option<bool>>(idx) {
            Ok(Some(v)) => Value::Bool(v),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Integers
        Type::INT2 => match row.try_get::<_, Option<i16>>(idx) {
            Ok(Some(v)) => Value::Int(v as i64),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },
        Type::INT4 => match row.try_get::<_, Option<i32>>(idx) {
            Ok(Some(v)) => Value::Int(v as i64),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },
        Type::INT8 => match row.try_get::<_, Option<i64>>(idx) {
            Ok(Some(v)) => Value::Int(v),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Floats
        Type::FLOAT4 => match row.try_get::<_, Option<f32>>(idx) {
            Ok(Some(v)) => Value::Float(v as f64),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },
        Type::FLOAT8 => match row.try_get::<_, Option<f64>>(idx) {
            Ok(Some(v)) => Value::Float(v),
            Ok(None) => sql_none(),
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
                Ok(None) => sql_none(),
                Err(_) => Value::Unit,
            }
        }

        // Strings
        Type::VARCHAR | Type::TEXT | Type::BPCHAR | Type::NAME => {
            match row.try_get::<_, Option<String>>(idx) {
                Ok(Some(v)) => Value::String(v),
                Ok(None) => sql_none(),
                Err(_) => Value::Unit,
            }
        }

        // JSON/JSONB
        Type::JSON | Type::JSONB => match row.try_get::<_, Option<serde_json::Value>>(idx) {
            Ok(Some(v)) => json_to_intent_value(&v),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Date
        Type::DATE => match row.try_get::<_, Option<NaiveDate>>(idx) {
            Ok(Some(v)) => Value::String(v.format("%Y-%m-%d").to_string()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Time
        Type::TIME => match row.try_get::<_, Option<NaiveTime>>(idx) {
            Ok(Some(v)) => Value::String(v.format("%H:%M:%S").to_string()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Timestamp without timezone
        Type::TIMESTAMP => match row.try_get::<_, Option<NaiveDateTime>>(idx) {
            Ok(Some(v)) => Value::String(v.format("%Y-%m-%dT%H:%M:%S").to_string()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Timestamp with timezone
        Type::TIMESTAMPTZ => match row.try_get::<_, Option<DateTime<Utc>>>(idx) {
            Ok(Some(v)) => Value::String(v.to_rfc3339()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // UUID
        Type::UUID => match row.try_get::<_, Option<uuid::Uuid>>(idx) {
            Ok(Some(v)) => Value::String(v.to_string()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Integer arrays
        Type::INT4_ARRAY => match row.try_get::<_, Option<Vec<i32>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(|i| Value::Int(i as i64)).collect()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },
        Type::INT8_ARRAY => match row.try_get::<_, Option<Vec<i64>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::Int).collect()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // String arrays
        Type::TEXT_ARRAY | Type::VARCHAR_ARRAY => {
            match row.try_get::<_, Option<Vec<String>>>(idx) {
                Ok(Some(v)) => Value::Array(v.into_iter().map(Value::String).collect()),
                Ok(None) => sql_none(),
                Err(_) => Value::Unit,
            }
        }

        // Float arrays
        Type::FLOAT8_ARRAY => match row.try_get::<_, Option<Vec<f64>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::Float).collect()),
            Ok(None) => sql_none(),
            Err(_) => Value::Unit,
        },

        // Boolean arrays
        Type::BOOL_ARRAY => match row.try_get::<_, Option<Vec<bool>>>(idx) {
            Ok(Some(v)) => Value::Array(v.into_iter().map(Value::Bool).collect()),
            Ok(None) => sql_none(),
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
    crate::stdlib::json::json_to_intent_value(json)
}

/// Connect to a PostgreSQL database — creates a connection pool
fn pg_connect(connection_string: &str) -> Result<Value> {
    // Build deadpool-postgres Config from connection string URL
    let mut cfg = Config::new();
    cfg.url = Some(connection_string.to_string());
    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });
    cfg.pool = Some(deadpool_postgres::PoolConfig {
        max_size: 20,
        ..Default::default()
    });

    // Create the pool
    let pool = match cfg.create_pool(Some(Runtime::Tokio1), NoTls) {
        Ok(p) => p,
        Err(e) => {
            let error_msg = e.to_string();
            let sanitized = if error_msg.contains("password")
                || error_msg.contains("authentication")
            {
                "Connection failed: authentication error (details hidden for security)".to_string()
            } else {
                let sanitized = error_msg.replace(connection_string, "<connection_string>");
                format!("Connection failed: {}", sanitized)
            };
            return Ok(Value::err(Value::String(sanitized)));
        }
    };

    // Verify the pool works by getting a connection
    let verify_result = DB_RUNTIME.block_on(async {
        match pool.get().await {
            Ok(_client) => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    });

    match verify_result {
        Ok(()) => {
            let id = CONNECTION_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

            // Store pool in registry
            if let Ok(mut registry) = POOL_REGISTRY.lock() {
                registry.insert(id, pool);
            }

            // Return a connection handle as a map
            let mut handle = HashMap::new();
            handle.insert("_pg_connection_id".to_string(), Value::Int(id as i64));
            handle.insert("connected".to_string(), Value::Bool(true));

            Ok(Value::ok(Value::Map(handle)))
        }
        Err(e) => {
            let sanitized = if e.contains("password") || e.contains("authentication") {
                "Connection failed: authentication error (details hidden for security)".to_string()
            } else {
                let sanitized = e.replace(connection_string, "<connection_string>");
                format!("Connection failed: {}", sanitized)
            };
            Ok(Value::err(Value::String(sanitized)))
        }
    }
}

/// Get a pool from the connection handle
fn get_pool(conn: &Value) -> Result<Pool> {
    match conn {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_pg_connection_id") {
                if let Ok(registry) = POOL_REGISTRY.lock() {
                    if let Some(pool) = registry.get(&(*id as u64)) {
                        return Ok(pool.clone());
                    }
                }
                Err(IntentError::runtime_error(
                    "Invalid or closed database connection".to_string(),
                ))
            } else {
                Err(IntentError::type_error(
                    "Expected a database connection handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::type_error(
            "Expected a database connection handle".to_string(),
        )),
    }
}

/// Get the connection ID from a connection handle
fn get_connection_id(conn: &Value) -> Result<u64> {
    match conn {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_pg_connection_id") {
                Ok(*id as u64)
            } else {
                Err(IntentError::type_error(
                    "Expected a database connection handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::type_error(
            "Expected a database connection handle".to_string(),
        )),
    }
}

/// Check if the connection handle has an active transaction
fn has_active_txn(conn: &Value) -> bool {
    if let Ok(id) = get_connection_id(conn) {
        if let Ok(registry) = TXN_REGISTRY.lock() {
            return registry.contains_key(&id);
        }
    }
    false
}

/// Format a tokio-postgres error with full detail from the database (#33)
fn format_pg_error(prefix: &str, e: &tokio_postgres::Error) -> String {
    if let Some(db_err) = e.as_db_error() {
        let mut msg = format!("{}: {}", prefix, db_err.message());
        if let Some(detail) = db_err.detail() {
            msg.push_str(&format!(" DETAIL: {}", detail));
        }
        if let Some(hint) = db_err.hint() {
            msg.push_str(&format!(" HINT: {}", hint));
        }
        if let Some(column) = db_err.column() {
            msg.push_str(&format!(" COLUMN: {}", column));
        }
        if let Some(constraint) = db_err.constraint() {
            msg.push_str(&format!(" CONSTRAINT: {}", constraint));
        }
        if let Some(table) = db_err.table() {
            msg.push_str(&format!(" TABLE: {}", table));
        }
        msg
    } else {
        format!("{}: {}", prefix, e)
    }
}

/// Execute a query using either the transaction client or a fresh pool connection
fn pg_query(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let sql_params: Vec<SqlParam> = params.iter().map(value_to_sql_param).collect();
    let param_refs: Vec<&(dyn ToSql + Sync)> = sql_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    let conn_id = get_connection_id(conn)?;

    // Check if there's an active transaction for this connection
    let has_txn = {
        if let Ok(registry) = TXN_REGISTRY.lock() {
            registry.contains_key(&conn_id)
        } else {
            false
        }
    };

    if has_txn {
        // Use the transaction's dedicated connection
        DB_RUNTIME.block_on(async {
            let txn_client = {
                let registry = TXN_REGISTRY.lock().map_err(|e| {
                    IntentError::runtime_error(format!("Failed to lock txn registry: {}", e))
                })?;
                registry.get(&conn_id).map(|c| c.clone()).ok_or_else(|| {
                    IntentError::runtime_error("Transaction connection lost".to_string())
                })
            }?;

            let client = txn_client.lock().await;
            match client.query(sql, &param_refs).await {
                Ok(rows) => {
                    let result: Vec<Value> = rows.iter().map(row_to_value).collect();
                    Ok(Value::ok(Value::Array(result)))
                }
                Err(e) => Ok(Value::err(Value::String(format_pg_error(
                    "Query failed",
                    &e,
                )))),
            }
        })
    } else {
        // Use a fresh connection from the pool
        let pool = get_pool(conn)?;
        DB_RUNTIME.block_on(async {
            let client = pool.get().await.map_err(|e| {
                IntentError::runtime_error(format!("Failed to get connection from pool: {}", e))
            })?;

            match client.query(sql, &param_refs).await {
                Ok(rows) => {
                    let result: Vec<Value> = rows.iter().map(row_to_value).collect();
                    Ok(Value::ok(Value::Array(result)))
                }
                Err(e) => Ok(Value::err(Value::String(format_pg_error(
                    "Query failed",
                    &e,
                )))),
            }
        })
    }
}

/// Execute a query and return a single row (or null)
fn pg_query_one(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let sql_params: Vec<SqlParam> = params.iter().map(value_to_sql_param).collect();
    let param_refs: Vec<&(dyn ToSql + Sync)> = sql_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    let conn_id = get_connection_id(conn)?;

    let has_txn = {
        if let Ok(registry) = TXN_REGISTRY.lock() {
            registry.contains_key(&conn_id)
        } else {
            false
        }
    };

    if has_txn {
        DB_RUNTIME.block_on(async {
            let txn_client = {
                let registry = TXN_REGISTRY.lock().map_err(|e| {
                    IntentError::runtime_error(format!("Failed to lock txn registry: {}", e))
                })?;
                registry.get(&conn_id).map(|c| c.clone()).ok_or_else(|| {
                    IntentError::runtime_error("Transaction connection lost".to_string())
                })
            }?;

            let client = txn_client.lock().await;
            match client.query_opt(sql, &param_refs).await {
                Ok(Some(row)) => Ok(Value::ok(Value::some(row_to_value(&row)))),
                Ok(None) => Ok(Value::ok(sql_none())),
                Err(e) => Ok(Value::err(Value::String(format_pg_error(
                    "Query failed",
                    &e,
                )))),
            }
        })
    } else {
        let pool = get_pool(conn)?;
        DB_RUNTIME.block_on(async {
            let client = pool.get().await.map_err(|e| {
                IntentError::runtime_error(format!("Failed to get connection from pool: {}", e))
            })?;

            match client.query_opt(sql, &param_refs).await {
                Ok(Some(row)) => Ok(Value::ok(Value::some(row_to_value(&row)))),
                Ok(None) => Ok(Value::ok(sql_none())),
                Err(e) => Ok(Value::err(Value::String(format_pg_error(
                    "Query failed",
                    &e,
                )))),
            }
        })
    }
}

/// Execute a statement (INSERT/UPDATE/DELETE) and return affected row count
fn pg_execute(conn: &Value, sql: &str, params: &[Value]) -> Result<Value> {
    let sql_params: Vec<SqlParam> = params.iter().map(value_to_sql_param).collect();
    let param_refs: Vec<&(dyn ToSql + Sync)> = sql_params
        .iter()
        .map(|p| p as &(dyn ToSql + Sync))
        .collect();

    let conn_id = get_connection_id(conn)?;

    let has_txn = {
        if let Ok(registry) = TXN_REGISTRY.lock() {
            registry.contains_key(&conn_id)
        } else {
            false
        }
    };

    if has_txn {
        DB_RUNTIME.block_on(async {
            let txn_client = {
                let registry = TXN_REGISTRY.lock().map_err(|e| {
                    IntentError::runtime_error(format!("Failed to lock txn registry: {}", e))
                })?;
                registry.get(&conn_id).map(|c| c.clone()).ok_or_else(|| {
                    IntentError::runtime_error("Transaction connection lost".to_string())
                })
            }?;

            let client = txn_client.lock().await;
            match client.execute(sql, &param_refs).await {
                Ok(count) => Ok(Value::ok(Value::Int(count as i64))),
                Err(e) => Ok(Value::err(Value::String(format_pg_error(
                    "Execute failed",
                    &e,
                )))),
            }
        })
    } else {
        let pool = get_pool(conn)?;
        DB_RUNTIME.block_on(async {
            let client = pool.get().await.map_err(|e| {
                IntentError::runtime_error(format!("Failed to get connection from pool: {}", e))
            })?;

            match client.execute(sql, &param_refs).await {
                Ok(count) => Ok(Value::ok(Value::Int(count as i64))),
                Err(e) => Ok(Value::err(Value::String(format_pg_error(
                    "Execute failed",
                    &e,
                )))),
            }
        })
    }
}

/// Close a database connection (drops the pool)
fn pg_close(conn: &Value) -> Result<Value> {
    match conn {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_pg_connection_id") {
                let conn_id = *id as u64;

                // Remove any active transaction
                if let Ok(mut registry) = TXN_REGISTRY.lock() {
                    registry.remove(&conn_id);
                }

                // Remove the pool
                if let Ok(mut registry) = POOL_REGISTRY.lock() {
                    registry.remove(&conn_id);
                    return Ok(Value::Bool(true));
                }
            }
            Ok(Value::Bool(false))
        }
        _ => Err(IntentError::type_error(
            "Expected a database connection handle".to_string(),
        )),
    }
}

/// Begin a transaction — checks out a dedicated connection from the pool
fn pg_begin(conn: &Value) -> Result<Value> {
    let pool = get_pool(conn)?;
    let conn_id = get_connection_id(conn)?;

    // Check if there's already an active transaction
    if has_active_txn(conn) {
        return Ok(Value::err(Value::String(
            "BEGIN failed: a transaction is already active on this connection".to_string(),
        )));
    }

    DB_RUNTIME.block_on(async {
        match pool.get().await {
            Ok(client) => {
                // Issue BEGIN on this dedicated connection
                match client.execute("BEGIN", &[]).await {
                    Ok(_) => {
                        // Store the dedicated client in the transaction registry
                        let txn_client = std::sync::Arc::new(tokio::sync::Mutex::new(client));
                        if let Ok(mut registry) = TXN_REGISTRY.lock() {
                            registry.insert(conn_id, txn_client);
                        }
                        // Return the same connection handle
                        Ok(Value::ok(conn.clone()))
                    }
                    Err(e) => Ok(Value::err(Value::String(format!("BEGIN failed: {}", e)))),
                }
            }
            Err(e) => Ok(Value::err(Value::String(format!(
                "BEGIN failed: could not get connection from pool: {}",
                e
            )))),
        }
    })
}

/// Commit a transaction
fn pg_commit(conn: &Value) -> Result<Value> {
    let conn_id = get_connection_id(conn)?;

    DB_RUNTIME.block_on(async {
        let txn_client = {
            let registry = TXN_REGISTRY.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock txn registry: {}", e))
            })?;
            registry.get(&conn_id).map(|c| c.clone()).ok_or_else(|| {
                IntentError::runtime_error("No active transaction to commit".to_string())
            })
        }?;

        let client = txn_client.lock().await;
        let result = match client.execute("COMMIT", &[]).await {
            Ok(_) => Ok(Value::ok(Value::Bool(true))),
            Err(e) => Ok(Value::err(Value::String(format!("COMMIT failed: {}", e)))),
        };

        // Remove the transaction client regardless of outcome
        drop(client);
        if let Ok(mut registry) = TXN_REGISTRY.lock() {
            registry.remove(&conn_id);
        }

        result
    })
}

/// Rollback a transaction
fn pg_rollback(conn: &Value) -> Result<Value> {
    let conn_id = get_connection_id(conn)?;

    DB_RUNTIME.block_on(async {
        let txn_client = {
            let registry = TXN_REGISTRY.lock().map_err(|e| {
                IntentError::runtime_error(format!("Failed to lock txn registry: {}", e))
            })?;
            registry.get(&conn_id).map(|c| c.clone()).ok_or_else(|| {
                IntentError::runtime_error("No active transaction to rollback".to_string())
            })
        }?;

        let client = txn_client.lock().await;
        let result = match client.execute("ROLLBACK", &[]).await {
            Ok(_) => Ok(Value::ok(Value::Bool(true))),
            Err(e) => Ok(Value::err(Value::String(format!("ROLLBACK failed: {}", e)))),
        };

        // Remove the transaction client regardless of outcome
        drop(client);
        if let Ok(mut registry) = TXN_REGISTRY.lock() {
            registry.remove(&conn_id);
        }

        result
    })
}

/// Initialize the std/db/postgres module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt connect
    // @module std/postgres
    // @module_description PostgreSQL database operations
    // @signature connect(connection_string: String) -> Result<Connection, String>
    // Open a connection pool to a PostgreSQL database.
    //
    // Establishes a connection pool using the provided connection string and
    // returns a connection handle that can be passed to query, execute, and
    // transaction functions. The handle is stored in a global registry keyed
    // by an internal connection ID. Uses deadpool-postgres for async pooling.
    // @param connection_string A PostgreSQL connection URI (e.g. "postgres://user:pass@localhost/mydb")
    // @returns Result::Ok containing a Connection map handle, or Result::Err with a description
    // @see_also close, query, execute, begin
    // @since v0.2.0
    // @tags #database
    // @example connect("postgres://user:pass@localhost/mydb") => Result::Ok(Connection) ~ "Open a database connection"
    // @error TypeError ~ "connect() requires a connection string" fix: "Pass a String connection URI as the argument"
    module.insert(
        "connect".to_string(),
        Value::NativeFunction {
            name: "connect".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| match &args[0] {
                Value::String(conn_str) => pg_connect(conn_str),
                _ => Err(IntentError::type_error(
                    "connect() requires a connection string".to_string(),
                )),
            },
        },
    );

    // @ntnt query
    // @module std/postgres
    // @signature query(conn: Connection, sql: String, params: Array | Unit) -> Result<Array<Map>, String>
    // Execute a SQL query and return all matching rows.
    //
    // Runs a parameterized SELECT (or any row-returning statement) against the
    // database. Parameters use PostgreSQL $1, $2, ... placeholders. Each
    // returned row is a Map whose keys are column names. Pass an empty array
    // or Unit when no parameters are needed.
    // @param conn A Connection handle obtained from connect()
    // @param sql The SQL query string with optional $N parameter placeholders
    // @param params An Array of bind parameter values, or Unit for no parameters
    // @returns Result::Ok containing an Array of row Maps, or Result::Err with a description
    // @see_also query_one, execute, connect
    // @since v0.2.0
    // @tags #database
    // @example query(db, "SELECT * FROM users WHERE active = $1", [true]) => Result::Ok([...]) ~ "Query with parameters"
    // @gotcha SQL NULL column values are returned as None, not Unit
    // @error TypeError ~ "query() requires (connection, sql_string, params_array)" fix: "Provide (Connection, String, Array) arguments"
    module.insert(
        "query".to_string(),
        Value::NativeFunction {
            name: "query".to_string(),
            arity: 3,
            max_arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => pg_query(conn, sql, params),
                (conn, Value::String(sql), Value::Unit) => pg_query(conn, sql, &[]),
                _ => Err(IntentError::type_error(
                    "query() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // @ntnt query_one
    // @module std/postgres
    // @signature query_one(conn: Connection, sql: String, params: Array | Unit) -> Result<Map | None, String>
    // Execute a SQL query and return at most one row.
    //
    // Behaves like query() but uses PostgreSQL's query_opt internally to
    // return either a single row Map or None when no row matches.
    // Ideal for lookups by primary key or unique column.
    // @param conn A Connection handle obtained from connect()
    // @param sql The SQL query string with optional $N parameter placeholders
    // @param params An Array of bind parameter values, or Unit for no parameters
    // @returns Result::Ok containing a row Map or None if no match, or Result::Err with a description
    // @see_also query, execute, connect
    // @since v0.2.0
    // @tags #database
    // @example query_one(db, "SELECT * FROM users WHERE id = $1", [1]) => Result::Ok({...}) ~ "Fetch one row by ID"
    // @example query_one(db, "SELECT * FROM users WHERE id = $1", [999]) => Result::Ok(None) ~ "No matching row"
    // @gotcha SQL NULL column values are returned as None, not Unit
    // @error TypeError ~ "query_one() requires (connection, sql_string, params_array)" fix: "Provide (Connection, String, Array) arguments"
    module.insert(
        "query_one".to_string(),
        Value::NativeFunction {
            name: "query_one".to_string(),
            arity: 3,
            max_arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => pg_query_one(conn, sql, params),
                (conn, Value::String(sql), Value::Unit) => pg_query_one(conn, sql, &[]),
                _ => Err(IntentError::type_error(
                    "query_one() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // @ntnt execute
    // @module std/postgres
    // @signature execute(conn: Connection, sql: String, params: Array | Unit) -> Result<Int, String>
    // Execute a SQL statement and return the number of affected rows.
    //
    // Use this for INSERT, UPDATE, DELETE, and other statements that do not
    // return row data. Parameters use $1, $2, ... placeholders. The Ok
    // variant contains an Int representing the count of rows affected.
    // @param conn A Connection handle obtained from connect()
    // @param sql The SQL statement string with optional $N parameter placeholders
    // @param params An Array of bind parameter values, or Unit for no parameters
    // @returns Result::Ok containing the Int count of affected rows, or Result::Err with a description
    // @see_also query, query_one, connect, begin
    // @since v0.2.0
    // @tags #database
    // @example execute(db, "INSERT INTO users (name) VALUES ($1)", ["Alice"]) => Result::Ok(1) ~ "Insert one row"
    // @error TypeError ~ "execute() requires (connection, sql_string, params_array)" fix: "Provide (Connection, String, Array) arguments"
    module.insert(
        "execute".to_string(),
        Value::NativeFunction {
            name: "execute".to_string(),
            arity: 3,
            max_arity: 3,
            func: |args| match (&args[0], &args[1], &args[2]) {
                (conn, Value::String(sql), Value::Array(params)) => pg_execute(conn, sql, params),
                (conn, Value::String(sql), Value::Unit) => pg_execute(conn, sql, &[]),
                _ => Err(IntentError::type_error(
                    "execute() requires (connection, sql_string, params_array)".to_string(),
                )),
            },
        },
    );

    // @ntnt close
    // @module std/postgres
    // @signature close(conn: Connection) -> Bool
    // Close a PostgreSQL database connection pool.
    //
    // Removes the connection pool from the internal registry, allowing all
    // pooled connections to be released. Returns true if the pool was found
    // and removed, false otherwise.
    // @param conn A Connection handle obtained from connect()
    // @returns true if the connection was successfully closed, false if it was not found
    // @see_also connect
    // @since v0.2.0
    // @tags #database
    // @example close(db) => true ~ "Close an open connection"
    // @error TypeError ~ "Expected a database connection handle" fix: "Pass a Connection handle returned by connect()"
    module.insert(
        "close".to_string(),
        Value::NativeFunction {
            name: "close".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| pg_close(&args[0]),
        },
    );

    // @ntnt begin
    // @module std/postgres
    // @signature begin(conn: Connection) -> Result<Connection, String>
    // Begin a database transaction.
    //
    // Checks out a dedicated connection from the pool and issues a SQL BEGIN
    // statement. On success the same connection handle is returned inside
    // Result::Ok -- subsequent query() and execute() calls on that handle
    // operate within the transaction until commit() or rollback() is called.
    // @param conn A Connection handle obtained from connect()
    // @returns Result::Ok containing the Connection handle (now in a transaction), or Result::Err with a description
    // @see_also commit, rollback, execute, query
    // @since v0.2.0
    // @tags #database
    // @example begin(db) => Result::Ok(Connection) ~ "Start a transaction"
    // @error RuntimeError ~ "Failed to lock connection: ..." fix: "Ensure the connection is not being used concurrently"
    module.insert(
        "begin".to_string(),
        Value::NativeFunction {
            name: "begin".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| pg_begin(&args[0]),
        },
    );

    // @ntnt commit
    // @module std/postgres
    // @signature commit(conn: Connection) -> Result<Bool, String>
    // Commit the current transaction.
    //
    // Issues a SQL COMMIT on the dedicated transaction connection, making all
    // changes since the last begin() permanent. Returns true on success.
    // The dedicated connection is returned to the pool after commit.
    // @param conn A Connection handle with an active transaction from begin()
    // @returns true on success, or Result::Err with a description on failure
    // @see_also begin, rollback
    // @since v0.2.0
    // @tags #database
    // @example commit(db) => true ~ "Commit an active transaction"
    // @error RuntimeError ~ "COMMIT failed: ..." fix: "Ensure a transaction was started with begin() before committing"
    module.insert(
        "commit".to_string(),
        Value::NativeFunction {
            name: "commit".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| pg_commit(&args[0]),
        },
    );

    // @ntnt rollback
    // @module std/postgres
    // @signature rollback(conn: Connection) -> Result<Bool, String>
    // Roll back the current transaction.
    //
    // Issues a SQL ROLLBACK on the dedicated transaction connection, discarding
    // all changes made since the last begin(). Returns true on success. The
    // dedicated connection is returned to the pool after rollback.
    // @param conn A Connection handle with an active transaction from begin()
    // @returns true on success, or Result::Err with a description on failure
    // @see_also begin, commit
    // @since v0.2.0
    // @tags #database
    // @example rollback(db) => true ~ "Roll back an active transaction"
    // @error RuntimeError ~ "ROLLBACK failed: ..." fix: "Ensure a transaction was started with begin() before rolling back"
    module.insert(
        "rollback".to_string(),
        Value::NativeFunction {
            name: "rollback".to_string(),
            arity: 1,
            max_arity: 1,
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
