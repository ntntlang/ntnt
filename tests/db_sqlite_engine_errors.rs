//! `ntnt db` SQLite engine-error diagnostics (#424, #425): a failure names
//! what failed and where, never a row value, and its next_action fixes it.
#[path = "support/sqlite_fixture.rs"]
mod sqlite_fixture;
use serde_json::Value;
use sqlite_fixture::Fixture;

/// The one diagnostic of report `r` with `code`.
fn diagnostic<'a>(r: &'a Value, code: &str) -> &'a Value {
    let found: Vec<&Value> = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|d| d["code"] == code)
        .collect();
    assert_eq!(found.len(), 1, "{r}");
    found[0]
}
fn message(d: &Value) -> &str {
    d["message"].as_str().unwrap()
}
fn next_action(d: &Value) -> &str {
    d["next_action"].as_str().unwrap()
}
/// Plan a project whose only migration is `sql`.
fn plan_one(sql: &str) -> Value {
    let f = Fixture::new();
    f.sql("0001_case.sql", sql);
    f.command("plan", &[])
}
/// Replay failures in the user's own SQL say what failed: SQLite's message
/// when it holds only identifiers, and the extended result code's name; a
/// value in the failing row never appears (#424 sq-09).
#[test]
fn sqlite_replay_failures_name_the_error_and_its_identifiers() {
    let cases: &[(&str, usize, &str)] = &[
        (
            "CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT); UPDATE t SET a = nope;",
            2,
            "no such column: nope (SQLITE_ERROR)",
        ),
        (
            "CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT); INSERT INTO t (nope) VALUES (1);",
            2,
            "table t has no column named nope (SQLITE_ERROR)",
        ),
        (
            "INSERT INTO missing (a) VALUES (1);",
            1,
            "no such table: missing (SQLITE_ERROR)",
        ),
        (
            "CREATE TABLE t (a INTEGER); CREATE TABLE t (b INTEGER);",
            2,
            "table t already exists (SQLITE_ERROR)",
        ),
        (
            "CREATE TABLE t (b TEXT UNIQUE); INSERT INTO t VALUES ('rowsecret-1'); INSERT INTO t VALUES ('rowsecret-1');",
            3,
            "UNIQUE constraint failed: t.b (SQLITE_CONSTRAINT_UNIQUE)",
        ),
        (
            "CREATE TABLE t (a TEXT, b TEXT NOT NULL); INSERT INTO t (a) VALUES ('rowsecret-2');",
            2,
            "NOT NULL constraint failed: t.b (SQLITE_CONSTRAINT_NOTNULL)",
        ),
        (
            "CREATE TABLE t (b TEXT CONSTRAINT long_enough CHECK (length(b) > 20)); INSERT INTO t VALUES ('rowsecret-3');",
            2,
            "CHECK constraint failed: long_enough (SQLITE_CONSTRAINT_CHECK)",
        ),
        // An unnamed CHECK is named by its expression, which is not shown.
        (
            "CREATE TABLE t (b TEXT CHECK (b <> 'schematext')); INSERT INTO t VALUES ('schematext');",
            2,
            "CHECK constraint failed (SQLITE_CONSTRAINT_CHECK)",
        ),
        (
            "CREATE TABLE p (id INTEGER PRIMARY KEY); CREATE TABLE c (p INTEGER REFERENCES p(id)); INSERT INTO c VALUES (777);",
            3,
            "FOREIGN KEY constraint failed (SQLITE_CONSTRAINT_FOREIGNKEY)",
        ),
        (
            "CREATE TABLE t (b INTEGER); CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(ABORT, 'b must be positive'); END; INSERT INTO t VALUES (1);",
            3,
            "a trigger raised: b must be positive (SQLITE_CONSTRAINT_TRIGGER)",
        ),
        // A RAISE message computed from the row is not a literal: suppressed.
        (
            "CREATE TABLE t (b TEXT); CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(ABORT, 'rejected ' || new.b); END; INSERT INTO t VALUES ('rowsecret-4');",
            3,
            "a trigger raised an error whose message is computed, so it is suppressed (SQLITE_CONSTRAINT_TRIGGER)",
        ),
        // RAISE(ROLLBACK) undoes the transaction that created the trigger,
        // and its literal is still shown; FAIL too (review r1).
        (
            "CREATE TABLE t (b INTEGER); CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(ROLLBACK, 'b must be positive'); END; INSERT INTO t VALUES (1);",
            3,
            "a trigger raised: b must be positive (SQLITE_CONSTRAINT_TRIGGER)",
        ),
        (
            "CREATE TABLE t (b INTEGER); CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(FAIL, 'b must be positive'); END; INSERT INTO t VALUES (1);",
            3,
            "a trigger raised: b must be positive (SQLITE_CONSTRAINT_TRIGGER)",
        ),
        (
            "CREATE TABLE t (b TEXT); CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(ROLLBACK, 'rejected ' || new.b); END; INSERT INTO t VALUES ('rowsecret-6');",
            3,
            "a trigger raised an error whose message is computed, so it is suppressed (SQLITE_CONSTRAINT_TRIGGER)",
        ),
        // A computed message is suppressed even when a trigger the migration
        // creates only after the failing statement spells out the same text.
        (
            "CREATE TABLE t (b TEXT); CREATE TRIGGER g BEFORE INSERT ON t BEGIN SELECT RAISE(ROLLBACK, new.b); END; INSERT INTO t VALUES ('rowsecret-8'); CREATE TRIGGER h BEFORE UPDATE ON t BEGIN SELECT RAISE(ABORT, 'rowsecret-8'); END;",
            3,
            "a trigger raised an error whose message is computed, so it is suppressed (SQLITE_CONSTRAINT_TRIGGER)",
        ),
        (
            "CREATE TABLE t (b INTEGER) STRICT; INSERT INTO t VALUES ('rowsecret-5');",
            2,
            "cannot store TEXT value in INTEGER column t.b (SQLITE_CONSTRAINT_DATATYPE)",
        ),
    ];
    for (sql, statement, detail) in cases {
        let r = plan_one(sql);
        let d = diagnostic(&r, "DB_SQLITE");
        assert_eq!(
            message(d),
            format!("Statement {statement}: {detail}."),
            "{sql}: {r}"
        );
        assert_eq!(d["source"], "0001_case.sql", "{r}");
        assert!(
            next_action(d).starts_with(&format!(
                "Fix statement {statement} of the migration named in source"
            )),
            "{r}"
        );
        for value in ["rowsecret", "schematext"] {
            assert!(!r.to_string().contains(value), "{sql}: {r}");
        }
    }
}

/// A statement that fails only against the target's data keeps the row out
/// of the report, says nothing from the migration was committed, and the
/// advice it gives works (#424 sq-09).
#[test]
fn sqlite_migrate_failure_names_the_statement_and_how_to_continue() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE t (b TEXT UNIQUE);");
    let p = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", p["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("INSERT INTO t VALUES ('rowsecret-target')")
        .unwrap();
    f.sql(
        "0002_seed.sql",
        "INSERT INTO t VALUES ('first'); INSERT INTO t VALUES ('rowsecret-target');",
    );
    let p = f.plan("seed.json");
    let failed = f.apply("seed.json", p["plan_digest"].as_str());
    let d = diagnostic(&failed, "DB_SQLITE");
    assert_eq!(
        message(d),
        "Statement 2: UNIQUE constraint failed: t.b (SQLITE_CONSTRAINT_UNIQUE).",
        "{failed}"
    );
    assert_eq!(d["source"], "0002_seed.sql");
    assert!(
        next_action(d).starts_with("Nothing from this migration was committed"),
        "{failed}"
    );
    assert!(!failed.to_string().contains("rowsecret"), "{failed}");
    assert_eq!(failed["progress"][1]["state"], "failed", "{failed}");
    // Following the advice: correct the conflicting row, rerun the same plan.
    f.target()
        .execute_batch("DELETE FROM t WHERE b = 'rowsecret-target'")
        .unwrap();
    let again = f.apply("seed.json", p["plan_digest"].as_str());
    assert_eq!(again["outcome"], "pass", "{again}");
}
/// RAISE(ROLLBACK) undoes the whole transaction, so in replay a trigger an
/// earlier migration created is gone from the catalog once the statement it
/// raised in has failed; its literal message is still shown, never the row,
/// and fixing the statement, as advised, gives a passing plan (review r1).
#[test]
fn sqlite_replay_shows_a_raise_rollback_literal_from_an_earlier_migration() {
    let f = Fixture::new();
    f.sql(
        "0001_t.sql",
        "CREATE TABLE t (b TEXT); CREATE TRIGGER g BEFORE INSERT ON t WHEN length(new.b) > 5 BEGIN SELECT RAISE(ROLLBACK, 'b is too long'); END;",
    );
    f.sql(
        "0002_rows.sql",
        "INSERT INTO t VALUES ('short'); INSERT INTO t VALUES ('rowsecret-7');",
    );
    let r = f.command("plan", &[]);
    let d = diagnostic(&r, "DB_SQLITE");
    assert_eq!(
        message(d),
        "Statement 2: a trigger raised: b is too long (SQLITE_CONSTRAINT_TRIGGER).",
        "{r}"
    );
    assert_eq!(d["source"], "0002_rows.sql", "{r}");
    assert!(
        next_action(d).starts_with("Fix statement 2 of the migration named in source"),
        "{r}"
    );
    assert!(!r.to_string().contains("rowsecret"), "{r}");
    // Following the advice: statement 2 fixed, the plan passes.
    f.sql(
        "0002_rows.sql",
        "INSERT INTO t VALUES ('short'); INSERT INTO t VALUES ('tiny');",
    );
    f.plan("rows.json");
}

/// In migrate, a trigger's RAISE(ROLLBACK) literal is shown whether the
/// failing migration creates the trigger, which the rollback undoes, or an
/// earlier migration committed it (review r1). The target's row never
/// appears, and correcting it, as advised, lets the same plan pass.
#[test]
fn sqlite_migrate_shows_a_raise_rollback_literal() {
    let trigger = "CREATE TRIGGER g BEFORE INSERT ON t WHEN (SELECT count(*) FROM t) > 0 BEGIN SELECT RAISE(ROLLBACK, 'only one row'); END;";
    let same = format!("{trigger} INSERT INTO t VALUES ('first');");
    let earlier = format!("CREATE TABLE t (b TEXT); {trigger}");
    for (initial, guard, statement) in [
        ("CREATE TABLE t (b TEXT);", same.as_str(), 2),
        (earlier.as_str(), "INSERT INTO t VALUES ('first');", 1),
    ] {
        let f = Fixture::new();
        f.sql("0001_initial.sql", initial);
        let p = f.plan("initial.json");
        assert_eq!(
            f.apply("initial.json", p["plan_digest"].as_str())["outcome"],
            "pass"
        );
        // A row the replay's empty database does not have.
        f.target()
            .execute_batch("INSERT INTO t VALUES ('rowsecret-target')")
            .unwrap();
        f.sql("0002_guard.sql", guard);
        let p = f.plan("guard.json");
        let failed = f.apply("guard.json", p["plan_digest"].as_str());
        let d = diagnostic(&failed, "DB_SQLITE");
        assert_eq!(
            message(d),
            format!(
                "Statement {statement}: a trigger raised: only one row (SQLITE_CONSTRAINT_TRIGGER)."
            ),
            "{guard}: {failed}"
        );
        assert_eq!(d["source"], "0002_guard.sql");
        assert!(
            next_action(d).starts_with("Nothing from this migration was committed"),
            "{failed}"
        );
        assert!(!failed.to_string().contains("rowsecret"), "{failed}");
        // Following the advice: correct the conflicting row, rerun the plan.
        f.target().execute_batch("DELETE FROM t").unwrap();
        let again = f.apply("guard.json", p["plan_digest"].as_str());
        assert_eq!(again["outcome"], "pass", "{guard}: {again}");
    }
}
#[cfg(unix)]
fn chmod(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
}
/// Root writes files whatever their mode, so these cases need another user.
#[cfg(unix)]
fn root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

/// A scratch file SQLite cannot write is named by its role and secret, not
/// blamed on statement 1, and the advice works (#424 sq-15).
#[cfg(unix)]
#[test]
fn sqlite_read_only_scratch_is_named_not_blamed_on_a_statement() {
    if root() {
        return;
    }
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t (id INTEGER PRIMARY KEY);",
    );
    chmod(&f.scratch_path, 0o444);
    let r = f.command("plan", &[]);
    let d = diagnostic(&r, "DB_SQLITE");
    assert!(
        message(d).starts_with(
            "The SQLite scratch file (secret DB_SCRATCH) is not writable (SQLITE_READONLY"
        ),
        "{r}"
    );
    assert!(!message(d).contains("Statement"), "{r}");
    assert!(next_action(d).contains("DB_SCRATCH names"), "{r}");
    assert!(
        !r.to_string().contains(f.root.path().to_str().unwrap()),
        "{r}"
    );
    chmod(&f.scratch_path, 0o644);
    let r = f.command("plan", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
}

/// A target file SQLite cannot write is named by its role and secret, and
/// the advice works (#424 sq-15).
#[cfg(unix)]
#[test]
fn sqlite_read_only_target_is_named_and_the_advice_works() {
    if root() {
        return;
    }
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t (id INTEGER PRIMARY KEY);",
    );
    chmod(&f.target_path, 0o444);
    let p = f.plan("read-only.json");
    let failed = f.apply("read-only.json", p["plan_digest"].as_str());
    let d = diagnostic(&failed, "DB_SQLITE");
    assert!(
        message(d).starts_with(
            "The SQLite target file (secret DB_TARGET) is not writable (SQLITE_READONLY"
        ),
        "{failed}"
    );
    assert!(!message(d).contains("Statement"), "{failed}");
    assert!(next_action(d).contains("DB_TARGET names"), "{failed}");
    assert!(
        next_action(d).contains("plan, review and approve again"),
        "{failed}"
    );
    // Following the advice: make it writable, then plan and migrate again.
    chmod(&f.target_path, 0o644);
    let p = f.plan("writable.json");
    let r = f.apply("writable.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
}
/// The parser gate names the statement it stops at and why: a syntax error
/// near a keyword, identifier or symbol (never a literal), the token limit,
/// an authored PRAGMA or a construct outside the subset (#424 sq-05, cli-20).
#[test]
fn sqlite_parser_gate_names_the_statement_and_the_cause() {
    let refusal = |sql: &str| {
        let r = plan_one(sql);
        let d = diagnostic(&r, "DB_REPLAY_UNSUPPORTED").clone();
        assert_eq!(d["source"], "0001_case.sql", "{r}");
        assert!(!next_action(&d).contains("CONCURRENTLY"), "{r}");
        assert!(!r.to_string().contains("rowsecret"), "{r}");
        d
    };
    // A syntax error in statement 40 of 40 (sq-05).
    let mut sql = String::from("CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT);\n");
    for i in 1..=38 {
        sql.push_str(&format!("INSERT INTO t (a) VALUES ('r{i}');\n"));
    }
    sql.push_str("CREATE INDEX t_a ON t (a) WHERE;\n");
    let d = refusal(&sql);
    assert_eq!(message(&d), "Statement 40 has a syntax error near `;`.");
    assert!(next_action(&d).starts_with("Correct statement 40 "), "{d}");
    // A one-word typo in statement 2 (cli-20).
    let d = refusal(
        "CREATE TABLE accounts (id INTEGER PRIMARY KEY, email TEXT NOT NULL);\nCREAT INDEX accounts_email ON accounts (email);\nCREATE TABLE notes (id INTEGER PRIMARY KEY);\n",
    );
    assert_eq!(message(&d), "Statement 2 has a syntax error near `CREAT`.");
    // A trigger body's semicolons do not end its statement.
    let d = refusal(
        "CREATE TABLE t (a TEXT);\nCREATE TRIGGER g AFTER INSERT ON t BEGIN DELETE FROM t WHERE a IS NULL; DELETE FROM t WHERE a = 'y'; END;\nCREAT TABLE u (id INTEGER);\n",
    );
    assert_eq!(message(&d), "Statement 3 has a syntax error near `CREAT`.");
    // A UTF-8 byte order mark.
    let d = refusal("\u{feff}CREATE TABLE a (id INTEGER PRIMARY KEY);\n");
    assert_eq!(
        message(&d),
        "Statement 1 has a syntax error near a UTF-8 byte order mark (BOM) at the start of the file."
    );
    // Near a literal: the literal is not shown.
    let d = refusal("CREATE TABLE t (a TEXT);\nINSERT INTO t VALUES ('rowsecret-syntax' 5);\n");
    assert_eq!(
        message(&d),
        "Statement 2 has a syntax error near a literal."
    );
    // The token limit, crossed in statement 316: each line is 13 tokens.
    let mut sql = String::from("CREATE TABLE t (a INTEGER);\n");
    for _ in 0..400 {
        sql.push_str("INSERT INTO t VALUES (1);\n");
    }
    let d = refusal(&sql);
    assert!(
        message(&d).starts_with("Statement 316 passes the 4096-token limit"),
        "{d}"
    );
    assert!(next_action(&d).starts_with("Split the migration"), "{d}");
    // An authored PRAGMA is named; its value is not shown.
    let d = refusal("CREATE TABLE a (id INTEGER PRIMARY KEY);\nPRAGMA user_version = 31337;\n");
    assert_eq!(
        message(&d),
        "Statement 2 is PRAGMA user_version; migrations cannot run PRAGMAs."
    );
    assert!(next_action(&d).starts_with("Remove the PRAGMA"), "{d}");
    let d = refusal("PRAGMA foreign_keys = ON;\nCREATE TABLE a (id INTEGER PRIMARY KEY);\n");
    assert_eq!(
        message(&d),
        "Statement 1 is PRAGMA foreign_keys; migrations cannot run PRAGMAs."
    );
    // A construct outside the subset, by its leading keywords.
    let d = refusal(
        "CREATE TABLE a (id INTEGER PRIMARY KEY);\nCREATE VIRTUAL TABLE v USING fts5(x);\n",
    );
    assert_eq!(
        message(&d),
        "Statement 2 (`CREATE VIRTUAL TABLE`) is outside the supported SQLite subset."
    );
    assert!(next_action(&d).starts_with("Rewrite statement 2 "), "{d}");
    let d =
        refusal("CREATE TABLE a (id INTEGER PRIMARY KEY);\nCREATE TABLE b AS SELECT id FROM a;\n");
    assert_eq!(
        message(&d),
        "Statement 2 (`CREATE TABLE ... AS SELECT`) is outside the supported SQLite subset."
    );
    let d = refusal("BEGIN;\nCREATE TABLE a (id INTEGER PRIMARY KEY);\nCOMMIT;\n");
    assert_eq!(
        message(&d),
        "Statement 1 (`BEGIN`) is outside the supported SQLite subset."
    );
}

/// An authorizer refusal without a name of its own names the action.
#[test]
fn sqlite_recursive_cte_refusal_names_the_action() {
    let r = plan_one(
        "CREATE TABLE n (x INTEGER);\nINSERT INTO n SELECT x FROM (WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c WHERE x < 3) SELECT x FROM c);\n",
    );
    assert_eq!(
        message(diagnostic(&r, "DB_SQL_UNSUPPORTED")),
        "Statement 2 uses a recursive common table expression (WITH RECURSIVE), which SQLite migrations cannot use.",
        "{r}"
    );
}
/// `ntnt db migrate` as a person runs it, without --json.
fn human_migrate(f: &Fixture, plan: &str, approval: &str) -> String {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["db", "migrate", "--target", "test", "--plan", plan])
        .env("DB_TARGET", &f.target_path)
        .env("DB_SCRATCH", &f.scratch_path)
        .env("DB_APPROVAL", approval)
        .env("NTNT_ENV", "development")
        .env("NTNT_SECRETS_PROVIDER", "env")
        .current_dir(f.root.path())
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A target another connection holds the write lock on: DB_BUSY says so,
/// gives the busy timeout, says nothing was committed and to retry once the
/// writer finishes, which works (#425 sq-11). The lock is held from a second
/// connection for the whole run, so the outcome never depends on timing.
#[test]
fn sqlite_busy_target_names_the_lock_and_retry_works() {
    let f = Fixture::new();
    f.sql(
        "0001_base.sql",
        "CREATE TABLE events (id INTEGER PRIMARY KEY, kind TEXT);",
    );
    let p = f.plan("base.json");
    assert_eq!(
        f.apply("base.json", p["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql(
        "0002_more.sql",
        "CREATE TABLE more (id INTEGER PRIMARY KEY);",
    );
    let p = f.plan("more.json");
    let digest = p["plan_digest"].as_str().unwrap();
    let app = f.target();
    app.execute_batch("BEGIN IMMEDIATE; INSERT INTO events (kind) VALUES ('rowsecret-app');")
        .unwrap();
    let failed = f.apply("more.json", Some(digest));
    let d = diagnostic(&failed, "DB_BUSY");
    let text = message(d);
    assert!(
        text.starts_with("The SQLite target file (secret DB_TARGET) is locked (SQLITE_BUSY): another connection holds its write lock"),
        "{failed}"
    );
    assert!(text.contains("EXCLUSIVE locking mode"), "{failed}");
    // The busy timeout is a limit, not a wait that happened: SQLite reports
    // some conflicts at once (CodeRabbit on #445, sqlite.rs).
    assert!(
        text.contains("ntnt waits up to its 2-second busy timeout for such a lock, and SQLite reports some conflicts at once, without waiting."),
        "{failed}"
    );
    assert!(!text.contains("waited"), "{failed}");
    assert!(text.contains("Nothing was committed"), "{failed}");
    assert!(
        next_action(d).starts_with("Retry once the writer finishes"),
        "{failed}"
    );
    assert!(!failed.to_string().contains("rowsecret"), "{failed}");
    assert_ne!(failed["progress"][1]["state"], "committed", "{failed}");
    let human = human_migrate(&f, "more.json", digest);
    assert!(human.contains("Target: \"test\" / \"main\""), "{human}");
    assert!(!human.contains("(null)"), "{human}");
    // Following the advice: once the writer finishes, the same plan runs.
    app.execute_batch("COMMIT").unwrap();
    let again = f.apply("more.json", Some(digest));
    assert_eq!(again["outcome"], "pass", "{again}");
}

/// Inspection of a target in EXCLUSIVE locking mode and replay on a scratch
/// another connection writes: DB_BUSY names the file's role and secret.
#[test]
fn sqlite_busy_inspection_and_replay_name_the_file() {
    let f = Fixture::new();
    f.sql(
        "0001_base.sql",
        "CREATE TABLE events (id INTEGER PRIMARY KEY, kind TEXT);",
    );
    let p = f.plan("base.json");
    assert_eq!(
        f.apply("base.json", p["plan_digest"].as_str())["outcome"],
        "pass"
    );
    // After its first write, a connection in EXCLUSIVE locking mode keeps
    // the file locked until it closes.
    let app = f.target();
    app.execute_batch("PRAGMA locking_mode=EXCLUSIVE; INSERT INTO events (kind) VALUES ('x');")
        .unwrap();
    let r = f.command("status", &[]);
    let d = diagnostic(&r, "DB_BUSY");
    assert!(
        message(d).starts_with("The SQLite target file (secret DB_TARGET) is locked"),
        "{r}"
    );
    assert!(message(d).contains("Inspection writes nothing"), "{r}");
    drop(app);
    let r = f.command("status", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
    let other = rusqlite::Connection::open(&f.scratch_path).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    let r = f.command("plan", &[]);
    let d = diagnostic(&r, "DB_BUSY");
    assert!(
        message(d).starts_with("The SQLite scratch file (secret DB_SCRATCH) is locked"),
        "{r}"
    );
    assert!(message(d).contains("the target was not changed"), "{r}");
    other.execute_batch("ROLLBACK").unwrap();
    let r = f.command("plan", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
}
/// An empty (0-byte) scratch takes the target's page size, encoding and
/// auto-vacuum before its first write, as it takes its journal mode (#425
/// sq-17).
#[test]
fn sqlite_empty_scratch_takes_the_target_file_settings() {
    let f = Fixture::new();
    f.target()
        .execute_batch("PRAGMA encoding='UTF-16le'; PRAGMA page_size=8192; PRAGMA auto_vacuum=INCREMENTAL; CREATE TABLE _p(a); DROP TABLE _p; PRAGMA journal_mode=WAL;")
        .unwrap();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t (id INTEGER PRIMARY KEY, label TEXT DEFAULT 'a');",
    );
    let p = f.plan("initial.json");
    // Replay rolls back, so the scratch file is empty again afterwards; the
    // report gives the settings replay ran with.
    let scratch = &p["scratch"]["identity"]["settings"];
    assert_eq!(scratch["encoding"], "UTF-16le", "{p}");
    assert_eq!(scratch["page_size"], 8192, "{p}");
    assert_eq!(scratch["auto_vacuum"], 2, "{p}");
    assert_eq!(scratch["journal_mode"], "wal", "{p}");
    let r = f.apply("initial.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}

/// A scratch that already has pages cannot change them: DB_SCRATCH_MISMATCH
/// says how to make a matching one, and doing so works (#425 sq-17).
#[test]
fn sqlite_scratch_mismatch_says_how_to_make_a_matching_scratch() {
    let f = Fixture::new();
    f.target()
        .execute_batch("PRAGMA page_size=8192; CREATE TABLE _p(a); DROP TABLE _p;")
        .unwrap();
    rusqlite::Connection::open(&f.scratch_path)
        .unwrap()
        .execute_batch("CREATE TABLE _p(a); DROP TABLE _p;")
        .unwrap();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t (id INTEGER PRIMARY KEY);",
    );
    let r = f.command("plan", &[]);
    let d = diagnostic(&r, "DB_SCRATCH_MISMATCH");
    assert_eq!(
        message(d),
        "SQLite scratch settings differ from the deployment file: page_size: target 8192, scratch 4096.",
        "{r}"
    );
    assert!(
        next_action(d)
            .starts_with("Replace the scratch file that DB_SCRATCH names with a new empty file"),
        "{r}"
    );
    // Following it: a new empty file at the same path.
    std::fs::remove_file(&f.scratch_path).unwrap();
    std::fs::File::create(&f.scratch_path).unwrap();
    let r = f.command("plan", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
}
