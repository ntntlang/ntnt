# NTNT Language Architecture

## Overview

The NTNT programming language ecosystem is designed as a comprehensive platform for AI-driven software development. This document outlines the high-level architecture, components, and design principles.

## Current Implementation

The current implementation is a tree-walking interpreter written in Rust:

```
src/
├── main.rs          # CLI entry point (run, repl, inspect, validate, intent commands)
├── lib.rs           # Library exports
├── lexer.rs         # Tokenizer for NTNT source code
├── parser.rs        # Recursive descent parser → AST
├── ast.rs           # Abstract syntax tree definitions
├── interpreter.rs   # Tree-walking evaluator with contracts
├── contracts.rs     # Contract checking, old() value storage
├── types.rs         # Type definitions and type checking
├── error.rs         # Error types and formatting
├── intent.rs        # Intent-Driven Development module
├── ial/             # Intent Assertion Language (IAL) engine
│   ├── mod.rs       # Public API: run_assertions(), run_scenario()
│   ├── vocabulary.rs # Pattern matching and term storage
│   ├── resolve.rs   # Recursive term rewriting (Term → Primitives)
│   ├── execute.rs   # Primitive execution against Context
│   ├── primitives.rs # Primitive enum (Http, Check) + CheckOp enum
│   └── standard.rs  # Standard vocabulary definitions
└── stdlib/          # Standard library modules
    ├── mod.rs       # Module registry
    ├── string.rs    # std/string
    ├── math.rs      # std/math
    ├── collections.rs # std/collections
    ├── env.rs       # std/env
    ├── fs.rs        # std/fs
    ├── path.rs      # std/path
    ├── json.rs      # std/json
    ├── time.rs      # std/time
    ├── crypto.rs    # std/crypto
    ├── url.rs       # std/url
    ├── http.rs      # std/http (client)
    ├── http_server.rs # std/http/server
    ├── postgres.rs  # std/db/postgres
    └── concurrent.rs # std/concurrent
```

### Key Features Implemented

- **Lexer**: Full tokenization including contracts, traits, ranges, interpolated strings, raw strings
- **Parser**: Expressions, statements, functions, structs, contracts, traits, for-in loops
- **Interpreter**: Variable scoping, function calls, struct instances, trait dispatch
- **Contracts**: Runtime `requires`/`ensures` enforcement
- **Invariants**: Automatic struct invariant checking
- **Built-ins**: 10 math functions + I/O utilities
- **Traits**: Trait declarations with default methods, `impl Trait for Type`
- **Trait Bounds**: Constrain generics with `<T: Trait>` syntax
- **Iteration**: For-in loops over arrays, ranges, strings, and maps
- **Defer**: Scope-exit cleanup with LIFO execution order
- **Ranges**: Exclusive (`..`) and inclusive (`..=`) range expressions
- **Maps**: Key-value literals with `map { key: value }` syntax
- **String Interpolation**: Embedded expressions with `"Hello, {name}!"`
- **Raw Strings**: Escape-free strings with `r"..."` and `r#"..."#`
- **Intent Assertion Language (IAL)**: Term rewriting engine for natural language → executable tests

### Intent Assertion Language (IAL) Engine

IAL is a **term rewriting system** that translates natural language assertions into executable tests. The engine uses three core functions:

```
┌─────────────────────────────────────────────────────────────────┐
│                         IAL ENGINE                               │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   ┌─────────────┐                                                │
│   │ VOCABULARY  │  ← All knowledge lives here                    │
│   │             │    (standard + glossary + components)          │
│   │ term → def  │                                                │
│   └──────┬──────┘                                                │
│          │                                                       │
│          ▼                                                       │
│   ┌─────────────┐                                                │
│   │  RESOLVE    │  ← Pure function: term → primitives            │
│   │             │    (recursive substitution)                    │
│   └──────┬──────┘                                                │
│          │                                                       │
│          ▼                                                       │
│   ┌─────────────┐     ┌─────────────┐                           │
│   │ PRIMITIVES  │ ──▶ │  EXECUTE    │ ──▶ Results               │
│   │ (actions +  │     │             │                           │
│   │  checks)    │     └─────────────┘                           │
│   └─────────────┘                                                │
│                                                                  │
│   Actions: Http, Cli, Sql, ReadFile                              │
│   Checks:  Equals, Contains, Matches, Exists, InRange, LessThan │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

**Key design principle:** The engine is fixed; all new assertions are vocabulary entries.

See [ROADMAP.md](ROADMAP.md) for the 10-phase plan toward production web applications.

## Core Components

### Language Runtime (Current)

- **Interpreter**: Tree-walking evaluator with contract enforcement and trait dispatch
- **Contract Checker**: Runtime precondition/postcondition validation with `old()` capture
- **Trait System**: Trait definitions, implementations, and method resolution
- **Defer Stack**: LIFO execution of deferred expressions on scope exit
- **Built-in Functions**: Math (`abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `sign`, `clamp`) and I/O (`print`, `len`)
- **Built-in Types**: Arrays, Maps, Ranges, Option, Result
- **Standard Library Modules**:
  - `std/string`: String manipulation (split, join, trim, replace, contains, etc.)
  - `std/math`: Trigonometry and logarithms (sin, cos, tan, log, exp, etc.) with PI, E constants
  - `std/collections`: Array utilities (push, pop, first, last, reverse, slice, concat)
  - `std/env`: Environment access (get_env, args, cwd)
  - `std/fs`: File system operations (read_file, write_file, exists, mkdir, readdir, remove)
  - `std/path`: Path manipulation (join, dirname, basename, extension, resolve, normalize)
  - `std/json`: JSON parsing and stringification (parse, stringify, stringify_pretty)
  - `std/time`: Time operations (now, sleep, elapsed, format_timestamp, duration_secs)
  - `std/crypto`: Cryptographic functions (sha256, hmac_sha256, uuid, random_bytes, hex_encode)
  - `std/url`: URL parsing and encoding (parse, encode, decode, build_query, join)
  - `std/http`: HTTP client (fetch, download, Cache) - unified fetch() API for all HTTP requests
  - `std/http/server`: HTTP server with routing, middleware, static files, parse_json, parse_form, on_shutdown
  - `std/db/postgres`: PostgreSQL database (connect, query, query_one, execute, begin, commit, rollback, close)
  - `std/concurrent`: Go-style concurrency (channel, send, recv, try_recv, recv_timeout, close, sleep_ms, thread_count)

### Language Runtime (Planned)

- **Compiler**: Transforms NTNT source code into executable bytecode or native code
- **Virtual Machine**: Executes NTNT programs with built-in observability
- **Standard Library**: Core types, data structures, and utilities optimized for AI development

### Development Ecosystem

- **Agent Framework**: Runtime environment for AI coding agents
- **Collaboration System**: Multi-agent communication and coordination protocols
- **Observability Engine**: Logging, monitoring, and explainability infrastructure

### Tooling

- **CLI**: `ntnt run <file>`, `ntnt repl`, `ntnt check`, `ntnt parse`, `ntnt lex`, `ntnt test`, `ntnt inspect`, `ntnt validate`, `ntnt intent` commands
- **Intent-Driven Development**: `ntnt intent check` (verify against intent), `ntnt intent coverage` (show feature implementations), `ntnt intent init` (generate scaffolding), `ntnt intent studio` (visual preview with live tests)
- **Intent Assertion Language (IAL)**: Glossary-based natural language assertions with deterministic translation to executable tests via term rewriting
- **Agent Introspection**: `ntnt inspect` outputs JSON describing project structure (functions, routes, middleware, static dirs, file-based routing)
- **Pre-Run Validation**: `ntnt validate` checks syntax and detects unused imports with JSON output
- **File-Based Routing**: `routes()` function auto-discovers routes from directory structure, lib/ modules, and middleware/
- **VS Code Extension**: Syntax highlighting for `.tnt` files
- **IDE Integration**: Language server protocol implementation (planned)
- **Build System**: Integrated compilation, testing, and deployment (planned)
- **Package Manager**: Dependency resolution with semantic versioning (planned)

## Architecture Principles

### AI-First Design

- Deterministic syntax for reliable code generation
- Formal contracts as first-class language constructs
- Structured editing primitives for safe refactoring

### Human Oversight

- Approval gates for critical decisions
- Transparent decision logging
- Human-in-the-loop workflows

### Composability

- Modular design with clear interfaces
- Effect system for predictable side effects
- Protocol-based concurrency

## System Layers

```
┌─────────────────────────────────────────────────┐
│  AI Agents           ← Development orchestration │
├─────────────────────────────────────────────────┤
│  HTTP/API Layer      ← Web server ✅             │
├─────────────────────────────────────────────────┤
│  Language Core       ← Syntax, types, contracts  │
├─────────────────────────────────────────────────┤
│  Runtime             ← Interpreter, contracts    │
├─────────────────────────────────────────────────┤
│  Storage             ← PostgreSQL ✅             │
├─────────────────────────────────────────────────┤
│  Concurrency         ← Channels, threads ✅      │
├─────────────────────────────────────────────────┤
│  Tooling             ← CLI, VS Code, deployment  │
└─────────────────────────────────────────────────┘
```

## Data Flow

1. **Specification** → Human/product requirements translated to contracts
2. **Generation** → AI agents produce code following contracts
3. **Validation** → Automated testing and contract verification
4. **Review** → Human oversight and approval
5. **Deployment** → Automated CI/CD with traceability

## Security Model

- Contract-based access control
- Effect tracking for side effect management
- Audit trails for all AI decisions
- Human approval for sensitive operations

## Scalability

- Distributed agent coordination
- Incremental compilation
- Lazy evaluation for large codebases
- Cloud-native deployment support

## Future Extensions

**Completed (Phases 1-6):**

- HTTP server with contract-verified endpoints ✅
- PostgreSQL database with transactions ✅
- Go-style concurrency with channels ✅
- File-based routing with hot reload ✅
- Agent introspection (`ntnt inspect`) ✅
- Intent-Driven Development (`ntnt intent check|coverage|init|studio`) ✅
- **Intent Assertion Language (IAL)** - Term rewriting engine for natural language tests ✅
  - Vocabulary-based assertion resolution
  - Glossary support for domain-specific terms
  - Standard vocabulary (HTTP, CLI, file, database assertions)
  - Component system for reusable assertion templates
  - Preconditions with verify + skip semantics

**Planned:**

- IAL domain extensions (temporal, state machines, streaming) (Phase 6+)
- Testing framework with contract-based test generation (Phase 7)
- LSP server for IDE integration (Phase 8)
- Package manager with registry (Phase 7)
- Bytecode compiler for performance (Phase 8)
- Native compilation via LLVM/Cranelift (Phase 8)
- Domain-specific dialects
- Formal verification integration
- Docker deployment and container support (Phase 11)

---

_See [ROADMAP.md](ROADMAP.md) for detailed implementation phases and timelines._
