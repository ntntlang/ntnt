use super::*;

#[test]
fn delayed_observer_cannot_move_result_recency_backwards() {
    let runtime = ConcurrencyRuntime::new();
    let at = Instant::now();
    let (id, arcs, _) = task(&runtime, TaskKind::Spawn);
    finish(&runtime, id, &arcs, Ok(Ok(Value::Unit)), at);
    runtime
        .try_await_at(id, at + Duration::from_secs(30))
        .unwrap();
    // Represents an older observer timestamp whose registry lock acquisition
    // happens after the newer observer wins the lock.
    runtime
        .try_await_at(id, at + Duration::from_secs(10))
        .unwrap();
    runtime.reap_tasks_at(at + Duration::from_secs(3610));
    assert!(runtime.get_task_arcs(id).unwrap().is_some());
}

fn finish(
    runtime: &ConcurrencyRuntime,
    id: u64,
    arcs: &TaskArcs,
    result: std::result::Result<Result<Value>, Box<dyn std::any::Any + Send>>,
    at: Instant,
) {
    runtime.finish_task(id, result, &arcs.inner, &arcs.completed_notify, at);
}

fn task(runtime: &ConcurrencyRuntime, kind: TaskKind) -> (u64, TaskArcs, Arc<CancelToken>) {
    let token = Arc::new(CancelToken::new());
    let id = runtime
        .register_task_kind(Arc::clone(&token), kind)
        .unwrap();
    runtime.active_tasks.fetch_add(1, AtomicOrdering::Release);
    (id, runtime.get_task_arcs(id).unwrap().unwrap(), token)
}

fn status(value: Value) -> String {
    let Value::Map(map) = value else {
        panic!("status map")
    };
    let Value::String(status) = &map["status"] else {
        panic!("status string")
    };
    status.clone()
}

#[test]
fn inactivity_clock_and_polling_keep_exactly_one_recency_index() {
    let runtime = ConcurrencyRuntime::new();
    let at = Instant::now();
    let (id, arcs, _) = task(&runtime, TaskKind::Spawn);
    finish(&runtime, id, &arcs, Ok(Ok(Value::Int(42))), at);
    for offset in 1..3600 {
        assert_eq!(
            status(
                runtime
                    .try_await_at(id, at + Duration::from_secs(offset))
                    .unwrap()
            ),
            "completed"
        );
    }
    let last = at + Duration::from_secs(3599);
    runtime.reap_tasks_at(last + Duration::from_secs(3599));
    assert!(runtime.get_task_arcs(id).unwrap().is_some());
    assert_eq!(runtime.tasks.lock().unwrap().results.len(), 1);
    assert_eq!(
        status(
            runtime
                .try_await_at(id, last + Duration::from_secs(3600))
                .unwrap()
        ),
        "expired"
    );
    assert_eq!(runtime.tasks.lock().unwrap().results.len(), 0);
    assert_eq!(runtime.tasks.lock().unwrap().result_bytes, 0);
}

#[test]
fn compact_history_ttl_removes_consumed_and_expired_handles() {
    let runtime = ConcurrencyRuntime::new();
    let at = Instant::now();
    let (consumed, arcs, _) = task(&runtime, TaskKind::Spawn);
    finish(&runtime, consumed, &arcs, Ok(Ok(Value::Unit)), at);
    runtime.await_task(consumed).unwrap();
    let (expired, arcs, _) = task(&runtime, TaskKind::After);
    finish(&runtime, expired, &arcs, Ok(Ok(Value::Unit)), at);
    runtime.reap_tasks_at(at + Duration::from_secs(3600));
    let consumed_at = runtime.tasks.lock().unwrap().history[&consumed].retired_at;
    runtime.reap_tasks_at(consumed_at + Duration::from_secs(86400));
    assert!(runtime.try_await(consumed).is_err());
    assert!(runtime.tasks.lock().unwrap().history.contains_key(&expired));
    runtime.reap_tasks_at(at + Duration::from_secs(3600 + 86400));
    assert!(runtime.try_await(expired).is_err());
    let tasks = runtime.tasks.lock().unwrap();
    assert!(tasks.history.is_empty());
    assert!(tasks.history_order.is_empty());
}

#[test]
fn byte_pressure_evicts_least_recent_public_result_not_running_work() {
    let mut runtime = ConcurrencyRuntime::new();
    runtime.retention.result_bytes = 2 * std::mem::size_of::<SerializedValue>();
    let at = Instant::now();
    let (running, _, _) = task(&runtime, TaskKind::Spawn);
    let (first, arcs, _) = task(&runtime, TaskKind::Spawn);
    finish(&runtime, first, &arcs, Ok(Ok(Value::Unit)), at);
    let (second, arcs, _) = task(&runtime, TaskKind::Worker);
    finish(&runtime, second, &arcs, Ok(Ok(Value::Unit)), at);
    runtime
        .try_await_at(first, at + Duration::from_secs(1))
        .unwrap();
    let (third, arcs, _) = task(&runtime, TaskKind::After);
    finish(
        &runtime,
        third,
        &arcs,
        Ok(Ok(Value::Unit)),
        at + Duration::from_secs(2),
    );
    assert!(runtime.get_task_arcs(running).unwrap().is_some());
    assert!(runtime.get_task_arcs(first).unwrap().is_some());
    assert!(runtime.get_task_arcs(second).unwrap().is_none());
    assert!(runtime.get_task_arcs(third).unwrap().is_some());
    assert_eq!(
        runtime.tasks.lock().unwrap().result_bytes,
        runtime.retention.result_bytes
    );
    runtime.await_task(first).unwrap();
    runtime.await_task(third).unwrap();
    assert_eq!(runtime.tasks.lock().unwrap().result_bytes, 0);
    runtime.reap_tasks_at(at + Duration::from_secs(1_000_000));
    assert!(runtime.get_task_arcs(running).unwrap().is_some());
}

#[test]
fn ephemeral_delivery_is_not_subject_to_public_retention_budgets() {
    let mut runtime = ConcurrencyRuntime::new();
    runtime.retention.result_count = 0;
    runtime.retention.result_bytes = 0;
    runtime.retention.result_ttl = Duration::ZERO;
    runtime.retention.history_count = 0;
    runtime.retention.history_bytes = 0;
    let (id, arcs, _) = task(&runtime, TaskKind::Ephemeral);
    finish(
        &runtime,
        id,
        &arcs,
        Ok(Ok(Value::String("large".repeat(4096)))),
        Instant::now(),
    );
    runtime.reap_tasks_at(Instant::now() + Duration::from_secs(1_000_000));
    assert!(runtime.get_task_arcs(id).unwrap().is_some());
    assert_eq!(runtime.tasks.lock().unwrap().result_bytes, 0);
    assert!(
        matches!(runtime.await_task(id).unwrap(), Value::EnumValue { variant, values, .. } if variant == "Ok" && matches!(&values[0], Value::String(s) if s.len() == 20480))
    );
    assert!(runtime.tasks.lock().unwrap().entries.is_empty());
    assert!(runtime.tasks.lock().unwrap().history.is_empty());
}

#[test]
fn error_and_panic_delivery_are_preserved_but_history_contains_no_messages() {
    let runtime = ConcurrencyRuntime::new();
    for (result, outcome) in [
        (
            Ok(Err(IntentError::runtime_error("private-error-canary"))),
            TaskState::Failed,
        ),
        (
            Err(Box::new("private-panic-canary".to_string()) as Box<dyn std::any::Any + Send>),
            TaskState::Panicked,
        ),
    ] {
        let (id, arcs, _) = task(&runtime, TaskKind::Spawn);
        runtime.mark_task_started(id);
        finish(&runtime, id, &arcs, result, Instant::now());
        let peek = runtime.try_await(id).unwrap();
        assert!(format!("{peek:?}").contains("canary"));
        let delivered = runtime.await_task(id).unwrap();
        assert!(format!("{delivered:?}").contains("canary"));
        assert!(arcs.inner.lock().unwrap().error_msg.is_none());
        let record = runtime.tasks.lock().unwrap().history[&id];
        assert_eq!(record.id, id);
        assert_eq!(record.state, TaskState::Consumed);
        assert_eq!(record.outcome, outcome);
        assert!(record.started_at.unwrap() >= record.created_at);
        assert!(record.completed_at >= record.started_at.unwrap());
        assert_eq!(
            record.duration,
            Some(
                record
                    .completed_at
                    .duration_since(record.started_at.unwrap())
            )
        );
        assert!(!format!("{record:?}").contains("canary"));
        assert_eq!(runtime.tasks.lock().unwrap().result_bytes, 0);
    }
}

#[test]
fn error_text_is_also_subject_to_result_byte_budget() {
    let mut runtime = ConcurrencyRuntime::new();
    runtime.retention.result_bytes = 32;
    let (id, arcs, _) = task(&runtime, TaskKind::Worker);
    finish(
        &runtime,
        id,
        &arcs,
        Ok(Err(IntentError::runtime_error("private".repeat(4096)))),
        Instant::now(),
    );
    assert_eq!(status(runtime.try_await(id).unwrap()), "expired");
    assert!(arcs.inner.lock().unwrap().error_msg.is_none());
    assert_eq!(runtime.tasks.lock().unwrap().result_bytes, 0);
}

#[test]
fn result_consumption_and_expiration_release_stored_channel_senders() {
    for consume in [true, false] {
        let runtime = ConcurrencyRuntime::new();
        let (tx, rx) = crossbeam::unbounded::<SerializedValue>();
        let (id, arcs, _) = task(&runtime, TaskKind::Spawn);
        let at = Instant::now();
        let value = SerializedValue::TxChannelHandle(7, Arc::new(tx)).to_value();
        finish(&runtime, id, &arcs, Ok(Ok(value)), at);
        assert!(matches!(rx.try_recv(), Err(crossbeam::TryRecvError::Empty)));
        if consume {
            let delivery = runtime.await_task(id).unwrap();
            assert!(
                matches!(rx.try_recv(), Err(crossbeam::TryRecvError::Empty)),
                "caller still owns returned sender"
            );
            drop(delivery);
        } else {
            runtime.reap_tasks_at(at + Duration::from_secs(3600));
        }
        assert!(matches!(
            rx.try_recv(),
            Err(crossbeam::TryRecvError::Disconnected)
        ));
        assert!(arcs.inner.lock().unwrap().result.is_none());
    }
}

#[test]
fn concurrent_expiration_and_consumption_deliver_once_or_expire_coherently() {
    for _ in 0..64 {
        let runtime = ConcurrencyRuntime::new();
        let (id, arcs, _) = task(&runtime, TaskKind::Spawn);
        let at = Instant::now();
        finish(
            &runtime,
            id,
            &arcs,
            Ok(Ok(Value::String("not-unit".to_string()))),
            at,
        );
        let barrier = std::sync::Barrier::new(3);
        thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                if let Ok(value) = runtime.await_task(id) {
                    assert!(matches!(value, Value::EnumValue { values, variant, .. } if variant == "Ok" && matches!(&values[0], Value::String(s) if s == "not-unit")));
                }
            });
            scope.spawn(|| {
                barrier.wait();
                runtime.reap_tasks_at(at + Duration::from_secs(3600));
            });
            barrier.wait();
        });
        let tasks = runtime.tasks.lock().unwrap();
        assert!(tasks.entries.is_empty());
        assert!(tasks.results.is_empty());
        assert_eq!(tasks.result_bytes, 0);
        assert_eq!(tasks.history[&id].outcome, TaskState::Completed);
        assert!(matches!(
            tasks.history[&id].state,
            TaskState::Consumed | TaskState::Expired
        ));
        assert!(arcs.inner.lock().unwrap().result.is_none());
    }
}

#[test]
fn structured_cleanup_covers_partial_validation_failure_and_unwind() {
    for unwind in [false, true] {
        let runtime = ConcurrencyRuntime::new();
        let (running, running_arcs, token) = task(&runtime, TaskKind::Ephemeral);
        let (done, done_arcs, _) = task(&runtime, TaskKind::Ephemeral);
        finish(
            &runtime,
            done,
            &done_arcs,
            Ok(Ok(Value::String("discard".repeat(1024)))),
            Instant::now(),
        );
        let _ = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
            let _children = StructuredTasks {
                runtime: &runtime,
                handles: vec![Value::TaskHandle(running), Value::TaskHandle(done)],
            };
            if unwind {
                panic!("parent panic");
            }
            validate_and_capture("spawn", &Value::Int(7))?;
            Ok(())
        }));
        assert!(runtime.tasks.lock().unwrap().entries.is_empty());
        assert!(runtime.tasks.lock().unwrap().history.is_empty());
        assert!(done_arcs.inner.lock().unwrap().result.is_none());
        assert!(token.is_cancelled());
        assert_eq!(runtime.active_tasks.load(AtomicOrdering::Acquire), 1);
        finish(
            &runtime,
            running,
            &running_arcs,
            Ok(Ok(Value::String("late".repeat(1024)))),
            Instant::now(),
        );
        assert_eq!(runtime.active_tasks.load(AtomicOrdering::Acquire), 0);
        assert!(running_arcs.inner.lock().unwrap().result.is_none());
        assert!(runtime.tasks.lock().unwrap().entries.is_empty());
        assert!(runtime.tasks.lock().unwrap().history.is_empty());
    }
}

#[test]
fn simultaneous_forget_and_finalization_never_reinsert_or_double_count() {
    for _ in 0..64 {
        let runtime = ConcurrencyRuntime::new();
        let (id, arcs, _) = task(&runtime, TaskKind::Ephemeral);
        let barrier = std::sync::Barrier::new(3);
        thread::scope(|scope| {
            scope.spawn(|| {
                barrier.wait();
                finish(&runtime, id, &arcs, Ok(Ok(Value::Int(7))), Instant::now());
            });
            scope.spawn(|| {
                barrier.wait();
                runtime.forget_task(id);
            });
            barrier.wait();
        });
        assert_eq!(runtime.active_tasks.load(AtomicOrdering::Acquire), 0);
        assert!(runtime.tasks.lock().unwrap().entries.is_empty());
        assert!(runtime.tasks.lock().unwrap().history.is_empty());
        assert!(arcs.inner.lock().unwrap().result.is_none());
        finish(&runtime, id, &arcs, Ok(Ok(Value::Int(9))), Instant::now());
        assert_eq!(runtime.active_tasks.load(AtomicOrdering::Acquire), 0);
        assert!(arcs.inner.lock().unwrap().result.is_none());
    }
}
