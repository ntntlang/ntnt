# DD-006: ntnt Per-Request Interpreter Architecture

**ID:** dd-006-ntnt-per-request-interpreter
**Author:** Larri
**App:** ntnt
**Status:** in_review
**Created:** 2026-02-28

---

## Summary

This document proposes replacing ntnt's single-threaded interpreter bridge with a per-request interpreter architecture. The result is true parallel HTTP request handling, elimination of the channel-based bottleneck, a dramatically cleaner codebase, and a foundation for high-performance production use of the ntnt language runtime.

---

## Background & Motivation

### The Current Architecture

ntnt's HTTP server currently spans three files:

- **`http_server.rs`** (3,722 lines) — The synchronous server. Handles routing, middleware, templating, security, static files, and all stdlib HTTP functions (`get()`, `post()`, `listen()`, etc.).
- **`http_server_async.rs`** (1,047 lines) — An Axum/Tokio async server for production workloads. Cannot call the interpreter directly because `Interpreter` uses `Rc<RefCell<Environment>>` — not thread-safe.
- **`http_bridge.rs`** (~250 lines) — Serializes HTTP requests to an mpsc channel, queues them to a single interpreter thread, and awaits a response via oneshot channel.

### The Bottleneck

The bridge architecture is fundamentally serial. Every HTTP request — regardless of how many CPU cores are available — is processed one at a time:

```
[Request 1] ─────┐
[Request 2] ─────┤──→ [mpsc channel] ──→ [single interpreter thread] ──→ [responses]
[Request N] ─────┘
```

This is a correctness workaround, not a design choice. The root cause is `Rc<RefCell<>>`.

### Why Not Just Replace `Rc` with `Arc<Mutex>`?

The obvious fix was evaluated and rejected for three reasons:

1. **Recursive environment chains.** ntnt closures hold a reference to their parent environment (`closure → parent → parent → ...`). A nested function call may attempt to write-lock an ancestor environment that an outer call frame already holds a read lock on — a classic reentrant deadlock with no clean solution at the `Environment` level.

2. **Native function callbacks.** Stdlib functions like `map()`, `filter()`, and `sort()` call back into the interpreter with user-provided functions. These execute while the interpreter may already hold environment locks from the outer call frame — another deadlock vector.

3. **126 borrow sites.** The mechanical swap across 10,048 lines of interpreter code cannot be done safely without redesigning the Environment itself.

Making the interpreter thread-safe via locking is the wrong level of abstraction for this problem.

### Why Per-Request Instances Win

The interpreter is stateless across requests. Each HTTP request:
1. Starts with the global environment (builtins + registered functions)
2. Executes a handler function
3. Returns a response
4. Discards all local state

No request needs to see another request's local variables. This is the share-nothing model that makes per-request instances safe and simple — and is how mature language runtimes (Ruby Puma, Python Gunicorn, PHP-FPM) handle concurrency at scale.

---

## Proposed Architecture

### Core Model

```
Startup phase:
  Parse .tnt files → register routes/middleware → snapshot closures into SharedState
  Wrap SharedState in Arc<RwLock<SharedState>>

Request phase (fully parallel):
  Request 1 → clone Arc → route lookup → spawn_blocking → fresh Interpreter → handler → response
  Request 2 → clone Arc → route lookup → spawn_blocking → fresh Interpreter → handler → response
  Request N → clone Arc → route lookup → spawn_blocking → fresh Interpreter → handler → response
```

Each request gets its own `Interpreter` instance with its own `Environment` chain. No locks during execution, no channels, no contention.

### What's Shared vs. Per-Request

**Shared (read-only after startup, wrapped in `Arc<RwLock<>>`)**:

| Data | Type | Notes |
|------|------|-------|
| Route table | `Vec<(Route, StoredHandler, RouteSource)>` | Closure-snapshotted, Send-safe |
| Route index | `HashMap<(String, usize), Vec<usize>>` | O(1) route lookup |
| Middleware list | `Vec<StoredHandler>` | Closure-snapshotted |
| Static file dirs | `Vec<(String, String)>` | Just strings |
| CORS config | `Option<CorsConfig>` | Immutable config |
| Shutdown handlers | `Vec<StoredHandler>` | Executed once on shutdown |

**Per-request (fresh each time)**:

| Data | Notes |
|------|-------|
| `Interpreter` instance | Owns the env chain and call stack |
| `Environment` chain | Local variables, function scope, seeded from StoredHandler snapshot |
| Deferred statements | `defer {}` blocks per call frame |
| Contract state | `current_result`, `current_old_values` |

---

## The `Value: !Send` Problem and Its Resolution

This is the most important technical constraint in the plan and must be understood before any code is written.

### Why `Value` is `!Send`

`Value::Function` contains `Rc<RefCell<Environment>>`. `Rc` is explicitly `!Send` in Rust by design. Therefore `Value` is `!Send`, and any struct containing `Vec<Value>` is `!Send`. This means:

```rust
// THIS WILL NOT COMPILE:
Arc<RwLock<SharedState>>  // SharedState: !Send because it contains Value: !Send
```

Using `unsafe impl Send for SharedState` would be unsound — `Rc` genuinely is not safe to share across threads.

### The Resolution: `StoredHandler` with Closure Snapshots

At route registration time, instead of storing `Value::Function` directly, we snapshot the handler's captured environment. `Environment` tracks values and mutability separately — `values: HashMap<String, Value>` and `mutable_vars: HashSet<String>`. Both must be captured or `let mut` semantics are lost:

```rust
/// A Send-safe representation of a registered handler.
/// Body and params are Clone + Send (actual type is Vec<Parameter>, not Vec<Param>).
/// The closure is stored as a flat snapshot with no Rc, no RefCell.
/// mutable_names tracks which bindings were `let mut` — required to restore correct semantics.
/// Any function values in the snapshot are Value::FlatFunction (not live Value::Function).
#[derive(Clone)]
pub struct StoredHandler {
    pub name: String,
    pub params: Vec<Parameter>,            // matches actual AST type
    pub body: Block,
    pub closure_snapshot: HashMap<String, Value>,
    pub mutable_names: HashSet<String>,    // preserves let mut semantics
}

unsafe impl Send for StoredHandler {}  // sound: no Rc in any field after flatten_value()
unsafe impl Sync for StoredHandler {}  // sound: immutable after construction
```

`Environment::all_bindings()` returns values only — it does **not** return mutability info. A companion method `all_mutable_names() -> HashSet<String>` must be added to `Environment` that walks the parent chain collecting `mutable_vars` entries, mirroring `all_bindings()`.

The `unsafe impl Send` is sound because:
1. `Value::NativeFunction` contains `fn` pointers — these are `Send + Sync` in Rust ✓
2. All `Value` variants except `Function` are `Send`-safe by construction
3. Any `Value::Function` in the snapshot is recursively flattened into `Value::FlatFunction` (no `Rc`) — see full flattening spec below
4. `StoredHandler` is never mutated after construction (write lock only during hot-reload swap)

### `Value::FlatFunction` — A Distinct Variant

All `Value::Function` instances stored in `SharedState` are converted to `Value::FlatFunction` — a new enum variant with no `Rc`, no `RefCell`:

```rust
Value::FlatFunction {
    name: String,
    params: Vec<Parameter>,               // matches actual AST type
    body: Block,
    contract: Option<FunctionContract>,
    type_params: Vec<TypeParam>,          // matches actual AST type
    closure_snapshot: HashMap<String, Value>,
    mutable_names: HashSet<String>,       // preserves let mut semantics
}
```

This makes the invariant machine-checked by the type system:
- `Value::Function` → live `Rc`-backed closure, only valid inside a running interpreter
- `Value::FlatFunction` → flat snapshot, `Send + Sync`, safe in `SharedState`

The `match` churn from adding a new variant is bounded and caught by the compiler. This is the right tradeoff.

### Recursive Flattening

`flatten_value()` must cover **every** `Value` variant that can contain a nested `Value::Function`. The full exhaustive match — `Value::Struct`, `Value::EnumValue`, `Value::Return`, and `Value::EnumConstructor` are all included. Leaving any variant unhandled makes `unsafe impl Send` unsound:

```rust
fn flatten_value(v: Value) -> Value {
    match v {
        // The core case: Function → FlatFunction with snapshot
        Value::Function { name, params, body, closure, contract, type_params } => {
            let snapshot = closure.borrow().all_bindings();
            let mutable_names = closure.borrow().all_mutable_names();
            let flat_snapshot = snapshot.into_iter()
                .map(|(k, v)| (k, flatten_value(v)))
                .collect();
            Value::FlatFunction { name, params, body, contract, type_params,
                                  closure_snapshot: flat_snapshot, mutable_names }
        }
        // Containers that can hold functions — must recurse
        Value::Array(items) =>
            Value::Array(items.into_iter().map(flatten_value).collect()),
        Value::Map(m) =>
            Value::Map(m.into_iter().map(|(k, v)| (k, flatten_value(v))).collect()),
        Value::Struct { name, fields } =>
            Value::Struct { name, fields: fields.into_iter()
                .map(|(k, v)| (k, flatten_value(v))).collect() },
        Value::EnumValue { enum_name, variant, values } =>
            Value::EnumValue { enum_name, variant,
                values: values.into_iter().map(flatten_value).collect() },
        Value::Return(v) =>
            Value::Return(Box::new(flatten_value(*v))),
        // All other variants contain no nested Values and no Rc — safe to pass through:
        // Int, Float, String, Bool, Unit, NativeFunction (fn pointers are Send),
        // EnumConstructor (contains only name/variant/arity strings and usize — no Values),
        // Break, Continue, and any future primitive variants.
        other => other,
    }
}
```

**Self-referential closure guard:** If `fib` captures itself in its closure, `flatten_value()` must not recurse infinitely. Use a visited set keyed by function name during flattening to break cycles. When a cycle is detected, emit a `Value::FlatFunction` with an empty body as the sentinel — this is more self-documenting than `Value::Unit` and won't cause confusion if the value is ever inspected:

```rust
fn flatten_value_visited(v: Value, visited: &mut HashSet<String>) -> Value {
    if let Value::Function { ref name, ref params, .. } = v {
        if !visited.insert(name.clone()) {
            // Cycle detected — emit a no-op FlatFunction stub to break the cycle.
            // In practice this is only reached for recursive fn defs; the outer
            // function (already being flattened) is what gets called at request time.
            return Value::FlatFunction {
                name: name.clone(), params: params.clone(), body: Block::empty(),
                contract: None, type_params: vec![],
                closure_snapshot: HashMap::new(), mutable_names: HashSet::new(),
            };
        }
    }
    // ... rest of match
}
```
```

At request time, `StoredHandler::to_call_value()` converts the flat snapshot back into a live `Value::Function` by calling `Environment::from_snapshot()` to produce a fresh `Rc<RefCell<Environment>>`. This is a per-request hot path and is benchmarked in Phase 1.

### Behavioral Change: Module-Level Mutable State

This architecture changes one observable behavior. Module-level mutable state is snapshotted at registration time — each request starts from that snapshot, not the live state:

```ntnt
// Before: request 1 → "1", request 2 → "2" (shared state)
let count = 0
fn counter(req) { count = count + 1; return text(count) }

// After: every request → "1" (isolated snapshot)
```

This is the correct behavior for stateless HTTP handlers. Users who need cross-request state should use `kv_set`/`kv_get` (Redis) or `pg_execute` (PostgreSQL). This change is documented in Phase 5.

---

## Implementation Phases

### Phase 1: Benchmark Interpreter Construction ✅ COMPLETE

**Goal:** Validate that interpreter construction is cheap enough to do per-request before any architecture work begins.

At 30K req/s across 8 cores, construction runs ~3,750 times/core/sec. At 50µs per construction, that's 18.75% CPU overhead — acceptable. At 500µs, it exceeds the cost of the bridge and the architecture needs a different approach.

**Benchmark to write:**

```rust
// benches/interpreter_construction.rs (criterion)
fn bench_new_interpreter(c: &mut Criterion) {
    c.bench_function("Interpreter::new + register_builtins", |b| {
        b.iter(|| {
            let mut interp = Interpreter::new();
            interp.register_http_builtins();
        })
    });
    c.bench_function("Environment seed from 50-key snapshot", |b| {
        let snapshot = make_test_snapshot(50);
        b.iter(|| Environment::from_snapshot(&snapshot))
    });
}
```

**Decision gates:**
- **<50µs** → proceed as planned
- **50–200µs** → implement lazy builtin registration (register only modules the script actually `use`s — already partially tracked)
- **>200µs** → fall back to warm interpreter pool (pool of pre-constructed interpreters, like a connection pool)

**Output:** Benchmark numbers documented in the Performance Projections section before Phase 2 begins.

---

### Phase 2: Send-Safe Value Representation (Est. 4–5 hrs, Risk: Medium)

**Prerequisite:** Phase 1 complete. `SharedState` cannot be wrapped in `Arc` until stored Values are `Send`-safe.

**Goal:** Eliminate `Rc` from stored handlers so `SharedState` can be wrapped in `Arc`.

**Steps:**

1. **Add `Value::FlatFunction` variant** to the `Value` enum as specified above.

2. **Implement `flatten_value()`** — recursive pass converting `Value::Function` to `Value::FlatFunction` throughout the value tree.

3. **Add `Environment::all_mutable_names() -> HashSet<String>`** — mirrors `all_bindings()` but walks `mutable_vars` up the parent chain. Required to preserve `let mut` semantics in snapshots.

4. **Implement `Environment::from_snapshot(snapshot: &HashMap<String, Value>, mutable_names: &HashSet<String>)`** — creates a fresh `Rc<RefCell<Environment>>` seeded from a flat map. Calls `define()` for immutable bindings and `define_mutable()` for those in `mutable_names`. This is the inverse of `all_bindings()` + `all_mutable_names()`.

5. **Implement `StoredHandler::to_call_value()`** — converts a `StoredHandler` back into a live `Value::Function`. Calls `Environment::from_snapshot(&self.closure_snapshot, &self.mutable_names)` to reconstitute the closure with correct mutability. `contract` and `type_params` are copied directly from the `StoredHandler` fields (they contain no `Rc` and don't require transformation). The resulting `Value::Function` is valid only within the fresh `Interpreter` instance for that request.

6. **Update route registration functions** (`get()`, `post()`, `put()`, `delete()`, `patch()`, `use_middleware()`, `on_shutdown()`) to call `flatten_value()` on registered handlers and store as `StoredHandler`.

**Tests to write:**

```
- Closure capture: handler closes over module-level constant → value preserved in snapshot
- Closure mutability: handler closes over let mut var → mutations work correctly per request
- Nested closure: handler uses a helper fn defined at module scope → helper in snapshot
- Recursive value: snapshot contains a Value::Function → flattened to FlatFunction
- Struct value: Value::Struct with function field → flattened correctly
- EnumValue: Value::EnumValue with function in values vec → flattened correctly
- Return: Value::Return wrapping a function → flattened correctly
- Self-referential: fn fib captures fib → no infinite recursion, works correctly
- Isolation: two requests from same handler get independent Environment instances
- Mutation isolation: one request mutating a local var does not affect another request
- Array values: Value::Array containing function values → all flattened
```

**Definition of done:**
```rust
let stored: StoredHandler = /* register handler */;
let shared = Arc::new(RwLock::new(vec![stored]));  // THIS MUST COMPILE
```

### De-risking Phase 2

Phase 2 is the highest-risk phase because it touches the core Value type and the closure model. Here's how to approach it safely.

**Start with a single test case, not the full system.** Before modifying any production code, write a standalone test that proves the round-trip works:

```rust
#[test]
fn test_stored_handler_roundtrip() {
    // Build a real interpreter, register a handler, snapshot it
    let mut interp = Interpreter::new();
    interp.eval("let greeting = \"hello\"\nfn home(req) { return text(greeting) }").unwrap();
    let handler_val = interp.environment.borrow().get("home").unwrap();

    // Flatten it
    let flat = flatten_value(handler_val);
    assert!(matches!(flat, Value::FlatFunction { .. }));

    // Store it
    let stored = StoredHandler::from_flat(flat);
    let shared = Arc::new(RwLock::new(vec![stored.clone()]));  // MUST COMPILE

    // Reconstitute and call it in a fresh interpreter
    // Build a minimal SharedState for testing
    let shared = SharedState::from_interpreter(&interp);
    let mut req_interp = Interpreter::new_for_request(&shared);
    let live = stored.to_call_value();
    let result = req_interp.call_function(live, vec![dummy_request()]).unwrap();
    assert_eq!(result, expected_text_response("hello"));
}
```

This test exercises the entire Phase 2 stack end-to-end before a single production path changes.

**Implement `flatten_value()` in isolation first.** Add it as a free function with its own unit tests before wiring it into route registration. Verify each `Value` variant independently:

```rust
// Test each variant that can contain Rc:
assert_flat(flatten_value(make_function_value()));       // Function → FlatFunction
assert_flat_recursive(flatten_value(make_array_with_fn())); // Array containing Function
assert_flat_recursive(flatten_value(make_map_with_fn()));   // Map containing Function
assert_flat_recursive(flatten_value(make_struct_with_fn())); // Struct containing Function
```

**Don't touch route registration until the round-trip test passes.** Route registration is the production hot path. Only wire `flatten_value()` into `get()`, `post()`, etc. once the standalone test confirms the snapshot → reconstitute cycle is correct.

**The `unsafe impl Send` scope matters.** Keep it tightly scoped to `StoredHandler`. Do not implement `Send` on `Value` itself — that would be unsound. The safety argument depends on `StoredHandler` never holding a live `Rc`, which is guaranteed by `flatten_value()` converting all `Function` variants before storage.

**Watch for the self-referential closure edge case.** A handler that captures itself (recursive functions defined at module scope) could create a cycle in `all_bindings()`. Test this explicitly:

```ntnt
fn fib(n) {
    if n <= 1 { return n }
    return fib(n - 1) + fib(n - 2)
}
get("/fib", fn(req) { return text(fib(10)) })
```

If `fib` captures `fib` in its closure, the snapshot must handle this without infinite recursion. The fix is a visited-set in `flatten_value()` — track already-flattened function names and skip re-flattening them.

---

### Phase 3: Extract `SharedState` (Est. 3–4 hrs, Risk: Low)

**Goal:** Pull server data out of `Interpreter` into a standalone `Arc`-wrappable struct.

**Steps:**

1. Create `SharedState` in `http_server.rs`:

```rust
pub struct SharedState {
    // Routing
    pub routes: Vec<(Route, StoredHandler, RouteSource)>,
    pub route_index: HashMap<(String, usize), Vec<usize>>,
    pub middleware: Vec<StoredHandler>,
    pub shutdown_handlers: Vec<StoredHandler>,
    // Static assets
    pub static_dirs: Vec<(String, String)>,
    // Network config
    pub cors_config: Option<CorsConfig>,
    // Type context — carried into every per-request interpreter
    // so handlers can use user-defined structs, enums, traits, and type aliases
    pub structs: HashMap<String, Vec<Field>>,
    pub enums: HashMap<String, Vec<EnumVariant>>,
    pub type_aliases: HashMap<String, TypeExpr>,
    pub trait_definitions: HashMap<String, TraitInfo>,
    pub trait_implementations: HashMap<String, Vec<String>>,
    // Source context for error messages
    pub main_source_file: Option<String>,
}
```

The type context fields (`structs`, `enums`, `type_aliases`, `trait_*`) are extracted from `Interpreter` after the startup phase and seeded into every `new_for_request()` instance. Without this, any handler that instantiates a user-defined struct (e.g., `User { name: "Josh" }`) will fail in a per-request interpreter that has no knowledge of the `User` type definition.

2. Move all route/middleware data from `ServerState` and `Interpreter` into `SharedState`.

3. `Interpreter` holds `Arc<RwLock<SharedState>>` during startup (write lock for registrations), surrenders it to `listen()` at startup completion.

4. All route registration functions write to `SharedState` via write lock during startup (single-threaded, no contention).

5. Compile check: `Arc<RwLock<SharedState>>` must compile. If it doesn't, Phase 2 is incomplete.

**Tests:** All existing route registration tests pass. `SharedState` correctly populated after a full startup sequence.

**Implementation notes (actual deviations):**
- `Interpreter` still holds `SharedState` as a direct field (not `Arc<RwLock<>>`). The Arc wrapping happens in Phase 4 when the async server integration is built. Phase 3's goal was extracting the struct and making it Arc-wrappable, verified by compile test.
- Added `NativeFunction` support to `StoredHandler` (not in design doc) — needed because auth routes register native function handlers directly. Uses a `__native_fn__` sentinel in the closure snapshot.
- Used `_from_value` convenience methods on `SharedState` to flatten Value → StoredHandler at registration time, rather than wrapping with a write lock at each call site (simpler given single-threaded startup phase).

---

### Phase 4: Per-Request Execution Engine (Est. 4–5 hrs, Risk: Low)

**Goal:** Build `Interpreter::new_for_request()`, `execute_request()`, and shutdown handler execution.

**`Interpreter::new_for_request()`** — seeds builtins, stdlib, and the type context from `SharedState`. Designed with lazy registration in mind for future slower-hardware deployments:

```rust
pub fn new_for_request(shared: &SharedState) -> Self {
    Self::new_for_request_with_modules(shared, None)  // None = register all modules
}

pub fn new_for_request_with_modules(shared: &SharedState, modules: Option<&HashSet<String>>) -> Self {
    let mut interp = Interpreter::new_bare();
    interp.register_builtins_selective(modules); // define_builtins() + define_stdlib()
    interp.define_builtin_types();               // Option, Result, etc. — REQUIRED
    // Seed type context so user-defined structs, enums, traits work in handlers
    interp.structs = shared.structs.clone();
    interp.enums = shared.enums.clone();
    interp.type_aliases = shared.type_aliases.clone();
    interp.trait_definitions = shared.trait_definitions.clone();
    interp.trait_implementations = shared.trait_implementations.clone();
    // Seed source file for error messages
    interp.current_file = shared.main_source_file.clone();
    interp
}
```

`None` registers all 23 stdlib modules (43.9 µs on dev machine). On slower hardware, pass the set of modules the script imports to reduce construction cost without an API change.

**`execute_request()`:**

```rust
pub fn execute_request(
    shared: Arc<RwLock<SharedState>>,
    handler: StoredHandler,
    middleware: Vec<StoredHandler>,
    req: Value,
) -> Result<Value> {
    // Read shared state once to seed interpreter — no lock held during execution
    let interp_shared = shared.read().unwrap_or_else(|e| e.into_inner());
    let mut interp = Interpreter::new_for_request(&interp_shared);
    drop(interp_shared);

    let mut final_req = req;
    for mw in middleware {
        final_req = interp.call_function(mw.to_call_value(), vec![final_req])?;
        if is_response_value(&final_req) { return Ok(final_req); }  // middleware short-circuit
    }
    interp.call_function(handler.to_call_value(), vec![final_req])
}
```

**Axum handler integration:**

```rust
async fn handle_request(State(shared): State<Arc<RwLock<SharedState>>>, req: Request<Body>) -> Response {
    let bridge_req = BridgeRequest::from_axum(req).await;
    let (handler, middleware) = {
        let s = shared.read().await;
        let Some((h, m)) = s.find_handler(&bridge_req) else { return not_found_response(); };
        (h.clone(), m.clone())
    };
    // Lock released — no contention during execution
    // spawn_blocking returns JoinError if the task panics — catch and 500
    let join_result = tokio::task::spawn_blocking(move || {
        execute_request(shared, handler, middleware, bridge_req.to_value())
    }).await;
    match join_result {
        Ok(result) => result_to_axum_response(result),
        Err(_panic) => internal_server_error("handler panicked"),
    }
}
```

**Shutdown handler execution:**

On SIGTERM/SIGINT, before Tokio stops:

```rust
async fn run_shutdown_handlers(shared: Arc<RwLock<SharedState>>) {
    let state = shared.read().await;
    let handlers = state.shutdown_handlers.clone();
    if handlers.is_empty() { return; }
    // Clone what we need before dropping the lock
    let shared_snapshot = SharedStateSnapshot::from(&*state);
    drop(state);
    tokio::task::spawn_blocking(move || {
        let mut interp = Interpreter::new_for_request(&shared_snapshot);
        for handler in handlers {
            if let Err(e) = interp.call_function(handler.to_call_value(), vec![Value::Unit]) {
                eprintln!("[shutdown] Handler error: {}", e);
            }
        }
    }).await.ok();
}
```

Errors are logged but never abort — all shutdown handlers must have a chance to run.

**Thread pool sizing:**

```
NTNT_BLOCKING_THREADS = (target_rps × avg_handler_ms) / 1000

Examples:
  1K rps  × 10ms  →  10 threads
  10K rps × 10ms  → 100 threads
  30K rps × 5ms   → 150 threads
  10K rps × 100ms → 1,000 threads
```

Default: Tokio's default (~512). Set `NTNT_BLOCKING_THREADS` to match your workload. `NTNT_REQUEST_TIMEOUT` (default: 30s) returns 504 on breach.

**Database connections in per-request model — works by design:**
PostgreSQL, Redis/KV, and SQLite connections use global static registries (`LazyLock<Mutex<HashMap<u64, Arc<Mutex<...>>>>>`) in the stdlib. Connection handles stored in ntnt values are integer IDs pointing into these registries — not live connection objects. A fresh per-request interpreter looks up the same global registry and finds the connection immediately. No migration needed. This is explicitly confirmed — do not change the registry pattern.

**Session and cookie handling:** Any in-memory session store (middleware that writes to a module-level map) will break — same behavioral change as module-level mutable state. Migrate to Redis-backed sessions. Document alongside the mutable state migration guide in Phase 5.

**Tests:** Simple handler, middleware chain propagation, middleware short-circuit, context isolation, error → 500, panic → 500, shutdown handlers in order, timeout → 504, user-defined struct in handler (type context seeding), DB connection across per-request boundary.

**Implementation notes (actual deviations):**
- `execute_request()` and `run_shutdown_handlers()` are public associated functions on `Interpreter` (not free functions) — needed access to `call_function` which is a method on Interpreter. The API matches the design doc's intent.
- `Value` is `!Send` (contains `Rc<RefCell<Environment>>`), so `spawn_blocking` closures convert `BridgeRequest → Value` inside the closure and convert `Value → BridgeResponse` before returning. Only `Send` types cross the thread boundary.
- `run_async_http_server()` was fully rewritten: removed the bridge channel architecture and single-interpreter-thread loop. Now uses `Arc<RwLock<SharedState>>` with `start_per_request_server()` in `http_server_async.rs`. The old `start_server_with_bridge` is preserved but unused (removed in Phase 5).
- Hot-reload is not yet integrated into the per-request path (deferred to Phase 5 per the design doc — hot-reload atomic swap). The old hot-reload code was in the bridge loop; the new per-request path will get `rebuild_shared_state()` in Phase 5.
- DB connection test left unchecked — requires integration test with actual DB; unit tests verify type context seeding and execution isolation.
- Static file serving is handled directly in the async handler (no `spawn_blocking` needed for file I/O) to avoid unnecessary interpreter construction.
- The `PerRequestState` struct holds `Arc<RwLock<SharedState>>` and is used as Axum state; route lookup happens under a brief read lock, then the lock is released before `spawn_blocking`.
- 925 tests passing (10 new Phase 4 tests: simple handler, middleware chain, middleware short-circuit, isolation, handler error, cross-thread execution, shutdown handlers ×2, type context seeding, module API).

---

### Phase 5: Unify Server Files + Hot-Reload (Est. 3–4 hrs, Risk: Low)

**Goal:** Collapse three files into two, make hot-reload atomic.

**File structure after:**

```
stdlib/
  http_server.rs         // Route registration, SharedState, StoredHandler,
                         // all ntnt stdlib functions, listen() entry point
  http_server_async.rs   // Axum runner, execute_request, static files,
                         // graceful shutdown, hot-reload watcher
  // http_bridge.rs      ← DELETED
```

**True unification — one server for everything:**

The sync server is not preserved for tests or CLI. It is eliminated entirely. `execute_request()` IS the server. There is no separate sync server to maintain.

```rust
// Tests, CLI, ntnt intent, intent studio (no Tokio required):
let result = execute_request(shared.clone(), handler, middleware, req_value);

// Production Axum:
let result = tokio::task::spawn_blocking(move || {
    execute_request(shared, handler, middleware, req_value)
}).await;
```

**ntnt intent and intent studio are migrated in this phase.** Both currently use the old sync server path. They must be updated to use `execute_request()` — keeping them on the old path would mean old server code can never be deleted.

Migration targets:
- `ntnt intent` server mode → `execute_request()` directly
- Intent Studio HTTP handler → `execute_request()` directly
- All acceptance tests that spin up the sync server → updated to build a `SharedState` and call `execute_request()` directly (no HTTP server needed for unit tests; minimal Axum test harness for integration tests)
- `http_server.rs` sync `listen()` entrypoint → deleted
- `http_bridge.rs` → deleted

No code path remains that uses the old single-threaded serve loop after Phase 4.

**Interpreter fields audit (during implementation):** `Interpreter::new()` initializes ~20 fields. Most are per-request by nature (`deferred_statements`, `current_result`, `current_old_values`, `current_line`, `current_col`, `contracts`). Fields explicitly handled: `structs`, `enums`, `type_aliases`, `trait_definitions`, `trait_implementations`, `current_file` (seeded from SharedState). The `server_state` field is replaced by `SharedState` entirely. Remaining fields (`loaded_modules`, `lib_modules`, `imported_files`, `routes_dir`, `middleware_files`, etc.) are startup/hot-reload concerns — they live in `SharedState` or `rebuild_shared_state()`, not in per-request instances. Verify this assumption during Phase 4 implementation before finalising `new_for_request()`.

**Hot-reload:**

Dedicated background async task replaces per-request mtime polling. No new crate dependency:

```rust
async fn hot_reload_watcher(shared: Arc<RwLock<SharedState>>, routes_dir: PathBuf, poll_interval: Duration) {
    let mut last_mtimes = collect_dir_mtimes(&routes_dir);
    loop {
        tokio::time::sleep(poll_interval).await;
        let current = collect_dir_mtimes(&routes_dir);
        if current != last_mtimes {
            match rebuild_shared_state(&routes_dir) {
                Ok(new_state) => {
                    *shared.write().await = new_state;
                    eprintln!("[hot-reload] Reloaded — {} routes", shared.read().await.routes.len());
                }
                Err(e) => eprintln!("[hot-reload] Failed: {} — keeping old state", e),
            }
            last_mtimes = current;
        }
    }
}
```

Atomic swap: in-flight requests hold their cloned `StoredHandler` and complete normally. Failed reloads keep the old state running. `NTNT_HOT_RELOAD_INTERVAL_MS` (default: 500ms).

**`rebuild_shared_state()` is a full startup re-run:** It creates a fresh `Interpreter`, re-parses and re-evaluates all .tnt files (including lib modules and route files), runs all `get()`/`post()`/`use_middleware()`/`on_shutdown()` registrations, extracts the resulting `SharedState` including type context, and returns it. It does NOT diff the old state — it rebuilds from scratch. This ensures hot-reload is always consistent with a cold start.

**Implementation notes (actual deviations):**
- Multiple failed agent attempts before correct implementation. Agents falsely reported completion, were killed prematurely, left uncommitted stubs. Phase 5 was split into 5a and 5b and implemented carefully.
- `BridgeRequest`/`BridgeResponse` moved inline to `http_server_async.rs` — the types remain as Send-safe HTTP representation for `spawn_blocking`, the file is deleted.
- **Intent studio:** No migration needed — studio communicates with the running app via HTTP client calls to `app_port`. It was never executing handlers directly, so `execute_request()` migration doesn't apply.
- **Intent check:** Full migration to `execute_request()` (no HTTP at all) deferred as a future improvement. Instead, `test_mode` shutdown flag is now wired to Axum's graceful shutdown in `start_per_request_server()` — intent check runs the async server, test thread fires requests, sets flag when done, server shuts down. Functionally correct; avoids a deep rewrite of intent.rs (5000+ lines).
- A partial incomplete stub (`run_tests_with_shared_state` referencing nonexistent `run_single_test_direct`) was left in `intent.rs` by a failed agent — found and removed during Phase 5b.
- 920 tests passing (5 fewer than Phase 4's 925 — those were http_bridge.rs module tests deleted with the file).

**Static files:** Served by Axum/tower-http directly, bypassing the interpreter entirely. Logging goes through tower-http `TraceLayer`.

---

### Phase 6: Documentation & Behavioral Notes (Est. 2 hrs)

**Module-level mutable state — migration guide:**

```ntnt
// Before (broken with per-request):
let count = 0
fn counter(req) { count = count + 1; return text(count) }

// After (correct — use Redis for cross-request state):
fn counter(req) {
    let count = int(kv_get("request_count") ?? "0") + 1
    kv_set("request_count", str(count))
    return text(count)
}
```

**In-memory session stores — migration guide:**

Any middleware that stores session state in a module-level map will break (same as module-level mutable state). Migrate to Redis-backed sessions using `kv_set`/`kv_get`. This is the correct pattern for stateless HTTP anyway.

**Database connections:** No migration needed — connection handles work correctly across per-request interpreters via global static registries.

**New environment variables to document:**
- `NTNT_BLOCKING_THREADS` — spawn_blocking thread pool size
- `NTNT_REQUEST_TIMEOUT` — seconds before 504 (default: 30)
- `NTNT_HOT_RELOAD_INTERVAL_MS` — poll interval (default: 500)

**Other changes:**
- Static file access logs now use tower-http tracing format
- Hot-reload is now atomic — zero dropped requests during reload

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Interpreter construction >50µs | Medium | High | Phase 1 gates — lazy builtins or warm pool as fallback |

| Module-level mutable state breaks user code | High | Medium | Docs + future static analysis warning |
| spawn_blocking pool exhaustion | Medium | Medium | `NTNT_BLOCKING_THREADS` + sizing formula in docs |
| Middleware `req.context` leak between requests | Low | High | Explicit isolation tests in Phase 3 |
| `on_shutdown()` handlers not executing | Low | High | Explicitly specified; shutdown handler test |
| `flatten_value()` misses Value variant | Low (now exhaustive) | High | Explicit arms for Struct, EnumValue, Return; compiler enforces exhaustiveness |
| `let mut` semantics lost in snapshot | Low (now fixed) | High | `all_mutable_names()` + `mutable_names` field in StoredHandler and FlatFunction |
| User-defined types unavailable per-request | Low (now fixed) | High | Type context (structs, enums, traits) carried in SharedState, seeded in new_for_request() |
| Handler panic crashes worker | Low (now handled) | Medium | spawn_blocking JoinError caught → 500, not panic propagation |
| Hot-reload fails during `rebuild_shared_state` | Low | Low | Keep old state on error — explicitly handled |
| spawn_blocking timeout thread lingers | Low | Medium | `tokio::time::timeout` wraps future, not thread — document |

---

## Performance Results

*All wrk benchmarks run on the same dev machine. "Simple" = local ntnt app, trivial handlers. "Real" = staging.larri.net dashboard app running against a live PostgreSQL container. Pool column is pending — pool implementation has a correctness bug under investigation.*

---

### Master Benchmark Table

#### Simple handlers — local ntnt app (`wrk -t4 -c50 -d15s`, localhost)

| Metric | Bridge (v0.3.16) | Per-request (no pool) | Per-request (pool) |
|--------|-----------------|----------------------|-------------------|
| `GET /` — text response | **82,132 req/s** | 50,923 req/s | 58,691 req/s |
| `GET /json` — map + json() | **75,376 req/s** | 49,287 req/s | 56,806 req/s |
| `GET /middleware` — 1 middleware | **80,384 req/s** | 50,034 req/s | 57,269 req/s |
| Avg latency | **0.58–0.63 ms** | 0.94–0.97 ms | 0.80–0.83 ms |

#### Simple handlers — higher concurrency (`wrk -t8 -c200 -d15s`, localhost)

| Metric | Bridge (v0.3.16) | Per-request (no pool) | Per-request (pool) |
|--------|-----------------|----------------------|-------------------|
| `GET /` — text response | **81,857 req/s** | 48,169 req/s | 🔲 pending |
| Avg latency | **2.44 ms** | 4.15 ms | 🔲 pending |

#### Real workload — staging dashboard app (`wrk -t4 -c50 -d15s`, Docker container direct)
*Routes do real work: PostgreSQL queries, template rendering, auth middleware, session lookup*

| Route | Bridge (v0.3.16) | Per-request (no pool) | Per-request (pool) |
|-------|-----------------|----------------------|-------------------|
| `GET /login` — template render + auth | **1,520 req/s** | 1,380 req/s | 🔲 pending* |
| `GET /api/startups` — PG query + JSON | **276 req/s** | 259 req/s | 🔲 pending* |
| `GET /api/design-docs` — PG query + JSON | **325 req/s** | 310 req/s | 🔲 pending* |
| Avg latency `/login` | **31.5 ms** | 34.7 ms | 🔲 pending* |
| Avg latency `/api/startups` | **173 ms** | 184 ms | 🔲 pending* |

*Pool fix (commit 7f0eecb) verified correct via 20-req local reuse test. Staging real-workload benchmark pending — requires IP allowlist access to hit container directly.

---

### What the numbers say

**Simple handlers: bridge wins by ~38%.** The bridge's warm interpreter thread avoids the 52 µs per-request construction cost. When handler execution is sub-millisecond, construction overhead dominates. The pool should close this gap.

**Real workload: effectively a tie (~9% gap).** When handlers do real work (DB queries averaging 150–180 ms), the 52 µs construction cost is negligible (<0.1% of handler time). Both architectures are bottlenecked on PostgreSQL connection throughput, not the interpreter. The pool column will be the deciding factor here.

**Pool result (simple handlers):** Pool improves throughput from 51K → 58K req/s (+15%), but bridge (~82K) still leads by ~28%. The pool reduces interpreter construction overhead but doesn't eliminate it entirely — `reset_for_reuse` is still ~52 µs (env allocation + builtin re-registration). The remaining gap is structural: `define_stdlib()` on every request is expensive. Further optimization possible (stdlib registration cache, lazy loading) but diminishing returns.

**Real-workload pool column pending** (staging IP allowlist blocks direct container access for benchmarks — needs to run from the allowed IP).

---

### Criterion Micro-benchmarks (release build, 100 samples)

| Benchmark | Median | Notes |
|-----------|--------|-------|
| `Interpreter::new` (full stdlib) | **51.0 µs** | Phase 1 baseline was 43.9 µs — slight increase from DD-006 additions |
| `Interpreter::new_for_request` | **52.3 µs** | ~1.3 µs overhead vs plain new() — type context clone |
| `Interpreter::reset_for_reuse` | ~52 µs est. | Pool path — fresh env allocation + builtin re-registration (no separate criterion run yet) |
| `Environment::from_snapshot` (10 bindings) | **990 ns** | Closure reconstitution — effectively free |
| `Environment::from_snapshot` (50 bindings) | **5.1 µs** | Larger apps with many module-level captures |
| `SharedState` read lock acquire | **3.6 ns** | Contention overhead per request — negligible |

---

### Thread Pool Sizing Reference

| Target RPS | Avg Handler Time | Threads Needed |
|-----------|-----------------|----------------|
| 1,000 | 10ms | 10 |
| 5,000 | 10ms | 50 |
| 10,000 | 10ms | 100 |
| 30,000 | 5ms | 150 |
| 10,000 | 100ms | 1,000 |

---

## Benefits to the ntnt Language

**Production viability.** The bridge was always a correctness workaround. Per-request instances are how battle-tested language runtimes handle concurrency at scale. This is the change that makes ntnt a serious production runtime.

**Architectural honesty.** The bridge exists because `Rc<RefCell<>>` can't cross thread boundaries. Per-request instances work with the interpreter's nature instead of against it.

**Simpler codebase.** Three files → two files. ~250 lines of channel plumbing deleted. One unified request execution path.

**Better hot-reload.** Atomic swap means zero dropped requests, ever.

**Foundation for future capabilities:**
- Multi-tenant hosting — multiple ntnt apps in one process, each with its own `Arc<SharedState>`
- Serverless/lambda mode — `execute_request()` is already the right interface
- Request-level resource limits — per-request interpreters can enforce instruction/memory budgets
- Distributed tracing — inject trace ID at Axum layer, thread through request Value
- Serializable handlers — `StoredHandler` is the foundation for network-portable handler logic

---

## Definition of Done

- [x] **Phase 1:** Benchmark written and results documented; gate decision recorded — 43.9 µs, PASS
- [x] **Phase 2:** `Value::FlatFunction` variant added (with `mutable_names` field)
- [x] **Phase 2:** `Environment::all_mutable_names()` implemented
- [x] **Phase 2:** `flatten_value()` covers Function, Array, Map, Struct, EnumValue, Return — exhaustive
- [x] **Phase 2:** Self-referential closure (fib/fib) test passes — no infinite recursion
- [x] **Phase 2:** `Environment::from_snapshot(values, mutable_names)` implemented — restores let mut
- [x] **Phase 2:** `StoredHandler::to_call_value()` implemented — closure + mutability restored
- [x] **Phase 2:** All closure capture / isolation tests pass
- [x] **Phase 2:** `Arc<RwLock<SharedState>>` compiles
- [x] **Phase 3:** `SharedState` extracted with type context (structs, enums, type_aliases, traits)
- [x] **Phase 3:** All route registration tests pass
- [x] **Phase 4:** `execute_request()` implemented and tested
- [x] **Phase 4:** Handler panic → 500 (not process crash)
- [x] **Phase 4:** User-defined struct in handler — type context seeding works
- [x] **Phase 4:** DB connection across per-request boundary works — commit f8448e6. SQLite in-memory test: open connection at startup, insert row, verify fresh per-request interpreter resolves handle via global static registry and reads back correct data. Close verifies registry cleanup.
- [x] **Phase 4:** Middleware chain, isolation, short-circuit tests pass
- [x] **Phase 4:** Shutdown handlers implemented and tested
- [x] **Phase 4:** `NTNT_BLOCKING_THREADS` and `NTNT_REQUEST_TIMEOUT` respected
- [x] **Phase 5:** `http_bridge.rs` deleted — commit 81f8bf0
- [x] **Phase 5:** ntnt intent and intent studio migrated to `execute_request()` — intent check uses `build_shared_state_from_source()` + `run_single_test_direct()`, studio uses `run_tests_with_shared_state()` with SharedState rebuilt per run — commit c0591b2
- [x] **Phase 5:** All acceptance tests migrated off old sync server — sync server deleted, 920 tests passing
- [x] **Phase 5:** Old sync `listen()` entrypoint deleted — no remnant serve loop — commit 81f8bf0
- [x] **Phase 5:** Hot-reload atomic swap implemented — `hot_reload_watcher()` + `rebuild_shared_state()` in http_server_async.rs, `NTNT_HOT_RELOAD_INTERVAL_MS` env var, spawned in `start_per_request_server()` — commits a1b088c, c0591b2
- [x] **Phase 6:** Module-level mutable state migration documented
- [x] **Phase 6:** All new env vars documented
- [x] All existing tests pass (893+) — 925 passing at commit 05ed172
- [x] Benchmark results documented — criterion micro-benchmarks + wrk throughput results in Performance Results section above
- [ ] PR passes CI — PR #17 open: https://github.com/ntntlang/ntnt/pull/17 (PR #16 was auto-closed by force-push during branch revert)

---

## Phase 1 Results

Benchmarks run on the ntnt dev machine (release build, criterion 0.5, 100 samples):

| Benchmark | Median |
|-----------|--------|
| `Interpreter::new()` — full construction + all 23 stdlib modules | **43.9 µs** |
| `new()` + eval trivial expression | **44.1 µs** |
| `new()` + define fn + call realistic handler | **53.3 µs** |

**Gate: PASS on dev machine. Hardware-conditional on deployment targets.**

43.9 µs is on fast modern x86. Construction is CPU-bound sequential work (HashMap insertions, no I/O) so it scales approximately linearly with single-core speed. Cache and memory pressure could cause super-linear slowdown on very constrained hardware, but the multipliers below already include generous margins. The gate decisions are therefore hardware-dependent:

| Hardware | Multiplier | Est. Construction | Decision |
|---|---|---|---|
| Fast dev/CI machine | 1× | ~44 µs | Proceed as-is |
| Mid-range VPS (2–4 vCPU) | 2–3× | 88–130 µs | Implement lazy builtin registration |
| Cheap VPS / single vCPU | 4–5× | 175–220 µs | Implement lazy builtin registration |
| Raspberry Pi 4 / ARM embedded | 8–10× | 350–440 µs | Warm interpreter pool |

**Consequence for Phase 2:** `Interpreter::new_for_request()` must be designed with lazy registration in mind from the start, even if it ships with full registration initially. The constructor should accept an optional module set so lazy registration can be added without an API change:

```rust
pub fn new_for_request(shared: &SharedState) -> Self {
    Self::new_for_request_with_modules(shared, None)  // None = all modules
}

pub fn new_for_request_with_modules(shared: &SharedState, modules: Option<&HashSet<String>>) -> Self {
    let mut interp = Interpreter::new_bare();
    interp.register_builtins_selective(modules);  // define_builtins() + define_stdlib()
    interp.define_builtin_types();                // Option, Result, etc. — REQUIRED
    // Seed type context from SharedState
    interp.structs = shared.structs.clone();
    interp.enums = shared.enums.clone();
    interp.type_aliases = shared.type_aliases.clone();
    interp.trait_definitions = shared.trait_definitions.clone();
    interp.trait_implementations = shared.trait_implementations.clone();
    interp.current_file = shared.main_source_file.clone();
    interp
}
```

This costs nothing now but prevents a breaking API change later when deploying to weaker hardware.

**Warm interpreter pool:** Not needed on the dev machine, but the architecture must not preclude it. `execute_request()` taking a `StoredHandler` (not a live interpreter) already supports this — a pool implementation would just supply a pre-warmed interpreter instead of constructing one. No API change required when/if needed.

The plan proceeds to Phase 2 as written, with the `new_for_request_with_modules()` API included in Phase 3.

---

*Design doc by Larri.*

---

## Decision (2026-02-28)

**PARKED — bridge model retained.**

Performance results did not justify the effort:
- Simple handlers: 51K req/s (per-request) vs 82K req/s (bridge) — 38% regression
- Pool partially closed the gap to 58K but benchmark real-workload was blocked by flatten_value bug
- Best realistic path (frozen stdlib cache) estimated at 75-80K — still a regression or marginal parity
- Original 10x prediction was not achievable; the bridge is efficient enough for current scale

Branch `feat/http-server-refactor` remains open for reference but will not be merged. Revisit when:
- Apps need genuine multi-core parallelism (sustained concurrent users with I/O-bound handlers)
- The flatten_value cycle detection bug is fixed as a standalone improvement (it's a correctness issue regardless)

**Key findings preserved for future work:**
- flatten_value cycle detection must use pointer identity, not function name
- define_stdlib() is ~40µs — frozen Arc<HashMap> stdlib cache is the right optimization
- execute_request_with needs current_file set before eval (Copilot comment)
