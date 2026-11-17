//! End-to-end runs of `examples/database-dev`, following its README: source
//! `dev.env` in a POSIX shell, then status, plan, approve, migrate and verify.
//!
//! SQLite runs everywhere a POSIX shell exists. PostgreSQL needs the example's
//! own Compose stack (`docker compose up -d --wait` in
//! examples/database-dev/postgresql); it is ignored by default and runs in the
//! CI db-catalog lane.
#![cfg(unix)]
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Hang guard for every child and query in this file: a stalled one fails the
/// test instead of hanging it. Only a hang reaches it; the slowest step, `docker
/// compose up --wait`, finishes far sooner.
const HANG_GUARD: Duration = Duration::from_secs(180);

/// Run a child with piped output and a whole-run hang guard; a stalled child is
/// killed and the test fails instead of hanging.
fn output_within(mut command: Command, deadline: Duration) -> Output {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let drain = |mut pipe: Box<dyn std::io::Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if started.elapsed() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child did not finish within {deadline:?}: {command:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Output {
        status,
        stdout: stdout.join().unwrap(),
        stderr: stderr.join().unwrap(),
    }
}

fn example(engine: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("examples/database-dev")
        .join(engine)
}

/// Start from a copy of the example, as a user would: manifest, dev.env and migrations.
fn copy_example(engine: &str) -> tempfile::TempDir {
    let source = example(engine);
    let root = tempfile::tempdir().unwrap();
    for name in ["ntnt.toml", "dev.env"] {
        std::fs::copy(source.join(name), root.path().join(name)).unwrap();
    }
    std::fs::create_dir(root.path().join("migrations")).unwrap();
    for entry in std::fs::read_dir(source.join("migrations")).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(
            entry.path(),
            root.path().join("migrations").join(entry.file_name()),
        )
        .unwrap();
    }
    root
}

/// `. ./dev.env && ntnt db <args> --json`, with only the approval (if any)
/// coming from outside, as the README's `export APP_MIGRATION_APPROVAL=...`.
fn db(root: &Path, args: &str, approval: Option<&str>) -> Value {
    // Only commands the README shows, word for word, so the two cannot drift.
    let readme = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/database-dev/README.md"),
    )
    .unwrap();
    assert!(
        readme.contains(&format!("ntnt db {args}\n")),
        "the README does not show `ntnt db {args}`"
    );
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg(format!(". ./dev.env && \"$NTNT\" db {args} --json"))
        .env("NTNT", env!("CARGO_BIN_EXE_ntnt"))
        // As in an interactive shell, which keeps a symlinked path in PWD.
        .env("PWD", root)
        .current_dir(root);
    for inherited in [
        "NTNT_ENV",
        "NTNT_SECRETS_PROVIDER",
        "APP_DATABASE",
        "APP_REPLAY_DATABASE",
        "APP_DATABASE_FILE",
        "APP_REPLAY_FILE",
        "APP_MIGRATION_APPROVAL",
    ] {
        c.env_remove(inherited);
    }
    if let Some(digest) = approval {
        c.env("APP_MIGRATION_APPROVAL", digest);
    }
    let output = output_within(c, HANG_GUARD);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// The README's reset steps, word for word.
const POSTGRESQL_RESET: &str =
    "docker compose down -v && rm -f review-*.json\ndocker compose up -d --wait\n";
const SQLITE_RESET: &str = "rm -rf data review-*.json\n. ./dev.env\n";

fn readme_shows(snippet: &str) {
    let readme = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/database-dev/README.md"),
    )
    .unwrap();
    assert!(
        readme.contains(snippet),
        "the README does not show:\n{snippet}"
    );
}

/// Run shell lines in `root`, bounded.
fn shell(root: &Path, lines: &str) {
    let mut c = Command::new("sh");
    c.arg("-c").arg(lines).current_dir(root);
    let output = output_within(c, HANG_GUARD);
    assert!(
        output.status.success(),
        "{lines}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The README's first block on a fresh database: status, plan, approve,
/// migrate and verify, applying every migration in `migrations/`.
fn fresh_workflow(root: &Path, migrations: usize) {
    let status = db(root, "status --target development", None);
    assert_eq!(status["outcome"], "pass", "{status}");
    assert_eq!(status["history"]["pending_count"], migrations, "{status}");
    let plan = db(
        root,
        "plan --target development --out review-0001.json",
        None,
    );
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let digest = plan["plan_digest"].as_str().unwrap().to_string();
    let applied = db(
        root,
        "migrate --target development --plan review-0001.json",
        Some(&digest),
    );
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(states(&applied), vec!["committed"; migrations], "{applied}");
    let verify = db(root, "verify --target development", None);
    assert_eq!(verify["outcome"], "pass", "{verify}");
    assert_eq!(verify["history"]["applied_count"], migrations, "{verify}");
}

fn has_code(report: &Value, code: &str) -> bool {
    report["diagnostics"]
        .as_array()
        .is_some_and(|d| d.iter().any(|d| d["code"] == code))
}

fn states(report: &Value) -> Vec<String> {
    report["progress"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| p["state"].as_str().unwrap_or("").to_string())
        .collect()
}

/// The README's workflow, then its "Next changes" section.
fn documented_workflow(root: &Path, next_migration: &str) {
    let status = db(root, "status --target development", None);
    assert_eq!(status["outcome"], "pass", "{status}");
    assert_eq!(status["history"]["pending_count"], 1, "{status}");

    let plan = db(
        root,
        "plan --target development --out review-0001.json",
        None,
    );
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let digest = plan["plan_digest"].as_str().unwrap().to_string();
    // The saved plan carries the digest the README extracts with jq.
    let saved: Value =
        serde_json::from_slice(&std::fs::read(root.join("review-0001.json")).unwrap()).unwrap();
    assert_eq!(saved["plan_digest"], digest.as_str());

    let unapproved = db(
        root,
        "migrate --target development --plan review-0001.json",
        None,
    );
    assert!(
        has_code(&unapproved, "DB_APPROVAL_REQUIRED"),
        "{unapproved}"
    );
    let applied = db(
        root,
        "migrate --target development --plan review-0001.json",
        Some(&digest),
    );
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(states(&applied), ["committed"], "{applied}");
    let verify = db(root, "verify --target development", None);
    assert_eq!(verify["outcome"], "pass", "{verify}");
    let rerun = db(
        root,
        "migrate --target development --plan review-0001.json",
        Some(&digest),
    );
    assert_eq!(rerun["outcome"], "pass", "{rerun}");
    assert_eq!(states(&rerun), ["already_applied"], "{rerun}");

    // Next change: a new numbered file, a new plan and a new digest. The old
    // approval does not apply to it.
    std::fs::write(root.join("migrations/0002_names.sql"), next_migration).unwrap();
    let next = db(
        root,
        "plan --target development --out review-0002.json",
        None,
    );
    assert_eq!(next["outcome"], "pass", "{next}");
    let next_digest = next["plan_digest"].as_str().unwrap().to_string();
    assert_ne!(next_digest, digest);
    let stale_approval = db(
        root,
        "migrate --target development --plan review-0002.json",
        Some(&digest),
    );
    assert!(
        has_code(&stale_approval, "DB_APPROVAL_REQUIRED"),
        "{stale_approval}"
    );
    let applied = db(
        root,
        "migrate --target development --plan review-0002.json",
        Some(&next_digest),
    );
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let verify = db(root, "verify --target development", None);
    assert_eq!(verify["outcome"], "pass", "{verify}");
    assert_eq!(verify["history"]["applied_count"], 2, "{verify}");
}

#[test]
fn sqlite_development_setup_runs_as_documented() {
    let root = copy_example("sqlite");
    documented_workflow(root.path(), "ALTER TABLE accounts ADD COLUMN name TEXT;\n");
    // dev.env created the files once and left the scratch file empty.
    assert_eq!(
        std::fs::metadata(root.path().join("data/replay.sqlite"))
            .unwrap()
            .len(),
        0
    );
    // The README's reset, then its workflow again from the start.
    readme_shows(SQLITE_RESET);
    shell(root.path(), SQLITE_RESET);
    fresh_workflow(root.path(), 2);
}

/// `ntnt db` refuses a path through a symlink, and a shell that entered the
/// directory through a link keeps the link in PWD: dev.env must not use it.
#[test]
fn sqlite_development_setup_runs_from_a_symlinked_directory() {
    let root = copy_example("sqlite");
    let links = tempfile::tempdir().unwrap();
    let link = links.path().join("project");
    std::os::unix::fs::symlink(root.path(), &link).unwrap();
    fresh_workflow(&link, 1);
}

/// `docker compose <args>` for the example's own stack, bounded.
fn compose(args: &[&str]) {
    let mut c = Command::new("docker");
    c.arg("compose")
        .arg("-f")
        .arg(example("postgresql").join("compose.yml"))
        .args(args);
    let output = output_within(c, HANG_GUARD);
    assert!(
        output.status.success(),
        "docker compose {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Run one SQL statement as the development owner from dev.env, bounded.
fn development_sql(root: &Path, sql: &str) -> Result<i64, String> {
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg(". ./dev.env && printf %s \"$APP_DATABASE\"")
        .current_dir(root);
    let secret = String::from_utf8(output_within(c, HANG_GUARD).stdout).unwrap();
    let sql = sql.to_string();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            tokio::time::timeout(HANG_GUARD, async move {
                let (client, connection) = secret
                    .parse::<tokio_postgres::Config>()
                    .unwrap()
                    .connect(tokio_postgres::NoTls)
                    .await
                    .unwrap();
                let task = tokio::spawn(connection);
                let value = client
                    .query_one(&sql, &[])
                    .await
                    .map(|row| row.get::<_, i64>(0))
                    .map_err(|e| format!("{e:?}"));
                drop(client);
                let _ = task.await;
                value
            })
            .await
            .unwrap_or_else(|_| {
                panic!("development PostgreSQL did not answer within {HANG_GUARD:?}")
            })
        })
}

#[test]
#[ignore = "requires docker and `docker compose up -d --wait` in examples/database-dev/postgresql; CI db catalog lane"]
fn postgresql_development_setup_runs_as_documented() {
    let root = copy_example("postgresql");
    let status = db(root.path(), "status --target development", None);
    assert_eq!(
        status["history"]["applied_count"], 0,
        "the development database is not fresh; reset it with `docker compose down -v && docker compose up -d --wait`: {status}"
    );
    documented_workflow(
        root.path(),
        "ALTER TABLE public.accounts ADD COLUMN name TEXT;\n",
    );
    // An ordinary `docker compose down` keeps the databases; only `down -v`
    // deletes them.
    let inserted = development_sql(
        root.path(),
        "WITH row AS (INSERT INTO public.accounts (email, name) VALUES ('kept@example.test', 'Kept') RETURNING id) SELECT count(*) FROM row",
    );
    assert_eq!(inserted, Ok(1));
    compose(&["down"]);
    compose(&["up", "-d", "--wait"]);
    let kept = development_sql(
        root.path(),
        "SELECT count(*) FROM public.accounts WHERE email = 'kept@example.test'",
    );
    assert_eq!(
        kept,
        Ok(1),
        "the migrated row did not survive `docker compose down`"
    );
    let verify = db(root.path(), "verify --target development", None);
    assert_eq!(verify["outcome"], "pass", "{verify}");
    assert_eq!(verify["history"]["applied_count"], 2, "{verify}");
    // The README's reset deletes the volume and the saved plans; the
    // workflow then runs again from the start.
    readme_shows(POSTGRESQL_RESET);
    compose(&["down", "-v"]);
    shell(root.path(), "rm -f review-*.json");
    compose(&["up", "-d", "--wait"]);
    fresh_workflow(root.path(), 2);
}
