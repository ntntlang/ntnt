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
