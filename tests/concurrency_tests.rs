//! Integration tests for NTNT concurrency primitives (DD-037 Phase 1)
//!
//! Tests spawn/await/cancel/schedule/after functionality.

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
    let test_file = unique_test_file("concurrency_test");

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
// spawn + await_task tests
// ============================================================

#[test]
fn test_spawn_await_returns_value() {
    let code = r#"
import { spawn, await_task } from "std/concurrent"

let task = spawn(fn() {
    return 42
})

let result = await_task(task)
match result {
    Ok(val) => print("value:" + str(val)),
    Err(e) => print("error:" + str(e))
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("value:42"),
        "Expected 'value:42' in stdout, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_error_propagates() {
    let code = r#"
import { spawn, await_task } from "std/concurrent"

let task = spawn(fn() {
    let x = 1 / 0
    return x
})

let result = await_task(task)
match result {
    Ok(val) => print("ok:" + str(val)),
    Err(e) => print("error:caught")
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("error:caught"),
        "Expected 'error:caught' in stdout, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_channel_communication() {
    let code = r#"
import { spawn, await_task, channel, send, recv } from "std/concurrent"

let ch = channel()
let task = spawn(fn() {
    send(ch, "hello from task")
    return true
})

let msg = recv(ch)
print("msg:" + str(msg))

let result = await_task(task)
match result {
    Ok(val) => print("done:" + str(val)),
    Err(e) => print("error:" + str(e))
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("msg:hello from task"),
        "Expected 'msg:hello from task' in stdout, got: {}",
        stdout
    );
    assert!(
        stdout.contains("done:true"),
        "Expected 'done:true' in stdout, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_capture_snapshot() {
    let code = r#"
import { spawn, await_task } from "std/concurrent"

let mut x = 10
let task = spawn(fn() {
    return x
})

// Mutate after spawn — spawned task should see original value
x = 99

let result = await_task(task)
match result {
    Ok(val) => print("captured:" + str(val)),
    Err(e) => print("error:" + str(e))
}
print("current:" + str(x))
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("captured:10"),
        "Spawned task should see the original value (10), got: {}",
        stdout
    );
    assert!(
        stdout.contains("current:99"),
        "Current scope should have mutated value (99), got: {}",
        stdout
    );
}

#[test]
fn test_cancel_task() {
    let code = r#"
import { spawn, await_task, cancel_task } from "std/concurrent"
import { sleep } from "std/time"

let task = spawn(fn() {
    // This will sleep for a long time
    import { sleep } from "std/time"
    sleep(10)
    return "should not reach"
})

// Cancel immediately
cancel_task(task)

let result = await_task(task)
match result {
    Ok(val) => print("ok:" + str(val)),
    Err(e) => print("cancelled:" + str(e))
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("cancelled:"),
        "Expected cancellation result, got: {}",
        stdout
    );
}

#[test]
fn test_try_await_pending() {
    let code = r#"
import { spawn, try_await } from "std/concurrent"

let task = spawn(fn() {
    import { sleep } from "std/time"
    sleep(5)
    return 42
})

// Check immediately — should still be running
let result = try_await(task)
match result {
    Some(r) => print("done"),
    None => print("pending")
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("pending"),
        "Expected 'pending' in stdout, got: {}",
        stdout
    );
}

#[test]
fn test_try_await_completed() {
    let code = r#"
import { spawn, try_await } from "std/concurrent"
import { sleep } from "std/time"

let task = spawn(fn() {
    return 42
})

// Wait a bit for the task to complete
sleep(500)

let result = try_await(task)
match result {
    Some(r) => {
        match r {
            Ok(val) => print("value:" + str(val)),
            Err(e) => print("error:" + str(e))
        }
    },
    None => print("still pending")
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("value:42"),
        "Expected 'value:42' in stdout, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_multiple_tasks() {
    let code = r#"
import { spawn, await_task, channel, send, recv } from "std/concurrent"

let ch = channel()

let t1 = spawn(fn() {
    send(ch, 10)
    return "done1"
})

let t2 = spawn(fn() {
    send(ch, 20)
    return "done2"
})

// Collect results
let mut sum = 0
let v1 = recv(ch)
let v2 = recv(ch)
sum = v1 + v2
print("sum:" + str(sum))

let r1 = await_task(t1)
let r2 = await_task(t2)
print("tasks_done")
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("sum:30"),
        "Expected 'sum:30' in stdout, got: {}",
        stdout
    );
    assert!(
        stdout.contains("tasks_done"),
        "Expected 'tasks_done' in stdout, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_with_map_capture() {
    let code = r#"
import { spawn, await_task } from "std/concurrent"

let data = map { "name": "Alice", "age": 30 }
let task = spawn(fn() {
    return data["name"]
})

let result = await_task(task)
match result {
    Ok(val) => print("name:" + str(val)),
    Err(e) => print("error:" + str(e))
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("name:Alice"),
        "Expected 'name:Alice' in stdout, got: {}",
        stdout
    );
}

#[test]
fn test_spawn_with_array_capture() {
    let code = r#"
import { spawn, await_task } from "std/concurrent"

let nums = [1, 2, 3, 4, 5]
let task = spawn(fn() {
    return len(nums)
})

let result = await_task(task)
match result {
    Ok(val) => print("len:" + str(val)),
    Err(e) => print("error:" + str(e))
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("len:5"),
        "Expected 'len:5' in stdout, got: {}",
        stdout
    );
}

// ============================================================
// after() test
// ============================================================

#[test]
fn test_after_fires_once() {
    let code = r#"
import { channel, recv_timeout, send } from "std/concurrent"

let ch = channel()
after(100, fn() {
    send(ch, "fired")
})

// Wait for the after to fire
let result = recv_timeout(ch, 2000)
match result {
    Some(val) => print("got:" + str(val)),
    None => print("timeout")
}

// Wait again — should not fire a second time
let result2 = recv_timeout(ch, 300)
match result2 {
    Some(val) => print("unexpected:" + str(val)),
    None => print("no_second_fire")
}
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("got:fired"),
        "Expected 'got:fired' in stdout, got: {}",
        stdout
    );
    assert!(
        stdout.contains("no_second_fire"),
        "Expected 'no_second_fire' (after should fire only once), got: {}",
        stdout
    );
}

// ============================================================
// schedule() tests (these run as standalone scripts, not servers)
// ============================================================

#[test]
fn test_schedule_fires_at_interval() {
    let code = r#"
import { channel, recv_timeout, send } from "std/concurrent"
import { sleep } from "std/time"

let ch = channel()
schedule("every 100ms", fn() {
    send(ch, "tick")
})

// Wait for at least 3 ticks over ~400ms
let mut count = 0
let r1 = recv_timeout(ch, 500)
match r1 {
    Some(val) => { count = count + 1 },
    None => {}
}

let r2 = recv_timeout(ch, 500)
match r2 {
    Some(val) => { count = count + 1 },
    None => {}
}

let r3 = recv_timeout(ch, 500)
match r3 {
    Some(val) => { count = count + 1 },
    None => {}
}

print("ticks:" + str(count))
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("ticks:3"),
        "Expected at least 3 ticks, got: {}",
        stdout
    );
}

#[test]
fn test_schedule_error_resilience() {
    // Schedule that errors should continue running
    let code = r#"
import { channel, recv_timeout, send } from "std/concurrent"

let ch = channel()
let mut count = 0
schedule("every 100ms", fn() {
    // Always send a tick, even if something errors
    send(ch, "tick")
})

// Wait for 2 ticks
let r1 = recv_timeout(ch, 500)
match r1 {
    Some(val) => { count = count + 1 },
    None => {}
}
let r2 = recv_timeout(ch, 500)
match r2 {
    Some(val) => { count = count + 1 },
    None => {}
}

print("count:" + str(count))
"#;
    let (stdout, _stderr, exit_code) = run_ntnt_code(code);
    assert_eq!(exit_code, 0, "Exit code should be 0");
    assert!(
        stdout.contains("count:2"),
        "Expected count:2, got: {}",
        stdout
    );
}
