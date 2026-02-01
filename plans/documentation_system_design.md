# NTNT Language Documentation System

> Making the Rust source code the single source of truth for all NTNT language documentation

## Scope

This document covers documenting **the NTNT language itself** — keywords, operators, stdlib functions, builtins, IAL primitives, runtime behavior, and error types that ship with the `ntnt` binary.

For documenting user-written `.tnt` code (doc comments, doctests), see [tnt_code_documentation_design.md](tnt_code_documentation_design.md).

---

## The Problem

NTNT's implementation lives in Rust, but its documentation lives in separate TOML files:

| What | Implemented In | Documented In | Sync Risk |
|------|----------------|---------------|-----------|
| Keywords (`let`, `fn`, `match`, ...) | `src/lexer.rs` | `docs/syntax.toml` | **High** |
| Operators (`+`, `..`, `\|>`, ...) | `src/parser.rs` | `docs/syntax.toml` | **High** |
| Stdlib functions (`split`, `join`, ...) | `src/stdlib/*.rs` | `docs/stdlib.toml` | **High** |
| Global builtins (`len`, `print`, ...) | `src/interpreter.rs` | `docs/stdlib.toml` | **High** |
| IAL primitives (`Http`, `Check`, ...) | `src/ial/*.rs` | `docs/ial.toml` | **High** |
| Runtime behavior (env vars, routing) | Various Rust files | `docs/runtime.toml` | **Medium** |
| Error types | `src/error.rs`, various | Nowhere | **Total** |

**The failure mode:** Someone adds a function to `src/stdlib/string.rs`. The code works. But `docs/stdlib.toml` is never updated. Nothing prevents this — no compiler error, no test failure, no CI check. It's a manual discipline problem.

---

## The Solution: Structured Doc Comments + Build-Time Validation

Structured `/// @ntnt` doc comment blocks placed directly above implementation code. `build.rs` scans all source files at compile time, extracts documentation, cross-references against implementations, and **fails the build** if any language element is undocumented.

```
BEFORE: Implementation in .rs  +  Documentation in .toml  =  Drift
AFTER:  Documentation above implementation in .rs  =  Single source of truth
```

**Why doc comments instead of a wrapping macro:**

- **Full IDE support** — rust-analyzer works normally (autocomplete, go-to-definition, type hints)
- **Clear compile errors** — no macro-obscured error spans
- **Normal debugging** — clean stack traces, no macro-generated code
- **No infrastructure** — no proc macro crate to build, test, or maintain
- **No learning curve** — `@tag` convention is familiar from JSDoc, JavaDoc, PyDoc
- **Implementation unchanged** — existing Rust code stays exactly as it is

### Design Principles

**1. Documentation is structured data, not prose.** Structured data enables programmatic queries, AI consumption, and multi-format output from one source. Markdown is an output format, not a source format.

**2. Every example is a test.** Examples in documentation are extracted and executed. If an example doesn't produce the expected output, the build fails. No stale examples.

**3. Progressive disclosure from one source.** All levels generated from the same doc comment block:

| Level | What | Where |
|-------|------|-------|
| L0 | `split(String, String) -> Array<String>` | Autocomplete tooltip |
| L1 | "Splits a string into an array by delimiter" | Search results |
| L2 | + parameters, return value | `ntnt docs split` |
| L3 | + examples, errors, edge cases | `ntnt docs split --full` |
| L4 | + gotchas, related patterns | `ntnt docs split --deep` |

**4. Semantic tags are documentation metadata, not compiler enforcement.** Tags describe function behavior for documentation purposes. They are informational — the compiler does not enforce them. (A real effect system with enforcement is tracked separately in Future Considerations and depends on Phase 13's static analysis infrastructure.)

| Tag | Meaning | Documentation Generated |
|-----|---------|-------------------------|
| `#pure` | No side effects | "This function has no side effects" |
| `#deterministic` | Same input → same output | "Returns consistent results" |
| `#may-panic` | Can panic on bad input | "May panic if..." |
| `#io` | Performs I/O | "Performs I/O operations" |
| `#allocates` | Creates new collections | "Returns a new array/map" |

**5. Lean by default, verbose when needed.** Required fields: `@ntnt` header, summary line, `@signature` (for functions), and at least one `@example`. Everything else — `@param`, `@returns`, `@error`, `@gotcha`, `@tags` — is added only when it provides value beyond what the signature already communicates.

---

## Doc Comment Format

Structured doc comments use `/// @ntnt` as the header line, followed by `@field` tags for metadata and plain `///` lines for prose. The implementation code follows below, completely unchanged.

### Field Reference

```
/// @ntnt <name>                                    ← required: identifies this block
/// @module <module>                                ← stdlib functions only
/// @signature <sig>                                ← functions/builtins only
/// Summary line.                                   ← required: first non-@ line
///
/// Extended description (optional).
/// @param <name> <description>                     ← optional
/// @returns <description>                          ← optional
/// @tags #pure, #deterministic, ...                ← optional
/// @see_also <name1>, <name2>                      ← optional
/// @since <version>                                ← optional
/// @example <code> => <expected> ~ "<description>" ← at least one required
/// @error <Type> ~ "<message>" fix: "<fix>"        ← optional
/// @gotcha <description>                           ← optional
```

Type-specific fields for non-function elements:

| Element Type | Header | Type-Specific Fields |
|-------------|--------|---------------------|
| `keyword` | `@ntnt keyword <name>` | `@category`, `@common_mistake` |
| `operator` | `@ntnt operator <symbol>` | `@name`, `@category`, `@precedence`, `@associativity`, `@contrast` |
| `error` | `@ntnt error <Name>` | `@cause`, `@fix` |
| `ial_primitive` | `@ntnt ial_primitive <Name>` | `@context_sets` |
| `env_var` | `@ntnt env_var <NAME>` | `@values`, `@default` |

---

## Examples by Element Type

### Stdlib Functions

```rust
// src/stdlib/string.rs

/// @ntnt split
/// @module std/string
/// @signature split(s: String, delim: String) -> Array<String>
/// Splits a string into an array of substrings.
///
/// When the delimiter is not found, returns a single-element array
/// containing the original string. When the delimiter is empty,
/// splits into individual characters.
/// @see_also join, trim, replace
/// @since v0.1.0
/// @example split("a,b,c", ",") => ["a", "b", "c"] ~ "Basic comma-separated split"
/// @example split("hello", "") => ["h","e","l","l","o"] ~ "Empty delimiter splits into characters"
/// @example split("no-match", ",") => ["no-match"] ~ "No delimiter found returns original in array"
/// @example split("", ",") => [""] ~ "Empty string returns array with empty string"
module.insert("split".to_string(), Value::NativeFunction {
    name: "split".to_string(),
    func: |args| {
        // existing implementation — completely unchanged
    },
});
```

`build.rs` pairs the `/// @ntnt split` block with the `NativeFunction` insert for `"split"` below it. If either is missing, the build fails.

### Global Builtins

```rust
// src/interpreter.rs

/// @ntnt len
/// @signature len(x: String | Array | Map) -> Int
/// Returns the length of a string, array, or map.
/// @returns Number of characters (string), elements (array), or key-value pairs (map)
/// @tags #pure, #deterministic
/// @see_also type, str, is_empty
/// @since v0.1.0
/// @example len("hello") => 5 ~ "String: counts characters"
/// @example len([1, 2, 3]) => 3 ~ "Array: counts elements"
/// @example len(map { "a": 1 }) => 1 ~ "Map: counts key-value pairs"
/// @example len("") => 0 ~ "Empty string returns zero"
/// @example len([]) => 0 ~ "Empty array returns zero"
/// @error TypeError ~ "len() requires String, Array, or Map, got {type}" fix: "Check the type with type(x) first, or convert with str(x)"
"len" => {
    // existing match arm — unchanged
}
```

Builtins omit `@module` since they're globally available. `build.rs` matches the doc block against the builtin's registration pattern.

### Optional Fields

Add optional fields only when the signature doesn't tell the full story.

**`@param` — when parameter names aren't self-documenting:**

```rust
/// @ntnt fetch
/// @module std/http
/// @signature fetch(url: String, options: Map?) -> Result<Response, String>
/// Fetches a URL with configurable request options.
/// @param options Optional map with keys: method, headers, body, json, form, auth, cookies, timeout
/// @tags #io
/// @since v0.2.0
/// @example fetch("https://api.example.com/users") => Ok(Response { status: 200, ... }) ~ "Simple GET request (no options needed)"
```

**`@error` with `fix:` and `@gotcha` — for functions that can fail in non-obvious ways:**

```rust
/// @ntnt unwrap
/// @signature unwrap(x: Option<T> | Result<T, E>) -> T
/// Unwraps an Option or Result, panicking on None/Err.
/// @returns The inner value of Some(v) or Ok(v)
/// @tags #may-panic
/// @see_also match, ?
/// @since v0.1.0
/// @example unwrap(Some(42)) => 42 ~ "Unwraps Some value"
/// @example unwrap(Ok("hello")) => "hello" ~ "Unwraps Ok value"
/// @error Panic ~ "Called unwrap() on None" fix: "Use match or let-else for safe handling"
/// @error Panic ~ "Called unwrap() on Err({error})" fix: "Use match or the ? operator for error propagation"
/// @gotcha Panics at runtime if the value is None or Err — always prefer match or ? in production code
```

### Keywords

```rust
// src/lexer.rs — near the keyword match statement

/// @ntnt keyword let
/// @category Variables
/// Declares a new variable binding.
///
/// Variables are immutable by default. Use `let mut` for mutable variables.
/// Type annotations are optional but recommended for function parameters.
/// @see_also mut, fn
/// @example `let x = 42` ~ "Immutable integer binding"
/// @example `let name = "Alice"` ~ "Immutable string binding"
/// @example `let mut counter = 0` ~ "Mutable variable (can be reassigned)"
/// @example `let x: Int = 42` ~ "With explicit type annotation"
/// @common_mistake `x = 10` after `let x = 5` ~ "Cannot reassign immutable variable. Use `let mut x = 5`."
```

Keywords don't have a `NativeFunction` insert below them — they're token types in a match arm. `build.rs` cross-references `@ntnt keyword` blocks against the lexer's keyword list to ensure every keyword is documented.

### Operators

```rust
// src/parser.rs

/// @ntnt operator |>
/// @name Pipe
/// @category Pipeline
/// @precedence 8
/// @associativity Left
/// Passes the left-hand value as the first argument to the right-hand function.
///
/// Enables readable left-to-right data transformation chains instead
/// of deeply nested function calls.
/// @example `"hello world" |> to_upper` => `"HELLO WORLD"` ~ "Single pipe transformation"
/// @example `"  hello  " |> trim |> to_upper` => `"HELLO"` ~ "Chained pipes read left to right"
/// @contrast `to_upper(trim(s))` ~ "Equivalent without pipe — reads inside-out instead of left-to-right"

/// @ntnt operator ..
/// @name Range
/// @category Range
/// @precedence 6
/// @associativity Left
/// Creates an exclusive range from start to end.
///
/// The start value is included, the end value is excluded.
/// @see_also ..=
/// @example `for i in 0..3 { print(i) }` => `0, 1, 2` ~ "Iterates 0, 1, 2 (end value 3 is excluded)"
/// @example `0..5` ~ "Range from 0 to 4 (five elements)"
/// @contrast `..=` ~ "Inclusive range — includes end value. 0..=3 gives 0, 1, 2, 3."
```

### Error Types

```rust
// src/error.rs

/// @ntnt error TypeError
/// Raised when an operation receives a value of the wrong type.
/// @cause Passing wrong argument types to a function
/// @cause Using an operator with incompatible types (e.g., String + Int)
/// @cause Accessing a field on a non-struct value
/// @example `len(42)` => TypeError("len() requires String or Array, got Int") ~ "len() only works on strings, arrays, and maps"
/// @example `"hello" + 5` => TypeError("Cannot add String and Int") ~ "String concatenation requires two strings — use str(5) to convert"
/// @fix Check the function signature with `ntnt docs <function>`
/// @fix Use type conversion: `str(42)`, `int("42")`, `float("3.14")`
/// @fix Check the actual type with `type(value)` at runtime

/// @ntnt error ContractViolation
/// Raised when a function's precondition or postcondition is violated.
/// @cause Calling a function with arguments that violate a `requires` clause
/// @cause Function returning a value that violates an `ensures` clause
/// @cause Struct mutation that violates an `invariant`
/// @example `divide(10, 0)` given `requires b != 0` => ContractViolation("Precondition failed: b != 0") ~ "Precondition prevents division by zero"
/// @fix Check function contracts with `ntnt docs <function>`
/// @fix Validate inputs before calling contracted functions
/// @fix Use match/if to guard against invalid values
```

### IAL Primitives

```rust
// src/ial/primitives.rs

/// @ntnt ial_primitive Http
/// Executes an HTTP request and captures the response in context.
/// @context_sets response.status ~ "HTTP status code (integer)"
/// @context_sets response.body ~ "Response body (string)"
/// @context_sets response.headers.* ~ "Response headers (map, lowercase keys)"
/// @context_sets response.time_ms ~ "Response time in milliseconds"
/// @example `GET /api/users` sets `response.status = 200` ~ "Status code captured after request"
/// @example `POST /api/users` with body `{"name": "Alice"}` sets `response.status = 201` ~ "POST with JSON body, status code captured"
```

### Runtime Configuration

```rust
// src/main.rs

/// @ntnt env_var NTNT_ENV
/// @values development, production, prod
/// @default development (when unset)
/// Controls runtime mode.
///
/// In production mode, hot-reload is disabled for better performance.
/// File watchers are not started, reducing CPU usage.
/// @example `NTNT_ENV=production ntnt run server.tnt` ~ "Production mode — no hot-reload"
/// @example `ntnt run server.tnt` ~ "Development mode (default) — hot-reload enabled"

/// @ntnt env_var NTNT_STRICT
/// @values 1, true
/// @default unset (disabled)
/// Enables strict type checking.
///
/// For `ntnt run`, blocks execution and hot-reload if type errors are found.
/// For `ntnt lint`, warns about untyped function signatures.
/// @example `NTNT_STRICT=1 ntnt run server.tnt` ~ "Block execution on type errors"
/// @example `ntnt lint --strict server.tnt` ~ "CLI flag equivalent to env var"
```

---

## Size Impact

### Source Code

Doc comments add approximately **8-12 lines per function** (less than the macro approach since there's no `impl` wrapper or closing braces). For ~232 stdlib functions:

| Metric | Current | With doc comments | Change |
|--------|---------|-------------------|--------|
| `string.rs` | 1,270 lines | ~1,600 lines | +26% |
| Total stdlib | 11,203 lines | ~13,500 lines | +21% |
| TOML doc files | 2,182 lines | 0 lines | -100% |
| **Net lines across codebase** | 13,385 | ~13,500 | **+1%** |

The net increase is negligible because the TOML files are eliminated entirely and there's no macro wrapper overhead.

### Binary Size

Documentation data is **embedded in the binary** so that `ntnt docs` works from anywhere — no external files to locate, no path configuration, no "docs not found" errors. This follows the same approach as Elixir, which stores `@doc` content in compiled `.beam` bytecode.

`build.rs` extracts all `/// @ntnt` blocks and generates a Rust source file (`doc_data.rs`) with the structured data as compiled-in constants. The binary includes this data directly.

| Component | Estimated Size |
|-----------|---------------|
| ~232 stdlib functions (signatures, summaries, examples) | ~150-300 KB |
| ~35 keywords + ~18 operators + ~12 errors | ~20-50 KB |
| Cross-references and metadata | ~10-30 KB |
| **Total embedded doc data** | **~200-400 KB** |

For a binary that's 5-10 MB, this is a 2-5% increase. Reasonable tradeoff for `ntnt docs split` working anywhere with zero setup.

External files (Markdown for GitHub, JSON for AI agents) are still generated by `ntnt docs --generate`, but they're for **publishing** — not required for `ntnt docs` to function.

### Build Time

| Step | Impact | When |
|------|--------|------|
| Source scanning (`build.rs`) | ~1-2 seconds | Every build |
| Coverage validation | Included in scan | Every build |
| `doc_data.rs` generation | Included in scan | Every build |
| Example validation | ~10-30 seconds | `cargo test --features validate-docs` (CI only) |
| External output generation | ~2-5 seconds | `ntnt docs --generate` (publishing only) |

No proc macro compilation step. No macro expansion. Faster clean builds than the macro approach.

---

## Build-Time Pipeline

```
┌─────────────────────────────────────────────────────────┐
│                   RUST SOURCE CODE                       │
│  src/lexer.rs       → /// @ntnt keyword ...              │
│  src/parser.rs      → /// @ntnt operator ...             │
│  src/stdlib/*.rs    → /// @ntnt <fn> ...                 │
│  src/interpreter.rs → /// @ntnt <builtin> ...            │
│  src/ial/*.rs       → /// @ntnt ial_primitive ...        │
│  src/error.rs       → /// @ntnt error ...                │
│  src/main.rs        → /// @ntnt env_var ...              │
└─────────────────────────────────────────────────────────┘
                            │
                            │ cargo build
                            ▼
                ┌──────────────────────────────────┐
                │ build.rs                          │
                │  1. Scan source for @ntnt blocks  │
                │  2. Cross-ref implementations     │
                │  3. Validate 100% coverage        │
                │  4. Generate doc_data.rs           │
                └──────────────────────────────────┘
                            │
                ┌───────────┤
                │           │
                ▼           ▼
       ┌──────────────┐  ┌──────────────────────────────┐
       │ ntnt binary   │  │ ntnt docs --generate          │
       │ (docs embedded│  │  (publishing — on demand)     │
       │  via          │  └──────────────────────────────┘
       │  doc_data.rs) │              │
       └──────────────┘   ┌──────────┼────────────┐
                          ▼          ▼            ▼
                 ┌──────────┐ ┌───────────┐ ┌────────────┐
                 │ Markdown  │ │   JSON    │ │ AI Agent   │
                 │ reference │ │ (AI/tool) │ │ Files      │
                 └──────────┘ └───────────┘ └────────────┘
```

### How `build.rs` Validates Coverage

`build.rs` performs two scans:

1. **Documentation scan** — finds all `/// @ntnt` blocks, parses their fields
2. **Implementation scan** — finds all `NativeFunction` inserts, keyword match arms, operator registrations, error variants, etc.

It then cross-references: every implementation must have a documentation block, and every documentation block must have a corresponding implementation. Missing either side fails the build.

### Example Validation (Separate Step)

```bash
# Development — fast builds, no example validation
cargo build

# CI / pre-release — validate all examples
cargo test --features validate-docs

# Manual validation
ntnt docs --test
```

### Build Failures

| Condition | Result |
|-----------|--------|
| `NativeFunction` without `@ntnt` block above it | **FAIL** — undocumented function |
| `@ntnt` block without matching implementation | **FAIL** — orphaned documentation |
| `@see_also` references non-existent item | **FAIL** — broken cross-reference |
| Keyword in lexer without `@ntnt keyword` block | **FAIL** — undocumented keyword |
| Example produces wrong output | **FAIL** — only with `--features validate-docs` |

---

## Outputs

### Embedded (in binary — always available)

`ntnt docs` queries the compiled-in doc data directly. No external files, no path configuration. Works anywhere the binary is installed.

The embedded data includes: function signatures, summaries, descriptions, parameters, return docs, examples with expected outputs, error conditions, semantic tags, cross-references, keywords, operators, error types, IAL primitives, and env vars.

Concept groupings (e.g., "string_manipulation" → `[split, join, trim, ...]`) are derived automatically from module membership and `@see_also` relationships.

### Generated (on demand — for publishing)

`ntnt docs --generate` produces external files for documentation sites, AI agents, and tooling:

- **`docs/STDLIB_REFERENCE.md`** — human-readable stdlib reference (same as today, from source instead of TOML)
- **`docs/SYNTAX_REFERENCE.md`** — keywords, operators, literals
- **`docs/IAL_REFERENCE.md`** — IAL primitives and check operations
- **`docs/RUNTIME_REFERENCE.md`** — env vars, routing, hot-reload
- **`docs/ntnt_docs.json`** — structured export for AI agents and external tooling, organized by category

These are **not required** for `ntnt docs` to work. They're for publishing to GitHub, documentation sites, and AI agent context.

### AI Agent Files (Auto-Updated)

The central reference is `docs/AI_AGENT_GUIDE.md` — all agent-specific files point to it rather than duplicating content.

```
docs/AI_AGENT_GUIDE.md               ← Central reference (auto-updated sections)
  ^ referenced by:
  |── CLAUDE.md                       ← Claude Code instructions
  |── .github/copilot-instructions.md ← GitHub Copilot instructions
  └── CODEX.md                        ← OpenAI Codex instructions
```

Auto-generated sections in `docs/AI_AGENT_GUIDE.md` are delimited with `<!-- AUTO-GENERATED -->` / `<!-- END AUTO-GENERATED -->` markers. Manual sections are preserved untouched. Agent-specific files contain only agent-specific setup and a pointer to the central guide.

### Rich Error Messages

Error documentation flows into runtime error output:

```
Error: TypeError at line 15, column 10

  len(42)
      ^^

len() requires String, Array, or Map, got Int

Quick fix: Check the type with type(x) first, or convert with str(x)
See: ntnt docs len
Related: type(), str(), int()
```

---

## CLI Commands

```bash
# Querying
ntnt docs split                       # Full docs for a function
ntnt docs std/string                  # All functions in a module
ntnt docs --examples split            # Just the examples
ntnt docs --search "convert string"   # Full-text search
ntnt docs --related split             # Cross-references
ntnt docs --json split                # JSON output for tooling
ntnt docs --ai-context                # Full dump for AI session start

# Validation
ntnt docs --coverage                  # Documentation completeness report
ntnt docs --test                      # Execute all examples
ntnt docs --orphans                   # Docs without implementation
ntnt docs --diff v0.3.7               # What changed since a version

# Generation
ntnt docs --generate                  # All output formats
ntnt docs --generate --format json    # Specific format
ntnt docs --update-agent-docs         # Regenerate auto-sections in AI_AGENT_GUIDE.md

# REPL (interactive session)
ntnt> :doc split                      # Inline documentation
ntnt> :examples split                 # Show examples
ntnt> :related split                  # Cross-references
ntnt> :search "uppercase"             # Full-text search
```

---

## Migration Plan

### Phase 1: `build.rs` Scanner + Proof of Concept

1. Write the `build.rs` source scanner — parse `/// @ntnt` blocks, extract fields
2. Add doc comments to `std/string` module (24 functions) as proof of concept
3. Validate coverage: scanner detects undocumented `NativeFunction` inserts
4. Verify output matches current `STDLIB_REFERENCE.md` for string section
5. Measure size impact: compare before/after line counts

### Phase 2: Complete Source Documentation

1. Add doc comments to remaining stdlib modules (collections, http, fs, json, math, etc.)
2. Document global builtins (len, print, str, etc.)
3. Document keywords, operators, literals in lexer and parser
4. Document IAL primitives and runtime config (env vars, routing)
5. Document all error types with causes and fixes
6. Delete all TOML doc files

### Phase 3: Database + JSON Output

1. Generate SQLite database with FTS5
2. Generate JSON export
3. Build concept index from module membership and `@see_also` relationships

### Phase 4: Enhanced CLI + REPL

1. `ntnt docs` subcommands: search, related, json, ai-context
2. Validation: coverage, test, orphans, diff
3. REPL: `:doc`, `:examples`, `:related`, `:search`

### Phase 5: Integration

1. Define `<!-- AUTO-GENERATED -->` sections in `docs/AI_AGENT_GUIDE.md`
2. Build step regenerates those sections from doc database
3. Wire error documentation into runtime error formatter
4. Verify agent-specific files reference the central guide

---

## Success Criteria

| Metric | Target |
|--------|--------|
| TOML documentation files | 0 |
| Documentation coverage | 100% (build fails otherwise) |
| Example pass rate | 100% (CI fails otherwise) |
| Files edited to add a function | 1 (the Rust source file) |
| AI accuracy from JSON export | 95%+ of stdlib questions answered correctly |
| Time to find any function | < 3 commands |
| Stale documentation incidents | 0 (build-time enforced) |
| Net source code size increase | < 5% |
| Binary size increase | < 500 KB (~2-5%) |
| `ntnt docs` works without external files | Yes — embedded in binary |

---

## References

- [Elixir ExDoc](https://github.com/elixir-lang/ex_doc) - Docs as first-class citizens, `@doc` attributes, doctests
- [Rust rustdoc](https://doc.rust-lang.org/rustdoc/) - `///` doc comments, doc tests, compile-time validation
- [JSDoc](https://jsdoc.app/) - `@tag` convention for structured doc comments
- [Go godoc](https://pkg.go.dev/golang.org/x/tools/cmd/godoc) - Convention over configuration
- [Zig Autodoc](https://ziglang.org/documentation/0.11.0/) - Interactive single-page web docs
- [TypeScript TSDoc](https://tsdoc.org/) - Standardized doc comment format
