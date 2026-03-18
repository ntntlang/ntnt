# DD-037 Phase 3: Implementation Plan

**Status:** Planning
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md) · [Phase 3 Design](dd-037-phase-3-implementation.md)
**Created:** 2026-03-18
**Depends on:** Phase 2 ✅, Phase 6 ✅

---

## Prioritized Feature List

Ordered by production impact — each can ship as its own commit, tested independently.

### Tier 1: Ship First (production correctness)

#### 1. Redis Atomic Claim via Lua Script
**Why first:** Without this, multi-worker Redis deployments can double-claim jobs. It's a correctness bug, not a feature.

**What changes:**
- `src/stdlib/kv.rs` — `RedisKV::claim()` (~line 601)
- Replace SCAN→GET→DEL sequence with a single `EVAL` Lua script
- Lua script: SCAN for matching keys, sort lexicographically, GET+DEL first match atomically

**Lua script:**
```lua
local keys = redis.call('KEYS', ARGV[1])
if #keys == 0 then return nil end
table.sort(keys)
local val = redis.call('GET', keys[1])
if val then
  redis.call('DEL', keys[1])
  -- Also delete type hint key if present
  local type_key = keys[1] .. ':__type'
  redis.call('DEL', type_key)
end
return {keys[1], val}
```

> Note: `KEYS` is acceptable here because the key space is scoped to `jobs:pending:*` which is bounded. For very high-volume queues (>10K pending), we'd switch to SCAN with COUNT. But that's an optimization, not a correctness issue.

**Tests:**
- Existing `test_worker_loop_end_to_end` validates claim behavior (SQLite)
- Add integration test: two threads claim concurrently from Redis, verify no double-claim
- Unit test: Lua script returns correct key/value pair, returns nil on empty

**Effort:** ~0.5 day

---

#### 2. Scheduled Job Claim Optimization
**Why second:** Current behavior wastes KV round-trips — every poll cycle claims future-dated jobs then re-enqueues them. With many scheduled jobs this is O(scheduled_jobs) per poll per worker.

**What changes:**
- `src/stdlib/kv.rs` — `SqliteKV::claim()` (~line 384)
  - Add `WHERE key < ?` bound using `format!("jobs:pending:{:020}:", now_nanos)` as upper bound
  - Future-dated pending keys are lexicographically greater → never claimed
- `src/stdlib/kv.rs` — `RedisKV::claim()` (Lua script from item 1)
  - Add timestamp ceiling to Lua script: only consider keys where the timestamp portion ≤ now
- `src/stdlib/jobs.rs` — `worker_loop()` (~line 555)
  - Remove the `scheduled_at` re-enqueue block — it becomes unreachable after KV-level filtering

**New `kv_claim` signature option:** Add optional `ceiling` parameter:
```rust
pub fn kv_claim(handle: &Value, prefix: &str) -> Result<Option<(String, Value)>>
// becomes:
pub fn kv_claim(handle: &Value, prefix: &str, ceiling: Option<&str>) -> Result<Option<(String, Value)>>
```
Worker passes `Some(&timestamp_key())` to filter at the KV layer. Existing callers pass `None` (no behavior change).

**Tests:**
- Test: enqueue_at with future timestamp → worker poll → job NOT claimed (stays pending)
- Test: enqueue_at with past timestamp → worker poll → job claimed and executed
- Test: mix of ready and future jobs → only ready ones claimed

**Effort:** ~0.5 day

---

### Tier 2: High-Value Features

#### 3. Job Deduplication (`unique: N`)
**Why:** Prevents duplicate work from retry storms, double-clicks, or idempotency-unaware callers. This is the single most-requested feature for production job systems.

**What changes:**
- `src/stdlib/jobs.rs` — `enqueue_internal()` (~line 362)
  - If job def has `unique` option (seconds):
    1. Compute SHA-256 of `format!("{}:{}", job_name, payload_json)`
    2. Check KV for `jobs:unique:<type>:<sha256>`
    3. If exists → return `Ok(Value::ok(existing_job_id))` (skip enqueue)
    4. If not → set dedup key with TTL = unique seconds, proceed with enqueue
- `src/stdlib/jobs.rs` — job option parsing
  - Add `unique` to `JobOptions` struct: `unique_secs: Option<u64>`
  - Parser extracts from `job X on q (unique: 3600) { ... }`
- `src/stdlib/jobs.rs` — on job completion/death
  - Optionally clear dedup key early (if job completes before TTL expires, allow re-enqueue)

**KV key format:** `jobs:unique:<type>:<sha256_first_16_chars>`
- 16 hex chars = 64 bits of collision resistance — more than enough for dedup within a TTL window

**Tests:**
- Test: enqueue same job twice within unique window → second returns existing ID
- Test: enqueue same job after unique window → new job created
- Test: enqueue different payload → always new job (different hash)
- Test: test mode respects dedup (test queue dedup)

**Typechecker:** No new function signatures — `unique` is a job option parsed in the DSL.

**Effort:** ~1 day

---

#### 4. Job Expiration (`expires: N`)
**Why:** Jobs stuck in pending for too long are usually stale. Without expiration they either execute with outdated data or pile up forever.

**What changes:**
- `src/stdlib/jobs.rs` — `worker_loop()` (~line 468)
  - After claiming, before execution: check `created_at + expires_secs < now`
  - If expired: set status → `"expired"`, emit `job.expired` event, skip execution
- `src/stdlib/jobs.rs` — job option parsing
  - Add `expires` to `JobOptions`: `expires_secs: Option<u64>`
- `src/stdlib/jobs.rs` — `list_jobs_filtered` / CLI
  - `"expired"` as a recognized status (display only — never enters retry loop)

**Status flow:** `pending → expired` (terminal, like `cancelled`)

**Tests:**
- Test: enqueue job with `expires: 1`, sleep 2s, worker claims → status is "expired", handler NOT called
- Test: enqueue job with `expires: 300`, claim immediately → handler called normally
- Test: `ntnt jobs list --status=expired` shows expired jobs

**Effort:** ~0.5 day

---

#### 5. Batch Enqueue (`enqueue_batch`)
**Why:** Enqueueing N jobs in a loop is N KV round-trips. Batch enqueue wraps them in a single SQLite transaction or Redis pipeline.

**What changes:**
- `src/stdlib/jobs.rs` — new function `enqueue_batch(job_name, args_array)`
  - Validates all args upfront, then writes in one batch
  - SQLite: single `BEGIN IMMEDIATE` / `COMMIT` wrapping N inserts
  - Redis: `PIPELINE` with N SET pairs
  - Returns `Result<Array<String>, String>` (array of job IDs)
- `src/typechecker.rs` — add `enqueue_batch` signature
  - `sig!("enqueue_batch", ["job_name" => Type::String, "args" => Type::Array(Box::new(Type::Map { ... }))], Type::Generic { name: "Result", args: [Type::Array(Box::new(Type::String)), Type::String] })`
- Doc blocks, stdlib reference auto-generated

**Tests:**
- Test: enqueue_batch with 10 items → 10 jobs created, all IDs returned
- Test: enqueue_batch with empty array → returns Ok([])
- Test: enqueue_batch in test mode → all 10 appear in test queue
- Test: enqueue_batch with dedup active → duplicates skipped, non-dupes enqueued

**Effort:** ~0.5 day

---

### Tier 3: Nice-to-Have (lower urgency)

#### 6. Priority Queues (`priority: N`)
**Why:** Some jobs are more important than others. But most systems don't need this until scale.

**What changes:**
- Pending key format changes from `jobs:pending:<timestamp>:<id>` to `jobs:pending:<priority>:<timestamp>:<id>`
  - Default priority: 5 (middle of 0-9 range)
  - Lower number = higher priority (lexicographic sort gives priority-first ordering)
- `enqueue_internal()` — include priority in pending key
- `worker_loop()` — no change (lexicographic claim already gives priority ordering)
- **Migration:** existing jobs have no priority prefix → treat as priority 5
  - Worker: if pending key has 2 segments after `jobs:pending:`, it's old format (no priority)
  - Backward compat: detect and handle both key formats during a transition period

**Effort:** ~1 day (mostly migration handling)

---

#### 7. Worker Heartbeat Refresh
**Why:** Jobs running >5 minutes lose visibility timeout protection. Only matters for long-running jobs.

**What changes:**
- `worker_loop()` — spawn a timer thread per job execution that refreshes `jobs:active:<id>` TTL every 30s
- Cancel timer when job completes/fails
- Configurable refresh interval: `work_async(map { "heartbeat_interval": 30 })`

**Effort:** ~0.5 day

---

#### 8. Graceful Shutdown Drain Timeout
**Why:** Currently Ctrl-C immediately stops workers. With drain timeout, in-flight jobs finish before shutdown.

**What changes:**
- `work_jobs(map { "drain_timeout": 30 })` option
- On cancellation signal: stop claiming, wait up to N seconds for in-flight
- After timeout: exit anyway (jobs become re-claimable via visibility timeout)

**Effort:** ~0.5 day

---

#### 9. `on_job_event` User Hook
**Why:** Programmatic integration — trigger custom logic on job lifecycle events. Currently only stderr JSON.

**Recommended approach:** Channel-based (option 3 from design doc)
- Worker threads send `JobEvent` structs through a `crossbeam::channel`
- Main thread runs a dispatcher that calls the user's handler function
- Clean separation: workers never touch user closures (no Send problem)

**Effort:** ~1 day

---

## Suggested PR Grouping

**PR A — Correctness & Optimization (Tier 1):**
Items 1 + 2 (Redis atomic claim + scheduled claim optimization)
- Tightly coupled (both modify `kv_claim`)
- Pure infrastructure, no new user-facing API
- ~1 day

**PR B — Dedup + Expiration (Tier 2 core):**
Items 3 + 4 (unique + expires)
- Both are job options parsed from the DSL
- Both affect `enqueue_internal` and `worker_loop`
- Natural pairing
- ~1.5 days

**PR C — Batch Enqueue (Tier 2 convenience):**
Item 5 alone
- New stdlib function + typechecker entry
- Independent of everything else
- ~0.5 day

**PR D — Priority + Polish (Tier 3):**
Items 6-9 as needed, based on demand
- Each independently shippable
- Can wait for real usage patterns

---

## Total Estimated Effort

| PR | Items | Effort | Priority |
|----|-------|--------|----------|
| A  | Redis claim + scheduled optimization | ~1 day | Ship first |
| B  | Dedup + expiration | ~1.5 days | Ship second |
| C  | Batch enqueue | ~0.5 day | Ship third |
| D  | Priority, heartbeat, drain, events | ~3 days | On demand |

**Tier 1+2 total: ~3 days.** Tier 3 is an additional ~3 days but none of it is urgent.

---

## What This Unlocks

After Phase 3 (Tier 1+2), the ntnt job system has:
- ✅ Declarative job definitions with DSL
- ✅ Multi-worker concurrent processing (correct under Redis)
- ✅ Automatic retry with exponential backoff
- ✅ Deduplication (prevent double-work)
- ✅ Job expiration (prevent stale execution)
- ✅ Batch enqueue (efficient bulk operations)
- ✅ Full observability CLI (list/inspect/retry/cancel/clear)
- ✅ Testing mode with assertions
- ✅ Streaming JSON logs

That's a complete, production-ready job system. Most frameworks (Sidekiq, Bull, Celery) ship with roughly this feature set at their core.
