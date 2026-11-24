//! #427: the SQLite migration policy admits SQLite's built-in functions,
//! AUTOINCREMENT, planner statistics, the statements SQLite runs for ALTER
//! TABLE, reads that name no column and WITH RECURSIVE, and still refuses
//! what loads code or reaches outside the database. Every test drives the
//! `ntnt db` CLI (plan, migrate, verify) on SQLite fixture files.
#[path = "support/sqlite_fixture.rs"]
mod sqlite_fixture;
use serde_json::{json, Value};
use sqlite_fixture::Fixture;

fn diagnostic<'a>(r: &'a Value, code: &str) -> &'a Value {
    r["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["code"] == code)
        .unwrap_or_else(|| panic!("no {code} in {r}"))
}

/// Plan, approve, migrate and verify what is pending; returns the verify
/// report, whose catalog is complete.
fn cycle(f: &Fixture, plan: &str) -> Value {
    let p = f.plan(plan);
    let applied = f.apply(plan, p["plan_digest"].as_str());
    assert_eq!(applied["outcome"], "pass", "{applied}");
    let verified = f.command("verify", &[]);
    assert_eq!(verified["outcome"], "pass", "{verified}");
    assert_eq!(verified["catalog"]["uncovered"], json!([]), "{verified}");
    verified
}

fn schema_objects(f: &Fixture) -> i64 {
    f.target()
        .query_row("SELECT count(*) FROM sqlite_schema", [], |r| r.get(0))
        .unwrap()
}

/// sq-16: AUTOINCREMENT tables, the default primary key of Django and Rails
/// schemas, plan, migrate and verify. `sqlite_sequence` is counter state:
/// never a catalog object, so the counters moving is not drift, and a
/// migration may set them. The table's definition, AUTOINCREMENT keyword
/// included, still counts: dropping the keyword out of band is drift.
#[test]
fn sqlite_autoincrement_tables_plan_migrate_and_verify() {
    let f = Fixture::new();
    f.sql(
        "0001_django.sql",
        r#"CREATE TABLE "django_migrations" ("id" integer NOT NULL PRIMARY KEY AUTOINCREMENT, "app" varchar(255) NOT NULL, "name" varchar(255) NOT NULL, "applied" datetime NOT NULL);"#,
    );
    let verified = cycle(&f, "0001.json");
    let objects = verified["catalog"]["objects"].as_object().unwrap();
    assert!(
        objects.keys().all(|key| !key.contains("sqlite_")),
        "{verified}"
    );
    let fingerprint = verified["drift"]["observed_fingerprint"].clone();
    f.target()
        .execute_batch(
            "INSERT INTO django_migrations(app, name, applied) VALUES ('a', '0001', '2026-01-01');",
        )
        .unwrap();
    let r = f.command("verify", &[]);
    assert_eq!(r["outcome"], "pass", "{r}");
    assert_eq!(r["drift"]["observed_fingerprint"], fingerprint, "{r}");
    // SQLite keeps sqlite_sequence right as migrations rename and drop
    // AUTOINCREMENT tables, and a migration may set a counter.
    f.sql(
        "0002_counters.sql",
        "UPDATE sqlite_sequence SET seq = 1000 WHERE name = 'django_migrations'; \
         CREATE TABLE jobs(id INTEGER PRIMARY KEY AUTOINCREMENT, kind TEXT NOT NULL); \
         INSERT INTO jobs(kind) VALUES ('import'); \
         ALTER TABLE jobs RENAME TO tasks; \
         CREATE TABLE retired(id INTEGER PRIMARY KEY AUTOINCREMENT); \
         DROP TABLE retired;",
    );
    cycle(&f, "0002.json");
    let c = f.target();
    c.execute_batch(
        "INSERT INTO django_migrations(app, name, applied) VALUES ('a', '0002', '2026-01-02');",
    )
    .unwrap();
    let counters: Vec<(String, i64)> = c
        .prepare("SELECT name, seq FROM sqlite_sequence ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        counters,
        [
            ("django_migrations".to_owned(), 1001),
            ("tasks".to_owned(), 1)
        ]
    );
    c.execute_batch(
        "CREATE TABLE tasks_new(id INTEGER PRIMARY KEY, kind TEXT NOT NULL); \
         INSERT INTO tasks_new SELECT id, kind FROM tasks; DROP TABLE tasks; \
         ALTER TABLE tasks_new RENAME TO tasks;",
    )
    .unwrap();
    let r = f.command("verify", &[]);
    assert_eq!(r["outcome"], "fail", "{r}");
    diagnostic(&r, "DB_DRIFT");
    let differences = r["drift"]["differences"].as_array().unwrap();
    assert_eq!(differences.len(), 1, "{r}");
    assert_eq!(differences[0]["object"], r#"table:"tasks""#, "{r}");
    assert_eq!(differences[0]["change"], "changed", "{r}");
}

/// SQLite's built-in functions, JSON and randomblob() included, work in
/// defaults, CHECK constraints, index expressions and predicates, trigger
/// and view bodies, and backfills.
#[test]
fn sqlite_builtin_functions_work_in_defaults_checks_indexes_and_backfills() {
    let f = Fixture::new();
    f.sql(
        "0001_documents.sql",
        "CREATE TABLE documents(\
            id INTEGER PRIMARY KEY, \
            token TEXT NOT NULL DEFAULT (lower(hex(randomblob(16)))), \
            body TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(body)), \
            price REAL CHECK (price IS NULL OR round(price, 2) = price), \
            slug TEXT CHECK (slug IS NULL OR (instr(slug, ' ') = 0 AND slug = lower(trim(slug)))), \
            tag_count INTEGER NOT NULL DEFAULT 0); \
         CREATE INDEX documents_kind ON documents(json_extract(body, '$.kind')) WHERE json_type(body, '$.kind') = 'text'; \
         CREATE INDEX documents_slug ON documents(replace(slug, '-', '')) WHERE nullif(slug, '') IS NOT NULL; \
         CREATE TABLE tags(document_id INTEGER NOT NULL REFERENCES documents(id), tag TEXT NOT NULL); \
         CREATE TRIGGER tags_counted AFTER INSERT ON tags BEGIN \
            UPDATE documents SET tag_count = tag_count + 1, body = json_set(body, '$.tagged', json('true')) \
            WHERE id = new.document_id; END; \
         CREATE VIEW document_tags AS SELECT d.id, group_concat(t.tag, ',') AS tags, \
            json_group_array(t.tag) AS tag_array, total(length(t.tag)) AS letters \
            FROM documents AS d LEFT JOIN tags AS t ON t.document_id = d.id GROUP BY d.id;",
    );
    cycle(&f, "0001.json");
    f.sql(
        "0002_backfill.sql",
        "INSERT INTO documents(body, price, slug) VALUES \
            ('{\"kind\":\"memo\",\"tags\":[\"a\",\"b\"]}', 12.5, 'first-memo'), \
            ('{\"kind\":\"note\",\"tags\":[\"c\"]}', NULL, NULL); \
         INSERT INTO tags(document_id, tag) SELECT d.id, upper(j.value) FROM documents AS d, json_each(d.body, '$.tags') AS j; \
         UPDATE documents SET \
            body = json_insert(body, '$.rank', (SELECT r FROM (SELECT id, row_number() OVER (ORDER BY id) AS r FROM documents) AS ranked WHERE ranked.id = documents.id)), \
            price = round(coalesce(price, 0) * 1.1, 2) \
            WHERE body ->> '$.kind' IN ('memo', 'note'); \
         UPDATE documents SET slug = iif(slug IS NULL, printf('doc-%d', id), slug);",
    );
    cycle(&f, "0002.json");
    let rows: Vec<(String, i64, i64, String, f64)> = f
        .target()
        .prepare(
            "SELECT token, tag_count, body ->> '$.rank', slug, price FROM documents ORDER BY id",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(rows.len(), 2);
    for (token, ..) in &rows {
        assert_eq!(token.len(), 32, "{token}");
        assert!(token
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
    }
    assert_ne!(rows[0].0, rows[1].0);
    let rest: Vec<(i64, i64, &str, f64)> = rows
        .iter()
        .map(|(_, count, rank, slug, price)| (*count, *rank, slug.as_str(), *price))
        .collect();
    assert_eq!(rest, [(2, 1, "first-memo", 13.75), (1, 2, "doc-2", 0.0)]);
    let tags: String = f
        .target()
        .query_row(
            "SELECT group_concat(tags, ';') FROM document_tags",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(tags, "A,B;C");
}

/// What loads code or reaches outside the database stays refused, by name,
/// with what to do: `load_extension`, `sqlite_log` and the functions an
/// extension registers. Nothing reaches the target.
#[test]
fn sqlite_functions_that_are_not_builtin_or_reach_outside_are_refused_by_name() {
    for (sql, statement, function) in [
        (
            "CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT DEFAULT (load_extension('private_marker_427')));",
            1,
            "load_extension",
        ),
        (
            "CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT); INSERT INTO t(x) SELECT fts3_tokenizer('simple');",
            2,
            "fts3_tokenizer",
        ),
        (
            "CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT); CREATE INDEX t_x ON t(x) WHERE sqlite_log(1, 'private_marker_427') IS NULL;",
            2,
            "sqlite_log",
        ),
        // A function the application registers on its own connections is
        // missing on ntnt's: in DML it is named like one in a DEFAULT.
        (
            "CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT); UPDATE t SET x = app_slugify(x);",
            2,
            "app_slugify",
        ),
        (
            "CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT); INSERT INTO t(x) VALUES (app_slugify('private_marker_427'));",
            2,
            "app_slugify",
        ),
        (
            "CREATE TABLE t(id INTEGER PRIMARY KEY, x TEXT DEFAULT (app_uuid()));",
            1,
            "app_uuid",
        ),
    ] {
        let f = Fixture::new();
        f.sql("0001_initial.sql", sql);
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{sql}: {r}");
        let d = diagnostic(&r, "DB_SQL_UNSUPPORTED");
        assert_eq!(
            d["message"],
            format!("Statement {statement} uses SQLite function `{function}`, which SQLite migrations cannot use."),
            "{r}"
        );
        assert!(
            d["next_action"].as_str().unwrap().starts_with(&format!(
                "Rewrite statement {statement} of the migration named in source without SQLite function `{function}`, then plan again."
            )),
            "{r}"
        );
        assert_eq!(d["source"], "0001_initial.sql", "{r}");
        assert!(!r.to_string().contains("private_marker_427"), "{r}");
        assert_eq!(schema_objects(&f), 0);
    }
}

/// sq-08: ADD COLUMN on a STRICT table, and ADD COLUMN with a CHECK, plan
/// like other ADD COLUMNs. SQLite re-checks the table's rows for them (with
/// `glob` and `pragma_quick_check`), and that re-check runs on the target:
/// a default that existing rows would break is refused there, naming the
/// failure, and nothing commits.
#[test]
fn sqlite_add_column_to_strict_table_or_with_check_rechecks_target_rows() {
    let f = Fixture::new();
    f.sql(
        "0001_tables.sql",
        "CREATE TABLE s(id INTEGER PRIMARY KEY, v TEXT) STRICT; CREATE TABLE c(id INTEGER PRIMARY KEY, v TEXT);",
    );
    cycle(&f, "0001.json");
    f.target()
        .execute_batch(
            "INSERT INTO s(id, v) VALUES (1, 'a'); INSERT INTO c(id, v) VALUES (1, 'a');",
        )
        .unwrap();
    f.sql(
        "0002_case.sql",
        "ALTER TABLE s ADD COLUMN note TEXT; ALTER TABLE c ADD COLUMN qty INTEGER NOT NULL DEFAULT 1 CHECK (qty > 0);",
    );
    cycle(&f, "0002.json");
    for (n, (sql, failure)) in [
        (
            "ALTER TABLE c ADD COLUMN level INTEGER NOT NULL DEFAULT 0 CHECK (level > 0);",
            "CHECK constraint failed",
        ),
        (
            "ALTER TABLE s ADD COLUMN size INTEGER DEFAULT 'large';",
            "type mismatch on DEFAULT",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        f.sql("0003_new_column.sql", sql);
        // Replay's tables are empty, so the plan passes.
        let plan = format!("0003-{n}.json");
        let p = f.plan(&plan);
        let r = f.apply(&plan, p["plan_digest"].as_str());
        assert_eq!(r["outcome"], "fail", "{sql}: {r}");
        let d = diagnostic(&r, "DB_SQLITE");
        assert_eq!(
            d["message"],
            format!("Statement 1: {failure} (SQLITE_ERROR)."),
            "{r}"
        );
        assert!(
            d["next_action"]
                .as_str()
                .unwrap()
                .starts_with("Nothing from this migration was committed"),
            "{r}"
        );
        assert_eq!(d["source"], "0003_new_column.sql", "{r}");
    }
    let columns: i64 = f
        .target()
        .query_row(
            "SELECT (SELECT count(*) FROM pragma_table_info('c')) + (SELECT count(*) FROM pragma_table_info('s'))",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(columns, 6);
}

/// sq-19: SQLite's planner statistics are not schema. After the
/// application runs ANALYZE and `PRAGMA optimize`, status, verify and plan
/// report the same complete catalog and fingerprint as before; migrations
/// that drop what the statistics describe still run (SQLite updates them as
/// it drops), and a migration may run ANALYZE itself.
#[test]
fn sqlite_planner_statistics_stay_out_of_coverage_and_drift() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE t(id INTEGER PRIMARY KEY, a TEXT); CREATE INDEX t_a ON t(a); \
         CREATE TABLE old(id INTEGER PRIMARY KEY, b TEXT); CREATE INDEX old_b ON old(b);",
    );
    let before = cycle(&f, "0001.json");
    f.target()
        .execute_batch("INSERT INTO t(a) VALUES ('x'), ('y'); INSERT INTO old(b) VALUES ('z'); ANALYZE; PRAGMA optimize;")
        .unwrap();
    let statistics: i64 = f
        .target()
        .query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name IN ('sqlite_stat1', 'sqlite_stat4')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(statistics, 2);
    for command in ["status", "verify", "plan"] {
        let r = f.command(command, &[]);
        assert_eq!(r["outcome"], "pass", "{command}: {r}");
        assert_eq!(r["catalog"]["uncovered"], json!([]), "{command}: {r}");
        if command == "verify" {
            assert_eq!(r["catalog"]["objects"], before["catalog"]["objects"], "{r}");
            assert_eq!(
                r["drift"]["observed_fingerprint"], before["drift"]["observed_fingerprint"],
                "{r}"
            );
        }
    }
    f.sql(
        "0002_retire.sql",
        "DROP INDEX t_a; DROP TABLE old; CREATE INDEX t_a_id ON t(a, id); ANALYZE;",
    );
    cycle(&f, "0002.json");
    // The application's ANALYZE above read ntnt's ledger too; the
    // migration's ANALYZE, which clears the statistics, leaves it out.
    let ledger: i64 = f
        .target()
        .query_row(
            "SELECT count(*) FROM sqlite_stat1 WHERE tbl = '_ntnt_schema_migrations_v2'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(ledger, 0);
    let described: Vec<(String, Option<String>)> = f
        .target()
        .prepare("SELECT DISTINCT tbl, idx FROM sqlite_stat1 ORDER BY tbl, idx")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        described.contains(&("t".to_owned(), Some("t_a_id".to_owned()))),
        "{described:?}"
    );
    assert!(
        described
            .iter()
            .all(|(table, index)| table != "old" && index.as_deref() != Some("t_a")),
        "{described:?}"
    );
}

/// A read that names no column of a table (SQLite reports it without a
/// database) runs when the name is a table or view of `main` or a common
/// table expression, and WITH RECURSIVE is allowed, leading an INSERT or
/// DELETE as well as inside one.
#[test]
fn sqlite_reads_without_columns_and_recursive_ctes_migrate() {
    let f = Fixture::new();
    f.sql(
        "0001_initial.sql",
        "CREATE TABLE events(id INTEGER PRIMARY KEY, kind TEXT NOT NULL); \
         CREATE TABLE stats(name TEXT NOT NULL, n INTEGER NOT NULL); \
         CREATE TABLE days(d INTEGER NOT NULL);",
    );
    cycle(&f, "0001.json");
    f.target()
        .execute_batch("INSERT INTO events(kind) VALUES ('a'), ('b'), ('a');")
        .unwrap();
    f.sql(
        "0002_backfill.sql",
        "INSERT INTO stats SELECT 'events', count(*) FROM events; \
         INSERT INTO stats SELECT 'any', 1 WHERE EXISTS (SELECT 1 FROM events); \
         WITH kinds AS (SELECT DISTINCT kind FROM events) INSERT INTO stats SELECT 'kinds', count(*) FROM kinds; \
         WITH RECURSIVE n(d) AS (SELECT 1 UNION ALL SELECT d + 1 FROM n WHERE d < 7) INSERT INTO days SELECT d FROM n; \
         INSERT INTO stats WITH RECURSIVE n(d) AS (SELECT 1 UNION ALL SELECT d + 1 FROM n WHERE d < 3) SELECT 'counted', count(*) FROM n; \
         WITH RECURSIVE n(d) AS (SELECT 6 UNION ALL SELECT d + 1 FROM n WHERE d < 7) DELETE FROM days WHERE d IN (SELECT d FROM n);",
    );
    cycle(&f, "0002.json");
    let c = f.target();
    let stats: Vec<(String, i64)> = c
        .prepare("SELECT name, n FROM stats ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    let expected = [("any", 1), ("counted", 3), ("events", 3), ("kinds", 2)]
        .map(|(name, n)| (name.to_owned(), n));
    assert_eq!(stats, expected);
    let days: i64 = c
        .query_row("SELECT count(*) FROM days", [], |r| r.get(0))
        .unwrap();
    assert_eq!(days, 5);
}

/// Reads that name no column still cannot reach the eponymous virtual
/// tables that read the file's pages or run a PRAGMA.
#[test]
fn sqlite_reads_without_columns_still_refuse_virtual_tables() {
    for (sql, refused) in [
        (
            "INSERT INTO t SELECT count(*) FROM dbstat;",
            "table `dbstat`",
        ),
        (
            "INSERT INTO t SELECT count(*) FROM pragma_table_list;",
            "table `pragma_table_list`",
        ),
        (
            "WITH pages AS (SELECT 1 FROM dbstat) INSERT INTO t SELECT count(*) FROM pages;",
            "table `dbstat`",
        ),
    ] {
        let f = Fixture::new();
        f.sql(
            "0001_initial.sql",
            &format!("CREATE TABLE t(n INTEGER); {sql}"),
        );
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{sql}: {r}");
        assert_eq!(
            diagnostic(&r, "DB_SQL_UNSUPPORTED")["message"],
            format!("Statement 2 uses {refused}, which SQLite migrations cannot use."),
            "{r}"
        );
        assert_eq!(schema_objects(&f), 0);
    }
}

/// The counters SQLite keeps in `sqlite_sequence`, by table.
fn counters(f: &Fixture) -> Vec<(String, i64)> {
    f.target()
        .prepare("SELECT name, seq FROM sqlite_sequence ORDER BY name")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

/// F3: a `sqlite_rebuild` of an AUTOINCREMENT table keeps its counter.
/// Copying the rows into the replacement starts its counter from the
/// highest id copied, so deleted ids would be given out again; the guard
/// refuses that, naming the table and both values, and its next_action is
/// the line that copies the counter, which then migrates.
#[test]
fn sqlite_rebuild_keeps_the_autoincrement_counter() {
    let f = Fixture::new();
    f.sql(
        "0001_orders.sql",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, ref TEXT NOT NULL); \
         CREATE TABLE lines (id INTEGER PRIMARY KEY, order_id INTEGER NOT NULL REFERENCES orders (id) ON DELETE CASCADE);",
    );
    cycle(&f, "0001.json");
    f.target()
        .execute_batch(
            "INSERT INTO orders (ref) VALUES ('a'), ('b'), ('c'); DELETE FROM orders WHERE id = 3; \
             INSERT INTO lines (order_id) VALUES (1), (2);",
        )
        .unwrap();
    assert_eq!(counters(&f), [("orders".to_owned(), 3)]);
    f.sql("0002_rebuild.toml", "sqlite_rebuild = \"orders\"");
    let rebuild = |counter: &str| {
        format!(
            "CREATE TABLE orders_new (id INTEGER PRIMARY KEY AUTOINCREMENT, ref TEXT NOT NULL CHECK (length(ref) > 0)); \
             INSERT INTO orders_new (id, ref) SELECT id, ref FROM orders; {counter} \
             DROP TABLE orders; ALTER TABLE orders_new RENAME TO orders;"
        )
    };
    f.sql("0002_rebuild.sql", &rebuild(""));
    // Replay's orders is empty, so the plan passes; the target's is not.
    let p = f.plan("lost.json");
    let r = f.apply("lost.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "fail", "{r}");
    let d = diagnostic(&r, "DB_REBUILD_GUARD");
    assert_eq!(
        d["message"],
        "The rebuild of table `orders` did not keep what a rebuild must: the AUTOINCREMENT counter of table `orders` went back from 3 to 2 (sqlite_sequence), so SQLite would give new rows the ids of deleted ones. Row values suppressed.",
        "{r}"
    );
    let line = "DELETE FROM sqlite_sequence WHERE name='orders_new'; INSERT INTO sqlite_sequence (name, seq) SELECT 'orders_new', seq FROM sqlite_sequence WHERE name='orders';";
    assert!(
        d["next_action"].as_str().unwrap().starts_with(&format!(
            "Before the DROP TABLE, copy the counter to the replacement table: `{line}`."
        )),
        "{r}"
    );
    assert_eq!(d["objects"], json!(["table:\"orders\""]), "{r}");
    assert_eq!(d["source"], "0002_rebuild.sql", "{r}");
    assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    assert_eq!(counters(&f), [("orders".to_owned(), 3)]);
    // The line the next_action gives, before the DROP, keeps the counter.
    f.sql("0002_rebuild.sql", &rebuild(line));
    cycle(&f, "kept.json");
    assert_eq!(counters(&f), [("orders".to_owned(), 3)]);
    let c = f.target();
    c.execute_batch("INSERT INTO orders (ref) VALUES ('d')")
        .unwrap();
    let id: i64 = c
        .query_row("SELECT id FROM orders WHERE ref = 'd'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(id, 4);
}

/// F3, review: a statement that writes the replacement, even an INSERT ...
/// SELECT that copies no row, gives it a counter row, and SQLite reads and
/// updates only the first row for a name. A second row is refused, naming
/// the table and the count, and the one repair the next_action gives keeps
/// the counter whether the replacement has a row, has none, or the table
/// never had a counter.
#[test]
fn sqlite_rebuild_counter_repair_holds_with_or_without_a_counter_row() {
    let repair = "DELETE FROM sqlite_sequence WHERE name='orders_new'; INSERT INTO sqlite_sequence (name, seq) SELECT 'orders_new', seq FROM sqlite_sequence WHERE name='orders';";
    let create = "CREATE TABLE orders_new (id INTEGER PRIMARY KEY AUTOINCREMENT, ref TEXT NOT NULL CHECK (length(ref) > 0));";
    let copy = "INSERT INTO orders_new (id, ref) SELECT id, ref FROM orders;";
    let swap = "DROP TABLE orders; ALTER TABLE orders_new RENAME TO orders;";
    let emptied = "INSERT INTO orders (ref) VALUES ('a'), ('b'), ('c'); DELETE FROM orders;";
    let migrated = |before: &str| {
        let f = Fixture::new();
        f.sql(
            "0001_orders.sql",
            "CREATE TABLE orders (id INTEGER PRIMARY KEY AUTOINCREMENT, ref TEXT NOT NULL);",
        );
        cycle(&f, "0001.json");
        f.target().execute_batch(before).unwrap();
        f.sql("0002_rebuild.toml", "sqlite_rebuild = \"orders\"");
        f
    };
    let next_id = |f: &Fixture| -> i64 {
        let c = f.target();
        c.execute_batch("INSERT INTO orders (ref) VALUES ('next')")
            .unwrap();
        c.query_row("SELECT id FROM orders WHERE ref = 'next'", [], |r| r.get(0))
            .unwrap()
    };
    // The line for a copy of no row that #455 gave first, after the copy,
    // leaves `orders` two rows: 0 first, then 3.
    let f = migrated(emptied);
    let second = "INSERT INTO sqlite_sequence (name, seq) SELECT 'orders_new', seq FROM sqlite_sequence WHERE name='orders';";
    f.sql(
        "0002_rebuild.sql",
        &format!("{create} {copy} {second} {swap}"),
    );
    let p = f.plan("twice.json");
    let r = f.apply("twice.json", p["plan_digest"].as_str());
    assert_eq!(r["outcome"], "fail", "{r}");
    let d = diagnostic(&r, "DB_REBUILD_GUARD");
    assert_eq!(
        d["message"],
        "The rebuild of table `orders` did not keep what a rebuild must: table `orders` has 2 AUTOINCREMENT counters in sqlite_sequence, and SQLite reads and updates only the first, so it could give new rows the ids of deleted ones. Row values suppressed.",
        "{r}"
    );
    assert!(
        d["next_action"].as_str().unwrap().starts_with(&format!(
            "Before the DROP TABLE, copy the counter to the replacement table: `{repair}`."
        )),
        "{r}"
    );
    assert_eq!(d["objects"], json!(["table:\"orders\""]), "{r}");
    assert_eq!(counters(&f), [("orders".to_owned(), 3)]);
    // The repair, with the copy of no row, migrates.
    f.sql(
        "0002_rebuild.sql",
        &format!("{create} {copy} {repair} {swap}"),
    );
    cycle(&f, "repaired.json");
    assert_eq!(counters(&f), [("orders".to_owned(), 3)]);
    assert_eq!(next_id(&f), 4);
    // Nothing writes the replacement, so it has no counter row to replace.
    let f = migrated(emptied);
    f.sql("0002_rebuild.sql", &format!("{create} {repair} {swap}"));
    cycle(&f, "absent.json");
    assert_eq!(counters(&f), [("orders".to_owned(), 3)]);
    assert_eq!(next_id(&f), 4);
    // `orders` never had a counter: the repair leaves the replacement none.
    let f = migrated("");
    f.sql(
        "0002_rebuild.sql",
        &format!("{create} {copy} {repair} {swap}"),
    );
    cycle(&f, "none.json");
    assert_eq!(counters(&f), []);
    assert_eq!(next_id(&f), 1);
}

/// F6, review: the ADD COLUMN advice follows SQLite's own refusal of an
/// ALTER TABLE, a plain error, not its text: a trigger that raises the same
/// text (SQLITE_CONSTRAINT_TRIGGER) gets the advice for its statement.
#[test]
fn sqlite_trigger_raising_add_column_text_gets_statement_advice() {
    let f = Fixture::new();
    f.sql(
        "0001_t.sql",
        "CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT); \
         CREATE TRIGGER t_guard BEFORE INSERT ON t BEGIN SELECT RAISE(ABORT, 'Cannot add a UNIQUE column'); END;",
    );
    f.sql("0002_insert.sql", "INSERT INTO t (a) VALUES ('x');");
    let r = f.command("plan", &[]);
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_SQLITE");
    assert_eq!(
        d["message"],
        "Statement 1: a trigger raised: Cannot add a UNIQUE column (SQLITE_CONSTRAINT_TRIGGER).",
        "{r}"
    );
    let next_action = d["next_action"].as_str().unwrap();
    assert!(
        next_action.starts_with("Fix statement 1 of the migration named in source"),
        "{r}"
    );
    assert!(!next_action.contains("ADD COLUMN"), "{r}");
}

/// F1, review: when the history itself inserted the rows a drop breaks,
/// replay fails the drop, and the next_action says that replay found it
/// and the migration must change, not that the target rolled back.
#[test]
fn sqlite_drop_failure_found_in_replay_says_to_fix_the_migration() {
    let f = Fixture::new();
    f.sql(
        "0001_tables.sql",
        "CREATE TABLE p (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
         CREATE TABLE c (id INTEGER PRIMARY KEY, p_id INTEGER NOT NULL REFERENCES p (id)); \
         INSERT INTO p VALUES (1, 'p'); INSERT INTO c VALUES (1, 1);",
    );
    f.sql(
        "0002_p.sql",
        "CREATE TABLE p_new (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT); \
         INSERT INTO p_new (id, name) SELECT id, name FROM p; DROP TABLE p; \
         ALTER TABLE p_new RENAME TO p;",
    );
    let r = f.command("plan", &[]);
    assert_ne!(r["outcome"], "pass", "{r}");
    let d = diagnostic(&r, "DB_SQLITE");
    assert_eq!(
        d["message"], "Statement 3: FOREIGN KEY constraint failed (SQLITE_CONSTRAINT_FOREIGNKEY).",
        "{r}"
    );
    let next_action = d["next_action"].as_str().unwrap();
    assert!(
        next_action.starts_with("Statement 3 drops table `p`, which `c` references. SQLite deletes every row of a dropped table first, so the drop fails while rows reference it. Replay found this failure: the history inserted those rows"),
        "{r}"
    );
    assert!(next_action.contains("sqlite_rebuild = \"p\""), "{r}");
    assert!(
        next_action.ends_with("Then rerun plan. Replay changed nothing on the target."),
        "{r}"
    );
    assert!(!next_action.contains("failed on the target"), "{r}");
    assert_eq!(schema_objects(&f), 0);
}

/// F2: DB_REBUILD_GUARD names the rule a rebuild broke and the object,
/// never a row value: a recreated index whose stored SQL text differs
/// (whitespace and quoting are compared), a view not recreated, and a
/// write to another table.
#[test]
fn sqlite_rebuild_guard_names_what_the_rebuild_did_not_keep() {
    let f = Fixture::new();
    f.sql(
        "0001_posts.sql",
        "CREATE TABLE \"posts\" (\"id\" integer PRIMARY KEY NOT NULL, \"user_id\" integer NOT NULL, \"title\" varchar NOT NULL); \
         CREATE INDEX \"index_posts_on_user_id\" ON \"posts\" (\"user_id\"); \
         CREATE VIEW titles AS SELECT id, title FROM posts; \
         CREATE TABLE log (id INTEGER PRIMARY KEY, msg TEXT);",
    );
    cycle(&f, "0001.json");
    f.sql("0002_rebuild.toml", "sqlite_rebuild = \"posts\"");
    let rebuild = |after: &str| {
        format!(
            "DROP VIEW titles; \
             CREATE TABLE posts_new (\"id\" integer PRIMARY KEY NOT NULL, \"user_id\" integer NOT NULL, \"title\" varchar NOT NULL CHECK (length(title) <= 200)); \
             INSERT INTO posts_new SELECT * FROM posts; DROP TABLE posts; ALTER TABLE posts_new RENAME TO posts; {after}"
        )
    };
    let index = "CREATE INDEX \"index_posts_on_user_id\" ON \"posts\" (\"user_id\");";
    let view = "CREATE VIEW titles AS SELECT id, title FROM posts;";
    for (after, broken, object) in [
        (
            format!("CREATE INDEX index_posts_on_user_id ON posts(user_id); {view}"),
            "index `index_posts_on_user_id` has another stored SQL text than before the rebuild. SQLite compares the stored SQL text of a definition, so whitespace and quoting must match the original.",
            "index:\"index_posts_on_user_id\"",
        ),
        (
            index.to_owned(),
            "view `titles` is missing.",
            "view:\"titles\"",
        ),
        (
            format!("{index} {view} INSERT INTO log (msg) VALUES ('private_marker_f2');"),
            "the row count of table `log` changed from 0 to 1.",
            "table:\"log\"",
        ),
    ] {
        f.sql("0002_rebuild.sql", &rebuild(&after));
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{after}: {r}");
        let d = diagnostic(&r, "DB_REBUILD_GUARD");
        assert_eq!(
            d["message"],
            format!("The rebuild of table `posts` did not keep what a rebuild must: {broken} Row values suppressed."),
            "{r}"
        );
        assert_eq!(d["objects"], json!([object]), "{r}");
        assert_eq!(d["source"], "0002_rebuild.sql", "{r}");
        assert!(!r.to_string().contains("private_marker_f2"), "{r}");
    }
    f.sql("0002_rebuild.sql", &rebuild(&format!("{index} {view}")));
    cycle(&f, "0002.json");
}

/// F6: SQLite's ALTER TABLE ... ADD COLUMN cannot add a UNIQUE or PRIMARY
/// KEY column. The refusal is SQLite's own, not the autoindex SQLite asks
/// for on its copy of the table, and says what to write instead, which
/// migrates.
#[test]
fn sqlite_add_unique_column_says_to_add_a_unique_index() {
    let f = Fixture::new();
    f.sql(
        "0001_t.sql",
        "CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT);",
    );
    cycle(&f, "0001.json");
    for (sql, refusal, advice) in [
        (
            "ALTER TABLE t ADD COLUMN c TEXT UNIQUE;",
            "Cannot add a UNIQUE column",
            "SQLite's ALTER TABLE ... ADD COLUMN cannot add a UNIQUE column. In statement 1, add the column without UNIQUE, then create a unique index on it in the same migration: `ALTER TABLE <table> ADD COLUMN <column> <type>;` followed by `CREATE UNIQUE INDEX <table>_<column>_key ON <table> (<column>);`.",
        ),
        (
            "ALTER TABLE t ADD COLUMN c TEXT PRIMARY KEY;",
            "Cannot add a PRIMARY KEY column",
            "SQLite's ALTER TABLE ... ADD COLUMN cannot add a PRIMARY KEY column. Instead of statement 1, rebuild the table with a `sqlite_rebuild` sidecar",
        ),
    ] {
        f.sql("0002_add.sql", sql);
        let r = f.command("plan", &[]);
        assert_ne!(r["outcome"], "pass", "{sql}: {r}");
        let d = diagnostic(&r, "DB_SQLITE");
        assert_eq!(
            d["message"],
            format!("Statement 1: {refusal} (SQLITE_ERROR)."),
            "{r}"
        );
        assert!(
            d["next_action"].as_str().unwrap().starts_with(advice),
            "{r}"
        );
        assert!(!r.to_string().contains("sqlite_altertab"), "{r}");
    }
    f.sql(
        "0002_add.sql",
        "ALTER TABLE t ADD COLUMN c TEXT; CREATE UNIQUE INDEX t_c_key ON t (c);",
    );
    cycle(&f, "0002.json");
}

/// F7 and F1: what replay's empty tables cannot show fails on the target
/// with what to do. ADD COLUMN with a non-constant default or NOT NULL
/// without one on a table with rows; and a drop of a table that other
/// tables reference without an ON DELETE action, whose immediate key fails
/// the DROP and whose deferred key fails the COMMIT.
#[test]
fn sqlite_target_only_failures_say_what_to_do() {
    let f = Fixture::new();
    f.sql(
        "0001_tables.sql",
        "CREATE TABLE t (id INTEGER PRIMARY KEY, a TEXT); \
         CREATE TABLE p (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
         CREATE TABLE c (id INTEGER PRIMARY KEY, p_id INTEGER NOT NULL REFERENCES p (id)); \
         CREATE TABLE q (id INTEGER PRIMARY KEY, name TEXT NOT NULL); \
         CREATE TABLE d (id INTEGER PRIMARY KEY, q_id INTEGER NOT NULL REFERENCES q (id) DEFERRABLE INITIALLY DEFERRED);",
    );
    cycle(&f, "0001.json");
    f.target()
        .execute_batch(
            "INSERT INTO t (a) VALUES ('x'); INSERT INTO p VALUES (1, 'p'); INSERT INTO c VALUES (1, 1); \
             INSERT INTO q VALUES (1, 'q'); INSERT INTO d VALUES (1, 1);",
        )
        .unwrap();
    let rebuild = |table: &str| {
        format!(
            "CREATE TABLE {table}_new (id INTEGER PRIMARY KEY, name TEXT NOT NULL, note TEXT); \
             INSERT INTO {table}_new (id, name) SELECT id, name FROM {table}; DROP TABLE {table}; \
             ALTER TABLE {table}_new RENAME TO {table};"
        )
    };
    for (n, (sql, message, advice)) in [
        (
            "ALTER TABLE t ADD COLUMN created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP;".to_owned(),
            "Statement 1: Cannot add a column with non-constant default (SQLITE_ERROR).",
            "SQLite's ALTER TABLE ... ADD COLUMN gives every existing row the new column's default, so on a table that has rows it accepts only a constant default",
        ),
        (
            "ALTER TABLE t ADD COLUMN b TEXT NOT NULL;".to_owned(),
            "Statement 1: Cannot add a NOT NULL column with default value NULL (SQLITE_ERROR).",
            "SQLite's ALTER TABLE ... ADD COLUMN cannot add a NOT NULL column without a default to a table that has rows",
        ),
        (
            rebuild("p"),
            "Statement 3: FOREIGN KEY constraint failed (SQLITE_CONSTRAINT_FOREIGNKEY).",
            "Statement 3 drops table `p`, which `c` references. SQLite deletes every row of a dropped table first",
        ),
        (
            rebuild("q"),
            "SQLite could not commit the migration: FOREIGN KEY constraint failed (SQLITE_CONSTRAINT_FOREIGNKEY).",
            "The migration dropped table `q`, which `d` references with a deferred foreign key.",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        f.sql("0002_change.sql", &sql);
        let plan = format!("0002-{n}.json");
        let p = f.plan(&plan);
        let r = f.apply(&plan, p["plan_digest"].as_str());
        assert_eq!(r["outcome"], "fail", "{sql}: {r}");
        let d = diagnostic(&r, "DB_SQLITE");
        assert_eq!(d["message"], message, "{r}");
        let next_action = d["next_action"].as_str().unwrap();
        assert!(next_action.starts_with(advice), "{r}");
        assert!(next_action.contains("sqlite_rebuild"), "{r}");
        assert!(next_action.contains("Explicit one-table rebuild"), "{r}");
        assert_eq!(d["source"], "0002_change.sql", "{r}");
        assert_eq!(r["progress"][1]["state"], "failed", "{r}");
    }
    let columns: i64 = f
        .target()
        .query_row("SELECT count(*) FROM pragma_table_info('t')", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(columns, 2);
}

/// F8: a saved plan that `ntnt db plan` could not make (outcome fail, no
/// plan and no plan_digest) is refused as blocked, not as edited, even
/// when approval holds jq's `null`.
#[test]
fn sqlite_saved_plan_without_a_plan_is_blocked() {
    let f = Fixture::new();
    f.target()
        .execute_batch("CREATE TABLE legacy (id INTEGER PRIMARY KEY);")
        .unwrap();
    f.sql("0001_t.sql", "CREATE TABLE t (id INTEGER PRIMARY KEY);");
    let p = f.command("plan", &["--out", "blocked.json"]);
    assert_eq!(p["outcome"], "fail", "{p}");
    diagnostic(&p, "DB_UNRECOGNIZED_HISTORY");
    let saved: Value =
        serde_json::from_slice(&std::fs::read(f.root.path().join("blocked.json")).unwrap())
            .unwrap();
    assert_eq!(saved["outcome"], "fail", "{saved}");
    assert!(
        saved.get("plan_digest").is_none_or(Value::is_null),
        "{saved}"
    );
    let r = f.apply("blocked.json", Some("null"));
    assert_eq!(r["outcome"], "fail", "{r}");
    let d = diagnostic(&r, "DB_PLAN_STALE");
    assert_eq!(
        d["message"],
        "Saved plan is blocked: its outcome is fail, and it holds no plan because `ntnt db plan` stopped before it could make one. A blocked plan is never executed.",
        "{r}"
    );
    assert!(
        d["next_action"]
            .as_str()
            .unwrap()
            .contains("Approve a plan only when its outcome is pass and it has a plan_digest."),
        "{r}"
    );
    assert_eq!(schema_objects(&f), 1);
}
