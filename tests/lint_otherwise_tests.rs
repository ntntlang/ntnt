#[path = "support/system_fixture.rs"]
mod fixture;

fn unused_imports(source: &str) -> Vec<String> {
    let mut lint = fixture::Fixture::start(source, &["lint"], &[("NTNT_LINT_MODE", "default")]);
    assert!(lint.wait().success(), "{} {}", lint.stdout(), lint.stderr());
    let report: serde_json::Value = serde_json::from_str(&lint.stdout()).unwrap();
    assert_eq!(report["summary"]["total_files"], 1, "{report}");
    assert_eq!(report["summary"]["errors"], 0, "{report}");
    report["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|file| file["issues"].as_array().unwrap())
        .filter(|issue| issue["rule"] == "unused_import")
        .map(|issue| issue["message"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn imports_used_only_in_let_otherwise_are_used() {
    let source = r#"
import { text, status, parse_json } from "std/http/server"
export fn post(req) {
    let data = parse_json(req) otherwise { return status(400, "bad") }
    return text("ok")
}
"#;
    assert_eq!(unused_imports(source), Vec::<String>::new());
}

#[test]
fn imports_used_only_in_destructuring_otherwise_are_used() {
    let source = r#"
import { text, status } from "std/http/server"
export fn response() {
    let [value] = Ok([1]) otherwise { return status(400, "bad") }
    return text(str(value))
}
"#;
    assert_eq!(unused_imports(source), Vec::<String>::new());
}

#[test]
fn aliased_imports_used_in_nested_otherwise_are_used() {
    let source = r#"
import { text, status as bad_request, parse_json } from "std/http/server"
export fn post(req) {
    let data = parse_json(req) otherwise {
        let count = int("bad") otherwise { return bad_request(400, "bad") }
        return text(str(count))
    }
    return text("ok")
}
"#;
    assert_eq!(unused_imports(source), Vec::<String>::new());
}

#[test]
fn imports_used_in_return_otherwise_remain_used() {
    let source = r#"
import { status, parse_json } from "std/http/server"
export fn post(req) {
    return parse_json(req) otherwise { return status(400, "bad") }
}
"#;
    assert_eq!(unused_imports(source), Vec::<String>::new());
}

#[test]
fn otherwise_usage_does_not_hide_genuinely_unused_imports() {
    let source = r#"
import { text, status, parse_json, redirect } from "std/http/server"
export fn post(req) {
    let data = parse_json(req) otherwise { return status(400, "bad") }
    return text("ok")
}
"#;
    assert_eq!(unused_imports(source), ["Unused import: 'redirect'"]);
}

#[test]
fn used_import_aliases_are_tracked_by_their_local_names() {
    let source = r#"
import { trim as strip } from "std/string"
print(strip(" hello "))
"#;
    assert_eq!(unused_imports(source), Vec::<String>::new());
}

#[test]
fn unused_import_aliases_are_reported_by_their_local_names() {
    let source = r#"
import { text, status as bad_request } from "std/http/server"
print(text("ok"))
"#;
    assert_eq!(unused_imports(source), ["Unused import: 'bad_request'"]);
}
