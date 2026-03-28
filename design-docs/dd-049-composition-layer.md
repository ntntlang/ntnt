# DD-049: Composition Layer — `parallel` and `race`

**Status:** In Review — PR #61  
**Author:** Larri  
**Created:** 2026-03-27  
**Parent:** DD-037 Phase 4  
**Reviewed by:** Codex (4 passes), Copilot, Greptile

---

## Problem

ntnt has solid low-level concurrency (`spawn`, `await_task`, channels, `select`), but common patterns require boilerplate:

```ntnt
let t1 = spawn(fn() { fetch(url1) })
let t2 = spawn(fn() { fetch(url2) })
let t3 = spawn(fn() { fetch(url3) })
let r1 = await_task(t1)
let r2 = await_task(t2)
let r3 = await_task(t3)
```

No way to race multiple sources and take the first success. No automatic cleanup when one task fails.

---

## API

```ntnt
import { parallel, race } from "std/concurrent"

let [a, b, c] = parallel([fn() { fetch(url1) }, fn() { fetch(url2) }, fn() { fetch(url3) }])

let winner = race([fn() { fetch(primary) }, fn() { fetch(fallback) }])
```

---

## `parallel(fns) → Array | Err`

Run N functions concurrently. Wait for all. Return results in input order.

- Empty array → `[]`
- All succeed → `Array` of `Ok(value)` results
- Any task fails (crash OR returned `Err`) → cancel all, return that `Err` value
- Failure detection is in await order, not chronological

Works with `otherwise`:
```ntnt
let results = parallel([fn() { fetch(url) }]) otherwise { return [] }
```

---

## `race(fns) → Ok(value) | Err`

Run N functions concurrently. Return the first successful result. Cancel the rest.

- Empty array → runtime error
- First `Ok` result wins → cancel all, return it
- Tasks that crash or return `Err` are skipped
- All fail → return last `Err`

---

## Error Handling

Both functions treat task crashes AND user-returned `Err` values as failures. A task that returns `Err("connection refused")` is treated the same as `1/0` — it's a failure, not a success. This matches `Promise.all`, `tokio::try_join!`, and Go's `errgroup`.

`is_task_failure()` checks both outer `Err` (crash) and `Ok(Err(...))` (returned error). `extract_inner_err()` unwraps the double-wrapped value for clean error returns.

---

## Future: `task_group` (v2)

Deferred. Requires runtime changes (`Value::NativeFunction` can't capture state, stdlib can't call user closures).

---

## Implementation Checklist

- [x] `concurrent_parallel` in `src/stdlib/concurrent.rs`
- [x] `concurrent_race` in `src/stdlib/concurrent.rs`
- [x] `is_task_failure` + `extract_inner_err` helpers
- [x] Spawn loop cleanup (cancel already-spawned on failure)
- [x] Await error cleanup (cancel siblings on Rust-level + ntnt-level errors)
- [x] Parent cancellation cleanup (cancel children before propagating)
- [x] Register in `std_concurrent_module()` — arity 1, `RuntimeCapability::TaskSpawning`
- [x] Typechecker signatures in `src/typechecker.rs`
- [x] `// @ntnt` doc blocks
- [x] `ntnt docs --generate`
- [x] Tests (11 total)

## Tests

- [x] `parallel` — results in input order
- [x] `parallel` — empty array returns `[]`
- [x] `parallel` — crash cancels others
- [x] `parallel` — returned `Err` cancels others
- [x] `parallel` — parent cancellation cancels children
- [x] `race` — fastest `Ok` wins
- [x] `race` — crash then success: second wins
- [x] `race` — returned `Err` skipped, `Ok` wins
- [x] `race` — all fail: `Err` returned
- [x] `race` — empty array: runtime error
- [x] `race` — parent cancellation cancels children
