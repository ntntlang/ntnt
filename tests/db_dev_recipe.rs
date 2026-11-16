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
use std::process::Command;

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
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg(format!(". ./dev.env && \"$NTNT\" db {args} --json"))
        .env("NTNT", env!("CARGO_BIN_EXE_ntnt"))
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
    let output = c.output().unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
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

    let plan = db(root, "plan --target development --out review.json", None);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    let digest = plan["plan_digest"].as_str().unwrap().to_string();
    // The saved plan carries the digest the README extracts with jq.
    let saved: Value =
        serde_json::from_slice(&std::fs::read(root.join("review.json")).unwrap()).unwrap();
    assert_eq!(saved["plan_digest"], digest.as_str());

    let unapproved = db(
        root,
        "migrate --target development --plan review.json",
        None,
    );
    assert!(
        has_code(&unapproved, "DB_APPROVAL_REQUIRED"),
        "{unapproved}"
    );
    let applied = db(
        root,
        "migrate --target development --plan review.json",
        Some(&digest),
    );
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(states(&applied), ["committed"], "{applied}");
    let verify = db(root, "verify --target development", None);
    assert_eq!(verify["outcome"], "pass", "{verify}");
    let rerun = db(
        root,
        "migrate --target development --plan review.json",
        Some(&digest),
    );
    assert_eq!(rerun["outcome"], "pass", "{rerun}");
    assert_eq!(states(&rerun), ["already_applied"], "{rerun}");

    // Next change: a new numbered file, a new plan and a new digest. The old
    // approval does not apply to it.
    std::fs::write(root.join("migrations/0002_names.sql"), next_migration).unwrap();
    let next = db(root, "plan --target development --out review-2.json", None);
    assert_eq!(next["outcome"], "pass", "{next}");
    let next_digest = next["plan_digest"].as_str().unwrap().to_string();
    assert_ne!(next_digest, digest);
    let stale_approval = db(
        root,
        "migrate --target development --plan review-2.json",
        Some(&digest),
    );
    assert!(
        has_code(&stale_approval, "DB_APPROVAL_REQUIRED"),
        "{stale_approval}"
    );
    let applied = db(
        root,
        "migrate --target development --plan review-2.json",
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
}

#[test]
#[ignore = "requires `docker compose up -d --wait` in examples/database-dev/postgresql; CI db catalog lane"]
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
}
