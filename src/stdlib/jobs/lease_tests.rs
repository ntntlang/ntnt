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
    retrying("inspection", || leases::inspect(&h, &mut previous));
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
    retrying("seed claim", || {
        kv::job_leases::claim(
            h,
            "jobs:pending:",
            "jobs:pending:zzz",
            "keeper",
            duration_ms,
        )
    })
    .expect("seeded job was not claimable")
}
/// Lease operations intentionally fast-fail on contention: the SQLite path
/// only try-locks the process-wide store registry, which parallel tests and
/// keepers hold briefly. Retry for up to a second; persistent errors still
/// fail the test.
fn retrying<T>(what: &str, mut op: impl FnMut() -> crate::error::Result<T>) -> T {
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        match op() {
            Ok(value) => return value,
            Err(error) => {
                assert!(
                    Instant::now() < until,
                    "{what} remained unavailable: {error}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}
fn recover_fixture(h: &Value) -> kv::job_leases::RecoveryCounts {
    retrying("recovery", || kv::job_leases::recover(h, 64))
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
/// Hold a real SQLite writer until the keeper reports a failed renewal, then
/// release it. No sleep is used to guess whether the keeper tried to renew.
fn keeper_after_contention(lost: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("keeper-contention.db");
    let h = kv::open_kv(path.to_str().unwrap()).unwrap();
    let duration_ms = 10_000;
    let normal_interval = Duration::from_millis((duration_ms / 3) as u64);
    let sent = Instant::now();
    let c = seed_status_with_lease(&h, "pending", duration_ms);
    let keeper = leases::Keeper::new(extract_kv_handle_info(&h).unwrap()).unwrap();
    let renewals = keeper.observe_renewals();
    let locked = rusqlite::Connection::open(path).unwrap();
    locked.execute_batch("BEGIN IMMEDIATE").unwrap();
    let scope = keeper.attach(&c, sent, leases::Policy { duration_ms });
    let (failed_at, result) = renewals
        .recv_timeout(Duration::from_secs(8))
        .expect("keeper never attempted renewal while the store was locked");
    let error = result.expect_err("locked store unexpectedly allowed renewal");
    assert!(kv::conditional::is_contention_error(&error), "{error}");
    assert!(!scope.cancel.is_cancelled());
    if lost {
        // Simulate ownership disappearing before the retry, using the same
        // held transaction so the keeper cannot race this fixture change.
        assert_eq!(
            locked
                .execute(
                    "DELETE FROM _kv WHERE key = ?",
                    [format!("jobs:lease:{}", c.id)]
                )
                .unwrap(),
            1
        );
    }
    locked.execute_batch("COMMIT").unwrap();

    // Half the normal interval leaves scheduling headroom for the 25 ms
    // retry but cannot admit the pre-#239 full-interval retry. Use the failed
    // attempt's timestamp, not when this test thread happened to receive it.
    let retry_deadline = failed_at + normal_interval / 2;
    let renewed_at = loop {
        let (at, result) = renewals
            .recv_timeout(retry_deadline.saturating_duration_since(Instant::now()))
            .expect("keeper did not retry contention before the next normal interval");
        match result {
            Ok(renewed) => {
                assert_eq!(renewed, !lost);
                break at;
            }
            // Parallel tests can briefly hold the process-wide store registry.
            Err(error) => assert!(kv::conditional::is_contention_error(&error), "{error}"),
        }
    };
    assert!(renewed_at < retry_deadline);
    assert!(renewed_at < failed_at + normal_interval);
    assert_eq!(scope.cancel.is_cancelled(), lost);
    let lease = retrying("lease after contention", || kv::job_leases::get(&h, &c.id));
    if lost {
        assert!(lease.is_none(), "retry resurrected a lost lease");
    } else {
        let lease = lease.expect("renewed lease disappeared");
        assert_eq!(lease.token, c.token);
        assert!(lease.deadline_ms > c.deadline_ms);
        let counts = recover_fixture(&h);
        assert_eq!((counts.requeued, counts.unknown), (0, 0));
    }
    assert_eq!(
        retrying("primary after contention", || kv::conditional::read(
            &h,
            "jobs:data:lease-fixture"
        ))
        .unwrap(),
        c.snapshot
    );
    drop(scope);
    assert!(!is_current_task_cancelled());
}

#[test]
fn keeper_retries_contention_before_next_normal_interval() {
    keeper_after_contention(false);
}

#[test]
fn keeper_contention_retry_cancels_a_lost_lease() {
    keeper_after_contention(true);
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
    let renewals = keeper.observe_renewals();
    let locked = rusqlite::Connection::open(path).unwrap();
    locked.execute_batch("BEGIN IMMEDIATE").unwrap();
    let scope = keeper.attach(&c, sent, leases::Policy { duration_ms });
    let (_, result) = renewals
        .recv_timeout(Duration::from_secs(2))
        .expect("keeper never attempted renewal before expiry");
    let error = result.expect_err("locked store unexpectedly allowed renewal");
    assert!(kv::conditional::is_contention_error(&error), "{error}");
    assert!(scope.cancel.wait_timeout(Duration::from_secs(5)));
    assert!(sent.elapsed() < Duration::from_millis(4_500));
    assert!(scope.cancel.is_cancelled());
    drop(scope);
    assert!(!is_current_task_cancelled());
    // A renewal may have passed its cancellation check just before expiry.
    // Keep the writer locked until the supervisor exits: the observer channel
    // disconnects only once no renewal can still be in flight. Every attempt
    // made under the lock must have failed.
    drop(keeper);
    loop {
        match renewals.recv_timeout(Duration::from_secs(10)) {
            Ok((_, Ok(renewed))) => panic!("expired lease renewed under lock: {renewed}"),
            Ok((_, Err(_))) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                panic!("lease supervisor did not exit")
            }
        }
    }
    locked.execute_batch("ROLLBACK").unwrap();
    let lease = retrying("expired lease", || kv::job_leases::get(&h, &c.id)).unwrap();
    assert_eq!(lease.deadline_ms, c.deadline_ms);
    // Local send-time deadline intentionally expires before the store deadline.
    let until = Instant::now() + Duration::from_secs(1);
    loop {
        match kv::job_leases::recover(&h, 64) {
            Ok(counts) if counts.unknown == 1 => break,
            Ok(_) => {}
            Err(error) if kv::conditional::is_contention_error(&error) => {}
            Err(error) => panic!("unexpected recovery failure: {error}"),
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn legacy_active_is_visible_as_unknown_and_not_retryable() {
    // retry_job_by_id and job_status_counts read the global JOB_RUNTIME
    // handle; hold the shared lock so parallel runtime tests cannot replace
    // or reset it mid-test.
    let _guard = tests::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    JOB_RUNTIME.reset();
    let h = kv::open_kv(":memory:").unwrap();
    *JOB_RUNTIME.kv_handle_info.lock().unwrap() = Some(extract_kv_handle_info(&h).unwrap());
    let mut data = HashMap::from([
        ("id".into(), Value::String("legacy".into())),
        ("status".into(), Value::String("active".into())),
    ]);
    kv::kv_set(&h, "jobs:data:legacy", &Value::Map(data.clone()), None).unwrap();
    retrying("inspection", || leases::inspect(&h, &mut data));
    assert!(matches!(data.get("status"),Some(Value::String(s)) if s=="outcome_unknown"));
    assert!(matches!(
        retry_job_by_id("legacy").unwrap(),
        RetryResult::NotRetryable(_)
    ));
    assert_eq!(job_status_counts().unwrap().outcome_unknown, 1);
    assert_eq!(recover_fixture(&h).requeued, 0);
}
