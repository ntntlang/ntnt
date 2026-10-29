//! Real fresh/populated rehearsal uses the production CLI and executor.
#[path = "support/db_fixture.rs"]
mod db_fixture;
use db_fixture::Fixture;
use serde_json::Value;

fn code(r: &Value, code: &str) -> bool {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|d| d["code"] == code)
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn exact_authority_fresh_execution_and_unchanged_rerun() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE app.accounts(id INTEGER PRIMARY KEY, label TEXT DEFAULT 'a; b');",
    );
    let p = f.plan("plan.json");
    for approval in [None, Some("not-the-reviewed-digest")] {
        let r = f.apply("plan.json", approval);
        assert!(code(&r, "DB_APPROVAL_REQUIRED"), "{r}");
        assert!(!f
            .target()
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM pg_namespace WHERE nspname='_ntnt')",
                &[]
            )
            .unwrap()
            .get::<_, bool>(0));
    }
    let digest = p["plan_digest"].as_str().unwrap();
    let r = f.apply("plan.json", Some(digest));
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["progress"][0]["state"], "committed", "{r}");
    f.target()
        .batch_execute("INSERT INTO app.accounts(id) VALUES(17)")
        .unwrap();
    let again = f.apply("plan.json", Some(digest));
    assert_eq!(again["outcome"], "pass", "{again}");
    assert_eq!(again["progress"][0]["state"], "already_applied");
    assert_eq!(
        f.target()
            .query_one("SELECT label FROM app.accounts WHERE id=17", &[])
            .unwrap()
            .get::<_, String>(0),
        "a; b"
    );
    assert_eq!(f.command("verify")["outcome"], "pass");
}
