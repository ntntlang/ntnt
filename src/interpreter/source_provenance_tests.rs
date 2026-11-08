use super::*;
use crate::source::SourceFile;
use std::path::Path;

fn execute(interpreter: &mut Interpreter, path: &Path) {
    let source = SourceFile::read(path).unwrap();
    interpreter.eval_source(&source.parse().unwrap()).unwrap();
}

fn edit(path: &Path, source: &str) {
    let previous = std::fs::metadata(path).unwrap().modified().unwrap();
    std::fs::write(path, source).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new().set_modified(previous + std::time::Duration::from_secs(2)),
        )
        .unwrap();
}

fn assert_mixed(interpreter: &Interpreter, name: &str) {
    let error = interpreter.source_snapshot().unwrap_err().to_string();
    assert!(error.contains("mixed") && error.contains(name), "{error}");
}

#[test]
fn main_reload_keeps_working_but_cannot_erase_old_source_provenance() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.tnt");
    std::fs::write(&main, "fn observed() { 1 }").unwrap();
    let mut interpreter = Interpreter::new();
    interpreter.set_main_source_file(main.to_str().unwrap());
    execute(&mut interpreter, &main);
    interpreter.server_state.hot_reload = true;
    interpreter.source_snapshot().unwrap();
    edit(&main, "fn observed() { 2 }");
    assert!(interpreter.check_and_reload_main_source());
    assert!(matches!(
        interpreter
            .call_function_by_name("observed", vec![])
            .unwrap(),
        Value::Int(2)
    ));
    assert_mixed(&interpreter, "main.tnt");
}

#[test]
fn partial_library_reload_records_changed_input_without_resetting_the_ledger() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("lib")).unwrap();
    let module = dir.path().join("lib/helper.tnt");
    std::fs::write(&module, "fn observed() { 1 }").unwrap();
    let main = dir.path().join("main.tnt");
    std::fs::write(&main, "libs(\"lib\")").unwrap();
    let mut interpreter = Interpreter::new();
    execute(&mut interpreter, &main);
    interpreter.server_state.hot_reload = true;
    interpreter.source_snapshot().unwrap();
    edit(&module, "fn observed() { 2 }");
    assert!(interpreter.check_and_reload_lib_modules());
    assert!(matches!(
        interpreter
            .call_function_by_name("observed", vec![])
            .unwrap(),
        Value::Int(2)
    ));
    assert_mixed(&interpreter, "helper.tnt");
}

#[test]
fn route_reload_records_changed_input_despite_temporary_import_tracking() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("routes")).unwrap();
    let route = dir.path().join("routes/index.tnt");
    std::fs::write(&route, "fn get(req) { 1 }").unwrap();
    let main = dir.path().join("main.tnt");
    std::fs::write(&main, "routes(\"routes\")").unwrap();
    let mut interpreter = Interpreter::new();
    execute(&mut interpreter, &main);
    interpreter.source_snapshot().unwrap();
    edit(&route, "fn get(req) { 2 }");
    let (handler, _) = interpreter
        .reload_route_handler(route.to_str().unwrap(), "GET")
        .unwrap();
    let value = interpreter
        .call_function(handler, vec![Value::Map(HashMap::new())])
        .unwrap();
    assert!(matches!(value, Value::Int(2)));
    assert_mixed(&interpreter, "index.tnt");
}
