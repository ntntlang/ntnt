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
- [x] 241 unit tests passing
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
- [x] Template strings (`"""..."""` with `{{}}` interpolation, for loops, conditionals)
- [x] Map iteration functions (`keys()`, `values()`, `entries()`, `has_key()`)
- [x] Nested map inference (nested maps don't require `map` keyword inside `map {}`)
- [x] Truthy/falsy values (0 is truthy, empty strings/arrays/maps are falsy)
- [x] CSV parsing (`std/csv`)
- [x] `ntnt test` command for HTTP endpoint testing

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
- [x] Nested map inference: `map { "a": { "b": 1 } }` (no inner `map` keyword needed)
- [x] Map iteration: `keys(map)`, `values(map)`, `entries(map)`, `has_key(map, key)`
- [x] Truthy/falsy values: 0 is truthy, empty strings/arrays/maps are falsy, None is falsy
- [x] Template strings: `"""..."""` with `{{expr}}` interpolation (CSS-safe)
  - `{{#for x in items}}...{{/for}}` for loops
  - `{{#if cond}}...{{#else}}...{{/if}}` for conditionals
  - `\{{` and `\}}` for literal double braces

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

## Phase 5: Concurrency, I/O & Web ✅ COMPLETE

**Status:** Complete

**Goal:** Everything needed to build a web application.

### 5.1 Concurrency ✅ COMPLETE

**Design Decision:** Go-style concurrency (threads + channels) instead of async/await.

- Simpler mental model (no function coloring)
- Existing stdlib works without modification
- Covers 99% of web app use cases

- [x] `std/concurrent`: channel, send, recv, try_recv, recv_timeout, close
- [x] Thread-safe value serialization for channel communication
- [x] sleep_ms, thread_count utilities

```ntnt
import { channel, send, recv, try_recv, close } from "std/concurrent"

// Create channel for communication
let ch = channel()

// Send values (primitives, arrays, maps, structs)
send(ch, map { "user_id": 123, "action": "signup" })

// Receive (blocks until value available)
let msg = recv(ch)

// Non-blocking receive
match try_recv(ch) {
    Some(value) => process(value),
    None => print("No message yet")
}

// With timeout
match recv_timeout(ch, 5000) {
    Some(value) => handle(value),
    None => print("Timeout after 5 seconds")
}

close(ch)
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

### 5.5 File-Based Routing & Introspection

**Goal:** Convention-based project structure with agent-friendly introspection. No configuration files—the folder structure IS the architecture.

---

#### Project Structure

```
my-app/
├── routes/                # File-based routing (path = URL)
│   ├── index.tnt          # GET /
│   ├── about.tnt          # GET /about
│   ├── users/
│   │   ├── index.tnt      # GET /users
│   │   └── [id].tnt       # GET/POST/DELETE /users/:id
│   └── api/
│       └── orders.tnt     # /api/orders
├── lib/                   # Shared modules (auto-imported)
│   └── db.tnt
└── middleware/            # Auto-loaded in alphabetical order
    ├── 01_logger.tnt
    └── 02_auth.tnt
```

**Conventions:**

- [x] `routes/` - File path = URL path, exports = HTTP methods
- [x] `[param].tnt` - Dynamic URL segments (e.g., `[id].tnt` → `/users/:id`)
- [x] `index.tnt` - Directory root handler
- [x] `lib/` - Shared code, auto-imported into all routes
- [x] `middleware/` - Auto-loaded in alphabetical order (use `01_`, `02_` prefixes)
- [x] Hot-reload on file changes

**Example Route:**

```ntnt
// routes/users/[id].tnt

export fn get(req) {
    let user = db.find_user(req.params.id)
    return json(user)
}

export fn delete(req)
    requires req.user.role == "admin"
{
    db.delete_user(req.params.id)
    return status(204)
}
```

**Entry Point:**

```ntnt
// app.tnt
routes("routes/")  // Auto-discover all routes
listen(3000)
```

---

#### CLI Commands

**`ntnt inspect [path]`** - JSON description of project structure (for agents)

```bash
$ ntnt inspect

{
  "routes": [
    {"method": "GET", "path": "/", "file": "routes/index.tnt"},
    {"method": "GET", "path": "/users/{id}", "file": "routes/users/[id].tnt",
     "contracts": ["requires req.params.id != \"\""]}
  ],
  "lib": ["lib/db.tnt"],
  "middleware": ["middleware/01_logger.tnt", "middleware/02_auth.tnt"]
}
```

**`ntnt validate`** - Check for errors before running

```bash
$ ntnt validate

✓ routes/index.tnt
✓ routes/users/[id].tnt
✗ routes/api/orders.tnt
  Line 15: Unused import 'status'

Errors: 1
```

---

**Why This Matters for Agents:**

| Task                     | Traditional               | NTNT                           |
| ------------------------ | ------------------------- | ------------------------------ |
| Add route `/api/orders`  | Edit router + create file | Create `routes/api/orders.tnt` |
| Understand app structure | Read all files            | `ntnt inspect`                 |
| Check for errors         | Run and hope              | `ntnt validate`                |

**Features:**

- [x] File-based route discovery via `routes()` function
- [x] Dynamic segments `[param].tnt` → `{param}` in URL
- [x] Auto-loaded middleware and lib directories
- [x] Hot-reload on file changes (mtime-based, zero dependencies)
- [x] `ntnt inspect` - JSON introspection (detects file-based routes)
- [x] `ntnt validate` - Pre-run validation

### 5.6 Database Connectivity ✅

- [x] Connection management
- [x] Parameterized queries (prevent SQL injection)
- [x] Transaction support (begin/commit/rollback)
- [x] PostgreSQL driver (`std/db/postgres`)

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
- [x] `std/url`: parse, encode, encode_component, decode, build_query, parse_query, join
- [x] `std/http`: get, post, put, delete, patch, head, request, get_json, post_json
- [x] `std/csv`: parse, parse_with_headers, stringify, stringify_with_headers

### 5.9 Response Caching

**Goal:** Simple in-memory caching for HTTP handlers and expensive operations.

- [ ] `std/cache` module with TTL-based caching
- [ ] `cache()` middleware for route handlers
- [ ] Cache key generation from request (path, query params)
- [ ] `vary` option to cache different responses per parameter
- [ ] Manual cache API: `create_cache`, `get_cached`, `set_cached`, `invalidate`

```ntnt
import { cache } from "std/cache"

// Cache response for 30 minutes (1800 seconds)
get("/api/weather", cache(1800, get_weather))

// With options - vary by query param
get("/api/weather", cache(map {
    "ttl": 1800,
    "vary": ["location"]
}, get_weather))

// Manual cache control
import { create_cache, get_cached, set_cached } from "std/cache"

let api_cache = create_cache()

fn fetch_data(key) {
    let cached = get_cached(api_cache, key, 1800)
    if is_some(cached) {
        return unwrap(cached)
    }
    let data = fetch("https://api.example.com/" + key)
    set_cached(api_cache, key, data)
    return data
}
```

### 5.8 CLI & Testing Tools ✅ COMPLETE

- [x] `ntnt run` - Execute NTNT files
- [x] `ntnt lint` / `ntnt validate` - Pre-run error checking with JSON output
- [x] `ntnt inspect` - JSON introspection for agents (functions, routes, imports)
- [x] `ntnt test` - HTTP endpoint testing (start server, make requests, validate responses)
  - `--get /path`, `--post /path`, `--put /path`, `--delete /path`
  - `--body 'key=value'` for form data
  - `--verbose` for detailed output
  - Automatic server startup and shutdown

**Phase 5 Deliverables:**

- [x] Concurrency primitives (`std/concurrent` - channels, send/recv, thread_count)
- [x] File system operations
- [x] HTTP client (blocking)
- [x] HTTP server with routing
- [x] File-based routing (`routes()` with `routes/`, `lib/`, `middleware/` conventions)
- [x] Hot-reload on file changes (mtime-based, no dependencies)
- [x] `ntnt inspect` - JSON introspection for agents
- [x] `ntnt validate` - Pre-run error checking
- [x] `ntnt test` - HTTP endpoint testing (auto start/stop server)
- [x] PostgreSQL database driver (`std/db/postgres`)
- [x] JSON, time, crypto, URL, CSV utilities
- [x] Template strings with `{{}}` interpolation
- [x] Map iteration functions (`keys`, `values`, `entries`, `has_key`)
- [x] Truthy/falsy value semantics

---

## Phase 6: Intent-Driven Development (IDD)

**Status:** Next Up 🚀

**Goal:** Make NTNT the first language with native Intent-Driven Development—where human intent becomes executable specification.

> See [docs/INTENT_DRIVEN_DEVELOPMENT.md](docs/INTENT_DRIVEN_DEVELOPMENT.md) for the complete design document.

### What is IDD?

Intent-Driven Development creates a **contract layer between human requirements and AI-generated code**. Instead of describing what you want and hoping the AI understands, you write a `.intent` file that is both:

- **Human-readable requirements** - Plain English descriptions anyone can understand
- **Machine-executable tests** - Assertions the system verifies automatically

```yaml
# snowgauge.intent
Feature: Site Selection
  id: feature.site_selection
  description: "Users can select from available monitoring sites"
  test:
    - request: GET /
      assert:
        - status: 200
        - body contains "Bear Lake"
        - body contains "Wild Basin"
```

### 6.1 POC Validation (Go/No-Go Checkpoint) ✅

**Goal:** Prove the concept works before full investment.

- [x] Intent file parser (YAML-based `.intent` files)
- [x] HTTP test runner (start server, make requests, check assertions)
- [x] Basic assertions (`status`, `body contains`, `body matches`)
- [x] `ntnt intent check` command
- [x] Apply to `snowgauge.tnt` example

```bash
# Target behavior
$ ntnt intent check snowgauge.tnt

Feature: Site Selection
  ✓ GET / returns status 200
  ✓ body contains "Bear Lake"
  ✓ body contains "Wild Basin"

2/2 features passing (5/5 assertions)
```

**Success criteria:** Use IDD to develop a new feature in snowgauge. Does it feel useful?

### 6.2 Core Intent Commands ✅

- [x] `ntnt intent check <file.tnt>` - Verify code matches intent
- [x] `ntnt intent init <file.intent>` - Generate code scaffolding from intent
- [x] `ntnt intent coverage <file.tnt>` - Show which features have implementations
- [ ] `ntnt intent diff <file.tnt>` - Gap analysis between intent and code

### 6.3 Code Annotations ✅

- [x] `// @implements: feature.X` comment parsing
- [x] `// @supports: constraint.Y` for supporting code
- [x] `// @utility`, `// @internal`, `// @infrastructure` markers
- [x] Link annotations to intent items
- [ ] Validate IDs exist in intent file

```ntnt
// @implements: feature.site_selection
fn home_handler(req) {
    // This function implements the site selection feature
}
```

### 6.4 Expanded Assertions

**Output Assertions (Current)**

- [x] Status code: `status: 200`
- [x] Body contains: `body contains "text"`
- [x] Regex matching: `body matches r"pattern"`
- [x] Header assertions: `header "Content-Type" contains "text/html"`
- [x] Negation: `body not contains "error"`
- [ ] JSON path: `body.json.users[0].name == "Alice"`
- [ ] Response timing: `response_time < 500ms`

**State & Database Assertions**

- [ ] Database verification: `verify_db:` with SQL queries
- [ ] State before/after comparison

### 6.5 Intent Studio

**Goal:** A collaborative workspace where humans and agents develop intent together.

The `.intent` format is optimized for machine parsing and testing, but humans deserve a better experience when creating and refining intent. Intent Studio provides a beautiful HTML view that makes intent development feel like a creative collaboration, not a chore.

**Phase 1: Basic Studio (MVP) ✅ COMPLETE**

- [x] `ntnt intent studio <file.intent>` - Start studio server
- [x] Rich HTML rendering with feature cards and visual hierarchy
- [x] Auto-refresh via polling (page refreshes every 2 seconds)
- [x] File watcher detects changes
- [x] Auto-open browser on launch (with `--no-open` flag to disable)
- [x] Beautiful dark theme with stats dashboard
- [x] Feature icons based on feature name/type
- [x] Error page with auto-retry when intent file has parse errors

**Phase 2: Enhanced Studio (Later)**

- [ ] WebSocket-based instant live reload (no polling)
- [ ] Implementation status indicators (linked to `@implements` annotations)
- [ ] Collapsible test case details
- [ ] Diff highlighting when intent changes

```bash
# Start intent studio
$ ntnt intent studio server.intent --port 3000

🎨 Intent Studio: http://localhost:3000
👀 Watching server.intent for changes...

# Phase 1: Page auto-refreshes every 2s to pick up changes
# Phase 2: WebSocket pushes instant updates
```

**Workflow:** Human and AI collaborate on intent in real-time:

1. Create or open an existing `.intent` file (`ntnt intent init` or edit directly)
2. Start the studio: `ntnt intent studio server.intent`
3. Human opens studio in browser (side-by-side with editor)
4. Human and AI collaborate—discussing, adding, removing, refining features
5. AI updates `.intent` file, studio refreshes with changes
6. Repeat until human says "looks good!"
7. AI implements the code with `@implements` annotations

### 6.6 Test Execution for All Program Types

- [x] HTTP servers (primary focus)
- [ ] CLI applications (`run:`, `exit_code:`, `stdout:`)
- [ ] Library functions (`eval:`, `result:`)
- [ ] Database operations (`verify_db:`, transactions)

### 6.7 Developer Experience

- [ ] `ntnt intent watch` - Continuous verification during development
- [x] Colored output (green/red for pass/fail)
- [x] Failure details with expected vs actual
- [ ] Intent file line numbers in error messages
- [ ] Parallel test execution

### 6.8 Intent History & Changelog

- [ ] `ntnt intent history <feature>` - View feature evolution
- [ ] `ntnt intent changelog v1 v2` - Generate release notes from intent diffs
- [ ] `ntnt intent archaeology "<term>"` - Search intent history
- [ ] Feature history timeline in Intent Studio
- [ ] Removed feature archive - browse features that were removed
- [ ] Shareable URLs for team review

### 6.9 Advanced Assertions & Behavioral Properties

**Behavioral Properties**

- [ ] Idempotency: `repeat: N` with result comparison
- [ ] Purity: `pure: true` (same input = same output, no side effects)
- [ ] Thread safety: `parallel:` concurrent request testing
- [ ] Sequencing: `sequence:` state machine transitions
- [ ] No unintended mutations: `no_db_writes: true`

**Side Effect Verification**

- [ ] Email sent: `email_sent_to:`
- [ ] Event published: `event_published:`
- [ ] Log verification: `log_contains:`
- [ ] External call verification

**Contract Integration**

- [ ] `contracts:` section linking intent to code contracts
- [ ] Precondition violation testing
- [ ] Postcondition verification
- [ ] Invariant checking across test sequences

**Resource Constraints**

- [ ] Query count: `db_query_count <= N`
- [ ] Memory bounds: `memory_delta < X`
- [ ] Connection limits

### 6.10 Browser & Visual Testing (Future)

- [ ] DOM assertions (element exists, visible, attributes)
- [ ] Browser automation (click, fill, navigate)
- [ ] Visual regression (screenshot comparison)
- [ ] LLM visual verification for subjective qualities

**Phase 6 Deliverables:**

- `.intent` file format and parser
- `ntnt intent check|init|coverage|diff|watch|studio` commands
- `@implements` annotation system
- Test execution engine for HTTP servers
- Intent history and changelog generation
- Intent Studio with WebSocket hot-reload for collaborative intent development
- Applied to `snowgauge.tnt` and other examples

---

## Phase 7: Testing Framework

**Goal:** Comprehensive testing infrastructure complementing Intent-Driven Development.

> IDD tests behavior at the feature level. This phase adds unit testing, mocking, and contract-based test generation for fine-grained code verification.

### 7.1 Unit Test Framework

- [ ] `#[test]` attribute for test functions
- [ ] Test discovery and runner
- [ ] Parallel test execution
- [ ] `assert`, `assert_eq`, `assert_ne` macros
- [ ] `#[should_panic]` for expected failures
- [ ] Test filtering and tagging

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

### 7.2 Contract-Based Test Generation

- [ ] Auto-generate test cases from contracts
- [ ] Property-based testing with contracts
- [ ] Fuzzing with contract guidance
- [ ] Contract coverage metrics
- [ ] Edge case generation from `requires` clauses

```ntnt
// Given this contract:
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{ a / b }

// Auto-generate tests:
// - divide(10, 2) → 5 ✓
// - divide(0, 1) → 0 ✓
// - divide(5, 0) → precondition failure ✓
// - divide(-10, -2) → 5 ✓ (negative handling)
```

### 7.3 Mocking & Test Utilities

- [ ] Mock trait implementations
- [ ] HTTP test client (complements IDD HTTP testing)
- [ ] Database test utilities (test transactions, fixtures)
- [ ] Test fixtures and factories
- [ ] Snapshot testing

```ntnt
#[test]
fn test_with_mock_db() {
    let mock_db = MockDatabase.new()
    mock_db.expect_query("SELECT * FROM users").returns([user1, user2])

    let result = get_all_users(mock_db)
    assert_eq(len(result), 2)
}
```

### 7.4 Test Integration

- [ ] `ntnt test` command (runs all tests)
- [ ] `ntnt test --unit` (unit tests only)
- [ ] `ntnt test --intent` (IDD tests only)
- [ ] Coverage reports (combined unit + IDD)
- [ ] CI/CD integration patterns

```bash
# Run all tests
ntnt test

# Run only unit tests
ntnt test --unit

# Run only IDD feature tests
ntnt test --intent

# Combined coverage report
ntnt test --coverage
```

**Deliverables:**

- `#[test]` attribute system
- Contract-based test generation
- Mocking framework
- Test runner with filtering
- Coverage reporting

---

## Phase 8: Tooling & Developer Experience

**Goal:** World-class developer experience with AI collaboration support.

### 8.1 Language Server (LSP)

- [ ] Go to definition
- [ ] Find references
- [ ] Hover documentation
- [ ] Code completion
- [ ] Inline diagnostics
- [ ] Code actions (quick fixes)
- [ ] Contract visualization

### 8.2 Package Manager

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

### 8.3 Documentation Generator

- [ ] Doc comments (`///`)
- [ ] Automatic API documentation
- [ ] Contract documentation
- [ ] Example extraction and testing
- [ ] NTNT documentation

### 8.4 Human Approval Mechanisms (From Whitepaper)

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

### 8.5 Debugger

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

## Phase 9: Performance & Compilation

**Goal:** Production-ready performance through progressive compilation strategies.

### Current Architecture

```
NTNT Source (.tnt)
       ↓
    Lexer (src/lexer.rs)         ✅ Reusable
       ↓
    Parser (src/parser.rs)       ✅ Reusable
       ↓
      AST (src/ast.rs)           ✅ Reusable
       ↓
  Interpreter (src/interpreter.rs)  ← Tree-walking (current, slowest)
       ↓
    Result
```

### Compilation Roadmap

| Approach                            | Effort     | Speedup   | When      |
| ----------------------------------- | ---------- | --------- | --------- |
| Tree-walking Interpreter            | ✅ Done    | Baseline  | Current   |
| Bytecode VM                         | 2-4 weeks  | 10-50x    | Phase 8.1 |
| Native Compilation (Cranelift/LLVM) | 2-3 months | 100-1000x | Phase 8.4 |

### What Can Be Reused

| Component   | Reusable?   | Notes                       |
| ----------- | ----------- | --------------------------- |
| Lexer       | ✅ 100%     | Tokens don't change         |
| Parser      | ✅ 100%     | AST structure stays same    |
| AST         | ✅ 100%     | Core data structures        |
| Type System | ✅ 100%     | Expansion for optimization  |
| Interpreter | ❌ Replaced | Becomes compiler/codegen    |
| Stdlib      | ⚠️ Partial  | Need native implementations |

### 9.1 Bytecode VM (First Target)

**Goal:** 10-50x performance improvement with moderate effort.

- [ ] Design NTNT bytecode format (NBC)
- [ ] Implement bytecode compiler (`src/compiler.rs`)
- [ ] Implement stack-based VM (`src/vm.rs`)
- [ ] Bytecode serialization/loading (`.tnc` files)
- [ ] Debug info preservation for stack traces
- [ ] Keep interpreter for REPL (faster startup)

```rust
// Example bytecode instructions
enum OpCode {
    LoadConst(usize),      // Push constant onto stack
    LoadVar(String),       // Push variable value
    StoreVar(String),      // Pop and store to variable
    Add, Sub, Mul, Div,    // Arithmetic
    Eq, Lt, Gt, Le, Ge,    // Comparison
    Call(usize),           // Call function with N args
    Return,                // Return from function
    Jump(usize),           // Unconditional jump
    JumpIfFalse(usize),    // Conditional jump
    MakeArray(usize),      // Create array from N stack values
    MakeMap(usize),        // Create map from N key-value pairs
    GetField(String),      // Map/struct field access
    SetField(String),      // Map/struct field assignment
}
```

**CLI Integration:**

```bash
ntnt compile app.tnt        # Compile to bytecode (.tnc)
ntnt run app.tnc            # Run bytecode directly
ntnt run app.tnt            # Auto-compile and run (caches .tnc)
```

### 9.2 VM Optimizations

- [ ] Constant folding at compile time
- [ ] Dead code elimination
- [ ] Inline caching for method calls
- [ ] Escape analysis for stack allocation
- [ ] Contract elision in release builds (configurable)
- [ ] Hot path detection and optimization

### 9.3 Memory Management

- [ ] Reference counting with cycle detection
- [ ] Memory pools for hot paths
- [ ] String interning
- [ ] Small string optimization
- [ ] Arena allocators for request handling

### 9.4 Native Compilation (Future)

**Goal:** Native machine code for maximum performance (100-1000x faster than interpreter).

#### Option A: Cranelift Backend (Recommended)

```
AST → Cranelift IR → Native Machine Code
```

- Simpler API than LLVM
- Good optimization passes
- Used by rustc (experimental) and Wasmtime
- Estimated effort: 1-2 months

```rust
// Using cranelift crate
use cranelift::prelude::*;
use cranelift_module::Module;

fn compile_function(ast: &Function, module: &mut Module) {
    let mut func = Function::new();
    let mut builder = FunctionBuilder::new(&mut func, &mut ctx);

    // Generate Cranelift IR from AST
    for stmt in &ast.body {
        compile_statement(stmt, &mut builder);
    }
}
```

#### Option B: LLVM Backend

```
AST → LLVM IR → LLVM Optimizer → Native Machine Code
```

- Best-in-class optimizations
- Used by Rust, Swift, Julia, Clang
- More complex API
- Estimated effort: 2-3 months

```rust
// Using inkwell (LLVM Rust bindings)
use inkwell::context::Context;
use inkwell::builder::Builder;

fn compile_to_llvm(ast: &Module) -> inkwell::module::Module {
    let context = Context::create();
    let module = context.create_module("ntnt");
    let builder = context.create_builder();

    // Generate LLVM IR from AST
}
```

#### Option C: Transpile to Rust (Creative Alternative)

```
AST → Rust Source Code → cargo build → Native Binary
```

- Leverage Rust's optimizer for free
- Easier debugging (human-readable output)
- Estimated effort: 2-4 weeks

```ntnt
// NTNT source
fn add(a: Int, b: Int) -> Int { a + b }

// Generated Rust
fn add(a: i64, b: i64) -> i64 { a + b }
```

**CLI Integration:**

```bash
ntnt build app.tnt              # Compile to native binary
ntnt build app.tnt --release    # Optimized build
./app                           # Run native binary directly
```

### 9.5 Static Type Checking

- [ ] Full type inference
- [ ] Flow-sensitive typing
- [ ] Exhaustive type checking at compile time
- [ ] Helpful error messages with suggestions
- [ ] Type narrowing in conditionals and match

### 9.6 Advanced Type System Features

- [ ] Associated types in traits
- [ ] Where clauses for complex constraints
- [ ] Contract inheritance (contracts propagate to trait implementations)
- [ ] Liskov Substitution Principle enforcement
- [ ] Contravariant preconditions, covariant postconditions
- [ ] Error context/wrapping: `result.context("message")?`

### 9.7 Runtime Library (for Native Compilation)

Native compilation requires re-implementing stdlib in the target:

- [ ] Core runtime (memory, strings, arrays, maps)
- [ ] I/O operations (file system, HTTP)
- [ ] Database drivers (PostgreSQL bindings)
- [ ] Concurrency primitives (threads, channels)

### 9.8 Advanced Concurrency

Building on Phase 5's channel-based concurrency:

- [ ] `spawn(fn)` / `join(handle)` - background task execution
- [ ] `parallel([fn1, fn2, ...])` - run multiple functions in parallel
- [ ] `select([ch1, ch2, ...])` - wait on multiple channels (Go-style)
- [ ] Async HTTP requests (requires async runtime)

**Deliverables:**

- Bytecode compiler and VM (10-50x speedup)
- Static type checker
- Advanced type system
- Optimized memory management
- Native compilation path (100-1000x speedup)

---

## Phase 10: AI Integration & Structured Edits

**Goal:** First-class AI development support—NTNT's key differentiator.

### 10.1 Structured Edits (From Whitepaper)

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

### 10.2 AI Agent SDK

- [ ] Agent communication protocol
- [ ] Context provision API (give AI relevant code context)
- [ ] Suggestion acceptance/rejection tracking
- [ ] Learning from corrections

### 10.3 Semantic Versioning Enforcement

- [ ] API signature tracking across versions
- [ ] Automatic breaking change detection
- [ ] Semver suggestions based on changes
- [ ] `@since` and `@deprecated` annotations

```ntnt
@since("1.2.0")
@deprecated("2.0.0", "Use get_user_by_id instead")
fn get_user(id: String) -> User { }
```

### 10.4 Commit Rationale Generation

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

## Phase 11: Deployment & Operations

**Goal:** Production deployment support.

### 11.1 Build & Distribution

- [ ] Single binary compilation
- [ ] Cross-compilation support
- [ ] Minimal Docker image generation
- [ ] Build profiles (dev, release, test)

### 11.2 Configuration

- [ ] Environment-based config
- [ ] Config file support (TOML, JSON)
- [ ] Secrets management patterns
- [ ] Validation with contracts

### 11.3 Observability

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

### 11.4 Graceful Lifecycle

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

### Pipeline Operator (`|>`)

Functional-style data transformation chaining (like Elixir, F#, OCaml):

```ntnt
// Current (nested, reads inside-out)
let result = filter(map(split(data, "\n"), trim), fn(x) { len(x) > 0 })

// With pipeline (linear, reads left-to-right)
let result = data
    |> split("\n")
    |> map(trim)
    |> filter(fn(x) { len(x) > 0 })
```

**Why it helps:**

- Linear data flow (reads like English)
- Easier to insert/remove transformation steps
- Self-documenting for agents and humans
- Ideal for CSV/JSON processing, HTTP request chains

**Implementation notes:**

- `x |> f` desugars to `f(x)`
- `x |> f(a, b)` desugars to `f(x, a, b)` (first argument insertion)
- Low implementation effort (parser change + AST node)

### Session Types

- Protocol definitions for typed communication
- Deadlock prevention at compile time
- Formal verification of message sequences

### Additional Database Drivers

**PostgreSQL Enhanced Support (Current):**

- [x] Basic types: INT, BIGINT, FLOAT, DOUBLE, TEXT, VARCHAR, BOOL
- [x] NUMERIC/DECIMAL (via rust_decimal)
- [x] DATE, TIME, TIMESTAMP, TIMESTAMPTZ (via chrono)
- [x] JSON/JSONB
- [x] UUID
- [x] Arrays: INT[], TEXT[], FLOAT[], BOOL[]
- [ ] BYTEA (binary data)
- [ ] INTERVAL
- [ ] PostGIS geometry types

**Additional Drivers:**

- MySQL/MariaDB
- SQLite
- Redis client

### High-Performance HTTP Server

The current HTTP server uses `tiny_http` which is simple and reliable but uses `Connection: close` for each request. For high-traffic production applications:

- Async runtime (tokio/hyper) for concurrent connections
- HTTP/2 support with multiplexing
- Connection pooling and keep-alive
- Request pipelining
- Configurable worker threads
- Zero-copy response streaming
- Performance target: 100k+ req/sec on commodity hardware

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
| 4 ✅   | Traits + Essentials | High               | Complete |
| 5 ✅   | Concurrency + Web   | **Critical**       | Complete |
| 6      | Testing + Intents   | High               | Medium   |
| 7      | Tooling             | Very High          | High     |
| 8      | Performance         | High               | Medium   |
| 9      | AI Integration      | **Differentiator** | Medium   |
| 10     | Deployment          | High               | Medium   |

---

## Milestones

### M1: Language Complete (End of Phase 4) ✅

- Traits and polymorphism
- All essential language features
- Comprehensive type system

### M2: Web Ready (End of Phase 5) ✅

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
- **Performance (Bytecode VM):** Within 5x of Go for web workloads
- **Performance (Native):** Within 2x of Go with Cranelift/LLVM backend
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
_Last updated: January 2026 (v0.1.10)_
