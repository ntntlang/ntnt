# DD-037: Concurrency & Job System

**Status:** Phases 0-3, 6 Complete. Priority Queues, Control Socket, Worker Environment, DX Fixes all shipped. Phases 4-5, 7-8 Planned.
**Author:** Larri
**Created:** 2026-03-15
**Last Updated:** 2026-03-27
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
Phase 0   ✅  Concurrency Primitives                    spawn, channels, schedules
Phase 1   ✅  Primitive Hardening                        handle types, select(), reaper
Phase 2   ✅  Job DSL + KV Backend                       PR 2a/2b/2c — declarative jobs, workers, testing, CLI
Phase 3   ✅  Job System Advanced Features               Lua claim, scheduled optimization, dedup, expiration, batch enqueue
Phase 3b  ✅  Priority Queues + Atomic Dedup             PR #41 — named priorities, worker bands, kv_set_nx
Phase 3c  ✅  Control Socket + CLI                       PR #42 — .ntnt.sock, ntnt workers status/scale
Phase 4   📋  Composition Layer                          parallel, race, task groups
Phase 5   📋  Dashboard + Production Hardening            real-time UI, simulation, contracts
Phase 6   ✅  Observability CLI                          PR #35 — list/inspect/retry/cancel/clear + dedup refactor
Phase 7   📋  Event Dispatch (std/events)                pub/sub fan-out over the job system
Phase 8   📋  Job Audit Log & Observability Pipeline    structured logs, sinks, webhooks, web viewer
```

### Cross-Cutting Work (shipped alongside phases)

```
DD-044  ✅  Jobs DX Fixes                               PRs #43, #54, #55 — all 6 findings resolved
DD-045  ✅  Job Worker Environment                       PRs #44, #45, #46, #49 — RuntimeCapability, full app context, jobs() discovery
```

### Phase Status Table

| Phase | Name | Status | Details |
|-------|------|--------|---------|
| 0 | Concurrency Primitives | ✅ Done | PR #31 merged |
| 1 | Primitive Hardening | ✅ Done | 2 Copilot reviews, 16/16 resolved |
| 2 | Job DSL + KV Backend | ✅ Done | PR #32 (parser/registry), #33 (workers/retry), #34 (DX/CLI/docs). See [dd-037-phase-2-implementation.md](dd-037-phase-2-implementation.md) |
| 3 | Job System Advanced Features | ✅ Done | PR #36 (atomic claim + scheduled opt), #38 (dedup + expiration), #39 (batch enqueue). See [dd-037-phase-3-plan.md](dd-037-phase-3-plan.md) |
| 3b | Priority Queues + Atomic Dedup | ✅ Done | PR #41 merged. Named priorities (critical/high/normal/low), worker bands with independent thread pools, `kv_set_nx`, `scale_workers()`, `worker_status()`, band validation. See [dd-037-priority-and-atomic-dedup-plan.md](dd-037-priority-and-atomic-dedup-plan.md) |
| 3c | Control Socket + CLI | ✅ Done | PR #42 merged. `.ntnt.sock` Unix domain socket, `ntnt workers status`, `ntnt workers scale <band> <n>`, poisoned lock recovery. |
| 4 | Composition Layer | 📋 Planned | parallel, race, task groups |
| 5 | Dashboard + Production Hardening | 📋 Planned | Dashboard, simulation, contracts, intent testing |
| 6 | Observability CLI | ✅ Done | PR #35 merged. list/inspect/retry/cancel/clear + CLI/stdlib dedup refactor. `tail`, `replay`, `workers` deferred. |
| 7 | Event Dispatch (`std/events`) | 📋 Planned | pub/sub fan-out over the job system |
| 8 | Job Audit Log & Observability Pipeline | 📋 Planned | See [dd-042-job-audit-log.md](dd-042-job-audit-log.md) |
| — | DD-044: Jobs DX Fixes | ✅ Done | All 6 fixes shipped. PRs #43 (Fixes C/D/E), #54 (Fix F), #55 (Fix B). Fix A superseded by DD-045. See [dd-044-jobs-dx-fixes.md](dd-044-jobs-dx-fixes.md) |
| — | DD-045: Worker Environment | ✅ Done | PRs #44 (RuntimeCapability), #45 (worker interpreter), #46 (jobs() discovery), #49 (enqueue gating). See [dd-045-job-worker-environment.md](dd-045-job-worker-environment.md) |

---

## Phase Details

### Phase 2: Job DSL + KV Backend ✅

**Status:** Complete. Implemented across 3 PRs.
**Implementation doc:** [dd-037-phase-2-implementation.md](dd-037-phase-2-implementation.md)

| PR | Title | Status |
|----|-------|--------|
| #32 | Parser + Registry + Enqueue MVP | ✅ Merged |
| #33 | Workers + Lifecycle + Retry | ✅ Merged |
| #34 | Testing Mode + Logs + CLI + Docs | ✅ Merged |

**What shipped:**
- [x] `job Name on queue (options) { perform(params) { body } on_failure(...) { body } }` parser syntax
- [x] `enqueue(name, args)`, `enqueue_at()`, `enqueue_in()` — immediate and scheduled enqueueing
- [x] `job_status(id)`, `cancel_job(id)` — status and cancellation
- [x] `work_async(opts?)`, `work_jobs(opts?)` — background and blocking workers
- [x] Retry with exponential/linear/constant backoff, `on_failure` handler
- [x] `configure_queue(map { "store": "..." })` — SQLite or Redis/Valkey backend via std/kv
- [x] Atomic job claiming (`kv_claim` — SQLite `BEGIN IMMEDIATE` transaction)
- [x] Visibility timeout (`jobs:active:<id>` with TTL)
- [x] Testing mode: `assert_enqueued`, `assert_not_enqueued`, `drain_jobs`, `clear_jobs`
- [x] Streaming JSON logs to stderr (job.enqueued, started, completed, failed, dead)
- [x] `ntnt worker` CLI command with --concurrency, --queues, --poll-interval
- [x] `ntnt jobs` CLI command — queue status summary
- [x] Ctrl-C graceful shutdown for `work_jobs()` and `ntnt worker`
- [x] Job timeout (post-execution elapsed check)
- [x] Full documentation: AI_AGENT_GUIDE.md, STDLIB_REFERENCE.md, examples/job_demo.tnt

**API (free functions, not methods — consistent with ntnt design):**
- `enqueue("JobName", args)` — string lookup, not `JobName.enqueue(args)`
- `configure_queue(map { "store": "..." })` — not `Queue.configure()`
- `work_async()` / `work_jobs()` — not `Queue.work()`

---

### Phase 3: Job System Advanced Features ✅

**Status:** Complete. Implemented across 3 PRs (#36, #38, #39).
**Implementation doc:** [dd-037-phase-3-plan.md](dd-037-phase-3-plan.md) · [dd-037-phase-3-implementation.md](dd-037-phase-3-implementation.md)

| PR | Title | Status |
|----|-------|--------|
| #36 | Atomic Claim + Scheduled Optimization | ✅ Merged |
| #38 | Dedup + Expiration | ✅ Merged |
| #39 | Batch Enqueue | ✅ Merged |

**What shipped:**
- [x] Redis atomic claim via Lua `EVAL` script (KEYS+sort+GET+DEL in one operation)
- [x] Scheduled job claim optimization (`ceiling` parameter — workers skip future jobs at KV layer)
- [x] Deduplication (`unique: N` — SHA-256 hash dedup with TTL, live-job validation, cleanup on terminal states)
- [x] Job expiration (`expires: N` — worker skips stale jobs, marks "expired")
- [x] Batch enqueue (`enqueue_batch` — upfront validation, FIFO ordering, 10K limit, item-indexed errors)

**Remaining items (deferred to on-demand, not blocking):**
- [ ] `on_job_event(handler)` — user callback for job events (cross-thread closure design needed)
- [x] ~~Priority queues (`priority: N`)~~ — shipped in PR #41 (Phase 3b)
- [ ] Worker heartbeat refresh (for jobs running >5 minutes)
- [ ] Graceful shutdown drain timeout
- [x] ~~Atomic dedup via `SET NX` (current is best-effort get+set)~~ — shipped in PR #41 (`kv_set_nx`)
- [ ] Redis SCAN in Lua (replace `KEYS` for large keyspaces)

---

### Phase 4: Composition Layer 📋

**Depends on:** Phase 1 ✅
**Estimated effort:** 2-3 days

- [ ] `parallel(fns) -> Array` — run N functions, collect all results, cancel on first error
- [ ] `race(fns) -> Value` — run N functions, return first result, cancel others
- [ ] `task_group(fn(group))` — structured scope, all tasks cancelled when scope exits

---

### Phase 5: Dashboard + Production Hardening 📋

**Depends on:** Phase 2 ✅, Phase 3 (some items)
**Estimated effort:** 5-7 days

Real-time dashboard and safety features that make jobs production-ready.

#### Real-Time Dashboard
- [ ] `configure_queue(map { "dashboard": true })` — adds `/jobs` routes
- [ ] Dashboard shows: pending/active/completed/failed/dead counts by queue
- [ ] Per-job-type breakdown with average duration
- [ ] Live-updating (SSE or polling)
- [ ] Job detail view (payload, attempts, error, duration)
- [ ] Retry/cancel actions from dashboard
- [ ] **Security: localhost-only by default** (see [Dashboard Security Model](#dashboard-security-model))
- [ ] API key auth option for remote access

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
- [x] ~~Rate limiting~~ — shipped as `rate: "N/interval"` sliding window counter in DD-051 (PR #62)
- [x] ~~Concurrency limits~~ — shipped as `concurrency: N` atomic counter semaphore in DD-051 (PR #62)
- [x] ~~Queue pause/resume~~ — shipped as `pause_queue()`/`resume_queue()` + CLI + control socket in DD-051 (PR #62)
- [x] ~~CancelToken~~ — instant cooperative cancellation replacing AtomicBool polling, DD-051 (PR #62)
- [ ] Cron expressions — `schedule("0 9 * * MON-FRI", fn)` with distributed lock
- [ ] Dead job caps — 10K max, auto-prune oldest
- [x] ~~Idempotency~~ — shipped as `unique: N` in Phase 3 (PR #38)

---

### Phase 6: Observability CLI ✅

**Depends on:** Phase 2 ✅ (streaming logs, `ntnt jobs status` already shipped)
**Status:** Complete. Implemented in PR #35 (merged 2026-03-18).

Extends the basic `ntnt jobs` CLI (shipped in Phase 2c) with full observability tools.

**Shipped:**
- [x] `ntnt jobs status` — shipped in Phase 2c (PR #34)
- [x] `ntnt jobs list [--status=X] [--queue=X] [--limit=N] [--format=json]` — filter and list jobs
- [x] `ntnt jobs inspect <job-id>` — full job details (payload, attempts, error, timestamps)
- [x] `ntnt jobs retry <job-id>` — re-queue a retrying/dead/failed job (resets attempts)
- [x] `ntnt jobs cancel <job-id> [--force]` — cancel pending/scheduled/retrying; force-cancel active
- [x] `ntnt jobs clear --status=X [--older-than=DURATION] [--yes]` — bulk delete by status with age filter
- [x] Stdlib parity: `retry_job()`, `cancel_job()`, `list_jobs()`, `delete_jobs()` — same logic as CLI
- [x] CLI/stdlib deduplication refactor: public API functions (`retry_job_by_id`, `cancel_job_by_id`, `list_jobs_filtered`, `delete_jobs_filtered`, `job_status_counts`) — both CLI and stdlib are thin wrappers
- [x] New job statuses: `"retrying"` (mid-backoff, replaces `"failed"`), `"scheduled"` (enqueue_at/enqueue_in), `"dead"` (terminal failure)
- [x] Backward compat: `"failed"` accepted in retry/cancel for pre-v0.4.6 data
- [x] RUNTIME_REFERENCE.md, AI_AGENT_GUIDE.md, CLAUDE.md, copilot-instructions.md all updated
- [x] Greptile review: strict filter matching (corrupt records rejected), CLI/stdlib `"failed"` parity

**Deferred (follow-up, not blocking):**
- [ ] `ntnt jobs tail [--queue=X] [--status=X] [--since=1h]` — streaming log view with filters
- [ ] `ntnt jobs replay <job-id> [--dry-run]` — re-run with same inputs
- [ ] `ntnt jobs workers` — list active workers with health status
- [ ] `--format=agent` option for LLM-optimized compact output

---

### Deferred Items (not blocking any phase)

- [x] ~~**Closure capture DX**~~ — solved via free-variable analysis in PR #55 (DD-044 Fix B). `schedule()`, `spawn()`, `after()` now only capture bindings the closure actually references. No more failures from unrelated user functions in scope.
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
| Priority system | Named (critical/high/normal/low) + raw 0-99 | Progressive disclosure — simple API, full control available |
| Worker bands | Independent thread pools per priority range | No starvation, obvious scaling knob, configurable per band |
| KV key format | `jobs:pending:<priority:02>:<timestamp>:<id>` | Lexicographic ordering = FIFO within band, priority across bands |
| Execution mode enforcement | RuntimeCapability enum + action registry | Structural enforcement — compiler prevents missing skip checks |
| Worker interpreter | Full source file eval in Worker mode | Workers have complete app context — all imports, functions, constants |
| Closure capture | Free-variable analysis (AST walker) | Only captures referenced bindings — unrelated functions don't cause failures |
| Control socket | Unix domain socket (`.ntnt.sock`) | Same-user access, no auth needed, works with Docker exec |

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
| Unique jobs | `unique: 3600` ✅ | $250/mo (Enterprise) | Free | Free | Custom code |
| Simulation/dry-run | `effect` blocks | ❌ | ❌ | ❌ | ❌ |
| Job contracts | `requires`/`ensures` | ❌ | ❌ | ❌ | ❌ |
| Intent testing | `.intent` files | RSpec (manual) | ExUnit (manual) | Jest (manual) | PHPUnit (manual) |
| Streaming logs | `ntnt jobs tail` | Manual | Manual | Manual | Docker logs + grep |
| Stuck job recovery | Automatic (heartbeat) | Automatic | Automatic | Automatic | **Manual clearing** |
| Batch enqueue | `enqueue_batch()` ✅ | `.perform_bulk()` | `Oban.insert_all()` | `.addBulk()` | Custom |
| Scaling | Same binary, add processes | Separate process | Built into Phoenix | Separate process | Docker containers |
| Event-driven dispatch | `std/events` subscribe/publish | `ActiveSupport::Notifications` (no durability) | ❌ (pub/sub separate) | ❌ (separate) | Custom event bus |

**ntnt wins on:** job definition simplicity, backend flexibility, priority queues (free, built-in, with named priorities), simulation mode (planned), job contracts (planned), intent testing (planned), dashboard (free + secure, planned), streaming logs, runtime worker scaling, free-variable analysis for closures.

**What ntnt already matches:** Priority queues (Sidekiq Enterprise $250/mo), unique/dedup jobs (Sidekiq Enterprise), batch enqueue, runtime scaling, control CLI, worker bands.

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

### Resolved (Phase 2)

| Question | Decision | Date |
|----------|----------|------|
| Job API style? | Free functions (`enqueue("Name", args)`), not methods (`Name.enqueue(args)`) | 2026-03-17 |
| `job` as keyword or contextual? | Contextual identifier — avoids breaking existing code using `job` as variable | 2026-03-17 |
| KV claiming strategy? | `kv_claim()` — SQLite `BEGIN IMMEDIATE` transaction; Redis SCAN+GET+DEL (Lua planned Phase 3) | 2026-03-17 |
| Job workers: thread-per-task or pool? | Thread-per-task via `std::thread::spawn` + ConcurrencyRuntime integration | 2026-03-17 |
| In-memory or persistent backend? | Persistent from day one via std/kv (SQLite default, Redis for prod) | 2026-03-17 |
| `on_job_event` handler storage? | Deferred to Phase 3 — user closures (Rc) are not Send, need captured bindings or channel design | 2026-03-17 |

### Resolved (Phase 3b — Priority + Atomic Dedup)

| Question | Decision | Date |
|----------|----------|------|
| Priority range? | 0-99, zero-padded in KV keys for lexicographic ordering | 2026-03-20 |
| Default priority? | 50 ("normal") — midpoint of normal band | 2026-03-20 |
| Named priorities? | critical=5, high=25, normal=50, low=85 | 2026-03-20 |
| Worker model for bands? | Independent thread pools per band — prevents starvation | 2026-03-20 |
| Custom bands? | Replace defaults entirely, no partial overrides | 2026-03-20 |
| Band validation? | Reject overlaps, gaps, bad values at startup. Full 0-99 coverage required. | 2026-03-20 |
| Atomic dedup? | `kv_set_nx` (SET NX / INSERT OR IGNORE) — closes race window | 2026-03-20 |
| Queue name optional? | Yes — defaults to "default". Parser updated. | 2026-03-20 |
| Control plane? | Stdlib functions + Unix domain socket + CLI (separate PRs) | 2026-03-20 |

### Resolved (DD-045 — Worker Environment)

| Question | Decision | Date |
|----------|----------|------|
| Worker execution model? | Full app context — workers evaluate entire .tnt source file, then process jobs | 2026-03-21 |
| Execution mode enforcement? | RuntimeCapability system — functions declare requirements, modes declare provisions, compiler enforces | 2026-03-21 |
| Closure capture DX? | Free-variable analysis — scope-aware AST walker captures only referenced bindings (PR #55) | 2026-03-22 |
| Job imports problem? | Dissolved — workers have the full app loaded. DD-044 Fix A unnecessary. | 2026-03-21 |
| Enqueue in worker bootstrap? | Gated behind `JobEnqueue` capability — workers can't fire side-effect enqueues during file eval (PR #49) | 2026-03-22 |

### Open

| Question | Options | Notes |
|----------|---------|-------|
| Reaper TTL configurable? | Env var, runtime config, or hardcoded | Currently 5min hardcoded |
| Where do composition functions live? | Rust (fast) vs ntnt stdlib (extensible) | Probably both |
| Dashboard SSE vs polling? | SSE (real-time) vs polling (simpler) | Leaning SSE |
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
| 2026-03-17 | Phase 2 complete: Job DSL shipped across PRs #32, #33, #34. Roadmap renumbered — old Phase 3/4 consolidated into Phase 2 (Job DSL + KV Backend). Phase 3 = advanced features (deferred items). Phase 4 = composition layer. Phase 5 = dashboard + hardening. |
| 2026-03-17 | DD-037 v7: Added Phase 7 — Event Dispatch (`std/events`). Pub/sub fan-out over the job system. Memory + Redis backends. Testing mode integration. |
| 2026-03-17 | PR #34 merged: Greptile review fixes — drain_jobs collects all errors (no silent job loss), timeout always overrides exec error, --concurrency 0 rejected, stderr JSON key order documented. Phase 2 fully complete. |
| 2026-03-18 | Phase 8 added: Job Audit Log & Observability Pipeline (DD-042). Structured logs with KV/file/stderr/webhook sinks, configurable verbosity, TTL, CLI tail, programmatic API. |
| 2026-03-17 | Phase 2 merged: PR #34 merged after Greptile review fixes (drain_jobs fail-fast, timeout-wins-over-error, --concurrency 0 guard). All 3 Phase 2 PRs now on main. |
| 2026-03-20 | Phase 3 complete: PR #36 (atomic Lua claim + scheduled optimization), #38 (dedup + expiration), #39 (batch enqueue) — all merged. Remaining items (heartbeat refresh, on_job_event, drain timeout) deferred to on-demand. |
| 2026-03-20 | Phase 3b: PR #41 merged — priority queues with worker bands (critical/high/normal/low), `kv_set_nx` atomic dedup, `scale_workers()`, `worker_status()`, optional queue syntax, band validation. |
| 2026-03-21 | Phase 3c: PR #42 merged — control socket (`.ntnt.sock`), `ntnt workers status`, `ntnt workers scale`, poisoned lock recovery. |
| 2026-03-21 | DD-044 Fixes C/D/E: PR #43 merged — idempotent job registration, `parse_json(None)` returns Err, KV handle hint for missing `unwrap()`. |
| 2026-03-22 | DD-045 complete: PRs #44 (RuntimeCapability system), #45 (worker interpreter with full app context), #46 (jobs() directory auto-discovery), #49 (enqueue gating behind JobEnqueue capability). Workers now evaluate the entire .tnt source file — perform blocks have access to all imports, functions, and constants. |
| 2026-03-22 | DD-044 Fix F: PR #54 merged — `ntnt jobs` CLI evaluates in Worker mode (no side effects). |
| 2026-03-23 | DD-044 Fix B: PR #55 merged — free-variable analysis for `schedule()`/`spawn()`/`after()` closures. Scope-aware AST walker captures only referenced bindings. All DD-044 fixes now complete. |
| 2026-03-23 | DD-037 v8: Updated roadmap, phase status table, resolved questions, architecture decisions, competitive analysis. All shipped work accurately reflected. 1,241 tests on main. |
| 2026-03-28 | DD-051 merged (PR #62): Rate limiting, concurrency limits, queue pause/resume, CancelToken. Phase 5 hardening items checked off. 1,317 tests on main. Remaining enterprise features tracked in DD-052. |
