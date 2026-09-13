//! Real worker-process loss and restart against persistent SQLite.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn command(file: &Path, mode: &str) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    c.current_dir(file.parent().unwrap())
        .env_remove("NTNT_STRICT")
        .env("CLAIM_FIXTURE_MODE", mode)
        .env("CLAIM_FIXTURE_GROUP", uuid::Uuid::new_v4().to_string())
        .arg("run")
        .arg(file);
    c
}
fn wait(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(25);
    while !condition() {
        assert!(Instant::now() < until, "recovery fixture deadline exceeded");
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn record(conn: &rusqlite::Connection, id: &str) -> serde_json::Value {
    let raw: String = conn
        .query_row(
            "SELECT value FROM _kv WHERE key=?",
            [format!("jobs:data:{id}")],
            |r| r.get(0),
        )
        .unwrap();
    serde_json::from_str(&raw).unwrap()
}
fn fixture(dir: &Path, pause_after_effect: bool) -> (PathBuf, PathBuf, PathBuf) {
    fixture_store(dir, pause_after_effect, None)
}
fn fixture_store(
    dir: &Path,
    pause_after_effect: bool,
    store: Option<&str>,
) -> (PathBuf, PathBuf, PathBuf) {
    let file = dir.join("app.tnt");
    let db = dir.join("jobs.db");
    let effect = dir.join("effect.txt");
    let id = dir.join("id.txt");
    let quote = |p: &Path| serde_json::to_string(p.to_str().unwrap()).unwrap();
    let scope = quote(dir);
    let pause = if pause_after_effect {
        "sleep_ms(60000)"
    } else {
        "1"
    };
    let source = format!(
        r#"
import {{ configure_queue, enqueue, work_jobs }} from "std/jobs"
import {{ write_file, append_file }} from "std/fs"
import {{ get_env }} from "std/env"
import {{ sleep_ms }} from "std/concurrent"
unwrap(configure_queue(map {{ "store": {}, "lease_seconds": 10 }}))
job Once on default (retry: 0, unique: 3600, concurrency: 1) {{
 perform() {{ unwrap(append_file({}, "effect\n"))
 {pause} }}
}}
if unwrap(get_env("CLAIM_FIXTURE_MODE")) == "seed" {{
 let id=unwrap(enqueue("Once",map {{ "scope": {scope} }}))
 unwrap(write_file({},id))
}} else {{ work_jobs(map {{ "concurrency": 1, "poll_interval": 50, "worker_group": unwrap(get_env("CLAIM_FIXTURE_GROUP")) }}) }}
"#,
        store
            .map(|s| serde_json::to_string(s).unwrap())
            .unwrap_or_else(|| quote(&db)),
        quote(&effect),
        quote(&id)
    );
    std::fs::write(&file, source).unwrap();
    let out = command(&file, "seed").output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    (file, db, effect)
}
fn start(file: &Path) -> Worker {
    Worker(
        command(file, "work")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    )
}
fn kill(worker: &mut Worker) {
    worker.0.kill().unwrap();
    worker.0.wait().unwrap();
}

#[test]
fn killed_unstarted_claim_is_recovered_and_executed_once() {
    let dir = tempfile::tempdir().unwrap();
    let (file, db, effect) = fixture(dir.path(), false);
    let id = std::fs::read_to_string(dir.path().join("id.txt")).unwrap();
    let conn = rusqlite::Connection::open(db).unwrap();
    // Keep the first process behind the execution-authorization write.
    conn.execute_batch("CREATE TRIGGER block_start BEFORE INSERT ON _kv WHEN NEW.key LIKE 'jobs:data:%' AND json_extract(NEW.value,'$.status')='active' BEGIN SELECT RAISE(ABORT,'hold before execution'); END").unwrap();
    let mut first = start(&file);
    wait(|| record(&conn, &id)["status"] == "claimed");
    let old_token = record(&conn, &id)["claim_token"].clone();
    kill(&mut first);
    assert!(!effect.exists(), "claimed phase must not execute the body");
    conn.execute_batch("DROP TRIGGER block_start").unwrap();
    let mut second = start(&file);
    wait(|| record(&conn, &id)["status"] == "completed");
    kill(&mut second);
    assert_ne!(record(&conn, &id)["claim_token"], old_token);
    assert_eq!(std::fs::read_to_string(&effect).unwrap(), "effect\n");
    // Completion still owns its independent uniqueness window after recovery.
    assert!(command(&file, "seed").output().unwrap().status.success());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("id.txt")).unwrap(),
        id
    );
}

#[test]
fn killed_execution_becomes_unknown_without_replaying_effect() {
    let dir = tempfile::tempdir().unwrap();
    let (file, db, effect) = fixture(dir.path(), true);
    let id = std::fs::read_to_string(dir.path().join("id.txt")).unwrap();
    let conn = rusqlite::Connection::open(db).unwrap();
    let mut first = start(&file);
    wait(|| effect.exists());
    assert_eq!(record(&conn, &id)["status"], "active");
    kill(&mut first);
    let mut second = start(&file);
    wait(|| record(&conn, &id)["status"] == "outcome_unknown");
    kill(&mut second);
    assert_eq!(std::fs::read_to_string(&effect).unwrap(), "effect\n");
    let ttl: Option<i64> = conn
        .query_row(
            "SELECT expires_at FROM _kv WHERE key=?",
            [format!("jobs:data:{id}")],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        ttl, None,
        "uncertain work must not expire as terminal history"
    );
    let out = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .env_remove("NTNT_STRICT")
        .env("CLAIM_FIXTURE_MODE", "work")
        .env("CLAIM_FIXTURE_GROUP", "inspect")
        .args(["jobs", "list"])
        .arg(&file)
        .args(["--status", "outcome_unknown", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let rows: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(rows.as_array().unwrap().iter().any(|r| r["id"] == id));
}

#[cfg(unix)]
#[test]
fn resumed_stale_process_cannot_execute_recovered_claim() {
    let dir = tempfile::tempdir().unwrap();
    let (file, db, effect) = fixture(dir.path(), false);
    let id = std::fs::read_to_string(dir.path().join("id.txt")).unwrap();
    let conn = rusqlite::Connection::open(db).unwrap();
    conn.execute_batch("CREATE TRIGGER block_start BEFORE INSERT ON _kv WHEN NEW.key LIKE 'jobs:data:%' AND json_extract(NEW.value,'$.status')='active' BEGIN SELECT RAISE(ABORT,'hold before execution'); END").unwrap();
    let mut first = start(&file);
    wait(|| record(&conn, &id)["status"] == "claimed");
    assert!(Command::new("kill")
        .args(["-STOP", &first.0.id().to_string()])
        .status()
        .unwrap()
        .success());
    conn.execute_batch("DROP TRIGGER block_start").unwrap();
    let mut second = start(&file);
    wait(|| record(&conn, &id)["status"] == "completed");
    assert!(Command::new("kill")
        .args(["-CONT", &first.0.id().to_string()])
        .status()
        .unwrap()
        .success());
    std::thread::sleep(Duration::from_millis(400));
    kill(&mut first);
    kill(&mut second);
    assert_eq!(std::fs::read_to_string(effect).unwrap(), "effect\n");
    assert_eq!(record(&conn, &id)["status"], "completed");
}

#[test]
#[ignore = "requires dedicated NTNT_CLAIMS_TEST_REDIS"]
fn redis_worker_crash_does_not_replay_recorded_effect() {
    let url = std::env::var("NTNT_CLAIMS_TEST_REDIS").expect("dedicated Redis required");
    let dir = tempfile::tempdir().unwrap();
    let (file, _, effect) = fixture_store(dir.path(), true, Some(&url));
    let id = std::fs::read_to_string(dir.path().join("id.txt")).unwrap();
    let client = redis::Client::open(url).unwrap();
    let mut conn = client.get_connection().unwrap();
    let mut first = start(&file);
    wait(|| effect.exists());
    kill(&mut first);
    let mut second = start(&file);
    wait(|| {
        let raw: String = redis::cmd("GET")
            .arg(format!("jobs:data:{id}"))
            .query(&mut conn)
            .unwrap();
        let data: serde_json::Value = serde_json::from_str(&raw).unwrap();
        data["v"]["status"] == "outcome_unknown"
    });
    kill(&mut second);
    assert_eq!(std::fs::read_to_string(effect).unwrap(), "effect\n");
    let ttl: i64 = redis::cmd("PTTL")
        .arg(format!("jobs:data:{id}"))
        .query(&mut conn)
        .unwrap();
    assert_eq!(ttl, -1);
}
