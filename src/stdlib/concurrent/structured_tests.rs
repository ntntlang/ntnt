use super::*;

fn native_handler(operation: fn(&[Value]) -> Result<Value>) -> Value {
    let mut env = crate::interpreter::Environment::new();
    env.define(
        "operation".to_string(),
        Value::NativeFunction {
            name: "operation".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: operation,
        },
    );
    Value::Function {
        name: "branch".to_string(),
        params: vec![],
        contract: None,
        type_params: vec![],
        closure: std::rc::Rc::new(std::cell::RefCell::new(env)),
        body: crate::ast::Block {
            statements: vec![crate::ast::Statement::Expression(
                crate::ast::Expression::Call {
                    function: Box::new(crate::ast::Expression::Identifier("operation".to_string())),
                    arguments: vec![],
                },
            )],
        },
    }
}

struct RaceProbe {
    started: (
        crossbeam::Sender<(u64, TaskArcs, Arc<CancelToken>)>,
        crossbeam::Receiver<(u64, TaskArcs, Arc<CancelToken>)>,
    ),
    both_started: std::sync::Barrier,
    release: (crossbeam::Sender<()>, crossbeam::Receiver<()>),
}

static RACE_PROBE: LazyLock<RaceProbe> = LazyLock::new(|| RaceProbe {
    started: crossbeam::unbounded(),
    both_started: std::sync::Barrier::new(2),
    release: crossbeam::unbounded(),
});

fn uncooperative_loser(_: &[Value]) -> Result<Value> {
    let token = CURRENT_CANCEL_TOKEN.with(|cell| cell.borrow().as_ref().unwrap().clone());
    let id = {
        let tasks = RUNTIME.tasks.lock().unwrap();
        *tasks
            .entries
            .iter()
            .find(|(_, entry)| Arc::ptr_eq(&entry.cancelled, &token))
            .unwrap()
            .0
    };
    let arcs = RUNTIME.get_task_arcs(id).unwrap().unwrap();
    RACE_PROBE.started.0.send((id, arcs, token)).unwrap();
    RACE_PROBE.both_started.wait();
    // Intentionally no cancellation yield point, and no timeout: the enclosing
    // race must return while this is still blocked, not join/drain it.
    RACE_PROBE.release.1.recv().unwrap();
    Ok(Value::String("late-result".repeat(4096)))
}

fn winner(_: &[Value]) -> Result<Value> {
    RACE_PROBE.both_started.wait();
    Ok(Value::Int(7))
}

#[test]
fn race_returns_before_uncooperative_loser_exits_and_discards_late_result() {
    struct Release;
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = RACE_PROBE.release.0.send(());
        }
    }
    let release = Release;
    let (done_tx, done_rx) = crossbeam::unbounded();
    let runner = thread::spawn(move || {
        let result = concurrent_race(&Value::Array(vec![
            native_handler(uncooperative_loser),
            native_handler(winner),
        ]))
        .unwrap();
        assert!(
            matches!(result, Value::EnumValue { variant, values, .. } if variant == "Ok" && matches!(values[0], Value::Int(7)))
        );
        done_tx.send(()).unwrap();
    });
    let (id, arcs, token) = RACE_PROBE
        .started
        .1
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("race blocked on loser");
    assert!(RUNTIME.get_task_arcs(id).unwrap().is_none());
    assert!(!RUNTIME.tasks.lock().unwrap().history.contains_key(&id));
    assert!(token.is_cancelled());
    assert_eq!(arcs.inner.lock().unwrap().state, TaskState::Running);
    assert!(RUNTIME.active_tasks.load(AtomicOrdering::Acquire) >= 1);
    drop(release);
    let (done, timeout) = arcs
        .completed_notify
        .1
        .wait_timeout_while(
            arcs.completed_notify.0.lock().unwrap(),
            Duration::from_secs(5),
            |done| !*done,
        )
        .unwrap();
    assert!(!timeout.timed_out());
    assert!(*done);
    drop(done);
    runner.join().unwrap();
    assert_eq!(arcs.inner.lock().unwrap().state, TaskState::Expired);
    assert!(arcs.inner.lock().unwrap().result.is_none());
    assert!(RUNTIME.get_task_arcs(id).unwrap().is_none());
    assert!(!RUNTIME.tasks.lock().unwrap().history.contains_key(&id));
}

static FAILED_IDS: LazyLock<Mutex<Vec<u64>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn record_failure_id() {
    let token = CURRENT_CANCEL_TOKEN.with(|cell| cell.borrow().as_ref().unwrap().clone());
    let tasks = RUNTIME.tasks.lock().unwrap();
    if let Some((id, _)) = tasks
        .entries
        .iter()
        .find(|(_, entry)| Arc::ptr_eq(&entry.cancelled, &token))
    {
        FAILED_IDS.lock().unwrap().push(*id);
    }
}
fn failure(_: &[Value]) -> Result<Value> {
    record_failure_id();
    Err(IntentError::runtime_error("failed branch"))
}
fn panic_branch(_: &[Value]) -> Result<Value> {
    record_failure_id();
    panic!("isolated branch panic")
}
fn returned_error(_: &[Value]) -> Result<Value> {
    record_failure_id();
    Ok(Value::err(Value::String("returned error".to_string())))
}

#[test]
fn structured_error_panic_and_partial_validation_exits_leave_no_recorded_children() {
    let race = concurrent_race(&Value::Array(vec![
        native_handler(failure),
        native_handler(panic_branch),
        native_handler(returned_error),
    ]))
    .unwrap();
    assert!(is_task_failure(&race));
    for operation in [
        failure as fn(&[Value]) -> Result<Value>,
        panic_branch,
        returned_error,
    ] {
        let result = concurrent_parallel(&Value::Array(vec![native_handler(operation)])).unwrap();
        assert!(is_task_failure(&result));
    }
    for id in FAILED_IDS.lock().unwrap().drain(..) {
        assert!(RUNTIME.get_task_arcs(id).unwrap().is_none());
        assert!(!RUNTIME.tasks.lock().unwrap().history.contains_key(&id));
    }
    // The local-runtime guard tests assert exact object/counter cleanup for the
    // partial-start path; this checks both real dispatchers propagate validation.
    assert!(concurrent_parallel(&Value::Array(vec![
        native_handler(|_| Ok(Value::Unit)),
        Value::Int(7)
    ]))
    .is_err());
    assert!(concurrent_race(&Value::Array(vec![
        native_handler(|_| Ok(Value::Unit)),
        Value::Int(7)
    ]))
    .is_err());
}
