# NTNT Language - Claude Code Instructions

This file provides instructions for Claude when working with NTNT (.tnt) code files.

## Project Overview

NTNT (pronounced "Intent") is an agent-native programming language designed for AI-driven web application development. File extension: `.tnt`

## Critical Syntax Rules

### 1. Map Literals REQUIRE `map` Keyword

```ntnt
// ✅ CORRECT
let data = map { "key": "value" }
let empty = map {}

// ❌ WRONG - {} is a block, not a map
let data = { "key": "value" }
```

### 2. String Interpolation Uses `{expr}` NOT `${expr}`

```ntnt
// ✅ CORRECT
let msg = "Hello, {name}!"

// ❌ WRONG
let msg = `Hello, ${name}!`
let msg = "Hello, ${name}!"
```

### 3. Route Patterns REQUIRE Raw Strings

```ntnt
// ✅ CORRECT
get(r"/users/{id}", handler)

// ❌ WRONG
get("/users/{id}", handler)  // {id} becomes interpolation
```

### 4. Contracts Go BETWEEN Return Type and Body

```ntnt
// ✅ CORRECT
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}
```

### 5. Range Syntax (Not `range()` Function)

```ntnt
// ✅ CORRECT
for i in 0..10 { }    // exclusive
for i in 0..=10 { }   // inclusive

// ❌ WRONG
for i in range(10) { }
```

### 6. Import Syntax

```ntnt
// ✅ CORRECT
import { split, join } from "std/string"
import "std/math" as math

// ❌ WRONG
import std.string
from std.string import split
use std::string;
```

### 7. Mutable Variables Need `mut`

```ntnt
let mut counter = 0
counter = counter + 1
```

### 8. Functions Not Methods

```ntnt
// ✅ CORRECT
len("hello")
str(42)
push(arr, item)

// ❌ WRONG
"hello".len()
42.toString()
```

## Commands

```bash
# Run a file
cargo run -- run examples/demo.tnt

# Validate syntax
cargo run -- validate file.tnt

# Inspect project structure (JSON output for agents)
cargo run -- inspect file.tnt --pretty

# Run tests
cargo test

# Build release
cargo build --release
```

## Use `ntnt inspect` for Project Understanding

Before making changes to an NTNT project, use `ntnt inspect` to understand:
- Function signatures and contracts
- HTTP routes and handlers  
- Module imports and dependencies
- Struct definitions and invariants

## Standard Library Modules

- `std/string` - split, join, trim, replace, contains
- `std/collections` - push, pop, map, filter, reduce
- `std/http` - get, post, put, delete, get_json
- `std/http_server` - listen, get, post, json, html
- `std/fs` - read_file, write_file, exists, mkdir
- `std/json` - parse, stringify
- `std/time` - now, format, add_days
- `std/env` - get_env, set_env
- `std/concurrent` - channel, send, recv, sleep_ms

## Full Reference

See [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md) for complete syntax reference, all stdlib functions, and code patterns.
