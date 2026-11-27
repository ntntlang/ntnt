use super::native_scratch::{assert_discarded, diagnostic_codes, report, NativeScratch};
use super::{code, index_state, ledger_rows, wait_for, Fixture};
use postgres::fallible_iterator::FallibleIterator;
use std::process::{Child, Stdio};

// A failed barrier assertion must not orphan the CLI on the test host.
struct RunningPlan(Child);
impl Drop for RunningPlan {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

const TABLES: &str = "CREATE TABLE app.events(id integer) PARTITION BY RANGE(id);
    CREATE TABLE app.leaf PARTITION OF app.events DEFAULT;
    INSERT INTO app.events VALUES (17); NOTIFY replay_probe, 'first';";
const LEAF: &str = "CREATE INDEX CONCURRENTLY leaf_idx ON app.leaf(id);";
const GUARD: &str = "DB_REPLAY_ISOLATION_REQUIRED";

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn native_plan_refuses_a_scratch_database_on_the_target_cluster() {
    let mut f = Fixture::new();
    f.sql("0001_table.sql", "CREATE TABLE app.t(id integer);");
    f.sql(
        "0002_index.sql",
        "CREATE INDEX CONCURRENTLY t_id ON app.t(id);",
    );
    let cluster: String = f
        .admin
        .query_one(
            "SELECT system_identifier::text FROM pg_control_system()",
            &[],
        )
        .unwrap()
        .get(0);
    let r = report(f.process("plan").env("NTNT_DB_REPLAY_CLUSTER", cluster));
    assert_ne!(r["outcome"], "pass");
    assert!(code(&r, GUARD), "{:?}", diagnostic_codes(&r));
}

fn baseline() -> Fixture {
    let f = Fixture::new();
    f.sql("0001_tables.sql", TABLES);
    // Ordinary history still plans on reusable same-cluster scratch without a binding.
    let first = f.plan("initial.json");
    assert_eq!(first["scratch"]["cleanup"], "pass");
    assert_eq!(
        f.apply("initial.json", first["plan_digest"].as_str())["outcome"],
        "pass"
    );
    f.sql("0002_index.sql", LEAF);
    f
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn ordinary_applied_prefix_uses_reusable_scratch_with_native_work_pending() {
    let f = baseline();
    for cmd in ["status", "verify"] {
        let r = f.command(cmd);
        assert_eq!(r["outcome"], "pass", "{cmd}: {:?}", diagnostic_codes(&r));
        assert_eq!(r["scratch"]["cleanup"], "pass");
    }
}

// Data, managed relations (including physical OIDs), and every ledger field.
fn target_state(f: &Fixture) -> (Vec<i32>, Vec<String>, String) {
    let row = f.target().query_one("SELECT ARRAY(SELECT id FROM app.events ORDER BY id),
        ARRAY(SELECT oid::text||':'||relname FROM pg_class WHERE relnamespace='app'::regnamespace ORDER BY oid),
        (SELECT json_agg(l ORDER BY position)::text FROM _ntnt.schema_migrations_v2 l)", &[]).unwrap();
    (row.get(0), row.get(1), row.get(2))
}

#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn leaf_replays_the_whole_prefix_once_and_applies_with_approval() {
    let f = baseline();
    let before = target_state(&f);
    let plan = {
        let scratch = NativeScratch::new(&f);
        let mut observer = scratch.observer();
        observer.batch_execute("LISTEN replay_probe").unwrap();
        let r = report(scratch.process(&f, "plan").args(["--out", "leaf.json"]));
        assert_eq!(r["outcome"], "pass", "{:?}", diagnostic_codes(&r));
        assert_eq!(r["scratch"]["cleanup"], "external");
        // PostgreSQL delivers committed notifications in commit order. Fence
        // the stream *after* the CLI exits: count actual replays of the
        // already-applied first migration without a timeout-as-absence test.
        observer
            .batch_execute("NOTIFY replay_probe, 'end'")
            .unwrap();
        let mut probe = observer.notifications();
        let mut messages = probe.timeout_iter(super::HANG_GUARD);
        assert_eq!(messages.next().unwrap().unwrap().payload(), "first");
        assert_eq!(messages.next().unwrap().unwrap().payload(), "end");
        r
    };
    assert_eq!(target_state(&f), before);
    assert!(code(&f.apply("leaf.json", None), "DB_APPROVAL_REQUIRED"));
    assert_eq!(target_state(&f), before);
    // An intact, separately approved pre-native descriptor still must not
    // start work. Refreshing the plan, not re-approving this digest, fixes it.
    let mut old = plan.clone();
    old["plan"]["execution"]
        .as_object_mut()
        .unwrap()
        .remove("replay");
    let old_digest = super::plan_digest(&old["plan"]);
    old["plan_digest"] = serde_json::json!(old_digest);
    std::fs::write(
        f.root.path().join("old.json"),
        serde_json::to_vec(&old).unwrap(),
    )
    .unwrap();
    // Read-only validation must agree with the artifact migrate can start.
    for (file, expected) in [("leaf.json", "pass"), ("old.json", "fail")] {
        let verified = report(f.process("verify").args(["--against", file]));
        assert_eq!(verified["plan_validation"]["outcome"], expected, "{file}");
        assert_eq!(verified["outcome"], expected, "{file}");
        assert_eq!(code(&verified, "DB_PLAN_STALE"), expected == "fail");
        assert_eq!(target_state(&f), before);
    }
    assert!(code(
        &f.apply("old.json", Some(&old_digest)),
        "DB_PLAN_STALE"
    ));
    assert_eq!(target_state(&f), before);
    let applied = f.apply("leaf.json", plan["plan_digest"].as_str());
    assert_eq!(
        applied["outcome"],
        "pass",
        "{:?}",
        diagnostic_codes(&applied)
    );
    assert_eq!(index_state(&f, "leaf_idx"), Some(true));
    assert_eq!(ledger_rows(&f), 2);
    for cmd in ["status", "verify"] {
        let r = f.isolated_command(cmd, &[]);
        assert_eq!(r["outcome"], "pass", "{cmd}: {:?}", diagnostic_codes(&r));
        assert_eq!(r["scratch"]["cleanup"], "external");
    }
}

#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn partitioned_parent_is_rejected_by_postgres_without_target_or_ledger_changes() {
    let f = baseline();
    // Keep the same table, role, and index shape as the accepted leaf case;
    // only the indexed relation changes. A replay stripping CONCURRENTLY passes.
    f.sql(
        "0002_index.sql",
        "CREATE INDEX CONCURRENTLY parent_idx ON app.events(id);",
    );
    let before = target_state(&f);
    let rejected = f.isolated_command("plan", &["--out", "parent.json"]);
    assert_eq!(rejected["outcome"], "fail");
    assert_eq!(rejected["plan"]["candidate"]["outcome"], "fail");
    let diagnostics = rejected["diagnostics"].as_array().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d["message"].as_str().is_some_and(|m| m.contains("0A000"))),
        "expected a diagnostic with PostgreSQL's rejection SQLSTATE"
    );
    assert!(!code(&rejected, GUARD));
    assert_eq!(rejected["plan"]["classification"], "blocked");
    assert!(
        !rejected
            .to_string()
            .contains("cannot create index on partitioned table"),
        "raw server error disclosed"
    );
    assert_eq!(target_state(&f), before);
    assert_eq!(index_state(&f, "parent_idx"), None);
    assert_eq!(ledger_rows(&f), 1);
}

#[test]
#[ignore = "requires disposable PostgreSQL; mandatory CI execution lane"]
fn rejected_script_names_only_a_position_in_the_submitted_sql() {
    let f = baseline();
    let before = target_state(&f);
    for (sql, numbered) in [
        ("SELECT 'é;'; SELECT private_canary FROM app.leaf;", true),
        (
            "DO $$ BEGIN EXECUTE 'SELECT private_canary'; END $$;",
            false,
        ),
    ] {
        f.sql("0002_index.sql", sql);
        let r = f.command("plan");
        let d = &r["diagnostics"][0];
        assert_eq!(d["source"], "0002_index.sql");
        let message = d["message"].as_str().unwrap();
        assert!(
            message.contains("undefined column") && message.contains("42703"),
            "{d}"
        );
        assert_eq!(message.contains("Statement 2:"), numbered, "{d}");
        if !numbered {
            assert!(!message.contains("Statement "), "{d}");
        }
        assert!(!r.to_string().contains("private_canary"));
        assert_eq!(r["drift"]["outcome"], "pass");
        assert_eq!(r["outcome"], "fail");
        assert_eq!(r["plan"]["candidate"]["outcome"], "fail");
        assert_eq!(r["plan"]["classification"], "blocked");
        assert_eq!(target_state(&f), before);
    }
    f.sql("0002_index.sql", "SELECT 'é;'; SELECT id FROM app.leaf;");
    assert_eq!(f.command("plan")["outcome"], "pass");
    assert_eq!(target_state(&f), before);
}

#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn absent_and_mismatched_bindings_refuse_before_replay() {
    let f = baseline();
    let before = target_state(&f);
    for missing in [true, false] {
        let scratch = NativeScratch::new(&f);
        let mut cmd = scratch.process(&f, "plan");
        if missing {
            cmd.env_remove("NTNT_DB_REPLAY_CLUSTER");
        } else {
            let wrong = scratch.cluster.parse::<u64>().unwrap() ^ 1;
            cmd.env("NTNT_DB_REPLAY_CLUSTER", wrong.to_string());
        }
        let r = report(&mut cmd);
        assert_ne!(r["outcome"], "pass");
        assert!(code(&r, GUARD), "{:?}", diagnostic_codes(&r));
        assert!(scratch
            .observer()
            .query_one("SELECT to_regclass('app.events') IS NULL", &[])
            .unwrap()
            .get::<_, bool>(0));
    }
    assert_eq!(target_state(&f), before);
}

#[cfg(unix)]
#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn lost_create_index_ack_blocks_plan_without_blaming_the_migration() {
    use super::db_proxy::{proxied_scratch_plan, Fault};
    use std::sync::atomic::Ordering;

    let f = baseline();
    let before = target_state(&f);
    let scratch = NativeScratch::new(&f);
    let storage = scratch.storage();
    let (proxy, mut plan) = proxied_scratch_plan(
        &f,
        &scratch,
        &storage.2.join("socket"),
        Fault::AfterCreateIndex,
    );
    let r = report(plan.args(["--out", "interrupted.json"]));
    assert!(
        proxy.fired.load(Ordering::SeqCst),
        "missing CREATE INDEX completion"
    );
    // Independent connection, bypassing the proxy: PostgreSQL actually finished
    // this build, but the planner did not receive its CommandComplete.
    assert!(scratch
        .observer()
        .query_one(
            "SELECT indisready AND indisvalid FROM pg_index
         WHERE indexrelid='app.leaf_idx'::regclass AND indrelid='app.leaf'::regclass",
            &[],
        )
        .unwrap()
        .get::<_, bool>(0));
    let cluster = scratch.cluster.clone();
    drop(proxy);
    drop(scratch);
    assert_discarded(storage);

    assert_ne!(r["outcome"], "pass");
    assert_eq!(r["plan"]["candidate"]["outcome"], "unknown");
    assert_eq!(r["plan"]["classification"], "blocked");
    assert_eq!(r["plan"]["authorized"], false);
    assert_eq!(r["plan"]["executable"], false);
    assert_eq!(r["scratch"]["cleanup"], "external");
    assert_eq!(target_state(&f), before);
    // Even presenting this exact blocked digest cannot authorize a migration.
    let digest = r["plan_digest"].as_str().unwrap();
    assert!(code(
        &f.apply("interrupted.json", Some(digest)),
        "DB_PLAN_STALE"
    ));
    assert_eq!(target_state(&f), before);

    let d = r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == "DB_POSTGRES")
        .expect("missing DB_POSTGRES");
    let action = d["next_action"].as_str().unwrap().to_ascii_lowercase();
    assert!(
        (action.contains("rerun") || action.contains("retry")) && action.contains("scratch"),
        "{action}"
    );
    let message = d["message"].as_str().unwrap().to_ascii_lowercase();
    assert!(
        !message.contains("postgresql rejected") && !action.contains("postgresql rejected"),
        "{message}: {action}"
    );

    let fresh = NativeScratch::new(&f);
    assert_ne!(fresh.cluster, cluster);
    let retry = report(&mut fresh.process(&f, "plan"));
    assert_eq!(retry["outcome"], "pass", "{:?}", diagnostic_codes(&retry));
    assert_eq!(retry["plan"]["candidate"]["outcome"], "pass");
    assert_eq!(target_state(&f), before);
}

#[test]
#[ignore = "requires disposable PostgreSQL and Docker; mandatory CI execution lane"]
fn killed_plan_discards_in_progress_native_index_and_reruns_on_fresh_storage() {
    let f = baseline();
    let before = target_state(&f);
    let (storage, cluster) = {
        let scratch = NativeScratch::new(&f);
        let mut observer = scratch.observer();
        let mut holder = scratch.observer();
        holder
            .batch_execute("BEGIN ISOLATION LEVEL REPEATABLE READ")
            .unwrap();
        let holder_pid: i32 = holder
            .query_one("SELECT pg_backend_pid(), pg_current_snapshot()::text", &[])
            .unwrap()
            .get(0);
        let mut child = RunningPlan(
            scratch
                .process(&f, "plan")
                .args(["--out", "interrupted.json"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        );
        // This phase only exists for a native concurrent build. The old
        // snapshot keeps it there until we kill the CLI; no sleeps or race
        // to catch a transient entry in the progress view.
        wait_for("native scratch build waiting for our old snapshot", || {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "plan exited before native build barrier"
            );
            let waiting: bool = observer
                .query_one(
                    "SELECT EXISTS (
                SELECT FROM pg_stat_progress_create_index
                WHERE command = 'CREATE INDEX CONCURRENTLY' AND phase = 'waiting for old snapshots'
                  AND index_relid = to_regclass('app.leaf_idx') AND current_locker_pid = $1::int)",
                    &[&holder_pid],
                )
                .unwrap()
                .get(0);
            waiting.then_some(())
        });
        // PG has committed the catalog entry, but has not marked it valid.
        assert!(observer.query_one("SELECT indisready AND NOT indisvalid FROM pg_index WHERE indexrelid='app.leaf_idx'::regclass", &[]).unwrap().get::<_, bool>(0));
        child.0.kill().unwrap();
        assert!(!child.0.wait().unwrap().success());
        // No cleanup SQL: drop the job's scratch owner just as for success or
        // rejection. The interrupted backend need not cooperate with cleanup.
        let result = (scratch.storage(), scratch.cluster.clone());
        drop(scratch);
        result
    };
    assert_discarded(storage);
    assert_eq!(target_state(&f), before);
    assert!(!f.root.path().join("interrupted.json").exists());
    let fresh = NativeScratch::new(&f);
    assert_ne!(fresh.cluster, cluster);
    let retry = report(fresh.process(&f, "plan").args(["--out", "retry.json"]));
    assert_eq!(retry["outcome"], "pass", "{:?}", diagnostic_codes(&retry));
    assert_eq!(retry["scratch"]["cleanup"], "external");
    assert_eq!(target_state(&f), before);
}
