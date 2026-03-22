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

/// Serialize a Value to a JSON envelope for Redis storage.
/// Non-string types get wrapped: {"__ntnt_t":"<type>","v":<json_value>}
/// Plain strings are stored as-is for backward compatibility and efficiency.
fn serialize_value_envelope(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Int(i) => serde_json::json!({"__ntnt_t": "int", "v": i}).to_string(),
        Value::Float(f) => serde_json::json!({"__ntnt_t": "float", "v": f}).to_string(),
        Value::Bool(b) => serde_json::json!({"__ntnt_t": "bool", "v": b}).to_string(),
        Value::Array(_) | Value::Map(_) => {
            let type_name = match value {
                Value::Array(_) => "array",
                Value::Map(_) => "map",
                _ => unreachable!(),
            };
            let json_val = value_to_json(value);
            serde_json::json!({"__ntnt_t": type_name, "v": json_val}).to_string()
        }
        _ => format!("{:?}", value),
    }
}

/// Deserialize a Redis value using envelope format, with backward compatibility.
/// Detects envelope `{"__ntnt_t":...,"v":...}` or falls back to legacy `__type` hint.
fn deserialize_value_envelope(data: &str, legacy_type_hint: Option<&str>) -> Value {
    // Try envelope format first
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) {
        if let Some(obj) = parsed.as_object() {
            if let (Some(serde_json::Value::String(t)), Some(v)) =
                (obj.get("__ntnt_t"), obj.get("v"))
            {
                return match t.as_str() {
                    "int" => v.as_i64().map(Value::Int).unwrap_or(Value::Unit),
                    "float" => v.as_f64().map(Value::Float).unwrap_or(Value::Unit),
                    "bool" => v.as_bool().map(Value::Bool).unwrap_or(Value::Unit),
                    "array" | "map" => json_to_value(v.clone()),
                    _ => Value::String(data.to_string()),
                };
            }
        }
    }

    // Fall back to legacy format (separate __type key)
    match legacy_type_hint {
        Some(hint) => deserialize_value(data, hint),
        None => Value::String(data.to_string()),
    }
}

/// Convert Value to serde_json::Value for serialization
/// Public wrapper for value_to_json, used by std/jobs for test queue serialization.
pub fn value_to_json_public(value: &Value) -> serde_json::Value {
    value_to_json(value)
}

pub fn json_to_value_public(json: &serde_json::Value) -> Value {
    json_to_value(json.clone())
}

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
        .map_err(|e| IntentError::runtime_error(format!("Failed to open KV store: {}", e)))?;

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
        .map_err(|e| IntentError::runtime_error(format!("Failed to create KV table: {}", e)))?;

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
            Err(e) => Err(IntentError::runtime_error(format!("KV get error: {}", e))),
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
            .map_err(|e| IntentError::runtime_error(format!("KV set error: {}", e)))?;

        Ok(())
    }

    /// Set a key only if it doesn't already exist (atomic NX operation).
    ///
    /// First deletes any expired entry for the key, then attempts INSERT OR IGNORE.
    /// Returns `Ok(true)` if the key was set, `Ok(false)` if it already existed (and is not expired).
    pub fn set_nx(&self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<bool> {
        let now = now_unix();

        // Delete any expired entry for this key first (treat expired as non-existent)
        self.conn
            .execute(
                "DELETE FROM _kv WHERE key = ? AND expires_at IS NOT NULL AND expires_at <= ?",
                params![key, now],
            )
            .map_err(|e| {
                IntentError::runtime_error(format!("KV set_nx delete expired error: {}", e))
            })?;

        let (serialized, type_hint) = serialize_value(value);
        let expires_at = ttl_seconds.map(|ttl| now + ttl);

        let changes = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO _kv (key, value, type, expires_at) VALUES (?, ?, ?, ?)",
                params![key, serialized, type_hint, expires_at],
            )
            .map_err(|e| IntentError::runtime_error(format!("KV set_nx error: {}", e)))?;

        Ok(changes > 0)
    }

    /// Delete a key
    pub fn del(&self, key: &str) -> Result<bool> {
        let changes = self
            .conn
            .execute("DELETE FROM _kv WHERE key = ?", params![key])
            .map_err(|e| IntentError::runtime_error(format!("KV del error: {}", e)))?;

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
            .map_err(|e| IntentError::runtime_error(format!("KV has error: {}", e)))?;

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
                .map_err(|e| IntentError::runtime_error(format!("KV list error: {}", e)))?,
            None => self
                .conn
                .prepare("SELECT key FROM _kv WHERE expires_at IS NULL OR expires_at > ?")
                .map_err(|e| IntentError::runtime_error(format!("KV list error: {}", e)))?,
        };

        let keys: Vec<String> = match prefix {
            Some(p) => {
                let pattern = format!("{}%", p);
                stmt.query_map(params![pattern, now], |row| row.get(0))
                    .map_err(|e| IntentError::runtime_error(format!("KV list error: {}", e)))?
                    .filter_map(|r| r.ok())
                    .collect()
            }
            None => stmt
                .query_map(params![now], |row| row.get(0))
                .map_err(|e| IntentError::runtime_error(format!("KV list error: {}", e)))?
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
            .map_err(|e| IntentError::runtime_error(format!("KV expire error: {}", e)))?;

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
            Err(e) => Err(IntentError::runtime_error(format!("KV ttl error: {}", e))),
        }
    }

    /// Delete all keys
    pub fn flush(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM _kv", [])
            .map_err(|e| IntentError::runtime_error(format!("KV flush error: {}", e)))?;

        Ok(())
    }

    /// Atomically claim the first key matching a prefix: SELECT + DELETE in one transaction.
    /// Returns the key and its raw (value, type) pair, or None if no matching key exists.
    ///
    /// `floor`: if provided, only claim keys where `key >= floor`. Used by band workers
    /// to restrict claims to their priority range.
    ///
    /// `ceiling`: if provided, only claim keys where `key <= ceiling`. This filters
    /// future-scheduled jobs at the SQL level — no claim+re-enqueue cycle needed.
    pub fn claim(
        &self,
        prefix: &str,
        floor: Option<&str>,
        ceiling: Option<&str>,
    ) -> Result<Option<(String, Value)>> {
        let now = now_unix();
        let pattern = format!("{}%", prefix);

        // BEGIN IMMEDIATE ensures no other writer can interleave
        self.conn
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| IntentError::runtime_error(format!("KV claim begin error: {}", e)))?;

        let row: std::result::Result<(String, String, String), rusqlite::Error> =
            match (floor, ceiling) {
                (Some(fl), Some(ceil)) => self.conn.query_row(
                    "SELECT key, value, type FROM _kv WHERE key LIKE ? AND key >= ? AND key <= ? AND (expires_at IS NULL OR expires_at > ?) ORDER BY key LIMIT 1",
                    params![pattern, fl, ceil, now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ),
                (Some(fl), None) => self.conn.query_row(
                    "SELECT key, value, type FROM _kv WHERE key LIKE ? AND key >= ? AND (expires_at IS NULL OR expires_at > ?) ORDER BY key LIMIT 1",
                    params![pattern, fl, now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ),
                (None, Some(ceil)) => self.conn.query_row(
                    "SELECT key, value, type FROM _kv WHERE key LIKE ? AND key <= ? AND (expires_at IS NULL OR expires_at > ?) ORDER BY key LIMIT 1",
                    params![pattern, ceil, now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ),
                (None, None) => self.conn.query_row(
                    "SELECT key, value, type FROM _kv WHERE key LIKE ? AND (expires_at IS NULL OR expires_at > ?) ORDER BY key LIMIT 1",
                    params![pattern, now],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                ),
            };

        match row {
            Ok((key, value, type_hint)) => {
                self.conn
                    .execute("DELETE FROM _kv WHERE key = ?", params![key])
                    .map_err(|e| {
                        let _ = self.conn.execute_batch("ROLLBACK");
                        IntentError::runtime_error(format!("KV claim delete error: {}", e))
                    })?;
                self.conn.execute_batch("COMMIT").map_err(|e| {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    IntentError::runtime_error(format!("KV claim commit error: {}", e))
                })?;
                Ok(Some((key, deserialize_value(&value, &type_hint))))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                self.conn.execute_batch("COMMIT").map_err(|e| {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    IntentError::runtime_error(format!("KV claim commit error: {}", e))
                })?;
                Ok(None)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(IntentError::runtime_error(format!("KV claim error: {}", e)))
            }
        }
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
            IntentError::runtime_error(format!("Failed to create Redis client: {}", e))
        })?;

        let conn = client.get_connection().map_err(|e| {
            IntentError::runtime_error(format!("Failed to connect to Redis: {}", e))
        })?;

        Ok(RedisKV { conn })
    }

    /// Get a value by key
    pub fn get(&mut self, key: &str) -> Result<Option<Value>> {
        let value: Option<String> = self
            .conn
            .get(key)
            .map_err(|e| IntentError::runtime_error(format!("Redis get error: {}", e)))?;

        match value {
            Some(data) => {
                // Try envelope format first, fall back to legacy __type key
                let type_key = format!("{}:__type", key);
                let legacy_hint: Option<String> = self.conn.get(&type_key).ok();
                Ok(Some(deserialize_value_envelope(
                    &data,
                    legacy_hint.as_deref(),
                )))
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

        // Use envelope format — single key, no __type sibling
        let serialized = serialize_value_envelope(value);

        match ttl_seconds {
            Some(ttl) => {
                self.conn
                    .set_ex::<_, _, ()>(key, &serialized, ttl as u64)
                    .map_err(|e| IntentError::runtime_error(format!("Redis set error: {}", e)))?;
            }
            None => {
                self.conn
                    .set::<_, _, ()>(key, &serialized)
                    .map_err(|e| IntentError::runtime_error(format!("Redis set error: {}", e)))?;
            }
        }

        // Clean up legacy __type key if it exists (migration)
        let type_key = format!("{}:__type", key);
        let _: std::result::Result<i32, _> = self.conn.del(&type_key);

        Ok(())
    }

    /// Set a key only if it doesn't already exist (atomic NX operation).
    ///
    /// Uses `SET key value NX [EX ttl]` — atomic in Redis.
    /// Returns `Ok(true)` if set, `Ok(false)` if the key already existed.
    pub fn set_nx(&mut self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<bool> {
        let serialized = serialize_value_envelope(value);

        let result: Option<String> = match ttl_seconds {
            Some(ttl) => redis::cmd("SET")
                .arg(key)
                .arg(&serialized)
                .arg("NX")
                .arg("EX")
                .arg(ttl)
                .query(&mut self.conn)
                .map_err(|e| IntentError::runtime_error(format!("Redis set_nx error: {}", e)))?,
            None => redis::cmd("SET")
                .arg(key)
                .arg(&serialized)
                .arg("NX")
                .query(&mut self.conn)
                .map_err(|e| IntentError::runtime_error(format!("Redis set_nx error: {}", e)))?,
        };

        // Redis returns "OK" if set, nil if key existed
        Ok(result.as_deref() == Some("OK"))
    }

    /// Delete a key
    pub fn del(&mut self, key: &str) -> Result<bool> {
        let type_key = format!("{}:__type", key);
        let deleted: i32 = self
            .conn
            .del(key)
            .map_err(|e| IntentError::runtime_error(format!("Redis del error: {}", e)))?;
        // Also delete the type key
        let _: i32 = self.conn.del(&type_key).unwrap_or(0);
        Ok(deleted > 0)
    }

    /// Check if a key exists
    pub fn has(&mut self, key: &str) -> Result<bool> {
        let exists: bool = self
            .conn
            .exists(key)
            .map_err(|e| IntentError::runtime_error(format!("Redis exists error: {}", e)))?;
        Ok(exists)
    }

    /// List keys with optional prefix (uses SCAN instead of KEYS to avoid blocking Redis)
    pub fn list(&mut self, prefix: Option<&str>) -> Result<Vec<String>> {
        let pattern = match prefix {
            Some(p) => format!("{}*", p),
            None => "*".to_string(),
        };

        let mut all_keys: Vec<String> = Vec::new();
        let mut cursor: u64 = 0;

        loop {
            let (next_cursor, batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .cursor_arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query(&mut self.conn)
                .map_err(|e| IntentError::runtime_error(format!("Redis scan error: {}", e)))?;

            all_keys.extend(batch);
            cursor = next_cursor;

            if cursor == 0 {
                break;
            }
        }

        // Deduplicate keys (SCAN may return duplicates across iterations)
        all_keys.sort_unstable();
        all_keys.dedup();

        // Filter out internal type keys
        let filtered: Vec<String> = all_keys
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
            .map_err(|e| IntentError::runtime_error(format!("Redis expire error: {}", e)))?;

        Ok(success)
    }

    /// Get remaining TTL in seconds
    pub fn ttl(&mut self, key: &str) -> Result<Option<i64>> {
        let ttl: i64 = self
            .conn
            .ttl(key)
            .map_err(|e| IntentError::runtime_error(format!("Redis ttl error: {}", e)))?;

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
            .map_err(|e| IntentError::runtime_error(format!("Redis flush error: {}", e)))?;
        Ok(())
    }

    /// Atomically claim the first key matching a prefix (sorted lexicographically).
    /// Uses a Lua script executed via EVAL for atomicity — no other Redis command
    /// can interleave, preventing double-claim under concurrent workers.
    ///
    /// `floor`: if provided, only claim keys where `key >= floor`. Used by band workers
    /// to restrict claims to their priority range.
    ///
    /// `ceiling`: if provided, only claim keys where `key <= ceiling`. This filters
    /// future-scheduled jobs at the Redis level.
    pub fn claim(
        &mut self,
        prefix: &str,
        floor: Option<&str>,
        ceiling: Option<&str>,
    ) -> Result<Option<(String, Value)>> {
        let pattern = format!("{}*", prefix);
        let ceil = ceiling.unwrap_or("");
        let fl = floor.unwrap_or("");

        // Lua script runs atomically in Redis — KEYS+sort+GET+DEL in one operation.
        // Note: KEYS scans the entire Redis keyspace (O(total keys), not O(matching keys)).
        // This is acceptable for typical job queue sizes. For very large Redis instances
        // with millions of non-job keys, consider a sorted-set approach instead.
        let lua_script = r#"
            local keys = redis.call('KEYS', ARGV[1])
            if #keys == 0 then return nil end
            table.sort(keys)
            local floor_val = ARGV[2]
            local ceiling = ARGV[3]
            local past_floor = (floor_val == '')
            for _, key in ipairs(keys) do
                -- Skip internal type metadata keys
                if not string.find(key, ':__type$') then
                    -- Floor filter: keys are sorted ascending, so once we pass floor
                    -- all subsequent keys are also past it — stop checking
                    if not past_floor then
                        if key >= floor_val then
                            past_floor = true
                        end
                    end
                    if past_floor then
                        if ceiling ~= '' and key > ceiling then
                            -- Early exit: all remaining keys exceed ceiling (sorted ascending)
                            break
                        end
                        local val = redis.call('GET', key)
                        if val then
                            -- Read legacy type hint before deleting
                            local type_hint = redis.call('GET', key .. ':__type')
                            redis.call('DEL', key)
                            redis.call('DEL', key .. ':__type')
                            if type_hint then
                                return {key, val, type_hint}
                            end
                            return {key, val}
                        end
                    end
                end
            end
            return nil
        "#;

        let result: redis::Value = redis::cmd("EVAL")
            .arg(lua_script)
            .arg(0) // no KEYS args, using ARGV only
            .arg(&pattern)
            .arg(fl)
            .arg(ceil)
            .query(&mut self.conn)
            .map_err(|e| IntentError::runtime_error(format!("Redis claim error: {}", e)))?;

        match result {
            redis::Value::Array(ref items) if items.len() >= 2 => {
                let key = match &items[0] {
                    redis::Value::BulkString(bytes) => {
                        String::from_utf8(bytes.clone()).map_err(|e| {
                            IntentError::runtime_error(format!(
                                "Redis claim key UTF-8 error: {}",
                                e
                            ))
                        })?
                    }
                    redis::Value::SimpleString(s) => s.clone(),
                    other => {
                        return Err(IntentError::runtime_error(format!(
                            "Redis claim: unexpected key type in Lua response: {:?}",
                            other
                        )))
                    }
                };
                let data = match &items[1] {
                    redis::Value::BulkString(bytes) => {
                        String::from_utf8(bytes.clone()).map_err(|e| {
                            IntentError::runtime_error(format!(
                                "Redis claim data UTF-8 error: {}",
                                e
                            ))
                        })?
                    }
                    redis::Value::SimpleString(s) => s.clone(),
                    other => {
                        return Err(IntentError::runtime_error(format!(
                            "Redis claim: unexpected data type in Lua response: {:?}",
                            other
                        )))
                    }
                };
                // Third element is the legacy type hint (if the key had one)
                let legacy_hint = items.get(2).and_then(|v| match v {
                    redis::Value::BulkString(bytes) => String::from_utf8(bytes.clone()).ok(),
                    redis::Value::SimpleString(s) => Some(s.clone()),
                    _ => None,
                });
                Ok(Some((
                    key,
                    deserialize_value_envelope(&data, legacy_hint.as_deref()),
                )))
            }
            redis::Value::Nil => Ok(None),
            other => Err(IntentError::runtime_error(format!(
                "Redis claim: unexpected Lua response type: {:?}",
                other
            ))),
        }
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
                    _ => Err(IntentError::type_error(format!(
                        "Unknown KV backend: {}",
                        backend
                    ))),
                }
            } else {
                Err(IntentError::type_error(
                    "Expected a KV store handle".to_string(),
                ))
            }
        }
        Value::EnumValue {
            enum_name, variant, ..
        } if enum_name == "Result" && (variant == "Ok" || variant == "Err") => {
            Err(IntentError::type_error(
                "Expected a KV store handle, got a Result — did you forget to unwrap() the open() call? \
                 Use: let kv = unwrap(open(\"...\"))"
                    .to_string(),
            ))
        }
        _ => Err(IntentError::type_error(
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
                Err(IntentError::runtime_error(
                    "Invalid or closed KV store".to_string(),
                ))
            } else {
                Err(IntentError::type_error(
                    "Expected a KV store handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::type_error(
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
                Err(IntentError::runtime_error(
                    "Invalid or closed KV store".to_string(),
                ))
            } else {
                Err(IntentError::type_error(
                    "Expected a KV store handle".to_string(),
                ))
            }
        }
        _ => Err(IntentError::type_error(
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
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "open() requires 1 argument (url)".to_string(),
                    ));
                }

                let url = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
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

                    return Ok(Value::ok(Value::Map(handle)));
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

                Ok(Value::ok(Value::Map(handle)))
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
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "get() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "get() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let result = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.get(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.get(&key)?
                    }
                };

                let option_value = match result {
                    Some(v) => Value::some(v),
                    None => Value::none(),
                };

                Ok(Value::ok(option_value))
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
            arity: 3, // variadic: 3-4 args (kv, key, value, opts?)
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 3 || args.len() > 4 {
                    return Err(IntentError::type_error(
                        "set() requires 3-4 arguments (kv, key, value, opts?)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
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
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.set(&key, value, ttl)?;
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.set(&key, value, ttl)?;
                    }
                }

                Ok(Value::ok(Value::Unit))
            },
        },
    );

    // @ntnt set_nx
    // @module std/kv
    // @signature set_nx(handle: KVStore, key: String, value: Any, opts?: Map) -> Result<Bool, String>
    // Set a key only if it doesn't already exist (atomic NX operation).
    //
    // Returns Ok(true) if the key was set (it did not exist before), or Ok(false)
    // if the key already existed. For SQLite, expired keys are treated as non-existent.
    // For Redis, uses SET NX which is atomic.
    // @param handle The KV store handle from open()
    // @param key The key to set
    // @param value The value to store
    // @param opts Optional map with "ttl" key (seconds until expiry)
    // @returns Ok(true) if set, Ok(false) if key existed
    // @see_also set, get, del
    // @example set_nx(kv, "lock:job:123", "worker-1") => Ok(true) ~ "Acquire lock"
    // @example set_nx(kv, "lock:job:123", "worker-2") => Ok(false) ~ "Lock already held"
    // @example set_nx(kv, "session:abc", data, map { "ttl": 3600 }) ~ "Set with TTL"
    module.insert(
        "set_nx".to_string(),
        Value::NativeFunction {
            name: "set_nx".to_string(),
            arity: 3,
            max_arity: 4,
            requires: None,
            func: |args| {
                if args.len() < 3 {
                    return Err(IntentError::type_error(
                        "set_nx() requires at least 3 arguments (handle, key, value)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "set_nx() key must be a string".to_string(),
                        ))
                    }
                };

                let ttl = if args.len() >= 4 {
                    match &args[3] {
                        Value::Map(opts) => match opts.get("ttl") {
                            Some(Value::Int(n)) => Some(*n),
                            Some(_) => {
                                return Err(IntentError::type_error(
                                    "set_nx() ttl must be an integer".to_string(),
                                ))
                            }
                            None => None,
                        },
                        Value::Unit => None,
                        _ => {
                            return Err(IntentError::type_error(
                                "set_nx() opts must be a map".to_string(),
                            ))
                        }
                    }
                } else {
                    None
                };

                let was_set = kv_set_nx(&args[0], &key, &args[2], ttl)?;
                Ok(Value::ok(Value::Bool(was_set)))
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
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "del() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "del() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let deleted = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.del(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.del(&key)?
                    }
                };

                Ok(Value::ok(Value::Bool(deleted)))
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
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "has() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "has() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let exists = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.has(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.has(&key)?
                    }
                };

                Ok(Value::ok(Value::Bool(exists)))
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
            arity: 1, // variadic: 1-2 args (kv, prefix?)
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "list() requires 1-2 arguments (kv, prefix?)".to_string(),
                    ));
                }

                let prefix = if args.len() == 2 {
                    match &args[1] {
                        Value::String(s) => Some(s.clone()),
                        Value::Unit => None,
                        _ => {
                            return Err(IntentError::type_error(
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
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.list(prefix.as_deref())?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.list(prefix.as_deref())?
                    }
                };

                let key_values: Vec<Value> = keys.into_iter().map(Value::String).collect();

                Ok(Value::ok(Value::Array(key_values)))
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
            max_arity: 3,
            requires: None,
            func: |args| {
                if args.len() != 3 {
                    return Err(IntentError::type_error(
                        "expire() requires 3 arguments (kv, key, seconds)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "expire() requires a string key".to_string(),
                        ))
                    }
                };

                let seconds = match &args[2] {
                    Value::Int(i) => *i,
                    _ => {
                        return Err(IntentError::type_error(
                            "expire() requires an integer for seconds".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let success = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.expire(&key, seconds)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.expire(&key, seconds)?
                    }
                };

                Ok(Value::ok(Value::Bool(success)))
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
            max_arity: 2,
            requires: None,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "ttl() requires 2 arguments (kv, key)".to_string(),
                    ));
                }

                let key = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "ttl() requires a string key".to_string(),
                        ))
                    }
                };

                let backend = get_backend_type(&args[0])?;
                let remaining = match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.ttl(&key)?
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.ttl(&key)?
                    }
                };

                let option_value = match remaining {
                    Some(secs) => Value::some(Value::Int(secs)),
                    None => Value::none(),
                };

                Ok(Value::ok(option_value))
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
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "flush() requires 1 argument (kv)".to_string(),
                    ));
                }

                let backend = get_backend_type(&args[0])?;
                match backend {
                    KVBackend::SQLite => {
                        let kv_arc = get_sqlite_kv(&args[0])?;
                        let kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.flush()?;
                    }
                    KVBackend::Redis => {
                        let kv_arc = get_redis_kv(&args[0])?;
                        let mut kv = kv_arc.lock().map_err(|e| {
                            IntentError::runtime_error(format!("KV lock error: {}", e))
                        })?;
                        kv.flush()?;
                    }
                }

                Ok(Value::ok(Value::Unit))
            },
        },
    );

    module
}

// ============================================================================
// Public API for cross-module use (used by std/jobs)
// ============================================================================

/// Open a KV store connection by URL. Returns the KV handle as a Value::Map.
/// This is the programmatic equivalent of the `open()` stdlib function.
pub fn open_kv(url: &str) -> Result<Value> {
    let id = KV_ID_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    if url.starts_with("redis://") || url.starts_with("valkey://") {
        let kv = RedisKV::new(url)?;
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
        handle.insert("_url".to_string(), Value::String(url.to_string()));
        handle.insert("_kv_store_id".to_string(), Value::Int(id as i64));
        return Ok(Value::Map(handle));
    }

    // SQLite backend
    let clean_url = url.strip_prefix("sqlite:").unwrap_or(url);
    let kv = SQLiteKV::new(clean_url)?;
    let shared = Arc::new(Mutex::new(kv));

    if let Ok(mut registry) = SQLITE_KV_REGISTRY.lock() {
        registry.insert(id, shared);
    }

    let mut handle = HashMap::new();
    handle.insert("_backend".to_string(), Value::String("sqlite".to_string()));
    handle.insert("_url".to_string(), Value::String(url.to_string()));
    handle.insert("_kv_store_id".to_string(), Value::Int(id as i64));
    Ok(Value::Map(handle))
}

/// Set a key-value pair in a KV store handle.
pub fn kv_set(handle: &Value, key: &str, value: &Value, ttl: Option<i64>) -> Result<()> {
    let backend = get_backend_type(handle)?;
    match backend {
        KVBackend::SQLite => {
            let kv_arc = get_sqlite_kv(handle)?;
            let kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.set(key, value, ttl)?;
        }
        KVBackend::Redis => {
            let kv_arc = get_redis_kv(handle)?;
            let mut kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.set(key, value, ttl)?;
        }
    }
    Ok(())
}

/// Get a value from a KV store handle. Returns Value::Unit if not found.
pub fn kv_get(handle: &Value, key: &str) -> Result<Value> {
    let backend = get_backend_type(handle)?;
    let result = match backend {
        KVBackend::SQLite => {
            let kv_arc = get_sqlite_kv(handle)?;
            let kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.get(key)?
        }
        KVBackend::Redis => {
            let kv_arc = get_redis_kv(handle)?;
            let mut kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.get(key)?
        }
    };
    Ok(result.unwrap_or(Value::Unit))
}

/// Delete a key from a KV store handle.
pub fn kv_del(handle: &Value, key: &str) -> Result<bool> {
    let backend = get_backend_type(handle)?;
    match backend {
        KVBackend::SQLite => {
            let kv_arc = get_sqlite_kv(handle)?;
            let kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.del(key)
        }
        KVBackend::Redis => {
            let kv_arc = get_redis_kv(handle)?;
            let mut kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.del(key)
        }
    }
}

/// List keys by optional prefix from a KV store handle.
pub fn kv_list(handle: &Value, prefix: Option<&str>) -> Result<Vec<String>> {
    let backend = get_backend_type(handle)?;
    match backend {
        KVBackend::SQLite => {
            let kv_arc = get_sqlite_kv(handle)?;
            let kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.list(prefix)
        }
        KVBackend::Redis => {
            let kv_arc = get_redis_kv(handle)?;
            let mut kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.list(prefix)
        }
    }
}

/// Claim the first key matching a prefix from a KV store handle.
/// Returns Some((key, value)) or None if no matching key exists.
/// The claimed key is deleted from the store.
///
/// `floor`: if provided, only claim keys where `key >= floor`. Used by band workers
/// to restrict claims to their priority range.
///
/// `ceiling`: if provided, only claim keys where `key <= ceiling`. Used by the
/// job worker to skip future-scheduled jobs without claiming and re-enqueuing.
///
/// Atomicity: SQLite uses `BEGIN IMMEDIATE` for full transaction safety.
/// Redis/Valkey uses an atomic Lua script via EVAL (KEYS+sort+GET+DEL in one operation).
pub fn kv_claim(
    handle: &Value,
    prefix: &str,
    floor: Option<&str>,
    ceiling: Option<&str>,
) -> Result<Option<(String, Value)>> {
    let backend = get_backend_type(handle)?;
    match backend {
        KVBackend::SQLite => {
            let kv_arc = get_sqlite_kv(handle)?;
            let kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.claim(prefix, floor, ceiling)
        }
        KVBackend::Redis => {
            let kv_arc = get_redis_kv(handle)?;
            let mut kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.claim(prefix, floor, ceiling)
        }
    }
}

/// Atomically set a key only if it doesn't already exist.
///
/// Returns `Ok(true)` if the key was set, `Ok(false)` if the key already existed.
/// SQLite: `DELETE` expired + `INSERT OR IGNORE`. Redis: `SET NX [EX ttl]`.
pub fn kv_set_nx(handle: &Value, key: &str, value: &Value, ttl: Option<i64>) -> Result<bool> {
    let backend = get_backend_type(handle)?;
    match backend {
        KVBackend::SQLite => {
            let kv_arc = get_sqlite_kv(handle)?;
            let kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.set_nx(key, value, ttl)
        }
        KVBackend::Redis => {
            let kv_arc = get_redis_kv(handle)?;
            let mut kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.set_nx(key, value, ttl)
        }
    }
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

    #[test]
    fn test_sqlite_claim_no_ceiling() {
        let kv = SQLiteKV::new(":memory:").unwrap();

        // Seed 3 keys with lexicographic ordering
        kv.set("prefix:aaa", &Value::String("first".to_string()), None)
            .unwrap();
        kv.set("prefix:bbb", &Value::String("second".to_string()), None)
            .unwrap();
        kv.set("prefix:ccc", &Value::String("third".to_string()), None)
            .unwrap();

        // Claim without ceiling — should get lexicographically first
        let result = kv.claim("prefix:", None, None).unwrap();
        assert!(result.is_some());
        let (key, val) = result.unwrap();
        assert_eq!(key, "prefix:aaa");
        assert!(matches!(val, Value::String(s) if s == "first"));

        // Key should be deleted — second claim gets next key
        let result = kv.claim("prefix:", None, None).unwrap();
        assert!(result.is_some());
        let (key, _) = result.unwrap();
        assert_eq!(key, "prefix:bbb");

        // Third claim
        let result = kv.claim("prefix:", None, None).unwrap();
        assert!(result.is_some());
        let (key, _) = result.unwrap();
        assert_eq!(key, "prefix:ccc");

        // Empty — nothing left
        let result = kv.claim("prefix:", None, None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_sqlite_claim_with_ceiling() {
        let kv = SQLiteKV::new(":memory:").unwrap();

        // Simulate job pending keys with timestamps:
        // "now" jobs and "future" jobs
        kv.set(
            "jobs:pending:00000000000000001000:id-1",
            &Value::String("id-1".to_string()),
            None,
        )
        .unwrap();
        kv.set(
            "jobs:pending:00000000000000002000:id-2",
            &Value::String("id-2".to_string()),
            None,
        )
        .unwrap();
        kv.set(
            "jobs:pending:00000000000000009000:id-future",
            &Value::String("id-future".to_string()),
            None,
        )
        .unwrap();

        // Ceiling at timestamp 3000 — should only claim jobs at or before 3000
        let ceiling = "jobs:pending:00000000000000003000:~";
        let result = kv.claim("jobs:pending:", None, Some(ceiling)).unwrap();
        assert!(result.is_some());
        let (key, _) = result.unwrap();
        assert!(key.contains("id-1"));

        // Second claim — id-2 is still before ceiling
        let result = kv.claim("jobs:pending:", None, Some(ceiling)).unwrap();
        assert!(result.is_some());
        let (key, _) = result.unwrap();
        assert!(key.contains("id-2"));

        // Third claim — id-future is AFTER ceiling, should not be claimed
        let result = kv.claim("jobs:pending:", None, Some(ceiling)).unwrap();
        assert!(result.is_none());

        // Verify future job is still in the store
        let keys = kv.list(Some("jobs:pending:")).unwrap();
        assert_eq!(keys.len(), 1);
        assert!(keys[0].contains("id-future"));
    }

    #[test]
    fn test_sqlite_claim_ceiling_empty_store() {
        let kv = SQLiteKV::new(":memory:").unwrap();
        let result = kv
            .claim("jobs:pending:", None, Some("jobs:pending:99999:~"))
            .unwrap();
        assert!(result.is_none());
    }
}
