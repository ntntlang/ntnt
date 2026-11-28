//! An already-applied migration must remain replayable for a forward repair (#441).
//!
//! Companion two-CLI check: against an empty target in this fixture, use the
//! pre-fix CLI to `db plan --target test --out old.json` for LEGACY_MIGRATION
//! alone. Keep that target (plans bind its file identity). Set DB_APPROVAL to
//! the saved plan_digest, then `db migrate --target test --plan old.json` with
//! the candidate. Expect the date-function refusal and unchanged target bytes,
//! not a stale-plan or missing-approval error.
#[path = "support/sqlite_fixture.rs"]
mod sqlite_fixture;
use sqlite_fixture::Fixture;

const LEGACY_MIGRATION: &str = "CREATE TABLE sessions(id INTEGER PRIMARY KEY, expires_at TEXT NOT NULL);\nCREATE INDEX active_sessions ON sessions(expires_at) WHERE expires_at > datetime('now');";
// From a disposable target after 3f933d7 planned and migrated LEGACY_MIGRATION
// as 0001_sessions.sql. Preserve its real ledger, including the checksum; don't
// ask the current planner to approve the unsafe SQL to set up this test.
const LEGACY_LEDGER: &str = "CREATE TABLE _ntnt_schema_migrations_v2 (position INTEGER NOT NULL PRIMARY KEY,id TEXT NOT NULL,name TEXT NOT NULL,checksum TEXT NOT NULL,state TEXT NOT NULL,logical_database TEXT NOT NULL,managed_schema TEXT NOT NULL) STRICT;
INSERT INTO _ntnt_schema_migrations_v2 VALUES(1,'1','0001_sessions.sql','4da11c23915b544f6d5554cbb90d958a6695af5b70861b00b2c582ccd39c6d72','applied','app','main');";

#[test]
fn applied_legacy_date_index_can_be_repaired_without_rewriting_history() {
    let f = Fixture::new();
    f.sql("0001_sessions.sql", LEGACY_MIGRATION);
    let c = f.target();
    c.execute_batch(LEGACY_MIGRATION).unwrap();
    c.execute_batch(LEGACY_LEDGER).unwrap();
    drop(c);

    f.sql(
        "0002_repair.sql",
        "DROP INDEX active_sessions; CREATE INDEX sessions_by_expiry ON sessions(expires_at);",
    );
    let plan = f.plan("repair.json");
    let applied = f.apply("repair.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(
        applied["progress"][0]["state"], "already_applied",
        "{applied}"
    );
    assert_eq!(applied["progress"][1]["state"], "committed", "{applied}");
    let verified = f.command("verify", &["--require-no-pending"]);
    assert_eq!(verified["outcome"], "pass", "{verified}");

    // Schema verification alone passed before #441 too. The first row must
    // actually commit and be retrievable through the replacement index.
    let c = f.target();
    c.execute("INSERT INTO sessions VALUES (1, '2030-01-01')", [])
        .unwrap();
    let id: i64 = c
        .query_row(
            "SELECT id FROM sessions INDEXED BY sessions_by_expiry WHERE expires_at='2030-01-01'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(id, 1);
}
