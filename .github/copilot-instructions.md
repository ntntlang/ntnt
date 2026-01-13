# NTNT Language Copilot Instructions

When generating or editing `.tnt` files (NTNT language), follow these critical syntax rules:

## ⚠️ ALWAYS Lint/Test Before Run

Before running any `.tnt` file, validate first:

```bash
ntnt lint myfile.tnt    # Always do this first
ntnt run myfile.tnt     # Only after lint passes

# For HTTP servers - test automatically
ntnt test server.tnt --get /api/status --post /users --body 'name=Alice'
```

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

### String Escapes

NTNT supports standard escape sequences in regular strings:

```ntnt
// Supported escapes
let newline = "line1\nline2"      // \n = newline
let tabbed = "col1\tcol2"         // \t = tab
let quoted = "She said \"hi\""    // \" = quote
let path = "C:\\Users\\name"      // \\ = backslash
let brace = "literal \{brace\}"   // \{ \} = literal braces

// Raw strings for complex content (no escape processing)
let html = r#"<div class="main">Hello</div>"#
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
int("42")         // Correct - string to integer
push(arr, item)   // Correct

"hello".len()     // Wrong
```

## HTTP POST/Form Handling

Use `parse_query()` from `std/url` to parse form data:

```ntnt
import { parse_query } from "std/url"

fn post(req) {
    // parse_query converts "name=Alice&age=25" → map { "name": "Alice", "age": "25" }
    let form = parse_query(req.body)

    let name = form["name"]
    let age = int(form["age"])  // Convert to int for database!
}
```

## Type Conversion for Database

**Form fields are strings. Convert before database operations:**

```ntnt
// WRONG - causes "db error"
execute(db, "INSERT INTO users (age) VALUES ($1)", [form["age"]])

// CORRECT
let age = int(form["age"])
execute(db, "INSERT INTO users (age) VALUES ($1)", [age])
```

## Standard Library Reference

### Built-in (no import)

`print`, `len`, `str`, `int`, `float`, `abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `Some`, `None`, `Ok`, `Err`, `unwrap`, `unwrap_or`, `is_some`, `is_none`, `is_ok`, `is_err`

### Environment Variables

```ntnt
import { get_env, load_env } from "std/env"

// Load from .env file (sets environment variables)
let _ = load_env(".env")  // Returns Result<(), String>

// Get individual variables
let db_url = match get_env("DATABASE_URL") {
    Some(url) => url,
    None => "postgres://localhost/db"
}
```

### Common imports

```ntnt
import { split, join, trim, replace } from "std/string"
import { encode, decode, parse_query, build_query } from "std/url"
import { push, pop, keys, values, entries, has_key } from "std/collections"
import { get, post, get_json } from "std/http"
import { listen, get, post, json, html } from "std/http_server"
import { connect, query, execute, close } from "std/db/postgres"
import { read_file, write_file, exists } from "std/fs"
import { parse, stringify } from "std/json"
import { parse, parse_with_headers, stringify, stringify_with_headers } from "std/csv"
import { now, format } from "std/time"
import { get_env } from "std/env"
import { channel, send, recv } from "std/concurrent"
```

## Full Reference

See docs/AI_AGENT_GUIDE.md for complete syntax documentation and patterns.
