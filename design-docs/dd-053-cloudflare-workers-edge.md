# DD-053: Cloudflare Workers / Edge Runtime

**Status:** Backlog  
**Author:** Larri  
**Created:** 2026-03-28  
**Priority:** Exploratory  
**Depends on:** Core interpreter stabilization (v0.5+)

---

## Problem

ntnt currently runs as a native binary on VPS/server infrastructure. There's no path to edge deployment — Cloudflare Workers, Deno Deploy, or similar runtimes. For ntnt to be a viable alternative to JavaScript/TypeScript for web development, edge deployment is table stakes.

The question: what would it take to run ntnt apps on Cloudflare Workers with the same code that runs on a VPS?

## Prior Art

### How Other Languages/Frameworks Do It

**Python (Cloudflare Workers):**
- CPython compiled to WASM via Pyodide (~11MB compressed with snapshotting)
- I/O replaced with JS API bindings (fetch, KV, D1, R2)
- Memory snapshots to avoid cold-start interpreter initialization
- Preloaded packages in snapshot

**Next.js (`@opennextjs/cloudflare`):**
- Build-time transform layer that adapts Node.js assumptions to Workers primitives
- Replaces filesystem cache with KV/D1, rewires routing
- Not all features work identically — some edge-runtime constraints

**Hono / Remix:**
- Built for Workers from day one — no adaptation layer needed
- Request/response model maps directly to Workers

**SvelteKit / Astro:**
- Adapter pattern: `adapter-cloudflare` swaps Node APIs for Workers APIs at build time

**Common pattern:** Either (1) native to the runtime, (2) build-time transform/adapter, or (3) WASM compilation of an interpreter.

### ntnt's Structural Advantages Over Python/WASM

- Rust interpreter is simpler than CPython — no GC, no GIL, smaller surface area
- stdlib is small and purpose-built for web servers — no general-purpose ecosystem drag
- Language already designed around request/response patterns
- Rust → WASM is a more natural compilation target than C → WASM (CPython)

## Cloudflare Workers Constraints

| Constraint | Limit (Paid Plan) | Impact on ntnt |
|---|---|---|
| WASM binary size | 10MB compressed | Interpreter is ~20MB native; needs aggressive stripping |
| Threading | Single-threaded (no SharedArrayBuffer) | spawn/channels/jobs/select all blocked |
| TCP sockets | No raw TCP | std/db/postgres, std/kv (Redis) need rewiring |
| Filesystem | None | Template loading, serve_static, std/fs all blocked |
| Execution time | 30s CPU / 15min wall (cron) | Fine for web requests |
| Memory | 128MB | Fine for most apps |

## Proposed Approach

**WASM interpreter with CF-native bindings.** Not "compile to JS" — ntnt stays ntnt. The interpreter itself runs as WASM inside the Worker, with stdlib functions routing through CF APIs.

### Phase 1: WASM Compilation Target (~2-4 weeks)

Compile the ntnt interpreter with `--target wasm32-wasi`.

**Strip out (feature-gated behind `#[cfg(not(target_arch = "wasm32"))]`):**
- Threading: spawn, channels, select, schedule, after, sleep_ms
- Job system: entire std/jobs
- Raw TCP: direct postgres/redis socket connections
- Filesystem: std/fs (read_file, write_file, etc.)
- SQLite: bundled sqlite3 (replaced by D1)

**Keep:**
- Core interpreter (parser, evaluator, type checker)
- Template engine (templates bundled at build time)
- String, JSON, collections, crypto, time stdlib
- HTTP request/response model
- Route matching and middleware pipeline

**Target:** `ntnt.wasm` under 5MB compressed.

- [ ] Add `wasm32-wasi` target to Cargo workspace
- [ ] Feature-gate threading/job/fs/tcp modules
- [ ] Compile and benchmark stripped interpreter WASM size
- [ ] Test core language features (parsing, evaluation, templates) in WASM

### Phase 2: Cloudflare Bindings (~2-3 weeks)

Implement CF-native backends behind the same ntnt stdlib API surface.

| ntnt API | Server Backend | Edge Backend |
|---|---|---|
| `fetch()` | Native HTTP | Workers fetch API (JS import) |
| `std/db/postgres` | TCP (deadpool-postgres) | Hyperdrive (PG over HTTP) |
| `std/db/sqlite` | Bundled SQLite | D1 (CF SQLite) |
| `std/kv` | Redis (TCP) | Workers KV |
| `serve_static()` | Filesystem | R2 bucket |
| `template()` | Read from disk | Bundled in WASM at build time |
| `listen()` | TCP listener | Workers fetch handler |

**Key principle:** Same .tnt code, different runtime backend. App developers don't import different modules for edge vs server.

- [ ] Design JS↔WASM bridge interface (wasm-bindgen or manual imports)
- [ ] Implement edge `fetch()` via JS fetch API
- [ ] Implement D1 adapter for `std/db/sqlite` API
- [ ] Implement Hyperdrive adapter for `std/db/postgres` API
- [ ] Implement Workers KV adapter for `std/kv` API
- [ ] Implement R2 adapter for static assets
- [ ] Template bundling at build time (embed in WASM binary)

### Phase 3: Developer Tooling (~1-2 weeks)

- [ ] `ntnt build --target cloudflare` — bundles .tnt files + WASM interpreter into a Worker
- [ ] Auto-generate `wrangler.toml` with D1/KV/R2 bindings
- [ ] `ntnt dev --edge` — local emulation via Miniflare
- [ ] Upload static assets to R2 as part of deploy
- [ ] `ntnt deploy --edge` — full deploy pipeline (build → wrangler publish)

### Phase 4: Cold Start Optimization

- [ ] Memory snapshots — snapshot initialized interpreter state (parsed .tnt files, loaded config)
- [ ] Pre-parse .tnt files at build time → include bytecode/AST in WASM binary
- [ ] Measure and benchmark cold start latency vs Python Workers
- [ ] Explore V8 snapshot integration (if CF supports custom snapshots)

## Edge Job System — CF Queues + Durable Objects

The job system actually maps *better* to edge than threading does. Jobs are inherently message-passing, and that's exactly what CF Queues are.

### Architecture

```
┌─────────────────────────────────────────────┐
│  Request Worker (your ntnt app)             │
│                                             │
│  enqueue("SendEmail", args)                 │
│    └─→ CF Queue.send()                      │
│                                             │
│  schedule("*/5 * * * *", cleanup_fn)        │
│    └─→ mapped to cron trigger at build time │
└─────────────────────────────────────────────┘
           │
           ▼
┌─────────────────────────────────────────────┐
│  Consumer Worker (auto-generated)           │
│                                             │
│  Receives queue messages                    │
│  Runs the Job's perform() function          │
│  Updates status in D1                       │
│  Retries with backoff on failure            │
└─────────────────────────────────────────────┘
           │
           ▼
┌─────────────────────────────────────────────┐
│  Durable Object (per-queue)                 │
│                                             │
│  Dedup (unique job tracking)                │
│  Delayed delivery (enqueue_in → alarm API)  │
│  Rate limiting per queue                    │
└─────────────────────────────────────────────┘
```

### API Mapping

| ntnt Jobs (server) | Edge Backend | Notes |
|---|---|---|
| `enqueue("Name", args)` | `CF Queue.send()` | Direct mapping |
| `enqueue_in("Name", delay, args)` | Durable Object alarm API | DO wakes after delay, pushes to Queue |
| `Job Name on queue { perform(args) { } }` | Consumer Worker | Auto-generated from Job definitions at build time |
| Job worker threads | CF Queue consumers | CF auto-scales consumers to match queue depth |
| Priority queues (critical/high/normal/low) | Multiple CF Queues | One queue per priority level |
| Dead letter | CF Queue built-in DLQ | Native support |
| Unique jobs (dedup) | Durable Object | DO holds dedup lock with TTL |
| `schedule("*/5 * * * *", fn)` | Cron Triggers in wrangler.toml | Mapped at build time |
| `job_status()` / `list_jobs()` | D1 table tracking job state | Build step generates status tracking schema |
| `worker_status()` / `scale_workers()` | N/A — CF manages scaling | Not needed; CF auto-scales |
| Control socket / `ntnt workers` CLI | `wrangler tail` + CF dashboard | Different tooling, same observability |

### What the .tnt Code Looks Like — Identical

```ntnt
Job SendEmail on emails {
    perform(args) {
        // same code, runs in consumer Worker instead of a thread
    }
}

enqueue("SendEmail", map { "to": user_email })
enqueue_in("FollowUp", 3600, map { "user_id": id })
```

### Build Step Responsibilities

`ntnt build --target cloudflare` would:
1. Parse Job definitions in .tnt files
2. Generate a consumer Worker that runs each job's `perform()` function
3. Wire up Queue bindings in `wrangler.toml`
4. Map `schedule()` calls to cron triggers
5. Generate Durable Object classes for dedup/delayed jobs
6. Create D1 migration for job status tracking table

### What You Lose vs Server

- `worker_status()` / `scale_workers()` — CF manages scaling, not you
- Control socket — no persistent process to connect to
- Fine-grained priority *within* a single queue — use separate queues instead
- `ntnt workers` CLI — replaced by `wrangler tail` and CF dashboard

### What You Gain

- **Auto-scaling** — CF scales consumers to match queue depth
- **Global distribution** — jobs process at the nearest edge location
- **Zero infrastructure** — no servers, no thread pools, no process management
- **Built-in DLQ and retry** — CF handles failure/retry natively
- **Cost model** — pay per message, not per idle server

## The Two-Profile Question

With edge deployment, ntnt would have two runtime profiles:

| Feature | Server (full) | Edge (Workers) |
|---|---|---|
| spawn/channels/select | ✅ | ❌ |
| Job system (std/jobs) | ✅ | ✅ (CF Queues + Durable Objects) |
| Filesystem (std/fs) | ✅ | ❌ |
| PostgreSQL (TCP) | ✅ | ✅ (Hyperdrive) |
| SQLite | ✅ | ✅ (D1) |
| KV / Redis | ✅ | ✅ (Workers KV) |
| Templates | ✅ | ✅ (bundled) |
| Static files | ✅ | ✅ (R2) |
| HTTP server | ✅ | ✅ |

**Mitigation options:**
1. **Runtime feature detection:** `if is_edge() { ... } else { spawn(...) }` — apps adapt at runtime
2. **Lint-time checking:** `ntnt lint --target edge` flags unsupported APIs before deploy
3. **Clear docs:** "ntnt edge" is a defined subset, not a broken version
4. **Graceful errors:** Calling `spawn()` on edge returns a clear error ("spawn not available on edge runtime") instead of crashing

Recommended: Option 2 + 3. Catch it at lint time, document the subset clearly.

## The Pitch

> Write web apps in ntnt. Deploy to your VPS or to 300+ Cloudflare edge locations with `ntnt deploy --edge`. Same code, same language, same stdlib. No JavaScript required.

ntnt would be the first non-JS, non-Python language with native edge deployment that isn't "compile to JS." That's a real differentiator.

## Implementation Independence

**No Cloudflare partnership or special access required.** Everything in this DD uses public, GA Cloudflare APIs:

- Workers WASM — upload a binary, CF runs it. Standard `wasm-bindgen` toolchain.
- Queues, Durable Objects, D1, R2, Hyperdrive, KV, Cron Triggers — all GA on paid plans.
- `wrangler` CLI for deployment — public, self-service.
- The entire build pipeline (`ntnt build --target cloudflare`) is our code.

The only potential blocker: if WASM binary exceeds 10MB compressed (limit increase isn't self-service). Size budget analysis suggests 3-4MB — well under.

Compare to Python Workers — CF had to modify the Workers runtime itself for Pyodide/memory snapshots. The ntnt approach ships a standard WASM binary that uses standard Workers APIs. From CF's perspective, it's just another Rust-based Worker.

## Codebase Complexity Analysis

### What Doesn't Change (~80% of codebase)

The core interpreter is completely untouched:
- Parser — unchanged
- Evaluator — unchanged (doesn't know what platform it's on)
- Type checker — unchanged
- Template engine — unchanged
- Language test suite — unchanged, runs on both targets
- Job DSL syntax — unchanged (build step handles the mapping)

### What's New

**1. Feature-gated modules (standard Rust conditional compilation):**
```rust
#[cfg(not(target_arch = "wasm32"))]
mod threading;        // spawn, channels, select

#[cfg(not(target_arch = "wasm32"))]
mod tcp_postgres;     // direct TCP connections

#[cfg(target_arch = "wasm32")]
mod edge_bindings;    // CF API bridges
```

**2. Stdlib abstraction layer (~500-800 lines):**
```rust
trait Database {
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>>;
    fn execute(&self, sql: &str, params: &[Value]) -> Result<u64>;
}

struct TcpPostgres { ... }        // server — existing code, wrapped
struct HyperdrivePostgres { ... } // edge — new, ~100 lines
```

Trait layer across postgres, sqlite, kv, and fetch. Once written, stable — APIs don't change often on either side. **This abstraction would improve the codebase even without edge support** — makes stdlib more testable and modular.

**3. Build tooling (~1000-2000 lines):**
`ntnt build --target cloudflare` — bundles .tnt files, generates consumer Workers for Job definitions, writes wrangler.toml. Leaf module: reads .tnt files, generates output. Doesn't touch the core.

### Ongoing Maintenance Cost

| Concern | Impact |
|---|---|
| Initial build | 4-8 weeks of focused work |
| New stdlib I/O functions | +15% effort per function (add edge impl, ~20-30 lines each) |
| New non-I/O stdlib functions | Zero — implement once, works on both targets |
| CI/testing | +1 build target, ~3min added to CI |
| Codebase size increase | +2000-3000 lines (~3% of total) |
| Cognitive complexity | Low — clean separation via compile-time feature gates |
| Risk of breaking server path | Minimal — feature gates are compile-time, not runtime branches |
| CF API changes | Rare, usually additive. Update edge binding only, server path untouched |

### Timing Consideration

The honest risk isn't complexity — it's **premature commitment**. Building this before the stdlib surface is stable means maintaining two backends through every breaking change. Building it after v1.0 when stdlib APIs are locked makes the abstraction layer write-once-maintain-forever.

**Recommended approach:**
- **Now:** Feature-gate runtime modules (threading, TCP, fs) with `#[cfg]` flags. Zero cost, makes edge work easier later.
- **Post-v1.0:** Build the actual edge implementation when stdlib is stable and "where can I deploy?" becomes a real user question.

## Size Budget Analysis

| Component | Native (est.) | WASM stripped (est.) |
|---|---|---|
| Core interpreter (parser + eval) | ~8MB | ~3-4MB |
| Template engine | ~1MB | ~0.5MB |
| Stdlib (string/json/collections/crypto/time) | ~2MB | ~1MB |
| HTTP request/response | ~1MB | ~0.5MB |
| CF bindings (wasm-bindgen glue) | — | ~0.2MB |
| **Total (pre-compression)** | — | **~5-6MB** |
| **Compressed (gzip)** | — | **~3-4MB** |

Workers limit: 10MB compressed (paid). **Should fit with room to spare.**

## Risks

1. **WASM binary size** — If stripping doesn't get us under 10MB, this is blocked until CF raises limits or we find more to cut
2. **Cold start latency** — Initializing a WASM interpreter on every request could be 50-200ms. Memory snapshots are critical.
3. **JS↔WASM bridge overhead** — Every CF API call crosses the WASM/JS boundary. For DB-heavy apps, this could add measurable latency.
4. **Maintenance burden** — Two runtime backends (native + edge) means testing both, keeping them in sync, and debugging platform-specific issues.
5. **CF API evolution** — Workers APIs change; bindings need maintenance.

## Decision

**Not yet.** This is exploratory. The right time to build this is when:
- ntnt has enough users that "where can I deploy?" is a real question
- The core language is stable enough that maintaining two backends isn't a moving target
- We've shipped the features that matter more first (SSE, auth excellence, etc.)

But the architecture should be kept in mind — feature-gating the runtime modules now (even without WASM) makes this easier later.

---

## Changelog

| Date | Change |
|---|---|
| 2026-03-28 | Initial design doc — exploratory research, phased plan, edge job system, independence & complexity analysis |
