#![allow(dead_code)]
use postgres::{Client, NoTls};
use serde_json::Value;
use std::process::Command;

pub struct Fixture {
    pub admin: Client,
    pub root: tempfile::TempDir,
    pub target: String,
    pub scratch: String,
    pub target_url: String,
    pub scratch_url: String,
}
impl Fixture {
    pub fn new() -> Self {
        let url = std::env::var("NTNT_DB_TEST_ADMIN")
            .expect("explicit disposable PostgreSQL fixture URL required");
        let mut admin = Client::connect(&url, NoTls).unwrap();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let target = format!("ntnt245_t_{token}");
        let scratch = format!("ntnt245_s_{token}");
        for name in [&target, &scratch] {
            admin.batch_execute(&format!("CREATE ROLE {name} LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS PASSWORD 'ntnt245_fixture_only'")).unwrap();
            admin
                .batch_execute(&format!(
                    "CREATE DATABASE {name} OWNER {name} TEMPLATE template0"
                ))
                .unwrap();
        }
        // URL supplied by fixture uses libpq key/value form, with trust only in
        // this isolated fixture. Production secrets never enter a process arg.
        let endpoint = std::env::var("NTNT_DB_TEST_HOST").expect("fixture host required");
        let target_url = format!("host={endpoint} user={target} dbname={target} password=ntnt245_fixture_only sslmode=disable");
        let scratch_url =
            format!("host={endpoint} user={scratch} dbname={scratch} password=ntnt245_fixture_only sslmode=disable");
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("migrations")).unwrap();
        std::fs::write(
            root.path().join("ntnt.toml"),
            format!(
                r#"
[databases.app]
engine = "postgresql"
migrations_dir = "migrations"
schema = "app"
approval_secret = "DB_APPROVAL"
[databases.app.targets.test]
connection_secret = "DB_TARGET"
database = "{target}"
role = "{target}"
[databases.app.scratch]
connection_secret = "DB_SCRATCH"
database = "{scratch}"
role = "{scratch}"
disposable = true
[secrets.DB_TARGET]
[secrets.DB_SCRATCH]
[secrets.DB_APPROVAL]
"#
            ),
        )
        .unwrap();
        let mut conn = Client::connect(&target_url, NoTls).unwrap();
        conn.batch_execute("CREATE SCHEMA app").unwrap();
        Self {
            admin,
            root,
            target,
            scratch,
            target_url,
            scratch_url,
        }
    }
    pub fn command(&self, command: &str) -> Value {
        self.command_args(command, &[])
    }
    pub fn command_args(&self, command: &str, args: &[&str]) -> Value {
        let output = self.process(command).args(args).output().unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
    }
    pub fn process(&self, command: &str) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_ntnt"));
        c.args(["db", command, "--target", "test", "--json"])
            .env("DB_TARGET", &self.target_url)
            .env("DB_SCRATCH", &self.scratch_url)
            .env_remove("DB_APPROVAL")
            .env("NTNT_SECRETS_PROVIDER", "env")
            .env("NTNT_ENV", "development")
            .current_dir(self.root.path());
        c
    }
    pub fn plan(&self, path: &str) -> Value {
        let r = self.command_args("plan", &["--out", path]);
        assert_eq!(r["outcome"], "pass", "{r}");
        r
    }
    pub fn apply(&self, path: &str, approval: Option<&str>) -> Value {
        let mut c = self.process("migrate");
        c.args(["--plan", path]);
        if let Some(value) = approval {
            c.env("DB_APPROVAL", value);
        }
        let output = c.output().unwrap();
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&output.stderr)))
    }
    pub fn seed_first(&self, sql: &str) -> Value {
        self.sql("0001_initial.sql", sql);
        let plan = self.command("plan");
        assert_eq!(plan["plan"]["candidate"]["outcome"], "pass", "{plan}");
        let checksum = plan["source"]["migrations"][0]["checksum"]
            .as_str()
            .unwrap();
        let mut c = self.target();
        c.batch_execute(sql).unwrap();
        c.batch_execute("CREATE SCHEMA _ntnt; CREATE TABLE _ntnt.schema_migrations_v1 (position bigint NOT NULL PRIMARY KEY,id text NOT NULL,name text NOT NULL,checksum text NOT NULL,state text NOT NULL)").unwrap();
        c.execute(
            "INSERT INTO _ntnt.schema_migrations_v1 VALUES(1,'1','0001_initial.sql',$1,'applied')",
            &[&checksum],
        )
        .unwrap();
        plan
    }
    pub fn target(&self) -> Client {
        Client::connect(&self.target_url, NoTls).unwrap()
    }
    pub fn sql(&self, name: &str, sql: &str) {
        std::fs::write(self.root.path().join("migrations").join(name), sql).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        for name in [&self.target, &self.scratch] {
            let _ = self
                .admin
                .batch_execute(&format!("DROP DATABASE {name} WITH (FORCE)"));
            let _ = self.admin.batch_execute(&format!("DROP ROLE {name}"));
        }
    }
}
