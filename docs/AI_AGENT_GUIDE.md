# NTNT AI Agent Guide

Critical syntax rules and patterns for AI agents generating NTNT code. For complete reference documentation, see:

- **[STDLIB_REFERENCE.md](STDLIB_REFERENCE.md)** - All functions and modules
- **[SYNTAX_REFERENCE.md](SYNTAX_REFERENCE.md)** - Keywords, operators, types, templates
- **[IAL_REFERENCE.md](IAL_REFERENCE.md)** - Intent Assertion Language

---

## Quick Start

### Installation

```bash
# One-line install (installs Rust if needed, clones repo, builds)
curl -sSf https://raw.githubusercontent.com/ntntlang/ntnt/main/install.sh | bash
source "$HOME/.cargo/env"
ntnt --version  # Verify: should print ntnt 0.4.6 or later
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

After trying NTNT, set up persistent agent knowledge so every future session writes correct code. Add the following to your project's agent configuration (`CLAUDE.md`, `.cursor/rules/*.mdc`, `AGENTS.md`, or equivalent):

~~~markdown
# NTNT Language Rules

## Critical Syntax (memorize these — every rule prevents a real mistake)
- Maps REQUIRE `map` keyword: `map { "key": "val" }` — bare `{}` is a code block
- String interpolation: `#{expr}` — hash-brace syntax, never `${expr}` or bare `{expr}`
- Template strings: `"""..{{expr}}.."""` — double braces inside triple quotes
- Semicolons are supported as statement separators, but newlines are still the normal style.
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

### Function Unit Testing (call: syntax)

Test individual functions without an HTTP server. Add `call:` and `source:` keywords in the glossary:

```intent
## Glossary

| Term | Means |
|------|-------|
| slugifying {text} | call: to_slug({text}), source: utils.tnt |
| validating email {email} | call: is_valid_email({email}), source: validators.tnt |

---

Feature: URL Slugs
  id: feature.slugs

  Scenario: Basic slug conversion
    When slugifying "Hello World"
    → result is "hello-world"
    → is lowercase
    → does not contain " "

  Scenario: Slug is deterministic
    When slugifying "Hello World"
    → is deterministic
```

**Required keywords:** `call:` (function with `{param}` placeholders) and `source:` (`.tnt` file containing the function).

### POST Requests in Intent Scenarios

To test POST endpoints with a request body, define glossary terms using the `body` keyword:

```yaml
## Glossary

| Term | Means |
|------|-------|
| a user creates a message with {body} | POST /messages body {body} |
| a user visits {path} | GET {path} |

---

Feature: Messages API
  id: feature.messages

  Scenario: Create a message
    When a user creates a message with {"text": "hello"}
    → status: 201
    → body has field "id"
    → body has field "text"
```

The `body` keyword in the Means column separates the path from the JSON body: `POST /path body {json}`.

> **Tip:** For endpoints that require a body, design your handler to parse `req.body` as JSON. The intent checker sends the body as `Content-Type: application/json`.

### Built-in Assertion Terms

These work in `→` lines without needing glossary entries:

| Category | Examples |
|----------|---------|
| **HTTP status** | `status: 200`, `status 2xx`, `status 4xx` |
| **HTTP body** | `body contains {text}`, `body not contains {text}`, `body matches {pattern}`, `body is empty`, `body has field {name}`, `response is valid JSON` |
| **Headers** | `header {name} exists`, `header {name} equals {value}`, `content-type is json` |
| **Function result** | `result is {expected}`, `is lowercase`, `is non-empty`, `starts with {prefix}`, `ends with {suffix}`, `does not contain {text}` |
| **Properties** | `is deterministic`, `is idempotent` |
| **Code quality** | `code passes lint`, `no syntax errors`, `no lint warnings` |
| **Response time** | `response time < {ms}ms` |

For the complete list, see [IAL_REFERENCE.md](IAL_REFERENCE.md).

### Output Symbols

| Symbol | Meaning |
|--------|---------|
| ✓ | Passed |
| ✗ | Failed |
| ⏭️ | Skipped (precondition not met) |
| ⧗ | Warning/Pending — unresolved outcome terms or unresolvable scenario. Counts as failed in summary. Check that all `→` outcome terms are in your Glossary or match a built-in assertion term. |

### Tips

- Prefer `body has field "key"` or `content-type is json` over `response is valid JSON` for checking JSON responses — they're more reliable and specific.
- If a scenario shows ⧗, an outcome term wasn't recognized. Rephrase using built-in terms like `body contains`, `status 200`, `body has field`.

### Commands

```bash
ntnt intent check server.tnt       # Verify code matches intent
ntnt intent studio server.intent   # Live visual feedback (opens :3001)
ntnt intent coverage server.tnt    # Feature coverage report
ntnt intent init server.intent     # Generate scaffolding from intent
```

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
int(form.age) ?? 0      // convert a value to a new type, handling parse failure

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

// Block expressions in branches (v0.4.6+) — multi-statement, last expression is the value
let result = if condition {
    let temp = compute()
    let adjusted = temp * 2
    adjusted + 1
} else {
    default_value()
}
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

### 17. `??` Precedence: Lower Than `[]`

The null-coalescing operator has lower precedence than bracket access. This is a common footgun:

```ntnt
// WRONG — when query_params exists, returns the entire map (not the "status" value);
// when query_params is None, silently falls through to "all" and hides the problem:
let x = req["query_params"] ?? map {}["status"] ?? "all"

// RIGHT — split into two lines:
let params = req["query_params"] ?? map {}
let status_filter = params["status"] ?? "all"
```

### 18. `[]` Returns None on Type Mismatch

Index access on a non-collection returns `None` instead of crashing (v0.3.17+):

```ntnt
42["key"]          // → None (not TypeError)
true["field"]      // → None
```

Use `??` as a universal safety net: `val["key"] ?? default`.

### 19. `for..in` Skips Non-Collections Silently

Iterating over a non-collection (string, int, None) does zero iterations with a dev-mode warning:

```ntnt
for c in "hello" { print(c) }   // Does nothing — use chars("hello") instead
for x in None { print(x) }      // Does nothing — no crash
```

### 20. Module-Level `let` Doesn't Support `map {}` Literals

`let X = map { ... }` at the top level of a library file fails. Use arrays or move maps inside functions:

```ntnt
// WRONG — at module top level:
let CONFIG = map { "timeout": 30 }

// RIGHT — wrap in a function:
fn get_config() { return map { "timeout": 30 } }

// RIGHT — arrays work at top level:
let ALLOWED = ["admin", "editor"]
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

### Network/IPAM Helpers (`std/net`)

`std/net` provides deterministic IPv4/IPv6 CIDR helpers, protocol-honest ICMP ping, explicit TCP connect probes, bounded port scans, high-level reachability, and DNS lookups:

```ntnt
import { ip_parse, subnet_contains, subnet_split, subnet_summarize, tcp_connect, port_scan, reachable, dns_lookup, dns_reverse } from "std/net"

let info = unwrap(ip_parse("192.168.1.0/24"))
let contains = unwrap(subnet_contains("10.0.0.0/8", "10.42.0.0/16"))
let children = unwrap(subnet_split("192.168.1.0/24", 28))
let summary = unwrap(subnet_summarize(["10.0.0.0/25", "10.0.0.128/25"]))
let tcp = unwrap(tcp_connect("example.com", 443))
let ports = unwrap(port_scan("example.com", [80, 443], map { "timeout_ms": 500 }))
let reachability = unwrap(reachable("example.com", map { "tcp_ports": [443] }))
let records = unwrap(dns_lookup("example.com", "A"))
let ptr_names = unwrap(dns_reverse("8.8.8.8"))
```

These helpers return `Result<..., String>`; use `unwrap(...)` for quick scripts/examples, or `match`/`otherwise` when the app should handle invalid input or policy denial.

`ip_parse()` supports IPv4 and IPv6. Large IPv6 address counts are returned as strings so `/64` and larger networks do not overflow integer values.

`ping()` is ICMP-only and does **not** silently fall back to TCP ports. In Phase 1, unsupported ICMP returns `Err(String)` with guidance. If an app intentionally wants a TCP port check, use `tcp_connect(host, port, opts?)`. If it wants a high-level “is this host reachable somehow?” check, use `reachable(host, opts?)` with explicit `tcp_ports` so the result can honestly report `method: "tcp"` and `fallback_from: "icmp"`.

```ntnt
let tcp = tcp_connect("example.com", 443, map { "count": 5 })

let reachability = reachable("example.com", map {
    "tcp_ports": [443],
    "count": 5
})
```

`tcp_connect()` and `reachable()` support optional `count` (1-10), `timeout_ms`, and `interval_ms`, returning per-attempt results plus `sent`, `received`, `failed`, and `loss_percent` summary fields.

`port_scan(host, ports, opts?)` scans an explicit `Array<Int>` of TCP ports. It rejects duplicate, invalid, or overly large port lists, clamps `concurrency`, applies the same private-target safety policy as `tcp_connect()`, and returns results sorted by port:

```ntnt
let scan = port_scan("example.com", [22, 80, 443], map {
    "timeout_ms": 500,
    "concurrency": 20
})
// Ok([map { "port": 22, "open": false, "reason": "connection refused" }, ...])
```

`dns_lookup(name, record_type?, opts?)` supports common data-bearing DNS record types, including `A`, `AAAA`, `MX`, `TXT`, `NS`, `CNAME`, `SOA`, `SRV`, `CAA`, `TLSA`, `HTTPS`, and `SVCB`. It returns `Ok([])` for ordinary no-answer DNS responses and `Err(String)` for invalid record types, invalid options, resolver configuration failures, or DNS transport failures. Record maps use the actual returned DNS type, so a resolver response that includes related records reports those records honestly instead of relabeling everything as the requested type. `dns_reverse(ip, opts?)` returns all PTR names as an array because PTR can legitimately have zero, one, or multiple names.

```ntnt
let a_records = dns_lookup("example.com", "A", map { "timeout_ms": 1000 })
let mx_records = dns_lookup("example.com", "MX", map { "timeout_ms": 1000 })
let txt_records = dns_lookup("example.com", "TXT", map { "timeout_ms": 1000 })
let ptr_names = dns_reverse("8.8.8.8", map { "timeout_ms": 1000 })
```

When passing `dns_lookup` options, include the record type explicitly: `dns_lookup(name, "A", opts)`. The shorter `dns_lookup(name)` form defaults to `A`; `dns_lookup(name, opts)` is intentionally rejected so the call shape stays unambiguous.

Private/internal targets are denied by default. Monitoring apps must opt in at process scope **and** call scope:

```bash
NTNT_NET_ALLOW_PRIVATE=1 ntnt run monitor.tnt
```

```ntnt
let result = tcp_connect("10.0.0.5", 443, map { "allow_private": true })
```

Special-purpose and high-risk targets such as cloud metadata endpoints, multicast, broadcast, unspecified, and documentation ranges remain blocked even with private-network opt-in.

Do not pipe user-controlled hostnames directly into `std/net` probes in public web apps. The stdlib blocks the worst SSRF targets by default, but app-level validation is still required.

### JSON Body Parsing

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

let age = int(age_str) otherwise { return Err("age must be an integer") }
execute(db, "INSERT INTO users (name, age) VALUES ($1, $2)", [name, age])

close(db)  // Releases the connection pool
```

**Type conversion for database:**
```ntnt
let form = parse_form(req)
let age = int(form["age"]) otherwise { return status(400, "age must be an integer") }
let price = float(form["price"]) ?? 0.0

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

### Path Resolution

Template paths resolve relative to the **entry-point `.tnt` file** (the file passed to `ntnt run`), NOT the file containing the `template()` call. If `server.tnt` is your entry point and `lib/helpers.tnt` calls `template("views/page.html", data)`, the path resolves from `server.tnt`'s directory.

```ntnt
// In lib/helpers.tnt, called from server.tnt in project root:
template("views/page.html", data)    // CORRECT — resolves from project root
template("../views/page.html", data) // WRONG — don't use relative paths from lib/
```

### Template Strings vs template() — Key Difference

Template strings in `.tnt` code (`"""...{{expr}}..."""`) **auto-escape HTML** in interpolated values. The `template()` function uses Mustache syntax where `{{var}}` escapes HTML and `{{{var}}}` outputs raw HTML.

If you need to inject pre-rendered HTML (like from another template call), use `{{{var}}}` triple-braces in the template file:

```html
<!-- views/layout.html -->
<div class="content">{{{body}}}</div>  <!-- Raw HTML, not escaped -->
<p>User: {{username}}</p>              <!-- Escaped (safe for user input) -->
```

### Conditionals: Use `{{#if}}`, NOT `{{#var}}`

NTNT uses `{{#if var}}` syntax for conditionals — NOT Mustache-style section syntax `{{#var}}...{{/var}}`:

```html
<!-- CORRECT -->
{{#if error}}<p class="error">{{error}}</p>{{/if}}
{{#if user}}Welcome, {{user.name}}!{{#else}}Please log in.{{/if}}

<!-- WRONG — Mustache section syntax is NOT supported -->
{{#error}}<p>{{error}}</p>{{/error}}
```

Loops use `{{#for}}`: `{{#for item in items}}...{{/for}}`

### Reserved Names — Don't Shadow Builtins

These names are built-in functions. Don't use them for your own functions:

`render`, `compile`, `template`, `sort`, `filter`, `reduce`, `find`, `any`, `all`, `count`, `transform`, `flat_map`

If you name a function `render()`, you'll get a confusing error like "render() first argument must be a compiled template" instead of calling your function. Use names like `render_page()`, `render_layout()`, etc.

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
    if m > 0 { m } else { int(now()) ?? 0 }
} else { int(now()) ?? 0 }

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

### Template Filters

Apply filters with `|` inside `{{}}`:

```html
{{name | uppercase}}                   <!-- ALICE -->
{{title | truncate(50)}}               <!-- First 50 chars... -->
{{description | escape}}               <!-- HTML-escaped -->
{{data | json}}                        <!-- JSON serialized -->
{{url | url_encode}}                   <!-- URL-encoded -->
{{name | default("Anonymous")}}        <!-- Default if empty/None -->
```

**Available filters:** `uppercase`/`upper`, `lowercase`/`lower`, `capitalize`, `trim`, `truncate(n)`, `replace(old, new)`, `escape`, `json`, `url_encode`, `safe`/`raw`, `default(val)`, `length`, `first`, `last`, `reverse`, `join(sep)`, `slice(start, end)`, `number(decimals)`

### Template Loop Metadata

Inside `{{#for item in items}}` blocks:

| Variable | Description |
|----------|-------------|
| `@index` | 0-based index |
| `@index1` | 1-based index |
| `@length` | Total items |
| `@first` | true if first item |
| `@last` | true if last item |
| `@even` | true if even index |
| `@odd` | true if odd index |

```html
{{#for item in items}}
<div class="item {{#if @last}}last{{/if}}">
    {{@index1}}. {{item.name}}
</div>
{{/for}}
```

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

> File-based route handlers return whatever your function returns. If you return `json(...)`, it's a JSON endpoint. If you return `html(template(...))`, it's a page. There is no auto-template-loading in this routing model. `routes/api/` is still a useful organizational convention for JSON endpoints, but not a runtime special case.

---

## Stdlib Prelude (Auto-Available Functions)

The most-used stdlib functions are auto-injected — no import needed:

| Module | Auto-available functions |
|--------|------------------------|
| `std/string` | `split`, `trim`, `contains`, `replace`, `join`, `starts_with`, `ends_with`, `to_lower`, `to_upper` |
| `std/json` | `parse_json` (also re-exported by `std/http/server`), `stringify` |
| `std/collections` | `keys`, `values`, `entries`, `has_key`, `get_key`, `reverse`, `sort` |
| `std/http/server` | `json`, `html`, `text`, `redirect`, `status`, `not_found`, `error`, `parse_form` |
| `std/env` | `get_env`, `load_env` |
| `std/time` | `now`, `format` |
| `std/crypto` | `uuid`, `sha256` |

**NOT in prelude** (still need explicit import): `fetch` (std/http), `std/net` IPAM/probe helpers, `connect`/`query`/`execute` (database modules), `set_cookie`/`get_cookie`/`with_cookie` (std/http/server), `sort_by`/`first`/`last`/`push`/`pop` (std/collections), KV, jobs, fs, csv, concurrent.

Explicit imports still work — prelude just makes them unnecessary for common functions.

---

## libs() — Auto-Import Directory

```ntnt
libs("lib/")   // All exports from lib/*.tnt injected flat into current scope
```

Replaces verbose per-file imports:
```ntnt
// Instead of:
import { SITES, TZ_OFFSET } from "./lib/config.tnt"
import { parse_data } from "./lib/parser.tnt"
import { round_1dp } from "./lib/helpers.tnt"

// Just use:
libs("lib/")
// All exported names from all .tnt files in lib/ are now available
```

> **⚠️ `libs()` only affects the calling file's scope.** Route files loaded by `routes()` have isolated scopes and do NOT see `libs()` exports. Route files must use explicit imports: `import { helper } from "../lib/module.tnt"`

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

Boundary rule: use `std/auth` for auth flows, sessions, CSRF, current-user helpers, and TOTP. Use `std/crypto` for generic crypto helpers like `uuid`, `hash_password`, and `verify_password`.

Full OAuth, session management, CSRF, JWT, TOTP, and local credential bootstrap, setup completion, password reset, and verification support.

### Local Auth Quickstart

The 0.4.9 local-auth path is intentionally explicit: `std/auth` owns credentials, reset tokens, TOTP state, sessions, cookies, CSRF, and route protection; the app owns the forms, delivery, roles, and policy copy. Local credentials and reset tokens are stored by the configured auth backend: memory, SQLite, PostgreSQL, or Redis/Valkey.

Supported local identifier kinds are:

- `email` — default; normalized case-insensitively.
- `phone` — accepts digits plus common separators and stores a normalized E.164-ish string.
- `username` — lowercase 3-64 character usernames using letters, digits, `_`, `-`, and `.`.
- `custom` — app-defined opaque identifiers, trimmed and rejected if empty/control-bearing.

```ntnt
import {
    bootstrap_local_user,
    consume_password_reset,
    enable_auth,
    issue_password_reset,
    logout_all,
    require_auth,
    sign_in_session,
    update_local_user_metadata,
    verify_local_password
} from "std/auth"
import { get_env } from "std/env"
import { json, parse_form, redirect } from "std/http/server"

enable_auth([], map {
    "session_secret": get_env("SESSION_SECRET"),
    "session_store": get_env("AUTH_STORE") ?? "sqlite:./auth.db",
    "login_url": "/login",
    "logout_url": "/"
})

fn create_invited_user(email, temporary_password) {
    // Server-side/admin code. Apps decide who may call this.
    let user = bootstrap_local_user(email, temporary_password, map {
        "identifier_kind": "email"
    })?
    update_local_user_metadata(email, map {
        "app": map { "group_ids": ["users"] }
    })?
    return user
}

fn post_login(req) {
    let form = parse_form(req)
    let verified = verify_local_password(form["email"] ?? "", form["password"] ?? "")?

    return sign_in_session(redirect("/dashboard"), req, map {
        "subject_id": verified["subject_id"],
        "email": verified["email"],
        "data": map { "group_ids": app_group_ids_for_local_user(verified) }
    })
}

fn post_password_reset_request(req) {
    let form = parse_form(req)
    let reset = issue_password_reset(form["email"] ?? "")?

    if reset["token"] != None {
        // App-owned delivery. Do not log or persist the token.
        send_reset_email(form["email"] ?? "", reset["token"])
    }

    return redirect("/password-reset/sent")
}

fn post_password_reset(req) {
    let form = parse_form(req)
    let user = consume_password_reset(
        form["token"] ?? "",
        form["new_password"] ?? "",
        map { "revoke_sessions": form["logout_all_devices"] == "on" }
    )?

    return sign_in_session(redirect("/dashboard"), req, map {
        "subject_id": user["subject_id"],
        "email": user["email"]
    })
}

fn post_logout_all(req) {
    let auth_response = require_auth(req)
    if typeof(auth_response) == "Map" { return auth_response }

    // Use this behind UI like “log me out of all active sessions”.
    return logout_all(req, false)
}
```

Password reset does **not** revoke sessions by default. If the reset form has a “log me out of all active sessions” checkbox, pass `map { "revoke_sessions": true }` to `consume_password_reset(...)`. For a standalone account-security page, use `logout_all(req, keep_current)`; pass `false` to revoke every active session including the current one, or `true` to keep the current session and revoke the rest.

### Local Credential Bootstrap and Setup Completion

`std/auth` owns local identity/credential lifecycle state. Use `bootstrap_local_user(...)` to provision a setup credential, then call `set_local_password(...)` with that current setup/forced-change credential plus a different replacement password to rotate it and clear setup-required state before granting regular access. Both helpers return only safe local-user metadata; they never expose passwords, password hashes, hash parameters, tokens, or raw credential records.

```ntnt
import { bootstrap_local_user, set_local_password, sign_in_session } from "std/auth"
import { parse_form, redirect } from "std/http/server"

fn provision_first_admin(email, setup_password) {
    return bootstrap_local_user(email, setup_password)?
}

fn complete_setup(req) {
    let form = parse_form(req)
    let user = set_local_password(
        form["email"] ?? "",
        form["setup_password"] ?? "",
        form["new_password"] ?? ""
    )?

    return sign_in_session(redirect("/admin"), req, map {
        "subject_id": user["subject_id"],
        "email": user["email"],
        "claims": app_claims_for_local_user(user)
    })
}
```

### Local Credential Verification

`std/auth` owns the local credential verification path; `std/crypto` remains the place for generic password hash helpers. After a local identity/credential has been provisioned by auth-owned setup/bootstrap flows, custom login UI should verify credentials, derive app-owned claims, then complete the session through the shared request-aware session primitive.

```ntnt
import { verify_local_password, sign_in_session } from "std/auth"
import { parse_form, redirect } from "std/http/server"

fn login(req) {
    let form = parse_form(req)
    let verified = verify_local_password(form["email"] ?? "", form["password"] ?? "")?

    return sign_in_session(redirect("/admin"), req, map {
        "subject_id": verified["subject_id"],
        "email": verified["email"],
        "claims": app_claims_for_local_user(verified)
    })
}
```

Session claims, roles, profiles, and organization membership stay app-owned; local auth only owns credential lifecycle state and safe verification/setup results.

### Local Password Reset

Use `issue_password_reset(...)` and `consume_password_reset(...)` for reset links instead of storing reset state in app metadata or generic auth challenges. Issuance stores only a hashed verifier plus selector in auth-owned reset-token storage for resettable local identities. The raw `selector.verifier` token is returned once so the app can email it or render a setup link; syntactically valid requests receive the same response shape whether or not a matching account exists. Missing, malformed, disabled, locked, and expired accounts/tokens use generic responses/errors to avoid account enumeration.

```ntnt
import { consume_password_reset, issue_password_reset, sign_in_session } from "std/auth"
import { parse_form, redirect } from "std/http/server"

fn request_password_reset(req) {
    let form = parse_form(req)
    let reset = issue_password_reset(form["email"] ?? "")?

    // `token` is present for valid-shaped requests, not as an account-existence signal.
    // Keep the same UI either way; send the link out-of-band if configured.
    if reset["token"] != None {
        send_reset_email(form["email"] ?? "", reset["token"])
    }

    return redirect("/password-reset/sent")
}

fn finish_password_reset(req) {
    let form = parse_form(req)
    let user = consume_password_reset(
        form["token"] ?? "",
        form["new_password"] ?? "",
        map { "revoke_sessions": form["logout_all_devices"] == "on" }
    )?

    return sign_in_session(redirect("/admin"), req, map {
        "subject_id": user["subject_id"],
        "email": user["email"],
        "claims": app_claims_for_local_user(user)
    })
}
```

Reset tokens are one-time records. Never log the returned token, store it in `local_user.metadata`, or echo it through unrelated API responses. If a token is missing, expired, malformed, replayed, or has a wrong verifier, `consume_password_reset(...)` returns the same `Err("Invalid password reset token")`. Existing sessions are preserved unless the app explicitly passes `map { "revoke_sessions": true }`; use that option for a reset-page checkbox like “log me out of all active sessions.”

### Local TOTP Enrollment and Verification

Use the local TOTP helpers when an app needs MFA without maintaining its own TOTP table. `begin_totp_enrollment(...)` stores pending secret material under the reserved `auth.totp` metadata namespace and returns an `otpauth://` setup URI for QR-code display. `confirm_totp_enrollment(...)`, `verify_local_totp(...)`, `totp_status(...)`, and `reset_totp(...)` return secret-free status maps; safe user payloads never expose pending or confirmed TOTP secrets.

The setup URI is secret-bearing because authenticator apps need it to enroll. Render it only in the setup response; do not log it, persist it in app metadata, cache it, or expose it through API responses unrelated to setup.

For login, keep password-verified users in a staged auth challenge until TOTP verification succeeds. Do not call `sign_in_session(...)` for a TOTP-required account until the second factor is complete. Pair TOTP routes with the app's normal rate limiting/backoff; `verify_local_totp(...)` verifies the code but does not own account lockout policy.

Staged auth challenges are not authenticated sessions, so session CSRF helpers (`csrf_field`, `verify_csrf`) are intentionally not enough for pre-session forms. `begin_auth_challenge(...)` creates a challenge-bound CSRF nonce automatically. Render it with `auth_challenge_csrf_field(req, kind)` and verify submissions with `verify_auth_challenge_csrf(req, form["_csrf"], kind)` before mutating credentials, MFA state, or sessions.

```ntnt
import {
    auth_challenge_csrf_field,
    begin_auth_challenge,
    begin_totp_enrollment,
    complete_auth_challenge,
    confirm_totp_enrollment,
    current_auth_challenge,
    current_user,
    sign_in_session,
    totp_status,
    verify_auth_challenge_csrf,
    verify_local_password,
    verify_local_totp
} from "std/auth"
import { html, parse_form, redirect } from "std/http/server"

fn start_totp_setup(req) {
    let user = current_user(req) otherwise return redirect("/login")
    let setup = begin_totp_enrollment(user["email"], map { "issuer": "Admin" })?
    return html(template("totp_setup.html", map { "uri": setup["uri"] }))
}

fn finish_totp_setup(req) {
    let user = current_user(req) otherwise return redirect("/login")
    let form = parse_form(req)
    confirm_totp_enrollment(user["email"], form["code"] ?? "")?
    return redirect("/admin/security")
}

fn login(req) {
    let form = parse_form(req)
    let verified = verify_local_password(form["email"] ?? "", form["password"] ?? "")?
    let mfa = totp_status(form["email"] ?? "")?

    if mfa["enabled"] {
        return begin_auth_challenge(redirect("/login/totp"), map {
            "subject_id": verified["subject_id"],
            "kind": "mfa_pending",
            "data": map { "email": verified["email"] }
        })
    }

    return sign_in_session(redirect("/admin"), req, map {
        "subject_id": verified["subject_id"],
        "email": verified["email"]
    })
}

fn totp_login_page(req) {
    return html("""
<form method="post" action="/login/totp">
  {{{auth_challenge_csrf_field(req, "mfa_pending")}}}
  <input name="code" inputmode="numeric" autocomplete="one-time-code">
  <button type="submit">Verify</button>
</form>
""")
}

fn finish_totp_login(req) {
    let challenge = current_auth_challenge(req) otherwise return redirect("/login")
    if challenge["kind"] != "mfa_pending" { return redirect("/login") }
    let form = parse_form(req)
    if !verify_auth_challenge_csrf(req, form["_csrf"] ?? "", "mfa_pending") {
        return html("CSRF token missing or invalid", 403)
    }
    let email = challenge["data"]["email"] ?? ""
    verify_local_totp(email, form["code"] ?? "")?

    return complete_auth_challenge(redirect("/admin"), req, map {
        "subject_id": challenge["subject_id"],
        "email": email
    })
}
```

### Local Metadata and Group Authorization

Use `local_user(...)` and `update_local_user_metadata(...)` from trusted server-side code to read/update app-owned local identity metadata without creating a parallel auth model. Metadata is namespaced: `auth.*` is reserved for `std/auth` lifecycle helpers, while app data should live under namespaces such as `app` or `template`. Safe local-user payloads omit reserved auth metadata so TOTP/reset internals do not leak into templates or API responses.

For authorization, attach app-owned group IDs or claims during request-aware session completion, then use `has_group(...)` to gate pages and JSON/API endpoints. `std/auth` does not define RBAC policy; it only provides the session-data convention and helper.

```ntnt
import { has_group, require_auth, sign_in_session, update_local_user_metadata, verify_local_password } from "std/auth"
import { json, parse_form, redirect } from "std/http/server"

fn set_admin_group(email) {
    return update_local_user_metadata(email, map {
        "app": map { "group_ids": ["admins"] }
    })?
}

fn login(req) {
    let form = parse_form(req)
    let verified = verify_local_password(form["email"] ?? "", form["password"] ?? "")?
    let group_ids = app_group_ids_for_local_user(verified["subject_id"])

    return sign_in_session(redirect("/admin"), req, map {
        "subject_id": verified["subject_id"],
        "email": verified["email"],
        "data": map { "group_ids": group_ids }
    })
}

fn admin_api(req) {
    let auth_response = require_auth(req)
    if typeof(auth_response) == "Map" { return auth_response }
    if !has_group(req, "admins") {
        return json(map { "error": "forbidden" }, 403)
    }
    return json(map { "ok": true })
}
```

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

### KV Typed Helpers

One-liners that handle Result unwrap + string conversion + type parsing + default on failure:

```ntnt
import { open, get_int, get_float, get_json, get_str } from "std/kv"

let count = get_int(kv, "stats:visits", 0)          // Int, default 0
let rate = get_float(kv, "config:rate", 1.0)         // Float, default 1.0
let data = get_json(kv, "cache:user:1", map {})      // Parsed JSON, default empty map
let name = get_str(kv, "user:name", "Anonymous")     // String, default "Anonymous"
```

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

### Composition: parallel and race

```ntnt
import { parallel, race } from "std/concurrent"

// Run N functions concurrently, wait for all, return results in input order
// If any task fails (crash or returned Err), cancels all and returns that Err
let [a, b, c] = parallel([fn() { fetch(url1) }, fn() { fetch(url2) }, fn() { fetch(url3) }])
let results = parallel([]) // Empty array → []

// Race N functions — first Ok wins, cancel the rest
// Tasks that fail or return Err are skipped; all fail → returns last Err
let winner = race([fn() { fetch(primary) }, fn() { fetch(fallback) }])
```

Use `otherwise` with parallel for error handling:
```ntnt
let results = parallel([fn() { fetch(url) }]) otherwise { return [] }
```

---

## Background Jobs (`std/jobs`)

Persistent background job processing with retry, backoff, priority queues, rate limiting, concurrency limits, deduplication, and scheduled execution.

### Defining Jobs

```ntnt
job SendEmail on emails (retry: 5, backoff: "exponential") {
    perform(to: String, subject: String) {
        print("Sending to #{to}: #{subject}")
    }
    on_failure(error, attempt) {
        print("Failed (attempt #{attempt}): #{error}")
    }
}
```

**Syntax:** `job Name on queue_name (options) { perform(params) { body } on_failure(params) { body } }`

- `on queue_name` — assigns the job to a named queue (optional, defaults to `"default"`)
- `on_failure` block is optional — called on each failure with error message and attempt count

### Job Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `retry` | Int | 3 | Max retry attempts before marking dead |
| `backoff` | String | `"exponential"` | `"exponential"`, `"linear"`, or `"constant"` |
| `timeout` | Int | none | Seconds — post-execution elapsed check |
| `priority` | String or Int | `"normal"` (50) | `"critical"` (5), `"high"` (25), `"normal"` (50), `"low"` (85), or raw 0-99 |
| `rate` | String | none | Rate limit: `"100/minute"`, `"5/second"`, `"1000/hour"` |
| `concurrency` | Int | none | Max simultaneous instances of this job type |
| `unique` | Int | none | Dedup window in seconds (SHA-256 hash of type + args) |
| `expires` | Int | none | Seconds — job skipped if older than this when claimed |

```ntnt
// Full example with all options
job ProcessPayment on payments (
    retry: 3,
    backoff: "exponential",
    timeout: 120,
    priority: "critical",
    rate: "100/minute",
    concurrency: 5,
    unique: 3600,
    expires: 7200
) {
    perform(payment_id: String) {
        charge(payment_id)
    }
}
```

### Job Contracts (requires/ensures)

Jobs support ntnt's contract system for argument validation:

```ntnt
job ProcessPayment on payments (retry: 3)
    requires payment_id != ""
    ensures result != None
{
    perform(payment_id: String) {
        let charge = process_charge(payment_id)
        return charge
    }
}
```

- `requires` — checked before perform runs; violation fails the job immediately
- `ensures` — checked after perform completes; violation fails the job

### Multi-File Job Organization

Jobs follow the same progressive disclosure pattern as routes:

**Small app — everything in one file:**
```ntnt
// server.tnt
import { fetch } from "std/http"

job SendEmail on emails {
    perform(to, body) { fetch("https://api.mailgun.net/...", map { ... }) }
}

listen(8080)
```

**Medium app — jobs in a separate file, explicitly imported:**
```ntnt
// server.tnt
import "./lib/jobs.tnt"
listen(8080)
```

```ntnt
// lib/jobs.tnt
import { fetch } from "std/http"

job SendEmail on emails {
    perform(to, body) { fetch("https://api.mailgun.net/...", map { ... }) }
}
```

**Large app — auto-discovered job directory:**
```
my-app/
├── server.tnt
├── lib/
│   └── notifications.tnt
└── jobs/
    ├── send_email.tnt
    ├── process_order.tnt
    └── generate_report.tnt
```

```ntnt
// server.tnt
jobs("jobs/")          // auto-discover and register all jobs
routes("routes/")      // auto-discover and register all routes
listen(8080)
```

Each job file has its own imports (evaluated in the shared interpreter context):
```ntnt
// jobs/send_email.tnt
import { fetch } from "std/http"
import { notify } from "../lib/notifications.tnt"

job SendEmail on emails (retry: 3) {
    perform(to, subject, body) {
        fetch("https://api.mailgun.net/v3/...", map { ... })
        notify("Email sent to #{to}")
    }
}
```

`jobs()` works exactly like `routes()`:
- Recursively scans for `.tnt` files
- Evaluates each file (registering `job` declarations)
- Files sorted alphabetically for deterministic order
- Hot-reload picks up new/changed job files automatically (dev mode)
- Workers re-discover jobs on startup via the same `jobs()` call

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

### Priority Queues and Worker Bands

Each priority band gets its own independent thread pool. Critical jobs don't compete with low-priority jobs for workers.

```ntnt
job ResetPassword on auth (priority: "critical") {
    perform(user_id: String) { ... }
}

job WeeklyDigest on notifications (priority: "low") {
    perform(user_id: String) { ... }
}
```

Scale workers per band at runtime (no restart needed):
```ntnt
import { scale_workers, worker_status } from "std/jobs"

scale_workers("critical", 4)
scale_workers("low", 1)

let status = worker_status()
// { "bands": [...], "pending": 42 }
```

**CLI (via control socket):**
```bash
ntnt workers status server.tnt
ntnt workers scale critical 4
ntnt workers scale low 1
```

### Batch Enqueue

```ntnt
import { enqueue_batch } from "std/jobs"

// Enqueue multiple jobs atomically (up to 10,000)
let ids = unwrap(enqueue_batch("SendEmail", [
    map { "to": "alice@example.com" },
    map { "to": "bob@example.com" },
    map { "to": "charlie@example.com" }
]))
```

All-or-nothing validation: if any argument map is invalid, none are enqueued.

### Job Batches

Batches group jobs and fire callbacks when all jobs complete:

```ntnt
import { batch, enqueue, seal, batch_status, batch_id, enqueue_into } from "std/jobs"

// Create a batch with callbacks
let b = batch("import-users", map {
    "on_complete": "NotifyAdmin",
    "on_success": "SendReport",
    "on_death": "AlertOps"
})

// Buffer jobs (not written to KV yet)
enqueue(b, "ImportUser", map { "id": 1 })
enqueue(b, "ImportUser", map { "id": 2 })

// Seal flushes jobs to KV and starts processing
seal(b)
```

**Dynamic additions** — add jobs to a sealed batch from within a running job:

```ntnt
job ImportUser on imports {
    perform(id) {
        let children = fetch_children(id)
        let bid = unwrap(batch_id())  // Get current batch ID (panics if not in a batch)
        for child in children {
            enqueue_into(bid, "ImportChild", map { "child_id": child })
        }
    }
}
```

`enqueue_into()` atomically increments the batch's pending and total counters. Callbacks only fire after all jobs (including dynamically added ones) reach terminal state.

**Batch status:**

```ntnt
let s = batch_status(b)  // Returns map with status, pending, succeeded, dead, cancelled, total
```

**Batch expiry:** Sealed batches expire after 30 days. Completed batches expire after 24 hours.

### Queue Pause and Resume

```ntnt
import { pause_queue, resume_queue } from "std/jobs"

pause_queue("webhooks")   // Workers stop claiming jobs from this queue
resume_queue("webhooks")  // Workers resume
```

Pause state is durable (persisted to KV, survives restarts). Also available via CLI:
```bash
ntnt workers pause webhooks
ntnt workers resume webhooks
```

### Scaling: Separate Web and Worker Processes

For production, run web servers and workers as separate processes:

```bash
# Web server — handles HTTP, enqueues jobs
ntnt run server.tnt

# Workers — process jobs, no HTTP (same file, different entry point)
ntnt worker server.tnt --concurrency 10
ntnt worker server.tnt --concurrency 5 --queues emails
```

Or use a dedicated worker file:
```ntnt
// worker.tnt — jobs only, no HTTP
import "./lib/helpers.tnt"
jobs("jobs/")
configure_queue(map { "store": "redis://redis:6379" })
work_jobs(map { "concurrency": 10 })  // blocks until Ctrl-C
```

`ntnt worker` evaluates the source in Worker mode — `listen()`, `work_async()`, and `serve_static()` are automatically suppressed. See the [Deployment Guide](DEPLOYMENT_GUIDE.md) for full Docker Compose examples with web + worker + Redis + PostgreSQL.

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

## Cryptography (`std/crypto`)

```ntnt
import { sha256, uuid, hash_password, verify_password } from "std/crypto"
import { aes_encrypt, aes_decrypt, aes_generate_key } from "std/crypto"
import { base64_encode, base64_decode, hmac_sha256 } from "std/crypto"
import { argon2_hash, argon2_verify } from "std/crypto"

let hash = sha256("data")                          // SHA-256 hex string
let id = uuid()                                     // Random UUID v4
let pw_hash = hash_password("secret", 12)           // bcrypt (cost 12)
let valid = verify_password("secret", pw_hash)      // true

let key = aes_generate_key()                        // 256-bit AES key (hex)
let encrypted = aes_encrypt("plaintext", key)?      // AES-256-GCM
let decrypted = aes_decrypt(encrypted, key)?        // "plaintext"

let encoded = base64_encode("hello")                // "aGVsbG8="
let decoded = base64_decode(encoded)?               // "hello"
let sig = hmac_sha256("message", "secret-key")      // HMAC signature
```

---

## File System (`std/fs`)

```ntnt
import { read_file, write_file, append_file, exists, is_dir, is_file } from "std/fs"
import { list_dir, create_dir, remove, copy, file_size, file_stat } from "std/fs"

let content = read_file("config.json")?
write_file("output.txt", "hello")?
append_file("log.txt", "new entry\n")?

if exists("data/") && is_dir("data/") {
    let files = list_dir("data/")?
    for f in files { print(f) }
}

create_dir("uploads")?
copy("a.txt", "b.txt")?
let size = file_size("data.db")?
```

---

## Math (`std/math`)

Built-in: `abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `clamp`

```ntnt
import { sin, cos, tan, asin, acos, atan, atan2, log, log2, log10, exp, cbrt } from "std/math"

let x = sin(3.14159 / 2)   // ~1.0
let y = log(100)            // natural log
let z = atan2(1, 1)         // π/4
```

`round(value, decimals?)` — round to N decimal places: `round(3.14159, 2)` → `3.14`

---

## Time (`std/time`)

```ntnt
import { now, now_millis, format, parse_datetime, diff } from "std/time"
import { add_days, add_hours, add_months, to_timezone } from "std/time"
import { year, month, day, hour, minute, weekday, month_name, day_name } from "std/time"

let ts = now()                                      // Unix timestamp (seconds)
let formatted = format(ts, "%Y-%m-%d %H:%M:%S")    // "2026-04-02 18:30:00"
let parsed = parse_datetime("2026-04-02", "%Y-%m-%d")?  // Unix timestamp

let tomorrow = add_days(ts, 1)
let next_month = add_months(ts, 1)
let hours_diff = diff(start, end, "hours")

let mountain = to_timezone(ts, "America/Denver")
let wd = weekday(ts)                                // 0=Mon ... 6=Sun
let name = day_name(ts)                             // "Wednesday"
```

---

> **Complete function listings:** For every function signature, parameter, and example across all 21 modules, see [STDLIB_REFERENCE.md](STDLIB_REFERENCE.md) or run `ntnt docs <module>` (e.g., `ntnt docs std/time`).

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

## Common Patterns

Patterns agents will need frequently. Copy these — they work as-is.

### Database: Check if Row Exists

```ntnt
import { connect, query_one } from "std/db/postgres"

let db = unwrap(connect(get_env("DATABASE_URL")))

// query_one returns Result<Map | None, String>
// After `otherwise`, the value is Map (row found) or None (no match)
let user = query_one(db, "SELECT * FROM users WHERE email = $1", [email]) otherwise {
    return status(500, "Database error")
}

// Check for no match — use is_none() or == None
if is_none(user) {
    return status(404, "User not found")
}

// user is now a Map — access fields directly
let name = user["name"]
```

### Sessions: Login with Cookies

```ntnt
import { html, json, redirect, parse_form, set_cookie, get_cookie, with_cookie } from "std/http/server"
import { connect, query_one, execute } from "std/db/postgres"

let db = unwrap(connect(get_env("DATABASE_URL")))

fn login(req) {
    let form = parse_form(req)
    let user = query_one(db, "SELECT * FROM users WHERE username = $1", [form["username"]]) otherwise {
        return status(500, "Database error")
    }
    if is_none(user) {
        return html("<p>Invalid credentials</p>")
    }
    // Set session cookie and redirect
    let resp = redirect("/dashboard")
    return with_cookie(resp, "session", user["id"], map { "httpOnly": true, "path": "/" })
}

fn dashboard(req) {
    let session_id = get_cookie(req, "session") ?? redirect("/login")
    let user = query_one(db, "SELECT * FROM users WHERE id = $1", [session_id]) otherwise {
        return redirect("/login")
    }
    if is_none(user) {
        return redirect("/login")
    }
    return html(template("views/dashboard.html", map { "user": user }))
}

post("/login", login)
get("/dashboard", dashboard)
```

### Parse JSON POST Body

```ntnt
import { json, parse_json } from "std/http/server"

fn create_item(req) {
    let data = parse_json(req) otherwise {
        return status(400, "Invalid JSON: #{err}")
    }
    // data is a Map — access fields
    let name = data["name"]
    let price = data["price"]
    return json(map { "created": true, "name": name }, 201)
}

post("/items", create_item)
```

### Templates + Static Files

```ntnt
import { html } from "std/http/server"

// Serve CSS/JS/images from ./public/ at /assets/
serve_static("/assets", "./public")

fn home(req) {
    // template() loads views/home.html and renders with data
    // Templates use {{var}} for escaped output, {{{var}}} for raw HTML
    return html(template("views/home.html", map {
        "title": "Home",
        "items": query(db, "SELECT * FROM items", [])
    }))
}

get("/", home)
listen(8080)
```

### Request Object Fields

| Field | Type | Description |
|-------|------|-------------|
| `req.method` | String | HTTP method (GET, POST, etc.) |
| `req.path` | String | Request path (e.g. "/users/42") |
| `req.params` | Map | Route parameters (e.g. `req.params["id"]` for `/users/{id}`) |
| `req.query_params` | Map | Query string parameters (e.g. `?page=2`) |
| `req.headers` | Map | Request headers (lowercase keys) |
| `req.body` | String | Raw request body |
| `req.json` | Map \| None | Parsed JSON body (if Content-Type is JSON) |
| `req.form` | Map \| None | Parsed form body (if Content-Type is form) |
| `req.ip` | String | Client IP address |
| `req.id` | String | Unique request ID |
| `req.context` | Map | Middleware-injected context |

### Response Helpers

```ntnt
import { html, json, redirect, status, text, with_cookie, with_header } from "std/http/server"

html("<h1>Hello</h1>")             // 200 HTML
html("<h1>Hello</h1>", 201)        // Custom status code
json(map { "ok": true })           // 200 JSON
json(map { "ok": true }, 201)      // Custom status code
text("plain text")                 // 200 text/plain
redirect("/login")                 // 302 redirect
status(404, "Not found")           // Custom status + body
with_cookie(resp, "k", "v", map { "httpOnly": true })  // Add cookie
with_header(resp, "X-Custom", "value")                  // Add header
```

---

## Quick Reference Tables

### Global Builtins (No Import)

**Conversion & Output:**

| Function | Description |
|----------|-------------|
| `print(x)` | Output to stdout |
| `str(x)` | Convert to string |
| `int(x)` | Convert to integer → `Result<Int, String>` |
| `float(x)` | Convert to float → `Result<Float, String>` |
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
| `sort_by(arr, fn)` | Sort with custom comparator: `sort_by(arr, fn(a, b) { a - b })` |
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
| `libs(dir)` | Auto-import all exports from directory (flat injection into scope) |
| `jobs(dir)` | Auto-discover and register all jobs from directory |
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
import { channel, send, recv, sleep_ms, spawn, await_task, schedule, cancel_schedule, parallel, race } from "std/concurrent"
import { enqueue, enqueue_in, enqueue_at, enqueue_batch, enqueue_into, batch, seal, batch_id, batch_status, configure_queue, work_async, work_jobs } from "std/jobs"
import { job_status, cancel_job, retry_job, list_jobs, delete_jobs } from "std/jobs"
import { scale_workers, worker_status, pause_queue, resume_queue } from "std/jobs"
import { assert_enqueued, assert_not_enqueued, drain_jobs, clear_jobs } from "std/jobs"
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
| `db error` | Type mismatch in query | Convert and handle parse failures: `let age = int(form["age"]) otherwise { return status(400, "invalid age") }` |

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
