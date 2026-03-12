# DD-039: Async Database I/O

**Status:** draft  
**App:** ntnt  
**Author:** Larri  
**Date:** 2026-03-12  
**Triggered by:** DD-038 benchmark results (ntnt DB performance 4–20× slower than competitors)

## Problem

ntnt's database performance is dramatically slower than every competitor:

| Benchmark | ntnt | FastAPI | Express | Gin | Gap |
|-----------|-----:|--------:|--------:|----:|-----|
| 1 query | 8,371 | 36,859 | 11,817 | 130,190 | **4–15× slower** |
| 20 queries | 457 | 5,818 | 2,419 | 9,296 | **5–20× slower** |
| Template (10q) | 899 | 10,431 | 4,112 | 18,014 | **5–20× slower** |

Meanwhile, HTTP-only benchmarks show ntnt at 118K req/sec — matching Bun/Hono. The HTTP layer isn't the problem.

## Root Cause (3 bottlenecks stacked)

### 1. Single-threaded request processing

```
Internet → Axum (multi-threaded) → mpsc channel → Single interpreter thread → Response
```

The interpreter runs in a **single OS thread**. All requests funnel through one `mpsc::Receiver`. This is fine for fast operations (JSON, routing) but devastating when any operation blocks.

**Code:** `interpreter.rs:6950` — `rx.blocking_recv()` loop processes one request at a time.

### 2. Synchronous `postgres` crate

The stdlib uses the **blocking** `postgres::Client` (not `tokio-postgres`):

```rust
// stdlib/postgres.rs — CURRENT
use postgres::{Client, NoTls};
client.query(sql, &param_refs)  // ← blocks the interpreter thread
```

When this blocks for ~0.5ms per query (network roundtrip to PG), the interpreter thread is idle — it can't serve other requests. With 100 concurrent connections, each waits in line.

### 3. Single connection behind Mutex

```rust
// stdlib/postgres.rs:24 — CURRENT
static CONNECTION_REGISTRY: LazyLock<Mutex<HashMap<u64, Arc<Mutex<Client>>>>> = ...
```

`connect()` creates ONE `postgres::Client`. Every `query()` call locks the same `Mutex`. Even if we had multiple threads, they'd serialize through one DB connection.

### Why HTTP benchmarks are fast despite single thread

Plaintext response takes ~1µs of CPU. At 118K req/sec, the interpreter thread is busy but never waiting. The channel buffer (1024 deep) absorbs burst arrivals.

A DB query takes ~500µs of **wall time** (mostly network wait). During that 500µs, the interpreter thread sits idle. That's 500× more wall time per request, and none of it is useful work.

### Same problem affects ALL database backends

| Module | Client type | Connection model |
|--------|-------------|-----------------|
| `std/db/postgres` | Sync `postgres::Client` | Single `Arc<Mutex<Client>>` |
| `std/kv` (Redis) | Sync `redis::Connection` | Single `Arc<Mutex<RedisKV>>` |
| `std/kv` (SQLite) | Sync `rusqlite::Connection` | Single `Arc<Mutex<SQLiteKV>>` |
| `std/db/sqlite` | Sync `rusqlite::Connection` | Single `Arc<Mutex<Connection>>` |

## What other languages do

| Language | Strategy | Key insight |
|----------|----------|-------------|
| **Python (asyncpg)** | Async driver + connection pool | Event loop serves other requests during DB wait |
| **Node.js (pg)** | Async callbacks + pool | Same — non-blocking I/O |
| **Go (pgx)** | Goroutines + pool | Each request gets its own goroutine + pooled conn |
| **Ruby (Rails)** | Thread pool + connection pool | Each request thread has its own DB connection |
| **Rust (Actix)** | Async tokio-postgres + deadpool | Each task awaits DB, Tokio serves others meanwhile |

**Common pattern:** Nobody blocks a shared thread on DB I/O. They either (a) go async so the thread can serve other requests, or (b) use multiple threads each with their own connection.

## Proposed Fix: Interpreter Suspension at I/O Points

### Design: Cooperative async at native function boundaries

The interpreter stays single-threaded (no need for `Send + Sync` on `Value`). But when it hits a native function that does I/O, it **suspends the current request** and picks up the next one from the channel.

```
CURRENT (blocking):
  Request A → start query → [wait 0.5ms] → get result → send response
  Request B → [blocked waiting for A] → start query → [wait 0.5ms] → ...

PROPOSED (suspending):
  Request A → start query → [suspend, spawn async task]
  Request B → start query → [suspend, spawn async task]  
  Request A → [async task done] → resume → send response
  Request B → [async task done] → resume → send response
```

The interpreter thread is never idle. While DB queries are in-flight, it processes other requests or resumes completed ones.

### Implementation approach

**Phase 1: Connection pooling (low-hanging fruit)** — No interpreter changes needed.

Replace the single `Client` with a connection pool. The interpreter still blocks on each query, but at least multiple connections are available if we later add concurrency.

```rust
// Cargo.toml: add
// deadpool-postgres = "0.14"
// tokio-postgres = { version = "0.7", features = ["with-chrono-0_4", "with-serde_json-1"] }

// stdlib/postgres.rs — PHASE 1
use deadpool_postgres::{Pool, Config, Runtime, ManagerConfig, RecyclingMethod};
use tokio_postgres::NoTls;

static POOL_REGISTRY: LazyLock<Mutex<HashMap<u64, Pool>>> = ...;

fn pg_connect(connection_string: &str) -> Result<Value> {
    // Create a pool instead of a single client
    let mut cfg = Config::new();
    cfg.url = Some(connection_string.to_string());
    cfg.manager = Some(ManagerConfig { recycling_method: RecyclingMethod::Fast });
    let pool = cfg.create_pool(Some(Runtime::Tokio1), NoTls)?;
    // ... store in POOL_REGISTRY, return handle
}
```

**Expected impact:** Modest alone (~1.2–1.5× improvement) because the interpreter thread still blocks. But this is the foundation for Phase 2.

**Phase 2: Async bridge for DB calls** — The big unlock.

When the interpreter encounters `query()`, instead of blocking:

1. Grab a connection from the pool
2. Spawn a `tokio::task` that runs the query asynchronously
3. Store a "continuation" (the rest of the handler after the query)
4. Return control to the main loop (process next request from channel)
5. When the query task completes, enqueue a "resume" event
6. When the interpreter picks up the resume event, continue the handler with the query result

```rust
// Conceptual flow in interpreter main loop:
enum Event {
    NewRequest(HandlerRequest),
    QueryComplete { request_id: u64, result: Value },
}

loop {
    match rx.recv() {
        Event::NewRequest(req) => {
            // Start evaluating handler
            // If handler calls query(), interpreter returns Suspend(continuation)
            // Spawn async query task, store continuation
        }
        Event::QueryComplete { request_id, result } => {
            // Look up suspended handler, inject result, continue evaluation
        }
    }
}
```

**This is the pattern that makes Python asyncio and Node.js fast.** The single thread never blocks — it always has useful work to do.

**Expected impact:** 5–10× improvement on DB benchmarks. Should bring ntnt to 40K–80K req/sec on single-query, competitive with FastAPI/Actix.

**Phase 3: Apply to all I/O** — Same pattern for Redis, SQLite, `fetch()`.

Once the suspension mechanism exists for Postgres, extend it to:
- `std/kv` Redis operations
- `std/kv` SQLite operations (less impactful — SQLite is local, but still blocks)
- `std/http` `fetch()` calls (currently also blocking)
- `std/fs` file operations (lower priority)

### What ntnt users see (API surface — no changes)

```ntnt
# BEFORE (works today):
let pg = connect("postgres://...")?
let rows = query(pg, "SELECT * FROM users WHERE id = $1", [id])?

# AFTER (identical — no code changes needed):
let pg = connect("postgres://...")?
let rows = query(pg, "SELECT * FROM users WHERE id = $1", [id])?
```

The fix is 100% transparent. `connect()` returns a pool handle instead of a single-connection handle. `query()` suspends instead of blocking. Same syntax, same semantics, 10× faster.

## Implementation Plan

### Phase 1: Connection Pooling (~2-3 hours)
- [ ] Add `deadpool-postgres` + `tokio-postgres` to Cargo.toml
- [ ] Refactor `pg_connect()` to create a pool
- [ ] Refactor `pg_query()`/`pg_execute()` to grab connection from pool
- [ ] Bridge sync↔async: use `tokio::runtime::Handle::block_on()` to call async pool from sync code
- [ ] Run benchmarks — measure improvement
- [ ] Same pattern for Redis in `std/kv`
- [ ] Tests: existing tests must pass unchanged

### Phase 2: Interpreter Suspension (~1-2 days)
- [ ] Define `SuspendReason` enum (DbQuery, HttpFetch, KvOp, etc.)
- [ ] Add `Suspend` variant to interpreter's eval return type
- [ ] Implement continuation capture (save interpreter state at suspension point)
- [ ] Modify interpreter main loop to handle `Event::QueryComplete`
- [ ] Implement for `pg_query`, `pg_query_one`, `pg_execute`
- [ ] Run benchmarks — measure improvement
- [ ] Stress test: 100+ concurrent DB queries, verify no data corruption

### Phase 3: Extend to All I/O (~1 day)
- [ ] Redis KV operations
- [ ] SQLite operations  
- [ ] `fetch()` HTTP client
- [ ] Run full benchmark suite
- [ ] Update DD-038 results

## Risk Assessment

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Continuation capture is complex | Medium | Phase 1 gives value without it |
| Order-of-evaluation changes | Low | Suspension only at native call boundaries — user code runs identically |
| Connection pool exhaustion | Low | deadpool handles queuing; configurable pool size |
| SQLite doesn't benefit much | Known | SQLite is local — latency is µs not ms. Lower priority. |
| Existing tests break | Low | API surface unchanged; internal refactor only |

## Success Criteria

| Metric | Current | Phase 1 target | Phase 2 target |
|--------|---------|----------------|----------------|
| DB single query | 8,371 req/sec | ~12,000 | ~50,000 |
| 20 queries | 457 req/sec | ~600 | ~4,000 |
| Template (10q) | 899 req/sec | ~1,200 | ~8,000 |
| Plaintext (no regression) | 118,208 | ≥118,000 | ≥118,000 |

## Decision

Phase 1 is pure low-hanging fruit — swap sync client for pooled async client with sync bridge. No interpreter changes. Guaranteed improvement with zero API changes. **Start here.**

Phase 2 is the real win but more invasive. The continuation/suspension mechanism touches the interpreter core. Do it after Phase 1 proves the approach.
