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

```bash
# Development (default) - hot-reload enabled
ntnt run server.tnt

# Production - hot-reload disabled
NTNT_ENV=production ntnt run server.tnt
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

fn handler(req) { return json(map { "ok": true }) }

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
