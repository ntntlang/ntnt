//! Interpreter-context builtins (#293): identity is the resolved value, so
//! user definitions shadow builtins and builtins are first-class.
use super::*;
use crate::{lexer::Lexer, parser::Parser};

fn run_in(dir: &std::path::Path, source: &str) -> Result<Value> {
    let main = dir.join("main.tnt");
    std::fs::write(&main, source).unwrap();
    let program = Parser::new(Lexer::new(source).collect()).parse()?;
    let mut it = Interpreter::new();
    it.set_current_file(main.to_str().unwrap());
    it.eval(&program)
}

fn run(source: &str) -> Result<Value> {
    run_in(tempfile::tempdir().unwrap().path(), source)
}

fn text(v: Result<Value>) -> String {
    match v {
        Ok(Value::String(s)) => s,
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn user_definitions_shadow_every_context_builtin() {
    let scratch = tempfile::tempdir().unwrap();
    let never = scratch.path().join("never.txt");
    let never_lit = serde_json::to_string(&never.to_string_lossy()).unwrap();
    let cases: Vec<(&str, &str, String)> = vec![
        ("filter", "a, b", "filter([1, 2], fn(x) { true })".into()),
        ("transform", "a, b", "transform([1], fn(x) { x })".into()),
        ("find", "a, b", "find([1], fn(x) { true })".into()),
        ("any", "a, b", "any([1], fn(x) { true })".into()),
        ("all", "a, b", "all([1], fn(x) { true })".into()),
        ("count", "a, b", "count([1], fn(x) { true })".into()),
        ("reduce", "a, b, c", "reduce([1], 0, fn(a, x) { a })".into()),
        ("flat_map", "a, b", "flat_map([1], fn(x) { [x] })".into()),
        ("sort", "a", "sort([2, 1])".into()),
        (
            "sort_by",
            "a, b",
            "sort_by([2, 1], fn(a, b) { a - b })".into(),
        ),
        ("sort_desc", "a", "sort_desc([1, 2])".into()),
        ("template", "a, b", "template(\"x.html\", map {})".into()),
        ("compile", "a", "compile(\"x.html\")".into()),
        ("render", "a, b", "render(map {}, map {})".into()),
        ("read_file", "p", "read_file(\"missing.txt\")".into()),
        (
            "write_file",
            "p, c",
            format!("write_file({never_lit}, \"x\")"),
        ),
        ("get", "a, b", "get(\"/x\", fn(req) { req })".into()),
        ("post", "a, b", "post(\"/x\", fn(req) { req })".into()),
        ("validate", "a, b", "validate(map {}, map {})".into()),
    ];
    for (name, params, call) in cases {
        let src = format!("fn {name}({params}) {{\n    return \"USER\"\n}}\n{call}");
        assert_eq!(text(run(&src)), "USER", "top-level fn {name}");
        let src = format!("let {name} = fn({params}) {{ \"LOCAL\" }}\n{call}");
        assert_eq!(text(run(&src)), "LOCAL", "local closure {name}");
    }
    assert!(!never.exists(), "user write_file must not write");
}

#[test]
fn imported_definitions_shadow_builtins() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.tnt"),
        "fn filter(xs, f) {\n    return \"IMPORTED\"\n}\n",
    )
    .unwrap();
    let out = run_in(
        dir.path(),
        "import { filter } from \"./lib.tnt\"\nfilter([1, 2], fn(x) { true })",
    );
    assert_eq!(text(out), "IMPORTED");
}

#[test]
fn kv_get_import_is_the_kv_function_not_a_route() {
    let out = run(
        "import { get } from \"std/kv\"\nlet r = get(\"/x\", \"k\") otherwise { return \"KV_ERR\" }\n\"NO_ERR\"",
    );
    // The KV function rejects a non-store first argument; a route
    // registration would have silently returned Unit.
    assert!(
        matches!(&out, Err(_)) || matches!(&out, Ok(Value::String(s)) if s == "KV_ERR"),
        "{out:?}"
    );
}

#[test]
fn builtins_are_first_class_values() {
    let cases = [
        ("let f = filter\nlen(f([1, 2, 3], fn(x) { x > 1 }))", 2),
        ("let xs = [1, 2, 3]\nlen(xs.filter(fn(x) { x > 1 }))", 2),
        ("fn apply(f, xs) {\n    return f(xs, fn(x) { x * 2 })\n}\napply(transform, [1, 2])[1]", 4),
        ("import { sort_by as sb } from \"std/collections\"\nsb([1, 3, 2], fn(a, b) { b - a })[0]", 3),
        ("import \"std/collections\" as c\nc.sort_by([1, 3, 2], fn(a, b) { b - a })[0]", 3),
        ("count([1, 2, 3], fn(x) { x > 1 })", 2),
        ("count(\"banana\", \"a\")", 3),
        ("reduce([1, 2, 3], 0, fn(a, x) { a + x })", 6),
    ];
    for (src, expected) in cases {
        match run(src) {
            Ok(Value::Int(n)) => assert_eq!(n, expected, "{src}"),
            other => panic!("{src}: {other:?}"),
        }
    }
}

#[test]
fn builtins_work_inside_tasks() {
    let out = run(
        "import { spawn, await_task } from \"std/concurrent\"\nlet t = spawn(fn() { len(filter([1, 2, 3], fn(x) { x > 1 })) })\nunwrap(await_task(t))",
    );
    assert!(matches!(out, Ok(Value::Int(2))), "{out:?}");
}

#[test]
fn fs_paths_resolve_relative_to_script_for_aliases_too() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data.txt"), "hello").unwrap();
    let out = run_in(
        dir.path(),
        "import { read_file, read_file as rf } from \"std/fs\"\nimport \"std/fs\" as fs\nunwrap(rf(\"data.txt\")) + unwrap(fs.read_file(\"data.txt\")) + unwrap(read_file(\"data.txt\"))",
    );
    assert_eq!(text(out), "hellohellohello");
}

#[test]
fn route_registration_requires_a_path_pattern() {
    let out = run("get(\"https://example.com\", fn(req) { req })");
    let err = out.expect_err("non-path pattern must not register silently");
    assert!(err.to_string().contains("fetch()"), "{err}");
}
