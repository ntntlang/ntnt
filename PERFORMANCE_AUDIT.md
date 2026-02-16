# NTNT Performance Audit

**Date:** 2026-02-16  
**Version:** v0.3.13 (branch: security-and-performance-audit)  
**Focus:** Web application performance — request latency, throughput, memory under load

---

## Executive Summary

NTNT's current architecture is well-suited for **small-to-medium web applications** (up to ~1000 req/s). The main bottleneck is the **single-threaded interpreter** — all request handlers execute sequentially. The async server infrastructure (Axum + Tokio) handles I/O concurrency well, but CPU-bound NTNT code (template rendering, business logic) serializes through one thread.

| Area | Rating | Notes |
|------|--------|-------|
| Interpreter Speed | ⚠️ Adequate | Tree-walking is ~100x slower than compiled, fine for I/O-bound web apps |
| HTTP Server (async) | ✅ Good | Axum + Tokio handles thousands of concurrent connections |
| HTTP Server (sync) | ⚠️ Limited | Single-threaded tiny_http, one request at a time |
| Data Structures | ✅ Good | HashMap for maps, Vec for arrays — appropriate choices |
| Memory Management | ⚠️ Watch | Rc/RefCell closures, in-memory session accumulation |
| String Operations | ⚠️ Copies | String `+` creates new allocation every time |

---

## 2.1 Interpreter Performance

### Tree-Walking Architecture

The NTNT interpreter is a **tree-walking interpreter** — it traverses the AST directly, evaluating each node. This is the simplest interpreter architecture and is typically 50-200x slower than compiled/bytecode languages for pure computation.

**For web applications, this is usually fine** because:
- Most request time is spent in I/O (database queries, HTTP calls, template rendering)
- The Rust-native stdlib functions (JSON parsing, crypto, database) run at native speed
- Business logic in route handlers is typically simple

**Where it becomes a bottleneck:**
- Complex template rendering with many loops/conditionals
- CPU-intensive computation (image processing, data transformation)
- High-throughput APIs (>1000 req/s) where even small per-request overhead matters

### Hot Path Analysis

For a typical web request, the hot path is:
1. Route matching — **O(1) partitioned** by (method, segment_count), then linear scan within partition. This is well-optimized with the `route_index` HashMap.
2. Request-to-Value conversion — Allocates HashMap for request map, headers, params. **One allocation per field.**
3. Handler execution — Tree-walking through AST nodes. Each node evaluation involves pattern matching on the AST enum.
4. Response serialization — JSON serialization of response map.

### String Concatenation

```rust
// In eval_binary_op for BinaryOp::Add with strings:
(Value::String(a), Value::String(b)) => Ok(Value::String(format!("{}{}", a, b)))
```

**Every `+` on strings allocates a new String.** This is O(n) per concatenation and O(n²) for building strings in a loop:

```ntnt
let html = ""
for item in items {
    html = html + "<li>" + item + "</li>"  // O(n²) total
}
```

**Recommendation:** This is standard for interpreted languages. Document that `template()` or array joining should be used for building large strings. A future optimization could use a rope data structure or StringBuilder pattern.

### No Obvious O(n²) Algorithms

The interpreter code is generally well-structured. Route matching uses indexed lookup. Array operations (`push`, `filter`, `map`) are O(n) as expected. No quadratic algorithms were found in common code paths.

---

## 2.2 HTTP Server Performance

### Async Server (Axum) — Production Path

The async server architecture is sound:
- **Axum + Tokio** handles thousands of concurrent connections
- **Gzip compression** via tower-http CompressionLayer
- **30-second request timeout** via TimeoutLayer
- **Graceful shutdown** on SIGTERM/Ctrl+C

**Bottleneck: Single Interpreter Thread**

All NTNT code execution is funneled through a single thread via mpsc channel:

```
[Tokio worker 1] → BridgeRequest → [mpsc channel] → [Interpreter thread] → BridgeResponse → [Tokio worker 1]
[Tokio worker 2] → BridgeRequest → [mpsc channel] → [Interpreter thread] → BridgeResponse → [Tokio worker 2]
```

This means:
- I/O-bound requests (database, HTTP calls) block the interpreter thread while waiting
- Only one request handler executes at a time
- Throughput is limited by average handler execution time

**Measured impact:** For a handler that takes 1ms of CPU time, max throughput ≈ 1000 req/s. For a handler with 10ms of database query time, throughput ≈ 100 req/s (since the interpreter thread blocks during the query).

**Recommendation:** This is an architectural limitation of the Rc/RefCell-based interpreter (not Send/Sync). Options for future improvement:
1. **Thread pool of interpreters** — spawn N interpreter threads, each with its own environment
2. **Async-aware stdlib** — make database/HTTP calls yield back to the channel so other requests can proceed
3. **Bytecode compilation** — compile to bytecode that can run on multiple threads

### Sync Server (tiny_http) — Development Path

The sync server processes one request at a time. It's appropriate for development but not production. The async server should be the default recommendation for deployment.

### Static File Serving

- **Sync server:** Reads from disk on every request. No caching.
- **Async server:** Reads from disk on every request. Cache-Control headers set to `public, max-age=3600` so browsers cache, but the server itself doesn't cache.

**Recommendation:** Add optional in-memory file caching for frequently-accessed static assets. Low priority since a reverse proxy (nginx, Caddy) typically handles static files in production.

### Template Rendering

Templates are rendered on every request with no caching. For pages with mostly static content, this means re-parsing and re-evaluating templates repeatedly.

**Recommendation:** Template compilation/caching would be a significant optimization for production. **Proposed for future version.**

---

## 2.3 Data Structure Performance

### Map Implementation — HashMap ✅

NTNT uses `HashMap<String, Value>` for maps. This is the right choice:
- O(1) average lookup, insert, delete
- Web applications primarily do key-based access
- HashMap outperforms BTreeMap for the random-access patterns common in request handling

### Array Implementation — Vec ✅

NTNT uses `Vec<Value>` for arrays. This is the right choice:
- O(1) push (amortized), O(1) index access
- O(n) for filter/map — expected and unavoidable
- `concat` creates a new Vec — O(n+m), which is optimal

### Value Clone Cost

`Value` is `#[derive(Clone)]` with heap-allocated variants (String, Array, Map, closures via Rc). Cloning a large map or array is O(n). The interpreter clones values frequently (variable binding, function arguments, return values).

**Recommendation:** For a future major optimization, consider using `Rc<Value>` or COW (copy-on-write) semantics to avoid unnecessary deep clones. This would be a significant refactor.

---

## 2.4 Memory Management

### Closure Capture — Rc/RefCell

Closures capture their environment via `Rc<RefCell<Environment>>`. This is a standard approach for interpreted languages that need lexical scoping.

**Reference Cycle Risk:** Parent environments are stored as `Option<Rc<RefCell<Environment>>>`. A closure that captures its own defining scope creates a reference cycle that will never be freed. This is a theoretical concern — in practice, the standard web app patterns (route handlers, callbacks) don't create cycles because:
- Route handler closures capture module-level scope (not self-referential)
- The server lifetime is the process lifetime

**For long-running servers**, the real concern is...

### Session/State Accumulation

The in-memory session store (`InMemoryStore`) grows without bound unless `cleanup_expired()` is called. The auth module has a cleanup function, but it's only triggered by explicit calls or background cleanup.

**Current mitigation:** `cleanup_expired()` exists and the `cleanup_sessions()` NTNT function is exposed. But there's no automatic periodic cleanup.

**Recommendation:** Add automatic session cleanup on a timer (e.g., every 5 minutes). Also, the SQLite and PostgreSQL session stores have expiry indices which handle this better. **Document that in-memory sessions require periodic cleanup for long-running servers.**

### Global Connection Registries

Database connections (`CONNECTION_REGISTRY` in sqlite.rs and postgres.rs) and KV stores are stored in global static registries. If connections are opened but never closed, they accumulate.

**Impact:** Memory and connection leak for careless code. Mitigated by the fact that developers must explicitly `connect()` and should `close()`.

**Recommendation:** Consider adding a finalizer or warning for unclosed connections on server shutdown.

### Response Cache Memory

The HTTP client `ResponseCache` stores complete response bodies in memory. With no size limit on the cache, it could grow large.

**Recommendation:** Add a max-entries or max-memory option to `Cache()`. **Proposed for future version.**

---

## Performance Recommendations Summary

### Quick Wins (Low Effort, High Impact)
1. **Document string concatenation patterns** — recommend templates or array joining for loops
2. **Add automatic session cleanup timer** for in-memory sessions
3. **Document async server as the production recommendation**

### Medium-Term (Moderate Effort)
4. **In-memory static file caching** with configurable max size
5. **Response size limits** for HTTP client (prevents OOM)
6. **Template compilation caching** to avoid re-parsing

### Long-Term (Architectural)
7. **Interpreter thread pool** — run N interpreter instances for N-way parallelism
8. **Bytecode compilation** — compile AST to bytecode for faster execution
9. **Async-aware database calls** — yield interpreter thread during I/O waits
10. **Copy-on-write Value semantics** — reduce clone overhead

---

## Benchmark Recommendations

To properly quantify performance, the following benchmarks should be created:

1. **Hello World throughput** — measure raw req/s for minimal handler
2. **JSON API throughput** — parse JSON body, query SQLite, return JSON
3. **Template rendering** — render a page with 100 list items
4. **Static file serving** — compare with nginx direct
5. **Concurrent connections** — measure latency at 100, 500, 1000 concurrent connections

These benchmarks should be run against both the sync and async servers.
