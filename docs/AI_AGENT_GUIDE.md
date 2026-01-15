# NTNT AI Agent Guide

This document provides critical syntax rules and patterns for AI agents generating NTNT code. Following these rules will prevent common errors and produce idiomatic code.

## ⚠️ MANDATORY Workflow: Lint, Test, and Verify Intent

**Before running ANY `.tnt` file, validate it first:**

```bash
# ALWAYS do this first - catches syntax errors
ntnt lint myfile.tnt

# Only after lint passes, run the file
ntnt run myfile.tnt

# For HTTP servers - test automatically without manual curl
ntnt test server.tnt --get /api/status --post /users --body 'name=Alice&age=25'
```

## 🎯 Intent-Driven Development (IDD) - COLLABORATIVE Workflow

**CRITICAL: IDD is a collaborative process between human and AI.** The intent file is a shared artifact that must be developed TOGETHER before any implementation begins.

### Native Hot-Reload

NTNT HTTP servers support native hot-reload. When you run `ntnt run server.tnt`, the server watches the source file for changes and automatically reloads on the next request - **no restart required**.

```bash
# Start your app with hot-reload enabled by default
ntnt run server.tnt

# Edit server.tnt in another terminal or editor
# Changes apply automatically on the next HTTP request!
```

This makes the IDD workflow seamless:

1. Start your app with `ntnt run`
2. Start Intent Studio to see test results
3. Edit code → save → tests re-run and pass/fail updates instantly

### Intent Studio - Visual Intent Development

For the best collaborative experience, use **Intent Studio** to preview intent files with **live test execution**:

```bash
ntnt intent studio server.intent                        # Starts BOTH studio AND app automatically!
ntnt intent studio server.intent --port 4000            # Custom studio port
ntnt intent studio server.intent --app-port 9000        # Custom app port
ntnt intent studio server.intent --no-open              # Don't auto-open browser
```

**Default ports:**

- Studio server: `http://127.0.0.1:3001`
- App server: `http://127.0.0.1:8081` (auto-started!)

**Intent Studio automatically:**

- 🚀 **Starts your app** from the matching `.tnt` file (e.g., `server.intent` → `server.tnt`)
- 🔥 **Hot-reload enabled** - edit your `.tnt` file and changes apply instantly
- ✅ **Live test execution** - tests run automatically against your app
- 🔴🟢 **Pass/fail indicators** on every assertion in real-time
- 🔄 Auto-refresh every 2 seconds when file changes
- ▶️ "Run Tests" button to re-execute anytime
- 🛑 Clean shutdown of app server on Ctrl+C

**One command to rule them all:**

```bash
ntnt intent studio server.intent
# That's it! Studio starts, app starts, browser opens, tests run.
```

### Phase 1: Draft and Present Intent (DO NOT SKIP)

When the user asks to build something using IDD:

1. **Draft the `.intent` file** based on user requirements (use correct format below!)
2. **Start Intent Studio** for visual review: `ntnt intent studio <file>.intent`
3. **STOP and present it to the user** for review - do NOT proceed to implementation
4. **Discuss and refine** the intent with the user
5. Only after user approval, proceed to Phase 2

### Intent File Format (CRITICAL - Use This Exact Format!)

**File must be named to match the .tnt file** (e.g., `server.intent` for `server.tnt`).

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

```markdown
# Example: Present intent to user BEFORE implementing

"Here's the draft intent file based on your requirements:

[show the intent file using the format above]

**Questions for refinement:**

- Should registration require email verification?
- What rate limit threshold makes sense (e.g., 5 attempts per minute)?
- Should we add password reset functionality?

Let me know your thoughts before I start implementing."
```

### Phase 2: Generate Scaffolding (After User Approval)

```bash
# Generate scaffolding from the approved intent (creates .tnt stub file)
ntnt intent init project.intent -o server.tnt
```

### Phase 3: Implement with Annotations

Link your code to intent features with `@implements` annotations:

```ntnt
// @implements: feature.user_authentication
fn register_user(req) {
    // Implementation here
}

// @implements: feature.user_authentication
// @supports: constraint.rate_limiting
fn login(req) {
    // Implementation here
}

// @utility
fn hash_password(password) {
    // Helper function (not linked to a feature)
}
```

### Phase 4: Verify Implementation

**ALWAYS run these commands before declaring success:**

```bash
ntnt lint server.tnt           # Check syntax first
ntnt intent check server.tnt   # Verify against intent (auto-finds server.intent)
ntnt intent coverage server.tnt # Show coverage
```

Example output from `ntnt intent check`:

```
Feature: User Authentication
  ✓ POST /register returns status 200 on success
  ✓ POST /login returns status 200 with valid credentials

1/1 features passing (2/2 assertions)
```

### IDD Workflow Summary

| Step | Action                               | Human Input Required |
| ---- | ------------------------------------ | -------------------- |
| 1    | Draft `.intent` file                 | No                   |
| 2    | Start Intent Studio (optional)       | No                   |
| 3    | **Present intent to user**           | **YES - STOP HERE**  |
| 4    | Refine based on feedback             | Yes                  |
| 5    | User approves intent                 | **YES**              |
| 6    | Run `ntnt intent init` (scaffolding) | No                   |
| 7    | Implement with `@implements`         | No                   |
| 8    | Run `ntnt intent check`              | No                   |
| 9    | Present results to user              | No                   |

### Intent Commands Reference

| Command                            | Purpose                              |
| ---------------------------------- | ------------------------------------ |
| `ntnt intent studio <file>.intent` | Visual preview with live refresh     |
| `ntnt intent check <file>.tnt`     | Run tests from intent file           |
| `ntnt intent init <file>.intent`   | Generate code scaffolding            |
| `ntnt intent coverage <file>.tnt`  | Show feature implementation coverage |

### Annotation Types

| Annotation                | Purpose                 | Example                               |
| ------------------------- | ----------------------- | ------------------------------------- |
| `@implements: feature.X`  | Links code to a feature | `// @implements: feature.login`       |
| `@supports: constraint.X` | Links to a constraint   | `// @supports: constraint.rate_limit` |
| `@utility`                | Marks helper functions  | `// @utility`                         |
| `@internal`               | Internal implementation | `// @internal`                        |
| `@infrastructure`         | Config/setup code       | `// @infrastructure`                  |

---

## Lint Commands

The `ntnt lint` command catches common mistakes like:

- JavaScript-style `${var}` interpolation (use `{var}` instead)
- Python-style `range()` calls (use `0..n` syntax)
- Missing `map {}` keyword for map literals
- Route patterns needing raw strings

The `ntnt test` command starts a server, makes HTTP requests, and shuts down automatically - perfect for testing APIs:

```bash
# Test multiple endpoints
ntnt test app.tnt --get /health --get /api/users/1 --verbose

# Test POST with form data
ntnt test app.tnt --post /users --body 'name=Alice&email=alice@test.com&age=30'

# Test PUT and DELETE
ntnt test app.tnt --put /api/users/1 --body 'name=Updated' --delete /api/users/999

# Custom port
ntnt test app.tnt --port 8080 --get /status
```

**This prevents 90% of debugging time.**

## Critical Syntax Rules

### 1. Map Literals Require `map` Keyword (at Top Level)

**Important:** In NTNT, `{}` creates a block expression, NOT a map/object. Use `map {}` for the top-level map.

However, **nested maps are inferred automatically** - inside a `map {}`, you can use plain `{}` for nested maps.

```ntnt
// ✅ CORRECT - Use `map {}` at top level
let user = map { "name": "Alice", "age": 30 }
let empty_map = map {}

// ✅ CORRECT - Nested maps are inferred (cleaner syntax)
let config = map {
    "server": { "host": "localhost", "port": 8080 },
    "database": { "url": "postgres://...", "pool": 5 }
}

// ✅ ALSO CORRECT - Explicit `map` for nested (backwards compatible)
let config = map {
    "server": map { "host": "localhost", "port": 8080 }
}

// ❌ WRONG - Top-level map still requires `map` keyword
let user = { "name": "Alice" }   // ERROR: This is a block, not a map
let empty = {}                    // This is an empty block, not empty map
```

### 2. String Interpolation Uses `{expr}` (Not `${expr}`)

```ntnt
// ✅ CORRECT - Direct curly braces in strings
let greeting = "Hello, {name}!"
let math = "Result: {a + b}"
let nested = "User {user.name} is {user.age} years old"

// ❌ WRONG - JavaScript/TypeScript style
let greeting = `Hello, ${name}!`     // Wrong: backticks and $
let greeting = "Hello, ${name}!"     // Wrong: $ prefix
```

### 3. Route Patterns Require Raw Strings

Route parameters use `{param}` which conflicts with string interpolation. Always use raw strings `r"..."` for routes:

```ntnt
// ✅ CORRECT - Raw strings for route patterns
get(r"/users/{id}", get_user)
post(r"/users/{user_id}/posts/{post_id}", create_post)
get(r"/api/v1/items/{category}/{item_id}", get_item)

// ❌ WRONG - {id} interpreted as variable interpolation
get("/users/{id}", get_user)          // ERROR: `id` is undefined
post("/users/{user_id}/posts", handler)  // ERROR
```

### 4. String Escape Sequences

NTNT supports standard escape sequences in regular strings:

```ntnt
// ✅ CORRECT - Escape sequences ARE supported
let with_newline = "line1\nline2\nline3"    // \n = newline
let with_tab = "col1\tcol2\tcol3"           // \t = tab
let with_quote = "She said \"hello\""        // \" = quote
let with_backslash = "path\\to\\file"        // \\ = backslash
let literal_brace = "use \{curly\} braces"   // \{ \} = literal braces

// Raw strings for complex content (no escape processing)
let html = r#"<div class="container">Hello</div>"#
let json = r#"{"name": "Alice", "age": 30}"#

// For multi-line content with lots of special characters
let template = r#"
<html>
    <body class="main">
        <h1>Welcome</h1>
    </body>
</html>
"#
```

**Supported escapes:** `\n` (newline), `\t` (tab), `\r` (carriage return), `\\` (backslash), `\"` (quote), `\'` (single quote), `\{` and `\}` (literal braces)

### 5. Template Strings (Triple-Quoted)

For HTML/template content, use triple-quoted strings `"""..."""` with `{{expr}}` interpolation. This is CSS-safe (single `{}` pass through).

```ntnt
// ✅ CORRECT - Template strings with {{}} interpolation
let name = "Alice"
let page = """
<!DOCTYPE html>
<style>
    h1 { color: blue; }    // Single braces pass through unchanged!
</style>
<body>
    <h1>Hello, {{name}}!</h1>
</body>
"""

// ✅ For loops in templates
let users = ["Alice", "Bob", "Charlie"]
let list = """
<ul>
{{#for user in users}}
    <li>{{user}}</li>
{{/for}}
</ul>
"""

// ✅ Conditionals in templates
let logged_in = true
let nav = """
<nav>
{{#if logged_in}}
    <a href="/profile">Profile</a>
{{#else}}
    <a href="/login">Login</a>
{{/if}}
</nav>
"""

// ✅ Complex expressions work too
let items = [map { "name": "Widget", "price": 99 }]
let store = """
{{#for item in items}}
<div>{{item["name"]}}: ${{item["price"]}}</div>
{{/for}}
"""

// ✅ Escape literal double braces with backslash
let code_sample = """
Use \{{ and \}} to output literal double braces.
"""
```

**Template String Rules:**

| Syntax                               | Result                 |
| ------------------------------------ | ---------------------- |
| `{{expr}}`                           | Interpolate expression |
| `{ ... }`                            | Literal (CSS/JS safe)  |
| `{{#for x in arr}}...{{/for}}`       | Loop over array        |
| `{{#if cond}}...{{/if}}`             | Conditional            |
| `{{#if cond}}...{{#else}}...{{/if}}` | If-else                |
| `\{{` and `\}}`                      | Literal `{{` and `}}`  |

### 6. Truthy/Falsy Values

NTNT supports truthy/falsy evaluation for cleaner conditionals. **Numbers (including 0) are always truthy** to avoid subtle bugs.

```ntnt
// ✅ CORRECT - Clean truthy checks
if query_string {           // Empty string is falsy
    process(query_string)
}

if results {                // Empty array is falsy
    return results[0]
}

if config {                 // Empty map is falsy
    apply(config)
}

// ❌ VERBOSE - These work but are unnecessary
if len(query_string) > 0 {  // Just use: if query_string
if len(results) > 0 {       // Just use: if results
```

**Truthy/Falsy Rules:**

| Value               | Truthy/Falsy |
| ------------------- | ------------ |
| `true`              | Truthy       |
| `false`             | Falsy        |
| `None`              | Falsy        |
| `Some(x)`           | Truthy       |
| `""` (empty string) | Falsy        |
| `"text"`            | Truthy       |
| `[]` (empty array)  | Falsy        |
| `[1, 2]`            | Truthy       |
| `map {}`            | Falsy        |
| `map { "a": 1 }`    | Truthy       |
| `0`, `0.0`          | **Truthy**   |
| Any number          | **Truthy**   |

**Why 0 is truthy:** Avoids bugs like `if count { }` failing when count is legitimately 0.

### 7. Contract Placement (requires/ensures)

Contracts go AFTER the return type but BEFORE the function body:

```ntnt
// ✅ CORRECT - Contract placement
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}

fn withdraw(amount: Int) -> Int
    requires amount > 0
    requires amount <= self.balance
    ensures result >= 0
    ensures self.balance == old(self.balance) - amount
{
    self.balance = self.balance - amount
    return self.balance
}

// ❌ WRONG - Contracts in wrong position
fn divide(a: Int, b: Int)
    requires b != 0          // Wrong: before return type
-> Int {
    return a / b
}

fn divide(a: Int, b: Int) -> Int {
    requires b != 0          // Wrong: inside function body
    return a / b
}
```

### 8. Range Expressions

```ntnt
// ✅ CORRECT - NTNT range syntax
for i in 0..10 { }      // 0 to 9 (exclusive end)
for i in 0..=10 { }     // 0 to 10 (inclusive end)
for i in 1..len(arr) { }

// ❌ WRONG - Python-style range function
for i in range(10) { }           // ERROR: range is not a function
for i in range(0, 10) { }        // ERROR
```

### 9. Import Syntax

NTNT uses JavaScript-style imports with quoted paths and `/` separators:

```ntnt
// ✅ CORRECT - NTNT import syntax
import { split, join, trim } from "std/string"
import { fetch, post } from "std/http"
import "std/math" as math
import { readFile as read } from "std/fs"

// Import from local files (relative paths)
import { helper } from "./lib/utils"
import { User } from "../models/user"

// ❌ WRONG - Other language styles
import std.string                    // Wrong: Python style
from std.string import split         // Wrong: Python style
use std::string::split;              // Wrong: Rust style
```

### 10. Mutable Variables

Variables are immutable by default. Use `mut` for mutability:

```ntnt
// ✅ CORRECT
let mut counter = 0
counter = counter + 1

let mut items = []
items = push(items, "new item")

// ❌ WRONG - Forgetting mut
let counter = 0
counter = counter + 1    // ERROR: cannot assign to immutable variable
```

### 11. Match Expression Syntax

Use `=>` (fat arrow) and commas between arms:

```ntnt
// ✅ CORRECT - Fat arrow and commas
match value {
    Ok(data) => process(data),
    Err(e) => handle_error(e),
}

match number {
    0 => "zero",
    n if n < 0 => "negative",
    _ => "positive",
}

// ❌ WRONG
match value {
    Ok(data) -> process(data)    // Wrong: thin arrow
    Err(e) => handle_error(e)    // Wrong: missing comma
}
```

### 12. Function Calls vs Methods

Many operations are standalone functions, not methods:

```ntnt
// ✅ CORRECT - Standalone functions
len("hello")              // Length of string
len(my_array)             // Length of array
str(42)                   // Convert to string
push(arr, item)           // Returns new array with item
split(text, ",")          // Split string

// ❌ WRONG - Method style (these don't exist)
"hello".len()             // ERROR
my_array.length           // ERROR
42.toString()             // ERROR
arr.push(item)            // May not work as expected
```

### 13. Result/Option Handling

Always handle `Result` and `Option` types explicitly:

```ntnt
// ✅ CORRECT - Pattern match or use helpers
match fetch("https://api.example.com") {
    Ok(response) => {
        if response.ok {
            print(response.body)
        } else {
            print("HTTP " + str(response.status) + ": " + response.status_text)
        }
    },
    Err(e) => print("Error: {e}"),
}

// Response object has: status, status_text, ok, headers, body, url, redirected
// response.ok is true for status 200-299
// response.url is the final URL after any redirects
// response.redirected is true if the request was redirected

// Using helper functions
let response = unwrap(fetch("https://api.example.com"))
let value = unwrap_or(optional_value, "default")

if is_ok(result) {
    let data = unwrap(result)
}

// ❌ WRONG - Treating Result as direct value
let response = fetch("https://api.example.com")
print(response.body)      // ERROR if fetch() returned Err
```

## Standard Library Quick Reference

### Built-in Functions (No Import Required)

| Function                                            | Description                 |
| --------------------------------------------------- | --------------------------- |
| `print(...)`                                        | Output to stdout            |
| `len(x)`                                            | Length of string/array      |
| `str(x)`                                            | Convert any value to string |
| `int(x)`                                            | Convert string to integer   |
| `float(x)`                                          | Convert string to float     |
| `abs(x)`                                            | Absolute value              |
| `min(a, b)`                                         | Minimum of two values       |
| `max(a, b)`                                         | Maximum of two values       |
| `sqrt(x)`                                           | Square root                 |
| `pow(base, exp)`                                    | Exponentiation              |
| `round(x)`, `floor(x)`, `ceil(x)`, `trunc(x)`       | Rounding functions          |
| `sign(x)`                                           | Sign (-1, 0, or 1)          |
| `clamp(x, min, max)`                                | Clamp to range              |
| `Some(v)`, `None`                                   | Option constructors         |
| `Ok(v)`, `Err(e)`                                   | Result constructors         |
| `unwrap(x)`, `unwrap_or(x, default)`                | Unwrap helpers              |
| `is_some(x)`, `is_none(x)`, `is_ok(x)`, `is_err(x)` | Type checks                 |

### Common Imports

```ntnt
// String operations
import { split, join, trim, replace, contains, starts_with, ends_with } from "std/string"

// URL encoding/decoding and form parsing
import { encode, decode, parse_query, build_query } from "std/url"

// Collections - Arrays and Maps
import { push, pop, first, last, reverse, slice, concat, is_empty } from "std/collections"
import { keys, values, entries, has_key } from "std/collections"  // Map iteration

// HTTP client
import { fetch, post, put, delete, get_json, post_json } from "std/http"

// HTTP server - ONLY response builders need importing
// NOTE: get(), post(), listen(), serve_static(), use_middleware() are GLOBAL BUILTINS (no import needed)
import { json, html, text, redirect, status } from "std/http/server"

// PostgreSQL database
import { connect, query, execute, close } from "std/db/postgres"

// File system
import { read_file, write_file, exists, is_file, is_dir, mkdir, readdir } from "std/fs"

// JSON
import { parse, stringify, stringify_pretty } from "std/json"

// CSV
import { parse, parse_with_headers, stringify, stringify_with_headers } from "std/csv"

// Math (trig, log, random)
import { sin, cos, tan, atan2, log, exp, PI, E } from "std/math"
import { random, random_int, random_range } from "std/math"

// Time
import { now, format, add_days, add_months } from "std/time"

// Environment
import { get_env, set_env, all_env } from "std/env"

// Concurrency
import { channel, send, recv, sleep_ms } from "std/concurrent"
```

## HTTP Server Patterns

### Global Builtins vs Module Exports

**CRITICAL:** HTTP routing functions are GLOBAL BUILTINS. Only response builders need importing.

| Function                    | Type           | Import Needed?              |
| --------------------------- | -------------- | --------------------------- |
| `get(pattern, handler)`     | Global builtin | No                          |
| `post(pattern, handler)`    | Global builtin | No                          |
| `put(pattern, handler)`     | Global builtin | No                          |
| `delete(pattern, handler)`  | Global builtin | No                          |
| `patch(pattern, handler)`   | Global builtin | No                          |
| `listen(port)`              | Global builtin | No                          |
| `serve_static(prefix, dir)` | Global builtin | No                          |
| `use_middleware(fn)`        | Global builtin | No                          |
| `json(data)`                | Module export  | **Yes** - `std/http/server` |
| `html(content)`             | Module export  | **Yes** - `std/http/server` |
| `text(content)`             | Module export  | **Yes** - `std/http/server` |
| `redirect(url)`             | Module export  | **Yes** - `std/http/server` |
| `status(code, body)`        | Module export  | **Yes** - `std/http/server` |

### Basic HTTP Server Example

```ntnt
// ONLY import response builders - routing functions are global
import { json, html } from "std/http/server"

// Define handler functions (named functions required, no inline lambdas)
fn get_user(req) {
    let user_id = req.params["id"]
    return json(map { "id": user_id })
}

fn create_user(req) {
    let body = parse_query(req.body)
    return json(map { "created": true, "name": body["name"] })
}

// Routes use global builtins - no import needed
// MUST use raw strings r"..." for patterns with {param}
get(r"/users/{id}", get_user)
post("/users", create_user)

// Static files - global builtin
serve_static("/static", "./public")

// Start server - global builtin
listen(8080)
```

### ❌ WRONG - Do NOT import routing functions

```ntnt
// ❌ WRONG - These will fail!
import { listen, get, post } from "std/http/server"  // ERROR: not exported
import { listen, get, post } from "std/http_server"  // ERROR: wrong module path
```

### ❌ WRONG - No inline lambdas in route handlers

```ntnt
// ❌ WRONG - Inline lambdas cause parser errors
get(r"/users/{id}", |req| {
    return json(map { "id": req.params["id"] })
})

// ❌ WRONG - fn(req) inline also doesn't work
get(r"/users/{id}", fn(req) {
    return json(map { "id": req.params["id"] })
})

// ✅ CORRECT - Use named functions
fn get_user(req) {
    return json(map { "id": req.params["id"] })
}
get(r"/users/{id}", get_user)
```

### HTTP Request Object Properties

**CRITICAL:** The request object (`req`) has specific properties. Do NOT assume other frameworks' conventions:

```ntnt
// ✅ CORRECT - Available request properties
req.method        // "GET", "POST", etc.
req.path          // "/users/123"
req.params        // Map of route parameters: params["id"] = "123"
req.query         // Map of query string: ?name=alice → query["name"] = "alice"
req.headers       // Map of headers: headers["content-type"]
req.body          // Raw request body as STRING (for POST/PUT)

// ❌ WRONG - These do NOT exist
req.form          // DOES NOT EXIST - use req.body and parse it
req.json          // DOES NOT EXIST as property - body is a string
req.data          // DOES NOT EXIST
req.params.id     // WRONG - use req.params["id"]
```

### Parsing Form Data (POST requests)

Use `parse_query()` from `std/url` to parse form data:

```ntnt
import { parse_query } from "std/url"

fn post(req) {
    // parse_query converts "name=Alice&email=alice%40example.com&age=30"
    // into map { "name": "Alice", "email": "alice@example.com", "age": "30" }
    let form = parse_query(req.body)

    let name = form["name"]     // "Alice"
    let email = form["email"]   // "alice@example.com" (auto URL-decoded)
    let age = int(form["age"])  // 30 (convert string to int for database)

    // ...process data...
}
```

**Note:** `parse_query()` automatically URL-decodes values (handles `%20`, `+`, etc.).

### Type Conversion Functions

**CRITICAL for database operations:** Form fields are always strings. Database columns may require specific types:

```ntnt
// ✅ Built-in conversion functions
int("42")         // Converts string to integer: 42
float("3.14")     // Converts string to float: 3.14
str(42)           // Converts integer to string: "42"
str(3.14)         // Converts float to string: "3.14"

// Common pattern: form field to database integer
let form = parse_query(req.body)
let age = int(form["age"])      // Convert "25" to 25 for DB
let user_id = int(form["id"])   // Convert "123" to 123 for DB

// ❌ WRONG - Passing string to integer column causes "db error"
execute(db, "INSERT INTO users (age) VALUES ($1)", [form["age"]])  // ERROR!

// ✅ CORRECT - Convert first
execute(db, "INSERT INTO users (age) VALUES ($1)", [int(form["age"])])  // Works!
```

## CSV Processing Patterns

```ntnt
import { parse, parse_with_headers, stringify, stringify_with_headers } from "std/csv"
import { fetch } from "std/http"

// Parse CSV into array of arrays
let csv_data = "name,age,city\nAlice,30,NYC\nBob,25,LA"
let rows = parse(csv_data)
// rows = [["name","age","city"], ["Alice","30","NYC"], ["Bob","25","LA"]]

// Parse CSV with first row as headers (returns array of maps)
let records = parse_with_headers(csv_data)
// records = [{"name": "Alice", "age": "30", "city": "NYC"}, ...]

for record in records {
    print("Name: " + record["name"] + ", Age: " + record["age"])
}

// Stringify array to CSV
let data = [["fruit", "count"], ["apple", "5"], ["banana", "3"]]
let csv_output = stringify(data)
// csv_output = "fruit,count\napple,5\nbanana,3"

// Stringify maps with headers
let maps = [
    map { "name": "Alice", "score": "95" },
    map { "name": "Bob", "score": "88" }
]
let headers = ["name", "score"]
let csv_from_maps = stringify_with_headers(maps, headers)
// csv_from_maps = "name,score\nAlice,95\nBob,88"

// Fetch and parse remote CSV
fn fetch_csv_data(url: String) {
    let response = get(url)
    let records = parse_with_headers(response.body)
    return records
}

// Custom delimiter (e.g., tab-separated)
let tsv = "name\tage\nAlice\t30"
let rows = parse(tsv, "\t")
```

**Note:** CSV field values are always strings. Use `int()` or `float()` to convert numeric fields.

## Contract Patterns

```ntnt
// Function with full contracts
fn transfer(from: Account, to: Account, amount: Int) -> Bool
    requires amount > 0
    requires from.balance >= amount
    ensures from.balance == old(from.balance) - amount
    ensures to.balance == old(to.balance) + amount
    ensures result == true
{
    from.balance = from.balance - amount
    to.balance = to.balance + amount
    return true
}

// Struct with invariant
struct BankAccount {
    balance: Int,
    owner: String
}

impl BankAccount {
    invariant self.balance >= 0

    fn deposit(self, amount: Int) -> Int
        requires amount > 0
        ensures self.balance == old(self.balance) + amount
    {
        self.balance = self.balance + amount
        return self.balance
    }
}
```

## Introspection with `ntnt inspect`

Use `ntnt inspect` to understand project structure programmatically:

```bash
# Inspect a file
ntnt inspect api.tnt --pretty

# Inspect a directory (auto-discovers routes)
ntnt inspect ./routes --pretty
```

Output includes:

- All functions with their contracts
- HTTP routes with handlers
- Module imports
- Struct definitions with invariants

## Common Patterns

### Defer for Cleanup

```ntnt
fn process_file(path: String) {
    let file = open(path)
    defer close(file)  // Always runs on scope exit

    // Work with file...
}
```

### Channel-based Concurrency

```ntnt
import { channel, send, recv, sleep_ms } from "std/concurrent"

let ch = channel()

// Sender
spawn(fn() {
    send(ch, "message")
})

// Receiver
let msg = recv(ch)
```

### Error Handling Pattern

```ntnt
fn fetch_user(id: String) -> Result<User, String> {
    let response = fetch("https://api.example.com/users/{id}")

    match response {
        Ok(r) => {
            if r.status == 200 {
                return Ok(parse(r.body))
            } else {
                return Err("User not found")
            }
        },
        Err(e) => return Err(e),
    }
}
```

## PostgreSQL Database Patterns

### Connection and Basic Operations

```ntnt
import { connect, query, execute, close } from "std/db/postgres"
import { get_env } from "std/env"

// Connect using environment variable (recommended)
fn get_db_connection() {
    let db_url = match get_env("DATABASE_URL") {
        Some(url) => url,
        None => "postgres://user:pass@localhost/mydb"
    }
    return connect(db_url)
}

// Usage
let db_result = get_db_connection()
match db_result {
    Ok(db) => {
        // ... use database ...
        close(db)  // Always close when done!
    }
    Err(e) => {
        print("Connection failed: {e}")
    }
}
```

### Query vs Execute

```ntnt
// query() - For SELECT statements that return rows
let users_result = query(db, "SELECT * FROM users WHERE active = $1", [true])
match users_result {
    Ok(users) => {
        for user in users {
            print("Name: {user[\"name\"]}, Age: {user[\"age\"]}")
        }
    }
    Err(e) => print("Query failed: {e}")
}

// execute() - For INSERT, UPDATE, DELETE (no rows returned)
let insert_result = execute(db,
    "INSERT INTO users (name, email, age) VALUES ($1, $2, $3)",
    [name, email, age]
)
match insert_result {
    Ok(_) => print("User added"),
    Err(e) => print("Insert failed: {e}")
}
```

### Parameter Binding

**CRITICAL:** Use `$1`, `$2`, etc. for parameters. Types must match column types:

```ntnt
// ✅ CORRECT - Parameters match column types
let name = "Alice"                    // String for VARCHAR
let age = int(age_str)                // Integer for INT column
let active = true                     // Boolean for BOOLEAN column

execute(db,
    "INSERT INTO users (name, age, active) VALUES ($1, $2, $3)",
    [name, age, active]
)

// ✅ CORRECT - Integer parameter for WHERE clause
let user_id = int(id_str)
execute(db, "DELETE FROM users WHERE id = $1", [user_id])

// ❌ WRONG - String passed to integer column causes "db error"
execute(db, "DELETE FROM users WHERE id = $1", [id_str])  // ERROR!
execute(db, "INSERT INTO users (age) VALUES ($1)", ["25"])  // ERROR!
```

### Query Results

Query results are arrays of maps. Access columns by name:

```ntnt
let users = unwrap(query(db, "SELECT id, name, email FROM users", []))

for user in users {
    let id = user["id"]         // Integer
    let name = user["name"]     // String
    let email = user["email"]   // String

    // Convert to string for display/concatenation
    let id_str = str(user["id"])
    print("User #{id_str}: {name}")
}
```

### Complete CRUD Example

```ntnt
import { connect, query, execute, close } from "std/db/postgres"
import { html } from "std/http_server"
import { parse_query } from "std/url"

// CREATE - POST handler
fn post(req) {
    let db = unwrap(connect("postgres://..."))

    let form = parse_query(req.body)
    let name = form["name"]
    let age = int(form["age"])  // Convert to int for database!

    execute(db, "INSERT INTO users (name, age) VALUES ($1, $2)", [name, age])
    close(db)

    return html(r#"<p>User created!</p>"#)
}

// READ - GET handler
fn get(req) {
    let db = unwrap(connect("postgres://..."))
    let users = unwrap(query(db, "SELECT * FROM users", []))
    close(db)

    // Build response...
}

// DELETE - POST handler (HTML forms only support GET/POST)
fn delete_user(req) {
    let db = unwrap(connect("postgres://..."))

    let form = parse_query(req.body)
    let user_id = int(form["id"])  // Convert to int for database!

    execute(db, "DELETE FROM users WHERE id = $1", [user_id])
    close(db)

    return html(r#"<p>User deleted!</p>"#)
}
```
