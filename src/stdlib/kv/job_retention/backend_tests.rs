use super::*;

fn value(id: &str, status: &str, ms: i64) -> Value {
    let mut j =
        serde_json::json!({"id":id,"status":status,"created_at":"0","payload":{"example":"kept"}});
    let field = match status {
        "completed" => "completed_at",
        "cancelled" => "cancelled_at",
        "expired" => "expired_at",
        "dead" => "dead_at",
        _ => "failed_at",
    };
    j[field] = serde_json::json!(format!("{}", ms as i128 * 1_000_000));
    json_to_value_public(&j)
}
fn exists(h: &Value, id: &str) -> bool {
    !matches!(kv_get(h, &format!("jobs:data:{id}")).unwrap(), Value::Unit)
}
fn insert(h: &Value, id: &str, status: &str, ms: i64) -> Snapshot {
    change(
        h,
        &format!("jobs:data:{id}"),
        None,
        &value(id, status, ms),
        ms,
    )
    .unwrap()
}
fn totals(h: &Value) -> (i64, i64) {
    match get_backend_type(h).unwrap() {
        KVBackend::SQLite => get_sqlite_kv(h)
            .unwrap()
            .lock()
            .unwrap()
            .conn
            .query_row(
                "SELECT records,bytes FROM _jobs_retention_meta_v1 WHERE id=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap(),
        KVBackend::Redis => {
            let s = get_redis_kv(h).unwrap();
            let mut s = s.lock().unwrap();
            let n: i64 = s.conn.zcard("__ntnt:jobs-retention:v1:all").unwrap();
            let b: Option<i64> = s
                .conn
                .hget("__ntnt:jobs-retention:v1:state", "bytes")
                .unwrap();
            (n, b.unwrap_or(0))
        }
    }
}
fn converge(h: &Value, p: &Policy, now: i64) -> (usize, usize) {
    let mut deleted = 0;
    for passes in 1..1000 {
        let n = maintain(h, p, now).unwrap();
        assert!(n <= p.batch_size);
        deleted += n;
        if !has_work(h, p, now).unwrap() {
            return (deleted, passes);
        }
    }
    panic!("bounded passes failed to converge")
}
fn category_age(h: &Value) {
    insert(h, "old-failure", "dead", 0);
    insert(h, "younger-completion", "completed", DAY_MS);
    let p = Policy::default();
    assert_eq!(converge(h, &p, 40 * DAY_MS).0, 1);
    assert!(exists(h, "old-failure"));
    assert!(!exists(h, "younger-completion"));
    assert_eq!(converge(h, &p, 90 * DAY_MS).0, 1);
}
fn count_pressure(h: &Value) {
    for i in 0..17 {
        insert(h, &format!("count-{i:02}"), "completed", i);
    }
    let p = Policy {
        batch_size: 3,
        max_records: 2,
        ..Policy::default()
    };
    let (deleted, passes) = converge(h, &p, 20);
    assert_eq!(deleted, 15);
    assert!(passes >= 5);
    assert_eq!(totals(h).0, 2);
    assert!(exists(h, "count-15") && exists(h, "count-16"));
    eprintln!("retention count backlog: 17 records -> 2 in {passes} bounded passes (batch=3)");
}
fn byte_pressure(h: &Value) {
    let s = insert(h, "bytes-0", "completed", 0);
    insert(h, "bytes-1", "completed", 0);
    insert(h, "bytes-2", "completed", 0);
    let p = Policy {
        max_bytes: s.raw.len() as i64 * 2,
        max_records: 100,
        ..Policy::default()
    };
    assert_eq!(converge(h, &p, 1).0, 1);
    assert!(!exists(h, "bytes-0"));
    assert_eq!(totals(h), (2, p.max_bytes));
}
fn protected_and_legacy(h: &Value) {
    for status in ["pending", "scheduled", "retrying", "active", "unknown"] {
        kv_set(
            h,
            &format!("jobs:data:{status}"),
            &value(status, status, 0),
            None,
        )
        .unwrap();
    }
    let legacy = json_to_value_public(
        &serde_json::json!({"id":"legacy","status":"failed","created_at":"0"}),
    );
    kv_set(h, "jobs:data:legacy", &legacy, None).unwrap();
    kv_set(
        h,
        "jobs:data:malformed",
        &Value::String("not a map".into()),
        None,
    )
    .unwrap();
    let p = Policy::default();
    assert_eq!(converge(h, &p, 100 * DAY_MS).0, 0);
    assert_eq!(converge(h, &p, 189 * DAY_MS).0, 0);
    assert_eq!(converge(h, &p, 190 * DAY_MS).0, 1);
    for status in [
        "pending",
        "scheduled",
        "retrying",
        "active",
        "unknown",
        "malformed",
    ] {
        assert!(exists(h, status));
    }
}
fn owner_and_callback(h: &Value) {
    let mut v = value("owner", "completed", 0);
    let Value::Map(m) = &mut v else { panic!() };
    m.insert("pending_key".into(), Value::String("unrelated:key".into()));
    m.insert(
        "dedup_key".into(),
        Value::String("jobs:unique:owned".into()),
    );
    change(h, "jobs:data:owner", None, &v, 0).unwrap();
    kv_set(h, "unrelated:key", &Value::String("untouched".into()), None).unwrap();
    kv_set(
        h,
        "jobs:unique:owned",
        &Value::String("new-owner".into()),
        Some(1000),
    )
    .unwrap();
    set_batch_meta(
        h,
        "jobs:batch:b",
        &json_to_value_public(&serde_json::json!({"id":"b"})),
        None,
    )
    .unwrap();
    let cb = json_to_value_public(
        &serde_json::json!({"id":"cb-b-on_death","type":"_BatchCallback","status":"completed","payload":{"batch_id":"b","callback_type":"on_death"}}),
    );
    change(h, "jobs:data:cb-b-on_death", None, &cb, 0).unwrap();
    assert_eq!(converge(h, &Policy::default(), 31 * DAY_MS).0, 2);
    assert!(change(h, "jobs:data:cb-b-on_death", None, &cb, 0).is_err());
    assert!(matches!(kv_get(h,"unrelated:key").unwrap(),Value::String(ref v) if v=="untouched"));
    assert!(
        matches!(kv_get(h,"jobs:unique:owned").unwrap(),Value::String(ref v) if v=="new-owner")
    );
    set_batch_meta(
        h,
        "jobs:batch:b",
        &json_to_value_public(&serde_json::json!({"id":"b"})),
        Some(60),
    )
    .unwrap();
    assert!(kv_ttl(h, &callback_key("b", "on_death")).unwrap().unwrap() <= 60);
    // A matched reservation survives history removal with its absolute TTL.
    let mut v = value("unique", "completed", 0);
    let Value::Map(m) = &mut v else { panic!() };
    m.insert(
        "dedup_key".into(),
        Value::String("jobs:unique:retired".into()),
    );
    change(h, "jobs:data:unique", None, &v, 0).unwrap();
    kv_set(
        h,
        "jobs:unique:retired",
        &Value::String("unique".into()),
        Some(1000),
    )
    .unwrap();
    assert_eq!(converge(h, &Policy::default(), 31 * DAY_MS).0, 1);
    assert!(matches!(
        kv_get(h, "jobs:unique:retired").unwrap(),
        Value::Map(_)
    ));
    assert!(kv_ttl(h, "jobs:unique:retired").unwrap().unwrap() >= 990);
}
fn duplicate_backfill(h: &Value) {
    for i in 0..19 {
        kv_set(
            h,
            &format!("jobs:data:legacy-{i:02}"),
            &json_to_value_public(
                &serde_json::json!({"id":format!("legacy-{i:02}"),"status":"completed"}),
            ),
            None,
        )
        .unwrap();
    }
    let p = Policy {
        batch_size: 2,
        ..Policy::default()
    };
    maintain(h, &p, 0).unwrap();
    let before = totals(h);
    match get_backend_type(h).unwrap() {
        KVBackend::SQLite => {
            get_sqlite_kv(h)
                .unwrap()
                .lock()
                .unwrap()
                .conn
                .execute(
                    "UPDATE _jobs_retention_meta_v1 SET cursor='jobs:data:',complete=0 WHERE id=1",
                    [],
                )
                .unwrap();
        }
        KVBackend::Redis => {
            let s = get_redis_kv(h).unwrap();
            let mut s = s.lock().unwrap();
            let keys: Vec<String> = s.conn.zrange("__ntnt:jobs-retention:v1:all", 0, 1).unwrap();
            for k in keys {
                let _: usize = s
                    .conn
                    .lpush("__ntnt:jobs-retention:v1:backfill", k)
                    .unwrap();
            }
        }
    }
    maintain(h, &p, DAY_MS).unwrap();
    assert_eq!(totals(h), before);
    converge(h, &p, DAY_MS);
    assert_eq!(totals(h).0, 19);
    // Duplicate observation did not reset the original conservative timestamp.
    assert!(converge(h, &p, 30 * DAY_MS).0 >= before.0 as usize);
}
fn backfill_pressure_preserves_unseen_ordering(h: &Value) {
    insert(h, "a-recent", "completed", 10 * DAY_MS);
    insert(h, "b-newest", "completed", 11 * DAY_MS);
    for i in 0..8 {
        kv_set(
            h,
            &format!("jobs:data:z-old-{i}"),
            &value(&format!("z-old-{i}"), "completed", 0),
            None,
        )
        .unwrap();
    }
    if get_backend_type(h).unwrap() == KVBackend::Redis {
        let s = get_redis_kv(h).unwrap();
        let _: usize = s
            .lock()
            .unwrap()
            .conn
            .rpush("__ntnt:jobs-retention:v1:backfill", "jobs:data:a-recent")
            .unwrap();
    }
    let p = Policy {
        batch_size: 1,
        max_records: 1,
        ..Policy::default()
    };
    assert_eq!(
        maintain(h, &p, 11 * DAY_MS).unwrap(),
        0,
        "pressure must not evict recent records before legacy backfill completes"
    );
    assert!(exists(h, "a-recent"));
    converge(h, &p, 11 * DAY_MS);
    assert_eq!(totals(h).0, 1);
    assert!(exists(h, "b-newest"));
}

fn changed_policy_fences_stale_batches(h: &Value) {
    let old = Policy::default();
    policy(h, Some(&old)).unwrap();
    insert(h, "policy", "completed", 0);
    let disabled = Policy {
        enabled: false,
        ..old.clone()
    };
    policy(h, Some(&disabled)).unwrap();
    assert_eq!(maintain(h, &old, 31 * DAY_MS).unwrap(), 0);
    assert!(exists(h, "policy"));
    let longer = Policy {
        completed_days: 60,
        ..old.clone()
    };
    policy(h, Some(&longer)).unwrap();
    assert_eq!(maintain(h, &old, 31 * DAY_MS).unwrap(), 0);
    assert!(exists(h, "policy"));
    assert_eq!(converge(h, &longer, 61 * DAY_MS).0, 1);
}

fn suite(fixture: impl Fn() -> Value) {
    for case in [
        category_age,
        count_pressure,
        byte_pressure,
        protected_and_legacy,
        owner_and_callback,
        duplicate_backfill,
        backfill_pressure_preserves_unseen_ordering,
        changed_policy_fences_stale_batches,
    ] {
        case(&fixture());
    }
}
#[test]
fn retention_sqlite_backend_contracts() {
    suite(|| open_kv("sqlite::memory:").unwrap());
}
#[test]
#[ignore = "requires explicitly supplied disposable Redis; flushes selected DB"]
fn retention_redis_backend_contracts() {
    let url = std::env::var("NTNT_RETENTION_TEST_REDIS").unwrap();
    assert!(url.starts_with("redis://127.0.0.1:"));
    suite(|| {
        let h = open_kv(&url).unwrap();
        let s = get_redis_kv(&h).unwrap();
        let _: () = redis::cmd("FLUSHDB")
            .query(&mut s.lock().unwrap().conn)
            .unwrap();
        h
    });
}
#[test]
#[ignore = "requires explicitly supplied disposable Redis; flushes selected DB"]
fn retention_redis_maintenance_uses_a_separate_connection() {
    let url = std::env::var("NTNT_RETENTION_TEST_REDIS").unwrap();
    assert!(url.starts_with("redis://127.0.0.1:"));
    let h = open_kv(&url).unwrap();
    let store = get_redis_kv(&h).unwrap();
    let _: () = redis::cmd("FLUSHDB")
        .query(&mut store.lock().unwrap().conn)
        .unwrap();
    insert(&h, "maintenance", "completed", 0);
    let (ready, started) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _lock = store.lock().unwrap();
        ready.send(()).unwrap();
        let _ = released.recv_timeout(std::time::Duration::from_secs(5));
    });
    started.recv().unwrap();
    let began = std::time::Instant::now();
    let outcome = maintenance::step(&h, 31 * DAY_MS);
    let elapsed = began.elapsed();
    let _ = release.send(());
    holder.join().unwrap();
    assert_eq!(outcome.unwrap().0, 1);
    assert!(
        elapsed < std::time::Duration::from_secs(3),
        "maintenance waited on worker connection: {elapsed:?}"
    );
    assert!(!exists(&h, "maintenance"));
}

#[test]
fn retention_sqlite_rollback_and_restart() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("old.db").display());
    let h = open_kv(&url).unwrap();
    for i in 0..9 {
        kv_set(
            &h,
            &format!("jobs:data:old-{i}"),
            &value(&format!("old-{i}"), "completed", 0),
            None,
        )
        .unwrap();
    }
    let p = Policy {
        batch_size: 2,
        ..Policy::default()
    };
    policy(&h, Some(&p)).unwrap();
    maintain(&h, &p, 0).unwrap();
    assert_eq!(totals(&h).0, 2);
    let other = open_kv(&url).unwrap();
    assert_eq!(policy(&other, None).unwrap(), p);
    maintain(&other, &p, 0).unwrap();
    assert_eq!(totals(&h).0, 4);
    let s = get_sqlite_kv(&h).unwrap();
    s.lock().unwrap().conn.execute_batch("CREATE TRIGGER fail_prune BEFORE DELETE ON _kv WHEN OLD.key='jobs:data:old-1' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
    let before = totals(&h);
    assert!(maintain(&other, &p, 31 * DAY_MS).is_err());
    assert!(exists(&h, "old-0") && exists(&h, "old-1"));
    assert_eq!(totals(&h), before);
    s.lock()
        .unwrap()
        .conn
        .execute_batch("DROP TRIGGER fail_prune")
        .unwrap();
    assert_eq!(converge(&other, &p, 31 * DAY_MS).0, 9);
    assert_eq!(totals(&h), (0, 0));
}
fn second_handle_race(url: &str) {
    use std::sync::Barrier;
    let h = open_kv(url).unwrap();
    let snapshot = insert(&h, "race", "dead", 0);
    let read = Arc::new(Barrier::new(2));
    let pruned = Arc::new(Barrier::new(2));
    let (r, p) = (read.clone(), pruned.clone());
    let url = url.to_string();
    let retry = std::thread::spawn(move || {
        let other = open_kv(&url).unwrap();
        let snapshot = snapshot.clone();
        r.wait();
        p.wait();
        change(&other,"jobs:data:race",Some(&snapshot),&json_to_value_public(&serde_json::json!({"id":"race","status":"pending","pending_key":"jobs:pending:50:0:race"})),100*DAY_MS).is_err()
    });
    read.wait();
    assert_eq!(converge(&h, &Policy::default(), 100 * DAY_MS).0, 1);
    pruned.wait();
    assert!(retry.join().unwrap());
    assert!(!exists(&h, "race"));
    assert!(matches!(
        kv_get(&h, "jobs:pending:50:0:race").unwrap(),
        Value::Unit
    ));
    let old = insert(&h, "race-retry-wins", "dead", 0);
    let live = value("race-retry-wins", "pending", 0);
    change(
        &h,
        "jobs:data:race-retry-wins",
        Some(&old),
        &live,
        100 * DAY_MS,
    )
    .unwrap();
    assert_eq!(converge(&h, &Policy::default(), 100 * DAY_MS).0, 0);
    assert!(exists(&h, "race-retry-wins"));
}
#[test]
fn retention_sqlite_second_handle_retry_prune_race() {
    let dir = tempfile::tempdir().unwrap();
    second_handle_race(&format!("sqlite:{}", dir.path().join("race.db").display()));
}
#[test]
#[ignore = "requires explicitly supplied disposable Redis; flushes selected DB"]
fn retention_redis_second_handle_retry_prune_race() {
    let url = std::env::var("NTNT_RETENTION_TEST_REDIS").unwrap();
    assert!(url.starts_with("redis://127.0.0.1:"));
    let h = open_kv(&url).unwrap();
    let s = get_redis_kv(&h).unwrap();
    let _: () = redis::cmd("FLUSHDB")
        .query(&mut s.lock().unwrap().conn)
        .unwrap();
    second_handle_race(&url);
}
#[test]
#[ignore = "requires explicitly supplied disposable Redis; flushes selected DB"]
fn retention_redis_prevalidation_failure_is_not_partial() {
    let url = std::env::var("NTNT_RETENTION_TEST_REDIS").unwrap();
    assert!(url.starts_with("redis://127.0.0.1:"));
    let h = open_kv(&url).unwrap();
    let s = get_redis_kv(&h).unwrap();
    let _: () = redis::cmd("FLUSHDB")
        .query(&mut s.lock().unwrap().conn)
        .unwrap();
    for id in ["a", "b"] {
        let mut v = value(id, "completed", 0);
        let Value::Map(m) = &mut v else { panic!() };
        m.insert(
            "pending_key".into(),
            Value::String(format!("jobs:pending:50:0:{id}")),
        );
        insert(&h, &format!("other-{id}"), "pending", 0);
        change(&h, &format!("jobs:data:{id}"), None, &v, 0).unwrap();
    }
    let before = totals(&h);
    let _: usize = s
        .lock()
        .unwrap()
        .conn
        .lpush("jobs:pending:50:0:b", "wrongtype")
        .unwrap();
    assert!(maintain(&h, &Policy::default(), 31 * DAY_MS).is_err());
    assert!(exists(&h, "a") && exists(&h, "b"));
    assert_eq!(totals(&h), before);
    let _: usize = s.lock().unwrap().conn.del("jobs:pending:50:0:b").unwrap();
    assert_eq!(converge(&h, &Policy::default(), 31 * DAY_MS).0, 2);
}
