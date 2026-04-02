# DD-059: Jobs Test Parallelism

**Status:** draft
**Author:** larri
**Created:** 2026-04-02

## Problem

CI runs the entire test suite with `--test-threads=1` (added in PR #68, commit ef3d79d) to work around shared global state in jobs tests. This serializes all 1,368 tests — including ~1,200 that have no shared state and could safely run in parallel. CI wall time is unnecessarily long.

## Root Cause

`JOB_RUNTIME` and `BATCH_RUNTIME` are global `LazyLock` statics. Jobs tests mutate this shared state (registering job types, initializing KV, creating batches, etc.). Without serialization, concurrent tests corrupt each other's state.

### Current State

- **112 of 139** jobs tests use `with_clean_runtime()` or `with_temp_kv()`, which acquire `TEST_LOCK` (a static Mutex) and call `JOB_RUNTIME.reset()` + `BATCH_RUNTIME.reset()`.
- **27 jobs tests** do NOT acquire `TEST_LOCK` — some touch global state unsafely, others are pure functions that don't need it.
- **~1,200 non-jobs tests** (typechecker, interpreter, parser, integration, CLI) have no shared state with jobs and are safe to parallelize.

## Proposed Fix

### Option A: `#[serial]` attribute (recommended)

Use the [`serial_test`](https://crates.io/crates/serial_test) crate to annotate jobs tests:

```rust
use serial_test::serial;

#[test]
#[serial]
fn test_enqueue_to_kv() {
    // ...
}
```

- Add `serial_test` as a dev dependency
- Add `#[serial]` to all 139 jobs tests
- Remove `TEST_LOCK` mutex (replaced by `#[serial]`)
- Remove `--test-threads=1` from CI
- Non-jobs tests run in parallel automatically

**Effort:** Low. Mechanical — add attribute to each test, remove mutex.

### Option B: Wrap remaining 27 tests in `with_clean_runtime`

- Audit each of the 27 tests without `TEST_LOCK`
- Pure function tests (backoff, parsing, band config): no change needed, they don't touch globals
- Tests that touch `JOB_RUNTIME` (enqueue, cancel, worker loop, etc.): wrap in `with_clean_runtime`
- Keep `TEST_LOCK` mutex, remove `--test-threads=1`

**Effort:** Low-medium. Need to verify each test's dependencies.

### Option C: Extract JOB_RUNTIME into a test-injectable parameter

- Refactor jobs functions to accept a `&JobRuntime` parameter instead of using the global
- Tests create isolated `JobRuntime` instances
- Most invasive but cleanest long-term

**Effort:** High. Touches every jobs function signature. Better suited for the `jobs/` module split (see below).

## Recommendation

**Option A** for immediate CI speedup. **Option C** as part of the larger `jobs.rs` → `jobs/` module refactor if/when that happens.

## Related

- `jobs.rs` is 10,607 lines (5,503 prod + 5,104 test). A `jobs/` module split would naturally create a `jobs/tests/` directory where test infrastructure can be more structured.
- PR #68 comment from Copilot flagged `--test-threads=1` as overly broad.

## Implementation Checklist

- [ ] Add `serial_test = "3"` to `[dev-dependencies]` in Cargo.toml
- [ ] Add `#[serial]` to all 139 jobs tests
- [ ] Remove `TEST_LOCK` static mutex from test module
- [ ] Remove `--test-threads=1` from `.github/workflows/ci.yml`
- [ ] Verify CI passes with parallel test execution
- [ ] Measure CI time improvement
