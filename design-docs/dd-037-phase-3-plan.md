# DD-037 Phase 3: Implementation Plan

**Status:** PR A ✅ in review (#36), PR B ✅ in review (#38), PR C 📋 planned
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md) · [Phase 3 Design](dd-037-phase-3-implementation.md)
**Created:** 2026-03-18
**Depends on:** Phase 2 ✅, Phase 6 ✅

---

## Completed PRs

### PR A — Atomic Claim + Scheduled Optimization (#36, in review)

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

### PR B — Dedup + Expiration (#38, in review)

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

| Item | Description | Priority |
|------|-------------|----------|
| Priority queues | `priority: N` option, key layout change | Low |
| Worker heartbeat refresh | TTL refresh every 30s during long jobs | Low |
| Graceful shutdown drain | Configurable drain timeout on Ctrl-C | Low |
| `on_job_event` user hook | Channel-based event dispatch to user closures | Medium |
| Atomic dedup (SET NX) | True atomic dedup under concurrent enqueues | Medium |
| Redis SCAN in Lua | Replace KEYS with cursor-based SCAN for large keyspaces | Low |

These can be addressed based on user demand / production usage patterns.

---

## What Phase 3 Unlocks (after PR A + B + C)

- ✅ Atomic multi-worker job claiming (Redis + SQLite)
- ✅ Efficient scheduled job polling (no KV churn)
- ✅ Deduplication (prevent double-work)
- ✅ Job expiration (prevent stale execution)
- ✅ Batch enqueue (efficient bulk operations)

Combined with Phase 2, this is a complete production-ready job system matching the core feature set of Sidekiq, Bull, and Celery.
