//! Single-threaded trial deletion for the existing Rc lexical graph.
//!
//! Only environment-owned edges are subtracted. Every other strong reference
//! (Rust locals, interpreter state, routes, module exports, host Values) is a
//! root, without requiring the embedding host to register it. Values own their
//! collections by value, so closures and parent links are the only Rc edges.
use super::{Environment, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Deref;
use std::rc::{Rc, Weak};

type Env = Rc<RefCell<Environment>>;
type WeakEnv = Weak<RefCell<Environment>>;
const MIN_BUDGET: usize = 256;

#[derive(Default)]
struct Collector {
    environments: HashMap<usize, Entry>,
    allocations: usize,
    releases: usize,
    budget: usize,
    interpreters: usize,
    external_roots: usize,
    #[cfg(test)]
    collections: usize,
}
struct Entry {
    weak: WeakEnv,
    incoming: usize,
    external_root: bool,
}
thread_local! {
    static COLLECTOR: RefCell<Collector> = RefCell::new(Collector::default());
}
fn id(env: &Env) -> usize {
    Rc::as_ptr(env) as usize
}

/// An Rc-compatible lexical capture with a last-release notification.
///
/// Dereferences to the original `Rc<RefCell<Environment>>`; strong captures,
/// aliasing and mutable bindings are unchanged. `From<Rc<_>>` supports native
/// embedders constructing function Values. The notification is necessary for
/// callbacks whose last host owner is released after their interpreter died.
#[derive(Debug, Clone)]
pub struct Closure {
    env: Option<Env>,
}
impl From<Env> for Closure {
    fn from(env: Env) -> Self {
        track(&env);
        Self { env: Some(env) }
    }
}
impl Deref for Closure {
    type Target = Env;
    fn deref(&self) -> &Env {
        self.env.as_ref().expect("live closure environment")
    }
}
impl Closure {
    pub(crate) fn into_rc(mut self) -> Env {
        self.env.take().expect("live closure environment")
    }
}
impl Drop for Closure {
    fn drop(&mut self) {
        if let Some(env) = self.env.take() {
            let key = id(&env);
            let weak = Rc::downgrade(&env);
            drop(env);
            // Cached root counts are scheduling hints, never proof of garbage.
            // Charge releases against the tracing budget, just like allocations:
            // dropping a copied callback array must not trace once per element.
            let force = COLLECTOR
                .try_with(|cell| {
                    let mut gc = cell.try_borrow_mut().ok()?;
                    let entry = gc.environments.get_mut(&key)?;
                    if entry.external_root && weak.strong_count() <= entry.incoming {
                        entry.external_root = false;
                        gc.external_roots = gc.external_roots.saturating_sub(1);
                    }
                    gc.releases = gc.releases.saturating_add(1);
                    // Coalesce host-owned roots too. When the last cached root
                    // disappears after all interpreters died, sweep promptly;
                    // other releases use the ordinary amortized work budget.
                    Some(gc.interpreters == 0 && gc.external_roots == 0)
                })
                .ok()
                .flatten();
            if let Some(force) = force {
                collect(force);
            }
        }
    }
}

/// Declared last in Interpreter: all its actual roots drop before this guard.
pub(super) struct InterpreterLifetime;
impl InterpreterLifetime {
    pub(super) fn new() -> Self {
        COLLECTOR.with(|cell| cell.borrow_mut().interpreters += 1);
        Self
    }
}
impl Drop for InterpreterLifetime {
    fn drop(&mut self) {
        let _ = COLLECTOR.try_with(|cell| {
            if let Ok(mut gc) = cell.try_borrow_mut() {
                gc.interpreters -= 1;
            }
        });
        collect(true);
    }
}

pub(super) fn shared(environment: Environment) -> Env {
    let env = Rc::new(RefCell::new(environment));
    track(&env);
    env
}
fn track(env: &Env) {
    // Also register raw Rc parent chains supplied by native embedders. Ordinary
    // runtime allocations stop immediately at their already tracked parent.
    let mut current = Some(Rc::clone(env));
    COLLECTOR.with(|cell| {
        let Ok(mut gc) = cell.try_borrow_mut() else {
            return;
        };
        while let Some(env) = current {
            let key = id(&env);
            if gc
                .environments
                .get(&key)
                .is_some_and(|entry| entry.weak.ptr_eq(&Rc::downgrade(&env)))
            {
                break;
            }
            gc.environments.insert(
                key,
                Entry {
                    weak: Rc::downgrade(&env),
                    incoming: 0,
                    external_root: false,
                },
            );
            gc.allocations = gc.allocations.saturating_add(1);
            current = env.try_borrow().ok().and_then(|env| env.parent.clone());
        }
    });
}

fn edges(value: &Value, output: &mut Vec<usize>, work: &mut usize) {
    *work = work.saturating_add(1);
    match value {
        Value::Function { closure, .. } => output.push(id(closure)),
        Value::Array(values) | Value::EnumValue { values, .. } => {
            for value in values {
                edges(value, output, work);
            }
        }
        Value::Map(values) | Value::Struct { fields: values, .. } => {
            for value in values.values() {
                edges(value, output, work);
            }
        }
        Value::Return(value) => edges(value, output, work),
        _ => {}
    }
}

/// Trial-delete a single just-finished scope, without scanning any older scope.
/// Scalar calls take only the strong-count check; local self-cycles release
/// resources promptly even when execution then becomes idle. Cross-scope cycles
/// remain for the budgeted complete graph sweep.
pub(super) fn retire(env: Env) {
    if Rc::strong_count(&env) == 1 {
        return;
    }
    let Ok(scope) = env.try_borrow() else {
        return;
    };
    let mut targets = Vec::new();
    let mut cost = 0;
    for value in scope.values.values() {
        edges(value, &mut targets, &mut cost);
    }
    let internal = targets
        .into_iter()
        .filter(|target| *target == id(&env))
        .count();
    drop(scope);
    if Rc::strong_count(&env) != internal + 1 {
        return;
    }
    let Ok(mut scope) = env.try_borrow_mut() else {
        return;
    };
    let garbage = (std::mem::take(&mut scope.values), scope.parent.take());
    scope.mutable_vars.clear();
    drop(scope);
    let _ = COLLECTOR.try_with(|cell| {
        if let Ok(mut gc) = cell.try_borrow_mut() {
            if let Some(entry) = gc.environments.remove(&id(&env)) {
                if entry.external_root {
                    gc.external_roots = gc.external_roots.saturating_sub(1);
                }
            }
        }
    });
    drop(garbage);
}

/// Safe-point sweep. Reentrant release notifications and borrowed RefCells are
/// conservative: they cannot panic or clear a live environment. No user code
/// runs between tracing and detaching unreachable edges.
pub(super) fn collect(force: bool) {
    let _ = COLLECTOR.try_with(|cell| {
        let Ok(mut gc) = cell.try_borrow_mut() else {
            return;
        };
        if !force && gc.allocations.saturating_add(gc.releases) < gc.budget.max(MIN_BUDGET) {
            return;
        }
        gc.allocations = 0;
        gc.releases = 0;
        #[cfg(test)]
        {
            gc.collections += 1;
        }
        let mut environments = Vec::new();
        gc.environments.retain(|_, entry| {
            if let Some(env) = entry.weak.upgrade() {
                environments.push(env);
                true
            } else {
                false
            }
        });
        let indices: HashMap<_, _> = environments
            .iter()
            .enumerate()
            .map(|(i, env)| (id(env), i))
            .collect();
        let mut outgoing = vec![Vec::new(); environments.len()];
        let mut incoming = vec![0usize; environments.len()];
        let mut borrowed = vec![false; environments.len()];
        let mut costs = vec![1usize; environments.len()];
        for (i, env) in environments.iter().enumerate() {
            let Ok(env) = env.try_borrow() else {
                borrowed[i] = true;
                continue;
            };
            let mut targets = Vec::new();
            if let Some(parent) = &env.parent {
                targets.push(id(parent));
            }
            for value in env.values.values() {
                edges(value, &mut targets, &mut costs[i]);
            }
            for target in targets {
                if let Some(&j) = indices.get(&target) {
                    incoming[j] += 1;
                    outgoing[i].push(j);
                }
            }
        }
        let mut live = borrowed;
        let mut queue = Vec::new();
        gc.external_roots = 0;
        for (i, env) in environments.iter().enumerate() {
            let external_root = live[i] || Rc::strong_count(env) > incoming[i] + 1;
            let entry = gc
                .environments
                .get_mut(&id(env))
                .expect("tracked environment");
            entry.incoming = incoming[i];
            entry.external_root = external_root;
            gc.external_roots += usize::from(external_root);
            if external_root {
                live[i] = true;
                queue.push(i);
            }
        }
        while let Some(i) = queue.pop() {
            for &j in &outgoing[i] {
                if !live[j] {
                    live[j] = true;
                    queue.push(j);
                }
            }
        }
        // Detach before dropping Values. Their closure notifications see the
        // collector already borrowed and do not recurse. Holding the upgraded
        // Rc vector also makes destruction independent of graph traversal order.
        let mut garbage = Vec::new();
        for (i, env) in environments.iter().enumerate() {
            if !live[i] {
                if let Ok(mut env) = env.try_borrow_mut() {
                    garbage.push((std::mem::take(&mut env.values), env.parent.take()));
                    env.mutable_vars.clear();
                    gc.environments.remove(&id(&environments[i]));
                }
            }
        }
        let live_work: usize = costs
            .iter()
            .zip(&live)
            .filter(|(_, live)| **live)
            .map(|(cost, _)| *cost)
            .sum();
        drop(garbage);
        drop(environments);
        gc.environments
            .retain(|_, entry| entry.weak.strong_count() != 0);
        // Charge for values/edges too: a large live module or callback array
        // must not be rescanned on every ordinary scalar function invocation.
        gc.budget = live_work.saturating_mul(2).max(MIN_BUDGET);
    });
}

#[cfg(test)]
pub(super) fn stats() -> (usize, usize, usize) {
    COLLECTOR.with(|cell| {
        let gc = cell.borrow();
        (
            gc.environments.len(),
            gc.budget.max(MIN_BUDGET),
            gc.collections,
        )
    })
}
