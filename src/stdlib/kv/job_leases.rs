//! Durable job ownership. Ready removal, primary state, lease and due index
//! share one transaction; expiry never makes executing work replayable.
use super::*;
use conditional::Snapshot;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

type R<T> = redis::RedisResult<T>;
const DUE: &str = "jobs:lease_due";
const DUE_PREFIX: &str = "jobs:lease_due:";
pub(super) const READY: &str = "jobs:ready";
const READY_INITIALIZED: &str = "jobs:ready:initialized";
const READY_MIGRATION_LOCK: &str = "jobs:ready:migration_lock";
const READY_MIGRATION_LOCK_MS: usize = 5_000;
const READY_SENTINEL: &str = "__ntnt_ready_index__";
const MAX_STALE_BATCHES_PER_CLAIM: usize = 4;
const MAX_RECOVERY: usize = 256;

pub(super) fn is_pending_key(key: &str) -> bool {
    key.starts_with("jobs:pending:") && !key.ends_with(":__type")
}

pub(super) fn add_ready(tx: &mut redis::Pipeline, key: &str) {
    if is_pending_key(key) {
        tx.cmd("ZADD").arg(READY).arg(0).arg(key).ignore();
    }
}

pub(super) fn remove_ready(tx: &mut redis::Pipeline, key: &str) {
    if is_pending_key(key) {
        tx.cmd("ZREM").arg(READY).arg(key).ignore();
    }
}

pub(super) fn watch_ready_index(conn: &mut redis::Connection) -> R<()> {
    redis::cmd("WATCH").arg(READY).query::<()>(conn)?;
    let kind: String = redis::cmd("TYPE").arg(READY).query(conn)?;
    if kind != "none" && kind != "zset" {
        let _ = redis::cmd("UNWATCH").query::<()>(conn);
        return Err(redis::RedisError::from((
            redis::ErrorKind::TypeError,
            "job ready index has the wrong Redis type",
        )));
    }
    Ok(())
}

fn ready_index_initialized(conn: &mut redis::Connection) -> R<bool> {
    if !redis::cmd("EXISTS")
        .arg(READY_INITIALIZED)
        .query::<bool>(conn)?
    {
        return Ok(false);
    }
    let kind: String = redis::cmd("TYPE").arg(READY).query(conn)?;
    if kind != "none" && kind != "zset" {
        return Err(redis::RedisError::from((
            redis::ErrorKind::TypeError,
            "job ready index has the wrong Redis type",
        )));
    }
    if kind == "zset"
        && redis::cmd("ZSCORE")
            .arg(READY)
            .arg(READY_SENTINEL)
            .query::<Option<f64>>(conn)?
            .is_some()
    {
        return Ok(true);
    }
    redis::cmd("DEL").arg(READY_INITIALIZED).query::<()>(conn)?;
    Ok(false)
}

pub(super) fn ensure_ready_index(conn: &mut redis::Connection) -> R<()> {
    if ready_index_initialized(conn)? {
        return Ok(());
    }

    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let token = uuid::Uuid::new_v4().to_string();
    'acquire: loop {
        if std::time::Instant::now() >= deadline {
            return Err(redis::RedisError::from((
                redis::ErrorKind::ResponseError,
                "READY_INDEX_BUSY migration did not finish within 30 seconds",
            )));
        }
        let acquired: Option<String> = redis::cmd("SET")
            .arg(READY_MIGRATION_LOCK)
            .arg(&token)
            .arg("NX")
            .arg("PX")
            .arg(READY_MIGRATION_LOCK_MS)
            .query(conn)?;
        if acquired.is_some() {
            break;
        }

        // Multiple worker processes can start together. Observe the elected
        // migrator, and re-elect immediately if its lock disappears.
        loop {
            if ready_index_initialized(conn)? {
                return Ok(());
            }
            if !redis::cmd("EXISTS")
                .arg(READY_MIGRATION_LOCK)
                .query::<bool>(conn)?
            {
                continue 'acquire;
            }
            if std::time::Instant::now() >= deadline {
                return Err(redis::RedisError::from((
                    redis::ErrorKind::ResponseError,
                    "READY_INDEX_BUSY migration did not finish within 30 seconds",
                )));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    let result = (|| {
        let kind: String = redis::cmd("TYPE").arg(READY).query(conn)?;
        if kind != "none" && kind != "zset" {
            return Err(redis::RedisError::from((
                redis::ErrorKind::TypeError,
                "job ready index has the wrong Redis type",
            )));
        }
        let add_live = redis::Script::new(
            r#"
                local kind = redis.call('TYPE', KEYS[1])
                if type(kind) == 'table' then kind = kind['ok'] end
                if kind ~= 'none' and kind ~= 'zset' then
                    return redis.error_reply('WRONGTYPE job ready index must be a sorted set')
                end
                for i = 1, #ARGV do
                    if redis.call('EXISTS', ARGV[i]) == 1 then
                        redis.call('ZADD', KEYS[1], 0, ARGV[i])
                    end
                end
                return 1
            "#,
        );
        let mut cursor = 0u64;
        loop {
            let renewed: i32 = redis::Script::new(
                "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('PEXPIRE', KEYS[1], ARGV[2]) else return 0 end",
            )
            .key(READY_MIGRATION_LOCK)
            .arg(&token)
            .arg(READY_MIGRATION_LOCK_MS)
            .invoke(conn)?;
            if renewed == 0 {
                return Err(redis::RedisError::from((
                    redis::ErrorKind::ResponseError,
                    "READY_INDEX_BUSY migration lock was lost",
                )));
            }
            let (next, mut batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("jobs:pending:*")
                .arg("COUNT")
                .arg(256)
                .query(conn)?;
            batch.retain(|key| is_pending_key(key));
            if !batch.is_empty() {
                let mut invocation = add_live.prepare_invoke();
                invocation.key(READY);
                for key in batch {
                    invocation.arg(key);
                }
                invocation.invoke::<i32>(conn)?;
            }
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        let published: i32 = redis::Script::new(
            r#"
                if redis.call('GET', KEYS[1]) ~= ARGV[1] then return 0 end
                redis.call('ZADD', KEYS[2], 0, ARGV[2])
                redis.call('SET', KEYS[3], '1')
                return 1
            "#,
        )
        .key(READY_MIGRATION_LOCK)
        .key(READY)
        .key(READY_INITIALIZED)
        .arg(&token)
        .arg(READY_SENTINEL)
        .invoke(conn)?;
        if published == 0 {
            return Err(redis::RedisError::from((
                redis::ErrorKind::ResponseError,
                "READY_INDEX_BUSY migration lock was lost",
            )));
        }
        Ok(())
    })();
    let _ = redis::Script::new(
        "if redis.call('GET', KEYS[1]) == ARGV[1] then return redis.call('DEL', KEYS[1]) else return 0 end",
    )
    .key(READY_MIGRATION_LOCK)
    .arg(token)
    .invoke::<i32>(conn);
    result
}

fn ready_candidate(conn: &mut redis::Connection, floor: &str, ceiling: &str) -> R<Option<String>> {
    const RETRY: &str = "__ntnt_ready_retry__";
    let script = redis::Script::new(
        r#"
            local candidates = redis.call('ZRANGEBYLEX', KEYS[1], ARGV[1], ARGV[2], 'LIMIT', 0, 256)
            for _, key in ipairs(candidates) do
                if redis.call('EXISTS', key) == 1 then return {key} end
                redis.call('ZREM', KEYS[1], key)
            end
            if #candidates == 256 then return {ARGV[3]} end
            return {}
        "#,
    );
    for _ in 0..MAX_STALE_BATCHES_PER_CLAIM {
        let result: Vec<String> = script
            .key(READY)
            .arg(format!("[{floor}"))
            .arg(format!("[{ceiling}"))
            .arg(RETRY)
            .invoke(conn)?;
        match result.first().map(String::as_str) {
            Some(RETRY) => continue,
            candidate => return Ok(candidate.map(str::to_owned)),
        }
    }
    Ok(None)
}

pub(crate) struct Claim {
    pub snapshot: Snapshot,
    pub data: HashMap<String, Value>,
    pub id: String,
    pub token: String,
    pub deadline_ms: i64,
    pub observed_now_ms: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Lease {
    pub owner: String,
    pub token: String,
    pub phase: String,
    pub deadline_ms: i64,
    pub pending_key: String,
    pub ready_status: String,
}
#[derive(Default, Debug)]
pub(crate) struct RecoveryCounts {
    pub requeued: usize,
    pub unknown: usize,
}
fn err<E>(_: E) -> redis::RedisError {
    redis::RedisError::from((
        redis::ErrorKind::ResponseError,
        "job lease storage operation failed",
    ))
}
fn lease_key(id: &str) -> String {
    format!("jobs:lease:{id}")
}
fn auth_key(id: &str) -> String {
    format!("jobs:lease_auth:{id}")
}
fn data_key(id: &str) -> String {
    format!("jobs:data:{id}")
}
fn due_key(id: &str, lease: &Lease) -> String {
    format!("{DUE_PREFIX}{:020}:{id}:{}", lease.deadline_ms, lease.token)
}
fn text<'a>(data: &'a HashMap<String, Value>, key: &str) -> Option<&'a str> {
    match data.get(key) {
        Some(Value::String(s)) => Some(s),
        _ => None,
    }
}
fn set_text(data: &mut HashMap<String, Value>, key: &str, value: &str) {
    data.insert(key.into(), Value::String(value.into()));
}
fn millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
fn deadline(now: i64, duration: i64) -> R<i64> {
    if duration <= 0 {
        return Err(err(()));
    }
    now.checked_add(duration).ok_or_else(|| err(()))
}

/// All Redis writes are buffered until validation completes. WATCH is allowed
/// to grow during reads, including the shared index and pending-key ownership.
struct Store<'a> {
    sql: Option<&'a Connection>,
    redis: Option<&'a mut redis::Connection>,
    writes: redis::Pipeline,
    index_checked: bool,
    ready_checked: bool,
}
impl<'a> Store<'a> {
    fn sql(conn: &'a Connection) -> Self {
        Self {
            sql: Some(conn),
            redis: None,
            writes: redis::pipe(),
            index_checked: false,
            ready_checked: false,
        }
    }
    fn redis(conn: &'a mut redis::Connection) -> Self {
        let mut writes = redis::pipe();
        writes.atomic();
        Self {
            sql: None,
            redis: Some(conn),
            writes,
            index_checked: false,
            ready_checked: false,
        }
    }
    fn now(&mut self) -> R<i64> {
        if let Some(conn) = self.redis.as_deref_mut() {
            let (sec, micros): (i64, i64) = redis::cmd("TIME").query(conn)?;
            sec.checked_mul(1000)
                .and_then(|s| s.checked_add(micros / 1000))
                .ok_or_else(|| err(()))
        } else {
            Ok(millis())
        }
    }
    fn snapshot(&self, value: &Value) -> R<Snapshot> {
        if self.sql.is_some() {
            let (raw, kind) = serialize_value(value).map_err(err)?;
            Ok(Snapshot {
                raw,
                kind: Some(kind),
                redis: false,
            })
        } else {
            Ok(Snapshot {
                raw: serialize_value_envelope(value).map_err(err)?,
                kind: None,
                redis: true,
            })
        }
    }
    fn read(&mut self, key: &str) -> R<Option<Snapshot>> {
        if let Some(conn) = self.sql {
            return conditional::sql_read(conn, key).map_err(err);
        }
        let conn = self.redis.as_deref_mut().unwrap();
        redis::cmd("WATCH")
            .arg(key)
            .arg(format!("{key}:__type"))
            .query::<()>(conn)?;
        let raw: Option<String> = redis::cmd("GET").arg(key).query(conn)?;
        let kind: Option<String> = redis::cmd("GET").arg(format!("{key}:__type")).query(conn)?;
        Ok(raw.map(|raw| Snapshot {
            raw,
            kind,
            redis: true,
        }))
    }
    fn put(&mut self, key: &str, snapshot: &Snapshot) -> R<()> {
        if let Some(conn) = self.sql {
            conn.execute("INSERT INTO _kv(key,value,type,expires_at) VALUES(?,?,?,NULL) ON CONFLICT(key) DO UPDATE SET value=excluded.value,type=excluded.type,expires_at=NULL", params![key, snapshot.raw, snapshot.kind]).map_err(err)?;
        } else {
            if is_pending_key(key) {
                self.ready_index()?;
            }
            self.writes
                .set(key, &snapshot.raw)
                .ignore()
                .del(format!("{key}:__type"))
                .ignore();
            add_ready(&mut self.writes, key);
        }
        Ok(())
    }
    fn delete(&mut self, key: &str) -> R<()> {
        if let Some(conn) = self.sql {
            conn.execute("DELETE FROM _kv WHERE key=?", [key])
                .map_err(err)?;
        } else {
            if is_pending_key(key) {
                self.ready_index()?;
            }
            self.writes
                .cmd("DEL")
                .arg(key)
                .arg(format!("{key}:__type"))
                .ignore();
            remove_ready(&mut self.writes, key);
        }
        Ok(())
    }
    fn lease(&mut self, id: &str) -> R<Option<Lease>> {
        self.read(&lease_key(id))?
            .map(|s| serde_json::from_value(value_to_json(&s.value())).map_err(err))
            .transpose()
    }
    fn authorized(&mut self, id: &str, token: &str) -> R<bool> {
        let Some(conn) = self.redis.as_deref_mut() else {
            // SQLite serializes deadline validation and writes in one transaction.
            return Ok(true);
        };
        let key = auth_key(id);
        redis::cmd("WATCH").arg(&key).query::<()>(conn)?;
        let live: Option<String> = redis::cmd("GET").arg(&key).query(conn)?;
        Ok(live.as_deref() == Some(token))
    }
    fn index(&mut self) -> R<()> {
        if self.index_checked {
            return Ok(());
        }
        if let Some(conn) = self.redis.as_deref_mut() {
            redis::cmd("WATCH").arg(DUE).query::<()>(conn)?;
            let kind: String = redis::cmd("TYPE").arg(DUE).query(conn)?;
            if kind != "none" && kind != "zset" {
                return Err(err(()));
            }
        }
        self.index_checked = true;
        Ok(())
    }
    fn ready_index(&mut self) -> R<()> {
        if self.ready_checked {
            return Ok(());
        }
        if let Some(conn) = self.redis.as_deref_mut() {
            watch_ready_index(conn)?;
        }
        self.ready_checked = true;
        Ok(())
    }
    fn remove_due(&mut self, member: &str) -> R<()> {
        self.index()?;
        if self.sql.is_some() {
            self.delete(member)?;
        } else {
            self.writes.cmd("ZREM").arg(DUE).arg(member).ignore();
        }
        Ok(())
    }
    fn save_lease(&mut self, id: &str, lease: &Lease) -> R<()> {
        self.index()?;
        let snapshot = self.snapshot(&json_to_value(serde_json::to_value(lease).map_err(err)?))?;
        self.put(&lease_key(id), &snapshot)?;
        if self.sql.is_some() {
            let snapshot = self.snapshot(&Value::String(id.into()))?;
            self.put(&due_key(id, lease), &snapshot)?;
        } else {
            self.writes
                .cmd("ZADD")
                .arg(DUE)
                .arg(lease.deadline_ms)
                .arg(due_key(id, lease))
                .ignore();
            // No TTL on the index, even if an operator previously set one.
            self.writes.cmd("PERSIST").arg(DUE).ignore();
            // Authorization alone expires. WATCH observes its expiry at EXEC,
            // fencing network/queue delays after TIME without losing recovery.
            self.writes
                .cmd("SET")
                .arg(auth_key(id))
                .arg(&lease.token)
                .arg("PXAT")
                .arg(lease.deadline_ms)
                .ignore();
        }
        Ok(())
    }
    fn remove_lease(&mut self, id: &str, lease: &Lease) -> R<()> {
        self.remove_due(&due_key(id, lease))?;
        if self.redis.is_some() {
            self.writes.cmd("DEL").arg(auth_key(id)).ignore();
        }
        self.delete(&lease_key(id))
    }
    fn due(&mut self, now: i64, limit: usize) -> R<Vec<(String, String)>> {
        self.index()?;
        if let Some(conn) = self.sql {
            let mut stmt = conn
                .prepare("SELECT key,value FROM _kv WHERE key>=? AND key<? ORDER BY key LIMIT ?")
                .map_err(err)?;
            let rows = stmt
                .query_map(
                    params![DUE_PREFIX, format!("{DUE_PREFIX}{now:020};"), limit as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(err)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)
        } else {
            let members: Vec<String> = redis::cmd("ZRANGEBYSCORE")
                .arg(DUE)
                .arg("-inf")
                .arg(now)
                .arg("LIMIT")
                .arg(0)
                .arg(limit)
                .query(self.redis.as_deref_mut().unwrap())?;
            members
                .into_iter()
                .map(|member| {
                    let id = member
                        .strip_prefix(DUE_PREFIX)
                        .and_then(|s| s.split_once(':'))
                        .and_then(|(_, s)| s.rsplit_once(':'))
                        .map(|(id, _)| id.to_owned())
                        .ok_or_else(|| err(()))?;
                    Ok((member, id))
                })
                .collect()
        }
    }
    fn commit(&mut self) -> R<()> {
        if let Some(conn) = self.redis.as_deref_mut() {
            let committed: Option<()> = self.writes.query(conn)?;
            if committed.is_none() {
                return Err(redis::RedisError::from((
                    redis::ErrorKind::ResponseError,
                    "job state transaction conflicted; retry",
                )));
            }
        }
        Ok(())
    }
}

/// Redis handle ownership waits are bounded; SQLite restores the original
/// connection busy timeout on every normal/error exit after the transaction.
fn transaction<T>(handle: &Value, mut call: impl FnMut(&mut Store<'_>) -> R<T>) -> Result<T> {
    match get_backend_type(handle)? {
        KVBackend::SQLite => {
            let Value::Map(map) = handle else {
                return Err(conditional::error(()));
            };
            let Some(Value::Int(id)) = map.get("_kv_store_id") else {
                return Err(conditional::error(()));
            };
            let shared = SQLITE_KV_REGISTRY
                .try_lock()
                .map_err(conditional::error)?
                .get(&(*id as u64))
                .cloned()
                .ok_or_else(|| conditional::error(()))?;
            let mut guard = shared.try_lock().map_err(conditional::error)?;
            let timeout: u64 = guard
                .conn
                .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
                .map_err(conditional::error)?;
            guard
                .conn
                .busy_timeout(Duration::ZERO)
                .map_err(conditional::error)?;
            let result: R<T> = (|| {
                let tx = guard
                    .conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(err)?;
                let result = call(&mut Store::sql(&tx))?;
                tx.commit().map_err(err)?;
                Ok(result)
            })();
            let restored = guard
                .conn
                .busy_timeout(Duration::from_millis(timeout))
                .map_err(conditional::error);
            restored?;
            result.map_err(conditional::error)
        }
        KVBackend::Redis => {
            // One connection per worker allows useful Redis concurrency, but all
            // lease mutations intentionally fence on shared indexes. Give a full
            // 32-slot worker wave a bounded chance to serialize its WATCH/EXEC.
            const ATTEMPTS: usize = 32;
            for attempt in 0..ATTEMPTS {
                let result = conditional::redis_call_raw(handle, |conn| {
                    let mut store = Store::redis(conn);
                    let result = call(&mut store).and_then(|value| {
                        store.commit()?;
                        Ok(value)
                    });
                    let _ = redis::cmd("UNWATCH").query::<()>(store.redis.as_deref_mut().unwrap());
                    result
                });
                match result {
                    Ok(value) => return Ok(value),
                    Err(failure) if failure.retryable() && attempt + 1 < ATTEMPTS => {
                        if failure == conditional::StorageFailure::Conflict {
                            conditional::record_redis_transaction_conflict();
                        }
                        let base_ms = 1u64 << attempt.min(3);
                        let jitter_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|duration| duration.subsec_nanos() as u64 % (base_ms + 1))
                            .unwrap_or(0);
                        std::thread::sleep(Duration::from_millis(base_ms + jitter_ms));
                    }
                    Err(failure) => {
                        if failure == conditional::StorageFailure::Conflict {
                            conditional::record_redis_transaction_conflict();
                        }
                        return Err(failure.intent());
                    }
                }
            }
            unreachable!()
        }
    }
}

pub(crate) fn get(handle: &Value, id: &str) -> Result<Option<Lease>> {
    transaction(handle, |store| store.lease(id))
}
pub(crate) fn claim(
    handle: &Value,
    floor: &str,
    ceiling: &str,
    worker_id: &str,
    duration_ms: i64,
) -> Result<Option<Claim>> {
    transaction(handle, |store| {
        let now = store.now()?;
        let deadline_ms = deadline(now, duration_ms)?;
        let candidate: Option<String> = if let Some(conn) = store.sql {
            conn.query_row("SELECT key FROM _kv WHERE key LIKE 'jobs:pending:%' AND key>=? AND key<=? AND (expires_at IS NULL OR expires_at>?) ORDER BY key LIMIT 1", params![floor, ceiling, now/1000], |r| r.get(0)).optional().map_err(err)?
        } else {
            let conn = store.redis.as_deref_mut().unwrap();
            ensure_ready_index(conn)?;
            ready_candidate(conn, floor, ceiling)?
        };
        let Some(pending_key) = candidate else {
            return Ok(None);
        };
        let Some(ready) = store.read(&pending_key)? else {
            return Ok(None);
        };
        let Value::String(id) = ready.value() else {
            return Err(err(()));
        };
        let Some(primary) = store.read(&data_key(&id))? else {
            store.delete(&pending_key)?;
            return Ok(None);
        };
        let Value::Map(mut data) = primary.value() else {
            return Err(err(()));
        };
        let ready_status = text(&data, "status").unwrap_or("pending").to_owned();
        if !matches!(
            ready_status.as_str(),
            "pending" | "scheduled" | "retrying" | "failed"
        ) {
            store.delete(&pending_key)?;
            return Ok(None);
        }
        if store.lease(&id)?.is_some() {
            return Ok(None);
        }
        let token = uuid::Uuid::new_v4().to_string();
        let lease = Lease {
            owner: worker_id.into(),
            token: token.clone(),
            phase: "claimed".into(),
            deadline_ms,
            pending_key: pending_key.clone(),
            ready_status: ready_status.clone(),
        };
        set_text(&mut data, "pending_key", &pending_key);
        set_text(&mut data, "status", "claimed");
        set_text(&mut data, "claim_token", &token);
        set_text(&mut data, "worker_id", worker_id);
        set_text(&mut data, "execution_phase", "claimed");
        set_text(&mut data, "_lease_ready_status", &ready_status);
        data.insert("lease_expires_at_ms".into(), Value::Int(deadline_ms));
        let snapshot = store.snapshot(&Value::Map(data.clone()))?;
        store.save_lease(&id, &lease)?;
        store.put(&data_key(&id), &snapshot)?;
        store.delete(&pending_key)?;
        Ok(Some(Claim {
            snapshot,
            data,
            id,
            token,
            deadline_ms,
            observed_now_ms: now,
        }))
    })
}
pub(crate) fn renew(handle: &Value, id: &str, token: &str, duration_ms: i64) -> Result<bool> {
    transaction(handle, |store| {
        let Some(mut lease) = store.lease(id)? else {
            return Ok(false);
        };
        // Sample after the watched read: a slow read must not renew based on
        // a timestamp captured before ownership expired.
        let now = store.now()?;
        let next_deadline = deadline(now, duration_ms)?;
        if lease.token != token || lease.deadline_ms <= now || !store.authorized(id, token)? {
            return Ok(false);
        }
        store.remove_due(&due_key(id, &lease))?;
        lease.deadline_ms = next_deadline.max(lease.deadline_ms);
        store.save_lease(id, &lease)?;
        Ok(true)
    })
}
pub(crate) fn recover(handle: &Value, limit: usize) -> Result<RecoveryCounts> {
    if limit == 0 {
        return Ok(RecoveryCounts::default());
    }
    transaction(handle, |store| {
        let now = store.now()?;
        let mut counts = RecoveryCounts::default();
        for (member, id) in store.due(now, limit.min(MAX_RECOVERY))? {
            let Some(lease) = store.lease(&id)? else {
                store.remove_due(&member)?;
                continue;
            };
            if member != due_key(&id, &lease) {
                store.remove_due(&member)?;
                continue;
            }
            if lease.deadline_ms > now {
                continue;
            }
            if let Some(primary) = store.read(&data_key(&id))? {
                let Value::Map(mut data) = primary.value() else {
                    return Err(err(()));
                };
                if text(&data, "claim_token") == Some(lease.token.as_str()) {
                    match (lease.phase.as_str(), text(&data, "status")) {
                        ("claimed", Some("claimed")) => {
                            let ready = store.read(&lease.pending_key)?;
                            if ready
                                .as_ref()
                                .is_some_and(|s| !matches!(s.value(), Value::String(v) if v == id))
                            {
                                return Err(err(()));
                            }
                            set_text(&mut data, "status", &lease.ready_status);
                            for key in [
                                "claim_token",
                                "worker_id",
                                "execution_phase",
                                "lease_expires_at_ms",
                                "_lease_ready_status",
                            ] {
                                data.remove(key);
                            }
                            let ready = store.snapshot(&Value::String(id.clone()))?;
                            store.put(&lease.pending_key, &ready)?;
                            counts.requeued += 1;
                        }
                        ("executing", Some("active")) => {
                            set_text(&mut data, "status", "outcome_unknown");
                            set_text(&mut data, "execution_phase", "outcome_unknown");
                            set_text(&mut data, "recovery_reason", "execution_lease_expired");
                            data.insert("recovered_at_ms".into(), Value::Int(now));
                            data.insert(
                                "lease_expires_at_ms".into(),
                                Value::Int(lease.deadline_ms),
                            );
                            counts.unknown += 1;
                        }
                        _ => {
                            store.remove_lease(&id, &lease)?;
                            continue;
                        }
                    }
                    let next = store.snapshot(&Value::Map(data))?;
                    store.put(&data_key(&id), &next)?;
                }
            }
            store.remove_lease(&id, &lease)?;
        }
        Ok(counts)
    })
}

fn transition(
    store: &mut Store<'_>,
    key: &str,
    expected: Option<&Snapshot>,
    next: &Snapshot,
    require_lease: bool,
) -> R<bool> {
    let Some(id) = key.strip_prefix("jobs:data:") else {
        return Ok(!require_lease);
    };
    let expected_data = expected.map(Snapshot::value);
    let expected_token = match &expected_data {
        Some(Value::Map(m)) => text(m, "claim_token"),
        _ => None,
    };
    let lease = store.lease(id)?;
    if require_lease
        && !lease
            .as_ref()
            .is_some_and(|l| Some(l.token.as_str()) == expected_token)
    {
        return Ok(false);
    }
    if let Some(mut lease) = lease {
        if require_lease
            && (lease.deadline_ms <= store.now()? || !store.authorized(id, &lease.token)?)
        {
            return Ok(false);
        }
        // Operator writes may cancel an expired lease, but never alter a lease
        // belonging to a different primary snapshot/token.
        if Some(lease.token.as_str()) != expected_token {
            return Ok(false);
        }
        let Value::Map(data) = next.value() else {
            return Ok(false);
        };
        match text(&data, "status") {
            Some("active") => {
                if text(&data, "claim_token") != expected_token {
                    return Ok(false);
                }
                lease.phase = "executing".into();
                store.save_lease(id, &lease)?;
            }
            Some("claimed") => {
                if lease.phase != "claimed" || text(&data, "claim_token") != expected_token {
                    return Ok(false);
                }
            }
            _ => store.remove_lease(id, &lease)?,
        }
    }
    Ok(true)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;

    fn fixture(handle: &Value, status: &str) -> (String, String) {
        let id = uuid::Uuid::new_v4().to_string();
        let pending = format!("jobs:pending:05:0000000000:{id}");
        let data = Value::Map(HashMap::from([
            ("id".into(), Value::String(id.clone())),
            ("status".into(), Value::String(status.into())),
            ("pending_key".into(), Value::String(pending.clone())),
            ("attempts".into(), Value::Int(3)),
            ("batch_id".into(), Value::String("untouched-batch".into())),
        ]));
        kv_set(handle, &data_key(&id), &data, None).unwrap();
        kv_set(handle, &pending, &Value::String(id.clone()), None).unwrap();
        (id, pending)
    }
    fn take(handle: &Value) -> Claim {
        claim(
            handle,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker-a",
            60_000,
        )
        .unwrap()
        .unwrap()
    }
    fn expire(handle: &Value, id: &str) {
        transaction(handle, |store| {
            let mut lease = store.lease(id)?.unwrap();
            store.remove_due(&due_key(id, &lease))?;
            lease.deadline_ms = store.now()? - 1;
            store.save_lease(id, &lease)
        })
        .unwrap();
    }
    fn state(handle: &Value, id: &str) -> HashMap<String, Value> {
        let Value::Map(data) = kv_get(handle, &data_key(id)).unwrap() else {
            panic!()
        };
        data
    }
    fn next(handle: &Value, claim: &Claim, status: &str) -> Snapshot {
        let mut data = claim.data.clone();
        set_text(&mut data, "status", status);
        Snapshot::prepare(handle, &Value::Map(data)).unwrap()
    }
    fn write(handle: &Value, claim: &Claim, next: &Snapshot, required: bool) -> bool {
        conditional::write(
            handle,
            &data_key(&claim.id),
            Some(&claim.snapshot),
            next,
            None,
            &claim.id,
            &[],
            None,
            required,
        )
        .unwrap()
    }
    fn claimed_contract(handle: &Value) {
        let (id, pending) = fixture(handle, "retrying");
        // Legacy ready records may lack bookkeeping or carry an obsolete key.
        let mut legacy = state(handle, &id);
        legacy.remove("pending_key");
        kv_set(handle, &data_key(&id), &Value::Map(legacy), None).unwrap();
        let claim = take(handle);
        assert_eq!(text(&claim.data, "pending_key"), Some(pending.as_str()));
        assert_eq!(text(&claim.data, "_lease_ready_status"), Some("retrying"));
        assert_eq!(claim.id, id);
        assert_eq!(claim.deadline_ms - claim.observed_now_ms, 60_000);
        assert_eq!(text(&claim.data, "status"), Some("claimed"));
        assert!(matches!(kv_get(handle, &pending).unwrap(), Value::Unit));
        assert_eq!(
            conditional::read(handle, &data_key(&id)).unwrap(),
            Some(claim.snapshot.clone())
        );
        let lease = get(handle, &id).unwrap().unwrap();
        assert_eq!(lease.owner, "worker-a");
        assert_eq!(lease.token, claim.token);
        assert_eq!(lease.ready_status, "retrying");
        assert_eq!(kv_ttl(handle, &lease_key(&id)).unwrap(), None);
        assert_eq!(kv_ttl(handle, &data_key(&id)).unwrap(), None);
        assert!(claim_again(handle).is_none());
        expire(handle, &id);
        let counts = recover(handle, 1).unwrap();
        assert_eq!((counts.requeued, counts.unknown), (1, 0));
        let data = state(handle, &id);
        assert_eq!(text(&data, "status"), Some("retrying"));
        assert!(matches!(data.get("attempts"), Some(Value::Int(3))));
        assert!(!data.contains_key("claim_token"));
        assert!(matches!(kv_get(handle, &pending).unwrap(), Value::String(s) if s == id));
        assert!(get(handle, &id).unwrap().is_none());
        assert_eq!(recover(handle, 1).unwrap().requeued, 0);
        kv_del(handle, &pending).unwrap();
    }
    fn claim_again(handle: &Value) -> Option<Claim> {
        claim(
            handle,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker-b",
            60_000,
        )
        .unwrap()
    }
    fn executing_contract(handle: &Value) {
        let (id, pending) = fixture(handle, "pending");
        let claim = take(handle);
        let unique = format!("jobs:unique:{id}");
        let batch = "jobs:batch:untouched-batch";
        kv_set(
            handle,
            &unique,
            &Value::String("new-owner".into()),
            Some(120),
        )
        .unwrap();
        kv_set(handle, batch, &Value::Int(9), None).unwrap();
        let active = next(handle, &claim, "active");
        assert!(write(handle, &claim, &active, true));
        assert_eq!(get(handle, &id).unwrap().unwrap().phase, "executing");
        expire(handle, &id);
        let expired_deadline = get(handle, &id).unwrap().unwrap().deadline_ms;
        let counts = recover(handle, 1).unwrap();
        assert_eq!((counts.requeued, counts.unknown), (0, 1));
        let data = state(handle, &id);
        assert_eq!(text(&data, "status"), Some("outcome_unknown"));
        assert_eq!(
            text(&data, "recovery_reason"),
            Some("execution_lease_expired")
        );
        assert!(matches!(data.get("recovered_at_ms"), Some(Value::Int(_))));
        assert!(
            matches!(data.get("lease_expires_at_ms"),Some(Value::Int(ms)) if *ms==expired_deadline)
        );
        assert!(matches!(data.get("attempts"), Some(Value::Int(3))));
        assert_eq!(kv_ttl(handle, &data_key(&id)).unwrap(), None);
        assert!(matches!(kv_get(handle, &pending).unwrap(), Value::Unit));
        assert!(matches!(kv_get(handle, &unique).unwrap(), Value::String(s) if s == "new-owner"));
        assert!(kv_ttl(handle, &unique).unwrap().unwrap() > 0);
        assert!(matches!(kv_get(handle, batch).unwrap(), Value::Int(9)));
        assert!(get(handle, &id).unwrap().is_none());
        assert!(!renew(handle, &id, &claim.token, 60_000).unwrap());
    }
    fn fence_contract(handle: &Value) {
        let (id, pending) = fixture(handle, "pending");
        let claim = take(handle);
        assert!(!renew(handle, &id, "wrong-token", 60_000).unwrap());
        assert!(renew(handle, &id, &claim.token, 120_000).unwrap());
        assert_eq!(
            conditional::read(handle, &data_key(&id)).unwrap(),
            Some(claim.snapshot.clone())
        );
        assert_eq!(get(handle, &id).unwrap().unwrap().owner, "worker-a");
        assert_eq!(kv_ttl(handle, &lease_key(&id)).unwrap(), None);
        assert_eq!(recover(handle, 1).unwrap().requeued, 0);
        let completed = next(handle, &claim, "completed");
        expire(handle, &id);
        assert!(!renew(handle, &id, &claim.token, 60_000).unwrap());
        assert!(!write(handle, &claim, &completed, true));
        let active = next(handle, &claim, "active");
        assert!(!write(handle, &claim, &active, true));
        // Operator cancellation still CAS-checks the primary, but may clear
        // ownership after expiry. A stale worker cannot overwrite it.
        let cancelled = next(handle, &claim, "cancelled");
        assert!(write(handle, &claim, &cancelled, false));
        assert!(get(handle, &id).unwrap().is_none());
        assert!(!write(handle, &claim, &completed, true));
        assert!(!write(handle, &claim, &completed, false));
        assert_eq!(recover(handle, 10).unwrap().requeued, 0);
        assert!(matches!(kv_get(handle, &pending).unwrap(), Value::Unit));
    }
    fn receipt_contract(handle: &Value) {
        for status in [
            "completed",
            "cancelled",
            "pending",
            "scheduled",
            "retrying",
            "failed",
        ] {
            let (id, pending) = fixture(handle, "pending");
            let claim = take(handle);
            let mut data = claim.data.clone();
            set_text(&mut data, "status", status);
            set_text(
                &mut data,
                "_job_write_id",
                &uuid::Uuid::new_v4().to_string(),
            );
            let next = Snapshot::prepare(handle, &Value::Map(data.clone())).unwrap();
            let ready = matches!(status, "pending" | "scheduled" | "retrying" | "failed")
                .then_some(pending.as_str());
            let unique = format!("jobs:unique:{id}");
            kv_set(handle, &unique, &Value::String(id.clone()), None).unwrap();
            let publish = || {
                conditional::write(
                    handle,
                    &data_key(&id),
                    Some(&claim.snapshot),
                    &next,
                    Some(60),
                    &id,
                    &[unique.clone()],
                    ready,
                    true,
                )
                .unwrap()
            };
            assert!(publish());
            assert!(get(handle, &id).unwrap().is_none());
            assert!(matches!(kv_get(handle, &unique).unwrap(), Value::Unit));
            if ready.is_some() {
                assert!(matches!(kv_get(handle, &pending).unwrap(), Value::String(s) if s == id));
            }
            // A lost ACK retries the identical prepared write, not its effects.
            kv_set(handle, &unique, &Value::String(id.clone()), None).unwrap();
            kv_del(handle, &pending).unwrap();
            assert!(
                publish(),
                "lost ACK for {status} must acknowledge its receipt"
            );
            assert!(matches!(kv_get(handle, &unique).unwrap(), Value::String(s) if s == id));
            assert!(matches!(kv_get(handle, &pending).unwrap(), Value::Unit));
            assert_eq!(
                conditional::read(handle, &data_key(&id)).unwrap(),
                Some(next.clone())
            );
            // Same status/token, different mutation: this is NOT our receipt.
            set_text(
                &mut data,
                "_job_write_id",
                &uuid::Uuid::new_v4().to_string(),
            );
            kv_set(handle, &data_key(&id), &Value::Map(data), None).unwrap();
            assert!(!publish(), "a newer write ID must reject the stale receipt");
        }
    }
    #[test]
    fn sqlite_terminal_ready_lost_ack_receipts() {
        receipt_contract(&open_kv("sqlite::memory:").unwrap());
    }
    #[test]
    fn sqlite_claimed_recovery() {
        claimed_contract(&open_kv("sqlite::memory:").unwrap());
    }
    #[test]
    fn sqlite_executing_unknown_preserves_owners_and_ttl() {
        executing_contract(&open_kv("sqlite::memory:").unwrap());
    }
    #[test]
    fn sqlite_renew_and_fences() {
        fence_contract(&open_kv("sqlite::memory:").unwrap());
    }
    #[test]
    fn sqlite_claim_rollback_keeps_ready_and_primary() {
        let handle = open_kv("sqlite::memory:").unwrap();
        let (id, pending) = fixture(&handle, "pending");
        let before = conditional::read(&handle, &data_key(&id)).unwrap();
        let shared = get_sqlite_kv(&handle).unwrap();
        shared.lock().unwrap().conn.execute_batch("CREATE TRIGGER fail_claim BEFORE DELETE ON _kv WHEN OLD.key LIKE 'jobs:pending:%' BEGIN SELECT RAISE(ABORT, 'injected'); END;").unwrap();
        assert!(claim(
            &handle,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker",
            60_000
        )
        .is_err());
        assert_eq!(conditional::read(&handle, &data_key(&id)).unwrap(), before);
        assert!(get(&handle, &id).unwrap().is_none());
        assert!(matches!(kv_get(&handle, &pending).unwrap(), Value::String(s) if s == id));
        let count: i64 = shared
            .lock()
            .unwrap()
            .conn
            .query_row(
                "SELECT count(*) FROM _kv WHERE key LIKE 'jobs:lease_due:%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
    fn terminal_contract(handle: &Value) {
        let (id, _) = fixture(handle, "pending");
        let old = take(handle);
        expire(handle, &id);
        assert_eq!(recover(handle, 1).unwrap().requeued, 1);
        let fresh = take(handle);
        assert_ne!(old.token, fresh.token);
        assert!(!renew(handle, &id, &old.token, 60_000).unwrap());
        assert!(!write(handle, &old, &next(handle, &old, "completed"), true));
        let unique = format!("jobs:unique:{id}");
        kv_set(
            handle,
            &unique,
            &Value::String("replacement-owner".into()),
            Some(120),
        )
        .unwrap();
        let completed = next(handle, &fresh, "completed");
        assert!(conditional::write(
            handle,
            &data_key(&id),
            Some(&fresh.snapshot),
            &completed,
            Some(60),
            &id,
            &[unique.clone()],
            None,
            true
        )
        .unwrap());
        assert!(get(handle, &id).unwrap().is_none());
        assert_eq!(recover(handle, 100).unwrap().requeued, 0);
        assert!(kv_ttl(handle, &data_key(&id)).unwrap().unwrap() > 0);
        assert!(
            matches!(kv_get(handle, &unique).unwrap(), Value::String(s) if s == "replacement-owner")
        );
        assert!(kv_ttl(handle, &unique).unwrap().unwrap() > 0);
        // Invalid TTL is rejected before lease/index/primary mutation.
        let (id, _) = fixture(handle, "pending");
        let claim = take(handle);
        let completed = next(handle, &claim, "completed");
        assert!(conditional::write(
            handle,
            &data_key(&id),
            Some(&claim.snapshot),
            &completed,
            Some(0),
            &id,
            &[],
            None,
            true
        )
        .is_err());
        assert!(get(handle, &id).unwrap().is_some());
        assert_eq!(
            conditional::read(handle, &data_key(&id)).unwrap(),
            Some(claim.snapshot.clone())
        );
        assert!(write(handle, &claim, &completed, true));
    }
    #[test]
    fn sqlite_reclaim_fences_and_terminal_owner_ttl() {
        terminal_contract(&open_kv("sqlite::memory:").unwrap());
    }
    #[test]
    fn sqlite_bounded_due_only() {
        let handle = open_kv("sqlite::memory:").unwrap();
        for _ in 0..3 {
            let (id, _) = fixture(&handle, "pending");
            take(&handle);
            expire(&handle, &id);
        }
        // A legacy active record without a lease is not discovered/replayed.
        let (legacy, pending) = fixture(&handle, "active");
        kv_del(&handle, &pending).unwrap();
        assert_eq!(recover(&handle, 0).unwrap().requeued, 0);
        assert_eq!(recover(&handle, 1).unwrap().requeued, 1);
        assert_eq!(recover(&handle, 1).unwrap().requeued, 1);
        assert_eq!(recover(&handle, 10).unwrap().requeued, 1);
        assert_eq!(text(&state(&handle, &legacy), "status"), Some("active"));
    }
    fn concurrency(url: &str) {
        let handle = open_kv(url).unwrap();
        let (id, _) = fixture(&handle, "pending");
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|n| {
                let url = url.to_owned();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let handle = open_kv(&url).unwrap();
                    barrier.wait();
                    for _ in 0..100 {
                        match claim(
                            &handle,
                            "jobs:pending:05:",
                            "jobs:pending:05:~",
                            &format!("worker-{n}"),
                            60_000,
                        ) {
                            Ok(result) => return result.map(|c| (c.id, c.token)),
                            Err(_) => std::thread::sleep(Duration::from_millis(5)),
                        }
                    }
                    panic!("contention did not settle")
                })
            })
            .collect();
        let winners: Vec<_> = threads
            .into_iter()
            .filter_map(|t| t.join().unwrap())
            .collect();
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].0, id);
        assert_eq!(get(&handle, &id).unwrap().unwrap().token, winners[0].1);
        expire(&handle, &id);
        assert_eq!(recover(&handle, 1).unwrap().requeued, 1);
        kv_del(&handle, &get_pending(&handle, &id)).unwrap();
    }
    fn get_pending(handle: &Value, id: &str) -> String {
        text(&state(handle, id), "pending_key").unwrap().to_owned()
    }
    #[test]
    fn sqlite_concurrent_claims() {
        let dir = tempfile::tempdir().unwrap();
        concurrency(dir.path().join("leases.db").to_str().unwrap());
    }
    #[test]
    fn sqlite_contention_is_bounded_and_timeout_restored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("leases.db");
        let handle = open_kv(path.to_str().unwrap()).unwrap();
        let (id, _) = fixture(&handle, "pending");
        let claim = take(&handle);
        let shared = get_sqlite_kv(&handle).unwrap();
        let guard = shared.lock().unwrap();
        let start = std::time::Instant::now();
        assert!(renew(&handle, &id, &claim.token, 60_000).is_err());
        assert!(recover(&handle, 1).is_err());
        assert!(start.elapsed() < Duration::from_millis(100));
        guard.conn.busy_timeout(Duration::from_millis(987)).unwrap();
        drop(guard);
        let other = Connection::open(path).unwrap();
        other.execute_batch("BEGIN IMMEDIATE").unwrap();
        let start = std::time::Instant::now();
        assert!(renew(&handle, &id, &claim.token, 60_000).is_err());
        assert!(recover(&handle, 1).is_err());
        assert!(start.elapsed() < Duration::from_millis(100));
        let timeout: i64 = shared
            .lock()
            .unwrap()
            .conn
            .query_row("PRAGMA busy_timeout", [], |r| r.get(0))
            .unwrap();
        assert_eq!(timeout, 987);
        other.execute_batch("ROLLBACK").unwrap();
        assert!(renew(&handle, &id, &claim.token, 60_000).unwrap());
    }
    // A real TCP proxy forwards every Redis reply unchanged except the chosen
    // EXEC boundary. This catches delays AFTER TIME/WATCH/queue validation.
    enum ExecFault {
        ExpireAt(i64),
        Delay(Duration),
        LoseAck,
    }
    fn resp_frame(reader: &mut impl std::io::BufRead) -> std::io::Result<Vec<u8>> {
        let mut raw = Vec::new();
        if reader.read_until(b'\n', &mut raw)? == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        match raw[0] {
            b'*' | b'$' => {
                let n: i64 = std::str::from_utf8(&raw[1..raw.len() - 2])
                    .unwrap()
                    .parse()
                    .unwrap();
                if n >= 0 {
                    if raw[0] == b'*' {
                        for _ in 0..n {
                            raw.extend(resp_frame(reader)?);
                        }
                    } else {
                        let start = raw.len();
                        raw.resize(start + n as usize + 2, 0);
                        reader.read_exact(&mut raw[start..])?;
                    }
                }
            }
            _ => {}
        }
        Ok(raw)
    }
    fn exec_proxy(url: &str, fault: ExecFault) -> (Value, std::thread::JoinHandle<Vec<u8>>) {
        use std::io::{BufReader, Write};
        use std::net::{TcpListener, TcpStream};
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let upstream_url = reqwest::Url::parse(url).unwrap();
        let mut proxy_url = upstream_url.clone();
        proxy_url.set_host(Some("127.0.0.1")).unwrap();
        proxy_url.set_port(Some(address.port())).unwrap();
        let url_owned = url.to_owned();
        let thread = std::thread::spawn(move || {
            let (mut client, _) = listener.accept().unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut upstream = TcpStream::connect((
                upstream_url.host_str().unwrap(),
                upstream_url.port().unwrap_or(6379),
            ))
            .unwrap();
            upstream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut requests = BufReader::new(client.try_clone().unwrap());
            let mut responses = BufReader::new(upstream.try_clone().unwrap());
            loop {
                let request = resp_frame(&mut requests).unwrap();
                let is_exec = request == redis::cmd("EXEC").get_packed_command();
                if is_exec {
                    if let ExecFault::Delay(duration) = fault {
                        std::thread::sleep(duration);
                    }
                    if let ExecFault::ExpireAt(deadline) = fault {
                        let mut clock = redis::Client::open(url_owned.as_str())
                            .unwrap()
                            .get_connection()
                            .unwrap();
                        let (sec, micros): (i64, i64) =
                            redis::cmd("TIME").query(&mut clock).unwrap();
                        let now = sec * 1000 + micros / 1000;
                        assert!(
                            now < deadline,
                            "test must reach EXEC while still authorized"
                        );
                        std::thread::sleep(Duration::from_millis((deadline - now + 25) as u64));
                    }
                }
                upstream.write_all(&request).unwrap();
                let response = resp_frame(&mut responses).unwrap();
                if is_exec && matches!(fault, ExecFault::LoseAck) {
                    return response; // Drop the successful reply and connection.
                }
                client.write_all(&response).unwrap();
                if is_exec {
                    // Production cleanup still receives its normal UNWATCH ACK.
                    let request = resp_frame(&mut requests).unwrap();
                    upstream.write_all(&request).unwrap();
                    client
                        .write_all(&resp_frame(&mut responses).unwrap())
                        .unwrap();
                    return response;
                }
            }
        });
        let Value::Map(mut handle) = open_kv(proxy_url.as_str()).unwrap() else {
            panic!()
        };
        // A lost ACK reconnects directly rather than needing another proxy fault.
        handle.insert("_url".into(), Value::String(url.into()));
        (Value::Map(handle), thread)
    }
    fn redis_fixture() -> (String, Value) {
        // These disposable-DB contracts run serially, as configured in CI.
        let url = std::env::var("NTNT_RETENTION_TEST_REDIS").unwrap();
        let handle = open_kv(&url).unwrap();
        conditional::redis_call(&handle, |conn| redis::cmd("FLUSHDB").query::<()>(conn)).unwrap();
        (url, handle)
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_claim_does_not_scan_unrelated_history() {
        let (_, handle) = redis_fixture();
        for n in 0..1_000 {
            kv_set(
                &handle,
                &format!("jobs:data:history-{n}"),
                &Value::String("completed".into()),
                None,
            )
            .unwrap();
        }
        assert!(claim(
            &handle,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker-a",
            60_000,
        )
        .unwrap()
        .is_none());
        fixture(&handle, "pending");
        conditional::redis_call(&handle, |conn| {
            redis::cmd("CONFIG").arg("RESETSTAT").query::<()>(conn)
        })
        .unwrap();

        assert!(take(&handle).id.len() > 1);
        let stats: String = conditional::redis_call(&handle, |conn| {
            redis::cmd("INFO").arg("commandstats").query(conn)
        })
        .unwrap();
        assert!(!stats.contains("cmdstat_scan:"), "claim used SCAN: {stats}");
        assert!(!stats.contains("cmdstat_keys:"), "claim used KEYS: {stats}");
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_ready_index_tracks_claim_recovery_and_migration() {
        let (_, handle) = redis_fixture();
        let (id, pending) = fixture(&handle, "pending");
        let indexed = |handle: &Value, pending: &str| {
            conditional::redis_call(handle, |conn| {
                redis::cmd("ZSCORE")
                    .arg(READY)
                    .arg(pending)
                    .query::<Option<f64>>(conn)
            })
            .unwrap()
            .is_some()
        };
        assert!(indexed(&handle, &pending));

        let claimed = take(&handle);
        assert_eq!(claimed.id, id);
        assert!(!indexed(&handle, &pending));
        expire(&handle, &id);
        assert_eq!(recover(&handle, 1).unwrap().requeued, 1);
        assert!(indexed(&handle, &pending));

        // Simulate an upgrade from a deployment that predates the ready index.
        conditional::redis_call(&handle, |conn| {
            redis::pipe()
                .del(READY)
                .del(READY_INITIALIZED)
                .query::<()>(conn)
        })
        .unwrap();
        assert_eq!(take(&handle).id, id);
        assert!(!indexed(&handle, &pending));

        // Losing the derived index while retaining its marker must rebuild it.
        let (next_id, next_pending) = fixture(&handle, "pending");
        assert!(indexed(&handle, &next_pending));
        conditional::redis_call(&handle, |conn| {
            redis::cmd("DEL").arg(READY).query::<()>(conn)
        })
        .unwrap();
        assert_eq!(take(&handle).id, next_id);
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_ready_migration_waits_and_reelects() {
        let (_, handle) = redis_fixture();
        conditional::redis_call(&handle, |conn| {
            redis::cmd("SET")
                .arg(READY_MIGRATION_LOCK)
                .arg("departed-migrator")
                .arg("PX")
                .arg(125)
                .query::<()>(conn)
        })
        .unwrap();
        let start = std::time::Instant::now();
        conditional::redis_call(&handle, ensure_ready_index).unwrap();
        assert!(start.elapsed() >= Duration::from_millis(100));
        assert!(start.elapsed() < Duration::from_secs(2));
        assert!(conditional::redis_call(&handle, ready_index_initialized).unwrap());
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_owned_valkey_handle_unregisters_on_drop() {
        let (url, _) = redis_fixture();
        let valkey_url = url.replacen("redis://", "valkey://", 1);
        let owned = open_owned_kv(&valkey_url).unwrap();
        let handle = owned.value().clone();
        assert!(matches!(
            &handle,
            Value::Map(map) if matches!(map.get("_backend"), Some(Value::String(name)) if name == "valkey")
        ));
        assert!(get_redis_kv(&handle).is_ok());
        drop(owned);
        assert!(get_redis_kv(&handle).is_err());
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_claim_prunes_stale_ready_members() {
        let (_, handle) = redis_fixture();
        let (id, _) = fixture(&handle, "pending");
        let stale = "jobs:pending:05:0000000000:00000000-stale";
        conditional::redis_call(&handle, |conn| {
            let mut tx = redis::pipe();
            for n in 0..1300 {
                tx.cmd("ZADD")
                    .arg(READY)
                    .arg(0)
                    .arg(format!("{stale}-{n:03}"))
                    .ignore();
            }
            tx.query::<()>(conn)
        })
        .unwrap();

        assert!(claim(
            &handle,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker-bounded-stale-cleanup",
            60_000,
        )
        .unwrap()
        .is_none());
        assert_eq!(take(&handle).id, id);
        let remaining: usize = conditional::redis_call(&handle, |conn| {
            redis::cmd("ZCOUNT")
                .arg(READY)
                .arg("-inf")
                .arg("+inf")
                .query(conn)
        })
        .unwrap();
        assert_eq!(remaining, 1, "only the persistent sentinel should remain");
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_ready_claim_preserves_order_bounds_and_legacy_prefix() {
        let (_, handle) = redis_fixture();
        let (id_a, pending_a) = fixture(&handle, "pending");
        let (id_b, pending_b) = fixture(&handle, "pending");
        let (first_id, first_pending) = if pending_a < pending_b {
            (id_a, pending_a)
        } else {
            (id_b, pending_b)
        };

        let exact = claim(
            &handle,
            &first_pending,
            &first_pending,
            "worker-exact",
            60_000,
        )
        .unwrap()
        .unwrap();
        assert_eq!(exact.id, first_id);

        let lower_band = "jobs:pending:04:0000000000:lower-band";
        kv_set(&handle, lower_band, &Value::String("owner".into()), None).unwrap();
        let claimed = super::super::kv_claim(
            &handle,
            "jobs:pending:05:",
            Some("jobs:pending:00:"),
            Some("jobs:pending:99:~"),
        )
        .unwrap()
        .unwrap();
        assert!(claimed.0.starts_with("jobs:pending:05:"));
        assert!(matches!(
            kv_get(&handle, lower_band).unwrap(),
            Value::String(_)
        ));
        let indexed: Option<f64> = conditional::redis_call(&handle, |conn| {
            redis::cmd("ZSCORE").arg(READY).arg(&claimed.0).query(conn)
        })
        .unwrap();
        assert!(indexed.is_none());
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_ready_wrong_type_cannot_partially_publish() {
        let (_, handle) = redis_fixture();
        conditional::redis_call(&handle, |conn| {
            redis::cmd("SET")
                .arg(READY)
                .arg("wrong-type")
                .query::<()>(conn)
        })
        .unwrap();
        let state_key = "jobs:data:wrong-ready-type";
        let pending = "jobs:pending:05:0000000000:wrong-ready-type";
        let next = Snapshot::prepare(&handle, &Value::String("pending".into())).unwrap();
        let error = conditional::write(
            &handle,
            state_key,
            None,
            &next,
            None,
            "wrong-ready-type",
            &[],
            Some(pending),
            false,
        )
        .unwrap_err();
        assert!(error.to_string().contains("(wrong_type)"));
        assert!(matches!(kv_get(&handle, state_key).unwrap(), Value::Unit));
        assert!(matches!(kv_get(&handle, pending).unwrap(), Value::Unit));
        assert!(!conditional::redis_call(&handle, |conn| {
            redis::cmd("EXISTS")
                .arg(READY_INITIALIZED)
                .query::<bool>(conn)
        })
        .unwrap());
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_transaction_serializes_32_worker_wave() {
        let (url, handle) = redis_fixture();
        let other = open_kv(&url).unwrap();
        kv_set(&handle, "retry:watch", &Value::Int(1), None).unwrap();
        let mut calls = 0;
        let conflicts_before = conditional::redis_transaction_conflicts();
        transaction(&handle, |store| {
            calls += 1;
            let _ = store.read("retry:watch")?;
            if calls < 32 {
                kv_set(&other, "retry:watch", &Value::Int(calls), None).map_err(err)?;
            }
            let value = store.snapshot(&Value::String("committed".into()))?;
            store.put("retry:result", &value)
        })
        .unwrap();
        assert_eq!(calls, 32);
        assert!(
            conditional::redis_transaction_conflicts() - conflicts_before >= 31,
            "each retried conflict must increment telemetry"
        );
        assert!(matches!(
            kv_get(&handle, "retry:result").unwrap(),
            Value::String(value) if value == "committed"
        ));
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_transaction_retries_shared_handle_busy() {
        let (_, handle) = redis_fixture();
        let shared = get_redis_kv(&handle).unwrap();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _guard = shared.lock().unwrap();
            locked_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(8));
        });
        locked_rx.recv().unwrap();
        let now = transaction(&handle, |store| store.now()).unwrap();
        assert!(now > 0);
        holder.join().unwrap();
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_shared_handle_serializes_32_callers() {
        let (_, handle) = redis_fixture();
        let Value::Map(info) = &handle else { panic!() };
        let Value::String(url) = info.get("_url").unwrap() else {
            panic!()
        };
        let Value::Int(store_id) = info.get("_kv_store_id").unwrap() else {
            panic!()
        };
        let url = url.clone();
        let store_id = *store_id;
        let barrier = Arc::new(Barrier::new(33));
        let start = std::time::Instant::now();
        let threads: Vec<_> = (0..32)
            .map(|_| {
                let url = url.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let handle = Value::Map(HashMap::from([
                        ("_backend".into(), Value::String("redis".into())),
                        ("_url".into(), Value::String(url)),
                        ("_kv_store_id".into(), Value::Int(store_id)),
                    ]));
                    barrier.wait();
                    for _ in 0..25 {
                        transaction(&handle, |store| store.now())?;
                    }
                    Ok::<(), IntentError>(())
                })
            })
            .collect();
        barrier.wait();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_shared_handle_wait_is_bounded_and_typed() {
        let (_, handle) = redis_fixture();
        let shared = get_redis_kv(&handle).unwrap();
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _guard = shared.lock().unwrap();
            locked_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(350));
        });
        locked_rx.recv().unwrap();
        let start = std::time::Instant::now();
        let error = transaction(&handle, |store| store.now()).unwrap_err();
        let elapsed = start.elapsed();
        assert!(error.to_string().contains("(local_busy)"));
        assert!(elapsed >= Duration::from_millis(200));
        assert!(elapsed < Duration::from_millis(500));
        holder.join().unwrap();
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_call_serializes_32_conflicts() {
        let (_, handle) = redis_fixture();
        let mut calls = 0;
        let conflicts_before = conditional::redis_transaction_conflicts();
        let pong: String = conditional::redis_call(&handle, |conn| {
            calls += 1;
            if calls < 32 {
                return Err(redis::RedisError::from((
                    redis::ErrorKind::ResponseError,
                    "job state transaction conflicted; retry",
                )));
            }
            redis::cmd("PING").query(conn)
        })
        .unwrap();
        assert_eq!(calls, 32);
        assert!(
            conditional::redis_transaction_conflicts() - conflicts_before >= 31,
            "each retried conflict must increment telemetry"
        );
        assert_eq!(pong, "PONG");
    }
    fn short_claim(handle: &Value) -> Claim {
        claim(
            handle,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker-a",
            700,
        )
        .unwrap()
        .unwrap()
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_late_initial_claim_keeps_recovery_but_not_authorization() {
        let (url, handle) = redis_fixture();
        let (id, pending) = fixture(&handle, "pending");
        let (proxy, thread) = exec_proxy(&url, ExecFault::Delay(Duration::from_millis(200)));
        let claim = claim(
            &proxy,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "late-worker",
            100,
        )
        .unwrap()
        .unwrap();
        let response = thread.join().unwrap();
        assert!(response.starts_with(b"*") && response != b"*-1\r\n");
        assert_eq!(claim.id, id);
        assert!(matches!(
            kv_get(&handle, &auth_key(&id)).unwrap(),
            Value::Unit
        ));
        assert_eq!(
            get(&handle, &id).unwrap().unwrap().deadline_ms,
            claim.deadline_ms
        );
        assert_eq!(kv_ttl(&handle, &lease_key(&id)).unwrap(), None);
        assert_eq!(kv_ttl(&handle, DUE).unwrap(), None);
        assert!(!renew(&handle, &id, &claim.token, 60_000).unwrap());
        assert!(!write(
            &handle,
            &claim,
            &next(&handle, &claim, "active"),
            true
        ));
        assert_eq!(recover(&handle, 1).unwrap().requeued, 1);
        assert!(matches!(kv_get(&handle, &pending).unwrap(), Value::String(s) if s == id));
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_authorization_token_and_absolute_deadline() {
        let (_, handle) = redis_fixture();
        let (id, _) = fixture(&handle, "pending");
        let claim = take(&handle);
        let check_auth = |deadline| {
            conditional::redis_call(&handle, |conn| {
                let token: String = redis::cmd("GET").arg(auth_key(&id)).query(conn)?;
                let expires: i64 = redis::cmd("PEXPIRETIME").arg(auth_key(&id)).query(conn)?;
                assert_eq!(token, claim.token);
                assert_eq!(expires, deadline);
                Ok(())
            })
            .unwrap()
        };
        check_auth(claim.deadline_ms);
        assert!(renew(&handle, &id, &claim.token, 120_000).unwrap());
        check_auth(get(&handle, &id).unwrap().unwrap().deadline_ms);
        for token in [Some("wrong-token"), None] {
            conditional::redis_call(&handle, |conn| {
                if let Some(token) = token {
                    redis::cmd("SET")
                        .arg(auth_key(&id))
                        .arg(token)
                        .query::<()>(conn)
                } else {
                    redis::cmd("DEL").arg(auth_key(&id)).query::<()>(conn)
                }
            })
            .unwrap();
            assert!(!renew(&handle, &id, &claim.token, 60_000).unwrap());
            assert!(!write(
                &handle,
                &claim,
                &next(&handle, &claim, "active"),
                true
            ));
            assert!(get(&handle, &id).unwrap().is_some());
        }
        assert!(write(
            &handle,
            &claim,
            &next(&handle, &claim, "cancelled"),
            false
        ));
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_delayed_exec_cannot_publish_after_expiry() {
        let (url, handle) = redis_fixture();
        fixture(&handle, "pending");
        let claim = short_claim(&handle);
        let active = next(&handle, &claim, "active");
        assert!(write(&handle, &claim, &active, true));
        let completed = next(&handle, &claim, "completed");
        let (proxy, thread) = exec_proxy(&url, ExecFault::ExpireAt(claim.deadline_ms));
        let result = conditional::write(
            &proxy,
            &data_key(&claim.id),
            Some(&active),
            &completed,
            Some(60),
            &claim.id,
            &[],
            None,
            true,
        );
        let response = thread.join().unwrap();
        assert_eq!(
            response, b"*-1\r\n",
            "expiry must abort EXEC, not publish completion"
        );
        assert!(result.is_err());
        assert_eq!(
            conditional::read(&handle, &data_key(&claim.id)).unwrap(),
            Some(active)
        );
        assert!(get(&handle, &claim.id).unwrap().is_some());
        assert_eq!(recover(&handle, 1).unwrap().unknown, 1);
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_delayed_exec_cannot_revive_expired_lease() {
        let (url, handle) = redis_fixture();
        fixture(&handle, "pending");
        let claim = short_claim(&handle);
        let (proxy, thread) = exec_proxy(&url, ExecFault::ExpireAt(claim.deadline_ms));
        let result = renew(&proxy, &claim.id, &claim.token, 60_000);
        let response = thread.join().unwrap();
        assert_eq!(response, b"*-1\r\n", "expiry must abort renewal EXEC");
        assert!(result.is_err());
        assert_eq!(
            get(&handle, &claim.id).unwrap().unwrap().deadline_ms,
            claim.deadline_ms
        );
        assert_eq!(recover(&handle, 1).unwrap().requeued, 1);
        kv_del(&handle, &get_pending(&handle, &claim.id)).unwrap();
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_execution_receipts_validate_watch_at_exec() {
        let (url, handle) = redis_fixture();
        for status in ["active", "claimed"] {
            fixture(&handle, "pending");
            let claim = short_claim(&handle);
            let receipt = if status == "active" {
                let active = next(&handle, &claim, status);
                assert!(write(&handle, &claim, &active, true));
                active
            } else {
                claim.snapshot.clone()
            };
            assert!(write(&handle, &claim, &receipt, true));
            let (proxy, thread) = exec_proxy(&url, ExecFault::ExpireAt(claim.deadline_ms));
            let result = conditional::write(
                &proxy,
                &data_key(&claim.id),
                Some(&claim.snapshot),
                &receipt,
                None,
                &claim.id,
                &[],
                None,
                true,
            );
            assert!(
                !matches!(result, Ok(true)),
                "{status} receipt must EXEC its WATCH fence"
            );
            assert_eq!(thread.join().unwrap(), b"*-1\r\n");
            assert!(!write(&handle, &claim, &receipt, true));
            let counts = recover(&handle, 1).unwrap();
            assert_eq!(
                (counts.requeued, counts.unknown),
                if status == "active" { (0, 1) } else { (1, 0) }
            );
            kv_del(&handle, &get_pending(&handle, &claim.id)).unwrap();
        }
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_terminal_ready_lost_ack_receipts() {
        let (url, handle) = redis_fixture();
        receipt_contract(&handle);
        for status in ["completed", "pending"] {
            let (id, pending) = fixture(&handle, "pending");
            let claim = take(&handle);
            let mut data = claim.data.clone();
            set_text(&mut data, "status", status);
            set_text(
                &mut data,
                "_job_write_id",
                &uuid::Uuid::new_v4().to_string(),
            );
            let next = Snapshot::prepare(&handle, &Value::Map(data)).unwrap();
            let ready = (status == "pending").then_some(pending.as_str());
            let (proxy, thread) = exec_proxy(&url, ExecFault::LoseAck);
            let publish = || {
                conditional::write(
                    &proxy,
                    &data_key(&id),
                    Some(&claim.snapshot),
                    &next,
                    Some(60),
                    &id,
                    &[],
                    ready,
                    true,
                )
            };
            assert!(
                publish().is_err(),
                "the first committed ACK must actually be lost"
            );
            let response = thread.join().unwrap();
            assert!(response.starts_with(b"*") && response != b"*-1\r\n");
            assert!(
                !response.contains(&b'-'),
                "EXEC must have succeeded before ACK loss"
            );
            assert_eq!(
                conditional::read(&handle, &data_key(&id)).unwrap(),
                Some(next.clone())
            );
            assert!(get(&handle, &id).unwrap().is_none());
            assert!(matches!(
                kv_get(&handle, &auth_key(&id)).unwrap(),
                Value::Unit
            ));
            // Retry on the same handle after the real reconnect handoff.
            let until = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                match publish() {
                    Ok(value) => {
                        assert!(value, "{status} receipt must acknowledge");
                        break;
                    }
                    Err(_) => {
                        assert!(
                            std::time::Instant::now() < until,
                            "reconnect did not finish"
                        );
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            if ready.is_some() {
                let fresh = take(&handle);
                assert_eq!(fresh.id, id);
                assert!(
                    !publish().unwrap(),
                    "a newer claim rejects the stale receipt"
                );
                assert!(write(
                    &handle,
                    &fresh,
                    &self::next(&handle, &fresh, "cancelled"),
                    true
                ));
            }
        }
    }
    #[test]
    #[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
    fn redis_lease_contracts() {
        let url =
            std::env::var("NTNT_RETENTION_TEST_REDIS").expect("set disposable Redis test URL");
        let handle = open_kv(&url).unwrap();
        let shared = get_redis_kv(&handle).unwrap();
        redis::cmd("FLUSHDB")
            .query::<()>(&mut shared.lock().unwrap().conn)
            .unwrap();
        claimed_contract(&handle);
        executing_contract(&handle);
        fence_contract(&handle);
        terminal_contract(&handle);
        concurrency(&url);
        let (id, pending) = fixture(&handle, "pending");
        kv_set(&handle, DUE, &Value::String("wrong-type".into()), None).unwrap();
        assert!(claim(
            &handle,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker",
            60_000
        )
        .is_err());
        assert_eq!(text(&state(&handle, &id), "status"), Some("pending"));
        assert!(get(&handle, &id).unwrap().is_none());
        assert!(matches!(kv_get(&handle, &pending).unwrap(), Value::String(s) if s == id));
        kv_del(&handle, DUE).unwrap();
        // Queue-time ACL denial cannot partially publish a claim. Also cover
        // recovery denial after the pending SET has already been queued.
        let username = format!("ntnt-leases-{}", uuid::Uuid::new_v4());
        redis::cmd("ACL")
            .arg("SETUSER")
            .arg(&username)
            .arg("reset")
            .arg("on")
            .arg(">lease-fixture")
            .arg("~*")
            .arg("+@all")
            .arg("-zadd")
            .query::<()>(&mut shared.lock().unwrap().conn)
            .unwrap();
        let mut restricted_url = reqwest::Url::parse(&url).unwrap();
        restricted_url.set_username(&username).unwrap();
        restricted_url.set_password(Some("lease-fixture")).unwrap();
        let restricted = open_kv(restricted_url.as_str()).unwrap();
        assert!(claim(
            &restricted,
            "jobs:pending:05:",
            "jobs:pending:05:~",
            "worker",
            60_000
        )
        .is_err());
        assert_eq!(text(&state(&handle, &id), "status"), Some("pending"));
        assert!(get(&handle, &id).unwrap().is_none());
        assert!(matches!(kv_get(&handle, &pending).unwrap(), Value::String(s) if s == id));
        let claim = take(&handle);
        let lease = get(&handle, &id).unwrap().unwrap();
        assert_eq!(kv_ttl(&handle, DUE).unwrap(), None);
        assert!(renew(&restricted, &id, &claim.token, 120_000).is_err());
        assert_eq!(
            get(&handle, &id).unwrap().unwrap().deadline_ms,
            lease.deadline_ms
        );
        assert_eq!(
            conditional::read(&handle, &data_key(&id)).unwrap(),
            Some(claim.snapshot.clone())
        );
        redis::cmd("ACL")
            .arg("SETUSER")
            .arg(&username)
            .arg("+zadd")
            .arg("-zrem")
            .query::<()>(&mut shared.lock().unwrap().conn)
            .unwrap();
        expire(&handle, &id);
        assert!(recover(&restricted, 1).is_err());
        assert!(matches!(kv_get(&handle, &pending).unwrap(), Value::Unit));
        assert!(get(&handle, &id).unwrap().is_some());
        assert_eq!(text(&state(&handle, &id), "status"), Some("claimed"));
        assert_eq!(recover(&handle, 1).unwrap().requeued, 1);
        redis::cmd("ACL")
            .arg("DELUSER")
            .arg(&username)
            .query::<i64>(&mut shared.lock().unwrap().conn)
            .unwrap();
        // Registry lookup remains immediate; each store operation waits for the
        // shared connection only up to the bounded local-ownership deadline.
        let guard = shared.lock().unwrap();
        let start = std::time::Instant::now();
        assert!(renew(&handle, &id, &claim.token, 60_000).is_err());
        assert!(recover(&handle, 1).is_err());
        assert!(start.elapsed() >= Duration::from_millis(400));
        assert!(start.elapsed() < Duration::from_millis(750));
        drop(guard);
        conditional::check_redis_reconnect(&handle);
    }
}

pub(super) fn sql_transition(
    conn: &Connection,
    key: &str,
    expected: Option<&Snapshot>,
    next: &Snapshot,
    require_lease: bool,
) -> Result<bool> {
    transition(&mut Store::sql(conn), key, expected, next, require_lease)
        .map_err(conditional::error)
}
pub(super) fn redis_transition(
    conn: &mut redis::Connection,
    writes: &mut redis::Pipeline,
    key: &str,
    expected: Option<&Snapshot>,
    next: &Snapshot,
    require_lease: bool,
) -> R<bool> {
    let mut store = Store::redis(conn);
    std::mem::swap(&mut store.writes, writes);
    let result = transition(&mut store, key, expected, next, require_lease);
    std::mem::swap(&mut store.writes, writes);
    result
}
