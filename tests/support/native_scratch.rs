//! A native replay owns one initdb and its storage for exactly one CLI command.
//! No published port, network, real credentials, or reusable database to clean.
use super::Fixture;
use postgres::{Client, NoTls};
use serde_json::Value;
use std::process::Command;

fn docker(args: &[&str]) -> String {
    let out = Command::new("docker").args(args).output().expect("docker");
    assert!(
        out.status.success(),
        "docker {}: {}",
        args[0],
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_owned()
}

struct Container(String);

impl Drop for Container {
    fn drop(&mut self) {
        // Scope removal to our exact container ID. -v includes the anonymous
        // data volume declared by the image (whose mount changed in PG 18).
        let removed = Command::new("docker").args(["rm", "-fv", &self.0]).output();
        if !removed.is_ok_and(|out| out.status.success()) {
            if std::thread::panicking() {
                eprintln!("failed to remove native replay container {}", self.0);
            } else {
                panic!("failed to remove native replay container {}", self.0);
            }
        }
    }
}

pub(super) struct NativeScratch {
    // Drop the container before the private socket directory.
    container: Container,
    directory: tempfile::TempDir,
    url: String,
    pub cluster: String,
}

impl NativeScratch {
    pub fn new(f: &Fixture) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("socket");
        std::fs::create_dir(&socket).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Only postgres needs access inside Docker; only the test's user
            // can traverse the parent on the host (local socket trust).
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o777)).unwrap();
        }
        let version: i32 = f
            .target()
            .query_one("SELECT current_setting('server_version_num')::int", &[])
            .unwrap()
            .get(0);
        let image = std::env::var("NTNT_DB_TEST_PG_IMAGE")
            .unwrap_or_else(|_| "mirror.gcr.io/library/postgres:{major}".to_owned())
            .replace("{major}", &(version / 10000).to_string());
        let owner = std::env::var("NTNT_DB_TEST_REPLAY_JOB")
            .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
        let id = docker(&[
            "run",
            "--rm",
            "-d",
            "--pull=never",
            "--network=none",
            "--label",
            &format!("ntnt.test.native-replay={owner}"),
            "--mount",
            &format!("type=bind,src={},dst=/socket", socket.display()),
            "-e",
            "POSTGRES_HOST_AUTH_METHOD=trust",
            &image,
            "-c",
            "unix_socket_directories=/var/run/postgresql,/socket",
        ]);
        let container = Container(id);
        super::wait_for("final native PostgreSQL server", || {
            // The image's temporary init server never listens on TCP.
            Command::new("docker")
                .args([
                    "exec",
                    &container.0,
                    "pg_isready",
                    "-h",
                    "127.0.0.1",
                    "-U",
                    "postgres",
                ])
                .output()
                .unwrap()
                .status
                .success()
                .then_some(())
        });
        let host = socket
            .display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('\'', "\\'");
        let admin = format!("host='{host}' user=postgres dbname=postgres sslmode=disable");
        let mut db = Client::connect(&admin, NoTls).unwrap();
        let name = &f.scratch; // UUID-derived identifier from Fixture::new
        db.batch_execute(&format!("CREATE ROLE {name} LOGIN"))
            .unwrap();
        db.batch_execute(&format!(
            "CREATE DATABASE {name} OWNER {name} TEMPLATE template0"
        ))
        .unwrap();
        let cluster = db
            .query_one(
                "SELECT system_identifier::text FROM pg_control_system()",
                &[],
            )
            .unwrap()
            .get(0);
        let url = format!("host='{host}' user={name} dbname={name} sslmode=disable");
        Self {
            container,
            directory: dir,
            url,
            cluster,
        }
    }

    pub fn process(&self, f: &Fixture, cmd: &str) -> Command {
        let mut cmd = f.process(cmd);
        cmd.env("DB_SCRATCH", &self.url)
            .env("NTNT_DB_REPLAY_CLUSTER", &self.cluster);
        cmd
    }

    pub fn observer(&self) -> Client {
        let mut config: postgres::Config = self.url.parse().unwrap();
        config.user("postgres").connect(NoTls).unwrap()
    }

    pub fn storage(&self) -> (String, Vec<String>, std::path::PathBuf) {
        let volumes = docker(&[
            "inspect",
            "-f",
            "{{range .Mounts}}{{if eq .Type \"volume\"}}{{println .Name}}{{end}}{{end}}",
            &self.container.0,
        ]);
        assert!(
            !volumes.is_empty(),
            "expected the PG image's anonymous data volume"
        );
        (
            self.container.0.clone(),
            volumes.lines().map(str::to_owned).collect(),
            self.directory.path().to_owned(),
        )
    }
}

pub(super) fn assert_discarded((id, volumes, dir): (String, Vec<String>, std::path::PathBuf)) {
    assert!(
        docker(&["ps", "-aq", "--filter", &format!("id={id}")]).is_empty(),
        "scratch container survived"
    );
    for v in volumes {
        assert!(
            docker(&["volume", "ls", "-q", "--filter", &format!("name={v}")]).is_empty(),
            "scratch volume survived"
        );
    }
    assert!(!dir.exists(), "scratch socket directory survived");
}

pub(super) fn diagnostic_codes(r: &Value) -> Vec<&str> {
    r["diagnostics"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|d| d["code"].as_str())
        .collect()
}

pub(super) fn report(cmd: &mut Command) -> Value {
    let out = cmd.output().unwrap();
    serde_json::from_slice(&out.stdout).expect("expected JSON command report")
}

impl Fixture {
    pub fn isolated_command(&self, cmd: &str, args: &[&str]) -> Value {
        let scratch = NativeScratch::new(self);
        report(scratch.process(self, cmd).args(args))
    }

    pub fn isolated_plan(&self, path: &str) -> Value {
        let r = self.isolated_command("plan", &["--out", path]);
        assert_eq!(r["outcome"], "pass", "{:?}", diagnostic_codes(&r));
        assert_eq!(r["scratch"]["cleanup"], "external");
        r
    }
}
