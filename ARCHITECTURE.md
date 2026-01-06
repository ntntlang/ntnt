# Intent Language Architecture

## Overview

The Intent programming language ecosystem is designed as a comprehensive platform for AI-driven software development. This document outlines the high-level architecture, components, and design principles.

## Current Implementation

The current implementation is a tree-walking interpreter written in Rust:

```
src/
├── main.rs          # CLI entry point (run, repl commands)
├── lexer.rs         # Tokenizer for Intent source code
├── parser.rs        # Recursive descent parser → AST
├── ast.rs           # Abstract syntax tree definitions
├── interpreter.rs   # Tree-walking evaluator with contracts
├── contracts.rs     # Contract checking, old() value storage
└── errors.rs        # Error types and formatting
```

### Key Features Implemented

- **Lexer**: Full tokenization including contracts keywords
- **Parser**: Expressions, statements, functions, structs, contracts
- **Interpreter**: Variable scoping, function calls, struct instances
- **Contracts**: Runtime `requires`/`ensures` enforcement
- **Invariants**: Automatic struct invariant checking
- **Built-ins**: 10 math functions + I/O utilities

See [ROADMAP.md](ROADMAP.md) for the 13-phase plan toward production web applications.

## Core Components

### Language Runtime (Current)

- **Interpreter**: Tree-walking evaluator with contract enforcement
- **Contract Checker**: Runtime precondition/postcondition validation with `old()` capture
- **Built-in Functions**: Math (`abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `sign`, `clamp`) and I/O (`print`, `len`)

### Language Runtime (Planned)

- **Compiler**: Transforms Intent source code into executable bytecode or native code
- **Virtual Machine**: Executes Intent programs with built-in observability
- **Standard Library**: Core types, data structures, and utilities optimized for AI development

### Development Ecosystem

- **Agent Framework**: Runtime environment for AI coding agents
- **Collaboration System**: Multi-agent communication and coordination protocols
- **Observability Engine**: Logging, monitoring, and explainability infrastructure

### Tooling

- **CLI**: `intent run <file>` and `intent repl` commands
- **VS Code Extension**: Syntax highlighting for `.intent` and `.itn` files
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
│  HTTP/API Layer      ← Web server (Phase 5)      │
├─────────────────────────────────────────────────┤
│  Language Core       ← Syntax, types, contracts  │
├─────────────────────────────────────────────────┤
│  Runtime             ← Interpreter, contracts    │
├─────────────────────────────────────────────────┤
│  Storage             ← Database (Phase 6)        │
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

- HTTP server with contract-verified endpoints (Phase 5)
- Database access with repository patterns (Phase 6)
- Async/await for concurrent operations (Phase 7)
- Domain-specific dialects
- Integration with existing languages
- Advanced AI reasoning capabilities
- Formal verification integration
- Docker deployment and container support (Phase 11)

---

_See [ROADMAP.md](ROADMAP.md) for detailed implementation phases and timelines._
