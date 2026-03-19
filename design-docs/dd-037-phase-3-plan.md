# DD-037 Phase 3: Implementation Plan

**Status:** PR A ✅ merged (#36), PR B ✅ merged (#38), PR C 🔄 in progress
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md) · [Phase 3 Design](dd-037-phase-3-implementation.md)
**Created:** 2026-03-18
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

## PR C — Batch Enqueue (planned)

**Goal:** `enqueue_batch(job_name, args_array)` — enqueue N jobs in one call with fewer KV round-trips.

### What it does

```ntnt
import { enqueue_batch } from "std/jobs"

let ids = unwrap(enqueue_batch("SendEmail", [
    map { "to": "alice@example.com" },
    map { "to": "bob@example.com" },
    map { "to": "carol@example.com" },
]))
// ids = ["uuid-1", "uuid-2", "uuid-3"]
```

### Implementation

**`src/stdlib/jobs.rs`:**
- New `enqueue_batch(job_name, args_array)` function
- Validates job exists in registry once, then loops over args
- For each arg: generate UUID, build job data map, compute dedup key (if `unique` set)
- SQLite: wrap all KV writes in a single transaction (one `get_or_init_kv`, reuse handle)
- Redis: sequential writes (pipeline optimization is a future enhancement)
- Returns `Result<Array<String>, String>` — array of job IDs (deduped entries return existing ID)
- Respects test mode: writes to test queue if active
- `// @ntnt` doc block required (new stdlib function)

**`src/typechecker.rs`:**
- Add signature: `enqueue_batch(job_name: String, args: Array<Map>) -> Result<Array<String>, String>`

**Tests:**
- [ ] `enqueue_batch` with 3 items → 3 jobs created, 3 IDs returned
- [ ] `enqueue_batch` with empty array → returns `Ok([])`
- [ ] `enqueue_batch` in test mode → all items in test queue
- [ ] `enqueue_batch` with dedup → duplicates return existing IDs, new ones created
- [ ] `enqueue_batch` with unregistered job name → error

**Docs:**
- `// @ntnt` doc block on new function
- `ntnt docs --generate` to sync

**Effort:** ~0.5 day

### What it does NOT include
- No SQLite transaction wrapping (would require new KV operation — `kv_batch_set`)
- No Redis pipeline — sequential writes for now
- These optimizations can come later when profiling shows they matter

---

## Remaining Tier 3 (on demand, not planned)

Each item is independently shippable. Can wait for real usage patterns.

### Priority Queues (`priority: N`)
**Why:** Some jobs are more important than others. But most systems don't need this until scale.

- [ ] Change pending key format from `jobs:pending:<timestamp>:<id>` to `jobs:pending:<priority>:<timestamp>:<id>`
- [ ] Default priority: 5 (middle of 0-9 range, lower = higher priority)
- [ ] `enqueue_internal()`: include priority in pending key
- [ ] Backward compat: detect old-format keys (2 segments after `jobs:pending:`) and treat as priority 5
- [ ] Handle both key formats during transition period

**Effort:** ~1 day (mostly migration handling)

### Worker Heartbeat Refresh
**Why:** Jobs running >5 minutes lose visibility timeout protection. Only matters for long-running jobs.

- [ ] `worker_loop()`: spawn a timer thread per job execution that refreshes `jobs:active:<id>` TTL every 30s
- [ ] Cancel timer when job completes/fails
- [ ] Configurable refresh interval: `work_async(map { "heartbeat_interval": 30 })`

**Effort:** ~0.5 day

### Graceful Shutdown Drain Timeout
**Why:** Currently Ctrl-C immediately stops workers. With drain timeout, in-flight jobs finish before shutdown.

- [ ] `work_jobs(map { "drain_timeout": 30 })` option
- [ ] On cancellation signal: stop claiming, wait up to N seconds for in-flight
- [ ] After timeout: exit anyway (jobs become re-claimable via visibility timeout)

**Effort:** ~0.5 day

### `on_job_event` User Hook
**Why:** Programmatic integration — trigger custom logic on job lifecycle events. Currently only stderr JSON.

**Recommended approach:** Channel-based (cleanest separation of concerns)
- [ ] Worker threads send `JobEvent` structs through a `crossbeam::channel`
- [ ] Main thread runs a dispatcher that calls the user's handler function
- [ ] Workers never touch user closures (no Send problem)

See [dd-037-phase-3-implementation.md](dd-037-phase-3-implementation.md) for 3 design options.

**Effort:** ~1 day

### Atomic Dedup (SET NX)
**Why:** Current dedup is best-effort (kv_get then kv_set). Under high-concurrency identical enqueues, two callers can both miss and create separate jobs.

- [ ] Add `kv_set_nx` (set-if-not-exists) to `std/kv` — `SET NX EX` for Redis, `INSERT OR IGNORE` for SQLite
- [ ] Use in dedup check instead of separate get+set
- [ ] Returns whether the key was actually set (true = we won the race)

**Effort:** ~0.5 day

### Redis SCAN in Lua
**Why:** `KEYS` scans the entire Redis keyspace (O(total keys)). For Redis instances with millions of non-job keys, this blocks the event loop.

- [ ] Replace `KEYS` with cursor-based `SCAN` inside the Lua script
- [ ] Or switch to a Redis Sorted Set for the pending queue (O(log N) claim)

**Effort:** ~0.5 day

---

## What Phase 3 Unlocks (after PR A + B + C)

- ✅ Atomic multi-worker job claiming (Redis + SQLite)
- ✅ Efficient scheduled job polling (no KV churn)
- ✅ Deduplication (prevent double-work)
- ✅ Job expiration (prevent stale execution)
- ✅ Batch enqueue (efficient bulk operations)

Combined with Phase 2, this is a complete production-ready job system matching the core feature set of Sidekiq, Bull, and Celery.
