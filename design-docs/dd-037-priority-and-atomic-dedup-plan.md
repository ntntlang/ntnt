# DD-037: Priority Queues + Atomic Dedup — Implementation Plan

**Status:** ✅ Complete — shipped in PR #41 (merged 2026-03-20)  
**Parent:** [DD-037 Phase 3](dd-037-phase-3-plan.md)  
**Created:** 2026-03-20  
**Last Updated:** 2026-03-27  
**Branch:** `feat/priority-and-atomic-dedup`

---

# Feature 1: Priority Queues with Worker Bands

## The Developer Experience

### Defining a Job

The simplest job definition — no queue, no priority, just works:

```ntnt
job SendEmail (retry: 3) {
    perform(to: String, subject: String) {
        // send the email
    }
}
```

This job gets priority `"normal"` and goes to the `"default"` queue. Workers pick it up automatically.

### Adding Priority

Use named priorities when some jobs matter more than others:

```ntnt
job ProcessPayment (priority: "critical") { ... }     // processed immediately
job SendNotification (priority: "high") { ... }        // processed quickly
job GenerateReport (priority: "low") { ... }           // can pile up, that's fine
job ProcessOrder { ... }                                // "normal" by default
```

Four named priorities:

| Priority | Meaning |
|----------|---------|
| `"critical"` | Real-time, processed immediately |
| `"high"` | Important, processed quickly |
| `"normal"` | Default — where jobs land if unspecified |
| `"low"` | Batch work, can pile up |

That's the entire API for most apps. No numbers, no configuration.

### Adding Queues (Multi-Machine Scaling)

For single-machine apps (the 90% case), you never touch queues. Jobs go to the `"default"` queue automatically.

When you need horizontal scaling across multiple machines, add an explicit queue:

```ntnt
job ProcessPayment on payments (priority: "critical") { ... }
job SendEmail on emails (priority: "high") { ... }
job GenerateReport { ... }   // queue: "default", priority: "normal"
```

Then run separate workers per machine, all reading from the same KV store:

```bash
# Machine A: only payment jobs
ntnt worker server.tnt --queues=payments

# Machine B: only email jobs
ntnt worker server.tnt --queues=emails

# Machine C: everything
ntnt worker server.tnt   # no filter = all queues
```

### Starting Workers

```ntnt
// That's it. Default bands, default everything.
work_jobs()
```

Out of the box, this spins up 4 worker bands:

| Band | Priority range | Workers | Poll interval | Use case |
|------|---------------|---------|---------------|----------|
| critical | 0-9 | 4 | 1s | Real-time work, immediate response |
| high | 10-39 | 3 | 2s | Important operations, main workload |
| normal | 40-69 | 2 | 5s | Default — where jobs land if you don't set priority |
| low | 70-99 | 1 | 20s | Batch work, cleanup, analytics — can pile up |

10 threads total. All sleep when idle — zero CPU cost.

### Monitoring Workers

```
$ ntnt workers status

Band        Workers  Active  Pending  Completed  Failed  Avg Time
──────────  ───────  ──────  ───────  ─────────  ──────  ────────
critical    4        1       3        1,247      12      45ms
high        3        0       0        8,831      34      120ms
normal      2        2       847      42,006     198     340ms
low         1        0      3,291     5,102      41      1.2s

Total: 10 workers │ 4,141 pending │ 57,186 completed │ 285 failed
Uptime: 4h 23m │ Throughput: 3.6 jobs/sec
```

### Scaling at Runtime

Low queue backing up? Scale it without restarting the app:

```ntnt
scale_workers("low", 8)       // low band now has 8 workers
scale_workers("critical", 1)  // scale critical down to 1
```

Or from the CLI:

```bash
ntnt workers scale low 8
```

Workers added immediately. Workers removed finish their current job first — no interrupted work.

### Programmatic Status

```ntnt
let bands = worker_status()
// → [
//   map { "name": "critical", "workers": 4, "active": 1, "pending": 3,
//         "completed": 1247, "failed": 12, "avg_ms": 45 },
//   map { "name": "high", "workers": 3, "active": 0, "pending": 0,
//         "completed": 8831, "failed": 34, "avg_ms": 120 },
//   ...
// ]
```

---

## How It Works

### Priority Range: 0-99

- 100 levels of granularity
- Two-digit zero-padded in KV keys for correct lexicographic ordering (`05` < `10` < `50` < `99`)
- Default priority: **50** (midpoint of the "normal" band)
- Named priorities map to band midpoints: critical=5, high=25, normal=50, low=85

### Pending Key Format

**Current:** `jobs:pending:<timestamp>:<id>`  
**New:** `jobs:pending:<priority>:<timestamp>:<id>`

```
jobs:pending:05:00000170000001:uuid-a   ← priority 5 (critical band)
jobs:pending:50:00000170000002:uuid-b   ← priority 50 (normal band)
jobs:pending:99:00000170000003:uuid-c   ← priority 99 (low band)
```

Within a band, jobs are FIFO by timestamp. Across bands, each band's workers only see their own range.

### Worker Band Architecture

Each band spawns its own thread pool. Each worker only claims jobs in its band's priority range using `kv_claim` with floor+ceiling parameters:

```
Critical workers: floor="jobs:pending:00:"  ceiling="jobs:pending:09:<now>:~"
High workers:     floor="jobs:pending:10:"  ceiling="jobs:pending:39:<now>:~"
Normal workers:   floor="jobs:pending:40:"  ceiling="jobs:pending:69:<now>:~"
Low workers:      floor="jobs:pending:70:"  ceiling="jobs:pending:99:<now>:~"
```

Bands are fully isolated. No starvation. When high-priority work is empty, high workers idle — they don't steal from low. When low work piles up, you scale the low band.

### How Priorities, Numbers, and Bands Relate

The **band range** is the primary concept. Everything else is derived:

1. **Band range** (e.g., 0-9) → defines which jobs a worker pool scans. Controls behavior: concurrency, poll interval.
2. **Numeric value** (e.g., 5) → placed into the pending key. Determines FIFO ordering *within* a band. Derived as the midpoint when using named priorities.
3. **Named priority** (e.g., "critical") → human-friendly alias. Maps to a band, which gives the numeric midpoint.

Workers don't see names or numbers — they scan a range. A job at priority 4 and a job at priority 5 both live in the 0-9 range, so critical workers claim both. The `4` just gets claimed before the `5`.

**Key implication:** A custom named priority that maps to the same band range as an existing band gets the same workers, same poll rate. To get genuinely different behavior, you need a different band with a different range.

### Per-Band Stats Tracking

`JOB_RUNTIME` maintains counters per band:
- **completed** — `AtomicU64`, incremented after each successful job execution
- **failed** — `AtomicU64`, incremented after each failed/dead job
- **active** — `AtomicU64`, incremented when a worker starts executing, decremented when done
- **total_duration_ms** — `AtomicU64`, summed execution time for avg calculation
- **pending** — computed on demand by counting keys in the band's priority range via `kv_list`

These are cheap atomics — no locking overhead in the hot path.

---

## Advanced: Custom Bands

For fine-grained control, define custom bands that replace the defaults entirely:

```ntnt
work_jobs(map {
  "bands": [
    map { "name": "payments",  "range": [0, 15],  "concurrency": 4, "poll": 1000 },
    map { "name": "emails",    "range": [16, 39],  "concurrency": 2, "poll": 3000 },
    map { "name": "general",   "range": [40, 69],  "concurrency": 2, "poll": 5000 },
    map { "name": "batch",     "range": [70, 99],  "concurrency": 1, "poll": 10000 },
  ]
})
```

Custom bands **replace the defaults entirely** — no partial overrides. If you want to tweak one band, redefine all four. The config is only a few lines.

**Advanced: raw numeric priorities** — when using custom bands, you can assign raw numeric priorities to job types for precise ordering within a band:

```ntnt
job ChargePayment (priority: 10) { ... }    // top of "emails" band
job SendReceipt (priority: 38) { ... }       // bottom of "emails" band
```

### Band Configuration Validation

All validation runs at `work_jobs()` startup before any threads spawn. Fail fast, fail loud.

| Rule | Behavior | Error message |
|------|----------|---------------|
| **Overlapping ranges** | Rejected | `Band ranges overlap — "critical" (0-9) and "very_high" (5-15) both cover priorities 5-9` |
| **Gaps in ranges** | Rejected | `Priority gap — no band covers priorities 5-9. Jobs at these priorities would never be processed.` |
| **Concurrency = 0** | Rejected | `Band "low" has concurrency 0 — must be at least 1` |
| **Concurrency < 0** | Rejected | `Band "low" has negative concurrency` |
| **Concurrency > 32** | Warning (allowed) | `Band "critical" has concurrency 128 — unusually high (sleeping threads are cheap, but verify this is intentional)` |
| **Poll interval < 100ms** | Rejected | `Band "high" has poll interval 50ms — minimum is 100ms` |
| **Poll interval ≤ 0** | Rejected | `Band "high" has poll interval 0ms — minimum is 100ms` |
| **Range min > max** | Rejected | `Band "ops" has invalid range [39, 35] — min must be ≤ max` |
| **Range outside 0-99** | Rejected | `Band "ultra" has range [0, 150] — priority must be 0-99` |
| **Doesn't cover 0-99** | Rejected | `Bands must cover the full 0-99 range. Missing: 40-69.` |

**Why full 0-99 coverage is required:** The default priority is 50. If a custom config doesn't cover 40-69, any job using `priority: "normal"` (or no priority at all) silently gets stuck forever. Requiring full coverage prevents this.

**Gap detection:** Sort bands by range start, verify `band[N].max + 1 == band[N+1].min` for all adjacent bands and that ranges span 0 to 99.

**Overlap detection:** Sort bands by range start, verify `band[N].max < band[N+1].min` for all adjacent bands.

---

# Feature 2: Atomic Dedup (`kv_set_nx`)

## Overview

Add a `kv_set_nx` operation to std/kv that atomically sets a key only if it doesn't exist. Use it in the job dedup path to close the race window between the current `kv_get` + `kv_set`.

## Changes Required

### 1. `SQLiteKV::set_nx` (src/stdlib/kv.rs)

```rust
pub fn set_nx(&self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<bool>
```

- Delete expired key first: `DELETE FROM _kv WHERE key = ? AND expires_at IS NOT NULL AND expires_at <= ?`
- Then: `INSERT OR IGNORE INTO _kv (key, value, type, expires_at) VALUES (?, ?, ?, ?)`
- Check `changes()` — 0 = key existed, 1 = key was inserted
- Returns `Ok(true)` if set, `Ok(false)` if key existed

### 2. `RedisKV::set_nx` (src/stdlib/kv.rs)

```rust
pub fn set_nx(&mut self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<bool>
```

- With TTL: `SET key value NX EX ttl`
- Without TTL: `SET key value NX`
- Redis returns `nil` if key exists, `"OK"` if set
- Returns `Ok(true)` if set, `Ok(false)` if key existed

### 3. `kv_set_nx` public function (src/stdlib/kv.rs)

```rust
pub fn kv_set_nx(handle: &Value, key: &str, value: &Value, ttl: Option<i64>) -> Result<bool>
```

Dispatches to SQLite or Redis backend.

### 4. `enqueue_internal` dedup rewrite (src/stdlib/jobs.rs)

**Current flow (racy):**
1. `kv_get(dedup_key)` — check if exists
2. If exists, check if terminal → re-enqueue or skip
3. ... enqueue job ...
4. `kv_set(dedup_key, job_id, ttl)` — write dedup key

**New flow (atomic):**
1. Generate job_id
2. `kv_set_nx(dedup_key, job_id, ttl)` — attempt atomic claim
3. If `true` (we set it): dedup key is written, proceed to enqueue job data
4. If `false` (key existed): `kv_get(dedup_key)` to get existing job ID
5. Check if existing job is terminal (dead/cancelled/expired/failed)
6. If terminal: `kv_del(dedup_key)` → `kv_set_nx(dedup_key, new_job_id, ttl)` again
7. If not terminal: return existing job ID (skip enqueue)

**Key change:** Dedup key is written BEFORE job data (it's the atomic gate). If subsequent enqueue fails, the dangling dedup key pointing to a non-existent job is treated as terminal by the existing code (`Ok(Value::Unit) => true`), so the next enqueue cleans it up.

Remove the post-enqueue `kv_set` for dedup — no longer needed.

### 5. Expose as stdlib function (optional, low priority)

`set_nx(handle, key, value, opts?)` — expose to ntnt code for general use beyond dedup. Same pattern as `set`, `get`, `del`.

---

# Implementation

## Order

1. **Parser: optional queue** — Make `on <queue>` optional, default to "default" (src/parser.rs)
2. **`kv_set_nx`** — Add to both backends + public function (src/stdlib/kv.rs)
3. **`kv_claim` floor parameter** — Add to both backends (src/stdlib/kv.rs)
4. **Priority in `enqueue_internal`** — New key format, named + numeric priorities (src/stdlib/jobs.rs)
5. **Band-aware `worker_loop`** — Band config, floor+ceiling per band (src/stdlib/jobs.rs)
6. **Band spawning + validation in `work_jobs`/`work_async`** — Default bands, custom config, overlap/gap/bounds checks (src/stdlib/jobs.rs)
7. **Atomic dedup in `enqueue_internal`** — Replace get+set with set_nx (src/stdlib/jobs.rs)
8. **Per-band stats** — AtomicU64 counters in JOB_RUNTIME for completed/failed/active/duration (src/stdlib/jobs.rs)
9. **`scale_workers` + `worker_status`** — Runtime scaling + stats (src/stdlib/jobs.rs)
10. **Typechecker** — Signatures for new functions (src/typechecker.rs)
11. **Tests** — All tests for both features
12. **Documentation** — Doc blocks + `ntnt docs --generate`
13. **Design doc updates** — Check off items in DD-037

## Key Implementation Details

### Parser Change (src/parser.rs)

Current: `job <Name> on <queue> (<options>) { ... }` — `on <queue>` required.  
New: `job <Name> [on <queue>] (<options>) { ... }` — `on <queue>` optional, defaults to `"default"`.

Detection: after parsing job name, peek for `on` keyword. If present, consume it and parse the queue name. If not, use `"default"`.

### `kv_claim` — Add floor parameter (src/stdlib/kv.rs)

```rust
pub fn kv_claim(
    handle: &Value,
    prefix: &str,
    floor: Option<&str>,    // NEW — only claim keys >= floor
    ceiling: Option<&str>,
) -> Result<Option<(String, Value)>>
```

- **SQLite:** `WHERE key >= ? AND key <= ?` (already uses lexicographic ordering)
- **Redis Lua script:** Add ARGV for floor, filter keys within [floor, ceiling] range
- All existing callers pass `None` for floor (backward compatible at call site)

### Priority in `enqueue_internal` (src/stdlib/jobs.rs)

```rust
let priority = match job_def.options.get("priority") {
    Some(JobOptionValue::String(s)) => match s.as_str() {
        "critical" => 5,
        "high" => 25,
        "normal" => 50,
        "low" => 85,
        other => return Err("Unknown priority '{}'. Use: critical, high, normal, low (or 0-99)"),
    },
    Some(JobOptionValue::Int(p)) if *p >= 0 && *p <= 99 => *p as u8,
    Some(JobOptionValue::Int(p)) => return Err("Priority must be 0-99, got {}"),
    _ => 50,
};
let pending_key = format!("jobs:pending:{:02}:{}:{}", priority, pending_ts, job_id);
```

Store in job_data: `priority` (numeric) and `band` (resolved band name).

### Band-Aware `worker_loop` (src/stdlib/jobs.rs)

Each worker receives band config (priority range + poll interval):

```rust
let floor = format!("jobs:pending:{:02}:", band.min_priority);
let ceiling = format!("jobs:pending:{:02}:{}:~", band.max_priority, timestamp_key());
kv_claim(&kv_handle, "jobs:pending:", Some(&floor), Some(&ceiling))
```

### Typechecker (src/typechecker.rs)

- `scale_workers(band_name: String, count: Int) -> Result<Unit, String>`
- `worker_status() -> Array<Map>`

## Estimated Effort

~3 days. Broken down:
- Parser change: 0.25 day
- kv_set_nx + kv_claim floor: 0.5 day
- Priority key format + named priorities: 0.25 day
- Band architecture + worker spawning + validation: 1 day
- Atomic dedup rewrite: 0.25 day
- Per-band stats + scale_workers + worker_status: 0.5 day
- Tests (22 tests): 0.5 day
- Documentation + doc generation: 0.25 day

---

# Tests

## Priority & Bands

- [x] Priority ordering within a band: enqueue at priorities 40, 50, 60 → claimed in order 40, 50, 60
- [x] Band isolation: enqueue critical (5) and low (85) → critical worker claims 5, low worker claims 85, neither crosses
- [x] Default priority: enqueue without priority → pending key contains `:50:`
- [x] Named priority: `priority: "high"` → pending key contains `:25:`
- [x] Unknown named priority: `priority: "urgent"` → runtime error listing valid names
- [x] Invalid numeric priority: `priority: -1` and `priority: 100` → runtime error
- [x] Default bands: `work_jobs()` with no config → 4 bands, 10 total workers
- [x] Custom bands: custom band config → overrides defaults entirely
- [x] Floor+ceiling in kv_claim: claim with floor/ceiling → only returns keys in range
- [x] Priority in job_data: enqueued job's data map contains `priority` and `band` fields
- [x] Batch with priority: `enqueue_batch` for a job with `priority: "high"` → all keys contain `:25:`
- [x] Scale up: `scale_workers("low", 4)` → band has 4 workers
- [x] Scale down: `scale_workers("low", 1)` → excess workers exit after current job
- [x] Overlapping band ranges → rejected at work_jobs() startup
- [x] Gap in band ranges → rejected at work_jobs() startup
- [x] Incomplete coverage (doesn't span 0-99) → rejected
- [x] Concurrency 0 → rejected
- [x] Poll interval below 100ms → rejected
- [x] Range outside 0-99 → rejected
- [x] Range min > max → rejected

## Parser

- [x] Optional queue: `job Foo { ... }` → queue is "default"
- [x] Explicit queue: `job Foo on emails { ... }` → queue is "emails"
- [x] Queue + priority: `job Foo (priority: "high") { ... }` → queue "default", priority "high"
- [x] Queue + priority + options: `job Foo on emails (priority: "high", retry: 3) { ... }` → all correct

## Atomic Dedup

- [x] `set_nx` on empty key → returns true, value stored
- [x] `set_nx` on existing key → returns false, original value preserved
- [x] `set_nx` with TTL → key expires, subsequent `set_nx` succeeds
- [x] `set_nx` with expired key (SQLite) → treats as not existing
- [x] Dedup atomic: enqueue same job twice → second returns existing ID
- [x] Dedup terminal replacement: enqueue, cancel job, enqueue again → new job created
- [x] Dedup key written before job data: if enqueue fails after dedup write, next attempt cleans up

---

# Follow-Up: Control Socket + CLI (separate PR)

The stdlib functions (`scale_workers`, `worker_status`) are the primitives. A Unix domain socket control channel makes them accessible from a second terminal without wiring up admin routes.

When `ntnt run server.tnt` starts workers, the runtime creates `.ntnt.sock` in the app's working directory. The CLI connects to it:

```bash
# Terminal 1: app running
cd ~/apps/payments && ntnt run server.tnt

# Terminal 2: ops
cd ~/apps/payments
ntnt workers status
ntnt workers scale low 8

# Or target a different app explicitly
ntnt workers --dir ~/apps/emails scale low 4
```

- Socket path: `.ntnt.sock` in the app's working directory (same model as `docker compose`)
- Multiple apps on same server: each has its own socket in its own directory
- Cleanup: socket file deleted on shutdown, stale file overwritten on next start
- Auth: none needed (local Unix socket = same-user access only)
- The socket handler internally calls `scale_workers()` / `worker_status()`

Not blocking this PR — ships as a follow-up.

---

# Design Decisions

| Decision | Choice | Why |
|----------|--------|-----|
| Priority range | 0-99 (not 0-9) | 100 levels gives headroom for fine-grained custom bands |
| Default priority | 50 ("normal") | Midpoint of normal band |
| Named priorities | critical/high/normal/low | Clean DX — numbers are hidden unless you want them |
| Key format | Zero-padded 2 digits (`{:02}`) | Correct lexicographic ordering |
| Worker model | Independent thread pools per band | Prevents starvation, obvious scaling knob |
| Default bands | 4 (critical 4w/1s, high 3w/2s, normal 2w/5s, low 1w/20s) | Biased toward fast critical response, slow burn on low |
| Queue name | Optional, defaults to "default" | Single-machine apps never need to think about queues |
| Custom bands | Replace defaults entirely, no partial overrides | Simple mental model, config is only a few lines |
| Band validation | Reject overlaps, gaps, bad values at startup | Silent misconfiguration = jobs disappearing into a black hole |
| Full 0-99 coverage required | Custom bands must span the full range | Default priority is 50 — gaps would silently strand jobs |
| Runtime scaling | scale_workers() + cooperative cancellation | No restart needed, uses existing ConcurrencyRuntime mechanism |
| Dedup atomicity | `SET NX` / `INSERT OR IGNORE` | Closes race window, optimistic path is one operation |
| Control plane | Stdlib functions (this PR) + socket/CLI (follow-up) | Ship primitives first, convenience layer second |
