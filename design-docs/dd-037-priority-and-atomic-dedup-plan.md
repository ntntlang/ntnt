# DD-037: Priority Queues + Atomic Dedup — Implementation Plan

**Status:** Design approved
**Parent:** [DD-037 Phase 3](dd-037-phase-3-plan.md)
**Created:** 2026-03-20
**Last Updated:** 2026-03-20
**Branch:** `feat/priority-and-atomic-dedup`

---

## Feature 1: Priority Queues with Worker Bands

### Overview

Jobs support a `priority` option (0-99, lower = higher importance). Default priority is 50.

Workers are organized into **bands** — each band covers a priority range, has its own concurrency (thread count), and its own poll interval. Bands are fully isolated: a critical worker never touches low-priority jobs and vice versa. This prevents starvation and makes scaling obvious.

### Priority Range: 0-99

- **0-99** range gives 100 levels of granularity
- Two-digit zero-padded in keys for correct lexicographic ordering (`05` < `10` < `50` < `99`)
- Default priority: **50** (middle of the "normal" band)
- Validated at enqueue time: values outside 0-99 return a runtime error

### Pending Key Format

**Current:** `jobs:pending:<timestamp>:<id>`
**New:** `jobs:pending:<priority>:<timestamp>:<id>`

Examples:
```
jobs:pending:05:00000170000001:uuid-a   ← priority 5 (critical)
jobs:pending:50:00000170000002:uuid-b   ← priority 50 (normal default)
jobs:pending:99:00000170000003:uuid-c   ← priority 99 (low)
```

Within a band, jobs are FIFO by timestamp. Across bands, each band's workers only see their own range.

### Default Bands

Out-of-the-box, `work_jobs()` with no configuration spins up 4 bands with sensible defaults:

| Band | Priority range | Workers | Poll interval | Use case |
|------|---------------|---------|---------------|----------|
| critical | 0-9 | 2 | 1s | Real-time work, immediate response |
| high | 10-39 | 2 | 2s | Important operations, main workload |
| normal | 40-69 | 2 | 5s | Default — where jobs land if you don't set priority |
| low | 70-99 | 1 | 10s | Batch work, cleanup, analytics — can pile up |

7 threads total. All sleep when idle (zero CPU cost). Even a single-core VPS handles this fine.

### Worker Band Architecture

Each band spawns its own worker threads. Each worker only claims jobs in its priority range using `kv_claim` with floor+ceiling parameters:

```
Critical workers: floor="jobs:pending:00:"  ceiling="jobs:pending:09:<now>:~"
High workers:     floor="jobs:pending:10:"  ceiling="jobs:pending:39:<now>:~"
Normal workers:   floor="jobs:pending:40:"  ceiling="jobs:pending:69:<now>:~"
Low workers:      floor="jobs:pending:70:"  ceiling="jobs:pending:99:<now>:~"
```

Bands are fully isolated. No starvation. When high-priority work is empty, high workers idle — they don't steal from low. When low work piles up, you scale the low band.

### How Priorities, Numbers, and Bands Relate

The band range is the primary concept. Everything else is derived:

1. **Band range** (e.g., 0-9) → defines which jobs a worker pool scans. This controls behavior: concurrency, poll interval.
2. **Numeric value** (e.g., 5) → placed into the pending key. Determines FIFO ordering *within* a band. Derived as the midpoint of the band range when using named priorities.
3. **Named priority** (e.g., "critical") → human-friendly alias. Maps to a band, which gives you the numeric midpoint.

Workers don't see names or numbers — they scan a range. A job at priority 4 and a job at priority 5 both live in the 0-9 range, so critical workers claim both. The `4` just gets claimed before the `5` (lexicographic ordering).

**Key implication:** If you define a custom named priority "very_high" that maps to numeric value 4, but the band range is still 0-9, it behaves identically to critical — same workers, same poll rate. To get different behavior, you need a different band:

```ntnt
work_jobs(map {
  "bands": [
    map { "name": "critical",  "range": [0, 4],  "concurrency": 2, "poll": 1000 },
    map { "name": "very_high", "range": [5, 9],  "concurrency": 3, "poll": 1500 },
    // ...
  ]
})
```

Now "very_high" has its own workers, its own poll interval — genuinely different behavior. That's the whole point of 0-99: enough room to slice bands as thin as you need.

### Custom Bands

Developers can define custom bands for fine-grained control:

```ntnt
work_jobs(map {
  "bands": [
    map { "name": "payments",  "range": [10, 15], "concurrency": 4, "poll": 1000 },
    map { "name": "emails",    "range": [20, 30], "concurrency": 2, "poll": 3000 },
    map { "name": "ops",       "range": [35, 39], "concurrency": 1, "poll": 5000 },
    map { "name": "batch",     "range": [70, 99], "concurrency": 1, "poll": 10000 },
  ]
})
```

When custom bands are provided, they **replace** the defaults entirely. This gives full control over resource allocation.

### Runtime Scaling (No Restart)

Workers can be scaled at runtime without stopping the app or dropping jobs:

```ntnt
scale_workers("low", 8)       // low band now has 8 workers
scale_workers("critical", 1)  // scale critical down to 1

let bands = worker_status()
// → [{ name: "critical", concurrency: 2, active: 1, idle: 1 },
//    { name: "low", concurrency: 8, active: 3, idle: 5 }]
```

**Scale up:** Spawn more threads for the band. They immediately start polling.
**Scale down:** Set cooperative cancellation flag on excess threads. They finish their current job and exit cleanly. No interrupted work.

Uses the existing ConcurrencyRuntime cancellation mechanism — `is_current_task_cancelled()` is already checked in every worker loop iteration.

CLI equivalent (talks to running app via HTTP):
```bash
ntnt workers scale low 8
ntnt workers status
```

### DX — Job Definition

Priority is optional, per-job-type. Use **named priorities** (the default experience):

```ntnt
job SendEmail on emails (priority: "high", retry: 3) { ... }
job ProcessPayment on payments (priority: "critical") { ... }
job CleanupLogs on maintenance (priority: "low") { ... }
job ProcessOrder on orders { ... }   // defaults to "normal"
```

Named priorities:

| Priority | Meaning |
|----------|---------|
| `"critical"` | Real-time, processed immediately |
| `"high"` | Important, processed quickly |
| `"normal"` | Default — where jobs land if unspecified |
| `"low"` | Batch work, can pile up |

That's the entire API. No numbers to think about.

**Advanced: raw numeric priorities (0-99)** — for power users defining custom bands on large workloads. Named priorities map to numeric midpoints internally (critical=5, high=25, normal=50, low=85), leaving room for fine-grained custom levels in between. See "Custom Bands" section above.

```ntnt
job ChargePayment on payments (priority: 10) { ... }    // custom numeric
job SendReceipt on emails (priority: 38) { ... }         // custom numeric
```

### Implementation Changes

#### 1. `kv_claim` — Add floor parameter (src/stdlib/kv.rs)

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

#### 2. `enqueue_internal` — Priority in pending key (src/stdlib/jobs.rs)

```rust
let priority = match job_def.options.get("priority") {
    // Named priorities → map to midpoints
    Some(JobOptionValue::String(s)) => match s.as_str() {
        "critical" => 5,
        "high" => 25,
        "normal" => 50,
        "low" => 85,
        other => return Err(format!("Unknown priority '{}'. Use: critical, high, normal, low (or 0-99)", other)),
    },
    // Numeric priorities → validate range
    Some(JobOptionValue::Int(p)) if *p >= 0 && *p <= 99 => *p as u8,
    Some(JobOptionValue::Int(p)) => return Err(format!("Priority must be 0-99, got {}", p)),
    _ => 50,  // default = normal
};
let pending_key = format!("jobs:pending:{:02}:{}:{}", priority, pending_ts, job_id);
```

Also store priority in job_data: `job_data.insert("priority", Value::Int(priority))`.
Also store the band name: `job_data.insert("band", Value::String(band_name_for(priority)))`.

#### 3. `worker_loop` — Band-aware claiming (src/stdlib/jobs.rs)

`worker_loop` receives a band configuration (priority range + poll interval). Constructs floor and ceiling from the band's range:

```rust
let floor = format!("jobs:pending:{:02}:", band.min_priority);
let ceiling = format!("jobs:pending:{:02}:{}:~", band.max_priority, timestamp_key());
kv_claim(&kv_handle, "jobs:pending:", Some(&floor), Some(&ceiling))
```

#### 4. `work_jobs` / `work_async` — Band spawning (src/stdlib/jobs.rs)

When called with no `"bands"` option → use default 4-band configuration.
When called with `"bands"` → parse and use custom bands.

For each band: spawn `concurrency` worker threads, each running `worker_loop` with that band's config.

#### 5. `scale_workers` / `worker_status` — New stdlib functions (src/stdlib/jobs.rs)

- `scale_workers(band_name, count)` — add/remove workers in a band at runtime
- `worker_status()` — return current band state (name, concurrency, active count, idle count)
- Band metadata stored in `JOB_RUNTIME` (band name → Vec of task handles + config)

#### 6. `ntnt workers` CLI (src/main.rs)

- `ntnt workers status` — show band state (connects to running app via HTTP or reads KV directly)
- `ntnt workers scale <band> <count>` — runtime scaling

#### 7. Typechecker (src/typechecker.rs)

- Add signatures for `scale_workers(band_name: String, count: Int) -> Result<Unit, String>`
- Add signatures for `worker_status() -> Array<Map>`

#### 8. Documentation

- Update `// @ntnt` doc blocks on `work_jobs`, `work_async`, `enqueue`, `enqueue_at`, `enqueue_in`
- Add priority + bands to AI_AGENT_GUIDE.md
- Run `ntnt docs --generate`

### Tests

- [ ] Priority ordering within a band: enqueue at priorities 40, 50, 60 → claimed in order 40, 50, 60
- [ ] Band isolation: enqueue critical (5) and low (85) → critical worker claims 5, low worker claims 85, neither crosses
- [ ] Default priority: enqueue without priority → pending key contains `:50:`
- [ ] Invalid priority: `priority: -1` and `priority: 100` → runtime error
- [ ] Default bands: `work_jobs()` with no config → 4 bands, 7 total workers
- [ ] Custom bands: custom band config → overrides defaults entirely
- [ ] Floor+ceiling in kv_claim: claim with floor="jobs:pending:40:" ceiling="jobs:pending:69:..." → only returns keys in range
- [ ] Priority in job_data: enqueued job's data map contains `priority` field
- [ ] Batch with priority: `enqueue_batch` for a job with `priority: 15` → all keys contain `:15:`
- [ ] Scale up: `scale_workers("low", 4)` → band has 4 workers
- [ ] Scale down: `scale_workers("low", 1)` → excess workers exit after current job

---

## Feature 2: Atomic Dedup (`kv_set_nx`)

### Overview

Add a `kv_set_nx` operation to std/kv that atomically sets a key only if it doesn't exist. Use it in the job dedup path to close the race window between the current `kv_get` + `kv_set`.

### Changes Required

#### 1. `SQLiteKV::set_nx` (src/stdlib/kv.rs)

```rust
pub fn set_nx(&self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<bool>
```

- Delete expired key first: `DELETE FROM _kv WHERE key = ? AND expires_at IS NOT NULL AND expires_at <= ?`
- Then: `INSERT OR IGNORE INTO _kv (key, value, type, expires_at) VALUES (?, ?, ?, ?)`
- Check `changes()` — 0 = key existed, 1 = key was inserted
- Returns `Ok(true)` if set, `Ok(false)` if key existed

#### 2. `RedisKV::set_nx` (src/stdlib/kv.rs)

```rust
pub fn set_nx(&mut self, key: &str, value: &Value, ttl_seconds: Option<i64>) -> Result<bool>
```

- With TTL: `SET key value NX EX ttl`
- Without TTL: `SET key value NX`
- Redis returns `nil` if key exists, `"OK"` if set
- Returns `Ok(true)` if set, `Ok(false)` if key existed

#### 3. `kv_set_nx` public function (src/stdlib/kv.rs)

```rust
pub fn kv_set_nx(handle: &Value, key: &str, value: &Value, ttl: Option<i64>) -> Result<bool>
```

Dispatches to SQLite or Redis backend.

#### 4. `enqueue_internal` dedup rewrite (src/stdlib/jobs.rs)

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

#### 5. Expose as stdlib function (optional, low priority)

`set_nx(handle, key, value, opts?)` — expose to ntnt code for general use beyond dedup. Same pattern as `set`, `get`, `del`.

### Tests

- [ ] `set_nx` on empty key → returns true, value stored
- [ ] `set_nx` on existing key → returns false, original value preserved
- [ ] `set_nx` with TTL → key expires, subsequent `set_nx` succeeds
- [ ] `set_nx` with expired key (SQLite) → treats as not existing
- [ ] Dedup atomic: enqueue same job twice → second returns existing ID
- [ ] Dedup terminal replacement: enqueue, cancel job, enqueue again → new job created
- [ ] Dedup key written before job data: if enqueue fails after dedup write, next attempt cleans up

---

## Implementation Order

1. **`kv_set_nx`** — Add to both backends + public function (src/stdlib/kv.rs)
2. **`kv_claim` floor parameter** — Add to both backends (src/stdlib/kv.rs)
3. **Priority in `enqueue_internal`** — New key format + validation (src/stdlib/jobs.rs)
4. **Band-aware `worker_loop`** — Band config, floor+ceiling per band (src/stdlib/jobs.rs)
5. **Band spawning in `work_jobs`/`work_async`** — Default bands + custom config (src/stdlib/jobs.rs)
6. **Atomic dedup in `enqueue_internal`** — Replace get+set with set_nx (src/stdlib/jobs.rs)
7. **`scale_workers` + `worker_status`** — Runtime scaling (src/stdlib/jobs.rs)
8. **Tests** — All tests for both features
9. **Documentation** — Doc blocks + `ntnt docs --generate`
10. **Design doc updates** — Check off items in DD-037

## Estimated Effort

~2 days. kv_set_nx is ~30 lines per backend. kv_claim floor is ~10 lines per backend. Priority key format is ~10 lines. Band architecture + worker spawning is the bulk of new work (~200 lines). Runtime scaling reuses existing cancellation. Tests and docs round it out.

## Follow-Up: Control Socket + CLI (separate PR)

The stdlib functions (`scale_workers`, `worker_status`) are the primitives. A Unix domain socket control channel makes them accessible from a second terminal without wiring up admin routes.

### How it works

When `ntnt run server.tnt` starts workers, the runtime creates `.ntnt.sock` in the app's working directory. The CLI connects to it:

```bash
# Terminal 1: app running
cd ~/apps/payments && ntnt run server.tnt
# → .ntnt.sock created in ~/apps/payments/

# Terminal 2: ops
cd ~/apps/payments
ntnt workers status         # → connects to ./.ntnt.sock
ntnt workers scale low 8    # → sends scale command via socket

# Or target a different app explicitly
ntnt workers --dir ~/apps/emails scale low 4
```

- Socket path: `.ntnt.sock` in the app's working directory (same model as `docker compose`)
- Multiple apps on same server: each has its own socket in its own directory
- Cleanup: socket file deleted on shutdown, stale file overwritten on next start
- Auth: none needed (local Unix socket = same-user access only)
- The socket handler internally calls `scale_workers()` / `worker_status()` — same primitives

### Not blocking this PR

The socket is a convenience layer on top of the stdlib functions. This PR ships the core: priority keys, bands, kv_claim floor, kv_set_nx, scale_workers(), worker_status(). The control socket + `ntnt workers` CLI subcommand ships as a follow-up.

---

## Design Decisions

| Decision | Choice | Why |
|----------|--------|-----|
| Priority range | 0-99 (not 0-9) | 100 levels gives headroom for fine-grained bands in large workloads |
| Default priority | 50 | Middle of "normal" band |
| Key format | Zero-padded 2 digits (`{:02}`) | Correct lexicographic ordering |
| Worker model | Independent thread pools per band | Prevents starvation, obvious scaling knob |
| Default bands | 4 (critical/high/normal/low) | Matches common usage patterns |
| Scaling | Runtime, no restart | Cooperative cancellation already exists |
| Backward compat | None needed | New system, no production data to migrate |
| Dedup atomicity | `SET NX` / `INSERT OR IGNORE` | Closes race window, optimistic path is one op |
