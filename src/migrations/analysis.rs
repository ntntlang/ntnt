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
    /// PostgreSQL only: the migration is a single `CREATE INDEX CONCURRENTLY`,
    /// which `migrate` runs outside a transaction.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub non_transactional: bool,
    /// PostgreSQL only: statements that may create, change, drop, grant or
    /// revoke objects in a schema outside the managed scope, which NTNT does
    /// not inspect (`analyze_in_scope`). Never a refusal.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub outside_scope: Vec<OutsideScope>,
}
/// A statement that may act in a schema outside the managed scope.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub(crate) struct OutsideScope {
    /// 1-based statement number within the migration script.
    pub statement: usize,
    /// The schema, folded like PostgreSQL.
    pub schema: String,
    /// How the statement names it.
    pub via: Via,
}
/// A migration whose whole script is one named `CREATE INDEX CONCURRENTLY` on
/// a managed table. Replay builds the same index without CONCURRENTLY, inside
/// its rollback-only transaction; the catalog result is identical.
#[derive(Clone, Debug)]
pub(crate) struct ConcurrentIndex {
    pub name: String,
    pub replay_sql: String,
}
pub(crate) fn concurrent_index(sql: &str, schema: &str) -> Option<ConcurrentIndex> {
    let statements = parse(sql)?;
    let [Statement::CreateIndex(index)] = statements.as_slice() else {
        return None;
    };
    if !index.concurrently {
        return None;
    }
    let index_name = index.name.as_ref()?;
    if !name(index_name, schema) {
        return None;
    }
    managed_table(&index.table_name, schema)?;
    let last = index_name.0.last()?.as_ident()?;
    let folded = if last.quote_style.is_some() {
        last.value.clone()
    } else {
        last.value.to_lowercase()
    };
    let mut replay = index.clone();
    replay.concurrently = false;
    Some(ConcurrentIndex {
        name: folded,
        replay_sql: Statement::CreateIndex(replay).to_string(),
    })
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
/// Whether `sql` holds no statement at all: only whitespace, comments and
/// semicolons, such as the header `ntnt db new` writes (#428). Input that does
/// not tokenize is not empty; the other checks report it.
pub(crate) fn is_empty(sql: &str, engine: &str) -> bool {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let tokens = if engine == "sqlite" {
        Tokenizer::new(&sqlparser::dialect::SQLiteDialect {}, sql).tokenize()
    } else {
        Tokenizer::new(&PostgreSqlDialect {}, sql).tokenize()
    };
    tokens.is_ok_and(|tokens| {
        tokens
            .iter()
            .all(|token| matches!(token, Token::Whitespace(_) | Token::SemiColon | Token::EOF))
    })
}

/// The most tokens, whitespace and comments included, that the parser reads
/// in one script.
pub(crate) const PARSER_TOKEN_LIMIT: usize = 4096;

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
    if tokens.len() > PARSER_TOKEN_LIMIT {
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
        non_transactional: false,
        outside_scope: vec![],
    };
    if engine == "sqlite" {
        sqlite_script(&mut a, sql);
    } else {
        if let Some(index) = concurrent_index(sql, schema) {
            postgres_script(&mut a, &index.replay_sql, schema);
            a.non_transactional = true;
            for lock in &mut a.locks {
                lock.mode = SHARE_UPDATE_EXCLUSIVE;
            }
            a.lock_warnings
                .retain(|w| w.code != "BLOCKS_WRITES_WHILE_INDEXING");
        } else {
            postgres_script(&mut a, sql, schema);
        }
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
        let supported = sqlite_kind_supported(&s);
        if !supported || a.hazards.iter().any(|h| h == "CASCADE_DEPENDENCIES") {
            a.replay_supported = false;
            a.unknowns.push("SQL_UNSUPPORTED".into());
        }
    }
}

/// Whether the SQLite subset includes statement `s`'s kind (see `sqlite_script`).
fn sqlite_kind_supported(s: &Statement) -> bool {
    !matches!(s, Statement::CreateTable(table) if table.query.is_some())
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
        )
}

/// Why the SQLite parser gate refuses a migration (#424): the statement it
/// stops at and the cause. It names keywords, identifiers and symbols, never
/// a literal. Statements are numbered by SQLite's own boundaries, so a gate
/// that prepares each statement with SQLite itself (#426) can say the same.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SqliteRefusal {
    /// The statement does not parse. `near` says where it stops: a keyword,
    /// identifier or symbol in backticks, or what else stands there.
    Syntax {
        statement: usize,
        near: Option<String>,
    },
    /// The script passes the parser's token limit inside this statement.
    TokenLimit { statement: usize, limit: usize },
    /// An authored PRAGMA, by name.
    Pragma { statement: usize, name: String },
    /// A statement the subset does not include, by its leading keywords.
    Unsupported { statement: usize, construct: String },
}

/// The first refusal of the SQLite parser gate in `sql`, or `None` when the
/// gate replays it.
pub(crate) fn sqlite_refusal(sql: &str) -> Option<SqliteRefusal> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let dialect = sqlparser::dialect::SQLiteDialect {};
    let tokens = match Tokenizer::new(&dialect, sql).tokenize_with_location() {
        Ok(tokens) => tokens,
        // Text that does not tokenize is in the statement after every one
        // that ends before it.
        Err(error) => {
            let end = offset(sql, error.location.line, error.location.column);
            let statement = Tokenizer::new(&dialect, &sql[..end])
                .tokenize_with_location()
                .map_or(1, |tokens| sqlite_statements(&tokens).ended + 1);
            return Some(SqliteRefusal::Syntax {
                statement,
                near: None,
            });
        }
    };
    let split = sqlite_statements(&tokens);
    if tokens.len() > PARSER_TOKEN_LIMIT {
        return Some(SqliteRefusal::TokenLimit {
            statement: split.statement_of(PARSER_TOKEN_LIMIT),
            limit: PARSER_TOKEN_LIMIT,
        });
    }
    for (index, range) in split.ranges.iter().enumerate() {
        let statement = index + 1;
        let piece = &tokens[range.clone()];
        let words: Vec<&Token> = piece
            .iter()
            .map(|t| &t.token)
            .filter(|t| !matches!(t, Token::Whitespace(_)))
            .collect();
        if matches!(words.first(), Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case("PRAGMA"))
        {
            let name: String = words[1..]
                .iter()
                .map_while(|t| match t {
                    Token::Word(w) => Some(w.to_string()),
                    Token::Period => Some(".".to_owned()),
                    _ => None,
                })
                .collect();
            return Some(SqliteRefusal::Pragma {
                statement,
                name: visible(&name),
            });
        }
        let parsed = match Parser::new(&dialect)
            .with_recursion_limit(64)
            .with_tokens_with_locations(piece.to_vec())
            .parse_statements()
        {
            Ok(parsed) => parsed,
            Err(error) => {
                let near = if statement == 1 && sql.starts_with('\u{feff}') {
                    Some("a UTF-8 byte order mark (BOM) at the start of the file".to_owned())
                } else {
                    near(&error.to_string(), piece)
                };
                return Some(SqliteRefusal::Syntax { statement, near });
            }
        };
        for s in &parsed {
            let mut probe = Analysis {
                replay_supported: true,
                operations: vec![],
                hazards: vec![],
                unknowns: vec![],
                locks: vec![],
                lock_warnings: vec![],
                non_transactional: false,
                outside_scope: vec![],
            };
            classify(&mut probe, s, "sqlite");
            let cascade = probe.hazards.iter().any(|h| h == "CASCADE_DEPENDENCIES");
            if sqlite_kind_supported(s) && !cascade {
                continue;
            }
            let leading: Vec<String> = words
                .iter()
                .map_while(|t| match t {
                    Token::Word(w)
                        if w.quote_style.is_none()
                            && w.keyword != sqlparser::keywords::Keyword::NoKeyword =>
                    {
                        Some(w.value.to_ascii_uppercase())
                    }
                    _ => None,
                })
                .take(3)
                .collect();
            let construct = match s {
                Statement::CreateTable(table) if table.query.is_some() => {
                    "CREATE TABLE ... AS SELECT".to_owned()
                }
                _ if cascade => format!("{} ... CASCADE", leading.join(" ")),
                _ => leading.join(" "),
            };
            return Some(SqliteRefusal::Unsupported {
                statement,
                construct,
            });
        }
    }
    None
}

/// The byte offset in `sql` of 1-based `line` and `column` (in characters),
/// as the tokenizer counts them.
fn offset(sql: &str, line: u64, column: u64) -> usize {
    let (mut at_line, mut at_column) = (1, 1);
    for (index, c) in sql.char_indices() {
        if (at_line, at_column) == (line, column) {
            return index;
        }
        if c == '\n' {
            at_line += 1;
            at_column = 1;
        } else {
            at_column += 1;
        }
    }
    sql.len()
}

/// Statement boundaries in a tokenized SQLite script, by SQLite's own rule
/// (`sqlite3_complete`): a semicolon ends a statement, except inside a
/// CREATE TRIGGER body, which ends at `END;`. Statements of only whitespace
/// and comments are skipped, as SQLite skips them.
struct Split {
    /// Each statement's tokens, with the semicolon that ends it.
    ranges: Vec<std::ops::Range<usize>>,
    /// How many of them a semicolon ends.
    ended: usize,
}
impl Split {
    /// The 1-based statement that holds token `index`.
    fn statement_of(&self, index: usize) -> usize {
        self.ranges
            .iter()
            .position(|range| index < range.end)
            .map_or(self.ranges.len().max(1), |i| i + 1)
    }
}
fn sqlite_statements(tokens: &[sqlparser::tokenizer::TokenWithSpan]) -> Split {
    use sqlparser::tokenizer::Token;
    // sqlite3_complete's transitions: the state (row) after a token of each
    // class (column): SEMI, WS, OTHER, EXPLAIN, CREATE, TEMP, TRIGGER, END.
    // State 1 follows a complete statement.
    const NEXT: [[u8; 8]; 8] = [
        [1, 0, 2, 3, 4, 2, 2, 2],
        [1, 1, 2, 3, 4, 2, 2, 2],
        [1, 2, 2, 2, 2, 2, 2, 2],
        [1, 3, 3, 2, 4, 2, 2, 2],
        [1, 4, 2, 2, 2, 4, 5, 2],
        [6, 5, 5, 5, 5, 5, 5, 5],
        [6, 6, 5, 5, 5, 5, 5, 7],
        [1, 7, 5, 5, 5, 5, 5, 5],
    ];
    let significant = |range: &std::ops::Range<usize>| {
        tokens[range.clone()]
            .iter()
            .any(|t| !matches!(t.token, Token::Whitespace(_) | Token::SemiColon))
    };
    let mut split = Split {
        ranges: vec![],
        ended: 0,
    };
    let (mut state, mut start) = (0u8, 0usize);
    for (index, token) in tokens.iter().enumerate() {
        let class = match &token.token {
            Token::SemiColon => 0,
            Token::Whitespace(_) => 1,
            Token::Word(w) if w.quote_style.is_none() => {
                match w.value.to_ascii_uppercase().as_str() {
                    "EXPLAIN" => 3,
                    "CREATE" => 4,
                    "TEMP" | "TEMPORARY" => 5,
                    "TRIGGER" => 6,
                    "END" => 7,
                    _ => 2,
                }
            }
            _ => 2,
        };
        state = NEXT[usize::from(state)][class];
        if class == 0 && state == 1 {
            let range = start..index + 1;
            if significant(&range) {
                split.ranges.push(range);
                split.ended += 1;
            }
            start = index + 1;
        }
    }
    let rest = start..tokens.len();
    if significant(&rest) {
        split.ranges.push(rest);
    }
    split
}

/// Where a statement stops parsing, from the position in the parser's
/// `error`: the keyword, identifier or symbol there, in backticks, or what
/// else stands there. A literal is never shown.
fn near(error: &str, tokens: &[sqlparser::tokenizer::TokenWithSpan]) -> Option<String> {
    use sqlparser::tokenizer::Token;
    let (_, at) = error.rsplit_once(" at Line: ")?;
    let (line, column) = at.split_once(", Column: ")?;
    let (line, column): (u64, u64) = (line.parse().ok()?, column.trim().parse().ok()?);
    let token = &tokens
        .iter()
        .find(|t| t.span.start.line == line && t.span.start.column == column)?
        .token;
    Some(match token {
        Token::Word(w) => format!("`{}`", visible(&w.to_string())),
        Token::Comma
        | Token::SemiColon
        | Token::LParen
        | Token::RParen
        | Token::Period
        | Token::Eq
        | Token::DoubleEq
        | Token::Neq
        | Token::Lt
        | Token::Gt
        | Token::LtEq
        | Token::GtEq
        | Token::Plus
        | Token::Minus
        | Token::Mul
        | Token::Div
        | Token::Mod
        | Token::StringConcat
        | Token::LBracket
        | Token::RBracket
        | Token::Colon
        | Token::Ampersand
        | Token::Pipe
        | Token::Caret => format!("`{token}`"),
        _ => "a literal".to_owned(),
    })
}

/// `text`, at most 64 characters of it, with characters that do not print
/// (such as a byte order mark) written as escapes.
fn visible(text: &str) -> String {
    let mut shown: String = text
        .chars()
        .take(64)
        .map(|c| {
            if c.is_alphanumeric() || c.is_ascii_graphic() || c == ' ' {
                c.to_string()
            } else {
                c.escape_unicode().to_string()
            }
        })
        .collect();
    if text.chars().count() > 64 {
        shown.push_str("...");
    }
    shown
}

/// The literal messages of the `RAISE(ABORT|FAIL|ROLLBACK, '...')` calls in
/// a SQLite trigger's SQL (#424).
pub(crate) fn sqlite_raise_messages(sql: &str) -> Vec<String> {
    use sqlparser::tokenizer::Tokenizer;
    let Ok(tokens) = Tokenizer::new(&sqlparser::dialect::SQLiteDialect {}, sql).tokenize() else {
        return vec![];
    };
    raise_literals(tokens.iter())
}

/// The literal RAISE messages of the CREATE TRIGGER statements among the
/// first `count` statements of the SQLite migration `sql`, numbered by
/// SQLite's own statement boundaries. A RAISE(ROLLBACK) undoes the
/// transaction that created such a trigger, so once the statement it raised
/// in has failed the catalog no longer holds it (review r1).
pub(crate) fn sqlite_created_raise_messages(sql: &str, count: usize) -> Vec<String> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let Ok(tokens) =
        Tokenizer::new(&sqlparser::dialect::SQLiteDialect {}, sql).tokenize_with_location()
    else {
        return vec![];
    };
    let keyword = |token: Option<&Token>, keywords: &[&str]| {
        matches!(token, Some(Token::Word(w)) if w.quote_style.is_none()
            && keywords.iter().any(|k| w.value.eq_ignore_ascii_case(k)))
    };
    sqlite_statements(&tokens)
        .ranges
        .iter()
        .take(count)
        .flat_map(|range| {
            let piece = tokens[range.clone()].iter().map(|t| &t.token);
            let mut words = piece.clone().filter(|t| !matches!(t, Token::Whitespace(_)));
            let create = keyword(words.next(), &["CREATE"]);
            let mut next = words.next();
            if keyword(next, &["TEMP", "TEMPORARY"]) {
                next = words.next();
            }
            if create && keyword(next, &["TRIGGER"]) {
                raise_literals(piece)
            } else {
                vec![]
            }
        })
        .collect()
}

/// The messages of the RAISEs in `tokens` whose message is one string
/// literal: text the SQL spells out, never one computed from a row.
fn raise_literals<'a>(
    tokens: impl Iterator<Item = &'a sqlparser::tokenizer::Token>,
) -> Vec<String> {
    use sqlparser::tokenizer::Token;
    let tokens: Vec<&Token> = tokens
        .filter(|t| !matches!(t, Token::Whitespace(_)))
        .collect();
    tokens
        .windows(6)
        .filter_map(|window| match window {
            [Token::Word(raise), Token::LParen, Token::Word(kind), Token::Comma, Token::SingleQuotedString(text), Token::RParen]
                if raise.value.eq_ignore_ascii_case("RAISE")
                    && ["ABORT", "FAIL", "ROLLBACK"]
                        .iter()
                        .any(|k| kind.value.eq_ignore_ascii_case(k)) =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
}

/// Whole migration scripts are bounded by the 1 MiB input cap; this bounds
/// tokenizer output. Individual statements keep the 4,096-token parser bound.
const MAX_SCRIPT_TOKENS: usize = 262_144;

/// Split a PostgreSQL script into its top-level statements at tokenizer-level
/// semicolons: quoted and dollar-quoted text stays intact, and a SQL-standard
/// routine body (`BEGIN ATOMIC ... END`) stays one statement. Statements with
/// only whitespace and comments are dropped. `None` when the script cannot be
/// tokenized within the token bound.
fn statements(sql: &str) -> Option<Vec<Vec<sqlparser::tokenizer::TokenWithSpan>>> {
    use sqlparser::tokenizer::{Token, Tokenizer};
    let dialect = PostgreSqlDialect {};
    let tokens = match Tokenizer::new(&dialect, sql).tokenize_with_location() {
        Ok(t) if t.len() <= MAX_SCRIPT_TOKENS => t,
        _ => return None,
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
    Some(statements)
}

/// PostgreSQL migrations run as complete scripts inside one transaction: in
/// disposable scratch for review, then on the target after approval. Analysis
/// splits on tokenizer-level semicolons (quoted and dollar-quoted text stays
/// intact), rejects statements that would escape that transaction or session,
/// and reports which statements it could not analyze. It does not sandbox SQL.
fn postgres_script(a: &mut Analysis, sql: &str, schema: &str) {
    use sqlparser::tokenizer::Token;
    let dialect = PostgreSqlDialect {};
    let Some(statements) = statements(sql).filter(|s| !s.is_empty()) else {
        a.replay_supported = false;
        a.unknowns.push("SQL_PARSE_EMPTY_OR_LIMIT".into());
        return;
    };
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
/// A name qualified with `pg_temp`, the session's temporary schema.
fn temporary(n: &ObjectName) -> bool {
    matches!(n.0.first().and_then(|p| p.as_ident()), Some(s) if n.0.len() == 2 && folded(&s.value, s.quote_style.is_some()) == "pg_temp")
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
        let Some(table) = managed_table(name, schema) else {
            // A table in another schema, such as one the history creates: its
            // lock is not predicted, and the analysis says so.
            if !temporary(name) {
                a.unknowns.push("LOCKS_NOT_PREDICTED".into());
            }
            return None;
        };
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
/// How PostgreSQL folds a name: unquoted letters lowercase (ASCII letters
/// only, as in a UTF-8 database), quoted names verbatim.
fn folded(value: &str, quoted: bool) -> String {
    if quoted {
        value.to_owned()
    } else {
        value.to_ascii_lowercase()
    }
}

/// Schema names PostgreSQL or NTNT reserve. A history that creates one never
/// brings it into the managed scope.
fn reserved_schema(name: &str) -> bool {
    name.starts_with("pg_") || name == "information_schema" || name == "_ntnt"
}

/// One script's top-level `CREATE SCHEMA <name>` names and `ALTER SCHEMA <a>
/// RENAME TO <b>` pairs, read from its tokens, so a statement sqlparser does
/// not parse (`CREATE SCHEMA audit CREATE TABLE ...`) still counts. A schema
/// named only by `AUTHORIZATION <role>`, a qualified or an over-long name
/// counts as none. DO and routine bodies are opaque.
fn schema_changes(sql: &str) -> (Vec<String>, Vec<(String, String)>) {
    use sqlparser::tokenizer::Token;
    let (mut created, mut renamed) = (vec![], vec![]);
    for tokens in statements(sql).unwrap_or_default() {
        let t: Vec<&Token> = tokens
            .iter()
            .map(|t| &t.token)
            .filter(|t| !matches!(t, Token::Whitespace(_)))
            .collect();
        let keyword = |i: usize, k: &str| matches!(t.get(i), Some(Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(k));
        let name = |i: usize| match t.get(i) {
            Some(Token::Word(w))
                if !matches!(t.get(i + 1), Some(Token::Period))
                    && (w.quote_style.is_some()
                        || !w.value.eq_ignore_ascii_case("AUTHORIZATION")) =>
            {
                Some(folded(&w.value, w.quote_style.is_some()))
                    .filter(|n| !n.is_empty() && n.len() <= 63 && !n.contains('\0'))
            }
            _ => None,
        };
        if keyword(0, "CREATE") && keyword(1, "SCHEMA") {
            let at = if keyword(2, "IF") && keyword(3, "NOT") && keyword(4, "EXISTS") {
                5
            } else {
                2
            };
            created.extend(name(at));
        } else if keyword(0, "ALTER")
            && keyword(1, "SCHEMA")
            && keyword(3, "RENAME")
            && keyword(4, "TO")
        {
            if let (Some(from), Some(to)) = (name(2), name(5)) {
                renamed.push((from, to));
            }
        }
    }
    (created, renamed)
}

/// The schemas a PostgreSQL migration history adds to the managed scope:
/// each name a top-level `CREATE SCHEMA` in any captured migration gives, and
/// each name an `ALTER SCHEMA ... RENAME TO` gives one of those. Sorted;
/// never the managed schema itself or a reserved name. Derived from the
/// captured source alone, so plan, verify and migrate inspect the same
/// schemas, before any replay, and the input digest binds them.
pub(crate) fn history_schemas<'a>(
    scripts: impl IntoIterator<Item = &'a str>,
    schema: &str,
) -> Vec<String> {
    let mut scope = std::collections::BTreeSet::new();
    let mut renames = vec![];
    for sql in scripts {
        let (created, renamed) = schema_changes(sql);
        scope.extend(created);
        renames.extend(renamed);
    }
    // Follow chains of renames (a to b, then b to c) to a fixed point.
    loop {
        let before = scope.len();
        for (from, to) in &renames {
            if scope.contains(from) {
                scope.insert(to.clone());
            }
        }
        if scope.len() == before {
            break;
        }
    }
    scope
        .into_iter()
        .filter(|s| s != schema && !reserved_schema(s))
        .collect()
}

/// What one parsed statement creates, changes or drops, by the schema its
/// name gives explicitly: (folded schema, name as written). Unqualified names
/// are not listed: they resolve in the managed schema, the search_path every
/// migration starts with. `None` for a statement kind this does not read.
fn targets(statement: &Statement) -> Option<Vec<(String, String)>> {
    fn parts(n: &ObjectName) -> Option<Vec<&Ident>> {
        n.0.iter().map(|p| p.as_ident()).collect()
    }
    // An object's schema: `schema.object`, or `database.schema.object`.
    fn object(n: &ObjectName) -> Option<(String, String)> {
        let ids = parts(n)?;
        let schema = match ids.as_slice() {
            [s, _] | [_, s, _] => s,
            _ => return None,
        };
        Some((
            folded(&schema.value, schema.quote_style.is_some()),
            n.to_string(),
        ))
    }
    // A schema itself.
    fn schema(n: &ObjectName) -> Option<(String, String)> {
        let ids = parts(n)?;
        let [.., s] = ids.as_slice() else { return None };
        (ids.len() <= 2).then(|| (folded(&s.value, s.quote_style.is_some()), n.to_string()))
    }
    fn grants(objects: Option<&GrantObjects>) -> Vec<(String, String)> {
        match objects {
            Some(
                GrantObjects::AllSequencesInSchema { schemas }
                | GrantObjects::AllTablesInSchema { schemas }
                | GrantObjects::AllViewsInSchema { schemas }
                | GrantObjects::AllMaterializedViewsInSchema { schemas }
                | GrantObjects::AllExternalTablesInSchema { schemas }
                | GrantObjects::AllFunctionsInSchema { schemas }
                | GrantObjects::FutureTablesInSchema { schemas }
                | GrantObjects::FutureViewsInSchema { schemas }
                | GrantObjects::FutureExternalTablesInSchema { schemas }
                | GrantObjects::FutureMaterializedViewsInSchema { schemas }
                | GrantObjects::FutureSequencesInSchema { schemas }
                | GrantObjects::Schemas(schemas),
            ) => schemas.iter().filter_map(schema).collect(),
            Some(
                GrantObjects::Sequences(names)
                | GrantObjects::Tables(names)
                | GrantObjects::Views(names),
            ) => names.iter().filter_map(object).collect(),
            Some(GrantObjects::Procedure { name, .. } | GrantObjects::Function { name, .. }) => {
                object(name).into_iter().collect()
            }
            _ => vec![],
        }
    }
    let one = |n: &ObjectName| object(n).into_iter().collect::<Vec<_>>();
    Some(match statement {
        Statement::CreateTable(t) => one(&t.name),
        Statement::CreateView(v) => one(&v.name),
        // An index, trigger or policy lives with its table.
        Statement::CreateIndex(i) => one(&i.table_name),
        Statement::CreateTrigger(t) => one(&t.table_name),
        Statement::CreatePolicy(p) => one(&p.table_name),
        Statement::CreateSequence { name, .. }
        | Statement::CreateProcedure { name, .. }
        | Statement::CreateType { name, .. }
        | Statement::AlterIndex { name, .. }
        | Statement::AlterView { name, .. } => one(name),
        Statement::CreateFunction(f) => one(&f.name),
        Statement::CreateDomain(d) => one(&d.name),
        Statement::CreateCollation(c) => one(&c.name),
        Statement::CreateExtension(e) => e
            .schema
            .iter()
            .map(|s| (folded(&s.value, s.quote_style.is_some()), s.to_string()))
            .collect(),
        Statement::CreateSchema {
            schema_name: SchemaName::Simple(n) | SchemaName::NamedAuthorization(n, _),
            ..
        } => schema(n).into_iter().collect(),
        Statement::AlterTable(t) => one(&t.name),
        Statement::AlterFunction(f) => one(&f.function.name),
        Statement::AlterType(t) => one(&t.name),
        Statement::AlterPolicy(p) => one(&p.table_name),
        Statement::AlterSchema(s) => schema(&s.name).into_iter().collect(),
        Statement::Drop {
            object_type: ObjectType::Schema,
            names,
            ..
        } => names.iter().filter_map(schema).collect(),
        Statement::Drop { names, .. } => names.iter().filter_map(object).collect(),
        Statement::DropFunction(d) => d.func_desc.iter().filter_map(|f| object(&f.name)).collect(),
        Statement::DropProcedure { proc_desc, .. } => {
            proc_desc.iter().filter_map(|f| object(&f.name)).collect()
        }
        Statement::DropTrigger(t) => t.table_name.iter().filter_map(object).collect(),
        Statement::DropPolicy(p) => one(&p.table_name),
        Statement::DropDomain(d) => one(&d.name),
        Statement::Truncate(t) => t
            .table_names
            .iter()
            .filter_map(|t| object(&t.name))
            .collect(),
        Statement::Grant(g) => grants(g.objects.as_ref()),
        Statement::Revoke(r) => grants(r.objects.as_ref()),
        // SELECT ... INTO creates a table.
        Statement::Query(q) => match &*q.body {
            SetExpr::Select(select) if select.into.is_some() => select
                .into
                .iter()
                .flat_map(|into| one(&into.name))
                .collect(),
            _ => return None,
        },
        _ => return None,
    })
}

/// Schemas a migration may never act on: `_ntnt`, which holds NTNT's
/// ledger, and PostgreSQL's system schemas (`pg_catalog`,
/// `information_schema`, `pg_toast` and other `pg_*` names). `pg_temp`, the
/// session's own temporary schema, is not one.
fn protected_schema(name: &str) -> bool {
    reserved_schema(name) && name != "pg_temp"
}

/// How a statement names a schema it may act in (see `schema_names`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Via {
    /// A qualified name, an `IN SCHEMA` list or `SET SCHEMA` in a statement
    /// NTNT parses.
    Name,
    /// `SET search_path`, `SET SCHEMA '...'` or `set_config('search_path',
    /// ...)`: later unqualified names resolve there.
    SearchPath,
    /// A dotted name, or a name after `SCHEMA`, in a statement NTNT does not
    /// parse or does not read names from. It may be a column of an alias.
    Token,
}

/// Leading words of statements that read or write rows, run code or change
/// settings rather than create, change, drop, grant or revoke objects: their
/// dotted names are not read as schemas they act in.
const ROW_STATEMENTS: &[&str] = &[
    "SELECT",
    "WITH",
    "VALUES",
    "TABLE",
    "INSERT",
    "UPDATE",
    "DELETE",
    "MERGE",
    "CALL",
    "DO",
    "SET",
    "RESET",
    "SHOW",
    "EXPLAIN",
    "LOCK",
    "ANALYZE",
    "NOTIFY",
    "LISTEN",
    "UNLISTEN",
    "PREPARE",
    "EXECUTE",
    "DEALLOCATE",
    "DECLARE",
    "FETCH",
    "MOVE",
    "CLOSE",
];

/// Whether `t` is the unquoted keyword `k`.
fn keyword(t: Option<&&sqlparser::tokenizer::Token>, k: &str) -> bool {
    matches!(t, Some(sqlparser::tokenizer::Token::Word(w)) if w.quote_style.is_none() && w.value.eq_ignore_ascii_case(k))
}

/// The names in a list-valued setting such as `search_path`, as
/// `set_config` reads them: comma-separated, a double-quoted name verbatim
/// (`""` for `"`), any other lowercased.
fn setting_list(value: &str) -> Vec<String> {
    let mut names = vec![];
    let mut chars = value.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let mut name = String::new();
        if chars.next_if_eq(&'"').is_some() {
            loop {
                match chars.next() {
                    Some('"') if chars.next_if_eq(&'"').is_some() => name.push('"'),
                    Some('"') | None => break,
                    Some(c) => name.push(c),
                }
            }
            while chars.next_if(|c| *c != ',').is_some() {}
        } else {
            while let Some(c) = chars.next_if(|c| *c != ',') {
                name.push(c);
            }
            name = name.trim_end().to_ascii_lowercase();
        }
        if !name.is_empty() {
            names.push(name);
        }
        if chars.next().is_none() {
            return names;
        }
    }
}

/// The schemas one statement's significant tokens put on the search path:
/// `SET [SESSION | LOCAL] search_path {TO | =} a, b` (a string literal is one
/// name, verbatim), `SET [SESSION | LOCAL] SCHEMA 'a'`, and each
/// `set_config('search_path', 'a, b', ...)` call with literal arguments.
/// `DEFAULT` names none.
fn search_path_schemas(t: &[&sqlparser::tokenizer::Token]) -> Vec<String> {
    use sqlparser::tokenizer::Token;
    let mut schemas = vec![];
    if keyword(t.first(), "SET") {
        let mut i = 1;
        while keyword(t.get(i), "SESSION") || keyword(t.get(i), "LOCAL") {
            i += 1;
        }
        let list = if keyword(t.get(i), "search_path")
            && (keyword(t.get(i + 1), "TO") || matches!(t.get(i + 1), Some(Token::Eq)))
        {
            Some(i + 2)
        } else if keyword(t.get(i), "SCHEMA") {
            Some(i + 1)
        } else {
            None
        };
        if let Some(mut i) = list.filter(|i| !keyword(t.get(*i), "DEFAULT")) {
            loop {
                match t.get(i) {
                    Some(Token::Word(w)) => schemas.push(folded(&w.value, w.quote_style.is_some())),
                    // PostgreSQL takes a number as a name too (`TO 42`).
                    Some(Token::Number(n, _)) => schemas.push(n.clone()),
                    Some(token) => match string_literal(token) {
                        Some(s) => schemas.push(s),
                        None => break,
                    },
                    None => break,
                }
                if !matches!(t.get(i + 1), Some(Token::Comma)) {
                    break;
                }
                i += 2;
            }
        }
    }
    for (i, token) in t.iter().enumerate() {
        if keyword(Some(token), "set_config")
            && matches!(t.get(i + 1), Some(Token::LParen))
            && t.get(i + 2)
                .and_then(|token| string_literal(token))
                .is_some_and(|name| name.eq_ignore_ascii_case("search_path"))
            && matches!(t.get(i + 3), Some(Token::Comma))
        {
            // A value that is not a literal (an expression) names nothing
            // NTNT can read.
            if let Some(list) = t.get(i + 4).and_then(|token| string_literal(token)) {
                schemas.extend(setting_list(&list));
            }
        }
    }
    schemas
}

/// The value of a string-literal token, whatever its spelling: `'...'`,
/// `E'...'`, `N'...'`, `U&'...'`, `$$...$$` or `$tag$...$tag$`.
fn string_literal(token: &sqlparser::tokenizer::Token) -> Option<String> {
    use sqlparser::tokenizer::Token;
    match token {
        Token::SingleQuotedString(s)
        | Token::EscapedStringLiteral(s)
        | Token::NationalStringLiteral(s)
        | Token::UnicodeStringLiteral(s) => Some(s.clone()),
        Token::DollarQuotedString(d) => Some(d.value.clone()),
        _ => None,
    }
}

/// Whether the dotted name at `i` names a column: it follows
/// `COMMENT ON COLUMN` or `OWNED BY`, where two parts are table.column.
fn names_a_column(t: &[&sqlparser::tokenizer::Token], i: usize) -> bool {
    (i >= 3
        && keyword(t.get(i - 3), "COMMENT")
        && keyword(t.get(i - 2), "ON")
        && keyword(t.get(i - 1), "COLUMN"))
        || (i >= 2 && keyword(t.get(i - 2), "OWNED") && keyword(t.get(i - 1), "BY"))
}

/// The name after `SCHEMA` at `i`, skipping `IF [NOT] EXISTS`, and any
/// further names a comma-separated `IN SCHEMA a, b` list gives, as (folded,
/// as written). An unqualified name only; `AUTHORIZATION` names none.
fn names_after_schema(t: &[&sqlparser::tokenizer::Token], mut i: usize) -> Vec<(String, String)> {
    use sqlparser::tokenizer::Token;
    if keyword(t.get(i), "IF") {
        i += if keyword(t.get(i + 1), "NOT") { 3 } else { 2 };
    }
    let mut names = vec![];
    while let Some(Token::Word(w)) = t.get(i) {
        if matches!(t.get(i + 1), Some(Token::Period))
            || (w.quote_style.is_none() && w.value.eq_ignore_ascii_case("AUTHORIZATION"))
        {
            break;
        }
        names.push((folded(&w.value, w.quote_style.is_some()), t[i].to_string()));
        if !matches!(t.get(i + 1), Some(Token::Comma)) {
            break;
        }
        i += 2;
    }
    names
}

/// Schemas a statement names by token shape, as (folded, as written): the
/// first part of each dotted name (`public.cleanup`, `"Audit".log.id`) and
/// each name after the keyword `SCHEMA` (`IN SCHEMA public`, `SCHEMA public`).
/// A column name after `COMMENT ON COLUMN` or `OWNED BY` names a schema only
/// with three parts (`public.log.id`); two are table.column.
fn token_schemas(t: &[&sqlparser::tokenizer::Token]) -> Vec<(String, String)> {
    use sqlparser::tokenizer::Token;
    let mut found = vec![];
    for i in 0..t.len() {
        let Token::Word(w) = t[i] else { continue };
        if matches!(t.get(i + 1), Some(Token::Period))
            && matches!(t.get(i + 2), Some(Token::Word(_)))
            && !(i > 0 && matches!(t[i - 1], Token::Period))
            && (!names_a_column(t, i)
                || (matches!(t.get(i + 3), Some(Token::Period))
                    && matches!(t.get(i + 4), Some(Token::Word(_)))))
        {
            found.push((
                folded(&w.value, w.quote_style.is_some()),
                format!("{}.{}", t[i], t[i + 2]),
            ));
        }
        if keyword(Some(&t[i]), "SCHEMA") {
            found.extend(names_after_schema(t, i + 1));
        }
    }
    found
}

/// A statement's significant tokens before the SQL-standard routine body
/// (`BEGIN ATOMIC ... END`) of a `CREATE [OR REPLACE] FUNCTION` or
/// `PROCEDURE`, or all of them. PostgreSQL parses such a body when it defines
/// the routine and runs it only when the routine is called, as it does a
/// quoted body. The body is the routine's last clause, so it runs to the end
/// of the statement, `CASE ... END` inside it included.
fn before_routine_body<'a, 'b>(
    t: &'b [&'a sqlparser::tokenizer::Token],
) -> &'b [&'a sqlparser::tokenizer::Token] {
    let kind = if keyword(t.get(1), "OR") && keyword(t.get(2), "REPLACE") {
        3
    } else {
        1
    };
    if !keyword(t.first(), "CREATE")
        || !(keyword(t.get(kind), "FUNCTION") || keyword(t.get(kind), "PROCEDURE"))
    {
        return t;
    }
    let body = (kind + 1..t.len())
        .find(|&i| keyword(t.get(i), "BEGIN") && keyword(t.get(i + 1), "ATOMIC"))
        .unwrap_or(t.len());
    &t[..body]
}

/// The schemas each top-level statement may create, change, drop, grant or
/// revoke objects in, as (statement number, folded schema, name as written,
/// how it is named):
/// - a statement NTNT parses: each target it names with a schema
///   (`targets`), such as `public.legacy` in `DROP TABLE public.legacy` or
///   `public` in `GRANT ... ON ALL TABLES IN SCHEMA public`, and the schema
///   an `ALTER ... SET SCHEMA` moves an object to;
/// - in any statement: each schema `SET search_path`, `SET SCHEMA '...'` or
///   `set_config('search_path', ...)` puts on the search path, where later
///   unqualified names resolve;
/// - any other statement, one sqlparser does not parse included, unless it
///   only reads or writes rows (`ROW_STATEMENTS`): the schemas its tokens
///   name (`token_schemas`).
///
/// Text inside DO and routine bodies is opaque.
fn schema_names(sql: &str) -> Vec<(usize, String, String, Via)> {
    use sqlparser::tokenizer::Token;
    let dialect = PostgreSqlDialect {};
    let mut found = vec![];
    for (index, tokens) in statements(sql).unwrap_or_default().into_iter().enumerate() {
        let number = index + 1;
        let t: Vec<&Token> = tokens
            .iter()
            .map(|t| &t.token)
            .filter(|t| !matches!(t, Token::Whitespace(_)))
            .collect();
        // A routine body runs when the routine is called, not here.
        let outer = before_routine_body(&t);
        for schema in search_path_schemas(outer) {
            found.push((number, schema.clone(), schema, Via::SearchPath));
        }
        let rows = matches!(t.first(), Some(Token::Word(w)) if w.quote_style.is_none() && ROW_STATEMENTS.iter().any(|k| w.value.eq_ignore_ascii_case(k)));
        let moved: Vec<(String, String)> = (0..t.len())
            .filter(|&i| i > 0 && keyword(t.get(i), "SET") && keyword(t.get(i + 1), "SCHEMA"))
            .flat_map(|i| names_after_schema(&t, i + 2))
            .collect();
        let shapes = if rows { vec![] } else { token_schemas(outer) };
        let parsed = (tokens.len() <= 4096)
            .then(|| {
                Parser::new(&dialect)
                    .with_recursion_limit(64)
                    .with_tokens_with_locations(tokens)
                    .parse_statements()
                    .ok()
            })
            .flatten();
        let named = match parsed.as_deref() {
            Some([statement]) => targets(statement),
            _ => None,
        };
        match named {
            Some(named) => {
                for (schema, written) in named.into_iter().chain(moved) {
                    found.push((number, schema, written, Via::Name));
                }
            }
            None => {
                for (schema, written) in shapes {
                    found.push((number, schema, written, Via::Token));
                }
            }
        }
    }
    found.dedup();
    found
}

/// Top-level statements that act on `_ntnt`, the schema holding NTNT's
/// ledger, or on a PostgreSQL system schema (`protected_schema`), as
/// (statement number, name as written): a statement that names its target
/// there (`DROP TABLE IF EXISTS _ntnt.schema_migrations_v2`, `GRANT ... ON
/// SCHEMA pg_catalog`) or moves an object there, one that puts `_ntnt` on the
/// search path, and a `_ntnt.<name>` or `SCHEMA _ntnt` in a statement NTNT
/// does not parse or read names from (`schema_names`). Such a statement can be
/// a no-op in scratch, which has no `_ntnt` and whose role cannot change
/// system schemas, yet act on the target, so it is refused by name before
/// it runs. `pg_catalog` on the search path or in SQL NTNT does not parse is
/// a reference, not a change. DO and routine bodies are opaque here: replay
/// refuses a `_ntnt` they create.
pub(crate) fn protected_targets(sql: &str) -> Vec<(usize, String)> {
    schema_names(sql)
        .into_iter()
        .filter(|(_, schema, _, via)| match via {
            Via::Name => protected_schema(schema),
            Via::SearchPath | Via::Token => schema == "_ntnt",
        })
        .map(|(number, _, written, _)| (number, written))
        .collect()
}

/// The statements of a PostgreSQL migration that may create, change, drop,
/// grant or revoke objects in a schema outside the managed scope: neither
/// `schema` nor one the history creates (`created`), `pg_catalog` or
/// `pg_temp`. Statements on `_ntnt` and system schemas are refused instead
/// (`protected_targets`). One entry per statement, schema and way of naming
/// it, in statement order.
pub(crate) fn outside_scope(sql: &str, schema: &str, created: &[String]) -> Vec<OutsideScope> {
    let mut found: Vec<OutsideScope> = schema_names(sql)
        .into_iter()
        .filter(|(_, named, _, _)| {
            named != schema
                && !created.contains(named)
                && named != "pg_temp"
                && !protected_schema(named)
        })
        .map(|(statement, schema, _, via)| OutsideScope {
            statement,
            schema,
            via,
        })
        .collect();
    found.dedup();
    found
}

/// `analyze_for`, and for PostgreSQL the statements that may act outside the
/// managed scope (`outside_scope`), with the hazard `OUTSIDE_MANAGED_SCOPE`.
/// Like a destructive change, this is for the reviewer: the plan stays
/// reviewable and nothing is refused. Migrations that act only in the
/// managed scope analyze exactly as `analyze_for` does.
pub(crate) fn analyze_in_scope(
    engine: &str,
    sql: &str,
    schema: &str,
    created: &[String],
) -> Analysis {
    let mut a = analyze_for(engine, sql, schema);
    if engine == "postgresql" {
        a.outside_scope = outside_scope(sql, schema, created);
        if !a.outside_scope.is_empty() {
            a.hazards.push("OUTSIDE_MANAGED_SCOPE".into());
            a.hazards.sort();
            a.hazards.dedup();
        }
    }
    a
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
                "CREATE INDEX CONCURRENTLY i ON app.t(id); CREATE TABLE app.x(id int)",
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
    fn locks_on_tables_in_other_schemas_are_not_predicted() {
        for sql in [
            "ALTER TABLE audit.log ADD COLUMN x int",
            "CREATE INDEX i ON audit.log (id)",
            "INSERT INTO audit.log VALUES (1)",
            "CREATE TABLE app.t (id int REFERENCES audit.log (id))",
        ] {
            let a = analyze(sql, "app");
            assert!(a.locks.iter().all(|l| l.relation != "log"), "{sql}: {a:?}");
            assert!(
                a.unknowns.iter().any(|u| u == "LOCKS_NOT_PREDICTED"),
                "{sql}: {a:?}"
            );
        }
        // The session's temporary tables are not another schema's.
        let temp = analyze("INSERT INTO pg_temp.scratch VALUES (1)", "app");
        assert!(
            !temp.unknowns.iter().any(|u| u == "LOCKS_NOT_PREDICTED"),
            "{temp:?}"
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
    #[test]
    fn the_history_creates_the_schemas_its_create_schema_statements_name() {
        let scripts = [
            "CREATE TABLE app.t (id int); CREATE SCHEMA audit; CREATE TABLE audit.log (id int);",
            // Folded like PostgreSQL; elements sqlparser does not parse still count.
            "create schema IF NOT EXISTS Reports CREATE TABLE r (id int); CREATE SCHEMA \"Mixed\"\"Case\" AUTHORIZATION CURRENT_USER;",
            // A rename brings the new name of a created schema into scope.
            "ALTER SCHEMA audit RENAME TO audit_archive; ALTER SCHEMA audit_archive RENAME TO history;",
            // Never: the managed schema, reserved names, an AUTHORIZATION-only
            // name, a qualified name, a rename of a schema the history did not
            // create, or text inside a DO body or a string.
            "CREATE SCHEMA app; CREATE SCHEMA _ntnt; CREATE SCHEMA IF NOT EXISTS information_schema; CREATE SCHEMA AUTHORIZATION someone; CREATE SCHEMA db.other; ALTER SCHEMA shared RENAME TO mine;",
            "DO $$ BEGIN CREATE SCHEMA hidden; END $$; SELECT 'CREATE SCHEMA quoted';",
        ];
        assert_eq!(
            history_schemas(scripts, "app"),
            [
                "Mixed\"Case",
                "audit",
                "audit_archive",
                "history",
                "reports"
            ]
        );
        assert!(history_schemas(["CREATE TABLE t (id int);"], "app").is_empty());
        // A history that renames before it creates still ends at one fixed point.
        assert_eq!(
            history_schemas(
                [
                    "ALTER SCHEMA b RENAME TO c",
                    "ALTER SCHEMA a RENAME TO b",
                    "CREATE SCHEMA a"
                ],
                "app"
            ),
            ["a", "b", "c"]
        );
    }
    #[test]
    fn statements_acting_on_the_ledger_or_a_system_schema_are_listed() {
        // Other schemas however they are reached, pg_temp, references to
        // pg_catalog, rows read from _ntnt and DO bodies are not listed.
        for sql in [
            "CREATE TABLE app.t (id int); CREATE TABLE public.leak (id int); DROP TABLE IF EXISTS public.legacy",
            "GRANT SELECT ON ALL TABLES IN SCHEMA public TO PUBLIC; REVOKE CREATE ON SCHEMA public FROM PUBLIC",
            "CREATE TEMP TABLE scratch (id int); CREATE TABLE pg_temp.x (id int)",
            "SET LOCAL search_path TO pg_catalog, public; SELECT pg_catalog.set_config('search_path', 'public', true)",
            "ALTER SEQUENCE IF EXISTS public.s RESTART WITH 5; DROP ROUTINE IF EXISTS public.cleanup()",
            "CREATE TABLE app.t (id pg_catalog.int4 DEFAULT pg_catalog.length('x'))",
            "SELECT pg_catalog.now(); INSERT INTO app.t SELECT id FROM _ntnt.schema_migrations_v2",
            "DO $$ BEGIN DROP TABLE IF EXISTS _ntnt.schema_migrations_v2; END $$",
            "CREATE SCHEMA audit; CREATE SCHEMA IF NOT EXISTS \"_NTNT\"; SET search_path TO DEFAULT",
        ] {
            assert!(protected_targets(sql).is_empty(), "{sql}: {:?}", protected_targets(sql));
        }
        for (sql, expected) in [
            (
                "CREATE TABLE app.t (id int); CREATE SCHEMA _ntnt",
                vec![(2, "_ntnt")],
            ),
            ("CREATE SCHEMA IF NOT EXISTS _NTNT", vec![(1, "_NTNT")]),
            (
                "DROP TABLE IF EXISTS _ntnt.schema_migrations_v2",
                vec![(1, "_ntnt.schema_migrations_v2")],
            ),
            (
                "ALTER TABLE IF EXISTS \"_ntnt\".schema_migrations_v2 ADD COLUMN x int",
                vec![(1, "\"_ntnt\".schema_migrations_v2")],
            ),
            ("ALTER TABLE app.t SET SCHEMA _ntnt", vec![(1, "_ntnt")]),
            ("SET LOCAL search_path TO app, _ntnt", vec![(1, "_ntnt")]),
            ("SET search_path = '_ntnt'", vec![(1, "_ntnt")]),
            ("SET LOCAL SCHEMA '_ntnt'", vec![(1, "_ntnt")]),
            (
                "SELECT set_config('search_path', 'app, \"_ntnt\"', true)",
                vec![(1, "_ntnt")],
            ),
            // Not parsed by sqlparser: read from tokens.
            (
                "ALTER SEQUENCE IF EXISTS _ntnt.positions RESTART WITH 5",
                vec![(1, "_ntnt.positions")],
            ),
            (
                "GRANT EXECUTE ON ALL ROUTINES IN SCHEMA app, _ntnt TO PUBLIC",
                vec![(1, "_ntnt")],
            ),
            (
                "GRANT USAGE ON SCHEMA pg_catalog TO PUBLIC",
                vec![(1, "pg_catalog")],
            ),
            (
                "REVOKE SELECT ON ALL TABLES IN SCHEMA information_schema FROM PUBLIC",
                vec![(1, "information_schema")],
            ),
            (
                "CREATE FUNCTION pg_catalog.f() RETURNS int LANGUAGE sql AS 'SELECT 1'",
                vec![(1, "pg_catalog.f")],
            ),
            (
                "CREATE EXTENSION pgcrypto WITH SCHEMA pg_catalog",
                vec![(1, "pg_catalog")],
            ),
            ("CREATE TABLE pg_toast.t (id int)", vec![(1, "pg_toast.t")]),
            (
                "SELECT * INTO _ntnt.copy FROM app.t",
                vec![(1, "_ntnt.copy")],
            ),
            // Parsed, but its names are read from tokens.
            (
                "COMMENT ON TABLE _ntnt.schema_migrations_v2 IS 'x'",
                vec![(1, "_ntnt.schema_migrations_v2")],
            ),
        ] {
            let expected: Vec<(usize, String)> = expected
                .into_iter()
                .map(|(n, s)| (n, s.to_string()))
                .collect();
            assert_eq!(protected_targets(sql), expected, "{sql}");
        }
    }
    /// A SQL-standard routine body (`BEGIN ATOMIC ... END`) runs only when
    /// the routine is called, like a quoted body: the schemas it names or
    /// puts on the search path are not listed. A top-level `set_config` call
    /// still is, after such a body too (review on #435, analysis.rs).
    #[test]
    fn routine_bodies_are_opaque_to_the_scope_scan() {
        let created = ["audit".to_string()];
        let flags = |sql: &str| serde_json::to_value(outside_scope(sql, "app", &created)).unwrap();
        let body = "CREATE FUNCTION app.saved_path() RETURNS text LANGUAGE SQL BEGIN ATOMIC SELECT pg_catalog.set_config('search_path', '_ntnt', true); END;";
        for sql in [
            body,
            // CASE ... END inside the body does not end it.
            "CREATE OR REPLACE FUNCTION app.f(x int) RETURNS text LANGUAGE sql BEGIN ATOMIC SELECT CASE WHEN x > 0 THEN set_config('search_path', 'public', true) END; SELECT count(*)::text FROM public.legacy; SELECT set_config('search_path', '_ntnt', true); END",
            "CREATE PROCEDURE app.p() LANGUAGE sql BEGIN ATOMIC INSERT INTO app.t SELECT id FROM _ntnt.schema_migrations_v2; END",
        ] {
            assert_eq!(protected_targets(sql), [], "{sql}");
            assert_eq!(flags(sql), serde_json::json!([]), "{sql}");
        }
        for (sql, statement) in [
            (
                "SELECT pg_catalog.set_config('search_path', '_ntnt', true);".to_owned(),
                1,
            ),
            (
                format!("{body} SELECT pg_catalog.set_config('search_path', '_ntnt', true);"),
                2,
            ),
        ] {
            assert_eq!(
                protected_targets(&sql),
                [(statement, "_ntnt".to_string())],
                "{sql}"
            );
        }
        assert_eq!(
            flags(&format!(
                "{body} SELECT set_config('search_path', 'public', true)"
            )),
            serde_json::json!([{"statement": 2, "schema": "public", "via": "search_path"}])
        );
    }
    #[test]
    fn statements_outside_the_managed_scope_are_flagged() {
        let created = ["audit".to_string()];
        let flags = |sql: &str| serde_json::to_value(outside_scope(sql, "app", &created)).unwrap();
        // The managed scope, pg_catalog, pg_temp, rows, DO bodies, references
        // and unqualified names are not flagged; system schemas are refused.
        for sql in [
            "CREATE TABLE app.t (id int); CREATE TABLE t2 (id int); CREATE SCHEMA audit; CREATE TABLE audit.log (id int)",
            "CREATE TEMP TABLE x (id int); CREATE TABLE pg_temp.y (id int); CREATE EXTENSION pgcrypto",
            "INSERT INTO app.t SELECT id FROM public.legacy; UPDATE public.legacy SET id = 1; SELECT public.f()",
            "DO $$ BEGIN CREATE TABLE public.leak (id int); END $$",
            "SET LOCAL search_path TO app, audit, pg_catalog, pg_temp; SET search_path TO DEFAULT",
            "GRANT SELECT ON app.t TO PUBLIC; COMMENT ON TABLE app.t IS 'x'; ALTER TABLE app.t RENAME TO t2",
            "GRANT USAGE ON SCHEMA pg_catalog TO PUBLIC; DROP TABLE IF EXISTS _ntnt.schema_migrations_v2",
        ] {
            assert_eq!(flags(sql), serde_json::json!([]), "{sql}");
        }
        for (sql, expected) in [
            (
                "CREATE TABLE app.t (id int); CREATE TABLE public.audit_log (id int)",
                serde_json::json!([[2, "public", "name"]]),
            ),
            (
                "DROP TABLE IF EXISTS public.legacy, app.t, \"Shared\".x",
                serde_json::json!([[1, "public", "name"], [1, "Shared", "name"]]),
            ),
            (
                "REVOKE CREATE ON SCHEMA Public FROM PUBLIC",
                serde_json::json!([[1, "public", "name"]]),
            ),
            (
                "GRANT SELECT ON ALL TABLES IN SCHEMA public, audit TO PUBLIC",
                serde_json::json!([[1, "public", "name"]]),
            ),
            (
                "CREATE EXTENSION pgcrypto WITH SCHEMA public",
                serde_json::json!([[1, "public", "name"]]),
            ),
            (
                "ALTER FUNCTION app.f() SET SCHEMA public",
                serde_json::json!([[1, "public", "name"]]),
            ),
            (
                "SET LOCAL search_path TO public; DROP TABLE IF EXISTS legacy",
                serde_json::json!([[1, "public", "search_path"]]),
            ),
            (
                "SELECT set_config('search_path', 'reporting, app', true)",
                serde_json::json!([[1, "reporting", "search_path"]]),
            ),
            // SQL sqlparser does not parse, or that NTNT does not read names from.
            (
                "ALTER SEQUENCE IF EXISTS public.s RESTART WITH 5",
                serde_json::json!([[1, "public", "token"]]),
            ),
            (
                "DROP ROUTINE IF EXISTS public.cleanup()",
                serde_json::json!([[1, "public", "token"]]),
            ),
            (
                "GRANT EXECUTE ON ALL ROUTINES IN SCHEMA public TO PUBLIC",
                serde_json::json!([[1, "public", "token"]]),
            ),
            (
                "ALTER VIEW IF EXISTS public.v RENAME TO v2",
                serde_json::json!([[1, "public", "token"]]),
            ),
            (
                "COMMENT ON TABLE public.legacy IS 'shared'",
                serde_json::json!([[1, "public", "token"]]),
            ),
            (
                "CREATE EXTENSION hstore SCHEMA public",
                serde_json::json!([[1, "public", "token"]]),
            ),
            (
                "ALTER TABLE app.t SET SCHEMA public",
                serde_json::json!([[1, "public", "token"]]),
            ),
            (
                "SELECT * INTO public.copy FROM app.t; SELECT * INTO app.copy FROM app.t",
                serde_json::json!([[1, "public", "name"]]),
            ),
        ] {
            let expected: Vec<serde_json::Value> = expected
                .as_array()
                .unwrap()
                .iter()
                .map(|e| serde_json::json!({"statement": e[0], "schema": e[1], "via": e[2]}))
                .collect();
            assert_eq!(flags(sql), serde_json::json!(expected), "{sql}");
        }
        // A flagged migration gains a hazard; any other analyzes exactly as
        // before, so plans of histories that stay in the managed scope keep
        // their digests.
        let flagged = analyze_in_scope(
            "postgresql",
            "REVOKE CREATE ON SCHEMA public FROM PUBLIC",
            "app",
            &created,
        );
        assert!(
            flagged.hazards.iter().any(|h| h == "OUTSIDE_MANAGED_SCOPE"),
            "{flagged:?}"
        );
        for sql in [
            "CREATE TABLE app.t (id int)",
            "CREATE TABLE audit.log (id int)",
        ] {
            assert_eq!(
                serde_json::to_value(analyze_in_scope("postgresql", sql, "app", &created)).unwrap(),
                serde_json::to_value(analyze_for("postgresql", sql, "app")).unwrap(),
                "{sql}"
            );
        }
        let sqlite = analyze_in_scope("sqlite", "CREATE TABLE t (id INTEGER)", "main", &[]);
        assert!(sqlite.outside_scope.is_empty());
    }
    #[test]
    fn search_path_settings_are_read_like_postgresql() {
        let t = |sql: &str| {
            let tokens = statements(sql).unwrap().remove(0);
            let t: Vec<&sqlparser::tokenizer::Token> = tokens
                .iter()
                .map(|t| &t.token)
                .filter(|t| !matches!(t, sqlparser::tokenizer::Token::Whitespace(_)))
                .collect();
            search_path_schemas(&t)
        };
        assert_eq!(
            t("SET search_path TO Public, \"Mixed Case\", 'Quoted'"),
            ["public", "Mixed Case", "Quoted"]
        );
        assert_eq!(t("set session search_path = app"), ["app"]);
        assert_eq!(t("SET SCHEMA 'audit'"), ["audit"]);
        assert!(t("SET search_path TO DEFAULT").is_empty());
        assert!(t("SET statement_timeout = 5").is_empty());
        assert_eq!(
            t("SELECT pg_catalog.set_config('search_path', ' Public , \"A\"\"B\", app', true), set_config('search_path', 'x', false)"),
            ["public", "A\"B", "app", "x"]
        );
        // A value that is not a literal names nothing NTNT can read.
        assert!(t("SELECT set_config('search_path', current_setting('x'), true)").is_empty());
        assert!(t("SELECT set_config('work_mem', 'public', true)").is_empty());
        assert_eq!(setting_list(""), Vec::<String>::new());
        assert_eq!(setting_list("a,,b"), ["a", "b"]);
    }
    #[test]
    fn statement_splitting_keeps_its_bounds_and_boundaries() {
        // An empty or comment-only script has no statements; neither does one
        // over the token bound, which reports None.
        assert_eq!(statements("  -- nothing\n").map(|s| s.len()), Some(0));
        assert!(statements(&"x ".repeat(MAX_SCRIPT_TOKENS)).is_none());
        let split = statements("CREATE FUNCTION app.f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; END; SELECT ';'; ;").unwrap();
        assert_eq!(split.len(), 2);
        let empty = analyze("-- nothing", "app");
        assert!(!empty.replay_supported);
        assert_eq!(empty.unknowns, ["SQL_PARSE_EMPTY_OR_LIMIT"]);
    }
    #[test]
    fn a_lone_named_concurrent_index_runs_outside_the_transaction() {
        let sql =
            "-- build without blocking writes\nCREATE INDEX CONCURRENTLY t_id ON app.t (id);\n";
        let a = analyze(sql, "app");
        assert!(a.replay_supported, "{a:?}");
        assert!(a.non_transactional, "{a:?}");
        assert_eq!(locks(sql), vec![pair("t", "ShareUpdateExclusiveLock")]);
        assert!(a.lock_warnings.is_empty(), "{a:?}");
        let ci = concurrent_index(sql, "app").unwrap();
        assert_eq!(ci.name, "t_id");
        assert!(
            !ci.replay_sql.to_uppercase().contains("CONCURRENTLY"),
            "{}",
            ci.replay_sql
        );
        assert!(ci.replay_sql.contains("CREATE INDEX"), "{}", ci.replay_sql);
        // Anything else stays rejected: other statements, no name, another schema.
        for sql in [
            "CREATE INDEX CONCURRENTLY t_id ON app.t(id); CREATE TABLE app.x(id int);",
            "CREATE INDEX CONCURRENTLY ON app.t(id);",
            "CREATE INDEX CONCURRENTLY t_id ON other.t(id);",
        ] {
            assert!(concurrent_index(sql, "app").is_none(), "{sql}");
            assert!(!analyze(sql, "app").replay_supported, "{sql}");
        }
        assert!(!analyze("CREATE INDEX t_id ON app.t(id)", "app").non_transactional);
    }

    #[test]
    fn a_migration_of_comments_and_semicolons_is_empty() {
        for engine in ["sqlite", "postgresql"] {
            for sql in [
                "",
                "  \n\t",
                "-- only a comment\n",
                "/* block */ ; ;",
                "-- a\n/* b; c */\n;",
            ] {
                assert!(is_empty(sql, engine), "{engine}: {sql:?}");
            }
            for sql in [
                "SELECT 1",
                "-- comment\nCREATE TABLE t(id integer);",
                "SELECT '--';",
                "x",
            ] {
                assert!(!is_empty(sql, engine), "{engine}: {sql:?}");
            }
            // Input that does not tokenize is left to the other checks.
            assert!(!is_empty("'unterminated", engine));
        }
    }

    /// After `COMMENT ON COLUMN` and `OWNED BY` a two-part name is
    /// table.column and names no schema; a three-part name still names one
    /// (#420, review r4654889033).
    #[test]
    fn column_names_name_a_schema_only_with_three_parts() {
        let schemas = |sql: &str| {
            outside_scope(sql, "app", &[])
                .into_iter()
                .map(|o| (o.statement, o.schema))
                .collect::<Vec<_>>()
        };
        for sql in [
            "COMMENT ON COLUMN accounts.id IS 'x'",
            "COMMENT ON COLUMN app.accounts.id IS 'x'",
            "COMMENT ON COLUMN \"Accounts\".\"Id\" IS 'x'",
            "ALTER SEQUENCE s OWNED BY accounts.id",
            "ALTER SEQUENCE app.s OWNED BY app.accounts.id",
            "ALTER SEQUENCE s OWNED BY NONE",
        ] {
            assert_eq!(schemas(sql), vec![], "{sql}");
        }
        for sql in [
            "COMMENT ON COLUMN public.audit_log.msg IS 'x'",
            "ALTER SEQUENCE app.s OWNED BY public.audit_log.id",
        ] {
            assert_eq!(schemas(sql), vec![(1, "public".to_string())], "{sql}");
        }
    }

    /// Every string-literal spelling of a search_path value is read
    /// (`E'...'`, `$$...$$`, `$tag$...$tag$`, `N'...'`, `U&'...'`), as is a
    /// number, and the ledger schema is refused however it is spelled (#420,
    /// review r4654889040).
    #[test]
    fn every_spelling_of_a_search_path_value_is_read() {
        let schemas = |sql: &str| {
            outside_scope(sql, "app", &[])
                .into_iter()
                .map(|o| (o.statement, o.schema))
                .collect::<Vec<_>>()
        };
        for sql in [
            "SET LOCAL search_path TO E'public'",
            "SET search_path TO $$public$$",
            "SET search_path = $tag$public$tag$, app",
            "SET search_path TO N'public'",
            "SET search_path TO U&'public'",
            "SELECT set_config(E'search_path', $$public$$, true)",
        ] {
            assert_eq!(schemas(sql), vec![(1, "public".to_string())], "{sql}");
        }
        assert_eq!(
            schemas("SET search_path TO 42"),
            vec![(1, "42".to_string())]
        );
        for sql in [
            "SET LOCAL search_path TO E'_ntnt', app",
            "SET search_path TO $$_ntnt$$",
            "SELECT set_config('search_path', E'_ntnt', true)",
        ] {
            assert_eq!(
                protected_targets(sql),
                vec![(1, "_ntnt".to_string())],
                "{sql}"
            );
        }
    }
}
