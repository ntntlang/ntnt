# Intent Programming Language

Intent is a revolutionary programming language and ecosystem designed specifically for AI-driven development. Unlike traditional languages built for human developers, Intent empowers AI agents as primary software creators while maintaining deep human oversight and collaboration.

**Goal: Build production-ready web applications and APIs with AI-powered development and runtime safety guarantees.**

## Current Status

**Phase 1: Core Contract System** ✅ Complete

- Function contracts (`requires`, `ensures`)
- `old()` function for pre-state capture
- `result` keyword in postconditions
- Struct invariants with automatic enforcement

**Phase 2: Type System & Pattern Matching** ✅ Complete

- Algebraic Data Types (enums with associated data)
- `Option<T>` and `Result<T, E>` built-ins
- Pattern matching with `match` expressions
- Generic functions and types
- Type aliases and union types

**Phase 3: Module System & Standard Library** ✅ Complete

- File-based modules with `import`/`export`
- Module aliasing: `import "std/math" as math`
- Selective imports: `import { split, join } from "std/string"`
- Standard library: `std/string`, `std/math`, `std/collections`, `std/env`, `std/fs`, `std/path`, `std/json`, `std/time`

**Phase 4: Traits & Essential Features** ✅ Complete

- Trait declarations with optional default implementations
- `impl Trait for Type` syntax for trait implementations
- Trait bounds in generics: `fn sort<T: Comparable>(arr: [T])`
- `for...in` loops for iterating over arrays, ranges, strings, and maps
- `defer` statement for cleanup code that runs on scope exit
- Range expressions: `0..10` (exclusive) and `0..=10` (inclusive)
- Map literals: `map { "key": value }`
- String interpolation: `"Hello, {name}!"`
- Raw strings: `r"no \n escapes"` and `r#"can use "quotes""#`

**Phase 5: File I/O, HTTP & Utilities** 🚧 In Progress

- File system: `std/fs` (read_file, write_file, exists, mkdir, remove, etc.)
- Path utilities: `std/path` (join, dirname, basename, extension, resolve)
- JSON parsing: `std/json` (parse, stringify)
- Time operations: `std/time` (now, sleep, format_timestamp)
- Cryptography: `std/crypto` (sha256, hmac_sha256, uuid, random_bytes)
- URL utilities: `std/url` (parse, encode, decode, build_query)
- HTTP Client: `std/http` (get, post, put, delete, request)
- **HTTP Server**: `std/http/server` (routing, path params, JSON responses)

**146 passing tests** | **Version 0.1.5**

**Next Up**: Async/await, Database connectivity (Phase 5 continued)

See [ROADMAP.md](ROADMAP.md) for the full 10-phase implementation plan.

## Quick Start

```bash
# Clone and build
git clone https://github.com/joshcramer/intent.git
cd intent
cargo build --release

# Run a program
./target/release/intent examples/contracts_full.intent

# Start the REPL
./target/release/intent repl
```

## File Extensions

Intent supports two file extensions:

- `.intent` - Standard Intent source files
- `.itn` - Short form (convenient for quick scripts)

## Overview

Intent reimagines software development for an era where AI agents handle the heavy lifting of coding, testing, and deployment. The language features:

- **First-Class Contracts**: Design-by-contract principles built into the syntax for guaranteed correctness
- **Runtime Safety**: Struct invariants and pre/post conditions enforced at runtime
- **Built-in Functions**: Math utilities (`abs`, `min`, `max`, `sqrt`, `pow`, `round`, `floor`, `ceil`, `sign`, `clamp`)
- **Typed Error Effects**: Explicit error handling and failure conditions
- **Semantic Versioning**: Automatic API compatibility management
- **Structured Edits**: AST-based code manipulation for safe refactoring
- **Multi-Agent Collaboration**: Built-in support for AI agents working together
- **Human-in-the-Loop Governance**: Transparent decision-making with human approval gates

### Production Roadmap

Intent is being developed toward production web application capabilities:

- **Phase 5**: HTTP server with contract-verified endpoints ✅
- **Phase 5**: Database access with repository patterns (in progress)
- **Phase 5**: Async/await for concurrent operations (planned)
- **Phase 11**: Docker deployment and container support

Performance targets: <1ms contract overhead, >10k requests/sec

## Example

```intent
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

## Option & Result Types

Intent provides built-in `Option<T>` and `Result<T, E>` types for safe handling of nullable values and errors:

```intent
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

## Pattern Matching

Use `match` expressions for powerful pattern matching:

```intent
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

## Enums

Define custom enumerated types with optional associated data:

```intent
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

## Generics

Generic functions and types enable reusable code:

```intent
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

## Traits

Traits define shared behavior that types can implement:

```intent
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

## For-In Loops

Iterate over collections with `for...in`:

```intent
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

## Defer

The `defer` statement schedules code to run when the current scope exits:

```intent
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

## Ranges

Range expressions create iterable sequences:

```intent
// Exclusive range (end not included)
let r1 = 0..10;    // 0, 1, 2, ..., 9

// Inclusive range (end included)
let r2 = 0..=10;   // 0, 1, 2, ..., 10

// Use in for loops
for i in 1..=5 {
    print(i * i);  // 1, 4, 9, 16, 25
}
```

## Maps

Key-value collections with the `map` keyword:

```intent
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

## String Interpolation

Embed expressions directly in strings using `{}`:

```intent
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

## Raw Strings

Raw strings don't process escape sequences, perfect for regex, SQL, and paths:

```intent
// Simple raw string
let path = r"C:\Users\name\file.txt";  // Backslashes preserved

// Raw string with quotes (use # delimiters)
let sql = r#"SELECT * FROM users WHERE name = "Alice""#;

// Multiple # for strings containing #
let code = r##"let x = r#"nested"#;"##;

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

## Trait Bounds

Constrain generic type parameters to types implementing specific traits:

```intent
// Single trait bound
fn sort<T: Comparable>(arr: [T]) -> [T] {
    // T must implement Comparable
}

// Multiple trait bounds with +
fn serialize<T: Serializable + Comparable>(item: T) -> String {
    return item.to_json();
}

// Struct with bounded type parameter
struct Cache<K: Hashable, V: Clone> {
    data: Map<K, V>,
}

// Works with any type implementing the required traits
sort([3, 1, 4, 1, 5]);  // Int implements Comparable
```

## Union Types

Union types allow a value to be one of several types:

```intent
// Function accepting multiple types
fn stringify(value: String | Int | Bool) -> String {
    return value;  // Will be converted to string
}

stringify("hello");  // works
stringify(42);       // works
stringify(true);     // works

// Useful for flexible APIs
fn process(input: String | [String]) {
    // Handle both single string and array of strings
}
```

## Effect Annotations

Mark functions with their side effects:

```intent
// Function with IO effect
fn read_config(path: String) -> String with io {
    // ... performs file I/O
}

// Pure function (no side effects)
fn add(a: Int, b: Int) -> Int pure {
    return a + b;
}
```

## Module System

Intent features a powerful module system for organizing code:

```intent
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

```intent
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

```intent
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

**std/collections** - Array utilities

```intent
import { push, pop, first, last, reverse, slice, concat, is_empty } from "std/collections"

let arr = [1, 2, 3]
let arr2 = push(arr, 4)        // [1, 2, 3, 4]
let rev = reverse(arr)         // [3, 2, 1]
let sub = slice(arr2, 1, 3)    // [2, 3]

match first(arr) {
    Some(v) => print("First: " + str(v)),
    None => print("Empty array")
}
```

**std/env** - Environment access

```intent
import { get_env, args, cwd } from "std/env"

let path = cwd()               // Current working directory
let argv = args()              // Command line arguments

match get_env("HOME") {
    Some(home) => print("Home: " + home),
    None => print("HOME not set")
}
```

**std/fs** - File system operations

```intent
import { read_file, write_file, append_file, exists, is_file, is_dir } from "std/fs"
import { mkdir, mkdir_all, remove, remove_dir, readdir, rename, copy } from "std/fs"
import { file_size, read_bytes } from "std/fs"

// Read and write files
match write_file("/tmp/test.txt", "Hello, Intent!") {
    Ok(_) => print("File written"),
    Err(e) => print("Error: " + e)
}

match read_file("/tmp/test.txt") {
    Ok(content) => print(content),
    Err(e) => print("Error: " + e)
}

// Check paths
if exists("/tmp") && is_dir("/tmp") {
    print("/tmp exists and is a directory")
}

// Create directories
mkdir_all("/tmp/intent/nested/dirs")

// List directory contents
match readdir("/tmp") {
    Ok(files) => {
        for file in files {
            print(file)
        }
    },
    Err(e) => print("Error: " + e)
}
```

**std/path** - Path manipulation utilities

```intent
import { join, dirname, basename, extension, stem, resolve } from "std/path"
import { is_absolute, is_relative, with_extension, normalize } from "std/path"

let path = "/home/user/documents/report.pdf"

// Decompose path
match dirname(path) {
    Some(d) => print(d),    // "/home/user/documents"
    None => print("no dir")
}

match basename(path) {
    Some(b) => print(b),    // "report.pdf"
    None => print("no base")
}

match extension(path) {
    Some(e) => print(e),    // "pdf"
    None => print("no ext")
}

// Join paths
let full = join(["home", "user", "file.txt"])  // "home/user/file.txt"

// Change extension
let txt_path = with_extension(path, "txt")     // "/home/user/documents/report.txt"

// Check absolute/relative
print(is_absolute("/usr/bin"))  // true
print(is_relative("./file"))    // true

// Normalize messy paths
let clean = normalize("/home/user/../user/./docs")  // "/home/user/docs"
```

**std/json** - JSON parsing and stringification

```intent
import { parse, stringify, stringify_pretty } from "std/json"

// Parse JSON string
let json_str = r#"{"name": "Alice", "age": 30}"#
match parse(json_str) {
    Ok(data) => {
        print(data.name)    // "Alice"
        print(data.age)     // 30
    },
    Err(e) => print("Parse error: " + e)
}

// Stringify to JSON
let user = map { "name": "Bob", "active": true }
let json = stringify(user)           // {"active":true,"name":"Bob"}
let pretty = stringify_pretty(user)  // Indented JSON output

// Arrays
match parse("[1, 2, 3]") {
    Ok(arr) => print(len(arr)),  // 3
    Err(e) => print("Error")
}
```

**std/time** - Time and date operations

```intent
import { now, now_millis, now_nanos, sleep, elapsed } from "std/time"
import { format_timestamp, duration_secs, duration_millis } from "std/time"

// Get current timestamp
let ts = now()              // Unix seconds
let ts_ms = now_millis()    // Unix milliseconds
let ts_ns = now_nanos()     // Unix nanoseconds

// Format timestamps
let formatted = format_timestamp(ts, "%Y-%m-%d %H:%M:%S")
print(formatted)            // "2024-01-15 12:30:45"

// Sleep and measure elapsed time
let start = now_millis()
sleep(100)                  // Sleep for 100ms
let elapsed_ms = elapsed(start)
print(elapsed_ms)           // ~100

// Duration conversions
let d = duration_secs(60)
print(d.millis)             // 60000
print(d.nanos)              // 60000000000
```

## Editor Support

### VS Code

Install the Intent Language extension for syntax highlighting:

```bash
cp -r editors/vscode/intent-lang ~/.vscode/extensions/
```

Then restart VS Code. The extension provides:

- Syntax highlighting for `.intent` and `.itn` files
- Code snippets for common patterns
- Bracket matching and auto-closing

## Vision

Intent bridges the gap between AI's speed and consistency with human judgment and design sense. The ecosystem includes:

- Integrated development workflows (CI/CD, reviews, pull requests)
- Rich observability and explainability features
- Formal concurrency protocols
- UI/UX constraint declarations
- Intent encoding for self-documenting code

## Documentation

- [Whitepaper](whitepaper.md) - Complete technical specification and motivation
- [Architecture](ARCHITECTURE.md) - System design and components
- [Language Spec](LANGUAGE_SPEC.md) - Formal language definition
- [Roadmap](ROADMAP.md) - 13-phase implementation plan for production web apps

## Built-in Functions

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

### I/O Functions

| Function          | Description            |
| ----------------- | ---------------------- |
| `print(args...)`  | Print to stdout        |
| `str(value)`      | Convert to string      |
| `len(collection)` | Length of string/array |

### Standard Library (import required)

| Module            | Functions                                                      |
| ----------------- | -------------------------------------------------------------- |
| `std/string`      | split, join, trim, replace, contains, starts_with, ends_with   |
|                   | to_upper, to_lower, char_at, substring                         |
| `std/math`        | sin, cos, tan, asin, acos, atan, atan2, log, log10, exp, PI, E |
| `std/collections` | push, pop, first, last, reverse, slice, concat, is_empty       |
| `std/env`         | get_env, args, cwd                                             |
| `std/fs`          | read_file, write_file, exists, mkdir, remove, readdir          |
| `std/path`        | join, dirname, basename, extension, resolve, normalize         |
| `std/json`        | parse, stringify, stringify_pretty                             |
| `std/time`        | now, sleep, elapsed, format_timestamp, duration_secs           |
| `std/crypto`      | sha256, hmac_sha256, uuid, random_bytes, hex_encode            |
| `std/url`         | parse, encode, decode, build_query, join                       |
| `std/http`        | get, post, put, delete, request, get_json, post_json           |
| `std/http/server` | text, html, json, status, redirect + get, post, put, listen    |

## HTTP Server

Intent includes a built-in HTTP server for building web APIs:

```intent
import { text, html, json } from "std/http/server"

// Simple text response
fn home(req) {
    return text("Hello, World!")
}

// JSON API endpoint
fn get_status(req) {
    return json(map {
        "status": "ok",
        "version": "0.1.5"
    })
}

// Path parameters - use raw strings to avoid interpolation
fn get_user(req) {
    let id = req.params.id
    return json(map {
        "id": id,
        "name": "User " + id
    })
}

// Register routes and start server
get("/", home)
get("/status", get_status)
get(r"/users/{id}", get_user)  // Raw string for {param} patterns
post("/users", create_user)

listen(8080)  // Start server on port 8080
```

### Running the Server

```bash
# Run a server file
./target/release/intent run examples/http_server.intent

# Server starts and listens for requests
# Starting server on http://0.0.0.0:8080
# Press Ctrl+C to stop
```

### Request Object

Handlers receive a request map with:
- `req.method` - HTTP method (GET, POST, etc.)
- `req.path` - Request path
- `req.params` - Path parameters (e.g., `{id}` → `req.params.id`)
- `req.query_params` - Query string parameters
- `req.headers` - Request headers
- `req.body` - Request body

### Response Helpers

- `text(body)` - Plain text response (200)
- `html(body)` - HTML response (200)
- `json(data)` - JSON response (200)
- `status(code, body)` - Custom status code
- `redirect(url)` - 302 redirect
- `not_found()` - 404 response
- `error(msg)` - 500 error response

## Contributing

Intent is an open-source project welcoming contributions from developers, language designers, and AI researchers. See our contributing guidelines for details.

## License

MIT

## Contact

For questions or discussions, please open an issue on this repository.
