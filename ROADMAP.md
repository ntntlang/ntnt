# NTNT Language Implementation Roadmap

This document outlines the implementation plan for NTNT, a programming language designed for AI-driven development. The roadmap prioritizes getting to a working web application quickly while focusing on NTNT's unique differentiators: contracts, AI integration, and intent encoding.

---

## Design Principles

1. **Self-Contained**: NTNT has no runtime dependencies on other languages. The interpreter/compiler is written in Rust, but NTNT programs are pure Intent.

2. **AI-First**: Features that enable AI development (contracts, intent annotations, structured edits) are core, not afterthoughts.

3. **Production-Ready Web Apps**: The goal is building real web applications with safety guarantees.

4. **Lean Standard Library**: Include essentials, leave specialized libraries to the community.

---

## Current Status

### Completed ✅

- [x] Lexer with full token support
- [x] Recursive descent parser
- [x] Complete AST definitions
- [x] Tree-walking interpreter
- [x] Basic type system (Int, Float, String, Bool, Array, Object, Function, Unit)
- [x] Full contract system (`requires`, `ensures`, `old()`, `result`)
- [x] Struct invariants with automatic checking
- [x] Built-in math functions (`abs`, `min`, `max`, `sqrt`, `pow`, etc.)
- [x] CLI with REPL, run, parse, lex, check commands
- [x] VS Code extension with syntax highlighting
- [x] 140 unit tests passing
- [x] File extension: `.tnt`
- [x] Algebraic Data Types with enums
- [x] Option<T> and Result<T, E> built-ins
- [x] Pattern matching with match expressions
- [x] Generic functions and types with trait bounds
- [x] Type aliases
- [x] Union types
- [x] Effect annotations foundation
- [x] Module system with imports/exports
- [x] Standard library: std/string, std/math, std/collections, std/env, std/fs, std/path, std/json, std/time, std/crypto, std/url, std/http
- [x] Traits with default implementations
- [x] For-in loops and ranges
- [x] Defer statement
- [x] Map literals with field access (dot notation)
- [x] String interpolation and raw strings

---

## Phase 1: Core Contract System ✅ COMPLETE

**Status:** Complete

### 1.1 Runtime Contract Enforcement ✅

- [x] Precondition (`requires`) evaluation before function execution
- [x] Postcondition (`ensures`) evaluation after function execution
- [x] Access to `old()` values in postconditions
- [x] Access to `result` in postconditions
- [x] Contract violation error handling with clear messages

### 1.2 Class/Struct Invariants ✅

- [x] `invariant` clause support in impl blocks
- [x] Invariant checking on struct construction
- [x] Invariant checking after method calls
- [x] Invariant checking after field assignments
- [x] `self` keyword access in invariant expressions

---

## Phase 2: Type System & Pattern Matching ✅ COMPLETE

**Status:** Complete

### 2.1 Algebraic Data Types ✅

- [x] Enum types with associated data
- [x] `Option<T>` and `Result<T, E>` as built-ins
- [x] Pattern matching with `match` expressions
- [x] Exhaustiveness checking
- [x] Destructuring in `let` bindings

### 2.2 Generics ✅

- [x] Generic functions: `fn map<T, U>(arr: [T], f: fn(T) -> U) -> [U]`
- [x] Generic types: `struct Stack<T> { items: [T] }`

### 2.3 Type System Improvements ✅

- [x] Type aliases: `type UserId = String`
- [x] Union types: `String | Int`
- [x] Nullable types: `String?` (sugar for `Option<String>`)
- [x] Never type for functions that don't return

### 2.4 Effects System (Foundation) ✅

- [x] Effect annotations: `fn read_file(path: String) -> String with io`
- [x] Pure function marking

---

## Phase 3: Module System & Standard Library ✅ COMPLETE

**Status:** Complete

### 3.1 Module System ✅

- [x] File-based modules
- [x] `import` / `export` syntax
- [x] Public/private visibility (`pub` keyword)
- [x] Module aliasing: `import "std/string" as str`
- [x] Selective imports: `import { split, join } from "std/string"`

### 3.2 Core Standard Library ✅

- [x] `std/string`: split, join, trim, replace, contains, starts_with, ends_with, to_upper, to_lower, char_at, substring
- [x] `std/math`: sin, cos, tan, asin, acos, atan, atan2, log, log10, exp, PI, E
- [x] `std/collections`: push, pop, first, last, reverse, slice, concat, is_empty
- [x] `std/env`: get_env, args, cwd

---

## Phase 4: Traits & Essential Features ✅ COMPLETE

**Status:** Complete

**Goal:** Polymorphism, code reuse, and missing language essentials.

### 4.1 Trait Definitions ✅

- [x] Trait declaration syntax
- [x] Required methods
- [x] Default method implementations

```ntnt
trait Serializable {
    fn to_json(self) -> String
    fn from_json(json: String) -> Self
}

trait Comparable {
    fn compare(self, other: Self) -> Int

    // Default implementation
    fn less_than(self, other: Self) -> Bool {
        return self.compare(other) < 0
    }
}
```

### 4.2 Trait Implementations ✅

- [x] `impl Trait for Type` syntax
- [x] Multiple trait implementations
- [x] Trait bounds in generics: `fn sort<T: Comparable>(arr: [T]) -> [T]`

### 4.3 Essential Language Features ✅

- [x] `defer` statement for cleanup (like Go)
- [x] `Map<K, V>` built-in type with literal syntax `map { "key": value }`
- [x] String interpolation: `"Hello, {name}!"`
- [x] Raw strings: `r"SELECT * FROM users"` and `r#"..."#`
- [x] Range syntax: `0..10`, `0..=10`
- [x] For-in loops: `for item in items { }`

```ntnt
fn process_file(path: String) -> Result<Data, Error> {
    let file = open(path)?
    defer file.close()  // Always runs, even on error

    let query = r"SELECT * FROM users WHERE name = 'test'"
    return Ok(data)
}
```

**Deliverables:**

- Full trait system with bounds
- defer statement
- Map type
- String interpolation and raw strings
- Ranges and for-in loops

---

## Phase 5: Async, I/O & Web

**Goal:** Everything needed to build a web application.

### 5.1 Async Runtime

- [ ] `async`/`await` syntax
- [ ] Future type
- [ ] Async function contracts
- [ ] Task executor

```ntnt
async fn fetch_user(id: String) -> Result<User, HttpError>
    requires id.len() > 0
{
    let response = await http.get("/users/" + id)
    return response.json()
}

// Concurrent execution
let (user, posts) = await all(
    fetch_user(id),
    fetch_posts(id)
)
```

### 5.2 File System I/O ✅ COMPLETE

- [x] `std/fs`: read_file, write_file, read_bytes, append_file, exists, is_file, is_dir, mkdir, mkdir_all, readdir, remove, remove_dir, remove_dir_all, rename, copy, file_size
- [x] `std/path`: join, dirname, basename, extension, stem, resolve, is_absolute, is_relative, with_extension, normalize

### 5.3 HTTP Server ✅ COMPLETE

- [x] Built-in HTTP server (using tiny_http)
- [x] Request/Response types
- [x] Router with path parameters
- [x] Middleware support
- [x] Static file serving with MIME type detection
- [x] Contract-verified endpoints (preconditions return 400, postconditions return 500)

```ntnt
import { text, html, json, status, redirect } from "std/http/server"

fn home(req) {
    return text("Welcome!")
}

fn get_user(req) {
    let id = req.params.id
    return json(map {
        "id": id,
        "name": "User " + id
    })
}

// Register routes (use raw strings for path params)
get("/", home)
get(r"/users/{id}", get_user)
post("/users", create_user)

listen(8080)  // Start server
```

### 5.4 HTTP Client ✅ COMPLETE

- [x] `std/http` for HTTP requests (get, post, put, delete, patch, head)
- [x] Full request control with `request()` (method, headers, body, timeout)
- [x] JSON request/response helpers (get_json, post_json)
- [ ] Async HTTP requests (deferred to async runtime)

### 5.5 File-Based Routing & Hot Reload

**Goal:** Enable modular, hot-reloadable applications where the filesystem defines the routing structure. This is critical for AI agents—adding a route means creating a single file, no wiring required.

**Architecture:**

```
app/
├── app.tnt           # Config, middleware, listen()
├── routes/
│   ├── index.tnt     # GET /
│   ├── about.tnt     # GET /about
│   ├── users/
│   │   ├── index.tnt # GET /users
│   │   ├── [id].tnt  # GET /users/:id (dynamic segment)
│   │   └── create.tnt
│   └── api/
│       └── status.tnt
```

**Features:**

- [ ] `routes/` directory convention: file path = URL path
- [ ] Dynamic segments via `[param].tnt` naming (e.g., `[id].tnt` → `/users/:id`)
- [ ] HTTP method exports: `export fn get(req)`, `export fn post(req)`, etc.
- [ ] Hot-reload: file changes detected, module re-parsed on next request
- [ ] Nested routes via subdirectories
- [ ] `index.tnt` as directory root handler
- [ ] Shared layouts/middleware per directory (optional `_layout.tnt`, `_middleware.tnt`)

**Example Route File:**

```ntnt
// routes/users/[id].tnt
// Automatically handles GET /users/:id and DELETE /users/:id

import { json, status } from "std/http/server"
import { find_user, delete_user } from "../../models/user"

export fn get(req) {
    let user = find_user(req.params.id)
    return json(user)
}

export fn delete(req)
    requires req.user.is_admin
{
    delete_user(req.params.id)
    return status(204)
}
```

**App Entry Point:**

```ntnt
// app.tnt
import { use_middleware, listen } from "std/http/server"
import { logger, auth } from "./middleware"

// Apply global middleware
use_middleware(logger)
use_middleware(auth)

// Auto-discover and register all routes from routes/ directory
// Routes are hot-reloaded when files change
routes("routes/")

listen(3000)
```

**Why File-Based Routing:**
| Benefit | Description |
|---------|-------------|
| **AI-Friendly** | "Create `/api/orders`" → Agent creates `routes/api/orders.tnt`. Done. |
| **Locality** | All logic for a route lives in one file |
| **Zero Wiring** | No router configuration to maintain |
| **Hot Reload** | Change a file, refresh browser, see changes |
| **Discoverability** | URL structure mirrors filesystem |

### 5.6 Database Connectivity

- [ ] Connection management
- [ ] Parameterized queries (prevent SQL injection)
- [ ] Transaction support with contracts
- [ ] PostgreSQL driver (built-in)

```ntnt
import { Database } from "std/db/postgres"

fn transfer(db: Database, from: String, to: String, amount: Int) -> Result<(), DbError>
    requires amount > 0
{
    db.transaction(|tx| {
        tx.execute("UPDATE accounts SET balance = balance - $1 WHERE id = $2", [amount, from])?
        tx.execute("UPDATE accounts SET balance = balance + $1 WHERE id = $2", [amount, to])?
        Ok(())
    })
}
```

### 5.7 Supporting Libraries ✅ COMPLETE

- [x] `std/json`: parse, stringify, stringify_pretty
- [x] `std/time`: now, now_millis, now_nanos, sleep, elapsed, format_timestamp, duration_secs, duration_millis
- [x] `std/crypto`: sha256, sha256_bytes, hmac_sha256, uuid, random_bytes, random_hex, hex_encode, hex_decode
- [x] `std/url`: parse, encode, encode_component, decode, build_query, join
- [x] `std/http`: get, post, put, delete, patch, head, request, get_json, post_json

**Deliverables:**

- [ ] Async/await runtime
- [x] File system operations
- [x] HTTP client (blocking)
- [x] HTTP server with routing
- [ ] File-based routing with hot-reload (`routes/` directory convention)
- [ ] PostgreSQL database driver
- [x] JSON, time, crypto, URL utilities

---

## Phase 6: Testing & Intent Annotations

**Goal:** Comprehensive testing with AI-friendly intent tracking.

### 6.1 Test Framework

- [ ] `#[test]` attribute for test functions
- [ ] Test discovery and runner
- [ ] Parallel test execution
- [ ] `assert`, `assert_eq`, `assert_ne` macros
- [ ] `#[should_panic]` for expected failures

```ntnt
#[test]
fn test_user_creation() {
    let user = User.new("Alice", "alice@example.com")
    assert_eq(user.name, "Alice")
    assert(user.email.contains("@"))
}

#[test]
#[should_panic(expected: "invariant violated")]
fn test_invalid_email() {
    User.new("Bob", "invalid-email")
}
```

### 6.2 Contract-Based Testing

- [ ] Auto-generate test cases from contracts
- [ ] Property-based testing with contracts
- [ ] Fuzzing with contract guidance
- [ ] Contract coverage metrics

### 6.3 Mocking & Test Utilities

- [ ] Mock trait implementations
- [ ] HTTP test client
- [ ] Database test utilities
- [ ] Test fixtures

### 6.4 Intent Annotations (From Whitepaper)

- [ ] `intent` blocks linking purpose to code
- [ ] Intent registry and tracking
- [ ] Intent coverage reports
- [ ] AI verification of intent-implementation alignment

```ntnt
intent "Calculate shipping cost based on weight and destination" {
    fn calculate_shipping(weight: Float, dest: String) -> Float
        requires weight > 0
        ensures result >= 0
    {
        let base = weight * 0.5
        let zone_multiplier = get_zone_multiplier(dest)
        return base * zone_multiplier
    }
}

intent "Users must have valid email addresses" {
    impl User {
        invariant self.email.contains("@")
        invariant self.email.contains(".")
    }
}
```

**Deliverables:**

- Test framework with discovery
- Contract-based test generation
- Mocking framework
- Intent annotation system

---

## Phase 7: Tooling & Developer Experience

**Goal:** World-class developer experience with AI collaboration support.

### 7.1 Language Server (LSP)

- [ ] Go to definition
- [ ] Find references
- [ ] Hover documentation
- [ ] Code completion
- [ ] Inline diagnostics
- [ ] Code actions (quick fixes)
- [ ] Contract visualization

### 7.2 Package Manager

- [ ] `ntnt.toml` project configuration
- [ ] Package registry
- [ ] Dependency resolution with lock files
- [ ] Semantic versioning enforcement
- [ ] `ntnt new`, `ntnt add`, `ntnt publish`

```bash
ntnt new my-app
ntnt add http
ntnt add db/postgres --version "^1.0"
ntnt test
ntnt build --release
```

### 7.3 Documentation Generator

- [ ] Doc comments (`///`)
- [ ] Automatic API documentation
- [ ] Contract documentation
- [ ] Example extraction and testing
- [ ] NTNT documentation

### 7.4 Human Approval Mechanisms (From Whitepaper)

- [ ] `@requires_approval` annotations
- [ ] Approval workflows in IDE
- [ ] Audit trails for approved changes
- [ ] Configurable approval policies

```ntnt
@requires_approval("security")
fn delete_all_users(db: Database) -> Result<Int, DbError> {
    db.execute("DELETE FROM users")
}

@requires_approval("api-change")
pub fn get_user(id: String) -> User {
    // Public API changes require review
}
```

### 7.5 Debugger

- [ ] Breakpoints
- [ ] Step debugging
- [ ] Variable inspection
- [ ] Call stack navigation
- [ ] Contract state inspection
- [ ] DAP (Debug Adapter Protocol) support

**Deliverables:**

- Full LSP server
- Package manager with registry
- Documentation generator
- Human approval system
- Debugger

---

## Phase 8: Performance & Compilation

**Goal:** Production-ready performance.

### 8.1 Bytecode Compiler

- [ ] NTNT bytecode format (NBC)
- [ ] Bytecode interpreter (10-50x faster than tree-walking)
- [ ] Bytecode serialization/loading
- [ ] Debug info preservation

### 8.2 Optimizations

- [ ] Constant folding
- [ ] Dead code elimination
- [ ] Inline caching for method calls
- [ ] Escape analysis
- [ ] Contract elision in release builds (configurable)

### 8.3 Memory Management

- [ ] Reference counting with cycle detection
- [ ] Memory pools for hot paths
- [ ] String interning

### 8.4 Static Type Checking

- [ ] Full type inference
- [ ] Flow-sensitive typing
- [ ] Exhaustive type checking at compile time
- [ ] Helpful error messages with suggestions

### 8.5 Advanced Type System Features

- [ ] Associated types in traits
- [ ] Where clauses for complex constraints
- [ ] Contract inheritance (contracts propagate to trait implementations)
- [ ] Liskov Substitution Principle enforcement
- [ ] Contravariant preconditions, covariant postconditions
- [ ] Error context/wrapping: `result.context("message")?`

**Deliverables:**

- Bytecode compiler and VM
- 10-50x performance improvement
- Static type checker
- Advanced type system
- Optimized memory management

---

## Phase 9: AI Integration & Structured Edits

**Goal:** First-class AI development support—NTNT's key differentiator.

### 9.1 Structured Edits (From Whitepaper)

- [ ] AST-based diff format
- [ ] Semantic-preserving transformations
- [ ] Edit operations: AddFunction, ModifyContract, RenameSymbol, etc.
- [ ] Machine-readable edit format for AI agents

```ntnt
// Instead of text diffs, edits are structured:
Edit {
    type: "ModifyContract",
    target: "fn calculate_shipping",
    add_requires: "dest.len() > 0",
    rationale: "Prevent empty destination strings"
}
```

### 9.2 AI Agent SDK

- [ ] Agent communication protocol
- [ ] Context provision API (give AI relevant code context)
- [ ] Suggestion acceptance/rejection tracking
- [ ] Learning from corrections

### 9.3 Semantic Versioning Enforcement

- [ ] API signature tracking across versions
- [ ] Automatic breaking change detection
- [ ] Semver suggestions based on changes
- [ ] `@since` and `@deprecated` annotations

```ntnt
@since("1.2.0")
@deprecated("2.0.0", "Use get_user_by_id instead")
fn get_user(id: String) -> User { }
```

### 9.4 Commit Rationale Generation

- [ ] Structured commit metadata
- [ ] Link commits to intents and requirements
- [ ] Auto-generate changelog entries
- [ ] AI-friendly commit format

**Deliverables:**

- Structured edit engine
- AI agent SDK
- Semantic versioning tools
- Commit rationale system

---

## Phase 10: Deployment & Operations

**Goal:** Production deployment support.

### 10.1 Build & Distribution

- [ ] Single binary compilation
- [ ] Cross-compilation support
- [ ] Minimal Docker image generation
- [ ] Build profiles (dev, release, test)

### 10.2 Configuration

- [ ] Environment-based config
- [ ] Config file support (TOML, JSON)
- [ ] Secrets management patterns
- [ ] Validation with contracts

### 10.3 Observability

- [ ] Structured logging (`std/log`)
- [ ] Metrics collection (Prometheus format)
- [ ] Distributed tracing (OpenTelemetry compatible)
- [ ] Health check endpoints
- [ ] Contract violation reporting

```ntnt
import { Logger, Metrics } from "std/observe"

let log = Logger.new("api")
let requests = Metrics.counter("http_requests_total")

fn handle_request(req: Request) -> Response {
    requests.inc({ path: req.path, method: req.method })
    log.info("Handling request", { path: req.path })
    // ...
}
```

### 10.4 Graceful Lifecycle

- [ ] Signal handling (SIGTERM, SIGINT)
- [ ] Connection draining
- [ ] Shutdown hooks
- [ ] Startup/readiness probes

**Deliverables:**

- Binary compilation
- Docker support
- Observability stack
- Graceful lifecycle management

---

## Future Considerations (Post-1.0)

These features are valuable but not essential for the initial release:

### Native Compilation

- LLVM or Cranelift backend
- Ahead-of-time compilation
- Sub-millisecond startup

### Session Types

- Protocol definitions for typed communication
- Deadlock prevention at compile time
- Formal verification of message sequences

### Additional Database Drivers

- MySQL/MariaDB
- SQLite
- Redis client

### WebSocket Support

- WebSocket server/client
- Message framing
- Connection state management

### Concurrency Primitives

- Channels for message passing
- Structured concurrency (task scopes)
- Parallel iterators

---

## Implementation Priority Matrix

| Phase  | Focus               | Business Value     | Effort   |
| ------ | ------------------- | ------------------ | -------- |
| 1-3 ✅ | Core Language       | Foundation         | Complete |
| 4      | Traits + Essentials | High               | Medium   |
| 5      | Async + Web         | **Critical**       | High     |
| 6      | Testing + Intents   | High               | Medium   |
| 7      | Tooling             | Very High          | High     |
| 8      | Performance         | High               | Medium   |
| 9      | AI Integration      | **Differentiator** | Medium   |
| 10     | Deployment          | High               | Medium   |

---

## Milestones

### M1: Language Complete (End of Phase 4)

- Traits and polymorphism
- All essential language features
- Comprehensive type system

### M2: Web Ready (End of Phase 5)

- HTTP server running
- Database connectivity
- Can build real web apps

### M3: Developer Ready (End of Phase 7)

- Full IDE support
- Package ecosystem
- Documentation
- Human approval workflows

### M4: Production Ready / 1.0 (End of Phase 10)

- Performance optimized
- AI integration complete
- Deployment tooling
- Observability

---

## Success Metrics

- **Time to First App:** Hello World web API in < 30 minutes
- **Performance:** Within 5x of Go for web workloads (bytecode), within 2x with native compilation (future)
- **Safety:** Zero contract violations reach production
- **AI Compatibility:** 95%+ of AI-generated code compiles on first try
- **Developer Satisfaction:** Tooling comparable to Go/Rust

---

## Example: Complete Web Application

```ntnt
// main.tnt - A complete NTNT web application

import { Server, Request, Response } from "std/http"
import { Database } from "std/db/postgres"
import { Logger } from "std/log"

let log = Logger.new("api")
let db = Database.connect(env("DATABASE_URL"))

struct User {
    id: String,
    name: String,
    email: String
}

impl User {
    invariant self.name.len() > 0
    invariant self.email.contains("@")
}

intent "Retrieve a user by their unique ID" {
    fn get_user(req: Request) -> Response
        requires req.params.id.len() > 0
    {
        match db.query_one("SELECT * FROM users WHERE id = $1", [req.params.id]) {
            Ok(user) => Response.json(user),
            Err(_) => Response.not_found("User not found")
        }
    }
}

intent "Create a new user with validated data" {
    fn create_user(req: Request) -> Response
        requires req.body.name.len() > 0
        requires req.body.email.contains("@")
        ensures result.status == 201 || result.status >= 400
    {
        let user = User {
            id: uuid(),
            name: req.body.name,
            email: req.body.email
        }

        db.insert("users", user)?
        log.info("Created user", { id: user.id })

        Response.created(user)
    }
}

@requires_approval("api-change")
pub fn main() {
    let app = Server.new()
        .get("/users/{id}", get_user)
        .post("/users", create_user)
        .use(logging)
        .use(cors)

    log.info("Starting server on port 8080")
    app.listen(8080)
}
```

---

_This roadmap is a living document updated as implementation progresses._
_Last updated: January 2026 (v0.1.2)_
