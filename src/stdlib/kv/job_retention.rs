//! Private durable-job storage. Never uses kv_list. Upgraded writers must all use
//! snapshot CAS; old binaries/direct std/kv writes cannot honor this protocol.
use super::*;
use crate::stdlib::jobs::retention::Policy;
use rusqlite::OptionalExtension;
use sha2::{Digest, Sha256};

pub(crate) mod maintenance;

const DAY_MS: i64 = 86_400_000;
fn storage_error<E>(_: E) -> IntentError {
    IntentError::runtime_error("durable job retention storage operation failed")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    raw: String,
    kind: String,
}
impl Snapshot {
    pub fn value(&self) -> Value {
        if let Some(hint) = self.kind.strip_prefix("redis:") {
            deserialize_value_envelope(&self.raw, if hint.is_empty() { None } else { Some(hint) })
        } else {
            deserialize_value(&self.raw, &self.kind)
        }
    }
    fn fingerprint(&self) -> String {
        format!(
            "{:x}",
            Sha256::digest(format!("{}\0{}", self.kind, self.raw))
        )
    }
}

#[derive(Clone, Debug)]
struct Terminal {
    category: i64,
    time: i64,
    fingerprint: String,
    bytes: i64,
}
fn terminal(key: &str, snap: &Snapshot, now: i64) -> Option<Terminal> {
    let Value::Map(m) = snap.value() else {
        return None;
    };
    let Value::String(id) = m.get("id")? else {
        return None;
    };
    if key != format!("jobs:data:{id}") {
        return None;
    }
    let Value::String(status) = m.get("status")? else {
        return None;
    };
    let (category, field) = match status.as_str() {
        "completed" => (0, "completed_at"),
        "cancelled" => (0, "cancelled_at"),
        "dead" => (1, "dead_at"),
        "failed" => (1, "failed_at"),
        "expired" => (1, "expired_at"),
        _ => return None,
    };
    let time = match m.get("_retention_terminal_at").or_else(|| m.get(field)) {
        Some(Value::String(s)) if s.bytes().all(|c| c.is_ascii_digit()) => s
            .parse::<u128>()
            .ok()
            .and_then(|n| i64::try_from(n / 1_000_000).ok()),
        _ => None,
    }
    .filter(|t| *t <= now)
    .unwrap_or(now);
    Some(Terminal {
        category,
        time,
        fingerprint: snap.fingerprint(),
        bytes: snap.raw.len() as i64,
    })
}

fn field(m: &HashMap<String, Value>, name: &str) -> String {
    match m.get(name) {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}
#[derive(Default)]
struct Related {
    id: String,
    pending: String,
    dedup: String,
    live: bool,
}
fn related(key: &str, snap: &Snapshot) -> Related {
    let Value::Map(m) = snap.value() else {
        return Related::default();
    };
    let id = field(&m, "id");
    if id.is_empty() || key != format!("jobs:data:{id}") {
        return Related::default();
    }
    let pk = field(&m, "pending_key");
    let parts: Vec<_> = pk.split(':').collect();
    let pending = if parts.len() == 5
        && parts[0] == "jobs"
        && parts[1] == "pending"
        && parts[2].len() == 2
        && parts[2].bytes().all(|c| c.is_ascii_digit())
        && !parts[3].is_empty()
        && parts[3].bytes().all(|c| c.is_ascii_digit())
        && parts[4] == id
    {
        pk
    } else {
        String::new()
    };
    let dk = field(&m, "dedup_key");
    let dedup = if dk.starts_with("jobs:unique:") {
        dk
    } else {
        String::new()
    };
    Related {
        id,
        pending,
        dedup,
        live: matches!(
            field(&m, "status").as_str(),
            "pending" | "scheduled" | "retrying"
        ),
    }
}
const CALLBACK_TYPES: [&str; 3] = ["on_success", "on_complete", "on_death"];
pub(crate) fn callback_key(batch: &str, kind: &str) -> String {
    format!("__ntnt:jobs-retention:v1:callback:{batch}:{kind}")
}
fn callback(s: &Snapshot) -> Option<(String, String)> {
    let Value::Map(m) = s.value() else {
        return None;
    };
    if field(&m, "type") != "_BatchCallback" {
        return None;
    }
    let Some(Value::Map(payload)) = m.get("payload") else {
        return None;
    };
    let batch = field(payload, "batch_id");
    let kind = field(payload, "callback_type");
    if batch.is_empty()
        || !CALLBACK_TYPES.contains(&kind.as_str())
        || field(&m, "id") != format!("cb-{batch}-{kind}")
    {
        return None;
    }
    Some((batch, kind))
}
fn sql_callback(c: &Connection, s: &Snapshot, creating: bool) -> Result<()> {
    if let Some((batch, kind)) = callback(s) {
        let guard = callback_key(&batch, &kind);
        if creating && sql_snapshot(c, &guard)?.is_some() {
            return Err(IntentError::runtime_error("callback already enqueued"));
        }
        let expiry: Option<Option<i64>> = c
            .query_row(
                "SELECT expires_at FROM _kv WHERE key=? AND (expires_at IS NULL OR expires_at>?)",
                params![format!("jobs:batch:{batch}"), now_unix()],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage_error)?;
        if let Some(expiry) = expiry {
            c.execute("INSERT INTO _kv(key,value,type,expires_at) VALUES(?,'true','bool',?) ON CONFLICT(key) DO UPDATE SET expires_at=excluded.expires_at",params![guard,expiry]).map_err(storage_error)?;
        } else if creating {
            return Err(IntentError::runtime_error(
                "callback batch no longer exists",
            ));
        }
    }
    Ok(())
}
/// Batch-lifetime guards (at most three small keys per batch) are coordination,
/// not terminal history. Their expiry follows every upgraded batch-meta write.
pub(crate) fn set_batch_meta(
    handle: &Value,
    key: &str,
    value: &Value,
    ttl: Option<i64>,
) -> Result<()> {
    let batch = key
        .strip_prefix("jobs:batch:")
        .filter(|b| !b.is_empty() && !b.contains(':'))
        .ok_or_else(|| storage_error(()))?;
    match get_backend_type(handle)? {
        KVBackend::SQLite=>{
            let store=get_sqlite_kv(handle)?;let mut store=store.lock().map_err(storage_error)?;
            let tx=store.conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate).map_err(storage_error)?;
            let (raw,kind)=serialize_value(value)?;let expiry=ttl.map(|t|now_unix()+t);
            tx.execute("INSERT INTO _kv(key,value,type,expires_at) VALUES(?,?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,type=excluded.type,expires_at=excluded.expires_at",params![key,raw,kind,expiry]).map_err(storage_error)?;
            for kind in CALLBACK_TYPES {tx.execute("UPDATE _kv SET expires_at=? WHERE key=?",params![expiry,callback_key(batch,kind)]).map_err(storage_error)?;}
            tx.commit().map_err(storage_error)
        }
        KVBackend::Redis=>redis_op(handle,serde_json::json!({"op":"batch_meta","key":key,"batch":batch,"raw":serialize_value_envelope(value)?,"ttl":ttl})).map(|_|()),
    }
}
fn sql_owned_delete(c: &Connection, key: &str, id: &str) -> Result<()> {
    if !key.is_empty() {
        c.execute(
            "DELETE FROM _kv WHERE key=? AND value=? AND type='string'",
            params![key, id],
        )
        .map_err(storage_error)?;
    }
    Ok(())
}
fn sql_related(
    c: &Connection,
    key: &str,
    old: Option<&Snapshot>,
    new: Option<&Snapshot>,
    prune: bool,
) -> Result<()> {
    if let Some(s) = old {
        if prune {
            sql_callback(c, s, false)?;
        }
        let r = related(key, s);
        sql_owned_delete(c, &r.pending, &r.id)?;
        if prune && !r.dedup.is_empty() {
            let retired = serde_json::json!({"__ntnt_retired_job":r.id}).to_string();
            // Preserve the exact absolute expiry; only replace the matching owner.
            c.execute(
                "UPDATE _kv SET value=?,type='map' WHERE key=? AND value=? AND type='string'",
                params![retired, r.dedup, r.id],
            )
            .map_err(storage_error)?;
        }
    }
    if let Some(s) = new {
        let r = related(key, s);
        let Value::Map(m) = s.value() else {
            return Err(storage_error(()));
        };
        let status = field(&m, "status");
        let active = format!("jobs:active:{}", r.id);
        if status == "active" {
            c.execute("INSERT INTO _kv(key,value,type,expires_at) VALUES(?,?,'string',?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,type='string',expires_at=excluded.expires_at",params![active,r.id,now_unix()+300]).map_err(storage_error)?;
        } else {
            sql_owned_delete(c, &active, &r.id)?;
        }
        if matches!(status.as_str(), "dead" | "cancelled" | "expired" | "failed") {
            sql_owned_delete(c, &r.dedup, &r.id)?;
        }

        if r.live && !r.pending.is_empty() {
            let owner = sql_snapshot(c, &r.pending)?;
            if owner
                .as_ref()
                .is_some_and(|s| !matches!(s.value(),Value::String(ref id) if id==&r.id))
            {
                return Err(storage_error(()));
            }
            c.execute("INSERT INTO _kv(key,value,type) VALUES(?,?,'string') ON CONFLICT(key) DO UPDATE SET value=excluded.value,type='string',expires_at=NULL",params![r.pending,r.id]).map_err(storage_error)?;
        }
    }
    Ok(())
}

fn schema(c: &Connection) -> Result<()> {
    c.execute_batch("CREATE TABLE IF NOT EXISTS _jobs_retention_v1 (
        key TEXT PRIMARY KEY, category INTEGER NOT NULL, time INTEGER NOT NULL,
        fingerprint TEXT NOT NULL, bytes INTEGER NOT NULL);
        CREATE INDEX IF NOT EXISTS _jobs_retention_v1_age ON _jobs_retention_v1(category,time,key);
        CREATE INDEX IF NOT EXISTS _jobs_retention_v1_oldest ON _jobs_retention_v1(time,key);
        CREATE TABLE IF NOT EXISTS _jobs_retention_meta_v1 (
            id INTEGER PRIMARY KEY CHECK(id=1), records INTEGER NOT NULL DEFAULT 0,
            bytes INTEGER NOT NULL DEFAULT 0, cursor TEXT NOT NULL DEFAULT 'jobs:data:',
            complete INTEGER NOT NULL DEFAULT 0, policy TEXT);
        INSERT OR IGNORE INTO _jobs_retention_meta_v1(id) VALUES(1);
        CREATE TRIGGER IF NOT EXISTS _jobs_retention_v1_insert AFTER INSERT ON _jobs_retention_v1 BEGIN
          UPDATE _jobs_retention_meta_v1 SET records=records+1,bytes=bytes+NEW.bytes WHERE id=1; END;
        CREATE TRIGGER IF NOT EXISTS _jobs_retention_v1_delete AFTER DELETE ON _jobs_retention_v1 BEGIN
          UPDATE _jobs_retention_meta_v1 SET records=records-1,bytes=bytes-OLD.bytes WHERE id=1; END;
        CREATE TRIGGER IF NOT EXISTS _jobs_retention_v1_update AFTER UPDATE ON _jobs_retention_v1 BEGIN
          UPDATE _jobs_retention_meta_v1 SET bytes=bytes+NEW.bytes-OLD.bytes WHERE id=1; END;") .map_err(storage_error)
}
fn sql_snapshot(c: &Connection, key: &str) -> Result<Option<Snapshot>> {
    c.query_row(
        "SELECT value,type FROM _kv WHERE key=? AND (expires_at IS NULL OR expires_at>?)",
        params![key, now_unix()],
        |r| {
            Ok(Snapshot {
                raw: r.get(0)?,
                kind: r.get(1)?,
            })
        },
    )
    .optional()
    .map_err(storage_error)
}
fn sql_index(c: &Connection, key: &str, snap: Option<&Snapshot>, now: i64) -> Result<()> {
    if let Some(t) = snap.and_then(|s| terminal(key, s, now)) {
        // Identical backfill snapshots preserve conservative first-observed time.
        c.execute(
            "INSERT INTO _jobs_retention_v1(key,category,time,fingerprint,bytes) VALUES(?,?,?,?,?)
            ON CONFLICT(key) DO UPDATE SET category=excluded.category,
              time=CASE WHEN fingerprint=excluded.fingerprint THEN time ELSE excluded.time END,
              fingerprint=excluded.fingerprint,bytes=excluded.bytes",
            params![key, t.category, t.time, t.fingerprint, t.bytes],
        )
        .map_err(storage_error)?;
    } else {
        c.execute("DELETE FROM _jobs_retention_v1 WHERE key=?", [key])
            .map_err(storage_error)?;
    }
    Ok(())
}
fn sql_maintain(store: &mut SQLiteKV, p: &Policy, now: i64) -> Result<usize> {
    schema(&store.conn)?;
    let tx = store
        .conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let stored_policy: Option<String> = tx
        .query_row(
            "SELECT policy FROM _jobs_retention_meta_v1 WHERE id=1",
            [],
            |r| r.get(0),
        )
        .map_err(storage_error)?;
    if let Some(raw) = stored_policy {
        let json: serde_json::Value = serde_json::from_str(&raw).map_err(storage_error)?;
        if Policy::parse(Some(&json_to_value_public(&json)))? != *p {
            return Ok(0); // A successful reconfiguration fences stale batches.
        }
    }
    let (cursor, complete): (String, bool) = tx
        .query_row(
            "SELECT cursor,complete FROM _jobs_retention_meta_v1 WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(storage_error)?;
    if !complete {
        let page = {
            let mut q = tx
                .prepare(
                    "SELECT key FROM _kv WHERE key>? AND key<'jobs:data;' ORDER BY key LIMIT ?",
                )
                .map_err(storage_error)?;
            let rows = q
                .query_map(params![cursor, p.batch_size as i64], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(storage_error)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(storage_error)?
        };
        for key in &page {
            let s = sql_snapshot(&tx, key)?;
            sql_index(&tx, key, s.as_ref(), now)?;
        }
        tx.execute(
            "UPDATE _jobs_retention_meta_v1 SET cursor=?,complete=? WHERE id=1",
            params![page.last().unwrap_or(&cursor), page.len() < p.batch_size],
        )
        .map_err(storage_error)?;
    }
    let (count, bytes, indexed_complete): (i64, i64, bool) = tx
        .query_row(
            "SELECT records,bytes,complete FROM _jobs_retention_meta_v1 WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .map_err(storage_error)?;
    // Unseen legacy records can be older than every indexed candidate.
    // Age expiry is safe during migration; capacity eviction waits for ordering.
    let pressure = indexed_complete && (count > p.max_records || bytes > p.max_bytes);
    let mut candidates = Vec::new();
    if pressure {
        let mut q = tx
            .prepare(
                "SELECT key,time,fingerprint FROM _jobs_retention_v1 ORDER BY time,key LIMIT ?",
            )
            .map_err(storage_error)?;
        for r in q
            .query_map([p.batch_size as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(storage_error)?
        {
            candidates.push(r.map_err(storage_error)?);
        }
    } else {
        for (cat, days) in [(0, p.completed_days), (1, p.failed_days)] {
            let mut q=tx.prepare("SELECT key,time,fingerprint FROM _jobs_retention_v1 WHERE category=? AND time<=? ORDER BY time,key LIMIT ?").map_err(storage_error)?;
            for r in q
                .query_map(
                    params![cat, now.saturating_sub(days * DAY_MS), p.batch_size as i64],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .map_err(storage_error)?
            {
                candidates.push(r.map_err(storage_error)?);
            }
        }
        candidates.sort_by(|a, b| (&a.1, &a.0).cmp(&(&b.1, &b.0)));
        candidates.truncate(p.batch_size);
    }
    let mut deleted = 0;
    for (key, time, fp) in candidates {
        let snap = sql_snapshot(&tx, &key)?;
        let valid = snap.as_ref().and_then(|s| terminal(&key, s, now));
        if let Some(t) = valid {
            if t.fingerprint == fp {
                let (count, bytes): (i64, i64) = tx
                    .query_row(
                        "SELECT records,bytes FROM _jobs_retention_meta_v1 WHERE id=1",
                        [],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(storage_error)?;
                let days = if t.category == 0 {
                    p.completed_days
                } else {
                    p.failed_days
                };
                if time <= now.saturating_sub(days * DAY_MS)
                    || (indexed_complete && (count > p.max_records || bytes > p.max_bytes))
                {
                    sql_related(&tx, &key, snap.as_ref(), None, true)?;
                    tx.execute("DELETE FROM _kv WHERE key=?", [&key])
                        .map_err(storage_error)?;
                    sql_index(&tx, &key, None, now)?;
                    deleted += 1;
                }
                continue;
            }
        }
        sql_index(&tx, &key, snap.as_ref(), now)?;
    }
    tx.commit().map_err(storage_error)?;
    Ok(deleted)
}

pub(crate) fn policy(handle: &Value, new: Option<&Policy>) -> Result<Policy> {
    let defaults = Policy::default();
    let raw = match get_backend_type(handle)? {
        KVBackend::SQLite => {
            let store = get_sqlite_kv(handle)?;
            let mut store = store.lock().map_err(storage_error)?;
            schema(&store.conn)?;
            let tx = store
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            let old: Option<String> = tx
                .query_row(
                    "SELECT policy FROM _jobs_retention_meta_v1 WHERE id=1",
                    [],
                    |r| r.get(0),
                )
                .map_err(storage_error)?;
            let raw = match (new, old) {
                (None, Some(raw)) => raw,
                (replacement, _) => {
                    let raw = serde_json::to_string(replacement.unwrap_or(&defaults))
                        .map_err(storage_error)?;
                    tx.execute(
                        "UPDATE _jobs_retention_meta_v1 SET policy=? WHERE id=1",
                        [&raw],
                    )
                    .map_err(storage_error)?;
                    raw
                }
            };
            tx.commit().map_err(storage_error)?;
            raw
        }
        KVBackend::Redis => redis_op(
            handle,
            serde_json::json!({"op":"policy","replace":new.is_some(),"policy":new.unwrap_or(&defaults)}),
        )?,
    };
    let json: serde_json::Value = serde_json::from_str(&raw).map_err(storage_error)?;
    Policy::parse(Some(&json_to_value_public(&json)))
}

pub(crate) fn has_work(handle: &Value, p: &Policy, now: i64) -> Result<bool> {
    if !p.enabled {
        return Ok(false);
    }
    match get_backend_type(handle)? {
        KVBackend::SQLite => {
            let store = get_sqlite_kv(handle)?;
            let store = store.lock().map_err(storage_error)?;
            let (complete, count, bytes): (bool, i64, i64) = store
                .conn
                .query_row(
                    "SELECT complete,records,bytes FROM _jobs_retention_meta_v1 WHERE id=1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .map_err(storage_error)?;
            if !complete || count > p.max_records || bytes > p.max_bytes {
                return Ok(true);
            }
            for (category, days) in [(0, p.completed_days), (1, p.failed_days)] {
                if store
                    .conn
                    .query_row(
                        "SELECT 1 FROM _jobs_retention_v1 WHERE category=? AND time<=? LIMIT 1",
                        params![category, now.saturating_sub(days * DAY_MS)],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(storage_error)?
                    .is_some()
                {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        KVBackend::Redis => Ok(redis_op(
            handle,
            serde_json::json!({"op":"has_work","policy":p,"now":now}),
        )? == "1"),
    }
}

pub(crate) fn maintain(handle: &Value, policy: &Policy, now_ms: i64) -> Result<usize> {
    if !policy.enabled {
        return Ok(0);
    }
    match get_backend_type(handle)? {
        KVBackend::SQLite => sql_maintain(
            &mut *get_sqlite_kv(handle)?.lock().map_err(storage_error)?,
            policy,
            now_ms,
        ),
        KVBackend::Redis => redis_op(
            handle,
            serde_json::json!({"op":"maintain","policy":policy,"now":now_ms}),
        )?
        .parse()
        .map_err(storage_error),
    }
}

pub(crate) fn remove(handle: &Value, key: &str, expected: &Snapshot, now: i64) -> Result<bool> {
    match get_backend_type(handle)? {
        KVBackend::SQLite => {
            let store = get_sqlite_kv(handle)?;
            let mut store = store.lock().map_err(storage_error)?;
            schema(&store.conn)?;
            let tx = store
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            if sql_snapshot(&tx, key)?.as_ref() != Some(expected) {
                return Ok(false);
            }
            sql_related(&tx, key, Some(expected), None, true)?;
            tx.execute("DELETE FROM _kv WHERE key=?", [key])
                .map_err(storage_error)?;
            sql_index(&tx, key, None, now)?;
            tx.commit().map_err(storage_error)?;
            Ok(true)
        }
        KVBackend::Redis => Ok(redis_op(
            handle,
            serde_json::json!({"op":"remove","key":key,"expected":{"raw":expected.raw,"kind":expected.kind.strip_prefix("redis:").unwrap_or("")},"now":now}),
        )? != "-1"),
    }
}
pub(crate) fn release_unique(handle: &Value, key: &str, id: &str) -> Result<()> {
    if !key.starts_with("jobs:unique:") {
        return Err(storage_error(()));
    }
    match get_backend_type(handle)? {
        KVBackend::SQLite => {
            let store = get_sqlite_kv(handle)?;
            let mut store = store.lock().map_err(storage_error)?;
            let tx = store
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            if let Some(s) = sql_snapshot(&tx, &format!("jobs:data:{id}"))? {
                if let Value::Map(m) = s.value() {
                    if matches!(
                        field(&m, "status").as_str(),
                        "dead" | "cancelled" | "expired" | "failed"
                    ) {
                        sql_owned_delete(&tx, key, id)?;
                    }
                }
            }
            tx.commit().map_err(storage_error)
        }
        KVBackend::Redis => redis_op(
            handle,
            serde_json::json!({"op":"release_unique","key":key,"id":id}),
        )
        .map(|_| ()),
    }
}

fn redis_op(handle: &Value, args: serde_json::Value) -> Result<String> {
    let store = get_redis_kv(handle)?;
    let mut store = store.lock().map_err(storage_error)?;
    redis::Script::new(include_str!("job_retention.lua"))
        .arg(args.to_string())
        .invoke(&mut store.conn)
        .map_err(storage_error)
}

pub(crate) fn snapshot(handle: &Value, key: &str) -> Result<Option<Snapshot>> {
    match get_backend_type(handle)? {
        KVBackend::SQLite => sql_snapshot(
            &get_sqlite_kv(handle)?.lock().map_err(storage_error)?.conn,
            key,
        ),
        KVBackend::Redis => {
            let data: serde_json::Value = serde_json::from_str(&redis_op(
                handle,
                serde_json::json!({"op":"snapshot","key":key}),
            )?)
            .map_err(storage_error)?;
            Ok(data["raw"].as_str().map(|raw| Snapshot {
                raw: raw.into(),
                kind: format!("redis:{}", data["hint"].as_str().unwrap_or("")),
            }))
        }
    }
}
pub(crate) struct PreparedChange {
    key: String,
    expected: Option<Snapshot>,
    replacement: Snapshot,
    now: i64,
}
impl PreparedChange {
    pub(crate) fn replacement(&self) -> &Snapshot {
        &self.replacement
    }
}

pub(crate) fn prepare_change(
    handle: &Value,
    key: &str,
    expected: Option<&Snapshot>,
    value: &Value,
    now: i64,
) -> Result<PreparedChange> {
    let Value::Map(mut map) = value.clone() else {
        return Err(storage_error(()));
    };
    let id = field(&map, "id");
    if id.is_empty() || key != format!("jobs:data:{id}") {
        return Err(storage_error(()));
    }
    map.insert(
        "_job_revision".into(),
        Value::String(uuid::Uuid::new_v4().to_string()),
    );
    let old_status = expected.and_then(|s| match s.value() {
        Value::Map(m) => Some(field(&m, "status")),
        _ => None,
    });
    if old_status.as_deref() != Some(field(&map, "status").as_str()) {
        map.remove("_retention_terminal_at");
        if matches!(
            field(&map, "status").as_str(),
            "completed" | "cancelled" | "dead" | "failed" | "expired"
        ) {
            map.insert(
                "_retention_terminal_at".into(),
                Value::String(format!("{}", now as i128 * 1_000_000)),
            );
        }
    }
    let value = &Value::Map(map);
    let (raw, kind) = if get_backend_type(handle)? == KVBackend::Redis {
        (serialize_value_envelope(value)?, "redis:".to_string())
    } else {
        serialize_value(value)?
    };
    let replacement = Snapshot { raw, kind };
    Ok(PreparedChange {
        key: key.into(),
        expected: expected.cloned(),
        replacement,
        now,
    })
}

pub(crate) fn commit_change(handle: &Value, prepared: &PreparedChange) -> Result<Snapshot> {
    commit_change_inner(handle, prepared, false)
}
pub(crate) fn commit_change_recovering(
    handle: &Value,
    prepared: &PreparedChange,
) -> Result<Snapshot> {
    commit_change_inner(handle, prepared, true)
}
fn commit_change_inner(
    handle: &Value,
    prepared: &PreparedChange,
    reconnect: bool,
) -> Result<Snapshot> {
    let key = prepared.key.as_str();
    let expected = prepared.expected.as_ref();
    let replacement = &prepared.replacement;
    let now = prepared.now;
    match get_backend_type(handle)? {
        KVBackend::SQLite => {
            let store = get_sqlite_kv(handle)?;
            let mut store = store.lock().map_err(storage_error)?;
            schema(&store.conn)?;
            let tx = store
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            let current = sql_snapshot(&tx, key)?;
            // A lost acknowledgement is not a new logical transition. The
            // prepared revision is stable across retries and unique to its owner.
            if current.as_ref() == Some(replacement) {
                return Ok(replacement.clone());
            }
            if current.as_ref() != expected {
                return Err(IntentError::runtime_error(
                    "job changed concurrently; retry operation",
                ));
            }
            tx.execute("INSERT INTO _kv(key,value,type) VALUES(?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,type=excluded.type,expires_at=NULL",params![key,replacement.raw,replacement.kind]).map_err(storage_error)?;
            if expected.is_none() {
                sql_callback(&tx, replacement, true)?;
            }
            sql_related(&tx, key, expected, Some(replacement), false)?;
            sql_index(&tx, key, Some(replacement), now)?;
            tx.commit().map_err(storage_error)?;
        }
        KVBackend::Redis => {
            let expected=expected.map(|s|serde_json::json!({"raw":s.raw,"kind":s.kind.strip_prefix("redis:").unwrap_or("")}));
            let args = serde_json::json!({"op":"change","key":key,"expected":expected,"raw":replacement.raw,"now":now});
            let result = if reconnect {
                maintenance::command(handle, args)?
            } else {
                redis_op(handle, args)?
            };
            if result == "-1" {
                return Err(IntentError::runtime_error(
                    "job changed concurrently; retry operation",
                ));
            }
        }
    }
    Ok(replacement.clone())
}

pub(crate) fn change(
    handle: &Value,
    key: &str,
    expected: Option<&Snapshot>,
    value: &Value,
    now: i64,
) -> Result<Snapshot> {
    commit_change(handle, &prepare_change(handle, key, expected, value, now)?)
}
#[cfg(test)]
mod backend_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires disposable Redis via NTNT_RETENTION_TEST_REDIS"]
    fn retention_redis_age() {
        let url =
            std::env::var("NTNT_RETENTION_TEST_REDIS").expect("explicit disposable Redis required");
        assert!(url.starts_with("redis://127.0.0.1:"));
        let h = open_kv(&url).unwrap();
        let store = get_redis_kv(&h).unwrap();
        let _: () = redis::cmd("FLUSHDB")
            .query(&mut store.lock().unwrap().conn)
            .unwrap();
        let record = json_to_value_public(
            &serde_json::json!({"id":"age","status":"completed","completed_at":"0"}),
        );
        kv_set(&h, "jobs:data:age", &record, None).unwrap();
        assert_eq!(maintain(&h, &Policy::default(), 31 * DAY_MS).unwrap(), 1);
    }
    #[test]
    fn retention_callback_guard_survives_history_prune() {
        let h = open_kv("sqlite::memory:").unwrap();
        kv_set(
            &h,
            "jobs:batch:b",
            &json_to_value_public(&serde_json::json!({"id":"b"})),
            Some(86400),
        )
        .unwrap();
        let v = json_to_value_public(
            &serde_json::json!({"id":"cb-b-on_complete","type":"_BatchCallback","status":"completed","completed_at":"0","payload":{"batch_id":"b","callback_type":"on_complete"}}),
        );
        change(&h, "jobs:data:cb-b-on_complete", None, &v, 0).unwrap();
        assert_eq!(maintain(&h, &Policy::default(), 31 * DAY_MS).unwrap(), 1);
        assert!(
            change(&h, "jobs:data:cb-b-on_complete", None, &v, 31 * DAY_MS).is_err(),
            "callback must not reenqueue after history removal"
        );
    }
    #[test]
    fn retention_snapshot_detects_aba() {
        let h = open_kv("sqlite::memory:").unwrap();
        let value = json_to_value_public(&serde_json::json!({"id":"aba","status":"pending"}));
        let first = change(&h, "jobs:data:aba", None, &value, 0).unwrap();
        change(&h, "jobs:data:aba", Some(&first), &value, 1).unwrap();
        assert!(change(&h, "jobs:data:aba", Some(&first), &value, 2).is_err());
    }
    #[test]
    fn retention_transition_atomically_schedules_and_retires_dedup() {
        let h = open_kv("sqlite::memory:").unwrap();
        let pending = json_to_value_public(
            &serde_json::json!({"id":"owned","status":"pending","pending_key":"jobs:pending:50:0:owned","dedup_key":"jobs:unique:owned"}),
        );
        let s = change(&h, "jobs:data:owned", None, &pending, 0).unwrap();
        assert!(
            matches!(
                kv_get(&h, "jobs:pending:50:0:owned").unwrap(),
                Value::String(_)
            ),
            "record and queue key must be atomic"
        );
        kv_set(
            &h,
            "jobs:unique:owned",
            &Value::String("owned".into()),
            Some(86400),
        )
        .unwrap();
        let Value::Map(mut completed) = pending else {
            panic!()
        };
        completed.insert("status".into(), Value::String("completed".into()));
        completed.insert("completed_at".into(), Value::String("0".into()));
        change(&h, "jobs:data:owned", Some(&s), &Value::Map(completed), 0).unwrap();
        assert_eq!(maintain(&h, &Policy::default(), 31 * DAY_MS).unwrap(), 1);
        assert!(
            matches!(kv_get(&h, "jobs:unique:owned").unwrap(), Value::Map(_)),
            "prune must preserve the uniqueness reservation even without a job record"
        );
        assert!(kv_ttl(&h, "jobs:unique:owned").unwrap().unwrap() > 86000);
    }
    #[test]
    fn retention_cas_prevents_missing_record_resurrection() {
        let h = open_kv("sqlite::memory:").unwrap();
        let dead =
            json_to_value_public(&serde_json::json!({"id":"race","status":"dead","dead_at":"0"}));
        let s = change(&h, "jobs:data:race", None, &dead, 0).unwrap();
        assert_eq!(maintain(&h, &Policy::default(), 100 * DAY_MS).unwrap(), 1);
        let retry = json_to_value_public(
            &serde_json::json!({"id":"race","status":"pending","pending_key":"jobs:pending:50:0:race"}),
        );
        assert!(
            change(&h, "jobs:data:race", Some(&s), &retry, 100 * DAY_MS).is_err(),
            "stale retry must not resurrect pruned data"
        );
        assert!(matches!(
            kv_get(&h, "jobs:pending:50:0:race").unwrap(),
            Value::Unit
        ));
    }
    #[test]
    fn retention_sqlite_age_uses_terminal_timestamp() {
        let handle = open_kv("sqlite::memory:").unwrap();
        let record = json_to_value_public(
            &serde_json::json!({"id":"age", "status":"completed", "created_at":"0", "completed_at":"86400000000000"}),
        );
        kv_set(&handle, "jobs:data:age", &record, None).unwrap();
        let p = Policy::default();
        assert_eq!(maintain(&handle, &p, 32 * 86400000).unwrap(), 1);
        assert!(matches!(
            kv_get(&handle, "jobs:data:age").unwrap(),
            Value::Unit
        ));
    }
}
