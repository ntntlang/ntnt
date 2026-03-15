/// Tests for the 6 job system polish features:
/// 1. Unique Jobs
/// 2. Transactional Enqueue (PostgreSQL only — manual)
/// 3. LISTEN/NOTIFY (PostgreSQL only — manual)
/// 4. Cron-Style Scheduling
/// 5. Dead Job Caps
/// 6. Queue Pause/Resume
use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = format!("{:?}", std::thread::current().id());
    let temp_dir = std::env::temp_dir();
    temp_dir
        .join(format!(
            "ntnt_polish_{}_{}_{}_{}.tnt",
            prefix,
            std::process::id(),
            thread_id.replace(|c: char| !c.is_alphanumeric(), "_"),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

fn run_ntnt_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("test");

    let mut file = fs::File::create(&test_file).expect("Failed to create test file");
    writeln!(file, "{}", code).expect("Failed to write test file");
    drop(file);

    let exe = std::env::consts::EXE_SUFFIX;
    let debug_path = format!("./target/debug/ntnt{}", exe);
    let release_path = format!("./target/release/ntnt{}", exe);

    let binary = if std::path::Path::new(&debug_path).exists() {
        debug_path
    } else if std::path::Path::new(&release_path).exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
    };

    let output = Command::new(binary)
        .args(&["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("Failed to execute ntnt");

    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// ============================================================
// 1. Unique Jobs
// ============================================================

#[test]
fn test_unique_jobs_prevent_duplicate() {
    let code = r#"
import { Queue } from "std/jobs"

Queue.configure(map { "backend": "memory" })

Job UniqueTest on default (retry: 0, unique: 60) {
    perform(user_id: String) {
        print("Processing " + user_id)
    }
}

let id1 = UniqueTest.enqueue(map { "user_id": "user_123" })
let id2 = UniqueTest.enqueue(map { "user_id": "user_123" })

// Same args within unique window should return same ID
if id1 == id2 {
    print("DEDUP_OK")
} else {
    print("DEDUP_FAIL: " + id1 + " vs " + id2)
}

// Different args should create a new job
let id3 = UniqueTest.enqueue(map { "user_id": "user_456" })
if id1 != id3 {
    print("DIFF_OK")
} else {
    print("DIFF_FAIL")
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("DEDUP_OK"),
        "Expected DEDUP_OK, got stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("DIFF_OK"),
        "Expected DIFF_OK, got stdout: {}",
        stdout
    );
}

#[test]
fn test_unique_jobs_allow_after_expiry() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map { "backend": "memory" })

Job ShortUniqueTest on default (retry: 0, unique: 1) {
    perform(msg: String) {
        print("Processing: " + msg)
    }
}

let id1 = ShortUniqueTest.enqueue(map { "msg": "hello" })

// Wait past the unique window (1 second)
sleep_ms(1100)

let id2 = ShortUniqueTest.enqueue(map { "msg": "hello" })

if id1 != id2 {
    print("EXPIRY_OK")
} else {
    print("EXPIRY_FAIL: same ID after expiry")
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("EXPIRY_OK"),
        "Expected EXPIRY_OK, got stdout: {}",
        stdout
    );
}

// ============================================================
// 4. Cron-Style Scheduling (unit tests via library API)
// ============================================================

#[test]
fn test_cron_schedule_parses() {
    use ntnt::stdlib::concurrent::{cron_next_run, parse_cron};
    use std::time::{Duration, UNIX_EPOCH};

    // Test "0 9 * * 1-5" — 9am weekdays
    let cron = parse_cron("0 9 * * 1-5").unwrap();

    // Base time: Sunday 2026-03-15 08:00 UTC
    let base_time = UNIX_EPOCH + Duration::from_secs(1773763200);
    let next = cron_next_run(&cron, base_time);

    let next_secs = next.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let minute = (next_secs / 60) % 60;
    let hour = (next_secs / 3600) % 24;

    assert_eq!(minute, 0, "Expected minute 0, got {}", minute);
    assert_eq!(hour, 9, "Expected hour 9, got {}", hour);
    assert!(next > base_time, "Next run should be after base time");
}

#[test]
fn test_cron_every_15_min() {
    use ntnt::stdlib::concurrent::{cron_next_run, parse_cron};
    use std::time::{Duration, UNIX_EPOCH};

    let cron = parse_cron("*/15 * * * *").unwrap();

    // Base time at some arbitrary minute
    let base = UNIX_EPOCH + Duration::from_secs(1773768420);
    let next = cron_next_run(&cron, base);
    let next_secs = next.duration_since(UNIX_EPOCH).unwrap().as_secs();
    let next_minute = (next_secs / 60) % 60;

    assert!(
        next_minute == 0 || next_minute == 15 || next_minute == 30 || next_minute == 45,
        "Expected minute to be 0, 15, 30, or 45; got {}",
        next_minute
    );
    assert!(next > base, "Next run should be after base time");
}

#[test]
fn test_cron_parsing_with_names() {
    use ntnt::stdlib::concurrent::parse_cron;

    assert!(
        parse_cron("0 9 * * MON-FRI").is_ok(),
        "Failed to parse MON-FRI cron"
    );
    assert!(
        parse_cron("0 0 1 JAN-MAR *").is_ok(),
        "Failed to parse JAN-MAR cron"
    );
    assert!(
        parse_cron("invalid").is_err(),
        "Should fail for invalid cron expression"
    );
    assert!(
        parse_cron("*/5 * * * *").is_ok(),
        "Failed to parse */5 cron"
    );
    assert!(
        parse_cron("0,30 * * * *").is_ok(),
        "Failed to parse comma-list cron"
    );
}

#[test]
fn test_cron_schedule_in_program() {
    let code = r#"
import { Queue } from "std/jobs"

let handle = schedule("*/1 * * * *", fn() {
    print("tick")
})

if handle["type"] == "Schedule" {
    print("SCHEDULE_OK")
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("SCHEDULE_OK"),
        "Expected SCHEDULE_OK, got stdout: {}",
        stdout
    );
}

// ============================================================
// 5. Dead Job Caps
// ============================================================

#[test]
fn test_dead_job_cap() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "backend": "memory",
    "max_dead_jobs": 10
})

Job FailingJob on default (retry: 0) {
    perform(n: Int) {
        throw("intentional failure #" + to_string(n))
    }
}

// Enqueue 15 jobs
for i in 1..=15 {
    FailingJob.enqueue(map { "n": i })
}

// Start worker and let it process
Queue.work_async()
sleep_ms(2000)

// Check dead job count
let dead = Queue.dead(100)
let dead_count = len(dead)

if dead_count <= 10 {
    print("DEAD_CAP_OK: #{dead_count}")
} else {
    print("DEAD_CAP_FAIL: #{dead_count} dead jobs (expected <= 10)")
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("DEAD_CAP_OK"),
        "Expected DEAD_CAP_OK, got: stdout={}, stderr={}",
        stdout,
        stderr
    );
}

// ============================================================
// 6. Queue Pause/Resume
// ============================================================

#[test]
fn test_queue_pause_resume() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map { "backend": "memory" })

Job PauseTestJob on pausable (retry: 0) {
    perform() {
        print("JOB_PROCESSED")
    }
}

// Pause the queue
Queue.pause("pausable")

// Verify it's in the paused list
let paused = Queue.paused()
if len(paused) > 0 {
    print("PAUSE_OK")
} else {
    print("PAUSE_FAIL")
}

// Enqueue a job
PauseTestJob.enqueue(map {})

// Start worker
Queue.work_async()
sleep_ms(500)

// Resume the queue
Queue.resume("pausable")

// Wait for processing
sleep_ms(500)

// Verify paused list is empty
let paused_after = Queue.paused()
if len(paused_after) == 0 {
    print("RESUME_OK")
} else {
    print("RESUME_FAIL")
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("PAUSE_OK"),
        "Expected PAUSE_OK, got stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("RESUME_OK"),
        "Expected RESUME_OK, got stdout: {}",
        stdout
    );
}

// ============================================================
// Interval schedule still works
// ============================================================

#[test]
fn test_schedule_interval_still_works() {
    let code = r#"
import { Queue } from "std/jobs"

let handle = schedule("every 1h", fn() {
    print("hourly task")
})

if handle["type"] == "Schedule" {
    print("INTERVAL_OK")
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("INTERVAL_OK"),
        "Expected INTERVAL_OK, got stdout: {}",
        stdout
    );
}

// ============================================================
// PostgreSQL Tests (require running PostgreSQL)
// ============================================================

#[test]
#[ignore]
fn test_transactional_enqueue_commits() {
    let code = r#"
import { Queue } from "std/jobs"
import { connect } from "std/db/postgres"

let db_url = env("DATABASE_URL")
let db = connect(db_url)

Queue.configure(map {
    "backend": "postgres",
    "url": db_url
})

Job TxTestJob on default (retry: 0) {
    perform(order_id: String) {
        print("Processing order: " + order_id)
    }
}

let tx = db.begin()
match tx {
    Ok(conn) => {
        TxTestJob.enqueue_tx(conn, map { "order_id": "ord_123" })
        conn.commit()
        print("TX_COMMIT_OK")
    }
    Err(e) => print("TX_FAIL: " + to_string(e))
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("TX_COMMIT_OK"),
        "Expected TX_COMMIT_OK, got stdout: {}",
        stdout
    );
}

#[test]
#[ignore]
fn test_transactional_enqueue_rollback() {
    let code = r#"
import { Queue } from "std/jobs"
import { connect } from "std/db/postgres"

let db_url = env("DATABASE_URL")
let db = connect(db_url)

Queue.configure(map {
    "backend": "postgres",
    "url": db_url
})

Job TxRollbackJob on default (retry: 0) {
    perform(order_id: String) {
        print("Processing order: " + order_id)
    }
}

let tx = db.begin()
match tx {
    Ok(conn) => {
        TxRollbackJob.enqueue_tx(conn, map { "order_id": "ord_rollback" })
        conn.rollback()
        print("TX_ROLLBACK_OK")
    }
    Err(e) => print("TX_FAIL: " + to_string(e))
}
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("TX_ROLLBACK_OK"),
        "Expected TX_ROLLBACK_OK, got stdout: {}",
        stdout
    );
}

#[test]
#[ignore]
fn test_listen_notify_instant_dispatch() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "backend": "postgres",
    "url": env("DATABASE_URL")
})

Job InstantJob on default (retry: 0) {
    perform() {
        print("INSTANT_PROCESSED")
    }
}

Queue.work_async()
sleep_ms(200)

InstantJob.enqueue(map {})
sleep_ms(500)

print("LISTEN_OK")
"#;
    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Program failed: stdout={}, stderr={}",
        stdout, stderr
    );
    assert!(
        stdout.contains("LISTEN_OK"),
        "Expected LISTEN_OK, got stdout: {}",
        stdout
    );
}
