# DD-048: Block Expressions — `let` Inside If/Match Expression Branches

**Status:** draft  
**Author:** larri  
**Created:** 2026-03-27  

## Problem

ntnt has two separate `if` constructs that look identical but have different capabilities:

**If statement** — allows `let` bindings inside branches:
```ntnt
let mut result = "default"
if condition {
    let x = compute()
    result = transform(x)
}
```

**If expression** — branches must be single expressions, no `let` allowed:
```ntnt
let result = if condition { compute() } else { "default" }
```

When a branch needs intermediate computation, you're forced out of expression form:
```ntnt
// This SHOULD work but fails with:
// "Parser error: Expected expression, but found 'let'"
let sig = if fpath != "" {
    let stat = file_stat(path)
    str(stat["modified"])
} else {
    "0"
}

// Workaround: imperative mutation
let mut sig = "0"
if fpath != "" {
    let stat = file_stat(path)
    sig = str(stat["modified"])
}
```

The workaround uses `let mut` + reassignment, which:
- Goes against ntnt's functional style
- Makes the code harder to reason about (mutable state)
- Loses the clarity of "this value is computed from one of these branches"
- Requires more lines for a conceptually simple operation

This affects every developer writing ntnt — it came up during dashboard development (DD-047 review cycle) when building the live-reload polling endpoints.

## Proposal

Allow blocks (sequences of statements ending in an expression) anywhere an expression is expected. The block's value is the last expression in the sequence.

### Syntax

```ntnt
// If expression with block branches
let sig = if fpath != "" {
    let stat = file_stat(path)
    let modified = stat["modified"] ?? 0
    str(modified)
} else {
    "0"
}

// Match expression with block arms
let label = match status {
    "draft" => {
        let count = get_pending_count()
        "Draft (" + str(count) + " pending)"
    }
    "complete" => "Complete"
    _ => status
}

// Standalone block expression
let result = {
    let a = compute_a()
    let b = compute_b()
    a + b
}
```

### Semantics

1. A block `{ stmt1; stmt2; ... ; expr }` evaluates all statements in order
2. The value of the block is the final expression
3. Variables declared with `let` inside the block are scoped to the block
4. If the last item is a statement (not an expression), the block evaluates to `Unit`
5. This applies to all expression contexts: if/else branches, match arms, function arguments, variable bindings

### Precedent

This is how Rust, Kotlin, Scala, and Ruby all work. It's the most natural "blocks are expressions" model and ntnt already has the infrastructure — if-statements already allow blocks, and if-expressions already return values. This just unifies them.

## Codex Review (2026-03-27)

**Reviewer:** Codex (gpt-5.2-codex)

### Key Findings

1. **No new AST node needed.** `Expression::Block(Block)` already exists in `src/ast.rs`, and `eval_block` in `src/interpreter.rs` already evaluates blocks and returns the last statement's value. The proposed `BlockExpression` variant is redundant — just reuse `Expression::Block`.

2. **The actual fix is parser-only.** The if-expression parser currently calls `expression()` for each branch. Change it to call `block()` and wrap the result in `Expression::Block`. That's the entire core change.

3. **Match arms already work.** Match expression parsing already accepts expressions, and `{ let x = 1; x }` already parses as a block expression in expression context. No match parser changes needed.

4. **Map literal ambiguity is more nuanced than stated.** Bare `{ "k": v }` *is* parsed as a map inside `in_map_context` (nested map inference in `src/parser.rs`). Outside map context, `{ }` is always a block. The doc should state this explicitly rather than claiming bare `{}` is always a block.

5. **All statements allowed in branches.** Since `block()` uses `declaration()`, if-expression branches would accept `fn`, `type`, `struct`, etc. — same as standalone block expressions today. This is consistent but worth calling out.

6. **Free-variable analysis already handles blocks.** `collect_free_vars_expr` in `src/stdlib/concurrent.rs` already handles `Expression::Block`. No changes needed there.

### Revised Scope

The implementation is much smaller than originally scoped:
- **Parser change:** ~20 lines in `src/parser.rs` — parse if-expression branches as blocks
- **AST change:** None (reuse existing `Expression::Block`)
- **Interpreter change:** None (`eval_block` already works)
- **Type checker change:** None (`Expression::Block` already handled)
- **Free-vars change:** None (already handled in `concurrent.rs`)

## Implementation

### Parser Changes (Revised per Codex Review)

The fix is entirely in the parser. Currently if-expression branches call `expression()` to parse each branch. Change them to call `block()` and wrap the result in the existing `Expression::Block`.

**Key change in `src/parser.rs`:**
- In if-expression parsing (around line 2046): after consuming `{` for a branch, call `block()` instead of `expression()`, wrap result as `Expression::Block(block)`
- Add a small helper for "parse if-expression branch" so `else if` chains remain clean

**No changes needed in:**
- `src/ast.rs` — `Expression::Block(Block)` already exists
- `src/interpreter.rs` — `eval_block` already returns the last statement's value
- `src/typechecker.rs` — `Expression::Block` already handled with proper scope
- `src/stdlib/concurrent.rs` — `collect_free_vars_expr` already handles `Expression::Block`
- Match expression parsing — already supports block expressions via `{ ... }` syntax

### Scope Rules

- `let` bindings inside a block expression are local to that block (existing behavior)
- They do NOT leak into the surrounding scope (existing behavior)
- All statement types are allowed in branches (`fn`, `type`, etc.) — same as standalone blocks

### Edge Cases

- **Empty block**: `{}` evaluates to `Unit`
- **Block with only statements**: `{ let x = 1; print(x); }` evaluates to `Unit` (last expression-statement's value)
- **Nested blocks**: `{ let a = { let b = 1; b + 1 }; a * 2 }` evaluates to `4`
- **Block in function args**: `foo({ let x = 1; x + 2 })` passes `3` to `foo`
- **Map in block context**: `map { "k": { let x = 1; x } }` — inside `in_map_context`, bare `{ }` is parsed as a nested map, so the explicit `map` keyword is required for the outer map. Block expressions inside map values work because they're parsed as expressions.
- **Control flow in expression blocks**: `return`, `break`, `continue` propagate as `Value::Return/Break/Continue` — consistent with existing block behavior

## Implementation Checklist

- [ ] Update if-expression parser to call `block()` for branches (`src/parser.rs`)
- [ ] Add helper function for parsing if-expression branches (handles `else if` chains)
- [ ] Update `ntnt docs --generate` for AI agent guide
- [ ] Tests: if-expression with `let` in branches — `let x = if true { let a = 1; a + 2 } else { 0 }`
- [ ] Tests: if-expression branch ending in statement — value is `Unit`
- [ ] Tests: nested if-expression with block branches
- [ ] Tests: scope isolation — `let` in if-expression branch doesn't leak
- [ ] Tests: map literal containing block expression value — `map { "k": { let x = 1; x } }`
- [ ] Tests: block in binary expression `{ 1 + 1 } * 2` (already works, verify)
- [ ] Tests: `spawn()` / `schedule()` with if-expression block branches (verify existing coverage)

## Risks

- **Map literal ambiguity (nuanced)**: Inside `in_map_context` (nested maps), bare `{ "k": v }` is parsed as a map via `is_nested_map_literal()`. Outside map context, bare `{ }` is always a block. This existing behavior is correct and the change doesn't affect it — but it should be documented clearly.
- **All statements in branches**: Since branches become full blocks, `fn`, `type`, `struct` etc. are technically allowed inside if-expression branches. This matches standalone block expressions and is unlikely to cause issues in practice.
- **Performance**: Negligible — same scope creation cost as existing blocks.
- **Migration**: Purely additive — no existing code breaks. New syntax is opt-in.
