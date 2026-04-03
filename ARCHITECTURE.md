# NTNT Language Architecture

## Overview

NTNT is an agent-native programming language designed for AI-driven web development. It combines a tree-walking interpreter written in Rust with runtime contracts, an intent-driven testing framework, a language-native job system, and a full standard library for building web applications. ~93,000 lines of Rust, ~1,375 tests.

This document describes the current implementation architecture as of v0.4.7.

## Source Structure

```
src/
├── main.rs                  # CLI entry point and command routing
├── lib.rs                   # Library exports
├── lexer.rs                 # Tokenizer (1,710 lines)
├── parser.rs                # Recursive descent parser → AST (3,223 lines)
├── ast.rs                   # Abstract syntax tree definitions
├── interpreter.rs           # Tree-walking evaluator (14,901 lines)
├── contracts.rs             # Contract checking, old() value capture
├── typechecker.rs           # Static type checker — gradual typing, strict lint (7,041 lines)
├── types.rs                 # Type definitions and compatibility
├── config.rs                # Runtime config (TypeMode, LintMode from env vars)
├── error.rs                 # Error types and formatting
├── intent.rs                # Intent-Driven Development module
├── intent_studio_server.rs  # Async Axum server for Intent Studio UI
├── control_socket.rs        # Unix domain socket for live worker management
│
├── ial/                     # Intent Assertion Language engine
│   ├── mod.rs               # Public API: run_assertions(), run_scenario()
│   ├── vocabulary.rs        # Pattern matching and term storage
│   ├── resolve.rs           # Recursive term rewriting (Term → Primitives)
│   ├── execute.rs           # Primitive execution against Context
│   ├── primitives.rs        # Primitive types (Http, Cli, Check) + CheckOp enum
│   └── standard.rs          # Standard vocabulary definitions + glossary parsing
│
└── stdlib/                  # Standard library modules (21 modules)
    ├── mod.rs               # Module registry (auto-discovered by build.rs)
    ├── string.rs            # std/string — String manipulation (35+ functions)
    ├── math.rs              # std/math — Math functions and constants
    ├── collections.rs       # std/collections — Array/map utilities
    ├── env.rs               # std/env — Environment variables
    ├── fs.rs                # std/fs — File system operations
    ├── path.rs              # std/path — Path manipulation
    ├── json.rs              # std/json — JSON parse/stringify
    ├── csv.rs               # std/csv — CSV parse/stringify with headers
    ├── time.rs              # std/time — Time with IANA timezone support
    ├── crypto.rs            # std/crypto — SHA256, HMAC, bcrypt, AES-GCM, UUID
    ├── url.rs               # std/url — URL encoding/parsing
    ├── markdown.rs          # std/markdown — Markdown → HTML (pulldown-cmark)
    ├── log.rs               # std/log — Structured logging (JSON, levels, stderr)
    ├── http.rs              # std/http — HTTP client (fetch, download)
    ├── http_server.rs       # std/http/server — Response builders
    ├── http_server_async.rs # Async HTTP server (Axum + Tokio)
    ├── http_bridge.rs       # Bridge between async server and sync interpreter
    ├── auth.rs              # std/auth — OAuth 2.0, OIDC, PKCE, TOTP, JWT (7,211 lines)
    ├── template.rs          # External template loading (Mustache-style)
    ├── postgres.rs          # std/db/postgres — PostgreSQL (deadpool connection pool)
    ├── sqlite.rs            # std/db/sqlite — SQLite (bundled via rusqlite)
    ├── kv.rs                # std/kv — Key-value store (SQLite + Redis backends)
    ├── concurrent.rs        # std/concurrent — Spawn, channels, select, scheduling
    └── jobs.rs              # std/jobs — Background job system (10,791 lines)
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `ntnt run <file>` | Execute an NTNT file (hot-reload enabled by default) |
| `ntnt repl` | Interactive REPL with `:doc` and `:type` commands |
| `ntnt lint <file>` | Comprehensive linting (style, types, unused vars) |
| `ntnt check <file>` | Quick syntax check |
| `ntnt test <file>` | Test HTTP endpoints |
| `ntnt parse <file>` | Show AST |
| `ntnt inspect <file>` | JSON project structure (for agents and tools) |
| `ntnt validate <file>` | Syntax validation with JSON output |
| `ntnt intent check <file>` | Verify implementation against intent spec |
| `ntnt intent coverage <file>` | Show feature implementation coverage |
| `ntnt intent init <file>` | Generate code scaffolding from intent |
| `ntnt intent studio <file>` | Visual preview with live tests (Axum server) |
| `ntnt docs [query]` | Search stdlib documentation |
| `ntnt docs --generate` | Generate reference docs + sync agent files |
| `ntnt docs --validate` | Validate documentation completeness |
| `ntnt learn <platform>` | Generate agent config (claude-code, cursor, codex, copilot) |
| `ntnt migrate <path>` | Migrate `{expr}` → `#{expr}` interpolation |
| `ntnt worker <file>` | Start background job workers |
| `ntnt jobs [status\|list\|retry\|cancel]` | Manage job queue |
| `ntnt workers [status\|scale]` | Live worker management via control socket |
| `ntnt completions <shell>` | Generate shell completions |

## Core Components

### Lexer (`lexer.rs`)

Tokenizes NTNT source code including:
- Keywords, identifiers, literals
- String interpolation (`"Hello, #{name}!"` — Ruby-style `#{expr}` sigil)
- Raw strings (`r"..."`, `r#"..."#`)
- Triple-quoted template strings (`"""..."""` with `{{expr}}` for HTML templates)
- Range operators (`..`, `..=`)
- Pipe operator (`|>`)
- Null-coalescing (`??`)
- Contract keywords (`requires`, `ensures`, `invariant`)

### Parser (`parser.rs`)

Recursive descent parser producing an AST:
- Expressions with operator precedence
- Statements (let, if, for, match, defer)
- Functions with contracts and generic type parameters
- Structs, traits, and enums (Option, Result built-in)
- Imports and exports
- Job definitions (`Job Name on queue { ... }`)
- Batch definitions (`Batch Name { ... }`)

### Interpreter (`interpreter.rs`)

Tree-walking evaluator (~14,900 lines) with:
- Lexical scoping with closures
- Contract enforcement at runtime (requires, ensures, invariant)
- Trait method dispatch with default methods
- Generic function instantiation (`identity<T>(42)`)
- Defer stack (LIFO cleanup)
- 60+ built-in functions
- HTTP server integration via bridge pattern
- Type safety modes (strict / warn / forgiving via `NTNT_TYPE_MODE`)

### Type Checker (`typechecker.rs`)

Static analysis pass (~7,000 lines):
- Gradual typing — runs before interpretation
- Real generics with type parameter unification
- `NTNT_LINT_MODE` controls lint-time behavior (warn-untyped, strict)
- Result/Option auto-unwrap detection with warnings

### Contract System (`contracts.rs`)

Runtime contract checking:
- `requires` — Preconditions checked on function entry
- `ensures` — Postconditions checked on function exit
- `old()` — Captures values at function entry for postconditions
- `invariant` — Struct invariants checked after mutations

### Configuration (`config.rs`)

Runtime modes from environment variables, cached via `OnceLock`:
- `NTNT_TYPE_MODE` — `strict` | `warn` (default) | `forgiving`
- `NTNT_LINT_MODE` — Controls lint strictness for missing annotations
- `NTNT_ENV` — `production` disables hot-reload
- `NTNT_TIMEOUT` — Request timeout in seconds (default: 30)

## HTTP Server Architecture

The HTTP server uses a bridge pattern to connect async Axum handlers to the synchronous interpreter:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Tokio Async Runtime                         │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐                        │
│  │ Task 1  │  │ Task 2  │  │ Task N  │  (Axum async handlers) │
│  └────┬────┘  └────┬────┘  └────┬────┘                        │
│       └────────────┼────────────┘                              │
│                    │                                           │
│              ┌─────▼─────┐                                     │
│              │  Channel   │  (mpsc + oneshot reply)            │
│              └─────┬─────┘                                     │
└────────────────────┼───────────────────────────────────────────┘
                     │
┌────────────────────▼───────────────────────────────────────────┐
│                  Interpreter Thread                             │
│  - Receives requests via channel                               │
│  - Finds and calls NTNT handler function                       │
│  - Sends response back via oneshot channel                     │
│  - Uses Rc<RefCell<>> (not thread-safe, hence single thread)   │
└────────────────────────────────────────────────────────────────┘
```

**Key files:**
- `http_server_async.rs` — Axum server setup, routes, middleware, static files, CORS, CSP
- `http_bridge.rs` — Request/response types, channel communication
- `http_server.rs` — Response builders (`json()`, `html()`, `redirect()`, `set_cookie()`, etc.)

**Server features:**
- Hot-reload (enabled by default, disabled with `NTNT_ENV=production`)
- `enable_cors()` / `enable_csp()` — Security middleware
- `enable_auth()` — Full OAuth 2.0 / OIDC integration
- `use_middleware()` — Custom middleware chain
- `on_shutdown()` — Graceful shutdown hooks
- `serve_static()` — Static file serving with compression
- HEAD request support (RFC 9110 §9.3.2)
- Connection pooling via deadpool-postgres

## Concurrency Model

NTNT uses a task-and-channel model (no async/await syntax):

### Tasks
- `spawn(fn)` → `TaskHandle(id)` — runs function on a thread
- `await_task(handle)` — blocks until task completes, returns result
- `try_await(handle)` → `{status, value}` — non-blocking check (status: `"ok"`, `"pending"`, `"consumed"`, `"expired"`)
- `after(delay_ms, fn)` → `TaskHandle` — delayed one-shot execution
- `schedule(interval_ms, fn)` → `ScheduleHandle` — repeating timer

### Channels (Two-Handle Design)
Modeled after Rust's `mpsc` — `TxChannel` and `RxChannel` are separate value types:

```
channel() → [TxChannel(id), RxChannel(id)]

send(tx, value)              — send a value (blocks if needed)
recv(rx)                     — blocking receive
try_recv(rx)                 — non-blocking, returns Some/None
recv_timeout(rx, ms)         — receive with timeout
select([rx_a, rx_b], ms?)    — wait on multiple channels (crossbeam)
close(rx)                    — close receiver
```

When a `TxChannel` is dropped (goes out of scope), the `RxChannel` sees disconnected and `recv()` returns `Unit` — no sentinel injection needed.

## Job System (`std/jobs`)

Language-native background job processing (~10,800 lines):

### Job DSL
```ntnt
Job SendEmail on "email" {
    perform(args) {
        // job implementation
    }
}

Batch DailyReport {
    perform(args) {
        // batch implementation  
    }
}
```

### Architecture
- **Priority queues** — 0-99 (named: critical=5, high=25, normal=50, low=85)
- **Worker bands** — Independent thread pools per priority range
- **KV-backed** — Jobs stored in `std/kv` (SQLite or Redis)
- **Atomic dedup** — Content-hash deduplication with configurable TTL
- **Batch system** — Dynamic adds, `batch_id()` context, TTL expiry, completion callbacks
- **Control socket** — Unix domain socket (`.ntnt.sock`) for live management

### CLI
```bash
ntnt worker app.tnt                    # Start workers
ntnt jobs status                       # Queue stats
ntnt jobs list --status pending        # List jobs by status
ntnt workers status                    # Live worker status
ntnt workers scale --band low --count 8  # Dynamic scaling
```

### Control Socket Protocol
Newline-delimited JSON over Unix socket:
```bash
echo '{"cmd":"status"}' | socat - UNIX-CONNECT:.ntnt.sock
echo '{"cmd":"scale","band":"low","count":8}' | socat - UNIX-CONNECT:.ntnt.sock
```

## Intent Assertion Language (IAL)

IAL is a term rewriting system that translates natural language assertions into executable tests.

```
┌─────────────────────────────────────────────────────────────────┐
│                         IAL ENGINE                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│   ┌─────────────┐                                               │
│   │ VOCABULARY  │  ← Standard terms + glossary + components     │
│   │ term → def  │                                               │
│   └──────┬──────┘                                               │
│          │                                                      │
│          ▼                                                      │
│   ┌─────────────┐                                               │
│   │  RESOLVE    │  ← Recursive term rewriting                   │
│   │             │    (term → expanded terms → primitives)       │
│   └──────┬──────┘                                               │
│          │                                                      │
│          ▼                                                      │
│   ┌─────────────┐     ┌─────────────┐                          │
│   │ PRIMITIVES  │ ──▶ │  EXECUTE    │ ──▶ Pass/Fail            │
│   └─────────────┘     └─────────────┘                          │
│                                                                 │
│   Primitives:                                                   │
│   - Http(method, path, body?)     - FunctionCall(name, args)   │
│   - Cli(command)                  - PropertyCheck(fn, type)    │
│   - CodeQuality(path)             - Check(op, path, expected)  │
│   - ReadFile(path)                                              │
│                                                                 │
│   Check Operations:                                             │
│   Equals, NotEquals, Contains, NotContains, Matches,           │
│   Exists, NotExists, LessThan, GreaterThan, InRange,           │
│   StartsWith, EndsWith, IsType, HasLength                      │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

**Key design principle:** The engine is fixed; all new assertions are vocabulary entries.

**Intent Studio** (`intent_studio_server.rs`) provides a browser-based UI for writing and running intents with live test feedback via an Axum server.

## Auth System (`std/auth`)

Full OAuth 2.0 and OIDC implementation (~7,200 lines):

- **Flows:** Authorization Code, Authorization Code + PKCE, Client Credentials, Refresh Token
- **Providers:** Google, GitHub, or any custom OIDC provider (auto-discovery from issuer URL)
- **Features:** ID token validation, nonce replay protection, JWT encode/decode, TOTP (2FA), bcrypt/argon2 password hashing, AES-GCM encryption, constant-time comparison

```ntnt
import { oauth, enable_auth, get_user } from "std/auth"

enable_auth(oauth("google", get_env("GOOGLE_CLIENT_ID"), get_env("GOOGLE_CLIENT_SECRET")))

fn dashboard(req) {
    let user = get_user(req) otherwise return redirect("/login")
    return html("<h1>Hello, #{user.name}!</h1>")
}
```

## Documentation System

Documentation is auto-generated from multiple sources:

```
src/stdlib/*.rs          → STDLIB_REFERENCE.md   (functions, via // @ntnt doc blocks)
src/interpreter.rs       → STDLIB_REFERENCE.md   (builtins, via // @ntnt doc blocks)
docs/syntax.toml         → SYNTAX_REFERENCE.md   (keywords, operators)
docs/ial.toml            → IAL_REFERENCE.md      (IAL primitives, terms)
docs/runtime.toml        → RUNTIME_REFERENCE.md  (CLI, environment, server)
docs/AI_AGENT_GUIDE.md   → CLAUDE.md, CODEX.md, .github/copilot-instructions.md
```

### Stdlib doc pipeline (`build.rs`)

`build.rs` auto-discovers all `src/stdlib/*.rs` files and scans `// @ntnt` comment blocks. It validates that every `NativeFunction` has a doc block and every doc block has a matching function — orphaned or undocumented functions **fail the build**. The result is embedded as `doc_data.json` in the binary via `include_str!()`.

```
build.rs scan → doc_data.json → binary (OnceLock) → REPL :doc / CLI ntnt docs / markdown
```

### Agent file sync

`ntnt docs --generate` syncs the coding guide from `docs/AI_AGENT_GUIDE.md` into `CLAUDE.md`, `CODEX.md`, and `.github/copilot-instructions.md` using `<!-- BEGIN/END NTNT CODING GUIDE -->` markers, rewriting doc links for each target's relative path.

`ntnt learn <platform>` generates complete agent configuration for Claude Code, Cursor, Codex, and Copilot.

CI validates that generated docs are up-to-date (build fails on drift).

## Standard Library

| Module | Description |
|--------|-------------|
| `std/string` | 35+ string functions (split, join, trim, replace, regex, chars) |
| `std/math` | Trig, log, exp, random, constants (PI, E) |
| `std/collections` | Array/map utilities (push, pop, keys, values, map, filter, reduce, sort, find, any, all) |
| `std/env` | Environment variables (get_env, load_env) |
| `std/fs` | File operations (read_file, write_file, append_file, mkdir, readdir, copy) |
| `std/path` | Path manipulation (join, dirname, basename, extension) |
| `std/json` | JSON parse/stringify |
| `std/csv` | CSV parse/stringify with headers |
| `std/time` | Time with IANA timezone support, is_after/is_before, add_days/add_months |
| `std/crypto` | SHA256, HMAC, bcrypt, argon2, AES-GCM, UUID, random bytes |
| `std/url` | URL encoding, query string parsing |
| `std/markdown` | Markdown → HTML (to_html, to_html_safe for untrusted input) |
| `std/log` | Structured logging with levels (debug/info/warn/error), JSON context |
| `std/http` | HTTP client (fetch with 1-arg and 2-arg forms, download) |
| `std/http/server` | Response builders (json, html, redirect, set_cookie) |
| `std/auth` | OAuth 2.0, OIDC, PKCE, JWT, TOTP, password hashing |
| `std/db/postgres` | PostgreSQL with connection pooling (deadpool), transactions |
| `std/db/sqlite` | SQLite (bundled, zero external deps) with transactions |
| `std/kv` | Unified key-value store (SQLite and Redis/Valkey backends) |
| `std/concurrent` | spawn, channels (Tx/Rx), select, schedule, after, sleep |
| `std/jobs` | Background job system (priority queues, batches, workers, dedup) |

## Built-in Functions

Available without import:

| Category | Functions |
|----------|-----------|
| Type conversion | `str`, `int`, `float`, `type` |
| Math | `abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `trunc`, `sign`, `clamp` |
| Collections | `len`, `push`, `sort`, `reverse`, `keys`, `values`, `entries`, `has_key`, `get_key`, `filter`, `reduce`, `find`, `any`, `all`, `count` |
| I/O | `print`, `assert` |
| Option/Result | `Some`, `None`, `Ok`, `Err`, `unwrap`, `unwrap_or`, `is_some`, `is_none`, `is_ok`, `is_err` |
| HTTP Server | `get`, `post`, `put`, `patch`, `delete`, `head`, `options`, `listen`, `serve_static`, `routes`, `template`, `use_middleware`, `enable_cors`, `enable_csp`, `enable_auth`, `on_shutdown` |

## System Layers

```
┌─────────────────────────────────────────────────┐
│  Intent-Driven Development                      │
│  (.intent files, IAL assertions, Studio UI)     │
├─────────────────────────────────────────────────┤
│  Job System                                     │
│  Job DSL, priority queues, batches, workers     │
├─────────────────────────────────────────────────┤
│  HTTP Server (Axum + Tokio)                     │
│  Routes, middleware, OAuth/OIDC, static files   │
├─────────────────────────────────────────────────┤
│  Concurrency                                    │
│  spawn, Tx/Rx channels, select, scheduling      │
├─────────────────────────────────────────────────┤
│  Language Core                                  │
│  Lexer → Parser → AST → Type Checker →          │
│  Interpreter (contracts, generics, closures)    │
├─────────────────────────────────────────────────┤
│  Standard Library (21 modules)                  │
│  String, math, fs, http, postgres, sqlite,      │
│  kv, auth, crypto, jobs, concurrent, etc.       │
├─────────────────────────────────────────────────┤
│  Tooling                                        │
│  CLI, REPL, VS Code extension, doc generation,  │
│  agent config (learn), control socket           │
└─────────────────────────────────────────────────┘
```

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| `axum` + `tokio` + `tower` | Async HTTP server and middleware |
| `deadpool-postgres` + `tokio-postgres` | PostgreSQL connection pooling |
| `rusqlite` (bundled) | SQLite with zero external deps |
| `redis` | Redis/Valkey client for KV and jobs |
| `reqwest` | HTTP client (blocking mode) |
| `crossbeam-channel` | Multi-channel `select()` |
| `jsonwebtoken` | JWT encode/decode for auth |
| `pulldown-cmark` | Markdown → HTML |
| `clap` | CLI argument parsing |
| `chrono` + `chrono-tz` | Time and timezone support |

## Testing

- **~1,375 tests** across 5 integration test files and inline unit tests
- Integration tests: `tests/language_features_tests.rs`, `tests/concurrency_tests.rs`, `tests/cli_tests.rs`, `tests/intent_studio_tests.rs`, `tests/type_checker_tests.rs`
- CI uses `dorny/paths-filter` to skip heavy test jobs for docs-only changes
- Redis integration tests gated behind `--features redis-tests`

## Key Design Decisions

- **Tree-walking interpreter** — Simple, debuggable execution model. No bytecode (yet — on roadmap).
- **Single interpreter thread** — Uses `Rc<RefCell<>>` (not `Arc<Mutex<>>`). Thread safety via the bridge pattern: async handlers queue work for the single interpreter thread.
- **Two-handle channels** — `TxChannel`/`RxChannel` as separate value types (like Rust's `mpsc`). When Tx drops, Rx disconnects automatically. No sentinel injection.
- **KV-backed jobs** — Jobs use `std/kv` rather than a dedicated queue system, making them work with either SQLite (zero-dep) or Redis (production scale).
- **Build-enforced docs** — `build.rs` fails if any stdlib function lacks documentation. Docs are part of the binary, not external files.
- **Gradual typing** — Type checker runs before interpretation as a lint pass. Configurable strictness avoids the "all or nothing" problem.

---

_See [ROADMAP.md](ROADMAP.md) for implementation phases and planned features._
_See [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) for the comprehensive language and syntax reference._
