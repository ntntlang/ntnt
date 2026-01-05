# Intent Language Specification

## Version 0.1 (Draft)

This document specifies the syntax, semantics, and core features of the Intent programming language.

## Table of Contents

1. [Lexical Structure](#lexical-structure)
2. [Types](#types)
3. [Contracts](#contracts)
4. [Functions and Methods](#functions-and-methods)
5. [Effects](#effects)
6. [Concurrency](#concurrency)
7. [Modules](#modules)

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

Contracts specify behavioral requirements for code.

### Function Contracts

```intent
fn transfer_funds(amount: Int, from: Account, to: Account) -> Result<(), Error>
requires amount > 0 && from.balance >= amount
ensures to.balance == old(to.balance) + amount
{
    // implementation
}
```

### Class Invariants

```intent
struct Account {
    balance: Int
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

_This specification is preliminary and subject to change. See the whitepaper for detailed motivation and examples._
