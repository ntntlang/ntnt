# Issue #183 part 1: Frame restoration — Implementation Plan

**Spec:** [2026-10-04-issue-183-state-restoration-design.md](../specs/2026-10-04-issue-183-state-restoration-design.md) (v4, approved)
**Status:** v5. Decisions D1–D8 accepted on 2026-10-04. Under review in PR #269; implementation has not started. The delivery strategy (§7) is still to be confirmed.
**Base:** `chore/bump-0.5.6` at `f71160d` (`main` `2823eae` plus the version bump). Line numbers come from that commit and will move as tasks land.
**PR:** this plan is reviewed in #269 (version bump + docs only). Code lands through the integration branch in §7 (D7 resolved that way).

**Changes from v4** (from the confirming review):
- **Every test is labelled** *regression* (fails before its fix) or *control* (passes before and after). Controls that guard existing behaviour (the strengthened `reset_call_depth` test, the false-seed index flag, the simple contracted-callee body, a callee defer reading its own `old()`) are now labelled as controls.
- **The exact `old()`/`result` restoration check after a false `ensures`** moved from Task 2 to Task 3. Today's code clears both, so it can't pass as a control.
- **The contracted-callee regression** now exercises `requires`/defaults (before the callee's own capture), which is where the leak actually is.
- **Two mutation claims removed:** the final-cleanup pass and the inner contract clears. No reachable in-scope test isolates either; they're kept as structure, not claimed as proven.
- **Job tests completed:** the test view exposes the import map with modification times; a test-only setter seeds the index flag; `on_failure` returns nothing, so a test counter proves each run really panicked; ordinary success/error dependency controls added.
- **Test infrastructure is assigned to commits:** the job view lands with Task 3, the imported-module panic builtin and flag setter with Task 7.
- **Block composition tests** (`match` and `otherwise` failing inside a block with a `defer`) land with the owner that fixes them.
- **New decision D8** in the spec: a route module's top-level `defer` reached through a function's cleanup now runs instead of never.
- **Rollback** tied to actual commits.

**Changes from v3** (each from the second review's required list):
- **Tests land with their fix,** so every commit is green. v3 asked for all tests first *and* a green suite after every task, which can't both be true. Failing-before evidence is still recorded for every test, locally, before its fix is applied.
- **Leak-probe fixtures are not copied verbatim** into recovery tests. Several print a leaked name first, which correctly fails once fixed. They're kept as "before" evidence, and recovery tests are written fresh.
- **`question_runtime` moved** from "unchanged" controls to the intended changes: it now prints `bad defer` before the division error.
- **Contract diagnostics are asserted directly** on the `ContractViolation` error, not inferred from warning text.
- **Task 4 (blocks) is cut.** The block symptoms are caused by the function-call bug and pass after Task 3 with no block change. There's no in-scope trigger left for a defensive edit.
- **Task 8's normal-error cleanup is cut.** Every leftover came from failing user functions, which Task 3 fixes. Task 8 now only extends panic recovery, at both `perform` and `on_failure`.
- **Jobs get a test bridge,** because `stdlib::jobs::tests` can't see the interpreter's private fields.
- **The imported-module panic needs a real trigger,** because modules don't inherit the worker's test-only `trigger_panic` native.
- **Function exit paths are spelled out** (spec §6.2): one kept outcome, two cleanup phases, inner contract clears removed.
- **Benchmark method fixed:** separate binaries, warm-up, interleaved runs, a measured noise floor, and an inconclusive zone.
- **Rollback is dependency-aware,** not "revert one commit and ignore its tests".

---

## 0. Ground rules

1. **Each fix commit includes its tests, and the full library suite is green at every commit.** Every new test is labelled *regression* or *control*. Before a fix is applied, its regressions are run against the unfixed code and must fail for the stated reason; the failure messages go in a table in the PR description. Controls must pass before and after. A mutation check (Task 8) is a separate requirement: a control can still be mutation-checked against existing code.
2. **Saved state on the call path is moved, never copied** (`mem::replace`, `Option::take`). No new `OldValues` or `Value` clones in `call_user_function`.
3. **No unrelated refactors.** Already-correct sites stay untouched, except `with_template_environment` being rebuilt on the helper.
4. **Each fix names its owner** (spec §4) and the tests that prove it.
5. **Each task ends with** `cargo fmt --all`, `cargo build --locked`, the task's tests, `cargo test --locked --lib`, and `cargo test --locked --test multi_file_app_tests --test language_features_tests`.
6. **Only the behaviour changes in spec §8.** Anything else is a bug.
7. **Stop and report** if a task needs a design change the spec doesn't cover, or if testing turns up a job-owned leftover defer (spec §6.6).

## 1. Pre-flight

- [ ] Rebase on current `main`. Record the exact base commit and build its release binary as `ntnt-base` (kept for benchmarks).
- [ ] Re-run every spec §1 reproduction on the base build and save the output for the PR description.
- [ ] Record the benchmark noise floor (Task 8): base against base.
- [ ] Confirm the D1–D7 answers in the spec match the maintainer's approval.

## 2. Test infrastructure

**Where tests go:**
- **Interpreter state tests:** new `src/interpreter/frame_restore_tests.rs`, declared from `interpreter.rs` like `fast_path_tests` (line ~11785). It's a child module, so it can read private fields.
- **Script tests:** `tests/language_features_tests.rs`, using its `unique_test_dir` (line 140) and `run_ntnt_file` (line 161).
- **Job tests:** the `src/stdlib/jobs.rs` test module, next to the `test_execute_in_worker*` tests.
- **Route tests:** `frame_restore_tests.rs`, calling `process_route_file` and `reload_route_handler` directly on files in a temp directory.

**Helpers in `frame_restore_tests.rs`:**
- `eval_src(&mut Interpreter, &str) -> Result<Value>`: lex, parse and `eval` on the same interpreter.
- `seed_caller_frame(&mut Interpreter) -> Seed`: installs a child environment with `own = 42`, **two** sentinel deferred expressions, a `current_old_values` holding one known entry (built with `OldValues::new` / `store`), `current_result = Some(Value::Int(7))`, a temp `current_file`, and one `imported_files` entry. This stops "reset to empty" from passing by accident.
- `assert_restored(&Interpreter, &Seed)`: `Rc::ptr_eq` on the environment; the two sentinels still present, in order, and nothing else; `OldValues::get` returns the seeded entry; `current_result` matches `Value::Int(7)`; `current_file` and the `imported_files` keys and modification times are equal. Typed checks only, no `Debug` text, because formatting a function value walks its captured environment, which can contain a cycle.
- `assert_reads_own(&mut Interpreter)`: calls a fresh user function returning `own`; it must be `42`.

**Bridge for job tests** (in `interpreter.rs`, `#[cfg(test)] pub(crate)`, read-only):
- `frame_view(&self) -> FrameView { deferred_len, has_old, has_result, call_depth, current_file, imported: HashMap<path, SystemTime>, suppress_index_warn }`, with `FrameView` and its fields `pub(crate)` under `cfg(test)`. Import maps are compared independent of iteration order. **Lands with Task 3** (first used by the F8 job tests).
- `set_suppress_index_warn_for_test(bool)`, `#[cfg(test)]` only, because a read-only view can't seed `true`. **Lands with Task 7.**

**Test-only panic trigger for imported modules:** a `#[cfg(test)]` native `__test_panic(msg)` added in `define_builtins`, which every new module environment calls, so a module file can genuinely panic. Natives are already excluded from exports. One string argument, an exact panic payload, and it increments a thread-local "panic reached" counter just before panicking. It exists only in the library's unit-test build; a check that a normal `ntnt` build reports `__test_panic` as undefined is part of Task 7. **Lands with Task 7.** (The confirming review recommended this over a narrower injection point.)

## Task 1 — Helper

**File:** `src/interpreter.rs`, next to `with_template_environment` (~8377).
- [ ] Add `with_environment` (spec §6.1); rebuild `with_template_environment` on it.
- [ ] **Done when:** template tests, `fast_path_tests` and `tests/performance_guard.rs` are unchanged.

## Task 2 — Controls (they pass before and after every later task)

Added first, so every later task is checked against them.

- [ ] Script controls:
  - `defer_ensures_order.tnt` prints `42`, then `1`
  - `result_nested.tnt` prints `42`
  - `question_err.tnt` prints `bad defer`, then `error: oops`
  - `return_otherwise_defer.tnt` prints `handler`, then `42`
  - `controls.tnt` prints `iteration 1`, `iteration 2`, `arm 7`, `42`, `100`
  - `ordinary_defer_old.tnt` (a callee defer reading its own `old()`) prints `7`, `7`
  - `contracted_body.tnt` (a contracted callee reading `old()` in its body) prints `99`
- [ ] **Diagnostics** (`frame_restore_tests.rs`, asserting on `IntentError::ContractViolation`): false `requires`, false `ensures` and false invariant each report the exact clause, values and location. The caller has a variable of the **same name with a different value**, so collecting values after restoring would be caught. These controls check diagnostics and that the caller's environment is back; the exact `old()`/`result` check after a false `ensures` is a Task 3 regression, because today's code clears both.
- [ ] **Ownership matrix** (state tests):
  - an arity error leaves the frame untouched
  - implicit result and explicit `Return` give the same value
  - the original body error is kept when a defer also errors
  - two local defers run newest first
  - the caller's two sentinels are kept, in order and unchanged
  - a returned closure still reads its captured local
  - an assignment to an outer variable survives both success and error
  - a successful route load returns callable handler closures and its own import set
- [ ] Existing tests that must stay green: `eval_block_runs_deferred_statements_after_statement_error`, `test_contract*`, `test_job_contract*`, `test_template_error_boundary*`, `test_execute_in_worker*`, `fast_path_tests`, `tests/performance_guard.rs`.
- [ ] **Done when** every control passes on the unfixed code.

## Task 3 — Function call frame (F1, F2, and symptoms F3, F8)

**File:** `call_user_function` (~9552). `call_function` (~9353) unchanged.

Implement spec §6.2 exactly:
- [ ] Outer `call_user_function`: arity check; save the caller's frame (environment moved, defer base, `old`/`result` taken); call `run_callee`; final cleanup (snapshot drain above the base, newest first, errors ignored); restore; return the kept outcome. `call_site_line` stays where it is, after defaults.
- [ ] `run_callee` = today's body, minus every `self.environment = previous`, minus the `current_old_values = None` / `current_result = None` clears, plus body cleanup after the body on every body outcome (success, `Return` or error) and before `result`/`ensures`. `result`/`ensures` only run on body success. Diagnostics are still collected in the callee's scope.
- [ ] One shared private helper, `run_deferred_above(base)`: moves the entries above `base` into a temporary list in one step (`split_off`), then runs them newest first. It does **not** pop until empty. Used by both cleanup phases; no third pass.
- [ ] Add the `frame_view` test bridge (§2) in this commit.

**Tests in this commit** (each run against unfixed code first; failure recorded):

*F1, recovery after an error* (each seeds the caller, runs the failing call directly, then `assert_restored` and `assert_reads_own`):
- [ ] `call_body_error_restores_caller_frame`
- [ ] `call_default_arg_error_restores_caller_frame`, plus variants: an earlier argument bound before the failing default, and a default that calls a failing function
- [ ] `call_destructure_error_restores_caller_frame`, written with two destructured parameters, where the first binds and the second fails (the review fixture only had one)
- [ ] `call_requires_eval_error_restores_caller_frame`
- [ ] `call_old_capture_error_restores_caller_frame`, plus a variant where the failing `old()` helper registers a `defer` (the review fixture had none), asserting it ran once
- [ ] `call_ensures_eval_error_restores_caller_frame`, asserting `result` is not visible afterwards, `current_result` equals the seeded value, and the returned error is the original evaluation error (not a `ContractViolation`)
- [ ] `false_ensures_restores_seeded_old_and_result` (moved from Task 2)
- [ ] Script `test_caught_callee_error_keeps_caller_locals`: freshly written; the handler returns `own` and separately checks that `leaked` is undefined, never printing it

*F2, contract context:*
- [ ] `nested_call_preserves_caller_old_snapshot` (`old_nested.tnt`)
- [ ] `uncontracted_callee_does_not_see_caller_old` (`old_inherit.tnt`): expects `99`
- [ ] `contracted_callee_requires_does_not_see_caller_old` (`contracted_requires.tnt`: caller captured `x = 7`, callee `requires old(x) == 7` receives `99`): today passes and prints `99`; after the fix the precondition fails
- [ ] `contracted_callee_default_does_not_see_caller_old` (`contracted_default.tnt`): today prints `7`; after the fix the default `old(x)` errors
- [ ] `caller_body_old_after_nested_call` (`old_body_success.tnt`): today prints `2`, then `42`; after the fix `1`, then `42`
- [ ] `failed_call_leaves_no_old_snapshot` (`old_after_error.tnt`): expects `old=1`
- [ ] `nested_call_inside_ensures_preserves_old`: `x = 1`, the body sets `x = 2`, `ensures helper() == 0 && result == old(x) + 1` (the mutation is essential)
- [ ] `callee_defer_reads_its_own_old_snapshot`: *control* (already passes; see Task 2)
- [ ] `call_restores_seeded_old_and_result_on_success`

*D1, defer timing (also proves the F3 symptoms):*
- [ ] `failing_function_defer_runs_before_caller_handler` (`defer_timing.tnt`)
- [ ] `block_defer_runs_in_block_scope_after_nested_error` (`block_wrong_defer_scope.tnt`)
- [ ] `defer_registered_during_block_cleanup_runs_in_that_cleanup` (`defer_during_cleanup.tnt`)
- [ ] `escaping_error_still_runs_callee_defer` (`question_runtime.tnt`): now prints `bad defer`, then exits with division by zero

*F8 integration* (`jobs.rs`, real `execute_in_worker` / `execute_on_failure_in_worker`, one reused interpreter, via `frame_view`):
- [ ] `job_requires_helper_error_leaves_no_defers`, `job_old_capture_helper_error_leaves_no_defers`, `job_ensures_helper_error_leaves_no_defers`, `job_deferred_helper_error_leaves_no_defers`. Each helper's `defer` appends to a log; the test asserts it ran exactly once and that `deferred_len` is back to its starting value. The deferred-helper case may finish `Ok`, because defer errors are ignored, and the test accepts that. The `ensures`-helper case also shows the helper cleaned up during its *own* body cleanup; it is not evidence for the outer final-cleanup pass.
- [ ] `on_failure_contracted_helper_error_leaves_no_contract_state`
- [ ] Each is followed by a normal job that calls a user function and succeeds.
- [ ] These are labelled as integration tests of Task 3, not of a job-level fix.

- [ ] **Done when** all of the above and every Task 2 control pass.

## Task 4 — `let … otherwise` handler (F5)

**File:** the `Statement::Let` otherwise branch (~5306–5356).
- [ ] Replace the hand-written scope and loop: a child environment with `err` bound, installed with `with_environment`, and the handler run through `eval_block`. The existing divergence check stays after it.
- [ ] **Tests in this commit:**
  - `let_otherwise_handler_error_restores_env`: `let x = Err("e") otherwise { let leak = 73; 1 / 0 }` run directly; afterwards `err` and `leak` are undefined and `assert_restored` passes
  - `let_otherwise_handler_defer_sees_handler_locals`: `return`, `continue` and `break` variants (the `break` variant is new)
  - `block_defer_scope_after_otherwise_error` (`block_otherwise_scope.tnt`): today prints `handler`; after the fix `block`
  - controls: the "must diverge" message is unchanged (`let x = Err("e") otherwise { 42 }`), `err` shadowing an outer `err`, assigning to `err` inside the handler, and a closure that escapes the handler still reads its captured value

## Task 5 — `match` arms and invariants (F6, F7)

**Files:** `Expression::Match` (~7878), `check_struct_invariants` (~11150).
- [ ] `match`: per arm, `with_environment(arm_env, ...)` returning `Option<Value>`, where `None` means the guard was false.
- [ ] Invariants: bind the fields and `self`, then evaluate and collect diagnostics inside `with_environment`.
- [ ] **Tests in this commit:**
  - `match_guard_error_restores_env`
  - `match_guarded_body_error_restores_env` and `match_unguarded_body_error_restores_env`, both run directly without `otherwise`, plus script versions
  - `invariant_eval_error_restores_env`, run directly, followed by a successful evaluation
  - `block_defer_scope_after_match_error` (`block_match_scope.tnt`): today prints `arm`; after the fix `block`
  - controls: a false guard falls through to the next arm, the no-match error, and `Return`/`Break`/`Continue` from arms

## Task 6 — Routes (F4)

**Files:** `process_route_file` (~5005), `reload_route_handler` (~5108).
- [ ] Keep the result of `self.eval(&ast)`; collect handlers only on `Ok`, while still in the module scope; move the environment, `current_file` and `imported_files` back; then propagate.
- [ ] **Tests in this commit** (temp directory; `routes/bad.tnt` successfully imports `../lib/dep.tnt`, then errors):
  - `route_load_error_restores_env_file_and_imports`: seed the caller's map; afterwards the keys and modification times equal the seed
  - `route_reload_error_restores_env_file_and_imports`: the same for reload
  - controls: read error and parse error for both; the missing-handler error for `reload_route_handler` only (`process_route_file` returns no handlers instead); a successful load returns its own import map and a handler closure that is callable after restoring
  - script `route_load_error_keeps_relative_import_base`: written fresh, with no read of `route_local` before the import check
  - `route_module_defer_runs_at_function_exit` (`final_drain_route_d6.tnt`, D8): today prints `42`, `after f`; after the fix `module defer`, `42`, `after f`. Lands with **Task 3**, since it's the final cleanup that changes it; it's documented as the D8 behaviour change, not as proof of the final pass.

## Task 7 — Job panic recovery (F9, F10)

**Files:** `src/stdlib/jobs.rs` `execute_in_worker` (~1426) and `execute_on_failure_in_worker` (~1495); `Interpreter` (one `pub(crate)` snapshot/restore pair).
- [ ] Add the `__test_panic` builtin and `set_suppress_index_warn_for_test` (§2) in this commit.
- [ ] At invocation start, inside both functions: `let meta = interp.take_panic_snapshot()` (clones `current_file`, `imported_files` and `suppress_index_warn` once).
- [ ] On a caught panic only: `interp.restore_panic_snapshot(meta)` alongside the existing `clear_deferred` / `reset_call_depth` / `clear_contract_state`. On any other exit the snapshot is dropped. The environment restore stays unconditional.
- [ ] No normal-error cleanup is added (spec §6.6).
- [ ] **Tests in this commit:**
  - *control:* `job_panic_in_user_function_resets_frame`, which merges with and replaces `test_execute_in_worker_panic_does_not_leak_call_depth` (F10). A user function registers a `defer`, captures `old()` and calls `trigger_panic`, run 6 times with `set_max_recursion_depth(5)`. Each run must report the expected panic message, so a recursion-limit error can't pass for a caught panic. Recovery then calls a user function. It passes today (the reset already exists) and is mutation-checked: it must fail with `reset_call_depth()` removed.
  - *regression:* `job_panic_in_imported_module_restores_file_and_imports`: seed one real dependency via a successful import and set `current_file`; the job imports a module that first imports a **different** file, then calls `__test_panic`; afterwards the import map (with modification times) and `current_file` equal the seed
  - *regression:* `job_panic_restores_index_warning_flag_true`: seed the flag `true`, then panic inside `let x = [1][trigger_panic()] otherwise { return 0 }`. `Expression::Index` resets the flag to `false` before evaluating, so today it ends `false`; after the fix it's `true`
  - *control:* the same with the flag seeded `false`
  - *controls:* an ordinary successful job and an ordinary failing job that each import a new file keep that import afterwards (the snapshot must not be restored on non-panic exits)
  - the same cases for `on_failure`. It returns nothing and swallows errors and panics, so each run proves it really panicked by checking the `__test_panic` / `trigger_panic` counter went up by one, then checks the frame and runs a real `perform` and a real `on_failure` call
  - a check that a normal `ntnt run` of `print(__test_panic("x"))` fails with an undefined name

## Task 8 — Proof

- [ ] **Mutation table.** Remove each item one at a time, run its named test, record that it fails, then put it back:
  - the call-frame environment restore
  - the `old`/`result` restore
  - the body-cleanup phase
  - the `otherwise` block runner
  - the `match` `with_environment`
  - the invariant `with_environment`
  - the two route restores
  - the panic file/import restore
  - the panic index-flag restore
  - `reset_call_depth` (existing code, guarded by the strengthened control)

  Nothing is listed that the final code can't kill. The final-cleanup pass and the removal of the inner contract clears are structural: no reachable in-scope program isolates them (the reviewer confirmed both), so they're not claimed in the table. The final pass is exercised by the D8 test.
- [ ] **Benchmarks** (new workloads in the existing `scripts/bench/` harness, not run in CI):
  - **Workloads:** 1M empty user-function calls; recursive `fib(25)`; closures and default arguments, 500k calls; a contract function with `old()` over a 1,000-element array that makes a **nested call while that snapshot is live**, 50k calls; a function with one `defer`, 500k calls. Each workload must produce the same checksum on the unfixed and fixed builds, so none can depend on behaviour this PR changes (e.g. a postcondition that needs the `old()` fix). Checked on the base build before timing.
  - **Method:** two release binaries (`ntnt-base` and `ntnt-fix`) built the same way; a warm-up run; then 10 runs per binary, interleaved base/fix; each run longer than 1s; the workloads print a checksum so both binaries do the same work.
  - **Noise floor:** the same procedure base against base, from pre-flight.
  - **Acceptance:** median slowdown ≤ 3% per workload. If the noise floor is wider than 3%, a result inside it is **inconclusive and rerun**, not a pass. A real slowdown above 3% needs an explanation and maintainer sign-off. `tests/performance_guard.rs` must stay green; it's a separate guard, not proof of the 3% limit.
- [ ] `cargo test --locked --tests`, `cargo clippy --locked --all-targets` (no new warnings in changed code), `git diff --check`.

## Task 9 — Docs, changelog, PR

- [ ] `CHANGELOG.md`: an `## Unreleased` section with Fixed entries and the spec §8 list, including the escaping-error defer change and the job panic change.
- [ ] `docs/AI_AGENT_GUIDE.md`: one paragraph on when a failing function's defers run, and that `old()` is per call.
- [ ] Open follow-up issues: module top-level `defer` (D6, with the `module_defer*.tnt` fixtures); an HTTP worker panic-recovery audit.
- [ ] Commits, in order:
  1. `chore: bump 0.5.6` (existing)
  2. `refactor: with_environment helper`
  3. `test: frame restoration controls`
  4. `fix: function call frames`, with its tests
  5. `fix: otherwise handler runs as a block`, with its tests
  6. `fix: match and invariant scopes`, with its tests
  7. `fix: route load restoration`, with its tests
  8. `fix: job panic recovery restores file, imports and index flag`, with its tests
  9. `docs: …`
- [ ] Retitle #269: "chore: bump to 0.5.6; fix interpreter frame restoration (#183 part 1)". The description covers the defects, behaviour changes, the before/after failure table, the mutation table, the benchmark table with the noise floor, and "part 2 stays open in #183".
- [ ] The spec's status changes to Implemented only after Task 10 passes.

## Task 10 — Review and CI

- [ ] One independent review of the final diff against the spec (Astra high; Sol xhigh if blocked).
- [ ] Fix real defects and rerun the affected tests; another review only for something materially new.
- [ ] Greptile: no unresolved threads, score read after the last push.
- [ ] CI green on Linux, macOS and Windows at the final commit.
- [ ] Comment on #183: part 1 done in #269, part 2 open.

## 3. Risks

| Risk | Mitigation |
|---|---|
| Defer order changes break working programs | `defer_ensures_order` control; spec §6.2 keeps today's order |
| Defers run twice or out of order on new exit paths | Entries are removed before running; tests count log entries and check LIFO order |
| Per-call `old()` breaks contracts that relied on inheriting it | Intended change (spec §8); changelog; full suite |
| Contract diagnostics read caller values after restore | Direct `ContractViolation` assertions with same-named caller variables |
| Cleanup runs after the contract context is cleared | Inner clears removed; the `callee_defer_reads_its_own_old_snapshot` mutation |
| Slower calls | Moves only; interleaved benchmark with noise floor |
| A missed site, as with #218 | Two reviews, per-owner tests, ownership controls; no AST change |
| Panic import rollback discards real dependencies | Rollback only on panic; ordinary exits keep dependencies; tested both ways |
| A test-only native leaks into release builds | `#[cfg(test)]`; checked by building in release |

## 4. Rollback

Rollback is by commit, not by task (Task 8 is proof, not a commit). The fix commits are 4–8 in the Task 9 list.

- **Reverting commit 4** (function call frames) means also reverting commits 5–8 with their tests, because later composition tests assume function calls no longer leak.
- **Reverting one of commits 5–8** means reverting that owner's fix together with its tests, then checking later commits for tests that depend on it (for example the block composition tests in commits 5 and 6).
- Every rollback ends with a full test-suite run, and the changelog, spec status and PR proof tables are updated to match.

## 5. Questions answered by the second review

1. **Run or drop leftover defers?** Run the ones owned by the exiting function, newest first, each removed before it runs, errors ignored, original outcome kept. Two phases only (after the body; on exit). No repeat-until-empty loop.
2. **Is `let … otherwise` via `eval_block` the same as `return … otherwise`?** The way the handler runs is the same; the forms deliberately differ (`let` must diverge, `return` accepts a value). The `let` divergence check stays.
3. **Clone or move `imported_files` for jobs?** Clone once per invocation, restore only on panic, drop otherwise. Moving it would break normal dependency tracking.
4. **Is 3% measurable?** Only with interleaved runs and a measured noise floor. Inside the noise it's inconclusive.
5. **HTTP worker panic path in this PR?** No. Follow-up audit issue.

## 6. Questions answered by the confirming review

1. **`__test_panic` in `define_builtins`?** Yes: it's the right injection point, because modules build fresh environments through it, and natives aren't exported.
2. **Does removing the inner contract clears change successful behaviour?** Not on its own. The broader per-call `old()` change does, and spec §8 now lists every case (`requires`, defaults, the caller's own body).
3. **Is the Task 4 → 7 order safe?** Yes, once controls and regressions are labelled correctly and each test lands with the owner that fixes it.

## 7. Delivery strategy (proposed, to be confirmed)

This change touches function calls, `defer` and contracts, so it lands through an **integration branch** rather than straight to `main`:

1. **Before any fix work**, one small PR to `main` lets CI run on PRs into `integration/**` branches. Today `ci.yml` only runs for PRs into `main`, so chunk PRs would otherwise get no CI.
2. **Create `integration/183-frame-restoration`** from `main` after #269 merges.
3. **One PR per chunk into the integration branch**, each green on Linux, macOS and Windows and reviewed by Greptile:
   - helper and controls (Tasks 1–2)
   - function call frames (Task 3)
   - `let … otherwise` (Task 4)
   - `match` and invariants (Task 5)
   - routes (Task 6)
   - job panic recovery (Task 7)
4. **Keep it current:** merge `main` into the integration branch whenever `main` changes, and rerun CI.
5. **Before the final PR:** the Task 8 proof on the integrated branch (mutation table, benchmarks), the full test suite, the multi-file fixture app, and real apps run against the integration build. Then one independent review of the whole diff.
6. **One final PR** from the integration branch into `main`. It merges with a merge commit, not a squash, so each chunk can still be reverted on its own.

No release is cut from the integration branch, and `main` doesn't receive any of these behaviour changes until the final PR merges.
