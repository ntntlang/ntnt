//! Dedicated SQLite sessions with native authorization and bounded worker I/O.
//! No pool, extension loading, implicit file creation or target repair.
use super::adapter::Access;
use super::inventory::{json_digest, Migration};
use super::model::{Catalog, History, HistoryEntry};
use super::sqlite_cache::{Cache, ForeignKey, Output, Stats};
use super::sqlite_file::FileBinding;
use super::{Diagnostic, Result};
use rusqlite::{
    config::DbConfig,
    fallible_iterator::FallibleIterator,
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    types::ValueRef,
    Connection, OpenFlags,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// How long SQLite waits for a lock another connection holds.
const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
/// How much longer than an operation's own deadline the session waits before
/// it discards the session: an operation SQLite can interrupt always returns
/// first, and only one stuck outside SQLite is discarded.
const DISCARD_GRACE: Duration = Duration::from_secs(5);
/// The stack of each thread that runs SQLite operations: the database
/// runtime gives its blocking threads this much (`database_runtime_on`).
/// Compiling a program recurses through every trigger and foreign-key action
/// it nests, up to SQLITE_LIMIT_TRIGGER_DEPTH (1,000) levels, and at that
/// limit needed up to 1.75 MiB, most of the 2 MiB std gives a thread by
/// default (#421). This is more than four times that.
pub(super) const OPERATION_STACK: usize = 8 << 20;
const LEDGER: &str = "_ntnt_schema_migrations_v2";
const LEDGER_SQL: &str = "CREATE TABLE _ntnt_schema_migrations_v2 (position INTEGER NOT NULL PRIMARY KEY,id TEXT NOT NULL,name TEXT NOT NULL,checksum TEXT NOT NULL,state TEXT NOT NULL,logical_database TEXT NOT NULL,managed_schema TEXT NOT NULL) STRICT";

#[derive(Clone, Debug, Serialize)]
pub(super) struct Identity {
    pub database: String,
    pub file_id: String,
    pub version: i32,
    pub settings: Value,
    pub authority: Value,
}

/// How a diagnostic about the session's file itself begins; the session
/// names the file's role and secret in its place (`attribute`).
const FILE_SUBJECT: &str = "The SQLite file ";

fn sql_error(error: rusqlite::Error) -> Diagnostic {
    // An error that points at a token of the input SQL carries its SQLite code
    // separately from an ordinary failure; classify both by that code.
    let (sqlite, text) = match &error {
        rusqlite::Error::SqliteFailure(error, text) => (Some(*error), text.as_deref()),
        rusqlite::Error::SqlInputError { error, msg, .. } => (Some(*error), Some(msg.as_str())),
        _ => (None, None),
    };
    if sqlite.is_some_and(|e| e.extended_code == rusqlite::ffi::SQLITE_READONLY_ROLLBACK) {
        return Diagnostic::new("DB_SQLITE_RECOVERY_REQUIRED", "SQLite requires rollback-journal recovery that read-only inspection cannot perform. A writer may still be active; details suppressed.");
    }
    let code = match sqlite.map(|e| e.code) {
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked) => "DB_BUSY",
        Some(rusqlite::ErrorCode::OperationInterrupted) => "DB_TIMEOUT",
        Some(rusqlite::ErrorCode::AuthorizationForStatementDenied) => "DB_SQL_UNSUPPORTED",
        _ => "DB_SQLITE",
    };
    // SQLite's own message only in shapes that hold identifiers alone, and
    // always the extended result code's name (#424).
    let message = match sqlite {
        // A locked file is not a statement's fault (#425). SQLITE_BUSY is
        // another connection's lock. The busy timeout bounds the wait for it,
        // but SQLite returns at once where waiting could deadlock, so the
        // message gives the limit and never claims the wait happened.
        Some(e) if e.code == rusqlite::ErrorCode::DatabaseBusy => format!(
            "{FILE_SUBJECT}is locked ({}): another connection holds its write lock or keeps it in EXCLUSIVE locking mode, or, in rollback-journal mode, is reading it while ntnt commits. ntnt waits up to its {}-second busy timeout for such a lock, and SQLite reports some conflicts at once, without waiting.",
            super::sqlite_errors::name(e.extended_code),
            BUSY_TIMEOUT.as_secs()
        ),
        // SQLITE_LOCKED is a conflict within one connection (ntnt opens no
        // shared cache), so no other writer is named.
        Some(e) if e.code == rusqlite::ErrorCode::DatabaseLocked => format!(
            "{FILE_SUBJECT}reported a lock conflict within ntnt's own connection to it ({}), not a lock held by another connection.",
            super::sqlite_errors::name(e.extended_code)
        ),
        // A file SQLite cannot write is not a statement's fault.
        Some(e) if e.code == rusqlite::ErrorCode::ReadOnly => format!(
            "{FILE_SUBJECT}is not writable ({}): {}.",
            super::sqlite_errors::name(e.extended_code),
            super::sqlite_errors::read_only_cause(e.extended_code)
        ),
        Some(e) => {
            let name = super::sqlite_errors::name(e.extended_code);
            match text.and_then(|text| super::sqlite_errors::detail(e.extended_code, text)) {
                Some(detail) => format!("{detail} ({name})."),
                None => format!(
                    "SQLite operation failed ({name}, code {}). Details suppressed.",
                    e.extended_code
                ),
            }
        }
        None => "SQLite operation failed (code -1). Details suppressed.".to_owned(),
    };
    let mut d = Diagnostic::new(code, &message);
    if sqlite.is_some_and(|e| e.code == rusqlite::ErrorCode::DatabaseLocked) {
        d.next_action = "SQLITE_LOCKED is a conflict inside ntnt's own connection, so waiting for another connection does not clear it. Rerun the same command (migrate, rerun with the same plan, skips any migration already committed); if it fails the same way again, report it as an ntnt bug at https://github.com/ntntlang/ntnt/issues with this message.".to_owned();
    }
    d
}
fn reserved(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("_ntnt")
}
fn internal(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("sqlite_")
}
fn schema_table(name: &str) -> bool {
    matches!(
        name,
        "sqlite_master" | "sqlite_schema" | "sqlite_temp_master" | "sqlite_temp_schema"
    )
}
/// SQLite's planner statistics tables: `sqlite_stat1`, `sqlite_stat4` in the
/// bundled build, and the legacy `sqlite_stat2` and `sqlite_stat3` an older
/// SQLite may have left in a file.
fn statistics_table(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "sqlite_stat1" | "sqlite_stat2" | "sqlite_stat3" | "sqlite_stat4"
    )
}
/// SQLite's own state tables, which hold no schema: the AUTOINCREMENT
/// counters (`sqlite_sequence`) and the planner statistics that `ANALYZE` and
/// `PRAGMA optimize` write (#427). No statement can create a `sqlite_` name;
/// SQLite creates these itself, and updates them when a statement drops or
/// renames a table or index they describe. Like PostgreSQL's sequence
/// counters and statistics, they are left out of the catalog, and migrations
/// may read and write them.
fn engine_state(name: &str) -> bool {
    name.eq_ignore_ascii_case("sqlite_sequence") || statistics_table(name)
}
/// SQLite's JSON table-valued functions (`json_each`, `json_tree` and their
/// JSONB forms): eponymous virtual tables that read only their arguments.
fn json_table(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "json_each" | "json_tree" | "jsonb_each" | "jsonb_tree"
    )
}
/// The bundled SQLite's other eponymous virtual tables, which a name can
/// resolve to without being in the catalog: `dbstat` reads every page of
/// the file, the ledger's included; `pragma_*` runs a PRAGMA; `fts3tokenize`
/// and `fts4aux` belong to full-text search.
fn eponymous(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    matches!(name.as_str(), "dbstat" | "fts3tokenize" | "fts4aux") || name.starts_with("pragma_")
}
/// Whether a read of `name` that uses none of its columns (`count(*) FROM
/// t`, `EXISTS (SELECT 1 FROM t)`) may run. SQLite gives such a read the
/// database the statement spells, so none for an unqualified name. That
/// name is a table or view in `main` (no other schema holds one: ntnt
/// attaches no database and refuses temporary objects), a common table
/// expression of the statement, or an eponymous virtual table. The first
/// two may be read; ntnt's ledger, SQLite's internal tables and the
/// eponymous virtual tables other than the JSON ones stay refused (#427).
fn unqualified_read(name: &str, tables: &HashSet<String>) -> bool {
    tables.contains(&name.to_ascii_lowercase())
        || !(reserved(name) || internal(name) || eponymous(name))
}
/// The table that `ALTER TABLE ... ADD COLUMN` is changing, when `name` is
/// the copy of it SQLite compiles the new column against.
fn altered_copy(name: &str) -> Option<&str> {
    name.get(..16)
        .filter(|prefix| prefix.eq_ignore_ascii_case("sqlite_altertab_"))
        .map(|_| &name[16..])
        .filter(|table| !table.is_empty())
}

/// SQLite's built-in SQL functions in the bundled SQLite 3.54 build, as its
/// `pragma_function_list` reports them (`builtin` is 1): the core scalar,
/// aggregate and window functions, the date and time functions and the JSON
/// functions and operators (`->`, `->>`). This build has no math functions
/// (`SQLITE_ENABLE_MATH_FUNCTIONS` is off). The two built-in functions that
/// SQLite itself marks direct-only reach outside the database and are left
/// out: `load_extension` loads code and `sqlite_log` writes to the process's
/// log. The functions SQLite's FTS3,
/// FTS5 and R*Tree extensions register (`fts3_tokenizer`, `snippet`, `bm25`,
/// `rtreenode`, ...) are not built in, and neither is any application
/// function. Where a function's value lands in the catalog, it lands as text:
/// a DEFAULT such as `(lower(hex(randomblob(16))))` or `CURRENT_TIMESTAMP` is
/// stored as its expression, so the catalog stays deterministic, and SQLite
/// itself refuses a non-deterministic call where determinism is required (a
/// CHECK constraint, generated column, index expression or predicate).
/// Sorted, for `binary_search`; a unit test compares the list with the
/// bundled SQLite.
const BUILTIN_FUNCTIONS: &[&str] = &[
    "->",
    "->>",
    "abs",
    "avg",
    "changes",
    "char",
    "coalesce",
    "concat",
    "concat_ws",
    "count",
    "cume_dist",
    "current_date",
    "current_time",
    "current_timestamp",
    "date",
    "datetime",
    "dense_rank",
    "first_value",
    "format",
    "glob",
    "group_concat",
    "hex",
    "if",
    "ifnull",
    "iif",
    "instr",
    "json",
    "json_array",
    "json_array_insert",
    "json_array_length",
    "json_error_position",
    "json_extract",
    "json_group_array",
    "json_group_object",
    "json_insert",
    "json_object",
    "json_patch",
    "json_pretty",
    "json_quote",
    "json_remove",
    "json_replace",
    "json_set",
    "json_type",
    "json_valid",
    "jsonb",
    "jsonb_array",
    "jsonb_array_insert",
    "jsonb_extract",
    "jsonb_group_array",
    "jsonb_group_object",
    "jsonb_insert",
    "jsonb_object",
    "jsonb_patch",
    "jsonb_remove",
    "jsonb_replace",
    "jsonb_set",
    "julianday",
    "lag",
    "last_insert_rowid",
    "last_value",
    "lead",
    "length",
    "like",
    "likelihood",
    "likely",
    "lower",
    "ltrim",
    "max",
    "min",
    "nth_value",
    "ntile",
    "nullif",
    "octet_length",
    "percent_rank",
    "printf",
    "quote",
    "random",
    "randomblob",
    "rank",
    "replace",
    "round",
    "row_number",
    "rtrim",
    "sign",
    "soundex",
    "sqlite_compileoption_get",
    "sqlite_compileoption_used",
    "sqlite_source_id",
    "sqlite_version",
    "strftime",
    "string_agg",
    "substr",
    "substring",
    "subtype",
    "sum",
    "time",
    "timediff",
    "total",
    "total_changes",
    "trim",
    "typeof",
    "unhex",
    "unicode",
    "unistr",
    "unistr_quote",
    "unixepoch",
    "unlikely",
    "upper",
    "zeroblob",
];
/// The internal functions SQLite's own ALTER TABLE statements call.
const ALTER_FUNCTIONS: &[&str] = &[
    "sqlite_drop_column",
    "sqlite_rename_column",
    "sqlite_rename_quotefix",
    "sqlite_rename_table",
    "sqlite_rename_test",
];

#[derive(Clone, Copy)]
enum Policy {
    Migration,
    Check,
    Definition,
}
/// Engine resolution, not just source spelling, mediates every authored access.
fn authorize(
    ctx: AuthContext<'_>,
    policy: Policy,
    tables: &mut std::collections::HashSet<String>,
) -> Authorization {
    use AuthAction::*;
    let write = !matches!(policy, Policy::Check);
    let schema_access = matches!(policy, Policy::Migration);
    let main = ctx.database_name == Some("main");
    let ordinary = |name: &str| !reserved(name) && !internal(name);
    let schema = |name: &str| {
        schema_access && schema_table(name) && matches!(ctx.database_name, Some("main" | "temp"))
    };
    // An unqualified name of a state table or a JSON table-valued function
    // resolves to `main`, as no other schema holds one.
    let local = matches!(ctx.database_name, None | Some("main"));
    let state = |name: &str| write && local && engine_state(name);
    // ANALYZE asks for each table it would analyze. IGNORE leaves ntnt's
    // ledger out without failing the statement, so a migration's ANALYZE
    // never reads it (sq-19, review).
    if let Analyze { table_name } = &ctx.action {
        if schema_access && main && reserved(table_name) {
            return Authorization::Ignore;
        }
    }
    let allowed = match ctx.action {
        Select => true,
        Read {
            table_name,
            column_name,
        } => {
            (main && ordinary(table_name) && tables.contains(&table_name.to_ascii_lowercase()))
                || schema(table_name)
                || state(table_name)
                || (write && local && json_table(table_name))
                // ALTER TABLE ... ADD COLUMN re-checks the table's constraints
                // with pragma_quick_check (sq-08).
                || (schema_access && main && table_name.eq_ignore_ascii_case("pragma_quick_check"))
                || (ctx.database_name.is_none()
                    && column_name.is_empty()
                    && unqualified_read(table_name, tables))
                // The key column of the autoindex below.
                || (schema_access
                    && main
                    && altered_copy(table_name)
                        .is_some_and(|table| tables.contains(&table.to_ascii_lowercase())))
        }
        // The quick_check that pragma_quick_check runs, on a managed table.
        Pragma {
            pragma_name,
            pragma_value,
        } => {
            schema_access
                && main
                && pragma_name.eq_ignore_ascii_case("quick_check")
                && pragma_value.is_some_and(|table| tables.contains(&table.to_ascii_lowercase()))
        }
        Function { function_name } => {
            let name = function_name.to_ascii_lowercase();
            name == "count"
                || (write
                    && (BUILTIN_FUNCTIONS.binary_search(&name.as_str()).is_ok()
                        || ALTER_FUNCTIONS.contains(&name.as_str())))
        }
        CreateTable { table_name } => {
            if schema_access && main && ordinary(table_name) {
                tables.insert(table_name.to_ascii_lowercase());
                true
            } else {
                // SQLite creates its state tables itself: sqlite_sequence for
                // the first AUTOINCREMENT table, the statistics for ANALYZE.
                schema_access && main && engine_state(table_name)
            }
        }
        CreateView { view_name } => {
            if schema_access && main && ordinary(view_name) {
                tables.insert(view_name.to_ascii_lowercase());
                true
            } else {
                false
            }
        }
        DropTable { table_name } => {
            schema_access
                && main
                && ((ordinary(table_name) && tables.contains(&table_name.to_ascii_lowercase()))
                    || statistics_table(table_name))
        }
        DropView { view_name } => {
            schema_access
                && main
                && ordinary(view_name)
                && tables.contains(&view_name.to_ascii_lowercase())
        }
        // ALTER TABLE t ADD COLUMN compiles the new column against a copy of
        // `t` named `sqlite_altertab_t`, never stored. A UNIQUE or PRIMARY
        // KEY column asks for an autoindex on that copy, which reads the
        // column, and SQLite then refuses the column itself ("Cannot add a
        // UNIQUE column"), so its own error, not this autoindex, reaches the
        // diagnostic (#427 F6).
        CreateIndex {
            index_name,
            table_name,
        } if altered_copy(table_name).is_some() => {
            schema_access
                && main
                && altered_copy(table_name)
                    .is_some_and(|table| tables.contains(&table.to_ascii_lowercase()))
                && index_name
                    .to_ascii_lowercase()
                    .starts_with("sqlite_autoindex_sqlite_altertab_")
        }
        CreateIndex {
            index_name,
            table_name,
        }
        | DropIndex {
            index_name,
            table_name,
        } => {
            schema_access
                && main
                && ordinary(table_name)
                && tables.contains(&table_name.to_ascii_lowercase())
                && !reserved(index_name)
        }
        Reindex { index_name } => schema_access && main && !reserved(index_name),
        CreateTrigger {
            trigger_name,
            table_name,
        }
        | DropTrigger {
            trigger_name,
            table_name,
        } => {
            schema_access
                && main
                && ordinary(table_name)
                && tables.contains(&table_name.to_ascii_lowercase())
                && ordinary(trigger_name)
        }
        AlterTable {
            database_name,
            table_name,
        } => {
            schema_access
                && database_name == "main"
                && ordinary(table_name)
                && tables.contains(&table_name.to_ascii_lowercase())
        }
        Insert { table_name } | Delete { table_name } | Update { table_name, .. } => {
            write
                && ((main
                    && ordinary(table_name)
                    && tables.contains(&table_name.to_ascii_lowercase()))
                    || schema(table_name)
                    || state(table_name))
        }
        // ANALYZE reads the tables of `main` it names (all of them when it
        // names none) and writes only the statistics tables (sq-19).
        Analyze { .. } => schema_access && main,
        Recursive => write,
        _ => false,
    };
    if allowed {
        Authorization::Allow
    } else {
        Authorization::Deny
    }
}
/// What an authored statement was refused, for its diagnostic. Names only
/// identifiers from the migration itself: never a literal, a PRAGMA value or an
/// ATTACH filename.
fn denial(action: &AuthAction<'_>) -> String {
    use AuthAction::*;
    match action {
        Function { function_name } => format!("SQLite function `{function_name}`"),
        Read { table_name, .. }
        | Insert { table_name }
        | Update { table_name, .. }
        | Delete { table_name }
        | CreateTable { table_name }
        | DropTable { table_name }
        | AlterTable { table_name, .. } => format!("table `{table_name}`"),
        CreateView { view_name } | DropView { view_name } => format!("view `{view_name}`"),
        CreateIndex { index_name, .. } | DropIndex { index_name, .. } | Reindex { index_name } => {
            format!("index `{index_name}`")
        }
        CreateTrigger { trigger_name, .. } | DropTrigger { trigger_name, .. } => {
            format!("trigger `{trigger_name}`")
        }
        Pragma { pragma_name, .. } => format!("PRAGMA {pragma_name}"),
        Transaction { .. } | Savepoint { .. } => "transaction control".into(),
        Attach { .. } | Detach { .. } => "ATTACH/DETACH".into(),
        CreateVtable { .. } | DropVtable { .. } => "a virtual table".into(),
        CreateTempIndex { .. }
        | CreateTempTable { .. }
        | CreateTempTrigger { .. }
        | CreateTempView { .. }
        | DropTempIndex { .. }
        | DropTempTable { .. }
        | DropTempTrigger { .. }
        | DropTempView { .. } => "a temporary object".into(),
        Analyze { .. } => "ANALYZE".into(),
        Recursive => "a recursive common table expression (WITH RECURSIVE)".into(),
        Unknown { code, .. } => format!("SQLite authorizer action {code}"),
        _ => "an operation".into(),
    }
}
/// What to do about statement `number`, which used `refused`.
fn denial_advice(number: usize, refused: &str) -> String {
    format!("Rewrite statement {number} of the migration named in source without {refused}, then plan again. A SQLite migration can use the tables and views of `main`, SQLite's built-in functions except load_extension and sqlite_log, the JSON table-valued functions, SQLite's AUTOINCREMENT counters and planner statistics, and ANALYZE. It cannot use ntnt's ledger, other virtual tables, temporary objects, functions an extension or application registers, ATTACH or PRAGMAs (docs/DATABASE_MIGRATIONS.md, Supported SQL and catalog coverage).")
}
type AuthScope = Option<(Policy, std::collections::HashSet<String>)>;

/// One native authorizer is installed per connection (rusqlite requires
/// `&mut Connection` to change it, which cannot coexist with a live `Batch`).
/// It reads a shared scope: `None` admits adapter-owned SQL only while no
/// authored program is being compiled; `Some` applies the authored policy.
/// A poisoned scope fails closed for every statement. The first refusal under
/// an authored scope is kept for that statement's diagnostic.
#[derive(Clone, Default)]
struct Authorizer {
    scope: Arc<Mutex<AuthScope>>,
    denied: Arc<Mutex<Option<String>>>,
    /// Tables the authored statement compiled since the last activation drops.
    dropped: Arc<Mutex<Vec<String>>>,
}
impl Authorizer {
    fn install(conn: &mut Connection) -> Result<Self> {
        let authorizer = Self::default();
        let (scope, denied, dropped) = (
            authorizer.scope.clone(),
            authorizer.denied.clone(),
            authorizer.dropped.clone(),
        );
        conn.authorizer(Some(move |context: AuthContext<'_>| match scope.lock() {
            Ok(mut scope) => match scope.as_mut() {
                None => Authorization::Allow,
                Some((policy, tables)) => {
                    let decision = authorize(context, *policy, tables);
                    if matches!(decision, Authorization::Deny) {
                        if let Ok(mut denied) = denied.lock() {
                            denied.get_or_insert_with(|| denial(&context.action));
                        }
                    }
                    if let AuthAction::DropTable { table_name } = &context.action {
                        if let Ok(mut dropped) = dropped.lock() {
                            dropped.push(table_name.to_string());
                        }
                    }
                    decision
                }
            },
            Err(_) => Authorization::Deny,
        }))
        .map_err(sql_error)?;
        Ok(authorizer)
    }
    fn set(&self, scope: AuthScope) -> Result<()> {
        *self.scope.lock().map_err(|_| {
            Diagnostic::new("DB_RUNTIME", "SQLite authorizer state is unavailable.")
        })? = scope;
        Ok(())
    }
    /// Resolve the allowlist with the policy inactive (it is adapter SQL), then
    /// enforce `policy` until the returned guard drops.
    fn activate(&self, conn: &Connection, policy: Policy) -> Result<AuthGuard<'_>> {
        self.set(None)?;
        let tables = known_tables(conn)?;
        if let Ok(mut denied) = self.denied.lock() {
            *denied = None;
        }
        if let Ok(mut dropped) = self.dropped.lock() {
            dropped.clear();
        }
        self.set(Some((policy, tables)))?;
        Ok(AuthGuard(self))
    }
    /// What the authored statement compiled since the last activation was refused.
    fn take_denial(&self) -> Option<String> {
        self.denied.lock().ok().and_then(|mut denied| denied.take())
    }
    /// The tables the authored statement compiled since the last activation drops.
    fn take_dropped(&self) -> Vec<String> {
        self.dropped
            .lock()
            .map(|mut dropped| std::mem::take(&mut *dropped))
            .unwrap_or_default()
    }
    /// Lift the authored policy so adapter SQL can run between compiling an
    /// authored statement and running it; `resume` restores it first.
    fn suspend(&self) -> Result<AuthScope> {
        let mut scope = self.scope.lock().map_err(|_| {
            Diagnostic::new("DB_RUNTIME", "SQLite authorizer state is unavailable.")
        })?;
        Ok(scope.take())
    }
    fn resume(&self, scope: AuthScope) -> Result<()> {
        self.set(scope)
    }
}
struct AuthGuard<'a>(&'a Authorizer);
impl Drop for AuthGuard<'_> {
    fn drop(&mut self) {
        // A poisoned scope already denies everything; nothing else to restore.
        let _ = self.0.set(None);
    }
}

fn authored<T>(
    state: &State,
    policy: Policy,
    operation: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let _active = state.auth.activate(&state.conn, policy)?;
    operation(&state.conn)
}

/// Eponymous virtual tables can report database=main without appearing in the
/// catalog. Admit only concrete schema relations, not names that merely look local.
fn known_tables(conn: &Connection) -> Result<std::collections::HashSet<String>> {
    Ok(rows(
        conn,
        "SELECT name FROM main.sqlite_schema WHERE type IN ('table','view')",
        &[],
    )?
    .into_iter()
    .filter_map(|row| {
        row[0]
            .as_str()
            .filter(|name| !reserved(name) && !internal(name))
            .map(str::to_ascii_lowercase)
    })
    .collect())
}

/// SQLite finds native statement boundaries (including trigger bodies and quoted
/// semicolons). Refresh the resolved-relation allowlist after each statement, so
/// a dropped name cannot fall through to an eponymous virtual table later.
fn execute_script(state: &State, sql: &str, limit: Duration, replay: bool) -> Result<Vec<String>> {
    let conn = &state.conn;
    // Every table the script drops, for a COMMIT that its deferred foreign
    // keys fail (`Sqlite::commit`).
    let mut all_dropped = Vec::new();
    // A RAISE(ROLLBACK) undoes the transaction, and with it any trigger it
    // created, so the triggers a RAISE may come from are read before any
    // statement runs (review r1).
    let triggers = trigger_sql(conn);
    let mut batch = rusqlite::Batch::new(conn, sql);
    let mut statement = 0usize;
    loop {
        let active = state.auth.activate(conn, Policy::Migration)?;
        statement += 1;
        let number = statement;
        let mut failure = Failure::default();
        let mut dropped = Vec::new();
        let result = (|| {
            let Some(mut statement) = batch.next().map_err(|e| failure.record(e))? else {
                return Ok(false);
            };
            if statement.parameter_count() != 0 {
                return Err(Diagnostic::new(
                    "DB_SQL_UNSUPPORTED",
                    "Migration SQL cannot contain uncaptured bind parameters.",
                ));
            }
            dropped = state.auth.take_dropped();
            if !dropped.is_empty() {
                refuse_on_delete_drops(state, number, &dropped)?;
            }
            let mut output = statement.raw_query();
            while output.next().map_err(|e| failure.record(e))?.is_some() {}
            Ok(true)
        })();
        drop(active);
        match result {
            Ok(true) => all_dropped.append(&mut dropped),
            Ok(false) => return Ok(all_dropped),
            // SQLite reports a refused function as a generic error, so the
            // authorizer's record of what it refused decides the diagnostic.
            Err(mut error) => {
                return Err(match state.auth.take_denial() {
                    Some(refused) => {
                        let mut d = Diagnostic::new(
                            "DB_SQL_UNSUPPORTED",
                            &format!(
                            "Statement {statement} uses {refused}, which SQLite migrations cannot use."
                        ),
                        );
                        d.next_action = denial_advice(statement, &refused);
                        d
                    }
                    // This refusal already names its statement.
                    None if error.code == "DB_REBUILD_REQUIRED" => error,
                    // The session's file, not this statement, failed (#424).
                    None if error.message.starts_with(FILE_SUBJECT) => error,
                    // SQLite interrupted the statement at its deadline.
                    None if error.code == "DB_TIMEOUT" => statement_limit(statement, limit, replay),
                    // SQLite resolves a function an application registers on
                    // its own connections as missing on ntnt's (#427 F4).
                    None if failure.missing_function().is_some() => {
                        let function = failure.missing_function().unwrap_or_default();
                        let refused = format!("SQLite function `{function}`");
                        let mut d = Diagnostic::new(
                            "DB_SQL_UNSUPPORTED",
                            &format!(
                                "Statement {statement} uses {refused}, which SQLite migrations cannot use."
                            ),
                        );
                        d.next_action = denial_advice(statement, &refused);
                        d
                    }
                    None => {
                        if let Some(text) = failure.raised.take() {
                            let created = (sql, statement);
                            error.message = if raised_literal(conn, &triggers, created, &text) {
                                format!("a trigger raised: {text} (SQLITE_CONSTRAINT_TRIGGER).")
                            } else {
                                "a trigger raised an error whose message is computed, so it is suppressed (SQLITE_CONSTRAINT_TRIGGER).".to_owned()
                            };
                        }
                        error.message = format!("Statement {statement}: {}", error.message);
                        if failure.authoring() {
                            error.next_action = failure
                                .message
                                .as_deref()
                                // SQLite refuses an ADD COLUMN with a plain
                                // error; a trigger's RAISE with the same text
                                // is SQLITE_CONSTRAINT_TRIGGER (review).
                                .filter(|_| failure.code == Some(rusqlite::ffi::SQLITE_ERROR))
                                .and_then(|message| add_column_advice(statement, message, replay))
                                .or_else(|| {
                                    (failure.code
                                        == Some(rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY))
                                    .then(|| {
                                        dropped_parent_advice(
                                            conn,
                                            Some(statement),
                                            &dropped,
                                            replay,
                                        )
                                    })
                                    .flatten()
                                })
                                .unwrap_or_else(|| statement_advice(statement, replay));
                        }
                        error
                    }
                });
            }
        }
    }
}

/// The SQLite result code an authored statement failed with and, for a
/// RAISE, its raw message, kept out of the diagnostic until it is shown to
/// be a literal the migration wrote (#424).
#[derive(Default)]
struct Failure {
    code: Option<i32>,
    raised: Option<String>,
    /// SQLite's own message, only ever matched against the shapes
    /// `sqlite_errors` shows, never shown itself.
    message: Option<String>,
}
impl Failure {
    fn record(&mut self, error: rusqlite::Error) -> Diagnostic {
        match &error {
            rusqlite::Error::SqliteFailure(e, text) => {
                self.code = Some(e.extended_code);
                self.message = text.clone();
                if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_TRIGGER {
                    self.raised = text.clone();
                }
            }
            rusqlite::Error::SqlInputError { error: e, msg, .. } => {
                self.code = Some(e.extended_code);
                self.message = Some(msg.clone());
            }
            _ => {}
        }
        sql_error(error)
    }
    /// The function SQLite found no definition of, by name.
    fn missing_function(&self) -> Option<&str> {
        super::sqlite_errors::missing_function(self.code?, self.message.as_deref()?)
    }
    /// An error in the statement's own SQL, or in the rows it writes, that a
    /// change to the migration (or to those rows) fixes.
    fn authoring(&self) -> bool {
        use rusqlite::ffi;
        self.code.is_some_and(|code| {
            matches!(
                code & 0xff,
                ffi::SQLITE_ERROR
                    | ffi::SQLITE_CONSTRAINT
                    | ffi::SQLITE_MISMATCH
                    | ffi::SQLITE_TOOBIG
                    | ffi::SQLITE_RANGE
            )
        })
    }
}

/// Whether `text` is the message of a `RAISE(..., '<literal>')` in a trigger
/// a migration wrote: one the catalog held before the migration ran
/// (`before`) or holds now, or one that a CREATE TRIGGER before statement
/// `number` of the migration `sql` created (`created`). Then no row value is
/// in it. The catalog alone cannot say once a RAISE(ROLLBACK) has undone
/// the triggers its transaction created (review r1).
fn raised_literal(
    conn: &Connection,
    before: &[String],
    (sql, number): (&str, usize),
    text: &str,
) -> bool {
    let literal = |trigger: &String| {
        super::analysis::sqlite_raise_messages(trigger)
            .iter()
            .any(|m| m == text)
    };
    before.iter().chain(&trigger_sql(conn)).any(literal)
        || super::analysis::sqlite_created_raise_messages(sql, number.saturating_sub(1))
            .iter()
            .any(|m| m == text)
}
/// The SQL of every trigger the catalog holds; none if it cannot be read.
fn trigger_sql(conn: &Connection) -> Vec<String> {
    rows(
        conn,
        "SELECT sql FROM main.sqlite_schema WHERE type='trigger'",
        &[],
    )
    .map(|triggers| {
        triggers
            .iter()
            .filter_map(|row| row[0].as_str().map(str::to_owned))
            .collect()
    })
    .unwrap_or_default()
}

/// What to do about statement `number` of a migration that failed in its
/// own SQL (#424): in replay, fix it; on a target, nothing from the
/// migration was committed.
fn statement_advice(number: usize, replay: bool) -> String {
    if replay {
        format!("Fix statement {number} of the migration named in source so that it succeeds when the history replays in order on an empty database, then rerun plan. Replay changed nothing on the target.")
    } else {
        format!("Nothing from this migration was committed: its transaction rolled back when statement {number} failed on the target, and progress shows what this run committed before it. If the target's data conflicts with the statement, correct that data and rerun migrate with the same plan; if the migration is wrong, change it, then plan, review and approve again.")
    }
}

/// How statement `number`, failed on the target or in replay, ends.
fn rolled_back(number: usize, replay: bool) -> String {
    if replay {
        "Then rerun plan. Replay changed nothing on the target.".to_owned()
    } else {
        format!("Nothing from this migration was committed: its transaction rolled back when statement {number} failed on the target, and progress shows what this run committed before it. Change the migration, then plan, review and approve again.")
    }
}

/// What to do about statement `number`, an `ALTER TABLE ... ADD COLUMN`
/// SQLite refused with `message`, by SQLite's rules for adding a column:
/// it cannot add a UNIQUE or PRIMARY KEY column, and on a table that has
/// rows it gives every row the default, so that default must be a
/// constant, and not NULL for a NOT NULL column (#427 F6, F7).
fn add_column_advice(number: usize, message: &str, replay: bool) -> Option<String> {
    const REBUILD: &str = "rebuild the table with a `sqlite_rebuild` sidecar: create it again with the new column, copy its rows, drop the old table and rename the new one (docs/DATABASE_MIGRATIONS.md, Explicit one-table rebuild)";
    let rows = if replay {
        "The table has rows that the history inserted before this statement."
    } else {
        "Replay starts from empty tables, so the plan could not show this; the target's table has rows."
    };
    let advice = match message {
        "Cannot add a UNIQUE column" => format!("SQLite's ALTER TABLE ... ADD COLUMN cannot add a UNIQUE column. In statement {number}, add the column without UNIQUE, then create a unique index on it in the same migration: `ALTER TABLE <table> ADD COLUMN <column> <type>;` followed by `CREATE UNIQUE INDEX <table>_<column>_key ON <table> (<column>);`. The index enforces what UNIQUE would, and like it allows any number of NULLs."),
        "Cannot add a PRIMARY KEY column" => format!("SQLite's ALTER TABLE ... ADD COLUMN cannot add a PRIMARY KEY column. Instead of statement {number}, {REBUILD}."),
        "Cannot add a column with non-constant default" => format!("SQLite's ALTER TABLE ... ADD COLUMN gives every existing row the new column's default, so on a table that has rows it accepts only a constant default (a literal or NULL), not CURRENT_TIMESTAMP, datetime('now'), randomblob() or another expression. {rows} In statement {number}, give the column a constant default and set the existing rows with an UPDATE in the same migration (`ALTER TABLE <table> ADD COLUMN <column> TEXT NOT NULL DEFAULT '';` then `UPDATE <table> SET <column> = CURRENT_TIMESTAMP;`); new rows then get the constant. To keep the expression as the default for new rows, {REBUILD}."),
        "Cannot add a NOT NULL column with default value NULL" => format!("SQLite's ALTER TABLE ... ADD COLUMN cannot add a NOT NULL column without a default to a table that has rows: each row would hold NULL. {rows} In statement {number}, give the column a constant default (`ALTER TABLE <table> ADD COLUMN <column> <type> NOT NULL DEFAULT <constant>`) and set the rows that need another value with an UPDATE in the same migration, or {REBUILD}, filling the column as the rows are copied."),
        "Cannot add a REFERENCES column with non-NULL default value" => format!("With foreign keys enforced, SQLite's ALTER TABLE ... ADD COLUMN cannot add a REFERENCES column with a non-NULL default to a table that has rows. {rows} In statement {number}, add the column without a default and set it with an UPDATE in the same migration, or {REBUILD}."),
        _ => return None,
    };
    Some(format!("{advice} {}", rolled_back(number, replay)))
}

/// What to do when a migration that dropped `dropped` failed a foreign key:
/// statement `number` (an immediate key), or its COMMIT (`None`, a deferred
/// key). SQLite deletes every row of a dropped table first, so on a target
/// with rows a drop breaks each reference to them, and a deferred key
/// counts those until COMMIT even when a replacement renamed to the old
/// name holds the same rows (#427 F1). `None` if no other table references
/// a dropped one. In replay, the rows that broke were inserted by the
/// history itself, so the migration must change (review).
fn dropped_parent_advice(
    conn: &Connection,
    number: Option<usize>,
    dropped: &[String],
    replay: bool,
) -> Option<String> {
    let parents: Vec<(&String, Vec<String>)> = dropped
        .iter()
        .filter_map(|table| {
            let children = referencing_tables(conn, table).ok()?;
            (!children.is_empty()).then_some((table, children))
        })
        .collect();
    let (table, _) = parents.first()?;
    let named = list(
        &parents
            .iter()
            .map(|(table, children)| {
                let children: Vec<String> = children.iter().map(|c| format!("`{c}`")).collect();
                format!(
                    "table `{table}`, which {} reference{}",
                    list(&children),
                    if children.len() == 1 { "s" } else { "" }
                )
            })
            .collect::<Vec<_>>(),
    );
    let sidecar = format!("If the migration rebuilds `{table}` (creates its replacement, copies the rows, drops it and renames the replacement), add a `sqlite_rebuild = \"{table}\"` sidecar (docs/DATABASE_MIGRATIONS.md, Explicit one-table rebuild): it runs the migration with foreign-key enforcement off, then checks every foreign key, the row counts and the other definitions. To retire `{table}` instead, first remove the foreign keys that reference it, or delete the rows that do, earlier in the migration or in an earlier one.");
    const FOUND: &str = "Replay found this failure: the history inserted those rows before the migration ran, or earlier in it.";
    Some(match (number, replay) {
        (Some(number), true) => format!("Statement {number} drops {named}. SQLite deletes every row of a dropped table first, so the drop fails while rows reference it. {FOUND} {sidecar} {}", rolled_back(number, true)),
        (None, true) => format!("The migration dropped {named} with a deferred foreign key. SQLite deletes every row of a dropped table first and counts each reference that breaks until COMMIT, even when a replacement renamed to the old name holds the same rows, so COMMIT fails while rows reference it. {FOUND} {sidecar} Then rerun plan. Replay changed nothing on the target."),
        (Some(number), false) => format!("Statement {number} drops {named}. SQLite deletes every row of a dropped table first, so the drop fails while rows reference it; replay starts from empty tables, so the plan could not show this. {sidecar} Nothing from this migration was committed: its transaction rolled back when statement {number} failed on the target, and progress shows what this run committed before it. Change the migration, then plan, review and approve again."),
        (None, false) => format!("The migration dropped {named} with a deferred foreign key. SQLite deletes every row of a dropped table first and counts each reference that breaks until COMMIT, even when a replacement renamed to the old name holds the same rows, so COMMIT fails on a target with rows; replay starts from empty tables, so the plan could not show this. {sidecar} The COMMIT failed, so the migration was not committed; progress shows what reconciliation found. Change the migration, then plan, review and approve again."),
    })
}

/// `a`, `a and b` or `a, b and c`.
fn list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// DB_REPLAY_UNSUPPORTED for a migration the SQLite parser gate refuses
/// (#424): the statement and the cause, with advice for that cause.
fn replay_unsupported(sql: &str) -> Diagnostic {
    use super::analysis::SqliteRefusal::*;
    let Some(refusal) = super::analysis::sqlite_refusal(sql) else {
        return Diagnostic::new(
            "DB_REPLAY_UNSUPPORTED",
            "Migration is outside the supported SQLite subset.",
        );
    };
    let (message, next_action) = match refusal {
        Syntax { statement, near } => (
            match near {
                Some(near) => format!("Statement {statement} has a syntax error near {near}."),
                None => format!("Statement {statement} has a syntax error."),
            },
            format!("Correct statement {statement} in the migration named in source, then rerun plan. Nothing in this migration ran."),
        ),
        TokenLimit { statement, limit } => (
            format!("Statement {statement} passes the {limit}-token limit for one SQLite migration file (whitespace and comments count), so the file is not read further."),
            format!("Split the migration into smaller migration files that keep its statements in order, each within {limit} tokens, then rerun plan. Nothing in this migration ran."),
        ),
        Pragma { statement, name } => (
            format!("Statement {statement} is PRAGMA {name}; migrations cannot run PRAGMAs."),
            "Remove the PRAGMA from the migration, then rerun plan. ntnt db sets foreign_keys and recursive_triggers on its own connections, and file settings such as journal_mode belong to the application's connection, not to a migration. Nothing in this migration ran.".to_owned(),
        ),
        Unsupported { statement, construct } => (
            if construct.is_empty() {
                format!("Statement {statement} is outside the supported SQLite subset.")
            } else {
                format!("Statement {statement} (`{construct}`) is outside the supported SQLite subset.")
            },
            format!("Rewrite statement {statement} with the statements SQLite migrations support: CREATE, ALTER and DROP TABLE; CREATE and DROP INDEX, VIEW and TRIGGER; INSERT, UPDATE and DELETE (write CREATE TABLE ... AS SELECT as CREATE TABLE, then INSERT ... SELECT); and ANALYZE, as docs/DATABASE_MIGRATIONS.md describes under Supported SQL and catalog coverage. Then rerun plan. Nothing in this migration ran."),
        ),
    };
    let mut d = Diagnostic::new("DB_REPLAY_UNSUPPORTED", &message);
    d.next_action = next_action;
    d
}

/// A statement SQLite interrupted at the migration's deadline (#429).
fn statement_limit(statement: usize, limit: Duration, replay: bool) -> Diagnostic {
    let seconds = limit.as_secs();
    let message = if replay {
        format!("Statement {statement} did not finish within the {seconds}-second replay limit; replay runs each migration on the empty scratch database within fixed bounds.")
    } else {
        format!("Statement {statement} did not finish within the {seconds}-second statement limit (statement_seconds); nothing from this migration was committed.")
    };
    Diagnostic::new("DB_STATEMENT_LIMIT", &message)
}

/// SQLite's DROP TABLE deletes every row first, which runs the ON DELETE
/// actions of the tables that reference the dropped one (#419). Outside
/// sqlite_rebuild mode those actions are on, so a drop that would run them is
/// refused before the statement runs, naming each child table and its action.
fn refuse_on_delete_drops(state: &State, number: usize, dropped: &[String]) -> Result<()> {
    // sqlite_rebuild mode turns foreign-key actions off for the migration.
    if !state
        .conn
        .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY)
        .map_err(sql_error)?
    {
        return Ok(());
    }
    let scope = state.auth.suspend()?;
    let found = dropped
        .iter()
        .map(|table| on_delete_children(&state.conn, table).map(|children| (table, children)))
        .find(|found| !matches!(found, Ok((_, children)) if children.is_empty()));
    state.auth.resume(scope)?;
    let Some((table, children)) = found.transpose()? else {
        return Ok(());
    };
    let named: Vec<String> = children
        .iter()
        .map(|(child, action)| format!("`{child}` (ON DELETE {action})"))
        .collect();
    let named = match named.as_slice() {
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        [] => unreachable!("only a non-empty set of children is refused"),
    };
    let (s, them) = if children.len() == 1 {
        ("s", "that table")
    } else {
        ("", "those tables")
    };
    // A rebuild keeps every table (`RebuildGuard`), so the sidecar cannot
    // retire one.
    Err(Diagnostic::new(
        "DB_REBUILD_REQUIRED",
        &format!(
            "Statement {number} drops table `{table}`, which {named} reference{s}. SQLite deletes every row of a dropped table first, which would run those actions. To rebuild `{table}` (create the new table and copy its rows), use a `sqlite_rebuild` sidecar; to retire it, first remove those foreign keys from {them} or drop {them}."
        ),
    ))
}

/// The tables whose foreign keys reference `table` with an ON DELETE action
/// that changes their rows, with that action, in name order.
fn on_delete_children(conn: &Connection, table: &str) -> Result<Vec<(String, String)>> {
    let mut statement = conn
        .prepare(
            "SELECT DISTINCT m.name, f.on_delete FROM main.sqlite_schema AS m, \
             pragma_foreign_key_list(m.name) AS f \
             WHERE m.type = 'table' AND m.name <> ?1 COLLATE NOCASE \
             AND f.\"table\" = ?1 COLLATE NOCASE \
             AND f.on_delete IN ('CASCADE', 'SET NULL', 'SET DEFAULT') \
             ORDER BY m.name, f.on_delete",
        )
        .map_err(sql_error)?;
    let children = statement
        .query_map([table], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<(String, String)>>>()
        .map_err(sql_error)?;
    Ok(children)
}

/// The other tables whose foreign keys reference `table`, in name order.
fn referencing_tables(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut statement = conn
        .prepare(
            "SELECT DISTINCT m.name FROM main.sqlite_schema AS m, \
             pragma_foreign_key_list(m.name) AS f \
             WHERE m.type = 'table' AND m.name <> ?1 COLLATE NOCASE \
             AND f.\"table\" = ?1 COLLATE NOCASE ORDER BY m.name",
        )
        .map_err(sql_error)?;
    let children = statement
        .query_map([table], |row| row.get(0))
        .map_err(sql_error)?
        .collect::<rusqlite::Result<Vec<String>>>()
        .map_err(sql_error)?;
    Ok(children)
}

fn rows(conn: &Connection, sql: &str, args: &[&dyn rusqlite::ToSql]) -> Result<Vec<Vec<Value>>> {
    let mut stmt = conn.prepare(sql).map_err(sql_error)?;
    let columns = stmt.column_count();
    let mut cursor = stmt.query(args).map_err(sql_error)?;
    let mut result = Vec::new();
    while let Some(row) = cursor.next().map_err(sql_error)? {
        if result.len() >= 10000 {
            return Err(Diagnostic::new(
                "DB_CATALOG_LIMIT",
                "SQLite inspection exceeded its row bound.",
            ));
        }
        let mut values = Vec::with_capacity(columns);
        for i in 0..columns {
            values.push(match row.get_ref(i).map_err(sql_error)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(v) => json!(v),
                ValueRef::Real(v) => json!(v),
                ValueRef::Text(v) => json!(std::str::from_utf8(v).map_err(|_| Diagnostic::new(
                    "DB_SQLITE",
                    "Catalog text is not valid UTF-8."
                ))?),
                ValueRef::Blob(_) => {
                    return Err(Diagnostic::new(
                        "DB_SQLITE",
                        "Unexpected binary catalog field.",
                    ))
                }
            });
        }
        result.push(values);
    }
    Ok(result)
}
fn integer(conn: &Connection, sql: &str) -> Result<i64> {
    conn.query_row(sql, [], |r| r.get(0)).map_err(sql_error)
}
fn quote(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// One table's columns and foreign keys, as the catalog reports them.
fn table_metadata(
    conn: &Connection,
    stats: &mut Stats,
    name: &str,
) -> Result<super::sqlite_cache::Table> {
    stats.metadata_queries += 1;
    let columns = rows(
        conn,
        "SELECT cid,name,type,\"notnull\",dflt_value,pk,hidden FROM pragma_table_xinfo(?1,'main')",
        &[&name],
    )?;
    let column_names = columns
        .iter()
        .map(|column| column[1].as_str().unwrap().to_owned())
        .collect();
    let primary_key = columns
        .iter()
        .filter(|column| column[5].as_i64().is_some_and(|pk| pk > 0))
        .map(|column| column[1].as_str().unwrap().to_ascii_lowercase())
        .collect();
    let mut entries = Vec::with_capacity(columns.len());
    let mut visible = true;
    for mut column in columns {
        let column_name = column[1].as_str().unwrap().to_owned();
        if column_name.len() > 255 || column[2].as_str().is_none_or(|t| t.len() > 255) {
            return Err(Diagnostic::new(
                "DB_CATALOG_LIMIT",
                "SQLite column metadata exceeds inspection bounds.",
            ));
        }
        visible &= column[6] == 0;
        if !column[4].is_null() {
            column[4] = json!({"sha256":json_digest(&column[4]),"redacted":true});
        }
        entries.push((
            format!("column:{}.{}", quote(name), quote(&column_name)),
            json!(column),
        ));
    }
    stats.metadata_queries += 1;
    let foreign_keys = json!(rows(
        conn,
        r#"SELECT id,seq,"table","from","to",on_update,on_delete,"match" FROM pragma_foreign_key_list(?1,'main') ORDER BY id,seq"#,
        &[&name]
    )?);
    let foreign_keys_supported = foreign_keys
        .as_array()
        .unwrap()
        .iter()
        .all(|fk| fk[2].as_str().is_some_and(|n| !reserved(n) && !internal(n)));
    Ok(super::sqlite_cache::Table {
        columns: entries,
        column_names,
        primary_key,
        visible,
        foreign_keys,
        foreign_keys_supported,
    })
}

/// One index's key columns and properties, as the catalog reports them.
fn index_metadata(
    conn: &Connection,
    stats: &mut Stats,
    table: &str,
    name: &str,
) -> Result<super::sqlite_cache::Index> {
    // A key column is a table column (cid >= 0), the rowid (-1) or an
    // expression (-2). Expressions and a partial index's WHERE predicate are
    // in the definition hash. SQLite resolves them when it loads the schema,
    // without the authorizer, so the function allowlist applies when a
    // migration's CREATE INDEX compiles; a target index that differs from the
    // replayed one is drift. The table's DML programs still compile them.
    stats.metadata_queries += 1;
    let columns = rows(
        conn,
        "SELECT seqno,cid,name,\"desc\",coll,key FROM pragma_index_xinfo(?1,'main') ORDER BY seqno",
        &[&name],
    )?;
    let columns_supported = columns.iter().all(|r| {
        r[1].as_i64().is_some_and(|v| v >= -2)
            && r[4]
                .as_str()
                .is_none_or(|v| matches!(v, "BINARY" | "NOCASE" | "RTRIM"))
    });
    stats.metadata_queries += 1;
    let properties = json!(rows(
        conn,
        "SELECT \"unique\",origin,partial FROM pragma_index_list(?1,'main') WHERE name=?2",
        &[&table, &name]
    )?);
    let properties_supported = properties.as_array().is_some_and(|p| p.len() == 1);
    Ok(super::sqlite_cache::Index {
        columns: json!(columns),
        columns_supported,
        properties,
        properties_supported,
    })
}

/// Name this session's file, by its role and secret and never its path,
/// in a diagnostic about the file itself: SQLite cannot write it (#424),
/// or another connection holds it locked (#425). `execute` is a session
/// that migrates the target.
fn attribute(owner: &super::sqlite_file::Owner, mut d: Diagnostic, execute: bool) -> Diagnostic {
    let Some(rest) = d.message.strip_prefix(FILE_SUBJECT) else {
        return d;
    };
    let (role, secret) = (owner.role, &owner.secret);
    d.message = format!("The SQLite {role} file (secret {secret}) {rest}");
    // What the operation that hit the lock had committed: nothing. Inspection
    // never writes and replay never commits. On a target, BEGIN
    // IMMEDIATE never began, a statement or the ledger insert rolls
    // back with its migration, and a busy COMMIT leaves the transaction
    // open until the session is discarded.
    if d.code == "DB_BUSY" {
        d.message.push(' ');
        d.message.push_str(match (role, execute) {
            ("scratch", _) => "Replay never commits: nothing was committed, and the target was not changed.",
            (_, true) => "Nothing was committed by the transaction that hit the lock; progress shows any migration this run committed before it.",
            _ => "Inspection writes nothing, so nothing was committed.",
        });
    }
    if d.code == "DB_SQLITE" {
        d.next_action = if role == "scratch" {
            format!("Make the scratch file that {secret} names, and its directory, writable by the user running ntnt (SQLite writes its journal or WAL beside the file), or set {secret} to another empty, writable file; then rerun.")
        } else {
            format!("Make the target file that {secret} names, and its directory, writable by the user running ntnt (SQLite writes its journal or WAL beside the file). A plan binds the file's permissions, so then plan, review and approve again, and run migrate with the new plan.")
        };
    }
    d
}

struct State {
    // Drop SQLite before the Windows pathname guard.
    conn: Connection,
    file: FileBinding,
    writable: bool,
    auth: Authorizer,
    /// What earlier catalog inspections on this session can lend later ones.
    cache: Cache,
    /// The migration `execute_captured` is running, for the rebuild guard's
    /// advice.
    migration_sql: Option<String>,
    /// The migration whose transaction is open and the tables it dropped,
    /// for a COMMIT that deferred foreign keys fail.
    dropped: Option<(String, Vec<String>)>,
}
impl State {
    fn identity(&self) -> Result<Identity> {
        let authority = self.file.check_connection(&self.conn)?;
        let settings = json!({
            "encoding":self.conn.query_row("PRAGMA encoding",[],|r|r.get::<_,String>(0)).map_err(sql_error)?,
            "page_size":integer(&self.conn,"PRAGMA page_size")?,
            "auto_vacuum":integer(&self.conn,"PRAGMA auto_vacuum")?,
            "journal_mode":self.conn.query_row("PRAGMA journal_mode",[],|r|r.get::<_,String>(0)).map_err(sql_error)?,
        });
        if !matches!(
            settings["journal_mode"].as_str(),
            Some("delete" | "wal" | "truncate" | "persist")
        ) {
            return Err(Diagnostic::new(
                "DB_SQLITE_SETTINGS",
                "SQLite requires a durable rollback-journal or WAL mode.",
            ));
        }
        Ok(Identity {
            database: "main".into(),
            file_id: self.file.id().into(),
            version: rusqlite::version_number(),
            settings,
            authority,
        })
    }
    /// The catalog. See `inspect`.
    fn catalog(&mut self) -> Result<Catalog> {
        self.inspect()?;
        Ok(self.cache.catalog())
    }
    /// What the catalog has outside coverage, and its fingerprint, without
    /// building it: exactly `catalog().uncovered` and `json_digest(&catalog())`.
    fn fingerprint(&mut self) -> Result<(Vec<String>, String)> {
        self.inspect()?;
        Ok((self.cache.uncovered().to_vec(), self.cache.fingerprint()))
    }
    /// Bring this session's view of the catalog up to date. Every object is
    /// checked against `sqlite_schema` and `pragma_table_list`; only groups
    /// of objects whose definitions or facts changed are read and compiled
    /// again (see `sqlite_cache`).
    fn inspect(&mut self) -> Result<()> {
        self.file.check_connection(&self.conn)?;
        self.history(None)?;
        self.cache.stats.catalogs += 1;
        let table_info = rows(
            &self.conn,
            "SELECT name,type,wr,strict FROM pragma_table_list WHERE schema='main'",
            &[],
        )?;
        // The first row of each name, as a search of the list finds it.
        let mut details: HashMap<&str, &Vec<Value>> = HashMap::new();
        for row in &table_info {
            if let Some(name) = row[0].as_str() {
                details.entry(name).or_insert(row);
            }
        }
        let tables: HashSet<&str> = table_info
            .iter()
            .filter(|r| r[1] == "table")
            .filter_map(|r| r[0].as_str())
            .collect();
        let objects=rows(&self.conn,"SELECT type,name,tbl_name,CASE WHEN length(sql)<=1048576 THEN sql ELSE NULL END,coalesce(length(sql),0)<=1048576 AND length(name)<=255 AND length(tbl_name)<=255 FROM main.sqlite_schema ORDER BY type,name LIMIT 10001",&[])?;
        let positions = self.cache.refresh(&objects);
        // What an object uses from pragma_table_list: a table's row, or
        // whether a trigger's table is an ordinary table.
        let fact = |object: &[Value]| -> Value {
            match object[0].as_str() {
                Some("table") => object[1]
                    .as_str()
                    .and_then(|name| details.get(name))
                    .map_or(Value::Null, |row| json!(row)),
                Some("trigger") => json!(object[2].as_str().is_some_and(|t| tables.contains(t))),
                _ => Value::Null,
            }
        };
        let unchanged = |stored: &Value, object: &[Value]| -> bool {
            match object[0].as_str() {
                Some("table") => match object[1].as_str().and_then(|name| details.get(name)) {
                    Some(row) => stored.as_array() == Some(*row),
                    None => stored.is_null(),
                },
                Some("trigger") => {
                    stored.as_bool() == Some(object[2].as_str().is_some_and(|t| tables.contains(t)))
                }
                _ => stored.is_null(),
            }
        };
        let stale: HashSet<&str> = objects
            .iter()
            .zip(&positions)
            .filter(|(object, (group, position))| {
                self.cache
                    .output(group)
                    .is_some_and(|output| !unchanged(&output.facts[*position], object))
            })
            .map(|(_, (group, _))| group.as_str())
            .collect();
        self.cache.forget(&stale);
        // The facts of each group to build, in row order.
        let mut facts: HashMap<&str, Vec<Value>> = HashMap::new();
        for (object, (group, _)) in objects.iter().zip(&positions) {
            if self.cache.output(group).is_none() {
                facts.entry(group.as_str()).or_default().push(fact(object));
            }
        }
        let mut built: HashMap<&str, Output> = HashMap::new();
        // `schema:main`, then each object with the columns it adds.
        let mut count = 1usize;
        for (object, (group, position)) in objects.iter().zip(&positions) {
            object[0]
                .as_str()
                .ok_or_else(|| Diagnostic::new("DB_SQLITE", "Invalid catalog kind."))?;
            let name = object[1].as_str().unwrap();
            let table = object[2].as_str().unwrap();
            if reserved(name) || reserved(table) {
                continue;
            }
            // SQLite's AUTOINCREMENT counters and planner statistics are
            // state, not schema, and are left out like PostgreSQL's sequence
            // counters and statistics (#427). An AUTOINCREMENT table's own
            // definition, keyword included, stays in its definition hash.
            if object[0] == "table" && engine_state(name) {
                continue;
            }
            if object[4] != 1 {
                return Err(Diagnostic::new(
                    "DB_CATALOG_LIMIT",
                    "SQLite definition or object name exceeds inspection bounds.",
                ));
            }
            let contributed = match self.cache.output(group) {
                Some(output) => output.contributions[*position],
                None => {
                    let output = built
                        .entry(group.as_str())
                        .or_insert_with(|| Output::new(facts.remove(group.as_str()).unwrap()));
                    let contributed = self.object(output, object, &details, &tables)?;
                    output.contributions[*position] = contributed;
                    contributed
                }
            };
            count += contributed;
            if count > 10000 {
                return Err(Diagnostic::new(
                    "DB_CATALOG_LIMIT",
                    "SQLite catalog exceeds 10,000 objects.",
                ));
            }
        }
        for (group, output) in built {
            self.cache.install(group, output);
        }
        if !self.cache.within_metadata_coverage() {
            self.cache.finish(None);
            return Ok(());
        }
        // Compile only: never step an application query during inspection.
        // Native authorization covers trigger/view/default programs as well
        // as their resolved base relations; SQL definitions remain opaque.
        let units = self.cache.plan();
        // Foreign-key enforcement changes what DML programs compile to;
        // rebuild migrations inspect with it off. If it cannot be read,
        // nothing is reused.
        let enforced = self
            .conn
            .db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY)
            .ok();
        // Programs nested this deep do not compile ("triggers nested too
        // deep"); reuse after a change below a program needs its tree to stay
        // shallower. If the limit cannot be read, that reuse is off.
        let depth_limit = self
            .conn
            .limit(Limit::SQLITE_LIMIT_TRIGGER_DEPTH)
            .map_or(0, |limit| u64::try_from(limit).unwrap_or(0));
        let State {
            conn, auth, cache, ..
        } = self;
        // One allowlist serves every program: inspection changes no schema.
        let (mut active, mut allowlists) = (None, 0);
        let failures = cache.decide(&units, enforced, depth_limit, |programs| {
            let mut failed = Vec::new();
            for (key, sql) in programs {
                let active = active.get_or_insert_with(|| {
                    allowlists += 1;
                    auth.activate(conn, Policy::Definition)
                });
                if !(active.is_ok() && conn.prepare(sql).is_ok()) {
                    failed.push(key.clone());
                }
            }
            failed
        });
        drop(active);
        cache.stats.allowlists += allowlists;
        cache.finish(Some(failures));
        Ok(())
    }
    /// Add one object (and a table's columns) to `output`; returns how many
    /// catalog objects it adds.
    fn object(
        &mut self,
        output: &mut Output,
        object: &[Value],
        details: &HashMap<&str, &Vec<Value>>,
        tables: &HashSet<&str>,
    ) -> Result<usize> {
        let (kind, name, table, sql) = (
            object[0].as_str().unwrap(),
            object[1].as_str().unwrap(),
            object[2].as_str().unwrap(),
            &object[3],
        );
        let key = format!("{kind}:{}", quote(name));
        let mut item =
            json!({"table":table,"definition":{"sha256":json_digest(sql),"redacted":true}});
        let mut supported = !internal(name) || name.starts_with("sqlite_autoindex_");
        let mut contributed = 1;
        match kind {
            "table" => {
                let detail = details.get(name).ok_or_else(|| {
                    Diagnostic::new("DB_CATALOG_AMBIGUOUS", "Missing SQLite table metadata.")
                })?;
                supported &= detail[1] == "table";
                item["without_rowid"] = detail[2].clone();
                item["strict"] = detail[3].clone();
                let metadata = table_metadata(&self.conn, &mut self.cache.stats, name)?;
                let assignment = metadata
                    .column_names
                    .iter()
                    .map(|name| format!("{}={}", quote(name), quote(name)))
                    .collect::<Vec<_>>()
                    .join(",");
                for query in [
                    format!("EXPLAIN INSERT INTO main.{} DEFAULT VALUES", quote(name)),
                    format!("EXPLAIN UPDATE main.{} SET {assignment}", quote(name)),
                    format!("EXPLAIN DELETE FROM main.{}", quote(name)),
                ] {
                    output.programs.push((key.clone(), query));
                }
                if detail[2] == 0 {
                    // UPDATE OF can name SQLite's hidden row identifiers,
                    // absent from table_xinfo. Each spelling has its own
                    // trigger event match, even when aliases share storage.
                    // WITHOUT ROWID tables have only the declared columns.
                    for alias in ["rowid", "_rowid_", "oid"] {
                        output.programs.push((
                            key.clone(),
                            format!(
                                "EXPLAIN UPDATE main.{} SET {}={}",
                                quote(name),
                                quote(alias),
                                quote(alias)
                            ),
                        ));
                    }
                }
                contributed += metadata.columns.len();
                output.entries.extend(metadata.columns);
                output.foreign_keys.extend(ForeignKey::parse(
                    metadata.foreign_keys.as_array().map_or(&[], Vec::as_slice),
                ));
                output.primary_key = metadata.primary_key;
                supported &= metadata.visible;
                item["foreign_keys"] = metadata.foreign_keys;
                supported &= metadata.foreign_keys_supported;
            }
            "index" => {
                let metadata = index_metadata(&self.conn, &mut self.cache.stats, table, name)?;
                supported &= metadata.columns_supported;
                item["columns"] = metadata.columns;
                item["properties"] = metadata.properties;
                supported &= metadata.properties_supported;
            }
            "view" => output.programs.push((
                key.clone(),
                format!("EXPLAIN SELECT * FROM main.{}", quote(name)),
            )),
            "trigger" => supported &= tables.contains(table),
            _ => supported = false,
        }
        if !supported {
            output
                .outside
                .push(((kind.to_owned(), name.to_owned()), key.clone()));
        }
        output.entries.push((key, item));
        Ok(contributed)
    }
    fn history(&self, database: Option<&str>) -> Result<History> {
        let bad = || {
            Diagnostic::new("DB_LEDGER_UNKNOWN","Unrecognized SQLite reserved ledger or dependencies; no adoption or repair is automatic.")
        };
        let objects=rows(&self.conn,"SELECT type,name,tbl_name,sql FROM main.sqlite_schema WHERE lower(substr(name,1,5))='_ntnt' OR lower(substr(tbl_name,1,5))='_ntnt' ORDER BY name",&[])?;
        if objects.is_empty() {
            return Ok(History {
                present: false,
                entries: vec![],
                scope: None,
            });
        }
        if objects.len() != 1
            || objects[0][0] != "table"
            || objects[0][1] != LEDGER
            || objects[0][2] != LEDGER
            || objects[0][3] != LEDGER_SQL
        {
            return Err(bad());
        }
        let all=rows(&self.conn,"SELECT position,id,name,checksum,state,logical_database,managed_schema FROM main._ntnt_schema_migrations_v2 ORDER BY position LIMIT 1025",&[]).map_err(|_|bad())?;
        if all.is_empty() || all.len() > 1024 {
            return Err(bad());
        }
        let scope = json!({"database":all[0][5],"schema":all[0][6]});
        let mut entries = Vec::new();
        for row in all {
            if row[0].as_i64().is_none()
                || row[1..]
                    .iter()
                    .any(|v| v.as_str().is_none_or(|v| v.len() > 255))
                || row[3].as_str().unwrap().len() != 64
                || json!({"database":row[5],"schema":row[6]}) != scope
            {
                return Err(bad());
            }
            entries.push(HistoryEntry {
                position: row[0].as_i64().unwrap(),
                id: row[1].as_str().unwrap().into(),
                name: row[2].as_str().unwrap().into(),
                checksum: row[3].as_str().unwrap().into(),
                state: row[4].as_str().unwrap().into(),
            });
        }
        if database.is_some_and(|name| scope != json!({"database":name,"schema":"main"})) {
            return Err(Diagnostic::new(
                "DB_LEDGER_SCOPE",
                "SQLite ledger belongs to another logical database or scope.",
            ));
        }
        Ok(History {
            present: true,
            scope: Some(scope),
            entries,
        })
    }
    fn foreign_keys(&self) -> Result<()> {
        let mut stmt = self
            .conn
            .prepare("PRAGMA main.foreign_key_check")
            .map_err(sql_error)?;
        if stmt
            .query([])
            .map_err(sql_error)?
            .next()
            .map_err(sql_error)?
            .is_some()
        {
            return Err(Diagnostic::new(
                "DB_FOREIGN_KEY",
                "SQLite foreign-key verification failed; row values suppressed.",
            ));
        }
        Ok(())
    }
}

/// Each engine setting that differs between the target and the scratch, with
/// both values, such as `page_size: target 4096, scratch 1024`.
fn differences(target: &Identity, scratch: &Identity) -> String {
    let shown = |value: &Value| {
        value
            .as_str()
            .map_or_else(|| value.to_string(), str::to_owned)
    };
    let mut differences = Vec::new();
    if target.version != scratch.version {
        differences.push(format!(
            "SQLite version: target {}, scratch {}",
            target.version, scratch.version
        ));
    }
    let keys: std::collections::BTreeSet<&String> = [&target.settings, &scratch.settings]
        .into_iter()
        .filter_map(Value::as_object)
        .flat_map(|settings| settings.keys())
        .collect();
    for key in keys {
        let (a, b) = (
            &target.settings[key.as_str()],
            &scratch.settings[key.as_str()],
        );
        if a != b {
            differences.push(format!("{key}: target {}, scratch {}", shown(a), shown(b)));
        }
    }
    differences.join("; ")
}

/// Give an empty scratch the target's encoding, page size and auto-vacuum
/// where they differ (#425), and say whether any did. Only values SQLite
/// itself can report are set; SQLite ignores one it can no longer change.
fn adopt_layout(conn: &Connection, scratch: &Value, target: &Value) -> Result<bool> {
    let mut pragmas = Vec::new();
    if scratch["encoding"] != target["encoding"] {
        if let Some(encoding @ ("UTF-8" | "UTF-16le" | "UTF-16be")) = target["encoding"].as_str() {
            pragmas.push(format!("PRAGMA encoding='{encoding}'"));
        }
    }
    if scratch["page_size"] != target["page_size"] {
        if let Some(size) = target["page_size"]
            .as_u64()
            .filter(|size| (512..=65536).contains(size) && size.is_power_of_two())
        {
            pragmas.push(format!("PRAGMA main.page_size={size}"));
        }
    }
    if scratch["auto_vacuum"] != target["auto_vacuum"] {
        if let Some(mode) = target["auto_vacuum"].as_u64().filter(|mode| *mode <= 2) {
            pragmas.push(format!("PRAGMA main.auto_vacuum={mode}"));
        }
    }
    for pragma in &pragmas {
        conn.execute_batch(pragma).map_err(sql_error)?;
    }
    Ok(!pragmas.is_empty())
}

/// A narrow, explicit mode for rebuilding one existing table. It may change that
/// table's column/constraint definition, but not silently retire other tables or
/// named dependent objects. Row counts are a guard, not proof of business meaning.
struct RebuildGuard {
    /// The rebuilt table, as the schema spells it.
    table: String,
    required: std::collections::BTreeMap<String, Value>,
    counts: std::collections::BTreeMap<String, i64>,
    /// The table's AUTOINCREMENT counter (`sqlite_sequence`), if it has one.
    sequence: Option<i64>,
}
impl RebuildGuard {
    fn counts(state: &State) -> Result<std::collections::BTreeMap<String, i64>> {
        let mut counts = std::collections::BTreeMap::new();
        for row in rows(
            &state.conn,
            "SELECT name FROM main.sqlite_schema WHERE type='table' ORDER BY name",
            &[],
        )? {
            let name = row[0].as_str().unwrap();
            if reserved(name) || internal(name) {
                continue;
            }
            counts.insert(
                name.to_owned(),
                integer(
                    &state.conn,
                    &format!("SELECT count(*) FROM main.{}", quote(name)),
                )?,
            );
        }
        Ok(counts)
    }
    /// The AUTOINCREMENT counter SQLite keeps for `table`, which it looks up
    /// by the table's exact name, and how many `sqlite_sequence` rows hold
    /// one for that name. Nothing keeps the name unique there, and SQLite
    /// reads and updates only the first row for it.
    fn sequence(state: &State, table: &str) -> Result<(Option<i64>, i64)> {
        let present = integer(
            &state.conn,
            "SELECT count(*) FROM main.sqlite_schema WHERE type='table' AND name='sqlite_sequence'",
        )? != 0;
        if !present {
            return Ok((None, 0));
        }
        let found = rows(
            &state.conn,
            "SELECT max(CASE WHEN typeof(seq) = 'integer' THEN seq END), count(*) FROM main.sqlite_sequence WHERE name = ?1",
            &[&table],
        )?;
        let row = found.first();
        Ok((
            row.and_then(|row| row[0].as_i64()),
            row.and_then(|row| row[1].as_i64()).unwrap_or(0),
        ))
    }
    fn capture(state: &mut State, table: &str) -> Result<Self> {
        let catalog = state.catalog()?;
        if !catalog.uncovered.is_empty() {
            return Err(Diagnostic::new(
                "DB_COVERAGE_INCOMPLETE",
                "SQLite rebuild requires complete supported catalog coverage.",
            )
            .with_objects(&catalog.uncovered));
        }
        let counts = Self::counts(state)?;
        let selected = counts
            .keys()
            .find(|name| name.eq_ignore_ascii_case(table))
            .ok_or_else(|| {
                Diagnostic::new(
                    "DB_REBUILD_GUARD",
                    "sqlite_rebuild must name an existing ordinary table.",
                )
            })?
            .clone();
        let table_key = format!("table:{}", quote(&selected));
        let column_prefix = format!("column:{}.", quote(&selected));
        let required = catalog
            .objects
            .into_iter()
            .filter(|(key, _)| {
                key != &table_key
                    && !key.starts_with(&column_prefix)
                    && !key.starts_with("index:\"sqlite_autoindex_")
            })
            .collect();
        let (sequence, _) = Self::sequence(state, &selected)?;
        Ok(Self {
            table: selected,
            required,
            counts,
            sequence,
        })
    }
    /// Refuse a rebuild that did not keep what it must, naming each rule it
    /// broke and the object: names and row counts, never row values (F2).
    fn verify(self, state: &mut State) -> Result<()> {
        let catalog = state.catalog()?;
        let counts = Self::counts(state)?;
        let table = &self.table;
        let mut broken = Vec::new();
        let mut objects = Vec::new();
        let (mut rows, mut definitions, mut stored, mut tables) = (false, false, false, false);
        for (name, before) in &self.counts {
            match counts.get(name) {
                None => broken.push(format!("table `{name}` is missing")),
                Some(after) if after != before => broken.push(format!(
                    "the row count of table `{name}` changed from {before} to {after}"
                )),
                Some(_) => continue,
            }
            rows = true;
            objects.push(format!("table:{}", quote(name)));
        }
        for name in counts
            .keys()
            .filter(|name| !self.counts.contains_key(*name))
        {
            broken.push(format!("table `{name}` is new"));
            rows = true;
            objects.push(format!("table:{}", quote(name)));
        }
        for (key, value) in &self.required {
            match catalog.objects.get(key) {
                None => broken.push(format!("{} is missing", subject(key))),
                Some(now) if now != value => {
                    // Only the definition hash differs: the stored SQL text.
                    let text =
                        now.as_object()
                            .zip(value.as_object())
                            .is_some_and(|(now, before)| {
                                now.iter().all(|(field, v)| {
                                    field == "definition" || before.get(field) == Some(v)
                                }) && now.len() == before.len()
                            });
                    stored |= text;
                    broken.push(if text {
                        format!(
                            "{} has another stored SQL text than before the rebuild",
                            subject(key)
                        )
                    } else {
                        format!(
                            "{} differs from its definition before the rebuild",
                            subject(key)
                        )
                    });
                }
                Some(_) => continue,
            }
            // Another table, or one of its columns.
            if key.starts_with("table:") || key.starts_with("column:") {
                tables = true;
            } else {
                definitions = true;
            }
            objects.push(key.clone());
        }
        // AUTOINCREMENT never reuses an id (F3). The counter follows the
        // table's name, so a rebuild that copies the rows into a new table
        // starts it again from the highest id copied, unless the migration
        // copies it. A rebuilt table without AUTOINCREMENT has no counter.
        // SQLite uses only the first of several counter rows for a name, so
        // a second one leaves the counter it uses in doubt (review).
        let rebuilt = counts.keys().find(|name| name.eq_ignore_ascii_case(table));
        let mut sequence = None;
        if let Some(rebuilt) = rebuilt {
            let (after, held) = Self::sequence(state, rebuilt)?;
            let broke = if held > 1 && autoincrement(state, rebuilt)? {
                Some(format!("table `{rebuilt}` has {held} AUTOINCREMENT counters in sqlite_sequence, and SQLite reads and updates only the first, so it could give new rows the ids of deleted ones"))
            } else if let Some(before) = self.sequence {
                (after.is_none_or(|after| after < before)
                    && (after.is_some() || autoincrement(state, rebuilt)?))
                .then(|| match after {
                    Some(after) => format!("the AUTOINCREMENT counter of table `{rebuilt}` went back from {before} to {after} (sqlite_sequence), so SQLite would give new rows the ids of deleted ones"),
                    None => format!("table `{rebuilt}` lost its AUTOINCREMENT counter (sqlite_sequence held {before} for it and now holds none), so SQLite would give new rows the ids of deleted ones"),
                })
            } else {
                None
            };
            if let Some(broke) = broke {
                broken.push(broke);
                sequence = Some(rebuilt.clone());
                objects.push(format!("table:{}", quote(rebuilt)));
            }
        }
        if !catalog.uncovered.is_empty() {
            broken.push("the rebuilt catalog has objects outside supported coverage".to_owned());
            objects.extend(catalog.uncovered.iter().cloned());
        }
        if broken.is_empty() {
            return Ok(());
        }
        const SHOWN: usize = 8;
        let more = broken.len().saturating_sub(SHOWN);
        broken.truncate(SHOWN);
        let mut message = format!(
            "The rebuild of table `{table}` did not keep what a rebuild must: {}{}.",
            broken.join("; "),
            if more > 0 {
                format!("; and {more} more")
            } else {
                String::new()
            }
        );
        if stored {
            message.push_str(" SQLite compares the stored SQL text of a definition, so whitespace and quoting must match the original.");
        }
        message.push_str(" Row values suppressed.");
        let mut advice = Vec::new();
        if rows {
            advice.push(format!("Copy every row of `{table}` into its replacement, keep every other table and its rows, and leave no new table behind; make other data changes in a separate migration without the sidecar."));
        }
        if tables {
            advice.push(format!("Leave the other tables as they were. SQLite rewrites the foreign keys of other tables that reference a table it renames, so create the replacement under a new name and rename it to `{table}` after dropping the old table, never rename the old table first; change other tables in a separate migration without the sidecar."));
        }
        if definitions {
            advice.push("Recreate each index, trigger and view the message names after the rename, with the CREATE statement it had before, copied exactly from the target's `SELECT sql FROM sqlite_master WHERE name = '<name>'`; only the rebuilt table, its columns and its autoindexes may change. Change or retire another object in a separate migration without the sidecar.".to_owned());
        }
        if let Some(rebuilt) = sequence {
            // Any write to the replacement, even one that copies no row,
            // gives it a counter row, so the repair raises that row or adds one.
            let (from, to) = (
                literal(&rebuilt),
                literal(&replacement_name(state, &rebuilt)),
            );
            advice.push(format!("Before the DROP TABLE, copy the counter to the replacement table: `UPDATE sqlite_sequence SET seq = (SELECT max(seq) FROM sqlite_sequence WHERE name IN ('{from}', '{to}')) WHERE name='{to}'; INSERT INTO sqlite_sequence (name, seq) SELECT '{to}', max(seq) FROM sqlite_sequence WHERE name='{from}' AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='{to}') HAVING count(*) > 0;`. A statement that writes the replacement gives it a counter row, even when it copies no row. The UPDATE raises that row to the higher of the two counters, and the INSERT adds one row only when it has none, so the replacement keeps one counter, never lower than that of `{rebuilt}` or its own."));
        }
        if !catalog.uncovered.is_empty() {
            advice.push("Keep the rebuilt catalog within the coverage in docs/DATABASE_MIGRATIONS.md, Supported SQL and catalog coverage.".to_owned());
        }
        advice.push("Nothing from this migration was committed. Then plan again (docs/DATABASE_MIGRATIONS.md, Explicit one-table rebuild).".to_owned());
        let mut d = Diagnostic::new("DB_REBUILD_GUARD", &message);
        d.next_action = advice.join(" ");
        let mut seen = HashSet::new();
        objects.retain(|key| seen.insert(key.clone()));
        Err(d.with_objects(&objects))
    }
}

/// `index `name``, `column `t.c``, ..., for the catalog key of an object.
fn subject(key: &str) -> String {
    let Some((kind, quoted)) = key.split_once(':') else {
        return format!("`{key}`");
    };
    let name = quoted
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .map_or_else(
            || quoted.to_owned(),
            |inner| {
                inner
                    .split("\".\"")
                    .map(|part| part.replace("\"\"", "\""))
                    .collect::<Vec<_>>()
                    .join(".")
            },
        );
    format!("{kind} `{name}`")
}

/// `name` inside a SQL string literal.
fn literal(name: &str) -> String {
    name.replace('\'', "''")
}

/// Whether table `name`'s stored definition declares AUTOINCREMENT.
fn autoincrement(state: &State, name: &str) -> Result<bool> {
    Ok(rows(
        &state.conn,
        "SELECT sql FROM main.sqlite_schema WHERE type='table' AND name=?1",
        &[&name],
    )?
    .first()
    .and_then(|row| row[0].as_str())
    .is_some_and(super::analysis::sqlite_autoincrement))
}

/// The table the migration renamed to `table`, which held the copied rows
/// before the rename, or a placeholder when the guard cannot tell.
fn replacement_name(state: &State, table: &str) -> String {
    state
        .migration_sql
        .as_deref()
        .and_then(|sql| super::analysis::sqlite_renamed_to(sql, table))
        .unwrap_or_else(|| "<replacement table>".to_owned())
}

pub(super) struct Sqlite {
    state: Arc<Mutex<State>>,
    /// Connected to execute on a target, where the plan's statement limit
    /// bounds each migration; scratch replay keeps the fixed bound.
    execute: bool,
    interrupted: Arc<AtomicBool>,
    interrupt: rusqlite::InterruptHandle,
}
impl Drop for Sqlite {
    fn drop(&mut self) {
        self.interrupted.store(true, Ordering::Release);
        self.interrupt.interrupt();
    }
}
impl Sqlite {
    pub async fn connect(secret: &crate::interpreter::SecretValue, access: Access) -> Result<Self> {
        let path = std::path::PathBuf::from(secret.expose());
        let execute = matches!(access, Access::Execute(_));
        // File identity refusals name the role and its secret, never the path.
        let owner = super::sqlite_file::Owner {
            role: if matches!(access, Access::Scratch(_)) {
                "scratch"
            } else {
                "target"
            },
            secret: secret.name().to_owned(),
        };
        // Opening reads the schema, so it can find the file locked too.
        let faults = super::sqlite_file::Owner {
            role: owner.role,
            secret: owner.secret.clone(),
        };
        let state=tokio::time::timeout(IO_TIMEOUT,tokio::task::spawn_blocking(move || {
            if rusqlite::version_number()/1000!=3054 {return Err(Diagnostic::new("DB_ENGINE_VERSION","SQLite adapter supports the bundled SQLite 3.54 series."));}
            let file=FileBinding::capture(&path,owner)?;
            let writable=!matches!(access,Access::Inspect);
            match &access {
                Access::Execute(expected) if expected["file_id"]!=file.id()=>return Err(Diagnostic::new("DB_WRONG_TARGET","SQLite file identity differs from the reviewed target; no writable connection opened.")),
                Access::Scratch(target) if target["file_id"]==file.id()=>return Err(Diagnostic::new("DB_SCRATCH_COLLISION","Scratch resolves to the selected deployment file.")),
                _=>(),
            }
            let flags=if writable {OpenFlags::SQLITE_OPEN_READ_WRITE} else {OpenFlags::SQLITE_OPEN_READ_ONLY};
            let mut conn=Connection::open_with_flags(file.path(),flags|OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(sql_error)?;
            file.check_connection(&conn)?;
            conn.busy_timeout(BUSY_TIMEOUT).map_err(sql_error)?;
            conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH,1_048_576).map_err(sql_error)?;
            conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,16*1_048_576).map_err(sql_error)?;
            conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_EXPR_DEPTH,64).map_err(sql_error)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE,true).map_err(sql_error)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DDL,false).map_err(sql_error)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML,false).map_err(sql_error)?;
            conn.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA ignore_check_constraints=OFF; PRAGMA legacy_alter_table=OFF; PRAGMA synchronous=FULL;").map_err(sql_error)?;
            let auth=Authorizer::install(&mut conn)?;
            Ok::<_,Diagnostic>(State{conn,file,writable,auth,cache:Cache::default(),migration_sql:None,dropped:None})
        })).await.map_err(|_|Diagnostic::new("DB_TIMEOUT","SQLite connection deadline exceeded."))?
            .map_err(|_|Diagnostic::new("DB_RUNTIME","SQLite connection worker failed."))?
            .map_err(|d| attribute(&faults, d, execute))?;
        let interrupt = state.conn.get_interrupt_handle();
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            execute,
            interrupted: Arc::new(AtomicBool::new(false)),
            interrupt,
        })
    }
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut State) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        self.run_for(IO_TIMEOUT, operation).await
    }
    /// Run `operation` under `limit`: SQLite interrupts it at the limit, and the
    /// session is discarded only if it has not returned `DISCARD_GRACE` later.
    async fn run_for<T: Send + 'static>(
        &self,
        limit: Duration,
        operation: impl FnOnce(&mut State) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let state = self.state.clone();
        let cancelled = self.interrupted.clone();
        let execute = self.execute;
        let work = tokio::task::spawn_blocking(move || {
            let mut state = state.lock().map_err(|_| {
                Diagnostic::new(
                    "DB_RUNTIME",
                    "SQLite session is unusable and must be discarded.",
                )
            })?;
            if cancelled.load(Ordering::Acquire) {
                return Err(Diagnostic::new(
                    "DB_CANCELLED",
                    "SQLite session was cancelled and discarded.",
                ));
            }
            let deadline = Instant::now() + limit;
            state
                .conn
                .progress_handler(
                    1000,
                    Some(move || cancelled.load(Ordering::Acquire) || Instant::now() >= deadline),
                )
                .map_err(sql_error)?;
            operation(&mut state).map_err(|d| attribute(state.file.owner(), d, execute))
        });
        match tokio::time::timeout(limit + DISCARD_GRACE, work).await {
            Ok(result) => result
                .map_err(|_| Diagnostic::new("DB_RUNTIME", "SQLite operation worker failed."))?,
            Err(_) => {
                self.interrupted.store(true, Ordering::Release);
                self.interrupt.interrupt();
                Err(Diagnostic::new(
                    "DB_TIMEOUT",
                    "SQLite operation deadline exceeded; dedicated session discarded.",
                ))
            }
        }
    }
    pub async fn identity(&self) -> Result<Identity> {
        self.run(|s| s.identity()).await
    }
    pub async fn begin_readonly(&self) -> Result<()> {
        self.run(|s| {
            s.file.check_connection(&s.conn)?;
            s.conn.execute_batch("BEGIN DEFERRED").map_err(sql_error)
        })
        .await
    }
    pub async fn begin_migration(&self) -> Result<()> {
        self.run(|s| {
            s.file.check_connection(&s.conn)?;
            if !s.writable {
                return Err(Diagnostic::new(
                    "DB_READ_ONLY",
                    "SQLite inspection session cannot migrate.",
                ));
            }
            s.dropped = None;
            s.conn.execute_batch("BEGIN IMMEDIATE").map_err(sql_error)
        })
        .await
    }
    /// Roll back the open transaction. Without one (a scratch refused before
    /// `BEGIN IMMEDIATE`) there is nothing to undo, which is not a failure.
    pub async fn rollback(&self) -> Result<()> {
        self.run(|s| {
            s.dropped = None;
            if s.conn.is_autocommit() {
                return Ok(());
            }
            s.conn.execute_batch("ROLLBACK").map_err(sql_error)
        })
        .await
    }
    pub async fn commit(&self) -> Result<()> {
        let replay = !self.execute;
        self.run(move |s| {
            let dropped = s.dropped.take();
            s.conn.execute_batch("COMMIT").map_err(|error| {
                let foreign_key = matches!(&error, rusqlite::Error::SqliteFailure(e, _)
                    if e.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY);
                let mut d = sql_error(error);
                // Deferred foreign keys fail at COMMIT, after a migration
                // dropped a table that other tables reference (F1).
                if let (true, Some((source, tables))) = (foreign_key, dropped) {
                    if let Some(advice) = dropped_parent_advice(&s.conn, None, &tables, replay) {
                        d.message = format!("SQLite could not commit the migration: {}", d.message);
                        d.next_action = advice;
                        d.source = Some(source);
                    }
                }
                d
            })
        })
        .await
    }
    pub async fn catalog(&self) -> Result<Catalog> {
        self.run(|s| s.catalog()).await
    }
    /// `catalog().uncovered` and `json_digest(&catalog())`, without the catalog.
    pub async fn fingerprint(&self) -> Result<(Vec<String>, String)> {
        self.run(|s| s.fingerprint()).await
    }
    /// `catalog().uncovered`, without the catalog.
    pub async fn coverage(&self) -> Result<Vec<String>> {
        self.run(|s| {
            s.inspect()?;
            Ok(s.cache.uncovered().to_vec())
        })
        .await
    }
    /// Inspection work done on this session so far.
    #[cfg(test)]
    pub async fn stats(&self) -> Result<Stats> {
        self.run(|s| Ok(s.cache.stats)).await
    }
    /// Lower SQLite's limit on nested trigger programs for this session, so
    /// a test reaches it with a small schema.
    #[cfg(test)]
    pub async fn set_trigger_depth_limit(&self, limit: i32) -> Result<()> {
        self.run(move |s| {
            s.conn
                .set_limit(Limit::SQLITE_LIMIT_TRIGGER_DEPTH, limit)
                .map_err(sql_error)?;
            Ok(())
        })
        .await
    }
    /// Compile `sql` under the definition policy, as inspection compiles a
    /// table's programs.
    #[cfg(test)]
    pub async fn compile(&self, sql: &str) -> Result<()> {
        let sql = sql.to_owned();
        self.run(move |s| {
            let _active = s.auth.activate(&s.conn, Policy::Definition)?;
            s.conn.prepare(&sql).map(drop).map_err(sql_error)
        })
        .await
    }
    /// The catalog as an inspection that reuses nothing reports it. The
    /// session keeps what it had for later inspections.
    #[cfg(test)]
    pub async fn fresh_catalog(&self) -> Result<Catalog> {
        self.run(|s| {
            let cache = std::mem::take(&mut s.cache);
            let fresh = s.catalog();
            s.cache = cache;
            fresh
        })
        .await
    }
    pub async fn history(&self, database: Option<&str>) -> Result<History> {
        let database = database.map(str::to_owned);
        self.run(move |s| s.history(database.as_deref())).await
    }
    pub async fn prepare_scratch(&self, target: &Identity) -> Result<Identity> {
        let target = target.clone();
        self.run(move |s| {
            let not_empty = || {
                Diagnostic::new(
                    "DB_SCRATCH_NOT_EMPTY",
                    "SQLite scratch must be an explicitly disposable empty file; it is never reset.",
                )
            };
            let empty = |s: &State| Ok::<_, Diagnostic>(integer(&s.conn, "SELECT count(*) FROM main.sqlite_schema")? == 0);
            let mut identity = s.identity()?;
            if identity.file_id == target.file_id {
                return Err(Diagnostic::new("DB_SCRATCH_COLLISION", "Scratch and target are the same physical file."));
            }
            if !empty(s)? {
                return Err(not_empty());
            }
            // The empty scratch takes the target's encoding, page size and
            // auto-vacuum where SQLite can still set them (#425), and then,
            // below, its journal mode: a file made with `touch` takes all
            // three. SQLite fixes the page size and auto-vacuum when the file
            // gets its first page, and the encoding when it first holds a
            // table (replay's tables roll back), and it ignores a setting it
            // can no longer change, so the settings are compared again below.
            if adopt_layout(&s.conn, &identity.settings, &target.settings)? {
                identity = s.identity()?;
            }
            // `std/db/sqlite` connect() puts every application database in WAL
            // mode, and a scratch made with `touch` is not. The scratch is
            // disposable and empty, so it takes the target's journal mode.
            let mode = target.settings["journal_mode"].as_str();
            if identity.settings["journal_mode"].as_str() != mode {
                if let Some(mode @ ("delete" | "wal" | "truncate" | "persist")) = mode {
                    s.conn
                        .query_row(&format!("PRAGMA main.journal_mode={mode}"), [], |r| r.get::<_, String>(0))
                        .map_err(sql_error)?;
                    identity = s.identity()?;
                }
            }
            if identity.version != target.version || identity.settings != target.settings {
                let mut d = Diagnostic::new(
                    "DB_SCRATCH_MISMATCH",
                    &format!(
                        "SQLite scratch settings differ from the deployment file: {}.",
                        differences(&target, &identity)
                    ),
                );
                d.next_action = format!("Replace the scratch file that {} names with a new empty file: delete it, then create an empty file at the same path, for example with `touch` (a 0-byte file is a valid SQLite database). ntnt gives an empty scratch the target's encoding, page size, auto-vacuum and journal mode before its first write. Do not reset the target.", s.file.owner().secret);
                return Err(d);
            }
            s.conn.execute_batch("BEGIN IMMEDIATE").map_err(sql_error)?;
            if !empty(s)? {
                return Err(not_empty());
            }
            Ok(identity)
        })
        .await
    }
    pub async fn execute_captured(&self, migration: &Migration, limit: Duration) -> Result<()> {
        let source = migration.name.clone();
        let migration = migration.clone();
        let replay = !self.execute;
        let limit = if replay { IO_TIMEOUT } else { limit };
        self.run_for(limit, move |s| {
            s.file.check_connection(&s.conn)?;
            if !s.writable || s.conn.is_autocommit() {
                return Err(Diagnostic::new(
                    "DB_TRANSACTION",
                    "SQLite execution requires an owned write transaction.",
                ));
            }
            if super::analysis::is_empty(&migration.sql, "sqlite") {
                return Err(super::empty_migration());
            }
            if !super::analysis::analyze_for("sqlite", &migration.sql, "main").replay_supported {
                return Err(replay_unsupported(&migration.sql));
            }
            let guard = migration
                .sqlite_rebuild
                .as_deref()
                .map(|table| RebuildGuard::capture(s, table))
                .transpose()?;
            if guard.is_some() {
                // SQLite's native configuration API (unlike PRAGMA) supports this
                // inside the already-owned transaction. Required for DROP/rebuild
                // without firing implicit ON DELETE cascades. Never authored SQL.
                if s.conn
                    .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, false)
                    .map_err(sql_error)?
                {
                    return Err(Diagnostic::new(
                        "DB_REBUILD_GUARD",
                        "Cannot enter the validated SQLite rebuild mode.",
                    ));
                }
            }
            s.migration_sql = Some(migration.sql.clone());
            let outcome = (|| {
                let dropped = execute_script(s, &migration.sql, limit, replay)?;
                s.dropped = Some((migration.name.clone(), dropped));
                s.history(None)?;
                s.foreign_keys()?;
                if let Some(guard) = guard {
                    guard.verify(s)?;
                }
                Ok(())
            })();
            // Restore enforcement before postconditions/ledger/COMMIT, on errors
            // too. A failed restoration discards the dedicated session.
            if !s
                .conn
                .set_db_config(DbConfig::SQLITE_DBCONFIG_ENABLE_FKEY, true)
                .map_err(sql_error)?
            {
                return Err(Diagnostic::new(
                    "DB_REBUILD_GUARD",
                    "Cannot restore SQLite foreign-key enforcement.",
                ));
            }
            outcome
        })
        .await
        .map_err(|mut d| {
            d.source = Some(source);
            d
        })
    }
    pub async fn check_purity(&self) -> Result<()> {
        self.run(|s| {
            s.inspect()?;
            if !s.cache.uncovered().is_empty() {
                return Err(Diagnostic::new(
                    "DB_CHECK_UNSUPPORTED",
                    "SQLite catalog exceeds contract coverage.",
                ));
            }
            Ok(())
        })
        .await
    }
    pub async fn check(&self, sql: &str) -> Result<()> {
        let sql = sql.to_owned();
        self.run(move|s|authored(s,Policy::Check,|conn|{
            if !super::analysis::contract_supported(&sql,"main") {return Err(Diagnostic::new("DB_CHECK_UNSUPPORTED","Unsupported SQLite contract syntax."));}
            let mut batch=rusqlite::Batch::new(conn,&sql);
            let mut stmt=batch.next().map_err(sql_error)?.ok_or_else(||Diagnostic::new("DB_CHECK_FAILED","Empty SQLite contract."))?;
            if batch.next().map_err(sql_error)?.is_some() || !stmt.readonly() {return Err(Diagnostic::new("DB_CHECK_UNSUPPORTED","SQLite contract must be one read-only SELECT."));}
            if stmt.column_count()!=1 {return Err(Diagnostic::new("DB_CHECK_FAILED","SQLite contract must return one Boolean column."));}
            let mut rows=stmt.query([]).map_err(sql_error)?;
            let passed=rows.next().map_err(sql_error)?.is_some_and(|r|matches!(r.get_ref(0),Ok(ValueRef::Integer(1)))) && rows.next().map_err(sql_error)?.is_none();
            if !passed {return Err(Diagnostic::new("DB_CHECK_FAILED","SQLite contract requires exactly one integer Boolean true (1); false, NULL, type and shape mismatches fail. Values suppressed."));}
            Ok(())
        })).await.map_err(|mut d|{if d.code=="DB_SQL_UNSUPPORTED" {d.code="DB_CHECK_UNSUPPORTED".into();}d})
    }
    pub async fn record_migration(
        &self,
        migration: &Migration,
        position: usize,
        database: &str,
        present: bool,
    ) -> Result<()> {
        self.record_entry(
            &HistoryEntry {
                position: position as i64,
                id: migration.id.to_string(),
                name: migration.name.clone(),
                checksum: migration.checksum.clone(),
                state: "applied".into(),
            },
            database,
            present,
        )
        .await
    }
    /// Insert a validated record inside the caller-owned migration transaction.
    /// Only an absent reserved ledger may be bootstrapped; eligibility is checked upstream.
    pub async fn record_entry(
        &self,
        entry: &HistoryEntry,
        database: &str,
        present: bool,
    ) -> Result<()> {
        let entry = entry.clone();
        let database = database.to_owned();
        self.run(move|s|{
            if !present {s.conn.execute_batch(LEDGER_SQL).map_err(sql_error)?;}
            s.conn.execute("INSERT INTO main._ntnt_schema_migrations_v2(position,id,name,checksum,state,logical_database,managed_schema) VALUES(?1,?2,?3,?4,?5,?6,'main')",rusqlite::params![entry.position,entry.id,entry.name,entry.checksum,entry.state,database]).map_err(sql_error)?;
            Ok(())
        }).await
    }
    pub fn capabilities(version: i32) -> Value {
        json!({"adapter":"sqlite-catalog-v1","canonicalization_version":1,"sqlite_version_number":version,
            "operations":{"catalog_inspection":"supported_subset","target_ddl":"exact_plan_transactional_subset","sql_contracts":"integer_boolean_native_authorizer","baseline_adoption":"reviewed_prefix_checkpoint","concurrent_indexes":"unsupported"},
            "guarantees":{"execution_transaction":"begin_immediate_before_inspection","atomic_target_ddl_checks_ledger":true,"checks":"native_authorizer_and_closed_expression_grammar","authority":"external_filesystem_and_secret_provider"}})
    }
}

#[cfg(test)]
mod tests {
    /// #427: a read that names no column, which SQLite reports without a
    /// database, may read a managed table or view or a common table
    /// expression, but not the ledger, SQLite's internal tables or the
    /// eponymous virtual tables other than the JSON ones.
    #[test]
    fn reads_without_a_database_resolve_to_main_or_the_statement() {
        let tables = ["events".to_owned(), "dbstat".to_owned()].into();
        for allowed in [
            "events",
            "EVENTS",
            "kinds",
            "json_each",
            "jsonb_tree",
            "dbstat",
        ] {
            assert!(super::unqualified_read(allowed, &tables), "{allowed}");
        }
        for refused in [
            "_ntnt_schema_migrations_v2",
            "sqlite_schema",
            "sqlite_temp_master",
            "fts3tokenize",
            "fts4aux",
            "pragma_table_list",
            "PRAGMA_quick_check",
        ] {
            assert!(!super::unqualified_read(refused, &tables), "{refused}");
        }
        assert!(!super::unqualified_read(
            "dbstat",
            &std::collections::HashSet::new()
        ));
    }

    /// #427: the migration function policy is the bundled SQLite's own
    /// built-in functions, less the two that reach outside the database.
    /// A libsqlite3-sys update that adds, drops or renames a built-in
    /// function fails here, so the list and its documentation are reviewed.
    #[test]
    fn builtin_functions_are_the_bundled_sqlite_builtins() {
        let list = super::BUILTIN_FUNCTIONS;
        assert!(
            list.windows(2).all(|pair| pair[0] < pair[1]),
            "BUILTIN_FUNCTIONS must stay sorted and unique for binary_search"
        );
        assert_eq!(rusqlite::version_number() / 1000, 3054);
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let names = |builtin: i64| -> std::collections::BTreeSet<String> {
            let mut statement = conn
                .prepare("SELECT DISTINCT name FROM pragma_function_list WHERE builtin = ?1")
                .unwrap();
            statement
                .query_map([builtin], |row| row.get(0))
                .unwrap()
                .map(Result::unwrap)
                .collect()
        };
        let mut expected: std::collections::BTreeSet<String> =
            list.iter().map(|name| (*name).to_owned()).collect();
        expected.extend(["load_extension".to_owned(), "sqlite_log".to_owned()]);
        assert_eq!(names(1), expected);
        // FTS3, FTS5 and R*Tree register functions of their own, which are
        // not built in and stay refused.
        let registered = names(0);
        assert!(registered.contains("fts3_tokenizer"), "{registered:?}");
        assert!(registered
            .iter()
            .all(|name| list.binary_search(&name.as_str()).is_err()));
        for refused in ["load_extension", "sqlite_log", "fts3_tokenizer", "snippet"] {
            assert!(list.binary_search(&refused).is_err(), "{refused}");
        }
    }

    /// CodeRabbit on #445 (sqlite.rs): DB_BUSY says only what is known.
    /// SQLITE_BUSY is another connection's lock, and the busy timeout is a
    /// limit, not a wait that happened. SQLITE_LOCKED is a conflict within
    /// ntnt's own connection, so its advice is not to wait for a writer.
    #[test]
    fn lock_errors_say_only_what_is_known() {
        let error = |code| {
            super::sql_error(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                None,
            ))
        };
        let busy = error(rusqlite::ffi::SQLITE_BUSY);
        assert_eq!(busy.code, "DB_BUSY");
        assert!(
            busy.message.starts_with(
                "The SQLite file is locked (SQLITE_BUSY): another connection holds its write lock"
            ),
            "{}",
            busy.message
        );
        assert!(
            busy.message.ends_with("ntnt waits up to its 2-second busy timeout for such a lock, and SQLite reports some conflicts at once, without waiting."),
            "{}",
            busy.message
        );
        assert!(busy
            .next_action
            .starts_with("Retry once the writer finishes"));
        assert!(!busy.next_action.contains("waited"), "{}", busy.next_action);
        let locked = error(rusqlite::ffi::SQLITE_LOCKED);
        assert_eq!(locked.code, "DB_BUSY");
        assert_eq!(
            locked.message,
            "The SQLite file reported a lock conflict within ntnt's own connection to it (SQLITE_LOCKED), not a lock held by another connection."
        );
        assert!(
            locked
                .next_action
                .starts_with("SQLITE_LOCKED is a conflict inside ntnt's own connection"),
            "{}",
            locked.next_action
        );
        assert!(!locked.next_action.contains("writer finishes"));
    }
}
