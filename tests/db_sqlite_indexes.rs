//! SQLite partial and expression indexes (#411), and coverage diagnostics that
//! name the uncovered catalog objects.
#[path = "support/sqlite_fixture.rs"]
mod sqlite_fixture;
use serde_json::{json, Value};
use sqlite_fixture::Fixture;

fn has(r: &Value, code: &str) -> bool {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}
fn diagnostic<'a>(r: &'a Value, code: &str) -> &'a Value {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
        .unwrap_or_else(|| panic!("no {code} diagnostic: {r}"))
}

/// Run a `ntnt db` command for human output (no `--json`): success and stdout.
fn human(f: &Fixture, command: &str, args: &[&str]) -> (bool, String) {
    let template = f.process(command);
    let mut process = std::process::Command::new(template.get_program());
    process.args(
        template
            .get_args()
            .filter(|arg| *arg != std::ffi::OsStr::new("--json")),
    );
    for (key, value) in template.get_envs() {
        match value {
            Some(value) => process.env(key, value),
            None => process.env_remove(key),
        };
    }
    let output = process
        .current_dir(f.root.path())
        .args(args)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

fn partial(c: &rusqlite::Connection, table: &str, index: &str) -> i64 {
    c.query_row(
        "SELECT partial FROM pragma_index_list(?1) WHERE name=?2",
        [table, index],
        |r| r.get(0),
    )
    .unwrap()
}
fn constraint_violation(result: rusqlite::Result<()>) -> bool {
    result
        .unwrap_err()
        .sqlite_error_code()
        .is_some_and(|code| code == rusqlite::ErrorCode::ConstraintViolation)
}
fn schema_objects(c: &rusqlite::Connection) -> i64 {
    c.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .unwrap()
}

const CONTACTS: &str = "CREATE TABLE contact_requests (id INTEGER PRIMARY KEY, email TEXT NOT NULL, created_at TEXT NOT NULL, notify_enqueued_at TEXT);
CREATE INDEX contact_requests_pending ON contact_requests (notify_enqueued_at) WHERE notify_enqueued_at IS NULL;
CREATE UNIQUE INDEX contact_requests_one_open ON contact_requests (email) WHERE notify_enqueued_at IS NULL;";
const PENDING: &str = "index:\"contact_requests_pending\"";
const ONE_OPEN: &str = "index:\"contact_requests_one_open\"";

#[test]
fn sqlite_partial_and_partial_unique_indexes_plan_migrate_and_verify() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", CONTACTS);
    let plan = f.plan("plan.json");
    assert_eq!(plan["plan"]["classification"], "requires_review", "{plan}");
    assert_eq!(plan["diagnostics"], json!([]), "{plan}");
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(applied["progress"][0]["state"], "committed");
    let c = f.target();
    assert_eq!(
        partial(&c, "contact_requests", "contact_requests_pending"),
        1
    );
    assert_eq!(
        partial(&c, "contact_requests", "contact_requests_one_open"),
        1
    );
    // At most one open request per email; handled requests do not count.
    c.execute_batch(
        "INSERT INTO contact_requests VALUES(1,'a@example.com','t1','t2'),(2,'a@example.com','t3',NULL)",
    )
    .unwrap();
    assert!(constraint_violation(c.execute_batch(
        "INSERT INTO contact_requests VALUES(3,'a@example.com','t4',NULL)"
    )));
    for command in ["status", "verify"] {
        let r = f.command(command, &[]);
        assert_eq!(r["outcome"], "pass", "{command}: {r}");
        assert_eq!(r["catalog"]["uncovered"], json!([]), "{r}");
        for key in [PENDING, ONE_OPEN] {
            assert_eq!(r["catalog"]["objects"][key]["properties"][0][2], 1, "{r}");
        }
    }
    let rerun = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(rerun["outcome"], "pass", "{rerun}");
    assert_eq!(rerun["progress"][0]["state"], "already_applied");
}

/// Indexes are compared by the hash of their stored definition, so a changed
/// predicate is drift on the named index. That includes a predicate calling a
/// function outside the migration allowlist: SQLite resolves index predicates
/// when it loads the schema, without the authorizer, so such an out-of-band
/// index is reported as drift rather than accepted.
#[test]
fn sqlite_partial_index_predicate_changed_out_of_band_is_drift() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", CONTACTS);
    let plan = f.plan("plan.json");
    assert_eq!(
        f.apply("plan.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
    let recreate = |predicate: &str| {
        f.target().execute_batch(&format!("DROP INDEX contact_requests_pending; CREATE INDEX contact_requests_pending ON contact_requests (notify_enqueued_at) WHERE {predicate};")).unwrap();
    };
    for predicate in [
        "notify_enqueued_at IS NOT NULL",
        "instr(email, 'private_marker_411') > 0",
    ] {
        recreate(predicate);
        for command in ["status", "verify", "plan"] {
            let r = f.command(command, &[]);
            assert_eq!(r["outcome"], "fail", "{predicate} {command}: {r}");
            assert!(has(&r, "DB_DRIFT"), "{r}");
            assert!(!has(&r, "DB_COVERAGE_INCOMPLETE"), "{r}");
            assert_eq!(r["catalog"]["uncovered"], json!([]), "{r}");
            let differences = r["drift"]["differences"].as_array().unwrap();
            assert_eq!(differences.len(), 1, "{r}");
            let difference = &differences[0];
            assert_eq!(difference["object"], PENDING, "{r}");
            assert_eq!(difference["change"], "changed", "{r}");
            assert_ne!(
                difference["expected"]["definition"], difference["observed"]["definition"],
                "{r}"
            );
            for field in ["table", "columns", "properties"] {
                assert_eq!(difference["expected"][field], difference["observed"][field]);
            }
            assert!(!r.to_string().contains("private_marker_411"), "{r}");
            if command == "plan" {
                assert_eq!(r["plan"]["classification"], "blocked", "{r}");
            }
        }
    }
    // The original definition, recreated out of band, matches again.
    recreate("notify_enqueued_at IS NULL");
    let r = f.command("verify", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
}

const ACCOUNTS: &str =
    "CREATE TABLE accounts(id INTEGER PRIMARY KEY, email TEXT, deleted_at TEXT, legacy TEXT);";
const LIVE_EMAIL: &str =
    "CREATE UNIQUE INDEX accounts_live_email ON accounts(email) WHERE deleted_at IS NULL;";
fn rebuild(index: &str) -> String {
    format!("CREATE TABLE replacement(id INTEGER PRIMARY KEY, email TEXT NOT NULL, deleted_at TEXT); INSERT INTO replacement SELECT id, lower(email), deleted_at FROM accounts; DROP TABLE accounts; ALTER TABLE replacement RENAME TO accounts; {index}")
}

#[test]
fn sqlite_rebuild_requires_partial_indexes_to_be_recreated_unchanged() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", &format!("{ACCOUNTS} {LIVE_EMAIL}"));
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    // Lowercasing makes two rows 'alice'; only one of them is live.
    f.target()
        .execute_batch("INSERT INTO accounts VALUES(1,'ALICE',NULL,'old'),(2,'alice','2026-01-01','old'),(3,'BOB',NULL,'old')")
        .unwrap();
    f.sql("0002_rebuild.toml", "sqlite_rebuild = \"accounts\"");
    // Omitting the partial index, or recreating it with another predicate,
    // fails the rebuild guard during replay.
    for index in [
        "",
        "CREATE UNIQUE INDEX accounts_live_email ON accounts(email) WHERE deleted_at IS NOT NULL;",
    ] {
        f.sql("0002_rebuild.sql", &rebuild(index));
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{index}: {r}");
        assert_eq!(
            diagnostic(&r, "DB_REBUILD_GUARD")["source"],
            "0002_rebuild.sql",
            "{r}"
        );
    }
    f.sql("0002_rebuild.sql", &rebuild(LIVE_EMAIL));
    let plan = f.plan("upgrade.json");
    let r = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][1]["state"], "committed");
    let c = f.target();
    assert_eq!(partial(&c, "accounts", "accounts_live_email"), 1);
    let rows: Vec<(i64, String, Option<String>)> = c
        .prepare("SELECT id,email,deleted_at FROM accounts ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            (1, "alice".into(), None),
            (2, "alice".into(), Some("2026-01-01".into())),
            (3, "bob".into(), None)
        ]
    );
    assert!(constraint_violation(
        c.execute_batch("INSERT INTO accounts VALUES(4,'bob',NULL)")
    ));
    c.execute_batch("INSERT INTO accounts VALUES(5,'bob','2026-02-01')")
        .unwrap();
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}

/// Expression indexes share the mechanism: definition hash, compile-only DML
/// programs at inspection, and the function allowlist when a migration
/// creates them.
#[test]
fn sqlite_expression_indexes_plan_migrate_verify_and_drift() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE accounts(id INTEGER PRIMARY KEY, email TEXT NOT NULL, handle TEXT); CREATE UNIQUE INDEX accounts_email_ci ON accounts(lower(email)); CREATE INDEX accounts_handle_prefix ON accounts(substr(handle, 1, 3), id DESC) WHERE handle IS NOT NULL;");
    let plan = f.plan("plan.json");
    assert_eq!(plan["diagnostics"], json!([]), "{plan}");
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let c = f.target();
    let expression_columns: i64 = c
        .query_row(
            "SELECT count(*) FROM pragma_index_xinfo('accounts_email_ci') WHERE cid=-2",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(expression_columns, 1);
    c.execute_batch("INSERT INTO accounts VALUES(1,'Alice@example.com','alice')")
        .unwrap();
    assert!(constraint_violation(c.execute_batch(
        "INSERT INTO accounts VALUES(2,'ALICE@example.com',NULL)"
    )));
    let r = f.command("verify", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["catalog"]["uncovered"], json!([]), "{r}");
    c.execute_batch("DROP INDEX accounts_email_ci; CREATE UNIQUE INDEX accounts_email_ci ON accounts(upper(email));")
        .unwrap();
    let r = f.command("verify", &[]);
    assert_eq!(r["outcome"], "fail", "{r}");
    assert!(has(&r, "DB_DRIFT"), "{r}");
    let differences = r["drift"]["differences"].as_array().unwrap();
    assert_eq!(differences.len(), 1, "{r}");
    assert_eq!(differences[0]["object"], "index:\"accounts_email_ci\"");
    assert_eq!(differences[0]["change"], "changed");
}

/// A migration's index predicate or expression is authorized when SQLite
/// compiles the CREATE INDEX, against the same function allowlist as other
/// migration SQL; the allowlisted equivalents are covered.
#[test]
fn sqlite_index_predicates_and_expressions_keep_the_function_allowlist() {
    let f = Fixture::new();
    for (index, function) in [
        (
            "CREATE INDEX t_marked ON t(x) WHERE instr(x, '@') > 0;",
            "instr",
        ),
        ("CREATE INDEX t_hex ON t(hex(x));", "hex"),
    ] {
        f.sql(
            "0001_initial.sql",
            &format!("CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT); {index}"),
        );
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{index}: {r}");
        assert_eq!(r["plan"]["classification"], "blocked", "{r}");
        // Analysis accepts the statement; native execution of the migration
        // in scratch refuses it, before any catalog is inspected.
        assert_eq!(
            r["plan"]["pending"][0]["analysis"]["replay_supported"], true,
            "{r}"
        );
        let diagnostics = r["diagnostics"].as_array().unwrap();
        assert_eq!(diagnostics.len(), 1, "{r}");
        assert_eq!(diagnostics[0]["source"], "0001_initial.sql", "{r}");
        // The refusal names the statement and the function (#414).
        assert_eq!(diagnostics[0]["code"], "DB_SQL_UNSUPPORTED", "{r}");
        let message = diagnostics[0]["message"].as_str().unwrap();
        assert!(
            message.starts_with(&format!("Statement 2 uses SQLite function `{function}`")),
            "{message}"
        );
        assert_eq!(schema_objects(&f.target()), 0);
    }
    f.sql("0001_initial.sql", "CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT); CREATE INDEX t_marked ON t(x) WHERE length(x) > 0; CREATE INDEX t_lower ON t(lower(x));");
    let plan = f.plan("plan.json");
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}

/// Replay coverage failures name the migration and the uncovered catalog
/// keys; the target's own `catalog.uncovered` does not contain them.
#[test]
fn sqlite_replay_coverage_diagnostic_names_the_migration_and_objects() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(value TEXT); CREATE TABLE audit(id INTEGER);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    // The trigger's program writes the reserved ledger: t's update program
    // fails native authorization, so t is outside coverage after 0002.
    f.sql(
        "0002_trigger.sql",
        "CREATE TRIGGER hidden AFTER UPDATE OF value ON t BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;",
    );
    let r = f.command("plan", &[]);
    assert_eq!(r["plan"]["classification"], "blocked", "{r}");
    assert_eq!(r["catalog"]["uncovered"], json!([]), "{r}");
    let d = diagnostic(&r, "DB_COVERAGE_INCOMPLETE");
    assert_eq!(d["source"], "0002_trigger.sql", "{r}");
    assert_eq!(d["objects"], json!(["table:\"t\""]), "{r}");
    let (passed, stdout) = human(&f, "plan", &[]);
    assert!(!passed, "{stdout}");
    assert!(
        stdout.contains(concat!(
            "\"DB_COVERAGE_INCOMPLETE\": \"Intermediate catalog exceeds execution coverage.\"\n",
            "  Source: \"0002_trigger.sql\"\n",
            "  Objects: [\"table:\\\"t\\\"\"]\n",
            "  Next: ",
        )),
        "{stdout}"
    );
}

/// Target coverage failures name the uncovered target objects too.
#[test]
fn sqlite_target_coverage_diagnostic_names_the_objects() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE t(value TEXT);");
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    // Generated columns are outside SQLite coverage.
    f.target()
        .execute_batch("CREATE TABLE generated(a INTEGER, b INTEGER GENERATED ALWAYS AS (a + 1));")
        .unwrap();
    for command in ["status", "verify", "plan"] {
        let r = f.command(command, &[]);
        assert_ne!(r["outcome"], "pass", "{command}: {r}");
        assert_eq!(
            r["catalog"]["uncovered"],
            json!(["table:\"generated\""]),
            "{r}"
        );
        let d = diagnostic(&r, "DB_COVERAGE_INCOMPLETE");
        assert_eq!(d["objects"], json!(["table:\"generated\""]), "{r}");
        assert_eq!(d["source"], Value::Null, "{r}");
        let (passed, stdout) = human(&f, command, &[]);
        assert!(!passed, "{stdout}");
        assert!(
            stdout.contains("  Objects: [\"table:\\\"generated\\\"\"]\n  Next: "),
            "{stdout}"
        );
    }
}
