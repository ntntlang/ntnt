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
**Effort:** Medium-Large (4-6 hours including tests)
**Risk:** Medium — under-capture causes runtime failures instead of compile-time errors

---

#### Scenario

A developer builds a web app with scheduled background work:

```ntnt
import { fetch } from "std/http"
import { html } from "std/http/server"

let API_URL = "https://api.weather.gov/stations"

fn home(req) { return html("<h1>Weather</h1>") }
fn dashboard(req) { return html("<h1>Dashboard</h1>") }

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

#### Problem

`schedule()` moves the closure to a new thread. `validate_and_capture()` calls `closure.borrow().all_bindings()` which grabs **everything** in scope:

| Binding | Type | Used by closure? | Can cross thread? |
|---------|------|:---:|:---:|
| `fetch` | NativeFunction | No | ✅ |
| `html` | NativeFunction | No | ✅ |
| `API_URL` | String | No | ✅ |
| `home` | Function (Rc\<RefCell\>) | **No** | ❌ |
| `dashboard` | Function (Rc\<RefCell\>) | **No** | ❌ |
| `refresh_weather` | Function (Rc\<RefCell\>) | **Yes** | ❌ |

`capture_bindings()` hits `home` and `dashboard` — user functions the closure never references — and fails. The error message lists all three functions, obscuring which one the closure actually needs.

#### Fix: Scope-Aware Free-Variable Analysis

Compute the set of identifiers the closure body actually references (free variables), then only capture those bindings from the scope. This is a standard compiler technique but must be implemented correctly for ntnt's AST.

**Key insight:** `collect_used_names()` in `main.rs` (line 4557) already walks the entire AST for import analysis. We adapt this with scope tracking — names bound locally are subtracted from the free set.

##### Architecture

```
free_variables(body: &Block) -> HashSet<String>
  └── collect_free_vars(stmts, referenced, bound)     // statement walker
        └── collect_free_vars_expr(expr, referenced, bound)  // expression walker
              └── names_bound_by_pattern(pattern) -> HashSet<String>  // pattern helper
```

##### Critical Design Decision: Scope Snapshots

The `bound` set must be **snapshot/restored at scope boundaries**, not shared mutably across siblings. Without this:

```ntnt
fn() {
    if cond {
        let x = 1    // binds x in this branch
    }
    print(x)          // x is FREE here — not bound in this scope
}
```

A naive `bound.insert("x")` would persist across the `if` boundary, incorrectly marking the outer `x` reference as locally bound. Fix: clone `bound` before entering each scope-forming construct (Block, If branches, While, Loop, ForIn, Lambda, Function, Match arms, TryCatch).

##### Expression Variants to Handle

| Expression | Action | Notes |
|-----------|--------|-------|
| `Identifier(name)` | Add to `referenced` if not in `bound` | Core case |
| `Call { function, arguments }` | Recurse into both | |
| `MethodCall { object, method, arguments }` | Recurse object + args, **add `method` to `referenced`** | ntnt looks up `method` in environment (interpreter.rs:5868) |
| `FieldAccess { object, field }` | Recurse object only | `field` is a property name, not a variable |
| `Binary`, `Unary` | Recurse operands | |
| `Index { object, index }` | Recurse both | |
| `Array(items)` | Recurse each | |
| `MapLiteral(pairs)` | Recurse keys + values | |
| `Range { start, end }` | Recurse both | |
| `InterpolatedString(parts)` | Recurse `StringPart::Expr` parts | |
| `TemplateString(parts)` | Recurse all embedded expressions | `ForLoop` binds `var` + 7 implicit loop metadata variables (`@index`, `@index1`, `@first`, `@last`, `@length`, `@even`, `@odd`) in the body scope. Also handle `IfBlock`, `Partial`, filter args. |
| `StructLiteral { name, fields }` | `name` is a type ref (skip), recurse field values | |
| `EnumVariant { arguments }` | Recurse arguments | `enum_name`/`variant` are type refs |
| `Lambda { params, body }` | **New scope**: bind param names (including pattern names), recurse param defaults left-to-right (each default only sees earlier params), recurse body with cloned `bound` | Nested closure — inner bindings must NOT leak to outer. Param defaults can reference free variables. |
| `Block(block)` | **New scope**: clone `bound`, recurse | |
| `IfExpr { condition, then, else }` | Recurse all three | |
| `Match { scrutinee, arms }` | Recurse scrutinee; for each arm: bind pattern names, recurse guard + body | Pattern bindings are scoped to the arm |
| `Assign { target, value }` | Recurse both | |
| `Await(inner)`, `Try(inner)` | Recurse | |
| `TryCatch { body }` | **New scope**: recurse | |
| Literals (`Integer`, `Float`, `String`, `Bool`, `Unit`) | Skip | |

##### Statement Variants to Handle

| Statement | Bindings | Sub-expressions | Notes |
|-----------|----------|-----------------|-------|
| `Let { name, pattern, value, otherwise }` | Bind `name`/pattern names AFTER recursing value | **Order matters:** (1) recurse `value` first (initializer can't reference its own binding), (2) recurse `otherwise` in a **fresh scope** containing only `err` (the `let` name is NOT in scope there), (3) THEN add `name`/pattern bindings to `bound` for subsequent sibling statements | `otherwise` block sees `err` but not the `let` binding |
| `Function { name, params, body, contract }` | Bind `name` in enclosing scope; bind params (including pattern names) in body scope | **Param defaults evaluated left-to-right** — each default only sees earlier params as bound. Recurse defaults, contract requires/ensures, body. Destructured params bind pattern names, not the synthetic `param.name`. | **New scope** for body |
| `Expression(expr)` | — | Recurse | |
| `Return(Some(expr))` | — | Recurse | |
| `If { condition, then, else }` | — | Recurse all; **clone bound** for each branch | Bindings in one branch don't leak to siblings |
| `While { condition, body }` | — | Recurse; **new scope** for body | |
| `ForIn { variable, pattern, iterable, body }` | Bind `variable` or pattern names **in body scope** | Recurse iterable, body | |
| `Loop { body }` | — | **New scope** for body | |
| `Defer(expr)` | — | Recurse | |
| `Module { body }` | — | Recurse statements | |
| `Export { statement }` | — | Recurse inner statement | |
| `Intent { target }` | — | Recurse into `target` statement | Desugars to eval of target |
| `Impl { methods, invariants }` | — | Recurse | |
| `Server { port, directives, routes, groups }` | — | Recurse all expressions | |
| `Job { perform_body, on_failure, options }` | Bind perform params in body; bind on_failure params | Recurse bodies and option expressions | |
| `Located { stmt }` | — | **Unwrap and recurse** | |
| `Break`, `Continue`, `Return(None)` | — | — | |
| `Import`, `Use`, `TypeAlias`, `Struct`, `Enum`, `Trait` | — | — | No runtime variable references |

##### Pattern Name Extraction

```rust
fn names_bound_by_pattern(pattern: &Pattern) -> HashSet<String> {
    match pattern {
        Pattern::Variable(name) => [name.clone()].into(),
        Pattern::Wildcard => HashSet::new(),
        Pattern::Literal(_) => HashSet::new(),
        Pattern::Array { elements, rest } => {
            let mut names: HashSet<_> = elements.iter()
                .flat_map(|p| names_bound_by_pattern(p)).collect();
            if let Some(rest_name) = rest { names.insert(rest_name.clone()); }
            names
        }
        Pattern::Map { fields, rest } => {
            let mut names: HashSet<_> = fields.iter()
                .flat_map(|(_, p)| names_bound_by_pattern(p)).collect();
            if let Some(rest_name) = rest { names.insert(rest_name.clone()); }
            names
        }
        Pattern::Struct { fields, .. } => fields.iter()
            .flat_map(|(_, p)| names_bound_by_pattern(p)).collect(),
        Pattern::Variant { fields, .. } => fields.as_ref()
            .map(|fs| fs.iter().flat_map(|p| names_bound_by_pattern(p)).collect())
            .unwrap_or_default(),
        Pattern::Tuple(patterns) => patterns.iter()
            .flat_map(|p| names_bound_by_pattern(p)).collect(),
    }
}
```

##### Integration Point

In `validate_and_capture()` (concurrent.rs:1204):

```rust
let free_vars = free_variables(body);
let all_bindings = closure.borrow().all_bindings();
let needed: HashMap<_, _> = all_bindings.into_iter()
    .filter(|(k, _)| free_vars.contains(k))
    .collect();
let captured = capture_bindings(&needed).map_err(|names| {
    // Error now only lists functions the closure ACTUALLY references
    IntentError::runtime_error(format!(
        "Cannot capture user-defined function(s) across task boundaries: {}. \
         These functions use Rc<RefCell> which cannot be sent between threads. \
         Inline the function body into the closure, or call a native function instead.",
        names.join(", ")
    ))
})?;
```

##### Result After Fix

```ntnt
// Works ✅ — only captures refresh_weather, not home/dashboard
schedule(3600000, fn() { refresh_weather() })
// Error is accurate: "Cannot capture 'refresh_weather'"
// Developer inlines the logic:
schedule(3600000, fn() {
    let data = fetch(API_URL)
    // ...
})
// Works ✅ — only captures fetch (native) and API_URL (string)
```

##### Risks

1. **Under-capture** — if the walker misses a free variable, the error shifts from capture-time ("cannot capture X") to runtime ("undefined variable X"). Mitigation: comprehensive test coverage for every AST node type.
2. **MethodCall.method** — easy to miss. ntnt resolves `obj.method(args)` by looking up `method` in the environment (interpreter.rs:5868). The walker must treat `method` as a referenced identifier.
3. **Pipe operator** — NOT a separate AST node. Desugared to `Call` during parsing (parser.rs:1415). No special handling needed.

---

#### Implementation Checklist

- [ ] Add `free_variables(body: &Block) -> HashSet<String>` in `concurrent.rs`
- [ ] Add `collect_free_vars(stmts, referenced, bound)` — statement walker with scope cloning
- [ ] Add `collect_free_vars_expr(expr, referenced, bound)` — expression walker (all variants from table above)
- [ ] Add `names_bound_by_pattern(pattern) -> HashSet<String>` — pattern helper
- [ ] Handle scope boundaries: clone `bound` at Block, If branches, While, Loop, ForIn, Lambda, Function, Match arms, TryCatch
- [ ] Handle `MethodCall.method` as a referenced identifier
- [ ] Handle `TemplateString` embedded expressions including `ForLoop.var` binding
- [ ] Handle `otherwise` implicit `err` binding
- [ ] Handle `Statement::Located` unwrapping
- [ ] Handle `Statement::Intent` — recurse into `target`
- [ ] Handle `Let` ordering: recurse value → recurse otherwise (fresh scope with `err`) → bind name/pattern
- [ ] Handle `Function` param defaults left-to-right (each default only sees earlier params)
- [ ] Handle `Lambda` param defaults (same semantics as Function)
- [ ] Handle destructured param patterns (bind pattern names, not synthetic param.name)
- [ ] Handle template `ForLoop` implicit bindings (`@index`, `@index1`, `@first`, `@last`, `@length`, `@even`, `@odd`)
- [ ] Update `validate_and_capture()` to filter bindings through `free_vars`
- [ ] Tests: `schedule()` succeeds with native fns + data when user functions exist in scope
- [ ] Tests: `schedule()` fails with clear error when closure references a user-defined function
- [ ] Tests: `spawn()` and `after()` also benefit (same code path)
- [ ] Tests: captured data values still work correctly
- [ ] Tests: nested closures don't leak inner bindings to outer scope
- [ ] Tests: destructuring patterns correctly bind names
- [ ] Tests: match arm bindings scoped correctly
- [ ] Tests: free_variables handles all expression types (Identifier, Call, MethodCall, FieldAccess, etc.)

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

### Fix F: `ntnt jobs` CLI re-executes the full app (#30) — ✅ COMPLETE (PR #54, pending merge)

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
