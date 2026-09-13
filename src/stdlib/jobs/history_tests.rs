use super::*;
use std::time::{Duration, Instant};

fn with_store(url: &str, test: impl FnOnce(&Value)) {
    let _guard = tests::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    JOB_RUNTIME.reset();
    *JOB_RUNTIME.kv_url.lock().unwrap() = url.into();
    let handle = JOB_RUNTIME.get_or_init_kv().unwrap();
    test(&handle);
    JOB_RUNTIME.reset();
}

fn data(id: &str, status: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("id".into(), Value::String(id.into())),
        ("type".into(), Value::String("HistoryTest".into())),
        ("queue".into(), Value::String("default".into())),
        ("status".into(), Value::String(status.into())),
        (
            "last_error".into(),
            Value::String("recorded failure".into()),
        ),
    ])
}

fn assert_ttl(handle: &Value, key: &str, expected: Option<i64>) {
    let actual = kv::kv_ttl(handle, key).unwrap();
    match expected {
        Some(seconds) => assert!(
            matches!(actual, Some(n) if (seconds - 2..=seconds).contains(&n)),
            "{key}: {actual:?}, expected {expected:?}"
        ),
        None => assert_eq!(actual, None, "{key}"),
    }
}

fn expiry_contract(handle: &Value) {
    for (state, ttl) in [
        ("completed", Some(30 * 86400)),
        ("cancelled", Some(30 * 86400)),
        ("dead", Some(90 * 86400)),
        ("failed", Some(90 * 86400)),
        ("expired", Some(90 * 86400)),
        ("pending", None),
        ("scheduled", None),
        ("retrying", None),
        ("active", None),
        ("unknown", None),
    ] {
        let key = format!("jobs:data:{state}");
        history::save(handle, &key, data(state, state)).unwrap();
        assert_ttl(handle, &key, ttl);
        let Value::Map(saved) = kv::kv_get(handle, &key).unwrap() else {
            panic!("missing history")
        };
        assert!(
            matches!(saved.get("last_error"), Some(Value::String(e)) if e == "recorded failure")
        );
    }
    // Retrying a terminal job strips its old TTL, using the public retry path.
    retry_job_by_id("dead").unwrap();
    assert_ttl(handle, "jobs:data:dead", None);
    cancel_job_by_id("pending", false).unwrap();
    assert_ttl(handle, "jobs:data:pending", Some(30 * 86400));
    cancel_job_by_id("active", true).unwrap();
    assert_ttl(handle, "jobs:data:active", Some(30 * 86400));

    // Short test-only TTL: actual expiry, not a mocked backend response.
    JOB_RUNTIME
        .history_retention
        .write()
        .unwrap()
        .completed_secs = Some(1);
    // A terminal job renewed into live state must survive its former deadline.
    JOB_RUNTIME.history_retention.write().unwrap().failed_secs = Some(1);
    history::save(handle, "jobs:data:renewed", data("renewed", "dead")).unwrap();
    retry_job_by_id("renewed").unwrap();
    assert_ttl(handle, "jobs:data:renewed", None);
    history::save(handle, "jobs:data:short", data("short", "completed")).unwrap();
    kv::kv_set(
        handle,
        "jobs:unique:short",
        &Value::String("short".into()),
        Some(60),
    )
    .unwrap();
    kv::kv_set(
        handle,
        "jobs:data:legacy",
        &Value::Map(data("legacy", "completed")),
        None,
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !matches!(kv::kv_get(handle, "jobs:data:short").unwrap(), Value::Unit) {
        assert!(Instant::now() < deadline, "history did not expire");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!kv::kv_list(handle, Some("jobs:data:"))
        .unwrap()
        .contains(&"jobs:data:short".into()));
    assert!(
        matches!(kv::kv_get(handle, "jobs:unique:short").unwrap(), Value::String(s) if s == "short")
    );
    for key in [
        "jobs:data:dead",
        "jobs:data:renewed",
        "jobs:data:scheduled",
        "jobs:data:retrying",
        "jobs:data:legacy",
    ] {
        assert!(
            matches!(kv::kv_get(handle, key).unwrap(), Value::Map(_)),
            "{key} must survive"
        );
    }
    // Disabled retention affects future writes, not already assigned TTLs.
    let disabled = Value::Map(HashMap::from([("enabled".into(), Value::Bool(false))]));
    *JOB_RUNTIME.history_retention.write().unwrap() =
        HistoryRetention::parse(Some(&disabled)).unwrap();
    history::save(handle, "jobs:data:forever", data("forever", "completed")).unwrap();
    assert_ttl(handle, "jobs:data:forever", None);
    assert_ttl(handle, "jobs:data:completed", Some(30 * 86400));
}

#[test]
fn history_ttl_sqlite_contract() {
    with_store("sqlite::memory:", expiry_contract);
}

#[test]
#[ignore = "requires a disposable Redis database via NTNT_RETENTION_TEST_REDIS"]
fn history_ttl_redis_contract() {
    let url = std::env::var("NTNT_RETENTION_TEST_REDIS").expect("set disposable Redis test URL");
    with_store(&url, |handle| {
        let Value::NativeFunction { func: flush, .. } = kv::create_kv_module()["flush"] else {
            panic!()
        };
        flush(std::slice::from_ref(handle)).unwrap();
        expiry_contract(handle);
        flush(std::slice::from_ref(handle)).unwrap();
    });
}

#[test]
fn history_configuration_is_validated_and_reuses_store() {
    with_store("sqlite::memory:", |handle| {
        let module = init();
        let Value::NativeFunction {
            func: configure, ..
        } = module["configure_queue"]
        else {
            panic!()
        };
        let mut opts = HashMap::from([
            ("store".into(), Value::String("sqlite::memory:".into())),
            (
                "retention".into(),
                Value::Map(HashMap::from([
                    ("completed_days".into(), Value::Int(7)),
                    ("failed_days".into(), Value::Int(14)),
                ])),
            ),
        ]);
        configure(&[Value::Map(opts.clone())]).unwrap();
        assert_eq!(
            extract_kv_handle_info(handle).unwrap().store_id,
            JOB_RUNTIME
                .kv_handle_info
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .store_id
        );
        for (state, days) in [("completed", 7), ("dead", 14)] {
            history::save(handle, state, data(state, state)).unwrap();
            assert_ttl(handle, state, Some(days * 86400));
        }
        for invalid in [
            Value::Unit,
            Value::Map(HashMap::from([("max_records".into(), Value::Int(1))])),
            Value::Map(HashMap::from([("completed_days".into(), Value::Int(0))])),
            Value::Map(HashMap::from([("enabled".into(), Value::Int(1))])),
        ] {
            opts.insert("retention".into(), invalid);
            assert!(configure(&[Value::Map(opts.clone())]).is_err());
            assert_eq!(
                JOB_RUNTIME.history_retention.read().unwrap().completed_secs,
                Some(7 * 86400)
            );
        }
    });
}

#[test]
fn history_expiry_keeps_existing_dedup_and_callback_guards() {
    with_store("sqlite::memory:", |handle| {
        let def = JobDefinition {
            name: "HistoryTest".into(),
            queue: "default".into(),
            options: HashMap::from([("unique".into(), JobOptionValue::Int(3600))]),
            perform_params: vec![],
            perform_contract: None,
            perform_body: Block { statements: vec![] },
            on_failure: None,
        };
        JOB_RUNTIME.register_job(def).unwrap();
        let enqueue = || {
            enqueue_internal(
                "HistoryTest",
                Value::Map(HashMap::new()),
                &timestamp_key(),
                None,
                None,
                None,
            )
            .unwrap()
        };
        let first = match enqueue() {
            EnqueueResult::Created(id) => id,
            _ => panic!(),
        };
        history::save(
            handle,
            &format!("jobs:data:{first}"),
            data(&first, "completed"),
        )
        .unwrap();
        // Redis/SQLite TTL expiry removes only the history key, not its uniqueness key.
        kv::kv_expire(handle, &format!("jobs:data:{first}"), -1).unwrap();
        assert!(matches!(enqueue(), EnqueueResult::Deduplicated(id) if id == first));

        let batch = "ttl-batch";
        let snapshot = build_batch_meta(batch, "test", "0", "sealed", 1, 0);
        fire_batch_callback(batch, "on_complete", &snapshot).unwrap();
        let cb_key = format!("jobs:data:cb-{batch}-on_complete");
        let Value::Map(cb) = kv::kv_get(handle, &cb_key).unwrap() else {
            panic!()
        };
        let Some(Value::String(pending)) = cb.get("pending_key") else {
            panic!()
        };
        kv::kv_del(handle, pending).unwrap(); // callback has been claimed
        kv::kv_set(
            handle,
            &format!("jobs:batch:{batch}:fired:on_complete"),
            &Value::Bool(true),
            Some(3600),
        )
        .unwrap();
        kv::kv_expire(handle, &cb_key, -1).unwrap();
        fire_batch_callback(batch, "on_complete", &snapshot).unwrap();
        assert!(matches!(kv::kv_get(handle, &cb_key).unwrap(), Value::Unit));
        assert!(matches!(kv::kv_get(handle, pending).unwrap(), Value::Unit));
    });
}

#[test]
fn history_ttl_worker_terminal_paths() {
    with_store("sqlite::memory:", |handle| {
        let source = "job Good on default { perform() {} }\njob Bad on default (retry: 0) { perform() { 1 / 0 } }";
        let ast = crate::parser::Parser::new(crate::lexer::Lexer::new(source).collect())
            .parse()
            .unwrap();
        crate::interpreter::Interpreter::new().eval(&ast).unwrap();
        let mut expected = Vec::new();
        for (kind, state, days) in [("Good", "completed", 30), ("Bad", "dead", 90)] {
            let EnqueueResult::Created(id) = enqueue_internal(
                kind,
                Value::Map(HashMap::new()),
                &timestamp_key(),
                None,
                None,
                None,
            )
            .unwrap() else {
                panic!()
            };
            expected.push((id, state, days));
        }
        // An unregistered type takes the separate dead-state path.
        let EnqueueResult::Created(id) = enqueue_internal(
            "Good",
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
        let Value::Map(mut record) = kv::kv_get(handle, &key).unwrap() else {
            panic!()
        };
        record.insert("type".into(), Value::String("Missing".into()));
        history::save(handle, &key, record).unwrap();
        expected.push((id, "dead", 90));
        let info = extract_kv_handle_info(handle).unwrap();
        let cancel = Arc::new(CancelToken::new());
        let stop = cancel.clone();
        let worker = std::thread::spawn(move || {
            CURRENT_CANCEL_TOKEN.with(|cell| *cell.borrow_mut() = Some(stop));
            worker_loop(
                info,
                BandConfig {
                    name: "ttl-test".into(),
                    min_priority: 0,
                    max_priority: 99,
                    concurrency: 1,
                    poll_interval_ms: 10,
                },
                None,
            );
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let ready = expected.iter().all(|(id, state, _)| matches!(kv::kv_get(handle, &format!("jobs:data:{id}")), Ok(Value::Map(m)) if matches!(m.get("status"), Some(Value::String(s)) if s == state)));
            if ready || Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        cancel.cancel();
        worker.join().unwrap();
        for (id, state, days) in expected {
            let key = format!("jobs:data:{id}");
            let Value::Map(record) = kv::kv_get(handle, &key).unwrap() else {
                panic!()
            };
            assert!(
                matches!(record.get("status"), Some(Value::String(s)) if s == state),
                "{record:?}"
            );
            assert_ttl(handle, &key, Some(days * 86400));
        }
    });
}
