/// DD-006 benchmarks: interpreter construction cost (Phase 1 gate + post-refactor).
///
/// Phase 1 baseline on dev machine: Interpreter::new() = 43.9 µs
/// These benchmarks show the per-request path cost vs full construction.
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use ntnt::interpreter::{Environment, Interpreter, Value};
use ntnt::stdlib::http_server::SharedState;
use std::collections::{HashMap, HashSet};

fn bench_interpreter_new(c: &mut Criterion) {
    c.bench_function("Interpreter::new (full stdlib)", |b| {
        b.iter(|| {
            let interp = black_box(Interpreter::new());
            drop(interp);
        })
    });
}

fn bench_interpreter_new_for_request(c: &mut Criterion) {
    let shared = SharedState::default();
    c.bench_function("Interpreter::new_for_request (per-request path)", |b| {
        b.iter(|| {
            let interp = black_box(Interpreter::new_for_request(&shared));
            drop(interp);
        })
    });
}

fn bench_environment_from_snapshot_10(c: &mut Criterion) {
    let snapshot: HashMap<String, Value> = (0..10)
        .map(|i| (format!("var_{}", i), Value::String(format!("value_{}", i))))
        .collect();
    let mutable_names: HashSet<String> = HashSet::new();

    c.bench_function("Environment::from_snapshot (10 bindings)", |b| {
        b.iter(|| {
            let env = black_box(Environment::from_snapshot(&snapshot, &mutable_names));
            drop(env);
        })
    });
}

fn bench_environment_from_snapshot_50(c: &mut Criterion) {
    let snapshot: HashMap<String, Value> = (0..50)
        .map(|i| (format!("var_{}", i), Value::String(format!("value_{}", i))))
        .collect();
    let mutable_names: HashSet<String> = HashSet::new();

    c.bench_function("Environment::from_snapshot (50 bindings)", |b| {
        b.iter(|| {
            let env = black_box(Environment::from_snapshot(&snapshot, &mutable_names));
            drop(env);
        })
    });
}

fn bench_shared_state_read_lock(c: &mut Criterion) {
    use std::sync::{Arc, RwLock};
    let shared = Arc::new(RwLock::new(SharedState::default()));

    c.bench_function("SharedState read lock acquire (per-request overhead)", |b| {
        b.iter(|| {
            let guard = black_box(shared.read().unwrap());
            let _ = guard.route_count();
            drop(guard);
        })
    });
}

criterion_group!(
    benches,
    bench_interpreter_new,
    bench_interpreter_new_for_request,
    bench_environment_from_snapshot_10,
    bench_environment_from_snapshot_50,
    bench_shared_state_read_lock,
);
criterion_main!(benches);
