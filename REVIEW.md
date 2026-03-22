# Code Review Guidelines — ntnt

## Always check

### Correctness
- Silent error swallows: `let _ =`, `.ok()`, `unwrap_or_default()` — verify the error is truly ignorable
- `_ =>` catch-all match arms — verify they don't swallow types that should be rejected
- KV read-then-write patterns — race condition if two workers can execute simultaneously. Use `kv_set_nx` for atomic claims
- Missing KV keys: public `std/kv::get()` returns `Option::None`, while internal Rust helpers (`kv::kv_get()`) use `Value::Unit` — must NOT treat either as terminal (could be mid-write by another thread)
- Return types: if doc/typechecker says `-> Result<T, String>`, return `Value::ok(value)` not bare `Value`
- ntnt `None` literal is `Value::EnumValue(Option::None)` — when matching on `Value` directly, handle BOTH `Unit` (internal helpers) and `EnumValue(Option::None)` (public API / ntnt code)
- EnumValue matching: always check `enum_name` AND `variant` together — `variant == "Ok"` alone matches user-defined enums
- Recursive bootstrapping: when re-evaluating a .tnt file (workers, hot-reload), verify the eval mode suppresses capabilities that trigger the sub-runtime itself (e.g., worker eval must NOT have JobWorkers, or work_async() recurses infinitely)
- Panic/error messages must include context (file paths, job names, key names) — "failed to X" without "which Y" is useless in production
- Mutex poison on critical data (source file, config): use `expect()`, not `.ok()?`. Silent degradation (bare interpreter, missing config) is worse than crashing — at least a crash tells you what happened
- eval_block() scope leak on panic: `eval_block` pushes a child scope and restores on normal return, but NOT on panic. If you wrap `eval_block` in `catch_unwind`, pop the leaked inner scope on the panic path: `if result.is_err() { interp.pop_scope(); } interp.pop_scope();`

### Concurrency
- Lock ordering on JOB_RUNTIME: `band_worker_task_ids` → `band_cancel_arcs` → `active_bands`. Never violate.
- Poisoned lock recovery: use `unwrap_or_else(|e| e.into_inner())` in observability/status paths
- State consistency: if modifying arcs + task_ids + active_bands, all must be updated

### Platform & Compatibility
- Redis Lua scripts: Lua 5.1 only (Redis < 7.0). No `goto`, `::label::`, `continue`.
- Integer casts: `as usize` from `i64`/`u64` — validate `<= usize::MAX` first
- Priority range: 0-99. Reject Bool/Float — don't silently default

### Resource Management
- Every error path after `bind()` / file creation must clean up (remove_file)
- Threads from `scale_workers` must be tracked in `JOB_RUNTIME.band_cancel_arcs`
- All blocking I/O needs timeouts. Size limits via `Read::take()` before `BufReader`
- Handle `ErrorKind::Interrupted` (EINTR) with `continue`, not `break`

### Documentation
- New stdlib functions need `@error`, `@example` (2+), `@see_also`, `@param` in doc blocks
- `ntnt docs --generate` must be run after any stdlib change
- Rustdoc comments must be updated when function behavior changes — stale docs mislead callers

## Skip
- Formatting and whitespace (cargo fmt enforces this)
- Test file organization
- Comment grammar/wording
- Performance suggestions without benchmark data
- Refactoring that doesn't fix a correctness issue
- Design-docs and markdown-only changes (no code review needed)
