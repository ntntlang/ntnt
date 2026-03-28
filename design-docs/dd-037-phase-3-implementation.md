# DD-037 Phase 3: Job System — Advanced Features

**Status:** ✅ Complete (core: PRs #36, #38, #39. Priority/dedup: PR #41. Observability CLI: PR #35. 3 items remain open.)
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md)
**Created:** 2026-03-17
**Last Updated:** 2026-03-27
**Depends on:** Phase 2 ✅ (all 3 PRs merged)

---

## Overview

Phase 2 delivers a shippable job system: parsing, enqueue, workers, retry, testing mode, streaming logs, CLI, and documentation. Phase 3 adds production-hardening and advanced features deferred from Phase 2 reviews.

These items were identified during PR 2b/2c review as "not blocking shippable" and explicitly deferred.

---

## Items Deferred from Phase 2

### ~~Atomic Claim Audit — All Backends~~ — ✅ Shipped (PR #36)

Redis atomic claim via Lua `EVAL` script. SQLite already atomic. See [dd-037-phase-3-plan.md](dd-037-phase-3-plan.md).

### ~~Scheduled Job Claim Optimization~~ — ✅ Shipped (PR #36)

`ceiling` parameter on both backends. Workers skip future-scheduled jobs at KV layer. No more re-enqueue churn.

### ~~Deduplication~~ — ✅ Shipped (PR #38)

`unique: N` with SHA-256 hash dedup, TTL, live-job validation, cleanup on terminal states. Atomic dedup via `kv_set_nx` shipped in PR #41.

### ~~Job Expiration~~ — ✅ Shipped (PR #38)

`expires: N` — worker skips stale jobs, marks "expired".

### ~~Batch Enqueue~~ — ✅ Shipped (PR #39)

`enqueue_batch()` with upfront validation, FIFO ordering, 10K limit, item-indexed errors.

### ~~Priority Queues~~ — ✅ Shipped (PR #41, Phase 3b)

Named priorities (critical/high/normal/low), 0-99 numeric range, worker bands with independent thread pools, `kv_set_nx`. See [dd-037-priority-and-atomic-dedup-plan.md](dd-037-priority-and-atomic-dedup-plan.md).

### ~~`ntnt jobs list` CLI~~ — ✅ Shipped (PR #35, Phase 6)

`ntnt jobs list` with --status, --queue, --limit, --format filters. Plus inspect, retry, cancel, clear.

### Worker Heartbeat Refresh — 📋 Open

Periodically refresh `jobs:active:<id>` TTL during long-running jobs (e.g., every 30s). Currently TTL is set once on claim (300s), meaning jobs running longer than 5 minutes lose their visibility timeout protection.

- Spawn a background timer thread per job execution
- Refresh TTL every 30s until job completes
- Cancel timer on job completion/failure

**File:** `src/stdlib/jobs.rs` — `worker_loop()` execution section
**Priority:** Low — only matters for jobs running > 5 minutes

### Graceful Shutdown Drain Timeout — 📋 Open

Configurable drain timeout (default 30s) for `work_jobs()`. After timeout, in-flight jobs are abandoned and become re-claimable via visibility timeout expiry.

- `work_jobs(map { "drain_timeout": 30 })` option
- On Ctrl-C: stop claiming new jobs, wait up to drain_timeout for in-flight to complete

**File:** `src/stdlib/jobs.rs` — `work_jobs()` + `worker_loop()`
**Priority:** Low — Ctrl-C immediate stop is acceptable for most use cases

### `on_job_event(handler)` — User Event Hook — 📋 Open

`on_job_event(fn(e) { ... })` for custom job event handling. Deferred from Phase 2c because `Value::Function` (user closures) contains `Rc<RefCell<Environment>>` which is not `Send` — cannot be stored in the global `JOB_RUNTIME` for worker threads to call.

**Design options:**
1. **Capture bindings** — same pattern as `spawn()` in std/concurrent: extract `CapturedBindings` from the closure, store those (Send-safe), reconstruct in the worker thread. Requires `validate_and_capture()` to be made `pub(crate)`.
2. **Main-thread-only hook** — store handler in a `thread_local!` instead of `JOB_RUNTIME`. Only fires for events on the calling thread (works for `work_jobs()`, not `work_async()`).
3. **Channel-based** — worker threads send events through a channel, main thread dispatches to the handler. Cleanest separation of concerns.

**File:** `src/stdlib/jobs.rs`
**Priority:** Medium — stderr JSON logs provide basic observability; the hook adds programmatic integration

---

## Implementation Notes

- All core Phase 3 items are shipped (PRs #36, #38, #39)
- Priority queues and atomic dedup shipped in Phase 3b (PR #41)
- `ntnt jobs list` shipped in Phase 6 (PR #35)
- Remaining open items (heartbeat refresh, drain timeout, on_job_event) are on-demand — not blocking any phase
- These can be addressed based on user demand / production usage patterns
