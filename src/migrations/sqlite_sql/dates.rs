//! SQLite defers the purity check for its date/time builtins until a value
//! is evaluated. An empty index build therefore cannot establish that future
//! writes can maintain it. This is a refusal policy, not SQL admission: the
//! native prepare/tail, authorizer, and catalog checks still apply.
use super::{classify, statements, Kind, Tokens};
use crate::migrations::{inventory::Migration, Diagnostic, Result};
use std::sync::LazyLock;

const FUNCTIONS: &[&str] = &[
    "date",
    "time",
    "datetime",
    "julianday",
    "unixepoch",
    "strftime",
    "timediff",
];
const CLOCK: &[&str] = &["current_date", "current_time", "current_timestamp"];

/// Match only a complete SQL numeric literal (including SQLite's digit
/// separators), not a Rust float spelling such as `NaN`, which may be a column.
static NUMBER: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(
    r"(?ix)\A[+-]?(?:0x[0-9a-f](?:_?[0-9a-f])*|(?:[0-9](?:_?[0-9])*(?:\.(?:[0-9](?:_?[0-9])*)?)?|\.[0-9](?:_?[0-9])*)(?:e[+-]?[0-9](?:_?[0-9])*)?)\z"
).expect("SQLite numeric literal")
});

/// For a fixed literal safe in this argument's role, return (last, is NULL).
/// Double-quoted words can resolve to columns and are not string literals.
fn literal(t: &mut Tokens<'_>, function: &str, i: usize) -> Option<(bool, bool)> {
    let mut parens = 0;
    while t.eat("(") {
        parens += 1;
    }
    let first = t.next()?;
    let mut end = t.next()?;
    if let Some(text) = first.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        // No special time word contains an escaped quote. The first argument
        // of strftime is a format; timediff has *two* time values.
        let time_value = function == "timediff" || i == usize::from(function == "strftime");
        let special: &[&str] = if time_value {
            &["now", "subsec", "subsecond"]
        } else if function == "strftime" && i == 0 {
            &[]
        } else {
            &["localtime", "utc"]
        };
        if special.iter().any(|s| text.eq_ignore_ascii_case(s)) {
            return None;
        }
    } else if !first.eq_ignore_ascii_case("NULL") {
        // Tokens splits a sign, decimal point or exponent sign. Joining these
        // tokens cannot admit a column: require the whole numeric grammar.
        let mut number = first.to_owned();
        while end != "," && end != ")" {
            number.push_str(end);
            end = t.next()?;
        }
        if !NUMBER.is_match(&number) {
            return None;
        }
    }
    for _ in 0..parens {
        if end != ")" {
            return None;
        }
        end = t.next()?;
    }
    matches!(end, "," | ")").then(|| (end == ")", first.eq_ignore_ascii_case("NULL")))
}

fn fixed_call(mut t: Tokens<'_>, function: &str) -> bool {
    // strftime() returns NULL without a format; strftime(format) implicitly
    // uses the clock. The other variadic date functions default to 'now'.
    if t.eat(")") {
        return function == "strftime";
    }
    for i in 0.. {
        match literal(&mut t, function, i) {
            Some((true, is_null)) => return function != "strftime" || i != 0 || is_null,
            Some((false, _)) => (),
            _ => return false,
        }
    }
    unreachable!()
}

/// Inspect a CHECK/generated expression through its closing parenthesis, or
/// the entire remainder of an index (including its optional WHERE predicate).
fn expression(t: &mut Tokens<'_>, grouped: bool) -> Option<&'static str> {
    let mut depth = 0usize;
    let mut qualified = false;
    while let Some(token) = t.next() {
        match token {
            "(" => depth += 1,
            ")" if grouped && depth == 0 => break,
            ")" => depth = depth.saturating_sub(1),
            // In a stored scalar expression AS introduces a CAST type, not
            // a function call. Its type may have several words and a size.
            _ if token.eq_ignore_ascii_case("AS") => {
                let mut nested = 0;
                for v in t.by_ref() {
                    match v {
                        "(" => nested += 1,
                        ")" if nested == 0 => break,
                        ")" => nested -= 1,
                        _ => (),
                    }
                }
                depth = depth.saturating_sub(1);
            }
            _ if token.eq_ignore_ascii_case("COLLATE") => {
                t.next();
            }
            _ if !qualified => {
                let call = t.peek() == Some("(");
                let quoted = call.then(|| super::unquote(token)).flatten();
                let name = quoted.as_deref().unwrap_or(token);
                if let Some(&f) = CLOCK.iter().find(|f| name.eq_ignore_ascii_case(f)) {
                    return Some(f);
                }
                if call {
                    if let Some(&f) = FUNCTIONS.iter().find(|f| name.eq_ignore_ascii_case(f)) {
                        let mut args = *t;
                        args.next();
                        if !fixed_call(args, f) {
                            return Some(f);
                        }
                    }
                }
            }
            _ => (),
        }
        qualified = token == ".";
    }
    None
}

fn suspect(sql: &str) -> Option<&'static str> {
    let mut t = Tokens(sql);
    match classify(sql).ok()? {
        Kind::Index => {
            t.find(|v| *v == "(")?;
            expression(&mut t, false)
        }
        kind @ (Kind::Table | Kind::Alter(_)) => {
            if matches!(kind, Kind::Table) {
                t.find(|v| *v == "(")?;
            } else {
                t.next();
                t.next();
                t.name()?;
                if !(t.eat("ADD") || t.eat("ALTER")) {
                    return None;
                }
            }
            let mut depth = 0usize;
            while let Some(v) = t.next() {
                if depth == 0
                    && (v.eq_ignore_ascii_case("CHECK") || v.eq_ignore_ascii_case("AS"))
                    && t.eat("(")
                {
                    if let Some(f) = expression(&mut t, true) {
                        return Some(f);
                    }
                } else if v == "(" {
                    depth += 1;
                } else if v == ")" {
                    depth = depth.saturating_sub(1);
                }
            }
            None
        }
        _ => None,
    }
}

pub(in crate::migrations) fn validate(m: &Migration) -> Result<()> {
    for (i, stmt) in statements(&m.sql).enumerate() {
        if let Some(f) = suspect(stmt) {
            let mut d = Diagnostic::new("DB_SQL_UNSUPPORTED", &format!("Statement {}: SQLite function `{f}` in an index, CHECK or generated-column expression is not proven deterministic for future writes.", i+1));
            d.source = Some(m.name.clone());
            d.next_action = "Use fixed date/time literals without the clock or local time zone, or store the timestamp in a column at write time and use that column directly. Defaults and trigger bodies may use the clock. Then rerun plan.".into();
            return Err(d);
        }
    }
    Ok(())
}
