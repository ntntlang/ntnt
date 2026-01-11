# Intent Programming Language

Intent is a revolutionary programming language and ecosystem designed specifically for AI-driven development. Unlike traditional languages built for human developers, Intent empowers AI agents as primary software creators while maintaining deep human oversight and collaboration.

**Goal: Build production-ready web applications and APIs with AI-powered development and runtime safety guarantees.**

## Current Status

**Phase 1: Core Contract System** ✅ Complete

- Function contracts (`requires`, `ensures`)
- `old()` function for pre-state capture
- `result` keyword in postconditions
- Struct invariants with automatic enforcement

**Phase 2: Type System & Pattern Matching** ✅ Complete

- Algebraic Data Types (enums with associated data)
- `Option<T>` and `Result<T, E>` built-ins
- Pattern matching with `match` expressions
- Generic functions and types
- Type aliases and union types

**Phase 3: Module System & Standard Library** ✅ Complete

- File-based modules with `import`/`export`
- Module aliasing: `import "std/math" as math`
- Selective imports: `import { split, join } from "std/string"`
- Standard library: `std/string`, `std/math`, `std/collections`, `std/env`

**85 passing tests** | **Version 0.1.2**

**Next Up**: Traits & Interfaces (Phase 4)

See [ROADMAP.md](ROADMAP.md) for the full 13-phase implementation plan.

## Quick Start

```bash
# Clone and build
git clone https://github.com/joshcramer/intent.git
cd intent
cargo build --release

# Run a program
./target/release/intent examples/contracts_full.intent

# Start the REPL
./target/release/intent repl
```

## File Extensions

Intent supports two file extensions:

- `.intent` - Standard Intent source files
- `.itn` - Short form (convenient for quick scripts)

## Overview

Intent reimagines software development for an era where AI agents handle the heavy lifting of coding, testing, and deployment. The language features:

- **First-Class Contracts**: Design-by-contract principles built into the syntax for guaranteed correctness
- **Runtime Safety**: Struct invariants and pre/post conditions enforced at runtime
- **Built-in Functions**: Math utilities (`abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `sign`, `clamp`)
- **Typed Error Effects**: Explicit error handling and failure conditions
- **Semantic Versioning**: Automatic API compatibility management
- **Structured Edits**: AST-based code manipulation for safe refactoring
- **Multi-Agent Collaboration**: Built-in support for AI agents working together
- **Human-in-the-Loop Governance**: Transparent decision-making with human approval gates

### Production Roadmap

Intent is being developed toward production web application capabilities:

- **Phase 5**: HTTP server with contract-verified endpoints
- **Phase 6**: Database access with repository patterns
- **Phase 7**: Async/await for concurrent operations
- **Phase 11**: Docker deployment and container support

Performance targets: <1ms contract overhead, >10k requests/sec

## Example

```intent
struct BankAccount {
    balance: Int,
    owner: String
}

impl BankAccount {
    // Invariant: balance can never go negative
    invariant self.balance >= 0
}

fn withdraw(account: BankAccount, amount: Int) -> Bool
    requires amount > 0
    ensures result == true implies account.balance == old(account.balance) - amount
{
    if account.balance >= amount {
        account.balance = account.balance - amount
        return true
    }
    return false
}
```

## Option & Result Types

Intent provides built-in `Option<T>` and `Result<T, E>` types for safe handling of nullable values and errors:

```intent
// Option type for nullable values
let maybe_value = Some(42);
let nothing = None;

// Check and unwrap
if is_some(maybe_value) {
    print(unwrap(maybe_value));  // 42
}

// Safe default
let value = unwrap_or(nothing, 0);  // 0

// Result type for error handling
let success = Ok(100);
let failure = Err("something went wrong");

if is_ok(success) {
    print(unwrap(success));  // 100
}
```

## Pattern Matching

Use `match` expressions for powerful pattern matching:

```intent
fn describe_option(opt) {
    match opt {
        Some(v) => print("Got value: " + v),
        None => print("No value")
    }
}

// Match on literals
fn describe_number(n) {
    match n {
        0 => "zero",
        1 => "one",
        _ => "many"
    }
}

// Match on enums with data
enum Shape {
    Circle(Float),
    Rectangle(Float, Float)
}

fn area(shape) {
    match shape {
        Shape::Circle(r) => 3.14159 * r * r,
        Shape::Rectangle(w, h) => w * h
    }
}
```

## Enums

Define custom enumerated types with optional associated data:

```intent
// Simple enum
enum Status {
    Pending,
    Active,
    Completed
}

let current = Status::Active;

// Enum with data
enum Message {
    Text(String),
    Number(Int),
    Pair(Int, Int)
}

let msg = Message::Text("hello");
```

## Generics

Generic functions and types enable reusable code:

```intent
// Generic function
fn identity<T>(x: T) -> T {
    return x;
}

identity(42);      // works with Int
identity("hello"); // works with String

// Generic struct
struct Stack<T> {
    items: [T]
}

// Type aliases
type UserId = Int;
type StringMap<V> = Map<String, V>;
```

## Union Types

Union types allow a value to be one of several types:

```intent
// Function accepting multiple types
fn stringify(value: String | Int | Bool) -> String {
    return value;  // Will be converted to string
}

stringify("hello");  // works
stringify(42);       // works
stringify(true);     // works

// Useful for flexible APIs
fn process(input: String | [String]) {
    // Handle both single string and array of strings
}
```

## Effect Annotations

Mark functions with their side effects:

```intent
// Function with IO effect
fn read_config(path: String) -> String with io {
    // ... performs file I/O
}

// Pure function (no side effects)
fn add(a: Int, b: Int) -> Int pure {
    return a + b;
}
```

## Module System

Intent features a powerful module system for organizing code:

```intent
// Import specific functions
import { split, join, trim } from "std/string"

// Import entire module with alias
import "std/math" as math

// Use imported functions
let words = split("hello world", " ")
let angle = math.sin(math.PI / 2)

// Import from local files
import { helper } from "./utils"
```

### Standard Library Modules

**std/string** - String manipulation

```intent
import { split, join, trim, replace, contains } from "std/string"
import { starts_with, ends_with, to_upper, to_lower } from "std/string"
import { char_at, substring } from "std/string"

let text = "  Hello, World!  "
let trimmed = trim(text)              // "Hello, World!"
let parts = split(trimmed, ", ")      // ["Hello", "World!"]
let upper = to_upper("hello")         // "HELLO"
let has_hello = contains(text, "Hello") // true
```

**std/math** - Mathematical functions and constants

```intent
import "std/math" as math

// Constants
math.PI    // 3.141592653589793
math.E     // 2.718281828459045

// Trigonometry
math.sin(x)   math.cos(x)   math.tan(x)
math.asin(x)  math.acos(x)  math.atan(x)
math.atan2(y, x)

// Logarithms and exponentials
math.log(x)    // Natural log
math.log10(x)  // Base-10 log
math.exp(x)    // e^x
```

**std/collections** - Array utilities

```intent
import { push, pop, first, last, reverse, slice, concat, is_empty } from "std/collections"

let arr = [1, 2, 3]
let arr2 = push(arr, 4)        // [1, 2, 3, 4]
let rev = reverse(arr)         // [3, 2, 1]
let sub = slice(arr2, 1, 3)    // [2, 3]

match first(arr) {
    Some(v) => print("First: " + str(v)),
    None => print("Empty array")
}
```

**std/env** - Environment access

```intent
import { get_env, args, cwd } from "std/env"

let path = cwd()               // Current working directory
let argv = args()              // Command line arguments

match get_env("HOME") {
    Some(home) => print("Home: " + home),
    None => print("HOME not set")
}
```

## Editor Support

### VS Code

Install the Intent Language extension for syntax highlighting:

```bash
cp -r editors/vscode/intent-lang ~/.vscode/extensions/
```

Then restart VS Code. The extension provides:

- Syntax highlighting for `.intent` and `.itn` files
- Code snippets for common patterns
- Bracket matching and auto-closing

## Vision

Intent bridges the gap between AI's speed and consistency with human judgment and design sense. The ecosystem includes:

- Integrated development workflows (CI/CD, reviews, pull requests)
- Rich observability and explainability features
- Formal concurrency protocols
- UI/UX constraint declarations
- Intent encoding for self-documenting code

## Documentation

- [Whitepaper](whitepaper.md) - Complete technical specification and motivation
- [Architecture](ARCHITECTURE.md) - System design and components
- [Language Spec](LANGUAGE_SPEC.md) - Formal language definition
- [Roadmap](ROADMAP.md) - 13-phase implementation plan for production web apps

## Built-in Functions

### Math Functions

| Function             | Description               | Example                   |
| -------------------- | ------------------------- | ------------------------- |
| `abs(x)`             | Absolute value            | `abs(-5)` → `5`           |
| `min(a, b)`          | Minimum of two values     | `min(3, 7)` → `3`         |
| `max(a, b)`          | Maximum of two values     | `max(3, 7)` → `7`         |
| `round(x)`           | Round to nearest integer  | `round(3.7)` → `4`        |
| `floor(x)`           | Round down                | `floor(3.7)` → `3`        |
| `ceil(x)`            | Round up                  | `ceil(3.2)` → `4`         |
| `sqrt(x)`            | Square root               | `sqrt(16)` → `4`          |
| `pow(base, exp)`     | Exponentiation            | `pow(2, 3)` → `8`         |
| `sign(x)`            | Sign of number (-1, 0, 1) | `sign(-5)` → `-1`         |
| `clamp(x, min, max)` | Clamp to range            | `clamp(15, 0, 10)` → `10` |

### Option & Result Functions

| Function                  | Description                    | Example                       |
| ------------------------- | ------------------------------ | ----------------------------- |
| `Some(value)`             | Create Option with value       | `Some(42)` → `Some(42)`       |
| `None`                    | Create empty Option            | `None` → `None`               |
| `Ok(value)`               | Create success Result          | `Ok(100)` → `Ok(100)`         |
| `Err(error)`              | Create error Result            | `Err("fail")` → `Err("fail")` |
| `is_some(opt)`            | Check if Option has value      | `is_some(Some(1))` → `true`   |
| `is_none(opt)`            | Check if Option is empty       | `is_none(None)` → `true`      |
| `is_ok(result)`           | Check if Result is Ok          | `is_ok(Ok(1))` → `true`       |
| `is_err(result)`          | Check if Result is Err         | `is_err(Err("x"))` → `true`   |
| `unwrap(opt)`             | Get value (panics if None/Err) | `unwrap(Some(42))` → `42`     |
| `unwrap_or(opt, default)` | Get value or default           | `unwrap_or(None, 0)` → `0`    |

### I/O Functions

| Function          | Description            |
| ----------------- | ---------------------- |
| `print(args...)`  | Print to stdout        |
| `str(value)`      | Convert to string      |
| `len(collection)` | Length of string/array |

### Standard Library (import required)

| Module            | Functions                                                      |
| ----------------- | -------------------------------------------------------------- |
| `std/string`      | split, join, trim, replace, contains, starts_with, ends_with   |
|                   | to_upper, to_lower, char_at, substring                         |
| `std/math`        | sin, cos, tan, asin, acos, atan, atan2, log, log10, exp, PI, E |
| `std/collections` | push, pop, first, last, reverse, slice, concat, is_empty       |
| `std/env`         | get_env, args, cwd                                             |

## Contributing

Intent is an open-source project welcoming contributions from developers, language designers, and AI researchers. See our contributing guidelines for details.

## License

MIT

## Contact

For questions or discussions, please open an issue on this repository.
