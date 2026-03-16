# DD-037 Phase 2: Job DSL + KV Backend — Implementation Plan

**Status:** Planning
**Parent:** [DD-037](dd-037-concurrency-and-jobs.md)
**Created:** 2026-03-16
**Target branch:** `feat/job-dsl-v2`
**Prior work:** `feat/job-dsl` branch (806-line jobs.rs, parser, AST, tests — reference only, needs rewrite for KV backend + std/concurrent integration)

---

## Guiding Principles

1. **Small PRs.** PR #31 was 25 commits and 7 review rounds. Never again. 3 focused PRs.
2. **Persistent from day one.** No memory backend. SQLite KV default, Redis/Valkey for prod.
3. **Build on std/concurrent.** Workers use `spawn()`. No reinventing thread management.
4. **Build on std/kv.** No raw Redis/SQLite calls. If std/kv is missing an operation, add it to std/kv first.
5. **Free functions everywhere.** `enqueue(JobName, args)`, not `JobName.enqueue(args)`.
6. **Each sub-phase is independently shippable.** PR 2a works without 2b. 2b works without 2c.

---

## Architecture Notes (Read Before Implementing)

### Job Registry Lives in RUNTIME, Not Per-Interpreter

`job_registry: HashMap<String, JobDefinition>` belongs in `RUNTIME` (global state), **not** in `Interpreter`. Workers use `run_in_fresh_interpreter()` — a fresh interpreter has no job definitions. The same pattern applies here as channels: `RUNTIME.job_registry` is populated when `Statement::Job` is evaluated, and looked up by workers via `RUNTIME`.

Implementation: add `job_registry: Arc<RwLock<HashMap<String, JobDefinition>>>` to `Runtime` struct (alongside `channels`, `tasks`). `eval_statement` for `Statement::Job` writes to `RUNTIME.job_registry`. Workers read from `RUNTIME.job_registry` when dispatching claimed jobs.

### Worker Startup Sequence

The full lifecycle from CLI to job execution:

1. `ntnt worker server.tnt` → interpreter loads and evaluates `server.tnt` top-level
2. Every `Statement::Job` encountered → `JobDefinition` registered in `RUNTIME.job_registry`
3. If jobs are defined in a module (`lib/jobs.tnt`), that module **must be imported** for jobs to register
4. `work_jobs()` called → reads `RUNTIME.job_registry` for dispatch
5. Worker loop: claims job by name from KV, looks up `JobDefinition` in `RUNTIME.job_registry`, runs `perform_body` in fresh interpreter with job args injected as local scope

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
PR 2a  📋  Parser + Registry + Enqueue MVP       parse Job syntax, store in KV, basic enqueue
PR 2b  📋  Workers + Lifecycle + Retry            claim jobs, run via spawn(), retry on failure
PR 2c  📋  DX: Testing Mode + Logs + CLI + Docs   assert_enqueued, streaming logs, ntnt worker
```

### Estimated total: 6-8 days across 3 PRs

---

## PR 2a: Parser + Registry + Enqueue MVP

**Goal:** `Job` syntax parses, jobs register, `enqueue()` writes to KV. No workers yet — jobs sit in the queue waiting to be claimed. This validates the entire front half of the pipeline.

**Estimated effort:** 2-3 days

### Lexer (`src/lexer.rs`)
- [ ] Add `Job` keyword token (the `feat/job-dsl` branch already has this — cherry-pick)
- [ ] `on` is already a keyword (used by `on_shutdown`, `on_error`) — no change needed
- [ ] `perform` and `on_failure` are parsed contextually inside `Job` blocks, not keywords

### AST (`src/ast.rs`)
- [ ] Add `Statement::Job` variant:
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
- [ ] The `feat/job-dsl` branch has this exact node — cherry-pick and verify

### Parser (`src/parser.rs`)
- [ ] Parse `Job Name on queue { perform(args) { body } }` syntax
- [ ] Parse optional inline options: `Job Name on queue (retry: 5, timeout: 120) { ... }`
- [ ] Parse optional `on_failure(error, attempt) { ... }` block
- [ ] `Job` declarations are top-level statements (like `fn`, `struct`, `enum`)
- [ ] Reference: `feat/job-dsl` parser has ~98 lines for this — adapt, don't copy blindly

### Interpreter — Job Registry (`src/interpreter.rs`)
- [ ] Add `JobDefinition` struct to interpreter state:
  ```rust
  struct JobDefinition {
      name: String,
      queue: String,
      options: HashMap<String, Value>,  // evaluated options
      perform_params: Vec<Parameter>,
      perform_body: Block,
      on_failure: Option<(Vec<Parameter>, Block)>,
  }
  ```
- [ ] `eval_statement` for `Statement::Job`: evaluate options, register in `job_registry: HashMap<String, JobDefinition>`
- [ ] Job names must be unique — error on duplicate registration
- [ ] Execution mode guards: skip job registration in HotReload worker mode (same pattern as `spawn`)

### KV Integration (`src/stdlib/jobs.rs` — new file)
- [ ] `configure_queue(opts)` — takes a map with `"store"` key
  - Default: `"sqlite:./jobs.db"` if no store specified
  - Opens a KV connection via `std/kv::open()`, stores handle in module-level state
  - Validates store URL format
- [ ] `enqueue(job_name, args)` — the core enqueue function:
  - Look up job name in registry (error if not registered)
  - Generate job ID (UUID via `std/crypto::uuid()`)
  - Serialize job data: `{ type, queue, payload, status: "pending", attempts: 0, created_at, ... }`
  - Write to KV: `set(kv, "jobs:data:<id>", job_data)` + add to queue sorted set
  - Return job ID as String
- [ ] `job_status(job_id)` — read job data from KV, return status map
- [ ] `cancel_job(job_id)` — set status to "cancelled", remove from queue

### std/kv gaps to resolve before PR 2a

Audit `std/kv` against these specific needs and add missing operations to std/kv **as part of PR 2a** (not as a separate PR):

**Sorted sets for queue ordering:**
- SQLite backend: likely needs a `score` column approach — a dedicated `kv_sorted` table with `(namespace, key, score REAL, value TEXT)`. Add `zadd(kv, key, score, value)`, `zrangebyscore(kv, key, min, max, limit?)`, `zrem(kv, key, member)` to std/kv.
- Redis backend: native `ZADD`/`ZRANGEBYSCORE` — map directly.
- If adding sorted set ops to std/kv is too heavy for PR 2a scope, use a simpler approach: store `scheduled_at` as a zero-padded ISO timestamp prefix in the KV key (`jobs:pending:<timestamp>:<id>`) — natural lexicographic ordering, scan with prefix range. This avoids sorted sets entirely.

**Atomic job claiming:**
- SQLite: wrap claim in `BEGIN IMMEDIATE` transaction — `SELECT ... WHERE status='pending' AND (scheduled_at IS NULL OR scheduled_at <= now) ORDER BY score LIMIT 1`, then `UPDATE ... SET status='active'`. Add an `atomic_claim(kv, queue_name)` operation to std/kv or implement directly in jobs.rs using the KV handle's underlying connection.
- Redis: `LMOVE` (move from pending list to active list atomically) or `ZPOPMIN` on the sorted set.
- Single-process workers on SQLite: SQLite's serialized writes mean a `get + set` without explicit transaction is *practically* safe, but don't rely on this — use a transaction.

**Decision:** Before writing any jobs.rs code, open std/kv, check what exists, and either add the missing operations or choose the key-prefix approach. Document the decision in a comment in jobs.rs.

### Typechecker (`src/typechecker.rs`)
- [ ] Add `Job` to statement checking (skip body for now, or treat like function body)
- [ ] Add signatures: `configure_queue`, `enqueue`, `job_status`, `cancel_job`

### Build system (`build.rs`)
- [ ] `src/stdlib/jobs.rs` will be auto-discovered by build.rs glob — no config needed
- [ ] Add `// @ntnt` doc blocks to all functions (build enforces this)

### Tests
- [ ] Parser test: `Job SendEmail on emails { perform(to) { print(to) } }` parses correctly
- [ ] Parser test: options, on_failure handler
- [ ] Integration test: register job → enqueue → verify in KV → check status
- [ ] Integration test: cancel job
- [ ] Integration test: enqueue unregistered job → error
- [ ] Integration test: duplicate job name → error

### PR 2a Completion Criteria
- [ ] All tests pass (`cargo test`)
- [ ] `cargo build --release --locked && ntnt docs --generate` — CI will fail on docs drift
- [ ] `// @ntnt` doc blocks on all new public functions
- [ ] Typechecker signatures complete (not partial — Copilot will flag this)
- [ ] No `eprintln!` for error handling — return `Err(...)` or `Value::Error`

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
- [ ] `work_async()` — starts a worker loop in-process via `spawn()`:
  - Poll KV queue for pending jobs (configurable poll interval, default 1s)
  - Claim job: atomically move from pending → active (KV get + set with TTL)
  - Deserialize job data, look up `JobDefinition` in registry
  - Run `perform` body in a fresh interpreter via `run_in_fresh_interpreter()` (same pattern as `spawn()` in std/concurrent)
  - On success: status → "completed", store result, set completion TTL
  - On failure: increment attempts, check retry policy, either re-queue or mark dead
  - Cancellation-aware: check `RUNTIME` shutdown flag between jobs
- [ ] `work_jobs(opts)` — blocking worker mode for `ntnt worker` CLI:
  - Same logic as `work_async()` but blocks the main thread
  - `opts.concurrency`: number of concurrent `spawn()` workers (default: 1)
  - `opts.queues`: which queues to process (default: all registered)
- [ ] Worker heartbeat: periodic KV write with TTL (e.g., every 10s, TTL 30s)

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
- [ ] All state transitions are KV writes (atomic where possible)
- [ ] `completed_at`, `failed_at`, `dead_at` timestamps recorded
- [ ] `error` field stores last failure message

### Retry Logic
- [ ] Retry count from job options: `retry: 5` (default: 3)
- [ ] Backoff strategies:
  - `exponential` (default): delay = base * 2^attempt (base = 5s)
  - `linear`: delay = base * attempt
  - `constant`: delay = base
- [ ] Syntax: `Job X on q (retry: 5, backoff: "exponential") { ... }`
- [ ] Retry delay → re-enqueue with `scheduled_at` in the future
- [ ] `on_failure(error, attempt)` hook called on each failure (before retry decision)

### Scheduled Jobs
- [ ] `enqueue_at(JobName, timestamp, args)` — enqueue with future `scheduled_at`
- [ ] `enqueue_in(JobName, delay_seconds, args)` — convenience wrapper
- [ ] Worker skips jobs where `scheduled_at > now()` during polling
- [ ] KV sorted set ordering: `scheduled_at` as score (pending jobs sorted by when they should run)

### Job Timeout
- [ ] `timeout` option: `Job X on q (timeout: 30) { ... }` (seconds)
- [ ] Worker wraps execution in a deadline check
- [ ] Timeout → failure (same as error, triggers retry)

### Visibility Timeout
- [ ] Active jobs get a KV TTL key: `jobs:active:<id>` with configurable timeout (default 5 min)
- [ ] If worker dies mid-job, TTL expires, and job becomes re-claimable
- [ ] Worker refreshes TTL periodically during long jobs (heartbeat pattern)

### Graceful Shutdown
- [ ] `RUNTIME.shutdown()` integration: stop claiming new jobs, wait for in-flight to complete
- [ ] Configurable drain timeout (default 30s) — after which in-flight jobs are abandoned and re-claimable via visibility timeout

### Tests
- [ ] Integration: enqueue → work_async → job runs → status is completed
- [ ] Integration: job fails → retries N times → eventually dead
- [ ] Integration: job fails → on_failure handler called with error and attempt count
- [ ] Integration: enqueue_in → job doesn't run until delay passes
- [ ] Integration: cancel pending job → worker skips it
- [ ] Integration: job timeout → treated as failure
- [ ] Integration: graceful shutdown → in-flight jobs complete, no new claims
- [ ] Unit: backoff calculation (exponential, linear, constant)
- [ ] Unit: state machine transitions

### PR 2b Completion Criteria
- [ ] All tests pass (`cargo test`)
- [ ] `cargo build --release --locked && ntnt docs --generate`
- [ ] `// @ntnt` doc blocks on all new public functions in jobs.rs
- [ ] Typechecker signatures complete for `work_async`, `work_jobs`, `enqueue_at`, `enqueue_in`
- [ ] No `eprintln!` — use structured logging (stderr JSON, same format as PR 2c streaming logs — define the format early even if the full hook isn't wired yet)

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

### Testing Mode
- [ ] `configure_queue(map { "mode": "testing" })` — jobs collected in memory, nothing runs
- [ ] `assert_enqueued(JobName, args)` — verify a job was enqueued
  - Partial match: `assert_enqueued(SendEmail, map { "email": "alice@example.com" })` matches if those keys exist in the job's args (extra keys OK)
- [ ] `assert_not_enqueued(JobName)` — verify no jobs of this type were enqueued
- [ ] `drain_jobs()` — run all collected jobs synchronously in current thread (for integration tests)
- [ ] `clear_jobs()` — reset test queue between tests
- [ ] Testing mode intercepts `enqueue()` — no KV writes happen

### Streaming Logs
- [ ] Every job event emits a structured JSON log line to stderr:
  ```json
  {"event": "job.started", "job_id": "abc123", "type": "SendEmail", "queue": "emails", "attempt": 1, "timestamp": "..."}
  {"event": "job.completed", "job_id": "abc123", "type": "SendEmail", "duration_ms": 142, "timestamp": "..."}
  {"event": "job.failed", "job_id": "abc123", "type": "SendEmail", "error": "...", "attempt": 1, "will_retry": true, "timestamp": "..."}
  ```
- [ ] `on_job_event(fn(event))` — user hook for custom handling
- [ ] Events: `job.enqueued`, `job.started`, `job.completed`, `job.failed`, `job.retried`, `job.dead`, `job.cancelled`

### Deduplication & Expiration
- [ ] `unique: 3600` option — skip enqueue if identical job (same type + SHA256 of args) was enqueued within N seconds
- [ ] Dedup key in KV: `jobs:unique:<sha256>` with TTL = unique duration
- [ ] `expires: 300` option — discard jobs that have been pending for longer than N seconds

### Batch Enqueue
- [ ] `enqueue_batch(JobName, array_of_args)` — enqueue N jobs in one call
- [ ] Single KV round-trip where possible

### Priority Queues
- [ ] Job-level priority: `Job X on q (priority: 1) { ... }` (lower = higher priority)
- [ ] Priority encoded in sorted set score (priority * 1e12 + scheduled_at)

### CLI: `ntnt worker`
- [ ] `ntnt worker server.tnt` — start workers for jobs defined in server.tnt
- [ ] `--concurrency N` — number of concurrent workers (default: 1)
- [ ] `--queues emails,payments` — which queues to process
- [ ] Loads the .tnt file (registers jobs), then calls `work_jobs()` in blocking mode
- [ ] Ctrl+C → graceful shutdown

### CLI: `ntnt jobs` (basic — full observability is Phase 6)
- [ ] `ntnt jobs status server.tnt` — summary of all queues (pending/active/completed/failed/dead counts)
- [ ] `ntnt jobs list server.tnt --queue=emails --status=failed` — list jobs with filters
- [ ] Loads the .tnt file to get KV config, then queries KV directly

### Documentation
- [ ] `// @ntnt` doc blocks on all new functions in `src/stdlib/jobs.rs`
- [ ] Typechecker signatures for all functions
- [ ] `cargo build --release` + `ntnt docs --generate`
- [ ] `docs/AI_AGENT_GUIDE.md` — Job DSL section with examples
- [ ] CLAUDE.md + copilot-instructions.md auto-synced
- [ ] Example: `examples/job_demo.tnt`
- [ ] Update DD-037 Phase 2 checkboxes

### Tests
- [ ] Testing mode: assert_enqueued, assert_not_enqueued, drain_jobs, clear_jobs
- [ ] Streaming logs: verify event format
- [ ] Dedup: enqueue same job twice within unique window → second is skipped
- [ ] Expiration: enqueue with expires → job discarded after timeout
- [ ] Batch: enqueue_batch creates N jobs
- [ ] Priority: higher-priority jobs claimed first
- [ ] CLI: `ntnt worker` starts and processes jobs (integration test)
- [ ] CLI: `ntnt jobs status` shows correct counts

---

## Pre-Implementation Checklist

Before writing any code, verify these:

- [ ] **std/kv audit**: Run the audit described in "std/kv gaps to resolve" above. Choose sorted set approach (native ops vs key-prefix). Document decision in jobs.rs before anything else.
- [ ] **Cherry-pick assessment**: Review `feat/job-dsl` branch specifically for: `Job` keyword token (lexer.rs), `Statement::Job` AST node (ast.rs), parser block (~98 lines). These are safe cherry-picks. `jobs.rs` itself is **reference only** — do not cherry-pick; the registry architecture has changed (RUNTIME not per-interpreter).
- [ ] **Base branch**: `feat/job-dsl-v2` branches from `feat/concurrency-v2` if that PR is not yet merged to main, or from `main` if it is. Check `git log main --oneline | head` — if you see the Phase 1 hardening commits (`e0c6e17` etc.), branch from main. If not, branch from `feat/concurrency-v2`.
- [ ] **KV key layout**: Validate the key layout from DD-037 against actual std/kv capabilities — especially the sorted set strategy chosen above.
- [ ] **enqueue() examples**: Update all examples in this doc and in code comments to use string form `enqueue("SendEmail", args)` before writing any implementation code.

---

## Sub-Agent Notes

If this work is delegated to a sub-agent (Claude Code, Codex, etc.), include the following in the task prompt:

**Must include:**
- Full content of `~/.openclaw/skills/ntnt/SKILL.md` (Core Development Workflow section at minimum)
- The Architecture Notes section from this doc verbatim
- The std/kv gaps section with chosen approach pre-filled

**Hard rules for the sub-agent:**
1. Read the ntnt skill before touching any file
2. No `eprintln!` — all errors returned via `Err(...)` or `Value::Error`
3. Complete `// @ntnt` doc blocks on every new public function — CI enforces this
4. Complete typechecker signatures — partial sigs will get flagged in review
5. Run `cargo build --release --locked && ntnt docs --generate` before every push
6. Report test count delta: baseline is ~1,076 passing. State final count in completion message.
7. Post plan before executing, confirm before merging or pushing to main

## Open Design Questions (resolve during implementation)

| Question | Context | Leaning |
|----------|---------|---------|
| How does `enqueue(JobName, args)` resolve `JobName` at runtime? | Is it a string lookup in the registry? A Value? | **Resolved: String literal** — `enqueue("SendEmail", args)`. Update all examples to string form. Identifier form is ambiguous and harder to implement. |
| Should `Job` declarations be importable across files? | A job defined in `lib/jobs.tnt` used in `routes/api.tnt` | Yes — jobs register globally like routes. Import the file, Job auto-registers. |
| Worker poll interval | How often to check KV for new jobs | 1s default, configurable via `work_async(map { "poll_interval": 500 })` |
| Job args: Map only or any serializable? | Oban/Sidekiq use maps/hashes | Map only — matches KV storage and keeps it simple |
| Queue auto-creation | Does enqueue auto-create the queue or require explicit config? | Auto-create. configure_queue() is for settings, not queue creation. |
