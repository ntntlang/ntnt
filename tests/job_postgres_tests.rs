//! Integration tests for NTNT Job DSL PostgreSQL Backend (DD-037 Phase 3)
//!
//! These tests require a running PostgreSQL instance.
//! Set NTNT_TEST_DATABASE_URL to run them:
//!   NTNT_TEST_DATABASE_URL=postgres://user:pass@localhost/ntnt_test cargo test --test job_postgres_tests
//!
//! All tests are #[ignore] by default — they only run when explicitly requested
//! with `cargo test --test job_postgres_tests -- --ignored` or when the env var is set.

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
            "ntnt_pg_{}_{}_{}_{}.tnt",
            prefix,
            std::process::id(),
            thread_id.replace(|c: char| !c.is_alphanumeric(), "_"),
            counter
        ))
        .to_string_lossy()
        .to_string()
}

/// Helper to run ntnt with a code string and DATABASE_URL set
fn run_ntnt_code_pg(code: &str) -> (String, String, i32) {
    let db_url = match std::env::var("NTNT_TEST_DATABASE_URL") {
        Ok(url) => url,
        Err(_) => {
            // Return a marker result so tests can skip gracefully
            return ("SKIP_NO_DB".to_string(), String::new(), 0);
        }
    };

    let test_file = unique_test_file("pg_job_test");

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
        .env("DATABASE_URL", &db_url)
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

/// Check if test DB is available
fn has_test_db() -> bool {
    std::env::var("NTNT_TEST_DATABASE_URL").is_ok()
}

#[test]
#[ignore]
fn test_postgres_configure_backend() {
    if !has_test_db() {
        eprintln!("Skipping: NTNT_TEST_DATABASE_URL not set");
        return;
    }

    let db_url = std::env::var("NTNT_TEST_DATABASE_URL").unwrap();
    let code = format!(
        r#"
import {{ Queue }} from "std/jobs"

Queue.configure(map {{
    "backend": "postgres",
    "url": "{}"
}})

print("Postgres backend configured")
"#,
        db_url
    );

    let (stdout, stderr, exit_code) = run_ntnt_code_pg(&code);
    if stdout == "SKIP_NO_DB" {
        return;
    }
    assert_eq!(
        exit_code, 0,
        "Postgres configure should succeed. stderr: {}",
        stderr
    );
    assert!(
        stdout.contains("Postgres backend configured"),
        "Should configure postgres backend. stdout: {}",
        stdout
    );
}

#[test]
#[ignore]
fn test_postgres_job_enqueue_and_process() {
    if !has_test_db() {
        eprintln!("Skipping: NTNT_TEST_DATABASE_URL not set");
        return;
    }

    let db_url = std::env::var("NTNT_TEST_DATABASE_URL").unwrap();
    let code = format!(
        r#"
import {{ Queue }} from "std/jobs"
import {{ sleep_ms }} from "std/concurrent"

Queue.configure(map {{
    "backend": "postgres",
    "url": "{}"
}})

Job PgGreet on default {{
    perform(name: String) {{
        print("PG Hello, #{{name}}!")
    }}
}}

PgGreet.enqueue(map {{ "name": "World" }})
Queue.work_async()
sleep_ms(1000)

let status = Queue.stats()
print("Completed: #{{status.completed}}")
"#,
        db_url
    );

    let (stdout, stderr, exit_code) = run_ntnt_code_pg(&code);
    if stdout == "SKIP_NO_DB" {
        return;
    }
    assert_eq!(
        exit_code, 0,
        "Postgres enqueue/process should succeed. stderr: {}",
        stderr
    );
    assert!(
        stdout.contains("PG Hello, World!"),
        "Job should execute. stdout: {}",
        stdout
    );
}

#[test]
#[ignore]
fn test_postgres_stats_and_recent() {
    if !has_test_db() {
        eprintln!("Skipping: NTNT_TEST_DATABASE_URL not set");
        return;
    }

    let db_url = std::env::var("NTNT_TEST_DATABASE_URL").unwrap();
    let code = format!(
        r#"
import {{ Queue }} from "std/jobs"
import {{ sleep_ms }} from "std/concurrent"

Queue.configure(map {{
    "backend": "postgres",
    "url": "{}"
}})

Job StatsJob on default {{
    perform(n: Int) {{
        print("Stats job #{{n}}")
    }}
}}

StatsJob.enqueue(map {{ "n": 1 }})
StatsJob.enqueue(map {{ "n": 2 }})

let status = Queue.stats()
print("Pending: #{{status.pending}}")

let recent = Queue.recent(5)
print("Recent count: #{{len(recent)}}")
"#,
        db_url
    );

    let (stdout, stderr, exit_code) = run_ntnt_code_pg(&code);
    if stdout == "SKIP_NO_DB" {
        return;
    }
    assert_eq!(
        exit_code, 0,
        "Stats/recent should succeed. stderr: {}",
        stderr
    );
    // At least 2 pending (could be more from previous test runs)
    assert!(
        stdout.contains("Recent count:"),
        "Should show recent jobs. stdout: {}",
        stdout
    );
}

#[test]
#[ignore]
fn test_postgres_job_persists() {
    if !has_test_db() {
        eprintln!("Skipping: NTNT_TEST_DATABASE_URL not set");
        return;
    }

    let db_url = std::env::var("NTNT_TEST_DATABASE_URL").unwrap();

    // First, enqueue a job
    let code1 = format!(
        r#"
import {{ Queue }} from "std/jobs"

Queue.configure(map {{
    "backend": "postgres",
    "url": "{}"
}})

Job PersistJob on default {{
    perform(msg: String) {{
        print("Persisted: #{{msg}}")
    }}
}}

let job_id = PersistJob.enqueue(map {{ "msg": "persistent" }})
print("Enqueued: #{{job_id}}")
"#,
        db_url
    );

    let (stdout1, stderr1, code1_exit) = run_ntnt_code_pg(&code1);
    if stdout1 == "SKIP_NO_DB" {
        return;
    }
    assert_eq!(code1_exit, 0, "Enqueue should succeed. stderr: {}", stderr1);
    assert!(
        stdout1.contains("Enqueued:"),
        "Should return job ID. stdout: {}",
        stdout1
    );

    // Second, verify the job exists by checking stats
    let code2 = format!(
        r#"
import {{ Queue }} from "std/jobs"

Queue.configure(map {{
    "backend": "postgres",
    "url": "{}"
}})

Job PersistJob on default {{
    perform(msg: String) {{
        print("Persisted: #{{msg}}")
    }}
}}

let status = Queue.stats()
// There should be at least 1 pending job from the previous run
let pending = status.pending
if pending > 0 {{
    print("Jobs persisted across runs")
}} otherwise {{
    return print("No pending jobs found")
}}
"#,
        db_url
    );

    let (stdout2, stderr2, code2_exit) = run_ntnt_code_pg(&code2);
    assert_eq!(
        code2_exit, 0,
        "Stats check should succeed. stderr: {}",
        stderr2
    );
    assert!(
        stdout2.contains("Jobs persisted across runs"),
        "Jobs should persist across process restarts. stdout: {}",
        stdout2
    );
}

#[test]
#[ignore]
fn test_postgres_dead_and_retry() {
    if !has_test_db() {
        eprintln!("Skipping: NTNT_TEST_DATABASE_URL not set");
        return;
    }

    let db_url = std::env::var("NTNT_TEST_DATABASE_URL").unwrap();
    let code = format!(
        r#"
import {{ Queue }} from "std/jobs"
import {{ sleep_ms }} from "std/concurrent"

Queue.configure(map {{
    "backend": "postgres",
    "url": "{}"
}})

Job FailingPg on default (retry: 1, backoff: 50) {{
    perform(x: Int) {{
        let result = 1 / 0
    }}
}}

FailingPg.enqueue(map {{ "x": 1 }})
Queue.work_async()
sleep_ms(3000)

let dead = Queue.dead(5)
print("Dead count: #{{len(dead)}}")
"#,
        db_url
    );

    let (stdout, stderr, exit_code) = run_ntnt_code_pg(&code);
    if stdout == "SKIP_NO_DB" {
        return;
    }
    assert_eq!(
        exit_code, 0,
        "Dead/retry should succeed. stderr: {}",
        stderr
    );
}
