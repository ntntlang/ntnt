# NTNT Standard Library Reference

> **Auto-generated from [stdlib.toml](stdlib.toml)** - Do not edit directly.
>
> Last updated: v0.3.5

## Table of Contents

- [Global Builtins](#global-builtins)
- [std/collections](#stdcollections)
- [std/concurrent](#stdconcurrent)
- [std/csv](#stdcsv)
- [std/env](#stdenv)
- [std/fs](#stdfs)
- [std/http](#stdhttp)
- [std/http/server](#stdhttpserver)
- [std/json](#stdjson)
- [std/math](#stdmath)
- [std/path](#stdpath)
- [std/string](#stdstring)
- [std/time](#stdtime)
- [std/url](#stdurl)

---

## Global Builtins

These functions are available everywhere without importing.

| Function | Description |
|----------|-------------|
| `abs(n: Int | Float)` | Returns the absolute value of a number |
| `assert(condition: Bool)` | Throws an error if the condition is false |
| `ceil(n: Float)` | Rounds a number up to the nearest integer |
| `clamp(value: Number, min: Number, max: Number)` | Constrains a value between min and max |
| `delete(pattern: String, handler: Fn)` | Registers a DELETE route handler |
| `float(x: Int | Float | String)` | Converts a value to a floating-point number |
| `floor(n: Float)` | Rounds a number down to the nearest integer |
| `get(pattern: String, handler: Fn)` | Registers a GET route handler |
| `int(x: Int | Float | String | Bool)` | Converts a value to an integer |
| `len(x: String | Array)` | Returns the length of a string or array |
| `listen(port: Int)` | Starts the HTTP server on the specified port |
| `max(a: Number, b: Number)` | Returns the larger of two numbers |
| `min(a: Number, b: Number)` | Returns the smaller of two numbers |
| `on_shutdown(handler: Fn)` | Registers a function to run when the server shuts down |
| `patch(pattern: String, handler: Fn)` | Registers a PATCH route handler |
| `post(pattern: String, handler: Fn)` | Registers a POST route handler |
| `pow(base: Number, exp: Number)` | Returns base raised to the power of exp |
| `print(value: Any)` | Prints a value to stdout with a newline |
| `push(arr: Array, item: Any)` | Returns a new array with the item appended |
| `put(pattern: String, handler: Fn)` | Registers a PUT route handler |
| `round(n: Float)` | Rounds a number to the nearest integer |
| `routes(dir: String)` | Loads file-based routes from a directory |
| `serve_static(prefix: String, dir: String)` | Serves static files from a directory |
| `sign(n: Number)` | Returns -1, 0, or 1 based on the sign of the number |
| `sqrt(n: Number)` | Returns the square root of a number |
| `str(x: Any)` | Converts any value to its string representation |
| `template(path: String, vars: Map)` | Renders an external template file with variable substitution |
| `trunc(n: Float)` | Truncates a number toward zero |
| `type(x: Any)` | Returns the type name of a value as a string |
| `use_middleware(handler: Fn)` | Registers middleware that runs before route handlers |

---

## std/collections

Collection manipulation utilities

```ntnt
import { entries, first, get_key } from "std/collections"
```

### Functions

| Function | Description |
|----------|-------------|
| `entries(map: Map) -> [[String, Any]]` | Returns array of [key, value] pairs |
| `first(arr: Array) -> Option<Any>` | Returns the first element or None |
| `get_key(map: Map, key: String) -> Option<Any>` | Gets value by key or None |
| `has_key(map: Map, key: String) -> Bool` | Returns true if map contains key |
| `keys(map: Map) -> [String]` | Returns array of map keys |
| `last(arr: Array) -> Option<Any>` | Returns the last element or None |
| `pop(arr: Array) -> Array` | Returns new array with last item removed |
| `push(arr: Array, item: Any) -> Array` | Returns new array with item appended |
| `values(map: Map) -> [Any]` | Returns array of map values |

---

## std/concurrent

Concurrency primitives

```ntnt
import { channel, recv, send } from "std/concurrent"
```

### Functions

| Function | Description |
|----------|-------------|
| `channel() -> [Sender, Receiver]` | Creates a channel for communication between tasks |
| `recv(receiver: Receiver) -> Any` | Receives a value from a channel (blocks until available) |
| `send(sender: Sender, value: Any) -> Unit` | Sends a value through a channel |
| `sleep_ms(ms: Int) -> Unit` | Pauses execution for specified milliseconds |

---

## std/csv

CSV parsing and generation

```ntnt
import { parse, parse_with_headers, stringify } from "std/csv"
```

### Functions

| Function | Description |
|----------|-------------|
| `parse(csv: String) -> [[String]]` | Parses CSV into array of rows (arrays of strings) |
| `parse_with_headers(csv: String) -> [Map]` | Parses CSV into array of maps using first row as headers |
| `stringify(rows: [[Any]]) -> String` | Converts array of rows to CSV string |
| `stringify_with_headers(rows: [Map], headers: [String]) -> String` | Converts array of maps to CSV with header row |

---

## std/env

Environment variables and process info

```ntnt
import { all_env, args, cwd } from "std/env"
```

### Functions

| Function | Description |
|----------|-------------|
| `all_env() -> Map` | Returns all environment variables as a map |
| `args() -> [String]` | Returns command-line arguments |
| `cwd() -> String` | Returns the current working directory |
| `get_env(name: String) -> Option<String>` | Gets an environment variable |
| `load_env(path?: String) -> Result<Unit, String>` | Loads environment variables from a .env file |
| `set_env(name: String, value: String) -> Unit` | Sets an environment variable for the current process |

---

## std/fs

File system operations

```ntnt
import { exists, is_dir, is_file } from "std/fs"
```

### Functions

| Function | Description |
|----------|-------------|
| `exists(path: String) -> Bool` | Returns true if path exists |
| `is_dir(path: String) -> Bool` | Returns true if path is a directory |
| `is_file(path: String) -> Bool` | Returns true if path is a file |
| `mkdir(path: String) -> Result<Unit, String>` | Creates a directory (including parents) |
| `read_file(path: String) -> Result<String, String>` | Reads entire file contents as a string |
| `readdir(path: String) -> Result<[String], String>` | Lists directory contents |
| `write_file(path: String, content: String) -> Result<Unit, String>` | Writes content to a file, creating or overwriting |

---

## std/http

HTTP client for making requests

```ntnt
import { download, fetch } from "std/http"
```

### Functions

| Function | Description |
|----------|-------------|
| `download(url: String, path: String) -> Result<Unit, String>` | Downloads a file from URL to local path |
| `fetch(url: String \| Map) -> Result<Response, String>` | Makes an HTTP request. Simple form takes URL for GET. Map form supports method, body, json, form, headers, auth, cookies, timeout. |

---

## std/http/server

HTTP response builders and request parsing utilities

```ntnt
import { html, json, parse_form } from "std/http/server"
```

### Functions

| Function | Description |
|----------|-------------|
| `html(content: String, status?: Int) -> Response` | Creates an HTML response |
| `json(data: Any, status?: Int) -> Response` | Creates a JSON response |
| `parse_form(req: Request) -> Map` | Parses URL-encoded form data from request body |
| `parse_json(req: Request) -> Result<Any, String>` | Parses JSON from request body |
| `redirect(url: String) -> Response` | Creates a 302 redirect response |
| `status(code: Int, body: String) -> Response` | Creates a response with custom status code |
| `text(content: String) -> Response` | Creates a plain text response |

---

## std/json

JSON parsing and serialization

```ntnt
import { parse, stringify, stringify_pretty } from "std/json"
```

### Functions

| Function | Description |
|----------|-------------|
| `parse(json: String) -> Result<Any, String>` | Parses a JSON string into a value |
| `stringify(value: Any) -> String` | Converts a value to a JSON string |
| `stringify_pretty(value: Any) -> String` | Converts a value to a pretty-printed JSON string |

---

## std/math

Mathematical functions and constants

```ntnt
import { acos, asin, atan } from "std/math"
```

### Constants

| Constant | Value | Description |
|----------|-------|-------------|
| `E` | 2.718281828459045 | Euler's number (e) |
| `INFINITY` | Infinity | Positive infinity |
| `NEG_INFINITY` | -Infinity | Negative infinity |
| `PI` | 3.141592653589793 | The mathematical constant π |
| `TAU` | 6.283185307179586 | The circle constant τ = 2π |

### Functions

| Function | Description |
|----------|-------------|
| `acos(x: Number) -> Float` | Returns the arc cosine of x |
| `asin(x: Number) -> Float` | Returns the arc sine of x |
| `atan(x: Number) -> Float` | Returns the arc tangent of x |
| `atan2(y: Number, x: Number) -> Float` | Returns the arc tangent of y/x using signs for quadrant |
| `cbrt(x: Number) -> Float` | Returns the cube root of x |
| `cos(x: Number) -> Float` | Returns the cosine of x (in radians) |
| `cosh(x: Number) -> Float` | Returns the hyperbolic cosine of x |
| `degrees(x: Number) -> Float` | Converts radians to degrees |
| `exp(x: Number) -> Float` | Returns e raised to the power x |
| `exp2(x: Number) -> Float` | Returns 2 raised to the power x |
| `hypot(x: Number, y: Number) -> Float` | Returns sqrt(x² + y²) |
| `is_finite(x: Number) -> Bool` | Returns true if x is finite (not NaN or infinite) |
| `is_infinite(x: Number) -> Bool` | Returns true if x is infinite |
| `is_nan(x: Number) -> Bool` | Returns true if x is NaN |
| `log(x: Number) -> Float` | Returns the natural logarithm of x |
| `log10(x: Number) -> Float` | Returns the base-10 logarithm of x |
| `log2(x: Number) -> Float` | Returns the base-2 logarithm of x |
| `radians(x: Number) -> Float` | Converts degrees to radians |
| `random() -> Float` | Returns a random float between 0 and 1 |
| `random_int(min: Int, max: Int) -> Int` | Returns a random integer between min and max (inclusive) |
| `random_range(min: Number, max: Number) -> Float` | Returns a random float between min and max |
| `sin(x: Number) -> Float` | Returns the sine of x (in radians) |
| `sinh(x: Number) -> Float` | Returns the hyperbolic sine of x |
| `tan(x: Number) -> Float` | Returns the tangent of x (in radians) |
| `tanh(x: Number) -> Float` | Returns the hyperbolic tangent of x |

---

## std/path

File path manipulation

```ntnt
import { basename, dirname, extname } from "std/path"
```

### Functions

| Function | Description |
|----------|-------------|
| `basename(path: String) -> String` | Returns the filename portion of a path |
| `dirname(path: String) -> String` | Returns the directory portion of a path |
| `extname(path: String) -> String` | Returns the file extension |
| `join(parts: [String]) -> String` | Joins path segments |

---

## std/string

Comprehensive string manipulation functions

```ntnt
import { capitalize, center, char_at } from "std/string"
```

### Functions

| Function | Description |
|----------|-------------|
| `capitalize(str: String) -> String` | Capitalizes the first character |
| `center(str: String, len: Int, char: String) -> String` | Centers string with padding on both sides |
| `char_at(str: String, index: Int) -> String` | Returns character at index |
| `chars(str: String) -> [String]` | Splits string into array of characters |
| `concat(a: String, b: String) -> String` | Concatenates two strings |
| `contains(str: String, substr: String) -> Bool` | Checks if string contains substring |
| `count(str: String, substr: String) -> Int` | Counts occurrences of substring |
| `ends_with(str: String, suffix: String) -> Bool` | Checks if string ends with suffix |
| `find_all_pattern(str: String, pattern: String) -> [String]` | Returns all regex matches |
| `find_pattern(str: String, pattern: String) -> Option<String>` | Returns first regex match or None |
| `index_of(str: String, substr: String) -> Int` | Returns index of first occurrence, or -1 if not found |
| `is_alpha(str: String) -> Bool` | Returns true if string contains only letters |
| `is_alphanumeric(str: String) -> Bool` | Returns true if string contains only letters and digits |
| `is_blank(str: String) -> Bool` | Returns true if string is empty or only whitespace |
| `is_empty(str: String) -> Bool` | Returns true if string is empty |
| `is_lowercase(str: String) -> Bool` | Returns true if all letters are lowercase |
| `is_numeric(str: String) -> Bool` | Returns true if string contains only digits |
| `is_uppercase(str: String) -> Bool` | Returns true if all letters are uppercase |
| `is_whitespace(str: String) -> Bool` | Returns true if string contains only whitespace |
| `join(arr: Array, delim: String) -> String` | Joins array elements into a string with a delimiter |
| `keep_chars(str: String, allowed: String) -> String` | Keeps only characters in the allowed set |
| `last_index_of(str: String, substr: String) -> Int` | Returns index of last occurrence, or -1 if not found |
| `lines(str: String) -> [String]` | Splits string by newlines |
| `matches(str: String, pattern: String) -> Bool` | Simple glob matching with * and ? |
| `matches_pattern(str: String, pattern: String) -> Bool` | Checks if string matches regex pattern |
| `pad_left(str: String, len: Int, char: String) -> String` | Pads string on the left to reach target length |
| `pad_right(str: String, len: Int, char: String) -> String` | Pads string on the right to reach target length |
| `remove_chars(str: String, chars: String) -> String` | Removes all characters in the chars set |
| `repeat(str: String, n: Int) -> String` | Repeats a string n times |
| `replace(str: String, from: String, to: String) -> String` | Replaces all occurrences of from with to |
| `replace_chars(str: String, chars: String, repl: String) -> String` | Replaces any character in chars set with replacement |
| `replace_first(str: String, from: String, to: String) -> String` | Replaces first occurrence of from with to |
| `replace_pattern(str: String, pattern: String, repl: String) -> String` | Replaces all regex matches with replacement |
| `reverse(str: String) -> String` | Reverses a string |
| `slugify(str: String) -> String` | Converts to URL-friendly slug |
| `split(str: String, delim: String) -> [String]` | Splits a string into an array using a delimiter |
| `split_pattern(str: String, pattern: String) -> [String]` | Splits string by regex pattern |
| `starts_with(str: String, prefix: String) -> Bool` | Checks if string starts with prefix |
| `substring(str: String, start: Int, end: Int) -> String` | Extracts substring from start to end (exclusive) |
| `title(str: String) -> String` | Capitalizes the first letter of each word |
| `to_camel_case(str: String) -> String` | Converts to camelCase |
| `to_kebab_case(str: String) -> String` | Converts to kebab-case |
| `to_lower(str: String) -> String` | Converts string to lowercase |
| `to_pascal_case(str: String) -> String` | Converts to PascalCase |
| `to_snake_case(str: String) -> String` | Converts to snake_case |
| `to_upper(str: String) -> String` | Converts string to uppercase |
| `trim(str: String) -> String` | Removes leading and trailing whitespace |
| `trim_chars(str: String, chars: String) -> String` | Removes specified characters from both ends |
| `trim_left(str: String) -> String` | Removes leading whitespace |
| `trim_right(str: String) -> String` | Removes trailing whitespace |
| `truncate(str: String, max_len: Int, suffix: String) -> String` | Truncates string to max length with suffix |
| `words(str: String) -> [String]` | Splits string by whitespace |

---

## std/time

Date and time operations

```ntnt
import { add_days, add_months, format } from "std/time"
```

### Functions

| Function | Description |
|----------|-------------|
| `add_days(dt: DateTime, days: Int) -> DateTime` | Adds days to a datetime |
| `add_months(dt: DateTime, months: Int) -> DateTime` | Adds months to a datetime |
| `format(dt: DateTime, fmt: String) -> String` | Formats a datetime using strftime-style format |
| `now() -> DateTime` | Returns the current date/time |
| `parse(str: String, fmt: String) -> Result<DateTime, String>` | Parses a string into a datetime |

---

## std/url

URL encoding and query string operations

```ntnt
import { build_query, decode, encode } from "std/url"
```

### Functions

| Function | Description |
|----------|-------------|
| `build_query(params: Map) -> String` | Builds a query string from a map |
| `decode(str: String) -> String` | URL-decodes a string |
| `encode(str: String) -> String` | URL-encodes a string |
| `parse_query(query: String) -> Map` | Parses a query string into a map |

---

