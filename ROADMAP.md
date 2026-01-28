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
- [x] Basic type system (Int, Float, String, Bool, Array, Object, Function, Unit) — parsed, not yet enforced (→ Phase 7.1)
- [x] Full contract system (`requires`, `ensures`, `old()`, `result`)
- [x] Struct invariants with automatic checking
- [x] Built-in math functions (`abs`, `min`, `max`, `sqrt`, `pow`, `round` with optional decimals, etc.)
- [x] CLI with REPL, run, parse, lex, check commands
- [x] VS Code extension with syntax highlighting
- [x] Comprehensive test suite
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
- [x] Template elif chains (`{{#elif cond}}`)
- [x] Template loop metadata (`@index`, `@first`, `@last`, `@length`, `@even`, `@odd`)
- [x] Template empty blocks (`{{#empty}}` fallback for empty loops)
- [x] Template comments (`{{! comment }}`)
- [x] Template filters (`{{expr | filter1 | filter2(arg)}}`)
- [x] Map iteration functions (`keys()`, `values()`, `entries()`, `has_key()`)
- [x] Nested map inference (nested maps don't require `map` keyword inside `map {}`)
- [x] Truthy/falsy values (0 is truthy, empty strings/arrays/maps are falsy)
- [x] CSV parsing (`std/csv`)
- [x] `ntnt test` command for HTTP endpoint testing
- [x] `ntnt docs` command for stdlib documentation search
- [x] `ntnt docs --generate` for auto-generating reference docs from TOML
- [x] `ntnt completions <shell>` for shell completions (bash, zsh, fish)
- [x] Auto-generated documentation (STDLIB_REFERENCE.md, SYNTAX_REFERENCE.md, IAL_REFERENCE.md)
- [x] External templates with `template()` function (Mustache-style syntax)
- [x] Async HTTP server (Axum + Tokio) with bridge to sync interpreter
- [x] Hot-reload for file-based routes (routes/*.tnt) in async server
- [x] Hot-reload tracks imported files (lib modules, local imports)
- [x] NTNT_ENV=production disables hot-reload for better performance
- [x] Runtime documentation (RUNTIME_REFERENCE.md)

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

### 2.4 Effects System (Foundation) ✅ → Removed in Phase 7.1

- [x] Effect annotations: `fn read_file(path: String) -> String with io`
- [x] Pure function marking
- **Removed:** The Effect enum, `with` keyword parsing, and `pure` keyword parsing are removed in Phase 7.1. The syntax was parsed but never enforced — no runtime or static checking existed. A real effect system requires the static analysis infrastructure from Phase 13+ and is tracked in Future Considerations.

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

- [x] `std/string`: 35+ string functions including split, join, trim, replace, regex (replace_pattern, matches_pattern, find_pattern), regex functions (replace_pattern, matches_pattern, find_pattern, find_all_pattern, split_pattern)
- [x] `std/math`: sin, cos, tan, asin, acos, atan, atan2, log, log10, exp, PI, E
- [x] `std/collections`: push, pop, first, last (with optional defaults), reverse, slice, concat, is_empty, filter, transform
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
  - `{{#if cond}}...{{#elif cond2}}...{{#else}}...{{/if}}` for conditionals with elif
  - `{{#empty}}` fallback for empty loops
  - `@index`, `@first`, `@last`, `@length`, `@even`, `@odd` loop metadata
  - `{{! comment }}` template comments
  - `{{expr | filter1 | filter2(arg)}}` filter/pipe syntax
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

- [x] Built-in HTTP server (Axum + Tokio async runtime)
- [x] Bridge pattern connecting async handlers to sync interpreter
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

- [x] `std/http` with unified `fetch()` API for all HTTP requests
- [x] Full request control via options: method, headers, body, json, form, auth, cookies, timeout
- [x] Response caching with `Cache(ttl)` and `cache_fetch(cache, request)`
- [x] File downloads with `download(url, path)`

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

- [x] `std/json`: parse_json, stringify, stringify_pretty
- [x] `std/time`: now, now_millis, now_nanos, sleep, elapsed, format_timestamp, parse_datetime, duration_secs, duration_millis
- [x] `std/crypto`: sha256, sha256_bytes, hmac_sha256, uuid, random_bytes, random_hex, hex_encode, hex_decode
- [x] `std/url`: parse_url, encode, encode_component, decode, build_query, parse_query, join
- [x] `std/http`: fetch (unified API), download, Cache
- [x] `std/csv`: parse_csv, parse_with_headers, stringify, stringify_with_headers

### 5.8 CLI & Testing Tools ✅ COMPLETE

- [x] `ntnt run` - Execute NTNT files
- [x] `ntnt lint` / `ntnt validate` - Pre-run error checking with JSON output
- [x] `ntnt inspect` - JSON introspection for agents (functions, routes, imports)
- [x] `ntnt test` - HTTP endpoint testing (start server, make requests, validate responses)
  - `--get /path`, `--post /path`, `--put /path`, `--delete /path`
  - `--body 'key=value'` for form data
  - `--verbose` for detailed output
  - Automatic server startup and shutdown
- [x] `ntnt docs` - Stdlib documentation search and generation
- [x] `ntnt completions <shell>` - Shell completions (bash, zsh, fish)

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
- [x] External templates via `template()` function (Mustache-style with partials)
- [x] Async HTTP server (Axum + Tokio) with bridge to sync interpreter

---

## Phase 6: Intent-Driven Development (IDD)

**Status:** Complete ✅

**Goal:** Make NTNT the first language with native Intent-Driven Development—where human intent becomes executable specification.

> See [docs/IAL_REFERENCE.md](docs/IAL_REFERENCE.md) for the Intent Assertion Language reference.

### What is IDD?

Intent-Driven Development creates a **contract layer between human requirements and AI-generated code**. Instead of describing what you want and hoping the AI understands, you write a `.intent` file that is both:

- **Human-readable requirements** - Plain English descriptions anyone can understand
- **Machine-executable tests** - Assertions the system verifies automatically

```yaml
# snowgauge.intent

## Glossary

| Term | Means |
|------|-------|
| a visitor goes to {path} | GET {path} |
| the home page | / |
| the page loads | status 200 |
| they see {text} | body contains {text} |

---

Feature: Site Selection
  id: feature.site_selection

  Scenario: Visitor sees available sites
    When a visitor goes to the home page
    → the page loads
    → they see "Bear Lake"
    → they see "Wild Basin"
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

### 6.2.1 Intent Assertion Language (IAL) Engine ✅

**IAL is a term rewriting system** where natural language assertions are recursively resolved to executable primitives.

Architecture:

```
"they see success response"
    ↓ vocabulary lookup
"component.success_response"
    ↓ component expansion
["status 2xx", "body contains 'ok'"]
    ↓ standard term resolution
[Check(InRange, "response.status", 200-299), Check(Contains, "response.body", "ok")]
    ↓ execution
[✓, ✓]
```

**Core Implementation (src/ial/):**

- [x] `vocabulary.rs` - Pattern matching and term storage
- [x] `resolve.rs` - Recursive term → primitive resolution (~30 lines core logic)
- [x] `execute.rs` - Primitive execution against Context
- [x] `primitives.rs` - Primitive enum (Http, Check) + CheckOp enum
- [x] `standard.rs` - Standard vocabulary definitions

**Primitives (fixed set - new assertions are vocabulary, not code):**

- Actions: `Http`, `Cli`, `Sql`, `ReadFile`
- Checks: `Equals`, `NotEquals`, `Contains`, `NotContains`, `Matches`, `Exists`, `NotExists`, `LessThan`, `GreaterThan`, `InRange`

**High-level API:**

```rust
pub fn run_assertions(assertions: &[String], vocab: &Vocabulary, port: u16) -> IalResult<Vec<ExecuteResult>>
pub fn run_scenario(method: &str, path: &str, body: Option<&str>, assertions: &[String], vocab: &Vocabulary, port: u16) -> IalResult<(bool, Vec<ExecuteResult>)>
```

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

### 6.4 Expanded Assertions (IAL Standard Vocabulary)

**HTTP Assertions (Implemented via IAL)**

- [x] Status code: `status: 200`, `status 2xx`, `status 4xx`
- [x] Body contains: `body contains "text"`, `they see "text"`
- [x] Body negation: `body not contains "error"`, `they don't see "text"`
- [x] Regex matching: `body matches r"pattern"`
- [x] Header assertions: `header "Content-Type" contains "text/html"`
- [x] JSON path: `body json "$.users[0].name" equals "Alice"`
- [x] Redirects: `redirects to /path`
- [x] Content-type: `returns JSON`, `returns HTML`
- [ ] Response timing: `responds in under {time}`

**CLI Assertions (IAL Vocabulary)**

- [x] Exit codes: `exits successfully`, `exits with code {n}`
- [x] Output: `output shows {text}`, `output matches {pattern}`
- [x] Errors: `error shows {text}`, `no error output`

**File Assertions (IAL Vocabulary)**

- [x] Existence: `file {path} exists`, `file {path} is created`
- [x] Content: `file {path} contains {text}`
- [x] Directories: `directory {path} exists`

**Database Assertions (IAL Vocabulary - Definitions ready)**

- [x] Row operations: `record is created`, `record is updated`, `record is deleted`
- [x] Queries: `row exists where {condition}`, `row count is {n}`
- [ ] Database verification: `verify_db:` with SQL queries (execution pending)
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
- [x] **Live test execution** - tests run against a running app
- [x] **Pass/fail indicators** - visual ✓/✗ on every assertion
- [x] **Run Tests button** - re-execute tests on demand
- [x] **Default ports** - Studio on 3001, app on 8081
- [x] **Native hot-reload** - edit .tnt file, changes apply on next request (no restart!)
- [x] **Auto-start app** - Studio automatically starts the matching .tnt file

**Phase 2: Intent Studio V2** (Mostly Complete)

Design: [design-docs/studio-mockup-v2.html](design-docs/studio-mockup-v2.html)

- [x] Health bar visualization (pass/fail/warning/skip percentages)
- [x] Filter chips (All, Failing, Warnings, Skipped, Unlinked, Unit Tests)
- [x] Search across features, scenarios, and assertions
- [x] Expanded feature cards with scenarios and assertions
- [x] Unit test section with test data, corpus testing, property checks
- [x] Invariant bundles display
- [x] Warning states for not-implemented features
- [x] Skip states with precondition failure reasons
- [ ] WebSocket-based instant live reload (currently polling at 10s interval)

**Phase 3: IAL Explorer** ✅ COMPLETE

Design: [design-docs/ial_explorer.html](design-docs/ial_explorer.html)

- [x] Intent file viewer with syntax highlighting
- [x] Interactive glossary term highlighting
- [x] Hover popover showing full resolution chain
- [x] Resolution depth visualization (Level 0 → 1 → 2 → primitive)
- [x] Sidebar glossary reference panel
- [x] Link between Studio and Explorer views

**Phase 4: Enhanced Studio (Later)**

- [ ] Implementation status indicators (linked to `@implements` annotations)
- [ ] Diff highlighting when intent changes

```bash
# Start intent studio (default ports: studio on 3001, app on 8081)
$ ntnt intent studio server.intent

🎨 Intent Studio
  File: server.intent
  URL:  http://127.0.0.1:3001
  App:  http://127.0.0.1:8081
  ✅ Live test execution enabled!

# Custom ports if needed
$ ntnt intent studio server.intent --port 4000 --app-port 9000
```

**Workflow:** Human and AI collaborate on intent with live test feedback:

1. Create or open an existing `.intent` file (`ntnt intent init` or edit directly)
2. Start your app on port 8081 (or use `--app-port` for custom port)
3. Start the studio: `ntnt intent studio server.intent`
4. Human opens studio in browser (side-by-side with editor)
5. Tests run automatically—see which assertions pass ✓ or fail ✗
6. Human and AI collaborate—discussing, adding, removing, refining features
7. AI updates `.intent` file, studio refreshes and re-runs tests
8. Watch tests fail for new features, implement until they pass
9. All tests green = intent is verified!

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

- [x] Idempotency: `property: idempotent` — verifies f(f(x)) == f(x)
- [x] Determinism: `property: deterministic` — verifies f(x) == f(x) across calls
- [x] Round-trip: `property: round_trips` — verifies g(f(x)) == x
- [ ] Purity: `pure: true` (same input = same output, no side effects — requires side-effect tracking)
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

### 6.11 Modular Intent Files (Future)

- [ ] `@include` directive for importing features from other `.intent` files
- [ ] Scoped feature IDs to prevent collisions across modules
- [ ] Module-level constraints that apply to all included features
- [ ] Selective imports: `@include "auth.intent" only [feature.login, feature.logout]`

```intent
# Main application intent file
# Includes modules for large applications

@include "modules/auth.intent"
@include "modules/products.intent"
@include "modules/checkout.intent" only [feature.cart, feature.payment]

## Overview
Full e-commerce platform composed from reusable intent modules.

---

Constraint: Global Rate Limiting
  description: "All API endpoints are rate limited"
  applies_to: [auth.*, products.*, checkout.*]
```

> **Note:** For most applications, a single `.intent` file with `## Module:` section headers is recommended. The `@include` directive is for very large projects or organizations that need to share intent modules across multiple applications.

---

## Phase 7: Language Ergonomics ← UP NEXT

**Status:** Not Started

**Goal:** Address the biggest daily friction points for AI agents and human developers writing NTNT code. The type system comes first because it's a foundation that makes every subsequent feature stronger.

> These features were identified through real-world usage as the highest-impact improvements to the language. The type system is sequenced first because error propagation (`?`) needs to know return types, closures benefit from type inference, and SQLite needs type mapping. Together, these features transform a typical web handler from ~22 lines of match pyramids to ~6 lines of linear, readable code.

### 7.1 Type System Enforcement

**Priority:** Foundation — everything else in this phase builds on real types.

Currently, type annotations are parsed but not enforced. This is the worst of both worlds: syntax noise without safety guarantees. NTNT needs to commit to real types.

**Design: Enforced types with aggressive inference.**

```ntnt
// Function signatures require types (the contract boundary)
fn add(a: Int, b: Int) -> Int {
    return a + b
}

// Local variables are inferred — no annotation needed
let x = 5              // inferred: Int
let name = "Alice"     // inferred: String
let nums = [1, 2, 3]  // inferred: [Int]

// Explicit annotation optional, useful for documentation
let threshold: Float = 3.14

// Type errors caught at lint/validate time, not runtime
fn greet(name: String) -> String {
    return "Hello, " + name
}
greet(42)  // ✗ Type error: expected String, got Int
```

**Two layers of safety — types + contracts:**

```ntnt
// Types catch STRUCTURAL errors (wrong kind of data)
// Contracts catch SEMANTIC errors (right kind, wrong value)
fn divide(a: Int, b: Int) -> Int
    requires b != 0                    // contract: semantic check
    ensures result * b == a            // contract: behavioral check
{
    return a / b    // types guarantee a and b are Int
}

// The contract checker can now verify:
// "ensures result > 0" — result is Int, comparison to Int is valid ✓
// "ensures len(result)" — result is Int, len() expects String/Array ✗
```

**Implementation plan:**

- [ ] Type inference engine for local variables (`let x = 5` → `Int`)
- [ ] Type checking at function call boundaries (argument types match parameter types)
- [ ] Return type verification (function body returns declared type)
- [ ] Type inference for expressions (arithmetic, string concat, comparisons)
- [ ] Generic type resolution (`Option<Int>`, `Result<String, Error>`)
- [ ] Union type checking (`String | Int` accepts either)
- [ ] Array element type inference (`[1, 2, 3]` → `[Int]`)
- [ ] Map value type inference (`map { "a": 1 }` → `Map<String, Int>`)
- [ ] Type errors in `ntnt lint` and `ntnt validate` output (not just runtime)
- [ ] Helpful error messages: "expected String, got Int" with line/column
- [ ] Contract expression type-checking (`ensures len(result) > 0` — verify `result` is a type `len()` accepts)
- [ ] Gradual typing: untyped parameters default to `Any` (backward compatible)
- [ ] Remove the `Effect` enum from `src/types.rs` (7 variants, never checked at runtime)
- [ ] Remove `with` keyword effect parsing from `src/parser.rs` (lines 244-258)
- [ ] Remove `pure` keyword parsing from function signatures
- [ ] Remove `TypeExpr::WithEffect` variant from AST
- [ ] Remove the two effect-related tests from `src/interpreter.rs` (test_effects_annotation, test_pure_function)
- [ ] Keep `TokenKind::With` and `TokenKind::Pure` as reserved keywords (forward compatibility)

**Backward compatibility:** Existing NTNT code continues to work. Untyped function parameters are treated as `Any`. Adding types is opt-in but encouraged. Over time, `ntnt lint` can warn about untyped public function signatures.

### 7.2 Error Propagation (`?` Operator)

**Priority:** Highest friction point — depends on 7.1 to verify return types are `Result`/`Option`.

Currently, every fallible operation requires an explicit `match`:

```ntnt
// Current: verbose error handling
fn handle_request(req) {
    match parse_json(req) {
        Ok(data) => {
            match validate(data) {
                Ok(valid) => {
                    match save_to_db(valid) {
                        Ok(result) => return json(result),
                        Err(e) => return status(500, "DB error: " + str(e))
                    }
                },
                Err(e) => return status(400, "Invalid: " + str(e))
            }
        },
        Err(e) => return status(400, "Parse error: " + str(e))
    }
}
```

With `?` operator:

```ntnt
// Target: concise error propagation
fn handle_request(req: Request) -> Result<Response, Error> {
    let data = parse_json(req)?
    let valid = validate(data)?
    let result = save_to_db(valid)?
    return Ok(json(result))
}
```

**Implementation plan:**

- [ ] `?` operator on `Result<T, E>` values — unwrap `Ok` or early-return `Err`
- [ ] `?` operator on `Option<T>` values — unwrap `Some` or early-return `None`
- [ ] Error type coercion (auto-convert between compatible error types)
- [ ] Type system verifies `?` is used in functions that return `Result` or `Option`
- [ ] Clear error message when `?` is used in a function with wrong return type

### 7.3 Anonymous Functions / Closures

**Priority:** High — needed for idiomatic use of `filter`, `transform`, and higher-order functions.

Currently, every callback requires a named function:

```ntnt
// Current: must define named functions for simple transformations
fn double(x) { return x * 2 }
fn is_even(x) { return x % 2 == 0 }

let doubled = transform(nums, double)
let evens = filter(nums, is_even)
```

With closures:

```ntnt
// Target: inline anonymous functions
let doubled = transform(nums, fn(x: Int) -> Int { x * 2 })
let evens = filter(nums, fn(x: Int) -> Bool { x % 2 == 0 })

// Type inference from context — when transform() expects fn(Int) -> T,
// parameter types can be inferred:
let doubled = transform(nums, fn(x) { x * 2 })

// Closures capture surrounding variables
let threshold = 10
let above = filter(nums, fn(x) { x > threshold })
```

**Implementation plan:**

- [ ] Anonymous function expression: `fn(params) { body }`
- [ ] Implicit return (last expression is return value) for single-expression closures
- [ ] Variable capture from enclosing scope (closure semantics)
- [ ] Closures as function arguments (higher-order functions)
- [ ] Type inference from call context (parameter types inferred from expected signature)
- [ ] Named function handlers still required for HTTP routes (readability rule)

### 7.4 SQLite Support (`std/db/sqlite`)

**Priority:** High — the most common database for small web apps, requires no external server.

SQLite is the natural database for NTNT's sweet spot: AI-generated web prototypes and small applications. Unlike PostgreSQL, it requires zero setup — just a file path.

```ntnt
import { connect, query, execute } from "std/db/sqlite"

let db = connect("app.db")

// Create tables
execute(db, "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)")

// Parameterized queries (safe from injection)
execute(db, "INSERT INTO users (name, email) VALUES (?, ?)", ["Alice", "alice@example.com"])

// Query returns array of maps
let users = query(db, "SELECT * FROM users WHERE name = ?", ["Alice"])
for user in users {
    print("User: " + user["name"])
}
```

**Implementation plan:**

- [x] `std/db/sqlite` module (Rust `rusqlite` crate with `bundled` feature)
- [x] `connect(path)` — open or create SQLite database file
- [x] `query(db, sql, params?)` — parameterized SELECT queries
- [x] `execute(db, sql, params?)` — INSERT/UPDATE/DELETE with param binding
- [x] `close(db)` — close database connection
- [x] Transaction support (`begin`, `commit`, `rollback`)
- [x] In-memory databases: `connect(":memory:")`
- [x] Type mapping: INTEGER↔Int, REAL↔Float, TEXT↔String, BLOB↔Array<Int>, NULL↔Unit

### 7.5 Pipe Operator (`|>`)

**Priority:** Moderate — low implementation effort, eliminates daily friction with nested function calls.

> Moved from Future Considerations. The assessment identified nested function calls as a concrete pain point: "the nested function call style gets ugly." With closures (7.3) now in the same phase, pipelines become especially powerful.

```ntnt
// Current: nested, reads inside-out
let result = join(split(trim(to_lower(input)), " "), "-")

// With pipe operator: linear, reads left-to-right
let result = input
    |> to_lower
    |> trim
    |> split(" ")
    |> join("-")

// Powerful with closures (7.3):
let active_emails = users
    |> filter(fn(u) { u.active })
    |> transform(fn(u) { u.email })
    |> join(", ")
```

**Implementation plan:**

- [x] `|>` operator in lexer (`PipeArrow` token) and parser (desugars to `Expression::Call` at parse time — no new AST node needed)
- [x] `x |> f` desugars to `f(x)` — simple rewrite
- [x] `x |> f(a, b)` desugars to `f(x, a, b)` — first-argument insertion
- [ ] Type checking flows through the pipe chain (output type of left = input type of right)
- [ ] Error messages show the failing step in the pipeline, not just the final result

### 7.6 Better Error Messages

**Priority:** High — the assessment rated NTNT's error messages as "Basic" and "notably behind languages like Rust or Elm." For an AI-first language, rich errors are essential for self-correction loops.

Currently, errors are terse:

```
Undefined variable: usr
Type error: expected Int, got String
```

Target: context-rich, actionable errors:

```
Error[E001]: Undefined variable `usr`
  --> server.tnt:45:12
   |
45 |     return json(usr)
   |                 ^^^ not found in this scope
   |
   help: did you mean `user`? (defined at line 42)
```

```
Error[E012]: Type mismatch in function call
  --> server.tnt:23:18
   |
23 |     let result = add("hello", 5)
   |                      ^^^^^^^ expected Int, got String
   |
   note: function `add` defined at server.tnt:10
         fn add(a: Int, b: Int) -> Int
```

**Implementation plan:**

- [x] Error codes (E001-E012) on all error variants for machine-parseable errors
- [x] "Did you mean?" suggestions for undefined variables (Levenshtein distance)
- [x] "Did you mean?" suggestions for undefined functions (Levenshtein distance)
- [x] Function name included in arity mismatch errors
- [x] Column info added to ParserError
- [x] Source code snippets in error output (3-line context for parser errors)
- [x] Color-coded CLI output (error codes in red, line numbers in blue, suggestions in green, help text in cyan)
- [ ] Full AST span tracking on all nodes (line, column, span) — deferred
- [ ] Runtime error line numbers via AST span tracking — deferred
- [ ] "Did you mean?" suggestions for wrong imports (scan stdlib for similar names)
- [ ] Contract violation messages show the contract expression and actual values
- [ ] `ntnt lint --format=json` structured error output for agent consumption

### 7.7 Route Pattern Auto-Detection

**Priority:** Moderate — eliminates the most common "gotcha" in NTNT web development.

Currently, route parameters use `{id}` syntax, which collides with string interpolation. Forgetting to use raw strings (`r""`) is a recurring friction point:

```ntnt
// Current: must remember r"" for every route
get(r"/users/{id}", get_user)        // ✅ works
get("/users/{id}", get_user)         // ❌ tries to interpolate {id}

// Target: route functions auto-detect patterns, no r"" needed
get("/users/{id}", get_user)         // ✅ just works
post("/api/orders/{id}/items", add_item)  // ✅ just works
```

Route registration functions (`get`, `post`, `put`, `patch`, `delete`) are already builtins — the compiler knows what they are. Their first argument should automatically suppress string interpolation and treat `{...}` as route parameter placeholders.

**Implementation plan:**

- [x] Route builtin functions treat their path argument as a route pattern (no interpolation)
- [x] `{param}` in route patterns is always a route parameter, never interpolation
- [x] Raw strings (`r""`) still work for backward compatibility
- [ ] `ntnt lint` warns if a raw string is used unnecessarily in a route (style hint)
- [x] Dynamic route registration (rare) uses an explicit API if needed

### 7.8 Destructuring Assignment

**Priority:** High — every POST handler parses form data or JSON into individual variables. This is the most repetitive boilerplate in NTNT web code.

```ntnt
// Current: 4 lines to extract fields
let form = parse_form(req)
let name = form["name"]
let email = form["email"]
let age = form["age"]

// With destructuring: 1 line
let { name, email, age } = parse_form(req)

// Works with type annotations
let { name: String, email: String, age: Int } = parse_form(req)

// Nested destructuring
let { user: { name, email }, role } = parse_json(req)?

// Array destructuring
let [first, second, ...rest] = split(line, ",")

// In function parameters
fn create_user({ name, email }: Map) -> User {
    return User { name: name, email: email }
}
```

**Implementation plan:**

- [ ] Map destructuring in `let` bindings: `let { key1, key2 } = expr`
- [ ] Array destructuring: `let [first, second] = expr`
- [ ] Rest patterns: `let [head, ...tail] = arr`
- [ ] Nested destructuring: `let { user: { name } } = data`
- [ ] Destructuring with type annotations
- [ ] Destructuring in function parameters
- [ ] Destructuring in `for` loops: `for { name, email } in users { ... }`
- [ ] Type checking: destructured fields are type-inferred from the source expression

### 7.9 Default Parameter Values

**Priority:** Moderate — reduces boilerplate in utility functions and makes APIs more ergonomic.

```ntnt
// Current: caller must always pass all arguments
fn paginate(items, page, per_page) {
    let start = (page - 1) * per_page
    return slice(items, start, start + per_page)
}
paginate(users, 1, 25)  // almost always 1 and 25

// With defaults: optional arguments have sensible fallbacks
fn paginate(items: [Any], page: Int = 1, per_page: Int = 25) -> [Any] {
    let start = (page - 1) * per_page
    return slice(items, start, start + per_page)
}
paginate(users)           // page=1, per_page=25
paginate(users, 3)        // page=3, per_page=25
paginate(users, 2, 50)    // page=2, per_page=50

// Works with web handler helpers
fn respond(data: Map, status_code: Int = 200, content_type: String = "application/json") -> Response {
    return status(status_code, stringify(data))
}
```

**Implementation plan:**

- [ ] Default value expressions in function parameter lists: `param: Type = expr`
- [ ] Default parameters must come after required parameters
- [ ] Default expressions evaluated at call time (not definition time)
- [ ] Type inference: default value provides type if annotation is missing
- [ ] `ntnt inspect` includes default values in function signatures
- [ ] Works with contracts: `requires` can reference defaulted parameters

### 7.10 Guard Clauses (`let-else`)

**Priority:** High — eliminates a level of nesting for every validation check. Pairs with `?` (7.2) to flatten web handlers from match pyramids to linear sequences.

The `?` operator handles `Result` propagation, but web handlers also bail out for non-Result conditions — "if the user doesn't exist, return 404." Today this requires a full `match` block that pushes the happy path one indent level deeper:

```ntnt
// Current: match pyramid for every check
match find_user(id) {
    Some(user) => {
        match find_order(user, order_id) {
            Some(order) => {
                // ... handler at indent level 2
            },
            None => return status(404, "Order not found")
        }
    },
    None => return status(404, "User not found")
}

// With guard clauses: flat, linear, reads top-to-bottom
let user = find_user(id) else return status(404, "User not found")
let order = find_order(user, order_id) else return status(404, "Order not found")
// ... handler at indent level 0
```

**Implementation plan:**

- [ ] `let x = expr else { diverging_expr }` syntax (like Rust's let-else)
- [ ] The `else` block must diverge (return, break, continue, or panic)
- [ ] Works with `Option`: unwraps `Some(v)` or runs else block on `None`
- [ ] Works with `Result`: unwraps `Ok(v)` or runs else block on `Err(e)`
- [ ] The error value is accessible in the else block: `let x = expr else |e| return status(500, str(e))`
- [ ] Type system verifies the else block diverges (doesn't fall through)

### 7.11 Intent File Cleanup

**Priority:** Low — small hygiene task, same spirit as the Effect enum removal.

- [ ] Remove unused `Meta:` section parsing from intent files (the `## Overview` section serves the same purpose)
- [ ] Clean up any other dead parsing paths identified during Phase 7 work

**Deliverables:**

- Type system with inference and enforcement
- Effect enum removed (dead code cleanup)
- `?` operator for Result and Option types
- Anonymous functions with closure semantics
- `std/db/sqlite` module with full CRUD support
- Pipe operator for linear data transformations
- Context-rich error messages with suggestions and source snippets
- Route pattern auto-detection (no more `r""` for route paths)
- Destructuring assignment (maps, arrays, nested, in parameters and loops)
- Default parameter values
- Guard clauses (`let-else`) for flat validation sequences
- Intent file Meta section cleanup
- Updated examples using new features

---

## Phase 8: Intent System Maturity

**Status:** Not Started

**Goal:** Make Intent-Driven Development a tool that AI agents and humans genuinely rely on — not just for testing, but as the shared plane of understanding and accountability between human and agent.

> Phase 6 proved the concept: intent files, the IAL engine, and `ntnt intent check` work. This phase makes the system something an agent *wants* to use by fixing the friction points discovered through real-world usage: opaque failures, offline validation, glossary debugging, and the lack of shared decision history.

### 8.1 Resolution Chain in Failure Output

**Priority:** Highest — when a test fails, the agent currently has to play detective.

Today, a failure looks like:

```
FAIL: they see "Welcome"
```

The agent doesn't know: was the response a 500? Was the body empty? Was "Welcome" misspelled? The IAL engine already has the full resolution chain internally — it just doesn't surface it.

**Target output:**

```
FAIL: they see "Welcome"
  Resolved: body contains "Welcome"
  Primitive: Check(Contains, response.body, "Welcome")

  Actual status: 200
  Actual body: "<h1>Welcom to the site</h1>"
  ─────────────────────────
  Closest match: "Welcom" (missing 'e')
```

**Implementation plan:**

- [ ] Surface the resolution chain in `ntnt intent check` failure output (glossary term → standard term → primitive)
- [ ] Show actual HTTP response data on failure (status, body excerpt, headers)
- [ ] Fuzzy match suggestions when `body contains` fails ("did you mean 'Welcom'?")
- [ ] JSON output mode (`--json`) for agent consumption of failure details
- [ ] Show resolution chain in Intent Studio failure cards

### 8.2 Offline Intent Validation (`ntnt intent validate`)

**Priority:** High — the collaborative design phase needs fast feedback without starting a server.

During the design phase (drafting features, refining scenarios with the human), there's no way to check if the intent file is well-formed without starting the full server. This makes iteration slow.

```bash
$ ntnt intent validate server.intent

✓ 12 features parsed
✓ 8 glossary terms defined
✓ All terms resolve to primitives
⚠ Feature "user.profile" has no scenarios
⚠ Glossary term "admin user" is defined but never used
✗ Scenario "Edit profile" uses undefined term "they are redirected"
  hint: did you mean "redirects to"? (standard term)

11 features valid, 1 error, 2 warnings
```

**Implementation plan:**

- [ ] `ntnt intent validate <file.intent>` — parse and validate without server
- [ ] Check all glossary terms resolve to primitives (no dangling references)
- [ ] Warn on unused glossary terms
- [ ] Warn on features with no scenarios
- [ ] Warn on duplicate feature IDs
- [ ] Validate `@implements` annotations reference existing feature IDs (cross-check with `.tnt` file)
- [ ] Suggest corrections for unresolved terms (Levenshtein distance against glossary + standard terms)
- [ ] JSON output mode for agent consumption

### 8.3 Glossary Inspector (`ntnt intent glossary`)

**Priority:** Moderate — the glossary is powerful but opaque. Agents and humans need to see what terms are available and how they resolve.

```bash
$ ntnt intent glossary server.intent

Custom Terms (8):
  "they see {text}"          → body contains {text}
  "success response"         → status 200
  "the home page"            → /
  "a logged in user"         → component.authenticated_user
  "a user posts to {path}"   → POST {path}
  ...

Standard Terms (24):
  "status {code}"            → Check(Equals, response.status, {code})
  "body contains {text}"     → Check(Contains, response.body, {text})
  "redirects to {path}"      → Check(Equals, response.headers.location, {path})
  ...

Resolution Trace:
  "they see success response"
    → "they see {text}" where text = "success response"
    → body contains "success response"
    → Check(Contains, response.body, "success response")
    ⚠ Note: "success response" is a glossary term, not literal text.
       The assertion checks for the literal string "success response" in the body.
       If you meant status 200, use "→ success response" as its own line.
```

**Implementation plan:**

- [ ] `ntnt intent glossary <file.intent>` — list all custom and standard terms
- [ ] `ntnt intent glossary <file.intent> --trace "<term>"` — show full resolution chain for a specific term
- [ ] Detect semantic misuse (glossary term used as literal text inside another term)
- [ ] Show which scenarios use each glossary term (reverse lookup)
- [ ] `--json` output for agent consumption
- [ ] Integration with Intent Studio (glossary panel)

### 8.4 Feature Status Tracking

**Priority:** Moderate — makes the intent file a living project document, not just a static test spec.

```intent
Feature: User Login
  id: feature.user_login
  status: implemented
  since: v0.3.0

Feature: Password Reset
  id: feature.password_reset
  status: planned

Feature: OAuth Integration
  id: feature.oauth
  status: deprecated
  reason: "Replaced by SAML in v0.4.0"
```

**Behavior:**

- `status: planned` — scenarios are **skipped** during `intent check` (not failed), shown as "planned" in Studio
- `status: implemented` — scenarios run normally (default if no status specified)
- `status: deprecated` — scenarios still run but shown with deprecation warning in Studio
- `since:` — tracks when a feature was introduced (informational, used in changelog generation)

**Implementation plan:**

- [ ] Parse `status:` field on Feature blocks (planned | implemented | deprecated)
- [ ] Parse `since:` field (version string, informational)
- [ ] Parse `reason:` field for deprecated features
- [ ] `ntnt intent check` skips planned features with clear "SKIP (planned)" output
- [ ] `ntnt intent check` shows deprecated warnings
- [ ] Intent Studio renders status badges on feature cards
- [ ] `ntnt intent check --include-planned` flag to run planned features (expect failures)

### 8.5 Decision Records

**Priority:** Moderate — the highest-leverage accountability feature. Records *why* choices were made, not just *what* was built.

The intent file currently records what the human and agent agreed to build. But it doesn't record the decisions that shaped those features — why session tokens instead of JWTs, why PostgreSQL instead of SQLite, why this API shape and not another. When an agent returns to a project in a new session, that context is lost.

```intent
Feature: User Authentication
  id: feature.user_auth

  Decision: Session tokens over JWTs
    date: 2026-01-15
    context: "MVP needs simple auth. JWTs add complexity (refresh tokens,
             signing keys) without clear benefit at this scale."
    decided_by: human
    alternatives_considered:
      - "JWT with refresh tokens"
      - "OAuth2 with external provider"

  Decision: Bcrypt for password hashing
    date: 2026-01-15
    context: "Industry standard, built into std/crypto."
    decided_by: agent

  Scenario: Successful login
    When a user posts valid credentials to /login
    → success response
    → they see "session_token"
```

**Why this matters for human-agent collaboration:**

- **Agent context recovery** — when I start a new session, I can read decisions to understand why the code looks the way it does, without asking questions the human already answered
- **Human accountability** — decisions have an author. If something breaks because of a design choice, the history shows who made it and why
- **Design archaeology** — `ntnt intent decisions` lists all decisions across features, creating a lightweight Architecture Decision Record (ADR) system built into the workflow

**Implementation plan:**

- [ ] Parse `Decision:` blocks inside Feature sections
- [ ] Fields: `date:`, `context:`, `decided_by:` (human | agent), `alternatives_considered:` (optional list)
- [ ] `ntnt intent decisions <file.intent>` — list all decisions across features
- [ ] `ntnt intent decisions <file.intent> --by human` — filter by decision maker
- [ ] Intent Studio renders decisions as expandable sections on feature cards
- [ ] Decision records are informational — they don't affect test execution

**Deliverables:**

- Resolution chain visibility in all failure output
- `ntnt intent validate` for offline structural checking
- `ntnt intent glossary` for term inspection and resolution tracing
- Feature status tracking with skip behavior for planned features
- Decision records for shared human-agent accountability
- All new commands support `--json` output for agent consumption

---

## Phase 9: Package & Module Ecosystem

**Status:** Not Started

**Goal:** Let NTNT be extended beyond the standard library. This is the single biggest barrier to adoption — every project eventually needs something the stdlib doesn't have.

> This phase delivers the foundation: local packages, git dependencies, and a project manifest. The full registry and publishing infrastructure comes later in Phase 12.2 (Tooling & DX). This is the single feature the assessment identified as "the biggest barrier to adoption."

### 9.1 Project Manifest (`ntnt.toml`)

Every NTNT project gets a manifest file that declares metadata and dependencies:

```toml
[project]
name = "my-app"
version = "0.1.0"
entry = "server.tnt"

[dependencies]
markdown = { path = "../ntnt-markdown" }        # Local path
email = { git = "https://github.com/user/ntnt-email.git" }  # Git URL
```

**Implementation plan:**

- [ ] `ntnt.toml` parser (TOML format, Rust `toml` crate)
- [ ] `ntnt new <name>` — scaffold a new project with `ntnt.toml`, `server.tnt`, and directory structure
- [ ] Project metadata: name, version, description, author, license, entry point
- [ ] `ntnt run` auto-detects `ntnt.toml` and resolves dependencies before execution
- [ ] `ntnt.lock` lockfile for reproducible builds

### 9.2 NTNT-Native Packages

Packages are directories of NTNT code with a `ntnt.toml` manifest that other projects can import:

```
ntnt-markdown/
├── ntnt.toml
├── lib.tnt          # Package entry point (exports public API)
├── src/
│   ├── parser.tnt
│   ├── renderer.tnt
│   └── extensions.tnt
└── tests/
    └── markdown_tests.tnt
```

**lib.tnt (package entry point):**
```ntnt
// Re-export public API
import { parse } from "./src/parser"
import { render_html, render_text } from "./src/renderer"

export { parse, render_html, render_text }
```

**Consumer usage:**
```ntnt
import { parse, render_html } from "markdown"

fn blog_handler(req: Request) -> Response {
    let content = read_file("posts/" + req.params["slug"] + ".md")
    let html_content = render_html(parse(content))
    return html(html_content)
}
```

**Implementation plan:**

- [ ] Package resolution: name in `ntnt.toml` [dependencies] → path or git URL → directory with `ntnt.toml`
- [ ] Package imports: `import { x } from "package-name"` resolves to the package's `lib.tnt` exports
- [ ] Local path dependencies: `{ path = "../my-package" }`
- [ ] Git dependencies: `{ git = "https://..." }` — clone to a cache directory
- [ ] Git ref pinning: `{ git = "...", tag = "v1.0.0" }` or `{ git = "...", rev = "abc123" }`
- [ ] Dependency caching: packages cached in `~/.ntnt/packages/`
- [ ] `ntnt add <name> --path <path>` — add a local dependency
- [ ] `ntnt add <name> --git <url>` — add a git dependency
- [ ] Circular dependency detection

### 9.3 Rust Extension Packages (FFI)

For capabilities that can't be written in pure NTNT (system libraries, performance-critical code, bindings to existing ecosystems):

```toml
# ntnt-redis/ntnt.toml
[project]
name = "redis"
version = "0.1.0"
type = "native"          # Indicates Rust extension

[native]
crate = "ntnt-redis"     # Rust crate name
```

**Implementation plan:**

- [ ] Extension API: Rust trait that native packages implement to expose functions to NTNT
- [ ] Dynamic loading: `.so`/`.dylib`/`.dll` loaded at runtime
- [ ] Type marshaling: Rust types ↔ NTNT `Value` conversion
- [ ] Standard extension trait with `register_functions()` method
- [ ] Pre-built extensions for common needs (Redis, email, image processing)
- [ ] `ntnt build-ext` command for compiling Rust extensions

### 9.4 Stdlib as Packages

Refactor parts of the standard library to use the same package infrastructure, proving the system works:

- [ ] Extract `std/csv` as a standalone package (simple, good test case)
- [ ] Extract `std/crypto` as a standalone package
- [ ] Built-in packages resolve from the interpreter binary (no download needed)
- [ ] Stdlib packages serve as reference implementations for package authors

**Deliverables:**

- `ntnt.toml` project manifest with dependency declaration
- `ntnt new` project scaffolding
- Local path and git URL dependency resolution
- Package import system (`import { x } from "package-name"`)
- Rust FFI extension API for native packages
- Dependency caching and lockfile
- At least two stdlib modules extracted as proof-of-concept packages

---

## Phase 10: Background Jobs, WebSockets & Real-Time

**Status:** Not Started

**Goal:** Production-ready background job system with a declarative Job DSL, pluggable backends, and deep IDD integration — plus WebSocket and SSE support for pushing data to clients. Jobs are first-class language constructs — the `Job` keyword is syntax, not a library import — with the runtime and queue management provided by `std/jobs`.

> Background jobs are essential for any non-trivial web application: sending emails, processing payments, syncing with external APIs, generating reports. NTNT's job system treats jobs as **intentional units of work** rather than just functions to execute, aligning with the IDD philosophy. The `Job` DSL is language-level syntax (like `fn` or `struct`), while the Queue runtime lives in `std/jobs` (like `json()` lives in `std/http/server`). See `design-docs/background_jobs.md` for the full design.

### 10.1 Job DSL & Core Runtime

**Priority:** Foundation — the `Job` declaration syntax and in-memory backend.

```ntnt
/// Sends personalized welcome email to newly registered users
Job SendWelcomeEmail on emails {
    perform(user_id: String) {
        let user = db.find_user(user_id)
        email.send(user.email, "Welcome!", "...")
    }
}

/// Charges customer credit card for completed orders
Job ProcessPayment on payments (retry: 5, timeout: 120s) {
    perform(order_id: String, amount: Float) {
        let order = db.find(order_id)
        stripe.charge(order.customer_id, amount)
    }

    on_failure(error, attempt) {
        alert.notify("Payment failed: {error}")
    }
}

// Enqueue jobs
SendWelcomeEmail.enqueue(map { "user_id": "123" })
ProcessPayment.enqueue_in(3600, map { "order_id": "456", "amount": 29.99 })
```

**Implementation plan:**

- [ ] `Job` declaration syntax in parser (new AST node: `JobDeclaration`)
- [ ] `perform()` handler with typed arguments
- [ ] `on_failure()` hook
- [ ] `Job.enqueue()`, `Job.enqueue_at()`, `Job.enqueue_in()` methods
- [ ] Queue configuration: `Queue.configure(map { "backend": "memory" })`
- [ ] In-memory backend (zero dependencies, default)
- [ ] Worker loop with retry logic and exponential backoff
- [ ] Priority queues (`low`, `normal`, `high`)
- [ ] Dead letter queue for exhausted retries
- [ ] Job cancellation: `Queue.cancel(job_id)`
- [ ] Graceful shutdown (drain in-progress jobs on SIGTERM)
- [ ] Job options: `retry`, `timeout`, `backoff`, `priority`, `rate`, `concurrency`, `unique`, `expires`, `idempotent`
- [ ] Doc comment metadata parsing (`/// Triggers:`, `/// Affects:`, `/// Side effects:`)

### 10.2 Resilience & Production Features

**Priority:** High — required for any production deployment.

- [ ] Worker heartbeats (detect crashed workers)
- [ ] Visibility timeout (re-enqueue stale jobs after no heartbeat)
- [ ] Rate limiting per job type (e.g., `rate: 100/minute`)
- [ ] Concurrency limits per job type
- [ ] Job TTL/expiration (`expires: 5m` — discard stale jobs)
- [ ] Automatic pruning of completed/cancelled jobs
- [ ] Weighted queue processing (prevent starvation of low-priority queues)
- [ ] `Queue.work_async()` for combined HTTP server + worker mode

### 10.3 Persistent Backends

**Priority:** High — in-memory jobs are lost on restart.

```ntnt
import { Queue } from "std/jobs"

// PostgreSQL backend (reliable, ACID, multi-worker)
Queue.configure(map {
    "backend": "postgres",
    "postgres_url": env("DATABASE_URL")
})

// Redis/Valkey backend (high throughput, 10k+ jobs/sec)
Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL")
})
```

- [ ] PostgreSQL backend with auto-migration (`ntnt_jobs` table)
- [ ] Distributed locking via `SELECT FOR UPDATE SKIP LOCKED`
- [ ] Redis/Valkey backend for high-throughput workloads
- [ ] Feature flags to avoid bloating the binary (`jobs-postgres`, `jobs-redis`)
- [ ] Separate worker processes for production: `Queue.work(map { "queues": ["emails", "payments"], "concurrency": 10 })`

### 10.4 Composition (Chains, Workflows, Batches)

**Priority:** Moderate — needed for multi-step business processes.

```ntnt
// Sequential chain — each job receives the previous job's result
Chain ProcessOrder {
    ValidateOrder -> ReserveInventory -> ChargePayment -> SendConfirmation
}
ProcessOrder.start(map { "order_id": "123" })

// DAG workflow — fan-out and fan-in
Workflow UserOnboarding {
    CreateAccount -> SendWelcomeEmail
    CreateAccount -> SetupBilling
    [SendWelcomeEmail, SetupBilling] -> ActivateAccount
}

// Batch — parallel with completion callback
let batch = Batch.create(map {
    "on_complete": fn(results) { db.update_total(sum(results)) },
    "on_failure": fn(errors) { alert("Batch failed") }
})
for chunk in data_chunks { batch.add(ProcessChunk, map { "chunk": chunk }) }
batch.run()
```

- [ ] `Chain` declaration syntax (sequential job pipelines)
- [ ] `Workflow` declaration syntax (DAG dependencies with fan-out/fan-in)
- [ ] `Batch.create()` / `batch.add()` / `batch.run()` API
- [ ] Unique jobs / deduplication (`unique: args for 1h`)
- [ ] Workflow status tracking: `Workflow.status(workflow_id)`

### 10.5 WebSocket Support

**Priority:** High — essential for modern web apps. Live dashboards, chat, notifications, and real-time job status updates all require pushing data to clients.

The assessment identified this as a key missing feature: "there's no way to push data to clients. This limits NTNT to traditional page-based web apps."

```ntnt
import { broadcast, send_to } from "std/ws"

// WebSocket route — handler called per connection
ws("/chat", fn(conn) {
    // Called when a message arrives
    conn.on_message(fn(msg) {
        // Broadcast to all connected clients
        broadcast("/chat", msg)
    })

    conn.on_close(fn() {
        print("Client disconnected")
    })
})

// Send to specific client from anywhere (e.g., from a job)
ws("/jobs/status", fn(conn) {
    // Client subscribes to job updates
    let job_id = conn.params["job_id"]
    conn.on_open(fn() {
        send_to(conn, json(Queue.status(job_id)))
    })
})

// Push from background jobs
Job ProcessPayment on payments {
    perform(order_id: String) {
        // ... process payment ...
        broadcast("/orders/{order_id}", json(map { "status": "paid" }))
    }
}

listen(8080)
```

**Implementation plan:**

- [ ] `ws(pattern, handler)` global builtin for WebSocket routes (mirrors `get`/`post` pattern)
- [ ] Connection object: `conn.on_message()`, `conn.on_open()`, `conn.on_close()`
- [ ] `send_to(conn, msg)` — send to a specific connection
- [ ] `broadcast(channel, msg)` — send to all connections on a channel
- [ ] `std/ws` module for additional utilities (rooms, connection tracking)
- [ ] Integration with background jobs — push job status updates to clients
- [ ] Server-Sent Events (SSE) as a simpler alternative: `sse(pattern, handler)`
- [ ] Connection state management (track connected clients, rooms/channels)
- [ ] Graceful connection cleanup on server shutdown

### 10.6 IDD Integration & CLI

**Priority:** Moderate — testable jobs are NTNT's differentiator over Sidekiq/Bull/Oban.

```intent
Feature: Welcome Email Job
  id: feature.welcome_email_job
  test:
    - job: SendWelcomeEmail
      args: { "user_id": "123" }
      given:
        - mock db.find_user returns { "id": "123", "email": "test@example.com" }
      assert:
        - status: completed
        - email.send was called with "test@example.com"
```

- [ ] Job testing in `.intent` files (`job:` assertion type)
- [ ] Mock support for job dependencies in IDD scenarios
- [ ] `ntnt jobs status` — summary of all queues
- [ ] `ntnt jobs list [--pending|--failed|--dead]` — filter jobs by status
- [ ] `ntnt jobs inspect <job-id>` — full job details
- [ ] `ntnt jobs retry <job-id>` — retry a failed/dead job
- [ ] `ntnt jobs cancel <job-id>` — cancel a pending job
- [ ] `ntnt jobs simulate <JobName> --args='...'` — dry-run without side effects
- [ ] `ntnt jobs replay <job-id>` — re-run with exact same inputs for debugging
- [ ] `--format=json` for agent-consumable output on all commands

### 10.7 Advanced Features (Future)

- [ ] `effect` blocks for explicit side-effect declaration (skipped in simulation mode)
- [ ] Job contracts (`requires(args) { ... }`, `ensures(args, result) { ... }`)
- [ ] Intent verification (`verify()` hook — did the job achieve its purpose, not just run?)
- [ ] Idempotency static analysis in `ntnt lint`
- [ ] Natural language queries: `ntnt jobs ask "why are emails failing?"`
- [ ] AI-powered diagnosis: `ntnt jobs diagnose <job-id>`
- [ ] Request tracing across job chains: `ntnt jobs trace <request-id>`

**Deliverables:**

- `Job`, `Chain`, `Workflow` language-level declaration syntax
- `std/jobs` module with Queue API and worker model
- In-memory, PostgreSQL, and Redis/Valkey backends
- Resilience: heartbeats, retries, dead letter queue, rate limiting, graceful shutdown
- Job composition: chains (sequential), workflows (DAG), batches (parallel)
- WebSocket and SSE support (`ws()` builtin, `broadcast()`, `send_to()`)
- IDD integration for testing jobs in `.intent` files
- `ntnt jobs` CLI commands for monitoring and management
- Simulation mode for dry-run execution

---

## Phase 11: Testing Framework

**Goal:** Comprehensive testing infrastructure complementing Intent-Driven Development.

> IDD tests behavior at the feature level. This phase adds unit testing, mocking, and contract-based test generation for fine-grained code verification.

### 11.1 Unit Test Framework

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

### 11.2 Contract-Based Test Generation

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

### 11.3 Mocking & Test Utilities

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

### 11.4 Test Integration

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

## Phase 12: Tooling & Developer Experience

**Goal:** World-class developer experience with AI collaboration support.

### 12.1 Language Server (LSP)

- [ ] Go to definition
- [ ] Find references
- [ ] Hover documentation
- [ ] Code completion
- [ ] Inline diagnostics
- [ ] Code actions (quick fixes)
- [ ] Contract visualization

### 12.2 Package Registry & Publishing

> **Note:** The package foundation (manifest, local/git dependencies, imports) is built in Phase 9 (Package Ecosystem). This section adds the public registry and publishing infrastructure.

- [ ] Central package registry (hosted service)
- [ ] `ntnt publish` — publish packages to the registry
- [ ] Semantic versioning enforcement
- [ ] Dependency resolution with version ranges (`^1.0`, `~2.3`)
- [ ] `ntnt add <name>` — install from registry (in addition to Phase 9's path/git support)
- [ ] Package search: `ntnt search <query>`

```bash
ntnt new my-app
ntnt add http
ntnt add db/postgres --version "^1.0"
ntnt test
ntnt build --release
```

### 12.3 Documentation Generator

- [ ] Doc comments (`///`)
- [x] Automatic API documentation from TOML source files
- [ ] Contract documentation
- [ ] Example extraction and testing
- [x] `ntnt docs --generate` command
- [x] `ntnt docs [query]` for searching stdlib documentation
- [x] Auto-generated references: STDLIB_REFERENCE.md, SYNTAX_REFERENCE.md, IAL_REFERENCE.md
- [x] CI/CD validation for documentation drift

### 12.4 Human Approval Mechanisms (From Whitepaper)

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

### 12.5 Debugger

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

## Phase 13: Performance & Compilation

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

| Approach                            | Effort     | Speedup   | When       |
| ----------------------------------- | ---------- | --------- | ---------- |
| Tree-walking Interpreter            | ✅ Done    | Baseline  | Current    |
| Bytecode VM                         | 2-4 weeks  | 10-50x    | Phase 13.1 |
| Native Compilation (Cranelift/LLVM) | 2-3 months | 100-1000x | Phase 13.4 |

### What Can Be Reused

| Component   | Reusable?   | Notes                       |
| ----------- | ----------- | --------------------------- |
| Lexer       | ✅ 100%     | Tokens don't change         |
| Parser      | ✅ 100%     | AST structure stays same    |
| AST         | ✅ 100%     | Core data structures        |
| Type System | ✅ 100%     | Expansion for optimization  |
| Interpreter | ❌ Replaced | Becomes compiler/codegen    |
| Stdlib      | ⚠️ Partial  | Need native implementations |

### 13.1 Bytecode VM (First Target)

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

### 13.2 VM Optimizations

- [ ] Constant folding at compile time
- [ ] Dead code elimination
- [ ] Inline caching for method calls
- [ ] Escape analysis for stack allocation
- [ ] Contract elision in release builds (configurable)
- [ ] Hot path detection and optimization

### 13.3 Memory Management

- [ ] Reference counting with cycle detection
- [ ] Memory pools for hot paths
- [ ] String interning
- [ ] Small string optimization
- [ ] Arena allocators for request handling

### 13.4 Native Compilation (Future)

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

### 13.5 Advanced Static Analysis & Contract Inference

> **Note:** Basic type inference and enforcement are in Phase 7.1 (including contract expression type-checking). This section covers deep analysis that builds on the bytecode compiler, including the full contract inference system.

**Contract Inference:**

Contract inference warns when you call a function with contracts without satisfying them. Contracts remain completely optional — inference only activates for contracts that someone chose to write. No contracts on your function? No warnings, no obligations.

```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
{
    return a / b
}

fn compute(x: Int, y: Int) -> Int {
    return divide(x, y)
    //              ^ Warning: `divide` requires `b != 0` but `y` has no such guarantee.
    //                hint: add `requires y != 0` to `compute`, or check before calling.
}
```

- [ ] **Single-level propagation** — warn when calling a `requires` function with an unchecked argument
- [ ] Suggest adding a matching `requires` clause to the caller
- [ ] Recognize common patterns: `if x != 0 { divide(a, x) }` satisfies `requires x != 0`
- [ ] Recognize `match` arms: `Some(v) => use(v)` satisfies `requires v != None`
- [ ] **Transitive propagation** — propagate contracts through entire call chains (A→B→C)
- [ ] Contract static verification (prove contracts hold using SMT solvers or abstract interpretation)
- [ ] Auto-generate `requires` clauses from analysis of function body
- [ ] Contract inference across module boundaries

**Type Analysis:**

- [ ] Flow-sensitive typing (type narrows after null checks)
- [ ] Exhaustive type checking at compile time (full coverage)
- [ ] Type narrowing in conditionals and match arms
- [ ] Escape analysis for optimization hints

### 13.6 Advanced Type System Features

- [ ] Associated types in traits
- [ ] Where clauses for complex constraints
- [ ] Contract inheritance (contracts propagate to trait implementations)
- [ ] Liskov Substitution Principle enforcement
- [ ] Contravariant preconditions, covariant postconditions
- [ ] Error context/wrapping: `result.context("message")?`

### 13.7 Runtime Library (for Native Compilation)

Native compilation requires re-implementing stdlib in the target:

- [ ] Core runtime (memory, strings, arrays, maps)
- [ ] I/O operations (file system, HTTP)
- [ ] Database drivers (PostgreSQL bindings)
- [ ] Concurrency primitives (threads, channels)

### 13.8 Advanced Concurrency

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

## Phase 14: AI Integration & Structured Edits

**Goal:** First-class AI development support—NTNT's key differentiator.

### 14.1 Structured Edits (From Whitepaper)

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

### 14.2 AI Agent SDK

- [ ] Agent communication protocol
- [ ] Context provision API (give AI relevant code context)
- [ ] Suggestion acceptance/rejection tracking
- [ ] Learning from corrections

### 14.3 Semantic Versioning Enforcement

- [ ] API signature tracking across versions
- [ ] Automatic breaking change detection
- [ ] Semver suggestions based on changes
- [ ] `@since` and `@deprecated` annotations

```ntnt
@since("1.2.0")
@deprecated("2.0.0", "Use get_user_by_id instead")
fn get_user(id: String) -> User { }
```

### 14.4 Commit Rationale Generation

- [ ] Structured commit metadata
- [ ] Link commits to intents and requirements
- [ ] Auto-generate changelog entries
- [ ] AI-friendly commit format

### 14.5 AI Agent Optimization

Targeting the specific weaknesses of LLMs: context limits, hallucinations, and safety.

#### 14.5.1 Machine-Readable Diagnostics (`--json` output)

Enable reliable "Self-Correction Loops" for agents.

- [ ] `ntnt check --format=json`
- [ ] `ntnt lint --format=json`
- [ ] Structured errors with remediation suggestions
- [ ] Codes for common agent mistakes (e.g., E023 "Undefined variable")

```json
{
  "file": "server.tnt",
  "line": 45,
  "column": 12,
  "severity": "error",
  "code": "E023",
  "message": "Undefined variable 'usr'",
  "suggestion": {
    "text": "Did you mean 'user'?",
    "replacement": "user",
    "start": 12,
    "end": 15
  }
}
```

#### 14.5.2 Token-Optimized Context (`ntnt describe`)

Provide compressed summaries of the codebase to save tokens and reduce distraction.

- [ ] `ntnt describe src/` command
- [ ] Extracts: Structs, Signatures, Contracts, Imports
- [ ] Strips: Function bodies, comments (unless doc comments)
- [ ] "Searchable Index" for agents to find correct imports

#### 14.5.3 Native "Simulation Mode" (Safety Nets)

Allow agents to execute code safely without side effects on production data.

- [ ] Global `--dry-run` flag
- [ ] `std/env` simulation context check
- [ ] Mocking of side-effecting built-ins (`execute`, `write_file`) in simulation mode

```ntnt
// In std/db
pub fn execute(query, params) {
    if (Global.is_simulation) {
        log("WOULD EXECUTE: " + query);
        return Ok(0);
    }
    // ... real execution
}
```

#### 14.5.4 First-Class `todo` Keyword (Hole-Driven Development)

Allow agents to partially implement features without blocking compilation.

- [ ] `todo` keyword (or `???`)
- [ ] Syntactically valid but panics at runtime
- [ ] Compiler passes `todo` blocks

```ntnt
fn complex_logic(user) {
    if (check_auth(user)) {
        todo "Implement retry logic"
    }
}
```

#### 14.5.5 "Smart Import" Resolution

reduce hallucinated imports by suggesting correct paths.

- [ ] "Smart Linker" in compiler/linter
- [ ] Scans standard library and local modules for missing exports
- [ ] Error message suggests correct path: "Error: `json` not found in `std/http`. Did you mean `std/http/server`?"

**Deliverables:**

- Structured edit engine
- AI agent SDK
- Semantic versioning tools
- Commit rationale system

---

## Phase 15: Deployment & Operations

**Goal:** Production deployment support.

### 15.1 Build & Distribution

- [ ] Single binary compilation
- [ ] Cross-compilation support
- [ ] Minimal Docker image generation
- [ ] Build profiles (dev, release, test)

### 15.2 Configuration

- [ ] Environment-based config
- [ ] Config file support (TOML, JSON)
- [ ] Secrets management patterns
- [ ] Validation with contracts

### 15.3 Observability

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

### 15.4 Graceful Lifecycle

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

### Pipeline Operator (`|>`) → Moved to Phase 7.5

### Response Caching (Server-Side)

In-memory caching for HTTP handler responses. Note: For most use cases, CDN caching via HTTP headers (e.g., `Cache-Control: s-maxage=N` for Cloudflare) is sufficient and preferred. Server-side response caching is only needed for expensive computations that can't be cached at the edge.

- [ ] `std/cache` module with TTL-based caching
- [ ] `cache()` middleware for route handlers
- [ ] Cache key generation from request (path, query params)
- [ ] Manual cache API: `create_cache`, `get_cached`, `set_cached`, `invalidate`

### Effect System (Rebuilt)

> **History:** An effect system was partially implemented in Phase 2.4 (syntax parsing only, no enforcement) and removed in Phase 7.1 as dead code. This section describes a proper rebuild that depends on the static analysis infrastructure from Phase 13.

Effect tracking lets the compiler verify that functions only perform the side effects they declare. A `pure` function can't call an `IO` function. A function that deletes data requires `approval("security")`. The compiler enforces this statically — no runtime cost.

```ntnt
fn read_config(path: String) -> String with io {
    return read_file(path)
}

fn add(a: Int, b: Int) -> Int pure {
    return a + b  // compiler error if this called read_file()
}

@requires_approval("destructive")
fn reset_database(db: Database) with io {
    execute(db, "DROP ALL TABLES")
}
```

**Prerequisites:**
- Phase 7.1: Enforced type system (effect checking extends type checking)
- Phase 13.1+: Bytecode compiler / static analysis passes (effect propagation through call chains)

**Implementation:**
- [ ] Effect inference (auto-detect effects from function body)
- [ ] Effect propagation (if `f` calls `g with io`, then `f` has `io` too)
- [ ] Static enforcement (`pure` functions cannot call `io` functions)
- [ ] `Approval` effect integrated with Human Approval Mechanisms (Phase 12.4)
- [ ] Effect polymorphism (generic functions that preserve caller's effects)
- [ ] Contract interaction (contracts on `pure` functions can be statically verified)

**Why wait:** A real effect system requires analyzing the full call graph statically. The tree-walking interpreter can't do this — it would need the bytecode compiler's static analysis passes to trace effect propagation across function calls, modules, and generics. Building it before that infrastructure exists would repeat the mistake of Phase 2.4: syntax without enforcement.

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
- SQLite (→ moved to Phase 7.4 as priority item)
- Redis client

### High-Performance HTTP Server ✅ PARTIAL

The HTTP server now uses Axum + Tokio for async request handling:

- [x] Async runtime (Tokio) for concurrent connections
- [x] Connection pooling and keep-alive
- [x] Bridge pattern connecting async handlers to sync interpreter
- [ ] HTTP/2 support with multiplexing
- [ ] Request pipelining
- [ ] Zero-copy response streaming
- [ ] Performance target: 100k+ req/sec on commodity hardware

### WebSocket Support → Moved to Phase 10.5

### Concurrency Primitives

- Channels for message passing
- Structured concurrency (task scopes)
- Parallel iterators

---

## Implementation Priority Matrix

| Phase  | Focus                      | Business Value     | Effort   |
| ------ | -------------------------- | ------------------ | -------- |
| 1-3 ✅ | Core Language              | Foundation         | Complete |
| 4 ✅   | Traits + Essentials        | High               | Complete |
| 5 ✅   | Concurrency + Web          | **Critical**       | Complete |
| 6 ✅   | Intent-Driven Dev          | High               | Complete |
| **7**  | **Language Ergonomics**        | **High (Up Next)** | **Medium** |
| **8**  | **Intent System Maturity**     | **High**           | **Medium** |
| **9**  | **Package Ecosystem**          | **Critical**       | **Medium** |
| **10** | **Jobs, WebSockets & Real-Time** | **High**           | **Medium** |
| 11     | Testing Framework          | High               | Medium   |
| 12     | Tooling & DX               | Very High          | High     |
| 13     | Performance                | High               | Medium   |
| 14     | AI Integration             | **Differentiator** | Medium   |
| 15     | Deployment                 | High               | Medium   |

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

### M3: Ergonomic Language (End of Phase 7)

- Enforced type system with inference
- Error propagation (`?` operator)
- Anonymous functions / closures
- SQLite support
- Pipe operator for linear data flow
- Context-rich error messages with suggestions
- Route pattern auto-detection (no `r""` needed)
- Destructuring, default parameters, guard clauses
- Two-layer safety: types (structural) + contracts (semantic)
- A typical web handler drops from ~22 lines to ~6

### M4: Mature Intent System (End of Phase 8)

- Resolution chain visibility in failure output
- Offline intent validation (`ntnt intent validate`)
- Glossary inspector (`ntnt intent glossary`)
- Feature status tracking (planned/implemented/deprecated)
- Decision records for human-agent accountability
- Intent system is a tool agents genuinely rely on

### M5: Extensible Language (End of Phase 9)

- Package manifest (`ntnt.toml`)
- Local and git dependencies
- NTNT-native packages with `lib.tnt` entry points
- Rust FFI for native extensions
- Ecosystem can grow beyond stdlib

### M6: Real-Time & Background Processing (End of Phase 10)

- `Job`, `Chain`, `Workflow` language-level declarations
- `std/jobs` with in-memory, PostgreSQL, and Redis backends
- Resilience: heartbeats, retries, dead letter queue, rate limiting
- WebSocket and SSE support for real-time client communication
- Job testing in `.intent` files
- `ntnt jobs` CLI for monitoring and management

### M7: Developer Ready (End of Phase 12)

- Full IDE support (LSP)
- Package registry and publishing
- Documentation generator
- Human approval workflows
- Comprehensive testing framework (unit + IDD)

### M8: Production Ready / 1.0 (End of Phase 15)

- Performance optimized (bytecode VM, native compilation)
- AI integration complete (structured edits, agent SDK)
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
_Last updated: January 2026 (v0.3.6 — Phases 7-10: Language Ergonomics, Intent System Maturity, Package Ecosystem, Background Jobs)_
