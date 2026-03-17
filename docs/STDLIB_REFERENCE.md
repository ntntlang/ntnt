# NTNT Standard Library Reference

> **Auto-generated from source code doc comments** - Do not edit directly.
>
> Last updated: v0.4.6

## Table of Contents

- [Global Builtins](#global-builtins)
- [std/auth](#stdauth)
- [std/collections](#stdcollections)
- [std/concurrent](#stdconcurrent)
- [std/crypto](#stdcrypto)
- [std/csv](#stdcsv)
- [std/env](#stdenv)
- [std/fs](#stdfs)
- [std/http](#stdhttp)
- [std/http/server](#stdhttpserver)
- [std/jobs](#stdjobs)
- [std/json](#stdjson)
- [std/kv](#stdkv)
- [std/log](#stdlog)
- [std/markdown](#stdmarkdown)
- [std/math](#stdmath)
- [std/path](#stdpath)
- [std/postgres](#stdpostgres)
- [std/sqlite](#stdsqlite)
- [std/string](#stdstring)
- [std/time](#stdtime)
- [std/url](#stdurl)

---

## Global Builtins

These functions are available everywhere without importing.

| Function | Description |
|----------|-------------|
| [`Err(error: Any)`](#err) | Wraps a value in Result::Err. |
| [`Ok(value: Any)`](#ok) | Wraps a value in Result::Ok. |
| [`Some(value: Any)`](#some) | Wraps a value in Option::Some. |
| [`abs(x: Int \| Float)`](#abs) | Returns the absolute value of a number. |
| [`assert(condition: Bool)`](#assert) | Asserts a condition is truthy, throws ContractViolation if not. |
| [`ceil(x: Int \| Float)`](#ceil) | Rounds up to the nearest integer. |
| [`clamp(x: Int \| Float, min_val: Int \| Float, max_val: Int \| Float)`](#clamp) | Constrains a value between a minimum and maximum. |
| [`delete(pattern: String, handler: Function)`](#delete) | Registers a DELETE route handler. |
| [`enable_cors(options?: Map)`](#enablecors) | Enable CORS (Cross-Origin Resource Sharing) for the HTTP server. |
| [`enable_csp(options?: Map \| Bool)`](#enablecsp) | Enable Content-Security-Policy headers for the HTTP server. |
| [`float(x: Int \| Float \| String)`](#float) | Converts a value to float. |
| [`floor(x: Int \| Float)`](#floor) | Rounds down to the nearest integer. |
| [`get(pattern: String, handler: Function)`](#get) | Registers a GET route handler. |
| [`int(x: Int \| Float \| String \| Bool)`](#int) | Converts a value to integer. |
| [`is_array(val: Any)`](#isarray) | Returns true if the value is an Array. |
| [`is_bool(val: Any)`](#isbool) | Returns true if the value is a Bool. |
| [`is_err(res: Result<Any, Any>)`](#iserr) | Checks if a Result is Err. |
| [`is_float(val: Any)`](#isfloat) | Returns true if the value is a Float. |
| [`is_int(val: Any)`](#isint) | Returns true if the value is an integer. |
| [`is_map(val: Any)`](#ismap) | Returns true if the value is a Map (dictionary/object). |
| [`is_none(opt: Option<Any>)`](#isnone) | Checks if an Option is None. |
| [`is_ok(res: Result<Any, Any>)`](#isok) | Checks if a Result is Ok. |
| [`is_some(opt: Option<Any>)`](#issome) | Checks if an Option is Some. |
| [`is_string(val: Any)`](#isstring) | Returns true if the value is a String. |
| [`len(x: String \| Array \| Map)`](#len) | Returns the length of a string, array, or map. |
| [`listen(port: Int)`](#listen) | Starts an HTTP server on the given port. |
| [`max(a: Int \| Float, b: Int \| Float)`](#max) | Returns the larger of two numbers. |
| [`min(a: Int \| Float, b: Int \| Float)`](#min) | Returns the smaller of two numbers. |
| [`new_server()`](#newserver) | Resets the server, clearing all registered routes. |
| [`patch(pattern: String, handler: Function)`](#patch) | Registers a PATCH route handler. |
| [`post(pattern: String, handler: Function)`](#post) | Registers a POST route handler. |
| [`pow(base: Int \| Float, exp: Int \| Float)`](#pow) | Raises base to the power of exponent. |
| [`print(value: Any)`](#print) | Prints values to stdout, one per line. |
| [`push(arr: Array, item: Any)`](#push) | Appends an item to an array, returns a new array. |
| [`put(pattern: String, handler: Function)`](#put) | Registers a PUT route handler. |
| [`round(x: Int \| Float, decimals?: Int)`](#round) | Rounds to the nearest integer, or to N decimal places. |
| [`sign(x: Int \| Float)`](#sign) | Returns the sign of a number: -1, 0, or 1. |
| [`sqrt(x: Int \| Float)`](#sqrt) | Returns the square root of a number. |
| [`str(x: Any)`](#str) | Converts any value to its string representation. |
| [`trunc(x: Int \| Float)`](#trunc) | Truncates a number toward zero. |
| [`type(x: Any)`](#type) | Returns the type name of a value as a string. |
| [`typeof(x: Any)`](#typeof) | Returns the type name of a value as a string. |
| [`unwrap(x: Option<Any> \| Result<Any, Any>)`](#unwrap) | Extracts the value from Some or Ok, panics on None or Err. |
| [`unwrap_or(x: Option<Any> \| Result<Any, Any>, default: Any)`](#unwrapor) | Extracts the value from Some or Ok, returns default on None or Err. |

#### `Err`

```ntnt
Err(error: Any) -> Result<Any, Any>
```

Wraps a value in Result::Err.

Creates a Result representing a failed outcome. Use to return error values from operations that can fail.

**Parameters:**

- `error` — The error value to wrap

**Returns:** A Result containing the error value

**Examples:**

```ntnt
Err("not found")  // => Err("not found")  // Wrap error message
Err(404)  // => Err(404)  // Wrap error code
```

**See also:** `Ok`, `is_ok`, `is_err`, `unwrap`, `unwrap_or`, `Some`

*Since v0.1.0*

---

#### `Ok`

```ntnt
Ok(value: Any) -> Result<Any, Any>
```

Wraps a value in Result::Ok.

Creates a Result representing a successful outcome. Use to return success values from operations that can fail.

**Parameters:**

- `value` — The success value to wrap

**Returns:** A Result containing the success value

**Examples:**

```ntnt
Ok(42)  // => Ok(42)  // Wrap success value
Ok("data")  // => Ok("data")  // Wrap success string
```

**See also:** `Err`, `is_ok`, `is_err`, `unwrap`, `unwrap_or`, `Some`

*Since v0.1.0*

---

#### `Some`

```ntnt
Some(value: Any) -> Option<Any>
```

Wraps a value in Option::Some.

Creates an Option that contains a value. Use to represent the presence of a value in optional contexts.

**Parameters:**

- `value` — The value to wrap

**Returns:** An Option containing the value

**Examples:**

```ntnt
Some(42)  // => Some(42)  // Wrap integer in Option
Some("hello")  // => Some("hello")  // Wrap string in Option
```

**See also:** `is_some`, `is_none`, `unwrap`, `unwrap_or`, `Ok`, `Err`

*Since v0.1.0*

---

#### `abs`

```ntnt
abs(x: Int | Float) -> Int | Float
```

Returns the absolute value of a number.

Preserves the input type: Int in, Int out; Float in, Float out.

**Parameters:**

- `x` — The number to take the absolute value of

**Examples:**

```ntnt
abs(-5)  // => 5  // Absolute value of negative integer
abs(-3.14)  // => 3.14  // Absolute value of negative float
```

**Errors:**

- **TypeError**: abs() requires a number — *Fix: Pass an Int or Float*

**See also:** `sign`, `min`, `max`, `clamp`

*Since v0.1.0*

---

#### `assert`

```ntnt
assert(condition: Bool) -> Unit
```

Asserts a condition is truthy, throws ContractViolation if not.

Used for runtime invariant checks. Any falsy value (false, 0, "", None, Unit) triggers the assertion failure.

**Parameters:**

- `condition` — The condition to check

**Examples:**

```ntnt
assert(1 + 1 == 2)  // => Unit  // Passing assertion
```

**Errors:**

- **ContractViolation**: Assertion failed — *Fix: Ensure the condition evaluates to a truthy value*

*Since v0.1.0*

---

#### `ceil`

```ntnt
ceil(x: Int | Float) -> Int
```

Rounds up to the nearest integer.

Always rounds toward positive infinity. Int values pass through unchanged.

**Parameters:**

- `x` — The number to round up

**Returns:** The ceiling value as Int

**Examples:**

```ntnt
ceil(3.1)  // => 4  // Ceil of positive float
ceil(-2.9)  // => -2  // Ceil rounds toward positive infinity
```

**Errors:**

- **TypeError**: ceil() requires a number — *Fix: Pass an Int or Float*

**See also:** `floor`, `round`, `trunc`

*Since v0.1.0*

---

#### `clamp`

```ntnt
clamp(x: Int | Float, min_val: Int | Float, max_val: Int | Float) -> Int | Float
```

Constrains a value between a minimum and maximum.

Returns min_val if x < min_val, max_val if x > max_val, otherwise returns x. All three arguments must be the same numeric type.

**Parameters:**

- `x` — The value to clamp
- `min_val` — The minimum bound
- `max_val` — The maximum bound

**Returns:** The clamped value

**Examples:**

```ntnt
clamp(15, 0, 10)  // => 10  // Clamped to maximum
clamp(-5, 0, 10)  // => 0  // Clamped to minimum
clamp(5, 0, 10)  // => 5  // Value within range
```

**Errors:**

- **TypeError**: clamp() requires numbers of same type — *Fix: Pass three numbers of the same type (all Int or all Float)*

**See also:** `min`, `max`, `abs`

*Since v0.1.0*

---

#### `delete`

```ntnt
delete(pattern: String, handler: Function) -> Unit
```

Registers a DELETE route handler.

The pattern can include path parameters using {param} syntax. The handler function receives a Request and must return a Response.

**Parameters:**

- `pattern` — The URL pattern to match (e.g. "/users/{id}")
- `handler` — A function(req: Request) -> Response

**Examples:**

```ntnt
delete("/users/{id}", fn(req) { return json(map { "deleted": true }) })  // => Unit  // Register user deletion route
```

**See also:** `get`, `post`, `put`, `patch`, `listen`

*Since v0.1.0*

---

#### `enable_cors`

```ntnt
enable_cors(options?: Map) -> Unit
```

Enable CORS (Cross-Origin Resource Sharing) for the HTTP server.

Configures the server to automatically handle CORS preflight (OPTIONS) requests and add appropriate CORS headers to all responses. Must be called before `listen()`.

Options map: - `origins`: String or Array<String> of allowed origins (default: ["*"]) - `methods`: Array<String> of allowed HTTP methods (default: standard methods) - `headers`: Array<String> of allowed request headers - `credentials`: Bool to allow credentials (default: false) - `max_age`: Int preflight cache duration in seconds (default: 86400)

**Parameters:**

- `options` — Optional configuration map

**Returns:** Unit

**Examples:**

```ntnt
enable_cors()  // Enable CORS with defaults (allow all origins)
enable_cors(map { "origins": ["https://example.com"], "credentials": true })  // Restrict to specific origin
```

**See also:** `listen`, `get`, `post`

*Since v0.3.11*

---

#### `enable_csp`

```ntnt
enable_csp(options?: Map | Bool) -> Unit
```

Enable Content-Security-Policy headers for the HTTP server.

Configures the server to include CSP headers on all responses. Call with no arguments for sensible defaults, a map of directives to customize, or `false` to disable CSP entirely. Must be called before `listen()`.

Default directives: `default-src 'self'`, `script-src 'self'`, `style-src 'self' 'unsafe-inline'`, `img-src 'self' data: https:`, `font-src 'self'`, `connect-src 'self'`, `frame-ancestors 'none'`, `base-uri 'self'`, `form-action 'self'`.

Options map keys are CSP directive names with string values. Use `report_only: true` to use the Report-Only header instead.

**Parameters:**

- `options` — Optional CSP configuration map or `false` to disable

**Returns:** Unit

**Examples:**

```ntnt
enable_csp()  // Enable CSP with sensible defaults
enable_csp(map { "script-src": "'self' 'unsafe-inline'", "style-src": "'self' 'unsafe-inline' https://fonts.googleapis.com" })  // Custom CSP directives
enable_csp(false)  // Disable CSP entirely
```

**See also:** `enable_cors`, `listen`

*Since v0.4.4*

---

#### `float`

```ntnt
float(x: Int | Float | String) -> Float
```

Converts a value to float.

Accepts Int (widens), Float (identity), and String (parses decimal).

**Parameters:**

- `x` — The value to convert

**Returns:** The float value

**Examples:**

```ntnt
float(42)  // => 42.0  // Integer widened to float
float("3.14")  // => 3.14  // String parsed to float
```

**Errors:**

- **TypeError**: Cannot parse as float — *Fix: Ensure the string contains a valid number*
- **TypeError**: Cannot convert to float — *Fix: Pass an Int, Float, or String*

**See also:** `int`, `str`

*Since v0.1.0*

---

#### `floor`

```ntnt
floor(x: Int | Float) -> Int
```

Rounds down to the nearest integer.

Always rounds toward negative infinity. Int values pass through unchanged.

**Parameters:**

- `x` — The number to round down

**Returns:** The floor value as Int

**Examples:**

```ntnt
floor(3.7)  // => 3  // Floor of positive float
floor(-2.1)  // => -3  // Floor rounds toward negative infinity
```

**Errors:**

- **TypeError**: floor() requires a number — *Fix: Pass an Int or Float*

**See also:** `ceil`, `round`, `trunc`

*Since v0.1.0*

---

#### `get`

```ntnt
get(pattern: String, handler: Function) -> Unit
```

Registers a GET route handler.

The pattern can include path parameters using {param} syntax. The handler function receives a Request and must return a Response.

**Parameters:**

- `pattern` — The URL pattern to match (e.g. "/users/{id}")
- `handler` — A function(req: Request) -> Response

**Examples:**

```ntnt
get("/health", fn(req) { return json(map { "ok": true }) })  // => Unit  // Register health check route
```

**See also:** `post`, `put`, `delete`, `patch`, `listen`

*Since v0.1.0*

---

#### `int`

```ntnt
int(x: Int | Float | String | Bool) -> Int
```

Converts a value to integer.

Accepts Int (identity), Float (truncates toward zero), String (parses decimal), and Bool (true=1, false=0).

**Parameters:**

- `x` — The value to convert

**Returns:** The integer value

**Examples:**

```ntnt
int(3.7)  // => 3  // Float truncated to int
int("42")  // => 42  // String parsed to int
```

**Errors:**

- **TypeError**: Cannot parse as int — *Fix: Ensure the string contains a valid integer*
- **TypeError**: Cannot convert to int — *Fix: Pass an Int, Float, String, or Bool*

**See also:** `float`, `str`

*Since v0.1.0*

---

#### `is_array`

```ntnt
is_array(val: Any) -> Bool
```

Returns true if the value is an Array.

Use to distinguish arrays from other value types. Pairs with is_map(), is_string(), is_int(), is_float(), is_bool().

**Parameters:**

- `val` — Any value to test.

**Returns:** Bool — true if val is an Array, false otherwise.

**Examples:**

```ntnt
is_array([1, 2, 3])  // => true  // Array is an array
is_array(map { "a": 1 })  // => false  // Map is not an array
is_array("hello")  // => false  // String is not an array
```

**See also:** `is_map`, `is_string`, `is_int`, `typeof`

*Since v0.3.16*

---

#### `is_bool`

```ntnt
is_bool(val: Any) -> Bool
```

Returns true if the value is a Bool.

**Parameters:**

- `val` — Any value to test.

**Returns:** Bool — true if val is a Bool, false otherwise.

**Examples:**

```ntnt
is_bool(true)  // => true  // Bool is a bool
is_bool(1)  // => false  // Int is not a bool
```

**See also:** `is_int`, `is_string`, `typeof`

*Since v0.3.16*

---

#### `is_err`

```ntnt
is_err(res: Result<Any, Any>) -> Bool
```

Checks if a Result is Err.

Returns true if the Result contains an error, false if it is Ok.

**Parameters:**

- `res` — The Result to check

**Returns:** true if Err, false if Ok

**Examples:**

```ntnt
is_err(Err("fail"))  // => true  // Err is err
is_err(Ok(42))  // => false  // Ok is not err
```

**Errors:**

- **TypeError**: is_err() requires a Result — *Fix: Pass a Result value*

**See also:** `is_ok`, `Ok`, `Err`, `unwrap`, `unwrap_or`

*Since v0.1.0*

---

#### `is_float`

```ntnt
is_float(val: Any) -> Bool
```

Returns true if the value is a Float.

**Parameters:**

- `val` — Any value to test.

**Returns:** Bool — true if val is a Float, false otherwise.

**Examples:**

```ntnt
is_float(3.14)  // => true  // Float is a float
is_float(42)  // => false  // Int is not a float
```

**See also:** `is_int`, `is_string`, `typeof`

*Since v0.3.16*

---

#### `is_int`

```ntnt
is_int(val: Any) -> Bool
```

Returns true if the value is an integer.

**Parameters:**

- `val` — Any value to test.

**Returns:** Bool — true if val is an Int, false otherwise.

**Examples:**

```ntnt
is_int(42)  // => true  // Int is an int
is_int(3.14)  // => false  // Float is not an int
```

**See also:** `is_map`, `is_array`, `is_string`, `is_float`, `typeof`

*Since v0.3.16*

---

#### `is_map`

```ntnt
is_map(val: Any) -> Bool
```

Returns true if the value is a Map (dictionary/object).

Use to distinguish maps from other value types, especially when a function accepts either a Map or a primitive. Pairs with is_array(), is_string(), is_int(), is_float(), is_bool().

**Parameters:**

- `val` — Any value to test.

**Returns:** Bool — true if val is a Map, false otherwise.

**Examples:**

```ntnt
is_map(map { "a": 1 })  // => true  // Map is a map
is_map([1, 2, 3])  // => false  // Array is not a map
is_map("hello")  // => false  // String is not a map
is_map(None)  // => false  // None is not a map
```

**See also:** `is_array`, `is_string`, `is_int`, `typeof`

*Since v0.3.16*

---

#### `is_none`

```ntnt
is_none(opt: Option<Any>) -> Bool
```

Checks if an Option is None.

Returns true if the Option is None, false if it contains a value.

**Parameters:**

- `opt` — The Option to check

**Returns:** true if None, false if Some

**Examples:**

```ntnt
is_none(None)  // => true  // None is none
is_none(Some(42))  // => false  // Some is not none
```

**Errors:**

- **TypeError**: is_none() requires an Option — *Fix: Pass an Option value*

**See also:** `is_some`, `Some`, `unwrap`, `unwrap_or`

*Since v0.1.0*

---

#### `is_ok`

```ntnt
is_ok(res: Result<Any, Any>) -> Bool
```

Checks if a Result is Ok.

Returns true if the Result contains a success value, false if it is Err.

**Parameters:**

- `res` — The Result to check

**Returns:** true if Ok, false if Err

**Examples:**

```ntnt
is_ok(Ok(42))  // => true  // Ok is ok
is_ok(Err("fail"))  // => false  // Err is not ok
```

**Errors:**

- **TypeError**: is_ok() requires a Result — *Fix: Pass a Result value*

**See also:** `is_err`, `Ok`, `Err`, `unwrap`, `unwrap_or`

*Since v0.1.0*

---

#### `is_some`

```ntnt
is_some(opt: Option<Any>) -> Bool
```

Checks if an Option is Some.

Returns true if the Option contains a value, false if it is None.

**Parameters:**

- `opt` — The Option to check

**Returns:** true if Some, false if None

**Examples:**

```ntnt
is_some(Some(42))  // => true  // Some is some
is_some(None)  // => false  // None is not some
```

**Errors:**

- **TypeError**: is_some() requires an Option — *Fix: Pass an Option value*

**See also:** `is_none`, `Some`, `unwrap`, `unwrap_or`

*Since v0.1.0*

---

#### `is_string`

```ntnt
is_string(val: Any) -> Bool
```

Returns true if the value is a String.

**Parameters:**

- `val` — Any value to test.

**Returns:** Bool — true if val is a String, false otherwise.

**Examples:**

```ntnt
is_string("hello")  // => true  // String is a string
is_string(42)  // => false  // Int is not a string
```

**See also:** `is_map`, `is_array`, `is_int`, `typeof`

*Since v0.3.16*

---

#### `len`

```ntnt
len(x: String | Array | Map) -> Int
```

Returns the length of a string, array, or map.

For strings, returns the number of bytes. For arrays, returns the number of elements. For maps, returns the number of key-value pairs.

**Parameters:**

- `x` — The value to measure

**Returns:** The length as an integer

**Examples:**

```ntnt
len("hello")  // => 5  // String length
len([1, 2, 3])  // => 3  // Array length
len(map { "a": 1, "b": 2 })  // => 2  // Map length
```

**Errors:**

- **TypeError**: len() requires a string, array, or map — *Fix: Pass a String, Array, or Map*

**See also:** `type`, `is_empty`

*Since v0.1.0*

---

#### `listen`

```ntnt
listen(port: Int) -> Unit
```

Starts an HTTP server on the given port.

This must be called after registering route handlers with get(), post(), put(), delete(), or patch(). The server blocks and serves requests until the process is terminated.

**Parameters:**

- `port` — The port number to listen on (e.g. 8080)

**Examples:**

```ntnt
listen(8080)  // => Unit  // Start server on port 8080
```

**See also:** `get`, `post`, `put`, `delete`, `patch`, `new_server`

*Since v0.1.0*

---

#### `max`

```ntnt
max(a: Int | Float, b: Int | Float) -> Int | Float
```

Returns the larger of two numbers.

When both arguments are Int, returns Int. If either is Float, returns Float.

**Parameters:**

- `a` — First number
- `b` — Second number

**Examples:**

```ntnt
max(3, 7)  // => 7  // Maximum of two integers
max(2.5, 1.0)  // => 2.5  // Maximum of two floats
```

**Errors:**

- **TypeError**: max() requires numbers — *Fix: Pass two numbers*

**See also:** `min`, `clamp`, `abs`

*Since v0.1.0*

---

#### `min`

```ntnt
min(a: Int | Float, b: Int | Float) -> Int | Float
```

Returns the smaller of two numbers.

When both arguments are Int, returns Int. If either is Float, returns Float.

**Parameters:**

- `a` — First number
- `b` — Second number

**Examples:**

```ntnt
min(3, 7)  // => 3  // Minimum of two integers
min(2.5, 1.0)  // => 1.0  // Minimum of two floats
```

**Errors:**

- **TypeError**: min() requires numbers — *Fix: Pass two numbers*

**See also:** `max`, `clamp`, `abs`

*Since v0.1.0*

---

#### `new_server`

```ntnt
new_server() -> Unit
```

Resets the server, clearing all registered routes.

Call this before re-registering routes if you need to rebuild the server configuration. Useful in hot-reload scenarios.

**Examples:**

```ntnt
new_server()  // => Unit  // Clear all routes and start fresh
```

**See also:** `listen`, `get`, `post`, `put`, `delete`, `patch`

*Since v0.2.0*

---

#### `patch`

```ntnt
patch(pattern: String, handler: Function) -> Unit
```

Registers a PATCH route handler.

The pattern can include path parameters using {param} syntax. The handler function receives a Request and must return a Response.

**Parameters:**

- `pattern` — The URL pattern to match (e.g. "/users/{id}")
- `handler` — A function(req: Request) -> Response

**Examples:**

```ntnt
patch("/users/{id}", fn(req) { return json(map { "patched": true }) })  // => Unit  // Register partial update route
```

**See also:** `get`, `post`, `put`, `delete`, `listen`

*Since v0.1.0*

---

#### `post`

```ntnt
post(pattern: String, handler: Function) -> Unit
```

Registers a POST route handler.

The pattern can include path parameters using {param} syntax. The handler function receives a Request and must return a Response.

**Parameters:**

- `pattern` — The URL pattern to match (e.g. "/users")
- `handler` — A function(req: Request) -> Response

**Examples:**

```ntnt
post("/users", fn(req) { return json(map { "created": true }) })  // => Unit  // Register user creation route
```

**See also:** `get`, `put`, `delete`, `patch`, `listen`

*Since v0.1.0*

---

#### `pow`

```ntnt
pow(base: Int | Float, exp: Int | Float) -> Int | Float
```

Raises base to the power of exponent.

Returns Int when both arguments are Int and the exponent is non-negative. Returns Float otherwise.

**Parameters:**

- `base` — The base number
- `exp` — The exponent

**Examples:**

```ntnt
pow(2, 10)  // => 1024  // Integer power
pow(2.0, 0.5)  // => 1.4142135623730951  // Float power (square root)
```

**Errors:**

- **TypeError**: pow() requires numbers — *Fix: Pass two numbers*

**See also:** `sqrt`, `abs`

*Since v0.1.0*

---

#### `print`

```ntnt
print(value: Any) -> Unit
```

Prints values to stdout, one per line.

Accepts any value type. Non-string values are automatically converted to their string representation.

**Parameters:**

- `value` — The value to print

**Examples:**

```ntnt
print("hello")  // => Unit  // Prints hello to stdout
print(42)  // => Unit  // Prints 42 to stdout
```

**See also:** `str`, `type`

*Since v0.1.0*

---

#### `push`

```ntnt
push(arr: Array, item: Any) -> Array
```

Appends an item to an array, returns a new array.

Does not mutate the original array. Returns a new array with the item appended at the end.

**Parameters:**

- `arr` — The array to append to
- `item` — The value to append

**Returns:** A new array with the item appended

**Examples:**

```ntnt
push([1, 2], 3)  // => [1, 2, 3]  // Append to array
```

**Errors:**

- **TypeError**: push() requires an array — *Fix: Pass an array as the first argument*

**See also:** `pop`, `concat`

*Since v0.1.0*

---

#### `put`

```ntnt
put(pattern: String, handler: Function) -> Unit
```

Registers a PUT route handler.

The pattern can include path parameters using {param} syntax. The handler function receives a Request and must return a Response.

**Parameters:**

- `pattern` — The URL pattern to match (e.g. "/users/{id}")
- `handler` — A function(req: Request) -> Response

**Examples:**

```ntnt
put("/users/{id}", fn(req) { return json(map { "updated": true }) })  // => Unit  // Register user update route
```

**See also:** `get`, `post`, `delete`, `patch`, `listen`

*Since v0.1.0*

---

#### `round`

```ntnt
round(x: Int | Float, decimals?: Int) -> Int | Float
```

Rounds to the nearest integer, or to N decimal places.

With one argument, rounds to the nearest integer and returns Int. With two arguments, rounds to the specified number of decimal places and returns Float.

**Parameters:**

- `x` — The number to round
- `decimals` — Optional number of decimal places (must be non-negative)

**Returns:** Int when called with 1 arg, Float when called with 2 args

**Examples:**

```ntnt
round(3.7)  // => 4  // Round to nearest integer
round(3.14159, 2)  // => 3.14  // Round to 2 decimal places
```

**Errors:**

- **TypeError**: round() requires 1 or 2 arguments — *Fix: Pass 1 or 2 arguments*
- **TypeError**: round() decimal places must be non-negative — *Fix: Use a non-negative integer for decimals*

**See also:** `floor`, `ceil`, `trunc`

*Since v0.1.0*

---

#### `sign`

```ntnt
sign(x: Int | Float) -> Int
```

Returns the sign of a number: -1, 0, or 1.

Returns -1 for negative, 0 for zero, 1 for positive. Always returns Int regardless of input type.

**Parameters:**

- `x` — The number to check

**Returns:** -1, 0, or 1

**Examples:**

```ntnt
sign(-42)  // => -1  // Negative number
sign(0)  // => 0  // Zero
sign(7)  // => 1  // Positive number
```

**Errors:**

- **TypeError**: sign() requires a number — *Fix: Pass an Int or Float*

**See also:** `abs`, `clamp`

*Since v0.1.0*

---

#### `sqrt`

```ntnt
sqrt(x: Int | Float) -> Float
```

Returns the square root of a number.

Always returns Float. Negative numbers produce a RuntimeError.

**Parameters:**

- `x` — The number to take the square root of (must be non-negative)

**Returns:** The square root as Float

**Examples:**

```ntnt
sqrt(9)  // => 3.0  // Square root of integer
sqrt(2.0)  // => 1.4142135623730951  // Square root of float
```

**Errors:**

- **RuntimeError**: sqrt() of negative number — *Fix: Ensure the argument is non-negative*
- **TypeError**: sqrt() requires a number — *Fix: Pass an Int or Float*

**See also:** `pow`, `abs`

*Since v0.1.0*

---

#### `str`

```ntnt
str(x: Any) -> String
```

Converts any value to its string representation.

Produces a human-readable string for any value type. Arrays and maps are formatted with brackets/braces.

**Parameters:**

- `x` — The value to convert

**Returns:** The string representation

**Examples:**

```ntnt
str(42)  // => "42"  // Integer to string
str(true)  // => "true"  // Boolean to string
```

**See also:** `int`, `float`, `type`

*Since v0.1.0*

---

#### `trunc`

```ntnt
trunc(x: Int | Float) -> Int
```

Truncates a number toward zero.

Removes the fractional part, rounding toward zero. Unlike floor(), negative values round toward zero (up). Int values pass through unchanged.

**Parameters:**

- `x` — The number to truncate

**Returns:** The truncated value as Int

**Examples:**

```ntnt
trunc(3.9)  // => 3  // Truncate positive float
trunc(-2.9)  // => -2  // Truncate toward zero (not negative infinity)
```

**Errors:**

- **TypeError**: trunc() requires a number — *Fix: Pass an Int or Float*

**See also:** `floor`, `ceil`, `round`

*Since v0.1.0*

---

#### `type`

```ntnt
type(x: Any) -> String
```

Returns the type name of a value as a string.

Returns one of: "Int", "Float", "String", "Bool", "Array", "Map", "Function", "Unit", or the enum/struct name.

**Parameters:**

- `x` — The value to inspect

**Returns:** The type name as a string

**Examples:**

```ntnt
type(42)  // => "Int"  // Integer type
type("hello")  // => "String"  // String type
```

**See also:** `str`, `len`

*Since v0.1.0*

---

#### `typeof`

```ntnt
typeof(x: Any) -> String
```

Returns the type name of a value as a string.

Alias for `type()` that works in all contexts, including where `type` is parsed as a keyword (type alias declarations). Use `typeof()` for runtime type checking in conditional logic. Returns one of: "Int", "Float", "String", "Bool", "Array", "Map", "Function", "Unit", or the enum/struct name.

**Parameters:**

- `x` — The value to inspect

**Returns:** The type name as a string

**Examples:**

```ntnt
typeof(42)  // => "Int"  // Integer type
typeof("hello")  // => "String"  // String type
typeof(map { "a": 1 })  // => "Map"  // Map type
typeof([1, 2])  // => "Array"  // Array type
```

**See also:** `type`, `str`, `len`

*Since v0.4.0*

---

#### `unwrap`

```ntnt
unwrap(x: Option<Any> | Result<Any, Any>) -> Any
```

Extracts the value from Some or Ok, panics on None or Err.

Use when you are certain the value is present. For safer alternatives, use unwrap_or() or pattern matching with match.

**Parameters:**

- `x` — The Option or Result to unwrap

**Returns:** The contained value

**Examples:**

```ntnt
unwrap(Some(42))  // => 42  // Unwrap Some
unwrap(Ok("data"))  // => "data"  // Unwrap Ok
```

**Errors:**

- **RuntimeError**: Called unwrap() on None — *Fix: Check with is_some() first or use unwrap_or()*
- **RuntimeError**: Called unwrap() on Err(*) — *Fix: Check with is_ok() first or use unwrap_or()*

**Gotchas:**

- Panics at runtime on None or Err. Prefer unwrap_or() or match for safe handling.

**See also:** `unwrap_or`, `is_some`, `is_ok`, `Some`, `Ok`

*Since v0.1.0*

---

#### `unwrap_or`

```ntnt
unwrap_or(x: Option<Any> | Result<Any, Any>, default: Any) -> Any
```

Extracts the value from Some or Ok, returns default on None or Err.

A safe alternative to unwrap() that never panics. Returns the contained value for Some/Ok, or the provided default for None/Err.

**Parameters:**

- `x` — The Option or Result to unwrap
- `default` — The fallback value to use if None or Err

**Returns:** The contained value or the default

**Examples:**

```ntnt
unwrap_or(Some(42), 0)  // => 42  // Unwrap Some with default
unwrap_or(None, 0)  // => 0  // Default returned for None
unwrap_or(Err("fail"), "fallback")  // => "fallback"  // Default returned for Err
```

**See also:** `unwrap`, `is_some`, `is_ok`, `Some`, `Ok`

*Since v0.1.0*

---

## std/auth

Full OAuth 2.0 and OIDC authentication with JWT support

```ntnt
import { oauth, oauth_discover, oauth_m2m } from "std/auth"
```

### Functions

| Function | Description |
|----------|-------------|
| [`auth_callback`](#authcallback) | Handle OAuth callback - exchanges code for tokens, creates session. |
| [`auth_logout`](#authlogout) | Handle logout - clears the session and redirects. |
| [`auth_me`](#authme) | Return current user as JSON for SPAs. |
| [`auth_start`](#authstart) | Handle OAuth login start - redirects to the provider's authorization page. |
| [`create_session_from_oauth`](#createsessionfromoauth) | Create a session from OAuth user info and tokens. |
| [`csrf_field`](#csrffield) | Get an HTML hidden input field with the CSRF token. |
| [`csrf_token`](#csrftoken) | Get the CSRF token for the current session. |
| [`enable_auth`](#enableauth) | Initialize the authentication system with OAuth providers. |
| [`get_session`](#getsession) | Get the current session from the request. |
| [`get_user`](#getuser) | Get the current authenticated user from the request. |
| [`hash_password`](#hashpassword) | Hash a password using bcrypt. |
| [`jwt_decode`](#jwtdecode) | Decode a JWT token WITHOUT verifying the signature. |
| [`jwt_sign`](#jwtsign) | Create a signed JWT token from claims. |
| [`jwt_verify`](#jwtverify) | Verify a JWT token and return its claims. |
| [`logout_all`](#logoutall) | Log out all sessions for the current user. |
| [`logout_user`](#logoutuser) | Log out the current user and return a redirect response. |
| [`oauth`](#oauth) | Create an OAuth provider configuration. |
| [`oauth_discover`](#oauthdiscover) | Create an OAuth provider using OIDC Discovery. |
| [`oauth_exchange`](#oauthexchange) | Exchange OAuth authorization code for tokens and user info. |
| [`oauth_introspect`](#oauthintrospect) | Introspect a token using the provider's introspection endpoint (RFC 7662). |
| [`oauth_m2m`](#oauthm2m) | Get an access token using client credentials grant (M2M authentication). |
| [`oauth_refresh`](#oauthrefresh) | Refresh the access token for the current session. |
| [`oauth_start`](#oauthstart) | Generate an OAuth authorization URL for manual flow control. |
| [`oauth_validate`](#oauthvalidate) | Validate an incoming bearer token (for APIs acting as resource servers). |
| [`session_data`](#sessiondata) | Get custom data stored in the current session. |
| [`sessions_cleanup`](#sessionscleanup) | Clean up expired sessions and OAuth states from the session store. |
| [`set_session`](#setsession) | Store custom data in the current session. |
| [`totp_secret`](#totpsecret) | Generate a new TOTP secret for MFA setup. |
| [`totp_uri`](#totpuri) | Generate an otpauth:// URI for QR codes. |
| [`user_sessions`](#usersessions) | Get all active sessions for the current user. |
| [`validate_csrf`](#validatecsrf) | Validate CSRF token on state-changing requests (POST, PUT, DELETE, PATCH). |
| [`verify_csrf`](#verifycsrf) | Verify a CSRF token against the session's token. |
| [`verify_password`](#verifypassword) | Verify a password against a bcrypt hash. |
| [`verify_totp`](#verifytotp) | Verify a TOTP code against a secret. |

#### `auth_callback`

```ntnt
auth_callback(req: Request) -> Response
```

Handle OAuth callback - exchanges code for tokens, creates session.

Use with a route like GET /auth/{provider}/callback. Reads state and code from query params, validates CSRF, exchanges code for tokens, and creates a user session.

**Parameters:**

- `req` — The HTTP request with query params state and code

**Returns:** Redirect response to after_login URL with session cookie

**Examples:**

```ntnt
get("/auth/{provider}/callback", auth_callback)  // Wire up callback route
```

**See also:** `enable_auth`, `auth_start`

*Since v0.3.11*

---

#### `auth_logout`

```ntnt
auth_logout(req: Request) -> Response
```

Handle logout - clears the session and redirects.

Use with a route like POST /auth/logout. Clears the session cookie and redirects to after_logout URL.

**Parameters:**

- `req` — The HTTP request

**Returns:** Redirect response to after_logout URL

**Examples:**

```ntnt
post("/auth/logout", auth_logout)  // Wire up logout route
```

**See also:** `enable_auth`, `get_user`

*Since v0.3.11*

---

#### `auth_me`

```ntnt
auth_me(req: Request) -> Response
```

Return current user as JSON for SPAs.

Use with a route like GET /auth/me. Returns the current user's session data as JSON, or 401 if not authenticated.

**Parameters:**

- `req` — The HTTP request

**Returns:** JSON response with user data or 401

**Examples:**

```ntnt
get("/auth/me", auth_me)  // Wire up user endpoint
```

**See also:** `get_user`, `enable_auth`

*Since v0.3.11*

---

#### `auth_start`

```ntnt
auth_start(req: Request) -> Response
```

Handle OAuth login start - redirects to the provider's authorization page.

Use with a route like GET /auth/{provider}. Reads the provider name from req.params.provider and generates the OAuth authorization URL with PKCE/nonce.

**Parameters:**

- `req` — The HTTP request with route param {provider}

**Returns:** Redirect response to OAuth provider

**Examples:**

```ntnt
get("/auth/{provider}", auth_start)  // Wire up login routes
```

**See also:** `enable_auth`, `auth_callback`

*Since v0.3.11*

---

#### `create_session_from_oauth`

```ntnt
create_session_from_oauth(provider_name: String, user_info: Map, tokens?: Map) -> Result<Map, String>
```

Create a session from OAuth user info and tokens.

Use this after oauth_exchange to create a session. Returns the session info and Set-Cookie header value.

**Parameters:**

- `provider_name` — Name of the provider (for user_id prefix)
- `user_info` — User info map from oauth_exchange
- `tokens` — Optional tokens map from oauth_exchange

**Returns:** Ok(map with session_id, user_id, cookie) or Err on failure

**Examples:**

```ntnt
create_session_from_oauth("github", user_info, tokens)  // => Ok({session_id: "...", cookie: "..."})  // Create session
```

**See also:** `oauth_exchange`, `get_session`

*Since v0.3.11*

---

#### `csrf_field`

```ntnt
csrf_field(req: Request) -> String
```

Get an HTML hidden input field with the CSRF token.

Returns a ready-to-use hidden input element for forms. Use this to easily include CSRF protection in your forms without manual formatting.

**Parameters:**

- `req` — The HTTP request object

**Returns:** HTML string like `<input type="hidden" name="_csrf" value="..."/>`

**Examples:**

```ntnt
csrf_field(req)  // Get hidden input for form
```

**See also:** `csrf_token`, `verify_csrf`

*Since v0.3.11*

---

#### `csrf_token`

```ntnt
csrf_token(req: Request) -> Option<String>
```

Get the CSRF token for the current session.

Use this token in forms to protect against Cross-Site Request Forgery. Include the token as a hidden field named "_csrf" and verify it with verify_csrf().

**Parameters:**

- `req` — The HTTP request object

**Returns:** Option containing the CSRF token string, or None if not authenticated

**Examples:**

```ntnt
csrf_token(req)  // Get token for form
```

**See also:** `verify_csrf`, `csrf_field`

*Since v0.3.11*

---

#### `enable_auth`

```ntnt
enable_auth(providers: [Provider], options?: Map) -> Unit
```

Initialize the authentication system with OAuth providers.

Stores provider configurations for use by auth handlers. After calling this, you can use auth_start, auth_callback, and auth_logout with routes to enable OAuth login.

Session storage options: "memory" (default), "sqlite:./path.db", "postgres://url", or "redis://url".

**Parameters:**

- `providers` — Array of provider configs created by oauth() or oauth_discover()
- `options` — Optional map with keys: session_secret, session_ttl, after_login, after_logout, session_store

**Returns:** Unit

**Examples:**

```ntnt
// Initialize auth with GitHub
let github = oauth("github", get_env("GITHUB_ID"), get_env("GITHUB_SECRET"))
enable_auth([github], map { "session_secret": "my-secret" })
enable_auth([github], map { "session_store": "sqlite:./sessions.db" })  // SQLite sessions
enable_auth([github], map { "session_store": "redis://localhost:6379" })  // Redis sessions
```

**See also:** `oauth`, `oauth_discover`, `auth_start`

*Since v0.3.11*

---

#### `get_session`

```ntnt
get_session(req: Request) -> Option<Session>
```

Get the current session from the request.

Returns the full session object including user, timestamps, tokens, and custom data.

**Parameters:**

- `req` — The HTTP request object

**Returns:** Option containing the Session map or None

**Examples:**

```ntnt
get_session(req)  // Get full session data
```

**See also:** `get_user`, `logout_user`, `oauth_refresh`

*Since v0.3.11*

---

#### `get_user`

```ntnt
get_user(req: Request) -> Option<User>
```

Get the current authenticated user from the request.

Returns Some(user) if authenticated, None if not. Use with `otherwise` for concise auth checks in handlers.

**Parameters:**

- `req` — The HTTP request object

**Returns:** Option containing the User map or None

**Examples:**

```ntnt
get_user(req) otherwise return redirect("/login")  // Require auth
```

**See also:** `get_session`, `logout_user`

*Since v0.3.11*

---

#### `hash_password`

```ntnt
hash_password(password: String) -> Result<String, String>
```

Hash a password using bcrypt.

Utility function to hash passwords for custom storage or verification. Uses bcrypt with default cost factor.

**Parameters:**

- `password` — The password to hash

**Returns:** Ok(hash) on success, Err(message) on failure

**Examples:**

```ntnt
hash_password("mypassword")  // => Ok("$2b$12$...")  // Hash a password
```

**See also:** `verify_password`

*Since v0.3.11*

---

#### `jwt_decode`

```ntnt
jwt_decode(token: String) -> Result<Map, String>
```

Decode a JWT token WITHOUT verifying the signature.

Use this only for debugging or when you need to inspect token contents before verification. Never trust the claims from this function for auth.

**Parameters:**

- `token` — The JWT token string

**Returns:** Result containing map with "header" and "payload" keys, or error

**Examples:**

```ntnt
jwt_decode(token)  // Inspect token without verification
```

**See also:** `jwt_sign`, `jwt_verify`

*Since v0.3.11*

---

#### `jwt_sign`

```ntnt
jwt_sign(claims: Map, secret: String, options?: Map) -> Result<String, String>
```

Create a signed JWT token from claims.

Signs the claims using HS256 algorithm and returns the JWT string. Optional options map can include: exp (expiration as unix timestamp), iat (issued-at, defaults to now), sub (subject), iss (issuer), aud (audience).

**Parameters:**

- `claims` — The payload claims as a map
- `secret` — The signing secret (should be at least 32 bytes)
- `options` — Optional map with exp, iat, sub, iss, aud

**Returns:** Result containing the JWT string, or error message

**Examples:**

```ntnt
jwt_sign(map { "user_id": 123 }, secret)  // Create a token
jwt_sign(map { "user_id": 123 }, secret, map { "exp": now() + 3600 })  // Token with 1hr expiry
```

**See also:** `jwt_verify`, `jwt_decode`

*Since v0.3.11*

---

#### `jwt_verify`

```ntnt
jwt_verify(token: String, secret: String) -> Result<Map, String>
```

Verify a JWT token and return its claims.

Validates the signature and expiration, then returns the claims as a map. Returns Err if the token is invalid, expired, or has wrong signature.

**Parameters:**

- `token` — The JWT token string
- `secret` — The signing secret used to create the token

**Returns:** Result containing the claims map, or error message

**Examples:**

```ntnt
jwt_verify(token, secret)  // Verify and get claims
```

**See also:** `jwt_sign`, `jwt_decode`

*Since v0.3.11*

---

#### `logout_all`

```ntnt
logout_all(req: Request, keep_current: Bool) -> Result<Int, String>
```

Log out all sessions for the current user.

Deletes all sessions for the user. If keep_current is true, keeps the current session active (useful for "log out everywhere else"). Returns the number of sessions that were deleted.

**Parameters:**

- `req` — The HTTP request object
- `keep_current` — If true, keep the current session active

**Returns:** Result containing number of sessions deleted, or error

**Examples:**

```ntnt
logout_all(req, true)  // Log out everywhere except here
logout_all(req, false)  // Log out from all devices
```

**See also:** `user_sessions`, `logout_user`

*Since v0.3.11*

---

#### `logout_user`

```ntnt
logout_user(req: Request) -> Response
```

Log out the current user and return a redirect response.

Clears the session and returns a redirect to the configured logout_url (default: "/") with the session cookie cleared.

**Parameters:**

- `req` — The HTTP request object

**Returns:** Redirect response with session cookie cleared

**Examples:**

```ntnt
logout_user(req)  // Log out and redirect to home
```

**See also:** `get_user`, `get_session`

*Since v0.3.11*

---

#### `oauth`

```ntnt
oauth(provider: String, client_id: String, client_secret: String, options?: Map) -> Provider
```

Create an OAuth provider configuration.

Supports built-in providers (google, github, facebook, microsoft, discord, twitter, linkedin, apple) with sensible defaults, or custom providers with full configuration. Supports OIDC (ID tokens, nonce validation) and PKCE.

**Parameters:**

- `provider` — Provider name (e.g., "google", "github") or custom name
- `client_id` — OAuth client ID (or config map for custom providers)
- `client_secret` — OAuth client secret (omit for PKCE public clients)
- `options` — Optional map: scopes, use_pkce, access_type, prompt

**Returns:** Provider configuration to pass to enable_auth()

**Examples:**

```ntnt
oauth("google", "client_id", "client_secret")  // => Provider  // Google OAuth with defaults
oauth("github", "id", "secret", map { "scopes": ["repo"] })  // => Provider  // GitHub with custom scopes
oauth("google", "id", "secret", map { "use_pkce": true })  // => Provider  // Google with PKCE
```

**See also:** `enable_auth`, `get_user`, `oauth_pkce`

*Since v0.3.11*

---

#### `oauth_discover`

```ntnt
oauth_discover(issuer: String, client_id: String, client_secret?: String, options?: Map) -> Result<Provider, String>
```

Create an OAuth provider using OIDC Discovery.

Automatically fetches configuration from the issuer's .well-known/openid-configuration endpoint. Useful for Okta, Auth0, Keycloak, and other OIDC providers.

**Parameters:**

- `issuer` — The OIDC issuer URL (e.g., "https://mycompany.okta.com")
- `client_id` — OAuth client ID
- `client_secret` — OAuth client secret (optional for PKCE)
- `options` — Optional map: scopes, use_pkce

**Returns:** Result containing Provider or error message

**Examples:**

```ntnt
oauth_discover("https://mycompany.okta.com", "client_id", "secret")  // => Ok(Provider)  // Okta with auto-discovery
```

**See also:** `oauth`, `enable_auth`

*Since v0.3.11*

---

#### `oauth_exchange`

```ntnt
oauth_exchange(provider: Map, code: String, state: String, redirect_uri: String) -> Result<Map, String>
```

Exchange OAuth authorization code for tokens and user info.

Use this after receiving the callback with code and state parameters. Returns tokens and user info - you decide what to do with them (create session, etc).

**Parameters:**

- `provider` — Provider config from oauth()
- `code` — Authorization code from callback
- `state` — State parameter from callback (for CSRF validation)
- `redirect_uri` — Same redirect_uri used in oauth_start

**Returns:** Ok(map with tokens and user_info) or Err on failure

**Examples:**

```ntnt
oauth_exchange(github, code, state, redirect_uri)  // => Ok({access_token: "...", user_info: {...}})  // Exchange code
```

**See also:** `oauth_start`, `create_session_from_oauth`

*Since v0.3.11*

---

#### `oauth_introspect`

```ntnt
oauth_introspect(introspection_url: String, token: String, client_id: String, client_secret: String) -> Result<Map, String>
```

Introspect a token using the provider's introspection endpoint (RFC 7662).

Calls the authorization server to validate the token. More reliable than local validation but adds network latency.

**Parameters:**

- `introspection_url` — The introspection endpoint URL
- `token` — The token to introspect
- `client_id` — OAuth client ID
- `client_secret` — OAuth client secret

**Returns:** Result containing introspection response or error

**Examples:**

```ntnt
oauth_introspect("https://auth.example.com/introspect", token, "id", "secret")  // Introspect token
```

**See also:** `oauth_validate`

*Since v0.3.11*

---

#### `oauth_m2m`

```ntnt
oauth_m2m(token_url: String, client_id: String, client_secret: String, scopes: [String]) -> Result<Map, String>
```

Get an access token using client credentials grant (M2M authentication).

Used for server-to-server API calls where no user is involved.

**Parameters:**

- `token_url` — The token endpoint URL
- `client_id` — OAuth client ID
- `client_secret` — OAuth client secret
- `scopes` — Array of scopes to request

**Returns:** Result containing token response map or error

**Examples:**

```ntnt
oauth_m2m("https://oauth.example.com/token", "id", "secret", ["api.read"])  // => Ok({access_token: "...", ...})  // Get M2M token
```

**See also:** `oauth`, `oauth_refresh`

*Since v0.3.11*

---

#### `oauth_refresh`

```ntnt
oauth_refresh(req: Request) -> Result<Map, String>
```

Refresh the access token for the current session.

Uses the stored refresh token to get a new access token. Updates the session with new tokens. Requires enable_auth() with store_tokens: true.

**Parameters:**

- `req` — The HTTP request object

**Returns:** Result containing new token info or error

**Examples:**

```ntnt
oauth_refresh(req)  // => Ok({access_token: "...", expires_in: 3600})  // Refresh tokens
```

**See also:** `get_session`, `oauth`

*Since v0.3.11*

---

#### `oauth_start`

```ntnt
oauth_start(provider: Map, redirect_uri: String) -> Result<String, String>
```

Generate an OAuth authorization URL for manual flow control.

Use this when you want to control the OAuth flow manually instead of using auth_start. Returns the authorization URL with state parameter for CSRF protection.

**Parameters:**

- `provider` — Provider config from oauth()
- `redirect_uri` — Your callback URL

**Returns:** Ok(auth_url) to redirect user to, Err on failure

**Examples:**

```ntnt
oauth_start(github, "https://myapp.com/callback")  // => Ok("https://github.com/...")  // Get auth URL
```

**See also:** `oauth_exchange`, `oauth`

*Since v0.3.11*

---

#### `oauth_validate`

```ntnt
oauth_validate(token: String, options: Map) -> Result<Map, String>
```

Validate an incoming bearer token (for APIs acting as resource servers).

Decodes and validates the token claims without calling the provider. For full validation, use oauth_introspect().

**Parameters:**

- `token` — The bearer token to validate
- `options` — Map with issuer, audience for validation

**Returns:** Result containing token claims or error

**Examples:**

```ntnt
oauth_validate(token, map { "issuer": "https://...", "audience": "my-api" })  // Validate bearer token
```

**See also:** `oauth_introspect`, `jwt_verify`

*Since v0.3.11*

---

#### `session_data`

```ntnt
session_data(req: Request) -> Option<Map>
```

Get custom data stored in the current session.

Returns the custom data map stored via set_session, or None if no session or no custom data. Use this to store and retrieve user roles, permissions, preferences, or other application-specific data.

**Parameters:**

- `req` — The HTTP request object

**Returns:** Option containing the custom data Map or None

**Examples:**

```ntnt
session_data(req)  // Get user roles and preferences
```

**See also:** `set_session`, `get_session`, `get_user`

*Since v0.3.11*

---

#### `sessions_cleanup`

```ntnt
sessions_cleanup() -> Result<Int, String>
```

Clean up expired sessions and OAuth states from the session store.

Call this periodically (e.g., via a cron job or scheduled task) to remove expired sessions and OAuth states from the database. For Redis, sessions use TTL so they expire automatically, but this will scan for any orphaned entries.

**Returns:** Result containing the number of expired sessions removed, or error

**Examples:**

```ntnt
sessions_cleanup()  // Remove expired sessions
```

**See also:** `enable_auth`

*Since v0.3.11*

---

#### `set_session`

```ntnt
set_session(req: Request, data: Map) -> Result<Unit, String>
```

Store custom data in the current session.

Use this to store user roles, permissions, preferences, or other application-specific data that should persist across requests. Data is stored as JSON in the session.

**Parameters:**

- `req` — The HTTP request object
- `data` — The custom data map to store

**Returns:** Result indicating success or error message

**Examples:**

```ntnt
set_session(req, map { "roles": ["admin"], "theme": "dark" })  // Store user preferences
```

**See also:** `session_data`, `get_session`

*Since v0.3.11*

---

#### `totp_secret`

```ntnt
totp_secret() -> String
```

Generate a new TOTP secret for MFA setup.

Creates a random base32-encoded secret suitable for TOTP authentication. Use this secret with totp_uri() to generate a QR code for authenticator apps.

**Returns:** Base32-encoded TOTP secret

**Examples:**

```ntnt
totp_secret()  // => "JBSWY3DPEHPK3PXP..."  // Generate secret
```

**See also:** `totp_uri`, `verify_totp`

*Since v0.3.11*

---

#### `totp_uri`

```ntnt
totp_uri(secret: String, email: String, issuer: String) -> Result<String, String>
```

Generate an otpauth:// URI for QR codes.

Creates a URI that can be encoded as a QR code for authenticator apps like Google Authenticator or Authy.

**Parameters:**

- `secret` — TOTP secret (base32 encoded)
- `email` — User's email for the account label
- `issuer` — App name shown in authenticator

**Returns:** Ok(uri) on success, Err(message) on failure

**Examples:**

```ntnt
totp_uri(secret, "user@example.com", "MyApp")  // => Ok("otpauth://...")  // Get URI for QR
```

**See also:** `totp_secret`, `verify_totp`

*Since v0.3.11*

---

#### `user_sessions`

```ntnt
user_sessions(req: Request) -> Result<Array<SessionInfo>, String>
```

Get all active sessions for the current user.

Returns an array of session info objects, each containing id, provider, created_at, expires_at, and is_current (boolean indicating if it's the current session). Useful for "manage your sessions" UI.

**Parameters:**

- `req` — The HTTP request object

**Returns:** Result containing array of session info, or error

**Examples:**

```ntnt
user_sessions(req)  // List all user's active sessions
```

**See also:** `logout_all`, `get_session`

*Since v0.3.11*

---

#### `validate_csrf`

```ntnt
validate_csrf(req: Request) -> Result<Bool, Map>
```

Validate CSRF token on state-changing requests (POST, PUT, DELETE, PATCH).

Compares the CSRF token from the request (form field `_csrf_token` or header `X-CSRF-Token`) against the token stored in the session. Returns `true` if valid. Returns an error response map (403) if invalid, which can be returned directly from a route handler.

Skips validation for: - GET, HEAD, OPTIONS requests (safe methods) - API key auth (Bearer token) — CSRF only applies to cookie-based sessions - Requests with no session (will fail auth check separately)

Usage in middleware: ```ntnt let csrf_ok = validate_csrf(req) if typeof(csrf_ok) == "Map" { return csrf_ok }  // Return 403 response ```

Usage in forms: ```html <input type="hidden" name="_csrf_token" value="{{user.csrf_token}}"> ```

**Parameters:**

- `req` — The HTTP request object

**Returns:** true if valid or safe method; a 403 error response Map if invalid

**Examples:**

```ntnt
validate_csrf(req)  // Check CSRF token on POST
```

**See also:** `get_user`, `get_session`

*Since v0.4.0*

---

#### `verify_csrf`

```ntnt
verify_csrf(req: Request, token: String) -> Bool
```

Verify a CSRF token against the session's token.

Returns true if the token matches the session's CSRF token, false otherwise. Use this in POST/PUT/DELETE handlers to validate the "_csrf" form field.

**Parameters:**

- `req` — The HTTP request object
- `token` — The CSRF token from the form submission

**Returns:** true if valid, false if invalid or not authenticated

**Examples:**

```ntnt
verify_csrf(req, form["_csrf"])  // Validate form submission
```

**See also:** `csrf_token`, `csrf_field`

*Since v0.3.11*

---

#### `verify_password`

```ntnt
verify_password(password: String, hash: String) -> Bool
```

Verify a password against a bcrypt hash.

Utility function to verify passwords hashed with hash_password.

**Parameters:**

- `password` — The password to verify
- `hash` — The bcrypt hash to verify against

**Returns:** true if password matches hash, false otherwise

**Examples:**

```ntnt
verify_password("mypassword", stored_hash)  // => true  // Verify password
```

**See also:** `hash_password`

*Since v0.3.11*

---

#### `verify_totp`

```ntnt
verify_totp(secret: String, code: String) -> Bool
```

Verify a TOTP code against a secret.

Checks if the provided 6-digit code is valid for the given secret. Allows for 30-second time window drift.

**Parameters:**

- `secret` — TOTP secret (base32 encoded)
- `code` — 6-digit code from authenticator app

**Returns:** true if code is valid, false otherwise

**Examples:**

```ntnt
verify_totp(secret, "123456")  // => true  // Verify 2FA code
```

**See also:** `totp_secret`, `totp_uri`

*Since v0.3.11*

---

## std/collections

Higher-order collection operations: transform, filter, reduce, sort, and group

```ntnt
import { push, pop, first } from "std/collections"
```

### Functions

| Function | Description |
|----------|-------------|
| [`concat`](#concat) | Concatenates two arrays into a new array. |
| [`entries`](#entries) | Returns an array of {key, value} maps from the map. |
| [`first`](#first) | Returns the first element of an array. |
| [`get_index`](#getindex) | Gets an element from an array by index with safe access. |
| [`get_key`](#getkey) | Gets a value from a map by key with safe access. |
| [`get_or`](#getor) | Gets a value from a map by key, returning a default if the key is missing. |
| [`has_key`](#haskey) | Returns true if the map contains the specified key. |
| [`has_value`](#hasvalue) | Deprecated: use includes() instead. Alias for backward compatibility. |
| [`includes`](#includes) | Returns true if the array includes the specified value (deep equality). |
| [`is_empty`](#isempty) | Returns true if the array or string is empty. |
| [`keys`](#keys) | Returns an array of all keys in the map. |
| [`last`](#last) | Returns the last element of an array. |
| [`merge`](#merge) | Shallow-merges two maps. Values from map2 win on key conflicts. |
| [`pop`](#pop) | Returns a tuple of [new array without last element, popped element as Option]. |
| [`push`](#push) | Returns a new array with the item appended. |
| [`reverse`](#reverse) | Returns a new array with elements in reverse order. |
| [`slice`](#slice) | Extracts a section of an array from start to end (exclusive). |
| [`values`](#values) | Returns an array of all values in the map. |

#### `concat`

```ntnt
concat(arr1: Array, arr2: Array) -> Array
```

Concatenates two arrays into a new array.

Does not mutate either input array. Returns a new array containing all elements of arr1 followed by all elements of arr2.

**Parameters:**

- `arr1` — The first array
- `arr2` — The second array to append

**Returns:** A new array containing elements from both arrays

**Examples:**

```ntnt
concat([1, 2], [3, 4])  // => [1, 2, 3, 4]  // Concatenate two arrays
```

**Errors:**

- **TypeError**: concat() requires two arrays — *Fix: Ensure both arguments are arrays*

**See also:** `push`, `slice`, `reverse`

*Since v0.1.0*

---

#### `entries`

```ntnt
entries(m: Map) -> Array<Map>
```

Returns an array of {key, value} maps from the map.

Each entry is a map with a "key" field (the string key) and a "value" field (the corresponding value). Access them as entry["key"] and entry["value"].

**Parameters:**

- `m` — The source map

**Returns:** An array of maps, each with "key" and "value" fields

**Examples:**

```ntnt
entries(map { "a": 1 })  // => [map { "key": "a", "value": 1 }]  // Get map entries as {key, value} maps
```

**Errors:**

- **TypeError**: entries() requires a map — *Fix: Ensure argument is a map*

**See also:** `keys`, `values`, `has_key`, `get_key`

*Since v0.1.0*

---

#### `first`

```ntnt
first(arr: Array, default?: Any) -> Option<Any> | Any
```

Returns the first element of an array.

Without a default, returns Option: Some(value) if the array is non-empty, None if empty. With a default, returns the first element directly or the default value if the array is empty.

**Parameters:**

- `arr` — The source array
- `default` — (optional) Value to return if the array is empty

**Returns:** The first element as Option, or the value/default directly

**Examples:**

```ntnt
first([1, 2, 3])  // => Some(1)  // First element wrapped in Option
first([], 0)  // => 0  // Default returned for empty array
```

**Errors:**

- **TypeError**: first() requires an array as first argument — *Fix: Ensure first argument is an array*

**See also:** `last`, `push`, `pop`, `slice`

*Since v0.1.0*

---

#### `get_index`

```ntnt
get_index(arr: Array, index: Int, default?: Any) -> Option<Any> | Any
```

Gets an element from an array by index with safe access.

Without a default, returns Option: Some(value) if the index is valid, None if out of bounds. With a default, returns the element directly or the default value if the index is out of bounds. Supports negative indexing: -1 is the last element, -2 is second-to-last, etc.

**Parameters:**

- `arr` — The source array
- `index` — The index to access (supports negative indexing)
- `default` — (optional) Value to return if the index is out of bounds

**Returns:** The element as Option, or the value/default directly

**Examples:**

```ntnt
get_index([10, 20, 30], 1)  // => Some(20)  // Index found, wrapped in Option
get_index([10, 20, 30], 5)  // => None  // Out of bounds returns None
get_index([10, 20, 30], 1, 0)  // => 20  // With default, returns value directly
get_index([10, 20, 30], 5, 0)  // => 0  // Out of bounds returns default
get_index([10, 20, 30], -1)  // => Some(30)  // Negative index from end
```

**Errors:**

- **TypeError**: get_index() requires an array and integer index — *Fix: Pass an array and an integer index*

**See also:** `first`, `last`, `get_key`, `slice`

*Since v0.4.0*

---

#### `get_key`

```ntnt
get_key(m: Map, key: String, default?: Any) -> Option<Any> | Any
```

Gets a value from a map by key with safe access.

Without a default, returns Option: Some(value) if the key exists, None if missing. With a default, returns the value directly or the default value if the key is not found.

**Parameters:**

- `m` — The source map
- `key` — The key to look up
- `default` — (optional) Value to return if the key is not found

**Returns:** The value as Option, or the value/default directly

**Examples:**

```ntnt
get_key(map { "a": 1 }, "a")  // => Some(1)  // Key found, wrapped in Option
get_key(map { "a": 1 }, "b", 0)  // => 0  // Key missing, default returned
```

**Errors:**

- **TypeError**: get_key() requires a map and string key — *Fix: Pass a map and a string key*

**See also:** `has_key`, `keys`, `values`, `entries`

*Since v0.1.0*

---

#### `get_or`

```ntnt
get_or(m: Map, key: String, default: Any) -> Any
```

Gets a value from a map by key, returning a default if the key is missing.

Unlike get_key which returns an Option, get_or always returns a plain value. If the key exists, returns its value. If not, returns the default.

**Parameters:**

- `m` — The source map
- `key` — The key to look up
- `default` — The value to return if the key is not found

**Returns:** The value for the key, or the default

**Examples:**

```ntnt
get_or(map { "name": "Alice" }, "name", "Anonymous")  // => "Alice"  // Key exists
get_or(map { "name": "Alice" }, "age", 0)  // => 0  // Key missing, returns default
```

**Errors:**

- **TypeError**: get_or() requires a map, string key, and default value — *Fix: Pass a map, string key, and default value*

**See also:** `get_key`, `has_key`, `merge`, `keys`

*Since v0.3.13*

---

#### `has_key`

```ntnt
has_key(m: Map, key: String) -> Bool
```

Returns true if the map contains the specified key.

**Parameters:**

- `m` — The map to search
- `key` — The key to look for

**Returns:** true if the key exists in the map, false otherwise

**Examples:**

```ntnt
has_key(map { "a": 1 }, "a")  // => true  // Key exists
has_key(map { "a": 1 }, "b")  // => false  // Key does not exist
```

**Errors:**

- **TypeError**: has_key() requires a map and string key — *Fix: Pass a map and a string key*

**See also:** `get_key`, `keys`, `values`, `entries`

*Since v0.1.0*

---

#### `has_value`

```ntnt
has_value(arr: Array, value: Any) -> Bool
```

Deprecated: use includes() instead. Alias for backward compatibility.

**Parameters:**

- `arr` — The array to search
- `value` — The value to look for

**Returns:** true if the value is found in the array, false otherwise

**Examples:**

```ntnt
has_value([1, 2, 3], 2)  // => true  // Deprecated alias for includes
```

**See also:** `includes`

*Since v0.4.0*

---

#### `includes`

```ntnt
includes(arr: Array, value: Any) -> Bool
```

Returns true if the array includes the specified value (deep equality).

Iterates through the array and checks each element for equality with the given value. Supports all value types including nested arrays and enums. Named `includes` (not `contains`) to avoid collision with contains() in std/string which checks substrings. Follows JavaScript convention.

**Parameters:**

- `arr` — The array to search
- `value` — The value to look for

**Returns:** true if the value is found in the array, false otherwise

**Examples:**

```ntnt
includes(["red", "green", "blue"], "green")  // => true  // Value found
includes([1, 2, 3], 5)  // => false  // Value not found
```

**Errors:**

- **TypeError**: includes() requires an array as first argument — *Fix: Ensure first argument is an array*

**See also:** `has_key`, `first`, `last`, `is_empty`

*Since v0.4.0*

---

#### `is_empty`

```ntnt
is_empty(x: Array | String) -> Bool
```

Returns true if the array or string is empty.

Works with both Array and String types. For arrays, checks if the length is zero. For strings, checks if the string has no characters.

**Parameters:**

- `x` — An array or string to check

**Returns:** true if the collection has no elements/characters, false otherwise

**Examples:**

```ntnt
is_empty([])  // => true  // Empty array
is_empty([1])  // => false  // Non-empty array
is_empty("")  // => true  // Empty string
```

**Errors:**

- **TypeError**: is_empty() requires array or string — *Fix: Pass an array or string*

*Since v0.1.0*

---

#### `keys`

```ntnt
keys(m: Map) -> Array<String>
```

Returns an array of all keys in the map.

The order of keys is not guaranteed to be consistent.

**Parameters:**

- `m` — The source map

**Returns:** An array of string keys

**Examples:**

```ntnt
keys(map { "a": 1, "b": 2 })  // => ["a", "b"]  // Get map keys
```

**Errors:**

- **TypeError**: keys() requires a map — *Fix: Ensure argument is a map*

**See also:** `values`, `entries`, `has_key`, `get_key`

*Since v0.1.0*

---

#### `last`

```ntnt
last(arr: Array, default?: Any) -> Option<Any> | Any
```

Returns the last element of an array.

Without a default, returns Option: Some(value) if the array is non-empty, None if empty. With a default, returns the last element directly or the default value if the array is empty.

**Parameters:**

- `arr` — The source array
- `default` — (optional) Value to return if the array is empty

**Returns:** The last element as Option, or the value/default directly

**Examples:**

```ntnt
last([1, 2, 3])  // => Some(3)  // Last element wrapped in Option
last([], 0)  // => 0  // Default returned for empty array
```

**Errors:**

- **TypeError**: last() requires an array as first argument — *Fix: Ensure first argument is an array*

**See also:** `first`, `push`, `pop`, `slice`

*Since v0.1.0*

---

#### `merge`

```ntnt
merge(map1: Map, map2: Map) -> Map
```

Shallow-merges two maps. Values from map2 win on key conflicts.

Returns a new map containing all key-value pairs from both maps. If both maps contain the same key, the value from map2 is used.

**Parameters:**

- `map1` — The base map
- `map2` — The map whose values take priority on conflict

**Returns:** A new map with merged key-value pairs

**Examples:**

```ntnt
merge(map { "a": 1, "b": 2 }, map { "b": 3, "c": 4 })  // => map { "a": 1, "b": 3, "c": 4 }  // Merge with conflict resolution
```

**Errors:**

- **TypeError**: merge() requires two maps — *Fix: Ensure both arguments are maps*

**See also:** `get_key`, `get_or`, `has_key`, `keys`, `values`

*Since v0.3.13*

---

#### `pop`

```ntnt
pop(arr: Array) -> Array<[Array, Option<Any>]>
```

Returns a tuple of [new array without last element, popped element as Option].

Does not mutate the original array. Returns a two-element array where the first element is the new array and the second is the popped value wrapped in an Option (Some(value) if the array was non-empty, None if empty).

**Parameters:**

- `arr` — The source array

**Returns:** A two-element array: [remaining array, Option of popped element]

**Examples:**

```ntnt
pop([1, 2, 3])  // => [[1, 2], Some(3)]  // Pop last element from array
```

**Errors:**

- **TypeError**: pop() requires an array — *Fix: Ensure argument is an array*

**See also:** `push`, `first`, `last`, `slice`

*Since v0.1.0*

---

#### `push`

```ntnt
push(arr: Array, item: Any) -> Array
```

Returns a new array with the item appended.

Does not mutate the original array. The new element is added at the end of the returned array.

**Parameters:**

- `arr` — The source array
- `item` — The element to append

**Returns:** A new array containing all original elements plus the new item

**Examples:**

```ntnt
push([1, 2], 3)  // => [1, 2, 3]  // Append element to array
```

**Errors:**

- **TypeError**: push() requires an array — *Fix: Ensure first argument is an array*

**See also:** `pop`, `concat`, `first`, `last`

*Since v0.1.0*

---

#### `reverse`

```ntnt
reverse(arr: Array) -> Array
```

Returns a new array with elements in reverse order.

Does not mutate the original array.

**Parameters:**

- `arr` — The source array

**Returns:** A new array with elements reversed

**Examples:**

```ntnt
reverse([1, 2, 3])  // => [3, 2, 1]  // Reverse array order
```

**Errors:**

- **TypeError**: reverse() requires an array — *Fix: Ensure argument is an array*

**See also:** `slice`, `concat`, `push`, `first`, `last`

*Since v0.1.0*

---

#### `slice`

```ntnt
slice(arr: Array, start: Int, end: Int) -> Array
```

Extracts a section of an array from start to end (exclusive).

Returns a new array containing elements from index start up to but not including index end. The end index is clamped to the array length.

**Parameters:**

- `arr` — The source array
- `start` — The starting index (inclusive)
- `end` — The ending index (exclusive)

**Returns:** A new array containing the sliced elements

**Examples:**

```ntnt
slice([1, 2, 3, 4], 1, 3)  // => [2, 3]  // Slice from index 1 to 3
```

**Errors:**

- **RuntimeError**: Invalid slice range — *Fix: Ensure start <= end and start <= array length*
- **TypeError**: slice() requires array, int, int — *Fix: Pass an array and two integer indices*

**See also:** `concat`, `reverse`, `first`, `last`

*Since v0.1.0*

---

#### `values`

```ntnt
values(m: Map) -> Array<Any>
```

Returns an array of all values in the map.

The order of values corresponds to the order of keys, which is not guaranteed to be consistent.

**Parameters:**

- `m` — The source map

**Returns:** An array of map values

**Examples:**

```ntnt
values(map { "a": 1, "b": 2 })  // => [1, 2]  // Get map values
```

**Errors:**

- **TypeError**: values() requires a map — *Fix: Ensure argument is a map*

**See also:** `keys`, `entries`, `has_key`, `get_key`

*Since v0.1.0*

---

## std/concurrent

Structured concurrency: tasks, channels, schedules, and cooperative cancellation

```ntnt
import { channel, send, recv } from "std/concurrent"
```

### Functions

| Function | Description |
|----------|-------------|
| [`after`](#after) | Runs a zero-parameter handler function after a delay. Returns a Task handle. Delay can be milliseconds (Int) or a human-readable string ("5s", "1m", "500ms"). The delay is cancellation-aware (50ms slices). |
| [`await_task`](#awaittask) | Blocks until the task completes and returns its result. Marks the task as consumed (the handle remains valid for try_await, which returns {status: "consumed"}). Returns Ok(value) on success, Err(message) on failure or panic. |
| [`cancel_schedule`](#cancelschedule) | Cancels a scheduled task. Sets the cancellation flag and removes from registry. Returns true if the schedule existed, false otherwise. |
| [`cancel_task`](#canceltask) | Requests cooperative cancellation of a task. Sets the cancellation flag; the task thread will exit at the next yield point (recv, recv_timeout, sleep_ms, or fetch). Does NOT force immediate termination. Returns true if the task existed, false otherwise. |
| [`channel`](#channel) | Creates a new unbounded channel and returns a [sender, receiver] pair. |
| [`close`](#close) | Closes a channel receiver by removing it from the registry. Once removed, future send(tx, ...) returns false (crossbeam Disconnected). recv(rx) immediately returns Unit since the id is no longer found. Returns true if existed, false otherwise. |
| [`recv`](#recv) | Receives a value from a channel. Blocks until a value is available. Returns Unit if all senders have been dropped (Disconnected) or the receiver was closed. This is a cancellation yield point: a cancelled task will exit here. Single-consumer: the receiver lock is held for the blocking duration. |
| [`recv_timeout`](#recvtimeout) | Receives with timeout. Returns None if timeout expires or all senders disconnected. Loops in ≤100ms slices checking cancellation between iterations. This is a cancellation yield point. |
| [`schedule`](#schedule) | Runs a zero-parameter handler repeatedly at the given interval. Returns a Schedule handle. Interval can be milliseconds (Int) or a string ("5s", "1m"). Zero intervals are rejected. Each tick spawns a thread with catch_unwind; overlap prevention ensures a new tick won't start until the previous one finishes. Panics in tick execution are caught and logged — they don't kill the schedule. |
| [`select`](#select) | Waits for the first available value from any of the given receiver handles. Returns a map with "status": "ok", "channel" (the RxChannel that fired), and "value" (the received value). On timeout: returns {"status": "timeout"}. If all channels are closed/disconnected: returns {"status": "closed"}. All return shapes include a "status" key for consistent pattern matching. This is a cancellation yield point. |
| [`send`](#send) | Sends a value through a channel using the sender handle (first element of channel()). Returns false if the receiver has been closed (crossbeam Disconnected). Serializable types: Int, Float, Bool, String, Array, Map, Struct, Enum. |
| [`sleep_ms`](#sleepms) | Pauses execution for specified milliseconds. This is a cancellation yield point: a cancelled task will exit during sleep_ms(). Uses 50ms slices internally. Note: sleep() from std/time is NOT cancellation-aware — use this for spawned tasks. |
| [`spawn`](#spawn) | Spawns a zero-parameter function as a background task. Returns a Task handle. The handler's closure environment is serialized for cross-thread use. Serializable capture types: Int, Float, Bool, String, Array, Map, Struct, Enum. The handler must have zero parameters (including no defaults). |
| [`thread_count`](#threadcount) | Returns the number of available CPU threads. Useful for sizing parallel work. |
| [`try_await`](#tryawait) | Non-blocking peek at task state. Does NOT remove the task from registry. Returns a map with "status" ("running", "completed", "failed", "panicked", "consumed", "expired") and "result" (Ok(value), Err(message), or None if still running/consumed/expired). |
| [`try_recv`](#tryrecv) | Non-blocking receive. Returns None if no value is available or all senders disconnected. |

#### `after`

```ntnt
after(delay: Int | String, handler: Function) -> Task
```

Runs a zero-parameter handler function after a delay. Returns a Task handle. Delay can be milliseconds (Int) or a human-readable string ("5s", "1m", "500ms"). The delay is cancellation-aware (50ms slices).

**Parameters:**

- `delay` — Delay in milliseconds (Int) or as a string interval
- `handler` — A zero-parameter function to run after the delay

**Returns:** Task handle

**Examples:**

```ntnt
after(1000, fn() { print("delayed!") })  // Run after 1 second
```

**See also:** `spawn`, `await_task`, `schedule`

*Since v0.4.6*

---

#### `await_task`

```ntnt
await_task(task: Task) -> Result<Any, String>
```

Blocks until the task completes and returns its result. Marks the task as consumed (the handle remains valid for try_await, which returns {status: "consumed"}). Returns Ok(value) on success, Err(message) on failure or panic.

**Parameters:**

- `task` — The task handle from spawn() or after()

**Returns:** Result containing the task's return value or error message

**Examples:**

```ntnt
await_task(task)  // => Ok(42)  // Wait for task result
```

**See also:** `spawn`, `try_await`, `cancel_task`

*Since v0.4.6*

---

#### `cancel_schedule`

```ntnt
cancel_schedule(schedule: Schedule) -> Bool
```

Cancels a scheduled task. Sets the cancellation flag and removes from registry. Returns true if the schedule existed, false otherwise.

**Parameters:**

- `schedule` — The schedule handle from schedule()

**Returns:** Bool indicating whether the schedule was cancelled

**Examples:**

```ntnt
cancel_schedule(sched)  // => true  // Cancel a scheduled task
```

**See also:** `schedule`

*Since v0.4.6*

---

#### `cancel_task`

```ntnt
cancel_task(task: Task) -> Bool
```

Requests cooperative cancellation of a task. Sets the cancellation flag; the task thread will exit at the next yield point (recv, recv_timeout, sleep_ms, or fetch). Does NOT force immediate termination. Returns true if the task existed, false otherwise.

**Parameters:**

- `task` — The task handle

**Returns:** Bool indicating whether the cancellation was requested

**Examples:**

```ntnt
cancel_task(task)  // => true  // Cancel a running task
```

**See also:** `spawn`, `await_task`

*Since v0.4.6*

---

#### `channel`

```ntnt
channel() -> [TxChannel, RxChannel]
```

Creates a new unbounded channel and returns a [sender, receiver] pair.

The sender (TxChannel) and receiver (RxChannel) are separate handles — exactly like Rust's own channels. Pass the TxChannel to whoever should send; keep (or pass) the RxChannel to whoever should recv.

Ownership semantics: when ALL TxChannel clones for a channel are dropped (e.g. a spawned task exits before or after calling send()), the receiver automatically sees Disconnected and recv() returns Unit. No sentinel injection required — this is structural, not approximate.

Channels are single-consumer: only one task should call recv() at a time.

**Returns:** Array containing [TxChannel, RxChannel]

**Examples:**

```ntnt
let [tx, rx] = channel()  // Create a channel for inter-task communication
// Pass tx to a spawned task; recv on rx disconnects naturally if task fails
let [tx, rx] = channel()
let task = spawn(fn() { send(tx, "hello") })
let msg = recv(rx)
// => "hello"
```

**See also:** `send`, `recv`, `close`, `select`

*Since v0.4.6*

---

#### `close`

```ntnt
close(rx: RxChannel) -> Bool
```

Closes a channel receiver by removing it from the registry. Once removed, future send(tx, ...) returns false (crossbeam Disconnected). recv(rx) immediately returns Unit since the id is no longer found. Returns true if existed, false otherwise.

**Parameters:**

- `rx` — The RxChannel receiver handle (second element of channel())

**Examples:**

```ntnt
let [tx, rx] = channel()  // Close the receiver end
close(rx)  // => true
```

**See also:** `channel`

*Since v0.4.6*

---

#### `recv`

```ntnt
recv(rx: RxChannel) -> Any
```

Receives a value from a channel. Blocks until a value is available. Returns Unit if all senders have been dropped (Disconnected) or the receiver was closed. This is a cancellation yield point: a cancelled task will exit here. Single-consumer: the receiver lock is held for the blocking duration.

**Parameters:**

- `rx` — The RxChannel receiver handle (second element of channel())

**Examples:**

```ntnt
let [tx, rx] = channel()  // Block until a value is received
recv(rx)
```

**See also:** `channel`, `send`, `try_recv`, `recv_timeout`

*Since v0.4.6*

---

#### `recv_timeout`

```ntnt
recv_timeout(rx: RxChannel, millis: Int) -> Option<Any>
```

Receives with timeout. Returns None if timeout expires or all senders disconnected. Loops in ≤100ms slices checking cancellation between iterations. This is a cancellation yield point.

**Parameters:**

- `rx` — The RxChannel receiver handle (second element of channel())
- `millis` — Timeout in milliseconds (negative values clamped to 0)

**Examples:**

```ntnt
let [tx, rx] = channel()  // Wait up to 5 seconds for a value
recv_timeout(rx, 5000)
```

**See also:** `recv`, `try_recv`

*Since v0.4.6*

---

#### `schedule`

```ntnt
schedule(interval: Int | String, handler: Function) -> Schedule
```

Runs a zero-parameter handler repeatedly at the given interval. Returns a Schedule handle. Interval can be milliseconds (Int) or a string ("5s", "1m"). Zero intervals are rejected. Each tick spawns a thread with catch_unwind; overlap prevention ensures a new tick won't start until the previous one finishes. Panics in tick execution are caught and logged — they don't kill the schedule.

**Parameters:**

- `interval` — Interval in milliseconds (Int) or as a string
- `handler` — A zero-parameter function to run on each tick

**Returns:** Schedule handle for use with cancel_schedule

**Examples:**

```ntnt
schedule(5000, fn() { print("tick") })  // Run every 5 seconds
```

**See also:** `cancel_schedule`, `after`

*Since v0.4.6*

---

#### `select`

```ntnt
select(channels: Array<RxChannel>, timeout_ms?: Int | String) -> Map
```

Waits for the first available value from any of the given receiver handles. Returns a map with "status": "ok", "channel" (the RxChannel that fired), and "value" (the received value). On timeout: returns {"status": "timeout"}. If all channels are closed/disconnected: returns {"status": "closed"}. All return shapes include a "status" key for consistent pattern matching. This is a cancellation yield point.

**Parameters:**

- `channels` — Array of RxChannel handles to wait on
- `timeout_ms` — Optional timeout in milliseconds (Int) or as a string interval

**Returns:** Map with channel/value on success, or status on timeout/closed

**Examples:**

```ntnt
let [tx_a, rx_a] = channel()  // Wait for first value from either channel
let [tx_b, rx_b] = channel()
select([rx_a, rx_b])
select([rx_a, rx_b], 5000)  // Wait up to 5 seconds
```

**See also:** `channel`, `recv`, `recv_timeout`

*Since v0.4.6*

---

#### `send`

```ntnt
send(tx: TxChannel, value: Any) -> Bool
```

Sends a value through a channel using the sender handle (first element of channel()). Returns false if the receiver has been closed (crossbeam Disconnected). Serializable types: Int, Float, Bool, String, Array, Map, Struct, Enum.

**Parameters:**

- `tx` — The TxChannel sender handle (first element of channel())
- `value` — The value to send (must be serializable)

**Examples:**

```ntnt
send(tx, "hello")  // => true  // Send a string through the channel
```

**See also:** `channel`, `recv`, `recv_timeout`

*Since v0.4.6*

---

#### `sleep_ms`

```ntnt
sleep_ms(ms: Int) -> Unit
```

Pauses execution for specified milliseconds. This is a cancellation yield point: a cancelled task will exit during sleep_ms(). Uses 50ms slices internally. Note: sleep() from std/time is NOT cancellation-aware — use this for spawned tasks.

**Parameters:**

- `ms` — Duration to sleep in milliseconds

**Examples:**

```ntnt
sleep_ms(1000)  // Sleep for 1 second (cancellation-aware)
```

*Since v0.4.6*

---

#### `spawn`

```ntnt
spawn(handler: Function) -> Task
```

Spawns a zero-parameter function as a background task. Returns a Task handle. The handler's closure environment is serialized for cross-thread use. Serializable capture types: Int, Float, Bool, String, Array, Map, Struct, Enum. The handler must have zero parameters (including no defaults).

**Parameters:**

- `handler` — A zero-parameter function to run in the background

**Returns:** Task handle for use with await_task, try_await, cancel_task

**Examples:**

```ntnt
spawn(fn() { 42 })  // Spawn a background task
```

**See also:** `await_task`, `try_await`, `cancel_task`

*Since v0.4.6*

---

#### `thread_count`

```ntnt
thread_count() -> Int
```

Returns the number of available CPU threads. Useful for sizing parallel work.

**Examples:**

```ntnt
thread_count()  // => 8  // Number of CPU threads
```

*Since v0.4.6*

---

#### `try_await`

```ntnt
try_await(task: Task) -> Map
```

Non-blocking peek at task state. Does NOT remove the task from registry. Returns a map with "status" ("running", "completed", "failed", "panicked", "consumed", "expired") and "result" (Ok(value), Err(message), or None if still running/consumed/expired).

**Parameters:**

- `task` — The task handle

**Returns:** Map with status and result fields

**Examples:**

```ntnt
try_await(task)  // => {"status": "running", "result": None}  // Check task status
```

**See also:** `spawn`, `await_task`, `cancel_task`

*Since v0.4.6*

---

#### `try_recv`

```ntnt
try_recv(rx: RxChannel) -> Option<Any>
```

Non-blocking receive. Returns None if no value is available or all senders disconnected.

**Parameters:**

- `rx` — The RxChannel receiver handle (second element of channel())

**Examples:**

```ntnt
let [tx, rx] = channel()  // Check for a value without blocking
try_recv(rx)
```

**See also:** `recv`, `recv_timeout`

*Since v0.4.6*

---

## std/crypto

Cryptographic hashing and random value generation

```ntnt
import { sha256, sha256_bytes, hmac_sha256 } from "std/crypto"
```

### Functions

| Function | Description |
|----------|-------------|
| [`aes_decrypt`](#aesdecrypt) | Decrypts AES-256-GCM encrypted data produced by aes_encrypt. The input is a Base64-encoded string containing the nonce and ciphertext. The key must be the same 64-character hex string used for encryption. |
| [`aes_encrypt`](#aesencrypt) | Encrypts plaintext using AES-256-GCM authenticated encryption. The key must be a 64-character hex string (32 bytes). A random 96-bit nonce is generated for each call and prepended to the ciphertext before Base64 encoding. |
| [`aes_generate_key`](#aesgeneratekey) | Generates a random 256-bit AES key, returned as a 64-character hex string. Use this key with aes_encrypt and aes_decrypt. |
| [`argon2_hash`](#argon2hash) | Hashes a password using Argon2id, the recommended password hashing algorithm. Returns a PHC-format string that includes the salt and parameters. Uses OWASP-recommended defaults: m=19456 KiB, t=2 iterations, p=1 parallelism. |
| [`argon2_verify`](#argon2verify) | Verifies a password against an Argon2 hash in PHC format. Returns true if the password matches, false otherwise (including for invalid hashes). |
| [`base64_decode`](#base64decode) | Decodes a standard Base64-encoded string back to plaintext. Returns Err if the input is not valid Base64 or not valid UTF-8. |
| [`base64_encode`](#base64encode) | Encodes a string using standard Base64 encoding (RFC 4648). |
| [`base64url_decode`](#base64urldecode) | Decodes a URL-safe Base64-encoded string (no padding) back to plaintext. Returns Err if the input is not valid URL-safe Base64 or not valid UTF-8. |
| [`base64url_encode`](#base64urlencode) | Encodes a string using URL-safe Base64 encoding (no padding). Uses the URL_SAFE_NO_PAD alphabet, suitable for URLs and filenames. |
| [`csrf_generate`](#csrfgenerate) | Generate a CSRF token and its HMAC signature for stateless CSRF protection. |
| [`csrf_validate`](#csrfvalidate) | Validate a CSRF token against its HMAC hash. |
| [`hash_password`](#hashpassword) | Hash a password using bcrypt with configurable cost factor. |
| [`hex_decode`](#hexdecode) | Decodes hex string to byte array. Returns Err for invalid hex. |
| [`hex_encode`](#hexencode) | Encodes bytes or string as hex. |
| [`hmac_sha256`](#hmacsha256) | HMAC-SHA256 message authentication code as hex string. |
| [`is_valid_hash`](#isvalidhash) | Check if a string is a valid bcrypt hash format. |
| [`random_bytes`](#randombytes) | Generates n cryptographically secure random bytes. Size limit 0-1048576. |
| [`random_hex`](#randomhex) | Generates n random bytes as hex string (2n chars). |
| [`sha256`](#sha256) | SHA-256 hash as hex string. Accepts string or byte array. |
| [`sha256_bytes`](#sha256bytes) | SHA-256 hash as byte array. Returns array of 32 integers (0-255). |
| [`uuid`](#uuid) | Generates a random UUID v4 string. |
| [`verify_password`](#verifypassword) | Verify a password against a bcrypt hash. |

#### `aes_decrypt`

```ntnt
aes_decrypt(ciphertext: String, key: String) -> Result<String, String>
```

Decrypts AES-256-GCM encrypted data produced by aes_encrypt. The input is a Base64-encoded string containing the nonce and ciphertext. The key must be the same 64-character hex string used for encryption.

**Parameters:**

- `ciphertext` — The Base64-encoded string from aes_encrypt
- `key` — A 64-character hex string (256-bit key)

**Returns:** Ok(plaintext) on success, Err(message) on failure (wrong key, tampered data, etc.)

**Examples:**

```ntnt
aes_decrypt(encrypted, key)  // Returns Ok with original plaintext
```

**See also:** `aes_encrypt`, `aes_generate_key`

*Since v0.3.13*

---

#### `aes_encrypt`

```ntnt
aes_encrypt(plaintext: String, key: String) -> Result<String, String>
```

Encrypts plaintext using AES-256-GCM authenticated encryption. The key must be a 64-character hex string (32 bytes). A random 96-bit nonce is generated for each call and prepended to the ciphertext before Base64 encoding.

**Parameters:**

- `plaintext` — The string to encrypt
- `key` — A 64-character hex string (256-bit key from aes_generate_key)

**Returns:** Ok(base64_encoded_nonce_and_ciphertext) on success, Err(message) on failure

**Examples:**

```ntnt
aes_encrypt("secret data", aes_generate_key())  // Returns Ok with base64 ciphertext
```

**See also:** `aes_decrypt`, `aes_generate_key`

*Since v0.3.13*

---

#### `aes_generate_key`

```ntnt
aes_generate_key() -> String
```

Generates a random 256-bit AES key, returned as a 64-character hex string. Use this key with aes_encrypt and aes_decrypt.

**Returns:** 64-character hex string representing a 256-bit key

**Examples:**

```ntnt
aes_generate_key()  // Returns a 64-char hex string like 'a1b2c3d4...'
```

**See also:** `aes_encrypt`, `aes_decrypt`

*Since v0.3.13*

---

#### `argon2_hash`

```ntnt
argon2_hash(password: String) -> String
```

Hashes a password using Argon2id, the recommended password hashing algorithm. Returns a PHC-format string that includes the salt and parameters. Uses OWASP-recommended defaults: m=19456 KiB, t=2 iterations, p=1 parallelism.

**Parameters:**

- `password` — The plaintext password to hash

**Returns:** PHC-format hash string starting with $argon2id$

**Examples:**

```ntnt
argon2_hash("my_password")  // Returns '$argon2id$v=19$m=19456,t=2,p=1$...'
```

**See also:** `argon2_verify`, `hash_password`

*Since v0.3.13*

---

#### `argon2_verify`

```ntnt
argon2_verify(password: String, hash: String) -> Bool
```

Verifies a password against an Argon2 hash in PHC format. Returns true if the password matches, false otherwise (including for invalid hashes).

**Parameters:**

- `password` — The plaintext password to verify
- `hash` — The Argon2 PHC-format hash string to verify against

**Returns:** true if password matches, false otherwise

**Examples:**

```ntnt
argon2_verify("my_password", argon2_hash("my_password"))  // => true  // Correct password
argon2_verify("wrong", argon2_hash("my_password"))  // => false  // Wrong password
```

**See also:** `argon2_hash`, `verify_password`

*Since v0.3.13*

---

#### `base64_decode`

```ntnt
base64_decode(encoded: String) -> Result<String, String>
```

Decodes a standard Base64-encoded string back to plaintext. Returns Err if the input is not valid Base64 or not valid UTF-8.

**Parameters:**

- `encoded` — The Base64-encoded string to decode

**Returns:** Ok(decoded_string) on success, Err(message) on failure

**Examples:**

```ntnt
base64_decode("SGVsbG8sIFdvcmxkIQ==")  // => Ok("Hello, World!")  // Decode base64 string
base64_decode("!!!invalid!!!")  // => Err("...")  // Invalid base64 returns Err
```

**See also:** `base64_encode`, `base64url_decode`

*Since v0.3.13*

---

#### `base64_encode`

```ntnt
base64_encode(data: String) -> String
```

Encodes a string using standard Base64 encoding (RFC 4648).

**Parameters:**

- `data` — The string to encode

**Returns:** Base64-encoded string

**Examples:**

```ntnt
base64_encode("Hello, World!")  // => "SGVsbG8sIFdvcmxkIQ=="  // Standard base64 encoding
```

**See also:** `base64_decode`, `base64url_encode`

*Since v0.3.13*

---

#### `base64url_decode`

```ntnt
base64url_decode(encoded: String) -> Result<String, String>
```

Decodes a URL-safe Base64-encoded string (no padding) back to plaintext. Returns Err if the input is not valid URL-safe Base64 or not valid UTF-8.

**Parameters:**

- `encoded` — The URL-safe Base64-encoded string to decode

**Returns:** Ok(decoded_string) on success, Err(message) on failure

**Examples:**

```ntnt
base64url_decode("SGVsbG8sIFdvcmxkIQ")  // => Ok("Hello, World!")  // Decode URL-safe base64
base64url_decode("!!!")  // => Err("...")  // Invalid input returns Err
```

**See also:** `base64url_encode`, `base64_decode`

*Since v0.3.13*

---

#### `base64url_encode`

```ntnt
base64url_encode(data: String) -> String
```

Encodes a string using URL-safe Base64 encoding (no padding). Uses the URL_SAFE_NO_PAD alphabet, suitable for URLs and filenames.

**Parameters:**

- `data` — The string to encode

**Returns:** URL-safe Base64-encoded string without padding

**Examples:**

```ntnt
base64url_encode("Hello, World!")  // => "SGVsbG8sIFdvcmxkIQ"  // URL-safe base64 (no padding)
```

**See also:** `base64url_decode`, `base64_encode`

*Since v0.3.13*

---

#### `csrf_generate`

```ntnt
csrf_generate() -> Map<String, String>
```

Generate a CSRF token and its HMAC signature for stateless CSRF protection.

Returns a map with `token` (random value) and `hash` (HMAC-SHA256 signature). Embed `token` in a hidden form field and `hash` in another hidden field or cookie. Validate on POST with `csrf_validate(token, hash)`.

**Returns:** A map with keys `"token"` and `"hash"`.

**Examples:**

```ntnt
csrf_generate()  // Returns map { \"token\": \"...\", \"hash\": \"...\" }
```

**See also:** `csrf_validate`, `hmac_sha256`, `uuid`

*Since v0.3.0*

---

#### `csrf_validate`

```ntnt
csrf_validate(token: String, hash: String) -> Bool
```

Validate a CSRF token against its HMAC hash.

Compares the provided token's HMAC-SHA256 against the provided hash using the same per-process secret used by `csrf_generate()`.

**Parameters:**

- `token` — The CSRF token from the form submission.
- `hash` — The HMAC hash from the form submission.

**Returns:** `true` if the token is valid, `false` otherwise.

**Examples:**

```ntnt
csrf_validate("some-token", "some-hash")  // => false  // Invalid token returns false
```

**See also:** `csrf_generate`, `hmac_sha256`

*Since v0.3.0*

---

#### `hash_password`

```ntnt
hash_password(password: String, cost?: Int) -> Result<String, String>
```

Hash a password using bcrypt with configurable cost factor.

Returns a bcrypt hash string that can be stored in the database. The hash includes the salt, so no separate salt storage is needed. The default cost of 12 provides good security for most applications. Higher costs are more secure but slower — each increment doubles the time.

**Parameters:**

- `password` — The plaintext password to hash
- `cost` — Work factor (10-31). Default 12. Higher = slower but more secure.

**Returns:** Ok(hash_string) on success, Err(message) on failure

**Examples:**

```ntnt
hash_password("secret123")  // => Ok("$2b$12$...")  // Hash with default cost
hash_password("secret123", 10)  // => Ok("$2b$10$...")  // Hash with minimum cost (faster but still secure)
hash_password("secret123", 14)  // => Ok("$2b$14$...")  // Hash with higher cost (more secure)
```

**Errors:**

- **InvalidCost**: Cost must be between 10 and 31 — *Fix: Use a cost value of 10 or higher (OWASP minimum)*

**See also:** `verify_password`, `is_valid_hash`

*Since v0.4.0*

---

#### `hex_decode`

```ntnt
hex_decode(hex: String) -> Result<Array<Int>, String>
```

Decodes hex string to byte array. Returns Err for invalid hex.

**Parameters:**

- `hex` — The hex string to decode

**Examples:**

```ntnt
hex_decode("6869")  // => Ok([104, 105])  // Decode hex to bytes for 'hi'
hex_decode("zz")  // => Err("...")  // Invalid hex returns Err
```

**See also:** `hex_encode`

*Since v0.2.0*

---

#### `hex_encode`

```ntnt
hex_encode(data: Array<Int> | String) -> String
```

Encodes bytes or string as hex.

**Parameters:**

- `data` — Byte array or string to encode

**Examples:**

```ntnt
hex_encode("hi")  // => "6869"
```

**See also:** `hex_decode`

*Since v0.2.0*

---

#### `hmac_sha256`

```ntnt
hmac_sha256(key: String, data: String) -> String
```

HMAC-SHA256 message authentication code as hex string.

**Parameters:**

- `key` — The secret key for HMAC
- `data` — The data to authenticate

**Examples:**

```ntnt
hmac_sha256("secret", "message")  // Returns HMAC-SHA256 as 64-char hex string
```

**See also:** `sha256`

*Since v0.2.0*

---

#### `is_valid_hash`

```ntnt
is_valid_hash(hash: String) -> Bool
```

Check if a string is a valid bcrypt hash format.

This is useful for migrations or validating data before calling verify_password. Does NOT verify the hash is correct — only that it has valid bcrypt structure.

**Parameters:**

- `hash` — The string to check

**Returns:** true if the string matches bcrypt hash format, false otherwise

**Examples:**

```ntnt
is_valid_hash("$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TtxMQJqhN8/X4.V")  // => true  // Valid bcrypt hash
is_valid_hash("not-a-hash")  // => false  // Plain string
is_valid_hash("")  // => false  // Empty string
is_valid_hash("$2a$10$N9qo8uLOickgx2ZMRZoMye")  // => false  // Truncated hash
```

**See also:** `hash_password`, `verify_password`

*Since v0.4.0*

---

#### `random_bytes`

```ntnt
random_bytes(n: Int) -> Array<Int>
```

Generates n cryptographically secure random bytes. Size limit 0-1048576.

**Parameters:**

- `n` — Number of random bytes to generate

**Examples:**

```ntnt
random_bytes(16)  // Returns 16 random bytes as array of integers 0-255
```

**Errors:**

- **RuntimeError**: size must be 0-1048576 — *Fix: Reduce the requested byte count*

*Since v0.2.0*

---

#### `random_hex`

```ntnt
random_hex(n: Int) -> String
```

Generates n random bytes as hex string (2n chars).

**Parameters:**

- `n` — Number of random bytes to generate

**Examples:**

```ntnt
random_hex(8)  // Returns 16-char hex string from 8 random bytes
```

**See also:** `random_bytes`

*Since v0.2.0*

---

#### `sha256`

```ntnt
sha256(data: String | Array<Int>) -> String
```

SHA-256 hash as hex string. Accepts string or byte array.

**Parameters:**

- `data` — The input data to hash (string or byte array)

**Examples:**

```ntnt
sha256("hello")  // => "2cf24dba..."  // Hash a string
```

*Since v0.2.0*

---

#### `sha256_bytes`

```ntnt
sha256_bytes(data: String) -> Array<Int>
```

SHA-256 hash as byte array. Returns array of 32 integers (0-255).

**Parameters:**

- `data` — The input string to hash

**Examples:**

```ntnt
sha256_bytes("hello")[0]  // => 44  // First byte of SHA-256 hash of 'hello'
```

**See also:** `sha256`

*Since v0.2.0*

---

#### `uuid`

```ntnt
uuid() -> String
```

Generates a random UUID v4 string.

**Examples:**

```ntnt
uuid()  // => "550e8400-e29b-41d4-a716-446655440000"  // Random UUID v4
```

*Since v0.2.0*

---

#### `verify_password`

```ntnt
verify_password(password: String, hash: String) -> Result<Bool, String>
```

Verify a password against a bcrypt hash.

Returns Ok(true) if the password matches, Ok(false) if it doesn't match, or Err if the hash is malformed.

**Parameters:**

- `password` — The plaintext password to verify
- `hash` — The bcrypt hash to verify against

**Returns:** Ok(true) if match, Ok(false) if no match, Err(message) if hash is invalid

**Examples:**

```ntnt
verify_password("secret123", "$2b$12$...valid_hash...")  // => Ok(true)  // Correct password
verify_password("wrong", "$2b$12$...valid_hash...")  // => Ok(false)  // Wrong password
verify_password("secret", "not-a-hash")  // => Err("...")  // Invalid hash format
```

**See also:** `hash_password`, `is_valid_hash`

*Since v0.4.0*

---

## std/csv

CSV parsing and generation

```ntnt
import { parse_csv, parse_with_headers, stringify } from "std/csv"
```

### Functions

| Function | Description |
|----------|-------------|
| [`parse_csv`](#parsecsv) | Parses a CSV string into an array of rows, where each row is an array of strings. |
| [`parse_with_headers`](#parsewithheaders) | Parses CSV into an array of maps using the first row as column headers. |
| [`stringify`](#stringify) | Converts an array of rows to a CSV string. |
| [`stringify_with_headers`](#stringifywithheaders) | Converts an array of maps to a CSV string with a header row. |

#### `parse_csv`

```ntnt
parse_csv(csv: String) -> Array<Array<String>>
```

Parses a CSV string into an array of rows, where each row is an array of strings.

Handles quoted fields with commas and escaped double-quotes. Empty rows are automatically skipped. Supports both LF and CRLF line endings.

**Parameters:**

- `csv` — The CSV string to parse

**Returns:** Array of rows, each row an array of field strings

**Examples:**

```ntnt
parse_csv("a,b\n1,2")  // => [["a", "b"], ["1", "2"]]  // Basic CSV parsing
```

**Errors:**

- **TypeError**: parse_csv() requires a string — *Fix: Pass a CSV string*

**See also:** `parse_with_headers`, `stringify`

*Since v0.2.0*

---

#### `parse_with_headers`

```ntnt
parse_with_headers(csv: String) -> Array<Map<String, String>>
```

Parses CSV into an array of maps using the first row as column headers.

The first row defines the map keys. Each subsequent row becomes a map with those keys mapped to the corresponding field values.

**Parameters:**

- `csv` — The CSV string with a header row

**Returns:** Array of maps, one per data row

**Examples:**

```ntnt
parse_with_headers("name,age\nAlice,30")  // => [map { "name": "Alice", "age": "30" }]  // Headers become map keys
```

**Errors:**

- **TypeError**: csv.parse_with_headers() requires a string — *Fix: Pass a CSV string*

**See also:** `parse_csv`, `stringify_with_headers`

*Since v0.2.0*

---

#### `stringify`

```ntnt
stringify(rows: Array<Array<Any>>) -> String
```

Converts an array of rows to a CSV string.

Values are converted to strings. Fields containing commas, quotes, or newlines are automatically quoted with escaped double-quotes.

**Parameters:**

- `rows` — Array of arrays, each inner array is a row

**Returns:** CSV-formatted string

**Examples:**

```ntnt
stringify([["a", "b"], [1, 2]])  // => "a,b\n1,2"  // Array rows to CSV
```

**Errors:**

- **TypeError**: csv.stringify() requires an array — *Fix: Pass an array of row arrays*

**See also:** `stringify_with_headers`, `parse_csv`

*Since v0.2.0*

---

#### `stringify_with_headers`

```ntnt
stringify_with_headers(rows: Array<Map>, headers: Array<String>) -> String
```

Converts an array of maps to a CSV string with a header row.

The headers array defines both the column order and the header row. Each map row is serialized by looking up the header keys.

**Parameters:**

- `rows` — Array of maps, one per data row
- `headers` — Array of column names (also used as map keys)

**Returns:** CSV string with header row followed by data rows

**Examples:**

```ntnt
stringify_with_headers([map { "name": "Alice" }], ["name"])  // => "name\nAlice"  // Maps to CSV with headers
```

**Errors:**

- **TypeError**: csv.stringify_with_headers() first arg must be array — *Fix: Pass array of maps*

**See also:** `stringify`, `parse_with_headers`

*Since v0.2.0*

---

## std/env

Environment variable access

```ntnt
import { get_env, args, cwd } from "std/env"
```

### Functions

| Function | Description |
|----------|-------------|
| [`args`](#args) | Returns command-line arguments as an array of strings. |
| [`cwd`](#cwd) | Returns the current working directory as a string. |
| [`get_env`](#getenv) | Gets an environment variable by name. |
| [`load_env`](#loadenv) | Loads environment variables from a .env file. |

#### `args`

```ntnt
args() -> Array<String>
```

Returns command-line arguments as an array of strings.

The first element is the program name, followed by any arguments passed on the command line.

**Returns:** Array of argument strings

**Examples:**

```ntnt
args()  // => ["ntnt", "script.tnt", "--flag"]  // Program name and arguments
```

*Since v0.1.0*

---

#### `cwd`

```ntnt
cwd() -> String
```

Returns the current working directory as a string.

**Returns:** Absolute path of the current working directory

**Examples:**

```ntnt
cwd()  // => "/Users/dev/project"  // Current directory path
```

**Errors:**

- **RuntimeError**: Failed to get cwd — *Fix: Ensure the process has filesystem access*

*Since v0.1.0*

---

#### `get_env`

```ntnt
get_env(name: String) -> Option<String>
```

Gets an environment variable by name.

Returns Some(value) if the variable is set, or None if it is not defined in the current process environment.

**Parameters:**

- `name` — The environment variable name

**Returns:** Option containing the value or None

**Examples:**

```ntnt
get_env("HOME")  // => Some("/Users/...")  // Get home directory
get_env("UNDEFINED_VAR")  // => None  // Missing variable
```

**See also:** `load_env`

*Since v0.1.0*

---

#### `load_env`

```ntnt
load_env(path: String) -> Result<Unit, String>
```

Loads environment variables from a .env file.

Reads the file line by line, parsing KEY=VALUE pairs. Lines starting with # are treated as comments and skipped. Variables are set in the current process environment.

**Parameters:**

- `path` — Path to the .env file

**Returns:** Result indicating success or file read error

**Examples:**

```ntnt
load_env(".env")  // Load default .env file
load_env(".env.local")  // Load environment-specific file
```

**See also:** `get_env`

*Since v0.2.0*

---

## std/fs

File system operations: reading, writing, and directory management

```ntnt
import { read_file, read_bytes, write_file } from "std/fs"
```

### Functions

| Function | Description |
|----------|-------------|
| [`append_file`](#appendfile) | Append a string to the end of a file, creating it if it does not exist. |
| [`copy`](#copy) | Copy a file to a new location, returning the number of bytes copied. |
| [`exists`](#exists) | Check whether a file or directory exists at the given path. |
| [`file_size`](#filesize) | Get the size of a file in bytes. |
| [`is_dir`](#isdir) | Check whether the path points to a directory. |
| [`is_file`](#isfile) | Check whether the path points to a regular file. |
| [`mkdir`](#mkdir) | Create a single directory. |
| [`mkdir_all`](#mkdirall) | Create a directory and all missing parent directories. |
| [`read_bytes`](#readbytes) | Read the entire contents of a file as raw bytes. |
| [`read_file`](#readfile) | Read the entire contents of a file as a UTF-8 string. |
| [`readdir`](#readdir) | List the entries of a directory. |
| [`remove`](#remove) | Remove a file from the filesystem. |
| [`remove_dir`](#removedir) | Remove an empty directory. |
| [`remove_dir_all`](#removedirall) | Recursively remove a directory and all of its contents. |
| [`rename`](#rename) | Rename or move a file or directory. |
| [`write_file`](#writefile) | Write a string to a file, creating or overwriting it. |

#### `append_file`

```ntnt
append_file(path: String, content: String) -> Result<Unit, String>
```

Append a string to the end of a file, creating it if it does not exist.

Opens the file in append mode and writes the content at the end. If the file does not exist it is created. Existing content is preserved.

**Parameters:**

- `path` — The filesystem path to append to.
- `content` — The string content to append.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
append_file("log.txt", "new line\n")  // => Ok(())  // Append to file
```

**Errors:**

- **TypeError**: append_file() requires path and content strings — *Fix: Pass two String arguments*

**See also:** `write_file`, `read_file`

*Since v0.1.0*

---

#### `copy`

```ntnt
copy(from: String, to: String) -> Result<Int, String>
```

Copy a file to a new location, returning the number of bytes copied.

Copies the file at `from` to the path `to`. If the destination file already exists it is overwritten. The source must be a regular file. On success the Result contains the number of bytes written.

**Parameters:**

- `from` — The filesystem path of the source file.
- `to` — The filesystem path for the destination copy.

**Returns:** Result<Int, String> Ok with byte count copied, or Err with error message.

**Examples:**

```ntnt
copy("src.txt", "dst.txt")  // => Ok(1024)  // Copy file and get byte count
```

**Errors:**

- **TypeError**: copy() requires two string paths — *Fix: Pass two String arguments*

**See also:** `rename`, `write_file`, `read_file`

*Since v0.1.0*

---

#### `exists`

```ntnt
exists(path: String) -> Bool
```

Check whether a file or directory exists at the given path.

Returns true if a filesystem entry (file, directory, or symlink) exists at the specified path, false otherwise.

**Parameters:**

- `path` — The filesystem path to check.

**Returns:** Bool True if the path exists, false otherwise.

**Examples:**

```ntnt
exists("/tmp")  // => true  // Check path existence
```

**Errors:**

- **TypeError**: exists() requires a string path — *Fix: Pass a String argument*

**See also:** `is_file`, `is_dir`

*Since v0.1.0*

---

#### `file_size`

```ntnt
file_size(path: String) -> Result<Int, String>
```

Get the size of a file in bytes.

Returns the length in bytes of the file at the given path by reading its filesystem metadata. Fails if the path does not exist or is not accessible.

**Parameters:**

- `path` — The filesystem path to query.

**Returns:** Result<Int, String> Ok with file size in bytes, or Err with error message.

**Examples:**

```ntnt
file_size("data.txt")  // => Ok(256)  // Get file size in bytes
```

**Errors:**

- **TypeError**: file_size() requires a string path — *Fix: Pass a String argument*

**See also:** `exists`, `is_file`, `read_file`

*Since v0.1.0*

---

#### `is_dir`

```ntnt
is_dir(path: String) -> Bool
```

Check whether the path points to a directory.

Returns true only if the path exists and is a directory.

**Parameters:**

- `path` — The filesystem path to check.

**Returns:** Bool True if the path is a directory, false otherwise.

**Examples:**

```ntnt
is_dir("/tmp")  // => true  // Check if path is a directory
```

**Errors:**

- **TypeError**: is_dir() requires a string path — *Fix: Pass a String argument*

**See also:** `is_file`, `exists`

*Since v0.1.0*

---

#### `is_file`

```ntnt
is_file(path: String) -> Bool
```

Check whether the path points to a regular file.

Returns true only if the path exists and is a regular file (not a directory or symlink to a directory).

**Parameters:**

- `path` — The filesystem path to check.

**Returns:** Bool True if the path is a regular file, false otherwise.

**Examples:**

```ntnt
is_file("config.tnt")  // => true  // Check if path is a file
```

**Errors:**

- **TypeError**: is_file() requires a string path — *Fix: Pass a String argument*

**See also:** `is_dir`, `exists`

*Since v0.1.0*

---

#### `mkdir`

```ntnt
mkdir(path: String) -> Result<Unit, String>
```

Create a single directory.

Creates the directory at the given path. The parent directory must already exist. Fails if the directory already exists or if the parent is missing. Use mkdir_all to create intermediate directories automatically.

**Parameters:**

- `path` — The filesystem path for the new directory.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
mkdir("new_dir")  // => Ok(())  // Create a directory
```

**Errors:**

- **TypeError**: mkdir() requires a string path — *Fix: Pass a String argument*

**See also:** `mkdir_all`, `remove_dir`, `readdir`

*Since v0.1.0*

---

#### `mkdir_all`

```ntnt
mkdir_all(path: String) -> Result<Unit, String>
```

Create a directory and all missing parent directories.

Recursively creates directories along the given path. If the directory already exists, this is not an error. Equivalent to `mkdir -p` on Unix.

**Parameters:**

- `path` — The filesystem path for the new directory tree.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
mkdir_all("a/b/c")  // => Ok(())  // Create nested directories
```

**Errors:**

- **TypeError**: mkdir_all() requires a string path — *Fix: Pass a String argument*

**See also:** `mkdir`, `remove_dir_all`, `readdir`

*Since v0.1.0*

---

#### `read_bytes`

```ntnt
read_bytes(path: String) -> Result<Array<Int>, String>
```

Read the entire contents of a file as raw bytes.

Opens the file at the given path and returns its contents as an array of integers (0-255), one per byte. Useful for binary files that are not valid UTF-8.

**Parameters:**

- `path` — The filesystem path to the file to read.

**Returns:** Result<Array<Int>, String> Ok with array of byte values, or Err with error message.

**Examples:**

```ntnt
read_bytes("data.bin")  // => Ok([72, 101, 108, 108, 111])  // Read binary file as byte array
```

**Errors:**

- **TypeError**: read_bytes() requires a string path — *Fix: Pass a String argument*

**See also:** `read_file`, `write_file`, `file_size`

*Since v0.1.0*

---

#### `read_file`

```ntnt
read_file(path: String) -> Result<String, String>
```

Read the entire contents of a file as a UTF-8 string.

Opens the file at the given path and returns its contents. The file must be valid UTF-8. Returns a Result wrapping the file content on success or an error message on failure.

**Parameters:**

- `path` — The filesystem path to the file to read.

**Returns:** Result<String, String> Ok with file contents, or Err with error message.

**Examples:**

```ntnt
read_file("hello.txt")  // => Ok("Hello, world!")  // Read file contents
```

**Errors:**

- **TypeError**: read_file() requires a string path — *Fix: Pass a String argument*

**See also:** `read_bytes`, `write_file`, `exists`

*Since v0.1.0*

---

#### `readdir`

```ntnt
readdir(path: String) -> Result<Array<String>, String>
```

List the entries of a directory.

Returns an array of full path strings for every entry in the directory. The order is filesystem-dependent and not guaranteed to be sorted. Entries that cannot be read are silently skipped.

**Parameters:**

- `path` — The filesystem path to the directory to list.

**Returns:** Result<Array<String>, String> Ok with array of entry paths, or Err with error message.

**Examples:**

```ntnt
readdir(".")  // => Ok(["./file.tnt", "./lib"])  // List directory entries
```

**Errors:**

- **TypeError**: readdir() requires a string path — *Fix: Pass a String argument*

**See also:** `mkdir`, `is_dir`, `exists`

*Since v0.1.0*

---

#### `remove`

```ntnt
remove(path: String) -> Result<Unit, String>
```

Remove a file from the filesystem.

Deletes the file at the given path. Fails if the path does not exist or points to a directory. Use remove_dir or remove_dir_all for directories.

**Parameters:**

- `path` — The filesystem path to the file to remove.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
remove("temp.txt")  // => Ok(())  // Delete a file
```

**Errors:**

- **TypeError**: remove() requires a string path — *Fix: Pass a String argument*

**See also:** `remove_dir`, `remove_dir_all`, `exists`

*Since v0.1.0*

---

#### `remove_dir`

```ntnt
remove_dir(path: String) -> Result<Unit, String>
```

Remove an empty directory.

Deletes the directory at the given path. The directory must be empty; if it contains any entries the operation will fail. Use remove_dir_all to recursively remove a directory and its contents.

**Parameters:**

- `path` — The filesystem path to the empty directory to remove.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
remove_dir("empty_dir")  // => Ok(())  // Remove an empty directory
```

**Errors:**

- **TypeError**: remove_dir() requires a string path — *Fix: Pass a String argument*

**See also:** `remove_dir_all`, `remove`, `mkdir`

*Since v0.1.0*

---

#### `remove_dir_all`

```ntnt
remove_dir_all(path: String) -> Result<Unit, String>
```

Recursively remove a directory and all of its contents.

Deletes the directory at the given path along with every file and subdirectory it contains. Use with caution as this operation is irreversible.

**Parameters:**

- `path` — The filesystem path to the directory to remove recursively.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
remove_dir_all("build")  // => Ok(())  // Remove directory tree
```

**Errors:**

- **TypeError**: remove_dir_all() requires a string path — *Fix: Pass a String argument*

**See also:** `remove_dir`, `remove`, `mkdir_all`

*Since v0.1.0*

---

#### `rename`

```ntnt
rename(from: String, to: String) -> Result<Unit, String>
```

Rename or move a file or directory.

Renames the filesystem entry at `from` to the path `to`. This can also be used to move entries across directories on the same filesystem. Fails if the source does not exist or the destination's parent directory is missing.

**Parameters:**

- `from` — The current filesystem path.
- `to` — The desired new filesystem path.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
rename("old.txt", "new.txt")  // => Ok(())  // Rename a file
```

**Errors:**

- **TypeError**: rename() requires two string paths — *Fix: Pass two String arguments*

**See also:** `copy`, `remove`, `exists`

*Since v0.1.0*

---

#### `write_file`

```ntnt
write_file(path: String, content: String) -> Result<Unit, String>
```

Write a string to a file, creating or overwriting it.

Writes the given content to the file at path. If the file already exists it is truncated and overwritten. If it does not exist it is created. Parent directories must already exist.

**Parameters:**

- `path` — The filesystem path to write to.
- `content` — The string content to write.

**Returns:** Result<Unit, String> Ok on success, or Err with error message.

**Examples:**

```ntnt
write_file("out.txt", "hello")  // => Ok(())  // Write string to file
```

**Errors:**

- **TypeError**: write_file() requires path and content strings — *Fix: Pass two String arguments*

**See also:** `read_file`, `append_file`, `copy`

*Since v0.1.0*

---

## std/http

HTTP client for making requests to external services

```ntnt
import { fetch, download, Cache } from "std/http"
```

### Functions

| Function | Description |
|----------|-------------|
| [`Cache`](#cache) | Create a response cache with a time-to-live (TTL) in seconds. |
| [`cache_clear`](#cacheclear) | Remove all cached responses from a cache. |
| [`cache_delete`](#cachedelete) | Remove a cached response for a specific URL. |
| [`cache_fetch`](#cachefetch) | Fetch a URL using a cache, returning a cached response if available. |
| [`download`](#download) | Download a file from a URL and save it to disk. |
| [`fetch`](#fetch) | Make an HTTP request to a URL. |

#### `Cache`

```ntnt
Cache(ttl_seconds: Int) -> Map
```

Create a response cache with a time-to-live (TTL) in seconds.

Returns a cache object (Map) that can be used with cache_fetch, cache_delete, and cache_clear to cache HTTP responses. Cached entries automatically expire after the specified TTL. The cache object contains an internal _cache_id field.

**Parameters:**

- `ttl_seconds` — The default time-to-live for cached entries, in seconds

**Returns:** Map containing a _cache_id field for use with cache helper functions

**Examples:**

```ntnt
Cache(300)  // => {_cache_id: 1}  // Create a cache with 5-minute TTL
```

**Errors:**

- **TypeError**: Cache() requires TTL in seconds (integer) — *Fix: Pass an Int value for the TTL*

**See also:** `cache_fetch`, `cache_delete`, `cache_clear`

*Since v0.1.0*

---

#### `cache_clear`

```ntnt
cache_clear(cache_obj: Map) -> Unit
```

Remove all cached responses from a cache.

Evicts every entry from the specified cache object, regardless of TTL. This is the internal function backing cache.clear() method calls.

**Parameters:**

- `cache_obj` — A cache object created by Cache()

**Returns:** Unit

**Examples:**

```ntnt
cache_clear(my_cache)  // => ()  // Clear all cached responses
```

**Errors:**

- **TypeError**: Invalid cache object — *Fix: Pass a cache object created by Cache()*
- **TypeError**: Expected cache object — *Fix: First argument must be a Map with _cache_id*

**See also:** `Cache`, `cache_fetch`, `cache_delete`

*Since v0.1.0*

---

#### `cache_delete`

```ntnt
cache_delete(cache_obj: Map, url: String) -> Unit
```

Remove a cached response for a specific URL.

Evicts the cached entry for the given URL from the cache, if present. This is the internal function backing cache.delete() method calls.

**Parameters:**

- `cache_obj` — A cache object created by Cache()
- `url` — The URL whose cached response should be removed

**Returns:** Unit

**Examples:**

```ntnt
cache_delete(my_cache, "https://api.example.com/data")  // => ()  // Invalidate a cached URL
```

**Errors:**

- **TypeError**: Invalid cache object — *Fix: Pass a cache object created by Cache()*
- **TypeError**: Expected cache object — *Fix: First argument must be a Map with _cache_id*
- **TypeError**: cache.delete() requires URL string — *Fix: Pass a String URL as the second argument*

**See also:** `Cache`, `cache_fetch`, `cache_clear`

*Since v0.1.0*

---

#### `cache_fetch`

```ntnt
cache_fetch(cache_obj: Map, url_or_options: String | Map) -> Result<Response, String>
```

Fetch a URL using a cache, returning a cached response if available.

Checks the cache for a previously stored response matching the URL. On a cache miss, performs the HTTP request via fetch(), stores the successful response in the cache, and returns it. This is the internal function backing cache.fetch() method calls.

**Parameters:**

- `cache_obj` — A cache object created by Cache()
- `url_or_options` — A URL string or options Map (must include 'url' key)

**Returns:** Result<Response, String> with the HTTP response (from cache or network)

**Examples:**

```ntnt
cache_fetch(my_cache, "https://api.example.com/data")  // => Ok({status: 200, ...})  // Fetch with caching
```

**Errors:**

- **TypeError**: Invalid cache object — *Fix: Pass a cache object created by Cache()*
- **TypeError**: Expected cache object — *Fix: First argument must be a Map with _cache_id*
- **TypeError**: Options must include 'url' — *Fix: Include 'url' key in the options map*
- **TypeError**: cache.fetch() requires URL string or options map — *Fix: Pass a String URL or a Map with request options*

**See also:** `Cache`, `cache_delete`, `cache_clear`, `fetch`

*Since v0.1.0*

---

#### `download`

```ntnt
download(url: String, file_path: String) -> Result<Map, String>
```

Download a file from a URL and save it to disk.

Fetches the resource at the given URL and writes the response bytes to the specified file path. Parent directories are created automatically if they do not exist. Returns a map with status, path, and size on success.

**Parameters:**

- `url` — The URL of the file to download
- `file_path` — The local file path to save the downloaded content

**Returns:** Result<Map{status: Int, path: String, size: Int}, String> on success; Err with message on failure

**Examples:**

```ntnt
download("https://example.com/file.zip", "./file.zip")  // => Ok({status: 200, path: "./file.zip", size: 1024})  // Download a file
```

**Errors:**

- **TypeError**: download() requires URL string and file path string — *Fix: Pass two String arguments: URL and file path*
- **RuntimeError**: Failed to create directory: ... — *Fix: Ensure the parent directory path is valid and writable*
- **RuntimeError**: Failed to create file: ... — *Fix: Ensure the file path is valid and writable*
- **RuntimeError**: HTTP error: status ... — *Fix: Check the URL and server availability*

**See also:** `fetch`

*Since v0.1.0*

---

#### `fetch`

```ntnt
fetch(url_or_options: String | Map, options?: Map) -> Result<Response, String>
```

Make an HTTP request to a URL.

Accepts one or two arguments: - One argument: a URL string for a simple GET request, or an options map   with full control over method, headers, body, authentication, cookies, and timeout. - Two arguments: a URL string and an options map. The URL is merged into   the options map automatically. Options map keys: url (set automatically in 2-arg form), method, headers, body, json, form, auth, cookies, timeout.

**Parameters:**

- `url_or_options` — A URL string for GET, or a Map with request options
- `options` — (optional) A Map with request options when first argument is a URL string

**Returns:** Result<Response, String> where Response is a Map with status, status_text, headers, body, ok, url, redirected, and cookies fields

**Examples:**

```ntnt
fetch("https://api.example.com/data")  // => Ok({status: 200, body: "...", ...})  // Simple GET request
// POST with JSON body (1-arg form)
let opts = map {
  "url": "https://api.example.com",
  "method": "POST",
  "json": map { "key": "value" }
}
fetch(opts)
// => Ok({status: 201, ...})
// POST with JSON body (2-arg form)
fetch("https://api.example.com", map {
  "method": "POST",
  "json": map { "key": "value" }
})
// => Ok({status: 201, ...})
```

**Errors:**

- **TypeError**: fetch() requires a URL string or options map — *Fix: Pass a String URL or a Map with request options*
- **TypeError**: fetch() requires 'url' option — *Fix: Include 'url' key in the options map*
- **RuntimeError**: Unsupported HTTP method: ... — *Fix: Use GET, POST, PUT, DELETE, PATCH, or HEAD*

**See also:** `download`, `cache_fetch`

*Since v0.1.0*

---

## std/http/server

HTTP response builders for server route handlers

```ntnt
import { text, html, json } from "std/http/server"
```

### Functions

| Function | Description |
|----------|-------------|
| [`delete_cookie`](#deletecookie) | Build a Set-Cookie header value that deletes a cookie. |
| [`error`](#error) | Create an HTTP 500 Internal Server Error response. |
| [`get_cookie`](#getcookie) | Get a specific cookie value from a request. |
| [`get_cookies`](#getcookies) | Get all cookies from a request as a map. |
| [`html`](#html) | Create an HTML HTTP response. |
| [`json`](#json) | Create a JSON HTTP response. |
| [`not_found`](#notfound) | Create an HTTP 404 Not Found response. |
| [`on_error`](#onerror) | Register a global error handler for HTTP route handlers. |
| [`parse_form`](#parseform) | Parse a request body (or raw string) as URL-encoded form data. |
| [`parse_json`](#parsejson) | Parse a request body (or raw string) as JSON. |
| [`parse_multipart`](#parsemultipart) | Parse a multipart/form-data request body. |
| [`redirect`](#redirect) | Create an HTTP 302 redirect response. |
| [`redirect_safe`](#redirectsafe) | Create a safe HTTP 302 redirect response that prevents open redirect attacks. |
| [`response`](#response) | Create a fully custom HTTP response. |
| [`save_upload`](#saveupload) | Save an uploaded file to disk. |
| [`set_cookie`](#setcookie) | Build a Set-Cookie header value string. |
| [`static_file`](#staticfile) | Create a cacheable HTTP response for static assets. |
| [`status`](#status) | Create a plain-text HTTP response with an explicit status code. |
| [`text`](#text) | Create a plain-text HTTP response with status 200. |
| [`with_cookie`](#withcookie) | Add a Set-Cookie header to a response. |

#### `delete_cookie`

```ntnt
delete_cookie(name: String, options?: Map) -> String
```

Build a Set-Cookie header value that deletes a cookie.

Returns a Set-Cookie header string with Max-Age=0 to instruct the browser to delete the cookie. The options map can specify `path` and `domain` to ensure the correct cookie is deleted.

**Parameters:**

- `name` — The name of the cookie to delete.
- `options` — Optional map with `path` and `domain` to match the original cookie.

**Returns:** A Set-Cookie header value string that deletes the cookie.

**Examples:**

```ntnt
delete_cookie("session")  // => "session=; Path=/; Max-Age=0"  // Delete cookie
```

**Errors:**

- **TypeError**: delete_cookie() requires 1 or 2 arguments — *Fix: Pass cookie name and optional options*

**See also:** `set_cookie`, `with_cookie`

*Since v0.3.11*

---

#### `error`

```ntnt
error(message: String) -> Response
```

Create an HTTP 500 Internal Server Error response.

Returns a Response map with status 500, Content-Type `text/plain; charset=utf-8`, and the provided message as the body.

**Parameters:**

- `message` — The error message to send as the response body.

**Returns:** A Response map with status 500, text/plain content-type, and the error message body.

**Examples:**

```ntnt
error("Something went wrong")  // => Response { status: 500, body: "Something went wrong" }  // 500 error response
```

**Errors:**

- **TypeError**: error() requires a string — *Fix: Pass a String message as the argument*

**See also:** `not_found`, `status`, `text`, `html`, `json`, `response`

*Since v0.1.0*

---

#### `get_cookie`

```ntnt
get_cookie(req: Request, name: String) -> Option<String>
```

Get a specific cookie value from a request.

Parses the request's Cookie header and returns the value of the named cookie wrapped in Some, or None if the cookie is not present.

**Parameters:**

- `req` — The Request map containing headers.
- `name` — The name of the cookie to retrieve.

**Returns:** Some(value) if the cookie exists, None otherwise.

**Examples:**

```ntnt
get_cookie(req, "session")  // => Some("abc123")  // Get existing cookie
get_cookie(req, "missing")  // => None  // Cookie not found
```

**Errors:**

- **TypeError**: get_cookie() requires a request map and cookie name — *Fix: Pass a Request and String*

**See also:** `get_cookies`, `set_cookie`, `with_cookie`

*Since v0.3.11*

---

#### `get_cookies`

```ntnt
get_cookies(req: Request) -> Map<String, String>
```

Get all cookies from a request as a map.

Parses the request's Cookie header and returns all cookie name-value pairs as a Map. Returns an empty map if no cookies are present.

**Parameters:**

- `req` — The Request map containing headers.

**Returns:** A Map<String, String> of cookie names to values.

**Examples:**

```ntnt
get_cookies(req)  // => map { "session": "abc", "theme": "dark" }  // All cookies
```

**Errors:**

- **TypeError**: get_cookies() requires a request map — *Fix: Pass a Request map*

**See also:** `get_cookie`, `set_cookie`, `with_cookie`

*Since v0.3.11*

---

#### `html`

```ntnt
html(body: String, status_code?: Int) -> Response
```

Create an HTML HTTP response.

Returns a Response map with Content-Type `text/html; charset=utf-8`. Accepts an optional second argument to override the default 200 status code. Includes cache-control and pragma headers to prevent browser caching of dynamic HTML content.

**Parameters:**

- `body` — The HTML string to send as the response body.
- `status_code` — Optional HTTP status code (defaults to 200).

**Returns:** A Response map with the given status, text/html content-type, and no-cache headers.

**Examples:**

```ntnt
html("<h1>Hello</h1>")  // => Response { status: 200, body: "<h1>Hello</h1>" }  // HTML response
html("<h1>Not Found</h1>", 404)  // => Response { status: 404 }  // HTML with custom status
html("<h1>Hi</h1>", 200, map { "x-custom": "value" })  // HTML with custom headers
```

**Errors:**

- **TypeError**: html() requires 1 to 3 arguments (body, optional status_code, optional headers) — *Fix: Pass 1 to 3 arguments*
- **TypeError**: html() body must be a string — *Fix: Ensure the first argument is a String*
- **TypeError**: html() status code must be an integer — *Fix: Pass an Int as the second argument*
- **TypeError**: html() headers must be a map — *Fix: Pass a Map as the third argument*

**See also:** `text`, `json`, `status`, `redirect`, `response`

*Since v0.1.0*

---

#### `json`

```ntnt
json(data: Any, status_code?: Int) -> Response
```

Create a JSON HTTP response.

Serializes the given value (typically a Map or Array) to a JSON string and returns a Response with Content-Type `application/json`. Accepts an optional second argument to override the default 200 status code. Includes cache-control headers to prevent browser caching of API responses.

**Parameters:**

- `data` — The value to serialize as JSON (Map, Array, String, Int, Float, Bool, or Unit).
- `status_code` — Optional HTTP status code (defaults to 200).

**Returns:** A Response map with the given status, application/json content-type, and no-cache headers.

**Examples:**

```ntnt
json(map { "ok": true })  // => Response { status: 200, body: "{\"ok\":true}" }  // JSON response
json(map { "error": "not found" }, 404)  // => Response { status: 404 }  // JSON with custom status
```

**Errors:**

- **TypeError**: json() requires 1 or 2 arguments (data, optional status_code) — *Fix: Pass 1 or 2 arguments*
- **TypeError**: json() status code must be an integer — *Fix: Pass an Int as the second argument*

**See also:** `text`, `html`, `status`, `redirect`, `response`, `parse_json`

*Since v0.1.0*

---

#### `not_found`

```ntnt
not_found() -> Response
```

Create an HTTP 404 Not Found response.

Returns a Response map with status 404, Content-Type `text/plain; charset=utf-8`, and body "Not Found". Takes no arguments.

**Returns:** A Response map with status 404, text/plain content-type, and body "Not Found".

**Examples:**

```ntnt
not_found()  // => Response { status: 404, body: "Not Found" }  // 404 response
```

**See also:** `error`, `status`, `text`, `html`, `json`, `response`

*Since v0.1.0*

---

#### `on_error`

```ntnt
on_error(handler: fn(req: Request, error: String) -> Response) -> Unit
```

Register a global error handler for HTTP route handlers.

When a route handler throws an unhandled error, the registered callback is called with the request and error message instead of returning the default 500 error page. If the callback itself errors, falls back to the default error response.

**Parameters:**

- `handler` — A function that receives (request, error_message) and returns a Response.

**Returns:** Unit

**Examples:**

```ntnt
on_error(fn(req, err) { html(template("views/error.html", map { "message": "Something went wrong" })) })  // Custom error page (use templates to avoid XSS)
```

**Gotchas:**

- The handler is called on the interpreter thread; if it errors, the default error page is shown

**See also:** `on_shutdown`

*Since v0.4.0*

---

#### `parse_form`

```ntnt
parse_form(req: Request | String) -> Map<String, String>
```

Parse a request body (or raw string) as URL-encoded form data.

Accepts either a Request map (extracts the `body` field) or a plain String. Splits the body on `&` and `=` to produce key-value pairs. Keys and values are URL-decoded automatically. Keys without a value are mapped to an empty string.

**Parameters:**

- `req` — A Request map with a `body` field, or a raw URL-encoded form string.

**Returns:** A Map<String, String> of decoded form field names to values.

**Examples:**

```ntnt
parse_form("name=Alice&age=30")  // => map { "name": "Alice", "age": "30" }  // Parse form data
parse_form("q=hello+world")  // => map { "q": "hello world" }  // URL-decoded values
```

**Errors:**

- **TypeError**: parse_form() requires a request with body — *Fix: Pass a Request map that contains a body field*
- **TypeError**: parse_form() requires a request map or body string — *Fix: Pass a Request map or a String*

**See also:** `parse_json`, `json`

*Since v0.1.0*

---

#### `parse_json`

```ntnt
parse_json(req: Request | String) -> Result<Map<String, Any>, String>
```

Parse a request body (or raw string) as JSON.

Accepts either a Request map (extracts the `body` field) or a plain String. Returns a Result enum: `Ok(value)` on success with the parsed data, or `Err(message)` if the JSON is malformed. JSON null values become None.

**Parameters:**

- `req` — A Request map with a `body` field, or a raw JSON string.

**Returns:** Result<Map<String, Any>, String> -- Ok with parsed value, or Err with parse error message.

**Examples:**

```ntnt
parse_json("{\"key\": \"value\"}")  // => Ok(map { "key": "value" })  // Parse JSON string
parse_json("not json")  // => Err("expected ...")  // Returns Err on invalid JSON
```

**Errors:**

- **TypeError**: parse_json() requires a request with body — *Fix: Pass a Request map that contains a body field*
- **TypeError**: parse_json() requires a request map or body string — *Fix: Pass a Request map or a String*

**Gotchas:**

- JSON null values are parsed as None (not Unit), matching std/json behavior

**See also:** `json`, `parse_form`

*Since v0.1.0*

---

#### `parse_multipart`

```ntnt
parse_multipart(req: Request) -> Result<Map<String, Any>, String>
```

Parse a multipart/form-data request body.

Extracts fields and files from a multipart request. Text fields are returned as String values. File fields are returned as Maps with: `filename` (String), `content_type` (String), `size` (Int), and `data` (String - may be lossy for binary files).

Note: Binary file data passes through String conversion and may be lossy. For binary files, use `save_upload()` to write directly to disk.

**Parameters:**

- `req` — The Request map with Content-Type header and body.

**Returns:** Ok(Map) with field names as keys, or Err(String) on parse failure.

**Examples:**

```ntnt
let fields = parse_multipart(req)?
let name = fields["name"]
let file = fields["document"]
print("Uploaded: #{file[\"filename\"]}, #{file[\"size\"]} bytes")
```

**Errors:**

- **TypeError**: parse_multipart() requires a request map — *Fix: Pass a Request map*
- **ParseError**: Invalid multipart boundary — *Fix: Ensure Content-Type header includes boundary*

**See also:** `save_upload`, `parse_form`

*Since v0.3.11*

---

#### `redirect`

```ntnt
redirect(url: String) -> Response
```

Create an HTTP 302 redirect response.

Returns a Response map with status 302 and a `Location` header set to the provided URL. The body is empty.

WARNING: This function does NOT validate the URL. If user input flows into this function, attackers can redirect users to malicious sites (open redirect). Use `redirect_safe()` instead when the URL comes from user input.

**Parameters:**

- `url` — The URL to redirect the client to (absolute or relative path).

**Returns:** A Response map with status 302, a Location header, and an empty body.

**Examples:**

```ntnt
redirect("/dashboard")  // => Response { status: 302, headers: { "location": "/dashboard" } }  // Redirect response
```

**Errors:**

- **TypeError**: redirect() requires a URL string — *Fix: Pass a String URL as the argument*

**Gotchas:**

- Does not validate URLs - use redirect_safe() for user-provided URLs

**See also:** `redirect_safe`, `text`, `html`, `json`, `status`, `response`

*Since v0.1.0*

---

#### `redirect_safe`

```ntnt
redirect_safe(url: String, fallback?: String) -> Response
```

Create a safe HTTP 302 redirect response that prevents open redirect attacks.

Only allows redirects to relative paths (e.g., /dashboard, ./page, ../back). Rejects absolute URLs, protocol-relative URLs (//evil.com), and dangerous schemes (javascript:, data:, etc.). If the URL is unsafe, redirects to the fallback URL (default: "/").

Use this function instead of `redirect()` when the URL comes from user input (e.g., query parameters, form fields, database values).

**Parameters:**

- `url` — The URL to redirect to (must be a relative path for safety).
- `fallback` — Optional fallback URL if the provided URL is unsafe (default: "/").

**Returns:** A Response map with status 302, a Location header, and an empty body.

**Examples:**

```ntnt
redirect_safe("/dashboard")  // => Response { status: 302, headers: { "location": "/dashboard" } }  // Safe relative redirect
redirect_safe("https://evil.com")  // => Response { status: 302, headers: { "location": "/" } }  // Unsafe URL redirects to fallback
redirect_safe("//evil.com/path", "/home")  // => Response { status: 302, headers: { "location": "/home" } }  // Protocol-relative URL rejected
```

**Errors:**

- **TypeError**: redirect_safe() requires a URL string — *Fix: Pass a String URL as the first argument*

**See also:** `redirect`, `text`, `html`, `json`, `status`, `response`

*Since v0.3.11*

---

#### `response`

```ntnt
response(status: Int, headers: Map<String, String>, body: String) -> Response
```

Create a fully custom HTTP response.

Provides complete control over status code, headers, and body. Header keys are lowercased automatically. Use this when the convenience builders (text, html, json) do not offer enough flexibility.

**Parameters:**

- `status` — The HTTP status code.
- `headers` — A Map of header names to header values.
- `body` — The response body string.

**Returns:** A Response map with the given status, headers (lowercased keys), and body.

**Examples:**

```ntnt
response(200, map { "X-Custom": "value" }, "OK")  // => Response { status: 200 }  // Custom response
```

**Errors:**

- **TypeError**: response() status must be an integer — *Fix: Pass an Int as the first argument*
- **TypeError**: response() headers must be a map — *Fix: Pass a Map as the second argument*
- **TypeError**: response() body must be a string — *Fix: Pass a String as the third argument*

**See also:** `text`, `html`, `json`, `status`, `redirect`, `static_file`

*Since v0.1.0*

---

#### `save_upload`

```ntnt
save_upload(file_field: Map, path: String) -> Result<Int, String>
```

Save an uploaded file to disk.

Writes the file data from a parsed multipart field to the specified path. Returns the number of bytes written on success. Parent directories are created automatically if they don't exist.

Security: Paths are validated to prevent directory traversal attacks. Relative paths are resolved from the current working directory. Paths containing `..` are rejected for security.

**Parameters:**

- `file_field` — The file field Map from parse_multipart() with a `data` key.
- `path` — The filesystem path to save the file to (relative or absolute).

**Returns:** Ok(Int) bytes written, or Err(String) on failure.

**Examples:**

```ntnt
save_upload(fields["photo"], "uploads/photo.jpg")  // => Ok(1024)  // Save to relative path
```

**Errors:**

- **TypeError**: save_upload() requires a file map and path — *Fix: Pass a file field and String path*
- **SecurityError**: Path traversal not allowed — *Fix: Use a path without '..' components*

**See also:** `parse_multipart`

*Since v0.3.11*

---

#### `set_cookie`

```ntnt
set_cookie(name: String, value: String, options?: Map) -> String
```

Build a Set-Cookie header value string.

Constructs a properly formatted Set-Cookie header value with the given name, value, and optional attributes. The returned string can be used as a header value directly or with the `with_cookie` helper.

Options map supports: - `path` (String): Cookie path scope (default: "/") - `domain` (String): Cookie domain scope - `max_age` (Int): Max age in seconds - `secure` (Bool): Only send over HTTPS - `http_only` (Bool): Not accessible via JavaScript - `same_site` (String): "Strict", "Lax", or "None" - `expires` (String): Expiration date (RFC 7231 format) - `partitioned` (Bool): CHIPS partitioned cookie

**Parameters:**

- `name` — The cookie name.
- `value` — The cookie value.
- `options` — Optional map of cookie attributes.

**Returns:** A Set-Cookie header value string.

**Examples:**

```ntnt
set_cookie("session", "abc123")  // => "session=abc123; Path=/"  // Basic cookie
set_cookie("token", "xyz", map { "http_only": true, "secure": true })  // => "token=xyz; Path=/; HttpOnly; Secure"  // Secure cookie
```

**Errors:**

- **TypeError**: set_cookie() requires 2 or 3 arguments — *Fix: Pass name, value, and optional options map*

**See also:** `get_cookie`, `get_cookies`, `delete_cookie`, `with_cookie`

*Since v0.3.11*

---

#### `static_file`

```ntnt
static_file(content: String, content_type: String, max_age?: Int) -> Response
```

Create a cacheable HTTP response for static assets.

Returns a Response map with status 200, the specified Content-Type, and a `Cache-Control: public, max-age=<seconds>` header. The optional `max_age` parameter controls how long browsers cache the asset (defaults to 3600 seconds / 1 hour).

**Parameters:**

- `content` — The file content as a string.
- `content_type` — The MIME type for the Content-Type header (e.g., "text/css", "image/png").
- `max_age` — Optional cache duration in seconds (defaults to 3600).

**Returns:** A Response map with status 200, the given content-type, and public cache-control headers.

**Examples:**

```ntnt
static_file(css, "text/css")  // => Response { status: 200 }  // Static CSS with default 1h cache
static_file(js, "application/javascript", 86400)  // => Response { status: 200 }  // Static JS with 24h cache
```

**Errors:**

- **TypeError**: static_file() requires 2-3 arguments (content, content_type, optional max_age) — *Fix: Pass 2 or 3 arguments*
- **TypeError**: static_file() content must be a string — *Fix: Ensure the first argument is a String*
- **TypeError**: static_file() content_type must be a string — *Fix: Ensure the second argument is a String*
- **TypeError**: static_file() max_age must be an integer — *Fix: Pass an Int as the third argument*

**See also:** `text`, `html`, `json`, `response`

*Since v0.1.0*

---

#### `status`

```ntnt
status(code: Int, body: String) -> Response
```

Create a plain-text HTTP response with an explicit status code.

Returns a Response map with the specified status code, Content-Type `text/plain; charset=utf-8`, and the provided body string.

**Parameters:**

- `code` — The HTTP status code (e.g., 201, 400, 503).
- `body` — The plain-text body string.

**Returns:** A Response map with the given status and text/plain content-type.

**Examples:**

```ntnt
status(201, "Created")  // => Response { status: 201, body: "Created" }  // Custom status response
```

**Errors:**

- **TypeError**: status() requires int and string — *Fix: Pass an Int status code and a String body*

**See also:** `text`, `html`, `json`, `redirect`, `error`, `response`

*Since v0.1.0*

---

#### `text`

```ntnt
text(body: String) -> Response
```

Create a plain-text HTTP response with status 200.

Wraps the given string in a Response map with Content-Type set to `text/plain; charset=utf-8` and cache-control headers that prevent browser caching of dynamic content.

**Parameters:**

- `body` — The plain-text string to send as the response body.

**Returns:** A Response map with status 200, text/plain content-type, and no-cache headers.

**Examples:**

```ntnt
text("Hello, World!")  // => Response { status: 200, body: "Hello, World!" }  // Plain text response
```

**Errors:**

- **TypeError**: text() requires a string — *Fix: Pass a String value as the argument*

**See also:** `html`, `json`, `status`, `redirect`, `response`

*Since v0.1.0*

---

#### `with_cookie`

```ntnt
with_cookie(response: Response, name: String, value: String, options?: Map) -> Response
```

Add a Set-Cookie header to a response.

Returns a new Response with the Set-Cookie header added. If the response already has Set-Cookie headers, the new cookie is appended (using an array for multiple Set-Cookie headers). This is the ergonomic way to set cookies without manually building headers.

**Parameters:**

- `response` — The Response map to add the cookie to.
- `name` — The cookie name.
- `value` — The cookie value.
- `options` — Optional map of cookie attributes (same as set_cookie).

**Returns:** A new Response map with the Set-Cookie header added.

**Examples:**

```ntnt
with_cookie(json(data), "session", "abc123")  // Add cookie to JSON response
with_cookie(html(page), "theme", "dark", map { "max_age": 86400 })  // Cookie with options
```

**Errors:**

- **TypeError**: with_cookie() requires 3 or 4 arguments — *Fix: Pass response, name, value, and optional options*

**See also:** `set_cookie`, `delete_cookie`, `get_cookie`

*Since v0.3.11*

---

## std/jobs

Background job queue with persistent storage

```ntnt
import { configure_queue, enqueue, job_status } from "std/jobs"
```

### Functions

| Function | Description |
|----------|-------------|
| [`cancel_job`](#canceljob) | Cancel a pending job by its ID. |
| [`configure_queue`](#configurequeue) | Configure the job queue storage backend. |
| [`enqueue`](#enqueue) | Enqueue a background job for processing. |
| [`job_status`](#jobstatus) | Get the current status and data for a job by its ID. |

#### `cancel_job`

```ntnt
cancel_job(job_id: String) -> Result<Bool, String>
```

Cancel a pending job by its ID.

Sets the job status to "cancelled" and removes it from the pending queue. Returns true if the job was cancelled, false if it was not in a cancellable state.

**Parameters:**

- `job_id` — The job ID returned by enqueue()

**Returns:** Result containing true if cancelled, false if not cancellable

**Examples:**

```ntnt
cancel_job("abc-123")  // Cancel a pending job
```

---

#### `configure_queue`

```ntnt
configure_queue(opts: Map) -> Result<Unit, String>
```

Configure the job queue storage backend.

Pass a map with a "store" key to set the KV backend for job storage. If never called, enqueue() auto-initializes with "sqlite:./jobs.db".

**Parameters:**

- `opts` — Configuration map with optional "store" key (e.g., "redis://localhost:6379" or "sqlite:./jobs.db")

**Returns:** Result indicating success or error

**Examples:**

```ntnt
configure_queue(map { "store": "sqlite:./jobs.db" })  // Use SQLite for job storage
configure_queue(map { "store": "redis://localhost:6379" })  // Use Redis for job storage
```

---

#### `enqueue`

```ntnt
enqueue(job_name: String, args: Map) -> Result<String, String>
```

Enqueue a background job for processing.

Looks up the job name in the registry, generates a unique ID, serializes the job data, and writes it to the configured KV store. Returns the job ID. If configure_queue() hasn't been called, auto-initializes with SQLite.

**Parameters:**

- `job_name` — The registered job name (e.g., "SendEmail")
- `args` — A map of arguments to pass to the job's perform block

**Returns:** Result containing the job ID string or an error

**Examples:**

```ntnt
enqueue("SendEmail", map { "to": "alice@example.com" })  // Enqueue an email job
enqueue("ProcessPayment", map { "amount": 100 })  // Enqueue a payment job
```

---

#### `job_status`

```ntnt
job_status(job_id: String) -> Result<Map, String>
```

Get the current status and data for a job by its ID.

Returns the full job data map including status, type, queue, payload, attempts, and timestamps. Returns an error if the job ID is not found.

**Parameters:**

- `job_id` — The job ID returned by enqueue()

**Returns:** Result containing the job data map or an error

**Examples:**

```ntnt
job_status("abc-123")  // Check job status
```

---

## std/json

JSON parsing and serialization

```ntnt
import { parse_json, stringify, stringify_pretty } from "std/json"
```

### Functions

| Function | Description |
|----------|-------------|
| [`parse_json`](#parsejson) | Parses a JSON string into a value. |
| [`stringify`](#stringify) | Converts a value to a compact JSON string. |
| [`stringify_pretty`](#stringifypretty) | Converts a value to a pretty-printed JSON string with indentation. |

#### `parse_json`

```ntnt
parse_json(json_str: String) -> Result<Any, String>
```

Parses a JSON string into a value.

Returns Ok with the parsed value on success, or Err with a descriptive parse error message. Supports all JSON types: objects become Maps, arrays become Arrays, numbers become Int or Float, and null becomes None.

**Parameters:**

- `json_str` — The JSON string to parse

**Returns:** Result containing the parsed value or an error message

**Examples:**

```ntnt
parse_json("{\"key\": \"value\"}")  // => Ok(map { "key": "value" })  // Parse JSON object
parse_json("null")  // => Ok(None)  // JSON null becomes None
```

**Errors:**

- **TypeError**: parse_json() requires a JSON string — *Fix: Pass a string argument*

**Gotchas:**

- JSON null is parsed as None (not Unit), enabling round-trip with stringify(None) → "null"

**See also:** `stringify`, `stringify_pretty`

*Since v0.1.0*

---

#### `stringify`

```ntnt
stringify(value: Any) -> String
```

Converts a value to a compact JSON string.

Maps, arrays, strings, numbers, booleans, and Unit are serialized to their JSON equivalents. Structs are serialized as JSON objects. Option values are unwrapped: None becomes null, Some(v) becomes v.

**Parameters:**

- `value` — The value to serialize

**Returns:** Compact JSON string with no extra whitespace

**Examples:**

```ntnt
stringify(map { "key": "value" })  // => "{\"key\":\"value\"}"  // Compact JSON
stringify(None)  // => "null"  // None serializes to null
stringify(Some(42))  // => "42"  // Some unwraps to inner value
```

**Gotchas:**

- Both None and Unit serialize to JSON null

**See also:** `stringify_pretty`, `parse_json`

*Since v0.1.0*

---

#### `stringify_pretty`

```ntnt
stringify_pretty(value: Any) -> String
```

Converts a value to a pretty-printed JSON string with indentation.

Behaves identically to stringify() but formats the output with newlines and 2-space indentation for readability. None becomes null, Some(v) becomes v.

**Parameters:**

- `value` — The value to serialize

**Returns:** Indented JSON string for human readability

**Examples:**

```ntnt
stringify_pretty(map { "a": 1 })  // Pretty-printed with newlines and indentation
```

**See also:** `stringify`, `parse_json`

*Since v0.1.0*

---

## std/kv

Key-value store with SQLite and Redis/Valkey backends

```ntnt
import { open, get, set } from "std/kv"
```

### Functions

| Function | Description |
|----------|-------------|
| [`del`](#del) | Delete a key from the KV store. |
| [`expire`](#expire) | Set a TTL (time-to-live) on an existing key. |
| [`flush`](#flush) | Delete all keys from the KV store. |
| [`get`](#get) | Get a value by key from the KV store. |
| [`has`](#has) | Check if a key exists in the KV store. |
| [`list`](#list) | List keys in the KV store, optionally filtered by prefix. |
| [`open`](#open) | Open a KV store connection. |
| [`set`](#set) | Set a key-value pair in the KV store. |
| [`ttl`](#ttl) | Get the remaining TTL (time-to-live) for a key in seconds. |

#### `del`

```ntnt
del(kv: KVStore, key: String) -> Result<Bool, String>
```

Delete a key from the KV store.

Returns true if the key existed and was deleted, false if it didn't exist.

**Parameters:**

- `kv` — The KV store handle from open()
- `key` — The key to delete

**Returns:** Result containing true if deleted, false if not found

**Examples:**

```ntnt
del(cache, "user:123")  // Delete a key
```

---

#### `expire`

```ntnt
expire(kv: KVStore, key: String, seconds: Int) -> Result<Bool, String>
```

Set a TTL (time-to-live) on an existing key.

Returns true if the key exists and TTL was set, false if key doesn't exist.

**Parameters:**

- `kv` — The KV store handle from open()
- `key` — The key to set expiration on
- `seconds` — Number of seconds until expiration

**Returns:** Result containing true if TTL was set, false if key not found

**Examples:**

```ntnt
expire(cache, "user:123", 600)  // Expire in 10 minutes
```

---

#### `flush`

```ntnt
flush(kv: KVStore) -> Result<Unit, String>
```

Delete all keys from the KV store.

Use with caution - this removes all data. Useful for tests and resets.

**Parameters:**

- `kv` — The KV store handle from open()

**Returns:** Result indicating success or error

**Examples:**

```ntnt
flush(cache)  // Clear all cached data
```

---

#### `get`

```ntnt
get(kv: KVStore, key: String) -> Result<Option<Any>, String>
```

Get a value by key from the KV store.

Returns None if the key doesn't exist or has expired. Values are automatically deserialized to their original type.

**Parameters:**

- `kv` — The KV store handle from open()
- `key` — The key to retrieve

**Returns:** Result containing Some(value) or None if not found

**Examples:**

```ntnt
get(cache, "user:123")  // Get user by key
get(cache, "session:abc")  // Get session data
```

---

#### `has`

```ntnt
has(kv: KVStore, key: String) -> Result<Bool, String>
```

Check if a key exists in the KV store.

Returns false for expired keys.

**Parameters:**

- `kv` — The KV store handle from open()
- `key` — The key to check

**Returns:** Result containing true if exists, false otherwise

**Examples:**

```ntnt
has(cache, "user:123")  // Check if key exists
```

---

#### `list`

```ntnt
list(kv: KVStore, prefix?: String) -> Result<Array<String>, String>
```

List keys in the KV store, optionally filtered by prefix.

Without a prefix, returns all keys (use sparingly on large stores).

**Parameters:**

- `kv` — The KV store handle from open()
- `prefix` — Optional prefix to filter keys

**Returns:** Result containing array of matching key names

**Examples:**

```ntnt
list(cache, "user:")  // List all user keys
list(cache, "session:")  // List all session keys
list(cache)  // List all keys
```

---

#### `open`

```ntnt
open(url: String) -> Result<KVStore, String>
```

Open a KV store connection.

For SQLite (bundled, zero-config), pass a file path or ":memory:". For Redis/Valkey (production), pass a URL like "redis://host:6379".

**Parameters:**

- `url` — Connection string: file path for SQLite, redis:// or valkey:// URL for Redis/Valkey

**Returns:** Result containing the KV store handle or an error message

**Examples:**

```ntnt
open("cache.db")  // Open SQLite KV store
open(":memory:")  // Open in-memory SQLite KV store
open("redis://localhost:6379")  // Open Redis connection
open("valkey://localhost:6379/0")  // Open Valkey connection with database 0
```

---

#### `set`

```ntnt
set(kv: KVStore, key: String, value: Any, opts?: Map) -> Result<Unit, String>
```

Set a key-value pair in the KV store.

Values are automatically serialized. Maps and arrays are stored as JSON. Setting a value to None deletes the key.

**Parameters:**

- `kv` — The KV store handle from open()
- `key` — The key to set
- `value` — The value to store (string, int, float, bool, map, or array)
- `opts` — Optional map with "ttl" key for expiration in seconds

**Returns:** Result indicating success or error

**Examples:**

```ntnt
set(cache, "user:123", map { "name": "Alice" })  // Set without TTL
set(cache, "session:abc", token, map { "ttl": 3600 })  // Set with 1 hour TTL
```

---

#### `ttl`

```ntnt
ttl(kv: KVStore, key: String) -> Result<Option<Int>, String>
```

Get the remaining TTL (time-to-live) for a key in seconds.

Returns None if the key doesn't exist or has no expiration set.

**Parameters:**

- `kv` — The KV store handle from open()
- `key` — The key to check TTL for

**Returns:** Result containing Some(seconds) or None

**Examples:**

```ntnt
ttl(cache, "session:abc")  // Get remaining TTL
```

---

## std/log

Structured logging with configurable levels and JSON context

```ntnt
import { log_debug, log_info, log_warn } from "std/log"
```

### Functions

| Function | Description |
|----------|-------------|
| [`log_debug`](#logdebug) | Log a message at DEBUG level. |
| [`log_error`](#logerror) | Log a message at ERROR level. |
| [`log_info`](#loginfo) | Log a message at INFO level. |
| [`log_warn`](#logwarn) | Log a message at WARN level. |
| [`request_logger`](#requestlogger) | Create a request logging middleware function. |
| [`set_log_level`](#setloglevel) | Set the global log level. |

#### `log_debug`

```ntnt
log_debug(message: String, data?: Any) -> Unit
```

Log a message at DEBUG level.

Debug logs are for detailed diagnostic information during development. They are hidden by default (log level is INFO). Use `set_log_level("debug")` to enable. Output goes to stderr in the format: `2026-02-02T10:30:00Z [DEBUG] message {"context":"data"}`

**Parameters:**

- `message` — The log message.
- `data` — Optional context data to serialize as JSON.

**Returns:** Unit

**Examples:**

```ntnt
log_debug("Processing request", map { "id": 123 })  // Debug with context
log_debug("Checkpoint reached")  // Simple debug message
```

**See also:** `log_info`, `log_warn`, `log_error`, `set_log_level`

*Since v0.3.11*

---

#### `log_error`

```ntnt
log_error(message: String, data?: Any) -> Unit
```

Log a message at ERROR level.

Error logs are for serious problems that may prevent normal operation. Output goes to stderr in the format: `2026-02-02T10:30:00Z [ERROR] message {"context":"data"}`

**Parameters:**

- `message` — The log message.
- `data` — Optional context data to serialize as JSON.

**Returns:** Unit

**Examples:**

```ntnt
log_error("Database connection failed", map { "host": "db.example.com" })  // Error with context
log_error("Critical failure")  // Simple error message
```

**See also:** `log_debug`, `log_info`, `log_warn`, `set_log_level`

*Since v0.3.11*

---

#### `log_info`

```ntnt
log_info(message: String, data?: Any) -> Unit
```

Log a message at INFO level.

Info logs are for general operational information. This is the default log level. Output goes to stderr in the format: `2026-02-02T10:30:00Z [INFO] message {"context":"data"}`

**Parameters:**

- `message` — The log message.
- `data` — Optional context data to serialize as JSON.

**Returns:** Unit

**Examples:**

```ntnt
log_info("Server started", map { "port": 8080 })  // Info with context
log_info("User logged in")  // Simple info message
```

**See also:** `log_debug`, `log_warn`, `log_error`, `set_log_level`

*Since v0.3.11*

---

#### `log_warn`

```ntnt
log_warn(message: String, data?: Any) -> Unit
```

Log a message at WARN level.

Warn logs are for potentially harmful situations or unexpected behavior that doesn't prevent operation. Output goes to stderr in the format: `2026-02-02T10:30:00Z [WARN] message {"context":"data"}`

**Parameters:**

- `message` — The log message.
- `data` — Optional context data to serialize as JSON.

**Returns:** Unit

**Examples:**

```ntnt
log_warn("Rate limit approaching", map { "current": 95, "max": 100 })  // Warning with context
log_warn("Deprecated API called")  // Simple warning
```

**See also:** `log_debug`, `log_info`, `log_error`, `set_log_level`

*Since v0.3.11*

---

#### `request_logger`

```ntnt
request_logger() -> Function
```

Create a request logging middleware function.

Returns a function suitable for use with `use_middleware()` that logs incoming HTTP requests at INFO level. Logs the HTTP method and path for each request.

**Returns:** A middleware function that logs requests.

**Examples:**

```ntnt
use_middleware(request_logger())  // Log all incoming requests
```

**See also:** `log_info`, `use_middleware`

*Since v0.3.11*

---

#### `set_log_level`

```ntnt
set_log_level(level: String) -> Unit
```

Set the global log level.

Controls which log messages are output. Messages below the set level are silently ignored. Valid levels: "debug", "info", "warn", "error". Default is "info".

**Parameters:**

- `level` — The log level name: "debug", "info", "warn", or "error".

**Returns:** Unit

**Examples:**

```ntnt
set_log_level("debug")  // Enable debug logging
set_log_level("error")  // Only show errors
```

**Errors:**

- **TypeError**: Invalid log level — *Fix: Use 'debug', 'info', 'warn', or 'error'*

**See also:** `log_debug`, `log_info`, `log_warn`, `log_error`

*Since v0.3.11*

---

## std/markdown

```ntnt
import { to_html, to_html_safe } from "std/markdown"
```

### Functions

| Function | Description |
|----------|-------------|
| [`to_html`](#tohtml) | Convert a Markdown string to HTML. Supports GitHub Flavored Markdown: tables, strikethrough, task lists, footnotes, heading attributes. Does NOT sanitize HTML — embedded HTML tags pass through as-is. Use to_html_safe() if the input is untrusted. |
| [`to_html_safe`](#tohtmlsafe) | Convert a Markdown string to HTML with embedded HTML tags stripped. Use this when rendering user-supplied or untrusted Markdown content. |

#### `to_html`

```ntnt
to_html(markdown: String) -> String
```

Convert a Markdown string to HTML. Supports GitHub Flavored Markdown: tables, strikethrough, task lists, footnotes, heading attributes. Does NOT sanitize HTML — embedded HTML tags pass through as-is. Use to_html_safe() if the input is untrusted.

---

#### `to_html_safe`

```ntnt
to_html_safe(markdown: String) -> String
```

Convert a Markdown string to HTML with embedded HTML tags stripped. Use this when rendering user-supplied or untrusted Markdown content.

---

## std/math

Mathematical functions and constants

```ntnt
import { sin, cos, tan } from "std/math"
```

### Functions

| Function | Description |
|----------|-------------|
| [`acos`](#acos) | Compute the arccosine (inverse cosine) of a value. |
| [`asin`](#asin) | Compute the arcsine (inverse sine) of a value. |
| [`atan`](#atan) | Compute the arctangent (inverse tangent) of a value. |
| [`atan2`](#atan2) | Compute the two-argument arctangent of y and x. |
| [`cbrt`](#cbrt) | Compute the cube root of a value. |
| [`cos`](#cos) | Compute the cosine of an angle in radians. |
| [`cosh`](#cosh) | Compute the hyperbolic cosine of a value. |
| [`degrees`](#degrees) | Convert an angle from radians to degrees. |
| [`exp`](#exp) | Compute e raised to the power of x. |
| [`exp2`](#exp2) | Compute 2 raised to the power of x. |
| [`hypot`](#hypot) | Compute the Euclidean distance sqrt(x^2 + y^2). |
| [`is_finite`](#isfinite) | Check whether a numeric value is finite (not NaN and not infinite). |
| [`is_infinite`](#isinfinite) | Check whether a numeric value is positive or negative infinity. |
| [`is_nan`](#isnan) | Check whether a numeric value is NaN (Not a Number). |
| [`log`](#log) | Compute the natural logarithm (base e) of a value. |
| [`log10`](#log10) | Compute the base-10 logarithm of a value. |
| [`log2`](#log2) | Compute the base-2 logarithm of a value. |
| [`radians`](#radians) | Convert an angle from degrees to radians. |
| [`random`](#random) | Generate a random floating-point number in [0, 1). |
| [`random_int`](#randomint) | Generate a random integer in the inclusive range [min, max]. |
| [`random_range`](#randomrange) | Generate a random float in the half-open range [min, max). |
| [`sin`](#sin) | Compute the sine of an angle in radians. |
| [`sinh`](#sinh) | Compute the hyperbolic sine of a value. |
| [`tan`](#tan) | Compute the tangent of an angle in radians. |
| [`tanh`](#tanh) | Compute the hyperbolic tangent of a value. |

#### `acos`

```ntnt
acos(x: Number) -> Float
```

Compute the arccosine (inverse cosine) of a value.

Returns the angle in radians whose cosine is x. The input must be in the range [-1, 1]; values outside produce NaN.

**Parameters:**

- `x` — A value in the range [-1, 1].

**Returns:** The arccosine of x in radians as a Float.

**Examples:**

```ntnt
acos(1)  // => 0.0  // Arccosine of one
acos(0)  // => 1.5707963267948966  // Arccosine of zero is pi/2
```

**Errors:**

- **TypeError**: acos() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `cos`, `asin`, `atan`, `degrees`

*Since v0.1.0*

---

#### `asin`

```ntnt
asin(x: Number) -> Float
```

Compute the arcsine (inverse sine) of a value.

Returns the angle in radians whose sine is x. The input must be in the range [-1, 1]; values outside produce NaN.

**Parameters:**

- `x` — A value in the range [-1, 1].

**Returns:** The arcsine of x in radians as a Float.

**Examples:**

```ntnt
asin(0)  // => 0.0  // Arcsine of zero
asin(1)  // => 1.5707963267948966  // Arcsine of one is pi/2
```

**Errors:**

- **TypeError**: asin() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `sin`, `acos`, `atan`, `degrees`

*Since v0.1.0*

---

#### `atan`

```ntnt
atan(x: Number) -> Float
```

Compute the arctangent (inverse tangent) of a value.

Returns the angle in radians whose tangent is x.

**Parameters:**

- `x` — The value to compute the arctangent of.

**Returns:** The arctangent of x in radians as a Float.

**Examples:**

```ntnt
atan(0)  // => 0.0  // Arctangent of zero
atan(1)  // => 0.7853981633974483  // Arctangent of one is pi/4
```

**Errors:**

- **TypeError**: atan() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `tan`, `atan2`, `asin`, `acos`, `degrees`

*Since v0.1.0*

---

#### `atan2`

```ntnt
atan2(y: Number, x: Number) -> Float
```

Compute the two-argument arctangent of y and x.

Returns the angle in radians between the positive x-axis and the point (x, y), using the signs of both arguments to determine the correct quadrant.

**Parameters:**

- `y` — The y-coordinate.
- `x` — The x-coordinate.

**Returns:** The angle in radians as a Float.

**Examples:**

```ntnt
atan2(0, 1)  // => 0.0  // Angle to (1,0) is zero
atan2(1, 0)  // => 1.5707963267948966  // Angle to (0,1) is pi/2
```

**Errors:**

- **TypeError**: atan2() requires numbers — *Fix: Pass Int or Float arguments*

**See also:** `atan`, `sin`, `cos`, `tan`, `degrees`

*Since v0.1.0*

---

#### `cbrt`

```ntnt
cbrt(x: Number) -> Float
```

Compute the cube root of a value.

Returns the cube root of x. Works for negative numbers as well.

**Parameters:**

- `x` — The value to take the cube root of.

**Returns:** The cube root of x as a Float.

**Examples:**

```ntnt
cbrt(27)  // => 3.0  // Cube root of 27
cbrt(8)  // => 2.0  // Cube root of 8
cbrt(-8)  // => -2.0  // Cube root of negative 8
```

**Errors:**

- **TypeError**: cbrt() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `hypot`, `exp`, `log`

*Since v0.1.0*

---

#### `cos`

```ntnt
cos(x: Number) -> Float
```

Compute the cosine of an angle in radians.

Returns the cosine of the given angle. Accepts Int or Float.

**Parameters:**

- `x` — The angle in radians.

**Returns:** The cosine of x as a Float.

**Examples:**

```ntnt
cos(0)  // => 1.0  // Cosine of zero
cos(PI)  // => -1.0  // Cosine of pi
```

**Errors:**

- **TypeError**: cos() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `sin`, `tan`, `acos`, `cosh`, `degrees`, `radians`

*Since v0.1.0*

---

#### `cosh`

```ntnt
cosh(x: Number) -> Float
```

Compute the hyperbolic cosine of a value.

Returns the hyperbolic cosine of x. Accepts Int or Float.

**Parameters:**

- `x` — The input value.

**Returns:** The hyperbolic cosine of x as a Float.

**Examples:**

```ntnt
cosh(0)  // => 1.0  // Hyperbolic cosine of zero
cosh(1)  // => 1.5430806348152437  // Hyperbolic cosine of one
```

**Errors:**

- **TypeError**: cosh() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `sinh`, `tanh`, `cos`

*Since v0.1.0*

---

#### `degrees`

```ntnt
degrees(x: Number) -> Float
```

Convert an angle from radians to degrees.

Multiplies x by 180/PI to convert radians to degrees.

**Parameters:**

- `x` — The angle in radians.

**Returns:** The angle in degrees as a Float.

**Examples:**

```ntnt
degrees(PI)  // => 180.0  // Pi radians is 180 degrees
degrees(0)  // => 0.0  // Zero radians is zero degrees
```

**Errors:**

- **TypeError**: degrees() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `radians`, `sin`, `cos`, `tan`, `PI`

*Since v0.1.0*

---

#### `exp`

```ntnt
exp(x: Number) -> Float
```

Compute e raised to the power of x.

Returns e^x where e is Euler's number (~2.71828).

**Parameters:**

- `x` — The exponent.

**Returns:** e^x as a Float.

**Examples:**

```ntnt
exp(0)  // => 1.0  // e to the zero
exp(1)  // => 2.718281828459045  // e to the one
```

**Errors:**

- **TypeError**: exp() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `exp2`, `log`, `E`

*Since v0.1.0*

---

#### `exp2`

```ntnt
exp2(x: Number) -> Float
```

Compute 2 raised to the power of x.

Returns 2^x.

**Parameters:**

- `x` — The exponent.

**Returns:** 2^x as a Float.

**Examples:**

```ntnt
exp2(0)  // => 1.0  // 2 to the zero
exp2(3)  // => 8.0  // 2 to the three
exp2(10)  // => 1024.0  // 2 to the ten
```

**Errors:**

- **TypeError**: exp2() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `exp`, `log2`

*Since v0.1.0*

---

#### `hypot`

```ntnt
hypot(x: Number, y: Number) -> Float
```

Compute the Euclidean distance sqrt(x^2 + y^2).

Returns the length of the hypotenuse of a right triangle with legs x and y, computed in a numerically stable way.

**Parameters:**

- `x` — The first leg.
- `y` — The second leg.

**Returns:** sqrt(x^2 + y^2) as a Float.

**Examples:**

```ntnt
hypot(3, 4)  // => 5.0  // Classic 3-4-5 triangle
hypot(5, 12)  // => 13.0  // 5-12-13 triangle
```

**Errors:**

- **TypeError**: hypot() requires numbers — *Fix: Pass Int or Float arguments*

**See also:** `cbrt`, `atan2`

*Since v0.1.0*

---

#### `is_finite`

```ntnt
is_finite(x: Number) -> Bool
```

Check whether a numeric value is finite (not NaN and not infinite).

Returns true if x is a finite number (neither NaN nor infinity), false otherwise. Integers always return true.

**Parameters:**

- `x` — The number to check.

**Returns:** true if x is finite, false otherwise.

**Examples:**

```ntnt
is_finite(42)  // => true  // Integers are always finite
is_finite(3.14)  // => true  // Normal floats are finite
```

**Errors:**

- **TypeError**: is_finite() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `is_nan`, `is_infinite`

*Since v0.1.0*

---

#### `is_infinite`

```ntnt
is_infinite(x: Number) -> Bool
```

Check whether a numeric value is positive or negative infinity.

Returns true if x is positive or negative infinity, false otherwise. Integers always return false.

**Parameters:**

- `x` — The number to check.

**Returns:** true if x is infinite, false otherwise.

**Examples:**

```ntnt
is_infinite(0.0)  // => false  // Zero is not infinite
is_infinite(1)  // => false  // Integers are never infinite
```

**Errors:**

- **TypeError**: is_infinite() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `is_nan`, `is_finite`, `INFINITY`, `NEG_INFINITY`

*Since v0.1.0*

---

#### `is_nan`

```ntnt
is_nan(x: Number) -> Bool
```

Check whether a numeric value is NaN (Not a Number).

Returns true if x is the special NaN floating-point value, false otherwise. Integers always return false.

**Parameters:**

- `x` — The number to check.

**Returns:** true if x is NaN, false otherwise.

**Examples:**

```ntnt
is_nan(0.0)  // => false  // Zero is not NaN
is_nan(1)  // => false  // Integers are never NaN
```

**Errors:**

- **TypeError**: is_nan() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `is_infinite`, `is_finite`

*Since v0.1.0*

---

#### `log`

```ntnt
log(x: Number) -> Float
```

Compute the natural logarithm (base e) of a value.

Returns ln(x). The input must be a positive number; zero or negative values produce a RuntimeError.

**Parameters:**

- `x` — A positive number.

**Returns:** The natural logarithm of x as a Float.

**Examples:**

```ntnt
log(1)  // => 0.0  // Natural log of one
log(E)  // => 1.0  // Natural log of e
```

**Errors:**

- **TypeError**: log() requires a number — *Fix: Pass an Int or Float argument*
- **RuntimeError**: log() requires positive number — *Fix: Ensure the argument is greater than zero*

**See also:** `log10`, `log2`, `exp`

*Since v0.1.0*

---

#### `log10`

```ntnt
log10(x: Number) -> Float
```

Compute the base-10 logarithm of a value.

Returns log10(x). The input must be a positive number; zero or negative values produce a RuntimeError.

**Parameters:**

- `x` — A positive number.

**Returns:** The base-10 logarithm of x as a Float.

**Examples:**

```ntnt
log10(1)  // => 0.0  // Log base 10 of one
log10(100)  // => 2.0  // Log base 10 of one hundred
```

**Errors:**

- **TypeError**: log10() requires a number — *Fix: Pass an Int or Float argument*
- **RuntimeError**: log10() requires positive number — *Fix: Ensure the argument is greater than zero*

**See also:** `log`, `log2`, `exp`

*Since v0.1.0*

---

#### `log2`

```ntnt
log2(x: Number) -> Float
```

Compute the base-2 logarithm of a value.

Returns log2(x). The input must be a positive number; zero or negative values produce a RuntimeError.

**Parameters:**

- `x` — A positive number.

**Returns:** The base-2 logarithm of x as a Float.

**Examples:**

```ntnt
log2(1)  // => 0.0  // Log base 2 of one
log2(8)  // => 3.0  // Log base 2 of eight
```

**Errors:**

- **TypeError**: log2() requires a number — *Fix: Pass an Int or Float argument*
- **RuntimeError**: log2() requires positive number — *Fix: Ensure the argument is greater than zero*

**See also:** `log`, `log10`, `exp2`

*Since v0.1.0*

---

#### `radians`

```ntnt
radians(x: Number) -> Float
```

Convert an angle from degrees to radians.

Multiplies x by PI/180 to convert degrees to radians.

**Parameters:**

- `x` — The angle in degrees.

**Returns:** The angle in radians as a Float.

**Examples:**

```ntnt
radians(180)  // => 3.141592653589793  // 180 degrees is pi radians
radians(0)  // => 0.0  // Zero degrees is zero radians
```

**Errors:**

- **TypeError**: radians() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `degrees`, `sin`, `cos`, `tan`, `PI`

*Since v0.1.0*

---

#### `random`

```ntnt
random() -> Float
```

Generate a random floating-point number in [0, 1).

Returns a uniformly distributed random Float greater than or equal to 0.0 and strictly less than 1.0.

**Returns:** A random Float in [0, 1).

**Examples:**

```ntnt
random()  // => 0.42  // Returns a random float (value varies)
```

**See also:** `random_int`, `random_range`

*Since v0.1.0*

---

#### `random_int`

```ntnt
random_int(min: Int, max: Int) -> Int
```

Generate a random integer in the inclusive range [min, max].

Returns a uniformly distributed random integer between min and max, inclusive on both ends. min must be less than or equal to max.

**Parameters:**

- `min` — The lower bound (inclusive).
- `max` — The upper bound (inclusive).

**Returns:** A random Int in [min, max].

**Examples:**

```ntnt
random_int(1, 6)  // => 3  // Simulates a dice roll (value varies)
```

**Errors:**

- **TypeError**: random_int() requires integers — *Fix: Pass Int arguments, not Float*
- **RuntimeError**: random_int() min must be <= max — *Fix: Ensure the first argument is less than or equal to the second*

**See also:** `random`, `random_range`

*Since v0.1.0*

---

#### `random_range`

```ntnt
random_range(min: Number, max: Number) -> Float
```

Generate a random float in the half-open range [min, max).

Returns a uniformly distributed random Float greater than or equal to min and strictly less than max. min must be less than or equal to max.

**Parameters:**

- `min` — The lower bound (inclusive).
- `max` — The upper bound (exclusive).

**Returns:** A random Float in [min, max).

**Examples:**

```ntnt
random_range(0.0, 10.0)  // => 4.2  // Random float between 0 and 10 (value varies)
```

**Errors:**

- **TypeError**: random_range() requires numbers — *Fix: Pass Int or Float arguments*
- **RuntimeError**: random_range() min must be <= max — *Fix: Ensure the first argument is less than or equal to the second*

**See also:** `random`, `random_int`

*Since v0.1.0*

---

#### `sin`

```ntnt
sin(x: Number) -> Float
```

Compute the sine of an angle in radians.

Returns the sine of the given angle. Accepts Int or Float.

**Parameters:**

- `x` — The angle in radians.

**Returns:** The sine of x as a Float.

**Examples:**

```ntnt
sin(0)  // => 0.0  // Sine of zero
sin(PI / 2)  // => 1.0  // Sine of pi/2
```

**Errors:**

- **TypeError**: sin() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `cos`, `tan`, `asin`, `sinh`, `degrees`, `radians`

*Since v0.1.0*

---

#### `sinh`

```ntnt
sinh(x: Number) -> Float
```

Compute the hyperbolic sine of a value.

Returns the hyperbolic sine of x. Accepts Int or Float.

**Parameters:**

- `x` — The input value.

**Returns:** The hyperbolic sine of x as a Float.

**Examples:**

```ntnt
sinh(0)  // => 0.0  // Hyperbolic sine of zero
sinh(1)  // => 1.1752011936438014  // Hyperbolic sine of one
```

**Errors:**

- **TypeError**: sinh() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `cosh`, `tanh`, `sin`

*Since v0.1.0*

---

#### `tan`

```ntnt
tan(x: Number) -> Float
```

Compute the tangent of an angle in radians.

Returns the tangent of the given angle. Accepts Int or Float.

**Parameters:**

- `x` — The angle in radians.

**Returns:** The tangent of x as a Float.

**Examples:**

```ntnt
tan(0)  // => 0.0  // Tangent of zero
```

**Errors:**

- **TypeError**: tan() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `sin`, `cos`, `atan`, `atan2`, `tanh`, `degrees`, `radians`

*Since v0.1.0*

---

#### `tanh`

```ntnt
tanh(x: Number) -> Float
```

Compute the hyperbolic tangent of a value.

Returns the hyperbolic tangent of x. Accepts Int or Float.

**Parameters:**

- `x` — The input value.

**Returns:** The hyperbolic tangent of x as a Float.

**Examples:**

```ntnt
tanh(0)  // => 0.0  // Hyperbolic tangent of zero
tanh(1)  // => 0.7615941559557649  // Hyperbolic tangent of one
```

**Errors:**

- **TypeError**: tanh() requires a number — *Fix: Pass an Int or Float argument*

**See also:** `sinh`, `cosh`, `tan`

*Since v0.1.0*

---

## std/path

File path manipulation and resolution

```ntnt
import { join_path, join, dirname } from "std/path"
```

### Functions

| Function | Description |
|----------|-------------|
| [`basename`](#basename) | Returns the filename portion of a path. |
| [`dirname`](#dirname) | Returns the directory portion of a path. |
| [`extension`](#extension) | Returns the file extension without the leading dot. |
| [`is_absolute`](#isabsolute) | Returns true if the path is absolute. |
| [`is_relative`](#isrelative) | Returns true if the path is relative. |
| [`join`](#join) | Deprecated: use join_path() instead. Alias for backward compatibility. |
| [`join_path`](#joinpath) | Joins path segments into a single path string. |
| [`normalize`](#normalize) | Cleans up `..` and `.` path components without touching the filesystem. |
| [`resolve`](#resolve) | Resolves a path to an absolute path using filesystem canonicalize. |
| [`stem`](#stem) | Returns the filename without its extension. |
| [`with_extension`](#withextension) | Returns the path with its extension changed to the given extension. |

#### `basename`

```ntnt
basename(path: String) -> Option<String>
```

Returns the filename portion of a path.

**Parameters:**

- `path` — The file path to extract the filename from

**Examples:**

```ntnt
basename("src/lib/main.tnt")  // => Some("main.tnt")  // Returns filename portion
```

**See also:** `dirname`, `extension`, `stem`, `join`

*Since v0.1.0*

---

#### `dirname`

```ntnt
dirname(path: String) -> Option<String>
```

Returns the directory portion of a path.

**Parameters:**

- `path` — The file path to extract the directory from

**Examples:**

```ntnt
dirname("src/lib/main.tnt")  // => Some("src/lib")  // Returns directory portion
```

**See also:** `basename`, `join`, `extension`, `stem`

*Since v0.1.0*

---

#### `extension`

```ntnt
extension(path: String) -> Option<String>
```

Returns the file extension without the leading dot.

**Parameters:**

- `path` — The file path to extract the extension from

**Examples:**

```ntnt
extension("main.tnt")  // => Some("tnt")  // Returns file extension without dot
```

**See also:** `stem`, `basename`, `with_extension`

*Since v0.1.0*

---

#### `is_absolute`

```ntnt
is_absolute(path: String) -> Bool
```

Returns true if the path is absolute.

**Parameters:**

- `path` — The file path to check

**Examples:**

```ntnt
is_absolute("/usr/bin")  // => true  // Absolute path starts with /
is_absolute("src/main.tnt")  // => false  // Relative path is not absolute
```

**See also:** `is_relative`

*Since v0.1.0*

---

#### `is_relative`

```ntnt
is_relative(path: String) -> Bool
```

Returns true if the path is relative.

**Parameters:**

- `path` — The file path to check

**Examples:**

```ntnt
is_relative("src/main.tnt")  // => true  // Relative path without leading /
is_relative("/usr/bin")  // => false  // Absolute path is not relative
```

**See also:** `is_absolute`

*Since v0.1.0*

---

#### `join`

```ntnt
join(parts: Array<String>) -> String
```

Deprecated: use join_path() instead. Alias for backward compatibility.

**Parameters:**

- `parts` — Array of path segments to join

**Examples:**

```ntnt
join(["src", "lib"])  // => "src/lib"  // Deprecated: use join_path()
```

**See also:** `join_path`

*Since v0.1.0*

---

#### `join_path`

```ntnt
join_path(parts: Array<String>) -> String
```

Joins path segments into a single path string.

Renamed from join() to avoid ambiguity with join() in std/string and std/url.

**Parameters:**

- `parts` — Array of path segments to join

**Examples:**

```ntnt
join_path(["src", "lib", "main.tnt"])  // => "src/lib/main.tnt"  // Joins path segments
```

**See also:** `dirname`, `basename`, `normalize`

*Since v0.4.0*

---

#### `normalize`

```ntnt
normalize(path: String) -> String
```

Cleans up `..` and `.` path components without touching the filesystem.

**Parameters:**

- `path` — The file path to normalize

**Examples:**

```ntnt
normalize("a/b/../c")  // => "a/c"  // Cleans up path components
```

**See also:** `join`, `resolve`, `dirname`

*Since v0.2.0*

---

#### `resolve`

```ntnt
resolve(path: String) -> Result<String, String>
```

Resolves a path to an absolute path using filesystem canonicalize.

**Parameters:**

- `path` — The file path to resolve

**Examples:**

```ntnt
resolve(".")  // => Ok("/Users/dev/project")  // Resolves current directory to absolute path
resolve("nonexistent")  // => Err("No such file or directory")  // Returns Err for missing path
```

**See also:** `is_absolute`, `normalize`

*Since v0.2.0*

---

#### `stem`

```ntnt
stem(path: String) -> Option<String>
```

Returns the filename without its extension.

**Parameters:**

- `path` — The file path to extract the stem from

**Examples:**

```ntnt
stem("main.tnt")  // => Some("main")  // Returns filename without extension
```

**See also:** `extension`, `basename`, `with_extension`

*Since v0.1.0*

---

#### `with_extension`

```ntnt
with_extension(path: String, ext: String) -> String
```

Returns the path with its extension changed to the given extension.

**Parameters:**

- `path` — The original file path
- `ext` — The new extension (without leading dot)

**Examples:**

```ntnt
with_extension("file.txt", "md")  // => "file.md"  // Changes file extension
```

**See also:** `extension`, `stem`, `basename`

*Since v0.2.0*

---

## std/postgres

PostgreSQL database operations

```ntnt
import { connect, query, query_one } from "std/postgres"
```

### Functions

| Function | Description |
|----------|-------------|
| [`begin`](#begin) | Begin a database transaction. |
| [`close`](#close) | Close a PostgreSQL database connection pool. |
| [`commit`](#commit) | Commit the current transaction. |
| [`connect`](#connect) | Open a connection pool to a PostgreSQL database. |
| [`execute`](#execute) | Execute a SQL statement and return the number of affected rows. |
| [`query`](#query) | Execute a SQL query and return all matching rows. |
| [`query_one`](#queryone) | Execute a SQL query and return at most one row. |
| [`rollback`](#rollback) | Roll back the current transaction. |

#### `begin`

```ntnt
begin(conn: Connection) -> Result<Connection, String>
```

Begin a database transaction.

Checks out a dedicated connection from the pool and issues a SQL BEGIN statement. On success the same connection handle is returned inside Result::Ok -- subsequent query() and execute() calls on that handle operate within the transaction until commit() or rollback() is called.

**Parameters:**

- `conn` — A Connection handle obtained from connect()

**Returns:** Result::Ok containing the Connection handle (now in a transaction), or Result::Err with a description

**Examples:**

```ntnt
begin(db)  // => Result::Ok(Connection)  // Start a transaction
```

**Errors:**

- **RuntimeError**: Failed to lock connection: ... — *Fix: Ensure the connection is not being used concurrently*

**See also:** `commit`, `rollback`, `execute`, `query`

*Since v0.2.0*

---

#### `close`

```ntnt
close(conn: Connection) -> Bool
```

Close a PostgreSQL database connection pool.

Removes the connection pool from the internal registry, allowing all pooled connections to be released. Returns true if the pool was found and removed, false otherwise.

**Parameters:**

- `conn` — A Connection handle obtained from connect()

**Returns:** true if the connection was successfully closed, false if it was not found

**Examples:**

```ntnt
close(db)  // => true  // Close an open connection
```

**Errors:**

- **TypeError**: Expected a database connection handle — *Fix: Pass a Connection handle returned by connect()*

**See also:** `connect`

*Since v0.2.0*

---

#### `commit`

```ntnt
commit(conn: Connection) -> Result<Bool, String>
```

Commit the current transaction.

Issues a SQL COMMIT on the dedicated transaction connection, making all changes since the last begin() permanent. Returns true on success. The dedicated connection is returned to the pool after commit.

**Parameters:**

- `conn` — A Connection handle with an active transaction from begin()

**Returns:** true on success, or Result::Err with a description on failure

**Examples:**

```ntnt
commit(db)  // => true  // Commit an active transaction
```

**Errors:**

- **RuntimeError**: COMMIT failed: ... — *Fix: Ensure a transaction was started with begin() before committing*

**See also:** `begin`, `rollback`

*Since v0.2.0*

---

#### `connect`

```ntnt
connect(connection_string: String) -> Result<Connection, String>
```

Open a connection pool to a PostgreSQL database.

Establishes a connection pool using the provided connection string and returns a connection handle that can be passed to query, execute, and transaction functions. The handle is stored in a global registry keyed by an internal connection ID. Uses deadpool-postgres for async pooling. Pool size defaults to 5 connections per pool (configurable via NTNT_DB_POOL_SIZE env var). Note: each worker creates its own pools, so total connections = num_workers × num_databases × pool_size.

**Parameters:**

- `connection_string` — A PostgreSQL connection URI (e.g. "postgres://user:pass@localhost/mydb")

**Returns:** Result::Ok containing a Connection map handle, or Result::Err with a description

**Examples:**

```ntnt
connect("postgres://user:pass@localhost/mydb")  // => Result::Ok(Connection)  // Open a database connection
```

**Errors:**

- **TypeError**: connect() requires a connection string — *Fix: Pass a String connection URI as the argument*

**See also:** `close`, `query`, `execute`, `begin`

*Since v0.2.0*

---

#### `execute`

```ntnt
execute(conn: Connection, sql: String, params: Array | Unit) -> Result<Int, String>
```

Execute a SQL statement and return the number of affected rows.

Use this for INSERT, UPDATE, DELETE, and other statements that do not return row data. Parameters use $1, $2, ... placeholders. The Ok variant contains an Int representing the count of rows affected.

**Parameters:**

- `conn` — A Connection handle obtained from connect()
- `sql` — The SQL statement string with optional $N parameter placeholders
- `params` — An Array of bind parameter values, or Unit for no parameters

**Returns:** Result::Ok containing the Int count of affected rows, or Result::Err with a description

**Examples:**

```ntnt
execute(db, "INSERT INTO users (name) VALUES ($1)", ["Alice"])  // => Result::Ok(1)  // Insert one row
```

**Errors:**

- **TypeError**: execute() requires (connection, sql_string, params_array) — *Fix: Provide (Connection, String, Array) arguments*

**See also:** `query`, `query_one`, `connect`, `begin`

*Since v0.2.0*

---

#### `query`

```ntnt
query(conn: Connection, sql: String, params: Array | Unit) -> Result<Array<Map>, String>
```

Execute a SQL query and return all matching rows.

Runs a parameterized SELECT (or any row-returning statement) against the database. Parameters use PostgreSQL $1, $2, ... placeholders. Each returned row is a Map whose keys are column names. Pass an empty array or Unit when no parameters are needed.

**Parameters:**

- `conn` — A Connection handle obtained from connect()
- `sql` — The SQL query string with optional $N parameter placeholders
- `params` — An Array of bind parameter values, or Unit for no parameters

**Returns:** Result::Ok containing an Array of row Maps, or Result::Err with a description

**Examples:**

```ntnt
query(db, "SELECT * FROM users WHERE active = $1", [true])  // => Result::Ok([...])  // Query with parameters
```

**Errors:**

- **TypeError**: query() requires (connection, sql_string, params_array) — *Fix: Provide (Connection, String, Array) arguments*

**Gotchas:**

- SQL NULL column values are returned as None, not Unit

**See also:** `query_one`, `execute`, `connect`

*Since v0.2.0*

---

#### `query_one`

```ntnt
query_one(conn: Connection, sql: String, params: Array | Unit) -> Result<Map | None, String>
```

Execute a SQL query and return at most one row.

Behaves like query() but uses PostgreSQL's query_opt internally to return either a single row Map or None when no row matches. Ideal for lookups by primary key or unique column.

**Parameters:**

- `conn` — A Connection handle obtained from connect()
- `sql` — The SQL query string with optional $N parameter placeholders
- `params` — An Array of bind parameter values, or Unit for no parameters

**Returns:** Result::Ok containing a row Map or None if no match, or Result::Err with a description

**Examples:**

```ntnt
query_one(db, "SELECT * FROM users WHERE id = $1", [1])  // => Result::Ok({...})  // Fetch one row by ID
query_one(db, "SELECT * FROM users WHERE id = $1", [999])  // => Result::Ok(None)  // No matching row
```

**Errors:**

- **TypeError**: query_one() requires (connection, sql_string, params_array) — *Fix: Provide (Connection, String, Array) arguments*

**Gotchas:**

- SQL NULL column values are returned as None, not Unit

**See also:** `query`, `execute`, `connect`

*Since v0.2.0*

---

#### `rollback`

```ntnt
rollback(conn: Connection) -> Result<Bool, String>
```

Roll back the current transaction.

Issues a SQL ROLLBACK on the dedicated transaction connection, discarding all changes made since the last begin(). Returns true on success. The dedicated connection is returned to the pool after rollback.

**Parameters:**

- `conn` — A Connection handle with an active transaction from begin()

**Returns:** true on success, or Result::Err with a description on failure

**Examples:**

```ntnt
rollback(db)  // => true  // Roll back an active transaction
```

**Errors:**

- **RuntimeError**: ROLLBACK failed: ... — *Fix: Ensure a transaction was started with begin() before rolling back*

**See also:** `begin`, `commit`

*Since v0.2.0*

---

## std/sqlite

SQLite database operations

```ntnt
import { connect, query, query_one } from "std/sqlite"
```

### Functions

| Function | Description |
|----------|-------------|
| [`begin`](#begin) | Begin a database transaction. |
| [`close`](#close) | Close a SQLite database connection. |
| [`commit`](#commit) | Commit the current transaction. |
| [`connect`](#connect) | Open a connection to a SQLite database. |
| [`execute`](#execute) | Execute a SQL statement and return the number of affected rows. |
| [`query`](#query) | Execute a SELECT query and return all matching rows. |
| [`query_one`](#queryone) | Execute a SELECT query and return a single row. |
| [`rollback`](#rollback) | Roll back the current transaction. |

#### `begin`

```ntnt
begin(conn: Connection) -> Result<Connection, String>
```

Begin a database transaction.

Starts a new SQLite transaction on the given connection. All subsequent execute and query calls on this connection will be part of the transaction until commit() or rollback() is called. Returns the same connection handle wrapped in a Result for chaining.

**Parameters:**

- `conn` — A connection handle obtained from connect()

**Returns:** Result containing the connection handle on success, or an error string on failure

**Examples:**

```ntnt
begin(db)  // => Result::Ok(db)  // Start a transaction
```

**Errors:**

- **RuntimeError**: BEGIN failed: ... — *Fix: Ensure no transaction is already active on this connection*
- **RuntimeError**: Invalid or closed SQLite connection — *Fix: Use an open connection handle from connect()*
- **RuntimeError**: Failed to lock connection: ... — *Fix: Ensure connection is not used concurrently in conflicting ways*

**See also:** `commit`, `rollback`

*Since v0.2.0*

---

#### `close`

```ntnt
close(conn: Connection) -> Bool
```

Close a SQLite database connection.

Removes the connection from the internal registry and releases the underlying SQLite resources. Returns true if the connection was found and closed, false otherwise. After closing, any further operations on the connection handle will fail.

**Parameters:**

- `conn` — A connection handle obtained from connect()

**Returns:** true if the connection was successfully closed, false otherwise

**Examples:**

```ntnt
close(db)  // => true  // Close an open connection
```

**Errors:**

- **TypeError**: Expected a SQLite connection handle — *Fix: Pass a valid connection handle from connect()*

**See also:** `connect`

*Since v0.2.0*

---

#### `commit`

```ntnt
commit(conn: Connection) -> Result<Bool, String>
```

Commit the current transaction.

Commits all changes made since the last begin() call, making them permanent in the database. Returns true on success. If no transaction is active, SQLite will return an error.

**Parameters:**

- `conn` — A connection handle with an active transaction

**Returns:** true on success, or a Result::Err with an error string on failure

**Examples:**

```ntnt
commit(db)  // => true  // Commit the active transaction
```

**Errors:**

- **RuntimeError**: COMMIT failed: ... — *Fix: Ensure a transaction was started with begin() before committing*
- **RuntimeError**: Invalid or closed SQLite connection — *Fix: Use an open connection handle from connect()*
- **RuntimeError**: Failed to lock connection: ... — *Fix: Ensure connection is not used concurrently in conflicting ways*

**See also:** `begin`, `rollback`

*Since v0.2.0*

---

#### `connect`

```ntnt
connect(path: String) -> Result<Connection, String>
```

Open a connection to a SQLite database.

Opens a file-based or in-memory SQLite database. Automatically enables WAL journal mode for better concurrent read performance and turns on foreign key enforcement. Returns a connection handle for use with query, execute, and transaction functions.

**Parameters:**

- `path` — File path to the database, or ":memory:" for an in-memory database

**Returns:** Result containing a connection handle map on success, or an error string on failure

**Examples:**

```ntnt
connect(":memory:")  // => Result::Ok(connection)  // Open in-memory database
connect("app.db")  // => Result::Ok(connection)  // Open file-based database
```

**Errors:**

- **TypeError**: connect() requires a database path string — *Fix: Pass a String path argument*

**See also:** `close`, `query`, `execute`

*Since v0.2.0*

---

#### `execute`

```ntnt
execute(conn: Connection, sql: String, params: Array) -> Result<Int, String>
```

Execute a SQL statement and return the number of affected rows.

Runs a parameterized INSERT, UPDATE, DELETE, or DDL statement against the database. Returns the count of rows affected by the operation. Use positional `?` placeholders for parameterized statements to prevent SQL injection.

**Parameters:**

- `conn` — A connection handle obtained from connect()
- `sql` — SQL statement string with optional `?` placeholders
- `params` — Array of parameter values to bind, or unit for no parameters

**Returns:** Result containing the number of affected rows (Int) on success, or an error string on failure

**Examples:**

```ntnt
execute(db, "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT)", [])  // => Result::Ok(0)  // Create table
execute(db, "INSERT INTO users (name) VALUES (?)", ["Alice"])  // => Result::Ok(1)  // Insert row
```

**Errors:**

- **TypeError**: execute() requires (connection, sql_string, params_array) — *Fix: Pass (connection, sql_string, params_array)*
- **RuntimeError**: Execute failed: ... — *Fix: Check SQL syntax, table/column names, and constraint violations*
- **RuntimeError**: Failed to lock connection: ... — *Fix: Ensure connection is not used concurrently in conflicting ways*

**See also:** `query`, `query_one`, `connect`

*Since v0.2.0*

---

#### `query`

```ntnt
query(conn: Connection, sql: String, params: Array) -> Result<Array<Map>, String>
```

Execute a SELECT query and return all matching rows.

Runs a parameterized SQL query against the database and returns all result rows as an array of maps. Each map represents a row with column names as keys. Use positional `?` placeholders for parameterized queries to prevent SQL injection.

**Parameters:**

- `conn` — A connection handle obtained from connect()
- `sql` — SQL query string with optional `?` placeholders
- `params` — Array of parameter values to bind, or unit for no parameters

**Returns:** Result containing an Array of row Maps on success, or an error string on failure

**Examples:**

```ntnt
query(db, "SELECT * FROM users", [])  // => Result::Ok([...])  // Fetch all rows
query(db, "SELECT * FROM users WHERE id = ?", [1])  // => Result::Ok([...])  // Parameterized query
```

**Errors:**

- **TypeError**: query() requires (connection, sql_string, params_array) — *Fix: Pass (connection, sql_string, params_array)*
- **RuntimeError**: Query preparation failed: ... — *Fix: Check SQL syntax and table/column names*
- **RuntimeError**: Failed to lock connection: ... — *Fix: Ensure connection is not used concurrently in conflicting ways*

**Gotchas:**

- SQL NULL column values are returned as None, not Unit

**See also:** `query_one`, `execute`, `connect`

*Since v0.2.0*

---

#### `query_one`

```ntnt
query_one(conn: Connection, sql: String, params: Array) -> Result<Map | None, String>
```

Execute a SELECT query and return a single row.

Runs a parameterized SQL query expecting at most one result row. Returns the row as a map if found, or None if no rows match. Useful for lookups by primary key or unique constraint.

**Parameters:**

- `conn` — A connection handle obtained from connect()
- `sql` — SQL query string with optional `?` placeholders
- `params` — Array of parameter values to bind, or unit for no parameters

**Returns:** Result containing a row Map if found, None if no match, or an error string on failure

**Examples:**

```ntnt
query_one(db, "SELECT * FROM users WHERE id = ?", [1])  // => Result::Ok({...})  // Fetch single row
query_one(db, "SELECT * FROM users WHERE id = ?", [999])  // => Result::Ok(None)  // No matching row
```

**Errors:**

- **TypeError**: query_one() requires (connection, sql_string, params_array) — *Fix: Pass (connection, sql_string, params_array)*
- **RuntimeError**: Query preparation failed: ... — *Fix: Check SQL syntax and table/column names*
- **RuntimeError**: Failed to lock connection: ... — *Fix: Ensure connection is not used concurrently in conflicting ways*

**Gotchas:**

- SQL NULL column values are returned as None, not Unit

**See also:** `query`, `execute`, `connect`

*Since v0.2.0*

---

#### `rollback`

```ntnt
rollback(conn: Connection) -> Result<Bool, String>
```

Roll back the current transaction.

Discards all changes made since the last begin() call, reverting the database to the state before the transaction started. Returns true on success. Typically used in error-handling paths to undo partial changes.

**Parameters:**

- `conn` — A connection handle with an active transaction

**Returns:** true on success, or a Result::Err with an error string on failure

**Examples:**

```ntnt
rollback(db)  // => true  // Roll back the active transaction
```

**Errors:**

- **RuntimeError**: ROLLBACK failed: ... — *Fix: Ensure a transaction was started with begin() before rolling back*
- **RuntimeError**: Invalid or closed SQLite connection — *Fix: Use an open connection handle from connect()*
- **RuntimeError**: Failed to lock connection: ... — *Fix: Ensure connection is not used concurrently in conflicting ways*

**See also:** `begin`, `commit`

*Since v0.2.0*

---

## std/string

String manipulation: splitting, joining, trimming, searching, and transforming text

```ntnt
import { split, join, concat } from "std/string"
```

### Functions

| Function | Description |
|----------|-------------|
| [`capitalize`](#capitalize) | Capitalizes the first character, lowercases the rest. |
| [`capture_all_pattern`](#captureallpattern) | Returns all matches with capture groups. |
| [`capture_named_pattern`](#capturenamedpattern) | Returns first match with named capture groups as a map, or None if no match. |
| [`capture_pattern`](#capturepattern) | Returns first match with capture groups, or None if no match. |
| [`center`](#center) | Centers string with padding on both sides. |
| [`char_at`](#charat) | Returns character at index. |
| [`chars`](#chars) | Splits string into array of characters. |
| [`concat`](#concat) | Concatenates two strings, or joins an array of strings. |
| [`contains`](#contains) | Checks if string contains substring. |
| [`count`](#count) | Counts occurrences of substring. |
| [`ends_with`](#endswith) | Checks if string ends with suffix. |
| [`find_all_pattern`](#findallpattern) | Returns all regex matches. |
| [`find_pattern`](#findpattern) | Returns first regex match or None. |
| [`html_escape`](#htmlescape) | Escape HTML special characters in a string. |
| [`index_of`](#indexof) | Returns index of first occurrence, or -1 if not found. |
| [`is_alpha`](#isalpha) | Returns true if string contains only letters. |
| [`is_alphanumeric`](#isalphanumeric) | Returns true if string contains only letters and digits. |
| [`is_blank`](#isblank) | Returns true if string is empty or only whitespace. |
| [`is_empty`](#isempty) | Returns true if string is empty. |
| [`is_lowercase`](#islowercase) | Returns true if all letters are lowercase. |
| [`is_numeric`](#isnumeric) | Returns true if string contains only digits. |
| [`is_uppercase`](#isuppercase) | Returns true if all letters are uppercase. |
| [`is_whitespace`](#iswhitespace) | Returns true if string contains only whitespace. |
| [`join`](#join) | Joins array elements into a string with a delimiter. |
| [`keep_chars`](#keepchars) | Keeps only characters in the allowed set. |
| [`last_index_of`](#lastindexof) | Returns index of last occurrence, or -1 if not found. |
| [`lines`](#lines) | Splits string by newlines. |
| [`lower`](#lower) | Alias for to_lower. Converts string to lowercase. |
| [`matches`](#matches) | Simple glob matching with * and ? wildcards. |
| [`matches_pattern`](#matchespattern) | Checks if string matches regex pattern. |
| [`pad_left`](#padleft) | Pads string on the left to reach target length. |
| [`pad_right`](#padright) | Pads string on the right to reach target length. |
| [`remove_chars`](#removechars) | Removes all characters in the chars set. |
| [`repeat`](#repeat) | Repeats a string n times. |
| [`replace`](#replace) | Replaces all occurrences of from with to. |
| [`replace_all`](#replaceall) | Alias for replace. Replaces all occurrences of from with to. |
| [`replace_chars`](#replacechars) | Replaces any character in the chars set with replacement. |
| [`replace_first`](#replacefirst) | Replaces first occurrence of from with to. |
| [`replace_pattern`](#replacepattern) | Replaces all regex matches with replacement. |
| [`reverse`](#reverse) | Reverses a string. |
| [`slugify`](#slugify) | Converts to a URL-friendly slug. |
| [`split`](#split) | Splits a string into an array of substrings. |
| [`split_pattern`](#splitpattern) | Splits string by regex pattern. |
| [`starts_with`](#startswith) | Checks if string starts with prefix. |
| [`substring`](#substring) | Extracts substring from start to end (exclusive). |
| [`title`](#title) | Capitalizes the first letter of each word. |
| [`to_camel_case`](#tocamelcase) | Converts to camelCase. |
| [`to_kebab_case`](#tokebabcase) | Converts to kebab-case. |
| [`to_lower`](#tolower) | Converts string to lowercase. |
| [`to_pascal_case`](#topascalcase) | Converts to PascalCase. |
| [`to_snake_case`](#tosnakecase) | Converts to snake_case. |
| [`to_upper`](#toupper) | Converts string to uppercase. |
| [`trim`](#trim) | Removes leading and trailing whitespace. |
| [`trim_chars`](#trimchars) | Removes specified characters from both ends. |
| [`trim_end`](#trimend) | Alias for trim_right. Removes trailing whitespace. |
| [`trim_left`](#trimleft) | Removes leading whitespace. |
| [`trim_right`](#trimright) | Removes trailing whitespace. |
| [`trim_start`](#trimstart) | Alias for trim_left. Removes leading whitespace. |
| [`truncate`](#truncate) | Truncates string to max length with suffix. |
| [`upper`](#upper) | Alias for to_upper. Converts string to uppercase. |
| [`words`](#words) | Splits string by whitespace. |

#### `capitalize`

```ntnt
capitalize(s: String) -> String
```

Capitalizes the first character, lowercases the rest.

**Parameters:**

- `s` — The string to capitalize

**Examples:**

```ntnt
capitalize("hello")  // => "Hello"  // Capitalize first letter
```

**See also:** `title`, `to_upper`

*Since v0.1.0*

---

#### `capture_all_pattern`

```ntnt
capture_all_pattern(s: String, pattern: String) -> Array<Array<String>>
```

Returns all matches with capture groups.

Each inner array has the full match at index 0 followed by capture groups. Returns an empty array if there are no matches.

**Parameters:**

- `s` — The string to search within
- `pattern` — A regular expression pattern with capture groups

**Returns:** Array of arrays, each containing [full_match, group1, group2, ...]

**Examples:**

```ntnt
capture_all_pattern("2024-01 and 2025-02", r"(\d{4})-(\d{2})")  // => [["2024-01", "2024", "01"], ["2025-02", "2025", "02"]]  // Extract multiple date pairs
capture_all_pattern("no match", r"(\d+)")  // => []  // Returns empty array when no matches
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `capture_pattern`, `capture_named_pattern`, `find_all_pattern`

*Since v0.3.10*

---

#### `capture_named_pattern`

```ntnt
capture_named_pattern(s: String, pattern: String) -> Option<Map<String, String>>
```

Returns first match with named capture groups as a map, or None if no match.

Named groups become map keys. Unnamed groups use their numeric index as a string key ("0" for full match, "1", "2", etc.). Unmatched optional groups produce empty string values.

**Parameters:**

- `s` — The string to search within
- `pattern` — A regular expression pattern with named capture groups using (?P<name>...) syntax

**Returns:** Option containing map of group names to matched strings, or None

**Examples:**

```ntnt
capture_named_pattern("2024-01-15", r"(?P<year>\d{4})-(?P<month>\d{2})-(?P<day>\d{2})")  // => Some({"0": "2024-01-15", "year": "2024", "month": "01", "day": "15"})  // Named groups as map keys
capture_named_pattern("no match", r"(?P<num>\d+)")  // => None  // Returns None when no match
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `capture_pattern`, `capture_all_pattern`, `find_pattern`

*Since v0.3.10*

---

#### `capture_pattern`

```ntnt
capture_pattern(s: String, pattern: String) -> Option<Array<String>>
```

Returns first match with capture groups, or None if no match.

Returns an array where index 0 is the full match and subsequent indices are the capture groups. Unmatched optional groups produce empty strings.

**Parameters:**

- `s` — The string to search within
- `pattern` — A regular expression pattern with capture groups

**Returns:** Option containing array of [full_match, group1, group2, ...] or None

**Examples:**

```ntnt
capture_pattern("2024-01-15", r"(\d{4})-(\d{2})-(\d{2})")  // => Some(["2024-01-15", "2024", "01", "15"])  // Extract date parts
capture_pattern("no match", r"(\d+)")  // => None  // Returns None when no match
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `capture_all_pattern`, `capture_named_pattern`, `find_pattern`

*Since v0.3.10*

---

#### `center`

```ntnt
center(s: String, len: Int, char: String) -> String
```

Centers string with padding on both sides.

**Parameters:**

- `s` — The string to center
- `len` — The desired total length of the result
- `char` — The character to use for padding on both sides

**Examples:**

```ntnt
center("hi", 6, "-")  // => "--hi--"  // Center with dashes
```

**See also:** `pad_left`, `pad_right`

*Since v0.2.0*

---

#### `char_at`

```ntnt
char_at(s: String, index: Int) -> String
```

Returns character at index.

**Parameters:**

- `s` — The string to index into
- `index` — Zero-based position of the character to retrieve

**Examples:**

```ntnt
char_at("hello", 0)  // => "h"  // First character
```

**Errors:**

- **RuntimeError**: Index out of bounds — *Fix: Check string length with len() first*

**See also:** `substring`, `chars`

*Since v0.1.0*

---

#### `chars`

```ntnt
chars(s: String) -> Array<String>
```

Splits string into array of characters.

**Parameters:**

- `s` — The string to split into individual characters

**Examples:**

```ntnt
chars("hi")  // => ["h", "i"]  // Split into characters
```

**See also:** `split`, `char_at`

*Since v0.1.0*

---

#### `concat`

```ntnt
concat(a: String, b: String) -> String
```

Concatenates two strings, or joins an array of strings.

Also accepts an array as the first argument, in which case all elements are concatenated without a delimiter.

**Parameters:**

- `a` — The first string, or an array of strings to concatenate
- `b` — The second string to append

**Examples:**

```ntnt
concat("hello", " world")  // => "hello world"  // Concatenate two strings
```

**See also:** `join`, `split`

*Since v0.1.0*

---

#### `contains`

```ntnt
contains(s: String, substr: String) -> Bool
```

Checks if string contains substring.

**Parameters:**

- `s` — The string to search within
- `substr` — The substring to search for

**Examples:**

```ntnt
contains("hello", "ell")  // => true  // Substring found
```

**See also:** `starts_with`, `ends_with`, `index_of`

*Since v0.1.0*

---

#### `count`

```ntnt
count(s: String, substr: String) -> Int
```

Counts occurrences of substring.

**Parameters:**

- `s` — The string to search within
- `substr` — The substring to count occurrences of

**Examples:**

```ntnt
count("ababa", "a")  // => 3  // Count character occurrences
```

**See also:** `contains`, `index_of`

*Since v0.1.0*

---

#### `ends_with`

```ntnt
ends_with(s: String, suffix: String) -> Bool
```

Checks if string ends with suffix.

**Parameters:**

- `s` — The string to check
- `suffix` — The suffix to look for at the end of the string

**Examples:**

```ntnt
ends_with("hello", "lo")  // => true  // Suffix match
```

**See also:** `starts_with`, `contains`

*Since v0.1.0*

---

#### `find_all_pattern`

```ntnt
find_all_pattern(s: String, pattern: String) -> Array<String>
```

Returns all regex matches.

**Parameters:**

- `s` — The string to search within
- `pattern` — A regular expression pattern to find all occurrences of

**Examples:**

```ntnt
find_all_pattern("a1b2", "\\d")  // => ["1", "2"]  // Find all digits
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `find_pattern`, `matches_pattern`, `capture_all_pattern`

*Since v0.2.0*

---

#### `find_pattern`

```ntnt
find_pattern(s: String, pattern: String) -> Option<String>
```

Returns first regex match or None.

**Parameters:**

- `s` — The string to search within
- `pattern` — A regular expression pattern to find

**Examples:**

```ntnt
find_pattern("ab12cd", "\\d+")  // => Some("12")  // Find first digit sequence
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `find_all_pattern`, `matches_pattern`, `capture_pattern`

*Since v0.2.0*

---

#### `html_escape`

```ntnt
html_escape(s: String) -> String
```

Escape HTML special characters in a string.

Replaces `&`, `<`, `>`, `"`, and `'` with their HTML entity equivalents. Use this when inserting user-provided content into HTML outside of templates (templates auto-escape by default with `{{var}}`).

**Parameters:**

- `s` — The string to escape.

**Returns:** The escaped string safe for HTML insertion.

**Examples:**

```ntnt
html_escape("<script>alert('xss')</script>")  // => "&lt;script&gt;alert(&#x27;xss&#x27;)&lt;/script&gt;"  // Escape HTML
html_escape("hello & world")  // => "hello &amp; world"  // Escape ampersand
```

**See also:** `contains`, `replace_all`

*Since v0.3.0*

---

#### `index_of`

```ntnt
index_of(s: String, substr: String) -> Int
```

Returns index of first occurrence, or -1 if not found.

**Parameters:**

- `s` — The string to search within
- `substr` — The substring to find the first occurrence of

**Examples:**

```ntnt
index_of("hello", "l")  // => 2  // First occurrence index
```

**See also:** `last_index_of`, `contains`

*Since v0.1.0*

---

#### `is_alpha`

```ntnt
is_alpha(s: String) -> Bool
```

Returns true if string contains only letters.

**Parameters:**

- `s` — The string to check for alphabetic-only content

**Examples:**

```ntnt
is_alpha("abc")  // => true  // All letters
```

**See also:** `is_numeric`, `is_alphanumeric`

*Since v0.2.0*

---

#### `is_alphanumeric`

```ntnt
is_alphanumeric(s: String) -> Bool
```

Returns true if string contains only letters and digits.

**Parameters:**

- `s` — The string to check for alphanumeric-only content

**Examples:**

```ntnt
is_alphanumeric("abc123")  // => true  // Letters and digits
```

**See also:** `is_alpha`, `is_numeric`

*Since v0.2.0*

---

#### `is_blank`

```ntnt
is_blank(s: String) -> Bool
```

Returns true if string is empty or only whitespace.

**Parameters:**

- `s` — The string to check for blankness (empty or whitespace-only)

**Examples:**

```ntnt
is_blank("  ")  // => true  // Whitespace-only is blank
```

**See also:** `is_empty`, `is_whitespace`

*Since v0.2.0*

---

#### `is_empty`

```ntnt
is_empty(s: String) -> Bool
```

Returns true if string is empty.

**Parameters:**

- `s` — The string to check for emptiness

**Examples:**

```ntnt
is_empty("")  // => true  // Empty string
```

**See also:** `is_blank`

*Since v0.1.0*

---

#### `is_lowercase`

```ntnt
is_lowercase(s: String) -> Bool
```

Returns true if all letters are lowercase.

**Parameters:**

- `s` — The string to check for lowercase letters

**Examples:**

```ntnt
is_lowercase("hello")  // => true  // All lowercase
```

**See also:** `is_uppercase`, `to_lower`

*Since v0.2.0*

---

#### `is_numeric`

```ntnt
is_numeric(s: String) -> Bool
```

Returns true if string contains only digits.

**Parameters:**

- `s` — The string to check for numeric-only content

**Examples:**

```ntnt
is_numeric("123")  // => true  // All digits
```

**See also:** `is_alpha`, `is_alphanumeric`

*Since v0.2.0*

---

#### `is_uppercase`

```ntnt
is_uppercase(s: String) -> Bool
```

Returns true if all letters are uppercase.

**Parameters:**

- `s` — The string to check for uppercase letters

**Examples:**

```ntnt
is_uppercase("HELLO")  // => true  // All uppercase
```

**See also:** `is_lowercase`, `to_upper`

*Since v0.2.0*

---

#### `is_whitespace`

```ntnt
is_whitespace(s: String) -> Bool
```

Returns true if string contains only whitespace.

**Parameters:**

- `s` — The string to check for whitespace-only content

**Examples:**

```ntnt
is_whitespace("  \t")  // => true  // Spaces and tabs
```

**See also:** `is_blank`, `is_empty`

*Since v0.2.0*

---

#### `join`

```ntnt
join(arr: Array, delim: String) -> String
```

Joins array elements into a string with a delimiter.

Non-string elements are converted to their string representation.

**Parameters:**

- `arr` — The array of elements to join
- `delim` — The delimiter to insert between elements

**Examples:**

```ntnt
join(["a", "b"], ",")  // => "a,b"  // Join with comma
```

**See also:** `split`, `concat`

*Since v0.1.0*

---

#### `keep_chars`

```ntnt
keep_chars(s: String, allowed: String) -> String
```

Keeps only characters in the allowed set.

**Parameters:**

- `s` — The input string to filter
- `allowed` — The set of characters to keep (all others are removed)

**Examples:**

```ntnt
keep_chars("a1b2", "ab")  // => "ab"  // Keep only letters
```

**See also:** `remove_chars`, `replace_chars`

*Since v0.2.0*

---

#### `last_index_of`

```ntnt
last_index_of(s: String, substr: String) -> Int
```

Returns index of last occurrence, or -1 if not found.

**Parameters:**

- `s` — The string to search within
- `substr` — The substring to find the last occurrence of

**Examples:**

```ntnt
last_index_of("hello", "l")  // => 3  // Last occurrence index
```

**See also:** `index_of`, `contains`

*Since v0.1.0*

---

#### `lines`

```ntnt
lines(s: String) -> Array<String>
```

Splits string by newlines.

**Parameters:**

- `s` — The string to split into newline-delimited lines

**Examples:**

```ntnt
lines("a\nb")  // => ["a", "b"]  // Split by newline
```

**See also:** `split`, `words`

*Since v0.1.0*

---

#### `lower`

```ntnt
lower(s: String) -> String
```

Alias for to_lower. Converts string to lowercase.

**Parameters:**

- `s` — The string to convert to lowercase

**Examples:**

```ntnt
lower("HELLO")  // => "hello"  // Convert to lowercase
```

**See also:** `to_lower`, `upper`

*Since v0.2.0*

---

#### `matches`

```ntnt
matches(s: String, pattern: String) -> Bool
```

Simple glob matching with * and ? wildcards.

Uses glob-style patterns, not regular expressions. Use matches_pattern() for regex matching.

**Parameters:**

- `s` — The string to test against the pattern
- `pattern` — A glob pattern using * (any chars) and ? (single char) wildcards

**Examples:**

```ntnt
matches("hello.txt", "*.txt")  // => true  // Glob wildcard match
```

**See also:** `matches_pattern`, `contains`

*Since v0.2.0*

---

#### `matches_pattern`

```ntnt
matches_pattern(s: String, pattern: String) -> Bool
```

Checks if string matches regex pattern.

**Parameters:**

- `s` — The string to test
- `pattern` — A regular expression pattern to match against

**Examples:**

```ntnt
matches_pattern("test123", "\\d+")  // => true  // Contains digits
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `find_pattern`, `replace_pattern`, `matches`

*Since v0.2.0*

---

#### `pad_left`

```ntnt
pad_left(s: String, len: Int, char: String) -> String
```

Pads string on the left to reach target length.

**Parameters:**

- `s` — The string to pad
- `len` — The desired total length of the result
- `char` — The character to use for padding (first char used if multi-char)

**Examples:**

```ntnt
pad_left("5", 3, "0")  // => "005"  // Zero-pad a number
```

**See also:** `pad_right`, `center`

*Since v0.2.0*

---

#### `pad_right`

```ntnt
pad_right(s: String, len: Int, char: String) -> String
```

Pads string on the right to reach target length.

**Parameters:**

- `s` — The string to pad
- `len` — The desired total length of the result
- `char` — The character to use for padding (first char used if multi-char)

**Examples:**

```ntnt
pad_right("5", 3, "0")  // => "500"  // Pad right with zeros
```

**See also:** `pad_left`, `center`

*Since v0.2.0*

---

#### `remove_chars`

```ntnt
remove_chars(s: String, chars: String) -> String
```

Removes all characters in the chars set.

**Parameters:**

- `s` — The input string to remove characters from
- `chars` — The set of characters to remove (each character is matched individually)

**Examples:**

```ntnt
remove_chars("a!b?", "!?")  // => "ab"  // Remove punctuation
```

**See also:** `replace_chars`, `keep_chars`

*Since v0.2.0*

---

#### `repeat`

```ntnt
repeat(s: String, n: Int) -> String
```

Repeats a string n times.

**Parameters:**

- `s` — The string to repeat
- `n` — The number of times to repeat the string

**Examples:**

```ntnt
repeat("ab", 3)  // => "ababab"  // Repeat string three times
```

**Errors:**

- **RuntimeError**: repeat count must be non-negative — *Fix: Ensure n >= 0*

**See also:** `pad_left`, `pad_right`

*Since v0.1.0*

---

#### `replace`

```ntnt
replace(s: String, from: String, to: String) -> String
```

Replaces all occurrences of from with to.

**Parameters:**

- `s` — The input string to perform replacements on
- `from` — The substring to search for
- `to` — The replacement string for each occurrence

**Examples:**

```ntnt
replace("hello", "l", "L")  // => "heLLo"  // Replace all occurrences
```

**See also:** `replace_first`, `replace_chars`, `replace_pattern`, `replace_all`

*Since v0.1.0*

---

#### `replace_all`

```ntnt
replace_all(s: String, from: String, to: String) -> String
```

Alias for replace. Replaces all occurrences of from with to.

**Parameters:**

- `s` — The input string to perform replacements on
- `from` — The substring to search for
- `to` — The replacement string for each occurrence

**Examples:**

```ntnt
replace_all("hello", "l", "L")  // => "heLLo"  // Replace all occurrences
```

**See also:** `replace`

*Since v0.2.0*

---

#### `replace_chars`

```ntnt
replace_chars(s: String, chars: String, repl: String) -> String
```

Replaces any character in the chars set with replacement.

**Parameters:**

- `s` — The input string to perform replacements on
- `chars` — The set of characters to replace (each character is matched individually)
- `repl` — The replacement string for each matched character

**Examples:**

```ntnt
replace_chars("a.b_c", "._", "-")  // => "a-b-c"  // Replace characters from set
```

**See also:** `remove_chars`, `keep_chars`, `replace`

*Since v0.2.0*

---

#### `replace_first`

```ntnt
replace_first(s: String, from: String, to: String) -> String
```

Replaces first occurrence of from with to.

**Parameters:**

- `s` — The input string to perform the replacement on
- `from` — The substring to search for
- `to` — The replacement string for the first match

**Examples:**

```ntnt
replace_first("abab", "a", "X")  // => "Xbab"  // Replace first only
```

**See also:** `replace`, `replace_pattern`

*Since v0.1.0*

---

#### `replace_pattern`

```ntnt
replace_pattern(s: String, pattern: String, repl: String) -> String
```

Replaces all regex matches with replacement.

**Parameters:**

- `s` — The input string to search within
- `pattern` — A regular expression pattern to match against
- `repl` — The replacement string for each match

**Examples:**

```ntnt
replace_pattern("a1b2", "\\d", "X")  // => "aXbX"  // Replace digits with X
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `matches_pattern`, `find_pattern`, `split_pattern`, `capture_pattern`, `replace`

*Since v0.2.0*

---

#### `reverse`

```ntnt
reverse(s: String) -> String
```

Reverses a string.

**Parameters:**

- `s` — The string to reverse

**Examples:**

```ntnt
reverse("abc")  // => "cba"  // Reverse characters
```

*Since v0.1.0*

---

#### `slugify`

```ntnt
slugify(s: String) -> String
```

Converts to a URL-friendly slug.

Lowercases, replaces spaces and underscores with hyphens, removes non-alphanumeric characters, and collapses consecutive hyphens.

**Parameters:**

- `s` — The string to convert into a URL-friendly slug

**Examples:**

```ntnt
slugify("Hello World!")  // => "hello-world"  // URL-friendly slug
```

**See also:** `to_kebab_case`, `to_snake_case`

*Since v0.2.0*

---

#### `split`

```ntnt
split(s: String, delim: String) -> Array<String>
```

Splits a string into an array of substrings.

When the delimiter is not found, returns a single-element array containing the original string. An empty delimiter splits into individual characters.

**Parameters:**

- `s` — The string to split
- `delim` — The delimiter to split on

**Examples:**

```ntnt
split("a,b,c", ",")  // => ["a", "b", "c"]  // Basic comma-separated split
split("no-match", ",")  // => ["no-match"]  // No delimiter found returns original in array
```

**See also:** `join`, `chars`, `split_pattern`

*Since v0.1.0*

---

#### `split_pattern`

```ntnt
split_pattern(s: String, pattern: String) -> Array<String>
```

Splits string by regex pattern.

**Parameters:**

- `s` — The string to split
- `pattern` — A regular expression pattern to split on

**Examples:**

```ntnt
split_pattern("a1b2c", "\\d")  // => ["a", "b", "c"]  // Split on digits
```

**Errors:**

- **RuntimeError**: Invalid regex pattern — *Fix: Check regex syntax*

**See also:** `split`, `find_all_pattern`

*Since v0.2.0*

---

#### `starts_with`

```ntnt
starts_with(s: String, prefix: String) -> Bool
```

Checks if string starts with prefix.

**Parameters:**

- `s` — The string to check
- `prefix` — The prefix to test for

**Examples:**

```ntnt
starts_with("hello", "he")  // => true  // Prefix match
```

**See also:** `ends_with`, `contains`

*Since v0.1.0*

---

#### `substring`

```ntnt
substring(s: String, start: Int, end: Int) -> String
```

Extracts substring from start to end (exclusive).

**Parameters:**

- `s` — The source string to extract from
- `start` — The zero-based starting index (inclusive)
- `end` — The zero-based ending index (exclusive)

**Examples:**

```ntnt
substring("hello", 1, 4)  // => "ell"  // Extract middle portion
```

**Errors:**

- **RuntimeError**: Invalid substring range — *Fix: Ensure 0 <= start <= end <= len(s)*

**See also:** `char_at`, `chars`

*Since v0.1.0*

---

#### `title`

```ntnt
title(s: String) -> String
```

Capitalizes the first letter of each word.

**Parameters:**

- `s` — The string to convert to title case

**Examples:**

```ntnt
title("hello world")  // => "Hello World"  // Title case each word
```

**See also:** `capitalize`, `to_upper`

*Since v0.1.0*

---

#### `to_camel_case`

```ntnt
to_camel_case(s: String) -> String
```

Converts to camelCase.

**Parameters:**

- `s` — The string to convert to camelCase

**Examples:**

```ntnt
to_camel_case("hello_world")  // => "helloWorld"  // snake_case to camelCase
```

**See also:** `to_snake_case`, `to_pascal_case`, `to_kebab_case`

*Since v0.1.0*

---

#### `to_kebab_case`

```ntnt
to_kebab_case(s: String) -> String
```

Converts to kebab-case.

**Parameters:**

- `s` — The string to convert to kebab-case

**Examples:**

```ntnt
to_kebab_case("helloWorld")  // => "hello-world"  // camelCase to kebab-case
```

**See also:** `to_snake_case`, `to_camel_case`, `slugify`

*Since v0.1.0*

---

#### `to_lower`

```ntnt
to_lower(s: String) -> String
```

Converts string to lowercase.

**Parameters:**

- `s` — The string to convert to lowercase

**Examples:**

```ntnt
to_lower("HELLO")  // => "hello"  // Convert to lowercase
```

**See also:** `to_upper`, `capitalize`, `lower`

*Since v0.1.0*

---

#### `to_pascal_case`

```ntnt
to_pascal_case(s: String) -> String
```

Converts to PascalCase.

**Parameters:**

- `s` — The string to convert to PascalCase

**Examples:**

```ntnt
to_pascal_case("hello_world")  // => "HelloWorld"  // snake_case to PascalCase
```

**See also:** `to_snake_case`, `to_camel_case`, `to_kebab_case`

*Since v0.1.0*

---

#### `to_snake_case`

```ntnt
to_snake_case(s: String) -> String
```

Converts to snake_case.

**Parameters:**

- `s` — The string to convert to snake_case

**Examples:**

```ntnt
to_snake_case("helloWorld")  // => "hello_world"  // camelCase to snake_case
```

**See also:** `to_camel_case`, `to_pascal_case`, `to_kebab_case`

*Since v0.1.0*

---

#### `to_upper`

```ntnt
to_upper(s: String) -> String
```

Converts string to uppercase.

**Parameters:**

- `s` — The string to convert to uppercase

**Examples:**

```ntnt
to_upper("hello")  // => "HELLO"  // Convert to uppercase
```

**See also:** `to_lower`, `capitalize`, `upper`

*Since v0.1.0*

---

#### `trim`

```ntnt
trim(s: String) -> String
```

Removes leading and trailing whitespace.

**Parameters:**

- `s` — The string to trim

**Examples:**

```ntnt
trim("  hello  ")  // => "hello"  // Trim both ends
```

**See also:** `trim_left`, `trim_right`, `trim_chars`

*Since v0.1.0*

---

#### `trim_chars`

```ntnt
trim_chars(s: String, chars: String) -> String
```

Removes specified characters from both ends.

**Parameters:**

- `s` — The string to trim
- `chars` — The set of characters to remove from both ends

**Examples:**

```ntnt
trim_chars("-hello-", "-")  // => "hello"  // Trim specific character
```

**See also:** `trim`, `remove_chars`

*Since v0.1.0*

---

#### `trim_end`

```ntnt
trim_end(s: String) -> String
```

Alias for trim_right. Removes trailing whitespace.

**Parameters:**

- `s` — The string to trim from the right

**Examples:**

```ntnt
trim_end("hello  ")  // => "hello"  // Remove trailing spaces
```

**See also:** `trim_right`, `trim_start`

*Since v0.3.0*

---

#### `trim_left`

```ntnt
trim_left(s: String) -> String
```

Removes leading whitespace.

**Parameters:**

- `s` — The string to trim from the left

**Examples:**

```ntnt
trim_left("  hello")  // => "hello"  // Remove leading spaces
```

**See also:** `trim`, `trim_right`, `trim_start`

*Since v0.1.0*

---

#### `trim_right`

```ntnt
trim_right(s: String) -> String
```

Removes trailing whitespace.

**Parameters:**

- `s` — The string to trim from the right

**Examples:**

```ntnt
trim_right("hello  ")  // => "hello"  // Remove trailing spaces
```

**See also:** `trim`, `trim_left`, `trim_end`

*Since v0.1.0*

---

#### `trim_start`

```ntnt
trim_start(s: String) -> String
```

Alias for trim_left. Removes leading whitespace.

**Parameters:**

- `s` — The string to trim from the left

**Examples:**

```ntnt
trim_start("  hello")  // => "hello"  // Remove leading spaces
```

**See also:** `trim_left`, `trim_end`

*Since v0.3.0*

---

#### `truncate`

```ntnt
truncate(s: String, max_len: Int, suffix: String) -> String
```

Truncates string to max length with suffix.

If the string is shorter than max_len, it is returned unchanged.

**Parameters:**

- `s` — The string to truncate
- `max_len` — The maximum length of the resulting string (including suffix)
- `suffix` — The string to append when truncation occurs (e.g. "...")

**Examples:**

```ntnt
truncate("hello world", 8, "...")  // => "hello..."  // Truncate with ellipsis
```

**See also:** `substring`

*Since v0.2.0*

---

#### `upper`

```ntnt
upper(s: String) -> String
```

Alias for to_upper. Converts string to uppercase.

**Parameters:**

- `s` — The string to convert to uppercase

**Examples:**

```ntnt
upper("hello")  // => "HELLO"  // Convert to uppercase
```

**See also:** `to_upper`, `lower`

*Since v0.2.0*

---

#### `words`

```ntnt
words(s: String) -> Array<String>
```

Splits string by whitespace.

**Parameters:**

- `s` — The string to split into whitespace-delimited words

**Examples:**

```ntnt
words("hello world")  // => ["hello", "world"]  // Split by whitespace
```

**See also:** `split`, `lines`

*Since v0.1.0*

---

## std/time

Date, time, and duration operations

```ntnt
import { now, now_millis, now_nanos } from "std/time"
```

### Functions

| Function | Description |
|----------|-------------|
| [`add_days`](#adddays) | Adds days to a Unix timestamp. |
| [`add_hours`](#addhours) | Adds hours to a Unix timestamp. |
| [`add_minutes`](#addminutes) | Adds minutes to a Unix timestamp. |
| [`add_months`](#addmonths) | Adds months to a Unix timestamp with calendar-aware logic. |
| [`add_seconds`](#addseconds) | Adds seconds to a Unix timestamp. |
| [`add_weeks`](#addweeks) | Adds weeks to a Unix timestamp. |
| [`add_years`](#addyears) | Adds years to a Unix timestamp with calendar-aware logic. |
| [`after`](#after) | Checks whether the first timestamp is after the second. |
| [`before`](#before) | Checks whether the first timestamp is before the second. |
| [`day`](#day) | Extracts the day of the month from a Unix timestamp (UTC). |
| [`day_of_year`](#dayofyear) | Extracts the ordinal day of the year from a timestamp (UTC). |
| [`diff`](#diff) | Computes the difference between two timestamps. |
| [`duration_millis`](#durationmillis) | Creates a duration map from milliseconds (legacy utility). |
| [`duration_secs`](#durationsecs) | Creates a duration map from seconds (legacy utility). |
| [`elapsed`](#elapsed) | Returns the number of milliseconds elapsed since the given start time. |
| [`equal`](#equal) | Checks whether two timestamps are equal. |
| [`format`](#format) | Formats a Unix timestamp as a string using a strftime format pattern (UTC). |
| [`format_in`](#formatin) | Formats a Unix timestamp as a string in the specified timezone. |
| [`format_timestamp`](#formattimestamp) | Formats a Unix timestamp as a string (legacy alias for format()). |
| [`hour`](#hour) | Extracts the hour from a Unix timestamp (UTC). |
| [`is_leap_year`](#isleapyear) | Checks whether the year of the given timestamp is a leap year. |
| [`list_timezones`](#listtimezones) | Returns a list of commonly used IANA timezone identifiers. |
| [`make_date`](#makedate) | Creates a Unix timestamp for midnight UTC from date components. |
| [`make_time`](#maketime) | Creates a Unix timestamp from individual date and time components (UTC). |
| [`minute`](#minute) | Extracts the minute from a Unix timestamp (UTC). |
| [`month`](#month) | Extracts the month from a Unix timestamp (UTC). |
| [`month_name`](#monthname) | Returns the full English name of the month for a timestamp (UTC). |
| [`now`](#now) | Returns the current Unix timestamp in seconds (UTC). |
| [`now_millis`](#nowmillis) | Returns the current Unix timestamp in milliseconds. |
| [`now_nanos`](#nownanos) | Returns the current Unix timestamp in nanoseconds. |
| [`parse_datetime`](#parsedatetime) | Parses a date/time string into a Unix timestamp using the given format. |
| [`parse_iso`](#parseiso) | Parses an ISO 8601 (RFC 3339) string into a Unix timestamp. |
| [`second`](#second) | Extracts the second from a Unix timestamp (UTC). |
| [`sleep`](#sleep) | Pauses execution for the specified number of milliseconds. |
| [`to_iso`](#toiso) | Formats a Unix timestamp as an ISO 8601 (RFC 3339) string. |
| [`to_timezone`](#totimezone) | Converts a Unix timestamp to a datetime map in the specified timezone. |
| [`to_utc`](#toutc) | Converts a Unix timestamp to a UTC datetime map. |
| [`weekday`](#weekday) | Extracts the day of the week from a Unix timestamp (UTC). |
| [`weekday_name`](#weekdayname) | Returns the full English name of the weekday for a timestamp (UTC). |
| [`year`](#year) | Extracts the year from a Unix timestamp (UTC). |

#### `add_days`

```ntnt
add_days(timestamp: Int, days: Int) -> Int
```

Adds days to a Unix timestamp.

Multiplies days by 86400 and adds to the timestamp. Use negative values to subtract. Not calendar-aware (always 86400 seconds per day). For DST-sensitive calculations, use to_timezone() with manual adjustment.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `days` — Number of days to add (negative to subtract)

**Returns:** New Unix timestamp with days added

**Examples:**

```ntnt
add_days(0, 1)  // => 86400  // Add 1 day to epoch
```

**Errors:**

- **TypeError**: add_days() requires (timestamp: Int, days: Int) — *Fix: Pass two Int arguments*

**See also:** `add_seconds`, `add_hours`, `add_weeks`, `add_months`, `diff`

*Since v0.1.0*

---

#### `add_hours`

```ntnt
add_hours(timestamp: Int, hours: Int) -> Int
```

Adds hours to a Unix timestamp.

Multiplies hours by 3600 and adds to the timestamp. Use negative values to subtract.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `hours` — Number of hours to add (negative to subtract)

**Returns:** New Unix timestamp with hours added

**Examples:**

```ntnt
add_hours(0, 2)  // => 7200  // Add 2 hours to epoch
```

**Errors:**

- **TypeError**: add_hours() requires (timestamp: Int, hours: Int) — *Fix: Pass two Int arguments*

**See also:** `add_seconds`, `add_minutes`, `add_days`, `add_weeks`, `diff`

*Since v0.1.0*

---

#### `add_minutes`

```ntnt
add_minutes(timestamp: Int, minutes: Int) -> Int
```

Adds minutes to a Unix timestamp.

Multiplies minutes by 60 and adds to the timestamp. Use negative values to subtract.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `minutes` — Number of minutes to add (negative to subtract)

**Returns:** New Unix timestamp with minutes added

**Examples:**

```ntnt
add_minutes(0, 5)  // => 300  // Add 5 minutes to epoch
```

**Errors:**

- **TypeError**: add_minutes() requires (timestamp: Int, minutes: Int) — *Fix: Pass two Int arguments*

**See also:** `add_seconds`, `add_hours`, `add_days`, `add_weeks`, `diff`

*Since v0.1.0*

---

#### `add_months`

```ntnt
add_months(timestamp: Int, months: Int) -> Int
```

Adds months to a Unix timestamp with calendar-aware logic.

Properly handles month boundaries and varying month lengths. For example, Jan 31 + 1 month = Feb 28 (or 29 in a leap year). Preserves the time-of-day component.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `months` — Number of months to add (negative to subtract)

**Returns:** New Unix timestamp with months added

**Examples:**

```ntnt
add_months(0, 1)  // => 2678400  // Add 1 month to epoch (Jan->Feb)
```

**Errors:**

- **TypeError**: add_months() requires (timestamp: Int, months: Int) — *Fix: Pass two Int arguments*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*
- **RuntimeError**: Invalid date after month addition — *Fix: The resulting date is out of representable range*

**See also:** `add_years`, `add_days`, `add_weeks`, `diff`

*Since v0.1.0*

---

#### `add_seconds`

```ntnt
add_seconds(timestamp: Int, seconds: Int) -> Int
```

Adds seconds to a Unix timestamp.

Simple arithmetic addition. Use negative values to subtract.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `seconds` — Number of seconds to add (negative to subtract)

**Returns:** New Unix timestamp with seconds added

**Examples:**

```ntnt
add_seconds(0, 60)  // => 60  // Add 60 seconds to epoch
```

**Errors:**

- **TypeError**: add_seconds() requires (timestamp: Int, seconds: Int) — *Fix: Pass two Int arguments*

**See also:** `add_minutes`, `add_hours`, `add_days`, `add_weeks`, `diff`

*Since v0.1.0*

---

#### `add_weeks`

```ntnt
add_weeks(timestamp: Int, weeks: Int) -> Int
```

Adds weeks to a Unix timestamp.

Multiplies weeks by 604800 (7 * 86400) and adds to the timestamp. Use negative values to subtract.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `weeks` — Number of weeks to add (negative to subtract)

**Returns:** New Unix timestamp with weeks added

**Examples:**

```ntnt
add_weeks(0, 1)  // => 604800  // Add 1 week to epoch
```

**Errors:**

- **TypeError**: add_weeks() requires (timestamp: Int, weeks: Int) — *Fix: Pass two Int arguments*

**See also:** `add_days`, `add_months`, `add_years`, `diff`

*Since v0.1.0*

---

#### `add_years`

```ntnt
add_years(timestamp: Int, years: Int) -> Int
```

Adds years to a Unix timestamp with calendar-aware logic.

Properly handles leap year boundaries. For example, Feb 29 of a leap year + 1 year = Feb 28 (non-leap year). Preserves the time-of-day component.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `years` — Number of years to add (negative to subtract)

**Returns:** New Unix timestamp with years added

**Examples:**

```ntnt
add_years(0, 1)  // => 31536000  // Add 1 year to epoch
```

**Errors:**

- **TypeError**: add_years() requires (timestamp: Int, years: Int) — *Fix: Pass two Int arguments*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*
- **RuntimeError**: Invalid date after year addition — *Fix: The resulting date is out of representable range*

**See also:** `add_months`, `add_days`, `is_leap_year`, `diff`

*Since v0.1.0*

---

#### `after`

```ntnt
after(timestamp1: Int, timestamp2: Int) -> Bool
```

Checks whether the first timestamp is after the second.

Returns true if timestamp1 > timestamp2.

**Parameters:**

- `timestamp1` — First Unix timestamp in seconds
- `timestamp2` — Second Unix timestamp in seconds

**Returns:** true if timestamp1 is later than timestamp2

**Examples:**

```ntnt
after(86400, 0)  // => true  // Day 1 is after epoch
after(0, 86400)  // => false  // Epoch is not after day 1
```

**Errors:**

- **TypeError**: after() requires two timestamps — *Fix: Pass two Int arguments*

**See also:** `before`, `equal`, `diff`

*Since v0.1.0*

---

#### `before`

```ntnt
before(timestamp1: Int, timestamp2: Int) -> Bool
```

Checks whether the first timestamp is before the second.

Returns true if timestamp1 < timestamp2.

**Parameters:**

- `timestamp1` — First Unix timestamp in seconds
- `timestamp2` — Second Unix timestamp in seconds

**Returns:** true if timestamp1 is earlier than timestamp2

**Examples:**

```ntnt
before(0, 86400)  // => true  // Epoch is before day 1
before(86400, 0)  // => false  // Day 1 is not before epoch
```

**Errors:**

- **TypeError**: before() requires two timestamps — *Fix: Pass two Int arguments*

**See also:** `after`, `equal`, `diff`

*Since v0.1.0*

---

#### `day`

```ntnt
day(timestamp: Int) -> Int
```

Extracts the day of the month from a Unix timestamp (UTC).

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The day as an integer (1-31)

**Examples:**

```ntnt
day(0)  // => 1  // Epoch is the 1st
```

**Errors:**

- **TypeError**: day() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `year`, `month`, `day_of_year`, `weekday`, `to_utc`

*Since v0.1.0*

---

#### `day_of_year`

```ntnt
day_of_year(timestamp: Int) -> Int
```

Extracts the ordinal day of the year from a timestamp (UTC).

Returns the day number within the year (1-366).

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The ordinal day of the year (1-366)

**Examples:**

```ntnt
day_of_year(0)  // => 1  // Epoch is day 1 of the year
```

**Errors:**

- **TypeError**: day_of_year() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `day`, `weekday`, `is_leap_year`

*Since v0.1.0*

---

#### `diff`

```ntnt
diff(timestamp1: Int, timestamp2: Int) -> Map
```

Computes the difference between two timestamps.

Returns a map with the difference expressed in multiple units: { seconds, minutes, hours, days }. The result is timestamp1 - timestamp2, so it is positive when timestamp1 is later.

**Parameters:**

- `timestamp1` — First Unix timestamp in seconds
- `timestamp2` — Second Unix timestamp in seconds

**Returns:** Map with keys: seconds, minutes, hours, days (all Int)

**Examples:**

```ntnt
diff(86400, 0).days  // => 1  // One day difference
diff(3600, 0).hours  // => 1  // One hour difference
```

**Errors:**

- **TypeError**: diff() requires two timestamps — *Fix: Pass two Int arguments*

**See also:** `before`, `after`, `equal`

*Since v0.1.0*

---

#### `duration_millis`

```ntnt
duration_millis(milliseconds: Int) -> Map
```

Creates a duration map from milliseconds (legacy utility).

Converts a duration in milliseconds to a map with keys: secs, millis, nanos. Retained for backward compatibility. Prefer using the SECOND/MINUTE/HOUR constants with arithmetic in new code.

**Parameters:**

- `milliseconds` — Duration in milliseconds

**Returns:** Map with keys: secs (Int), millis (Int), nanos (Int)

**Examples:**

```ntnt
duration_millis(1000).secs  // => 1  // 1000 milliseconds = 1 second
duration_millis(500).nanos  // => 500000000  // 500ms in nanoseconds
```

**Errors:**

- **TypeError**: duration_millis() requires an integer — *Fix: Pass an Int value*

**See also:** `duration_secs`, `SECOND`, `MINUTE`, `HOUR`

*Since v0.1.0*

---

#### `duration_secs`

```ntnt
duration_secs(seconds: Int) -> Map
```

Creates a duration map from seconds (legacy utility).

Converts a duration in seconds to a map with keys: secs, millis, nanos. Retained for backward compatibility. Prefer using the SECOND/MINUTE/HOUR constants with arithmetic in new code.

**Parameters:**

- `seconds` — Duration in seconds

**Returns:** Map with keys: secs (Int), millis (Int), nanos (Int)

**Examples:**

```ntnt
duration_secs(1).millis  // => 1000  // 1 second = 1000 milliseconds
duration_secs(1).nanos  // => 1000000000  // 1 second = 1 billion nanoseconds
```

**Errors:**

- **TypeError**: duration_secs() requires an integer — *Fix: Pass an Int value*

**See also:** `duration_millis`, `SECOND`, `MINUTE`, `HOUR`

*Since v0.1.0*

---

#### `elapsed`

```ntnt
elapsed(start_millis: Int) -> Int
```

Returns the number of milliseconds elapsed since the given start time.

Computes (now_millis() - start_millis). Useful for measuring execution time of code blocks.

**Parameters:**

- `start_millis` — A starting timestamp in milliseconds (from now_millis())

**Returns:** Milliseconds elapsed since start_millis

**Examples:**

```ntnt
elapsed(now_millis())  // => 0  // Elapsed time is near 0 when called immediately
```

**Errors:**

- **TypeError**: elapsed() requires a start timestamp — *Fix: Pass an Int value from now_millis()*

**See also:** `now_millis`, `now`, `sleep`

*Since v0.1.0*

---

#### `equal`

```ntnt
equal(timestamp1: Int, timestamp2: Int) -> Bool
```

Checks whether two timestamps are equal.

Returns true if both timestamps represent the same instant.

**Parameters:**

- `timestamp1` — First Unix timestamp in seconds
- `timestamp2` — Second Unix timestamp in seconds

**Returns:** true if both timestamps are the same value

**Examples:**

```ntnt
equal(0, 0)  // => true  // Same timestamps are equal
equal(0, 1)  // => false  // Different timestamps are not equal
```

**Errors:**

- **TypeError**: equal() requires two timestamps — *Fix: Pass two Int arguments*

**See also:** `before`, `after`, `diff`

*Since v0.1.0*

---

#### `format`

```ntnt
format(timestamp: Int, format_str: String) -> String
```

Formats a Unix timestamp as a string using a strftime format pattern (UTC).

Supports standard strftime directives: %Y %m %d %H %M %S %f %Z %z %a %A %b %B %j %U %W %w. To format in a specific timezone, use format_in() instead.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `format_str` — strftime-compatible format string

**Returns:** Formatted date/time string

**Examples:**

```ntnt
format(0, "%Y-%m-%d")  // => "1970-01-01"  // Epoch date formatted
```

**Errors:**

- **TypeError**: format() requires (timestamp: Int, format: String) — *Fix: Pass an Int timestamp and a String format*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `format_in`, `to_iso`, `format_timestamp`

*Since v0.1.0*

---

#### `format_in`

```ntnt
format_in(timestamp: Int, timezone: String, format_str: String) -> String
```

Formats a Unix timestamp as a string in the specified timezone.

Combines timezone conversion and formatting in a single call. Uses strftime directives: %Y %m %d %H %M %S %f %Z %z %a %A %b %B %j %U %W %w.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `timezone` — IANA timezone string (e.g., "America/New_York")
- `format_str` — strftime-compatible format string

**Returns:** Formatted date/time string in the given timezone

**Examples:**

```ntnt
format_in(0, "UTC", "%Y-%m-%d %H:%M:%S")  // => "1970-01-01 00:00:00"  // Epoch in UTC
```

**Errors:**

- **TypeError**: format_in() requires (timestamp: Int, timezone: String, format: String) — *Fix: Pass Int, String, String arguments*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*
- **RuntimeError**: Invalid timezone — *Fix: Use IANA format like 'America/New_York'*

**See also:** `format`, `to_timezone`, `to_iso`, `list_timezones`

*Since v0.1.0*

---

#### `format_timestamp`

```ntnt
format_timestamp(timestamp: Int, format_str: String) -> String
```

Formats a Unix timestamp as a string (legacy alias for format()).

Deprecated alias for format(). Retained for backward compatibility. Prefer using format() in new code.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `format_str` — strftime-compatible format string

**Returns:** Formatted date/time string

**Examples:**

```ntnt
format_timestamp(0, "%Y-%m-%d")  // => "1970-01-01"  // Epoch date formatted
```

**Errors:**

- **TypeError**: format_timestamp() requires int and format string — *Fix: Pass an Int timestamp and a String format*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `format`, `format_in`, `to_iso`

*Since v0.1.0*

---

#### `hour`

```ntnt
hour(timestamp: Int) -> Int
```

Extracts the hour from a Unix timestamp (UTC).

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The hour as an integer (0-23)

**Examples:**

```ntnt
hour(0)  // => 0  // Epoch is midnight
```

**Errors:**

- **TypeError**: hour() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `minute`, `second`, `year`, `month`, `day`, `to_utc`

*Since v0.1.0*

---

#### `is_leap_year`

```ntnt
is_leap_year(timestamp: Int) -> Bool
```

Checks whether the year of the given timestamp is a leap year.

Uses the standard Gregorian leap year rules: divisible by 4, except centuries unless also divisible by 400.

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** true if the timestamp falls in a leap year

**Examples:**

```ntnt
is_leap_year(0)  // => false  // 1970 is not a leap year
```

**Errors:**

- **TypeError**: is_leap_year() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `year`, `day_of_year`, `add_years`

*Since v0.1.0*

---

#### `list_timezones`

```ntnt
list_timezones() -> Array<String>
```

Returns a list of commonly used IANA timezone identifiers.

Provides a curated set of ~25 widely-used timezone strings that can be passed to to_timezone() and format_in(). Includes major cities across all continents.

**Returns:** Array of IANA timezone identifier strings

**Examples:**

```ntnt
list_timezones()[0]  // => "UTC"  // First timezone is UTC
```

**See also:** `to_timezone`, `format_in`

*Since v0.1.0*

---

#### `make_date`

```ntnt
make_date(year: Int, month: Int, day: Int) -> Result<Int, String>
```

Creates a Unix timestamp for midnight UTC from date components.

Shorthand for make_time(year, month, day, 0, 0, 0). Returns a Result so invalid dates are handled gracefully.

**Parameters:**

- `year` — The year (e.g., 2024)
- `month` — The month (1-12)
- `day` — The day (1-31)

**Returns:** Result containing the Unix timestamp at midnight UTC, or an error message on failure

**Examples:**

```ntnt
make_date(1970, 1, 1)  // => Result::Ok(0)  // Epoch date
```

**Errors:**

- **TypeError**: make_date() requires 3 integers: (year, month, day) — *Fix: Pass three Int arguments*

**See also:** `make_time`, `parse_datetime`, `year`, `month`, `day`

*Since v0.1.0*

---

#### `make_time`

```ntnt
make_time(year: Int, month: Int, day: Int, hour: Int, minute: Int, second: Int) -> Result<Int, String>
```

Creates a Unix timestamp from individual date and time components (UTC).

Validates the components and returns a Result. Invalid combinations (e.g., month 13 or day 32) produce an Err variant.

**Parameters:**

- `year` — The year (e.g., 2024)
- `month` — The month (1-12)
- `day` — The day (1-31)
- `hour` — The hour (0-23)
- `minute` — The minute (0-59)
- `second` — The second (0-59)

**Returns:** Result containing the Unix timestamp on success, or an error message on failure

**Examples:**

```ntnt
make_time(1970, 1, 1, 0, 0, 0)  // => Result::Ok(0)  // Epoch from components
```

**Errors:**

- **TypeError**: make_time() requires 6 integers: (year, month, day, hour, minute, second) — *Fix: Pass six Int arguments*

**See also:** `make_date`, `parse_datetime`, `to_utc`

*Since v0.1.0*

---

#### `minute`

```ntnt
minute(timestamp: Int) -> Int
```

Extracts the minute from a Unix timestamp (UTC).

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The minute as an integer (0-59)

**Examples:**

```ntnt
minute(0)  // => 0  // Epoch minute is 0
```

**Errors:**

- **TypeError**: minute() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `hour`, `second`, `to_utc`

*Since v0.1.0*

---

#### `month`

```ntnt
month(timestamp: Int) -> Int
```

Extracts the month from a Unix timestamp (UTC).

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The month as an integer (1-12)

**Examples:**

```ntnt
month(0)  // => 1  // Epoch is January
```

**Errors:**

- **TypeError**: month() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `year`, `day`, `month_name`, `to_utc`

*Since v0.1.0*

---

#### `month_name`

```ntnt
month_name(timestamp: Int) -> String
```

Returns the full English name of the month for a timestamp (UTC).

Returns one of: "January" through "December".

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** Full month name as a string

**Examples:**

```ntnt
month_name(0)  // => "January"  // Epoch is in January
```

**Errors:**

- **TypeError**: month_name() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `month`, `weekday_name`, `year`

*Since v0.1.0*

---

#### `now`

```ntnt
now() -> Int
```

Returns the current Unix timestamp in seconds (UTC).

Provides the current wall-clock time as a Unix epoch timestamp. Use now_millis() or now_nanos() for higher precision.

**Returns:** The current Unix timestamp in seconds

**Examples:**

```ntnt
now()  // => 1700000000  // Returns current Unix timestamp (value varies)
```

**See also:** `now_millis`, `now_nanos`, `elapsed`

*Since v0.1.0*

---

#### `now_millis`

```ntnt
now_millis() -> Int
```

Returns the current Unix timestamp in milliseconds.

Higher precision variant of now(). Useful for measuring elapsed time or generating unique-ish identifiers.

**Returns:** The current Unix timestamp in milliseconds

**Examples:**

```ntnt
now_millis()  // => 1700000000000  // Returns current timestamp in ms (value varies)
```

**See also:** `now`, `now_nanos`, `elapsed`

*Since v0.1.0*

---

#### `now_nanos`

```ntnt
now_nanos() -> Int
```

Returns the current Unix timestamp in nanoseconds.

Highest precision variant of now(). May fail if the timestamp is out of range for nanosecond representation.

**Returns:** The current Unix timestamp in nanoseconds

**Examples:**

```ntnt
now_nanos()  // => 1700000000000000000  // Returns current timestamp in ns (value varies)
```

**Errors:**

- **RuntimeError**: Timestamp out of range for nanoseconds — *Fix: Use now() or now_millis() for timestamps outside nanosecond range*

**See also:** `now`, `now_millis`

*Since v0.1.0*

---

#### `parse_datetime`

```ntnt
parse_datetime(date_str: String, format_str: String) -> Result<Int, String>
```

Parses a date/time string into a Unix timestamp using the given format.

Uses strftime format directives to parse the input string. Returns a Result so parsing failures are handled gracefully without raising exceptions. The parsed datetime is treated as UTC.

**Parameters:**

- `date_str` — The date/time string to parse
- `format_str` — strftime-compatible format string

**Returns:** Result containing the Unix timestamp on success, or an error message on failure

**Examples:**

```ntnt
parse_datetime("2024-01-01 00:00:00", "%Y-%m-%d %H:%M:%S")  // => Result::Ok(1704067200)  // Parsed to timestamp
```

**Errors:**

- **TypeError**: parse_datetime() requires (date_str: String, format: String) — *Fix: Pass two String arguments*

**See also:** `parse_iso`, `format`, `make_time`

*Since v0.1.0*

---

#### `parse_iso`

```ntnt
parse_iso(iso_str: String) -> Result<Int, String>
```

Parses an ISO 8601 (RFC 3339) string into a Unix timestamp.

Accepts standard ISO 8601 datetime strings with timezone offset. Returns a Result so parsing failures are handled gracefully.

**Parameters:**

- `iso_str` — An ISO 8601 / RFC 3339 formatted string

**Returns:** Result containing the Unix timestamp on success, or an error message on failure

**Examples:**

```ntnt
parse_iso("1970-01-01T00:00:00+00:00")  // => Result::Ok(0)  // Epoch parsed from ISO
```

**Errors:**

- **TypeError**: parse_iso() requires a string — *Fix: Pass a String argument*

**See also:** `to_iso`, `parse_datetime`

*Since v0.1.0*

---

#### `second`

```ntnt
second(timestamp: Int) -> Int
```

Extracts the second from a Unix timestamp (UTC).

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The second as an integer (0-59)

**Examples:**

```ntnt
second(0)  // => 0  // Epoch second is 0
```

**Errors:**

- **TypeError**: second() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `hour`, `minute`, `to_utc`

*Since v0.1.0*

---

#### `sleep`

```ntnt
sleep(millis: Int) -> Unit
```

Pauses execution for the specified number of milliseconds.

Blocks the current thread. Requires a non-negative value. Use sparingly in production code; primarily useful for testing, rate limiting, or animation delays.

**Parameters:**

- `millis` — Duration to sleep in milliseconds (must be >= 0)

**Returns:** Unit

**Examples:**

```ntnt
sleep(100)  // => Unit  // Pauses for 100ms
```

**Errors:**

- **TypeError**: sleep() requires an integer (milliseconds) — *Fix: Pass an Int value*
- **RuntimeError**: sleep() requires non-negative milliseconds — *Fix: Pass a value >= 0*

**See also:** `elapsed`, `now_millis`

*Since v0.1.0*

---

#### `to_iso`

```ntnt
to_iso(timestamp: Int) -> String
```

Formats a Unix timestamp as an ISO 8601 (RFC 3339) string.

Produces a standard ISO 8601 datetime string in UTC, suitable for APIs, JSON serialization, and interoperability.

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** ISO 8601 formatted string (e.g., "1970-01-01T00:00:00+00:00")

**Examples:**

```ntnt
to_iso(0)  // => "1970-01-01T00:00:00+00:00"  // Epoch as ISO 8601
```

**Errors:**

- **TypeError**: to_iso() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `parse_iso`, `format`, `format_in`

*Since v0.1.0*

---

#### `to_timezone`

```ntnt
to_timezone(timestamp: Int, timezone: String) -> Map
```

Converts a Unix timestamp to a datetime map in the specified timezone.

Returns a map with keys: year, month, day, hour, minute, second, nanosecond, weekday, day_of_year, timestamp, timezone, offset.

**Parameters:**

- `timestamp` — Unix timestamp in seconds
- `timezone` — IANA timezone string (e.g., "America/New_York")

**Returns:** Map with datetime components in the given timezone

**Examples:**

```ntnt
to_timezone(0, "UTC").year  // => 1970  // Epoch in UTC is 1970
```

**Errors:**

- **TypeError**: to_timezone() requires (timestamp: Int, timezone: String) — *Fix: Pass an Int timestamp and a String timezone*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*
- **RuntimeError**: Invalid timezone — *Fix: Use IANA format like 'America/New_York'*

**See also:** `to_utc`, `format_in`, `list_timezones`

*Since v0.1.0*

---

#### `to_utc`

```ntnt
to_utc(timestamp: Int) -> Map
```

Converts a Unix timestamp to a UTC datetime map.

Returns a map with keys: year, month, day, hour, minute, second, nanosecond, weekday, day_of_year, timestamp, timezone, offset.

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** Map with datetime components in UTC

**Examples:**

```ntnt
to_utc(0).year  // => 1970  // Epoch is January 1, 1970
```

**Errors:**

- **TypeError**: to_utc() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `to_timezone`, `format`, `year`, `month`, `day`

*Since v0.1.0*

---

#### `weekday`

```ntnt
weekday(timestamp: Int) -> Int
```

Extracts the day of the week from a Unix timestamp (UTC).

Returns 0 for Sunday, 1 for Monday, through 6 for Saturday.

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The weekday as an integer (0=Sunday, 6=Saturday)

**Examples:**

```ntnt
weekday(0)  // => 4  // Epoch (Jan 1, 1970) was a Thursday
```

**Errors:**

- **TypeError**: weekday() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `weekday_name`, `day`, `day_of_year`

*Since v0.1.0*

---

#### `weekday_name`

```ntnt
weekday_name(timestamp: Int) -> String
```

Returns the full English name of the weekday for a timestamp (UTC).

Returns one of: "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday".

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** Full weekday name as a string

**Examples:**

```ntnt
weekday_name(0)  // => "Thursday"  // Epoch was a Thursday
```

**Errors:**

- **TypeError**: weekday_name() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `weekday`, `month_name`, `day`

*Since v0.1.0*

---

#### `year`

```ntnt
year(timestamp: Int) -> Int
```

Extracts the year from a Unix timestamp (UTC).

**Parameters:**

- `timestamp` — Unix timestamp in seconds

**Returns:** The year as an integer (e.g., 2024)

**Examples:**

```ntnt
year(0)  // => 1970  // Epoch year
```

**Errors:**

- **TypeError**: year() requires a timestamp — *Fix: Pass an Int timestamp*
- **RuntimeError**: Invalid timestamp — *Fix: Ensure the timestamp is a valid Unix epoch value*

**See also:** `month`, `day`, `hour`, `minute`, `second`, `to_utc`

*Since v0.1.0*

---

## std/url

URL parsing, encoding, and query string handling

```ntnt
import { parse_url, encode, encode_component } from "std/url"
```

### Functions

| Function | Description |
|----------|-------------|
| [`build_query`](#buildquery) | Builds a URL query string from a map of key-value pairs. |
| [`decode`](#decode) | URL-decodes a percent-encoded string. |
| [`encode`](#encode) | URL-encodes a string, preserving URL-safe characters. |
| [`encode_component`](#encodecomponent) | URL-encodes a string component aggressively, safe for query parameters. |
| [`join`](#join) | Deprecated: use join_url() instead. Alias for backward compatibility. |
| [`join_url`](#joinurl) | Joins a base URL with a path, handling trailing/leading slashes. |
| [`parse_query`](#parsequery) | Parses a URL query string into a map of key-value pairs. |
| [`parse_url`](#parseurl) | Parses a URL into its components: scheme, host, port, path, query, fragment. |

#### `build_query`

```ntnt
build_query(params: Map) -> String
```

Builds a URL query string from a map of key-value pairs.

Keys and values are URL-encoded using component encoding. Pairs are joined with & separators.

**Parameters:**

- `params` — Map of query parameter names to values

**Returns:** Query string like "key1=value1&key2=value2"

**Examples:**

```ntnt
build_query(map { "a": "1", "b": "2" })  // => "a=1&b=2"  // Map to query string
```

**See also:** `parse_query`, `encode_component`

*Since v0.2.0*

---

#### `decode`

```ntnt
decode(s: String) -> Result<String, String>
```

URL-decodes a percent-encoded string.

Converts %XX hex sequences back to characters and + signs to spaces. Returns Err if the string contains invalid percent encoding.

**Parameters:**

- `s` — The URL-encoded string to decode

**Returns:** Result containing the decoded string or an error

**Examples:**

```ntnt
decode("hello%20world")  // => Ok("hello world")  // Decode percent-encoded spaces
```

**See also:** `encode`, `encode_component`

*Since v0.2.0*

---

#### `encode`

```ntnt
encode(s: String) -> String
```

URL-encodes a string, preserving URL-safe characters.

Preserves characters that are safe in URLs (slashes, colons, etc.) while encoding spaces and other special characters. For encoding query parameter values, use encode_component instead.

**Parameters:**

- `s` — The string to encode

**Returns:** URL-encoded string

**Examples:**

```ntnt
encode("hello world")  // => "hello%20world"  // Encode spaces
```

**See also:** `decode`, `encode_component`

*Since v0.2.0*

---

#### `encode_component`

```ntnt
encode_component(s: String) -> String
```

URL-encodes a string component aggressively, safe for query parameters.

Unlike encode(), this encodes all non-alphanumeric characters except hyphens, underscores, periods, and tildes. Use this for query parameter keys and values.

**Parameters:**

- `s` — The string to encode

**Returns:** Aggressively URL-encoded string

**Examples:**

```ntnt
encode_component("a=b&c=d")  // => "a%3Db%26c%3Dd"  // Encode special chars
```

**See also:** `encode`, `decode`, `build_query`

*Since v0.2.0*

---

#### `join`

```ntnt
join(base: String, path: String) -> String
```

Deprecated: use join_url() instead. Alias for backward compatibility.

**Parameters:**

- `base` — The base URL
- `path` — The path to append

**Returns:** Combined URL string

**Examples:**

```ntnt
join("https://example.com", "/api")  // => "https://example.com/api"  // Deprecated: use join_url()
```

*Since v0.2.0*

---

#### `join_url`

```ntnt
join_url(base: String, path: String) -> String
```

Joins a base URL with a path, handling trailing/leading slashes.

Trims trailing slashes from the base and leading slashes from the path, then joins them with a single slash. Renamed from join() to avoid ambiguity with join() in std/string and std/path.

**Parameters:**

- `base` — The base URL
- `path` — The path to append

**Returns:** Combined URL string

**Examples:**

```ntnt
join_url("https://example.com", "/api/v1")  // => "https://example.com/api/v1"  // Join base and path
```

*Since v0.4.0*

---

#### `parse_query`

```ntnt
parse_query(query: String) -> Map<String, String>
```

Parses a URL query string into a map of key-value pairs.

Splits on & separators and = key-value delimiters. Both keys and values are URL-decoded. Keys without values get empty string values.

**Parameters:**

- `query` — The query string to parse (without leading ?)

**Returns:** Map of decoded query parameters

**Examples:**

```ntnt
parse_query("a=1&b=2")  // => map { "a": "1", "b": "2" }  // Query string to map
```

**See also:** `build_query`, `parse_url`

*Since v0.2.0*

---

#### `parse_url`

```ntnt
parse_url(url: String) -> Result<Map, String>
```

Parses a URL into its components: scheme, host, port, path, query, fragment.

Also extracts username/password from auth URLs and parses query parameters into a nested params map. The original URL is preserved as href.

**Parameters:**

- `url` — The URL string to parse

**Returns:** Result containing a map of URL components

**Examples:**

```ntnt
parse_url("https://example.com/path?q=1")  // Parse URL into components map
```

**Errors:**

- **TypeError**: parse() requires a URL string — *Fix: Pass a string*

**See also:** `build_query`, `parse_query`

*Since v0.2.0*

---

