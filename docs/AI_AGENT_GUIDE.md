# NTNT AI Agent Guide

This document provides critical syntax rules and patterns for AI agents generating NTNT code. Following these rules will prevent common errors and produce idiomatic code.

## Critical Syntax Rules

### 1. Map Literals Require `map` Keyword

**This is the most common mistake.** In NTNT, `{}` creates a block expression, NOT a map/object.

```ntnt
// ✅ CORRECT - Use `map {}` for key-value structures
let user = map { "name": "Alice", "age": 30 }
let empty_map = map {}
let config = map {
    "host": "localhost",
    "port": 8080,
    "debug": true
}

// ❌ WRONG - This creates a block expression, not a map
let user = { "name": "Alice" }   // ERROR or unexpected behavior
let empty = {}                    // This is an empty block, not empty map
```

### 2. String Interpolation Uses `{expr}` (Not `${expr}`)

```ntnt
// ✅ CORRECT - Direct curly braces in strings
let greeting = "Hello, {name}!"
let math = "Result: {a + b}"
let nested = "User {user.name} is {user.age} years old"

// ❌ WRONG - JavaScript/TypeScript style
let greeting = `Hello, ${name}!`     // Wrong: backticks and $
let greeting = "Hello, ${name}!"     // Wrong: $ prefix
```

### 3. Route Patterns Require Raw Strings

Route parameters use `{param}` which conflicts with string interpolation. Always use raw strings `r"..."` for routes:

```ntnt
// ✅ CORRECT - Raw strings for route patterns
get(r"/users/{id}", get_user)
post(r"/users/{user_id}/posts/{post_id}", create_post)
get(r"/api/v1/items/{category}/{item_id}", get_item)

// ❌ WRONG - {id} interpreted as variable interpolation
get("/users/{id}", get_user)          // ERROR: `id` is undefined
post("/users/{user_id}/posts", handler)  // ERROR
```

### 4. Contract Placement (requires/ensures)

Contracts go AFTER the return type but BEFORE the function body:

```ntnt
// ✅ CORRECT - Contract placement
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}

fn withdraw(amount: Int) -> Int
    requires amount > 0
    requires amount <= self.balance
    ensures result >= 0
    ensures self.balance == old(self.balance) - amount
{
    self.balance = self.balance - amount
    return self.balance
}

// ❌ WRONG - Contracts in wrong position
fn divide(a: Int, b: Int)
    requires b != 0          // Wrong: before return type
-> Int {
    return a / b
}

fn divide(a: Int, b: Int) -> Int {
    requires b != 0          // Wrong: inside function body
    return a / b
}
```

### 5. Range Expressions

```ntnt
// ✅ CORRECT - NTNT range syntax
for i in 0..10 { }      // 0 to 9 (exclusive end)
for i in 0..=10 { }     // 0 to 10 (inclusive end)
for i in 1..len(arr) { }

// ❌ WRONG - Python-style range function
for i in range(10) { }           // ERROR: range is not a function
for i in range(0, 10) { }        // ERROR
```

### 6. Import Syntax

NTNT uses JavaScript-style imports with quoted paths and `/` separators:

```ntnt
// ✅ CORRECT - NTNT import syntax
import { split, join, trim } from "std/string"
import { get, post } from "std/http"
import "std/math" as math
import { readFile as read } from "std/fs"

// Import from local files (relative paths)
import { helper } from "./lib/utils"
import { User } from "../models/user"

// ❌ WRONG - Other language styles
import std.string                    // Wrong: Python style
from std.string import split         // Wrong: Python style
use std::string::split;              // Wrong: Rust style
```

### 7. Mutable Variables

Variables are immutable by default. Use `mut` for mutability:

```ntnt
// ✅ CORRECT
let mut counter = 0
counter = counter + 1

let mut items = []
items = push(items, "new item")

// ❌ WRONG - Forgetting mut
let counter = 0
counter = counter + 1    // ERROR: cannot assign to immutable variable
```

### 8. Match Expression Syntax

Use `=>` (fat arrow) and commas between arms:

```ntnt
// ✅ CORRECT - Fat arrow and commas
match value {
    Ok(data) => process(data),
    Err(e) => handle_error(e),
}

match number {
    0 => "zero",
    n if n < 0 => "negative",
    _ => "positive",
}

// ❌ WRONG
match value {
    Ok(data) -> process(data)    // Wrong: thin arrow
    Err(e) => handle_error(e)    // Wrong: missing comma
}
```

### 9. Function Calls vs Methods

Many operations are standalone functions, not methods:

```ntnt
// ✅ CORRECT - Standalone functions
len("hello")              // Length of string
len(my_array)             // Length of array
str(42)                   // Convert to string
push(arr, item)           // Returns new array with item
split(text, ",")          // Split string

// ❌ WRONG - Method style (these don't exist)
"hello".len()             // ERROR
my_array.length           // ERROR
42.toString()             // ERROR
arr.push(item)            // May not work as expected
```

### 10. Result/Option Handling

Always handle `Result` and `Option` types explicitly:

```ntnt
// ✅ CORRECT - Pattern match or use helpers
match get("https://api.example.com") {
    Ok(response) => print(response.body),
    Err(e) => print("Error: {e}"),
}

// Using helper functions
let response = unwrap(get("https://api.example.com"))
let value = unwrap_or(optional_value, "default")

if is_ok(result) {
    let data = unwrap(result)
}

// ❌ WRONG - Treating Result as direct value
let response = get("https://api.example.com")
print(response.body)      // ERROR if get() returned Err
```

## Standard Library Quick Reference

### Built-in Functions (No Import Required)

| Function | Description |
|----------|-------------|
| `print(...)` | Output to stdout |
| `len(x)` | Length of string/array |
| `str(x)` | Convert to string |
| `abs(x)` | Absolute value |
| `min(a, b)` | Minimum of two values |
| `max(a, b)` | Maximum of two values |
| `sqrt(x)` | Square root |
| `pow(base, exp)` | Exponentiation |
| `round(x)`, `floor(x)`, `ceil(x)` | Rounding |
| `Some(v)`, `None` | Option constructors |
| `Ok(v)`, `Err(e)` | Result constructors |
| `unwrap(x)`, `unwrap_or(x, default)` | Unwrap helpers |
| `is_some(x)`, `is_none(x)`, `is_ok(x)`, `is_err(x)` | Type checks |

### Common Imports

```ntnt
// String operations
import { split, join, trim, replace, contains, starts_with, ends_with } from "std/string"

// Collections
import { push, pop, first, last, reverse, slice, map, filter, reduce, find } from "std/collections"

// HTTP client
import { get, post, put, delete, get_json, post_json } from "std/http"

// HTTP server
import { listen, get, post, json, html, text, redirect, serve_static, routes } from "std/http_server"

// File system
import { read_file, write_file, exists, is_file, is_dir, mkdir, readdir } from "std/fs"

// JSON
import { parse, stringify, stringify_pretty } from "std/json"

// Time
import { now, format, add_days, add_months } from "std/time"

// Environment
import { get_env, set_env, all_env } from "std/env"

// Concurrency
import { channel, send, recv, sleep_ms } from "std/concurrent"
```

## HTTP Server Patterns

```ntnt
import { listen, get, post, json, html, serve_static, routes } from "std/http_server"

// Basic route with raw string pattern
get(r"/users/{id}", fn(req) {
    let user_id = req.params["id"]
    return json(map { "id": user_id })
})

// POST with JSON body
post(r"/users", fn(req) {
    let body = req.json()
    return json(map { "created": true, "user": body })
})

// Static files
serve_static("/static", "./public")

// Start server
listen(8080)
```

## Contract Patterns

```ntnt
// Function with full contracts
fn transfer(from: Account, to: Account, amount: Int) -> Bool
    requires amount > 0
    requires from.balance >= amount
    ensures from.balance == old(from.balance) - amount
    ensures to.balance == old(to.balance) + amount
    ensures result == true
{
    from.balance = from.balance - amount
    to.balance = to.balance + amount
    return true
}

// Struct with invariant
struct BankAccount {
    balance: Int,
    owner: String
}

impl BankAccount {
    invariant self.balance >= 0
    
    fn deposit(self, amount: Int) -> Int
        requires amount > 0
        ensures self.balance == old(self.balance) + amount
    {
        self.balance = self.balance + amount
        return self.balance
    }
}
```

## Introspection with `ntnt inspect`

Use `ntnt inspect` to understand project structure programmatically:

```bash
# Inspect a file
ntnt inspect api.tnt --pretty

# Inspect a directory (auto-discovers routes)
ntnt inspect ./routes --pretty
```

Output includes:
- All functions with their contracts
- HTTP routes with handlers
- Module imports
- Struct definitions with invariants

## Common Patterns

### Defer for Cleanup

```ntnt
fn process_file(path: String) {
    let file = open(path)
    defer close(file)  // Always runs on scope exit
    
    // Work with file...
}
```

### Channel-based Concurrency

```ntnt
import { channel, send, recv, sleep_ms } from "std/concurrent"

let ch = channel()

// Sender
spawn(fn() {
    send(ch, "message")
})

// Receiver
let msg = recv(ch)
```

### Error Handling Pattern

```ntnt
fn fetch_user(id: String) -> Result<User, String> {
    let response = get("https://api.example.com/users/{id}")
    
    match response {
        Ok(r) => {
            if r.status == 200 {
                return Ok(parse(r.body))
            } else {
                return Err("User not found")
            }
        },
        Err(e) => return Err(e),
    }
}
```
