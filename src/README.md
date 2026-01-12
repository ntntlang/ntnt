# NTNT Core Implementation

This directory contains the primary implementation of the NTNT language, currently built as a tree-walking interpreter in Rust.

## Architecture

NTNT follows a standard compiler front-end pipeline, but executes the resulting Abstract Syntax Tree (AST) directly rather than compiling to bytecode or machine code.

1.  **Lexical Analysis ([lexer.rs](lexer.rs))**: Converts source text into a stream of tokens. Handles custom AI keywords like `intent` and `approve` alongside standard language primitives.
2.  **Syntactic Analysis ([parser.rs](parser.rs))**: A recursive-descent parser that transforms the token stream into a structured **AST** ([ast.rs](ast.rs)).
3.  **Type System ([types.rs](types.rs))**: Defines the type environment and effect system. Currently, type checking is integrated into the interpreter flow.
4.  **Contract Enforcement ([contracts.rs](contracts.rs))**: Manages the evaluation of `requires`, `ensures`, and `invariant` blocks, including state capture for `old()` expressions.
5.  **Execution ([interpreter.rs](interpreter.rs))**: The evaluation engine that walks the AST and manages runtime scope, memory, and side-effects (like HTTP or File I/O).

## File Map

| File                             | Purpose                                                     |
| :------------------------------- | :---------------------------------------------------------- |
| [main.rs](main.rs)               | CLI entry point and argument parsing via `clap`.            |
| [lib.rs](lib.rs)                 | Library root; exports core types for testing and embedding. |
| [ast.rs](ast.rs)                 | Data structures for the Abstract Syntax Tree.               |
| [lexer.rs](lexer.rs)             | Tokenizer and scanner logic.                                |
| [parser.rs](parser.rs)           | Recursive descent parser.                                   |
| [interpreter.rs](interpreter.rs) | Tree-walking execution engine.                              |
| [types.rs](types.rs)             | Internal representation of types and generics.              |
| [contracts.rs](contracts.rs)     | Runtime contract validation logic.                          |
| [error.rs](error.rs)             | Unified error handling for all phases.                      |

## Development

### Building

```bash
cargo build
```

### Running Tests

The project maintains high test coverage for both implementation and language behavior.

```bash
cargo test
```

### Running a Script

```bash
cargo run -- run examples/hello.tnt
```

### Inspector Tools

You can inspect the internal representation of any `.tnt` file:

```bash
# See tokens
cargo run -- tokens examples/hello.tnt

# See AST
cargo run -- ast examples/hello.tnt
```
