//! Small conditional KV writes used to retry job state persistence safely.
//! No retention indexes, policies, backfill, or capacity accounting.
use super::*;
use rusqlite::OptionalExtension;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Snapshot {
    raw: String,
    kind: Option<String>,
    redis: bool,
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
fn error<E>(_: E) -> IntentError {
    IntentError::runtime_error("job state storage operation failed")
}
fn sql_read(conn: &Connection, key: &str) -> Result<Option<Snapshot>> {
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
fn redis_call<T>(
    handle: &Value,
    call: impl FnOnce(&mut redis::Connection) -> redis::RedisResult<T>,
) -> Result<T> {
    let shared = get_redis_kv(handle)?;
    let mut store = shared.lock().map_err(error)?;
    if store.reconnecting {
        return Err(error(()));
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
    result.map_err(error)
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
) -> Result<bool> {
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
                return Ok(true);
            }
            if current.as_ref() != expected {
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
            for key in remove {
                watch.arg(key);
            }
            if let Some(key) = pending {
                watch.arg(key);
            }
            watch.query::<()>(conn)?;
            let result = (|| {
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
                tx.cmd("SET").arg(key).arg(&next.raw);
                if let Some(ttl) = ttl {
                    tx.arg("EX").arg(ttl);
                }
                tx.ignore().del(format!("{key}:__type")).ignore();
                for (key, value) in remove.iter().zip(owners) {
                    if value.as_deref() == Some(owner) {
                        tx.cmd("DEL").arg(key).arg(format!("{key}:__type")).ignore();
                    }
                }
                if let Some(key) = pending {
                    tx.set(key, owner).ignore();
                }
                let committed: Option<()> = tx.query(conn)?;
                committed.map(|_| true).ok_or_else(|| {
                    redis::RedisError::from((
                        redis::ErrorKind::ResponseError,
                        "job state transaction conflicted; retry",
                    ))
                })
            })();
            let _ = redis::cmd("UNWATCH").query::<()>(conn);
            result
        }),
    }
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
    assert!(write(handle, key, None, &next, Some(60), "owner", &[], None).is_err());
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    // Handoff must also heal ordinary KV callers, without a conditional poll.
    while kv_get(handle, key).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "reconnect did not finish"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(write(handle, key, None, &next, Some(60), "owner", &[], None).unwrap());
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
    assert!(write(handle, key, None, &old, None, "owner", &[], None).unwrap());
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
        Some(pending)
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
        Some(pending)
    )
    .is_err());
    assert!(matches!(kv_get(handle,key).unwrap(),Value::String(s) if s=="old"));
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg(&name)
        .arg("+exec")
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
        Some(pending)
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
