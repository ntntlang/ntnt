//! PostgreSQL dialect parsing, not keyword scanning. Unknown syntax never replays.
use serde::Serialize;
use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Analysis {
    pub replay_supported: bool,
    pub operations: Vec<String>,
    pub hazards: Vec<String>,
    pub unknowns: Vec<String>,
    /// PostgreSQL only: table locks each statement takes, predicted from SQL.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locks: Vec<PredictedLock>,
    /// PostgreSQL only: locks that block or scan an existing table.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub lock_warnings: Vec<LockWarning>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct PredictedLock {
    /// 1-based statement number within the migration script.
    pub statement: usize,
    /// Last statement of a run of consecutive statements that take exactly
    /// the same locks (for example, many seed INSERTs). Omitted for one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub through: Option<usize>,
    pub relation: String,
    /// pg_locks mode name, as reported in migrate's progress[].locks.
    pub mode: &'static str,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct LockWarning {
    pub statement: usize,
    pub relation: String,
    pub code: &'static str,
    pub message: &'static str,
}
/// Parse PostgreSQL syntax within fixed token and recursion bounds.
/// Invalid, empty or unsupported input is not evidence that replay would be safe.
fn parse(sql: &str) -> Option<Vec<Statement>> {
    parse_with(sql, &PostgreSqlDialect {})
}
fn parse_with(sql: &str, dialect: &dyn sqlparser::dialect::Dialect) -> Option<Vec<Statement>> {
    let tokens = sqlparser::tokenizer::Tokenizer::new(dialect, sql)
        .tokenize_with_location()
        .ok()?;
    // Bound both parser work and recursive AST destruction on all CLI stacks.
    if tokens.len() > 4096 {
        return None;
    }
    Parser::new(dialect)
        .with_recursion_limit(64)
        .with_tokens_with_locations(tokens)
        .parse_statements()
        .ok()
}
use sqlparser::ast::*;

fn ident(i: &Ident) -> bool {
    !i.value.is_empty() && i.value.len() <= 63 && !i.value.contains('\0')
}
fn name(n: &ObjectName, schema: &str) -> bool {
    let ids: Option<Vec<_>> = n.0.iter().map(|p| p.as_ident()).collect();
    let Some(ids) = ids else { return false };
    ids.iter().all(|i| ident(i))
        && match ids.as_slice() {
            [_] => true,
            [s, _] => {
                if s.quote_style.is_some() {
                    s.value == schema
                } else {
                    s.value.to_lowercase() == schema
                }
            }
            _ => false,
        }
}
fn builtin(t: &DataType) -> bool {
    // Only PostgreSQL built-ins; no domains, custom input functions or casts.
    matches!(
        t,
        DataType::Int(_)
            | DataType::Integer(_)
            | DataType::SmallInt(_)
            | DataType::BigInt(_)
            | DataType::Int2(_)
            | DataType::Int4(_)
            | DataType::Int8(_)
            | DataType::Text
            | DataType::Bool
            | DataType::Boolean
            | DataType::Varchar(_)
            | DataType::CharacterVarying(_)
            | DataType::Numeric(_)
            | DataType::Decimal(_)
            | DataType::Real
            | DataType::DoublePrecision
            | DataType::Date
            | DataType::Timestamp(_, _)
            | DataType::Uuid
            | DataType::Bytea
    )
}
fn literal(e: &Expr) -> bool {
    match e {
        Expr::Value(v) => matches!(
            v.value,
            Value::Number(_, _) | Value::SingleQuotedString(_) | Value::Boolean(_) | Value::Null
        ),
        Expr::UnaryOp {
            op: UnaryOperator::Plus | UnaryOperator::Minus,
            expr,
        } => matches!(&**expr, Expr::Value(v) if matches!(v.value, Value::Number(_, _))),
        Expr::Nested(e) => literal(e),
        _ => false,
    }
}
fn expression(e: &Expr, depth: usize) -> bool {
    if depth > 32 {
        return false;
    }
    match e {
        e if literal(e) => true,
        Expr::Identifier(i) => ident(i),
        Expr::Nested(e) | Expr::IsNull(e) | Expr::IsNotNull(e) => expression(e, depth + 1),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr,
        } => expression(expr, depth + 1),
        Expr::BinaryOp { left, op, right } => {
            matches!(
                op,
                BinaryOperator::Eq
                    | BinaryOperator::NotEq
                    | BinaryOperator::Gt
                    | BinaryOperator::Lt
                    | BinaryOperator::GtEq
                    | BinaryOperator::LtEq
                    | BinaryOperator::And
                    | BinaryOperator::Or
                    | BinaryOperator::Plus
                    | BinaryOperator::Minus
                    | BinaryOperator::Multiply
                    | BinaryOperator::Divide
                    | BinaryOperator::Modulo
            ) && expression(left, depth + 1)
                && expression(right, depth + 1)
        }
        _ => false,
    }
}
fn index_columns(cs: &[IndexColumn]) -> bool {
    cs.iter().all(|c| {
        c.operator_class.is_none()
            && matches!(&c.column.expr, Expr::Identifier(i) if ident(i))
            && c.column.with_fill.is_none()
    })
}
fn constraint(c: &TableConstraint, schema: &str) -> bool {
    match c {
        TableConstraint::PrimaryKey(c) => {
            c.index_type.is_none() && c.index_options.is_empty() && index_columns(&c.columns)
        }
        TableConstraint::Unique(c) => {
            c.index_type.is_none() && c.index_options.is_empty() && index_columns(&c.columns)
        }
        TableConstraint::ForeignKey(c) => name(&c.foreign_table, schema),
        TableConstraint::Check(c) => expression(&c.expr, 0),
        _ => false,
    }
}
fn column(c: &ColumnDef, schema: &str) -> bool {
    ident(&c.name)
        && builtin(&c.data_type)
        && c.options.iter().all(|o| match &o.option {
            ColumnOption::Null | ColumnOption::NotNull => true,
            ColumnOption::Default(e) => literal(e),
            ColumnOption::PrimaryKey(c) => {
                constraint(&TableConstraint::PrimaryKey(c.clone()), schema)
            }
            ColumnOption::Unique(c) => constraint(&TableConstraint::Unique(c.clone()), schema),
            ColumnOption::ForeignKey(c) => {
                constraint(&TableConstraint::ForeignKey(c.clone()), schema)
            }
            ColumnOption::Check(c) => expression(&c.expr, 0),
            _ => false,
        })
}
/// The closed AST subset whose effects NTNT analyzes in detail. PostgreSQL
/// statements outside it still replay; they are reported as EFFECTS_NOT_ANALYZED.
fn allowed(statement: &Statement, schema: &str) -> bool {
    match statement {
        Statement::CreateTable(t)
            if name(&t.name, schema)
                && !t.columns.is_empty()
                && t.columns.iter().all(|c| column(c, schema))
                && t.constraints.iter().all(|c| constraint(c, schema)) =>
        {
            // Equality to the minimal parsed form rejects every unhandled table
            // option (AS, LIKE, access methods, inheritance, partitioning, etc.).
            let fields = t
                .columns
                .iter()
                .map(ToString::to_string)
                .chain(t.constraints.iter().map(ToString::to_string))
                .collect::<Vec<_>>()
                .join(", ");
            parse(&format!("CREATE TABLE {} ({fields})", t.name))
                .as_ref()
                .is_some_and(|s| s.as_slice() == std::slice::from_ref(statement))
        }
        Statement::AlterTable(t) => {
            name(&t.name, schema)
                && t.location.is_none()
                && t.on_cluster.is_none()
                && t.table_type.is_none()
                && !t.operations.is_empty()
                && t.operations.iter().all(|op| match op {
                    AlterTableOperation::AddColumn {
                        column_def,
                        column_position,
                        ..
                    } => column_position.is_none() && column(column_def, schema),
                    AlterTableOperation::AddConstraint { constraint: c, .. } => {
                        constraint(c, schema)
                    }
                    AlterTableOperation::DropColumn { .. }
                    | AlterTableOperation::DropConstraint { .. }
                    | AlterTableOperation::RenameColumn { .. } => true,
                    AlterTableOperation::AlterColumn { op, .. } => match op {
                        AlterColumnOperation::SetNotNull
                        | AlterColumnOperation::DropNotNull
                        | AlterColumnOperation::DropDefault => true,
                        AlterColumnOperation::SetDefault { value } => literal(value),
                        AlterColumnOperation::SetDataType {
                            data_type, using, ..
                        } => builtin(data_type) && using.is_none(),
                        _ => false,
                    },
                    _ => false,
                })
        }
        Statement::CreateIndex(i) => {
            i.name
                .as_ref()
                .is_some_and(|n| n.0.len() == 1 && name(n, schema))
                && name(&i.table_name, schema)
                && i.using.is_none()
                && !i.concurrently
                && index_columns(&i.columns)
                && i.predicate.is_none()
                && i.with.is_empty()
                && i.index_options.is_empty()
                && i.alter_options.is_empty()
        }
        Statement::Drop {
            object_type: ObjectType::Table | ObjectType::Index,
            names,
            purge,
            temporary,
            table,
            ..
        } => !purge && !temporary && table.is_none() && names.iter().all(|n| name(n, schema)),
        _ => false,
    }
}
/// Classify supported operations, known hazards and analysis gaps without executing SQL.
/// Unknown syntax remains explicitly unsupported rather than inheriting a safe verdict.
pub(crate) fn analyze(sql: &str, schema: &str) -> Analysis {
    analyze_for("postgresql", sql, schema)
}
pub(crate) fn analyze_for(engine: &str, sql: &str, schema: &str) -> Analysis {
    let mut a = Analysis {
        replay_supported: true,
        operations: vec![],
        hazards: vec![],
        unknowns: vec![],
        locks: vec![],
        lock_warnings: vec![],
    };
    if engine == "sqlite" {
        sqlite_script(&mut a, sql);
    } else {
        postgres_script(&mut a, sql, schema);
    }
    a.hazards.sort();
    a.hazards.dedup();
    a.unknowns.sort();
    a.unknowns.dedup();
    a
}
/// Record one parsed statement's operation and lock/effect hazard.
/// Returns the operation name, empty when the statement kind is not classified.
fn classify(a: &mut Analysis, s: &Statement, engine: &str) -> &'static str {
    let (operation, hazard) = match s {
        Statement::CreateTable(_) => ("create_table", "DDL_LOCKS"),
        Statement::CreateIndex(_) => ("create_index", "INDEX_BUILD_LOCK_SCAN"),
        Statement::CreateView { .. } if engine == "sqlite" => ("create_view", "VIEW_DEPENDENCIES"),
        Statement::CreateTrigger { .. } if engine == "sqlite" => {
            ("create_trigger", "TRIGGER_EFFECTS")
        }
        Statement::DropTrigger { .. } if engine == "sqlite" => ("drop_trigger", "REMOVED_TRIGGER"),
        Statement::AlterTable(t) => {
            for op in &t.operations {
                if matches!(
                    op,
                    AlterTableOperation::DropColumn {
                        drop_behavior: Some(DropBehavior::Cascade),
                        ..
                    } | AlterTableOperation::DropConstraint {
                        drop_behavior: Some(DropBehavior::Cascade),
                        ..
                    }
                ) {
                    a.hazards.push("CASCADE_DEPENDENCIES".into());
                }
                match op {
                    AlterTableOperation::DropColumn { .. } => {
                        a.hazards.push("DESTRUCTIVE_DROP_COLUMN".into())
                    }
                    AlterTableOperation::DropConstraint { .. } => {
                        a.hazards.push("REMOVED_CONSTRAINT".into())
                    }
                    AlterTableOperation::AlterColumn {
                        op: AlterColumnOperation::SetDataType { .. },
                        ..
                    } => a.hazards.push("CAST_DATA_LOSS_OR_REWRITE".into()),
                    AlterTableOperation::AlterColumn {
                        op: AlterColumnOperation::DropNotNull,
                        ..
                    } => a.hazards.push("REMOVED_CONSTRAINT".into()),
                    AlterTableOperation::EnableRowLevelSecurity
                    | AlterTableOperation::DisableRowLevelSecurity => {
                        a.hazards.push("AUTHORIZATION_CHANGE".into())
                    }
                    _ => (),
                }
            }
            ("alter_table", "EXCLUSIVE_LOCK_AND_POSSIBLE_SCAN")
        }
        Statement::Drop { cascade, .. } => {
            if *cascade {
                a.hazards.push("CASCADE_DEPENDENCIES".into());
            }
            ("drop", "DESTRUCTIVE_DROP")
        }
        Statement::Insert(_)
        | Statement::Update(_)
        | Statement::Delete(_)
        | Statement::Truncate { .. } => ("data_write", "DESTRUCTIVE_OR_OVERWRITING_DML"),
        Statement::Grant { .. } | Statement::Revoke { .. } => {
            ("authorization", "AUTHORIZATION_CHANGE")
        }
        _ => ("", "UNANALYZED_EFFECTS"),
    };
    a.hazards.push(hazard.into());
    operation
}
fn sqlite_script(a: &mut Analysis, sql: &str) {
    let Some(statements) =
        parse_with(sql, &sqlparser::dialect::SQLiteDialect {}).filter(|s| !s.is_empty())
    else {
        a.replay_supported = false;
        a.unknowns.push("SQL_PARSE_EMPTY_OR_LIMIT".into());
        return;
    };
    for s in statements {
        let operation = classify(a, &s, "sqlite");
        a.operations.push(
            if operation.is_empty() {
                "unsupported"
            } else {
                operation
            }
            .into(),
        );
        // SQLite's native authorizer, not PostgreSQL's AST rules, checks
        // resolved objects/functions before any authored statement executes.
        a.unknowns.push("SQLITE_EFFECTS_NOT_INFERRED".into());
        // CREATE authorizes references in its own CHECK/index definitions.
        // CTAS must not use that newly declared name to authorize an
        // eponymous virtual-table source during the same preparation. Keep
        // copies explicit: CREATE ordinary table, then INSERT ... SELECT.
        let supported = !matches!(&s, Statement::CreateTable(table) if table.query.is_some())
            && matches!(
                s,
                Statement::CreateTable(_)
                    | Statement::AlterTable(_)
                    | Statement::CreateIndex(_)
                    | Statement::CreateView { .. }
                    | Statement::CreateTrigger { .. }
                    | Statement::DropTrigger { .. }
                    | Statement::Insert(_)
                    | Statement::Update(_)
                    | Statement::Delete(_)
                    | Statement::Drop {
                        object_type: ObjectType::Table | ObjectType::Index | ObjectType::View,
                        ..
                    }
            );
        if !supported || a.hazards.iter().any(|h| h == "CASCADE_DEPENDENCIES") {
            a.replay_supported = false;
            a.unknowns.push("SQL_UNSUPPORTED".into());
        }
    }
}

/// Whole migration scripts are bounded by the 1 MiB input cap; this bounds
/// tokenizer output. Individual statements keep the 4,096-token parser bound.
const MAX_SCRIPT_TOKENS: usize = 262_144;

/// PostgreSQL migrations run as complete scripts inside one transaction: in
/// disposable scratch for review, then on the target after approval. Analysis
/// splits on tokenizer-level semicolons (quoted and dollar-quoted text stays
/// intact), rejects statements that would escape that transaction or session,
/// and reports which statements it could not analyze. It does not sandbox SQL.
fn postgres_script(a: &mut Analysis, sql: &str, schema: &str) {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let dialect = PostgreSqlDialect {};
    let tokens = match Tokenizer::new(&dialect, sql).tokenize_with_location() {
        Ok(t) if t.len() <= MAX_SCRIPT_TOKENS => t,
        _ => {
            a.replay_supported = false;
            a.unknowns.push("SQL_PARSE_EMPTY_OR_LIMIT".into());
            return;
        }
    };
    let mut statements = vec![];
    let mut current: Vec<sqlparser::tokenizer::TokenWithSpan> = vec![];
    // A SQL-standard routine body (`BEGIN ATOMIC ... END`) holds semicolons of
    // its own; keep it in one statement until the `END` that closes it.
    // `CASE ... END` expressions inside the body are counted so their `END`
    // does not close the body.
    let mut atomic = false;
    let mut case_depth = 0usize;
    let mut last_words: [Option<String>; 2] = [None, None];
    for t in tokens {
        if t.token == Token::SemiColon {
            if atomic && !(case_depth == 0 && last_words[1].as_deref() == Some("END")) {
                current.push(t);
                continue;
            }
            atomic = false;
            case_depth = 0;
            last_words = [None, None];
            statements.push(std::mem::take(&mut current));
            continue;
        }
        if !matches!(t.token, Token::Whitespace(_)) {
            let word = match &t.token {
                Token::Word(w) if w.quote_style.is_none() => Some(w.value.to_ascii_uppercase()),
                _ => None,
            };
            if word.as_deref() == Some("ATOMIC") && last_words[1].as_deref() == Some("BEGIN") {
                atomic = true;
            } else if atomic {
                match word.as_deref() {
                    Some("CASE") => case_depth += 1,
                    // The END that closes a CASE is an ordinary word here.
                    Some("END") if case_depth > 0 => {
                        case_depth -= 1;
                        last_words = [last_words[1].take(), None];
                        current.push(t);
                        continue;
                    }
                    _ => (),
                }
            }
            last_words = [last_words[1].take(), word];
        }
        current.push(t);
    }
    statements.push(current);
    statements.retain(|s| s.iter().any(|t| !matches!(t.token, Token::Whitespace(_))));
    if statements.is_empty() {
        a.replay_supported = false;
        a.unknowns.push("SQL_PARSE_EMPTY_OR_LIMIT".into());
        return;
    }
    let mut created = std::collections::BTreeSet::new();
    for (index, tokens) in statements.into_iter().enumerate() {
        let number = index + 1;
        // Significant tokens: unquoted words uppercased; anything else is None.
        let words: Vec<Option<String>> = tokens
            .iter()
            .filter(|t| !matches!(t.token, Token::Whitespace(_)))
            .map(|t| match &t.token {
                Token::Word(w) if w.quote_style.is_none() => Some(w.value.to_ascii_uppercase()),
                _ => None,
            })
            .collect();
        if let Some(escape) = escapes_transaction(&words) {
            a.replay_supported = false;
            a.unknowns.push(escape.into());
        }
        // A chunk that kept semicolons is a `BEGIN ATOMIC` body, found by a
        // heuristic. Re-check every piece: none may be transaction or session
        // control, and the body must close with a bare END. Otherwise the
        // boundary is uncertain and the script is refused, never replayed.
        if tokens.iter().any(|t| t.token == Token::SemiColon) {
            let pieces: Vec<Vec<Option<String>>> = tokens
                .split(|t| t.token == Token::SemiColon)
                .map(|piece| {
                    piece
                        .iter()
                        .filter(|t| !matches!(t.token, Token::Whitespace(_)))
                        .map(|t| match &t.token {
                            Token::Word(w) if w.quote_style.is_none() => {
                                Some(w.value.to_ascii_uppercase())
                            }
                            _ => None,
                        })
                        .collect()
                })
                .collect();
            let (last, body) = pieces.split_last().expect("split yields a piece");
            if body.iter().any(|p| escapes_transaction(p).is_some())
                || last.as_slice() != [Some("END".to_string())]
            {
                a.replay_supported = false;
                a.unknowns.push("STATEMENT_BOUNDARY_UNCERTAIN".into());
            }
        }
        // ON DELETE/UPDATE CASCADE is a foreign-key action, not a dependency drop.
        if words.iter().enumerate().any(|(i, w)| {
            w.as_deref() == Some("CASCADE")
                && !matches!(
                    i.checked_sub(1).and_then(|p| words[p].as_deref()),
                    Some("DELETE" | "UPDATE")
                )
        }) {
            a.hazards.push("CASCADE_DEPENDENCIES".into());
            a.replay_supported = false;
            a.unknowns.push("SQL_UNSUPPORTED".into());
        }
        let parsed = (tokens.len() <= 4096)
            .then(|| {
                Parser::new(&dialect)
                    .with_recursion_limit(64)
                    .with_tokens_with_locations(tokens)
                    .parse_statements()
                    .ok()
            })
            .flatten();
        match parsed.as_deref() {
            Some([statement]) => {
                predict_locks(a, statement, schema, number, &mut created);
                let operation = classify(a, statement, "postgresql");
                a.operations.push(if operation.is_empty() {
                    leading(&words)
                } else {
                    operation.into()
                });
                if a.hazards.iter().any(|h| h == "CASCADE_DEPENDENCIES") {
                    a.replay_supported = false;
                    a.unknowns.push("SQL_UNSUPPORTED".into());
                }
                if !allowed(statement, schema) {
                    a.unknowns.push("EFFECTS_NOT_ANALYZED".into());
                }
            }
            _ => {
                a.operations.push(leading(&words));
                a.hazards.push("UNANALYZED_EFFECTS".into());
                a.unknowns.push("SQL_NOT_PARSED".into());
                a.unknowns.push("LOCKS_NOT_PREDICTED".into());
            }
        }
    }
    compact_locks(&mut a.locks);
}
/// Merge runs of consecutive statements that take exactly the same locks into
/// one entry per lock, recording the run's last statement in `through`.
type LockSet = Vec<(String, &'static str)>;
/// A run of statements (first, last) taking the same lock set.
type LockRun = (usize, usize, LockSet);
fn compact_locks(locks: &mut Vec<PredictedLock>) {
    let mut by_statement: Vec<(usize, LockSet)> = vec![];
    for l in locks.iter() {
        match by_statement.last_mut() {
            Some((n, set)) if *n == l.statement => set.push((l.relation.clone(), l.mode)),
            _ => by_statement.push((l.statement, vec![(l.relation.clone(), l.mode)])),
        }
    }
    let mut out: Vec<PredictedLock> = vec![];
    let mut run: Option<LockRun> = None;
    let flush = |run: Option<LockRun>, out: &mut Vec<PredictedLock>| {
        if let Some((start, end, set)) = run {
            for (relation, mode) in set {
                out.push(PredictedLock {
                    statement: start,
                    through: (end > start).then_some(end),
                    relation,
                    mode,
                });
            }
        }
    };
    for (n, set) in by_statement {
        run = match run {
            Some((start, end, current)) if end + 1 == n && current == set => {
                Some((start, n, current))
            }
            previous => {
                flush(previous, &mut out);
                Some((n, n, set))
            }
        };
    }
    flush(run, &mut out);
    *locks = out;
}
const ACCESS_EXCLUSIVE: &str = "AccessExclusiveLock";
const SHARE_ROW_EXCLUSIVE: &str = "ShareRowExclusiveLock";
const SHARE_UPDATE_EXCLUSIVE: &str = "ShareUpdateExclusiveLock";
const SHARE: &str = "ShareLock";
const ROW_EXCLUSIVE: &str = "RowExclusiveLock";
fn lock_rank(mode: &str) -> usize {
    [
        "AccessShareLock",
        "RowShareLock",
        ROW_EXCLUSIVE,
        SHARE_UPDATE_EXCLUSIVE,
        SHARE,
        SHARE_ROW_EXCLUSIVE,
        "ExclusiveLock",
        ACCESS_EXCLUSIVE,
    ]
    .iter()
    .position(|m| *m == mode)
    .unwrap_or(0)
}
/// Unqualified or managed-schema table name, lowercased like PostgreSQL.
fn managed_table(n: &ObjectName, schema: &str) -> Option<String> {
    let ids: Vec<_> = n.0.iter().filter_map(|p| p.as_ident()).collect();
    let fold = |i: &Ident| {
        if i.quote_style.is_some() {
            i.value.clone()
        } else {
            i.value.to_lowercase()
        }
    };
    match ids.as_slice() {
        [t] => Some(fold(t)),
        [s, t] if fold(s) == schema => Some(fold(t)),
        _ => None,
    }
}
fn constant_default(e: &Expr) -> bool {
    match e {
        e if literal(e) => true,
        Expr::Cast { expr, .. } => constant_default(expr),
        Expr::Function(f) => {
            let name = f.name.to_string().to_lowercase();
            // Stable functions are evaluated once for ADD COLUMN, not per row.
            matches!(
                name.as_str(),
                "now" | "current_timestamp" | "current_date" | "transaction_timestamp"
            )
        }
        Expr::Value(_) => true,
        _ => false,
    }
}

/// Predict the table locks a statement takes and flag the ones that block or
/// scan an existing table. Modes match PostgreSQL's documented lock levels and
/// were checked against pg_locks. Tables created earlier in the same script
/// are new and unread, so they do not produce warnings.
fn predict_locks(
    a: &mut Analysis,
    statement: &Statement,
    schema: &str,
    number: usize,
    created: &mut std::collections::BTreeSet<String>,
) {
    let take = |a: &mut Analysis, name: &ObjectName, mode: &'static str| -> Option<String> {
        let table = managed_table(name, schema)?;
        if created.contains(&table) {
            return None;
        }
        let existing = a
            .locks
            .iter_mut()
            .find(|l| l.statement == number && l.relation == table);
        match existing {
            Some(l) if lock_rank(mode) > lock_rank(l.mode) => l.mode = mode,
            Some(_) => (),
            None => a.locks.push(PredictedLock {
                statement: number,
                through: None,
                relation: table.clone(),
                mode,
            }),
        }
        a.locks
            .sort_by(|x, y| (x.statement, &x.relation).cmp(&(y.statement, &y.relation)));
        Some(table)
    };
    let warn =
        |a: &mut Analysis, table: Option<String>, code: &'static str, message: &'static str| {
            if let Some(relation) = table {
                if !a
                    .lock_warnings
                    .iter()
                    .any(|w| w.statement == number && w.relation == relation && w.code == code)
                {
                    a.lock_warnings.push(LockWarning {
                        statement: number,
                        relation,
                        code,
                        message,
                    });
                }
            }
        };
    const SCAN: &str = "Checks every existing row while holding the table lock; add it NOT VALID, then VALIDATE in a later migration.";
    const NOT_NULL: &str = "Checks every existing row while holding an exclusive lock; add a CHECK (column IS NOT NULL) NOT VALID, VALIDATE it, then SET NOT NULL.";
    const UNIQUE: &str = "Builds an index while holding an exclusive lock; build a unique index first, then add the constraint USING INDEX.";
    match statement {
        Statement::CreateTable(t) => {
            for c in &t.constraints {
                if let TableConstraint::ForeignKey(fk) = c {
                    take(a, &fk.foreign_table, SHARE_ROW_EXCLUSIVE);
                }
            }
            for col in &t.columns {
                for o in &col.options {
                    if let ColumnOption::ForeignKey(fk) = &o.option {
                        take(a, &fk.foreign_table, SHARE_ROW_EXCLUSIVE);
                    }
                }
            }
            // IF NOT EXISTS may find an existing, populated table.
            if let Some(table) = managed_table(&t.name, schema).filter(|_| !t.if_not_exists) {
                created.insert(table);
            }
        }
        Statement::CreateIndex(i) => {
            let mode = if i.concurrently {
                SHARE_UPDATE_EXCLUSIVE
            } else {
                SHARE
            };
            let table = take(a, &i.table_name, mode);
            if !i.concurrently {
                warn(
                    a,
                    table,
                    "BLOCKS_WRITES_WHILE_INDEXING",
                    "Blocks writes to the table until the index is built.",
                );
            }
        }
        Statement::AlterTable(t) => {
            for op in &t.operations {
                match op {
                    AlterTableOperation::AddConstraint {
                        constraint,
                        not_valid,
                    } => match constraint {
                        TableConstraint::ForeignKey(fk) => {
                            let table = take(a, &t.name, SHARE_ROW_EXCLUSIVE);
                            take(a, &fk.foreign_table, SHARE_ROW_EXCLUSIVE);
                            if !not_valid {
                                warn(a, table, "SCANS_TABLE_UNDER_LOCK", SCAN);
                            }
                        }
                        TableConstraint::Check(_) => {
                            let table = take(a, &t.name, ACCESS_EXCLUSIVE);
                            if !not_valid {
                                warn(a, table, "SCANS_TABLE_UNDER_LOCK", SCAN);
                            }
                        }
                        TableConstraint::PrimaryKey(c) if c.index_name.is_none() => {
                            let table = take(a, &t.name, ACCESS_EXCLUSIVE);
                            warn(a, table, "SCANS_TABLE_UNDER_LOCK", UNIQUE);
                        }
                        TableConstraint::Unique(c) if c.index_name.is_none() => {
                            let table = take(a, &t.name, ACCESS_EXCLUSIVE);
                            warn(a, table, "SCANS_TABLE_UNDER_LOCK", UNIQUE);
                        }
                        _ => {
                            take(a, &t.name, ACCESS_EXCLUSIVE);
                        }
                    },
                    AlterTableOperation::ValidateConstraint { .. } => {
                        take(a, &t.name, SHARE_UPDATE_EXCLUSIVE);
                    }
                    AlterTableOperation::EnableTrigger { .. }
                    | AlterTableOperation::DisableTrigger { .. } => {
                        take(a, &t.name, SHARE_ROW_EXCLUSIVE);
                    }
                    AlterTableOperation::AlterColumn {
                        op: AlterColumnOperation::SetNotNull,
                        ..
                    } => {
                        let table = take(a, &t.name, ACCESS_EXCLUSIVE);
                        warn(a, table, "SCANS_TABLE_UNDER_LOCK", NOT_NULL);
                    }
                    AlterTableOperation::AlterColumn {
                        op: AlterColumnOperation::SetDataType { .. },
                        ..
                    } => {
                        let table = take(a, &t.name, ACCESS_EXCLUSIVE);
                        warn(a, table, "REWRITES_TABLE", "Usually rewrites the whole table and its indexes while holding an exclusive lock.");
                    }
                    AlterTableOperation::AddColumn { column_def, .. } => {
                        let table = take(a, &t.name, ACCESS_EXCLUSIVE);
                        let rewrites = column_def.options.iter().any(|o| match &o.option {
                            ColumnOption::Default(e) => !constant_default(e),
                            ColumnOption::Generated { .. } => true,
                            _ => false,
                        });
                        if rewrites {
                            warn(a, table.clone(), "MAY_REWRITE_TABLE", "A volatile default, identity or stored generated column rewrites the whole table while holding an exclusive lock.");
                        }
                        for o in &column_def.options {
                            match &o.option {
                                ColumnOption::Unique(_) | ColumnOption::PrimaryKey(_) => {
                                    warn(a, table.clone(), "SCANS_TABLE_UNDER_LOCK", UNIQUE)
                                }
                                ColumnOption::Check(_) => {
                                    warn(a, table.clone(), "SCANS_TABLE_UNDER_LOCK", SCAN)
                                }
                                _ => (),
                            }
                        }
                        for o in &column_def.options {
                            if let ColumnOption::ForeignKey(fk) = &o.option {
                                take(a, &fk.foreign_table, SHARE_ROW_EXCLUSIVE);
                            }
                        }
                    }
                    _ => {
                        take(a, &t.name, ACCESS_EXCLUSIVE);
                    }
                }
            }
        }
        Statement::CreateTrigger(t) => {
            take(a, &t.table_name, SHARE_ROW_EXCLUSIVE);
        }
        Statement::DropTrigger(t) => {
            if let Some(table) = &t.table_name {
                take(a, table, ACCESS_EXCLUSIVE);
            }
        }
        Statement::CreatePolicy(p) => {
            take(a, &p.table_name, ACCESS_EXCLUSIVE);
        }
        Statement::AlterPolicy(p) => {
            take(a, &p.table_name, ACCESS_EXCLUSIVE);
        }
        Statement::DropPolicy(p) => {
            take(a, &p.table_name, ACCESS_EXCLUSIVE);
        }
        Statement::Insert(i) => {
            if let TableObject::TableName(n) = &i.table {
                take(a, n, ROW_EXCLUSIVE);
            }
        }
        Statement::Update(u) => {
            if let TableFactor::Table { name, .. } = &u.table.relation {
                take(a, name, ROW_EXCLUSIVE);
            }
        }
        Statement::Delete(d) => {
            let from = match &d.from {
                FromTable::WithFromKeyword(t) | FromTable::WithoutKeyword(t) => t,
            };
            for t in from {
                if let TableFactor::Table { name, .. } = &t.relation {
                    take(a, name, ROW_EXCLUSIVE);
                }
            }
        }
        Statement::Truncate(t) => {
            for target in &t.table_names {
                take(a, &target.name, ACCESS_EXCLUSIVE);
            }
        }
        Statement::Drop {
            object_type: ObjectType::Table,
            names,
            ..
        } => {
            for n in names {
                take(a, n, ACCESS_EXCLUSIVE);
            }
        }
        Statement::Drop {
            object_type: ObjectType::Index,
            ..
        } => {
            // Locks the index's table, which the statement does not name.
            a.unknowns.push("LOCKS_NOT_PREDICTED".into());
        }
        // Grants, comments, settings, functions, views, schemas, extensions
        // and sequences take no lock that blocks access to table rows.
        Statement::Grant(_)
        | Statement::Revoke(_)
        | Statement::Comment { .. }
        | Statement::Set(_)
        | Statement::CreateFunction(_)
        | Statement::CreateView(_)
        | Statement::CreateSchema { .. }
        | Statement::CreateExtension(_)
        | Statement::CreateSequence { .. } => (),
        // Row locks (FOR UPDATE) and called functions can take any lock.
        Statement::Query(_) => a.unknowns.push("LOCKS_NOT_PREDICTED".into()),
        _ => a.unknowns.push("LOCKS_NOT_PREDICTED".into()),
    }
}
/// Readable operation name from leading keywords, e.g. `create_function`, `do`.
fn leading(words: &[Option<String>]) -> String {
    let mut parts = vec![];
    for w in words.iter().map_while(|w| w.as_deref()) {
        if parts.is_empty() {
            parts.push(w.to_ascii_lowercase());
            if !matches!(w, "CREATE" | "ALTER" | "DROP" | "COMMENT") {
                break;
            }
        } else if !matches!(
            w,
            "OR" | "REPLACE" | "UNIQUE" | "CONSTRAINT" | "TEMP" | "TEMPORARY" | "UNLOGGED" | "ON"
        ) {
            parts.push(w.to_ascii_lowercase());
            break;
        }
    }
    if parts.is_empty() {
        "unknown".into()
    } else {
        parts.join("_")
    }
}
/// Top-level statements that would end/split NTNT's migration transaction,
/// change session identity, or act outside the current database. Text inside
/// DO/function bodies is opaque here; those run inside the transaction and
/// with the connecting role's privileges.
fn escapes_transaction(words: &[Option<String>]) -> Option<&'static str> {
    let w = |i: usize| words.get(i).and_then(|w| w.as_deref());
    let concurrently = words.iter().any(|x| x.as_deref() == Some("CONCURRENTLY"));
    match w(0)? {
        "BEGIN" | "START" | "COMMIT" | "END" | "ROLLBACK" | "ABORT" | "SAVEPOINT" | "RELEASE" => {
            Some("TRANSACTION_CONTROL")
        }
        "PREPARE" if w(1) == Some("TRANSACTION") => Some("TRANSACTION_CONTROL"),
        "SET" => {
            let mut i = 1;
            while matches!(w(i), Some("LOCAL" | "SESSION")) {
                i += 1;
            }
            matches!(
                w(i),
                Some("ROLE" | "AUTHORIZATION" | "SESSION_REPLICATION_ROLE")
            )
            .then_some("SESSION_AUTHORIZATION")
        }
        "RESET" => matches!(
            w(1),
            Some("ROLE" | "SESSION" | "ALL" | "SESSION_REPLICATION_ROLE")
        )
        .then_some("SESSION_AUTHORIZATION"),
        "DISCARD" => Some("SESSION_AUTHORIZATION"),
        "VACUUM" => Some("NON_TRANSACTIONAL"),
        "REINDEX" if concurrently || matches!(w(1), Some("DATABASE" | "SYSTEM")) => {
            Some("NON_TRANSACTIONAL")
        }
        "COPY" | "LOAD" | "CHECKPOINT" => Some("SERVER_SCOPE"),
        "ALTER" if w(1) == Some("SYSTEM") => Some("SERVER_SCOPE"),
        "CREATE" | "ALTER" | "DROP"
            if matches!(
                w(1),
                Some("DATABASE" | "TABLESPACE" | "ROLE" | "USER" | "GROUP")
            ) =>
        {
            Some("SERVER_SCOPE")
        }
        "CREATE" | "DROP" | "ALTER" if concurrently => Some("NON_TRANSACTIONAL"),
        _ => None,
    }
}
/// Checks use a separate closed query grammar, not a SELECT/volatility heuristic.
/// Live relation/type/operator resolution still requires the adapter purity gate
/// under native locks; this syntax predicate alone is deliberately insufficient.
pub(crate) fn contract_supported(sql: &str, schema: &str) -> bool {
    let Some(statements) = parse(sql) else {
        return false;
    };
    matches!(statements.as_slice(), [Statement::Query(q)] if check_query(q, schema, 0))
}

/// Managed tables a supported check reads, as (schema, table). Only valid for
/// SQL that `contract_supported` accepts; other input yields no tables.
pub(crate) fn check_tables(sql: &str, schema: &str) -> Vec<(String, String)> {
    fn query(q: &Query, out: &mut Vec<(String, String)>) {
        let SetExpr::Select(s) = &*q.body else { return };
        for t in &s.from {
            if let TableFactor::Table { name, .. } = &t.relation {
                let ids: Vec<_> = name.0.iter().filter_map(|p| p.as_ident()).collect();
                if let [s, t] = ids.as_slice() {
                    let schema = if s.quote_style.is_some() {
                        s.value.clone()
                    } else {
                        s.value.to_lowercase()
                    };
                    let table = if t.quote_style.is_some() {
                        t.value.clone()
                    } else {
                        t.value.to_lowercase()
                    };
                    out.push((schema, table));
                }
            }
        }
        for item in &s.projection {
            if let SelectItem::UnnamedExpr(e) = item {
                expr(e, out);
            }
        }
        for e in s.selection.iter().chain(s.having.iter()) {
            expr(e, out);
        }
    }
    fn expr(e: &Expr, out: &mut Vec<(String, String)>) {
        match e {
            Expr::Exists { subquery, .. } | Expr::Subquery(subquery) => query(subquery, out),
            Expr::Nested(e)
            | Expr::IsNull(e)
            | Expr::IsNotNull(e)
            | Expr::UnaryOp { expr: e, .. } => expr(e, out),
            Expr::BinaryOp { left, right, .. } => {
                expr(left, out);
                expr(right, out);
            }
            _ => (),
        }
    }
    let mut out = vec![];
    if contract_supported(sql, schema) {
        if let Some(statements) = parse(sql) {
            if let [Statement::Query(q)] = statements.as_slice() {
                query(q, &mut out);
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

fn check_expr(e: &Expr, schema: &str, depth: usize) -> bool {
    if depth > 24 {
        return false;
    }
    match e {
        e if literal(e) => true,
        Expr::Identifier(i) => ident(i),
        Expr::Nested(e) | Expr::IsNull(e) | Expr::IsNotNull(e) => check_expr(e, schema, depth + 1),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr,
        } => check_expr(expr, schema, depth + 1),
        Expr::BinaryOp { left, op, right } => {
            // Reuse only the operator allowlist, not an open function visitor.
            expression(
                &Expr::BinaryOp {
                    left: Box::new(Expr::Identifier(Ident::new("x"))),
                    op: op.clone(),
                    right: Box::new(Expr::Identifier(Ident::new("x"))),
                },
                0,
            ) && check_expr(left, schema, depth + 1)
                && check_expr(right, schema, depth + 1)
        }
        Expr::Exists { subquery, .. } | Expr::Subquery(subquery) => {
            check_query(subquery, schema, depth + 1)
        }
        Expr::Function(_) => ["SELECT count(*)", "SELECT pg_catalog.count(*)"]
            .iter()
            .any(|sql| {
                let Some(s) = parse(sql) else { return false };
                let Statement::Query(q) = &s[0] else {
                    return false;
                };
                let SetExpr::Select(s) = &*q.body else {
                    return false;
                };
                s.projection == [SelectItem::UnnamedExpr(e.clone())]
            }),
        _ => false,
    }
}

fn check_query(q: &Query, schema: &str, depth: usize) -> bool {
    if depth > 24 {
        return false;
    }
    let SetExpr::Select(s) = &*q.body else {
        return false;
    };
    if s.projection.is_empty()
        || s.projection.len() > 8
        || !s
            .projection
            .iter()
            .all(|p| matches!(p, SelectItem::UnnamedExpr(e) if check_expr(e,schema,depth+1)))
    {
        return false;
    }
    let mut minimal = format!(
        "SELECT {}",
        s.projection
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    match s.from.as_slice() {
        [] => (),
        [table] => {
            let TableFactor::Table { name: n, .. } = &table.relation else {
                return false;
            };
            // Preserve pg_catalog-first builtin lookup without letting a table
            // reference silently resolve outside the inspected managed schema.
            if n.0.len() != 2 || !name(n, schema) {
                return false;
            }
            minimal.push_str(&format!(" FROM {n}"));
        }
        _ => return false,
    }
    if let Some(e) = &s.selection {
        if !check_expr(e, schema, depth + 1) {
            return false;
        }
        minimal.push_str(&format!(" WHERE {e}"));
    }
    match &s.group_by {
        GroupByExpr::Expressions(es, modifiers) if modifiers.is_empty() => {
            if !es
                .iter()
                .all(|e| matches!(e, Expr::Identifier(i) if ident(i)))
            {
                return false;
            }
            if !es.is_empty() {
                minimal.push_str(&format!(
                    " GROUP BY {}",
                    es.iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        _ => return false,
    }
    if let Some(e) = &s.having {
        if !check_expr(e, schema, depth + 1) {
            return false;
        }
        minimal.push_str(&format!(" HAVING {e}"));
    }
    parse(&minimal)
        .is_some_and(|ss| matches!(ss.as_slice(), [Statement::Query(expected)] if **expected==*q))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_relations_require_the_managed_schema_at_every_query_level() {
        for sql in [
            "SELECT true FROM pg_class",
            "SELECT true FROM PG_CLASS",
            "SELECT true FROM \"pg_class\"",
            "SELECT true FROM pg_stat_activity",
            "SELECT true FROM pg_authid",
            "SELECT true FROM accounts",
            "SELECT NOT EXISTS (SELECT 1 FROM pg_class WHERE oid = 0)",
            "SELECT (SELECT count(*) FROM pg_class) = 0 FROM app.accounts",
            "SELECT true FROM pg_catalog.pg_class",
            "SELECT true FROM public.accounts",
        ] {
            assert!(!contract_supported(sql, "app"), "{sql}");
        }
        for sql in [
            "SELECT true",
            "SELECT NOT EXISTS (SELECT 1 FROM app.pg_class WHERE oid = 0)",
            "SELECT true FROM \"app\".\"pg_class\"",
            "SELECT true FROM APP.accounts",
            "SELECT (SELECT count(*) FROM app.pg_class) = 0 FROM app.accounts",
        ] {
            assert!(contract_supported(sql, "app"), "{sql}");
        }
    }

    #[test]
    fn parser_bounds_expression_depth_on_small_thread_stack() {
        let sql = format!(
            "CREATE TABLE app.t (id INTEGER CHECK({} > 0));",
            vec!["1"; 900].join(" + ")
        );
        let supported = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || analyze(&sql, "app").unknowns)
            .unwrap()
            .join()
            .unwrap();
        // Bounded recursion: reported as not understood, never a stack overflow.
        assert!(!supported.is_empty(), "{supported:?}");
    }
    #[test]
    fn oversized_token_streams_and_cascade_hazards_are_explicit() {
        assert!(parse(&"CREATE TABLE app.t (id INTEGER);".repeat(600)).is_none());
        let a = analyze("ALTER TABLE app.t DROP COLUMN id CASCADE", "app");
        assert!(a.hazards.iter().any(|h| h == "CASCADE_DEPENDENCIES"));
    }
    #[test]
    fn parser_preserves_quoted_semicolons_and_rejects_routines() {
        let sql = "CREATE TABLE app.t (id INTEGER PRIMARY KEY, label TEXT DEFAULT 'a; b'); ALTER TABLE app.t ADD COLUMN active BOOLEAN NOT NULL DEFAULT true;";
        let statements = parse(sql).unwrap();
        assert_eq!(statements.len(), 2);
        assert_eq!(
            statements,
            parse(
                &statements
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(";")
            )
            .unwrap()
        );
        let result = analyze(sql, "app");
        assert!(result.replay_supported, "{result:?}");
        assert!(result.unknowns.is_empty(), "{result:?}");
        // Outside the analyzed subset: replayable in scratch, but never silently
        // presented as understood.
        for sql in [
            "SELECT pg_advisory_lock(1)",
            "CREATE TABLE app.bad (id int DEFAULT pg_advisory_lock(1))",
            "CREATE TABLE app.bad AS SELECT 1",
            "CREATE TABLE app.bad (id custom_type)",
        ] {
            let a = analyze(sql, "app");
            assert!(a.replay_supported, "{sql}");
            assert!(
                a.unknowns.iter().any(|u| u == "EFFECTS_NOT_ANALYZED"),
                "{sql}"
            );
        }
        assert!(!analyze("COMMIT", "app").replay_supported);
    }

    #[test]
    fn postgres_replays_complete_transactional_sql() {
        // Real applications use DO blocks, SET CONSTRAINTS, seed data, routines
        // and scripts larger than the AST token budget. Scratch runs them.
        let long = "INSERT INTO app.t VALUES (1);\n".repeat(700);
        for sql in [
            "DO $$ BEGIN IF true THEN CREATE TABLE app.t(id int); END IF; END $$;",
            "SET CONSTRAINTS ALL DEFERRED; INSERT INTO app.t VALUES (1);",
            "CREATE FUNCTION app.f() RETURNS int LANGUAGE sql AS $fn$ SELECT 1; $fn$;",
            "CREATE TABLE app.c(id int REFERENCES app.t(id) ON DELETE CASCADE);",
            "ALTER TABLE app.c ADD FOREIGN KEY (id) REFERENCES app.t(id) ON UPDATE CASCADE;",
            "SELECT pg_advisory_xact_lock(1)",
            long.as_str(),
        ] {
            let a = analyze(sql, "app");
            assert!(a.replay_supported, "{sql}: {a:?}");
        }
        let unparsed = analyze("DO $$ BEGIN PERFORM 1; END $$", "app");
        assert!(unparsed.unknowns.iter().any(|u| u == "SQL_NOT_PARSED"));
        assert_eq!(unparsed.operations, ["do"]);
        let outside = analyze(
            "CREATE FUNCTION app.f() RETURNS int LANGUAGE sql AS 'SELECT 1'",
            "app",
        );
        assert!(
            outside.unknowns.iter().any(|u| u == "EFFECTS_NOT_ANALYZED"),
            "{outside:?}"
        );
    }
    #[test]
    fn postgres_rejects_statements_that_escape_the_replay_transaction() {
        for (sql, unknown) in [
            ("COMMIT", "TRANSACTION_CONTROL"),
            (
                "CREATE TABLE app.t(id int); BEGIN; DROP TABLE app.t",
                "TRANSACTION_CONTROL",
            ),
            ("end", "TRANSACTION_CONTROL"),
            ("SAVEPOINT s", "TRANSACTION_CONTROL"),
            ("PREPARE TRANSACTION 'x'", "TRANSACTION_CONTROL"),
            ("SET ROLE postgres", "SESSION_AUTHORIZATION"),
            ("SET LOCAL ROLE postgres", "SESSION_AUTHORIZATION"),
            (
                "set session authorization postgres",
                "SESSION_AUTHORIZATION",
            ),
            ("RESET ALL", "SESSION_AUTHORIZATION"),
            (
                "CREATE INDEX CONCURRENTLY i ON app.t(id)",
                "NON_TRANSACTIONAL",
            ),
            ("DROP INDEX CONCURRENTLY app.i", "NON_TRANSACTIONAL"),
            ("VACUUM", "NON_TRANSACTIONAL"),
            ("COPY app.t FROM PROGRAM 'id'", "SERVER_SCOPE"),
            ("ALTER SYSTEM SET work_mem = '1GB'", "SERVER_SCOPE"),
            ("CREATE DATABASE other", "SERVER_SCOPE"),
            (
                "DO $$ BEGIN END $$; DROP TABLE app.t CASCADE",
                "SQL_UNSUPPORTED",
            ),
            ("TRUNCATE app.t CASCADE", "SQL_UNSUPPORTED"),
        ] {
            let a = analyze(sql, "app");
            assert!(!a.replay_supported, "{sql}");
            assert!(a.unknowns.iter().any(|u| u == unknown), "{sql}: {a:?}");
        }
        // A quoted or dollar-quoted keyword is data, not a statement boundary.
        let a = analyze("DO $$ BEGIN RAISE NOTICE 'COMMIT; SET ROLE x'; END $$; INSERT INTO app.t VALUES ('BEGIN;')", "app");
        assert!(a.replay_supported, "{a:?}");
        assert_eq!(a.operations.len(), 2, "{a:?}");
    }
    #[test]
    fn check_tables_lists_every_table_a_check_reads() {
        assert_eq!(
            check_tables(
                "SELECT NOT EXISTS (SELECT 1 FROM app.orders WHERE (SELECT count(*) FROM APP.\"Items\") > 0) FROM app.orders",
                "app"
            ),
            [("app".to_string(), "Items".to_string()), ("app".to_string(), "orders".to_string())]
        );
        assert!(check_tables("SELECT true", "app").is_empty());
        assert!(check_tables("DELETE FROM app.orders", "app").is_empty());
    }
    #[test]
    fn sql_standard_routine_bodies_stay_one_statement() {
        let sql = "CREATE FUNCTION app.f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; SELECT 2; END; CREATE TABLE app.t(id int);";
        let a = analyze(sql, "app");
        assert!(a.replay_supported, "{a:?}");
        assert_eq!(a.operations.len(), 2, "{a:?}");
        assert!(
            !a.unknowns.iter().any(|u| u == "TRANSACTION_CONTROL"),
            "{a:?}"
        );
        // CASE ... END inside the body does not close it.
        let case = "CREATE FUNCTION app.f(x int) RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT CASE WHEN x > 0 THEN 1 ELSE 2 END; SELECT 2; END; CREATE TABLE app.t(id int);";
        let a = analyze(case, "app");
        assert!(a.replay_supported, "{a:?}");
        assert_eq!(a.operations.len(), 2, "{a:?}");
        // Splitting is a heuristic; it must never hide transaction control.
        // A column named `case` defeats the CASE counter, so these are refused.
        for hidden in [
            "CREATE FUNCTION app.f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1 AS case; END; COMMIT;",
            "CREATE FUNCTION app.f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1 AS case; END; COMMIT; SELECT CASE WHEN true THEN 1 END; END;",
        ] {
            let a = analyze(hidden, "app");
            assert!(!a.replay_supported, "{hidden}: {a:?}");
            assert!(a.unknowns.iter().any(|u| u == "STATEMENT_BOUNDARY_UNCERTAIN"), "{a:?}");
        }
        // A bare END after the body is still transaction control.
        assert!(!analyze("CREATE TABLE app.t(id int); END;", "app").replay_supported);
    }
    fn locks(sql: &str) -> Vec<(String, String)> {
        analyze(sql, "app")
            .locks
            .iter()
            .map(|l| (l.relation.clone(), l.mode.to_string()))
            .collect()
    }
    fn warnings(sql: &str) -> Vec<(String, String)> {
        analyze(sql, "app")
            .lock_warnings
            .iter()
            .map(|w| (w.relation.clone(), w.code.to_string()))
            .collect()
    }
    fn pair(r: &str, m: &str) -> (String, String) {
        (r.to_string(), m.to_string())
    }
    #[test]
    fn predicts_postgresql_lock_modes_per_statement() {
        // Modes observed on PostgreSQL 17 (pg_locks) for each statement form.
        for (sql, expected) in [
            ("CREATE INDEX i ON app.t(id)", vec![pair("t", "ShareLock")]),
            ("ALTER TABLE app.t ADD CONSTRAINT ck CHECK (id > 0) NOT VALID", vec![pair("t", "AccessExclusiveLock")]),
            ("ALTER TABLE app.t VALIDATE CONSTRAINT ck", vec![pair("t", "ShareUpdateExclusiveLock")]),
            ("ALTER TABLE app.t ADD CONSTRAINT fk FOREIGN KEY (pid) REFERENCES app.p(id) NOT VALID", vec![pair("p", "ShareRowExclusiveLock"), pair("t", "ShareRowExclusiveLock")]),
            ("ALTER TABLE t DISABLE TRIGGER tr", vec![pair("t", "ShareRowExclusiveLock")]),
            ("CREATE TRIGGER tr BEFORE INSERT ON app.t FOR EACH ROW EXECUTE FUNCTION app.f()", vec![pair("t", "ShareRowExclusiveLock")]),
            ("CREATE POLICY pol ON app.t USING (true)", vec![pair("t", "AccessExclusiveLock")]),
            ("INSERT INTO app.t(id) VALUES (1)", vec![pair("t", "RowExclusiveLock")]),
            ("UPDATE app.t SET n = 'a'", vec![pair("t", "RowExclusiveLock")]),
            ("DELETE FROM app.t", vec![pair("t", "RowExclusiveLock")]),
            ("TRUNCATE app.t", vec![pair("t", "AccessExclusiveLock")]),
            ("CREATE TABLE app.c(pid int REFERENCES app.p(id))", vec![pair("p", "ShareRowExclusiveLock")]),
            ("GRANT SELECT ON app.t TO PUBLIC", vec![]),
        ] {
            assert_eq!(locks(sql), expected, "{sql}");
            assert!(!analyze(sql, "app").unknowns.iter().any(|u| u == "LOCKS_NOT_PREDICTED"), "{sql}");
        }
        // Queries can lock rows or call functions that take any lock.
        for sql in [
            "SELECT * FROM app.t FOR UPDATE",
            "SELECT pg_advisory_lock(1)",
        ] {
            assert!(
                analyze(sql, "app")
                    .unknowns
                    .iter()
                    .any(|u| u == "LOCKS_NOT_PREDICTED"),
                "{sql}"
            );
        }
        // Consecutive statements with the same lock collapse into one range,
        // so large seed migrations stay within the plan size limit.
        let seed = "INSERT INTO app.t(id) VALUES (1);".repeat(5000);
        let a = analyze(&seed, "app");
        assert_eq!(a.locks.len(), 1, "{:?}", &a.locks[..3.min(a.locks.len())]);
        assert_eq!((a.locks[0].statement, a.locks[0].through), (1, Some(5000)));
        assert!(serde_json::to_string(&a).unwrap().len() < 200_000);
        let unknown = analyze("DO $$ BEGIN PERFORM 1; END $$", "app");
        assert!(
            unknown.unknowns.iter().any(|u| u == "LOCKS_NOT_PREDICTED"),
            "{unknown:?}"
        );
    }
    #[test]
    fn warns_about_locks_that_block_or_scan_existing_tables() {
        for (sql, expected) in [
            ("CREATE INDEX i ON app.t(id)", vec![pair("t", "BLOCKS_WRITES_WHILE_INDEXING")]),
            ("ALTER TABLE app.t ADD CONSTRAINT ck CHECK (id > 0)", vec![pair("t", "SCANS_TABLE_UNDER_LOCK")]),
            ("ALTER TABLE app.t ADD CONSTRAINT fk FOREIGN KEY (pid) REFERENCES app.p(id)", vec![pair("t", "SCANS_TABLE_UNDER_LOCK")]),
            ("ALTER TABLE app.t ALTER COLUMN n SET NOT NULL", vec![pair("t", "SCANS_TABLE_UNDER_LOCK")]),
            ("ALTER TABLE app.t ADD CONSTRAINT u UNIQUE (id)", vec![pair("t", "SCANS_TABLE_UNDER_LOCK")]),
            ("ALTER TABLE app.t ALTER COLUMN id TYPE bigint", vec![pair("t", "REWRITES_TABLE")]),
            ("ALTER TABLE app.t ADD COLUMN token uuid DEFAULT gen_random_uuid()", vec![pair("t", "MAY_REWRITE_TABLE")]),
            // The safe forms carry no warning.
            ("ALTER TABLE app.t ADD CONSTRAINT ck CHECK (id > 0) NOT VALID", vec![]),
            ("ALTER TABLE app.t VALIDATE CONSTRAINT ck", vec![]),
            ("ALTER TABLE app.t ADD COLUMN x int DEFAULT 1", vec![]),
            // A table created earlier in the same migration has no readers yet.
            ("CREATE TABLE app.n(id int); CREATE INDEX i ON app.n(id); ALTER TABLE app.n ALTER COLUMN id SET NOT NULL", vec![]),
        ] {
            assert_eq!(warnings(sql), expected, "{sql}");
        }
        // IF NOT EXISTS may find an existing, populated table.
        assert_eq!(
            warnings("CREATE TABLE IF NOT EXISTS app.t(id int); CREATE INDEX i ON app.t(id)"),
            vec![pair("t", "BLOCKS_WRITES_WHILE_INDEXING")]
        );
        // Inline constraints on a new column still do work under the lock.
        for sql in [
            "ALTER TABLE app.t ADD COLUMN x int UNIQUE",
            "ALTER TABLE app.t ADD COLUMN x int CHECK (x > 0)",
            "ALTER TABLE app.t ADD COLUMN x int PRIMARY KEY",
        ] {
            assert_eq!(
                warnings(sql),
                vec![pair("t", "SCANS_TABLE_UNDER_LOCK")],
                "{sql}"
            );
        }
        let a = analyze(
            "CREATE TABLE app.n(id int); CREATE INDEX i ON app.n(id)",
            "app",
        );
        assert!(a.locks.is_empty(), "{a:?}");
        assert_eq!(
            analyze("SELECT 1; CREATE INDEX i ON app.t(id)", "app").lock_warnings[0].statement,
            2
        );
    }
}
