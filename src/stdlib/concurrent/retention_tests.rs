use super::*;

#[test]
fn failed_schedule_thread_start_rolls_back_schedule_ownership() {
    let runtime = ConcurrencyRuntime::new();
    let (id, cancelled, _) = runtime.register_schedule().unwrap();
    {
        let _start = ScheduleStartGuard::new(&runtime, id);
    }
    assert!(runtime.schedules.lock().unwrap().is_empty());
    assert!(cancelled.is_cancelled());
}

#[test]
fn cancelled_delayed_task_has_no_execution_start_or_duration() {
    let runtime = ConcurrencyRuntime::new();
    let token = Arc::new(CancelToken::new());
    let id = runtime
        .register_task_kind(Arc::clone(&token), TaskKind::After)
        .unwrap();
    let arcs = runtime.get_task_arcs(id).unwrap().unwrap();
    runtime.active_tasks.fetch_add(1, AtomicOrdering::Release);
    token.cancel();
    runtime.finish_task(
        id,
        Ok(Err(IntentError::runtime_error("Task cancelled"))),
        &arcs.inner,
        &arcs.completed_notify,
        Instant::now(),
    );
    runtime.await_task(id).unwrap();
    let tasks = runtime.tasks.lock().unwrap();
    let record = tasks.history[&id];
    assert_eq!(record.kind, TaskKind::After);
    assert_eq!(record.outcome, TaskState::Failed);
    assert!(record.started_at.is_none());
    assert!(record.duration.is_none());
}

#[test]
fn await_cannot_revive_an_inactive_expired_result_before_reaper_tick() {
    let runtime = ConcurrencyRuntime::new();
    let (id, arcs) = completed(
        &runtime,
        SerializedValue::Unit,
        Instant::now() - Duration::from_secs(3601),
    );
    assert!(runtime.await_task(id).is_err());
    assert_eq!(arcs.inner.lock().unwrap().state, TaskState::Expired);
}

#[test]
fn failed_thread_start_rolls_back_registration_and_active_count() {
    let runtime = ConcurrencyRuntime::new();
    let token = Arc::new(CancelToken::new());
    let id = runtime.register_task(Arc::clone(&token)).unwrap();
    runtime.active_tasks.fetch_add(1, AtomicOrdering::Release);
    {
        let _registration = TaskStartGuard::new(&runtime, id);
        // Same guard drop as a fallible OS spawn or unwind before handoff.
    }
    assert!(token.is_cancelled());
    assert!(runtime.get_task_arcs(id).unwrap().is_none());
    assert!(runtime.tasks.lock().unwrap().history.is_empty());
    assert_eq!(runtime.active_tasks.load(AtomicOrdering::Acquire), 0);
}

fn identifying_handler() -> Value {
    let mut env = crate::interpreter::Environment::new();
    env.define(
        "identify".to_string(),
        Value::NativeFunction {
            name: "identify".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_| {
                CURRENT_CANCEL_TOKEN.with(|cell| {
                    let token = cell.borrow();
                    let tasks = RUNTIME.tasks.lock().unwrap();
                    let id = tasks
                        .entries
                        .iter()
                        .find(|(_, entry)| Arc::ptr_eq(&entry.cancelled, token.as_ref().unwrap()))
                        .unwrap()
                        .0;
                    Ok(Value::Int(*id as i64))
                })
            },
        },
    );
    Value::Function {
        name: "child".to_string(),
        params: vec![],
        contract: None,
        type_params: vec![],
        closure: std::rc::Rc::new(std::cell::RefCell::new(env)),
        body: crate::ast::Block {
            statements: vec![crate::ast::Statement::Expression(
                crate::ast::Expression::Call {
                    function: Box::new(crate::ast::Expression::Identifier("identify".to_string())),
                    arguments: vec![],
                },
            )],
        },
    }
}

#[test]
fn parallel_children_leave_neither_execution_entries_nor_history() {
    let Value::Array(results) = concurrent_parallel(&Value::Array(vec![
        identifying_handler(),
        identifying_handler(),
    ]))
    .unwrap() else {
        panic!("ordered results")
    };
    for result in results {
        let Value::EnumValue {
            values, variant, ..
        } = result
        else {
            panic!("Result")
        };
        assert_eq!(variant, "Ok");
        let Value::Int(id) = values[0] else {
            panic!("id")
        };
        assert!(RUNTIME.get_task_arcs(id as u64).unwrap().is_none());
        assert!(
            !RUNTIME
                .tasks
                .lock()
                .unwrap()
                .history
                .contains_key(&(id as u64)),
            "internal children must not leave tombstones"
        );
    }
}

#[test]
fn compact_history_is_bounded_by_bytes_independently_of_count() {
    let mut runtime = ConcurrencyRuntime::new();
    runtime.retention.history_bytes = HISTORY_BASE_BYTES + 2 * HISTORY_RECORD_BYTES;
    let mut ids = Vec::new();
    for _ in 0..3 {
        let (id, _) = completed(&runtime, SerializedValue::Unit, Instant::now());
        runtime.await_task(id).unwrap();
        ids.push(id);
    }
    assert_eq!(runtime.tasks.lock().unwrap().history.len(), 2);
    assert!(runtime.try_await(ids[0]).is_err());
    assert!(runtime.try_await(ids[1]).is_ok());
}

#[test]
fn structural_result_bytes_enforce_budget_at_completion() {
    let mut runtime = ConcurrencyRuntime::new();
    runtime.retention.result_bytes = std::mem::size_of::<SerializedValue>() * 5;
    let (id, arcs) = completed(
        &runtime,
        SerializedValue::Array(vec![SerializedValue::Unit; 8]),
        Instant::now(),
    );
    assert!(runtime.get_task_arcs(id).unwrap().is_none());
    assert!(arcs.inner.lock().unwrap().result.is_none());
    assert_eq!(runtime.tasks.lock().unwrap().result_bytes, 0);
}

#[test]
fn completion_enforces_public_result_cardinality_immediately() {
    let mut runtime = ConcurrencyRuntime::new();
    runtime.retention.result_count = 2;
    let mut handles = Vec::new();
    for _ in 0..3 {
        let id = runtime.register_task(Arc::new(CancelToken::new())).unwrap();
        let arcs = runtime.get_task_arcs(id).unwrap().unwrap();
        runtime.active_tasks.fetch_add(1, AtomicOrdering::Release);
        runtime.finish_task(
            id,
            Ok(Ok(Value::Unit)),
            &arcs.inner,
            &arcs.completed_notify,
            Instant::now(),
        );
        handles.push(id);
    }
    let tasks = runtime.tasks.lock().unwrap();
    assert_eq!(tasks.entries.len(), 2);
    assert_eq!(tasks.history[&handles[0]].state, TaskState::Expired);
    assert_eq!(runtime.active_tasks.load(AtomicOrdering::Acquire), 0);
}

fn completed(runtime: &ConcurrencyRuntime, value: SerializedValue, at: Instant) -> (u64, TaskArcs) {
    let id = runtime.register_task(Arc::new(CancelToken::new())).unwrap();
    let arcs = runtime.get_task_arcs(id).unwrap().unwrap();
    runtime.active_tasks.fetch_add(1, AtomicOrdering::Release);
    runtime.finish_task(
        id,
        Ok(Ok(value.to_value())),
        &arcs.inner,
        &arcs.completed_notify,
        at,
    );
    (id, arcs)
}

#[test]
fn compact_history_has_a_finite_default_cardinality() {
    let runtime = ConcurrencyRuntime::new();
    for _ in 0..100_001 {
        let (id, _) = completed(&runtime, SerializedValue::Unit, Instant::now());
        runtime.await_task(id).unwrap();
    }
    assert_eq!(runtime.tasks.lock().unwrap().history.len(), 100_000);
}

#[test]
fn expiration_drops_payload_and_heavy_entry_without_future_spawns() {
    let runtime = ConcurrencyRuntime::new();
    let (id, arcs) = completed(
        &runtime,
        SerializedValue::String("retained".repeat(1024)),
        Instant::now() - Duration::from_secs(3601),
    );
    runtime.reap_expired_tasks();
    assert!(
        arcs.inner.lock().unwrap().result.is_none(),
        "expiration must discard graphs, not just change the status"
    );
    assert!(runtime.get_task_arcs(id).unwrap().is_none());
    let Value::Map(map) = runtime.try_await(id).unwrap() else {
        panic!("status")
    };
    assert!(matches!(&map["status"], Value::String(s) if s == "expired"));
}

#[test]
fn consumption_releases_payload_and_heavy_entry_but_keeps_recent_status() {
    let runtime = ConcurrencyRuntime::new();
    let (id, arcs) = completed(
        &runtime,
        SerializedValue::String("payload".repeat(1024)),
        Instant::now(),
    );
    assert!(
        matches!(runtime.await_task(id).unwrap(), Value::EnumValue { variant, .. } if variant == "Ok")
    );
    assert!(
        arcs.inner.lock().unwrap().result.is_none(),
        "consumption must take the payload"
    );
    assert!(
        runtime.get_task_arcs(id).unwrap().is_none(),
        "consumption must drop registry synchronization ownership"
    );
    let Value::Map(status) = runtime.try_await(id).unwrap() else {
        panic!("status map")
    };
    assert!(matches!(&status["status"], Value::String(s) if s == "consumed"));
    assert!(matches!(&status["result"], Value::EnumValue { variant, .. } if variant == "None"));
    assert!(runtime.await_task(id).is_err());
}
