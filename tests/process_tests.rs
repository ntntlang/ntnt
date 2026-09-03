use std::fs;
use std::io::Write;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);

fn run_ntnt(code: &str, allowlist: &str) -> (String, String, i32) {
    let source = std::env::temp_dir().join(format!(
        "ntnt-process-language-{}-{}.tnt",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = fs::File::create(&source).unwrap();
    file.write_all(code.as_bytes()).unwrap();
    drop(file);
    let output = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .arg("run")
        .arg(&source)
        .env("NTNT_PROCESS_ENABLE", "1")
        .env("NTNT_PROCESS_ALLOW", allowlist)
        .output()
        .expect("run NTNT process fixture");
    fs::remove_file(source).ok();
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
    let directory = std::env::temp_dir().join(format!(
        "ntnt-process-direct-exit-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("server.tnt");
    let marker = directory.join("child.pid");
    let store = directory.join("jobs.db");
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
let pause = run({program}, ["--exact", "process_fixture_sleep_short", "--ignored", "--nocapture"])
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
    fs::remove_dir_all(directory).unwrap();
    assert!(!survived, "child {pid} survived CLI exit");
}

#[cfg(unix)]
#[test]
fn sigint_shutdowns_started_processes() {
    let fixture = std::env::current_exe().unwrap();
    let fixture_text = fixture.to_string_lossy();
    let program = program_literal(&fixture_text);
    let directory = std::env::temp_dir().join(format!(
        "ntnt-process-sigint-{}-{}",
        std::process::id(),
        TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("server.tnt");
    let marker = directory.join("child.pid");
    let code = format!(
        r#"
import {{ start, wait }} from "std/process"

match start({program}, ["--exact", "process_fixture_write_pid_and_sleep", "--ignored", "--nocapture", "--", {marker}], map {{
    "stdout": map {{ "mode": "null" }},
    "stderr": map {{ "mode": "null" }}
}}) {{
    Ok(child) => wait(child),
    Err(error) => print(error)
}}
"#,
        marker = program_literal(&marker.to_string_lossy()),
    );
    fs::write(&source, code).unwrap();

    let mut ntnt = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .arg("run")
        .arg(&source)
        .env("NTNT_PROCESS_ENABLE", "1")
        .env("NTNT_PROCESS_ALLOW", &*fixture_text)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("run NTNT SIGINT fixture");

    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let pid: u32 = fs::read_to_string(&marker)
        .expect("started fixture wrote pid")
        .parse()
        .expect("valid fixture pid");
    unsafe {
        libc::kill(ntnt.id() as libc::pid_t, libc::SIGINT);
    }
    for _ in 0..100 {
        if ntnt.try_wait().unwrap().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if ntnt.try_wait().unwrap().is_none() {
        ntnt.kill().ok();
        ntnt.wait().ok();
        panic!("NTNT did not exit after SIGINT");
    }

    let survived = process_exists(pid);
    if survived {
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
    }
    fs::remove_dir_all(directory).unwrap();
    assert!(!survived, "child {pid} survived NTNT SIGINT");
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

#[test]
#[ignore]
fn process_fixture_sleep_short() {
    std::thread::sleep(std::time::Duration::from_millis(100));
}

#[test]
#[ignore]
fn process_fixture_write_pid_and_sleep() {
    let marker = std::env::args()
        .skip_while(|argument| argument != "--")
        .nth(1)
        .expect("pid marker argument");
    fs::write(marker, std::process::id().to_string()).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(10));
}
