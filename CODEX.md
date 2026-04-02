# NTNT Language - Codex Instructions

NTNT (pronounced "Intent") is an agent-native programming language for AI-driven web development. File extension: `.tnt`

## OpenClaw Skill (Core Development)

For core NTNT language/runtime development (Rust compiler work, stdlib functions, interpreter changes), load the full OpenClaw skill for deep context:

```
~/.openclaw/skills/ntnt/SKILL.md
```

This guide covers compiler internals, adding stdlib functions, modifying the interpreter, or any Rust-level changes to the NTNT runtime.

## Building NTNT

```bash
cargo build --profile dev-release   # Fast dev build
cargo build --release               # Distribution build
```

## Mandatory Workflow

```bash
ntnt lint file.tnt                  # ALWAYS lint first — catches 90% of errors
ntnt run file.tnt                   # Only after lint passes
ntnt test server.tnt --get /health  # Test HTTP endpoints
ntnt intent check file.tnt          # Verify code matches intent specs
```

## Documentation References

Read these files for full language details. Do NOT rely solely on this CLAUDE.md for NTNT syntax — consult the references when writing non-trivial NTNT code.

| Document | Contents |
|----------|----------|
| [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) | **Canonical NTNT coding guide** — full syntax, patterns, all stdlib APIs, HTTP server, DB, auth, concurrency, jobs, templates, error handling |
| [docs/STDLIB_REFERENCE.md](docs/STDLIB_REFERENCE.md) | All stdlib functions (auto-generated from source) |
| [docs/SYNTAX_REFERENCE.md](docs/SYNTAX_REFERENCE.md) | Keywords, operators, types, templates |
| [docs/IAL_REFERENCE.md](docs/IAL_REFERENCE.md) | Intent Assertion Language |

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

### 3. Free functions, not methods — dot reads properties

```ntnt
len(s)              // CORRECT — free function transforms data
s.len()             // WRONG — dot is for reading properties only
trim(input)         // CORRECT
input.trim()        // WRONG
req.method          // CORRECT — reading a property
req.params.id       // CORRECT — reading a map key
```

### 4. Route functions are GLOBAL builtins — never import them

```ntnt
get("/users/{id}", handler)     // CORRECT — auto-detects {param}
listen(8080)                    // CORRECT — global builtin
import { get, listen } from "std/http/server"  // WRONG
```

Only import response builders: `json`, `html`, `text`, `redirect`, `status`, `parse_form`, `parse_json` from `"std/http/server"`.

### 5. No semicolons — use newlines

`;` silently corrupts parser state. Never use semicolons.

### 6. `otherwise` blocks MUST diverge

```ntnt
let data = parse_json(req) otherwise { return status(400, "Bad JSON: #{err}") }  // CORRECT
let data = parse_json(req) otherwise { status(400, "Bad JSON") }                 // WRONG — missing return
```

### 7. Ranges: `0..10` — `range()` doesn't exist

### 8. Mutable variables need `mut`: `let mut counter = 0`

### 9. Closures: `fn(x) { x * 2 }` — pipe-style `|x| x * 2` doesn't exist

### 10. Module-level `let` can't use `map {}` — move maps inside functions

### 11. `for..in` on strings does nothing — use `chars(s)` from `std/string`

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

```bash
ntnt intent check server.tnt       # Verify code matches intent
ntnt intent studio server.intent   # Visual studio with live tests
ntnt intent coverage server.tnt    # Feature coverage report
ntnt intent init server.intent     # Generate scaffolding from intent
```

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

---

## Editing the NTNT Language (Rust Development)

For compiler/runtime Rust work, use the **`/ntnt-core-dev`** skill (`.claude/skills/ntnt-core-dev.md`). It covers: source structure, compiler pipeline, HTTP server architecture, writing correct Rust code, adding stdlib functions, doc system (`// @ntnt`), pre-push checklist, CI/CD workflows, and testing patterns.

---

## Documentation Maintenance (MANDATORY)

**After implementing any language feature, update these:**

1. `// @ntnt` doc blocks on all new/changed functions (build enforces this)
2. `docs/AI_AGENT_GUIDE.md` for user-facing syntax or patterns
3. `ntnt docs --generate` to regenerate `STDLIB_REFERENCE.md` and sync agent files (`.github/copilot-instructions.md`)
4. `LANGUAGE_GUIDE.md` for detailed explanations
5. `ARCHITECTURE.md` for structural changes
6. `ROADMAP.md` to update feature status
7. Add integration tests and example files

Do not wait for the user to ask — update docs as part of every implementation task.

## Greptile Review Process

Greptile auto-reviews every PR on this repo. AI agents must self-service Greptile feedback before requesting human review.

### Comment Triage

| Bucket | Type | Action |
|--------|------|--------|
| 1 | **Correctness** — bugs, race conditions, dead code, wrong logic | Auto-fix. Commit + reply with hash. |
| 2 | **Hardening** — missing validation, undocumented behavior, incomplete tests, error messages | Auto-fix. Clear right answer exists. |
| 3 | **Architecture** — data structure changes, API redesign, flow restructuring, design philosophy | **Do not fix.** Escalate to maintainer with the suggestion and your assessment. |

### Workflow

1. Push PR → Greptile reviews (~3 min)
2. Read comments, triage into buckets
3. Fix all Bucket 1/2 items → commit → push → Greptile re-reviews
4. Loop until only Bucket 3 items remain (or clean)
5. Post summary to maintainer: Fixed (list), Need your call (Bucket 3 items)
