# NTNT Language Architecture

## Overview

NTNT is an agent-native programming language designed for AI-driven web development. This document describes the current implementation architecture.

## Source Structure

```
src/
├── main.rs              # CLI entry point and command handlers
├── lib.rs               # Library exports
├── lexer.rs             # Tokenizer for NTNT source code
├── parser.rs            # Recursive descent parser → AST
├── ast.rs               # Abstract syntax tree definitions
├── interpreter.rs       # Tree-walking evaluator with contracts
├── contracts.rs         # Contract checking, old() value capture
├── typechecker.rs       # Static type checker (gradual typing, strict lint)
├── types.rs             # Type definitions and compatibility
├── error.rs             # Error types and formatting
├── intent.rs            # Intent-Driven Development module
│
├── ial/                 # Intent Assertion Language engine
│   ├── mod.rs           # Public API: run_assertions(), run_scenario()
│   ├── vocabulary.rs    # Pattern matching and term storage
│   ├── resolve.rs       # Recursive term rewriting (Term → Primitives)
│   ├── execute.rs       # Primitive execution against Context
│   ├── primitives.rs    # Primitive types (Http, Cli, Check) + CheckOp enum
│   └── standard.rs      # Standard vocabulary definitions + glossary parsing
│
└── stdlib/              # Standard library modules
    ├── mod.rs           # Module registry
    ├── string.rs        # std/string - String manipulation
    ├── math.rs          # std/math - Math functions and constants
    ├── collections.rs   # std/collections - Array/map utilities
    ├── env.rs           # std/env - Environment variables
    ├── fs.rs            # std/fs - File system operations
    ├── path.rs          # std/path - Path manipulation
    ├── json.rs          # std/json - JSON parse/stringify
    ├── csv.rs           # std/csv - CSV parse/stringify
    ├── time.rs          # std/time - Time operations with timezone support
    ├── crypto.rs        # std/crypto - SHA256, HMAC, UUID, random
    ├── url.rs           # std/url - URL encoding/parsing
    ├── http.rs          # std/http - HTTP client (fetch, download)
    ├── http_server.rs   # std/http/server - Response builders
    ├── http_server_async.rs  # Async HTTP server (Axum + Tokio)
    ├── http_bridge.rs   # Bridge between async server and sync interpreter
    ├── template.rs      # External template loading
    ├── postgres.rs      # std/db/postgres - PostgreSQL client
    └── concurrent.rs    # std/concurrent - Channels, sleep
```

## CLI Commands

| Command | Description |
|---------|-------------|
| `ntnt run <file>` | Execute an NTNT file |
| `ntnt repl` | Interactive REPL |
| `ntnt lint <file>` | Validate syntax (recommended before run) |
| `ntnt test <file>` | Test HTTP endpoints |
| `ntnt parse <file>` | Show AST |
| `ntnt lex <file>` | Show tokens |
| `ntnt inspect <file>` | JSON project structure for agents |
| `ntnt validate <file>` | Syntax validation with JSON output |
| `ntnt intent check <file>` | Verify implementation against intent |
| `ntnt intent coverage <file>` | Show feature implementation coverage |
| `ntnt intent init <file>` | Generate code scaffolding from intent |
| `ntnt intent studio <file>` | Visual preview with live tests |
| `ntnt docs [query]` | Search stdlib documentation |
| `ntnt docs --generate` | Generate reference docs from TOML |
| `ntnt docs --validate` | Validate documentation completeness |
| `ntnt completions <shell>` | Generate shell completions |

## Core Components

### Lexer (`lexer.rs`)

Tokenizes NTNT source code including:
- Keywords, identifiers, literals
- String interpolation (`"Hello, {name}!"`)
- Raw strings (`r"..."`, `r#"..."#`)
- Template strings (`"""..."""`)
- Range operators (`..`, `..=`)
- Contract keywords (`requires`, `ensures`, `invariant`)

### Parser (`parser.rs`)

Recursive descent parser producing an AST:
- Expressions with operator precedence
- Statements (let, if, for, match, defer)
- Functions with contracts
- Structs and traits
- Imports and exports

### Interpreter (`interpreter.rs`)

Tree-walking evaluator with:
- Lexical scoping
- Contract enforcement at runtime
- Trait method dispatch
- Defer stack (LIFO cleanup)
- Built-in functions (30+)
- HTTP server integration

### Contract System (`contracts.rs`)

Runtime contract checking:
- `requires` - Preconditions checked on function entry
- `ensures` - Postconditions checked on function exit
- `old()` - Captures values at function entry for postconditions
- `invariant` - Struct invariants checked after mutations

## HTTP Server Architecture

The HTTP server uses a bridge pattern to connect async Axum handlers to the synchronous interpreter:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Tokio Async Runtime                         │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐                         │
│  │ Task 1  │  │ Task 2  │  │ Task N  │  (async handlers)       │
│  └────┬────┘  └────┬────┘  └────┬────┘                         │
│       └────────────┼────────────┘                               │
│                    │                                            │
│              ┌─────▼─────┐                                      │
│              │  Channel  │  (mpsc + oneshot reply)              │
│              └─────┬─────┘                                      │
└────────────────────┼────────────────────────────────────────────┘
                     │
┌────────────────────▼────────────────────────────────────────────┐
│                  Interpreter Thread                              │
│  - Receives requests via channel                                 │
│  - Finds and calls NTNT handler function                         │
│  - Sends response back via oneshot channel                       │
│  - Uses Rc<RefCell<>> (not thread-safe, hence single thread)     │
└─────────────────────────────────────────────────────────────────┘
```

**Key files:**
- `http_server_async.rs` - Axum server setup, async handlers, static files
- `http_bridge.rs` - Request/response types, channel communication
- `http_server.rs` - Response builders (`json()`, `html()`, etc.)

## Intent Assertion Language (IAL)

IAL is a term rewriting system that translates natural language assertions into executable tests.

```
┌─────────────────────────────────────────────────────────────────┐
│                         IAL ENGINE                               │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   ┌─────────────┐                                                │
│   │ VOCABULARY  │  ← Standard terms + glossary + components      │
│   │ term → def  │                                                │
│   └──────┬──────┘                                                │
│          │                                                       │
│          ▼                                                       │
│   ┌─────────────┐                                                │
│   │  RESOLVE    │  ← Recursive term rewriting                    │
│   │             │    (term → expanded terms → primitives)        │
│   └──────┬──────┘                                                │
│          │                                                       │
│          ▼                                                       │
│   ┌─────────────┐     ┌─────────────┐                           │
│   │ PRIMITIVES  │ ──▶ │  EXECUTE    │ ──▶ Pass/Fail             │
│   └─────────────┘     └─────────────┘                           │
│                                                                  │
│   Primitives:                                                    │
│   - Http(method, path, body?)     - FunctionCall(name, args)    │
│   - Cli(command)                  - PropertyCheck(fn, type)     │
│   - CodeQuality(path)             - Check(op, path, expected)   │
│   - ReadFile(path)                                               │
│                                                                  │
│   Check Operations:                                              │
│   Equals, NotEquals, Contains, NotContains, Matches,            │
│   Exists, NotExists, LessThan, GreaterThan, InRange,            │
│   StartsWith, EndsWith, IsType, HasLength                       │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Key design principle:** The engine is fixed; all new assertions are vocabulary entries.

## Documentation System

Documentation is auto-generated from TOML source files:

```
docs/
├── stdlib.toml          → STDLIB_REFERENCE.md   (functions)
├── syntax.toml          → SYNTAX_REFERENCE.md   (keywords, operators)
├── ial.toml             → IAL_REFERENCE.md      (IAL primitives, terms)
```

Generate with: `ntnt docs --generate`

The CI pipeline validates that generated docs are up-to-date.

## Standard Library

| Module | Description |
|--------|-------------|
| `std/string` | 35+ string functions (split, join, trim, replace, regex) |
| `std/math` | Trig, log, exp, random, constants (PI, E) |
| `std/collections` | Array/map utilities (push, pop, keys, values, map, filter) |
| `std/env` | Environment variables (get_env, load_env) |
| `std/fs` | File operations (read_file, write_file, mkdir, readdir) |
| `std/path` | Path manipulation (join, dirname, basename, extension) |
| `std/json` | JSON parse/stringify |
| `std/csv` | CSV parse/stringify with headers |
| `std/time` | Time with IANA timezone support |
| `std/crypto` | SHA256, HMAC, UUID, random bytes |
| `std/url` | URL encoding, query string parsing |
| `std/http` | HTTP client (fetch, download) |
| `std/http/server` | Response builders (json, html, redirect) |
| `std/db/postgres` | PostgreSQL with transactions |
| `std/concurrent` | Channels, sleep |

## Built-in Functions

Available without import:

| Category | Functions |
|----------|-----------|
| Type conversion | `str`, `int`, `float`, `type` |
| Math | `abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `trunc`, `sign`, `clamp` |
| Collections | `len`, `push`, `assert` |
| I/O | `print` |
| Option/Result | `Some`, `None`, `Ok`, `Err`, `unwrap`, `unwrap_or`, `is_some`, `is_none`, `is_ok`, `is_err` |
| HTTP Server | `get`, `post`, `put`, `patch`, `delete`, `listen`, `serve_static`, `routes`, `template`, `use_middleware`, `on_shutdown` |

## System Layers

```
┌─────────────────────────────────────────────────┐
│  Intent-Driven Development                       │
│  (.intent files, IAL assertions, Studio)         │
├─────────────────────────────────────────────────┤
│  HTTP Server (Axum + Tokio)                      │
│  Routes, middleware, static files, templates     │
├─────────────────────────────────────────────────┤
│  Language Core                                   │
│  Lexer → Parser → AST → Interpreter              │
├─────────────────────────────────────────────────┤
│  Contract System                                 │
│  requires, ensures, invariant, old()             │
├─────────────────────────────────────────────────┤
│  Standard Library                                │
│  String, math, fs, http, postgres, time, etc.    │
├─────────────────────────────────────────────────┤
│  Tooling                                         │
│  CLI, VS Code extension, doc generation          │
└─────────────────────────────────────────────────┘
```

## Key Features

- **Tree-walking interpreter** - Simple, debuggable execution model
- **Runtime contracts** - Preconditions, postconditions, invariants
- **Traits** - Declaration, implementation, default methods, bounds
- **Defer** - Scope-exit cleanup with LIFO order
- **Channels** - Go-style concurrency (no async/await)
- **File-based routing** - Auto-discover routes from directory structure
- **External templates** - Mustache-style with loops, conditionals, filters
- **Intent-Driven Development** - Collaborative workflow with assertions
- **Auto-generated docs** - TOML source files generate markdown references

## Future Directions

- LSP server for IDE integration
- Package manager with registry
- Bytecode compiler for performance
- Native compilation (LLVM/Cranelift)
- Docker deployment support

---

_See [ROADMAP.md](ROADMAP.md) for implementation phases._
