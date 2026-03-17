# DD-037: Concurrency & Job System

**Status:** In Progress  
**Author:** Larri  
**Created:** 2026-03-15  
**Last Updated:** 2026-03-16  
**Branch:** `feat/concurrency-v2` (current work)  
**Supersedes:** `background_jobs.md`, `dd-037-structured-tasks.md`

---

## Table of Contents

1. [Vision](#vision)
2. [Architecture](#architecture)
3. [Current State — What's Built](#current-state--whats-built)
4. [Roadmap](#roadmap)
5. [Phase Details](#phase-details)
6. [Dashboard Security Model](#dashboard-security-model)
7. [Lessons Learned](#lessons-learned)
8. [Competitive Analysis](#competitive-analysis)
9. [Open Questions](#open-questions)
10. [std/events — Event Dispatch Layer](#stdevents--event-dispatch-layer)

---

## Vision

ntnt needs two things:

1. **Concurrency primitives** — spawn tasks, communicate between them, schedule recurring work. Go-inspired CSP, ntnt's simplicity.
2. **Background job system** — persistent, reliable, production-grade job queuing with declarative syntax. Better than Sidekiq, Oban, and hand-rolled PHP systems — with less code, better DX, and built-in safety.

**Design principles:**
- **Zero-config start** — `spawn()` just works. Jobs work in-memory with no setup.
- **CSP by architecture** — `Rc<RefCell>` forces serialization at task boundaries. Two tasks physically cannot share memory.
- **No async/await** — synchronous model, no function coloring. `spawn` + channels.
- **Declarative jobs** — `Job SendEmail on emails { perform(id) { ... } }` — the parser IS the registry
- **Two backends** — Memory (dev/simple) and KV (production) via `std/kv`. KV supports Redis, SQLite, Valkey, DragonflyDB.
- **Secure by default** — Dashboard is localhost-only. Remote access requires explicit auth.
- **IDD-native** — Jobs are testable with intent specs. Intent verification is table stakes, not a feature.

---

## Architecture

### How Concurrency Works in ntnt

The interpreter is single-threaded by design (`Rc<RefCell<Environment>>`). All concurrency uses thread-per-task with serialized value passing:

```
┌─────────────────────────────────────────────────────────────┐
│                  Main Thread (Interpreter)                    │
│  let task = spawn(fn() { heavy_work() })                     │
│  let result = await_task(task)  // blocks                    │
└─────────┬───────────────────────────────────┬───────────────┘
          │ serialize captures                 │ deserialize result
          ▼                                    ▲
┌─────────────────────┐              ┌────────────────────────┐
│   Worker Thread     │              │  SerializedValue       │
│  ┌───────────────┐  │   result     │  (thread-safe enum)    │
│  │ Fresh         │──┼──────────────│  Int|Float|Bool|String │
│  │ Interpreter   │  │              │  Array|Map             │
│  │ (own Rc/Ref)  │  │              └────────────────────────┘
│  └───────────────┘  │
└─────────────────────┘
```

Each spawned task gets a **fresh interpreter instance** with captured bindings injected. Cross-task communication goes through channels (crossbeam). This gives us:
- True parallelism (OS threads)
- Zero shared mutable state (architecturally impossible)
- Panic isolation (catch_unwind per task)

**Value serialization for cross-thread use:**

| Type | Serializable | Notes |
|------|-------------|-------|
| Int, Float, Bool, String | ✅ | Primitive values |
| Array, Map | ✅ | Recursively serialized |
| Struct, Enum | ✅ | Field values serialized |
| TaskHandle, ChannelHandle, ScheduleHandle | ✅ (capture) | Capturable at spawn time; cannot be sent through channels |
| Function (closure) | ❌ | Contains `Rc<RefCell>` — not thread-safe |

### Why KV Instead of Raw PostgreSQL/Redis

ntnt already has `std/kv` — a key-value module that works with Redis, SQLite, and Redis-compatible stores:

```ntnt
let kv = unwrap(open("redis://localhost:6379"))   // Redis / Valkey / Dragonfly
let kv = unwrap(open("sqlite:./jobs.db"))          // SQLite (single-node)
```

Building the job backend on `std/kv` means:
- **One implementation, multiple stores.** No separate PG and Redis backends.
- **Users choose their store.** Same API whether Redis in prod or SQLite in dev.
- **KV operations map naturally to job queues.** Sorted sets for scheduling, atomic ops for claiming, TTL keys for heartbeats.
- **No SQL schema to manage.** KV is schemaless.

Sidekiq doesn't use Redis Streams either — it uses lists (LPUSH/BRPOP) and sorted sets (ZADD/ZRANGEBYSCORE). Same pattern works through `std/kv`.

### Job Chaining

No `Chain` or `Workflow` DSL needed. Just use ntnt:

```ntnt
Job ProcessOrder on orders {
    perform(order_id: String) {
        let order = validate_order(order_id)
        ChargePayment.enqueue(map { "order_id": order_id, "amount": order.amount })
    }
}

Job ChargePayment on payments {
    perform(order_id: String, amount: Float) {
        let charge = stripe.charge(amount)
        SendConfirmation.enqueue(map { "order_id": order_id, "charge_id": charge.id })
    }
}
```

When a job completes, it enqueues the next one. Simple, explicit, debuggable. No magic dependency graph.

### ConcurrencyRuntime

Single global instance (`LazyLock<ConcurrencyRuntime>`) owns all state:
- Monotonic ID counter (`AtomicU64`) shared by tasks, channels, and schedules
- Task registry with state, result, error, cancellation flag, completion time
- Channel registry (crossbeam sender/receiver pairs)
- Schedule registry with cancellation and overlap-prevention flags
- Lock discipline: acquire → clone Arcs → drop → operate. Never nest locks.

---

## Current State — What's Built

### ✅ Phase 0: Concurrency Primitives

**Branch:** `feat/concurrency-v2`  
**Status:** Complete.

- [x] `spawn(fn) -> TaskHandle` — run function in background thread
- [x] `await_task(task) -> Result` — block until done, consume handle
- [x] `try_await(task) -> Map` — non-blocking peek with status (running/completed/failed/consumed/expired)
- [x] `cancel_task(task) -> Bool` — cooperative cancellation
- [x] `channel() -> ChannelHandle` — create unbounded crossbeam channel
- [x] `send(ch, value) -> Bool` — send value (false if closed)
- [x] `recv(ch) -> Value` — blocking receive
- [x] `recv_timeout(ch, ms) -> Option` — receive with timeout
- [x] `try_recv(ch) -> Option` — non-blocking receive
- [x] `close(ch) -> Bool` — close channel
- [x] `select(channels, timeout?) -> Map` — multi-channel wait (crossbeam::Select)
- [x] `schedule(interval, fn()) -> ScheduleHandle` — recurring execution
- [x] `cancel_schedule(sched) -> Bool` — stop recurring execution
- [x] `after(delay, fn()) -> TaskHandle` — delayed one-shot execution
- [x] `sleep_ms(ms) -> Unit` — cancellation-aware sleep
- [x] `thread_count() -> Int` — available CPU threads

### ✅ Phase 1: Primitive Hardening

**Commits:** `f63f23b`→`c2b2685` on `feat/concurrency-v2`  
**Status:** Complete. All Copilot review comments addressed and resolved. CI green.

#### Core Changes
- [x] **Opaque handle types** — `Value::TaskHandle(u64)`, `Value::ChannelHandle(u64)`, `Value::ScheduleHandle(u64)`. Type-safe at the Value level.
- [x] **`try_await` consumed/expired states** — `TaskState::Consumed` and `TaskState::Expired`. Never errors for handles that existed.
- [x] **`select()` with crossbeam-channel** — Replaced all `std::sync::mpsc`. 100ms cancellation-aware slices.
- [x] **Docs generated** — `ntnt docs --generate` run, STDLIB_REFERENCE.md updated with `select()`

#### Copilot Review Fixes (2 rounds, 16 comments, all resolved)
- [x] **`select()` busy-loop fix** — track disconnected channels in `alive[]` vec, rebuild `crossbeam::Select` excluding dead channels, return `{status: "closed"}` when all dead
- [x] **Consumed task reaping** — `TaskState::Consumed` included in reaper expiry (5-min TTL, respects `last_checked_at`)
- [x] **Reaper lock discipline** — clone Arcs while holding registry lock, drop lock, inspect state outside lock (no nested locks)
- [x] **NativeFunction ambiguity detection** — `find_all_in_loaded_modules()` errors when multiple modules export same name+arity
- [x] **NativeFunction capture removed** — child interpreter uses own stdlib; no name-based re-injection
- [x] **Closure capture fail-fast** — `capture_bindings()` returns `Err` listing non-serializable closures
- [x] **Builtin injection** — `inject_captured` searches builtins via `get_global()` 
- [x] **Typechecker signatures** — proper return types: `Channel`, `Task`, `Schedule`, `Map`, `Optional<Any>`
- [x] **Unconditional shutdown** — `RUNTIME.shutdown()` runs even on eval error
- [x] **Doc wording** — "marks as consumed" (not "removes") in 4 doc files + docstrings

#### Test Status
- [x] 1,076 tests passing (37 concurrency tests)
- [x] CI green (fmt + test + build + docs drift)
- [x] 2 Copilot review rounds, 16 comments, all replied to and resolved

---

## Roadmap

### Overview

```
Phase 0  ✅  Concurrency Primitives                    DONE
Phase 1  ✅  Primitive Hardening                        DONE
Phase 2  📋  Composition Layer                          parallel, race, task groups
Phase 3  📋  Job DSL + In-Memory Backend                declarative jobs, streaming logs
Phase 4  📋  KV Backend + Dashboard                     persistent jobs, real-time UI
Phase 5  📋  Production Hardening                       simulation, contracts, intent testing
Phase 6  📋  Observability CLI                          ntnt jobs status/list/tail/replay
Phase 7  📋  Event Dispatch (std/events)                pub/sub fan-out over the job system
```

### Phase Status Table

| Phase | Name | Status | Priority |
|-------|------|--------|----------|
| 0 | Concurrency Primitives | ✅ Done | — |
| 1 | Primitive Hardening | ✅ Done (2 Copilot reviews, 16/16 resolved) | — |
| 2 | Composition Layer | 📋 Planned | P1 |
| 3 | Job DSL + In-Memory Backend | 📋 Planned | P0 — core feature |
| 4 | KV Backend + Dashboard | 📋 Planned | P0 — production req |
| 5 | Production Hardening | 📋 Planned | P1 — safety |
| 6 | Observability CLI | 📋 Planned | P1 — DX |
| 7 | Event Dispatch (`std/events`) | 📋 Planned | P2 — event-driven DX |

---

## Phase Details

### Phase 2: Composition Layer 📋

**Depends on:** Phase 1 ✅  
**Estimated effort:** 2-3 days

- [ ] `parallel(fns) -> Array` — run N functions, collect all results, cancel on first error
- [ ] `race(fns) -> Value` — run N functions, return first result, cancel others
- [ ] `task_group(fn(group))` — structured scope, all tasks cancelled when scope exits
- [ ] Tests for each composition primitive
- [ ] Documentation in STDLIB_REFERENCE.md + agent guide

```ntnt
// parallel — fan-out, fan-in
let [users, posts] = parallel([
    fn() { db.query("SELECT * FROM users") },
    fn() { db.query("SELECT * FROM posts") },
])

// race — first wins
let fastest = race([
    fn() { fetch("https://api1.example.com/data") },
    fn() { fetch("https://api2.example.com/data") },
])

// task_group — structured concurrency scope
task_group(fn(group) {
    group.spawn(fn() { process_a() })
    group.spawn(fn() { process_b() })
    // All tasks cancelled when this block exits
})
```

---

### Phase 3: Job DSL + In-Memory Backend 📋

**Depends on:** Phase 1 ✅  
**Estimated effort:** 4-5 days  
**Source:** `feat/job-dsl` branch (preserved code to cherry-pick from)

The core job system. Declarative syntax, streaming logs, batch operations.

#### Job Declaration & Registry
- [ ] `Job Name on queue { perform(args) { ... } }` parser syntax
- [ ] `Job Name on queue (retry: N, timeout: Xs, unique: N) { ... }` — inline options
- [ ] `on_failure(error, attempt) { ... }` hook
- [ ] Doc comment metadata (`/// Triggers:`, `/// Affects:`, `/// Side effects:`)
- [ ] Parser auto-registers jobs — no manual registry needed

#### In-Memory Backend
- [ ] `Queue.configure(map { "backend": "memory" })` — default, zero-config
- [ ] Job lifecycle: Scheduled → Pending → Active → Completed/Failed/Dead
- [ ] Retry with configurable backoff (exponential, linear, constant)
- [ ] Dead letter queue (jobs that exhaust retries)
- [ ] Job cancellation and timeout enforcement
- [ ] Priority queues (job-level and queue-level)
- [ ] Deduplication — `unique: 3600` skips if identical job already queued (SHA256)
- [ ] Job expiration — `expires: 5m` discards stale jobs
- [ ] Graceful shutdown (drain in-flight jobs)

#### Enqueue API
- [ ] `JobName.enqueue(args)` — immediate enqueue
- [ ] `JobName.enqueue_at(timestamp, args)` — schedule for specific time
- [ ] `JobName.enqueue_in(delay_seconds, args)` — schedule with delay
- [ ] `JobName.enqueue_batch(array_of_args)` — bulk enqueue (one round-trip)
- [ ] `Queue.cancel(job_id)` — cancel a pending job
- [ ] `Queue.status(job_id)` — check job status

#### Worker Modes
- [ ] `Queue.work_async()` — in-process alongside HTTP server
- [ ] `Queue.work(opts)` — blocking worker mode (separate process: `ntnt worker`)
- [ ] Configurable concurrency: `Queue.work(map { "concurrency": 10 })`
- [ ] Queue selection: `Queue.work(map { "queues": ["emails", "payments"] })`
- [ ] Weighted queues: `map { "critical": 5, "default": 3, "low": 1 }`

#### Streaming Logs (Core — not deferred)
- [ ] Workers emit structured JSON logs for every job event (enqueued, started, completed, failed, retried)
- [ ] `Queue.on_event(fn(event))` — hook for custom log handling
- [ ] Log format includes: job_id, job_type, queue, status, duration, error, attempt, timestamp
- [ ] Logs are streamable — foundation for `ntnt jobs tail` in Phase 6

#### Tests & Docs
- [ ] Tests for each feature
- [ ] STDLIB_REFERENCE.md updated
- [ ] Agent guide updated
- [ ] `ntnt docs --generate` passing

---

### Phase 4: KV Backend + Dashboard 📋

**Depends on:** Phase 3  
**Estimated effort:** 4-5 days

Persistent job storage and real-time visibility.

#### KV Backend
- [ ] `Queue.configure(map { "backend": "kv", "kv_url": env("KV_URL") })`
- [ ] Key layout implementation (see below)
- [ ] Job claiming via atomic KV operations (sorted set pop or set-if-not-exists)
- [ ] Worker heartbeats via KV TTL keys — auto-requeue on worker death (solves stuck jobs)
- [ ] Visibility timeout — configurable, default 5 minutes
- [ ] Delayed/scheduled jobs via sorted set (sorted by scheduled_at)
- [ ] Job history with automatic expiry (completed: 24h TTL, dead: 180 days)
- [ ] Distributed locking for cron via set-if-not-exists + TTL
- [ ] Automatic pruning of old completed/cancelled jobs
- [ ] Batch enqueue — single KV round-trip for N jobs
- [ ] Tests with both Redis and SQLite KV backends

#### KV Key Layout
```
jobs:queue:<queue>          = sorted set of job_ids (by priority + scheduled_at)
jobs:data:<job_id>          = { type, payload, status, attempts, created_at, ... }
jobs:active:<job_id>        = claimed job (TTL = visibility_timeout)
jobs:heartbeat:<worker_id>  = worker alive signal (TTL = 30s, refreshed continuously)
jobs:dead:<job_id>          = failed job (TTL = 180 days, capped at 10K)
jobs:completed:<job_id>     = completed job + result (TTL = 24h)
jobs:unique:<sha256>        = dedup key (TTL = unique duration)
jobs:lock:cron:<name>       = distributed lock for cron (TTL = interval)
jobs:stats:<queue>          = { pending, active, completed, failed, dead }
jobs:stats:ts:<queue>:<ts>  = time-bucketed stats (5-min buckets, 7-day retention)
```

#### Real-Time Dashboard
- [ ] `Queue.configure(map { "dashboard": true })` — adds `/jobs` routes
- [ ] Dashboard shows: pending/active/completed/failed/dead counts by queue
- [ ] Per-job-type breakdown with average duration
- [ ] Time-series charts (jobs/minute, error rate, queue depth over time)
- [ ] Live-updating (SSE or polling)
- [ ] Job detail view (payload, attempts, error, duration)
- [ ] Retry/cancel actions from dashboard
- [ ] **Security: localhost-only by default** (see [Dashboard Security Model](#dashboard-security-model))
- [ ] API key auth option for remote access
- [ ] App auth middleware integration option

#### Worker Health
- [ ] Worker registration in KV (worker_id, started_at, queues, last_heartbeat)
- [ ] `/jobs/workers` endpoint — which workers are alive, what they're processing
- [ ] Automatic dead worker detection (heartbeat TTL expiry)
- [ ] Stats: jobs processed per worker, average processing time

#### Time-Series Stats
- [ ] 5-minute bucketed stats per queue (jobs_completed, jobs_failed, avg_duration)
- [ ] 7-day retention with automatic pruning
- [ ] `Queue.stats(map { "period": "1h" })` — aggregated stats over time window
- [ ] Foundation for dashboard charts

---

### Phase 5: Production Hardening 📋

**Depends on:** Phase 3 (some items), Phase 4 (others)  
**Estimated effort:** 3-4 days

Safety features that make jobs production-ready and developer-friendly.

#### Simulation Mode (dry-run)
- [ ] `JobName.simulate(args)` — run the job without side effects
- [ ] `effect "sends email" { email.send(...) }` blocks — skipped in simulation
- [ ] Simulation output: what WOULD happen (functions called, data accessed, side effects)
- [ ] Estimated duration based on non-effect code
- [ ] `ntnt jobs simulate <type> --args='{"user_id": "123"}'` CLI
- [ ] Critical for development — prevents blowing out thousands of invalid jobs

```ntnt
Job SendEmail on emails {
    perform(user_id: String) {
        let user = db.find(user_id)  // runs in simulation (read-only)

        effect "sends email" {
            email.send(user.email, subject, body)  // SKIPPED in simulation
        }

        effect "updates analytics" {
            analytics.track("email_sent", user_id)  // SKIPPED in simulation
        }
    }
}

// Simulation output:
// ✓ db.find("123") → { email: "alice@example.com", name: "Alice" }
// ⏭ SKIPPED: sends email (email.send)
// ⏭ SKIPPED: updates analytics (analytics.track)
// Estimated duration: 0.1s
```

#### Job Contracts (extends ntnt's contract system)
- [ ] `requires(args) { ... }` — precondition checked before job runs
- [ ] `ensures(args, result) { ... }` — postcondition checked after job completes
- [ ] Contract violations → job fails with clear error (not silently wrong)
- [ ] Static analysis: `ntnt lint` warns on contract issues

```ntnt
Job ProcessPayment on payments {
    requires(args) {
        args["amount"] > 0 && args["order_id"] != ""
    }

    ensures(args, result) {
        result["status"] in ["charged", "declined", "pending"]
    }

    perform(order_id: String, amount: Float) -> Map {
        let charge = stripe.charge(amount)
        return map { "status": charge.status }
    }
}
```

#### Intent Verification (IDD-native — table stakes)
- [ ] Jobs testable in `.intent` files — same as HTTP routes
- [ ] `ntnt intent check` verifies job behavior against specs
- [ ] Job intent specs: given args + mocks → assert status + side effects

```intent
Feature: Welcome Email Job
  id: feature.welcome_email_job

  test:
    - job: SendWelcomeEmail
      args: { "user_id": "123" }
      given:
        - mock db.find_user returns { "id": "123", "email": "test@example.com" }
        - mock email.send returns { "sent": true }
      assert:
        - status: completed
        - email.send was called with "test@example.com"

    - job: SendWelcomeEmail
      args: { "user_id": "invalid" }
      given:
        - mock db.find_user throws "User not found"
      assert:
        - status: failed
        - error contains "User not found"
```

#### Additional Hardening
- [ ] Rate limiting — `rate: 100/minute` per job type
- [ ] Concurrency limits — `concurrency: 5` per job type
- [ ] Queue pause/resume — `Queue.pause("emails")` / `Queue.resume("emails")`
- [ ] Cron expressions — `schedule("0 9 * * MON-FRI", fn)` with distributed lock
- [ ] Dead job caps — 10K max, auto-prune oldest
- [ ] Idempotency — `idempotent: true` with key-based dedup

---

### Phase 6: Observability CLI 📋

**Depends on:** Phase 3 (streaming logs), Phase 4 (KV backend)  
**Estimated effort:** 2-3 days

Developer tools for monitoring and debugging jobs from the terminal.

- [ ] `ntnt jobs status` — summary of all queues (pending/active/completed/failed/dead)
- [ ] `ntnt jobs list [--pending|--failed|--dead|--queue=<name>]` — filter jobs by status/queue
- [ ] `ntnt jobs inspect <job-id>` — full job details (payload, attempts, error, duration, history)
- [ ] `ntnt jobs retry <job-id>` — retry a failed/dead job
- [ ] `ntnt jobs cancel <job-id>` — cancel a pending/scheduled job
- [ ] `ntnt jobs tail [--queue=<name>] [--status=failed] [--since=1h]` — streaming log view with filters
- [ ] `ntnt jobs replay <job-id> [--dry-run]` — re-run with same inputs (dry-run uses simulation mode)
- [ ] `ntnt jobs clear --dead [--older-than=7d]` — clear old dead/completed jobs
- [ ] `ntnt jobs workers` — list active workers with health status
- [ ] `--format=agent` option for LLM-optimized compact output

---

### Deferred Items (not blocking any phase)

- [ ] **Closure capture DX** — explicit capture syntax: `spawn(capture: [x, y], fn() { ... })`. Error messages already good.
- [ ] **Schedule string validation** — lint warning for unparseable intervals. Pure polish.
- [ ] **Configurable reaper TTL** — currently 5min hardcoded. Low priority.
- [ ] **Bounded channels** — `channel(capacity)` for backpressure. crossbeam supports it, just needs exposure.
- [ ] **Threadpool for job workers** — thread-per-task may not scale for thousands of jobs. Consider for Phase 4+.
- [ ] **Progress reporting** — `Job.progress(0.5)` for long-running jobs. Nice for dashboard.
- [ ] **Middleware/plugins** — `Queue.use(fn(job, next) { ... })` for cross-cutting concerns.

---

### Phase 7: Event Dispatch (`std/events`) 📋

**Depends on:** Phase 3 + Phase 4 (job system)
**Estimated effort:** 2-3 days

A thin pub/sub event dispatch layer built *on top of* the job system. This is not a message broker — it's an event router. When you publish an event, `std/events` enqueues all subscribed jobs. Durability, retry, workers, and observability all come for free from the job system underneath.

**The gap `std/events` fills:** right now, if a user signs up and you need to send a welcome email AND update analytics, the call site has to call `enqueue()` twice. It's coupled to the consumer list. `std/events` decouples emitter from consumers — the call site publishes the event, and any number of jobs can subscribe to it independently.

#### Core API

```ntnt
import { subscribe, publish, unsubscribe, on_event } from "std/events"

// Subscribe a job to an event — any publish fires the job
subscribe("user.signed_up", "SendWelcomeEmail")
subscribe("user.signed_up", "UpdateAnalytics")
subscribe("payment.processed", "SendReceipt")

// Publish — enqueues all subscribed jobs with the payload
publish("user.signed_up", map { "user_id": "123", "email": "alice@example.com" })
// → enqueues SendWelcomeEmail + UpdateAnalytics jobs

// Subscribe an inline handler (synchronous, no job overhead — fires in current thread)
on_event("payment.failed", fn(event) {
    log_warn("Payment failed", event)
})

// Unsubscribe a specific job from an event
unsubscribe("user.signed_up", "UpdateAnalytics")

// Unsubscribe all handlers from an event
unsubscribe("user.signed_up")
```

- `subscribe(event, job_name)` — registers a job as a consumer; `publish` calls `enqueue(job_name, payload)` for each
- `publish(event, payload)` — fan-out: iterates subscribers, enqueues each job
- `on_event(event, fn)` — synchronous inline handler, runs on the publishing thread (not a job)
- `unsubscribe(event, job_name?)` — remove one subscriber or all

#### Event Naming Convention

Events use dot-namespaced strings: `"domain.action"` (e.g. `"user.signed_up"`, `"payment.processed"`, `"order.shipped"`). No schema enforcement at this phase — strings are the schema. Convention: past tense (`signed_up`, not `sign_up`).

#### Queue Routing

By default, subscribed jobs run on their own declared queue. No extra configuration needed:

```ntnt
job SendWelcomeEmail on emails (retry: 3) {
    perform(user_id, email) { ... }
}

subscribe("user.signed_up", "SendWelcomeEmail")
// → publish fires enqueue("SendWelcomeEmail", payload) → runs on the "emails" queue
```

If you want a specific event's jobs to run on a dedicated queue, define the job that way — events stay out of routing decisions.

#### Backend Configuration

**Memory (default):** Subscription registry is in-process. Works for single-process ntnt apps — the common case.

```ntnt
// Default — no configuration needed
subscribe("user.signed_up", "SendWelcomeEmail")
publish("user.signed_up", payload)
```

**Redis (cross-process fan-out):** For multi-process deployments where the publisher and consumer run as separate ntnt processes (e.g. a web server and a worker process). Uses Redis pub/sub for the event signal; the receiving process handles `enqueue`.

```ntnt
import { configure_events } from "std/events"

configure_events(map { "store": "redis://localhost:6379" })
// Publisher: publish() → Redis PUBLISH
// Consumer process: subscribes to Redis channel, calls enqueue() on arrival
```

This is an explicit opt-in. Most apps don't need it — the memory backend is correct when publish and consume happen within the same process.

#### Testing Mode

Events work naturally with the job system's testing mode. Set `configure_queue(map { "mode": "testing" })`, then publish events and assert jobs were enqueued:

```ntnt
import { configure_queue, assert_enqueued, clear_jobs } from "std/jobs"
import { subscribe, publish } from "std/events"

configure_queue(map { "mode": "testing" })
subscribe("user.signed_up", "SendWelcomeEmail")
subscribe("user.signed_up", "UpdateAnalytics")

publish("user.signed_up", map { "user_id": "123" })

assert_enqueued("SendWelcomeEmail", map { "user_id": "123" })
assert_enqueued("UpdateAnalytics", map { "user_id": "123" })
clear_jobs()
```

For convenience, two additional assertions surface directly in `std/events`:

```ntnt
import { assert_published, assert_not_published } from "std/events"

// Assert that publish() was called with this event
assert_published("user.signed_up", map { "user_id": "123" })
assert_not_published("payment.processed")
```

These check an in-memory publish log (similar to the job test queue). Useful when you want to verify event emission without caring about which jobs are subscribed.

#### Implementation Notes

- **`src/stdlib/events.rs`** — new module (~200 lines)
- Event registry: `LazyLock<Mutex<HashMap<String, Vec<EventSubscriber>>>>` where `EventSubscriber` is `Job(String)` or `Handler(fn)`
- Publish log (testing mode): `LazyLock<Mutex<Option<Vec<PublishedEvent>>>>`
- `publish()` iterates subscribers, calls `enqueue_internal()` for job subscribers, calls fn directly for `on_event` handlers
- `configure_events(map { "store": "redis://..." })` sets up a Redis pub/sub listener thread; on receive → `enqueue_internal()`
- Subscriptions registered at module load time (before the HTTP server starts) — same pattern as job registration

#### Deferred to Later

- **Wildcard subscriptions** — `subscribe("user.*", "AuditLog")` matches all `user.` events. Useful but adds regex overhead.
- **Event schemas** — typed payloads with compile-time checking. Requires a new type in the type system.
- **Event sourcing patterns** — event log replay, projections. A different design problem; not blocking the router.
- **Cross-language event interop** — publishing from a non-ntnt service into the event bus. Probably a Redis pub/sub contract; design when needed.

#### Why Not a Full Message Broker

ntnt's target is single-process apps behind a Cloudflare tunnel. A broker (Kafka, RabbitMQ, NATS) adds: a separate process to run, a protocol to speak, ordering guarantees that need a log, consumer group semantics, offset management. None of that is needed for "when user signs up, fire these jobs." The job system already handles durability, retry, and ordering within a queue. `std/events` is the glue that maps events to jobs — the broker is already there.

If someone needs genuine cross-service distributed pub/sub at scale, that's a Redis pub/sub contract with `configure_events(map { "store": "redis://..." })`. ntnt shouldn't pretend to be Kafka.

---

## Dashboard Security Model

The job dashboard is a powerful tool that must not become an attack surface.

### Three Security Levels

**Level 1: Localhost Only (Default)**
```ntnt
Queue.configure(map { "dashboard": true })
// Dashboard at /jobs — ONLY accessible from 127.0.0.1
// Requests from any other IP get 403
// This is already how ntnt apps deploy (behind Cloudflare tunnel, no exposed ports)
```

A lazy admin literally cannot leak this. The dashboard only responds to localhost. Since ntnt apps run behind Cloudflare tunnels with no port exposure, the dashboard is unreachable from the internet by default.

**Level 2: API Key (Remote Access)**
```ntnt
Queue.configure(map {
    "dashboard": true,
    "dashboard_key": env("JOBS_DASHBOARD_KEY")
})
// Requires ?key=<secret> query param or Authorization: Bearer <secret> header
// Key comes from env var — never hardcoded
```

For accessing the dashboard through a tunnel or VPN. Explicit opt-in.

**Level 3: App Auth Integration (Full Auth)**
```ntnt
Queue.configure(map {
    "dashboard": true,
    "dashboard_auth": admin_only_middleware
})
// Uses your app's existing auth system
// Dashboard routes pass through the middleware before rendering
// Supports session auth, OAuth, API keys — whatever the app uses
```

For production apps with existing user auth. The dashboard becomes another admin route protected by the same auth as the rest of the app.

### Security Rules
- **Default is localhost-only.** No configuration needed, no risk.
- **No dashboard without explicit opt-in.** `"dashboard": true` must be set.
- **Remote access requires auth.** Level 2 or 3 — never "dashboard: true" with remote access and no key.
- **If `dashboard: true` and no auth and not localhost → startup warning.** The runtime warns: "⚠️ Job dashboard is accessible without authentication. Set dashboard_key or dashboard_auth."
- **Dashboard is read-only by default.** Retry/cancel actions require Level 2+ auth.

---

## Lessons Learned

### From Building the System (Phases 0-1) + Josh's PHP Experience

1. **Primitives before patterns.** We built the job system before hardening spawn/channels. The job system inherited every rough edge. Fix the foundation first.

2. **The parser IS the registry.** PHP requires handler classes + service container registration. ntnt's `Job` syntax makes declaration and registration the same step. Massive DX win.

3. **Stuck jobs are a heartbeat problem.** Josh's PHP system has stuck jobs that need manual clearing. Heartbeat TTL keys solve this automatically — worker dies, heartbeat expires, job re-queues. Zero manual intervention.

4. **Streaming logs are core, not an afterthought.** Real-time log aggregation across workers is essential for production debugging. Build it into the event system from day one.

5. **Simulation prevents catastrophe.** Blowing out thousands of invalid jobs is scary. Dry-run mode with `effect` blocks lets you test the job logic without side effects before committing to production enqueue.

6. **Intent verification is table stakes.** ntnt is IDD. Jobs without intent specs are untested. Make job testing as natural as route testing.

7. **Job chaining is just code.** No need for a `Chain` DSL — when a job completes, enqueue the next one. Explicit, debuggable, no magic.

8. **`cargo fmt` before push.** CI caught formatting twice. Always run it.

9. **`ntnt docs --generate` after stdlib changes.** CI drift check requires docs to match source annotations.

10. **Use your own abstractions.** We built `std/kv` for Redis/SQLite. The job system should use it, not reimplement raw Redis Streams.

11. **Copilot review catches real bugs.** Across 2 review rounds (16 comments), it found: select() busy-loop on closed channels, consumed task memory leak, lock discipline violation, NativeFunction ambiguity. These are not style nits — they're correctness bugs that would have hit production.

12. **Reply and resolve Copilot comments immediately.** Don't let review comments pile up. Fix, reply with the commit hash, resolve the thread. Keeps the PR clean and reviewable.

13. **Read the credentials file format.** The GitHub PAT is in a key-value file (`GITHUB_PAT=ghp_...`), not a bare token. Wasted time on "Bad credentials" because of wrong parsing. Check file format before assuming.

### Architecture Decisions (Resolved)

| Decision | Choice | Why |
|----------|--------|-----|
| Thread model | Thread-per-task | Simple, debuggable, OS threads give true parallelism |
| Panic isolation | catch_unwind per task | Essential — one bad job can't kill the worker |
| Cancellation | Cooperative (flag + yield points) | Simpler than preemption, we control yield points |
| ID generation | Monotonic AtomicU64 | No UUID overhead, no collisions within process |
| Channel library | crossbeam-channel | Enables `select`, drop-in replacement for mpsc |
| Handle types | Value::TaskHandle(u64) etc. | Compile-time exhaustiveness, no accidental construction |
| Job backend | Memory + KV (not raw PG/Redis) | One implementation, multiple stores |
| Job chaining | Application logic (not DSL) | Explicit > magic |
| Dashboard auth | Localhost-only default | Secure by default, opt-in escalation |
| NativeFunction capture | Don't capture; child uses own stdlib | Avoids name ambiguity across modules |
| Handle serialization | Capturable at spawn, not sendable via channels | spawn(fn() { send(ch,...) }) works; send(ch, another_ch) doesn't |
| Reaper scope | All terminal states incl. Consumed | Prevents memory leak in long-running servers |

---

## Competitive Analysis

### The Full Picture

| Feature | ntnt (planned) | Sidekiq | Oban | BullMQ | Josh's PHP |
|---------|---------------|---------|------|--------|-----------|
| Job definition | `Job X on q { }` | Ruby class + mixin | Elixir module + callback | JS class | PHP class + interface + registry |
| Lines to define a job | ~5 | ~20 | ~15 | ~20 | ~40 |
| Configuration | 1 line | YAML + initializer | Ecto config + migrations | JS config | Docker compose + env |
| Backend flexibility | Memory/Redis/SQLite/Valkey | Redis only | PG only | Redis only | Redis only |
| Dashboard | Built-in, secure | Built-in (open) | $49/mo (Oban Web) | Bull Board (separate) | Custom-built |
| Unique jobs | `unique: 3600` | $250/mo (Enterprise) | Free | Free | Custom code |
| Simulation/dry-run | `effect` blocks | ❌ | ❌ | ❌ | ❌ |
| Job contracts | `requires`/`ensures` | ❌ | ❌ | ❌ | ❌ |
| Intent testing | `.intent` files | RSpec (manual) | ExUnit (manual) | Jest (manual) | PHPUnit (manual) |
| Streaming logs | `ntnt jobs tail` | Manual | Manual | Manual | Docker logs + grep |
| Stuck job recovery | Automatic (heartbeat) | Automatic | Automatic | Automatic | **Manual clearing** |
| Batch enqueue | `.enqueue_batch()` | `.perform_bulk()` | `Oban.insert_all()` | `.addBulk()` | Custom |
| Scaling | Same binary, add processes | Separate process | Built into Phoenix | Separate process | Docker containers |
| Event-driven dispatch | `std/events` subscribe/publish | `ActiveSupport::Notifications` (no durability) | ❌ (pub/sub separate) | ❌ (separate) | Custom event bus |

**ntnt wins on:** job definition simplicity, backend flexibility, simulation mode, job contracts, intent testing, dashboard (free + secure), streaming logs.

**What ntnt needs to match:** Sidekiq's raw throughput (Redis Streams), Oban's transactional guarantees (PG ACID). KV backend trades these for simplicity and flexibility — good tradeoff for 99% of apps.

---

## Open Questions

### Resolved

| Question | Decision | Date |
|----------|----------|------|
| Thread model for primitives | Thread-per-task | 2026-03-15 |
| async/await in ntnt? | No. Synchronous + spawn + channels. | 2026-03-15 |
| Job DSL as syntax vs library? | Syntax (`Job` keyword). | 2026-03-15 |
| One PR or many? | One per phase. | 2026-03-16 |
| crossbeam vs std::mpsc? | crossbeam. Enables select. | 2026-03-16 |
| Handle types? | Value variants (TaskHandle, ChannelHandle, ScheduleHandle). | 2026-03-16 |
| select return format? | `{channel, value}` map. `{status: "timeout"}` on timeout. | 2026-03-16 |
| PG+Redis backends or KV? | KV only (via std/kv). | 2026-03-16 |
| Job chaining approach? | Application logic, not DSL. | 2026-03-16 |
| Dashboard auth? | Localhost-only default, opt-in API key or app auth. | 2026-03-16 |
| Job contracts? | Yes — extend ntnt's existing contract system. | 2026-03-16 |
| Simulation mode? | Yes — `effect` blocks, critical for safe development. | 2026-03-16 |
| Intent verification? | Yes — table stakes for IDD language. | 2026-03-16 |
| AI diagnosis (ntnt jobs ask/diagnose)? | No — cut. Not a good fit. | 2026-03-16 |
| NativeFunction capture strategy? | Don't capture. Child interpreter uses own stdlib. Ambiguity detection for edge cases. | 2026-03-16 |
| Handle serialization? | Capturable at spawn time (closure capture). NOT sendable through channels. | 2026-03-16 |
| Shutdown on eval error? | Unconditional. Capture result, shutdown, then propagate. | 2026-03-16 |

### Open

| Question | Options | Notes |
|----------|---------|-------|
| Reaper TTL configurable? | Env var, runtime config, or hardcoded | Currently 5min hardcoded |
| Job workers: thread-per-task or pool? | Pool (for throughput) | Phase 3+ decision |
| KV claiming strategy? | Sorted set pop vs set-if-not-exists vs BRPOP | Backend-dependent |
| Where do composition functions live? | Rust (fast) vs ntnt stdlib (extensible) | Probably both |
| Dashboard SSE vs polling? | SSE (real-time) vs polling (simpler) | Leaning SSE |
| Time-series bucket size? | 1min vs 5min vs configurable | Leaning 5min default |
| `effect` block implementation? | Compile-time flag vs runtime flag | Needs design |

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-15 | Initial implementation: Phases 0-6 built across 6 feature branches |
| 2026-03-15 | v2 rewrite: stripped job system, rebuilt concurrency primitives clean |
| 2026-03-16 | DD-037 v3: comprehensive roadmap with DX hardening plan |
| 2026-03-16 | Phase 1 complete: handle types, try_await states, select() — `f63f23b` |
| 2026-03-16 | DD-037 v4: KV backend replaces PG+Redis. Checkboxes for tracking. |
| 2026-03-16 | DD-037 v5: Dashboard security model. Simulation, contracts, intent verification promoted. Job chaining via application logic. Streaming logs promoted to core. Batch enqueue added. AI diagnosis cut. Competitive analysis updated with Josh's PHP system. |
| 2026-03-16 | Copilot review fixes: `c2b2685` — select() busy-loop, consumed task leak, reaper lock discipline, NativeFunction ambiguity, typechecker sigs, doc wording. All 16 comments resolved. |
| 2026-03-16 | DD-037 v6: Updated Phase 1 with full Copilot review resolution details. Added lessons 11-13. |
| 2026-03-17 | DD-037 v7: Added Phase 7 — Event Dispatch (`std/events`). Pub/sub fan-out over the job system. Memory + Redis backends. Testing mode integration. |
