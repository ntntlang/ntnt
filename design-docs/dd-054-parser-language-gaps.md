# DD-054: Parser Language Gaps — Verified Status and Implementation Plan

- **Status:** draft
- **Author:** larri
- **Created:** 2026-03-30
- **Revised:** 2026-04-05
- **Related findings:** ntnt-findings #54, #55, #56

---

## Executive Summary

This DD needed a reality check.

After re-verifying the three original findings against the current ntnt parser/runtime:

- **#54 — `return expr otherwise { ... }`** → still broken / real
- **#55 — module-level `let` with `map {}` literal** → **does not reproduce now**
- **#56 — semicolons as statement separators corrupt parser state** → **does not reproduce now**

So the original framing of DD-054 as “three active parser gaps” is no longer correct.

### Recommendation
Treat DD-054 as:
1. a **real implementation plan for #54**, and
2. a **verification + regression-lock plan for #55 and #56**.

That makes this a much tighter and more valuable next task.

---

## Verified Status Table

| Finding | Old DD status | Current status (2026-04-05) | Recommendation |
|---------|---------------|------------------------------|----------------|
| #54 `return expr otherwise { ... }` parse error | Active | **Confirmed still broken** | Fix now |
| #55 module-level `let` with `map {}` literal parse error | Active | **No longer reproduces** | Add regression test, mark resolved |
| #56 semicolons corrupt parser state / misreport errors | Active | **No longer reproduces** | Add regression test, mark resolved |

### Verification notes
Reproduced directly with `./target/release/ntnt check`:

#### #54 — still broken
```ntnt
fn get_users(pg) {
  return pg_query(pg, "SELECT * FROM users", []) otherwise { [] }
}
```

Current result:
- parser error at `otherwise`

#### #55 — does not reproduce now
```ntnt
let STATUS_LABELS = map { "active": "Active", "archived": "Archived" }
```

Current result:
- parses successfully

#### #56 — does not reproduce now
```ntnt
fn f(x) { let a = 1; return a }
```

Current result:
- parses successfully

---

## Critical Review of the Previous DD

The previous DD had two problems:

### 1. It treated all three findings as still-open parser bugs
That is no longer true. Two of them appear to have been fixed implicitly by later parser work.

### 2. It proposed the wrong implementation shape for #54
The old draft assumed `return expr otherwise { ... }` should mirror `let x = expr otherwise { ... }` exactly.

That is **not** the best design.

Why:
- `let ... otherwise { ... }` today is a **diverging** recovery path
- the natural meaning of
  ```ntnt
  return expr otherwise { [] }
  ```
  is **fallback value production**, not “run a diverging block”

If we copied `let-otherwise` semantics directly, the natural syntax people want would still feel wrong. We’d force things like:

```ntnt
return pg_query(...) otherwise { return [] }
```

which defeats most of the ergonomic value.

So the right fix for #54 is **not** “attach `otherwise` to `Statement::Return` and make it behave exactly like `let-otherwise`.”

The right fix is:
- support `return <expr> otherwise { <fallback-value-expr> }`
- interpret the `otherwise` block as a **fallback value block** for the return statement

That is a cleaner user-facing feature and matches the way developers naturally read the syntax.

---

## Issue #54 — `return expr otherwise { ... }` is a real parser gap

### Description
This natural pattern currently fails to parse:

```ntnt
fn get_users(pg) {
  return pg_query(pg, "SELECT * FROM users", []) otherwise { [] }
}
```

The parser consumes `pg_query(...)` as the return expression and then treats `otherwise` as unexpected.

### Root Cause
`statement_inner()` handles `return` by parsing an optional expression and immediately finalizing the statement:

```rust
if self.match_token(&[TokenKind::Return]) {
    ...
    let value = ... Some(self.expression()?) ...
    self.match_token(&[TokenKind::Semicolon]);
    Ok(Statement::Return(value))
}
```

Unlike `let_declaration()`, there is no post-expression `otherwise` hook.

### Recommended semantics
For **return statements only**, `otherwise` should mean:

> if the return expression evaluates to `Err`, `None`, or a caught runtime error, evaluate the `otherwise` block as a fallback value and return that value instead.

Example:

```ntnt
fn get_users(pg) {
  return pg_query(pg, "SELECT * FROM users", []) otherwise { [] }
}
```

Meaning:
- success → return query result
- `Err` / `None` / caught runtime error → evaluate `{ [] }`, then return `[]`

### Why this is better than copying `let-otherwise`
Because users want a fallback-return value, not a diverging control-flow block.

This gives a real ergonomic win instead of just moving the awkwardness around.

---

## Language Design Decision for #54

### Chosen direction
Implement `return ... otherwise { ... }` with **fallback-value semantics**.

### Explicit scope
Support:
- `return <expr> otherwise { <block> }`

Where the block is evaluated as a value-producing block.

### Non-goals for the initial implementation
Do **not** try to add all of these at once:
- arbitrary statement-form `otherwise return ...`
- `break ... otherwise`
- `continue ... otherwise`
- generalized postfix `otherwise` on every expression form

This DD is only about fixing the natural return fallback form cleanly.

---

## Proposed AST / Parser Shape

### AST recommendation
Change `Statement::Return` from:

```rust
Return(Option<Expression>)
```

to a struct-style variant:

```rust
Return {
    value: Option<Expression>,
    otherwise: Option<Block>,
}
```

Why:
- clearer than adding another tuple field later
- easier to extend/test
- keeps the design explicit instead of fragile positional matching

### Parser recommendation
In `statement_inner()`:
1. parse the optional return expression exactly as today
2. if an expression was present, look for `otherwise`
3. if found, require a `{ ... }` block in the initial implementation
4. attach that block to the return statement

I recommend **block-only** syntax for the initial version.

Why:
- simpler parser surface
- avoids muddying “single statement” semantics with a value-producing fallback
- matches the natural form people already want: `{ [] }`

---

## Interpreter Semantics for #54

### Recommended behavior
When evaluating:

```ntnt
return EXPR otherwise { BLOCK }
```

Use this decision table:

| EXPR result | Behavior |
|-------------|----------|
| normal value | return it immediately |
| `Result::Err` | bind `err` if appropriate, evaluate `BLOCK` as fallback value, return that |
| `Option::None` | evaluate `BLOCK` as fallback value, return that |
| caught runtime error | bind `err`, evaluate `BLOCK` as fallback value, return that |

### Important distinction from `let-otherwise`
For `return-otherwise`, the fallback block should **produce the returned value**.
It should **not** be required to diverge.

So this should work:

```ntnt
return risky() otherwise { [] }
```

and should mean exactly what it looks like.

### `err` binding
For parity with existing `otherwise` ergonomics, runtime errors and `Result::Err` should bind `err` inside the fallback block.

Example:

```ntnt
return risky() otherwise { "failed: #{err}" }
```

---

## Issue #55 — module-level `map {}` literals

### Current status
This no longer reproduces on current ntnt.

### Interpretation
Either:
- the original bug was fixed implicitly by later parser work, or
- it was more context-specific than the old DD captured

Either way, it is not justified to carry this forward as an active parser gap without a current reproducer.

### Recommendation
Do **not** spend implementation time fixing #55 right now.
Instead:
- add a regression test proving top-level `map {}` literals parse successfully
- mark the finding as resolved (or “not reproducible on current head”)

---

## Issue #56 — semicolons corrupt parser state

### Current status
This no longer reproduces on current ntnt for the documented example.

### Interpretation
The parser already accepts semicolons widely enough that the old “silent corruption” report no longer appears to describe current behavior.

### Recommendation
Do **not** spend implementation time on semicolon tolerance or diagnostics in this DD.
Instead:
- add a regression test proving the documented semicolon case parses successfully
- mark the finding as resolved (or “not reproducible on current head”)

If later we find a narrower semicolon bug still exists, that should become its own focused finding/DD with a fresh reproducer.

---

## How hard is this?

### Overall DD-054 scope as revised
Because #55 and #56 are no longer active implementation items, the real scope is much smaller than the old DD implied.

### Difficulty estimate
- **#54 parser/AST/interpreter fix:** **medium**
- **#55 / #56 regression locking:** **small**
- **overall DD after revision:** **medium**

This is now a very reasonable next parser/runtime cleanup task.

It is **not** a giant parser overhaul.

---

## Implementation Checklist

### Phase 0 — Re-verify and lock scope
- [ ] Add a regression test for #55 showing module-level `map {}` literals parse successfully
- [ ] Add a regression test for #56 showing semicolons in the documented case parse successfully
- [ ] Update the DD / findings tracking to mark #55 and #56 as resolved or non-repro on current head
- [ ] Keep DD-054 scoped to #54 implementation + #55/#56 regression coverage

### Phase 1 — AST and parser for #54
- [ ] Change `Statement::Return` from tuple form to struct form:
  - [ ] `value: Option<Expression>`
  - [ ] `otherwise: Option<Block>`
- [ ] Update all parser/interpreter/tests affected by the `Statement::Return` shape change
- [ ] In `statement_inner()`, after parsing `return <expr>`, accept optional `otherwise { ... }`
- [ ] Require block form for the initial implementation (`otherwise { ... }` only)
- [ ] Reject malformed `return otherwise ...` / missing expression / missing block with targeted parser diagnostics

### Phase 2 — Interpreter semantics for #54
- [ ] Implement fallback-value semantics for `return ... otherwise { ... }`
- [ ] On success: return the original value
- [ ] On `Result::Err`: evaluate fallback block and return its value
- [ ] On `Option::None`: evaluate fallback block and return its value
- [ ] On caught runtime error: bind `err`, evaluate fallback block, and return its value
- [ ] Ensure fallback block is value-producing, not divergence-required

### Phase 3 — Test matrix
- [ ] Parser test: `return expr otherwise { ... }` parses successfully
- [ ] Interpreter test: success path does not execute fallback
- [ ] Interpreter test: `Result::Err` executes fallback and returns fallback value
- [ ] Interpreter test: `Option::None` executes fallback and returns fallback value
- [ ] Interpreter test: runtime error executes fallback and exposes `err`
- [ ] Interpreter test: fallback block can return array/map/string/etc. as the final returned value
- [ ] Parser regression test: top-level `map {}` module constant still parses
- [ ] Parser regression test: documented semicolon sample still parses

### Phase 4 — Docs and cleanup
- [ ] Update ntnt skill / workaround docs to remove the split-form workaround for #54 once fixed
- [ ] Update `ntnt-findings/README.md` for #54, #55, and #56 status
- [ ] Regenerate docs: `cargo build --release --locked && ./target/release/ntnt docs --generate`
- [ ] Run the standard validation loop
- [ ] Push PR and complete Greptile self-review loop

---

## Risks

| Risk | Why it matters | Mitigation |
|------|----------------|------------|
| Reusing `otherwise` with the wrong semantics | Could make the feature still feel awkward | Explicitly choose fallback-value semantics for `return-otherwise` |
| AST churn | `Statement::Return` shape change touches many matches | Use struct-style variant and update exhaustively |
| Error-handling inconsistency | `return-otherwise` and `let-otherwise` will not be identical | Document the semantic difference clearly; it is intentional |
| Hidden #55/#56 edge cases remain | Old report may have had a narrower reproducer | Add regression tests for the confirmed passing examples and keep future reports scoped with fresh repros |

---

## Open Questions

| Question | Recommendation |
|----------|----------------|
| Should `return-otherwise` require divergence like `let-otherwise`? | No — that defeats the ergonomic goal |
| Should initial syntax support single-statement fallback form? | No — block-only first |
| Should #55 still be treated as open? | No, not without a current reproducer |
| Should #56 still be treated as open? | No, not without a current reproducer |

---

## Bottom-Line Recommendation

If you want to work on DD-054 next, the best version of the project is:

- **fix #54 properly** with fallback-value semantics
- **do not waste time “fixing” #55 and #56 again**
- **lock #55 and #56 in with regression tests** so we stop rediscovering ghosts

That turns DD-054 from a fuzzy three-bug cleanup into a tight, worthwhile parser/runtime improvement.

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-30 | Initial draft — treated #54, #55, and #56 as active parser gaps |
| 2026-04-05 | Major revision — re-verified all three findings against current ntnt, narrowed active implementation scope to #54, reclassified #55/#56 as regression-lock items, and rewrote the plan around fallback-value semantics for `return-otherwise` |
