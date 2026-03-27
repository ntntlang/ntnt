# DD-048: DX Simplification — Reduce Boilerplate in ntnt Apps

**Status:** draft  
**Author:** larri  
**Created:** 2026-03-25  

## Problem

Real ntnt apps contain significant boilerplate that wouldn't exist in PHP, Python, or Node.js equivalents. This friction compounds across every app.

**Evidence from snowgauge.app/server.tnt** (after free-win refactors already applied):

### 1. Stdlib Import Wall (10 lines)
```ntnt
import { fetch } from "std/http"
import { split, trim, contains } from "std/string"
import { html } from "std/http/server"
import { stringify, parse_json } from "std/json"
import { keys, push, reverse } from "std/collections"
import { now, now_millis, format } from "std/time"
import { get_env, load_env } from "std/env"
import { open, get, set, list } from "std/kv"
import { configure_queue, enqueue, enqueue_in, work_async, worker_status } from "std/jobs"
```

In PHP: no imports needed. In Python: `import json` gets you everything. In ntnt: every function from every module.

### 2. Redis Values Need Manual Parsing (4 lines per value)
```ntnt
let mut success_count = 0
match get(cache, "stats:success") {
    Ok(v) => { let s = str(v)   if len(s) > 0 && s != "none" { success_count = int(s) } },
    Err(_) => {}
}
```

In PHP: `$count = (int) $redis->get('stats:success');` — one line.

### 3. Cache JSON Read is Deeply Nested
```ntnt
match get(cache, "cache:site:#{site_param}") {
    Ok(cached_json) => {
        let cached_str = str(cached_json)
        if len(cached_str) > 2 {
            match parse_json(cached_str) {
                Ok(data) => { return render_dashboard(data, site_options) },
                Err(_) => { return show_loading(site_config, site_options) }
            }
        } else { return show_loading(site_config, site_options) }
    },
    Err(_) => { return show_loading(site_config, site_options) }
}
```

~12 lines to do "get JSON from cache or show loading."

## Proposals

### Proposal 1: Stdlib Prelude (Biggest Impact)

Auto-inject the most-used stdlib functions into every .tnt file — no import needed. Same as how `print`, `len`, `str`, `int`, `float`, `push` already work as builtins.

**Prelude candidates** (based on usage across all ntnt apps):

| Module | Functions |
|--------|-----------|
| `std/string` | `split`, `trim`, `contains`, `replace`, `join`, `starts_with`, `ends_with`, `to_lower`, `to_upper` |
| `std/json` | `parse_json`, `stringify` |
| `std/collections` | `keys`, `values`, `entries`, `has_key`, `get_key`, `reverse`, `sort` |
| `std/http/server` | `json`, `html`, `text`, `redirect`, `status`, `not_found`, `error`, `parse_form` |
| `std/env` | `get_env`, `load_env` |
| `std/time` | `now`, `format` |
| `std/crypto` | `uuid`, `sha256` |

**Result:** The 10-line import wall in snowgauge drops to 2 lines (just `std/kv` and `std/jobs` which are more specialized).

**Implementation:** In `define_builtins()` or a new `define_prelude()`, call `init_all_modules()` and inject the prelude set into the interpreter environment. Explicit imports still work — prelude just makes them unnecessary for common functions.

### Proposal 2: KV Type Helpers

Add typed getters to `std/kv`:

```ntnt
import { get_int, get_float, get_json, get_str } from "std/kv"

// Before (4 lines):
let mut count = 0
match get(cache, "stats:success") {
    Ok(v) => { let s = str(v)   if len(s) > 0 && s != "none" { count = int(s) } },
    Err(_) => {}
}

// After (1 line):
let count = get_int(cache, "stats:success", 0)
```

| Function | Signature | Description |
|----------|-----------|-------------|
| `get_int` | `get_int(store, key, default?) -> Int` | Get as integer, default on miss/error |
| `get_float` | `get_float(store, key, default?) -> Float` | Get as float |
| `get_json` | `get_json(store, key, default?) -> Any` | Get + JSON parse, default on miss/error |
| `get_str` | `get_str(store, key, default?) -> String` | Get as string, handles "none"/empty |

### Proposal 3: sort / sort_by

Add to `std/collections`:

```ntnt
let sorted = sort([3, 1, 2])                                    // [1, 2, 3]
let sorted = sort_by(entries, fn(a, b) { a["date"] > b["date"] })  // custom comparator
```

- `sort()` works on Int, Float, String arrays (natural ordering)
- `sort_by()` takes a comparator function

## Implementation Plan

### Phase 1: KV Type Helpers
- [ ] Add `get_int`, `get_float`, `get_json`, `get_str` to `src/stdlib/kv.rs`
- [ ] Each handles: Result unwrap → str conversion → "none"/empty check → type parse → default
- [ ] `// @ntnt` doc blocks, typechecker `sig!` entries, tests
- [ ] Refactor snowgauge to use them (validation)

### Phase 2: Stdlib Prelude
- [ ] Define prelude function list
- [ ] Add `define_prelude()` to interpreter, called after `define_stdlib()`
- [ ] Inject prelude functions into environment (same as builtins)
- [ ] Update `builtin_bindings` snapshot to include prelude
- [ ] Explicit imports still work (no breaking change)
- [ ] Refactor snowgauge: remove import lines covered by prelude

### Phase 3: sort / sort_by
- [ ] Add `sort(array)` to `std/collections` — natural ordering for Int/Float/String
- [ ] Add `sort_by(array, comparator)` — comparator returns Bool or Int
- [ ] Doc blocks, typechecker sigs, tests

## Success Criteria

**snowgauge server.tnt target:**
- Import lines: 10 → 2 (only specialized modules)
- Redis reads: 4 lines each → 1 line each
- Total: ~230 lines → ~150 lines
- Every line is business logic, not ceremony
