# DD-044: Jobs DX Fixes — Addressing v0.4.6 Field Findings

**Status:** Partially Complete (5/6 fixes done)
**Author:** Larri
**Created:** 2026-03-21
**Source:** [ntnt-findings.md §5](https://app.larri.net/admin/design-docs/dd-005-ntnt-findings) — findings #25-30 from snowgauge.app build
**Implemented in:** PR #43 (Fixes C, D, E), DD-045 PRs #44-#46 (Fix A superseded), PR #54 (Fix F)
**Remaining:** Fix B (schedule smart capture)

---

## Summary

Six issues found building snowgauge.app on `std/jobs`. Two are bugs (#26, #30), three are DX improvements (#25, #27, #28), and one is cosmetic (#29). All are in `src/stdlib/jobs.rs`, `src/stdlib/concurrent.rs`, `src/stdlib/json.rs`, and `src/main.rs`.

This PR bundles them into one branch because they're small, self-contained, and share the same test app (snowgauge).

---

## Findings → Fixes

### Fix A: Auto-inject parent file imports into job perform interpreter (#25)
**Status:** ✅ SUPERSEDED by DD-045 — workers now get full app context via `create_job_interpreter()`
**File:** `src/stdlib/jobs.rs` — `execute_job_perform()`
**Effort:** Medium (2-3 hours)

**Problem:** Job perform blocks run in `Interpreter::new()` which has builtins and stdlib module registry but no imports from the parent file. Users must add `import { ... } from "std/..."` inside every perform block.

**Fix:** After `Interpreter::new()`, call `define_stdlib()` (already happens) AND replay the parent file's import statements. The job declaration is parsed from the parent AST — we can walk the AST to collect all top-level `Statement::Import` nodes and evaluate them in the fresh interpreter before running the perform body.

**Implementation:**
1. In `execute_job_perform()`, accept an additional `imports: &[Statement]` parameter
2. The `Statement::Job` evaluation in `interpreter.rs` collects all `Statement::Import` nodes from the current AST and stores them in the `JobDefinition` (new field: `imports: Vec<Statement>`)
3. `execute_job_perform()` calls `interp.eval_statement()` for each import before evaluating the perform body
4. **Result:** `now()`, `fetch()`, `stringify()` etc. work in perform blocks without manual re-import

**Tests:**
- [ ] Job perform can call `now()` from `std/time` when parent file imports it
- [ ] Job perform can call `fetch()` from `std/http` when parent file imports it
- [ ] Job perform that uses an unimported function still fails with clear error
- [ ] Imports inside perform block still work (backward compatible)

---

### Fix B: `schedule()` — capture only free variables, not entire scope (#26)
**Status:** ⏳ NOT YET IMPLEMENTED — deferred (medium effort)
**File:** `src/stdlib/concurrent.rs` — `validate_and_capture()` / `capture_bindings()`
**Effort:** Medium (2-3 hours)

**Problem:** `schedule(3600000, fn() { enqueue_all_sites() })` fails because `capture_bindings()` walks `closure.borrow().all_bindings()` — the entire environment, including user-defined functions (`home_handler`, `render_dashboard`, etc.) that the closure never references. User functions can't cross thread boundaries (`Rc<RefCell>` is not `Send`).

**Fix:** Instead of `closure.borrow().all_bindings()`, analyze the closure body's AST to determine its free variables, then capture only those bindings.

**Implementation:**
1. Add `fn free_variables(body: &Block) -> HashSet<String>` that walks the AST and collects all `Identifier` references minus locally-bound names (`let`, `for`, `fn` params)
2. In `validate_and_capture()`, compute `free_vars = free_variables(body)`
3. Filter `bindings` to only include keys in `free_vars`
4. User-defined functions that ARE referenced still fail with the existing error (correct — they genuinely can't cross boundaries)
5. User-defined functions NOT referenced are simply not captured (fix)

**Tests:**
- [ ] `schedule()` works when closure references only native functions + data, even if user functions exist in scope
- [ ] `schedule()` still fails if closure directly references a user-defined function
- [ ] `spawn()` and `after()` also benefit (same code path)
- [ ] Captured data values still work correctly

---

### Fix C: `parse_json(None)` returns `Err` instead of throwing (#27)
**Status:** ✅ COMPLETE — implemented in PR #43
**File:** `src/stdlib/json.rs` — `parse_json` function
**Effort:** Small (30 min)

**Problem:** `kv::get()` on missing key returns `Ok(None)`. Passing `None` to `parse_json()` throws a `TypeError` instead of returning `Err`. Every get→parse pipeline needs a manual guard.

**Fix:** Handle `None`/`Unit` inputs gracefully.

**Implementation:**
```rust
// Current:
func: |args| match &args[0] {
    Value::String(json_str) => { ... }
    _ => Err(IntentError::type_error("parse_json() requires a JSON string"))
}

// Fixed:
func: |args| match &args[0] {
    Value::String(json_str) => { ... }
    Value::Unit | Value::Option(None) => Ok(Value::err(Value::String(
        "parse_json(): input is None/null — did you check for a missing key?".to_string()
    ))),
    _ => Err(IntentError::type_error("parse_json() requires a JSON string"))
}
```

**Tests:**
- [x] `parse_json(None)` returns `Err("...None/null...")` — not a thrown error
- [x] `parse_json("null")` still returns `Ok(None)` (existing behavior)
- [x] `parse_json("{}")` still returns `Ok(map {})` (existing behavior)
- [x] `parse_json(42)` still throws TypeError (non-string, non-None)

---

### Fix D: Better error message for unwrapped KV handles (#28)
**Status:** ✅ COMPLETE — implemented in PR #43
**File:** `src/stdlib/kv.rs` — type check in `get`/`set`/`list`/`del`
**Effort:** Small (30 min)

**Problem:** `let cache = open("redis://...")` (without `unwrap()`) stores a `Result`. Later `get(cache, key)` says "Expected a KV store handle" — no hint about the missing unwrap.

**Fix:** When the first argument is a `Result` or `EnumValue` wrapping a KV handle, detect it and produce a better error.

**Implementation:**
In `extract_kv_handle()` (or equivalent), before the generic type error:
```rust
Value::EnumValue { variant, .. } if variant == "Ok" || variant == "Err" => {
    Err(IntentError::type_error(
        "Expected a KV store handle, got Result — did you forget to unwrap() the open() call?"
    ))
}
```

**Tests:**
- [x] `get(open("redis://..."), "key")` produces the hint message
- [x] `get(unwrap(open("redis://...")), "key")` works normally

---

### Fix E: Idempotent job registration — suppress duplicate warnings (#29)
**Status:** ✅ COMPLETE — implemented in PR #43
**File:** `src/stdlib/jobs.rs` — `register_job()`
**Effort:** Small (15 min)

**Problem:** Worker threads re-execute the .tnt file, hitting `job` declarations again. Each one logs "Duplicate job definition" as an error. 8 workers × N job types = noisy logs.

**Fix:** Make `register_job()` idempotent — if the job name already exists, silently skip (or log at debug level). The job registry is a global singleton so the first registration wins.

**Implementation:**
```rust
pub fn register_job(&self, def: JobDefinition) -> Result<()> {
    let mut registry = self.job_registry.write().map_err(|e| {
        IntentError::runtime_error(format!("Job registry lock poisoned: {}", e))
    })?;
    if registry.contains_key(&def.name) {
        // Idempotent: silently skip re-registration (workers re-execute the file)
        return Ok(());
    }
    registry.insert(def.name.clone(), def);
    Ok(())
}
```

**Tests:**
- [x] Registering same job name twice returns `Ok(())` (not error)
- [x] First registration's definition is preserved (not overwritten)
- [x] Worker startup logs are clean (no "Duplicate" warnings)

---

### Fix F: `ntnt jobs` CLI — don't re-execute the full app (#30)
**Status:** ✅ COMPLETE — implemented in PR #54 (1 line — use ExecutionMode::Worker instead of Normal)
**File:** `src/main.rs` — `jobs_load_kv()`
**Effort:** Trivial (leverages DD-045 RuntimeCapability system)

**Problem:** `ntnt jobs list server.tnt` runs `interpreter.eval(&ast)` on the entire file. This re-enqueues jobs, tries to bind the port, and spawns workers. The output is correct but mixed with startup noise.

**Fix:** Parse the file but only evaluate the `configure_queue()` call (to get the KV store URL), then connect directly.

**Implementation:**
1. Parse the AST as today
2. Walk the AST looking for `Statement::Expression` that is a function call to `configure_queue`
3. Extract the `"store"` value from the map literal argument (static analysis — covers the common case)
4. If found: open the KV store directly via `kv::open()` with that URL. Skip full eval.
5. If not found (dynamic config): fall back to full eval with a `--force-eval` flag, or accept `--store redis://...` as an explicit override
6. Alternative simpler approach: add `--store redis://...` CLI flag that bypasses file execution entirely

**Simpler approach (--store flag):**
```
ntnt jobs list --store redis://localhost:6379
ntnt jobs status --store redis://localhost:6379
ntnt jobs inspect --store redis://localhost:6379 <job-id>
```
This is the most robust fix — no file parsing needed at all. The `.tnt` file argument becomes optional when `--store` is provided.

**Tests:**
- [ ] `ntnt jobs list --store redis://... --status completed` works without running the app
- [ ] `ntnt jobs status --store redis://...` shows counts without side effects
- [ ] `ntnt jobs list server.tnt` still works as fallback (existing behavior)
- [ ] No duplicate job enqueues when using `--store`

---

## Implementation Order

| PR | Fix | Effort | Risk |
|----|-----|--------|------|
| 1  | E: Idempotent job registration | 15 min | None — behavior change is strictly less noisy |
| 2  | C: `parse_json(None)` → `Err` | 30 min | Low — new code path for None, existing paths unchanged |
| 3  | D: Better KV handle error message | 30 min | None — error message improvement only |
| 4  | F: `--store` flag for jobs CLI | 2-3 hr | Low — additive flag, existing behavior preserved |
| 5  | A: Auto-inject imports into job perform | 2-3 hr | Medium — touches interpreter eval path |
| 6  | B: Smart capture for schedule() | 2-3 hr | Medium — touches concurrent capture logic |

**Total estimated effort:** 8-10 hours across 6 focused PRs (or bundle into 1-2 larger PRs).

**Suggested bundling:**
- **PR 1:** Fixes E + C + D (quick wins, <2 hours total)
- **PR 2:** Fixes A + B (interpreter/concurrent changes, shared test patterns)
- **PR 3:** Fix F (CLI-only change, independent)

---

## Validation

After all fixes, snowgauge.app should:
1. Work without any imports inside `perform` blocks (Fix A)
2. Use `schedule()` directly instead of the `enqueue_in` self-scheduling workaround (Fix B)
3. Use `parse_json(get(kv, key))` without guards (Fix C — pipeline returns Err on missing)
4. Get a helpful error if `unwrap()` is forgotten on `open()` (Fix D)
5. Start cleanly with no "Duplicate job" warnings (Fix E)
6. Allow `ntnt jobs list --store redis://redis:6379` without re-running the app (Fix F)
