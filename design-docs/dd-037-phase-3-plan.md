# DD-037 Phase 3: Implementation Plan

**Status:** ✅ Complete — All 3 core PRs merged. Priority/dedup shipped in PR #41. Atomic dedup shipped in PR #41. 3 Tier 3 items remain open.
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md) · [Phase 3 Design](dd-037-phase-3-implementation.md)
**Created:** 2026-03-18
**Last Updated:** 2026-03-27
**Depends on:** Phase 2 ✅, Phase 6 ✅

---

## Completed PRs

### PR A — Atomic Claim + Scheduled Optimization (#36, merged)

**What shipped:**
- [x] Redis `claim()` rewritten as atomic Lua `EVAL` script (KEYS+sort+GET+DEL in one operation)
- [x] SQLite `claim()` verified already atomic (`BEGIN IMMEDIATE`)
- [x] `ceiling` parameter on both backends — workers skip future-scheduled jobs at KV layer
- [x] Removed `scheduled_at` re-enqueue block from `worker_loop` (replaced by ceiling filter)
- [x] Runtime defense-in-depth: if ceiling filter bypassed, log + re-enqueue + sleep
- [x] Legacy type hint preserved in Lua script (returned as 3rd element)
- [x] UTF-8 errors propagated (not `unwrap_or_default`)
- [x] Unexpected Redis response types return `Err` (not `Ok(None)`)
- [x] `KEYS` performance trade-off documented accurately

**Review rounds:** 5 (Greptile + Copilot), all resolved

### PR B — Dedup + Expiration (#38, merged)

**What shipped:**
- [x] `unique: N` job option — SHA-256 hash dedup with TTL
- [x] `expires: N` job option — worker skips stale jobs, marks "expired"
- [x] Dedup validates existing job is still live (not cancelled/dead/expired/failed)
- [x] Dedup key stored in job data for O(1) cleanup
- [x] Dedup key cleaned up on: cancel, dead, expired, bulk delete
- [x] Dedup write failure emits `job.dedup_warning` event
- [x] Hash serialization failure propagated (not `unwrap_or_default`)
- [x] Hash determinism documented (serde_json BTreeMap sorts keys)
- [x] Race condition documented (best-effort, like Sidekiq)
- [x] `expired` status recognized in JobStatusCounts, force-cancel, CLI
- [x] `failed` legacy status included in terminal check

**Review rounds:** 2 (Greptile + Copilot), all resolved. 5 tests total.

---

## PR C — Batch Enqueue (#39, merged)

**What shipped:**
- [x] `enqueue_batch(job_name, args_array)` — enqueue N jobs in one call
- [x] Validates job name exists in registry once (fast-fail before any writes)
- [x] Validates all items are maps upfront before any KV writes
- [x] Calls `enqueue_internal` per item — reuses all existing logic (dedup, test mode, events)
- [x] FIFO ordering via base timestamp + per-item offset (wall-clock too coarse for tight loops)
- [x] MAX_BATCH_SIZE (10,000) guard against pathological batches
- [x] Error messages include item index (`enqueue_batch: item 3 failed: ...`)
- [x] Dedup behavior documented and tested (identical payloads → same job ID)
- [x] Empty array returns `Ok([])` immediately
- [x] Partial-success documented (no rollback — matches loop semantics)
- [x] Typechecker signature registered
- [x] Full `@ntnt` doc block with `@error` directives
- [x] STDLIB_REFERENCE.md updated (382 → 383 functions)

**Tests:** 6 new tests (basic, empty, unregistered, bad type, test mode, dedup)
**Review rounds:** 5 (Greptile), all resolved. Issues found and fixed: N+1 comment accuracy, FIFO timestamp collision, incomplete uniqueness assertion, undocumented dedup behavior, no batch size limit, dead error path with lost item index.

---

## Remaining Tier 3 (on demand, not planned)

Items shipped in later PRs are marked. Remaining items are independently shippable and can wait for real usage patterns.

### ~~Priority Queues (`priority: N`)~~ — ✅ Shipped in Phase 3b (PR #41)

Named priorities (critical/high/normal/low), 0-99 numeric range, worker bands with independent thread pools, band validation. See [dd-037-priority-and-atomic-dedup-plan.md](dd-037-priority-and-atomic-dedup-plan.md).

### Worker Heartbeat Refresh — 📋 Open
**Why:** Jobs running >5 minutes lose visibility timeout protection. Only matters for long-running jobs.

- [ ] `worker_loop()`: spawn a timer thread per job execution that refreshes `jobs:active:<id>` TTL every 30s
- [ ] Cancel timer when job completes/fails
- [ ] Configurable refresh interval: `work_async(map { "heartbeat_interval": 30 })`

**Effort:** ~0.5 day

### Graceful Shutdown Drain Timeout — 📋 Open
**Why:** Currently Ctrl-C immediately stops workers. With drain timeout, in-flight jobs finish before shutdown.

- [ ] `work_jobs(map { "drain_timeout": 30 })` option
- [ ] On cancellation signal: stop claiming, wait up to N seconds for in-flight
- [ ] After timeout: exit anyway (jobs become re-claimable via visibility timeout)

**Effort:** ~0.5 day

### `on_job_event` User Hook — 📋 Open
**Why:** Programmatic integration — trigger custom logic on job lifecycle events. Currently only stderr JSON.

**Recommended approach:** Channel-based (cleanest separation of concerns)
- [ ] Worker threads send `JobEvent` structs through a `crossbeam::channel`
- [ ] Main thread runs a dispatcher that calls the user's handler function
- [ ] Workers never touch user closures (no Send problem)

See [dd-037-phase-3-implementation.md](dd-037-phase-3-implementation.md) for 3 design options.

**Effort:** ~1 day

### ~~Atomic Dedup (SET NX)~~ — ✅ Shipped in Phase 3b (PR #41)

`kv_set_nx` added to both backends. Used in dedup path to close the race window. See [dd-037-priority-and-atomic-dedup-plan.md](dd-037-priority-and-atomic-dedup-plan.md).

### Redis SCAN in Lua — 📋 Open
**Why:** `KEYS` scans the entire Redis keyspace (O(total keys)). For Redis instances with millions of non-job keys, this blocks the event loop.

- [ ] Replace `KEYS` with cursor-based `SCAN` inside the Lua script
- [ ] Or switch to a Redis Sorted Set for the pending queue (O(log N) claim)

**Effort:** ~0.5 day

---

## What Phase 3 Delivered

All three PRs merged. Phase 3 is complete:

- ✅ Atomic multi-worker job claiming (Redis Lua EVAL + SQLite BEGIN IMMEDIATE)
- ✅ Efficient scheduled job polling (ceiling parameter, no KV churn)
- ✅ Deduplication (`unique: N` — SHA-256 hash with TTL, live-job validation)
- ✅ Job expiration (`expires: N` — worker skips stale jobs)
- ✅ Batch enqueue (`enqueue_batch` — bulk operations with upfront validation)

Combined with Phases 0-2 and 6, this is a complete production-ready job system matching the core feature set of Sidekiq, Bull, and Celery.
