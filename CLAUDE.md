# NTNT Language - Claude Code Instructions

This file provides instructions for Claude when working with NTNT (.tnt) code files.

## Project Overview

NTNT (pronounced "Intent") is an agent-native programming language designed for AI-driven web application development. File extension: `.tnt`

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
```

## Standard Library Modules

- `std/string` - split, join, trim, replace, contains
- `std/url` - encode, decode, parse_query, build_query
- `std/env` - get_env, load_env, args, cwd
- `std/collections` - push, pop, keys, values, has_key, get_key
- `std/http` - fetch, post, put, delete, get_json
- `std/http/server` - json, html, text, redirect, status (response builders ONLY)
- `std/db/postgres` - connect, query, execute, close
- `std/fs` - read_file, write_file, exists, mkdir
- `std/json` - parse, stringify
- `std/time` - now, format, add_days
- `std/concurrent` - channel, send, recv, sleep_ms

## HTTP Form Handling

```ntnt
import { parse_query } from "std/url"

fn post(req) {
    let form = parse_query(req.body)
    let name = form["name"]
    let age = int(form["age"])  // Convert to int for database!
}
```

## Full Reference

See [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) for complete syntax reference, all stdlib functions, and code patterns.
