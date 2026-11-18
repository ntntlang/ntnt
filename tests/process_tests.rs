use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

/// Hang guard for fixture processes; only a bug reaches it.
const FIXTURE_HANG_GUARD: Duration = Duration::from_secs(120);

/// A private directory with an unpredictable name, removed on drop. Fixture
/// files inside it cannot be planted or replaced by another local user.
fn private_fixture_dir(prefix: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(prefix)
        .tempdir()
        .expect("create private fixture directory")
}

fn run_ntnt(code: &str, allowlist: &str) -> (String, String, i32) {
    let directory = private_fixture_dir("ntnt-process-language-");
    let source = directory.path().join("main.tnt");
    fs::write(&source, code).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .arg("run")
        .arg(&source)
        .env("NTNT_PROCESS_ENABLE", "1")
        .env("NTNT_PROCESS_ALLOW", allowlist)
        .output()
        .expect("run NTNT process fixture");
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        output.status.code().unwrap_or(-1),
    )
}

fn program_literal(path: &str) -> String {
    serde_json::to_string(path).unwrap()
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if result == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

#[test]
fn process_run_and_lifecycle_work_through_language_api() {
    let fixture = std::env::current_exe().unwrap();
    let fixture_text = fixture.to_string_lossy();
    let program = program_literal(&fixture_text);
    let code = format!(
        r#"
import {{ run, start, try_wait, wait, terminate }} from "std/process"

match run({program}, ["--exact", "process_fixture_print_args", "--ignored", "--nocapture", "--", "semi;colon", "$(literal)"]) {{
    Ok(result) => {{
        print(result["success"])
        print(result["stdout"])
    }},
    Err(error) => print("run-error:" + error)
}}

match start({program}, ["--exact", "process_fixture_sleep", "--ignored", "--nocapture"], map {{
    "stdout": map {{ "mode": "capture" }},
    "stderr": map {{ "mode": "capture" }},
    "termination_grace_ms": 20
}}) {{
    Ok(child) => {{
        match try_wait(child) {{
            Ok(None) => print("pending"),
            Ok(Some(_)) => print("finished-early"),
            Err(error) => print("try-error:" + error)
        }}
        match terminate(child) {{
            Ok(value) => print(value),
            Err(error) => print("terminate-error:" + error)
        }}
        match wait(child) {{
            Ok(result) => print(result["success"]),
            Err(error) => print("wait-error:" + error)
        }}
    }},
    Err(error) => print("start-error:" + error)
}}
"#
    );

    let (stdout, stderr, exit) = run_ntnt(&code, &fixture_text);
    assert_eq!(exit, 0, "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains("semi;colon"), "stdout={stdout}");
    assert!(stdout.contains("$(literal)"), "stdout={stdout}");
    assert!(
        stdout.lines().any(|line| line == "pending"),
        "stdout={stdout}"
    );
    assert!(stdout.lines().any(|line| line == "true"), "stdout={stdout}");
    assert!(!stdout.contains("-error:"), "stdout={stdout}");
}

#[test]
fn process_language_api_rejects_executable_outside_allowlist() {
    let fixture = std::env::current_exe().unwrap();
    let fixture_text = fixture.to_string_lossy();
    let ntnt = program_literal(env!("CARGO_BIN_EXE_ntnt"));
    let code = format!(
        r#"
import {{ run }} from "std/process"
match run({ntnt}, ["--version"]) {{
    Ok(_) => print("unexpected"),
    Err(error) => print(error)
}}
"#
    );
    let (stdout, stderr, exit) = run_ntnt(&code, &fixture_text);
    assert_eq!(exit, 0, "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains("is not allowed"), "stdout={stdout}");
}

#[cfg(unix)]
#[test]
fn direct_exit_commands_shutdown_started_processes() {
    let fixture = std::env::current_exe().unwrap();
    let fixture_text = fixture.to_string_lossy();
    let program = program_literal(&fixture_text);
    // Kept until the end of the test: the program, the PID marker the test
    // signals and the job store all live in a private directory.
    let directory = private_fixture_dir("ntnt-process-direct-exit-");
    let source = directory.path().join("server.tnt");
    let marker = directory.path().join("child.pid");
    let store = directory.path().join("jobs.db");
    let code = format!(
        r#"
import {{ run, start }} from "std/process"
import {{ configure_queue }} from "std/jobs"

match start({program}, ["--exact", "process_fixture_write_pid_and_sleep", "--ignored", "--nocapture", "--", {marker}], map {{
    "stdout": map {{ "mode": "null" }},
    "stderr": map {{ "mode": "null" }}
}}) {{
    Ok(_) => {{}},
    Err(error) => print(error)
}}
let marker_written = run({program}, ["--exact", "process_fixture_wait_for_file", "--ignored", "--nocapture", "--", {marker}])
configure_queue(map {{ "store": {store} }})
"#,
        marker = program_literal(&marker.to_string_lossy()),
        store = program_literal(&format!("sqlite:{}", store.to_string_lossy())),
    );
    fs::write(&source, code).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["jobs", "inspect"])
        .arg(&source)
        .arg("definitely-missing")
        .env("NTNT_PROCESS_ENABLE", "1")
        .env("NTNT_PROCESS_ALLOW", &*fixture_text)
        .output()
        .expect("run jobs inspect fixture");

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not found"),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let pid: u32 = fs::read_to_string(&marker)
        .expect("started fixture wrote pid")
        .parse()
        .expect("valid fixture pid");
    let survived = process_exists(pid);
    if survived {
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
    assert!(!survived, "child {pid} survived CLI exit");
}

#[test]
#[ignore]
fn process_fixture_print_args() {
    for argument in std::env::args()
        .skip_while(|argument| argument != "--")
        .skip(1)
    {
        println!("{argument}");
    }
}

#[test]
#[ignore]
fn process_fixture_sleep() {
    std::thread::sleep(std::time::Duration::from_secs(10));
}

/// Exits once the file named after `--` exists, so a program can wait for a
/// started fixture instead of sleeping.
#[test]
#[ignore]
fn process_fixture_wait_for_file() {
    let path = std::env::args()
        .skip_while(|argument| argument != "--")
        .nth(1)
        .expect("path argument");
    let deadline = Instant::now() + FIXTURE_HANG_GUARD;
    while !std::path::Path::new(&path).exists() {
        assert!(Instant::now() < deadline, "{path} never appeared");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Writes its PID to the marker named after `--`, then runs until killed.
#[test]
#[ignore]
fn process_fixture_write_pid_and_sleep() {
    let marker = std::env::args()
        .skip_while(|argument| argument != "--")
        .nth(1)
        .expect("pid marker argument");
    // Rename into place so a waiter never sees a partly written marker.
    let partial = format!("{marker}.partial");
    fs::write(&partial, std::process::id().to_string()).unwrap();
    fs::rename(&partial, &marker).unwrap();
    std::thread::sleep(FIXTURE_HANG_GUARD);
}
