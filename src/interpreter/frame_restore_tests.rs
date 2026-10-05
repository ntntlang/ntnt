//! Function-call frame restoration (#183 part 1).
//!
//! A user function call temporarily replaces the interpreter's scope, its
//! deferred-statement list and its `old()`/`result` contract context. Every
//! test here runs on one reused interpreter whose caller frame is seeded with
//! non-empty state, so "reset to empty" cannot pass for "restored".
use super::*;
use crate::contracts::{OldValues, StoredValue};
use crate::{lexer::Lexer, parser::Parser};

fn eval_src(interpreter: &mut Interpreter, source: &str) -> Result<Value> {
    let program = Parser::new(Lexer::new(source).collect()).parse()?;
    interpreter.eval(&program)
}

struct Seed {
    env: Rc<RefCell<Environment>>,
}

/// Install a caller frame: a child scope holding `own = 42`, two sentinel
/// deferred entries, an `old()` snapshot and a `result` value.
fn seed_caller_frame(interpreter: &mut Interpreter) -> Seed {
    let env = Rc::new(RefCell::new(Environment::with_parent(Rc::clone(
        &interpreter.environment,
    ))));
    env.borrow_mut().define("own".to_string(), Value::Int(42));
    interpreter.environment = Rc::clone(&env);
    interpreter
        .deferred_statements
        .push(Expression::Identifier("sentinel_a".to_string()));
    interpreter
        .deferred_statements
        .push(Expression::Identifier("sentinel_b".to_string()));
    let mut old = OldValues::new();
    old.store("seed".to_string(), StoredValue::Int(5));
    interpreter.current_old_values = Some(old);
    interpreter.current_result = Some(Value::Int(7));
    Seed { env }
}

fn assert_restored(interpreter: &Interpreter, seed: &Seed) {
    assert!(
        Rc::ptr_eq(&interpreter.environment, &seed.env),
        "caller scope was not restored"
    );
    let names: Vec<&str> = interpreter
        .deferred_statements
        .iter()
        .map(|e| match e {
            Expression::Identifier(n) => n.as_str(),
            _ => "<other>",
        })
        .collect();
    assert_eq!(
        names,
        ["sentinel_a", "sentinel_b"],
        "caller defer list changed"
    );
    let old = interpreter
        .current_old_values
        .as_ref()
        .expect("caller old() snapshot was cleared");
    assert!(
        matches!(old.get("seed"), Some(StoredValue::Int(5))),
        "caller old() snapshot changed"
    );
    assert!(
        matches!(interpreter.current_result, Some(Value::Int(7))),
        "caller result was not restored: {:?}",
        interpreter.current_result
    );
}

fn assert_undefined(interpreter: &mut Interpreter, name: &str) {
    assert!(
        eval_src(interpreter, name).is_err(),
        "`{name}` leaked into the caller"
    );
}

fn strings(value: Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .map(|v| match v {
                Value::String(s) => s,
                other => format!("{other:?}"),
            })
            .collect(),
        other => panic!("expected an array, got {other:?}"),
    }
}

fn int(value: Result<Value>) -> i64 {
    match value {
        Ok(Value::Int(n)) => n,
        other => panic!("expected Int, got {other:?}"),
    }
}

/// Run `call` (which must fail) in a seeded caller frame, then check the
/// frame came back exactly.
fn assert_failed_call_restores(defs: &str, call: &str, leaked: &[&str]) -> IntentError {
    let mut interpreter = Interpreter::new();
    let seed = seed_caller_frame(&mut interpreter);
    eval_src(&mut interpreter, defs).expect("definitions");
    let err = eval_src(&mut interpreter, call).expect_err("call should fail");
    assert_restored(&interpreter, &seed);
    for name in leaked {
        assert_undefined(&mut interpreter, name);
    }
    assert_eq!(int(eval_src(&mut interpreter, "own")), 42);
    // The same interpreter keeps working for unrelated calls.
    assert_eq!(
        int(eval_src(
            &mut interpreter,
            "fn ok(n) { return n + 1 }\nok(own)"
        )),
        43
    );
    assert_restored(&interpreter, &seed);
    err
}

// ---------------------------------------------------------------------------
// Regressions: each failed on the unfixed interpreter.
// ---------------------------------------------------------------------------

#[test]
fn call_body_error_restores_caller_frame() {
    let err =
        assert_failed_call_restores("fn bad() { let leaked = 73\n 1 / 0 }", "bad()", &["leaked"]);
    assert!(matches!(err, IntentError::DivisionByZero { .. }), "{err:?}");
}

#[test]
fn call_default_arg_error_restores_caller_frame() {
    assert_failed_call_restores("fn bad(a = 1 / 0) { 0 }", "bad()", &["a"]);
    // An earlier argument bound successfully before the default failed.
    assert_failed_call_restores("fn bad(first, a = 1 / 0) { 0 }", "bad(1)", &["first"]);
    // A default that calls a failing function.
    assert_failed_call_restores(
        "fn boom() { let inner = 1\n 1 / 0 }\nfn bad(a = boom()) { 0 }",
        "bad()",
        &["a", "inner"],
    );
}

#[test]
fn call_destructure_error_restores_caller_frame() {
    // The first parameter binds; the second does not match.
    assert_failed_call_restores(
        "fn bad([a, b], [c, d]) { 0 }",
        "bad([1, 2], [3])",
        &["a", "b"],
    );
}

#[test]
fn call_requires_eval_error_restores_caller_frame() {
    assert_failed_call_restores("fn bad(a) requires 1 / 0 { 0 }", "bad(73)", &["a"]);
}

#[test]
fn call_old_capture_error_restores_caller_frame() {
    assert_failed_call_restores(
        "fn bad(a) ensures result == old(1 / 0) { 0 }",
        "bad(73)",
        &["a"],
    );
}

#[test]
fn old_capture_helper_defer_runs_once_at_helper_exit() {
    let mut interpreter = Interpreter::new();
    let seed = seed_caller_frame(&mut interpreter);
    eval_src(
        &mut interpreter,
        r##"
let mut log = []
fn helper() { defer log = log + ["helper"]
 1 / 0 }
fn bad(a) ensures result == old(helper()) { 0 }
"##,
    )
    .unwrap();
    assert!(eval_src(&mut interpreter, "bad(1)").is_err());
    assert_restored(&interpreter, &seed);
    assert_eq!(
        strings(eval_src(&mut interpreter, "log").unwrap()),
        ["helper"]
    );
}

#[test]
fn call_ensures_eval_error_restores_caller_frame() {
    let err = assert_failed_call_restores("fn bad(a) ensures 1 / 0 { 0 }", "bad(73)", &["a"]);
    // The original evaluation error, not a contract violation.
    assert!(matches!(err, IntentError::DivisionByZero { .. }), "{err:?}");
    let mut interpreter = Interpreter::new();
    seed_caller_frame(&mut interpreter);
    eval_src(&mut interpreter, "fn bad(a) ensures 1 / 0 { 0 }").unwrap();
    let _ = eval_src(&mut interpreter, "bad(73)");
    assert_undefined(&mut interpreter, "result");
}

#[test]
fn false_ensures_restores_seeded_old_and_result() {
    let err = assert_failed_call_restores("fn bad(a) ensures a < 0 { 0 }", "bad(5)", &["a"]);
    assert!(
        matches!(err, IntentError::ContractViolation { .. }),
        "{err:?}"
    );
}

#[test]
fn successful_call_restores_seeded_old_and_result() {
    let mut interpreter = Interpreter::new();
    let seed = seed_caller_frame(&mut interpreter);
    eval_src(
        &mut interpreter,
        "fn ok(x) ensures result == old(x) { return x }",
    )
    .unwrap();
    assert_eq!(int(eval_src(&mut interpreter, "ok(3)")), 3);
    assert_restored(&interpreter, &seed);
}

#[test]
fn caught_callee_error_keeps_caller_locals() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
fn bad() { let leaked = 73
 1 / 0 }
fn caller() {
    let own = 42
    let x = bad() otherwise { return own }
    return 0
}
caller()
"##,
    );
    assert_eq!(int(value), 42);
    // The handler did not run in bad()'s scope.
    let value = eval_src(
        &mut interpreter,
        r##"
fn probe() {
    let x = bad() otherwise { return try { leaked } }
    return 0
}
probe()
"##,
    )
    .unwrap();
    assert!(
        !matches!(value, Value::EnumValue { ref variant, .. } if variant == "Ok"),
        "handler could read the callee's local: {value:?}"
    );
}

#[test]
fn nested_call_preserves_caller_old_snapshot() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut x = 1
fn nested() { 0 }
fn outer() ensures result == old(x) + 1 {
    x = 2
    nested()
    return x
}
outer()
"##,
    );
    assert_eq!(int(value), 2);
}

#[test]
fn nested_call_inside_ensures_preserves_old() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut x = 1
fn helper() { 0 }
fn outer() ensures helper() == 0 && result == old(x) + 1 {
    x = 2
    return x
}
outer()
"##,
    );
    assert_eq!(int(value), 2);
}

#[test]
fn uncontracted_callee_does_not_see_caller_old() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let x = 1
let mut seen = 0
fn nested(x) { seen = old(x) }
fn outer() ensures result == old(x) { nested(99)
 return 1 }
outer()
seen
"##,
    );
    assert_eq!(int(value), 99);
}

#[test]
fn contracted_callee_requires_does_not_see_caller_old() {
    let mut interpreter = Interpreter::new();
    let err = eval_src(
        &mut interpreter,
        r##"
fn inner(x) requires old(x) == 7 ensures true { return x }
fn outer(x) ensures old(x) == x { return inner(99) }
outer(7)
"##,
    )
    .expect_err("inner's precondition reads its own x (99), not the caller's snapshot");
    assert!(
        matches!(err, IntentError::ContractViolation { .. }),
        "{err:?}"
    );
}

#[test]
fn contracted_callee_default_does_not_see_caller_old() {
    let mut interpreter = Interpreter::new();
    let result = eval_src(
        &mut interpreter,
        r##"
fn inner(x = old(x)) ensures true { return x }
fn outer(x) ensures old(x) == x { return inner() }
outer(7)
"##,
    );
    assert!(
        result.is_err(),
        "default read the caller's snapshot: {result:?}"
    );
}

#[test]
fn caller_body_old_after_nested_call() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut seen = 0
fn helper() { return 0 }
fn outer(x) ensures old(x) > 0 {
    let mut x = 2
    helper()
    seen = old(x)
    return 42
}
outer(1)
seen
"##,
    );
    assert_eq!(int(value), 1);
}

#[test]
fn failed_call_leaves_no_old_snapshot() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let x = 1
fn bad(x) ensures result == old(x) { 1 / 0 }
let _ = try { bad(99) }
old(x)
"##,
    );
    assert_eq!(int(value), 1);
}

#[test]
fn failing_function_defer_runs_before_caller_handler() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut log = []
fn bad() { let tag = "bad"
 defer log = log + ["defer #{tag}"]
 1 / 0 }
fn caller() {
    let v = bad() otherwise { log = log + ["handler"]
 return 0 }
    return 1
}
caller()
log
"##,
    )
    .unwrap();
    assert_eq!(strings(value), ["defer bad", "handler"]);
}

#[test]
fn block_defer_runs_in_block_scope_after_nested_error() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut seen = ""
fn bad() { let tag = "bad"
 1 / 0 }
let _ = try { let tag = "block"
 defer seen = tag
 bad() }
seen
"##,
    )
    .unwrap();
    assert!(
        matches!(value, Value::String(ref s) if s == "block"),
        "{value:?}"
    );
}

#[test]
fn defer_registered_during_block_cleanup_runs_in_that_cleanup() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut log = []
fn bad() { defer log = log + ["late"]
 1 / 0 }
fn outer() {
    if true { defer bad() }
    log = log + ["after block"]
}
outer()
log
"##,
    )
    .unwrap();
    assert_eq!(strings(value), ["late", "after block"]);
}

#[test]
fn escaping_error_still_runs_callee_defer() {
    let mut interpreter = Interpreter::new();
    eval_src(
        &mut interpreter,
        r##"
let mut log = []
fn bad() { defer log = log + ["bad defer"]
 let x = (1 / 0)?
 return 99 }
"##,
    )
    .unwrap();
    let err = eval_src(&mut interpreter, "bad()").expect_err("division escapes");
    assert!(matches!(err, IntentError::DivisionByZero { .. }), "{err:?}");
    assert_eq!(
        strings(eval_src(&mut interpreter, "log").unwrap()),
        ["bad defer"]
    );
    assert_eq!(interpreter.deferred_statements.len(), 0);
}

// ---------------------------------------------------------------------------
// Controls: existing behaviour that must not change.
// ---------------------------------------------------------------------------

#[test]
fn control_defers_run_before_ensures() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        "let mut x = 0\nfn f() ensures x == 1 { defer x = 1\n return 42 }\nf()",
    );
    assert_eq!(int(value), 42);
}

#[test]
fn control_result_binding_survives_nested_call_in_ensures() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        "fn nested() { 99 }\nfn outer() ensures nested() == 99 && result == 42 { return 42 }\nouter()",
    );
    assert_eq!(int(value), 42);
}

#[test]
fn control_question_on_err_value_runs_defer() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut log = []
fn bad() { defer log = log + ["bad defer"]
 let x = Err("oops")?
 return 99 }
let r = bad()
log = log + ["#{r}"]
log
"##,
    )
    .unwrap();
    assert_eq!(strings(value), ["bad defer", "error: oops"]);
}

#[test]
fn control_callee_defer_reads_its_own_old_snapshot() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        "let mut seen = 0\nfn f(x) ensures result == old(x) { defer seen = old(x)\n return x }\nf(7)\nseen",
    );
    assert_eq!(int(value), 7);
}

#[test]
fn control_contracted_callee_body_reads_its_own_old() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut seen = 0
fn inner(x) ensures true { seen = old(x)
 return x }
fn outer(x) ensures old(x) == x { return inner(99) }
outer(7)
seen
"##,
    );
    assert_eq!(int(value), 99);
}

#[test]
fn control_arity_error_leaves_frame_untouched() {
    // Checked straight after the failing call: arity is rejected before any
    // frame state is switched, today and after the fix.
    let mut interpreter = Interpreter::new();
    let seed = seed_caller_frame(&mut interpreter);
    eval_src(&mut interpreter, "fn two(a, b) { 0 }").unwrap();
    let err = eval_src(&mut interpreter, "two(1)").expect_err("arity");
    assert!(matches!(err, IntentError::ArityMismatch { .. }), "{err:?}");
    assert_restored(&interpreter, &seed);
}

#[test]
fn control_implicit_and_explicit_return_match() {
    let mut interpreter = Interpreter::new();
    assert_eq!(int(eval_src(&mut interpreter, "fn a() { 5 }\na()")), 5);
    assert_eq!(
        int(eval_src(&mut interpreter, "fn b() { return 5 }\nb()")),
        5
    );
}

#[test]
fn control_original_error_kept_when_defer_errors() {
    let mut interpreter = Interpreter::new();
    let err = eval_src(
        &mut interpreter,
        "fn bad() { defer undefined_name\n let x = unwrap(Err(\"first\"))\n 0 }\nbad()",
    )
    .expect_err("body fails");
    assert!(
        format!("{err}").contains("first"),
        "body error replaced: {err}"
    );
}

#[test]
fn control_defers_run_newest_first() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut log = []
fn f() { defer log = log + ["first"]
 defer log = log + ["second"]
 return 0 }
f()
log
"##,
    )
    .unwrap();
    assert_eq!(strings(value), ["second", "first"]);
}

#[test]
fn control_returned_closure_keeps_captured_local() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        "fn make() { let hidden = 11\n return fn() { hidden } }\nlet f = make()\nf()",
    );
    assert_eq!(int(value), 11);
}

#[test]
fn control_outer_assignment_survives_success_and_error() {
    let mut interpreter = Interpreter::new();
    let value = eval_src(
        &mut interpreter,
        r##"
let mut count = 0
fn ok() { count = count + 1
 return 0 }
fn bad() { count = count + 10
 1 / 0 }
ok()
let _ = try { bad() }
count
"##,
    );
    assert_eq!(int(value), 11);
}

#[test]
fn control_contract_diagnostics_read_callee_values() {
    // The caller holds a same-named variable with a different value, so
    // collecting diagnostics after restoring would report 999.
    for (defs, call) in [
        ("fn f(a) requires a > 100 { 0 }", "f(1)"),
        ("fn f(a) ensures a > 100 { 0 }", "f(1)"),
    ] {
        let mut interpreter = Interpreter::new();
        eval_src(&mut interpreter, &format!("let a = 999\n{defs}")).unwrap();
        match eval_src(&mut interpreter, call) {
            Err(IntentError::ContractViolation { values, .. }) => {
                assert!(
                    values.iter().any(|(k, v)| k == "a" && v == "1"),
                    "{defs}: {values:?}"
                );
            }
            other => panic!("{defs}: expected ContractViolation, got {other:?}"),
        }
        assert_eq!(int(eval_src(&mut interpreter, "a")), 999);
    }
}
