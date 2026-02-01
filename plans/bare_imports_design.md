# Design: Import Error Quality — Collision Warnings + "Did You Mean?"

## Goal

Handle name collisions and import typos gracefully:

1. **Collision warnings** — When the same name is imported from two modules, warn at lint time with alias suggestions
2. **"Did you mean?"** — Suggest corrections for typos in module names and export names

## Non-goals

- ~~Bare imports without `from`~~ — Collisions make this ambiguous. Not worth forcing uniqueness.
- ~~Rename collection functions~~ — `push`, `reverse`, `concat` keep their natural names in both modules.
- ~~Build-time uniqueness enforcement~~ — Other languages allow collisions; so should NTNT.
- ~~Auto-resolve map~~ — Not needed without bare imports.
- ~~Unquoted module paths~~ — Saves two characters per import, not worth the file churn.
- ~~Type checker changes~~ — Marginal improvement, defer.

## Background

Three function names exist in multiple modules (`push`, `reverse`, `concat`). Currently, importing the same name twice silently shadows the first binding — `Environment::define()` does `HashMap::insert` with no check.

The Levenshtein distance infrastructure already exists in `src/error.rs:114-176` (`find_suggestion()`) but is only used for undefined variable/function errors, not import errors.

## Part 1: Import collision warnings in lint

**File:** `src/main.rs` (lint function, around line 2227)

Track imported names across the file. When the same name is imported from two different modules, emit a warning:

```
Warning: 'reverse' imported from both "std/string" (line 2) and "std/collections" (line 3)
  The second import shadows the first. Consider using an alias:
    import { reverse as reverse_alias } from "std/collections"
```

### Implementation

During lint's AST walk, maintain a `HashMap<String, (String, usize)>` mapping imported names to `(module, line_number)`. When a collision is detected, emit a warning with the alias suggestion.

## Part 2: "Did you mean?" suggestions for import errors

**Files:** `src/interpreter.rs` (import error paths)

Wire the existing `find_suggestion()` into the two import error paths:

### 2a: Wrong module name

**Current:** `"Unknown standard library module: std/sting"` (no suggestion)

**Target:**
```
Error[E005]: Unknown standard library module: std/sting
  Did you mean: std/string?
```

In `import_std_module()` (`interpreter.rs:2014-2025`), when the module isn't found, collect all module keys and call `find_suggestion()`.

### 2b: Wrong export name

**Current:** `"'spllit' is not exported from 'std/string'"` (no suggestion)

**Target:**
```
Error[E005]: 'spllit' is not exported from 'std/string'
  Did you mean: split?
  Available exports: split, join, trim, replace, ...
```

In `bind_imports()` (`interpreter.rs:2027-2065`), when the export isn't found, collect all export keys, call `find_suggestion()`, and list available exports.

## Files to modify

| File | Change |
|------|--------|
| `src/interpreter.rs` | Add "Did you mean?" to `import_std_module()` and `bind_imports()` error paths |
| `src/main.rs` | Add import collision detection to lint |

## Verification

1. `cargo build --profile dev-release` — compiles
2. `cargo test` — all tests pass
3. Verify collision warning: import `reverse` from both `std/string` and `std/collections`, run `ntnt lint`, see warning with alias suggestion
4. Verify "Did you mean?" for module typo: `import { split } from "std/sting"`, see suggestion
5. Verify "Did you mean?" for export typo: `import { spllit } from "std/string"`, see suggestion
6. Full test suite passes
