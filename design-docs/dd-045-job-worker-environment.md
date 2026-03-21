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

A developer writes one .tnt file. It has imports, helper functions, constants, and job definitions. The perform block should be able to call any function or reference any value defined in that file — the same way an HTTP route handler can. There is no separate "job environment." Jobs are part of the application.

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
1. Read the .tnt source file from disk
2. Parse → AST
3. Create Interpreter with ExecutionMode::Job
4. Evaluate the full AST (imports, functions, constants, job registrations all run;
   server calls like listen(), serve_static(), work_async() are no-ops)
5. Enter the job processing loop:
   a. Claim a job from the KV queue
   b. Look up the JobDefinition by name
   c. Evaluate the perform block in a child scope (locals isolated per job)
   d. Handle success/failure/retry
   e. Repeat
```

This is the same pattern HTTP server workers use. Each worker is an independent interpreter instance with its own `Rc<RefCell<Environment>>`. No cross-thread sharing, no `Send` constraints on the interpreter itself.

### Function Capabilities — Making Execution Modes Self-Documenting

The current approach to execution modes — string matching in `should_skip_server_call()` and scattered `if name == "X"` checks — fails silently when new functions are added. A developer adds `enable_rate_limit()` to the interpreter and forgets to update the skip list. Job workers now try to configure rate limiting. Nobody notices until production.

**The fix: every function declares what it needs.** Instead of execution modes maintaining deny-lists, functions declare their capability requirements. The execution mode defines which capabilities are active. If a function requires a capability the mode doesn't provide, it's automatically a no-op.

#### Capability Definitions

```rust
/// What a function needs from the runtime to execute.
/// Functions declare these; execution modes provide them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeCapability {
    /// Can bind network ports and accept connections (listen, serve_static, routes)
    HttpServer,
    /// Can register middleware, CORS, CSP, auth, error handlers
    HttpConfig,
    /// Can spawn background threads or schedule recurring work
    Concurrency,
    /// Can start job worker loops (work_async, work_jobs)
    JobWorkers,
    /// Can register and configure job queues
    JobConfig,
}
```

#### Execution Mode → Capability Mapping

Each execution mode declares exactly which capabilities it provides:

```rust
impl ExecutionMode {
    /// Capabilities active in this mode.
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

Reading this table tells you immediately: Job mode can configure queues but can't start an HTTP server, can't spawn threads, and can't start workers. Normal mode can do everything. If we add a new mode later (e.g., `REPL`, `Preview`, `Migration`), you define its capabilities in one place.

#### Functions Declare Their Requirements

Where a function is registered or special-cased, it declares what it requires:

```rust
// In the interpreter's special-case handling:
if name == "listen" && arguments.len() == 1 {
    if !self.execution_mode.has(RuntimeCapability::HttpServer) {
        return Ok(Value::Unit);
    }
    // ... actual listen logic ...
}

if name == "serve_static" && arguments.len() == 2 {
    if !self.execution_mode.has(RuntimeCapability::HttpServer) {
        return Ok(Value::Unit);
    }
    // ... actual serve_static logic ...
}

if name == "use_middleware" && arguments.len() == 1 {
    if !self.execution_mode.has(RuntimeCapability::HttpConfig) {
        return Ok(Value::Unit);
    }
    // ... actual middleware logic ...
}
```

And for NativeFunctions dispatched through the module system:

```rust
// In NativeFunction dispatch (replaces the current string-matching block):
Value::NativeFunction { name: fn_name, .. } => {
    // Check function capability requirements
    if let Some(required) = function_capability(fn_name) {
        if !self.execution_mode.has(required) {
            return Ok(Value::Unit);
        }
    }
    // ... normal dispatch ...
}

/// Map NativeFunction names to their capability requirement.
/// Functions not listed here run in all modes.
fn function_capability(name: &str) -> Option<RuntimeCapability> {
    use RuntimeCapability::*;
    match name {
        // Concurrency primitives
        "spawn" | "schedule" | "after" => Some(Concurrency),
        // Job worker startup
        "work_async" | "work_jobs" => Some(JobWorkers),
        // Everything else runs unconditionally
        _ => None,
    }
}
```

#### Why This Design

1. **New functions are obvious.** When you add `enable_rate_limit()` to the interpreter, you write `if !self.execution_mode.has(RuntimeCapability::HttpConfig)` right next to the implementation. The capability check is co-located with the function — you can't miss it.

2. **New modes are obvious.** When you add `ExecutionMode::Repl`, you define its capabilities in one place: `&[Concurrency, JobConfig]`. You don't hunt through string lists.

3. **New capabilities are obvious.** When you add a new subsystem (e.g., `WebSocket`), you add one enum variant. The compiler forces you to handle it in the mode capability mapping. Every function that touches WebSocket gets `RuntimeCapability::WebSocket` as its guard.

4. **Compile-time safety.** The enum is exhaustive. `capabilities()` returns a static slice per mode. Adding a mode variant without defining its capabilities is a compile error (non-exhaustive match). Adding a capability variant prompts review of which modes should have it.

5. **Self-documenting.** The capability table in `ExecutionMode::capabilities()` is the single source of truth for what each mode can do. No scattered string lists. No separate skip functions. Reading the table tells you the full story.

#### Current Functions → Capabilities

| Function | Capability | Where Checked |
|----------|-----------|---------------|
| `listen()` | `HttpServer` | Interpreter special-case |
| `serve_static()` | `HttpServer` | Interpreter special-case |
| `routes()` | `HttpServer` | Interpreter special-case |
| `use_middleware()` | `HttpConfig` | Interpreter special-case |
| `enable_cors()` | `HttpConfig` | Interpreter special-case |
| `enable_csp()` | `HttpConfig` | Interpreter special-case |
| `enable_auth()` | `HttpConfig` | Interpreter special-case |
| `on_shutdown()` | `HttpConfig` | Interpreter special-case |
| `on_error()` | `HttpConfig` | Interpreter special-case |
| `spawn()` | `Concurrency` | NativeFunction dispatch |
| `schedule()` | `Concurrency` | NativeFunction dispatch |
| `after()` | `Concurrency` | NativeFunction dispatch |
| `work_async()` | `JobWorkers` | NativeFunction dispatch |
| `work_jobs()` | `JobWorkers` | NativeFunction dispatch |
| `configure_queue()` | `JobConfig` | NativeFunction dispatch |
| Route handlers (`get`, `post`, etc.) | `HttpServer` | Interpreter special-case (route registration) |

Functions without capability requirements (imports, `let`, `fn`, `job`, `print`, `fetch`, `parse_json`, all stdlib) run in every mode.

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
| `should_skip_server_call()` | Replaced by capability checks co-located with each function |
| NativeFunction string-matching skip block | Replaced by `function_capability()` lookup |
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

### Step 1: RuntimeCapability Enum and ExecutionMode Overhaul

Replace `should_skip_server_call()` with the capability system:

- [ ] Define `RuntimeCapability` enum: `HttpServer`, `HttpConfig`, `Concurrency`, `JobWorkers`, `JobConfig`
- [ ] Add `ExecutionMode::Job` variant
- [ ] Implement `ExecutionMode::capabilities() -> &'static [RuntimeCapability]` for all modes
- [ ] Implement `ExecutionMode::has(RuntimeCapability) -> bool`
- [ ] Delete `should_skip_server_call()` — replace all call sites with `self.execution_mode.has(cap)`
- [ ] Delete `should_skip_route_registration()` — replace with `!self.execution_mode.has(HttpServer)`
- [ ] Replace NativeFunction string-matching skip block with `function_capability()` lookup
- [ ] Update all existing `ExecutionMode` tests
- [ ] Add capability tests: verify each mode provides exactly the right capabilities
- [ ] Add test: adding a new `RuntimeCapability` variant without updating `capabilities()` fails to compile

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

## Adding New Features — The Capability Checklist

When adding a new function or subsystem to ntnt:

1. **Does it have side effects?** (binds ports, spawns threads, writes files, starts loops)
   - Yes → it needs a capability gate
   - No → it runs in all modes, no gate needed

2. **Which capability?** Pick the most specific existing one, or add a new variant if the function represents a genuinely new subsystem.

3. **Where to add the check:**
   - Interpreter special-case functions → `if !self.execution_mode.has(Cap) { return Ok(Value::Unit); }` right next to the implementation
   - NativeFunction modules → add to `function_capability()` mapping

4. **Update tests:** Add the new function to the capability test for each mode.

The compiler enforces the structural parts (new enum variants require exhaustive matches in `capabilities()`). The convention enforces the rest: every side-effecting function has a capability check co-located with its implementation.

---

## Open Questions (Resolved)

**Q: Should we share infrastructure with HTTP server workers?**
A: No. The patterns are similar (read file → parse → eval with skip mode → process loop) but the concerns are different. HTTP workers handle request/response routing; job workers handle queue claiming and retry logic. Coupling them would create an unwanted dependency between the job system and the HTTP server. If a third system needs the same pattern later, we can extract a shared utility then.

**Q: What about `schedule()` closures?**
A: `schedule()` is a concurrency primitive, not part of the job system. It uses the capture-and-serialize model because it runs ad-hoc closures mid-execution. That isolation is correct for `schedule()`. Fix B from DD-044 (smart capture / free-variable analysis) is still relevant for `schedule()` but is a separate concern.

**Q: What about top-level mutable state (`let mut`)?**
A: Each worker has its own interpreter, so `let mut counter = 0` at file level is per-worker state. This is correct — it matches how HTTP server workers and systems like Sidekiq handle per-process state. Workers are independent; shared state goes through KV.
