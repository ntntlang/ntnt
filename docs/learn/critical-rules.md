# NTNT Language — Critical Rules

NTNT (pronounced "Intent") is an agent-native programming language. File extension: `.tnt`

## Building & Running

```bash
ntnt lint file.tnt                  # ALWAYS lint first — catches 90% of errors
ntnt run file.tnt                   # Run (hot-reload in dev)
ntnt test server.tnt --get /health  # Test HTTP endpoints
ntnt intent check file.tnt          # Verify code matches intent spec
```

## Looking Up Functions

Use `ntnt docs` to look up any function — docs are embedded in the binary:

```bash
ntnt docs fetch              # Full docs for fetch()
ntnt docs std/time           # All functions in a module
ntnt docs query_one          # Database functions
ntnt docs set_cookie         # Cookie/session functions
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
req.method          // CORRECT — reading a property
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
import { json, html, text, redirect, status, parse_form, parse_json,
         set_cookie, get_cookie, with_cookie } from "std/http/server"
import { connect, query, query_one, execute } from "std/db/postgres"
import { connect, query, query_one, execute } from "std/db/sqlite"
import { get_env, load_env } from "std/env"
import { split, join, trim, replace, contains } from "std/string"
import { read_file, write_file, exists } from "std/fs"
import { stringify } from "std/json"
import { now, format } from "std/time"
import { sha256, uuid } from "std/crypto"
import { keys, values, entries, has_key } from "std/collections"
import { open, get, set, del } from "std/kv"
import { enqueue, configure_queue } from "std/jobs"
```

## IDD Quick Reference

Write `.intent` files, link code with `@implements`, verify with `ntnt intent check`.

```intent
## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the page loads | status: 200 |
| they see {text} | body contains {text} |

---

Feature: Home Page
  id: feature.home

  Scenario: Shows welcome
    When a user visits the home page
    → the page loads
    → they see "Welcome"
```

```ntnt
// @implements: feature.home
fn home(req) { return html("<h1>Welcome</h1>") }
```

For full IDD docs (glossary patterns, unit testing, property testing, all assertion terms), see the complete reference in `.claude/rules/ntnt.md`.
