use super::{
    attempt_state, code, concurrent_fixture, index_state, plan_digest, reconcile, Fixture,
};
use serde_json::{json, Value};

fn save(path: &std::path::Path, plan: &mut Value) {
    plan["plan_digest"] = json!(plan_digest(&plan["plan"]));
    std::fs::write(path, serde_json::to_vec(plan).unwrap()).unwrap();
}

#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn compact_partial_index_reconciles_with_its_pre_native_plan_but_cannot_restart() {
    // Within the parser's token limit as authored, over it when the old
    // planner inserted a space after each comma before classifying the SQL.
    // Repeat the same value: the token count is what exercises compatibility.
    // Distinct constants can make the predicate too large for pg_index on PG16.
    let values = vec!["1"; 1500].join(",");
    let sql = format!("CREATE UNIQUE INDEX CONCURRENTLY t_id ON app.t(id) WHERE id IN ({values});");
    let (f, _) = concurrent_fixture(&sql);
    let path = f.root.path().join("upgrade.json");
    let mut old: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    // Exact analysis in the pre-native descriptor, not simply a native
    // plan with its replay marker removed. Keep this fixture independent
    // of the implementation used to validate the descriptor.
    let legacy = json!({
        "operations": ["create_index"], "hazards": ["UNANALYZED_EFFECTS"],
        "unknowns": ["LOCKS_NOT_PREDICTED", "SQL_NOT_PARSED"],
        "replay_supported": true, "non_transactional": true
    });
    let pending = &mut old["plan"]["pending"][0];
    assert_ne!(pending["analysis"], legacy);
    pending["analysis"] = legacy;
    old["plan"]["execution"]
        .as_object_mut()
        .unwrap()
        .remove("replay");

    // An interrupted attempt: PostgreSQL has left an invalid index, and
    // migrate did not get to clean it up or settle its running ledger row.
    let mut db = f.target();
    db.execute("INSERT INTO _ntnt.schema_migrations_v2 VALUES(2,'2','0002_index.sql',$1,'running','app','app')",
        &[&old["plan"]["pending"][0]["checksum"].as_str().unwrap()]).unwrap();
    db.batch_execute("INSERT INTO app.t VALUES (1)").unwrap();
    assert_eq!(
        db.batch_execute(&sql).unwrap_err().code(),
        Some(&postgres::error::SqlState::UNIQUE_VIOLATION)
    );
    assert_eq!(index_state(&f, "t_id"), Some(false));

    // Even with its own digest and approval, an analysis neither planner
    // would have produced cannot authorize dropping the invalid index.
    let mut wrong = old.clone();
    wrong["plan"]["pending"][0]["analysis"]["unknowns"] = json!([]);
    save(&path, &mut wrong);
    let r = reconcile(&f, &wrong);
    assert!(code(&r, "DB_PLAN_STALE"), "{r}");
    assert_eq!(index_state(&f, "t_id"), Some(false));
    assert_eq!(attempt_state(&f).as_deref(), Some("running"));

    save(&path, &mut old);
    let r = reconcile(&f, &old);
    assert_eq!(r["reconciliation"]["resolution"], "rolled_back", "{r}");
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(index_state(&f, "t_id"), None);
    assert_eq!(attempt_state(&f), None);

    let r = f.apply("upgrade.json", old["plan_digest"].as_str());
    assert!(code(&r, "DB_PLAN_STALE"), "{r}");
    assert_eq!(index_state(&f, "t_id"), None);
    assert_eq!(attempt_state(&f), None);
}

/// Pin the 0.6.3 *scope* without pinning a PG-major-specific digest. The
/// first fixture supplies exactly the catalog a managed-schema-only reader
/// saw. The approval, ledger and target identity belong to the second one.
#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn legacy_scope_needs_the_whole_applied_prefix_even_for_a_valid_index() {
    let index = "CREATE INDEX CONCURRENTLY t_id ON app.t(id);";
    let (_control, narrow) = concurrent_fixture(index);
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER); CREATE SCHEMA audit; CREATE TABLE audit.log(id INTEGER); DO $$ BEGIN CREATE TABLE public.receipts(id INTEGER); END $$;");
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql("0002_index.sql", index);
    let mut old = f.isolated_plan("upgrade.json");
    let plan = &mut old["plan"];
    let execution = plan["execution"].as_object_mut().unwrap();
    execution.remove("replay").unwrap();
    execution.remove("outside_objects").unwrap();
    let prints = &narrow["plan"]["execution"]["prefix_fingerprints"];
    assert_ne!(execution["prefix_fingerprints"][1], prints[1]);
    execution.insert("prefix_fingerprints".into(), prints.clone());
    for key in ["expected_base_digest", "base_catalog_digest"] {
        plan["binding"][key] = prints[1].clone();
    }
    plan["candidate"]["fingerprint"] = prints[2].clone();
    save(&f.root.path().join("upgrade.json"), &mut old);
    let mut db = f.target();
    db.execute("INSERT INTO _ntnt.schema_migrations_v2 VALUES(2,'2','0002_index.sql',$1,'running','app','app')", &[&old["plan"]["pending"][0]["checksum"].as_str().unwrap()]).unwrap();
    db.batch_execute(index).unwrap();
    for (drift, repair) in [
        (
            "ALTER SCHEMA audit RENAME TO elsewhere",
            "ALTER SCHEMA elsewhere RENAME TO audit",
        ),
        (
            "ALTER TABLE public.receipts ADD COLUMN extra int",
            "ALTER TABLE public.receipts DROP COLUMN extra",
        ),
    ] {
        db.batch_execute(drift).unwrap();
        let refused = reconcile(&f, &old);
        assert!(code(&refused, "DB_RECONCILE_MISMATCH"), "{refused}");
        assert_eq!(refused["reconciliation"]["observed_index"], "valid");
        assert_eq!(attempt_state(&f).as_deref(), Some("running"));
        assert_eq!(index_state(&f, "t_id"), Some(true));
        db.batch_execute(repair).unwrap();
    }
    let settled = reconcile(&f, &old);
    assert_eq!(
        settled["reconciliation"]["resolution"], "completed",
        "{settled}"
    );
    assert_eq!(settled["scratch"]["cleanup"], "pass");
    assert_eq!(attempt_state(&f).as_deref(), Some("applied"));
}

/// 0.6.3 reviewed the original SQL and checksums, before adding per-statement
/// outside-scope notes. An invalid index is real PostgreSQL state; a missing
/// annotation may not strand it, nor can resolving it authorize a new build.
#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn pre_scope_notes_plan_can_settle_an_invalid_build_but_cannot_start_one() {
    let f = Fixture::new();
    f.sql("0001_initial.sql", "CREATE TABLE app.t(id INTEGER);");
    let initial = f.plan("initial.json");
    assert_eq!(
        f.apply("initial.json", initial["plan_digest"].as_str())["outcome"],
        "pass"
    );
    // Give the last migration an observable effect. Reconcile may settle the
    // earlier build, but must not execute this later statement.
    f.target()
        .batch_execute("GRANT CREATE ON SCHEMA public TO PUBLIC")
        .unwrap();
    let index_sql = "CREATE UNIQUE INDEX CONCURRENTLY t_id ON app.t(id);";
    f.sql("0002_index.sql", index_sql);
    f.sql(
        "0003_scope.sql",
        "REVOKE CREATE ON SCHEMA public FROM PUBLIC;",
    );
    let mut old = f.isolated_plan("upgrade.json");
    let path = f.root.path().join("upgrade.json");
    old["plan"]["execution"]
        .as_object_mut()
        .unwrap()
        .remove("replay");
    // Captured from the official 0.6.3 planner; retain every other warning.
    old["plan"]["pending"][1]["analysis"] = json!({
        "hazards": ["AUTHORIZATION_CHANGE"], "operations": ["authorization"],
        "replay_supported": true, "unknowns": ["EFFECTS_NOT_ANALYZED"]
    });
    save(&path, &mut old);
    let mut db = f.target();
    db.execute("INSERT INTO _ntnt.schema_migrations_v2 VALUES(2,'2','0002_index.sql',$1,'running','app','app')", &[&old["plan"]["pending"][0]["checksum"].as_str().unwrap()]).unwrap();
    db.batch_execute("INSERT INTO app.t VALUES (7),(7)")
        .unwrap();
    assert_eq!(
        db.batch_execute(index_sql).unwrap_err().code(),
        Some(&postgres::error::SqlState::UNIQUE_VIOLATION)
    );
    let unchanged = || {
        assert_eq!(index_state(&f, "t_id"), Some(false));
        assert_eq!(attempt_state(&f).as_deref(), Some("running"));
    };
    unchanged();
    let denied = f
        .process("reconcile")
        .args(["--plan", "upgrade.json"])
        .output()
        .unwrap();
    let denied: Value = serde_json::from_slice(&denied.stdout).unwrap();
    assert!(code(&denied, "DB_APPROVAL_REQUIRED"), "{denied}");
    unchanged();
    // Missing scope evidence requires a scratch reference, never a target
    // guess or a downgrade, even with exact approval.
    let unavailable = f
        .process("reconcile")
        .env_remove("DB_SCRATCH")
        .env("DB_APPROVAL", old["plan_digest"].as_str().unwrap())
        .args(["--plan", "upgrade.json"])
        .output()
        .unwrap();
    let unavailable: Value = serde_json::from_slice(&unavailable.stdout).unwrap();
    assert!(code(&unavailable, "DB_SECRET"), "{unavailable}");
    unchanged();
    // The fallback is exact; it doesn't discard unexpected historical analysis.
    let mut altered = old.clone();
    altered["plan"]["pending"][1]["analysis"]["hazards"] = json!([]);
    save(&path, &mut altered);
    assert!(code(&reconcile(&f, &altered), "DB_PLAN_STALE"));
    unchanged();
    save(&path, &mut old);
    let verify = f.command_args("verify", &["--against", "upgrade.json"]);
    assert_eq!(verify["plan_validation"]["outcome"], "fail");
    let apply = f.apply("upgrade.json", old["plan_digest"].as_str());
    assert!(code(&apply, "DB_PLAN_STALE"), "{apply}");
    unchanged();
    // A matching digest does not excuse unrelated catalog drift.
    db.batch_execute("ALTER TABLE app.t ADD COLUMN unrelated text")
        .unwrap();
    let drifted = reconcile(&f, &old);
    assert!(code(&drifted, "DB_RECONCILE_MISMATCH"), "{drifted}");
    unchanged();
    db.batch_execute("ALTER TABLE app.t DROP COLUMN unrelated")
        .unwrap();
    let settled = reconcile(&f, &old);
    assert_eq!(
        settled["reconciliation"]["resolution"], "rolled_back",
        "{settled}"
    );
    assert_eq!(index_state(&f, "t_id"), None);
    assert_eq!(attempt_state(&f), None);
    assert_eq!(
        db.query_one("SELECT count(*) FROM app.t WHERE id=7", &[])
            .unwrap()
            .get::<_, i64>(0),
        2
    );
    assert!(db.query_one("SELECT EXISTS(SELECT FROM pg_namespace n, aclexplode(n.nspacl) a WHERE n.nspname='public' AND a.grantee=0 AND a.privilege_type='CREATE')", &[]).unwrap().get::<_,bool>(0));
    assert!(code(
        &f.apply("upgrade.json", old["plan_digest"].as_str()),
        "DB_PLAN_STALE"
    ));
    assert_eq!(index_state(&f, "t_id"), None);
    assert_eq!(attempt_state(&f), None);
}
