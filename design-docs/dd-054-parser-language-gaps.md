# DD-054: Parser Language Gaps — Verified Status and Implemented Plan

- **Status:** implemented on branch / awaiting PR review
- **Author:** larri
- **Created:** 2026-03-30
- **Revised:** 2026-04-05
- **Related findings:** ntnt-findings #54, #55, #56 (documented in SKILL.md)

---

## Executive Summary

This DD originally treated three parser/language findings as equally active. After re-verifying them against current ntnt on this branch, that was no longer true.

Verified status:
- **#54 — `return expr otherwise { ... }`** → real gap, now implemented on this branch
- **#55 — module-level `let` with `map {}` literal** → does not reproduce now; locked with regression coverage
- **#56 — semicolon separator corruption/misreporting** → does not reproduce now; locked with regression coverage

So DD-054 became:
1. a real implementation plan for #54, and
2. a regression-lock plan for #55 and #56.

That revised scope has now been implemented on this branch.

---

## Verified Status Table

| Finding | Old DD status | Current status on this branch | Recommendation |
|---------|---------------|-------------------------------|----------------|
| #54 `return expr otherwise { ... }` parse error | Active | **Implemented** | Ship |
| #55 module-level `let` with `map {}` literal parse error | Active | **Does not reproduce** | Regression-lock only |
| #56 semicolons corrupt parser state / misreport errors | Active | **Does not reproduce** | Regression-lock only |

---

## Final Design Decision for #54

### Chosen semantics
Implement:

```ntnt
return EXPR otherwise { BLOCK }
```

with **fallback-value semantics**.

Meaning:
- success → return the original successful value
- `Result::Err` → evaluate fallback block and return its value
- `Option::None` → evaluate fallback block and return its value
- caught runtime error → bind `err`, evaluate fallback block, and return its value

### Why this design
This is better than copying `let-otherwise` divergence semantics onto `return`, because users want a natural value fallback:

```ntnt
return pg_query(...) otherwise { [] }
```

not an awkward diverging block like:

```ntnt
return pg_query(...) otherwise { return [] }
```

---

## Implemented Shape

### AST
`Statement::Return` now uses a struct-style variant:

```rust
Return {
    value: Option<Expression>,
    otherwise: Option<Block>,
}
```

### Parser
`statement_inner()` now:
1. parses optional `return <expr>` as before
2. accepts optional `otherwise { ... }` after a return expression
3. requires block form for the initial implementation
4. rejects malformed `return otherwise ...` with a targeted parser error

### Interpreter
`return ... otherwise { ... }` now:
- unwraps `Result::Ok` / `Option::Some`
- evaluates fallback for `Result::Err` / `Option::None`
- catches runtime errors from the return expression, binds `err`, and evaluates fallback
- returns the fallback block value directly

### Typechecker
The typechecker now:
- understands the new `Statement::Return` shape
- unwraps `Optional<T>` / `Result<T, E>` for `return-otherwise`
- unions the success type with the fallback-block type
- binds `err` as `Any` inside the fallback block for checking purposes

---

## Implementation Checklist

### Phase 0 — Re-verify and lock scope
- [x] Add a regression test for #55 showing module-level `map {}` literals parse successfully
- [x] Add a regression test for #56 showing semicolons in the documented case parse successfully
- [x] Update the DD to mark #55 and #56 as resolved/non-repro on current head
- [x] Keep DD-054 scoped to #54 implementation + #55/#56 regression coverage

### Phase 1 — AST and parser for #54
- [x] Change `Statement::Return` from tuple form to struct form:
  - [x] `value: Option<Expression>`
  - [x] `otherwise: Option<Block>`
- [x] Update all parser/interpreter/tests affected by the `Statement::Return` shape change
- [x] In `statement_inner()`, after parsing `return <expr>`, accept optional `otherwise { ... }`
- [x] Require block form for the initial implementation (`otherwise { ... }` only)
- [x] Reject malformed `return otherwise ...` / missing expression / missing block with targeted parser diagnostics

### Phase 2 — Interpreter semantics for #54
- [x] Implement fallback-value semantics for `return ... otherwise { ... }`
- [x] On success: return the original value
- [x] On `Result::Err`: evaluate fallback block and return its value
- [x] On `Option::None`: evaluate fallback block and return its value
- [x] On caught runtime error: bind `err`, evaluate fallback block, and return that
- [x] Ensure fallback block is value-producing, not divergence-required

### Phase 3 — Test matrix
- [x] Parser/runtime test: `return expr otherwise { ... }` parses successfully
- [x] Interpreter test: success path does not execute fallback
- [x] Interpreter test: `Result::Err` executes fallback and returns fallback value
- [x] Interpreter test: `Option::None` executes fallback and returns fallback value
- [x] Interpreter test: runtime error executes fallback and exposes `err`
- [x] Interpreter test: fallback block can return array/map/string/etc. as the final returned value
- [x] Parser regression test: top-level `map {}` module constant still parses
- [x] Parser regression test: documented semicolon sample still parses

### Phase 4 — Docs and cleanup
- [x] Update DD-054 and implementation notes so the split-form workaround is no longer the recommended path for #54
- [x] Update DD-054 itself as the active status tracker for #54, #55, and #56 (no standalone `ntnt-findings/README.md` exists in the current repo)
- [x] Regenerate docs: `cargo build --release --locked && ./target/release/ntnt docs --generate`
- [x] Run the standard validation loop
- [ ] Push PR and complete Greptile self-review loop

---

## Validation Note

Implemented and validated on branch `feat/dd-054-return-otherwise`.

Validated behavior:
- `return Ok(42) otherwise { 0 }` returns `42`
- `return Err("fail") otherwise { "fallback" }` returns fallback value and exposes `err`
- `return None otherwise { "empty" }` returns fallback value
- `return 1 / 0 otherwise { 7 }` catches the runtime error, binds `err`, and returns `7`
- top-level `map { ... }` module constants still parse
- documented semicolon statement-separator case still parses

Validation loop completed:
- `cargo fmt`
- `cargo build --profile dev-release`
- `cargo test --lib`
- `cargo test --test language_features_tests --test type_checker_tests --test cli_tests`
- `cargo build --release --locked`
- `./target/release/ntnt docs --generate`
- `cargo fmt -- --check`

---

## Risks

| Risk | Why it matters | Mitigation |
|------|----------------|------------|
| Reusing `otherwise` with the wrong semantics | Could make the feature still feel awkward | Explicitly choose fallback-value semantics for `return-otherwise` |
| AST churn | `Statement::Return` shape change touches many matches | Use struct-style variant and update exhaustively |
| Error-handling inconsistency | `return-otherwise` and `let-otherwise` are intentionally different | Document the difference clearly |
| Hidden #55/#56 edge cases remain | Old report may have had a narrower reproducer | Keep future reports scoped to fresh repros |

---

## Open Questions

| Question | Recommendation |
|----------|----------------|
| Should `return-otherwise` require divergence like `let-otherwise`? | No |
| Should initial syntax support single-statement fallback form? | No — block-only first |
| Should #55 still be treated as open? | No, not without a current reproducer |
| Should #56 still be treated as open? | No, not without a current reproducer |

---

## Bottom-Line Recommendation

This is now a good next parser/runtime improvement to ship:
- #54 is real and now implemented with the right semantics
- #55 and #56 are not active implementation work and should stay locked down with regression tests

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-30 | Initial draft — treated #54, #55, and #56 as active parser gaps |
| 2026-04-05 | Major revision and implementation — re-verified all three findings against current ntnt, narrowed active scope to #54, reclassified #55/#56 as regression-lock items, implemented `return-otherwise` with fallback-value semantics, and completed the validation loop |
