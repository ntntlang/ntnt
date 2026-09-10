#![allow(dead_code)]
use std::fs;
use std::net::SocketAddr;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
pub struct Fixture {
    pub child: Child,
    pub dir: tempfile::TempDir,
}
impl Fixture {
    pub fn start(source: &str, args: &[&str], env: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("fixture.tnt");
        fs::write(&file, source).unwrap();
        let stdout = fs::File::create(dir.path().join("stdout")).unwrap();
        let stderr = fs::File::create(dir.path().join("stderr")).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_ntnt"));
        for key in [
            "NTNT_ENV",
            "APP_ENV",
            "NTNT_SECRETS_PROVIDER",
            "NTNT_LINT_MODE",
            "NTNT_STRICT",
            "NTNT_TYPE_MODE",
            "NTNT_OOB_MODE",
            "NTNT_LISTEN_PORT",
            "NTNT_WORKERS",
        ] {
            command.env_remove(key);
        }
        command
            .env("NTNT_TYPE_MODE", "strict")
            .env("NTNT_LINT_MODE", "strict");
        command
            .args(args)
            .arg(&file)
            .envs(env.iter().copied())
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr));
        let child = command.spawn().unwrap();
        Self { child, dir }
    }
    pub fn stdout(&self) -> String {
        read_bounded(&self.dir.path().join("stdout"))
    }
    pub fn stderr(&self) -> String {
        read_bounded(&self.dir.path().join("stderr"))
    }
    pub fn wait(&mut self) -> std::process::ExitStatus {
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(
                Instant::now() < end,
                "child deadline: {} {}",
                self.stdout(),
                self.stderr()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    pub fn ready(&mut self) -> SocketAddr {
        let end = Instant::now() + Duration::from_secs(10);
        loop {
            for line in self.stdout().lines() {
                if let Some(json) = line.strip_prefix("NTNT_READY ") {
                    let v: serde_json::Value = serde_json::from_str(json).unwrap();
                    return SocketAddr::new(
                        v["host"].as_str().unwrap().parse().unwrap(),
                        v["port"].as_u64().unwrap().try_into().unwrap(),
                    );
                }
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "exited before readiness: {} {}",
                self.stdout(),
                self.stderr()
            );
            assert!(
                Instant::now() < end,
                "readiness deadline: {} {}",
                self.stdout(),
                self.stderr()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
fn read_bounded(path: &Path) -> String {
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .unwrap()
        .take(1024 * 1024)
        .read_to_end(&mut bytes)
        .unwrap();
    assert!(bytes.len() < 1024 * 1024, "fixture output exceeded cap");
    String::from_utf8(bytes).unwrap()
}
pub fn strict_run(source: &str) -> String {
    let mut lint = Fixture::start(source, &["lint", "--strict"], &[]);
    assert!(
        lint.wait().success(),
        "lint: {} {}",
        lint.stdout(),
        lint.stderr()
    );
    assert!(
        !lint.stdout().contains("warning[") && !lint.stderr().contains("warning["),
        "lint warnings: {} {}",
        lint.stdout(),
        lint.stderr()
    );
    let mut run = Fixture::start(source, &["run"], &[]);
    assert!(
        run.wait().success(),
        "run: {} {}",
        run.stdout(),
        run.stderr()
    );
    run.stdout()
}
