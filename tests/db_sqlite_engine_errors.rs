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
