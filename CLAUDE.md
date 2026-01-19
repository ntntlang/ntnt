# NTNT Language - Claude Code Instructions

This file provides instructions for Claude when working with NTNT (.tnt) code files.

## Project Overview

NTNT (pronounced "Intent") is an agent-native programming language designed for AI-driven web application development. File extension: `.tnt`

## Building NTNT

```bash
# Standard release build (for distribution)
cargo build --release
cargo install --path . --locked

# Fast dev-release build (for development, ~2x faster)
cargo build --profile dev-release
cargo install --path . --profile dev-release --locked
```

Use `dev-release` when iterating on the compiler. Use `release` for final builds.

## ⚠️ MANDATORY: Always Lint Before Run

**Before running ANY `.tnt` file, run lint first:**

```bash
ntnt lint myfile.tnt    # Check for common mistakes
ntnt run myfile.tnt     # Only after lint passes
```

This prevents wasted debugging time on parser errors.

## 🎯 Intent-Driven Development (IDD) - COLLABORATIVE Workflow

**CRITICAL: IDD is a collaborative process.** The intent file is a shared artifact developed TOGETHER with the user before implementation.

### IDD Workflow

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

### Phase 1: Draft and Present Intent (DO NOT SKIP)

When user asks to build something using IDD:

1. Draft the `.intent` file based on requirements (use correct format below!)
2. **STOP and present it to the user** - do NOT proceed to implementation
3. Ask clarifying questions
4. Wait for user approval before implementing

### Intent File Format (CRITICAL!)

**File must be named to match the .tnt file** (e.g., `server.intent` for `server.tnt`).

```intent
# Project Name
# Run: ntnt intent check server.tnt

## Overview
Brief description of what this project does.

---

Feature: Feature Name
  id: feature.feature_id
  description: "Human-readable description"
  test:
    - request: GET /path
      assert:
        - status: 200
        - body contains "expected text"

Feature: Another Feature
  id: feature.another_feature
  description: "Description"
  test:
    - request: POST /api/data
      body: '{"key": "value"}'
      assert:
        - status: 201

---

Constraint: Constraint Name
  description: "Description of the constraint"
  applies_to: [feature.feature_id, feature.another_feature]
```

**Key rules:**

- Use `Feature:` (capitalized) followed by feature name
- `id:` must be `feature.<snake_case_id>` - used for `@implements`
- `test:` contains HTTP test assertions
- Separate sections with `---`

### File Linking: How `.intent` and `.tnt` Files Connect

Intent files are linked to source files **by filename**:

- `server.tnt` ↔ `server.intent`
- `crypto.tnt` ↔ `crypto.intent`

All `ntnt intent` commands work with either file extension:

```bash
ntnt intent check server.tnt      # Finds server.intent automatically
ntnt intent check server.intent   # Finds server.tnt automatically
```

### File Organization for Larger Apps

**Use a single `.intent` file per application**, even for multi-page apps:

1. **Full context** - Agent sees all features and relationships
2. **Easier reasoning** - No coordination across files
3. **Single source of truth** - One file to review

**Organize with `## Module:` headers:**

```intent
# E-Commerce Platform

## Module: Authentication

Feature: User Login
  id: feature.user_login
  ...

---

## Module: Products

Feature: List Products
  id: feature.list_products
  ...
```

### Phase 2: Implement (After Approval)

```bash
ntnt intent init project.intent -o server.tnt  # Generate scaffolding
```

Add `@implements` annotations:

```ntnt
// @implements: feature.user_login
fn login_handler(req) { ... }

// @utility
fn validate_email(email) { ... }
```

### Phase 3: Verify (MANDATORY)

**ALWAYS run before saying "done":**

```bash
ntnt lint server.tnt           # Check syntax
ntnt intent check server.tnt   # Verify against intent (auto-finds server.intent)
ntnt intent coverage server.tnt # Show coverage
```

## Intent Assertion Language (IAL)

IAL is a term rewriting engine that powers IDD assertions. It translates natural language into executable tests.

### Glossary Definitions

Define domain-specific terms in your `.intent` file:

```yaml
## Glossary

| Term | Means |
|------|-------|
| success response | status 2xx, body contains "ok" |
| they see "$text" | body contains "$text" |
| they don't see "$text" | body not contains "$text" |
| logged in user | component.authenticated_user |
```

Then use natural language in tests:

```yaml
Feature: User Profile
  id: feature.user_profile
  test:
    - request: GET /profile
      given: logged in user
      assert:
        - they see success response
        - they see "Welcome"
```

### Standard Terms (Built-in)

| Pattern | Description |
|---------|-------------|
| `status 200` | Exact status code |
| `status 2xx` | Any success status |
| `body contains "$text"` | Body includes text |
| `body not contains "$text"` | Body excludes text |
| `body matches $pattern` | Regex match |
| `header "$name" contains "$value"` | Header check |
| `redirects to $path` | Redirect check |
| `returns JSON` | Content-Type check |
| `exits successfully` | CLI exit code 0 |
| `file "$path" exists` | File existence |

### Components (Reusable Templates)

```yaml
Component: Authenticated User
  id: component.authenticated_user
  parameters:
    - username: the user's login name
  inherent_behavior:
    - valid session token exists

  preconditions:
    verify: POST /login with valid credentials returns status 200
    skip: "User authentication not available"
```

### Preconditions (Given Clauses)

```yaml
Scenario: View profile
  Given: logged in user
  When: GET /profile
  Then:
    - status 200
```

If the `Given` precondition fails, the test is **SKIPPED** (not failed).

## Critical Syntax Rules

### 1. Map Literals REQUIRE `map` Keyword

```ntnt
// ✅ CORRECT
let data = map { "key": "value" }
let empty = map {}

// ❌ WRONG - {} is a block, not a map
let data = { "key": "value" }
```

### 2. String Interpolation Uses `{expr}` NOT `${expr}`

```ntnt
// ✅ CORRECT
let msg = "Hello, {name}!"

// ❌ WRONG
let msg = `Hello, ${name}!`
let msg = "Hello, ${name}!"
```

### 3. Route Patterns REQUIRE Raw Strings

```ntnt
// ✅ CORRECT
get(r"/users/{id}", handler)

// ❌ WRONG
get("/users/{id}", handler)  // {id} becomes interpolation
```

### 4. String Escapes ARE Supported

```ntnt
// ✅ CORRECT - Escape sequences work
let newline = "line1\nline2"
let quoted = "She said \"hi\""
let path = "C:\\Users\\name"

// Raw strings for complex content
let html = r#"<div class="main">Hello</div>"#
```

### 5. Contracts Go BETWEEN Return Type and Body

```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}
```

### 6. Range Syntax (Not `range()` Function)

```ntnt
for i in 0..10 { }    // exclusive
for i in 0..=10 { }   // inclusive
```

### 7. Import Syntax

```ntnt
import { split, join } from "std/string"
import "std/math" as math
```

### 8. Mutable Variables Need `mut`

```ntnt
let mut counter = 0
counter = counter + 1
```

### 9. Functions Not Methods

```ntnt
len("hello")      // ✅ CORRECT
str(42)           // ✅ CORRECT
push(arr, item)   // ✅ CORRECT

"hello".len()     // ❌ WRONG
```

## HTTP Server - Global Builtins vs Module Exports

**CRITICAL:** HTTP routing functions are GLOBAL BUILTINS. Only response builders need importing.

| Function                    | Type           | Import Needed?              |
| --------------------------- | -------------- | --------------------------- |
| `get(pattern, handler)`     | Global builtin | **No**                      |
| `post(pattern, handler)`    | Global builtin | **No**                      |
| `put(pattern, handler)`     | Global builtin | **No**                      |
| `patch(pattern, handler)`   | Global builtin | **No**                      |
| `delete(pattern, handler)`  | Global builtin | **No**                      |
| `listen(port)`              | Global builtin | **No**                      |
| `serve_static(prefix, dir)` | Global builtin | **No**                      |
| `use_middleware(fn)`        | Global builtin | **No**                      |
| `on_shutdown(fn)`           | Global builtin | **No**                      |
| `routes(dir)`               | Global builtin | **No** (file-based routing) |
| `template(path, vars)`      | Global builtin | **No** (external templates) |
| `json(data, status?)`       | Module export  | **Yes** - `std/http/server` |
| `html(content, status?)`    | Module export  | **Yes** - `std/http/server` |
| `text(content)`             | Module export  | **Yes** - `std/http/server` |
| `redirect(url)`             | Module export  | **Yes** - `std/http/server` |
| `status(code, body)`        | Module export  | **Yes** - `std/http/server` |
| `parse_form(req)`           | Module export  | **Yes** - `std/http/server` |
| `parse_json(req)`           | Module export  | **Yes** - `std/http/server` |

### HTTP Server Example

```ntnt
// ONLY import response builders - routing functions are global!
import { json, html } from "std/http/server"

// Named handler functions (required - no inline lambdas)
fn get_user(req) {
    let user_id = req.params["id"]
    return json(map { "id": user_id })
}

fn home_page(req) {
    return html("<h1>Welcome</h1>")
}

// Routes use global builtins - no import needed
get("/", home_page)
get(r"/users/{id}", get_user)

// Static files and server start
serve_static("/static", "./public")
listen(8080)
```

### ❌ Common HTTP Server Mistakes

```ntnt
// ❌ WRONG - Do NOT import routing functions
import { listen, get, post } from "std/http/server"  // ERROR!
import { listen, get, post } from "std/http_server"  // ERROR! (wrong path)

// ❌ WRONG - No inline lambdas in route handlers
get(r"/users/{id}", |req| { ... })      // Parser error!
get(r"/users/{id}", fn(req) { ... })    // Parser error!

// ✅ CORRECT - Use named functions
fn handler(req) { ... }
get(r"/users/{id}", handler)
```

## Commands

```bash
ntnt run file.tnt              # Run a file
ntnt lint file.tnt             # Validate syntax
ntnt validate file.tnt         # Validate with JSON output
ntnt inspect file.tnt --pretty # Project structure (for agents)
ntnt test server.tnt --get /   # Test HTTP endpoints
ntnt intent check file.tnt     # Verify intent implementation
ntnt intent coverage file.tnt  # Show intent coverage
ntnt intent studio file.intent # Visual preview with live tests (opens :3001)
```

## Standard Library Modules

- `std/string` - split, join, trim, replace, contains, starts_with, ends_with
- `std/url` - encode, decode, parse_query, build_query
- `std/env` - get_env, load_env, set_env, all_env
- `std/collections` - push, pop, keys, values, entries, has_key, get_key, first, last
- `std/http` - fetch, download, Cache (HTTP client)
- `std/http/server` - json, html, text, redirect, status, parse_json, parse_form (response builders + helpers)
- `std/db/postgres` - connect, query, execute, close
- `std/fs` - read_file, write_file, exists, is_file, is_dir, mkdir, readdir
- `std/json` - parse, stringify, stringify_pretty
- `std/csv` - parse, parse_with_headers, stringify, stringify_with_headers
- `std/math` - sin, cos, tan, log, exp, random, random_int, PI, E
- `std/time` - now, format, add_days, add_months
- `std/concurrent` - channel, send, recv, sleep_ms
- `std/path` - join, dirname, basename, extname

## External Templates

Use `template()` to load external HTML files with variable substitution. Templates use Mustache-style syntax.

### Basic Usage

```ntnt
// template(path, variables) - path is relative to the .tnt file
let page = template("views/home.html", map {
    "title": "Welcome",
    "username": "Alice"
})
return html(page)
```

**views/home.html:**
```html
<!DOCTYPE html>
<html>
<head><title>{{title}}</title></head>
<body>
    <h1>Hello, {{username}}!</h1>
</body>
</html>
```

### Template Syntax

```html
<!-- Variables -->
{{title}}
{{user.name}}

<!-- Loops -->
{{#for item in items}}
    <li>{{item.name}} - {{item.price}}</li>
{{/for}}

<!-- Conditionals -->
{{#if logged_in}}
    <p>Welcome back!</p>
{{#else}}
    <p>Please log in</p>
{{/if}}

<!-- Nested templates (partials) -->
{{> partials/header.html}}
```

### Recommended Project Structure

```
my-app/
├── server.tnt
├── views/
│   ├── layout.html
│   ├── home.html
│   ├── styles.css
│   └── partials/
│       ├── header.html
│       └── footer.html
└── public/
    └── (static assets)
```

### Complete Example

```ntnt
import { html } from "std/http/server"

fn render_page(content, title) {
    let styles = template("views/styles.css", map {})
    let header = template("views/partials/header.html", map {})
    let footer = template("views/partials/footer.html", map {})

    return template("views/layout.html", map {
        "title": title,
        "styles": styles,
        "header": header,
        "footer": footer,
        "content": content
    })
}

fn home(req) {
    let content = template("views/home.html", map {
        "featured_items": get_featured_items()
    })
    return html(render_page(content, "Home"))
}

get("/", home)
listen(8080)
```

**Note:** Template paths resolve relative to the `.tnt` file location, not the current working directory. This means your app works the same whether you run `ntnt run app.tnt` from the project folder or from anywhere else.

## File-Based Routing

Use `routes()` to auto-discover route handlers from a directory structure (like Next.js/SvelteKit):

```ntnt
import { html } from "std/http/server"

// Load all routes from routes/ directory
routes("routes")

// Middleware auto-loads from middleware/ directory
listen(8080)
```

### Directory Structure

```
my-app/
├── server.tnt
├── routes/
│   ├── index.tnt          # GET /
│   ├── about.tnt          # GET /about
│   ├── api/
│   │   ├── users.tnt      # GET/POST /api/users
│   │   └── [id].tnt       # GET /api/users/:id (dynamic segment)
│   └── blog/
│       ├── index.tnt      # GET /blog
│       └── [slug].tnt     # GET /blog/:slug
├── middleware/
│   └── logging.tnt        # Auto-applied to all routes
└── views/
    └── ...
```

### Route File Format

Each route file exports handler functions named after HTTP methods:

**routes/api/users.tnt:**
```ntnt
import { json } from "std/http/server"

// GET /api/users
fn get(req) {
    return json(map { "users": get_all_users() })
}

// POST /api/users
fn post(req) {
    let form = parse_form(req)
    return json(map { "created": true })
}
```

**routes/api/[id].tnt (dynamic segment):**
```ntnt
import { json } from "std/http/server"

// GET /api/users/:id
fn get(req) {
    let id = req.params["id"]
    return json(map { "id": id })
}
```

### Middleware

**middleware/logging.tnt:**
```ntnt
fn middleware(req) {
    print("Request: {req.method} {req.path}")
    // Return nothing to continue, or return a response to short-circuit
}
```

## HTTP Form Handling

```ntnt
import { parse_form, parse_json } from "std/http/server"

fn handle_form(req) {
    let form = parse_form(req)  // Parse URL-encoded form data
    let name = form["name"]
    let age = int(form["age"])  // Convert to int for database!
}

fn handle_json(req) {
    match parse_json(req) {     // Parse JSON body
        Ok(data) => {
            let name = data["name"]
            // ...
        },
        Err(e) => print("Parse error: " + e)
    }
}
```

### Request Properties

The request object includes these helpful fields:

- `req.method` - HTTP method (GET, POST, etc.)
- `req.path` - URL path
- `req.query_params` - Parsed query string as map
- `req.params` - Route parameters (from `/users/{id}`)
- `req.headers` - Request headers map
- `req.body` - Raw request body
- `req.ip` - Client IP (from X-Forwarded-For if behind proxy)
- `req.id` - Request ID (from X-Request-ID or auto-generated)

### Server Lifecycle

```ntnt
on_shutdown(fn() {
    print("Server shutting down...")
    // Close database connections, flush logs, etc.
})
```

## Full Reference

See [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) for complete syntax reference, all stdlib functions, and code patterns.
