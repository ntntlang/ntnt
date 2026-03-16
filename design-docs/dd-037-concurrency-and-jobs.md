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
4. [Roadmap — What's Coming](#roadmap--whats-coming)
5. [Phase Details](#phase-details)
6. [Lessons Learned](#lessons-learned)
7. [Competitive Analysis](#competitive-analysis)
8. [Open Questions](#open-questions)
9. [Appendix: Job DSL Design](#appendix-job-dsl-design)

---

## Vision

ntnt needs two things:

1. **Concurrency primitives** — spawn tasks, communicate between them, schedule recurring work. The Go-inspired CSP model, but with ntnt's simplicity.
2. **Background job system** — persistent, reliable, production-grade job queuing with declarative syntax. The Sidekiq/Oban of the ntnt world, but backend-agnostic and language-native.

These are layered: primitives first (they're useful alone), jobs built on top.

**Design principles:**
- **Zero-config start** — `spawn()` just works, no setup
- **CSP by architecture** — `Rc<RefCell>` forces serialization at task boundaries. Two tasks literally cannot share memory. Go says "don't share memory"; ntnt physically can't.
- **No async/await** — synchronous model, no function coloring. `spawn` + channels achieve the same results.
- **Declarative jobs** — `Job SendEmail on emails { perform(id) { ... } }` reads like documentation
- **Two backends** — Memory (dev/simple) and KV (production) via `std/kv`. KV already supports Redis, SQLite, and any Redis-compatible store. No raw PostgreSQL backend — use KV's abstraction.

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
- panic isolation (catch_unwind per task)

### Why KV Instead of Raw PostgreSQL/Redis

ntnt already has `std/kv` — a key-value module that works with Redis, SQLite, and Redis-compatible stores (Valkey, DragonflyDB, etc.):

```ntnt
let kv = unwrap(open("redis://localhost:6379"))   // Redis / Valkey / Dragonfly
let kv = unwrap(open("sqlite:./jobs.db"))          // SQLite (single-node)
```

Building the job backend on `std/kv` means:
- **One implementation, multiple stores.** No separate PG backend and Redis backend to maintain.
- **Users choose their store.** Same API whether you're using Redis in prod or SQLite in dev.
- **KV operations map naturally to job queues.** Set with TTL = delayed jobs. List with prefix = queue listing. Atomic get+delete = job claiming.
- **No SQL schema to manage.** KV is schemaless — the job system owns its key layout.

What we lose vs raw PostgreSQL:
- No `SELECT FOR UPDATE SKIP LOCKED` (but KV's atomic operations + Lua scripts on Redis achieve the same thing)
- No transactional enqueue across application tables (but most apps don't need this — enqueue in an `after_commit` hook instead)
- No advisory locks for cron (but distributed locks via KV's `set-if-not-exists` + TTL work)

The tradeoff is worth it: simpler codebase, fewer dependencies, same production reliability.

### SerializedValue

Thread-safe enum for crossing task boundaries:

| ntnt Type | Serialized? | Notes |
|-----------|-------------|-------|
| Int, Float, Bool, String | ✅ | Direct mapping |
| Array, Map | ✅ | Recursive serialization |
| Struct | ✅ | Via `__type` marker in Map |
| EnumValue | ✅ | Via `__enum` marker in Map |
| Function | ❌ | Cannot cross boundaries — error with names |
| NativeFunction | ✅ | Re-injected by name in fresh interpreter |
| Result, Option | ✅ | Via Ok/Err/Some/None wrappers |
| TaskHandle, ChannelHandle, ScheduleHandle | ❌ | Process-local, cannot cross boundaries |

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
- [x] `try_await(task) -> Map` — non-blocking peek with status
- [x] `cancel_task(task) -> Bool` — cooperative cancellation
- [x] `channel() -> ChannelHandle` — create unbounded crossbeam channel
- [x] `send(ch, value) -> Bool` — send value (false if closed)
- [x] `recv(ch) -> Value` — blocking receive
- [x] `recv_timeout(ch, ms) -> Option` — receive with timeout
- [x] `try_recv(ch) -> Option` — non-blocking receive
- [x] `close(ch) -> Bool` — close channel
- [x] `select(channels, timeout?) -> Map` — multi-channel wait
- [x] `schedule(interval, fn()) -> ScheduleHandle` — recurring execution
- [x] `cancel_schedule(sched) -> Bool` — stop recurring execution
- [x] `after(delay, fn()) -> TaskHandle` — delayed one-shot execution
- [x] `sleep_ms(ms) -> Unit` — cancellation-aware sleep
- [x] `thread_count() -> Int` — available CPU threads

### ✅ Phase 1: Primitive Hardening

**Commit:** `f63f23b` on `feat/concurrency-v2`  
**Status:** Complete. CI green.

- [x] **Opaque handle types** — `Value::TaskHandle(u64)`, `Value::ChannelHandle(u64)`, `Value::ScheduleHandle(u64)` replace Map-based handles. Type-safe at the Value level.
- [x] **`try_await` consumed/expired states** — `TaskState::Consumed` and `TaskState::Expired`. `await_task` marks Consumed, reaper marks Expired. `try_await` never errors for handles that existed.
- [x] **`select()` with crossbeam-channel** — Replaced all `std::sync::mpsc` with `crossbeam::channel`. `select([ch_a, ch_b], 5000)` uses `crossbeam::Select` with 100ms cancellation-aware slices.
- [x] 37 concurrency tests passing (11 new)
- [x] CI green (`cargo fmt`, `cargo test --locked`, `cargo build --locked`)

---

## Roadmap — What's Coming

### Overview

```
Phase 0  ✅ Primitives (spawn, channels, schedule, after, select)    DONE
Phase 1  ✅ Primitive Hardening (try_await, handles, select)          DONE
Phase 2  📋 Composition Layer (parallel, race, task groups)
Phase 3  📋 Job DSL (declarative jobs, in-memory backend)
Phase 4  📋 KV Backend (persistent jobs via std/kv)
Phase 5  📋 Polish & Production Features
Phase 6  📋 Observability & CLI
Phase 7  📋 Agent-First Features
```

### Phase Status Table

| Phase | Name | Status | Branch | Tests | Priority |
|-------|------|--------|--------|-------|----------|
| 0 | Primitives | ✅ Done | `feat/concurrency-v2` | 26 | — |
| 1 | Primitive Hardening | ✅ Done | `feat/concurrency-v2` | 37 | — |
| 2 | Composition Layer | 📋 Planned | TBD | — | P1 — high value |
| 3 | Job DSL | 📋 Planned | TBD | — | P1 — core feature |
| 4 | KV Backend | 📋 Planned | TBD | — | P1 — production req |
| 5 | Polish & Production | 📋 Planned | TBD | — | P2 — hardening |
| 6 | Observability & CLI | 📋 Planned | TBD | — | P2 — DX |
| 7 | Agent-First Features | 📋 Planned | TBD | — | P3 — differentiator |

---

## Phase Details

### Phase 2: Composition Layer 📋

**Depends on:** Phase 1 ✅  
**Estimated effort:** 2-3 days

- [ ] `parallel(fns) -> Array` — run N functions, collect all results, cancel on first error
- [ ] `race(fns) -> Value` — run N functions, return first result, cancel others
- [ ] `task_group(fn(group))` — structured scope, all tasks cancelled when scope exits
- [ ] `pipeline(fns) -> Value` — chain: output of fn1 → input of fn2 → ...
- [ ] Tests for each composition primitive
- [ ] Documentation in STDLIB_REFERENCE.md
- [ ] Agent guide updated

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
    // Block waits for all tasks to complete
})
```

These can potentially be implemented in ntnt itself (stdlib .tnt files) once the primitives are solid. Or thin Rust wrappers for performance.

---

### Phase 3: Job DSL 📋

**Depends on:** Phase 1 ✅ (solid primitives)  
**Estimated effort:** 3-5 days (rebuilding from preserved branches)  
**Source:** `feat/job-dsl` branch (preserved)

Rebuild the Job DSL on the hardened primitive foundation.

- [ ] `Job Name on queue { perform(args) { ... } }` parser syntax
- [ ] In-memory backend (default, zero-config)
- [ ] Retry with configurable backoff (exponential, linear, constant)
- [ ] Dead letter queue
- [ ] Job cancellation and timeout
- [ ] Priority queues (job-level)
- [ ] `Queue.configure()` for backend selection
- [ ] `Queue.stats()` for monitoring
- [ ] `Queue.work()` / `Queue.work_async()` worker modes
- [ ] Graceful shutdown (drain in-flight jobs)
- [ ] Job lifecycle: Scheduled → Pending → Active → Completed/Failed/Dead
- [ ] `on_failure(error, attempt)` hook
- [ ] Doc comment metadata (`/// Triggers:`, `/// Affects:`)
- [ ] Tests for each feature
- [ ] Documentation in STDLIB_REFERENCE.md

```ntnt
Job SendWelcomeEmail on emails {
    perform(user_id: String) {
        let user = db.find_user(user_id)
        email.send(user.email, "Welcome!", body)
    }
}

// Enqueue
SendWelcomeEmail.enqueue(map { "user_id": "123" })

// With options
Job ProcessPayment on payments (retry: 5, timeout: 120s) {
    perform(order_id: String, amount: Float) {
        let order = db.find(order_id)
        stripe.charge(order.customer_id, amount)
    }

    on_failure(error, attempt) {
        alert.notify("Payment failed: #{error}")
    }
}
```

**Preserved from original work:**
- Parser support for Job keyword (in `feat/job-dsl`)
- In-memory backend with full lifecycle
- 10 job-specific tests passing

---

### Phase 4: KV Backend 📋

**Depends on:** Phase 3  
**Estimated effort:** 3-4 days

Persistent job storage using ntnt's `std/kv` module. Works with any KV store that `std/kv` supports: Redis, SQLite, Valkey, DragonflyDB, etc.

- [ ] `Queue.configure(map { "backend": "kv", "kv_url": env("KV_URL") })`
- [ ] Key layout design (e.g., `jobs:pending:<queue>:<id>`, `jobs:active:<id>`, `jobs:dead:<id>`)
- [ ] Job claiming via atomic KV operations (get + delete / set-if-not-exists)
- [ ] Worker heartbeats via KV TTL keys (`jobs:heartbeat:<worker_id>`)
- [ ] Visibility timeout — re-enqueue when heartbeat key expires
- [ ] Delayed/scheduled jobs via sorted set or TTL polling
- [ ] Job history with automatic expiry
- [ ] Distributed locking for cron via `set-if-not-exists` + TTL
- [ ] Automatic pruning of completed/cancelled jobs
- [ ] Per-queue statistics via KV counters
- [ ] Tests with both Redis and SQLite backends
- [ ] Graceful fallback if KV connection lost (queue to memory, drain on reconnect)

```ntnt
import { Queue } from "std/jobs"

// Redis in production
Queue.configure(map {
    "backend": "kv",
    "kv_url": env("KV_URL", "redis://localhost:6379")
})

// SQLite for simple deploys
Queue.configure(map {
    "backend": "kv",
    "kv_url": "sqlite:./data/jobs.db"
})

// Still works with zero config (in-memory)
Queue.configure(map { "backend": "memory" })
```

**Key layout (draft):**
```
jobs:queue:<queue_name>:<job_id>   = { payload, status, attempts, created_at, ... }
jobs:pending:<queue_name>          = sorted set by priority/scheduled_at
jobs:active:<job_id>               = claimed job data
jobs:heartbeat:<worker_id>         = TTL key (30s), worker health
jobs:dead:<job_id>                 = failed job data (180-day TTL)
jobs:completed:<job_id>            = completed job data (24h TTL)
jobs:stats:<queue_name>            = { pending, active, completed, failed, dead }
jobs:unique:<hash>                 = dedup key with TTL
jobs:lock:cron:<schedule_name>     = distributed lock for cron schedules
```

**Why this works:**
- `std/kv` `set` with `ttl` option = automatic job expiry and heartbeats
- `std/kv` `list` with prefix = queue listing
- `std/kv` `has` + `set` = atomic claiming (Redis `SETNX`, SQLite `INSERT OR IGNORE`)
- `std/kv` `del` = job completion/cleanup
- Same code path for Redis (production) and SQLite (single-node/dev)

---

### Phase 5: Polish & Production Features 📋

**Depends on:** Phases 3-4  
**Estimated effort:** 2-3 days  
**Source:** `feat/job-dsl-polish` branch (preserved, needs adaptation from PG/Redis to KV)

- [ ] Unique jobs — SHA256 dedup with TTL: `unique: 3600`
- [ ] Cron expressions — `schedule("0 9 * * MON-FRI", fn)` with distributed lock via KV
- [ ] Dead job caps — 10K max, 180-day retention, auto-prune
- [ ] Queue pause/resume — `Queue.pause("name")` / `Queue.resume("name")`
- [ ] Weighted queue processing — `{ "critical": 5, "default": 3, "low": 1 }`
- [ ] Job expiration — `expires: 5m` (discard stale jobs)
- [ ] Rate limiting — `rate: 100/minute` per job type
- [ ] Concurrency limits — `concurrency: 5` per job type
- [ ] Idempotency support — `idempotent: true` with key-based dedup

---

### Phase 6: Observability & CLI 📋

**Depends on:** Phase 3  
**Estimated effort:** 2-3 days

- [ ] `ntnt jobs status` — summary of all queues
- [ ] `ntnt jobs list [--pending|--failed|--dead]` — filter by status
- [ ] `ntnt jobs inspect <job-id>` — full job details
- [ ] `ntnt jobs retry <job-id>` — retry a failed/dead job
- [ ] `ntnt jobs cancel <job-id>` — cancel a pending job
- [ ] `ntnt jobs tail [--queue=<name>]` — live streaming
- [ ] `ntnt jobs replay <job-id> [--dry-run]` — re-run with same inputs
- [ ] `Queue.stats()` programmatic API
- [ ] Optional `/jobs/status` HTTP endpoint (localhost only)
- [ ] `--format=agent` for LLM-optimized output
- [ ] IDD integration — job testing in `.intent` files

---

### Phase 7: Agent-First Features 📋

**Depends on:** Phase 6  
**Estimated effort:** Ongoing

The differentiator. No other job system has this.

- [ ] Semantic job metadata — `/// Triggers: user.created` parsed from doc comments
- [ ] `ntnt jobs ask "why are emails failing?"` — natural language queries
- [ ] `ntnt jobs diagnose <job-id>` — AI-powered root cause analysis
- [ ] Auto-generated test suggestions from job code
- [ ] Impact analysis — "If SendEmail fails, what's affected?"
- [ ] Simulation mode — dry-run with `effect` blocks
- [ ] Intent verification — did the job *achieve its purpose*, not just run?
- [ ] Job contracts — `requires(args) { ... }` / `ensures(args, result) { ... }`

---

### Deferred Items (not blocking any phase)

- [ ] **Closure capture DX** — explicit capture syntax: `spawn(capture: [x, y], fn() { ... })`. Error messages are already good. Syntax addition deserves its own design.
- [ ] **Schedule string validation** — lint warning for unparseable intervals. Works fine today, pure polish.
- [ ] **Configurable reaper TTL** — currently hardcoded at 5 minutes. Add env var or `ConcurrencyRuntime.configure()`.
- [ ] **Bounded channels** — `channel(capacity)` for backpressure. crossbeam supports this already, just need to expose it.
- [ ] **Threadpool for job workers** — thread-per-task works for primitives but may not scale for job workers processing thousands of jobs. Consider `rayon` or custom pool.

---

## Lessons Learned

### From Building Phases 0-1 (First + Second Pass)

1. **Primitives before patterns.** We built the job system before hardening spawn/channels. That's backwards. The job system inherited every primitive rough edge. Fix the foundation first.

2. **Monolith then modularize.** The initial `jobs.rs` grew to 4,483 lines before we refactored into modules. Starting with modules would have been premature — you don't know the right boundaries until you've built the thing. But don't wait too long.

3. **Copilot review is genuinely useful.** Across 4 review rounds, it caught: brace-depth tracking gaps, non-deterministic test modes, missing struct field error handling, indentation edge cases. Not a replacement for human review, but a great first pass.

4. **Test count is a vanity metric.** We had 1,063 tests but most were job-specific. The 37 concurrency primitive tests are more valuable because they test the foundation everything else depends on.

5. **`Rc<RefCell>` as a feature, not a limitation.** The interpreter's single-threaded design forces CSP. We leaned into this with serialized capture and it's genuinely better than trying to make the interpreter thread-safe.

6. **Capture errors are the #1 DX issue.** Users will write `spawn(fn() { my_helper(x) })` and not understand why `my_helper` can't cross the thread boundary. The error message is good but the mental model is surprising.

7. **Feature branches as preservation.** All 6 original branches still exist with working code. The v2 rewrite can cherry-pick proven implementations rather than rewriting from scratch.

8. **`cargo fmt` before push.** CI failed on formatting. Sub-agents don't always run rustfmt. Add it to the commit checklist.

9. **Use your own abstractions.** We built `std/kv` to abstract over Redis/SQLite. The job system should use it instead of reimplementing raw Redis Streams and PostgreSQL polling. Less code, fewer backends to maintain, same reliability.

### Architecture Decisions That Held Up

- **Thread-per-task** (not threadpool) — simpler, debuggable, good enough for ntnt's use case
- **catch_unwind per task** — panic isolation is essential
- **Cooperative cancellation** — simpler than preemption, works because we control the yield points
- **Monotonic IDs** — no UUID overhead, no collisions within a process
- **LazyLock global runtime** — simple, no initialization ceremony
- **crossbeam-channel** — drop-in replacement for mpsc, enables `select`, battle-tested
- **Opaque Value variants for handles** — compile-time exhaustiveness, no accidental construction

---

## Competitive Analysis

### Concurrency Primitives

| Feature | ntnt | Go | Elixir | Rust (tokio) |
|---------|------|-----|--------|--------------|
| Spawn task | ✅ `spawn(fn)` | `go func()` | `spawn(fn)` | `tokio::spawn(async)` |
| Await result | ✅ `await_task(t)` | N/A (channels) | `Task.await(t)` | `.await` |
| Channels | ✅ `channel()` | `make(chan)` | N/A (mailbox) | `mpsc::channel()` |
| Select | ✅ `select(chs)` | `select {}` | `receive do` | `tokio::select!` |
| Parallel | 📋 Phase 2 | `errgroup` | `Task.async_stream` | `join!` |
| Structured | 📋 Phase 2 | N/A | `Task.Supervisor` | `JoinSet` |
| Schedule | ✅ `schedule("5s", fn)` | ticker | `:timer` | `tokio::interval` |
| Cancel | ✅ Cooperative | Context | `Task.shutdown` | `CancellationToken` |

### Job Systems

| Feature | ntnt (planned) | Sidekiq (Ruby) | Oban (Elixir) | BullMQ (JS) |
|---------|---------------|----------------|---------------|-------------|
| Declaration | `Job X on q { }` | Class + include | `use Oban.Worker` | Class |
| Backend | Memory + KV | Redis only | PG only | Redis only |
| Backend flexibility | Redis/SQLite/Valkey | Redis only | PG only | Redis only |
| Unique jobs | ✅ SHA256 dedup | Pro ($) | ✅ Free | ✅ |
| Cron | ✅ Distributed lock | Enterprise ($) | ✅ Free | ✅ |
| Rate limiting | ✅ | Enterprise ($) | ✅ Pro ($) | ✅ |
| Pause/resume | ✅ | ✅ | ✅ | ✅ |
| Intent verification | ✅ Phase 7 | ❌ | ❌ | ❌ |
| AI diagnosis | ✅ Phase 7 | ❌ | ❌ | ❌ |

**Key insight:** ntnt's job system gives away everything Sidekiq charges $250/mo for, works with any KV store (not locked to Redis or PG), and adds AI-native features nobody else has.

---

## Open Questions

### Resolved

| Question | Decision | Date |
|----------|----------|------|
| Thread-per-task vs threadpool? | Thread-per-task for primitives. Revisit for job workers. | 2026-03-15 |
| async/await in ntnt? | No. Synchronous model with spawn + channels. | 2026-03-15 |
| Job DSL as syntax vs library? | Syntax (`Job` keyword). First-class, testable in IDD. | 2026-03-15 |
| One PR or many? | One per phase. Each phase is shippable independently. | 2026-03-16 |
| crossbeam vs std::mpsc? | **crossbeam.** Enables `select`, drop-in API, battle-tested. | 2026-03-16 |
| Handle types: Value variants vs tagged maps? | **Value variants.** `TaskHandle(u64)`, `ChannelHandle(u64)`, `ScheduleHandle(u64)`. | 2026-03-16 |
| `select` return format? | **`{channel: <handle>, value: <data>}`** map. `{status: "timeout"}` on timeout. | 2026-03-16 |
| PG backend vs KV backend? | **KV.** Use `std/kv` (Redis/SQLite/Valkey). No raw PG backend. | 2026-03-16 |

### Open

| Question | Options | Notes |
|----------|---------|-------|
| Reaper TTL configurable? | Env var, runtime config, or hardcoded | Currently 5min hardcoded |
| Job workers: thread-per-task or pool? | Pool (for throughput) | Phase 3+ decision |
| KV job claiming strategy? | `set-if-not-exists` vs Lua script vs list pop | Depends on KV backend |
| Feature flags for job backends? | Compile-time (Cargo features) vs runtime | Leaning runtime |
| Where do composition functions live? | Rust (fast) vs ntnt stdlib (extensible) | Both? |
| KV key layout? | Draft in Phase 4 details | Needs validation |

---

## Appendix: Job DSL Design

Full Job DSL syntax, lifecycle, and backend details preserved from the original `background_jobs.md`. See that file for the complete reference. Key highlights:

### Job Declaration

```ntnt
Job ProcessPayment on payments (retry: 5, timeout: 120s) {
    perform(order_id: String, amount: Float) {
        let order = db.find(order_id)
        stripe.charge(order.customer_id, amount)
    }

    on_failure(error, attempt) {
        alert.notify("Payment failed: #{error}")
    }
}
```

### Job Lifecycle

```
Scheduled → Pending → Active → Completed
                        ↓
                      Retry → (retries exhausted) → Dead Letter
                        ↑
                      Pending
```

### Worker Models

```ntnt
// Combined (simple apps): HTTP + jobs in same process
listen(8080)
Queue.work_async()

// Separate (production): dedicated worker process
Queue.work(map { "queues": ["emails", "payments"], "concurrency": 10 })
```

### Composition (Future — Phase 2+)

```ntnt
// Chains (sequential)
Chain ProcessOrder {
    ValidateOrder -> ReserveInventory -> ChargePayment -> SendConfirmation
}

// Workflows (DAG)
Workflow UserOnboarding {
    CreateAccount -> SendWelcomeEmail
    CreateAccount -> SetupBilling
    [SendWelcomeEmail, SetupBilling] -> ActivateAccount
}

// Batches (parallel with callback)
let batch = Batch.create(map { "on_complete": fn(results) { ... } })
batch.add(ProcessChunk, map { "chunk": data })
batch.run()
```

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-15 | Initial implementation: Phases 0-6 built across 6 feature branches |
| 2026-03-15 | v2 rewrite: stripped job system, rebuilt concurrency primitives clean |
| 2026-03-16 | DD-037 v3: comprehensive roadmap with lessons learned, DX hardening plan |
| 2026-03-16 | Phase 1 complete: handle types, try_await states, select() — commit `f63f23b` |
| 2026-03-16 | DD-037 v4: KV backend replaces PG+Redis. Checkboxes for progress tracking. |
