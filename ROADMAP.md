# Intent Language Implementation Roadmap

This document outlines the implementation plan for Intent, a programming language designed for AI-driven development. The roadmap prioritizes getting to a working web application quickly while focusing on Intent's unique differentiators: contracts, AI integration, and intent encoding.

---

## Design Principles

1. **Self-Contained**: Intent has no runtime dependencies on other languages. The interpreter/compiler is written in Rust, but Intent programs are pure Intent.

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
- [x] 127 unit tests passing
- [x] Dual file extensions: `.intent` and `.itn`
- [x] Algebraic Data Types with enums
- [x] Option<T> and Result<T, E> built-ins
- [x] Pattern matching with match expressions
- [x] Generic functions and types with trait bounds
- [x] Type aliases
- [x] Union types
- [x] Effect annotations foundation
- [x] Module system with imports/exports
- [x] Standard library: std/string, std/math, std/collections, std/env, std/fs, std/path, std/json, std/time
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

```intent
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

```intent
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

```intent
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

### 5.3 HTTP Server

- [ ] Built-in HTTP server (no external dependencies)
- [ ] Request/Response types with contracts
- [ ] Router with path parameters
- [ ] Middleware support
- [ ] Static file serving

```intent
import { Server, Request, Response } from "std/http"

fn get_user(req: Request) -> Response
    requires req.params.id.len() > 0
    ensures result.status >= 200
    ensures result.status < 600
{
    let user = db.find_user(req.params.id)?
    return Response.json(user)
}

let app = Server.new()
    .get("/users/{id}", get_user)
    .post("/users", create_user)
    .use(logging)
    .use(cors)

app.listen(8080)
```

### 5.4 HTTP Client

- [ ] `std/http/client` for outbound requests
- [ ] Async HTTP requests
- [ ] Timeout and retry configuration
- [ ] JSON request/response helpers

### 5.5 Database Connectivity

- [ ] Connection management
- [ ] Parameterized queries (prevent SQL injection)
- [ ] Transaction support with contracts
- [ ] PostgreSQL driver (built-in)

```intent
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

### 5.6 Supporting Libraries ✅ PARTIALLY COMPLETE

- [x] `std/json`: parse, stringify, stringify_pretty
- [x] `std/time`: now, now_millis, now_nanos, sleep, elapsed, format_timestamp, duration_secs, duration_millis
- [ ] `std/crypto`: sha256, hmac, uuid, random_bytes
- [ ] `std/url`: parse, encode, decode

**Deliverables:**

- [ ] Async/await runtime
- [x] File system operations
- [ ] HTTP server and client
- [ ] PostgreSQL database driver
- [x] JSON, time utilities
- [ ] Crypto, URL utilities

---

## Phase 6: Testing & Intent Annotations

**Goal:** Comprehensive testing with AI-friendly intent tracking.

### 6.1 Test Framework

- [ ] `#[test]` attribute for test functions
- [ ] Test discovery and runner
- [ ] Parallel test execution
- [ ] `assert`, `assert_eq`, `assert_ne` macros
- [ ] `#[should_panic]` for expected failures

```intent
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

```intent
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

- [ ] `intent.toml` project configuration
- [ ] Package registry
- [ ] Dependency resolution with lock files
- [ ] Semantic versioning enforcement
- [ ] `intent new`, `intent add`, `intent publish`

```bash
intent new my-app
intent add http
intent add db/postgres --version "^1.0"
intent test
intent build --release
```

### 7.3 Documentation Generator

- [ ] Doc comments (`///`)
- [ ] Automatic API documentation
- [ ] Contract documentation
- [ ] Example extraction and testing
- [ ] Intent documentation

### 7.4 Human Approval Mechanisms (From Whitepaper)

- [ ] `@requires_approval` annotations
- [ ] Approval workflows in IDE
- [ ] Audit trails for approved changes
- [ ] Configurable approval policies

```intent
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

- [ ] Intent bytecode format (IBC)
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

**Goal:** First-class AI development support—Intent's key differentiator.

### 9.1 Structured Edits (From Whitepaper)

- [ ] AST-based diff format
- [ ] Semantic-preserving transformations
- [ ] Edit operations: AddFunction, ModifyContract, RenameSymbol, etc.
- [ ] Machine-readable edit format for AI agents

```intent
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

```intent
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

```intent
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

```intent
// main.intent - A complete Intent web application

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
