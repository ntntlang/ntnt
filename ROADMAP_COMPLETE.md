# NTNT Language — Completed Roadmap Phases

> These phases are **100% complete** and preserved here for historical reference.
> For active and upcoming work, see [ROADMAP.md](ROADMAP.md).

---

## Phase 1: Core Contract System ✅ COMPLETE

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

## Completed Milestones

### M1: Language Complete (End of Phase 4) ✅

- Traits and polymorphism
- All essential language features
- Comprehensive type system

### M2: Web Ready (End of Phase 5) ✅

- HTTP server running
- Database connectivity
- Can build real web apps

---

_Phases 1-5 completed through v0.3.8. See [ROADMAP.md](ROADMAP.md) for active development._
