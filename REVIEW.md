# Code Review Guidelines — ntnt

Rules for automated reviewers (Copilot, Greptile) and self-review.
Full skill with workflow, examples, and deep review techniques: see ntnt-code-review skill.

## Always Check

### Correctness
- Silent error swallows: `let _ =`, `.ok()`, `unwrap_or_default()` — verify the error is truly ignorable
- `_ =>` catch-all match arms — verify they don't swallow types that should be rejected
- KV read-then-write patterns — race condition if two workers can execute simultaneously. Use `kv_set_nx` for atomic claims
- Missing KV keys: public `std/kv::get()` returns `Option::None`, internal `kv::kv_get()` returns `Value::Unit` — must NOT treat either as terminal (could be mid-write)
- Return types: if doc/typechecker says `-> Result<T, String>`, return `Value::ok(value)` not bare `Value`
- None has two forms: `Value::Unit` (internal helpers) and `Value::EnumValue(Option::None)` (public API / ntnt code) — handle BOTH
- EnumValue matching: always check `enum_name` AND `variant` together — `variant == "Ok"` alone matches user-defined enums

### Scope & Execution Modes
- eval_block() scope leak on panic: `eval_block` pushes a child scope and restores on normal return, but NOT on panic. If wrapped in `catch_unwind`, pop the leaked inner scope on the panic path: `if result.is_err() { interp.pop_scope(); } interp.pop_scope();`
- Recursive bootstrapping: when re-evaluating a .tnt file (workers, hot-reload, CLI), the eval mode must suppress capabilities that trigger re-evaluation (e.g., worker eval uses Worker mode, not Job — prevents `work_async()` from recursively spawning)
- Check ALL eval paths for the same pattern, not just the one being edited

### Attribute Placement
- `#[cfg(test)]` attaches to the next syntactic *item* (fn, struct, impl, etc.) — inserting a new item between the attribute and its intended target silently compiles away the wrong method. When adding code near `#[cfg(test)]`, verify the attribute still applies to the correct item.

### Hot-Reload & Re-Registration
- Re-registration paths (hot-reload, re-discovery): snapshot before clearing so you can rollback on failure. Clear is needed to remove ghosts (renamed/deleted items). Overwrite-only (without clear) avoids empty-registry windows but leaves ghosts.
- Recursive file collection must skip `node_modules/`, `target/`, and hidden directories (`.git`, `.cache`, etc.)

### Error Quality
- Panic/error messages must include context: file paths, job names, key names. "Failed to X" without "which Y" is useless in production
- Mutex poison on critical data (source file, config): use `expect()`, not `.ok()?`. Silent degradation is worse than crashing

### Concurrency
- Lock ordering on JOB_RUNTIME: `band_worker_task_ids` → `band_cancel_arcs` → `active_bands`. Never violate.
- Poisoned lock recovery: `unwrap_or_else(|e| e.into_inner())` in observability/stats paths only. `expect()` for critical data.
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

## Auth-Specific Guardrails

Apply this section to any change touching `src/stdlib/auth.rs`, `src/stdlib/auth/**`, auth docs, auth generated docs, auth typechecker signatures, or auth tests. DD-043 and DD-062 are the current roadmap for the auth architecture.

### Module Ownership
- `src/stdlib/auth.rs` should stay focused on public API registration, shared surface types that cannot move yet, and minimal coordination glue. Do not add new domain logic there just because it is convenient.
- Prefer focused internal modules for domain behavior: config, cookies, providers, OAuth, guards, routes, request helpers, sessions, storage, primitives, local auth, password reset, and TOTP enrollment.
- If a new auth feature needs durable state, define the record family and storage owner before implementing helpers or routes.
- Avoid hiding durable behavior in generic `data_json` payloads. Challenge `data_json` is acceptable for pending flow metadata, not long-lived credential/reset/TOTP enrollment state.

### Storage and Fallback Semantics
- Treat auth storage behavior as a compatibility surface. Every new auth record family needs explicit store/get/consume/update/delete/cleanup semantics.
- Existing transient auth state may use documented memory fallback paths. Do not copy those semantics to durable credential state by default.
- Durable local-auth state must fail closed in production: local identities, password hashes, password-reset tokens, TOTP enrollment state, bootstrap state, and account-state changes must not silently fall back to process memory when the configured backend fails.
- One-time records (`OAuthState`, exchange tokens, auth challenges, future reset tokens) need atomic consume semantics per backend. Verify replay rejection.
- Schema/migration code must surface real failures. Do not swallow migration errors that leave metadata/credential columns missing.

### Session and Metadata Plumbing
- Request-derived session metadata (`device_name`, `user_agent_hash`, `last_ip_hash`) must be captured, persisted, and round-tripped deliberately. If it is intentionally hidden from public APIs, document why.
- Local/manual sign-in flows must not be weaker than OAuth callback flows. Successful local auth should rotate/migrate existing sessions, attach cookies through the shared cookie policy, and preserve request metadata.
- Remember-me changes must cover the full path: request/config capture → OAuth/local pending state → session TTL selection → cookie Max-Age/Expires → tests.
- Current-user helpers (`user_sessions(req)`, `logout_all(req, ...)`) are not the same as future admin/arbitrary-user APIs (`list_sessions(user_id)`, `revoke_session(session_id)`, `revoke_all_sessions(user_id)`). Keep docs and API names precise.

### Auth Contract Tests
- New auth persistence/state shapes must extend the backend contract harness in the same PR that introduces them.
- Contract tests should assert every field that matters, not only IDs. For session metadata, assert `device_name`, `user_agent_hash`, `last_ip_hash`, and `remember_me` where applicable.
- Cover memory and SQLite by default. Postgres/Redis tests may be env-gated locally, but CI must make skipped backend coverage visible rather than silently green.
- For route protection, prefer at least one end-to-end ntnt/server test in addition to helper-level path matching.

### Local Auth Review Checklist
- Is local auth implemented as one subsystem under `std/auth`, not as disconnected helpers plus template-owned tables?
- Are local credential/reset/TOTP stores auth-owned and fail-closed on backend errors?
- Does local login use request-aware `sign_in_session(response, req, session, options?)` or the same internal completion primitive instead of creating a weaker parallel session path?
- Are reset tokens hash-stored, TTL-bound, one-time-use, and replay-tested?
- Is TOTP enrollment durable account state, not only pending challenge metadata?
- Is the email/SMS delivery boundary explicit (`std/auth` issues/validates tokens and URLs; apps/plugins deliver messages)?
- Does the template integration delete custom auth persistence instead of wrapping it?

## Skip
- Formatting and whitespace (cargo fmt enforces this)
- Test file organization
- Comment grammar/wording
- Performance suggestions without benchmark data
- Refactoring that doesn't fix a correctness issue
- Design-docs and markdown-only changes
