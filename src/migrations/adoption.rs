//! Explicit baseline identity, distinct from evidence of executed migrations.
use super::inventory::CapturedInputs;
use super::model::History;
use super::{Diagnostic, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Captured adoption action; count is covered source files, not executed SQL.
pub(super) struct Adoption {
    pub kind: String,
    pub through_id: String,
    pub count: usize,
    pub prefix_checksum: String,
    pub checks: Vec<String>,
}
fn invalid() -> Diagnostic {
    Diagnostic::new("DB_ADOPTION_INPUT","Adoption requires an exact captured prefix, named adoption_checks, and either no ledger or a fully matching recognized legacy v1 ledger. No history is inferred or repaired.")
}
/// A refusal for one adoption requirement, with advice for it (#422).
fn refuse(message: &str, next_action: &str) -> Diagnostic {
    let mut d = Diagnostic::new("DB_ADOPTION_INPUT", message);
    d.next_action = next_action.to_owned();
    d
}
impl Adoption {
    fn new(inputs: &CapturedInputs, count: usize, kind: &str) -> Result<Self> {
        if inputs.config.adoption_checks.is_empty() {
            let table = format!("[databases.{}.adoption_checks]", inputs.database_name);
            let schema = &inputs.config.schema;
            return Err(refuse(
                &format!("Adoption needs at least one named check, and {table} declares none. No history is inferred or repaired."),
                &format!("Add a {table} table to the manifest with at least one named check that the existing data must pass, such as no_null_email = \"SELECT NOT EXISTS (SELECT 1 FROM {schema}.accounts WHERE email IS NULL)\", then plan again. See docs/DATABASE_MIGRATIONS.md#explicit-adoption-of-existing-databases."),
            ));
        }
        if !(kind == "adopt_baseline"
            || (kind == "import_legacy_v1" && inputs.config.engine == "postgresql"))
        {
            return Err(invalid());
        }
        let checkpoint = inputs.checkpoint(count)?;
        Ok(Self {
            kind: kind.into(),
            through_id: checkpoint.id,
            count,
            prefix_checksum: checkpoint.checksum,
            checks: inputs.config.adoption_checks.keys().cloned().collect(),
        })
    }
    /// Resolve the explicit ID/check policy before any database I/O.
    pub fn selected(inputs: &CapturedInputs, through: u64) -> Result<Self> {
        let count = inputs
            .migrations
            .iter()
            .position(|m| m.id == through)
            .map(|n| n + 1)
            .ok_or_else(|| {
                let dir = &inputs.config.migrations_dir;
                match inputs.migrations.last() {
                    Some(last) => refuse(
                        &format!("Migration ID {through} is not in {dir}; the highest ID there is {}.", last.id),
                        &format!("Pass --adopt-through with the ID of the last migration the existing database already matches, such as --adopt-through {}: a migration ID in {dir}, not a count of files.", last.id),
                    ),
                    None => refuse(
                        &format!("Migration ID {through} is not in {dir}, which holds no migrations."),
                        &format!("Add the migrations the existing database already matches to {dir}, then pass the ID of the last one to --adopt-through."),
                    ),
                }
            })?;
        Self::new(inputs, count, "adopt_baseline")
    }
    /// Select an absent-ledger checkpoint or a fully matching recognized import.
    pub fn request(inputs: &CapturedInputs, through: u64, history: &History) -> Result<Self> {
        let mut action = Self::selected(inputs, through)?;
        if history.present && history.scope.is_none() && inputs.config.engine == "postgresql" {
            action.kind = "import_legacy_v1".into();
        }
        action.before(inputs, history)?;
        Ok(action)
    }
    /// Reconstruct the descriptor from captured inputs before resolving approval.
    pub fn from_plan(inputs: &CapturedInputs, saved: &Value) -> Result<Option<Self>> {
        let Some(value) = saved["plan"].get("action") else {
            return Ok(None);
        };
        let action: Self = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
        if action != Self::new(inputs, action.count, &action.kind)? {
            return Err(invalid());
        }
        Ok(Some(action))
    }
    /// Covered-prefix index expected before this adoption action.
    pub fn base_count(&self) -> usize {
        if self.kind == "import_legacy_v1" {
            self.count
        } else {
            0
        }
    }
    /// Validate original ledger eligibility without mutating or repairing it.
    pub fn before(&self, inputs: &CapturedInputs, history: &History) -> Result<()> {
        const NEVER_FORCE: &str = "never delete, reset or rewrite a ledger to force adoption";
        if self.kind == "import_legacy_v1" {
            if !history.present || history.scope.is_some() || inputs.config.engine != "postgresql" {
                return Err(invalid());
            }
            if history.entries.is_empty() {
                return Err(refuse(
                    "The target's legacy v1 ledger is empty, so it records no history to import. No history is inferred or repaired.",
                    &format!("Inspect _ntnt.schema_migrations_v1 on the target with an operator: an empty legacy ledger is not eligible for adoption; {NEVER_FORCE}."),
                ));
            }
            if history.entries.iter().any(|e| e.state != "applied") {
                return Err(refuse(
                    "The target's legacy v1 ledger records migrations that are not applied, so it is not eligible for import. No history is inferred or repaired.",
                    &format!("Investigate the unapplied entries in _ntnt.schema_migrations_v1 with an operator; {NEVER_FORCE}."),
                ));
            }
            let covered = super::report::prefix(history, inputs)?;
            if covered != self.count {
                let last = history.entries.last().map_or("", |e| e.id.as_str());
                return Err(refuse(
                    &format!(
                        "The target's legacy v1 ledger records {covered} migration(s), through ID {last}, but --adopt-through {} covers {}; a legacy ledger is imported only through its last migration.",
                        self.through_id, self.count
                    ),
                    &format!("Pass --adopt-through {last}, the legacy ledger's last migration ID."),
                ));
            }
        } else if history.present || !history.entries.is_empty() || history.scope.is_some() {
            let entries = history.entries.len();
            return Err(refuse(
                &format!(
                    "The target already has an ntnt migration ledger ({entries} {}), so it is not eligible for adoption, which only records a baseline for a database without one.",
                    if entries == 1 { "entry" } else { "entries" }
                ),
                &format!("Its history is already recorded: plan and migrate without --adopt-through; {NEVER_FORCE}."),
            ));
        }
        Ok(())
    }
    /// Exact expected ledger for post-write validation and fresh reconciliation.
    pub fn after(&self, inputs: &CapturedInputs) -> Result<History> {
        Ok(History {
            present: true,
            scope: Some(json!({"database":inputs.database_name,"schema":inputs.config.schema})),
            entries: if self.kind == "import_legacy_v1" {
                inputs
                    .migrations
                    .iter()
                    .take(self.count)
                    .enumerate()
                    .map(|(i, m)| super::model::HistoryEntry {
                        position: i as i64 + 1,
                        id: m.id.to_string(),
                        name: m.name.clone(),
                        checksum: m.checksum.clone(),
                        state: "imported".into(),
                    })
                    .collect()
            } else {
                vec![inputs.checkpoint(self.count)?]
            },
        })
    }
}
pub(super) fn format(engine: &str) -> &'static str {
    if engine == "sqlite" {
        "sqlite-adoption-v1"
    } else {
        "postgresql-adoption-v1"
    }
}

/// Where the docs say how to retire an application's previous migration ledger.
const RETIRE_LEDGER_DOCS: &str =
    "docs/DATABASE_MIGRATIONS.md#retiring-an-applications-previous-migration-ledger";
/// PostgreSQL keywords that cannot name a schema unquoted: the reserved ones,
/// and those reserved except as function or type names.
const POSTGRES_RESERVED: &str = "all analyse analyze and any array as asc asymmetric authorization binary both case cast check collate collation column concurrently constraint create cross current_catalog current_date current_role current_schema current_time current_timestamp current_user default deferrable desc distinct do else end except false fetch for foreign freeze from full grant group having ilike in initially inner intersect into is isnull join lateral leading left like limit localtime localtimestamp natural not notnull null offset on only or order outer overlaps placing primary references returning right select session_user similar some symmetric system_user table tablesample then to trailing true union unique user using variadic verbose when where window with";
/// Every SQLite keyword; quoting one that SQLite would also take bare is harmless.
const SQLITE_KEYWORDS: &str = "abort action add after all alter always analyze and as asc attach autoincrement before begin between by cascade case cast check collate column commit conflict constraint create cross current current_date current_time current_timestamp database default deferrable deferred delete desc detach distinct do drop each else end escape except exclude exclusive exists explain fail filter first following for foreign from full generated glob group groups having if ignore immediate in index indexed initially inner insert instead intersect into is isnull join key last left like limit match materialized natural no not nothing notnull null nulls of offset on or order others outer over partition plan pragma preceding primary query raise range recursive references regexp reindex release rename replace restrict returning right rollback row rows savepoint select set table temp temporary then ties to transaction trigger unbounded union unique update using vacuum values view virtual when where window with without";

/// `name` as SQL on `engine` writes it: bare when it is lowercase ASCII
/// letters, digits and underscores, starts with a letter or underscore and is
/// no keyword that would need quoting; double-quoted otherwise.
fn sql_name(engine: &str, name: &str) -> String {
    let keywords = if engine == "sqlite" {
        SQLITE_KEYWORDS
    } else {
        POSTGRES_RESERVED
    };
    let bare = name
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        && !keywords.split(' ').any(|keyword| keyword == name);
    if bare {
        name.to_owned()
    } else {
        format!("\"{}\"", name.replace('"', "\"\""))
    }
}

/// The schema and name, as SQL writes them, and the name itself, of a
/// table that a drift difference with catalog key `key` and observed
/// definition `observed` names; `None` for any other object. PostgreSQL keys
/// a table in the managed schema `relation:<name>` and one elsewhere
/// `<quoted schema>.relation:<name>` (a relation of kind `r` or `p`); SQLite
/// keys it `table:"<name>"`.
fn table(
    engine: &str,
    schema: &str,
    key: &str,
    observed: &Value,
) -> Option<(String, String, String)> {
    if engine == "sqlite" {
        let quoted = key.strip_prefix("table:\"")?.strip_suffix('"')?;
        let name = quoted.replace("\"\"", "\"");
        return Some(("main".into(), sql_name(engine, &name), name));
    }
    if !matches!(observed["kind"].as_str(), Some("r" | "p")) {
        return None;
    }
    if let Some(name) = key.strip_prefix("relation:") {
        return Some((
            sql_name(engine, schema),
            sql_name(engine, name),
            name.to_owned(),
        ));
    }
    // quote_ident quotes a schema name that holds a dot or a colon, and doubles
    // any quote inside it, so its key ends at the first quote that is not
    // doubled; a bare one ends at the first dot.
    let end = if key.starts_with('"') {
        let mut quotes = key.match_indices('"').skip(1).peekable();
        loop {
            let (at, _) = quotes.next()?;
            if quotes.peek().is_some_and(|(next, _)| *next == at + 1) {
                quotes.next();
            } else {
                break at + 1;
            }
        }
    } else {
        key.find('.')?
    };
    let (qualifier, rest) = key.split_at(end);
    let name = rest.strip_prefix(".relation:")?;
    Some((
        qualifier.to_owned(),
        sql_name(engine, name),
        name.to_owned(),
    ))
}

/// Advice for an adoption of the migrations through `through_id` that is
/// refused as drift because the target holds tables those migrations do not
/// create, such as the application's previous migration ledger (#431). It
/// names those tables, from the drift `differences` and never from rows, says
/// how to retire a ledger on `engine` before planning again, and otherwise
/// gives the `ordinary` advice. `None` when no such table differs. The refusal
/// stays: an unexpected object is never adopted, moved or dropped. On SQLite
/// the steps lose no data when followed as written: the shell stops at the
/// first error, a table that refers to the ledger is found and its foreign
/// key removed first, and the ledger is dropped only once its copy holds
/// every row.
pub(super) fn retire_ledger_advice(
    ordinary: &str,
    engine: &str,
    schema: &str,
    through_id: &str,
    differences: &Value,
) -> Option<String> {
    let tables: Vec<(String, String, String)> = differences
        .as_array()?
        .iter()
        .filter(|d| d["change"] == "unexpected")
        .filter_map(|d| table(engine, schema, d["object"].as_str()?, &d["observed"]))
        .collect();
    let listed: Vec<String> = tables.iter().map(|(s, n, _)| format!("{s}.{n}")).collect();
    // The ledger's name in SQL, its schema-qualified name, and the name as a
    // SQL string; `<table>` when there are several to choose from.
    let (held, subject, qualified, name, literal) = match tables.as_slice() {
        [] => return None,
        [(s, n, raw)] => (
            format!("a table that the migrations through ID {through_id} do not create: {s}.{n}"),
            "it is",
            format!("{s}.{n}"),
            n.clone(),
            format!("'{}'", raw.replace('\'', "''")),
        ),
        _ => (
            format!(
                "tables that the migrations through ID {through_id} do not create: {}",
                listed.join(", ")
            ),
            "one of them is",
            "<table>".to_owned(),
            "<table>".to_owned(),
            "'<table>'".to_owned(),
        ),
    };
    let several = tables.len() > 1;
    let steps = if engine == "sqlite" {
        let each = if several {
            "writing the ledger's table name for <table> in each command, "
        } else {
            ""
        };
        format!("stop the old migration runner so it cannot recreate the table; {each}open the target with `sqlite3 -bail` in the directory where legacy-ledger.sqlite should be written, so that the shell stops at the first error; check that `SELECT s.name FROM main.sqlite_schema AS s, pragma_foreign_key_list(s.name) AS f WHERE s.type = 'table' AND s.name <> {literal} COLLATE NOCASE AND f.\"table\" = {literal} COLLATE NOCASE;` lists no table, and if it lists any, remove their foreign keys into the ledger with a reviewed migration first, because dropping the ledger would delete or change their rows (never turn foreign-key enforcement off instead); copy the table with `ATTACH DATABASE 'legacy-ledger.sqlite' AS legacy; CREATE TABLE legacy.{name} AS SELECT * FROM main.{name}; SELECT (SELECT count(*) FROM legacy.{name}) = (SELECT count(*) FROM main.{name}) AND NOT EXISTS (SELECT * FROM main.{name} EXCEPT SELECT * FROM legacy.{name});`, which must print 1; only then drop it with `DROP TABLE main.{name};`; then plan the adoption again")
    } else {
        let placeholder = if several {
            ", with <table> the ledger's name as listed"
        } else {
            ""
        };
        format!("stop the old migration runner so it cannot recreate the table, then move the table to a schema of its own with `CREATE SCHEMA IF NOT EXISTS app_legacy; ALTER TABLE {qualified} SET SCHEMA app_legacy;`{placeholder} (or archive its rows and drop it), and plan the adoption again")
    };
    let mut otherwise = ordinary.chars();
    let otherwise = otherwise
        .next()
        .map(|first| first.to_lowercase().chain(otherwise).collect::<String>())
        .unwrap_or_default();
    Some(format!("The target holds {held}. If {subject} the application's previous migration ledger, retire it as {RETIRE_LEDGER_DOCS} describes: {steps}. Otherwise, {otherwise}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDINARY: &str = "Review drift.differences against the applied release. Reconcile deliberately; these commands never repair or adopt state.";

    /// An unexpected drift difference, observed with relation `kind` if any.
    fn unexpected(object: &str, kind: Option<&str>) -> Value {
        json!({"object": object, "change": "unexpected", "expected": null,
            "observed": kind.map_or(json!({}), |kind| json!({"kind": kind}))})
    }

    #[test]
    fn a_postgresql_ledger_is_named_with_the_commands_that_retire_it() {
        let differences = json!([
            unexpected("column:schema_migrations.version", None),
            unexpected("constraint:schema_migrations.schema_migrations_pkey", None),
            unexpected("relation:schema_migrations", Some("r")),
            unexpected("relation:schema_migrations_pkey", Some("i")),
        ]);
        assert_eq!(
            retire_ledger_advice(ORDINARY, "postgresql", "public", "33", &differences).unwrap(),
            "The target holds a table that the migrations through ID 33 do not create: public.schema_migrations. If it is the application's previous migration ledger, retire it as docs/DATABASE_MIGRATIONS.md#retiring-an-applications-previous-migration-ledger describes: stop the old migration runner so it cannot recreate the table, then move the table to a schema of its own with `CREATE SCHEMA IF NOT EXISTS app_legacy; ALTER TABLE public.schema_migrations SET SCHEMA app_legacy;` (or archive its rows and drop it), and plan the adoption again. Otherwise, review drift.differences against the applied release. Reconcile deliberately; these commands never repair or adopt state."
        );
    }

    #[test]
    fn a_sqlite_ledger_is_named_with_the_commands_that_retire_it() {
        let differences = json!([
            unexpected("column:\"schema_migrations\".\"version\"", None),
            unexpected("index:\"sqlite_autoindex_schema_migrations_1\"", None),
            unexpected("table:\"schema_migrations\"", None),
        ]);
        assert_eq!(
            retire_ledger_advice(ORDINARY, "sqlite", "main", "1", &differences).unwrap(),
            "The target holds a table that the migrations through ID 1 do not create: main.schema_migrations. If it is the application's previous migration ledger, retire it as docs/DATABASE_MIGRATIONS.md#retiring-an-applications-previous-migration-ledger describes: stop the old migration runner so it cannot recreate the table; open the target with `sqlite3 -bail` in the directory where legacy-ledger.sqlite should be written, so that the shell stops at the first error; check that `SELECT s.name FROM main.sqlite_schema AS s, pragma_foreign_key_list(s.name) AS f WHERE s.type = 'table' AND s.name <> 'schema_migrations' COLLATE NOCASE AND f.\"table\" = 'schema_migrations' COLLATE NOCASE;` lists no table, and if it lists any, remove their foreign keys into the ledger with a reviewed migration first, because dropping the ledger would delete or change their rows (never turn foreign-key enforcement off instead); copy the table with `ATTACH DATABASE 'legacy-ledger.sqlite' AS legacy; CREATE TABLE legacy.schema_migrations AS SELECT * FROM main.schema_migrations; SELECT (SELECT count(*) FROM legacy.schema_migrations) = (SELECT count(*) FROM main.schema_migrations) AND NOT EXISTS (SELECT * FROM main.schema_migrations EXCEPT SELECT * FROM legacy.schema_migrations);`, which must print 1; only then drop it with `DROP TABLE main.schema_migrations;`; then plan the adoption again. Otherwise, review drift.differences against the applied release. Reconcile deliberately; these commands never repair or adopt state."
        );
    }

    /// Several tables are all named, as SQL writes them, and the command
    /// takes the ledger's name in place of `<table>`.
    #[test]
    fn several_tables_are_named_and_the_command_takes_the_ledgers_name() {
        let differences = json!([
            unexpected("\"Odd.Sche\"\"ma\".relation:x", Some("r")),
            unexpected("audit.relation:log", Some("p")),
            unexpected("relation:SequelizeMeta", Some("r")),
            unexpected("relation:order", Some("r")),
            unexpected("relation:schema_migrations", Some("r")),
        ]);
        let advice =
            retire_ledger_advice(ORDINARY, "postgresql", "user", "2", &differences).unwrap();
        assert!(
            advice.starts_with("The target holds tables that the migrations through ID 2 do not create: \"Odd.Sche\"\"ma\".x, audit.log, \"user\".\"SequelizeMeta\", \"user\".\"order\", \"user\".schema_migrations. If one of them is the application's previous migration ledger,"),
            "{advice}"
        );
        assert!(
            advice.contains("`CREATE SCHEMA IF NOT EXISTS app_legacy; ALTER TABLE <table> SET SCHEMA app_legacy;`, with <table> the ledger's name as listed (or archive"),
            "{advice}"
        );
        let differences = json!([
            unexpected("table:\"order\"", None),
            unexpected("table:\"schema_migrations\"", None),
            unexpected("table:\"we\"\"ird\"", None),
        ]);
        let advice = retire_ledger_advice(ORDINARY, "sqlite", "main", "2", &differences).unwrap();
        assert!(
            advice.contains("do not create: main.\"order\", main.schema_migrations, main.\"we\"\"ird\". If one of them is"),
            "{advice}"
        );
        for part in [
            "cannot recreate the table; writing the ledger's table name for <table> in each command, open the target with `sqlite3 -bail` in the directory",
            "`SELECT s.name FROM main.sqlite_schema AS s, pragma_foreign_key_list(s.name) AS f WHERE s.type = 'table' AND s.name <> '<table>' COLLATE NOCASE AND f.\"table\" = '<table>' COLLATE NOCASE;` lists no table",
            "`ATTACH DATABASE 'legacy-ledger.sqlite' AS legacy; CREATE TABLE legacy.<table> AS SELECT * FROM main.<table>; SELECT (SELECT count(*) FROM legacy.<table>) = (SELECT count(*) FROM main.<table>) AND NOT EXISTS (SELECT * FROM main.<table> EXCEPT SELECT * FROM legacy.<table>);`, which must print 1; only then drop it with `DROP TABLE main.<table>;`; then plan",
        ] {
            assert!(advice.contains(part), "{part:?} is not in {advice:?}");
        }
    }

    /// A ledger whose name holds a quote is written as SQL needs it: quoted
    /// as a name, and with the quote doubled in the check's strings.
    #[test]
    fn a_sqlite_ledger_named_with_a_quote_is_written_as_sql_needs_it() {
        let differences = json!([unexpected("table:\"it's\"", None)]);
        let advice = retire_ledger_advice(ORDINARY, "sqlite", "main", "1", &differences).unwrap();
        for part in [
            "do not create: main.\"it's\".",
            "AND s.name <> 'it''s' COLLATE NOCASE AND f.\"table\" = 'it''s' COLLATE NOCASE;`",
            "`DROP TABLE main.\"it's\";`",
        ] {
            assert!(advice.contains(part), "{part:?} is not in {advice:?}");
        }
    }

    /// Drift with no table the history does not create keeps the ordinary advice.
    #[test]
    fn other_differences_keep_the_ordinary_advice() {
        let changed = |object: &str, change: &str| json!({"object": object, "change": change, "expected": {"kind": "r"}, "observed": {"kind": "r"}});
        for (engine, differences) in [
            ("postgresql", json!([])),
            (
                "postgresql",
                json!([
                    unexpected("column:t.extra", None),
                    unexpected("index:t_extra", None),
                    unexpected("relation:t_extra", Some("i")),
                    unexpected("relation:t_id_seq", Some("S")),
                    unexpected("relation:v", Some("v")),
                    unexpected("relation:m", Some("m")),
                    unexpected("schema:audit", None),
                    changed("relation:t", "changed"),
                    changed("relation:u", "missing"),
                ]),
            ),
            (
                "sqlite",
                json!([
                    unexpected("index:\"i\"", None),
                    unexpected("view:\"v\"", None),
                    unexpected("trigger:\"g\"", None),
                    unexpected("column:\"t\".\"extra\"", None),
                    changed("table:\"t\"", "changed"),
                    changed("table:\"u\"", "missing"),
                ]),
            ),
        ] {
            assert_eq!(
                retire_ledger_advice(ORDINARY, engine, "app", "1", &differences),
                None,
                "{differences}"
            );
        }
    }

    #[test]
    fn names_are_quoted_only_where_sql_needs_it() {
        for (engine, name, written) in [
            ("postgresql", "schema_migrations", "schema_migrations"),
            ("postgresql", "public", "public"),
            ("postgresql", "_sqlx_migrations", "_sqlx_migrations"),
            ("postgresql", "user", "\"user\""),
            ("postgresql", "SequelizeMeta", "\"SequelizeMeta\""),
            ("postgresql", "1st", "\"1st\""),
            ("postgresql", "we\"ird", "\"we\"\"ird\""),
            ("postgresql", "", "\"\""),
            ("sqlite", "main", "main"),
            ("sqlite", "user", "user"),
            ("sqlite", "order", "\"order\""),
            ("sqlite", "Settings", "\"Settings\""),
        ] {
            assert_eq!(sql_name(engine, name), written, "{engine} {name}");
        }
    }
}
