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

/// The allowlist is re-resolved before every statement of a script: a name
/// dropped earlier must not keep authority and fall through to an eponymous
/// virtual table, while a legitimately recreated name regains it.
#[test]
fn sqlite_authorizer_refreshes_relations_before_each_statement() {
    let f = Fixture::new();
    for sql in [
        "CREATE TABLE json_each(value INTEGER); DROP TABLE json_each; CREATE TABLE t(value INTEGER); INSERT INTO t SELECT value FROM json_each('[1,2]');",
        "CREATE TABLE json_tree(value INTEGER); CREATE TABLE t(value INTEGER); DROP TABLE json_tree; CREATE VIEW v AS SELECT value FROM json_tree('[1]');",
    ] {
        f.sql("0001_dropped.sql", sql);
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{sql}: {r}");
        assert_eq!(f.target().query_row("SELECT count(*) FROM sqlite_schema",[],|r|r.get::<_,i64>(0)).unwrap(),0);
    }
    f.sql(
        "0001_dropped.sql",
        "CREATE TABLE a(x INTEGER); DROP TABLE a; CREATE TABLE a(y INTEGER); INSERT INTO a VALUES(1); CREATE VIEW v AS SELECT y FROM a;",
    );
    let r = f.command("plan", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
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

fn diagnostic<'a>(r: &'a Value, code: &str) -> &'a Value {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
        .unwrap_or_else(|| panic!("no {code} in {r}"))
}

fn journal_mode(path: &std::path::Path) -> String {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("PRAGMA journal_mode", [], |r| r.get(0))
        .unwrap()
}

fn set_journal_mode(connection: &rusqlite::Connection, mode: &str) {
    let set: String = connection
        .query_row(&format!("PRAGMA journal_mode={mode}"), [], |r| r.get(0))
        .unwrap();
    assert_eq!(set, mode);
}

/// #414: SQLite's date and time functions are allowed in migration programs, so
/// the usual timestamp defaults and an updated-at trigger plan, apply and verify.
#[test]
fn sqlite_time_defaults_and_triggers_plan_apply_and_verify() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE events(\
            id INTEGER PRIMARY KEY, \
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, \
            created_on TEXT NOT NULL DEFAULT CURRENT_DATE, \
            created_time TEXT NOT NULL DEFAULT CURRENT_TIME, \
            updated_at TEXT NOT NULL DEFAULT (datetime('now')), \
            epoch INTEGER NOT NULL DEFAULT (unixepoch()), \
            day REAL NOT NULL DEFAULT (julianday('now')), \
            stamp TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), \
            due TEXT NOT NULL DEFAULT (date('now', '+7 days')), \
            at TEXT NOT NULL DEFAULT (time('now')), \
            label TEXT); \
         CREATE TRIGGER events_touch AFTER UPDATE OF label ON events BEGIN \
            UPDATE events SET updated_at = datetime('now') WHERE id = new.id; END;",
    );
    let p = f.plan("time.json");
    let r = f.apply("time.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "pass", "{r}");
    let target = f.target();
    target
        .execute_batch("INSERT INTO events(label) VALUES('a'); UPDATE events SET label='b';")
        .unwrap();
    let unset: i64 = target
        .query_row(
            "SELECT count(*) FROM events WHERE created_at IS NULL OR updated_at IS NULL OR epoch IS NULL OR due IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(unset, 0);
    assert_eq!(f.command("verify", &[])["outcome"], "pass");
}

/// #414: a function outside the allowed set is reported as DB_SQL_UNSUPPORTED
/// naming the function, the statement and the migration, not as an opaque
/// DB_SQLITE code -1. Literals in the statement stay out of the report.
#[test]
fn sqlite_unsupported_function_names_the_function_and_statement() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE a(id INTEGER PRIMARY KEY); \
         CREATE TABLE b(id INTEGER PRIMARY KEY, token TEXT DEFAULT (hex(randomblob(16)) || 'private_marker_414'));",
    );
    let r = f.command("plan", &[]);
    assert_ne!(r["outcome"], "pass", "{r}");
    assert!(!code(&r, "DB_SQLITE"), "{r}");
    let d = diagnostic(&r, "DB_SQL_UNSUPPORTED");
    let message = d["message"].as_str().unwrap();
    assert!(message.contains("Statement 2"), "{message}");
    assert!(
        message.contains("`randomblob`") || message.contains("`hex`"),
        "{message}"
    );
    assert_eq!(d["source"], "0001_initial.sql", "{d}");
    assert!(!r.to_string().contains("private_marker_414"), "{r}");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

/// #413: `std/db/sqlite` connect() puts application databases in WAL mode, and a
/// scratch made with `touch` is not. An empty scratch takes the target's journal
/// mode, in either direction, instead of failing the replay.
#[test]
fn sqlite_empty_scratch_takes_the_target_journal_mode() {
    for (target_mode, scratch_mode) in [("wal", "delete"), ("delete", "wal")] {
        let f = Fixture::new();
        set_journal_mode(&f.target(), target_mode);
        set_journal_mode(
            &rusqlite::Connection::open(&f.scratch_path).unwrap(),
            scratch_mode,
        );
        f.sql(
            "0001_initial.sql",
            "CREATE TABLE notes(id INTEGER PRIMARY KEY, body TEXT NOT NULL);",
        );
        let p = f.plan("journal.json");
        assert_eq!(
            p["scratch"]["identity"]["settings"]["journal_mode"], target_mode,
            "{p}"
        );
        assert_eq!(journal_mode(&f.scratch_path), target_mode);
        let r = f.apply("journal.json", p["plan_digest"].as_str());
        assert_eq!(r["outcome"], "pass", "{r}");
        assert_eq!(f.command("verify", &[])["outcome"], "pass");
    }
}

/// #423 (sq-12): starting the application between plan and migrate puts the
/// target in WAL mode (std/db/sqlite connect() does), which makes the plan
/// stale. migrate and plan --against name that setting with both values, and
/// following the advice (plan, approve and migrate again) applies it.
#[test]
fn a_changed_target_setting_is_named_in_the_stale_plan() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE events(id INTEGER PRIMARY KEY, kind TEXT NOT NULL);",
    );
    let plan = f.plan("p1.json");
    set_journal_mode(&f.target(), "wal");
    let expected = "target setting journal_mode: plan delete, now wal";
    let r = f.apply("p1.json", plan["plan_digest"].as_str());
    let d = diagnostic(&r, "DB_PLAN_STALE");
    assert!(d["message"].as_str().unwrap().contains(expected), "{d}");
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .starts_with("Plan, review and approve again"),
        "{d}"
    );
    let r = f.command("plan", &["--against", "p1.json"]);
    assert_eq!(r["plan_validation"]["outcome"], "fail", "{r}");
    let d = diagnostic(&r, "DB_PLAN_STALE");
    assert!(d["message"].as_str().unwrap().contains(expected), "{d}");
    let again = f.plan("p2.json");
    let applied = f.apply("p2.json", again["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
}

/// #425 (sq-18, cli-13): DB_APPROVAL_REQUIRED says to set this database's
/// approval_secret, by name, to the reviewed plan's digest through the
/// provider and rerun the same plan; following it runs. A wrong approval
/// value is never shown.
#[test]
fn approval_refusals_say_how_to_approve_the_plan() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    let plan = f.plan("review.json");
    let digest = plan["plan_digest"].as_str().unwrap();
    for approval in [None, Some("ntnt425_wrong_approval")] {
        let r = f.apply("review.json", approval);
        let d = diagnostic(&r, "DB_APPROVAL_REQUIRED");
        let message = d["message"].as_str().unwrap();
        assert!(
            message.contains("DB_APPROVAL") && message.contains("database app"),
            "{d}"
        );
        let next_action = d["next_action"].as_str().unwrap();
        assert!(next_action.starts_with("Set DB_APPROVAL"), "{d}");
        for says in [digest, "secrets provider", "same plan"] {
            assert!(next_action.contains(says), "{says}: {d}");
        }
        assert!(!r.to_string().contains("ntnt425_wrong_approval"), "{r}");
    }
    // A provider that can read no secret says how to fix the provider.
    let owned = digest.to_owned();
    let r = report_with(&f, "migrate", &move |c| {
        c.args(["--plan", "review.json"])
            .env("DB_APPROVAL", &owned)
            .env("NTNT_ENV", "production");
    });
    let d = diagnostic(&r, "DB_APPROVAL_REQUIRED");
    assert!(
        d["message"].as_str().unwrap().starts_with(
            "Approval secret DB_APPROVAL cannot be read: the env secrets provider is disabled"
        ),
        "{d}"
    );
    assert!(
        d["next_action"].as_str().unwrap().contains("NTNT_ENV"),
        "{d}"
    );
    // Following the advice literally: the same plan, approved.
    let applied = f.apply("review.json", Some(digest));
    assert_eq!(applied["outcome"], "pass", "{applied}");
}

/// #425 (cli-13): a database that declares no approval_secret, or one whose
/// secret is not declared under [secrets], is told so, naming the database
/// or the secret; following that advice gets the plan approved and run.
#[test]
fn a_missing_approval_declaration_is_named() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    let manifest = f.root.path().join("ntnt.toml");
    let declared = std::fs::read_to_string(&manifest).unwrap();
    let without = declared
        .replace("approval_secret = \"DB_APPROVAL\"\n", "")
        .replace("[secrets.DB_APPROVAL]\n", "");
    std::fs::write(&manifest, &without).unwrap();
    let plan = f.plan("review.json");
    let r = f.apply("review.json", plan["plan_digest"].as_str());
    let d = diagnostic(&r, "DB_APPROVAL_REQUIRED");
    assert_eq!(
        d["message"],
        "Database app declares no approval_secret, so migrate cannot run any plan for it. No target or ledger writes were attempted.",
        "{d}"
    );
    let next_action = d["next_action"].as_str().unwrap();
    for says in [
        "approval_secret = \"APP_MIGRATION_APPROVAL\"",
        "[databases.app]",
        "[secrets.APP_MIGRATION_APPROVAL]",
        "plan, review and approve again",
    ] {
        assert!(next_action.contains(says), "{says}: {d}");
    }
    let migrate = |plan: &str, approval: &str| {
        let out = f
            .process("migrate")
            .args(["--plan", plan])
            .env("APP_MIGRATION_APPROVAL", approval)
            .output()
            .unwrap();
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    // Declared for the database, but not under [secrets].
    let half = without.replace(
        "migrations_dir = \"migrations\"\n",
        "migrations_dir = \"migrations\"\napproval_secret = \"APP_MIGRATION_APPROVAL\"\n",
    );
    std::fs::write(&manifest, &half).unwrap();
    let plan = f.plan("review-2.json");
    let r = migrate("review-2.json", plan["plan_digest"].as_str().unwrap());
    let d = diagnostic(&r, "DB_APPROVAL_REQUIRED");
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .starts_with("Approval secret APP_MIGRATION_APPROVAL is not declared under [secrets]"),
        "{d}"
    );
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("[secrets.APP_MIGRATION_APPROVAL]"),
        "{d}"
    );
    // Following both: declared, planned again and approved.
    std::fs::write(
        &manifest,
        format!("{half}[secrets.APP_MIGRATION_APPROVAL]\n"),
    )
    .unwrap();
    let plan = f.plan("review-3.json");
    let applied = migrate("review-3.json", plan["plan_digest"].as_str().unwrap());
    assert_eq!(applied["outcome"], "pass", "{applied}");
}

/// #425 (cli-01): a plan whose save fails after the replay (here: the saved
/// plan would exceed the 1 MiB readback limit) prints no plan digest in the
/// human report, since there is no saved plan to approve.
#[test]
fn a_plan_whose_save_failed_prints_no_digest() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    // About 1,000,000 bytes of intent IDs in the sidecar: within the 1 MiB
    // input limit, but over it in the pretty-printed saved plan.
    let id = "x".repeat(128);
    let ids = vec![format!("{id:?}"); 7600].join(",\n");
    f.sql("0001_initial.toml", &format!("intent_ids = [\n{ids}\n]\n"));
    let template = f.process("plan");
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
        .args(["--out", "big.json"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "{stdout}");
    assert!(stdout.contains("DB_OUTPUT_LIMIT"), "{stdout}");
    assert!(!stdout.contains("Plan digest"), "{stdout}");
    assert!(!f.root.path().join("big.json").exists());
}

/// #413: a scratch that is not empty keeps its journal mode and is reported as
/// not empty; nothing in it is changed.
#[test]
fn sqlite_nonempty_scratch_keeps_its_journal_mode() {
    let f = Fixture::new();
    set_journal_mode(&f.target(), "wal");
    rusqlite::Connection::open(&f.scratch_path)
        .unwrap()
        .execute_batch("CREATE TABLE leftover(id INTEGER)")
        .unwrap();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE notes(id INTEGER PRIMARY KEY);",
    );
    let r = f.command("plan", &[]);
    assert_ne!(r["outcome"], "pass", "{r}");
    assert!(code(&r, "DB_SCRATCH_NOT_EMPTY"), "{r}");
    assert!(!code(&r, "DB_SQLITE"), "{r}");
    assert_eq!(journal_mode(&f.scratch_path), "delete");
}

/// #413: a setting that cannot be matched automatically is named with both
/// values, and the cleanup after the refusal no longer replaces that diagnostic
/// with an opaque DB_SQLITE code 1.
#[test]
fn sqlite_scratch_setting_mismatch_names_the_setting_and_values() {
    let f = Fixture::new();
    rusqlite::Connection::open(&f.scratch_path)
        .unwrap()
        .execute_batch("PRAGMA page_size=1024; VACUUM;")
        .unwrap();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE notes(id INTEGER PRIMARY KEY);",
    );
    let r = f.command("plan", &[]);
    assert_ne!(r["outcome"], "pass", "{r}");
    assert!(!code(&r, "DB_SQLITE"), "{r}");
    let message = diagnostic(&r, "DB_SCRATCH_MISMATCH")["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        message.contains("page_size: target 4096, scratch 1024"),
        "{message}"
    );
}

/// #414: any failing statement is numbered, and an error that SQLite ties to a
/// token of the statement (here an unknown column) reports its real SQLite code
/// by name (#424), not -1.
#[test]
fn sqlite_failed_statement_is_numbered() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER); CREATE TABLE u(id INTEGER); INSERT INTO t SELECT missing FROM main.u;",
    );
    let r = f.command("plan", &[]);
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_SQLITE");
    let message = d["message"].as_str().unwrap();
    assert!(message.starts_with("Statement 3: "), "{message}");
    assert!(message.contains("(SQLITE_ERROR)"), "{message}");
    assert_eq!(d["source"], "0001_initial.sql", "{d}");
}

/// Longest the replay-timeout test waits for `ntnt db plan` to exit. The plan
/// stops at the replay's statement deadline after a few seconds; only a
/// deadline that never fires reaches this guard, which then fails the test
/// instead of leaving CI waiting on a query that would run for hours.
const HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(120);

/// Hang guard for the target statement-limit test: below the 90-second default,
/// so a configured limit that is not applied shows as a hang (#429).
const LIMIT_HANG_GUARD: std::time::Duration = std::time::Duration::from_secs(60);

/// The JSON `command` prints, failing the test if it outlives `guard`.
fn json_within_hang_guard(mut command: std::process::Command, guard: std::time::Duration) -> Value {
    use std::io::Read;
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    // Drain both pipes on their own threads, so a full pipe never stalls the child.
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes).map(|_| bytes)
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let started = std::time::Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > guard {
            let _ = child.kill();
            let _ = child.wait();
            panic!("`ntnt db` still ran after its hang guard ({guard:?}): a deadline never fired");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let (stdout, stderr) = (
        stdout.join().unwrap().unwrap(),
        stderr.join().unwrap().unwrap(),
    );
    serde_json::from_slice(&stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&stderr)))
}

/// A replay that runs out of time is reported as DB_TIMEOUT, not as its
/// cleanup's error. SQLite rolls back the transaction of an INSERT it
/// interrupts, so the cleanup finds none open; before 0.6.4 its failing
/// ROLLBACK replaced the timeout with DB_SQLITE code 1 (#413). When the
/// session's outer deadline fires first, the session is discarded and the
/// cleanup fails with DB_CANCELLED, listed after the timeout (that order is
/// unit-tested in `report.rs`; which deadline fires first is not fixed). The
/// statement is a cross join of 10^12 rows that never writes, so it always
/// reaches a deadline. The tables are qualified because SQLite gives a read
/// that names no column no database, which the migration authorizer refuses.
#[test]
fn sqlite_replay_timeout_is_not_masked_by_its_cleanup() {
    let f = Fixture::new();
    let rows = (1..=100)
        .map(|i| format!("({i})"))
        .collect::<Vec<_>>()
        .join(",");
    f.sql(
        "0001_initial.sql",
        &format!(
            "CREATE TABLE a(x INTEGER); INSERT INTO a VALUES {rows}; CREATE TABLE b(n INTEGER); \
             INSERT INTO b SELECT count(*) FROM main.a, main.a AS a2, main.a AS a3, main.a AS a4, main.a AS a5, main.a AS a6;"
        ),
    );
    let r = json_within_hang_guard(f.process("plan"), HANG_GUARD);
    assert_ne!(r["outcome"], "pass", "{r}");
    let codes: Vec<&str> = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap())
        .collect();
    assert_eq!(codes.first(), Some(&"DB_STATEMENT_LIMIT"), "{r}");
    let message = r["diagnostics"][0]["message"].as_str().unwrap();
    assert!(
        message.starts_with("Statement 4 did not finish within the 5-second replay limit"),
        "{message}"
    );
    // Anything after the timeout is the failed cleanup of a discarded session.
    assert!(codes[1..].iter().all(|c| *c == "DB_CANCELLED"), "{r}");
    let cleanup = if codes.len() == 1 { "pass" } else { "unknown" };
    assert_eq!(r["scratch"]["cleanup"], cleanup, "{r}");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

/// #414: where a function outside the allowed set is reported depends on when
/// SQLite compiles the code that calls it. A statement compiles its own calls
/// and those of a trigger it fires or a view it reads, so those are refused and
/// named. A trigger or view body nothing in the migration uses is compiled only
/// during inspection, where it is uncovered. Authored PRAGMAs never reach
/// SQLite. docs/DATABASE_MIGRATIONS.md states each case.
#[test]
fn sqlite_refusals_are_named_where_the_statement_compiles_them() {
    let trigger = "CREATE TABLE t(id INTEGER PRIMARY KEY, token TEXT); \
        CREATE TRIGGER t_token AFTER INSERT ON t BEGIN \
        UPDATE t SET token = hex(randomblob(8)) WHERE id = new.id; END;";
    let view = "CREATE TABLE t(id INTEGER PRIMARY KEY); \
        CREATE VIEW v AS SELECT id, random() AS r FROM t;";
    let cases = [
        (format!("{trigger} INSERT INTO t(id) VALUES (1);"), "DB_SQL_UNSUPPORTED", Some("Statement 3 uses SQLite function `hex`"), None),
        (trigger.to_owned(), "DB_COVERAGE_INCOMPLETE", None, Some(r#"table:"t""#)),
        (format!("{view} CREATE TABLE copy(id INTEGER, r INTEGER); INSERT INTO copy SELECT id, r FROM main.v;"), "DB_SQL_UNSUPPORTED", Some("Statement 4 uses SQLite function `random`"), None),
        (view.to_owned(), "DB_COVERAGE_INCOMPLETE", None, Some(r#"view:"v""#)),
        ("CREATE TABLE t(id INTEGER PRIMARY KEY); PRAGMA foreign_keys = ON;".to_owned(), "DB_REPLAY_UNSUPPORTED", None, None),
    ];
    for (sql, expected, message, object) in cases {
        let f = Fixture::new();
        f.sql("0001_initial.sql", &sql);
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{sql}: {r}");
        let d = diagnostic(&r, expected);
        assert_eq!(d["source"], "0001_initial.sql", "{sql}: {d}");
        if let Some(message) = message {
            assert!(
                d["message"].as_str().unwrap().starts_with(message),
                "{sql}: {d}"
            );
        }
        if let Some(object) = object {
            assert_eq!(d["objects"], serde_json::json!([object]), "{sql}: {d}");
        }
        assert_eq!(
            f.target()
                .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
}

/// #419: SQLite's DROP TABLE deletes every row first, which runs the ON DELETE
/// actions of the tables that reference it. Outside sqlite_rebuild mode a
/// migration that drops such a table is refused before it runs, naming each
/// child table and its action; the target keeps every row.
#[test]
fn sqlite_dropping_a_table_with_on_delete_children_requires_rebuild_mode() {
    let f = Fixture::new();
    f.sql(
        "0001_base.sql",
        "CREATE TABLE accounts(id INTEGER PRIMARY KEY, email TEXT NOT NULL); \
         CREATE TABLE posts(id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE); \
         CREATE TABLE comments(id INTEGER PRIMARY KEY, account_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL); \
         CREATE TABLE tags(id INTEGER PRIMARY KEY, account_id INTEGER REFERENCES accounts(id));",
    );
    let base = f.plan("base.json");
    assert_eq!(
        f.apply("base.json", base["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("INSERT INTO accounts VALUES(1, 'A@example.com'); INSERT INTO posts VALUES(1, 1); INSERT INTO comments VALUES(1, 1);")
        .unwrap();
    f.sql(
        "0002_rebuild.sql",
        "CREATE TABLE accounts_new(id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE); \
         INSERT INTO accounts_new SELECT id, lower(email) FROM accounts; \
         DROP TABLE accounts; \
         ALTER TABLE accounts_new RENAME TO accounts;",
    );
    let r = f.command("plan", &[]);
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_REBUILD_REQUIRED");
    let message = d["message"].as_str().unwrap();
    assert!(
        message.starts_with("Statement 3 drops table `accounts`"),
        "{message}"
    );
    assert!(
        message.contains("`comments` (ON DELETE SET NULL)"),
        "{message}"
    );
    assert!(message.contains("`posts` (ON DELETE CASCADE)"), "{message}");
    assert!(!message.contains("tags"), "{message}");
    assert_eq!(d["source"], "0002_rebuild.sql", "{d}");
    let kept: i64 = f
        .target()
        .query_row("SELECT (SELECT count(*) FROM posts) + (SELECT count(*) FROM comments WHERE account_id = 1)", [], |r| r.get(0))
        .unwrap();
    assert_eq!(kept, 2);
}

/// #419: the refusal is only for drops that would run ON DELETE actions. A
/// parent dropped after its children, a child without an action, and a
/// self-reference stay allowed.
#[test]
fn sqlite_drops_that_run_no_on_delete_actions_stay_allowed() {
    let f = Fixture::new();
    f.sql(
        "0001_base.sql",
        "CREATE TABLE parents(id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parents(id) ON DELETE CASCADE); \
         CREATE TABLE children(id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parents(id) ON DELETE CASCADE); \
         CREATE TABLE others(id INTEGER PRIMARY KEY, parent_id INTEGER REFERENCES parents(id));",
    );
    let base = f.plan("base.json");
    assert_eq!(
        f.apply("base.json", base["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql(
        "0002_drop.sql",
        "DROP TABLE others; DROP TABLE children; DROP TABLE parents;",
    );
    let plan = f.plan("drop.json");
    let applied = f.apply("drop.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
}

/// #429: on a target, a SQLite migration runs under the plan's statement limit
/// (`statement_seconds`, 90 seconds by default) instead of a fixed 5 seconds.
/// The plan binds a declared limit; a statement past it fails with
/// DB_STATEMENT_LIMIT naming it, nothing from that migration is committed, and
/// a changed limit makes the plan stale. The statement is a cross join of
/// 2.7 * 10^13 rows on the target's data (instant on the empty scratch), so it
/// always reaches the limit.
#[test]
fn sqlite_target_migration_runs_under_the_plans_statement_limit() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE a(x INTEGER); CREATE TABLE b(n INTEGER);",
    );
    let initial = f.plan("initial.json");
    assert!(
        initial["plan"]["execution"].get("limits").is_none(),
        "{initial}"
    );
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.target()
        .execute_batch("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x + 1 FROM n WHERE x < 30000) INSERT INTO a SELECT x FROM n;")
        .unwrap();
    let manifest = f.root.path().join("ntnt.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        format!("{text}\n[databases.app.limits]\nstatement_seconds = 2\n"),
    )
    .unwrap();
    f.sql(
        "0002_count.sql",
        "INSERT INTO b SELECT count(*) FROM main.a, main.a AS a2, main.a AS a3;",
    );
    let plan = f.plan("limit.json");
    assert_eq!(
        plan["plan"]["execution"]["limits"],
        serde_json::json!({"statement_seconds": 2}),
        "{plan}"
    );
    let mut migrate = f.process("migrate");
    migrate
        .args(["--plan", "limit.json"])
        .env("DB_APPROVAL", plan["plan_digest"].as_str().unwrap());
    let r = json_within_hang_guard(migrate, LIMIT_HANG_GUARD);
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_STATEMENT_LIMIT");
    let message = d["message"].as_str().unwrap();
    assert!(
        message.starts_with("Statement 1 did not finish within the 2-second statement limit"),
        "{message}"
    );
    assert_eq!(d["source"], "0002_count.sql", "{d}");
    assert_eq!(
        f.target()
            .query_row("SELECT count(*) FROM b", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    // The limit is part of the plan: a different declared limit makes it stale.
    std::fs::write(
        &manifest,
        format!("{text}\n[databases.app.limits]\nstatement_seconds = 3\n"),
    )
    .unwrap();
    let stale = f.apply("limit.json", plan["plan_digest"].as_str());
    assert!(code(&stale, "DB_PLAN_STALE"), "{stale}");
}

/// A fixture command's report, after `configure` changes its environment.
fn report_with(
    f: &Fixture,
    command: &str,
    configure: &dyn Fn(&mut std::process::Command),
) -> Value {
    let mut process = f.process(command);
    configure(&mut process);
    let out = process.output().unwrap();
    serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)))
}

/// #422 (cli-12): DB_SECRET names the connection secret it could not
/// resolve and its role, never its value, and says which cause applies;
/// each cause has its own advice, and following it resolves the refusal.
#[test]
fn unresolved_connection_secrets_name_the_secret_role_and_cause() {
    type Configure = Box<dyn Fn(&mut std::process::Command)>;
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE main.t(id INTEGER);");
    let target = f.target_path.to_str().unwrap().to_owned();
    let scratch = f.scratch_path.to_str().unwrap().to_owned();
    let mut cases: Vec<(Configure, &str, &str)> = vec![
        (
            Box::new(|c| {
                c.env_remove("DB_TARGET");
            }),
            "Target connection secret DB_TARGET is unset or empty in the environment (provider env).",
            "Set the environment variable DB_TARGET",
        ),
        (
            Box::new(|c| {
                c.env("DB_TARGET", "");
            }),
            "Target connection secret DB_TARGET is unset or empty in the environment (provider env).",
            "Set the environment variable DB_TARGET",
        ),
        (
            Box::new(|c| {
                c.env_remove("DB_SCRATCH");
            }),
            "Scratch connection secret DB_SCRATCH is unset or empty in the environment (provider env).",
            "Set the environment variable DB_SCRATCH",
        ),
        (
            Box::new(|c| {
                c.env("NTNT_ENV", "production");
            }),
            "Target connection secret DB_TARGET cannot be read: the env secrets provider is disabled when NTNT_ENV is production.",
            "set NTNT_ENV=development",
        ),
        (
            Box::new(|c| {
                c.env("NTNT_SECRETS_PROVIDER", "evn");
            }),
            "Target connection secret DB_TARGET cannot be read: NTNT_SECRETS_PROVIDER names an unsupported provider (supported: env, and unix-socket on Unix).",
            "Set NTNT_SECRETS_PROVIDER to env",
        ),
    ];
    #[cfg(unix)]
    cases.push((
        Box::new(|c| {
            c.env("NTNT_SECRETS_PROVIDER", "unix-socket")
                .env_remove("NTNT_SECRETS_SOCKET_ENDPOINTS");
        }),
        "Target connection secret DB_TARGET cannot be read: the unix-socket secrets provider's configuration is invalid.",
        "NTNT_SECRETS_SOCKET_ENDPOINTS",
    ));
    let mut advice = std::collections::BTreeSet::new();
    for (configure, message, next_action) in &cases {
        let r = report_with(&f, "status", &**configure);
        let d = diagnostic(&r, "DB_SECRET");
        assert_eq!(d["message"], *message, "{d}");
        let action = d["next_action"].as_str().unwrap();
        assert!(action.contains(next_action), "{d}");
        advice.insert(action.to_owned());
        // Names only: neither file path is ever shown.
        let text = r.to_string();
        assert!(!text.contains(&target) && !text.contains(&scratch), "{r}");
    }
    // Each cause has its own advice (the two unset targets share theirs).
    assert_eq!(advice.len(), cases.len() - 1, "{advice:?}");
    // Following the advice, with the fixture's environment, inspection passes.
    assert_eq!(report_with(&f, "status", &|_| {})["outcome"], "pass");

    // A secret the manifest does not declare.
    let manifest = f.root.path().join("ntnt.toml");
    let declared = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(&manifest, declared.replace("[secrets.DB_SCRATCH]\n", "")).unwrap();
    let r = report_with(&f, "status", &|_| {});
    let d = diagnostic(&r, "DB_SECRET");
    assert_eq!(
        d["message"],
        "Scratch connection secret DB_SCRATCH is not declared under [secrets] in the manifest.",
        "{d}"
    );
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("[secrets.DB_SCRATCH]"),
        "{d}"
    );
    std::fs::write(&manifest, &declared).unwrap();
    assert_eq!(report_with(&f, "status", &|_| {})["outcome"], "pass");

    // migrate names the target secret the same way.
    let plan = f.plan("plan.json");
    let digest = plan["plan_digest"].as_str().unwrap().to_owned();
    let r = report_with(&f, "migrate", &move |c| {
        c.args(["--plan", "plan.json"])
            .env("DB_APPROVAL", &digest)
            .env_remove("DB_TARGET");
    });
    let d = diagnostic(&r, "DB_SECRET");
    assert_eq!(
        d["message"],
        "Target connection secret DB_TARGET is unset or empty in the environment (provider env).",
        "{d}"
    );
}

/// #422 (sq-14, cli-14): DB_FILE_IDENTITY names the role and its secret,
/// as the manifest names it, and the rule the path broke, never the path;
/// following its advice resolves it.
#[test]
fn file_identity_refusals_name_the_secret_and_the_rule() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE main.t(id INTEGER);");
    let root = f.target_path.parent().unwrap().to_path_buf();
    // The first DB_FILE_IDENTITY of `command` with `secret` set to `value`,
    // which the report never shows.
    let refused = |command: &str, secret: &'static str, value: std::path::PathBuf| {
        let shown = value.to_str().unwrap().to_owned();
        let r = report_with(&f, command, &move |c| {
            c.env(secret, &value);
        });
        assert!(!r.to_string().contains(&shown), "{r}");
        diagnostic(&r, "DB_FILE_IDENTITY").clone()
    };
    let passes = |command: &str| {
        let r = report_with(&f, command, &|_| {});
        assert_eq!(r["outcome"], "pass", "{r}");
    };
    let says = |d: &Value, message: &[&str], next_action: &str| {
        for fragment in message {
            assert!(
                d["message"].as_str().unwrap().contains(fragment),
                "{fragment:?} missing: {d}"
            );
        }
        assert!(
            d["next_action"].as_str().unwrap().contains(next_action),
            "{next_action:?} missing: {d}"
        );
    };

    // A relative scratch path, the most common first-run mistake.
    let d = refused("plan", "DB_SCRATCH", "relative-replay.sqlite".into());
    says(
        &d,
        &["Scratch connection secret DB_SCRATCH", "a relative path"],
        "Set DB_SCRATCH to the file's absolute path",
    );
    passes("plan");

    // A target file that does not exist yet.
    let missing = root.join("missing.sqlite");
    let d = refused("status", "DB_TARGET", missing.clone());
    says(
        &d,
        &["Target connection secret DB_TARGET", "does not exist"],
        "touch",
    );
    std::fs::File::create(&missing).unwrap();
    let r = report_with(&f, "status", &|c| {
        c.env("DB_TARGET", &missing);
    });
    assert_eq!(r["outcome"], "pass", "{r}");
    std::fs::remove_file(&missing).unwrap();

    // A directory.
    let d = refused("status", "DB_TARGET", root.join("migrations"));
    says(
        &d,
        &["Target connection secret DB_TARGET", "not a regular file"],
        "regular SQLite file",
    );

    // A second hard link to the target.
    let alias = root.join("alias.sqlite");
    std::fs::hard_link(&f.target_path, &alias).unwrap();
    let r = report_with(&f, "status", &|_| {});
    let d = diagnostic(&r, "DB_FILE_IDENTITY");
    says(
        d,
        &[
            "Target connection secret DB_TARGET",
            "more than one hard link",
        ],
        "single link",
    );
    std::fs::remove_file(&alias).unwrap();
    passes("status");

    #[cfg(unix)]
    {
        // A link to the file, and a path through a linked directory.
        let link = root.join("link.sqlite");
        std::os::unix::fs::symlink(&f.target_path, &link).unwrap();
        let d = refused("status", "DB_TARGET", link);
        says(
            &d,
            &["Target connection secret DB_TARGET", "symbolic link"],
            "pwd -P",
        );
        let linked = root.join("linked-dir");
        std::os::unix::fs::symlink(&root, &linked).unwrap();
        let d = refused("plan", "DB_SCRATCH", linked.join("scratch.sqlite"));
        says(
            &d,
            &["Scratch connection secret DB_SCRATCH", "symbolic link"],
            "pwd -P",
        );
        passes("plan");
    }
}

/// #422 (cli-04): DB_ADOPTION_INPUT says which adoption requirement failed,
/// with advice for it, and following the advice gets the adoption planned.
#[test]
fn adoption_refusals_say_which_requirement_failed() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY);",
    );
    // An existing database that matches the migration but has no ledger.
    f.target()
        .execute_batch("CREATE TABLE t(id INTEGER PRIMARY KEY);")
        .unwrap();
    let plan = |args: &[&str]| f.command("plan", args);
    let refusal = |r: &Value| diagnostic(r, "DB_ADOPTION_INPUT").clone();

    // No adoption checks declared.
    let d = refusal(&plan(&["--adopt-through", "1"]));
    assert_eq!(
        d["message"],
        "Adoption needs at least one named check, and [databases.app.adoption_checks] declares none. No history is inferred or repaired.",
        "{d}"
    );
    let next_action = d["next_action"].as_str().unwrap();
    assert!(
        next_action.contains("[databases.app.adoption_checks]") && next_action.contains("main."),
        "{d}"
    );
    let manifest = f.root.path().join("ntnt.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str("[databases.app.adoption_checks]\nids_present = \"SELECT NOT EXISTS (SELECT 1 FROM main.t WHERE id IS NULL)\"\n");
    std::fs::write(&manifest, text).unwrap();

    // An ID that is not in the migrations directory.
    let d = refusal(&plan(&["--adopt-through", "7"]));
    assert_eq!(
        d["message"], "Migration ID 7 is not in migrations; the highest ID there is 1.",
        "{d}"
    );
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("--adopt-through 1"),
        "{d}"
    );

    // Following both pieces of advice, adoption is planned.
    let adoption = f.command("plan", &["--adopt-through", "1", "--out", "adoption.json"]);
    assert_eq!(adoption["outcome"], "pass", "{adoption}");
    let adopted = f.apply("adoption.json", adoption["plan_digest"].as_str());
    assert_eq!(adopted["outcome"], "pass", "{adopted}");

    // A database whose history is already recorded is not eligible.
    let d = refusal(&plan(&["--adopt-through", "1"]));
    assert_eq!(
        d["message"],
        "The target already has an ntnt migration ledger (1 entry), so it is not eligible for adoption, which only records a baseline for a database without one.",
        "{d}"
    );
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("without --adopt-through"),
        "{d}"
    );
    assert_eq!(plan(&[])["outcome"], "pass");
}
