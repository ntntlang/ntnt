# DD-044: Jobs DX Fixes — Addressing v0.4.6 Field Findings

**Status:** Nearly Complete (5/6 fixes done)
**Author:** Larri
**Created:** 2026-03-21
**Source:** [ntnt-findings.md §5](https://app.larri.net/admin/design-docs/dd-005-ntnt-findings) — findings #25-30 from snowgauge.app build
**Implemented in:** PR #43 (Fixes C, D, E), DD-045 PRs #44-#46 (Fix A superseded), PR #54 (Fix F)
**Remaining:** Fix B (schedule smart capture)

---

## Summary

Six issues found building snowgauge.app on `std/jobs`. All have been addressed except Fix B.

---

## Findings → Fixes

### Fix A: Job perform blocks lack imports (#25) — ✅ SUPERSEDED by DD-045

**Problem:** Job perform blocks ran in `Interpreter::new()` with no imports from the parent file.

**Resolution:** DD-045 (PRs #44-#46) replaced `execute_job_perform()` with `create_job_interpreter()`, which evaluates the entire source file in Worker mode. Workers now have full app context — all imports, functions, and constants.

- [x] Job perform can call `now()` from `std/time` when parent file imports it
- [x] Job perform can call `fetch()` from `std/http` when parent file imports it
- [x] Job perform that uses an unimported function still fails with clear error
- [x] Imports inside perform block still work (backward compatible)

---

### Fix B: `schedule()` captures entire scope, fails on user functions (#26) — ⏳ OPEN

**File:** `src/stdlib/concurrent.rs` — `validate_and_capture()` / `capture_bindings()`

**Problem:** `schedule(3600000, fn() { enqueue_all_sites() })` fails because `capture_bindings()` walks `closure.borrow().all_bindings()` — the entire environment. User-defined functions (like route handlers) can't cross thread boundaries (`Rc<RefCell>` is not `Send`), so `schedule()` fails even when the closure never references them.

**Current workaround:** Use `enqueue_in()` self-scheduling pattern instead of `schedule()`.

**Proposed fix (simplified):** Instead of full free-variable analysis (AST walking), skip user-defined functions during capture instead of failing on them. If the closure actually references a skipped function, it gets a clear "undefined variable" runtime error at execution time.

```rust
// In capture_bindings(): change user-defined function handling from error to skip
Value::Function { .. } => {
    // Skip — can't cross threads. If the closure body references this,
    // it will fail at runtime with "undefined variable" (clear error).
    skipped_fns.push(key.clone());
}
```

**Why simpler than free-variable analysis:**
- No AST walking needed, no new `free_variables()` function
- Change isolated to `capture_bindings()` — one function
- Dev-mode warning lists what was skipped

- [ ] `schedule()` works when closure references only native functions + data, even if user functions exist in scope
- [ ] `schedule()` closure that references a user-defined function fails with "undefined" error at runtime
- [ ] `spawn()` and `after()` also benefit (same code path)
- [ ] Captured data values still work correctly
- [ ] Dev warning printed listing skipped functions

---

### Fix C: `parse_json(None)` returns `Err` instead of throwing (#27) — ✅ COMPLETE (PR #43)

**Problem:** `kv::get()` on missing key returns `Ok(None)`. Passing `None` to `parse_json()` throws a `TypeError` instead of returning `Err`.

**Fix:** Handle `None`/`Unit` inputs gracefully — return `Err` instead of throwing.

- [x] `parse_json(None)` returns `Err("...None/null...")` — not a thrown error
- [x] `parse_json("null")` still returns `Ok(None)` (existing behavior)
- [x] `parse_json("{}")` still returns `Ok(map {})` (existing behavior)
- [x] `parse_json(42)` still throws TypeError (non-string, non-None)

---

### Fix D: Better error message for unwrapped KV handles (#28) — ✅ COMPLETE (PR #43)

**Problem:** `get(open("redis://..."), "key")` says "Expected a KV store handle" — no hint about the missing `unwrap()`.

**Fix:** Detect `Result` wrapper and suggest `unwrap()`.

- [x] `get(open("redis://..."), "key")` produces the hint message
- [x] `get(unwrap(open("redis://...")), "key")` works normally

---

### Fix E: Idempotent job registration (#29) — ✅ COMPLETE (PR #43)

**Problem:** Worker threads re-execute the .tnt file, logging "Duplicate job definition" errors. 8 workers × N jobs = noisy.

**Fix:** `register_job()` silently skips if the job name already exists.

- [x] Registering same job name twice returns `Ok(())` (not error)
- [x] First registration's definition is preserved (not overwritten)
- [x] Worker startup logs are clean (no "Duplicate" warnings)

---

### Fix F: `ntnt jobs` CLI re-executes the full app (#30) — ✅ COMPLETE (PR #54)

**Problem:** `ntnt jobs list server.tnt` evaluates in Normal mode — fires `listen()`, `enqueue()`, `work_async()`.

**Fix:** Evaluate in `ExecutionMode::Worker` — only `configure_queue()` and job definitions run. One-line change leveraging DD-045's RuntimeCapability system.

- [x] `ntnt jobs list server.tnt` works without binding ports or spawning workers
- [x] `ntnt jobs status server.tnt` shows counts without side effects
- [x] No duplicate job enqueues when running jobs CLI commands

---

## Validation

After all fixes, snowgauge.app should:
1. ~~Work without any imports inside `perform` blocks~~ ✅ (DD-045)
2. Use `schedule()` directly instead of the `enqueue_in` self-scheduling workaround (Fix B — pending)
3. ~~Use `parse_json(get(kv, key))` without guards~~ ✅ (Fix C)
4. ~~Get a helpful error if `unwrap()` is forgotten on `open()`~~ ✅ (Fix D)
5. ~~Start cleanly with no "Duplicate job" warnings~~ ✅ (Fix E)
6. ~~`ntnt jobs list server.tnt` runs without side effects~~ ✅ (Fix F)
