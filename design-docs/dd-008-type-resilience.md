# DD-008: ntnt Type Resilience — Graceful Handling of Data Shape Mismatches

**Status:** approved  
**Author:** Larri  
**Date:** 2026-03-07  
**PR Target:** `fixes/batch-2026-03` (PR #18)  
**Triggered by:** 500 error on `/admin/startups/deskagent-ai-001` — DeskAgent's `score_breakdown.justifications` stored as a flat string instead of a JSON map

---

## The Problem

A startup detail page crashed with `Type error: Invalid index operation` because one field in the database was a string where the code expected a map.

**Root cause chain:**

```
DB: score_breakdown.justifications = "D1(3): 40% non-billable..."  (string)
                                     ↕ expected: {"d1_problem": "...", ...}  (map)

Handler: for sdk in score_dims {
           let dv = score_dims[sdk] ?? ""     ← dv is now a string
         }

Later:   for mk in (s["mvp"] ?? map {}) {    ← s["mvp"] is a non-null string
           mvp_entries = ... (s["mvp"] ?? map {})[mk]   ← string["W"] → CRASH
         }
```

The `??` operator only catches `None`. When the value exists but has the wrong type, `??` can't help and `[]` throws a fatal error. The entire page 500s.

**This is a class of bug, not a one-off.** Any ntnt app that consumes JSON from a database, API, or user input is vulnerable. Data shape mismatches are the #1 real-world bug category in web apps.

---

## Proposed Changes

Three changes, ordered by priority. Each is independently valuable.

### Change 1: `[]` Returns None on Type Mismatch (Critical)

**Current behavior:**

| Expression | Result |
|-----------|--------|
| `map["key"]` where key exists | Value ✅ |
| `map["key"]` where key missing | None ✅ |
| `array[0]` where index valid | Value ✅ |
| `array[99]` where out of bounds | IndexOutOfBounds ❌ crash |
| `string["key"]` | TypeError ❌ crash |
| `int["key"]` | TypeError ❌ crash |
| `None["key"]` | TypeError ❌ crash |

**Proposed behavior:**

| Expression | Result |
|-----------|--------|
| `map["key"]` where key exists | Value ✅ (unchanged) |
| `map["key"]` where key missing | None ✅ (unchanged) |
| `array[0]` where index valid | Value ✅ (unchanged) |
| `array[99]` where out of bounds | None ✅ (changed — was crash) |
| `string["key"]` | None ✅ (changed — was crash) |
| `int["key"]` | None ✅ (changed — was crash) |
| `None["key"]` | None ✅ (changed — was crash) |

**The principle:** `[]` is an access operation, not an assertion. If the thing you're accessing doesn't exist or doesn't make sense, the answer is "nothing" (None), not "crash." This makes `??` the universal safety net:

```ntnt
let name = data["name"] ?? "Unknown"   // Works if data is a map, string, int, None, anything
```

**Implementation** (`src/interpreter.rs`, `Expression::Index` match):

```rust
// Replace the catch-all:
_ => Err(IntentError::TypeError("Invalid index operation".to_string()))

// With:
_ => Ok(Value::EnumValue {
    enum_name: "Option".to_string(),
    variant: "None".to_string(),
    values: vec![],
})
```

Also change `IndexOutOfBounds` for arrays to return None:

```rust
(Value::Array(arr), Value::Int(i)) => {
    let index = if i < 0 { (arr.len() as i64 + i) as usize } else { i as usize };
    Ok(arr.get(index).cloned().unwrap_or_else(|| Value::none()))
}
```

**Tests to add:**
- `string["key"] == None` → true
- `42["key"] == None` → true  
- `None["key"] == None` → true
- `[1,2,3][99] == None` → true (was IndexOutOfBounds)
- `[1,2,3][-99] == None` → true
- `map["key"] ?? "default"` still works (regression check)
- `string["key"] ?? "fallback"` → "fallback"

**Breaking change risk:** Low. Code that previously crashed now returns None instead. Any code that relied on catching IndexOutOfBounds or TypeError from `[]` would change behavior, but that's almost certainly zero real-world code — you don't try-catch index operations in ntnt.

---

### Change 2: `for..in` Skips Non-Collections (Important)

**Current behavior:**

| `for k in value` | Behavior |
|-----------------|----------|
| Array | Iterate elements ✅ |
| Map | Iterate keys ✅ |
| Range | Iterate range ✅ |
| String | Iterate characters ⚠️ (footgun) |
| Int/Bool/None | RuntimeError crash ❌ |

**Proposed behavior:**

| `for k in value` | Behavior |
|-----------------|----------|
| Array | Iterate elements ✅ (unchanged) |
| Map | Iterate keys ✅ (unchanged) |
| Range | Iterate range ✅ (unchanged) |
| String | Zero iterations + dev warning ⚠️ (changed) |
| Int/Bool/None/Function | Zero iterations + dev warning ⚠️ (changed) |

**String character iteration via explicit `chars()`:**

```ntnt
// Old way (being removed from for..in):
for ch in "hello" { print(ch) }

// New way (explicit intent):
for ch in chars("hello") { print(ch) }
```

**Implementation** (`src/interpreter.rs`, `Statement::ForIn`):

```rust
let items: Vec<Value> = match &iterable_value {
    Value::Array(arr) => arr.clone(),
    Value::Range { start, end, inclusive } => { /* unchanged */ },
    Value::Map(map) => map.keys().map(|k| Value::String(k.clone())).collect(),
    // String no longer auto-iterates. Use chars() builtin.
    _ => {
        // In dev mode, emit a warning
        if std::env::var("NTNT_ENV").unwrap_or_default() != "production" {
            eprintln!("[WARN] for..in on {} — skipping (not a collection). \
                       Use chars() for string iteration.",
                      iterable_value.type_name());
        }
        vec![]  // Zero iterations, no crash
    }
};
```

**New builtin: `chars(s)`**

```rust
// @ntnt chars
// @signature chars(s: String) -> Array<String>
// Split a string into an array of single-character strings.
// @param s Input string
// @returns Array of single-character strings
// @example chars("hello") => ["h", "e", "l", "l", "o"]
// @see_also split, len
```

**Tests to add:**
- `for k in "hello" { }` → zero iterations (was 5)
- `for k in 42 { }` → zero iterations (was crash)
- `for k in None { }` → zero iterations (was crash)
- `for ch in chars("hi") { }` → 2 iterations
- `for k in map { "a": 1 } { }` → 1 iteration (regression check)
- `for x in [1,2,3] { }` → 3 iterations (regression check)

**Breaking change risk:** Medium. Code that intentionally iterates string characters via `for ch in str` will silently get zero iterations. Mitigation: the dev-mode warning makes this visible, and `chars()` is the explicit replacement. Grep the codebase for `for.*in.*string` patterns before release.

---

### Change 3: Template Error Boundaries (Recommended)

**Current behavior:** Any expression error inside `{{expr}}` propagates up and crashes the entire page with a 500.

**Proposed behavior:**

| Mode | `{{expr}}` throws | Result |
|------|-------------------|--------|
| Development | Type/runtime error | Renders `<!-- ⚠️ TEMPLATE ERROR: {message} at {location} -->` inline + stderr log |
| Production | Type/runtime error | Renders empty string `""` + structured `[ERROR]` log |

**Implementation** (`src/stdlib/template.rs` or wherever template expressions are evaluated):

Wrap each expression evaluation in a catch:

```rust
match self.eval_expression(expr) {
    Ok(val) => render_value(val),
    Err(e) => {
        eprintln!("[ERROR] Template expression failed: {} at {}", e, location);
        if is_dev_mode() {
            format!("<!-- ⚠️ TEMPLATE ERROR: {} -->", e)
        } else {
            String::new()
        }
    }
}
```

**This applies to:**
- `{{expr}}` — variable interpolation
- `{{#if expr}}` — condition evaluation (treat as false on error)
- `{{#for x in expr}}` — iterable evaluation (treat as empty on error)

**Tests to add:**
- Template with `{{bad_expr}}` renders without crashing
- Dev mode shows HTML comment with error
- Prod mode renders empty string
- `{{#if bad_expr}}` treats as false
- `{{#for x in bad_expr}}` iterates zero times
- Valid expressions still render correctly (regression)

**Breaking change risk:** Low-positive. Pages that previously 500'd now render with missing data. This is strictly better UX and doesn't affect correctly-typed data.

---

## Implementation Plan

### Phase 1: Add to PR #18 (`fixes/batch-2026-03`)

All three changes go into the existing batch PR. ~120 lines of Rust + ~100 lines of tests.

**Change 1 — `[]` returns None on type mismatch**
- [x] Modify `Expression::Index` catch-all in `src/interpreter.rs` to return None instead of TypeError
- [x] Change `IndexOutOfBounds` for arrays to return None instead of crashing
- [x] Change `IndexOutOfBounds` for string char access to return None
- [x] Add test: `string["key"] == None` → true
- [x] Add test: `42["key"] == None` → true
- [x] Add test: `None["key"] == None` → true
- [x] Add test: `[1,2,3][99] == None` → true
- [x] Add test: `[1,2,3][-99] == None` → true
- [x] Add test: `string["key"] ?? "fallback"` → "fallback"
- [x] Regression test: `map["existing"] ?? "default"` still returns value

**Change 2 — `for..in` skips non-collections + `chars()` builtin**
- [x] Modify `Statement::ForIn` in `src/interpreter.rs` — String and catch-all return `vec![]`
- [x] Add dev-mode warning to stderr when `for..in` skips
- [x] Implement `chars()` builtin in `src/interpreter.rs`
- [x] Add `chars` signature in `src/typechecker.rs`
- [x] Add `// @ntnt chars` doc annotation
- [x] Add test: `for k in "hello" { }` → zero iterations
- [x] Add test: `for k in 42 { }` → zero iterations
- [x] Add test: `for k in None { }` → zero iterations
- [x] Add test: `chars("hi")` → `["h", "i"]`
- [x] Add test: `for ch in chars("abc") { }` → 3 iterations
- [x] Regression test: `for k in map { "a": 1 } { }` → 1 iteration
- [x] Regression test: `for x in [1,2,3] { }` → 3 iterations

**Change 3 — Template error boundaries**
- [x] Wrap template `{{expr}}` evaluation in error catch
- [x] Dev mode: render `<!-- ⚠️ TEMPLATE ERROR: {msg} -->` inline
- [x] Prod mode: render empty string `""`
- [x] Log `[ERROR] Template expression failed: ...` to stderr in both modes
- [x] Handle `{{#if expr}}` — treat errored condition as false
- [x] Handle `{{#for x in expr}}` — treat errored iterable as empty
- [x] Add test: template with bad expression renders without crash
- [x] Add test: dev mode includes error comment in output
- [x] Add test: prod mode renders empty for bad expression
- [x] Regression test: valid expressions still render correctly

**Docs**
- [x] Update `@ntnt` annotations for changed builtins
- [x] Run `cargo build --release --locked`
- [x] Run `ntnt docs --generate`
- [x] Update `AI_AGENT_GUIDE.md` with new `[]`, `for..in`, and template behavior
- [x] Run `cargo fmt --all`

### Phase 2: Build and Test on Staging

- [x] `cargo build --release --locked` (full release binary)
- [x] All tests pass: `cargo test --locked` (919 passed, 0 failed)
- [x] Copy binary to staging via `docker cp`
- [x] Restart staging container
- [x] Verify DeskAgent page loads without 500 ✅
- [x] Check logs for dev-mode warnings (Change 2) — `[WARN] for..in on String` confirmed
- [x] No template error crashes in logs ✅
- [x] All 16 startup detail pages return 200 ✅ (zero failures)
- [x] Startup list page returns 200 (71KB) ✅
- [ ] Verify Running Wild pages unaffected (if using same binary)

### Phase 3: Release (when ready)

- [x] Push to `fixes/batch-2026-03` (commit `f0b0a21`)
- [x] Version bumped to v0.3.17
- [ ] Tag `v0.3.17` (after PR is merged by Josh)
- [ ] Deploy new binary to prod dashboard
- [ ] Deploy new binary to Running Wild
- [ ] Deploy new binary to any other ntnt apps
- [ ] Verify prod DeskAgent page loads

### Phase 4: Data Cleanup (post-deploy)

- [x] Fix DeskAgent `justifications`: string → map ✅
- [x] Fix DeskAgent `mvp`: string → map ✅
- [x] Fix DeskAgent `unit_economics`: string → map ✅
- [x] Add DeskAgent `kill_screen` map ✅
- [x] Remove orphaned k-prefixed string keys ✅
- [ ] Verify DeskAgent page renders all sections correctly with fixed data

---

## Design Principles Alignment

| ntnt Principle | How This Aligns |
|---------------|-----------------|
| **Simple & Intuitive** | `value["key"] ?? "default"` just works, regardless of value's type. No `is_map()` guards needed. |
| **Strong & Robust** | Errors in data don't crash pages. Structured logging captures every issue for debugging. |
| **Consistent** | `[]` returns None uniformly for "nothing here" — missing key, wrong type, out of bounds. One pattern. |
| **Secure by Default** | Template error boundaries prevent information leakage in production (no stack traces in HTML). |
| **Progressive Types** | Untyped code becomes safer. Typed code can still use strict mode for compile-time catches. |

---

## Appendix: The DeskAgent Data Fix

Independent of language changes, the DeskAgent data should be normalized:

```sql
-- Fix justifications: string → map (parse the "D1(3): ... D2(2): ..." format)
-- Fix mvp: string → map (parse into structured fields)  
-- Fix unit_economics: string → map
-- Add kill_screen map (currently missing)
```

This is a data migration, not a code change. But with Change 1 in place, the page would render (with missing sections) even without fixing the data — which is the correct behavior for a web app.
