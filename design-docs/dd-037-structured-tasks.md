# DD-037: Structured Tasks & Job System for NTNT (v2)

**Status:** Draft
**Author:** Larri
**Date:** 2026-03-15 (revised from 2026-03-12 v1)
**Depends on:** std/concurrent (channels), HTTP bridge architecture

---

## Motivation

Every non-trivial web app needs background work: session cleanup, email delivery, API polling, upload processing. NTNT has no way to express "do this concurrently" beyond the HTTP request/response cycle.

Three independent phases, each shippable and useful on its own:

1. **Primitives** — `spawn`, `await_task`, `schedule`, `after`
2. **Job DSL + in-memory backend** — declarative jobs with retry, backoff, dead letter
3. **PostgreSQL backend** — persistent, production-grade job processing
4. **Redis Streams backend** — high-throughput persistent jobs via Redis consumer groups

### What We Have Today (std/concurrent)

`channel()`, `send(ch, val)`, `recv(ch)`, `recv_timeout(ch, ms)`, `try_recv(ch)`, `close(ch)`, `sleep_ms(ms)`, `thread_count()` — all implemented. Plus `on_shutdown(fn)` as a server builtin.

Good building blocks. The gap: **no way to start concurrent work that outlives a single expression**.

---

## Architecture Constraints

**The interpreter is single-threaded by design.** `Rc<RefCell<Environment>>` keeps it simple. The HTTP bridge pattern (interpreter on dedicated thread, async work on Tokio's pool, mpsc channels between them) is the blueprint for all concurrency.

**CSP by construction.** `Rc<RefCell>` forces deep copies when crossing thread boundaries. Two spawned tasks physically cannot mutate the same variable — they must communicate through channels. This is CSP enforced by architecture, not by convention. Go says "don't communicate by sharing memory" but lets you. NTNT physically can't.

**No async/await.** The synchronous model is simpler. `spawn` + channels achieve the same results without function coloring or async propagation.

---

## Phase 1: Concurrency Primitives

### `spawn(fn) -> Task`

Run a function on the Tokio thread pool. Returns an opaque handle.

```ntnt
let task = spawn(fn() {
    let resp = fetch("https://api.example.com/users")
    parse_json(resp)
})
let users = await_task(task)  // blocks, returns Result
```

**Variable capture:** deep copy of all captured values (CSP by construction). Channels are shared via Arc. Functions/closures cannot be captured (typechecker error).

> ⚠️ **Large captures are expensive.** Capturing a 10MB array deep-copies it. For large data, send through a channel instead.

### `await_task(task) -> Result` / `try_await(task) -> Option<Result>`

`await_task` blocks until done. `try_await` returns `None` if still running.

```ntnt
let data = await_task(task) otherwise { return error("Task failed: #{err}") }
```

### `cancel_task(task)`

Cooperative cancellation checked at yield points: `recv`, `sleep_ms`, `fetch`.

### `schedule(interval, fn)`

Server-lifecycle-aware recurring tasks. Global builtin (never imported).

```ntnt
schedule("every 1h", fn() {
    pg_execute(db, "DELETE FROM sessions WHERE expires_at < now()", [])
})
schedule("every 30s", fn() {
    let resp = fetch("https://api.stripe.com/health")
    if resp.status != 200 { log("Stripe is down!") }
})
listen(8080)
```

- Interval: `"every Ns"` / `"every Nm"` / `"every Nh"`
- Overlap prevention (skip if previous still running)
- Errors logged, never fatal. Auto-cancelled on shutdown. Re-registered on hot-reload.

### `after(ms, fn)`

Delayed one-shot. Sugar over `spawn` + `sleep_ms`, lifecycle-aware.

```ntnt
after(5000, fn() { send_welcome_email(user.email) })
```

### Error Handling

| Primitive | On Error |
|-----------|----------|
| `spawn` | Captured in Task. Surfaces on `await_task()` as `Err`. |
| `schedule` | Logged. Schedule continues. |
| `after` | Logged. One-shot, no retry. |

### Thread Model

```
┌──────────────────────────────────────────────────────────┐
│                   Tokio Async Runtime                    │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  ┌────────┐ │
│  │ HTTP     │  │ Spawned  │  │ Scheduled │  │ After  │ │
│  │ Handlers │  │ Tasks    │  │ Tasks     │  │ Tasks  │ │
│  └─────┬────┘  └─────┬────┘  └─────┬─────┘  └───┬────┘ │
│        └──────────────┴─────────────┴────────────┘      │
│                    ┌──▼───┐                              │
│                    │Bridge│ (mpsc)                       │
│                    └──┬───┘                              │
└───────────────────────┼──────────────────────────────────┘
                        │
┌───────────────────────▼──────────────────────────────────┐
│  Interpreter Thread — Rc<RefCell<Environment>>           │
│  Single-threaded, safe. Evaluates ntnt code.             │
└──────────────────────────────────────────────────────────┘
```

Phase 1 spawned tasks are **I/O tasks** — they call thread-safe stdlib functions (`fetch`, `sleep_ms`, channels, file I/O) but don't evaluate arbitrary ntnt expressions. Covers 90% of use cases.

### Value Serialization (Rust)

```rust
pub struct TaskHandle {
    id: u64,
    result: Arc<Mutex<Option<Result<SerializedValue, String>>>>,
    cancel: tokio::sync::watch::Sender<bool>,
    join: Option<tokio::task::JoinHandle<()>>,
}
```

### Implementation Checklist

- [ ] `spawn(fn)` — start on Tokio pool, return Task
- [ ] `SerializedValue` capture — deep-copy at spawn time
- [ ] Typechecker: reject non-serializable captures
- [ ] `await_task(task)` — block, return Result
- [ ] `try_await(task)` — non-blocking, return Option<Result>
- [ ] `cancel_task(task)` — cooperative via watch channel
- [ ] Cancellation checks in `recv`, `sleep_ms`, `fetch`
- [ ] `schedule(interval, fn)` — server builtin
- [ ] Interval parsing: `"every Ns"` / `"every Nm"` / `"every Nh"`
- [ ] Overlap prevention for scheduled tasks
- [ ] `after(ms, fn)` — delayed one-shot
- [ ] Lifecycle: cancel all on shutdown, re-register on hot-reload
- [ ] Test: spawn + await returns correct value
- [ ] Test: spawn + error → Err on await
- [ ] Test: spawn + channel communication
- [ ] Test: capture snapshot (mutation doesn't leak)
- [ ] Test: cancel_task stops at next yield point
- [ ] Test: schedule interval + overlap prevention + error resilience
- [ ] Test: after fires once after delay
- [ ] Test: shutdown cancels everything
- [ ] Docs: @ntnt annotations, sig! macros, AI_AGENT_GUIDE.md, ROADMAP.md

---

## Phase 2: Job DSL + In-Memory Backend

### Job Declaration

```ntnt
/// Sends welcome email to newly registered users
Job SendWelcomeEmail on emails {
    perform(user_id: String) {
        let user = db.find_user(user_id)
        email.send(user.email, "Welcome!", welcome_body(user))
    }
}

/// Charges customer credit card
Job ProcessPayment on payments (retry: 5, timeout: 120s) {
    perform(order_id: String, amount: Float) {
        stripe.charge(db.find(order_id).customer_id, amount)
    }
    on_failure(error, attempt) {
        log("Payment failed: #{error}")
    }
}
```

### Defaults

| Option | Default | Description |
|--------|---------|-------------|
| `retry` | `3` | Max attempts before dead letter |
| `timeout` | `30s` | Kill if exceeds |
| `backoff` | `exponential` | 1s, 2s, 4s, 8s... |
| `priority` | `normal` | `low` / `normal` / `high` |

### Enqueuing

```ntnt
SendWelcomeEmail.enqueue(map { "user_id": "123" })
SendWelcomeEmail.enqueue_in(3600, map { "user_id": "123" })  // 1 hour delay
SendWelcomeEmail.enqueue_at(tomorrow_9am, map { "user_id": "123" })
```

### Queue Configuration

```ntnt
import { Queue } from "std/jobs"
Queue.configure(map {
    "backend": "memory",
    "shutdown_timeout": 30,
    "prune_completed_after": 86400
})
```

### Job Lifecycle

```
Pending ──claim──▶ Active ──success──▶ Completed
  ▲                  │ failure
  │ (retries left)   ▼
  └──────────── Retry (backoff)
                     │ (exhausted)
                     ▼
Cancelled          Dead Letter
```

### Graceful Shutdown

Stop accepting → wait for in-flight (up to timeout) → release incomplete → exit.

### CLI

```bash
ntnt jobs status              # Counts by state
ntnt jobs list [--dead]       # Recent jobs, filterable
ntnt jobs retry <id>          # Re-enqueue dead job
ntnt jobs cancel <id>         # Cancel pending job
```

### Combined Mode

```ntnt
Queue.configure(map { "backend": "memory" })
get("/", home)
post("/api/signup", signup)
Queue.work_async()  // process jobs alongside HTTP
listen(8080)
```

### Implementation Checklist

- [ ] `Job` keyword in parser (new AST node)
- [ ] Job registry: name → definition
- [ ] `Job.enqueue(args)`, `.enqueue_in(delay, args)`, `.enqueue_at(time, args)`
- [ ] In-memory backend: thread-safe priority queue
- [ ] Worker loop: claim → execute → complete/retry/dead
- [ ] Exponential backoff retry
- [ ] Timeout enforcement
- [ ] Dead letter queue
- [ ] `on_failure` hook
- [ ] Graceful shutdown
- [ ] `Queue.configure()`, `Queue.work_async()`, `Queue.status()`, `Queue.cancel()`
- [ ] `ntnt jobs status` / `ntnt jobs list` / `ntnt jobs retry` CLI
- [ ] Test: enqueue → process → complete
- [ ] Test: failure → retry → dead letter
- [ ] Test: on_failure hook fires
- [ ] Test: graceful shutdown finishes in-flight
- [ ] Test: priority ordering
- [ ] Docs: @ntnt annotations, sig! macros, AI_AGENT_GUIDE.md

---

## Phase 3: PostgreSQL Backend

### Config

```ntnt
Queue.configure(map {
    "backend": "postgres",
    "postgres_url": env("DATABASE_URL"),
    "visibility_timeout": 300,
    "heartbeat_interval": 30
})
```

### Schema (auto-created)

```sql
CREATE TABLE ntnt_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue VARCHAR(255) NOT NULL DEFAULT 'default',
    job_type VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    result JSONB,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    priority INT NOT NULL DEFAULT 0,
    attempts INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    error TEXT,
    scheduled_at TIMESTAMPTZ DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    locked_by VARCHAR(255),
    locked_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_ntnt_jobs_pending ON ntnt_jobs(queue, priority DESC, scheduled_at)
    WHERE status = 'pending';
CREATE INDEX idx_ntnt_jobs_locked ON ntnt_jobs(locked_by, heartbeat_at)
    WHERE status = 'active';
```

### Job Claiming

`SELECT FOR UPDATE SKIP LOCKED` — atomic, no double-processing.

### Worker Heartbeats

Workers update `heartbeat_at` periodically. Stale jobs (no heartbeat > `visibility_timeout`) auto-released back to pending.

### Separate Worker Mode

```ntnt
// worker.tnt — dedicated job processor
Queue.configure(map { "backend": "postgres", "postgres_url": env("DATABASE_URL") })
import { SendEmail, ProcessPayment } from "./jobs.tnt"
Queue.work(map { "queues": ["emails", "payments"], "concurrency": 10 })  // blocking
```

### Interpreter Pool

N independent interpreters in N threads, round-robin dispatch. Each has its own memory — no locks, no contention. Jobs share state through the database, not memory.

### Observability

```ntnt
Queue.stats()      // { pending: 12, active: 3, completed: 1547, failed: 8, dead: 2 }
Queue.recent(20)   // recent job list
Queue.dead(10)     // dead letter queue
Queue.retry(id)    // re-enqueue dead job
```

### Implementation Checklist

- [ ] PostgreSQL backend + auto-migration
- [ ] `SELECT FOR UPDATE SKIP LOCKED` claiming
- [ ] Worker heartbeats + stale job release
- [ ] `Queue.work(opts)` blocking mode
- [ ] Interpreter pool: N interpreters, N threads
- [ ] `Queue.stats()`, `Queue.recent()`, `Queue.dead()`, `Queue.retry()`
- [ ] `ntnt jobs` CLI with postgres backend
- [ ] Test: job persists across restart
- [ ] Test: two workers don't double-claim (SKIP LOCKED)
- [ ] Test: crash → release after visibility timeout
- [ ] Test: interpreter pool parallel processing
- [ ] Docs: deployment guide (worker.tnt alongside server.tnt)

---

## Phase 4: Redis Streams Backend

### Configuration

```ntnt
Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL"),      // e.g. "redis://localhost:6379"
    "visibility_timeout": 300,          // seconds before stale jobs are re-claimed
    "consumer_group": "ntnt_workers",   // Redis consumer group name
    "prune_completed_after": 3600       // seconds to auto-expire completed job hashes
})
```

### Redis Data Model

| Concept | Redis Structure | Key Pattern |
|---------|----------------|-------------|
| Job queue | Stream | `ntnt:queue:{name}` |
| Job details | Hash | `ntnt:job:{id}` |
| Scheduled jobs | Sorted Set | `ntnt:scheduled` (score = unix ms) |
| Consumer groups | Stream groups | `XGROUP` on each stream |

### Job Claiming

`XREADGROUP GROUP {group} {worker_id} COUNT 1 BLOCK 100 STREAMS ntnt:queue:{name} >` — consumer groups provide atomic claiming with no double-processing, equivalent to PostgreSQL's `SELECT FOR UPDATE SKIP LOCKED`.

### Stale Job Recovery

`XPENDING` + `XCLAIM` replaces heartbeat-based detection. Messages idle longer than `visibility_timeout` are claimed by a recovery worker, ACKed, and re-added to the stream.

### Scheduled/Delayed Jobs

Stored in `ntnt:scheduled` sorted set with score = `scheduled_at` timestamp. Worker thread periodically checks `ZRANGEBYSCORE ntnt:scheduled -inf {now}` and moves ready jobs to their queue stream.

### Auto-Expiry

Completed job hashes get `EXPIRE ntnt:job:{id} {prune_seconds}` — Redis handles cleanup automatically.

### CLI

```bash
ntnt jobs status --redis-url redis://localhost:6379
ntnt jobs list --redis-url redis://localhost:6379
ntnt jobs retry <id> --redis-url redis://localhost:6379
ntnt jobs cancel <id> --redis-url redis://localhost:6379
# Also respects REDIS_URL env var
```

### Implementation Checklist

- [x] Redis Streams backend with consumer groups
- [x] `XREADGROUP` atomic job claiming
- [x] `XPENDING` + `XCLAIM` stale job recovery
- [x] Scheduled jobs via sorted set
- [x] Auto-expiry of completed job hashes
- [x] `Queue.work(opts)` blocking mode with concurrency
- [x] `Queue.stats()`, `Queue.recent()`, `Queue.dead()`, `Queue.retry()`
- [x] Per-queue stats (`Queue.stats(queue_name)`)
- [x] `ntnt jobs` CLI with `--redis-url` flag
- [x] Integration tests (all `#[ignore]` without `NTNT_TEST_REDIS_URL`)

---

## Future Work (Explicitly Deferred)

| Feature | Rationale |
|---------|-----------|
| `scope()` structured concurrency | `spawn` + `cancel_task` covers 95% of cases. Scoped lifetimes add complexity for patterns most web apps don't need. Revisit when real users hit spawn limitations. |
| Chains / Workflows / Batches | Workflow engine territory. If you need this, use Temporal. |
| ~~Redis backend~~ | ✅ Implemented in Phase 4. |
| AI observability (`ntnt jobs ask/diagnose`) | Zero value until real users generate real failures. |
| Simulation / dry-run mode | Requires effect tracking. Interesting but premature. |
| Job contracts (requires/ensures) | Function-level contracts need to mature first. |
| Intent verification | Webhooks + polling + external integration = big scope, uncertain value. |
| `parallel([fns])` | Sugar for spawn-all + await-all. Easy to add later. |
| Cron syntax | `"every Nh"` covers 90%. Add cron parsing if needed. |

---

## Design Principles

1. **No orphans.** Server shutdown cancels all tasks. No leaked work.
2. **Errors propagate, not evaporate.** `await_task` surfaces errors. `schedule` logs them. Nothing is silent.
3. **Communication over shared state.** Channels, not shared variables. Enforced by `Rc<RefCell>` architecture.
4. **Progressive complexity.** `spawn(fn)` is one line. Jobs with retry and persistence layer on top.

---

## Open Questions

1. **`spawn` outside server context?** Leaning yes — useful in CLI scripts.
2. **Task naming?** `spawn("fetch-users", fn() { ... })` — optional first arg, probably worth it.
3. **Max concurrent tasks?** Global limit (default 1000?) to prevent runaway spawns.
