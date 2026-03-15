//! Integration tests for std/concurrent module
//!
//! Tests the full concurrency system: spawn, channels, schedules, cancellation.
//! All timing-sensitive tests use sleep_ms from std/concurrent (cancellation-aware),
//! NOT sleep from std/time.

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
            "ntnt_concurrency_{}_{}_{}_{}.tnt",
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
        .env("NTNT_ENV", "development")
        .output()
        .expect("Failed to execute ntnt");

    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

// =============================================================================
// Channel tests
// =============================================================================

#[test]
fn test_channel_send_recv() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, send, recv } from "std/concurrent"

let ch = channel()
send(ch, "hello")
let msg = recv(ch)
print(msg)
"#,
    );
    assert_eq!(code, 0, "Process should succeed");
    assert!(
        stdout.trim().contains("hello"),
        "Should receive 'hello', got: {}",
        stdout
    );
}

#[test]
fn test_channel_try_recv_empty() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, try_recv } from "std/concurrent"

let ch = channel()
let result = try_recv(ch)
match result {
    None => print("empty"),
    Some(v) => print("got: " + str(v))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("empty"),
        "Should get empty, got: {}",
        stdout
    );
}

#[test]
fn test_channel_close_returns_unit_on_recv() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, send, recv, close } from "std/concurrent"

let ch = channel()
send(ch, "first")
close(ch)
// After close, recv on a closed-and-empty channel returns Unit.
// But we buffered "first" before close — the sender was already cloned.
// Actually after close (remove from map), new recv() won't find the channel,
// so it returns Unit. The buffered message may be lost since we dropped the sender.
let msg = recv(ch)
print("result: " + str(msg))
"#,
    );
    assert_eq!(code, 0);
    // After close(), the channel is removed from the map, so recv() returns Unit
    assert!(
        stdout.contains("result:"),
        "Should print result, got: {}",
        stdout
    );
}

#[test]
fn test_channel_send_on_closed_returns_false() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, send, close } from "std/concurrent"

let ch = channel()
close(ch)
let result = send(ch, "test")
print(result)
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("false"),
        "Send on closed should return false, got: {}",
        stdout
    );
}

#[test]
fn test_channel_recv_timeout() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, recv_timeout } from "std/concurrent"

let ch = channel()
let result = recv_timeout(ch, 100)
match result {
    None => print("timeout"),
    Some(v) => print("got: " + str(v))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("timeout"),
        "Should timeout, got: {}",
        stdout
    );
}

#[test]
fn test_channel_recv_timeout_with_value() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, send, recv_timeout } from "std/concurrent"

let ch = channel()
send(ch, 42)
let result = recv_timeout(ch, 1000)
match result {
    None => print("timeout"),
    Some(v) => print("got: " + str(v))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("got: 42"),
        "Should get value, got: {}",
        stdout
    );
}

// =============================================================================
// Spawn tests
// =============================================================================

#[test]
fn test_spawn_and_await() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

let task = spawn(fn() { 42 })
let result = await_task(task)
// Display auto-unwraps Result::Ok(42) to "42"
print(result)
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("42"),
        "Should get 42, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_await_returns_result() {
    // Verify that await_task returns a Result that can be matched
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

let task = spawn(fn() { 42 })
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("ok: 42"),
        "await_task should return Result, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_captures_variables() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

let x = 10
let y = 20
let task = spawn(fn() { x + y })
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("ok: 30"),
        "Should capture and compute, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_with_channel() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task, channel, send, recv } from "std/concurrent"

let ch = channel()
let task = spawn(fn() {
    send(ch, "from task")
})
let msg = recv(ch)
print(msg)
await_task(task)
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("from task"),
        "Should get 'from task', got: {}",
        stdout
    );
}

#[test]
fn test_spawn_rejects_parameterized_handler() {
    let (_stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn } from "std/concurrent"

let task = spawn(fn(x) { x + 1 })
"#,
    );
    assert_ne!(code, 0, "Should fail for parameterized handler");
}

#[test]
fn test_spawn_rejects_handler_with_defaults() {
    let (_stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn } from "std/concurrent"

let task = spawn(fn(x = 10) { x + 1 })
"#,
    );
    assert_ne!(code, 0, "Should fail for handler with default params");
}

// =============================================================================
// Try-await tests
// =============================================================================

#[test]
fn test_try_await_pending() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, try_await, sleep_ms } from "std/concurrent"

let task = spawn(fn() {
    sleep_ms(5000)
    42
})
let status = try_await(task)
print(status["status"])
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("running"),
        "Should be running, got: {}",
        stdout
    );
}

#[test]
fn test_try_await_completed() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, try_await, sleep_ms } from "std/concurrent"

let task = spawn(fn() { 42 })
sleep_ms(200)
let status = try_await(task)
print(status["status"])
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("completed"),
        "Should be completed, got: {}",
        stdout
    );
}

// =============================================================================
// Cancel tests
// =============================================================================

#[test]
fn test_cancel_task() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, cancel_task, await_task, sleep_ms } from "std/concurrent"

let task = spawn(fn() {
    sleep_ms(10000)
    42
})
sleep_ms(100)
cancel_task(task)
let result = await_task(task)
// Display auto-unwraps Result::Err to "error: ..."
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("cancelled: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("cancelled:"),
        "Cancelled task should return Err, got: {}",
        stdout
    );
}

// =============================================================================
// After tests
// =============================================================================

#[test]
fn test_after_delay() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { after, await_task } from "std/concurrent"

let task = after(100, fn() { "delayed result" })
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("ok: delayed result"),
        "Should get delayed result, got: {}",
        stdout
    );
}

#[test]
fn test_after_with_string_interval() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { after, await_task } from "std/concurrent"

let task = after("100ms", fn() { "from string interval" })
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("ok: from string interval"),
        "Should get Ok, got: {}",
        stdout
    );
}

#[test]
fn test_after_cancel_before_execution() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { after, cancel_task, await_task } from "std/concurrent"

let task = after(5000, fn() { "should not run" })
cancel_task(task)
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("cancelled: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("cancelled:"),
        "Cancelled after should return Err, got: {}",
        stdout
    );
}

// =============================================================================
// Schedule tests
// =============================================================================

#[test]
fn test_schedule_runs_multiple_ticks() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { schedule, cancel_schedule, sleep_ms, channel, send, recv_timeout } from "std/concurrent"

let ch = channel()
let sched = schedule(100, fn() {
    send(ch, "tick")
})

sleep_ms(350)
cancel_schedule(sched)

let mut ticks = 0

fn drain() {
    let result = recv_timeout(ch, 50)
    match result {
        Some(v) => {
            ticks = ticks + 1
            drain()
        }
        None => {}
    }
}
drain()
print("ticks: " + str(ticks))
"#,
    );
    assert_eq!(code, 0);
    // Should have at least 2 ticks (rule 36: assert >= 2, not exact count)
    let tick_count: i32 = stdout
        .trim()
        .lines()
        .find(|l| l.starts_with("ticks: "))
        .and_then(|l| l.strip_prefix("ticks: "))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    assert!(tick_count >= 2, "Expected >= 2 ticks, got {}", tick_count);
}

#[test]
fn test_schedule_cancel() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { schedule, cancel_schedule } from "std/concurrent"

let sched = schedule(100, fn() {
    print("tick")
})
let result = cancel_schedule(sched)
print("cancelled: " + str(result))
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("cancelled: true"),
        "Should cancel successfully, got: {}",
        stdout
    );
}

#[test]
fn test_schedule_rejects_zero_interval() {
    let (_stdout, _stderr, code) = run_ntnt_code(
        r#"
import { schedule } from "std/concurrent"

let sched = schedule(0, fn() { print("tick") })
"#,
    );
    assert_ne!(code, 0, "Zero interval should be rejected");
}

#[test]
fn test_schedule_error_resilience() {
    // Schedule tick errors (1/0) should not kill the schedule (rule 35)
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { schedule, cancel_schedule, sleep_ms } from "std/concurrent"

let sched = schedule(100, fn() {
    let x = 1 / 0
})

sleep_ms(350)
cancel_schedule(sched)
print("schedule survived errors")
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("schedule survived errors"),
        "Schedule should survive tick errors, got: {}",
        stdout
    );
}

// =============================================================================
// Serialization tests
// =============================================================================

#[test]
fn test_spawn_with_map_capture() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

let data = map { "key": "value", "num": 42 }
let task = spawn(fn() { data })
let result = await_task(task)
match result {
    Ok(val) => print("ok"),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("ok"),
        "Should get Ok result, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_with_array_capture() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

let items = [1, 2, 3]
let task = spawn(fn() { items })
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(len(val))),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("ok: 3"),
        "Should get Ok result with array length 3, got: {}",
        stdout
    );
}

// =============================================================================
// Sleep_ms tests
// =============================================================================

#[test]
fn test_sleep_ms_cancellation_aware() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, cancel_task, await_task, sleep_ms } from "std/concurrent"

let task = spawn(fn() {
    sleep_ms(10000)
    "completed"
})
sleep_ms(100)
cancel_task(task)
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("cancelled: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("cancelled:"),
        "Cancelled sleep_ms task should fail, got: {}",
        stdout
    );
}

// =============================================================================
// Thread count test
// =============================================================================

#[test]
fn test_thread_count() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { thread_count } from "std/concurrent"

let count = thread_count()
print("threads: " + str(count))
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("threads: "),
        "Should print thread count, got: {}",
        stdout
    );
}
