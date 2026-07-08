//! Integration tests for std/email (DD-058 item 2)
//!
//! Uses the log transport throughout — no SMTP server is contacted.

use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn unique_test_file(prefix: &str) -> String {
    let counter = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let thread_id = format!("{:?}", std::thread::current().id());
    std::env::temp_dir()
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

fn ntnt_binary() -> std::path::PathBuf {
    let exe = std::env::consts::EXE_SUFFIX;
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let debug_path = manifest.join(format!("target/debug/ntnt{}", exe));
    let dev_release_path = manifest.join(format!("target/dev-release/ntnt{}", exe));
    let release_path = manifest.join(format!("target/release/ntnt{}", exe));

    if debug_path.exists() {
        debug_path
    } else if dev_release_path.exists() {
        dev_release_path
    } else if release_path.exists() {
        release_path
    } else {
        panic!("No ntnt binary found. Run 'cargo build' first.");
    }
}

fn run(code: &str, env_vars: &[(&str, &str)]) -> (String, String, i32) {
    let test_file = unique_test_file("email");
    let mut file = fs::File::create(&test_file).expect("create test file");
    writeln!(file, "{}", code).expect("write test file");
    drop(file);

    let mut cmd = Command::new(ntnt_binary());
    cmd.args(["run", &test_file])
        .current_dir(env!("CARGO_MANIFEST_DIR"));
    for (k, v) in env_vars {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("execute ntnt");
    let _ = fs::remove_file(&test_file);

    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.code().unwrap_or(-1),
    )
}

#[test]
fn send_email_with_log_transport() {
    let code = r#"
import { configure_email, send_email } from "std/email"

configure_email(map { "transport": "log", "from": "dev@localhost" })

match send_email(map { "to": "user@example.com", "subject": "Welcome!", "text": "Hello!" }) {
    Ok(result) => {
        print("sent via " + result["transport"])
        print(len(result["message_id"]) > 0)
    },
    Err(e) => print("unexpected err: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("sent via log"), "{stdout}");
    assert!(stdout.contains("true"), "message id present: {stdout}");
    assert!(
        stderr.contains("[EMAIL:log]") && stderr.contains("user@example.com"),
        "log transport should print the email: {stderr}"
    );
}

#[test]
fn send_email_requires_configuration() {
    let code = r#"
import { send_email } from "std/email"

let r = send_email(map { "to": "a@b.co", "subject": "Hi", "text": "x" })
    otherwise { print("caught: #{err}") return }
print("unexpected: " + str(r))
"#;
    let (stdout, _stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0);
    assert!(
        stdout.contains("configure_email() has not been called"),
        "{stdout}"
    );
}

#[test]
fn bad_options_are_err_values_not_crashes() {
    let code = r#"
import { configure_email, send_email } from "std/email"

configure_email(map { "transport": "log", "from": "dev@localhost" })

match send_email(map { "to": "not an address", "subject": "Hi", "text": "x" }) {
    Ok(_) => print("unexpected ok"),
    Err(e) => print("err: " + e)
}
match send_email(map { "to": "a@b.co", "subject": "Hi" }) {
    Ok(_) => print("unexpected ok"),
    Err(e) => print("err: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("Invalid email address"), "{stdout}");
    assert!(
        stdout.contains("\"text\" and/or \"html\""),
        "missing body message: {stdout}"
    );
}

#[test]
fn batch_reports_per_email_results() {
    let code = r#"
import { configure_email, send_email_batch } from "std/email"

configure_email(map { "transport": "log", "from": "dev@localhost" })

match send_email_batch([
    map { "to": "a@b.co", "subject": "One", "text": "x" },
    map { "to": "broken", "subject": "Two", "text": "x" },
    map { "to": "c@d.co", "subject": "Three", "text": "x" }
]) {
    Ok(results) => {
        print("count: " + str(len(results)))
        match results[0] { Ok(_) => print("0 ok"), Err(_) => print("0 err") }
        match results[1] { Ok(_) => print("1 ok"), Err(_) => print("1 err") }
        match results[2] { Ok(_) => print("2 ok"), Err(_) => print("2 err") }
    },
    Err(e) => print("unexpected: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run(code, &[]);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("count: 3"), "{stdout}");
    assert!(stdout.contains("0 ok"), "{stdout}");
    assert!(
        stdout.contains("1 err"),
        "one bad email doesn't stop the rest: {stdout}"
    );
    assert!(stdout.contains("2 ok"), "{stdout}");
}

#[test]
fn env_var_forces_log_mode_over_smtp_config() {
    // SMTP config pointing nowhere, but NTNT_EMAIL_MODE=log short-circuits
    // before any connection attempt
    let code = r#"
import { configure_email, send_email } from "std/email"

configure_email(map { "host": "smtp.invalid.example", "from": "a@b.co" })

match send_email(map { "to": "user@example.com", "subject": "Hi", "text": "x" }) {
    Ok(result) => print("via " + result["transport"]),
    Err(e) => print("unexpected err: " + e)
}
"#;
    let (stdout, stderr, exit_code) = run(code, &[("NTNT_EMAIL_MODE", "log")]);
    assert_eq!(exit_code, 0, "stderr: {stderr}");
    assert!(stdout.contains("via log"), "{stdout}");
}
