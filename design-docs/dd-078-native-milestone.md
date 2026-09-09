# DD-078: Intent-led native testing — first useful milestone

Status: first native milestone implemented and locally verified; see [validation](../plans/native-intent-validation.md) and the PR's exact-snapshot review/hosted checks. One independent architecture pass approved the compact plan before runtime code. This replaces the dependency wall for the first usable testing path, not the entire DD-078 roadmap.

## User-facing contract

`.intent` remains the front door. Preserve glossary `call: helper(args), source: relative/module.tnt` and ordinary direct HTTP scenarios. Add exactly one result predicate, `native assertions pass`, to designate an assertion-bearing test entry. Such entries must execute at least one existing native `assert`, must not return `Result::Err`, and must have no runtime/load error. Ordinary production-function calls instead use explicit result predicates and may return Err as expected data; they need not call assert.

Arguments and `result is ...` use bounded native literals: strings (quoted), Int, Float, Bool, arrays, `map { ... }`, None, Some(...), Ok(...), Err(...). Bare non-literal legacy text remains a String for compatibility, never numeric-string coercion. Literal parsing must reject malformed structured values and arbitrary executable expressions. Nested equality preserves native types, including enum name/variant/payload; strings do not equal numbers. Complex fixture setup and assertions remain ordinary `.tnt` functions.

`ntnt intent check library.intent` needs no paired `.tnt` or listener for native-only selections. `--case <exact scenario name>` selects a single scenario (unknown/empty selection fails). Full root-suite imports/composition are deferred. The same small function-loading/calling seam is used by Intent's compatibility executor and IAL's function-call executor, not a third divergent loader.

## Loading and assertions

Load the whole module normally once with current_file set before eval; imports use existing language relative-import rules. Never filter arbitrary statements. Modules must be authored import-safe, without test autorun footers; loading executes top-level statements. Detect a selected entry called during module loading and reject rather than invoke twice. Native execution disallows silently skipped server actions; report unsupported capability use rather than pretend it executed.

Record actual builtin assert invocations during the selected call only (not module initialization). Preserve runtime source lines and call attribution; a shadowed user function named assert must not produce builtin evidence. Failed assertions remain failures even if user code catches an error. Missing functions, unsupported predicates, no executable result checks, unresolved setup/outcomes, and no selected cases are non-green. Native Given setup is unsupported in this milestone and is reported unresolved rather than executing the helper a second time as a precondition.

## Execution/resource boundary

Native CLI runs use a clean child environment and run-owned unique temporary working directory under std::env::temp_dir; source paths are absolute. No automatic dotenv/application configuration inheritance. OS-required launch variables only; no ambient app secrets, process permission flags, DB URLs or HOME. This is local trusted-code execution, not a security sandbox: code can explicitly open absolute files/network resources. Per-case fresh interpreters alone do not isolate process-global pools/queues/environment. Prefer process-per-case using existing std/process supervision with a small internal runtime launch API (not a second supervisor), bounded deadline/output, and cleanup on success/error/timeout. The parent owns and removes fixture roots after child termination. Existing HTTP launch behavior remains compatible and is not described as hermetic.

The transport is an internal minimal serializable native value/assertion result, not a versioned evidence schema. Reports retain the existing small human/JSON shape with one shared pass predicate. Independent native debugging uses --case through this same seam. In-memory SQLite may demonstrate a disposable domain case; no managed external DB or browser adapter is required.

## Implementation ownership/checklist

- [x] Shared native loader/caller, bounded typed literals/equality, real assert attribution and non-vacuity.
- [x] Reuse process supervision for clean native case launch, owned temporary-root cleanup and non-green cleanup failures.
- [x] CLI native-only planning, optional HTTP startup, exact case selection, human/JSON/exit agreement.
- [x] CLI RED/GREEN cases: pure pass, assertion failure, returned Err failure, empty helper/no selection, structured values and expected Err, relative imports, no double invocation, unresolved Given/predicate, clean env/cleanup, simple HTTP compatibility.
- [x] Generic checked-in native examples and disposable SQLite domain example.
- [x] Fresh dev-release build, CI-equivalent nextest/doctest command, generated docs, examples/lint/Intent checks.

Publication/merge gates are recorded on the PR: exact-snapshot independent review and hosted exact-head CI. Local success does not substitute for either.

Likely files: src/intent.rs, src/ial/execute.rs, src/interpreter.rs, a small src/native_test.rs shared seam, src/stdlib/process.rs internal launch entry, src/main.rs, tests/native_intent_cli.rs, examples/native_testing, docs/ial.toml, docs/runtime.toml and generated references. No package bump/deployment/main push.

## Next milestone: root suite composition (not shipped here)

- [ ] One root .intent explicitly includes two .intent files with source-relative resolution; deterministic order is presentation, never shared test-state dependency.
- [ ] Select a child native case independently and run the same case through root with identical results.
- [ ] Root mixes native unit, disposable SQLite integration and existing HTTP behavior; HTTP starts only when selected and resources clean on failure.
- [ ] Duplicate/cyclic/missing include and empty selection reject; aggregate human/JSON/exit agree.

Next design decision: smallest explicit include syntax and qualified cross-file case selector, reviewed with these concrete cases. Then stateful HTTP/process/external-DB/browser integration reuses stdlib with bounded optional resource adapters. Protected CI, signed evidence, exhaustive source/asset/migration inventories, provider brokers and opaque grants remain separate deferred governance tracks, not test prerequisites.
