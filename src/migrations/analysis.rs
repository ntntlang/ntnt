//! PostgreSQL dialect parsing, not keyword scanning. Unknown syntax never replays.
use serde::Serialize;
use sqlparser::{ast::Statement, dialect::PostgreSqlDialect, parser::Parser};

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Analysis {
    pub replay_supported: bool,
    pub operations: Vec<String>,
    pub hazards: Vec<String>,
    pub unknowns: Vec<String>,
}
/// Parse PostgreSQL syntax within fixed token and recursion bounds.
/// Invalid, empty or unsupported input is not evidence that replay would be safe.
fn parse(sql: &str) -> Option<Vec<Statement>> {
    let dialect = PostgreSqlDialect {};
    let tokens = sqlparser::tokenizer::Tokenizer::new(&dialect, sql)
        .tokenize_with_location()
        .ok()?;
    // Bound both parser work and recursive AST destruction on all CLI stacks.
    if tokens.len() > 4096 {
        return None;
    }
    Parser::new(&dialect)
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
/// Recognize only the closed AST subset eligible for rollback-only scratch replay.
/// This predicate does not authorize target writes or establish general SQL purity.
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
    let mut a = Analysis {
        replay_supported: true,
        operations: vec![],
        hazards: vec![],
        unknowns: vec![],
    };
    let Some(statements) = parse(sql).filter(|s| !s.is_empty()) else {
        a.replay_supported = false;
        a.unknowns.push("SQL_PARSE_EMPTY_OR_LIMIT".into());
        return a;
    };
    for s in statements {
        let (operation, hazard) = match &s {
            Statement::CreateTable(_) => ("create_table", "DDL_LOCKS"),
            Statement::CreateIndex(_) => ("create_index", "INDEX_BUILD_LOCK_SCAN"),
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
            _ => ("unsupported", "UNANALYZED_EFFECTS"),
        };
        a.operations.push(operation.into());
        a.hazards.push(hazard.into());
        if !allowed(&s, schema) || a.hazards.iter().any(|h| h == "CASCADE_DEPENDENCIES") {
            a.replay_supported = false;
            a.unknowns.push("SQL_UNSUPPORTED".into());
        }
    }
    a.hazards.sort();
    a.hazards.dedup();
    a.unknowns.sort();
    a.unknowns.dedup();
    a
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_bounds_expression_depth_on_small_thread_stack() {
        let sql = format!(
            "CREATE TABLE app.t (id INTEGER CHECK({} > 0));",
            vec!["1"; 900].join(" + ")
        );
        let supported = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || analyze(&sql, "app").replay_supported)
            .unwrap()
            .join()
            .unwrap();
        assert!(!supported);
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
        for sql in [
            "SELECT pg_advisory_lock(1)",
            "DO $$ BEGIN PERFORM 1; END $$",
            "CREATE TABLE app.bad (id int DEFAULT pg_advisory_lock(1))",
            "COMMIT",
            "CREATE TABLE app.bad AS SELECT 1",
            "CREATE TABLE app.bad (id custom_type)",
        ] {
            assert!(!analyze(sql, "app").replay_supported, "{sql}");
        }
    }
}
