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
    weak_budget: usize,
    interpreters: usize,
    external_roots: usize,
    roots_dirty: bool,
    mutations: usize,
    blocked: Vec<WeakEnv>,
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
    // Use the stable inner address so Environment mutators can identify their
    // registered source without borrowing its RefCell or changing its layout.
    env.as_ref().as_ptr() as usize
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
            // Releasing aliases of a mutably borrowed capture cannot make it
            // collectible yet. Leave dirty hints pending for a later release,
            // rather than charging/tracing the same blocked scope repeatedly.
            let borrowed = env.try_borrow().is_err();
            let key = id(&env);
            let weak = Rc::downgrade(&env);
            drop(env);
            if borrowed {
                return;
            }
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
                    Some(gc.interpreters == 0 && (gc.roots_dirty || gc.external_roots == 0))
                })
                .ok()
                .flatten();
            if let Some(force) = force {
                collect(force);
            }
        }
    }
}

/// Refresh incoming scheduling hints using only the replaced/added Values, not
/// the live graph (or the existing array on append). Unnotified scope/parent
/// drops can leave overcounts, which only schedule an earlier conservative
/// trace. A partial borrowed trace stays dirty until it can be retried.
pub(super) fn bindings_changed<'a>(
    scope: usize,
    removed: impl IntoIterator<Item = &'a Value>,
    added: impl IntoIterator<Item = &'a Value>,
) -> BindingMutation {
    let releases = COLLECTOR.with(|cell| {
        let mut gc = cell.borrow_mut();
        gc.mutations += 1;
        let releases = gc.releases;
        // Stack/clone/native Environment values are external owners, not edges
        // subtracted by the graph trace. Their mutations must not change hints.
        if !gc.environments.contains_key(&scope) {
            return releases;
        }
        for (value, add) in removed
            .into_iter()
            .map(|value| (value, false))
            .chain(added.into_iter().map(|value| (value, true)))
        {
            let mut targets = Vec::new();
            let mut work = 0;
            edges(value, &mut targets, &mut work);
            for target in targets {
                if let Some(entry) = gc.environments.get_mut(&target) {
                    entry.incoming = if add {
                        entry.incoming.saturating_add(1)
                    } else {
                        entry.incoming.saturating_sub(1)
                    };
                }
            }
        }
        releases
    });
    BindingMutation { releases }
}
pub(super) struct BindingMutation {
    releases: usize,
}
impl Drop for BindingMutation {
    fn drop(&mut self) {
        let force = COLLECTOR
            .try_with(|cell| {
                let mut gc = cell.try_borrow_mut().ok()?;
                gc.mutations -= 1;
                (gc.mutations == 0 && gc.releases != self.releases)
                    .then_some(gc.interpreters == 0 && (gc.roots_dirty || gc.external_roots == 0))
            })
            .ok()
            .flatten();
        // Coalesce internal Value drops into one safe-point after the binding
        // update is complete. The source may still be borrowed, so the normal
        // conservative trace protects it; unrelated final owners can release.
        if let Some(force) = force {
            collect(force);
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
            gc.roots_dirty = true;
            current = env.try_borrow().ok().and_then(|env| env.parent.clone());
        }
        if gc.environments.len() >= gc.weak_budget.max(MIN_BUDGET) {
            purge_dead(&mut gc);
        }
    });
}

/// Weak metadata has its own geometric budget: a large live Value collection
/// must not permit an equally large backlog of dead Rc allocation headers.
/// This pass never upgrades an Rc or visits a binding/parent edge.
fn purge_dead(gc: &mut Collector) {
    let mut removed_roots = 0;
    gc.environments.retain(|_, entry| {
        let live = entry.weak.strong_count() != 0;
        removed_roots += usize::from(!live && entry.external_root);
        live
    });
    gc.external_roots = gc.external_roots.saturating_sub(removed_roots);
    gc.weak_budget = gc.environments.len().saturating_mul(2).max(MIN_BUDGET);
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
        if gc.mutations != 0 {
            return;
        }
        // A blocked native borrow defers forced hint refreshes, not ordinary
        // budgeted collection of unrelated cycles. Resolved blockers are
        // popped once; a full trace replaces the list with fresh blockers.
        let mut force = force;
        if gc.interpreters == 0 {
            while let Some(weak) = gc.blocked.last() {
                if weak.upgrade().is_some_and(|env| env.try_borrow().is_err()) {
                    force = false;
                    break;
                }
                gc.blocked.pop();
            }
        }
        if !force && gc.allocations.saturating_add(gc.releases) < gc.budget.max(MIN_BUDGET) {
            return;
        }
        gc.allocations = 0;
        gc.releases = 0;
        gc.blocked.clear();
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
                gc.blocked.push(Rc::downgrade(&environments[i]));
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
        gc.roots_dirty = !gc.blocked.is_empty();
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
        purge_dead(&mut gc);
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
