# NTNT Language Specification

## Version 0.1.8

This document specifies the syntax, semantics, and core features of the NTNT programming language.

## Table of Contents

1. [Lexical Structure](#lexical-structure)
2. [Types](#types)
3. [Contracts](#contracts)
4. [Functions and Methods](#functions-and-methods)
5. [Built-in Functions](#built-in-functions)
6. [Traits](#traits)
7. [Control Flow](#control-flow)
8. [Effects](#effects)
9. [Concurrency](#concurrency)
10. [Modules](#modules)
11. [Standard Library](#standard-library)

## Lexical Structure

### Keywords

```
// Contracts
requires, ensures, invariant, old, result

// Functions & Control
fn, let, mut, if, else, match, for, in, while, loop, return, break, continue, defer

// Types & Structures
type, struct, enum, impl, trait, pub, self

// Modules
import, from, export

// Literals
true, false, map, Ok, Err, Some, None
```

### Identifiers

- Start with letter or underscore
- Contain letters, digits, underscores
- Case-sensitive

### Literals

- Integers: `42`, `-17`
- Floats: `3.14`, `1.0e-10`
- Strings: `"hello"`, `"with {interpolation}"`
- Raw Strings: `r"no \n escapes"`, `r#"can use "quotes""#`
- Booleans: `true`, `false`
- Arrays: `[1, 2, 3]`
- Maps: `map { "key": value, "key2": value2 }`
- Ranges: `0..10` (exclusive), `0..=10` (inclusive)

## Types

### Primitive Types

- `Int`: Arbitrary precision integers
- `Float`: IEEE 754 floating point
- `Bool`: Boolean values
- `String`: UTF-8 encoded text
- `Unit`: The unit type `()`

### Compound Types

- Arrays: `[T]`
- Maps: `Map<K, V>` with literal syntax `map { key: value }`
- Structs: Named product types
- Enums: Tagged union types with `Option<T>` and `Result<T, E>` built-in
- Functions: `fn(T1, T2) -> T3`
- Ranges: `Range` (from `..` and `..=` expressions)

### Type Annotations

```ntnt
let x: Int = 42;
let name: String = "NTNT";
```

## Contracts

Contracts specify behavioral requirements for code. NTNT enforces contracts at runtime with detailed error messages.

### Function Contracts

The `requires` clause specifies preconditions that must be true when a function is called.
The `ensures` clause specifies postconditions that must be true when a function returns.

```ntnt
fn transfer_funds(amount: Int, from: Account, to: Account) -> Result<(), Error>
requires amount > 0 && from.balance >= amount
ensures to.balance == old(to.balance) + amount
{
    // implementation
}
```

### The `old()` Function

The `old()` function captures the value of an expression at function entry, allowing postconditions to compare pre-state and post-state:

```ntnt
fn increment(counter: Counter)
ensures counter.value == old(counter.value) + 1
{
    counter.value = counter.value + 1
}
```

### The `result` Keyword

In postconditions, `result` refers to the return value of the function:

```ntnt
fn double(x: Int) -> Int
ensures result == x * 2
{
    return x * 2
}
```

### Conditional Postconditions

Use `implies` for conditional guarantees:

```ntnt
fn safe_divide(a: Int, b: Int) -> Int
requires b != 0
ensures b > 0 implies result >= 0
{
    return a / b
}
```

### Struct Invariants

Invariants are automatically checked after construction and after any method call or field assignment:

```ntnt
struct Account {
    balance: Int,
    owner: String
}

impl Account {
    invariant self.balance >= 0
}
```

## Functions and Methods

### Function Definition

```ntnt
fn add(x: Int, y: Int) -> Int {
    return x + y;
}
```

### Methods

```ntnt
impl Point {
    fn distance(&self, other: &Point) -> Float {
        // implementation
    }
}
```

## Built-in Functions

NTNT provides built-in functions available without imports.

### I/O Functions

| Function | Signature             | Description               |
| -------- | --------------------- | ------------------------- |
| `print`  | `(...args) -> Unit`   | Print values to stdout    |
| `len`    | `(collection) -> Int` | Length of string or array |

### Math Functions

| Function | Signature                                         | Description              |
| -------- | ------------------------------------------------- | ------------------------ |
| `abs`    | `(x: Number) -> Number`                           | Absolute value           |
| `min`    | `(a: Number, b: Number) -> Number`                | Minimum of two values    |
| `max`    | `(a: Number, b: Number) -> Number`                | Maximum of two values    |
| `round`  | `(x: Float) -> Int`                               | Round to nearest integer |
| `floor`  | `(x: Float) -> Int`                               | Round down to integer    |
| `ceil`   | `(x: Float) -> Int`                               | Round up to integer      |
| `sqrt`   | `(x: Number) -> Float`                            | Square root              |
| `pow`    | `(base: Number, exp: Number) -> Number`           | Exponentiation           |
| `sign`   | `(x: Number) -> Int`                              | Sign (-1, 0, or 1)       |
| `clamp`  | `(x: Number, min: Number, max: Number) -> Number` | Clamp to range           |

### Examples

```ntnt
// Math operations
let x = abs(-42)           // 42
let smaller = min(10, 20)  // 10
let larger = max(10, 20)   // 20
let rounded = round(3.7)   // 4
let root = sqrt(16)        // 4.0
let squared = pow(2, 3)    // 8
let bounded = clamp(15, 0, 10)  // 10
```

## Effects

Effects track side effects and error conditions (foundation implemented, full effect system planned).

```ntnt
fn read_file(path: String) -> Result<String, Error> with io {
    // implementation
}
```

## Traits

Traits define shared behavior that types can implement.

### Trait Declaration

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

## Control Flow

### For-In Loops

```ntnt
// Iterate over arrays
for item in items {
    print(item)
}

// Iterate over ranges
for i in 0..10 {
    print(i)  // 0 through 9
}

for i in 0..=10 {
    print(i)  // 0 through 10 (inclusive)
}

// Iterate over strings (characters)
for char in "hello" {
    print(char)
}

// Iterate over maps
for key in my_map {
    print(key + ": " + str(my_map[key]))
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

## Concurrency

NTNT uses Go-style concurrency with channels (no async/await).

### Channels

```ntnt
import { channel, send, recv, try_recv, recv_timeout, close } from "std/concurrent"

// Create a channel
let ch = channel()

// Send values (blocks if no receiver ready)
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

// Close channel
close(ch)
```

## Modules

### Import Syntax

```ntnt
// Import specific items
import { split, join, trim } from "std/string"

// Import with alias
import { get as http_get } from "std/http"

// Import entire module
import "std/math" as math
```

### File-Based Modules

```ntnt
// lib/utils.tnt
pub fn helper() -> String {
    return "help"
}

// main.tnt
import { helper } from "./lib/utils"
```

## Standard Library

### Core Modules

| Module            | Functions                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `std/string`      | **Comprehensive:** split, join, concat, repeat, reverse, trim, trim_left, trim_right, trim_chars, to_upper, to_lower, capitalize, title, to_snake_case, to_camel_case, to_pascal_case, to_kebab_case, slugify, contains, starts_with, ends_with, index_of, last_index_of, count, replace, replace_all, char_at, substring, chars, lines, words, truncate, pad_left, pad_right, center, is_empty, is_blank, is_numeric, is_alpha, is_alphanumeric, is_lowercase, is_uppercase, is_whitespace, matches |
| `std/math`        | sin, cos, tan, asin, acos, atan, atan2, log, log10, exp, PI, E                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `std/collections` | push, pop, shift, first, last, reverse, slice, concat, is_empty, contains, index_of, sort, map, filter, reduce, find                                                                                                                                                                                                                                                                                                                                                                                 |
| `std/env`         | get_env, load_env, args, cwd                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `std/fs`          | read_file, write_file, append_file, exists, is_file, is_dir, mkdir, mkdir_all, readdir, remove, remove_dir, remove_dir_all, rename, copy, file_size                                                                                                                                                                                                                                                                                                                                                  |
| `std/path`        | join, dirname, basename, extension, stem, resolve, is_absolute, is_relative, with_extension, normalize                                                                                                                                                                                                                                                                                                                                                                                               |
| `std/json`        | parse, stringify, stringify_pretty                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `std/time`        | **Go-like time module:** now, now_millis, now_nanos, to_utc, to_timezone, format, format_in, to_iso, parse, parse_iso, make_time, make_date, add_seconds/minutes/hours/days/weeks/months/years, diff, before, after, equal, year/month/day/hour/minute/second, weekday, weekday_name, month_name, day_of_year, is_leap_year, list_timezones, sleep, elapsed, SECOND/MINUTE/HOUR/DAY/WEEK constants                                                                                                   |
| `std/crypto`      | sha256, sha256_bytes, hmac_sha256, uuid, random_bytes, random_hex, hex_encode, hex_decode                                                                                                                                                                                                                                                                                                                                                                                                            |
| `std/url`         | parse, encode, encode_component, decode, build_query, parse_query, join                                                                                                                                                                                                                                                                                                                                                                                                                              |

### String Module (`std/string`)

Comprehensive string manipulation matching Go and JavaScript capabilities.

```ntnt
import {
    trim, trim_left, trim_right, trim_chars,
    to_upper, to_lower, capitalize, title,
    to_snake_case, to_camel_case, to_kebab_case, slugify,
    split, join, lines, words, chars,
    contains, starts_with, ends_with, index_of, last_index_of, count,
    replace, replace_all, truncate,
    pad_left, pad_right, center,
    is_empty, is_blank, is_numeric, is_alpha, is_alphanumeric, matches
} from "std/string"

// Trimming
trim("  hello  ")           // "hello"
trim_left("  hello")        // "hello"
trim_chars("###hi###", "#") // "hi"

// Case conversion
capitalize("hello world")   // "Hello world"
title("hello world")        // "Hello World"
to_snake_case("helloWorld") // "hello_world"
to_camel_case("hello_world") // "helloWorld"
slugify("Hello World!")     // "hello-world"

// Search & Count
contains(s, "fox")          // true/false
last_index_of(s, "the")     // last position
count(s, "the")             // occurrence count
replace("hi hi", "hi", "yo") // "yo hi" (first only)
replace_all("hi hi", "hi", "yo") // "yo yo"

// Extraction
lines("a\nb\nc")            // ["a", "b", "c"]
words("hello   world")      // ["hello", "world"]
chars("abc")                // ["a", "b", "c"]
truncate("hello world", 8, "...") // "hello..."

// Padding
pad_left("42", 5, "0")      // "00042"
pad_right("hi", 5, ".")     // "hi..."
center("hi", 8, "*")        // "***hi***"

// Validation
is_empty("")                // true
is_blank("   ")             // true (only whitespace)
is_numeric("123")           // true
is_alpha("abc")             // true
is_alphanumeric("abc123")   // true
matches("test.txt", "*.txt") // true (glob-like)
```

### Time Module (`std/time`)

The time module provides comprehensive Go-like time handling with IANA timezone support.

```ntnt
import { now, to_timezone, format, add_days, weekday_name, HOUR } from "std/time"

// Current timestamp
let ts = now()  // Unix timestamp (seconds)

// Timezone conversion (returns map with year, month, day, hour, minute, second, etc.)
let ny = to_timezone(ts, "America/New_York")
print("NYC time: " + str(ny["hour"]) + ":" + str(ny["minute"]))  // EST/EDT

// Formatting (strftime format)
print(format(ts, "%Y-%m-%d %H:%M:%S"))    // 2026-01-13 04:06:53
print(format_in(ts, "Asia/Tokyo", "%H:%M %Z"))  // 13:06 JST

// Date creation
match make_date(2024, 12, 25) {
    Ok(xmas) => print(weekday_name(xmas)),  // Wednesday
    Err(e) => print(e)
}

// Date arithmetic (calendar-aware)
let tomorrow = add_days(ts, 1)
let next_month = add_months(ts, 1)  // Handles month overflow
let in_2_hours = add_seconds(ts, 2 * HOUR)

// Parsing
match parse_iso("2024-03-20T10:30:00+00:00") {
    Ok(parsed) => print(parsed),
    Err(e) => print(e)
}

// Comparisons
print(before(ts, add_days(ts, 1)))  // true

// Common timezones
let tzs = list_timezones()  // ["UTC", "America/New_York", "Asia/Tokyo", ...]
```

### HTTP Client (`std/http`)

```ntnt
import { get, post, put, delete, patch, head, request, fetch } from "std/http"
import { get_json, post_json, post_form, basic_auth, download, upload } from "std/http"

// Simple GET
match get("https://api.example.com/data") {
    Ok(response) => print(response.body),
    Err(e) => print("Error: " + e)
}

// POST with body
match post("https://api.example.com/users", "{\"name\": \"Alice\"}") {
    Ok(response) => print(response.status),
    Err(e) => print("Error: " + e)
}

// POST form data (application/x-www-form-urlencoded)
match post_form("https://api.example.com/login", map {
    "username": "alice",
    "password": "secret"
}) {
    Ok(response) => print(response.status),
    Err(e) => print("Error: " + e)
}

// Basic Authentication
match basic_auth("https://api.example.com/secure", "user", "pass") {
    Ok(response) => print(response.body),
    Err(e) => print("Auth failed: " + e)
}

// Download file
match download("https://example.com/file.pdf", "./downloads/file.pdf") {
    Ok(result) => print("Downloaded " + result.size + " bytes"),
    Err(e) => print("Download failed: " + e)
}

// Upload file (multipart form)
match upload("https://api.example.com/upload", "./photo.jpg", "file") {
    Ok(response) => print(response.body),
    Err(e) => print("Upload failed: " + e)
}

// Full request with headers
match request(map {
    "url": "https://api.example.com/data",
    "method": "POST",
    "headers": map {
        "Authorization": "Bearer token123",
        "Content-Type": "application/json"
    },
    "body": "{\"key\": \"value\"}",
    "timeout": 30
}) {
    Ok(response) => print(response.body),
    Err(e) => print("Error: " + e)
}

// fetch() - Full request with cookies support
match fetch(map {
    "url": "https://api.example.com/session",
    "method": "GET",
    "cookies": map { "session_id": "abc123" },
    "headers": map { "Accept": "application/json" }
}) {
    Ok(response) => {
        print(response.body)
        // Response includes cookies from server
        if response.cookies != nil {
            print(response.cookies)
        }
    },
    Err(e) => print("Error: " + e)
}
```

**HTTP Functions:**
| Function | Description |
|----------|-------------|
| `get(url)` | Simple GET request |
| `post(url, body)` | POST with text body |
| `put(url, body)` | PUT request |
| `delete(url)` | DELETE request |
| `patch(url, body)` | PATCH request |
| `head(url)` | HEAD request (headers only) |
| `get_json(url)` | GET with JSON response parsing |
| `post_json(url, data)` | POST JSON data |
| `post_form(url, form)` | POST form-urlencoded data |
| `basic_auth(url, user, pass)` | GET with Basic auth |
| `download(url, path)` | Download to file |
| `upload(url, path, field)` | Multipart file upload |
| `request(opts)` | Full control (method, headers, timeout) |
| `fetch(opts)` | Full control + cookies |

### HTTP Server (`std/http/server`)

```ntnt
import { text, html, json, status, redirect } from "std/http/server"

fn home(req) {
    return text("Welcome!")
}

fn get_user(req) {
    let id = req.params.id
    return json(map { "id": id, "name": "User " + id })
}

fn create_user(req)
    requires len(req.body) > 0
{
    // Contract violations return 400 Bad Request
    return json(map { "created": true })
}

// Register routes
get("/", home)
get(r"/users/{id}", get_user)
post("/users", create_user)

// Serve static files
serve_static("/assets", "./public")

// Start server
listen(8080)
```

### Database (`std/db/postgres`)

```ntnt
import { connect, query, execute, begin, commit, rollback, close } from "std/db/postgres"

let db = connect("postgresql://user:pass@localhost/mydb")

// Query with parameters
match query(db, "SELECT * FROM users WHERE id = $1", [user_id]) {
    Ok(rows) => {
        for row in rows {
            print(row.name)
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

### Concurrency (`std/concurrent`)

```ntnt
import { channel, send, recv, close, sleep_ms, thread_count } from "std/concurrent"

print("Running on " + str(thread_count()) + " threads")

let ch = channel()

// Producer pattern
send(ch, map { "type": "task", "data": process_item() })

// Consumer pattern
let msg = recv(ch)
print("Received: " + str(msg))

sleep_ms(1000)  // Sleep for 1 second
close(ch)
```

---

_This specification reflects NTNT v0.1.8. See [ROADMAP.md](ROADMAP.md) for the implementation plan._
