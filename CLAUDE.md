# NTNT Language - Claude Code Instructions

NTNT (pronounced "Intent") is an agent-native programming language for AI-driven web development. File extension: `.tnt`

## Building NTNT

```bash
# Fast dev-release build (for development)
cargo build --profile dev-release

# Standard release build (for distribution)
cargo build --release
```

## Quick Start

```bash
ntnt lint file.tnt    # ALWAYS lint first
ntnt run file.tnt     # Run after lint passes
ntnt test server.tnt --get /health  # Test HTTP endpoints
```

## Documentation

**See [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) for complete syntax reference.**

Auto-generated references:
- [STDLIB_REFERENCE.md](docs/STDLIB_REFERENCE.md) - All functions
- [SYNTAX_REFERENCE.md](docs/SYNTAX_REFERENCE.md) - Language syntax
- [IAL_REFERENCE.md](docs/IAL_REFERENCE.md) - Intent Assertion Language

## Environment Variables

| Variable | Values | Description |
|----------|--------|-------------|
| `NTNT_ENV` | `production`, `prod` | Disables hot-reload for better performance |
| `NTNT_STRICT` | `1`, `true` | Enables strict type checking — blocks execution and hot-reload if type errors are found |

```bash
# Development (default) - hot-reload enabled
ntnt run server.tnt

# Production - hot-reload disabled
NTNT_ENV=production ntnt run server.tnt

# Strict type checking - blocks on type errors
NTNT_STRICT=1 ntnt run server.tnt
```

## Type System

NTNT has a **gradual type system** with a static type checker that runs in `ntnt lint` and `ntnt validate`. The interpreter remains fully dynamic — types are checked before execution, never during.

### How it works

- **Type annotations are optional.** Untyped parameters and variables default to `Any`, which is compatible with everything. Existing untyped code produces zero errors.
- **`ntnt lint` and `ntnt validate`** always run the type checker and report type errors alongside other lint issues.
- **`ntnt run`** does NOT run the type checker by default. Set `NTNT_STRICT=1` to enable it (blocks execution and hot-reload on type errors).
- **`ntnt lint --strict`** warns about untyped function parameters and missing return type annotations. Activated by CLI flag, `NTNT_STRICT=1` env var, or `ntnt.toml` config.
- **Local inference only.** The checker infers types from literals, operators, and function return types — no global inference.
- **Two-pass design.** Declarations are collected first, so forward references work (call a function before it's defined).

### What the type checker catches

```ntnt
// Argument type mismatch
fn greet(name: String) -> String { return "Hello, " + name }
greet(42)  // error: expected String but got Int

// Return type mismatch
fn get_count() -> Int { return "not a number" }  // error: expected Int, got String

// Wrong argument count
fn add(a: Int, b: Int) -> Int { return a + b }
add(1)  // error: expects 2 arguments, got 1

// Struct field type mismatch
struct Point { x: Int, y: Int }
let p = Point { x: "wrong", y: 2 }  // error: expected Int but got String

// Contract expressions are type-checked too
fn greet(name: String) -> String
    ensures len(result) > 0  // ✓ checker knows result is String, len(String) is valid
{ return "Hello, " + name }
```

### Built-in types for HTTP

The type checker provides `Request` and `Response` as built-in struct types:
- **`Request`** — fields: `method`, `path`, `body`, `url`, `query`, `id`, `ip`, `protocol` (all `String`), `params`, `query_params`, `headers` (all `Map<String, String>`)
- **`Response`** — fields: `status` (`Int`), `body` (`String`), `headers` (`Map<String, String>`)
- `html()`, `json()`, `text()`, `redirect()`, `status()` all return `Response`
- `unwrap()` is generic-aware: `unwrap(Optional<T>)` → `T`, `unwrap(Result<T,E>)` → `T`
- Collection functions preserve element types: `filter()`, `sort()`, `reverse()`, `slice()`, `concat()`, `push()` → `Array<T>`; `first()`, `last()`, `pop()` → `T`; `flatten(Array<Array<T>>)` → `Array<T>`
- Math builtins preserve numeric types: `abs()`, `min()`, `max()`, `clamp()` → `Int` or `Float`
- Map accessors are type-aware: `keys(Map<K,V>)` → `Array<K>`, `values(Map<K,V>)` → `Array<V>`, `get_key(Map<K,V>, key)` → `V`
- `transform(arr, callback)` infers return type from callback: `transform(Array<Int>, fn(Int)->String)` → `Array<String>`
- `parse_json()` returns `Result<Map<String, Any>, String>` — unwrap gives a map, match narrows correctly
- `fetch()` returns `Result<Response, String>` — unwrap gives `Response` with typed field access
- Cross-file imports propagate types: `import { foo } from "./lib/utils"` resolves `foo`'s signature from the file

### What it does NOT catch (gradual typing)

```ntnt
// Untyped code is always fine — no false positives
fn process(data) { return data + 1 }  // no error (data is Any)

// Mixed typed/untyped is fine
fn typed(x: Int) -> Int { return x + 1 }
let val = get_input()  // val is Any
typed(val)             // no error (Any is compatible with Int)
```

## Critical Syntax Rules (Most Common Mistakes)

### Map literals require `map` keyword
```ntnt
let data = map { "key": "value" }  // CORRECT
let data = { "key": "value" }      // WRONG - {} is a block
```

### String interpolation uses `{expr}` not `${expr}`
```ntnt
let msg = "Hello, {name}!"  // CORRECT
let msg = "Hello, ${name}!" // WRONG
```

### Route patterns auto-detect `{param}` — raw strings optional
```ntnt
get("/users/{id}", handler)    // CORRECT — auto-detected as route param
get(r"/users/{id}", handler)   // Also works (backward compatible)
```

### Use pipe operator for data transformation chains
```ntnt
// Pipe passes left side as first argument to right side
let result = "  Hello  " |> trim |> to_lower  // "hello"
let parts = "a,b,c" |> split(",") |> join("-")  // "a-b-c"
```

### HTTP routing functions are global builtins
```ntnt
// ONLY import response builders
import { json, html } from "std/http/server"

// Use Request/Response types for fully typed handlers
fn handler(req: Request) -> Response { return json(map { "ok": true }) }

// Routes are global - no import needed
get("/api", handler)
listen(8080)
```

## Error Messages

Error messages include error codes (E001-E012) and color-coded output. Typos in variable or function names trigger "Did you mean?" suggestions using Levenshtein distance. Parser errors show source code snippets with line/column context.

## Template Strings

Use triple-quoted strings `"""..."""` with `{{expr}}` for HTML templates (single `{}` pass through for CSS):

```ntnt
let page = """
<style>h1 { color: blue; }</style>
<h1>Hello, {{name}}!</h1>
{{#for item in items}}<li>{{item}}</li>{{/for}}
"""
```

See [LANGUAGE_GUIDE.md](LANGUAGE_GUIDE.md#template-strings) for loops, conditionals, and filters.

## Intent-Driven Development (IDD)

IDD is **collaborative**. Always present the `.intent` file to the user before implementing.

```bash
ntnt intent studio server.intent  # Visual preview + live tests
ntnt intent check server.tnt      # Verify implementation
```

See [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md#intent-driven-development-idd) for full IDD workflow.

## Documentation Maintenance (MANDATORY)

**After implementing any language feature, follow the checklist in [.plans/doc-maintenance-guide.md](.plans/doc-maintenance-guide.md).**

This includes: integration tests, example files, TOML doc sources, AI_AGENT_GUIDE.md, LANGUAGE_GUIDE.md, ARCHITECTURE.md, ROADMAP.md, and this file. Run `ntnt docs --generate` after editing any TOML file. Do not wait for the user to ask — update docs as part of every implementation task.
