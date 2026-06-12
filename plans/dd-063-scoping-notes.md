# DD-063 Scoping Notes (v0.4.11 implementation)

Full file-level scoping for DD-063 recommendations 1-4, produced 2026-06-12 against v0.4.10 (`6374d95`). The tracking checklist lives in [design-docs/dd-063-language-assessment.md](../design-docs/dd-063-language-assessment.md); this file holds the complete per-PR plans (approach, files, tests, risks, open decisions) so implementers do not need to re-do discovery. Line references were verified against source at scoping time but may drift as PRs land.

## PR-1 — `${}` interpolation detection (Rec 1)

**Size:** S &nbsp;|&nbsp; **Estimated LOC:** ~150 impl (typechecker ~80, interpreter ~35, main.rs ~15, docs ~20) + ~280 tests (12 integration cases with per-file helpers + detector unit tests)

**Scoping verdict:** Implement scope-resolved detection (option b) in the typechecker so it fires through `ntnt lint` in all modes (warning by default, promoted to error in --strict), plus a deduped scope-checked runtime [WARN] in the interpreter following the type_warn_dedup pattern. Reject option (a) (any `${ident}`) because legitimate shell/JS content in regular strings would create false positives that erode lint trust, and reject option (c) (content heuristics) as fragile. Reject lexer/parse-time detection: the lexer has no scope info (forcing option a's imprecision), no warning/dedup channel, and would fire for never-executed code; lint already covers static detection. The runtime warning IS worth keeping despite lint coverage because agents demonstrably skip the lint step and the warning lands adjacent to the visibly-wrong output, closing the feedback loop.

### Summary

Add a scope-aware `javascript_style_interpolation` check in two layers. (1) Lint: a new `DiagnosticKind::JsStyleInterpolation` emitted from the typechecker's `infer_expression` for `Expression::String` and the `StringPart::Literal` segments of `Expression::InterpolatedString` — it warns only when the head identifier inside `${...}` resolves via `self.lookup()` to a variable in scope, which makes shell (`${HOME}`, `${1}`) and most embedded-JS content pass clean while catching the actual bug (`print("hello ${name}")` with `name` in scope) with near-certainty. The typechecker already runs in `lint_project` in ALL lint modes, has scopes, and has line attribution, so this is the natural home; the AST-based approach also structurally excludes template strings (`Expression::TemplateString` is a different node) where `{{expr}}` and embedded `${...}` JS are legitimate. (2) Runtime: a `type_warn_dedup`-deduped `[WARN]` in `eval_expression` for string literals containing `${`, checked against the live runtime environment, silent in `TypeMode::Forgiving`. Also fix two stale lint messages that claim NTNT interpolation is `"{variable}"` instead of `#{variable}`. The existing backtick-based raw-source rule stays (it catches pasted JS template literals) with its message corrected.

### Approach

PHASE 1 — shared detector (src/typechecker.rs, near expr_search_hint at ~line 289):
Add `pub fn find_js_interpolation_idents(s: &str) -> Vec<(String, String)>` returning (head_ident, full_snippet) pairs. Hand-rolled scanner, no regex dep needed: for each occurrence of "${", the next char must be [A-Za-z_]; consume [A-Za-z0-9_]* as head ident; then require a closing '}' before the next newline (this admits `${user.name}` and `${VAR:-x}` with head idents `user`/`VAR`); record (head_ident, the full `${...}` text). Skip `${1}`, `${}`, `${{`. Unit-test this directly in the existing `mod tests` (typechecker.rs:4290).

PHASE 2 — lint via typechecker:
1. Add `JsStyleInterpolation` variant to `DiagnosticKind` (typechecker.rs:24-33).
2. Add field `js_interp_reported: HashSet<String>` to `TypeContext` (struct ~line 71, init in `new()` ~line 385). This is REQUIRED: `infer_expression` is provably called twice on expression statements (`check_statement` at line 1702 then `infer_statement_terminal_type` at line 1703 → line 754), so without dedup every finding doubles.
3. In `infer_expression`, change the `Expression::String(s) => Type::String` arm (line 1714) to a block: if `s.contains("${")`, run the detector; for each (ident, snippet) where `self.lookup(ident).is_some()` and `js_interp_reported.insert(format!("{}#{}", snippet, ident))` is true, compute `line = self.find_line_near(&snippet)` and call `self.emit_with_kind(Severity::Warning, DiagnosticKind::JsStyleInterpolation, format!("String literal contains \"{snippet}\" — NTNT interpolation is \"#{{{ident}}}\"; the \"${{...}}\" text will be output literally"), line, Some(format!("Use \"#{{{ident}}}\". If literal ${{...}} is intended (shell/JS output), build it via concatenation (\"$\" + \"{{...}}\") or a \"\"\"template string\"\"\".")))`. Return Type::String.
4. In the `Expression::InterpolatedString(parts)` arm (line 1987), add the same scan over `StringPart::Literal(s)` parts, OUTSIDE the existing `if self.strict_lint` gate (this catches `"hi #{a} and ${b}"`). The existing strict-only complex-type check is untouched.
5. In `check_program_with_lint_mode` (lines 198-212), add `DiagnosticKind::JsStyleInterpolation` to the warning→error promotion list for `LintMode::Strict` so `ntnt lint --strict` exits 1. NOTE: this does NOT make `ntnt run` fail under strict type mode — `strict_check_with_file` (typechecker.rs:134-155) calls `check_program_with_options` directly and filters `Severity::Error` only, so the diagnostic remains a Warning on the run path. Verify this with a test.

PHASE 3 — lint output plumbing (src/main.rs):
In the `lint_project` diagnostics loop (lines 3350-3362), replace the hardcoded `"rule": "type_check"` with a kind-based mapping: `let rule = match diag.kind { ntnt::typechecker::DiagnosticKind::JsStyleInterpolation => "javascript_style_interpolation", _ => "type_check" };` so agents see a distinct rule name in the JSON. The `hint` field already flows through (line 3360).
Stale-message fixes: line 3826 message and 3828-3830 fix text of `javascript_template_string` say NTNT uses `"{variable}"` — change to `#{variable}` / `Replace ${var} with #{var}`. Line 3454 `syntax_hints.string_interpolation` similarly says `"{variable}"` — change to `#{variable}`. Leave the backtick-gated raw-source rule otherwise unchanged (it serves the pasted-JS-template-literal case and already skips triple-quoted regions via the `in_triple_quote` tracker at lines 3799-3819).

PHASE 4 — runtime warning (src/interpreter.rs):
1. New private method on Interpreter: `fn warn_js_style_interpolation(&self, s: &str)`: early-return if `get_type_mode() == TypeMode::Forgiving`; call `crate::typechecker::find_js_interpolation_idents(s)`; for each (ident, snippet) where `self.environment.borrow().get(&ident).is_some()`, call `type_warn_dedup(&format!("js_interp:{}:{}", self.current_line, snippet), &format!("String literal contains \"{snippet}\" — NTNT interpolation is \"#{{{ident}}}\"; printed literally (line {})", self.current_line))`. It stays a WARN even in TypeMode::Strict — a literal string is type-valid, and strict-mode users get the hard failure from lint --strict instead; erroring at runtime could break working generators on a name collision.
2. Hook 1: `Expression::String(s)` arm of `eval_expression` (line 5660): `if s.contains("${") { self.warn_js_style_interpolation(s); }` before returning the value. The `contains` pre-check (memchr-backed) keeps the hot path nearly free; the detector/env-lookup only runs on the rare `${` hit.
3. Hook 2: `Expression::InterpolatedString` arm (lines 7197-7210): same pre-check + call for each `StringPart::Literal(s)`.
4. Dedup semantics: keyed by line+snippet in the thread-local WARNED_LOCATIONS set (config.rs:141-167), cleared per HTTP request like existing type warnings — one warn per site per request/run, matching established behavior.

PHASE 5 — tests and docs:
New integration test file `tests/js_interpolation_tests.rs` copying the `unique_test_file`/binary-invocation helpers from tests/language_features_tests.rs (helpers are duplicated per-file by convention), covering the lint JSON output (`ntnt lint` emits machine-readable JSON to stdout) and runtime stderr. Update docs/AI_AGENT_GUIDE.md interpolation section: document the rule name, the runtime WARN, and the two suppression escapes (concatenation, template strings). No stdlib functions added, so `ntnt docs --generate` is not required, but run `cargo fmt`, `cargo clippy`, and full `cargo test` per the pre-push checklist. Optionally mark Rec 1 as implemented in design-docs/dd-063-language-assessment.md.

### Files

- `/home/larimonious/repos/ntnt/src/typechecker.rs` — Add DiagnosticKind::JsStyleInterpolation (lines 24-33); add pub fn find_js_interpolation_idents() detector near expr_search_hint (~line 289); add js_interp_reported: HashSet<String> to TypeContext (struct ~line 71, new() ~line 385); emit scope-checked warning in infer_expression's Expression::String arm (line 1714) and InterpolatedString arm's Literal parts (line 1987, outside strict_lint gate); add JsStyleInterpolation to strict promotion list in check_program_with_lint_mode (lines 198-212); unit tests for detector in mod tests (line 4290).
- `/home/larimonious/repos/ntnt/src/interpreter.rs` — Add warn_js_style_interpolation() method using crate::typechecker::find_js_interpolation_idents + self.environment lookup + type_warn_dedup keyed on current_line+snippet, gated off in TypeMode::Forgiving; hook it behind s.contains("${") pre-checks in eval_expression's Expression::String arm (line 5660) and InterpolatedString Literal parts (lines 7197-7210).
- `/home/larimonious/repos/ntnt/src/main.rs` — lint_project diagnostics loop (lines 3350-3362): map DiagnosticKind::JsStyleInterpolation to rule name "javascript_style_interpolation" instead of "type_check". Fix stale interpolation syntax in javascript_template_string message/fix (lines 3826-3830) and syntax_hints.string_interpolation (line 3454) from "{variable}" to #{variable}.
- `/home/larimonious/repos/ntnt/tests/js_interpolation_tests.rs` — New integration test file (helpers copied per existing convention from tests/language_features_tests.rs) covering lint JSON output, strict promotion, runtime WARN, dedup, forgiving mode, scope-negative cases, and template-string non-firing.
- `/home/larimonious/repos/ntnt/docs/AI_AGENT_GUIDE.md` — In the string-interpolation section, document that ${...} referencing an in-scope variable triggers lint rule javascript_style_interpolation and a runtime [WARN], and document suppression for intentional shell/JS content: string concatenation ("$" + "{...}") or triple-quoted template strings.
- `/home/larimonious/repos/ntnt/design-docs/dd-063-language-assessment.md` — Optional: annotate Recommendation 1 (line 109) as implemented with the chosen scope-resolved design.

### Tests

- Lint positive: file with `let name = "world"` + `print("hello ${name}")` → `ntnt lint` JSON contains rule javascript_style_interpolation, severity warning, correct line number, hint mentioning #{name}; exit code 0 (warnings don't fail default lint).
- Lint strict promotion: same file with `ntnt lint --strict` → severity error in JSON and exit code 1 (this is the DD-063 acceptance criterion: --strict no longer passes clean).
- Lint scope-negative: `write_file("d.sh", "echo ${HOME} ${1}")` with no HOME variable in scope → no javascript_style_interpolation issue (shell-script false-positive guard).
- Lint template-string exclusion: `let b = 1` + `let s = """const x = `${b}`;"""` → no javascript_style_interpolation issue (TemplateString node never scanned; also verifies the {{expr}} path is untouched).
- Lint mixed interpolation: `"hi #{a} and ${b}"` with both a and b in scope → exactly one finding, for b only.
- Lint dedup: a .tnt file where the same string literal is an expression statement (double-visited via check_statement + infer_statement_terminal_type) → exactly one finding, not two.
- Runtime warn: `ntnt run` on the name-in-scope file → stdout contains literal `hello ${name}`, stderr contains `[WARN]` and `#{name}`, exit 0.
- Runtime dedup: same print inside `for i in 0..3` loop → stderr contains exactly one `[WARN]` line for that site (assert via match count).
- Runtime forgiving: NTNT_TYPE_MODE=forgiving → stdout unchanged, stderr contains no [WARN].
- Runtime scope-negative: `print("path is ${UNDEFINED_THING}")` → no [WARN].
- Runtime strict-mode non-fatal: NTNT_TYPE_MODE=strict (and separately NTNT_LINT_MODE=strict ntnt run) → program still executes with exit 0 and a [WARN], confirming strict_check_with_file's Error-only filter doesn't block the run path.
- Unit tests for find_js_interpolation_idents: extracts (name, ${name}); extracts head ident from ${user.name} and ${VAR:-default}; rejects ${1}, ${}, ${{, and `${` with no closing } before newline; handles multiple occurrences in one string.

### Risks

- Raw strings: parser collapses r"..." to plain Expression::String (parser.rs:1954-1961), so r"echo ${name}" with an in-scope `name` variable WILL warn at both lint and runtime. Scope-checking makes this rare (requires a genuine name collision), and concat/template-string suppression exists, but it is a real false-positive class. The principled fix (an Expression::RawString variant or a raw flag) touches parser, typechecker, interpreter, and every site that pattern-matches Expression::String — deferred as a maintainer decision.
- Recall gap inherent to option (b): typo'd interpolation like "hello ${nmae}" (ident not in scope) is NOT caught — same blind spot that makes shell vars safe. The backtick raw-source rule and docs partially cover; full coverage would need the rejected option (a).
- Typechecker line attribution uses find_line_near's forward-cursor substring search (typechecker.rs:479-494); if the identical ${snippet} appears on multiple lines, later occurrences may get the wrong line and the snippet-keyed dedup reports only the first. Cosmetic, consistent with existing diagnostics in this file.
- Hot-path cost: one s.contains("${") per string-literal evaluation in the interpreter. memchr-backed and branch-predicted false in normal code, but worth a quick before/after run of a string-heavy example to confirm no measurable regression.
- infer_expression double-visit (check_statement line 1702 + infer_statement_terminal_type line 1703) WILL produce duplicate diagnostics unless the js_interp_reported dedup set is implemented — this is a correctness requirement, not an optimization.
- Strict-mode promotion makes ntnt lint --strict exit 1 on code that previously passed; that is the intended DD-063 outcome but may surface in existing example files or CI fixtures — grep examples/ and tests/fixtures for `${` before merging.
- Match-pattern string literals (Pattern::Literal) are not routed through infer_expression, so ${} in match patterns is unscanned. Extremely rare; acceptable gap.
- In long-running servers the runtime dedup set is cleared per request (clear_type_warnings), so one [WARN] per request per site — identical to existing type-warning behavior, but means noisy logs for a high-traffic endpoint with the bug (arguably desirable).

### Open decisions

- Confirm strict-mode promotion: should javascript_style_interpolation become a hard error under `ntnt lint --strict` (recommended yes — it is high-confidence once scope-resolved, and DD-063 frames --strict passing clean as the failure)?
- Raw-string suppression: accept the r"..." false-positive class for v1 with documented concat/template-string workarounds (recommended), or invest in an Expression::RawString AST variant for principled suppression at both lint and runtime (~30 extra LOC plus an audit of every Expression::String match site, including route-pattern handling)?
- Should LintMode::Strict additionally flag out-of-scope ${ident} (option (a) as a strict-only fallback) to catch typo'd interpolations, at the cost of flagging shell/JS strings under --strict? Recommended no for v1 to protect precision.
- Confirm the runtime warning stays a WARN (never an error) even in NTNT_TYPE_MODE=strict (recommended — a string literal is type-valid; hard failure belongs to lint --strict).

---

## PR-2 — Doc-claim regression tests + UFCS embrace (Rec 2)

**Size:** M &nbsp;|&nbsp; **Estimated LOC:** ~70 impl (src/main.rs generator+lint, toml entries) + ~500 tests (doc_claims_tests.rs) + ~130 doc-line edits (CLAUDE.md, AI_AGENT_GUIDE.md) + regenerated markdown

**Scoping verdict:** Implement DD-063 Rec 2 in three commits: (1) new tests/doc_claims_tests.rs locking actual behavior for all 16 CLAUDE.md rules; (2) doc corrections (CLAUDE.md rules 3/5/8/10/11, AI_AGENT_GUIDE.md, syntax.toml + generator) embracing UFCS; (3) IAL honesty fix (status field in ial.toml + generator) and the one small lint fix UFCS requires (count dot-call method names as import usage). Lock plain-reassignment-without-mut as a documented behavior test but escalate whether it should instead be enforced.

### Summary

Empirical testing of all 16 CLAUDE.md "Critical Syntax Rules" against the v0.4.10 binary confirmed the two known drifts (semicolons run fine with only a lint warning; UFCS dot-call fully works for builtins, imports, and user functions) and found two more: rule 8 (`mut`) is only enforced for indexed/deep mutation, not plain rebinding, and rule 10 (module-level `let` with `map {}`) is stale — it works now. The plan adds tests/doc_claims_tests.rs (~26 tests, one per behavioral claim, following language_features_tests.rs conventions), rewrites the four wrong/stale CLAUDE.md rules, updates AI_AGENT_GUIDE.md and syntax.toml to document UFCS as first-class sugar (with its real gaps: map-stored closures are not dot-callable, parens disambiguate function call from key lookup), fixes the one lint bug UFCS-embrace exposes (collect_from_expr drops dot-call method names, so imports used only via UFCS are flagged unused), and makes IAL_REFERENCE.md honest by adding a status field to ial.toml's sql/invariant_check primitives plus a Status column in generate_ial_markdown.

### Approach

PER-RULE ACCURACY FINDINGS (verified against target/dev-release/ntnt v0.4.10, commit 6374d95):
1 ACCURATE (nuance: `let user = { "name": ... }` is a LOUD parse error E002 "Expected expression, but found ':'", not a silent block; `let x = { 1 + 2 }` is a block expression yielding 3).
2 ACCURATE (nuance: `"${name}"` is silently left as literal text — no lint or runtime warning; optional lint gap).
3 INACCURATE — UFCS works: `"abc".len()`→3, `"  hi  ".trim()` works with import, `5.double()`→10 for user fns, `m.keys()` works WITHOUT import (stdlib fns are globally registered), `xs.sort()` works. Dispatch (src/interpreter.rs:6980-7050 Expression::MethodCall): module-struct fields first, else receiver inserted as args[0] and `method` looked up in environment; else E007 UndefinedFunction with suggestion:None/line:0. Gaps: (a) closure stored in a map value is NOT dot-callable — `m.f(10)` → E007 "Undefined function: f"; (b) parens disambiguate: `m.keys` (FieldAccess, interpreter.rs:6586) reads map key (→42 if key "keys" exists, silent None if missing), `m.keys()` ALWAYS calls keys(m) ignoring the key; (c) lint bug: src/main.rs:4948-4955 collect_from_expr MethodCall arm deliberately drops the method name, so `import { trim }` used only as `s.trim()` is flagged unused_import.
4 ACCURATE — `import { get, listen } from "std/http/server"` fails at runtime: E005 "'get' is not exported from 'std/http/server'" with available-exports list; lint does NOT catch it.
5 INACCURATE — semicolons parse and run fine (exit 0, correct output); lint emits warning rule unnecessary_semicolon (src/main.rs:3901) with autofix; lint exit code stays 0 for warnings. "Silently corrupts parser state" is false.
6 ACCURATE and enforced twice: lint reports severity:error "otherwise block does not diverge — it must end with return, break, or continue" (exit 1); runtime raises E005 "otherwise block must diverge (use return, break, or continue)" only when the error path is actually taken (happy path runs fine).
7 ACCURATE — `range(3)` → E006 "Undefined variable: range" (note: variable, not function), exit 1; `0..3` works.
8 PARTIALLY INACCURATE — `mut` is enforced ONLY for indexed/deep mutation: `arr[0] = 99` without mut → E005 "Cannot mutate 'arr': variable is not declared with 'let mut'" (sole is_mutable check at interpreter.rs:6828). Plain rebinding `let x = 0` then `x = x + 1` succeeds (even NTNT_TYPE_MODE=strict, lint clean), including inside for-loops.
9 ACCURATE — `|x| x * 2` → parse error E002 at the '|'.
10 STALE/INACCURATE — module-level `let config = map { "k": "v" }` runs fine and prints `{ k: v }`.
11 ACCURATE with nuance — for..in on a string yields zero iterations but is NOT silent: runtime "[WARN] for..in on String — skipping (not a collection). Use chars()..." plus lint warning (rule type_check) with chars() hint.
12 ACCURATE — `"""Hello {{name}}"""` interpolates; single `{name}` left literal.
13 ACCURATE — contracts after return type before body parse and run.
14 ACCURATE — 0 truthy; ""/[]/map{} falsy; each non-boolean condition emits "[WARN] Non-boolean condition in if/while. Got <Type>."
15 ACCURATE — `m["missing"]` → none, has_key → false, `m.missing` → none (silent for maps).
16 ACCURATE — `for k in m` yields keys; entries(m) yields {key,value} maps. CAUTION for tests: printing an entry map has nondeterministic field order ("{ key: a, value: 1 }" vs "{ value: 2, key: b }") — tests must read e.key/e.value, and key iteration order appears sorted ([a, keys]) but assert via contains, not exact order.

IMPLEMENTATION STEPS:

STEP 1 — tests/doc_claims_tests.rs (new). Copy the helper trio from tests/language_features_tests.rs (unique_test_file, write via fs::File + writeln!, binary resolution preferring target/debug then target/release, current_dir(env!("CARGO_MANIFEST_DIR"))). Add a second helper run_ntnt_lint(code) that invokes `ntnt lint <file>` the same way (lint exits 0 with warnings, 1 with errors — verified). Write the ~26 tests listed in the tests field. Assert on stable substrings: error codes (E002/E005/E006/E007), lint rule ids (unnecessary_semicolon, unused_import, type_check), and exact stdout lines for value-producing programs. Header comment: "Doc-claim regression tests (DD-063 Rec 2): each test locks a behavioral claim made in CLAUDE.md's Critical Syntax Rules. If one fails, the language changed — update CLAUDE.md and docs/AI_AGENT_GUIDE.md in the same PR."

STEP 2 — CLAUDE.md corrections (keep numbered ### format):
- Rule 3 retitle "Dot-call is UFCS sugar — `x.f(a)` calls `f(x, a)`". Body: canonical style is free functions (`len(s)`); dot-call is equivalent and supported for builtins, imports, and user functions; dot WITHOUT parens is property/map-key read; parens always mean function call even when a map key has the same name (`m.keys` reads the "keys" key, `m.keys()` calls keys(m)); caveat: closures stored in map values are not dot-callable — bind first (`let f = m.f` then `f(x)`).
- Rule 5: "No semicolons — use newlines. Semicolons parse fine but are unnecessary; `ntnt lint` warns (unnecessary_semicolon). Omit them."
- Rule 8: "`let mut` is required for indexed mutation (`arr[0] = x`, `m[\"k\"] = v`); plain rebinding currently succeeds without `mut` — still declare `mut` for anything you reassign."
- Rule 10: delete (stale) and replace its slot with the map-closure UFCS caveat (avoids renumbering rules 11-16): "Closures stored in map values can't be called with dot — `m.f(x)` fails; bind first."
- Rule 11: append "(runtime warning is emitted; lint also flags it)".

STEP 3 — docs/AI_AGENT_GUIDE.md: (a) line ~85 bullet "Free functions, not methods: len(s) not s.len()" → "Free functions are canonical; dot-call sugar `s.len()` ≡ `len(s)` (UFCS) also works"; (b) section ~395-420: replace the "WRONG - method-style calls on stdlib functions" block with a "Dot-call sugar (UFCS)" subsection documenting the dispatch rule, the parens-disambiguation on maps, and the map-closure caveat; (c) troubleshooting table ~line 3252: remove/replace the row "`unexpected token '.'` | Method-style call on stdlib function" — that parse error does not exist; replace with a row for E007 "Undefined function: f" | dot-calling a closure stored in a map | bind to a local first.

STEP 4 — docs/syntax.toml + generator: add after [operators.member] a new [operators.method_call] entry (symbols [".()"], description "Method-call sugar (UFCS): x.f(a) resolves to f(x, a) for any builtin, imported, or user function; parens distinguish call from property read", example "s.len(), value.double(), m.keys()"). REQUIRED companion change: add "method_call" to the fixed op_categories array in generate_syntax_markdown (src/main.rs ~6220-6232) or the entry will not render.

STEP 5 — lint fix for UFCS (the one code change embrace requires): in src/main.rs collect_from_expr, Expression::MethodCall arm (~4948), change pattern to bind `method` and add `names.insert(method.clone())` so imports used via dot-call are not flagged unused. Locked by test doc_rule03_lint_no_unused_import_for_ufcs.

STEP 6 — IAL honesty: in docs/ial.toml add `status = "not_implemented"` to [primitives.sql] (executor src/ial/execute.rs:154-160 unconditionally fails with "SQL execution not yet implemented") and `status = "partial"` with a note to [primitives.invariant_check] (execute.rs:1108-1122 is a resolution-time marker; reaching the executor means the invariant was not expanded, and the documented invariant.passed/invariant.failures context keys are never set). In generate_ial_markdown (src/main.rs:6697-6735) add a Status column to the primitives table: read optional `status` per primitive, default "implemented", render "not implemented (always fails)" / "partial (resolution-time only)"; also correct the context_sets cell for invariant_check or annotate it. Run `ntnt docs --generate` to regenerate IAL_REFERENCE.md and SYNTAX_REFERENCE.md and sync .github/copilot-instructions.md (per repo policy).

STEP 7 — optional polish (small, flag in PR): interpreter.rs:7044 E007 from MethodCall has suggestion:None — wire crate::error::find_suggestion against environment keys, mirroring the UndefinedVariable path at interpreter.rs:6838-6840, so `m.f()` typos get "did you mean" hints.

Verification: cargo build (debug, so tests pick it up), cargo test --test doc_claims_tests, cargo test (full suite — the lint fix touches unused_import so check cli_tests), ntnt docs --generate, git diff docs/ to confirm regeneration is clean.

### Files

- `/home/larimonious/repos/ntnt/tests/doc_claims_tests.rs` — NEW. ~26 integration tests, one per CLAUDE.md behavioral claim; helpers cloned from tests/language_features_tests.rs plus a run_ntnt_lint helper that invokes `ntnt lint` and captures stdout/stderr/exit.
- `/home/larimonious/repos/ntnt/CLAUDE.md` — Rewrite rule 3 (UFCS embraced: dot-call = free function with receiver as first arg; parens disambiguate; map-closure caveat); rule 5 (semicolons parse, lint warns, not parser corruption); rule 8 (mut enforced only for indexed mutation); replace stale rule 10 (module-level map works) with the map-closure caveat to preserve numbering; annotate rule 11 (warning is emitted, not silent).
- `/home/larimonious/repos/ntnt/docs/AI_AGENT_GUIDE.md` — Line ~85 critical-syntax bullet, ~395-420 'WRONG - method-style calls' block, and troubleshooting row ~3252 (`unexpected token '.'` does not exist) — all rewritten to document UFCS and its real failure mode (E007 on map-stored closures).
- `/home/larimonious/repos/ntnt/docs/syntax.toml` — Add [operators.method_call] entry documenting UFCS sugar after [operators.member].
- `/home/larimonious/repos/ntnt/src/main.rs` — Three edits: (1) add "method_call" to op_categories in generate_syntax_markdown (~6220-6232); (2) generate_ial_markdown (~6697-6735): read optional per-primitive `status` from ial.toml, add Status column to primitives table; (3) collect_from_expr MethodCall arm (~4948): insert method name into used-names set so UFCS counts as import usage.
- `/home/larimonious/repos/ntnt/docs/ial.toml` — Add status = "not_implemented" to [primitives.sql]; status = "partial" + corrected context note to [primitives.invariant_check] (invariant.passed/failures are never set; executor at src/ial/execute.rs:1108 is a not-expanded fallback).
- `/home/larimonious/repos/ntnt/docs/IAL_REFERENCE.md` — Regenerated via `ntnt docs --generate` (do not hand-edit; rows for Sql/InvariantCheck gain Status).
- `/home/larimonious/repos/ntnt/docs/SYNTAX_REFERENCE.md` — Regenerated via `ntnt docs --generate` (gains method_call operator row).
- `/home/larimonious/repos/ntnt/src/interpreter.rs` — OPTIONAL (flag separately): MethodCall E007 at ~7044 currently returns suggestion:None — wire crate::error::find_suggestion over environment keys like the UndefinedVariable path at ~6838.

### Tests

- doc_rule01_bare_brace_map_is_parse_error: `let user = { "name": "Alice" }` → exit 1, stderr contains E002 and "Expected expression"
- doc_rule01_bare_brace_is_block_expression: `let x = { 1 + 2 }` + print(x) → stdout "3", exit 0
- doc_rule02_dollar_interpolation_is_literal: prints `Hello, ${name}!` literally and `Hello, World!` for #{name}, exit 0
- doc_rule03_ufcs_builtin: print("abc".len()) → "3", exit 0
- doc_rule03_ufcs_user_function: fn double(x: Int) -> Int {...}; print(5.double()) → "10"
- doc_rule03_ufcs_imported_function: import { trim } from "std/string"; print("  hi  ".trim()) → "hi"
- doc_rule03_ufcs_no_import_needed: print(map { "a": 1 }.keys()) → "[a]" (stdlib globally registered)
- doc_rule03_parens_disambiguate_map_key_shadow: m = map { "keys": 42, "a": 1 }; m.keys → "42"; m.keys() output contains both "a" and "keys" (assert contains, not order)
- doc_rule03_map_closure_not_dot_callable: map { "f": fn(x){x+1} }; m.f(10) → exit 1, stderr contains E007 and "Undefined function: f"
- doc_rule03_lint_no_unused_import_for_ufcs (post lint-fix): import { trim } used only as s.trim() → lint output does NOT contain unused_import
- doc_rule04_route_fns_not_importable: import { get, listen } from "std/http/server" → exit 1, stderr contains "is not exported from 'std/http/server'"
- doc_rule05_semicolons_execute_correctly: let x = 1; let y = 2; print(x + y); → stdout "3", exit 0
- doc_rule05_semicolons_lint_warns: same file → lint exit 0, stdout contains "unnecessary_semicolon"
- doc_rule06_otherwise_nondiverge_is_lint_error: `let v = f() otherwise { 0 }` → lint exit 1, output contains "otherwise block does not diverge"
- doc_rule06_otherwise_nondiverge_runtime_error_on_error_path: int("notanum") otherwise { 0 } → exit 1, stderr contains "otherwise block must diverge"
- doc_rule06_otherwise_diverging_recovers: otherwise { return 0 } on failing int() → prints "0", exit 0
- doc_rule07_range_function_undefined: range(3) → exit 1, stderr contains E006 and "Undefined variable: range"; companion 0..3 prints 0,1,2
- doc_rule08_plain_reassign_without_mut_succeeds: let x = 0 / x = x + 1 / print(x) → "1", exit 0 (locks ACTUAL behavior)
- doc_rule08_index_assign_requires_mut: let arr=[1,2,3]; arr[0]=99 → exit 1, stderr contains "Cannot mutate 'arr': variable is not declared with 'let mut'"
- doc_rule08_index_assign_with_mut_works: let mut arr; arr[0]=99 → prints "[99, 2, 3]"
- doc_rule09_pipe_closure_is_parse_error: let f = |x| x * 2 → exit 1, stderr contains E002; fn(x){x*2} variant prints "6"
- doc_rule10_module_level_map_works: module-level let config = map {...}; print from main → "{ k: v }", exit 0 (locks the un-stale behavior)
- doc_rule11_for_in_string_zero_iterations_with_warning: stdout's only program output is "done"; combined output contains "for..in on String"; chars("ab") loop prints a,b
- doc_rule12_template_double_braces: """Hello {{name}}""" → "Hello World"; single-brace stays literal "Hello {name}"
- doc_rule13_contract_placement_parses_and_runs: divide with requires/ensures between return type and body → prints "5", exit 0
- doc_rule14_truthiness: if 0 → truthy branch; ""/[]/map{} → falsy branches; combined output contains "Non-boolean condition" warning
- doc_rule15_missing_map_key_returns_none: m["missing"] → "none"; has_key(m,"missing") → "false"; m.missing → "none"
- doc_rule16_for_map_iterates_keys: for k in map prints keys; entries(m) accessed via e.key/e.value (NOT printed whole — field order nondeterministic) prints a=1, b=2

### Risks

- Message-string brittleness: tests assert on error/warning text (e.g. "otherwise block must diverge", "Cannot mutate"). Mitigate by asserting error codes (E002/E005/E006/E007) and lint rule ids plus one short stable fragment, never full lines.
- Nondeterministic output: printing entry maps interleaves field order ({ key: a, value: 1 } vs { value: 2, key: b }); map key iteration order looked sorted but is unverified — tests must use e.key/e.value access and contains-assertions.
- Locking rule-8 behavior cuts both ways: if the maintainer later decides plain reassignment without mut is a bug, doc_rule08_plain_reassign_without_mut_succeeds must be inverted in the same PR — the test header should say so explicitly.
- The unused_import lint fix (counting MethodCall names) can mask a genuinely unused import when an unrelated dot-call shares the name (e.g. import { keys } unused but m.keys() builtin used) — acceptable false-negative; note in commit message.
- syntax.toml/ial.toml generator uses fixed key arrays (op_categories, prim_names) — adding toml entries without updating the arrays in src/main.rs silently renders nothing; verify regenerated markdown diff contains the new rows.
- docs --generate also syncs .github/copilot-instructions.md; review that diff so the UFCS change propagates and nothing else regresses.
- CLAUDE.md rule renumbering: external references (skills, CODEX.md, memory files) may cite rules by number — plan keeps numbering stable by replacing rule 10 in place rather than deleting.
- Warning channel ambiguity: the [WARN] runtime lines' stream (stdout vs stderr) was not isolated during discovery — implementer should assert against the correct stream after one local run, or assert on combined output.

### Open decisions

- Rule 8 semantics: keep plain reassignment-without-mut as documented behavior (this plan), or fix the interpreter to enforce is_mutable on simple Assign targets too (breaking change for existing .tnt code; the only current check is interpreter.rs:6828 for indexed assignment)?
- IAL Sql primitive: mark as not_implemented in docs (this plan), or remove it from ial.toml/IAL_REFERENCE.md entirely until implemented?
- Should a new lint rule warn on `${...}` inside regular strings (rule 2's failure mode is currently completely silent)? Cheap to add next to unnecessary_semicolon but expands scope.
- Include the optional interpreter.rs E007 suggestion improvement (find_suggestion for dot-call typos) in this PR or defer?

---

## PR-5 — Index out-of-bounds loudness (Rec 3, decision-gated)

**Size:** M &nbsp;|&nbsp; **Estimated LOC:** ~120-160 impl (interpreter gating ~70, suppression ~40, comment/docs touches ~30) + ~250 tests (10 unit ~80, 10 integration ~170)

**Scoping verdict:** Stage it: ship option (b) — runtime loudness for out-of-bounds ARRAY (and String) indexing, TypeMode-gated, map-missing-key stays silent — in 0.4.11 now. Defer option (a) (typechecker infers Option<T> for index) to a 0.5-track design doc, because (a) is not safe as a typechecker-only change: the runtime returns BARE elements for in-bounds access (not Some-wrapped), so a checker that says Option<T> would bless `unwrap(arr[i])` and `match arr[i] { Some(v) => ... }`, both of which FAIL at runtime on valid in-bounds values (verified: unwrap() on a bare value is a runtime TypeError, interpreter.rs ~3307; match_pattern has no implicit-Some coercion, ~8240; is_some(bare) returns false, ~3046). (a) requires first deciding the runtime representation question (wrap index results in Some, or formalize Optional-as-nullable and make unwrap/is_some/match accept bare values). Skip option (c) as specified — the typechecker has no flow narrowing, so 'without a None-guard' is undetectable and every len()/has_key-guarded access would false-positive; the only honest degraded form is (a)-with-Warning-severity, which inherits (a)'s problems.

### Summary

Resolve the DD-063 index/Option type-reality mismatch by making out-of-bounds array/string read access loud at runtime following the existing TypeMode pattern (strict → E010 IndexOutOfBounds error; warn → deduped [WARN] + None; forgiving → silent None), exactly mirroring how the same Expression::Index arm already gates Option/Result indexing (interpreter.rs:6481-6552) and how non-boolean if-conditions are handled (interpreter.rs:5175-5196). Map-missing-key stays silent-None: it is documented intentional DX (CLAUDE.md rule 15, has_key() exists), it dominates real usage (127 of 150 index occurrences in examples/ are string-key map access vs 23 array-style), map.field access has identical silent semantics and would need symmetric noisy treatment, and 'optional key' is a legitimate idiom (req.params, config) whereas OOB array access is almost always a bug (off-by-one, short split() result, missing regex group). A small suppression mechanism keeps the documented escape hatches (`arr[i] ?? default`, `arr[i]?`) warning-free. Typechecker honesty (option a) is deferred: verified blast radius is that `ntnt lint` reports Error-severity diagnostics in DEFAULT mode at every typed boundary (annotated lets ~typechecker.rs:1005, returns ~1218, call args ~2806-2840), lint exits 1 on errors (main.rs:3463), and CI lints examples/ (ci.yml:178) — and more fundamentally the runtime's actual index type is the nullable union 'T | None', not enum Option<T>, so the annotation Option<T> would be a new lie that crashes checker-blessed code.

### Approach

VERIFIED CURRENT STATE (refs confirmed on main @ 6374d95, v0.4.11):
- Index eval: src/interpreter.rs Expression::Index arm at 6437-6584. Array OOB returns Value::none() unconditionally at 6452 (positive) and 6446 (negative-overflow); String OOB at 6459/6465-6468; Map missing key at 6472-6474. NO TypeMode gate on any of these. The Option/Result-indexing arm at 6483-6552 IS TypeMode-gated and is the template (Strict → runtime_error_with_context; Warn → crate::config::type_warn_dedup(key, msg) then degrade; Forgiving → silent).
- Typechecker Index inference: src/typechecker.rs:1933-1947 (TODO comment at 1933-1937 confirmed; infers unwrapped element type).
- type_warn_dedup: src/config.rs:150-161, thread-local WARNED_LOCATIONS HashSet, key-based, prints '[WARN] {msg}' to stderr; clear_type_warnings() resets per request.
- E010 IndexOutOfBounds { index: i64, length: usize } already exists (src/error.rs:112, code mapping :218) and is already used for index ASSIGNMENT OOB (interpreter.rs:6873, 6912) — read access is the only silent path.
- Existing regression locks: unit tests interpreter.rs ~14397-14440 assert OOB→None explicitly under TypeMode::Forgiving via set_test_type_mode + TYPE_MODE_MUTEX (they survive this change unmodified).
- get_type_mode default = Warn (config.rs:67-79); tests use thread-local override.

OPTIONS ANALYSIS (for the maintainer):
(a) Typechecker honesty — DO NOT do as a standalone change. Mechanics verified: Type::Optional(T) is incompatible with bare T (types.rs:108-153 has no (Optional(a), b) arm; even Optional<Any> vs String is incompatible since the Any arm only matches top-level Any). New Error-severity diagnostics would fire in DEFAULT lint mode at: annotated let init (typechecker.rs ~1005), return-type checks (~1218), call-arg checks vs concrete sigs (~2806-2840). ntnt lint exits 1 on errors (main.rs:3463-3465); CI runs `ntnt lint examples/` (ci.yml:178). Quantified usage: 150 index expressions across 24/55 example files (127 map-string-key, 23 array-int); est. 80-150 more embedded in tests/*.rs .tnt snippets. Many flow into print/+ (no diagnostic since print is Any-variadic and String+Any→String), so direct new-error count is likely dozens, but transitive flow through unannotated lets into typed params spreads it. Chained index m['a']['b'] silently degrades to Any (Index over Optional hits the `_ => Type::Any` arm) unless auto-unwrap is mirrored. Migration via `??` IS mechanical (NullCoalesce typing at 2435-2444 unwraps cleanly; runtime 5778-5808 passes bare values through). Migration via `?` is NOT fully mechanical (typechecker.rs:2358-2396 warns when enclosing fn doesn't return Optional/Result). otherwise-blocks unwrap cleanly (merge_return_otherwise_type ~726, ~956-979). THE BLOCKER: runtime in-bounds index returns the bare element, so unwrap(arr[i]) runtime-errors ('unwrap() requires Option or Result'), match Some(v) never matches, is_some() returns false — the typechecker would actively steer agents into checker-approved runtime crashes. (a) is only sound after a runtime-representation decision (wrap in Some = true semver-major: print(arr[0]) output changes everywhere; OR formalize nullable-union semantics and make unwrap/is_some/match accept bare values = smaller break, blesses current runtime). That decision deserves its own DD.
(c) Lint-only None-guard analysis — NOT FEASIBLE as specified. lint_ast (src/main.rs:3471+) is purely syntactic (serde_json issues, no type info). The typechecker is the only home, but it has no flow narrowing (acknowledged in DD-063 §E), so it cannot see `if i < len(arr)` or `has_key(m,k)` guards — every guarded access false-positives. A degraded variant (Warning when an Index expression is the DIRECT argument/init/return in a typed position without an immediate ??/?/otherwise parent) is implementable at the three check sites since they hold the Expression, but it inherits the false-positive problem and trains agents to distrust warnings. Skip, or revisit as strict-lint-only after (b) ships.
(b) Runtime loudness — RECOMMENDED. Implementation steps:
1. src/interpreter.rs Expression::Index, Array arm (6442-6453): extract a helper `fn index_oob_outcome(&self, kind: &str, idx: i64, len: usize) -> Result<Value>` (or inline match) following the 6493-6521 template: TypeMode::Strict → Err(IntentError::IndexOutOfBounds { index: idx, length: len }) (reuses E010; same no-line-info precedent as DivisionByZero); TypeMode::Warn → type_warn_dedup(&format!("index_oob:{}:{}:{}", kind, idx, len), &format!("{} index {} out of bounds (length {}) — returning None. Use `?? default`, a len() guard, or get_index({}, i, default).", kind, idx, len, ...)) then Ok(Value::none()); TypeMode::Forgiving → Ok(Value::none()). Route BOTH the arr.get() miss at 6452 AND the negative-overflow early-return at 6444-6447 through it.
2. Same gate for the String arm (6454-6468), kind="String", including negative-overflow path 6456-6460.
3. Leave Map arm (6470-6474) and FieldAccess-on-Map (6592) untouched.
4. Suppression for documented escape hatches: add `suppress_index_warn: std::cell::Cell<bool>` (or u32 depth) on Interpreter. In NullCoalesce eval (5778, before evaluating left), Try eval (7135), set it when the immediate operand `matches!(expr, Expression::Index{..})` via a small RAII-style set/restore around the eval_expression call; the OOB gate checks the flag and returns Ok(none()) silently when set. Note `otherwise` needs no special handling in strict mode (Statement::Let otherwise already catches runtime errors at 4876-4886 and converts to Err, which triggers the block) but DOES need the flag for warn-mode noise (`let x = arr[i] otherwise {...}` at 4878). Apply at the three sites: NullCoalesce lhs, Try inner, let/return-with-otherwise value eval (4878, ~5126). Recommend suppression applies in BOTH Warn and Strict (?? is the documented 'universal safety net'; strict-erroring under ?? would leave strict mode with no ergonomic optional-access form besides get_index).
5. Update the typechecker TODO comment (typechecker.rs:1933-1937) to record the decision: runtime is loud per DD-063 Rec 3; Option<T> inference deferred pending nullable-vs-enum unification DD.
6. Docs (mandatory per CLAUDE.md): docs/AI_AGENT_GUIDE.md indexing section — document warn/strict OOB behavior and the `?? / get_index / len() guard` idioms; add a line to CLAUDE.md critical rules (amend rule 15 area: 'array OOB warns in warn mode, errors in strict; map missing key returns None by design'); note in design-docs/dd-063-language-assessment.md that Rec 3 is resolved via direction (b); run `ntnt docs --generate` (no stdlib sig changes, so expect no STDLIB_REFERENCE diff, but it also syncs agent files).
7. Sanity-check examples/: the 23 array-style usages are in-bounds or loop-bounded (spot-checked environment.tnt, contracts_full.tnt, collections_demo.tnt); CI's `ntnt lint examples/` is unaffected (this is runtime-only); any example that runs in CI keeps exit 0 since warn mode doesn't change values or exit codes.

### Files

- `/home/larimonious/repos/ntnt/src/interpreter.rs` — Expression::Index arm (~6437-6584): TypeMode-gate Array OOB (6442-6453 incl. negative path 6444-6447) and String OOB (6454-6468) — Strict → Err(IntentError::IndexOutOfBounds), Warn → type_warn_dedup + Ok(none), Forgiving → Ok(none). Add suppress_index_warn Cell on Interpreter; set it around lhs/inner eval in NullCoalesce (5778), Try (7135), and let/return-otherwise value eval (4878, ~5126) when operand is Expression::Index. Add unit tests beside existing OOB tests (~14397-14440) using set_test_type_mode for Warn (value still None) and Strict (Err with E010).
- `/home/larimonious/repos/ntnt/src/typechecker.rs` — Update the TODO comment at 1933-1937 only: record that runtime is now loud (DD-063 Rec 3 direction b) and Option<T> inference is deferred pending a nullable-union vs enum-Option unification DD. No inference change.
- `/home/larimonious/repos/ntnt/tests/language_features_tests.rs` — Add integration tests using run_ntnt_file with NTNT_TYPE_MODE env: warn/strict/forgiving matrix for array OOB, string OOB, negative OOB, ??-suppression, otherwise-suppression, map-missing-key-stays-silent, in-bounds-no-warn.
- `/home/larimonious/repos/ntnt/docs/AI_AGENT_GUIDE.md` — Document indexing semantics: in-bounds returns element; array/string OOB warns (warn mode) / errors E010 (strict) / silent None (forgiving); map missing key returns None by design (use has_key); blessed patterns: `arr[i] ?? default`, len() guard, get_index(arr, i, default).
- `/home/larimonious/repos/ntnt/CLAUDE.md` — Amend critical-rules list (near rule 15): array OOB indexing now warns in default warn mode and errors in strict; map missing key remains silent None.
- `/home/larimonious/repos/ntnt/design-docs/dd-063-language-assessment.md` — Mark Rec 3 resolved: direction (b) shipped in 0.4.11; (a) deferred to a future DD on Option/nullable runtime unification; (c) rejected (no flow narrowing).

### Tests

- tests/language_features_tests.rs: array OOB in default (warn) mode — `let arr = [1, 2]\nprint(arr[10])` → exit 0, stdout 'none', stderr contains '[WARN]' and 'out of bounds'
- tests/language_features_tests.rs: array OOB with NTNT_TYPE_MODE=strict → exit != 0, stderr contains 'Index out of bounds' / E010
- tests/language_features_tests.rs: array OOB with NTNT_TYPE_MODE=forgiving → exit 0, stdout 'none', stderr has no '[WARN]'
- tests/language_features_tests.rs: `print(arr[10] ?? 0)` in warn mode → stdout '0', stderr has NO OOB warn (suppression); same program under strict → exit 0 (?? suppresses strict error)
- tests/language_features_tests.rs: `let x = arr[10] otherwise { print("fallback") return }` in warn mode → otherwise fires, no OOB warn
- tests/language_features_tests.rs: map missing key `m["missing"]` in warn AND strict mode → stdout 'none', no warning, exit 0 (documented DX preserved)
- tests/language_features_tests.rs: string OOB `"hi"[99]` warn mode → 'none' + [WARN]; negative OOB `arr[-99]` warn mode → 'none' + [WARN]
- tests/language_features_tests.rs: in-bounds access `arr[0]` and `arr[-1]` in strict mode → correct value, exit 0, no warning
- tests/language_features_tests.rs: warn dedup — two identical OOB accesses in a loop produce exactly one [WARN] line (count occurrences in stderr)
- src/interpreter.rs unit tests (beside ~14397): eval('[1,2,3][99]') under set_test_type_mode(Warn) → Value None variant; under Strict → Err matching IntentError::IndexOutOfBounds { index: 99, length: 3 }; existing Forgiving tests unchanged and passing

### Risks

- Strict-mode behavioral change: programs run under NTNT_TYPE_MODE=strict that previously tolerated OOB-as-None now hard-error with E010. Strict is opt-in and documented as 'production', so this is arguably the point — but it is a runtime break within 0.4.x for strict users.
- Warn-mode noise in HTTP servers: clear_type_warnings() resets the dedup set per request, so a hot handler with an OOB bug warns once per request to stderr. This is consistent with existing non-bool-condition warns but will be much more visible.
- The dedup key has no source location (AST Expressions carry no spans), so two distinct OOB bugs with the same collection-type/index/length signature warn once; and warnings cannot cite a line number — same limitation as all existing runtime warns, worth stating in the warning text guidance ('returning None' + idiom hints compensate).
- Suppression mechanism only sees the IMMEDIATE parent (?? / ? / otherwise with a direct Index operand). `let x = arr[i]` followed by `x ?? 0` on the next line still warns at the index site — technically correct (the access was unguarded) but may surprise users who consider it guarded.
- Regex-capture and split() patterns (`groups[1]`, `fields[2]`) are the most common legitimately-dynamic accesses in examples and agent code; on malformed input these now warn. That is the desired behavior per DD-063, but expect a transition period of agents adding `??` to such sites.
- If the maintainer later picks option (a) without the runtime-representation DD, the unwrap/match/is_some incompatibility documented here becomes a checker-blessed crash class — this analysis should be linked from the deferral note to prevent that path being taken casually.
- Index assignment OOB already errors unconditionally (E010 at interpreter.rs:6873) regardless of TypeMode — minor asymmetry remains (reads gated, writes always error); acceptable but Greptile may flag it.

### Open decisions

- Map missing-key scope: confirm m['missing'] (and map.field access) stays silent-None in all modes (recommended), or make strict mode error on missing keys too for symmetry? Note 127 of 150 index usages in examples/ are map-key access — symmetric treatment would be very loud.
- Escape-hatch suppression: should `arr[i] ?? default` / `arr[i]?` / `... otherwise {}` suppress the OOB warn AND the strict-mode error (recommended: yes for both — ?? is the documented universal safety net), or should strict error even under ?? (forcing get_index/len-guards as the only strict-safe forms)?
- Include String[i] OOB in the same gate (recommended: yes, symmetric), or arrays only?
- Is a strict-mode runtime behavior change (OOB now errors) acceptable in a 0.4.x release, or does it need a release-notes breaking-change callout / 0.5?
- Direction for the deferred option (a): when the Option DD is written, which unification — wrap in-bounds index results in Some(v) (honest enum Option, semver-major: changes print output and all value flows) or formalize Optional-as-nullable-union and make unwrap/is_some/match accept bare values (smaller break, blesses current runtime)? This decision gates any future Option<T> inference for index expressions.
- Whether to also add the degraded option (c) (Warning-severity, strict-lint-only, direct-flow-into-typed-position, no guard detection) as a follow-up, accepting false positives on len()/has_key-guarded code — my recommendation is no, skip it.

---

## PR-3 — Diagnostics I: parser error recovery + contract violation context (Rec 4a)

**Size:** M &nbsp;|&nbsp; **Estimated LOC:** ~420 impl (parser ~130, error.rs ~50, interpreter ~150, main.rs ~70, ast/parser-contract/typechecker/contracts ~20) + ~380 tests (parser/error unit ~120, tests/diagnostics_tests.rs ~260)

**Scoping verdict:** Implement both halves in one diagnostics PR: (1) opt-in panic-mode parser recovery via a new `Parser::parse_with_recovery() -> (Program, Vec<IntentError>)` consumed only by lint/validate/check paths, leaving `parse()` byte-for-byte unchanged so the run/import/REPL pipeline carries zero risk; (2) convert `IntentError::ContractViolation(String)` to a struct variant carrying clause line, call-site line, and runtime values, with clause lines threaded from `parse_contract()` through `ast::Contract` into the interpreter's `FunctionContract`.

### Summary

Two coupled diagnostics improvements for NTNT. Parser recovery: today `Parser::parse()` (src/parser.rs:29-37) aborts on the first `Err` from `declaration()`, so `lint_project` (src/main.rs:3337-3419) and `validate_project` (src/main.rs:3178-3246) report exactly one parse error per file. Add a separate `parse_with_recovery()` entry point plus a `recover` flag that enables statement-level synchronization inside `block()` (src/parser.rs:1435-1445) and the top-level loop, capped at 5 errors with cascade suppression (same-line dedup + minimum token gap). The existing `parse()` keeps its exact signature and single-error behavior, so all 18 existing call sites (run path, module imports at interpreter.rs:4264 etc., REPL, intent.rs) are untouched. E004 enrichment: contract clauses currently lose all position info — `ast::Contract` (src/ast.rs:543-547) holds bare `Vec<Expression>`, `ContractViolation(String)` has no line field, `at_line()` ignores it, and `line()` returns None so `format_error` (src/main.rs:709-815) never renders a source frame. Add per-clause line capture in `parse_contract()` (src/parser.rs:367-384), a new `ast::ContractCondition { expression, line }`, a struct variant `ContractViolation { message, line, call_line, values }`, and an identifier-walking helper in the interpreter (modeled on `extract_old_calls`, interpreter.rs:9732) that reads parameter values from the function environment *before* it is restored, producing output like `where: b = 0` plus a real source-context frame pointing at the failing `requires`/`ensures` clause.

### Approach

PART 1 — PARSER RECOVERY (src/parser.rs)

1. Add fields to `Parser` (line 12-17): `recover: bool` (default false), `errors: Vec<IntentError>`, `last_error_line: usize`, `last_error_pos: usize`. Add `const MAX_PARSE_ERRORS: usize = 5;` and `const MIN_ERROR_TOKEN_GAP: usize = 2;`.

2. Keep `pub fn parse(&mut self) -> Result<Program>` EXACTLY as-is (line 29-37). All 18 call sites (main.rs:1181/1204/1269/1581/2532/2620/2649/2684, intent.rs:3804/4276, interpreter.rs:1216/1677/4264/4520/4652/4739/9687, parser unit-test helper at parser.rs:2929) remain valid and behaviorally identical; run/import paths still abort on the first error.

3. Add `pub fn parse_with_recovery(&mut self) -> (Program, Vec<IntentError>)`:
   - Set `self.recover = true`.
   - Loop `while !self.is_at_end()`: match `self.declaration()`; on Ok push statement; on Err call `self.record_error(e)`, break if `self.errors.len() >= MAX_PARSE_ERRORS`, then `self.synchronize()`, then consume any stray top-level `RightBrace` tokens (`while self.check(&TokenKind::RightBrace) { self.advance(); }` — a bare `}` is never a valid top-level declaration; it is leftover from a broken construct).
   - Return `(Program { statements }, std::mem::take(&mut self.errors))`.

4. Add `fn record_error(&mut self, e: IntentError)` — cascade suppression: skip recording (but still synchronize) if `e.line() == Some(self.last_error_line)` or `self.current < self.last_error_pos + MIN_ERROR_TOKEN_GAP`; otherwise push and update `last_error_line`/`last_error_pos = self.current`.

5. Add `fn synchronize(&mut self)`: `self.advance()` once unconditionally (guarantees progress — this is the infinite-loop guard), then loop: stop without consuming if `peek()` is `TokenKind::RightBrace` (lets an enclosing `block()` close); stop if the token is the first on its line (`self.previous().map_or(true, |p| p.line < tok.line)` — the lexer's `skip_whitespace` at lexer.rs:267 swallows `\n`, the `Newline` TokenKind is never emitted, so line-start detection MUST use `Token.line` comparison, not a Newline token) AND `is_statement_start(&tok.kind)`; else `self.advance()`. Add `fn is_statement_start(kind: &TokenKind) -> bool` matching `Let | Fn | Type | Struct | Enum | Trait | Impl | Mod | Use | Import | Export | Pub | Server | If | While | Loop | For | Defer | Break | Continue | Return | Hash` (Hash starts `#[...]` attributes, parser.rs:173).

6. Modify `block()` (line 1435-1445): wrap the `self.declaration()` call: `Err(e) if self.recover && self.errors.len() < MAX_PARSE_ERRORS => { self.record_error(e); self.synchronize(); }` (loop continues; sync stops at `}` so the block closes normally), `Err(e) => return Err(e)` otherwise. With `recover == false` this compiles to today's behavior — this conditionality is what keeps `parse()` callers (and therefore the 47K-line interpreter pipeline) untouched.

7. main.rs consumers:
   - `lint_project` (3337-3419): replace `match parser.parse()` with `let (_, parse_errors) = parser.parse_with_recovery();`. If `parse_errors` is non-empty: push one issue per error — `{"severity":"error","rule":"parse_error","message":e.to_string(),"line":e.line(),"column":e.column()}` — increment `error_count` by `parse_errors.len()`, print the `✗` line, and SKIP `lint_ast` + `check_program_with_lint_mode` for that file (running semantic checks on a partial AST produces false positives that would confuse agents). If 5 errors were collected, append an informational issue "additional errors may be suppressed; re-lint after fixing". If empty, proceed exactly as today with the recovered AST (identical to what `parse()` would have returned). `e.line()` comes straight from `ParserError { line, .. }` — no need for the `extract_line_from_error` string-scrape (main.rs:5361), keep it only as fallback.
   - `validate_project` (3178-3246): same transformation on its Err arm (3231-3245), emitting one entry per error in the `errors` array.
   - `check_file` (2642-2653): use `parse_with_recovery`, print each error via `eprintln` with `error[E002]` prefix + line, bail with count.
   - `run_file` (1204) and all other `parse()?` sites: unchanged — run still aborts on the first parse error with the existing rich `format_error` frame.

PART 2 — E004 ENRICHMENT

8. src/ast.rs (543-547): add `#[derive(Debug, Clone, Serialize, Deserialize)] pub struct ContractCondition { pub expression: Expression, /// 1-based line of the requires/ensures keyword; 0 = unknown pub line: usize }` and change `Contract` to `{ pub requires: Vec<ContractCondition>, pub ensures: Vec<ContractCondition> }`. Name it `ContractCondition`, NOT `ContractClause` — `contracts.rs` already exports a different `ContractClause` and interpreter.rs imports both modules.

9. src/parser.rs `parse_contract()` (367-384): inside each `while self.match_token(&[TokenKind::Requires])` / `Ensures` loop, capture `let line = self.previous().map(|t| t.line).unwrap_or(0);` then push `ContractCondition { expression: self.expression()?, line }`.

10. Compiler-enforced consumers of `Contract.requires/.ensures` to update (iterate `.expression`): typechecker.rs:1106 and 1129 (bonus: set the diagnostic's line from `clause.line`); main.rs:4821-4822 (`inspect_project` JSON, `expr_to_string(&c.requires[i].expression)` — keep the JSON output as plain strings so `ntnt inspect` shape doesn't change); main.rs:5216-5219 (`collect_used_names`).

11. src/interpreter.rs `FunctionContract` (150-155): change fields to `Vec<crate::ast::ContractCondition>`. Construction sites 4989-4992 (`Statement::Function`) and 5067-5070 (job `perform_contract`) keep `c.requires.clone()`. Change `capture_old_values` (9721) signature to `&[ContractCondition]`, iterating `&clause.expression`.

12. src/error.rs: replace `ContractViolation(String)` (line 72-73) with:
    `#[error("Contract violation: {message}")] ContractViolation { message: String, line: usize, call_line: usize, values: Vec<(String, String)> }`.
    - Add `pub fn contract_violation(message: impl Into<String>) -> Self` (line 0, call_line 0, values empty) for legacy sites.
    - `error_code()` (212): pattern becomes `ContractViolation { .. }`.
    - `line()` (225-237): add `ContractViolation { line, .. } if *line > 0 => Some(*line)` — this single line is what makes `format_error`'s existing source-frame renderer (main.rs:732-792) light up for E004.
    - `at_line()` (160-171): add a ContractViolation arm (only sets when 0) so the `Statement::Located` annotation at interpreter.rs:4858-4865 backfills the call-site line when clause line is unknown.
    - `rich_display()` (183-204): for ContractViolation with non-empty `values`, append `\n  │ where: {name} = {value}` per pair and `\n  │ called from line {call_line}` when call_line > 0.
    - Update the variant construction in `test_error_codes_unique` (424).
    Display text stays `Contract violation: Precondition failed in 'divide': b != 0`, preserving every `contains("Precondition failed")` assertion (interpreter.rs:10602/10635/10737/10779) and the HTTP status mapping.

13. src/contracts.rs (221, 245, 269): switch to `IntentError::contract_violation(msg)`. Same for `assert` at interpreter.rs:2478.

14. Interpreter violation sites — enrich:
    - New helpers next to `extract_old_calls` (9732): `fn collect_identifiers(expr: &Expression, out: &mut Vec<String>)` — recursive walk over `Identifier` (skip literal name "old" and "result" handled naturally), `Binary{left,right}`, `Unary{operand}`, `Call{function, arguments}` (recurse args; for the function position only recurse if not a bare identifier — avoids listing `len`), `Index`, `FieldAccess`/property bases, grouping, with `_ => {}` catch-all, mirroring the existing extract_old_calls style. And `fn collect_clause_values(&self, expr: &Expression) -> Vec<(String, String)>`: collect identifiers, order-preserving dedup, for each do a pure `self.environment.borrow().get(name)` (NO eval — side-effect free), skip `Value::Function`/builtins, format via the existing `impl fmt::Display for Value` (interpreter.rs:244), quote strings, truncate each rendered value to ~60 chars, cap at 8 entries.
    - `call_user_function` (8607): capture `let call_site_line = self.current_line;` at function entry (at the precondition check the interpreter's `current_line` still holds the caller's `Located` statement line; after the body runs it points at the last body statement, so capture early).
    - Precondition site (8664-8678): on failure, compute `let values = self.collect_clause_values(&clause.expression);` BEFORE `self.environment = previous;` (line 8670 restores the env — collecting after would read the caller's scope and silently produce wrong/missing values; this ordering is the most important correctness detail in Part 2). Return `ContractViolation { message: format!("Precondition failed in '{}': {}", name, condition_str), line: clause.line, call_line: call_site_line, values }`. `condition_str` still comes from `Self::format_expression` (9857).
    - Postcondition site (8713-8731): same, collected before the env restore at 8722; `result` is bound in func_env at 8709-8711 so `ensures result * b == a` yields `where: result = 5, b = 2, a = 10`.
    - Job path `eval_block_with_contract_inner` (5579-5594 pre, 5637-5651 post): same treatment using the new clause fields; environment is the worker scope, no restore ordering issue.
    - Struct invariants (~9962-9972): use `contract_violation()` + `collect_clause_values` against `inv_env` before the restore at 9966; line stays 0 (invariant expressions have no captured line yet — acceptable partial coverage, note in PR).
    - HTTP mapping (9052-9080): pattern update to `IntentError::ContractViolation { message, .. }`; logic unchanged (400 for pre, 500 for post).

15. main.rs `format_error` (709-815): after the source-snippet block, add: `if let Some(IntentError::ContractViolation { values, call_line, .. }) = error.downcast_ref::<IntentError>()` print `  where: b = 0` lines (cyan label) and `  note: contract checked for call at line {call_line}` when call_line > 0 and differs from the clause line. The source frame itself needs no work — it keys off `line()`/file_hint (main.rs:821-828) which already passes the run file.

16. Docs (mandatory per project CLAUDE.md): update docs/AI_AGENT_GUIDE.md error-handling section (multi-error lint output, enriched E004 format with example), regenerate via `ntnt docs --generate` if any `// @ntnt` blocks change (the `assert` doc comment at interpreter.rs:2459-2466 mentions ContractViolation — keep text accurate), update ROADMAP.md DD-063 Rec 4a status.

REGRESSION PROOFING: before/after the change run `cargo test` (full suite: unit tests in parser/interpreter/typechecker + tests/cli_tests.rs, language_features_tests.rs, etc.), `cargo clippy`, and a script that runs `ntnt lint` + `ntnt run` over every examples/**/*.tnt (including examples/contracts.tnt and contracts_full.tnt which exercise the E004 path) and diffs the output; lint output must be identical for all currently-clean files.

### Files

- `/home/larimonious/repos/ntnt/src/parser.rs` — Add recover/errors/last_error_* fields to Parser (12-17); new parse_with_recovery(), record_error(), synchronize(), is_statement_start(); conditional recovery in block() loop (1435-1445); parse_contract() (367-384) captures per-clause line into ast::ContractCondition; new unit tests in mod tests (2924+). parse() at 29-37 unchanged.
- `/home/larimonious/repos/ntnt/src/ast.rs` — Add ContractCondition { expression, line } struct; change Contract (543-547) to Vec<ContractCondition> fields (name avoids clash with contracts.rs::ContractClause).
- `/home/larimonious/repos/ntnt/src/error.rs` — ContractViolation becomes struct variant { message, line, call_line, values } (72-73); add contract_violation() constructor; update error_code() (212), line() (225-237), at_line() (160-171), rich_display() (183-204, append 'where:' values + call-site note); fix variant construction in test_error_codes_unique (424); add rich_display test for values.
- `/home/larimonious/repos/ntnt/src/contracts.rs` — Lines 221, 245, 269: use IntentError::contract_violation() constructor instead of tuple variant.
- `/home/larimonious/repos/ntnt/src/interpreter.rs` — FunctionContract (150-155) holds Vec<ContractCondition>; construction at 4989-4992 and 5067-5070; capture_old_values (9721) takes &[ContractCondition]; new collect_identifiers/collect_clause_values helpers near extract_old_calls (9732); enrich violation sites — call_user_function pre (8664-8678, collect values BEFORE env restore at 8670) and post (8713-8731, before restore at 8722) with call_site_line captured at function entry; job path 5579-5594/5637-5651; invariants ~9962-9972 via contract_violation(); assert at 2478; HTTP mapping pattern at 9052-9080; update unit tests ~10590-10790 that construct/match the variant.
- `/home/larimonious/repos/ntnt/src/typechecker.rs` — Lines 1106, 1129: iterate clause.expression; optionally set diagnostic line from clause.line.
- `/home/larimonious/repos/ntnt/src/main.rs` — lint_project (3337-3419): use parse_with_recovery, emit one parse_error issue per error with line/column from e.line()/e.column(), skip lint_ast/typechecker when parse errors exist, note when 5-error cap hit; validate_project (3178-3246) same; check_file (2642-2653) reports all errors; format_error (709-815) prints 'where:' values and call-site note for ContractViolation; inspect_project (4821-4822) and collect_used_names (5216-5219) use .expression.
- `/home/larimonious/repos/ntnt/tests/diagnostics_tests.rs` — New integration test file following language_features_tests.rs conventions (unique_test_file, run compiled binary, assert stdout/stderr/exit code) covering multi-error lint and enriched E004 output.
- `/home/larimonious/repos/ntnt/docs/AI_AGENT_GUIDE.md` — Document multi-error lint behavior (up to 5 parse errors per file) and the enriched E004 format with a worked example.
- `/home/larimonious/repos/ntnt/ROADMAP.md` — Mark DD-063 Rec 4a (parser recovery + E004 diagnostics) as shipped.

### Tests

- Integration (tests/diagnostics_tests.rs): .tnt file with 3 distinct syntax errors in 3 separate functions → `ntnt lint` JSON contains exactly 3 parse_error issues with 3 distinct, correct line numbers; exit code 1.
- Integration: file with 8+ syntax errors → at most 5 parse_error issues reported plus the cap notice.
- Integration: single mid-expression error (e.g. `let x = 1 +` then valid code) → exactly 1 error, no cascading garbage errors from the recovered region.
- Integration: error inside a fn body followed by a later valid fn with its own error → both errors reported (proves block-level recovery + top-level stray-brace skip).
- Integration: `ntnt run` on a file with 2 syntax errors → aborts with the FIRST error only, error[E002] header and source frame unchanged (run-path regression guard).
- Integration: `ntnt lint` on a file with parse errors emits NO type_check/lint_ast issues (partial-AST suppression).
- Integration: clean file lints identically to before (golden run over examples/*.tnt as a pre/post diff script, not a checked-in test).
- Integration E004 precondition: divide with `requires b != 0`, called divide(10, 0) → stderr contains error[E004], "Precondition failed in 'divide': b != 0", `--> file:<line of requires clause>`, a source-context frame, and `where: b = 0`; exit 1.
- Integration E004 postcondition: `ensures result * b == a` violated → stderr shows clause line frame and `where:` including `result = ...`.
- Integration E004 multi-param: `requires from_balance >= amount` → where-line lists both identifiers with values, order-preserving.
- Parser unit tests (src/parser.rs mod tests): parse_with_recovery returns (partial Program with the valid statements, Vec of N errors); synchronize stops at line-start keywords and at RightBrace; recovery never loops (pathological token soup input terminates); parse() on the same input still returns Err with the same first error as before.
- Error unit tests (src/error.rs): ContractViolation line()/error_code()/at_line() behavior; rich_display renders `where: b = 0` and call-site note.
- Interpreter unit tests: collect_identifiers on `result * b == a` yields [result, b, a]; existing contract tests at interpreter.rs:10590-10790 still pass unmodified (they assert via contains() on Display text); HTTP mapping test: precondition violation in a handler still returns 400, postcondition 500.
- Job-path test: perform-block contract violation message includes clause line and values (eval_block_with_contract path).

### Risks

- Block-level recovery is gated on the `recover` flag; if any future code sets it outside parse_with_recovery, `parse()` could return Ok for broken programs and the run path would execute partial ASTs. Mitigate: keep the flag private, assert errors.is_empty() in parse(), and add the run-path regression test.
- Synchronization heuristics can mis-sync (e.g. error in a fn signature resumes inside the body, treating body statements as top-level), producing under- or over-reporting on pathological files. The same-line dedup + 2-token gap + 5-error cap bound the damage; lint correctness for clean files is unaffected because recovery only alters the error path.
- Infinite-loop risk in synchronize() if it ever fails to advance — guarded by the unconditional advance() on entry; add a token-soup termination test.
- ContractViolation variant change is crate-internal API breakage; the compiler finds all sites (contracts.rs x3, interpreter.rs x7 incl. the 9052 match, error.rs test), but external string-format consumers (HTTP error pages, IAL test runner at intent.rs, user tests asserting on stderr) depend on the Display text — message format is deliberately kept byte-identical.
- `ast::Contract` serde shape changes: `ntnt parse --json` output for functions with contracts now nests {expression, line} objects. Any external tooling parsing that JSON breaks. inspect_project output is kept unchanged (strings via expr_to_string).
- Collecting values BEFORE environment restore is easy to get wrong in review-driven refactors (restore happens first today at interpreter.rs:8670/8722); a wrong order silently yields empty/wrong `where:` values rather than a compile error — covered by the multi-param integration test.
- Value formatting in `where:` must not evaluate expressions (only env lookups) or it could trigger side effects/panics during error construction; the helper is restricted to Environment::get.
- Lint behavior change (skipping type_check when parse errors exist) reduces per-pass information for files that previously had only 1 parse error + N type notes; net win for agents but flagged as a deliberate tradeoff.

### Open decisions

- When a file has parse errors, lint now reports only parse errors and skips lint_ast/typechecker on the partial AST (recommended, avoids false positives). OK, or should semantic checks still run best-effort?
- Should `ntnt run` also print all recovered parse errors before aborting, or keep today's first-error-only rich display (recommended for v1; lint is the multi-error surface)?
- MAX_PARSE_ERRORS = 5 per file — confirm the cap.
- AST JSON shape change for `ntnt parse --json` (contract clauses become {expression, line} objects) — acceptable, or should a compatibility serializer flatten them back to bare expressions?
- Struct invariants get values but no clause line (their expressions don't flow through parse_contract) — ship as partial coverage or extend invariant storage to carry lines in this PR?

---

## PR-4 — Diagnostics II: method bridge hints, unknown-method lint, IAL suggestions, intent lint (Rec 4b)

**Size:** M &nbsp;|&nbsp; **Estimated LOC:** ~650 impl (error.rs 25, interpreter.rs 45, typechecker.rs 85, ial/resolve.rs 75, vocabulary.rs 10, intent.rs 250, main.rs 160) + ~500 tests + ~80 docs

**Scoping verdict:** Implement as a single diagnostics PR in four self-contained commits: (1) runtime method-call bridge hints, (2) typechecker unknown-method diagnostic, (3) IAL did-you-mean, (4) `ntnt intent lint`. All four claimed gaps verified against current source; line refs below are current as of HEAD 6374d95. Recommend the unknown-method lint warning fire in ALL lint modes (including Default) since the DD-063 complaint is precisely that `ntnt lint` (Default mode) passes clean on `s.length()`; promote to Error only in Strict via a new DiagnosticKind.

### Summary

Four coordinated diagnostics improvements. (1) Runtime: the MethodCall miss at src/interpreter.rs:7044-7049 hardcodes `suggestion: None`; populate it via the existing `crate::error::find_suggestion` over `Environment::keys()` (interpreter.rs:424-432, which already enumerates builtins + imported stdlib + user functions through parent scopes — this is how undefined-variable suggestions work at :5664-5672), consult a small shared alias table (length→len, map→transform, to_string→str, ...) before Levenshtein, and add a new `hint: Option<String>` field on `IntentError::UndefinedFunction` carrying the UFCS bridge hint ("methods resolve to free functions: try len(s)", receiver rendered via the existing `Interpreter::format_expression`). (2) Typechecker: the MethodCall arm at src/typechecker.rs:1811-1894 silently types unknown methods as `Type::Any`; add an unknown-method warning when the name is in neither `self.functions`, `self.builtin_sigs`, nor any variable scope (`self.lookup`), gated to non-`Any` receiver types; register the five runtime-global option/result helpers (is_some/is_none/is_ok/is_err/unwrap_or) into `builtin_sigs` so the check needs no parallel static table — this deliberately makes the broken inference-table names (length, to_str, to_string, map) warn. (3) IAL: extend `ResolveError` (src/ial/resolve.rs:55-59) with a `kind` enum and `suggestions: Vec<String>` computed in the None arm (:326-330) by Levenshtein over `Vocabulary::pattern_texts()` (new accessor), normalizing both sides with the existing `normalize_term_for_cycle` (:173-195); suggestions flow to users automatically because both `IalError::from` and the `run_assertions_ial` fallback message (src/intent.rs:2439) stringify via Display. (4) New `ntnt intent lint` subcommand: clap variant + dispatch in src/main.rs, analysis in a new `intent::lint_intent_file()` that builds the vocabulary via `Glossary::to_ial_vocabulary_full`, statically resolves every when-clause and outcome (mirroring the exact fallback chain `run_tests_against_server` uses), classifies failures as unknown-term (with suggestions) or cycle via `resolve_with_trace`, scans all glossary entries for cycles with dummy params, and reports orphan glossary entries; exit 1 on unresolved/cycles, 0 otherwise, `--json` for CI.

### Approach

COMMIT 1 — runtime bridge hints.
1a. src/error.rs: add `hint: Option<String>` to `IntentError::UndefinedFunction` (lines 91-97; only one non-test construction site exists, interpreter.rs:7044, so this is cheap). Add `pub fn hint(&self) -> Option<&str>` accessor. Extend `rich_display()` (:183-204) to append `\n  └─ hint: {hint}` after the did-you-mean line. Add `pub const METHOD_ALIAS_HINTS: &[(&str, &str)]` here (shared by interpreter + typechecker): `[("length","len"),("size","len"),("count","len"),("map","transform"),("to_string","str"),("to_str","str"),("append","push"),("upper","to_upper"),("lower","to_lower")]`. Update the test constructions in error.rs (~line 431) with `hint: None`.
1b. src/interpreter.rs MethodCall Err arm (:7043-7049): `let candidates = self.environment.borrow().keys();` then suggestion = alias-table hit IF the alias target is present in candidates (handles to_upper/to_lower needing an import), else `crate::error::find_suggestion(method, &candidates)`. Build hint: when a suggestion exists, `format!("NTNT methods resolve to free functions — try {}({})", sugg, Self::format_expression(object))` (append `, ...` if `arguments` non-empty); when none, a generic "NTNT methods resolve to free functions: call name(receiver, args) or define fn {method}(self, ...)". Set `line: self.current_line` instead of 0 (the Located-statement annotator at :4858-4865 only fills line when None, so this is strictly better). Optional polish in same commit: the module-miss error at :7010-7013 gains `find_suggestion(method, &fields.keys()...)` appended to its message.
1c. src/main.rs `format_error` (:708-815): after the "Did you mean" block (:803-810), print `help: {hint}` via the new `hint()` accessor.

COMMIT 2 — typechecker unknown-method diagnostic.
2a. src/typechecker.rs `register_builtins` (:3363): add sigs for `is_some`/`is_none`/`is_ok`/`is_err` ([value=>Any]→Bool) and `unwrap_or` ([value=>Any, default=>Any]→Any). Verified these ARE runtime global builtins (interpreter.rs:3040-3356) but are missing from `builtin_sigs` — registering them both fixes free-function arity coverage and lets the unknown-method check use builtin_sigs as the single source of truth. Critically, do NOT add length/to_str/to_string/map: verified they do not exist at runtime ("length" is only a template filter, interpreter.rs:8121), so they must warn even though the inference table at :1862-1863/:1848 types them.
2b. Add `DiagnosticKind::UnknownMethod` to the enum (:24-33). In `check_program_with_lint_mode` (:199-212), promote `UnknownMethod` warnings to Error in Strict alongside the annotation kinds.
2c. In the MethodCall arm (:1811-1894), after `obj_type` is computed: skip when `matches!(obj_type, Type::Any)` (suppresses module-alias receivers — `register_import` binds aliases as `Type::Any` at :3281 — and untyped params). Known-check: `self.functions.contains_key(method) || self.builtin_sigs.contains_key(method) || self.lookup(method).is_some()` (the scope lookup at :434 covers let-bound lambdas used via UFCS, and unknown-import bindings at :3322/:3348/:3357). If unknown: `let line = self.find_line_near(&format!(".{}(", method));` candidates = functions ∪ builtin_sigs keys; suggestion = alias table (only if target in candidates) else `crate::error::find_suggestion`; emit via `emit_with_kind(Severity::Warning, DiagnosticKind::UnknownMethod, format!("Unknown method '{}' — no function with this name is defined or imported", method), line, Some(format!("NTNT methods resolve to free functions — try {}({})", target, expr_search_hint(object))))`. Keep the inference arms unchanged (typing stays useful for the alias-hinted names).
2d. False-positive guard: add `has_unresolved_import: bool` to `TypeContext`; set it in `register_import` when the wildcard file-resolution branch (:3294) fails AND in the unknown-module fallback (:3354-3358); suppress UnknownMethod emission when set. Warnings emit unconditionally of `strict_lint` (matching how comparison warnings already work), so Default-mode `ntnt lint` reports them.

COMMIT 3 — IAL did-you-mean.
3a. src/ial/vocabulary.rs: `pub fn pattern_texts(&self) -> Vec<String>` over `self.patterns` (:203).
3b. src/ial/resolve.rs: add `pub enum ResolveErrorKind { UnknownTerm, Cycle, MaxDepth }` and extend `ResolveError` (:55-59) with `pub kind: ResolveErrorKind` and `pub suggestions: Vec<String>`. Update the three construction sites (:208, :235, :326). New helper `fn suggest_similar_terms(text: &str, vocab: &Vocabulary) -> Vec<String>`: normalize the unknown text and each `pattern_texts()` entry with `normalize_term_for_cycle` (:173-195, exactly as the task suggests — strips `{param}`/quoted args to `<p>`), score with `crate::error::levenshtein_distance` (pub, error.rs:259), keep `dist <= max(2, normalized.len()/4)` (identifier thresholds in `find_suggestion` are too tight for phrases), return top 3 ORIGINAL pattern texts (with `{param}` placeholders visible) sorted by distance. Display impl appends `\n  did you mean: 'a', 'b'?` when suggestions non-empty — this auto-propagates through `IalError::from` (mod.rs:80-83) and the `run_assertions_ial` fallback message (intent.rs:2439), which is what `ntnt intent check -vv` prints (main.rs:4216-4221). Change `resolve_with_trace`'s `trace.error` mapping (:122) from `e.message.clone()` to `e.to_string()` so Studio traces include suggestions (verified no tests assert on the old exact string).
3c. src/ial/mod.rs: re-export `ResolveErrorKind` (:45-47).

COMMIT 4 — ntnt intent lint.
4a. src/main.rs: add `Lint { intent_file: PathBuf, #[arg(long)] json: bool }` to `IntentCommands` (:392-472) with doc comment; dispatch in `run_intent_command` (:3916-3939) to new `run_intent_lint_command`. Unlike Check, lint needs only the .intent file — no server subprocess, no .tnt pairing (accept either path and use `intent::resolve_intent_tnt_pair` only to locate the .intent when given a .tnt).
4b. src/intent.rs: new public analysis entry `pub fn lint_intent_file(intent: &IntentFile) -> IntentLintReport` (Serialize-able structs: `IntentLintReport { errors: Vec<IntentLintFinding>, warnings: Vec<IntentLintFinding>, terms_checked, scenarios_checked }`, `IntentLintFinding { kind: "unresolved_term"|"cycle"|"orphan_term"|"unresolved_when", feature, scenario, text, suggestions: Vec<String>, detail }`). Mechanism:
  - `let glossary = intent.glossary.clone().unwrap_or_default();` (Glossary derives Default, :96-99); `let vocab = glossary.to_ial_vocabulary_full(&intent.components, &intent.invariants);` (:285).
  - Per scenario: (i) when-clause — `glossary.resolve_when_clause(&s.when_clause)` (:515) returning None → `unresolved_when` error with suggestions from glossary action-type term patterns; (ii) each outcome — MIRROR the execution path, not raw ial::resolve: first `resolve_component_reference` (:2023), else `resolve_outcomes_with_context(outcome, components, invariants)` (:961); empty result == exactly what `resolve_scenario_with_base_dir` (:1604-1633) records as `unresolved`. For each unresolved outcome, run `ial::resolve_with_trace(&Term::new(outcome), &vocab)` purely to classify: `kind == Cycle` → cycle finding (with the path message resolve.rs already formats); else unresolved_term finding carrying `err.suggestions`. Given-clauses are intentionally skipped (execution treats them as descriptive, :1564).
  - Glossary-wide cycle scan (catches cycles in entries no scenario touches): for each `pattern_texts()` entry from the glossary portion, substitute every `{param}` with a dummy quoted value (`"x"`), `ial::resolve` it, collect Cycle-kind errors, dedupe by normalized cycle message.
  - Orphans: usage corpus = all scenario when/given/outcome strings + every other glossary term's comma-split meaning parts (after `convert_params_to_ial`, :485) + component `inherent_behavior` lines + invariant assertions. A glossary term is used iff its `Pattern::new(convert_params_to_ial(term.term)).match_text(usage)` hits any corpus entry (whole-string match is correct — that is how vocabulary lookup works, vocabulary.rs:240-261) or exact case-insensitive equality. Unused → orphan warning.
4c. run_intent_lint_command rendering: header, then errors with `did you mean: '...'?` lines, cycles with arrow paths, orphan warnings, summary counts. `--json`: serde_json of the report. Exit semantics for CI: `std::process::exit(1)` when `report.errors` non-empty (unresolved terms/when-clauses/cycles); orphans are warnings → exit 0; parse failure → `anyhow::bail!` which main() routes through format_error + exit(1) (:898-901). Matches `lint_project`'s convention (exit 1 on errors only, :3463-3465).

DOCS (mandatory per CLAUDE.md): docs/IAL_REFERENCE.md (intent lint section + suggestion behavior), docs/AI_AGENT_GUIDE.md (method-call error hint + intent lint in CLI section), CLAUDE.md CLI Commands block (add `ntnt intent lint <intent>`), ROADMAP.md status, design-docs/dd-063-language-assessment.md mark rec 4 items (c)/(d) shipped. No `// @ntnt` blocks needed (no .tnt-visible stdlib functions added) — but run `ntnt docs --generate` to confirm no drift.

### Files

- `/home/larimonious/repos/ntnt/src/error.rs` — Add hint field to UndefinedFunction variant (lines 91-97), hint() accessor, extend rich_display() (183-204), add shared METHOD_ALIAS_HINTS const, fix test constructions (~line 431)
- `/home/larimonious/repos/ntnt/src/interpreter.rs` — MethodCall Err arm (7043-7049): populate suggestion via Environment::keys() + alias table, add UFCS bridge hint using format_expression (9857), set line from self.current_line; optional suggestion on module-miss error (7010-7013)
- `/home/larimonious/repos/ntnt/src/typechecker.rs` — register_builtins (3363): add is_some/is_none/is_ok/is_err/unwrap_or sigs; new DiagnosticKind::UnknownMethod (24-33) + Strict promotion (199-212); MethodCall arm (1811-1894): unknown-method warning gated on non-Any receiver, checking functions/builtin_sigs/scope lookup, alias+Levenshtein hint; has_unresolved_import suppression flag set in register_import (3294, 3354-3358)
- `/home/larimonious/repos/ntnt/src/ial/resolve.rs` — ResolveErrorKind enum + kind/suggestions fields on ResolveError (55-59), update 3 construction sites (208, 235, 326), suggest_similar_terms helper reusing normalize_term_for_cycle (173-195) + levenshtein_distance, Display renders suggestions, trace.error uses e.to_string() (122)
- `/home/larimonious/repos/ntnt/src/ial/vocabulary.rs` — Add pub fn pattern_texts(&self) -> Vec<String> on Vocabulary (203)
- `/home/larimonious/repos/ntnt/src/ial/mod.rs` — Re-export ResolveErrorKind (45-47)
- `/home/larimonious/repos/ntnt/src/intent.rs` — New lint_intent_file() + IntentLintReport/IntentLintFinding structs: vocab build via to_ial_vocabulary_full (285), when-clause check via resolve_when_clause (515), outcome resolution mirroring resolve_scenario_with_base_dir fallback chain (1604-1633), cycle scan with dummy params, orphan detection via Pattern::match_text
- `/home/larimonious/repos/ntnt/src/main.rs` — IntentCommands::Lint variant (392-472), dispatch (3916-3939), run_intent_lint_command with rendering/--json/exit codes; format_error prints new hint field (after 803-810)
- `/home/larimonious/repos/ntnt/tests/language_features_tests.rs` — Runtime integration tests: E007 with did-you-mean + free-function hint for s.length(), user-fn typo suggestion, UFCS positive control
- `/home/larimonious/repos/ntnt/tests/type_checker_tests.rs` — Lint integration tests: unknown-method warning in default mode, strict promotion to error, Any-receiver and UFCS suppression, map→transform alias hint
- `/home/larimonious/repos/ntnt/tests/intent_studio_tests.rs` — CLI tests for ntnt intent lint: clean fixture exit 0, unresolved term exit 1 with suggestion, cycle detection, orphan warning, --json shape (reuse run_ntnt helper)
- `/home/larimonious/repos/ntnt/docs/IAL_REFERENCE.md` — Document intent lint subcommand, exit codes, suggestion behavior
- `/home/larimonious/repos/ntnt/docs/AI_AGENT_GUIDE.md` — Method-call error hint behavior + intent lint in CLI/workflow sections
- `/home/larimonious/repos/ntnt/CLAUDE.md` — Add ntnt intent lint to CLI Commands and IDD sections
- `/home/larimonious/repos/ntnt/ROADMAP.md` — Mark DD-063 rec 4 diagnostics items shipped

### Tests

- Runtime (tests/language_features_tests.rs, binary-on-temp-file pattern): `let s = "hi"` + `s.length()` → exit != 0, stderr contains `error[E007]`, `Did you mean 'len'`, and the free-function hint with `len(s)`
- Runtime: user fn `fn greet(u) {...}` + `u.gret()` → suggestion 'greet' (env keys include user functions)
- Runtime positive control: `s.len()` and `arr.push(1)` via UFCS still succeed with no error (guards against regressing UFCS dispatch)
- Runtime: `m.map(f)` on array → hint suggests `transform(m, ...)` (alias table, target in env)
- Runtime: module alias miss `mod.nofunc()` → 'Module X has no function' message unchanged (no E007 regression), optionally with suggestion
- Typechecker (tests/type_checker_tests.rs, `ntnt lint` on temp file, default mode): `let s = "hi"\nprint(s.length())` → JSON output contains warning with 'Unknown method' and hint mentioning len; exit 0 (warning, not error)
- Typechecker strict: same file with NTNT_LINT_MODE=strict or --strict → severity error, exit 1
- Typechecker suppression: receiver of unknown type (untyped fn param) with unknown method → no warning; let-bound lambda `let d = fn(x){x}` + `5.d()` → no warning; file with unresolvable wildcard import → no unknown-method warnings
- Typechecker unit (src/typechecker.rs #[cfg(test)]): is_some/unwrap_or now in builtin_sigs — free call `is_some(x, y)` flags arity error
- IAL unit (src/ial/resolve.rs): resolving 'user sees the dashbord' against vocab containing 'user sees the dashboard' → Err with kind UnknownTerm and suggestions[0] == original pattern text; param normalization: vocab 'body contains {text}', term 'body containz "x"' → suggestion found; cycle error has kind Cycle and empty suggestions; Display contains 'did you mean'
- intent.rs unit: lint_intent_file on in-memory IntentFile with one resolvable scenario, one typo'd outcome, one cyclic glossary pair, one unused term → report has 1 unresolved (with suggestion), 1 cycle, 1 orphan; clean file → empty report
- CLI (tests/intent_studio_tests.rs): `ntnt intent lint fixtures/simple_server/*.intent` → exit 0, 'no issues' output; fixture with unknown term → exit 1, stdout contains 'did you mean'; `--json` → parses as JSON with errors/warnings arrays; orphan-only fixture → exit 0 with warning printed
- Regression: `ntnt intent check` on existing simple_server fixture still passes (vocab/resolution path untouched); existing test at language_features_tests.rs:2811 asserting absence of 'Did you mean' in some context still passes

### Risks

- Typechecker false positives: method calls on names the typechecker cannot see (functions defined dynamically, names injected by the HTTP runtime, unresolvable wildcard imports). Mitigated three ways — Any-receiver suppression, has_unresolved_import suppression, and Warning (not Error) severity outside Strict — but real-world .tnt projects should be lint-run before merging to confirm noise level.
- The Any-receiver gate means `s.length()` is only caught when the receiver type is inferable (literal or annotated); untyped fn params escape the lint (runtime hint still catches them). This is a deliberate precision/recall tradeoff.
- The typechecker inference table types 'length'/'to_string'/'map' (typechecker.rs:1848-1863) even though they don't exist at runtime — this plan makes them warn but keeps the typing arms; a reviewer may push to delete the dead arms instead (behavior change for downstream type inference).
- Changing trace.error from e.message to e.to_string() alters Intent Studio's displayed error strings; no tests assert on the old text (verified), but the Studio HTML UI renders it — visual check recommended.
- Adding is_some/is_none/is_ok/is_err/unwrap_or to builtin_sigs introduces arity checking on calls that previously passed silently — could surface new (correct) lint errors in existing user code; this is a behavior change worth a release note.
- Environment::keys() at the method-miss site includes plain variables, so Levenshtein may occasionally suggest a non-function name; same tradeoff already accepted for undefined-variable suggestions (interpreter.rs:5666).
- Orphan detection uses whole-string Pattern matching mirroring vocabulary lookup; glossary terms only used via the legacy direct-pattern fallback (resolve_outcome_direct) could be flagged as orphans — keep orphans as warnings (never exit-1) for exactly this reason.
- intent lint mirrors resolve_outcomes_with_context rather than pure ial::resolve to avoid false unresolved reports; if the execution fallback chain changes later, lint and check can drift — note this coupling in a code comment.

### Open decisions

- Default lint mode: plan recommends the unknown-method warning fires in Default mode (so plain `ntnt lint` catches s.length(), matching the DD-063 complaint), promoted to Error in Strict. Confirm, or restrict to Warn/Strict only.
- Alias hint table contents: proposed [length→len, size→len, count→len, map→transform, to_string→str, to_str→str, append→push, upper→to_upper, lower→to_lower]. Approve/trim.
- Orphan glossary entries: warnings that never fail CI, or add a --strict flag to intent lint that promotes them to errors?
- Should `ntnt intent lint` also accept a .tnt path and auto-locate the paired .intent (proposed: yes, via resolve_intent_tnt_pair), or take only .intent files?

---

