# DD-063: NTNT Language Assessment — Strengths, Gaps, and Priorities

## Status: active — implementation tracking for v0.4.11 (v3; assessment finalized in v2)

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

## Implementation plan (v0.4.11)

This is the tracking section: check items off as PRs land. v0.4.10 was tagged at `6374d95` (the commit carrying this assessment); everything below targets v0.4.11 (main bumped at `35c4c5f`). Each PR below was scoped at file level against v0.4.10 source — the full per-PR plans (step-by-step approach, exact functions, test lists, risk registers) live in [plans/dd-063-scoping-notes.md](../plans/dd-063-scoping-notes.md); this section is the condensed tracking view.

**Why not one PR for Recs 1–4:** scoped totals are ~1,300 implementation + ~1,700 test LOC spanning the parser, typechecker, interpreter, IAL engine, and CLI, with one item (parser error recovery) carrying meaningfully higher regression risk than the rest. Split into four PRs with one risk profile each, plus a fifth gated on maintainer decisions. PR-1 and PR-2 are independent and can proceed in parallel; PR-3 and PR-4 should follow PR-2 (its doc-claim tests act as a behavioral safety net for parser/diagnostic changes).

Scoping also surfaced **two additional doc drifts** beyond the v2 findings: CLAUDE.md rule 8 (`mut`) is only enforced for indexed/deep mutation — plain rebinding without `mut` succeeds; and rule 10 (module-level `let` can't use `map {}`) is stale — it works now. Both are folded into PR-2.

### Progress at a glance

- [ ] **PR-1** — `${}` interpolation detection (Rec 1) — S
- [ ] **PR-2** — Doc-claim regression tests, CLAUDE.md truth, UFCS embrace (Rec 2) — M
- [ ] **PR-3** — Diagnostics I: parser error recovery + contract violation context (Rec 4a) — M
- [ ] **PR-4** — Diagnostics II: method bridge hints, unknown-method lint, IAL suggestions, `ntnt intent lint` (Rec 4b) — M
- [ ] **PR-5** — Index out-of-bounds loudness (Rec 3) — M — **blocked on decisions**
- [ ] Recs 5–10 — unscheduled, see end of section

### PR-1 — `${}` interpolation detection (Rec 1) — S, ~150 impl + ~280 tests

Scope-resolved detection in two layers: lint warns (error under `--strict`) when a string literal contains `${ident}` and `ident` resolves to a variable in scope — this keeps legitimate shell/JS content (`${HOME}`, `${1}`) clean while catching the actual bug with near-certainty; plus a deduped runtime `[WARN]` at string evaluation following the `type_warn_dedup` pattern. Emitted from the typechecker (which runs in all lint modes and has scopes + line attribution), structurally excluding template strings.

- [ ] Shared detector `find_js_interpolation_idents()` in `src/typechecker.rs` + unit tests
- [ ] `DiagnosticKind::JsStyleInterpolation` emitted from `infer_expression` for `Expression::String` and `InterpolatedString` literal parts, with double-visit dedup set (required: expression statements are inferred twice)
- [ ] Strict-mode promotion in `check_program_with_lint_mode`; distinct rule name `javascript_style_interpolation` in lint JSON (`src/main.rs` ~3350)
- [ ] Runtime hook in `eval_expression` (`${` pre-check, scope-checked, deduped, silent in forgiving mode; stays WARN even in strict — hard failure belongs to `lint --strict`)
- [ ] Fix three stale lint messages claiming NTNT interpolation is `"{variable}"` instead of `#{variable}` (`src/main.rs` ~3454, ~3826-3830)
- [ ] `tests/js_interpolation_tests.rs` (~12 integration cases) + `docs/AI_AGENT_GUIDE.md` interpolation section
- [ ] Grep `examples/` and test fixtures for `${` before merging (strict promotion may surface existing hits)

Defaults applied (object before implementation if wrong): strict promotion **yes**; raw-string `r"..."` false-positive class accepted for v1 (concat/template-string workarounds documented); no out-of-scope-ident detection (protects precision; typo'd `${nmae}` stays uncaught in v1).

### PR-2 — Doc-claim regression tests, CLAUDE.md truth, UFCS embrace (Rec 2) — M, ~70 impl + ~500 tests + doc edits

One regression test per behavioral claim in CLAUDE.md's 16 Critical Syntax Rules, locking *actual* binary behavior; rewrite the four wrong/stale rules; document UFCS as first-class sugar with its real gaps (map-stored closures not dot-callable; parens disambiguate call vs key lookup); make IAL_REFERENCE honest about unimplemented primitives.

- [ ] `tests/doc_claims_tests.rs` (~26 tests; assert on error codes + short stable fragments, never full message lines)
- [ ] CLAUDE.md corrections: rule 3 (UFCS works — reframe as "free functions are canonical, dot-call is sugar"), rule 5 (semicolons are lint warnings, not parser corruption), rule 8 (`mut` enforced only for indexed/deep mutation — document actual semantics), rule 10 (module-level `map {}` works — replace in place to keep rule numbering stable)
- [ ] `docs/AI_AGENT_GUIDE.md` + `syntax.toml` UFCS documentation; review regenerated `.github/copilot-instructions.md` diff
- [ ] Fix `collect_from_expr` unused-import lint bug exposed by UFCS embrace (dot-call method names are dropped, so imports used only via UFCS get flagged unused)
- [ ] `ial.toml` status field for `sql`/`invariant_check` + Status column in `generate_ial_markdown` (and update the fixed key arrays in `src/main.rs`, or new rows silently render nothing)
- [ ] Regenerate `docs/IAL_REFERENCE.md` / `STDLIB_REFERENCE.md` via `ntnt docs --generate`

Maintainer decisions: **(a)** rule 8 — document current `mut` semantics (this plan) or fix enforcement for plain rebinding (breaking; would need its own DD)? **(b)** IAL `Sql` primitive — mark `not_implemented` in docs (this plan) or remove from `ial.toml` entirely until built?

### PR-3 — Diagnostics I: parser error recovery + contract violation context (Rec 4a) — M, ~420 impl + ~380 tests

New opt-in `Parser::parse_with_recovery() -> (Program, Vec<Error>)` with statement-level panic-mode synchronization (cap 5 errors/file, same-line dedup + token-gap cascade suppression); existing `parse()` keeps its exact signature and single-error behavior so all 18 call sites (run path, module imports, REPL, intent) are untouched — only `lint_project`/`validate_project` consume the multi-error path. E004 gains what every other error class has: per-clause line capture (`ast::ContractCondition { expression, line }`), a struct `ContractViolation { message, line, call_line, values }`, and parameter values read from the function environment *before* restore (`where: b = 0`), rendered with a real source frame. Display text stays byte-identical for existing consumers.

- [ ] `parse_with_recovery` + private `recover` flag + `synchronize()` with guaranteed token advance
- [ ] `lint_project`/`validate_project` report all recovered errors; semantic checks skipped on parse-error files
- [ ] `ast::ContractCondition` + line capture in `parse_contract()`
- [ ] `ContractViolation` struct variant; identifier-walking value capture (env lookups only — never expression re-evaluation, which could fire side effects during error construction); capture **before** env restore
- [ ] `rich_display` source frame for E004; `error.rs` wiring
- [ ] `tests/diagnostics_tests.rs`: multi-error lint, token-soup termination, run-path regression (run still aborts on first error), multi-param `where:` values
- [ ] Lint all `examples/*.tnt` before/after as a recovery-regression sweep

Defaults applied: run path stays first-error-only (lint is the multi-error surface); MAX_PARSE_ERRORS = 5; struct invariants get values but not clause lines in v1. Maintainer decision: `ntnt parse --json` contract-clause shape changes to `{expression, line}` objects — acceptable, or add a compatibility serializer?

### PR-4 — Diagnostics II: bridge hints, unknown-method lint, IAL suggestions, `ntnt intent lint` (Rec 4b) — M, ~650 impl + ~500 tests

Four self-contained commits in one PR: (1) method-call misses get Levenshtein suggestions over `Environment::keys()` plus a small alias table (`length→len`, …) and a UFCS bridge hint ("methods resolve to free functions: try `len(s)`"); (2) the typechecker warns on method names found in neither the function registry, builtin sigs, nor scope (gated to non-`Any` receivers; warning in default lint mode, error in strict); (3) `ResolveError` gains `kind` + `suggestions` computed over normalized vocabulary patterns, flowing to intent-check output automatically; (4) `ntnt intent lint` statically resolves every scenario assertion against glossary+standard vocabulary without executing primitives — reports unresolved terms (with suggestions), cycles, orphan glossary entries; exit 1 on unresolved/cycles, `--json` for CI.

- [ ] Method-miss suggestions + alias table + `hint` field on `IntentError::UndefinedFunction` (`src/interpreter.rs` ~7044)
- [ ] Typechecker unknown-method diagnostic + register the five runtime-global option/result helpers (`is_some`/`is_none`/`is_ok`/`is_err`/`unwrap_or`) in `builtin_sigs` (note: introduces arity checking on previously-unchecked calls — release-note item)
- [ ] IAL `ResolveError.suggestions` via vocabulary-key Levenshtein with `normalize_term_for_cycle` normalization
- [ ] `intent::lint_intent_file()` + clap subcommand + `--json`; orphan entries stay warnings (never exit-1 — legacy direct-pattern fallback makes orphan detection imperfect)
- [ ] Visual check of Intent Studio error rendering (trace.error string changes)
- [ ] Lint-run real .tnt projects to confirm unknown-method noise level before merging

Defaults applied: unknown-method fires in default lint mode (that's the DD-063 complaint — `lint` passed clean on `s.length()`); `intent lint` accepts a `.tnt` path and auto-locates the paired `.intent`. Maintainer decision: approve/trim the alias table (`length→len, size→len, count→len, map→transform, to_string→str, to_str→str, append→push, upper→to_upper, lower→to_lower`).

### PR-5 — Index out-of-bounds loudness (Rec 3) — M, ~120-160 impl + ~250 tests — **blocked on decisions**

Scoping analysis recommends **staging**: ship runtime loudness for out-of-bounds **array/string read** access now (strict → E010 error; warn → deduped `[WARN]` + `None`; forgiving → silent), exactly mirroring the existing TypeMode gates in the same `Expression::Index` arm; **map missing-key stays silent-`None`** (documented intentional DX, 127 of 150 index usages in examples/ are map-key access, `has_key()` exists). `arr[i] ?? default` / `arr[i]?` / `otherwise` suppress both warn and strict error. Typechecker honesty (inferring `Option<T>`) is **deferred with cause**: the runtime's index result is a nullable union (`T | None`), not enum `Option<T>` — annotating `Option<T>` would be a new lie that crashes checker-blessed code on `unwrap`/`match`; unifying that representation needs its own DD first.

Decisions required before this PR starts:
- [ ] Confirm map missing-key (and `map.field`) stays silent in all modes
- [ ] Confirm `??` / `?` / `otherwise` suppress the strict-mode error too (recommended: yes — `??` is the documented universal safety net)
- [ ] Confirm `String[i]` OOB included alongside arrays (recommended: yes)
- [ ] Confirm a strict-mode runtime behavior change is acceptable within 0.4.x (or needs 0.5 / a breaking-change release note)
- [ ] Acknowledge the deferred-option-(a) constraint: any future `Option<T>` inference for index expressions requires the runtime-representation DD first

Implementation (after decisions):
- [ ] TypeMode-gated OOB handling in `Expression::Index` read path + suppression for guarded parents
- [ ] Resolve the read/write asymmetry note (index *assignment* OOB already errors unconditionally)
- [ ] Tests across all three modes + guarded-access suppression; update CLAUDE.md rule 15 area and AI_AGENT_GUIDE error-handling docs

### Recs 5–10 — assessed, not yet scheduled

- [ ] **Rec 5** — IAL stub completion: implement invariant execution (expansion already works); decide `Sql` primitive fate (PR-2 makes docs honest in the interim)
- [ ] **Rec 6** — lint for silent block-binding (`let x = { 5 }` / `let e = {}`)
- [ ] **Rec 7** — strict type mode in verification contexts (`intent check`, `ntnt test`)
- [ ] **Rec 8** — whitepaper restructure: shipped vs aspirational
- [ ] **Rec 9** — static contract lint for literal-argument violations (stretch)
- [ ] **Rec 10** — execute existing DDs: DD-061 phase 1 (fib(30) ≈ 35x CPython baseline recorded above), DD-058 priority-1 stdlib gaps, DD-062 extension model

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
