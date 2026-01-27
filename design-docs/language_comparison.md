# NTNT Language Comparison

A comprehensive comparison of NTNT against popular programming languages for web development.

## Quick Overview

| Language | Primary Use | Typing | Paradigm | Runtime |
|----------|-------------|--------|----------|---------|
| **NTNT** | AI-driven web apps | Static (gradual) | Multi-paradigm | Interpreted (Rust) |
| TypeScript | Web (full-stack) | Static (gradual) | Multi-paradigm | Node.js/Deno/Bun |
| Go | Backend services | Static | Imperative | Compiled |
| Ruby | Web apps (Rails) | Dynamic | OOP | Interpreted |
| Rust | Systems/Backend | Static | Multi-paradigm | Compiled |
| Python | General/ML/Web | Dynamic | Multi-paradigm | Interpreted |
| PHP | Web apps | Dynamic (gradual) | Multi-paradigm | Interpreted |
| Elixir | Distributed systems | Dynamic | Functional | BEAM VM |
| Java | Enterprise | Static | OOP | JVM |
| C# | Enterprise/.NET | Static | Multi-paradigm | CLR |

---

## Type System

| Feature | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|---------|------|------------|-----|------|------|--------|-----|--------|
| Static typing | ✅ | ✅ | ✅ | ❌ | ✅ | ❌* | ❌* | ❌ |
| Type inference | ✅ | ✅ | ✅ | N/A | ✅ | N/A | N/A | N/A |
| Generics | ✅ | ✅ | ✅ | ❌ | ✅ | ✅* | ❌ | ❌ |
| Union types | ✅ | ✅ | ❌ | ❌ | ✅* | ✅* | ✅* | ❌ |
| Option/Maybe | ✅ | ❌* | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ |
| Result type | ✅ | ❌ | ❌* | ❌ | ✅ | ❌ | ❌ | ✅* |
| Pattern matching | ✅ | ❌* | ❌ | ✅* | ✅ | ✅ | ✅* | ✅ |
| Algebraic data types | ✅ | ✅* | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ |
| Null safety | ✅ | ✅* | ❌ | ❌ | ✅ | ❌ | ❌ | ❌ |
| Type aliases | ✅ | ✅ | ✅ | N/A | ✅ | ✅ | ✅* | N/A |

*Notes: Python/PHP have optional type hints. TypeScript union types approximate ADTs. Go uses multiple returns for errors. Ruby has case/when. Elixir has {:ok, val}/{:error, reason}.*

---

## Contract & Safety Features

| Feature | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|---------|------|------------|-----|------|------|--------|-----|--------|
| **Preconditions (requires)** | ✅ | ❌ | ❌ | ❌ | ❌* | ❌ | ❌ | ❌ |
| **Postconditions (ensures)** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Struct invariants** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **old() in postconditions** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Runtime contract checking | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Memory safety | ✅* | ✅* | ✅ | ✅* | ✅ | ✅* | ✅* | ✅ |
| Data race prevention | ❌ | ❌ | ✅* | ❌ | ✅ | ❌ | ❌ | ✅ |
| Immutability by default | ✅ | ❌ | ❌ | ❌ | ✅ | ❌ | ❌ | ✅ |

*Notes: NTNT's contract system is a key differentiator. Rust has debug_assert! but not language-level contracts. Go has race detector. GC languages have memory safety via GC.*

**NTNT Contract Example:**
```ntnt
fn withdraw(account: Account, amount: Int) -> Account
    requires amount > 0
    requires account.balance >= amount
    ensures result.balance == old(account.balance) - amount
{
    return Account { balance: account.balance - amount }
}
```

---

## Web Development Features

| Feature | NTNT | TypeScript | Go | Ruby/Rails | Rust | Python/Django | PHP/Laravel | Elixir/Phoenix |
|---------|------|------------|-----|------------|------|---------------|-------------|----------------|
| Built-in HTTP server | ✅ | ❌* | ✅ | ✅ | ❌* | ❌* | ❌* | ✅ |
| File-based routing | ✅ | ✅* | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Route params `{id}` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Middleware | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Static file serving | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Template engine | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| JSON handling | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Form parsing | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| WebSocket | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Hot reload (dev) | ✅ | ✅ | ❌* | ✅ | ❌ | ✅ | ✅ | ✅ |
| Database (Postgres) | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| ORM | ❌ | ✅* | ✅* | ✅ | ✅* | ✅ | ✅ | ✅ |

*Notes: TypeScript needs Express/Fastify/etc. Next.js has file-based routing. Go has net/http. Rust needs Actix/Axum. Python needs Flask/Django.*

**NTNT Web Example:**
```ntnt
import { json, html } from "std/http/server"

fn get_user(req) {
    let id = req.params["id"]
    return json(map { "id": id, "name": "User " + id })
}

get(r"/users/{id}", get_user)
serve_static("/static", "./public")
listen(8080)
```

---

## AI & Agent Features

| Feature | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|---------|------|------------|-----|------|------|--------|-----|--------|
| **Intent files (.intent)** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **@implements annotations** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Intent-driven testing** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| **Intent Studio (visual)** | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| JSON introspection | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Self-documenting contracts | ✅ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ | ❌ |
| Machine-readable errors | ✅* | ✅* | ✅* | ❌ | ✅ | ❌ | ❌ | ❌ |

*Notes: Intent-Driven Development (IDD) is unique to NTNT. Most languages have LSP for editor integration.*

**NTNT Intent Example:**
```yaml
## Glossary

| Term | Means |
|------|-------|
| a user requests {path} | GET {path} |
| the request succeeds | status 200 |
| they see {text} | body contains {text} |

---

Feature: User Management
  id: feature.user_management

  Scenario: Get user by ID
    When a user requests /users/123
    → the request succeeds
    → they see "User"
```

---

## Concurrency Model

| Feature | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|---------|------|------------|-----|------|------|--------|-----|--------|
| Model | Channels | Event loop | Goroutines | Threads/Ractors | Threads/async | GIL/async | None* | Actors |
| Channels | ✅ | ❌ | ✅ | ❌ | ✅ | ✅* | ❌ | ✅ |
| Async/await | ❌ | ✅ | ❌ | ❌ | ✅ | ✅ | ✅* | ❌ |
| Parallel execution | ✅* | ❌* | ✅ | ✅* | ✅ | ❌* | ❌ | ✅ |
| Shared memory | ❌ | ✅ | ✅* | ✅ | ✅ | ✅ | ❌ | ❌ |
| Message passing | ✅ | ❌ | ✅ | ❌ | ✅ | ✅* | ❌ | ✅ |

*Notes: NTNT uses Go-style channels. TypeScript is single-threaded (workers available). Python has GIL limiting parallelism. PHP typically uses process-per-request.*

**NTNT Concurrency Example:**
```ntnt
import { channel, send, recv } from "std/concurrent"

let ch = channel()
send(ch, map { "task": "process", "id": 123 })
let result = recv(ch)
```

---

## Developer Experience

| Feature | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|---------|------|------------|-----|------|------|--------|-----|--------|
| REPL | ✅ | ✅* | ❌ | ✅ | ❌* | ✅ | ✅ | ✅ |
| Package manager | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| LSP support | ❌* | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Syntax highlighting | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Debugger | ❌ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Linter | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Formatter | ❌ | ✅ | ✅ | ✅* | ✅ | ✅ | ✅ | ✅ |
| Test framework | ✅* | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Documentation gen | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Shell completions | ✅ | ✅* | ✅* | ✅* | ✅ | ✅* | ✅* | ✅* |

*Notes: NTNT has intent-based testing. LSP is planned. TypeScript REPL via ts-node.*

---

## Standard Library

| Module | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|--------|------|------------|-----|------|------|--------|-----|--------|
| String manipulation | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Math functions | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| Collections | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| File system | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| JSON | ✅ | ✅ | ✅ | ✅ | ✅* | ✅ | ✅ | ✅ |
| HTTP client | ✅ | ✅* | ✅ | ✅ | ✅* | ✅ | ✅ | ✅ |
| HTTP server | ✅ | ❌* | ✅ | ✅* | ❌* | ❌* | ❌* | ✅ |
| Crypto/hashing | ✅ | ✅* | ✅ | ✅ | ✅* | ✅ | ✅ | ✅ |
| Date/time | ✅ | ✅ | ✅ | ✅ | ✅* | ✅ | ✅ | ✅ |
| URL parsing | ✅ | ✅ | ✅ | ✅ | ✅* | ✅ | ✅ | ✅ |
| CSV | ✅ | ❌* | ✅ | ✅ | ✅* | ✅ | ✅* | ✅* |
| Regex | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| UUID | ✅ | ❌* | ❌* | ✅ | ✅* | ✅ | ✅* | ✅ |
| Environment | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |

*Notes: Many features require third-party packages in some languages. Rust stdlib is minimal by design.*

---

## Syntax Comparison

### Variable Declaration

| Language | Immutable | Mutable |
|----------|-----------|---------|
| NTNT | `let x = 5` | `let mut x = 5` |
| TypeScript | `const x = 5` | `let x = 5` |
| Go | `x := 5` (inferred) | Same |
| Ruby | N/A | `x = 5` |
| Rust | `let x = 5` | `let mut x = 5` |
| Python | N/A | `x = 5` |
| PHP | N/A | `$x = 5` |
| Elixir | `x = 5` (rebindable) | N/A |

### Function Definition

| Language | Syntax |
|----------|--------|
| NTNT | `fn add(a: Int, b: Int) -> Int { a + b }` |
| TypeScript | `function add(a: number, b: number): number { return a + b }` |
| Go | `func add(a, b int) int { return a + b }` |
| Ruby | `def add(a, b) a + b end` |
| Rust | `fn add(a: i32, b: i32) -> i32 { a + b }` |
| Python | `def add(a: int, b: int) -> int: return a + b` |
| PHP | `function add(int $a, int $b): int { return $a + $b; }` |
| Elixir | `def add(a, b), do: a + b` |

### Map/Object Literal

| Language | Syntax |
|----------|--------|
| NTNT | `map { "key": "value" }` |
| TypeScript | `{ key: "value" }` |
| Go | `map[string]string{"key": "value"}` |
| Ruby | `{ key: "value" }` |
| Rust | `HashMap::from([("key", "value")])` |
| Python | `{"key": "value"}` |
| PHP | `["key" => "value"]` |
| Elixir | `%{key: "value"}` |

### String Interpolation

| Language | Syntax |
|----------|--------|
| NTNT | `"Hello, {name}!"` |
| TypeScript | `` `Hello, ${name}!` `` |
| Go | `fmt.Sprintf("Hello, %s!", name)` |
| Ruby | `"Hello, #{name}!"` |
| Rust | `format!("Hello, {}!", name)` |
| Python | `f"Hello, {name}!"` |
| PHP | `"Hello, {$name}!"` |
| Elixir | `"Hello, #{name}!"` |

### Error Handling

| Language | Pattern |
|----------|---------|
| NTNT | `match result { Ok(v) => v, Err(e) => handle(e) }` |
| TypeScript | `try { } catch (e) { }` |
| Go | `if err != nil { return err }` |
| Ruby | `begin rescue end` |
| Rust | `match result { Ok(v) => v, Err(e) => handle(e) }` |
| Python | `try: except:` |
| PHP | `try { } catch (Exception $e) { }` |
| Elixir | `case result do {:ok, v} -> v; {:error, e} -> handle(e) end` |

---

## Performance Characteristics

| Aspect | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|--------|------|------------|-----|------|------|--------|-----|--------|
| Execution | Interpreted | JIT (V8) | Compiled | Interpreted | Compiled | Interpreted | JIT (opcache) | BEAM VM |
| Startup time | Fast | Medium | Very fast | Slow | Very fast | Slow | Fast | Medium |
| Peak performance | Medium | Medium | High | Low | Very high | Low | Medium | Medium |
| Memory usage | Medium | High | Low | High | Very low | High | Medium | Medium |
| GC pauses | Yes | Yes | Yes | Yes | No | Yes | Yes | Yes* |

*Notes: NTNT is currently interpreted; bytecode VM and native compilation planned. Elixir has per-process GC with minimal pauses.*

---

## Learning Curve

| Language | From Zero | From JavaScript | From Python | From Go |
|----------|-----------|-----------------|-------------|---------|
| NTNT | Medium | Easy | Easy | Easy |
| TypeScript | Medium | Very easy | Easy | Medium |
| Go | Easy | Medium | Medium | - |
| Ruby | Easy | Easy | Very easy | Medium |
| Rust | Hard | Hard | Hard | Medium |
| Python | Very easy | Easy | - | Easy |
| PHP | Easy | Easy | Easy | Easy |
| Elixir | Medium | Medium | Medium | Medium |

---

## Ecosystem Maturity

| Aspect | NTNT | TypeScript | Go | Ruby | Rust | Python | PHP | Elixir |
|--------|------|------------|-----|------|------|--------|-----|--------|
| Age | 2024 | 2012 | 2009 | 1995 | 2010 | 1991 | 1995 | 2011 |
| Package count | ~15 | 2M+ | 500K+ | 180K+ | 140K+ | 500K+ | 350K+ | 15K+ |
| Community size | Small | Very large | Large | Large | Large | Very large | Very large | Medium |
| Job market | Emerging | Very high | High | Medium | Growing | Very high | High | Niche |
| Production use | Early | Widespread | Widespread | Widespread | Growing | Widespread | Widespread | Niche |
| Documentation | Good | Excellent | Excellent | Excellent | Excellent | Excellent | Good | Excellent |

---

## Best Use Cases

| Language | Ideal For |
|----------|-----------|
| **NTNT** | AI-assisted web apps, contract-driven APIs, rapid prototyping with safety guarantees |
| TypeScript | Full-stack web, large team projects, gradual typing adoption |
| Go | Microservices, CLI tools, high-concurrency backends |
| Ruby/Rails | Rapid web development, MVPs, content-heavy sites |
| Rust | Systems programming, performance-critical services, WebAssembly |
| Python | ML/AI, scripting, data science, rapid prototyping |
| PHP | Traditional web apps, WordPress/CMS, shared hosting |
| Elixir | Real-time systems, chat, high-availability services |

---

## NTNT Unique Advantages

1. **Contract System**: First-class preconditions, postconditions, and invariants
2. **Intent-Driven Development**: `.intent` files bridge requirements and implementation
3. **Intent Studio**: Visual tool for collaborative intent development
4. **AI-Native Design**: Built for AI agent collaboration from the ground up
5. **Zero-Config Web**: Built-in HTTP server with file-based routing
6. **Hot Reload**: Development changes apply instantly
7. **Self-Documenting**: Contracts serve as executable documentation
8. **Single Binary**: No runtime dependencies (Rust-based interpreter)

---

## NTNT Current Limitations

1. **No LSP**: Editor integration is syntax highlighting only (planned)
2. **No Package Manager**: Standard library only (planned)
3. **No Debugger**: Print debugging only (planned)
4. **Interpreted Only**: Bytecode VM and native compilation planned
5. **Small Ecosystem**: New language, limited community packages
6. **No WebSocket**: HTTP only (planned)
7. **Limited Database Support**: PostgreSQL only (more planned)

---

## Migration Guides

### From TypeScript/JavaScript

| TypeScript | NTNT |
|------------|------|
| `const x = 5` | `let x = 5` |
| `let x = 5` | `let mut x = 5` |
| `{ key: value }` | `map { "key": value }` |
| `` `${expr}` `` | `"{expr}"` |
| `async/await` | Channels |
| `null ?? default` | `unwrap_or(option, default)` |
| `express.get()` | `get()` (global) |

### From Python

| Python | NTNT |
|--------|------|
| `x = 5` | `let x = 5` or `let mut x = 5` |
| `{"key": value}` | `map { "key": value }` |
| `f"{expr}"` | `"{expr}"` |
| `def func():` | `fn func() { }` |
| `for x in list:` | `for x in list { }` |
| `if x:` | `if x { }` |
| `import json` | `import { parse_json } from "std/json"` |

### From Go

| Go | NTNT |
|----|------|
| `x := 5` | `let x = 5` |
| `var x int = 5` | `let x: Int = 5` |
| `map[string]int{}` | `map { }` |
| `fmt.Sprintf()` | `"{expr}"` |
| `func name() {}` | `fn name() { }` |
| `if err != nil` | `match result { Ok(v) => ..., Err(e) => ... }` |
| `go func()` | Channels (no goroutines) |

---

*Last updated: January 2026 (v0.3.6)*
