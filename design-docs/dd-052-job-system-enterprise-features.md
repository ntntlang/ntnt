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

### Design

**ntnt syntax:**

```ntnt
import { batch, enqueue, work_jobs } from "std/jobs"

job ProcessRow on imports (retry: 3) {
    perform(row_id: String) {
        let row = db.find("rows", row_id)
        transform(row)
    }
}

// Create a batch with callbacks
let b = batch("csv-import", map {
    "on_success": fn() { send_email("admin@co.com", "Import complete") },
    "on_complete": fn() { log_info("All jobs finished (some may have failed)") },
    "on_death": fn(failures: Int) { alert("#{failures} rows failed permanently") }
})

// Add jobs to the batch
for row in rows {
    enqueue(b, "ProcessRow", map { "row_id": row.id })
}

// Seal the batch — no more jobs can be added after this
seal(b)
```

**Semantics:**
- `on_success` — fires when ALL jobs in the batch complete successfully (0 failures)
- `on_complete` — fires when ALL jobs reach a terminal state (completed, dead, or cancelled)
- `on_death` — fires when any job in the batch is moved to dead (max retries exhausted)
- Callbacks run as regular jobs themselves (durable, retryable)
- Batches are nestable — a batch callback can create child batches

**KV storage:**

```
jobs:batch:<id>:meta     → {total, pending, succeeded, failed, dead, status, callbacks}
jobs:batch:<id>:jobs     → set of job IDs
```

**Worker integration:** When a job completes/fails/dies, check if it belongs to a batch. Decrement pending, increment the appropriate counter. When pending reaches 0, fire callbacks.

### Implementation Scope

- [ ] `Batch` struct with ID, counters, callback storage
- [ ] `batch(name, opts)` stdlib function — creates batch, returns handle
- [ ] `enqueue(batch_handle, job_type, args)` — overload that adds to batch
- [ ] `seal(batch_handle)` — marks batch as sealed (no more additions)
- [ ] Worker loop: on job terminal state, update batch counters + fire callbacks
- [ ] Batch status: `batch_status(id)` returns counter map
- [ ] Nested batches: callback can create child batch
- [ ] Tests: success callback, complete callback, death callback, nested

**Estimated effort:** 3-4 days

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
