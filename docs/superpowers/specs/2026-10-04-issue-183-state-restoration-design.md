# Issue #183 part 1: Deterministic interpreter frame restoration

**Issue:** [#183](https://github.com/ntntlang/ntnt/issues/183) (part 1 only)
**Status:** Approved design (v4). Decisions D1–D8 accepted by the maintainer on 2026-10-04. Revised after three independent reviews (Astra, gpt-6-astra, high effort: a code audit, a plan review, a confirming review). Not yet implemented.
**Date:** 2026-10-04
**Target:** 0.5.6. This document is reviewed in PR #269; the code lands through the integration branch described in plan §7
**Evidence:** every reproduction below was re-run on `2823eae`; the programs become regression tests

---

## 1. Problem

When the interpreter enters a function, block, `otherwise` handler, `match` arm, struct invariant check or module/route file, it switches some of its state and is meant to switch it back afterwards. Several sites only switch back on the success path, and some state is cleared instead of restored. Interpreters are reused (HTTP workers, job workers, hot reload, native tests), and user code can catch errors with `otherwise` and `try`, so state left behind leaks into later work.

Reproduced on `main` (`2823eae`):

| Case | Program (abbreviated) | Observed | Expected |
|---|---|---|---|
| Caller loses its own variables and sees the callee's | `bad()` errors, caught by `otherwise { print(leaked); return own }` | prints `leaked=73`, then `Undefined variable: own` | `own` visible, `leaked` not |
| Error in a default argument | `fn bad(a = 1/0)`, caught by `otherwise` | `Undefined variable: own` | caller's variables visible |
| `old()` wrong after a successful nested call | `outer` changes `x`, calls `nested()`, `ensures result == old(x) + 1` | postcondition fails, because `nested` cleared the snapshot | passes |
| `old()` leaks into another function | uncontracted `nested(x)` reads `old(x)` | sees the caller's snapshot, `1` | sees its own value |
| `old()` left behind by a failed call | `try { bad(99) }`, then top-level `old(x)` | `old=99` | `old=1` |
| A failing function's `defer` runs late | `bad()` has a `defer`; the caller's `otherwise` catches the error | `handler`, then `defer bad` | `defer bad`, then `handler` |
| A block's `defer` runs in the wrong scope | `try { let tag="block"; defer print(tag); bad() }` | prints `bad` | prints `block` |
| A `defer` added during cleanup escapes | a block defers a call to a function that defers, then errors | `late` runs when the outer function exits | runs during the block's cleanup |
| An `otherwise` handler's `defer` loses its variables | handler: `let msg; defer print(msg); return 42` | `msg` silently missing, nothing printed | prints `handler` |
| `match` leaks pattern names | `match 73 { x => 1/0 } otherwise { print(x) }`, outer `x = 1` | `x=73` | `x=1` |
| Error while loading a route file | route file fails to load | caller loses its variables; later relative imports resolve from the route file's folder | caller state intact |

## 2. Goals

- **G1.** Each *scope owner* (§4) restores the state it temporarily replaced, on every normal exit: `Ok`, `Err`, `Return`, `Break` and `Continue`.
- **G2.** Each owner runs the `defer` entries it owns, in its own scope, before handing control back, and never touches entries that belong to its callers.
- **G3.** Each function call has its own `old()`/`result` contract context, kept separate from its callers and callees.
- **G4.** One small helper (`with_environment`) for the simple owners that only swap the environment (`otherwise` handler, `match` arms, invariants, templates). The owners that restore more than that (function calls, routes, jobs) each restore their own state in one place. Already-correct owners are not rewritten just to use the helper.
- **G5.** Function calls get no slower than a set threshold. Task 8 of the plan defines the threshold and how it is measured; a result inside measurement noise is "inconclusive, rerun", not a pass or a fail.

**Some behaviour changes are intended, and §8 lists them.** The first draft said successful programs wouldn't change, which was wrong: fixing `old()` and `otherwise` handler defers does change some successful programs.

## 3. Non-goals

- Reference-cycle collection or changes to how environments are owned (#183 part 2). This design must not make part 2 harder; §9 explains why it should make it easier.
- Exact error locations (#257). Diagnostic `current_line`/`current_col` behaviour stays as it is.
- Making module evaluation all-or-nothing. Routes, jobs, module caches and anything else registered before an error stays registered.
- Catching Rust panics in `otherwise`, `try` or `?`. They only catch interpreter errors, and `?` doesn't catch runtime errors at all (it only unwraps `Ok`/`Some`). This is existing behaviour and doesn't change.
- **`defer` at the top level of a module (D6).** Left out unless you decide otherwise.

## 4. Scope owners and what each restores

A *scope owner* is code that temporarily replaces interpreter state. There's no blanket rule that every expression leaves state unchanged: a successful `defer` legitimately adds an entry, and imports legitimately record dependencies.

| Owner | Environment | `current_file` / `imported_files` | `defer` entries it owns | `old()` / `result` context |
|---|---|---|---|---|
| User function call (`call_user_function`) | restore | — | run in the callee's scope before `ensures` (§6.2), then run any added later, before restoring | start empty, restore the caller's on exit |
| Block (`eval_block_inner`) | restore (already does) | — | no code change: becomes correct once every nested owner stops leaking (§6.3) | — |
| `let … otherwise` handler | restore | — | run while `err` and the handler's variables are visible (D5) | — |
| `match` arm (guard and body) | restore | — | — | — |
| Struct invariant check | restore | — | — | — |
| Route load / reload | restore | restore both | — | — |
| Module import / export load | restore (already does) | restore (already does) | out of scope (D6) | — |
| Job execution (`execute_in_worker`, `execute_on_failure_in_worker`) | restore (already does) | roll back after a panic only (D4) | no new code: function calls own their defers (§6.6) | reset at the top-level boundary (existing policy) |

**What must survive restoration:** assignments to outer variables, including ones made by defers; returned values and closures, with their captured environments; module caches, route/job/type registrations; import dependency records from successful or ordinarily-failing work (the one exception: a job that **panics** has `imported_files` rolled back to its value at job start); contract checker statistics; the caller's own `defer` entries, unchanged in order and content.

Restoring means putting the old environment pointer back. It never rolls back the *contents* of an environment.

## 5. Defects found

| # | Site | Defect | Severity |
|---|---|---|---|
| F1 | `call_user_function`: default arguments, destructuring, evaluating `requires`, capturing `old()`, the body, evaluating `ensures` | environment not restored; the function's defers are left behind, or run later in the wrong scope | High |
| F2 | `call_user_function` contract context | nested calls clear the caller's `old()` snapshot even when they succeed; uncontracted callees read the caller's snapshot; failed calls leave their own snapshot installed | High |
| F3 | `eval_block_inner` (symptom) | defers run in whatever environment is installed after a nested failure, and entries added during cleanup escape it | High. Caused by F1; fixed by F1's fix, no block change |
| F4 | `process_route_file`, `reload_route_handler` | environment, `current_file` and `imported_files` left behind on an evaluation error | High |
| F5 | `let … otherwise` handler | `err` and the handler's variables leak on error; a handler `defer` runs after its scope is gone, even on success | Medium |
| F6 | `match` arm guard and body | pattern bindings leak on error | Medium |
| F7 | `check_struct_invariants` | struct fields and `self` leak on an evaluation error | Medium |
| F8 | Job boundary, normal errors (symptom) | defers from failing helpers called in `requires`/`old()`/`ensures` are left on the list | Medium. Caused by F1; fixed by F1's fix, no job change |
| F9 | Job boundary, panics | `current_file`, `imported_files` and the guarded-index warning flag aren't reset after a caught panic | Medium |
| F10 | Existing test `test_execute_in_worker_panic_does_not_leak_call_depth` | never calls a user function, so it can't catch a missing `call_depth` reset | Test gap |

Confirmed already correct for their own environment: `for` loops, the `return … otherwise` form, `with_template_environment`, `render_template_loop_values`, `import_file_module`, `load_module_exports`, the native-test call branch, and `call_depth` on normal errors.

## 6. Design

### 6.1 The helper

```rust
/// Run `f` with `env` installed, restoring the previous environment on every
/// normal exit (Ok or Err). Not unwind-safe: panics are handled at worker
/// boundaries (§6.6).
fn with_environment<T>(
    &mut self,
    env: Rc<RefCell<Environment>>,
    f: impl FnOnce(&mut Self) -> Result<T>,
) -> Result<T> {
    let previous = std::mem::replace(&mut self.environment, env);
    let result = f(self);
    self.environment = previous;
    result
}
```

`with_template_environment` becomes a call to it. Saved state is always moved, never copied, so the call path never clones `OldValues` or `Value`s. The helper only swaps the environment: it doesn't run defers, restore diagnostics or handle panics.

### 6.2 Function calls (F1, F2)

The order matters. Today a function's defers run **before** its `ensures`, and this successful program depends on that:

```ntnt
let mut x = 0
fn f() ensures x == 1 { defer x = 1; return 42 }   // passes today; must keep passing
```

So the fix can't just run the whole function and clean up at the end. It has two cleanup phases: one after the body, one on the way out.

**Shape.** An outer `call_user_function` saves the caller's state, calls an inner `run_callee` that does the work and returns one outcome, then does the final cleanup and restores. The inner function never restores anything itself. Today's in-body `self.environment = previous` lines **and** today's `current_old_values = None` / `current_result = None` clears are removed from it, so the callee's contract context stays alive until the final cleanup (defers can read `old()`).

**Steps:**

1. **Arity check**, before any state changes (unchanged; `call_function` still lowers `call_depth`).
2. **Save** the caller's state: environment (moved out with `mem::replace`), the length of the `defer` list (the *base*), and its `old()` and `result` fields (moved out with `take`). The callee starts with an empty contract context. This must happen before defaults and `requires`, because `old()` reads the live field and would otherwise see the caller's snapshot.
3. Bind arguments, defaults and patterns in the callee's environment.
4. Check `requires`. A false `requires` collects its diagnostic values here, in the callee's scope.
5. Capture the callee's own `old()` values.
6. Run the body and keep its outcome: a value (unwrapping `Return` exactly as today) or an error.
7. **Body cleanup:** run the function's defers (everything above the base), newest first, in the callee's scope. This is a **snapshot drain**: the entries above the base are moved out into a temporary list in one step, then run newest first. Entries added while they run are not picked up by this pass. Nothing runs twice. This runs after the body whether it finished, returned early or errored.
8. If the body succeeded: bind the lexical `result` and set the `current_result` field, check `ensures`, and collect diagnostic values for a false `ensures` in the callee's scope.
9. **Final cleanup** (inside the outer function, on every path, including errors in steps 3–5 and 8): one more snapshot drain of anything above the base, in the same way. The callee's environment and contract context stay installed through both phases. In practice a correctly-behaving helper cleans up after itself, so this pass is a structural guarantee rather than something an in-scope test can isolate; the one case known to reach it is D8.
10. **Restore** the caller's environment, `old()` and `result` exactly, then return the kept outcome.

**Exit paths:**

| Exit | What happens |
|---|---|
| Arity error | returns before step 2; nothing to restore |
| Error or false `requires` in steps 3–5 | body and `ensures` skipped, no `result` bound; step 9 runs any defers already added; restore; original error |
| Body error | step 7 runs defers; `result`/`ensures` skipped; step 9; restore; original error |
| Body value or `Return` | step 7; step 8; step 9; restore; value |
| False `ensures`, or an error while evaluating `ensures` | defers already ran in step 7; for a false clause, diagnostics collected in the callee's scope; step 9; restore; the original error (a `ContractViolation`, or the evaluation error itself, e.g. division by zero) |
| A defer that errors, or returns `Break`/`Continue`/`Return` | ignored as today; the rest still run; the outcome is unchanged |
| Rust panic | no cleanup here; the job worker boundary recovers (§6.6) |

Raw `Break`/`Continue` reaching a function body keeps its current behaviour; this fix doesn't widen how functions treat them.

`current_result` is an internal field. The name `result` that user code reads is a normal environment binding, which already survives nested calls; the fix restores the field and doesn't touch the binding.

`call_function` keeps handling `call_depth` and native-test file/line reporting, as now. Source-line reporting is unchanged; in particular `call_site_line` is still captured **after** default arguments are evaluated, as today. The defer base moves earlier (before defaults) on purpose.

### 6.3 Blocks (F3)

No block code changes. A block's defers run in the wrong scope whenever a nested owner fails and leaves its scope installed: a function call, but also a `match` arm (`block_match_scope.tnt` prints `arm` instead of `block`) or an `otherwise` handler (`block_otherwise_scope.tnt` prints `handler`). Once every nested owner restores correctly (§6.2, §6.4, §6.5), the block runs its defers in its own scope with no change of its own. Each composition is tested in the commit of the owner that fixes it.

Running the block's cleanup once stays enough for everything in this PR. Module top-level `defer` is the one exception (D6, D8). No repeat-until-empty loop and no blanket truncation are added.

### 6.4 `let … otherwise` handlers (F5, D5)

Run the handler like a block: a child scope holding `err`, installed with `with_environment`, and the handler statements run through `eval_block`, which owns their defers. Those defers run while `err` and the handler's variables are still visible.

The two `otherwise` forms deliberately stay different. `let … otherwise` must diverge (`let x = Err("e") otherwise { 42 }` is an error today and stays one), while `return … otherwise` accepts a plain value (`return Err("e") otherwise { 42 }` returns 42). Only the *way the handler runs* is shared. The `let` form's divergence check stays where it is, after the handler runs. `Return`, `Break` and `Continue` propagate as before; an error escapes after cleanup and restoration.

### 6.5 `match`, invariants, routes (F4, F6, F7)

- **`match`:** each arm's guard and body run inside one `with_environment` call that holds the pattern bindings. It returns `Option<Value>`, with `None` meaning "guard false", so a guard-false arm is distinguishable from an arm that returns `Unit`. The scope is restored before the next arm is tried. Both the guarded and unguarded body paths are covered, because today's code duplicates them.
- **Invariants:** the struct fields and `self` are bound inside `with_environment`, and the values for a failed-invariant message are collected there too, before restoring.
- **Routes:** keep the evaluation result, collect handlers while still in the module's scope, restore environment, `current_file` and `imported_files` (by moving them back), then return the error. Registrations stay in place, and handler closures stay live and callable after restoration.

### 6.6 Jobs and panics (F8, F9, D4)

- **Normal errors:** no new job code. Every leftover found by the reviews came from a failing user function inside the job, which now cleans up after itself. A job-level cleanup would have nothing left to clean in this PR's scope, and placing it outside `catch_unwind` would let a panic in a defer escape recovery. If testing finds a real job-owned leftover, it is reported and designed separately, not swept up here.
- **Panics (both `execute_in_worker` and `execute_on_failure_in_worker`):** the existing recovery is extended to also roll back `current_file`, `imported_files` and the guarded-index warning flag to a snapshot taken when the job starts. The flag is restored to its saved value, not forced to `false`. The snapshot is taken once per job invocation (`imported_files` is cloned, not moved, because moving it would break normal dependency tracking) and is thrown away on any non-panic exit. The environment restore stays unconditional on every exit, and the existing whole-list `clear_deferred`, `reset_call_depth` and `clear_contract_state` stay as they are. There's no panic-safe guard on the function-call path, and defers aren't run during panic recovery, because a second panic during that cleanup would abort the process.
- **About the index flag:** `Expression::Index` resets the flag to `false` before evaluating its operands, so a panic inside a guarded index normally leaves it `false`, which is already the usual saved value. The flag restore is defensive and exact-value; no stale-`true` bug in normal workers has been shown, and none is claimed.
- **Access for tests:** the fields stay private. Production gets one small `pub(crate)` pair on `Interpreter` to take and restore the panic snapshot. Tests get, under `#[cfg(test)]` only: a read-only view of the frame fields (including the import map with modification times), a setter to seed the index flag, and a builtin `__test_panic(msg)` installed by `define_builtins`, so an imported module can genuinely panic. None of these exist in non-test builds.

## 7. Decisions

| # | Decision | Recommendation | Why |
|---|---|---|---|
| D1 | When a failing function's `defer` runs | **When that function exits, in its own scope, before the caller continues; the original error is kept** | Matches blocks. Today it runs at some later point, or never |
| D2 | `old()`/`result` across nested calls | **Separate per call: the callee starts empty and the caller's is restored exactly** | Today contracts can pass or fail when they shouldn't |
| D3 | Route load/reload fixes in this PR | **Yes** | The worst case: scope, file and import base all left wrong |
| D4 | Panics | **Recovery at both job worker boundaries (`perform` and `on_failure`), extended to roll back `current_file`, `imported_files` and the index-warning flag** (changed from "rely on what exists") | The existing recovery misses those three |
| D5 *(new)* | Defers in `let … otherwise` handlers | **Run the handler like a block, with its own scope and its own defers** | Today a handler's `defer` silently loses its variables, even when nothing fails |
| D6 *(new)* | `defer` at a module's top level | **Leave out of this PR and open a separate issue** | A real problem, but a separate decision about when a module "exits" |
| D7 *(new)* | Where it lands | **This spec and plan are reviewed in #269. The code lands chunk by chunk through an integration branch, then one PR into `main` (plan §7)** | Changes reach `main`, and any release, only once the whole set is in and tested together |
| D8 *(new)* | Module top-level `defer` reached through a function's cleanup | **Accept the change and document it.** A function that defers `routes("./dir")` loads a module whose top-level `defer` lands on the list during the function's cleanup. Today that entry is never run; after the fix the final cleanup runs it at the function's exit. Keeping today's behaviour would need tagging defer entries by owner, which belongs to the D6 follow-up | Keeping exact old behaviour costs a bigger design for a case that was already wrong (a defer that never runs) |

## 8. Behaviour changes users could notice

Every one of these corrects behaviour that is wrong today, and each gets a changelog entry. Any other change in behaviour is a bug in the fix.

- A failing function's `defer` now runs before the caller's `otherwise` handler, instead of later or not at all (D1). This includes a program whose error escapes to the top: `fn bad() { defer print("bad defer"); let x = (1 / 0)?; return 99 }` used to print nothing before the division error, and now prints `bad defer` first.
- Every function call starts with an empty `old()` context while its defaults, `requires` and `old()` capture are evaluated, and the caller's snapshot is put back afterwards for every later `old()` use in the caller: its body, its `ensures` and its defers (D2). Concretely:
  - postconditions using `old()` after nested calls now see the function's own starting values (`old_nested.tnt` used to fail, now passes);
  - an uncontracted callee reading `old(x)` no longer sees the caller's snapshot;
  - a contracted callee's `requires` or default that reads `old()` no longer sees the caller's snapshot (`contracted_requires.tnt` used to pass with `99`, now its precondition fails; a default `x = old(x)` now errors);
  - a caller's own body reading `old(x)` after a nested call sees its real starting value (`old_body_success.tnt` used to print `2`, now prints `1`).
- A `let … otherwise` handler's `defer` runs while the handler's variables are still visible (D5).
- Names that leaked out of a failed function, `match` arm, handler or invariant are no longer visible after the error.
- After a route file fails to load or reload, the caller keeps its own variables and later relative imports resolve from the caller's file, not the route's (D3).
- After a job panics, the next job on the same worker resolves relative paths from its own file and sees the import list from before the failed job (D4).
- A route module's top-level `defer`, loaded from inside a function's own `defer`, now runs when that function exits instead of never (D8). Module top-level `defer` timing is otherwise not guaranteed by this PR (D6).

## 9. How this relates to part 2 (cycle collection)

The scope owners in §4 are the same safe points part 2 needs: when a function finishes or fails, when a block exits, when a route reloads. After this fix each of them restores its state in exactly one place, which gives part 2 a small, known set of places to hook in. This PR doesn't change how environments are owned.

## 10. Verification

1. **Regression tests for each owner, on one reused `Interpreter`**, landed in the same commit as the fix they prove. Each test runs the failing program, checks the owner's fields exactly, then runs a second program that reads its own variables. Environments are compared with `Rc::ptr_eq`; the caller starts with two sentinel defers, a non-empty `old()` and a `result`, so restoring to "empty" doesn't pass by accident. Snapshots use typed values (`OldValues::get`, `Value` matches), not `Debug` text.
2. **Script-level tests** for every row in §1, rewritten so a test of correct recovery doesn't first print a leaked name (which would now, correctly, fail).
3. **Existing behaviour that must keep working:** defers running before `ensures`, `result` surviving a successful nested call, `?` on `Err` values, `return … otherwise`, loop `break`/`continue`, and contract diagnostics asserted directly on the `ContractViolation` error (values, clause and location), with a same-named caller variable so collecting after restoring would be caught.
4. **Jobs:** helper errors in each contract phase as integration tests; panics in both `perform` and `on_failure`, inside a user function (with defers and `old()`), inside an imported module that first imports another file, and inside a guarded index. The F10 test is strengthened so it fails if `reset_call_depth` is removed.
5. **Mutation check:** remove each fix one at a time; its own test must fail.
6. **Performance:** see plan Task 9 for the threshold and method.
7. **Full test suite, the multi-file fixture app, and CI on Linux, macOS and Windows.**
8. **One focused independent review of the final diff** against this spec.

## 11. Known limits

- User code can't recover from Rust panics, and stack overflows or abort-mode panics can't be recovered at all.
- `defer` at a module's top level (D6) isn't owned by anything yet. Its timing isn't guaranteed by this PR and can change when surrounding cleanup runs (D8). The follow-up issue decides when a module "exits".
- HTTP worker panic handling wasn't part of the review; this design only covers the job worker boundaries. A follow-up issue asks for a separate audit.
- A plain `import` inside a deferred block already runs the module's defers when that block ends; the D8 change only affects route loading, which has no block of its own.
