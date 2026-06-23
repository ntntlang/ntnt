# DD-061: Interpreter Performance Roadmap for ntnt

## Status: in_review

**Updated:** 2026-06-20
**Target baseline:** v0.4.11
**Theme:** lock template semantics first, then ship measured performance wins for current ntnt apps before bytecode/JIT work

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

The first tempting optimization target is `template(path, data)`, because it currently re-reads and re-parses stable external templates on the ergonomic path every app uses. Before optimizing that path, we should lock the template contract: syntax, escaping, partial resolution, error behavior, and docs. Performance work should not fossilize stale docs or accidental semantics.

This refresh turns DD-061 into a shippable sequence: first a template-system contract cleanup that preserves existing apps, then benchmark harness, then automatic `template()` caching and the remaining measured interpreter fast paths.

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

`compile(path)` / `render(compiled, data)` exists, but normal apps use `template(path, data)` directly. Current local app/repo inspection found more than 100 `template(...)` calls and no meaningful current use of manual `compile()` / `render()` in the apps that matter. That means the user-facing ergonomic path is still the expensive path.

**Performance implication:** Larri Dashboard-style pages that render layout + partials + detail views re-read and reparse stable templates repeatedly. This is likely the cleanest near-term win once the template contract is clarified.

### 2. The template syntax is mostly right, but docs and behavior need a contract pass

The current explicit template syntax is a good fit for ntnt:

- `{{expr}}` for escaped output
- `{{{expr}}}` for raw output
- `{{#if cond}}...{{#elif cond2}}...{{#else}}...{{/if}}`
- `{{#for item in items}}...{{#empty}}...{{/for}}`
- `{{> partial}}` and `{{> partial data_expr}}`
- filters such as `{{title | default("Untitled")}}`

The important design choice is to keep explicit `#if` and `#for` instead of adopting ambiguous Mustache sections like `{{#items}}...{{/items}}`. `{{#items}}` changes meaning by runtime type in Mustache-style engines; `{{#for item in items}}` and `{{#if items}}` are more predictable and more consistent with ntnt.

However, current docs and implementation are not perfectly aligned:

- `docs/STDLIB_REFERENCE.md` claims `{{#key}}...{{/key}}` sections, but implementation does not support that form.
- docs claim no literal `{{` escape, but the lexer supports `\{{` and `\}}` in template strings.
- inline triple-quoted template strings support both `{{expr}}` and `#{expr}`; external `.html` templates should document `{{expr}}` as canonical.
- partial lookup is useful but under-specified in docs.
- `{{#if}}` / `{{#elif}}` condition errors do not flow through the same strict/warn/forgiving template error helper as interpolation and loop errors.
- filters support both parenthesized and space-separated arguments; the docs should choose a canonical style while preserving compatibility.

**Performance implication:** automatic template AST caching needs stable semantics and dependency/invalidation rules, especially around partials. The first PR should fix the contract without breaking existing apps.

### 3. Environment lookup is still recursive string-keyed HashMap lookup with cloning

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

### 4. Function-call dispatch contains many string-special cases in the generic call path

`Expression::Call` currently checks identifiers for `old`, server actions, `template`, `compile`, `render`, path-relative `std/fs` functions, `filter`, `transform`, `sort`, and more before falling through to ordinary function evaluation.

That is maintainable today, but as a performance path it means every ordinary call flows through a growing special-case ladder.

**Performance implication:** hot template filters, helper functions, and stdlib calls pay generic dispatch cost even when the parser/runtime can recognize common direct-call shapes.

### 5. Route patterns are already compiled; avoid re-solving a solved problem

`src/stdlib/http_server.rs` already stores parsed route segments in `Route`. DD-061 should not prioritize “route matcher compilation” as the first win unless profiling proves it. The better route-layer target is request/response map construction and avoidable cloning, not route pattern parsing.

### 6. Template caching is partially present but not the cache current apps need

`src/stdlib/template.rs` has a global compiled-template cache keyed by explicit template ids from `compile(path)`. It checks mtime when a compiled template is retrieved.

That is not the same as an automatic path-keyed cache for ordinary `template(path, data)`. The current DD should distinguish:

- explicit user-managed compiled templates: already present
- automatic ergonomic `template()` AST/source cache: not done, high priority

---

## Template System Recommendation

The template language should be tightened, not redesigned.

### Keep as canonical

```html
{{expr}}                              <!-- escaped output -->
{{{expr}}}                            <!-- raw/pre-rendered HTML output -->
{{#if user_is_admin}}...{{/if}}
{{#if status == "draft"}}...{{#elif status == "live"}}...{{#else}}...{{/if}}
{{#for row in rows}}...{{#empty}}...{{/for}}
{{> nav}}
{{> card item}}
{{title | default("Untitled")}}
```

### Preserve as compatibility behavior

- `{{expr | default "Untitled"}}` space-separated filter arguments should continue to work, but parenthesized filter args should be documented as canonical.
- `#{expr}` inside inline triple-quoted ntnt template strings should continue to work for normal-string interpolation consistency, but external `.html` templates should use `{{expr}}`.
- Missing template variables should continue to render as empty strings for optional layout slots and partial data.
- `{{{expr}}}` and `{{expr | safe}}` / `{{expr | raw}}` should remain explicit raw-output escape hatches.

### Do not add now

- Do not add Mustache-style ambiguous sections such as `{{#items}}...{{/items}}`.
- Do not replace `template(path, data)` with mandatory manual `compile()` / `render()` calls.
- Do not introduce a full component/layout/slot system before the current performance work.
- Do not require typed `SafeHtml` before caching; that can be a separate future safety design.

---

## Design Principles

1. **Clarify semantics before optimizing them.** Template caching should preserve the template language we want, not stale docs or accidental behavior.
2. **Measure before and after every performance PR.** No “feels faster” commits.
3. **Optimize current apps first.** Dashboard/article/template/database workloads beat synthetic arithmetic loops.
4. **Preserve debuggability.** Tree-walking remains fine; bytecode comes later only with evidence.
5. **Prefer localized fast paths.** Template AST caching and direct native-call dispatch are lower risk than rewriting `Value` or the environment model immediately.
6. **Keep development invalidation correct.** Any cache must be boringly obvious in dev mode and stable in worker/prod mode.
7. **Do not fossilize bad semantics for speed.** Correctness, diagnostics, and current language behavior stay primary.

---

## Proposed PR Sequence

### PR 1: Template system contract cleanup before caching

**Goal:** make the existing template system consistent, documented, and predictable before optimizing `template()`.

This should be a docs/tests/runtime-semantics cleanup PR, not a syntax redesign. It should preserve existing apps and explicitly bless the current good shape: `template(path, data)`, escaped-by-default `{{expr}}`, explicit `#if` / `#for`, partials, and raw output as an opt-in.

Scope:

- [x] Update the canonical template docs/spec so implementation and docs agree:
  - [x] `{{expr}}` is escaped output.
  - [x] `{{{expr}}}` is raw/unescaped output.
  - [x] `{{#if cond}}`, `{{#elif cond}}`, `{{#else}}`, `{{/if}}` are explicit conditional forms.
  - [x] `{{#for item in items}}`, `{{#empty}}`, `{{/for}}` are explicit loop forms.
  - [x] `{{> partial}}` and `{{> partial data_expr}}` are partial forms.
  - [x] filters use parenthesized args as canonical, with space-separated args preserved as compatibility sugar.
  - [x] `\{{` and `\}}` produce literal braces where supported.
- [x] Remove or correct doc claims that Mustache `{{#key}}...{{/key}}` sections are supported.
- [x] Explicitly document that ambiguous Mustache sections are a non-goal for now; use `#if` or `#for` instead.
- [x] Document external `.html` templates as `{{expr}}`-first; keep `#{expr}` documented only for inline triple-quoted ntnt strings.
- [x] Document partial lookup order exactly as implemented, or adjust implementation/tests to match the chosen lookup order.
- [x] Normalize template error handling so interpolation, filters, loops, and `#if` / `#elif` conditions consistently honor strict/warn/forgiving mode.
- [x] Add regression tests for the chosen contract without changing existing app-visible syntax.

Likely files:

- `src/interpreter.rs` template error handling for `#if` / `#elif`
- `src/lexer.rs` only if docs reveal a small escaping/diagnostic mismatch
- `docs/syntax.toml`
- `src/interpreter.rs` `// @ntnt template` doc block
- generated docs from `./target/dev-release/ntnt docs --generate`
- `tests/language_features_tests.rs` or a focused template test file
- `docs/AI_AGENT_GUIDE.md` if the template guidance lives there too

Non-goals:

- no broad template syntax redesign
- no Mustache `{{#key}}...{{/key}}` sections
- no automatic template cache yet
- no breaking changes to existing apps
- no SafeHtml type system work

Verification:

```bash
cargo fmt
cargo build --profile dev-release
cargo test --test language_features_tests template
./target/dev-release/ntnt docs --generate
git diff --check
```

Acceptance criteria:

- [x] Existing app template syntax remains valid.
- [x] Docs no longer claim unsupported Mustache section behavior.
- [x] Partial lookup rules are precise enough for cache dependency tracking.
- [x] Template condition errors follow the same TypeMode policy as other template errors.
- [x] The follow-up cache PR has a stable semantic contract to preserve.

### PR 2: Benchmark harness and current-use-case baseline

**Goal:** create a repeatable benchmark suite before touching performance-sensitive code.

This should be a small PR that adds scripts/examples only. It should establish baseline numbers for v0.4.11+ and make future performance PRs honest.

Scope:

- [x] Add a benchmark script under `scripts/bench/` or `tools/bench/` that can build `dev-release`, start benchmark servers, run `wrk`, and write JSON/Markdown results.
- [x] Add representative ntnt benchmark apps under `examples/perf/` or `benchmarks/`:
  - [x] plaintext response
  - [x] small JSON response
  - [x] route param + map read
  - [x] compute loop (`for i in 0..N`) for interpreter-only cost
  - [x] external template render with layout + partial + loop
  - [x] template-heavy page with 100 row maps
  - [x] single PostgreSQL query handler, optional/gated by env
  - [x] multi-query handler, optional/gated by env
- [x] Capture interpreter-only CLI timings separately from HTTP throughput where practical.
- [x] Document how to run the suite locally and how to compare before/after.
- [x] Ensure benchmarks are opt-in and do not make normal CI flaky.

Likely files:

- `scripts/bench/run-benchmarks.py` or `scripts/bench/run-benchmarks.sh`
- `examples/perf/*.tnt`
- `examples/perf/views/*.html`
- `docs/AI_AGENT_GUIDE.md` or this DD for benchmark instructions

Verification:

```bash
cargo build --profile dev-release
python3 scripts/bench/run-benchmarks.py --quick
```

Acceptance criteria:

- [x] A future PR can run one command and produce comparable baseline/after numbers.
- [x] The suite includes at least one template-heavy route and one interpreter-only route.
- [x] DB benchmarks are skipped unless env config is present.

### PR 3: Automatic path-keyed template AST cache for `template()`

**Goal:** make the normal ergonomic template path fast without requiring apps to manually call `compile()`.

Current `template()` reparses the template every call. This PR should cache the parsed template expression / template parts for external template files by resolved path and invalidation metadata.

Scope:

- [x] Add a path-keyed template cache for `template(path, data)`.
- [x] Cache parsed template expression / `TemplatePart` AST, not only raw file contents.
- [x] Include partial dependency invalidation based on the PR 1 partial lookup contract.
- [x] In production/worker mode, avoid per-request `metadata()` checks when hot reload is disabled.
- [x] In development/hot-reload mode, invalidate by mtime and reload safely.
- [x] Preserve `compile()` / `render()` compatibility.
- [x] Preserve template error behavior in strict/warn/forgiving modes.
- [x] Add tests proving edits invalidate cached templates in dev mode.
- [x] Benchmark external template render before/after.

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

- [x] Existing `template(path, data)` behavior is unchanged.
- [x] Template-heavy benchmark improves meaningfully.
- [x] Dev edits still show up without restarting when hot reload is enabled.
- [x] Worker/prod mode does not stat stable templates on every render.
- [x] Partial edits invalidate the parent render path correctly.

### PR 4: Template render scope and loop fast-path cleanup

**Goal:** reduce per-render and per-loop environment churn in templates.

`render_template_with_data()` currently creates a new `Environment` scope and defines every data key. Template loops also create new scopes per iteration. This is correct but expensive for row-heavy pages.

Scope:

- [x] Measure cost of template data scope creation and per-row loop scope creation.
- [x] Add a template data lookup path that can read from a borrowed render context before falling back to interpreter environment, or otherwise reduce data-scope setup/cloning.
- [x] Reduce unnecessary `Value` clones when binding template data and loop metadata.
- [x] Keep template variable shadowing semantics unchanged.
- [x] Add regression tests for loop metadata, nested loops, missing vars, and parent-scope fallback.

Likely files:

- `src/interpreter.rs` template rendering section
- possibly a small `src/template_runtime.rs` helper module if extraction improves clarity
- template tests and perf examples

Acceptance criteria:

- [x] Row-heavy template benchmark improves.
- [x] Missing variables still render as empty where current template semantics require it.
- [x] Nested template loops and parent-scope references remain correct.

Candidate benchmark note: local 3s/3-run `wrk` pass at 16 connections/2 threads, comparing `origin/main` at `52554e9` to this PR's dev-release binary, showed `/template/layout` RPS `23807.29 -> 24701.15` (+3.8%) and `/template/rows` RPS `6053.89 -> 6420.77` (+6.1%), while non-template routes stayed within noise.

### PR 5: Targeted native/global call fast path

**Goal:** make common function calls cheaper without changing language semantics.

The interpreter already snapshots `builtin_bindings`, but ordinary identifier call evaluation still goes through generic expression evaluation and clones argument values before invoking `Value::NativeFunction`. A broad direct-call path for every stable native/global call is not automatically a win: it can spend more time proving the callee is unshadowed than it saves. The useful first slice is narrower and measurable: `len(identifier)` should not clone large arrays/maps just to compute their length.

Scope:

- [x] Profile common native calls in template/page workloads (`len`, string helpers, collections helpers, response builders, template filters).
- [x] Add a targeted fast path for `len(identifier)` after preserving normal callee shadowing semantics.
- [x] Avoid cloning large identifier-bound arrays/maps when only their length is needed.
- [x] Preserve user-defined shadowing behavior. If an app defines `len`, the app binding must win.
- [x] Add tests for builtin behavior, shadowing, type errors, and parent-scope lookup behavior.

Likely files:

- `src/interpreter.rs`
- focused interpreter tests
- `examples/perf/*` and benchmark harness docs for the native-call fixture

Acceptance criteria:

- [x] No behavior change for shadowing/imports.
- [x] Simple native-call benchmark improves.
- [x] Call dispatch code stays localized rather than more haunted.

Candidate benchmark note: local 3s/3-run `wrk` pass at 16 connections/2 threads, comparing `origin/main` plus the new `/native/calls` fixture to this PR's dev-release binary, showed `/native/calls` RPS `349.95 -> 1574.70` (+350.0%). Other routes stayed within normal local noise.

### PR 6: Array self-append fast path

**Goal:** eliminate the current O(n²) cliff for the common NTNT array-building idiom without changing syntax or observable semantics.

Profiling after PR 5 showed `arr = arr + [item]` is the clearest next target. A 20k append CLI microbenchmark took about `7.17s`, and per-append cost increased with array size (`35.6µs` at 2k, `358.4µs` at 20k), proving repeated full-array cloning. Active app scans found this pattern in Larri Dashboard, larri.net, Portugal Counter, and examples.

Scope:

- [x] Add benchmark coverage for array-building via repeated `arr = arr + [item]`.
- [x] Add a targeted interpreter fast path for assignment shaped like `identifier = same_identifier + [single_or_many_items]`.
- [x] Avoid cloning the existing array before appending; mutate the stored binding in place only after RHS item evaluation succeeds.
- [x] Preserve existing semantics for mutability, undefined variables, alias/copy behavior, nested scopes, returned assignment value, and error paths.
- [x] Keep ordinary `arr = other + [item]`, `arr = arr + other_array`, call-containing RHS arrays, and non-array `+` behavior on the generic path unless explicitly covered.

Likely files:

- `src/interpreter.rs`
- focused interpreter tests
- `examples/perf/*` / benchmark harness docs for the array-build fixture

Acceptance criteria:

- [x] Array append microbenchmark curve becomes near-linear.
- [x] Existing array concatenation semantics remain unchanged outside the targeted assignment shape.
- [x] Tests cover mutability, alias preservation, nested-scope assignment, RHS failure/no-partial-mutation, call-RHS fallback semantics, and assignment expression return value.

Candidate benchmark note: local CLI microbenchmarks with the dev-release binary showed the 20k append fixture improve from `7.11s` before this PR to median `0.0122s` after this PR. Larger after-only checks stayed near-linear: 50k appends median `0.0238s`; 100k appends median `0.0439s`.

### PR 7: String self-concat fast path

**Goal:** reduce repeated string-building cliffs in manual HTML/XML/SVG routes.

A follow-up microbenchmark showed `s = s + "x"` also grows non-linearly (`0.020s` at 20k, `2.49s` at 200k). Active app scans found manual string accumulation in larri.net blog/sitemap routes and Larri Dashboard admin/SVG/API code.

Scope:

- [ ] Add a string-building benchmark fixture.
- [ ] Add a targeted fast path for `s = s + piece` when `s` currently resolves to a mutable string binding.
- [ ] Preserve TypeMode behavior for implicit string conversions and return/assignment semantics.
- [ ] Add tests for strict/warn/forgiving behavior, RHS failure, alias preservation, and nested scopes.

Acceptance criteria:

- [ ] Repeated string self-concat benchmark improves materially.
- [ ] Existing `+` behavior and TypeMode diagnostics remain unchanged.

### PR 8: Environment lookup measurement + low-risk lookup cache

**Goal:** reduce repeated recursive name lookup where semantics are stable, after clone/allocation cliffs are handled.

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

Acceptance criteria:

- [ ] Lookup-heavy benchmark improves.
- [ ] Shadowing and mutation semantics remain unchanged.
- [ ] The implementation is easy to remove if the benchmark delta is weak.

### PR 9: Request/response allocation cleanup

**Goal:** reduce cloning/allocation in current HTTP request paths after template/call/collection wins are measured.

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

### PR 10: Decide whether deeper interpreter work is justified

Only after PRs 1-9 have benchmark data should we choose one of:

- symbol interning / binder metadata for locals
- slot-based local frames
- map/object shape versioning and inline caches
- lowered IR / bytecode spike

This should be a DD update or spike PR, not an automatic implementation.

Acceptance criteria:

- [ ] DD-061 includes measured deltas from PRs 3-9.
- [ ] The next deeper design is justified by remaining measured bottlenecks.
- [ ] We explicitly choose whether to keep optimizing the tree walker or start a bytecode/lowered-IR DD.

---

## Prioritization for Current Use Cases

| Priority | Work | Why |
|---|---|---|
| P0 | Template contract cleanup | Locks the semantics/docs before caching them; preserves existing apps while removing ambiguity. |
| P0 | Benchmark harness | Without this, every performance PR is vibes wearing a stopwatch costume. |
| P1 | Automatic `template()` AST cache | Directly targets dashboard/article/server-rendered apps and the current ergonomic path. |
| P1 | Template render scope/loop cleanup | Current apps render lists, dashboards, docs, and article grids heavily. |
| P2 | Array self-append fast path | Profiling found `arr = arr + [item]` has an O(n²) clone cliff and active apps use it heavily for route/API list shaping. |
| P2 | String self-concat fast path | Manual HTML/XML/SVG builders show the same growth pattern; do after array append because TypeMode string coercion is trickier. |
| P3 | Environment lookup cache/instrumentation | Function calls and lookup are still expensive, but clone/allocation cliffs are more clearly measurable first. |
| P3 | Request/response allocation cleanup | Useful after template/call/collection costs are reduced; likely smaller than template parse and self-append wins. |
| P4 | Bytecode/lowered IR | Powerful later; premature until tree-walker wins are measured. |

---

## Measurement Plan

PR 1 is semantic/docs cleanup and should be verified by tests/docs drift checks, not throughput benchmarks. Every implementation performance PR after the benchmark harness should include a small benchmark table in the PR body:

```text
Benchmark                         main/v0.4.11+     branch        delta
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
- No broad template language redesign before caching.
- No ambiguous Mustache `{{#key}}...{{/key}}` sections in the cleanup PR.
- No “optimize by disabling diagnostics” trickery.
- No broad rewrite of `Value` or `Environment` without benchmark evidence.
- No production-only behavior that makes development impossible to reason about.

---

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| We optimize stale or accidental template semantics | PR 1 locks docs/tests/error behavior before caching. |
| Caching returns stale templates | Separate dev/prod invalidation rules; add mtime/hot-reload tests. |
| Partial edits fail to invalidate cached parents | PR 1 defines partial lookup; PR 3 tracks dependencies or invalidates conservatively. |
| Fast paths break shadowing/import semantics | Add focused tests for shadowing, imports, libs, route modules, and user-defined helpers. |
| Benchmarks become flaky/noisy | Use quick local benchmarks for direction and compare medians; keep DB benchmarks opt-in. |
| Performance work bloats interpreter complexity | Require simplification pass and readable helper extraction in every PR. |
| We optimize the wrong workload | Benchmark Larri Dashboard/article/template-shaped routes, not just arithmetic loops. |
| Bytecode temptation derails smaller wins | Explicitly defer bytecode until after measured PRs 3-7. |

---

## Updated Definition of Done

- [x] PR 1 clarifies and tests the template contract without breaking existing apps.
- [x] PR 2 adds a repeatable benchmark harness and baseline results.
- [x] PR 3 makes ordinary `template(path, data)` use a safe automatic AST/cache path.
- [x] PR 4 reduces template render/loop scope overhead without semantic drift.
- [x] PR 5 adds a safe targeted native/global call fast path for `len(identifier)`.
- [x] PR 6 removes the O(n²) array self-append cliff for `arr = arr + [item]`.
- [ ] PR 7 removes the repeated string self-concat cliff if measurements justify it after PR 6.
- [ ] PR 8 adds lookup instrumentation/cache only if measurements justify it.
- [ ] PR 9 cleans request/response allocation only if profiles show meaningful headroom.
- [ ] DD-061 is updated after each merged PR with measured deltas and completed checkboxes.
- [ ] A final follow-up decision chooses whether deeper symbol/binder/slot/bytecode work is worth a separate DD.

---

## Current Recommendation

With the automatic template AST cache, loop-scope cleanup, targeted `len(identifier)` fast path, and array self-append fast path complete, use the same evidence gate for **PR 7: string self-concat fast path** next. Do not start broader lookup/cache work until the remaining clone/allocation cliffs are either fixed or rejected by measurements.
