# DD-037 Phase 3: Job System — Advanced Features

**Status:** Planning
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md)
**Created:** 2026-03-17
**Depends on:** Phase 2 (PR 2a ✅, PR 2b ✅, PR 2c 🔄)

---

## Overview

Phase 2 delivers a shippable job system: parsing, enqueue, workers, retry, testing mode, streaming logs, CLI, and documentation. Phase 3 adds production-hardening and advanced features deferred from Phase 2 reviews.

These items were identified during PR 2b/2c review as "not blocking shippable" and explicitly deferred.

---

## Items Deferred from Phase 2

### Redis Atomic Claim via Lua Script

Current Redis `claim()` uses SCAN+GET+DEL which is not atomic under concurrent workers — two workers can claim the same job. Replace with a Lua script (`EVAL`) that atomically finds and deletes the first matching key.

```lua
-- Atomic claim: find first key matching pattern, GET+DEL in one operation
local keys = redis.call('SCAN', 0, 'MATCH', ARGV[1], 'COUNT', 100)
-- ... sort, GET first, DEL, return
```

**File:** `src/stdlib/kv.rs` — `RedisKV::claim()`
**Priority:** High — needed before anyone runs multi-worker Redis in production

### Scheduled Job Claim Optimization

Worker currently claims all pending keys including future-scheduled ones, then re-enqueues if not ready (KV churn on every poll). Use timestamp-prefix-aware claiming:

- **SQLite:** Add `WHERE key < ?` bound to the claim query using current timestamp prefix
- **Redis:** Filter keys client-side before claiming based on timestamp portion

**File:** `src/stdlib/kv.rs` + `src/stdlib/jobs.rs` worker_loop
**Priority:** Medium — only impacts queues with many scheduled-future jobs

### Deduplication

`unique: 3600` job option — skip enqueue if identical job (same type + SHA256 of payload args) was enqueued within N seconds.

- Dedup key in KV: `jobs:unique:<type>:<sha256>` with TTL = unique duration
- Check on enqueue: if dedup key exists, skip enqueue, return existing job ID
- Set dedup key on successful enqueue

**File:** `src/stdlib/jobs.rs` — `enqueue_internal()`
**Priority:** Medium — prevents duplicate work from retry storms or double-clicks

### Job Expiration

`expires: 300` job option — discard jobs that have been pending for longer than N seconds.

- Worker checks `created_at + expires < now` before execution
- Expired jobs: status → "expired", no execution, no retry

**File:** `src/stdlib/jobs.rs` — `worker_loop()`
**Priority:** Low — most jobs should be processed quickly

### Batch Enqueue

`enqueue_batch(job_name, array_of_args)` — enqueue N jobs in one call.

- Single KV round-trip where possible (SQLite: single transaction wrapping N inserts)
- Returns array of job IDs

**File:** `src/stdlib/jobs.rs` — new function
**Priority:** Low — convenience, not blocking

### Priority Queues

Job-level priority: `job X on q (priority: 1) { ... }` (lower = higher priority).

- Priority encoded in pending key: `jobs:pending:<priority>:<timestamp>:<id>`
- Lexicographic ordering gives priority-first, then FIFO within priority
- Requires key layout migration from current `jobs:pending:<timestamp>:<id>`

**File:** `src/stdlib/jobs.rs` — `enqueue_internal()` + `worker_loop()`
**Priority:** Low — most job systems don't need priority until scale

### Worker Heartbeat Refresh

Periodically refresh `jobs:active:<id>` TTL during long-running jobs (e.g., every 30s). Currently TTL is set once on claim (300s), meaning jobs running longer than 5 minutes lose their visibility timeout protection.

- Spawn a background timer thread per job execution
- Refresh TTL every 30s until job completes
- Cancel timer on job completion/failure

**File:** `src/stdlib/jobs.rs` — `worker_loop()` execution section
**Priority:** Low — only matters for jobs running > 5 minutes

### Graceful Shutdown Drain Timeout

Configurable drain timeout (default 30s) for `work_jobs()`. After timeout, in-flight jobs are abandoned and become re-claimable via visibility timeout expiry.

- `work_jobs(map { "drain_timeout": 30 })` option
- On Ctrl-C: stop claiming new jobs, wait up to drain_timeout for in-flight to complete

**File:** `src/stdlib/jobs.rs` — `work_jobs()` + `worker_loop()`
**Priority:** Low — Ctrl-C immediate stop is acceptable for most use cases

### `ntnt jobs list` CLI

`ntnt jobs list server.tnt --queue=emails --status=failed` — list jobs with filters.

- Load .tnt file to get KV config
- Query KV for jobs matching filters
- Display as table or JSON

**File:** `src/main.rs`
**Priority:** Low — `ntnt jobs status` (Phase 2c) covers the basic observability need

---

## Implementation Notes

- Each item is independently implementable as a single PR
- Priority ordering: Redis Lua claim > Scheduled claim optimization > Dedup > everything else
- No items here block Phase 2 from being shippable
- These can be addressed based on user demand / production usage patterns
