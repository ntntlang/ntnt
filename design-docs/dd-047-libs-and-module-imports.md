# DD-047: `libs()` Builtin + Module-as-Namespace Imports

**Status:** draft  
**Author:** larri  
**Created:** 2026-03-25  
**Reviewed by:** Claude Code — 2026-03-25, Codex — 2026-03-25  

## Problem

Every app module import today requires listing every function individually:

```ntnt
import { SITES, STATION_IDS, TZ_OFFSET, LOG_TTL, CACHE_TTL, REFRESH_INTERVAL } from "./lib/config.tnt"
import { parse_snotel_name, parse_snotel_csv, calculate_snow_stats } from "./lib/snotel.tnt"
import { round_1dp, build_site_options, time_ago_str, format_count } from "./lib/helpers.tnt"
```

When you add a function to a lib file, you also have to update the import line in server.tnt. Hot reload picks up the new function, but it's not accessible until the import is manually updated. This is friction that slows down development.

## Proposal

Two complementary features:

### Feature 1: `libs()` — Auto-Import Directory (Zero Friction)

A new builtin that loads all `.tnt` files from a directory and injects their exports directly into the current scope:

```ntnt
libs("lib/")
```

**Behavior:**
- **Recursive scan** of the directory for `.tnt` files (skips hidden dirs, `node_modules`, `target` — same as `collect_tnt_files`)
- Evaluates each file in a fresh environment (same as `load_module_exports` today)
- Injects all exports **flat** into the caller's scope — no namespace prefix
- Files loaded in deterministic order (sorted by path)
- Hot reload watches all discovered files
- Subdirectories are included automatically — organize `lib/utils/`, `lib/models/`, etc. and everything loads

**After:**
```ntnt
libs("lib/")          // SITES, parse_snotel_csv, round_1dp, etc. all available
listen(8089)
```

Add `fn new_helper()` to `lib/helpers.tnt` or `lib/utils/new_file.tnt` → hot reload picks it up → call `new_helper()` from any route handler or server.tnt. No import line to touch.

**Name collisions:** If `lib/config.tnt` and `lib/helpers.tnt` both export `format`, last-loaded wins (sorted path order). In dev mode, emit:
```
[warn] lib/helpers.tnt overwrites 'format' from lib/config.tnt
```
This collision warning is **new code** — no existing pattern to reuse. Implementation adds a `seen_exports: HashMap<String, String>` (name → source file) during injection, warning when a name is overwritten.

**Missing directory:** `libs()` with a non-existent path is a **runtime error** with a clear message. This requires an explicit existence check before calling `collect_tnt_files` (which silently returns `Ok([])` for missing dirs). Same pattern as `load_jobs_from_directory`.

### Feature 2: Module-as-Namespace Import (Explicit, Qualified Access)

For when you want explicit namespacing without listing individual functions:

```ntnt
import config from "./lib/config.tnt"
import snotel from "./lib/snotel.tnt"

// Access via namespace
let sites = config.SITES
let stats = snotel.calculate_snow_stats(data)
```

**Behavior:**
- `import <name> from "<path>"` — single import, everything accessible via `<name>.X`
- The module is loaded once, cached (same as today's `loaded_modules`)
- The namespace binding is a `Value::Struct` with all exports as fields (same representation already used for `lib:` modules in route file injection)
- Hot reload tracks the file for changes

**Note:** This functionality already exists today via `import "./lib/config.tnt" as config`. Phase 2 adds the more natural `import config from "./lib/config.tnt"` syntax as sugar.

**⚠️ Parser fix required:** The current parser incorrectly handles `import config from "./lib/config.tnt"` — it parses `config` as a selective import item (like `import { config } from "..."`), which silently fails at runtime if no export named `config` exists. The parser must be updated to recognize this as a namespace import (`Import { items: [], source, alias: Some("config") }`), distinguishing it from the selective import case by lookahead (bare identifier followed by `from` keyword, vs `{` for selective imports).

## Hot-Reload Behavior

### With File-Based Routing (`routes()`)

Hot reload works fully. Route files are re-evaluated on each reload cycle, so they pick up updated `libs()` exports automatically.

### With Inline Routing (handlers defined in server.tnt)

**Limitation:** Closures capture values at definition time. If a route handler is defined as:
```ntnt
libs("lib/")
get("/users", fn(req) { format_count(len(users)) })
```
...the closure captures `format_count` when the `fn` expression is evaluated. Hot-reloading lib files will update the top-level environment, but won't update already-captured closure bindings.

**This is consistent with existing behavior** — server.tnt changes already require a restart. `libs()` inherits the same constraint. For apps that want full hot-reload, use file-based routing with `routes()`.

### Hot-Reload: New and Deleted Files

**Current limitation:** `check_and_reload_lib_modules()` only watches files it already knows about (tracked in `lib_module_files`). It does not rescan the directory, so:
- **New files** added to `lib/` after startup are never discovered
- **Deleted files** leave stale modules in `lib_modules` (metadata check fails silently)

**Required fix for `libs()`:** On each hot-reload cycle, **rescan the directory** (call `collect_tnt_files()` again) instead of only checking tracked mtimes. Compare the new file list against the tracked set:
- New files → load and inject
- Deleted files → remove from `lib_modules` and tracked set
- Changed files → re-evaluate (existing behavior)

This is new work beyond the current `check_and_reload_lib_modules` implementation.

### `libs()` + `routes()` Interaction

**⚠️ Important nuance:** `process_route_file()` creates a **fresh environment** for each route file and only injects namespaced Structs from `self.lib_modules` (e.g., `config.X`, `helpers.Y`). It does **not** inherit flat bindings from the server.tnt top-level environment.

This means with both `libs("lib/")` and `routes("routes/")`:
- **server.tnt** sees flat bindings: `SITES`, `round_1dp()`, etc.
- **Route files** see namespaced bindings only: `config.SITES`, `helpers.round_1dp()`, etc.
- Route files do **not** see `libs()` flat bindings unless we explicitly inject them

**Decision:** For Phase 1, `libs()` affects the server.tnt scope only. Route files continue using the existing namespaced injection from `routes()`. A future enhancement could add a `libs()` flat-injection path into route files, but this adds complexity and the namespace pattern is arguably better for route files anyway (explicit provenance).

To make route files also benefit from `libs()` recursive discovery (loading `lib/utils/*.tnt` etc.), the `load_file_based_routes()` lib discovery should be updated to use `collect_tnt_files()` (recursive) instead of its current flat `read_dir` scan. This is a separate improvement tracked in the implementation plan.

## Implementation Plan

### Phase 1: `libs()` Builtin

- [ ] Add `libs` to `define_server_actions()` in `interpreter.rs` (same pattern as `routes`, `jobs`)
- [ ] **Explicit directory existence check** before scanning (runtime error with clear message if missing)
- [ ] Implementation: call `collect_tnt_files()` on the directory (recursive), then `load_module_exports()` for each file
- [ ] Inject exports flat into the current environment (iterate each module's exports, `env.define(name, value)`)
- [ ] **Build collision warning from scratch:** Track `seen_exports: HashMap<String, String>` during injection, `eprintln!("[warn] {new_file} overwrites '{name}' from {old_file}")` in dev mode
- [ ] **Normalize module cache keys:** Add a helper that resolves import paths to canonical absolute paths. Use in both `handle_import()` cache lookup and `import_file_module()` cache storage. This prevents double evaluation when `libs()` and explicit `import` target the same file with different path strings.
- [ ] **Fix environment restore on error:** Both `load_module_exports()` and `import_file_module()` use `?` after `self.eval(&ast)`, which skips environment restore if evaluation fails. Change to `match` + restore pattern (same as the file context save/restore rule in the ntnt skill). Prevents environment leaks when `libs()` loads many files and one has a parse error.
- [ ] **Hot-reload with directory rescan:** Don't just check tracked file mtimes — rescan the directory on each cycle to detect new/deleted files. Compare against tracked set, add new files, remove deleted files, re-evaluate changed files. Track the `libs()` directory path separately from ad-hoc `lib_module_files`.
- [ ] **Update `load_file_based_routes()` lib discovery** to use `collect_tnt_files()` (recursive) instead of flat `read_dir`. This ensures route files see modules from `lib/utils/` subdirectories via namespaced access.
- [ ] `// @ntnt` doc block (NO `@module` — this is a server action builtin)
- [ ] Add typechecker signature via `sig!` macro
- [ ] Tests:
  - [ ] Basic: `libs("lib/")` makes functions callable
  - [ ] Recursive: `libs("lib/")` discovers files in `lib/utils/subfolder.tnt`
  - [ ] Collision warning: two files exporting same name → dev mode warning
  - [ ] Hot reload: modify file, verify new export available (file-routing path)
  - [ ] Hot reload: add new file to lib dir, verify discovered on next cycle
  - [ ] Hot reload: delete file from lib dir, verify stale module removed
  - [ ] Empty directory: no error, no exports
  - [ ] Missing directory: runtime error with clear message
  - [ ] Interaction with explicit `import { X } from "./lib/file.tnt"` (no conflict, both work)
  - [ ] No double evaluation when `libs()` and explicit `import` target the same file (cache key normalization)
  - [ ] Environment restored correctly when a lib file has a parse/eval error (other libs still work)
  - [ ] Route files see recursive lib modules via namespaced access after `load_file_based_routes` update

### Phase 2: Module-as-Namespace Import Syntax

- [ ] **Fix parser misparse:** `import config from "./lib/config.tnt"` currently parses as selective import `import { config } from "..."`. Add lookahead: bare identifier + `from` keyword → namespace import (`Import { items: [], alias: Some(ident), source }`)
- [ ] Verify `bind_imports` handles `items: []` + `alias: Some(name)` correctly (code path exists and works)
- [ ] Works for both stdlib and file modules: `import http from "std/http"`, `import config from "./lib/config.tnt"`
- [ ] Tests:
  - [ ] `import config from "./lib/config.tnt"` → `config.FIELD` access works
  - [ ] `import http from "std/http"` → `http.fetch(url)` works
  - [ ] Module cached after first import (no double evaluation)
  - [ ] Hot reload tracks the imported file
  - [ ] Verify `import "path" as alias` still works (existing syntax, must not regress)

### Phase 3: Wildcard Import (Optional — Low Priority)

- [ ] Parser: `import * from "./lib/config.tnt"` → new AST representation (distinguish from namespace import)
- [ ] **New `bind_imports` code path:** Current empty-items path always creates a Struct namespace. Wildcard needs flat injection — iterate all exports and `env.define()` each one. Distinguish via a `wildcard: bool` flag on the Import AST node, or a sentinel alias value.
- [ ] Collision warning if overwriting existing bindings
- [ ] Tests

## Interaction Between Features

All styles coexist. Use whichever fits:

| Style | Syntax | Scope | Use Case |
|-------|--------|-------|----------|
| **Auto-import all** | `libs("lib/")` | server.tnt (flat) | Most apps — zero friction, hot reload just works |
| **Namespace import** | `import config from "./lib/config.tnt"` | Anywhere | Large apps — avoid collisions, explicit provenance |
| **Namespace import (today)** | `import "./lib/config.tnt" as config` | Anywhere | **Already works** — same behavior, different syntax |
| **Selective import** | `import { X, Y } from "./lib/config.tnt"` | Anywhere | Existing style — still works, no changes needed |
| **Wildcard import** | `import * from "./lib/config.tnt"` | Anywhere | Phase 3 — flat injection from a single file |
| **Route file auto-inject** | Via `routes()` lib discovery | Route files only | Namespaced access (`config.X`) — updated to be recursive |

**Mixing is fine:** You could use `libs("lib/")` for server.tnt convenience and route files get the same modules via namespaced injection from `routes()`.

## Existing Infrastructure (What We Can Reuse)

| Component | Status | Notes |
|-----------|--------|-------|
| `load_module_exports()` | ✅ Exists | Evaluates a .tnt file, returns `HashMap<String, Value>`. Calls `define_stdlib()`, filters out NativeFunctions. **Does not cache** — caching must be added. |
| `lib_module_files` mtime tracking | ✅ Exists | Hot reload watches lib files. **Only tracks known files** — does not detect new/deleted files. |
| `check_and_reload_lib_modules()` | ⚠️ Needs extension | Re-evaluates changed lib files. Detects one changed file per cycle (breaks at first). Does not rescan directories. Does not handle new/deleted files. |
| `bind_imports()` with alias → Struct | ✅ Exists | Creates `Value::Struct` namespace from module exports |
| `collect_tnt_files()` dir scanner | ✅ Exists | **Recursive** — walks subdirectories. Skips hidden/node_modules/target. Returns `Ok([])` for missing dirs (need explicit existence check). |
| `define_server_actions()` pattern | ✅ Exists | How `routes()`, `jobs()`, `listen()` are registered |
| `import "path" as alias` syntax | ✅ Exists | Namespace imports work today with this syntax |
| `load_file_based_routes()` lib discovery | ⚠️ Flat only | Only scans immediate `lib/` directory for route injection. Needs update to use `collect_tnt_files()` for recursive discovery. |
| Module cache key normalization | ❌ New | `handle_import()` looks up by raw source string, `import_file_module()` caches by resolved path. Need canonical path helper to prevent double evaluation. |
| Environment restore on error | ❌ Fix needed | `load_module_exports()` and `import_file_module()` skip env restore if `eval()` fails (early `?`). Must use match + restore pattern. |
| Collision warning | ❌ New | No existing pattern — build from scratch with `seen_exports` HashMap |
| Parser for `import <name> from` | ❌ Fix needed | Current parser misinterprets as selective import — needs lookahead fix |
| Phase 3 flat injection in `bind_imports` | ❌ New | Current empty-items path creates Struct; wildcard needs new flat injection path |

## Migration

**No breaking changes.** Existing `import { X } from "./lib/file.tnt"` continues working exactly as before. `libs()` and module imports are purely additive.

Apps can migrate at their own pace:
```diff
-import { SITES, STATION_IDS, TZ_OFFSET, LOG_TTL, CACHE_TTL, REFRESH_INTERVAL } from "./lib/config.tnt"
-import { parse_snotel_name, parse_snotel_csv, calculate_snow_stats } from "./lib/snotel.tnt"
-import { round_1dp, build_site_options, time_ago_str, format_count } from "./lib/helpers.tnt"
+libs("lib/")
```

Users who want namespace imports today (before Phase 2) can already use:
```ntnt
import "./lib/config.tnt" as config
// config.SITES, config.CACHE_TTL, etc.
```

## Known Limitations (Hot-Reload)

The following edge cases in the hot-reload system are documented for a future hardening PR. They do not affect initial load behavior, only hot-reload during development.

1. **User binding override on reload** — If user code redefines a name that was originally injected by `libs()`, and then the lib file is modified/deleted, the hot-reload will `undefine()` the user's binding and re-inject the lib version (or remove it entirely). Root cause: no environment layering to track binding provenance. Fix requires a dedicated lib injection layer.

2. **Multiple `libs()` calls — collision order may drift** — While initial load preserves call order for collision resolution, the hot-reload path iterates `libs_flat_directories` in insertion order with per-dir file scan. If the same directory is registered by both `libs()` and `routes()`, the re-injection order may differ from initial load in edge cases.

3. **Hot-reload does not restore non-builtin shadows** — If a lib export shadows a user-defined global and the lib is later deleted, the user global is not restored (only builtins are restored from the snapshot). The user would need to restart the server.

**Recommended future work:** Implement environment layering — a dedicated "lib injection layer" that sits between builtins and user scope. Hot-reload swaps this layer atomically without touching user bindings. This eliminates all three issues structurally.
