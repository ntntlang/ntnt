# DD-052 Phase 3: Dynamic Batch Additions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

---

## Review Notes (2026-04-01 — Codex + Larri)

**Status: Approved with corrections.** The plan is well-structured, task ordering is correct, all call sites and test helpers are verified against the codebase. The following items need attention during implementation:

### ⚠️ Bug: Task 6 Early Return Skips Metadata Update

The `return Ok(())` when `kv_set_nx` on the closed flag fails (second worker loses the race) causes that worker to skip the metadata update at the end of `update_batch_on_terminal()`. This means `batch_status()` could show stale metadata — missing `status: "complete"` and `completed_at`.

**Fix:** Don't early-return. Instead, set `fire_complete = false` (and `fire_death`/`fire_success` to false) and let execution continue to the metadata update. The closed flag still prevents duplicate callbacks, but both workers write metadata. The second metadata write is idempotent (same status, slightly different timestamp — harmless).

```rust
// INSTEAD OF:
if !we_closed { return Ok(()); }

// DO:
if !we_closed {
    // Another worker closed — skip callbacks but still update metadata.
    fire_complete = false;
    fire_death = false;
    fire_success = false;
}
```

### Line Reference Offsets (~60 lines)

Plan line numbers are approximate and will shift as earlier tasks insert code. Actual locations verified:
- Thread-local insertion point: after line ~540 (BATCH_RUNTIME static), not ~480
- `kv_expire` in kv.rs: line 2304, not 2340 (Task 1 `kv_ttl` should go after 2340, which is after `kv_expire`)
- `batch_id()` stub: line ~5082, not ~5092
- Worker loop `execute_in_worker` call: line 1930

These are close enough for sequential implementation — each task's context (function names, surrounding code) is sufficient to locate the right spot.

### Minor: `kv_expire` Failure Logging

Spec §4 says: "If `kv_expire` fails on a counter key, log a warning but don't fail the completion." Task 7 silently ignores with `let _ =`. Consider adding `eprintln!` for debuggability, but this is not blocking.

### Minor: `enqueue_into` Accepts Map Handles

The plan's `enqueue_into` registration accepts both `Value::String` (batch ID) and `Value::Map` (batch handle with `_batch_id`). The spec says String only. The Map acceptance is actually a nice ergonomic addition — keeps `enqueue_into(batch_id(), ...)` and `enqueue_into(handle, ...)` both working. No change needed.

### Verified ✅

- All 7 `enqueue_internal` call sites identified and correctly handled
- `get_fn` helper defined at line 6077 in the test module
- `test_job_def_with_opts` exists at line 5179
- `build_batch_meta` and `update_batch_on_terminal` accessible via `use super::*`
- Task dependency chain is clean (no cycles)
- `RefCell` import may be needed — check if already in scope via existing imports
- `kv_ttl` function doesn't exist yet — Task 1 correctly adds it as prerequisite

---

**Goal:** Wire `batch_id()` context into perform blocks, enable `enqueue_into()` for dynamic batch job addition, add batch expiry TTLs, and achieve full test coverage for all edge cases.

**Architecture:** All changes are in `src/stdlib/jobs.rs` (with one small addition to `src/stdlib/kv.rs` for TTL testing). Thread-local `CURRENT_BATCH_ID` with RAII guard provides batch context. `enqueue_into()` is a new stdlib function that writes directly to KV + atomically increments counters for post-seal dynamic adds. A `closed` flag serializes completion vs dynamic add races. `counter:total` becomes an atomic KV key (replacing metadata-only `total`) for correctness under dynamic adds. Seal and completion paths get proper TTLs.

**Tech Stack:** Rust, SQLite KV backend, existing `kv::kv_set`, `kv::kv_incr`, `kv::kv_set_nx`, `kv::kv_expire` primitives.

**Spec:** `docs/superpowers/specs/2026-04-01-dd052-phase3-dynamic-batch-additions-design.md`

---

## File Map

| File | Changes |
|------|---------|
| `src/stdlib/jobs.rs:~480` | Add `CURRENT_BATCH_ID` thread-local + `BatchIdGuard` struct (near batch runtime section) |
| `src/stdlib/jobs.rs:~692` | Modify `update_batch_on_terminal()`: closed flag, `counter:total` read, TTL shortening |
| `src/stdlib/jobs.rs:~1159` | Modify `enqueue_internal()`: return `EnqueueResult` enum instead of `Result<Value>` |
| `src/stdlib/jobs.rs:~1930` | Modify worker loop: set/clear `CURRENT_BATCH_ID` around `execute_in_worker()` |
| `src/stdlib/jobs.rs:~4862` | Modify `seal()`: add 30d TTL to metadata + counters, add `counter:total` init |
| `src/stdlib/jobs.rs:~5065` | Modify `batch_status()`: merge `counter:total` into response |
| `src/stdlib/jobs.rs:~5092` | Modify `batch_id()`: read from thread-local instead of returning `None` |
| `src/stdlib/jobs.rs:~5101` | Add `enqueue_into()` stdlib function registration |
| `src/stdlib/jobs.rs:~5101` | Add `enqueue_to_sealed_batch()` helper function |
| `src/stdlib/jobs.rs:~7859+` | Add 26 new tests across 6 groups |
| `src/stdlib/kv.rs` | Add `pub fn kv_ttl()` wrapper (needed for TTL tests in jobs.rs) |

---

## Task 1: Add `kv_ttl()` Public Function

**Files:**
- Modify: `src/stdlib/kv.rs`

This is a prerequisite — TTL tests in later tasks need to read TTL values from KV keys. The internal `SQLiteKV::ttl()` and `RedisKV::ttl()` methods already exist but aren't exposed via a public `kv_ttl()` function.

- [ ] **Step 1: Add `kv_ttl` function to kv.rs**

Add this after the `kv_expire` function (around line 2340 in `src/stdlib/kv.rs`):

```rust
/// Read the remaining TTL (in seconds) for a key.
/// Returns `Ok(Some(seconds))` if a TTL is set, `Ok(None)` if no expiry, or error.
pub fn kv_ttl(handle: &Value, key: &str) -> Result<Option<i64>> {
    let backend = get_backend_type(handle)?;
    match backend {
        KVBackend::SQLite => {
            let kv_arc = get_sqlite_kv(handle)?;
            let kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.ttl(key)
        }
        KVBackend::Redis => {
            let kv_arc = get_redis_kv(handle)?;
            let mut kv = kv_arc
                .lock()
                .map_err(|e| IntentError::runtime_error(format!("KV lock error: {}", e)))?;
            kv.ttl(key)
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --profile dev-release 2>&1 | tail -5`
Expected: Build succeeds (no errors).

- [ ] **Step 3: Commit**

```bash
git add src/stdlib/kv.rs
git commit -m "feat(kv): expose kv_ttl() public function for TTL inspection"
```

---

## Task 2: Add `EnqueueResult` Enum and Refactor `enqueue_internal()`

**Files:**
- Modify: `src/stdlib/jobs.rs:1159-1437` (enqueue_internal)
- Modify: `src/stdlib/jobs.rs` (all call sites of enqueue_internal)

The spec requires `enqueue_into()` to detect dedup collisions and roll back counters. Currently `enqueue_internal` returns `Result<Value>` with no way to distinguish "created" from "deduplicated".

- [ ] **Step 1: Add `EnqueueResult` enum**

Add this near the top of the batch runtime section (around line 478 in `src/stdlib/jobs.rs`, before the `BufferedJob` struct):

```rust
/// Result of an enqueue operation, distinguishing new jobs from dedup collisions.
/// Used by `enqueue_into()` to roll back batch counters on dedup.
#[derive(Debug)]
enum EnqueueResult {
    /// A new job was created and written to KV.
    Created(String),
    /// A dedup collision occurred — the returned ID is the existing job.
    Deduplicated(String),
}
```

- [ ] **Step 2: Change `enqueue_internal` return type**

Change the function signature at line 1159 from:

```rust
fn enqueue_internal(
    job_name: &str,
    payload: Value,
    pending_ts: &str,
    scheduled_at: Option<&str>,
    batch_id: Option<&str>,
    override_job_id: Option<&str>,
) -> Result<Value> {
```

to:

```rust
fn enqueue_internal(
    job_name: &str,
    payload: Value,
    pending_ts: &str,
    scheduled_at: Option<&str>,
    batch_id: Option<&str>,
    override_job_id: Option<&str>,
) -> Result<EnqueueResult> {
```

- [ ] **Step 3: Update the dedup early-return at line ~1309**

Change the dedup collision return from:

```rust
return Ok(Value::ok(Value::String(existing_id)));
```

to:

```rust
return Ok(EnqueueResult::Deduplicated(existing_id));
```

- [ ] **Step 4: Update the successful return at line ~1436**

Change the final return from:

```rust
Ok(Value::ok(Value::String(job_id)))
```

to:

```rust
Ok(EnqueueResult::Created(job_id))
```

- [ ] **Step 5: Update the test-mode early return at line ~1340**

Change:

```rust
return Ok(Value::ok(Value::String(job_id)));
```

to:

```rust
return Ok(EnqueueResult::Created(job_id));
```

- [ ] **Step 6: Update all call sites to unwrap `EnqueueResult` back to `Value`**

There are 7 call sites. Each currently uses the `Result<Value>` directly. Wrap each with a helper or inline conversion. Add this helper function just before `enqueue_internal`:

```rust
/// Convert an `EnqueueResult` to the `Value::ok(Value::String(job_id))` format
/// expected by ntnt stdlib callers.
fn enqueue_result_to_value(result: EnqueueResult) -> Value {
    match result {
        EnqueueResult::Created(id) | EnqueueResult::Deduplicated(id) => {
            Value::ok(Value::String(id))
        }
    }
}
```

Then update each call site:

**Call site 1** — `fire_batch_callback` (~line 651):
```rust
// Before:
enqueue_internal("_BatchCallback", Value::Map(payload), &timestamp_key(), None, None, Some(&cb_job_id))?;
// After:
enqueue_internal("_BatchCallback", Value::Map(payload), &timestamp_key(), None, None, Some(&cb_job_id))?;
```
No change needed — this caller only checks `?` for errors, ignores the Ok value.

**Call site 2** — 2-arg enqueue (~line 3357):
```rust
// Before:
enqueue_internal(&job_name, payload, &timestamp_key(), None, None, None)
// After:
enqueue_internal(&job_name, payload, &timestamp_key(), None, None, None)
    .map(enqueue_result_to_value)
```

**Call site 3** — enqueue_in (~line 3604):
```rust
// Before:
enqueue_internal(&job_name, payload, &pending_ts, Some(&scheduled_ts), None, None)
// After:
enqueue_internal(&job_name, payload, &pending_ts, Some(&scheduled_ts), None, None)
    .map(enqueue_result_to_value)
```

**Call site 4** — enqueue_at (~line 3678):
```rust
// Before:
enqueue_internal(&job_name, payload, &pending_ts, Some(&scheduled_ts), None, None)
// After:
enqueue_internal(&job_name, payload, &pending_ts, Some(&scheduled_ts), None, None)
    .map(enqueue_result_to_value)
```

**Call site 5** — enqueue_bulk (~line 4625):
```rust
// Before:
let result = enqueue_internal(&job_name, item, &ts, None, None, None).map_err(|e| { ... })?;
// After:
let result = enqueue_internal(&job_name, item, &ts, None, None, None)
    .map(enqueue_result_to_value)
    .map_err(|e| { ... })?;
```

**Call site 6** — seal flush loop (~line 4930):
```rust
// Before:
enqueue_internal(job_type, payload.clone(), ts, None, Some(&batch_id), None)?;
// After:
enqueue_internal(job_type, payload.clone(), ts, None, Some(&batch_id), None)?;
```
No change needed — uses `?`, ignores Ok value.

**Call site 7** — test code (~line 8552):
```rust
// Before:
enqueue_internal("ProcessRow", ...).unwrap();
// After:
enqueue_internal("ProcessRow", ...).unwrap();
```
No change needed — tests ignore Ok value.

- [ ] **Step 7: Verify it compiles**

Run: `cargo build --profile dev-release 2>&1 | tail -5`
Expected: Build succeeds.

- [ ] **Step 8: Run existing tests**

Run: `cargo test --lib jobs::tests 2>&1 | tail -20`
Expected: All existing tests pass (no regressions).

- [ ] **Step 9: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "refactor(jobs): add EnqueueResult enum to distinguish dedup from creation"
```

---

## Task 3: Add Thread-Local `CURRENT_BATCH_ID` and `BatchIdGuard`

**Files:**
- Modify: `src/stdlib/jobs.rs:~480` (add thread-local + guard)
- Modify: `src/stdlib/jobs.rs:~5092` (update `batch_id()` implementation)

- [ ] **Step 1: Write test `test_batch_id_returns_none_outside_job`**

Add at the end of the test module (after the last existing batch test):

```rust
#[test]
fn test_batch_id_returns_none_outside_job() {
    with_clean_runtime(|| {
        let module = init();
        let batch_id_fn = get_fn(&module, "batch_id");
        let result = batch_id_fn(&[]).unwrap();
        assert!(
            matches!(result, Value::EnumValue { ref variant, .. } if variant == "None"),
            "batch_id() must return None outside a job perform block"
        );
    });
}
```

- [ ] **Step 2: Run test to verify it passes (existing stub returns None)**

Run: `cargo test --lib jobs::tests::test_batch_id_returns_none_outside_job -- --exact 2>&1 | tail -10`
Expected: PASS (the current stub already returns `Value::none()`).

- [ ] **Step 3: Write test `test_batch_id_returns_id_inside_batch_job`**

```rust
#[test]
fn test_batch_id_returns_id_inside_batch_job() {
    with_clean_runtime(|| {
        let module = init();
        let batch_id_fn = get_fn(&module, "batch_id");

        // Simulate worker setting the thread-local before execute_in_worker
        CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = Some("test-batch-123".to_string()));
        let _guard = BatchIdGuard;

        let result = batch_id_fn(&[]).unwrap();
        match result {
            Value::String(ref s) => assert_eq!(s, "test-batch-123"),
            _ => panic!("batch_id() must return String inside a batch job, got {:?}", result),
        }
    });
}
```

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo test --lib jobs::tests::test_batch_id_returns_id_inside_batch_job -- --exact 2>&1 | tail -10`
Expected: FAIL — `CURRENT_BATCH_ID` and `BatchIdGuard` don't exist yet.

- [ ] **Step 5: Add thread-local and guard**

Add after the `BATCH_RUNTIME` static (around line 540) in `src/stdlib/jobs.rs`:

```rust
// Thread-local batch ID context for batch_id() — set by worker loop before
// execute_in_worker(), read by batch_id() stdlib function.
thread_local! {
    static CURRENT_BATCH_ID: RefCell<Option<String>> = RefCell::new(None);
}

/// RAII guard that unconditionally clears CURRENT_BATCH_ID on drop.
/// Guarantees cleanup even on panic (panic=unwind) or early return.
struct BatchIdGuard;

impl Drop for BatchIdGuard {
    fn drop(&mut self) {
        CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = None);
    }
}
```

Also add the `RefCell` import if not already present. Check the existing imports at the top of the file — `std::cell::RefCell` may need to be added.

- [ ] **Step 6: Update `batch_id()` implementation**

Replace the stub at line ~5092-5101:

```rust
module.insert(
    "batch_id".to_string(),
    Value::NativeFunction {
        name: "batch_id".to_string(),
        arity: 0,
        max_arity: 0,
        requires: None,
        func: |_args| {
            let bid = CURRENT_BATCH_ID.with(|c| c.borrow().clone());
            match bid {
                Some(id) => Ok(Value::String(id)),
                None => Ok(Value::none()),
            }
        },
    },
);
```

Update the `@ntnt` doc comment above it — change "Phase 1: always returns None. Phase 2 wires up thread-local job context." to "Uses thread-local context set by the worker loop. Returns None when called outside a batch job."

- [ ] **Step 7: Run both tests**

Run: `cargo test --lib jobs::tests::test_batch_id -- 2>&1 | tail -10`
Expected: Both tests PASS.

- [ ] **Step 8: Write test `test_batch_id_cleared_after_job_execution`**

```rust
#[test]
fn test_batch_id_cleared_after_job_execution() {
    with_clean_runtime(|| {
        let module = init();
        let batch_id_fn = get_fn(&module, "batch_id");

        // Simulate: set batch_id, create guard, drop guard
        {
            CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = Some("batch-abc".to_string()));
            let _guard = BatchIdGuard;
            // Guard drops here
        }

        // After guard drops, batch_id() must return None
        let result = batch_id_fn(&[]).unwrap();
        assert!(
            matches!(result, Value::EnumValue { ref variant, .. } if variant == "None"),
            "batch_id() must return None after guard drops"
        );
    });
}
```

- [ ] **Step 9: Write test `test_batch_id_none_for_non_batch_job`**

```rust
#[test]
fn test_batch_id_none_for_non_batch_job() {
    with_clean_runtime(|| {
        let module = init();
        let batch_id_fn = get_fn(&module, "batch_id");

        // Simulate: batch job runs (set batch_id), then non-batch job runs (set None)
        {
            CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = Some("batch-xyz".to_string()));
            let _guard = BatchIdGuard;
        }
        // Now simulate non-batch job — explicitly set None (as the worker loop would)
        {
            CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = None);
            let _guard = BatchIdGuard;

            let result = batch_id_fn(&[]).unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "None"),
                "batch_id() must return None for non-batch job"
            );
        }
    });
}
```

- [ ] **Step 10: Write test `test_batch_id_cleared_on_panic`**

```rust
#[test]
fn test_batch_id_cleared_on_panic() {
    with_clean_runtime(|| {
        let module = init();
        let batch_id_fn = get_fn(&module, "batch_id");

        // Simulate: set batch_id, panic inside scope, guard should still clean up
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = Some("panic-batch".to_string()));
            let _guard = BatchIdGuard;
            panic!("simulated panic in perform block");
        }));
        assert!(result.is_err(), "should have caught panic");

        // After panic, batch_id() must return None (guard cleaned up during unwind)
        let result = batch_id_fn(&[]).unwrap();
        assert!(
            matches!(result, Value::EnumValue { ref variant, .. } if variant == "None"),
            "batch_id() must return None after panic (guard cleanup)"
        );
    });
}
```

- [ ] **Step 11: Write test `test_batch_id_available_in_on_failure`**

This test verifies the guard scope encompasses both perform and on_failure execution. Since we can't easily invoke `execute_on_failure_in_worker` without a full interpreter, we verify the simpler invariant: the guard stays alive across multiple function calls within its scope.

```rust
#[test]
fn test_batch_id_available_in_on_failure() {
    with_clean_runtime(|| {
        let module = init();
        let batch_id_fn = get_fn(&module, "batch_id");

        // Simulate: set batch_id, call batch_id() twice (simulating perform + on_failure)
        CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = Some("failure-batch".to_string()));
        let _guard = BatchIdGuard;

        // First call (simulates perform block reading batch_id)
        let result1 = batch_id_fn(&[]).unwrap();
        match &result1 {
            Value::String(s) => assert_eq!(s, "failure-batch"),
            _ => panic!("batch_id() must return String, got {:?}", result1),
        }

        // Second call (simulates on_failure handler reading batch_id — guard still alive)
        let result2 = batch_id_fn(&[]).unwrap();
        match &result2 {
            Value::String(s) => assert_eq!(s, "failure-batch"),
            _ => panic!("batch_id() must still return String in on_failure context, got {:?}", result2),
        }
    });
}
```

- [ ] **Step 12: Run all batch_id tests**

Run: `cargo test --lib jobs::tests::test_batch_id -- 2>&1 | tail -15`
Expected: All 6 tests PASS.

- [ ] **Step 13: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "feat(jobs): wire batch_id() to thread-local context with RAII guard"
```

---

## Task 4: Wire `CURRENT_BATCH_ID` in the Worker Loop

**Files:**
- Modify: `src/stdlib/jobs.rs:~1928-1932` (around execute_in_worker call)

- [ ] **Step 1: Add batch context setup in the worker loop**

Find the `execute_in_worker` call at line ~1930:

```rust
let exec_result = execute_in_worker(&mut interp, &def, &payload);
```

Replace with:

```rust
// Set batch context for batch_id() — read batch_id from job_data.
// Explicitly set None for non-batch jobs to clear any stale value.
let batch_id_for_context = match job_data.get("batch_id") {
    Some(Value::String(bid)) => Some(bid.clone()),
    _ => None,
};
CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = batch_id_for_context);
let _batch_guard = BatchIdGuard;

let exec_result = execute_in_worker(&mut interp, &def, &payload);
```

The `_batch_guard` will drop at the end of the current loop iteration's scope, which is after both `execute_in_worker` and `execute_on_failure_in_worker` — this means `batch_id()` is available in `on_failure` handlers too.

Verify that the `_batch_guard` variable lives long enough by checking the scope — it must survive past the `execute_on_failure_in_worker` call at line ~2034. The guard is declared in the main loop body scope, which encompasses both the exec_result handling and the on_failure call, so it will remain alive.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build --profile dev-release 2>&1 | tail -5`
Expected: Build succeeds.

- [ ] **Step 3: Run all existing tests**

Run: `cargo test --lib jobs::tests 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 4: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "feat(jobs): set CURRENT_BATCH_ID in worker loop before job execution"
```

---

## Task 5: Add `counter:total` to `seal()` and Update `batch_status()` and `update_batch_on_terminal()`

**Files:**
- Modify: `src/stdlib/jobs.rs:~4862` (seal empty batch path)
- Modify: `src/stdlib/jobs.rs:~4921` (seal non-empty counter init)
- Modify: `src/stdlib/jobs.rs:~5065` (batch_status counter merge)
- Modify: `src/stdlib/jobs.rs:~734` (update_batch_on_terminal total read)

- [ ] **Step 1: Write test `test_total_counter_initialized_at_seal`**

```rust
#[test]
fn test_total_counter_initialized_at_seal() {
    with_temp_kv("ntnt_total_counter_seal_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("total-ctr-test".to_string())]).unwrap();
        for i in 0..5 {
            let mut payload = HashMap::new();
            payload.insert("x".to_string(), Value::Int(i));
            enqueue_fn(&[
                handle.clone(),
                Value::String("ProcessRow".to_string()),
                Value::Map(payload),
            ])
            .unwrap();
        }
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };
        let cp = format!("jobs:batch:{}:counter", bid);
        let total = kv::kv_get(kv, &format!("{}:total", cp)).unwrap();
        assert!(
            matches!(total, Value::Int(5)),
            "counter:total must be 5 after sealing 5 jobs, got {:?}",
            total
        );
    });
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib jobs::tests::test_total_counter_initialized_at_seal -- --exact 2>&1 | tail -10`
Expected: FAIL — `counter:total` key doesn't exist yet.

- [ ] **Step 3: Add `counter:total` initialization in `seal()` — non-empty path**

At line ~4925, after the `cancelled` counter init, add:

```rust
kv::kv_set(&kv_handle, &format!("{}:total", cp), &Value::Int(total), None)?;
```

- [ ] **Step 4: Add `counter:total` initialization in `seal()` — empty batch path**

At line ~4879, after the `cancelled` counter init in the empty batch branch, add:

```rust
kv::kv_set(&kv_handle, &format!("{}:total", cp), &Value::Int(0), None)?;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `cargo test --lib jobs::tests::test_total_counter_initialized_at_seal -- --exact 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 6: Write test `test_batch_status_includes_total_counter`**

```rust
#[test]
fn test_batch_status_includes_total_counter() {
    with_temp_kv("ntnt_batch_status_total_test.db", |_kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let status_fn = get_fn(&module, "batch_status");

        let handle = batch_fn(&[Value::String("status-total-test".to_string())]).unwrap();
        for i in 0..3 {
            let mut payload = HashMap::new();
            payload.insert("x".to_string(), Value::Int(i));
            enqueue_fn(&[
                handle.clone(),
                Value::String("ProcessRow".to_string()),
                Value::Map(payload),
            ])
            .unwrap();
        }
        seal_fn(&[handle.clone()]).unwrap();

        let status = status_fn(&[handle.clone()]).unwrap();
        let status_map = match status {
            Value::EnumValue { ref value, .. } => match value.as_ref() {
                Value::Map(m) => m.clone(),
                _ => panic!("expected map in Ok"),
            },
            _ => panic!("expected Ok variant"),
        };
        assert!(
            matches!(status_map.get("total"), Some(Value::Int(3))),
            "batch_status total must be 3 (from counter), got {:?}",
            status_map.get("total")
        );
    });
}
```

- [ ] **Step 7: Update `batch_status()` to merge `counter:total`**

At line ~5065, change the counter merge loop from:

```rust
for counter in &["pending", "succeeded", "dead", "cancelled"] {
```

to:

```rust
for counter in &["pending", "succeeded", "dead", "cancelled", "total"] {
```

- [ ] **Step 8: Update `update_batch_on_terminal()` to read `total` from counter key**

At line ~734, replace the metadata `total` read:

```rust
let total_jobs = match kv::kv_get(kv_handle, &meta_key) {
    Ok(Value::Map(ref m)) => match m.get("total") {
        Some(Value::Int(n)) => *n,
        _ => 0,
    },
    _ => {
        // Metadata missing — release the done-set claim so a future retry
        // can attempt this update once the metadata reappears.
        let _ = kv::kv_del(kv_handle, &done_key);
        eprintln!(
            "[ntnt] warning: batch metadata not found for batch '{}' (job '{}'), released done-set",
            batch_id, job_id
        );
        return Ok(());
    }
};
```

with:

```rust
// Read total from atomic counter key (not metadata) — dynamic adds
// increment counter:total, making the metadata total stale.
let counter_prefix = format!("jobs:batch:{}:counter", batch_id);
let total_jobs = match kv::kv_get(kv_handle, &format!("{}:total", counter_prefix)) {
    Ok(Value::Int(n)) => n,
    _ => {
        // Counter key missing — fall back to metadata total for
        // backwards compatibility with batches sealed before Phase 3.
        match kv::kv_get(kv_handle, &meta_key) {
            Ok(Value::Map(ref m)) => match m.get("total") {
                Some(Value::Int(n)) => *n,
                _ => 0,
            },
            _ => {
                let _ = kv::kv_del(kv_handle, &done_key);
                eprintln!(
                    "[ntnt] warning: batch metadata not found for batch '{}' (job '{}'), released done-set",
                    batch_id, job_id
                );
                return Ok(());
            }
        }
    }
};
```

Note: the `counter_prefix` variable is now declared earlier (before the counter init `kv_set_nx` loop). Move the existing `let counter_prefix = ...` declaration (line ~755) up to right after the `total_jobs` calculation, or use the one declared above. Make sure there's no duplicate declaration — the existing one at line ~755 should be removed and replaced by the earlier one.

Also add `"total"` to the counter init `kv_set_nx` loop at line ~759:

```rust
for suffix in &["pending", "succeeded", "dead", "cancelled", "total"] {
```

- [ ] **Step 9: Run all tests**

Run: `cargo test --lib jobs::tests 2>&1 | tail -20`
Expected: All tests pass (including the two new ones).

- [ ] **Step 10: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "feat(jobs): add atomic counter:total, use it in batch_status and update_batch_on_terminal"
```

---

## Task 6: Add Closed Flag to `update_batch_on_terminal()`

**Files:**
- Modify: `src/stdlib/jobs.rs:~830-961` (callback firing section of update_batch_on_terminal)

- [ ] **Step 1: Write test `test_closed_flag_set_on_completion`**

```rust
#[test]
fn test_closed_flag_set_on_completion() {
    with_temp_kv("ntnt_closed_flag_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("closed-flag-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Find the job ID
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_id = data_keys
            .iter()
            .find(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .expect("should find a non-callback job");

        // Read job data for the update call
        let job_data = match kv::kv_get(kv, &format!("jobs:data:{}", job_id)).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };

        // Simulate job completion
        update_batch_on_terminal(kv, &job_data, &job_id, "succeeded").unwrap();

        // Closed flag must exist
        let closed_key = format!("jobs:batch:{}:closed", bid);
        let closed = kv::kv_get(kv, &closed_key).unwrap();
        assert!(
            matches!(closed, Value::Bool(true)),
            "closed flag must be set after batch completion"
        );
    });
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib jobs::tests::test_closed_flag_set_on_completion -- --exact 2>&1 | tail -10`
Expected: FAIL — closed flag not set.

- [ ] **Step 3: Add closed flag to `update_batch_on_terminal()`**

In the callback conditions section (after line ~837, where `fire_complete` is calculated), add the closed flag logic.

**Important:** The existing `fire_death`, `fire_complete`, and `fire_success` bindings must become `let mut` since the closed flag logic may reassign them when a second worker loses the `kv_set_nx` race (see Review Notes above).

Find:

```rust
let fire_complete = new_pending == 0 && !pending_underflow;
```

Change to `let mut` and add after it:

```rust
// Set closed flag atomically BEFORE firing callbacks.
// Only the worker that sets this flag proceeds with callbacks.
// enqueue_into() checks this flag to reject dynamic adds after completion.
let did_close = if fire_complete {
    let closed_key = format!("jobs:batch:{}:closed", batch_id);
    let we_closed = kv::kv_set_nx(
        kv_handle,
        &closed_key,
        &Value::Bool(true),
        batch_ttl,
    )
    .unwrap_or(false);
    if !we_closed {
        // Another worker already closed — skip callbacks but continue
        // to the metadata update so batch_status() shows correct state.
        fire_complete = false;
        fire_death = false;
        fire_success = false;
    }
    we_closed
} else {
    false
};
```

Then update `fire_complete` references — the existing `fire_complete` condition already gates the callback logic, and the `did_close` early-return handles the race. No further changes needed to the callback firing code since it already checks `fire_complete`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test --lib jobs::tests::test_closed_flag_set_on_completion -- --exact 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Write test `test_only_one_worker_fires_callbacks`**

```rust
#[test]
fn test_only_one_worker_fires_callbacks() {
    with_temp_kv("ntnt_one_worker_callbacks_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("one-worker-test".to_string())]).unwrap();
        // Two jobs in the batch
        for i in 0..2 {
            let mut payload = HashMap::new();
            payload.insert("x".to_string(), Value::Int(i));
            enqueue_fn(&[
                handle.clone(),
                Value::String("ProcessRow".to_string()),
                Value::Map(payload),
            ])
            .unwrap();
        }
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Find both job IDs
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_ids: Vec<String> = data_keys
            .iter()
            .filter(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .collect();
        assert_eq!(job_ids.len(), 2);

        // Complete first job
        let job_data_1 = match kv::kv_get(kv, &format!("jobs:data:{}", job_ids[0])).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &job_data_1, &job_ids[0], "succeeded").unwrap();

        // Complete second job (this brings pending to 0, sets closed flag)
        let job_data_2 = match kv::kv_get(kv, &format!("jobs:data:{}", job_ids[1])).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &job_data_2, &job_ids[1], "succeeded").unwrap();

        // Only ONE callback job should be enqueued for on_complete
        let cb_keys = kv::kv_list(kv, Some(&format!("jobs:data:cb-{}", bid)))
            .unwrap_or_default();
        // Count on_complete callbacks
        let complete_cbs: Vec<&String> = cb_keys
            .iter()
            .filter(|k| k.contains("on_complete"))
            .collect();
        assert!(
            complete_cbs.len() <= 1,
            "at most one on_complete callback should be enqueued, got {}",
            complete_cbs.len()
        );
    });
}
```

- [ ] **Step 6: Run test**

Run: `cargo test --lib jobs::tests::test_only_one_worker_fires_callbacks -- --exact 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "feat(jobs): add closed flag to serialize batch completion vs dynamic adds"
```

---

## Task 7: Add TTLs to `seal()` and Completion

**Files:**
- Modify: `src/stdlib/jobs.rs:~4862-4941` (seal function)
- Modify: `src/stdlib/jobs.rs:~930-961` (update_batch_on_terminal metadata update section)
- Modify: `src/stdlib/kv.rs` (already done in Task 1)

- [ ] **Step 1: Write test `test_seal_sets_30d_ttl_on_metadata`**

```rust
#[test]
fn test_seal_sets_30d_ttl_on_metadata() {
    with_temp_kv("ntnt_seal_ttl_meta_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("ttl-meta-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };
        let meta_key = format!("jobs:batch:{}", bid);
        let ttl = kv::kv_ttl(kv, &meta_key).unwrap();
        // TTL should be roughly 30 days (±60 seconds for test execution time)
        let thirty_days = 30 * 24 * 3600;
        match ttl {
            Some(t) => assert!(
                t > thirty_days - 60 && t <= thirty_days,
                "metadata TTL should be ~30 days, got {} seconds",
                t
            ),
            None => panic!("metadata key should have a TTL after seal"),
        }
    });
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test --lib jobs::tests::test_seal_sets_30d_ttl_on_metadata -- --exact 2>&1 | tail -10`
Expected: FAIL — seal currently writes with `None` TTL.

- [ ] **Step 3: Add 30d TTL to `seal()` metadata and counter writes**

Define a constant at the top of the `seal` closure (inside the `kv_result` closure, after `let kv_handle = ...`):

```rust
let batch_ttl_30d: Option<i64> = Some(30 * 24 * 3600);
```

Then update every `kv::kv_set` call in the seal function to use `batch_ttl_30d` instead of `None`:

**Empty batch path** (~line 4873):
```rust
// Before:
kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), None)?;
// After:
let completion_ttl: Option<i64> = Some(24 * 3600);
kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), completion_ttl)?;
```

And for the counter keys in the empty batch path (~line 4876-4879):
```rust
// Before:
kv::kv_set(&kv_handle, &format!("{}:pending", cp), &Value::Int(0), None)?;
kv::kv_set(&kv_handle, &format!("{}:succeeded", cp), &Value::Int(0), None)?;
kv::kv_set(&kv_handle, &format!("{}:dead", cp), &Value::Int(0), None)?;
kv::kv_set(&kv_handle, &format!("{}:cancelled", cp), &Value::Int(0), None)?;
// After (add total too):
for suffix in &["pending", "succeeded", "dead", "cancelled", "total"] {
    kv::kv_set(&kv_handle, &format!("{}:{}", cp, suffix), &Value::Int(0), completion_ttl)?;
}
```

Note: empty batches use 24h TTL (`completion_ttl`) since they complete immediately at seal time.

**Non-empty "sealing" metadata** (~line 4916):
```rust
// Before:
kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), None)?;
// After:
kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), batch_ttl_30d)?;
```

**Non-empty counter init** (~line 4922-4925):
```rust
// Before:
kv::kv_set(&kv_handle, &format!("{}:pending", cp), &Value::Int(total), None)?;
kv::kv_set(&kv_handle, &format!("{}:succeeded", cp), &Value::Int(0), None)?;
kv::kv_set(&kv_handle, &format!("{}:dead", cp), &Value::Int(0), None)?;
kv::kv_set(&kv_handle, &format!("{}:cancelled", cp), &Value::Int(0), None)?;
// After:
kv::kv_set(&kv_handle, &format!("{}:pending", cp), &Value::Int(total), batch_ttl_30d)?;
kv::kv_set(&kv_handle, &format!("{}:succeeded", cp), &Value::Int(0), batch_ttl_30d)?;
kv::kv_set(&kv_handle, &format!("{}:dead", cp), &Value::Int(0), batch_ttl_30d)?;
kv::kv_set(&kv_handle, &format!("{}:cancelled", cp), &Value::Int(0), batch_ttl_30d)?;
kv::kv_set(&kv_handle, &format!("{}:total", cp), &Value::Int(total), batch_ttl_30d)?;
```

**Non-empty "sealed" metadata** (~line 4941):
```rust
// Before:
kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), None)?;
// After:
kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), batch_ttl_30d)?;
```

- [ ] **Step 4: Run TTL test**

Run: `cargo test --lib jobs::tests::test_seal_sets_30d_ttl_on_metadata -- --exact 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Write test `test_seal_sets_30d_ttl_on_counters`**

```rust
#[test]
fn test_seal_sets_30d_ttl_on_counters() {
    with_temp_kv("ntnt_seal_ttl_counters_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("ttl-counters-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };
        let cp = format!("jobs:batch:{}:counter", bid);
        let thirty_days = 30 * 24 * 3600;
        for suffix in &["pending", "succeeded", "dead", "cancelled", "total"] {
            let ttl = kv::kv_ttl(kv, &format!("{}:{}", cp, suffix)).unwrap();
            match ttl {
                Some(t) => assert!(
                    t > thirty_days - 60 && t <= thirty_days,
                    "counter:{} TTL should be ~30 days, got {}s",
                    suffix, t
                ),
                None => panic!("counter:{} should have TTL after seal", suffix),
            }
        }
    });
}
```

- [ ] **Step 6: Write test `test_completion_shortens_ttl_to_24h`**

```rust
#[test]
fn test_completion_shortens_ttl_to_24h() {
    with_temp_kv("ntnt_completion_ttl_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("completion-ttl-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Complete the single job
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_id = data_keys
            .iter()
            .find(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .expect("should find a job");
        let job_data = match kv::kv_get(kv, &format!("jobs:data:{}", job_id)).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &job_data, &job_id, "succeeded").unwrap();

        // TTLs should now be ~24h
        let meta_key = format!("jobs:batch:{}", bid);
        let twenty_four_hours = 24 * 3600;
        let meta_ttl = kv::kv_ttl(kv, &meta_key).unwrap();
        match meta_ttl {
            Some(t) => assert!(
                t > twenty_four_hours - 60 && t <= twenty_four_hours,
                "metadata TTL should be ~24h after completion, got {}s",
                t
            ),
            None => panic!("metadata should have TTL after completion"),
        }

        let cp = format!("jobs:batch:{}:counter", bid);
        for suffix in &["pending", "succeeded", "dead", "cancelled", "total"] {
            let ttl = kv::kv_ttl(kv, &format!("{}:{}", cp, suffix)).unwrap();
            match ttl {
                Some(t) => assert!(
                    t > twenty_four_hours - 60 && t <= twenty_four_hours,
                    "counter:{} TTL should be ~24h after completion, got {}s",
                    suffix, t
                ),
                None => panic!("counter:{} should have TTL after completion", suffix),
            }
        }
    });
}
```

- [ ] **Step 7: Write test `test_closed_flag_has_ttl`**

```rust
#[test]
fn test_closed_flag_has_ttl() {
    with_temp_kv("ntnt_closed_flag_ttl_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("closed-ttl-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_id = data_keys
            .iter()
            .find(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .expect("should find a job");
        let job_data = match kv::kv_get(kv, &format!("jobs:data:{}", job_id)).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &job_data, &job_id, "succeeded").unwrap();

        let closed_key = format!("jobs:batch:{}:closed", bid);
        let twenty_four_hours = 24 * 3600;
        let ttl = kv::kv_ttl(kv, &closed_key).unwrap();
        match ttl {
            Some(t) => assert!(
                t > twenty_four_hours - 60 && t <= twenty_four_hours,
                "closed flag TTL should be ~24h, got {}s",
                t
            ),
            None => panic!("closed flag should have TTL"),
        }
    });
}
```

- [ ] **Step 8: Write test `test_empty_batch_gets_24h_ttl`**

```rust
#[test]
fn test_empty_batch_gets_24h_ttl() {
    with_temp_kv("ntnt_empty_batch_ttl_test.db", |kv| {
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let seal_fn = get_fn(&module, "seal");

        let handle = batch_fn(&[Value::String("empty-ttl-test".to_string())]).unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };
        let meta_key = format!("jobs:batch:{}", bid);
        let twenty_four_hours = 24 * 3600;
        let ttl = kv::kv_ttl(kv, &meta_key).unwrap();
        match ttl {
            Some(t) => assert!(
                t > twenty_four_hours - 60 && t <= twenty_four_hours,
                "empty batch metadata TTL should be ~24h, got {}s",
                t
            ),
            None => panic!("empty batch metadata should have 24h TTL"),
        }
    });
}
```

- [ ] **Step 9: Add TTL shortening to `update_batch_on_terminal()`**

In the metadata update section at the end of `update_batch_on_terminal()` (~line 930-961), add TTL shortening after the `did_fire_complete` metadata update. Find:

```rust
let _ = kv::kv_set(kv_handle, &meta_key, &Value::Map(meta), None);
```

Replace `None` with the appropriate TTL. When `did_fire_complete` (or `did_close`), use completion TTL. Otherwise keep the existing TTL (just use `None` since it's already 30d from seal):

```rust
let meta_write_ttl = if did_close { Some(24 * 3600i64) } else { None };
let _ = kv::kv_set(kv_handle, &meta_key, &Value::Map(meta), meta_write_ttl);
```

Also add TTL shortening for counter keys and closed flag after the metadata write:

```rust
if did_close {
    let completion_ttl = 24 * 3600i64;
    // Shorten counter and closed flag TTLs to 24h
    for suffix in &["pending", "succeeded", "dead", "cancelled", "total"] {
        let _ = kv::kv_expire(kv_handle, &format!("{}:{}", counter_prefix, suffix), completion_ttl);
    }
    let closed_key = format!("jobs:batch:{}:closed", batch_id);
    let _ = kv::kv_expire(kv_handle, &closed_key, completion_ttl);
}
```

Note: `counter_prefix` is already declared earlier in the function (from Task 5). `did_close` is from Task 6.

- [ ] **Step 10: Run all TTL tests**

Run: `cargo test --lib jobs::tests::test_seal_sets_30d_ttl -- 2>&1 | tail -10`
Run: `cargo test --lib jobs::tests::test_completion_shortens_ttl -- 2>&1 | tail -10`
Run: `cargo test --lib jobs::tests::test_closed_flag_has_ttl -- 2>&1 | tail -10`
Run: `cargo test --lib jobs::tests::test_empty_batch_gets_24h_ttl -- 2>&1 | tail -10`
Expected: All PASS.

- [ ] **Step 11: Run all existing tests (regression check)**

Run: `cargo test --lib jobs::tests 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 12: Commit**

```bash
git add src/stdlib/jobs.rs src/stdlib/kv.rs
git commit -m "feat(jobs): add 30d TTL at seal, 24h TTL at batch completion"
```

---

## Task 8: Implement `enqueue_into()` and `enqueue_to_sealed_batch()`

**Files:**
- Modify: `src/stdlib/jobs.rs` (add helper function + stdlib registration)

- [ ] **Step 1: Write test `test_enqueue_into_rejects_invalid_batch_id`**

```rust
#[test]
fn test_enqueue_into_rejects_invalid_batch_id() {
    with_temp_kv("ntnt_enqueue_into_invalid_test.db", |_kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        let result = enqueue_into_fn(&[
            Value::String("nonexistent-batch-id".to_string()),
            Value::String("ProcessRow".to_string()),
            Value::Map(HashMap::new()),
        ]);
        assert!(result.is_err(), "enqueue_into with invalid batch_id must error");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("not found"),
            "error must mention 'not found', got: {}",
            err_msg
        );
    });
}
```

- [ ] **Step 2: Write test `test_enqueue_into_rejects_unknown_job_type`**

```rust
#[test]
fn test_enqueue_into_rejects_unknown_job_type() {
    with_temp_kv("ntnt_enqueue_into_unknown_type_test.db", |_kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        // Create and seal an empty batch (so it exists in KV)
        let handle = batch_fn(&[Value::String("unknown-type-test".to_string())]).unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Try to enqueue a non-existent job type
        let result = enqueue_into_fn(&[
            Value::String(bid),
            Value::String("NonExistentJob".to_string()),
            Value::Map(HashMap::new()),
        ]);
        assert!(
            result.is_err(),
            "enqueue_into with unknown job type must error"
        );
    });
}
```

- [ ] **Step 3: Run tests to verify they fail (function doesn't exist yet)**

Run: `cargo test --lib jobs::tests::test_enqueue_into -- 2>&1 | tail -10`
Expected: FAIL — `enqueue_into` not found in module.

- [ ] **Step 4: Implement `enqueue_to_sealed_batch()` helper**

Add this function near `fire_batch_callback` (around line 676), before `update_batch_on_terminal`:

```rust
/// Dynamically add a job to a sealed batch. Writes directly to KV and
/// atomically increments pending + total counters.
///
/// Returns `Ok(job_id)` on success, or an error if the batch doesn't exist,
/// is complete, or is closed.
fn enqueue_to_sealed_batch(
    batch_id: &str,
    job_name: &str,
    payload: Value,
) -> Result<String> {
    // Step 1: Validate job type is registered (before any counter mutation).
    let _job_def = JOB_RUNTIME.get_job(job_name)?.ok_or_else(|| {
        IntentError::runtime_error(format!(
            "Unknown job type '{}' — define it with a job block before enqueueing",
            job_name
        ))
    })?;

    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    let meta_key = format!("jobs:batch:{}", batch_id);

    // Step 2-4: Read metadata and validate status.
    let status = match kv::kv_get(&kv_handle, &meta_key)? {
        Value::Map(m) => match m.get("status") {
            Some(Value::String(s)) => s.clone(),
            _ => {
                return Err(IntentError::runtime_error(format!(
                    "batch '{}' has missing or invalid status",
                    batch_id
                )));
            }
        },
        _ => {
            return Err(IntentError::runtime_error(format!(
                "batch '{}' not found — it may have expired or the ID is invalid",
                batch_id
            )));
        }
    };

    if status == "complete" {
        return Err(IntentError::runtime_error(format!(
            "batch '{}' is complete — cannot add jobs after all jobs have reached terminal state",
            batch_id
        )));
    }

    // Step 5: Check closed flag.
    let closed_key = format!("jobs:batch:{}:closed", batch_id);
    if let Ok(Value::Bool(true)) = kv::kv_get(&kv_handle, &closed_key) {
        return Err(IntentError::runtime_error(format!(
            "batch '{}' is closing — a completion callback is in flight, cannot add jobs",
            batch_id
        )));
    }

    // Step 6: Only allow dynamic adds for sealing/sealed batches.
    if status != "sealing" && status != "sealed" {
        return Err(IntentError::runtime_error(format!(
            "batch '{}' has status '{}' — dynamic adds only allowed for sealing/sealed batches",
            batch_id, status
        )));
    }

    // Step 7: Increment counters BEFORE writing the job.
    let counter_prefix = format!("jobs:batch:{}:counter", batch_id);
    kv::kv_incr(&kv_handle, &format!("{}:pending", counter_prefix), 1).map_err(|e| {
        IntentError::runtime_error(format!(
            "batch '{}' pending counter increment failed: {}",
            batch_id, e
        ))
    })?;
    kv::kv_incr(&kv_handle, &format!("{}:total", counter_prefix), 1).map_err(|e| {
        // Roll back pending on total increment failure.
        let _ = kv::kv_incr(&kv_handle, &format!("{}:pending", counter_prefix), -1);
        IntentError::runtime_error(format!(
            "batch '{}' total counter increment failed: {}",
            batch_id, e
        ))
    })?;

    // Step 8: Write job to KV via enqueue_internal.
    let result = enqueue_internal(
        job_name,
        payload,
        &timestamp_key(),
        None,
        Some(batch_id),
        None,
    );

    match result {
        Ok(EnqueueResult::Created(job_id)) => Ok(job_id),
        Ok(EnqueueResult::Deduplicated(existing_id)) => {
            // Dedup collision — roll back both counters.
            let _ = kv::kv_incr(&kv_handle, &format!("{}:pending", counter_prefix), -1);
            let _ = kv::kv_incr(&kv_handle, &format!("{}:total", counter_prefix), -1);
            Ok(existing_id)
        }
        Err(e) => {
            // Enqueue failed — roll back both counters.
            let _ = kv::kv_incr(&kv_handle, &format!("{}:pending", counter_prefix), -1);
            let _ = kv::kv_incr(&kv_handle, &format!("{}:total", counter_prefix), -1);
            Err(e)
        }
    }
}
```

- [ ] **Step 5: Register `enqueue_into()` stdlib function**

Add this after the `batch_id()` registration (before the `_BatchCallback` registration, around line 5101):

```rust
    // @ntnt enqueue_into
    // @module std/jobs
    // @signature enqueue_into(batch_id: String, job_type: String, args: Map) -> Result<String, String>
    // Dynamically add a job to a sealed batch.
    //
    // Writes the job directly to KV and atomically increments the batch's
    // pending and total counters. Use this from within a batch job's perform
    // block to add more work to the same batch.
    // @param batch_id The batch ID string (from batch_id() or batch handle)
    // @param job_type The registered job type name
    // @param args The job payload map
    // @returns Result<String, String> — Ok(job_id) or Err(message)
    // @example enqueue_into(batch_id(), "ProcessChild", map { "id": child.id }) ~ "Add a child job to the current batch"
    // @see_also batch, batch_id, enqueue, seal
    module.insert(
        "enqueue_into".to_string(),
        Value::NativeFunction {
            name: "enqueue_into".to_string(),
            arity: 3,
            max_arity: 3,
            requires: Some(crate::interpreter::RuntimeCapability::JobEnqueue),
            func: |args| {
                if args.len() != 3 {
                    return Err(IntentError::type_error(
                        "enqueue_into() requires 3 arguments (batch_id, job_type, args)".to_string(),
                    ));
                }
                let batch_id = match &args[0] {
                    Value::String(s) => s.clone(),
                    Value::Map(m) => match m.get("_batch_id") {
                        Some(Value::String(bid)) => bid.clone(),
                        _ => {
                            return Err(IntentError::type_error(
                                "enqueue_into() first argument must be a batch ID string or handle"
                                    .to_string(),
                            ))
                        }
                    },
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_into() first argument must be a batch ID string or handle"
                                .to_string(),
                        ))
                    }
                };
                let job_type = match &args[1] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_into() second argument must be a job type string".to_string(),
                        ))
                    }
                };
                let payload = args[2].clone();

                match enqueue_to_sealed_batch(&batch_id, &job_type, payload) {
                    Ok(job_id) => Ok(Value::ok(Value::String(job_id))),
                    Err(e) => Err(e),
                }
            },
        },
    );
```

- [ ] **Step 6: Run the two error-case tests**

Run: `cargo test --lib jobs::tests::test_enqueue_into_rejects -- 2>&1 | tail -15`
Expected: Both PASS.

- [ ] **Step 7: Write test `test_enqueue_into_writes_job_and_increments_counters`**

```rust
#[test]
fn test_enqueue_into_writes_job_and_increments_counters() {
    with_temp_kv("ntnt_enqueue_into_counters_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        // Create batch with 1 job, seal it
        let handle = batch_fn(&[Value::String("dynamic-add-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Verify initial counters: total=1, pending=1
        let cp = format!("jobs:batch:{}:counter", bid);
        assert!(matches!(kv::kv_get(kv, &format!("{}:total", cp)).unwrap(), Value::Int(1)));
        assert!(matches!(kv::kv_get(kv, &format!("{}:pending", cp)).unwrap(), Value::Int(1)));

        // Dynamically add a second job
        let mut payload2 = HashMap::new();
        payload2.insert("x".to_string(), Value::Int(2));
        let result = enqueue_into_fn(&[
            Value::String(bid.clone()),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload2),
        ])
        .unwrap();
        assert!(
            matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
            "enqueue_into must return Ok"
        );

        // Counters must be incremented: total=2, pending=2
        assert!(
            matches!(kv::kv_get(kv, &format!("{}:total", cp)).unwrap(), Value::Int(2)),
            "total should be 2 after dynamic add"
        );
        assert!(
            matches!(kv::kv_get(kv, &format!("{}:pending", cp)).unwrap(), Value::Int(2)),
            "pending should be 2 after dynamic add"
        );

        // The new job should exist in KV with batch_id
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        assert_eq!(data_keys.len(), 2, "should have 2 jobs in KV");
    });
}
```

- [ ] **Step 8: Write test `test_enqueue_into_returns_job_id`**

```rust
#[test]
fn test_enqueue_into_returns_job_id() {
    with_temp_kv("ntnt_enqueue_into_returns_id_test.db", |_kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        let handle = batch_fn(&[Value::String("returns-id-test".to_string())]).unwrap();
        let enqueue_fn = get_fn(&module, "enqueue");
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        let result = enqueue_into_fn(&[
            Value::String(bid),
            Value::String("ProcessRow".to_string()),
            Value::Map(HashMap::new()),
        ])
        .unwrap();

        // Should be Ok(String) where the string is a UUID
        match result {
            Value::EnumValue {
                ref variant,
                ref value,
                ..
            } => {
                assert_eq!(variant, "Ok");
                assert!(
                    matches!(value.as_ref(), Value::String(_)),
                    "enqueue_into must return Ok(String(job_id))"
                );
            }
            _ => panic!("expected EnumValue Ok"),
        }
    });
}
```

- [ ] **Step 9: Write test `test_enqueue_into_rejects_complete_batch`**

```rust
#[test]
fn test_enqueue_into_rejects_complete_batch() {
    with_temp_kv("ntnt_enqueue_into_complete_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        let handle = batch_fn(&[Value::String("complete-reject-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Complete the job to move batch to "complete"
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_id = data_keys
            .iter()
            .find(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .expect("should find a job");
        let job_data = match kv::kv_get(kv, &format!("jobs:data:{}", job_id)).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &job_data, &job_id, "succeeded").unwrap();

        // Now try to add — should fail
        let result = enqueue_into_fn(&[
            Value::String(bid),
            Value::String("ProcessRow".to_string()),
            Value::Map(HashMap::new()),
        ]);
        assert!(result.is_err(), "enqueue_into to complete batch must error");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("complete") || err_msg.contains("closing"),
            "error should mention complete or closing, got: {}",
            err_msg
        );
    });
}
```

- [ ] **Step 10: Write test `test_enqueue_into_rejects_closed_batch`**

```rust
#[test]
fn test_enqueue_into_rejects_closed_batch() {
    with_temp_kv("ntnt_enqueue_into_closed_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_fn = get_fn(&module, "enqueue");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        let handle = batch_fn(&[Value::String("closed-reject-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Manually set the closed flag (simulating a worker completing the batch)
        let closed_key = format!("jobs:batch:{}:closed", bid);
        kv::kv_set(kv, &closed_key, &Value::Bool(true), None).unwrap();

        // Try to add — should fail with "closing" error
        let result = enqueue_into_fn(&[
            Value::String(bid),
            Value::String("ProcessRow".to_string()),
            Value::Map(HashMap::new()),
        ]);
        assert!(result.is_err(), "enqueue_into to closed batch must error");
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("closing"),
            "error should mention 'closing', got: {}",
            err_msg
        );
    });
}
```

- [ ] **Step 11: Write test `test_enqueue_into_allowed_during_sealing`**

This tests that `enqueue_into()` works when the batch status is `"sealing"` (the transient state during seal). We simulate this by writing batch metadata directly to KV with status `"sealing"`.

```rust
#[test]
fn test_enqueue_into_allowed_during_sealing() {
    with_temp_kv("ntnt_enqueue_into_sealing_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        // Manually write batch metadata with "sealing" status (simulating mid-seal)
        let bid = "test-sealing-batch";
        let meta = build_batch_meta(bid, "sealing-test", "0", "sealing", 1, 1);
        kv::kv_set(kv, &format!("jobs:batch:{}", bid), &Value::Map(meta), None).unwrap();
        // Initialize counters
        let cp = format!("jobs:batch:{}:counter", bid);
        kv::kv_set(kv, &format!("{}:pending", cp), &Value::Int(1), None).unwrap();
        kv::kv_set(kv, &format!("{}:total", cp), &Value::Int(1), None).unwrap();
        kv::kv_set(kv, &format!("{}:succeeded", cp), &Value::Int(0), None).unwrap();
        kv::kv_set(kv, &format!("{}:dead", cp), &Value::Int(0), None).unwrap();
        kv::kv_set(kv, &format!("{}:cancelled", cp), &Value::Int(0), None).unwrap();

        let result = enqueue_into_fn(&[
            Value::String(bid.to_string()),
            Value::String("ProcessRow".to_string()),
            Value::Map(HashMap::new()),
        ]);
        assert!(
            result.is_ok(),
            "enqueue_into during sealing should succeed, got: {:?}",
            result.err()
        );
    });
}
```

- [ ] **Step 12: Write test `test_enqueue_into_allowed_when_sealed`**

```rust
#[test]
fn test_enqueue_into_allowed_when_sealed() {
    with_temp_kv("ntnt_enqueue_into_sealed_test.db", |_kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        let handle = batch_fn(&[Value::String("sealed-add-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        let result = enqueue_into_fn(&[
            Value::String(bid),
            Value::String("ProcessRow".to_string()),
            Value::Map(HashMap::new()),
        ]);
        assert!(
            result.is_ok(),
            "enqueue_into on sealed batch should succeed, got: {:?}",
            result.err()
        );
    });
}
```

- [ ] **Step 12: Run all enqueue_into tests**

Run: `cargo test --lib jobs::tests::test_enqueue_into -- 2>&1 | tail -20`
Expected: All PASS.

- [ ] **Step 13: Run full test suite**

Run: `cargo test --lib jobs::tests 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 14: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "feat(jobs): implement enqueue_into() for dynamic batch job addition"
```

---

## Task 9: Dedup Rollback Test

**Files:**
- Modify: `src/stdlib/jobs.rs` (add test)

- [ ] **Step 1: Write test `test_enqueue_into_dedup_rolls_back_counters`**

```rust
#[test]
fn test_enqueue_into_dedup_rolls_back_counters() {
    with_temp_kv("ntnt_enqueue_into_dedup_test.db", |kv| {
        // Register a job with unique option
        let mut opts = HashMap::new();
        opts.insert("unique".to_string(), JobOptionValue::Int(3600));
        JOB_RUNTIME
            .register_job(test_job_def_with_opts("UniqueJob", "default", opts))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        let handle = batch_fn(&[Value::String("dedup-rollback-test".to_string())]).unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };
        let cp = format!("jobs:batch:{}:counter", bid);

        // First enqueue — should succeed (empty batch was completed, so re-create it)
        // Actually, empty batch is already complete. We need a non-empty sealed batch.
        // Let's use a different approach: create batch with a non-unique job, seal, then
        // try to enqueue_into with two identical unique jobs.

        // Re-register non-unique job too
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();

        // Create a new batch with one ProcessRow job
        let handle2 = batch_fn(&[Value::String("dedup-rollback-test2".to_string())]).unwrap();
        let enqueue_fn = get_fn(&module, "enqueue");
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle2.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle2.clone()]).unwrap();

        let bid2 = match &handle2 {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };
        let cp2 = format!("jobs:batch:{}:counter", bid2);

        // Initial: total=1, pending=1
        assert!(matches!(kv::kv_get(kv, &format!("{}:total", cp2)).unwrap(), Value::Int(1)));
        assert!(matches!(kv::kv_get(kv, &format!("{}:pending", cp2)).unwrap(), Value::Int(1)));

        // First unique job add — should succeed, counters go to 2
        let mut unique_payload = HashMap::new();
        unique_payload.insert("key".to_string(), Value::String("same".to_string()));
        enqueue_into_fn(&[
            Value::String(bid2.clone()),
            Value::String("UniqueJob".to_string()),
            Value::Map(unique_payload.clone()),
        ])
        .unwrap();
        assert!(matches!(kv::kv_get(kv, &format!("{}:total", cp2)).unwrap(), Value::Int(2)));
        assert!(matches!(kv::kv_get(kv, &format!("{}:pending", cp2)).unwrap(), Value::Int(2)));

        // Second identical unique job — should dedup, counters should stay at 2
        enqueue_into_fn(&[
            Value::String(bid2.clone()),
            Value::String("UniqueJob".to_string()),
            Value::Map(unique_payload),
        ])
        .unwrap();
        let total_after = kv::kv_get(kv, &format!("{}:total", cp2)).unwrap();
        let pending_after = kv::kv_get(kv, &format!("{}:pending", cp2)).unwrap();
        assert!(
            matches!(total_after, Value::Int(2)),
            "total should still be 2 after dedup, got {:?}",
            total_after
        );
        assert!(
            matches!(pending_after, Value::Int(2)),
            "pending should still be 2 after dedup, got {:?}",
            pending_after
        );
    });
}
```

- [ ] **Step 2: Run test**

Run: `cargo test --lib jobs::tests::test_enqueue_into_dedup_rolls_back_counters -- --exact 2>&1 | tail -10`
Expected: PASS (the dedup rollback logic was already implemented in Task 8).

- [ ] **Step 3: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "test(jobs): add dedup rollback test for enqueue_into"
```

---

## Task 10: Integration Test — Dynamic Add Through Completion With Callbacks

**Files:**
- Modify: `src/stdlib/jobs.rs` (add tests)

- [ ] **Step 1: Write test `test_dynamic_add_then_complete_fires_callbacks`**

```rust
#[test]
fn test_dynamic_add_then_complete_fires_callbacks() {
    with_temp_kv("ntnt_dynamic_add_callbacks_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        // Create batch with 1 job
        let handle = batch_fn(&[Value::String("dynamic-callback-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Dynamically add a second job
        let mut payload2 = HashMap::new();
        payload2.insert("x".to_string(), Value::Int(2));
        enqueue_into_fn(&[
            Value::String(bid.clone()),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload2),
        ])
        .unwrap();

        // Now total=2, pending=2. Complete both jobs.
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_ids: Vec<String> = data_keys
            .iter()
            .filter(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .collect();
        assert_eq!(job_ids.len(), 2, "should have 2 jobs");

        // Complete first job
        let jd1 = match kv::kv_get(kv, &format!("jobs:data:{}", job_ids[0])).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &jd1, &job_ids[0], "succeeded").unwrap();

        // Batch should NOT be complete yet (pending=1)
        let cp = format!("jobs:batch:{}:counter", bid);
        assert!(
            matches!(kv::kv_get(kv, &format!("{}:pending", cp)).unwrap(), Value::Int(1)),
            "pending should be 1 after first completion"
        );

        // Complete second job
        let jd2 = match kv::kv_get(kv, &format!("jobs:data:{}", job_ids[1])).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &jd2, &job_ids[1], "succeeded").unwrap();

        // Batch should now be complete
        let meta = match kv::kv_get(kv, &format!("jobs:batch:{}", bid)).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected metadata map"),
        };
        assert!(
            matches!(meta.get("status"), Some(Value::String(s)) if s == "complete"),
            "batch status should be complete"
        );
        assert!(
            matches!(meta.get("fired_complete"), Some(Value::Bool(true))),
            "fired_complete should be true"
        );
        assert!(
            matches!(meta.get("fired_success"), Some(Value::Bool(true))),
            "fired_success should be true (all succeeded)"
        );

        // Verify total reflects dynamic add
        let total = kv::kv_get(kv, &format!("{}:total", cp)).unwrap();
        assert!(
            matches!(total, Value::Int(2)),
            "counter:total should be 2 (1 original + 1 dynamic), got {:?}",
            total
        );
    });
}
```

- [ ] **Step 2: Write test `test_on_success_uses_counter_total`**

```rust
#[test]
fn test_on_success_uses_counter_total() {
    with_temp_kv("ntnt_on_success_counter_total_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        // Seal with 1 job, dynamically add 1 more
        let handle = batch_fn(&[Value::String("success-counter-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        enqueue_into_fn(&[
            Value::String(bid.clone()),
            Value::String("ProcessRow".to_string()),
            Value::Map(HashMap::new()),
        ])
        .unwrap();

        // counter:total = 2, metadata total = 1 (stale)
        // Complete both jobs — on_success should fire because succeeded(2) == counter:total(2)
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_ids: Vec<String> = data_keys
            .iter()
            .filter(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .collect();

        for jid in &job_ids {
            let jd = match kv::kv_get(kv, &format!("jobs:data:{}", jid)).unwrap() {
                Value::Map(m) => m,
                _ => panic!("expected map"),
            };
            update_batch_on_terminal(kv, &jd, jid, "succeeded").unwrap();
        }

        let meta = match kv::kv_get(kv, &format!("jobs:batch:{}", bid)).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected metadata"),
        };
        assert!(
            matches!(meta.get("fired_success"), Some(Value::Bool(true))),
            "on_success should fire — succeeded(2) == counter:total(2), not metadata total(1)"
        );
    });
}
```

- [ ] **Step 3: Write test `test_total_counter_incremented_on_dynamic_add`**

```rust
#[test]
fn test_total_counter_incremented_on_dynamic_add() {
    with_temp_kv("ntnt_total_counter_dynamic_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let enqueue_into_fn = get_fn(&module, "enqueue_into");

        let handle = batch_fn(&[Value::String("total-dynamic-test".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle.clone()]).unwrap();

        let bid = match &handle {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };
        let cp = format!("jobs:batch:{}:counter", bid);

        // Add 3 more jobs dynamically
        for i in 0..3 {
            let mut p = HashMap::new();
            p.insert("y".to_string(), Value::Int(i));
            enqueue_into_fn(&[
                Value::String(bid.clone()),
                Value::String("ProcessRow".to_string()),
                Value::Map(p),
            ])
            .unwrap();
        }

        let total = kv::kv_get(kv, &format!("{}:total", cp)).unwrap();
        assert!(
            matches!(total, Value::Int(4)),
            "counter:total should be 4 (1 sealed + 3 dynamic), got {:?}",
            total
        );
    });
}
```

- [ ] **Step 4: Write test `test_nested_batch_via_callback`**

```rust
#[test]
fn test_nested_batch_via_callback() {
    with_temp_kv("ntnt_nested_batch_test.db", |kv| {
        JOB_RUNTIME
            .register_job(test_job_def("ProcessRow", "imports"))
            .unwrap();
        let module = init();
        let batch_fn = get_fn(&module, "batch");
        let enqueue_fn = get_fn(&module, "enqueue");
        let seal_fn = get_fn(&module, "seal");
        let status_fn = get_fn(&module, "batch_status");

        // Batch A: 1 job
        let handle_a = batch_fn(&[Value::String("parent-batch".to_string())]).unwrap();
        let mut payload = HashMap::new();
        payload.insert("x".to_string(), Value::Int(1));
        enqueue_fn(&[
            handle_a.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload),
        ])
        .unwrap();
        seal_fn(&[handle_a.clone()]).unwrap();

        let bid_a = match &handle_a {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Complete batch A
        let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
        let job_id_a = data_keys
            .iter()
            .find(|k| !k.contains("cb-"))
            .map(|k| k.strip_prefix("jobs:data:").unwrap().to_string())
            .expect("should find a job");
        let jd_a = match kv::kv_get(kv, &format!("jobs:data:{}", job_id_a)).unwrap() {
            Value::Map(m) => m,
            _ => panic!("expected map"),
        };
        update_batch_on_terminal(kv, &jd_a, &job_id_a, "succeeded").unwrap();

        // Batch A is now complete. Simulate callback creating batch B.
        let handle_b = batch_fn(&[Value::String("child-batch".to_string())]).unwrap();
        let mut payload_b = HashMap::new();
        payload_b.insert("y".to_string(), Value::Int(2));
        enqueue_fn(&[
            handle_b.clone(),
            Value::String("ProcessRow".to_string()),
            Value::Map(payload_b),
        ])
        .unwrap();
        seal_fn(&[handle_b.clone()]).unwrap();

        let bid_b = match &handle_b {
            Value::Map(m) => match m.get("_batch_id") {
                Some(Value::String(s)) => s.clone(),
                _ => panic!("no _batch_id"),
            },
            _ => panic!("not a map"),
        };

        // Batch B is independent — it has its own metadata and counters
        let status_b = status_fn(&[Value::String(bid_b.clone())]).unwrap();
        let status_map = match status_b {
            Value::EnumValue { ref value, .. } => match value.as_ref() {
                Value::Map(m) => m.clone(),
                _ => panic!("expected map"),
            },
            _ => panic!("expected Ok"),
        };
        assert!(
            matches!(status_map.get("status"), Some(Value::String(s)) if s == "sealed"),
            "child batch should be sealed"
        );
        assert!(
            matches!(status_map.get("total"), Some(Value::Int(1))),
            "child batch total should be 1"
        );

        // Batch A status is still complete
        let status_a = status_fn(&[Value::String(bid_a.clone())]).unwrap();
        let status_map_a = match status_a {
            Value::EnumValue { ref value, .. } => match value.as_ref() {
                Value::Map(m) => m.clone(),
                _ => panic!("expected map"),
            },
            _ => panic!("expected Ok"),
        };
        assert!(
            matches!(status_map_a.get("status"), Some(Value::String(s)) if s == "complete"),
            "parent batch should still be complete"
        );
    });
}
```

- [ ] **Step 5: Run all new tests**

Run: `cargo test --lib jobs::tests::test_dynamic_add_then_complete -- --exact 2>&1 | tail -10`
Run: `cargo test --lib jobs::tests::test_on_success_uses_counter_total -- --exact 2>&1 | tail -10`
Run: `cargo test --lib jobs::tests::test_total_counter_incremented -- --exact 2>&1 | tail -10`
Run: `cargo test --lib jobs::tests::test_nested_batch_via_callback -- --exact 2>&1 | tail -10`
Expected: All PASS.

- [ ] **Step 6: Run full test suite**

Run: `cargo test --lib jobs::tests 2>&1 | tail -20`
Expected: All tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/stdlib/jobs.rs
git commit -m "test(jobs): add integration tests for dynamic adds, callbacks, nested batches"
```

---

## Task 11: Update `@ntnt` Doc Blocks and Design Doc Checklist

**Files:**
- Modify: `src/stdlib/jobs.rs` (doc blocks)
- Modify: `design-docs/dd-052-job-system-enterprise-features.md` (checklist)

- [ ] **Step 1: Update `batch_id()` doc block**

The `@ntnt` comment block above `batch_id` (lines 5080-5091) should be updated. Change:

```
// Phase 1: always returns None. Phase 2 wires up thread-local job context.
```

to:

```
// Uses thread-local context set by the worker loop. Returns None when called
// outside a batch job's perform block.
```

- [ ] **Step 2: Add `@see_also enqueue_into` to `batch_id` doc block**

Change:
```
// @see_also batch, enqueue
```
to:
```
// @see_also batch, enqueue, enqueue_into
```

- [ ] **Step 3: Update DD-052 Phase 3 checklist**

In `design-docs/dd-052-job-system-enterprise-features.md`, update the Phase 3 checklist items from `[ ]` to `[x]`:

```markdown
**Phase 3: Dynamic additions + edge cases**
- [x] `batch_id()` available in perform block context
- [x] `enqueue_into(batch_id, job_type, args)` from within a batch job — increments pending atomically
- [x] Empty batch: seal with 0 jobs → immediate callbacks
- [x] Batch expiry: TTL on completed batches (24h), abandoned batches (30d)
- [x] Idempotent seal
- [x] Tests: dynamic job addition, empty batch, nested batches via callbacks
```

- [ ] **Step 4: Commit**

```bash
git add src/stdlib/jobs.rs design-docs/dd-052-job-system-enterprise-features.md
git commit -m "docs(jobs): update batch_id doc block and DD-052 Phase 3 checklist"
```

---

## Task 12: Final Verification

- [ ] **Step 1: Run full test suite**

Run: `cargo test --lib jobs::tests 2>&1 | tail -30`
Expected: All tests pass (original + 26 new).

- [ ] **Step 2: Run full project build**

Run: `cargo build --profile dev-release 2>&1 | tail -5`
Expected: Build succeeds with no warnings related to the changes.

- [ ] **Step 3: Lint check**

Run: `cargo clippy -- -D warnings 2>&1 | tail -20`
Expected: No new warnings.

- [ ] **Step 4: Count new tests**

Run: `cargo test --lib jobs::tests 2>&1 | grep "test result"`
Expected: Test count increased by ~26 from baseline.
