# DD-063: NTNT Language Assessment — Strengths, Gaps, and Priorities

## Status: draft (v2 — deep assessment)

## Problem

NTNT has grown to a substantial implementation (~117K lines of Rust, a full web-focused stdlib, contracts, intent verification) guided by the whitepaper's agent-native thesis. There has been no consolidated, evidence-based assessment of where the language actually stands: which bets are paying off, which design choices work against the stated goals, and — as this assessment discovered — where the project's own agent-facing documentation has drifted from the implementation. Without this, roadmap effort can drift toward new features while higher-leverage gaps (silent failure modes, type-reality mismatches, repair-loop diagnostics) go unaddressed, and agents are steered by stale instructions.

## Method

Three inputs, cross-checked against each other:

1. **Subsystem code review** — parser/lexer/lint, typechecker/runtime/contracts, IAL/intent engine, stdlib (all modules), with file:line evidence.
2. **Empirical behavior testing** — 14 experiments run against the `dev-release` binary (v0.4.10, commit d8557b5), specifically targeting the "critical syntax rules" claims in CLAUDE.md and common AI-generation mistakes. Results in the appendix.
3. **Survey of existing DDs and ROADMAP** (DD-047/048/049/050/054/058/060/061/062, language_comparison.md) so this assessment doesn't duplicate already-planned work.

A key lesson from the method itself: code review alone repeated several stale claims; only empirical testing caught that the implementation is **better hardened than its own documentation says**. Several findings below are doc bugs, not code bugs.

## Assessment

### What is working

**1. The thesis is right, and early.**
"Languages are designed for human authors, but AI is increasingly the author" is a real observation, and NTNT acts on it concretely rather than philosophically. The bet that agent-native ergonomics matter — unambiguous syntax, machine-checkable intent, a batteries-included web stack — strengthens every year.

**2. IAL is real tech, not a veneer.**
The intent engine (`src/ial/`) is genuine term rewriting: recursive resolution with parameter substitution, direct/indirect cycle detection with path reporting, MAX_DEPTH safety net, and resolution-trace collection (`src/ial/resolve.rs:86-330`). 37 unit tests cover the core including diamond patterns and parameterized cycles. The glossary extension mechanism (markdown table → user vocabulary) is clean and composable. `ntnt intent check` resolving "a user visits /" down to executable HTTP probes is the differentiated capability no mainstream language has. Caveats in the gaps section — but the foundation is solid engineering.

**3. The stdlib's security posture is genuinely strong — secure by default, not by discipline.**
- Parameterized queries only, in both DB drivers (`postgres.rs:536-567`, `sqlite.rs:117-198`) — no string-concatenation SQL path exists.
- Template `{{expr}}` auto-escapes HTML; raw output requires explicit `{{{expr}}}` or `| raw` (`interpreter.rs:7704, 10513`).
- bcrypt cost 12 / argon2id, AES-256-GCM, HMAC-signed sessions, challenge-bound CSRF tokens (`crypto.rs:298-368`, `auth.rs:1944-1989`).
- Security headers and body-size limits on by default; detailed errors suppressed in production (`http_server.rs:44-161`).

This is the right call for agent-written apps: the agent cannot forget to escape output or parameterize a query because there is no unsafe path to reach for.

**4. Documentation is build-enforced.**
`build.rs:544-580` fails the build on any undocumented stdlib function or orphaned doc block; 365+ functions are documented with zero gaps. Mandatory machine-readable docs are exactly the right invariant for a language whose primary consumers hallucinate APIs that lack documentation.

**5. Error reporting infrastructure is above its weight class.**
Error codes (E001–E012), source-context frames with carets, Levenshtein "did you mean 'print'?" suggestions for undefined variables/functions, and structured hints ("Use int(x) to convert"). Verified empirically: a typo'd function name produces a precise, actionable error. The infrastructure exists; the gaps (below) are about coverage, not architecture.

**6. The concurrency model is sound by construction.**
`spawn`/`channel`/`parallel` give each task an isolated interpreter; values are serialized across thread boundaries (`concurrent.rs:156-295, 1869-1988`). No shared interpreter state means no data races and no reentrancy hazards by design — a much better property for agent-generated concurrent code than "be careful with locks."

**7. Substantial hardening has already shipped — more than the agent docs admit.**
Empirically verified on v0.4.10: semicolons are harmless (optional, lint-warned); `for..in` on a string warns at runtime with a `chars()` hint; non-boolean `if` conditions warn (error in strict mode); bare `{ "a": 1 }` is a **loud parse error** with source context; `range()` is lint-warned; string+int concatenation warns with a `str()` hint. The "silent failure" picture painted by CLAUDE.md's critical-rules list is largely outdated — most of those failures are now loud.

**8. Contracts work as advertised.**
`requires`/`ensures`/`old()`/`result` evaluate correctly at runtime with clean failure messages. Eiffel's best idea, placed where agents will actually use it.

### What works against the goals

**A. Verified silent failures (the real list, post-testing).**

1. **`${name}` interpolation fails silently — the top remaining footgun.** `print("hello ${name}")` prints the literal text `hello ${name}`. No runtime warning. `ntnt lint` — including `--strict` — reports **no issues**, because the `javascript_template_string` rule requires a backtick on the same line (`main.rs:3821-3832`). This is the single most common interpolation syntax in the training distribution of every model, and NTNT swallows it without a sound.
2. **Out-of-bounds index and missing map key silently return `none` while the typechecker claims `T`.** `arr[10]` on a 2-element array prints `none` with no warning, but the typechecker infers the unwrapped element type, not `Option<T>` (acknowledged TODO at `typechecker.rs:1933-1937`). The type system and the runtime disagree about the language's most common operation. Related: `m["k"]` cannot distinguish stored-`None` from missing key (`has_key()` is the workaround, documented).
3. **Bare braces with a single expression silently form a block.** `let m = { 5 }` binds 5; `let e = {}` binds Unit — no warning at lint or runtime. (The multi-entry map case `{ "a": 1 }` errors loudly, so the blast radius is small, but the single-expression case is silent.)
4. **Unknown escape sequences degrade silently** (`"\k"` → literal `k`, no lint).

**B. The agent-facing docs have drifted from the implementation — and for this language, that's a product bug.**

CLAUDE.md's critical-rules list makes at least two claims that are false on v0.4.10:
- "`;` silently corrupts parser state. Never use semicolons." — Semicolons are tokenized, optionally consumed, and merely lint-warned (`parser.rs:245+`; regression-locked per DD-054). Verified: runs correctly.
- "`s.len()` WRONG — dot is for reading properties only." — Dot-call is full uniform-function-call sugar: `s.len()` returns 3, and `5.double()` works for *user-defined* functions. Verified empirically.

Additionally, `IAL_REFERENCE.md` documents `Sql` and `InvariantCheck` as available primitives, but `Sql` returns "not yet implemented" and invariant execution is a placeholder stub (`execute.rs:154, 1108-1122`).

For an agent-native language, the instruction files **are** part of the product surface: stale rules make agents avoid valid code, distrust accurate rules, and burn repair iterations on phantom footguns. Nothing currently prevents this drift — there is no mechanism tying documented behavioral claims to regression tests.

(Decision needed on UFCS specifically: either embrace dot-call sugar and document it, or make it error consistently. The current state — works fine, docs forbid it — is the worst of both.)

**C. Repair-loop diagnostics stop short of what the infrastructure could deliver.**

An agent's edit-lint-run loop is the core UX of this language. Today, each iteration is throttled by:
- **No parser error recovery** — one parse error reported per run (`main.rs:3402-3418`). N syntax errors cost N round trips.
- **Contract violations omit location and values** — `error[E004] Precondition failed in 'divide': b != 0` has no line number, no source frame, and doesn't print the actual arguments (`b` was 0). Every other error class got the rich treatment; E004 didn't.
- **Unknown method calls get no bridge hint** — `s.length()` errors loudly but doesn't suggest `len(s)`; the typechecker types unknown methods as `Any` silently rather than diagnosing (`typechecker.rs:1811-1893`).
- **IAL unknown terms get a bare error** — "Unknown term - not found in vocabulary," no near-miss suggestions despite the vocabulary being right there, and no `intent lint` to validate a glossary before execution (`resolve.rs:326-330`).

**D. The flagship has stubs behind documented features.**
Within IAL: invariant execution is unimplemented (parsing and expansion work; execution is a stub), the SQL primitive is unimplemented, and the property checks (deterministic/idempotent/round-trips) are run-twice-and-compare — honest smoke checks, but not property-based testing (no generators, no shrinking; acknowledged as Phase 4-5 in `ial_unit_testing.md`). The differentiated subsystem deserves either completed depth or narrowed documentation.

**E. Typechecker depth.**
Gradual typing with shallow heuristic inference: no cross-function inference without annotations, function generics unimplemented (DD-050 backlog — callbacks type as `Any`), no flow narrowing. Combined with A.2, annotations currently can't be trusted for index safety. Strictness defaults are reasonable for `run` (warn mode is genuinely loud), but nothing forces strict mode in verification contexts (`intent check`, CI).

**F. Contracts are runtime-only; the whitepaper promises more.**
No static or lint-level checking even of statically-obvious violations (`divide(x, 0)` against `requires b != 0`). Also note `ContractChecker::set_enabled(false)` exists (`contracts.rs:167`) — "contracts are law" in the whitepaper, optional in the implementation.

**G. Performance ceiling is real and now has a number.**
Tree-walking interpreter with clone-heavy values (arrays/maps deep-clone on assignment and parameter passing, `interpreter.rs:44-143`). Measured: recursive `fib(30)` takes 2.9s vs CPython's 0.083s — **~35x slower than CPython** on call-heavy code. Fine for I/O-bound web handlers (the target), but it bounds the "agents write compute too" story. DD-061 covers the remediation roadmap; this measurement is a useful baseline for it.

**H. Whitepaper over-promises relative to the implementation.**
Typed effects, session-typed concurrency protocols, structured AST edits, semantic-versioning enforcement, belief-shift observability — none implemented. The shipped subset (intent checking + contracts + secure web stack) is strong enough to stand alone; aspirational features should be explicitly labeled future work rather than implied capability.

**I. Ecosystem boundary remains the adoption ceiling.**
No FFI today; interop is `fetch` and shelling out. DD-058 catalogs the stdlib gaps (validation, email, multipart streaming, WebSocket/SSE); DD-062 designs the extension model but none is implemented. NTNT wins while the app fits in the stdlib and hits a wall at the first third-party SDK. This will matter more to adoption than any syntax decision.

## Where this lands

The implementation is **better than its own agent-facing documentation** — most of the famous footguns are already loud, security is genuinely default-safe, docs are build-enforced, and IAL is real engineering rather than demo-ware. The differentiated asset (the intent-verification loop) is worth continued investment and is closer to production-ready than the stub list suggests.

The highest-leverage problems are now narrow and concrete: one truly silent syntax failure (`${}`), one type-reality mismatch (index/`Option`), a drifted instruction file, and a repair loop that reports less than the error infrastructure can already express. None requires redesign; all are within reach of normal release work. The structural risks — performance ceiling, ecosystem boundary, whitepaper gap — are real but already have DDs or are presentation-level fixes.

## Recommendations (priority order)

Deduplicated against existing DDs; items already covered elsewhere are deferred, not repeated.

1. **Catch `${...}` in all strings.** Drop the backtick requirement from `javascript_template_string`; add a runtime warn (warn mode) when a string literal containing `${identifier}` is constructed. Hours of work; eliminates the worst remaining silent failure.
2. **Establish doc-claim regression tests, then fix the drift.** Every behavioral claim in CLAUDE.md's critical-rules list gets a test asserting the actual behavior (the DD-054 pattern, extended); CI fails when docs and implementation diverge. Immediately: correct the semicolon and method-call rules, and decide the UFCS story (recommend: embrace and document it — it works, it matches agent priors, and `req.params.id` property access coexists with it today).
3. **Resolve the index/`Option` type-reality mismatch.** Decide direction: typechecker infers `Option<T>` for index expressions (honest, but breaking), or strict mode errors on out-of-bounds at runtime, or at minimum lint warns on unguarded index results flowing into typed positions. Needs a maintainer call on breaking-change tolerance — flagging rather than prescribing.
4. **Upgrade the repair loop to match the error infrastructure.** (a) Parser error recovery, 3–5 errors per pass; (b) E004 contract violations get line numbers, source frames, and actual argument values; (c) unknown-method errors suggest the free-function form; typechecker diagnoses unknown methods instead of silently typing `Any`; (d) IAL unknown terms get Levenshtein suggestions from the loaded vocabulary, plus an `ntnt intent lint` that validates glossaries before execution.
5. **Close the IAL stub gap — in code or in docs.** Implement invariant execution (small: expansion already works) and either implement the SQL primitive or remove both from IAL_REFERENCE until real. The flagship feature should not document capabilities it doesn't have.
6. **Lint the silent block-binding case.** Warn when a bare-brace block expression is bound by `let` / passed as an argument (likely intended a map or has a stray brace).
7. **Strict mode in verification contexts.** Keep warn as the `run` default (empirically it is loud enough), but run `intent check` and `ntnt test` under strict type mode so verification means verification.
8. **Separate shipped from aspirational in the whitepaper.** Lead with what exists (intent verification, contracts, secure-by-default stack — a strong story on its own); move effects/session types/structured edits/semver enforcement to an explicit future-work section.
9. **Static contract lint (stretch).** Flag call sites that statically violate `requires` clauses with literal arguments. Narrows the vision gap cheaply; full verification stays out of scope.
10. **Performance and ecosystem: execute existing DDs.** DD-061 Phase 1 (the fib(30) ≈ 35x-CPython baseline above gives it a benchmark to beat); DD-058 Priority 1 stdlib gaps; DD-062 extension model. No new design needed here — they're the right plans; this assessment just confirms their priority.

## Non-goals

- No redesign proposals: hardening over novelty throughout.
- No changes to IAL/intent semantics — the architecture is the strength; the gaps are completion and error UX.
- Performance and stdlib-gap roadmaps live in DD-061/DD-058; this DD only contributes baselines and priority confirmation.

## Appendix: Empirical results (v0.4.10 dev-release, commit d8557b5)

| # | Experiment | Claimed behavior (CLAUDE.md / review) | Actual behavior | Verdict |
|---|------------|----------------------------------------|-----------------|---------|
| 1 | `let x = 1; print(x);` | "Silently corrupts parser state" | Runs correctly; lint warns `unnecessary_semicolon` | Doc claim **false** |
| 2 | `let m = { "a": 1 }` | Silently parses as block | **Loud parse error** E002 with source context | Already hardened |
| 3 | `"hello ${name}"` | — | Prints literal `${name}`; **no warning**; `lint --strict` passes clean | **Silent failure** |
| 4 | `s.len()` | "WRONG — dot reads properties only" | Returns 3 (UFCS); `5.double()` also works for user fns | Doc claim **false** |
| 5 | `if 0 { }` | Silent truthy surprise | Runtime `[WARN]` non-boolean condition; **error** in strict mode; lint warns | Already hardened |
| 6 | `for c in "abc"` | "Silently does nothing" | Skips, but with runtime `[WARN]` suggesting `chars()`; lint warns | Hardened (warn, not error) |
| 7 | `let m = { 5 }` / `let e = {}` | — | Binds 5 / Unit silently, no warning | **Silent failure** (narrow) |
| 8 | `s.length()` (unknown method) | — | Loud E007, but no "did you mean `len(s)`"; lint passes clean | Diagnostic gap |
| 9 | `prnt("hi")` (typo) | — | E006 with `help: Did you mean 'print'?` | Good |
| 10 | `divide(10, 0)` with `requires b != 0` | — | E004 fires, but no line number, no source frame, no argument values | Diagnostic gap |
| 11 | `arr[10]`, `m["missing"]` | — | Both print `none`, no warning; typechecker infers element type, not Option | **Type-reality mismatch** |
| 12 | `"count: " + 5` | — | `[WARN]` implicit conversion with `str()` hint; concatenates | Hardened |
| 13 | `range(0, 3)` | — | Loud E006 at runtime; lint warns `python_style_range` | Hardened |
| 14 | `fib(30)` benchmark | — | ntnt 2.9s vs CPython 0.083s (~35x) | Baseline for DD-061 |

## References

- `whitepaper.md` — vision document assessed above
- DD-047 (module imports, shipped), DD-048 (DX simplification, shipped), DD-049 (composition layer, shipped), DD-050 (function generics, backlog), DD-054 (parser gaps; semicolon/map regressions locked), DD-058 (stdlib gap analysis), DD-060 (AI-native DX strategy — complementary product framing; this DD is the language-level evidence), DD-061 (interpreter performance), DD-062 (extension libraries)
- Key code: `src/ial/resolve.rs` (term rewriting), `src/typechecker.rs:1933` (index TODO), `main.rs:3821` (`${}` lint rule), `src/contracts.rs`, `build.rs:544` (doc enforcement)
