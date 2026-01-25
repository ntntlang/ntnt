# NTNT Language - Copilot Instructions

When generating `.tnt` files (NTNT language), follow these rules.

## Quick Reference

**Full documentation:** See [docs/AI_AGENT_GUIDE.md](../docs/AI_AGENT_GUIDE.md)

```bash
ntnt lint file.tnt    # ALWAYS lint first
ntnt run file.tnt     # Run after lint passes
```

## Critical Syntax Rules

### Map literals require `map` keyword
```ntnt
let data = map { "key": "value" }  // CORRECT
let data = { "key": "value" }      // WRONG - {} is a block
```

### String interpolation uses `{expr}` not `${expr}`
```ntnt
let msg = "Hello, {name}!"  // CORRECT
let msg = "Hello, ${name}!" // WRONG
```

### Route patterns require raw strings
```ntnt
get(r"/users/{id}", handler)  // CORRECT
get("/users/{id}", handler)   // WRONG
```

### Contracts go after return type, before body
```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
{
    return a / b
}
```

### Range syntax (not function)
```ntnt
for i in 0..10 { }   // CORRECT
for i in range(10) { } // WRONG
```

### Functions not methods
```ntnt
len("hello")    // CORRECT
"hello".len()   // WRONG
```

## HTTP Server Pattern

```ntnt
// ONLY import response builders
import { json, html } from "std/http/server"

fn handler(req) {
    return json(map { "id": req.params["id"] })
}

// Routes are GLOBAL - no import needed
get(r"/users/{id}", handler)
listen(8080)
```

## Common Imports

```ntnt
import { split, join, trim } from "std/string"
import { json, html, parse_form } from "std/http/server"
import { connect, query, execute, close } from "std/db/postgres"
import { fetch } from "std/http"
import { read_file, write_file } from "std/fs"
import { parse, stringify } from "std/json"
```

## Intent-Driven Development

For IDD workflow, see [docs/AI_AGENT_GUIDE.md](../docs/AI_AGENT_GUIDE.md#intent-driven-development-idd).
