# Intent Language Specification

## Version 0.1 (Draft)

This document specifies the syntax, semantics, and core features of the Intent programming language.

## Table of Contents

1. [Lexical Structure](#lexical-structure)
2. [Types](#types)
3. [Contracts](#contracts)
4. [Functions and Methods](#functions-and-methods)
5. [Built-in Functions](#built-in-functions)
6. [Effects](#effects)
7. [Concurrency](#concurrency)
8. [Modules](#modules)

## Lexical Structure

### Keywords

```
contract, requires, ensures, invariant, effect, protocol, intent, approve
fn, let, mut, if, else, match, loop, while, return, break, continue
type, struct, enum, impl, use, mod, pub
```

### Identifiers

- Start with letter or underscore
- Contain letters, digits, underscores
- Case-sensitive

### Literals

- Integers: `42`, `0x2A`, `0b101010`
- Floats: `3.14`, `1.0e-10`
- Strings: `"hello"`, `'single'`
- Booleans: `true`, `false`
- Arrays: `[1, 2, 3]`
- Objects: `{key: value}`

## Types

### Primitive Types

- `Int`: Arbitrary precision integers
- `Float`: IEEE 754 floating point
- `Bool`: Boolean values
- `String`: UTF-8 encoded text
- `Unit`: The unit type `()`

### Compound Types

- Arrays: `[T]`
- Tuples: `(T1, T2, ...)`
- Structs: Named product types
- Enums: Tagged union types
- Functions: `(T1, T2) -> T3`

### Type Annotations

```intent
let x: Int = 42;
let name: String = "Intent";
```

## Contracts

Contracts specify behavioral requirements for code. Intent enforces contracts at runtime with detailed error messages.

### Function Contracts

The `requires` clause specifies preconditions that must be true when a function is called.
The `ensures` clause specifies postconditions that must be true when a function returns.

```intent
fn transfer_funds(amount: Int, from: Account, to: Account) -> Result<(), Error>
requires amount > 0 && from.balance >= amount
ensures to.balance == old(to.balance) + amount
{
    // implementation
}
```

### The `old()` Function

The `old()` function captures the value of an expression at function entry, allowing postconditions to compare pre-state and post-state:

```intent
fn increment(counter: Counter)
ensures counter.value == old(counter.value) + 1
{
    counter.value = counter.value + 1
}
```

### The `result` Keyword

In postconditions, `result` refers to the return value of the function:

```intent
fn double(x: Int) -> Int
ensures result == x * 2
{
    return x * 2
}
```

### Conditional Postconditions

Use `implies` for conditional guarantees:

```intent
fn safe_divide(a: Int, b: Int) -> Int
requires b != 0
ensures b > 0 implies result >= 0
{
    return a / b
}
```

### Struct Invariants

Invariants are automatically checked after construction and after any method call or field assignment:

```intent
struct Account {
    balance: Int,
    owner: String
}

impl Account {
    invariant self.balance >= 0
}
```

## Functions and Methods

### Function Definition

```intent
fn add(x: Int, y: Int) -> Int {
    return x + y;
}
```

### Methods

```intent
impl Point {
    fn distance(&self, other: &Point) -> Float {
        // implementation
    }
}
```

## Built-in Functions

Intent provides built-in functions available without imports.

### I/O Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `print` | `(...args) -> Unit` | Print values to stdout |
| `len` | `(collection) -> Int` | Length of string or array |

### Math Functions

| Function | Signature | Description |
|----------|-----------|-------------|
| `abs` | `(x: Number) -> Number` | Absolute value |
| `min` | `(a: Number, b: Number) -> Number` | Minimum of two values |
| `max` | `(a: Number, b: Number) -> Number` | Maximum of two values |
| `round` | `(x: Float) -> Int` | Round to nearest integer |
| `floor` | `(x: Float) -> Int` | Round down to integer |
| `ceil` | `(x: Float) -> Int` | Round up to integer |
| `sqrt` | `(x: Number) -> Float` | Square root |
| `pow` | `(base: Number, exp: Number) -> Number` | Exponentiation |
| `sign` | `(x: Number) -> Int` | Sign (-1, 0, or 1) |
| `clamp` | `(x: Number, min: Number, max: Number) -> Number` | Clamp to range |

### Examples

```intent
// Math operations
let x = abs(-42)           // 42
let smaller = min(10, 20)  // 10
let larger = max(10, 20)   // 20
let rounded = round(3.7)   // 4
let root = sqrt(16)        // 4.0
let squared = pow(2, 3)    // 8
let bounded = clamp(15, 0, 10)  // 10
```

## Effects

Effects track side effects and error conditions.

### Effect Types

```intent
fn read_file(path: String) -> Result<String, IOError> / {IO, Error}
```

### Effect Handlers

```intent
try {
    let content = read_file("data.txt")?;
    process(content);
} catch IOError as e {
    log_error(e);
}
```

## Concurrency

### Protocols

```intent
protocol Handshake {
    send Hello(String) -> receive HelloAck(String) -> end
}
```

### Async Operations

```intent
async fn fetch_data(url: String) -> Result<Data, NetworkError> {
    // implementation
}
```

## Modules

### Module Declaration

```intent
mod math {
    pub fn add(a: Int, b: Int) -> Int {
        a + b
    }
}
```

### Imports

```intent
use math::add;
use std::collections::HashMap;
```

## Structured Edits

Code modifications are represented as typed operations:

```intent
edit AddParameter {
    target: "fn process_data",
    param: "config: Config"
}
```

## Intent Annotations

```intent
/// intent: "Implements Dijkstra's shortest path algorithm for routing"
fn shortest_path(graph: Graph, start: Node) -> Map<Node, Distance> {
    // implementation
}
```

## Observability

Built-in logging of AI decisions:

```intent
#[observe]
fn optimize_query(query: Query) -> OptimizedQuery {
    // AI reasoning logged automatically
}
```

## Human Approval

```intent
#[requires_approval("UI changes")]
fn update_ui(component: Component) {
    // implementation
}
```

---

_This specification is preliminary and subject to change. See the whitepaper for detailed motivation and the [ROADMAP.md](ROADMAP.md) for the 13-phase implementation plan._
