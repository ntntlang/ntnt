//! Restore-stable catalog values (#460).
//!
//! A database restored with pg_dump and pg_restore behaves as the original,
//! but PostgreSQL can store two kinds of value that inspection compares
//! differently there:
//!
//! - Expressions. Parse analysis nests an AND (OR) as an operand of another
//!   when it expands BETWEEN, NOT BETWEEN, an IN list of columns, a row
//!   comparison or IS DISTINCT FROM between rows. A dump holds the deparsed
//!   text, `(((x >= 1) AND (x <= 5)) AND (y > 0))`, and when the grammar
//!   parses it again it merges an AND (OR) that is the first operand of
//!   another into it (`makeAndExpr`, `makeOrExpr`): the restored expression
//!   is `((x >= 1) AND (x <= 5) AND (y > 0))`. Nesting anywhere else
//!   survives a restore: `y > 0 AND x BETWEEN 1 AND 5` comes back as it was.
//! - `connoinherit` of an index-backed constraint. A partition's PRIMARY
//!   KEY, UNIQUE or EXCLUDE constraint cloned from its parent has it false;
//!   pg_dump adds the constraint with `ALTER TABLE ONLY` and attaches its
//!   index, which leaves it true. It has no effect on these constraints.
//!
//! The restore-stable value is the one a dump and restore leaves: those
//! groups merged, and an index-backed constraint's `no_inherit` as
//! PostgreSQL sets it when it creates the constraint, true unless it is
//! attached to a parent's (`local` false). A value already in that form,
//! which is every value a restore does not change, stays as it is, and so
//! do the catalogs and plans of histories that have no other.
use serde_json::Value;
use sha2::{Digest, Sha256};

/// `value`, a catalog object of `kind` as inspection reads it (owner and
/// grants canonical, definitions not yet redacted), restore-stable, and,
/// when that differs, the value as PostgreSQL stores it, which plans with
/// `canonicalization_version` 1 fingerprinted. Domain constraints and
/// routines with an SQL-standard body arrive with their text
/// (`definition`) and leave with its digest (`definition_digest`), in both.
pub(super) fn restore_stable(kind: &str, value: Value) -> (Value, Option<Value>) {
    let mut stable = value.clone();
    let changed = stabilize(kind, &mut stable);
    digest_definitions(kind, &mut stable);
    if !changed {
        return (stable, None);
    }
    let mut stored = value;
    digest_definitions(kind, &mut stored);
    (stable, Some(stored))
}

/// Make `value` restore-stable in place; whether anything changed.
fn stabilize(kind: &str, value: &mut Value) -> bool {
    let mut changed = false;
    let mut text = |value: &mut Value, key: &str| {
        // `get_mut`: a missing key must stay missing.
        if let Some(field) = value.get_mut(key) {
            if let Some(merged) = field.as_str().and_then(reassociate) {
                *field = Value::String(merged);
                changed = true;
            }
        }
    };
    match kind {
        "constraint" => {
            text(value, "definition");
            // Only PostgreSQL's own flags: never invent one.
            if matches!(value["kind"].as_str(), Some("p" | "u" | "x")) {
                if let (Some(no_inherit), Some(local)) =
                    (value["no_inherit"].as_bool(), value["local"].as_bool())
                {
                    if no_inherit != local {
                        value["no_inherit"] = Value::Bool(local);
                        changed = true;
                    }
                }
            }
        }
        "index" | "view" | "trigger" | "routine" => text(value, "definition"),
        "column" => text(value, "default"),
        "policy" => {
            text(value, "using");
            text(value, "check");
        }
        "relation" => text(value, "partition_key"),
        "type" => {
            text(value, "default");
            if let Some(constraints) = value.get_mut("constraints").and_then(Value::as_array_mut) {
                for constraint in constraints {
                    text(constraint, "definition");
                }
            }
        }
        // A security label's definition is its text, not SQL.
        "other" if value["kind"] == "rule" => text(value, "definition"),
        _ => (),
    }
    changed
}

/// Replace the definitions inspection compares by digest with it: a domain
/// constraint's, and a routine's with an SQL-standard body (the SQL hashes
/// every other routine itself), as `encode(sha256(...),'hex')` did.
fn digest_definitions(kind: &str, value: &mut Value) {
    let digest = |object: &mut Value| {
        if let Some(map) = object.as_object_mut() {
            if let Some(Value::String(text)) = map.remove("definition") {
                map.insert(
                    "definition_digest".into(),
                    Value::String(hex::encode(Sha256::digest(text.as_bytes()))),
                );
            }
        }
    };
    match kind {
        "type" => {
            if let Some(constraints) = value.get_mut("constraints").and_then(Value::as_array_mut) {
                constraints.iter_mut().for_each(digest);
            }
        }
        "routine" => digest(value),
        _ => (),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    And,
    Or,
}

/// One item at one depth of a deparsed text.
enum Item {
    /// `AND` or `OR`, in capitals.
    Op(Op),
    /// `CASE` and `END`, in capitals.
    Case,
    End,
    /// `BETWEEN`: a window frame, since deparsed expressions hold none.
    Between,
    Comma,
    Group(Group),
    /// Anything else: a name, a literal, an operator or a number.
    Atom,
}

/// A parenthesized (`paren`) or bracketed group, with the byte offsets of
/// its opening and closing characters.
struct Group {
    paren: bool,
    open: usize,
    close: usize,
    items: Vec<Item>,
}

/// `text`, an expression or definition PostgreSQL deparsed without pretty
/// printing (`pg_get_constraintdef(oid, false)`, `pg_get_expr`, ...), with
/// every parenthesized AND (OR) whose first operand is itself one merged
/// into it, as the grammar does when it parses the text again. That only
/// drops parentheses, and AND and OR are associative, so the expression
/// means what it meant. `None` when nothing changes, and when the text is
/// not certainly in that form: an unbalanced parenthesis or bracket, an
/// unterminated literal or quoted name, a `$` or a comment outside quotes,
/// or `and`, `or`, `case`, `end` or `between` not in capitals. Then it is
/// compared as stored, as before #460.
pub(super) fn reassociate(text: &str) -> Option<String> {
    let mut items = parse(text)?;
    let mut removed = vec![];
    merge(&mut items, false, &mut removed);
    if removed.is_empty() {
        return None;
    }
    removed.sort_unstable();
    let mut merged = String::with_capacity(text.len());
    let mut from = 0;
    // Each removed offset holds a one-byte parenthesis.
    for at in removed {
        merged.push_str(&text[from..at]);
        from = at + 1;
    }
    merged.push_str(&text[from..]);
    Some(merged)
}

fn word_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte >= 0x80
}
fn word_byte(byte: u8) -> bool {
    word_start(byte) || byte.is_ascii_digit()
}
fn special(byte: u8) -> bool {
    matches!(
        byte,
        b'\'' | b'"' | b'$' | b'(' | b')' | b'[' | b']' | b',' | b'-' | b'/'
    ) || byte.is_ascii_whitespace()
        || byte == 0x0b
}

/// The offset just past a literal or quoted name opened by `quote` at
/// `start`: a doubled quote is one quote, and in an escape string (`E'...'`)
/// a backslash escapes the next byte.
fn quoted(bytes: &[u8], start: usize, quote: u8, escapes: bool) -> Option<usize> {
    let mut at = start + 1;
    loop {
        match *bytes.get(at)? {
            b'\\' if escapes => at += 2,
            byte if byte == quote => {
                if bytes.get(at + 1) == Some(&quote) {
                    at += 2;
                } else {
                    return Some(at + 1);
                }
            }
            _ => at += 1,
        }
    }
}

/// The items of `text` at its top level, groups holding theirs.
fn parse(text: &str) -> Option<Vec<Item>> {
    let bytes = text.as_bytes();
    let mut open: Vec<Group> = vec![Group {
        paren: false,
        open: 0,
        close: 0,
        items: vec![],
    }];
    // The end of the last word, and whether it was `E` (an escape string's prefix).
    let mut last_word: Option<(usize, bool)> = None;
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        let start = at;
        let item = match byte {
            _ if byte.is_ascii_whitespace() || byte == 0x0b => {
                at += 1;
                continue;
            }
            b'\'' => {
                let escapes = last_word == Some((at, true));
                at = quoted(bytes, at, b'\'', escapes)?;
                Item::Atom
            }
            b'"' => {
                at = quoted(bytes, at, b'"', false)?;
                Item::Atom
            }
            // Dollar quoting or a parameter.
            b'$' => return None,
            b'-' if bytes.get(at + 1) == Some(&b'-') => return None,
            b'/' if bytes.get(at + 1) == Some(&b'*') => return None,
            b'(' | b'[' => {
                open.push(Group {
                    paren: byte == b'(',
                    open: at,
                    close: 0,
                    items: vec![],
                });
                at += 1;
                last_word = None;
                continue;
            }
            b')' | b']' => {
                if open.len() < 2 {
                    return None;
                }
                let mut group = open.pop()?;
                if group.paren != (byte == b')') {
                    return None;
                }
                group.close = at;
                at += 1;
                Item::Group(group)
            }
            b',' => {
                at += 1;
                Item::Comma
            }
            _ if word_start(byte) => {
                while at < bytes.len() && word_byte(bytes[at]) {
                    at += 1;
                }
                let word = &text[start..at];
                let item = match word {
                    "AND" => Item::Op(Op::And),
                    "OR" => Item::Op(Op::Or),
                    "CASE" => Item::Case,
                    "END" => Item::End,
                    "BETWEEN" => Item::Between,
                    _ if ["and", "or", "case", "end", "between"]
                        .iter()
                        .any(|keyword| word.eq_ignore_ascii_case(keyword)) =>
                    {
                        return None
                    }
                    _ => Item::Atom,
                };
                open.last_mut()?.items.push(item);
                last_word = Some((at, word == "E" || word == "e"));
                continue;
            }
            // Operators and numbers, up to the next word, group or literal.
            _ => {
                at += 1;
                while at < bytes.len() && !special(bytes[at]) && !word_start(bytes[at]) {
                    at += 1;
                }
                Item::Atom
            }
        };
        open.last_mut()?.items.push(item);
        last_word = None;
    }
    if open.len() != 1 {
        return None;
    }
    Some(open.pop()?.items)
}

/// The operator a parenthesized group's `items` join operands with, when
/// they are certainly `operand OP operand [OP operand ...]` with one OP, as
/// PostgreSQL deparses an AND or an OR; `None` for anything else (an
/// argument list, a window frame, an AND inside a CASE).
fn chain(items: &[Item]) -> Option<Op> {
    let mut op = None;
    let mut case = 0usize;
    let mut operand = false;
    for item in items {
        match item {
            Item::Comma | Item::Between => return None,
            Item::Op(next) => {
                if case > 0 || !operand || op.is_some_and(|op| op != *next) {
                    return None;
                }
                op = Some(*next);
                operand = false;
            }
            Item::Case => {
                case += 1;
                operand = true;
            }
            Item::End => {
                case = case.checked_sub(1)?;
                operand = true;
            }
            Item::Group(_) | Item::Atom => operand = true,
        }
    }
    if case == 0 && operand {
        op
    } else {
        None
    }
}

/// Merge, innermost first, each group in `items` whose first operand is a
/// group of the same operator; `paren` when `items` are a parenthesized
/// group's. `removed` collects the offsets of the parentheses dropped.
fn merge(items: &mut Vec<Item>, paren: bool, removed: &mut Vec<usize>) {
    for item in items.iter_mut() {
        if let Item::Group(group) = item {
            merge(&mut group.items, group.paren, removed);
        }
    }
    if !paren {
        return;
    }
    let Some(op) = chain(items) else {
        return;
    };
    while matches!(items.get(1), Some(Item::Op(next)) if *next == op)
        && matches!(items.first(), Some(Item::Group(first)) if first.paren && chain(&first.items) == Some(op))
    {
        let Item::Group(first) = items.remove(0) else {
            unreachable!("checked above")
        };
        removed.extend([first.open, first.close]);
        items.splice(0..0, first.items);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `(text, as a restore leaves it)`, PostgreSQL 16-18's deparsed text
    /// before and after pg_dump and pg_restore (#460).
    const RESTORED: &[(&str, &str)] = &[
        (
            "CHECK ((((timeout_ms >= 1000) AND (timeout_ms <= 15000)) AND ((timeout_ms % 1000) = 0)))",
            "CHECK (((timeout_ms >= 1000) AND (timeout_ms <= 15000) AND ((timeout_ms % 1000) = 0)))",
        ),
        (
            "CHECK ((((n < 5) OR (n > 9)) OR (n IS NULL)))",
            "CHECK (((n < 5) OR (n > 9) OR (n IS NULL)))",
        ),
        (
            "CHECK ((((x >= 1) AND (x <= 5)) AND ((y >= 2) AND (y <= 6))))",
            "CHECK (((x >= 1) AND (x <= 5) AND ((y >= 2) AND (y <= 6))))",
        ),
        (
            "CHECK (((((x >= 9) AND (x <= 1)) OR ((x >= 1) AND (x <= 9))) OR (y > 3)))",
            "CHECK ((((x >= 9) AND (x <= 1)) OR ((x >= 1) AND (x <= 9)) OR (y > 3)))",
        ),
        (
            "CHECK ((\nCASE\n    WHEN (((x >= 1) AND (x <= 5)) AND (y > 0)) THEN (z > 0)\n    ELSE false\nEND AND (a > 0)))",
            "CHECK ((\nCASE\n    WHEN ((x >= 1) AND (x <= 5) AND (y > 0)) THEN (z > 0)\n    ELSE false\nEND AND (a > 0)))",
        ),
        (
            "CHECK ((COALESCE((((x >= 1) AND (x <= 5)) AND (y > 0)), false) AND (b > 0)))",
            "CHECK ((COALESCE(((x >= 1) AND (x <= 5) AND (y > 0)), false) AND (b > 0)))",
        ),
        (
            "CHECK (((NOT (((x >= 1) AND (x <= 5)) AND (y > 0))) AND (c > 0)))",
            "CHECK (((NOT ((x >= 1) AND (x <= 5) AND (y > 0))) AND (c > 0)))",
        ),
        (
            "CHECK ((((x IS DISTINCT FROM a) OR (y IS DISTINCT FROM b)) OR (z IS DISTINCT FROM c)))",
            "CHECK (((x IS DISTINCT FROM a) OR (y IS DISTINCT FROM b) OR (z IS DISTINCT FROM c)))",
        ),
        (
            "CREATE INDEX t_expr ON app.t USING btree (((((x >= 1) AND (x <= 5)) AND (y > 0))))",
            "CREATE INDEX t_expr ON app.t USING btree ((((x >= 1) AND (x <= 5) AND (y > 0))))",
        ),
        (
            "CREATE INDEX t_pred ON app.t USING btree (id) WHERE (((x >= 1) AND (x <= 5)) AND (y > 0))",
            "CREATE INDEX t_pred ON app.t USING btree (id) WHERE ((x >= 1) AND (x <= 5) AND (y > 0))",
        ),
        (
            "((((now())::date >= '2026-01-01'::date) AND ((now())::date <= '2026-12-31'::date)) AND true)",
            "(((now())::date >= '2026-01-01'::date) AND ((now())::date <= '2026-12-31'::date) AND true)",
        ),
        (
            "CREATE TRIGGER tr BEFORE INSERT OR UPDATE ON app.t FOR EACH ROW WHEN ((((new.x >= 1) AND (new.x <= 5)) AND (new.y > 0))) EXECUTE FUNCTION app.trg()",
            "CREATE TRIGGER tr BEFORE INSERT OR UPDATE ON app.t FOR EACH ROW WHEN (((new.x >= 1) AND (new.x <= 5) AND (new.y > 0))) EXECUTE FUNCTION app.trg()",
        ),
        (
            " SELECT id,\n    sum(x) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING) AS w\n   FROM app.t\n  WHERE (((x >= 1) AND (x <= 5)) AND (y > 0));",
            " SELECT id,\n    sum(x) OVER (ORDER BY id ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING) AS w\n   FROM app.t\n  WHERE ((x >= 1) AND (x <= 5) AND (y > 0));",
        ),
        (
            " SELECT t.id\n   FROM (app.t\n     JOIN app.t t2 ON (((t.id = t2.id) AND ((t.x >= 1) AND (t.x <= 5)) AND (t2.y > 0))))\n  WHERE (((t.x = t.y) OR (t.x = t.z)) OR (t.x = t.a));",
            " SELECT t.id\n   FROM (app.t\n     JOIN app.t t2 ON (((t.id = t2.id) AND ((t.x >= 1) AND (t.x <= 5)) AND (t2.y > 0))))\n  WHERE ((t.x = t.y) OR (t.x = t.z) OR (t.x = t.a));",
        ),
        (
            "CREATE OR REPLACE FUNCTION app.f(x integer)\n RETURNS boolean\n LANGUAGE sql\nRETURN (((x >= 1) AND (x <= 5)) AND (x > 0))\n",
            "CREATE OR REPLACE FUNCTION app.f(x integer)\n RETURNS boolean\n LANGUAGE sql\nRETURN ((x >= 1) AND (x <= 5) AND (x > 0))\n",
        ),
        (
            "LIST (((((k >= 1) AND (k <= 5)) AND (id > 0))))",
            "LIST ((((k >= 1) AND (k <= 5) AND (id > 0))))",
        ),
    ];

    /// Texts a restore leaves as they are, which stay as they are.
    const UNCHANGED: &[&str] = &[
        // Nested, but not as the first operand.
        "CHECK (((y > 0) AND ((x >= 1) AND (x <= 5))))",
        "CHECK (((y > 0) AND ((x >= 1) AND (x <= 5)) AND (z > 0)))",
        "CHECK (((x IS DISTINCT FROM y) AND ((x >= 1) AND (x <= 5)) AND true))",
        // Literals and quoted names holding AND, OR and parentheses.
        "CHECK (((s <> 'a AND (b) OR c''s )'::text) AND ((x >= 1) AND (x <= 9)) AND (s <> 'x'::text)))",
        "CHECK (((s <> 'AND'::text) AND ((x >= 1) AND (x <= 2))))",
        "CHECK (((\"a AND (b\" > 0) AND (\"c)\" > 0)))",
        "'a\\b AND (c)'::text",
        // Different operators, NOT, argument lists, CASE, plain chains.
        "CHECK ((((a > 0) OR (b > 0)) AND (c > 0)))",
        "CHECK (((NOT ((a > 0) AND (b > 0))) AND (c > 0)))",
        "CHECK ((COALESCE(((a > 0) AND (b > 0)), false) AND (c > 0)))",
        "CHECK (((a > 0) AND (b > 0) AND (c > 0)))",
        "CHECK ((CASE WHEN ((a > 0) AND (b > 0)) THEN true ELSE false END AND (c > 0)))",
        "PRIMARY KEY (id, observed_at)",
        "FOREIGN KEY (a, b) REFERENCES app.t(a, b) ON DELETE CASCADE",
        "NOT NULL id",
        "",
    ];

    #[test]
    fn restore_reassociation_is_reproduced_exactly() {
        for (stored, restored) in RESTORED {
            assert_eq!(reassociate(stored).as_deref(), Some(*restored), "{stored}");
            // A restored text is already restore-stable.
            assert_eq!(reassociate(restored), None, "{restored}");
        }
        for text in UNCHANGED {
            assert_eq!(reassociate(text), None, "{text}");
        }
    }

    #[test]
    fn uncertain_texts_are_left_as_stored() {
        for text in [
            // Unbalanced or mismatched groups, unterminated quotes.
            "((((a) AND (b)) AND (c))",
            "(((a) AND (b)) AND (c)))",
            "(((a) AND (b]) AND (c))",
            "(((a) AND (b)) AND (c = 'x))",
            "(((a) AND (b)) AND (\"c))",
            // Dollar quoting, parameters and comments.
            "(((a) AND (b)) AND ($1 > 0))",
            "(((a) AND (b)) AND (c = $$x$$))",
            "(((a) AND (b)) AND (c > 0)) -- x",
            "(((a) AND (b)) AND (c > 0)) /* x */",
            // Keywords PostgreSQL deparses in capitals.
            "(((a) and (b)) AND (c))",
            "(((a) AND (b)) Or (c))",
            "(((a) AND (b)) AND case when x then y end)",
        ] {
            assert_eq!(reassociate(text), None, "{text}");
        }
        // Inside literals none of those matter.
        assert_eq!(
            reassociate("(((a = '$1 -- /*') AND (b = 'and')) AND (c = E'\\' AND ('))").as_deref(),
            Some("((a = '$1 -- /*') AND (b = 'and') AND (c = E'\\' AND ('))")
        );
    }

    #[test]
    fn groups_that_are_not_one_operator_chains_are_kept() {
        for text in [
            // A window frame's BETWEEN ... AND.
            "(sum(x) OVER (ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING))",
            "(((a) AND (b)) AND (c) ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING)",
            // A group mixing AND and OR is not one PostgreSQL deparses.
            "((((a) AND (b)) AND (c) OR (d)))",
            // An AND inside a CASE at the group's own depth.
            "(CASE WHEN ((a) AND (b)) AND (c) THEN x END)",
            // Argument lists and rows.
            "(((a) AND (b)), (c))",
            // A first operand that is more than the group.
            "(((a) AND (b))::boolean AND (c))",
            "(((a) AND (b)) IS TRUE AND (c))",
        ] {
            assert_eq!(reassociate(text), None, "{text}");
        }
        // The groups inside such a group are still merged.
        assert_eq!(
            reassociate(
                "(f((((a) AND (b)) AND (c)), x) OVER (ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING))"
            )
            .as_deref(),
            Some("(f(((a) AND (b) AND (c)), x) OVER (ROWS BETWEEN 1 PRECEDING AND 1 FOLLOWING))")
        );
    }

    /// A small boolean expression tree.
    #[derive(Clone, Debug)]
    enum Expr {
        Leaf(u32),
        Not(Box<Expr>),
        Bool(Op, Vec<Expr>),
        Call(Vec<Expr>),
        Case(Box<Expr>, Box<Expr>),
    }

    /// `expr` as PostgreSQL deparses it without pretty printing.
    fn deparse(expr: &Expr) -> String {
        match expr {
            Expr::Leaf(n) => match n % 5 {
                0 => format!("(c{n} > {n})"),
                1 => format!("(s <> 'x AND ({n}) OR ''y'''::text)"),
                2 => format!("(\"q AND (\" IS DISTINCT FROM {n})"),
                3 => format!("flag{n}"),
                _ => format!("(c{n} IS NULL)"),
            },
            Expr::Not(inner) => format!("(NOT {})", deparse(inner)),
            Expr::Bool(op, args) => {
                let separator = if *op == Op::And { " AND " } else { " OR " };
                let args: Vec<String> = args.iter().map(deparse).collect();
                format!("({})", args.join(separator))
            }
            Expr::Call(args) => {
                let args: Vec<String> = args.iter().map(deparse).collect();
                format!("COALESCE({}, false)", args.join(", "))
            }
            Expr::Case(when, then) => {
                format!(
                    "\nCASE\n    WHEN {} THEN {}\n    ELSE false\nEND",
                    deparse(when),
                    deparse(then)
                )
            }
        }
    }

    /// `expr` as the grammar leaves it after parsing its deparsed text:
    /// every AND (OR) whose first argument is one merged into it.
    fn restored(expr: &Expr) -> Expr {
        match expr {
            Expr::Leaf(_) => expr.clone(),
            Expr::Not(inner) => Expr::Not(Box::new(restored(inner))),
            Expr::Call(args) => Expr::Call(args.iter().map(restored).collect()),
            Expr::Case(when, then) => {
                Expr::Case(Box::new(restored(when)), Box::new(restored(then)))
            }
            Expr::Bool(op, args) => {
                let mut args: Vec<Expr> = args.iter().map(restored).collect();
                if let Expr::Bool(inner, first) = &args[0] {
                    if inner == op {
                        let first = first.clone();
                        args.splice(0..1, first);
                    }
                }
                Expr::Bool(*op, args)
            }
        }
    }

    /// A deterministic pseudo-random expression of at most `depth` levels.
    fn generate(seed: &mut u64, depth: u32) -> Expr {
        *seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let roll = (*seed >> 33) as u32;
        if depth == 0 || roll.is_multiple_of(7) {
            return Expr::Leaf(roll % 1000);
        }
        match roll % 9 {
            0 => Expr::Not(Box::new(generate(seed, depth - 1))),
            1 => Expr::Call(vec![generate(seed, depth - 1)]),
            2 => Expr::Case(
                Box::new(generate(seed, depth - 1)),
                Box::new(generate(seed, depth - 1)),
            ),
            n => {
                let op = if n.is_multiple_of(2) { Op::And } else { Op::Or };
                let count = 2 + (roll / 9 % 3) as usize;
                Expr::Bool(op, (0..count).map(|_| generate(seed, depth - 1)).collect())
            }
        }
    }

    #[test]
    fn reassociation_matches_the_grammar_on_generated_expressions() {
        let mut seed = 460;
        let mut merged = 0;
        for _ in 0..3000 {
            let expr = generate(&mut seed, 4);
            let text = format!("CHECK ({})", deparse(&expr));
            let expected = format!("CHECK ({})", deparse(&restored(&expr)));
            let actual = reassociate(&text).unwrap_or_else(|| text.clone());
            assert_eq!(actual, expected, "{expr:?}");
            // Idempotent: a restore-stable text stays as it is.
            assert_eq!(reassociate(&actual), None, "{actual}");
            merged += usize::from(actual != text);
        }
        // The generator reaches the case this is for.
        assert!(merged > 300, "{merged}");
    }

    #[test]
    fn values_are_restore_stable_and_keep_the_stored_form() {
        // A partition's primary key restored: no_inherit follows attachment.
        let restored = json!({"kind":"p","definition":"PRIMARY KEY (id, observed_at)","local":false,"no_inherit":true});
        let (stable, stored) = restore_stable("constraint", restored.clone());
        assert_eq!(stable["no_inherit"], false);
        assert_eq!(stored, Some(restored));
        for unchanged in [
            json!({"kind":"p","definition":"PRIMARY KEY (id)","local":true,"no_inherit":true}),
            json!({"kind":"u","definition":"UNIQUE (a)","local":false,"no_inherit":false}),
            // CHECK and NOT NULL keep connoinherit: it changes what they do.
            json!({"kind":"c","definition":"CHECK ((a > 0)) NO INHERIT","local":true,"no_inherit":true}),
            json!({"kind":"c","definition":"CHECK ((a > 0))","local":false,"no_inherit":false}),
            json!({"kind":"n","definition":"NOT NULL a","local":true,"no_inherit":false}),
        ] {
            assert_eq!(
                restore_stable("constraint", unchanged.clone()),
                (unchanged, None)
            );
        }
        // Expressions by kind and field; a missing field stays missing.
        let nested = "(((a > 0) AND (b > 0)) AND (c > 0))";
        let flat = "((a > 0) AND (b > 0) AND (c > 0))";
        for (kind, key) in [
            ("constraint", "definition"),
            ("index", "definition"),
            ("view", "definition"),
            ("trigger", "definition"),
            ("column", "default"),
            ("policy", "using"),
            ("policy", "check"),
            ("relation", "partition_key"),
            ("type", "default"),
        ] {
            let (stable, stored) = restore_stable(kind, json!({ key: nested, "kind": "x" }));
            assert_eq!(stable, json!({ key: flat, "kind": "x" }), "{kind}.{key}");
            assert_eq!(
                stored,
                Some(json!({ key: nested, "kind": "x" })),
                "{kind}.{key}"
            );
        }
        assert_eq!(
            restore_stable("policy", json!({"using": nested})).0,
            json!({"using": flat})
        );
        // Free text is never SQL: comments, labels, a security label.
        for (kind, value) in [
            ("relation", json!({"comment": nested})),
            (
                "other",
                json!({"kind":"security_label","definition": nested}),
            ),
            ("type", json!({"labels": [nested]})),
        ] {
            assert_eq!(restore_stable(kind, value.clone()), (value, None), "{kind}");
        }
        assert_eq!(
            restore_stable("other", json!({"kind":"rule","definition": nested})).0,
            json!({"kind":"rule","definition": flat})
        );
    }

    #[test]
    fn domain_constraints_and_sql_bodies_are_compared_by_digest() {
        let digest = |text: &str| hex::encode(Sha256::digest(text.as_bytes()));
        let nested = "CHECK ((((VALUE >= 1) AND (VALUE <= 100)) AND ((VALUE % 2) = 0)))";
        let flat = "CHECK (((VALUE >= 1) AND (VALUE <= 100) AND ((VALUE % 2) = 0)))";
        let domain = json!({"kind":"d","constraints":[{"name":"small_check","kind":"c","validated":true,"definition":nested}]});
        let (stable, stored) = restore_stable("type", domain);
        assert_eq!(
            stable["constraints"],
            json!([{"name":"small_check","kind":"c","validated":true,"definition_digest":digest(flat)}])
        );
        assert_eq!(
            stored.unwrap()["constraints"],
            json!([{"name":"small_check","kind":"c","validated":true,"definition_digest":digest(nested)}])
        );
        // Unchanged, the digest is the one the SQL computed before #460.
        let plain = json!({"kind":"d","constraints":[{"name":"c","kind":"c","validated":true,"definition":"CHECK ((VALUE > 0))"}]});
        let (stable, stored) = restore_stable("type", plain);
        assert_eq!(
            stable["constraints"][0]["definition_digest"],
            digest("CHECK ((VALUE > 0))")
        );
        assert_eq!(stable["constraints"][0].get("definition"), None);
        assert_eq!(stored, None);
        // A routine with an SQL-standard body; any other keeps the SQL's digest.
        let body = "CREATE OR REPLACE FUNCTION app.f(x integer)\n RETURNS boolean\n LANGUAGE sql\nRETURN (((x >= 1) AND (x <= 5)) AND (x > 0))\n";
        let (stable, stored) = restore_stable(
            "routine",
            json!({"definition_digest":null,"definition":body}),
        );
        assert_eq!(
            stable,
            json!({"definition_digest":digest(&reassociate(body).unwrap())})
        );
        assert_eq!(stored, Some(json!({"definition_digest":digest(body)})));
        let other = json!({"definition_digest":"ab","language":"plpgsql"});
        assert_eq!(restore_stable("routine", other.clone()), (other, None));
    }
}
