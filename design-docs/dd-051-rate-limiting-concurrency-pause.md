# DD-051: Rate Limiting, Concurrency Limits & Queue Pause/Resume

**Status:** Draft
**Author:** Larri
**Created:** 2026-03-28
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md) (Phase 5 - Production Hardening)
**Depends on:** Phase 3b ✅ (priority queues, `kv_set_nx`), Phase 3c ✅ (control socket)

---

## Overview

Three production-hardening features that protect external services, prevent worker overcommit, and give operators live queue control. All three are listed in DD-037 Phase 5 under "Additional Hardening" and are independent of the dashboard, simulation, contracts, and intent verification work.

These features share a common need: **per-job-type enforcement in the worker loop**. Rate limiting gates _how fast_ jobs fire, concurrency limits gate _how many_ run simultaneously, and pause/resume gates _whether_ a queue processes at all.

---

## 1. Rate Limiting - `rate: N/interval`

### Problem

A `SendEmail` job that processes 10K pending items will hammer the email provider API with 10K requests as fast as workers can execute. Most APIs enforce rate limits (100/minute, 1000/hour). Without client-side throttling, jobs fail with 429s, retry, and create a thundering herd.

### Design

**Job-level option:**

```ntnt
job SendEmail on emails (rate: "100/minute") {
    perform(to: String, body: String) {
        email.send(to, body)
    }
}

job WebhookDelivery on webhooks (rate: "1000/hour") {
    perform(url: String, payload: Map) {
        fetch(url, map { "method": "POST", "body": to_json(payload) })
    }
}
```

**Supported intervals:** `second`, `minute`, `hour`. Parser accepts `N/interval` string format.

**Parsing rules:**
- `"100/minute"` → 100 per 60s
- `"1000/hour"` → 1000 per 3600s
- `"5/second"` → 5 per 1s
- Invalid format → startup error with helpful message

### Implementation: Sliding Window Counter (KV-backed)

Use KV keys with TTL for a lightweight sliding window. `kv_incr` is preferred for correctness; without it, use `kv_set_nx` on per-slot keys.

**Approach: Token bucket via KV (simple, distributed-safe)**

Each rate limit window uses a single KV key per job type per window:

```
jobs:ratelimit:<job_type>:<window_start>  →  current_count (as Int)
```

**Worker-side check (in `worker_loop`, after claiming, before executing):**

1. Compute `window_start = now_secs - (now_secs % window_secs)` (floor to window boundary)
2. Build key: `jobs:ratelimit:{job_type}:{window_start}`
3. `kv_get` the key → current count (or 0 if absent)
4. If count >= limit → **re-enqueue** the job and sleep for `remaining_window_secs` (don't lose the job)
5. If count < limit → `kv_set(key, count + 1, ttl: window_secs * 2)` and proceed with execution
6. TTL ensures old windows clean up automatically

**Race condition:** Two workers might read the same count and both increment. This is acceptable - rate limiting is best-effort (same as Sidekiq's `rate_limiter`). The window resets naturally. For strict enforcement, use `kv_set_nx` on per-slot keys:

```
jobs:ratelimit:<job_type>:<window_start>:<slot_N>  →  ""  (TTL: window_secs * 2)
```

Where `slot_N` goes from 0 to limit-1. `kv_set_nx` returns false when the slot is taken → rate exceeded. This is atomic per slot.

**Recommended: hybrid approach.** Use the simple counter for most cases. The slot approach is available as a follow-up if strict enforcement is needed.

### KV Primitive Gap: `kv_incr`

The current `std/kv` has no atomic increment. Rate limiting would benefit from `kv_incr(handle, key, amount) -> Int`:

- **Redis:** `INCRBY key amount` (atomic, returns new value)
- **SQLite:** `UPDATE _kv SET value = CAST(value AS INTEGER) + ? WHERE key = ? RETURNING value` (atomic within transaction)

**Proposal:** Add `kv_incr` to `std/kv` as part of this DD. It's a general-purpose primitive, not job-specific.

- `kv_incr(handle, key, amount)` → new value as Int
- If key doesn't exist, starts from 0 (like Redis `INCRBY` behavior)
- If key exists with a non-integer value → returns `Err` (not coercion). Redis returns `WRONGTYPE`, SQLite should match.
- Respects existing TTL (doesn't reset it)
- For the rate limit path: `kv_incr` + `kv_expire` on first increment (TTL = window_secs * 2)

With `kv_incr`, the rate limit check becomes:

1. Build key: `jobs:ratelimit:{job_type}:{window_start}`
2. `kv_incr(kv, key, 1)` → new count
3. If new count == 1 → `kv_expire(kv, key, window_secs * 2)` (first hit sets TTL)
4. If new count > limit → re-enqueue job, sleep `remaining_window_secs`
5. If new count <= limit → proceed with execution

This is atomic, distributed-safe, and simple. One KV round-trip in the common case (under limit), two for the first request in a window.

### Re-enqueue vs Sleep-and-Retry

When rate limit is hit, the worker **re-enqueues** the job to its original pending key and sleeps for the remaining window time. This is better than holding the thread:

- Other bands/queues keep processing during sleep
- The re-enqueued job maintains FIFO order
- If the worker crashes during sleep, the job is already safely re-enqueued

Sleep duration: `window_secs - (now_secs % window_secs)` (time until next window opens).

### Streaming Event

When a job is rate-limited: emit `job.rate_limited` event to stderr JSON stream:

```json
{"event": "job.rate_limited", "job_id": "abc-123", "type": "SendEmail", "window": "100/minute", "current": 101, "retry_after_secs": 42}
```

Note: `current` is the post-increment count (i.e., the value returned by `kv_incr`). A value of 101 with a limit of 100 means this request was the first to exceed the window.
```

### Job Options Storage

In `JobDefinition.options`:

```rust
// New variant in JobOptionValue (if needed), or parse into struct fields:
pub rate_limit: Option<RateLimit>,

pub struct RateLimit {
    pub count: u64,       // e.g., 100
    pub window_secs: u64, // e.g., 60
}
```

Parser change: `rate` option with string value → parse `"N/interval"` in the parser or at registration time.

---

## 2. Concurrency Limits - `concurrency: N`

### Problem

A `ProcessVideo` job takes 2 minutes and uses 4GB RAM. If 20 are queued and all 20 start simultaneously, the server runs out of memory. Workers need to limit how many instances of a specific job type run at the same time, regardless of how many worker threads exist.

### Design

**Job-level option:**

```ntnt
job ProcessVideo on media (concurrency: 3) {
    perform(video_id: String) {
        // Only 3 of these run at once across all workers
        transcode(video_id)
    }
}

job SendEmail on emails (concurrency: 50, rate: "100/minute") {
    // Concurrency and rate limiting compose - both are checked
    perform(to: String) { email.send(to) }
}
```

### Implementation: Distributed Semaphore via KV

Each running instance of a job type holds a "slot" in KV. Slots are TTL-protected (same as visibility timeout) so crashed workers release slots automatically. The slot TTL **must** be refreshed whenever the job's `jobs:active:<id>` TTL is refreshed (heartbeat), so the concurrency limit remains accurate for long-running jobs.

**KV key layout:**

```
jobs:concurrency:<job_type>:<job_id>  →  ""  (TTL: visibility_timeout_secs, refreshed with the same heartbeat as `jobs:active:<id>`)
```

**Worker-side check (in `worker_loop`, after rate limit check, before executing):**

1. Look up `concurrency` from job definition options
2. `kv_list(kv, "jobs:concurrency:{job_type}:")` → count active slots
3. If count >= limit → **re-enqueue** job, sleep `poll_interval_ms`, continue
4. If count < limit → `kv_set_nx(kv, "jobs:concurrency:{job_type}:{job_id}", "", ttl: visibility_timeout_secs)`:
   - Returns `true` → slot acquired, proceed
   - Returns `false` → race lost, re-enqueue and retry
5. Set `jobs:active:<id>` only **after** slot acquisition succeeds (avoids leaking active keys when the slot is denied)
6. On job completion/failure/cancellation → `kv_del(kv, "jobs:concurrency:{job_type}:{job_id}")`

**Cleanup:** The `kv_del` in the existing job completion path (where `jobs:active:<id>` is deleted) should also delete the concurrency slot. If the worker crashes, the TTL expires and the slot is released - same pattern as visibility timeout. If there is no heartbeat today, long-running jobs already exceed the visibility timeout and can be reclaimed; concurrency limit accuracy will match that same bound.

**Race condition:** Between `kv_list` count check and `kv_set_nx`, another worker could acquire a slot. Worst case: `concurrency + N_workers` instances run briefly. This is acceptable - concurrency limits are a guardrail, not a hard mutex. The `kv_set_nx` prevents double-claiming the same slot.

**For strict enforcement (optional follow-up):** Use a Redis Lua script that atomically counts + acquires in one operation. Not needed for v1.

### Interaction with Worker Bands

Concurrency limits are orthogonal to band thread pools. A band might have 4 threads, but if `concurrency: 2`, only 2 of those threads will be executing that job type at any time (the other 2 will pick up different job types or sleep).

### Streaming Event

```json
{"event": "job.concurrency_limited", "job_id": "abc-123", "type": "ProcessVideo", "limit": 3, "active": 3}
```

### Job Options Storage

```rust
// In JobDefinition options or as a parsed field:
pub concurrency_limit: Option<u64>,
```

Parser: `concurrency` option with integer value.

---

## 3. Queue Pause/Resume

### Problem

An operator needs to stop processing a queue during a deployment, maintenance window, or incident. Currently the only option is to scale all bands to 0 (which stops everything) or kill the process.

### Design

**Stdlib API:**

```ntnt
import { pause_queue, resume_queue, queue_status } from "std/jobs"

// Pause - workers stop claiming from this queue
pause_queue("emails")

// Resume - workers start claiming again
resume_queue("emails")

// Status - check if paused
let status = queue_status("emails")
// Paused:     → map { "name": "emails", "paused": true, "paused_at": "1711612800000000000" }
// Not paused: → map { "name": "emails", "paused": false }
// (paused_at is a nanosecond epoch string, omitted when not paused)
```

**CLI (via control socket):**

```bash
ntnt workers pause emails
ntnt workers resume emails
ntnt workers status   # shows paused queues
```

**Control socket protocol:**

```json
{"cmd": "pause", "queue": "emails"}
{"cmd": "resume", "queue": "emails"}
```

### Implementation: Paused Set in JOB_RUNTIME

**Option A (in-memory, simple):** Add a `paused_queues: Mutex<HashSet<String>>` to `JobRuntime`. Workers check this set before claiming. Pause state is lost on restart.

**Option B (KV-persisted):** Store pause state in KV:

```
jobs:paused:<queue_name>  →  "<timestamp>"
```

Workers check `kv_get("jobs:paused:{queue}")` before claiming. Pause state survives restart. Slightly more overhead per poll cycle (one extra KV read per queue per poll).

**Recommendation: Option B (KV-persisted) with in-memory cache.**

1. `pause_queue(name)` → `kv_set(kv, "jobs:paused:{name}", timestamp, None)` + update in-memory cache
2. `resume_queue(name)` → `kv_del(kv, "jobs:paused:{name}")` + update in-memory cache
3. Worker check: read from in-memory `HashSet` (no KV round-trip per poll). Cache refreshed every 5 seconds from KV (handles multi-process deployments where another process paused a queue).
4. `queue_status(name)` → check in-memory cache + KV for paused_at timestamp

**In-memory cache:** Add to `JobRuntime`:

```rust
pub paused_queues: RwLock<HashSet<String>>,
paused_cache_updated_at: Mutex<std::time::Instant>,
```

Workers call `is_queue_paused(queue)` which:
1. Reads `paused_queues` (RwLock read - fast, no contention)
2. If `paused_cache_updated_at` is >5s old, refresh from KV in the background (lazy refresh)

### Worker Loop Integration

In `worker_loop`, **after claiming a job, before executing:**

```rust
// Check queue pause status
let job_queue = job_data.get("queue").unwrap_or("default");
if is_queue_paused(job_queue) {
    // Re-enqueue and sleep
    kv_set(&kv_handle, &pending_key, &Value::String(job_id), None);
    emit_job_event("job.queue_paused", &[...]);
    std::thread::sleep(poll_duration);
    continue;
}
```

**Alternative: check before claiming.** This avoids claiming and re-enqueueing, but requires knowing the queue before looking at the job. Since bands scan across queues, checking after claim is simpler. The re-enqueue cost is negligible (one KV write).

### Streaming Events

```json
{"event": "queue.paused", "queue": "emails", "paused_at": "1711612800000000000"}
{"event": "queue.resumed", "queue": "emails"}
{"event": "job.queue_paused", "job_id": "abc-123", "queue": "emails"}
```

### Control Socket Extension

Add two new commands to `dispatch_command` in `control_socket.rs`:

```rust
Some("pause") => {
    let queue = cmd.get("queue").and_then(|v| v.as_str());
    // → call pause_queue_impl(queue)
}
Some("resume") => {
    let queue = cmd.get("queue").and_then(|v| v.as_str());
    // → call resume_queue_impl(queue)
}
```

Update `cmd_status()` to include paused queues in the response.

### CLI Extension

In `main.rs`, under the `Workers` subcommand:

```rust
/// Pause a queue - workers stop claiming jobs from it.
Pause {
    /// Queue name to pause
    queue: String,
    #[arg(long, default_value = ".")]
    dir: PathBuf,
},
/// Resume a paused queue.
Resume {
    /// Queue name to resume
    queue: String,
    #[arg(long, default_value = ".")]
    dir: PathBuf,
},
```

---

## Implementation Order

### Check Order in Worker Loop

The three features integrate into `worker_loop` at specific points. The check order matters:

```
claim job from KV
  → read job data
    → skip if cancelled (existing)
    → skip if wrong queue (existing)
    → skip if future scheduled_at (existing)
    → skip if expired (existing)
    → [NEW] skip if queue is paused → re-enqueue
    → [NEW] look up job definition (moved earlier)
    → [NEW] skip if concurrency limited → re-enqueue, sleep 500ms
    → [NEW] acquire concurrency slot
    → [NEW] skip if rate limited → re-enqueue, release slot, sleep until window
    → set active key (existing, moved after checks)
    → execute job
    → [NEW] release concurrency slot
    → cleanup active key (existing)
```

**Why this order:**
1. **Pause first** — cheapest check (in-memory HashSet read). No point checking concurrency/rate if queue is paused.
2. **Concurrency second** — `kv_list` + `kv_set_nx`. Checked before rate limit so concurrency-gated re-enqueues don't consume rate limit tokens. A token spent on a job that can't run is a token wasted.
3. **Rate limit last** — `kv_incr` (one atomic KV op). Only increments the window counter after concurrency is confirmed, preventing token starvation under concurrent load.

### PR Plan

**PR 1: `kv_incr` primitive** (~0.5 day)
- [ ] Add `incr(key, amount) -> Result<i64>` to `SqliteKvStore`
- [ ] Add `incr(key, amount) -> Result<i64>` to `RedisKvStore`
- [ ] Add `kv_incr(handle, key, amount) -> Result<Value>` public API
- [ ] Register `kv_incr` in NativeFunction table
- [ ] Add `@ntnt` doc block, update STDLIB_REFERENCE.md
- [ ] Tests for both backends: basic increment, create-on-missing, negative values
- [ ] `cargo build --release --locked && ./target/release/ntnt docs --generate`

**PR 2: Queue Pause/Resume** (~1 day)
- [ ] Add `paused_queues: RwLock<HashSet<String>>` + `paused_cache_updated_at` to `JobRuntime`
- [ ] Implement `pause_queue_impl(name)`, `resume_queue_impl(name)`, `is_queue_paused(name)`
- [ ] KV persistence: `jobs:paused:<name>` keys
- [ ] Lazy cache refresh (5s interval)
- [ ] Worker loop integration: check after claim, before execution
- [ ] Stdlib functions: `pause_queue`, `resume_queue`, `queue_status`
- [ ] Register in NativeFunction table with `@ntnt` doc blocks
- [ ] Control socket: add `pause` and `resume` commands to `dispatch_command`
- [ ] CLI: add `Pause` and `Resume` subcommands under `Workers`
- [ ] Streaming events: `queue.paused`, `queue.resumed`, `job.queue_paused`
- [ ] Tests: pause prevents execution, resume allows execution, status reflects state, cache refresh
- [ ] Update `worker_status_impl` to include paused queues in response
- [ ] `cargo build --release --locked && ./target/release/ntnt docs --generate`

**PR 3: Rate Limiting** (~1.5 days)
- [ ] Parse `rate: "N/interval"` in job option parser → `RateLimit { count, window_secs }`
- [ ] Validate format at registration time (helpful error for bad formats)
- [ ] `check_rate_limit(kv, job_type, rate_limit) -> bool` using `kv_incr` + `kv_expire`
- [ ] Worker loop integration: after pause check, before concurrency check
- [ ] Re-enqueue on limit hit with `remaining_window_secs` sleep
- [ ] Streaming event: `job.rate_limited`
- [ ] Tests: rate limit enforcement, window reset, re-enqueue behavior
- [ ] Update STDLIB_REFERENCE.md with `rate` option documentation
- [ ] `cargo build --release --locked && ./target/release/ntnt docs --generate`

**PR 4: Concurrency Limits** (~1 day)
- [ ] Parse `concurrency: N` in job option parser
- [ ] Slot acquisition: `kv_set_nx("jobs:concurrency:{type}:{id}", "", ttl: visibility_timeout_secs)`
- [ ] Slot count check: `kv_list("jobs:concurrency:{type}:")` before acquisition
- [ ] Worker loop integration: after rate limit check
- [ ] Slot release: `kv_del` in job completion + failure + cancellation paths
- [ ] Re-enqueue on limit hit
- [ ] Streaming event: `job.concurrency_limited`
- [ ] Tests: concurrency enforcement, slot release on completion, TTL expiry on crash
- [ ] Refresh concurrency slot TTL alongside `jobs:active:<id>` (if/when heartbeat refresh exists)
- [ ] Interaction test: concurrency + rate limit together
- [ ] Update STDLIB_REFERENCE.md with `concurrency` option documentation
- [ ] `cargo build --release --locked && ./target/release/ntnt docs --generate`

### Estimated Total: 4-5 days

---

## Phase 5: CancelToken — Instant Cooperative Cancellation

### Problem

Worker threads use `std::thread::sleep()` for all backoff paths (rate limit window, concurrency backoff, pause backoff, poll idle). These sleeps are **not interruptible** — the thread cannot respond to cancellation, scale-down, or process shutdown until the sleep completes. For `/hour` rate limits, a worker can be unresponsive for up to 3600 seconds.

The current workaround (chunked sleep with poll-interval-sized iterations) is functional but wasteful: it wakes the thread every few seconds just to check a flag, and the pattern is duplicated at every sleep site.

The same problem exists in `std/concurrent`: `sleep_ms()` already uses 50ms polling chunks, and `schedule()` interval ticks use uninterruptible `thread::sleep`.

### Current Cancel Infrastructure

```
AtomicBool (per task)
  ├── Created in spawn_worker_task() / register_task()
  ├── Stored in TaskEntry.cancelled (concurrent.rs)
  ├── Cloned into band_cancel_arcs (jobs.rs)
  ├── Set to thread-local CURRENT_TASK_CANCELLED on thread entry
  ├── Polled by is_current_task_cancelled() at yield points
  └── Triggered by:
        ├── scale_workers (drain excess arcs)
        ├── shutdown handler (all arcs)
        ├── cancel_task() in RUNTIME
        └── test helpers (3 sites)
```

**Cancel sites (6):**
1. `scale_workers_impl` — `arc.store(true, Release)` on drained arcs
2. Shutdown handler in `work_async` — iterates all band arcs
3. `RUNTIME.cancel_task(id)` — single task cancel
4. Test: `test_worker_loop_end_to_end` cancel
5. Test: `test_worker_loop_cancel_mid_execution` cancel
6. Test: `test_scheduled_job_not_claimed_by_worker` cancel

**Sleep sites in worker_loop (12):**
1-5. Poll-cycle idle sleeps (`poll_duration`) — 5 sites
6. Queue pause backoff (`poll_duration`)
7-8. Concurrency limit backoff (500ms) — 2 sites
9. Rate limit window sleep (chunked, up to `window_secs`)
10. Rate limit KV error backoff (`poll_duration`)
11. Shutdown poll in `work_async` (500ms)
12. Shutdown delay in `work_async` (100ms)

**Sleep sites in concurrent.rs (3):**
1. `sleep_ms()` — 50ms polling chunks (already cancellation-aware, but polling)
2. `schedule()` interval tick — uninterruptible `thread::sleep(interval)`
3. `after()` delay — 50ms polling chunks

### Design: CancelToken

Replace `Arc<AtomicBool>` with a `CancelToken` that wraps `Condvar` for instant wakeup:

```rust
/// Cooperative cancellation token with instant notification.
///
/// Replaces Arc<AtomicBool> polling with Condvar-based instant wakeup.
/// All cancel paths call cancel() which sets the flag AND notifies
/// all threads waiting on wait_timeout().
pub struct CancelToken {
    inner: Mutex<bool>,
    condvar: Condvar,
}

impl CancelToken {
    pub fn new() -> Self {
        CancelToken {
            inner: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    /// Signal cancellation. Sets flag and wakes all waiting threads instantly.
    pub fn cancel(&self) {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        *guard = true;
        self.condvar.notify_all();
    }

    /// Check if cancelled (non-blocking, equivalent to AtomicBool load).
    pub fn is_cancelled(&self) -> bool {
        self.inner
            .lock()
            .map(|g| *g)
            .unwrap_or(true) // poisoned → treat as cancelled (fail-safe)
    }

    /// Sleep for up to `duration`, returning immediately if cancelled.
    /// Returns true if cancelled, false if timeout elapsed normally.
    /// Handles spurious wakeups by looping with remaining time.
    pub fn wait_timeout(&self, duration: Duration) -> bool {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(_) => return true, // poisoned → treat as cancelled (fail-safe)
        };
        if *guard {
            return true; // already cancelled
        }
        // wait_timeout_while handles spurious wakeups internally —
        // it loops until either the predicate is true or the timeout elapses.
        let result = self.condvar.wait_timeout_while(guard, duration, |cancelled| !*cancelled);
        match result {
            Ok((guard, _)) => *guard,   // true = cancelled, false = timeout elapsed
            Err(_) => true,             // poisoned → treat as cancelled (fail-safe)
        }
    }
}
```

### Performance Characteristics

| Operation | AtomicBool (current) | CancelToken (proposed) |
|-----------|---------------------|----------------------|
| `is_cancelled()` | `load(Acquire)` — ~1ns | `Mutex::lock()` — ~25-100ns (platform-dependent) |
| `cancel()` | `store(true, Release)` — ~1ns | lock + `notify_all()` — ~50-200ns |
| Wait (not cancelled) | `sleep()` polling — wakes every N ms | `wait_timeout()` — zero wakeups, OS parks thread |
| Cancel during wait | Latency = sleep chunk size (50ms–5s) | Instant (~microseconds) |

The `is_cancelled()` check is ~20× slower per call (25ns vs 1ns) due to mutex lock. This is called at yield points in the worker loop — roughly 1-5 times per job execution. At 25ns per check, even 100K jobs/second adds only 2.5ms total overhead. Negligible.

The real win is in the wait path: zero polling wakeups during idle periods, and instant response to cancellation.

### Migration Plan

**Thread-local change:**
```rust
// Before:
pub static CURRENT_TASK_CANCELLED: RefCell<Option<Arc<AtomicBool>>>
pub fn is_current_task_cancelled() -> bool {
    CURRENT_TASK_CANCELLED.with(|cell| {
        cell.borrow().as_ref().map(|flag| flag.load(Acquire)).unwrap_or(false)
    })
}

// After:
pub static CURRENT_CANCEL_TOKEN: RefCell<Option<Arc<CancelToken>>>
pub fn is_current_task_cancelled() -> bool {
    CURRENT_CANCEL_TOKEN.with(|cell| {
        cell.borrow().as_ref().map(|t| t.is_cancelled()).unwrap_or(false)
    })
}

/// Sleep cancellably using the current thread's token.
/// Returns true if cancelled, false if duration elapsed.
pub fn sleep_cancellable(duration: Duration) -> bool {
    CURRENT_CANCEL_TOKEN.with(|cell| {
        match cell.borrow().as_ref() {
            Some(token) => token.wait_timeout(duration),
            None => { std::thread::sleep(duration); false }
        }
    })
}
```

**Type changes:**
| Location | Before | After |
|----------|--------|-------|
| `TaskEntry.cancelled` | `Arc<AtomicBool>` | `Arc<CancelToken>` |
| `ScheduleEntry.cancelled` | `Arc<AtomicBool>` | `Arc<CancelToken>` |
| `JOB_RUNTIME.band_cancel_arcs` | `HashMap<String, Vec<Arc<AtomicBool>>>` | `HashMap<String, Vec<Arc<CancelToken>>>` |
| `register_task()` param | `Arc<AtomicBool>` | `Arc<CancelToken>` |
| `spawn_worker_task()` return | `Arc<AtomicBool>` | `Arc<CancelToken>` |
| `CURRENT_TASK_CANCELLED` | `RefCell<Option<Arc<AtomicBool>>>` | `RefCell<Option<Arc<CancelToken>>>` |
| `register_schedule()` return | `Arc<AtomicBool>` | `Arc<CancelToken>` |

**Cancel site changes (all identical):**
```rust
// Before:
arc.store(true, AtomicOrdering::Release);

// After:
arc.cancel();
```

**Sleep site changes (worker_loop):**
```rust
// Before (every sleep site):
std::thread::sleep(poll_duration);

// After:
if sleep_cancellable(poll_duration) { break; }
```

```rust
// Before (rate limit window — chunked sleep):
let chunk_ms = band.poll_interval_ms.max(1000);
let total_ms = (remaining.max(1) as u64) * 1000;
let mut slept_ms = 0u64;
while slept_ms < total_ms {
    if is_current_task_cancelled() { break; }
    let sleep_ms = (total_ms - slept_ms).min(chunk_ms);
    std::thread::sleep(Duration::from_millis(sleep_ms));
    slept_ms += sleep_ms;
}

// After:
sleep_cancellable(Duration::from_secs(remaining.max(1) as u64));
```

**Sleep site changes (concurrent.rs):**
```rust
// Before (sleep_ms — 50ms polling):
let slice = remaining.min(Duration::from_millis(50));
thread::sleep(slice);

// After:
if sleep_cancellable(remaining) { return Err(...) }
```

### What NOT to Change

- **Reaper thread** (`REAPER_STARTED`) — background daemon, not cancellable, keep `thread::sleep`
- **`await_task()`** — already uses its own `completed_notify` Condvar, not related to cancel
- **Test sleeps** (`thread::sleep(Duration::from_millis(300))` etc.) — timing waits in tests, not cancel-aware

### Thread-Local Token Coverage

`sleep_cancellable()` only works when the thread has `CURRENT_CANCEL_TOKEN` set. Threads without a token fall back to `std::thread::sleep()` (non-interruptible).

**Threads that MUST set a token:**
- Worker threads (`spawn_worker_task`) — already set via `CURRENT_TASK_CANCELLED`
- Schedule tick threads (`schedule()`) — must be updated to set token
- `after()` delay threads — must be updated to set token

**Threads that intentionally DON'T get a token:**
- Reaper thread — permanent background daemon, no cancel concept
- `work_async` main thread (shutdown poller) — uses its own `AtomicBool` shutdown flag, not a task; its 500ms/100ms sleeps are acceptable latency for graceful shutdown
- Test threads — timing sleeps, not cancel-aware

If a sleep site runs in a thread without a token, `sleep_cancellable()` falls back to `thread::sleep()` — it does NOT panic or error. This is a safe degradation, not a bug.

### Implementation Checklist

- [ ] Add `CancelToken` struct to `concurrent.rs` with `new()`, `cancel()`, `is_cancelled()`, `wait_timeout()`
- [ ] Add `CURRENT_CANCEL_TOKEN` thread-local (rename from `CURRENT_TASK_CANCELLED`)
- [ ] Keep `CURRENT_TASK_CANCELLED` as a deprecated alias during transition (or rename all at once)
- [ ] Update `is_current_task_cancelled()` to use `CancelToken::is_cancelled()`
- [ ] Add `sleep_cancellable(duration) -> bool` public utility
- [ ] Update `TaskEntry.cancelled` type to `Arc<CancelToken>`
- [ ] Update `ScheduleEntry.cancelled` type to `Arc<CancelToken>`
- [ ] Update `register_task()` signature
- [ ] Update `cancel_task()` to call `.cancel()` instead of `.store(true)`
- [ ] Update `register_schedule()` return type
- [ ] Update `cancel_schedule()` to call `.cancel()`
- [ ] Update `spawn_worker_task()` return type and creation
- [ ] Update `band_cancel_arcs` type in `JobRuntime`
- [ ] Update all 6 cancel sites in `jobs.rs` (`.store(true)` → `.cancel()`)
- [ ] Replace all 12 worker_loop sleep sites with `sleep_cancellable()`
- [ ] Replace `sleep_ms()` 50ms polling with single `sleep_cancellable()`
- [ ] Replace `schedule()` interval sleep with `sleep_cancellable()`
- [ ] Replace `after()` delay polling with `sleep_cancellable()`
- [ ] Remove chunked sleep code from rate limit path
- [ ] Update schedule thread to set `CURRENT_CANCEL_TOKEN`
- [ ] Tests: cancel during sleep wakes instantly
- [ ] Tests: cancel token poisoned-mutex fail-safe
- [ ] Tests: sleep_cancellable returns false on timeout, true on cancel
- [ ] Tests: worker responds to scale-down during rate limit sleep
- [ ] `cargo build --release --locked && ./target/release/ntnt docs --generate`

### Estimated Effort: 1-1.5 days

### Risk Assessment

**Medium risk.** The cancel token is load-bearing across the entire concurrency system. Mitigations:
1. The API surface is nearly identical (`is_cancelled()` vs `load()`, `cancel()` vs `store(true)`)
2. All existing tests continue to work — they test cancel behavior, not implementation
3. `is_cancelled()` on poisoned mutex returns `true` (fail-safe, same as current behavior if AtomicBool were somehow corrupted)
4. `wait_timeout()` on poisoned mutex returns `true` (fail-safe, thread wakes up and exits)

---

## Testing Strategy

### Unit Tests (per PR)

Each PR includes isolated tests for its feature. Run within the existing test harness.

### Integration Tests

- **Rate limit + concurrency together:** Enqueue 20 jobs with `rate: "5/second"` and `concurrency: 2`. Verify no more than 2 execute simultaneously AND no more than 5 per second.
- **Pause + resume round-trip:** Enqueue jobs, pause queue, verify none execute, resume, verify execution resumes.
- **Pause + rate limit:** Paused queue should not count against rate limit (no jobs executing = no rate consumption).
- **Re-enqueue ordering:** After a rate-limited job is re-enqueued, verify it maintains its original priority ordering.

### Manual Testing

- `ntnt worker` with rate-limited jobs → observe stderr events
- `ntnt workers pause emails` → verify via `ntnt workers status`
- Scale workers during active concurrency limits → verify slot accounting

---

## ntnt DSL Examples (Complete)

```ntnt
import { configure_queue, enqueue, work_jobs, pause_queue, resume_queue } from "std/jobs"

configure_queue(map { "store": "redis://localhost:6379" })

// Rate-limited email sending
job SendEmail on emails (retry: 3, rate: "100/minute") {
    perform(to: String, subject: String, body: String) {
        let result = email.send(to, subject, body)
        log_info("Sent email to #{to}")
    }
}

// Concurrency-limited video processing
job ProcessVideo on media (retry: 1, concurrency: 3, priority: "high") {
    perform(video_id: String) {
        let video = db.find("videos", video_id)
        transcode(video.path, video.format)
    }
}

// Both rate limit and concurrency
job WebhookDelivery on webhooks (rate: "1000/hour", concurrency: 10) {
    perform(url: String, payload: Map) {
        fetch(url, map { "method": "POST", "body": to_json(payload) })
    }
}

// Pause during deployment
pause_queue("emails")
// ... deploy ...
resume_queue("emails")

work_jobs()
```

---

## Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| `kv_incr` TTL behavior - reset on increment? | Reset (Redis default with INCRBY) vs preserve | Preserve (don't reset). Explicit `kv_expire` call when needed. |
| Rate limit scope - per queue or per job type? | Per queue vs per job type | Per job type. A queue can have multiple job types; rate limiting an API is per-API (per job type), not per queue. |
| Concurrency slot TTL - match visibility timeout? | Match visibility timeout vs configurable | Match visibility timeout. Refresh with the same heartbeat as `jobs:active:<id>`. |
| Cache refresh interval - configurable? | Hardcoded 5s vs option | Hardcoded 5s. Configurable is premature. |
| Pause scope - per queue or per job type? | Per queue vs per job type | Per queue. Operators think in queues ("stop all email processing"). Per-type pause can be added later. |

---

## Competitive Analysis (these features)

| Feature | ntnt (this DD) | Sidekiq | Oban | BullMQ |
|---------|---------------|---------|------|--------|
| Rate limiting | `rate: "100/minute"` | Enterprise ($250/mo) | Pro ($99/mo) | `limiter: { max: N, duration: N }` |
| Concurrency limits | `concurrency: 5` | Enterprise ($250/mo) | Pro ($99/mo) | `concurrency: N` (free) |
| Queue pause/resume | `pause_queue()` + CLI | Free (API) | Free (API) | Free (API) |
| Combined rate+concurrency | ✅ composable | Enterprise only | Pro only | ✅ free |
| Distributed (multi-process) | ✅ via KV | ✅ via Redis | ✅ via PG | ✅ via Redis |

ntnt ships rate limiting + concurrency limits for free. Sidekiq charges $250/month for these. Oban charges $99/month. This is a significant competitive advantage.

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-28 | Initial draft — rate limiting, concurrency limits, queue pause/resume |
| 2026-03-28 | Review Cycle 1 — Codex found 2 blocking issues (concurrency slot ordering, TTL refresh), fixed. 3 minor issues resolved. |
| 2026-03-28 | Review Cycle 2 — Codex found 3 hardening issues (pause cache init, check order swap, Redis incr envelope). Fixed. Check order updated: concurrency before rate limit. |
| 2026-03-28 | Greptile review — Updated design doc check order to match implementation (concurrency → rate limit). |
| 2026-03-28 | Phase 5 added — CancelToken design for instant cooperative cancellation (replaces AtomicBool polling). |

---

## Review Cycle 1

### Pass 1 Findings (Pre-fix)

- 🔴 BLOCKING - Concurrency slot acquisition order was inverted: the worker loop diagram set `jobs:active:<id>` before acquiring `jobs:concurrency:<type>:<id>`, which can leak active keys and allow execution without a slot if `kv_set_nx` loses the race. Fix by acquiring the concurrency slot first, then setting `jobs:active:<id>`, and only executing once both succeed. (Lines 355-368, 176-185)
- 🔴 BLOCKING - Slot TTL refresh was underspecified: the doc used a fixed 300s TTL and "refreshed if heartbeat exists" without defining the refresh path. For long-running jobs, concurrency limits would silently expire and over-admit. Fix by tying slot TTL to `visibility_timeout_secs` and refreshing alongside `jobs:active:<id>` heartbeat. (Lines 166-187)

### Pass 2 Findings (Post-fix)

- 🟢 MINOR - `kv_incr` behavior on non-integer existing values is unspecified. Define whether it errors or coerces, and ensure consistent behavior across Redis/SQLite. → **Fixed:** Added explicit error behavior for non-integer values.
- 🟢 MINOR - `job.rate_limited` event's `current` field should be defined as "post-increment count" (if using `kv_incr`) to avoid off-by-one confusion in logs. → **Fixed:** Clarified `current` is post-increment, updated example to 101.
- 🟢 MINOR - `queue_status()` return shape should document whether `paused_at` is omitted vs `null` when not paused, and whether it is an integer or string epoch. → **Fixed:** Documented both states, `paused_at` omitted when not paused, nanosecond epoch string.
