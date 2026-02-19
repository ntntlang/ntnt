//! Key-Value Store module for NTNT
//!
//! Provides a unified KV interface with SQLite (bundled) and Redis/Valkey backends.
//!
//! Example usage:
//! ```ntnt
//! import { open, get, set, del, has, list, expire, ttl, flush } from "std/kv"
//!
//! let cache = open("cache.db")?
//! set(cache, "user:123", map { "name": "Alice" }, map { "ttl": 3600 })?
//! let user = get(cache, "user:123")?
//! ```

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use redis::Commands;
use rusqlite::{params, Connection};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// Thread-safe SQLite KV store registry
static SQLITE_KV_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, Arc<Mutex<SQLiteKV>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Thread-safe Redis KV store registry
static REDIS_KV_REGISTRY: std::sync::LazyLock<Mutex<HashMap<u64, Arc<Mutex<RedisKV>>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

static KV_ID_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Backend type identifier
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum KVBackend {
    SQLite,
    Redis,
}

/// Wrapper for SQLite connection
pub struct SQLiteKV {
    conn: Connection,
}

/// Wrapper for Redis connection
pub struct RedisKV {
    conn: redis::Connection,
}

/// Get current Unix timestamp
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Serialize a Value to string for storage
fn serialize_value(value: &Value) -> (String, String) {
    match value {
        Value::String(s) => (s.clone(), "string".to_string()),
        Value::Int(i) => (i.to_string(), "int".to_string()),
        Value::Float(f) => (f.to_string(), "float".to_string()),
        Value::Bool(b) => (b.to_string(), "bool".to_string()),
        Value::Array(_) => {
            let json = serde_json::to_string(&value_to_json(value)).unwrap_or_default();
            (json, "array".to_string())
        }
        Value::Map(_) => {
            let json = serde_json::to_string(&value_to_json(value)).unwrap_or_default();
            (json, "map".to_string())
        }
        _ => (format!("{:?}", value), "unknown".to_string()),
    }
}

/// Convert Value to serde_json::Value for serialization
fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::Unit => serde_json::Value::Null,
        Value::Bool(b) => serde_json::Value::Bool(*b),
        Value::Int(i) => serde_json::Value::Number((*i).into()),
        Value::Float(f) => serde_json::Number::from_f64(*f)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::String(s) => serde_json::Value::String(s.clone()),
        Value::Array(arr) => serde_json::Value::Array(arr.iter().map(value_to_json).collect()),
        Value::Map(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        _ => serde_json::Value::Null,
    }
}

/// Convert serde_json::Value to Value
fn json_to_value(json: serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::Unit,
        serde_json::Value::Bool(b) => Value::Bool(b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Int(i)
            } else if let Some(f) = n.as_f64() {
                Value::Float(f)
            } else {
                Value::Unit
            }
        }
        serde_json::Value::String(s) => Value::String(s),
        serde_json::Value::Array(arr) => Value::Array(arr.into_iter().map(json_to_value).collect()),
        serde_json::Value::Object(obj) => {
            let map: HashMap<String, Value> = obj
                .into_iter()
                .map(|(k, v)| (k, json_to_value(v)))
                .collect();
            Value::Map(map)
        }
    }
}

/// Deserialize a stored value back to Value
fn deserialize_value(data: &str, type_hint: &str) -> Value {
    match type_hint {
        "string" => Value::String(data.to_string()),
        "int" => data.parse::<i64>().map(Value::Int).unwrap_or(Value::Unit),
        "float" => data.parse::<f64>().map(Value::Float).unwrap_or(Value::Unit),
        "bool" => Value::Bool(data == "true"),
        "array" | "map" => serde_json::from_str(data)
            .map(json_to_value)
            .unwrap_or(Value::Unit),
        _ => Value::String(data.to_string()),
    }
}

// ============================================================================
// SQLite Backend Implementation
// ============================================================================

impl SQLiteKV {
    /// Create a new SQLite KV store
    pub fn new(path: &str) -> Result<Self> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(path)
        }
        .map_err(|e| IntentError::RuntimeError(format!("Failed to open KV store: {}", e)))?;

        // Create the KV table if it doesn't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS _kv (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                type TEXT NOT NULL DEFAULT 'string',
                expires_at INTEGER
            )",
            [],
        )
        .map_err(|e| IntentError::RuntimeError(format!("Failed to create KV table: {}", e)))?;

        // Create indices for performance
        conn.execute(
            "CREATE INDEX IF NOT EXISTS _kv_expires ON _kv(expires_at) WHERE expires_at IS NOT NULL",
            [],
        )
        .ok();

        conn.execute("CREATE INDEX IF NOT EXISTS _kv_prefix ON _kv(key)", [])
            .ok();

        Ok(SQLiteKV { conn })
    }

    /// Get a value by key
    pub fn get(&self, key: &str) -> Result<Option<Value>> {
        let now = now_unix();

        let result: std::result::Result<(String, String), rusqlite::Error> = self.conn.query_row(
            "SELECT value, type FROM _kv WHERE key = ? AND (expires_at IS NULL OR expires_at > ?)",
            params![key, now],
            |row| Ok((row.get(0)?, row.get(1)?)),
        );

        match result {
            Ok((value, type_hint)) => Ok(Some(deserialize_value(&value, &type_hint))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(IntentError::RuntimeError(format!("KV get error: {}", e))),
        }
    }

    /// Set a value with optional TTL
    pub fn set(&self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<()> {
        // Setting Unit deletes the key
        if matches!(value, Value::Unit) {
            return self.del(key).map(|_| ());
        }

        let (serialized, type_hint) = serialize_value(value);
        let expires_at = ttl_seconds.map(|ttl| now_unix() + ttl);

        self.conn
            .execute(
                "INSERT OR REPLACE INTO _kv (key, value, type, expires_at) VALUES (?, ?, ?, ?)",
                params![key, serialized, type_hint, expires_at],
            )
            .map_err(|e| IntentError::RuntimeError(format!("KV set error: {}", e)))?;

        Ok(())
    }

    /// Delete a key
    pub fn del(&self, key: &str) -> Result<bool> {
        let changes = self
            .conn
            .execute("DELETE FROM _kv WHERE key = ?", params![key])
            .map_err(|e| IntentError::RuntimeError(format!("KV del error: {}", e)))?;

        Ok(changes > 0)
    }

    /// Check if a key exists (and is not expired)
    pub fn has(&self, key: &str) -> Result<bool> {
        let now = now_unix();

        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM _kv WHERE key = ? AND (expires_at IS NULL OR expires_at > ?)",
                params![key, now],
                |row| row.get(0),
            )
            .map_err(|e| IntentError::RuntimeError(format!("KV has error: {}", e)))?;

        Ok(count > 0)
    }

    /// List keys with optional prefix
    pub fn list(&self, prefix: Option<&str>) -> Result<Vec<String>> {
        let now = now_unix();

        let mut stmt = match prefix {
            Some(_) => self
                .conn
                .prepare(
                    "SELECT key FROM _kv WHERE key LIKE ? AND (expires_at IS NULL OR expires_at > ?)",
                )
                .map_err(|e| IntentError::RuntimeError(format!("KV list error: {}", e)))?,
            None => self
                .conn
                .prepare("SELECT key FROM _kv WHERE expires_at IS NULL OR expires_at > ?")
                .map_err(|e| IntentError::RuntimeError(format!("KV list error: {}", e)))?,
        };

        let keys: Vec<String> = match prefix {
            Some(p) => {
                let pattern = format!("{}%", p);
                stmt.query_map(params![pattern, now], |row| row.get(0))
                    .map_err(|e| IntentError::RuntimeError(format!("KV list error: {}", e)))?
                    .filter_map(|r| r.ok())
                    .collect()
            }
            None => stmt
                .query_map(params![now], |row| row.get(0))
                .map_err(|e| IntentError::RuntimeError(format!("KV list error: {}", e)))?
                .filter_map(|r| r.ok())
                .collect(),
        };

        // Filter out internal type metadata keys (consistent with Redis impl)
        let filtered: Vec<String> = keys
            .into_iter()
            .filter(|k| !k.ends_with(":__type"))
            .collect();

        Ok(filtered)
    }

    /// Set TTL on existing key
    pub fn expire(&self, key: &str, seconds: i64) -> Result<bool> {
        let now = now_unix();
        let expires_at = now + seconds;

        let changes = self
            .conn
            .execute(
                "UPDATE _kv SET expires_at = ? WHERE key = ? AND (expires_at IS NULL OR expires_at > ?)",
                params![expires_at, key, now],
            )
            .map_err(|e| IntentError::RuntimeError(format!("KV expire error: {}", e)))?;

        Ok(changes > 0)
    }

    /// Get remaining TTL in seconds
    pub fn ttl(&self, key: &str) -> Result<Option<i64>> {
        let now = now_unix();

        let result: std::result::Result<Option<i64>, rusqlite::Error> = self.conn.query_row(
            "SELECT expires_at FROM _kv WHERE key = ? AND (expires_at IS NULL OR expires_at > ?)",
            params![key, now],
            |row| row.get(0),
        );

        match result {
            Ok(Some(expires_at)) => Ok(Some((expires_at - now).max(0))),
            Ok(None) => Ok(None), // Key exists but has no expiry
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None), // Key doesn't exist
            Err(e) => Err(IntentError::RuntimeError(format!("KV ttl error: {}", e))),
        }
    }

    /// Delete all keys
    pub fn flush(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM _kv", [])
            .map_err(|e| IntentError::RuntimeError(format!("KV flush error: {}", e)))?;

        Ok(())
    }
}

// ============================================================================
// Redis/Valkey Backend Implementation
// ============================================================================

impl RedisKV {
    /// Create a new Redis/Valkey connection
    pub fn new(url: &str) -> Result<Self> {
        // Convert valkey:// to redis:// since the redis crate only recognizes redis://
        let normalized_url = if url.starts_with("valkey://") {
            url.replacen("valkey://", "redis://", 1)
        } else {
            url.to_string()
        };

        let client = redis::Client::open(normalized_url.as_str()).map_err(|e| {
            IntentError::RuntimeError(format!("Failed to create Redis client: {}", e))
        })?;

        let conn = client
            .get_connection()
            .map_err(|e| IntentError::RuntimeError(format!("Failed to connect to Redis: {}", e)))?;

        Ok(RedisKV { conn })
    }

    /// Get a value by key
    pub fn get(&mut self, key: &str) -> Result<Option<Value>> {
        // Redis stores type hint as a separate key: key:__type
        let value: Option<String> = self
            .conn
            .get(key)
            .map_err(|e| IntentError::RuntimeError(format!("Redis get error: {}", e)))?;

        match value {
            Some(data) => {
                let type_key = format!("{}:__type", key);
                let type_hint: String = self
                    .conn
                    .get(&type_key)
                    .unwrap_or_else(|_| "string".to_string());
                Ok(Some(deserialize_value(&data, &type_hint)))
            }
            None => Ok(None),
        }
    }

    /// Set a value with optional TTL
    pub fn set(&mut self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<()> {
        // Setting Unit deletes the key
        if matches!(value, Value::Unit) {
            return self.del(key).map(|_| ());
        }

        let (serialized, type_hint) = serialize_value(value);
        let type_key = format!("{}:__type", key);

        match ttl_seconds {
            Some(ttl) => {
                // Set with expiration
                self.conn
                    .set_ex::<_, _, ()>(key, &serialized, ttl as u64)
                    .map_err(|e| IntentError::RuntimeError(format!("Redis set error: {}", e)))?;
                self.conn
                    .set_ex::<_, _, ()>(&type_key, &type_hint, ttl as u64)
                    .map_err(|e| IntentError::RuntimeError(format!("Redis set error: {}", e)))?;
            }
            None => {
                // Set without expiration
                self.conn
                    .set::<_, _, ()>(key, &serialized)
                    .map_err(|e| IntentError::RuntimeError(format!("Redis set error: {}", e)))?;
                self.conn
                    .set::<_, _, ()>(&type_key, &type_hint)
                    .map_err(|e| IntentError::RuntimeError(format!("Redis set error: {}", e)))?;
            }
        }

        Ok(())
    }

    /// Delete a key
    pub fn del(&mut self, key: &str) -> Result<bool> {
        let type_key = format!("{}:__type", key);
        let deleted: i32 = self
            .conn
            .del(key)
            .map_err(|e| IntentError::RuntimeError(format!("Redis del error: {}", e)))?;
        // Also delete the type key
        let _: i32 = self.conn.del(&type_key).unwrap_or(0);
        Ok(deleted > 0)
    }

    /// Check if a key exists
    pub fn has(&mut self, key: &str) -> Result<bool> {
        let exists: bool = self
            .conn
            .exists(key)
            .map_err(|e| IntentError::RuntimeError(format!("Redis exists error: {}", e)))?;
        Ok(exists)
    }

    /// List keys with optional prefix
    pub fn list(&mut self, prefix: Option<&str>) -> Result<Vec<String>> {
        let pattern = match prefix {
            Some(p) => format!("{}*", p),
            None => "*".to_string(),
        };

        let keys: Vec<String> = self
            .conn
            .keys(&pattern)
            .map_err(|e| IntentError::RuntimeError(format!("Redis keys error: {}", e)))?;

        // Filter out internal type keys
        let filtered: Vec<String> = keys
            .into_iter()
            .filter(|k| !k.ends_with(":__type"))
            .collect();

        Ok(filtered)
    }

    /// Set TTL on existing key
    pub fn expire(&mut self, key: &str, seconds: i64) -> Result<bool> {
        let success: bool = self
            .conn
            .expire(key, seconds)
            .map_err(|e| IntentError::RuntimeError(format!("Redis expire error: {}", e)))?;

        if success {
            // Also set TTL on the type key
            let type_key = format!("{}:__type", key);
            let _: bool = self.conn.expire(&type_key, seconds).unwrap_or(false);
        }

        Ok(success)
    }

    /// Get remaining TTL in seconds
    pub fn ttl(&mut self, key: &str) -> Result<Option<i64>> {
        let ttl: i64 = self
            .conn
            .ttl(key)
            .map_err(|e| IntentError::RuntimeError(format!("Redis ttl error: {}", e)))?;

        match ttl {
            -2 => Ok(None), // Key doesn't exist
            -1 => Ok(None), // Key exists but has no expiry (return None to match SQLite behavior)
            t => Ok(Some(t)),
        }
    }

    /// Delete all keys (FLUSHDB)
    pub fn flush(&mut self) -> Result<()> {
        redis::cmd("FLUSHDB")
            .query::<()>(&mut self.conn)
            .map_err(|e| IntentError::RuntimeError(format!("Redis flush error: {}", e)))?;
        Ok(())
    }
}

// ============================================================================
// Backend Detection and Dispatch
// ============================================================================

/// Determine backend type from handle
fn get_backend_type(handle: &Value) -> Result<KVBackend> {
    match handle {
        Value::Map(map) => {
            if let Some(Value::String(backend)) = map.get("_backend") {
                match backend.as_str() {
                    "sqlite" => Ok(KVBackend::SQLite),
                    "redis" | "valkey" => Ok(KVBackend::Redis),
                    _ => Err(IntentError::TypeError(format!(
                        "Unknown KV backend: {}",
                        backend
                    ))),
                }
            } else {
                Err(IntentError::TypeError(
                    "Expected a KV store handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::TypeError(
            "Expected a KV store handle".to_string(),
        )),
    }
}

/// Get SQLite KV store from the registry
fn get_sqlite_kv(handle: &Value) -> Result<Arc<Mutex<SQLiteKV>>> {
    match handle {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_kv_store_id") {
                if let Ok(registry) = SQLITE_KV_REGISTRY.lock() {
                    if let Some(kv) = registry.get(&(*id as u64)) {
                        return Ok(Arc::clone(kv));
                    }
                }
                Err(IntentError::RuntimeError(
                    "Invalid or closed KV store".to_string(),
                ))
            } else {
                Err(IntentError::TypeError(
                    "Expected a KV store handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::TypeError(
            "Expected a KV store handle".to_string(),
        )),
    }
}

/// Get Redis KV store from the registry
fn get_redis_kv(handle: &Value) -> Result<Arc<Mutex<RedisKV>>> {
    match handle {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_kv_store_id") {
                if let Ok(registry) = REDIS_KV_REGISTRY.lock() {
                    if let Some(kv) = registry.get(&(*id as u64)) {
                        return Ok(Arc::clone(kv));
                    }
                }
                Err(IntentError::RuntimeError(
                    "Invalid or closed KV store".to_string(),
                ))
            } else {
                Err(IntentError::TypeError(
                    "Expected a KV store handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::TypeError(
            "Expected a KV store handle".to_string(),
        )),
    }
}

// ============================================================================
// Module Export
// ============================================================================

/// Create the std/kv module
pub fn create_kv_module() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt open
    // @module std/kv
    // @module_description Key-value store with SQLite and Redis/Valkey backends
    // @signature open(url: String) -> Result<KVStore, String>
    // Open a KV store connection.
    //
    // For SQLite (bundled, zero-config), pass a file path or ":memory:".
    // For Redis/Valkey (production), pass a URL like "redis://host:6379".
    // @param url Connection string: file path for SQLite, redis:// or valkey:// URL for Redis/Valkey
    // @returns Result containing the KV store handle or an error message
    // @example open("cache.db") ~ "Open SQLite KV store"
    // @example open(":memory:") ~ "Open in-memory SQLite KV store"
    // @example open("redis://localhost:6379") ~ "Open Redis connection"
    // @example open("valkey://localhost:6379/0") ~ "Open Valkey connection with database 0"
    module.insert(
        "open".to_string(),
        Value::NativeFunction {
            name: "open".to_string(),
            arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::TypeError(
                        "open() requires 1 argument (url)".to_string(),
                    ));
                }

                let url = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "open() requires a string argument".to_string(),
                        ))
                    }
                };

                let id = KV_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

                // Determine backend from URL
                if url.starts_with("redis://") || url.starts_with("valkey://") {
                    // Redis/Valkey backend
                    let kv = RedisKV::new(&url)?;
                    let shared = Arc::new(Mutex::new(kv));

                    if let Ok(mut registry) = REDIS_KV_REGISTRY.lock() {
                        registry.insert(id, shared);
                    }

                    let backend_name = if url.starts_with("valkey://") {
                        "valkey"
                    } else {
                        "redis"
                    };
                    let mut handle = HashMap::new();
                    handle.insert(
                        "_backend".to_string(),
                        Value::String(backend_name.to_string()),
                    );
                    handle.insert("_url".to_string(), Value::String(url));
                    handle.insert("_kv_store_id".to_string(), Value::Int(id as i64));

                    return Ok(Value::EnumValue {
                        enum_name: "Result".to_string(),
                        variant: "Ok".to_string(),
                        values: vec![Value::Map(handle)],
                    });
                }

                // SQLite backend (default for file paths)
                let kv = SQLiteKV::new(&url)?;
                let shared = Arc::new(Mutex::new(kv));

                if let Ok(mut registry) = SQLITE_KV_REGISTRY.lock() {
                    registry.insert(id, shared);
                }

                let mut handle = HashMap::new();
                handle.insert("_backend".to_string(), Value::String("sqlite".to_string()));
                handle.insert("_url".to_string(), Value::String(url));
                handle.insert("_kv_store_id".to_string(), Value::Int(id as i64));

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![Value::Map(handle)],
                })
            },
        },
    );

    // @ntnt get
    // @module std/kv
    // @signature get(kv: KVStore, key: String) -> Result<Option<Any>, String>
    // Get a value by key from the KV store.
    //
    // Returns None if the key doesn't exist or has expired.
    // Values are automatically deserialized to their original type.
    // @param kv The KV store handle from open()
    // @param key The key to retrieve
    // @returns Result containing Some(value) or None if not found
    // @example get(cache, "user:123") ~ "Get user by key"
    // @example get(cache, "session:abc") ~ "Get session data"
    module.insert(
        "get".to_string(),
        Value::NativeFunction {
            name: "get".to_string(),
            arity: 2,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::TypeError(
                        "get() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "get() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let result = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.get(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.get(&key)?
                    }
                };

                let option_value = match result {
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

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![option_value],
                })
            },
        },
    );

    // @ntnt set
    // @module std/kv
    // @signature set(kv: KVStore, key: String, value: Any, opts?: Map) -> Result<Unit, String>
    // Set a key-value pair in the KV store.
    //
    // Values are automatically serialized. Maps and arrays are stored as JSON.
    // Setting a value to None deletes the key.
    // @param kv The KV store handle from open()
    // @param key The key to set
    // @param value The value to store (string, int, float, bool, map, or array)
    // @param opts Optional map with "ttl" key for expiration in seconds
    // @returns Result indicating success or error
    // @example set(cache, "user:123", map { "name": "Alice" }) ~ "Set without TTL"
    // @example set(cache, "session:abc", token, map { "ttl": 3600 }) ~ "Set with 1 hour TTL"
    module.insert(
        "set".to_string(),
        Value::NativeFunction {
            name: "set".to_string(),
            arity: 0, // variadic: 3-4 args (kv, key, value, opts?)
            func: |args| {
                if args.len() < 3 || args.len() > 4 {
                    return Err(IntentError::TypeError(
                        "set() requires 3-4 arguments (kv, key, value, opts?)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "set() requires a string key".to_string(),
                        ))
                    }
                };

                let value = &args[2];

                let ttl = if args.len() == 4 {
                    match &args[3] {
                        Value::Map(opts) => opts.get("ttl").and_then(|v| match v {
                            Value::Int(i) => Some(*i),
                            _ => None,
                        }),
                        _ => None,
                    }
                } else {
                    None
                };

                let backend = get_backend_type(&args[0])?;
                match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.set(&key, value, ttl)?;
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.set(&key, value, ttl)?;
                    }
                }

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![Value::Unit],
                })
            },
        },
    );

    // @ntnt del
    // @module std/kv
    // @signature del(kv: KVStore, key: String) -> Result<Bool, String>
    // Delete a key from the KV store.
    //
    // Returns true if the key existed and was deleted, false if it didn't exist.
    // @param kv The KV store handle from open()
    // @param key The key to delete
    // @returns Result containing true if deleted, false if not found
    // @example del(cache, "user:123") ~ "Delete a key"
    module.insert(
        "del".to_string(),
        Value::NativeFunction {
            name: "del".to_string(),
            arity: 2,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::TypeError(
                        "del() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "del() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let deleted = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.del(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.del(&key)?
                    }
                };

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![Value::Bool(deleted)],
                })
            },
        },
    );

    // @ntnt has
    // @module std/kv
    // @signature has(kv: KVStore, key: String) -> Result<Bool, String>
    // Check if a key exists in the KV store.
    //
    // Returns false for expired keys.
    // @param kv The KV store handle from open()
    // @param key The key to check
    // @returns Result containing true if exists, false otherwise
    // @example has(cache, "user:123") ~ "Check if key exists"
    module.insert(
        "has".to_string(),
        Value::NativeFunction {
            name: "has".to_string(),
            arity: 2,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::TypeError(
                        "has() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "has() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let exists = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.has(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.has(&key)?
                    }
                };

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![Value::Bool(exists)],
                })
            },
        },
    );

    // @ntnt list
    // @module std/kv
    // @signature list(kv: KVStore, prefix?: String) -> Result<Array<String>, String>
    // List keys in the KV store, optionally filtered by prefix.
    //
    // Without a prefix, returns all keys (use sparingly on large stores).
    // @param kv The KV store handle from open()
    // @param prefix Optional prefix to filter keys
    // @returns Result containing array of matching key names
    // @example list(cache, "user:") ~ "List all user keys"
    // @example list(cache, "session:") ~ "List all session keys"
    // @example list(cache) ~ "List all keys"
    module.insert(
        "list".to_string(),
        Value::NativeFunction {
            name: "list".to_string(),
            arity: 0, // variadic: 1-2 args (kv, prefix?)
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::TypeError(
                        "list() requires 1-2 arguments (kv, prefix?)".to_string(),
                    ));
                }

                let prefix = if args.len() == 2 {
                    match &args[1] {
                        Value::String(s) => Some(s.clone()),
                        Value::Unit => None,
                        _ => {
                            return Err(IntentError::TypeError(
                                "list() prefix must be a string".to_string(),
                            ))
                        }
                    }
                } else {
                    None
                };

                let backend = get_backend_type(&args[0])?;
                let keys = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.list(prefix.as_deref())?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.list(prefix.as_deref())?
                    }
                };

                let key_values: Vec<Value> = keys.into_iter().map(Value::String).collect();

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![Value::Array(key_values)],
                })
            },
        },
    );

    // @ntnt expire
    // @module std/kv
    // @signature expire(kv: KVStore, key: String, seconds: Int) -> Result<Bool, String>
    // Set a TTL (time-to-live) on an existing key.
    //
    // Returns true if the key exists and TTL was set, false if key doesn't exist.
    // @param kv The KV store handle from open()
    // @param key The key to set expiration on
    // @param seconds Number of seconds until expiration
    // @returns Result containing true if TTL was set, false if key not found
    // @example expire(cache, "user:123", 600) ~ "Expire in 10 minutes"
    module.insert(
        "expire".to_string(),
        Value::NativeFunction {
            name: "expire".to_string(),
            arity: 3,
            func: |args| {
                if args.len() != 3 {
                    return Err(IntentError::TypeError(
                        "expire() requires 3 arguments (kv, key, seconds)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "expire() requires a string key".to_string(),
                        ))
                    }
                };

                let seconds = match &args[2] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(IntentError::TypeError(
                            "expire() requires an integer for seconds".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let success = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.expire(&key, seconds)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.expire(&key, seconds)?
                    }
                };

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![Value::Bool(success)],
                })
            },
        },
    );

    // @ntnt ttl
    // @module std/kv
    // @signature ttl(kv: KVStore, key: String) -> Result<Option<Int>, String>
    // Get the remaining TTL (time-to-live) for a key in seconds.
    //
    // Returns None if the key doesn't exist or has no expiration set.
    // @param kv The KV store handle from open()
    // @param key The key to check TTL for
    // @returns Result containing Some(seconds) or None
    // @example ttl(cache, "session:abc") ~ "Get remaining TTL"
    module.insert(
        "ttl".to_string(),
        Value::NativeFunction {
            name: "ttl".to_string(),
            arity: 2,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::TypeError(
                        "ttl() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::TypeError(
                            "ttl() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let remaining = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.ttl(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.ttl(&key)?
                    }
                };

                let option_value = match remaining {
                    Some(secs) => Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        values: vec![Value::Int(secs)],
                    },
                    None => Value::EnumValue {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        values: vec![],
                    },
                };

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![option_value],
                })
            },
        },
    );

    // @ntnt flush
    // @module std/kv
    // @signature flush(kv: KVStore) -> Result<Unit, String>
    // Delete all keys from the KV store.
    //
    // Use with caution - this removes all data. Useful for tests and resets.
    // @param kv The KV store handle from open()
    // @returns Result indicating success or error
    // @example flush(cache) ~ "Clear all cached data"
    module.insert(
        "flush".to_string(),
        Value::NativeFunction {
            name: "flush".to_string(),
            arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::TypeError(
                        "flush() requires 1 argument (kv)".to_string(),
                    ));
                }

                let backend = get_backend_type(&args[0])?;
                match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.flush()?;
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::RuntimeError(format!("KV lock error: {}", e))
                        })?;
                        kv.flush()?;
                    }
                }

                Ok(Value::EnumValue {
                    enum_name: "Result".to_string(),
                    variant: "Ok".to_string(),
                    values: vec![Value::Unit],
                })
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sqlite_kv_basic_operations() {
        let kv = SQLiteKV::new(":memory:").unwrap();

        // Test set and get
        kv.set("key1", &Value::String("value1".to_string()), None)
            .unwrap();
        let result = kv.get("key1").unwrap();
        assert!(matches!(result, Some(Value::String(s)) if s == "value1"));

        // Test missing key
        let result = kv.get("nonexistent").unwrap();
        assert!(result.is_none());

        // Test has
        assert!(kv.has("key1").unwrap());
        assert!(!kv.has("nonexistent").unwrap());

        // Test del
        assert!(kv.del("key1").unwrap());
        assert!(!kv.del("key1").unwrap()); // Already deleted
        assert!(!kv.has("key1").unwrap());
    }

    #[test]
    fn test_sqlite_kv_types() {
        let kv = SQLiteKV::new(":memory:").unwrap();

        // Test int
        kv.set("int_key", &Value::Int(42), None).unwrap();
        let result = kv.get("int_key").unwrap();
        assert!(matches!(result, Some(Value::Int(42))));

        // Test float
        kv.set("float_key", &Value::Float(3.14), None).unwrap();
        let result = kv.get("float_key").unwrap();
        assert!(matches!(result, Some(Value::Float(f)) if (f - 3.14).abs() < 0.001));

        // Test bool
        kv.set("bool_key", &Value::Bool(true), None).unwrap();
        let result = kv.get("bool_key").unwrap();
        assert!(matches!(result, Some(Value::Bool(true))));

        // Test map
        let mut map = HashMap::new();
        map.insert("name".to_string(), Value::String("Alice".to_string()));
        map.insert("age".to_string(), Value::Int(30));
        kv.set("map_key", &Value::Map(map), None).unwrap();
        let result = kv.get("map_key").unwrap();
        assert!(matches!(result, Some(Value::Map(_))));
    }

    #[test]
    fn test_sqlite_kv_ttl() {
        let kv = SQLiteKV::new(":memory:").unwrap();

        // Set with TTL
        kv.set("ttl_key", &Value::String("expires".to_string()), Some(3600))
            .unwrap();

        // Check TTL
        let ttl = kv.ttl("ttl_key").unwrap();
        assert!(matches!(ttl, Some(t) if t > 3500 && t <= 3600));

        // Set expire on existing key
        kv.set("no_ttl", &Value::String("value".to_string()), None)
            .unwrap();
        assert!(kv.expire("no_ttl", 600).unwrap());
        let ttl = kv.ttl("no_ttl").unwrap();
        assert!(matches!(ttl, Some(t) if t > 500 && t <= 600));
    }

    #[test]
    fn test_sqlite_kv_list() {
        let kv = SQLiteKV::new(":memory:").unwrap();

        // Set some keys
        kv.set("user:1", &Value::String("alice".to_string()), None)
            .unwrap();
        kv.set("user:2", &Value::String("bob".to_string()), None)
            .unwrap();
        kv.set("session:abc", &Value::String("token".to_string()), None)
            .unwrap();

        // List with prefix
        let users = kv.list(Some("user:")).unwrap();
        assert_eq!(users.len(), 2);
        assert!(users.contains(&"user:1".to_string()));
        assert!(users.contains(&"user:2".to_string()));

        // List all
        let all = kv.list(None).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_sqlite_kv_flush() {
        let kv = SQLiteKV::new(":memory:").unwrap();

        kv.set("key1", &Value::String("value1".to_string()), None)
            .unwrap();
        kv.set("key2", &Value::String("value2".to_string()), None)
            .unwrap();

        kv.flush().unwrap();

        assert!(!kv.has("key1").unwrap());
        assert!(!kv.has("key2").unwrap());
        assert_eq!(kv.list(None).unwrap().len(), 0);
    }

    // Note: Redis tests require a running Redis server
    // They can be run manually with: cargo test --features redis-tests
    #[cfg(feature = "redis-tests")]
    mod redis_tests {
        use super::*;

        #[test]
        fn test_redis_kv_basic_operations() {
            let mut kv = RedisKV::new("redis://localhost:6379").unwrap();
            kv.flush().unwrap(); // Start clean

            // Test set and get
            kv.set("test:key1", &Value::String("value1".to_string()), None)
                .unwrap();
            let result = kv.get("test:key1").unwrap();
            assert!(matches!(result, Some(Value::String(s)) if s == "value1"));

            // Test missing key
            let result = kv.get("test:nonexistent").unwrap();
            assert!(result.is_none());

            // Cleanup
            kv.del("test:key1").unwrap();
        }
    }
}
