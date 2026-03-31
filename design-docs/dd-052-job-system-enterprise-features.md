# DD-052: Job System Enterprise Features

**Status:** Draft
**Author:** Larri
**Created:** 2026-03-28
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md) (Jobs System), [DD-051](dd-051-rate-limiting-concurrency-pause.md) (Production Hardening)

---

## Overview

ntnt's job system already ships 5 of Sidekiq Enterprise's 7 core features for free. This DD scopes the remaining enterprise-grade features that would make ntnt's job system fully competitive with Sidekiq Pro ($99/mo) + Enterprise ($250/mo).

## Current State

### Already Shipped (Free in ntnt)

| Feature | Sidekiq Tier | ntnt Implementation | DD/PR |
|---------|-------------|---------------------|-------|
| Rate Limiting | Enterprise ($250/mo) | Sliding window counter, `rate: "N/interval"` | DD-051/PR#62 ✅ Merged |
| Concurrency Limits | Enterprise ($250/mo) | Atomic counter semaphore, `concurrency: N` | DD-051/PR#62 ✅ Merged |
| Unique Jobs | Enterprise ($250/mo) | SHA256 dedup via `kv_set_nx` | DD-037/PR#41 |
| Periodic Jobs | Enterprise ($250/mo) | Cron expressions in job DSL | DD-037 |
| Queue Pause/Resume | Enterprise ($250/mo) | KV-persisted + CLI + control socket | DD-051/PR#62 ✅ Merged |
| CancelToken | N/A (Sidekiq uses signals) | Condvar-based instant cooperative cancellation | DD-051/PR#62 ✅ Merged |
| Priority Queues | Pro ($99/mo) | Band-based priority with configurable ranges | DD-037/PR#41 |
| Dead Letter Queue | Pro ($99/mo) | Auto-dead after max retries, `retry_job()` | DD-037 |

### Gaps

| Feature | Sidekiq Tier | Effort | Priority | Description |
|---------|-------------|--------|----------|-------------|
| **Batches** | Pro ($99/mo) | Large | High | Group jobs, callbacks on complete/success/death |
| **Encryption** | Enterprise ($250/mo) | Medium | Medium | Transparent payload encrypt/decrypt at rest |
| **Historical Metrics** | Enterprise ($250/mo) | Medium | Medium | Time-series aggregation of job stats |
| **Rolling Restarts** | Enterprise ($250/mo) | Medium | Low | Zero-downtime deploy with process hand-off |
| **Leader Election** | Enterprise ($250/mo) | Small | Low | Single-leader across processes for cron/metrics |

---

## 1. Batches

### Problem

A common pattern: enqueue 1,000 jobs to process a CSV import, then send a notification when all are done. Without batches, the caller has to manually track completion — poll job statuses, handle partial failures, manage the "all done" callback. This is the single most-requested feature in every job system.

Sidekiq Pro sells this feature alone for $99/month. ntnt will ship it for free.

### Design

#### API Surface

```ntnt
import { batch, enqueue, batch_status, work_jobs } from "std/jobs"

job ProcessRow on imports (retry: 3) {
    perform(row_id: String) {
        let row = db.find("rows", row_id)
        transform(row)
    }
}

// Create a batch with callbacks
let b = batch("csv-import", map {
    "on_success": fn(status) { send_email("admin@co.com", "Import complete: #{status.total} rows") },
    "on_complete": fn(status) { log_info("All done: #{status.succeeded} ok, #{status.failed} failed") },
    "on_death": fn(status) { alert("Row import failed permanently: #{status.dead} dead jobs") }
})

// Add jobs to the batch — all enqueues are atomic (buffered until seal)
for row in rows {
    enqueue(b, "ProcessRow", map { "row_id": row.id })
}

// Seal the batch — flushes all buffered jobs to KV atomically
// No more jobs can be added after this (except from within batch jobs)
seal(b)

// Check status
let s = batch_status(b)
// → map { "id": "batch-abc", "total": 1000, "pending": 950, "succeeded": 50,
//          "failed": 0, "dead": 0, "status": "open", "created_at": "..." }
```

#### Callback Semantics

| Callback | Fires when | Can fire multiple times? |
|----------|-----------|------------------------|
| `on_success` | All jobs completed successfully (0 dead, 0 cancelled) | No — exactly once |
| `on_complete` | All jobs reached terminal state (completed, dead, or cancelled) | No — exactly once |
| `on_death` | First job in the batch dies (max retries exhausted) | No — first death only |

**Key rules:**
- `on_death` and `on_success` are mutually exclusive — once any job dies, `on_success` will never fire. Manual retry of a dead job does NOT reopen the `on_success` path (death is permanent for batch semantics). When a dead job is manually retried via `retry_job()`, its `batch_id` is stripped — it runs as an independent job outside the batch.
- `on_complete` always fires (success or not) — use it for cleanup
- Callbacks receive a status map with batch counters
- Callbacks run as regular jobs on the `_batch_callbacks` queue (durable, retryable)
- Empty batches (sealed with 0 jobs) immediately fire `on_success` + `on_complete`
- **Exactly-once guarantee:** Each callback has a `fired_*` flag in batch metadata. The flag is set atomically with the counter update — only the transition `false→true` enqueues the callback job.

#### Counter Semantics

| Counter | Incremented when | Decremented when | Notes |
|---------|-----------------|-----------------|-------|
| `total` | Job added to batch (at seal, or dynamic add) | Never | Immutable after seal (grows with dynamic adds) |
| `pending` | Job added | Job reaches terminal state (success, dead, cancelled) | Retryable failures do NOT decrement pending |
| `succeeded` | Job completes successfully | Never | Monotonic |
| `dead` | Job exhausts all retries | Never | Monotonic. Triggers `on_death` on first increment |
| `cancelled` | Job cancelled | Never | Monotonic |

**Terminal states** (decrement pending): `completed`, `dead`, `cancelled`.
**Non-terminal states** (pending unchanged): `failed` (will retry), `active` (executing).

There is no `failed` counter in batch metadata. Transient failures are invisible to the batch — only terminal outcomes matter.

#### Adding Jobs from Within a Batch

Jobs executing inside a batch can add more jobs to it:

```ntnt
job ProcessRow on imports {
    perform(row_id: String) {
        let row = db.find("rows", row_id)
        if row.has_children {
            // batch_id is available in the job's context
            for child in row.children {
                enqueue(batch_id(), "ProcessRow", map { "row_id": child.id })
            }
        }
        transform(row)
    }
}
```

When a job adds to its own batch:
1. **Atomically** increment `pending` AND `total` AND write the new job data + pending key in a single Lua script (Redis) or transaction (SQLite)
2. If the batch status is already `complete`, reject the add with an error
3. The increment and enqueue are a single atomic operation — no crash window where pending is incremented but the job doesn't exist

**Redis Lua for dynamic add:**
```lua
local meta = cjson.decode(redis.call('GET', KEYS[1]))  -- batch meta
if meta.status == 'complete' then return redis.error_reply('batch complete') end
meta.pending = meta.pending + 1
meta.total = meta.total + 1
redis.call('SET', KEYS[1], cjson.encode(meta))
redis.call('SET', KEYS[2], ARGV[1])  -- jobs:data:<id>
redis.call('SET', KEYS[3], ARGV[2])  -- jobs:pending:<priority>:<ts>:<id>
return 1
```

#### Nested Batches (Workflows)

Batch callbacks can create child batches, enabling multi-step workflows:

```ntnt
// Step 1: Process all images
let step1 = batch("step1-images", map {
    "on_success": fn(status) {
        // Step 2: Generate thumbnails (only if all uploads succeeded)
        let step2 = batch("step2-thumbnails", map {
            "on_success": fn(status) { mark_product_visible(product_id) }
        })
        for img in images { enqueue(step2, "GenerateThumbnail", map { "img": img }) }
        seal(step2)
    }
})
for img in images { enqueue(step1, "UploadImage", map { "img": img }) }
seal(step1)
```

Child batches are independent — the parent doesn't track child batch completion. If you need parent→child tracking, use the parent's `on_complete` to check child batch status.

#### KV Storage Layout

```
jobs:batch:<bid>              → map {
                                  "id": bid,
                                  "name": "csv-import",
                                  "status": "open" | "sealed" | "complete",
                                  "total": 0,
                                  "pending": 0,
                                  "succeeded": 0,
                                  "dead": 0,
                                  "cancelled": 0,
                                  "fired_success": false,
                                  "fired_complete": false,
                                  "fired_death": false,
                                  "created_at": "...",
                                  "sealed_at": null | "...",
                                  "completed_at": null | "...",
                                  "callbacks": { ... }
                                }
```

```
jobs:batch:<bid>:done         → set of job IDs that reached terminal state
                                (Redis: SET, SQLite: kv_set_nx per job)
```

Two keys per batch (meta + done set). Individual jobs store `batch_id` in `jobs:data:<id>`.

**Status values:**
- `open` — accepting buffered enqueues, not yet sealed
- `sealed` — jobs flushed, workers can process, accepting dynamic adds
- `complete` — all jobs terminal (pending == 0)

`complete` is the only terminal status. Whether the batch "succeeded" is derived: `dead == 0 && cancelled == 0`.

**Atomic counter updates (critical):**

All batch state transitions use atomic read-modify-write. This is the single most important correctness property.

**SQLite:** Worker loop is single-threaded per interpreter — `kv_get` + `kv_set` within BEGIN IMMEDIATE is sufficient.

**Redis:** Lua script for every counter update:

```lua
local batch_key = KEYS[1]
local job_id = ARGV[1]
local field = ARGV[2]      -- "succeeded" | "dead" | "cancelled"
local timestamp = ARGV[3]

-- Per-job terminal idempotency: only process each job's terminal event once.
-- SADD returns 0 if the job was already recorded → skip duplicate.
local added = redis.call('SADD', batch_key .. ':done', job_id)
if added == 0 then return cjson.encode({}) end  -- already processed

local meta = cjson.decode(redis.call('GET', batch_key))
if meta == nil then return cjson.encode({}) end

meta.pending = meta.pending - 1
meta[field] = (meta[field] or 0) + 1

-- Determine which callbacks to fire (exactly-once via flag transitions)
local fire = {}
if meta.pending == 0 and not meta.fired_complete then
    meta.fired_complete = true
    meta.status = 'complete'
    meta.completed_at = timestamp
    table.insert(fire, 'on_complete')
    if meta.dead == 0 and meta.cancelled == 0 and not meta.fired_success then
        meta.fired_success = true
        table.insert(fire, 'on_success')
    end
end
if field == 'dead' and meta.dead == 1 and not meta.fired_death then
    meta.fired_death = true
    table.insert(fire, 'on_death')
end

-- Enqueue callback jobs INSIDE the atomic script (no crash-loss window)
for _, cb_type in ipairs(fire) do
    if meta.callbacks[cb_type] then
        local cb_job = cjson.encode({
            type = '_BatchCallback',
            queue = '_batch_callbacks',
            batch_id = meta.id,
            callback_type = cb_type,
            status = meta
        })
        local cb_key = 'jobs:pending:50:' .. timestamp .. ':cb-' .. meta.id .. '-' .. cb_type
        redis.call('SET', 'jobs:data:cb-' .. meta.id .. '-' .. cb_type, cb_job)
        redis.call('SET', cb_key, 'cb-' .. meta.id .. '-' .. cb_type)
    end
end

redis.call('SET', batch_key, cjson.encode(meta))
return cjson.encode(fire)
```

**Critical design decisions in this script:**
1. **Per-job idempotency:** `SADD` to `jobs:batch:<bid>:done` set. If the job was already recorded (duplicate completion), the entire update is skipped. No double-decrement possible.
2. **Callback enqueue inside Lua:** Callback jobs are written directly to KV inside the atomic script. No crash-loss window between flag-set and enqueue. If the script completes, both the flag AND the callback job are guaranteed to exist.
3. **Callback execution idempotency:** The `_BatchCallback` job uses a deterministic ID (`cb-<bid>-<type>`). If somehow enqueued twice, the second write is a no-op (same key). The callback executor checks `fired_*` + an `executed_*` flag before running.

**SQLite equivalent:** Same logic in Rust within a single `BEGIN IMMEDIATE` transaction. Per-job idempotency via `INSERT OR IGNORE` into a `batch_done_jobs` tracking table (or a `_batch_done:<bid>:<job_id>` KV key checked with `kv_set_nx`).

#### Job Data Integration

Jobs track their batch membership via a `batch_id` field in `jobs:data:<id>`:

```rust
// In enqueue(), when batch handle is provided:
job_data.insert("batch_id".to_string(), Value::String(batch_id.clone()));
```

The worker loop checks for `batch_id` after job completion/death and updates the batch:

```
job completes → read batch_id from job_data
             → if batch_id present:
                 → atomically update batch counters
                 → if pending == 0 → fire on_complete callback
                 → if pending == 0 && dead == 0 → fire on_success callback
                 → if this is first death → fire on_death callback
```

#### Callback Execution

Callbacks are serialized as job definitions and enqueued on `_batch_callbacks` queue:

```rust
// When batch reaches terminal state:
let callback_job_data = map {
    "type": "_BatchCallback",
    "queue": "_batch_callbacks",
    "payload": map {
        "batch_id": bid,
        "callback_type": "on_success",
        "batch_status": current_batch_meta
    }
};
enqueue_internal(callback_job_data);
```

The `_BatchCallback` job type is a built-in that deserializes the callback function and executes it with the batch status. Callback closures are captured and serialized at `batch()` creation time using the same serialization as `spawn()` closures.

**Callback execution guarantees:** Callbacks are enqueued exactly once (guaranteed by atomic `fired_*` flags). Execution is **at-least-once** — if a callback job fails mid-execution (after side effects), it will retry. Callback authors should design for idempotency (same as any retryable job). The `executed_<type>` flag in batch metadata provides best-effort deduplication: set atomically before execution, checked on retry.

**Callback failure:** If a callback job fails, it retries like any other job. Batch data persists in KV for 24 hours after completion.

#### Seal Semantics

`seal(batch)` is what makes the batch "live":

1. Write batch metadata with `status: "sealed"`, `total: N`, `pending: N`
2. Write all buffered jobs to their pending keys
3. Both writes happen in a single atomic operation

**Atomicity per backend:**
- **SQLite:** Single `BEGIN IMMEDIATE` transaction wrapping batch meta write + all job writes
- **Redis:** `MULTI/EXEC` pipeline: `SET batch meta` + `SET job1 data` + `SET job1 pending` + ... + `EXEC`

The batch metadata (with final counts) is written FIRST within the transaction. This means if the transaction partially fails (Redis EXEC error), no jobs are visible and no callbacks can fire — safe to retry seal.

4. If count is 0 (empty batch), atomically set `fired_success`, `fired_complete`, `status: "complete"` AND enqueue callback jobs within the same transaction. Idempotent seal prevents double-fire.

**Why buffer?** Without buffering, a fast worker could complete a job before all jobs are enqueued, see `pending == 0`, and fire callbacks prematurely.

**Seal is idempotent** — calling it on an already-sealed batch is a no-op.

**Batches are not durable until sealed.** If the process crashes before `seal()`, all buffered jobs are lost. This is an explicit tradeoff for simplicity — in-memory buffering avoids partial-batch cleanup logic. Document this clearly in stdlib docs.

#### Batch Expiry

- Active batches (pending > 0): TTL refreshed on every counter update (30 days)
- Completed batches: TTL 24 hours (status queries still work during this window)
- Stale batches (no counter update for 30 days): expire automatically
- `jobs:batch:<bid>:done` set: TTL always matches batch metadata TTL (refreshed together in the counter update Lua script)

This handles the "all workers died permanently" case — the batch metadata eventually expires rather than leaking forever. The done set expires alongside the metadata so idempotency is maintained for the full batch lifetime.

#### `batch_id()` Context Function

Inside a job's perform block, `batch_id()` returns the batch ID if the job belongs to a batch, or `None` otherwise. This is passed through the job's execution context (same mechanism as `job_id`).

### Implementation Checklist

**Phase 1: Core batch lifecycle**
- [x] `BatchMeta` struct (id, name, status, counters, callbacks, timestamps)
- [x] `batch(name, opts)` stdlib function — creates batch handle, buffers enqueues
- [x] `enqueue(batch_handle, job_type, args)` — buffers job in batch, does NOT write to KV yet
- [x] `seal(batch_handle)` — atomic flush: write all jobs + set batch metadata in KV
- [x] `batch_status(batch_id)` — read batch metadata from KV
- [x] `batch_id()` — stub (Phase 3 wires context) returning current job's batch ID
- [x] Batch handle type: `Value::BatchHandle(id)` or use `Value::Map` with `_batch_id` field
- [x] `@ntnt` doc blocks + typechecker signatures
- [x] Tests: create batch, add jobs, seal, verify status

**Phase 2: Worker integration**
- [x] Worker loop: after job completion, check `batch_id` in job_data
- [x] Atomic batch counter update (kv_incr per counter key) (SQLite: single-threaded, Redis: Lua script)
- [x] Detect terminal state: `pending == 0` → mark complete
- [x] Detect success: `succeeded == total` (race-free) → mark succeeded
- [x] Detect first death: `dead` goes from 0 to 1 → fire on_death
- [x] Fire callbacks by enqueuing `_BatchCallback` jobs
- [x] `_BatchCallback` built-in job type (empty perform, Phase 3 wires closures) that executes serialized closures
- [x] Tests: success callback fires, complete callback fires, death callback fires

**Phase 3: Dynamic additions + edge cases**
- [ ] `batch_id()` available in perform block context
- [ ] `enqueue(batch_id, job_type, args)` from within a batch job — increments pending atomically
- [x] Empty batch: seal with 0 jobs → immediate callbacks
- [ ] Batch expiry: TTL on completed batches (24h), abandoned batches (30d)
- [x] Idempotent seal
- [ ] Tests: dynamic job addition, empty batch, nested batches via callbacks

**Phase 4: CLI + observability**
- [ ] `ntnt jobs batches` — list active batches with counters
- [ ] `ntnt jobs batch <bid>` — detail view
- [ ] Streaming events: `batch.created`, `batch.sealed`, `batch.complete`, `batch.succeeded`, `batch.death`
- [ ] Control socket: `batch_status` command

### Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| Callback serialization | Serialize closures (like spawn) vs store as source + re-eval | Serialize like spawn — already proven, works across processes |
| Atomic counter update (Redis) | Lua script vs optimistic locking | Lua script — one round-trip, no retry loop |
| Batch handle type | New Value variant vs Map with convention | Map with `_batch_id` — avoids AST/Value changes, consistent with KV handles |
| Max batch size | Unlimited vs configurable cap | Unlimited for v1, cap is premature optimization |
| Batch job priority | Inherit from batch vs per-job | Per-job (existing behavior) — batch doesn't change priority semantics |

### Estimated Effort: 3-4 days (4 phases)

---

## 2. Encryption

### Problem

Jobs may contain sensitive data (PII, credentials, API keys). When stored in KV (Redis/SQLite), this data is at rest in plaintext. Compliance frameworks (SOC2, HIPAA, GDPR) often require encryption at rest.

### Design

**ntnt syntax:**

```ntnt
configure_queue(map {
    "store": "redis://localhost:6379",
    "encryption_key": get_env("JOB_ENCRYPTION_KEY")
})
```

When an encryption key is configured:
- Job payloads are encrypted with AES-256-GCM before storage
- Decrypted transparently when the worker reads the job
- Job metadata (status, timestamps, queue) stays in plaintext (needed for queries)
- Only the `payload` field in `jobs:data:<id>` is encrypted

**Key management:**
- Key provided via env var or config
- Key rotation: support `encryption_keys: [current, old1, old2]` — try current first, fall back to older keys for decryption
- No custom KMS integration (env var is sufficient for v1)

### Implementation Scope

- [ ] AES-256-GCM encrypt/decrypt helpers (use `ring` or `aes-gcm` crate)
- [ ] Encrypt payload on `enqueue()` if encryption_key is configured
- [ ] Decrypt payload in worker loop before passing to perform block
- [ ] Key rotation: try each key in order for decryption
- [ ] Config: `encryption_key` / `encryption_keys` in `configure_queue`
- [ ] Tests: round-trip encrypt/decrypt, key rotation, missing key error

**Estimated effort:** 1.5-2 days

---

## 3. Historical Metrics

### Problem

ntnt already emits streaming events (`job.enqueued`, `job.completed`, `job.failed`, etc.) to stderr JSON. But there's no aggregation — operators can't ask "what was the p95 latency for SendEmail jobs last hour?" without external tooling.

### Design

**ntnt syntax:**

```ntnt
import { job_metrics } from "std/jobs"

let stats = job_metrics("SendEmail", map { "period": "1h" })
// → map {
//     "total": 4521,
//     "succeeded": 4480,
//     "failed": 41,
//     "avg_duration_ms": 230,
//     "p95_duration_ms": 890,
//     "p99_duration_ms": 1200,
//     "throughput_per_min": 75.3
// }
```

**Storage:** Time-bucketed counters in KV:

```
jobs:metrics:<type>:<bucket>  → {total, succeeded, failed, duration_sum, duration_max, ...}
```

Buckets: 1-minute granularity, kept for 24 hours. Worker loop updates counters on job completion (one `kv_incr` per metric per job — batched if possible).

**CLI:**

```bash
ntnt workers metrics              # All job types, last hour
ntnt workers metrics SendEmail    # Specific type
ntnt workers metrics --period 24h # Last 24 hours
```

### Implementation Scope

- [ ] Metrics accumulator: update KV counters on job complete/fail
- [ ] `job_metrics(type, opts)` stdlib function
- [ ] Duration tracking: p95/p99 via histogram approximation (HDR histogram or t-digest in KV)
- [ ] CLI: `ntnt workers metrics` command
- [ ] Auto-expiry: metrics buckets TTL'd at 24h
- [ ] Tests: counter accuracy, period aggregation

**Estimated effort:** 2-3 days

---

## 4. Rolling Restarts

### Problem

Deploying a new version of an ntnt app currently requires stopping workers, deploying, and starting again. During the stop, in-flight jobs are cancelled (CancelToken fires) and re-enqueued. This creates a gap where no jobs are processed.

### Design

Sidekiq Enterprise handles this by having the old process stop accepting new jobs while finishing in-flight work, then exiting. The new process starts alongside and picks up new jobs immediately.

**ntnt approach:**

1. New process starts, begins polling for jobs
2. Operator sends `ntnt workers drain` to old process (via control socket)
3. Old process stops claiming new jobs, waits for in-flight jobs to complete (with configurable timeout)
4. Old process exits
5. New process is already running and processing

**Control socket:**

```json
{"cmd": "drain", "timeout_secs": 30}
```

**CLI:**

```bash
ntnt workers drain --timeout 30
```

### Implementation Scope

- [ ] `drain` control socket command
- [ ] Drain mode: stop claiming, wait for in-flight, exit
- [ ] Configurable timeout — force-cancel after N seconds
- [ ] CLI: `ntnt workers drain` command
- [ ] Streaming event: `workers.draining`, `workers.drained`
- [ ] Tests: drain completes in-flight, drain timeout force-cancels

**Estimated effort:** 1-2 days

---

## 5. Leader Election

### Problem

Some operations should run on exactly one process: cron job scheduling, metrics aggregation, stale job reaping. Without leader election, each process does these independently, causing duplicates or conflicts.

### Design

**KV-based leader election:**

```
jobs:leader  → {process_id, elected_at, heartbeat_at, ttl: 30s}
```

- On startup, each process tries `kv_set_nx("jobs:leader", process_info, ttl: 30s)`
- Winner is leader, losers are followers
- Leader refreshes heartbeat every 10s
- If heartbeat expires (TTL), followers race to become leader
- Leader-only operations: cron scheduling, metrics aggregation, stale job cleanup

**ntnt syntax:**

```ntnt
import { is_leader, on_leader } from "std/jobs"

if is_leader() {
    // Schedule cron jobs only on leader
    schedule_cron("0 * * * *", "HourlyReport")
}

// Or use callback style
on_leader(fn() {
    log_info("I am the leader now")
})
```

### Implementation Scope

- [ ] Leader election via `kv_set_nx` with TTL
- [ ] Heartbeat refresh (10s interval)
- [ ] Automatic failover on heartbeat expiry
- [ ] `is_leader()` / `on_leader()` stdlib functions
- [ ] Integration: cron scheduler runs only on leader
- [ ] Tests: election, failover, split-brain prevention

**Estimated effort:** 1-1.5 days

---

## Priority Order

| Priority | Feature | Effort | Impact |
|----------|---------|--------|--------|
| 1 | **Batches** | 3-4 days | Highest — most requested feature in every job system |
| 2 | **Leader Election** | 1-1.5 days | Enables correct cron + metrics in multi-process |
| 3 | **Historical Metrics** | 2-3 days | Operational visibility |
| 4 | **Rolling Restarts** | 1-2 days | Zero-downtime deploys |
| 5 | **Encryption** | 1.5-2 days | Compliance requirement |

**Total estimated effort:** 9-13 days for full Sidekiq Pro+Enterprise feature parity.

---

## Competitive Position After Completion

| | Sidekiq OSS (free) | Sidekiq Pro ($99/mo) | Sidekiq Enterprise ($250/mo) | ntnt (free) |
|---|---|---|---|---|
| Basic queuing | ✅ | ✅ | ✅ | ✅ |
| Retries + dead letter | ✅ | ✅ | ✅ | ✅ |
| Priority queues | ❌ | ✅ | ✅ | ✅ |
| Unique jobs | ❌ | ❌ | ✅ | ✅ |
| Rate limiting | ❌ | ❌ | ✅ | ✅ |
| Concurrency limits | ❌ | ❌ | ✅ | ✅ |
| Queue pause/resume | ❌ | ❌ | ✅ | ✅ |
| Periodic/cron jobs | ❌ | ❌ | ✅ | ✅ |
| Batches | ❌ | ✅ | ✅ | 🔜 |
| Encryption | ❌ | ❌ | ✅ | 🔜 |
| Historical metrics | ❌ | ❌ | ✅ | 🔜 |
| Rolling restarts | ❌ | ❌ | ✅ | 🔜 |
| Leader election | ❌ | ❌ | ✅ | 🔜 |
| Instant cancellation | ❌ | ❌ | ❌ | ✅ |
| Language-native DSL | ❌ | ❌ | ❌ | ✅ |
| Multi-backend (Redis+SQLite+PG) | ❌ | ❌ | ❌ | ✅ |

ntnt ships features that Sidekiq doesn't have at any tier: CancelToken instant cancellation, language-native job DSL, and multi-backend support.

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-28 | Initial draft — gap analysis and feature designs |
| 2026-03-28 | DD-051 merged (PR #62) — rate limiting, concurrency, pause/resume, CancelToken all shipped. 5 of 7 Sidekiq Enterprise features now free in ntnt. |
