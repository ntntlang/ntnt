//! Deterministic ownership regressions for automatic lexical cycle reclamation.
use super::*;
use crate::{lexer::Lexer, parser::Parser};

fn run(interpreter: &mut Interpreter, source: &str) -> Result<Value> {
    interpreter.eval(&Parser::new(Lexer::new(source).collect()).parse()?)
}

#[test]
fn interpreter_root_function_cycle_is_reclaimed() {
    let root = {
        let mut interpreter = Interpreter::new();
        run(
            &mut interpreter,
            "fn recursive(n) { if n == 0 { return 0 }\n recursive(n - 1) }\nrecursive(3)",
        )
        .unwrap();
        Rc::downgrade(&interpreter.environment)
    };
    assert!(root.upgrade().is_none(), "root function cycle leaked");
}

#[test]
fn escaped_callback_survives_owner_then_reclaims_on_last_release() {
    let (callback, weak) = {
        let mut interpreter = Interpreter::new();
        let callback = run(
            &mut interpreter,
            "fn make() { let mut n = 40; let callback = fn() { n = n + 1; n }; callback }\nmake()",
        )
        .unwrap();
        let Value::Function { closure, .. } = &callback else {
            panic!("callback")
        };
        (callback.clone(), Rc::downgrade(closure))
    };
    let mut caller = Interpreter::new();
    assert!(matches!(
        caller.call_function(callback.clone(), vec![]).unwrap(),
        Value::Int(41)
    ));
    drop(caller);
    assert!(
        weak.upgrade().is_some(),
        "escaped callback lost its capture"
    );
    drop(callback);
    assert!(
        weak.upgrade().is_none(),
        "last escaped callback release leaked its cycle"
    );
}

#[test]
fn imported_module_cycle_is_reclaimed_on_interpreter_drop() {
    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join("module.tnt");
    std::fs::write(&module, "fn exported() { 42 }\n").unwrap();
    for _ in 0..8 {
        let weak = {
            let mut interpreter = Interpreter::new();
            let value = run(
                &mut interpreter,
                &format!(
                    "import {{ exported }} from {:?}\nexported",
                    module.to_str().unwrap()
                ),
            )
            .unwrap();
            let Value::Function { closure, .. } = value else {
                panic!("export")
            };
            Rc::downgrade(&closure)
        };
        assert!(
            weak.upgrade().is_none(),
            "imported module function cycle leaked"
        );
    }
}

thread_local! {
    static OBSERVED: RefCell<Vec<std::rc::Weak<RefCell<Environment>>>> = const { RefCell::new(Vec::new()) };
}
fn observe(interpreter: &mut Interpreter, _: &[Value]) -> Result<Value> {
    OBSERVED.with(|seen| {
        seen.borrow_mut()
            .push(Rc::downgrade(&interpreter.environment))
    });
    Ok(Value::Unit)
}
fn install_observer(interpreter: &mut Interpreter) {
    OBSERVED.with(|seen| seen.borrow_mut().clear());
    interpreter.environment.borrow_mut().define(
        "observe".into(),
        Value::ContextFunction {
            name: "observe".into(),
            arity: 0,
            max_arity: 0,
            func: observe,
            requires: None,
        },
    );
}

#[test]
fn persistent_function_and_loop_cycles_are_automatically_bounded() {
    let mut interpreter = Interpreter::new();
    install_observer(&mut interpreter);
    run(
        &mut interpreter,
        r#"
fn local() {
    let captured = "payload"
    let callbacks = [fn() { captured }, map { "callback": fn() { captured } }]
    observe()
    len(callbacks)
}
for n in 0..8000 {
    local()
    let callbacks = map { "nested": [fn() { n }] }
    observe()
}
"#,
    )
    .unwrap();
    let retained = OBSERVED.with(|seen| {
        seen.borrow()
            .iter()
            .filter(|env| env.strong_count() != 0)
            .count()
    });
    let (tracked, budget, sweeps) = cycles::stats();
    assert!(
        retained < 2000,
        "unbounded cycles: {retained}, budget {budget}"
    );
    assert!(
        tracked <= budget + 10,
        "unbounded Weak metadata: {tracked}, budget {budget}"
    );
    assert!(sweeps < 1000, "collection ran per call: {sweeps}");
    eprintln!(
        "persistent: retained={retained}, tracked={tracked}, budget={budget}, sweeps={sweeps}"
    );
    drop(interpreter);
    OBSERVED.with(|seen| assert!(seen.borrow().iter().all(|env| env.strong_count() == 0)));
}

#[test]
fn scalar_calls_do_not_scan_the_live_graph_per_invocation() {
    let mut interpreter = Interpreter::new();
    run(&mut interpreter, "fn scalar(n) { n + 1 }").unwrap();
    cycles::collect(true);
    let before = cycles::stats().2;
    run(&mut interpreter, "for n in 0..10000 { scalar(n) }").unwrap();
    let after = cycles::stats().2;
    assert!(
        after - before < 200,
        "{} sweeps for 10000 scalar calls",
        after - before
    );
    let (tracked, budget, _) = cycles::stats();
    assert!(tracked <= budget + 10);
}

#[test]
fn release_notifications_for_callback_array_aliases_are_coalesced() {
    let interpreter = Interpreter::new();
    let mut callbacks = Vec::new();
    let mut weak = Vec::new();
    for _ in 0..400 {
        let env = Environment::shared(Environment::with_parent(Rc::clone(
            &interpreter.environment,
        )));
        weak.push(Rc::downgrade(&env));
        callbacks.push(callback(&env));
    }
    interpreter
        .environment
        .borrow_mut()
        .define("callbacks".into(), Value::Array(callbacks));
    cycles::collect(true);
    let before = cycles::stats().2;
    let copy = interpreter.environment.borrow().get("callbacks").unwrap();
    drop(copy);
    assert!(
        cycles::stats().2 - before <= 1,
        "dropping aliases swept the entire graph once per captured scope"
    );
    cycles::collect(false);
    assert!(cycles::stats().2 - before <= 1);
    assert!(weak.iter().all(|env| env.strong_count() != 0));
    drop(interpreter);
    assert!(weak.iter().all(|env| env.strong_count() == 0));
}

#[test]
fn release_notifications_for_host_aliases_do_not_rescan_live_captures() {
    let (callback, weak) = {
        let mut interpreter = Interpreter::new();
        let callback = run(
            &mut interpreter,
            "fn make() { let callback = fn() { 42 }; callback }\nmake()",
        )
        .unwrap();
        let Value::Function { closure, .. } = &callback else {
            panic!("callback")
        };
        (callback.clone(), Rc::downgrade(closure))
    };
    let before = cycles::stats().2;
    for _ in 0..400 {
        drop(callback.clone());
    }
    assert!(
        cycles::stats().2 - before <= 1,
        "temporary host aliases repeatedly swept a still-live capture"
    );
    assert!(weak.upgrade().is_some());
    drop(callback);
    assert!(
        weak.upgrade().is_none(),
        "final host release must still reclaim"
    );
}

#[test]
fn release_notifications_coalesce_host_callback_array_roots() {
    let interpreter = Interpreter::new();
    let mut callbacks = Vec::new();
    let mut weak = Vec::new();
    for _ in 0..400 {
        let env = Environment::shared(Environment::with_parent(Rc::clone(
            &interpreter.environment,
        )));
        env.borrow_mut().define("self".into(), callback(&env));
        callbacks.push(callback(&env));
        weak.push(Rc::downgrade(&env));
    }
    drop(interpreter);
    let before = cycles::stats().2;
    drop(callbacks.clone());
    assert!(cycles::stats().2 - before <= 1);
    assert!(weak.iter().all(|env| env.strong_count() != 0));
    let before = cycles::stats().2;
    drop(callbacks);
    assert!(
        cycles::stats().2 - before <= 1,
        "final host array release must not trace once per root"
    );
    assert!(weak.iter().all(|env| env.strong_count() == 0));
}

fn callback(env: &Rc<RefCell<Environment>>) -> Value {
    Value::Function {
        name: "callback".into(),
        params: vec![],
        body: Block { statements: vec![] },
        closure: Rc::clone(env).into(),
        contract: None,
        type_params: vec![],
    }
}

#[test]
fn indirect_parent_cycles_and_all_value_containers_are_traced() {
    let mut interpreter = Interpreter::new();
    let parent = Environment::shared(Environment::new());
    let middle = Environment::shared(Environment::with_parent(Rc::clone(&parent)));
    let child = Environment::shared(Environment::with_parent(Rc::clone(&middle)));
    let value = Value::Return(Box::new(Value::Array(vec![Value::Map(HashMap::from([(
        "struct".into(),
        Value::Struct {
            name: "Holder".into(),
            fields: HashMap::from([("enum".into(), Value::some(callback(&child)))]),
        },
    )]))])));
    parent.borrow_mut().define("nested".into(), value.clone());
    let weak = [
        Rc::downgrade(&parent),
        Rc::downgrade(&middle),
        Rc::downgrade(&child),
    ];
    drop((parent, middle, child));
    cycles::collect(true);
    assert!(
        weak.iter().all(|env| env.strong_count() != 0),
        "host-held nested Value lost captures"
    );
    drop(value);
    // Also works with a different live interpreter: ownership is not tied to
    // whichever interpreter happens to run the collector.
    run(&mut interpreter, "42").unwrap();
    cycles::collect(true);
    assert!(weak.iter().all(|env| env.strong_count() == 0));
}

#[test]
fn completed_local_function_cycle_releases_channel_without_more_allocations() {
    let mut interpreter = Interpreter::new();
    let (sender, receiver) = std::sync::mpsc::channel::<()>();
    interpreter.environment.borrow_mut().define(
        "resource".into(),
        Value::TxChannelHandle(0, ChannelSender(std::sync::Arc::new(sender))),
    );
    run(
        &mut interpreter,
        "fn consume(tx) { fn local() { 42 } }\nconsume(resource)",
    )
    .unwrap();
    interpreter.environment.borrow_mut().undefine("resource");
    assert!(
        matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Disconnected)
        ),
        "completed local function retained its resource without further allocation"
    );
}

#[test]
fn return_wrapped_resource_is_released_with_its_cycle() {
    let interpreter = Interpreter::new();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let tx = std::sync::Arc::new(tx);
    let weak_tx = std::sync::Arc::downgrade(&tx);
    interpreter.environment.borrow_mut().define(
        "resource".into(),
        Value::Return(Box::new(Value::TxChannelHandle(0, ChannelSender(tx)))),
    );
    let function = callback(&interpreter.environment);
    interpreter
        .environment
        .borrow_mut()
        .define("cycle".into(), function);
    drop(interpreter);
    assert!(weak_tx.upgrade().is_none());
    assert!(matches!(
        rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
}

#[test]
fn escaped_mutable_recursion_and_later_declarations_survive_sweeps() {
    let mut interpreter = Interpreter::new();
    run(
        &mut interpreter,
        r#"
fn make() {
    let mut count = 0
    fn even(n) { if n == 0 { return true }; odd(n - 1) }
    fn odd(n) { if n == 0 { return false }; even(n - 1) }
    let f = fn(n) { count = count + 1; [later + count, even(n)] }
    let later = 40
    f
}
let escaped = make()
"#
        .replace("}; ", "}\n")
        .as_str(),
    )
    .unwrap();
    cycles::collect(true);
    let first = run(&mut interpreter, "escaped(4)").unwrap();
    assert!(
        matches!(first, Value::Array(ref a) if matches!(a.as_slice(), [Value::Int(41), Value::Bool(true)]))
    );
    cycles::collect(true);
    let second = run(&mut interpreter, "escaped(3)").unwrap();
    assert!(
        matches!(second, Value::Array(ref a) if matches!(a.as_slice(), [Value::Int(42), Value::Bool(false)]))
    );
}

fn sweep(_: &mut Interpreter, _: &[Value]) -> Result<Value> {
    cycles::collect(true);
    Ok(Value::Bool(true))
}

#[test]
fn active_arguments_contracts_defer_and_registered_routes_are_roots() {
    let mut interpreter = Interpreter::new();
    interpreter.environment.borrow_mut().define(
        "sweep".into(),
        Value::ContextFunction {
            name: "sweep".into(),
            arity: 0,
            max_arity: 0,
            func: sweep,
            requires: None,
        },
    );
    let value = run(
        &mut interpreter,
        r#"
fn make() {
    let captured = 42
    let callback = fn() { captured }
    callback
}
fn exercise(callback)
    requires sweep()
    ensures sweep() && result == old(callback())
{
    defer sweep()
    sweep()
    callback()
}
exercise(make())
"#,
    )
    .unwrap();
    assert!(matches!(value, Value::Int(42)));
    let handler = run(&mut interpreter, "make()").unwrap();
    let weak = match &handler {
        Value::Function { closure, .. } => Rc::downgrade(closure),
        _ => panic!("handler"),
    };
    interpreter.server_state.add_route("GET", "/", handler);
    cycles::collect(true);
    let handler = interpreter.server_state.routes[0].1.clone();
    assert!(matches!(
        interpreter.call_function(handler, vec![]).unwrap(),
        Value::Int(42)
    ));
    assert!(weak.upgrade().is_some());
    interpreter.server_state.clear();
    cycles::collect(true);
    assert!(weak.upgrade().is_none());
}

#[test]
fn success_error_and_caught_native_panic_keep_reused_interpreter_roots() {
    fn panic_native(_: &mut Interpreter, _: &[Value]) -> Result<Value> {
        panic!("cycle test native panic")
    }
    let mut interpreter = Interpreter::new();
    interpreter.environment.borrow_mut().define(
        "panic_native".into(),
        Value::ContextFunction {
            name: "panic_native".into(),
            arity: 0,
            max_arity: 0,
            func: panic_native,
            requires: None,
        },
    );
    run(&mut interpreter, "let keep = 42\nfn cycle() { keep }").unwrap();
    assert!(run(
        &mut interpreter,
        "fn bad() { let f = fn() { 1 }; 1 / 0 }\nbad()"
    )
    .is_err());
    cycles::collect(true);
    assert!(matches!(
        run(&mut interpreter, "cycle()").unwrap(),
        Value::Int(42)
    ));
    // Native embedders may catch a top-level panic and reuse their interpreter.
    let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run(&mut interpreter, "panic_native()")
    }));
    assert!(panic.is_err());
    cycles::collect(true);
    assert!(matches!(
        run(&mut interpreter, "cycle()").unwrap(),
        Value::Int(42)
    ));
}

#[test]
fn borrowed_environment_teardown_is_conservative_and_does_not_panic() {
    let interpreter = Interpreter::new();
    let env = Rc::clone(&interpreter.environment);
    let function = callback(&env);
    env.borrow_mut().define("cycle".into(), function.clone());
    let borrow = env.borrow_mut();
    drop(interpreter);
    drop(function);
    drop(borrow);
    assert!(env.borrow().get("cycle").is_some());
    let weak = Rc::downgrade(&env);
    drop(env);
    cycles::collect(true);
    assert!(weak.upgrade().is_none());
}
