# DD-057: Interpreter Suspension for Async I/O

**Status:** Phase 1 verified shipped (2026-07-08 spike); Phase 2 deferred pending production-mode re-benchmark
**Author:** Larri + Josh
**Created:** 2026-03-31
**App:** ntnt
**Origin:** DD-038 benchmarks (2026-03-12) — ntnt matches Hono/Bun on HTTP-only but is 4-20× slower on DB workloads. Phase 1 (connection pooling) shipped in v0.4.2 with zero throughput gain, confirming the bottleneck is the interpreter, not the pool.

---

## Verified State (2026-07-08 spike)

Code inspection and an empirical test against current main show **the Phase 1
worker pool below is already shipped** — it landed in v0.4.2 alongside the
connection pool (DD-039, PR #23), before this DD was written:

- `run_worker` (interpreter.rs) gives each worker its own interpreter in
  Worker mode, re-evaluating the program for its own route table; workers
  1..N never hot-reload.
- Worker count resolution (interpreter.rs, async server startup):
  production defaults to `min(num_cpus, 8)`, dev to 1, `NTNT_WORKERS`
  overrides both; `ntnt run --workers N` sets it from the CLI.
- The flume channel is MPMC and all cross-worker state
  (`SHARED_POOL_REGISTRY`, `DB_RUNTIME`, KV registries, sqlite
  `CONNECTION_REGISTRY`, auth session stores, `EMAIL_STATE`) is
  static + `Mutex`/`Arc` — audited 2026-07-08, including post-March
  additions.

**Empirical proof that workers parallelize blocking I/O** (no local
Postgres; `sleep_ms(200)` in a handler blocks the worker thread exactly
like `DB_RUNTIME.block_on`):

| 8 concurrent 200ms-blocking requests | wall time |
|--------------------------------------|----------:|
| `NTNT_WORKERS=1` | 1.61s (fully serialized, 8 × 200ms) |
| `NTNT_WORKERS=4` | 0.41s (theoretical 4× exactly) |

**Implication for the DD-038 numbers above:** the "zero throughput gain"
v0.4.2 measurement is consistent with running in dev mode, where the
default is one worker — the pool cannot help when a single thread
serializes every query. The 4-20× DB gap must be re-measured with
`NTNT_ENV=production` (or `NTNT_WORKERS=N`) before any Phase 2
investment; Phase 1's expected gains appear to already exist behind the
right configuration.

Phase 1 gaps found and closed by the spike PR: panicked workers were
never respawned (silent capacity loss — now supervised and respawned),
and the `--workers` CLI flag was missing (env var only).

---

## Problem

The ntnt interpreter blocks the entire worker thread during every database call. Even with a deadpool connection pool (5 connections available), only one query executes at a time because the interpreter calls `DB_RUNTIME.block_on()` synchronously for every operation.

### Current Architecture

```
Axum (async, multi-threaded)
  │
  ▼
flume MPMC channel
  │
  ▼
Single interpreter worker thread (Rc<RefCell<>>, not Send)
  │
  ├─ Evaluate request handler (fast — microseconds)
  ├─ Hit pg_query() → DB_RUNTIME.block_on(pool.get() + client.query()) ← BLOCKS HERE
  ├─ ... entire worker frozen for 1-10ms ...
  ├─ Result comes back, continue evaluation
  ├─ Maybe hit another pg_query() → BLOCKS AGAIN
  └─ Return response
```

While the interpreter is blocked on a DB call, every other request in the channel queue waits. Under load, this serializes all DB access regardless of how many pool connections exist.

### Benchmark Evidence (DD-038, 2026-03-12)

| Benchmark | ntnt v0.4.1 | ntnt v0.4.2 (pool) | FastAPI | Gin | Hono/Bun |
|-----------|----------:|--------:|--------:|----:|---------:|
| plaintext | 118K | 119K | 174K | 406K | 118K |
| db (1 query) | 8.4K | 8.3K | 37K | 130K | 32K |
| 20 queries | 457 | 418 | 5.8K | 9.3K | 2.8K |

Pool alone: **no improvement** (8.4K → 8.3K). The interpreter thread is the bottleneck, not the connection count. HTTP-only throughput (118K) proves the Axum/Tokio layer is fine — the interpreter can evaluate handlers at >100K/s when there's no I/O blocking.

### The 11 `block_on()` Calls

`src/stdlib/postgres.rs` has 11 `DB_RUNTIME.block_on()` calls:
- `connect()` — pool creation + verify
- `pg_query()` — query execution
- `pg_query_one()` — single-row query
- `pg_execute()` — execute (no results)
- `pg_transaction_begin/commit/rollback()` — transaction management
- `pg_close()` — pool shutdown

Each one freezes the interpreter thread for the duration of the network round-trip.

---

## Solution: Interpreter Suspension

When the interpreter hits an I/O operation (DB query, HTTP fetch, file read), instead of blocking, it **suspends** the current evaluation, returns control to the worker loop, and picks up another request from the channel. When the I/O completes, the suspended evaluation resumes where it left off.

### Target Architecture

```
Axum (async, multi-threaded)
  │
  ▼
flume MPMC channel
  │
  ▼
Interpreter worker thread
  │
  ├─ Pick up Request A from channel
  ├─ Evaluate A's handler → hit pg_query()
  ├─ Suspend A, save continuation
  ├─ Pick up Request B from channel       ← no longer blocked!
  ├─ Evaluate B's handler → hit pg_query()
  ├─ Suspend B, save continuation
  ├─ A's query result arrives → resume A
  ├─ A finishes → send response
  ├─ B's query result arrives → resume B
  ├─ B finishes → send response
  └─ ...
```

The interpreter thread is never idle while waiting for I/O. It interleaves evaluations like an async runtime, but at the language level — ntnt code stays synchronous.

---

## Design Options

### Option A: Continuation-Based Suspension (Recommended)

Save the interpreter's evaluation state as a continuation when hitting I/O, restore it when the I/O completes.

**What gets saved (the continuation):**
- Environment chain (`Rc<RefCell<Environment>>` — the scope stack)
- Current statement index in the block being evaluated
- Call stack (which function called which, with locals)
- The pending I/O operation + callback channel

**How it works:**

```rust
enum EvalResult {
    Complete(Value),                          // Normal return
    Suspended(Continuation, IoOperation),     // Yielded on I/O
}

struct Continuation {
    env: Rc<RefCell<Environment>>,
    call_stack: Vec<StackFrame>,
    resume_point: ResumePoint,                // Where to continue after I/O
    request_context: BridgeCallContext,        // Axum reply channel, etc.
}

enum IoOperation {
    DbQuery { pool_id: u64, sql: String, params: Vec<Value>, result_tx: oneshot::Sender<Value> },
    HttpFetch { url: String, opts: Value, result_tx: oneshot::Sender<Value> },
    FileRead { path: String, result_tx: oneshot::Sender<Value> },
}
```

**Worker loop becomes:**

```rust
loop {
    // 1. Check for completed I/O results (non-blocking)
    while let Ok(completed) = io_completion_rx.try_recv() {
        let continuation = suspended.remove(&completed.id);
        let result = resume_evaluation(continuation, completed.value);
        match result {
            EvalResult::Complete(response) => send_response(response),
            EvalResult::Suspended(cont, io) => {
                dispatch_io(io);
                suspended.insert(cont);
            }
        }
    }

    // 2. Pick up new requests (non-blocking if we have suspended work)
    match request_rx.try_recv() {
        Ok(request) => {
            let result = evaluate_handler(request);
            match result {
                EvalResult::Complete(response) => send_response(response),
                EvalResult::Suspended(cont, io) => {
                    dispatch_io(io);
                    suspended.insert(cont);
                }
            }
        }
        Err(TryRecvError::Empty) if suspended.is_empty() => {
            // Nothing to do — block on the channel
            let request = request_rx.recv().unwrap();
            // ... evaluate ...
        }
        _ => {}  // Channel empty but we have suspended work — keep polling
    }
}
```

**Pros:**
- Single interpreter thread (no thread-safety changes to `Rc<RefCell<>>`)
- Cooperative scheduling — no preemption, no race conditions
- ntnt user code stays 100% synchronous
- Compatible with existing environment/scope model

**Cons:**
- Requires refactoring the interpreter's recursive `evaluate()` to be resumable
- Call stack must be capturable and restorable — significant refactor
- Every I/O call site needs a suspension point

### Option B: Worker Pool (Multiple Interpreter Instances)

Spawn N interpreter threads, each with its own `Rc<RefCell<>>` environment. The flume channel already supports MPMC — just add more consumers.

**What changes:**
- `num_workers` default goes from 1 → `num_cpus` (or configurable)
- Each worker gets a fresh interpreter clone with the same loaded code
- Workers are fully independent — no shared mutable state between interpreters

```rust
for _ in 0..config.num_workers {
    let rx = request_rx.clone();  // flume supports MPMC
    let program = program.clone();
    std::thread::spawn(move || {
        let mut interpreter = Interpreter::new();
        interpreter.load(program);
        loop {
            let call = rx.recv().unwrap();
            let response = interpreter.handle_request(call.request);
            call.reply_tx.send(response).ok();
        }
    });
}
```

**Pros:**
- Simple — no interpreter changes at all
- Each worker blocks independently (while worker 1 is blocked on DB, workers 2-N handle requests)
- `Rc<RefCell<>>` stays as-is (each worker has its own)
- The bridge architecture already supports this (flume MPMC + the ASCII art in http_bridge.rs shows it)
- Proven model (uvicorn/gunicorn workers, PHP-FPM, etc.)

**Cons:**
- N× memory usage (each interpreter is a full clone of the program + stdlib)
- Shared state (DB pools, KV stores) needs to be `Arc` — already is (`POOL_REGISTRY` is static)
- Hot-reload must propagate to all workers
- N workers with M pool connections = N×M potential concurrent queries (may overwhelm DB)
- Doesn't solve the fundamental per-worker blocking — just parallelizes it

### Option C: Hybrid (Workers Now, Suspension Later)

Ship Option B first (it's a small change — `num_workers` already exists, flume already supports MPMC). Then pursue Option A for the long term.

**Rationale:** Option B is a config change + some worker initialization code. It gets immediate throughput gains on DB workloads by paralleling the blocking. Option A is the architecturally correct solution but requires a major interpreter refactor.

---

## Recommendation: Option C (Hybrid)

### Phase 1: Worker Pool (Quick Win)

The infrastructure is already there — `num_workers` config field exists, flume is MPMC, the bridge diagram shows multi-worker. Just wire it up.

**Expected impact:** With 4 workers and a 5-connection pool, DB throughput should approach 4× current (limited by the slowest of workers or connections). 8.3K → ~25-30K on single-query benchmark, closing most of the gap with FastAPI (37K).

**Checklist (statuses verified 2026-07-08):**
- [x] Worker pool initialization: spawn N interpreter threads from the loaded program — shipped v0.4.2 (DD-039)
- [x] Each worker gets its own `Interpreter` instance (re-reads and re-evaluates the program) — `run_worker`
- [x] `num_workers` config: defaults to `min(num_cpus, 8)` in production, 1 in development; `NTNT_WORKERS` overrides
- [x] Hot-reload propagation — resolved by design instead: dev defaults to 1 worker (which hot-reloads); workers 1..N run with hot-reload off. Multi-worker dev (`NTNT_WORKERS>1` without production) leaves workers 1..N stale after edits — documented tradeoff, restart to pick up changes
- [x] Worker health monitoring: panicked workers are logged and respawned (spike PR; previously a panic silently lost the worker)
- [x] `ntnt run server.tnt --workers 8` CLI flag (spike PR; env var existed)
- [ ] Benchmark: re-run DD-038 suite **with `NTNT_ENV=production` / `NTNT_WORKERS≥4`**, compare against the v0.4.2 numbers — the original measurement almost certainly ran with the dev default of 1 worker (see Verified State)
- [x] Verify shared state is safe: `SHARED_POOL_REGISTRY`, `DB_RUNTIME`, KV/sqlite/auth/email statics all `Mutex`/`Arc` — re-audited 2026-07-08 including post-March additions

**Risks:**
- Memory: each worker holds a full interpreter clone. Measure RSS with 4 workers on a real app.
- DB overload: 4 workers × 5 pool connections = 20 concurrent queries possible. PG default `max_connections` is 100, so fine. Add `NTNT_DB_POOL_SIZE` awareness to the docs.
- Hot-reload race: if file watcher fires while a worker is mid-request, need to queue the reload for the next request boundary.

### Phase 2: Interpreter Suspension (Long Term)

The architecturally clean solution. The interpreter yields on I/O and interleaves multiple evaluations on a single thread — true cooperative multitasking.

This is a significant refactor of the interpreter's evaluation model. The current recursive `evaluate()` function would need to become resumable — either via:

1. **Explicit continuation passing** — refactor `evaluate()` to return `EvalResult::Suspended` at I/O points
2. **Stackful coroutines** — use a crate like `corosensei` or `genawaiter` to save/restore the call stack
3. **CPS transform** — transform the interpreter to continuation-passing style internally

Each has tradeoffs in complexity, performance, and debuggability. This needs its own design spike before committing to an approach.

**Checklist:**
- [ ] Design spike: prototype each suspension approach, measure overhead
- [ ] Pick approach based on spike results
- [ ] Refactor interpreter evaluation to be suspendable
- [ ] Add suspension points at all I/O operations: `pg_query*`, `pg_execute`, `fetch()`, `read_file()`, etc.
- [ ] Worker loop: poll for I/O completions + new requests
- [ ] Benchmark: single-worker suspended vs multi-worker blocking
- [ ] Consider: does suspension replace workers, or complement them? (suspended workers = best of both)

**Deferred until:** Phase 1 results are measured **under production configuration** — the 2026-07-08 spike's sleep-proxy result (near-perfect 4× parallelization with 4 workers) predicts the DB benchmark re-run will close most of the gap, which would make Phase 2 an optimization for memory footprint and beyond-N-workers concurrency rather than a necessity. Do not start the suspension refactor before that re-benchmark exists.

---

## I/O Operations Requiring Suspension Points

| Module | Functions | Current Behavior |
|--------|-----------|-----------------|
| `std/db/postgres` | `pg_query`, `pg_query_one`, `pg_execute`, `connect`, transaction ops | `DB_RUNTIME.block_on()` — 11 call sites |
| `std/db/sqlite` | `query`, `execute`, `connect` | Synchronous (file I/O, usually fast) |
| `std/http` | `fetch`, `download` | `DB_RUNTIME.block_on()` via reqwest |
| `std/fs` | `read_file`, `write_file` | Synchronous `std::fs` |
| `std/kv` | `get`, `set` (when backed by Redis) | Synchronous |

Priority order: postgres (biggest impact), HTTP fetch (common in APIs), everything else (diminishing returns).

---

## Success Criteria

### Phase 1 (Worker Pool)

| Metric | Current (1 worker) | Target (4 workers) |
|--------|-------------------:|-------------------:|
| db single query | 8.3K req/s | 25-30K req/s |
| db 20 queries | 418 req/s | 1.5-2K req/s |
| plaintext (baseline) | 119K req/s | ~119K req/s (no change expected) |
| Memory (real app) | ~50MB | <200MB |

### Phase 2 (Suspension)

| Metric | Target |
|--------|--------|
| db single query (1 worker, suspended) | 30-40K req/s |
| db 20 queries (1 worker, suspended) | 3-5K req/s |
| Memory overhead per suspended request | <10KB |
| Max concurrent suspended evaluations | 1000+ |

---

## Open Questions

| Question | Options | Recommendation |
|----------|---------|----------------|
| Default worker count? | 1 (safe) vs `num_cpus` (fast) | `min(num_cpus, 4)` in production, 1 in development |
| Worker memory sharing? | Clone AST per worker vs Arc-shared AST | Clone for simplicity — AST is small (~1-5MB for real apps) |
| Should `num_workers` affect dev mode? | Same as prod vs always 1 | Always 1 in dev (simpler debugging, hot-reload) |
| Suspension approach? | Continuations vs coroutines vs CPS | Defer to Phase 2 design spike |
| Phase 2 timeline? | After Phase 1 ships vs parallel | After Phase 1 benchmarks — measure first |

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-31 | Initial draft — consolidating DD-038 benchmark findings and Phase 1/2 plan from daily notes |
