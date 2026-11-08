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

fn resource_callback(
    interpreter: &Interpreter,
) -> (
    Value,
    std::rc::Weak<RefCell<Environment>>,
    std::sync::mpsc::Receiver<()>,
) {
    let env = &interpreter.environment;
    let (sender, receiver) = std::sync::mpsc::channel::<()>();
    env.borrow_mut().define(
        "resource".into(),
        Value::TxChannelHandle(0, ChannelSender(std::sync::Arc::new(sender))),
    );
    let value = callback(env);
    env.borrow_mut().define("self".into(), value.clone());
    env.borrow_mut().define("slot".into(), Value::Unit);
    env.borrow_mut()
        .define("array".into(), Value::Array(vec![]));
    (value, Rc::downgrade(env), receiver)
}

fn escaped_resource_callback() -> (
    Value,
    std::rc::Weak<RefCell<Environment>>,
    std::sync::mpsc::Receiver<()>,
) {
    resource_callback(&Interpreter::new())
}

#[test]
fn mutated_escaped_capture_reclaims_on_final_host_release() {
    let (callback, weak, receiver) = escaped_resource_callback();
    let Value::Function { closure, .. } = &callback else {
        panic!("callback")
    };
    closure
        .borrow_mut()
        .define("another".into(), callback.clone());
    assert!(
        weak.strong_count() != 0,
        "live host callback lost its capture"
    );
    drop(callback);
    assert!(weak.upgrade().is_none(), "mutated capture cycle leaked");
    assert!(matches!(
        receiver.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
}

#[test]
fn escaped_capture_binding_mutators_invalidate_release_hints() {
    for operation in ["mutable", "set", "append", "parent_set", "parent_append"] {
        let (callback, weak, receiver) = escaped_resource_callback();
        let Value::Function { closure, .. } = &callback else {
            panic!("callback")
        };
        match operation {
            "mutable" => closure
                .borrow_mut()
                .define_mutable("another".into(), Value::some(callback.clone())),
            "set" => assert!(closure.borrow_mut().set("slot", callback.clone())),
            "append" => assert!(closure
                .borrow_mut()
                .append_to_array_binding("array", vec![callback.clone()])),
            // Exercise recursive binding resolution without registering a new
            // scope (registration itself would invalidate the cached hints).
            "parent_set" | "parent_append" => {
                let mut child = Environment::with_parent(Rc::clone(closure));
                if operation == "parent_set" {
                    assert!(child.set("slot", callback.clone()));
                } else {
                    assert!(child.append_to_array_binding("array", vec![callback.clone()]));
                }
            }
            _ => unreachable!(),
        }
        assert!(matches!(
            receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        drop(callback);
        assert!(weak.upgrade().is_none(), "{operation} leaked its capture");
        assert!(
            matches!(
                receiver.try_recv(),
                Err(std::sync::mpsc::TryRecvError::Disconnected)
            ),
            "{operation} retained its resource"
        );
    }
}

#[test]
fn borrowed_mutation_defers_tracing_until_a_later_final_release() {
    let interpreter = Interpreter::new();
    let (callback, weak, receiver) = resource_callback(&interpreter);
    let alias = callback.clone();
    let Value::Function { closure, .. } = &alias else {
        panic!("callback")
    };
    let mut scope = closure.borrow_mut();
    drop(interpreter); // The lifetime sweep cannot inspect this borrowed scope.
    let before = cycles::stats().2;
    scope.define("another".into(), callback.clone());
    scope.define("removed".into(), callback.clone());
    scope.undefine("removed");
    assert_eq!(
        cycles::stats().2,
        before,
        "mutators traced partial bindings"
    );
    drop(callback);
    let before = cycles::stats().2;
    for _ in 0..400 {
        drop(alias.clone());
    }
    assert_eq!(
        cycles::stats().2,
        before,
        "borrowed aliases retraced the graph"
    );
    assert!(scope.get("self").is_some(), "borrowed capture was cleared");
    assert!(matches!(
        receiver.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    drop(scope);
    drop(alias);
    assert!(weak.upgrade().is_none(), "borrowed trace prevented retry");
    assert!(matches!(
        receiver.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
}

#[test]
fn borrowed_scope_does_not_suspend_unrelated_budgeted_collection() {
    let interpreter = Interpreter::new();
    let env = Rc::clone(&interpreter.environment);
    env.borrow_mut().define("self".into(), callback(&env));
    let borrow = env.borrow_mut();
    drop(interpreter);
    let mut weak = Vec::new();
    for _ in 0..1000 {
        let parent = Environment::shared(Environment::new());
        let child = Environment::shared(Environment::with_parent(Rc::clone(&parent)));
        parent.borrow_mut().define("child".into(), callback(&child));
        weak.push(Rc::downgrade(&child));
        drop((parent, child));
        cycles::collect(false);
    }
    let retained = weak.iter().filter(|env| env.strong_count() != 0).count();
    assert!(
        retained < 256,
        "native borrow blocked {retained} unrelated cycles"
    );
    assert!(
        borrow.get("self").is_some(),
        "native borrow lost its capture"
    );
    drop(borrow);
    drop(env);
    cycles::collect(true);
    assert!(weak.iter().all(|env| env.strong_count() == 0));
}

#[test]
fn untracked_native_bindings_do_not_subtract_traced_incoming_edges() {
    let interpreter = Interpreter::new();
    let (callback, weak, receiver) = resource_callback(&interpreter);
    let mut native = Environment::new();
    native.define("host_copy".into(), callback.clone());
    drop(interpreter);
    native.undefine("host_copy");
    drop(callback);
    assert!(
        weak.upgrade().is_none(),
        "untracked binding corrupted incoming hints"
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
}

#[test]
fn removing_final_native_binding_releases_its_callback_cycle() {
    let interpreter = Interpreter::new();
    let (callback, weak, receiver) = resource_callback(&interpreter);
    let mut native = Environment::new();
    native.define("last_host_owner".into(), callback);
    drop(interpreter);
    native.undefine("last_host_owner");
    assert!(
        weak.upgrade().is_none(),
        "final native binding release was deferred forever"
    );
    assert!(matches!(
        receiver.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
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
fn dead_scope_metadata_is_bounded_independently_of_live_value_work() {
    let interpreter = Interpreter::new();
    interpreter
        .environment
        .borrow_mut()
        .define("large".into(), Value::Array(vec![Value::Int(42); 100_000]));
    let env = Environment::shared(Environment::with_parent(Rc::clone(
        &interpreter.environment,
    )));
    let escaped = callback(&env);
    env.borrow_mut().define("self".into(), escaped.clone());
    let weak = Rc::downgrade(&env);
    drop(env);
    cycles::collect(true);
    let (live, budget, before) = cycles::stats();
    assert!(budget > 100_000);
    for i in 0..8000 {
        let scope = Environment::shared(Environment::with_parent(Rc::clone(
            &interpreter.environment,
        )));
        scope.borrow_mut().define("scalar".into(), Value::Int(i));
        if i % 2 == 0 {
            cycles::retire(scope);
        }
        // The other scopes deliberately bypass retire (e.g. native owners).
        if i % 127 == 0 {
            assert!(
                cycles::stats().0 <= live * 2 + 256,
                "dead Weak registry entries grew with the large value budget: {:?}",
                cycles::stats()
            );
        }
    }
    assert!(cycles::stats().0 <= live * 2 + 256);
    assert_eq!(cycles::stats().2, before, "metadata cleanup traced Values");
    assert!(
        weak.upgrade().is_some(),
        "metadata cleanup removed live capture"
    );
    drop(interpreter);
    assert!(weak.upgrade().is_some(), "escaped root was lost");
    drop(escaped);
    assert!(weak.upgrade().is_none(), "root release accounting was lost");
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

#[test]
fn mutated_host_callback_array_keeps_release_traces_coalesced() {
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
    for callback in &callbacks {
        let Value::Function { closure, .. } = callback else {
            panic!("callback")
        };
        let before_mutation = cycles::stats().2;
        closure
            .borrow_mut()
            .define("another".into(), callback.clone());
        assert_eq!(cycles::stats().2, before_mutation, "binding change traced");
        drop(callback.clone());
    }
    assert!(
        cycles::stats().2 - before <= 1,
        "binding changes and temporary aliases traced per element"
    );
    drop(callbacks.clone());
    assert!(
        cycles::stats().2 - before <= 1,
        "dirty aliases traced per element"
    );
    assert!(weak.iter().all(|env| env.strong_count() != 0));
    let before = cycles::stats().2;
    drop(callbacks);
    assert!(
        cycles::stats().2 - before <= 1,
        "final roots traced per element"
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
