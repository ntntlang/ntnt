<!-- NTNT coding guide sections are sourced from docs/AI_AGENT_GUIDE.md -->
<!-- To update NTNT coding instructions, edit AI_AGENT_GUIDE.md and copy to all agent files -->
<!-- Last synced: 2026-03-18 -->

# NTNT Language - Claude Code Instructions

NTNT (pronounced "Intent") is an agent-native programming language for AI-driven web development. File extension: `.tnt`

## OpenClaw Skill (Core Development)

For core NTNT language/runtime development (Rust compiler work, stdlib functions, interpreter changes), load the full OpenClaw skill for deep context:

```
~/.openclaw/skills/ntnt/SKILL.md
```

Read this skill file when working on compiler internals, adding stdlib functions, modifying the interpreter, or any Rust-level changes to the NTNT runtime. It contains comprehensive guidance beyond what this CLAUDE.md covers.

## Building NTNT

```bash
# Fast dev-release build (for development)
cargo build --profile dev-release

# Standard release build (for distribution)
cargo build --release
```

## Quick Start

```bash
ntnt lint file.tnt    # ALWAYS lint first
ntnt run file.tnt     # Run after lint passes
ntnt test server.tnt --get /health  # Test HTTP endpoints
```

## Documentation References

- [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) - Canonical NTNT coding guide (source of truth for all agent files)
- [STDLIB_REFERENCE.md](docs/STDLIB_REFERENCE.md) - All functions (auto-generated from source)
- [SYNTAX_REFERENCE.md](docs/SYNTAX_REFERENCE.md) - Keywords, operators, types, templates
- [IAL_REFERENCE.md](docs/IAL_REFERENCE.md) - Intent Assertion Language

---

<!-- BEGIN NTNT CODING GUIDE (sourced from docs/AI_AGENT_GUIDE.md) -->

## Quick Start

### Installation

```bash
# One-line install (installs Rust if needed, clones repo, builds)
curl -sSf https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash
source "$HOME/.cargo/env"
ntnt --version  # Verify: should print ntnt 0.4.2 or later
```

### Your First App (60 seconds)

Create `hello.tnt`:

```ntnt
import { html } from "std/http/server"

fn home(req) {
    let name = req.query_params["name"] ?? "World"
    return html("<h1>Hello, #{name}!</h1>")
}

get("/", home)
listen(3000)
```

Create `hello.intent`:

```yaml
## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| page loads | status 200 |
| they see "{text}" | body contains "{text}" |

---

Feature: Greeting
  id: feature.greeting

  Scenario: Default greeting
    When a user visits /
    → page loads
    → they see "Hello, World!"

  Scenario: Named greeting
    When a user visits /?name=Alice
    → page loads
    → they see "Hello, Alice!"
```

Verify and run:

```bash
ntnt lint hello.tnt              # Check for errors
ntnt intent check hello.tnt -v   # Verify intent specs pass
ntnt run hello.tnt               # Visit http://localhost:3000
```

### Set Up Your Agent for NTNT Development

After trying NTNT, set up persistent agent knowledge so every future session writes correct code. Add the following to your project's agent configuration (`CLAUDE.md`, `.cursorrules`, `AGENTS.md`, or equivalent):

~~~markdown
# NTNT Language Rules

## Critical Syntax (memorize these — every rule prevents a real mistake)
- Maps REQUIRE `map` keyword: `map { "key": "val" }` — bare `{}` is a code block
- String interpolation: `#{expr}` — hash-brace syntax, never `${expr}` or bare `{expr}`
- Template strings: `"""..{{expr}}.."""` — double braces inside triple quotes
- No semicolons — use newlines to separate statements. `;` silently corrupts the parser.
- Free functions, not methods: `len(s)` not `s.len()`, `trim(s)` not `s.trim()`
- Dot notation reads properties: `req.params.id`, `user.name`
- Mutable variables: `let mut x = 0`
- Ranges: `0..10` — `range()` doesn't exist
- Null coalescing: `expr ?? default`
- Route params auto-detect: `get("/users/{id}", handler)` — no raw strings needed
- HTTP routing functions are GLOBAL builtins — never import get/post/listen/serve_static
- `otherwise` blocks MUST diverge: `otherwise { return x }` ✅ — `otherwise { x }` ❌
- Module-level `let` doesn't support `map {}` literals — use arrays or move map inside a function
- `for..in` skips non-collection values silently — use `chars(s)` to iterate string characters

## Workflow
1. `ntnt lint file.tnt` — ALWAYS lint first
2. `ntnt intent check file.tnt` — verify code matches intent specs
3. `ntnt run file.tnt` — run after lint + intent check pass

## IDD (Intent-Driven Development)
- Write `.intent` files describing features in plain English
- Annotate code with `// @implements: feature.id`
- Verify with `ntnt intent check` — automated testing from natural language specs
- Use `ntnt intent studio file.intent` for live visual feedback

## Full Reference
- Syntax & patterns: `docs/AI_AGENT_GUIDE.md`
- All stdlib functions: `docs/STDLIB_REFERENCE.md`
- Language syntax: `docs/SYNTAX_REFERENCE.md`
~~~

**For agent systems that support skill files** (OpenClaw, Claude Code skills, etc.), create a skill containing the full Critical Syntax Rules and Quick Reference sections from this guide. The more context your agent has, the fewer mistakes it will make on the first try.

---

## Mandatory Workflow

**Always lint before run:**

```bash
ntnt lint myfile.tnt        # Catches 90% of errors
ntnt run myfile.tnt         # Only after lint passes

# For HTTP servers - automated testing
ntnt test server.tnt --get /health --post /users --body 'name=Alice'
```

---

## Type Safety Modes (v0.4.0+)

Two independent axes for type control:

**Runtime (`NTNT_TYPE_MODE`):** Controls behavior on type mismatches at runtime.
- `strict` — crash on mismatch (use for auth/payment apps)
- `warn` — log `[WARN]` and continue **(default)**
- `forgiving` — silent degradation

**Lint (`NTNT_LINT_MODE`):** Controls annotation requirements.
- `default` — only check annotated code **(default)**
- `warn` — also warn about missing annotations (`--warn-untyped`)
- `strict` — missing annotations are errors (`--strict`)

```bash
# Recommended for production apps with auth:
NTNT_TYPE_MODE=strict NTNT_LINT_MODE=strict ntnt run server.tnt

# Development:
NTNT_TYPE_MODE=warn ntnt run server.tnt
```

**Type syntax (v0.4.0+):**
- Optional shorthand: `fn find(id: Int) -> User?` (equivalent to `Optional<User>`)
- Type aliases: `type UserId = Int`, `type Handler = (Request) -> Response`
- Array types: `fn sum(nums: [Int]) -> Int`
- Generics: `fn identity<T>(x: T) -> T` — type checker infers concrete types from call args

**Error messages** include file:line, source snippets, expected/got context, and fix hints.

`NTNT_STRICT` is deprecated — use `NTNT_LINT_MODE=strict`.

---

## Intent-Driven Development (IDD)

IDD is **the core workflow** for NTNT development. Write requirements as `.intent` files, implement with annotations, verify automatically.

### Workflow

1. **Draft** a `.intent` file from user requirements
2. **Present** to user for approval — **do NOT implement before approval**
3. **Implement** with `@implements: feature.id` annotations
4. **Verify** with `ntnt intent check` or `ntnt intent studio`

### Intent File Format

```yaml
# server.intent

## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the page loads | status 200 |
| they see "{text}" | body contains "{text}" |

---

Feature: Home Page
  id: feature.home
  description: "Welcome page for visitors"

  Scenario: Shows welcome message
    When a user visits the home page
    → the page loads
    → they see "Welcome"

---

Constraint: Security Headers
  description: "All pages include security headers"
  applies_to: [feature.home]
```

### Code Annotations

```ntnt
// @implements: feature.home
fn home_handler(req) { return html("<h1>Welcome</h1>") }

// @utility — helper, not a feature
fn hash_password(pw) { ... }
```

### Commands

```bash
ntnt intent check server.tnt       # Verify code matches intent
ntnt intent studio server.intent   # Live visual feedback (opens :3001)
ntnt intent coverage server.tnt    # Feature coverage report
ntnt intent init server.intent     # Generate scaffolding from intent
```

For unit testing individual functions with IAL `call:` syntax, see [IAL_REFERENCE.md](docs/IAL_REFERENCE.md).

---

## Critical Syntax Rules (Common Mistakes)

### 1. Map Literals Require `map` Keyword

```ntnt
// CORRECT
let user = map { "name": "Alice", "age": 30 }
let empty = map {}

// Nested maps are inferred automatically
let config = map {
    "server": { "host": "localhost", "port": 8080 }
}

// WRONG - {} is a block, not a map
let user = { "name": "Alice" }
```

### 2. String Interpolation Uses `#{expr}` NOT `${expr}` or `{expr}`

```ntnt
// CORRECT
let msg = "Hello, #{name}!"

// WRONG
let msg = "Hello, ${name}!"
let msg = "Hello, {name}!"
let msg = `Hello, ${name}!`
```

### 3. Route Patterns Auto-Detect `{param}`

```ntnt
// Route builtins auto-detect {param} as route parameters — no raw strings needed
get("/users/{id}", handler)
post("/api/{category}/items/{id}", handler)

// Raw strings still work (backward compatible)
get(r"/users/{id}", handler)
```

### 4. Contracts Go AFTER Return Type, BEFORE Body

```ntnt
// CORRECT
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}

// WRONG - contracts in wrong position
fn divide(a: Int, b: Int) -> Int {
    requires b != 0  // Inside body - wrong!
}
```

### 5. Range Syntax (Not Function)

```ntnt
// CORRECT
for i in 0..10 { }     // 0-9 exclusive
for i in 0..=10 { }    // 0-10 inclusive

// WRONG
for i in range(10) { }  // range() doesn't exist
```

### 6. Dot Reads, Functions Transform (Use Pipe for Chaining)

NTNT uses a consistent two-part access model:

- **Dot notation** reads properties and fields (accessing what's already there)
- **Free functions** transform data (computing new values)
- **Pipe operator** chains transformations left-to-right

```ntnt
// READING data → dot notation
req.method              // read a property
req.path                // read a property
req.params.id           // read a map key (static key)
req.params["id"]        // read a map key (bracket form — required for dynamic keys or keys with special chars)
req.headers["content-type"]  // bracket form for hyphenated keys
user.name               // read a struct field
config.port             // read a struct field

// TRANSFORMING data → free functions
len("hello")            // compute a value from input
split(text, ",")        // create a new array from a string
trim(input)             // create a new string
push(arr, item)         // create a new array with item added
int(form.age)           // convert a value to a new type

// WRONG - method-style calls on stdlib functions
"hello".len()           // Use len("hello")
arr.push(item)          // Use push(arr, item)
text.split(",")         // Use split(text, ",")
```

**When to use dot vs brackets on maps:**
- **Dot notation** for static keys known at write time: `req.params.id`
- **Bracket notation** for dynamic keys or keys with special characters: `req.headers["content-type"]`, `row[column_name]`

Use the pipe operator `|>` for readable left-to-right data transformations:

```ntnt
import { split, join, trim, to_lower } from "std/string"

// Pipe passes left side as FIRST argument to right side
let result = "  Hello World  " |> trim |> to_lower |> split(" ") |> join("-")
// Equivalent to: join(split(to_lower(trim("  Hello World  ")), " "), "-")

// Works with any function (builtin or user-defined)
fn double(x) { return x * 2 }
let n = 5 |> double  // 10

// Extra arguments: x |> f(a, b) becomes f(x, a, b)
let parts = "a,b,c" |> split(",")  // split("a,b,c", ",")
```

### 7. Mutable Variables Need `mut`

```ntnt
// CORRECT
let mut counter = 0
counter = counter + 1

// WRONG
let counter = 0
counter = 1  // ERROR: immutable
```

### 8. Anonymous Functions / Closures

Use `fn(params) { body }` in expression position for inline callbacks:

```ntnt
// Single-expression body (implicit return)
let double = fn(x) { x * 2 }

// Multi-statement body
let process = fn(item) {
    let cleaned = trim(item)
    return to_lower(cleaned)
}

// With type annotations
let multiply = fn(a: Int, b: Int) -> Int { a * b }

// Inline with higher-order functions
let evens = filter(nums, fn(x) { x % 2 == 0 })
let doubled = transform(nums, fn(x) { x * 2 })

// Closures capture enclosing variables
let threshold = 10
let above = filter(nums, fn(x) { x > threshold })

// Nested closures (currying)
let make_adder = fn(x) { fn(y) { x + y } }
let add5 = make_adder(5)
print(add5(10))  // 15

// Immediate invocation
let result = fn(x) { x + 1 }(5)  // 6

// WRONG - pipe-style lambdas don't exist
let f = |x| x * 2               // Use fn(x) { x * 2 }
```

### 9. No Semicolons — Use Newlines

NTNT uses newlines as statement separators. **Never use semicolons** — `;` silently corrupts parser state and causes errors on unrelated lines.

```ntnt
// CORRECT — newlines separate statements
let x = 1
let y = 2
print(x + y)

// WRONG — semicolons corrupt the parser
let x = 1; let y = 2; print(x + y)
```

### 10. `otherwise` Blocks MUST Diverge

The `otherwise` block must use `return`, `break`, or `continue` — it cannot yield a value directly.

```ntnt
// CORRECT — block diverges with return
let data = parse_json(req) otherwise {
    return status(400, "Bad JSON: #{err}")
}

// CORRECT — single-expression form with return
let user = find(id) otherwise return not_found()

// WRONG — yielding a value without return
let data = parse_json(req) otherwise {
    status(400, "Bad JSON")  // Missing return!
}

// WRONG — return-then-otherwise is a parse error
return parse_json(req) otherwise { return status(400, "err") }
// FIX: use let binding
let data = parse_json(req) otherwise { return status(400, "err") }
return data
```

### 11. Module-Level `let` Doesn't Support `map {}` Literals

Top-level `let` bindings in lib files cannot use `map {}`. Use arrays instead, or move the map inside a function.

```ntnt
// WRONG — fails at module level
let CONFIG = map { "timeout": 30, "retries": 3 }

// CORRECT — use an array of pairs or a function
let CONFIG_KEYS = ["timeout", "retries"]

fn get_config() {
    return map { "timeout": 30, "retries": 3 }
}
```

### 12. `for..in` Skips Non-Collections Silently

Iterating over a non-collection value (string, nil, number) does zero iterations instead of crashing. Use `chars()` for string character iteration.

```ntnt
// WRONG — does nothing, no error
for c in "hello" { print(c) }

// CORRECT — use chars() for string iteration
import { chars } from "std/string"
for c in chars("hello") { print(c) }
```

### 13. Default Parameter Values

Functions and lambdas support default values for parameters. Parameters with defaults must come after all required parameters:

```ntnt
// Basic default
fn greet(name = "World") {
    return "Hello, #{name}!"
}
greet()        // "Hello, World!"
greet("Alice") // "Hello, Alice!"

// Multiple defaults — required params first
fn paginate(items, page = 1, per_page = 25) {
    // items is required, page and per_page are optional
}
paginate("users")           // page=1, per_page=25
paginate("users", 2)        // page=2, per_page=25
paginate("users", 3, 10)    // page=3, per_page=10

// With type annotations
fn add(a: Int, b: Int = 10) -> Int {
    return a + b
}

// Defaults can reference earlier parameters
fn make_range(start = 0, end = start + 10) {
    return "#{start}..#{end}"
}
make_range()     // "0..10"
make_range(5)    // "5..15"

// Works with contracts
fn divide(a, b = 1)
    requires b != 0
{
    return a / b
}

// WRONG - required params after defaults
fn bad(a = 1, b) { }  // Parse error!
```

The type checker infers parameter types from default expressions when no type annotation is provided.

### 14. If-Expressions (Conditional Values)

`if`/`else` can be used in expression position to return a value. Both branches are single expressions, and `else` is required:

```ntnt
// Basic if-expression
let x = if a > b { a } else { b }

// In function arguments
print(if debug { "verbose" } else { "summary" })

// In return statements
return if found { json(data) } else { not_found() }

// Else-if chains
let label = if x > 0 { "positive" } else if x == 0 { "zero" } else { "negative" }

// Nested
let result = if outer { if inner { 1 } else { 2 } } else { 3 }

// WRONG - else is required for if-expressions
let x = if true { 1 }  // ERROR: If-expressions require an else branch
```

### 15. Destructuring Assignment

Map, array, and nested destructuring in `let` bindings, `match`, `for` loops, and function parameters:

```ntnt
// Map destructuring
let { name, age } = map { "name": "Alice", "age": 30 }

// Rename fields
let { name: n } = map { "name": "Alice" }

// Nested destructuring
let { user: { name } } = map { "user": { "name": "Bob" } }

// Works with structs
struct User { name: String }
let u = User { name: "Eve" }
let { name } = u

// Array destructuring
let [a, b, c] = [1, 2, 3]

// Rest patterns with ...
let [first, ...rest] = [1, 2, 3, 4]    // first=1, rest=[2,3,4]
let { name, ...other } = map { "name": "A", "age": 30 }  // other={"age": 30}

// For-loop destructuring
import { entries } from "std/collections"
for [k, v] in entries(data) {
    print("#{k}=#{v}")
}
for { name } in users {
    print(name)
}

// Map destructuring in match
match data {
    { name, age } => print("#{name} is #{age}"),
    _ => print("no match")
}

// Function parameter destructuring
fn greet({ name, email }) {
    print("Hello #{name} (#{email})")
}

fn first_two([a, b, ...rest]) {
    print("#{a}, #{b}")
}

// With type annotation
fn process({ name, email }: Map) {
    print("#{name}: #{email}")
}
```

### 16. Regex Capture Groups

Extract capture groups from regex matches:

```ntnt
import { capture_pattern, capture_all_pattern, capture_named_pattern } from "std/string"

// Single match with groups (index 0 = full match)
let groups = capture_pattern("Bear Lake (1042)", r"(.+) \((\d+)\)")
// groups = Some(["Bear Lake (1042)", "Bear Lake", "1042"])

// All matches with groups
let all = capture_all_pattern("2024-01 and 2025-02", r"(\d{4})-(\d{2})")
// all = [["2024-01", "2024", "01"], ["2025-02", "2025", "02"]]

// Named groups as map keys (use (?P<name>...) syntax)
let m = capture_named_pattern("2024-01-15", r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})")
// m = Some({"0": "2024-01-15", "year": "2024", "month": "01", "day": "15"})
```

---

## HTTP Server Pattern

**CRITICAL:** Routing functions are GLOBAL BUILTINS. Only response builders need importing.

```ntnt
// ONLY import response builders
import { json, html, parse_form, parse_json } from "std/http/server"

// Handler function (named functions recommended for routes)
// Use Request/Response types for fully typed handlers
fn get_user(req: Request) -> Response {
    let id = req.params["id"]
    return json(map { "id": id })
}

// Routes - global builtins, {param} auto-detected
get("/users/{id}", get_user)
post("/users", create_user)

// Static files
serve_static("/static", "./public")

// Server lifecycle
on_shutdown(fn() {
    print("Cleaning up...")
})

listen(8080)  // Starts with hot reload enabled
```

**Request object properties** (type `Request` — all fields typed):
```ntnt
req.method        // String: "GET", "POST"
req.path          // String: "/users/123"
req.params        // Map<String, String>: route params
req.query_params  // Map<String, String>: query string params
req.headers       // Map<String, String>: headers map
req.body          // String: raw body
req.ip            // String: client IP (supports X-Forwarded-For)
req.id            // String: request ID (from X-Request-ID or auto-generated)
```

**Accessing request data** (dot reads properties, brackets for dynamic/special keys):
```ntnt
req.params.id              // dot for static keys
req.params["id"]           // bracket form also works
req.query_params.page      // dot for simple keys
req.headers["content-type"] // brackets required (hyphenated key)
```

**Common mistakes:**
```ntnt
// WRONG - Do NOT import routing functions
import { listen, get, post } from "std/http/server"

// WRONG - Pipe-style lambdas don't exist; use fn() syntax
get("/users/{id}", |req| { ... })
// OK (but named handlers preferred for routes for readability)
get("/health", fn(req) { json(map { "ok": true }) })

// WRONG - These don't exist as properties
req.json       // Use parse_json(req) — a transform function
req.form       // Use parse_form(req) — a transform function
```

### Declarative Server Block Syntax

For cleaner route definitions, use the `server` block syntax:

```ntnt
import { json, html } from "std/http/server"

fn home(req) { return html("<h1>Welcome</h1>") }
fn get_user(req) { return json(map { "id": req.params.id }) }
fn create_user(req) { return json(map { "created": true }, 201) }
fn admin_dashboard(req) { return html("<h1>Admin</h1>") }
fn logger(req) { print("Request: #{req.method} #{req.path}") }

server 8080 {
    static "/assets" from "./public"
    cors map { "origins": ["*"] }
    middleware [logger]

    GET / -> home
    GET /users/{id: Int} -> get_user
    POST /users -> create_user

    group "/admin" {
        middleware [require_admin]
        GET / -> admin_dashboard
    }
}
```

**Key features:**
- **Typed route parameters**: `{id: Int}` validates the parameter is an integer, returning 400 Bad Request on type mismatch
- **Route groups**: `group "/prefix" { ... }` groups routes with a common prefix
- **Directives**: `static`, `cors`, `middleware` configure server behavior
- **Route conflict detection**: Ambiguous routes like `GET /users/{id}` and `GET /users/{name}` are detected at startup

**Typed parameters:**
| Type | Validation |
|------|------------|
| `Int` | Must be a valid integer |
| `Float` | Must be a valid float |
| (none) | String (no validation) |

### Environment Variables

| Variable | Values | Description |
|----------|--------|-------------|
| `NTNT_ENV` | `production`, `prod` | Disables hot-reload for better performance |
| `NTNT_STRICT` | `1`, `true` | Blocks execution on type errors (runs type checker before `ntnt run`) |
| `NTNT_ALLOW_PRIVATE_IPS` | `true` | Allows `fetch()` to connect to private/internal IPs (see below) |
| `NTNT_WORKERS` | `1` (dev) / CPU cores (prod) | Number of interpreter worker threads. Auto-scales in production. |

```bash
# Development (default) - hot-reload enabled, single worker
ntnt run server.tnt

# Production - hot-reload disabled, multi-worker
NTNT_ENV=production ntnt run server.tnt

# Custom worker count
NTNT_WORKERS=4 ntnt run server.tnt
```

**Hot-reload** watches your `.tnt` files and imported modules for changes, automatically reloading on the next request. Disable in production for zero filesystem overhead per request.

### SSRF Protection (Private IP Blocking)

By default, `fetch()` blocks requests to private/internal IP ranges (`10.x`, `172.16-31.x`, `192.168.x`, `127.x`, `localhost`). This prevents Server-Side Request Forgery attacks.

**In Docker**, this blocks inter-container communication (e.g., calling a sidecar service at `172.19.0.1:8889`). Set `NTNT_ALLOW_PRIVATE_IPS=true` to allow it:

```yaml
# docker-compose.yml
services:
  ntnt:
    environment:
      - NTNT_ALLOW_PRIVATE_IPS=true
```

⚠️ Only enable this when your app needs to call internal services. Keep disabled in public-facing apps that don't need internal network access.

### Response Builder Functions

All response builders are imported from `std/http/server`:

| Function | Description | Example |
|----------|-------------|---------|
| `json(data, status?)` | JSON response (default 200) | `json(map { "ok": true })` |
| `html(content, status?)` | HTML response | `html("<h1>Hello</h1>")` |
| `text(content, status?)` | Plain text response | `text("OK")` |
| `redirect(url, status?)` | Redirect (default 302) | `redirect("/login")` |
| `status(code, body)` | Custom status with body | `status(404, "Not found")` |
| `not_found(body?)` | 404 response | `not_found("Page not found")` |
| `error(body?)` | 500 response | `error("Server error")` |

**Low-level response function:**

For full control, use `response(status, headers, body)`:

```ntnt
import { response } from "std/http/server"

fn custom_handler(req) {
    return response(
        201,
        map { "Content-Type": "application/json", "X-Custom": "value" },
        "{\"created\": true}"
    )
}
```

### HTTP Client (`std/http`)

`fetch()` accepts one or two arguments:

```ntnt
import { fetch, download } from "std/http"

// Simple GET (string argument)
let resp = fetch("https://api.example.com/data")

// POST with options map (url inside the map)
let resp = fetch(map {
    "url": "https://api.example.com/users",
    "method": "POST",
    "json": map { "name": "Alice", "age": 30 }
})

// Two-argument form: URL + options (v0.4.4+)
let resp = fetch("https://api.example.com/users", map {
    "method": "POST",
    "json": map { "name": "Alice", "age": 30 }
})

// POST with form body
let resp = fetch(map {
    "url": "https://api.example.com/submit",
    "method": "POST",
    "form": map { "field": "value" }
})

// Custom headers
let resp = fetch("https://api.example.com/data", map {
    "headers": map { "Authorization": "Bearer #{token}" }
})
```

Both `fetch(map { "url": url, ... })` and `fetch(url, map { ... })` work. The two-argument form is typically more natural.

Use `"json": map{...}` for JSON POST or `"form": map{...}` for form POST — auto-encodes and sets Content-Type.

### CORS (Cross-Origin Resource Sharing)

Configure CORS with `enable_cors()` — works on both sync and async (production) server paths:

```ntnt
// Allow all origins (default)
enable_cors()

// Restrict to specific origins
enable_cors(map {
    "origins": ["https://example.com", "https://app.example.com"],
    "methods": ["GET", "POST", "PUT", "DELETE"],
    "headers": ["Content-Type", "Authorization"],
    "credentials": true,
    "max_age": 86400
})
```

Also available as a directive in server blocks: `cors map { "origins": ["*"] }`.

### Content Security Policy (CSP)

Configure CSP with `enable_csp()` (v0.4.4+):

```ntnt
// Sensible defaults (restrictive but practical)
enable_csp()

// Custom directives
enable_csp(map {
    "default-src": "'self'",
    "script-src": "'self' 'unsafe-inline'",
    "style-src": "'self' 'unsafe-inline' https://fonts.googleapis.com",
    "font-src": "'self' https://fonts.gstatic.com",
    "img-src": "'self' data: https:",
    "connect-src": "'self'",
    "frame-ancestors": "'none'"
})

// Disable CSP
enable_csp(false)
```

Default directives: `default-src 'self'`, `script-src 'self'`, `style-src 'self' 'unsafe-inline'`, `img-src 'self' data: https:`, `font-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`, `base-uri 'self'`, `form-action 'self'`.

CSP is applied independently of `NTNT_SECURITY_HEADERS` — disabling security headers does not affect CSP configured via `enable_csp()`.

---

## Design by Contract

Use contracts to specify function behavior - they become automatic validation in HTTP routes:

```ntnt
// In HTTP routes:
// - Failed requires → 400 Bad Request
// - Failed ensures → 500 Internal Server Error

fn create_user(req)
    requires len(req.body) > 0
    ensures result.status == 201 || result.status == 400
{
    let form = parse_form(req)
    let name = form["name"]

    if len(name) < 2 {
        return json(map { "error": "Name too short" }, 400)
    }

    return json(map { "created": true }, 201)
}
```

**Type Checking:** Contract expressions are statically checked by `ntnt lint`:
- `requires` and `ensures` clauses must evaluate to `Bool`
- In `ensures`, `result` is typed to the function's return type
- `old(expr)` returns the same type as `expr`
- Struct invariants are checked with field types in scope

---

## Error Handling with Result/Option

### The `?` Operator (Error Propagation)

The `?` operator unwraps `Ok`/`Some` values or early-returns `Err`/`None` from the enclosing function:

```ntnt
// ? flattens nested match pyramids into linear code
fn process_request(req) {
    let data = parse_json(req)?          // Err → early-return Err
    let valid = validate(data)?          // Err → early-return Err
    let result = save_to_db(valid)?      // Err → early-return Err
    return Ok(json(result))
}

// Also works with Option
fn find_user_email(id) {
    let user = find_user(id)?            // None → early-return None
    let email = user_email(user)?        // None → early-return None
    return Some(email)
}
```

**Behavior:**
- `Ok(v)?` → evaluates to `v`
- `Err(e)?` → early-returns `Err(e)` from the enclosing function
- `Some(v)?` → evaluates to `v`
- `None?` → early-returns `None` from the enclosing function
- Non-Result/Option values pass through unchanged (gradual typing)

### The `??` Operator (Null Coalescing)

The `??` operator provides a default value when the left side is `None`:

```ntnt
// Map access returns None for missing keys — ?? provides a default
let name = user["name"] ?? "Anonymous"
let port = get_env("PORT") ?? "8080"

// Replaces verbose get_or() pattern:
// Before: let name = get_or(user, "name", "Anonymous")
// After:  let name = user["name"] ?? "Anonymous"

// Chain with ? for Result<Option<T>> (e.g., database queries):
// pg_query_one returns Result<Option<Map>>
// First ? unwraps Result (early-returns Err)
// Second ? unwraps Option (early-returns None)
fn get_user(id) {
    let row = pg_query_one(pg, "SELECT * FROM users WHERE id = $1", [id])? ?
    return Some(row)
}
```

**Behavior:**
- `Some(v) ?? default` → `v` (unwrapped)
- `None ?? default` → `default`
- Non-Option values pass through unchanged

### The `otherwise` Keyword (Inline Error Handling)

`otherwise` unwraps `Ok`/`Some` or runs a diverging block for `Err`/`None`. It also catches runtime errors (type mismatches, arithmetic errors, etc.) from the expression, converting them to `Err` values so the otherwise block can handle them. Unlike `?`, it handles errors at the call site with custom recovery logic:

```ntnt
// Block form — err is automatically bound to the error value
fn create_user(req) {
    let data = parse_json(req) otherwise {
        return status(400, "Invalid JSON: #{err}")
    }

    let saved = execute(db, "INSERT INTO users (name) VALUES ($1)", [data["name"]]) otherwise {
        return status(500, "Database error: #{err}")
    }

    return json(map { "created": true }, 201)
}

// Single-expression form (no braces needed)
fn get_user(req) {
    let user = find_user(req.params.id) otherwise return not_found("User not found")
    return json(user)
}

// Catches runtime errors too — no more unhandled crashes
fn safe_compute(req) {
    let result = (some_value * 33) otherwise {
        return json(map { "error": "Computation failed: #{err}" }, 400)
    }
    return json(map { "result": result })
}

// In loops — use continue to skip, break to stop
for line in lines {
    let value = parse_line(line) otherwise {
        print("Skipping bad line: #{err}")
        continue
    }
    process(value)
}
```

**Behavior:**
- `Ok(v)` / `Some(v)` → binds `v` to the variable
- `Err(e)` / `None` → runs the otherwise block with `err` bound to `e` (or `Unit` for None)
- Runtime errors (type mismatches, etc.) → caught and converted to `Err`, then handled by the otherwise block with `err` bound to the error message
- The otherwise block **must diverge**: `return`, `break`, `continue`, or call a function that doesn't return
- Non-Result/Option values bind as-is (gradual typing)
- In dev mode, caught runtime errors emit a `[WARN]` to stderr for visibility

### When to Use Each

| Pattern | Use When |
|---------|----------|
| `?` operator | Propagating errors to the caller (library/internal code) |
| `otherwise` | Handling errors with specific recovery at the call site |
| `match` | Complex branching on multiple variants |
| `unwrap()` | Quick prototyping (panics on error) |

### Match for Explicit Handling

```ntnt
import { connect, query } from "std/db/postgres"

// Using match for explicit handling
let result = connect("postgres://...")
match result {
    Ok(db) => {
        // Use the connection
        let users = query(db, "SELECT * FROM users", [])
        match users {
            Ok(rows) => print("Found #{len(rows)} users"),
            Err(e) => print("Query failed: #{e}")
        }
    },
    Err(e) => print("Connection failed: #{e}")
}

// Using unwrap for quick prototyping (panics on error)
let db = unwrap(connect("postgres://..."))
let users = unwrap(query(db, "SELECT * FROM users", []))
```

---

## Type System

NTNT uses **gradual typing** — type annotations are optional, and untyped code continues to work as before. When annotations are present, the type checker catches errors at lint time.

### Type Annotations

```ntnt
// Variable annotations
let name: String = "Alice"
let age: Int = 30
let scores: Array<Float> = [9.5, 8.2, 7.8]

// Function parameter and return types
fn greet(name: String) -> String {
    return "Hello, #{name}!"
}

// Default parameter values (with or without type annotations)
fn connect(host: String = "localhost", port: Int = 5432) -> String {
    return "#{host}:#{port}"
}

// No annotations required — these work fine
let x = 42
fn add(a, b) { return a + b }
```

### Available Types

| Type | Description | Example |
|------|-------------|---------|
| `Int` | Integer | `let x: Int = 42` |
| `Float` | Floating-point | `let x: Float = 3.14` |
| `Bool` | Boolean | `let x: Bool = true` |
| `String` | String | `let x: String = "hi"` |
| `Unit` | No value | Return type of `print()` |
| `Array<T>` | Array of type T | `let x: Array<Int> = [1, 2, 3]` |
| `Map<K, V>` | Map with typed keys/values | `let m: Map<String, Int>` |
| `Option<T>` | Optional value | `Some(42)` or `None` |
| `Result<T, E>` | Success or error | `Ok(value)` or `Err(msg)` |
| `Request` | HTTP request object | `fn handler(req: Request)` |
| `Response` | HTTP response object | `fn handler(req: Request) -> Response` |
| `T1 \| T2` | Union type | `Int \| String` |

### Generic-Aware Type Inference

The type checker tracks types through common operations:

- **`unwrap()`** — `unwrap(Optional<T>)` → `T`, `unwrap(Result<T, E>)` → `T`
- **Collection functions preserve element types** — `filter()`, `sort()`, `reverse()`, `slice()`, `concat()`, `push()` return `Array<T>` when given `Array<T>`
- **Element accessors return element type** — `first()`, `last()`, `pop()` on `Array<T>` return `T`
- **`flatten()`** — `flatten(Array<Array<T>>)` → `Array<T>` (unwraps one level)
- **Math functions preserve numeric type** — `abs()`, `min()`, `max()`, `clamp()` return `Int` or `Float` based on input
- **Map accessors return typed results** — `keys(Map<K, V>)` → `Array<K>`, `values(Map<K, V>)` → `Array<V>`, `get_key(Map<K, V>, key)` → `V`
- **Map index access** — `map["key"]` on `Map<K, V>` returns `V`
- **`transform()` infers callback return** — `transform(Array<T>, fn(T)->R)` → `Array<R>` when callback is a typed named function
- **`html()`, `json()`, `text()`, `redirect()`** — all return `Response`
- **`parse_json()`** — returns `Result<Map<String, Any>, String>` (unwrap gives a map). JSON `null` becomes `None`.
- **`fetch()`** — returns `Result<Response, String>` (unwrap gives `Response`)
- **`parse_datetime()`** — returns `Result<Int, String>`
- **`parse_csv()`** — returns `Array<Array<String>>`
- **Match arm narrowing** — `Ok(data)` on `Result<T, E>` binds `data` as `T`; `Some(x)` on `Option<T>` binds `x` as `T`; struct patterns bind field types
- **Cross-file imports** — `import { foo } from "./lib/utils"` resolves function signatures from the imported `.tnt` file
- **Circular import detection** — if files form an import cycle (e.g. `a.tnt → b.tnt → a.tnt`), a warning is emitted showing the exact chain

### What the Type Checker Catches

The type checker runs during `ntnt lint` and `ntnt validate`, and reports:

- **Argument type mismatches**: passing `Int` where `String` is expected
- **Wrong argument count**: calling `f(a, b)` with one argument
- **Return type mismatches**: returning `String` from a function declared `-> Int`
- **Let binding mismatches**: `let x: Int = "hello"`

```ntnt
fn greet(name: String) -> String {
    return "Hello, #{name}!"
}

greet(42)  // Type error: expected String, got Int
```

### What It Does NOT Catch (Gradual Typing)

- Untyped parameters default to `Any` — compatible with everything
- Functions without return type annotations skip return checking
- Cross-file types from imported `.tnt` modules use `Any`
- No flow-sensitive narrowing (e.g., checking `None` before access)

**This means existing untyped code produces zero type errors.**

Type checking strictness is controlled by `NTNT_TYPE_MODE` and `NTNT_LINT_MODE` — see [Type Safety Modes](#type-safety-modes-v040) above.

---

## Database Pattern

### SQLite (bundled, no server needed)

```ntnt
import { connect, query, execute, close } from "std/db/sqlite"

let db = unwrap(connect("app.db"))        // File-based
let db = unwrap(connect(":memory:"))      // In-memory

// Create tables
execute(db, "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT, age INTEGER)", [])

// Parameterized queries (? placeholders)
execute(db, "INSERT INTO users (name, age) VALUES (?, ?)", ["Alice", 30])
let users = unwrap(query(db, "SELECT * FROM users WHERE age > ?", [18]))
for user in users {
    print("Name: #{user[\"name\"]}")
}

close(db)
```

### PostgreSQL (Connection Pooled)

`connect()` returns a pooled connection handle (via deadpool-postgres). Connections are automatically managed — you don't need to worry about pool sizing or checkout/checkin.

```ntnt
import { connect, query, execute, close } from "std/db/postgres"

let db = unwrap(connect("postgres://user:pass@localhost/mydb"))

// Parameterized queries ($1, $2 placeholders)
let users = unwrap(query(db, "SELECT * FROM users WHERE active = $1", [true]))
for user in users {
    print("Name: #{user[\"name\"]}")
}

execute(db, "INSERT INTO users (name, age) VALUES ($1, $2)", [name, int(age_str)])

close(db)  // Releases the connection pool
```

**Type conversion for database:**
```ntnt
let form = parse_form(req)
let age = int(form["age"])     // Convert string to int!
let price = float(form["price"])

// WRONG - String to integer column causes "db error"
execute(db, "INSERT INTO users (age) VALUES ($1)", [form["age"]])
```

**JSONB and UUID bind parameters** (v0.4.4+): Strings are automatically coerced to the correct binary format when the target column is JSONB, JSON, or UUID. No manual casting needed:
```ntnt
// JSONB — string is parsed as JSON automatically
let json_str = "{\"key\": \"value\"}"
execute(db, "INSERT INTO settings (data) VALUES ($1)", [json_str])  // Just works

// UUID — string is parsed as UUID automatically
let id = uuid()
execute(db, "INSERT INTO users (id) VALUES ($1)", [id])  // Just works

// No need for ::text::jsonb or ::text::uuid double casts
```

**NULL handling:** SQL NULL values are returned as `None` (not `Unit`) in query results. Use `None` when inserting NULL values:
```ntnt
// Reading NULL from database
let user = unwrap(query_one(db, "SELECT * FROM users WHERE id = ?", [1]))
match user["middle_name"] {
    None => print("No middle name"),
    Some(name) => print(name),
    name => print(name)  // also works with gradual typing
}

// Inserting NULL
execute(db, "INSERT INTO users (name, age) VALUES (?, ?)", ["Alice", None])

// query_one returns Ok(None) when no row matches
let result = query_one(db, "SELECT * FROM users WHERE id = ?", [999])
match result {
    Ok(Some(row)) => print("Found: #{row}"),
    Ok(None) => print("No row found"),
    Err(e) => print("Query error: #{e}")
}
```

---

## Template Strings

Triple-quoted with `{{expr}}` interpolation (CSS-safe):

```ntnt
let page = """
<style>h1 { color: blue; }</style>
<h1>Hello, {{name}}!</h1>

{{! This is a comment — not rendered }}

{{#for item in items}}
<p>{{@index1}}. {{item.name}}: ${{item.price}}</p>
{{#empty}}
<p>No items found.</p>
{{/for}}

{{#if status == "active"}}
<span class="active">Active</span>
{{#elif status == "draft"}}
<span class="draft">Draft</span>
{{#else}}
<span>Unknown</span>
{{/if}}
"""
```

**Output modes:**
- `{{expr}}` — HTML-escaped output
- `{{{expr}}}` — Raw/unescaped output (use for HTML content like layout slots)

**Optional variables:** Undefined variables render as empty string (not an error). Undefined vars in `{{#if}}` are falsy.

**Comparisons in conditions:** `{{#if x == "val"}}`, `{{#if count > 0}}` — full expression support.

**Filters:** `uppercase`/`upper`, `lowercase`/`lower`, `capitalize`, `trim`, `truncate(n)`, `replace(old, new)`, `escape`, `raw`/`safe`, `default(val)`, `length`, `first`, `last`, `reverse`, `join(sep)`, `slice(start, end)`, `json`, `number(decimals)`, `url_encode`

Filter args use parens or spaces: `{{var | truncate(100)}}` or `{{var | default "N/A"}}`

**Loop metadata:** `@index` (0-based), `@index1` (1-based), `@length`, `@first`, `@last`, `@even`, `@odd`

**Partials:** Include reusable template fragments with `{{> name}}`:

```ntnt
{{! Include a partial — inherits current scope variables }}
{{> header}}

{{! Include with explicit data }}
{{> card map { "title": item.name, "desc": item.summary } }}
```

Partial resolution order (relative to project root):
1. `views/partials/{name}.html`
2. `views/partials/{name}` (if name includes extension)
3. `views/{name}.html`
4. `{name}.html` (relative to script)
5. `{name}` (exact path relative to script)

Without a data expression, partials inherit all variables from the current scope. With a data expression (must be a map), only that data is available inside the partial.

---

## External Templates

```ntnt
let page = template("views/home.html", map {
    "title": "Welcome",
    "items": items
})
return html(page)
```

Template paths are relative to the `.tnt` file.

**Important:** External template files (`.html`) are rendered internally by wrapping their content in `"""..."""` triple quotes. This means template HTML **must not contain literal `"""`** anywhere in the content — the lexer will interpret it as the closing delimiter and truncate the output. If you need to display triple quotes (e.g., in code examples showing Elixir's `@doc """`), use HTML entities `&quot;&quot;&quot;` instead. They render identically in the browser.

### Cache Busting for Static Assets

Use `file_stat` to automatically bust browser/CDN caches based on actual file modification time. Compute it once at module load in your layout lib, then pass it to templates:

```ntnt
// lib/layout.tnt
import { file_stat } from "std/fs"
import { now } from "std/time"

// Computed once at server start — every deploy/restart = new value
// Handles: file missing, mtime unavailable (returns 0), strict type mode
let stat_result = file_stat("public/css/styles.css")
let CACHE_BUST = if is_ok(stat_result) {
    let m = unwrap(stat_result)["modified"]
    if m > 0 { m } else { int(now()) }
} else { int(now()) }

fn render_page(options) {
    // ... build nav, footer, etc.
    return template("views/layout.html", map {
        "title": options["title"] ?? "My App",
        "content": options["content"] ?? "",
        "cache_bust": CACHE_BUST
    })
}
```

```html
<!-- views/layout.html -->
<link rel="stylesheet" href="/assets/css/styles.css?v={{cache_bust}}">
<script src="/assets/js/app.js?v={{cache_bust}}"></script>
```

This eliminates manual `?v=N` bumping. The timestamp changes on every server restart, which in Docker deployments happens on every build.

---

## File-Based Routing

```ntnt
routes("routes")   // Auto-discover from directory
listen(8080)
```

```
routes/
├── index.tnt          # GET /
├── api/
│   ├── users.tnt      # GET/POST /api/users
│   └── [id].tnt       # GET /api/:id (dynamic segment)
```

Route files export `get`, `post`, etc. functions.

---

## Middleware

```ntnt
// Global middleware applied to all routes
use_middleware(fn(req) {
    print("Request: #{req.method} #{req.path}")
    // Return nothing to continue, return response to short-circuit
})

// Middleware for authentication
use_middleware(fn(req) {
    if starts_with(req.path, "/api/") {
        let token = req.headers["authorization"]
        if !is_valid_token(token) {
            return json(map { "error": "Unauthorized" }, 401)
        }
    }
})
```

### Middleware Request Context

Every request includes an empty `context` map that middleware can populate. Use `merge()` to pass data (like the authenticated user) to downstream handlers — this avoids re-fetching in every route.

```ntnt
import { merge, get_or } from "std/collections"

// middleware/01_auth.tnt
fn middleware(req) {
    let session = get_session(req)
    if is_none(session) {
        return merge(req, map { "context": map { "user": None } })
    }
    let user = get_user_by_session(unwrap(session))
    return merge(req, map { "context": map { "user": user } })
}
```

```ntnt
// routes/admin/index.tnt — handler reads from context
fn get(req) {
    let user = get_or(req.context, "user", None)
    if is_none(user) {
        return redirect("/login")
    }
    // user is available — no need to re-fetch from session
    template("views/admin.html", map { "user": unwrap(user) })
}
```

**Rules:**
- Middleware must `return merge(req, map { "context": ... })` to pass context forward
- Returning a map with `"status"` key short-circuits (treated as HTTP response)
- Returning `Unit` (nothing) passes the original request unchanged
- Context keys are convention-based — use `"user"`, `"permissions"`, `"feature_flags"`, etc.

---

## Authentication (`std/auth`)

Full OAuth, session management, CSRF, JWT, and TOTP support — 34 functions.

### Basic OAuth Setup

```ntnt
import { oauth, enable_auth, get_user, logout_user } from "std/auth"
import { json, html, redirect } from "std/http/server"
import { get_env } from "std/env"

let google = oauth("google", get_env("GOOGLE_CLIENT_ID"), get_env("GOOGLE_CLIENT_SECRET"))

enable_auth([google], map {
    "session_secret": get_env("SESSION_SECRET"),
    "session_store": "redis://localhost:6379",     // or "sqlite:./sessions.db"
    "session_ttl": 86400 * 7,                      // 7 days
    "login_url": "/auth/login",
    "logout_url": "/",
    "callback_url": "/auth/callback"
})
```

### Protecting Routes

```ntnt
import { get_user, validate_csrf } from "std/auth"

fn dashboard(req) {
    let user = get_user(req) otherwise return redirect("/auth/login")
    return html(template("dashboard.html", map { "user": user }))
}

fn update_settings(req) {
    let user = get_user(req) otherwise return redirect("/auth/login")
    let csrf_ok = validate_csrf(req)
    if typeof(csrf_ok) == "Map" { return csrf_ok }  // Returns 403 if invalid
    // ... handle form
}
```

### Session Data

```ntnt
import { get_user, get_session, set_session, session_data } from "std/auth"

// Store custom data in session
set_session(req, map { "theme": "dark", "role": "admin" })

// Retrieve session data
let data = session_data(req)  // Returns the custom data map

// get_user(req) returns Option<User> with: name, email, picture, provider, raw, csrf_token
// get_session(req) returns Option<Session> with full session including tokens and timestamps
```

### CSRF Protection in Forms

```html
<form method="POST" action="/settings">
    <input type="hidden" name="_csrf_token" value="{{user.csrf_token}}">
    <!-- form fields -->
</form>
```

### JWT (Stateless Tokens)

```ntnt
import { jwt_sign, jwt_verify } from "std/auth"

let token = unwrap(jwt_sign(map { "user_id": 42, "role": "admin" }, "secret", map { "exp": 3600 }))
let claims = unwrap(jwt_verify(token, "secret"))
// claims = { "user_id": 42, "role": "admin", "exp": ..., "iat": ... }
```

### Built-in Auth Routes

`enable_auth()` automatically registers these routes:
- `GET /auth/login` — Redirect to OAuth provider
- `GET /auth/callback` — Handle OAuth callback
- `GET /auth/logout` — Clear session and redirect
- `GET /auth/me` — JSON user info (for SPAs)

---

## Key-Value Store (`std/kv`)

Redis/Valkey or SQLite-backed key-value store with TTL support.

```ntnt
import { open, get, set, del, has, list, expire, ttl } from "std/kv"

// Connect (Redis or SQLite)
let kv = unwrap(open("redis://localhost:6379"))
let kv = unwrap(open("sqlite:./cache.db"))

// Basic operations
set(kv, "user:1", map { "name": "Alice", "role": "admin" })
let user = unwrap(get(kv, "user:1"))    // Option<Any> — returns None if missing

// TTL (time-to-live)
set(kv, "session:abc", data, map { "ttl": 3600 })  // Expires in 1 hour
expire(kv, "user:1", 86400)                         // Set TTL on existing key
let remaining = unwrap(ttl(kv, "session:abc"))       // Seconds remaining

// Check and list
let exists = unwrap(has(kv, "user:1"))               // Bool
let keys = unwrap(list(kv, "user:"))                  // All keys with prefix "user:"

// Delete
del(kv, "session:abc")
```

Values are automatically serialized — maps and arrays are stored as JSON.

---

## Logging (`std/log`)

Structured logging with configurable levels.

```ntnt
import { log_info, log_warn, log_error, log_debug, set_log_level, request_logger } from "std/log"

set_log_level("info")  // "debug", "info", "warn", "error"

log_info("Server started", map { "port": 8080 })
log_warn("Rate limit approaching", map { "current": 95, "max": 100 })
log_error("Database connection failed", map { "host": "localhost", "error": err })
log_debug("Cache hit", map { "key": "user:42" })

// Request logging middleware
use_middleware(request_logger())  // Logs method, path, status, duration for every request
```

---

## Concurrency (`std/concurrent`)

Structured concurrency: tasks, channels, schedules, and cooperative cancellation.

### Channels

```ntnt
import { channel, send, recv, recv_timeout, try_recv, close } from "std/concurrent"

// channel() returns [TxChannel, RxChannel] — always destructure
let [tx, rx] = channel()
send(tx, "hello")                // Returns true (false if receiver closed)
let msg = recv(rx)               // Blocks until value available; returns Unit on disconnect
let msg = recv_timeout(rx, 5000) // Option — None on timeout or disconnect
let msg = try_recv(rx)           // Option — None if no message waiting
close(rx)                        // Removes receiver from registry; future send() → false
```

**Two-handle design:** `tx` (TxChannel) is the sender; `rx` (RxChannel) is the receiver. When all `tx` clones drop (e.g. a spawned task exits), `recv(rx)` automatically returns `Unit` — no sentinel needed. Mirrors Rust's own channel ownership semantics.

Channels are single-consumer: only one task should call `recv()` at a time.

**Serializable types for send/recv:** Int, Float, Bool, String, Array, Map, Struct, Enum.

### Tasks (spawn/await)

```ntnt
import { spawn, await_task, try_await, cancel_task, sleep_ms } from "std/concurrent"

// Handler must be zero-parameter (no params, including no defaults)
let task = spawn(fn() {
    sleep_ms(100)
    42
})

// await_task blocks and returns Result, then marks the task as consumed
// The handle remains valid for try_await, which returns {status: "consumed"}
let result = await_task(task)  // Ok(42) or Err("message")
match result {
    Ok(val) => print("got: " + str(val)),
    Err(e) => print("error: " + str(e))
}

// try_await peeks without removing (returns Map with status + result)
let status = try_await(task)
// { "status": "running"|"completed"|"failed"|"panicked"|"consumed"|"expired", "result": Ok(val)|Err(msg)|None }

// cancel_task sets cooperative cancellation flag (checked at yield points)
cancel_task(task)  // Task exits at next recv/recv_timeout/sleep_ms/fetch call
```

### Delayed Execution

```ntnt
import { after, await_task } from "std/concurrent"

let task = after(1000, fn() { "delayed" })       // 1000ms delay
let task = after("5s", fn() { "five seconds" })  // String interval: ms, s, m, h
let result = await_task(task)                     // Result<Any, String>
```

### Scheduled Execution

```ntnt
import { schedule, cancel_schedule, sleep_ms } from "std/concurrent"

// Zero-duration intervals are rejected
let sched = schedule("5s", fn() {
    print("tick")
})

sleep_ms(30000)
cancel_schedule(sched)  // Sets flag AND removes from registry
```

Schedule ticks run in separate threads with `catch_unwind` — panics don't kill the schedule.
Overlap prevention: a new tick won't start until the previous one finishes.

### Cancellation Yield Points

These functions check the cooperative cancellation flag:
- `recv()`, `recv_timeout()` — from `std/concurrent`
- `sleep_ms()` — from `std/concurrent`
- `fetch()` — from `std/http`

**Important:** `sleep()` from `std/time` is NOT cancellation-aware. Use `sleep_ms()` from `std/concurrent` in spawned tasks.

### Timing

```ntnt
import { sleep_ms, thread_count } from "std/concurrent"

sleep_ms(1000)         // Cancellation-aware sleep (50ms slices)
let cpus = thread_count()  // Available CPU threads
```

---

## Background Jobs (`std/jobs`)

Persistent background job processing with retry, backoff, and scheduled execution.

### Defining Jobs

```ntnt
job SendEmail on emails (retry: 5, backoff: "exponential") {
    perform(to, subject) {
        print("Sending to #{to}: #{subject}")
    }
    on_failure(error, attempt) {
        print("Failed (attempt #{attempt}): #{error}")
    }
}
```

**Syntax:** `job Name on queue_name (options) { perform(params) { body } on_failure(params) { body } }`

- `on queue_name` — assigns the job to a named queue
- Options: `retry: N` (default 3), `backoff: "exponential"|"linear"|"constant"`, `timeout: N` (seconds, post-execution check — does not preemptively interrupt)
- `on_failure` block is optional — called on each failure with error message and attempt count

### Enqueueing Jobs

```ntnt
import { enqueue, enqueue_in, enqueue_at } from "std/jobs"

// Immediate
let id = unwrap(enqueue("SendEmail", map { "to": "alice@example.com", "subject": "Hello" }))

// Delayed (seconds)
enqueue_in("SendEmail", 3600, map { "to": "bob@example.com", "subject": "Reminder" })

// At specific time (nanosecond timestamp)
enqueue_at("SendEmail", future_nanos, map { "to": "eve@example.com", "subject": "Scheduled" })
```

### Running Workers

```ntnt
import { work_async, work_jobs } from "std/jobs"

// Background workers (returns Array<TaskHandle>)
let workers = work_async(map { "concurrency": 4, "poll_interval": 500 })

// Blocking worker (for CLI scripts — exits on Ctrl-C)
work_jobs()
```

**CLI — Workers:**
```bash
ntnt worker server.tnt                          # Single worker
ntnt worker server.tnt --concurrency 4          # 4 parallel workers
ntnt worker server.tnt --queues emails,payments # Specific queues
```

**CLI — Observability & Management:**
```bash
ntnt jobs status server.tnt                              # Counts by status
ntnt jobs list server.tnt                                # List all jobs (newest first)
ntnt jobs list server.tnt --status=dead --limit=20       # Filter by status
ntnt jobs list server.tnt --queue=emails --format=json   # Filter by queue, JSON output
ntnt jobs inspect server.tnt <JOB_ID>                    # Full job details
ntnt jobs retry server.tnt <JOB_ID>                      # Re-queue a dead/retrying job
ntnt jobs cancel server.tnt <JOB_ID>                     # Cancel pending/scheduled job
ntnt jobs cancel server.tnt <JOB_ID> --force             # Force-cancel active job
ntnt jobs clear server.tnt --status=completed            # Bulk delete by status
ntnt jobs clear server.tnt --status=dead --older-than=7d --yes  # Age filter, skip prompt
```

### Job Status & Control

```ntnt
import { job_status, cancel_job, retry_job, list_jobs, delete_jobs } from "std/jobs"

let status = unwrap(job_status(id))   // Map with status, attempts, timestamps
cancel_job(id)                         // Cancel a pending/scheduled/retrying job
cancel_job(id, map { "force": true })  // Force-cancel an active (running) job
retry_job(id)                          // Re-queue a dead or retrying job
list_jobs()                            // List all jobs (up to 100)
list_jobs(map { "status": "dead", "queue": "emails", "limit": 10 })
delete_jobs(map { "status": "completed" })              // Bulk delete
delete_jobs(map { "status": "dead", "older_than_secs": 604800 })  // 7 days
```

### Testing Mode

```ntnt
import { configure_queue, enqueue, assert_enqueued, assert_not_enqueued, drain_jobs, clear_jobs } from "std/jobs"

configure_queue(map { "mode": "testing" })  // No KV writes — jobs collected in memory

enqueue("SendEmail", map { "to": "test@example.com" })

// Assertions (partial match — extra keys OK)
assert_enqueued("SendEmail", map { "to": "test@example.com" })
assert_not_enqueued("ProcessPayment")

// Execute all queued jobs synchronously
let count = unwrap(drain_jobs())

// Reset between tests
clear_jobs()
```

### Queue Configuration

```ntnt
import { configure_queue } from "std/jobs"

// SQLite (default, zero-config)
configure_queue(map { "store": "sqlite:./jobs.db" })

// Redis (production)
configure_queue(map { "store": "redis://localhost:6379" })

// Testing mode (in-memory, no persistence)
configure_queue(map { "mode": "testing" })
```

### Lifecycle & Retry

Jobs follow this state machine:
```
Pending   → Active → Completed
Scheduled → Active → Completed
                 ├→ Retrying (retries left, waiting for backoff) → Active (retry)
                 └→ Dead (retries exhausted)
```
- `pending` — ready to run immediately
- `scheduled` — enqueued for the future (`enqueue_at`/`enqueue_in`)
- `active` — currently being processed by a worker
- `completed` — finished successfully
- `retrying` — execution failed, waiting for retry backoff (will auto-retry)
- `dead` — all retries exhausted (final failure state)
- `cancelled` — manually cancelled via `cancel_job()` or `ntnt jobs cancel`

Backoff strategies:
- `"exponential"` (default): 5s base, doubles each retry, capped at 1 hour
- `"linear"`: 5s × attempt
- `"constant"`: 5s fixed

### Streaming Logs

Workers emit structured JSON events to stderr:
```json
{"event":"job.started","job_id":"abc","type":"SendEmail","queue":"emails","attempt":1,"timestamp":"..."}
{"event":"job.completed","job_id":"abc","type":"SendEmail","timestamp":"..."}
{"event":"job.failed","job_id":"abc","type":"SendEmail","error":"...","attempt":1,"will_retry":true,"timestamp":"..."}
```

Custom event handling via `on_job_event(handler)` is planned for a future release.

---

## CSV (`std/csv`)

```ntnt
import { parse_csv, parse_with_headers, stringify, stringify_with_headers } from "std/csv"

let rows = parse_csv("a,b\n1,2\n3,4")
// [["a", "b"], ["1", "2"], ["3", "4"]]

let maps = parse_with_headers("name,age\nAlice,30\nBob,25")
// [{"name": "Alice", "age": "30"}, {"name": "Bob", "age": "25"}]

let csv_str = stringify([["a", "b"], [1, 2]])
let csv_str = stringify_with_headers(maps, ["name", "age"])
```

---

## URL Handling (`std/url`)

```ntnt
import { parse_url, encode, encode_component, decode, build_query, parse_query } from "std/url"

let parts = unwrap(parse_url("https://example.com/path?q=hello"))
// { "scheme": "https", "host": "example.com", "path": "/path", "query": "q=hello", ... }

let encoded = encode_component("hello world & more")  // "hello%20world%20%26%20more"
let decoded = unwrap(decode("hello%20world"))          // "hello world"

let qs = build_query(map { "q": "search", "page": "1" })  // "page=1&q=search"
let params = parse_query("q=search&page=1")                // { "q": "search", "page": "1" }
```

---

## Path Manipulation (`std/path`)

```ntnt
import { join_path, dirname, basename, extension, stem, resolve, is_absolute } from "std/path"

join_path(["src", "lib", "utils.tnt"])  // "src/lib/utils.tnt"
dirname("src/lib/utils.tnt")            // Some("src/lib")
basename("src/lib/utils.tnt")           // Some("utils.tnt")
extension("utils.tnt")                  // Some("tnt")
stem("utils.tnt")                       // Some("utils")
is_absolute("/usr/bin")                 // true
```

---

## Markdown (`std/markdown`)

```ntnt
import { to_html, to_html_safe } from "std/markdown"

let html_str = to_html("# Hello\n\nThis is **bold**")
// "<h1>Hello</h1>\n<p>This is <strong>bold</strong></p>\n"

let safe_html = to_html_safe("<script>alert('xss')</script> **bold**")
// Script tags stripped, only safe HTML output
```

---

## Testing with `ntnt test`

Test HTTP servers directly from the command line — starts the server, makes requests, prints responses, then shuts down:

```bash
# Simple GET
ntnt test server.tnt --get /api/status

# POST with body
ntnt test server.tnt --post /users --body '{"name":"Alice"}'

# Multiple requests in one run
ntnt test server.tnt --get /health --get /api/users --post /api/users --body '{"name":"Bob"}'

# Verbose (show headers)
ntnt test server.tnt -v --get /api/status

# Custom port
ntnt test server.tnt --port 9090 --get /
```

---

## Quick Reference Tables

### Global Builtins (No Import)

**Conversion & Output:**

| Function | Description |
|----------|-------------|
| `print(x)` | Output to stdout |
| `str(x)` | Convert to string |
| `int(x)` | Convert to integer |
| `float(x)` | Convert to float |
| `type(x)` | Get type name as string |
| `typeof(x)` | Get type name (alias for `type`) |

**Collections:**

| Function | Description |
|----------|-------------|
| `len(x)` | Length of string, array, or map |
| `push(arr, item)` | Add to array |
| `filter(arr, fn)` | Filter array with predicate |
| `transform(arr, fn)` | Transform (map) array elements |
| `find(arr, fn)` | First element matching predicate → `Option` |
| `sort(arr)`, `sort(arr, key)` | Sort array (key: string field name or function) |
| `sort_desc(arr)`, `sort_desc(arr, key)` | Sort descending |
| `any(arr, fn)` | True if any element matches |
| `all(arr, fn)` | True if all elements match |
| `reduce(arr, init, fn)` | Reduce array to single value |
| `flat_map(arr, fn)` | Map + flatten results |

**Math:**

| Function | Description |
|----------|-------------|
| `abs(n)`, `min(a,b)`, `max(a,b)` | Basic math |
| `round(n)`, `round(n, decimals)` | Round to integer or decimal places |
| `floor(n)`, `ceil(n)`, `trunc(n)` | Floor, ceiling, truncate |
| `clamp(n, min, max)` | Clamp value to range |
| `pow(base, exp)`, `sqrt(n)`, `sign(n)` | Power, square root, sign |

**Error Handling:**

| Function | Description |
|----------|-------------|
| `Ok(value)`, `Err(value)` | Construct Result values |
| `Some(value)` | Construct Option value |
| `unwrap(result)` | Extract value or panic on error |
| `unwrap_or(result, default)` | Extract value or use default |
| `assert(cond)` | Assert condition (panics on false) |

**Type Checks:**

| Function | Description |
|----------|-------------|
| `is_string(x)`, `is_int(x)`, `is_float(x)`, `is_bool(x)` | Type checks |
| `is_array(x)`, `is_map(x)` | Collection type checks |
| `is_some(x)`, `is_none(x)` | Option checks |
| `is_ok(x)`, `is_err(x)` | Result checks |

**HTTP Server:**

| Function | Description |
|----------|-------------|
| `get/post/put/patch/delete(pattern, handler)` | HTTP routes |
| `listen(port)` | Start server |
| `serve_static(prefix, dir)` | Static files |
| `routes(dir)` | File-based routing |
| `template(path, vars)` | Load template |
| `use_middleware(fn)` | Add middleware |
| `enable_cors(options?)` | Configure CORS |
| `enable_csp(options?)` | Configure Content Security Policy |
| `on_shutdown(fn)` | Cleanup handler on server stop |
| `on_error(fn)` | Custom error handler |

### Common Imports

```ntnt
import { split, join, trim, replace, contains, chars, capture_pattern } from "std/string"
import { json, html, text, redirect, status, not_found, error, response, parse_form, parse_json } from "std/http/server"
import { connect, query, query_one, execute, begin, commit, rollback, close } from "std/db/postgres"
import { connect, query, query_one, execute, begin, commit, rollback, close } from "std/db/sqlite"
import { fetch, download } from "std/http"
import { read_file, write_file, exists } from "std/fs"
import { parse_json, stringify } from "std/json"
import { get_env, load_env } from "std/env"
import { now, format } from "std/time"
import { sha256, uuid } from "std/crypto"
import { first, last, keys, values, entries, has_key, get_key, get_index } from "std/collections"
import { oauth, enable_auth, get_user, get_session, validate_csrf } from "std/auth"
import { open, get, set, del, list } from "std/kv"
import { log_info, log_warn, log_error, set_log_level, request_logger } from "std/log"
import { parse_url, encode_component, build_query, parse_query } from "std/url"
import { parse_csv, parse_with_headers } from "std/csv"
import { to_html } from "std/markdown"
import { join_path, dirname, basename, extension } from "std/path"
import { channel, send, recv, sleep_ms, spawn, await_task, schedule, cancel_schedule } from "std/concurrent"
```

### CLI Commands

```bash
ntnt run <file>              # Run a .tnt file (hot-reload in dev mode)
ntnt repl                    # Interactive REPL
ntnt lint <file>             # Check for errors and style issues
ntnt lint --strict <file>    # Strict type warnings
ntnt check <file>            # Quick syntax check
ntnt validate <file>         # Validate with JSON output (for tools)
ntnt inspect <file>          # Project structure as JSON
ntnt test <file> --get /     # Test HTTP server (start, request, shutdown)
ntnt test <file> --post /api --body '{"key":"val"}'  # POST with body
ntnt intent check <file>     # Verify code matches intent
ntnt intent studio <intent>  # Visual studio with live tests
ntnt intent coverage <file>  # Show feature coverage
ntnt intent init <intent>    # Generate scaffolding from intent
ntnt docs [query]            # Search stdlib documentation
ntnt docs --generate         # Regenerate reference docs from source
ntnt docs --validate         # Check documentation coverage
ntnt completions bash|zsh|fish  # Shell completions
```

---

## Troubleshooting

NTNT error messages include error codes (E001-E012), source snippets, line numbers, and "did you mean?" suggestions for typos.

### Common Parse Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `unexpected token '{'` | Using `{}` for map literal | Add `map` keyword: `map { "key": "value" }` |
| `unexpected token '$'` | Using `${expr}` interpolation | Use `#{expr}` — hash-brace syntax |
| `expected identifier` | Inline lambda in route | Use named function: `fn handler(req) { ... }` |
| `unexpected token '.'` | Method-style call on stdlib function | Use function style: `len(s)` not `s.len()`. Dot notation is for reading properties, not calling stdlib functions. |
| `Required parameter 'x' cannot follow a parameter with a default value` | Non-default param after default | Move all required params before defaulted ones: `fn f(a, b = 1)` not `fn f(a = 1, b)` |

### Common Runtime Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `requires clause failed` | Precondition not met | Check input values meet contract requirements |
| `ensures clause failed` | Postcondition not met | Fix function to return correct values |
| `key not found` | Missing map key | Use `has_key()` to check, or `get_key()` for Option |
| `index out of bounds` | Array index invalid | Check `len()` before accessing |
| `db error` | Type mismatch in query | Convert types: `int(form["age"])` for integers |

### Contract Violations in HTTP Routes

`requires` failure → 400 Bad Request. `ensures` failure → 500 Internal Server Error.

### Intent Check Failures

| Issue | Meaning | Fix |
|-------|---------|-----|
| `unresolved term` | Glossary term not defined | Add term to `## Glossary` section |
| `feature not implemented` | Missing `@implements` | Add `// @implements: feature.id` to function |
| `assertion failed` | Test didn't pass | Fix implementation to match expected behavior |
| `status mismatch` | Wrong HTTP status | Check route returns correct status code |

### Type Check Errors

When using `ntnt lint` or `NTNT_STRICT=1`, you may see type diagnostics:

| Error | Cause | Fix |
|-------|-------|-----|
| `expected String, got Int` | Wrong argument type | Convert with `str(x)` or fix the call |
| `expected 2 args, got 1` | Wrong argument count | Check function signature |
| `returns Int, expected String` | Return type mismatch | Fix return value or annotation |
| `expected Int, got String` | Let binding mismatch | Fix the assigned value or annotation |

### Debugging Tips

1. `ntnt lint file.tnt` — catches 90% of issues
2. `print("Debug: #{variable}")` / `print("Type: #{type(variable)}")`
3. Add type annotations and contracts for precise error locations
4. Use Intent Studio for live feedback

## Common Patterns & Gotchas

### Imports Are Per-File

Each `.tnt` file has its own import scope. Importing `std/collections` in `lib/data.tnt` does NOT make `get_or` or `merge` available in `routes/admin/index.tnt`. Every file that uses a stdlib function must import it explicitly.

```ntnt
// routes/admin/index.tnt
import { get_or } from "std/collections"  // Required even if lib/data.tnt also imports it
```

This is intentional — explicit imports make dependencies clear and prevent hidden coupling between files.

### Shared State Across Files (Singleton Pattern)

Module-level variables are not accessible across files. Use the function-wrapper pattern for shared resources like database connections:

```ntnt
// lib/data.tnt — shared database module
import { connect } from "std/db/sqlite"

let db = connect("app.db")

fn get_db() {
    return db
}

fn get_users() {
    return query(get_db(), "SELECT * FROM users", [])
}
```

```ntnt
// routes/api/users.tnt — uses the shared module
import { get_users } from "../lib/data"

fn handle(req) {
    let users = get_users()
    return json(users)
}
```

The `get_db()` function pattern is the idiomatic way to share state. The module's `let db = connect(...)` runs once when first imported, and the function provides access to it from any file.

### Map Access Returns None for Missing Keys

Map bracket and dot access return `None` for missing keys instead of throwing errors:

```ntnt
let m = map { "name": "Alice" }
m["name"]     // "Alice" (raw value, NOT wrapped in Some)
m["missing"]  // None
m.name        // "Alice"
m.missing     // None
```

**Important:** Use `has_key()` to check key existence, NOT `is_some()`:

```ntnt
import { has_key } from "std/collections"

// ✅ Correct — use has_key()
if has_key(m, "name") { ... }

// ❌ Wrong — is_some() won't work because existing keys return raw values, not Some
if is_some(m["name"]) { ... }
```

**Edge case:** If a map contains an explicit `None` value, bracket access returns `None` for both the key-with-None-value and missing keys. Use `has_key()` to distinguish:

```ntnt
let m = map { "a": None }
m["a"]            // None (key exists, value is None)
m["b"]            // None (key doesn't exist)
has_key(m, "a")   // true
has_key(m, "b")   // false
```

### Truthy/Falsy Values

⚠️ **`0` is truthy** — unlike JS/Python. All numbers (including 0) are truthy. Falsy values: `false`, `""`, `None`, `[]`, `map {}`. Check zero explicitly: `if value == 0 { ... }`.

### Map Iteration

`for k in map` iterates over **keys**, not entries. Use `entries()` for key-value pairs, `values()` for values only:

```ntnt
import { entries, values } from "std/collections"

for name in users { print("#{name}: #{users[name]}") }         // keys
for entry in entries(users) { print("#{entry[\"key\"]}") }    // key-value
for age in values(users) { print(age) }                      // values only
```
<!-- END NTNT CODING GUIDE -->

---

## Editing the NTNT Language (Rust Development)

This section covers modifying the NTNT compiler/interpreter itself — adding stdlib functions, changing builtins, or updating the Rust implementation. This is specific to Claude Code development workflows.

### Stdlib Documentation System (`// @ntnt`)

Documentation lives as structured `// @ntnt` comment blocks directly above function implementations in Rust source. `build.rs` scans all `.rs` files in `src/stdlib/` plus `src/interpreter.rs` at compile time, validates coverage, and embeds the data as JSON in the binary.

**Build enforcement:**
- Every `NativeFunction` insert must have a `// @ntnt` block → **build fails** if missing
- Every `// @ntnt` block must have a matching `NativeFunction` insert → **build fails** if orphaned
- Source file discovery is automatic (glob) — no need to register new files

#### Doc Block Format

Place a `// @ntnt` block directly above the `module.insert(` call:

```rust
// @ntnt my_function
// @module std/string
// @module_description String manipulation functions (only on first fn in module)
// @signature my_function(s: String, n: Int) -> String
// Brief summary of what the function does (first non-@ lines).
//
// Extended description (after blank comment line). Can span multiple
// lines — consecutive lines are joined into paragraphs, blank lines
// separate paragraphs.
// @param s The input string
// @param n Number of times to repeat
// @returns A new string with s repeated n times
// @see_also repeat, concat
// @since v0.3.9
// @tags #pure, #deterministic
// @example my_function("ab", 3) => "ababab" ~ "Repeat string 3 times"
// @example my_function("", 5) => "" ~ "Empty string stays empty"
// @error TypeError ~ "my_function() requires a string and integer" fix: "Check argument types"
// @gotcha Returns empty string for n <= 0, does not error
module.insert(
    "my_function".to_string(),
    Value::NativeFunction { ... },
);
```

Global builtins in `src/interpreter.rs` use the same format but omit `@module`.

#### All Directives

| Directive | Required | Description |
|-----------|----------|-------------|
| `// @ntnt <name>` | **Yes** | Block header — must match the function name in `module.insert()` |
| `// @module <path>` | Stdlib only | Module path (e.g., `std/string`, `std/http/server`) |
| `// @module_description <text>` | First fn only | One-line module description (on first function in each module) |
| `// @signature <sig>` | **Yes** | Full typed signature: `name(params) -> ReturnType` |
| Summary line(s) | **Yes** | First non-`@` lines before a blank `//` line |
| Description | No | Lines after the blank separator — joined into paragraphs |
| `// @param <name> <desc>` | Per param | Parameter name and description (one per line) |
| `// @returns <desc>` | When helpful | Return value description |
| `// @see_also <a>, <b>` | No | Comma-separated related function names |
| `// @since <version>` | No | Version when function was added |
| `// @tags <t1>, <t2>` | No | Comma-separated: `#pure`, `#deterministic`, `#io`, `#network`, `#filesystem`, `#random` |
| `// @example ...` | **Yes** (1+) | Usage example (see formats below) |
| `// @error <Type> ~ "<msg>" fix: "<fix>"` | No | Error condition with optional fix suggestion |
| `// @gotcha <text>` | No | Non-obvious behavior or common pitfall |

#### Single-Line Examples

Most examples fit on one line:

```rust
// @example split("a,b,c", ",") => ["a", "b", "c"] ~ "Comma-separated split"
//          ^^^^^^^^^^^^^^^^^^^^    ^^^^^^^^^^^^^^^    ^^^^^^^^^^^^^^^^^^^^^^
//          Code                    Expected (opt)     Description (opt)

// All parts are optional except code:
// @example split("hello", "")                          // Code only
// @example split("a,b", ",") => ["a", "b"]             // Code + expected
// @example split("a,b", ",") ~ "Basic split"           // Code + description
```

#### Multi-Line Examples

For complex code that doesn't fit on one line, use the block format:

```rust
// @example ~ "POST request with JSON body"
//   let opts = map {
//     "url": "https://api.example.com",
//     "method": "POST",
//     "json": map { "key": "value" }
//   }
//   fetch(opts)
// @expected Ok({status: 201, ...})
```

Rules:
- `// @example` alone or `// @example ~ "description"` starts a multi-line block
- Continuation lines are **indented by 2+ spaces** (after the `// ` prefix)
- `// @expected <value>` provides the expected result (optional, must be inside the block)
- The block ends at the next non-indented line or `@` directive

#### Error Format

```rust
// @error TypeError ~ "split() requires two strings" fix: "Pass two string arguments"
//        ^^^^^^^^^   ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^   ^^^^^^^^^^^^^^^^^^^^^^^^^^
//        Error type  Error message (in quotes)              Fix suggestion (optional)
```

### Build Pipeline

```
build.rs                    Runtime
┌─────────────────────┐     ┌──────────────────────────────────┐
│ Scan src/stdlib/*.rs │     │ Binary embeds doc_data.json      │
│ Scan src/interpreter │────>│                                  │
│                     │     │ Consumed by:                     │
│ Validate coverage:  │     │   :doc <name>     (REPL)         │
│   Missing → ERROR   │     │   :search <query> (REPL)         │
│   Orphaned → ERROR  │     │   ntnt docs <q>   (CLI)          │
│   No summary → WARN │     │   ntnt docs --generate (markdown)│
│                     │     │   ntnt docs --validate (coverage) │
│ Output: doc_data.json│     └──────────────────────────────────┘
└─────────────────────┘
```

Source file discovery is automatic — `build.rs` globs `src/stdlib/*.rs` and always includes `src/interpreter.rs`. New stdlib files are picked up without any configuration.

### REPL Documentation Commands

After building (`cargo build --profile dev-release`):

```
ntnt> :doc split              # Full docs for a function
ntnt> :doc splitt             # Typo → "Did you mean split?"
ntnt> :doc std/string         # List all functions in a module
ntnt> :search "uppercase"     # Search names, summaries, descriptions
```

### Checklist for Adding a Stdlib Function

1. Write the `// @ntnt` doc block with at least: `@ntnt`, `@module`, `@signature`, summary, one `@example`
2. Write the `module.insert()` implementation directly below the doc block
3. Build: `cargo build --profile dev-release` — **build fails** if docs are missing or orphaned
4. Test in REPL: `:doc my_function` — verify docs render correctly
5. Run `ntnt docs --generate` to regenerate `STDLIB_REFERENCE.md` and sync agent files
6. Follow the doc maintenance checklist below

---

## Documentation Maintenance (MANDATORY)

**After implementing any language feature, update these:**

1. `// @ntnt` doc blocks on all new/changed functions (build enforces this)
2. `docs/AI_AGENT_GUIDE.md` for user-facing syntax or patterns
3. `ntnt docs --generate` to regenerate `STDLIB_REFERENCE.md` and auto-sync agent files (`CLAUDE.md`, `.github/copilot-instructions.md`)
4. `LANGUAGE_GUIDE.md` for detailed explanations
5. `ARCHITECTURE.md` for structural changes
6. `ROADMAP.md` to update feature status
7. Add integration tests and example files

Do not wait for the user to ask — update docs as part of every implementation task.

## Greptile Review Process

Greptile auto-reviews every PR on this repo. AI agents must self-service Greptile feedback before requesting human review.

### Comment Triage

| Bucket | Type | Action |
|--------|------|--------|
| 1 | **Correctness** — bugs, race conditions, dead code, wrong logic | Auto-fix. Commit + reply with hash. |
| 2 | **Hardening** — missing validation, undocumented behavior, incomplete tests, error messages | Auto-fix. Clear right answer exists. |
| 3 | **Architecture** — data structure changes, API redesign, flow restructuring, design philosophy | **Do not fix.** Escalate to maintainer with the suggestion and your assessment. |

### Workflow

1. Push PR → Greptile reviews (~3 min)
2. Read comments, triage into buckets
3. Fix all Bucket 1/2 items → commit → push → Greptile re-reviews
4. Loop until only Bucket 3 items remain (or clean)
5. Post summary to maintainer: ✅ Fixed (list), ❓ Need your call (Bucket 3 items)
