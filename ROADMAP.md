# NTNT Language Implementation Roadmap

This document outlines the implementation plan for NTNT, a programming language designed for AI-driven development. The roadmap prioritizes getting to a working web application quickly while focusing on NTNT's unique differentiators: contracts, AI integration, and intent encoding.

> **Completed Phases:** Phases 1-5 (Core Language, Type System, Modules, Traits, Web) are 100% complete and documented in [ROADMAP_COMPLETE.md](ROADMAP_COMPLETE.md).

---

## Design Principles

1. **Self-Contained**: NTNT has no runtime dependencies on other languages. The interpreter/compiler is written in Rust, but NTNT programs are pure Intent.

2. **AI-First**: Features that enable AI development (contracts, intent annotations, structured edits) are core, not afterthoughts.

3. **Production-Ready Web Apps**: The goal is building real web applications with safety guarantees.

4. **Lean Standard Library**: Include essentials, leave specialized libraries to the community.

---

## Current Status

### Completed ✅

- [x] Lexer with full token support
- [x] Recursive descent parser
- [x] Complete AST definitions
- [x] Tree-walking interpreter
- [x] Basic type system (Int, Float, String, Bool, Array, Object, Function, Unit) — enforced with gradual typing (Phase 7.1)
- [x] Full contract system (`requires`, `ensures`, `old()`, `result`)
- [x] Struct invariants with automatic checking
- [x] Built-in math functions (`abs`, `min`, `max`, `sqrt`, `pow`, `round` with optional decimals, etc.)
- [x] CLI with REPL, run, parse, lex, check commands
- [x] VS Code extension with syntax highlighting
- [x] Comprehensive test suite
- [x] File extension: `.tnt`
- [x] Algebraic Data Types with enums
- [x] Option<T> and Result<T, E> built-ins
- [x] Pattern matching with match expressions
- [x] Generic functions and types with trait bounds
- [x] Type aliases
- [x] Union types
- [x] Effect annotations foundation
- [x] Module system with imports/exports
- [x] Standard library: std/string, std/math, std/collections, std/env, std/fs, std/path, std/json, std/time, std/crypto, std/url, std/http, std/auth, std/kv, std/log
- [x] Traits with default implementations
- [x] For-in loops and ranges
- [x] Defer statement
- [x] Map literals with field access (dot notation)
- [x] String interpolation and raw strings
- [x] Template strings (`"""..."""` with `{{}}` interpolation, for loops, conditionals)
- [x] Template elif chains (`{{#elif cond}}`)
- [x] Template loop metadata (`@index`, `@first`, `@last`, `@length`, `@even`, `@odd`)
- [x] Template empty blocks (`{{#empty}}` fallback for empty loops)
- [x] Template comments (`{{! comment }}`)
- [x] Template filters (`{{expr | filter1 | filter2(arg)}}`)
- [x] Map iteration functions (`keys()`, `values()`, `entries()`, `has_key()`)
- [x] Nested map inference (nested maps don't require `map` keyword inside `map {}`)
- [x] Truthy/falsy values (0 is truthy, empty strings/arrays/maps are falsy)
- [x] CSV parsing (`std/csv`)
- [x] `ntnt test` command for HTTP endpoint testing
- [x] `ntnt docs` command for stdlib documentation search
- [x] `ntnt docs --generate` for auto-generating reference docs and AI agent guide sync
- [x] `ntnt completions <shell>` for shell completions (bash, zsh, fish)
- [x] Auto-generated documentation (STDLIB_REFERENCE.md, SYNTAX_REFERENCE.md, IAL_REFERENCE.md)
- [x] External templates with `template()` function (Mustache-style syntax)
- [x] Async HTTP server (Axum + Tokio) with bridge to sync interpreter
- [x] Hot-reload for file-based routes (routes/*.tnt) in async server
- [x] Hot-reload tracks imported files (lib modules, local imports)
- [x] Hot-reload for lib modules, middleware, and route directory changes
- [x] Template cache invalidation (mtime-based hot-reload for compiled templates)
- [x] NTNT_ENV=production disables hot-reload for better performance
- [x] Runtime documentation (RUNTIME_REFERENCE.md)
- [x] Documentation system: `// @ntnt` doc comments in Rust source, `build.rs` validation, 267 functions documented
- [x] 100% documentation coverage enforced at compile time (undocumented function = build error)

---

## Phase 6: Intent-Driven Development (IDD)

**Status:** Complete ✅

**Goal:** Make NTNT the first language with native Intent-Driven Development—where human intent becomes executable specification.

> See [docs/IAL_REFERENCE.md](docs/IAL_REFERENCE.md) for the Intent Assertion Language reference.

### What is IDD?

Intent-Driven Development creates a **contract layer between human requirements and AI-generated code**. Instead of describing what you want and hoping the AI understands, you write a `.intent` file that is both:

- **Human-readable requirements** - Plain English descriptions anyone can understand
- **Machine-executable tests** - Assertions the system verifies automatically

```yaml
# snowgauge.intent

## Glossary

| Term | Means |
|------|-------|
| a visitor goes to {path} | GET {path} |
| the home page | / |
| the page loads | status 200 |
| they see {text} | body contains {text} |

---

Feature: Site Selection
  id: feature.site_selection

  Scenario: Visitor sees available sites
    When a visitor goes to the home page
    → the page loads
    → they see "Bear Lake"
    → they see "Wild Basin"
```

### 6.1 POC Validation (Go/No-Go Checkpoint) ✅

**Goal:** Prove the concept works before full investment.

- [x] Intent file parser (YAML-based `.intent` files)
- [x] HTTP test runner (start server, make requests, check assertions)
- [x] Basic assertions (`status`, `body contains`, `body matches`)
- [x] `ntnt intent check` command
- [x] Apply to `snowgauge.tnt` example

```bash
# Target behavior
$ ntnt intent check snowgauge.tnt

Feature: Site Selection
  ✓ GET / returns status 200
  ✓ body contains "Bear Lake"
  ✓ body contains "Wild Basin"

2/2 features passing (5/5 assertions)
```

**Success criteria:** Use IDD to develop a new feature in snowgauge. Does it feel useful?

### 6.2 Core Intent Commands ✅

- [x] `ntnt intent check <file.tnt>` - Verify code matches intent
- [x] `ntnt intent init <file.intent>` - Generate code scaffolding from intent
- [x] `ntnt intent coverage <file.tnt>` - Show which features have implementations
- [ ] `ntnt intent diff <file.tnt>` - Gap analysis between intent and code

### 6.2.1 Intent Assertion Language (IAL) Engine ✅

**IAL is a term rewriting system** where natural language assertions are recursively resolved to executable primitives.

Architecture:

```
"they see success response"
    ↓ vocabulary lookup
"component.success_response"
    ↓ component expansion
["status 2xx", "body contains 'ok'"]
    ↓ standard term resolution
[Check(InRange, "response.status", 200-299), Check(Contains, "response.body", "ok")]
    ↓ execution
[✓, ✓]
```

**Core Implementation (src/ial/):**

- [x] `vocabulary.rs` - Pattern matching and term storage
- [x] `resolve.rs` - Recursive term → primitive resolution (~30 lines core logic)
- [x] `execute.rs` - Primitive execution against Context
- [x] `primitives.rs` - Primitive enum (Http, Check) + CheckOp enum
- [x] `standard.rs` - Standard vocabulary definitions

**Primitives (fixed set - new assertions are vocabulary, not code):**

- Actions: `Http`, `Cli`, `Sql`, `ReadFile`
- Checks: `Equals`, `NotEquals`, `Contains`, `NotContains`, `Matches`, `Exists`, `NotExists`, `LessThan`, `GreaterThan`, `InRange`

**High-level API:**

```rust
pub fn run_assertions(assertions: &[String], vocab: &Vocabulary, port: u16) -> IalResult<Vec<ExecuteResult>>
pub fn run_scenario(method: &str, path: &str, body: Option<&str>, assertions: &[String], vocab: &Vocabulary, port: u16) -> IalResult<(bool, Vec<ExecuteResult>)>
```

### 6.3 Code Annotations ✅

- [x] `// @implements: feature.X` comment parsing
- [x] `// @supports: constraint.Y` for supporting code
- [x] `// @utility`, `// @internal`, `// @infrastructure` markers
- [x] Link annotations to intent items
- [ ] Validate IDs exist in intent file

```ntnt
// @implements: feature.site_selection
fn home_handler(req) {
    // This function implements the site selection feature
}
```

### 6.4 Expanded Assertions (IAL Standard Vocabulary)

**HTTP Assertions (Implemented via IAL)**

- [x] Status code: `status: 200`, `status 2xx`, `status 4xx`
- [x] Body contains: `body contains "text"`, `they see "text"`
- [x] Body negation: `body not contains "error"`, `they don't see "text"`
- [x] Regex matching: `body matches r"pattern"`
- [x] Header assertions: `header "Content-Type" contains "text/html"`
- [x] JSON path: `body json "$.users[0].name" equals "Alice"`
- [x] Redirects: `redirects to /path`
- [x] Content-type: `returns JSON`, `returns HTML`
- [ ] Response timing: `responds in under {time}`

**CLI Assertions (IAL Vocabulary)**

- [x] Exit codes: `exits successfully`, `exits with code {n}`
- [x] Output: `output shows {text}`, `output matches {pattern}`
- [x] Errors: `error shows {text}`, `no error output`

**File Assertions (IAL Vocabulary)**

- [x] Existence: `file {path} exists`, `file {path} is created`
- [x] Content: `file {path} contains {text}`
- [x] Directories: `directory {path} exists`

**Database Assertions (IAL Vocabulary - Definitions ready)**

- [x] Row operations: `record is created`, `record is updated`, `record is deleted`
- [x] Queries: `row exists where {condition}`, `row count is {n}`
- [ ] Database verification: `verify_db:` with SQL queries (execution pending)
- [ ] State before/after comparison

### 6.5 Intent Studio

**Goal:** A collaborative workspace where humans and agents develop intent together.

The `.intent` format is optimized for machine parsing and testing, but humans deserve a better experience when creating and refining intent. Intent Studio provides a beautiful HTML view that makes intent development feel like a creative collaboration, not a chore.

**Phase 1: Basic Studio (MVP) ✅ COMPLETE**

- [x] `ntnt intent studio <file.intent>` - Start studio server
- [x] Rich HTML rendering with feature cards and visual hierarchy
- [x] Auto-refresh via polling (page refreshes every 2 seconds)
- [x] File watcher detects changes
- [x] Auto-open browser on launch (with `--no-open` flag to disable)
- [x] Beautiful dark theme with stats dashboard
- [x] Feature icons based on feature name/type
- [x] Error page with auto-retry when intent file has parse errors
- [x] **Live test execution** - tests run against a running app
- [x] **Pass/fail indicators** - visual ✓/✗ on every assertion
- [x] **Run Tests button** - re-execute tests on demand
- [x] **Default ports** - Studio on 3001, app on 8081
- [x] **Native hot-reload** - edit .tnt file, changes apply on next request (no restart!)
- [x] **Auto-start app** - Studio automatically starts the matching .tnt file

**Phase 2: Intent Studio V2** (Mostly Complete)

Design: [design-docs/studio-mockup-v2.html](design-docs/studio-mockup-v2.html)

- [x] Health bar visualization (pass/fail/warning/skip percentages)
- [x] Filter chips (All, Failing, Warnings, Skipped, Unlinked, Unit Tests)
- [x] Search across features, scenarios, and assertions
- [x] Expanded feature cards with scenarios and assertions
- [x] Unit test section with test data, corpus testing, property checks
- [x] Invariant bundles display
- [x] Warning states for not-implemented features
- [x] Skip states with precondition failure reasons
- [ ] WebSocket-based instant live reload (currently polling at 10s interval)

**Phase 3: IAL Explorer** ✅ COMPLETE

Design: [design-docs/ial_explorer.html](design-docs/ial_explorer.html)

- [x] Intent file viewer with syntax highlighting
- [x] Interactive glossary term highlighting
- [x] Hover popover showing full resolution chain
- [x] Resolution depth visualization (Level 0 → 1 → 2 → primitive)
- [x] Sidebar glossary reference panel
- [x] Link between Studio and Explorer views

**Phase 4: Enhanced Studio (Later)**

- [ ] Implementation status indicators (linked to `@implements` annotations)
- [ ] Diff highlighting when intent changes

```bash
# Start intent studio (default ports: studio on 3001, app on 8081)
$ ntnt intent studio server.intent

🎨 Intent Studio
  File: server.intent
  URL:  http://127.0.0.1:3001
  App:  http://127.0.0.1:8081
  ✅ Live test execution enabled!

# Custom ports if needed
$ ntnt intent studio server.intent --port 4000 --app-port 9000
```

**Workflow:** Human and AI collaborate on intent with live test feedback:

1. Create or open an existing `.intent` file (`ntnt intent init` or edit directly)
2. Start your app on port 8081 (or use `--app-port` for custom port)
3. Start the studio: `ntnt intent studio server.intent`
4. Human opens studio in browser (side-by-side with editor)
5. Tests run automatically—see which assertions pass ✓ or fail ✗
6. Human and AI collaborate—discussing, adding, removing, refining features
7. AI updates `.intent` file, studio refreshes and re-runs tests
8. Watch tests fail for new features, implement until they pass
9. All tests green = intent is verified!

### 6.6 Test Execution for All Program Types

- [x] HTTP servers (primary focus)
- [ ] CLI applications (`run:`, `exit_code:`, `stdout:`)
- [ ] Library functions (`eval:`, `result:`)
- [ ] Database operations (`verify_db:`, transactions)

### 6.7 Developer Experience

- [ ] `ntnt intent watch` - Continuous verification during development
- [x] Colored output (green/red for pass/fail)
- [x] Failure details with expected vs actual
- [ ] Intent file line numbers in error messages
- [ ] Parallel test execution

### 6.8 Intent History & Changelog

- [ ] `ntnt intent history <feature>` - View feature evolution
- [ ] `ntnt intent changelog v1 v2` - Generate release notes from intent diffs
- [ ] `ntnt intent archaeology "<term>"` - Search intent history
- [ ] Feature history timeline in Intent Studio
- [ ] Removed feature archive - browse features that were removed
- [ ] Shareable URLs for team review

### 6.9 Advanced Assertions & Behavioral Properties

**Behavioral Properties**

- [x] Idempotency: `property: idempotent` — verifies f(f(x)) == f(x)
- [x] Determinism: `property: deterministic` — verifies f(x) == f(x) across calls
- [x] Round-trip: `property: round_trips` — verifies g(f(x)) == x
- [ ] Purity: `pure: true` (same input = same output, no side effects — requires side-effect tracking)
- [ ] Thread safety: `parallel:` concurrent request testing
- [ ] Sequencing: `sequence:` state machine transitions
- [ ] No unintended mutations: `no_db_writes: true`

**Side Effect Verification**

- [ ] Email sent: `email_sent_to:`
- [ ] Event published: `event_published:`
- [ ] Log verification: `log_contains:`
- [ ] External call verification

**Contract Integration**

- [ ] `contracts:` section linking intent to code contracts
- [ ] Precondition violation testing
- [ ] Postcondition verification
- [ ] Invariant checking across test sequences

**Resource Constraints**

- [ ] Query count: `db_query_count <= N`
- [ ] Memory bounds: `memory_delta < X`
- [ ] Connection limits

### 6.10 Browser & Visual Testing (Future)

- [ ] DOM assertions (element exists, visible, attributes)
- [ ] Browser automation (click, fill, navigate)
- [ ] Visual regression (screenshot comparison)
- [ ] LLM visual verification for subjective qualities

**Phase 6 Deliverables:**

- `.intent` file format and parser
- `ntnt intent check|init|coverage|diff|watch|studio` commands
- `@implements` annotation system
- Test execution engine for HTTP servers
- Intent history and changelog generation
- Intent Studio with WebSocket hot-reload for collaborative intent development
- Applied to `snowgauge.tnt` and other examples

### 6.11 Modular Intent Files (Future)

- [ ] `@include` directive for importing features from other `.intent` files
- [ ] Scoped feature IDs to prevent collisions across modules
- [ ] Module-level constraints that apply to all included features
- [ ] Selective imports: `@include "auth.intent" only [feature.login, feature.logout]`

```intent
# Main application intent file
# Includes modules for large applications

@include "modules/auth.intent"
@include "modules/products.intent"
@include "modules/checkout.intent" only [feature.cart, feature.payment]

## Overview
Full e-commerce platform composed from reusable intent modules.

---

Constraint: Global Rate Limiting
  description: "All API endpoints are rate limited"
  applies_to: [auth.*, products.*, checkout.*]
```

> **Note:** For most applications, a single `.intent` file with `## Module:` section headers is recommended. The `@include` directive is for very large projects or organizations that need to share intent modules across multiple applications.

---

## Phase 7: Language Ergonomics & Documentation ← UP NEXT

**Status:** In Progress

**Goal:** Address the biggest daily friction points for AI agents and human developers writing NTNT code, and establish the documentation systems that will serve the language long-term. The type system comes first because it's a foundation that makes every subsequent feature stronger.

> These features were identified through real-world usage as the highest-impact improvements to the language. The type system is sequenced first because error propagation (`?`) needs to know return types, closures benefit from type inference, and SQLite needs type mapping. Together, these features transform a typical web handler from ~22 lines of match pyramids to ~6 lines of linear, readable code.

### 7.1 Type System Enforcement

**Priority:** Foundation — everything else in this phase builds on real types.

Currently, type annotations are parsed but not enforced. This is the worst of both worlds: syntax noise without safety guarantees. NTNT needs to commit to real types.

**Design: Enforced types with aggressive inference.**

```ntnt
// Function signatures require types (the contract boundary)
fn add(a: Int, b: Int) -> Int {
    return a + b
}

// Local variables are inferred — no annotation needed
let x = 5              // inferred: Int
let name = "Alice"     // inferred: String
let nums = [1, 2, 3]  // inferred: [Int]

// Explicit annotation optional, useful for documentation
let threshold: Float = 3.14

// Type errors caught at lint/validate time, not runtime
fn greet(name: String) -> String {
    return "Hello, " + name
}
greet(42)  // ✗ Type error: expected String, got Int
```

**Two layers of safety — types + contracts:**

```ntnt
// Types catch STRUCTURAL errors (wrong kind of data)
// Contracts catch SEMANTIC errors (right kind, wrong value)
fn divide(a: Int, b: Int) -> Int
    requires b != 0                    // contract: semantic check
    ensures result * b == a            // contract: behavioral check
{
    return a / b    // types guarantee a and b are Int
}

// The contract checker can now verify:
// "ensures result > 0" — result is Int, comparison to Int is valid ✓
// "ensures len(result)" — result is Int, len() expects String/Array ✗
```

**Implementation plan:**

- [x] Type inference engine for local variables (`let x = 5` → `Int`)
- [x] Type checking at function call boundaries (argument types match parameter types)
- [x] Return type verification (function body returns declared type)
- [x] Type inference for expressions (arithmetic, string concat, comparisons)
- [x] Generic type resolution (`Option<Int>`, `Result<String, Error>`)
- [x] Union type checking (`String | Int` accepts either)
- [x] Array element type inference (`[1, 2, 3]` → `[Int]`)
- [x] Map value type inference (`map { "a": 1 }` → `Map<String, Int>`)
- [x] Type errors in `ntnt lint` and `ntnt validate` output (not just runtime)
- [x] Helpful error messages: "expected String, got Int" with line/column
- [x] Contract expression type-checking (`ensures len(result) > 0` — verify `result` is a type `len()` accepts)
- [x] Gradual typing: untyped parameters default to `Any` (backward compatible)
- [x] Remove the `Effect` enum from `src/types.rs` (7 variants, never checked at runtime)
- [x] Remove `with` keyword effect parsing from `src/parser.rs` (lines 244-258)
- [x] Remove `pure` keyword parsing from function signatures
- [x] Remove `TypeExpr::WithEffect` variant from AST
- [x] Remove the two effect-related tests from `src/interpreter.rs` (test_effects_annotation, test_pure_function)
- [x] Keep `TokenKind::With` and `TokenKind::Pure` as reserved keywords (forward compatibility)
- [x] Builtin and stdlib function signature registry (~180 functions across 16 modules)
- [x] Two-pass type checking (collect declarations, then check — enables forward references)
- [x] Struct field type checking
- [x] Option/Result pattern binding in match arms

- [x] `ntnt lint --strict` warns about untyped function parameters and missing return types
- [x] Strict lint mode activated by CLI flag, `NTNT_STRICT=1` env var, or `ntnt.toml` config
- [x] Project config file (`ntnt.toml`) with `[lint] strict = true` support
- [x] `Array<T>` and `Map<K, V>` generic annotation resolution (type compatibility with inferred types)
- [x] Built-in `Request` and `Response` struct types for HTTP handlers (field access returns real types)
- [x] Generic-aware `unwrap()`: `unwrap(Optional<T>)` → `T`, `unwrap(Result<T, E>)` → `T`
- [x] `filter()` preserves array element type: `filter(Array<T>, pred)` → `Array<T>`
- [x] Fixed missing stdlib signatures: `Cache`/`cache_fetch` (std/http), `parse_csv` (std/csv), `parse_datetime` (std/time)
- [x] `html()`/`json()`/`text()`/`redirect()`/`status()` return `Response` instead of `Any`
- [x] Collection functions preserve element types: `sort()`, `reverse()`, `slice()`, `concat()` return `Array<T>`
- [x] `flatten()` unwraps one nesting level: `Array<Array<T>>` → `Array<T>`
- [x] `push()` preserves array type: `push(Array<T>, T)` → `Array<T>`
- [x] `first()`, `last()`, `pop()` return element type `T` from `Array<T>`
- [x] Math builtins preserve numeric types: `abs(Int)` → `Int`, `abs(Float)` → `Float`
- [x] `min()`/`max()` with float promotion: `min(Int, Float)` → `Float`
- [x] `clamp()` preserves numeric type of first argument
- [x] `keys(Map<K,V>)` → `Array<K>`, `values(Map<K,V>)` → `Array<V>` (type-aware map accessors)
- [x] `get_key(Map<K,V>, key)` → `V` (returns map value type instead of `Any`)
- [x] `entries(Map<K,V>)` → `Array<Array<Any>>` (structured return)
- [x] `transform()`/`.map()` callback return type inference: `transform(Array<T>, fn(T)->R)` → `Array<R>`
- [x] `parse_json()` returns `Result<Map<String, Any>, String>` instead of `Any`
- [x] `fetch()` returns `Result<Response, String>` instead of `Any`
- [x] Match arm struct pattern narrowing: fields bind with struct field types
- [x] Cross-file import type propagation: `import { foo } from "./lib/utils"` resolves function signatures
- [x] Union type soundness: union VALUES require ALL members compatible with target, union TARGETS require ANY member match
- [x] Union type flattening and deduplication in `union_type()` computation
- [x] Block divergence analysis (`block_diverges`) for otherwise/lambda validation
- [x] `otherwise` block divergence enforcement (lint error, not just warning — must end with return/break/continue)
- [x] Lambda return type inference via `collected_returns` (early returns included in union)
- [x] Flow-sensitive type narrowing after guards (`if x == None { return }` narrows `x` to inner type)
- [x] Narrowing patterns: `x == None`, `x != None`, `is_some(x)`, `is_none(x)`, `is_ok(x)`, `is_err(x)`, `!cond`
- [x] Guard clause narrowing: diverging then-branch applies false-facts to subsequent code
- [x] If-expression narrowing: branches get narrowed types from condition
- [x] Static match exhaustiveness checking for `Option`, `Result`, and user-defined enums
- [x] Map field access returns value type: `map.field` on `Map<K, V>` returns `V`
- [x] Variadic functions check declared parameter types (not just skip all checking)
- [x] Variadic functions check minimum argument count
- [x] Cross-file Pass 2 inference: unannotated exported functions get inferred return types
- [x] Circular import safety: shared module cache prevents infinite recursion during cross-file analysis
- [x] Circular import diagnostics: cycle chain shown in warnings (e.g. `a.tnt → b.tnt → a.tnt`)
- [x] Strict mode: string interpolation warning for complex types (Array, Map, Function)
- [x] Strict mode: Float→Int precision loss warning
- [ ] Cross-file struct/enum propagation (extend import type resolution to include struct and enum definitions)
- [ ] Closure parameter type inference from call context (7.3 closures done; inference from `Array<T>` context remaining)
- [ ] Heterogeneous map return types: functions returning `map { "name": str, "values": arr }` need user-defined structs for real types

**Backward compatibility:** Existing NTNT code continues to work. Untyped function parameters are treated as `Any`. Adding types is opt-in but encouraged. `ntnt lint --strict` warns about untyped public function signatures.

### 7.2 Error Propagation (`?` Operator)

**Priority:** Highest friction point — depends on 7.1 to verify return types are `Result`/`Option`.

Currently, every fallible operation requires an explicit `match`:

```ntnt
// Current: verbose error handling
fn handle_request(req) {
    match parse_json(req) {
        Ok(data) => {
            match validate(data) {
                Ok(valid) => {
                    match save_to_db(valid) {
                        Ok(result) => return json(result),
                        Err(e) => return status(500, "DB error: " + str(e))
                    }
                },
                Err(e) => return status(400, "Invalid: " + str(e))
            }
        },
        Err(e) => return status(400, "Parse error: " + str(e))
    }
}
```

With `?` operator:

```ntnt
// Target: concise error propagation
fn handle_request(req: Request) -> Result<Response, Error> {
    let data = parse_json(req)?
    let valid = validate(data)?
    let result = save_to_db(valid)?
    return Ok(json(result))
}
```

**Implementation plan:**

- [x] `?` operator on `Result<T, E>` values — unwrap `Ok` or early-return `Err`
- [x] `?` operator on `Option<T>` values — unwrap `Some` or early-return `None`
- [x] Type checker unwraps `Result<T, E>` → `T` and `Option<T>` → `T` for `?` expressions
- [x] Gradual typing: non-Result/Option values pass through unchanged
- [ ] Error type coercion (auto-convert between compatible error types)
- [ ] Type system verifies `?` is used in functions that return `Result` or `Option`
- [ ] Clear error message when `?` is used in a function with wrong return type

### 7.2.1 Inline Error Handling (`otherwise`)

> **Design Doc:** [plans/syntax-analysis-and-innovation.md](plans/syntax-analysis-and-innovation.md) § 3.3

**Priority:** High — can ship independently of `?` (no type system dependencies). Immediate ergonomic win for both web handlers and CLI scripts.

The `?` operator propagates errors to the caller. `otherwise` handles errors at the call site — each failure gets its own recovery logic inline. They are complementary: `?` for library internals, `otherwise` for handlers and scripts.

```ntnt
// Web handler — each error gets a specific HTTP response
fn create_user(req) {
    let data = parse_json(req)
        otherwise return status(400, "Invalid JSON: {err}")

    let name = data["name"]
        otherwise return status(400, "Missing field: name")

    let saved = execute(db, "INSERT INTO users (name) VALUES ($1)", [name])
        otherwise return status(500, "Database error: {err}")

    return json(map { "created": true, "name": name }, 201)
}

// CLI script — handle errors, skip bad data, keep going
let content = read_file("data/input.csv")
    otherwise { print("Cannot read input: {err}"); return }

for line in lines {
    let value = float(fields[2])
        otherwise { print("Bad number on line: {line}"); continue }
    // ...
}
```

**Semantics:**
- Works on `Result<T, E>`: unwraps `Ok(v)` into the binding, runs the otherwise block on `Err(e)`
- Works on `Option<T>`: unwraps `Some(v)` into the binding, runs the otherwise block on `None`
- `err` is automatically bound to the error value (or `None` for Option) inside the otherwise block
- The otherwise block must diverge: `return`, `continue`, `break`, or panic
- Single-expression form: `otherwise return status(400, "msg")` (no braces needed)
- Block form: `otherwise { print("error"); return }` (braces for multiple statements)

**Implementation plan:**

- [x] `Otherwise` keyword in lexer
- [x] Parse `otherwise` clause on `let` bindings: `let x = expr otherwise { diverging }`
- [x] `err` auto-binding for the error/None value
- [x] Single-expression shorthand (no braces for simple cases)
- [x] Works with `Result<T, E>` — unwrap Ok or run otherwise on Err
- [x] Works with `Option<T>` — unwrap Some or run otherwise on None
- [x] Verify otherwise block diverges (return/break/continue)
- [x] Type checker unwraps `Result<T, E>` → `T` and `Option<T>` → `T` for let with otherwise
- [x] Lint integration: error if otherwise block doesn't diverge

### 7.3 Anonymous Functions / Closures ✅

**Status:** Complete (v0.4.0)

Anonymous functions use `fn(params) { body }` syntax in expression position:

```ntnt
// Inline closures with filter and transform
let doubled = transform(nums, fn(x) { x * 2 })
let evens = filter(nums, fn(x) { x % 2 == 0 })

// Typed params and return type
let multiply = fn(a: Int, b: Int) -> Int { a * b }

// Multi-statement body
let process = fn(item) {
    let cleaned = trim(item)
    return to_lower(cleaned)
}

// Closures capture surrounding variables
let threshold = 10
let above = filter(nums, fn(x) { x > threshold })

// Nested closures (currying)
let make_adder = fn(x) { fn(y) { x + y } }

// Immediate invocation
let result = fn(x) { x + 1 }(5)
```

**Implementation:**

- [x] Anonymous function expression: `fn(params) { body }`
- [x] Block body with implicit return (last expression is return value)
- [x] Multi-statement closures with explicit `return`
- [x] Optional type annotations: `fn(x: Int) -> Int { ... }`
- [x] Variable capture from enclosing scope (closure semantics)
- [x] Closures as function arguments (higher-order functions)
- [x] Nested closures and immediate invocation
- [x] Lint, type checker, and interpreter support
- [ ] Type inference from call context (parameter types inferred from expected signature)

### 7.4 SQLite Support (`std/db/sqlite`)

**Priority:** High — the most common database for small web apps, requires no external server.

SQLite is the natural database for NTNT's sweet spot: AI-generated web prototypes and small applications. Unlike PostgreSQL, it requires zero setup — just a file path.

```ntnt
import { connect, query, execute } from "std/db/sqlite"

let db = connect("app.db")

// Create tables
execute(db, "CREATE TABLE IF NOT EXISTS users (id INTEGER PRIMARY KEY, name TEXT, email TEXT)")

// Parameterized queries (safe from injection)
execute(db, "INSERT INTO users (name, email) VALUES (?, ?)", ["Alice", "alice@example.com"])

// Query returns array of maps
let users = query(db, "SELECT * FROM users WHERE name = ?", ["Alice"])
for user in users {
    print("User: " + user["name"])
}
```

**Implementation plan:**

- [x] `std/db/sqlite` module (Rust `rusqlite` crate with `bundled` feature)
- [x] `connect(path)` — open or create SQLite database file
- [x] `query(db, sql, params?)` — parameterized SELECT queries
- [x] `execute(db, sql, params?)` — INSERT/UPDATE/DELETE with param binding
- [x] `close(db)` — close database connection
- [x] Transaction support (`begin`, `commit`, `rollback`)
- [x] In-memory databases: `connect(":memory:")`
- [x] Type mapping: INTEGER↔Int, REAL↔Float, TEXT↔String, BLOB↔Array<Int>, NULL↔None

### 7.5 Pipe Operator (`|>`)

**Priority:** Moderate — low implementation effort, eliminates daily friction with nested function calls.

> Moved from Future Considerations. The assessment identified nested function calls as a concrete pain point: "the nested function call style gets ugly." With closures (7.3) now in the same phase, pipelines become especially powerful.

```ntnt
// Current: nested, reads inside-out
let result = join(split(trim(to_lower(input)), " "), "-")

// With pipe operator: linear, reads left-to-right
let result = input
    |> to_lower
    |> trim
    |> split(" ")
    |> join("-")

// Powerful with closures (7.3):
let active_emails = users
    |> filter(fn(u) { u.active })
    |> transform(fn(u) { u.email })
    |> join(", ")
```

**Implementation plan:**

- [x] `|>` operator in lexer (`PipeArrow` token) and parser (desugars to `Expression::Call` at parse time — no new AST node needed)
- [x] `x |> f` desugars to `f(x)` — simple rewrite
- [x] `x |> f(a, b)` desugars to `f(x, a, b)` — first-argument insertion
- [ ] Type checking flows through the pipe chain (output type of left = input type of right)
- [ ] Error messages show the failing step in the pipeline, not just the final result

### 7.6 Better Error Messages

**Priority:** High — the assessment rated NTNT's error messages as "Basic" and "notably behind languages like Rust or Elm." For an AI-first language, rich errors are essential for self-correction loops.

Currently, errors are terse:

```
Undefined variable: usr
Type error: expected Int, got String
```

Target: context-rich, actionable errors:

```
Error[E001]: Undefined variable `usr`
  --> server.tnt:45:12
   |
45 |     return json(usr)
   |                 ^^^ not found in this scope
   |
   help: did you mean `user`? (defined at line 42)
```

```
Error[E012]: Type mismatch in function call
  --> server.tnt:23:18
   |
23 |     let result = add("hello", 5)
   |                      ^^^^^^^ expected Int, got String
   |
   note: function `add` defined at server.tnt:10
         fn add(a: Int, b: Int) -> Int
```

**Implementation plan:**

- [x] Error codes (E001-E012) on all error variants for machine-parseable errors
- [x] "Did you mean?" suggestions for undefined variables (Levenshtein distance)
- [x] "Did you mean?" suggestions for undefined functions (Levenshtein distance)
- [x] Function name included in arity mismatch errors
- [x] Column info added to ParserError
- [x] Source code snippets in error output (3-line context for parser errors)
- [x] Color-coded CLI output (error codes in red, line numbers in blue, suggestions in green, help text in cyan)
- [x] Type checker forward-scanning cursor (`find_line_near`) — fixes line number accuracy for all 26 diagnostic sites without AST changes
- [x] Expression-aware search hints (`expr_search_hint`) — uses AST structure to build better search needles for line lookup
- [x] Actionable type hints for conditions (Int → "use != 0"), comparisons (Int vs String → "use int()/str()"), and let bindings (untyped map access)
- [x] AST location tracking (`Statement::Located { line, col, stmt }`) — parser wraps every statement with source position, interpreter tracks `current_line`/`current_col`, runtime errors annotated with line numbers
- [x] Runtime error line numbers via AST location tracking
- [x] "Did you mean?" suggestions for wrong imports — both wrong module names and wrong export names get Levenshtein suggestions with available exports list
- [x] Contract violation messages show the contract expression (`"Precondition failed in 'fn': b != 0"`)
- [ ] Contract violation messages show actual values (e.g., "b was 0") — expression is shown but runtime values are not
- [x] `ntnt lint --fix` outputs JSON patch suggestions for auto-fixes
- [ ] `ntnt lint --format=json` explicit structured output mode for full agent consumption

### 7.7 Route Pattern Auto-Detection

**Priority:** Moderate — eliminates the most common "gotcha" in NTNT web development.

Currently, route parameters use `{id}` syntax, which collides with string interpolation. Forgetting to use raw strings (`r""`) is a recurring friction point:

```ntnt
// Current: must remember r"" for every route
get(r"/users/{id}", get_user)        // ✅ works
get("/users/{id}", get_user)         // ❌ tries to interpolate {id}

// Target: route functions auto-detect patterns, no r"" needed
get("/users/{id}", get_user)         // ✅ just works
post("/api/orders/{id}/items", add_item)  // ✅ just works
```

Route registration functions (`get`, `post`, `put`, `patch`, `delete`) are already builtins — the compiler knows what they are. Their first argument should automatically suppress string interpolation and treat `{...}` as route parameter placeholders.

**Implementation plan:**

- [x] Route builtin functions treat their path argument as a route pattern (no interpolation)
- [x] `{param}` in route patterns is always a route parameter, never interpolation
- [x] Raw strings (`r""`) still work for backward compatibility
- [ ] `ntnt lint` warns if a raw string is used unnecessarily in a route (style hint)
- [x] Dynamic route registration (rare) uses an explicit API if needed

### 7.8 Destructuring Assignment

**Priority:** High — every POST handler parses form data or JSON into individual variables. This is the most repetitive boilerplate in NTNT web code.

```ntnt
// Current: 4 lines to extract fields
let form = parse_form(req)
let name = form["name"]
let email = form["email"]
let age = form["age"]

// With destructuring: 1 line
let { name, email, age } = parse_form(req)

// Works with type annotations
let { name: String, email: String, age: Int } = parse_form(req)

// Nested destructuring
let { user: { name, email }, role } = parse_json(req)?

// Array destructuring
let [first, second, ...rest] = split(line, ",")

// In function parameters
fn create_user({ name, email }: Map) -> User {
    return User { name: name, email: email }
}
```

**Implementation plan:**

- [x] Map destructuring in `let` bindings: `let { key1, key2 } = expr`
- [x] Array destructuring: `let [first, second] = expr`
- [x] Rest patterns: `let [head, ...tail] = arr`, `let { name, ...other } = map`
- [x] Nested destructuring: `let { user: { name } } = data`
- [ ] Destructuring with type annotations
- [x] Destructuring in function parameters
- [x] Destructuring in `for` loops: `for { name, email } in users { ... }`
- [x] Type checking: destructured fields are type-inferred from the source expression
- [x] Map destructuring with rename: `let { name: n } = data`
- [x] Map destructuring works with structs: `let { name } = user`
- [x] Map destructuring in `match` expressions

### 7.9 Default Parameter Values ✅

**Priority:** Moderate — reduces boilerplate in utility functions and makes APIs more ergonomic.

```ntnt
// Current: caller must always pass all arguments
fn paginate(items, page, per_page) {
    let start = (page - 1) * per_page
    return slice(items, start, start + per_page)
}
paginate(users, 1, 25)  // almost always 1 and 25

// With defaults: optional arguments have sensible fallbacks
fn paginate(items: [Any], page: Int = 1, per_page: Int = 25) -> [Any] {
    let start = (page - 1) * per_page
    return slice(items, start, start + per_page)
}
paginate(users)           // page=1, per_page=25
paginate(users, 3)        // page=3, per_page=25
paginate(users, 2, 50)    // page=2, per_page=50

// Works with web handler helpers
fn respond(data: Map, status_code: Int = 200, content_type: String = "application/json") -> Response {
    return status(status_code, stringify(data))
}
```

**Implementation plan:**

- [x] Default value expressions in function parameter lists: `param: Type = expr`
- [x] Default parameters must come after required parameters (parser error if violated)
- [x] Default expressions evaluated at call time (not definition time)
- [x] Type inference: default value provides type if annotation is missing
- [x] `ntnt inspect` includes default values in function signatures (`has_default: true`)
- [x] Works with contracts: `requires` can reference defaulted parameters

### ~~7.10 Guard Clauses (`let-else`)~~ — Removed

Superseded by `otherwise` (7.2.1), which provides the same unwrap-or-diverge pattern with better ergonomics: `err` auto-binding, readable keyword, and works with both `Result`/`Option` and non-Result values.

### 7.11 Intent File Cleanup

**Priority:** Low — small hygiene task, same spirit as the Effect enum removal.

- [ ] Remove unused `Meta:` section parsing from intent files (the `## Overview` section serves the same purpose)
- [ ] Clean up any other dead parsing paths identified during Phase 7 work

### 7.12 NTNT Language Documentation (Rust Source → Reference Docs)

> **Design Doc:** [plans/documentation_system_design.md](plans/documentation_system_design.md)
>
> Make the Rust source code the single source of truth for all NTNT language documentation. Replace disconnected TOML files with structured `/// @ntnt` doc comments placed directly above implementation code. `build.rs` validates 100% coverage at compile time. Documentation is embedded in the binary — `ntnt docs` works anywhere with zero setup.

**Core Principles:**
1. **Impossible to go stale** — `build.rs` cross-references doc comments against implementations; undocumented elements fail the build
2. **AI-native** — Structured data (JSON, embedded) for queries, not just markdown
3. **Self-validating** — Every example executes and passes, or CI fails
4. **Multi-level** — L0 (signature) → L4 (gotchas/patterns) from one source
5. **Embedded in binary** — Like Elixir's bytecode docs; no external files or path configuration needed

**Current (TOML-based) — To Be Replaced:**
- [x] TOML files as documentation source (stdlib.toml, syntax.toml, etc.)
- [x] `ntnt docs --generate` generates markdown from TOML
- [x] `ntnt docs [query]` searches stdlib
- [x] Pre-commit regeneration

#### Phase 1: `build.rs` Scanner + Proof of Concept ✅

**Goal:** Replace TOML with structured doc comments that live above implementation code.

```rust
// @ntnt split
// @module std/string
// @signature split(s: String, delim: String) -> Array<String>
// Splits a string into an array of substrings.
//
// When the delimiter is not found, returns a single-element array
// containing the original string.
// @see_also join, trim, replace
// @since v0.1.0
// @example split("a,b,c", ",") => ["a", "b", "c"] ~ "Basic comma-separated split"
// @example split("no-match", ",") => ["no-match"] ~ "No delimiter found"
module.insert("split".to_string(), Value::NativeFunction {
    name: "split".to_string(),
    func: |args| { /* existing implementation — unchanged */ },
});
```

- [x] Write `build.rs` source scanner — parse `// @ntnt` blocks, extract fields
- [x] Add doc comments to `std/string` module (24 functions) as proof of concept
- [x] Validate coverage: scanner detects undocumented `NativeFunction` inserts
- [x] Generate `doc_data.json` embedded in binary via `include_str!()`
- [x] Auto-discover source files (no hardcoded list — directory scanning)
- [x] Bidirectional validation: undocumented functions AND orphaned doc blocks fail the build

#### Phase 2: Complete Source Documentation ✅

- [x] Add doc comments to all 16 stdlib modules (string, math, collections, http, fs, json, csv, url, path, time, crypto, env, postgres, sqlite, concurrent, http_server)
- [x] Document global builtins (len, print, str, int, float, type, assert, etc.)
- [x] Document all 267 functions with full structured metadata (@description, @param, @returns, @example, @see_also, @since, @tags, @error, @gotcha)
- [x] 100% documentation coverage enforced at compile time
- [x] Delete TOML documentation files (replaced by source-embedded docs)

#### Phase 3: Enhanced CLI + Embedded Docs

```bash
# Querying (works anywhere — docs are in the binary)
ntnt docs split                       # Full docs for a function
ntnt docs std/string                  # All functions in a module
ntnt docs --examples split            # Just the examples
ntnt docs --search "convert string"   # Full-text search
ntnt docs --related split             # Cross-references
ntnt docs --json split                # JSON output for tooling
ntnt docs --ai-context                # Full dump for AI session start

# Validation
ntnt docs --coverage                  # Documentation completeness report
ntnt docs --test                      # Execute all examples (doctests)
ntnt docs --orphans                   # Docs without implementation
ntnt docs --diff v0.3.7               # What changed since a version

# Generation (publishing — on demand)
ntnt docs --generate                  # Markdown + JSON output
ntnt docs --update-agent-docs         # Regenerate auto-sections in AI_AGENT_GUIDE.md
```

- [x] `ntnt docs [query]` — search and display function docs from embedded data (full-text search across all modules)
- [x] `ntnt docs std/module` — list all functions in a module with signatures
- [x] `ntnt docs --generate` — generate markdown reference docs + AI agent guide sync
- [x] `ntnt docs --json` — JSON output for tooling (full structured data including examples, params, see_also)
- [x] `ntnt docs --validate` — documentation coverage report (also enforced at compile time via build.rs)
- [x] REPL integration: `:doc` command
- [ ] `ntnt docs --examples` — show just examples for a function (data available in --json, needs dedicated flag)
- [ ] `ntnt docs --related` — cross-reference via `@see_also` (data in --json, needs dedicated flag)
- [ ] `ntnt docs --ai-context` — full dump for AI session start
- [ ] `ntnt docs --test` — execute all doc examples (doctests, see below)
- [ ] `ntnt docs --orphans` — detect orphaned doc blocks (build.rs catches at compile time, needs CLI)
- [ ] `ntnt docs --diff` — version diffing between releases

#### Phase 3.5: Doctests (Execute Documentation Examples)

> **Design Doc:** [plans/doctest_design.md](plans/doctest_design.md)
>
> Run the 329 `@example` directives as automated tests during `cargo test`. Eval both the example code and expected value as NTNT expressions, compare structurally. ~260 examples in pure modules are testable; I/O modules are skipped. Inspired by Elixir's doctests.

- [ ] Expose embedded doc JSON from `lib.rs` for integration test access
- [ ] Add `Interpreter::import_module_all()` for programmatic wildcard imports
- [ ] Write `tests/doctest_tests.rs` integration test (~150-200 lines)
- [ ] Implement `values_equal()` recursive structural comparison (no `PartialEq` on Value)
- [ ] Fix any doc examples that fail (the whole point — catch documentation bugs)
- [ ] Wire into `ntnt docs --test` CLI command

#### Phase 4: AI-Native Features + Error Integration

- [ ] Semantic concept index from module membership and `@see_also` relationships
- [ ] Gotchas per function (non-obvious behaviors)
- [ ] Rich error messages with doc links and suggested fixes
- [ ] `ntnt docs --ai-context` for efficient AI session start

**Success Criteria:**

| Metric | Target |
|--------|--------|
| TOML files remaining | 0 |
| Documentation coverage | 100% (build fails otherwise) |
| Example pass rate | 100% (CI fails otherwise) |
| Files edited to add a function | 1 (the Rust source file) |
| AI query accuracy | 95%+ from structured JSON |
| Binary size increase | < 500 KB (~2-5%) |
| `ntnt docs` works without external files | Yes — embedded in binary |

### 7.13 Import Error Quality — Collision Warnings + "Did You Mean?"

**Priority:** Moderate — better error messages for the most common language operation.

> **Design Doc:** [plans/bare_imports_design.md](plans/bare_imports_design.md)
>
> Add collision warnings in lint when the same name is imported from two modules, and wire "Did you mean?" suggestions into import error paths for both wrong module names and wrong export names.

**Implementation plan:**

- [ ] Import collision warnings in `ntnt lint` (detect when same name imported from two modules, suggest aliases)
- [ ] "Did you mean?" suggestions for wrong module names (wire existing Levenshtein into `import_std_module()`)
- [ ] "Did you mean?" suggestions for wrong export names (wire into `bind_imports()`, show available exports)

### 7.14 If-Expressions (Ternary / Conditional Expressions)

**Priority:** Moderate — the AST and interpreter already support this; only the parser needs updating.

Currently, NTNT has no way to use `if/else` as an expression that returns a value. This forces unnecessary mutability:

```ntnt
// Current: must use mut
let mut label = ""
if count > 0 {
    label = "active"
} else {
    label = "inactive"
}

// Target: if-expression returns a value
let label = if count > 0 { "active" } else { "inactive" }

// Works in any expression position
return json(map { "status": if ok { "success" } else { "error" } })
```

**Implementation status:** ✅ Complete (v0.3.10)

- [x] `Expression::IfExpr` in AST (`src/ast.rs`)
- [x] `IfExpr` evaluation in interpreter (`src/interpreter.rs`)
- [x] Parse `if` in expression position in `primary()` (`src/parser.rs`)
- [x] Type inference: both branches return union type (`src/typechecker.rs`)
- [x] Require `else` branch (no dangling if-expressions)
- [x] Else-if chains via recursive `primary()` parsing
- [x] Integration tests (8 tests covering all patterns)
- [x] Snowgauge examples updated to use if-expressions

### 7.15 Regex Capture Groups (`capture_pattern`)

**Priority:** Moderate — the existing regex functions (`find_pattern`, `matches_pattern`, etc.) only work with the full match. Capture groups are essential for structured text extraction.

```ntnt
// Current: find_pattern returns only the full match, not groups
let match = find_pattern("Bear Lake (1042)", r"([^()]+)\s*\((\d+)\)")
// match = Some("Bear Lake (1042)") — no access to capture groups

// Target: capture_pattern returns an array of captured groups
let groups = capture_pattern("Bear Lake (1042)", r"([^()]+)\s*\((\d+)\)")
// groups = Some(["Bear Lake (1042)", "Bear Lake ", "1042"])
//           ^full match             ^group 1      ^group 2

// Named capture groups (stretch goal)
let groups = capture_pattern(line, r"(?P<name>[^()]+)\s*\((?P<id>\d+)\)")
// groups = Some(map { "0": "Bear Lake (1042)", "name": "Bear Lake ", "id": "1042" })
```

**Why this matters:** PHP's `preg_match` with capture groups is one of the places where PHP is more concise than NTNT for text extraction. The snowgauge example's `extract_snotel_name` function could be reduced from 6 lines to 2 with capture groups.

**Implementation plan:**

- [x] `capture_pattern(s: String, pattern: String) -> Option<Array<String>>` — returns all capture groups (index 0 = full match, 1+ = groups)
- [x] `capture_all_pattern(s: String, pattern: String) -> Array<Array<String>>` — all matches with their capture groups
- [x] `capture_named_pattern(s: String, pattern: String) -> Option<Map<String, String>>` — named capture groups as map keys
- [x] Add `// @ntnt` doc blocks and update STDLIB_REFERENCE.md

### 7.16 None/null JSON Serialization ✅

**Priority:** Low — edge case, but affects data interchange.

`None` serializes as JSON `null` in `stringify()`, and `parse_json("null")` returns `None`. Consistent NULL→None mapping across all modules (json, sqlite, postgres, http_server). Previously:

```ntnt
// Current: None values have no JSON representation
let data = map { "name": "Alice", "phone": None }
stringify(data)  // behavior undefined — may skip the key or error

// Target: None serializes to null
stringify(data)  // {"name":"Alice","phone":null}
```

**Implementation plan:**

- [x] `Value::None` serializes as `null` in `stringify()`
- [x] `parse_json()` maps JSON `null` to `None`
- [x] Round-trip: `parse_json(stringify(map { "x": None }))` preserves `None`
- [x] Consistent NULL→None across all modules: `std/json`, `std/db/sqlite`, `std/db/postgres`, `std/http/server`

### 7.17 Web Application Essentials ✅

**Priority:** High — these are the last-mile features blocking real web application development.

Added 17 stdlib functions across 3 modules plus 1 global builtin to enable building production web applications:

**Password Hashing (`std/crypto`):**
- [x] `hash_password(password, cost?)` — bcrypt hashing with configurable cost (default 12)
- [x] `verify_password(password, hash)` — verify password against bcrypt hash
- [x] `is_valid_hash(hash)` — check if string is a valid bcrypt hash

**Cookie Management (`std/http/server`):**
- [x] `set_cookie(name, value, options?)` — build Set-Cookie header string
- [x] `get_cookie(req, name)` — get single cookie from request
- [x] `get_cookies(req)` — get all cookies as map
- [x] `delete_cookie(name, options?)` — build cookie deletion header
- [x] `with_cookie(resp, name, value, options?)` — add cookie to response
- [x] Multi-value header support (arrays emit multiple headers with same name)

**Structured Logging (`std/log` — new module):**
- [x] `log_debug(message, data?)` — debug level logging
- [x] `log_info(message, data?)` — info level logging
- [x] `log_warn(message, data?)` — warning level logging
- [x] `log_error(message, data?)` — error level logging
- [x] `set_log_level(level)` — set global log level
- [x] `request_logger()` — middleware function for request logging

**CORS (global builtin):**
- [x] `enable_cors(options?)` — configure CORS with origins, methods, headers, credentials
- [x] Automatic OPTIONS preflight handling
- [x] CORS headers applied to all responses

**File Uploads (`std/http/server`):**
- [x] `parse_multipart(req)` — parse multipart/form-data requests
- [x] `save_upload(file_field, path)` — save uploaded file to disk

### 7.18 Security Hardening ✅

**Goal:** Make NTNT inherently secure by default — no configuration required for safe defaults.

**Environment Variables:**

| Variable | Default | Description |
|----------|---------|-------------|
| `NTNT_MAX_BODY_SIZE` | `10MB` | Maximum request body size (supports KB/MB/GB suffixes) |
| `NTNT_SECURITY_HEADERS` | `true` | Add security headers to all responses |
| `NTNT_DETAILED_ERRORS` | dev: `true`, prod: `false` | Show detailed error messages |
| `NTNT_SSRF_PROTECTION` | `true` | Block requests to private IPs and cloud metadata |
| `NTNT_ALLOW_LOCALHOST` | dev: `true`, prod: `false` | Allow fetch() to localhost |
| `NTNT_ALLOW_PRIVATE_IPS` | `false` | Allow `fetch()` to private IP ranges |
| `NTNT_NET_ALLOW_PRIVATE` | `false` | Allow `std/net` probes to private/internal targets when each call also passes `allow_private: true` |
| `NTNT_BLOCKED_HOSTS` | `` | Comma-separated list of blocked hostnames |

**Request Body Limits:**
- [x] Configurable max body size via `NTNT_MAX_BODY_SIZE`
- [x] Content-Length header checked before reading
- [x] Returns 413 Payload Too Large with helpful message

**Security Headers (automatic on all responses):**
- [x] `X-Content-Type-Options: nosniff` — prevent MIME sniffing
- [x] `X-Frame-Options: DENY` — prevent clickjacking
- [x] `X-XSS-Protection: 1; mode=block` — legacy XSS filter
- [x] `Referrer-Policy: strict-origin-when-cross-origin` — control referrer leakage
- [x] Server header hidden in production mode

**Open Redirect Protection:**
- [x] `redirect_safe(url, fallback?)` — safe redirect that rejects absolute URLs
- [x] Blocks protocol-relative URLs (`//evil.com`)
- [x] Blocks dangerous schemes (`javascript:`, `data:`, etc.)

**SSRF Protection (fetch, download):**
- [x] Blocks private IP ranges (10.x, 172.16-31.x, 192.168.x)
- [x] Blocks loopback addresses (127.x, ::1)
- [x] Blocks cloud metadata endpoints (169.254.169.254, etc.)
- [x] Blocks link-local addresses
- [x] DNS resolution validation before request

**Path Traversal Protection:**
- [x] Static file serving rejects `..` patterns
- [x] URL-encoded traversal patterns detected and blocked
- [x] `save_upload()` validates destination paths
- [x] Filename sanitization on multipart uploads

**Cookie Security (production defaults):**
- [x] `Secure: true` by default in production
- [x] `SameSite: Lax` by default in production
- [x] `HttpOnly: true` for session/auth cookies in production
- [x] Cookie value encoding prevents header injection

**Error Message Handling:**
- [x] Production mode returns generic error messages
- [x] Development mode shows full details
- [x] Configurable via `NTNT_DETAILED_ERRORS`

**Password Hashing:**
- [x] Minimum bcrypt cost raised to 10 (OWASP compliance)

### 7.19 OAuth/OIDC Authentication (`std/auth`) ✅

**Goal:** Full OAuth 2.0 and OIDC support with progressive disclosure — simple things simple, complex things possible.

**Supported OAuth Flows:**
- [x] Authorization Code (server-side apps)
- [x] Authorization Code + PKCE (SPAs, mobile, CLI)
- [x] Client Credentials (machine-to-machine)
- [x] Refresh Token (long-lived sessions)

**OIDC Support:**
- [x] ID token extraction and validation
- [x] Nonce for replay attack protection
- [x] OIDC Discovery (`oauth_discover()` auto-configures from issuer)
- [x] ID token claims as user info source (fixes Apple Sign In)
- [x] Issuer and audience validation

**Progressive Disclosure API:**

```ntnt
import { oauth, get_user, get_session, oauth_discover } from "std/auth"

// One line for common cases
enable_auth(oauth("google", client_id, client_secret))

// With PKCE (for SPAs/mobile)
enable_auth(oauth("google", id, secret, map { "use_pkce": true }))

// OIDC Discovery (Okta, Auth0, Keycloak)
let provider = oauth_discover("https://mycompany.okta.com", client_id, client_secret)?
enable_auth(provider)

// M2M authentication (server-to-server)
let token = oauth_client_credentials(token_url, client_id, client_secret, ["api.read"])?

// API acting as resource server
let claims = oauth_validate_token(req.headers["authorization"], map {
    "issuer": "https://accounts.google.com",
    "audience": "my-api-client-id"
})?
```

**Built-in Providers (8 fully configured + 2 discovery-based):**
- [x] Google (OIDC, PKCE)
- [x] GitHub (OAuth2)
- [x] Facebook (PKCE)
- [x] Microsoft (OIDC, PKCE)
- [x] Discord (PKCE)
- [x] Twitter (OAuth 2.0, PKCE required)
- [x] LinkedIn (OIDC)
- [x] Apple (OIDC, uses ID token)
- [x] Okta (via `oauth_discover()`)
- [x] Auth0 (via `oauth_discover()`)

**Core Functions (12 total):**
- [x] `oauth(provider, client_id, client_secret, options?)` — create provider configuration
- [x] `oauth_discover(issuer, client_id, secret?)` — auto-configure from OIDC discovery
- [x] `oauth_client_credentials(token_url, id, secret, scopes)` — M2M token grant
- [x] `oauth_refresh(req)` — refresh access token using stored refresh token
- [x] `oauth_validate_token(token, options)` — validate incoming bearer tokens
- [x] `oauth_introspect(url, token, id, secret)` — token introspection (RFC 7662)
- [x] `get_user(req)` — get authenticated user from request
- [x] `get_session(req)` — get full session with tokens and data
- [x] `logout_user(req)` — clear session and redirect

**JWT Support:**
- [x] `jwt_sign(payload, secret, options?)` — create signed JWT (HS256)
- [x] `jwt_verify(token, secret, options?)` — verify and decode JWT
- [x] `jwt_decode(token)` — decode without verification

**Auto-Registered Routes:**
- [x] `GET /auth/{provider}` — start OAuth flow (with OIDC nonce, PKCE)
- [x] `GET /auth/callback` — handle callback, validate ID token
- [x] `POST /auth/logout` — clear session

**Security Features:**
- [x] CSRF protection via OAuth state parameter
- [x] OIDC nonce validation (replay attack protection)
- [x] PKCE for public clients (code verifier/challenge)
- [x] ID token issuer/audience/expiry validation
- [x] HttpOnly, Secure, SameSite cookies

**Developer Experience:**
- [x] Typo suggestions for provider names (Levenshtein distance)
- [x] Sensible defaults for each provider's scopes
- [x] In-memory session storage (zero-config, works out of box)
- [x] Provider-specific user info extraction (id, email, name, picture)
- [x] Token storage in session (opt-in via `store_tokens: true`)

**Phase 7 Deliverables:**

- ✅ Semicolons removed from language (lint warning `unnecessary_semicolon`, examples cleaned up, return parser updated)
- ✅ Ghost keywords removed (`approve`, `observe`, `protocol` — no longer reserved)
- ✅ `otherwise` keyword for inline error handling on Result/Option
- ✅ Type system with inference and enforcement (gradual typing, strict mode)
- ✅ Effect enum removed (dead code cleanup)
- ✅ `?` operator for Result and Option types
- ✅ Anonymous functions with closure semantics
- ✅ `std/db/sqlite` module with full CRUD support
- ✅ Pipe operator for linear data transformations
- ✅ Context-rich error messages with suggestions and source snippets
- ✅ Route pattern auto-detection (no more `r""` for route paths)
- ✅ Destructuring assignment (maps, arrays, nested, in parameters and loops)
- ✅ Default parameter values (with type inference from defaults, contract support)
- ✅ NTNT language documentation system (`// @ntnt` doc comments, `build.rs` validation, embedded binary docs)
- ✅ If-expressions (conditional ternary returning a value)
- ✅ Regex capture groups (`capture_pattern`, `capture_all_pattern`, `capture_named_pattern`)
- ✅ None/null JSON serialization (consistent NULL→None across json, sqlite, postgres, http_server)
- ✅ Web application essentials (password hashing, cookies, logging, CORS, file uploads)
- ✅ OAuth authentication (`std/auth` with 8 providers, JWT support, zero-config)
- ~~Guard clauses (`let-else`)~~ — superseded by `otherwise` (7.2.1)
- Intent file Meta section cleanup (7.11 — pending)
- Import error quality (7.13 — pending)
- Updated examples using new features

---

## Phase 8: Intent System Maturity

**Status:** Not Started

**Goal:** Make Intent-Driven Development a tool that AI agents and humans genuinely rely on — not just for testing, but as the shared plane of understanding and accountability between human and agent.

> Phase 6 proved the concept: intent files, the IAL engine, and `ntnt intent check` work. This phase makes the system something an agent *wants* to use by fixing the friction points discovered through real-world usage: opaque failures, offline validation, glossary debugging, and the lack of shared decision history.

### 8.1 Resolution Chain in Failure Output

**Priority:** Highest — when a test fails, the agent currently has to play detective.

Today, a failure looks like:

```
FAIL: they see "Welcome"
```

The agent doesn't know: was the response a 500? Was the body empty? Was "Welcome" misspelled? The IAL engine already has the full resolution chain internally — it just doesn't surface it.

**Target output:**

```
FAIL: they see "Welcome"
  Resolved: body contains "Welcome"
  Primitive: Check(Contains, response.body, "Welcome")

  Actual status: 200
  Actual body: "<h1>Welcom to the site</h1>"
  ─────────────────────────
  Closest match: "Welcom" (missing 'e')
```

**Implementation plan:**

- [ ] Surface the resolution chain in `ntnt intent check` failure output (glossary term → standard term → primitive)
- [ ] Show actual HTTP response data on failure (status, body excerpt, headers)
- [ ] Fuzzy match suggestions when `body contains` fails ("did you mean 'Welcom'?")
- [ ] JSON output mode (`--json`) for agent consumption of failure details
- [ ] Show resolution chain in Intent Studio failure cards

### 8.2 Offline Intent Validation (`ntnt intent validate`)

**Priority:** High — the collaborative design phase needs fast feedback without starting a server.

During the design phase (drafting features, refining scenarios with the human), there's no way to check if the intent file is well-formed without starting the full server. This makes iteration slow.

```bash
$ ntnt intent validate server.intent

✓ 12 features parsed
✓ 8 glossary terms defined
✓ All terms resolve to primitives
⚠ Feature "user.profile" has no scenarios
⚠ Glossary term "admin user" is defined but never used
✗ Scenario "Edit profile" uses undefined term "they are redirected"
  hint: did you mean "redirects to"? (standard term)

11 features valid, 1 error, 2 warnings
```

**Implementation plan:**

- [ ] `ntnt intent validate <file.intent>` — parse and validate without server
- [ ] Check all glossary terms resolve to primitives (no dangling references)
- [ ] Warn on unused glossary terms
- [ ] Warn on features with no scenarios
- [ ] Warn on duplicate feature IDs
- [ ] Validate `@implements` annotations reference existing feature IDs (cross-check with `.tnt` file)
- [ ] Suggest corrections for unresolved terms (Levenshtein distance against glossary + standard terms)
- [ ] JSON output mode for agent consumption

### 8.3 Glossary Inspector (`ntnt intent glossary`)

**Priority:** Moderate — the glossary is powerful but opaque. Agents and humans need to see what terms are available and how they resolve.

```bash
$ ntnt intent glossary server.intent

Custom Terms (8):
  "they see {text}"          → body contains {text}
  "success response"         → status 200
  "the home page"            → /
  "a logged in user"         → component.authenticated_user
  "a user posts to {path}"   → POST {path}
  ...

Standard Terms (24):
  "status {code}"            → Check(Equals, response.status, {code})
  "body contains {text}"     → Check(Contains, response.body, {text})
  "redirects to {path}"      → Check(Equals, response.headers.location, {path})
  ...

Resolution Trace:
  "they see success response"
    → "they see {text}" where text = "success response"
    → body contains "success response"
    → Check(Contains, response.body, "success response")
    ⚠ Note: "success response" is a glossary term, not literal text.
       The assertion checks for the literal string "success response" in the body.
       If you meant status 200, use "→ success response" as its own line.
```

**Implementation plan:**

- [ ] `ntnt intent glossary <file.intent>` — list all custom and standard terms
- [ ] `ntnt intent glossary <file.intent> --trace "<term>"` — show full resolution chain for a specific term
- [ ] Detect semantic misuse (glossary term used as literal text inside another term)
- [ ] Show which scenarios use each glossary term (reverse lookup)
- [ ] `--json` output for agent consumption
- [ ] Integration with Intent Studio (glossary panel)

### 8.4 Feature Status Tracking

**Priority:** Moderate — makes the intent file a living project document, not just a static test spec.

```intent
Feature: User Login
  id: feature.user_login
  status: implemented
  since: v0.3.0

Feature: Password Reset
  id: feature.password_reset
  status: planned

Feature: OAuth Integration
  id: feature.oauth
  status: deprecated
  reason: "Replaced by SAML in v0.4.0"
```

**Behavior:**

- `status: planned` — scenarios are **skipped** during `intent check` (not failed), shown as "planned" in Studio
- `status: implemented` — scenarios run normally (default if no status specified)
- `status: deprecated` — scenarios still run but shown with deprecation warning in Studio
- `since:` — tracks when a feature was introduced (informational, used in changelog generation)

**Implementation plan:**

- [ ] Parse `status:` field on Feature blocks (planned | implemented | deprecated)
- [ ] Parse `since:` field (version string, informational)
- [ ] Parse `reason:` field for deprecated features
- [ ] `ntnt intent check` skips planned features with clear "SKIP (planned)" output
- [ ] `ntnt intent check` shows deprecated warnings
- [ ] Intent Studio renders status badges on feature cards
- [ ] `ntnt intent check --include-planned` flag to run planned features (expect failures)

### 8.5 Decision Records

**Priority:** Moderate — the highest-leverage accountability feature. Records *why* choices were made, not just *what* was built.

The intent file currently records what the human and agent agreed to build. But it doesn't record the decisions that shaped those features — why session tokens instead of JWTs, why PostgreSQL instead of SQLite, why this API shape and not another. When an agent returns to a project in a new session, that context is lost.

```intent
Feature: User Authentication
  id: feature.user_auth

  Decision: Session tokens over JWTs
    date: 2026-01-15
    context: "MVP needs simple auth. JWTs add complexity (refresh tokens,
             signing keys) without clear benefit at this scale."
    decided_by: human
    alternatives_considered:
      - "JWT with refresh tokens"
      - "OAuth2 with external provider"

  Decision: Bcrypt for password hashing
    date: 2026-01-15
    context: "Industry standard, built into std/crypto."
    decided_by: agent

  Scenario: Successful login
    When a user posts valid credentials to /login
    → success response
    → they see "session_token"
```

**Why this matters for human-agent collaboration:**

- **Agent context recovery** — when I start a new session, I can read decisions to understand why the code looks the way it does, without asking questions the human already answered
- **Human accountability** — decisions have an author. If something breaks because of a design choice, the history shows who made it and why
- **Design archaeology** — `ntnt intent decisions` lists all decisions across features, creating a lightweight Architecture Decision Record (ADR) system built into the workflow

**Implementation plan:**

- [ ] Parse `Decision:` blocks inside Feature sections
- [ ] Fields: `date:`, `context:`, `decided_by:` (human | agent), `alternatives_considered:` (optional list)
- [ ] `ntnt intent decisions <file.intent>` — list all decisions across features
- [ ] `ntnt intent decisions <file.intent> --by human` — filter by decision maker
- [ ] Intent Studio renders decisions as expandable sections on feature cards
- [ ] Decision records are informational — they don't affect test execution

**Deliverables:**

- Resolution chain visibility in all failure output
- `ntnt intent validate` for offline structural checking
- `ntnt intent glossary` for term inspection and resolution tracing
- Feature status tracking with skip behavior for planned features
- Decision records for shared human-agent accountability
- All new commands support `--json` output for agent consumption

---

## Phase 9: Reserved

*Content moved to Future Considerations.*

---

## Phase 10: Background Jobs, WebSockets & Real-Time

**Status:** Jobs Complete ✅ (DD-037, DD-051, DD-052) · WebSockets Not Started

**Goal:** Production-ready background job system with a declarative Job DSL, pluggable backends, and deep IDD integration — plus WebSocket and SSE support for pushing data to clients. Jobs are first-class language constructs — the `Job` keyword is syntax, not a library import — with the runtime and queue management provided by `std/jobs`.

> Background jobs are essential for any non-trivial web application: sending emails, processing payments, syncing with external APIs, generating reports. NTNT's job system treats jobs as **intentional units of work** rather than just functions to execute, aligning with the IDD philosophy. The `Job` DSL is language-level syntax (like `fn` or `struct`), while the Queue runtime lives in `std/jobs` (like `json()` lives in `std/http/server`). See `design-docs/background_jobs.md` for the full design.

### 10.1 Job DSL & Core Runtime

**Priority:** Foundation — the `Job` declaration syntax and in-memory backend.

```ntnt
/// Sends personalized welcome email to newly registered users
Job SendWelcomeEmail on emails {
    perform(user_id: String) {
        let user = db.find_user(user_id)
        email.send(user.email, "Welcome!", "...")
    }
}

/// Charges customer credit card for completed orders
Job ProcessPayment on payments (retry: 5, timeout: 120s) {
    perform(order_id: String, amount: Float) {
        let order = db.find(order_id)
        stripe.charge(order.customer_id, amount)
    }

    on_failure(error, attempt) {
        alert.notify("Payment failed: {error}")
    }
}

// Enqueue jobs
SendWelcomeEmail.enqueue(map { "user_id": "123" })
ProcessPayment.enqueue_in(3600, map { "order_id": "456", "amount": 29.99 })
```

**Implementation plan:**

- [x] `job` declaration syntax in parser (language-level keyword)
- [x] `perform()` handler with typed arguments
- [x] `on_failure()` hook
- [x] `enqueue()`, `enqueue_at()`, `enqueue_in()` functions
- [x] Queue configuration: `configure_queue(map { ... })`
- [x] In-memory backend (zero dependencies, default)
- [x] Worker loop with retry logic and exponential backoff
- [x] Priority queues with configurable bands (critical/high/normal/low, 0-99)
- [x] Dead letter queue for exhausted retries
- [x] Job cancellation: `cancel_job(job_id)`
- [x] Graceful shutdown (drain in-progress jobs on SIGTERM)
- [x] Job options: `retry`, `timeout`, `backoff`, `priority`, `rate`, `concurrency`, `unique`, `expires`
- [ ] Doc comment metadata parsing (`/// Triggers:`, `/// Affects:`, `/// Side effects:`)

### 10.2 Resilience & Production Features ✅

- [x] Rate limiting per job type (`rate: "100/minute"`)
- [x] Concurrency limits per job type (`concurrency: 5`)
- [x] Job TTL/expiration
- [x] Queue pause/resume (`pause_queue()`, `resume_queue()`)
- [x] `work_async()` for combined HTTP server + worker mode
- [x] `work_jobs()` for dedicated worker processes
- [x] Priority bands with independent thread pools
- [ ] Worker heartbeats (detect crashed workers)
- [ ] Visibility timeout (re-enqueue stale jobs after no heartbeat)
- [ ] Automatic pruning of completed/cancelled jobs

### 10.3 Persistent Backends ✅

- [x] SQLite KV backend (default — reliable, zero-config)
- [x] PostgreSQL backend (`ntnt_jobs` table, connection pooling via deadpool-postgres)
- [x] Redis backend (Redis Streams with XADD/XREADGROUP consumer groups)
- [x] Separate worker processes: `work_jobs()` with queue/band configuration
- [ ] Distributed locking via `SELECT FOR UPDATE SKIP LOCKED` (PostgreSQL)

### 10.4 Composition (Chains, Workflows, Batches)

**Batches: Complete ✅ (DD-052 Phases 1-4)**

- [x] `batch(name, callbacks)` / `seal(handle)` — batch lifecycle
- [x] `enqueue_into(batch_id, job_type, args)` — dynamic batch additions post-seal
- [x] `batch_id()` — thread-local batch context in perform blocks
- [x] `batch_status(handle)` — counter/state introspection
- [x] Batch callbacks: `on_complete`, `on_success`, `on_death`
- [x] Atomic counters (pending/succeeded/dead/cancelled/total)
- [x] Closed-flag race protection, TTL expiry (30d seal, 24h complete)
- [x] Unique jobs / deduplication (`unique: 3600` with SHA256 hash)
- [x] `ntnt jobs batches` / `ntnt jobs batch <bid>` CLI
- [x] Control socket: `batches` and `batch_status` commands
- [x] Streaming events: `batch.created`, `batch.sealed`, `batch.complete`, `batch.succeeded`, `batch.death`

**Chains & Workflows: Not Started**

- [ ] `Chain` declaration syntax (sequential job pipelines)
- [ ] `Workflow` declaration syntax (DAG dependencies with fan-out/fan-in)
- [ ] Workflow status tracking

### 10.5 WebSocket Support

**Priority:** High — essential for modern web apps. Live dashboards, chat, notifications, and real-time job status updates all require pushing data to clients.

The assessment identified this as a key missing feature: "there's no way to push data to clients. This limits NTNT to traditional page-based web apps."

```ntnt
import { broadcast, send_to } from "std/ws"

// WebSocket route — handler called per connection
ws("/chat", fn(conn) {
    // Called when a message arrives
    conn.on_message(fn(msg) {
        // Broadcast to all connected clients
        broadcast("/chat", msg)
    })

    conn.on_close(fn() {
        print("Client disconnected")
    })
})

// Send to specific client from anywhere (e.g., from a job)
ws("/jobs/status", fn(conn) {
    // Client subscribes to job updates
    let job_id = conn.params["job_id"]
    conn.on_open(fn() {
        send_to(conn, json(Queue.status(job_id)))
    })
})

// Push from background jobs
Job ProcessPayment on payments {
    perform(order_id: String) {
        // ... process payment ...
        broadcast("/orders/{order_id}", json(map { "status": "paid" }))
    }
}

listen(8080)
```

**Implementation plan:**

- [ ] `ws(pattern, handler)` global builtin for WebSocket routes (mirrors `get`/`post` pattern)
- [ ] Connection object: `conn.on_message()`, `conn.on_open()`, `conn.on_close()`
- [ ] `send_to(conn, msg)` — send to a specific connection
- [ ] `broadcast(channel, msg)` — send to all connections on a channel
- [ ] `std/ws` module for additional utilities (rooms, connection tracking)
- [ ] Integration with background jobs — push job status updates to clients
- [ ] Server-Sent Events (SSE) as a simpler alternative: `sse(pattern, handler)`
- [ ] Connection state management (track connected clients, rooms/channels)
- [ ] Graceful connection cleanup on server shutdown

### 10.6 IDD Integration & CLI

**Priority:** Moderate — testable jobs are NTNT's differentiator over Sidekiq/Bull/Oban.

```intent
Feature: Welcome Email Job
  id: feature.welcome_email_job
  test:
    - job: SendWelcomeEmail
      args: { "user_id": "123" }
      given:
        - mock db.find_user returns { "id": "123", "email": "test@example.com" }
      assert:
        - status: completed
        - email.send was called with "test@example.com"
```

- [ ] Job testing in `.intent` files (`job:` assertion type)
- [ ] Mock support for job dependencies in IDD scenarios
- [x] `ntnt jobs status` — summary of all queues
- [x] `ntnt jobs list [--status|--queue|--limit|--format]` — filter jobs
- [x] `ntnt jobs inspect <job-id>` — full job details
- [x] `ntnt jobs retry <job-id>` — retry a failed/dead job
- [x] `ntnt jobs cancel <job-id>` — cancel a pending job
- [x] `ntnt jobs clear --status=completed` — bulk delete
- [x] `ntnt jobs batches` — list batches with counters
- [x] `ntnt jobs batch <bid>` — batch detail view
- [ ] `ntnt jobs simulate <JobName> --args='...'` — dry-run without side effects
- [ ] `ntnt jobs replay <job-id>` — re-run with exact same inputs for debugging
- [ ] `--format=json` for agent-consumable output on all commands

### 10.7 Advanced Features (Future)

- [ ] `effect` blocks for explicit side-effect declaration (skipped in simulation mode)
- [ ] Job contracts (`requires(args) { ... }`, `ensures(args, result) { ... }`)
- [ ] Intent verification (`verify()` hook — did the job achieve its purpose, not just run?)
- [ ] Idempotency static analysis in `ntnt lint`
- [ ] Natural language queries: `ntnt jobs ask "why are emails failing?"`
- [ ] AI-powered diagnosis: `ntnt jobs diagnose <job-id>`
- [ ] Request tracing across job chains: `ntnt jobs trace <request-id>`

**Deliverables:**

- `Job`, `Chain`, `Workflow` language-level declaration syntax
- `std/jobs` module with Queue API and worker model
- In-memory, PostgreSQL, and Redis/Valkey backends
- Resilience: heartbeats, retries, dead letter queue, rate limiting, graceful shutdown
- Job composition: chains (sequential), workflows (DAG), batches (parallel)
- WebSocket and SSE support (`ws()` builtin, `broadcast()`, `send_to()`)
- IDD integration for testing jobs in `.intent` files
- `ntnt jobs` CLI commands for monitoring and management
- Simulation mode for dry-run execution

---

## Phase 11: Testing Framework

**Goal:** Comprehensive testing infrastructure complementing Intent-Driven Development.

> IDD tests behavior at the feature level. This phase adds unit testing, mocking, and contract-based test generation for fine-grained code verification.

### 11.1 Unit Test Framework

- [ ] `#[test]` attribute for test functions
- [ ] Test discovery and runner
- [ ] Parallel test execution
- [ ] `assert`, `assert_eq`, `assert_ne` macros
- [ ] `#[should_panic]` for expected failures
- [ ] Test filtering and tagging

```ntnt
#[test]
fn test_user_creation() {
    let user = User.new("Alice", "alice@example.com")
    assert_eq(user.name, "Alice")
    assert(user.email.contains("@"))
}

#[test]
#[should_panic(expected: "invariant violated")]
fn test_invalid_email() {
    User.new("Bob", "invalid-email")
}
```

### 11.2 Contract-Based Test Generation

- [ ] Auto-generate test cases from contracts
- [ ] Property-based testing with contracts
- [ ] Fuzzing with contract guidance
- [ ] Contract coverage metrics
- [ ] Edge case generation from `requires` clauses

```ntnt
// Given this contract:
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{ a / b }

// Auto-generate tests:
// - divide(10, 2) → 5 ✓
// - divide(0, 1) → 0 ✓
// - divide(5, 0) → precondition failure ✓
// - divide(-10, -2) → 5 ✓ (negative handling)
```

### 11.3 Mocking & Test Utilities

- [ ] Mock trait implementations
- [ ] HTTP test client (complements IDD HTTP testing)
- [ ] Database test utilities (test transactions, fixtures)
- [ ] Test fixtures and factories
- [ ] Snapshot testing

```ntnt
#[test]
fn test_with_mock_db() {
    let mock_db = MockDatabase.new()
    mock_db.expect_query("SELECT * FROM users").returns([user1, user2])

    let result = get_all_users(mock_db)
    assert_eq(len(result), 2)
}
```

### 11.4 Test Integration

- [ ] `ntnt test` command (runs all tests)
- [ ] `ntnt test --unit` (unit tests only)
- [ ] `ntnt test --intent` (IDD tests only)
- [ ] Coverage reports (combined unit + IDD)
- [ ] CI/CD integration patterns

```bash
# Run all tests
ntnt test

# Run only unit tests
ntnt test --unit

# Run only IDD feature tests
ntnt test --intent

# Combined coverage report
ntnt test --coverage
```

**Deliverables:**

- `#[test]` attribute system
- Contract-based test generation
- Mocking framework
- Test runner with filtering
- Coverage reporting

---

## Phase 12: Tooling & Developer Experience

**Goal:** World-class developer experience with AI collaboration support.

### 12.1 Language Server (LSP)

- [ ] Go to definition
- [ ] Find references
- [ ] Hover documentation
- [ ] Code completion
- [ ] Inline diagnostics
- [ ] Code actions (quick fixes)
- [ ] Contract visualization

### 12.2 Human Approval Mechanisms (From Whitepaper)

- [ ] `@requires_approval` annotations
- [ ] Approval workflows in IDE
- [ ] Audit trails for approved changes
- [ ] Configurable approval policies

```ntnt
@requires_approval("security")
fn delete_all_users(db: Database) -> Result<Int, DbError> {
    db.execute("DELETE FROM users")
}

@requires_approval("api-change")
pub fn get_user(id: String) -> User {
    // Public API changes require review
}
```

### 12.3 Debugger

- [ ] Breakpoints
- [ ] Step debugging
- [ ] Variable inspection
- [ ] Call stack navigation
- [ ] Contract state inspection
- [ ] DAP (Debug Adapter Protocol) support

### 12.4 User Code Documentation (.tnt Files)

> **Design Doc:** [plans/tnt_code_documentation_design.md](plans/tnt_code_documentation_design.md)
>
> Doc comments (`///`, `//!`), doctests, and contract-as-documentation for user-written `.tnt` code. Depends on LSP (12.1) for hover integration.

- [ ] Add `DocComment` (`///`) and `ModuleDocComment` (`//!`) token types to lexer
- [ ] Attach doc comments to Function, Struct, Enum AST nodes
- [ ] Parse doc comments into structured data (summary, params, examples, metadata annotations)
- [ ] `ntnt docs <file> [function]` displays formatted output for user code
- [ ] Doctest runner: extract `// =>` examples from doc comments, execute, report pass/fail
- [ ] Contract extraction: `requires`/`ensures`/`invariant` auto-generate documentation sections
- [ ] Include doc comments in `ntnt inspect` JSON output
- [ ] LSP hover support for user-defined functions

**Deliverables:**

- Full LSP server
- Human approval system
- Debugger
- User code documentation (doc comments, doctests, contract-as-documentation)

---

## Phase 13: Advanced Static Analysis & Type System

**Status:** Not Started

**Goal:** Deeper static analysis, contract inference, and advanced type system features that make NTNT's safety guarantees stronger without runtime cost.

### 13.1 Contract Inference

Contract inference warns when you call a function with contracts without satisfying them. Contracts remain completely optional — inference only activates for contracts that someone chose to write.

```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
{
    return a / b
}

fn compute(x: Int, y: Int) -> Int {
    return divide(x, y)
    //              ^ Warning: `divide` requires `b != 0` but `y` has no such guarantee.
    //                hint: add `requires y != 0` to `compute`, or check before calling.
}
```

- [ ] **Single-level propagation** — warn when calling a `requires` function with an unchecked argument
- [ ] Suggest adding a matching `requires` clause to the caller
- [ ] Recognize common patterns: `if x != 0 { divide(a, x) }` satisfies `requires x != 0`
- [ ] Recognize `match` arms: `Some(v) => use(v)` satisfies `requires v != None`
- [ ] **Transitive propagation** — propagate contracts through entire call chains (A→B→C)
- [ ] Contract static verification (prove contracts hold using SMT solvers or abstract interpretation)
- [ ] Auto-generate `requires` clauses from analysis of function body
- [ ] Contract inference across module boundaries
- [ ] Escape analysis for optimization hints

### 13.2 Advanced Type System Features

- [ ] Associated types in traits
- [ ] Where clauses for complex constraints
- [ ] Contract inheritance (contracts propagate to trait implementations)
- [ ] Liskov Substitution Principle enforcement
- [ ] Contravariant preconditions, covariant postconditions
- [ ] Error context/wrapping: `result.context("message")?`

**Already completed (Phase 7.1):**
- [x] Flow-sensitive typing (type narrows after null checks)
- [x] Exhaustive type checking at compile time (match exhaustiveness)
- [x] Type narrowing in conditionals and match arms

**Deliverables:**

- Contract inference with single-level and transitive propagation
- Advanced type system features (associated types, where clauses, contract inheritance)

---

## Phase 14: AI Integration & Structured Edits

**Goal:** First-class AI development support—NTNT's key differentiator.

### 14.1 Structured Edits (From Whitepaper)

- [ ] AST-based diff format
- [ ] Semantic-preserving transformations
- [ ] Edit operations: AddFunction, ModifyContract, RenameSymbol, etc.
- [ ] Machine-readable edit format for AI agents

```ntnt
// Instead of text diffs, edits are structured:
Edit {
    type: "ModifyContract",
    target: "fn calculate_shipping",
    add_requires: "dest.len() > 0",
    rationale: "Prevent empty destination strings"
}
```

### 14.2 AI Agent SDK

- [ ] Agent communication protocol
- [ ] Context provision API (give AI relevant code context)
- [ ] Suggestion acceptance/rejection tracking
- [ ] Learning from corrections

### 14.3 Semantic Versioning Enforcement

- [ ] API signature tracking across versions
- [ ] Automatic breaking change detection
- [ ] Semver suggestions based on changes
- [ ] `@since` and `@deprecated` annotations

```ntnt
@since("1.2.0")
@deprecated("2.0.0", "Use get_user_by_id instead")
fn get_user(id: String) -> User { }
```

### 14.4 Commit Rationale Generation

- [ ] Structured commit metadata
- [ ] Link commits to intents and requirements
- [ ] Auto-generate changelog entries
- [ ] AI-friendly commit format

### 14.5 AI Agent Optimization

Targeting the specific weaknesses of LLMs: context limits, hallucinations, and safety.

#### 14.5.1 Machine-Readable Diagnostics (`--json` output)

Enable reliable "Self-Correction Loops" for agents.

- [ ] `ntnt check --format=json`
- [ ] `ntnt lint --format=json`
- [ ] Structured errors with remediation suggestions
- [ ] Codes for common agent mistakes (e.g., E023 "Undefined variable")

```json
{
  "file": "server.tnt",
  "line": 45,
  "column": 12,
  "severity": "error",
  "code": "E023",
  "message": "Undefined variable 'usr'",
  "suggestion": {
    "text": "Did you mean 'user'?",
    "replacement": "user",
    "start": 12,
    "end": 15
  }
}
```

#### 14.5.2 Token-Optimized Context (`ntnt describe`)

Provide compressed summaries of the codebase to save tokens and reduce distraction.

- [ ] `ntnt describe src/` command
- [ ] Extracts: Structs, Signatures, Contracts, Imports
- [ ] Strips: Function bodies, comments (unless doc comments)
- [ ] "Searchable Index" for agents to find correct imports

#### 14.5.3 Native "Simulation Mode" (Safety Nets)

Allow agents to execute code safely without side effects on production data.

- [ ] Global `--dry-run` flag
- [ ] `std/env` simulation context check
- [ ] Mocking of side-effecting built-ins (`execute`, `write_file`) in simulation mode

```ntnt
// In std/db
pub fn execute(query, params) {
    if (Global.is_simulation) {
        log("WOULD EXECUTE: " + query);
        return Ok(0);
    }
    // ... real execution
}
```

#### 14.5.4 First-Class `todo` Keyword (Hole-Driven Development)

Allow agents to partially implement features without blocking compilation.

- [ ] `todo` keyword (or `???`)
- [ ] Syntactically valid but panics at runtime
- [ ] Compiler passes `todo` blocks

```ntnt
fn complex_logic(user) {
    if (check_auth(user)) {
        todo "Implement retry logic"
    }
}
```

#### 14.5.5 "Smart Import" Resolution

reduce hallucinated imports by suggesting correct paths.

- [ ] "Smart Linker" in compiler/linter
- [ ] Scans standard library and local modules for missing exports
- [ ] Error message suggests correct path: "Error: `json` not found in `std/http`. Did you mean `std/http/server`?"

**Deliverables:**

- Structured edit engine
- AI agent SDK
- Semantic versioning tools
- Commit rationale system

---

## Phase 15: Deployment & Operations

**Goal:** Production deployment support.

### 15.1 Build & Distribution ✅

- [x] Single binary compilation (Rust `cargo build --release`)
- [x] Cross-compilation — CI builds for macOS (aarch64), Linux (x86_64), Windows (x86_64)
- [x] Build profiles (`dev-release`, `release`)
- [x] GitHub Release workflow — tag push triggers cross-platform builds + artifacts
- [x] Install script (`install.sh`, `install.ps1`)
- [ ] Minimal Docker image generation (users create Dockerfiles manually)

### 15.2 Configuration

- [x] Environment-based config (`get_env()`, `load_env()` for .env files)
- [x] Config file support (`ntnt.toml` for lint settings)
- [ ] Secrets management patterns
- [ ] Validation with contracts

### 15.3 Observability

- [x] Structured logging (`std/log` — `log_debug`, `log_info`, `log_warn`, `log_error`, `set_log_level`)
- [x] Request logger middleware (`request_logger()`)
- [ ] Metrics collection (Prometheus format)
- [ ] Distributed tracing (OpenTelemetry compatible)
- [ ] Health check endpoint (trivial to add manually, not built-in)
- [ ] Contract violation reporting

### 15.4 Graceful Lifecycle

- [x] Signal handling — `ctrlc` handler in worker mode, `tokio::signal::ctrl_c` in async server
- [x] Graceful shutdown — async server uses `with_graceful_shutdown`, workers drain via channel close
- [ ] Connection draining (in-flight request completion)
- [ ] Shutdown hooks (first-class API)
- [ ] Startup/readiness probes

**Deliverables:**

- ✅ Cross-platform binary distribution via GitHub Releases
- ✅ Structured logging
- ✅ Graceful shutdown on SIGTERM/SIGINT
- Remaining: Prometheus metrics, OpenTelemetry, shutdown hooks

---

## Future Considerations (Post-1.0)

These features are valuable but not essential for the initial release:

### Package & Module Ecosystem (was Phase 9)

Deferred for simplicity and security. The stdlib-first approach covers the majority of web application needs.

- Project manifest (`ntnt.toml`) with dependency declaration
- NTNT-native packages with `lib.tnt` entry points
- Local path and git URL dependency resolution
- Rust FFI extension API for native packages
- Package registry and publishing (`ntnt publish`, `ntnt add`)
- Dependency caching and lockfile

### Performance & Compilation (was Phase 13)

Deferred — current interpreter performance is sufficient for target use cases.

- Bytecode VM (10-50x speedup) — bytecode format, compiler, stack-based VM
- VM optimizations — constant folding, dead code elimination, inline caching
- Memory management — reference counting, string interning, arena allocators
- Native compilation via Cranelift, LLVM, or Rust transpilation (100-1000x speedup)
- Runtime library for native compilation targets

### Concurrency — Async HTTP Requests

- [ ] Async HTTP requests (requires async runtime integration with interpreter)

### Pipeline Operator (`|>`) → Moved to Phase 7.5

### Response Caching (Server-Side)

In-memory caching for HTTP handler responses. Note: For most use cases, CDN caching via HTTP headers (e.g., `Cache-Control: s-maxage=N` for Cloudflare) is sufficient and preferred. Server-side response caching is only needed for expensive computations that can't be cached at the edge.

- [ ] `std/cache` module with TTL-based caching
- [ ] `cache()` middleware for route handlers
- [ ] Cache key generation from request (path, query params)
- [ ] Manual cache API: `create_cache`, `get_cached`, `set_cached`, `invalidate`

### Effect System (Rebuilt)

> **History:** An effect system was partially implemented in Phase 2.4 (syntax parsing only, no enforcement) and removed in Phase 7.1 as dead code. This section describes a proper rebuild that depends on the static analysis infrastructure from Phase 13.

Effect tracking lets the compiler verify that functions only perform the side effects they declare. A `pure` function can't call an `IO` function. A function that deletes data requires `approval("security")`. The compiler enforces this statically — no runtime cost.

```ntnt
fn read_config(path: String) -> String with io {
    return read_file(path)
}

fn add(a: Int, b: Int) -> Int pure {
    return a + b  // compiler error if this called read_file()
}

@requires_approval("destructive")
fn reset_database(db: Database) with io {
    execute(db, "DROP ALL TABLES")
}
```

**Prerequisites:**
- Phase 7.1: Enforced type system (effect checking extends type checking)
- Phase 13.1+: Bytecode compiler / static analysis passes (effect propagation through call chains)

**Implementation:**
- [ ] Effect inference (auto-detect effects from function body)
- [ ] Effect propagation (if `f` calls `g with io`, then `f` has `io` too)
- [ ] Static enforcement (`pure` functions cannot call `io` functions)
- [ ] `Approval` effect integrated with Human Approval Mechanisms (Phase 12.3)
- [ ] Effect polymorphism (generic functions that preserve caller's effects)
- [ ] Contract interaction (contracts on `pure` functions can be statically verified)

**Why wait:** A real effect system requires analyzing the full call graph statically. The tree-walking interpreter can't do this — it would need the bytecode compiler's static analysis passes to trace effect propagation across function calls, modules, and generics. Building it before that infrastructure exists would repeat the mistake of Phase 2.4: syntax without enforcement.

### Session Types

- Protocol definitions for typed communication
- Deadlock prevention at compile time
- Formal verification of message sequences

### Additional Database Drivers

**PostgreSQL Enhanced Support (Current):**

- [x] Basic types: INT, BIGINT, FLOAT, DOUBLE, TEXT, VARCHAR, BOOL
- [x] NUMERIC/DECIMAL (via rust_decimal)
- [x] DATE, TIME, TIMESTAMP, TIMESTAMPTZ (via chrono)
- [x] JSON/JSONB
- [x] UUID
- [x] Arrays: INT[], TEXT[], FLOAT[], BOOL[]
- [ ] BYTEA (binary data)
- [ ] INTERVAL
- [ ] PostGIS geometry types

**Additional Drivers:**

- MySQL/MariaDB
- SQLite (→ moved to Phase 7.4 as priority item)
- Redis client

### High-Performance HTTP Server ✅ PARTIAL

The HTTP server now uses Axum + Tokio for async request handling:

- [x] Async runtime (Tokio) for concurrent connections
- [x] Connection pooling and keep-alive
- [x] Bridge pattern connecting async handlers to sync interpreter
- [ ] HTTP/2 support with multiplexing
- [ ] Request pipelining
- [ ] Zero-copy response streaming
- [ ] Performance target: 100k+ req/sec on commodity hardware

### WebSocket Support → Moved to Phase 10.5

### Concurrency Primitives

- Channels for message passing
- Structured concurrency (task scopes)
- Parallel iterators

---

## Implementation Priority Matrix

| Phase      | Focus                            | Business Value     | Status     |
| ---------- | -------------------------------- | ------------------ | ---------- |
| 1-5 ✅     | Core Language + Web              | Foundation         | Complete   |
| 6 ✅       | Intent-Driven Dev                | High               | Complete   |
| 7 ✅       | Ergonomics & Documentation       | High               | ~95% done  |
| **8**      | **Intent System Maturity**       | **High**           | **Up Next**|
| 9          | Reserved                         |                    |            |
| 10 🟡     | Jobs ✅, WebSockets ❌           | High               | Partial    |
| 11         | Testing Framework                | High               | Not Started|
| 12         | Tooling & DX (LSP, Debugger)     | Very High          | Not Started|
| 13         | Static Analysis & Type System    | High               | Not Started|
| 14         | AI Integration                   | **Differentiator** | Not Started|
| 15         | Deployment                       | High               | Not Started|

---

## Milestones

> Milestones M1 (Language Complete) and M2 (Web Ready) are complete. See [ROADMAP_COMPLETE.md](ROADMAP_COMPLETE.md).

### M3: Ergonomic Language (End of Phase 7)

- Enforced type system with inference
- Error propagation (`?` operator)
- Anonymous functions / closures
- SQLite support
- Pipe operator for linear data flow
- Context-rich error messages with suggestions
- Route pattern auto-detection (no `r""` needed)
- Destructuring, default parameters
- If-expressions (inline conditional values)
- Regex capture groups for structured text extraction
- Two-layer safety: types (structural) + contracts (semantic)
- NTNT language documentation system (doc comments + `build.rs` validation, embedded in binary)
- A typical web handler drops from ~22 lines to ~6

### M4: Mature Intent System (End of Phase 8)

- Resolution chain visibility in failure output
- Offline intent validation (`ntnt intent validate`)
- Glossary inspector (`ntnt intent glossary`)
- Feature status tracking (planned/implemented/deprecated)
- Decision records for human-agent accountability
- Intent system is a tool agents genuinely rely on

### M5: Background Processing (End of Phase 10) ✅ PARTIAL

- ✅ `job` language-level declarations with perform/on_failure
- ✅ `std/jobs` with SQLite KV, PostgreSQL, and Redis backends
- ✅ Resilience: retries, rate limiting, concurrency limits, dead letter queue, pause/resume
- ✅ Batch system with dynamic adds, callbacks, TTL expiry
- ✅ `ntnt jobs` CLI for monitoring and management
- ❌ WebSocket and SSE support (not started)

### M6: Developer Ready (End of Phase 12)

- Full IDE support (LSP)
- Human approval workflows
- Comprehensive testing framework (unit + IDD)

### M7: Production Ready / 1.0 (End of Phase 15)

- ✅ Cross-platform distribution
- ✅ Structured logging + graceful shutdown
- AI integration (structured edits, agent SDK)
- Metrics + tracing (Prometheus, OpenTelemetry)

---

## Success Metrics

- **Time to First App:** Hello World web API in < 30 minutes
- **Performance (Bytecode VM):** Within 5x of Go for web workloads
- **Performance (Native):** Within 2x of Go with Cranelift/LLVM backend
- **Safety:** Zero contract violations reach production
- **AI Compatibility:** 95%+ of AI-generated code compiles on first try
- **Developer Satisfaction:** Tooling comparable to Go/Rust

---

## Example: Complete Web Application

```ntnt
// main.tnt - A complete NTNT web application

import { Server, Request, Response } from "std/http"
import { Database } from "std/db/postgres"
import { Logger } from "std/log"

let log = Logger.new("api")
let db = Database.connect(env("DATABASE_URL"))

struct User {
    id: String,
    name: String,
    email: String
}

impl User {
    invariant self.name.len() > 0
    invariant self.email.contains("@")
}

intent "Retrieve a user by their unique ID" {
    fn get_user(req: Request) -> Response
        requires req.params.id.len() > 0
    {
        match db.query_one("SELECT * FROM users WHERE id = $1", [req.params.id]) {
            Ok(user) => Response.json(user),
            Err(_) => Response.not_found("User not found")
        }
    }
}

intent "Create a new user with validated data" {
    fn create_user(req: Request) -> Response
        requires req.body.name.len() > 0
        requires req.body.email.contains("@")
        ensures result.status == 201 || result.status >= 400
    {
        let user = User {
            id: uuid(),
            name: req.body.name,
            email: req.body.email
        }

        db.insert("users", user)?
        log.info("Created user", { id: user.id })

        Response.created(user)
    }
}

@requires_approval("api-change")
pub fn main() {
    let app = Server.new()
        .get("/users/{id}", get_user)
        .post("/users", create_user)
        .use(logging)
        .use(cors)

    log.info("Starting server on port 8080")
    app.listen(8080)
}
```

### 7.20 Nested Assignment (Deep Mutation) ✅

> **Status: Implemented.** Nested assignment works for arbitrary depth including `array[i]["key"]`, `map["a"]["b"]["c"]`, and mixed nesting. Comprehensive tests cover single-level through four-level nesting, error cases (immutable, out-of-bounds), and new key creation.

**Goal:** Support assigning to nested structures like `array[i]["key"] = value` and `map["a"]["b"] = value`, which currently fails with "Invalid assignment target".

**Motivation:** This was hit repeatedly while building a real app (Larri Dashboard) using `std/auth`. Any app that stores data in arrays of maps (users, tasks, sessions) needs to update individual fields without rebuilding the entire object. The current workaround — reconstructing the map and replacing the array element — is verbose and error-prone.

**Current behavior:**
```ntnt
let users = load_users()
users[0]["role"] = "admin"  // ❌ Runtime error: Invalid assignment target
```

**Workaround (painful):**
```ntnt
let user = users[0]
let updated = map {
    "id": user["id"],
    "email": user["email"],
    "role": "admin",
    // ... copy every other field manually
}
let new_users = []
let i = 0
for u in users {
    if i == 0 { new_users = new_users + [updated] } else { new_users = new_users + [u] }
    i = i + 1
}
```

**Desired behavior:**
```ntnt
users[0]["role"] = "admin"           // ✅ Nested index + key assignment
users[0]["profile"]["name"] = "Bob"  // ✅ Arbitrary depth
task["tags"][2] = "updated"          // ✅ Map → array nesting
```

**Implementation notes:**
- The interpreter's assignment handling needs to walk nested `Index` / `FieldAccess` chains
- Each intermediate step must resolve to a mutable reference
- Arrays and maps both need to support being the "parent" at any level
- Should work with `let mut` variables (or all variables if NTNT stays default-mutable)

**Priority:** High — this is the most common "papercut" when building real NTNT web apps with data persistence.

### 7.21 Nested `{{#if}}` Blocks in Templates ✅

> **Status: Implemented.** Nested `{{#if}}` blocks work to arbitrary depth, including with `{{#else}}`, `{{#elif}}`, and inside `{{#for}}` loops. Comprehensive tests cover two/three-level nesting, mixed content, and various condition combinations.

**Goal:** Support nested `{{#if}}` conditionals inside template strings and external template files.

**Motivation:** Hit while building the Larri Dashboard nav component. The template engine correctly handles a single `{{#if}}` block, but any `{{#if}}` nested inside another `{{#if}}` renders as literal text instead of being evaluated.

**Current behavior:**
```html
{{#if is_admin}}
  <a class="{{#if admin_active}} active{{/if}}" href="/admin">Admin</a>
{{/if}}
```
Renders the inner `{{#if admin_active}}` and `{{/if}}` as visible text in the HTML output.

**Desired behavior:**
Both levels of `{{#if}}` should evaluate. Nesting should work to arbitrary depth, matching the behavior users expect from any template engine.

**Root cause:** The lexer's `find_matching_end()` and `parse_if_block_content()` in `src/lexer.rs` don't account for nested blocks when scanning for the closing `{{/if}}` tag — the first `{{/if}}` encountered closes the outer block, leaving the inner block's closing tag as literal text.

**Workaround:** Pre-compute all conditional values in the data map and use only simple `{{var}}` interpolation in templates. This works but defeats the purpose of having `{{#if}}` in templates.

**Priority:** Medium — templates are usable with the workaround, but nested conditionals are a basic expectation of any template system.

---

### 7.22 Collections & Functional Array Operations ✅

**Goal:** Provide sort, merge, and functional array operations (filter, map, find, reduce) to eliminate manual loops and verbose patterns in real applications.

**Motivation:** Building the Larri Dashboard revealed repeated patterns: bubble sort implementations (3 instances, ~45 lines), manual map reconstruction for updates, verbose `match get_key` for defaults, and for-loop filtering. These are solved problems in every modern language.

**Functions (all added in v0.3.13):**

- [x] `sort(array, key_or_fn?)` — Sort ascending by natural order, map key, or function
- [x] `sort_desc(array, key_or_fn?)` — Sort descending
- [x] `merge(map1, map2)` — Shallow merge, map2 wins on conflict
- [x] `get_or(map, key, default)` — Get with default (no Option unwrapping needed)
- [x] `filter(array, fn)` — Keep elements where fn returns true
- [x] `transform(array, fn)` — Apply fn to each element (map operation)
- [x] `find(array, fn)` — First element matching fn, as Option
- [x] `any(array, fn)` — True if any element matches
- [x] `all(array, fn)` — True if all elements match
- [x] `count(array, fn)` — Count matching elements
- [x] `reduce(array, initial, fn)` — Fold/reduce with accumulator
- [x] `flat_map(array, fn)` — Map then flatten one level

**Implementation:** `merge` and `get_or` are NativeFunctions in `src/stdlib/collections.rs`. All higher-order functions (`sort`, `sort_desc`, `filter`, `transform`, `find`, `any`, `all`, `count`, `reduce`, `flat_map`) are special-cased builtins in `src/interpreter.rs` because they need access to the interpreter to call closures.

---

---

## Security & Performance Hardening (v0.3.14+)

Based on the comprehensive security and performance audit conducted February 2026.
See [SECURITY_AUDIT.md](SECURITY_AUDIT.md) and [PERFORMANCE_AUDIT.md](PERFORMANCE_AUDIT.md) for full details.

### Security Fixes (Completed in v0.3.14)
- [x] Path traversal protection for async HTTP server static file serving
- [x] HTTP client response size limits (50MB default, configurable)
- [x] PostgreSQL connection error sanitization (prevent credential leakage)

### Security Hardening (Proposed)
- [x] Interpreter recursion depth limit (default: 256)
- [ ] Per-request timeout for sync HTTP server
- [ ] Optional filesystem sandboxing (`NTNT_FS_ROOT`)
- [x] Random dev-mode session secret generation
- [x] Warning on CORS wildcard origin in production

### Performance Improvements (Proposed)
- [ ] Template compilation/caching
- [ ] In-memory static file caching
- [x] Replace Redis `KEYS` with `SCAN` in KV module
- [ ] Interpreter thread pool for async server (multi-interpreter)
- [x] Automatic session cleanup timer for in-memory sessions
- [ ] Copy-on-write Value semantics (major refactor)

---

_This roadmap is a living document updated as implementation progresses._
_Last updated: April 2026 (v0.4.7 — Jobs complete, Phase 9/13 deferred)_
