# DD-045: Job Worker Environment — Full App Context

**Status:** Draft
**Author:** Larri
**Created:** 2026-03-21
**Branch:** `feat/job-worker-env`

---

## Summary

Job perform blocks execute in a full application context. Workers evaluate the entire .tnt source file at startup (with server side-effects suppressed), giving perform blocks access to all imports, functions, constants, and variables defined in the application. This is the same model HTTP server workers use: each worker is an independent interpreter with the complete application loaded.

---

## Motivation

A developer writes an application. It might be one file or dozens. It has imports, helper functions, constants, and job definitions — possibly spread across multiple files. The perform block should be able to call any function or reference any value defined in the application — the same way an HTTP route handler can. There is no separate "job environment." Jobs are part of the application.

```ntnt
import { fetch } from "std/http"
import { now } from "std/time"
import { stringify } from "std/json"

let API_BASE = "https://api.example.com"

fn build_headers(token) {
    return map { "Authorization": "Bearer #{token}", "Content-Type": "application/json" }
}

fn notify_slack(message) {
    fetch("#{API_BASE}/slack/post", map {
        "method": "POST",
        "headers": build_headers(env("SLACK_TOKEN")),
        "body": stringify(map { "text": message })
    })
}

job ProcessOrder on orders (retry: 3, timeout: 120) {
    perform(order_id) {
        let order = fetch("#{API_BASE}/orders/#{order_id}")
        // ... process ...
        notify_slack("Order #{order_id} processed at #{now()}")
    }
}
```

Everything in this file — `fetch`, `now`, `stringify`, `API_BASE`, `build_headers`, `notify_slack` — is available inside `perform`. No re-imports, no re-declarations, no ceremony.

---

## Architecture

### Worker Lifecycle

Each job worker thread follows this lifecycle:

```
1. Read the entrypoint .tnt file from disk
2. Parse → AST
3. Create Interpreter with ExecutionMode::Job
4. Evaluate the full AST — imports, jobs(), helper functions, constants, and job
   registrations all execute normally. Server calls (listen, serve_static,
   work_async) are no-ops. The full application is loaded.
5. Enter the job processing loop:
   a. Claim a job from the KV queue
   b. Look up the JobDefinition by name
   c. Evaluate the perform block in a child scope (locals isolated per job)
   d. Handle success/failure/retry
   e. Repeat
```

This is the same pattern HTTP server workers use. Each worker is an independent interpreter instance with its own `Rc<RefCell<Environment>>`. No cross-thread sharing, no `Send` constraints on the interpreter itself.

### Multi-File Job Organization

Real applications don't live in one file. Jobs follow the same progressive disclosure as routes:

**Small app — everything in one file:**
```ntnt
// server.tnt
import { fetch } from "std/http"

fn notify(msg) { ... }

job SendEmail on emails {
    perform(to, body) { ... }
}

listen(8080)
```

**Medium app — jobs in a separate file, explicitly imported:**
```ntnt
// server.tnt
import "lib/jobs.tnt"   // registers all jobs in JOB_RUNTIME
listen(8080)
```

```ntnt
// lib/jobs.tnt
import { fetch } from "std/http"
import { notify } from "lib/notifications.tnt"

job SendEmail on emails {
    perform(to, body) { notify("Sending to #{to}"); ... }
}
```

This already works today. `import` evaluates the file, hits `Statement::Job`, and registers it globally. The imported file's functions and imports are available in the perform body because they're defined in the same module scope.

**Large app — auto-discovered job directory:**
```
my-app/
├── server.tnt
├── lib/
│   └── notifications.tnt
└── jobs/
    ├── send_email.tnt
    ├── process_order.tnt
    └── generate_report.tnt
```

```ntnt
// server.tnt
jobs("jobs/")          // auto-discover and register all jobs
routes("routes/")      // auto-discover and register all routes
listen(8080)
```

`jobs("jobs/")` works exactly like `routes("routes/")`:
1. Scan the directory recursively for `.tnt` files
2. Evaluate each file (which registers its `job` declarations in `JOB_RUNTIME`)
3. Each file has its own imports and helper functions — they're available in its perform blocks
4. `lib/` modules are available to job files via import (same as route files)
5. Hot-reload picks up new/changed job files automatically (dev mode)

Each job file is self-contained:
```ntnt
// jobs/send_email.tnt
import { fetch } from "std/http"
import { notify } from "lib/notifications.tnt"

job SendEmail on emails (retry: 3) {
    perform(to, subject, body) {
        fetch("https://api.mailgun.net/v3/...", map { ... })
        notify("Email sent to #{to}")
    }
}
```

The progressive path:
- Start with jobs inline in your main file (zero ceremony)
- Move them to `lib/jobs.tnt` when the file gets big (one import)
- Move to `jobs/` directory when you have many jobs (one `jobs()` call, auto-discovery)

At no point do you re-learn anything. Each step is a natural reorganization, not a new concept.

#### Worker File Discovery

When workers start, they need to evaluate the same files the main process evaluated. The source file stored in `JOB_RUNTIME` is the entrypoint (`server.tnt`). Evaluating it in `ExecutionMode::Job` naturally follows the same imports and `jobs()` calls, loading all job files. No separate discovery mechanism needed — the worker just re-evaluates the app.

### Function Capabilities — Structurally Enforced

The current approach to execution modes — string matching in `should_skip_server_call()` and scattered `if name == "X"` checks — fails silently when new functions are added. A developer adds `enable_rate_limit()` to the interpreter and forgets to update the skip list. Job workers now try to configure rate limiting. Nobody notices until production.

**The fix: restructure the interpreter so that side-effecting functions can only be registered through a path that requires declaring their execution mode behavior.** No checklists, no documentation to remember. The code structure makes it obvious and the compiler makes it mandatory.

#### The Problem with the Current Structure

Today, side-effecting functions live in two places:

1. **Interpreter special-cases** — a long chain of `if name == "listen"` / `if name == "serve_static"` blocks in the `Expression::Call` eval path. Each one independently calls `should_skip_server_call()`.

2. **NativeFunction dispatch** — a string-matching block that skips `spawn`/`schedule`/`after` based on execution mode.

Both require manual coordination. If you add a function to category 1 and forget the skip check, it runs in all modes. If you add a function to category 2 and forget the string match, same thing. There's no structural forcing function.

#### Solution: Server Actions Registry

Replace the scattered `if name ==` special-cases with a **registered action table** that the interpreter dispatches through. Every server-related function goes through this table, and the table requires a capability declaration at registration time.

```rust
/// What a function needs from the runtime to execute.
/// Used as Option<RuntimeCapability> — None means "requires nothing, runs in all modes."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeCapability {
    /// Bind ports, accept connections, register routes, serve static files
    HttpServer,
    /// Configure middleware, CORS, CSP, auth, error/shutdown handlers
    HttpConfig,
    /// Spawn threads, schedule recurring work, delayed execution
    Concurrency,
    /// Start job worker loops
    JobWorkers,
    /// Configure job queues and discover job directories
    JobConfig,
}

impl ExecutionMode {
    pub fn capabilities(&self) -> &'static [RuntimeCapability] {
        use RuntimeCapability::*;
        match self {
            ExecutionMode::Normal     => &[HttpServer, HttpConfig, Concurrency, JobWorkers, JobConfig],
            ExecutionMode::Worker     => &[HttpConfig, JobConfig],
            ExecutionMode::Job        => &[JobConfig],
            ExecutionMode::HotReload  => &[HttpConfig, Concurrency, JobWorkers, JobConfig],
            ExecutionMode::UnitTest   => &[JobConfig],
        }
    }

    pub fn has(&self, cap: RuntimeCapability) -> bool {
        self.capabilities().contains(&cap)
    }
}
```

**Adding a new `ExecutionMode` variant:** The Rust compiler forces you to add a branch to `capabilities()`. You can't compile without deciding what the new mode can do.

**Adding a new `RuntimeCapability` variant:** Doesn't require touching mode definitions — capabilities are additive. But the new variant only has effect when a function requires it, which makes you think about which modes should include it.

#### Refactoring the Special-Case Functions

The interpreter currently has ~15 `if name == "X"` blocks for server functions. These get restructured into a dispatch table:

```rust
/// A server action: a named function with a capability requirement and an implementation.
struct ServerAction {
    capability: RuntimeCapability,
    /// Expected argument count (for dispatch matching)
    arity: AritySpec,
    /// The implementation — receives the interpreter and pre-evaluated args
    handler: fn(&mut Interpreter, &[Expression]) -> Result<Value>,
}

/// Arity matching for dispatch
enum AritySpec {
    Exact(usize),
    Range(usize, usize),  // min..=max
}
```

Registration happens once, in a `define_server_actions()` method:

```rust
fn define_server_actions(&mut self) {
    use RuntimeCapability::*;

    // --- HttpServer: port binding, route registration, static files ---
    self.register_action("listen",       HttpServer, Exact(1), Self::action_listen);
    self.register_action("serve_static", HttpServer, Exact(2), Self::action_serve_static);
    self.register_action("routes",       HttpServer, Exact(1), Self::action_routes);

    // --- HttpConfig: middleware, security, lifecycle handlers ---
    self.register_action("use_middleware", HttpConfig, Exact(1), Self::action_use_middleware);
    self.register_action("enable_cors",   HttpConfig, Range(0, 1), Self::action_enable_cors);
    self.register_action("enable_csp",    HttpConfig, Range(0, 1), Self::action_enable_csp);
    self.register_action("enable_auth",   HttpConfig, Exact(1), Self::action_enable_auth);
    self.register_action("on_shutdown",   HttpConfig, Exact(1), Self::action_on_shutdown);
    self.register_action("on_error",      HttpConfig, Exact(1), Self::action_on_error);

    // --- JobConfig: job directory discovery ---
    self.register_action("jobs",          JobConfig,  Exact(1), Self::action_jobs_directory);

    // Route registration (get, post, put, delete, patch, head, options)
    for method in &["get", "post", "put", "delete", "patch", "head", "options"] {
        self.register_action(method, HttpServer, Exact(2), Self::action_route_handler);
    }
}
```

**The key insight:** `register_action` takes a `RuntimeCapability` as a required parameter. You literally cannot register a server action without declaring what it needs. There's no path to add `enable_rate_limit()` without answering "what capability does this require?"

#### Dispatch: One Place, Automatic

The interpreter's `Expression::Call` handler checks the action table before falling through to normal function dispatch:

```rust
Expression::Call { function, arguments } => {
    if let Expression::Identifier(name) = function.as_ref() {
        // Server action dispatch — capability check is automatic
        if let Some(action) = self.get_action(name, arguments.len()) {
            if !self.execution_mode.has(action.capability) {
                return Ok(Value::Unit);  // silent no-op in this mode
            }
            return (action.handler)(self, arguments);
        }
    }

    // ... normal function eval (NativeFunctions, user functions, etc.) ...
}
```

This replaces the entire chain of `if name == "listen"` / `if name == "serve_static"` / etc. with a single table lookup. The capability check happens automatically — there's no way to bypass it.

#### NativeFunction Capabilities

For NativeFunctions in stdlib modules (`spawn`, `schedule`, `work_async`, etc.), the capability is declared at the `Value::NativeFunction` level:

```rust
Value::NativeFunction {
    name: String,
    arity: usize,
    max_arity: usize,
    func: fn(&[Value]) -> Result<Value>,
    /// Capability required to execute. None = runs in all modes.
    requires: Option<RuntimeCapability>,
}
```

When registering functions in stdlib modules:

```rust
// std/concurrent — requires Concurrency capability
module.insert("spawn".to_string(), Value::NativeFunction {
    name: "spawn".to_string(),
    arity: 1, max_arity: 1,
    func: |args| { ... },
    requires: Some(RuntimeCapability::Concurrency),
});

// std/json — no requirements, runs everywhere
module.insert("parse_json".to_string(), Value::NativeFunction {
    name: "parse_json".to_string(),
    arity: 1, max_arity: 1,
    func: |args| { ... },
    requires: None,
});
```

The NativeFunction dispatch checks this automatically:

```rust
Value::NativeFunction { name, func, requires, .. } => {
    if let Some(cap) = requires {
        if !self.execution_mode.has(*cap) {
            return Ok(Value::Unit);
        }
    }
    // ... normal arity check and dispatch ...
}
```

**Why this works:** When you add a new NativeFunction, the `requires` field is right there in the struct literal. You see it in every existing function. `requires: None` reads as "requires nothing." `requires: Some(HttpServer)` reads as "requires the HTTP server." The compiler enforces the field — forget it and the code won't compile.

#### Why No Checklist Needed

The structure makes the right thing automatic:

1. **Adding a server-action function** → you call `register_action()`, which requires a `RuntimeCapability` parameter. You can't register without declaring the capability. The other registrations in `define_server_actions()` show you exactly how.

2. **Adding a NativeFunction with side effects** → the `capability` field is in the struct literal. Every existing NativeFunction shows the pattern. The compiler requires the field.

3. **Adding a new execution mode** → `capabilities()` has a non-exhaustive match. The compiler forces you to define what the new mode can do.

4. **Adding a new capability** → you add an enum variant. The compiler forces `capabilities()` to handle it. The existing `register_action()` calls and NativeFunction registrations show the convention.

There's no separate list to maintain, no documentation to remember, no skip function to update. The architecture is the documentation.

### Per-Job Scoping

`eval_block()` already creates a child environment scope and restores the parent on exit. This means:

```
Worker interpreter environment (after eval):
├── imports: fetch, now, stringify, ...
├── constants: API_BASE = "https://..."
├── functions: build_headers, notify_slack, ...
│
├── Job execution 1 (child scope):
│   └── order_id = "abc-123"    ← injected from payload
│   └── order = { ... }         ← local to this job
│   └── (scope destroyed after job completes)
│
├── Job execution 2 (child scope):
│   └── order_id = "def-456"
│   └── order = { ... }
│   └── (scope destroyed after job completes)
```

Each job runs in an isolated child scope. Locals from one job cannot leak into the next. But the parent scope — with all imports, functions, and constants — is always accessible.

### Source File Discovery

`JobRuntime` stores the source file path, set automatically when the interpreter evaluates a file containing job declarations:

```rust
pub struct JobRuntime {
    // ... existing fields ...
    /// Path to the .tnt source file (set during job registration).
    /// Workers read and re-evaluate this file at startup.
    source_file: Mutex<Option<String>>,
}
```

The path is set during `Statement::Job` evaluation in the interpreter — the interpreter already knows its current file via `main_source_file`. The first job declaration stores it. Workers read it at startup.

### Worker Job Execution

Replace the current `execute_job_perform` (which creates a naked interpreter) with scoped evaluation in the worker's full interpreter:

```rust
fn execute_in_worker(
    interp: &mut Interpreter,
    def: &JobDefinition,
    payload: &HashMap<String, Value>,
) -> std::result::Result<Value, String> {
    // Create a child scope for this job execution
    let previous_env = Rc::clone(&interp.environment);
    interp.environment = Rc::new(RefCell::new(
        Environment::with_parent(Rc::clone(&previous_env))
    ));

    // Inject perform parameters from the payload
    for param in &def.perform_params {
        let val = payload.get(&param.name).cloned().unwrap_or(Value::Unit);
        interp.environment.borrow_mut().define(param.name.clone(), val);
    }

    // Evaluate the perform body
    let body = def.perform_body.clone();
    let result = std::panic::catch_unwind(
        std::panic::AssertUnwindSafe(|| interp.eval_block_inner(&body))
    );

    // Restore parent scope (cleanup even on panic)
    interp.environment = previous_env;

    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{}", e)),
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "job perform panicked".to_string()
            };
            Err(msg)
        }
    }
}
```

### Worker Loop

The worker loop changes from "claim job → create naked interpreter → run" to "claim job → run in existing interpreter":

```rust
fn worker_loop(
    kv_info: KvHandleInfo,
    band: BandConfig,
    queues: Option<Vec<String>>,
) {
    // Create a full interpreter for this worker thread
    let mut interp = create_job_interpreter(&kv_info);

    let kv_handle = kv_info.to_value();
    let poll_duration = Duration::from_millis(band.poll_interval_ms);

    loop {
        if is_current_task_cancelled() { break; }

        // ... claim job from KV (unchanged) ...

        // Look up the job definition
        let def = match JOB_RUNTIME.get_job(&job_type) { ... };

        // Execute in the worker's interpreter (scoped)
        let exec_result = execute_in_worker(&mut interp, &def, &payload);

        // ... handle success/failure/retry (unchanged) ...
    }
}

fn create_job_interpreter(kv_info: &KvHandleInfo) -> Interpreter {
    let source_path = JOB_RUNTIME.source_file.lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("source file must be set before workers start");

    let source = std::fs::read_to_string(&source_path)
        .expect("failed to read source file for worker");

    let tokens: Vec<_> = Lexer::new(&source).collect();
    let ast = Parser::new(tokens).parse()
        .expect("failed to parse source file for worker");

    let mut interp = Interpreter::new();
    interp.set_execution_mode(ExecutionMode::Job);
    interp.set_current_file(&source_path);
    interp.set_main_source_file(&source_path);
    interp.eval(&ast)
        .expect("failed to evaluate source file for worker");

    interp
}
```

### on_failure Handling

Same treatment. `on_failure` blocks run in the worker's interpreter in a child scope, with `error` and `attempt` injected as parameters:

```rust
fn execute_on_failure_in_worker(
    interp: &mut Interpreter,
    def: &JobDefinition,
    error: &str,
    attempt: i64,
) {
    let Some((params, body)) = def.on_failure.as_ref() else { return; };

    // Child scope with error + attempt bindings
    // ... same pattern as execute_in_worker ...
    // Errors are silently discarded (fire-and-forget)
}
```

---

## What Gets Deleted

| Item | Reason |
|------|--------|
| `execute_job_perform()` | Replaced by `execute_in_worker()` — scoped eval in worker interpreter |
| `execute_on_failure()` | Replaced by `execute_on_failure_in_worker()` |
| `should_skip_server_call()` | Replaced by server actions registry — capability check is structural |
| `should_skip_route_registration()` | Absorbed into action registry (`HttpServer` capability) |
| NativeFunction string-matching skip block | Replaced by `capability` field on `Value::NativeFunction` |
| `if name == "listen"` / `"serve_static"` / etc. chain | Replaced by action table dispatch in `Expression::Call` |
| DD-044 Fix A (import replay) | Unnecessary — worker interpreter already has all imports |

Fix A from DD-044 is no longer needed. The problem it solved (imports unavailable in perform blocks) doesn't exist when the worker has the full application loaded. Fix B remains relevant but is about `schedule()` / concurrency primitives, not the job system.

---

## What Stays Unchanged

- **Job declaration syntax** — `job Name on queue { perform(...) { ... } }` is unchanged
- **Enqueue API** — `enqueue()`, `enqueue_at()`, `enqueue_in()`, `enqueue_batch()` are unchanged
- **KV backend** — Redis/SQLite/Valkey store layer is unchanged
- **Priority queues** — Band configuration, priority ranges unchanged
- **Retry/backoff** — Retry counts, exponential/linear/constant backoff unchanged
- **Deduplication** — `unique: N` option unchanged
- **Job expiration** — `expires: N` option unchanged
- **Testing mode** — `assert_enqueued`, `drain_jobs`, `clear_jobs` unchanged
- **Observability** — `ntnt jobs list/inspect/retry/cancel` unchanged
- **Worker scaling** — `ntnt workers status/scale` unchanged
- **Streaming events** — `job.enqueued`, `job.started`, etc. unchanged
- **Worker bands** — Band-based worker pools unchanged

---

## Implementation Plan

### Step 1: RuntimeCapability Enum, Server Actions Registry, NativeFunction Capability Field

Replace `should_skip_server_call()` and the scattered `if name ==` chain with structural enforcement:

- [ ] Define `RuntimeCapability` enum: `HttpServer`, `HttpConfig`, `Concurrency`, `JobWorkers`, `JobConfig`
- [ ] Add `ExecutionMode::Job` variant
- [ ] Implement `ExecutionMode::capabilities() -> &'static [RuntimeCapability]` for all modes
- [ ] Implement `ExecutionMode::has(RuntimeCapability) -> bool`
- [ ] Define `ServerAction` struct with `capability`, `arity`, and `handler`
- [ ] Add action registry (`HashMap<String, ServerAction>`) to `Interpreter`
- [ ] Implement `register_action()` — requires `RuntimeCapability` parameter (impossible to skip)
- [ ] Implement `define_server_actions()` — move all `if name == "X"` blocks into registered action handlers
- [ ] Replace the `if name ==` chain in `Expression::Call` with single action table lookup + automatic capability check
- [ ] Add `capability: Option<RuntimeCapability>` field to `Value::NativeFunction`
- [ ] Update all NativeFunction registrations in stdlib modules to include `capability` field
- [ ] Update NativeFunction dispatch to check `capability` automatically
- [ ] Delete `should_skip_server_call()` — no longer needed
- [ ] Delete `should_skip_route_registration()` — absorbed into action registry
- [ ] Delete NativeFunction string-matching skip block — absorbed into `capability` field
- [ ] Update all existing `ExecutionMode` tests
- [ ] Add capability tests: verify each mode provides exactly the right capabilities

### Step 1b: `jobs()` Directory Auto-Discovery

Implement `jobs("jobs/")` following the same pattern as `routes("routes/")`:

- [ ] Add `jobs()` as a server action with `JobConfig` capability
- [ ] Implement `load_job_directory()` — scan directory recursively for `.tnt` files
- [ ] Evaluate each job file in the current interpreter (registers jobs via `Statement::Job`)
- [ ] `lib/` modules available to job files via import (same as route files)
- [ ] Track file mtimes for hot-reload in dev mode (detect new/changed/deleted job files)
- [ ] Test: `jobs("jobs/")` discovers and registers jobs from multiple files
- [ ] Test: job files can import from `lib/` modules
- [ ] Test: hot-reload picks up new job files added to the directory
- [ ] Test: `jobs()` works in `ExecutionMode::Job` (workers re-discover on startup)

### Step 2: Source File Tracking

Store the source file path in `JobRuntime`:

- [ ] Add `source_file: Mutex<Option<String>>` to `JobRuntime`
- [ ] Set `source_file` during `Statement::Job` evaluation (from `interpreter.main_source_file`)
- [ ] Add `JOB_RUNTIME.get_source_file() -> Option<String>` accessor

### Step 3: Worker Interpreter Creation

Add `create_job_interpreter()`:

- [ ] Read source file from `JOB_RUNTIME.source_file`
- [ ] Parse and evaluate with `ExecutionMode::Job`
- [ ] Handle errors (file not found, parse error, eval error) with clear messages
- [ ] Test: verify interpreter has imports, functions, constants after creation
- [ ] Test: verify `HttpServer` functions are suppressed (listen, serve_static, routes)
- [ ] Test: verify `Concurrency` functions are suppressed (spawn, schedule, after)
- [ ] Test: verify `JobWorkers` functions are suppressed (work_async, work_jobs)
- [ ] Test: verify `JobConfig` functions run normally (configure_queue)

### Step 4: Scoped Job Execution

Replace `execute_job_perform` with `execute_in_worker`:

- [ ] Implement child-scope creation with parameter injection
- [ ] Implement scope cleanup (restore parent on success, error, and panic)
- [ ] Replace all `execute_job_perform` call sites in `worker_loop`
- [ ] Delete `execute_job_perform`
- [ ] Test: perform block can call imported functions
- [ ] Test: perform block can call user-defined functions from the file
- [ ] Test: perform block can access top-level constants
- [ ] Test: locals from one job execution don't leak to the next
- [ ] Test: panic in perform block doesn't corrupt the worker interpreter

### Step 5: on_failure in Worker Context

Replace `execute_on_failure` with worker-scoped version:

- [ ] Implement `execute_on_failure_in_worker` with child scope
- [ ] Replace all `execute_on_failure` call sites
- [ ] Delete `execute_on_failure`
- [ ] Test: on_failure can call helper functions
- [ ] Test: errors in on_failure don't affect subsequent job execution

### Step 6: ntnt worker CLI

Update `run_worker_command` in `main.rs`:

- [ ] Set `ExecutionMode::Job` when evaluating the source file
- [ ] Workers create their own interpreters via `create_job_interpreter`
- [ ] Test: `ntnt worker server.tnt` starts cleanly without binding ports or spawning schedules

### Step 7: Documentation

- [ ] Update AI_AGENT_GUIDE.md job system section
- [ ] Update STDLIB_REFERENCE.md
- [ ] Run `ntnt docs --generate`
- [ ] Update DD-037 (main concurrency/jobs DD) to reflect the worker environment model and capability system
- [ ] Remove DD-044 Fix A references (no longer applicable)

---

## Testing Strategy

### Capability System Tests

```rust
#[test]
fn test_normal_mode_has_all_capabilities() {
    let mode = ExecutionMode::Normal;
    assert!(mode.has(RuntimeCapability::HttpServer));
    assert!(mode.has(RuntimeCapability::HttpConfig));
    assert!(mode.has(RuntimeCapability::Concurrency));
    assert!(mode.has(RuntimeCapability::JobWorkers));
    assert!(mode.has(RuntimeCapability::JobConfig));
}

#[test]
fn test_job_mode_only_has_job_config() {
    let mode = ExecutionMode::Job;
    assert!(!mode.has(RuntimeCapability::HttpServer));
    assert!(!mode.has(RuntimeCapability::HttpConfig));
    assert!(!mode.has(RuntimeCapability::Concurrency));
    assert!(!mode.has(RuntimeCapability::JobWorkers));
    assert!(mode.has(RuntimeCapability::JobConfig));
}

#[test]
fn test_worker_mode_has_http_and_job_config() {
    let mode = ExecutionMode::Worker;
    assert!(!mode.has(RuntimeCapability::HttpServer));
    assert!(mode.has(RuntimeCapability::HttpConfig));
    assert!(!mode.has(RuntimeCapability::Concurrency));
    assert!(!mode.has(RuntimeCapability::JobWorkers));
    assert!(mode.has(RuntimeCapability::JobConfig));
}
```

### Worker Environment Tests

```rust
#[test]
fn test_job_worker_has_imports() {
    // Create a .tnt file with imports + job definition
    // Create worker interpreter with ExecutionMode::Job
    // Verify imported functions are callable in perform body
}

#[test]
fn test_job_worker_has_user_functions() {
    // Create a .tnt file with helper functions + job definition
    // Verify perform body can call helper functions
}

#[test]
fn test_job_worker_has_constants() {
    // Create a .tnt file with top-level let bindings + job definition
    // Verify perform body can read constants
}

#[test]
fn test_job_scope_isolation() {
    // Run two jobs sequentially in the same worker interpreter
    // Verify locals from job 1 are not visible in job 2
}

#[test]
fn test_job_mode_skips_server_calls() {
    // Evaluate a .tnt file with listen(), serve_static(), work_async()
    // Verify none of them execute (no port binding, no worker spawning)
}

#[test]
fn test_job_worker_panic_recovery() {
    // Run a job that panics
    // Verify the worker interpreter is still functional for the next job
}
```

### Integration Tests

- [ ] Snowgauge app works with no imports inside perform blocks
- [ ] Helper functions callable from perform blocks
- [ ] Top-level constants accessible in perform blocks
- [ ] `ntnt worker server.tnt` starts without side effects
- [ ] Multiple workers process jobs correctly (each with independent interpreter)

---

## Complete Capability Mapping

Every function in the language, classified. 22 functions require a capability; everything else is `requires: None`.

### Server Actions (interpreter special-cases → action registry)

| Function | `requires` | Rationale |
|----------|-----------|-----------|
| `listen()` | `HttpServer` | Binds port, starts server |
| `serve_static()` | `HttpServer` | Registers static file dirs |
| `routes()` | `HttpServer` | Discovers and registers route handlers |
| `new_server()` | `HttpServer` | Resets server state |
| `get/post/put/delete/patch/head/options()` | `HttpServer` | Registers route handlers (7 functions) |
| `use_middleware()` | `HttpConfig` | Registers middleware handler |
| `enable_cors()` | `HttpConfig` | Configures CORS policy |
| `enable_csp()` | `HttpConfig` | Configures Content Security Policy |
| `enable_auth()` | `HttpConfig` | Sets up auth routes and providers |
| `on_shutdown()` | `HttpConfig` | Registers shutdown handler |
| `on_error()` | `HttpConfig` | Registers error handler |
| `jobs()` | `JobConfig` | Discovers and registers job files (new) |

**Total: 18 functions** (counting HTTP methods as 7)

### NativeFunctions with Capabilities

| Function | Module | `requires` | Rationale |
|----------|--------|-----------|-----------|
| `spawn()` | std/concurrent | `Concurrency` | Spawns OS thread |
| `schedule()` | std/concurrent | `Concurrency` | Starts recurring timer thread |
| `after()` | std/concurrent | `Concurrency` | Spawns delayed one-shot thread |
| `work_async()` | std/jobs | `JobWorkers` | Starts background worker threads |
| `work_jobs()` | std/jobs | `JobWorkers` | Starts blocking worker loop |
| `scale_workers()` | std/jobs | `JobWorkers` | Modifies live worker count |

**Total: 6 functions**

### NativeFunctions — `requires: None` (runs in all modes)

| Module | Count | Examples |
|--------|-------|---------|
| std/string | 61 | trim, split, replace, starts_with, ... |
| std/jobs | 37 | enqueue, enqueue_at, enqueue_batch, job_status, configure_queue, ... |
| std/time | 40 | now, format_time, parse_time, ... |
| std/auth | 34 | hash_password, verify_password, create_session, ... |
| std/http_server | 32 | json, html, redirect, parse_form, set_cookie, ... |
| std/math | 25 | abs, floor, ceil, sin, cos, ... |
| std/crypto | 23 | sha256, hmac, random_bytes, ... |
| std/concurrent | 13 | channel, send, recv, select, await_task, cancel_task, sleep_ms, ... |
| std/collections | 18 | map, keys, values, merge, group_by, ... |
| std/fs | 17 | read, write, exists, mkdir, ... |
| std/path | 11 | join, basename, dirname, extension, ... |
| std/kv | 10 | open, get, set, del, list, ... |
| std/postgres | 8 | query, execute, connect, ... |
| std/sqlite | 8 | query, execute, open, ... |
| std/url | 8 | parse_url, encode, decode, ... |
| std/log | 7 | info, warn, error, debug, ... |
| std/http | 6 | fetch, download, ... |
| std/json | 4 | parse_json, stringify, stringify_pretty, ... |
| std/env | 4 | env, set_env, ... |
| std/csv | 4 | parse_csv, generate_csv, ... |
| std/markdown | 2 | render_markdown, ... |

**Total: ~350+ functions**

### Interpreter Special-Cases — No Capability (pure transforms, stay as-is)

These are special-cased in the interpreter for closure-argument handling (HOFs) or template rendering, not for side effects. They don't need capabilities and don't move to the action registry:

| Function | Reason for special-case |
|----------|------------------------|
| `template()` | Resolves file path relative to .tnt file |
| `compile()` | Template compilation |
| `render()` | Template rendering |
| `filter()` | HOF — needs closure eval in current scope |
| `transform()` | HOF — same |
| `sort()` / `sort_desc()` | HOF — same |
| `find()` | HOF — same |
| `any()` / `all()` | HOF — same |
| `count()` | HOF — same |
| `reduce()` | HOF — same |
| `flat_map()` | HOF — same |
| `old()` | Contract system (postconditions) |

These run in all modes. No changes needed.

---

## Open Questions (Resolved)

**Q: Should we share infrastructure with HTTP server workers?**
A: No. The patterns are similar (read file → parse → eval with skip mode → process loop) but the concerns are different. HTTP workers handle request/response routing; job workers handle queue claiming and retry logic. Coupling them would create an unwanted dependency between the job system and the HTTP server. If a third system needs the same pattern later, we can extract a shared utility then.

**Q: What about `schedule()` closures?**
A: `schedule()` is a concurrency primitive, not part of the job system. It uses the capture-and-serialize model because it runs ad-hoc closures mid-execution. That isolation is correct for `schedule()`. Fix B from DD-044 (smart capture / free-variable analysis) is still relevant for `schedule()` but is a separate concern.

**Q: What about top-level mutable state (`let mut`)?**
A: Each worker has its own interpreter, so `let mut counter = 0` at file level is per-worker state. This is correct — it matches how HTTP server workers and systems like Sidekiq handle per-process state. Workers are independent; shared state goes through KV.
