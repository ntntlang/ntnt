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

## Completed Fixes

### Fix A: Job perform blocks lack imports (#25) — ✅ SUPERSEDED

**Original problem:** Job perform blocks ran in `Interpreter::new()` with no imports from the parent file.

**Resolution:** DD-045 (PRs #44-#46) replaced `execute_job_perform()` with `create_job_interpreter()`, which evaluates the entire source file in Worker mode. Workers now have full app context — all imports, functions, and constants. This is a fundamentally better solution than the import-replay approach originally proposed here.

---

### Fix C: `parse_json(None)` returns `Err` instead of throwing (#27) — ✅ COMPLETE

Implemented in PR #43. `parse_json(None)` now returns `Err("parse_json(): input is None/null")` instead of throwing a TypeError. Enables clean `get → parse_json` pipelines.

---

### Fix D: Better error for unwrapped KV handles (#28) — ✅ COMPLETE

Implemented in PR #43. `get(open("redis://..."), "key")` now says "did you forget to unwrap() the open() call?" instead of the generic "Expected a KV store handle."

---

### Fix E: Idempotent job registration (#29) — ✅ COMPLETE

Implemented in PR #43. `register_job()` silently skips when a job with the same name is already registered. Worker startup logs are clean.

---

### Fix F: `ntnt jobs` CLI side effects (#30) — ✅ COMPLETE

Implemented in PR #54. `jobs_load_kv()` now evaluates in `ExecutionMode::Worker` — `listen()`, `enqueue()`, `work_async()`, and `schedule()` are all suppressed. One line change leveraging the DD-045 RuntimeCapability system.

---

## Remaining Fix

### Fix B: `schedule()` captures entire scope, fails on user functions (#26) — ⏳ OPEN

**File:** `src/stdlib/concurrent.rs` — `validate_and_capture()` / `capture_bindings()`

**Problem:** `schedule(3600000, fn() { enqueue_all_sites() })` fails because `capture_bindings()` walks `closure.borrow().all_bindings()` — the entire environment. User-defined functions (like route handlers) can't cross thread boundaries (`Rc<RefCell>` is not `Send`), so `schedule()` fails even when the closure never references them.

**Current workaround:** Use `enqueue_in()` self-scheduling pattern instead of `schedule()`.

**Proposed fix (simplified):** Instead of full free-variable analysis (AST walking), use a simpler approach — try to capture each binding individually and skip any that fail serialization, with a warning:

```rust
fn capture_bindings(bindings: &HashMap<String, Value>) -> Result<CapturedBindings, Vec<String>> {
    let mut values = HashMap::new();
    let mut native_fns = Vec::new();
    let mut skipped_fns = Vec::new();

    for (key, value) in bindings {
        match value {
            Value::NativeFunction { .. } => {
                // Capture native functions (they're Send-safe)
                native_fns.push(/* ... */);
            }
            Value::Function { .. } => {
                // User-defined closures can't cross threads — skip silently.
                // If the closure body actually references this function,
                // it will fail at runtime with a clear "undefined" error.
                skipped_fns.push(key.clone());
            }
            _ => {
                // Try to serialize data values
                match SerializedValue::from_value(value) {
                    Ok(serialized) => { values.insert(key.clone(), serialized); }
                    Err(_) => { /* skip non-serializable values */ }
                }
            }
        }
    }

    if !skipped_fns.is_empty() {
        eprintln!(
            "[dev] schedule/spawn: skipping {} user-defined function(s) \
             that can't cross thread boundaries: {}",
            skipped_fns.len(),
            skipped_fns.join(", ")
        );
    }

    Ok(CapturedBindings { values, native_fns })
}
```

**Why this is simpler than free-variable analysis:**
- No AST walking needed
- No new `free_variables()` function
- Change is isolated to `capture_bindings()` — one function
- Behavior: user functions are skipped instead of causing an error
- If the closure actually needs a skipped function, it gets a clear "undefined variable" runtime error
- Dev-mode warning shows what was skipped so the developer knows why

**Risk:** Low. The only behavior change is: closures that DON'T reference user functions now succeed (previously they failed). Closures that DO reference them get a different error ("undefined variable" instead of "cannot capture"). Both errors are clear.

**Tests:**
- [ ] `schedule()` works when closure references only native functions + data, even if user functions exist in scope
- [ ] `schedule()` closure that references a user-defined function fails with "undefined" error
- [ ] `spawn()` and `after()` also benefit (same code path)
- [ ] Captured data values still work correctly
- [ ] Dev warning printed listing skipped functions

---

## Validation

After all fixes, snowgauge.app should:
1. ~~Work without any imports inside `perform` blocks~~ ✅ (DD-045)
2. Use `schedule()` directly instead of the `enqueue_in` self-scheduling workaround (Fix B — pending)
3. ~~Use `parse_json(get(kv, key))` without guards~~ ✅ (Fix C)
4. ~~Get a helpful error if `unwrap()` is forgotten on `open()`~~ ✅ (Fix D)
5. ~~Start cleanly with no "Duplicate job" warnings~~ ✅ (Fix E)
6. ~~`ntnt jobs list server.tnt` runs without side effects~~ ✅ (Fix F)
