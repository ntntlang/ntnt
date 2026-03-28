# DD-050: Function Type in Generics

**Status:** Backlog  
**Author:** Larri  
**Created:** 2026-03-27  
**Triggered by:** PR #61 review — `parallel`/`race` typed as `Array<Any>` because `Array<Function>` isn't expressible

---

## Problem

ntnt's type system has no way to express "array of functions" or "function with specific arity" in type signatures. This means:

```rust
// What we want:
sig!("parallel", ["fns" => Type::Array(Box::new(Type::Function { params: vec![], return_type: Box::new(Type::Any) }))], ...);
sig!("spawn",    ["handler" => Type::Function { params: vec![], return_type: Box::new(Type::Any) }], ...);

// What we have:
sig!("parallel", ["fns" => Type::Array(Box::new(Type::Any))], Type::Any);
sig!("spawn",    ["handler" => Type::Any], Type::Named("Task".to_string()));
```

Every function that takes a callback (`spawn`, `parallel`, `race`, `schedule`, `after`, `map`, `filter`, `reduce`, `sort_by`) is typed as `Any` for the function parameter. The typechecker can't catch:

- `parallel([1, 2, 3])` — passes type check, crashes at runtime
- `spawn("hello")` — passes type check, crashes at runtime
- `map([1,2,3], 42)` — passes type check, crashes at runtime

## Scope

### What's needed

1. **`Type::Function` usable in signatures** — the enum variant exists in `src/typechecker.rs` but isn't used in `sig!` macro calls for stdlib functions
2. **`Type::Function` in `Type::Array` generics** — `Array<Function>` and `Array<() -> Any>`
3. **Arity checking at type level** — `parallel` requires zero-arg functions, `map` requires one-arg functions
4. **Return type propagation** — if `parallel` takes `Array<() -> T>`, it returns `Array<Result<T, String>>`

### What's NOT needed (out of scope)

- Named function types / type aliases for functions
- Higher-kinded types
- Closure type inference
- Generic function definitions

## Affected Functions

| Function | Current Sig | Desired Sig |
|----------|------------|-------------|
| `spawn` | `Any -> Task` | `(() -> Any) -> Task` |
| `parallel` | `Array<Any> -> Any` | `Array<() -> Any> -> Array \| Err` |
| `race` | `Array<Any> -> Any` | `Array<() -> Any> -> Result<Any, String>` |
| `after` | `(Int, Any) -> Task` | `(Int, () -> Any) -> Task` |
| `schedule` | `(Int, Any) -> Schedule` | `(Int, () -> Any) -> Schedule` |
| `map` | `(Array, Any) -> Array` | `(Array<T>, (T) -> U) -> Array<U>` |
| `filter` | `(Array, Any) -> Array` | `(Array<T>, (T) -> Bool) -> Array<T>` |
| `reduce` | `(Array, Any, Any) -> Any` | `(Array<T>, (U, T) -> U, U) -> U` |
| `sort_by` | `(Array, Any) -> Array` | `(Array<T>, (T, T) -> Int) -> Array<T>` |

## Implementation Plan

- [ ] Verify `Type::Function { params, return_type }` works in `sig!` macro
- [ ] Update `spawn`, `after`, `schedule` signatures to use `Type::Function`
- [ ] Update `parallel`, `race` signatures to use `Array<Function>`
- [ ] Update `map`, `filter`, `reduce`, `sort_by` signatures
- [ ] Add typechecker tests: wrong type passed to function parameter → type error
- [ ] Add typechecker tests: wrong arity function → type error
- [ ] Verify no false positives on valid code

## Estimated Effort

1-2 days. The `Type::Function` variant already exists — this is wiring it into signatures and making the checker enforce it.

## Risks

- **False positives:** Functions stored in variables lose type info. `let f = fn() { 42 }; spawn(f)` — does the typechecker know `f` is a zero-arg function? Needs investigation.
- **Backward compatibility:** Tightening types could break existing code that passes through intermediaries. Should be warnings first, not errors.
