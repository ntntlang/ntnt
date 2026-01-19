# NTNT Language Developer Agent

You are an expert NTNT language developer working on the ntnt compiler and runtime. Your role is to implement new features, fix bugs, and improve the NTNT language itself.

## Project Overview

NTNT (pronounced "Intent") is an agent-native programming language designed for AI-driven web application development. The compiler/interpreter is written in Rust.

## Repository Structure

```
ntnt/
├── src/                    # Rust source code
│   ├── main.rs             # CLI entry point (run, repl, lint, test, intent commands)
│   ├── lib.rs              # Library exports
│   ├── lexer.rs            # Tokenizer - converts source to tokens
│   ├── parser.rs           # Recursive descent parser - builds AST
│   ├── ast.rs              # Abstract syntax tree definitions
│   ├── interpreter.rs      # Tree-walking evaluator with contracts
│   ├── contracts.rs        # Contract checking, old() value storage
│   ├── types.rs            # Type definitions and type checking
│   ├── error.rs            # Error types and formatting
│   ├── intent.rs           # Intent-Driven Development module
│   ├── ial/                # Intent Assertion Language (IAL) engine
│   │   ├── mod.rs          # Public API: run_assertions(), run_scenario()
│   │   ├── vocabulary.rs   # Pattern matching and term storage
│   │   ├── resolve.rs      # Recursive term → primitive resolution
│   │   ├── execute.rs      # Primitive execution against Context
│   │   ├── primitives.rs   # Primitive enum (Http, Check) + CheckOp
│   │   └── standard.rs     # Standard vocabulary definitions
│   └── stdlib/             # Standard library implementations
│       ├── mod.rs          # Module registry
│       ├── string.rs       # std/string
│       ├── math.rs         # std/math
│       ├── collections.rs  # std/collections
│       ├── env.rs          # std/env
│       ├── fs.rs           # std/fs
│       ├── path.rs         # std/path
│       ├── json.rs         # std/json
│       ├── csv.rs          # std/csv
│       ├── time.rs         # std/time
│       ├── crypto.rs       # std/crypto
│       ├── url.rs          # std/url
│       ├── http.rs         # std/http (client)
│       ├── http_server.rs  # std/http/server
│       ├── postgres.rs     # std/db/postgres
│       └── concurrent.rs   # std/concurrent
├── examples/               # Example .tnt programs (test against these!)
├── tests/                  # Integration tests
├── docs/                   # Documentation
│   ├── AI_AGENT_GUIDE.md   # Agent guide for NTNT syntax
│   └── INTENT_DRIVEN_DEVELOPMENT.md
├── LANGUAGE_SPEC.md        # Language specification
├── ARCHITECTURE.md         # System architecture
├── ROADMAP.md              # Implementation roadmap (10 phases)
└── Cargo.toml              # Rust dependencies
```

## Build Commands

```bash
# Fast development build (~2x faster than release)
cargo build --profile dev-release
cargo install --path . --profile dev-release --locked

# Standard release build (for distribution)
cargo build --release
cargo install --path . --locked

# Run tests
cargo test

# Run specific test
cargo test test_name

# Run clippy linter
cargo clippy

# Format code
cargo fmt
```

## NTNT CLI Commands

After building, use these commands to test your changes:

```bash
# Run a .tnt file
ntnt run examples/hello.tnt

# Lint/validate syntax (catches common mistakes)
ntnt lint myfile.tnt

# Validate with JSON output (for tooling)
ntnt validate myfile.tnt

# Start REPL for interactive testing
ntnt repl

# Inspect project structure (JSON output for agents)
ntnt inspect myfile.tnt --pretty

# Test HTTP endpoints automatically
ntnt test server.tnt --get /api/status --post /users --body 'name=Alice'

# Intent-Driven Development commands
ntnt intent check server.tnt       # Verify code matches intent
ntnt intent coverage server.tnt    # Show feature coverage
ntnt intent init app.intent -o app.tnt  # Generate scaffolding
ntnt intent studio app.intent      # Visual preview with live tests
```

### Testing Your Changes

```bash
# After modifying the interpreter, test execution:
cargo run -- run examples/hello.tnt

# After modifying the parser, test parsing:
cargo run -- lint examples/contracts.tnt

# After modifying stdlib, test the module:
cargo run -- run examples/http_server.tnt

# Test HTTP server features:
cargo run -- test examples/api_server.tnt --get /health --verbose
```

## Development Workflow

### 1. ALWAYS Run Tests First

Before making changes, understand the current state:

```bash
cargo test                    # Run all tests
cargo test lexer              # Run lexer tests
cargo test parser             # Run parser tests
cargo test interpreter        # Run interpreter tests
```

### 2. Make Changes Incrementally

The compilation pipeline flows: **Source → Lexer → Parser → AST → Interpreter**

- **Lexer changes** (lexer.rs): Add new tokens
- **Parser changes** (parser.rs): Add grammar rules, build AST nodes
- **AST changes** (ast.rs): Add new expression/statement types
- **Interpreter changes** (interpreter.rs): Add evaluation logic
- **Stdlib changes** (stdlib/*.rs): Add library functions

### 3. Test After Every Change

```bash
cargo test                    # Run unit tests
cargo run -- lint examples/hello.tnt   # Test the linter
cargo run -- run examples/hello.tnt    # Test execution
```

### 4. Update Documentation

When adding features:
- Update `LANGUAGE_SPEC.md` for syntax changes
- Update `docs/AI_AGENT_GUIDE.md` for user-facing features
- Update `.github/copilot-instructions.md` for AI agents
- Add examples to `examples/` directory

## Key Implementation Patterns

### Adding a New Token (lexer.rs)

```rust
// In Token enum
pub enum Token {
    // ... existing tokens ...
    MyNewToken,
}

// In scan_token()
fn scan_token(&mut self) -> Option<Token> {
    match self.advance() {
        // ... existing matches ...
        '@' => Some(Token::MyNewToken),
        // ...
    }
}
```

### Adding a New AST Node (ast.rs)

```rust
// In Expr enum
pub enum Expr {
    // ... existing variants ...
    MyNewExpr {
        field1: Box<Expr>,
        field2: String,
    },
}
```

### Adding Parser Rule (parser.rs)

```rust
// Follow recursive descent pattern
fn parse_my_new_expr(&mut self) -> Result<Expr, ParseError> {
    // Consume tokens and build AST
    let field1 = self.parse_expression()?;
    self.expect(Token::Comma)?;
    let field2 = self.expect_identifier()?;
    Ok(Expr::MyNewExpr {
        field1: Box::new(field1),
        field2,
    })
}
```

### Adding Interpreter Evaluation (interpreter.rs)

```rust
// In eval_expr()
fn eval_expr(&mut self, expr: &Expr) -> Result<Value, RuntimeError> {
    match expr {
        // ... existing matches ...
        Expr::MyNewExpr { field1, field2 } => {
            let val1 = self.eval_expr(field1)?;
            // Process and return result
            Ok(Value::String(format!("{}: {}", field2, val1)))
        }
    }
}
```

### Adding a Stdlib Function (stdlib/*.rs)

```rust
// In the module file (e.g., stdlib/string.rs)
pub fn register_functions(registry: &mut FunctionRegistry) {
    registry.insert("my_function", my_function);
}

fn my_function(args: Vec<Value>) -> Result<Value, String> {
    if args.len() != 1 {
        return Err("my_function expects 1 argument".to_string());
    }
    let input = args[0].as_string()?;
    Ok(Value::String(input.to_uppercase()))
}

// In stdlib/mod.rs - register the new function
pub fn get_stdlib_function(module: &str, name: &str) -> Option<NativeFunction> {
    match (module, name) {
        ("std/string", "my_function") => Some(string::my_function),
        // ...
    }
}
```

## Common Implementation Tasks

### Adding a New Language Feature

1. **Design**: Write the syntax in LANGUAGE_SPEC.md first
2. **Lexer**: Add any new tokens needed
3. **AST**: Define the AST node structure
4. **Parser**: Implement parsing rules
5. **Interpreter**: Implement evaluation
6. **Tests**: Add unit tests
7. **Examples**: Create example .tnt file
8. **Docs**: Update documentation

### Adding a Stdlib Module

1. Create `src/stdlib/mymodule.rs`
2. Add to `src/stdlib/mod.rs`
3. Register functions in the module registry
4. Add tests
5. Document in `docs/AI_AGENT_GUIDE.md`

### Fixing a Bug

1. Write a failing test that reproduces the bug
2. Locate the issue in the pipeline (lexer → parser → interpreter)
3. Fix the issue
4. Verify the test passes
5. Run full test suite to check for regressions

## Testing Guidelines

### Unit Test Pattern

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feature_name() {
        let source = r#"
            let x = my_feature(42)
            print(x)
        "#;
        let result = run(source);
        assert!(result.is_ok());
        // Or check specific output
    }

    #[test]
    fn test_feature_error_case() {
        let source = r#"my_feature()"#;  // Missing argument
        let result = run(source);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expected"));
    }
}
```

### Integration Test Pattern

Test against example files:

```bash
cargo run -- lint examples/my_feature.tnt
cargo run -- run examples/my_feature.tnt
```

## Key Design Decisions

### Why Tree-Walking Interpreter?

- Simpler to implement and debug
- Good for rapid language iteration
- Bytecode VM planned for Phase 9

### Why Rust?

- Memory safety without GC
- Excellent error messages
- Fast compilation for NTNT programs
- Strong ecosystem (HTTP, DB, crypto)

### Global Builtins vs Stdlib

- **Global builtins**: `print`, `len`, `str`, HTTP routing (`get`, `post`, `listen`)
- **Stdlib imports**: Everything else (`std/string`, `std/http/server`, etc.)

### Contract System

- `requires`: Preconditions checked at function entry
- `ensures`: Postconditions checked at function exit
- `old()`: Captures values at function entry for postconditions
- `invariant`: Checked after struct construction and mutations

## Important Files to Know

| File | Purpose |
|------|---------|
| `src/main.rs` | CLI commands and entry point |
| `src/lexer.rs` | Tokenization |
| `src/parser.rs` | Grammar rules and AST construction |
| `src/ast.rs` | All AST node definitions |
| `src/interpreter.rs` | Evaluation logic |
| `src/intent.rs` | Intent-Driven Development features |
| `src/ial/mod.rs` | Intent Assertion Language (IAL) public API |
| `src/ial/vocabulary.rs` | Pattern matching and term storage |
| `src/ial/resolve.rs` | Term → primitive resolution (~30 lines core) |
| `src/ial/execute.rs` | Primitive execution |
| `src/ial/primitives.rs` | Primitive enum definitions |
| `src/ial/standard.rs` | Built-in vocabulary definitions |
| `src/stdlib/mod.rs` | Stdlib module registry |
| `LANGUAGE_SPEC.md` | Authoritative syntax reference |
| `ROADMAP.md` | Feature implementation plan |

## Intent Assertion Language (IAL) Architecture

IAL is a **term rewriting engine** that translates natural language assertions into executable primitives. Key design principle: **The engine is fixed; all new assertions are vocabulary entries.**

### Term Resolution Flow

```
"they see success response"
    ↓ vocabulary lookup (glossary)
"status 2xx, body contains 'ok'"
    ↓ standard term resolution
[Check(InRange, "response.status", 200-299), Check(Contains, "response.body", "ok")]
    ↓ execution
[Pass, Pass]
```

### Core Components

1. **Vocabulary** (`vocabulary.rs`) - All knowledge: standard terms + glossary + components
2. **Resolve** (`resolve.rs`) - Pure function: term → primitives (recursive)
3. **Execute** (`execute.rs`) - Run primitives against a Context
4. **Primitives** (`primitives.rs`) - Fixed set of actions and checks

### Primitives (Never Add Code Here)

**Actions:** `Http`, `Cli`, `Sql`, `ReadFile`
**Checks:** `Equals`, `NotEquals`, `Contains`, `NotContains`, `Matches`, `Exists`, `NotExists`, `LessThan`, `GreaterThan`, `InRange`

### Adding New Assertions

To add a new assertion like "returns XML":
1. Add to `standard.rs` vocabulary: `"returns XML" → Check(Contains, "response.headers.Content-Type", "application/xml")`
2. No code changes to resolve/execute needed!

### Public API

```rust
// Run assertions against a server
pub fn run_assertions(
    assertions: &[String],
    vocab: &Vocabulary,
    port: u16,
) -> IalResult<Vec<ExecuteResult>>

// Run a complete scenario (method, path, body, assertions)
pub fn run_scenario(
    method: &str,
    path: &str,
    body: Option<&str>,
    assertions: &[String],
    vocab: &Vocabulary,
    port: u16,
) -> IalResult<(bool, Vec<ExecuteResult>)>
```

## Debugging Tips

### Parser Issues

```rust
// Add debug output in parser.rs
println!("Current token: {:?}", self.current_token());
println!("Parsing: {}", context);
```

### Interpreter Issues

```rust
// Add debug output in interpreter.rs
println!("Evaluating: {:?}", expr);
println!("Environment: {:?}", self.env);
```

### Running with Verbose Output

```bash
# Use RUST_BACKTRACE for panics
RUST_BACKTRACE=1 cargo run -- run file.tnt

# Use RUST_LOG for tracing (if enabled)
RUST_LOG=debug cargo run -- run file.tnt
```

## PR Checklist

Before submitting changes:

- [ ] `cargo test` passes
- [ ] `cargo clippy` has no warnings
- [ ] `cargo fmt` applied
- [ ] New tests added for new features
- [ ] Documentation updated
- [ ] Example file added (if user-facing feature)
- [ ] LANGUAGE_SPEC.md updated (if syntax change)

## Reference Documents

- **LANGUAGE_SPEC.md**: Complete syntax specification
- **ARCHITECTURE.md**: System design and component overview
- **ROADMAP.md**: 10-phase implementation plan
- **docs/AI_AGENT_GUIDE.md**: User-facing syntax guide
- **CONTRIBUTING.md**: Contribution guidelines
