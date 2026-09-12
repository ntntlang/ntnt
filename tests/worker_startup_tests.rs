//! Task limits and worker globals are isolated in child test processes.
use ntnt::interpreter::Value;
use ntnt::stdlib::jobs::{self, JOB_RUNTIME};

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
    )
}
fn call(name: &str, args: &[Value]) -> ntnt::error::Result<Value> {
    match jobs::init().remove(name).unwrap() {
        Value::NativeFunction { func, .. } => func(args),
        _ => unreachable!(),
    }
}
fn band(name: &str, min: i64, max: i64, count: i64) -> Value {
    map(vec![
        ("name", Value::String(name.into())),
        (
            "range",
            Value::Array(vec![Value::Int(min), Value::Int(max)]),
        ),
        ("concurrency", Value::Int(count)),
        ("poll", Value::Int(100)),
    ])
}
fn options(bands: Vec<Value>, queue: &str, socket: &str) -> Value {
    let mut opts = match map(vec![
        ("bands", Value::Array(bands)),
        ("queues", Value::Array(vec![Value::String(queue.into())])),
    ]) {
        Value::Map(m) => m,
        _ => unreachable!(),
    };
    // Windows has no listener, but must retain the same worker transaction semantics.
    if cfg!(unix) {
        opts.insert("control_socket".into(), Value::String(socket.into()));
    }
    Value::Map(opts)
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn isolated(name: &str, method: &str, existing: bool) {
    if std::env::var("NTNT_STARTUP_CHILD").as_deref() == Ok(name) {
        scenario(method, existing);
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let output = tempfile::tempfile().unwrap();
    let mut child = Process(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", name, "--nocapture"])
            .current_dir(dir.path())
            .env("NTNT_STARTUP_CHILD", name)
            .env("NTNT_MAX_TASKS", "8")
            .env("XDG_RUNTIME_DIR", dir.path())
            .env_remove("NTNT_CONTROL_SOCKET")
            .env_remove("NTNT_WORKER_GROUP")
            .stdout(Stdio::from(output.try_clone().unwrap()))
            .stderr(Stdio::from(output.try_clone().unwrap()))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline, "worker startup child timed out");
        std::thread::sleep(Duration::from_millis(20));
    };
    use std::io::{Read, Seek};
    let mut output = output;
    output.rewind().unwrap();
    let mut text = String::new();
    output.read_to_string(&mut text).unwrap();
    assert!(status.success(), "{name}: {text}");
}
fn scenario(method: &str, existing: bool) {
    jobs::use_host_shutdown_handler();
    call(
        "configure_queue",
        &[map(vec![(
            "store",
            Value::String("sqlite::memory:".into()),
        )])],
    )
    .unwrap();
    if existing {
        call(
            "work_async",
            &[options(
                vec![band("old", 0, 99, 1)],
                "old-queue",
                "old.sock",
            )],
        )
        .unwrap();
    }
    let before = format!("{:?}", JOB_RUNTIME.active_bands.lock().unwrap());
    let ids = JOB_RUNTIME.band_worker_task_ids.lock().unwrap().clone();
    let cancels = JOB_RUNTIME.band_cancel_arcs.lock().unwrap().clone();
    let queues = JOB_RUNTIME.active_queues.lock().unwrap().clone();
    // Race status and no-op scaling against publication. Each snapshot must
    // describe the original pool, never staged bands paired with old task IDs.
    let observer = std::thread::spawn(move || {
        for _ in 0..40 {
            let status = call("worker_status", &[]).unwrap();
            let Value::Map(status) = status else {
                panic!("expected status map")
            };
            let Value::Array(bands) = &status["bands"] else {
                panic!("expected bands")
            };
            assert_eq!(bands.len(), usize::from(existing));
            if existing {
                let Value::Map(band) = &bands[0] else {
                    panic!("expected band")
                };
                assert!(matches!(&band["name"], Value::String(name) if name == "old"));
                assert!(matches!(&band["workers"], Value::Int(1)));
                assert!(matches!(&band["concurrency"], Value::Int(1)));
                call(
                    "scale_workers",
                    &[Value::String("old".into()), Value::Int(1)],
                )
                .unwrap();
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    });
    // The first band completely spawns, then a later spawn in band two hits
    // the real process task limit. This covers both rollback collections.
    let err = call(
        method,
        &[options(
            vec![band("staged-a", 0, 49, 6), band("staged-b", 50, 99, 4)],
            "rejected-queue",
            "rejected.sock",
        )],
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Maximum concurrent task limit reached (8)"),
        "{err}"
    );
    assert_eq!(
        format!("{:?}", JOB_RUNTIME.active_bands.lock().unwrap()),
        before,
        "failed startup changed active bands"
    );
    assert_eq!(
        *JOB_RUNTIME.active_queues.lock().unwrap(),
        queues,
        "failed startup changed queue filter"
    );
    assert_eq!(*JOB_RUNTIME.band_worker_task_ids.lock().unwrap(), ids);
    let now = JOB_RUNTIME.band_cancel_arcs.lock().unwrap();
    assert_eq!(
        now.keys().collect::<std::collections::HashSet<_>>(),
        cancels.keys().collect()
    );
    for (band, arcs) in &cancels {
        assert_eq!(now[band].len(), arcs.len());
        for (a, b) in now[band].iter().zip(arcs) {
            assert!(std::sync::Arc::ptr_eq(a, b));
            assert!(!a.is_cancelled(), "rejected startup cancelled the old pool");
        }
    }
    drop(now);
    observer.join().unwrap();
    // A rejected batch must never enter worker_loop (which initializes stats
    // before touching the queue), not merely notice cancellation on a later poll.
    std::thread::sleep(Duration::from_millis(150));
    let stats = JOB_RUNTIME.band_stats.read().unwrap();
    assert!(
        !stats.contains_key("staged-a") && !stats.contains_key("staged-b"),
        "rejected workers entered the consuming loop"
    );
    drop(stats);
    #[cfg(unix)]
    {
        assert!(!std::path::Path::new("rejected.sock").exists());
        if existing {
            use std::io::{BufRead, BufReader, Write};
            let mut stream = std::os::unix::net::UnixStream::connect("old.sock").unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            stream.write_all(b"{\"cmd\":\"status\"}\n").unwrap();
            let mut response = String::new();
            BufReader::new(stream).read_line(&mut response).unwrap();
            let status: serde_json::Value = serde_json::from_str(&response).unwrap();
            assert_eq!(status["bands"][0]["name"], "old", "{response}");
            assert_eq!(status["bands"][0]["workers"], 1, "{response}");
        }
    }
    if existing {
        // Scaling still resolves the old band and reuses its original filter.
        call(
            "scale_workers",
            &[Value::String("old".into()), Value::Int(2)],
        )
        .unwrap();
        assert_eq!(
            JOB_RUNTIME.band_worker_task_ids.lock().unwrap()["old"].len(),
            2
        );
        assert_eq!(*JOB_RUNTIME.active_queues.lock().unwrap(), queues);
        // Exercise the preserved pool, not just its descriptive metadata.
        JOB_RUNTIME
            .register_job(jobs::JobDefinition {
                name: "OldPoolProbe".into(),
                queue: "old-queue".into(),
                options: Default::default(),
                perform_params: vec![],
                perform_contract: None,
                perform_body: ntnt::ast::Block { statements: vec![] },
                on_failure: None,
            })
            .unwrap();
        call(
            "enqueue",
            &[Value::String("OldPoolProbe".into()), map(vec![])],
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let counts = jobs::job_status_counts().unwrap();
            if counts.completed == 1 {
                break;
            }
            assert_eq!(counts.dead, 0, "old pool failed its probe job");
            assert!(
                Instant::now() < deadline,
                "old pool no longer processes its queue"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    } else {
        // A rejected first startup must not prevent a subsequent valid startup.
        call(
            "work_async",
            &[options(
                vec![band("recovered", 0, 99, 1)],
                "recovered",
                "recovered.sock",
            )],
        )
        .unwrap();
    }
    for arcs in JOB_RUNTIME.band_cancel_arcs.lock().unwrap().values() {
        for arc in arcs {
            arc.cancel();
        }
    }
    ntnt::control_socket::stop_control_socket();
}
#[test]
fn failed_async_start_preserves_pool() {
    isolated("failed_async_start_preserves_pool", "work_async", true);
}
#[test]
fn failed_blocking_start_preserves_pool() {
    isolated("failed_blocking_start_preserves_pool", "work_jobs", true);
}
#[test]
fn failed_async_first_start_is_unpublished() {
    isolated(
        "failed_async_first_start_is_unpublished",
        "work_async",
        false,
    );
}
#[test]
fn failed_blocking_first_start_is_unpublished() {
    isolated(
        "failed_blocking_first_start_is_unpublished",
        "work_jobs",
        false,
    );
}
