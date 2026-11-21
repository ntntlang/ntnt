//! Dedicated SQLite sessions with native authorization and bounded worker I/O.
//! No pool, extension loading, implicit file creation or target repair.
use super::adapter::Access;
use super::inventory::{json_digest, Migration};
use super::model::{Catalog, History, HistoryEntry};
use super::sqlite_file::FileBinding;
use super::{Diagnostic, Result};
use rusqlite::{
    config::DbConfig,
    fallible_iterator::FallibleIterator,
    hooks::{AuthAction, AuthContext, Authorization},
    types::ValueRef,
    Connection, OpenFlags,
};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

const IO_TIMEOUT: Duration = Duration::from_secs(5);
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

fn sql_error(error: rusqlite::Error) -> Diagnostic {
    // An error that points at a token of the input SQL carries its SQLite code
    // separately from an ordinary failure; classify both by that code.
    let sqlite = match &error {
        rusqlite::Error::SqliteFailure(error, _) | rusqlite::Error::SqlInputError { error, .. } => {
            Some(*error)
        }
        _ => None,
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
    Diagnostic::new(
        code,
        &format!(
            "SQLite operation failed (code {}). Details suppressed.",
            sqlite.map(|e| e.extended_code).unwrap_or(-1)
        ),
    )
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
    let allowed = match ctx.action {
        Select => true,
        Read { table_name, .. } => {
            (main && ordinary(table_name) && tables.contains(&table_name.to_ascii_lowercase()))
                || schema(table_name)
        }
        Function { function_name } => match function_name.to_ascii_lowercase().as_str() {
            "count" => true,
            "sqlite_rename_table"
            | "sqlite_rename_column"
            | "sqlite_rename_test"
            | "sqlite_rename_quotefix"
            | "sqlite_drop_column"
            | "like"
            | "substr"
            | "length"
            | "printf"
            | "quote"
            | "coalesce"
            | "ifnull"
            | "lower"
            | "upper"
            | "trim"
            | "abs"
            | "typeof"
            // Date and time functions: a column DEFAULT stores the expression as
            // text, so `CURRENT_TIMESTAMP` or `datetime('now')` leaves the catalog
            // deterministic; SQLite itself refuses them where determinism is
            // required (CHECK, generated columns, index expressions and predicates).
            | "date"
            | "time"
            | "datetime"
            | "julianday"
            | "unixepoch"
            | "strftime"
            | "timediff"
            | "current_date"
            | "current_time"
            | "current_timestamp" => write,
            _ => false,
        },
        CreateTable { table_name } => {
            if schema_access && main && ordinary(table_name) {
                tables.insert(table_name.to_ascii_lowercase());
                true
            } else {
                false
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
                && ordinary(table_name)
                && tables.contains(&table_name.to_ascii_lowercase())
        }
        DropView { view_name } => {
            schema_access
                && main
                && ordinary(view_name)
                && tables.contains(&view_name.to_ascii_lowercase())
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
                    || schema(table_name))
        }
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
        _ => "an operation".into(),
    }
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
fn execute_script(state: &State, sql: &str) -> Result<()> {
    let conn = &state.conn;
    let mut batch = rusqlite::Batch::new(conn, sql);
    let mut statement = 0usize;
    loop {
        let active = state.auth.activate(conn, Policy::Migration)?;
        statement += 1;
        let number = statement;
        let result = (|| {
            let Some(mut statement) = batch.next().map_err(sql_error)? else {
                return Ok(false);
            };
            if statement.parameter_count() != 0 {
                return Err(Diagnostic::new(
                    "DB_SQL_UNSUPPORTED",
                    "Migration SQL cannot contain uncaptured bind parameters.",
                ));
            }
            let dropped = state.auth.take_dropped();
            if !dropped.is_empty() {
                refuse_on_delete_drops(state, number, &dropped)?;
            }
            let mut output = statement.raw_query();
            while output.next().map_err(sql_error)?.is_some() {}
            Ok(true)
        })();
        drop(active);
        match result {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            // SQLite reports a refused function as a generic error, so the
            // authorizer's record of what it refused decides the diagnostic.
            Err(mut error) => {
                return Err(match state.auth.take_denial() {
                    Some(refused) => Diagnostic::new(
                        "DB_SQL_UNSUPPORTED",
                        &format!(
                        "Statement {statement} uses {refused}, which SQLite migrations cannot use."
                    ),
                    ),
                    // This refusal already names its statement.
                    None if error.code == "DB_REBUILD_REQUIRED" => error,
                    None => {
                        error.message = format!("Statement {statement}: {}", error.message);
                        error
                    }
                });
            }
        }
    }
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
    Err(Diagnostic::new(
        "DB_REBUILD_REQUIRED",
        &format!(
            "Statement {number} drops table `{table}`, which {named} reference{s}. SQLite deletes every row of a dropped table first, which would run those actions.",
            s = if children.len() == 1 { "s" } else { "" }
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

struct State {
    // Drop SQLite before the Windows pathname guard.
    conn: Connection,
    file: FileBinding,
    writable: bool,
    auth: Authorizer,
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
    fn catalog(&self) -> Result<Catalog> {
        self.file.check_connection(&self.conn)?;
        self.history(None)?;
        let mut catalog = Catalog::default();
        let mut programs = Vec::new();
        catalog.objects.insert(
            "schema:main".into(),
            json!({"engine":"sqlite","scope":"entire_main_file"}),
        );
        let table_info = rows(
            &self.conn,
            "SELECT name,type,wr,strict FROM pragma_table_list WHERE schema='main'",
            &[],
        )?;
        let objects=rows(&self.conn,"SELECT type,name,tbl_name,CASE WHEN length(sql)<=1048576 THEN sql ELSE NULL END,coalesce(length(sql),0)<=1048576 AND length(name)<=255 AND length(tbl_name)<=255 FROM main.sqlite_schema ORDER BY type,name LIMIT 10001",&[])?;
        for object in objects {
            let kind = object[0]
                .as_str()
                .ok_or_else(|| Diagnostic::new("DB_SQLITE", "Invalid catalog kind."))?;
            let name = object[1].as_str().unwrap();
            let table = object[2].as_str().unwrap();
            if reserved(name) || reserved(table) {
                continue;
            }
            if object[4] != 1 {
                return Err(Diagnostic::new(
                    "DB_CATALOG_LIMIT",
                    "SQLite definition or object name exceeds inspection bounds.",
                ));
            }
            let key = format!("{kind}:{}", quote(name));
            let mut item = json!({"table":table,"definition":{"sha256":json_digest(&object[3]),"redacted":true}});
            let mut supported = !internal(name) || name.starts_with("sqlite_autoindex_");
            match kind {
                "table" => {
                    let detail = table_info.iter().find(|r| r[0] == name).ok_or_else(|| {
                        Diagnostic::new("DB_CATALOG_AMBIGUOUS", "Missing SQLite table metadata.")
                    })?;
                    supported &= detail[1] == "table";
                    item["without_rowid"] = detail[2].clone();
                    item["strict"] = detail[3].clone();
                    let columns=rows(&self.conn,"SELECT cid,name,type,\"notnull\",dflt_value,pk,hidden FROM pragma_table_xinfo(?1,'main')",&[&name])?;
                    let assignment = columns
                        .iter()
                        .map(|column| {
                            let name = column[1].as_str().unwrap();
                            format!("{}={}", quote(name), quote(name))
                        })
                        .collect::<Vec<_>>()
                        .join(",");
                    for query in [
                        format!("EXPLAIN INSERT INTO main.{} DEFAULT VALUES", quote(name)),
                        format!("EXPLAIN UPDATE main.{} SET {assignment}", quote(name)),
                        format!("EXPLAIN DELETE FROM main.{}", quote(name)),
                    ] {
                        programs.push((key.clone(), query));
                    }
                    if detail[2] == 0 {
                        // UPDATE OF can name SQLite's hidden row identifiers,
                        // absent from table_xinfo. Each spelling has its own
                        // trigger event match, even when aliases share storage.
                        // WITHOUT ROWID tables have only the declared columns.
                        for alias in ["rowid", "_rowid_", "oid"] {
                            programs.push((
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
                    for mut column in columns {
                        let column_name = column[1].as_str().unwrap().to_owned();
                        if column_name.len() > 255
                            || column[2].as_str().is_none_or(|t| t.len() > 255)
                        {
                            return Err(Diagnostic::new(
                                "DB_CATALOG_LIMIT",
                                "SQLite column metadata exceeds inspection bounds.",
                            ));
                        }
                        supported &= column[6] == 0;
                        if !column[4].is_null() {
                            column[4] = json!({"sha256":json_digest(&column[4]),"redacted":true});
                        }
                        catalog.objects.insert(
                            format!("column:{}.{}", quote(name), quote(&column_name)),
                            json!(column),
                        );
                    }
                    item["foreign_keys"] = json!(rows(
                        &self.conn,
                        r#"SELECT id,seq,"table","from","to",on_update,on_delete,"match" FROM pragma_foreign_key_list(?1,'main') ORDER BY id,seq"#,
                        &[&name]
                    )?);
                    supported &= item["foreign_keys"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .all(|fk| fk[2].as_str().is_some_and(|n| !reserved(n) && !internal(n)));
                }
                "index" => {
                    // A key column is a table column (cid >= 0), the rowid
                    // (-1) or an expression (-2). Expressions and a partial
                    // index's WHERE predicate are in the definition hash.
                    // SQLite resolves them when it loads the schema, without
                    // the authorizer, so the function allowlist applies when
                    // a migration's CREATE INDEX compiles; a target index
                    // that differs from the replayed one is drift. The
                    // table's DML programs below still compile them.
                    let columns=rows(&self.conn,"SELECT seqno,cid,name,\"desc\",coll,key FROM pragma_index_xinfo(?1,'main') ORDER BY seqno",&[&name])?;
                    supported &= columns.iter().all(|r| {
                        r[1].as_i64().is_some_and(|v| v >= -2)
                            && r[4]
                                .as_str()
                                .is_none_or(|v| matches!(v, "BINARY" | "NOCASE" | "RTRIM"))
                    });
                    item["columns"] = json!(columns);
                    item["properties"]=json!(rows(&self.conn,"SELECT \"unique\",origin,partial FROM pragma_index_list(?1,'main') WHERE name=?2",&[&table,&name])?);
                    supported &= item["properties"].as_array().is_some_and(|p| p.len() == 1);
                }
                "view" => programs.push((
                    key.clone(),
                    format!("EXPLAIN SELECT * FROM main.{}", quote(name)),
                )),
                "trigger" => {
                    supported &= table_info.iter().any(|r| r[0] == table && r[1] == "table")
                }
                _ => supported = false,
            }
            if !supported {
                catalog.uncovered.push(key.clone());
            }
            catalog.objects.insert(key, item);
            if catalog.objects.len() > 10000 {
                return Err(Diagnostic::new(
                    "DB_CATALOG_LIMIT",
                    "SQLite catalog exceeds 10,000 objects.",
                ));
            }
        }
        if catalog.uncovered.is_empty() {
            // Compile only: never step an application query during inspection.
            // Native authorization covers trigger/view/default programs as well
            // as their resolved base relations; SQL definitions remain opaque.
            for (key, sql) in programs {
                if authored(self, Policy::Definition, |conn| {
                    conn.prepare(&sql).map_err(sql_error)?;
                    Ok(())
                })
                .is_err()
                {
                    catalog.uncovered.push(key);
                }
            }
            catalog.uncovered.sort();
            catalog.uncovered.dedup();
        }
        Ok(catalog)
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

/// A narrow, explicit mode for rebuilding one existing table. It may change that
/// table's column/constraint definition, but not silently retire other tables or
/// named dependent objects. Row counts are a guard, not proof of business meaning.
struct RebuildGuard {
    required: std::collections::BTreeMap<String, Value>,
    counts: std::collections::BTreeMap<String, i64>,
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
    fn capture(state: &State, table: &str) -> Result<Self> {
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
            })?;
        let table_key = format!("table:{}", quote(selected));
        let column_prefix = format!("column:{}.", quote(selected));
        let required = catalog
            .objects
            .into_iter()
            .filter(|(key, _)| {
                key != &table_key
                    && !key.starts_with(&column_prefix)
                    && !key.starts_with("index:\"sqlite_autoindex_")
            })
            .collect();
        Ok(Self { required, counts })
    }
    fn verify(self, state: &State) -> Result<()> {
        let catalog = state.catalog()?;
        if !catalog.uncovered.is_empty()
            || self
                .required
                .iter()
                .any(|(key, value)| catalog.objects.get(key) != Some(value))
            || self.counts != Self::counts(state)?
        {
            return Err(Diagnostic::new("DB_REBUILD_GUARD","SQLite rebuild must preserve existing table row counts, other table definitions, and named indexes/triggers/views. Split deliberate retirements into separately reviewed migrations; row values suppressed."));
        }
        Ok(())
    }
}

pub(super) struct Sqlite {
    state: Arc<Mutex<State>>,
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
        let state=tokio::time::timeout(IO_TIMEOUT,tokio::task::spawn_blocking(move || {
            if rusqlite::version_number()/1000!=3054 {return Err(Diagnostic::new("DB_ENGINE_VERSION","SQLite adapter supports the bundled SQLite 3.54 series."));}
            let file=FileBinding::capture(&path)?;
            let writable=!matches!(access,Access::Inspect);
            match &access {
                Access::Execute(expected) if expected["file_id"]!=file.id()=>return Err(Diagnostic::new("DB_WRONG_TARGET","SQLite file identity differs from the reviewed target; no writable connection opened.")),
                Access::Scratch(target) if target["file_id"]==file.id()=>return Err(Diagnostic::new("DB_SCRATCH_COLLISION","Scratch resolves to the selected deployment file.")),
                _=>(),
            }
            let flags=if writable {OpenFlags::SQLITE_OPEN_READ_WRITE} else {OpenFlags::SQLITE_OPEN_READ_ONLY};
            let mut conn=Connection::open_with_flags(file.path(),flags|OpenFlags::SQLITE_OPEN_NO_MUTEX).map_err(sql_error)?;
            file.check_connection(&conn)?;
            conn.busy_timeout(Duration::from_secs(2)).map_err(sql_error)?;
            conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_SQL_LENGTH,1_048_576).map_err(sql_error)?;
            conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_LENGTH,16*1_048_576).map_err(sql_error)?;
            conn.set_limit(rusqlite::limits::Limit::SQLITE_LIMIT_EXPR_DEPTH,64).map_err(sql_error)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DEFENSIVE,true).map_err(sql_error)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DDL,false).map_err(sql_error)?;
            conn.set_db_config(DbConfig::SQLITE_DBCONFIG_DQS_DML,false).map_err(sql_error)?;
            conn.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA ignore_check_constraints=OFF; PRAGMA legacy_alter_table=OFF; PRAGMA synchronous=FULL;").map_err(sql_error)?;
            let auth=Authorizer::install(&mut conn)?;
            Ok::<_,Diagnostic>(State{conn,file,writable,auth})
        })).await.map_err(|_|Diagnostic::new("DB_TIMEOUT","SQLite connection deadline exceeded."))?
            .map_err(|_|Diagnostic::new("DB_RUNTIME","SQLite connection worker failed."))??;
        let interrupt = state.conn.get_interrupt_handle();
        Ok(Self {
            state: Arc::new(Mutex::new(state)),
            interrupted: Arc::new(AtomicBool::new(false)),
            interrupt,
        })
    }
    async fn run<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&mut State) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let state = self.state.clone();
        let cancelled = self.interrupted.clone();
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
            let deadline = Instant::now() + IO_TIMEOUT;
            state
                .conn
                .progress_handler(
                    1000,
                    Some(move || cancelled.load(Ordering::Acquire) || Instant::now() >= deadline),
                )
                .map_err(sql_error)?;
            operation(&mut state)
        });
        match tokio::time::timeout(IO_TIMEOUT, work).await {
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
            s.conn.execute_batch("BEGIN IMMEDIATE").map_err(sql_error)
        })
        .await
    }
    /// Roll back the open transaction. Without one (a scratch refused before
    /// `BEGIN IMMEDIATE`) there is nothing to undo, which is not a failure.
    pub async fn rollback(&self) -> Result<()> {
        self.run(|s| {
            if s.conn.is_autocommit() {
                return Ok(());
            }
            s.conn.execute_batch("ROLLBACK").map_err(sql_error)
        })
        .await
    }
    pub async fn commit(&self) -> Result<()> {
        self.run(|s| s.conn.execute_batch("COMMIT").map_err(sql_error))
            .await
    }
    pub async fn catalog(&self) -> Result<Catalog> {
        self.run(|s| s.catalog()).await
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
                return Err(Diagnostic::new(
                    "DB_SCRATCH_MISMATCH",
                    &format!(
                        "SQLite scratch settings differ from the deployment file: {}.",
                        differences(&target, &identity)
                    ),
                ));
            }
            s.conn.execute_batch("BEGIN IMMEDIATE").map_err(sql_error)?;
            if !empty(s)? {
                return Err(not_empty());
            }
            Ok(identity)
        })
        .await
    }
    pub async fn execute_captured(&self, migration: &Migration) -> Result<()> {
        let source = migration.name.clone();
        let migration = migration.clone();
        self.run(move |s| {
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
                return Err(Diagnostic::new(
                    "DB_REPLAY_UNSUPPORTED",
                    "Migration is outside the supported SQLite subset.",
                ));
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
            let outcome = (|| {
                execute_script(s, &migration.sql)?;
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
            if !s.catalog()?.uncovered.is_empty() {
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
