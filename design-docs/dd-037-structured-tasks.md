# DD-037: Structured Tasks — Concurrency Primitives for NTNT

**Status:** Draft
**Author:** Larri
**Date:** 2026-03-12
**Depends on:** std/concurrent (channels), std/time (sleep_ms), HTTP bridge architecture
**Enables:** Phase 10 (Job DSL), WebSocket support, background processing

---

## 1. Motivation

Every non-trivial web application needs to do things in the background: clean up expired sessions, send emails after signup, poll an external API, process uploads. NTNT currently has no way to express "do this concurrently" beyond the HTTP request/response cycle.

The roadmap (Phase 10) envisions a full Job DSL with persistent backends and workflow orchestration. But that's a castle — and we haven't poured the foundation yet. This document designs the foundation: the **concurrency primitives** that everything else builds on.

### What We Have Today

| Primitive | Module | What It Does |
|-----------|--------|--------------|
| `channel()` | std/concurrent | Create a typed communication channel |
| `send(ch, val)` | std/concurrent | Send a value through a channel |
| `recv(ch)` | std/concurrent | Blocking receive from a channel |
| `try_recv(ch)` | std/concurrent | Non-blocking receive (returns Option) |
| `close(ch)` | std/concurrent | Close a channel |
| `parallel([fns])` | std/concurrent | Run functions in parallel, wait for all results |
| `sleep_ms(ms)` | std/time | Block the current thread for N milliseconds |
| `on_shutdown(fn)` | (server builtin) | Register a cleanup function for graceful shutdown |

These are good building blocks. But they have a critical gap: **there is no way to start concurrent work that outlives a single expression**. `parallel()` blocks until all tasks complete. There is no `spawn`.

### The Interpreter Constraint

NTNT's interpreter uses `Rc<RefCell<Environment>>` — reference-counted, single-threaded. This is **not** an accident or a limitation to work around. It's a design choice that keeps the interpreter simple and predictable.

The HTTP bridge already solves concurrent request handling by running the interpreter on a dedicated thread and communicating via mpsc channels. **This pattern is the blueprint for all concurrency in NTNT**: the interpreter thread is the single source of truth; concurrent work happens on other threads and communicates results back via channels.

This constraint actually gives us something valuable: **freedom from data races by construction**. Two spawned tasks cannot both mutate the same variable. They must communicate through channels. This is CSP (Communicating Sequential Processes) enforced by architecture, not by programmer discipline.

---

## 2. Design Philosophy

### 2.1 Three Traditions, One Synthesis

We draw from three proven concurrency traditions, taking the best of each while avoiding their pitfalls:

**Hoare's CSP (1978) → Go (2009)**
- Core idea: concurrent processes communicate through channels, not shared memory.
- What Go got right: goroutines are cheap, channels are first-class, `select` multiplexes.
- What Go got wrong: `go` statements are fire-and-forget. Goroutines can leak. No structured lifetime management. Error propagation from goroutines is manual and error-prone. As Nathaniel Smith argues: "the `go` statement is a form of goto for concurrency."

**Erlang/OTP (1986) → Elixir → Gleam**
- Core idea: isolated processes, message passing, supervisor trees. "Let it crash" + automatic restart.
- What Erlang got right: fault isolation is built into the model. A crashing process doesn't take down the system. Supervision trees provide declarative recovery.
- What Erlang got wrong (for our context): the full OTP supervision model is complex and requires a VM designed around it. NTNT runs on Rust/Tokio, not the BEAM.

**Structured Concurrency (2016-present) → Trio, Kotlin, Swift, Java 21**
- Core idea: concurrent lifetimes are scoped. Every spawned task belongs to a parent scope. The scope doesn't exit until all its children complete. Errors propagate upward automatically.
- What structured concurrency got right: no orphaned tasks, no leaked goroutines, error propagation is automatic, reasoning about concurrent code is local (you can understand a scope by reading it).
- What structured concurrency got wrong: nothing, really — it's just not widely adopted yet.

### 2.2 NTNT's Synthesis: Structured Tasks

NTNT's concurrency model combines:

1. **CSP's channels** for communication (we already have these)
2. **Structured concurrency's scoped lifetimes** for safety
3. **Erlang's fault philosophy** for resilience — adapted to a non-BEAM runtime
4. **A web-native `schedule` primitive** for the 90% case

The mental model is simple: **tasks are like function calls that happen concurrently. They belong to a scope, they return results, they propagate errors, and they clean up after themselves.**

### 2.3 Design Principles

1. **No orphans.** Every task has an owner. When the owner exits, all its tasks are cancelled. No leaked goroutines, ever.

2. **Errors propagate, not evaporate.** A failing task surfaces its error to the scope that spawned it. Silent failures are bugs.

3. **Communication over shared state.** Tasks exchange data through channels, not through shared variables. This is enforced by the interpreter architecture, not by linting.

4. **Progressive complexity.** The simplest case (`spawn` a function) should be one line. Supervision, retry, and scheduling layer on top without changing the core model.

5. **Agent-friendly.** An AI coding agent should be able to reason about concurrency from the source code alone. Structured scopes make concurrent behavior visible in the code structure. No hidden background threads, no global event buses, no implicit state.

---

## 3. The Primitives

### 3.1 `spawn(fn) -> Task`

The foundational primitive. Runs a function concurrently and returns a handle.

```ntnt
import { spawn, await_task } from "std/concurrent"

// Fire and await
let task = spawn(fn() {
    let data = fetch("https://api.example.com/users")
    parse_json(data)
})

// ... do other work ...

let users = await_task(task)  // blocks until result is ready
```

**Semantics:**
- `spawn(fn)` starts the function on a worker thread (from the Tokio thread pool).
- Returns a `Task` value (opaque handle).
- The function receives a **snapshot** of captured variables, not references. Mutations inside the spawned function do not affect the parent scope. This is CSP: communicate through channels, not shared state.
- If the spawned function panics/throws, the error is captured in the `Task`. It surfaces when `await_task` is called (as a `Result`).

**What can be captured:**
- Primitives (Int, Float, Bool, String): copied.
- Collections (Array, Map): deep-copied.
- Channels: shared (they're already thread-safe).
- Functions: cannot be captured (closures contain `Rc` references). Must be defined inside the spawn or passed via channel.

```ntnt
// ✅ Works — primitives and channels
let user_id = "123"
let results = channel()
spawn(fn() {
    let data = fetch("/api/users/{user_id}")
    send(results, data)
})

// ❌ Error — cannot capture mutable binding
let mut counter = 0
spawn(fn() {
    counter = counter + 1  // compile error: cannot capture mutable binding in spawn
})
```

### 3.2 `scope(fn(s))` — Structured Task Scope

The key innovation. A `scope` block creates a **structured concurrency boundary**: all tasks spawned within it must complete before the scope exits. Errors in any child task cancel siblings and propagate to the parent.

```ntnt
import { scope } from "std/concurrent"

// All tasks complete before scope exits
let results = scope(fn(s) {
    let t1 = s.spawn(fn() { fetch("/api/users") })
    let t2 = s.spawn(fn() { fetch("/api/orders") })
    let t3 = s.spawn(fn() { fetch("/api/inventory") })

    // scope implicitly awaits all tasks
    // returns when all three are done
    map {
        "users": s.result(t1),
        "orders": s.result(t2),
        "inventory": s.result(t3)
    }
})
// At this point, ALL three fetches are guaranteed complete.
print(results)
```

**Semantics:**
- `scope(fn(s))` creates a `Scope` and passes it to the function.
- `s.spawn(fn)` spawns a task **bound to this scope**.
- `s.result(task)` retrieves the task's result (blocks if not ready).
- When the scope function returns, it waits for all spawned tasks to complete.
- If any task fails, all sibling tasks are cancelled and the error propagates to the scope.

**Why this matters:**

```ntnt
// Go-style (unstructured) — what we're AVOIDING
go fetch_users()    // Who owns this? What if it fails?
go fetch_orders()   // When do these finish? Who knows.
// ... 500 lines later, one of them is still running and leaking memory

// NTNT-style (structured) — what we're DOING
scope(fn(s) {
    s.spawn(fn() { fetch_users() })
    s.spawn(fn() { fetch_orders() })
})
// RIGHT HERE we know both are done. No leaks. No orphans.
```

This is the insight from Nathaniel Smith's "Go statement considered harmful": unstructured concurrency (`go f()`, `thread.spawn()`) is the concurrent equivalent of `goto`. Structured concurrency is the equivalent of structured programming: clear entry, clear exit, predictable lifetime.

### 3.3 Cancellation

Tasks respect **cooperative cancellation**. When a scope is cancelled (due to a sibling failure or explicit cancellation), tasks check for cancellation at yield points.

```ntnt
import { scope, is_cancelled } from "std/concurrent"

scope(fn(s) {
    s.spawn(fn() {
        for item in large_dataset {
            if is_cancelled() { return }  // check cancellation
            process(item)
        }
    })

    s.spawn(fn() {
        // If this task fails...
        let result = risky_operation()
        otherwise { panic("failed!") }
        // ...the sibling task above gets cancelled
    })
})
```

**Automatic cancellation points:**
- `recv(ch)` — checks cancellation while waiting
- `sleep_ms(ms)` — checks cancellation during sleep
- `fetch()` / HTTP calls — checks cancellation during I/O wait
- `await_task(t)` — checks cancellation while waiting

Users rarely need to call `is_cancelled()` manually — the stdlib functions handle it.

### 3.4 `schedule(interval, fn)` — Recurring Tasks

The web-native primitive. Declares a function that runs on a recurring schedule, tied to the server's lifecycle.

```ntnt
import { schedule } from "std/concurrent"

// Run every hour
schedule("every 1h", fn() {
    let expired = db.query("SELECT id FROM sessions WHERE expires_at < now()")
    for session in expired {
        db.delete("sessions", session.id)
    }
    log("Cleaned {len(expired)} expired sessions")
})

// Run every 5 minutes
schedule("every 5m", fn() {
    let health = fetch("https://api.stripe.com/health")
    if health.status != 200 {
        alert("Stripe is down!")
    }
})

// Run every 30 seconds
schedule("every 30s", fn() {
    kv_set("stats:active_users", count_active_users())
})

listen(8080)
```

**Semantics:**
- `schedule(interval, fn)` registers a recurring task with the server runtime.
- The task runs on a background thread (not the interpreter thread).
- Interval format: `"every Ns"`, `"every Nm"`, `"every Nh"` (seconds, minutes, hours).
- Future: cron syntax `"0 * * * *"` for complex schedules.
- Scheduled tasks are **automatically cancelled** on server shutdown (integrates with `on_shutdown`).
- If a scheduled function takes longer than the interval, the next execution is skipped (no overlap).
- Errors in scheduled functions are logged, not fatal. The schedule continues.

**Why not `setInterval`?**
- `schedule` is declarative: it says *what* should happen *when*, not *how* to set up a timer.
- `schedule` is lifecycle-aware: it starts with the server and stops with the server.
- `schedule` has built-in overlap protection and error handling.
- `schedule` reads like intent: "clean up sessions every hour" vs "set an interval of 3600000 milliseconds to call this callback."

### 3.5 `after(ms, fn)` — Delayed One-Shot

A convenience for "do this once, later." Syntactic sugar over `spawn` + `sleep_ms`.

```ntnt
import { after } from "std/concurrent"

// Send welcome email 5 seconds after signup
after(5000, fn() {
    send_welcome_email(user.email)
})
```

Equivalent to:
```ntnt
spawn(fn() {
    sleep_ms(5000)
    send_welcome_email(user.email)
})
```

The `after` version is more readable and communicates intent. It also integrates with server lifecycle (cancelled on shutdown).

---

## 4. Error Handling Strategy

### 4.1 The Erlang Lesson: Failure Is Normal

Erlang's deepest insight isn't actors or message passing — it's that **failure is a normal part of system operation**, not an exceptional case to prevent at all costs. Instead of writing defensive code to prevent every possible failure, you build systems that recover from failure automatically.

NTNT adapts this philosophy within structured concurrency:

| Primitive | On Error |
|-----------|----------|
| `spawn` | Error captured in `Task`. Surfaces on `await_task()` as `Err`. |
| `scope` | Error in any child cancels siblings, propagates to scope as `Err`. |
| `schedule` | Error logged. Schedule continues. After N consecutive failures, alerts. |
| `after` | Error logged. One-shot, no retry. |

### 4.2 Scope Error Policies

Scopes support configurable error policies:

```ntnt
// Default: fail-fast (cancel siblings on first error)
scope(fn(s) {
    s.spawn(fn() { fetch_users() })     // if this fails...
    s.spawn(fn() { fetch_orders() })    // ...this gets cancelled
})

// Shield: isolate failures (siblings continue)
scope(fn(s) {
    s.policy("shield")
    s.spawn(fn() { fetch_users() })     // if this fails...
    s.spawn(fn() { fetch_orders() })    // ...this keeps running
    // scope collects all results/errors
})
```

Two policies:
- **`"fail_fast"`** (default): First failure cancels all siblings. Good for dependent tasks.
- **`"shield"`**: Tasks are isolated. Failures are collected. Good for independent tasks where partial results are acceptable.

### 4.3 Retry with Backoff

For spawned tasks that might transiently fail:

```ntnt
import { spawn_with } from "std/concurrent"

let task = spawn_with(map {
    "retry": 3,
    "backoff": "exponential",  // 1s, 2s, 4s
    "timeout": 30000           // 30s total timeout
}, fn() {
    fetch("https://flaky-api.example.com/data")
})
```

This replaces the need for manual retry loops. The retry logic runs within the task's thread, not the interpreter thread.

---

## 5. The Schedule + Server Lifecycle

### 5.1 Lifecycle Integration

Scheduled tasks are first-class members of the server lifecycle:

```
Server Start
    │
    ├── Register routes
    ├── Register middleware
    ├── Register scheduled tasks ←── schedule() calls
    ├── Register shutdown hooks ←── on_shutdown() calls
    │
    ▼
    listen(8080)
    │
    ├── HTTP request handling (interpreter thread)
    ├── Scheduled task execution (background threads)  ←── runs concurrently
    │
    ▼
    SIGTERM / shutdown
    │
    ├── Cancel all scheduled tasks
    ├── Wait for in-flight tasks to complete (grace period)
    ├── Run on_shutdown hooks
    └── Exit
```

### 5.2 Hot Reload Interaction

In development mode (`ntnt dev`), hot reload re-executes the source file on changes. Scheduled tasks need special handling:

- On reload: cancel all existing scheduled tasks, re-register from the new source.
- This matches how `listen()` and `on_shutdown()` already work in hot-reload mode.
- Scheduled tasks are skipped during the hot-reload re-execution (like `listen()` is), then re-registered with the runtime.

### 5.3 Schedule as a First-Class Server Primitive

`schedule()` joins `listen()`, `on_shutdown()`, and `use_middleware()` as server-level builtins. It's not a library function — it's a declaration of server behavior:

```ntnt
// This reads as a server specification:
use_middleware(logger)
use_middleware(auth)

schedule("every 1h", cleanup_sessions)
schedule("every 5m", health_check)

get("/", home_page)
post("/api/users", create_user)

on_shutdown(fn() { flush_logs() })

listen(8080)
```

The entire server's behavior — routes, middleware, scheduled work, and shutdown — is visible in one place. An agent (or human) reading this file understands the complete system.

---

## 6. Implementation Architecture

### 6.1 Thread Model

```
┌─────────────────────────────────────────────────────────────────┐
│                    Tokio Async Runtime                           │
│                                                                 │
│  ┌──────────┐  ┌──────────┐  ┌───────────┐  ┌──────────────┐  │
│  │ HTTP     │  │ HTTP     │  │ Scheduled │  │ Spawned      │  │
│  │ Handler 1│  │ Handler N│  │ Task Loop │  │ Task Pool    │  │
│  └────┬─────┘  └────┬─────┘  └─────┬─────┘  └──────┬───────┘  │
│       │              │              │               │           │
│       └──────────────┼──────────────┼───────────────┘           │
│                      │              │                           │
│              ┌───────▼──────────────▼──────┐                    │
│              │    Bridge Channel (mpsc)     │                    │
│              └──────────────┬──────────────┘                    │
└─────────────────────────────┼───────────────────────────────────┘
                              │
┌─────────────────────────────▼───────────────────────────────────┐
│                     Interpreter Thread                           │
│                                                                 │
│    Rc<RefCell<Environment>>  (single-threaded, safe)             │
│                                                                 │
│    loop {                                                        │
│        match rx.recv() {                                         │
│            HttpRequest(req, reply) => handle_http(req, reply)    │
│            TaskRequest(fn, reply) => eval_fn(fn, reply)          │
│            ScheduledTask(fn) => eval_fn(fn, _)                   │
│        }                                                         │
│    }                                                             │
└─────────────────────────────────────────────────────────────────┘
```

**Key insight:** Spawned tasks that need interpreter access (calling ntnt functions, accessing state) must go through the bridge channel, just like HTTP requests. Tasks that do pure computation or I/O (fetch, sleep, file ops) can run entirely on worker threads.

This creates two categories of spawned work:
1. **I/O tasks**: Run on Tokio threads. Don't touch interpreter state. Call stdlib functions that are thread-safe (fetch, file I/O, sleep, channel ops).
2. **Interpreter tasks**: Need to evaluate ntnt expressions. Queued to the interpreter thread via the bridge. Inherently sequential (but interleaved with other bridge requests).

For v1, all spawned tasks are **I/O tasks** — they can use stdlib functions and channels but cannot evaluate arbitrary ntnt expressions. This is a pragmatic constraint that matches the 90% use case (background I/O, scheduled cleanup, etc.) while keeping the implementation simple.

### 6.2 Value Serialization

Spawned tasks receive **serialized copies** of captured values, using the existing `SerializedValue` infrastructure from `std/concurrent`:

```rust
enum SerializedValue {
    Unit, Int(i64), Float(f64), Bool(bool), String(String),
    Array(Vec<SerializedValue>),
    Map(HashMap<String, SerializedValue>),
    // Channels are shared via Arc, not serialized
}
```

Functions, closures, and mutable references **cannot** be serialized. Attempting to capture them in a `spawn` block is a compile-time (typechecker) error.

### 6.3 Task Handle

```rust
pub struct TaskHandle {
    id: u64,
    result: Arc<Mutex<Option<Result<SerializedValue, String>>>>,
    cancel: tokio::sync::watch::Sender<bool>,
    join: Option<tokio::task::JoinHandle<()>>,
}
```

In ntnt, this is exposed as a `Task` value (opaque):
```ntnt
let t = spawn(fn() { ... })
type(t)  // "Task"
let result = await_task(t)  // blocks, returns Result
```

### 6.4 Scope Implementation

A scope is a collection of task handles with a cancellation token:

```rust
pub struct TaskScope {
    tasks: Vec<TaskHandle>,
    cancel_all: tokio::sync::watch::Sender<bool>,
    policy: ScopePolicy,
}

enum ScopePolicy {
    FailFast,  // cancel siblings on first error
    Shield,    // collect all results/errors
}
```

When the scope function returns:
1. Wait for all tasks to complete (or be cancelled).
2. Collect results.
3. If FailFast and any task errored, return the first error.
4. If Shield, return a map of task_id → Result.

---

## 7. Comparison Table

| Feature | Go | Erlang/Elixir | Trio (Python) | Kotlin | Swift | **NTNT** |
|---------|-----|---------------|---------------|--------|-------|----------|
| Spawn primitive | `go f()` | `spawn(fn)` | `nursery.start_soon(fn)` | `launch { }` | `Task { }` | `spawn(fn)` / `s.spawn(fn)` |
| Structured? | ❌ | ❌ (manual) | ✅ (nursery) | ✅ (coroutineScope) | ✅ (TaskGroup) | ✅ (scope) |
| Error propagation | Manual | Supervisor links | Automatic | Automatic | Automatic | Automatic |
| Cancellation | Context (manual) | Process.exit | Automatic | Automatic | Automatic | Cooperative (auto at I/O points) |
| Communication | Channels | Messages | Channels | Channels/Flow | AsyncSequence | Channels |
| Scheduling | External (cron) | `:timer` module | External | External | External | Built-in `schedule()` |
| Orphan prevention | ❌ (goroutine leaks) | ❌ (process leaks) | ✅ | ✅ | ✅ | ✅ |
| Web-native | ❌ | ✅ (Phoenix) | ❌ | ❌ | ❌ | ✅ |

**Where NTNT is genuinely novel:**

1. **`schedule` as a server-level primitive.** No other language has recurring task scheduling built into the server lifecycle at the language level. Everyone else delegates to external tools (cron, Sidekiq, Celery) or library-level solutions.

2. **Structured concurrency + web server integration.** Trio, Kotlin, and Swift have structured concurrency, but none integrate it with an HTTP server lifecycle. NTNT's scope/spawn/schedule are designed to work alongside routes, middleware, and shutdown hooks as equal citizens.

3. **CSP enforced by architecture, not convention.** Go says "don't communicate by sharing memory" but lets you do it anyway. NTNT physically can't — the interpreter's `Rc<RefCell<>>` architecture makes shared mutation impossible across spawn boundaries. Channels aren't a recommendation; they're the only option.

---

## 8. What This Enables

### 8.1 Immediate Use Cases

```ntnt
// Session cleanup
schedule("every 1h", fn() {
    pg_execute("DELETE FROM sessions WHERE expires_at < now()")
})

// Health monitoring
schedule("every 30s", fn() {
    let status = fetch("https://api.stripe.com/health")
    if status.code != 200 {
        send(alert_channel, "Stripe API is down!")
    }
})

// Parallel API aggregation
let dashboard = scope(fn(s) {
    let users = s.spawn(fn() { fetch("/api/users") })
    let revenue = s.spawn(fn() { fetch("/api/revenue") })
    let alerts = s.spawn(fn() { fetch("/api/alerts") })

    map {
        "users": s.result(users),
        "revenue": s.result(revenue),
        "alerts": s.result(alerts)
    }
})

// Delayed follow-up
post("/api/signup", fn(req) {
    let user = create_user(req.body)
    after(5000, fn() {
        send_welcome_email(user.email)
    })
    json(map { "id": user.id })
})
```

### 8.2 Path to Phase 10 (Job DSL)

The Job DSL becomes syntactic sugar over these primitives:

```ntnt
// This Job declaration...
Job SendWelcomeEmail on emails (retry: 3, timeout: 30s) {
    perform(user_id: String) {
        let user = db.find(user_id)
        email.send(user.email, "Welcome!")
    }
}

// ...desugars to something like:
fn _job_SendWelcomeEmail(user_id: String) {
    spawn_with(map { "retry": 3, "timeout": 30000, "queue": "emails" }, fn() {
        let user = db.find(user_id)
        email.send(user.email, "Welcome!")
    })
}
```

The persistent backends (PostgreSQL, Redis) add durability on top of the same spawn/channel architecture, rather than being a separate system.

---

## 9. Implementation Plan

### Phase 1: `spawn` + `await_task` (Minimal)
- Add `spawn(fn)` to `std/concurrent`
- Returns `Task` value
- `await_task(task)` blocks and returns `Result`
- `try_await(task)` non-blocking check
- Error capture in task handle
- Typechecker: warn on capturing non-serializable values
- **Tests:** spawn + await, spawn + error, spawn + channel communication

### Phase 2: `scope` (Structured Concurrency)
- Add `scope(fn(s))` to `std/concurrent`
- `s.spawn(fn)` for scoped tasks
- `s.result(task)` for retrieving results
- FailFast cancellation policy
- Cooperative cancellation at I/O points
- **Tests:** scope + all-succeed, scope + fail-fast, scope + cancellation

### Phase 3: `schedule` + `after` (Web Integration)
- Add `schedule(interval, fn)` as server builtin
- Interval parsing ("every Ns/Nm/Nh")
- Integration with server lifecycle (auto-cancel on shutdown)
- Overlap prevention (skip if previous execution still running)
- `after(ms, fn)` convenience
- Hot-reload support (re-register schedules)
- **Tests:** schedule + execution, schedule + shutdown, schedule + overlap

### Phase 4: Polish
- Shield scope policy
- `spawn_with` (retry, backoff, timeout)
- Concurrency limits (max spawned tasks)
- Observability: `tasks()` returns list of active tasks
- Documentation and examples

---

## 10. Open Questions

1. **Should `spawn` be allowed outside server context?** Currently leaning yes — it's useful in CLI scripts too. But lifecycle management is simpler in server context.

2. **Cron syntax for schedule?** The `"every Nh"` format covers 90% of cases. Full cron (`"0 */2 * * *"`) adds complexity. Defer to Phase 4?

3. **Task naming?** Should tasks have optional names for debugging? e.g., `spawn("fetch-users", fn() { ... })`. Probably yes, but optional.

4. **Max concurrency?** Should there be a global limit on spawned tasks? Default of 1000? Configurable? Erlang allows millions of processes, Go allows millions of goroutines, but we're on a thread pool.

5. **Integration with `otherwise`?** Should `await_task(t) otherwise { default_value }` work? It's a natural fit since `await_task` returns a `Result`.

---

## 11. What We're NOT Building (Yet)

- **Job persistence** (PostgreSQL/Redis backends) — Phase 10.3
- **Job DSL syntax** (`Job` keyword) — Phase 10.1, after primitives are proven
- **Workflows/Chains/Batches** — Phase 10.4
- **WebSocket support** — Phase 10.5 (separate design doc)
- **Actor model** — Considered and rejected for v1. Actors add complexity without clear benefit for NTNT's use case. Structured scopes provide the safety guarantees actors are typically used for.
- **Async/await** — Considered and rejected. NTNT's synchronous model is simpler. Spawn + channels achieve the same effect without coloring all functions async.

---

## 12. References

- Hoare, C.A.R. "Communicating Sequential Processes" (1978) — the foundation
- Smith, Nathaniel J. "Notes on structured concurrency, or: Go statement considered harmful" (2018) — the argument for structured concurrency
- Sústrik, Martin. "Structured Concurrency" (2016) — original formulation
- Elizarov, Roman. "Structured concurrency" (2018) — Kotlin's approach
- Armstrong, Joe. "Making reliable distributed systems in the presence of software errors" (2003) — Erlang's "let it crash"
- Zig devlog 2026-03-10 — inspiration for better diagnostics in concurrent error reporting
