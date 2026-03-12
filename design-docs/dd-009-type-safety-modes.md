# DD-009: ntnt Type Safety Modes — Two Axes of Type Control

**Status:** in-review  
**Author:** Larri  
**Date:** 2026-03-08  
**Builds on:** DD-008 (Type Resilience)  
**Triggered by:** Discussion about whether DD-008's forgiving runtime behavior is unconditionally good, or whether it trades safety for uptime in ways that should be explicit

---

## The Problem

DD-008 introduced runtime type resilience: `[]` returns `None` on type mismatch instead of crashing, `for..in` skips non-collections, template errors degrade gracefully. This solved real production crashes — 500 errors from bad database data taking down entire pages.

But it also introduced a new class of risk: **silent correctness failures.**

```ntnt
// Auth bypass: banned user's flag has wrong type → None → falsy → not banned
let blocked = user["is_banned"]  // None due to type mismatch
if blocked {
    return redirect("/banned")
}
// banned user proceeds normally

// Financial: price has wrong type → None → total is wrong
let price = item["price"]  // None due to mismatch
let total = price * quantity  // 0? None? Wrong charge either way
```

The core security principle: **fail-closed vs fail-open.** A lock that breaks should stay locked, not swing open. DD-008 made ntnt fail-open by default. That's fine for content sites; it's dangerous for anything with auth, payments, or safety implications.

Additionally, ntnt's type story is incomplete. The language has gradual typing (optional type annotations) and a static lint checker, but there's no unified model for how strictness scales across the development lifecycle. You can't independently control "how strict is my code analysis?" and "how strict is my runtime?"

---

## Prior Art

| Language | Approach | Levels | Scope |
|----------|----------|--------|-------|
| **TypeScript** | Granular compiler flags | 8+ strict flags, `"strict": true` enables all | Per-project (tsconfig.json) |
| **Perl** | Pragmas | `use strict` (error) + `use warnings` (warn) | Per-file |
| **Ruby/Sorbet** | Typed sigils | 5 levels: ignore → false → true → strict → strong | Per-file |
| **Go** | No dial | One mode, errors are errors, no warnings | Global (no choice) |
| **Rust** | Safe by default | `unsafe {}` blocks to opt out, no strictness dial | Per-block |
| **Python/mypy** | Flags | `--strict` (all-or-nothing) | Per-project |

**Key insight from TypeScript:** People adopt strictness incrementally, one flag at a time. But 8+ flags creates a maintenance burden. **Key insight from Sorbet:** A linear progression on a single axis (ignore → false → true → strict → strong) is maintainable and intuitive. **Key insight from Go/Rust:** Languages with one mode are simpler but can't accommodate the prototyping-to-production lifecycle.

**ntnt's position:** Gradually typed, designed for rapid prototyping and AI-agent authorship. Needs the dial, but must keep it simple. Three levels per axis, not eight flags.

---

## Proposal: Two Independent Axes

ntnt's type system is controlled by two independent axes:

### Axis 1: Static Analysis (Lint) — "How thoroughly is my code checked before it runs?"

Controlled by `ntnt lint` flags. Determines what the linter reports.

| Level | Flag | What it checks | CI behavior |
|-------|------|---------------|-------------|
| **default** | `ntnt lint` | Syntax errors + type errors where annotations exist. If you wrote `fn add(a: Int, b: Int)` and call it with a string, that's an error. If you didn't annotate, no complaint. | Fails on type conflicts in annotated code |
| **warn** | `ntnt lint --warn-untyped` | Everything above + warnings for unannotated code. "This function has no type signature — consider adding one." | Fails on type conflicts; warns on missing annotations (non-fatal) |
| **strict** | `ntnt lint --strict` | Everything above, but unannotated code is an error. Every function needs signatures. Every variable needs a type. Full static coverage. | Fails on any missing annotation or type conflict |

**`ntnt lint --strict` effectively makes ntnt a statically typed language.** This is the "production-hardened codebase" mode. AI agents can easily generate full annotations; human prototypers can opt out.

**Important:** Default lint always catches type conflicts in annotated code. You can never declare types and have the linter ignore a mismatch. The dial only controls what happens with *unannotated* code.

### Axis 2: Runtime Behavior — "What happens when types mismatch while the app is running?"

Controlled by `NTNT_TYPE_MODE` env var or `ntnt.toml` config. Determines what happens at runtime when actual data doesn't match expected types (bad database values, malformed API responses, user input edge cases).

| Level | Config | Runtime behavior | Use case |
|-------|--------|-----------------|----------|
| **strict** | `NTNT_TYPE_MODE=strict` | Type mismatches are runtime errors. `[]` on wrong type crashes. `for..in` on non-collection crashes. Template type errors return 500. | Apps with auth, payments, safety-critical logic. Fail-closed. |
| **warn** | `NTNT_TYPE_MODE=warn` | Type mismatches log `[WARN]` with file/line/details but the app keeps running. Same graceful degradation as forgiving, but with a full paper trail. | Production apps where you want visibility without downtime. Pair with log monitoring. |
| **forgiving** | `NTNT_TYPE_MODE=forgiving` | Silent degradation. `[]` returns `None`, `for..in` skips, templates render empty. No warnings, no logs, maximum uptime. | Content sites, blogs, dashboards where uptime > correctness. |

### Axis Independence

The two axes are fully independent. You configure them separately based on your needs:

| Scenario | Lint | Runtime | Why |
|----------|------|---------|-----|
| **Prototype** | default | forgiving | Move fast, don't fight types, just make it work |
| **Growing project** | `--warn-untyped` | warn | Start seeing what needs types, catch runtime issues in logs |
| **Production SaaS** | `--strict` | strict | Full static coverage, crash on bad data, fail-closed |
| **Production content site** | `--strict` | warn | Code is fully typed and verified, but bad external data doesn't take down the site |
| **CI pipeline** | `--strict` | n/a | Lint runs at build time, runtime mode is deployment config |

### Defaults

| Context | Lint default | Runtime default |
|---------|-------------|----------------|
| `ntnt lint` (no flags) | default (check annotated code only) | n/a |
| `NTNT_ENV=development` | n/a | **warn** |
| `NTNT_ENV=production` | n/a | **warn** |
| Explicit `NTNT_TYPE_MODE` set | n/a | Whatever was set |

**Why `warn` as the runtime default (not `strict`):** Every existing ntnt app was written under forgiving runtime behavior. Flipping to `strict` would break deployed apps on edge cases that were previously silently handled. `warn` is the safe migration path — it surfaces issues in logs without breaking anything. Strict is opt-in for now, with a path to making it the default in a future major version once the ecosystem has adapted.

**Migration path:**
1. **v0.3.x (now):** Forgiving behavior, no mode flag (DD-008 status quo)
2. **v0.4.0:** Introduce `NTNT_TYPE_MODE`, default to `warn`. Existing apps start seeing warnings in logs but don't break.
3. **v0.5.0 (future):** Consider changing default to `strict` once apps have had time to address warnings. Announce in advance.

---

## Configuration

### Environment Variables — One Per Axis

```bash
# Axis 1: Static analysis (lint)
NTNT_LINT_MODE=default       # default | warn | strict

# Axis 2: Runtime behavior
NTNT_TYPE_MODE=warn          # strict | warn | forgiving
```

`ntnt lint` with no CLI flags reads `NTNT_LINT_MODE` and applies that level automatically. Each app configures its own defaults — no need to remember flags.

### ntnt.toml

```toml
[types]
lint_mode = "default"        # default | warn | strict
runtime_mode = "warn"        # strict | warn | forgiving
```

### Docker / docker-compose.yml

```yaml
# SaaS app with auth + payments
environment:
  - NTNT_LINT_MODE=strict
  - NTNT_TYPE_MODE=strict
  - NTNT_ENV=production

# Content site / blog
environment:
  - NTNT_LINT_MODE=warn
  - NTNT_TYPE_MODE=warn
  - NTNT_ENV=production

# Prototype / hackathon
environment:
  - NTNT_LINT_MODE=default
  - NTNT_TYPE_MODE=forgiving
```

### Precedence

```
CLI flag > Environment variable > ntnt.toml > built-in default
```

- `ntnt lint --strict` overrides `NTNT_LINT_MODE=warn`
- `NTNT_TYPE_MODE=strict` overrides `ntnt.toml` `runtime_mode = "warn"`
- If nothing is set: lint defaults to `default`, runtime defaults to `warn`

This follows the same precedence pattern as `NTNT_ENV` and other existing env vars.

---

## Implementation Scope

### Runtime Mode (Interpreter Changes)

The DD-008 resilience code paths already exist. Implementation is adding a mode check at each resilience point:

**Affected code paths in `interpreter.rs`:**

1. **`[]` index on type mismatch** — Currently returns `None`. Add match on mode: `strict` → return `RuntimeError`, `warn` → log + return `None`, `forgiving` → return `None`.

2. **`for..in` on non-collection** — Currently skips with dev-mode warning. Add match: `strict` → return `RuntimeError`, `warn` → log `[WARN]` + skip, `forgiving` → skip silently.

3. **Template error boundaries** — Currently render empty/comment. Add match: `strict` → return 500, `warn` → log + render empty, `forgiving` → render empty.

**Estimated touch points:** ~5-8 locations in interpreter.rs, ~2-3 in template.rs. Each is a simple match on an enum.

```rust
enum TypeMode {
    Strict,
    Warn,
    Forgiving,
}

// At each resilience point:
match type_mode {
    TypeMode::Strict => return Err(RuntimeError::TypeMismatch { ... }),
    TypeMode::Warn => {
        eprintln!("[WARN] Type mismatch at {}:{}: ...", file, line);
        // continue with graceful behavior
    },
    TypeMode::Forgiving => {
        // continue with graceful behavior, no log
    },
}
```

### Lint Changes

1. **`--warn-untyped` flag:** Walk AST after parsing, identify functions without parameter/return type annotations, variables without type annotations. Emit warnings (non-fatal exit code).

2. **`--strict` flag (enhanced):** Same as `--warn-untyped` but warnings become errors (fatal exit code). This already partially exists as `NTNT_STRICT=1` — extend it to cover the unannotated code case.

3. **Lint always checks annotated code:** No change needed — the type checker already does this.

### Test Coverage

| Test | What it verifies |
|------|-----------------|
| `test_strict_mode_crashes_on_type_mismatch` | `[]` on wrong type returns RuntimeError in strict mode |
| `test_warn_mode_logs_and_continues` | `[]` on wrong type logs warning, returns None |
| `test_forgiving_mode_silent` | `[]` on wrong type returns None, no log output |
| `test_for_in_strict_crashes` | `for..in` on string/int crashes in strict mode |
| `test_for_in_warn_logs` | `for..in` on non-collection logs and skips |
| `test_template_strict_500` | Template type error returns 500 in strict mode |
| `test_template_warn_logs` | Template type error logs and renders empty |
| `test_env_var_precedence` | `NTNT_TYPE_MODE` overrides ntnt.toml |
| `test_lint_warn_untyped` | Untyped functions produce warnings with `--warn-untyped` |
| `test_lint_strict_untyped_fails` | Untyped functions are errors with `--strict` |
| `test_lint_always_catches_annotated_conflicts` | Type conflicts in annotated code fail at all lint levels |

---

## Security Considerations

**Recommendation for app developers:**
- Apps with authentication, authorization, or permission checks → `strict` runtime
- Apps handling financial transactions or sensitive data → `strict` runtime  
- Apps where data correctness has safety implications → `strict` runtime
- Content sites, blogs, informational dashboards → `warn` runtime is acceptable
- Never use `forgiving` in production unless you have external monitoring

**Documentation must be clear:** Forgiving mode is not "recommended for production." It exists for specific use cases where teams have made a conscious decision that availability outweighs correctness. The blog post / docs should frame this as a tradeoff with real consequences, not a feature to celebrate.

---

## Blog Post: "Two Dials for Type Safety"

This design doc doubles as the foundation for a blog post on ntnt-lang.org. The post would cover:

1. **The problem:** Static vs dynamic, strong vs weak — what do these actually mean?
2. **The tradeoff:** Fail-closed (crash, safe) vs fail-open (degrade, available)
3. **Prior art:** How TypeScript, Rust, Go, Ruby/Sorbet, Perl each chose differently
4. **ntnt's answer:** Two independent axes — lint strictness (how thoroughly is code checked?) and runtime mode (what happens when types mismatch at runtime?)
5. **The honest framing:** These are tradeoffs with real consequences. Security-critical apps should fail closed. The language gives you the dial, not the answer.

**Tone:** Not "look how smart our type system is." More "here's what we learned shipping real apps about when types help and when they hurt, and how we're letting developers make that choice explicitly."

---

---

## Comprehensive Type System Audit (v0.4.0)

Full review of every type boundary in the ntnt runtime and type checker. Categorized by coverage status.

### Value Types (Runtime)

ntnt has 12 `Value` variants:

| Value Variant | Type System Coverage | TypeMode-Aware? | Notes |
|--------------|---------------------|-----------------|-------|
| `Unit` | ✅ Lint + Runtime | N/A | No operations produce type mismatches |
| `Int(i64)` | ✅ Lint + Runtime | ✅ Index, for..in | Arithmetic, comparison, conversion all handled |
| `Float(f64)` | ✅ Lint + Runtime | ✅ Index, for..in | Mixed Int/Float auto-promotion works |
| `Bool(bool)` | ✅ Lint + Runtime | ✅ for..in | Truthy/falsy evaluation handled; comparison limited to Eq/Ne |
| `String(String)` | ✅ Lint + Runtime | ✅ Index, for..in | String[Int] works; String[String] → TypeMode-aware None/error |
| `Array(Vec<Value>)` | ✅ Lint + Runtime | ✅ Out-of-bounds | Array[Int] works; out-of-bounds returns None |
| `Map(HashMap)` | ✅ Lint + Runtime | N/A | Map[String] returns None for missing keys (not a type error) |
| `Range { start, end, inclusive }` | ⚠️ Partial | ❌ | Iterates correctly but no TypeMode check on non-Int bounds |
| `Struct { name, fields }` | ✅ Lint + Runtime | ❌ Field access | Unknown field → hard error always. No TypeMode gate. |
| `EnumValue { enum_name, variant, values }` | ✅ Lint + Runtime | N/A | Match/equality works; variant type checking in lint |
| `Function / NativeFunction` | ⚠️ Partial | ❌ | Arity checked; param types only checked if annotated |
| `Return / Break / Continue` | N/A | N/A | Control flow, not user-facing types |

### Operation Categories — TypeMode Coverage

#### ✅ Fully TypeMode-Aware (implemented in PR #19)

- [x] **Index (`[]`) type mismatch** — `obj[key]` where obj isn't indexable by key type
- [x] **`for..in` on non-collection** — iterating Int, Bool, String, None, Float
- [x] **Template `{{expr}}` errors** — expression evaluation failures in templates
- [x] **Template `{{{raw_expr}}}` errors** — raw expression evaluation failures
- [x] **Template `{{expr|filter}}` errors** — filtered expression failures
- [x] **Template `{{#for}}` iterable errors** — for-loop iterable evaluation failures
- [x] **Template `{{#for}}` non-iterable value** — iterable evaluates to non-collection

#### ❌ NOT TypeMode-Aware — Hard Errors Always

These operations always throw `TypeError` or `RuntimeError` regardless of TypeMode. Some should remain hard errors; others should respect the dial.

**Should remain hard errors (intentional — programmer error, not data mismatch):**
- [x] Binary op type mismatch (`5 + [1,2]`) → `InvalidOperation` — correct, this is a code bug
- [x] Unary negate on non-numeric (`-"hello"`) → `TypeError` — correct, code bug
- [x] Division by zero → `DivisionByZero` — correct, arithmetic invariant
- [x] Struct unknown field (`struct.nonexistent`) → `RuntimeError` — correct, known schema
- [x] `push()` on non-array → `TypeError` — correct, code bug
- [x] `int("not_a_number")` → `TypeError` — correct, explicit conversion failure

**Should be reviewed — these involve external data boundaries:**
- [x] **Field access on non-struct** (`value.field` where value is Int/String/etc) → ✅ Now TypeMode-aware (commit 0ab1460). Strict errors, warn logs + returns None, forgiving returns None.
- [ ] **Struct field access with bad data** — `Struct["field"]` works (goes through Index), but `struct.field` on an unknown field is a hard error. When the struct comes from deserialized external data and might be missing fields, this doesn't respect TypeMode.
- [ ] **Method call on wrong type** — `value.method()` when value doesn't support that method. Currently attempts trait lookup then errors. Could degrade like field access in forgiving mode.

#### ⚠️ Gaps in the Type Checker (Static Analysis — Lint Axis)

**Currently checked by lint:**
- [x] Function parameter type mismatches (when annotated)
- [x] Return type mismatches (when annotated)
- [x] Struct field type mismatches (when struct is defined with types)
- [x] Enum variant argument count and types
- [x] Binary operation type compatibility
- [x] Match exhaustiveness (Option, Result, user enums)
- [x] `?` operator on non-Optional/Result
- [x] Double-wrapped Optional warning (`Some(Some(x))`)
- [x] Flow-sensitive type narrowing (`is_some` / `is_none` checks)

**Not checked by lint — needed for `--strict` mode:**
- [x] **Missing function parameter annotations** — ✅ warn/strict mode warns/errors (existing, now promoted to error in strict via `check_program_with_lint_mode`)
- [x] **Missing function return type** — ✅ warn/strict mode warns/errors (existing, now promoted to error in strict)
- [ ] **Missing variable type on let** — `let x = 42` could warn/error in strict (though type inference makes this less critical)
- [x] **Untyped lambda parameters** — ✅ Now warns in warn/strict mode (commit 0ab1460)
- [ ] **Generic collection element types** — `let arr = [1, "two", 3]` mixed array has no type annotation; lint doesn't flag the mixed types unless annotated
- [ ] **Map value type heterogeneity** — `map { "a": 1, "b": "two" }` mixed value types not flagged

#### ⚠️ Type Coercion Boundaries

Places where ntnt implicitly converts types. These are intentional design choices but should be documented and configurable in strict mode:

- [ ] **Int ↔ Float auto-promotion in arithmetic** — `3 + 2.5` → `5.5`. Implicit widening. TypeScript and Go reject this; Rust rejects this. Should strict mode require explicit `float(3) + 2.5`?
- [ ] **String concatenation with non-strings** — `"count: " + 42` → `"count: 42"`. Implicit `str()` on RHS. Python rejects this; JavaScript allows it. Should strict mode require explicit `"count: " + str(42)`?
- [ ] **Truthy/falsy evaluation** — `if value { ... }` where value is any type. `0`, `""`, `None`, `[]`, `false` are falsy. TypeScript strict requires explicit boolean check. Should ntnt strict require `if value != None { ... }` or `if len(value) > 0 { ... }`?
- [ ] **Int/Float comparison** — `3 == 3.0` → `true`. Cross-type comparison via promotion. Correct but potentially surprising.

#### ⚠️ Type System Features — Missing / Incomplete

Features expected in a comprehensive type system that ntnt doesn't have yet:

- [x] **Type aliases** — `type UserId = Int` and `type Handler = (Request) -> Response` implemented (PR #19, Phase 7.2).
- [ ] **Interface / trait types** — No way to say "any type that has a `.name` field and a `.validate()` method". Currently uses `Any` or named struct types only.
- [ ] **Literal types** — No `type Direction = "north" | "south" | "east" | "west"`. Union types exist at the type level but can't be refined to specific values.
- [x] **Nullable vs Optional distinction** — `T?` shorthand now works in annotations (PR #19, Phase 7.2). Database null → None mapping unchanged.
- [ ] **Type guards / user-defined narrowing** — `is_some()` narrows types via hard-coded flow analysis. No way for user functions to declare "this function narrows type X to Y" (like TypeScript's `x is T`).
- [x] **Generic functions** — Full unification in type checker: `fn identity<T>(x: T) -> T` infers return type, conflicting args detected (PR #19, Phase 7.4).
- [ ] **Recursive types** — No way to express `type Tree = { value: Int, children: [Tree] }`. Named types exist but can't self-reference in definitions.
- [ ] **Intersection types** — Union (`A | B`) exists but no intersection (`A & B`) for composing capabilities.
- [ ] **Type assertions / casts** — No `value as Int` or `value: Int` assertion syntax. Only runtime conversion functions (`int()`, `str()`, etc).
- [ ] **Const/readonly types** — No way to mark values as immutable at the type level. Values are mutable by default.
- [ ] **Tuple types** — `Tuple(Vec<Type>)` exists in the type system but no syntax for tuple literals or destructuring with typed positions.

---

## Implementation Status Checklist

### ✅ Completed (PR #19)

- [x] `TypeMode` enum (Strict / Warn / Forgiving)
- [x] `LintMode` enum (Default / Warn / Strict)
- [x] `NTNT_TYPE_MODE` env var reading with OnceLock caching
- [x] `NTNT_LINT_MODE` env var reading with OnceLock caching
- [x] Index type mismatch → TypeMode-aware
- [x] `for..in` non-collection → TypeMode-aware
- [x] Template expression errors (5 locations) → TypeMode-aware
- [x] 5 new runtime mode tests with mutex isolation
- [x] 8 existing DD-008 tests guarded against parallel race conditions
- [x] Version bump to 0.4.0
- [x] All 945 tests passing

### ✅ Phase 2: Lint Mode Integration (commit 0ab1460)

- [x] `--warn-untyped` CLI flag: warn on functions missing parameter type annotations
- [x] `--warn-untyped` CLI flag: warn on functions missing return type annotations
- [x] `--warn-untyped` CLI flag: warn on untyped lambda parameters
- [x] `NTNT_LINT_MODE=warn` reads env var when no CLI flag given
- [x] `--strict` enhanced: untyped functions are errors (not just warnings)
- [x] `NTNT_LINT_MODE=strict` reads env var when no CLI flag given
- [x] `check_program_with_lint_mode()` API accepting `LintMode` enum

### ✅ Phase 3: Extend TypeMode to Remaining Boundaries (partial, commit 0ab1460)

- [x] Field access on non-struct/map → TypeMode-aware (return None in forgiving)
- [ ] Method call on unsupported type → TypeMode-aware
- [ ] Struct unknown field access → TypeMode-aware (when struct from external data)

### ✅ Phase 4: Type Coercion Controls (Strict Mode Tightening) — PR #20

- [x] Strict mode rejects implicit Int→Float promotion in arithmetic
- [x] Strict mode rejects implicit non-String→String concatenation
- [x] Strict mode requires explicit boolean in `if` conditions (no truthy/falsy)
- [x] Strict mode gates `!`, `&&`, `||` on non-Bool operands
- [x] Document all implicit coercion points and rationale
- [x] 10 new tests + existing test race-condition fix (TYPE_MODE_MUTEX)

### 🔲 Phase 5: Type System Features

- [x] Type aliases (`type UserId = Int`)
- [x] `T?` shorthand for `Optional<T>` in annotations
- [ ] Type guards / user-defined narrowing functions
- [x] Full generic function support in type checker
- [ ] Interface / trait type definitions
- [ ] Literal types for union-of-values
- [ ] Recursive type support
- [ ] Intersection types
- [ ] Type assertion syntax (`value as Type`)
- [ ] Const/readonly type modifier
- [ ] Tuple literal syntax and typed destructuring

---

---

## Phase 6: Polish & Production Readiness (from top-to-bottom review)

Findings from a comprehensive code review of the type system as shipped in PR #19.

### ✅ 6.1: Documentation — Feature doesn't exist if it's not documented
- [x] Add `NTNT_TYPE_MODE` and `NTNT_LINT_MODE` to `docs/RUNTIME_REFERENCE.md`
- [x] Add `--warn-untyped` flag to `docs/RUNTIME_REFERENCE.md` lint section
- [x] Update `CLAUDE.md` with type safety modes guidance
- [x] Deprecation note for `NTNT_STRICT` in docs (use `NTNT_LINT_MODE=strict`)
- [x] Regenerate docs after changes

### ✅ 6.2: Consolidate template error handling — DRY violation
6 copy-pasted `match get_type_mode()` blocks (DONE — `handle_template_error()` helper) in `eval_template_parts`. Refactor to a single helper:
```rust
fn handle_template_error(&self, error: IntentError, result: &mut String) -> Result<()>
```
Returns `Err` in strict, appends comment/nothing in warn/forgiving.

### ✅ 6.3: Warning deduplication — warn mode is noisy
- [x] Add `HashSet<(String, usize)>` (file:line) to track already-warned locations
- [x] Same type mismatch at same location only warns once per request/evaluation
- [x] Prevents 50 identical warnings from a template for-loop over bad data

### ✅ 6.4: Annotate non-TypeMode error paths — 124 hard errors need justification
- [x] Add `// TypeMode: hard error — [reason]` comments to all 124 TypeError/RuntimeError paths not behind get_type_mode()
- [x] Categories: "code bug" (arity, syntax), "explicit conversion" (int/float parse), "arithmetic invariant" (div by zero)
- [x] Makes future contributors understand which errors are intentionally exempt

### ✅ 6.5: Deprecate NTNT_STRICT explicitly
- [x] Print `[DEPRECATED]` warning to stderr when `NTNT_STRICT` env var is detected
- [x] Message: "NTNT_STRICT is deprecated, use NTNT_LINT_MODE=strict"
- [x] Still works (backward compatible), just warns

### ✅ 6.6: Add --warn-untyped integration test
- [x] Test that `ntnt lint --warn-untyped` produces warnings (not errors)
- [x] Test that exit code is 0 (warnings are non-fatal)
- [x] Test that `NTNT_LINT_MODE=warn` produces same behavior

---

## Open Questions

1. **Should `warn` mode warnings include a request ID?** For web apps, correlating a type warning with the specific request that triggered it would make debugging much easier.

2. **Per-file runtime overrides?** Sorbet does per-file strictness. Should ntnt support `// @type_mode: strict` at the top of a file to override the global setting? Useful for marking auth modules as strict while leaving templates forgiving. Adds complexity — defer to a future version?

3. **Intent check integration:** Should `ntnt intent check` respect the runtime type mode? If running in strict mode, intent check could flag type mismatches as scenario failures. This would close the loop between static analysis and runtime behavior verification.

4. **Warning format / structured logging:** Should `[WARN]` output be structured (JSON) for production log aggregation? Would make it easier for monitoring tools (or AI agents watching logs) to parse and act on type warnings.

5. **Metrics / counters:** Should warn mode track a count of type mismatches per request? A request that triggers 50 type warnings is probably broken even if no single one is fatal. Could expose via a `/debug/type-warnings` endpoint in development.

---

## Phase 7: World-Class Type System (items 1-4 below; LSP deferred)

### ✅ 7.0: Copilot review round 3 — 10 comments
- [x] Fix `handle_template_error` doc comment (said "returns true", actually returns `Result<()>`)
- [x] Wire up `clear_type_warnings()` at `eval()` and `call_function_by_name()` boundaries
- [x] Make `strict_check_with_file()` respect `NTNT_LINT_MODE=strict` (not just `NTNT_STRICT`)
- [x] Make `--strict` and `--warn-untyped` mutually exclusive via `conflicts_with`
- [x] Fix `--warn-untyped` description to include all strict-mode warnings (not just annotations)
- [x] Fix variable shadowing: rename `code` → `source`/`exit_code` in 3 test functions
- [x] Add `DiagnosticKind` enum (`MissingParamAnnotation`, `MissingReturnAnnotation`, `MissingLambdaParamAnnotation`, `General`)
- [x] Replace brittle substring matching in `check_program_with_lint_mode` with `DiagnosticKind` matching

### ✅ 7.1: Error Message Quality (highest ROI)
**Goal:** Elm/Rust-tier error messages that suggest fixes, not just report mismatches.
- [x] Add source location tracking (file, line, column) to all RuntimeError/TypeError
- [x] Add "expected vs got" formatting: "Expected Array, found Int"
- [ ] Add origin tracking: "This value came from line 12 where you called `get_user()`"
- [x] Add suggestion engine: "Did you mean to handle the None case with `??`?"
- [x] Color-coded terminal output for error messages (red errors, yellow warnings, blue hints)
- [x] Add code snippets in error output showing the offending line with underline/caret
- [x] Test: every TypeMode error path should produce a message with at least expected/got types

### ✅ 7.2: T? Shorthand + Type Aliases (quick DX win)
**Goal:** Make annotations pleasant to write.
- [x] Parser already handles `T?` → `TypeExpr::Optional(T)` — verify
- [x] `type UserId = Int` — parser support exists, verify checker resolves aliases
- [x] `type Handler = (Request) -> Response` — function type aliases
- [x] `type JsonValue = String | Int | Float | Bool | [JsonValue] | Map<String, JsonValue>` — recursive type aliases (PR #20, v0.4.1)
- [x] Test: `fn get_user(id: UserId) -> User?` should work end-to-end
- [x] Update docs with examples

### ✅ 7.3: Deeper Type Inference
**Goal:** Reduce annotation burden — infer types through chains and function returns.
- [x] Infer function return types from body when unambiguous (single return path)
- [x] Infer collection element types: `let x = [1, 2, 3]` → `[Int]`
- [x] Infer map value types: `let m = map { "a": 1, "b": 2 }` → `Map<String, Int>`
- [x] Infer through method chains: `.filter(fn(x) { ... }).map(fn(x) { ... })`
- [x] Infer lambda parameter types from call context (e.g., `arr.map(fn(x) { x + 1 })` → x is element type)
- [x] Track: which `--warn-untyped` warnings become unnecessary after inference improvements
- [x] Test: code with obvious types should produce zero warnings in `--warn-untyped` mode

### ✅ 7.4: Full Generics in Type Checker
**Goal:** Type params provide real safety, not just `Any` pass-through.
- [x] Resolve generic type params during function call checking
- [x] Constraint solving: `fn identity<T>(x: T) -> T` — infer T from argument
- [x] Error on constraint violations: `identity<Int>("hello")` should error
- [x] Generic struct support: `struct Pair<A, B> { first: A, second: B }` — construction, field inference, type param substitution (PR #20, v0.4.1)
- [x] Generic function return type narrowing: `identity(42)` returns `Int`, not `Any`
- [ ] Bounded generics / constraints (future): `fn sum<T: Numeric>(items: [T]) -> T`
- [x] Test: generic function with wrong argument type should produce type error

### 🔲 7.5: LSP Server (deferred — months of work)
**Status:** Not in current scope. Tracked here for completeness.
- [ ] Basic LSP: diagnostics (push type errors to editor)
- [ ] Hover: show inferred types
- [ ] Autocomplete: field names, function params
- [ ] Go-to-definition
- [ ] Find references
- [ ] Rename symbol

---

## Phase 8: Error Reporting & Execution Context Fixes (PR #19, commits e5fe366 + 3d32eac)

### ✅ 8.1: Accurate Line Numbers on Runtime Errors
**Problem:** Runtime errors (TypeError, UndefinedVariable, ArityMismatch, etc.) showed no file or line info. Parser EOF errors reported "line 0".

**Root causes:**
1. `declaration()` in parser returned raw `Let`/`Function`/`Struct`/etc. without `Located` wrapper — only `statement()` (for if/while/for/return) was wrapped. Errors from declarations had no source position.
2. Error structs (`TypeError`, `RuntimeError`, `UndefinedVariable`, `UndefinedFunction`, `ArityMismatch`) had no `line` field. Error propagation had nowhere to attach location.
3. `current_line()` returned `unwrap_or(0)` at EOF instead of using the last token's position.

**Fixes:**
- [x] `parser.rs`: `current_line()`/`current_column()` fall back to `previous()` when `peek()` is None (EOF)
- [x] `parser.rs`: `consume()` uses `current_line()` instead of raw `peek()` for error positions
- [x] `parser.rs`: `declaration()` wraps ALL paths (let, fn, struct, enum, import, etc.) in `Located`
- [x] `error.rs`: Added `line: usize` field to `TypeError`, `RuntimeError`, `UndefinedVariable`, `UndefinedFunction`, `ArityMismatch`
- [x] `error.rs`: `at_line()` builder method for annotating errors at propagation boundaries
- [x] `error.rs`: `line()` method returns `Some(line)` for all error types when line > 0
- [x] `interpreter.rs`: `Statement::Located` handler uses `map_err(|e| e.at_line(*line))` to annotate errors
- [x] `main.rs`: `format_error` shows `file:line` header + source snippet for all runtime errors

### ✅ 8.2: Unified Execution Contexts (findings #69, #73)
**Problem:** `lib/` modules couldn't use stdlib imports (`std/string`, `std/env`, `std/http`). `concat()`, `get_env()`, `fetch()` all failed in lib context.

**Root cause:** `load_module_exports()`, `process_route_file()`, and `load_middleware_file()` all created fresh `Environment` and called `define_builtins()` + `define_builtin_types()` but NOT `define_stdlib()`. The stdlib module registry was empty, so `import { concat } from "std/string"` couldn't resolve.

**Fix:**
- [x] Added `self.define_stdlib()` to `load_module_exports()` (lib files)
- [x] Added `self.define_stdlib()` to `process_route_file()` (route files)
- [x] Added `self.define_stdlib()` to middleware loading path

**Note:** Hot-reload for lib files already works (`check_and_reload_lib_modules()` runs on every request). MEMORY.md note about needing `docker compose restart` was incorrect.
