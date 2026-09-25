use super::*;
use std::time::{Duration, Instant};
#[test]
fn requeue_detaches_attempt_before_worker_backoff() {
    let parent = Arc::new(CancelToken::new());
    CURRENT_CANCEL_TOKEN.with(|c| *c.borrow_mut() = Some(parent.clone()));
    let h = kv::open_kv(":memory:").unwrap();
    let sent = Instant::now();
    let c = seed(&h);
    let keeper = leases::Keeper::new(extract_kv_handle_info(&h).unwrap()).unwrap();
    let mut scope = keeper.attach(&c, sent, leases::Policy { duration_ms: 500 });
    let mut expected = Some(c.snapshot);
    reenqueue_job(&h, &mut expected, &c.data, &c.id, &mut scope);
    assert!(scope.cancel.is_cancelled());
    assert!(!sleep_cancellable(Duration::from_millis(600)));
    assert!(!parent.is_cancelled());
    drop(scope);
    parent.cancel();
    assert!(is_current_task_cancelled());
    CURRENT_CANCEL_TOKEN.with(|c| *c.borrow_mut() = None);
}

#[test]
fn deferring_a_legacy_failed_ready_job_keeps_it_runnable() {
    let h = kv::open_kv(":memory:").unwrap();
    let sent = Instant::now();
    let c = seed_status(&h, "failed");
    let keeper = leases::Keeper::new(extract_kv_handle_info(&h).unwrap()).unwrap();
    let mut scope = keeper.attach(&c, sent, leases::Policy { duration_ms: 500 });
    let mut expected = Some(c.snapshot);
    reenqueue_job(&h, &mut expected, &c.data, &c.id, &mut scope);
    let Value::Map(data) = kv::kv_get(&h, "jobs:data:lease-fixture").unwrap() else {
        panic!()
    };
    assert!(matches!(data.get("status"),Some(Value::String(s)) if s=="pending"));
    let Some(Value::String(pk)) = data.get("pending_key") else {
        panic!()
    };
    assert!(matches!(kv::kv_get(&h,pk).unwrap(),Value::String(s) if s==c.id));
    assert_eq!(kv::kv_ttl(&h, "jobs:data:lease-fixture").unwrap(), None);
}
#[test]
fn inspection_refreshes_a_previous_attempt_snapshot() {
    let h = kv::open_kv(":memory:").unwrap();
    let c = seed(&h);
    let mut previous = c.data;
    previous.insert("claim_token".into(), Value::String("old-attempt".into()));
    leases::inspect(&h, &mut previous).unwrap();
    assert!(matches!(previous.get("claim_token"),Some(Value::String(s)) if s==&c.token));
    assert!(matches!(previous.get("status"),Some(Value::String(s)) if s=="claimed"));
}
fn seed(h: &Value) -> kv::job_leases::Claim {
    seed_status(h, "pending")
}
fn seed_status(h: &Value, status: &str) -> kv::job_leases::Claim {
    seed_status_with_lease(h, status, 500)
}
fn seed_status_with_lease(h: &Value, status: &str, duration_ms: i64) -> kv::job_leases::Claim {
    let id = "lease-fixture";
    let pk = "jobs:pending:050:00000000000000000000:lease-fixture";
    let data = HashMap::from([
        ("id".into(), Value::String(id.into())),
        ("status".into(), Value::String(status.into())),
        ("pending_key".into(), Value::String(pk.into())),
    ]);
    kv::kv_set(h, &format!("jobs:data:{id}"), &Value::Map(data), None).unwrap();
    kv::kv_set(h, pk, &Value::String(id.into()), None).unwrap();
    kv::job_leases::claim(
        h,
        "jobs:pending:",
        "jobs:pending:zzz",
        "keeper",
        duration_ms,
    )
    .unwrap()
    .unwrap()
}
fn recover_fixture(h: &Value) -> kv::job_leases::RecoveryCounts {
    // Lease operations intentionally fast-fail on handle/SQLite contention.
    // The keeper uses this handle concurrently, so retry the observation rather
    // than assuming every recovery poll succeeds. Persistent errors still fail.
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        match kv::job_leases::recover(h, 64) {
            Ok(counts) => return counts,
            Err(error) => {
                assert!(
                    Instant::now() < until,
                    "recovery remained unavailable: {error}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
#[test]
#[should_panic(expected = "recovery remained unavailable")]
fn recovery_fixture_does_not_hide_permanent_storage_failure() {
    recover_fixture(&Value::Unit);
}
#[test]
fn recovery_fixture_handles_transient_sqlite_writer_contention() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recovery-contention.db");
    let h = kv::open_kv(path.to_str().unwrap()).unwrap();
    // This fixture tests contention, not renewal: keep its lease longer than
    // the recovery observation's one-second retry budget.
    let c = seed_status_with_lease(&h, "pending", 10_000);
    let locked = rusqlite::Connection::open(&path).unwrap();
    locked.execute_batch("BEGIN IMMEDIATE").unwrap();
    // Fast failure is the storage API contract, not a failed lease renewal.
    assert!(kv::job_leases::recover(&h, 64).is_err());
    let release = std::thread::spawn(move || {
        // Exceed the former 500 ms lease to guard against expiry masquerading
        // as a contention-handling failure after delayed CI scheduling.
        std::thread::sleep(Duration::from_millis(600));
        locked.execute_batch("ROLLBACK").unwrap();
    });
    let counts = recover_fixture(&h);
    release.join().unwrap();
    assert_eq!((counts.requeued, counts.unknown), (0, 0));
    assert_eq!(
        kv::conditional::read(&h, "jobs:data:lease-fixture")
            .unwrap()
            .unwrap(),
        c.snapshot
    );
}
#[test]
fn keeper_renews_claim_without_changing_primary_snapshot() {
    let h = kv::open_kv(":memory:").unwrap();
    // Use the shortest public lease policy rather than requiring a loaded CI
    // scheduler to service a test-only 500 ms lease. Observe real renewal below.
    let duration_ms = 10_000;
    let sent = Instant::now();
    let c = seed_status_with_lease(&h, "pending", duration_ms);
    let keeper = leases::Keeper::new(extract_kv_handle_info(&h).unwrap()).unwrap();
    let scope = keeper.attach(&c, sent, leases::Policy { duration_ms });
    let renewal_deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let counts = recover_fixture(&h);
        assert_eq!((counts.requeued, counts.unknown), (0, 0));
        assert!(!scope.cancel.is_cancelled());
        // This read can also contend with the renewing keeper. A successful
        // read must prove a later persisted deadline, not just an unexpired job.
        if let Ok(Some(lease)) = kv::job_leases::get(&h, &c.id) {
            if lease.deadline_ms > c.deadline_ms {
                break;
            }
        }
        assert!(
            Instant::now() < renewal_deadline,
            "keeper did not renew lease"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let current = kv::conditional::read(&h, "jobs:data:lease-fixture")
        .unwrap()
        .unwrap();
    assert_eq!(current, c.snapshot);
    drop(scope);
    assert!(!is_current_task_cancelled());
    let expiry_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let counts = recover_fixture(&h);
        assert_eq!(counts.unknown, 0);
        if counts.requeued == 1 {
            break;
        }
        assert_eq!(counts.requeued, 0);
        assert!(
            Instant::now() < expiry_deadline,
            "detached claim did not expire"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[test]
fn expired_child_stops_even_when_backend_cannot_renew() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lease.db");
    let h = kv::open_kv(path.to_str().unwrap()).unwrap();
    let sent = Instant::now();
    // Leave enough startup headroom for loaded Windows CI before exercising
    // expiry while the backend is locked.
    let duration_ms = 3_000;
    let c = seed_status_with_lease(&h, "pending", duration_ms);
    let mut active = c.data.clone();
    active.insert("status".into(), Value::String("active".into()));
    assert!(history::Prepared::new(
        &h,
        "jobs:data:lease-fixture",
        Some(c.snapshot.clone()),
        active
    )
    .unwrap()
    .apply_owned(&h)
    .unwrap());
    let keeper = leases::Keeper::new(extract_kv_handle_info(&h).unwrap()).unwrap();
    let scope = keeper.attach(&c, sent, leases::Policy { duration_ms });
    let locked = rusqlite::Connection::open(path).unwrap();
    locked.execute_batch("BEGIN IMMEDIATE").unwrap();
    assert!(scope.cancel.wait_timeout(Duration::from_secs(5)));
    assert!(sent.elapsed() < Duration::from_millis(4_500));
    locked.execute_batch("ROLLBACK").unwrap();
    drop(scope);
    assert!(!is_current_task_cancelled());
    // Local send-time deadline intentionally expires before the store deadline.
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        if kv::job_leases::recover(&h, 64).unwrap().unknown == 1 {
            break;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn legacy_active_is_visible_as_unknown_and_not_retryable() {
    let h = kv::open_kv(":memory:").unwrap();
    *JOB_RUNTIME.kv_handle_info.lock().unwrap() = Some(extract_kv_handle_info(&h).unwrap());
    let mut data = HashMap::from([
        ("id".into(), Value::String("legacy".into())),
        ("status".into(), Value::String("active".into())),
    ]);
    kv::kv_set(&h, "jobs:data:legacy", &Value::Map(data.clone()), None).unwrap();
    leases::inspect(&h, &mut data).unwrap();
    assert!(matches!(data.get("status"),Some(Value::String(s)) if s=="outcome_unknown"));
    assert!(matches!(
        retry_job_by_id("legacy").unwrap(),
        RetryResult::NotRetryable(_)
    ));
    assert_eq!(job_status_counts().unwrap().outcome_unknown, 1);
    assert_eq!(kv::job_leases::recover(&h, 64).unwrap().requeued, 0);
}
