# DD-037 Phase 2: Job DSL + KV Backend — Implementation Plan

**Status:** PR 2a ✅ merged, PR 2b ✅ merged, PR 2c 🔄 in review
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md)
**Created:** 2026-03-16
**Target branch:** `feat/job-dsl-v2`
**Base:** `main` (includes Phase 1 structured concurrency — `TxChannelHandle`/`RxChannelHandle`, `select()`, `try_await` states, `ConcurrencyRuntime`)

---

## Guiding Principles

1. **Small PRs.** 3 focused PRs, each independently shippable and reviewable.
2. **Persistent from day one.** No memory backend. SQLite KV default, Redis/Valkey for prod.
3. **Build on std/concurrent.** Workers use `spawn()`. No reinventing thread management.
4. **Build on std/kv.** No raw Redis/SQLite calls. If std/kv is missing an operation, add it to std/kv first.
5. **Free functions everywhere.** `enqueue(JobName, args)`, not `JobName.enqueue(args)`.
6. **Each sub-phase is independently shippable.** PR 2a works without 2b. 2b works without 2c.

---

## Architecture Notes (Read Before Implementing)

### Job Registry Lives in JOB_RUNTIME, Not Per-Interpreter

`job_registry: HashMap<String, JobDefinition>` belongs in global state, **not** in `Interpreter`. Workers use `run_in_fresh_interpreter()` — a fresh interpreter has no job definitions.

**Do not add to `ConcurrencyRuntime`.** That struct (in `concurrent.rs`) owns tasks/channels/schedules — jobs are a separate concern. Instead, create a parallel `JOB_RUNTIME: LazyLock<JobRuntime>` in `src/stdlib/jobs.rs`, same pattern as `RUNTIME` in `concurrent.rs`:

```rust
pub struct JobRuntime {
    job_registry: RwLock<HashMap<String, JobDefinition>>,
    kv_handle: Mutex<Option<KvHandle>>,        // lazy-init KV connection
    test_queue: Mutex<Option<Vec<EnqueuedJob>>>, // Some(...) when in testing mode
}

pub static JOB_RUNTIME: LazyLock<JobRuntime> = LazyLock::new(JobRuntime::new);
```

`eval_statement` for `Statement::Job` writes to `JOB_RUNTIME.job_registry`. Workers read from `JOB_RUNTIME.job_registry` when dispatching claimed jobs. Workers still call `RUNTIME.spawn()` for their threads — they just don't live inside `ConcurrencyRuntime`.

### Worker Startup Sequence

The full lifecycle from CLI to job execution:

1. `ntnt worker server.tnt` → interpreter loads and evaluates `server.tnt` top-level
2. Every `Statement::Job` encountered → `JobDefinition` registered in `JOB_RUNTIME.job_registry`
3. If jobs are defined in a module (`lib/jobs.tnt`), that module **must be imported** for jobs to register
4. `work_jobs()` called → reads `JOB_RUNTIME.job_registry` for dispatch
5. Worker loop: claims job by name from KV, looks up `JobDefinition` in `JOB_RUNTIME.job_registry`, runs `perform_body` in fresh interpreter with job args injected as local scope

Cross-file jobs require explicit `import "lib/jobs.tnt"` in the entrypoint. No magic discovery.

### enqueue() API: String, Not Identifier

Pin this down now: `enqueue("SendEmail", args)` — **string literal, not identifier**. The examples in the doc inconsistently use identifier form (`enqueue(SendEmail, args)`); ignore those. String lookup is simpler to implement, easier to read, and consistent with how routes work. Update examples to use string form before implementation starts.

### configure_queue() Lazy Init

If `enqueue()` is called before `configure_queue()`: auto-initialize with SQLite default (`"sqlite:./jobs.db"`). First call to `enqueue()` triggers lazy init. `configure_queue()` can be called explicitly to override or change settings, but is not required.

---

## Performance Impact: None

The job system is entirely additive. It does not touch the HTTP request hot path:

- `Job` declarations evaluate once at module load (like `fn` declarations)
- `enqueue()` is a KV write — same cost as `execute()` for a DB query, only when explicitly called
- Workers run on `spawn()` threads, separate from the request interpreter thread
- One new match arm in the eval loop for `Statement::Job` — negligible

The 0.4.2 async connection pool and worker pool performance gains are unaffected.

---

## Sub-Phases

```
PR 2a  ✅  Parser + Registry + Enqueue MVP       parse Job syntax, store in KV, basic enqueue           (#32, merged 2026-03-17)
PR 2b  ✅  Workers + Lifecycle + Retry            claim jobs, run via spawn(), retry on failure          (#33, merged 2026-03-17)
PR 2c  🔄  DX: Testing Mode + Logs + CLI + Docs   assert_enqueued, streaming logs, ntnt worker    (#34, in review)
```

### Estimated total: 6-8 days across 3 PRs

---

## PR 2a: Parser + Registry + Enqueue MVP

**Goal:** `Job` syntax parses, jobs register, `enqueue()` writes to KV. No workers yet — jobs sit in the queue waiting to be claimed. This validates the entire front half of the pipeline.

**Estimated effort:** 2-3 days

### Lexer (`src/lexer.rs`)
- [x] ~~Add `Job` keyword token~~ → `job` is a contextual keyword (parsed as identifier, not token), avoids breaking existing code using `job` as a variable name
- [x] `on` is already a keyword (used by `on_shutdown`, `on_error`) — no change needed
- [x] `perform` and `on_failure` are parsed contextually inside `Job` blocks, not keywords

### AST (`src/ast.rs`)
- [x] Add `Statement::Job` variant:
  ```rust
  Job {
      name: String,              // "SendEmail"
      queue: String,             // "emails"
      options: Vec<(String, Expression)>,  // retry: 5, timeout: 120
      perform_params: Vec<Parameter>,
      perform_body: Box<Block>,
      on_failure: Option<(Vec<Parameter>, Block)>,
  }
  ```
### Parser (`src/parser.rs`)
- [x] Parse `job Name on queue { perform(args) { body } }` syntax
- [x] Parse optional inline options: `job Name on queue (retry: 5, timeout: 120) { ... }`
- [x] Parse optional `on_failure(error, attempt) { ... }` block
- [x] `job` declarations are top-level statements (like `fn`, `struct`, `enum`)

### Interpreter — Job Registry (`src/interpreter.rs`)
- [x] Add `eval_statement` arm for `Statement::Job`: evaluate options, write `JobDefinition` to `JOB_RUNTIME.job_registry` (see Architecture Notes — registry is global, not per-interpreter)
- [x] Job names must be unique — error on duplicate registration
- [x] Execution mode guards: skip job registration in HotReload worker mode (same pattern as `spawn`)

### KV Integration (`src/stdlib/jobs.rs` — new file)
- [x] `configure_queue(opts)` — takes a map with `"store"` key
  - Default: `"sqlite:./jobs.db"` if no store specified
  - Opens a KV connection via `std/kv::open()`, stores handle in module-level state
  - Validates store URL format
- [x] `enqueue(job_name, args)` — the core enqueue function:
  - Look up job name in registry (error if not registered)
  - Generate job ID (UUID via `std/crypto::uuid()`)
  - Serialize job data: `{ type, queue, payload, status: "pending", attempts: 0, created_at, ... }`
  - Write to KV: `set(kv, "jobs:data:<id>", job_data)` + add to queue sorted set
  - Return job ID as String
- [x] `job_status(job_id)` — read job data from KV, return status map
- [x] `cancel_job(job_id)` — set status to "cancelled", remove from queue

### std/kv approach for PR 2a

**Queue ordering: key-prefix approach (decided).** No sorted set operations needed. Use zero-padded ISO timestamp prefix in the KV key:

```
jobs:pending:<zero-padded-timestamp>:<id>   →  natural lexicographic ordering
jobs:data:<id>                               →  full job data (status, payload, attempts, etc.)
jobs:active:<id>                             →  TTL key for visibility timeout (PR 2b)
```

`list(kv, "jobs:pending:")` returns keys in lexicographic order — SQLite's `ORDER BY key`, Redis sorts client-side after SCAN. Zero-padded timestamps sort correctly. **No new std/kv operations needed for PR 2a.**

**Atomic claiming (PR 2b concern, not PR 2a):** Deferred. When PR 2b is started, add a `claim(kv, prefix)` operation to std/kv that does `BEGIN IMMEDIATE; SELECT..LIMIT 1; UPDATE; COMMIT` for SQLite and `ZPOPMIN` / Lua script for Redis. Do not implement in PR 2a.

### Typechecker (`src/typechecker.rs`)
- [x] Add `Job` to statement checking — type-checks perform body, on_failure body, and option expressions
- [x] Add signatures: `configure_queue`, `enqueue`, `job_status`, `cancel_job`

### Build system (`build.rs`)
- [x] `src/stdlib/jobs.rs` will be auto-discovered by build.rs glob — no config needed
- [x] Add `// @ntnt` doc blocks to all functions (build enforces this)

### Tests
- [x] Parser test: `job SendEmail on emails { perform(to) { print(to) } }` parses correctly
- [x] Parser test: options, on_failure handler
- [x] Integration test: register job → enqueue → verify in KV → check status
- [x] Integration test: cancel job
- [x] Integration test: enqueue unregistered job → error
- [x] Integration test: duplicate job name → error

### PR 2a Completion Criteria
- [x] All tests pass (`cargo test`)
- [x] `cargo build --release --locked && ntnt docs --generate` — CI will fail on docs drift
- [x] `// @ntnt` doc blocks on all new public functions
- [x] Typechecker signatures complete (not partial — Copilot will flag this)
- [x] No `eprintln!` for error handling — return `Err(...)` or `Value::Error`

### What this PR does NOT include
- No workers (jobs sit in queue)
- No retry logic
- No scheduling (`enqueue_at`, `enqueue_in`)
- No batch enqueue
- No dedup or expiration
- No streaming logs

---

## PR 2b: Workers + Lifecycle + Retry

**Goal:** Jobs actually run. Workers claim jobs from KV, execute via `spawn()`, handle success/failure, retry with backoff. This is the "it works end-to-end" PR.

**Estimated effort:** 2-3 days
**Depends on:** PR 2a merged

### Worker Loop (`src/stdlib/jobs.rs`)
- [x] `work_async(opts?)` — starts worker loop(s) via `std::thread::spawn`, integrated with ConcurrencyRuntime for cancellation/await
  - Poll KV queue for pending jobs (configurable poll interval, default 1s)
  - Claim job: atomically via `kv_claim()` (`BEGIN IMMEDIATE; SELECT; DELETE; COMMIT`)
  - Deserialize job data, look up `JobDefinition` in registry
  - Run `perform` body in a fresh `Interpreter::new()` with params injected from payload
  - On success: status → "completed", record `completed_at`
  - On failure: increment attempts, check retry policy, either re-queue or mark dead
  - Cancellation-aware: checks `is_current_task_cancelled()` between iterations
- [x] `work_jobs(opts?)` — blocking worker mode for `ntnt worker` CLI:
  - Same logic as `work_async()` but runs on the calling thread
  - `opts.concurrency`: number of concurrent `spawn()` workers (default: 1)
  - `opts.queues`: array of queue names to process (default: all)
- [ ] Worker heartbeat: periodic KV write with TTL — deferred to PR 2c (visibility timeout TTL key is set on claim)

### Job Lifecycle State Machine
```
Scheduled ─→ Pending ─→ Active ─→ Completed
                │          │
                │          ├─→ Failed (retries remaining) ─→ Pending (retry)
                │          │
                │          └─→ Dead (retries exhausted)
                │
                └─→ Cancelled
```
- [x] All state transitions are KV writes
- [x] `completed_at`, `failed_at`, `dead_at` timestamps recorded
- [x] `error` field stores last failure message

### Retry Logic
- [x] Retry count from job options: `retry: 5` (default: 3)
- [x] Backoff strategies:
  - `exponential` (default): delay = base * 2^attempt (base = 5s, capped at 3600s)
  - `linear`: delay = base * attempt
  - `constant`: delay = base
- [x] Syntax: `job X on q (retry: 5, backoff: "exponential") { ... }`
- [x] Retry delay → re-enqueue with `scheduled_at` in the future
- [x] `on_failure(error, attempt)` hook called on each failure (before retry decision)

### Scheduled Jobs
- [x] `enqueue_at(job_name, timestamp_nanos, args)` — enqueue with future `scheduled_at`
- [x] `enqueue_in(job_name, delay_seconds, args)` — convenience wrapper
- [x] Worker skips jobs where `scheduled_at > now()` during polling (re-enqueues them)
- [x] Pending key timestamp controls ordering — scheduled jobs sort after immediate ones

### Job Timeout
- [ ] `timeout` option — deferred to PR 2c (post-execution elapsed check)

### Visibility Timeout
- [x] Active jobs get a KV TTL key: `jobs:active:<id>` with 300s TTL
- [ ] If worker dies mid-job, TTL expires, and job becomes re-claimable — recovery mechanism deferred
- [ ] Worker refreshes TTL periodically during long jobs — deferred to PR 2c

### Graceful Shutdown
- [x] Cancellation-aware: `cancel_task()` on the worker TaskHandle stops claiming new jobs
- [ ] Configurable drain timeout — deferred to PR 2c

### Tests
- [ ] Integration: enqueue → work_async → job runs → status is completed (requires subprocess test, deferred)
- [ ] Integration: job fails → retries N times → eventually dead (requires subprocess test, deferred)
- [ ] Integration: job fails → on_failure handler called (requires subprocess test, deferred)
- [ ] Integration: enqueue_in → job doesn't run until delay passes (requires subprocess test, deferred)
- [ ] Integration: cancel pending job → worker skips it (requires subprocess test, deferred)
- [ ] Integration: job timeout → treated as failure (deferred with timeout feature)
- [ ] Integration: graceful shutdown (deferred with drain timeout)
- [x] Unit: backoff calculation (exponential, linear, constant, default)
- [x] Unit: execute_job_perform with empty body
- [x] Unit: enqueue_at and enqueue_in
- [x] Unit: type error handling for enqueue_at/enqueue_in
- [x] Unit: parse_work_opts defaults and custom values

### PR 2b Completion Criteria
- [x] All tests pass (`cargo test`) — 724 lib, 314 integration
- [x] `cargo build --release --locked && ntnt docs --generate` — 373 total functions
- [x] `// @ntnt` doc blocks on all new public functions in jobs.rs
- [x] Typechecker signatures complete for `work_async`, `work_jobs`, `enqueue_at`, `enqueue_in`
- [x] No `eprintln!` in new code

### What this PR does NOT include
- No testing mode (assert_enqueued)
- No streaming logs
- No dedup, expiration, batch, priority, weighted queues
- No `ntnt worker` CLI command (work_jobs exists but CLI wiring is PR 2c)
- No docs update yet

---

## PR 2c: DX — Testing Mode + Logs + CLI + Docs

**Goal:** Make the job system developer-friendly. Testing helpers, observability, CLI, documentation. This is the polish PR that makes Phase 2 shippable.

**Estimated effort:** 2 days
**Depends on:** PR 2b merged

### Items deferred from PR 2b

- [x] **Job timeout** — worker checks elapsed time after execution; treat timeout as failure
- [x] **`work_jobs()` cooperative cancellation** — Ctrl-C signal handler via `ctrlc` crate sets `CURRENT_TASK_CANCELLED`
- [x] **`work_async` return type consistency** — always returns `Array<TaskHandle>`
- [ ] ~~**Redis atomic claim via Lua script**~~ → deferred to [Phase 3](dd-037-phase-3-implementation.md)
- [ ] ~~**Worker heartbeat refresh**~~ → deferred to [Phase 3](dd-037-phase-3-implementation.md)
- [ ] ~~**Graceful shutdown drain timeout**~~ → deferred to [Phase 3](dd-037-phase-3-implementation.md)
- [ ] ~~**`work_async` partial spawn cleanup**~~ → deferred to [Phase 3](dd-037-phase-3-implementation.md) (edge case, task limit rarely hit)
- [ ] ~~**Scheduled job claim optimization**~~ → deferred to [Phase 3](dd-037-phase-3-implementation.md)

### Testing Mode
- [x] `configure_queue(map { "mode": "testing" })` — jobs collected in memory, nothing runs
- [x] `assert_enqueued(job_name, args?)` — verify a job was enqueued (partial match on args)
- [x] `assert_not_enqueued(job_name)` — verify no jobs of this type were enqueued
- [x] `drain_jobs()` — run all collected jobs synchronously in current thread
- [x] `clear_jobs()` — reset test queue between tests
- [x] Testing mode intercepts `enqueue()` — no KV writes happen (already worked in PR 2a via test_queue)

### Streaming Logs
- [x] Every job event emits a structured JSON log line to stderr
- ~~`on_job_event(fn(event))`~~ — removed, deferred to [Phase 3](dd-037-phase-3-implementation.md) (cross-thread closure design needed)
- [x] Events: `job.enqueued`, `job.started`, `job.completed`, `job.failed`, `job.dead`

### Deduplication & Expiration
- ~~deferred to [Phase 3](dd-037-phase-3-implementation.md)~~

### Batch Enqueue
- ~~deferred to [Phase 3](dd-037-phase-3-implementation.md)~~

### Priority Queues
- ~~deferred to [Phase 3](dd-037-phase-3-implementation.md)~~

### CLI: `ntnt worker`
- [x] `ntnt worker server.tnt` — start workers for jobs defined in server.tnt
- [x] `--concurrency N` — number of concurrent workers (default: 1)
- [x] `--queues emails,payments` — which queues to process
- [x] `--poll-interval N` — poll interval in milliseconds
- [x] Loads the .tnt file (registers jobs), then calls `work_jobs()` in blocking mode
- [x] Ctrl+C → graceful shutdown via `ctrlc` handler

### CLI: `ntnt jobs`
- [x] `ntnt jobs server.tnt` — summary of all queues (pending/active/completed/failed/dead counts)
- [ ] ~~`ntnt jobs list`~~ → deferred to [Phase 3](dd-037-phase-3-implementation.md)

### Documentation
- [x] `// @ntnt` doc blocks on all new functions in `src/stdlib/jobs.rs`
- [x] Typechecker signatures for all functions (5 new)
- [x] `cargo build --release` + `ntnt docs --generate` — 378 total functions
- [x] `docs/AI_AGENT_GUIDE.md` — Background Jobs section with full syntax reference
- [x] CLAUDE.md + copilot-instructions.md auto-synced
- [x] Example: `examples/job_demo.tnt`
- [x] Update DD-037 Phase 2 checkboxes

### Tests
- [x] Testing mode: assert_enqueued (found, partial match, not found), assert_not_enqueued (pass, fail), drain_jobs, clear_jobs — 8 new tests
- [ ] ~~Dedup, expiration, batch, priority~~ → deferred to Phase 3
- [ ] CLI subprocess tests → deferred (requires test infrastructure for long-running processes)
- [ ] Integration subprocess tests (retries → dead, on_failure, enqueue_in delay, graceful shutdown) → deferred

---

## Pre-Implementation Checklist

- [x] **Branch**: `feat/job-dsl-v2` (PR 2a, merged), `feat/job-workers-v2` (PR 2b, in review)
- [x] **std/kv approach confirmed**: Key-prefix ordering. `kv_claim()` added in PR 2b for atomic claiming.
- [x] **enqueue() API**: String literal — `enqueue("SendEmail", args)`. All examples use this form.

---

## Implementation Notes

**Hard rules:**
1. Everything is written fresh from `main`. No external references.
2. No `eprintln!` — all errors returned via `Err(...)` or `Value::Error`
3. Complete `// @ntnt` doc blocks on every new public function — build fails if missing
4. Complete typechecker signatures — partial sigs will be flagged in review
5. Run `cargo build --release --locked && ntnt docs --generate` before every push
6. Test count baseline: ~383 `#[test]` functions (post Phase 1 merge). Report final count on completion.

## Open Design Questions (resolved)

| Question | Resolution |
|----------|------------|
| How does `enqueue(JobName, args)` resolve `JobName` at runtime? | **Resolved (PR 2a):** String literal — `enqueue("SendEmail", args)`. |
| Should `Job` declarations be importable across files? | **Resolved (PR 2a):** Yes — jobs register globally via `JOB_RUNTIME`. Import the file, `job` auto-registers. |
| Worker poll interval | **Resolved (PR 2b):** 1s default, configurable via `work_async(map { "poll_interval": 500 })`. |
| Job args: Map only or any serializable? | **Resolved (PR 2a):** Map only — matches KV storage. |
| Queue auto-creation | **Resolved (PR 2a):** Auto-create. `configure_queue()` is for settings, not queue creation. |
| `job` as keyword or contextual? | **Resolved (PR 2a review):** Contextual identifier — avoids breaking existing code using `job` as a variable name. |
| `JobDefinition` stores perform body? | **Resolved (PR 2a review):** Yes — `perform_params`, `perform_body`, `on_failure` stored for worker execution. |
| Atomic claiming mechanism | **Resolved (PR 2b):** `kv_claim()` — SQLite `BEGIN IMMEDIATE` transaction, Redis SCAN+GET+DEL. |
