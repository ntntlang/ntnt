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

### ExecutionMode::Job

A new execution mode that suppresses server and worker startup side-effects:

| Function | Behavior in Job mode |
|----------|---------------------|
| `listen()` | No-op (returns Unit) |
| `serve_static()` | No-op |
| `routes()` | No-op |
| `use_middleware()` | No-op |
| `enable_cors()` | No-op |
| `enable_csp()` | No-op |
| `enable_auth()` | No-op |
| `on_shutdown()` | No-op |
| `on_error()` | No-op |
| `work_async()` | No-op (prevents workers spawning workers) |
| `work_jobs()` | No-op (same reason) |
| `schedule()` | No-op (main process owns schedules) |
| `after()` | No-op |
| `spawn()` | No-op |
| `configure_queue()` | Runs normally (workers need the KV connection) |
| `import` statements | Run normally |
| `let` / `let mut` bindings | Run normally |
| `fn` definitions | Run normally |
| `job` declarations | Run normally (idempotent — first registration wins) |

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
| DD-044 Fix A (import replay) | Unnecessary — worker interpreter already has all imports |
| DD-044 Fix B (smart capture for schedule) | Separate concern — schedule closures are concurrency primitives, not jobs |

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

### Step 1: Add ExecutionMode::Job

Add the new execution mode to the interpreter:

- [ ] Add `Job` variant to `ExecutionMode` enum
- [ ] Update `should_skip_server_call()` — Job mode skips: `listen`, `serve_static`, `routes`, `use_middleware`, `enable_cors`, `enable_csp`, `enable_auth`, `on_shutdown`, `on_error`
- [ ] Update `should_skip_route_registration()` — Job mode skips route registration
- [ ] Update NativeFunction dispatch — Job mode skips: `work_async`, `work_jobs`, `schedule`, `after`, `spawn`
- [ ] Update existing `ExecutionMode` tests, add Job mode tests

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
- [ ] Test: verify server calls are suppressed (listen, work_async, etc.)

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

- [ ] Set `ExecutionMode::Job` instead of `Normal` when evaluating the source file
- [ ] Remove the full `interpreter.eval(&ast)` approach that runs the whole app (workers create their own interpreters)
- [ ] Or: use `ExecutionMode::Job` for the initial eval, then pass source path to workers
- [ ] Test: `ntnt worker server.tnt` starts cleanly without binding ports or spawning schedules

### Step 7: Documentation

- [ ] Update AI_AGENT_GUIDE.md job system section
- [ ] Update STDLIB_REFERENCE.md
- [ ] Run `ntnt docs --generate`
- [ ] Update DD-037 (main concurrency/jobs DD) to reflect the worker environment model
- [ ] Remove DD-044 Fix A references (no longer applicable)

---

## Testing Strategy

### Unit Tests

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

## Open Questions (Resolved)

**Q: Should we share infrastructure with HTTP server workers?**
A: No. The patterns are similar (read file → parse → eval with skip mode → process loop) but the concerns are different. HTTP workers handle request/response routing; job workers handle queue claiming and retry logic. Coupling them would create an unwanted dependency between the job system and the HTTP server. If a third system needs the same pattern later, we can extract a shared utility then.

**Q: What about `schedule()` closures?**
A: `schedule()` is a concurrency primitive, not part of the job system. It uses the capture-and-serialize model because it runs ad-hoc closures mid-execution. That isolation is correct for `schedule()`. Fix B from DD-044 (smart capture / free-variable analysis) is still relevant for `schedule()` but is a separate concern.

**Q: What about top-level mutable state (`let mut`)?**
A: Each worker has its own interpreter, so `let mut counter = 0` at file level is per-worker state. This is correct — it matches how HTTP server workers and systems like Sidekiq handle per-process state. Workers are independent; shared state goes through KV.
