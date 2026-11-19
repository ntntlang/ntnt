//! Host runtime policy (process execution, fetch SSRF policy) belongs to the
//! environment that launches ntnt. Application code must not be able to grant
//! itself capabilities by writing a dotenv file and loading it, and a dotenv
//! file must not replace any variable the host set (such as `NTNT_ENV`).
//!
//! Every case runs the real CLI with an explicit environment; this test process
//! never mutates its own environment.
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Hang guard for one CLI child. It never decides an outcome: every case
/// finishes as soon as the child exits.
const CLI_EXIT_HANG_GUARD: Duration = Duration::from_secs(120);

struct CliOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn literal(text: &str) -> String {
    serde_json::to_string(text).unwrap()
}

/// `ntnt run <script>` with no inherited NTNT_* policy, app mode or proxy
/// settings; each case adds the host policy it needs.
fn ntnt_run(script: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
    command.arg("run").arg(script).stdin(Stdio::null());
    for (key, _) in std::env::vars_os() {
        let name = key.to_string_lossy();
        if name.to_ascii_uppercase().starts_with("NTNT_")
            || name == "APP_ENV"
            || name.to_ascii_lowercase().ends_with("_proxy")
        {
            command.env_remove(&key);
        }
    }
    command
}

fn run_cli(mut command: Command, directory: &Path) -> CliOutput {
    let stdout_path = directory.join("cli-stdout.log");
    let stderr_path = directory.join("cli-stderr.log");
    command
        .stdout(File::create(&stdout_path).unwrap())
        .stderr(File::create(&stderr_path).unwrap());
    let mut child = command.spawn().expect("spawn ntnt");
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("poll ntnt") {
            break status;
        }
        if started.elapsed() >= CLI_EXIT_HANG_GUARD {
            let _ = child.kill();
            let _ = child.wait();
            panic!("CLI_EXIT_HANG_GUARD expired: ntnt did not exit within 120 s");
        }
        thread::sleep(Duration::from_millis(10));
    };
    CliOutput {
        status,
        stdout: std::fs::read_to_string(stdout_path).unwrap(),
        stderr: std::fs::read_to_string(stderr_path).unwrap(),
    }
}

/// Lines in which load_env reports that it skipped `key`.
fn skip_warnings<'a>(stderr: &'a str, key: &str) -> Vec<&'a str> {
    stderr
        .lines()
        .filter(|line| line.contains("load_env") && line.contains(key))
        .collect()
}

/// Loopback HTTP server that counts contacts and answers each with a marker body.
struct HttpFixture {
    port: u16,
    stop: Arc<AtomicBool>,
    contacts: Arc<AtomicUsize>,
    worker: Option<JoinHandle<()>>,
}

impl HttpFixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let contacts = Arc::new(AtomicUsize::new(0));
        let (stopped, accepted) = (stop.clone(), contacts.clone());
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        accepted.fetch_add(1, Ordering::AcqRel);
                        socket.set_nonblocking(false).unwrap();
                        // Hang guard only; a client always sends its request at once.
                        socket
                            .set_read_timeout(Some(Duration::from_secs(60)))
                            .unwrap();
                        let mut request = Vec::new();
                        let mut buffer = [0; 1024];
                        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                            match socket.read(&mut buffer) {
                                Ok(0) | Err(_) => break,
                                Ok(read) => request.extend_from_slice(&buffer[..read]),
                            }
                        }
                        let _ = socket.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\nfixture-body!",
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("HTTP fixture accept: {error}"),
                }
            }
        });
        Self {
            port,
            stop,
            contacts,
            worker: Some(worker),
        }
    }

    /// Stops the server after the client process has exited and returns how
    /// many connections it accepted.
    fn finish(mut self) -> usize {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        self.contacts.load(Ordering::Acquire)
    }
}

/// NTNT source that writes `dotenv` contents, loads it and fetches the fixture.
fn fetch_after_load_env_source(dotenv: &Path, contents: &str, port: u16) -> String {
    format!(
        r#"
import {{ write_file }} from "std/fs"
import {{ load_env, get_env }} from "std/env"
import {{ fetch }} from "std/http"

let dotenv = {dotenv}
match write_file(dotenv, {contents}) {{
    Ok(_) => print("write=ok"),
    Err(error) => print("write=err:" + error)
}}
match load_env(dotenv) {{
    Ok(_) => print("load_env=ok"),
    Err(error) => print("load_env=err:" + error)
}}
print("ordinary=" + (get_env("HOST_POLICY_ORDINARY") ?? "missing"))
print("mode=" + (get_env("NTNT_ENV") ?? "unset"))
match fetch("http://127.0.0.1:{port}/policy") {{
    Ok(response) => print("fetch=reached:" + response["body"]),
    Err(error) => print("fetch=denied:" + error)
}}
"#,
        dotenv = literal(&dotenv.to_string_lossy()),
        contents = literal(contents),
    )
}

#[test]
fn load_env_cannot_enable_process_execution() {
    let directory = tempfile::tempdir().unwrap();
    let dotenv = directory.path().join("app.env");
    let executable = env!("CARGO_BIN_EXE_ntnt");
    let source = format!(
        r#"
import {{ write_file }} from "std/fs"
import {{ load_env, get_env }} from "std/env"
import {{ run }} from "std/process"

let dotenv = {dotenv}
match write_file(dotenv, "NTNT_PROCESS_ENABLE=1\nNTNT_PROCESS_ALLOW=" + {executable} + "\nHOST_POLICY_ORDINARY=loaded\n") {{
    Ok(_) => print("write=ok"),
    Err(error) => print("write=err:" + error)
}}
match load_env(dotenv) {{
    Ok(_) => print("load_env=ok"),
    Err(error) => print("load_env=err:" + error)
}}
print("ordinary=" + (get_env("HOST_POLICY_ORDINARY") ?? "missing"))
match run({executable}, ["--version"]) {{
    Ok(_) => print("process=ran"),
    Err(error) => print("process=denied:" + error)
}}
"#,
        dotenv = literal(&dotenv.to_string_lossy()),
        executable = literal(executable),
    );
    let script = directory.path().join("self_authorize.tnt");
    std::fs::write(&script, source).unwrap();

    let output = run_cli(ntnt_run(&script), directory.path());

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    for expected in ["write=ok", "load_env=ok", "ordinary=loaded"] {
        assert!(
            output.stdout.contains(expected),
            "missing {expected}: stdout={}",
            output.stdout
        );
    }
    assert!(
        output
            .stdout
            .contains("process=denied:process execution is disabled"),
        "app code authorized its own process launch: stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    for key in ["NTNT_PROCESS_ENABLE", "NTNT_PROCESS_ALLOW"] {
        assert_eq!(
            skip_warnings(&output.stderr, key).len(),
            1,
            "expected one load_env warning for {key}: stderr={}",
            output.stderr
        );
    }
}

#[test]
fn load_env_cannot_relax_fetch_ssrf_policy() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = HttpFixture::new();
    let script = directory.path().join("relax_ssrf.tnt");
    let contents = "NTNT_SSRF_PROTECTION=false\nNTNT_ALLOW_LOCALHOST=true\nNTNT_ALLOW_PRIVATE_IPS=true\nNTNT_BLOCKED_HOSTS=\nHOST_POLICY_ORDINARY=loaded\n";
    std::fs::write(
        &script,
        fetch_after_load_env_source(&directory.path().join("app.env"), contents, fixture.port),
    )
    .unwrap();
    let mut command = ntnt_run(&script);
    // The host explicitly blocks loopback fetches.
    command
        .env("NTNT_ENV", "development")
        .env("NTNT_ALLOW_LOCALHOST", "false");

    let output = run_cli(command, directory.path());
    let contacts = fixture.finish();

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert!(
        output.stdout.contains("ordinary=loaded"),
        "stdout={}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .contains("fetch=denied:SSRF protection: Localhost requests blocked"),
        "app code relaxed the host SSRF policy: stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert_eq!(contacts, 0, "the loopback fixture was contacted");
    for key in [
        "NTNT_SSRF_PROTECTION",
        "NTNT_ALLOW_LOCALHOST",
        "NTNT_ALLOW_PRIVATE_IPS",
        "NTNT_BLOCKED_HOSTS",
    ] {
        assert_eq!(
            skip_warnings(&output.stderr, key).len(),
            1,
            "expected one load_env warning for {key}: stderr={}",
            output.stderr
        );
    }
}

#[test]
fn load_env_cannot_leave_production_fetch_policy() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = HttpFixture::new();
    let script = directory.path().join("leave_production.tnt");
    std::fs::write(
        &script,
        fetch_after_load_env_source(
            &directory.path().join("app.env"),
            "NTNT_ENV=development\n",
            fixture.port,
        ),
    )
    .unwrap();
    let mut command = ntnt_run(&script);
    // Production hosts block loopback fetches unless NTNT_ALLOW_LOCALHOST says otherwise.
    command.env("NTNT_ENV", "production");

    let output = run_cli(command, directory.path());
    let contacts = fixture.finish();

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    // The host set NTNT_ENV, so the dotenv value does not replace it.
    assert!(
        output.stdout.contains("mode=production"),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert_eq!(
        skip_warnings(&output.stderr, "NTNT_ENV").len(),
        1,
        "expected one load_env warning for NTNT_ENV: stderr={}",
        output.stderr
    );
    assert!(
        output
            .stdout
            .contains("fetch=denied:SSRF protection: Localhost requests blocked"),
        "app code relaxed the production SSRF default: stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert_eq!(contacts, 0, "the loopback fixture was contacted");
}

#[test]
fn load_env_production_mode_still_tightens_fetch_policy() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = HttpFixture::new();
    let script = directory.path().join("enter_production.tnt");
    std::fs::write(
        &script,
        fetch_after_load_env_source(
            &directory.path().join("app.env"),
            "NTNT_ENV=production\n",
            fixture.port,
        ),
    )
    .unwrap();
    // The host leaves NTNT_ENV and NTNT_ALLOW_LOCALHOST unset (development default).
    let output = run_cli(ntnt_run(&script), directory.path());
    let contacts = fixture.finish();

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert!(
        output.stdout.contains("mode=production"),
        "stdout={}",
        output.stdout
    );
    assert!(
        output
            .stdout
            .contains("fetch=denied:SSRF protection: Localhost requests blocked"),
        "an application-selected production mode no longer blocks loopback fetches: stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert_eq!(contacts, 0, "the loopback fixture was contacted");
}

/// Every line in which load_env reports something.
fn load_env_warnings(stderr: &str) -> Vec<&str> {
    stderr
        .lines()
        .filter(|line| line.contains("load_env"))
        .collect()
}

/// NTNT source that loads `.env` then `.env.local` from its working directory
/// and prints each of `keys` as `KEY=value`.
fn load_dotenv_files_source(keys: &[&str]) -> String {
    let mut source = String::from(
        r#"
import { load_env, get_env } from "std/env"

match load_env(".env") {
    Ok(_) => print("load .env=ok"),
    Err(error) => print("load .env=err:" + error)
}
match load_env(".env.local") {
    Ok(_) => print("load .env.local=ok"),
    Err(error) => print("load .env.local=err:" + error)
}
"#,
    );
    for key in keys {
        source.push_str(&format!(
            "print({label} + (get_env({key}) ?? \"unset\"))\n",
            label = literal(&format!("{key}=")),
            key = literal(key),
        ));
    }
    source
}

#[test]
fn load_env_keeps_variables_the_host_set() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(".env"),
        "NTNT_ENV=development\nAPP_SETTING=dotenv-app-value\nNEW_SETTING=dotenv-new-value\n",
    )
    .unwrap();
    std::fs::write(
        directory.path().join(".env.local"),
        "NEW_SETTING=local-new-value\nAPP_SETTING=local-app-value\n",
    )
    .unwrap();
    let script = directory.path().join("host_wins.tnt");
    std::fs::write(
        &script,
        load_dotenv_files_source(&["NTNT_ENV", "APP_SETTING", "NEW_SETTING"]),
    )
    .unwrap();
    let mut command = ntnt_run(&script);
    command
        .current_dir(directory.path())
        .env("NTNT_ENV", "production")
        .env("APP_SETTING", "host-app-value")
        .env_remove("NEW_SETTING");

    let output = run_cli(command, directory.path());

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert_eq!(
        output.stdout.lines().collect::<Vec<_>>(),
        [
            "load .env=ok",
            "load .env.local=ok",
            "NTNT_ENV=production",
            "APP_SETTING=host-app-value",
            // The host did not set it, so the later file replaces the earlier one.
            "NEW_SETTING=local-new-value",
        ],
        "a dotenv file replaced a host variable: stderr={}",
        output.stderr
    );
    // One warning per kept key, although APP_SETTING is in both files.
    for key in ["NTNT_ENV", "APP_SETTING"] {
        assert_eq!(
            skip_warnings(&output.stderr, key).len(),
            1,
            "expected one load_env warning for {key}: stderr={}",
            output.stderr
        );
    }
    assert!(
        skip_warnings(&output.stderr, "NEW_SETTING").is_empty(),
        "stderr={}",
        output.stderr
    );
    let warnings = load_env_warnings(&output.stderr);
    for value in [
        "production",
        "development",
        "host-app-value",
        "dotenv-app-value",
        "dotenv-new-value",
        "local-app-value",
        "local-new-value",
    ] {
        assert!(
            warnings.iter().all(|line| !line.contains(value)),
            "a load_env warning printed the value {value:?}: {warnings:#?}"
        );
    }
}

/// Windows environment names are case-insensitive, so `MIXED_CASE_SETTING`
/// in a dotenv file names the host's `Mixed_Case_Setting` there; on other
/// platforms they are two variables and the dotenv one loads.
#[test]
fn load_env_matches_host_names_by_platform_case_rules() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join(".env"),
        "MIXED_CASE_SETTING=dotenv-mixed-value\n",
    )
    .unwrap();
    std::fs::write(directory.path().join(".env.local"), "").unwrap();
    let script = directory.path().join("case_rules.tnt");
    std::fs::write(
        &script,
        load_dotenv_files_source(&["MIXED_CASE_SETTING", "Mixed_Case_Setting"]),
    )
    .unwrap();
    let mut command = ntnt_run(&script);
    command
        .current_dir(directory.path())
        .env_remove("MIXED_CASE_SETTING")
        .env("Mixed_Case_Setting", "host-mixed-value");

    let output = run_cli(command, directory.path());

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    let (loaded, warnings) = if cfg!(windows) {
        ("host-mixed-value", 1)
    } else {
        ("dotenv-mixed-value", 0)
    };
    assert_eq!(
        output.stdout.lines().collect::<Vec<_>>(),
        [
            "load .env=ok".to_string(),
            "load .env.local=ok".to_string(),
            format!("MIXED_CASE_SETTING={loaded}"),
            "Mixed_Case_Setting=host-mixed-value".to_string(),
        ],
        "stderr={}",
        output.stderr
    );
    assert_eq!(
        skip_warnings(&output.stderr, "MIXED_CASE_SETTING").len(),
        warnings,
        "stderr={}",
        output.stderr
    );
}

/// A development dotenv file deployed with the app must not turn off a
/// production-only protection: `enable_auth` without a `session_secret`
/// still refuses to start when the host runs in production.
#[test]
fn development_dotenv_keeps_production_session_secret_check() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(directory.path().join(".env"), "NTNT_ENV=development\n").unwrap();
    let script = directory.path().join("auth_without_secret.tnt");
    std::fs::write(
        &script,
        r#"
import { load_env, get_env } from "std/env"
import { enable_auth } from "std/auth"

match load_env(".env") {
    Ok(_) => print("load .env=ok"),
    Err(error) => print("load .env=err:" + error)
}
print("NTNT_ENV=" + (get_env("NTNT_ENV") ?? "unset"))
enable_auth([], map { "session_store": "memory" })
print("auth=enabled")
"#,
    )
    .unwrap();
    let mut command = ntnt_run(&script);
    command
        .current_dir(directory.path())
        .env("NTNT_ENV", "production");

    let output = run_cli(command, directory.path());

    assert_eq!(
        output.stdout.lines().collect::<Vec<_>>(),
        ["load .env=ok", "NTNT_ENV=production"],
        "auth started without a session secret: stderr={}",
        output.stderr
    );
    assert_eq!(
        output.status.code(),
        Some(1),
        "stdout={}\nstderr={}",
        output.stdout,
        output.stderr
    );
    assert!(
        output
            .stderr
            .contains("FATAL: Cannot use default session_secret in production!"),
        "stderr={}",
        output.stderr
    );
}
