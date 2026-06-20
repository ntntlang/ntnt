# DD-061: Interpreter Performance Roadmap for ntnt

## Status: in_review

**Updated:** 2026-06-20
**Target baseline:** v0.4.11
**Theme:** measurable performance wins for current ntnt apps before bytecode/JIT work

---

## Problem

ntnt already shipped the big runtime-shell improvements: async HTTP serving, worker interpreters, production-mode hot-reload suppression, and async PostgreSQL pooling. Those wins made DB-heavy HTTP workloads much more viable, but the language still pays tree-walking interpreter costs on every request.

For the current body of ntnt use cases, the hottest practical workloads are not abstract compute benchmarks. They are:

- server-rendered pages using `template("views/...")`
- admin/dashboard pages with many helper calls and map field reads
- article/list pages with loops, filters, Markdown/string helpers, and DB rows
- JSON/API endpoints with route params and map indexing
- single-query and moderate multi-query PostgreSQL handlers
- file-routed apps with route/module/template discovery in dev and stable worker execution in production

The current DD named the right broad categories, but it was too “interpreter theory” shaped and not specific enough about the PRs that would materially improve the apps we actually run. This refresh turns DD-061 into a shippable sequence.

---

## Current Baseline Observations

These are based on v0.4.11 source inspection and current app usage patterns.

### 1. External `template()` still pays repeated file + parse work

`src/interpreter.rs` currently handles `template(path, data)` by:

1. evaluating `path` and `data`
2. cloning the data map
3. loading the template file through `std/template::load_template_file(...)`
4. calling `render_template_with_data(...)`
5. wrapping the full template source in triple quotes
6. lexing and parsing it into an expression
7. evaluating the template expression in a fresh data scope

`compile(path)` / `render(compiled, data)` exists, but normal apps use `template(path, data)` directly. That means the user-facing ergonomic path is still the expensive path.

**Performance implication:** Larri Dashboard-style pages that render layout + partials + detail views re-read and reparse stable templates repeatedly. This is likely the cleanest near-term win.

### 2. Environment lookup is still recursive string-keyed HashMap lookup with cloning

`Environment` is currently:

- `HashMap<String, Value>` for values
- `HashSet<String>` for mutability
- optional parent `Rc<RefCell<Environment>>`

`Environment::get(name)` recursively walks parent scopes and returns `Value` by clone. This is clear and correct, but it means variable-heavy template and handler execution repeatedly pays:

- string hashing/comparison
- `RefCell` borrow boundaries
- scope-chain traversal
- `Value` cloning

**Performance implication:** template loops, helper-heavy admin pages, and route functions with repeated globals/prelude/native calls all pay lookup overhead.

### 3. Function-call dispatch contains many string-special cases in the generic call path

`Expression::Call` currently checks identifiers for `old`, server actions, `template`, `compile`, `render`, path-relative `std/fs` functions, `filter`, `transform`, `sort`, and more before falling through to ordinary function evaluation.

That is maintainable today, but as a performance path it means every ordinary call flows through a growing special-case ladder.

**Performance implication:** hot template filters, helper functions, and stdlib calls pay generic dispatch cost even when the parser/runtime can recognize common direct-call shapes.

### 4. Route patterns are already compiled; avoid re-solving a solved problem

`src/stdlib/http_server.rs` already stores parsed route segments in `Route`. DD-061 should not prioritize “route matcher compilation” as the first win unless profiling proves it. The better route-layer target is request/response map construction and avoidable cloning, not route pattern parsing.

### 5. Template caching is partially present but not the cache current apps need

`src/stdlib/template.rs` has a global compiled-template cache keyed by explicit template ids from `compile(path)`. It checks mtime when a compiled template is retrieved.

That is not the same as an automatic path-keyed cache for ordinary `template(path, data)`. The current DD should distinguish:

- explicit user-managed compiled templates: already present
- automatic ergonomic `template()` AST/source cache: not done, high priority

---

## Design Principles

1. **Measure before and after every PR.** No “feels faster” commits.
2. **Optimize current apps first.** Dashboard/article/template/database workloads beat synthetic arithmetic loops.
3. **Preserve debuggability.** Tree-walking remains fine; bytecode comes later only with evidence.
4. **Prefer localized fast paths.** Template AST caching and direct native-call dispatch are lower risk than rewriting `Value` or the environment model immediately.
5. **Keep development invalidation correct.** Any cache must be boringly obvious in dev mode and stable in worker/prod mode.
6. **Do not fossilize bad semantics for speed.** Correctness, diagnostics, and current language behavior stay primary.

---

## Proposed PR Sequence

### PR 1: Benchmark harness and current-use-case baseline

**Goal:** create a repeatable benchmark suite before touching performance-sensitive code.

This should be a small PR that adds scripts/examples only. It should establish baseline numbers for v0.4.11 and make future performance PRs honest.

Scope:

- [ ] Add a benchmark script under `scripts/bench/` or `tools/bench/` that can build `dev-release`, start benchmark servers, run `wrk`, and write JSON/Markdown results.
- [ ] Add representative ntnt benchmark apps under `examples/perf/` or `benchmarks/`:
  - [ ] plaintext response
  - [ ] small JSON response
  - [ ] route param + map read
  - [ ] compute loop (`for i in 0..N`) for interpreter-only cost
  - [ ] external template render with layout + partial + loop
  - [ ] template-heavy page with 100 row maps
  - [ ] single PostgreSQL query handler, optional/gated by env
  - [ ] multi-query handler, optional/gated by env
- [ ] Capture interpreter-only CLI timings separately from HTTP throughput where practical.
- [ ] Document how to run the suite locally and how to compare before/after.
- [ ] Ensure benchmarks are opt-in and do not make normal CI flaky.

Likely files:

- `scripts/bench/run-benchmarks.py` or `scripts/bench/run-benchmarks.sh`
- `examples/perf/*.tnt`
- `examples/perf/views/*.html`
- `docs/AI_AGENT_GUIDE.md` or `design-docs/dd-061-interpreter-performance-roadmap.md` for benchmark instructions

Verification:

```bash
cargo build --profile dev-release
python3 scripts/bench/run-benchmarks.py --quick
```

Acceptance criteria:

- [ ] A future PR can run one command and produce comparable baseline/after numbers.
- [ ] The suite includes at least one template-heavy route and one interpreter-only route.
- [ ] DB benchmarks are skipped unless env config is present.

### PR 2: Automatic path-keyed template AST cache for `template()`

**Goal:** make the normal ergonomic template path fast without requiring apps to manually call `compile()`.

Current `template()` reparses the template every call. This PR should cache the parsed template expression / template parts for external template files by resolved path and invalidation metadata.

Scope:

- [ ] Add a path-keyed template cache for `template(path, data)`.
- [ ] Cache parsed template expression / `TemplatePart` AST, not only raw file contents.
- [ ] In production/worker mode, avoid per-request `metadata()` checks when hot reload is disabled.
- [ ] In development/hot-reload mode, invalidate by mtime and reload safely.
- [ ] Preserve `compile()` / `render()` compatibility.
- [ ] Preserve template error behavior in strict/warn/forgiving modes.
- [ ] Add tests proving edits invalidate cached templates in dev mode.
- [ ] Benchmark external template render before/after.

Likely files:

- `src/stdlib/template.rs`
- `src/interpreter.rs` (`template`, `compile`, `render`, `render_template_with_data`)
- `tests/language_features_tests.rs` or a focused template test file
- `examples/perf/*`

Non-goals:

- no new public API required
- no bytecode/lowered template VM
- no broad template syntax changes

Acceptance criteria:

- [ ] Existing `template(path, data)` behavior is unchanged.
- [ ] Template-heavy benchmark improves meaningfully.
- [ ] Dev edits still show up without restarting when hot reload is enabled.
- [ ] Worker/prod mode does not stat stable templates on every render.

### PR 3: Template render scope and loop fast-path cleanup

**Goal:** reduce per-render and per-loop environment churn in templates.

`render_template_with_data()` currently creates a new `Environment` scope and defines every data key. Template loops also create new scopes per iteration. This is correct but expensive for row-heavy pages.

Scope:

- [ ] Measure cost of template data scope creation and per-row loop scope creation.
- [ ] Add a template data lookup path that can read from a borrowed render context before falling back to interpreter environment, or otherwise reduce data-scope setup/cloning.
- [ ] Reduce unnecessary `Value` clones when binding template data and loop metadata.
- [ ] Keep template variable shadowing semantics unchanged.
- [ ] Add regression tests for loop metadata, nested loops, `{{#if}}`, missing vars, and parent-scope fallback.

Likely files:

- `src/interpreter.rs` template rendering section
- possibly a small `src/template_runtime.rs` helper module if extraction improves clarity
- template tests and perf examples

Acceptance criteria:

- [ ] Row-heavy template benchmark improves.
- [ ] Missing variables still render as empty where current template semantics require it.
- [ ] Nested template loops and parent-scope references remain correct.

### PR 4: Direct native/global call fast path

**Goal:** make common function calls cheaper without changing language semantics.

The interpreter already snapshots `builtin_bindings`, but ordinary identifier call evaluation still goes through generic expression evaluation and `Value::NativeFunction` call handling. We should add a constrained fast path for simple identifier calls where the callee is a stable native/global function.

Scope:

- [ ] Profile common native calls in template/page workloads (`len`, string helpers, collections helpers, response builders, template filters).
- [ ] Add a direct-call path for `Expression::Call { function: Identifier(name), ... }` after server-action/template/fs special cases are handled.
- [ ] Avoid re-looking-up stable native functions through recursive environment chains when the name is known to be an unshadowed builtin/prelude binding.
- [ ] Preserve user-defined shadowing behavior. If an app defines `len`, the app binding must win.
- [ ] Add tests for shadowing, imported functions, prelude functions, and ordinary user functions.

Likely files:

- `src/interpreter.rs`
- possibly a small call-dispatch helper to reduce the existing special-case ladder
- focused interpreter tests

Acceptance criteria:

- [ ] No behavior change for shadowing/imports.
- [ ] Simple native-call benchmark improves.
- [ ] Call dispatch code reads cleaner after the change, not more haunted.

### PR 5: Environment lookup measurement + low-risk lookup cache

**Goal:** reduce repeated recursive name lookup where semantics are stable.

Do not jump straight to a full binder/slot system. First add measurements and the smallest safe lookup cache.

Candidate shape:

- per-interpreter cache for global/prelude/native names that are not shadowed in the current local scope
- or per-call-frame local lookup helper that avoids repeated parent traversal for globals
- explicit cache invalidation when definitions/imports/libs mutate global scope

Scope:

- [ ] Measure lookup depth/frequency in representative routes and templates.
- [ ] Add instrumentation behind an env var such as `NTNT_PROFILE_LOOKUPS=1` if useful.
- [ ] Implement only a safe, local fast path with obvious invalidation.
- [ ] Add tests for shadowing, mutation, imports, libs, and route hot reload.

Likely files:

- `src/interpreter.rs`
- maybe `src/perf.rs` or a small internal instrumentation helper
- tests for environment semantics

Acceptance criteria:

- [ ] Lookup-heavy benchmark improves.
- [ ] Shadowing and mutation semantics remain unchanged.
- [ ] The implementation is easy to remove if the benchmark delta is weak.

### PR 6: Request/response allocation cleanup

**Goal:** reduce cloning/allocation in current HTTP request paths after template/call lookup wins are measured.

Scope:

- [ ] Profile request map construction and response conversion.
- [ ] Reduce avoidable `HashMap<String, Value>` and `String` clones in request/response helpers.
- [ ] Keep public request object shape unchanged.
- [ ] Avoid binary upload regressions; preserve `body_bytes` behavior for multipart paths.

Likely files:

- `src/stdlib/http_bridge.rs`
- `src/stdlib/http_server.rs`
- `src/stdlib/http_server_async.rs`
- `src/interpreter.rs` request handling paths

Acceptance criteria:

- [ ] Plaintext/JSON route benchmark improves or allocation profile improves clearly.
- [ ] Multipart/body byte tests remain green.
- [ ] Request maps keep the same user-visible fields.

### PR 7: Decide whether deeper interpreter work is justified

Only after PRs 1-6 have benchmark data should we choose one of:

- symbol interning / binder metadata for locals
- slot-based local frames
- map/object shape versioning and inline caches
- lowered IR / bytecode spike

This should be a DD update or spike PR, not an automatic implementation.

Acceptance criteria:

- [ ] DD-061 includes measured deltas from PRs 2-6.
- [ ] The next deeper design is justified by remaining measured bottlenecks.
- [ ] We explicitly choose whether to keep optimizing the tree walker or start a bytecode/lowered-IR DD.

---

## Prioritization for Current Use Cases

| Priority | Work | Why |
|---|---|---|
| P0 | Benchmark harness | Without this, every performance PR is vibes wearing a stopwatch costume. |
| P1 | Automatic `template()` AST cache | Directly targets dashboard/article/server-rendered apps and the current ergonomic path. |
| P1 | Template render scope/loop cleanup | Current apps render lists, dashboards, docs, and article grids heavily. |
| P2 | Direct native/global call fast path | Likely useful in templates and helper-heavy pages; must preserve shadowing. |
| P2 | Environment lookup cache/instrumentation | High theoretical ROI, but needs careful semantic guardrails. |
| P3 | Request/response allocation cleanup | Useful after template/call costs are reduced; likely smaller than template parse wins. |
| P4 | Bytecode/lowered IR | Powerful later; premature until tree-walker wins are measured. |

---

## Measurement Plan

Every implementation PR should include a small benchmark table in the PR body:

```text
Benchmark                         main/v0.4.11      branch        delta
plaintext route                   ...               ...           ...
small JSON route                  ...               ...           ...
route param + map read            ...               ...           ...
template layout + partial         ...               ...           ...
template 100-row loop             ...               ...           ...
compute loop                      ...               ...           ...
```

Required rules:

- Run each benchmark at least 3 times and report median or best-of-three consistently.
- Keep DB benchmarks separate and env-gated.
- Record machine/container context briefly.
- If a PR changes behavior or semantics to get speed, it is not a performance cleanup PR; it needs separate language-design review.

Recommended local toolchain:

- `cargo build --profile dev-release`
- `wrk` for HTTP throughput/latency
- `/usr/bin/time` for CLI/interpreter-only scripts
- `perf record` / `perf report` for targeted local profiling when available

---

## Non-Goals for This Roadmap Refresh

- No JIT.
- No bytecode VM in the first implementation PRs.
- No breaking changes to map, field, template, import, or shadowing semantics.
- No “optimize by disabling diagnostics” trickery.
- No broad rewrite of `Value` or `Environment` without benchmark evidence.
- No production-only behavior that makes development impossible to reason about.

---

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| Caching returns stale templates | Separate dev/prod invalidation rules; add mtime/hot-reload tests. |
| Fast paths break shadowing/import semantics | Add focused tests for shadowing, imports, libs, route modules, and user-defined helpers. |
| Benchmarks become flaky/noisy | Use quick local benchmarks for direction and compare medians; keep DB benchmarks opt-in. |
| Performance work bloats interpreter complexity | Require simplification pass and readable helper extraction in every PR. |
| We optimize the wrong workload | Benchmark Larri Dashboard/article/template-shaped routes, not just arithmetic loops. |
| Bytecode temptation derails smaller wins | Explicitly defer bytecode until after measured PRs 2-6. |

---

## Updated Definition of Done

- [ ] PR 1 adds a repeatable benchmark harness and baseline results.
- [ ] PR 2 makes ordinary `template(path, data)` use a safe automatic AST/cache path.
- [ ] PR 3 reduces template render/loop scope overhead without semantic drift.
- [ ] PR 4 adds a safe direct native/global call fast path, or documents why profiling does not justify it.
- [ ] PR 5 adds lookup instrumentation/cache only if measurements justify it.
- [ ] PR 6 cleans request/response allocation only if profiles show meaningful headroom.
- [ ] DD-061 is updated after each merged PR with measured deltas and completed checkboxes.
- [ ] A final follow-up decision chooses whether deeper symbol/binder/slot/bytecode work is worth a separate DD.

---

## Current Recommendation

Start with **PR 1: benchmark harness** immediately, then **PR 2: automatic `template()` AST cache**.

The template cache is the best first implementation target because it is:

- directly relevant to current server-rendered apps
- visible in source as repeated work on the normal ergonomic API
- lower risk than environment/frame representation changes
- benchmarkable with a contained route and template fixture
- compatible with the current tree-walking interpreter

After that, use benchmark data to decide whether template loop scopes, native-call dispatch, or environment lookup is the next real bottleneck. Computers are annoyingly literal; we should let them tell us where they hurt.
