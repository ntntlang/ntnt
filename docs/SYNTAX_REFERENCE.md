# NTNT Syntax Reference

> **Auto-generated from [syntax.toml](syntax.toml)** - Do not edit directly.
>
> Last updated: v0.3.15

## Table of Contents

- [Keywords](#keywords)
- [Operators](#operators)
- [Literals](#literals)
- [Escape Sequences](#escape-sequences)
- [String Interpolation](#string-interpolation)
- [Template Strings](#template-strings)
- [Truthy/Falsy Values](#truthyfalsy-values)
- [Contracts](#contracts)
- [Types](#types)
- [Imports](#imports)
- [Match Expressions](#match-expressions)
- [Destructuring Patterns](#destructuring-patterns)
- [Function Parameters](#function-parameters)

---

## Keywords

Reserved words in the NTNT language

### Contracts

`requires`, `ensures`, `invariant`, `old`, `result`

_Design-by-contract keywords for specifying function behavior_

### Functions

`fn`, `return`

_Function definition and control_

### Variables

`let`, `mut`

_Variable declaration (mut for mutable)_

### Control Flow

`if`, `else`, `match`, `for`, `in`, `while`, `loop`, `break`, `continue`, `defer`

_Control flow statements_

### Error Handling

`otherwise`

_Inline error handling on let bindings — unwraps Ok/Some or runs a diverging block for Err/None_

### Types

`type`, `struct`, `enum`, `impl`, `trait`, `pub`, `self`

_Type system keywords_

### Modules

`import`, `from`, `export`

_Module system keywords_

### Literals

`true`, `false`, `map`, `Ok`, `Err`, `Some`, `None`

_Literal value keywords_

---

## Operators

NTNT operators by precedence (lowest to highest)

| Category | Operators | Description | Example |
|----------|-----------|-------------|----------|
| assignment | `=` | Assignment (requires `mut` variable) | `let mut x = 5; x = 10` |
| logical or | `||` | Logical OR (short-circuit) | `a || b` |
| logical and | `&&` | Logical AND (short-circuit) | `a && b` |
| comparison | `==`, `!=`, `<`, `>`, `<=`, `>=` | Comparison operators | `x == 5, y != 0, z < 10` |
| arithmetic | `+`, `-`, `*`, `/`, `%` | Arithmetic operators | `a + b, x * y, n % 2` |
| unary | `-`, `!` | Unary negation and logical NOT | `-x, !condition` |
| range | `..`, `..=` | Range operators (exclusive and inclusive) | `0..10 (0-9), 0..=10 (0-10)` |
| postfix | `?` | Try operator — unwraps Ok/Some or early-returns Err/None | `parse_json(req)?` |
| member | `.`, `[]` | Member access and indexing | `user.name, arr[0], map["key"]` |
| pipe | `|>` | Pipeline operator (passes left as first arg to right) | `data |> transform |> validate` |

---

## Literals

Value literal syntax

| Type | Syntax | Description |
|------|--------|-------------|
| integers | `42, -17, 0` | Integer literals (arbitrary precision) |
| floats | `3.14, 1.0e-10, -0.5` | Floating-point literals (IEEE 754) |
| strings | `"hello", "with {interpolation}"` | Double-quoted strings with escape sequences and interpolation |
| raw_strings | `r"no escapes", r#"with "quotes""#` | Raw strings - no escape processing, useful for regex patterns |
| template_strings | `"""...{{expr}}..."""` | Triple-quoted template strings with {{}} interpolation, loops, conditionals |
| booleans | `true, false` | Boolean literals |
| arrays | `[1, 2, 3], []` | Array literals |
| maps | `map { "key": value }` | Map literals (MUST use `map` keyword at top level) |
| ranges | `0..10, 0..=10` | Range literals (exclusive and inclusive) |
| closures | `fn(params) { body }` | Anonymous functions / closures in expression position |
| if_expression | `if cond { expr } else { expr }` | If-expression returns a value from the selected branch. Else is required. |

---

## Escape Sequences

Escape sequences in regular strings (not raw strings)

| Escape | Result |
|--------|--------|
| `\"` | Double quote |
| `\'` | Single quote |
| `\\` | Backslash |
| `\n` | Newline |
| `\r` | Carriage return |
| `\t` | Tab |
| `\{` | Literal { (prevents interpolation) |
| `\}` | Literal } |

---

## String Interpolation

String interpolation syntax

### Regular Strings

Syntax: `{expr}`

In regular strings, {expr} interpolates the expression

### Template Strings

Syntax: `{{expr}}`

In template strings, {{expr}} interpolates (single {} pass through for CSS)

---

## Template Strings

Template string (triple-quoted) features

| Feature | Syntax | Description |
|---------|--------|-------------|
| interpolation | `{{expr}}` | Interpolate any expression |
| filters | `{{expr \| filter}}` | Apply filter to expression |
| loops | `{{#for item in items}}...{{/for}}` | Loop over arrays |
| empty_fallback | `{{#for item in items}}...{{#empty}}...{{/for}}` | Fallback content when array is empty |
| conditionals | `{{#if cond}}...{{/if}}` | Conditional rendering |
| if_else | `{{#if cond}}...{{#else}}...{{/if}}` | If-else rendering |
| elif | `{{#if cond}}...{{#elif cond2}}...{{#else}}...{{/if}}` | Elif chains |
| comments | `{{! comment }}` | Template comments (not rendered) |
| escape_braces | `\{{ and \}}` | Literal {{ and }} in output |

### Available Filters

`uppercase`, `lowercase`, `capitalize`, `trim`, `truncate(n)`, `replace(old, new)`, `escape`, `raw`, `default(val)`, `length`, `first`, `last`, `reverse`, `join(sep)`, `slice(start, end)`, `json`, `number`, `url_encode`

### Loop Metadata Variables

- `@index (0-based)`
- `@length (total)`
- `@first (bool)`
- `@last (bool)`
- `@even (bool)`
- `@odd (bool)`

---

## Truthy/Falsy Values

Values that evaluate to true/false in conditionals

### Truthy

- `true`
- `Some(x)`
- `non-empty string`
- `non-empty array`
- `non-empty map`
- `ALL numbers (including 0)`

**Note:** 0 is truthy to avoid subtle bugs like `if count { }` failing when count is legitimately 0

### Falsy

- `false`
- `None`
- `"" (empty string)`
- `[] (empty array)`
- `map {} (empty map)`

---

## Contracts

Design-by-contract syntax for functions and structs

| Keyword | Syntax | Description |
|---------|--------|-------------|
| `requires` | `requires <condition>` | Precondition that must be true when function is called |
| `ensures` | `ensures <condition>` | Postcondition that must be true when function returns |
| `old` | `old(expr)` | Captures value of expression at function entry (for use in ensures) |
| `result` | `result` | Refers to the return value in ensures clauses |
| `invariant` | `invariant <condition>` | Struct invariant checked after construction and mutations |

### Placement

Contracts go AFTER return type, BEFORE function body

```ntnt
fn f(x: Int) -> Int
    requires x > 0
    ensures result > x
{
    return x + 1
}
```

---

## Types

Type system syntax

### PRIMITIVES

`Int`, `Float`, `Bool`, `String`, `Unit`

Built-in primitive types

### COMPOUND

`[T] (Array)`, `Map<K, V>`, `fn(T1, T2) -> T3`, `Range`

Compound types

### OPTION RESULT

`Option<T> (Some/None)`, `Result<T, E> (Ok/Err)`

Built-in sum types for optional values and error handling

| Function | Description | Example |
|----------|-------------|---------|
| `unwrap(x)` | Extract value from Some/Ok, panic on None/Err | `unwrap(Some(42)) → 42` |
| `unwrap_or(x, default)` | Extract value or return default | `unwrap_or(None, 0) → 0` |
| `is_some(x)` | Check if Option is Some | `is_some(Some(1)) → true` |
| `is_none(x)` | Check if Option is None | `is_none(None) → true` |
| `is_ok(x)` | Check if Result is Ok | `is_ok(Ok(1)) → true` |
| `is_err(x)` | Check if Result is Err | `is_err(Err("fail")) → true` |

### UNION

Syntax: `T1 | T2 | T3`

Union types for values that can be multiple types

### ANNOTATION

Syntax: `let x: Type = value`

Optional type annotations on variables

---

## Imports

Module import syntax

| Style | Syntax | Example |
|-------|--------|----------|
| named | `import { name1, name2 } from "module/path"` | `import { split, join } from "std/string"` |
| aliased | `import { name as alias } from "module/path"` | `import { fetch as http_fetch } from "std/http"` |
| namespace | `import "module/path" as name` | `import "std/math" as math` |
| local | `import { name } from "./relative/path"` | `import { helper } from "./lib/utils"` |

---

## Match Expressions

Pattern matching syntax

| Feature | Syntax | Description |
|---------|--------|-------------|
| basic | `match expr { pattern => result, ... }` | Match expression with patterns |
| guards | `pattern if condition => result` | Pattern with guard condition |
| wildcard | `_` | Wildcard pattern matches anything |
| binding | `name` | Bind matched value to name |

---

## Destructuring Patterns

| Pattern | Syntax | Description |
|---------|--------|-------------|
| map basic | `let { field1, field2 } = expr` | Extract map/struct fields into variables |
| map rename | `let { field: alias } = expr` | Extract map field with a different variable name |
| map nested | `let { field: { subfield } } = expr` | Nested map destructuring |
| array | `let [a, b, c] = expr` | Extract array elements into variables |
| array rest | `let [first, ...rest] = expr` | Extract leading elements and collect remaining into array |
| map rest | `let { field, ...rest } = expr` | Extract named fields and collect remaining into map |
| for loop | `for [a, b] in expr { }` | Destructure each element during iteration |
| spread token | `...` | Rest/spread operator in destructuring patterns |

---

## Function Parameters

Function parameter syntax

| Feature | Syntax | Description |
|---------|--------|-------------|
| basic | `fn name(a, b) { }` | Basic parameters (untyped, gradual typing) |
| typed | `fn name(a: Int, b: String) -> Bool { }` | Parameters with type annotations and return type |
| default values | `fn name(a, b = expr) { }` | Parameters with default values (must come after required params) |
| default typed | `fn name(a: Int, b: Int = 10) -> Int { }` | Default values with type annotations |
| default reference | `fn name(a = 0, b = a + 10) { }` | Default expressions can reference earlier parameters |

