# NTNT Language - GitHub Copilot Instructions

NTNT (pronounced "Intent") is an agent-native programming language for AI-driven web development. File extension: `.tnt`

## Documentation References

Read these files for full language details. Consult the references when writing non-trivial NTNT code.

| Document | Contents |
|----------|----------|
| [AI_AGENT_GUIDE.md](../docs/AI_AGENT_GUIDE.md) | **Canonical NTNT coding guide** — full syntax, patterns, all stdlib APIs, HTTP server, DB, auth, concurrency, jobs, templates, error handling |
| [STDLIB_REFERENCE.md](../docs/STDLIB_REFERENCE.md) | All stdlib functions (auto-generated from source) |
| [SYNTAX_REFERENCE.md](../docs/SYNTAX_REFERENCE.md) | Keywords, operators, types, templates |
| [IAL_REFERENCE.md](../docs/IAL_REFERENCE.md) | Intent Assertion Language |

## Mandatory Workflow

```bash
ntnt lint file.tnt                  # ALWAYS lint first — catches 90% of errors
ntnt run file.tnt                   # Only after lint passes
ntnt test server.tnt --get /health  # Test HTTP endpoints
ntnt intent check file.tnt          # Verify code matches intent specs
```

## Type Safety Modes

```bash
# Runtime (NTNT_TYPE_MODE): strict | warn (default) | forgiving
# Lint (NTNT_LINT_MODE): default | warn | strict
NTNT_TYPE_MODE=strict NTNT_LINT_MODE=strict ntnt run server.tnt  # Production
```

---

## Critical Syntax Rules

These are the most common mistakes. Memorize them.

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

### 3. Free functions are canonical — dot-call is UFCS sugar

`x.f(a)` resolves to `f(x, a)` for any builtin, imported, or user-defined function. Both styles work; prefer free functions.

```ntnt
len(s)              // CANONICAL — free function transforms data
s.len()             // ALSO WORKS — UFCS sugar for len(s)
5.double()          // works for user-defined fn double(x) too
req.method          // dot WITHOUT parens reads a property
req.params.id       // reading a map key
m.keys              // reads the map key "keys" (None if missing)
m.keys()            // parens ALWAYS mean a call: keys(m), even if a "keys" key exists
```

### 4. Route functions are GLOBAL builtins — never import them

```ntnt
get("/users/{id}", handler)     // CORRECT — auto-detects {param}
listen(8080)                    // CORRECT — global builtin
import { get, listen } from "std/http/server"  // WRONG
```

Only import response builders: `json`, `html`, `text`, `redirect`, `status`, `parse_form`, `parse_json` from `"std/http/server"`.

### 5. No semicolons — use newlines

Semicolons parse fine but are unnecessary; `ntnt lint` warns (`unnecessary_semicolon`). Omit them.

### 6. `otherwise` blocks MUST diverge

```ntnt
let data = parse_json(req) otherwise { return status(400, "Bad JSON: #{err}") }  // CORRECT
let data = parse_json(req) otherwise { status(400, "Bad JSON") }                 // WRONG — missing return
```

### 7. Ranges: `0..10` — `range()` doesn't exist

### 8. Declare `mut` for anything you reassign: `let mut counter = 0`

`mut` is enforced for indexed mutation (`arr[0] = x`, `m["k"] = v` error without it). Plain rebinding (`x = x + 1`) currently succeeds without `mut` — still declare `mut` for clarity and forward compatibility.

### 9. Closures: `fn(x) { x * 2 }` — pipe-style `|x| x * 2` doesn't exist

### 10. Closures stored in map values are not dot-callable

`m.f(x)` fails with E007 when `f` is a closure stored in map `m` (parens make it a UFCS call to a function named `f`). Bind first: `let f = m.f` then `f(x)`.

### 11. `for..in` on strings skips with a runtime warning — use `chars(s)` from `std/string`

### 12. Template strings: `"""..{{expr}}.."""` — double braces, not single

### 13. Contracts go AFTER return type, BEFORE body

```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{ return a / b }
```

### 14. `0` is truthy — unlike JS/Python. Falsy: `false`, `""`, `None`, `[]`, `map {}`

### 15. Map access returns `None` for missing keys — use `has_key()` to check existence, not `is_some()`

### 16. `for k in map` iterates keys — use `entries()` for key-value pairs

---

## Error Handling Quick Reference

| Pattern | Use When |
|---------|----------|
| `val?` | Propagate `Err`/`None` to caller (unwraps `Ok`/`Some`) |
| `val ?? default` | Provide default for `None` |
| `val otherwise { return ... }` | Handle error at call site with custom recovery (block must diverge) |
| `match val { Ok(v) => ..., Err(e) => ... }` | Complex branching |
| `unwrap(val)` | Quick prototyping (panics on error) |

---

## IDD (Intent-Driven Development)

1. **Draft** `.intent` file from requirements
2. **Present** to user for approval — do NOT implement before approval
3. **Implement** with `// @implements: feature.id` annotations
4. **Verify** with `ntnt intent check` or `ntnt intent studio`

---

## Common Imports

```ntnt
import { split, join, trim, replace, contains, chars } from "std/string"
import { json, html, text, redirect, status, parse_form, parse_json } from "std/http/server"
import { connect, query, query_one, execute, close } from "std/db/postgres"
import { connect, query, query_one, execute, close } from "std/db/sqlite"
import { fetch } from "std/http"
import { read_file, write_file, exists } from "std/fs"
import { stringify } from "std/json"
import { get_env, load_env } from "std/env"
import { now, format } from "std/time"
import { sha256, uuid } from "std/crypto"
import { first, last, keys, values, entries, has_key, get_key } from "std/collections"
import { oauth, enable_auth, get_user, validate_csrf } from "std/auth"
import { open, get, set, del, list } from "std/kv"
import { log_info, log_warn, log_error, set_log_level } from "std/log"
import { channel, send, recv, sleep_ms, spawn, await_task, parallel, race } from "std/concurrent"
import { enqueue, enqueue_in, configure_queue, work_async, work_jobs } from "std/jobs"
```

---

## CLI Commands

```bash
ntnt run <file>              # Run (hot-reload in dev)
ntnt lint <file>             # Check for errors
ntnt lint --strict <file>    # Strict type warnings
ntnt test <file> --get /     # Test HTTP endpoints
ntnt intent check <file>     # Verify intent specs
ntnt intent studio <intent>  # Visual studio
ntnt docs [query]            # Search stdlib docs
ntnt docs --generate         # Regenerate reference docs from source
ntnt worker <file>           # Run background job workers
ntnt jobs status <file>      # Job queue status
```
