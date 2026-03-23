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

#### The Scenario

A developer builds a web app with scheduled background work:

```ntnt
import { fetch } from "std/http"
import { stringify } from "std/json"
import { html } from "std/http/server"

let API_URL = "https://api.weather.gov/stations"

// Route handlers (user-defined functions)
fn home(req) { return html("<h1>Weather</h1>") }
fn dashboard(req) { return html("<h1>Dashboard</h1>") }

// Background task — refresh weather data every hour
fn refresh_weather() {
    let data = fetch(API_URL)
    // ... store in KV ...
}

get("/", home)
get("/dashboard", dashboard)

// This fails ❌
schedule(3600000, fn() { refresh_weather() })

listen(8080)
```

#### The Problem

`schedule()` needs to move the closure to a new thread. To do this, `validate_and_capture()` calls `closure.borrow().all_bindings()` which grabs **everything** in the current scope — not just what the closure references. This includes:

| Binding | Type | Referenced by closure? | Can cross thread? |
|---------|------|----------------------|-------------------|
| `fetch` | NativeFunction | No | ✅ Yes |
| `stringify` | NativeFunction | No | ✅ Yes |
| `html` | NativeFunction | No | ✅ Yes |
| `API_URL` | String | No | ✅ Yes |
| `home` | Function (Rc\<RefCell\>) | No | ❌ No |
| `dashboard` | Function (Rc\<RefCell\>) | No | ❌ No |
| `refresh_weather` | Function (Rc\<RefCell\>) | **Yes** | ❌ No |

`capture_bindings()` hits `home` and `dashboard`, sees they're `Value::Function` (which contains `Rc<RefCell<Environment>>` — not `Send`), and fails with: "Cannot capture user-defined function(s) across task boundaries: home, dashboard, refresh_weather."

The developer's closure only needs `refresh_weather`, but the capture system grabs everything and chokes on unrelated route handlers.

#### The Fix: Free-Variable Analysis

Only capture bindings that the closure body actually references. This is a standard compiler technique — walk the AST, collect identifier references, subtract locally-bound names.

**Step 1:** Add `free_variables(body: &Block) -> HashSet<String>` to `concurrent.rs`:

```rust
/// Walk a block's AST and collect all referenced identifiers,
/// minus names bound locally (let, for, fn params). What remains
/// are free variables that must be captured from the enclosing scope.
fn free_variables(body: &Block) -> HashSet<String> {
    let mut referenced = HashSet::new();
    let mut locally_bound = HashSet::new();
    collect_free_vars(&body.statements, &mut referenced, &mut locally_bound);
    referenced.difference(&locally_bound).cloned().collect()
}

fn collect_free_vars(
    stmts: &[Statement],
    referenced: &mut HashSet<String>,
    bound: &mut HashSet<String>,
) {
    for stmt in stmts {
        match stmt {
            Statement::Let { name, value, .. } => {
                // Visit the value expression first (it can reference outer scope)
                collect_free_vars_expr(value, referenced, bound);
                // Then bind the name (not visible to the value expression)
                bound.insert(name.clone());
            }
            Statement::Expression(expr) => {
                collect_free_vars_expr(expr, referenced, bound);
            }
            // ... handle For, If, Return, etc.
        }
    }
}

fn collect_free_vars_expr(
    expr: &Expression,
    referenced: &mut HashSet<String>,
    bound: &mut HashSet<String>,
) {
    match expr {
        Expression::Identifier(name) => {
            if !bound.contains(name) {
                referenced.insert(name.clone());
            }
        }
        Expression::Call { function, arguments } => {
            collect_free_vars_expr(function, referenced, bound);
            for arg in arguments {
                collect_free_vars_expr(arg, referenced, bound);
            }
        }
        // ... handle other expression types
    }
}
```

**Step 2:** Use it in `validate_and_capture()`:

```rust
fn validate_and_capture(caller: &str, handler: &Value) -> Result<(CapturedBindings, Block)> {
    match handler {
        Value::Function { params, closure, body, .. } => {
            // ... existing param check ...

            // Only capture what the closure body actually references
            let free_vars = free_variables(body);
            let all_bindings = closure.borrow().all_bindings();
            let needed: HashMap<String, Value> = all_bindings
                .into_iter()
                .filter(|(k, _)| free_vars.contains(k))
                .collect();

            let captured = capture_bindings(&needed).map_err(|names| {
                // Error now only fires for functions the closure ACTUALLY uses
                IntentError::runtime_error(format!(
                    "Cannot capture user-defined function(s) across task boundaries: {}. \
                     These functions use Rc<RefCell> which is not Send. \
                     Consider moving the logic into the closure body or using a native function.",
                    names.join(", ")
                ))
            })?;
            Ok((captured, body.clone()))
        }
        // ...
    }
}
```

#### Result After Fix

The same code now works:

```ntnt
// This succeeds ✅
schedule(3600000, fn() { refresh_weather() })
```

Free-variable analysis determines the closure only references `refresh_weather`. `home` and `dashboard` are never touched. `capture_bindings()` only sees `refresh_weather` — which IS a user function the closure needs. Since it can't cross threads, the error message is now accurate and actionable:

"Cannot capture user-defined function 'refresh_weather' across task boundaries."

The developer knows exactly which function is the problem and can inline its logic:

```ntnt
// Inlined — works ✅
schedule(3600000, fn() {
    let data = fetch(API_URL)
    // ... store in KV ...
})
```

And closures that only use native functions + data work without any changes:

```ntnt
// Only references native fetch + string API_URL — works ✅
schedule(3600000, fn() {
    fetch("#{API_URL}/refresh")
})
```

#### Implementation

- [ ] Add `free_variables(body: &Block) -> HashSet<String>` — AST walker (~30-40 lines)
- [ ] Add `collect_free_vars()` and `collect_free_vars_expr()` — recursive helpers
- [ ] Update `validate_and_capture()` to filter bindings through `free_vars`
- [ ] Tests: `schedule()` works when closure uses only native functions + data, even with user functions in scope
- [ ] Tests: `schedule()` fails with clear error when closure references a user-defined function
- [ ] Tests: `spawn()` and `after()` also benefit (same code path)
- [ ] Tests: captured data values still work correctly
- [ ] Tests: free_variables correctly identifies identifiers in nested blocks, calls, and control flow

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
