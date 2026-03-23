# DD-045: Job Worker Environment — Full App Context

**Status:** Complete
**Author:** Larri
**Created:** 2026-03-21
**Completed:** 2026-03-22
**PRs:** #44 (RuntimeCapability), #45 (worker interpreter), #46 (jobs() discovery), #49 (enqueue gating)

---

## Summary

Job perform blocks execute in a full application context. Workers evaluate the entire .tnt source file at startup (with server side-effects suppressed), giving perform blocks access to all imports, functions, constants, and variables defined in the application. This is the same model HTTP server workers use: each worker is an independent interpreter with the complete application loaded.

This DD also introduces a **capability system** that replaces scattered string-matching skip lists with structurally enforced declarations. Functions declare what they require; execution modes declare what they provide. The compiler enforces both.

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
3. Create Interpreter with ExecutionMode::Worker (bootstrap mode)
4. Evaluate the full AST — imports, helper functions, constants, and job
   registrations all execute normally. Server startup calls (listen, serve_static)
   and worker calls (work_async, work_jobs) are no-ops. The full application is loaded.
5. Switch interpreter to ExecutionMode::Job (execution mode)
6. Enter the job processing loop:
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
import "./lib/jobs.tnt"   // registers all jobs in JOB_RUNTIME
listen(8080)
```

```ntnt
// lib/jobs.tnt
import { fetch } from "std/http"
import { notify } from "./notifications.tnt"

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
import { notify } from "../lib/notifications.tnt"

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

When workers start, they need to evaluate the same files the main process evaluated. The source file stored in `JOB_RUNTIME` is the entrypoint (`server.tnt`). Evaluating it in `ExecutionMode::Worker` (bootstrap mode) naturally follows the same imports and `jobs()` calls, loading all job files — while suppressing `listen()`, `work_async()`, and other side-effecting calls. After eval, the interpreter switches to `ExecutionMode::Job` for actual job execution. No separate discovery mechanism needed — the worker just re-evaluates the app.

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
    /// Task spawning: spawn()
    TaskSpawning,
    /// Scheduled/delayed execution: schedule(), after()
    Scheduling,
    /// Start job worker loops: work_async, work_jobs, scale_workers
    JobWorkers,
    /// Configure job queues and discover job directories
    JobConfig,
}

impl ExecutionMode {
    pub fn capabilities(&self) -> &'static [RuntimeCapability] {
        use RuntimeCapability::*;
        match self {
            ExecutionMode::Normal     => &[HttpServer, HttpConfig, TaskSpawning, Scheduling, JobWorkers, JobConfig],
            ExecutionMode::HotReload  => &[HttpServer, HttpConfig, JobConfig],
            ExecutionMode::Worker     => &[HttpServer, HttpConfig, JobConfig],
            ExecutionMode::Job        => &[JobConfig],
            ExecutionMode::UnitTest   => &[TaskSpawning, JobConfig],
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
    requires: RuntimeCapability,
    /// Expected argument count (for dispatch matching)
    arity: AritySpec,
    /// The implementation — receives the interpreter and unevaluated argument expressions
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

    // --- HttpServer: port binding, static files ---
    self.register_action("listen",       HttpServer, Exact(1), Self::action_listen);
    self.register_action("serve_static", HttpServer, Exact(2), Self::action_serve_static);
    self.register_action("routes",       HttpServer, Exact(1), Self::action_routes);
    self.register_action("new_server",   HttpServer, Exact(0), Self::action_new_server);

    // --- HttpConfig: middleware, security, lifecycle handlers ---
    self.register_action("use_middleware", HttpConfig, Exact(1), Self::action_use_middleware);
    self.register_action("enable_cors",   HttpConfig, Range(0, 1), Self::action_enable_cors);
    self.register_action("enable_csp",    HttpConfig, Range(0, 1), Self::action_enable_csp);
    self.register_action("enable_auth",   HttpConfig, Exact(1), Self::action_enable_auth);
    self.register_action("on_shutdown",   HttpConfig, Exact(1), Self::action_on_shutdown);
    self.register_action("on_error",      HttpConfig, Exact(1), Self::action_on_error);

    // --- JobConfig: job directory discovery ---
    self.register_action("jobs",          JobConfig,  Exact(1), Self::action_jobs_directory);

    // NOTE: get/post/put/delete/patch/head/options are NOT registered here.
    // They have dual behavior: get("/route", handler) registers a route (HttpServer),
    // but get("http://...") makes an HTTP client call (no capability needed).
    // This dual dispatch is handled in the Expression::Call path — see below.
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
            if !self.execution_mode.has(action.requires) {
                return Ok(Value::Unit);  // silent no-op in this mode
            }
            return (action.handler)(self, arguments);
        }

        // HTTP method dual dispatch: get/post/etc. with route pattern → HttpServer,
        // with URL → falls through to normal function call (HTTP client, no capability)
        if HTTP_METHODS.contains(&name.as_str()) && arguments.len() == 2 {
            let pattern = self.eval_route_pattern(&arguments[0])?;
            if let Value::String(s) = &pattern {
                if s.starts_with('/') {
                    // Route registration — requires HttpServer
                    if !self.execution_mode.has(RuntimeCapability::HttpServer) {
                        return Ok(Value::Unit);
                    }
                    let handler = self.eval_expression(&arguments[1])?;
                    self.server_state.add_route(&name.to_uppercase(), s, handler);
                    return Ok(Value::Unit);
                }
                // URL string — fall through to normal function call (HTTP client)
            }
        }
    }

    // ... normal function eval (NativeFunctions, user functions, etc.) ...
}
```

This replaces the entire chain of `if name == "listen"` / `if name == "serve_static"` / etc. with a single table lookup plus the HTTP method dual-dispatch handler. The capability check happens automatically — there's no way to bypass it.

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

**Note on diff size:** Adding `requires: None` to ~350 existing NativeFunction registrations across 21 stdlib files is a large mechanical diff. This is done as a single preparatory commit ("add `requires` field to NativeFunction, default None everywhere") so the capability-specific changes remain small and focused.

#### Why No Checklist Needed

The structure makes the right thing automatic:

1. **Adding a server-action function** → you call `register_action()`, which requires a `RuntimeCapability` parameter. You can't register without declaring the capability. The other registrations in `define_server_actions()` show you exactly how.

2. **Adding a NativeFunction with side effects** → the `requires` field is in the struct literal. Every existing NativeFunction shows the pattern. The compiler requires the field.

3. **Adding a new execution mode** → `capabilities()` has a non-exhaustive match. The compiler forces you to define what the new mode can do.

4. **Adding a new capability** → you add an enum variant. The compiler forces `capabilities()` to handle it. The existing `register_action()` calls and NativeFunction registrations show the convention.

There's no separate list to maintain, no documentation to remember, no skip function to update. The architecture is the documentation.

### Per-Job Scoping

`eval_block()` already creates a child environment scope and restores the parent on exit. For job execution, we create a param scope (to inject payload params), then `eval_block()` creates its own child scope for the body's locals:

```
Worker interpreter environment (after eval):
├── imports: fetch, now, stringify, ...
├── constants: API_BASE = "https://..."
├── functions: build_headers, notify_slack, ...
│
├── Job execution 1:
│   ├── param scope (injected): order_id = "abc-123"
│   │   └── body scope (via eval_block): order = { ... }, temp locals
│   └── (both scopes destroyed after job completes)
│
├── Job execution 2:
│   ├── param scope (injected): order_id = "def-456"
│   │   └── body scope (via eval_block): order = { ... }, temp locals
│   └── (both scopes destroyed after job completes)
```

Each job runs in an isolated child scope. Locals from one job cannot leak into the next. But the parent scope — with all imports, functions, and constants — is always accessible.

### Source File Discovery

`JobRuntime` stores the source file path, set automatically when the interpreter evaluates a file containing job declarations:

```rust
pub struct JobRuntime {
    // ... existing fields ...
    /// Path to the entrypoint .tnt file (set during job registration).
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
    // Create a child scope for payload parameters
    let previous_env = Rc::clone(&interp.environment);
    interp.environment = Rc::new(RefCell::new(
        Environment::with_parent(Rc::clone(&previous_env))
    ));

    // Inject perform parameters from the payload
    for param in &def.perform_params {
        let val = payload.get(&param.name).cloned().unwrap_or(Value::Unit);
        interp.environment.borrow_mut().define(param.name.clone(), val);
    }

    // Evaluate the perform body — eval_block() creates its own child scope
    // for the body's locals, so params and locals are naturally separated.
    let body = def.perform_body.clone();
    let result = std::panic::catch_unwind(
        std::panic::AssertUnwindSafe(|| interp.eval_block(&body))
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
    let mut interp = create_job_interpreter();

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

fn create_job_interpreter() -> Interpreter {
    let source_path = JOB_RUNTIME.get_source_file()
        .expect("source file must be set before workers start");

    let source = std::fs::read_to_string(&source_path)
        .expect("failed to read source file for worker");

    let tokens: Vec<_> = Lexer::new(&source).collect();
    let ast = Parser::new(tokens).parse()
        .expect("failed to parse source file for worker");

    let mut interp = Interpreter::new();
    // Bootstrap in Worker mode — work_async()/work_jobs() are no-ops,
    // preventing recursive worker spawning during source evaluation.
    interp.set_execution_mode(ExecutionMode::Worker);
    interp.set_current_file(&source_path);
    interp.set_main_source_file(&source_path);
    interp.eval(&ast)
        .expect("failed to evaluate source file for worker");
    // Switch to Job mode for actual job execution semantics.
    interp.set_execution_mode(ExecutionMode::Job);

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

    // Child scope with error + attempt bindings — same pattern as execute_in_worker.
    // Errors are silently discarded (fire-and-forget).
}
```

---

## What Gets Deleted

| Item | Reason |
|------|--------|
| `execute_job_perform()` | Replaced by `execute_in_worker()` — scoped eval in worker interpreter |
| `execute_on_failure()` | Replaced by `execute_on_failure_in_worker()` |
| `should_skip_server_call()` | Replaced by server actions registry — capability check is structural |
| `should_skip_route_registration()` | Absorbed into HTTP method dual-dispatch in `Expression::Call` |
| NativeFunction string-matching skip block | Replaced by `requires` field on `Value::NativeFunction` |
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

Steps are ordered so each is independently testable and mergeable. CI catches regressions at each stage.

### Step 1a: RuntimeCapability Enum + ExecutionMode::Job

Foundational types. No behavioral change yet.

- [x] Define `RuntimeCapability` enum: `HttpServer`, `HttpConfig`, `Concurrency`, `JobWorkers`, `JobConfig`
- [x] Add `ExecutionMode::Job` variant
- [x] Implement `ExecutionMode::capabilities() -> &'static [RuntimeCapability]` for all modes
- [x] Implement `ExecutionMode::has(RuntimeCapability) -> bool`
- [x] Tests: verify each mode provides exactly the right capabilities (Normal has all, Job has only JobConfig, etc.)

### Step 1b: `requires` Field on NativeFunction

Large mechanical diff — add the field everywhere, defaulting to `None`. Behavioral change only for the 6 functions that get `Some(...)`.

- [x] Add `requires: Option<RuntimeCapability>` field to `Value::NativeFunction`
- [x] Preparatory commit: add `requires: None` to all ~350 NativeFunction registrations across 21 stdlib files
- [x] Set `requires: Some(Concurrency)` on `spawn`, `schedule`, `after` in std/concurrent
- [x] Set `requires: Some(JobWorkers)` on `work_async`, `work_jobs`, `scale_workers` in std/jobs
- [x] Update NativeFunction dispatch to check `requires` automatically
- [x] Delete the string-matching skip block for `spawn`/`schedule`/`after` in interpreter
- [x] Tests: verify mode-gated functions are no-ops in restricted modes
- [x] Tests: verify `requires: None` functions still work in all modes

### Step 1c: Server Actions Registry

Refactor the `if name ==` chain into action table. Same behavior, better structure.

- [x] Define `ServerAction` struct with `requires`, `arity`, and `handler`
- [x] Add action registry (`HashMap<String, ServerAction>`) to `Interpreter`
- [x] Implement `register_action()` — requires `RuntimeCapability` parameter
- [x] Implement `define_server_actions()` — register `listen`, `serve_static`, `routes`, `new_server`, `use_middleware`, `enable_cors`, `enable_csp`, `enable_auth`, `on_shutdown`, `on_error`
- [x] Extract each `if name == "X"` block into a standalone `action_X` method on `Interpreter`
- [x] Replace the `if name ==` chain in `Expression::Call` with action table lookup + automatic capability check
- [x] Handle HTTP method dual dispatch: `get("/route", handler)` → `HttpServer` capability, `get("http://url")` → fall through to normal function call (no capability)
- [x] Delete `should_skip_server_call()` — no longer needed
- [x] Delete `should_skip_route_registration()` — absorbed into dual-dispatch handler
- [x] Tests: all existing server function tests still pass
- [x] Tests: Job mode suppresses all server actions
- [x] Tests: `get("http://api.com/data")` still works as HTTP client in Job mode

### Step 2: Source File Tracking

Store the source file path in `JobRuntime`:

- [x] Add `source_file: Mutex<Option<String>>` to `JobRuntime`
- [x] Set `source_file` during `Statement::Job` evaluation (from `interpreter.main_source_file`)
- [x] Add `JOB_RUNTIME.get_source_file() -> Option<String>` accessor

### Step 3: Worker Interpreter Creation

Add `create_job_interpreter()`:

- [x] Read source file from `JOB_RUNTIME.get_source_file()`
- [x] Parse and evaluate with `ExecutionMode::Worker` (bootstrap), then switch to `ExecutionMode::Job`
- [x] Handle errors (file not found, parse error, eval error) with clear messages
- [x] Test: verify interpreter has imports, functions, constants after creation
- [x] Test: verify `HttpServer` functions are suppressed (listen, serve_static, routes)
- [x] Test: verify `Concurrency` functions are suppressed (spawn, schedule, after)
- [x] Test: verify `JobWorkers` functions are suppressed (work_async, work_jobs)
- [x] Test: verify `JobConfig` functions run normally (configure_queue)

### Step 4: Scoped Job Execution

Replace `execute_job_perform` with `execute_in_worker`:

- [x] Implement child-scope creation with parameter injection
- [x] `eval_block()` handles body-local scoping (creates its own child scope)
- [x] Implement scope cleanup (restore parent on success, error, and panic)
- [x] Replace all `execute_job_perform` call sites in `worker_loop`
- [x] Delete `execute_job_perform`
- [x] Test: perform block can call imported functions
- [x] Test: perform block can call user-defined functions from the file
- [x] Test: perform block can access top-level constants
- [x] Test: locals from one job execution don't leak to the next
- [x] Test: panic in perform block doesn't corrupt the worker interpreter

### Step 5: on_failure in Worker Context

Replace `execute_on_failure` with worker-scoped version:

- [x] Implement `execute_on_failure_in_worker` with child scope
- [x] Replace all `execute_on_failure` call sites
- [x] Delete `execute_on_failure`
- [x] Test: on_failure can call helper functions
- [x] Test: errors in on_failure don't affect subsequent job execution

### Step 6: ntnt worker CLI

Update `run_worker_command` in `main.rs`:

- [x] Set `ExecutionMode::Worker` when evaluating the source file (not Job — prevents recursive worker spawning)
- [x] Workers create their own interpreters via `create_job_interpreter`
- [x] Test: `ntnt worker server.tnt` starts cleanly without binding ports or spawning schedules

### Step 7: `jobs()` Directory Auto-Discovery

New feature — depends on `ExecutionMode::Job` existing but independent of worker interpreter changes. Implemented last so the core fix ships without blocking on a new feature.

- [x] Add `jobs()` as a server action with `JobConfig` capability in `define_server_actions()`
- [x] Implement `action_jobs_directory()` — scan directory recursively for `.tnt` files
- [x] Evaluate each job file in the current interpreter (registers jobs via `Statement::Job`)
- [x] `lib/` modules available to job files via import (same as route files)
- [x] Track file mtimes for hot-reload in dev mode (detect new/changed/deleted job files)
- [x] Test: `jobs("jobs/")` discovers and registers jobs from multiple files
- [x] Test: job files can import from `lib/` modules
- [x] Test: hot-reload picks up new job files added to the directory
- [x] Test: `jobs()` works in `ExecutionMode::Job` (workers re-discover on startup)

### Step 8: Documentation

- [x] Update AI_AGENT_GUIDE.md job system section
- [x] Update STDLIB_REFERENCE.md
- [x] Run `ntnt docs --generate`
- [x] Update DD-037 (main concurrency/jobs DD) to reflect the worker environment model and capability system
- [x] Remove DD-044 Fix A references (no longer applicable)

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
fn test_job_mode_suppresses_server_but_not_http_client() {
    // Evaluate a .tnt file with listen(), serve_static(), work_async()
    // Verify none of them execute (no port binding, no worker spawning)
    // Verify get("http://...") still works as HTTP client
}

#[test]
fn test_job_worker_panic_recovery() {
    // Run a job that panics
    // Verify the worker interpreter is still functional for the next job
}
```

### Integration Tests

- [x] Snowgauge app works with no imports inside perform blocks
- [x] Helper functions callable from perform blocks
- [x] Top-level constants accessible in perform blocks
- [x] `ntnt worker server.tnt` starts without side effects
- [x] Multiple workers process jobs correctly (each with independent interpreter)
- [x] HTTP client functions (fetch, get/post with URLs) work in job perform blocks

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
| `get/post/put/delete/patch/head/options()` | `HttpServer` | Registers route handlers — **only when first arg starts with `/`**. When first arg is a URL, falls through to HTTP client (no capability) |
| `use_middleware()` | `HttpConfig` | Registers middleware handler |
| `enable_cors()` | `HttpConfig` | Configures CORS policy |
| `enable_csp()` | `HttpConfig` | Configures Content Security Policy |
| `enable_auth()` | `HttpConfig` | Sets up auth routes and providers |
| `on_shutdown()` | `HttpConfig` | Registers shutdown handler |
| `on_error()` | `HttpConfig` | Registers error handler |
| `jobs()` | `JobConfig` | Discovers and registers job files (new, Step 7) |

**Total: 18 functions** (counting HTTP methods as 7)

### NativeFunctions with Capabilities

| Function | Module | `requires` | Rationale |
|----------|--------|-----------|-----------|
| `spawn()` | std/concurrent | `TaskSpawning` | Spawns OS thread (allowed in UnitTest for concurrency tests) |
| `schedule()` | std/concurrent | `Scheduling` | Starts recurring timer thread (skipped in UnitTest — would interfere with tests) |
| `after()` | std/concurrent | `Scheduling` | Spawns delayed one-shot thread (skipped in UnitTest) |
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

**Q: What about HTTP method dual behavior (`get("/route")` vs `get("http://url")`)?**
A: HTTP methods (get, post, etc.) are NOT registered in the server actions registry because they have dual behavior. When the first argument starts with `/`, it's a route registration (requires `HttpServer`). When it's a URL, it's an HTTP client call (no capability needed, must work everywhere including job workers). This dual dispatch is handled explicitly in the `Expression::Call` path, right after the action table lookup.
