//! Maintenance never borrows a worker's Redis connection. A deadline spans
//! connection setup (including auth/SELECT), policy, pruning and backlog checks.
use super::*;
const IO_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

pub(crate) fn step(handle: &Value, now: i64) -> Result<(usize, bool, Policy)> {
    if get_backend_type(handle)? == KVBackend::SQLite {
        let p = policy(handle, None)?;
        let deleted = maintain(handle, &p, now)?;
        return Ok((deleted, has_work(handle, &p, now)?, p));
    }
    let Value::Map(h) = handle else {
        return Err(storage_error(()));
    };
    let url = field(h, "_url").replacen("valkey://", "redis://", 1);
    let client = redis::Client::open(url).map_err(storage_error)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(storage_error)?;
    let result = runtime.block_on(async {
        tokio::time::timeout(IO_DEADLINE, async {
            let mut conn = client
                .get_multiplexed_async_connection()
                .await
                .map_err(storage_error)?;
            let script = redis::Script::new(include_str!("../job_retention.lua"));
            let raw: String = script
                .arg(
                    serde_json::json!({"op":"policy","replace":false,"policy":Policy::default()})
                        .to_string(),
                )
                .invoke_async(&mut conn)
                .await
                .map_err(storage_error)?;
            let json: serde_json::Value = serde_json::from_str(&raw).map_err(storage_error)?;
            let p = Policy::parse(Some(&json_to_value_public(&json)))?;
            let count: String = script
                .arg(serde_json::json!({"op":"maintain","policy":p,"now":now}).to_string())
                .invoke_async(&mut conn)
                .await
                .map_err(storage_error)?;
            let more: String = script
                .arg(serde_json::json!({"op":"has_work","policy":p,"now":now}).to_string())
                .invoke_async(&mut conn)
                .await
                .map_err(storage_error)?;
            Ok((
                count.parse::<usize>().map_err(storage_error)?,
                more == "1",
                p,
            ))
        })
        .await
        .map_err(storage_error)?
    });
    // A timed-out DNS lookup may live in Tokio's blocking pool. Never join it
    // during worker shutdown; dropping the runtime cancels its async I/O tasks.
    runtime.shutdown_timeout(std::time::Duration::ZERO);
    result
}

/// Recover a metadata write using a fresh connection after the worker's
/// connection reports an error. The prepared revision makes retries idempotent.
pub(crate) fn command(handle: &Value, args: serde_json::Value) -> Result<String> {
    let Value::Map(h) = handle else {
        return Err(storage_error(()));
    };
    let client = redis::Client::open(field(h, "_url").replacen("valkey://", "redis://", 1))
        .map_err(storage_error)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(storage_error)?;
    let result = runtime.block_on(async {
        tokio::time::timeout(IO_DEADLINE, async {
            let mut connection = client
                .get_multiplexed_async_connection()
                .await
                .map_err(storage_error)?;
            redis::Script::new(include_str!("../job_retention.lua"))
                .arg(args.to_string())
                .invoke_async(&mut connection)
                .await
                .map_err(storage_error)
        })
        .await
        .map_err(storage_error)?
    });
    runtime.shutdown_timeout(std::time::Duration::ZERO);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redis_maintenance_deadline_covers_a_stalled_handshake() {
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = server.local_addr().unwrap().port();
        let (release, wait) = std::sync::mpsc::channel();
        let peer = std::thread::spawn(move || {
            let (stream, _) = server.accept().unwrap();
            let _ = wait.recv();
            drop(stream);
        });
        let handle = Value::Map(HashMap::from([
            ("_backend".into(), Value::String("redis".into())),
            (
                "_url".into(),
                Value::String(format!("redis://127.0.0.1:{port}/0")),
            ),
        ]));
        let started = std::time::Instant::now();
        let result = step(&handle, 0);
        let elapsed = started.elapsed();
        let _ = release.send(());
        peer.join().unwrap();
        assert!(result.is_err());
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "unbounded handshake: {elapsed:?}"
        );
    }
}
