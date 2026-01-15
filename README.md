# NTNT Programming Language

NTNT (pronounced "Intent") is an experimental Agent-Native programming language designed for AI-assisted software development. It introduces Intent-Driven Development (IDD), where human requirements become executable specifications that AI agents implement and the system verifies. Design-by-contract syntax, machine-readable introspection, and `@implements` annotations create full traceability from intent to code.

## Quick Start

### Installation

```bash
git clone https://github.com/ntntlang/ntnt.git
cd ntnt
cargo build --release
cargo install --path . --locked
```

### Hello World

```bash
echo 'print("Hello, World!")' > hello.tnt
ntnt run hello.tnt
```

### A Complete Web API

```ntnt
// api.tnt
import { json } from "std/http/server"

get("/", |req| json(map { "message": "Hello!" }))
get("/users/{id}", |req| json(map { "id": req.params.id }))

listen(3000)
```

```bash
ntnt run api.tnt
# Visit http://localhost:3000
```

---

## Why NTNT?

Traditional programming languages were designed for humans typing code character by character. As AI agents become capable of generating and deploying software, the requirements shift: agents need structured validation and predictable patterns; humans need to understand what code is supposed to do without reading every line. NTNT bridges this gap with Intent-Driven Development, design-by-contract syntax, and tooling built for human-agent collaboration.

### Intent-Driven Tooling

NTNT's CLI commands are designed for AI agents and automated workflows.

**Intent Studio**: `ntnt intent studio` opens a beautiful live preview of your `.intent` file. Edit side-by-side with an AI agent and watch updates appear instantly.

```bash
ntnt intent studio server.intent
# 🎨 Intent Studio: http://localhost:3000
# 👀 Watching server.intent for changes...
```

**Intent verification**: `ntnt intent check` verifies that code with `@implements` annotations matches human intent described in natural language `.intent` files.

```bash
ntnt intent check myapp.tnt
# Feature: User Authentication
#   ✓ POST /login returns status 200
#   ✓ body contains "token"
# 1/1 features passing (2/2 assertions)
```

**Coverage tracking**: `ntnt intent coverage` shows which features have implementations and which are missing.

**Scaffolding generation**: `ntnt intent init` generates code structure from intent files, giving agents a starting point.

**Machine-readable introspection**: `ntnt inspect` outputs JSON describing every function, route, contract, and import. Agents can understand a codebase in a single call.

**Pre-execution validation**: `ntnt validate` checks code for errors before running, with structured JSON output.

### Contracts as First-Class Citizens

Contracts are part of the language syntax:

```ntnt
fn withdraw(amount: Int) -> Int
    requires amount > 0
    requires amount <= self.balance
    ensures result >= 0
{
    self.balance = self.balance - amount
    return self.balance
}
```

Agents and humans agree via contracts on the intended functionality of the code.

- **For agents**: Contracts are machine-readable specifications. An agent can read `requires amount > 0` and know exactly what inputs are valid without analyzing the implementation.
- **For humans**: Contracts serve as executable documentation. You can understand what a function expects and guarantees at a glance.
- **For HTTP APIs**: A failed precondition automatically returns 400 Bad Request. A failed postcondition returns 500 Internal Server Error. The contract _is_ the validation layer.

### Ready to Build

NTNT includes a standard library for common tasks. No package manager, no dependency decisions, no setup:

| Category | Modules | What's Included |
| -------- | ------- | --------------- |
| **Web** | `std/http/server`, `std/http` | HTTP server with routing, middleware, static files; HTTP client with fetch/post |
| **Data** | `std/json`, `std/csv`, `std/db/postgres` | Parse and stringify; PostgreSQL with transactions |
| **I/O** | `std/fs`, `std/path`, `std/env` | File operations, path manipulation, environment variables |
| **Text** | `std/string`, `std/url` | Split, join, trim, replace; URL encode/decode/parse |
| **Utilities** | `std/time`, `std/math`, `std/crypto` | Timestamps, formatting, sleep; trig, log, exp; SHA256, HMAC, UUID |
| **Collections** | `std/collections` | Array and map operations: push, pop, keys, values, get_key |
| **Concurrency** | `std/concurrent` | Go-style channels: send, recv, try_recv |

---

## Intent-Driven Development (IDD)

NTNT pioneers **Intent-Driven Development (IDD)**, a paradigm where human intent becomes executable specification.

Instead of writing tests that verify implementation details, you write **intent files** that describe what your software should do in plain English. The system then verifies that your code fulfills those intentions.

```yaml
# myapp.intent

Feature: User Greeting
  id: feature.greeting
  description: "Display a personalized greeting to users"
  test:
    - request: GET /?name=Alice
      assert:
        - status: 200
        - body contains "Hello, Alice"

Feature: API Status
  id: feature.api_status  
  description: "Health check endpoint for monitoring"
  test:
    - request: GET /api/status
      assert:
        - status: 200
        - body contains "ok"
```

```ntnt
// myapp.tnt

// @implements: feature.greeting
fn home(req) {
    let name = req.query_params.name ?? "World"
    return html("<h1>Hello, {name}!</h1>")
}

// @implements: feature.api_status
fn status(req) {
    return json(map { "status": "ok" })
}
```

```bash
$ ntnt intent check myapp.tnt

=== NTNT Intent Check ===

Feature: User Greeting
  ✓ GET /?name=Alice returns status 200
  ✓ body contains "Hello, Alice"

Feature: API Status
  ✓ GET /api/status returns status 200
  ✓ body contains "ok"

2/2 features passing (4/4 assertions)
```

### Why IDD?

| Aspect               | Traditional TDD           | Intent-Driven Development   |
| -------------------- | ------------------------- | --------------------------- |
| **Answers**          | "Does the code work?"     | "Does it do what we meant?" |
| **Written in**       | Code (pytest, Jest)       | Plain English + assertions  |
| **Readable by**      | Developers only           | Anyone on the team          |
| **AI collaboration** | Not designed for agents   | Built for human-agent work  |
| **Living docs**      | Tests diverge from intent | Intent IS the documentation |

**What IDD provides:**

- **Human-readable requirements** - Plain English descriptions anyone can understand
- **Machine-executable tests** - Assertions the system verifies automatically
- **Living documentation** - Always in sync because it IS the test suite
- **Traceability** - `@implements` annotations link code to features
- **Coverage reporting** - See which features are implemented and which are missing

### The IDD Workflow

IDD changes how you work with AI agents. Instead of describing code changes, you describe intent changes. The agent implements accordingly.

**1. Express intent in natural language**

You write (or discuss with the agent) what you want the software to do. The agent helps you clarify and structure this into a `.intent` file with features, descriptions, and testable assertions.

**2. Agent implements to match intent**

The agent writes code with `@implements` annotations linking each function to the feature it fulfills. You verify the intent is met, not the implementation details.

**3. Verify with `ntnt intent check`**

Run verification to confirm the code matches your expressed intent. If tests pass, the implementation is correct by definition.

**4. Modify by changing intent**

To change the application, modify the `.intent` file in natural language. Add a feature, change an assertion, update a description. Then ask the agent to update the implementation to match the new intent.

```
You: "Add a feature for rate limiting - max 100 requests per minute per IP"

# Agent adds to app.intent:
Feature: Rate Limiting
  id: feature.rate_limit
  description: "Limit requests to 100 per minute per IP address"
  test:
    - request: GET /api/status (101 times from same IP)
      assert:
        - status: 429

# Agent implements with @implements: feature.rate_limit
# You verify: ntnt intent check app.tnt
```

This creates a continuous loop: **intent → implementation → verification → intent changes → updated implementation**. The intent file becomes the source of truth for what the software should do.

> 📖 See [docs/INTENT_DRIVEN_DEVELOPMENT.md](docs/INTENT_DRIVEN_DEVELOPMENT.md) for the complete design document.

---

## Get Started with IDD

Follow this tutorial to build your first intent-driven application.

### Step 1: Create an Intent File

Create `greeting.intent` describing what you want to build:

```yaml
# greeting.intent

Feature: Personalized Greeting
  id: feature.greeting
  description: "Greet users by name with a friendly message"
  test:
    - request: GET /?name=Alice
      assert:
        - status: 200
        - body contains "Hello, Alice"
    - request: GET /
      assert:
        - status: 200
        - body contains "Hello, World"

Feature: JSON API
  id: feature.json_api
  description: "Return greeting as JSON for programmatic access"
  test:
    - request: GET /api/greet?name=Bob
      assert:
        - status: 200
        - body contains "Bob"
```

### Step 2: Generate Scaffolding

Use `ntnt intent init` to create a starting point:

```bash
ntnt intent init greeting.intent > greeting.tnt
```

This generates a skeleton with TODO comments for each feature.

### Step 3: Implement the Features

Add the implementation with `@implements` annotations:

```ntnt
// greeting.tnt
import { html, json } from "std/http/server"
import { get_key } from "std/collections"

// @implements: feature.greeting
fn home(req) {
    let name = get_key(req.query_params, "name") ?? "World"
    return html("<h1>Hello, {name}!</h1>")
}

// @implements: feature.json_api
fn api_greet(req) {
    let name = get_key(req.query_params, "name") ?? "World"
    return json(map { "greeting": "Hello, {name}!" })
}

get(r"/", home)
get(r"/api/greet", api_greet)
listen(8080)
```

### Step 4: Verify Against Intent

Run `ntnt intent check` to verify your implementation:

```bash
$ ntnt intent check greeting.tnt

=== NTNT Intent Check ===

Source: greeting.tnt
Intent: greeting.intent

Feature: Personalized Greeting
  ✓ GET /?name=Alice returns status 200
  ✓ body contains "Hello, Alice"
  ✓ GET / returns status 200
  ✓ body contains "Hello, World"

Feature: JSON API
  ✓ GET /api/greet?name=Bob returns status 200
  ✓ body contains "Bob"

2/2 features passing (6/6 assertions)
```

### Step 5: Check Coverage

See which features have implementations:

```bash
$ ntnt intent coverage greeting.tnt

=== Intent Coverage Report ===

✓ Personalized Greeting (feature.greeting)
    └─ greeting.tnt:6 in fn home

✓ JSON API (feature.json_api)
    └─ greeting.tnt:12 in fn api_greet

[██████████████████████████████] 100.0% coverage (2/2 features)
```

### IDD Commands Reference

| Command | Description |
|---------|-------------|
| `ntnt intent check <file>` | Verify code matches intent, run tests |
| `ntnt intent coverage <file>` | Show feature implementation coverage |
| `ntnt intent init <intent>` | Generate code scaffolding from intent |

---

## Who Should Use NTNT?

> ⚠️ **NTNT is experimental and not production-ready.** Use it for learning, prototyping, and exploring Intent-Driven Development. Do not use it for systems that require stability, security audits, or long-term maintenance.

**Good fit:**

- Prototypes and proof-of-concepts where learning matters more than longevity
- Experiments with AI-assisted development and Intent-Driven Development
- Internal tools and scripts where you control the environment
- Learning projects where contracts make expected behavior explicit
- Exploring what agent-native programming could look like

**Not a good fit:**

- Production applications of any kind
- Performance-critical systems (use Rust, Go, or C++)
- Projects requiring third-party libraries or a package ecosystem
- Teams that need mature IDE support and debugging tools

### Limitations

**Experimental**: NTNT is a research language. The API will change. There is no stability guarantee.

**Performance**: Interpreted, not compiled. Handles hundreds of requests per second, sufficient for demos and prototypes.

**Ecosystem**: No package manager. No third-party libraries. The standard library covers common tasks; everything else requires writing code or calling external services.

**Tooling**: No debugger. Debugging is done with print statements and contracts. IDE support is syntax highlighting only.

---

## Current Status

**Version 0.2.1** - Intent-Driven Development

NTNT includes:
- ✅ Full contract system (`requires`, `ensures`, struct invariants)
- ✅ Type system with generics, enums, pattern matching
- ✅ Standard library (HTTP, PostgreSQL, JSON, CSV, time, crypto, etc.)
- ✅ File-based routing with hot-reload
- ✅ IDD commands (`intent check`, `intent coverage`, `intent init`)
- ✅ Agent tooling (`inspect`, `validate`, `test`)
- 🔄 Intent diff and watch (coming soon)

See [ROADMAP.md](ROADMAP.md) for the complete 11-phase implementation plan.

---

## Language Features

### Option & Result Types

NTNT provides built-in `Option<T>` and `Result<T, E>` types for safe handling of nullable values and errors:

```ntnt
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

### Pattern Matching

Use `match` expressions for powerful pattern matching:

```ntnt
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

### Enums

Define custom enumerated types with optional associated data:

```ntnt
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

### Generics

Generic functions and types enable reusable code:

```ntnt
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

### Traits

Traits define shared behavior that types can implement:

```ntnt
// Define a trait
trait Display {
    fn display(self) -> String;
}

trait Comparable {
    fn compare(self, other) -> Int;

    // Default implementation
    fn equals(self, other) -> Bool {
        return self.compare(other) == 0;
    }
}

// Implement trait for a type
struct Point {
    x: Int,
    y: Int
}

impl Display for Point {
    fn display(self) -> String {
        return "({self.x}, {self.y})";
    }
}
```

### For-In Loops

Iterate over collections with `for...in`:

```ntnt
// Iterate over arrays
let numbers = [1, 2, 3, 4, 5];
for n in numbers {
    print(n);
}

// Iterate over ranges
for i in 0..5 {
    print(i);  // 0, 1, 2, 3, 4
}

for i in 0..=5 {
    print(i);  // 0, 1, 2, 3, 4, 5 (inclusive)
}

// Iterate over strings (by character)
for char in "hello" {
    print(char);
}

// Iterate over map keys
let scores = map { "alice": 100, "bob": 85 };
for name in scores {
    print(name);
}
```

### Defer

The `defer` statement schedules code to run when the current scope exits:

```ntnt
fn process_file(path: String) {
    let file = open(path);
    defer close(file);  // Always runs, even on error

    // ... process file ...
    if error_condition {
        return;  // defer still runs!
    }
    // ... more processing ...
}  // close(file) runs here

// Multiple defers run in reverse order (LIFO)
fn example() {
    defer print("first");
    defer print("second");
    defer print("third");
}  // Prints: third, second, first
```

### Maps

Key-value collections with the `map` keyword:

```ntnt
// Create a map
let scores = map {
    "alice": 100,
    "bob": 85,
    "charlie": 92
};

// Iterate over keys
for name in scores {
    print(name);
}
```

### String Interpolation

Embed expressions directly in strings using `{}`:

```ntnt
let name = "Alice";
let age = 30;

// Basic interpolation
print("Hello, {name}!");  // Hello, Alice!

// Expressions in interpolation
print("Next year: {age + 1}");  // Next year: 31

// Complex expressions
let items = ["apple", "banana"];
print("Count: {len(items)}");  // Count: 2

// Escape braces with backslash
print("Use \{braces\} literally");  // Use {braces} literally
```

### Raw Strings

Raw strings don't process escape sequences, perfect for regex, SQL, and paths:

```ntnt
// Simple raw string
let path = r"C:\Users\name\file.txt";  // Backslashes preserved

// Raw string with quotes (use # delimiters)
let sql = r#"SELECT * FROM users WHERE name = "Alice""#;

// Great for regex patterns
let pattern = r"\d{3}-\d{4}";

// Multi-line SQL
let query = r#"
    SELECT id, name, email
    FROM users
    WHERE active = true
    ORDER BY name
"#;
```

### Template Strings

Triple-quoted template strings are perfect for HTML templates with dynamic content. They use `{{}}` for interpolation. Single `{}` pass through unchanged, making them CSS-safe:

```ntnt
let name = "Alice"
let items = ["apple", "banana", "cherry"]

let page = """
<!DOCTYPE html>
<style>
    h1 { color: blue; }
</style>
<body>
    <h1>Hello, {{name}}!</h1>
    <ul>
    {{#for item in items}}
        <li>{{item}}</li>
    {{/for}}
    </ul>
</body>
"""
```

**Template Features:**

| Syntax                               | Description                  |
| ------------------------------------ | ---------------------------- |
| `{{expr}}`                           | Interpolate expression       |
| `{ ... }`                            | Literal braces (CSS/JS safe) |
| `{{#for x in arr}}...{{/for}}`       | Loop over array              |
| `{{#if cond}}...{{/if}}`             | Conditional                  |
| `{{#if cond}}...{{#else}}...{{/if}}` | If-else                      |
| `\{{` and `\}}`                      | Literal `{{` and `}}`        |

### Struct Invariants

Define invariants that are automatically enforced:

```ntnt
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

---

## Module System

NTNT features a powerful module system for organizing code:

```ntnt
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

```ntnt
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

```ntnt
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

**std/collections** - Array and map utilities

```ntnt
import { push, pop, first, last, reverse, slice, concat, is_empty } from "std/collections"
import { keys, values, entries, has_key, get_key } from "std/collections"

let arr = [1, 2, 3]
let arr2 = push(arr, 4)        // [1, 2, 3, 4]
let rev = reverse(arr)         // [3, 2, 1]

// Safe map access with get_key
let scores = map { "alice": 100, "bob": 85 }
let score = get_key(scores, "alice") ?? 0     // 100
let other = get_key(scores, "charlie") ?? 0   // 0
```

**std/env** - Environment access

```ntnt
import { get_env, load_env, args, cwd } from "std/env"

// Load from .env file
load_env(".env")

match get_env("DATABASE_URL") {
    Some(url) => print("DB: " + url),
    None => print("DATABASE_URL not set")
}
```

**std/fs** - File system operations

```ntnt
import { read_file, write_file, exists, mkdir, remove, readdir } from "std/fs"

match write_file("/tmp/test.txt", "Hello, NTNT!") {
    Ok(_) => print("File written"),
    Err(e) => print("Error: " + e)
}

match read_file("/tmp/test.txt") {
    Ok(content) => print(content),
    Err(e) => print("Error: " + e)
}
```

**std/json** - JSON parsing and stringification

```ntnt
import { parse, stringify, stringify_pretty } from "std/json"

let json_str = r#"{"name": "Alice", "age": 30}"#
match parse(json_str) {
    Ok(data) => print(data.name),  // "Alice"
    Err(e) => print("Parse error: " + e)
}

let user = map { "name": "Bob", "active": true }
let json = stringify(user)  // {"active":true,"name":"Bob"}
```

**std/csv** - CSV parsing and stringification

```ntnt
import { parse, parse_with_headers, stringify } from "std/csv"

let csv_data = "name,age\nAlice,30\nBob,25"

// Parse with headers (returns array of maps)
let records = parse_with_headers(csv_data)
// [map { "name": "Alice", "age": "30" }, map { "name": "Bob", "age": "25" }]
```

**std/time** - Time and date operations

```ntnt
import { now, sleep, format, elapsed } from "std/time"

let ts = now()
let formatted = format(ts, "%Y-%m-%d %H:%M:%S")
print(formatted)  // "2026-01-15 12:30:45"

sleep(100)  // Sleep for 100ms
```

**std/concurrent** - Go-style concurrency primitives

```ntnt
import { channel, send, recv, try_recv, recv_timeout, close } from "std/concurrent"

let ch = channel()

send(ch, "hello")
send(ch, map { "count": 42 })

let msg = recv(ch)  // Blocks until value available

match try_recv(ch) {
    Some(value) => print("Got: " + str(value)),
    None => print("No message yet")
}

close(ch)
```

---

## HTTP Server

NTNT includes a built-in HTTP server for building web APIs:

```ntnt
import { text, html, json } from "std/http/server"

fn home(req) {
    return text("Hello, World!")
}

fn get_user(req) {
    let id = req.params.id
    return json(map { "id": id, "name": "User " + id })
}

get("/", home)
get(r"/users/{id}", get_user)  // Raw string for {param} patterns
post("/users", create_user)

listen(8080)
```

### Static File Serving

```ntnt
serve_static("/static", "./public")
serve_static("/", "./public")  // Serve index.html at root
listen(8080)
```

### Middleware

```ntnt
fn logger(req) {
    print(req.method + " " + req.path)
    return ()  // Continue to next handler
}

fn auth_check(req) {
    if req.headers["authorization"] == "" {
        return status(401, "Unauthorized")
    }
    return ()
}

use_middleware(logger)
use_middleware(auth_check)

get("/api/data", get_data)
listen(8080)
```

### Contract-Verified Endpoints

```ntnt
fn create_user(req)
    requires req.body != ""
{
    return json(map { "status": "created" })
}

// Precondition failure → 400 Bad Request
// Postcondition failure → 500 Internal Server Error
```

### File-Based Routing

For larger applications, use convention-over-configuration routing:

```
my-app/
├── app.tnt              # Entry point
├── routes/              # File path = URL path
│   ├── index.tnt        # GET /
│   ├── about.tnt        # GET /about
│   └── api/
│       └── users/
│           ├── index.tnt    # GET/POST /api/users
│           └── [id].tnt     # GET/PUT/DELETE /api/users/{id}
├── lib/                 # Shared modules (auto-imported)
└── middleware/          # Auto-loaded in alphabetical order
```

```ntnt
// app.tnt
routes("routes")
listen(3000)
```

```ntnt
// routes/api/users/[id].tnt
import { json } from "std/http/server"

fn get(req) {
    return json(map { "id": req.params.id })
}

fn delete(req) {
    return json(map { "deleted": true })
}
```

**Hot-reload enabled** - edit route files and changes take effect on next request!

---

## PostgreSQL Database

```ntnt
import { connect, query, execute, close } from "std/db/postgres"
import { get_env } from "std/env"

match connect(get_env("DATABASE_URL")) {
    Ok(db) => {
        // Parameterized queries (safe from SQL injection)
        let users = query(db, "SELECT * FROM users WHERE active = $1", [true])

        match users {
            Ok(rows) => {
                for user in rows {
                    print(user["name"])
                }
            },
            Err(e) => print("Query error: " + e)
        }

        // Insert/Update/Delete
        execute(db, "INSERT INTO users (name) VALUES ($1)", ["Alice"])

        close(db)
    },
    Err(e) => print("Connection failed: " + e)
}
```

### Transactions

```ntnt
import { connect, begin, commit, rollback, execute } from "std/db/postgres"

match begin(db) {
    Ok(tx) => {
        execute(tx, "UPDATE accounts SET balance = balance - $1 WHERE id = $2", [100, 1])
        execute(tx, "UPDATE accounts SET balance = balance + $1 WHERE id = $2", [100, 2])
        commit(tx)
    },
    Err(e) => print("Transaction failed: " + e)
}
```

---

## Agent Tooling

### `ntnt inspect` - Project Introspection

Output JSON describing the structure of an NTNT project:

```bash
ntnt inspect api.tnt --pretty
```

```json
{
  "functions": [{ "name": "get_user", "line": 12, "contracts": { "requires": ["id > 0"] } }],
  "routes": [{ "method": "GET", "path": "/users/{id}", "handler": "get_user" }],
  "imports": [{ "source": "std/http/server", "items": ["json"] }]
}
```

### `ntnt validate` - Pre-Run Validation

Check files for errors before running:

```bash
ntnt validate examples/
```

### `ntnt test` - HTTP Test Mode

Test HTTP servers without manual curl commands:

```bash
ntnt test server.tnt --get /api/status
ntnt test server.tnt --post /users --body '{"name":"Alice"}'
ntnt test server.tnt --get /health --verbose
```

---

## Built-in Functions Reference

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

### Standard Library Quick Reference

| Module            | Key Functions                                                |
| ----------------- | ------------------------------------------------------------ |
| `std/string`      | split, join, trim, replace, contains, to_upper, to_lower     |
| `std/math`        | sin, cos, tan, log, exp, PI, E                               |
| `std/collections` | push, pop, first, last, keys, values, has_key, get_key       |
| `std/env`         | get_env, load_env, args, cwd                                 |
| `std/fs`          | read_file, write_file, exists, mkdir, remove, readdir        |
| `std/json`        | parse, stringify, stringify_pretty                           |
| `std/csv`         | parse, parse_with_headers, stringify                         |
| `std/time`        | now, sleep, format, elapsed                                  |
| `std/crypto`      | sha256, hmac_sha256, uuid, random_bytes                      |
| `std/url`         | parse, encode, decode, build_query, parse_query              |
| `std/http`        | fetch, post, put, delete, get_json, post_json                |
| `std/http/server` | text, html, json, status, get, post, put, listen             |
| `std/db/postgres` | connect, query, execute, begin, commit, rollback, close      |
| `std/concurrent`  | channel, send, recv, try_recv, recv_timeout, close           |

---

## Editor Support

### VS Code

Install the NTNT Language extension for syntax highlighting:

```bash
cp -r editors/vscode/intent-lang ~/.vscode/extensions/
```

Then restart VS Code. The extension provides:

- Syntax highlighting for `.tnt` files
- Code snippets for common patterns
- Bracket matching and auto-closing

---

## Documentation

- [Whitepaper](whitepaper.md) - Complete technical specification and motivation
- [Architecture](ARCHITECTURE.md) - System design and components
- [Language Spec](LANGUAGE_SPEC.md) - Formal language definition
- [Roadmap](ROADMAP.md) - 11-phase implementation plan
- [IDD Design](docs/INTENT_DRIVEN_DEVELOPMENT.md) - Intent-Driven Development guide
- [AI Agent Guide](docs/AI_AGENT_GUIDE.md) - Complete syntax reference for AI agents

---

## CLI Commands

```bash
ntnt run <file>              # Run a .tnt file
ntnt repl                    # Interactive REPL
ntnt lint <file|dir>         # Check for errors and warnings
ntnt validate <file|dir>     # Validate with JSON output
ntnt inspect <file>          # Project structure as JSON
ntnt test <file> [options]   # Test HTTP endpoints
ntnt intent check <file>     # Verify code matches intent
ntnt intent coverage <file>  # Show feature coverage
ntnt intent init <intent>    # Generate scaffolding from intent
ntnt --help                  # See all commands
```
