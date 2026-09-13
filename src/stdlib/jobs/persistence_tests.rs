use super::*;
use std::time::{Duration, Instant};

fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !predicate() {
        assert!(
            Instant::now() < deadline,
            "worker did not reach expected state"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn fault_case(target: &str, competing_cancel: bool) {
    let interrupted = target == "interrupted";
    let target = if interrupted { "active" } else { target };
    let _guard = tests::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    JOB_RUNTIME.reset();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fault.db");
    let source_file = dir.path().join("worker.tnt");
    let path_literal = serde_json::to_string(path.to_str().unwrap()).unwrap();
    let body = if matches!(target, "dead" | "retrying") {
        "1 / 0"
    } else {
        "1"
    };
    let retries = if target == "retrying" { 2 } else { 0 };
    let source = format!(
        r#"
import {{ open, incr }} from "std/kv"
let db=unwrap(open({path_literal}))
job Fault on default (retry: {retries}, backoff: "constant", backoff_base: 60, expires: 10) {{
 perform() {{ unwrap(incr(db,"runs"))
 {body} }}
 on_failure(error,attempt) {{ unwrap(incr(db,"failures")) }}
}}
"#
    );
    std::fs::write(&source_file, &source).unwrap();
    let ast = crate::parser::Parser::new(crate::lexer::Lexer::new(&source).collect())
        .parse()
        .unwrap();
    crate::interpreter::Interpreter::new().eval(&ast).unwrap();
    JOB_RUNTIME.set_source_file(source_file.to_str().unwrap().into());
    *JOB_RUNTIME.kv_url.lock().unwrap() = path.to_str().unwrap().into();
    let handle = JOB_RUNTIME.get_or_init_kv().unwrap();
    let EnqueueResult::Created(id) = enqueue_internal(
        "Fault",
        Value::Map(HashMap::new()),
        &timestamp_key(),
        None,
        None,
        None,
    )
    .unwrap() else {
        panic!()
    };
    let key = format!("jobs:data:{id}");
    let Value::Map(mut record) = kv::kv_get(&handle, &key).unwrap() else {
        panic!()
    };
    let Some(Value::String(pending)) = record.get("pending_key").cloned() else {
        panic!()
    };
    let unique = "jobs:unique:fault";
    record.insert("dedup_key".into(), Value::String(unique.into()));
    kv::kv_set(
        &handle,
        unique,
        &Value::String("new-owner".into()),
        Some(3600),
    )
    .unwrap();
    if target == "expired" {
        record.insert(
            "created_at".into(),
            Value::String("00000000000000000000".into()),
        );
    }
    history::save(&handle, &key, record).unwrap();
    let observer = rusqlite::Connection::open(&path).unwrap();
    // Real backend rejection: every matching state write fails until unblocked.
    observer.execute_batch(&format!("CREATE TRIGGER fault BEFORE INSERT ON _kv WHEN NEW.key='{key}' AND json_extract(NEW.value,'$.status')='{target}' BEGIN SELECT RAISE(ABORT,'injected write failure'); END")).unwrap();
    let cancel = Arc::new(CancelToken::new());
    let stop = cancel.clone();
    let info = extract_kv_handle_info(&handle).unwrap();
    let worker = std::thread::spawn(move || {
        CURRENT_CANCEL_TOKEN.with(|c| *c.borrow_mut() = Some(stop));
        worker_loop(
            info,
            BandConfig {
                name: "fault".into(),
                min_priority: 0,
                max_priority: 99,
                concurrency: 1,
                poll_interval_ms: 10,
            },
            None,
        );
    });
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for(|| matches!(kv::kv_get(&handle, &pending).unwrap(), Value::Unit));
        let ran = !matches!(target, "active" | "expired");
        if ran {
            wait_for(|| matches!(kv::kv_get(&handle, "runs").unwrap(), Value::Int(1)));
        }
        std::thread::sleep(Duration::from_millis(250));
        let Value::Map(blocked) = kv::kv_get(&handle, &key).unwrap() else {
            panic!()
        };
        let expected = if ran { "active" } else { "claimed" };
        assert!(
            matches!(blocked.get("status"),Some(Value::String(s)) if s==expected),
            "write must still be blocked"
        );
        if competing_cancel {
            cancel_job_by_id(&id, true).unwrap();
        }
        if interrupted {
            cancel.cancel();
            wait_for(|| matches!(kv::kv_get(&handle,&pending).unwrap(),Value::String(s) if s==id));
        }
        observer.execute_batch("DROP TRIGGER fault").unwrap();
        let final_status = if interrupted {
            "pending"
        } else if competing_cancel {
            "cancelled"
        } else if target == "active" {
            "completed"
        } else {
            target
        };
        wait_for(
            || matches!(kv::kv_get(&handle,&key).unwrap(),Value::Map(m) if matches!(m.get("status"),Some(Value::String(s)) if s==final_status)),
        );
    }));
    cancel.cancel();
    worker.join().unwrap();
    outcome.unwrap();
    // Stop the polling worker before inspecting its future queue entry.
    if target == "retrying" && !competing_cancel {
        let Value::Map(m) = kv::kv_get(&handle, &key).unwrap() else {
            panic!()
        };
        let Some(Value::String(pk)) = m.get("pending_key") else {
            panic!()
        };
        assert!(
            matches!(kv::kv_get(&handle,pk).unwrap(),Value::String(s) if s==id),
            "retry state and its queue entry must commit together"
        );
    }

    assert!(
        matches!(kv::kv_get(&handle, unique).unwrap(), Value::String(id) if id == "new-owner"),
        "stale terminal cleanup must not release a new owner's reservation"
    );
    let expected_runs =
        if interrupted || target == "expired" || (target == "active" && competing_cancel) {
            0
        } else {
            1
        };
    let runs = match kv::kv_get(&handle, "runs").unwrap() {
        Value::Int(n) => n,
        Value::Unit => 0,
        _ => panic!(),
    };
    assert_eq!(
        runs, expected_runs,
        "storage retries must not replay perform"
    );
    let failures = match kv::kv_get(&handle, "failures").unwrap() {
        Value::Int(n) => n,
        Value::Unit => 0,
        _ => panic!(),
    };
    assert_eq!(
        failures,
        if matches!(target, "dead" | "retrying") {
            1
        } else {
            0
        },
        "storage retries must not replay on_failure"
    );
    JOB_RUNTIME.reset();
}

#[test]
fn interrupted_active_write_restores_unexecuted_queue_entry() {
    fault_case("interrupted", false);
}

#[test]
fn transient_active_write_recovers() {
    fault_case("active", false)
}
#[test]
fn transient_completed_write_recovers_without_reexecution() {
    fault_case("completed", false)
}
#[test]
fn transient_retry_write_publishes_queue_atomically() {
    fault_case("retrying", false)
}
#[test]
fn transient_dead_write_recovers_without_reexecution() {
    fault_case("dead", false)
}
#[test]
fn transient_expired_write_recovers_without_execution() {
    fault_case("expired", false)
}
#[test]
fn cancel_during_active_write_failure_wins() {
    fault_case("active", true)
}
#[test]
fn cancel_during_completed_write_failure_wins() {
    fault_case("completed", true)
}
