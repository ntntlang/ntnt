# DD-057: Interpreter Suspension for Async I/O (Post-Worker-Pool Plan)

**Status:** Draft  
**Author:** Larri + Josh  
**Created:** 2026-03-31  
**Revised:** 2026-04-05  
**App:** ntnt  
**Origin:** DD-038 benchmarks (2026-03-12) showed ntnt matching Hono/Bun on HTTP-only workloads but lagging badly on DB-heavy workloads. Connection pooling shipped earlier and did not materially improve throughput, confirming the bottleneck is interpreter-side blocking during I/O.

---

## Executive Summary

This problem is **real**, but the old version of this DD was stale in one important way: the “quick win” worker-pool phase is no longer hypothetical — ntnt already has a production worker pool.

So the actual question now is **not**:
> should ntnt move from one worker to many workers?

It is:
> should ntnt add **per-worker interpreter suspension** on top of the existing worker pool so each worker can multiplex requests while waiting on DB/HTTP/file I/O?

My judgment:
- **Worker pool:** easy / already done
- **True interpreter suspension:** hard, but tractable
- **Best plan:** keep the worker pool, add suspension **inside worker mode first**, and scope the MVP tightly to the highest-value blocking operations (`std/db/postgres` query path)

This is a **high-complexity runtime refactor**, not a quick patch. It is probably one of the harder runtime changes on the current ntnt roadmap, but it is also one of the few with clear benchmark upside.

---

## Problem

The interpreter still blocks **each worker thread** during synchronous I/O operations such as PostgreSQL queries.

Even after connection pooling, a worker that hits `DB_RUNTIME.block_on(...)` is frozen until the operation completes. The worker pool improves throughput by parallelizing the blocking across multiple interpreters, but it does **not** remove the per-worker idle time.

### Current Real Baseline

The old version of this DD described a single-worker bridge as if that were the whole production architecture. That is no longer accurate.

Current server behavior:
- production mode already supports multiple interpreter workers (`NTNT_WORKERS`)
- worker 0 (main thread) handles hot-reload-aware behavior
- additional workers parse/eval the same source independently and process requests without hot-reload
- each worker still blocks on DB calls inside the interpreter

So the current architecture is already:

```
Axum (async, multi-threaded)
  │
  ▼
flume MPMC channel
  │
  ├─ Worker 0 interpreter (main thread, hot-reload aware)
  ├─ Worker 1 interpreter (blocking on I/O per request)
  ├─ Worker 2 interpreter (blocking on I/O per request)
  └─ Worker N interpreter (blocking on I/O per request)
```

This means DD-057 should be framed as a **Phase 2 / Phase 3 runtime improvement after worker-pool support**, not as the first step.

### Why this still matters

Worker pools help, but they have limits:
- each worker still goes idle while waiting on I/O
- more workers means more interpreter clones and more memory
- more workers also mean more potential DB fan-out (`workers × pool size`)
- hot-reload complexity remains concentrated in the main worker

Interpreter suspension attacks the real inefficiency directly:
- don’t waste a worker while it waits on network/file I/O
- let one worker advance other suspended requests instead

---

## Critical Review of the Previous DD

The previous version of DD-057 had the right instinct but several structural problems:

### 1. It treated the worker pool as future work when it already exists
That made the whole plan look more greenfield than it really is.

### 2. It mixed two different documents into one
The old draft tried to be both:
- a justification for the already-shipped worker-pool direction, and
- a design for true interpreter suspension

Those are different decisions with very different difficulty.

### 3. It made Phase 1 look more speculative than it should
The worker-pool path is no longer “what if we did this?” It is the live baseline we should benchmark against.

### 4. It made suspension sound slightly cleaner than it really is
The hard part is **not** dispatching async I/O. The hard part is making the recursive interpreter resumable without turning it into a bug farm.

### 5. It lacked a tight MVP boundary
“suspend all I/O” is too broad. The right first target is:
- production worker mode only
- PostgreSQL query path first
- keep dev/hot-reload behavior unchanged initially

### 6. Its success metrics were too speculative
The old draft gave optimistic numeric targets without benchmarking the current worker-pool baseline first. That’s not a great basis for implementation gating.

---

## Recommendation

### Recommended architecture: **hybrid, worker-mode-first suspension**

Keep the existing worker pool.

Then add **cooperative suspension inside each worker** so a worker can pause one request on I/O, continue another, and resume the first when the operation completes.

This gives us:
- current production parallelism from multiple workers
- future per-worker multiplexing within each worker
- much lower risk than trying to replace the whole server model in one shot

### Recommended initial scope

**Implement suspension only in production worker mode first.**

Do **not** try to solve all of these at once:
- hot-reload-aware main worker
- all stdlib I/O modules
- every blocking native function
- dev mode semantics

#### MVP scope
- production worker mode only
- `std/db/postgres` first
  - `pg_query`
  - `pg_query_one`
  - `pg_execute`
- optionally transaction begin/commit/rollback once the basic mechanism works

#### Explicitly out of MVP scope
- dev-mode hot reload
- file I/O suspension
- HTTP fetch suspension
- Redis / KV suspension
- replacing the worker pool entirely

---

## How hard is this?

### Short answer
**Hard.** Roughly **7.5/10** difficulty.

### Why
Because the problem is not “make DB async.” The DB is already async-capable behind the blocking boundary. The hard part is teaching the interpreter to **yield and resume safely**.

That means:
- preserving execution state at an I/O boundary
- restoring the right continuation with the right value/error
- keeping environment/call-frame semantics correct
- not breaking error propagation, transactions, or return behavior
- avoiding weird starvation/reentrancy bugs in the worker loop

### My practical estimate
- **Not** a one-evening patch
- **Not** something I’d want to squeeze into a noisy feature train casually
- **Yes** something we can do if scoped tightly and benchmarked carefully

If the benchmark delta over the current worker-pool baseline is small, this may not be worth the complexity right now.
If the delta is big, it becomes one of the highest-leverage runtime projects in ntnt.

---

## Design Direction

### Core principle
Do **not** try to capture the Rust call stack.

Instead, make the interpreter resumable via an **explicit evaluation stack / continuation model**.

That means moving toward:
- resumable `EvalOutcome`
- explicit `Continuation`
- explicit frame / program-counter state

### Recommended approach: explicit continuation / scheduler model

```rust
enum EvalOutcome {
    Complete(Value),
    Suspended(Continuation, PendingIo),
}

struct Continuation {
    env: Rc<RefCell<Environment>>,
    frames: Vec<EvalFrame>,
    request_context: RequestContext,
}

enum PendingIo {
    PgQuery { op_id: u64, ... },
    PgQueryOne { op_id: u64, ... },
    PgExecute { op_id: u64, ... },
}
```

Each worker then becomes a tiny scheduler:
- receive new requests
- evaluate until complete or suspended
- dispatch async I/O for suspended operations
- poll completion queue
- resume the saved continuation

### Why not stackful coroutines first?
Because they look elegant but usually make debugging, control-flow reasoning, and runtime portability worse. They may still be worth a spike, but they should not be the default assumption.

### Why not keep pure worker parallelism and stop?
That is the control/baseline. It may be enough. But we should decide that with fresh benchmarks against the already-shipped worker pool, not by assumption.

---

## Proposed Architecture

### Current target architecture (after suspension MVP)

```
Axum
  │
  ▼
flume MPMC channel
  │
  ├─ Worker 0 (hot-reload aware, still mostly current behavior)
  ├─ Worker 1 scheduler
  │    ├─ ready queue
  │    ├─ suspended map
  │    └─ I/O completion queue
  ├─ Worker 2 scheduler
  └─ Worker N scheduler
```

Each production worker can:
1. start evaluating a request
2. suspend on `pg_query` / `pg_query_one` / `pg_execute`
3. dispatch the query onto the async DB runtime
4. continue processing other ready requests
5. resume the suspended continuation when the result arrives

### Key design choice
**Suspension complements workers; it does not replace them initially.**

That means:
- we keep multi-core parallelism
- we reduce idle time inside each worker
- we avoid having one giant all-powerful scheduler refactor as the first move

---

## Implementation Plan

### Phase 0 — Re-baseline the current system
Before any runtime refactor:
- [ ] Benchmark the current worker-pool baseline at `NTNT_WORKERS=1,2,4,8`
- [ ] Measure memory/RSS for a representative real app under those worker counts
- [ ] Measure single-query and multi-query throughput against the current baseline, not the old DD-038 single-worker numbers
- [ ] Decide whether the current worker-pool baseline is already “good enough” for near-term needs

### Phase 1 — Design spike for suspension shape
- [ ] Identify the smallest interpreter slice that can suspend and resume safely
- [ ] Prototype an explicit `EvalOutcome::{Complete,Suspended}` path at one native-function boundary
- [ ] Prove that a saved continuation can resume with a returned `Value` cleanly
- [ ] Decide whether the interpreter needs a partial frame-stack refactor before real suspension work can proceed
- [ ] Explicitly reject or accept stackful-coroutine experiments based on debuggability, not aesthetics

### Phase 2 — Worker-mode scheduler core
- [ ] Implement a per-worker ready/suspended/completion scheduling loop
- [ ] Keep the implementation limited to production worker mode initially
- [ ] Leave hot-reload-aware main-worker behavior unchanged for the first suspension slice
- [ ] Add operation IDs and completion routing for suspended I/O
- [ ] Ensure errors resume into the interpreter with normal ntnt error semantics

### Phase 3 — PostgreSQL MVP
- [ ] Add suspension support for `pg_query`
- [ ] Add suspension support for `pg_query_one`
- [ ] Add suspension support for `pg_execute`
- [ ] Benchmark after the PostgreSQL MVP before widening scope
- [ ] Decide whether transaction begin/commit/rollback should be included immediately after the MVP or in Phase 4

### Phase 4 — Transaction and semantics hardening
- [ ] Add transaction-safe suspension handling
- [ ] Verify suspended operations do not violate transaction pinning semantics
- [ ] Verify request-scoped environment/state survives suspend/resume correctly
- [ ] Verify return/error propagation across suspend/resume boundaries
- [ ] Add stress tests for many concurrent suspended requests inside one worker

### Phase 5 — Re-evaluate scope expansion
- [ ] Decide whether to extend suspension to `std/http`
- [ ] Decide whether to extend suspension to file I/O
- [ ] Decide whether dev-mode/main-worker hot-reload path should ever become suspendable
- [ ] Revisit default worker counts once suspension exists (current worker defaults may be too high or too low after multiplexing)

---

## Success Criteria

### Benchmark gates
Use benchmark deltas relative to the **current worker-pool baseline**, not the old single-worker DD-038 numbers.

#### Required for Phase 3 to be considered a success
- [ ] 1-worker suspended PostgreSQL throughput beats 1-worker blocking baseline materially
- [ ] 4-worker suspended PostgreSQL throughput beats current 4-worker blocking baseline materially
- [ ] plaintext / non-I/O benchmarks do not regress in a meaningful way
- [ ] memory growth is measured and acceptable for the achieved throughput gain

### Correctness gates
- [ ] No request-context leakage across suspended continuations
- [ ] No broken transaction semantics
- [ ] No broken return / throw / contract behavior after resume
- [ ] No hot-reload regressions in dev mode (because dev mode is intentionally left mostly unchanged initially)

---

## Risks

| Risk | Why it matters | Mitigation |
|------|----------------|------------|
| Continuation bugs | Most likely source of subtle runtime breakage | Keep MVP tiny; add explicit scheduler tests before widening scope |
| Transaction breakage | Suspending in the middle of DB workflows can corrupt semantics | Treat transaction support as a separate hardening phase |
| Hot-reload interaction | Main worker has special behavior today | Keep suspension out of hot-reload path initially |
| Complexity > payoff | Worker pool may already close enough of the gap | Benchmark current baseline first |
| Scheduler starvation | A bad polling loop can starve ready or resumed work | Add fairness tests and explicit queue policy |

---

## Open Questions

| Question | Recommendation |
|----------|----------------|
| Should suspension replace workers eventually? | Not initially. Keep workers + add per-worker suspension first. |
| Should dev mode use suspension? | No, not in the MVP. Keep dev simpler. |
| Should HTTP/file I/O be in scope for the MVP? | No. PostgreSQL first. |
| Should transactions be in the MVP? | Probably not. Add them immediately after the query/execute MVP if the core scheduler works. |
| What if worker pool benchmarks are already good enough? | Then this becomes a deferred optimization, not an immediate roadmap item. |

---

## Recommendation Summary

If you’re asking **“how hard would this be?”**, my answer is:

- **worker-pool part:** already done
- **true suspension MVP:** hard but tractable
- **full generalized suspension runtime:** very hard / should be staged carefully

If you’re asking **“should we do it?”**, my answer is:

- **yes, maybe** — but only after re-benchmarking the already-shipped worker-pool baseline
- and only with a **very tight MVP**: production worker mode + PostgreSQL query path only

That is the version of this project that feels ambitious but sane, instead of clever but unbounded.

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-31 | Initial draft — consolidated DD-038 benchmark findings and the original worker-pool + suspension concept |
| 2026-04-05 | Major revision — updated for the reality that worker-pool support already exists, narrowed the suspension MVP, added critical review, staged implementation plan, and reframed success around the current multi-worker baseline |
