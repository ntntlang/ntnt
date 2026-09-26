//! Small conditional KV writes used to retry job state persistence safely.
//! No retention indexes, policies, backfill, or capacity accounting.
use super::*;
use rusqlite::OptionalExtension;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    pub(super) raw: String,
    pub(super) kind: Option<String>,
    pub(super) redis: bool,
}
impl Snapshot {
    pub fn value(&self) -> Value {
        if self.redis {
            deserialize_value_envelope(&self.raw, self.kind.as_deref())
        } else {
            deserialize_value(&self.raw, self.kind.as_deref().unwrap_or("string"))
        }
    }
    pub fn prepare(handle: &Value, value: &Value) -> Result<Self> {
        if get_backend_type(handle)? == KVBackend::Redis {
            Ok(Self {
                raw: serialize_value_envelope(value)?,
                kind: None,
                redis: true,
            })
        } else {
            let (raw, kind) = serialize_value(value)?;
            Ok(Self {
                raw,
                kind: Some(kind),
                redis: false,
            })
        }
    }
}
pub(super) fn error<E>(_: E) -> IntentError {
    IntentError::runtime_error("job state storage operation failed")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StorageFailure {
    LocalBusy,
    MigrationBusy,
    Busy,
    Conflict,
    Connection,
    Permission,
    WrongType,
    Command,
    Other,
}

impl StorageFailure {
    fn from_redis(error: &redis::RedisError) -> Self {
        let message = error.to_string().to_ascii_uppercase();
        if error.is_io_error() {
            Self::Connection
        } else if message.contains("READY_INDEX_BUSY") {
            Self::MigrationBusy
        } else if message.contains("TRANSACTION CONFLICTED") {
            Self::Conflict
        } else if message.contains("NOPERM") || message.contains("NOAUTH") {
            Self::Permission
        } else if message.contains("WRONGTYPE") || error.kind() == redis::ErrorKind::TypeError {
            Self::WrongType
        } else if message.contains("BUSY") || message.contains("TRYAGAIN") {
            Self::Busy
        } else if error.kind() == redis::ErrorKind::ResponseError {
            Self::Command
        } else {
            Self::Other
        }
    }
    pub(super) fn retryable(self) -> bool {
        matches!(self, Self::Busy | Self::Conflict)
    }
    fn label(self) -> &'static str {
        match self {
            Self::LocalBusy => "local_busy",
            Self::MigrationBusy => "migration_busy",
            Self::Busy => "busy",
            Self::Conflict => "conflict",
            Self::Connection => "connection",
            Self::Permission => "permission",
            Self::WrongType => "wrong_type",
            Self::Command => "command",
            Self::Other => "other",
        }
    }
    pub(super) fn intent(self) -> IntentError {
        IntentError::runtime_error(format!(
            "job state storage operation failed ({})",
            self.label()
        ))
    }
}

pub(crate) fn is_contention_error(error: &IntentError) -> bool {
    let message = error.to_string();
    message.contains("(local_busy)")
        || message.contains("(migration_busy)")
        || message.contains("(busy)")
        || message.contains("(conflict)")
}

static REDIS_TRANSACTION_CONFLICTS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

pub(super) fn record_redis_transaction_conflict() {
    REDIS_TRANSACTION_CONFLICTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn redis_transaction_conflicts() -> u64 {
    REDIS_TRANSACTION_CONFLICTS.load(std::sync::atomic::Ordering::Relaxed)
}

fn lock_redis_store(
    shared: &Arc<Mutex<RedisKV>>,
) -> std::result::Result<std::sync::MutexGuard<'_, RedisKV>, StorageFailure> {
    let deadline = std::time::Instant::now() + Duration::from_millis(250);
    loop {
        match shared.try_lock() {
            Ok(store) => return Ok(store),
            Err(std::sync::TryLockError::Poisoned(_)) => return Err(StorageFailure::Other),
            Err(std::sync::TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    return Err(StorageFailure::LocalBusy);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
}
pub(super) fn sql_read(conn: &Connection, key: &str) -> Result<Option<Snapshot>> {
    conn.query_row(
        "SELECT value,type FROM _kv WHERE key=? AND (expires_at IS NULL OR expires_at>?)",
        params![key, now_unix()],
        |r| {
            Ok(Snapshot {
                raw: r.get(0)?,
                kind: Some(r.get(1)?),
                redis: false,
            })
        },
    )
    .optional()
    .map_err(error)
}
pub(super) fn redis_call_raw<T>(
    handle: &Value,
    call: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
) -> std::result::Result<T, StorageFailure> {
    let Value::Map(map) = handle else {
        return Err(StorageFailure::Other);
    };
    let Some(Value::Int(id)) = map.get("_kv_store_id") else {
        return Err(StorageFailure::Other);
    };
    let shared = REDIS_KV_REGISTRY
        .lock()
        .map_err(|_| StorageFailure::Other)?
        .get(&(*id as u64))
        .cloned()
        .ok_or(StorageFailure::Other)?;
    let mut store = lock_redis_store(&shared)?;
    if store.reconnecting {
        return Err(StorageFailure::Connection);
    }
    let result = (|| {
        store.conn.set_read_timeout(Some(Duration::from_secs(1)))?;
        store.conn.set_write_timeout(Some(Duration::from_secs(1)))?;
        call(&mut store.conn)
    })();
    let broken = result.as_ref().is_err_and(|e| e.is_io_error());
    if !broken {
        if result.is_err() {
            let _ = redis::cmd("DISCARD").query::<()>(&mut store.conn);
            let _ = redis::cmd("UNWATCH").query::<()>(&mut store.conn);
        }
        let _ = store.conn.set_read_timeout(None);
        let _ = store.conn.set_write_timeout(None);
    }
    if broken {
        // The pinned Redis client's timeout covers TCP, not protocol setup.
        // One pending connector keeps retries/cancellation responsive without
        // blocking this mutex or accumulating threads during a blackhole.
        if let Value::Map(m) = handle {
            if let Some(Value::String(url)) = m.get("_url") {
                let url = url.replacen("valkey://", "redis://", 1);
                let shared = Arc::clone(&shared);
                store.reconnecting = true;
                if std::thread::Builder::new()
                    .name("ntnt-redis-reconnect".into())
                    .spawn(move || {
                        let result = redis::Client::open(url)
                            .and_then(|c| c.get_connection_with_timeout(Duration::from_secs(1)));
                        if let Ok(mut store) = shared.lock() {
                            if let Ok(conn) = result {
                                store.conn = conn;
                            }
                            store.reconnecting = false;
                        }
                    })
                    .is_err()
                {
                    store.reconnecting = false;
                }
            }
        }
    }
    result.map_err(|error| StorageFailure::from_redis(&error))
}

pub(super) fn redis_call<T>(
    handle: &Value,
    mut f: impl FnMut(&mut redis::Connection) -> redis::RedisResult<T>,
) -> Result<T> {
    const ATTEMPTS: usize = 32;
    for attempt in 0..ATTEMPTS {
        match redis_call_raw(handle, |conn| f(conn)) {
            Ok(value) => return Ok(value),
            Err(failure) if failure.retryable() && attempt + 1 < ATTEMPTS => {
                if failure == StorageFailure::Conflict {
                    record_redis_transaction_conflict();
                }
                let base = 1_u64 << attempt.min(3);
                let jitter = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |time| u64::from(time.subsec_nanos()) % (base + 1));
                std::thread::sleep(std::time::Duration::from_millis(base + jitter));
            }
            Err(failure) => {
                if failure == StorageFailure::Conflict {
                    record_redis_transaction_conflict();
                }
                return Err(failure.intent());
            }
        }
    }
    unreachable!()
}
pub(crate) fn read(handle: &Value, key: &str) -> Result<Option<Snapshot>> {
    match get_backend_type(handle)? {
        KVBackend::SQLite => sql_read(&get_sqlite_kv(handle)?.lock().map_err(error)?.conn, key),
        KVBackend::Redis => redis_call(handle, |conn| {
            let (raw, kind): (Option<String>, Option<String>) = redis::cmd("MGET")
                .arg(key)
                .arg(format!("{key}:__type"))
                .query(conn)?;
            Ok(raw.map(|raw| Snapshot {
                raw,
                kind,
                redis: true,
            }))
        }),
    }
}

// Exact terminal/ready receipts acknowledge an already committed mutation,
// whose lease was removed. Only a receipt permitting more execution needs
// ownership; raw snapshot equality above includes the stable _job_write_id.
fn execution_receipt(next: &Snapshot) -> bool {
    matches!(next.value(), Value::Map(m)
        if matches!(m.get("status"), Some(Value::String(s)) if s == "claimed" || s == "active"))
}

/// Publish state and related queue changes in one transaction. A repeat of the
/// same prepared write is acknowledged without refreshing TTL or republishing
/// a pending key that another worker may already have claimed.
pub(crate) fn write(
    handle: &Value,
    key: &str,
    expected: Option<&Snapshot>,
    next: &Snapshot,
    ttl: Option<i64>,
    owner: &str,
    remove: &[String],
    pending: Option<&str>,
    require_lease: bool,
) -> Result<bool> {
    // Invalid EX arguments fail during EXEC, not queueing: reject them before
    // any transaction can mutate related keys.
    if ttl.is_some_and(|t| {
        t <= 0
            || now_unix()
                .checked_add(t)
                .and_then(|s| s.checked_mul(1000))
                .is_none()
    }) {
        return Err(error(()));
    }
    match get_backend_type(handle)? {
        KVBackend::SQLite => {
            let store = get_sqlite_kv(handle)?;
            let mut store = store.lock().map_err(error)?;
            let tx = store
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(error)?;
            let current = sql_read(&tx, key)?;
            if current.as_ref() == Some(next) {
                if require_lease && execution_receipt(next) {
                    return job_leases::sql_transition(&tx, key, expected, next, true);
                }
                return Ok(true);
            }
            if current.as_ref() != expected {
                return Ok(false);
            }
            if !job_leases::sql_transition(&tx, key, expected, next, require_lease)? {
                return Ok(false);
            }
            tx.execute("INSERT INTO _kv(key,value,type,expires_at) VALUES(?,?,?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value,type=excluded.type,expires_at=excluded.expires_at",params![key,next.raw,next.kind,ttl.map(|t|now_unix()+t)]).map_err(error)?;
            for key in remove {
                tx.execute(
                    "DELETE FROM _kv WHERE key=? AND value=? AND type='string'",
                    params![key, owner],
                )
                .map_err(error)?;
            }
            if let Some(key) = pending {
                tx.execute("INSERT INTO _kv(key,value,type,expires_at) VALUES(?,?,'string',NULL) ON CONFLICT(key) DO UPDATE SET value=excluded.value,type='string',expires_at=NULL",params![key,owner]).map_err(error)?;
            }
            tx.commit().map_err(error)?;
            Ok(true)
        }
        KVBackend::Redis => redis_call(handle, |conn| {
            // WATCH covers ownership as well as state. MULTI checks command/key
            // permissions while queueing; EXECABORT leaves every key unchanged.
            // Unlike a multi-write Lua script, denied commands cannot leave a
            // committed write ID that falsely certifies missing queue work.
            let mut watch = redis::cmd("WATCH");
            watch.arg(key).arg(format!("{key}:__type"));
            let touches_ready =
                pending.is_some() || remove.iter().any(|key| job_leases::is_pending_key(key));
            for key in remove {
                watch.arg(key);
            }
            if let Some(key) = pending {
                watch.arg(key);
            }
            watch.query::<()>(conn)?;
            let result = (|| {
                if touches_ready {
                    let kind: String = redis::cmd("TYPE").arg(job_leases::READY).query(conn)?;
                    if kind != "none" && kind != "zset" {
                        return Err(redis::RedisError::from((
                            redis::ErrorKind::TypeError,
                            "job ready index has the wrong Redis type",
                        )));
                    }
                }
                let (raw, kind): (Option<String>, Option<String>) = redis::cmd("MGET")
                    .arg(key)
                    .arg(format!("{key}:__type"))
                    .query(conn)?;
                let current = raw.map(|raw| Snapshot {
                    raw,
                    kind,
                    redis: true,
                });
                if current.as_ref() == Some(next) {
                    if require_lease && execution_receipt(next) {
                        let mut fence = redis::pipe();
                        if !job_leases::redis_transition(
                            conn, &mut fence, key, expected, next, true,
                        )? {
                            return Ok(false);
                        }
                        // A receipt must not replay writes. Still EXEC a read-only
                        // transaction so expiry or a changed watched snapshot
                        // between validation and acknowledgement aborts the fence.
                        fence.clear();
                        fence.atomic().cmd("PING").ignore();
                        return redis_commit(conn, &fence);
                    }
                    return Ok(true);
                }
                if current.as_ref() != expected {
                    return Ok(false);
                }
                let owners: Vec<Option<String>> = if remove.is_empty() {
                    vec![]
                } else {
                    redis::cmd("MGET").arg(remove).query(conn)?
                };
                let mut tx = redis::pipe();
                tx.atomic();
                if !job_leases::redis_transition(conn, &mut tx, key, expected, next, require_lease)?
                {
                    return Ok(false);
                }
                tx.cmd("SET").arg(key).arg(&next.raw);
                if let Some(ttl) = ttl {
                    tx.arg("EX").arg(ttl);
                }
                tx.ignore().del(format!("{key}:__type")).ignore();
                for (key, value) in remove.iter().zip(owners) {
                    if value.as_deref() == Some(owner) {
                        tx.cmd("DEL").arg(key).arg(format!("{key}:__type")).ignore();
                        job_leases::remove_ready(&mut tx, key);
                    }
                }
                if let Some(key) = pending {
                    tx.set(key, owner).ignore();
                    job_leases::add_ready(&mut tx, key);
                }
                redis_commit(conn, &tx)
            })();
            let _ = redis::cmd("UNWATCH").query::<()>(conn);
            result
        }),
    }
}

fn redis_commit(conn: &mut redis::Connection, tx: &redis::Pipeline) -> redis::RedisResult<bool> {
    let committed: Option<()> = tx.query(conn)?;
    committed.map(|_| true).ok_or_else(|| {
        redis::RedisError::from((
            redis::ErrorKind::ResponseError,
            "job state transaction conflicted; retry",
        ))
    })
}

#[cfg(test)]
pub(crate) fn check_redis_reconnect(handle: &Value) {
    let key = "conditional-reconnect-test";
    let next = Snapshot::prepare(handle, &Value::String("prepared".into())).unwrap();
    {
        let store = get_redis_kv(handle).unwrap();
        let mut store = store.lock().unwrap();
        redis::cmd("QUIT").query::<String>(&mut store.conn).unwrap();
    }
    assert!(write(
        handle,
        key,
        None,
        &next,
        Some(60),
        "owner",
        &[],
        None,
        false
    )
    .is_err());
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    // Handoff must also heal ordinary KV callers, without a conditional poll.
    while kv_get(handle, key).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "reconnect did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(write(
        handle,
        key,
        None,
        &next,
        Some(60),
        "owner",
        &[],
        None,
        false
    )
    .unwrap());
    assert_eq!(read(handle, key).unwrap(), Some(next));
    kv_del(handle, key).unwrap();
}

#[cfg(test)]
pub(crate) fn check_redis_acl_abort(handle: &Value) {
    let key = "conditional-acl:data";
    let pending = "conditional-acl:pending";
    let unique = "conditional-acl:unique";
    let old = Snapshot::prepare(handle, &Value::String("old".into())).unwrap();
    let next = Snapshot::prepare(handle, &Value::String("new".into())).unwrap();
    assert!(write(handle, key, None, &old, None, "owner", &[], None, false).unwrap());
    kv_set(handle, unique, &Value::String("owner".into()), Some(60)).unwrap();
    let name = format!("ntnt-test-{}", uuid::Uuid::new_v4());
    let store = get_redis_kv(handle).unwrap();
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&name)
        .arg("reset")
        .arg("on")
        .arg(">fixture-only")
        .arg("~*")
        .arg("+@all")
        .arg("-set")
        .arg("-type")
        .arg("(+set ~conditional-acl:data)")
        .query::<()>(&mut store.lock().unwrap().conn)
        .unwrap();
    let Value::Map(m) = handle else { panic!() };
    let Some(Value::String(raw_url)) = m.get("_url") else {
        panic!()
    };
    let mut url = reqwest::Url::parse(raw_url).unwrap();
    url.set_username(&name).unwrap();
    url.set_password(Some("fixture-only")).unwrap();
    let restricted = open_kv(url.as_str()).unwrap();
    // Primary SET is allowed, pending SET denied. EXEC must abort all changes.
    assert!(write(
        &restricted,
        key,
        Some(&old),
        &next,
        None,
        "owner",
        &[unique.into()],
        Some(pending),
        false
    )
    .is_err());
    assert_eq!(read(handle, key).unwrap(), Some(old));
    assert!(matches!(kv_get(handle,unique).unwrap(),Value::String(s) if s=="owner"));
    assert!(matches!(kv_get(handle, pending).unwrap(), Value::Unit));
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&name)
        .arg("+set")
        .arg("-exec")
        .query::<()>(&mut store.lock().unwrap().conn)
        .unwrap();
    assert!(write(
        &restricted,
        key,
        read(handle, key).unwrap().as_ref(),
        &next,
        None,
        "owner",
        &[unique.into()],
        Some(pending),
        false
    )
    .is_err());
    assert!(matches!(kv_get(handle,key).unwrap(),Value::String(s) if s=="old"));
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&name)
        .arg("+exec")
        .arg("+type")
        .query::<()>(&mut store.lock().unwrap().conn)
        .unwrap();
    assert!(write(
        &restricted,
        key,
        read(handle, key).unwrap().as_ref(),
        &next,
        None,
        "owner",
        &[unique.into()],
        Some(pending),
        false
    )
    .unwrap());
    assert!(matches!(kv_get(handle,pending).unwrap(),Value::String(s) if s=="owner"));
    redis::cmd("ACL")
        .arg("DELUSER")
        .arg(&name)
        .query::<i64>(&mut store.lock().unwrap().conn)
        .unwrap();
}

#[cfg(test)]
pub(crate) fn check_stalled_reconnect(handle: &Value) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (ready, accepted) = std::sync::mpsc::channel();
    let (release, finish) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        ready.send(()).unwrap();
        let _ = finish.recv_timeout(Duration::from_secs(5));
        drop(socket);
    });
    let store = get_redis_kv(handle).unwrap();
    redis::cmd("QUIT")
        .query::<String>(&mut store.lock().unwrap().conn)
        .unwrap();
    let Value::Map(mut fake) = handle.clone() else {
        panic!()
    };
    fake.insert(
        "_url".into(),
        Value::String(format!("redis://{address}/15")),
    );
    let fake = Value::Map(fake);
    assert!(read(&fake, "anything").is_err());
    accepted.recv_timeout(Duration::from_secs(3)).unwrap();
    let start = std::time::Instant::now();
    for _ in 0..10 {
        assert!(read(&fake, "anything").is_err());
    }
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "a stalled setup must not block callers"
    );
    assert!(
        store.lock().unwrap().reconnecting,
        "retain one connector, not one per retry"
    );
    release.send(()).unwrap();
    server.join().unwrap();
}

/// Release only the caller's reservation, never one acquired after its TTL.
pub(crate) fn release(handle: &Value, key: &str, owner: &str) -> Result<bool> {
    match get_backend_type(handle)? {
        KVBackend::SQLite => get_sqlite_kv(handle)?
            .lock()
            .map_err(error)?
            .conn
            .execute(
                "DELETE FROM _kv WHERE key=? AND value=? AND type='string'",
                params![key, owner],
            )
            .map(|n| n > 0)
            .map_err(error),
        KVBackend::Redis => redis_call(handle, |conn| {
            redis::Script::new("if redis.call('GET',KEYS[1])==ARGV[1] then return redis.call('DEL',KEYS[1],KEYS[1]..':__type') else return 0 end").key(key).arg(owner).invoke::<i64>(conn).map(|n|n>0)
        }),
    }
}
