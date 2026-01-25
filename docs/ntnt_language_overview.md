# NTNT Language Overview

NTNT is a modern programming language designed specifically for **Intent-Driven Development (IDD)** and high-performance web services. It can be best described as **"Rust for the Web"** or **"Scripting with Contracts."**

It combines the ergonomic syntax and data modeling of Rust with the agility and garbage collection of dynamic languages like JavaScript or Python, adding a native layer of Design-by-Contract for robust runtime safety.

## Core Philosophy

1.  **Rust-Flavored Syntax**: It adopts Rust's clean syntax, expression-oriented control flow, and powerful `Result<T, E>` / `Option<T>` error handling patterns.
2.  **Managed Memory**: It removes the complexity of the Borrow Checker and Lifetimes (`'a`) in favor of a fast Garbage Collector, allowing for rapid iteration similar to Go or TypeScript.
3.  **Design-by-Contract**: It replaces compile-time memory safety with runtime behavioral safety. Functions and data structures enforce `requires`, `ensures`, and `invariant` clauses to guarantee correctness.
4.  **Web-Native**: Concepts like JSON, HTTP servers, and HTML templating are first-class citizens, not external libraries.

## Key Features

### 1. Hybrid Type System

NTNT supports both rigid, static data modeling and flexible, dynamic data structures.

- **Static:** Structs, Enums, and Traits for core domain logic.
- **Dynamic:** `Map` literals (`map { "key": "value" }`) and Union Types (`String | Int`) for flexible API handling.

### 2. Design-by-Contract

Safety is enforced at the API boundary using contracts.

```ntnt
fn transfer(amount: Int, from: Account, to: Account) -> Result<(), Error>
    requires amount > 0
    requires from.balance >= amount
    ensures to.balance == old(to.balance) + amount
{
    // Implementation is significantly safer ensuring these rules
}
```

### 3. Built-in HTTP & Web

No heavy frameworks required. A production-ready server is just a few lines of code.

```ntnt
import { json, html } from "std/http/server"

// Route handlers are standard functions
fn get_user(req) {
    return json(map { "id": req.params["id"], "active": true })
}

// Global built-ins for routing
get(r"/users/{id}", get_user)
listen(8080)
```

### 4. Concurrency via Channels

NTNT avoids specific `async/await` syntax overhead in favor of Go-style channels for simple, robust concurrency.

```ntnt
import { channel, send, recv } from "std/concurrent"

let ch = channel()
send(ch, "data")
let msg = recv(ch)
```

## Comparison: NTNT vs. Rust

NTNT is roughly **40-50%** of Rust, keeping the "application logic" subset while abstracting away the "systems programming" subset.

| Feature Category | Rust                  | NTNT                 | Comparison                                                          |
| :--------------- | :-------------------- | :------------------- | :------------------------------------------------------------------ | --------------------------------- |
| **Syntax**       | `fn`, `let`, `match`  | `fn`, `let`, `match` | **Identical**. If you know Rust, you know NTNT.                     |
| **Memory**       | Manual (Ownership)    | Automated (GC)       | NTNT is faster to write; Rust is faster to run.                     |
| **Safety**       | Compile-time (Types)  | Runtime (Contracts)  | Rust catches mem-leaks; NTNT catches logic bugs.                    |
| **Polymorphism** | Traits + Generics     | Traits + Unions      | NTNT adds Union types (`Int                                         | String`) for JSON-friendly logic. |
| **Async**        | Futures + Async/Await | Channels             | NTNT uses a simpler CSP (Communicating Sequential Processes) model. |

## Why NTNT?

Use NTNT when you want the **correctness and structure of a Rust project** but need the **speed of development and iteration of a Node.js/Python project**. It is ideal for:

- HTTP API Servers
- Microservices
- Internal Tooling & Scripts
- Intent-Driven Development Workflows
