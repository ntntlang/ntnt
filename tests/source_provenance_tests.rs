use ntnt::interpreter::{Interpreter, Value};
use ntnt::source::SourceFile;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn write(root: &Path, name: &str, text: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, text).unwrap();
    path
}

fn run(interpreter: &mut Interpreter, path: &Path) {
    let source = SourceFile::read(path).unwrap();
    interpreter.eval_source(&source.parse().unwrap()).unwrap();
}

fn digest(text: &str) -> [u8; 32] {
    Sha256::digest(text.as_bytes()).into()
}

#[cfg(unix)]
#[test]
fn readable_pipe_input_executes_without_attesting_a_regular_file_identity() {
    use std::os::fd::AsRawFd;
    use std::process::{Command, Stdio};
    let mut writer = Command::new("sh")
        .args(["-c", "printf 'assert(true)\\n'"])
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let pipe = writer.stdout.take().unwrap();
    let path = PathBuf::from(format!("/dev/fd/{}", pipe.as_raw_fd()));
    let source = SourceFile::read(&path).unwrap();
    assert!(writer.wait().unwrap().success());
    let mut interpreter = Interpreter::new();
    interpreter.eval_source(&source.parse().unwrap()).unwrap();
    assert!(
        interpreter.source_snapshot().is_err(),
        "pipe input must not get regular-file attestation"
    );
}

#[cfg(unix)]
#[test]
fn cli_still_accepts_a_piped_dev_stdin_program() {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_ntnt"))
        .args(["run", "/dev/stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"assert(true)\nprint(\"pipe input preserved\")\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("pipe input preserved"));
}

fn check_opaque_block_entry(with_contract: bool) {
    let dir = tempfile::tempdir().unwrap();
    let path = write(
        dir.path(),
        "main.tnt",
        "fn known() { if true { 73 } else { 0 } }",
    );
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, &path);
    let snapshot = interpreter.source_snapshot().unwrap();
    assert!(matches!(
        interpreter.call_function_by_name("known", vec![]).unwrap(),
        Value::Int(73)
    ));
    assert_eq!(
        interpreter.source_snapshot().unwrap(),
        snapshot,
        "normal file-backed blocks must stay tracked"
    );
    let program = ntnt::parser::Parser::new(ntnt::lexer::Lexer::new("73").collect())
        .parse()
        .unwrap();
    let block = ntnt::ast::Block {
        statements: program.statements,
    };
    let result = if with_contract {
        interpreter.eval_block_with_contract(&block, "opaque-host-block", None)
    } else {
        interpreter.eval_block(&block)
    };
    assert!(matches!(result.unwrap(), Value::Int(73)));
    assert!(
        interpreter.source_snapshot().is_err(),
        "opaque AST block kept file-backed attestation"
    );
}

#[test]
fn opaque_block_entry_does_not_inherit_file_backed_attestation() {
    check_opaque_block_entry(false);
}

#[test]
fn opaque_contract_block_entry_does_not_inherit_file_backed_attestation() {
    check_opaque_block_entry(true);
}

#[test]
fn file_replacement_after_parse_cannot_relabel_executed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let old = "fn observed() { 1 }";
    let path = write(dir.path(), "main.tnt", old);
    let source = SourceFile::read(&path).unwrap();
    let parsed = source.parse().unwrap();
    std::fs::write(&path, "fn observed() { 2 }").unwrap();
    let mut interpreter = Interpreter::new();
    interpreter.eval_source(&parsed).unwrap();
    assert!(matches!(
        interpreter
            .call_function_by_name("observed", vec![])
            .unwrap(),
        Value::Int(1)
    ));
    let snapshot = interpreter.source_snapshot().unwrap();
    assert_eq!(
        snapshot.files().get(&path.canonicalize().unwrap()),
        Some(&digest(old))
    );
}

#[test]
fn cached_module_keeps_original_provenance_and_new_interpreter_sees_changed_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let helper = write(dir.path(), "helper.tnt", "export let value = 1");
    let root = write(
        dir.path(),
        "main.tnt",
        r#"
import { value } from "./helper.tnt"
fn again() {
    import { value } from "./helper.tnt"
    return value
}
"#,
    );
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, &root);
    let old = interpreter.source_snapshot().unwrap();
    let mtime = std::fs::metadata(&helper).unwrap().modified().unwrap();
    std::fs::write(&helper, "export let value = 2").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&helper)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(mtime))
        .unwrap();
    assert_eq!(
        std::fs::metadata(&helper).unwrap().modified().unwrap(),
        mtime
    );
    assert!(matches!(
        interpreter.call_function_by_name("again", vec![]).unwrap(),
        Value::Int(1)
    ));
    assert_eq!(interpreter.source_snapshot().unwrap(), old);
    let mut fresh = Interpreter::new();
    run(&mut fresh, &root);
    assert!(matches!(
        fresh.call_function_by_name("again", vec![]).unwrap(),
        Value::Int(2)
    ));
    assert_ne!(fresh.source_snapshot().unwrap().digest(), old.digest());
}

#[test]
fn transitive_helper_changes_with_preserved_mtime_change_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let leaf = write(dir.path(), "leaf.tnt", "export let value = 1");
    write(
        dir.path(),
        "middle.tnt",
        "import { value } from \"./leaf.tnt\"\nexport let middle = value",
    );
    let root = write(
        dir.path(),
        "main.tnt",
        "import { middle } from \"./middle.tnt\"",
    );
    let mut first = Interpreter::new();
    run(&mut first, &root);
    let before = first.source_snapshot().unwrap();
    assert_eq!(before.files().len(), 3);
    let mtime = std::fs::metadata(&leaf).unwrap().modified().unwrap();
    std::fs::write(&leaf, "export let value = 2").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&leaf)
        .unwrap()
        .set_times(std::fs::FileTimes::new().set_modified(mtime))
        .unwrap();
    let mut second = Interpreter::new();
    run(&mut second, &root);
    assert_ne!(before.digest(), second.source_snapshot().unwrap().digest());
}

#[test]
fn canonical_aliases_deduplicate_without_collapsing_distinct_relative_imports() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    write(dir.path(), "helper.tnt", "export let value = 1");
    write(dir.path(), "a/helper.tnt", "export let value = 2");
    write(dir.path(), "b/helper.tnt", "export let value = 3");
    write(
        dir.path(),
        "a/module.tnt",
        "import { value } from \"./helper.tnt\"\nexport let a = value",
    );
    write(
        dir.path(),
        "b/module.tnt",
        "import { value } from \"./helper.tnt\"\nexport let b = value",
    );
    let root = write(
        dir.path(),
        "main.tnt",
        r#"
import { value as first } from "./helper.tnt"
import { value as alias } from "./sub/../helper.tnt"
import { a } from "./a/module.tnt"
import { b } from "./b/module.tnt"
assert(first == 1)
assert(alias == 1)
assert(a == 2)
assert(b == 3)
"#,
    );
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, &root);
    assert_eq!(interpreter.source_snapshot().unwrap().files().len(), 6);
}

#[test]
fn lazy_imports_extend_observed_inputs_but_untaken_imports_are_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let helper = write(dir.path(), "helper.tnt", "export let value = 4");
    let root = write(
        dir.path(),
        "main.tnt",
        r#"
fn lazy() {
    import { value } from "./helper.tnt"
    return value
}
if false { import { nonexistent } from "./does-not-exist.tnt" }
"#,
    );
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, &root);
    let before = interpreter.source_snapshot().unwrap();
    assert_eq!(before.files().len(), 1);
    assert!(matches!(
        interpreter.call_function_by_name("lazy", vec![]).unwrap(),
        Value::Int(4)
    ));
    let after = interpreter.source_snapshot().unwrap();
    assert_eq!(after.files().len(), 2);
    assert!(after.files().contains_key(&helper.canonicalize().unwrap()));
    assert_ne!(before.digest(), after.digest());
    assert_eq!(before.files().len(), 1, "a snapshot must remain immutable");
}

#[test]
fn source_less_ast_is_not_attested_by_setting_a_file_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(dir.path(), "main.tnt", "let value = 1");
    let mut interpreter = Interpreter::new();
    interpreter.set_main_source_file(root.to_str().unwrap());
    let tokens = ntnt::lexer::Lexer::new("let value = 99").collect();
    let program = ntnt::parser::Parser::new(tokens).parse().unwrap();
    interpreter.eval(&program).unwrap();
    assert!(interpreter
        .source_snapshot()
        .unwrap_err()
        .to_string()
        .contains("source-less"));
    run(&mut interpreter, &root);
    assert!(
        interpreter.source_snapshot().is_err(),
        "later file eval cannot erase untracked input"
    );
}

#[test]
fn mixed_source_versions_are_reported_without_disabling_evaluation() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(dir.path(), "main.tnt", "fn observed() { 1 }");
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, &root);
    std::fs::write(&root, "fn observed() { 2 }").unwrap();
    run(&mut interpreter, &root);
    assert!(matches!(
        interpreter
            .call_function_by_name("observed", vec![])
            .unwrap(),
        Value::Int(2)
    ));
    let error = interpreter.source_snapshot().unwrap_err().to_string();
    assert!(error.contains("mixed"), "{error}");
    assert!(error.contains("main.tnt"), "{error}");
    std::fs::write(&root, "fn observed() { 1 }").unwrap();
    run(&mut interpreter, &root);
    assert!(
        interpreter.source_snapshot().is_err(),
        "revisiting the first version cannot hide mixed input"
    );
}

#[test]
fn direct_loaders_and_route_local_imports_are_retained_in_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        ("lib/math.tnt", "fn plus(a, b) { a + b }"),
        ("middleware/01_pass.tnt", "fn middleware(req) { req }"),
        (
            "routes/index.tnt",
            "import { answer } from \"../helpers/answer.tnt\"\nfn get(req) { answer }",
        ),
        ("helpers/answer.tnt", "export let answer = 42"),
        ("jobs/definition.tnt", "let job_file_was_evaluated = true"),
    ];
    for (name, source) in files {
        write(dir.path(), name, source);
    }
    let root = write(
        dir.path(),
        "main.tnt",
        "libs(\"lib\")\nroutes(\"routes\")\njobs(\"jobs\")",
    );
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, &root);
    let snapshot = interpreter.source_snapshot().unwrap();
    assert_eq!(snapshot.files().len(), 6);
    for (name, source) in files {
        assert_eq!(
            snapshot
                .files()
                .get(&dir.path().join(name).canonicalize().unwrap()),
            Some(&digest(source)),
            "{name}"
        );
    }
}

#[test]
fn partially_executed_file_remains_an_observed_input() {
    let dir = tempfile::tempdir().unwrap();
    let root = write(dir.path(), "main.tnt", "let entered = true\nassert(false)");
    let source = SourceFile::read(&root).unwrap();
    let mut interpreter = Interpreter::new();
    assert!(interpreter.eval_source(&source.parse().unwrap()).is_err());
    assert!(interpreter
        .source_snapshot()
        .unwrap()
        .files()
        .contains_key(&root.canonicalize().unwrap()));
}

#[cfg(unix)]
#[test]
fn retargeted_symlink_does_not_change_a_buffered_sources_identity_or_import_origin() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let a = write(
        dir.path(),
        "a/main.tnt",
        "import { value } from \"./helper.tnt\"\nfn observed() { value }",
    );
    let b = write(
        dir.path(),
        "b/main.tnt",
        "import { value } from \"./helper.tnt\"\nfn observed() { value }",
    );
    write(dir.path(), "a/helper.tnt", "export let value = 1");
    write(dir.path(), "b/helper.tnt", "export let value = 2");
    let alias = dir.path().join("current.tnt");
    symlink(&a, &alias).unwrap();
    let source = SourceFile::read(&alias).unwrap();
    std::fs::remove_file(&alias).unwrap();
    symlink(&b, &alias).unwrap();
    let mut interpreter = Interpreter::new();
    run_buffered(&mut interpreter, &source);
    assert!(matches!(
        interpreter
            .call_function_by_name("observed", vec![])
            .unwrap(),
        Value::Int(1)
    ));
    let snapshot = interpreter.source_snapshot().unwrap();
    assert!(snapshot.files().contains_key(&a.canonicalize().unwrap()));
    assert!(!snapshot.files().contains_key(&b.canonicalize().unwrap()));
}

#[cfg(unix)]
fn run_buffered(interpreter: &mut Interpreter, source: &SourceFile) {
    interpreter.eval_source(&source.parse().unwrap()).unwrap();
}

#[cfg(unix)]
#[test]
fn source_identity_does_not_roundtrip_non_utf8_paths_through_display_strings() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let app = dir
        .path()
        .join(std::ffi::OsString::from_vec(b"app-\xff".to_vec()));
    if let Err(error) = std::fs::create_dir(&app) {
        if cfg!(target_os = "macos") && error.raw_os_error() == Some(libc::EILSEQ) {
            assert!(!app.exists());
            return;
        }
        panic!("could not create non-UTF8 fixture: {error}");
    }
    let helper = write(&app, "helper.tnt", "export let value = 7");
    let main = write(
        &app,
        "main.tnt",
        "import { value } from \"./helper.tnt\"\nassert(value == 7)",
    );
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, &main);
    let snapshot = interpreter.source_snapshot().unwrap();
    assert!(snapshot.files().contains_key(&main.canonicalize().unwrap()));
    assert!(snapshot
        .files()
        .contains_key(&helper.canonicalize().unwrap()));
}
