# NTNT Language — Critical Rules

NTNT (pronounced "Intent") is an agent-native programming language. File extension: `.tnt`

## Building & Running

```bash
ntnt lint file.tnt                  # ALWAYS lint first — catches 90% of errors
ntnt run file.tnt                   # Run (hot-reload in dev)
ntnt test server.tnt --get /health  # Test HTTP endpoints
```

## Looking Up Functions

Use `ntnt docs` to look up any function — docs are embedded in the binary:

```bash
ntnt docs fetch              # Full docs for fetch()
ntnt docs std/time           # All functions in a module
ntnt docs std/crypto         # Crypto functions: sha256, bcrypt, AES, etc.
ntnt docs hash_password      # Search by function name
ntnt docs fetch --json       # JSON output for structured access
```

**When unsure about a function's signature or behavior, run `ntnt docs <name>` — it's faster and more accurate than guessing.**

## Critical Syntax Rules

### 1. Maps require `map` keyword — bare `{}` is a code block

```ntnt
let user = map { "name": "Alice", "age": 30 }   // CORRECT
let user = { "name": "Alice" }                   // WRONG — {} is a block
```

### 2. String interpolation: `#{expr}` — not `${expr}`

```ntnt
let msg = "Hello, #{name}!"    // CORRECT
let msg = "Hello, ${name}!"    // WRONG
```

### 3. Free functions, not methods — dot reads properties

```ntnt
len(s)              // CORRECT — free function
s.len()             // WRONG — dot is for reading properties only
trim(input)         // CORRECT
input.trim()        // WRONG
req.method          // CORRECT — reading a property
req.params.id       // CORRECT — reading a map key
```

### 4. Route functions are GLOBAL builtins — never import them

```ntnt
get("/users/{id}", handler)     // CORRECT — global builtin
listen(8080)                    // CORRECT — global builtin
import { get, listen } from "std/http/server"  // WRONG
```

Only import response builders: `json`, `html`, `text`, `redirect`, `status`, `parse_form`, `parse_json` from `"std/http/server"`.

### 5. No semicolons — use newlines

### 6. `otherwise` blocks MUST diverge (use `return`)

```ntnt
let data = parse_json(req) otherwise { return status(400, "Bad JSON: #{err}") }  // CORRECT
let data = parse_json(req) otherwise { status(400, "Bad JSON") }                 // WRONG — missing return
```

### 7. Ranges: `0..10` — `range()` doesn't exist

### 8. Mutable variables: `let mut counter = 0`

### 9. Closures: `fn(x) { x * 2 }` — pipe-style `|x| x * 2` doesn't exist

### 10. `0` is truthy — Falsy: `false`, `""`, `None`, `[]`, `map {}`

### 11. Template strings: `"""..{{expr}}.."""` — double braces for interpolation

### 12. Contracts go AFTER return type, BEFORE body

```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{ return a / b }
```

## Error Handling

| Pattern | Use When |
|---------|----------|
| `val?` | Propagate `Err`/`None` to caller |
| `val ?? default` | Provide default for `None` |
| `val otherwise { return ... }` | Handle error at call site (block must diverge) |
| `match val { Ok(v) => ..., Err(e) => ... }` | Complex branching |

## Common Imports

```ntnt
import { json, html, text, redirect, status, parse_form, parse_json } from "std/http/server"
import { fetch } from "std/http"
import { split, join, trim, replace, contains, chars } from "std/string"
import { connect, query, query_one, execute, close } from "std/db/postgres"
import { connect, query, query_one, execute, close } from "std/db/sqlite"
import { read_file, write_file, exists } from "std/fs"
import { stringify, parse_json } from "std/json"
import { get_env, load_env } from "std/env"
import { now, format } from "std/time"
import { sha256, uuid } from "std/crypto"
import { first, last, keys, values, entries, has_key, get_key } from "std/collections"
import { open, get, set, del, list } from "std/kv"
import { log_info, log_warn, log_error } from "std/log"
import { enqueue, configure_queue, work_async, batch, seal, enqueue_into, batch_id } from "std/jobs"
```

---

## IDD (Intent-Driven Development)

IDD is the core workflow for ntnt. Write requirements as `.intent` files, implement with annotations, verify automatically.

### Workflow

1. **Draft** a `.intent` file from requirements
2. **Present** to user for approval — do NOT implement before approval
3. **Implement** with `@implements: feature.id` annotations
4. **Verify** with `ntnt intent check`

### .intent File Format

```intent
# My App

## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the page loads | status: 200 |
| they see {text} | body contains {text} |
| they don't see {text} | body not contains {text} |

---

Feature: Home Page
  id: feature.home
  description: "Welcome page for visitors"

  Scenario: Shows welcome message
    When a user visits the home page
    → the page loads
    → they see "Welcome"

  Scenario: Has correct content type
    When a user visits the home page
    → content-type is html

---

Constraint: Security Headers
  description: "All pages include security headers"
  applies_to: [feature.home]
```

### File Linking

Intent files link to source files by name: `server.tnt ↔ server.intent`

### Code Annotations

```ntnt
// @implements: feature.home
fn home_handler(req) { return html("<h1>Welcome</h1>") }

// @supports: constraint.security
fn add_headers(req) { ... }

// @utility — helper, not a feature
fn hash_password(pw) { ... }
```

| Annotation | Purpose |
|------------|---------|
| `@implements: feature.X` | Links function to a feature |
| `@supports: constraint.X` | Links function to a constraint |
| `@utility` | Marks helper functions |
| `@infrastructure` | Config/setup code |
| `@internal` | Internal implementation |

---

### Function Unit Testing (call: syntax)

Test individual functions without an HTTP server. Add `call:` and `source:` keywords in the glossary:

```intent
## Glossary

| Term | Means |
|------|-------|
| slugifying {text} | call: to_slug({text}), source: utils.tnt |
| validating email {email} | call: is_valid_email({email}), source: validators.tnt |
| hashing {input} | call: hash_password({input}), source: auth.tnt |

---

Feature: URL Slugs
  id: feature.slugs

  Scenario: Basic slug conversion
    When slugifying "Hello World"
    → result is "hello-world"

  Scenario: Special characters removed
    When slugifying "café & résumé!"
    → result is "caf-rsum"
    → is lowercase
    → does not contain " "

  Scenario: Leading/trailing hyphens stripped
    When slugifying "  Hello  "
    → does not start with "-"
    → does not end with "-"
    → is non-empty

---

Feature: Email Validation
  id: feature.email

  Scenario: Valid email accepted
    When validating email "user@example.com"
    → result is true

  Scenario: Invalid email rejected
    When validating email "not-an-email"
    → result is false
```

**Required keywords in glossary:**
- `call:` — function to invoke with `{param}` placeholders
- `source:` — `.tnt` file containing the function (required)

### Property Testing

Verify function properties without specific input/output pairs:

```intent
## Glossary

| Term | Means |
|------|-------|
| slugifying {text} | call: to_slug({text}), source: utils.tnt |

---

Feature: Slug Properties
  id: feature.slug_properties

  Scenario: Slug is deterministic
    When slugifying "Hello World"
    → is deterministic

  Scenario: Slug is idempotent
    When slugifying "hello-world"
    → is idempotent
```

- `is deterministic` — calling with same input always produces same output
- `is idempotent` — applying the function twice gives same result as once: `f(f(x)) == f(x)`

---

### Built-in Assertion Terms

These work in any `→` line without needing a glossary entry:

**HTTP Response:**

| Term | What it checks |
|------|---------------|
| `status: {code}` | Response status equals code |
| `status 2xx` | Status in 200-299 range |
| `status 4xx` | Status in 400-499 range |
| `status 5xx` | Status in 500-599 range |
| `body contains {text}` | Response body contains text |
| `body not contains {text}` | Response body does not contain text |
| `body matches {pattern}` | Body matches regex pattern |
| `body is empty` | Body is empty string |
| `body is not empty` | Body is not empty |
| `body has field {field}` | Body contains the field name as text |
| `response is valid JSON` | Body parses as JSON |
| `header {name} exists` | Response header exists |
| `header {name} equals {value}` | Header has exact value |
| `header {name} contains {value}` | Header contains value |
| `content-type is json` | Content-Type is application/json |
| `content-type is html` | Content-Type is text/html |
| `content-type is text` | Content-Type is text/plain |
| `response time < {ms}ms` | Response faster than N milliseconds |
| `response time < {seconds}s` | Response faster than N seconds |

**Function Result (unit tests):**

| Term | What it checks |
|------|---------------|
| `result is {expected}` | Function returned expected value |
| `result equals {expected}` | Same as `result is` |
| `is deterministic` | Same input → same output |
| `is idempotent` | f(f(x)) == f(x) |
| `is lowercase` | Result is all lowercase |
| `is non-empty` | Result is not empty string |
| `starts with {prefix}` | Result starts with prefix |
| `ends with {suffix}` | Result ends with suffix |
| `does not contain {text}` | Result doesn't contain text |
| `does not start with {prefix}` | Result doesn't start with prefix |
| `does not end with {suffix}` | Result doesn't end with suffix |
| `uses only {pattern}` | Result matches character pattern regex |
| `is at least {min}` | Numeric result >= min |
| `is at most {max}` | Numeric result <= max |
| `length is at most {max}` | String/array length <= max |

**Code Quality:**

| Term | What it checks |
|------|---------------|
| `code passes lint` | All .tnt files pass lint + validate |
| `code quality passes` | Same as above |
| `code is valid` | Code quality check passed |
| `no syntax errors` | Zero lint errors |
| `no lint errors` | Same as above |
| `no lint warnings` | Zero lint warnings |

**CLI:**

| Term | What it checks |
|------|---------------|
| `exit code is {code}` | CLI command exit code |
| `stdout contains {text}` | CLI stdout contains text |
| `stderr contains {text}` | CLI stderr contains text |
| `stderr is empty` | CLI stderr is empty |

---

### Glossary Patterns

The `{param}` syntax captures values from scenario text:

```intent
| a user visits {path} | GET {path} |
```

When a scenario says `When a user visits /about`, the glossary captures `path="/about"` and expands to `GET /about`.

**Two formats supported:**

2-column: `| Term | Means |`
3-column: `| Term | Type | Means |` (Type is metadata, e.g., "action", "check")

**Glossary list format** (alternative to table):

```intent
## Glossary

valid user:
  - body contains "id"
  - body contains "name"

authenticated response:
  - status: 200
  - header authorization exists
```

---

### Commands

```bash
ntnt intent check file.tnt       # Run tests against implementation
ntnt intent studio file.intent   # Visual preview with live tests
ntnt intent coverage file.tnt    # Feature coverage report
ntnt intent init file.intent     # Generate code scaffolding from intent
```

### Complete Example: App with HTTP + Unit Tests

```intent
# Task Manager

## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| a user posts to {path} with {body} | POST {path} body {body} |
| the page loads | status: 200 |
| they see {text} | body contains {text} |
| slugifying {text} | call: to_slug({text}), source: utils.tnt |

---

Feature: Task List
  id: feature.tasks
  description: "Display and manage tasks"

  Scenario: View task list
    When a user visits /tasks
    → the page loads
    → they see "Tasks"
    → content-type is html

  Scenario: Create a task
    When a user posts to /tasks with {"title": "Buy milk"}
    → status: 201
    → body contains "Buy milk"

---

Feature: URL Utilities
  id: feature.utils

  Scenario: Slug generation
    When slugifying "Buy Milk Today!"
    → result is "buy-milk-today"
    → is lowercase
    → does not contain "!"

  Scenario: Slug is deterministic
    When slugifying "Hello World"
    → is deterministic

---

Constraint: Code Quality
  description: "All code passes lint"
  applies_to: [feature.tasks, feature.utils]

  Scenario: No errors
    → code passes lint
    → no lint warnings
```
