# Native Intent milestone validation

Base: `c55089414c97ec7b34fad0679979c14d7183c936` (main). Branch: `feat/intent-native-tests`.

## Design and bounded review

One independent architecture pass approved the compact plan before runtime implementation. The first exact-candidate review identified two concrete regressions: unobserved concurrent assertions could go green, and typed IAL numeric wrappers were not understood by ordering/type/accessor consumers. Both were reproduced through executable paths and corrected in one bounded repair batch. Task/scheduling capability rejection is sticky, including aliases and caught errors; numeric ordering/accessors retain compatibility without weakening typed equality. Final exact-snapshot closure and hosted CI are reported on the PR, not inferred from this document.

## RED/GREEN evidence

Actual CLI regressions cover the former paired-server requirement, native assertions and returned errors, zero-work/no-selection rejection, typed structured values and quoted numeric strings, relative imports/source attribution, autorun rejection, unresolved Given/input, unsupported predicates, clean environment/owned-root cleanup and a real 30-second deadline. Simple HTTP cases and the independently authored in-memory SQLite example run through the same actual CLI. Lower-level IAL tests additionally exercise FunctionCall → Context → numeric/equality/type checks.

The first complete gate exposed a stale negative fixture for quoted placeholder-shaped strings and the existing DD-078 whole-document checksum lock. The fixture now distinguishes quoted literal data from unquoted placeholders. The design checksum was refreshed only after inspecting the small approved status/ownership diff; historical acceptance/ownership tests remain intact. These were candidate issues, not claimed baseline failures. One interrupted incremental build also produced a local linker-cache error; the gate used `CARGO_INCREMENTAL=0` successfully.

## Initial reviewed candidate gate

Environment removed: `CARGO_TARGET_DIR NTNT_ENV APP_ENV NTNT_SECRETS_PROVIDER NTNT_LINT_MODE NTNT_STRICT NTNT_TYPE_MODE NTNT_OOB_MODE`.

Resource controls: `CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 RUST_MIN_STACK=8388608 NEXTEST_TEST_THREADS=2`. All binaries are worktree-local; no installed binary or production release build was used.

Commands, executed in order and all successful:

```sh
cargo fmt
cargo build --profile dev-release --locked
./target/dev-release/ntnt docs --generate
cargo build --locked
cargo nextest run --locked --no-fail-fast
cargo test --doc --locked
./target/dev-release/ntnt docs --validate
./target/dev-release/ntnt validate examples/
./target/dev-release/ntnt lint examples/
./target/dev-release/ntnt intent lint tests/fixtures/simple_server/server.intent
./target/dev-release/ntnt intent lint examples/ial_demo/server.intent
./target/dev-release/ntnt intent lint examples/native_testing/library.intent
./target/dev-release/ntnt intent check examples/native_testing/library.intent -vv
cargo fmt -- --check
git diff --check
```

- Nextest: **2,039 passed, 13 skipped**, 105.921 seconds. Includes all 16 actual native CLI regression tests and existing HTTP/Intent tests.
- Doctest command: successful; zero doctests defined (not additional executed coverage).
- Examples: 64 validated; lint reported zero errors/warnings and 151 suggestions. No unrelated lint cleanup.
- Three focused Intent lint files: zero errors/warnings.
- Checked-in native example: 3 scenarios, 10 assertions passed, including disposable SQLite.
- Generated documentation validation: successful; built before generation, then checked for diff whitespace/formatting.

## Bounded hosted follow-up

The first hosted run passed Linux, auth-backend contracts, lint and docs/examples. macOS and Windows both exposed the same test-only path assumption: imported sources are canonicalized, while the expected tempfile path used an alias (`/var` versus `/private/var`, or Windows short/verbatim paths). The fixture now compares the full canonical expected path; source attribution was not weakened to a basename check.

Hosted review also found two truthfulness defects: primitive recognition bypassed project result glossary terms, and focused selection reported unselected empty components as passing. Actual CLI regressions reproduced both. Explicit/parameterized/nested project definitions now retain priority, while unselected component definitions remain available for resolution but are not reported as executed.

After these scoped fixes, the full command block above ran again successfully: **2,041 tests passed, 13 skipped**, 110.131 seconds, including **18 actual CLI tests**. Doctest command, fresh generated docs, all 64 examples, lint, three Intent lint files, the 3-scenario/10-assertion native example and formatting/diff checks passed again. This is the final local candidate gate; no further reviewer fan-out or non-blocking cleanup was added.

Hosted Linux/macOS/Windows tests remain separate exact-head gates; the existing path-filtered release-profile socket job may legitimately skip for this slice. No unavailable local platform or production-release execution is claimed. No full root-suite composition, private-consumer migration, browser, managed external database, or governance capability is claimed.
