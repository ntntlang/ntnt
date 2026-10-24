use super::*;

fn isolated_runtime() -> ConcurrencyRuntime {
    let mut runtime = ConcurrencyRuntime::new();
    runtime.retention.history_ttl = Duration::from_secs(60);
    runtime
}

fn snapshot(runtime: &ConcurrencyRuntime) -> HashMap<String, i64> {
    let Value::Map(fields) = runtime.task_stats().unwrap() else {
        panic!("statistics must be a map");
    };
    let keys = [
        "active_tasks",
        "tracked_tasks",
        "retained_results",
        "retained_result_bytes",
        "history_records",
        "history_bytes",
    ];
    assert_eq!(
        fields.len(),
        keys.len(),
        "output must have fixed cardinality"
    );
    for key in keys {
        assert!(fields.contains_key(key), "missing {key}");
    }
    fields
        .into_iter()
        .map(|(name, value)| match value {
            Value::Int(count) if count >= 0 => (name, count),
            _ => panic!("statistics may contain only nonnegative integers, never task contents"),
        })
        .collect()
}

fn publish(runtime: &ConcurrencyRuntime, result: Result<Value>, at: Instant) -> u64 {
    let id = runtime.register_task(Arc::new(CancelToken::new())).unwrap();
    let arcs = runtime.get_task_arcs(id).unwrap().unwrap();
    runtime.active_tasks.fetch_add(1, AtomicOrdering::Release);
    runtime.finish_task(id, Ok(result), &arcs.inner, &arcs.completed_notify, at);
    id
}

#[test]
fn task_stats_accounts_for_retained_success_errors_and_consumption_without_disclosure() {
    let runtime = isolated_runtime();
    assert!(snapshot(&runtime).values().all(|count| *count == 0));
    let success = publish(
        &runtime,
        Ok(Value::String("private-result-marker".repeat(32))),
        Instant::now(),
    );
    let failure = publish(
        &runtime,
        Err(IntentError::runtime_error(
            "private-error-marker".repeat(16),
        )),
        Instant::now(),
    );
    let retained = snapshot(&runtime);
    assert_eq!(retained["active_tasks"], 0);
    assert_eq!(retained["tracked_tasks"], 2);
    assert_eq!(retained["retained_results"], 2);
    assert_eq!(retained["history_records"], 0);
    assert_eq!(retained["history_bytes"], 0);
    {
        let tasks = runtime.tasks.lock().unwrap();
        let expected: usize = tasks
            .entries
            .values()
            .map(|entry| entry.retained_bytes)
            .sum();
        assert!(expected > 0);
        assert_eq!(retained["retained_result_bytes"], expected as i64);
    }
    runtime.await_task(success).unwrap();
    runtime.await_task(failure).unwrap();
    let consumed = snapshot(&runtime);
    assert_eq!(consumed["tracked_tasks"], 0);
    assert_eq!(consumed["retained_results"], 0);
    assert_eq!(consumed["retained_result_bytes"], 0);
    assert_eq!(consumed["history_records"], 2);
    assert_eq!(
        consumed["history_bytes"],
        (HISTORY_BASE_BYTES + 2 * HISTORY_RECORD_BYTES) as i64
    );
}

#[test]
fn task_stats_does_not_refresh_consume_or_evict_results() {
    let mut runtime = isolated_runtime();
    let at = Instant::now();
    let id = publish(&runtime, Ok(Value::Int(42)), at);
    let retained = snapshot(&runtime);
    // Make this stored result eligible for the next normal enforcement pass.
    // Inspection itself must neither enforce policy nor refresh its last-check time.
    runtime.retention.result_ttl = Duration::ZERO;
    for _ in 0..32 {
        assert_eq!(snapshot(&runtime), retained);
    }
    {
        let tasks = runtime.tasks.lock().unwrap();
        assert_eq!(tasks.entries[&id].last_checked_at, Some(at));
        assert!(tasks.entries[&id].inner.lock().unwrap().result.is_some());
    }
    runtime.reap_tasks_at(at + Duration::from_secs(1));
    let expired = snapshot(&runtime);
    assert_eq!(expired["tracked_tasks"], 0);
    assert_eq!(expired["retained_results"], 0);
    assert_eq!(expired["retained_result_bytes"], 0);
    assert_eq!(expired["history_records"], 1);
}

#[test]
fn task_stats_matches_bounded_accounting_under_load_and_after_history_eviction() {
    let mut runtime = isolated_runtime();
    runtime.retention.result_count = 3;
    runtime.retention.history_count = 4;
    for _ in 0..64 {
        publish(
            &runtime,
            Ok(Value::String("held".repeat(16))),
            Instant::now(),
        );
        let observed = snapshot(&runtime);
        let tasks = runtime.tasks.lock().unwrap();
        assert_eq!(observed["tracked_tasks"], tasks.entries.len() as i64);
        assert_eq!(observed["retained_results"], tasks.results.len() as i64);
        assert_eq!(observed["retained_result_bytes"], tasks.result_bytes as i64);
        assert_eq!(observed["history_records"], tasks.history.len() as i64);
    }
    let held = snapshot(&runtime);
    assert_eq!(held["active_tasks"], 0);
    assert_eq!(held["retained_results"], 3);
    assert_eq!(held["history_records"], 4);
    assert_eq!(
        held["history_bytes"],
        (HISTORY_BASE_BYTES + 4 * HISTORY_RECORD_BYTES) as i64
    );
    runtime.retention.result_bytes = 0;
    runtime.reap_tasks_at(Instant::now());
    let byte_evicted = snapshot(&runtime);
    assert_eq!(byte_evicted["tracked_tasks"], 0);
    assert_eq!(byte_evicted["retained_results"], 0);
    assert_eq!(byte_evicted["retained_result_bytes"], 0);
    assert_eq!(byte_evicted["history_records"], 4);
    runtime.reap_tasks_at(Instant::now() + Duration::from_secs(61));
    assert!(snapshot(&runtime).values().all(|count| *count == 0));
}

#[test]
fn task_stats_keeps_forgotten_but_unfinished_work_in_active_accounting() {
    let runtime = isolated_runtime();
    let id = runtime.register_task(Arc::new(CancelToken::new())).unwrap();
    let arcs = runtime.get_task_arcs(id).unwrap().unwrap();
    runtime.active_tasks.fetch_add(1, AtomicOrdering::Release);
    runtime.forget_task(id);
    let forgotten = snapshot(&runtime);
    assert_eq!(forgotten["active_tasks"], 1);
    assert_eq!(forgotten["tracked_tasks"], 0);
    assert_eq!(forgotten["retained_results"], 0);
    assert_eq!(forgotten["history_records"], 0);
    runtime.finish_task(
        id,
        Ok(Ok(Value::Int(1))),
        &arcs.inner,
        &arcs.completed_notify,
        Instant::now(),
    );
    assert!(snapshot(&runtime).values().all(|count| *count == 0));
}

#[test]
fn task_stats_reports_poisoned_registry_instead_of_fabricating_zero_counts() {
    let runtime = isolated_runtime();
    let _ = catch_unwind(AssertUnwindSafe(|| {
        let _guard = runtime.tasks.lock().unwrap();
        panic!("poison the owned test registry");
    }));
    let error = runtime.task_stats().unwrap_err().to_string();
    assert!(error.contains("Task registry poisoned"), "{error}");
}

#[test]
fn task_stats_rejects_counts_outside_ntnt_int_range() {
    let runtime = isolated_runtime();
    runtime
        .active_tasks
        .store(u64::MAX, AtomicOrdering::Release);
    let error = runtime.task_stats().unwrap_err().to_string();
    assert!(error.contains("exceeds NTNT Int range"), "{error}");
}
