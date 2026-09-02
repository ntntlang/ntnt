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
