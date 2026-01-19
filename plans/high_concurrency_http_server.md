# High-Concurrency HTTP Server Migration Plan

## Executive Summary

This plan outlines the migration from `tiny_http` (thread-per-connection blocking model) to an async runtime for handling high-concurrency production workloads. The goal is to support 10,000+ concurrent connections efficiently.

---

## Current Architecture Analysis

### How It Works Today

```
┌─────────────────────────────────────────────────────────────┐
│                    Current: tiny_http                        │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   Client Request                                             │
│        │                                                     │
│        ▼                                                     │
│   ┌─────────────┐     spawn      ┌─────────────┐            │
│   │ tiny_http   │ ──────────────▶│   Thread    │            │
│   │   Server    │                │  (blocked)  │            │
│   └─────────────┘                └──────┬──────┘            │
│                                         │                    │
│                                         ▼                    │
│                                  ┌─────────────┐            │
│                                  │ Interpreter │            │
│                                  │  (sync)     │            │
│                                  └──────┬──────┘            │
│                                         │                    │
│                                         ▼                    │
│                                    Response                  │
└─────────────────────────────────────────────────────────────┘
```

**Key Files:**
- `src/stdlib/http_server.rs` - Server state, routing, response helpers
- `src/interpreter.rs:3756-3889` - Request handling loop
- `Cargo.toml:49` - `tiny_http = "0.12"`

**Current Characteristics:**
| Metric | Value |
|--------|-------|
| Concurrency Model | Thread-per-connection |
| Async Support | None |
| Memory per connection | ~8KB stack |
| Max practical connections | ~1000 (OS thread limit) |
| Keep-alive | Disabled (`Connection: close`) |
| HTTP/2 | Not supported |

### Why It Can't Scale

1. **Thread exhaustion**: Each connection spawns a thread. At 10k connections = 10k threads = ~80MB stack memory just for thread stacks
2. **Context switching**: OS struggles with >1000 threads
3. **No keep-alive**: Forces TCP handshake per request (3-way handshake overhead)
4. **Blocking I/O**: Thread blocked during all I/O operations

---

## Recommended Solution: Tokio + Hyper (via Axum)

### Why Axum?

| Framework | Pros | Cons |
|-----------|------|------|
| **Axum** (recommended) | Modern API, tower middleware, great ergonomics, active maintenance | Newer |
| Actix-web | Fastest benchmarks, mature | Actor model complexity, macros |
| Hyper (raw) | Maximum control | Verbose, low-level |

**Axum is the best fit because:**
1. Built on tokio+hyper (proven async stack)
2. Tower middleware ecosystem (logging, tracing, compression)
3. Type-safe extractors match NTNT's contract philosophy
4. Excellent documentation
5. Maintained by tokio team

### Target Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Target: Axum + Tokio                      │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   10,000+ Clients                                            │
│        │                                                     │
│        ▼                                                     │
│   ┌─────────────┐     epoll/      ┌─────────────┐           │
│   │   Hyper     │    kqueue       │   Tokio     │           │
│   │  (HTTP/1+2) │ ◄──────────────▶│  Runtime    │           │
│   └─────────────┘                 │  (N threads)│           │
│        │                          └──────┬──────┘           │
│        │                                 │                   │
│        │          ┌──────────────────────┼──────────────┐   │
│        │          │                      │              │   │
│        ▼          ▼                      ▼              ▼   │
│   ┌─────────┐ ┌─────────┐         ┌─────────┐    ┌─────────┐│
│   │ Task 1  │ │ Task 2  │   ...   │ Task N  │    │ Task M  ││
│   │ (async) │ │ (async) │         │ (async) │    │ (async) ││
│   └────┬────┘ └────┬────┘         └────┬────┘    └────┬────┘│
│        │           │                   │              │     │
│        ▼           ▼                   ▼              ▼     │
│   ┌─────────────────────────────────────────────────────┐   │
│   │              NTNT Interpreter (sync)                 │   │
│   │         (called via spawn_blocking)                  │   │
│   └─────────────────────────────────────────────────────┘   │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

**Target Characteristics:**
| Metric | Value |
|--------|-------|
| Concurrency Model | Event-loop + task pool |
| Async Support | Full (tokio) |
| Memory per connection | ~few KB (no thread stack) |
| Max practical connections | 100,000+ |
| Keep-alive | Enabled |
| HTTP/2 | Supported |
| Target throughput | 100k+ req/sec |

---

## Implementation Plan

### Phase 1: Foundation (Non-Breaking)

**Goal:** Add async infrastructure without changing current behavior.

**Changes:**

1. **Add dependencies to Cargo.toml:**
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
axum = "0.7"
tower = "0.4"
tower-http = { version = "0.5", features = ["cors", "compression-gzip", "trace"] }
hyper = { version = "1", features = ["full"] }
# Keep tiny_http for backward compatibility during migration
```

2. **Create new module `src/stdlib/http_server_async.rs`:**
   - Async versions of response helpers
   - New `AsyncServerState` with `Arc<RwLock<>>` for shared state
   - Axum router construction from NTNT routes

3. **Add feature flag:**
```toml
[features]
default = []
async-http = ["tokio", "axum", "tower", "tower-http", "hyper"]
```

### Phase 2: Async Server Implementation

**New file: `src/stdlib/http_server_async.rs`**

Key components:

```rust
use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
    body::Body,
};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared state for the async server
pub struct AsyncServerState {
    pub routes: Vec<(Route, Value)>,
    pub static_dirs: Vec<(String, String)>,
    pub middleware: Vec<Value>,
    // Reference to interpreter for handler execution
    pub interpreter: Arc<RwLock<Interpreter>>,
}

/// Start the async HTTP server
pub async fn start_server_async(
    port: u16,
    state: Arc<AsyncServerState>,
) -> Result<()> {
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .map_err(|e| IntentError::RuntimeError(format!("Failed to bind: {}", e)))?;

    println!("🚀 NTNT async server running on http://localhost:{}", port);

    axum::serve(listener, app)
        .await
        .map_err(|e| IntentError::RuntimeError(format!("Server error: {}", e)))
}
```

**Key Challenge: Interpreter Integration**

The NTNT interpreter is synchronous. We need to bridge async Axum handlers to sync interpreter calls:

```rust
async fn handle_request(
    State(state): State<Arc<AsyncServerState>>,
    req: axum::extract::Request,
) -> impl IntoResponse {
    // Convert Axum request to NTNT Value
    let req_value = axum_request_to_value(&req).await;

    // Execute handler in blocking thread pool
    // This prevents blocking the async runtime
    let response = tokio::task::spawn_blocking(move || {
        let mut interpreter = state.interpreter.blocking_write();
        interpreter.call_function(handler, vec![req_value])
    })
    .await
    .unwrap();

    // Convert NTNT Value to Axum response
    value_to_axum_response(response)
}
```

### Phase 3: Interpreter Changes

**File: `src/interpreter.rs`**

1. **Add async listen variant:**
```rust
// In execute_call for "listen" builtin
if self.async_mode {
    self.run_async_server(port)?;
} else {
    self.run_sync_server(port)?; // Current implementation
}
```

2. **Make interpreter `Send + Sync`:**
   - Wrap mutable state in `Arc<RwLock<>>`
   - Or use message-passing for interpreter access

3. **Add CLI flag:**
```bash
ntnt run server.tnt --async    # Use async server
ntnt run server.tnt            # Default: sync (backward compatible)
```

### Phase 4: Feature Parity

Ensure all existing features work with async server:

| Feature | Status | Notes |
|---------|--------|-------|
| Route handlers | Required | Via spawn_blocking |
| Middleware | Required | Tower middleware layer |
| Static files | Required | tower-http ServeDir |
| Hot-reload | Required | File watcher + state update |
| Query params | Required | Axum Query extractor |
| Route params | Required | Axum Path extractor |
| JSON responses | Required | Same logic |
| HTML responses | Required | Same logic |
| Contract violations | Required | Error handling |

### Phase 5: Performance & Production Features

1. **Graceful shutdown:**
```rust
let shutdown = async {
    tokio::signal::ctrl_c().await.ok();
    println!("Shutting down...");
};

axum::serve(listener, app)
    .with_graceful_shutdown(shutdown)
    .await
```

2. **Connection limits:**
```rust
use tower::limit::ConcurrencyLimitLayer;

let app = Router::new()
    .layer(ConcurrencyLimitLayer::new(10_000));
```

3. **Request timeouts:**
```rust
use tower_http::timeout::TimeoutLayer;

let app = Router::new()
    .layer(TimeoutLayer::new(Duration::from_secs(30)));
```

4. **Compression:**
```rust
use tower_http::compression::CompressionLayer;

let app = Router::new()
    .layer(CompressionLayer::new());
```

5. **HTTP/2:**
```rust
// Automatic with Axum when using HTTPS
// For HTTP/2 over cleartext (h2c), additional config needed
```

---

## Migration Path

### For Users

**Phase 1-2:** No changes required. Existing code works.

**Phase 3:** Opt-in async mode:
```bash
# Old way (still works)
ntnt run server.tnt

# New way (opt-in)
ntnt run server.tnt --async
```

**Phase 4+:** Consider making async the default:
```bash
# New default
ntnt run server.tnt

# Explicit sync mode for compatibility
ntnt run server.tnt --sync
```

### Breaking Changes (None Required Initially)

The migration can be 100% backward compatible:
1. Keep tiny_http as default
2. Add async as opt-in feature
3. Eventually flip the default (major version bump)

---

## Risks & Mitigations

| Risk | Impact | Mitigation |
|------|--------|------------|
| Interpreter not thread-safe | High | Use `spawn_blocking` + locks |
| Feature regression | Medium | Comprehensive test suite |
| Performance worse than expected | Medium | Benchmark before/after |
| Complexity increase | Medium | Good abstractions, documentation |
| Async complexity for users | Low | NTNT code stays synchronous |

---

## Testing Strategy

1. **Unit tests:** Async versions of existing http_server tests
2. **Integration tests:** Full request/response cycle
3. **Load tests:**
   - wrk benchmark: `wrk -t12 -c1000 -d30s http://localhost:8080/`
   - Compare sync vs async throughput
4. **Soak tests:** Run under load for extended periods

---

## Success Criteria

| Metric | Current | Target |
|--------|---------|--------|
| Max concurrent connections | ~1,000 | 10,000+ |
| Requests/sec (hello world) | ~10k | 100k+ |
| Memory at 1k connections | ~80MB | ~10MB |
| P99 latency under load | 50ms+ | <10ms |
| Connection reuse | None | Keep-alive |

---

## Timeline Estimate

| Phase | Work |
|-------|------|
| Phase 1 | Add dependencies, feature flags, async module skeleton |
| Phase 2 | Implement async server, request/response conversion |
| Phase 3 | Interpreter integration, CLI flag |
| Phase 4 | Feature parity, middleware, static files |
| Phase 5 | Performance tuning, production features |

---

## Dependencies to Add

```toml
# Cargo.toml additions
[dependencies]
tokio = { version = "1", features = ["full"], optional = true }
axum = { version = "0.7", optional = true }
tower = { version = "0.4", optional = true }
tower-http = { version = "0.5", features = ["cors", "compression-gzip", "trace", "fs"], optional = true }

[features]
default = []
async-http = ["tokio", "axum", "tower", "tower-http"]
```

---

## Alternative Considered: Threadpool with tiny_http

Instead of full async migration, we could add a bounded threadpool:

```rust
let pool = rayon::ThreadPoolBuilder::new()
    .num_threads(num_cpus::get() * 2)
    .build()
    .unwrap();

for request in server.incoming_requests() {
    pool.spawn(move || {
        handle_request(request);
    });
}
```

**Pros:** Simpler, less code change
**Cons:** Still limited to thread count, no HTTP/2, no keep-alive

**Verdict:** Not recommended for "high load" production use. Go with async.

---

## Implementation Status

### Phase 1: Foundation ✅ Complete
- Created `src/stdlib/http_server_async.rs` module
- Created `src/stdlib/http_bridge.rs` for interpreter communication

### Phase 2: Async Server Implementation ✅ Complete
- Channel-based communication between Axum and interpreter
- `BridgeRequest` / `BridgeResponse` for thread-safe data passing
- Interpreter runs in main thread, Tokio in worker threads

### Phase 3: Feature Parity ✅ Complete
- [x] Static file serving with MIME type detection
- [x] Middleware pass-through (via interpreter loop)
- [x] Hot-reload support (auto-enabled, syncs routes on file change)

### Phase 4: Production Features ✅ Complete
- [x] Graceful shutdown with SIGTERM/SIGINT handling
- [x] Request timeouts (30s default via Tower middleware)
- [x] Connection limits (10,000 max via config)
- [x] Gzip compression (via Tower middleware)
- [x] Tracing layer for request logging

### Phase 5: Production Configuration ✅ Complete
- [x] Configurable request timeout via `--timeout` flag and `NTNT_TIMEOUT` env var
- [x] HTTP/2 - Not needed (handled by reverse proxy like nginx/cloudflared)
- [x] Rate limiting - Not needed (handled by reverse proxy)
- [x] TLS - Not needed (handled by reverse proxy)

### Phase 6: Full Migration ✅ Complete
- [x] Removed `tiny_http` dependency
- [x] Removed `--async` flag (async is now the only mode)
- [x] Removed sync server code from interpreter
- [x] Async server is now the default for all `ntnt run` commands

## Building and Running

```bash
# Build NTNT
cargo build --release

# Run HTTP server (uses Axum + Tokio automatically)
ntnt run server.tnt

# Configure request timeout (default: 30 seconds)
ntnt run server.tnt --timeout 60

# Or via environment variable
NTNT_TIMEOUT=60 ntnt run server.tnt
```

## Production Readiness Checklist

| Feature | Status |
|---------|--------|
| High concurrency (10k+ connections) | ✅ Implemented |
| Request timeouts | ✅ 30s default |
| Graceful shutdown | ✅ SIGTERM/SIGINT |
| Static file serving | ✅ With MIME types |
| Gzip compression | ✅ Enabled |
| Keep-alive | ✅ Automatic |
| Route parameters | ✅ Working |
| JSON responses | ✅ Working |
| Middleware | ✅ Working |
| Hot-reload | ✅ Auto-enabled |

## Benchmark Results

Tested on macOS with `wrk` benchmark tool.

### 100 Concurrent Connections

| Endpoint | Requests/sec | Notes |
|----------|-------------|-------|
| `/` (text) | 138,931 | Simple text response |
| `/json` | 124,225 | JSON serialization |
| `/compute` | 2,786 | CPU-bound (interpreter bottleneck) |

### 1000 Concurrent Connections

| Endpoint | Requests/sec | Latency (avg) |
|----------|-------------|---------------|
| `/json` | 128,161 | 7.71ms |

### Key Observations

1. **I/O-bound workloads** achieve 100k+ requests/sec
2. **CPU-bound workloads** are limited by interpreter speed (~2.8k req/s)
3. **Low latency** even under high concurrency (sub-10ms average)
4. **High connection counts** handled efficiently via Tokio event loop

### Running Benchmarks

```bash
# Build release
cargo build --release

# Start server
./target/release/ntnt run examples/benchmark_server.tnt

# Run benchmark (in another terminal)
wrk -t4 -c100 -d10s http://localhost:8080/json
```
