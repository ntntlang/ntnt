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
2. **Background job system** — persistent, reliable, production-grade job queuing with declarative syntax. The Sidekiq/Oban of the ntnt world, but multi-backend and language-native.

These are layered: primitives first (they're useful alone), jobs built on top.

**Design principles:**
- **Zero-config start** — `spawn()` just works, no setup
- **CSP by architecture** — `Rc<RefCell>` forces serialization at task boundaries. Two tasks literally cannot share memory. Go says "don't share memory"; ntnt physically can't.
- **No async/await** — synchronous model, no function coloring. `spawn` + channels achieve the same results.
- **Declarative jobs** — `Job SendEmail on emails { perform(id) { ... } }` reads like documentation
- **Multi-backend** — Memory (dev), PostgreSQL (production), Redis (high-throughput)

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

Each spawned task gets a **fresh interpreter instance** with captured bindings injected. Cross-task communication goes through channels (mpsc). This gives us:
- True parallelism (OS threads)
- Zero shared mutable state (architecturally impossible)
- panic isolation (catch_unwind per task)

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

### ConcurrencyRuntime

Single global instance (`LazyLock<ConcurrencyRuntime>`) owns all state:
- Monotonic ID counter (`AtomicU64`) shared by tasks, channels, and schedules
- Task registry with state, result, error, cancellation flag, completion time
- Channel registry (mpsc sender/receiver pairs)
- Schedule registry with cancellation and overlap-prevention flags
- Lock discipline: acquire → clone Arcs → drop → operate. Never nest locks.

---

## Current State — What's Built

### ✅ Phase 1: Concurrency Primitives (`feat/concurrency-v2`)

**Branch:** `feat/concurrency-v2` (2 commits, +2,895 lines, 26 tests)  
**Status:** Code complete. Copilot review addressed. Ready for primitive hardening.

#### API Surface

| Function | Signature | Description |
|----------|-----------|-------------|
| `spawn` | `spawn(fn() -> T) -> Task` | Run function in background thread |
| `await_task` | `await_task(task) -> Result<T, String>` | Block until done, consume handle |
| `try_await` | `try_await(task) -> Map` | Non-blocking peek: `{status, result}` |
| `cancel_task` | `cancel_task(task) -> Bool` | Cooperative cancellation (flag-based) |
| `channel` | `channel() -> Channel` | Create unbounded mpsc channel |
| `send` | `send(ch, value) -> Bool` | Send value (false if closed) |
| `recv` | `recv(ch) -> Value` | Blocking receive (Unit if closed) |
| `recv_timeout` | `recv_timeout(ch, ms) -> Option` | Receive with timeout |
| `try_recv` | `try_recv(ch) -> Option` | Non-blocking receive |
| `close` | `close(ch) -> Bool` | Close channel |
| `schedule` | `schedule(interval, fn()) -> Schedule` | Recurring execution |
| `cancel_schedule` | `cancel_schedule(sched) -> Bool` | Stop recurring execution |
| `after` | `after(delay, fn()) -> Task` | Delayed one-shot execution |
| `sleep_ms` | `sleep_ms(ms) -> Unit` | Cancellation-aware sleep |
| `thread_count` | `thread_count() -> Int` | Available CPU threads |

#### What Works Well
- Clean API, reads like English
- CSP enforced by architecture (serialize-at-boundary)
- Cooperative cancellation with yield points (recv, sleep_ms, fetch)
- Schedule overlap prevention (skip tick if previous still running)
- Panic isolation (catch_unwind per task)
- Task reaper (5-minute TTL on terminal tasks, respects try_await)
- String interval parsing ("5s", "1m", "500ms")

### ❌ Phase 2–6: Job DSL (Removed from `feat/concurrency-v2`)

The job system (Job DSL, PostgreSQL backend, Redis backend, polish features, refactor) was built across 6 feature branches during the initial implementation. In `feat/concurrency-v2`, **all job code was removed** (-10,440 lines) to rebuild on a solid primitive foundation first.

**The job code still exists on the original branches:**
- `feat/job-dsl` — Job keyword, in-memory backend, retry/backoff/dead letter
- `feat/job-dsl-postgres` — PostgreSQL backend, `SELECT FOR UPDATE SKIP LOCKED`, heartbeats
- `feat/job-dsl-redis` — Redis Streams, consumer groups, XPENDING/XCLAIM recovery
- `feat/job-dsl-polish` — Unique jobs, transactional enqueue, LISTEN/NOTIFY, cron, queue pause
- `feat/job-dsl-refactor` — Modular architecture, JobBackend trait

**Total work preserved:** ~4,800 lines Rust, 1,063 tests passing, 14 PG-only ignored.

---

## Roadmap — What's Coming

### Overview

```
Phase 0  ✅ Primitives (spawn, channels, schedule, after)     ← DONE
Phase 1  🔨 Primitive Hardening (try_await, handles, select)  ← NEXT
Phase 2  📋 Composition Layer (parallel, race, task groups)
Phase 3  📋 Job DSL Revival (clean rebuild on solid primitives)
Phase 4  📋 PostgreSQL Backend
Phase 5  📋 Redis Backend  
Phase 6  📋 Polish & Production Features
Phase 7  📋 Observability & CLI
Phase 8  📋 Agent-First Features
```

### Phase Status Table

| Phase | Name | Status | Branch | Tests | Priority |
|-------|------|--------|--------|-------|----------|
| 0 | Primitives | ✅ Done | `feat/concurrency-v2` | 26 | — |
| 1 | Primitive Hardening | 🔨 Next | `feat/concurrency-v2` | — | **P0 — ship blocker** |
| 2 | Composition Layer | 📋 Planned | TBD | — | P1 — high value |
| 3 | Job DSL Revival | 📋 Planned | TBD | — | P1 — core feature |
| 4 | PostgreSQL Backend | 📋 Planned | TBD | — | P1 — production req |
| 5 | Redis Backend | 📋 Planned | TBD | — | P2 — high-throughput |
| 6 | Polish & Production | 📋 Planned | TBD | — | P2 — hardening |
| 7 | Observability & CLI | 📋 Planned | TBD | — | P2 — DX |
| 8 | Agent-First Features | 📋 Planned | TBD | — | P3 — differentiator |

---

## Phase Details

### Phase 1: Primitive Hardening 🔨

**Priority:** P0 — ship blocker. These are the primitive contracts. Get them wrong now, break everyone later.

**Estimated effort:** 1-2 days Rust work, all in `concurrent.rs` + evaluator.

#### Issue 1: `try_await` Return Value Ambiguity

**Problem:** `try_await` errors when the task handle is invalid (already awaited or expired), but the error message doesn't distinguish between "consumed by await_task" and "expired by reaper." More importantly, the caller has no way to distinguish these two cases programmatically.

**Current behavior:**
```ntnt
let task = spawn(fn() { return 42 })
// ... 6 minutes pass, reaper runs ...
try_await(task)  // RuntimeError: "Invalid task handle (task already awaited or expired)"
```

**Proposed behavior:** `try_await` returns a map with explicit status:

```ntnt
// Still running
try_await(task) // => { status: "running", result: None }

// Completed
try_await(task) // => { status: "completed", result: Ok(42) }

// Failed  
try_await(task) // => { status: "failed", result: Err("timeout") }

// Handle was consumed by await_task
try_await(task) // => { status: "consumed", result: None }

// Handle expired (reaper cleaned it up)
try_await(task) // => { status: "expired", result: None }
```

**Implementation:**
- Don't remove from registry on `await_task` — instead, mark state as `Consumed`
- Add `TaskState::Consumed` and `TaskState::Expired` variants
- Reaper marks as `Expired` instead of removing
- `try_await` never errors on valid-format handles — always returns a status map
- `await_task` on Consumed/Expired returns clear error

**Why now:** This is a return value contract. Once people write `if try_await(t).status == "running"`, we can't change what statuses exist.

---

#### Issue 2: Handle Type Safety

**Problem:** Handles are plain Maps with `{type: "Task", _handle_id: 7}`. Nothing stops you from:
```ntnt
let ch = channel()
await_task(ch)  // Runtime error: "Expected a Task handle" — but only at runtime
```

Or worse:
```ntnt
let fake = map { "type": "Task", "_handle_id": 999 }
await_task(fake)  // Looks valid, fails with confusing error
```

**Proposed solution:** Introduce opaque handle types at the Value level.

**Option A: New Value variants (preferred)**
```rust
// In interpreter.rs
enum Value {
    // ... existing variants ...
    TaskHandle(u64),
    ChannelHandle(u64),
    ScheduleHandle(u64),
}
```

- Typechecker can validate: `await_task` requires `TaskHandle`, `send` requires `ChannelHandle`
- No accidental construction (ntnt code can't create `Value::TaskHandle` directly)
- Pattern matching in Rust is exhaustive — compiler catches missed cases
- Display: `Task(7)`, `Channel(3)`, `Schedule(1)`
- Serialization: handles are NOT serializable across task boundaries (they're process-local)

**Option B: Tagged Maps with validation (simpler, less safe)**
- Keep Map-based handles
- Add `_opaque: true` field that ntnt code can't set
- Runtime validation only

**Recommendation:** Option A. It's more Rust work upfront but eliminates an entire class of bugs. Handles become a real type, not a convention.

**Typechecker integration:**
```ntnt
let task: Task = spawn(fn() { 42 })        // Typechecker knows this is Task
let ch: Channel = channel()                  // Typechecker knows this is Channel
await_task(ch)                               // ← TYPE ERROR at check time
```

**Why now:** If handles are Maps in v0.5.0 and we change them to opaque types in v0.6.0, every program that inspects handle fields breaks. Ship the right type from day one.

---

#### Issue 3: `select()` — Multi-Channel Wait

**Problem:** Without `select`, you can't express "wait for data from channel A OR channel B, whichever comes first." The workaround is busy-polling with `try_recv`:

```ntnt
// UGLY: busy-polling pattern
loop {
    let a = try_recv(ch_a)
    if a != None { handle_a(a); break }
    let b = try_recv(ch_b)
    if b != None { handle_b(b); break }
    sleep_ms(10)  // arbitrary delay, wastes CPU or adds latency
}
```

Every concurrent language has a primitive for this:
- Go: `select { case msg := <-ch1: ... case msg := <-ch2: ... }`
- Rust: `tokio::select! { v = rx1.recv() => ..., v = rx2.recv() => ... }`
- Erlang: `receive Msg1 -> ...; Msg2 -> ... end`

**Proposed API:**

```ntnt
import { channel, select } from "std/concurrent"

let ch_a = channel()
let ch_b = channel()

// Wait for first available value
let result = select([ch_a, ch_b])
// => { channel: ch_a, value: "hello" }  (whichever fires first)

// With timeout
let result = select([ch_a, ch_b], 5000)
// => { channel: ch_a, value: "data" }   or
// => { status: "timeout" }              if 5s passes
```

**Implementation approach:**
- Spawn monitor threads per channel that forward to a collector channel
- Or: use `recv_timeout` rotation with decreasing timeouts
- Or: restructure channels to use `crossbeam::Select` under the hood (cleanest)

**Recommended: crossbeam-channel**
- Replace `std::sync::mpsc` with `crossbeam::channel` (already battle-tested, similar API)
- `crossbeam::Select` gives us multi-channel wait for free
- Also gives us bounded channels (future feature)
- Minimal diff — crossbeam's API mirrors mpsc

```rust
// Rust implementation sketch
fn concurrent_select(channels: &[Value], timeout_ms: Option<i64>) -> Result<Value> {
    let mut sel = crossbeam::channel::Select::new();
    // Register each channel's receiver
    for ch in channels {
        let id = get_handle_id(ch, "Channel")?;
        let receiver = RUNTIME.get_receiver(id)?;
        sel.recv(&receiver);
    }
    // Wait with optional timeout
    match timeout_ms {
        Some(ms) => sel.ready_timeout(Duration::from_millis(ms as u64)),
        None => Ok(sel.ready()),
    }
}
```

**Why now:** `select` is not composition — it's a primitive. Channels without select are like arrays without indexing. Every non-trivial concurrent program needs it, and the implementation choice (crossbeam vs polling) affects the entire channel subsystem.

---

#### Issue 4: Closure Capture DX (Deferred — P1)

**Problem:** When `spawn()` captures variables from the enclosing scope, non-serializable captures fail at runtime with a good error message, but there's no way to know which variables will be captured without running the code.

**Current error (already decent):**
```
Cannot capture user-defined function(s) across task boundaries: my_helper.
Use closure capture for data, not function references.
```

**Future improvement (not blocking):**
- Explicit capture syntax: `spawn(capture: [x, y], fn() { ... })`
- Or: lint warning at parse time when spawn closure references outer functions
- Or: `@serializable` annotation on functions that should be capture-safe

**Why deferred:** The error message is clear enough. The fix is a syntax addition that deserves its own design discussion. Won't break anyone to add later.

---

#### Issue 5: `parallel()` / Structured Concurrency (Deferred — P1)

**Problem:** No way to say "run these N things concurrently, collect all results, cancel on first error."

```ntnt
// TODAY: manual boilerplate
let tasks = urls.map(fn(url) { spawn(fn() { fetch(url) }) })
let results = tasks.map(fn(t) { await_task(t) })
// No error handling, no cancellation on failure
```

**Future API:**
```ntnt
// Run all, collect results (cancel remaining on first error)
let results = parallel([
    fn() { fetch("/api/users") },
    fn() { fetch("/api/posts") },
    fn() { fetch("/api/comments") },
])
// => [users, posts, comments]  or throws on first error

// With named tasks
let results = parallel(map {
    "users": fn() { fetch("/api/users") },
    "posts": fn() { fetch("/api/posts") },
})
// => { users: [...], posts: [...] }
```

**Why deferred:** This is composition built on solid primitives. Once spawn + channels + select work correctly, parallel is ~50 lines of ntnt stdlib code (or a thin Rust wrapper). Adding it later doesn't break anything.

---

#### Issue 6: Schedule String Validation (Deferred — P2)

**Problem:** `schedule("every 30s", fn() { ... })` parses at runtime. Invalid strings fail silently or with confusing errors.

**Future improvements:**
- Lint/typecheck warning for unparseable interval strings
- Autocomplete support in IDE integrations
- Document all supported interval formats

**Why deferred:** It works. The error messages are adequate. Pure polish.

---

### Phase 2: Composition Layer 📋

**Depends on:** Phase 1 (especially `select`)  
**Estimated effort:** 2-3 days

| Feature | Description | Priority |
|---------|-------------|----------|
| `parallel(fns)` | Run N functions, collect all results, cancel on first error | High |
| `race(fns)` | Run N functions, return first result, cancel others | High |
| `any(channels)` | Alias for select with simpler return | Medium |
| `task_group()` | Structured scope — all tasks cancelled when scope exits | Medium |
| `pipeline(fns)` | Chain: output of fn1 → input of fn2 → ... | Low |

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

### Phase 3: Job DSL Revival 📋

**Depends on:** Phase 1 (solid primitives)  
**Estimated effort:** 3-5 days (rebuilding from preserved branches)  
**Source:** `feat/job-dsl` branch (preserved)

Rebuild the Job DSL on the hardened primitive foundation. Core features from the original implementation:

```ntnt
Job SendWelcomeEmail on emails {
    perform(user_id: String) {
        let user = db.find_user(user_id)
        email.send(user.email, "Welcome!", body)
    }
}

// Enqueue
SendWelcomeEmail.enqueue(map { "user_id": "123" })
```

**In scope:**
- `Job Name on queue { perform(args) { ... } }` syntax
- In-memory backend (default, zero-config)
- Retry with configurable backoff (exponential, linear, constant)
- Dead letter queue
- Job cancellation and timeout
- Priority queues
- Queue.configure() for backend selection
- Queue.stats() for monitoring
- Graceful shutdown (drain in-flight jobs)

**Preserved from original work:**
- Parser support for Job keyword (in `feat/job-dsl`)
- In-memory backend with full lifecycle (pending → active → completed/failed/dead)
- 10 job-specific tests passing

---

### Phase 4: PostgreSQL Backend 📋

**Depends on:** Phase 3  
**Estimated effort:** 2-3 days (rebuilding from preserved branch)  
**Source:** `feat/job-dsl-postgres` branch (preserved)

```ntnt
Queue.configure(map {
    "backend": "postgres",
    "postgres_url": env("DATABASE_URL")
})
```

**In scope:**
- `ntnt_jobs` table (auto-created on first use)
- `SELECT FOR UPDATE SKIP LOCKED` for distributed locking
- Worker heartbeats (30s default)
- Visibility timeout (re-enqueue stale jobs after 5 min)
- Job history queries
- Automatic pruning of completed/cancelled jobs

**Preserved from original work:**
- Full PostgreSQL backend (~1,132 lines)
- LISTEN/NOTIFY for instant job pickup (from polish branch)
- Transactional enqueue: `Job.enqueue_tx(tx, args)` — atomic with PG transactions
- Cron expressions with advisory lock for cluster safety

---

### Phase 5: Redis Backend 📋

**Depends on:** Phase 3  
**Estimated effort:** 2-3 days (rebuilding from preserved branch)  
**Source:** `feat/job-dsl-redis` branch (preserved)

```ntnt
Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL")
})
```

**In scope:**
- Redis Streams (`XADD`/`XREADGROUP`/`XACK`)
- Consumer groups for distributed processing
- `XPENDING`/`XCLAIM` for stale job recovery
- Sorted set for delayed/scheduled jobs
- Per-queue statistics

**Preserved from original work:**
- Full Redis backend (~1,377 lines)
- Multi-threaded memory worker
- All tests passing

---

### Phase 6: Polish & Production Features 📋

**Depends on:** Phases 3-5  
**Estimated effort:** 2-3 days (rebuilding from preserved branch)  
**Source:** `feat/job-dsl-polish` branch (preserved)

| Feature | Description | Backend Support |
|---------|-------------|-----------------|
| Unique jobs | SHA256 dedup with TTL: `unique: 3600` | All 3 |
| Transactional enqueue | `Job.enqueue_tx(tx, args)` — atomic with PG | PostgreSQL |
| LISTEN/NOTIFY | Instant job pickup via `pg_notify` | PostgreSQL |
| Cron expressions | `schedule("0 9 * * MON-FRI", fn)` with advisory lock | All 3 |
| Dead job caps | 10K max, 180-day retention, auto-prune | All 3 |
| Queue pause/resume | `Queue.pause("name")` / `Queue.resume("name")` | All 3 + CLI |
| Weighted queues | `{ "critical": 5, "default": 3, "low": 1 }` | All 3 |
| Job expiration | `expires: 5m` — discard stale jobs | All 3 |
| Rate limiting | `rate: 100/minute` per job type | All 3 |

---

### Phase 7: Observability & CLI 📋

**Depends on:** Phase 3  
**Estimated effort:** 2-3 days

```bash
ntnt jobs status              # Summary of all queues
ntnt jobs list --pending      # Filter by status
ntnt jobs inspect <job-id>    # Full job details
ntnt jobs retry <job-id>      # Retry a failed/dead job
ntnt jobs cancel <job-id>     # Cancel a pending job
ntnt jobs tail                # Live streaming
ntnt jobs replay <job-id>     # Re-run with same inputs
```

Plus:
- `Queue.stats()` programmatic API (already designed)
- Optional `/jobs/status` HTTP endpoint (localhost only)
- `--format=agent` for LLM-optimized output

---

### Phase 8: Agent-First Features 📋

**Depends on:** Phase 7  
**Estimated effort:** Ongoing

The differentiator. No other job system has this.

| Feature | Description |
|---------|-------------|
| Semantic metadata | `/// Triggers: user.created` parsed from doc comments |
| `ntnt jobs ask` | Natural language queries: "why are emails failing?" |
| `ntnt jobs diagnose` | AI-powered root cause analysis |
| Auto-generated tests | Suggest IDD tests from job code |
| Impact analysis | "If SendEmail fails, what's affected?" |
| Simulation mode | Dry-run with `effect` blocks |
| Intent verification | Did the job *achieve its purpose*, not just run? |
| Job contracts | `requires(args) { ... }` / `ensures(args, result) { ... }` |

---

## Lessons Learned

### From Building Phases 0-6 (First Pass)

1. **Primitives before patterns.** We built the job system before hardening spawn/channels. That's backwards. The job system inherited every primitive rough edge. Fix the foundation first.

2. **Monolith then modularize.** The initial `jobs.rs` grew to 4,483 lines before we refactored into modules. Starting with modules would have been premature — you don't know the right boundaries until you've built the thing. But don't wait too long.

3. **Copilot review is genuinely useful.** Across 4 review rounds, it caught: brace-depth tracking gaps, non-deterministic test modes, missing struct field error handling, indentation edge cases. Not a replacement for human review, but a great first pass.

4. **Test count is a vanity metric.** We had 1,063 tests but most were job-specific. The 26 concurrency primitive tests are more valuable because they test the foundation everything else depends on.

5. **`Rc<RefCell>` as a feature, not a limitation.** The interpreter's single-threaded design forces CSP. We leaned into this with serialized capture and it's genuinely better than trying to make the interpreter thread-safe.

6. **Capture errors are the #1 DX issue.** Users will write `spawn(fn() { my_helper(x) })` and not understand why `my_helper` can't cross the thread boundary. The error message is good but the mental model is surprising.

7. **Feature branches as preservation.** All 6 original branches still exist with working code. The v2 rewrite can cherry-pick proven implementations rather than rewriting from scratch.

### Architecture Decisions That Held Up

- **Thread-per-task** (not threadpool) — simpler, debuggable, good enough for ntnt's use case
- **catch_unwind per task** — panic isolation is essential
- **Cooperative cancellation** — simpler than preemption, works because we control the yield points
- **Monotonic IDs** — no UUID overhead, no collisions within a process
- **LazyLock global runtime** — simple, no initialization ceremony

### Architecture Decisions to Revisit

- **std::sync::mpsc** → should switch to `crossbeam::channel` for `select` support
- **Map-based handles** → should become proper Value variants (Phase 1, Issue 2)
- **5-minute task reaper** → TTL should be configurable, not hardcoded
- **Thread-per-task** may not scale for job workers processing thousands of jobs — consider a threadpool for Phase 3+

---

## Competitive Analysis

### Concurrency Primitives

| Feature | ntnt (current) | Go | Elixir | Rust (tokio) |
|---------|---------------|-----|--------|--------------|
| Spawn task | `spawn(fn)` | `go func()` | `spawn(fn)` | `tokio::spawn(async)` |
| Await result | `await_task(t)` | N/A (channels) | `Task.await(t)` | `.await` |
| Channels | `channel()` | `make(chan)` | N/A (mailbox) | `mpsc::channel()` |
| Select | ❌ **Phase 1** | `select {}` | `receive do` | `tokio::select!` |
| Parallel | ❌ **Phase 2** | `errgroup` | `Task.async_stream` | `join!` |
| Structured | ❌ **Phase 2** | N/A | `Task.Supervisor` | `JoinSet` |
| Schedule | `schedule("5s", fn)` | ticker | `:timer` | `tokio::interval` |
| Cancel | Cooperative | Context | `Task.shutdown` | `CancellationToken` |

### Job Systems

| Feature | ntnt (planned) | Sidekiq (Ruby) | Oban (Elixir) | BullMQ (JS) |
|---------|---------------|----------------|---------------|-------------|
| Declaration | `Job X on q { }` | Class + include | `use Oban.Worker` | Class |
| Multi-backend | Memory+PG+Redis | Redis only | PG only | Redis only |
| Unique jobs | ✅ SHA256 dedup | Pro ($) | ✅ Free | ✅ |
| Transactional | ✅ `enqueue_tx` | ❌ | ✅ (Oban's best feature) | ❌ |
| Cron | ✅ Advisory lock | Enterprise ($) | ✅ Free | ✅ |
| Rate limiting | ✅ | Enterprise ($) | ✅ Pro ($) | ✅ |
| LISTEN/NOTIFY | ✅ | N/A | ✅ | N/A |
| Pause/resume | ✅ | ✅ | ✅ | ✅ |
| Intent verification | ✅ (Phase 8) | ❌ | ❌ | ❌ |
| AI diagnosis | ✅ (Phase 8) | ❌ | ❌ | ❌ |

**Key insight:** ntnt's job system gives away everything Sidekiq charges $250/mo for, plus Oban's transactional enqueue, plus multi-backend flexibility nobody else has.

---

## Open Questions

### Resolved

| Question | Decision | Date |
|----------|----------|------|
| Thread-per-task vs threadpool? | Thread-per-task for primitives. Revisit for job workers. | 2026-03-15 |
| async/await in ntnt? | No. Synchronous model with spawn + channels. | 2026-03-15 |
| Job DSL as syntax vs library? | Syntax (`Job` keyword). Makes it first-class, testable in IDD. | 2026-03-15 |
| One PR or many? | One per phase. Each phase is shippable independently. | 2026-03-16 |

### Open

| Question | Options | Notes |
|----------|---------|-------|
| crossbeam vs std::mpsc? | crossbeam (for select), std (simpler) | Leaning crossbeam |
| Handle types: Value variants vs tagged maps? | Variants (safer), Maps (simpler) | Leaning variants |
| Reaper TTL configurable? | Env var, runtime config, or hardcoded | Needs decision |
| Job workers: thread-per-task or pool? | Pool (for throughput) | Phase 3 decision |
| `select` return format? | `{channel, value}` map vs `(index, value)` tuple | Phase 1 decision |
| Feature flags for backends? | Compile-time (Cargo features) vs runtime | Leaning runtime |
| Where do composition functions live? | Rust (fast) vs ntnt stdlib (extensible) | Both? |

---

## Appendix: Job DSL Design

Full Job DSL syntax, lifecycle, and backend details preserved from the original `background_jobs.md`. See that file for the complete reference (still valid as a design target). Key highlights:

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

### Composition (Future)

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
