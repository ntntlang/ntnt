#![allow(dead_code)]
use rusqlite::Connection;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

pub struct Fixture {
    pub root: tempfile::TempDir,
    pub target_path: PathBuf,
    pub scratch_path: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        // macOS temp roots commonly pass through /var -> /private/var. The
        // endpoint contract intentionally rejects aliases; give it native paths.
        let native_root = root.path().canonicalize().unwrap();
        let target_path = native_root.join("deployment.sqlite");
        let scratch_path = native_root.join("scratch.sqlite");
        Connection::open(&target_path).unwrap();
        Connection::open(&scratch_path).unwrap();
        std::fs::create_dir(root.path().join("migrations")).unwrap();
        std::fs::write(
            root.path().join("ntnt.toml"),
            r#"
[databases.app]
engine = "sqlite"
schema = "main"
migrations_dir = "migrations"
approval_secret = "DB_APPROVAL"
[databases.app.targets.test]
connection_secret = "DB_TARGET"
[databases.app.scratch]
connection_secret = "DB_SCRATCH"
disposable = true
[secrets.DB_TARGET]
[secrets.DB_SCRATCH]
[secrets.DB_APPROVAL]
"#,
        )
        .unwrap();
        Self {
            root,
            target_path,
            scratch_path,
        }
    }
    pub fn process(&self, command: &str) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ntnt"));
        c.args(["db", command, "--target", "test", "--json"])
            .env("DB_TARGET", &self.target_path)
            .env("DB_SCRATCH", &self.scratch_path)
            .env_remove("DB_APPROVAL")
            .env("NTNT_ENV", "development")
            .env("NTNT_SECRETS_PROVIDER", "env")
            .current_dir(self.root.path());
        c
    }
    pub fn command(&self, command: &str, args: &[&str]) -> Value {
        let out = self.process(command).args(args).output().unwrap();
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
    }
    pub fn plan(&self, path: &str) -> Value {
        let r = self.command("plan", &["--out", path]);
        assert_eq!(r["outcome"], "pass", "{r}");
        r
    }
    pub fn apply(&self, path: &str, approval: Option<&str>) -> Value {
        let mut c = self.process("migrate");
        c.args(["--plan", path]);
        if let Some(value) = approval {
            c.env("DB_APPROVAL", value);
        }
        let out = c.output().unwrap();
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
    }
    pub fn sql(&self, name: &str, sql: &str) {
        std::fs::write(self.root.path().join("migrations").join(name), sql).unwrap();
    }
    pub fn target(&self) -> Connection {
        let c = Connection::open(&self.target_path).unwrap();
        c.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        c
    }
}

/// An owned CLI child; a failed synchronization assertion must not orphan it.
pub struct Running(Option<std::process::Child>);
impl Running {
    pub fn spawn(f: &Fixture, path: &str, approval: &str) -> Self {
        Self(Some(
            f.process("migrate")
                .args(["--plan", path])
                .env("DB_APPROVAL", approval)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        ))
    }
    pub fn until(&mut self, mut predicate: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            if predicate() {
                return;
            }
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                let out = self.0.take().unwrap().wait_with_output().unwrap();
                panic!(
                    "CLI exited before the controlled boundary: {} {}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            assert!(
                std::time::Instant::now() < deadline,
                "CLI did not reach the controlled boundary"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    pub fn kill(mut self) {
        let mut child = self.0.take().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
    }
    pub fn result(mut self) -> Value {
        let out = self.0.take().unwrap().wait_with_output().unwrap();
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
    }
}
impl Drop for Running {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
