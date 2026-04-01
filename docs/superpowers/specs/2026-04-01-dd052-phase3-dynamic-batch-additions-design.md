# DD-052 Phase 3: Dynamic Batch Additions + Edge Cases

**Parent:** [DD-052](../../../design-docs/dd-052-job-system-enterprise-features.md) (Job System Enterprise Features)
**Status:** Approved
**Date:** 2026-04-01
**Revised:** 2026-04-01 (post-review — race hardening, API split, TTL corrections, dedup safety)

---

## Overview

Phase 3 completes the batch system by wiring `batch_id()` context into perform blocks, enabling dynamic job addition from within batch jobs, and adding batch expiry with TTL-based cleanup. Phases 1-2 (core lifecycle + worker integration) are already merged.

## Scope

| Item | Description |
|------|-------------|
| `batch_id()` context | Thread-local wiring so `batch_id()` returns the current job's batch ID inside a perform block |
| `enqueue_into()` | New function: `enqueue_into(batch_id_string, job_type, args)` — writes directly to KV + atomically increments counters for post-seal dynamic addition |
| Batch expiry | 30d TTL on sealed batches (NEW — not currently implemented), shortened to 24h on completion |
| `total` counter key | New atomic counter `counter:total` — replaces metadata-only `total` for correctness under dynamic adds |
| Batch closed flag | Atomic `jobs:batch:<bid>:closed` flag to serialize completion vs dynamic add |
| Tests | Full coverage: context wiring, dynamic adds, expiry, nested batches, race conditions, dedup interaction |

## Non-Scope

- Phase 4 (CLI + observability) — separate work
- Callback closure execution (Phase 2 already enqueues `_BatchCallback` jobs; closure deserialization is a separate concern)
- Redis Lua scripts (SQLite-only for now, matching existing implementation)

---

## Design

### 1. `batch_id()` Context Wiring

**Mechanism:** Thread-local `RefCell<Option<String>>` with RAII guard for cleanup.

```rust
thread_local! {
    static CURRENT_BATCH_ID: RefCell<Option<String>> = RefCell::new(None);
}

/// Guard that unconditionally clears CURRENT_BATCH_ID on drop.
/// Guarantees cleanup even on panic (panic=unwind) or early return.
struct BatchIdGuard;

impl Drop for BatchIdGuard {
    fn drop(&mut self) {
        CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = None);
    }
}
```

**Lifecycle:**
1. Worker loop reads `batch_id` from `job_data` before calling `execute_in_worker()`
2. Sets thread-local: `CURRENT_BATCH_ID.with(|c| *c.borrow_mut() = Some(bid.clone()))` — or `None` for non-batch jobs (explicitly clearing any stale value from a prior job on the same thread)
3. Creates `let _guard = BatchIdGuard;` — guard lives on the stack
4. `execute_in_worker()` runs the perform block — `batch_id()` reads the thread-local
5. Guard drops when scope exits (normal return, early return, or panic unwind), clearing the thread-local

**`batch_id()` implementation:**
- Inside a batch job: returns `Value::String(id)`
- Outside a batch job (or non-batch job): returns `Value::none()`
- No panic, no error — always safe to call

**`on_failure` context:** The batch ID guard remains active during `execute_on_failure_in_worker()` — `batch_id()` works inside `on_failure` handlers. The guard is created before `execute_in_worker()` and drops after the entire job lifecycle (perform + on_failure) completes.

**Why thread-local over interpreter scope injection:** Follows existing patterns in the codebase (e.g., `KV_HANDLE`). Doesn't pollute the user's variable namespace. Zero cost when not in a batch job.

**Why RAII guard over manual clear:** `execute_in_worker` already uses `catch_unwind` for panics, but the TLS set happens in the worker loop *before* that call. A guard ensures cleanup regardless of where the panic originates — including between TLS set and `execute_in_worker()` entry, or in `on_failure` execution.

### 2. Dynamic Addition via `enqueue_into()`

**New function** — separate from `enqueue()` to avoid dispatch ambiguity.

```
enqueue_into(batch_id: String, job_type: String, args: Map) -> Result<String>
```

**Why a separate function instead of 3-arg `enqueue()` overload:**
The existing 3-arg `enqueue(batch_handle, job_type, args)` accepts a `Map` with `_batch_id` for pre-seal buffering. Overloading the first argument on `String` vs `Map` creates a footgun: `enqueue("SomeString", "JobType", args)` could be a typo where the user meant 2-arg enqueue with a forgotten argument, but would silently be treated as "add to batch ID 'SomeString'." A dedicated `enqueue_into()` makes intent explicit and errors obvious.

**`enqueue_into(batch_id, job_type, args)` steps:**

1. **Validate job type is registered** — `JOB_RUNTIME.get_job(&job_type)?`. Reject before any counter mutation. Error: `"Unknown job type '<type>' — define it with a job block before enqueueing"`
2. **Read batch metadata** from KV (`jobs:batch:<bid>`)
3. **Reject** if metadata doesn't exist. Error: `"batch '<bid>' not found — it may have expired or the ID is invalid"`
4. **Reject** if `status == "complete"`. Error: `"batch '<bid>' is complete — cannot add jobs after all jobs have reached terminal state"`
5. **Check closed flag** — `kv_get(jobs:batch:<bid>:closed)`. If exists, reject. Error: `"batch '<bid>' is closing — a completion callback is in flight, cannot add jobs"`. (See §2.1 for closed flag semantics.)
6. **Allowed statuses for dynamic add:** `"sealing"` and `"sealed"`. Jobs can begin executing while `seal()` is still flushing (status transitions sealing → sealed). A dynamic add during sealing is valid — the job being executed was already written to KV by seal.
7. **Increment counters** — `kv_incr(counter:pending, 1)` then `kv_incr(counter:total, 1)`
8. **Write job to KV** via `enqueue_internal()` with `batch_id` in job data. If `enqueue_internal` indicates a dedup collision (unique job already exists — see §2.2), **roll back counters**: `kv_incr(counter:pending, -1)` and `kv_incr(counter:total, -1)`. Return the existing job ID.
9. **Return** the job ID (same as 2-arg `enqueue()` behavior)

**Counter ordering rationale:** Counters incremented before job write. If crash after counter increment but before job write: `pending` is incremented but no job will complete to decrement it. The batch becomes stale and expires via TTL. This is the safer ordering — `pending > 0` correctly signals "still waiting" rather than a ghost job completing and triggering premature callbacks. Same tradeoff as `seal()`.

#### 2.1 Batch Closed Flag

**Problem:** Without serialization, a dynamic add can race with batch completion. Worker A brings `pending` to 0, fires `on_complete`. Concurrently, Worker B increments `pending` to 1 via `enqueue_into()`. The dynamically-added job runs to completion, decrementing `pending` back to 0, but `on_complete` has already fired. The `fired_complete` flag prevents a *second* firing — but the first firing's counts are wrong, and the dynamic job never participates in callbacks. This is a semantic violation, not just a timing issue.

**Solution:** Atomic closed flag set before callbacks fire.

In `update_batch_on_terminal()`, when `new_pending == 0` (before firing any callbacks):

```rust
let closed_key = format!("jobs:batch:{}:closed", batch_id);
let we_closed = kv::kv_set_nx(&kv_handle, &closed_key, &Value::Bool(true), batch_ttl)?;
if !we_closed {
    // Another worker already closed — skip callback firing.
    // This handles the rare double-zero race (two workers decrement pending
    // to 0 near-simultaneously due to kv_incr ordering).
    return Ok(());
}
```

`enqueue_into()` checks this flag at step 5 *before* incrementing counters. If the flag exists, the add is rejected with a clear error. No counter drift.

**Race window analysis:** Between `enqueue_into()`'s flag check (step 5) and counter increment (step 7), a worker *could* set the closed flag. In this case, the dynamic job's counters are incremented and the job is written, but `on_complete` has already fired. This is the same narrow window as the original spec, but now:
- The window is much smaller (just steps 5→7, not the entire status-check→increment span)
- The `fired_complete` flag still prevents duplicate callback firing
- The batch expires via TTL (24h after the erroneous completion, or 30d if the dynamic job fails)
- The dynamic job runs and completes normally — it just doesn't trigger callbacks

This residual race is acceptable without KV transactions. Fully eliminating it would require atomic check-and-increment (Lua scripts for Redis, or SQLite transactions), which is out of scope for Phase 3.

#### 2.2 Dedup Interaction

If `enqueue_internal()` encounters a dedup collision (unique job with matching payload already exists), the job is not written — only the existing job ID is returned. Without rollback, `pending` and `total` would drift upward permanently.

**Required change:** `enqueue_internal()` must signal whether a new job was created or an existing one was returned. Options:
- Return `(job_id, bool)` where `bool` = `true` if newly created
- Or return a tagged enum: `EnqueueResult::Created(job_id) | EnqueueResult::Deduplicated(job_id)`

`enqueue_into()` checks this signal and rolls back both counters on dedup. The counter rollback is two separate `kv_incr(..., -1)` calls — not atomic with the original increments, but counter drift from a crash between increment and rollback is bounded (stale batch expires via TTL, same as the counter-before-job tradeoff).

**Non-unique jobs:** When the dynamic job has no `unique` option, `enqueue_internal` always creates a new job. No rollback needed.

### 3. Atomic `total` Counter

**Problem:** `update_batch_on_terminal()` currently reads `total` from batch metadata (line ~735) to decide `on_success` (`succeeded == total`). Under dynamic adds, the metadata `total` is stale — it reflects the count at seal time, not after dynamic additions.

**Solution:** Add `counter:total` as an atomic KV key, alongside `counter:pending`, `counter:succeeded`, etc.

**Changes:**

| Location | Change |
|----------|--------|
| `seal()` (~line 4920) | Initialize `counter:total` alongside other counters: `kv_set(&format!("{}:total", cp), &Value::Int(total), ttl)` |
| `seal()` empty batch (~line 4867) | Initialize `counter:total` at 0 |
| `enqueue_into()` | Increment `counter:total` via `kv_incr` (step 7) |
| `update_batch_on_terminal()` (~line 735) | Read `total` from `kv_get(counter:total)` instead of metadata map |
| `batch_status()` (~line 5065) | Merge `counter:total` into response map (same pattern as other counters) |
| Expiry (§4) | Include `counter:total` in TTL updates |

Metadata `total` field is retained for human-readable context (e.g., "batch was sealed with N jobs") but is no longer authoritative for callback decisions.

### 4. Batch Expiry + TTL Corrections

**Current state:** `seal()` writes metadata and counters with `None` TTL (no expiry). The done-set keys use `Some(30 * 24 * 3600)`. This means sealed batches that are never completed **live forever** — the spec's claim that "abandoned batches expire after 30d" is incorrect today.

**Phase 3 must fix this.** TTL changes are new work, not relying on existing behavior.

#### Seal-time TTLs (30 days)

When `seal()` writes metadata and counter keys, use `Some(30 * 24 * 3600)` TTL:

```rust
let batch_ttl = Some(30 * 24 * 3600); // 30 days

// Metadata
kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), batch_ttl)?;

// Counters (pending, succeeded, dead, cancelled, total)
for suffix in &["pending", "succeeded", "dead", "cancelled", "total"] {
    kv::kv_set(&kv_handle, &format!("{}:{}", cp, suffix), &Value::Int(val), batch_ttl)?;
}
```

This applies to both the initial "sealing" write and the final "sealed" write. The second write refreshes the TTL (30d from seal completion, not from seal start).

Empty batches (total == 0) get 24h TTL immediately since they complete at seal time.

#### Completion TTLs (24 hours)

When `update_batch_on_terminal()` sets the `closed` flag (pending hits 0), use `kv_expire` to shorten TTLs without rewriting values:

```rust
let completion_ttl = 24 * 3600; // 24 hours

// Shorten metadata and all counter keys
kv::kv_expire(&kv_handle, &meta_key, completion_ttl)?;
for suffix in &["pending", "succeeded", "dead", "cancelled", "total"] {
    let _ = kv::kv_expire(&kv_handle, &format!("{}:{}", counter_prefix, suffix), completion_ttl);
}
// Closed flag gets same TTL
let _ = kv::kv_expire(&kv_handle, &closed_key, completion_ttl);
```

**Why `kv_expire` over `kv_set`:** `kv_expire` preserves the existing value and only updates the TTL. `kv_set` would require reading the value first, creating a read-modify-write race. `kv_expire` is already implemented (line 2304 of kv.rs).

**TTL update failure:** If `kv_expire` fails on a counter key, log a warning but don't fail the completion. The key falls back to its 30d TTL — suboptimal but not incorrect. Metadata TTL update failure *is* propagated since it affects `batch_status()` behavior.

#### Dynamic add TTL refresh

`enqueue_into()` does **not** refresh TTLs. The 30d window from seal is sufficient. A batch that takes >30 days to process has bigger problems.

#### `batch_status()` after expiry

Returns an error — metadata key doesn't exist in KV. This is correct behavior: the batch is gone.

**Done-set keys** (`jobs:batch:<bid>:done:<job_id>`) keep their existing 30d TTL. They naturally outlive the 24h metadata window and self-clean. Harmless orphans.

---

## Test Plan

All tests follow existing patterns: `with_temp_kv()` for setup, `TEST_LOCK` for serialization, direct KV reads to verify state.

### Group 1: `batch_id()` Context

| Test | Verifies |
|------|----------|
| `test_batch_id_returns_none_outside_job` | `batch_id()` returns `None` when called outside a perform block |
| `test_batch_id_returns_id_inside_batch_job` | Set thread-local, call `batch_id()`, returns correct batch ID |
| `test_batch_id_cleared_after_job_execution` | After `execute_in_worker` completes, `batch_id()` returns `None` (no leakage) |
| `test_batch_id_cleared_on_panic` | Perform block panics → `batch_id()` returns `None` on next call (guard cleanup) |
| `test_batch_id_none_for_non_batch_job` | Non-batch job runs on same thread after batch job → `batch_id()` returns `None` (no stale value) |
| `test_batch_id_available_in_on_failure` | `on_failure` handler can read `batch_id()` and gets the correct ID |

### Group 2: Dynamic Addition (`enqueue_into`)

| Test | Verifies |
|------|----------|
| `test_enqueue_into_writes_job_and_increments_counters` | `enqueue_into(bid, "JobType", args)` writes job to KV, increments `pending` and `total` counters |
| `test_enqueue_into_returns_job_id` | Return value is the new job's ID string |
| `test_enqueue_into_rejects_complete_batch` | Adding to a completed batch returns an error |
| `test_enqueue_into_rejects_invalid_batch_id` | Adding to a nonexistent batch ID returns an error |
| `test_enqueue_into_rejects_closed_batch` | Adding after `closed` flag is set returns an error |
| `test_enqueue_into_rejects_unknown_job_type` | Unknown job type returns error; counters unchanged |
| `test_enqueue_into_allowed_during_sealing` | Dynamic add succeeds when batch status is `"sealing"` |
| `test_enqueue_into_allowed_when_sealed` | Dynamic add succeeds when batch status is `"sealed"` |
| `test_enqueue_into_dedup_rolls_back_counters` | Unique job dedup collision → counters decremented back, existing job ID returned |
| `test_dynamic_add_then_complete_fires_callbacks` | Add dynamic job, complete all jobs (including dynamic), verify `on_success` and `on_complete` fire with updated `total` from counter |

### Group 3: Closed Flag + Race Hardening

| Test | Verifies |
|------|----------|
| `test_closed_flag_set_on_completion` | When `pending` hits 0, `jobs:batch:<bid>:closed` exists |
| `test_closed_flag_prevents_dynamic_add` | After closed flag set, `enqueue_into()` returns error; counters unchanged |
| `test_only_one_worker_fires_callbacks` | Simulate two workers decrementing pending to 0 — only one `kv_set_nx` on closed succeeds, only one fires callbacks |

### Group 4: Total Counter

| Test | Verifies |
|------|----------|
| `test_total_counter_initialized_at_seal` | `counter:total` equals number of buffered jobs after seal |
| `test_total_counter_incremented_on_dynamic_add` | `counter:total` reflects original + dynamic jobs |
| `test_on_success_uses_counter_total` | `on_success` fires when `succeeded == counter:total` (not metadata total) |
| `test_batch_status_includes_total_counter` | `batch_status()` response includes merged `total` from counter key |

### Group 5: Expiry

| Test | Verifies |
|------|----------|
| `test_seal_sets_30d_ttl_on_metadata` | After seal, metadata key has TTL ~30 days |
| `test_seal_sets_30d_ttl_on_counters` | After seal, all counter keys have TTL ~30 days |
| `test_completion_shortens_ttl_to_24h` | After batch completes, metadata and counter TTLs shortened to ~24h |
| `test_closed_flag_has_ttl` | Closed flag key has TTL matching completion TTL |
| `test_empty_batch_gets_24h_ttl` | Empty batch (0 jobs) sealed → metadata gets 24h TTL immediately |

### Group 6: Nested Batches

| Test | Verifies |
|------|----------|
| `test_nested_batch_via_callback` | Batch A completes → `on_success` creates batch B → seal B → B is independent and functional |

---

## Files Modified

| File | Changes |
|------|---------|
| `src/stdlib/jobs.rs` | Thread-local `CURRENT_BATCH_ID` + `BatchIdGuard`, `enqueue_into()` function, `enqueue_to_sealed_batch()` helper, `EnqueueResult` return type for `enqueue_internal`, closed flag in `update_batch_on_terminal()`, `counter:total` initialization in `seal()`, `total` read from counter in `update_batch_on_terminal()`, TTL additions to `seal()` and `update_batch_on_terminal()`, `batch_status()` total counter merge, tests |

---

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Counter increment without job write (crash window) | Counters incremented first; stale batch expires after 30d TTL. Same tradeoff as `seal()`. |
| Counter increment then dedup rollback (crash between increment and rollback) | Same TTL-based expiry. Counter drift is bounded — `pending` stays elevated, batch expires. No premature callbacks. |
| Thread-local leakage on panic | `BatchIdGuard` with `Drop` impl clears TLS on all exit paths (normal, early return, panic unwind). Explicit `None` set for non-batch jobs prevents stale values across sequential jobs on same thread. Note: `panic=abort` bypasses Drop — but that terminates the entire process, so TLS leakage is moot. |
| Dynamic add racing with completion | `closed` flag set atomically via `kv_set_nx` before callbacks fire. `enqueue_into()` checks flag before incrementing counters. Residual race window (between flag check and counter increment) is narrow and bounded: `fired_complete` prevents duplicate callbacks, batch expires via TTL. Full elimination requires KV transactions (out of scope). |
| `on_success` firing with stale `total` | `total` read from atomic `counter:total` key, not metadata. Dynamic adds increment this counter. `succeeded == counter_total` check is accurate. |
| TTL update failure on completion | `kv_expire` failure on counter keys logged as warning, falls back to 30d TTL. Metadata TTL failure propagated (affects `batch_status()` semantics). |
| `enqueue_into()` with wrong argument order | Separate function name makes intent explicit. Accidentally calling `enqueue("batch-id", "Type", args)` is a type error (3-arg enqueue requires Map first arg). `enqueue_into("Type", "batch-id", args)` would fail at step 2 (batch not found) with a clear error message. |
