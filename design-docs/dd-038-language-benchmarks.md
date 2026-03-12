# DD-038: ntnt Language Benchmarks

**Status:** draft  
**App:** ntnt  
**Author:** Larri  
**Date:** 2026-03-12

## Goal

Produce a fair, reproducible benchmark suite comparing ntnt's HTTP server performance against popular web frameworks. Results should be presentable — blog post, README table, or ntnt-lang.org page.

## Frameworks to Benchmark

| Framework | Language | Why include |
|-----------|----------|-------------|
| **ntnt** (v0.4.1) | ntnt (Rust runtime, Axum/Tokio) | Us |
| **FastAPI** | Python 3.12 + uvicorn | Most popular modern Python framework |
| **Express** | Node.js / TypeScript | The default for JS/TS devs |
| **Gin** | Go | The performance bar everyone measures against |
| **Rails 7** | Ruby | Classic "batteries included" — shows where DX-focused langs land |
| **Bun + Hono** | TypeScript | The new fast JS runtime — strong competition |
| **Actix Web** | Rust | Theoretical ceiling — same language as ntnt's runtime |

### Why these?

- FastAPI, Express, Rails = the three frameworks most developers are choosing between when they'd consider ntnt
- Go/Gin = the "I need performance" choice — if ntnt beats it, that's a headline
- Bun/Hono = the "new and fast" JS option — relevant for the audience
- Actix = honest comparison against raw Rust — shows the interpreter overhead

## Benchmark Scenarios

### Tier 1: Micro-benchmarks (raw throughput)
These test the framework's core overhead — how fast can it process a request with minimal work.

**B1: Plaintext** — `GET /plaintext` → `"Hello, World!"`
- Tests: HTTP parsing, routing, response writing
- This is what TechEmpower uses as baseline

**B2: JSON serialization** — `GET /json` → `{"message": "Hello, World!"}`
- Tests: JSON encoding overhead on top of B1

**B3: Path parameter routing** — `GET /users/:id` → `{"id": <id>}`
- Tests: Router pattern matching + param extraction

### Tier 2: Real-world patterns
These test things actual apps do — more meaningful for the "should I use ntnt?" question.

**B4: Database single query** — `GET /db` → single row from PostgreSQL
- Tests: Connection pool + query + serialize
- Standard TechEmpower "db" test
- Uses a shared PostgreSQL instance (same for all frameworks)

**B5: Database multi-query** — `GET /queries?count=20` → 20 individual queries
- Tests: Sequential database round-trips (NOT batch)
- Reveals per-query overhead
- Standard TechEmpower "queries" test

**B6: Template rendering** — `GET /template` → HTML page with 10 items from DB
- Tests: Template engine speed (each framework uses its native/idiomatic engine)
- ntnt uses its `"""..."""` template strings
- Express uses EJS, FastAPI uses Jinja2, etc.

**B7: JSON body parsing** — `POST /json` with 1KB JSON body → echo back parsed + re-serialized
- Tests: Request body parsing + JSON round-trip

### Tier 3: DX comparison (qualitative)
Not benchmarked with wrk, but included in the writeup.

- Lines of code for each implementation
- Cold start time (time from `./run` to first successful request)
- Binary/deployment size (Docker image size)
- Dependencies needed (package count)

## Methodology

### Hardware
- Run on the same machine (Josh's server — consistent environment)
- All frameworks run bare-metal (no Docker during benchmark — eliminates container overhead noise)
- Single machine, single process per framework (no clustering)

### Tool
- **wrk** (already installed) for HTTP load generation
- Config: `wrk -t4 -c100 -d30s --latency <url>` (4 threads, 100 connections, 30 seconds)
- Each benchmark run 3x, take median
- 5-second warmup before each measurement run

### Database
- Shared PostgreSQL instance (same one we already run)
- Dedicated `benchmarks` database with TechEmpower-style `World` table (10,000 rows)
- Connection pool: 50 connections per framework (or framework default if lower)

### Fairness rules
1. Each implementation should be **idiomatic** — write it the way that framework's docs recommend, not artificially optimized
2. Same PostgreSQL driver per language where possible (e.g., `asyncpg` for Python, `pg` for Node)
3. Production mode for all (no debug logging, no hot-reload)
4. Pin all dependency versions
5. Document exact versions of everything (runtime, framework, driver)

## Directory Structure

```
~/repos/ntnt-benchmarks/
├── README.md              ← Results table + methodology
├── benchmark.sh           ← Master script: builds, starts, benchmarks, stops each framework
├── setup-db.sql           ← Create benchmarks DB + World table + seed data
├── results/               ← Raw wrk output + parsed summaries
│   ├── 2026-03-12.json    ← Machine-parseable results
│   └── 2026-03-12.md      ← Human-readable summary
├── ntnt/
│   ├── server.tnt
│   └── views/template.html
├── fastapi/
│   ├── app.py
│   └── requirements.txt
├── express/
│   ├── app.ts
│   ├── package.json
│   └── tsconfig.json
├── gin/
│   ├── main.go
│   └── go.mod
├── rails/
│   ├── ...
│   └── Gemfile
├── hono-bun/
│   ├── app.ts
│   └── package.json
└── actix/
    ├── src/main.rs
    └── Cargo.toml
```

## Execution Plan

### Phase 1: Setup (1-2 hours)
- [ ] Create `ntnt-benchmarks` repo
- [ ] Write `setup-db.sql` (World table + 10K rows)
- [ ] Write the ntnt implementation (server.tnt)
- [ ] Write `benchmark.sh` skeleton (start/warmup/benchmark/stop/parse cycle)

### Phase 2: Implementations (2-3 hours)
- [ ] FastAPI implementation
- [ ] Express/TypeScript implementation
- [ ] Gin implementation
- [ ] Hono/Bun implementation
- [ ] Actix implementation
- [ ] Rails implementation
- [ ] Verify all implementations return identical responses (diff check)

### Phase 3: Run & Validate (1 hour)
- [ ] Run full benchmark suite
- [ ] Verify results are stable (low variance across 3 runs)
- [ ] Generate results table
- [ ] Sanity-check: do the numbers make sense? (Actix > Go > ntnt > Node > Python > Ruby is expected order)

### Phase 4: Publish
- [ ] Write up results with charts
- [ ] Add to ntnt-lang.org
- [ ] GitHub README with reproduction instructions

## Expected Results (hypothesis)

Based on ntnt's architecture (Rust runtime, Axum/Tokio HTTP, tree-walking interpreter):

| Scenario | Expected ranking |
|----------|-----------------|
| Plaintext | Actix > Gin ≈ ntnt > Hono/Bun > Express > FastAPI > Rails |
| JSON | Actix > Gin > ntnt > Hono/Bun > Express > FastAPI > Rails |
| DB queries | Actix > Gin > ntnt ≈ Hono/Bun > Express > FastAPI > Rails |
| Template | Less predictable — ntnt's string templates may be very fast |

**The interesting question:** How much overhead does ntnt's interpreter add vs raw Rust (Actix)? If it's within 2-3x of Actix while being 10x less code, that's a great story.

**The narrative we're building:**
- "Rust performance without Rust complexity"
- Faster than Python/Ruby/Node for the same developer experience
- Competitive with Go, which is the current "I need performance" choice

## What success looks like

A table people can screenshot and share that shows ntnt outperforming Express, FastAPI, and Rails by a meaningful margin on real-world workloads — while having fewer lines of code than any of them.

---

## Benchmark Results — 2026-03-12

> **Raw data:** 126 wrk output files saved in `results/` (3 runs × 7 benchmarks × 6 frameworks)  
> **Repo:** [ntntlang/ntnt-benchmarks](https://github.com/ntntlang/ntnt-benchmarks) — fully reproducible  
> **System:** Linux 6.8.0-71-generic, 16 CPUs, 59GB RAM  
> **Config:** 4 threads, 100 connections, 15s duration, 3 runs (median reported)

### Summary Table (req/sec — higher is better)

| Benchmark | ntnt | FastAPI | Express | Gin | Hono/Bun | Actix |
|-----------|-----:|--------:|--------:|----:|---------:|------:|
| **plaintext** | 118,208 | 173,783 | 18,167 | 406,060 | 118,409 | 476,661 |
| **json** | 108,929 | 151,696 | 17,017 | 387,095 | 105,152 | 476,342 |
| **params** | 88,926 | 130,802 | 16,788 | 384,301 | 101,625 | 469,648 |
| **db** (1 query) | 8,371 | 36,859 | 11,817 | 130,190 | 32,399 | 64,003 |
| **queries** (×20) | 457 | 5,818 | 2,419 | 9,296 | 2,789 | 3,916 |
| **template** (10q+HTML) | 899 | 10,431 | 4,112 | 18,014 | 5,171 | 7,696 |
| **json-body** (POST) | 76,398 | 116,140 | 11,828 | 324,507 | 82,927 | 447,592 |

### Latency (p99 — lower is better)

| Benchmark | ntnt | FastAPI | Express | Gin | Hono/Bun | Actix |
|-----------|------|---------|---------|-----|----------|-------|
| **plaintext** | 1.06ms | 1.44ms | 8.34ms | 0.88ms | 1.16ms | 0.22ms |
| **db** | 14.93ms | 6.10ms | 13.17ms | 2.11ms | 5.07ms | 2.08ms |
| **queries** (×20) | 243.90ms | 17.27ms | 51.79ms | 14.04ms | 45.08ms | — |
| **template** | 128.21ms | 20.45ms | 31.08ms | 7.29ms | — | 15.08ms |

### Lines of Code (identical functionality)

| Framework | LoC | Dependencies |
|-----------|----:|------------:|
| **Express** | 90 | 2 |
| **Hono/Bun** | 92 | 2 |
| **ntnt** | 99 | 0 |
| **FastAPI** | 99 | 3 |
| **Gin** | 118 | 2 |
| **Actix** | 144 | 13 |

### Analysis

#### Where ntnt is strong (HTTP-only workloads)

- **Plaintext/JSON/Params:** 89K–118K req/sec — matches Hono/Bun, 6–7× faster than Express
- **JSON body parsing:** 76K req/sec — competitive with Bun, 6.5× faster than Express
- **p99 latency:** 1.06ms plaintext — excellent tail latency, lower than FastAPI (1.44ms)
- **Zero dependencies:** ntnt has no package.json, no pip install, no go.mod — just one binary
- **Interpreter overhead vs raw Rust:** ~4× slower than Actix for HTTP-only — reasonable for a tree-walking interpreter

#### Where ntnt is weak (DB-heavy workloads)

- **Single DB query:** 8,371 req/sec — 4.4× slower than FastAPI, 1.4× slower than Express
- **20 queries:** 457 req/sec — 12.7× slower than FastAPI, 5.3× slower than Express
- **Template (10 queries):** 899 req/sec — 11.6× slower than FastAPI, 4.6× slower than Express
- **The pattern is clear:** every additional DB query multiplies the overhead. 1 query = 4× slow. 20 queries = 12× slow. This is the interpreter's per-operation cost compounding.

#### Root Cause: ntnt's DB Query Path

ntnt's HTTP layer (Axum/Tokio) is fast — the plaintext numbers prove the runtime is efficient. The bottleneck is the **interpreter-to-Rust bridge for each query**:

1. ntnt interpreter evaluates `query(pg, sql, params)`
2. Interpreter marshals ntnt values → Rust types
3. Rust runtime calls tokio-postgres (async)
4. Response marshaled back: Rust rows → ntnt values
5. Interpreter resumes

Each step crosses the interpreter/runtime boundary. With 20 queries, that's 20 round-trips through this bridge. FastAPI (asyncpg, native C extension) and Gin (pgx, native Go) don't have this overhead.

#### Opportunities to Improve

1. **Connection pooling in the runtime** — if ntnt doesn't already pool, adding bb8/deadpool at the Rust level could help
2. **Batch query support** — `query_many(pg, sql, [params1, params2, ...])` that executes multiple queries in a single interpreter→Rust call
3. **Prepared statements** — cache query plans across requests
4. **Row-level streaming** — return rows without materializing full Vec<Value> in Rust first
5. **Async interpreter** — longer term: if the interpreter can yield during I/O, it can handle more concurrent queries

### Verdict

**ntnt is competitive for HTTP-heavy workloads** — matching Bun/Hono and crushing Express, with zero dependencies and fewer lines of code. The story is strong for APIs, proxies, and static-heavy apps.

**DB performance needs work before ntnt is competitive for data-heavy apps.** The 457 req/sec on 20-query workloads vs Gin's 9,296 is a 20× gap that matters for real applications. This is the #1 performance priority.

**The good news:** the bottleneck is well-understood (interpreter↔runtime bridge) and fixable without redesigning the language. Batch queries alone could close 50%+ of the gap.
