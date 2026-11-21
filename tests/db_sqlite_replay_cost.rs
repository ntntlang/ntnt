//! #421: replay inspects the catalog after every migration and reuses what it
//! found for an object only while nothing that object depends on has
//! changed. In each history here a later migration takes an object outside
//! coverage without changing that object's own definition, and the plan must
//! still stop at that migration, naming the object.
#[path = "support/sqlite_fixture.rs"]
mod sqlite_fixture;
use serde_json::{json, Value};
use sqlite_fixture::Fixture;

/// Plan `migrations`: the replay must stop at `source` with exactly `objects`
/// outside coverage, none of them on the (empty) target.
fn plan_stops_at(migrations: &[(&str, &str)], source: &str, objects: Value) {
    let f = Fixture::new();
    for (name, sql) in migrations {
        f.sql(name, sql);
    }
    let r = f.command("plan", &[]);
    assert_eq!(r["plan"]["classification"], "blocked", "{r}");
    assert_eq!(r["catalog"]["uncovered"], json!([]), "{r}");
    let diagnostics = r["diagnostics"].as_array().unwrap();
    assert_eq!(diagnostics.len(), 1, "{r}");
    let d = &diagnostics[0];
    assert_eq!(d["code"], "DB_COVERAGE_INCOMPLETE", "{r}");
    assert_eq!(
        d["message"],
        "Intermediate catalog exceeds execution coverage."
    );
    assert_eq!(d["source"], source, "{r}");
    assert_eq!(d["objects"], objects, "{r}");
}

/// A trigger body names the table it writes; dropping that table breaks
/// the programs of the triggering table, which itself did not change.
#[test]
fn dropping_the_table_a_trigger_writes_uncovers_the_triggering_table() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); CREATE TABLE audit(id INTEGER); CREATE TRIGGER t_audit AFTER UPDATE OF v ON t BEGIN INSERT INTO audit VALUES(new.id); END;"),
            ("0002_drop_audit.sql", "DROP TABLE audit;"),
        ],
        "0002_drop_audit.sql",
        json!(["table:\"t\""]),
    );
}

/// The same through a chain of triggers: t's trigger writes u, whose trigger
/// writes w. Dropping w breaks both t and u.
#[test]
fn dropping_the_end_of_a_trigger_chain_uncovers_every_table_before_it() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE t(id INTEGER PRIMARY KEY); CREATE TABLE u(id INTEGER); CREATE TABLE w(id INTEGER); CREATE TRIGGER t_u AFTER INSERT ON t BEGIN INSERT INTO u VALUES(new.id); END; CREATE TRIGGER u_w AFTER INSERT ON u BEGIN INSERT INTO w VALUES(new.id); END;"),
            ("0002_unrelated.sql", "CREATE TABLE other(id INTEGER PRIMARY KEY);"),
            ("0003_drop_w.sql", "DROP TABLE w;"),
        ],
        "0003_drop_w.sql",
        json!(["table:\"t\"", "table:\"u\""]),
    );
}

/// Deleting from a parent table runs the foreign-key actions of the tables
/// that reference it. A new referencing table whose delete trigger writes the
/// ledger takes the unchanged parent outside coverage too, through every
/// level of cascades.
#[test]
fn a_new_referencing_table_uncovers_its_unchanged_parents() {
    plan_stops_at(
        &[
            ("0001_parent.sql", "CREATE TABLE p(id INTEGER PRIMARY KEY);"),
            ("0002_child.sql", "CREATE TABLE c(id INTEGER PRIMARY KEY, p_id INTEGER REFERENCES p(id) ON DELETE CASCADE);"),
            ("0003_grandchild.sql", "CREATE TABLE g(id INTEGER PRIMARY KEY, c_id INTEGER REFERENCES c(id) ON DELETE CASCADE); CREATE TRIGGER g_gone AFTER DELETE ON g BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;"),
        ],
        "0003_grandchild.sql",
        json!(["table:\"c\"", "table:\"g\"", "table:\"p\""]),
    );
}

/// A table a trigger writes is written by the triggering table's programs,
/// so the tables that reference it count as well: t's trigger deletes from u,
/// and a new table referencing u cascades into a delete trigger that writes
/// the ledger.
#[test]
fn a_table_referencing_what_a_trigger_writes_uncovers_the_triggering_table() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE t(id INTEGER PRIMARY KEY); CREATE TABLE u(id INTEGER PRIMARY KEY); CREATE TRIGGER t_u AFTER DELETE ON t BEGIN DELETE FROM u WHERE id = old.id; END;"),
            ("0002_child.sql", "CREATE TABLE c(id INTEGER PRIMARY KEY, u_id INTEGER REFERENCES u(id) ON DELETE CASCADE); CREATE TRIGGER c_gone AFTER DELETE ON c BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;"),
        ],
        "0002_child.sql",
        json!(["table:\"c\"", "table:\"t\"", "table:\"u\""]),
    );
}

/// A view's program reads the tables its query names.
#[test]
fn dropping_a_table_a_view_reads_uncovers_the_view() {
    plan_stops_at(
        &[
            (
                "0001_initial.sql",
                "CREATE TABLE t(x INTEGER); CREATE VIEW v AS SELECT x FROM t;",
            ),
            ("0002_drop.sql", "DROP TABLE t;"),
        ],
        "0002_drop.sql",
        json!(["view:\"v\""]),
    );
}

/// A dependency that keeps its name but changes its definition: t's trigger
/// writes audit's `at` column, and a later migration rebuilds audit without it.
#[test]
fn rebuilding_the_table_a_trigger_writes_uncovers_the_triggering_table() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); CREATE TABLE audit(id INTEGER, at TEXT); CREATE TRIGGER t_audit AFTER UPDATE OF v ON t BEGIN INSERT INTO audit(id, at) VALUES(new.id, 'x'); END;"),
            ("0002_rebuild_audit.sql", "DROP TABLE audit; CREATE TABLE audit(id INTEGER);"),
        ],
        "0002_rebuild_audit.sql",
        json!(["table:\"t\""]),
    );
}

/// A name belongs to every object that has it. A trigger on u may share the
/// name of table t, and the view that reads t still depends on t.
#[test]
fn a_view_depends_on_its_table_when_a_trigger_shares_the_table_name() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE t(x INTEGER); CREATE TABLE u(y INTEGER); CREATE TRIGGER t AFTER INSERT ON u BEGIN SELECT 1; END; CREATE VIEW v AS SELECT x FROM t;"),
            ("0002_drop.sql", "DROP TABLE t;"),
        ],
        "0002_drop.sql",
        json!(["view:\"v\""]),
    );
}

/// The same for foreign keys: a trigger named like the parent table does not
/// hide the parent from a new table that references it.
#[test]
fn a_new_referencing_table_uncovers_a_parent_whose_name_a_trigger_shares() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE p(id INTEGER PRIMARY KEY); CREATE TABLE q(id INTEGER); CREATE TRIGGER p AFTER INSERT ON q BEGIN SELECT 1; END;"),
            ("0002_child.sql", "CREATE TABLE c(id INTEGER PRIMARY KEY, p_id INTEGER REFERENCES p(id) ON DELETE CASCADE); CREATE TRIGGER c_gone AFTER DELETE ON c BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;"),
        ],
        "0002_child.sql",
        json!(["table:\"c\"", "table:\"p\""]),
    );
}

/// A table's programs are not compiled again when a table below it in a
/// chain of ON DELETE CASCADE keys is added; when the new one's delete
/// trigger writes the ledger, every table above it is outside coverage.
#[test]
fn a_cascade_into_a_new_table_uncovers_every_table_above_it() {
    let mut migrations = vec![(
        "0001_t1.sql".to_owned(),
        "CREATE TABLE t1(id INTEGER PRIMARY KEY);".to_owned(),
    )];
    for i in 2..=5 {
        migrations.push((
            format!("{i:04}_t{i}.sql"),
            format!("CREATE TABLE t{i}(id INTEGER PRIMARY KEY, prev INTEGER REFERENCES t{}(id) ON DELETE CASCADE);", i - 1),
        ));
    }
    migrations.push(("0006_t6.sql".to_owned(), "CREATE TABLE t6(id INTEGER PRIMARY KEY, prev INTEGER REFERENCES t5(id) ON DELETE CASCADE); CREATE TRIGGER t6_gone AFTER DELETE ON t6 BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;".to_owned()));
    let migrations: Vec<(&str, &str)> = migrations
        .iter()
        .map(|(name, sql)| (name.as_str(), sql.as_str()))
        .collect();
    plan_stops_at(
        &migrations,
        "0006_t6.sql",
        json!([
            "table:\"t1\"",
            "table:\"t2\"",
            "table:\"t3\"",
            "table:\"t4\"",
            "table:\"t5\"",
            "table:\"t6\""
        ]),
    );
}

/// ON DELETE SET NULL updates the child's key column, so a trigger on an
/// UPDATE of that column, added to the unchanged child, takes the parent
/// and the table above it outside coverage.
#[test]
fn a_set_null_child_s_update_trigger_uncovers_its_parents() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE a(id INTEGER PRIMARY KEY); CREATE TABLE b(id INTEGER PRIMARY KEY, a_id INTEGER REFERENCES a(id) ON DELETE CASCADE);"),
            ("0002_child.sql", "CREATE TABLE c(id INTEGER PRIMARY KEY, b_id INTEGER REFERENCES b(id) ON DELETE SET NULL);"),
            ("0003_trigger.sql", "CREATE TRIGGER c_b AFTER UPDATE OF b_id ON c BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;"),
        ],
        "0003_trigger.sql",
        json!(["table:\"a\"", "table:\"b\"", "table:\"c\""]),
    );
}

/// ON UPDATE CASCADE through two levels of unique keys: a trigger on the
/// grandchild's UPDATE takes the child and the grandparent outside
/// coverage, though neither changed.
#[test]
fn an_update_cascade_through_a_grandchild_uncovers_the_grandparent() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE a(id INTEGER PRIMARY KEY, k TEXT UNIQUE); CREATE TABLE f(id INTEGER PRIMARY KEY, a_k TEXT UNIQUE REFERENCES a(k) ON UPDATE CASCADE);"),
            ("0002_grandchild.sql", "CREATE TABLE g(id INTEGER PRIMARY KEY, f_a_k TEXT REFERENCES f(a_k) ON UPDATE CASCADE);"),
            ("0003_trigger.sql", "CREATE TRIGGER g_up AFTER UPDATE ON g BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;"),
        ],
        "0003_trigger.sql",
        json!(["table:\"a\"", "table:\"f\"", "table:\"g\""]),
    );
}

/// The per-migration guarantee itself: an object outside coverage that a
/// later migration removes is still found, attributed to the migration that
/// made it.
#[test]
fn an_object_outside_coverage_only_between_two_migrations_is_found() {
    plan_stops_at(
        &[
            ("0001_initial.sql", "CREATE TABLE t(value TEXT);"),
            ("0002_trigger.sql", "CREATE TRIGGER hidden AFTER UPDATE OF value ON t BEGIN DELETE FROM _ntnt_schema_migrations_v2; END;"),
            ("0003_untrigger.sql", "DROP TRIGGER hidden;"),
        ],
        "0002_trigger.sql",
        json!(["table:\"t\""]),
    );
}

/// What inspection reuses is keyed by each object's kind as well as its name:
/// a trigger may share its table's name, and still reports its own definition.
#[test]
fn a_trigger_named_like_its_table_keeps_its_own_definition() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY, v TEXT); CREATE TABLE audit(id INTEGER); CREATE TRIGGER t AFTER UPDATE OF v ON t BEGIN INSERT INTO audit VALUES(new.id); END;",
    );
    f.sql(
        "0002_more.sql",
        "CREATE TABLE later(id INTEGER PRIMARY KEY);",
    );
    let plan = f.plan("plan.json");
    let applied = f.apply("plan.json", plan["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let r = f.command("verify", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
    let objects = &r["catalog"]["objects"];
    let (table, trigger) = (&objects["table:\"t\""], &objects["trigger:\"t\""]);
    assert_eq!(trigger["table"], "t", "{r}");
    assert!(table["definition"]["sha256"].is_string(), "{r}");
    assert_ne!(table["definition"], trigger["definition"], "{r}");
}
