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

## Critical Syntax Rules (Common Mistakes)

### 1. Map Literals Require `map` Keyword

```ntnt
// ✅ CORRECT
let user = map { "name": "Alice", "age": 30 }
let empty = map {}

// Nested maps are inferred automatically
let config = map {
    "server": { "host": "localhost", "port": 8080 }
}

// ❌ WRONG - {} is a block, not a map
let user = { "name": "Alice" }
```

### 2. String Interpolation Uses `{expr}` NOT `${expr}`

```ntnt
// ✅ CORRECT
let msg = "Hello, {name}!"

// ❌ WRONG
let msg = "Hello, ${name}!"
let msg = `Hello, ${name}!`
```

### 3. Route Patterns Require Raw Strings

```ntnt
// ✅ CORRECT - Raw strings for route patterns
get(r"/users/{id}", handler)

// ❌ WRONG - {id} becomes interpolation
get("/users/{id}", handler)
```

### 4. Contracts Go AFTER Return Type, BEFORE Body

```ntnt
// ✅ CORRECT
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}

// ❌ WRONG - contracts in wrong position
fn divide(a: Int, b: Int) -> Int {
    requires b != 0  // Inside body - wrong!
}
```

### 5. Range Syntax (Not Function)

```ntnt
// ✅ CORRECT
for i in 0..10 { }     // 0-9 exclusive
for i in 0..=10 { }    // 0-10 inclusive

// ❌ WRONG
for i in range(10) { }  // range() doesn't exist
```

### 6. Functions Not Methods

```ntnt
// ✅ CORRECT - Standalone functions
len("hello")
str(42)
push(arr, item)
split(text, ",")

// ❌ WRONG - Method style
"hello".len()
arr.push(item)
```

### 7. Mutable Variables Need `mut`

```ntnt
// ✅ CORRECT
let mut counter = 0
counter = counter + 1

// ❌ WRONG
let counter = 0
counter = 1  // ERROR: immutable
```

---

## HTTP Server Pattern

**CRITICAL:** Routing functions are GLOBAL BUILTINS. Only response builders need importing.

```ntnt
// ONLY import response builders
import { json, html, parse_form } from "std/http/server"

// Handler function (named - no inline lambdas)
fn get_user(req) {
    let id = req.params["id"]
    return json(map { "id": id })
}

// Routes - global builtins, raw strings for patterns
get(r"/users/{id}", get_user)
post("/users", create_user)

serve_static("/static", "./public")
listen(8080)
```

**Request object properties:**
```ntnt
req.method        // "GET", "POST"
req.path          // "/users/123"
req.params        // Route params: req.params["id"]
req.query_params  // Query string: req.query_params["name"]
req.headers       // Headers map
req.body          // Raw body string
```

**Common mistakes:**
```ntnt
// ❌ WRONG - Do NOT import routing functions
import { listen, get, post } from "std/http/server"

// ❌ WRONG - No inline lambdas
get(r"/users/{id}", |req| { ... })

// ❌ WRONG - These don't exist
req.json       // Use parse_json(req)
req.form       // Use parse_form(req)
req.params.id  // Use req.params["id"]
```

---

## Database Pattern

```ntnt
import { connect, query, execute, close } from "std/db/postgres"

let db = unwrap(connect("postgres://..."))

// Query returns array of maps
let users = unwrap(query(db, "SELECT * FROM users WHERE active = $1", [true]))
for user in users {
    print("Name: {user[\"name\"]}")
}

// Execute for INSERT/UPDATE/DELETE
execute(db, "INSERT INTO users (name, age) VALUES ($1, $2)", [name, int(age_str)])

close(db)
```

**Type conversion for database:**
```ntnt
let form = parse_form(req)
let age = int(form["age"])     // Convert string to int!
let price = float(form["price"])

// ❌ WRONG - String to integer column causes "db error"
execute(db, "INSERT INTO users (age) VALUES ($1)", [form["age"]])
```

---

## Intent-Driven Development (IDD)

IDD is a **collaborative process**. The intent file is developed together with the user before implementation.

### Workflow

| Step | Action | User Input |
|------|--------|------------|
| 1 | Draft `.intent` file | No |
| 2 | **Present to user** | **YES - STOP** |
| 3 | Refine based on feedback | Yes |
| 4 | User approves | **YES** |
| 5 | `ntnt intent init` | No |
| 6 | Implement with `@implements` | No |
| 7 | `ntnt intent check` | No |

### Intent File Format

```intent
# Project Name
# Run: ntnt intent check server.tnt

## Overview
Brief description.

---

Feature: User Login
  id: feature.user_login
  description: "Users can log in"
  test:
    - request: POST /login
      body: '{"email": "test@test.com", "password": "secret"}'
      assert:
        - status: 200
        - body contains "token"

---

Constraint: Rate Limiting
  description: "Prevent brute force"
  applies_to: [feature.user_login]
```

### Code Annotations

```ntnt
// @implements: feature.user_login
fn login_handler(req) { ... }

// @utility
fn hash_password(password) { ... }
```

### Commands

```bash
ntnt intent studio server.intent  # Visual preview + live tests
ntnt intent check server.tnt      # Run tests
ntnt intent coverage server.tnt   # Show coverage
ntnt intent init server.intent    # Generate scaffolding
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

See [SYNTAX_REFERENCE.md](SYNTAX_REFERENCE.md#template-strings) for filters and metadata variables.

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
│   └── [id].tnt       # GET /api/:id
```

Route files export `get`, `post`, etc. functions.

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
| `get/post/put/delete(pattern, handler)` | HTTP routes |
| `listen(port)` | Start server |
| `serve_static(prefix, dir)` | Static files |
| `template(path, vars)` | Load template |

### Common Imports

```ntnt
import { split, join, trim, replace } from "std/string"
import { json, html, parse_form, parse_json } from "std/http/server"
import { connect, query, execute, close } from "std/db/postgres"
import { fetch, download } from "std/http"
import { read_file, write_file, exists } from "std/fs"
import { parse, stringify } from "std/json"
import { get_env } from "std/env"
```

See [STDLIB_REFERENCE.md](STDLIB_REFERENCE.md) for complete function documentation.
