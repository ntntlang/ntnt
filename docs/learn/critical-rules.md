# ntnt — Critical Syntax Rules
# Regenerate: ntnt learn <platform>
# Full reference: ntnt docs or docs/AI_AGENT_GUIDE.md

## Workflow (mandatory)

```bash
ntnt lint file.tnt        # ALWAYS lint first — catches 90% of errors
ntnt run file.tnt         # Only run after lint passes
ntnt test server.tnt --get /health  # Test HTTP endpoints
```

## Rules that break every time

1. **Map literals require `map` keyword** — bare `{}` is a code block
   ```ntnt
   let user = map { "name": "Alice", "age": 30 }  # CORRECT
   let user = { "name": "Alice" }                  # WRONG — block, not map
   ```

2. **String interpolation: `#{expr}` — hash + braces**
   ```ntnt
   let msg = "Hello, #{name}!"   # CORRECT
   let msg = "Hello, ${name}!"   # WRONG — no dollar sign
   let msg = "Hello, {name}!"    # WRONG — bare braces don't interpolate
   ```
   Escape with `\#` to prevent interpolation: `"Price: \#100"` prints literally.

3. **No semicolons** — use newlines. `;` silently corrupts the parser.

4. **Mutable variables need `mut`**
   ```ntnt
   let mut counter = 0
   counter = counter + 1
   ```

5. **Free functions, not methods** — `len(s)` not `s.len()`, `trim(s)` not `s.trim()`

6. **Dot notation reads properties** — `req.params.id`, `req.headers["content-type"]`
   Brackets required for dynamic keys or keys with special characters.

7. **Route params auto-detect `{param}`** — no raw strings needed
   ```ntnt
   get("/users/{id}", handler)
   ```

8. **HTTP routing functions are GLOBAL builtins** — never import get/post/listen/serve_static
   ```ntnt
   # WRONG — do NOT import routing functions
   import { listen, get, post } from "std/http/server"
   # Only import response builders
   import { json, html, parse_form, parse_json } from "std/http/server"
   ```

9. **Contracts go AFTER return type, BEFORE body**
   ```ntnt
   fn divide(a: Int, b: Int) -> Int
       requires b != 0
       ensures result * b == a
   { return a / b }
   ```

10. **`otherwise` blocks MUST diverge** — use `return`, `break`, or `continue`
    ```ntnt
    let data = parse_json(req) otherwise { return status(400, "Bad JSON: #{err}") }
    ```

11. **Ranges: `0..10`** — `range()` doesn't exist

12. **`for..in` skips non-collections silently** — use `chars(s)` for string iteration

13. **Null coalescing: `expr ?? default`** — for missing map keys or None values

14. **`fetch()` accepts 1 or 2 args**
    ```ntnt
    import { fetch } from "std/http"
    let resp = fetch("https://api.example.com/data")
    let resp = fetch("https://api.example.com", map { "method": "POST", "json": data })
    ```

15. **Module-level `let` doesn't support `map {}` literals** — move inside a function

16. **Template strings: `"""..{{expr}}.."""`** — double braces inside triple quotes

## Quick type reference

- Optional shorthand: `fn find(id: Int) -> User?`
- Arrays: `[Int]`, Generics: `fn identity<T>(x: T) -> T`
- `0` is truthy (unlike JS/Python) — check zero explicitly: `if value == 0 { ... }`
- Map access returns `None` for missing keys — use `has_key()` to check existence

## Common imports

```ntnt
import { json, html, parse_form, parse_json } from "std/http/server"
import { fetch } from "std/http"
import { split, join, trim, contains, chars } from "std/string"
import { connect, query, execute, close } from "std/db/postgres"
import { read_file, write_file, exists } from "std/fs"
import { parse_json, stringify } from "std/json"
import { get_env } from "std/env"
import { sha256, uuid } from "std/crypto"
import { keys, values, entries, has_key } from "std/collections"
```

## IDD (Intent-Driven Development)

IDD is the core workflow for ntnt. Write requirements as `.intent` files, implement with annotations, verify automatically.

### Workflow

1. **Draft** a `.intent` file from requirements
2. **Present** to user for approval — do NOT implement before approval
3. **Implement** with `@implements: feature.id` annotations
4. **Verify** with `ntnt intent check` or `ntnt intent studio`

### .intent File Format

```yaml
# server.intent

## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the page loads | status 200 |
| they see "{text}" | body contains "{text}" |

---

Feature: Home Page
  id: feature.home
  description: "Welcome page for visitors"

  Scenario: Shows welcome message
    When a user visits the home page
    → the page loads
    → they see "Welcome"

---

Constraint: Security Headers
  description: "All pages include security headers"
  applies_to: [feature.home]
```

### Code Annotations

```ntnt
// @implements: feature.home
fn home_handler(req) { return html("<h1>Welcome</h1>") }

// @utility — helper, not a feature
fn hash_password(pw) { ... }
```

### Commands

```bash
ntnt intent check file.tnt       # Verify code matches .intent specs
ntnt intent studio file.intent   # Live visual feedback (opens :3001)
ntnt intent coverage file.tnt    # Feature coverage report
ntnt intent init file.intent     # Generate scaffolding from intent
```
