# DD-054: Parser Language Gaps — ntnt-findings #54, #55, #56

- **Status:** draft
- **Author:** larri
- **Created:** 2026-03-30
- **Related findings:** ntnt-findings #54, #55, #56 (documented in SKILL.md)

---

## Summary

Three parser-level issues surfaced during real-world ntnt development, filed as ntnt-findings in February 2026 and documented with workarounds in SKILL.md. All three are confirmed still present as of v0.4.5. They are not blockers (workarounds exist) but they are user-hostile — especially #54 and #56, which bite any developer coming from JS/Rust/Go. This DD formalizes the decisions and fix plan.

---

## Issue #54 — `return expr otherwise { ... }` is a parse error

### Description

The natural pattern for returning a fallback value from a function that calls a failable operation:

```ntnt
fn get_users(pg) {
  return pg_query(pg, "SELECT * FROM users", []) otherwise { [] }
}
```

…is a parse error. The parser consumes `pg_query(...)` as the return value, then hits `otherwise` as an unexpected token.

### Root Cause

`statement_inner()` handles `return` by calling `self.expression()`. The `otherwise` clause is only wired into `let_declaration()`. The grammar doesn't allow `return <expr> otherwise { ... }`.

Workaround: split into two statements:
```ntnt
let r = pg_query(pg, "SELECT * FROM users", []) otherwise { return [] }
return r
```

This was used across 11 lib functions before the bug was identified.

### Fix

In `statement_inner()`, after parsing the return expression, check for `otherwise` and attach it — the same pattern `let_declaration()` already uses at line 229 of `parser.rs`:

```rust
// After: Some(self.expression()?)
let otherwise = if self.match_token(&[TokenKind::Otherwise]) {
    if self.match_token(&[TokenKind::LeftBrace]) {
        let block = self.block()?;
        Some(block)
    } else {
        let stmt = self.statement()?;
        Some(Block { statements: vec![stmt] })
    }
} else {
    None
};
```

Then `Statement::Return` needs an `otherwise` field, and the interpreter needs to handle it (evaluate the otherwise block if the return value is an error/None).

### Effort: Medium
- Parser change: small and well-defined
- AST change: `Statement::Return` gains an `otherwise: Option<Block>` field
- Interpreter change: evaluate otherwise block on error/None return value
- Tests: straightforward

### Decision needed
- [ ] Fix it (recommended — this is the natural pattern and the current behavior is surprising)
- [ ] Leave as-is and document split-form as idiomatic

---

## Issue #55 — Module-level `let` doesn't support `map {}` literals

### Description

In a lib file, array constants work at the top level:
```ntnt
let ALLOWED_METHODS = ["GET", "POST", "PUT", "DELETE"]
```

But map constants don't:
```ntnt
let STATUS_LABELS = map { "active": "Active", "archived": "Archived" }  // parse error
```

Forces map constants into function bodies, which either means re-creating them on every call or threading them as parameters.

### Root Cause

`expression()` → `primary()` handles `map { ... }` via `TokenKind::Map`, but the `is_nested_map_literal()` heuristic at line 2041 of `parser.rs` may reject top-level map literals due to parser context state. Needs a targeted test to confirm the exact failure point.

### Fix

Two options:

**Option A (targeted):** Identify and fix the context flag/heuristic that rejects map literals at top-level `let` scope. Low risk.

**Option B (consistency):** Audit all literal types at module scope — verify arrays, maps, strings, numbers, and structs all work uniformly. Fix any gaps. Slightly broader but gives a stronger guarantee.

### Effort: Small–Medium

### Decision needed
- [ ] Option A — targeted fix
- [ ] Option B — consistency audit + fix
- [ ] Leave as-is (not recommended — inconsistent behavior with no good reason)

---

## Issue #56 — Semicolons as statement separators silently corrupt parser state

### Description

```ntnt
fn f(x) { let a = 1; return a }
```

This silently mangles parser state and reports errors on unrelated lines — not on the actual semicolon. The semicolon is tokenized (`TokenKind::Semicolon`) and consumed opportunistically via `match_token` in most places, but inside function bodies the surrounding statement loop gets confused and the error message points at a random later line.

Every developer coming from JS, Rust, Go, or C hits this immediately. The current error message gives no indication that the semicolon is the problem.

### Fix Options

**Option A — Better error message (low effort, high value):**

In the statement parser, when we hit a `Semicolon` token as a statement start (i.e., after a statement boundary), emit a targeted diagnostic:

```
Error at line N: unexpected ';'
  ntnt uses newlines as statement separators, not semicolons.
  Remove the semicolon and place each statement on its own line.
```

**Option B — Tolerate semicolons as whitespace:**

Treat `;` as a newline-equivalent statement separator. This is a design call — ntnt is intentionally newline-delimited, but silently accepting semicolons would reduce friction significantly for JS/Rust/Go immigrants.

**Option C — Both:** Tolerate them in normal code, warn in strict/lint mode.

### Effort
- Option A: Very low — targeted error diagnostic
- Option B: Medium — parser/lexer change with broad testing needed
- Option C: Medium + adds a lint-mode toggle

### Decision needed
- [ ] Option A only (recommended baseline — minimum viable fix)
- [ ] Option B — tolerate semicolons as statement separators
- [ ] Option C — tolerate + warn in lint mode
- [ ] Leave as-is (not recommended)

---

## Implementation Plan

Once decisions are locked, implementation order:

- [ ] #56-A: Add semicolon diagnostic error message (if A chosen)
- [ ] #56-B: Tolerate semicolons as separators (if B/C chosen)
- [ ] #55: Fix module-level map literal parsing
- [ ] #54-parser: Extend `statement_inner()` return to support `otherwise`
- [ ] #54-ast: Add `otherwise: Option<Block>` to `Statement::Return`
- [ ] #54-interpreter: Handle otherwise block on error/None return value
- [ ] Tests: Add parser tests for all three fixes
- [ ] Update SKILL.md: Remove workaround notes for fixed items
- [ ] Update ntnt-findings/README.md: Mark #54/#55/#56 as resolved
- [ ] `cargo build --release --locked && ./target/release/ntnt docs --generate`
- [ ] PR + Greptile self-review loop

---

## References

- `src/parser.rs` — `statement_inner()` (line ~1301), `let_declaration()` (line ~202), `is_nested_map_literal()` (line ~1795), `parse_map_contents()` (line ~1849)
- `src/lexer.rs` — `TokenKind::Semicolon` (line ~172)
- `~/.openclaw/skills/ntnt/SKILL.md` — lines 61–63 (workaround docs)
- `~/repos/ntnt-findings/README.md` — findings log
