# NTNT User Code Documentation (.tnt files)

> Doc comments, doctests, and contract-as-documentation for user-written `.tnt` code

## Scope

This document covers how **users of NTNT** document their own `.tnt` code — doc comments, executable examples (doctests), and automatic documentation extraction from contracts and type annotations.

For documenting the NTNT language itself (keywords, stdlib, operators, etc.), see [documentation_system_design.md](documentation_system_design.md).

---

## The Problem

NTNT users have no way to document their functions, structs, or modules:

```ntnt
// This is just a regular comment — invisible to tooling
fn calculate_tax(income: Float, rate: Float) -> Float {
    return income * rate
}
```

There is no:
- Doc comment syntax recognized by the parser
- Way to attach documentation to AST nodes
- `ntnt docs` query for user-defined functions
- Doctest system to verify examples
- Contract-to-documentation extraction
- IDE hover support for user functions

---

## Doc Comment Syntax

### Item Documentation (`///`)

```ntnt
/// Calculates income tax for a given income and rate.
///
/// The rate should be expressed as a decimal (e.g., 0.25 for 25%).
/// Returns the tax amount, which is always non-negative.
///
/// ## Parameters
/// - `income` - Gross income amount
/// - `rate` - Tax rate as decimal (0.0 to 1.0)
///
/// ## Returns
/// The calculated tax amount
///
/// ## Examples
/// ```
/// calculate_tax(100000.0, 0.25)  // => 25000.0
/// calculate_tax(0.0, 0.25)       // => 0.0
/// ```
///
/// @since: v1.0.0
fn calculate_tax(income: Float, rate: Float) -> Float
    requires rate >= 0.0
    requires rate <= 1.0
    requires income >= 0.0
    ensures result >= 0.0
{
    return income * rate
}
```

### Module Documentation (`//!`)

```ntnt
//! Tax calculation utilities for the accounting module.
//!
//! This module provides functions for computing various tax amounts
//! based on income brackets and applicable rates.
//!
//! ## Usage
//! ```
//! import { calculate_tax, tax_bracket } from "./tax"
//! let tax = calculate_tax(75000.0, tax_bracket(75000.0))
//! ```

fn calculate_tax(income: Float, rate: Float) -> Float { ... }
fn tax_bracket(income: Float) -> Float { ... }
```

### Struct Documentation

```ntnt
/// A user account with authentication credentials.
///
/// ## Fields
/// - `email` - The user's email address (used for login)
/// - `name` - Display name
/// - `role` - Access role (admin, user, guest)
///
/// ## Invariants
/// - Email must contain '@'
/// - Name must not be empty
struct User {
    email: String,
    name: String,
    role: String
}

impl User {
    invariant contains(self.email, "@")
    invariant len(self.name) > 0
}
```

### Metadata Annotations

| Annotation | Purpose | Example |
|------------|---------|---------|
| `@since: v1.0.0` | Version when added | `/// @since: v1.0.0` |
| `@deprecated: Use X instead` | Mark as deprecated | `/// @deprecated: Use calculate_tax_v2` |
| `@internal` | Hide from public docs | `/// @internal` |
| `@see: function_name` | Cross-reference | `/// @see: tax_bracket` |
| `@implements: feature.X` | Link to intent feature | `/// @implements: feature.tax_calc` |

---

## Doctests

Code blocks in doc comments are extracted and executed as tests.

### Basic Doctests

```ntnt
/// Doubles a number.
///
/// ```
/// double(5)  // => 10
/// double(0)  // => 0
/// double(-3) // => -6
/// ```
fn double(x: Int) -> Int {
    return x * 2
}
```

### Doctest Syntax

```ntnt
/// ```
/// expression  // => expected_result
/// ```
```

The `// =>` comment indicates the expected return value. The doctest runner evaluates the expression and compares against the expected result.

### Multi-Statement Doctests

```ntnt
/// ```
/// let users = fetch_users()
/// assert(len(users) > 0)
/// assert(users[0].name == "Alice")
/// ```
```

When there is no `// =>`, the doctest passes if it runs without error.

### Skip and Error Examples

```ntnt
/// ```ntnt,skip
/// // This example is for illustration only — not executed
/// let db = connect("postgres://production-server/...")
/// ```

/// ```ntnt,should_panic
/// // This example demonstrates an error case
/// divide(10, 0)
/// ```
```

### Running Doctests

```bash
ntnt doctest myfile.tnt
#   Running doctests for myfile.tnt...
#     double:
#       Example 1 (line 4)... ok
#       Example 2 (line 5)... ok
#       Example 3 (line 6)... ok
#     calculate_tax:
#       Example 1 (line 15)... ok
#
#   4 doctests passed

ntnt doctest myfile.tnt --verbose
#   Shows actual vs expected for each test

ntnt doctest .
#   Run doctests for all .tnt files in current directory
```

---

## Contract-as-Documentation

NTNT contracts (`requires`, `ensures`, `invariant`) automatically become documentation sections. No duplication needed.

### Function Contracts

```ntnt
fn withdraw(account: BankAccount, amount: Int) -> BankAccount
    requires amount > 0
    requires account.balance >= amount
    ensures result.balance == old(account.balance) - amount
    ensures result.balance >= 0
{
    return BankAccount { balance: account.balance - amount, owner: account.owner }
}
```

Generated documentation:

> **withdraw**(account: BankAccount, amount: Int) -> BankAccount
>
> **Preconditions:**
> - `amount` must be greater than 0
> - `account.balance` must be at least `amount`
>
> **Postconditions:**
> - Result balance equals original balance minus `amount`
> - Result balance is non-negative

### Struct Invariants

```ntnt
struct BoundedCounter {
    value: Int,
    min: Int,
    max: Int
}

impl BoundedCounter {
    invariant self.value >= self.min
    invariant self.value <= self.max
}
```

Generated documentation:

> **BoundedCounter**
>
> **Invariants:**
> - `value` is always at least `min`
> - `value` never exceeds `max`

---

## Type-Derived Documentation

Type signatures and annotations automatically enrich documentation.

```ntnt
fn parse_config(path: String) -> Result<Config, String>
```

Generated documentation automatically includes:

> **Returns:** `Result<Config, String>` — either a `Config` on success or an error message string.
>
> **Usage pattern:**
> ```ntnt
> match parse_config("app.toml") {
>     Ok(config) => use_config(config),
>     Err(msg) => print("Config error: {msg}")
> }
> ```

---

## Parser Implementation

### AST Changes

```rust
// In ast.rs

pub struct DocComment {
    pub summary: String,                // First paragraph (one-liner)
    pub body: String,                   // Full markdown body
    pub params: Vec<ParamDoc>,          // Extracted from ## Parameters
    pub returns: Option<String>,        // Extracted from ## Returns
    pub examples: Vec<DocExample>,      // Extracted from ``` blocks
    pub errors: Vec<String>,            // Extracted from ## Errors
    pub implements: Vec<String>,        // @implements annotations
    pub since: Option<String>,          // @since
    pub deprecated: Option<String>,     // @deprecated
    pub see_also: Vec<String>,          // @see
    pub internal: bool,                 // @internal
}

pub struct DocExample {
    pub code: String,
    pub expected: Option<String>,       // The // => value
    pub line: usize,
    pub skip: bool,                     // ```ntnt,skip
    pub should_panic: bool,             // ```ntnt,should_panic
}

// Added to existing AST nodes:
pub struct Function {
    pub name: String,
    pub params: Vec<Param>,
    pub return_type: Option<TypeExpr>,
    pub body: Block,
    pub contracts: Vec<Contract>,
    pub doc: Option<DocComment>,        // NEW
}

pub struct Struct {
    pub name: String,
    pub fields: Vec<Field>,
    pub doc: Option<DocComment>,        // NEW
}
```

### Lexer Changes

The lexer recognizes `///` and `//!` as distinct token types:

```rust
// In lexer.rs
DocComment(String),       // /// ...
ModuleDocComment(String), // //! ...
```

Multiple consecutive doc comment lines are merged into a single token.

### Parser Changes

The parser attaches doc comments to the following declaration:

```rust
// In parser.rs
fn parse_function(&mut self) -> Result<Statement, ParseError> {
    let doc = self.consume_doc_comment(); // Collect preceding /// lines
    // ... parse fn keyword, name, params, etc.
    Ok(Statement::Function { name, params, body, doc })
}
```

---

## CLI Integration

### Querying User Docs

```bash
# Show docs for a function in a .tnt file
ntnt docs myfile.tnt calculate_tax

# Show all documented items in a file
ntnt docs myfile.tnt

# JSON output for tooling
ntnt docs myfile.tnt --json
```

### Doctest Execution

```bash
ntnt doctest myfile.tnt          # Run doctests in one file
ntnt doctest .                   # Run doctests in all .tnt files
ntnt doctest myfile.tnt -v       # Verbose output
```

### Inspect Integration

`ntnt inspect` already outputs project structure as JSON. With doc comments, it includes documentation:

```bash
ntnt inspect myfile.tnt --pretty
```

```json
{
  "functions": [
    {
      "name": "calculate_tax",
      "params": [
        {"name": "income", "type": "Float"},
        {"name": "rate", "type": "Float"}
      ],
      "return_type": "Float",
      "doc": {
        "summary": "Calculates income tax for a given income and rate.",
        "params": [
          {"name": "income", "description": "Gross income amount"},
          {"name": "rate", "description": "Tax rate as decimal (0.0 to 1.0)"}
        ]
      },
      "contracts": {
        "requires": ["rate >= 0.0", "rate <= 1.0", "income >= 0.0"],
        "ensures": ["result >= 0.0"]
      }
    }
  ]
}
```

---

## Implementation Phases

### Phase 1: Lexer + Parser

1. Add `DocComment` and `ModuleDocComment` token types
2. Merge consecutive `///` lines into single token
3. Attach doc comments to Function, Struct, Enum AST nodes
4. Add `doc: Option<DocComment>` to relevant AST types

### Phase 2: `ntnt docs` for .tnt Files

1. Parse doc comments into `DocComment` struct
2. Extract `## Parameters`, `## Returns`, `## Examples` sections
3. Parse metadata annotations (`@since`, `@deprecated`, etc.)
4. `ntnt docs <file> <function>` displays formatted output

### Phase 3: Doctest Runner

1. Extract code blocks from doc comments
2. Parse `// =>` expected values
3. Execute in isolated environment
4. Report pass/fail with line numbers
5. `ntnt doctest` command

### Phase 4: Contract Extraction

1. Convert `requires` clauses to "Preconditions" documentation
2. Convert `ensures` clauses to "Postconditions" documentation
3. Convert `invariant` clauses to "Invariants" documentation
4. Include in `ntnt docs` output and `ntnt inspect` JSON

### Phase 5: Integration

1. Include doc comments in `ntnt inspect` JSON output
2. Wire into intent system (`@implements` validation)
3. Future: LSP hover support for user functions

---

## Success Criteria

| Metric | Target |
|--------|--------|
| Doc comment parsing | `///` and `//!` recognized on all declarations |
| Doctest execution | All `// =>` examples verified |
| Contract extraction | All contracts appear in generated docs |
| Inspect integration | Doc comments in JSON output |
| No runtime overhead | Doc comments stripped at parse time for execution |

---

## References

- [Elixir Doctests](https://hexdocs.pm/elixir/writing-documentation.html) - `iex>` examples executed as tests
- [Rust Doc Tests](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html) - Code blocks compiled and run
- [Python Doctest](https://docs.python.org/3/library/doctest.html) - Interactive examples as tests
- [Zig Doc Comments](https://ziglang.org/documentation/0.11.0/) - `///` and `//!` syntax
