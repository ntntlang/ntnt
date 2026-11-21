//! Runtime errors raised in another file are reported against that file (#412).
//!
//! Each case writes a small app to a temp dir, runs `ntnt run` on it and checks
//! the `--> file:line` header, the source row printed under it and the call-site
//! notes. Bare imports (`"lib/x"`) resolve from the working directory, so these
//! apps run from their own directory unless a case says otherwise.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Hang guard for one `ntnt run` of a script that fails straight away.
const RUN_HANG_GUARD: Duration = Duration::from_secs(120);

struct Run {
    stdout: String,
    stderr: String,
    code: Option<i32>,
}

fn write(root: &Path, name: &str, source: &str) {
    let path = root.join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, source).unwrap();
}

fn app() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app");
    fs::create_dir_all(&app).unwrap();
    (root, app)
}

/// `ntnt run <entry>` from `cwd`. Output goes to files, so a full pipe cannot
/// block the child.
fn run_in(cwd: &Path, entry: &str) -> Run {
    let out = tempfile::tempdir().unwrap();
    let stdout_path = out.path().join("stdout");
    let stderr_path = out.path().join("stderr");
    let mut child = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["run", entry])
        .current_dir(cwd)
        .env("NTNT_ENV", "development")
        .env("NO_COLOR", "1")
        .env_remove("CLICOLOR_FORCE")
        .env_remove("NTNT_STRICT")
        .env_remove("NTNT_TYPE_MODE")
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout_path).unwrap())
        .stderr(fs::File::create(&stderr_path).unwrap())
        .spawn()
        .expect("spawn ntnt run");
    let deadline = Instant::now() + RUN_HANG_GUARD;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("hang guard: `ntnt run {entry}` still running after {RUN_HANG_GUARD:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    Run {
        stdout: fs::read_to_string(&stdout_path).unwrap(),
        stderr: fs::read_to_string(&stderr_path).unwrap(),
        code: status.code(),
    }
}

fn run(app: &Path) -> Run {
    run_in(app, "main.tnt")
}

/// A relative path as `ntnt` prints it on this platform.
fn shown(parts: &[&str]) -> String {
    parts
        .iter()
        .collect::<PathBuf>()
        .to_string_lossy()
        .into_owned()
}

fn header(path: &str, line: usize) -> String {
    format!("  --> {path}:{line}\n")
}

fn source_row(line: usize, text: &str) -> String {
    format!(" {line:>3} | {text}\n")
}

fn note(verb: &str, path: &str, line: usize) -> String {
    format!("  note: {verb} from {path}:{line}\n")
}

/// The error report names `path:line`, shows that line of that file, and
/// lists exactly `notes` as call sites, in order.
fn assert_reported(run: &Run, path: &str, line: usize, text: &str, notes: &[String]) {
    let all = format!("stdout:\n{}\nstderr:\n{}", run.stdout, run.stderr);
    assert_eq!(run.code, Some(1), "{all}");
    assert!(run.stderr.contains(&header(path, line)), "{all}");
    assert_eq!(run.stderr.matches("  --> ").count(), 2, "{all}");
    assert!(run.stderr.contains(&source_row(line, text)), "{all}");
    let listed: Vec<&str> = run
        .stderr
        .lines()
        .filter(|row| {
            row.starts_with("  note: called from ") || row.starts_with("  note: imported from ")
        })
        .collect();
    let expected: Vec<&str> = notes.iter().map(|n| n.trim_end_matches('\n')).collect();
    assert_eq!(listed, expected, "{all}");
}

const CHECKS: &str = r#"// lib/checks.tnt
// Padding so the failing line number is clearly
// different from any line in main.tnt.
//
//
//
//
fn refuse_startup() {
    return unwrap(Err("startup refused"))
}
"#;

#[test]
fn imported_function_error_names_the_module_line_and_call_site() {
    // The #412 repro.
    let (_root, app) = app();
    write(&app, "lib/checks.tnt", CHECKS);
    write(
        &app,
        "main.tnt",
        "import { refuse_startup } from \"lib/checks\"\nprint(\"starting\")\nrefuse_startup()\n",
    );
    let run = run(&app);
    assert!(run.stdout.contains("starting"), "{}", run.stdout);
    assert_reported(
        &run,
        &shown(&["lib", "checks.tnt"]),
        9,
        "    return unwrap(Err(\"startup refused\"))",
        &[note("called", "main.tnt", 3)],
    );
    assert!(!run.stderr.contains("main.tnt:9"), "{}", run.stderr);
}

#[test]
fn nested_import_error_names_the_innermost_module_and_each_call_site() {
    let (_root, app) = app();
    write(
        &app,
        "lib/a.tnt",
        "import { inner } from \"./b\"\n// a pad\nfn outer() {\n    // pad\n    inner()\n}\n",
    );
    write(
        &app,
        "lib/b.tnt",
        "// b pad 1\n// b pad 2\n// b pad 3\nfn inner() {\n    let parts = split(\"a,b\", \",\")\n    return unwrap(Err(\"inner failed\"))\n}\n",
    );
    write(
        &app,
        "main.tnt",
        "import { outer } from \"lib/a\"\nprint(\"starting\")\nouter()\n",
    );
    let run = run(&app);
    assert_reported(
        &run,
        &shown(&["lib", "b.tnt"]),
        6,
        "    return unwrap(Err(\"inner failed\"))",
        &[
            note("called", &shown(&["lib", "a.tnt"]), 5),
            note("called", "main.tnt", 3),
        ],
    );
}

#[test]
fn lambda_returned_by_a_module_function_names_the_module() {
    let (_root, app) = app();
    write(
        &app,
        "lib/factory.tnt",
        "// factory pad\n// pad\nfn make_checker() {\n    let label = \"checker\"\n    return fn(x) {\n        return unwrap(Err(\"lambda refused\"))\n    }\n}\n",
    );
    write(
        &app,
        "main.tnt",
        "import { make_checker } from \"lib/factory\"\nlet check = make_checker()\nprint(\"made\")\ncheck(1)\n",
    );
    let run = run(&app);
    assert_reported(
        &run,
        &shown(&["lib", "factory.tnt"]),
        6,
        "        return unwrap(Err(\"lambda refused\"))",
        &[note("called", "main.tnt", 4)],
    );
}

#[test]
fn module_level_lambda_binding_names_the_module() {
    let (_root, app) = app();
    write(
        &app,
        "lib/handlers.tnt",
        "// handlers pad\n// pad\nlet on_start = fn() {\n    let ready = false\n    return unwrap(Err(\"handler refused\"))\n}\n",
    );
    write(
        &app,
        "main.tnt",
        "import { on_start } from \"lib/handlers\"\nprint(\"registered\")\non_start()\n",
    );
    let run = run(&app);
    assert_reported(
        &run,
        &shown(&["lib", "handlers.tnt"]),
        5,
        "    return unwrap(Err(\"handler refused\"))",
        &[note("called", "main.tnt", 3)],
    );
}

const HELPER: &str = "// helper pad\n//\n//\n//\n//\n//\n//\nfn helper() {\n    let a = 1\n    let b = 2\n    return a + b\n}\n";

#[test]
fn error_in_main_after_a_module_call_returned_names_main() {
    let (_root, app) = app();
    write(&app, "lib/helper.tnt", HELPER);
    write(
        &app,
        "main.tnt",
        "import { helper } from \"lib/helper\"\nlet value = helper()\nprint(value)\nlet boom = unwrap(Err(\"main refused\"))\n",
    );
    let run = run(&app);
    assert!(run.stdout.contains('3'), "{}", run.stdout);
    assert_reported(
        &run,
        "main.tnt",
        4,
        "let boom = unwrap(Err(\"main refused\"))",
        &[],
    );
    assert!(!run.stderr.contains("helper.tnt"), "{}", run.stderr);
}

#[test]
fn method_miss_after_a_module_call_names_the_main_statement() {
    // The method-miss error used the last line executed anywhere, which was
    // the helper's `return` (line 11), paired with main.tnt.
    let (_root, app) = app();
    write(&app, "lib/helper.tnt", HELPER);
    write(
        &app,
        "main.tnt",
        "import { helper } from \"lib/helper\"\nlet text = helper().shout()\n",
    );
    let run = run(&app);
    assert_reported(&run, "main.tnt", 2, "let text = helper().shout()", &[]);
    assert!(run.stderr.contains("error[E007]"), "{}", run.stderr);
}

#[test]
fn error_in_main_after_a_caught_module_error_names_main() {
    // Line 9 and E005 on purpose: the same line and code as the caught error
    // in lib/checks.tnt, which must not lend main's error its file.
    let (_root, app) = app();
    write(&app, "lib/checks.tnt", CHECKS);
    write(
        &app,
        "main.tnt",
        r#"import { refuse_startup } from "lib/checks"
fn guarded() {
    let ok = refuse_startup() otherwise { return "recovered" }
    return ok
}
print(guarded())
let caught = try { refuse_startup() }
print(caught)
let boom = unwrap(Err("main refused"))
"#,
    );
    let run = run(&app);
    assert!(run.stdout.contains("recovered"), "{}", run.stdout);
    assert_reported(
        &run,
        "main.tnt",
        9,
        "let boom = unwrap(Err(\"main refused\"))",
        &[],
    );
    assert!(!run.stderr.contains("checks.tnt:"), "{}", run.stderr);
}

#[test]
fn err_propagated_with_question_mark_and_unwrapped_in_main_names_main() {
    let (_root, app) = app();
    write(
        &app,
        "lib/parse.tnt",
        r#"// parse pad
fn read_port(text) {
    if text == "" {
        return Err("empty port")
    }
    return Ok(text)
}
fn load_port(text) {
    let port = read_port(text)?
    return Ok(port)
}
"#,
    );
    write(
        &app,
        "main.tnt",
        "import { load_port } from \"lib/parse\"\nprint(\"loading\")\nlet port = unwrap(load_port(\"\"))\n",
    );
    let run = run(&app);
    assert_reported(
        &run,
        "main.tnt",
        3,
        "let port = unwrap(load_port(\"\"))",
        &[],
    );
    assert!(!run.stderr.contains("parse.tnt"), "{}", run.stderr);
}

#[test]
fn module_code_failing_at_import_time_names_the_module_and_the_import() {
    let (_root, app) = app();
    write(
        &app,
        "lib/boot.tnt",
        "// boot pad\n// pad\nlet config = unwrap(Err(\"boot refused\"))\nfn noop() { return 1 }\n",
    );
    write(
        &app,
        "main.tnt",
        "print(\"starting\")\nimport { noop } from \"lib/boot\"\nprint(noop())\n",
    );
    let run = run(&app);
    assert_reported(
        &run,
        &shown(&["lib", "boot.tnt"]),
        3,
        "let config = unwrap(Err(\"boot refused\"))",
        &[note("imported", "main.tnt", 2)],
    );
}

#[test]
fn parse_error_in_an_imported_module_names_the_module() {
    let (_root, app) = app();
    write(
        &app,
        "lib/broken.tnt",
        "// a\n// b\n// c\nfn broken( {\n  return 1\n}\n",
    );
    write(
        &app,
        "main.tnt",
        "import { broken } from \"lib/broken\"\nprint(\"x\")\n",
    );
    let run = run(&app);
    assert!(run.stderr.contains("error[E002]"), "{}", run.stderr);
    assert_reported(
        &run,
        &shown(&["lib", "broken.tnt"]),
        5,
        "  return 1",
        &[note("imported", "main.tnt", 1)],
    );
}

#[test]
fn main_callback_failing_inside_a_module_call_names_main() {
    let (_root, app) = app();
    write(
        &app,
        "lib/runner.tnt",
        "// runner pad\nfn run_twice(cb) {\n    cb(1)\n    cb(2)\n}\n",
    );
    write(
        &app,
        "main.tnt",
        r#"import { run_twice } from "lib/runner"
fn check(n) {
    if n == 2 {
        let boom = unwrap(Err("callback refused"))
    }
}
run_twice(check)
"#,
    );
    let run = run(&app);
    assert_reported(
        &run,
        "main.tnt",
        4,
        "        let boom = unwrap(Err(\"callback refused\"))",
        &[],
    );
    assert!(!run.stderr.contains("runner.tnt"), "{}", run.stderr);
}

const MATH: &str =
    "// math pad\n// pad\nfn safe_div(a, b)\n    requires b != 0\n{\n    return a / b\n}\n";

#[test]
fn module_contract_violation_names_the_module_clause() {
    let (_root, app) = app();
    write(&app, "lib/math.tnt", MATH);
    write(
        &app,
        "main.tnt",
        "import { safe_div } from \"lib/math\"\nprint(\"dividing\")\nlet x = safe_div(1, 0)\n",
    );
    let run = run(&app);
    assert!(run.stderr.contains("error[E004]"), "{}", run.stderr);
    assert_reported(
        &run,
        &shown(&["lib", "math.tnt"]),
        4,
        "    requires b != 0",
        &[note("called", "main.tnt", 3)],
    );
    assert!(run.stderr.contains("where: b = 0"), "{}", run.stderr);
    // The `called from` note names the call site with its file; the file-less
    // `contract checked for call at line N` note is not repeated beside it.
    assert!(
        !run.stderr.contains("contract checked for call"),
        "{}",
        run.stderr
    );
}

#[test]
fn contract_violation_in_main_after_a_caught_module_violation_names_main() {
    // Both `requires` clauses sit on line 4 and both errors are E004.
    let (_root, app) = app();
    write(&app, "lib/math.tnt", MATH);
    write(
        &app,
        "main.tnt",
        r#"import { safe_div } from "lib/math"
// main pad
fn need_positive(n)
    requires n > 0
{
    return n
}
fn try_div() {
    let q = safe_div(1, 0) otherwise { return -1 }
    return q
}
print(try_div())
need_positive(0)
"#,
    );
    let run = run(&app);
    assert!(run.stdout.contains("-1"), "{}", run.stdout);
    assert_reported(&run, "main.tnt", 4, "    requires n > 0", &[]);
    assert!(!run.stderr.contains("math.tnt:"), "{}", run.stderr);
}

#[test]
fn deferred_cleanup_failing_while_an_error_unwinds_keeps_the_original_location() {
    // `cleanup` fails on main.tnt:4 with E005 while the E005 from
    // lib/work.tnt:4 is unwinding; its ignored error must not relabel the
    // one that is reported.
    let (_root, app) = app();
    write(
        &app,
        "lib/work.tnt",
        "// work pad\n// pad\nfn work() {\n    return unwrap(Err(\"work failed\"))\n}\n",
    );
    write(
        &app,
        "main.tnt",
        r#"import { work } from "lib/work"
fn cleanup() {
    print("cleanup ran")
    return unwrap(Err("cleanup failed"))
}
fn run() {
    defer cleanup()
    work()
}
run()
"#,
    );
    let run = run(&app);
    assert!(run.stdout.contains("cleanup ran"), "{}", run.stdout);
    assert!(run.stderr.contains("work failed"), "{}", run.stderr);
    assert_reported(
        &run,
        &shown(&["lib", "work.tnt"]),
        4,
        "    return unwrap(Err(\"work failed\"))",
        &[
            note("called", "main.tnt", 8),
            note("called", "main.tnt", 10),
        ],
    );
}

#[test]
fn single_file_error_inside_nested_calls_has_no_call_site_notes() {
    let (_root, app) = app();
    write(
        &app,
        "main.tnt",
        "fn inner() {\n    return unwrap(Err(\"inner failed\"))\n}\nfn outer() {\n    inner()\n}\nouter()\n",
    );
    let run = run(&app);
    assert_reported(
        &run,
        "main.tnt",
        2,
        "    return unwrap(Err(\"inner failed\"))",
        &[],
    );
    assert!(!run.stderr.contains("note:"), "{}", run.stderr);
}

#[test]
fn error_origin_names_the_module_line_and_each_call_site_for_embedders() {
    use ntnt::error::{IntentError, OriginFrame, OriginStep};
    let (_root, app) = app();
    write(&app, "lib/checks.tnt", CHECKS);
    write(
        &app,
        "main.tnt",
        "import { refuse_startup } from \"./lib/checks\"\nfn start() {\n    refuse_startup()\n}\nstart()\n",
    );
    let source = ntnt::source::SourceFile::read(app.join("main.tnt")).unwrap();
    let parsed = source.parse().unwrap();
    let mut interpreter = ntnt::interpreter::Interpreter::new();
    let error = interpreter.eval_source(&parsed).unwrap_err();
    assert_eq!(error.line(), Some(9));
    let lib = app.join("lib").join("checks.tnt").canonicalize().unwrap();
    let main = app.join("main.tnt").canonicalize().unwrap();
    assert_eq!(
        interpreter.error_origin(&error).unwrap().frames,
        vec![
            OriginFrame {
                file: Some(lib),
                line: 9,
                step: None,
            },
            // `start` is defined in the entry file's own scope, which records
            // no file; its call site is still named.
            OriginFrame {
                file: None,
                line: 3,
                step: Some(OriginStep::Call),
            },
            OriginFrame {
                file: Some(main),
                line: 5,
                step: Some(OriginStep::Call),
            },
        ]
    );
    // An error with another line is not the one that was recorded.
    let other = IntentError::runtime_error("startup refused").at_line(5);
    assert!(interpreter.error_origin(&other).is_none());
}

#[test]
fn module_path_follows_the_entry_path_as_typed_from_another_directory() {
    // `./` imports resolve from the importing file, so this runs from a
    // sibling directory with a relative entry path.
    let (root, app) = app();
    write(&app, "lib/checks.tnt", CHECKS);
    write(
        &app,
        "main.tnt",
        "import { refuse_startup } from \"./lib/checks\"\nprint(\"starting\")\nrefuse_startup()\n",
    );
    let cwd = root.path().join("cwd");
    fs::create_dir_all(&cwd).unwrap();
    let entry = shown(&["..", "app", "main.tnt"]);
    let run = run_in(&cwd, &entry);
    assert_reported(
        &run,
        &shown(&["..", "app", "lib", "checks.tnt"]),
        9,
        "    return unwrap(Err(\"startup refused\"))",
        &[note("called", &entry, 3)],
    );
}

const RATIO: &str = "// ratio pad\n// pad\nfn ratio(a, b)\n    requires a / b > 0\n{\n    return a / b\n}\nfn scaled(a)\n    ensures result / 0 > 1\n{\n    return a\n}\n";

#[test]
fn contract_clause_that_fails_to_evaluate_names_the_module_clause() {
    // Each clause divides by zero while it is evaluated. The error is the
    // clause's own, in lib/ratio.tnt, not the call's in main.tnt.
    for (call, line, text) in [
        ("ratio(1, 0)", 4, "    requires a / b > 0"),
        ("scaled(1)", 9, "    ensures result / 0 > 1"),
    ] {
        let (_root, app) = app();
        write(&app, "lib/ratio.tnt", RATIO);
        write(
            &app,
            "main.tnt",
            &format!("import {{ ratio, scaled }} from \"lib/ratio\"\nprint(\"checking\")\nlet x = {call}\n"),
        );
        let run = run(&app);
        assert_reported(
            &run,
            &shown(&["lib", "ratio.tnt"]),
            line,
            text,
            &[note("called", "main.tnt", 3)],
        );
    }
}

#[test]
fn a_default_argument_does_not_move_the_contract_call_line() {
    // `b = pad()` runs pad's statements before the precondition is checked;
    // the note still names the call on line 11, not pad's last line, 3.
    let (_root, app) = app();
    write(
        &app,
        "main.tnt",
        "fn pad() {\n    let unused = 0\n    return 0\n}\nfn need(a, b = pad())\n    requires a > 0\n{\n    return a\n}\nprint(\"calling\")\nneed(0)\n",
    );
    let run = run(&app);
    assert_reported(&run, "main.tnt", 6, "    requires a > 0", &[]);
    assert!(
        run.stderr.contains("contract checked for call at line 11"),
        "{}",
        run.stderr
    );
}

#[cfg(unix)]
#[test]
fn a_symlinked_entry_file_never_names_a_module_path_that_does_not_exist() {
    // alias/main.tnt is a symlink to app/main.tnt, so the module is
    // app/lib/checks.tnt. The typed directory joined with the module's path
    // would be alias/lib/checks.tnt, which does not exist; the module is named
    // relative to the working directory instead.
    let (root, app) = app();
    write(&app, "lib/checks.tnt", CHECKS);
    write(
        &app,
        "main.tnt",
        "import { refuse_startup } from \"./lib/checks\"\nprint(\"starting\")\nrefuse_startup()\n",
    );
    let alias = root.path().join("alias");
    fs::create_dir_all(&alias).unwrap();
    std::os::unix::fs::symlink(app.join("main.tnt"), alias.join("main.tnt")).unwrap();
    let entry = shown(&["alias", "main.tnt"]);
    let run = run_in(root.path(), &entry);
    assert_reported(
        &run,
        &shown(&["app", "lib", "checks.tnt"]),
        9,
        "    return unwrap(Err(\"startup refused\"))",
        &[note("called", &entry, 3)],
    );
}

#[test]
fn a_caught_module_error_leaves_no_origin_for_another_error() {
    // The E005 from lib/checks.tnt:9 is handled by `try` or `otherwise`. Its
    // trace ends there: a different E005 on the same line is not that error.
    use ntnt::error::IntentError;
    for caught in [
        "let caught = try { refuse_startup() }\nprint(caught)\n",
        "fn guarded() {\n    let caught = refuse_startup() otherwise { return -1 }\n    return caught\n}\nprint(guarded())\n",
    ] {
        let (_root, app) = app();
        write(&app, "lib/checks.tnt", CHECKS);
        write(
            &app,
            "main.tnt",
            &format!("import {{ refuse_startup }} from \"./lib/checks\"\n{caught}"),
        );
        let source = ntnt::source::SourceFile::read(app.join("main.tnt")).unwrap();
        let parsed = source.parse().unwrap();
        let mut interpreter = ntnt::interpreter::Interpreter::new();
        interpreter.eval_source(&parsed).unwrap();
        let other = IntentError::runtime_error("another failure").at_line(9);
        assert!(interpreter.error_origin(&other).is_none(), "{caught}");
    }
}
