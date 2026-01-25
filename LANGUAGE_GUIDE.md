# NTNT Language Guide

A practical guide to learning NTNT with examples. For complete reference documentation, see:

- **[SYNTAX_REFERENCE.md](docs/SYNTAX_REFERENCE.md)** - Keywords, operators, types (auto-generated)
- **[STDLIB_REFERENCE.md](docs/STDLIB_REFERENCE.md)** - All functions (auto-generated)
- **[IAL_REFERENCE.md](docs/IAL_REFERENCE.md)** - Intent Assertion Language (auto-generated)

---

## Table of Contents

1. [Intent-Driven Development](#intent-driven-development)
2. [Template Strings](#template-strings)
3. [Types](#types)
4. [Contracts](#contracts)
5. [Traits](#traits)
6. [Control Flow](#control-flow)
7. [Concurrency](#concurrency)
8. [HTTP Client](#http-client)
9. [HTTP Server](#http-server)
10. [Database](#database)
11. [Time](#time)

---

## Intent-Driven Development

NTNT's core workflow is **Intent-Driven Development (IDD)** - you define requirements as executable specifications, then implement code that satisfies them.

### Quick Start

1. **Define requirements** in a `.intent` file:

```yaml
## Glossary

| Term | Means |
|------|-------|
| a user visits {path} | GET {path} |
| the home page | / |
| the page loads | status 200 |
| they see {text} | body contains {text} |

---

Feature: Home Page
  id: feature.home

  Scenario: Welcome message
    When a user visits the home page
    → the page loads
    → they see "Welcome"
```

2. **Implement** with `@implements` annotations:

```ntnt
import { html } from "std/http/server"

// @implements: feature.home
fn home(req) {
    return html("<h1>Welcome</h1>")
}

get("/", home)
listen(8080)
```

3. **Verify** with Intent Studio or command line:

```bash
ntnt intent studio server.intent  # Visual preview with live tests
ntnt intent check server.tnt      # Command line verification
```

For comprehensive IDD documentation, see [docs/AI_AGENT_GUIDE.md](docs/AI_AGENT_GUIDE.md#intent-driven-development-idd).

---

## Template Strings

Triple-quoted strings `"""..."""` with `{{expr}}` interpolation. Single `{}` pass through for CSS/JS.

### Basic Interpolation

```ntnt
let name = "Alice"
let page = """
<html>
<style>
    h1 { color: blue; }  // CSS braces pass through
</style>
<body>
    <h1>Hello, {{name}}!</h1>  // Interpolation uses {{}}
</body>
</html>
"""
```

### Loops

```ntnt
let users = ["Alice", "Bob", "Charlie"]
let list = """
<ul>
{{#for user in users}}
    <li>{{user}}</li>
{{/for}}
</ul>
"""
```

**Loop metadata variables:**

| Variable | Description |
|----------|-------------|
| `@index` | 0-based index |
| `@length` | Total items |
| `@first` | True if first |
| `@last` | True if last |
| `@even` / `@odd` | Alternating rows |

```ntnt
let colors = ["Red", "Green", "Blue"]
let indexed = """
{{#for color in colors}}
    <li class="{{#if @first}}first{{#elif @last}}last{{#else}}middle{{/if}}">
        {{@index}}/{{@length}}: {{color}}
    </li>
{{/for}}
"""
```

### Empty Fallback

```ntnt
let items = []
let list = """
<ul>
{{#for item in items}}
    <li>{{item}}</li>
{{#empty}}
    <li class="empty">No items found</li>
{{/for}}
</ul>
"""
```

### Conditionals

```ntnt
let logged_in = true
let nav = """
<nav>
{{#if logged_in}}
    <a href="/profile">Profile</a>
{{#else}}
    <a href="/login">Login</a>
{{/if}}
</nav>
"""
```

### Elif Chains

```ntnt
let score = 75
let grade = """
{{#if score >= 90}}A
{{#elif score >= 80}}B
{{#elif score >= 70}}C
{{#elif score >= 60}}D
{{#else}}F{{/if}}
"""
```

### Filters

Filters transform values using pipe syntax:

```ntnt
let name = "john doe"
let html = """
<p>{{name | capitalize}}</p>
<p>{{name | uppercase}}</p>
<p>{{missing_var | default("N/A")}}</p>
<p>{{user_input | escape}}</p>
"""
```

Filters can be chained:

```ntnt
let items = [3, 1, 4, 1, 5]
let output = """{{items | reverse | join(", ")}}"""
// Output: 5, 1, 4, 1, 3
```

See [SYNTAX_REFERENCE.md](docs/SYNTAX_REFERENCE.md#template-strings) for the complete filter list.

---

## Types

### Union Types

Allow a value to be one of several types:

```ntnt
fn stringify(value: String | Int | Bool) -> String {
    return str(value)
}

stringify("hello")  // works
stringify(42)       // works
stringify(true)     // works

// Useful for flexible APIs
fn process(input: String | [String]) {
    // Handle both single string and array of strings
}
```

### Map Literals

Top-level maps require `map` keyword. Nested maps are inferred:

```ntnt
// Top-level requires `map`
let user = map { "name": "Alice", "age": 30 }

// Nested maps are inferred
let config = map {
    "server": { "host": "localhost", "port": 8080 },
    "database": { "url": "postgres://...", "pool": 5 }
}

// Access nested values
print(config["server"]["host"])  // "localhost"
```

---

## Contracts

Contracts specify behavioral requirements enforced at runtime.

### Preconditions and Postconditions

```ntnt
fn transfer_funds(amount: Int, from: Account, to: Account) -> Result<(), Error>
    requires amount > 0 && from.balance >= amount
    ensures to.balance == old(to.balance) + amount
{
    // implementation
}
```

### The `old()` Function

Captures the value of an expression at function entry:

```ntnt
fn increment(counter: Counter)
    ensures counter.value == old(counter.value) + 1
{
    counter.value = counter.value + 1
}
```

### The `result` Keyword

Refers to the return value in postconditions:

```ntnt
fn double(x: Int) -> Int
    ensures result == x * 2
{
    return x * 2
}
```

### Struct Invariants

Invariants are checked after construction and after any mutation:

```ntnt
struct Account {
    balance: Int,
    owner: String
}

impl Account {
    invariant self.balance >= 0
}
```

---

## Traits

Traits define shared behavior:

```ntnt
trait Serializable {
    fn to_json(self) -> String
    fn from_json(json: String) -> Self
}

trait Comparable {
    fn compare(self, other: Self) -> Int

    // Default implementation
    fn less_than(self, other: Self) -> Bool {
        return self.compare(other) < 0
    }
}
```

### Trait Implementation

```ntnt
impl Comparable for User {
    fn compare(self, other: User) -> Int {
        return self.id - other.id
    }
    // less_than uses default implementation
}
```

### Trait Bounds

```ntnt
fn sort<T: Comparable>(arr: [T]) -> [T] {
    // Can use compare() and less_than() on elements
}
```

---

## Control Flow

### Defer Statement

Execute cleanup code when leaving scope (LIFO order):

```ntnt
fn process_file(path: String) -> Result<Data, Error> {
    let file = open(path)
    defer close(file)  // Always runs, even on error/return

    let data = read(file)
    return Ok(data)
}
```

### Match Expressions

```ntnt
match value {
    Ok(data) => process(data),
    Err(e) => handle_error(e),
}

match number {
    0 => "zero",
    1 => "one",
    n if n < 0 => "negative",
    _ => "other",
}
```

### Map Iteration

```ntnt
import { keys, values, entries, has_key } from "std/collections"

let users = map {
    "alice": { "score": 100, "level": 5 },
    "bob": { "score": 85, "level": 3 }
}

for name in keys(users) {
    let user = users[name]
    print("{name}: score={user[\"score\"]}")
}

if has_key(users, "alice") {
    print("Alice found!")
}
```

---

## Concurrency

NTNT uses Go-style channels (no async/await):

```ntnt
import { channel, send, recv, try_recv, recv_timeout, close, sleep_ms } from "std/concurrent"

// Create a channel
let ch = channel()

// Send values (blocks if no receiver)
send(ch, "hello")
send(ch, map { "user_id": 123 })

// Receive (blocks until value available)
let msg = recv(ch)

// Non-blocking receive
match try_recv(ch) {
    Some(value) => process(value),
    None => print("No message")
}

// Receive with timeout (milliseconds)
match recv_timeout(ch, 5000) {
    Some(value) => handle(value),
    None => print("Timeout")
}

close(ch)
```

---

## HTTP Client

```ntnt
import { fetch, download } from "std/http"
import { parse } from "std/json"

// Simple GET
match fetch("https://api.example.com/data") {
    Ok(response) => {
        if response.ok {
            print(response.body)
        }
    },
    Err(e) => print("Error: " + e)
}

// POST with JSON
match fetch(map {
    "url": "https://api.example.com/users",
    "method": "POST",
    "json": map { "name": "Alice", "email": "alice@example.com" }
}) {
    Ok(response) => print("Created!"),
    Err(e) => print("Error: " + e)
}

// POST form data
match fetch(map {
    "url": "https://api.example.com/login",
    "method": "POST",
    "form": map { "username": "alice", "password": "secret" }
}) {
    Ok(response) => print(response.status),
    Err(e) => print("Error: " + e)
}

// With headers, auth, timeout
match fetch(map {
    "url": "https://api.example.com/secure",
    "headers": map { "Authorization": "Bearer token123" },
    "timeout": 30
}) {
    Ok(response) => print(response.body),
    Err(e) => print("Error: " + e)
}

// Download file
match download("https://example.com/file.pdf", "./file.pdf") {
    Ok(result) => print("Downloaded " + str(result.size) + " bytes"),
    Err(e) => print("Failed: " + e)
}
```

---

## HTTP Server

```ntnt
import { json, html, redirect, parse_form } from "std/http/server"

fn home(req) {
    return html("<h1>Welcome!</h1>")
}

fn get_user(req) {
    let id = req.params["id"]
    return json(map { "id": id, "name": "User " + id })
}

fn create_user(req)
    requires len(req.body) > 0  // Contract: returns 400 if empty
{
    let form = parse_form(req)
    return json(map { "created": true, "name": form["name"] })
}

// Routes (global builtins - no import needed)
get("/", home)
get(r"/users/{id}", get_user)  // Raw string for route params
post("/users", create_user)

// Static files
serve_static("/assets", "./public")

// Start server
listen(8080)
```

---

## Database

```ntnt
import { connect, query, execute, begin, commit, rollback, close } from "std/db/postgres"

let db = unwrap(connect("postgresql://user:pass@localhost/mydb"))

// Query with parameters
match query(db, "SELECT * FROM users WHERE id = $1", [user_id]) {
    Ok(rows) => {
        for row in rows {
            print(row["name"])
        }
    },
    Err(e) => print("Query failed: " + e)
}

// Transaction
begin(db)
execute(db, "UPDATE accounts SET balance = balance - $1 WHERE id = $2", [100, from_id])
execute(db, "UPDATE accounts SET balance = balance + $1 WHERE id = $2", [100, to_id])
commit(db)

close(db)
```

---

## Time

```ntnt
import { now, to_timezone, format, add_days, weekday_name, HOUR } from "std/time"

// Current timestamp
let ts = now()

// Timezone conversion
let ny = to_timezone(ts, "America/New_York")
print("NYC: " + str(ny["hour"]) + ":" + str(ny["minute"]))

// Formatting
print(format(ts, "%Y-%m-%d %H:%M:%S"))

// Date arithmetic
let tomorrow = add_days(ts, 1)
let next_month = add_months(ts, 1)

// Parsing
match parse_iso("2024-03-20T10:30:00+00:00") {
    Ok(parsed) => print(parsed),
    Err(e) => print(e)
}
```

---

See [STDLIB_REFERENCE.md](docs/STDLIB_REFERENCE.md) for complete function documentation.
