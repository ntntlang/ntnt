---
name: ntnt-core-dev
description: NTNT core language/runtime development in Rust. Use when working on the compiler, interpreter, stdlib, typechecker, or any Rust-level changes.
allowed-tools:
  - Read
  - Write
  - Edit
  - Bash
  - Glob
  - Grep
  - Agent
  - AskUserQuestion
---

# NTNT Core Development (Rust)

Working on the ntnt compiler/runtime Rust codebase (`~/repos/ntnt/`, `github.com/ntntlang/ntnt`).

## Core Language Philosophy (Non-Negotiable)

Every change must pass these filters:

1. **Simple & Intuitive** — A new user should guess what code does. Minimize surprise and ceremony.
2. **Strong & Robust** — Catch mistakes early, fail clearly, never silently corrupt. Handle edge cases gracefully.
3. **Consistent** — Same patterns everywhere. If `len()` works on strings and arrays, it should work on maps.
4. **Secure by Default** — Auto-escaping, SSRF protection, security headers, CSRF. Unsafe requires explicit opt-out.
5. **Progressive Type System** — Types optional but real. When present, enforced. Gradual adoption path.

**The test**: Would a junior developer building their first web app find this obvious? If not, fix the language, not the docs.

## Build

```bash
cargo build --profile dev-release   # Fast dev build (release speed, no LTO, incremental)
cargo build --release               # Distribution build
cargo test                          # Run all tests
```

## Source Structure

```
src/
├── main.rs                  # CLI entry (clap), lint, format_error(), doc generator
├── lexer.rs                 # Tokenizer
├── parser.rs                # Recursive descent → AST (Located wrapping on all declarations)
├── ast.rs                   # AST node definitions (Statement::Located for line tracking)
├── interpreter.rs           # Tree-walking evaluator + ALL global builtins (~251 tests)
├── config.rs                # TypeMode/LintMode enums, OnceLock caching, config resolution
├── contracts.rs             # requires/ensures/old()/invariant
├── typechecker.rs           # Static type checker — generics, type aliases, T? (~182 tests)
├── types.rs                 # Type definitions (Type::Named for generic params)
├── error.rs                 # Error types E001-E012, TypeContext, at_line() builder
├── intent.rs                # IDD module
├── intent_studio_server.rs  # Intent Studio HTML server
├── ial/                     # Intent Assertion Language engine
└── stdlib/                  # Standard library (21 modules, 357 functions)
    ├── mod.rs               # Module registry
    ├── auth.rs              # std/auth (sessions, CSRF, OAuth, Turnstile, API keys)
    ├── string.rs, math.rs, collections.rs, crypto.rs
    ├── http.rs              # std/http (client: fetch, download)
    ├── http_server.rs       # std/http/server (response builders)
    ├── http_server_async.rs # Axum + Tokio async server
    ├── http_bridge.rs       # Async↔sync bridge (mpsc + oneshot)
    ├── template.rs          # External template loading
    ├── postgres.rs, sqlite.rs, fs.rs, json.rs, csv.rs, kv.rs
    ├── env.rs, path.rs, time.rs, url.rs, concurrent.rs, log.rs, markdown.rs
```

## Compiler Pipeline

```
Source (.tnt) → Lexer (tokens) → Parser (AST) → Type Checker (diagnostics) → Config (TypeMode/LintMode) → Interpreter (execution)
```

- **Lexer**: String interpolation (`#{expr}`), raw strings, template strings, ranges
- **Parser**: Recursive descent, operator precedence, contracts, destructuring, pipe desugaring. All declarations wrapped in `Located` for line tracking.
- **Type Checker**: Two-pass (collect declarations, then check). Gradual typing. 350+ builtin/stdlib signatures. Real generics with type unification (`unify_type_params`/`substitute_type_params`).
- **Config**: `TypeMode` (Strict/Warn/Forgiving) + `LintMode` (Default/Warn/Strict). `OnceLock` caching in prod, per-call in test builds. CLI > env var > toml > default.
- **Interpreter**: Tree-walking. `Rc<RefCell<Environment>>` for lexical scoping. 7+ TypeMode-aware resilience points. Warning dedup via thread-local `HashSet`.

## HTTP Server Architecture

```
Tokio async (Axum) → mpsc channel → Single interpreter thread (Rc<RefCell>, not Send)
                   ← oneshot reply ←
```

Key files: `http_server_async.rs` (Axum), `http_bridge.rs` (channel types), `http_server.rs` (response builders). Hot-reload watches .tnt files, reloads on next request. Disabled in production.

## Testing

```bash
cargo test                              # All tests
cargo test --test language_features     # Integration tests
cargo test --test type_checker_tests    # Type checker tests
cargo test --test cli_tests             # CLI integration
cargo test -- test_name                 # Specific test
```

**Unit tests**: Inline `#[test]` — major concentrations in `interpreter.rs` (~160), `typechecker.rs` (~240), `intent.rs` (~31), `ial/` (~37).

**Integration tests** (`tests/`): `language_features_tests.rs`, `type_checker_tests.rs`, `cli_tests.rs`, `intent_studio_tests.rs`.

---

## Documentation System (`// @ntnt`)

Docs live as structured comments above `module.insert()` calls. `build.rs` validates 100% coverage at compile time — **undocumented functions fail the build**.

### Auto-Generated Docs — NEVER Hand-Edit

All reference docs in `docs/` are generated by `ntnt docs --generate`. Do not edit them directly — changes will be overwritten by CI.

**CI WILL FAIL if you:**
1. Manually edit `docs/STDLIB_REFERENCE.md`, `SYNTAX_REFERENCE.md`, `IAL_REFERENCE.md`, or `RUNTIME_REFERENCE.md` — edit the `@ntnt` source docstring in Rust instead
2. Skip `cargo fmt` before pushing — CI runs `rustfmt --check`
3. Commit generated files after editing source without running `ntnt docs --generate`

| Generated File | Source of Truth |
|---------------|-----------------|
| `STDLIB_REFERENCE.md` | `// @ntnt` annotations in Rust source |
| `SYNTAX_REFERENCE.md` | `docs/syntax.toml` |
| `IAL_REFERENCE.md` | `docs/ial.toml` |
| `RUNTIME_REFERENCE.md` | `docs/runtime.toml` |

The only hand-maintained doc *in `docs/`* is `AI_AGENT_GUIDE.md` — tutorial/cookbook with patterns and gotchas. `CLAUDE.md` and `.github/copilot-instructions.md` are also manually maintained (not auto-generated).

### syntax.toml Features

- `[types.<category>]` with `types`, `description`, `syntax`, `example` fields
- `[[types.<category>.functions]]` arrays — rendered as markdown tables
- `[operators]`, `[literals]`, `[keywords]`, `[templates]` sections

### Adding a Stdlib Function

1. Write `// @ntnt` doc block above `module.insert()`:
```rust
// @ntnt my_func
// @module std/string          // ← ONLY for stdlib modules, NOT for builtins
// @signature my_func(s: String) -> String
// Brief summary.
// @param s Input string
// @returns Transformed string
// @example my_func("hi") => "HI" ~ "Uppercase"
// @see_also related_func
// @since v0.4.0
module.insert("my_func".to_string(), Value::NativeFunction { ... });
```

**Builtins vs Stdlib:** Global builtins dispatched via server action table (`listen`, `routes`, `jobs`, `serve_static`) do NOT get `@module`. They're defined in `define_builtins()` or `define_server_actions()`, not in a stdlib module. Using `@module` on a builtin generates fake import syntax in docs that doesn't work.

2. `cargo build --profile dev-release` — fails if doc missing/orphaned
3. **MUST add typechecker signature** in `src/typechecker.rs` (search for `sig!` macro)
4. `ntnt docs --generate` to regenerate all reference docs + sync agent files
5. **Read the generated docs** for your new function — verify import syntax, examples, and description are correct from a user's perspective

### Doc Directives

| Directive | Required | Description |
|-----------|----------|-------------|
| `@ntnt <name>` | Yes | Must match function name |
| `@module <path>` | Stdlib only | e.g., `std/string` |
| `@signature <sig>` | Yes | Full typed signature |
| Summary lines | Yes | First non-@ lines |
| `@param <name> <desc>` | Per param | Parameter docs |
| `@example code => expected ~ "desc"` | Yes (1+) | Usage example |
| `@error Type ~ "msg" fix: "fix"` | No | Error conditions |
| `@see_also a, b` | No | Cross-references |

### Multi-Line Examples

```rust
// @example ~ "POST request with JSON body"
//   let opts = map { "url": "https://api.example.com", "method": "POST" }
//   fetch(opts)
// @expected Ok({status: 201, ...})
```

Continuation lines indented 2+ spaces. `// @expected` provides expected result. Block ends at next non-indented line.

---

## Writing Correct Code

These patterns are baked in from PR #36-#46 bugs. Write it right the first time.

### Locks: Match the Existing Pattern

Before writing any new `Mutex`/`RwLock` access, find an existing access to the SAME field and match its error handling exactly:
```bash
grep -n "job_registry" src/stdlib/jobs.rs | head -10
```
If existing code uses `.expect("message")`, yours does too. If it uses `.map_err()`, yours does too. **Never downgrade** from `.expect()` to `if let Ok(...)` — that silently swallows a poisoned lock on critical data.

### State Mutation: Always Ask "What If the Next Line Fails?"

Every time you modify shared state followed by an operation that can fail:

```rust
// WRONG — broken state on failure
clear_definitions();         // point of no return
reload_from_disk()?;         // if this fails, definitions are gone

// RIGHT — rollback on failure
let snapshot = snapshot_definitions();
clear_definitions();
match reload_from_disk() {
    Ok(v) => v,
    Err(e) => { restore_definitions(snapshot); return Err(e); }
}
```

**The rule:** If you mutate state, you MUST handle the failure path. Either snapshot+restore, or guarantee the mutation is harmless if the follow-up fails.

### File Context Save/Restore: All Three Paths

When temporarily changing interpreter state (`current_file`, `execution_mode`, etc.) to process sub-files, restore on ALL paths:

```rust
let previous_file = self.current_file.clone();
let mut error = None;

// ... do work, capture errors instead of ? ...

// ALWAYS restore (success, error, AND panic if using catch_unwind)
self.current_file = previous_file;

if let Some(e) = error { return Err(e); }
```

**Never use `?` between a state save and its restore** — it skips the restore. Accumulate errors in an `Option<Error>` and propagate after restoring.

### Doc Annotations: Match Existing Dispatch Style

- **Server actions** (global builtins via action table — `listen`, `routes`, `jobs`): Do NOT add `@module`.
- **Stdlib functions** (inside `module.insert()` — `trim`, `fetch`, `query`): DO add `@module std/whatever`.
- After adding any doc annotation, read the generated output in `STDLIB_REFERENCE.md` — does it show the right import syntax?

### Hot-Reload: Clear Before Load, Track File AND Dir Mtimes

1. **Clear stale state THEN reload** — `clear_job_definitions()` before `load_jobs_from_directory()`. Without clearing, `register_job()` is idempotent (first-registration wins), so updated perform bodies are silently ignored.
2. **Track file mtimes, not just directory mtimes** — editing a file doesn't change its parent directory's mtime on most filesystems.
3. **Snapshot before clear** — so you can restore on reload failure (see State Mutation above).

### Recursive File Collection: Always Exclude Non-Source Dirs

When scanning directories for `.tnt` files:
```rust
// Skip hidden dirs, node_modules, target
if dir_name.starts_with('.') || dir_name == "node_modules" || dir_name == "target" {
    continue;
}
```
Copy from the existing `collect_tnt_files` pattern. Don't reinvent it.

### Shared-Scope Evaluation: Warn on Name Collisions

When evaluating multiple files into the same interpreter scope (job files, route files), track which names each file defines and `eprintln!("[warn]")` when one file overwrites another's definition. Silent overwrites are how bugs hide for months.

### Tests: Cross-Platform, All Modes

- **Temp dirs**: Always `std::env::temp_dir()`, never `/tmp/`. Tests must work on macOS, Windows, and sandboxed CI.
- **Execution modes**: If a feature uses a capability gate, test in EVERY mode that has that capability. `jobs()` requires `JobConfig` — test in Normal, Worker, Job, UnitTest, and HotReload.
- **Reused interpreter tests**: If code runs on a reused interpreter (job workers), test sequential execution: success → success, error → success, panic → success. Verify no state leaks between runs.

### Examples in Docs: Verify Against Runtime

Import paths, function signatures, and examples in `AI_AGENT_GUIDE.md` and `@ntnt` doc blocks must actually work when run:
- `import "lib/x.tnt"` resolves relative to CWD. Use `"./lib/x.tnt"` for relative-to-current-file.
- `import { X } from "std/Y"` — verify the module name matches the actual stdlib module.
- If an example shows `jobs("jobs/")`, make sure a test verifies that exact string works.

---

## Adding a Language Feature (Checklist)

1. **Lexer**: Add tokens if needed (`lexer.rs`)
2. **AST**: Add node types (`ast.rs`) — wrap in `Located` if it's a declaration
3. **Parser**: Parse new syntax (`parser.rs`) — declarations go through `declaration()` which auto-wraps in `Located`
4. **Type Checker**: Add type rules + signatures (`typechecker.rs`) — use `sig!` macro for stdlib sigs
5. **Config**: If TypeMode-aware, check `get_type_mode()` from `config.rs` for strict/warn/forgiving behavior
6. **Interpreter**: Add evaluation logic (`interpreter.rs`) — use `IntentError::type_error(msg)` constructor (not `TypeError(String)`)
7. **Error types**: Add `line` field if new error variant. Use `TypeContext` for expected/got/hint messages.
8. **Tests**: Unit tests in source + integration tests in `tests/`
9. **Docs**: Update `docs/AI_AGENT_GUIDE.md`, run `ntnt docs --generate`
10. **Lint paths**: If new statement type, ensure `check_stmt_for_issues()` and `collect_used_names()` handle it (or `unwrap_located()` to see through `Located`)
11. **Roadmap**: Update `ROADMAP.md` checkboxes
12. **Pre-push**: Run the full Pre-Push Checklist

---

## Pre-Push Checklist (MANDATORY)

### During development (fast iteration)

```bash
cargo fmt                                          # Format
cargo build --profile dev-release                  # Fast dev build
cargo test --lib                                   # Unit tests (~1-2s)
cargo test --test language_features_tests --test type_checker_tests --test cli_tests  # Integration
```

### Before pushing (final validation)

```bash
# Release build — MUST happen before docs generate
# The docs generator reads from this binary. Stale binary = stale docs.
cargo build --release --locked

# Regenerate docs from the freshly built binary (CI drift-checks these)
./target/release/ntnt docs --generate

# Verify formatting is clean
cargo fmt -- --check

# Stage specific files and commit (NEVER use git add -A — risks adding secrets/binaries)
git add src/specific_file.rs docs/STDLIB_REFERENCE.md
git commit -m "..."
git push origin <branch>
```

**ORDER MATTERS**: `cargo build --release` MUST come before `ntnt docs --generate`. A stale binary generates stale docs, silently undoing your @ntnt docstring changes. This has caused repeated CI failures.

### Common mistakes

- **Two builds**: Don't run both `cargo build --locked` (debug) AND `cargo build --release --locked`. Use `dev-release` for iteration, `release` once before push.
- **`cargo fmt --all`**: The `--all` flag is for workspaces. NTNT is a single crate — just `cargo fmt`.
- **`git add -A`**: Dangerous — can stage .env files, binaries, or generated artifacts. Always add specific files.
- **`cargo test --locked`**: The `--locked` flag is for build, not test.
- **Skipping typechecker signatures**: Every new stdlib function needs a signature in `src/typechecker.rs`. Build won't catch this — lint will.

---

## CI/CD Workflows

### CI (`ci.yml`) — On push/PR to main

**Path-filtered:** Full CI (build/test/lint/examples/docs) only runs when Rust code, Cargo files, examples, or doc config change. Markdown-only changes skip the heavy jobs.

Pipeline: Test (ubuntu, macos, windows) → Build + Lint → Examples → Docs validate + drift check

### Release (`release.yml`) — On `v*` tags

Pipeline: Docs → Cross-platform builds (macos-arm64, linux-x64, windows-x64) → GitHub release with binaries + checksums

```bash
git tag v0.X.Y -a -m "v0.X.Y: description" && git push origin v0.X.Y
```

---

## Agent Instruction Sync

`docs/AI_AGENT_GUIDE.md` syncs to `.github/copilot-instructions.md` via `<!-- BEGIN/END NTNT CODING GUIDE -->` markers. `CLAUDE.md` is manually maintained (condensed version with references).

Edit the source, run `ntnt docs --generate`.

## Roadmap

`ROADMAP.md` tracks all phases. Completed phases archived in `ROADMAP_COMPLETE.md`.

## VS Code Extension

Location: `editors/vscode/intent-lang/`. TextMate grammars for `.tnt` and `.intent`, 20+ snippets, bracket matching. Update `ntnt.tmLanguage.json` when adding new keywords.

---

## Key Lessons (from PR #36-#46 and production usage)

- `Rc<RefCell<>>` means single-threaded — async bridge mandatory for HTTP
- `build.rs` doc validation catches stale docs automatically
- `dev-release` profile essential — full release LTO too slow for dev
- Template cache uses mtime-based invalidation for hot-reload
- Route pattern auto-detection suppresses interpolation in builtin route functions
- `IntentError::type_error(msg)` constructor — never use `IntentError::TypeError(String)` directly
- `define_stdlib()` must be called in all module loading paths (lib, routes, middleware) — not just `define_builtins()`
- `declaration()` wraps all results in `Located` — lint functions must use `unwrap_located()` to see through it
- `Stdio::null()` for subprocess stdout in intent check — `Stdio::piped()` causes pipe buffer deadlock on verbose apps
- `collect_tnt_files` skips non-source dirs (`static-archive`, `node_modules`, `target`)
- `@module` annotation is ONLY for stdlib modules, never for builtins/server actions
- Import paths: `"lib/x.tnt"` resolves from CWD, `"./lib/x.tnt"` resolves from current file
- Cargo.lock: ALWAYS commit after version bumps or dependency changes. CI uses `--locked`.
