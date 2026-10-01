//! Real parser output must reach the optimized execution branches, not merely
//! their recognizers. Value assertions alone also pass on the slow fallback.
use super::*;
use crate::{lexer::Lexer, parser::Parser};

fn run(interpreter: &mut Interpreter, source: &str) -> Result<Value> {
    let program = Parser::new(Lexer::new(source).collect()).parse()?;
    interpreter.eval(&program)
}

#[test]
fn array_append_dispatch_runs_on_parsed_for_and_while_loops() {
    let mut interpreter = Interpreter::new();
    let result = run(
        &mut interpreter,
        r#"
let mut rows = []
for i in 0..3 { rows = rows + [i] }
let mut i = 3
while i < 5 {
    rows = rows + [i]
    i = i + 1
}
rows
"#,
    )
    .unwrap();
    assert_eq!(interpreter.fast_path_hits.array_append, 5);
    assert!(
        matches!(result, Value::Array(ref values) if values.iter().enumerate().all(|(i, v)| matches!(v, Value::Int(n) if *n == i as i64)) && values.len() == 5)
    );
}

#[test]
fn string_concat_dispatch_runs_on_parsed_loops_and_chains() {
    let mut interpreter = Interpreter::new();
    let result = run(
        &mut interpreter,
        r#"
let mut text = ""
let piece = "y"
for i in 0..3 { text = text + "x" }
for i in 0..2 { text = text + piece + "z" }
let mut i = 0
while i < 2 {
    text = text + "!"
    i = i + 1
}
text
"#,
    )
    .unwrap();
    assert_eq!(interpreter.fast_path_hits.string_concat, 7);
    assert!(matches!(result, Value::String(ref s) if s == "xxxyzyz!!"));
}

#[test]
fn append_dispatch_rejects_side_effects_and_value_producing_assignments() {
    let mut interpreter = Interpreter::new();
    let result = run(
        &mut interpreter,
        r#"
let mut text = "a"
let mut rows = [1]
fn suffix() { text = "z"; return "b" }
fn item() { rows = [9]; return 2 }
for i in 0..1 {
    text = text + suffix()
    rows = rows + [item()]
}
let string_value = (text = text + "c")
let array_value = (rows = rows + [3])
let values = [string_value, array_value]
values
"#,
    )
    .unwrap();
    assert_eq!(interpreter.fast_path_hits.string_concat, 0);
    assert_eq!(interpreter.fast_path_hits.array_append, 0);
    let Value::Array(values) = result else {
        panic!("expected values")
    };
    assert!(matches!(&values[0], Value::String(s) if s == "abc"));
    assert!(
        matches!(&values[1], Value::Array(rows) if matches!(rows.as_slice(), [Value::Int(1), Value::Int(2), Value::Int(3)]))
    );
}

#[test]
fn len_binding_dispatch_runs_on_parsed_collection_and_parent_bindings() {
    let mut interpreter = Interpreter::new();
    let result = run(
        &mut interpreter,
        r#"
let rows = [1, 2, 3]
let labels = map { "a": 1, "b": 2 }
let word = "éx"
fn parent_length() { return len(rows) }
[len(rows), len(labels), len(word), parent_length()]
"#,
    )
    .unwrap();
    assert_eq!(interpreter.fast_path_hits.len_binding, 4);
    assert!(
        matches!(result, Value::Array(ref values) if matches!(values.as_slice(), [Value::Int(3), Value::Int(2), Value::Int(3), Value::Int(3)]))
    );
}

#[test]
fn len_binding_dispatch_preserves_fallback_and_shadowing() {
    let mut interpreter = Interpreter::new();
    let literal = run(&mut interpreter, "len([1, 2])").unwrap();
    assert!(matches!(literal, Value::Int(2)));
    let shadowed = run(
        &mut interpreter,
        r#"
fn len(value) { return 99 }
let rows = [1, 2, 3]
len(rows)
"#,
    )
    .unwrap();
    assert!(matches!(shadowed, Value::Int(99)));
    assert_eq!(interpreter.fast_path_hits.len_binding, 0);

    let mut interpreter = Interpreter::new();
    let error = run(&mut interpreter, "let n = 123\nlen(n)").unwrap_err();
    assert!(error.to_string().contains("len() requires a collection"));
    assert_eq!(interpreter.fast_path_hits.len_binding, 1);
}

#[test]
fn file_import_dispatch_reuses_exact_and_canonical_exports() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("module.tnt");
    std::fs::write(&path, "export fn answer() { return 42 }").unwrap();
    let canonical = path.canonicalize().unwrap();
    let import = format!(
        "import {{ answer }} from {}\nanswer",
        serde_json::to_string(canonical.to_str().unwrap()).unwrap()
    );
    let mut interpreter = Interpreter::new();
    interpreter.set_current_file(dir.path().join("app.tnt").to_str().unwrap());
    let Value::Function {
        closure: original, ..
    } = run(&mut interpreter, &import).unwrap()
    else {
        panic!("expected exported function")
    };
    assert_eq!(interpreter.fast_path_hits.import_exact, 0);
    assert_eq!(interpreter.fast_path_hits.import_canonical, 0);
    // A reload would fail to parse. Cache hits must retain the original closure.
    std::fs::write(&path, "fn broken(").unwrap();
    let Value::Function { closure: exact, .. } = run(&mut interpreter, &import).unwrap() else {
        panic!("expected cached function")
    };
    assert!(Rc::ptr_eq(&original, &exact));
    assert_eq!(interpreter.fast_path_hits.import_exact, 1);
    for source in ["./module", "./module.tnt"] {
        let source = format!("import {{ answer }} from \"{source}\"\nanswer");
        let Value::Function {
            closure: cached, ..
        } = run(&mut interpreter, &source).unwrap()
        else {
            panic!("expected cached function")
        };
        assert!(Rc::ptr_eq(&original, &cached));
    }
    assert_eq!(interpreter.fast_path_hits.import_canonical, 2);
    assert!(matches!(
        run(&mut interpreter, "answer()").unwrap(),
        Value::Int(42)
    ));
}

// is_production_mode() uses a process-global OnceLock. Execute these two modes
// in fresh processes even under ordinary `cargo test`, not only nextest.
fn template_cache_mode(mode: &str, test_name: &str) {
    const CHILD: &str = "NTNT_CACHE_PATH_TEST_CHILD";
    if std::env::var(CHILD).as_deref() != Ok(test_name) {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(CHILD, test_name)
            .env("NTNT_ENV", mode)
            .status()
            .unwrap();
        assert!(status.success(), "{mode} template cache child failed");
        return;
    }
    assert_eq!(is_production_mode(), mode == "production");
    let dir = tempfile::tempdir().unwrap();
    let views = dir.path().join("views");
    let partials = views.join("partials");
    std::fs::create_dir_all(&partials).unwrap();
    let page = views.join("page.html");
    let card = partials.join("card.html");
    std::fs::write(&page, "Hello {{name}} {{> card}}").unwrap();
    std::fs::write(&card, "Partial {{name}}").unwrap();
    let mut interpreter = Interpreter::new();
    interpreter.set_current_file(dir.path().join("app.tnt").to_str().unwrap());
    let first = run(
        &mut interpreter,
        r#"template("views/page.html", map { "name": "A" })"#,
    )
    .unwrap();
    assert!(matches!(first, Value::String(ref s) if s == "Hello A Partial A"));
    let cached: HashMap<_, _> = interpreter
        .external_template_cache
        .iter()
        .map(|(path, entry)| (path.clone(), Rc::clone(&entry.parts)))
        .collect();
    assert_eq!(cached.len(), 2);
    let second = run(
        &mut interpreter,
        r#"template("views/page.html", map { "name": "B" })"#,
    )
    .unwrap();
    assert!(matches!(second, Value::String(ref s) if s == "Hello B Partial B"));
    for (path, original) in &cached {
        assert!(
            Rc::ptr_eq(original, &interpreter.external_template_cache[path].parts),
            "unchanged AST was reparsed: {path}"
        );
    }
    // Advance file metadata explicitly: no sleeps or filesystem clock races.
    for (path, content) in [(&page, "New {{name}} {{> card}}"), (&card, "Card {{name}}")] {
        let newer = std::fs::metadata(path).unwrap().modified().unwrap()
            + std::time::Duration::from_secs(60);
        std::fs::write(path, content).unwrap();
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(newer))
            .unwrap();
    }
    let third = run(
        &mut interpreter,
        r#"template("views/page.html", map { "name": "C" })"#,
    )
    .unwrap();
    let expected = if mode == "production" {
        "Hello C Partial C"
    } else {
        "New C Card C"
    };
    assert!(matches!(third, Value::String(ref s) if s == expected));
    for (path, original) in &cached {
        assert_eq!(
            Rc::ptr_eq(original, &interpreter.external_template_cache[path].parts),
            mode == "production",
            "incorrect {mode} cache invalidation: {path}"
        );
    }
}

#[test]
fn template_cache_dispatch_reuses_asts_in_development() {
    template_cache_mode(
        "development",
        "interpreter::fast_path_tests::template_cache_dispatch_reuses_asts_in_development",
    );
}

#[test]
fn template_cache_dispatch_reuses_asts_in_production() {
    template_cache_mode(
        "production",
        "interpreter::fast_path_tests::template_cache_dispatch_reuses_asts_in_production",
    );
}
