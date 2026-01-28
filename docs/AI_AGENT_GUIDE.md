# NTNT AI Agent Guide

Critical syntax rules and patterns for AI agents generating NTNT code. For complete reference documentation, see:

- **[STDLIB_REFERENCE.md](STDLIB_REFERENCE.md)** - All functions and modules
- **[SYNTAX_REFERENCE.md](SYNTAX_REFERENCE.md)** - Keywords, operators, types, templates
- **[IAL_REFERENCE.md](IAL_REFERENCE.md)** - Intent Assertion Language

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

## Intent-Driven Development (IDD)

IDD is **the core workflow** for NTNT development. You capture user requirements as executable specifications, then implement code that satisfies them.

### The IDD Mindset

When a user describes what they want to build, your job is to:
1. **Listen** for testable assertions (what should happen, what users should see)
2. **Capture** these as Glossary terms and Scenarios in a `.intent` file
3. **Present** the intent file for user approval before writing code
4. **Implement** with `@implements` annotations
5. **Verify** continuously with `ntnt intent check` or Intent Studio

### Workflow Steps

| Step | Action | User Input | What You Do |
|------|--------|------------|-------------|
| 1 | Listen to requirements | User describes features | Extract testable behaviors |
| 2 | Draft `.intent` file | No | Create Glossary + Features + Scenarios |
| 3 | **Present to user** | **YES - STOP HERE** | Show the intent file, ask for feedback |
| 4 | Refine based on feedback | Yes | Update Glossary and Scenarios |
| 5 | User approves | **YES** | Get explicit approval before coding |
| 6 | Generate scaffolding | No | Run `ntnt intent init` (optional) |
| 7 | Implement with `@implements` | No | Write code, link to features |
| 8 | Verify with Intent Studio | No | Run `ntnt intent studio` for live feedback |
| 9 | Final check | No | Run `ntnt intent check` to confirm all passing |

### Capturing User Requirements

When users describe what they want, listen for:

| User Says | Capture As |
|-----------|------------|
| "The home page should show a welcome message" | Scenario: `they see "Welcome"` |
| "Users need to log in" | Feature: User Login |
| "The API returns JSON" | Glossary: `returns JSON` → `content-type is json` |
| "It should be fast" | Glossary: `responds quickly` → `response time < 500ms` |
| "Only admins can delete" | Constraint: Admin Only, applies_to features |

### Building the Glossary

The Glossary is your **domain-specific vocabulary**. Build it from how the user naturally describes things:

```yaml
## Glossary

| Term | Means |
|------|-------|
# Navigation terms (how users describe going places)
| a user visits {path} | GET {path} |
| a visitor goes to {path} | GET {path} |
| the home page | / |
| the login page | /login |
| the dashboard | /dashboard |

# Success terms (how users describe things working)
| the page loads | status 200 |
| it works | status 200 |
| succeeds | status 200 |

# Content terms (what users should see)
| they see {text} | body contains {text} |
| they don't see {text} | body not contains {text} |
| shows {text} | body contains {text} |

# API terms (for JSON APIs)
| returns JSON | content-type is json |
| returns the {field} | body has field {field} |

# Error terms
| shows an error | status 4xx |
| page not found | status 404 |
| unauthorized | status 401 |

# Performance terms
| responds quickly | response time < 500ms |
```

### Writing Scenarios

Use the **When → outcomes** format with natural language that maps to your Glossary:

```yaml
Feature: User Dashboard
  id: feature.dashboard
  description: "Authenticated users can view their dashboard"

  Scenario: User views dashboard
    When a user visits the dashboard
    → the page loads
    → they see "Welcome back"
    → they see "Recent Activity"

  Scenario: Dashboard shows user data
    When a user visits the dashboard
    → they see their username
    → they see their email
```

**Scenario naming tips:**
- Use active voice: "User views dashboard" not "Dashboard is viewed"
- Be specific: "User sees welcome message" not "Page works"
- One scenario = one user story or behavior

### Intent File Complete Template

```yaml
# Project Name
# Run: ntnt intent check server.tnt

## Title
My Application Name

## Overview
Brief description of what this application does and who it's for.

## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the page loads | status 200 |
| they see {text} | body contains {text} |
| they don't see {text} | body not contains {text} |

---

Feature: Feature Name
  id: feature.feature_name
  description: "What this feature does for the user"

  Scenario: Descriptive scenario name
    When a user visits the home page
    → the page loads
    → they see "Expected content"

---

Constraint: Constraint Name
  description: "Cross-cutting concern that applies to multiple features"
  applies_to: [feature.feature_name, feature.other_feature]
```

### Code Annotations

Link your code to intent features:

```ntnt
// @implements: feature.homepage
fn home_handler(req) {
    return html("<h1>Welcome</h1>")
}

// @implements: feature.user_login
fn login_handler(req) {
    // Login logic
}

// @utility - Helper function, not a feature
fn hash_password(password) {
    // Utility code
}

// @internal - Implementation detail
fn validate_session(token) {
    // Internal logic
}

// @infrastructure - Setup/config code
fn setup_database() {
    // Database initialization
}
```

### Intent Commands

```bash
ntnt intent studio server.intent  # Visual preview with live tests (RECOMMENDED)
ntnt intent check server.tnt      # Run tests from command line
ntnt intent coverage server.tnt   # Show which features have implementations
ntnt intent init server.intent    # Generate code scaffolding from intent
```

**Use Intent Studio during development** - it shows live pass/fail indicators as you code!

### Unit Testing Functions with IAL

IAL supports unit testing individual functions using the `call:` keyword in glossary terms.

#### Glossary Syntax for Function Calls

```yaml
## Glossary

| Term | Means |
|------|-------|
# Unit test terms - MUST include source: to specify the .tnt file
| rounding {value} to 1dp | call: round_1dp({value}), source: myfile.tnt |
| extracting name from {data} | call: extract_name({data}), source: myfile.tnt |
| checking if {line} is valid | call: is_valid_line({line}), source: myfile.tnt |
```

**Key requirements:**
- `call: function_name({params})` - specifies the function to call with parameter placeholders
- `source: filename.tnt` - **REQUIRED** - specifies which .tnt file contains the function
- Parameters in `{braces}` are captured from the When clause and substituted

#### Writing Unit Test Scenarios

```yaml
Feature: Decimal Rounding
  id: feature.unit_round_1dp
  description: "Round values to one decimal place for display"

  Scenario: Rounds down correctly
    When rounding 45.24 to 1dp
    → result is "45.2"

  Scenario: Rounds up correctly
    When rounding 45.25 to 1dp
    → result is "45.3"

  Scenario: Handles negative values
    When rounding -5.67 to 1dp
    → result is "-5.7"
```

#### Result Assertions

| Assertion | Description |
|-----------|-------------|
| `result is {value}` | Exact equality check |
| `result equals {value}` | Exact equality check (alias) |
| `result.field is {value}` | Check a field on a map result |
| `result is true` / `result is false` | Boolean checks |

#### Complete Unit Test Example

```yaml
## Glossary

| Term | Means |
|------|-------|
| validating email {email} | call: is_valid_email({email}), source: validators.tnt |
| formatting date {date} | call: format_date({date}), source: utils.tnt |

---

Feature: Email Validation
  id: feature.unit_email_validation
  description: "Validate email address format"

  Scenario: Accepts valid email
    When validating email "user@example.com"
    → result is true

  Scenario: Rejects email without @
    When validating email "userexample.com"
    → result is false

  Scenario: Rejects empty string
    When validating email ""
    → result is false
```

#### Current Limitations

- **Array parameters**: Complex types like `[1, 2, 3]` may not parse correctly as function arguments
- **Nested results**: Deep field access like `result.nested.field` may have issues
- Keep unit test parameters simple (strings, numbers, booleans)

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

### 2. String Interpolation Uses `{expr}` NOT `${expr}`

```ntnt
// CORRECT
let msg = "Hello, {name}!"

// WRONG
let msg = "Hello, ${name}!"
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

### 6. Functions Not Methods (Use Pipe for Chaining)

```ntnt
// CORRECT - Standalone functions
len("hello")
str(42)
push(arr, item)
split(text, ",")

// WRONG - Method style
"hello".len()
arr.push(item)
```

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

---

## HTTP Server Pattern

**CRITICAL:** Routing functions are GLOBAL BUILTINS. Only response builders need importing.

```ntnt
// ONLY import response builders
import { json, html, parse_form, parse_json } from "std/http/server"

// Handler function (named - no inline lambdas)
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
req.params        // Map<String, String>: route params, req.params["id"]
req.query_params  // Map<String, String>: query string, req.query_params["name"]
req.headers       // Map<String, String>: headers map
req.body          // String: raw body
req.ip            // String: client IP (supports X-Forwarded-For)
req.id            // String: request ID (from X-Request-ID or auto-generated)
```

**Common mistakes:**
```ntnt
// WRONG - Do NOT import routing functions
import { listen, get, post } from "std/http/server"

// WRONG - No inline lambdas
get("/users/{id}", |req| { ... })

// WRONG - These don't exist
req.json       // Use parse_json(req)
req.form       // Use parse_form(req)
req.params.id  // Use req.params["id"]
```

### Environment Variables

| Variable | Values | Description |
|----------|--------|-------------|
| `NTNT_ENV` | `production`, `prod` | Disables hot-reload for better performance |
| `NTNT_STRICT` | `1`, `true` | Blocks execution on type errors (runs type checker before `ntnt run`) |

```bash
# Development (default) - hot-reload enabled
ntnt run server.tnt

# Production - hot-reload disabled
NTNT_ENV=production ntnt run server.tnt
```

**Hot-reload** watches your `.tnt` files and imported modules for changes, automatically reloading on the next request. Disable in production for zero filesystem overhead per request.

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

```ntnt
import { connect, query } from "std/db/postgres"

// Using match for explicit handling
let result = connect("postgres://...")
match result {
    Ok(db) => {
        // Use the connection
        let users = query(db, "SELECT * FROM users", [])
        match users {
            Ok(rows) => print("Found {len(rows)} users"),
            Err(e) => print("Query failed: {e}")
        }
    },
    Err(e) => print("Connection failed: {e}")
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
    return "Hello, {name}!"
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
- **`transform()` / `.map()` infer callback return** — `transform(Array<T>, fn(T)->R)` → `Array<R>` when callback is a typed named function
- **`html()`, `json()`, `text()`, `redirect()`** — all return `Response`
- **`parse_json()`** — returns `Result<Map<String, Any>, String>` (unwrap gives a map)
- **`fetch()`** — returns `Result<Response, String>` (unwrap gives `Response`)
- **`parse_datetime()`** — returns `Result<Int, String>`
- **`parse_csv()`** — returns `Array<Array<String>>`
- **Match arm narrowing** — `Ok(data)` on `Result<T, E>` binds `data` as `T`; `Some(x)` on `Option<T>` binds `x` as `T`; struct patterns bind field types
- **Cross-file imports** — `import { foo } from "./lib/utils"` resolves function signatures from the imported `.tnt` file

### What the Type Checker Catches

The type checker runs during `ntnt lint` and `ntnt validate`, and reports:

- **Argument type mismatches**: passing `Int` where `String` is expected
- **Wrong argument count**: calling `f(a, b)` with one argument
- **Return type mismatches**: returning `String` from a function declared `-> Int`
- **Let binding mismatches**: `let x: Int = "hello"`

```ntnt
fn greet(name: String) -> String {
    return "Hello, {name}!"
}

greet(42)  // Type error: expected String, got Int
```

### What It Does NOT Catch (Gradual Typing)

- Untyped parameters default to `Any` — compatible with everything
- Functions without return type annotations skip return checking
- Cross-file types from imported `.tnt` modules use `Any`
- No flow-sensitive narrowing (e.g., checking `None` before access)

**This means existing untyped code produces zero type errors.**

### Strict Mode

Strict mode warns about untyped function signatures and blocks execution on type errors. Three ways to activate:

1. **CLI flag:** `ntnt lint --strict`
2. **Environment variable:** `NTNT_STRICT=1`
3. **Project config:** Create `ntnt.toml` in project root:

```toml
[lint]
strict = true
```

```bash
# Lint with strict warnings (untyped params, missing return types)
ntnt lint --strict server.tnt

# Block execution on type errors
NTNT_STRICT=1 ntnt run server.tnt

# Also blocks hot-reload — keeps previous working version running
NTNT_STRICT=1 ntnt run server.tnt
```

### Where Type Checking Runs

| Command | Type Checker | Blocks? |
|---------|-------------|---------|
| `ntnt lint` | Always runs | Reports diagnostics (never blocks) |
| `ntnt lint --strict` | Runs + warns on untyped signatures | Reports diagnostics (never blocks) |
| `ntnt validate` | Always runs | Type errors count as validation failures |
| `ntnt run` | Only with `NTNT_STRICT=1` | Blocks execution if type errors found |
| Hot-reload | Only with `NTNT_STRICT=1` | Blocks reload, keeps previous version |

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
    print("Name: {user[\"name\"]}")
}

close(db)
```

### PostgreSQL

```ntnt
import { connect, query, execute, close } from "std/db/postgres"

let db = unwrap(connect("postgres://user:pass@localhost/mydb"))

// Parameterized queries ($1, $2 placeholders)
let users = unwrap(query(db, "SELECT * FROM users WHERE active = $1", [true]))
for user in users {
    print("Name: {user[\"name\"]}")
}

execute(db, "INSERT INTO users (name, age) VALUES ($1, $2)", [name, int(age_str)])

close(db)
```

**Type conversion for database:**
```ntnt
let form = parse_form(req)
let age = int(form["age"])     // Convert string to int!
let price = float(form["price"])

// WRONG - String to integer column causes "db error"
execute(db, "INSERT INTO users (age) VALUES ($1)", [form["age"]])
```

---

## Template Strings

Triple-quoted with `{{expr}}` interpolation (CSS-safe):

```ntnt
let page = """
<style>h1 { color: blue; }</style>
<h1>Hello, {{name}}!</h1>

{{#for item in items}}
<p>{{item.name}}: ${{item.price}}</p>
{{/for}}

{{#if logged_in}}
<a href="/logout">Logout</a>
{{#else}}
<a href="/login">Login</a>
{{/if}}
"""
```

**Available filters:** `uppercase`, `lowercase`, `capitalize`, `trim`, `truncate(n)`, `escape`, `json`, `url_encode`

**Loop metadata:** `@index`, `@length`, `@first`, `@last`, `@even`, `@odd`

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
    print("Request: {req.method} {req.path}")
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

---

## Debugging

NTNT doesn't have a debugger. Use these strategies:

1. **Print statements:** `print("Debug: {variable}")`
2. **Contracts:** Add `requires`/`ensures` to catch invalid states
3. **Lint first:** `ntnt lint` catches most syntax errors
4. **Intent Studio:** Shows live test results as you code

---

## Quick Reference Tables

### Global Builtins (No Import)

| Function | Description |
|----------|-------------|
| `print(x)` | Output to stdout |
| `len(x)` | Length of string/array |
| `str(x)` | Convert to string |
| `int(x)` | Convert to integer |
| `float(x)` | Convert to float |
| `type(x)` | Get type name |
| `push(arr, item)` | Add to array |
| `filter(arr, fn)` | Filter array with predicate |
| `transform(arr, fn)` | Transform array elements |
| `assert(cond)` | Assert condition |
| `abs(n)`, `min(a,b)`, `max(a,b)` | Math functions |
| `round(n)`, `round(n, decimals)`, `floor(n)` | Rounding |
| `get/post/put/patch/delete(pattern, handler)` | HTTP routes |
| `listen(port)` | Start server |
| `serve_static(prefix, dir)` | Static files |
| `routes(dir)` | File-based routing |
| `template(path, vars)` | Load template |
| `use_middleware(fn)` | Add middleware |
| `on_shutdown(fn)` | Cleanup handler |

### Common Imports

```ntnt
import { split, join, trim, replace, contains } from "std/string"
import { json, html, text, redirect, status, not_found, error, response, parse_form, parse_json } from "std/http/server"
import { connect, query, execute, close } from "std/db/postgres"
import { fetch, download } from "std/http"
import { read_file, write_file, exists } from "std/fs"
import { parse_json, stringify } from "std/json"
import { get_env, load_env } from "std/env"
import { now, format } from "std/time"
import { sha256, uuid } from "std/crypto"
import { first, last, keys, values, entries, has_key, get_key } from "std/collections"
```

### CLI Commands

```bash
ntnt run <file>              # Run a .tnt file
ntnt lint <file>             # Check for errors
ntnt intent check <file>     # Verify code matches intent
ntnt intent studio <intent>  # Visual studio with live tests
ntnt intent coverage <file>  # Show feature coverage
ntnt intent init <intent>    # Generate scaffolding
ntnt inspect <file>          # Project structure as JSON
ntnt docs [query]            # Search stdlib documentation
ntnt test <file> --get /     # Quick HTTP endpoint testing
```

---

## Troubleshooting

### Error Message Format

NTNT provides context-rich error messages with error codes, source snippets, and suggestions:

- **Error codes (E001-E012):** Every error type has a unique code (e.g., `E001` for undefined variables, `E003` for arity mismatches). These are color-coded red in terminal output.
- **"Did you mean?" suggestions:** Typos in variable or function names trigger Levenshtein-distance-based suggestions (shown in green). For example, writing `usr` when `user` is defined will suggest the correct name.
- **Source code snippets:** Parser errors display 3 lines of context around the error location with line and column numbers (line numbers shown in blue).
- **Function names in arity errors:** When calling a function with the wrong number of arguments, the error message includes the function name for easier debugging.

Example error output:
```
Error[E001]: Undefined variable `usr`
  --> server.tnt:45:12
   |
45 |     return json(usr)
   |
   help: did you mean `user`?
```

### Common Parse Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `unexpected token '{'` | Using `{}` for map literal | Add `map` keyword: `map { "key": "value" }` |
| `unexpected token '$'` | Using `${expr}` interpolation | Use `{expr}` without the `$` |
| `expected identifier` | Inline lambda in route | Use named function: `fn handler(req) { ... }` |
| `unexpected token '.'` | Method-style call | Use function style: `len(s)` not `s.len()` |

### Common Runtime Errors

| Error | Cause | Fix |
|-------|-------|-----|
| `requires clause failed` | Precondition not met | Check input values meet contract requirements |
| `ensures clause failed` | Postcondition not met | Fix function to return correct values |
| `key not found` | Missing map key | Use `has_key()` to check, or `get_key()` for Option |
| `index out of bounds` | Array index invalid | Check `len()` before accessing |
| `db error` | Type mismatch in query | Convert types: `int(form["age"])` for integers |

### Contract Violations in HTTP Routes

When contracts fail in HTTP handlers:

- **`requires` fails** → Returns `400 Bad Request` with contract message
- **`ensures` fails** → Returns `500 Internal Server Error` with contract message

Example:
```ntnt
fn create_user(req)
    requires len(req.body) > 0  // 400 if body is empty
{
    // ...
}
```

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

1. **Always lint first**: `ntnt lint file.tnt` catches 90% of issues (including type errors)
2. **Use print statements**: `print("Debug: {variable}")`
3. **Check types**: `print("Type: {type(variable)}")`
4. **Add type annotations**: Helps the type checker catch more bugs
5. **Add contracts**: They catch bugs at precise locations
6. **Use Intent Studio**: Live feedback as you code

See [STDLIB_REFERENCE.md](STDLIB_REFERENCE.md) for complete function documentation.
