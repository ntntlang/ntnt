//! SQLite migration statement kinds. These are policy and review labels, not
//! a syntax parser. Replay and migrate prepare the original bytes with SQLite.
//! Source-only reports and saved-plan validation need the same labels without
//! a live schema: their proposed statement spans are checked against SQLite's
//! own prepare/tail boundaries before the prepared programs can run.
use super::analysis::Analysis;
use super::Diagnostic;

/// Significant source tokens for identifying the statement *kind*. Keep
/// quoted text opaque; SQLite doubles matching quotes, does not nest block
/// comments, and recognizes a BOM as whitespace at the start of a token.
#[derive(Clone, Copy)]
struct Tokens<'a>(&'a str);

fn word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$') || b >= 0x80
}

impl<'a> Iterator for Tokens<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let sql = self.0;
            let bytes = sql.as_bytes();
            let &first = bytes.first()?;
            let trivia = if first.is_ascii_whitespace() {
                1
            } else if sql.starts_with('\u{feff}') {
                3
            } else if sql.starts_with("--") {
                sql.find('\n').unwrap_or(sql.len())
            } else if let Some(comment) = sql.strip_prefix("/*").filter(|s| !s.is_empty()) {
                comment.find("*/").map_or(sql.len(), |n| n + 4)
            } else {
                0
            };
            if trivia != 0 {
                self.0 = &sql[trivia..];
                continue;
            }
            let len = match first {
                b'\'' | b'"' | b'`' => {
                    let mut i = 1;
                    while i < bytes.len() {
                        let b = bytes[i];
                        i += 1;
                        if b == first {
                            if bytes.get(i) != Some(&first) {
                                break;
                            }
                            i += 1;
                        }
                    }
                    i
                }
                b'[' => sql.find(']').map_or(sql.len(), |n| n + 1),
                b if word_byte(b) => bytes.iter().take_while(|b| word_byte(**b)).count(),
                _ => 1,
            };
            self.0 = &sql[len..];
            return Some(&sql[..len]);
        }
    }
}

impl<'a> Tokens<'a> {
    fn eat(&mut self, keyword: &str) -> bool {
        let mut probe = *self;
        if probe
            .next()
            .is_some_and(|t| t.eq_ignore_ascii_case(keyword))
        {
            *self = probe;
            true
        } else {
            false
        }
    }

    fn peek(mut self) -> Option<&'a str> {
        self.next()
    }

    fn name(&mut self) -> Option<&'a str> {
        let first = self.next()?;
        if self.eat(".") {
            self.next()
        } else {
            Some(first)
        }
    }
}

/// Skip leading trivia and empty statements (which prepare does not yield).
fn content(sql: &str) -> &str {
    let mut tokens = Tokens(sql);
    match tokens.find(|&t| t != ";") {
        Some(first) => &sql[sql.len() - tokens.0.len() - first.len()..],
        None => "",
    }
}

/// Source projection only. A CREATE TRIGGER ends at `; END ;` rather than
/// the first semicolon (the sqlite3_complete rule). Neither this projection
/// nor a matching header on its own admits SQL to execution.
pub(super) fn statements(mut rest: &str) -> impl Iterator<Item = &str> {
    std::iter::from_fn(move || {
        let sql = content(rest);
        if sql.is_empty() {
            return None;
        }
        let mut prefix = Tokens(sql);
        let create = prefix.eat("CREATE");
        let _ = prefix.eat("TEMP") || prefix.eat("TEMPORARY");
        let trigger = create && prefix.eat("TRIGGER");
        let mut tokens = Tokens(sql);
        let (mut semi, mut end) = (false, false);
        for t in tokens.by_ref() {
            if t == ";" && (!trigger || end) {
                break;
            }
            end = semi && t.eq_ignore_ascii_case("END");
            semi = t == ";";
        }
        rest = tokens.0;
        Some(&sql[..sql.len() - rest.len()])
    })
}

#[derive(Clone, Copy)]
enum Kind {
    Table,
    Index,
    View,
    Trigger,
    DropTrigger,
    Analyze,
    Write,
    Drop,
    Alter(Option<&'static str>),
}

impl Kind {
    fn labels(self) -> (&'static str, &'static str) {
        match self {
            Self::Table => ("create_table", "DDL_LOCKS"),
            Self::Index => ("create_index", "INDEX_BUILD_LOCK_SCAN"),
            Self::View => ("create_view", "VIEW_DEPENDENCIES"),
            Self::Trigger => ("create_trigger", "TRIGGER_EFFECTS"),
            Self::DropTrigger => ("drop_trigger", "REMOVED_TRIGGER"),
            Self::Analyze => ("analyze", "STATISTICS_SCAN"),
            Self::Write => ("data_write", "DESTRUCTIVE_OR_OVERWRITING_DML"),
            Self::Drop => ("drop", "DESTRUCTIVE_DROP"),
            Self::Alter(_) => ("alter_table", "EXCLUSIVE_LOCK_AND_POSSIBLE_SCAN"),
        }
    }
}

enum Refused<'a> {
    Pragma(&'a str),
    Temporary,
    ReservedRename,
    Unsupported(&'static str),
}

fn write(t: &str) -> bool {
    ["INSERT", "REPLACE", "UPDATE", "DELETE"]
        .iter()
        .any(|k| t.eq_ignore_ascii_case(k))
}

fn classify(sql: &str) -> Result<Kind, Refused<'_>> {
    use Refused::Unsupported;
    let mut t = Tokens(sql);
    if t.eat("CREATE") {
        if t.eat("TEMP") || t.eat("TEMPORARY") {
            return Err(Refused::Temporary);
        }
        if t.eat("TABLE") {
            if t.eat("IF") && !(t.eat("NOT") && t.eat("EXISTS")) {
                return Err(Unsupported(""));
            }
            t.name();
            // Only a column-list declaration, never CTAS: CREATE authorizes
            // the new table name while compiling the same statement.
            return if t.eat("(") {
                Ok(Kind::Table)
            } else if t.eat("AS") {
                Err(Unsupported("CREATE TABLE ... AS SELECT"))
            } else {
                Err(Unsupported(""))
            };
        }
        if t.eat("VIEW") {
            return Ok(Kind::View);
        }
        if t.eat("TRIGGER") {
            return Ok(Kind::Trigger);
        }
        if t.eat("VIRTUAL") {
            return Err(Unsupported("CREATE VIRTUAL TABLE"));
        }
        t.eat("UNIQUE");
        if t.eat("INDEX") {
            return Ok(Kind::Index);
        }
    } else if t.eat("ALTER") && t.eat("TABLE") {
        t.name();
        // SQLite's ALTER authorization names the old table, not the new
        // name. Do not let a rename manufacture an ntnt ledger object.
        if t.eat("RENAME")
            && t.eat("TO")
            && t.next()
                .and_then(unquote)
                .is_some_and(|name| super::sqlite::reserved(&name))
        {
            return Err(Refused::ReservedRename);
        }
        let extra = if t.eat("DROP") {
            Some(if t.eat("CONSTRAINT") {
                "REMOVED_CONSTRAINT"
            } else {
                "DESTRUCTIVE_DROP_COLUMN"
            })
        } else if t.eat("ALTER") {
            t.eat("COLUMN");
            t.next();
            t.eat("DROP").then_some("REMOVED_CONSTRAINT")
        } else {
            None
        };
        return Ok(Kind::Alter(extra));
    } else if t.eat("DROP") {
        if t.eat("TRIGGER") {
            return Ok(Kind::DropTrigger);
        }
        if t.eat("TABLE") || t.eat("VIEW") || t.eat("INDEX") {
            return Ok(Kind::Drop);
        }
    } else if t.eat("ANALYZE") {
        return Ok(Kind::Analyze);
    } else if t.eat("WITH") {
        // A CTE body ends at its matching ')'. After the final body the
        // next token is the main verb; do not take a word in a SELECT,
        // quoted text or a nested expression for an authored write.
        let mut depth = 0usize;
        while let Some(token) = t.next() {
            match token {
                "(" => depth += 1,
                ")" if depth > 0 => {
                    depth -= 1;
                    if depth == 0 && t.peek().is_some_and(write) {
                        return Ok(Kind::Write);
                    }
                    if depth == 0 && (t.eat("SELECT") || t.eat("VALUES")) {
                        return Err(Unsupported("WITH"));
                    }
                }
                _ => (),
            }
        }
        return Err(Unsupported(""));
    } else if t.eat("PRAGMA") {
        return Err(Refused::Pragma(t.name().unwrap_or("")));
    } else if let Some(first) = Tokens(sql).next() {
        if write(first) {
            return Ok(Kind::Write);
        }
        for keyword in [
            "BEGIN",
            "COMMIT",
            "END",
            "ROLLBACK",
            "SAVEPOINT",
            "RELEASE",
            "ATTACH",
            "DETACH",
            "VACUUM",
            "REINDEX",
            "EXPLAIN",
            "SELECT",
            "VALUES",
        ] {
            if first.eq_ignore_ascii_case(keyword) {
                return Err(Unsupported(keyword));
            }
        }
    }
    Err(Unsupported(""))
}

pub(super) fn analyze(a: &mut Analysis, sql: &str) {
    for stmt in statements(sql) {
        match classify(stmt) {
            Ok(kind) => {
                let (operation, hazard) = kind.labels();
                a.operations.push(operation.into());
                a.hazards.push(hazard.into());
                if let Kind::Alter(Some(extra)) = kind {
                    a.hazards.push(extra.into());
                }
            }
            Err(_) => {
                a.operations.push("unsupported".into());
                a.hazards.push("UNANALYZED_EFFECTS".into());
                a.replay_supported = false;
                a.unknowns.push("SQL_UNSUPPORTED".into());
                break;
            }
        }
    }
    if a.operations.is_empty() {
        a.replay_supported = false;
        a.unknowns.push("SQL_PARSE_EMPTY_OR_LIMIT".into());
    } else {
        a.unknowns.push("SQLITE_EFFECTS_NOT_INFERRED".into());
    }
    if sql.contains('\0') {
        a.replay_supported = false;
    }
}

impl Refused<'_> {
    fn diagnostic(self, statement: usize) -> Diagnostic {
        let (message, action) = match self {
            Self::Pragma(name) => {
                let name = if name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    && name.len() <= 64
                {
                    name
                } else {
                    "(name suppressed)"
                };
                (format!("Statement {statement} is PRAGMA {name}; migrations cannot run PRAGMAs."), "Remove the PRAGMA from the migration, then rerun plan. ntnt db sets foreign_keys and recursive_triggers on its own connections, and file settings such as journal_mode belong to the application's connection, not to a migration.".to_owned())
            }
            Self::ReservedRename => {
                return super::sqlite::denied_statement(
                    statement,
                    "a table name reserved for ntnt",
                );
            }
            Self::Temporary => {
                return super::sqlite::denied_statement(statement, "a temporary object");
            }
            Self::Unsupported(construct) => {
                let construct = if construct.is_empty() {
                    String::new()
                } else {
                    format!(" (`{construct}`)")
                };
                (format!("Statement {statement}{construct} is outside the supported SQLite subset."),format!("Rewrite statement {statement} with the statements SQLite migrations support: CREATE, ALTER and DROP TABLE; CREATE and DROP INDEX, VIEW and TRIGGER; INSERT, UPDATE and DELETE (write CREATE TABLE ... AS SELECT as CREATE TABLE, then INSERT ... SELECT); and ANALYZE, as docs/DATABASE_MIGRATIONS.md describes under Supported SQL and catalog coverage. Then rerun plan."))
            }
        };
        let mut d = Diagnostic::new("DB_REPLAY_UNSUPPORTED", &message);
        d.next_action = action;
        d
    }
}

/// Catch forbidden headers before prepare, too: some PRAGMAs take effect
/// during compilation. Unknown/malformed headers get SQLite's syntax error
/// if they have one; no successfully prepared unknown kind can be stepped.
pub(super) fn before_prepare(sql: &str, number: usize) -> Result<(), Diagnostic> {
    match classify(sql) {
        Err(Refused::Unsupported("")) | Ok(_) => Ok(()),
        Err(refused) => Err(refused.diagnostic(number)),
    }
}

pub(super) fn check_prepared(
    stmt: &rusqlite::Statement<'_>,
    projected: Option<&str>,
    number: usize,
) -> Result<(), Diagnostic> {
    // Bind parameters are already refused, so expanded_sql is the original
    // text of this single prepared statement, up to SQLite's native tail.
    let sql = stmt.expanded_sql().ok_or_else(|| {
        Diagnostic::new(
            "DB_SQL_UNSUPPORTED",
            "SQLite could not supply the prepared migration statement.",
        )
    })?;
    let sql = content(&sql);
    if Some(sql) != projected {
        return Err(boundary_error(number));
    }
    classify(sql).map(|_| ()).map_err(|e| e.diagnostic(number))
}

pub(super) fn boundary_error(number: usize) -> Diagnostic {
    Diagnostic::new("DB_SQL_UNSUPPORTED", &format!("Statement {number}: the reviewed migration statements do not match SQLite's prepared statements."))
}

/// Only show a name or punctuation at SQLite's byte offset. Never format
/// SqlInputError itself: it contains the complete remaining migration SQL.
pub(super) fn syntax_error(error: &rusqlite::Error, number: usize) -> Option<Diagnostic> {
    use rusqlite::Error::*;
    let near = match error {
        SqlInputError {
            error,
            msg,
            sql,
            offset,
        } if error.extended_code == rusqlite::ffi::SQLITE_ERROR
            && (msg.ends_with(": syntax error") || msg.starts_with("unrecognized token:")) =>
        {
            let token = Tokens(sql.get(usize::try_from(*offset).ok()?..)?).next()?;
            let show = token.len() <= 64
                && (token.as_bytes()[0].is_ascii_alphabetic()
                    && token
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    || matches!(
                        token,
                        ";" | ","
                            | "("
                            | ")"
                            | "."
                            | "="
                            | "*"
                            | "/"
                            | "-"
                            | "+"
                            | "%"
                            | "<"
                            | ">"
                            | "|"
                            | "&"
                            | ":"
                    ));
            if show {
                format!(" near `{token}`")
            } else {
                " near a literal".into()
            }
        }
        SqliteFailure(error, Some(msg))
            if error.extended_code == rusqlite::ffi::SQLITE_ERROR
                && (msg == "incomplete input" || msg.starts_with("unrecognized token:")) =>
        {
            String::new()
        }
        _ => return None,
    };
    let mut d = Diagnostic::new(
        "DB_REPLAY_UNSUPPORTED",
        &format!("Statement {number} has a syntax error{near}."),
    );
    d.next_action = format!("Correct statement {number} in the migration named in source, then rerun plan. Nothing from this migration was committed.");
    Some(d)
}

// These helpers inspect catalog or already captured authored SQL. They share
// the same quoted-token rules as statement classification.
fn unquote(token: &str) -> Option<String> {
    let first = token.as_bytes().first().copied()?;
    if matches!(first, b'\'' | b'"' | b'`' | b'[') {
        let end = if first == b'[' { b']' } else { first };
        let body = token[1..].strip_suffix(char::from(end))?;
        return Some(if first == b'[' {
            body.to_owned()
        } else {
            let quote = char::from(first);
            body.replace(&format!("{quote}{quote}"), &quote.to_string())
        });
    }
    Some(token.to_owned())
}

pub(super) fn autoincrement(sql: &str) -> bool {
    Tokens(sql).any(|t| t.eq_ignore_ascii_case("AUTOINCREMENT"))
}

pub(super) fn renamed_to(sql: &str, table: &str) -> Option<String> {
    statements(sql)
        .filter_map(|stmt| {
            let mut t = Tokens(stmt);
            if !(t.eat("ALTER") && t.eat("TABLE")) {
                return None;
            }
            let from = t.name()?;
            if !(t.eat("RENAME") && t.eat("TO")) {
                return None;
            }
            unquote(t.next()?)?
                .eq_ignore_ascii_case(table)
                .then(|| unquote(from))
                .flatten()
        })
        .last()
}

pub(super) fn raise_messages(sql: &str) -> Vec<String> {
    let mut messages = vec![];
    let mut tokens = Tokens(sql);
    while let Some(token) = tokens.next() {
        if token.eq_ignore_ascii_case("RAISE") {
            let mut t = tokens;
            if t.eat("(") && (t.eat("ABORT") || t.eat("FAIL") || t.eat("ROLLBACK")) && t.eat(",") {
                if let Some(literal) = t.next().filter(|s| s.starts_with('\'')) {
                    if t.eat(")") {
                        messages.extend(unquote(literal));
                    }
                }
            }
        }
    }
    messages
}

pub(super) fn created_raise_messages(sql: &str, count: usize) -> Vec<String> {
    statements(sql)
        .take(count)
        .filter(|stmt| matches!(classify(stmt), Ok(Kind::Trigger)))
        .flat_map(raise_messages)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::fallible_iterator::FallibleIterator;

    #[test]
    fn native_boundaries_match_source_review_labels() {
        let db = rusqlite::Connection::open_in_memory().unwrap();
        let sql = "\u{feff} ; -- header\n\
            CREATE TABLE [semi;colon] (k TEXT PRIMARY KEY, a TEXT CHECK(a GLOB 'a;*'), retired TEXT) STRICT, WITHOUT ROWID;\n\
            CREATE TRIGGER touch AFTER UPDATE ON [semi;colon] WHEN OLD.a IS NOT NEW.a BEGIN\n\
                SELECT CASE WHEN NEW.a IS NULL THEN 'ok; END;' ELSE 'it''s fine' END;\n\
                SELECT 2; /* ; END; */ END; ;\n\
            WITH x(k) AS NOT MATERIALIZED (VALUES ('a')), y AS (SELECT (k) FROM x) INSERT INTO [semi;colon](k) SELECT k FROM y;\n\
            ALTER TABLE [semi;colon] DROP COLUMN retired /* no trailing semicolon */";
        let mut projected = statements(sql);
        let mut batch = rusqlite::Batch::new(&db, sql);
        let mut ops = vec![];
        while let Some(mut stmt) = batch.next().unwrap() {
            let source = projected.next().unwrap();
            check_prepared(&stmt, Some(source), ops.len() + 1).unwrap();
            let kind = classify(source).ok().unwrap();
            ops.push(kind.labels().0);
            if let Kind::Alter(extra) = kind {
                assert_eq!(extra, Some("DESTRUCTIVE_DROP_COLUMN"));
            }
            stmt.raw_execute().unwrap();
        }
        assert!(projected.next().is_none());
        assert_eq!(
            ops,
            [
                "create_table",
                "create_trigger",
                "data_write",
                "alter_table"
            ]
        );
    }

    #[test]
    fn empty_comments_follow_sqlite() {
        for sql in ["-- comment\n ;", "/* unfinished comment", "/*"] {
            let db = rusqlite::Connection::open_in_memory().unwrap();
            let native_empty = matches!(rusqlite::Batch::new(&db, sql).next(), Ok(None));
            assert_eq!(statements(sql).next().is_none(), native_empty, "{sql}");
        }
    }

    /// The replacement a rebuild renames to the table, with its names
    /// quoted or not (review).
    #[test]
    fn sqlite_renamed_to_reads_single_quoted_names() {
        for sql in [
            "ALTER TABLE orders_new RENAME TO orders;",
            "ALTER TABLE orders_new RENAME TO 'orders';",
            "ALTER TABLE 'orders_new' RENAME TO \"orders\";",
            "ALTER TABLE main.'orders_new' RENAME TO 'orders';",
        ] {
            assert_eq!(
                renamed_to(sql, "orders").as_deref(),
                Some("orders_new"),
                "{sql}"
            );
        }
        assert_eq!(
            renamed_to("ALTER TABLE orders_new RENAME TO 'other';", "orders"),
            None
        );
        assert_eq!(
            renamed_to(
                "SELECT 'ALTER', 'TABLE', 'RENAME', 'TO', 'orders';",
                "orders"
            ),
            None
        );
    }
}
