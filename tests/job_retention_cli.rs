//! Real CLI coverage for old SQLite stores and idle-worker retention.
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct OwnedWorker(Child);
impl Drop for OwnedWorker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn fixture_dir() -> tempfile::TempDir {
    // Keep Unix socket paths below macOS's small sockaddr_un path limit.
    #[cfg(unix)]
    let root = std::path::PathBuf::from("/tmp");
    #[cfg(not(unix))]
    let root = std::env::temp_dir();
    tempfile::Builder::new()
        .prefix("njr-")
        .tempdir_in(root)
        .unwrap()
}

fn command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    cmd.current_dir(root);
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("NTNT_") || name == "APP_ENV" {
            cmd.env_remove(name);
        }
    }
    cmd.env("NTNT_ENV", "production");
    cmd
}

fn record(id: &str, status: &str, days_ago: u64) -> Value {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let at = format!(
        "{:020}",
        now.saturating_sub(u128::from(days_ago) * 86_400 * 1_000_000_000)
    );
    json!({"id":id,"type":"RetentionFixture","queue":"protected","payload":{},
        "status":status,"attempts":1,"retry":3,"priority":50,
        "created_at":at,"completed_at":at,"cancelled_at":at,"failed_at":at,
        "dead_at":at,"expired_at":at,"error":"retained-job-error-canary"})
}

fn insert(conn: &Connection, key: &str, value: &Value) {
    conn.execute(
        "INSERT INTO _kv(key,value,type) VALUES (?1,?2,'map')",
        params![key, value.to_string()],
    )
    .unwrap();
}

fn seed(root: &Path) -> (Connection, BTreeSet<String>, BTreeSet<String>) {
    let db = root.join("jobs.db");
    let conn = Connection::open(&db).unwrap();
    conn.busy_timeout(Duration::from_secs(2)).unwrap();
    // Deliberately create only the old schema, without retention tables/indexes.
    conn.execute_batch("CREATE TABLE _kv(key TEXT PRIMARY KEY,value TEXT NOT NULL,type TEXT NOT NULL DEFAULT 'string',expires_at INTEGER)").unwrap();
    let mut prune = BTreeSet::new();
    let mut preserve = BTreeSet::new();
    for (status, days) in [
        ("completed", 31),
        ("cancelled", 31),
        ("dead", 91),
        ("failed", 91),
        ("expired", 91),
    ] {
        let id = format!("old-{status}");
        let key = format!("jobs:data:{id}");
        insert(&conn, &key, &record(&id, status, days));
        prune.insert(key);
    }
    for n in 0..8 {
        let id = format!("old-completed-{n}");
        let key = format!("jobs:data:{id}");
        insert(&conn, &key, &record(&id, "completed", 31));
        prune.insert(key);
    }
    for status in ["pending", "scheduled", "retrying", "active", "unknown"] {
        let id = format!("protected-{status}");
        let key = format!("jobs:data:{id}");
        insert(&conn, &key, &record(&id, status, 120));
        preserve.insert(key);
    }
    for (id, status, days) in [
        ("recent-completion", "completed", 0),
        ("recent-failure", "dead", 31),
    ] {
        let key = format!("jobs:data:{id}");
        let mut value = record(id, status, days);
        // Creation age must not substitute for the terminal transition age.
        value["created_at"] = record(id, status, 400)["created_at"].clone();
        insert(&conn, &key, &value);
        preserve.insert(key);
    }
    insert(
        &conn,
        "application:unrelated",
        &json!({"canary":"preserve"}),
    );
    preserve.insert("application:unrelated".into());
    let url = serde_json::to_string(&format!("sqlite:{}", db.display())).unwrap();
    fs::write(root.join("app.tnt"), format!(r#"import {{ configure_queue }} from "std/jobs"
configure_queue(map {{ "store": {url}, "retention": map {{ "interval_secs": 1, "batch_size": 4 }} }})
job RetentionFixture on idle {{
    perform() {{ print("UNEXPECTED_JOB_EXECUTION") }}
}}
"#)).unwrap();
    (conn, prune, preserve)
}

#[test]
fn execution_errors_remain_logged_when_job_record_persistence_fails() {
    let dir = fixture_dir();
    let (conn, _, _) = seed(dir.path());
    let mut job = record("write-failure", "pending", 0);
    job["queue"] = json!("idle");
    job["retry"] = json!(1);
    job["attempts"] = json!(0);
    job["pending_key"] = json!("jobs:pending:50:0:write-failure");
    insert(&conn, "jobs:data:write-failure", &job);
    conn.execute("INSERT INTO _kv(key,value,type) VALUES('jobs:pending:50:0:write-failure','write-failure','string')", []).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_terminal BEFORE UPDATE ON _kv WHEN OLD.key='jobs:data:write-failure' BEGIN SELECT CASE WHEN json_extract(NEW.value,'$.status') IN ('dead','retrying','completed') THEN RAISE(ABORT,'injected terminal-write failure') END; END;").unwrap();
    let url =
        serde_json::to_string(&format!("sqlite:{}", dir.path().join("jobs.db").display())).unwrap();
    fs::write(
        dir.path().join("app.tnt"),
        format!(
            r#"import {{ configure_queue }} from "std/jobs"
configure_queue(map {{ "store": {url}, "retention": map {{ "enabled": false }} }})
job RetentionFixture on idle {{ perform() {{ print("EXECUTED_FAILURE_FIXTURE"); assert(false) }} }}
"#
        ),
    )
    .unwrap();
    let out = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let err = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let mut worker = OwnedWorker(
        command(dir.path())
            .args([
                "worker",
                "app.tnt",
                "--queues",
                "idle",
                "--poll-interval",
                "20",
            ])
            .stdout(Stdio::from(out.reopen().unwrap()))
            .stderr(Stdio::from(err.reopen().unwrap()))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let log = fs::read_to_string(err.path()).unwrap();
        if let Some(event) = log
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .find(|v| v["event"] == "job.failed" && v["job_id"] == "write-failure")
        {
            assert_eq!(event["state_persisted"], false);
            assert_eq!(event["persistence_error"], "storage_error");
            assert!(event["error"].as_str().is_some_and(|s| !s.is_empty()));
            assert!(
                event.get("will_retry").is_none(),
                "must not claim a retry was persisted"
            );
            // Storage recovers. Persist the already-produced failure without
            // executing the job (or its external side effects) a second time.
            conn.execute_batch("DROP TRIGGER reject_terminal").unwrap();
            loop {
                let raw: String = conn
                    .query_row(
                        "SELECT value FROM _kv WHERE key='jobs:data:write-failure'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap();
                if serde_json::from_str::<Value>(&raw).unwrap()["status"] == "dead" {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "recovered storage left the executed job stranded active"
                );
                thread::sleep(Duration::from_millis(20));
            }
            assert_eq!(
                fs::read_to_string(out.path())
                    .unwrap()
                    .lines()
                    .filter(|s| *s == "EXECUTED_FAILURE_FIXTURE")
                    .count(),
                1
            );
            break;
        }
        assert!(
            worker.0.try_wait().unwrap().is_none() && Instant::now() < deadline,
            "execution error disappeared after failed persistence\nstdout={}\nstderr={log}",
            fs::read_to_string(out.path()).unwrap()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn claimed_job_runs_once_after_activation_write_recovers() {
    let dir = fixture_dir();
    let (conn, _, _) = seed(dir.path());
    let mut job = record("activation", "pending", 0);
    job["queue"] = json!("idle");
    job["attempts"] = json!(0);
    job["pending_key"] = json!("jobs:pending:50:0:activation");
    insert(&conn, "jobs:data:activation", &job);
    conn.execute("INSERT INTO _kv(key,value,type) VALUES('jobs:pending:50:0:activation','activation','string')",[]).unwrap();
    conn.execute_batch("CREATE TRIGGER reject_active BEFORE UPDATE ON _kv WHEN OLD.key='jobs:data:activation' BEGIN SELECT CASE WHEN json_extract(NEW.value,'$.status')='active' THEN RAISE(ABORT,'injected active write failure') END; END;").unwrap();
    let out = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let err = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let mut worker = OwnedWorker(
        command(dir.path())
            .args([
                "worker",
                "app.tnt",
                "--queues",
                "idle",
                "--poll-interval",
                "20",
            ])
            .stdout(Stdio::from(out.reopen().unwrap()))
            .stderr(Stdio::from(err.reopen().unwrap()))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(8);
    while !fs::read_to_string(err.path())
        .unwrap()
        .contains("job.state_persistence_failed")
    {
        assert!(worker.0.try_wait().unwrap().is_none() && Instant::now() < deadline);
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!fs::read_to_string(out.path())
        .unwrap()
        .contains("UNEXPECTED_JOB_EXECUTION"));
    conn.execute_batch("DROP TRIGGER reject_active").unwrap();
    loop {
        let raw: String = conn
            .query_row(
                "SELECT value FROM _kv WHERE key='jobs:data:activation'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        if serde_json::from_str::<Value>(&raw).unwrap()["status"] == "completed" {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "claimed job remained stranded after storage recovered"
        );
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        fs::read_to_string(out.path())
            .unwrap()
            .lines()
            .filter(|s| *s == "UNEXPECTED_JOB_EXECUTION")
            .count(),
        1
    );
}

fn user_keys(conn: &Connection) -> BTreeSet<String> {
    conn.prepare(
        "SELECT key FROM _kv WHERE key LIKE 'jobs:data:%' OR key = 'application:unrelated'",
    )
    .unwrap()
    .query_map([], |row| row.get::<_, String>(0))
    .unwrap()
    .collect::<rusqlite::Result<BTreeSet<_>>>()
    .unwrap()
}

#[test]
fn inspecting_job_errors_does_not_start_destructive_maintenance() {
    let dir = fixture_dir();
    let (conn, prune, preserve) = seed(dir.path());
    let output = command(dir.path())
        .args(["jobs", "inspect", "app.tnt", "old-dead"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("retained-job-error-canary"));
    assert_eq!(user_keys(&conn), prune.union(&preserve).cloned().collect());
}

#[test]
fn inspection_does_not_reconfigure_an_existing_store_policy() {
    let dir = fixture_dir();
    let (conn, _, _) = seed(dir.path());
    let url =
        serde_json::to_string(&format!("sqlite:{}", dir.path().join("jobs.db").display())).unwrap();
    fs::write(dir.path().join("configure.tnt"), format!(r#"import {{ configure_queue }} from "std/jobs"
configure_queue(map {{ "store": {url}, "retention": map {{ "enabled": false, "failed_days": 120 }} }})
"#)).unwrap();
    let configured = command(dir.path())
        .args(["run", "configure.tnt"])
        .output()
        .unwrap();
    assert!(
        configured.status.success(),
        "{}",
        String::from_utf8_lossy(&configured.stderr)
    );
    let before: String = conn
        .query_row(
            "SELECT policy FROM _jobs_retention_meta_v1 WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    // app.tnt has enabled/default retention. Viewing it must not overwrite the
    // policy used by other running worker processes sharing the same store.
    let inspected = command(dir.path())
        .args(["jobs", "inspect", "app.tnt", "old-dead"])
        .output()
        .unwrap();
    assert!(inspected.status.success());
    let after: String = conn
        .query_row(
            "SELECT policy FROM _jobs_retention_meta_v1 WHERE id=1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&before).unwrap(),
        serde_json::from_str::<Value>(&after).unwrap()
    );
}

#[test]
fn idle_workers_incrementally_prune_old_terminal_records_only() {
    let dir = fixture_dir();
    let (conn, prune, preserve) = seed(dir.path());
    let out = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let err = tempfile::NamedTempFile::new_in(dir.path()).unwrap();
    let mut worker = OwnedWorker(
        command(dir.path())
            .args([
                "worker",
                "app.tnt",
                "--queues",
                "idle",
                "--concurrency",
                "1",
                "--poll-interval",
                "20",
            ])
            .stdout(Stdio::from(out.reopen().unwrap()))
            .stderr(Stdio::from(err.reopen().unwrap()))
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let keys = user_keys(&conn);
        assert!(
            preserve.is_subset(&keys),
            "protected/unrelated records were deleted: {keys:?}"
        );
        if prune.is_disjoint(&keys) {
            break;
        }
        let status = worker.0.try_wait().unwrap();
        assert!(status.is_none() && Instant::now()<deadline,
            "idle worker did not prune legacy history; status={status:?}, remaining={keys:?}\nstdout={}\nstderr={}",
            fs::read_to_string(out.path()).unwrap(),fs::read_to_string(err.path()).unwrap());
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(user_keys(&conn), preserve);
    assert!(!fs::read_to_string(out.path())
        .unwrap()
        .contains("UNEXPECTED_JOB_EXECUTION"));
}
