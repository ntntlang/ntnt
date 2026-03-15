//! Integration tests for NTNT Job DSL (DD-037 Phase 2)
//!
//! Tests Job declaration, enqueue, worker processing, retry, and queue management.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Generate a unique test file path
fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = format!("{:?}", std::thread::current().id());
    let temp_dir = std::env::temp_dir();
    temp_dir
        .join(format!(
            "ntnt_{}_{}_{}_{}.tnt",
            prefix,
            std::process::id(),
            thread_id.replace(|c: char| !c.is_alphanumeric(), "_"),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

/// Helper to run ntnt with a code string
fn run_ntnt_code(code: &str) -> (String, String, i32) {
    let test_file = unique_test_file("job_test");

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

#[test]
fn test_job_declaration_and_parse() {
    let code = r#"
Job Greet on default (retry: 3) {
    perform(name: String) {
        print("Hello, #{name}!")
    }
}

print("Job declared")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(
        code, 0,
        "Job declaration should succeed. stderr: {}",
        stderr
    );
    assert!(
        stdout.contains("Job declared"),
        "Should print after job declaration. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_enqueue_returns_id() {
    let code = r#"
Job Greet on default {
    perform(name: String) {
        print("Hello, #{name}!")
    }
}

let job_id = Greet.enqueue(map { "name": "World" })
print("Job ID: #{job_id}")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Job enqueue should succeed. stderr: {}", stderr);
    assert!(
        stdout.contains("Job ID: job_"),
        "Should return a job ID. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_enqueue_process_complete() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Job Greet on default {
    perform(name: String) {
        print("Hello, #{name}!")
    }
}

Greet.enqueue(map { "name": "World" })
Queue.work_async()
sleep_ms(500)

let status = Queue.status()
print("Completed: #{status.completed}")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Job processing should succeed. stderr: {}", stderr);
    assert!(
        stdout.contains("Hello, World!"),
        "Job perform body should execute. stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("Completed: 1"),
        "Should show 1 completed job. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_failure_retry_dead_letter() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Job Failing on default (retry: 2, backoff: 50) {
    perform(x: Int) {
        let result = 1 / 0
    }
}

Failing.enqueue(map { "x": 1 })
Queue.work_async()
sleep_ms(2000)

let status = Queue.status()
print("Dead: #{status.dead}")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Should not crash. stderr: {}", stderr);
    assert!(
        stdout.contains("Dead: 1"),
        "Failed job should end up in dead letter queue. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_on_failure_hook() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Job Failing on default (retry: 1, backoff: 50) {
    perform(x: Int) {
        let result = 1 / 0
    }
    on_failure(error, attempt) {
        print("FAILURE: #{error} attempt #{attempt}")
    }
}

Failing.enqueue(map { "x": 1 })
Queue.work_async()
sleep_ms(2000)

print("done")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Should not crash. stderr: {}", stderr);
    assert!(
        stdout.contains("FAILURE:"),
        "on_failure should fire. stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("attempt"),
        "on_failure should receive attempt count. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_enqueue_in_delay() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Job Delayed on default {
    perform(msg: String) {
        print("Delayed: #{msg}")
    }
}

Delayed.enqueue_in(500, map { "msg": "hello" })
Queue.work_async()

// Check immediately — should not have processed yet
sleep_ms(100)
let early_status = Queue.status()
print("Early completed: #{early_status.completed}")

// Wait for delay to pass
sleep_ms(600)
let late_status = Queue.status()
print("Late completed: #{late_status.completed}")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Should not crash. stderr: {}", stderr);
    assert!(
        stdout.contains("Early completed: 0"),
        "Job should not process before delay. stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("Late completed: 1"),
        "Job should process after delay. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_cancel() {
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Job Cancellable on default {
    perform(x: Int) {
        print("Should not run")
    }
}

// Enqueue with delay so it stays pending
let job_id = Cancellable.enqueue_in(10000, map { "x": 1 })
let cancelled = Queue.cancel(job_id)
print("Cancelled: #{cancelled}")

let status = Queue.status()
print("Cancelled count: #{status.cancelled}")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Should not crash. stderr: {}", stderr);
    assert!(
        stdout.contains("Cancelled: true"),
        "Should be able to cancel a pending job. stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("Cancelled count: 1"),
        "Status should show 1 cancelled. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_queue_status() {
    let code = r#"
import { Queue } from "std/jobs"

Job Counter on default {
    perform(n: Int) {
        print("count #{n}")
    }
}

Counter.enqueue(map { "n": 1 })
Counter.enqueue(map { "n": 2 })
Counter.enqueue(map { "n": 3 })

let status = Queue.status()
print("Pending: #{status.pending}")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Should not crash. stderr: {}", stderr);
    assert!(
        stdout.contains("Pending: 3"),
        "Should show 3 pending jobs. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_with_options() {
    let code = r#"
Job Configured on emails (retry: 5, timeout: 30, backoff: 500) {
    perform(to: String) {
        print("Sending to #{to}")
    }
}

let job_id = Configured.enqueue(map { "to": "test@example.com" })
print("Enqueued: #{job_id}")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Should parse all options. stderr: {}", stderr);
    assert!(
        stdout.contains("Enqueued: job_"),
        "Should enqueue with options. stdout: {}",
        stdout
    );
}

#[test]
fn test_job_queue_configure() {
    let code = r#"
import { Queue } from "std/jobs"

Queue.configure(map { "shutdown_timeout": 5000, "prune_completed_after": 60000 })
print("Configured")
"#;

    let (stdout, stderr, code) = run_ntnt_code(code);
    assert_eq!(code, 0, "Queue.configure should work. stderr: {}", stderr);
    assert!(
        stdout.contains("Configured"),
        "Should configure successfully. stdout: {}",
        stdout
    );
}
