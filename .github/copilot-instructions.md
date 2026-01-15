# NTNT Language Copilot Instructions

When generating or editing `.tnt` files (NTNT language), follow these critical syntax rules:

## ⚠️ ALWAYS Lint/Test Before Run

Before running any `.tnt` file, validate first:

```bash
ntnt lint myfile.tnt    # Always do this first
ntnt run myfile.tnt     # Only after lint passes

# For HTTP servers - test automatically
ntnt test server.tnt --get /api/status --post /users --body 'name=Alice'
```

## 🎯 Intent-Driven Development (IDD) - COLLABORATIVE Workflow

**CRITICAL: IDD is a collaborative process.** The intent file is a shared artifact developed TOGETHER with the user before implementation.

### Intent Studio - Visual Intent Development

For the best collaborative experience, use **Intent Studio** - a beautiful live preview of intent files with **live test execution**:

```bash
ntnt intent studio server.intent                        # Studio on :3001, app auto-starts on :8081
ntnt intent studio server.intent --port 4000            # Custom studio port
ntnt intent studio server.intent --app-port 9000        # Custom app port
```

**Default ports:**

- Studio runs on `http://127.0.0.1:3001`
- App auto-starts on `http://127.0.0.1:8081`

**Intent Studio automatically:**

- Starts your app server from the matching `.tnt` file (e.g., `server.intent` → `server.tnt`)
- Runs tests against the live app and shows pass/fail status
- Auto-refreshes every 2 seconds when you save changes
- Shows feature count, test cases, assertions, and **live pass/fail counts**
- Provides "Run Tests" button to re-execute anytime
- Stops the app server cleanly on Ctrl+C

**Recommended workflow:** Run `ntnt intent studio server.intent` and start editing - the app starts automatically with hot-reload enabled!

### Phase 1: Draft and Present Intent (STOP FOR USER INPUT)

When user asks to build something using IDD:

1. **Draft the `.intent` file** based on requirements (use correct format below!)
2. **Optionally start Intent Studio** for visual review: `ntnt intent studio <file>.intent`
3. **STOP and present it to the user** - do NOT proceed to implementation
4. **Ask clarifying questions** about requirements
5. **Wait for user approval** before implementing

Example response pattern:

```
"Here's a draft intent file based on your requirements:

[show intent file]

**Questions before I implement:**
- [clarifying question 1]
- [clarifying question 2]

Let me know your thoughts and I'll refine this before starting implementation."
```

### Intent File Format (CRITICAL - Use This Exact Format!)

Intent files use a specific YAML-like format. **The file must be named to match the .tnt file** (e.g., `server.intent` for `server.tnt`).

```intent
# Project Name
# Description of the project
# Run: ntnt intent check server.tnt

## Overview
Brief description of what this project does.

## Design
- Design decision 1
- Design decision 2

---

Feature: Feature Name
  id: feature.feature_id
  description: "Human-readable description of this feature"
  test:
    - request: GET /path
      assert:
        - status: 200
        - body contains "expected text"
        - body contains "another expected text"

Feature: Another Feature
  id: feature.another_feature
  description: "Description of this feature"
  content:
    - What this feature includes
    - More details
  test:
    - request: GET /another-path
      assert:
        - status: 200
        - body contains "something"
    - request: POST /api/data
      body: '{"key": "value"}'
      assert:
        - status: 201
        - body contains "created"

---

Constraint: Constraint Name
  description: "Description of the constraint"
  applies_to: [feature.feature_id, feature.another_feature]
```

**Key format rules:**

- Use `Feature:` (capitalized) followed by the feature name
- `id:` must be `feature.<snake_case_id>` - used for `@implements` annotations
- `test:` contains HTTP test assertions
- `request:` specifies HTTP method and path
- `assert:` is a list of assertions (status codes, body contains)
- Separate sections with `---`

### Phase 2: Implement (After User Approval)

Only after user approves the intent:

```bash
# Generate scaffolding from intent (creates .tnt stub file)
ntnt intent init project.intent -o server.tnt
```

Add `@implements` annotations to link code to features:

```ntnt
// @implements: feature.user_login
fn login_handler(req) {
    // Implementation
}

// @utility
fn validate_email(email) {
    // Helper function
}
```

### Phase 3: Verify (MANDATORY Before Declaring Success)

**ALWAYS run these before saying "done":**

```bash
ntnt lint server.tnt           # Check syntax
ntnt intent check server.tnt   # Verify against intent (auto-finds server.intent)
ntnt intent coverage server.tnt # Show coverage
```

### IDD Workflow Summary

| Step | Action                       | User Input Required |
| ---- | ---------------------------- | ------------------- |
| 1    | Draft `.intent` file         | No                  |
| 2    | **Present to user**          | **YES - STOP**      |
| 3    | Refine based on feedback     | Yes                 |
| 4    | User approves                | **YES**             |
| 5    | Run `ntnt intent init`       | No                  |
| 6    | Implement with `@implements` | No                  |
| 7    | Run `ntnt intent check`      | No                  |
| 8    | Present results              | No                  |

### Annotation Reference

- `// @implements: feature.X` - Links function to a feature
- `// @supports: constraint.X` - Links to a constraint
- `// @utility` - Helper function
- `// @internal` - Internal implementation
- `// @infrastructure` - Config/setup code

---

## Mandatory Syntax Rules

### Map Literals

Use `map {}` for top-level maps. Nested maps inside a `map {}` are inferred automatically.

```ntnt
// Correct - top level requires map keyword
let data = map { "name": "Alice", "age": 30 }

// Correct - nested maps inferred (cleaner)
let config = map {
    "server": { "host": "localhost", "port": 8080 },
    "db": { "url": "postgres://..." }
}

// Also correct - explicit nested maps (backwards compatible)
let config = map {
    "server": map { "host": "localhost" }
}

// Wrong - top-level {} is a block expression
let data = { "name": "Alice" }
```

### String Interpolation

Use `{expr}` directly in strings. Do not use `${expr}` or backticks.

```ntnt
// Correct
let msg = "Hello, {name}!"

// Wrong
let msg = `Hello, ${name}!`
```

### Route Patterns

Always use raw strings `r"..."` for HTTP route patterns containing parameters.

```ntnt
// Correct
get(r"/users/{id}", handler)

// Wrong - {id} interpreted as interpolation
get("/users/{id}", handler)
```

### String Escapes

NTNT supports standard escape sequences in regular strings:

```ntnt
// Supported escapes
let newline = "line1\nline2"      // \n = newline
let tabbed = "col1\tcol2"         // \t = tab
let quoted = "She said \"hi\""    // \" = quote
let path = "C:\\Users\\name"      // \\ = backslash
let brace = "literal \{brace\}"   // \{ \} = literal braces

// Raw strings for complex content (no escape processing)
let html = r#"<div class="main">Hello</div>"#
```

### Template Strings (Triple-Quoted)

For HTML templates with dynamic content, use `"""..."""` with `{{expr}}` interpolation. Single `{}` pass through (CSS-safe).

```ntnt
// Template with interpolation
let name = "Alice"
let page = """
<style>
    h1 { color: blue; }
</style>
<h1>Hello, {{name}}!</h1>
"""

// For loops in templates
let items = ["a", "b", "c"]
let list = """
<ul>
{{#for item in items}}
    <li>{{item}}</li>
{{/for}}
</ul>
"""

// Conditionals in templates
let show = true
let out = """
{{#if show}}
    <p>Visible</p>
{{#else}}
    <p>Hidden</p>
{{/if}}
"""

// Escape literal {{ with backslash
let code = """Use \{{ and \}} for literal braces"""
```

### Contract Placement

Place `requires` and `ensures` between return type and function body.

```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}
```

### Range Expressions

Use `..` for exclusive ranges, `..=` for inclusive. No `range()` function.

```ntnt
for i in 0..10 { }   // 0 to 9
for i in 0..=10 { }  // 0 to 10
```

### Imports

Use JavaScript-style imports with `/` path separators.

```ntnt
import { split, join } from "std/string"
import "std/math" as math
```

### Mutability

Variables are immutable by default. Use `let mut` for mutable variables.

```ntnt
let mut counter = 0
counter = counter + 1
```

### Functions Over Methods

Use standalone functions, not method calls.

```ntnt
len("hello")      // Correct
str(42)           // Correct
int("42")         // Correct - string to integer
push(arr, item)   // Correct

"hello".len()     // Wrong
```

### Safe Map Access with get_key

Use `get_key()` for safe map access instead of direct indexing:

```ntnt
import { get_key, has_key } from "std/collections"

let params = map { "name": "Alice" }

// With 2 args: returns Option (Some or None)
let name = get_key(params, "name")       // Some("Alice")
let age = get_key(params, "age")         // None

// With 3 args: returns value or default
let name = get_key(params, "name", "Unknown")  // "Alice"
let age = get_key(params, "age", 0)            // 0
```

### Null Coalescing Operator (??)

Use `??` to provide a default when a value is `None`:

```ntnt
import { get_key } from "std/collections"

// ?? unwraps Some values or returns the right side for None
let name = get_key(params, "name") ?? "Anonymous"
let age = get_key(params, "age") ?? 0

// Works with any Option value
let user = get_env("USER") ?? "guest"
let first = first(items) ?? default_item
```

## HTTP POST/Form Handling

Use `parse_query()` from `std/url` to parse form data:

```ntnt
import { parse_query } from "std/url"

fn post(req) {
    // parse_query converts "name=Alice&age=25" → map { "name": "Alice", "age": "25" }
    let form = parse_query(req.body)

    let name = form["name"]
    let age = int(form["age"])  // Convert to int for database!
}
```

## Type Conversion for Database

**Form fields are strings. Convert before database operations:**

```ntnt
// WRONG - causes "db error"
execute(db, "INSERT INTO users (age) VALUES ($1)", [form["age"]])

// CORRECT
let age = int(form["age"])
execute(db, "INSERT INTO users (age) VALUES ($1)", [age])
```

## Standard Library Reference

### Built-in (no import)

`print`, `len`, `str`, `int`, `float`, `abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `trunc`, `sign`, `clamp`, `Some`, `None`, `Ok`, `Err`, `unwrap`, `unwrap_or`, `is_some`, `is_none`, `is_ok`, `is_err`

### Environment Variables

```ntnt
import { get_env, load_env } from "std/env"

// Load from .env file (sets environment variables)
let _ = load_env(".env")  // Returns Result<(), String>

// Get individual variables
let db_url = match get_env("DATABASE_URL") {
    Some(url) => url,
    None => "postgres://localhost/db"
}
```

### HTTP Response Object

The `fetch()` and other HTTP functions return a `Result<Response, Error>`. The response object has:

```ntnt
match fetch("https://api.example.com") {
    Ok(response) => {
        response.status       // Int: 200, 404, etc.
        response.status_text  // String: "OK", "Not Found", etc.
        response.ok           // Bool: true if status 200-299
        response.headers      // Map: response headers
        response.body         // String: response body
        response.url          // String: final URL after redirects
        response.redirected   // Bool: true if request was redirected
    },
    Err(e) => print("Error: " + e)
}

// With timeout (seconds)
fetch(map { "url": "https://...", "timeout": 30 })

// With cache control and referrer
fetch(map {
    "url": "https://api.example.com",
    "cache": "no-cache",
    "referrer": "https://myapp.com/page",
    "referrerPolicy": "strict-origin"
})
```

### Common imports

```ntnt
import { split, join, trim, replace } from "std/string"
import { encode, decode, parse_query, build_query } from "std/url"
import { push, pop, keys, values, entries, has_key, get_key } from "std/collections"
import { fetch, post, get_json } from "std/http"
import { json, html, text, redirect, status } from "std/http/server"  // ONLY response builders!
import { connect, query, execute, close } from "std/db/postgres"
import { read_file, write_file, exists } from "std/fs"
import { parse, stringify } from "std/json"
import { parse, parse_with_headers, stringify, stringify_with_headers } from "std/csv"
import { sin, cos, tan, atan2, log, exp, random, random_int, PI, E } from "std/math"
import { now, format } from "std/time"
import { get_env } from "std/env"
import { channel, send, recv } from "std/concurrent"
```

## HTTP Server - Global Builtins vs Module Exports

**CRITICAL:** HTTP routing functions are GLOBAL BUILTINS. Only response builders need importing.

| Function                    | Type           | Import Needed?              |
| --------------------------- | -------------- | --------------------------- |
| `get(pattern, handler)`     | Global builtin | **No**                      |
| `post(pattern, handler)`    | Global builtin | **No**                      |
| `put(pattern, handler)`     | Global builtin | **No**                      |
| `delete(pattern, handler)`  | Global builtin | **No**                      |
| `listen(port)`              | Global builtin | **No**                      |
| `serve_static(prefix, dir)` | Global builtin | **No**                      |
| `use_middleware(fn)`        | Global builtin | **No**                      |
| `json(data)`                | Module export  | **Yes** - `std/http/server` |
| `html(content)`             | Module export  | **Yes** - `std/http/server` |
| `text(content)`             | Module export  | **Yes** - `std/http/server` |
| `redirect(url)`             | Module export  | **Yes** - `std/http/server` |

### HTTP Server Example

```ntnt
// ONLY import response builders - routing functions are global builtins!
import { json, html } from "std/http/server"

// Named handler functions (required - no inline lambdas)
fn get_user(req) {
    let user_id = req.params["id"]
    return json(map { "id": user_id })
}

fn home_page(req) {
    return html("<h1>Welcome</h1>")
}

// Routes - global builtins, no import needed
// MUST use raw strings r"..." for patterns with {param}
get("/", home_page)
get(r"/users/{id}", get_user)

// Static files and server start - also global builtins
serve_static("/static", "./public")
listen(8080)
```

### ❌ Common HTTP Server Mistakes

```ntnt
// ❌ WRONG - Do NOT import routing functions (they're global builtins)
import { listen, get, post } from "std/http/server"  // ERROR!
import { listen, get, post } from "std/http_server"  // ERROR! (wrong path)

// ❌ WRONG - No inline lambdas in route handlers
get(r"/users/{id}", |req| { ... })      // Parser error!
get(r"/users/{id}", fn(req) { ... })    // Parser error!

// ✅ CORRECT - Use named functions
fn handler(req) { ... }
get(r"/users/{id}", handler)
```

## Full Reference

See docs/AI_AGENT_GUIDE.md for complete syntax documentation and patterns.
