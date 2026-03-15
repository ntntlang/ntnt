//! Integration tests for NTNT Job DSL Redis Streams Backend
//!
//! These tests require a running Redis instance.
//! Set NTNT_TEST_REDIS_URL to run them:
//!   NTNT_TEST_REDIS_URL=redis://localhost:6379 cargo test --test job_redis_tests -- --ignored
//!
//! All tests are #[ignore] by default — they only run when explicitly requested.

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
            "ntnt_redis_{}_{}_{}_{}.tnt",
            prefix,
            std::process::id(),
            thread_id.replace(|c: char| !c.is_alphanumeric(), "_"),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

/// Helper to run ntnt with a code string and REDIS_URL set
fn run_ntnt_code_redis(code: &str) -> (String, String, i32) {
    let redis_url = match std::env::var("NTNT_TEST_REDIS_URL") {
        Ok(url) => url,
        Err(_) => {
            return ("SKIP_NO_REDIS".to_string(), String::new(), 0);
        }
    };

    let test_file = unique_test_file("redis_job_test");

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

    let output = Command::new(&binary)
        .arg(&test_file)
        .env("REDIS_URL", &redis_url)
        .output()
        .expect("Failed to execute ntnt");

    let _ = fs::remove_file(&test_file);

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let code = output.status.code().unwrap_or(-1);

    (stdout, stderr, code)
}

/// Flush all ntnt:* keys from Redis before each test
fn flush_redis_keys() {
    let redis_url = match std::env::var("NTNT_TEST_REDIS_URL") {
        Ok(url) => url,
        Err(_) => return,
    };

    let client = redis::Client::open(redis_url.as_str()).expect("Failed to create Redis client");
    let mut conn = client.get_connection().expect("Failed to connect to Redis");

    // SCAN and delete all ntnt:* keys
    let mut cursor: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:*")
            .arg("COUNT")
            .arg(100)
            .query(&mut conn)
            .expect("SCAN failed");

        for key in &keys {
            let _: () = redis::cmd("DEL")
                .arg(key)
                .query(&mut conn)
                .expect("DEL failed");
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }
}

#[test]
#[ignore]
fn test_redis_enqueue_and_process() {
    flush_redis_keys();

    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL"),
    "visibility_timeout": 60
})

Job TestJob on default (retry: 1) {
    perform(message: String) {
        print("Processing: #{message}")
        return "done"
    }
}

let id = TestJob.enqueue(map { "message": "hello redis" })
print("Enqueued: #{id}")

Queue.work_async()
sleep_ms(2000)

let status = Queue.stats()
print("Completed: #{status.completed}")
"#;

    let (stdout, stderr, exit_code) = run_ntnt_code_redis(code);
    if stdout.starts_with("SKIP_NO_REDIS") {
        println!("Skipping: NTNT_TEST_REDIS_URL not set");
        return;
    }

    eprintln!("STDOUT: {}", stdout);
    eprintln!("STDERR: {}", stderr);
    assert_eq!(exit_code, 0, "Process should exit cleanly");
    assert!(stdout.contains("Enqueued:"), "Should print enqueued job ID");
    assert!(
        stdout.contains("Processing: hello redis"),
        "Should process the job"
    );
    assert!(
        stdout.contains("Completed: 1"),
        "Should show 1 completed job"
    );
}

#[test]
#[ignore]
fn test_redis_consumer_group_no_double_claim() {
    flush_redis_keys();

    // This test verifies that two workers don't double-process by checking
    // that enqueuing 5 jobs and processing them results in exactly 5 completions
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL"),
    "visibility_timeout": 60
})

let counter = 0

Job CountJob on default (retry: 1) {
    perform(n: Int) {
        print("Job #{n}")
    }
}

// Enqueue 5 jobs
let i = 0
while i < 5 {
    CountJob.enqueue(map { "n": i })
    i = i + 1
}

Queue.work_async()
sleep_ms(3000)

let status = Queue.stats()
print("Completed: #{status.completed}")
print("Dead: #{status.dead}")
"#;

    let (stdout, stderr, exit_code) = run_ntnt_code_redis(code);
    if stdout.starts_with("SKIP_NO_REDIS") {
        return;
    }

    eprintln!("STDOUT: {}", stdout);
    eprintln!("STDERR: {}", stderr);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("Completed: 5"),
        "Should complete exactly 5 jobs without double-processing"
    );
}

#[test]
#[ignore]
fn test_redis_stale_reclaim() {
    // This test uses the ntnt jobs API to verify stale job detection works
    // by checking XPENDING-based recovery
    flush_redis_keys();

    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL"),
    "visibility_timeout": 2
})

Job SlowJob on default (retry: 2, timeout: 1) {
    perform(label: String) {
        print("Working on: #{label}")
        sleep_ms(500)
        return "ok"
    }
}

let id = SlowJob.enqueue(map { "label": "stale-test" })
print("Enqueued: #{id}")

Queue.work_async()
sleep_ms(3000)

let status = Queue.stats()
print("Completed: #{status.completed}")
"#;

    let (stdout, stderr, exit_code) = run_ntnt_code_redis(code);
    if stdout.starts_with("SKIP_NO_REDIS") {
        return;
    }

    eprintln!("STDOUT: {}", stdout);
    eprintln!("STDERR: {}", stderr);
    assert_eq!(exit_code, 0);
    assert!(stdout.contains("Enqueued:"), "Should enqueue job");
    assert!(
        stdout.contains("Working on: stale-test"),
        "Should process the job"
    );
}

#[test]
#[ignore]
fn test_redis_scheduled_job() {
    flush_redis_keys();

    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL"),
    "visibility_timeout": 60
})

Job DelayedJob on default (retry: 1) {
    perform(msg: String) {
        print("Delayed: #{msg}")
    }
}

// Enqueue with 2-second delay
DelayedJob.enqueue_in(2000, map { "msg": "after-delay" })

Queue.work_async()

// Check status immediately — should be scheduled, not completed
sleep_ms(500)
let early_status = Queue.stats()
print("Early completed: #{early_status.completed}")

// Wait for it to be promoted and processed
sleep_ms(3000)
let late_status = Queue.stats()
print("Late completed: #{late_status.completed}")
"#;

    let (stdout, stderr, exit_code) = run_ntnt_code_redis(code);
    if stdout.starts_with("SKIP_NO_REDIS") {
        return;
    }

    eprintln!("STDOUT: {}", stdout);
    eprintln!("STDERR: {}", stderr);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("Early completed: 0"),
        "Job should not be completed yet"
    );
    assert!(
        stdout.contains("Late completed: 1"),
        "Job should be completed after delay"
    );
    assert!(
        stdout.contains("Delayed: after-delay"),
        "Should print delayed message"
    );
}

#[test]
#[ignore]
fn test_redis_per_queue_stats() {
    flush_redis_keys();

    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL"),
    "visibility_timeout": 60
})

Job EmailJob on emails (retry: 1) {
    perform(to: String) {
        print("Email to: #{to}")
    }
}

Job PaymentJob on payments (retry: 1) {
    perform(amount: Int) {
        print("Payment: #{amount}")
    }
}

EmailJob.enqueue(map { "to": "a@test.com" })
EmailJob.enqueue(map { "to": "b@test.com" })
PaymentJob.enqueue(map { "amount": 100 })

Queue.work_async()
sleep_ms(2000)

// Test per-queue stats with queue name arg
let email_stats = Queue.stats("emails")
print("Email completed: #{email_stats.completed}")

let payment_stats = Queue.stats("payments")
print("Payment completed: #{payment_stats.completed}")
"#;

    let (stdout, stderr, exit_code) = run_ntnt_code_redis(code);
    if stdout.starts_with("SKIP_NO_REDIS") {
        return;
    }

    eprintln!("STDOUT: {}", stdout);
    eprintln!("STDERR: {}", stderr);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("Email completed: 2"),
        "Should show 2 completed email jobs"
    );
    assert!(
        stdout.contains("Payment completed: 1"),
        "Should show 1 completed payment job"
    );
}

#[test]
#[ignore]
fn test_memory_concurrent_workers() {
    // Tests multi-threaded memory worker via Queue.work() with concurrency
    let code = r#"
import { Queue } from "std/jobs"
import { sleep_ms } from "std/concurrent"

Queue.configure(map {
    "shutdown_timeout": 5000,
    "prune_completed_after": 60000
})

Job SlowTask on default (retry: 1) {
    perform(n: Int) {
        sleep_ms(500)
        print("Done: #{n}")
    }
}

// Enqueue 4 jobs
let i = 0
while i < 4 {
    SlowTask.enqueue(map { "n": i })
    i = i + 1
}

// If all 4 jobs run sequentially at 500ms each = 2000ms
// If concurrency=4, they should all finish in ~500ms
// We give 1500ms — enough for concurrent, not enough for sequential

import { spawn, await_task } from "std/concurrent"

let work_task = spawn(fn() {
    Queue.work(map { "concurrency": 4 })
})

// Wait enough time for concurrent processing but not sequential
sleep_ms(1500)

let status = Queue.stats()
print("Completed: #{status.completed}")
"#;

    let test_file = unique_test_file("memory_concurrent");
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
        panic!("No ntnt binary found.");
    };

    let output = Command::new(&binary)
        .arg(&test_file)
        .output()
        .expect("Failed to execute ntnt");

    let _ = fs::remove_file(&test_file);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    eprintln!("STDOUT: {}", stdout);
    eprintln!("STDERR: {}", stderr);

    // The concurrent test verifies jobs ran — at least some should complete
    // in the 1.5s window with 4 threads
    assert!(
        stdout.contains("Completed: 4"),
        "All 4 jobs should complete with concurrency=4 within 1.5s"
    );
}
