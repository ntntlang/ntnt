//! Explicit relative imports belong to their declaration, not their executor (#296).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn write(root: &Path, name: &str, source: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    let cwd = root.path().join("cwd");
    fs::create_dir_all(&app).unwrap();
    fs::create_dir_all(&cwd).unwrap();
    (root, app, cwd)
}

fn run(app: &Path, cwd: &Path, source: &str) -> Output {
    write(app, "main.tnt", source);
    Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["run", "../app/main.tnt"])
        .current_dir(cwd)
        .env("NTNT_ENV", "development")
        .env_remove("NTNT_STRICT")
        .env_remove("NTNT_TYPE_MODE")
        .output()
        .unwrap()
}

fn success(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    stdout.into_owned()
}

#[test]
fn deferred_imports_keep_the_declaring_file_across_calls_and_tasks() {
    let (_root, app, cwd) = fixture();
    write(&app, "lib/value.tnt", "export fn value() { \"source\" }");
    write(&app, "value.tnt", "export fn value() { \"wrong caller\" }");
    write(&cwd, "lib/value.tnt", "export fn value() { \"wrong cwd\" }");
    write(&cwd, "value.tnt", "export fn value() { \"wrong cwd\" }");
    write(
        &app,
        "lib/helpers.tnt",
        r#"export fn helper() {
    import { value } from "./value"
    value()
}
export fn callback() {
    let f = fn() {
        import { value } from "./value.tnt"
        value()
    }
    f
}"#,
    );
    let output = run(
        &app,
        &cwd,
        r#"import { spawn, after, await_task, parallel, race, schedule, cancel_schedule, channel, send, recv_timeout } from "std/concurrent"
import { helper, callback } from "./lib/helpers.tnt"
assert(helper() == "source")
let f = callback()
assert(f() == "source")
let start = spawn
assert(unwrap(await_task(start(f))) == "source")
assert(unwrap(await_task(spawn(fn() { helper() }))) == "source")
assert(unwrap(await_task(after(1, fn() {
    import { value } from "./lib/value.tnt"
    value()
}))) == "source")
let values = parallel([fn() {
    import { value } from "./lib/value.tnt"
    value()
}])
assert(unwrap(values[0]) == "source")
assert(unwrap(race([fn() {
    import { value } from "./lib/value.tnt"
    value()
}])) == "source")
assert(unwrap(await_task(spawn(fn() {
    let child = spawn(fn() {
        import { value } from "./lib/value.tnt"
        value()
    })
    unwrap(await_task(child))
}))) == "source")
let [tx, rx] = channel()
let timer = schedule(10, fn() {
    import { value } from "./lib/value.tnt"
    send(tx, value())
})
let tick = recv_timeout(rx, 2000)
cancel_schedule(timer)
assert(unwrap(tick) == "source")
print("declaration origins preserved")"#,
    );
    assert_eq!(success(&output).trim(), "declaration origins preserved");
}

#[test]
fn missing_source_relative_import_never_loads_a_cwd_decoy() {
    let (_root, app, cwd) = fixture();
    write(
        &cwd,
        "only_cwd.tnt",
        "print(\"DECOY_EVALUATED\")\nexport fn value() { 42 }",
    );
    let output = run(
        &app,
        &cwd,
        r#"import { spawn, await_task } from "std/concurrent"
print(unwrap(await_task(spawn(fn() {
    import { value } from "./only_cwd.tnt"
    value()
}))))"#,
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stdout}\n{stderr}");
    assert!(!stdout.contains("DECOY_EVALUATED"), "{stdout}");
    assert!(stderr.contains("only_cwd.tnt"), "{stderr}");
}

#[test]
fn same_relative_spelling_in_distinct_modules_has_distinct_cache_identity() {
    let (_root, app, cwd) = fixture();
    for (dir, value) in [("a", "A"), ("b", "B")] {
        write(
            &app,
            &format!("{dir}/loader.tnt"),
            "export fn load() {\nimport { value } from \"./value\"\nvalue()\n}",
        );
        write(
            &app,
            &format!("{dir}/value.tnt"),
            &format!("print(\"loaded {value}\")\nexport fn value() {{ \"{value}\" }}"),
        );
    }
    write(&app, "value.tnt", "export fn value() { \"wrong caller\" }");
    let output = run(
        &app,
        &cwd,
        r#"import { load as a } from "./a/loader.tnt"
import { load as b } from "./b/loader.tnt"
assert(a() == "A")
assert(b() == "B")
assert(a() == "A")
assert(b() == "B")"#,
    );
    assert_eq!(
        success(&output).lines().collect::<Vec<_>>(),
        ["loaded A", "loaded B"]
    );
}

#[test]
fn bare_absolute_std_lazy_and_resource_paths_keep_their_existing_meanings() {
    let (_root, app, cwd) = fixture();
    write(&app, "data.txt", "entry data");
    write(&cwd, "data.txt", "cwd data");
    write(&app, "views/page.html", "entry template");
    write(&cwd, "views/page.html", "cwd template");
    write(&cwd, "value.tnt", "export fn value() { \"cwd\" }");
    write(&app, "value.tnt", "export fn value() { \"absolute\" }");
    write(
        &app,
        "lib/resources.tnt",
        r#"export fn resources() {
    import { read_file } from "std/fs"
    unwrap(read_file("data.txt")) + ":" + template("views/page.html", map {})
}"#,
    );
    let absolute = serde_json::to_string(&app.join("value.tnt").to_string_lossy()).unwrap();
    let source = format!(
        r#"import {{ spawn, await_task }} from "std/concurrent"
import {{ resources }} from "./lib/resources.tnt"
assert(resources() == "entry data:entry template")
assert(unwrap(await_task(spawn(fn() {{ resources() }}))) == "cwd data:cwd template")
assert(unwrap(await_task(spawn(fn() {{
    import {{ value }} from "value.tnt"
    value()
}}))) == "cwd")
assert(unwrap(await_task(spawn(fn() {{
    import {{ value }} from {absolute}
    value()
}}))) == "absolute")
assert(unwrap(await_task(spawn(fn() {{
    import {{ trim }} from "std/string"
    trim(" okay ")
}}))) == "okay")
if false {{ import {{ missing }} from "./never-loaded.tnt" }}
print("unchanged controls")"#
    );
    let output = run(&app, &cwd, &source);
    assert_eq!(success(&output).trim(), "unchanged controls");
}

#[test]
fn source_less_embedders_and_ast_json_keep_their_existing_contract() {
    use ntnt::{ast::Program, interpreter::Interpreter, lexer::Lexer, parser::Parser};
    let (_root, app, _cwd) = fixture();
    write(&app, "value.tnt", "export fn value() { 42 }");
    let source = "import { value } from \"./value.tnt\"\nvalue()";
    let plain = Parser::new(Lexer::new(source).collect()).parse().unwrap();
    let anchored = Parser::new(Lexer::new(source).collect())
        .with_source_file(app.join("main.tnt"))
        .parse()
        .unwrap();
    let json = serde_json::to_string(&anchored).unwrap();
    assert_eq!(json, serde_json::to_string(&plain).unwrap());
    assert!(!json.contains(&*app.to_string_lossy()));
    let restored: Program = serde_json::from_str(&json).unwrap();
    for ast in [&plain, &restored] {
        let mut interpreter = Interpreter::new();
        // Existing embedders can continue parsing before supplying file context.
        interpreter.set_current_file(&app.join("main.tnt").to_string_lossy());
        assert!(matches!(
            interpreter.eval(ast).unwrap(),
            ntnt::interpreter::Value::Int(42)
        ));
    }
}

#[test]
fn nested_parsers_and_external_templates_keep_their_own_import_origin() {
    let (_root, app, cwd) = fixture();
    write(&app, "value.tnt", "export fn value() { \"entry\" }");
    write(
        &app,
        "views/value.tnt",
        "export fn value() { \"template\" }",
    );
    write(
        &app,
        "views/partials/value.tnt",
        "export fn value() { \"partial\" }",
    );
    write(
        &app,
        "views/page.html",
        "{{ (fn() { import { value } from \"./value.tnt\"\nvalue() })() }}",
    );
    write(
        &app,
        "views/partials/piece.html",
        "{{ (fn() { import { value } from \"./value.tnt\"\nvalue() })() }}",
    );
    write(&app, "views/partial.html", "{{> piece }}");
    let output = run(
        &app,
        &cwd,
        r##"let interpolated = "#{(fn() { import { value } from "./value.tnt"
value() })()}"
assert(interpolated == "entry")
assert(template("views/page.html", map {}) == "template")
assert(template("views/page.html", map {}) == "template")
let compiled = compile("views/page.html")
assert(render(compiled, map {}) == "template")
assert(template("views/partial.html", map {}) == "partial")
print("nested origins preserved")"##,
    );
    assert_eq!(success(&output).trim(), "nested origins preserved");
}

#[test]
fn jobs_loader_preserves_import_origin_when_draining_in_a_fresh_interpreter() {
    let (_root, app, cwd) = fixture();
    write(&app, "jobs/value.tnt", "export fn value() { \"job\" }");
    write(&cwd, "value.tnt", "export fn value() { \"wrong cwd\" }");
    write(
        &app,
        "jobs/origin.tnt",
        r#"job OriginJob on origins (retry: 0) {
    perform() {
        import { value } from "./value.tnt"
        assert(value() == "job")
        print("job origin preserved")
    }
}"#,
    );
    let output = run(
        &app,
        &cwd,
        r#"import { configure_queue, enqueue, drain_jobs } from "std/jobs"
unwrap(configure_queue(map { "mode": "testing" }))
jobs("jobs")
unwrap(enqueue("OriginJob", map {}))
assert(unwrap(drain_jobs()) == 1)"#,
    );
    assert!(success(&output).contains("job origin preserved"));
}
