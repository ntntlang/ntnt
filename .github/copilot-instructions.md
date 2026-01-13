# NTNT Language Copilot Instructions

When generating or editing `.tnt` files (NTNT language), follow these critical syntax rules:

## Mandatory Syntax Rules

### Map Literals
Always use `map {}` for key-value structures. Never use bare `{}` for maps.
```ntnt
// Correct
let data = map { "name": "Alice", "age": 30 }

// Wrong - {} is a block expression
let data = { "name": "Alice" }
```

### String Interpolation
Use `{expr}` directly in strings. Do not use `${expr}` or backticks.
```ntnt
// Correct
let msg = "Hello, {name}!"

// Wrong
let msg = `Hello, ${name}!`
```

### Route Patterns
Always use raw strings `r"..."` for HTTP route patterns containing parameters.
```ntnt
// Correct
get(r"/users/{id}", handler)

// Wrong - {id} interpreted as interpolation
get("/users/{id}", handler)
```

### Contract Placement
Place `requires` and `ensures` between return type and function body.
```ntnt
fn divide(a: Int, b: Int) -> Int
    requires b != 0
    ensures result * b == a
{
    return a / b
}
```

### Range Expressions
Use `..` for exclusive ranges, `..=` for inclusive. No `range()` function.
```ntnt
for i in 0..10 { }   // 0 to 9
for i in 0..=10 { }  // 0 to 10
```

### Imports
Use JavaScript-style imports with `/` path separators.
```ntnt
import { split, join } from "std/string"
import "std/math" as math
```

### Mutability
Variables are immutable by default. Use `let mut` for mutable variables.
```ntnt
let mut counter = 0
counter = counter + 1
```

### Functions Over Methods
Use standalone functions, not method calls.
```ntnt
len("hello")      // Correct
str(42)           // Correct
push(arr, item)   // Correct

"hello".len()     // Wrong
```

## Standard Library Reference

### Built-in (no import)
`print`, `len`, `str`, `abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `Some`, `None`, `Ok`, `Err`, `unwrap`, `unwrap_or`, `is_some`, `is_none`, `is_ok`, `is_err`

### Common imports
```ntnt
import { split, join, trim, replace } from "std/string"
import { push, pop, map, filter, reduce } from "std/collections"
import { get, post, get_json } from "std/http"
import { listen, get, post, json, html } from "std/http_server"
import { read_file, write_file, exists } from "std/fs"
import { parse, stringify } from "std/json"
import { now, format } from "std/time"
import { channel, send, recv } from "std/concurrent"
```

## Full Reference

See docs/AI_AGENT_GUIDE.md for complete syntax documentation and patterns.
