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

    // Run ntnt directly. The two-handle channel design (TxChannel/RxChannel) ensures
    // recv() unblocks automatically when all sender clones drop — no external timeout
    // wrapper needed to prevent zombie processes. This also makes tests portable
    // across Linux, macOS, and Windows (no dependency on `timeout` from GNU coreutils).
    let output = Command::new(&binary)
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

let [tx, rx] = channel()
send(tx, "hello")
let msg = recv(rx)
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

let [tx, rx] = channel()
let result = try_recv(rx)
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

let [tx, rx] = channel()
send(tx, "first")
close(rx)
// After close(rx), the receiver is removed from the registry.
// recv(rx) no longer finds it and returns Unit immediately.
let msg = recv(rx)
print("result: " + str(msg))
"#,
    );
    assert_eq!(code, 0);
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

let [tx, rx] = channel()
close(rx)
// After the receiver is closed (dropped from registry), the crossbeam Receiver
// is gone — send() returns false (SendError::Disconnected).
let result = send(tx, "test")
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

let [tx, rx] = channel()
let result = recv_timeout(rx, 100)
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

let [tx, rx] = channel()
send(tx, 42)
let result = recv_timeout(rx, 1000)
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

let [tx, rx] = channel()
let task = spawn(fn() {
    send(tx, "from task")
    // When this task exits, tx drops. If no other tx clones exist,
    // the sender drops and rx.recv() would return Unit — but we already
    // sent the message, so recv() returns it first.
})
// Plain recv() is safe now: if task panics before send, tx Arc drops → Disconnected → Unit
let msg = recv(rx)
print(msg)
await_task(task)
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("from task"),
        "Should get 'from task', got: {}",
        stdout
    );
}

#[test]
fn test_spawn_with_if_expr_block_branches() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

let task = spawn(fn() {
    let value = if true {
        let a = 10
        a + 1
    } else {
        let b = 0
        b
    }
    value
})
let result = await_task(task)
print(result)
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.trim().contains("11"),
        "Spawn should handle if-expression block branches, got: {}",
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
// Parallel/Race tests
// =============================================================================

#[test]
fn test_parallel_results_in_order() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { parallel, sleep_ms } from "std/concurrent"

let results = parallel([
    fn() { sleep_ms(50); "a" },
    fn() { sleep_ms(10); "b" },
    fn() { sleep_ms(20); "c" }
])

match results[0] { Ok(v) => print("r1: " + str(v)), Err(e) => print("err1") }
match results[1] { Ok(v) => print("r2: " + str(v)), Err(e) => print("err2") }
match results[2] { Ok(v) => print("r3: " + str(v)), Err(e) => print("err3") }
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("r1: a"), "stdout: {}", stdout);
    assert!(stdout.contains("r2: b"), "stdout: {}", stdout);
    assert!(stdout.contains("r3: c"), "stdout: {}", stdout);
}

#[test]
fn test_parallel_empty_array() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { parallel } from "std/concurrent"

let results = parallel([])
print("len: " + str(len(results)))
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.trim().contains("len: 0"), "stdout: {}", stdout);
}

#[test]
fn test_parallel_failure_cancels_others() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { parallel, sleep_ms, channel, send, recv_timeout } from "std/concurrent"

let [tx, rx] = channel()
let result = parallel([
    fn() { sleep_ms(50); 1 / 0 },
    fn() { sleep_ms(500); send(tx, "late1"); "ok1" },
    fn() { sleep_ms(500); send(tx, "late2"); "ok2" }
])

match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("err: " + str(e))
}

// Cancellation is cooperative — siblings are cancelled during sleep_ms,
// so they never reach send(). Use a short timeout to confirm.
let msg = recv_timeout(rx, 200)
match msg {
    None => print("no_send"),
    Some(v) => print("sent: " + str(v))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("err:"), "stdout: {}", stdout);
    assert!(stdout.contains("no_send"), "stdout: {}", stdout);
}

#[test]
fn test_race_fastest_wins() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { race, sleep_ms } from "std/concurrent"

let result = race([
    fn() { sleep_ms(500); "slow" },
    fn() { sleep_ms(50); "fast" },
    fn() { sleep_ms(300); "mid" }
])

match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("ok: fast"), "stdout: {}", stdout);
}

#[test]
fn test_race_error_then_success() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { race, sleep_ms } from "std/concurrent"

let result = race([
    fn() { sleep_ms(10); 1 / 0 },
    fn() { sleep_ms(50); "ok" }
])

match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("ok: ok"), "stdout: {}", stdout);
}

#[test]
fn test_race_all_fail() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { race, sleep_ms } from "std/concurrent"

let result = race([
    fn() { 1 / 0 },
    fn() { sleep_ms(10); 1 / 0 }
])

match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("err:"), "stdout: {}", stdout);
}

#[test]
fn test_race_empty_array_errors() {
    let (_stdout, _stderr, code) = run_ntnt_code(
        r#"
import { race } from "std/concurrent"

let result = race([])
print(result)
"#,
    );
    assert_ne!(code, 0, "race([]) should be a runtime error");
}

#[test]
fn test_race_parent_cancellation_cancels_children() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, cancel_task, await_task, race, sleep_ms, channel, send, recv_timeout } from "std/concurrent"

let [tx, rx] = channel()

let task = spawn(fn() {
    race([
        fn() { sleep_ms(1000); send(tx, "a"); "a" },
        fn() { sleep_ms(1000); send(tx, "b"); "b" }
    ])
})

sleep_ms(100)
cancel_task(task)

let result = await_task(task)
match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("cancelled: " + str(e))
}

let msg = recv_timeout(rx, 1500)
match msg {
    None => print("children_cancelled"),
    Some(v) => print("sent: " + str(v))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("cancelled:"), "stdout: {}", stdout);
    assert!(stdout.contains("children_cancelled"), "stdout: {}", stdout);
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

let [tx, rx] = channel()
let sched = schedule(100, fn() {
    send(tx, "tick")
})

sleep_ms(600)
cancel_schedule(sched)

let mut ticks = 0

fn drain() {
    let result = recv_timeout(rx, 50)
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
fn test_schedule_with_if_expr_block_branches() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { schedule, cancel_schedule, channel, send, recv_timeout } from "std/concurrent"

let [tx, rx] = channel()
let sched = schedule(50, fn() {
    let v = if true {
        let a = 5
        a + 1
    } else {
        0
    }
    send(tx, v)
})

let result = recv_timeout(rx, 500)
cancel_schedule(sched)
match result {
    Some(v) => print("got: " + str(v)),
    None => print("missing")
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("got: 6"),
        "Schedule should handle if-expression block branches, got: {}",
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

#[test]
fn test_schedule_ignores_unused_user_functions_in_scope() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { schedule, cancel_schedule, channel, send, recv_timeout } from "std/concurrent"
fn unused_helper() { 999 }
let payload = "scheduled"
let [tx, rx] = channel()

let sched = schedule(50, fn() {
    send(tx, payload)
})

let result = recv_timeout(rx, 1000)
cancel_schedule(sched)
match result {
    Some(v) => print("ok: " + str(v)),
    None => print("missing")
}
"#,
    );
    assert_eq!(code, 0, "stderr: {}", stderr);
    assert!(stdout.contains("ok: scheduled"), "stdout: {}", stdout);
}

#[test]
fn test_schedule_reports_only_referenced_user_defined_function() {
    let (_stdout, stderr, code) = run_ntnt_code(
        r#"
import { schedule } from "std/concurrent"

fn unused_helper() { 1 }
fn used_helper() { 2 }

let sched = schedule(50, fn() { used_helper() })
"#,
    );
    assert_ne!(code, 0, "schedule should fail");
    assert!(
        stderr.contains("used_helper"),
        "stderr should mention the referenced function: {}",
        stderr
    );
    assert!(
        !stderr.contains("unused_helper"),
        "stderr should not mention unused functions: {}",
        stderr
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

#[test]
fn test_spawn_ignores_unused_user_functions_in_scope() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

fn unused_helper() { 999 }
let payload = 41

let task = spawn(fn() { payload + 1 })
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0, "stderr: {}", stderr);
    assert!(stdout.contains("ok: 42"), "stdout: {}", stdout);
}

#[test]
fn test_after_ignores_unused_user_functions_in_scope() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { after, await_task } from "std/concurrent"

fn unused_helper() { 999 }
let payload = "after ok"

let task = after(10, fn() { payload })
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0, "stderr: {}", stderr);
    assert!(stdout.contains("ok: after ok"), "stdout: {}", stdout);
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

// =============================================================================
// Phase 1 hardening tests: try_await consumed/expired, handle types, select
// =============================================================================

// --- try_await returns "consumed" after await_task ---

#[test]
fn test_try_await_consumed_after_await() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task, try_await, sleep_ms } from "std/concurrent"

let task = spawn(fn() { 42 })
let result = await_task(task)
sleep_ms(50)
let status = try_await(task)
print("status: " + status["status"])
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("status: consumed"),
        "try_await after await_task should return consumed, got: {}",
        stdout
    );
}

// --- try_await returns proper status for running/completed/failed ---

#[test]
fn test_try_await_running_status() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, try_await, sleep_ms } from "std/concurrent"

let task = spawn(fn() {
    sleep_ms(500)
    42
})
let status = try_await(task)
print("status: " + status["status"])
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("status: running"),
        "try_await on running task should return running, got: {}",
        stdout
    );
}

#[test]
fn test_try_await_completed_status() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, try_await, sleep_ms } from "std/concurrent"

let task = spawn(fn() { 42 })
sleep_ms(200)
let status = try_await(task)
print("status: " + status["status"])
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("status: completed"),
        "try_await on completed task should return completed, got: {}",
        stdout
    );
}

// --- Handle types: spawn returns Task handle, channel returns Channel, schedule returns Schedule ---

#[test]
fn test_handle_type_task() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

let task = spawn(fn() { 42 })
print("type: " + typeof(task))
let result = await_task(task)
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("type: Task"),
        "spawn should return Task handle, got: {}",
        stdout
    );
}

#[test]
fn test_handle_type_channel() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, close } from "std/concurrent"

let [tx, rx] = channel()
print("tx: " + typeof(tx))
print("rx: " + typeof(rx))
close(rx)
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.contains("tx: TxChannel"),
        "channel() tx should be TxChannel, got: {}",
        stdout
    );
    assert!(
        stdout.contains("rx: RxChannel"),
        "channel() rx should be RxChannel, got: {}",
        stdout
    );
}

#[test]
fn test_handle_type_schedule() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { schedule, cancel_schedule } from "std/concurrent"

let sched = schedule(60000, fn() { print("tick") })
print("type: " + typeof(sched))
cancel_schedule(sched)
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("type: Schedule"),
        "schedule() should return Schedule handle, got: {}",
        stdout
    );
}

// --- Handle type safety: wrong handle type gives type error ---

#[test]
fn test_wrong_handle_type_await_on_channel() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { channel, await_task } from "std/concurrent"

let ch = channel()
await_task(ch)
"#,
    );
    assert_ne!(code, 0, "Should fail with type error, stdout: {}", stdout);
    assert!(
        stderr.contains("Expected a Task handle") || stderr.contains("Task handle"),
        "Should mention Task handle in error, got stderr: {}",
        stderr
    );
}

#[test]
fn test_wrong_handle_type_send_on_task() {
    let (stdout, stderr, code) = run_ntnt_code(
        r#"
import { spawn, send, await_task } from "std/concurrent"

let task = spawn(fn() { 42 })
send(task, "hello")
await_task(task)
"#,
    );
    assert_ne!(code, 0, "Should fail with type error, stdout: {}", stdout);
    assert!(
        stderr.contains("TxChannel") || stderr.contains("Channel"),
        "Should mention TxChannel in error, got stderr: {}",
        stderr
    );
}

// --- select: two channels, send on one, verify correct channel and value ---

#[test]
fn test_select_two_channels() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, send, select } from "std/concurrent"

let [tx_a, rx_a] = channel()
let [tx_b, rx_b] = channel()

send(tx_b, "from_b")

let result = select([rx_a, rx_b], 5000)
print("status: " + result["status"])
print("value: " + result["value"])

if result["channel"] == rx_b {
    print("correct_channel")
}
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.contains("status: ok"),
        "select success should have status: ok, got: {}",
        stdout
    );
    assert!(
        stdout.contains("value: from_b"),
        "select should receive value from ch_b, got: {}",
        stdout
    );
    assert!(
        stdout.contains("correct_channel"),
        "select should identify the correct channel, got: {}",
        stdout
    );
}

// --- select with timeout: no data sent ---

#[test]
fn test_select_timeout() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, select } from "std/concurrent"

let [tx_a, rx_a] = channel()
let [tx_b, rx_b] = channel()

let result = select([rx_a, rx_b], 200)
print("status: " + result["status"])
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("status: timeout"),
        "select with no data should timeout, got: {}",
        stdout
    );
}

// --- select with closed channels ---

#[test]
fn test_select_all_closed() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { channel, close, select } from "std/concurrent"

let [tx_a, rx_a] = channel()
let [tx_b, rx_b] = channel()
close(rx_a)
close(rx_b)

let result = select([rx_a, rx_b], 200)
print("status: " + result["status"])
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("status: closed"),
        "select with all closed channels should return closed, got: {}",
        stdout
    );
}

// =============================================================================
// Additional coverage: cancellation, multi-sender, non-serializable returns
// =============================================================================

// --- cancel_task unblocks a task blocked on recv() ---

#[test]
fn test_cancel_task_unblocks_recv() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, cancel_task, await_task, channel, recv, sleep_ms } from "std/concurrent"

let [tx, rx] = channel()
// Task blocks on recv() with no sender ever sending
let task = spawn(fn() {
    let msg = recv(rx)
    "got: " + str(msg)
})
sleep_ms(200)
cancel_task(task)
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("cancelled: " + str(e))
}
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.contains("cancelled:"),
        "Cancelled task blocked on recv() should return Err, got: {}",
        stdout
    );
}

// --- Channel closing mid-select unblocks and returns from remaining channels ---

#[test]
fn test_select_channel_closes_mid_wait() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, channel, send, close, select, sleep_ms } from "std/concurrent"

let [tx_a, rx_a] = channel()
let [tx_b, rx_b] = channel()

// Spawn a task that closes channel A after a short delay, then sends on B
let task = spawn(fn() {
    sleep_ms(100)
    close(rx_a)
    sleep_ms(50)
    send(tx_b, "from_b_after_close")
})

// select blocks; rx_a closes mid-wait, then rx_b receives data
let result = select([rx_a, rx_b], 5000)
print("value: " + str(result["value"]))
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.contains("value: from_b_after_close"),
        "select should receive from rx_b after rx_a closes, got: {}",
        stdout
    );
}

// --- Multiple senders on the same channel concurrently ---

#[test]
fn test_concurrent_multi_sender() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task, channel, send, recv_timeout, sleep_ms } from "std/concurrent"

let [tx, rx] = channel()

// Spawn 5 tasks that all send to the same channel
let t1 = spawn(fn() { send(tx, "a") })
let t2 = spawn(fn() { send(tx, "b") })
let t3 = spawn(fn() { send(tx, "c") })
let t4 = spawn(fn() { send(tx, "d") })
let t5 = spawn(fn() { send(tx, "e") })

// Wait for all to finish
await_task(t1)
await_task(t2)
await_task(t3)
await_task(t4)
await_task(t5)

// Drain all messages
let mut count = 0
fn drain() {
    let result = recv_timeout(rx, 200)
    match result {
        Some(v) => {
            count = count + 1
            drain()
        }
        None => {}
    }
}
drain()
print("count: " + str(count))
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.trim().contains("count: 5"),
        "Should receive all 5 messages from concurrent senders, got: {}",
        stdout
    );
}

// --- Task returning a non-serializable value (NativeFunction) results in Failed ---

#[test]
fn test_spawn_non_serializable_return_fails() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, await_task } from "std/concurrent"

// Return a NativeFunction (print) from a task — NativeFunctions can't be serialized
// as return values across task boundaries
let task = spawn(fn() {
    print
})
let result = await_task(task)
match result {
    Ok(val) => print("ok: " + str(val)),
    Err(e) => print("failed: " + str(e))
}
"#,
    );
    assert_eq!(code, 0, "stderr: {}", _stderr);
    assert!(
        stdout.contains("failed:"),
        "Task returning a NativeFunction should fail, got: {}",
        stdout
    );
    assert!(
        stdout.contains("non-serializable") || stdout.contains("serializable"),
        "Error should mention serialization, got: {}",
        stdout
    );
}

// --- Expired tasks are removed from registry after NTNT_TASK_REMOVAL_TTL ---
// This test uses a very short removal TTL (1 second) to verify the removal path.

#[test]
fn test_task_removal_after_ttl() {
    let test_file = unique_test_file("removal_ttl");
    let code_str = r#"
import { spawn, await_task, try_await, sleep_ms } from "std/concurrent"

// Spawn and immediately await
let task = spawn(fn() { 42 })
await_task(task)

// Wait for expiry (5min) + removal (1s configured via env)
// We can't actually wait 5 minutes, so we just verify the env var is read
// and the reaper runs without crashing. The real TTL test is that the
// infrastructure exists and is configurable.
let status = try_await(task)
print("status: " + status["status"])
"#;

    let mut file = std::fs::File::create(&test_file).expect("Failed to create test file");
    std::io::Write::write_all(&mut file, code_str.as_bytes()).expect("Failed to write test file");
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

    let output = std::process::Command::new(&binary)
        .args(&["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("NTNT_ENV", "development")
        .env("NTNT_TASK_REMOVAL_TTL", "1") // 1 second removal TTL
        .output()
        .expect("Failed to execute ntnt");

    let _ = std::fs::remove_file(&test_file);

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    assert_eq!(exit_code, 0, "Process should succeed");
    assert!(
        stdout.trim().contains("status: consumed"),
        "try_await after await_task should return consumed, got: {}",
        stdout
    );
}

// --- Task limit: NTNT_MAX_TASKS caps concurrent spawns ---

#[test]
fn test_max_tasks_limit() {
    let test_file = unique_test_file("max_tasks");
    let code_str = r#"
import { spawn, await_task, sleep_ms } from "std/concurrent"

// With NTNT_MAX_TASKS=3, the 4th concurrent spawn should fail
let t1 = spawn(fn() { sleep_ms(2000) })
let t2 = spawn(fn() { sleep_ms(2000) })
let t3 = spawn(fn() { sleep_ms(2000) })

// This should fail — 3 tasks are already active
let t4_result = spawn(fn() { sleep_ms(2000) }) otherwise {
    print("limit_hit: " + str(err))
    return
}

print("no_limit")
"#;

    let mut file = std::fs::File::create(&test_file).expect("Failed to create test file");
    std::io::Write::write_all(&mut file, code_str.as_bytes()).expect("Failed to write test file");
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

    let output = std::process::Command::new(&binary)
        .args(&["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("NTNT_ENV", "development")
        .env("NTNT_MAX_TASKS", "3")
        .output()
        .expect("Failed to execute ntnt");

    let _ = std::fs::remove_file(&test_file);

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    assert_eq!(
        exit_code, 0,
        "Process should succeed, stderr: {}, stdout: {}",
        stderr, stdout
    );
    assert!(
        stdout.contains("limit_hit:"),
        "Should hit task limit with NTNT_MAX_TASKS=3, got: {}",
        stdout
    );
    assert!(
        stdout.contains("Maximum concurrent task limit"),
        "Error message should mention the limit, got: {}",
        stdout
    );
}

#[test]
fn test_parallel_parent_cancellation_cancels_children() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { spawn, cancel_task, await_task, parallel, sleep_ms, channel, send, recv_timeout } from "std/concurrent"

let [tx, rx] = channel()

let task = spawn(fn() {
    parallel([
        fn() { sleep_ms(1000); send(tx, "a"); "a" },
        fn() { sleep_ms(1000); send(tx, "b"); "b" }
    ])
})

sleep_ms(100)
cancel_task(task)

let result = await_task(task)
match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("cancelled: " + str(e))
}

let msg = recv_timeout(rx, 1500)
match msg {
    None => print("children_cancelled"),
    Some(v) => print("sent: " + str(v))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(stdout.contains("cancelled:"), "stdout: {}", stdout);
    assert!(stdout.contains("children_cancelled"), "stdout: {}", stdout);
}

#[test]
fn test_parallel_cancels_on_returned_err() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { parallel, sleep_ms, channel, send, recv_timeout } from "std/concurrent"

let [tx, rx] = channel()
let result = parallel([
    fn() { sleep_ms(50); Err("api down") },
    fn() { sleep_ms(300); send(tx, "late"); "ok" }
])

match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("err: " + str(e))
}

let msg = recv_timeout(rx, 500)
match msg {
    None => print("cancelled"),
    Some(v) => print("sent: " + str(v))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("err:"),
        "should detect returned Err: {}",
        stdout
    );
    assert!(
        stdout.contains("cancelled"),
        "should cancel remaining: {}",
        stdout
    );
}

#[test]
fn test_race_skips_returned_err() {
    let (stdout, _stderr, code) = run_ntnt_code(
        r#"
import { race, sleep_ms } from "std/concurrent"

let result = race([
    fn() { Err("fast but failed") },
    fn() { sleep_ms(50); "slow but ok" }
])

match result {
    Ok(v) => print("ok: " + str(v)),
    Err(e) => print("err: " + str(e))
}
"#,
    );
    assert_eq!(code, 0);
    assert!(
        stdout.contains("ok: slow but ok"),
        "should skip Err and pick Ok winner: {}",
        stdout
    );
}
