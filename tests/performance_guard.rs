//! Coarse regression alarms, not benchmarks. Run explicitly on Linux in CI:
//! cargo test --release --locked --test performance_guard -- --ignored --nocapture --test-threads=1
//!
//! Parsing, interpreter construction and fixture I/O are outside the timer.
//! Deterministic path-taken tests in interpreter.rs complement these generous
//! ceilings: this suite catches catastrophic slowdowns, not small timing drift.

use ntnt::interpreter::{Interpreter, Value};
use ntnt::lexer::Lexer;
use ntnt::parser::Parser;
use std::time::{Duration, Instant};

fn timed_eval(name: &str, source: &str, ceiling: Duration) -> Value {
    assert!(!cfg!(debug_assertions), "run timing guards with --release");
    let program = Parser::new(Lexer::new(source).collect()).parse().unwrap();
    let mut interpreter = Interpreter::new();
    let start = Instant::now();
    let result = interpreter.eval(&program).unwrap();
    let elapsed = start.elapsed();
    eprintln!("{name}: {elapsed:?} (ceiling {ceiling:?})");
    assert!(
        elapsed < ceiling,
        "{name} took {elapsed:?}, ceiling {ceiling:?}"
    );
    result
}

#[test]
#[ignore = "release-profile timing guard; explicitly run on Linux in CI"]
fn string_append_40k() {
    let result = timed_eval(
        "40k string appends",
        r#"
let mut text = ""
for i in 0..40000 {
    text = text + "x"
}
text
"#,
        Duration::from_millis(500),
    );
    assert!(matches!(result, Value::String(ref s) if s == &"x".repeat(40000)));
}

#[test]
#[ignore = "release-profile timing guard; explicitly run on Linux in CI"]
fn array_append_40k() {
    let result = timed_eval(
        "40k array appends",
        r#"
let mut rows = []
for i in 0..40000 {
    rows = rows + [i]
}
rows
"#,
        Duration::from_millis(500),
    );
    let Value::Array(rows) = result else {
        panic!("expected array")
    };
    assert_eq!(rows.len(), 40000);
    assert!(rows
        .iter()
        .enumerate()
        .all(|(i, v)| matches!(v, Value::Int(n) if *n == i as i64)));
}

#[test]
#[ignore = "release-profile timing guard; explicitly run on Linux in CI"]
fn cached_template_2000_expressions() {
    assert!(!cfg!(debug_assertions), "run timing guards with --release");
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("page.html"), "{{title}}".repeat(2000)).unwrap();
    let source = r#"template("page.html", map { "title": "<&>" })"#;
    let program = Parser::new(Lexer::new(source).collect()).parse().unwrap();
    let mut interpreter = Interpreter::new();
    interpreter.set_current_file(dir.path().join("app.tnt").to_str().unwrap());
    let expected = "&lt;&amp;&gt;".repeat(2000);
    // Warm the parsed-AST cache, then render 100 times (200k expressions).
    assert!(matches!(interpreter.eval(&program).unwrap(), Value::String(ref s) if s == &expected));
    let start = Instant::now();
    let mut outputs = Vec::with_capacity(100);
    for _ in 0..100 {
        outputs.push(interpreter.eval(&program).unwrap());
    }
    let elapsed = start.elapsed();
    let ceiling = Duration::from_secs(1);
    eprintln!("100 cached renders × 2000 expressions: {elapsed:?} (ceiling {ceiling:?})");
    assert!(
        elapsed < ceiling,
        "template renders took {elapsed:?}, ceiling {ceiling:?}"
    );
    assert!(outputs
        .iter()
        .all(|v| matches!(v, Value::String(s) if s == &expected)));
}
