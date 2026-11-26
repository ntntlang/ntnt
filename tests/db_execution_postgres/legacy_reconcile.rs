use super::{attempt_state, code, concurrent_fixture, index_state, plan_digest, reconcile};
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
