//! Job bodies are cloned out of the declaring AST, just like task callbacks.
use super::*;
use crate::{interpreter::Interpreter, lexer::Lexer, parser::Parser};

#[test]
fn perform_and_failure_handler_keep_their_import_origin_in_a_reused_worker() {
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("jobs");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("value.tnt"), "export fn value() { \"source\" }").unwrap();
    let marker = root.path().join("failed.txt");
    let marker_literal = serde_json::to_string(&marker.to_string_lossy()).unwrap();
    let source = format!(
        r#"job ImportOriginBoundary296 on origins (retry: 0) {{
    perform() {{
        import {{ value }} from "./value.tnt"
        value()
    }}
    on_failure(error, attempt) {{
        import {{ value }} from "./value.tnt"
        import {{ write_file }} from "std/fs"
        assert(value() == "source")
        unwrap(write_file({marker_literal}, value()))
    }}
}}"#
    );
    let source_path = dir.join("job.tnt");
    std::fs::write(&source_path, &source).unwrap();
    let ast = Parser::new(Lexer::new(&source).collect())
        .with_source_file(&source_path)
        .parse()
        .unwrap();
    Interpreter::new().eval(&ast).unwrap();
    let def = JOB_RUNTIME
        .get_job("ImportOriginBoundary296")
        .unwrap()
        .unwrap();
    // Deliberately no current_file or application bootstrap in this interpreter.
    let mut worker = Interpreter::new();
    for _ in 0..2 {
        assert!(matches!(
            execute_in_worker(&mut worker, &def, &HashMap::new()).unwrap(),
            Value::String(value) if value == "source"
        ));
        execute_on_failure_in_worker(&mut worker, &def, "forced failure", 1);
        assert_eq!(std::fs::read_to_string(&marker).unwrap(), "source");
        std::fs::remove_file(&marker).unwrap();
    }
}
