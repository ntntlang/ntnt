#[path = "support/sqlite_fixture.rs"]
mod sqlite_fixture;
use serde_json::Value;
use sqlite_fixture::Fixture;

fn code(r: &Value, code: &str) -> bool {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}

#[test]
fn sqlite_fresh_plan_contract_execution_and_unchanged_rerun() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE main.accounts(id INTEGER PRIMARY KEY, label TEXT DEFAULT 'a; b');",
    );
    f.sql(
        "0001_initial.toml",
        "[ensures]\nempty='SELECT NOT EXISTS (SELECT 1 FROM main.accounts)'",
    );
    let p = f.plan("plan.json");
    assert_eq!(p["binding"]["adapter"], "sqlite-catalog-v1");
    assert_eq!(p["assertions"]["executed"], 0);
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(code(&f.apply("plan.json", None), "DB_APPROVAL_REQUIRED"));
    let r = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][0]["state"], "committed");
    assert_eq!(r["assertions"]["executed"], 1);
    f.target()
        .execute_batch("INSERT INTO accounts(id) VALUES(17)")
        .unwrap();
    let again = f.apply("plan.json", p["plan_digest"].as_str());
    assert_eq!(again["outcome"], "pass", "{again}");
    assert_eq!(again["progress"][0]["state"], "already_applied");
    assert_eq!(again["assertions"]["executed"], 0);
    assert_eq!(
        f.target()
            .query_row("SELECT label FROM accounts WHERE id=17", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "a; b"
    );
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
    assert_eq!(
        rusqlite::Connection::open(&f.scratch_path)
            .unwrap()
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(!r.to_string().contains(f.root.path().to_str().unwrap()));
}

const DEPENDENCIES: &str = "CREATE INDEX email_idx ON accounts(email); CREATE VIEW account_emails AS SELECT id,email FROM accounts; CREATE TRIGGER email_changed AFTER UPDATE OF email ON accounts BEGIN INSERT INTO audit(id) VALUES(new.id); END;";

#[test]
fn sqlite_populated_rebuild_preserves_rows_dependencies_and_foreign_keys() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", &format!("CREATE TABLE accounts(id INTEGER PRIMARY KEY,email TEXT,legacy TEXT); CREATE TABLE children(id INTEGER PRIMARY KEY,account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE); CREATE TABLE audit(id INTEGER); {DEPENDENCIES}"));
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target().execute_batch("INSERT INTO accounts VALUES(1,'ALICE','old'),(2,'BOB','old'); INSERT INTO children VALUES(10,1),(11,2)").unwrap();
    f.sql("0002_rebuild.sql",&format!("CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT NOT NULL); INSERT INTO replacement SELECT id,lower(email) FROM accounts; DROP VIEW account_emails; DROP TABLE accounts; ALTER TABLE replacement RENAME TO accounts; {DEPENDENCIES}"));
    f.sql(
        "0002_rebuild.toml",
        r#"sqlite_rebuild = "accounts"
[requires]
no_nulls = "SELECT NOT EXISTS (SELECT 1 FROM main.accounts WHERE email IS NULL)"
[ensures]
retained_rows = "SELECT count(*) = 2 FROM main.accounts"
"#,
    );
    let plan = f.plan("upgrade.json");
    assert_eq!(plan["assertions"]["executed"], 0);
    let result = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(result["outcome"], "pass", "{result}");
    assert_eq!(result["assertions"]["executed"], 2);
    let c = f.target();
    let values: Vec<(i64, String)> = c
        .prepare("SELECT id,email FROM accounts ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(values, vec![(1, "alice".into()), (2, "bob".into())]);
    let children: Vec<(i64, i64)> = c
        .prepare("SELECT id,account_id FROM children ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(children, vec![(10, 1), (11, 2)]);
    assert_eq!(
        c.query_row("SELECT count(*) FROM account_emails", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM pragma_index_list('accounts') WHERE name='email_idx'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    c.execute_batch("UPDATE accounts SET email='changed' WHERE id=1")
        .unwrap();
    assert_eq!(
        c.query_row("SELECT id FROM audit", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
    assert_eq!(
        f.apply("upgrade.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
}

#[test]
fn sqlite_readonly_commands_do_not_create_or_reset_files() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE main.t(id INTEGER);");
    let before = std::fs::read(&f.target_path).unwrap();
    for command in ["status", "plan", "verify"] {
        assert_eq!(f.command(command, &[])["outcome"], "pass");
        assert_eq!(std::fs::read(&f.target_path).unwrap(), before);
    }
    let scratch = rusqlite::Connection::open(&f.scratch_path).unwrap();
    scratch
        .execute_batch(
            "CREATE TABLE sentinel(value TEXT); INSERT INTO sentinel VALUES('preserved')",
        )
        .unwrap();
    let rejected = f.command("plan", &[]);
    assert!(code(&rejected, "DB_SCRATCH_NOT_EMPTY"), "{rejected}");
    assert_eq!(
        scratch
            .query_row("SELECT value FROM sentinel", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "preserved"
    );
    assert_eq!(std::fs::read(&f.target_path).unwrap(), before);
    std::fs::remove_file(&f.target_path).unwrap();
    assert_ne!(f.command("status", &[])["outcome"], "pass");
    assert!(!f.target_path.exists());
}

#[test]
fn sqlite_scratch_aliases_and_replaced_targets_fail_closed() {
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE main.t(id INTEGER);");
    let plan = f.plan("plan.json");
    let original_scratch = f.scratch_path.clone();
    f.scratch_path = f
        .target_path
        .parent()
        .unwrap()
        .join(".")
        .join("deployment.sqlite");
    let collision = f.command("plan", &[]);
    assert!(code(&collision, "DB_SCRATCH_COLLISION"), "{collision}");
    f.scratch_path = original_scratch;
    let alias = f.root.path().join("alias.sqlite");
    std::fs::hard_link(&f.target_path, &alias).unwrap();
    assert!(code(&f.command("status", &[]), "DB_FILE_IDENTITY"));
    std::fs::remove_file(alias).unwrap();
    std::fs::rename(&f.target_path, f.root.path().join("original.sqlite")).unwrap();
    rusqlite::Connection::open(&f.target_path).unwrap();
    let wrong = f.apply("plan.json", plan["plan_digest"].as_str());
    assert!(code(&wrong, "DB_WRONG_TARGET"), "{wrong}");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn sqlite_native_authorizer_rejects_escape_and_opaque_programs() {
    let f = Fixture::new();
    for sql in [
        "CREATE TABLE main.t(id INTEGER); COMMIT;",
        "PRAGMA foreign_keys=OFF; CREATE TABLE main.t(id INTEGER);",
        "ATTACH ':memory:' AS other; CREATE TABLE other.t(id INTEGER);",
        "CREATE TEMP TABLE t(id INTEGER);",
        "CREATE VIRTUAL TABLE main.t USING fts5(value);",
        "CREATE TABLE main.t(id INTEGER); ALTER TABLE main.t RENAME TO _ntnt_hidden;",
        "CREATE TABLE main.t(id INTEGER); CREATE VIEW v AS SELECT load_extension('not-a-library');",
        "CREATE TABLE main.t(id INTEGER); CREATE VIEW v AS SELECT * FROM pragma_database_list;",
        "CREATE TABLE main.t(id INTEGER); CREATE VIEW v AS SELECT * FROM json_each('[1,2]');",
        "CREATE TABLE t(value INTEGER); INSERT INTO t SELECT value FROM json_each('[1,2]');",
        "CREATE TABLE json_each(value INTEGER); DROP TABLE json_each; CREATE TABLE t(value INTEGER); INSERT INTO t SELECT value FROM json_each('[1,2]');",
        "CREATE TABLE t(value INTEGER); INSERT INTO t VALUES(?1);",
        "CREATE TABLE main.t(id INTEGER); CREATE TRIGGER hidden AFTER INSERT ON main.t BEGIN SELECT load_extension('not-a-library'); END;",
    ] {
        f.sql("0001_escape.sql",sql);
        let r=f.command("plan",&[]);
        assert_ne!(r["outcome"],"pass","{sql}: {r}");
        assert_eq!(f.target().query_row("SELECT count(*) FROM sqlite_schema",[],|r|r.get::<_,i64>(0)).unwrap(),0);
        assert!(!r.to_string().contains("not-a-library"));
        assert!(!r.to_string().contains(f.root.path().to_str().unwrap()));
    }
}

fn rebuild_fixture() -> Fixture {
    let f = Fixture::new();
    f.sql("0001_initial.sql",&format!("CREATE TABLE accounts(id INTEGER PRIMARY KEY,email TEXT,legacy TEXT); CREATE TABLE children(id INTEGER PRIMARY KEY,account_id INTEGER REFERENCES accounts(id) ON DELETE CASCADE); CREATE TABLE audit(id INTEGER); {DEPENDENCIES}"));
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target().execute_batch("INSERT INTO accounts VALUES(-1,'negative','old'),(17,'retained','old'); INSERT INTO children VALUES(1,-1),(2,17)").unwrap();
    f.sql("0002_rebuild.toml", "sqlite_rebuild='accounts'");
    f
}
#[test]
fn sqlite_rebuild_rejects_implicit_dependency_loss_and_data_loss() {
    let f = rebuild_fixture();
    f.sql("0002_rebuild.sql","CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT NOT NULL); INSERT INTO replacement SELECT id,email FROM accounts; DROP VIEW account_emails; DROP TABLE accounts; ALTER TABLE replacement RENAME TO accounts;");
    let bad = f.command("plan", &[]);
    assert!(code(&bad, "DB_REBUILD_GUARD"), "{bad}");
    // No child rows remain to make the FK check catch this. The row-count guard
    // itself must detect loss that an empty replay cannot observe.
    f.target().execute_batch("DELETE FROM children").unwrap();
    f.sql("0002_rebuild.sql",&format!("CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT NOT NULL); INSERT INTO replacement SELECT id,email FROM accounts WHERE id>0; DROP VIEW account_emails; DROP TABLE accounts; ALTER TABLE replacement RENAME TO accounts; {DEPENDENCIES}"));
    let plan = f.plan("filtered.json");
    let failed = f.apply("filtered.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_REBUILD_GUARD"), "{failed}");
    assert_eq!(failed["progress"][1]["state"], "failed");
    let c = f.target();
    assert_eq!(
        c.query_row("SELECT count(*) FROM accounts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM children", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        c.query_row("SELECT legacy FROM accounts WHERE id=17", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "old"
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}
#[test]
fn sqlite_rebuild_foreign_key_verification_precedes_commit() {
    let f = rebuild_fixture();
    f.sql("0002_rebuild.sql",&format!("CREATE TABLE replacement(id INTEGER PRIMARY KEY,email TEXT NOT NULL); INSERT INTO replacement SELECT id+100,email FROM accounts; DROP VIEW account_emails; DROP TABLE accounts; ALTER TABLE replacement RENAME TO accounts; {DEPENDENCIES}"));
    let plan = f.plan("orphan.json");
    let r = f.apply("orphan.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_FOREIGN_KEY"), "{r}");
    assert_eq!(r["progress"][1]["state"], "failed");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM children", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        f.target()
            .query_row("SELECT email FROM accounts WHERE id=17", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "retained"
    );
}
#[test]
fn sqlite_ledger_tampering_blocks_writes() {
    for tamper in [
        "CREATE TRIGGER trap AFTER INSERT ON _ntnt_schema_migrations_v2 BEGIN DELETE FROM t; END;",
        "CREATE INDEX hidden ON _ntnt_schema_migrations_v2(id);",
        "ALTER TABLE _ntnt_schema_migrations_v2 ADD COLUMN extra TEXT;",
        "DROP TABLE _ntnt_schema_migrations_v2; CREATE VIEW _ntnt_schema_migrations_v2 AS SELECT 1;",
    ] {
        let f=Fixture::new(); f.sql("0001_initial.sql","CREATE TABLE t(id INTEGER);");
        let initial=f.plan("initial.json");
        assert_eq!(f.apply("initial.json",initial["plan_digest"].as_str())["outcome"],"pass");
        f.target().execute_batch("INSERT INTO t VALUES(17)").unwrap();
        f.sql("0002_add.sql","ALTER TABLE t ADD COLUMN label TEXT;");
        let plan=f.plan("upgrade.json"); f.target().execute_batch(tamper).unwrap();
        let r=f.apply("upgrade.json",plan["plan_digest"].as_str());
        assert!(code(&r,"DB_LEDGER_UNKNOWN"),"{r}");
        assert_eq!(f.target().query_row("SELECT count(*) FROM pragma_table_info('t')",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert_eq!(f.target().query_row("SELECT id FROM t",[],|r|r.get::<_,i64>(0)).unwrap(),17);
    }
}
#[test]
fn sqlite_competing_processes_do_not_execute_twice() {
    use sqlite_fixture::Running;
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    let plan = f.plan("plan.json");
    let digest = plan["plan_digest"].as_str().unwrap();
    let a = Running::spawn(&f, "plan.json", digest);
    let b = Running::spawn(&f, "plan.json", digest);
    let a = a.result();
    let b = b.result();
    assert_eq!(a["outcome"], "pass", "{a}");
    assert_eq!(b["outcome"], "pass", "{b}");
    let mut states = vec![
        a["progress"][0]["state"].as_str().unwrap(),
        b["progress"][0]["state"].as_str().unwrap(),
    ];
    states.sort();
    assert_eq!(states, vec!["already_applied", "committed"]);
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn sqlite_busy_writer_is_bounded_and_cleanup_allows_retry() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE t(id INTEGER);");
    let plan = f.plan("plan.json");
    let c = f.target();
    c.execute_batch("BEGIN IMMEDIATE").unwrap();
    let start = std::time::Instant::now();
    let r = f.apply("plan.json", plan["plan_digest"].as_str());
    assert!(code(&r, "DB_BUSY"), "{r}");
    assert!(start.elapsed() < std::time::Duration::from_secs(8));
    assert_eq!(
        c.query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        f.apply("plan.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
}
#[test]
fn sqlite_crashes_before_commit_and_before_ack_resume_from_real_prefix() {
    use sqlite_fixture::Running;
    for after_commit in [false, true] {
        let f = Fixture::new();
        f.sql("0001_initial.sql","CREATE TABLE a(id INTEGER); CREATE TABLE b(id INTEGER); CREATE TABLE logs(value INTEGER);");
        let initial = f.plan("initial.json");
        assert_eq!(
            f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
            "pass"
        );
        f.target().execute_batch("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO a SELECT x FROM n; INSERT INTO b SELECT id FROM a;").unwrap();
        if after_commit {
            f.sql(
                "0002_add.sql",
                "ALTER TABLE a ADD COLUMN label TEXT DEFAULT 'once';",
            );
        }
        f.sql(
            if after_commit {
                "0003_bulk.sql"
            } else {
                "0002_bulk.sql"
            },
            "INSERT INTO logs SELECT a.id+b.id FROM a CROSS JOIN b;",
        );
        let plan = f.plan("upgrade.json");
        let mut journal = f.target_path.as_os_str().to_os_string();
        journal.push("-journal");
        let journal = std::path::PathBuf::from(journal);
        assert!(!journal.exists());
        let mut child = Running::spawn(&f, "upgrade.json", plan["plan_digest"].as_str().unwrap());
        child.until(|| {
            if !journal.exists() {
                return false;
            }
            if !after_commit {
                return true;
            }
            let c = f.target();
            c.busy_timeout(std::time::Duration::from_millis(20))
                .unwrap();
            c.query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| {
                r.get::<_, i64>(0)
            })
            .is_ok_and(|n| n == 2)
        });
        child.kill();
        let c = f.target();
        assert_eq!(
            c.query_row("SELECT count(*) FROM logs", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            if after_commit { 2 } else { 1 }
        );
        c.execute_batch("DELETE FROM b").unwrap();
        let recovered = f.apply("upgrade.json", plan["plan_digest"].as_str());
        assert_eq!(recovered["outcome"], "pass", "{recovered}");
        if after_commit {
            assert_eq!(recovered["progress"][1]["state"], "already_applied");
            assert_eq!(
                c.query_row("SELECT label FROM a WHERE id=1", [], |r| r
                    .get::<_, String>(0))
                    .unwrap(),
                "once"
            );
        }
    }
}
#[test]
fn sqlite_wal_files_use_the_same_transactional_lifecycle() {
    let f = Fixture::new();
    f.target().execute_batch("PRAGMA journal_mode=WAL").unwrap();
    rusqlite::Connection::open(&f.scratch_path)
        .unwrap()
        .execute_batch("PRAGMA journal_mode=WAL")
        .unwrap();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    let plan = f.plan("wal.json");
    assert_eq!(
        plan["target"]["identity"]["settings"]["journal_mode"],
        "wal"
    );
    assert_eq!(
        f.apply("wal.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}

#[test]
fn sqlite_commit_busy_reconciles_on_a_fresh_locked_connection() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("INSERT INTO t VALUES(17)")
        .unwrap();
    f.sql(
        "0002_add.sql",
        "ALTER TABLE t ADD COLUMN label TEXT DEFAULT 'once';",
    );
    let plan = f.plan("upgrade.json");
    let reader = f.target();
    reader.execute_batch("BEGIN DEFERRED").unwrap();
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let failed = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_BUSY"), "{failed}");
    assert_eq!(failed["progress"][1]["phase"], "commit", "{failed}");
    assert_eq!(failed["reconciliation"]["outcome"], "pass", "{failed}");
    assert_eq!(failed["progress"][1]["state"], "failed");
    assert_eq!(failed["applied_count"], 1);
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM pragma_table_info('t')", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.apply("upgrade.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
    assert_eq!(
        f.target()
            .query_row("SELECT label FROM t WHERE id=17", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "once"
    );
}
#[test]
fn sqlite_statement_timeout_discards_session_and_preserves_prefix() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE a(id INTEGER); CREATE TABLE b(value INTEGER);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target().execute_batch("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<30000) INSERT INTO a SELECT x FROM n; INSERT INTO b SELECT id FROM a;").unwrap();
    f.sql("0002_add.sql", "ALTER TABLE a ADD COLUMN guarded TEXT;");
    f.sql("0002_add.toml","[requires]\nexpensive='SELECT NOT EXISTS (SELECT 1 FROM main.a WHERE (SELECT count(*) FROM main.b WHERE value > id) < 0)'");
    let plan = f.plan("upgrade.json");
    let start = std::time::Instant::now();
    let failed = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_TIMEOUT"), "{failed}");
    assert_ne!(failed["outcome"], "pass");
    assert_eq!(failed["assertions"]["executed"], 1);
    assert_eq!(failed["assertions"]["outcome"], "fail");
    assert_eq!(failed["progress"][0]["state"], "already_applied");
    assert!(start.elapsed() < std::time::Duration::from_secs(12));
    let c = f.target();
    c.busy_timeout(std::time::Duration::from_secs(2)).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM pragma_table_info('a')", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    c.execute_batch("DELETE FROM b").unwrap();
    assert_eq!(
        f.apply("upgrade.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
}
#[test]
fn sqlite_constraint_errors_do_not_disclose_sql_literals_or_row_values() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(value TEXT CHECK(value <> 'private_marker_245'));",
    );
    let initial = f.plan("initial.json");
    assert!(!initial.to_string().contains("private_marker_245"));
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("INSERT INTO t VALUES('retained')")
        .unwrap();
    f.sql("0002_bad.sql", "UPDATE t SET value='private_marker_245';");
    let plan = f.plan("bad.json");
    let failed = f.apply("bad.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_SQLITE"), "{failed}");
    assert!(!failed.to_string().contains("private_marker_245"));
    assert_eq!(failed["progress"][1]["state"], "failed");
    assert_eq!(
        f.target()
            .query_row("SELECT value FROM t", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "retained"
    );
}

fn reject_ctas_alias(sql: &str) {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE existing(id INTEGER);");
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("INSERT INTO existing VALUES(17)")
        .unwrap();
    f.sql("0002_copy.sql", sql);
    let rejected = f.command("plan", &[]);
    assert_ne!(rejected["outcome"], "pass", "{rejected}");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    // The narrow supported copy path separates declaration from row mapping,
    // so native preparation resolves only already-existing managed relations.
    f.sql(
        "0002_copy.sql",
        "CREATE TABLE copied(id INTEGER); INSERT INTO copied SELECT id FROM existing;",
    );
    let allowed = f.plan("allowed.json");
    let applied = f.apply("allowed.json", allowed["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    assert_eq!(
        f.target()
            .query_row("SELECT id FROM copied", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        17
    );
}
#[test]
fn sqlite_ctas_cannot_promote_a_pending_name_to_virtual_table_read_authority() {
    reject_ctas_alias("CREATE TABLE json_each AS SELECT value FROM json_each('[1,2]');");
}
#[test]
fn sqlite_ctas_cannot_read_ledger_pages_through_dbstat() {
    reject_ctas_alias("CREATE TABLE dbstat AS SELECT name,pageno,pagetype,ncell FROM dbstat;");
}
#[test]
fn sqlite_filename_stream_syntax_is_rejected_before_target_access() {
    let mut f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE existing(id INTEGER);");
    let plan = f.plan("plan.json");
    let original = f.target_path.clone();
    let mut stream = original.as_os_str().to_os_string();
    stream.push(":other");
    let stream = std::path::PathBuf::from(stream);
    // A real NTFS data stream on Windows; a colon filename on Unix. The endpoint
    // contract rejects stream syntax portably, before metadata/opening SQLite.
    std::fs::write(&stream, []).unwrap();
    f.target_path = stream;
    let inspected = f.command("status", &[]);
    assert!(code(&inspected, "DB_FILE_IDENTITY"), "{inspected}");
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert!(code(&applied, "DB_FILE_IDENTITY"), "{applied}");
    f.target_path = original;
    assert_eq!(
        f.apply("plan.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
}

fn hidden_rowid_trigger(alias: &str) {
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
    f.target()
        .execute_batch("INSERT INTO t VALUES('retained')")
        .unwrap();
    f.sql("0002_trigger.sql", &format!("CREATE TRIGGER hidden AFTER UPDATE OF {alias} ON t BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;"));
    let rejected = f.command("plan", &[]);
    assert!(code(&rejected, "DB_COVERAGE_INCOMPLETE"), "{rejected}");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    // A supported trigger on that same hidden alias must still work normally.
    f.sql("0002_trigger.sql", &format!("CREATE TRIGGER observed AFTER UPDATE OF {alias} ON t BEGIN INSERT INTO audit VALUES(new.rowid); END;"));
    let safe = f.plan("safe.json");
    let applied = f.apply("safe.json", safe["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    f.target()
        .execute_batch(&format!("UPDATE t SET {alias}=17"))
        .unwrap();
    assert_eq!(
        f.target()
            .query_row("SELECT id FROM audit", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        17
    );
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}
#[test]
fn sqlite_hidden_rowid_trigger_cannot_escape_catalog_authorization() {
    hidden_rowid_trigger("rowid");
}
#[test]
fn sqlite_hidden_underscore_rowid_trigger_cannot_escape_catalog_authorization() {
    hidden_rowid_trigger("_rowid_");
}
#[test]
fn sqlite_hidden_oid_trigger_cannot_escape_catalog_authorization() {
    hidden_rowid_trigger("oid");
}
#[test]
fn sqlite_without_rowid_trigger_cannot_escape_catalog_authorization() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY) WITHOUT ROWID;",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("INSERT INTO t VALUES(17)")
        .unwrap();
    f.sql(
        "0002_trigger.sql",
        "CREATE TRIGGER hidden AFTER UPDATE OF id ON t BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;",
    );
    let rejected = f.command("plan", &[]);
    assert!(code(&rejected, "DB_COVERAGE_INCOMPLETE"), "{rejected}");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.target()
            .query_row("SELECT id FROM t", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        17
    );
}

#[test]
fn sqlite_without_rowid_table_trigger_keeps_native_coverage() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE t(id INTEGER PRIMARY KEY) WITHOUT ROWID; CREATE TABLE audit(id INTEGER); CREATE TRIGGER observed AFTER UPDATE OF id ON t BEGIN INSERT INTO audit VALUES(new.id); END;");
    let plan = f.plan("plan.json");
    assert_eq!(
        f.apply("plan.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("INSERT INTO t VALUES(1); UPDATE t SET id=17")
        .unwrap();
    assert_eq!(
        f.target()
            .query_row("SELECT id FROM audit", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        17
    );
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}

#[test]
fn sqlite_hot_journal_inspection_is_readonly_and_names_the_recovery_action() {
    use sqlite_fixture::Running;
    use std::io::Read;
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE a(id INTEGER); CREATE TABLE b(id INTEGER); CREATE TABLE logs(value INTEGER);",
    );
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target().execute_batch("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<1000) INSERT INTO a SELECT x FROM n; INSERT INTO b SELECT id FROM a;").unwrap();
    f.sql(
        "0002_bulk.sql",
        "INSERT INTO logs SELECT a.id+b.id FROM a CROSS JOIN b;",
    );
    let plan = f.plan("upgrade.json");
    let mut path = f.target_path.as_os_str().to_os_string();
    path.push("-journal");
    let journal = std::path::PathBuf::from(path);
    let mut child = Running::spawn(&f, "upgrade.json", plan["plan_digest"].as_str().unwrap());
    child.until(|| {
        // Wait for SQLite's synced, nonzero journal header, not just creation
        // of a zero-header file that does not yet require native recovery.
        let mut header = [0; 8];
        std::fs::File::open(&journal)
            .and_then(|mut f| f.read_exact(&mut header))
            .is_ok()
            && header == [0xd9, 0xd5, 0x05, 0xf9, 0x20, 0xa1, 0x63, 0xd7]
    });
    child.kill();
    // Do not open a writable fixture handle: that would hide the actual state
    // an operator sees immediately after an interrupted migration.
    let before = std::fs::read(&f.target_path).unwrap();
    let journal_before = std::fs::read(&journal).unwrap();
    for command in ["status", "plan", "verify"] {
        let r = f.command(command, &[]);
        assert!(code(&r, "DB_SQLITE_RECOVERY_REQUIRED"), "{r}");
        let action = r["diagnostics"][0]["next_action"].as_str().unwrap();
        assert!(
            action.contains("approved") && action.contains("migrate"),
            "{action}"
        );
        assert_eq!(std::fs::read(&f.target_path).unwrap(), before);
        assert_eq!(std::fs::read(&journal).unwrap(), journal_before);
    }
    let recovered = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(recovered["outcome"], "pass", "{recovered}");
    assert_eq!(recovered["progress"][0]["state"], "already_applied");
    assert_eq!(recovered["progress"][1]["state"], "committed");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM logs", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1_000_000
    );
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}

fn existing_adoption_fixture(check: &str) -> (Fixture, Value) {
    use std::io::Write;
    let f = Fixture::new();
    f.sql(
        "0001_baseline.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY,value TEXT);",
    );
    f.target().execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY,value TEXT); INSERT INTO t VALUES(17,'retained')").unwrap();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(f.root.path().join("ntnt.toml"))
            .unwrap(),
        "\n[databases.app.adoption_checks]\ncurrent={check:?}"
    )
    .unwrap();
    let plan = f.command("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    (f, plan)
}
#[test]
fn sqlite_competing_adopters_record_one_checkpoint() {
    use sqlite_fixture::Running;
    let (f, plan) = existing_adoption_fixture("SELECT count(*)=1 FROM main.t");
    let digest = plan["plan_digest"].as_str().unwrap();
    let a = Running::spawn(&f, "adopt.json", digest);
    let b = Running::spawn(&f, "adopt.json", digest);
    let a = a.result();
    let b = b.result();
    assert_eq!(a["outcome"], "pass", "{a}");
    assert_eq!(b["outcome"], "pass", "{b}");
    let mut states = vec![
        a["progress"][0]["state"].as_str().unwrap(),
        b["progress"][0]["state"].as_str().unwrap(),
    ];
    states.sort();
    assert_eq!(states, vec!["already_adopted", "committed"]);
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.target()
            .query_row("SELECT value FROM t WHERE id=17", [], |r| r
                .get::<_, String>(0))
            .unwrap(),
        "retained"
    );
}
#[test]
fn sqlite_adoption_busy_commit_reconciles_without_inventing_execution() {
    let (f, plan) = existing_adoption_fixture("SELECT count(*)=1 FROM main.t");
    let reader = f.target();
    reader.execute_batch("BEGIN DEFERRED").unwrap();
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let failed = f.apply("adopt.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_BUSY"), "{failed}");
    assert_eq!(failed["reconciliation"]["outcome"], "pass", "{failed}");
    assert_eq!(failed["progress"][0]["state"], "failed");
    assert_eq!(failed["covered_count"], 0);
    assert_eq!(failed["applied_count"], 0);
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        f.target()
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='_ntnt_schema_migrations_v2'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let recovered = f.apply("adopt.json", plan["plan_digest"].as_str());
    assert_eq!(recovered["outcome"], "pass", "{recovered}");
    assert_eq!(recovered["covered_count"], 1);
    assert_eq!(recovered["adopted_count"], 1);
    assert_eq!(recovered["applied_count"], 0);
}

#[test]
fn sqlite_adoption_cancellation_and_timeout_leave_no_checkpoint() {
    use sqlite_fixture::Running;
    use std::io::Write;
    for kill in [true, false] {
        let f = Fixture::new();
        let schema = "CREATE TABLE a(id INTEGER); CREATE TABLE b(value INTEGER);";
        f.sql("0001_baseline.sql", schema);
        f.target().execute_batch(schema).unwrap();
        f.target().execute_batch("WITH RECURSIVE n(x) AS(VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<30000) INSERT INTO a SELECT x FROM n; INSERT INTO b SELECT id FROM a;").unwrap();
        writeln!(std::fs::OpenOptions::new().append(true).open(f.root.path().join("ntnt.toml")).unwrap(),"\n[databases.app.adoption_checks]\nexpensive='SELECT NOT EXISTS (SELECT 1 FROM main.a WHERE (SELECT count(*) FROM main.b WHERE value>id)<0)'").unwrap();
        let plan = f.command("plan", &["--adopt-through", "1", "--out", "adopt.json"]);
        assert_eq!(plan["outcome"], "pass", "{plan}");
        if kill {
            let mut child = Running::spawn(&f, "adopt.json", plan["plan_digest"].as_str().unwrap());
            let contender = f.target();
            contender.busy_timeout(std::time::Duration::ZERO).unwrap();
            child.until(|| match contender.execute_batch("BEGIN IMMEDIATE") {
                Ok(()) => {
                    contender.execute_batch("ROLLBACK").unwrap();
                    false
                }
                Err(e) => {
                    assert_eq!(
                        e.sqlite_error_code(),
                        Some(rusqlite::ErrorCode::DatabaseBusy)
                    );
                    true
                }
            });
            child.kill();
        } else {
            let start = std::time::Instant::now();
            let failed = f.apply("adopt.json", plan["plan_digest"].as_str());
            assert!(code(&failed, "DB_TIMEOUT"), "{failed}");
            assert_eq!(failed["assertions"]["executed"], 1);
            assert_eq!(failed["assertions"]["outcome"], "fail");
            assert!(start.elapsed() < std::time::Duration::from_secs(12));
        }
        let c = f.target();
        c.busy_timeout(std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(
            c.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='_ntnt_schema_migrations_v2'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
        assert_eq!(
            c.query_row("SELECT count(*) FROM a", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            30000
        );
        c.execute_batch("DELETE FROM b").unwrap();
        let recovered = f.apply("adopt.json", plan["plan_digest"].as_str());
        assert_eq!(recovered["outcome"], "pass", "{recovered}");
        assert_eq!(recovered["adopted_count"], 1);
        assert_eq!(recovered["applied_count"], 0);
    }
}

#[test]
fn sqlite_documented_application_adopts_upgrades_and_preserves_old_readers() {
    let f = Fixture::new();
    let example =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/database-adoption");
    for name in ["ntnt.toml", "app-v1.tnt", "app-v2.tnt"] {
        std::fs::copy(example.join(name), f.root.path().join(name)).unwrap();
    }
    for name in ["0001_existing.sql", "0010_locale.sql", "0010_locale.toml"] {
        std::fs::copy(
            example.join("migrations").join(name),
            f.root.path().join("migrations").join(name),
        )
        .unwrap();
    }
    let baseline = std::fs::read_to_string(example.join("migrations/0001_existing.sql")).unwrap();
    // The old application already exists, with real representative rows and no
    // NTNT ledger. This fixture does not fabricate historical migration entries.
    f.target().execute_batch(&baseline).unwrap();
    f.target().execute_batch("INSERT INTO customers VALUES(17,'Ada Example','ada@example.test'); INSERT INTO orders VALUES(80,17,2500),(81,17,4100); PRAGMA journal_mode=WAL;").unwrap();
    rusqlite::Connection::open(&f.scratch_path)
        .unwrap()
        .execute_batch("PRAGMA journal_mode=WAL")
        .unwrap();
    let app = |name: &str| {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_ntnt"))
            .args(["run", name])
            .env("EXAMPLE_DATABASE_FILE", &f.target_path)
            .env("NTNT_ENV", "development")
            .env("NTNT_SECRETS_PROVIDER", "env")
            .current_dir(f.root.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<Value>(&output.stdout).unwrap()
    };
    let old = app("app-v1.tnt");
    assert_eq!(
        old,
        serde_json::json!([
            {"id":17,"name":"Ada Example","email":"ada@example.test","total_cents":2500},
            {"id":17,"name":"Ada Example","email":"ada@example.test","total_cents":4100}
        ])
    );
    assert!(code(&f.command("status", &[]), "DB_UNRECOGNIZED_HISTORY"));
    let plan = f.command("plan", &["--adopt-through", "1", "--out", "adoption.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    assert_eq!(plan["plan"]["action"]["count"], 1);
    assert!(!plan.to_string().contains("ada@example.test"));
    let adopted = f.apply("adoption.json", plan["plan_digest"].as_str());
    assert_eq!(adopted["outcome"], "pass", "{adopted}");
    assert_eq!(adopted["assertions"]["executed"], 3);
    assert_eq!(app("app-v1.tnt"), old);
    let plan = f.plan("upgrade.json");
    let upgraded = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert_eq!(upgraded["outcome"], "pass", "{upgraded}");
    assert_eq!(upgraded["assertions"]["executed"], 2);
    let mut new = old.clone();
    for row in new.as_array_mut().unwrap() {
        row["locale"] = serde_json::json!("en");
    }
    assert_eq!(app("app-v2.tnt"), new);
    assert_eq!(app("app-v1.tnt"), old);
    assert_eq!(
        f.apply("upgrade.json", plan["plan_digest"].as_str())["assertions"]["executed"],
        0
    );
    let verified = f.command("verify", &[]);
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["history"]["adopted_count"], 1);
    assert_eq!(verified["history"]["applied_count"], 1);
    assert_eq!(verified["assertions"]["executed"], 0);
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM orders", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
}

#[test]
fn sqlite_upgrade_reconciliation_distinguishes_adopted_from_applied() {
    use std::io::Write;
    let f = Fixture::new();
    f.sql(
        "0001_existing.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    f.sql(
        "0010_expand.sql",
        "ALTER TABLE t ADD COLUMN value TEXT DEFAULT 'retained';",
    );
    f.target().execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY); ALTER TABLE t ADD COLUMN value TEXT DEFAULT 'retained'; INSERT INTO t(id) VALUES(17)").unwrap();
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(f.root.path().join("ntnt.toml"))
            .unwrap(),
        "\n[databases.app.adoption_checks]\ncurrent='SELECT count(*)=1 FROM main.t'"
    )
    .unwrap();
    let plan = f.command("plan", &["--adopt-through", "10", "--out", "adopt.json"]);
    assert_eq!(plan["outcome"], "pass", "{plan}");
    assert_eq!(
        f.apply("adopt.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql(
        "0020_upgrade.sql",
        "ALTER TABLE t ADD COLUMN label TEXT DEFAULT 'new';",
    );
    let plan = f.plan("upgrade.json");
    let reader = f.target();
    reader.execute_batch("BEGIN DEFERRED").unwrap();
    assert_eq!(
        reader
            .query_row("SELECT count(*) FROM t", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let failed = f.apply("upgrade.json", plan["plan_digest"].as_str());
    assert!(code(&failed, "DB_BUSY"), "{failed}");
    assert_eq!(failed["reconciliation"]["outcome"], "pass");
    assert_eq!(failed["applied_count"], 0);
    assert_eq!(failed["reconciliation"]["applied_count"], 0, "{failed}");
    assert_eq!(failed["reconciliation"]["adopted_count"], 2);
    assert_eq!(failed["reconciliation"]["imported_count"], 0);
    assert_eq!(failed["reconciliation"]["covered_count"], 2);
    reader.execute_batch("ROLLBACK").unwrap();
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM _ntnt_schema_migrations_v2", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM pragma_table_info('t')", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        f.apply("upgrade.json", plan["plan_digest"].as_str())["outcome"],
        "pass"
    );
}

#[test]
fn sqlite_human_adoption_output_names_the_checkpoint_not_a_null_migration() {
    let (f, plan) = existing_adoption_fixture("SELECT count(*)=1 FROM main.t");
    let human = |command: &str, args: &[&str], approval: Option<&str>| {
        let template = f.process(command);
        let mut process = std::process::Command::new(template.get_program());
        process.args(
            template
                .get_args()
                .filter(|arg| *arg != std::ffi::OsStr::new("--json")),
        );
        for (key, value) in template.get_envs() {
            if let Some(value) = value {
                process.env(key, value);
            } else {
                process.env_remove(key);
            }
        }
        process.current_dir(f.root.path()).args(args);
        if let Some(approval) = approval {
            process.env("DB_APPROVAL", approval);
        }
        let output = process.output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    };
    let review = human("verify", &["--against", "adopt.json"], None);
    assert!(
        review.contains("Adoption reference: adopt_baseline through ID 1"),
        "{review}"
    );
    assert!(review.contains("adoption not executed"), "{review}");
    let applied = human(
        "migrate",
        &["--plan", "adopt.json"],
        plan["plan_digest"].as_str(),
    );
    assert!(
        applied.contains("Adoption adopt_baseline through ID 1: committed"),
        "{applied}"
    );
    assert!(
        applied.contains("migration SQL executed: false"),
        "{applied}"
    );
    assert!(!applied.contains("Migration null"));
}
