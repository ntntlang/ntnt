# DD-078 Intent Verification Runtime — Implementation Plan

> **For Hermes and ntnt contributors:** Execute in order with RED → GREEN → REFACTOR. Keep ntnt runtime changes and Larrimon migrations in separate PRs. Do not delete an old Larrimon check until the replacement has parity evidence from the same clean revision.

**Goal:** Deliver the DD-078 Intent Verification Runtime so a production application can declare durable behavior in `.intent`, implement all project-owned tests in `.tnt`, declare resources/profiles in `ntnt.toml`, and run the complete suite through `ntnt intent check .` without project-local Bash, Python, JavaScript test, or SQL-only test harnesses.

**Architecture:** Compile Intent into stable obligations; discover linked `.tnt` verification functions; plan an authority-checked resource DAG; execute cases in isolated interpreters through one action/observation model; supervise external resources; record one evidence ledger; render human, JSON, and JUnit outputs from that ledger. Specialist systems remain external resources behind typed, bounded providers.

**Baseline:** `origin/main` at plan authoring was `79c61dd98b0f10e3f6c1bce4f1d6e4df2343a21f` (ntnt 0.5.3). Rebase each implementation branch onto current `origin/main` before work. The audited DD-077 candidate is in `https://github.com/ntntlang/ntnt.git` at commit `f0132afcff984bb43305be39122d7e74a6850396`, document `design-docs/dd-077-correctness-primitives-roadmap.md`, Git blob `31a6d82f79e6051a7f00bfb182c979e5e78f2c3f`, on a separate unmerged lineage; it is not an ancestor of this plan. Only DD-077's literal owning sections/PR identifiers below are dependencies. Their owning design and implementations must merge, and their exact merge commits must replace the candidate SHA in this ledger, before any dependent DD-078 branch starts. No DD-078 fallback contract is permitted.

**Primary pressure test:** [`larimonious/larrimon`](https://github.com/larimonious/larrimon) at immutable commit `ceadfd992d1435ac27afb054968ff5569d697ce1`, recorded in [`dd-078-larrimon-baseline.md`](dd-078-larrimon-baseline.md), especially its seven `.intent` files, `tests/`, `ARCHITECTURE.md`, and `design-docs/dd-001-larrimon-mvp.md` §21.4/§22. Dirty-worktree bytes are not baseline evidence. Any changed base requires regenerating the canonical inventories and protected contract before deletion.

**Baseline gate note:** On the authoring host with Rust 1.94, `cargo nextest run` passed 1,962/1,962 tests (including the two-test DD-078 plan-consistency binary), but `cargo clippy --all-targets -- -D warnings` is already red on three unchanged `build.rs` lints (`collapsible_if` and two `manual_strip`). Before Task 1, either repair those baseline warnings in a separate narrow hygiene PR or pin/document the supported Rust toolchain that remains green. Do not bury baseline repair in the first verification feature diff.

---

## Delivery rules

1. Numbered tasks are portfolio epics. One focused ntnt PR lands per lettered slice in the dependency table below; no PR may silently combine slices.
2. Every behavior change starts with a failing Rust integration/unit test or fixture-app test.
3. Every new authority-bearing feature receives negative, timeout, cancellation, redaction, and cleanup tests in the same PR.
4. Existing compatibility paths remain green until their documented removal window.
5. No unrestricted shell execution, inline Intent SQL, ambient secret access, or unbounded provider output.
6. New public stdlib APIs require generated docs and examples.
7. New report/provider schemas carry explicit integer versions and committed fixtures.
8. Larrimon migration PRs consume pinned reviewed ntnt commits; they do not patch the runtime inside the app repository.
9. Independent review uses immutable commits, not moving worktrees.
10. A PR is not complete because focused tests pass; run the appropriate full ntnt gate and record actual output.
11. Every Larrimon deletion gate inherits the invariant-ledger and semantic mutation/fault-witness requirements from DD-078 §22; count/line parity alone is never sufficient.

### External prerequisite ledger

Every dependency beginning with `DD-` must name one exact row below. A design source is audit provenance, not a satisfied implementation prerequisite. Before a dependent branch starts, a docs truth-sync PR records the source design's merged commit/blob and every required implementation merge commit here; absent/unmerged rows remain hard blockers. A spike is a gate, never a production contract.

DD-077 source identity: repository `https://github.com/ntntlang/ntnt.git`, candidate commit `f0132afcff984bb43305be39122d7e74a6850396`, path `design-docs/dd-077-correctness-primitives-roadmap.md`, blob `31a6d82f79e6051a7f00bfb182c979e5e78f2c3f`. DD-047 source identity: the same repository, design commit `5a24c0cd1ff2f4d58e77ef263346cf6828cd28d6`, path `design-docs/dd-047-std-netmon.md`, blob `41b644195e2aaa81997f76631daa8bae5e5cb53c`.

| External owner | Required artifact | Depends on | Current status in this plan |
|---|---|---|---|
| DD-077 PR 0A | reusable native callback bridge with cleanup on every exit shape | merged DD-077 design | unmerged design only |
| DD-077 Design spike 0B | outbound transport binding feasibility note/fixtures; no production API | merged DD-077 design | unrun design gate |
| DD-077 PR 2C | trusted network configuration, internal capability core, shared target classification | DD-077 Design spike 0B | unimplemented |
| DD-077 PR 2D | policy-bound HTTP transport and public capability API | DD-077 PR 2C | unimplemented |
| DD-077 PR 2E | `std/net` integration with the same policy engine | DD-077 PR 2D | unimplemented |
| DD-077 PR 1B | `ntnt db status/plan/migrate/verify`, checksums, lock, lifecycle foundation | merged DD-077 design | unimplemented |
| DD-077 PR 1C | non-transactional migration hardening and dirty recovery | DD-077 PR 1B | unimplemented |
| DD-047 Slice 1C | canonical MIB catalog compiler/runtime registry, profiles, and finite plans from DD-047 §Implementation Plan | landed DD-047 design identity above | unimplemented; no implementation merge identity recorded |
| DD-047 PR 2 | device recognition and bounded inventory execution from DD-047 §Implementation Plan | DD-047 Slice 1C | unimplemented; no implementation merge identity recorded |

DD-077 Design spike 0C and PR 4B own static effect-metadata coverage/transitive analysis. They do not own runtime authority and are not DD-078 prerequisites: Slice 2G independently inventories and mediates every verification-mode sink with concrete grants. DD-077 defines no runtime clock/observation seam, so this plan assigns that generic internal seam exactly once to DD-078 Slices 10P/10B. DD-065 has no source design artifact in this baseline; production agent/tool execution therefore remains explicitly outside this plan rather than appearing as a satisfiable dependency.

### Dependency-closed DD-078 PR slices

| Slice | Scope | Depends on |
|---|---|---|
| 1A | status algebra, stable IDs, false-pass fixes | Task 0 |
| 1B | JSON/human report schema and exit parity | 1A |
| 2A | canonical project root, manifest, deterministic discovery | 1B |
| 2B | privileged host-policy authentication and ceiling intersection | 2A |
| 2C | protected evidence contract and base-ref scope continuity | 2B |
| 2D | exhaustive pure-authoring project-file classification | 2C |
| 2E | immutable input snapshot and launch identity | 2D |
| 2F | resource/profile planner and deterministic dry plan | 2E |
| 2G | concrete resource-grant substrate and verification authority enforcement | 2F |
| 3A | adapt verification invocation to landed DD-077 PR 0A; no new bridge | DD-077 PR 0A, 2G |
| 3B | test metadata and binding discovery only | 3A |
| 3C | fresh verification interpreter, opaque context, environment/registry isolation | 3B |
| 3D | typed assertions, redaction, snapshots, assertion evidence | 3C |
| 3E | fixture DAG and bounded teardown | 3D |
| 4 | seeded table/property generation and replay; no virtual clock | 3D |
| 5A | adapt verification HTTP to landed DD-077 PRs 2C–2E | DD-077 PR 2E, 2G |
| 5B | stateful test sessions/captures/assertions | 5A, 3D |
| 6A | cross-platform containment/readiness feasibility spike | 2E |
| 6B | process supervisor, host ledger, cleanup, attach mode | 5A, 6A, 3D |
| 7P | provider-protocol feasibility/adversarial framing spike; no public API | 2G, 6B |
| 7A | frozen stdio provider protocol and adversarial conformance fixture | 7P |
| 7B | strict scripted HTTP/HTTPS and webhook fixture | 7A |
| 7C | SMTP/mail capture fixture | 7A |
| 7D | TCP/UDP/DNS and byte-script fixture | 7A |
| 7E | root-confined temporary workspace fixture | 7A |
| 7F | deterministic typed AI/payment/API stub fixture | 7A |
| 8 | PostgreSQL lifecycle/assertions and migration evidence | DD-077 PR 1C, 7A |
| 9 | Redis disposable lifecycle plus queue/mail/webhook observations | 3E, 7B, 7C |
| 10A | eventual observations on monotonic deadlines | DD-077 PR 0A, 3D |
| 10P | runtime clock/observation inventory and feasibility spike; no public API | 3C |
| 10B | verification clock controls over the DD-078-owned internal runtime observation seam | 2G, 10A, 10P |
| 10C | stop/restart/readiness/fault lifecycle controls | 6B, 10A–10B |
| 11A | deterministic actors/barriers/race observations | 6B, 8, 10A–10B |
| 12P | Chromium/CDP containment/egress feasibility spike; no public API | 5A, 6A, 7P |
| 12A | sandboxed browser provider, provenance, containment, egress | 6B, 7A, 12P |
| 12B | typed browser sessions, DOM/network/screenshot and reconciliation API | 3D, 12A |
| 13A | extract reusable project inspection from `src/main.rs`/Studio/interpreter scanners | 2A |
| 13B | core ntnt AST/import/route/effect/project facts | 2G, 3D, 13A |
| 13C | Git and bounded JSON/YAML/TOML/XML/text facts | 2G, 13A |
| 13D | OCI/migration/runtime provenance facts and reusable read-only OCI client | DD-077 PR 1C, 7A, 8, 13A |
| 13E | first-class `Constraint` parser/binding after a dedicated syntax decision | 13B |
| 14A | imported evidence and canonical signed envelope | 1B, 7A |
| 14B | JUnit renderer, Studio, docs, editor migration | 14A |
| 14C | typed project-state, locks/leases, allocation transaction substrate | 2G, 6B |
| 14D | typed `ntnt project env` init/up/down/status OCI lifecycle, brokered daemon allocation/ingress, and effective-config validation | DD-077 PR 1C, 7A, 13D, 14C |
| 16M | Larrimon production migration/legacy-ledger/upgrade compatibility matrix | DD-077 PR 1C, 8, Task 16 DB conversion |
| 18P | streaming/event-source feasibility spike: NETCONF, HTTP/2/gRPC, TLS/syslog, flow control | 7A, 7D |
| 18A | typed streaming/event-source fixture/provider contracts | 18P, 7A |
| 18B | monitoring protocol, catalog, and inventory acceptance profiles | 18A, 13A, DD-047 Slice 1C, DD-047 PR 2 |
| 19A | KMS/secret-service and encrypted completion-spool fixtures | 7A, 7E, 10C, 14C |
| 19B | bounded load/backpressure provider and evidence schema | 6B, 7A, 10B, 11A |
| 19C | multi-agent, AI, alerting, retention acceptance profiles | 7F, 9, 19A, 19B |
| 20P | backup/restore and multi-node fault/topology feasibility spike | 14D, 19A, 19B |
| 20A | backup/PITR/restore provider and immutable recovery evidence | 20P, 7A |
| 20B | multi-node topology, fencing, partition, and outage provider | 20P, 7A, 14D |
| 20C | upgrade, HA, on-prem, BYO-KMS/private-AI acceptance profiles | 7F, 19A, 20A, 20B |

Each task below supplies the acceptance detail for exactly one slice. The table above is the sole dependency source of truth; every owner repeats its exact dependency cell as `Table dependencies`. Task 0's mechanical plan-consistency check rejects duplicate/unknown/cyclic dependencies, owner/table drift, external-ledger drift, release groups without transitive closure, production-bearing spikes, and missing/duplicate owners.

---

## Planned module layout

Create the new subsystem outside the current 7,500-line `src/intent.rs`:

```text
src/verification/
  mod.rs
  model.rs             # obligation/evidence/report domain types
  ids.rs               # stable IDs and validation
  discovery.rs         # Intent/test/project discovery
  manifest.rs          # ntnt.toml verification schema
  policy.rs            # host grants and ceiling intersection
  contract.rs          # operator-owned protected obligation/evidence baseline
  purity.rs            # mechanically proven pure-ntnt authoring closure
  snapshot.rs          # immutable content-addressed execution inputs
  planner.rs            # resource DAG and executable plan
  executor.rs           # case orchestration
  assertions.rs         # typed comparisons/diffs
  actions.rs            # action/observation traits/enums
  report.rs             # ledger aggregation and exit decision
  redact.rs             # recursive redaction/truncation
  supervisor.rs         # processes/resources/deadlines/cleanup
  provider/
    mod.rs
    protocol.rs
    process.rs
    http.rs
    postgres.rs
    fixtures.rs
    browser.rs

src/project_inspection.rs # shared root-confined project facts; available before provider namespace
```

The exact split may be adjusted to keep files coherent. Shared operational support lives outside verification in `src/project_state.rs` and `src/project_env/`; it still consumes the same canonical project, policy, provider, grant, and lifecycle-ledger contracts. Do not add new execution behavior to `src/main.rs`; CLI code should parse arguments and call library functions.

---

# Track A — Truth before power

## Task 0: Land DD-078 design only

**Files:**

- Add: `design-docs/dd-078-intent-verification-runtime.md`
- Add: `plans/dd-078-intent-verification-implementation.md`
- Add: `plans/dd-078-larrimon-baseline.md`
- Add: `tests/dd078_plan_tests.rs` (documentation/DAG consistency only)
- Modify: `design-docs/README.md`
- Modify: `design-docs/ial_vision_v2.md`

**Steps:**

1. Add DD-078 and this plan.
2. Mark the execution phase order in `ial_vision_v2.md` as superseded by DD-078; retain historical term-rewriting/Studio material.
3. Register DD-078 in the design-document index.
4. Add tests that parse the table and every owner, expand ranges, and reject duplicate/unknown/cyclic dependencies, owner/table drift, generalized external-ledger or task-owner drift, incomplete release closure including Larrimon/16M, missing parent-module registration or transitive parent-creator dependency, production-bearing feasibility spikes, fictional aliases, identity loss, and missing/duplicate owners; include negative mutation fixtures for representative failures.
5. Run Markdown/link checks, the focused DAG test, and `cargo fmt --check`; the only Rust change is non-runtime plan validation.
6. Obtain architecture, security, and implementation-plan review against the exact staged diff.

**Acceptance:** Design has explicit pure-project scope, authority model, provider boundary, Larrimon deletion gates, real DD-077 owner identifiers, report truth model, and mechanically valid implementation DAG. No runtime behavior changes.

---

## Task 1A: Stable obligation identity and truth model

**Table dependencies:** Task 0

**Create:** `src/verification/mod.rs`, `src/verification/model.rs`, `src/verification/ids.rs`, `tests/verification_truth_tests.rs`, and truth `.intent` fixtures.
**Modify:** `src/lib.rs` and `src/intent.rs` scenario/feature/outcome parsing.

**RED:** Reject duplicate/malformed feature/scenario/outcome IDs with source locations; report zero-outcome behavioral features as `unproven`; distinguish justified feature-level documentation-only declarations without allowing outcome-level suppression; and prove linked-but-unexecuted obligations have implementation coverage but zero executable/verified coverage.

**GREEN:** Add stable IDs and compatibility-derived IDs with warnings. Define `Obligation`, `EvidenceBinding`, orthogonal declaration/linkage/executability/disposition/freshness dimensions, source spans, and implementation/executable/verified coverage types. Unknown or unresolved assertions fail closed. No renderer, JSON schema, threshold, or exit-code behavior enters 1A.

**Gate:** focused parser/ID/model tests, full Intent parser tests, fmt/clippy, and immutable review.

---

## Task 1B: One evidence ledger, schema, rendering, and exit status

**Table dependencies:** 1A

**Create:** `src/verification/report.rs` and `tests/fixtures/verification/reports/schema-v1.json`.
**Modify:** `src/verification/mod.rs` registration, `src/main.rs` Intent check/coverage JSON and exit mapping, `docs/IAL_REFERENCE.md`, and `tests/intent_studio_tests.rs`.

**RED:** Cover unbound, unsupported, blocked, skipped, stale, failed, flaky, cancelled, no-result, and current-passed evidence; fail strict exit for every unmet required binding; retain diagnostic fail-then-pass history; qualify results by profile; require one evidence atom for every selected binding; keep advisory/excluded bindings visible but non-satisfying; reject fast-profile evidence as global/full pass; require banner-free schema-valid JSON; and prove human totals and exit status consume the same ledger. JUnit remains deferred to 14B.

**GREEN:** Define `EvidenceResult`, `CoverageSummary`, `RunReport`, schema/freshness/version fields, live-result compatibility conversion, centralized exit decisions, JSON/threshold flags, and one human renderer. Remove ad hoc summary arithmetic from `run_intent_check_command`.

**Gate:** `cargo test verification_truth`, full Intent and Studio tests, committed schema validation, fmt/clippy, and immutable review.

**Larrimon gate:** Run its current Intent files through the new static ledger and capture the exact inventory of declared, documentation-only, linked, unbound, executable, and verified obligations. Delete nothing.

---

## Task 2A: Canonical project root, manifest, and discovery

**Table dependencies:** 1B

**Create:** `src/project.rs`, `src/verification/discovery.rs`, `src/verification/manifest.rs`, `tests/verification_manifest_tests.rs`, canonical project fixtures, and `docs/verification-manifest.md`.
**Modify:** `src/lib.rs` and `src/verification/mod.rs` registrations, `src/config.rs`/shared manifest loading, and `src/stdlib/secrets.rs` only to reuse ancestor-root logic.

**RED/GREEN:** Discover nested Intent and configured verification files deterministically; define the versioned exhaustive file-class manifest; reject unknown fields, overlapping/unclassified classes, traversal, duplicate resources, ambiguous roots, build-output ambiguity, and symlink/hardlink escape. Consolidate existing root lookup without changing secret behavior. No policy, contract, purity verdict, snapshot, resource DAG, profile execution, or CLI plan enters 2A.

**Gate:** focused manifest/root/discovery tests, secret-root regression tests, schema docs, fmt/clippy, and immutable review.

---

## Task 2B: Shared `TrustedInput` and host-policy authentication

**Table dependencies:** 2A

**Create:** `src/verification/trusted_input.rs`, `src/verification/policy.rs`, policy/envelope fixtures, producer/consumer interoperability fixtures, and focused trust tests.
**Modify:** `src/verification/mod.rs` registrations, the operator-launcher integration, and report trust fields; repository CLI/env may only reduce authority.

**RED:** Prove requested capabilities cannot exceed external grants and broad labels cannot choose arbitrary executable/provider/image/argument/destination/mount/output. Default/untrusted PR policy cannot reach production secrets, private networks, OCI sockets, devices, privileged containers, arbitrary mounts, deployment credentials, spend, or public mutation. Repository files/argv/env/workflow/symlinks/same-CI-user paths cannot install privileged policy. Authenticate an inherited exact payload handle plus closed canonical `PolicyTrustedInputV1` envelope using domain `ntnt-policy-trusted-input-v1\0`: verify Ed25519/key/issuer/audience/repository/ref/workflow/validity/nonce and signed lowercase SHA-256 of exact raw payload before parsing. Reject duplicate/unknown fields, non-JCS envelope bytes, payload mutation, cross-type envelope, hardlink/non-regular/writable path or ancestor, owner/ACL failure, rename/TOCTOU swap, revoked key, wrong identity, and unknown algorithm.

**GREEN:** Implement the shared inherited-handle loader, exact policy envelope producer/consumer contract, policy parser, ceiling intersection, and raw/canonical digest plus trust-class reporting. No protected-contract semantics enter 2B.

**Gate:** trust interoperability/adversarial fixtures on supported platforms, focused policy tests, full security-sensitive review, fmt/clippy, and immutable review.

---

## Task 2C: Authenticated protected contract and base continuity

**Table dependencies:** 2B

**Create:** `src/verification/contract.rs`, contract/envelope/base fixtures, and focused contract tests.
**Modify:** `src/verification/mod.rs` registration and report claim-scope/input-identity fields only; planner consumption begins in 2F after `planner.rs` exists.

**RED:** Load the contract through the same inherited-handle `TrustedInput` machinery and closed `ProtectedContractTrustedInputV1` envelope using domain `ntnt-protected-contract-trusted-input-v1\0`; apply identical exact raw-digest, file identity, owner/ACL/ancestor, hardlink/symlink, pre/post-open, signature/key/validity/identity checks. Resolve base ref in the trusted launcher to immutable repository ID plus full commit/tree OIDs. Reject cross-type envelopes, raw/canonical digest swaps, mutable-ref substitution, wrong repository/ref/workflow, rename/hardlink/mid-run replacement, contract/base/inventory retargeting, deleted/renamed obligations, weakened globs/profiles/evidence/file classes, forbidden deltas, and count drops.

**GREEN:** Parse the authenticated contract, derive canonical semantic digest only after raw signature verification, compare the immutable base, bind raw/canonical/base/inventory identity into the plan, and emit only `project-authored-claim` or `protected-contract-execution-claim`. No project purity scan or checkout snapshot enters 2C.

**Gate:** producer/consumer and adversarial trust fixtures, base-continuity tests, report-schema checks, fmt/clippy, and immutable review.

---

## Task 2D: Exhaustive pure-ntnt classification

**Table dependencies:** 2C

**Create:** `src/verification/purity.rs`, executable-bearing metadata parsers, operator exclusion-lock fixtures, and adversarial purity projects.
**Modify:** `src/verification/mod.rs` registration, discovery inventory, and report purity fields.

**RED/GREEN:** Require every tracked path exactly once in the protected classes; reject omissions/overlap, relevant untracked executables, shebangs, extensionless/renamed wrappers, symlink/hardlink escape, non-ntnt helpers, legacy CLI/file and generic shell/process/provider routes, SQL/browser harnesses, inline workflow/package/Compose/Docker execution, YAML block scripts/heredocs/substitution/operators, unpinned actions/images, arbitrary containers, unknown executable-bearing formats, Git mode `160000`, nested repositories, and unclassified generated executable closure. Check import/build graphs, provider origins, generated outputs, argv, and file identity. External exceptions require an operator origin/digest lock recursively pinning every committed object; project-generated support is never exempt. Emit the complete inventory and `proven|not_checked|violated`.

**Gate:** all adversarial purity fixtures, deterministic inventory schema, fmt/clippy, and immutable review.

---

## Task 2E: Immutable execution snapshot

**Table dependencies:** 2D

**Create:** `src/verification/snapshot.rs`, concurrent-mutation fixtures, and focused snapshot tests.
**Modify:** `src/verification/mod.rs` registration, report input-identity fields, and launch/open-handle adapters.

**RED/GREEN:** Capture source, Intent, verifier, fixture, migration, manifest, inventory, lockfile, provider inputs, policy, and raw contract bytes once into a private content-addressed read-only closure. Bind immutable repository/subject/ref/workflow/run/trust, policy raw/canonical digest, contract raw/canonical digest, base repository/ref/commit/tree, protected inventory, and every captured digest. Fail rename/hardlink/mid-run swaps, mutable base replacement, path/executable replacement, pre-capture races, checkout drift, wrong repository/ref/workflow, contract retargeting, and cross-identity replay. Execute only captured bytes and prove deterministic hashes under concurrent checkout mutation.

**Gate:** focused snapshot/race/replay tests on supported filesystems, report-schema checks, fmt/clippy, and immutable review.

---

## Task 2F: Resource/profile planner and non-authoritative Intent bindings

**Table dependencies:** 2E

**Create:** `src/verification/planner.rs`, planner fixtures, and `tests/verification_planner_tests.rs`.
**Modify:** `src/verification/mod.rs` registration, `src/main.rs` directory input/`intent plan`/profile arguments, and IAL/agent docs.

**RED/GREEN:** Dry planning executes no project code/provider/process/network/secret lookup and has stable ordering/hash. Reject resource/profile cycles, unknown dependencies, unsupported platform/provider, and invalid limit/containment/readiness. Compute required/advisory/excluded bindings before startup; narrower profiles cannot claim broader verification. Intent cannot choose destinations/providers/resources/executables/paths/secrets/headers/arbitrary URLs or legacy CLI/file actions; auto HTTP is app-relative only. Strict fast/full cannot weaken truth, purity, contract, or host clamps; diagnostic mode is non-verifying. Implement the resource DAG and `ntnt intent plan . --profile NAME --json` without starting resources.

**Gate:** planner/manifest/secret tests, no-execution proof, CLI schema/docs, fmt/clippy, and immutable review.

**Larrimon gate:** Add a non-executing draft `[verification]` section in a separate Larrimon branch and prove the full resource/test inventory plans without startup. Do not merge it until the consuming runtime PR is pinned.

---

## Task 2G: Runtime effect inventory and concrete verification grants

**Table dependencies:** 2F
**Boundary:** DD-077 Design spike 0C/PR 4B may later supply static `EffectKind` metadata, but it is not runtime authority or a prerequisite. DD-078 `VerificationGrant` is concrete runtime authorization and Slice 2G independently inventories every mediated sink.

**Create:**

- `src/verification/runtime_authority.rs`
- `src/verification/grant.rs`
- `tests/verification_runtime_authority_tests.rs`
- a committed machine-readable inventory fixture for native/server actions and effect classes

**Modify:**

- `src/interpreter.rs` (`Value::NativeFunction`, `RuntimeCapability`, server-action dispatch)
- every `src/stdlib/*.rs` native registration as required by the compile-enforced metadata field
- `src/stdlib/mod.rs`
- `src/verification/mod.rs`
- `src/verification/policy.rs`, `planner.rs`, `report.rs`

**RED:**

1. Inventory every native/server action plus direct environment/cwd/args/filesystem/clock/random/network read and mutable `OnceLock`/`LazyLock`/registry reached during interpreter construction, module loading, or execution. Classify semantic effects; fail the inventory test when a new entry lacks classification.
2. Preserve existing behavior in Normal/Worker/Job/HotReload/UnitTest modes.
3. In Verification mode, deny every unassigned authority-bearing effect with a structured error rather than silent `Unit`.
4. Prove pure calls remain available without authority; clock/random sinks require exact case grants and record real observations, while virtual controls remain unavailable until Slice 10B.
5. Prove imports, aliases, UFCS/method bridges, prelude exposure, and user-function indirection cannot bypass the check.
6. Prove direct/transitive imports and module initializers are checked before effects; importing `std/env`, `std/fs`, `std/http`, database, job, auth, process, or secret modules does not gain authority.
7. Overlay cwd/args/environment per interpreter; prove process environment mutation and dotenv loading cannot escape the case.
8. Inventory process-global auth/job/database/HTTP/cache/time/random registries; namespace/reset them or force process isolation before the first native-case release.
9. Prove dynamic/unknown native calls fail closed in Verification mode.
10. Record requested/denied effect class and source location without leaking arguments.
11. Prove a broad effect class never grants authority. Every sink requires an opaque `{run, case, generation, resource, operation, scope, expiry, budget}` grant; database grant A cannot construct/connect to B, network resource A cannot reach another endpoint, and read authority cannot write.
12. Prove strings, maps, environment, captures, provider output, durable jobs, serialization, aliases, callbacks, or imported evidence cannot forge or widen a grant.
13. Run concurrent cases and runs against constructors, handles, global registries, and stale generations; prove no cross-resource, cross-case, or cross-run escape.
14. Install verification mode, overlays, grant table, and registry namespace before interpreter initialization or module evaluation can read `NTNT_MAX_RECURSION` or any host state.

**GREEN:**

1. Add compile-enforced internal semantic effect metadata to native/server actions.
2. Add supervisor-minted attenuating `VerificationGrant`/opaque handles plus per-interpreter cwd/args/environment overlay; do not authorize by `EffectKind` or mode-wide booleans.
3. Validate exact resource/operation/scope/generation/budget at the final sink shared by every invocation form and module initializer.
4. Namespace/reset process-global registries or mark their APIs unavailable until process-isolated.
5. Keep public effect-system syntax in DD-077; expose only the pinned adapter and concrete grant/reporting contract needed by DD-078.

**Verify:** full interpreter, stdlib, typechecker, language-feature, and verification policy tests; then full nextest because every native registration is touched mechanically.

**Larrimon gate:** none yet. This is the authority floor required before project verification code can execute safely.

---

# Track B — Native ntnt verification cases

## Task 3A: Adapt verification invocation to DD-077 PR 0A

**Table dependencies:** DD-077 PR 0A, 2G
**Invocation owner:** Adapt to the pinned callback/invocation bridge; do not extract, recreate, or special-case another bridge.

**Create:** `src/verification/invocation.rs` and `tests/verification_invocation_tests.rs`.
**Modify:** `src/verification/mod.rs` registration, the landed callback adapter, `src/interpreter.rs`, and `src/types.rs` only at the published integration seam.

**RED/GREEN:** Invoke an ordinary `fn(ctx)` through the shared native callback path with nested arrays/maps/options/results, structured errors, deadline, and cancellation; reject missing functions, invalid signatures, stale/forged invocation handles, duplicate terminal results, and stringification. Keep old function-call Intent tests through a compatibility adapter over this typed invocation. No metadata scanner, verification interpreter mode, assertion API, or fixture code enters 3A.

**Gate:** Focused callback/interpreter/type tests, full callback conformance, fmt/clippy, and immutable review.

---

## Task 3B: Discover and bind verification metadata

**Table dependencies:** 3A

**Create:** metadata fixtures and `tests/verification_discovery_tests.rs`.
**Modify:** Slice 2A's `src/verification/discovery.rs`, parser/AST comment metadata path or annotation scanner, plus planner/report schemas.

**RED/GREEN:** Discover `@test`, `@verifies`, `@uses`, and `@tags` without executing project code. Reject duplicate IDs, unknown obligation/resource IDs, missing functions, invalid signatures, annotations on the wrong declaration, unlinked strict cases, ambiguous obligation defaults, and nondeterministic tag/profile selection. Produce immutable typed `CaseBinding` records only; no interpreter startup or assertions enter 3B.

**Gate:** Focused discovery/planner/schema tests, static no-execution proof, fmt/clippy, and immutable review.

---

## Task 3C: Execute isolated cases under concrete grants

**Table dependencies:** 3B

**Create:** `src/verification/executor.rs`, `src/verification/redact.rs`, `tests/verification_case_tests.rs`, and case fixtures.
**Modify:** `src/verification/mod.rs` registration, `src/interpreter.rs`, `src/types.rs`, stdlib registration, planner/report, and typechecker.

**RED/GREEN:** Add `ExecutionMode::Verification` and an opaque generation-bound `TestContext`; install environment/cwd/args overlays and the case's concrete grants before interpreter/module initialization. Prove a fresh interpreter/module environment, deterministic seed, bounded output/artifacts, timeout/cancellation, recursive redaction, and reset on every exit. Module globals, imports, deferred state, mutable values, registries, and contexts cannot bleed across cases/runs. Source initializers pass through the same final authority seam and fail before unassigned effects; production source and ordinary `ntnt run` cannot import `std/test`, and application imports cannot reach verification files. Every effectful stdlib sink denies unassigned network/database/secret/environment/filesystem/job/process authority with a structured failure. No assertion vocabulary or fixture DAG enters 3C.

**Gate:** Focused interpreter/isolation/authority tests, full nextest because registration changes mechanically, fmt/clippy, and immutable review.

---

## Task 3D: Typed assertions and assertion-level evidence

**Table dependencies:** 3C

**Create:** `src/verification/assertions.rs`, `src/stdlib/test.rs`, `tests/verification_assertion_tests.rs`, and assertion fixtures.
**Modify:** `src/verification/mod.rs` and `src/stdlib/mod.rs` registrations, executor/report, `src/intent.rs` compatibility result path, generated stdlib docs, and typechecker.

**RED/GREEN:** Record nested typed observations without stringification; implement structural diffs, error/exit expectations, approximate numbers, order/count/unique/path/contains/regex, subcase labels, and snapshots. Multiple failed expectations accumulate while fatal runtime errors stop the case. A successful zero-assertion function yields `no-result`; multi-obligation cases require assertion-level IDs and every candidate obligation needs an evidence atom. Reject unsupported assertions, stale contexts, obvious literal vacuity, secret/tainted snapshots, and bounds violations. Ordinary/CI runs cannot update goldens. An explicit update command writes only a restrictive private candidate plus generated patch bound to source snapshot, target/prior identity/digest/mode, and proposed digest; ntnt never overwrites the committed target. Human/VCS apply and a fresh verification run are mandatory. Delete pass-on-unsupported branches from `run_function_call_test` and render only the shared ledger.

**Gate:** `cargo test verification_assertion`, case/function compatibility tests, docs validation, full applicable nextest, fmt/clippy, and immutable review.

**Larrimon deletion gate A1:** Convert representative reducer, validation, probe-shape, and application-service files to discovered `.tnt` cases. Run old and new together. Once all 18 current direct `ntnt run` cases have parity, remove their manual print/pass conventions and the 18-run loop from `tests/intent.sh`.

---

## Task 3E: Typed project fixture DAG and teardown

**Table dependencies:** 3D

**Create:**

- `src/verification/fixtures.rs`
- `tests/verification_project_fixture_tests.rs`
- `tests/fixtures/verification/fixtures/*.tnt`

**Modify:**

- `src/verification/discovery.rs`, `planner.rs`, `executor.rs`, `report.rs`
- `src/verification/mod.rs` registration
- `src/stdlib/test.rs`
- metadata/annotation discovery from Task 3
- generated stdlib/verification docs

**RED:**

1. Discover `@fixture`, `@scope`, `@teardown`, and test `@fixtures` metadata.
2. Reject duplicate IDs, unknown fixtures/resources, fixture cycles, invalid signatures, and unsupported scopes before resource startup.
3. Return nested typed values and preserve opaque/secret taint.
4. Block dependent cases on setup failure without satisfying obligations.
5. Run teardown in reverse dependency order after pass, failed expectation, runtime error, timeout, and cancellation.
6. Report teardown failure separately and fail strict mode without hiding the original case failure.
7. Default to case scope; reject shared mutable fixture parallelism without explicit reset and scheduling semantics.
8. Prove stale fixture/context values cannot cross cases, generations, or runs.

**GREEN:**

1. Implement case-scoped project fixtures and `fixture(ctx, id)` lookup.
2. Add fixture DAG planning alongside resource DAG planning.
3. Invoke optional teardown functions under their own bounded deadline.
4. Add suite/run scope only after reset declarations and scheduler serialization are enforced.

**Verify:** focused fixture tests, verification planner/case tests, docs generation, fmt, clippy.

**Larrimon gate A2:** Replace repeated scalar seed/setup builders used by pure `.tnt` cases. Resource-backed database/auth fixtures wait for Tasks 8–9.

---

## Task 4 / Slice 4: Table/property execution and deterministic test observations

**Table dependencies:** 3D

**Create:** `src/stdlib/test/generators.rs` if module organization permits, `tests/verification_property_tests.rs`, and shrinking/replay fixtures under `tests/fixtures/verification/properties/`.

**Modify:** `src/verification/mod.rs`, `executor.rs`, `assertions.rs`, and `report.rs`; `src/stdlib/mod.rs` and `src/stdlib/test.rs` submodule registration; and `src/intent.rs` test-data/corpus expansion.

**RED:**

1. Preserve nested typed table values instead of converting every input to string.
2. Record stable subcase IDs and source row/data labels.
3. Run bounded generated cases with a recorded seed.
4. Prove failure shrinking has case-count, time, depth, and output ceilings.
5. Prove exact seed/case replay.
6. Prove generator PRNG state is scoped to the case and reset afterward; do not claim wall-clock virtualization in this slice.
7. Reject shrinkable property cases that request resource/network/database/browser effects; allow such data sets only as ordinary named subcases until a provider defines transactional reset semantics.

**GREEN:**

1. Route existing Intent `test_data` and generated corpus through typed case parameters.
2. Add deterministic generator/replay infrastructure; do not imply broad QuickCheck semantics until implemented.
3. Add seeded test-generator APIs only in verification mode. Clock APIs wait for Slice 10B's runtime observation seam.
4. Report original and minimized failures.

**Verify:** focused tests, full Intent tests, docs generation, clippy.

**Larrimon gate:** Move large validation matrices and reducer golden streams into typed data fixtures without growing hand-written assertion helpers.

---

# Track C — Shared actions and application lifecycle

## Task 5A: Shared HTTP action and policy-bound transport adaptation

**Table dependencies:** DD-077 PR 2E, 2G

**Create:** `src/verification/actions.rs`, `src/verification/provider/mod.rs`, `src/verification/provider/http.rs`, and focused shared-transport tests.
**Modify:** `src/verification/mod.rs` registration, IAL execute/primitives/mod, Intent `WhenAction`/compatibility execution, direct `ntnt test` compatibility, and the landed `src/stdlib/http.rs`/`src/stdlib/net/policy.rs` transport seam.

**RED/GREEN:** Define `HttpAction`, `HttpObservation`, and structured transport errors; send custom headers plus JSON/form/raw/multipart/query payloads; preserve repeated headers and binary/chunked/compressed bodies; enforce connect/request/body/redirect/total limits and cancellation; prove production HTTP, net classification, IAL compatibility, and verification consume DD-077 PRs 2C–2E's identical all-address resolution/binding, proxy, mapped/private/metadata denial, rebinding, redirect/reconnect, TLS, credential-stripping, and deadline semantics. Auto-compiled Intent is app-relative and cannot select destination/provider/resource/secret headers. Adapt legacy IAL/live Intent to this one action and remove bespoke raw-TCP HTTP only after compatibility. No cookie jar, capture store, `std/test/http`, or assertion evidence enters 5A.

**Gate:** shared transport/policy/IAL/Intent compatibility tests, network security review, fmt/clippy, and immutable review.

---

## Task 5B: Stateful verification HTTP sessions, captures, and assertions

**Table dependencies:** 3D, 5A

**Create:** `src/stdlib/test/http.rs`, verification session/capture stores, `tests/verification_http_tests.rs`, and HTTP app fixtures.
**Modify:** `src/stdlib/test.rs` submodule registration plus executor/assertions/redaction/report and generated stdlib docs.

**RED/GREEN:** Maintain independent named cookie jars; preserve multiple `Set-Cookie`; capture approved header/cookie/JSON/regex/URL values; support bounded redirect opt-in; prove no cross-session leakage; recursively redact Authorization/Cookie/Set-Cookie/query/body; turn token/magic-link captures into opaque tainted values usable by later approved actions but impossible to stringify/snapshot/attach/emit; and emit typed assertion-level HTTP evidence. Reuse only 5A transport and policy—no parallel client or destination logic.

**Gate:** focused session/capture/taint/assertion tests, full Intent HTTP and Studio compatibility tests, docs, fmt/clippy, and immutable review.

**Larrimon deletion gate B1:** Migrate public health, headers, origin/HTMX, form fallback, auth request/consume, cookie, role, and multiple-identity cases. Keep server lifecycle in the old harness until Task 6.

---

## Task 6A: Cross-platform containment and readiness feasibility spike

**Table dependencies:** 2E
**Artifact:** `plans/dd078-process-containment-spike.md`; no public API or production supervisor

Adversarially prove the implementable guarantees and unsupported-platform behavior for Linux namespace/rootless-OCI+cgroup/seccomp containment, Windows Job Object/AppContainer, and an approved macOS boundary. Include descendant escape, daemonization, stable process identity, CPU/memory/PID/file/socket/disk enforcement, private scratch/HOME, read-only inputs, egress brokerage, inherited listeners, authenticated readiness, and the exact suspended/pre-owned `reserve → create → finalize → resume/expose` process protocol. Crash the controller and durable broker at every transition and identify which OS ownership primitive closes each gap. The note records exact kernel/API/dependency choices and a platform matrix. A failed spike revises 6B's containment/cleanup classes or blocks the platform; lifecycle primitives alone never become a sandbox or strict-cleanup claim.

**Gate:** Independent review approves the immutable spike artifact before 6B begins.

---

## Task 6B: Contained process supervisor, attach mode, and cleanup

**Table dependencies:** 3D, 5A, 6A

**Create:**

- `src/verification/supervisor.rs`
- `src/verification/lifecycle_broker.rs`
- `src/bin/ntnt-verification-broker.rs` (ntnt-installed/internal; never repository-selected)
- `src/verification/provider/process.rs`
- `src/stdlib/test/process.rs`
- `tests/verification_process_tests.rs`
- helper binaries/fixtures under `tests/fixtures/verification/processes/`

**Modify:**

- `src/main.rs` current server spawn/readiness/kill code
- `src/verification/provider/mod.rs` and `src/stdlib/test.rs` submodule registrations
- `src/verification/mod.rs`, `planner.rs`, `executor.rs`, `report.rs`
- `Cargo.toml` for a small cross-platform process-group dependency only if necessary
- `docs/verification-manifest.md`

**RED:**

1. Start with clean env and prove undeclared ambient values are absent.
2. Capture bounded stdout/stderr and include the tail on readiness/exit failure.
3. Support TCP, HTTP, process-alive, and provider-defined readiness under one deadline.
4. Treat early exit as failure unless expected; support expected startup rejection and exact error assertions.
5. Stop/restart and observe generation changes.
6. Kill process trees on pass, assertion failure, interpreter error, timeout, Ctrl-C, and provider failure.
8. Persist and fsync an authenticated reservation before process creation; launch suspended or into a broker-owned cgroup/Job Object, record pidfd/start/executable/token creation receipt, finalize the ledger, and resume/expose only afterward. Crash at every boundary and recover only by exact reservation/object identity.
9. Prove cleanup failure changes strict exit status but does not hide the original test failure.
10. Prove `--base-url` attach mode starts no app process and never claims ownership of the external service.
11. Pass a run-scoped child policy to managed ntnt children and prove stdlib capabilities remain enforced; record containment level and prove plain process mode never claims OS sandboxing for arbitrary executables.
12. Prove untrusted-PR profiles pass no sensitive inputs to child processes and reject non-ntnt executables unless their identity/trust or sandbox boundary is explicitly granted.
13. Prove managed verification disables implicit dotenv loading and denies project `.env`/credential files unless host policy explicitly grants a named file; an empty process environment alone is not accepted as hermeticity.
14. Enforce CPU, memory, PID/thread, file-descriptor, disk/temp, socket, process-launch, and aggregate connection limits below project code; prove exact-limit and limit-plus-one behavior.
15. Prove daemonized/`setsid` descendants cannot escape a profile that claims process-tree cleanup; otherwise block that profile.
16. Use inherited/reserved listeners or a run-nonce-bound readiness channel; prove an unrelated process cannot win a port race and fabricate readiness.
17. Linux stale-process ownership uses pidfd and/or run-owned cgroup/subreaper plus PID start time, executable identity, and run token; Windows uses kill-on-close Job Objects/process identity; macOS either proves equivalent identity or refuses stale process cleanup.
18. Prove parent/broker crash, crash-before-create, crash-after-create-before-finalize, daemonized descendant, PID/PGID reuse, partial/corrupt ledger write, writable ledger path, concurrent runs, and identity mismatch never leak a strict resource or kill an unrelated process. Platforms without a closed crash window are reported non-verifying and rejected in protected profiles.
19. Authenticate broker binary/config/state and inherited or mutually authenticated local IPC outside repository control; reject repository-selected endpoints, state roots, policy, cleanup authority, or broker identity. Protected profiles fail before startup when no durable broker class exists.

**GREEN:**

1. Implement the durable `reserve → create → finalize → expose` process lifecycle state machine, exact-receipt recovery, and reverse-order teardown.
2. Move current Intent app startup into a process resource.
3. Add exact argv, working directory, env allowlist, dynamic port allocation, readiness, expected exit, restart, and logs.
4. Add attach mode.
5. Add `intent doctor` diagnostics for executable, port, provider, and containment prerequisites.
6. Persist each reservation/finalization atomically with restrictive permissions outside the repository; implement exact OS-identity/token validation and bounded TTL recovery without PID/name/prefix scans.

**REFACTOR:** Remove null stdout/stderr spawning and fixed-port assumptions from `run_intent_check_command`.

**Larrimon deletion gate B2:** Move server/config/startup-failure and authenticated HTTP suites to manifest-managed app resources. Remove equivalent process, port, wait, and curl helpers from shell.

---

# Track D — Providers and stateful resources

## Task 7P: Provider protocol feasibility spike

**Table dependencies:** 2G, 6B
**Artifact:** throwaway branch or `spikes/dd078-provider-protocol/`; no public protocol/API

Before Slice 7A, adversarially prove inherited stdin/stdout pipe handling on Linux/macOS/Windows; four-byte big-endian pre-allocation checks; strict UTF-8 JSON/schema/unknown-field behavior; one-request correlation or explicitly bounded multiplexing; EOF/trailing bytes; cancellation and heartbeat races; stdout protocol/stderr diagnostics separation; inherited handle closure; provider crash/hang/slow-drip/oversized frames; and child/process-tree cleanup. Record exact frame limits, state machine, and platform behavior. A failed spike changes the design before public implementation; it does not silently choose another transport.

---

## Task 7A: Frozen out-of-process provider protocol

**Table dependencies:** 7P

**Create:**

- `src/verification/provider/protocol.rs`
- `tests/verification_provider_protocol_tests.rs`
- malformed/crash/hang provider fixtures

**Modify:** Slice 5A's `src/verification/provider/mod.rs`, plus `policy.rs`, `planner.rs`, `supervisor.rs`, `report.rs`, `docs/verification-provider-protocol.md`, and `docs/verification-manifest.md`

**RED:** Handshake exact protocol/provider versions and capabilities; model `reserve/create/recover/finalize/expose/cleanup` with ownership token, deterministic creation identity, exact object ID, and signed/provider-authenticated creation receipt; reject expose before finalized ledger state. Crash the broker/provider/controller at every transition and require exact-token recovery or a non-verifying cleanup class. Reject oversize, malformed, unknown-field, duplicate/late/wrong-request-ID frames; cancel/kill hangs; reject capability escalation; bind handles to run/provider/generation; recursively redact diagnostics; enforce four-byte big-endian pre-allocation checks, strict UTF-8 JSON, protocol-only stdout, bounded stderr, deadlines, cancellation, heartbeat, inherited-handle closure, and process-tree cleanup on Linux/macOS/Windows. Classify providers as sandboxed or trusted-uncontained and prove protocol validation is not syscall confinement.

**GREEN:** Implement only the frozen framing/state machine, conformance fixture, provenance, deadlines, cancellation, heartbeat, and structured errors. No built-in service fixture lands in 7A.

---

## Task 7B: Strict scripted HTTP/HTTPS and webhook fixture

**Table dependencies:** 7A
**Create:** `src/verification/provider/http_fixture.rs`, `src/stdlib/test/http_fixture.rs`, and focused HTTP/webhook fixture tests.
**Modify:** `src/verification/provider/mod.rs` and `src/stdlib/test.rs` submodule registrations.

**RED/GREEN:** Add finite loopback-only request scripts, exact method/path/header/body matching, response/redirect/delay/disconnect scripts, generated ephemeral HTTPS identity, webhook signature/attempt capture, consumption counts, strict unexpected/unused-traffic failures, byte/deadline caps, taint/redaction, and bounded request evidence. No SMTP or generic TCP behavior enters this PR.

**Larrimon gate:** Replace redirect/resend HTTP mocks and webhook receiver programs after mutation parity.

---

## Task 7C: SMTP/mail capture fixture

**Table dependencies:** 7A
**Create:** `src/verification/provider/smtp_fixture.rs`, `src/stdlib/test/mail_fixture.rs`, and focused SMTP tests.
**Modify:** `src/verification/provider/mod.rs` and `src/stdlib/test.rs` submodule registrations.

**RED/GREEN:** Implement a finite loopback SMTP script/capture with envelope/header/body/attachment assertions, delayed/rejected/disconnected replies, message/byte/deadline limits, required-message consumption, and recursive secret redaction. No queue observation or HTTP fixture code enters this PR.

**Larrimon gate:** Replace the project-owned SMTP capture program after magic-link and alert-delivery mutation parity.

---

## Task 7D: TCP/UDP/DNS and byte-script fixture

**Table dependencies:** 7A
**Create:** `src/verification/provider/network_fixture.rs`, `src/stdlib/test/network_fixture.rs`, and focused packet/stream tests.
**Modify:** `src/verification/provider/mod.rs` and `src/stdlib/test.rs` submodule registrations.

**RED/GREEN:** Implement finite loopback TCP/UDP request/response scripts, strict binary caps, source identity, delay/hold/disconnect/malformed behavior, deterministic DNS answers where platform support permits, exact consumption, and request recording. Keep protocol-domain semantics in DD-047 or later typed adapters; this slice is bounded transport scripting, not a raw-network escape.

---

## Task 7E: Root-confined workspace fixture

**Table dependencies:** 7A
**Create:** `src/verification/provider/workspace_fixture.rs`, `src/stdlib/test/workspace.rs`, and focused filesystem tests.
**Modify:** `src/verification/provider/mod.rs` and `src/stdlib/test.rs` submodule registrations.

**RED/GREEN:** Return opaque workspace handles, copy only declared bounded fixture inputs, permit operations only beneath the private root, reject traversal/symlink/hardlink/device/FIFO escape, bound files/depth/bytes, and prove teardown on every exit path. Handles cannot become arbitrary host paths.

---

## Task 7F: Deterministic typed AI/payment/API stubs

**Table dependencies:** 7A
**Create:** `src/verification/provider/api_stub.rs`, `src/stdlib/test/api_stub.rs`, and focused typed-stub tests.
**Modify:** `src/verification/provider/mod.rs` and `src/stdlib/test.rs` submodule registrations.

**RED/GREEN:** Support only registered typed request/response schemas, deterministic finite scripts, usage/cost ceilings, exact call counts, delay/rate-limit/error/disconnect cases, taint/redaction, and unused/unexpected-call failures. Reject arbitrary command, arbitrary destination, or real-spend/public-mutation behavior.

---

## Task 8 / Slice 8: PostgreSQL verification provider and migration evidence

**Table dependencies:** DD-077 PR 1C, 7A

**Create:**

- `src/verification/provider/postgres.rs`
- `src/stdlib/test/postgres.rs`
- `tests/verification_postgres_tests.rs`
- PostgreSQL fixtures under `tests/fixtures/verification/postgres/`

**Modify:**

- `src/verification/provider/mod.rs` and `src/stdlib/test.rs` submodule registrations
- `src/verification/manifest.rs`, `planner.rs`, `supervisor.rs`, `redact.rs`
- `Cargo.toml` only for disposable-test infrastructure not already available
- `docs/verification-manifest.md`, generated stdlib docs
- pinned DD-077 migration contract docs/adapter only; this task does not own another runner

**RED:**

1. External server mode creates an isolated database/schema with unique run identity and committed seed visible to another connection.
2. Managed OCI mode is capability/policy-gated and labeled for cleanup; skip only when profile explicitly does not require it.
3. Apply migrations as migrator and run app/worker observations under distinct least-privilege roles.
4. Prove FORCE RLS with no tenant context and cross-tenant access denial.
5. Prove fixed `search_path`, narrow grants, and security-definer caller behavior.
6. Bound query rows/bytes/time, lock waits, and diagnostics.
7. Hold a transaction/lock as an actor and release/rollback deterministically.
8. Clean database/schema after pass, failure, timeout, cancellation, and stale-run reconciliation.
9. Preserve credentials/parameters as redacted opaque values.
10. Produce migration inventory/checksum/applied-state evidence only through the landed DD-077 interface; there is no fallback trait or alternate runner.
11. Pin external endpoint identity and generated name prefixes; prove repository-controlled URLs/names cannot make cleanup drop or mutate a pre-existing database/schema.
12. Exercise fresh install, idempotent rerun, every supported legacy ledger, legacy checksum backfill, pre-package unverifiable rows, unknown-ledger rejection before mutation, malformed/missing manifests, missing or mutated applied files, database-enforced checksum policy, and role configuration.
13. Prove per-migration transactional rollback, dirty-state recovery, concurrent migrators/advisory locks, cancellation, and retry semantics with injected failures.

**GREEN:**

1. Implement external PostgreSQL provider first.
2. Add database-per-run and schema-per-run; default app-backed suites to database-per-run.
3. Add role-aware bounded query/execute and held transaction actors.
4. Integrate migrations through one versioned interface.
5. Add optional managed OCI mode after the external mode is green.

**Larrimon deletion gate C1:** Migrate schema, migration, checksum, RLS, security-definer, immutability, tenant-isolation, rollback, and role cases. Commit a file/range-level old-to-new ledger for every migration invariant. Task 16 owns replacement/deletion of `tests/assertions.sql`, `tests/probe_run_state_fixture.sql`, and `tests/security_definer_tenant_case.sql` after same-revision positive, negative, race, cleanup, and mutation parity. Run old migration checks and Slice 16M on the same revision with injected failures; keep only `scripts/migrate.sh`, `scripts/migrate-prod.sh`, `scripts/check-migration-checksums.py`, and `tests/migrate_prod_integration.sh` until the landed production runner and supported legacy/upgrade matrix pass.

---

## Task 9 / Slice 9: Redis/queue/mail/resource observations and named fixtures

**Table dependencies:** 3E, 7B, 7C

**Create:**

- `src/verification/provider/redis.rs`, `src/stdlib/test/redis.rs`, and `src/verification/resource_observations.rs`
- queue observation adapters plus typed consumers of 7B webhook and 7C mail captures; do not recreate those fixtures
- `tests/verification_resource_fixture_tests.rs`

**Modify:** `src/verification/mod.rs`, `src/verification/provider/mod.rs`, `src/stdlib/test.rs`, `src/verification/manifest.rs`, `planner.rs`, `executor.rs`, `fixtures.rs`, `supervisor.rs`, and `report.rs` for submodule registration, exact lease ownership, reset, teardown, and cleanup dispositions; extend manifest/provider docs.

**RED:**

1. Require a disposable per-run Redis instance for strict, hermetic, protected, and cleanup-claiming profiles. It uses the durable reservation/creation-receipt/finalization protocol; crash at every transition is recoverable by exact object/token identity, and ordinary failure/cancellation leaves zero residual keys and revokes all credentials.
2. Permit attached Redis only in an explicitly trusted non-hermetic/non-verifying profile. An operator-created random-pattern ACL user, strict command allowlist, and mandatory bounded TTL reduce risk but do not imply exact key ownership or immediate cleanup; attached evidence cannot satisfy protected obligations.
3. Observe queue depth/job state through supported ntnt job APIs rather than Redis implementation strings where possible.
4. Consume 7B/7C typed webhook/mail captures and assert headers/body/signature/attempts without owning their lifecycle implementations.
5. Return named fixture values to test contexts with scope/reset semantics.
6. Reject mutable shared fixture parallelism without an explicit reset/serialization policy.
7. Prove secret-bearing payloads and supervisor/admin Redis credentials never enter app/test code or reports; stale cleanup acts on the exact disposable-instance ledger object, never keys discovered by logical DB or prefix scans.

**GREEN:** Implement disposable Redis lifecycle plus the minimum generic queue/mail/webhook observations required by Larrimon; attached Redis remains visibly non-verifying unless a future enforcing broker proves exact transactional key ownership and cleanup.

**Larrimon deletion gate C2:** Migrate magic-link email, queue wakeup/reconciliation, alert delivery, and resource cleanup cases.

---

# Track E — Time, eventual behavior, and races

## Task 10A: Bounded eventual observations on real monotonic deadlines

**Table dependencies:** DD-077 PR 0A, 3D

**Create:** `src/stdlib/test/eventually.rs` and `tests/verification_eventually_tests.rs`.
**Modify:** `src/stdlib/test.rs` submodule registration, executor/report, and generated docs.

**RED/GREEN:** Re-run each observation under one monotonic deadline without reusing stale values; report attempts, elapsed time, final typed observation, and terminal reason; cancel promptly; reject zero/negative/unbounded intervals. Use the landed callback bridge only. No virtual clock/random or lifecycle faults enter 10A.

**Gate:** focused eventual/cancellation/deadline tests, fmt/clippy, and immutable review.

---

## Task 10P: Runtime clock/observation feasibility and inventory spike

**Table dependencies:** 3C
**Artifact:** `plans/dd078-runtime-observation-spike.md`; no public API or production seam

Inventory every wall/monotonic clock, sleep, auth/job expiry, UUID/random, retry, scheduler, and runtime deadline site. Prototype one per-interpreter internal seam without changing production behavior; prove thread/process ownership, callback re-entry, and reset on success/error/cancellation. Record unsupported sites/platform behavior. A failed spike revises 10B; it never creates a provider fallback.

**Gate:** Independent review approves the immutable artifact before 10B.

---

## Task 10B: Verification clock/random controls over the proven internal seam

**Table dependencies:** 2G, 10A, 10P

**Create:** `src/runtime_observation.rs`, `tests/verification_clock_tests.rs`, and build-enforced inventory fixtures.
**Modify:** `src/lib.rs` registration, interpreter, and every exact time/random/expiry owner identified by 10P.

**RED/GREEN:** Route the approved inventory through one internal seam; fail coverage when a new owned site bypasses classification. Add verification-only clock/random controls over generation-bound concrete grants; prove case/run reset, no cross-interpreter or production influence, and monotonic-deadline safety. Managed-app control is separately token-bound, loopback-only, absent in production, and cannot alter an unbound process.

**Gate:** focused inventory/clock/random/isolation tests, applicable full nextest, fmt/clippy, and immutable review.

---

## Task 10C: Expected lifecycle failure and stop/restart observations

**Table dependencies:** 6B, 10A, 10B

**Create:** `tests/verification_lifecycle_tests.rs`.
**Modify:** executor, supervisor, report, and generated docs.

**RED/GREEN:** Verify expected process/provider/startup failures without converting unexpected failures into data; stop/restart exact owned dependencies and prove generation, readiness loss/recovery, cancellation, cleanup, and fault disposition.

**Gate:** focused lifecycle/crash/cleanup tests, full supervisor tests, fmt/clippy, and immutable review.

**Larrimon gate C3:** Replace sleeps/poll loops for queued/running/terminal runs, readiness, session revocation, alert delivery, and scheduler recovery.

---

## Task 11 / Slice 11A: Actors, barriers, and deterministic coordination

**Table dependencies:** 6B, 8, 10A, 10B

**Create:**

- `src/verification/coordination.rs`
- `src/stdlib/test/concurrency.rs`
- `tests/verification_coordination_tests.rs`
- deadlock/race fixtures

**Modify:**

- `src/verification/mod.rs`, `executor.rs`, `report.rs`
- `src/stdlib/test.rs` submodule registration
- interpreter callback/suspension integration only through existing general mechanisms
- generated docs

**RED:**

1. Start named actors, wait at barriers, release in a recorded order, and join under deadlines.
2. Reject duplicate actor/barrier names, wrong participant counts, cross-case handles, and release after expiry.
3. Diagnose actor stacks/last steps on deadlock without leaking values.
4. Cancel remaining actors after one fatal failure.
5. Coordinate held PostgreSQL locks and held mock responses.
6. Reproduce duplicate scheduling, claim/revoke linearization, terminal-write fencing, projection serialization, enqueue failure/reconciliation, and alert/outbox idempotency patterns.
7. Prove the report describes controlled seams and does not claim deterministic kernel/database scheduling.

**GREEN:** Implement bounded actor groups/barriers and provider hold/release integration.

**Larrimon deletion gate C4:** Port every background/FIFO/`xargs -P` race case. No sleep-based race may be deleted until the replacement forces the intended interleaving.

---

# Track F — Browser and project evidence

## Task 12P: Chromium/CDP feasibility spike

**Table dependencies:** 5A, 6A, 7P
**Artifact:** throwaway branch or `spikes/dd078-browser-provider/`; no public API

**Questions to prove:**

1. Launch a policy-pinned Chromium with isolated profile and connect over CDP.
2. Navigate, query DOM, click/fill, inspect focus/history, disable JavaScript, intercept/hold/abort requests, capture console/network failures, screenshot, and clean process/profile.
3. Run on hosted Linux and determine macOS/Windows executable/job-object differences.
4. Bound CDP messages, artifacts, page count, time, and script evaluation output.
5. Decide core module versus maintained external provider using dependency size, release coupling, and sandbox boundaries.
6. Prove an enforcement point beneath CDP: isolated network namespace/container or mandatory brokered proxy covers DNS, redirects, subresources, WebSockets, WebRTC, service workers, loopback/private/metadata destinations, and downloads. If unavailable, untrusted-PR browser planning blocks.

**Gate:** Do not start Slice 12A until the spike records exact dependency/API choice and cleanup/security review. A failed spike may choose a maintained external provider; it does not permit project-owned Playwright scripts as the final model.

---

## Task 12A: Frozen browser provider contract and containment

**Table dependencies:** 6B, 7A, 12P

**Create:**

- `src/verification/provider/browser.rs` or a separate pinned provider crate/repository
- `tests/verification_browser_provider_tests.rs`
- browser sandbox/network fixtures

**Modify:** `src/verification/provider/mod.rs` registration when 12P selects the in-tree provider; an external provider instead records its pinned crate/repository registration in this slice.

**RED:**

1. Launch only policy-pinned Chromium/provider identities with isolated profiles, bounded CDP messages/pages/time/output, and explicit containment class.
2. Enforce navigation, DNS, redirects, subresources, WebSockets/WebRTC, service workers, downloads, loopback/private/metadata access, and reconnect below CDP through the approved broker/namespace.
3. Reject `file:` URLs, reused profiles, extensions, arbitrary remote-debug targets, undeclared services, unapproved downloads, missing/unsupported containment, and mutable executable identity.
4. Prove browser/provider crash, hang, cancellation, daemon descendants, and stale cleanup remove only exact owned processes/profiles/artifacts.
5. Record executable/version/digest, provider identity, sandbox/egress guarantees, sensitive-artifact disposition, and cleanup. `trusted-uncontained` is non-hermetic and cannot satisfy protected CI or receive protected secrets.

**GREEN:** Land the supervised provider and security/provenance evidence only; no project-facing DOM API in this slice.

---

## Task 12B: `std/test/browser` sessions and assertions

**Table dependencies:** 3D, 12A

**Create:**

- `src/stdlib/test/browser.rs`
- `tests/verification_browser_tests.rs`
- fixture web app/pages
- `docs/verification-browser.md`

**Modify:** `src/stdlib/test.rs` submodule registration and generated stdlib docs.

**RED:**

1. Context isolation for two users; cookie/storage cleanup.
2. JavaScript and no-JavaScript modes.
3. Locator text/HTML/attribute/count/visibility/focus/accessibility assertions.
4. Click/fill/select/submit/keyboard/history behavior.
5. Request interception, delay, abort, offline, replacement, and navigation races.
6. Console errors/failed requests as configurable failures.
7. Bounded screenshot/trace artifacts, sensitive-artifact policy, selector masking, restrictive permissions, and an explicit test proving arbitrary pixels are never described as generically redacted.
8. Browser crash/hang/cancel cleanup.
9. Executable/version/digest provenance and policy rejection.
10. Explicit script evaluation bounds and disabled-by-policy mode.
11. Apply network policy to navigation and every subresource; deny `file:` URLs, reused profiles, extensions, arbitrary remote-debug targets, undeclared loopback services, and downloads outside the artifact directory.
12. Prove CDP interception is treated as evidence, not containment; trusted-uncontained browser mode is non-hermetic and cannot receive protected secrets or satisfy protected CI.

**GREEN:** Expose the typed API and evidence; keep provider internals unavailable to project code.

**Larrimon deletion gate D:** Rewrite reconciliation and staging browser smoke in `.tnt`; cover desktop/mobile, HTMX/full-page, no-JavaScript, focus, URL, abort/replacement, mutation ambiguity, and auth. Remove project test `.js`/`.mjs` after parity.

---

## Task 13A: Canonical root-confined project inspection

**Table dependencies:** 2A

**Create:** `src/project_inspection.rs`, canonical inspection tests, and tracked-file fixtures.
**Modify:** `src/lib.rs` registration plus current `src/main.rs`, Studio, and interpreter scanner consumers.

**RED/GREEN:** Extract one reusable inspection library; enumerate tracked paths/blob hashes under the canonical root; exclude protected private classes by default; reject symlink/hardlink escape; and return stable bounded facts without executing project code. No AST constraint, data parser, OCI client, or new Intent syntax enters 13A.

**Gate:** focused scanner parity/root-escape tests, existing inspector/Studio tests, fmt/clippy, and immutable review.

---

## Task 13B: Structural ntnt facts and declarative constraint engine

**Table dependencies:** 2G, 3D, 13A

**Create:** `src/stdlib/test/project.rs`, `src/verification/constraints.rs`, structural fact/constraint tests, and AST/import/route fixtures.
**Modify:** `src/verification/mod.rs` and `src/stdlib/test.rs` submodule registrations, generated stdlib docs, and AST/inspect APIs to expose stable import/function/route/annotation/effect/ownership facts.

**RED/GREEN:** Detect forbidden dependencies and architecture invariants structurally without implementation-string matching. Facts remain typed/bounded and consume concrete project-read grants. No Git/data/OCI provider or first-class Intent syntax enters 13B.

**Gate:** focused structural-fact/constraint tests, AST/parser regressions, fmt/clippy, and immutable review.

---

## Task 13C: Bounded Git and structured-data facts

**Table dependencies:** 2G, 13A

**Create:** `src/project_data.rs`, focused Git/data reader tests, and adversarial JSON/TOML/YAML/XML/text fixtures.
**Modify:** `src/lib.rs` registration, shared project-inspection facts, and docs.

**RED/GREEN:** Read local tracked/blob/ref/dirty facts without network; parse bounded data with parser/version provenance; mark textual evidence; enforce root/depth/count/byte limits; reject traversal, entities/includes, aliases/expansion, and secret-class reads. No OCI daemon access or syntax changes enter 13C.

**Gate:** focused parser/Git/root/bounds tests, dependency review, fmt/clippy, and immutable review.

---

## Task 13D: Read-only OCI, migration, generated-doc, and runtime provenance facts

**Table dependencies:** DD-077 PR 1C, 7A, 8, 13A

**Create:** `src/oci_inspection.rs`, focused read-only OCI client tests, and provenance/migration fixtures.
**Modify:** `src/lib.rs` registration, shared project-inspection facts, provider policy, report, and docs.

**RED/GREEN:** Inspect config/labels/platform/content digest through exact grants; integrate DD-077 migration inventory/checksums, generated-doc facts, and runtime/image provenance; reject mutation, untrusted daemon/network access, raw effective config, mutable identity, and secret output. No project environment lifecycle or first-class constraint syntax enters 13D.

**Gate:** fake/adversarial OCI tests, migration/provenance parity, security review, fmt/clippy, and immutable review.

---

## Task 13E: First-class Intent constraint syntax

**Table dependencies:** 13B

**Create:** failing parser/binding/diagnostic fixtures for a separately approved `Constraint` syntax.
**Modify:** parser/AST/Intent/Studio docs only after the syntax decision.

**RED/GREEN:** Add the smallest declarative binding layer over 13B facts; no provider/fact implementation enters this PR. Generic `.tnt` project tests remain available where new syntax is unjustified.

**Gate:** syntax decision, parser/binder diagnostics, docs, fmt/clippy, and immutable review.

**Larrimon deletion gate E1:** Port `architecture_cases.py`, CI policy, assets, runtime/image provenance, and Compose/project assertions. Dual-run the new migration-checksum facts through 13D, but retain `scripts/check-migration-checksums.py`; only Slice 16M may authorize its removal after Task 8 and the landed migration matrix pass. Compare exact findings against old Python on the same immutable revision, then remove only the architecture/CI/assets/runtime-image Python files with demonstrated parity. Staging-state files are owned only by Slice 14C's gate.

---

## Task 14A: Bounded provenance-checked evidence import

**Table dependencies:** 1B, 7A

**Create:** `src/verification/import.rs`, `tests/verification_import_tests.rs`, and committed evidence-envelope/key-policy/JUnit/TAP/JSON adversarial fixtures.
**Modify:** `src/verification/mod.rs` registration, report schema, and evidence docs.

**RED/GREEN:** Reject imported pass claims with missing/unknown schema, provider identity, obligation/assertion IDs, hashes, invocation provenance, or signatures; mark current-input mismatch stale. Preserve bounded/redacted failure diagnostics and reject XXE/DTD/XInclude/external resolution, oversized/deep input, traversal artifacts, duplicate claims, and archive expansion. Require either a current authenticated-supervisor record or exact closed `EvidenceEnvelopeV1` signed as `ntnt-evidence-v1\0 || JCS(envelope_without_signature)`; both carry repository/commit/ref/workflow/run/attempt/trust, contract raw/canonical/base/inventory, operation/profile/plan/policy, complete inputs/runtime/providers/environment, assertion results, artifacts/cleanup, timestamps/expiry/nonce. Reject unknown/missing/duplicate fields, tamper, artifact swaps, replay/skew, downgrade, revoked keys, mutable-environment mismatch, and every cross-repository/ref/workflow/contract/base/profile/plan/policy/environment/provider substitution.

**Gate:** focused import/parser/signature/replay tests, committed schema interoperability, security review, fmt/clippy, and immutable review.

---

## Task 14B: Deterministic JUnit, replay, and Studio ledger adapters

**Table dependencies:** 14A

**Create:** `src/verification/junit.rs`, `src/verification/replay.rs`, `tests/verification_replay_tests.rs`, and committed JUnit/report examples.
**Modify:** `src/verification/mod.rs` registrations, CLI replay/output arguments, and Studio server/UI.

**RED/GREEN:** Generate deterministic JUnit from the ledger; replay one selected case by rebuilding current hashes, snapshot, grants, and authority rather than trusting prior pass; show implementation/executable/verified separately; never convert warning/pending/skip to pass.

**Gate:** focused JUnit/replay/Studio schema tests, full report tests, fmt/clippy, and immutable review.

**Larrimon gate:** CI uploads one versioned JSON report and optional JUnit; no shell post-processing infers coverage or status.

---

## Task 14C: Typed project-state, lock/lease, and allocation substrate

**Table dependencies:** 2G, 6B

**Create:**

- `src/project_state.rs`
- `src/stdlib/project_environment.rs`
- `tests/project_state_tests.rs`
- committed state-schema and corruption fixtures

**Modify:** `src/lib.rs` and `src/stdlib/mod.rs` registrations, shared canonical project loader, runtime authority/grants, and generated stdlib docs

**RED:**

1. Key state outside the checkout by canonical root digest plus environment; reject ambient root/project override, foreign-root state, stale generation, and identity collisions.
2. Require restrictive state/runtime directories, exclusive cross-process locks, atomic temporary write plus file/directory fsync and replace, schema validation, compare-and-swap transitions, and idempotent recovery.
3. Reject symlink/hardlink/non-regular/writable-parent paths, partial/corrupt files, unknown fields/schema, unsafe legacy upgrade, and concurrent lost updates.
4. Model exact `uninitialized → reserved → creating → finalized → starting → ready|degraded → stopping → stopped|cleanup-failed` transitions with cancellation/crash dispositions.
5. Never probe-and-release managed-process ports: retain inherited listeners through binding. Expose provider-neutral global allocator locks, leases, compare-and-swap transitions, and opaque ownership records, but do not inspect or mutate OCI here.
6. Validate host-clamped port/subnet pool schemas, lock ordering, lease expiry, bounded retry state, overlap/exhaustion/IPv4/IPv6 rules, and concurrent-worktree serialization. A candidate lease is not reported as an external object until the provider's exact creation receipt is durably finalized; 14C itself never claims filesystem/provider atomicity.
7. Keep generated credentials/secret outputs opaque and restrictive; reports/status expose only safe names, allocation IDs, endpoints approved as non-secret, and dispositions.
8. Prove bounded stale-state reconciliation acts on exact ownership/object records, never root-derived names or prefixes.

**GREEN:** Implement the versioned state service and opaque typed `std/project/environment` handle used by native project-environment commands; do not expose generic key/value storage, arbitrary paths, process execution, or OCI commands.

**Larrimon deletion gate:** Port every state/transition invariant and mutation from `scripts/staging-state.py` and `tests/staging_state_cases.py`; preserve worktree identity, legacy-state upgrade, restrictive permissions, and foreign-state rejection. Do not delete either file until 14D also proves effective interpolation and atomic OCI port/subnet allocation parity.

---

## Task 14D: Typed development/staging OCI environment lifecycle

**Table dependencies:** DD-077 PR 1C, 7A, 13D, 14C

**Create:**

- `src/project_env/mod.rs`
- `src/project_env/manifest.rs`
- `src/project_env/oci.rs`
- `src/project_env/oci_allocation.rs`
- `src/project_env/report.rs`
- `tests/project_environment_tests.rs`
- fake/adversarial OCI provider fixtures
- `docs/project-environments.md`

**Modify:** `src/lib.rs` registration, `src/main.rs`, `src/config.rs`/shared manifest model, provider policy, and generated CLI docs

**RED:**

1. Parse strict `[project.environments.NAME]` declarations for pinned provider, Compose/OCI files, profiles, allowed services, build/create/up order, migration action, readiness, non-secret outputs, allocation pools, and cleanup policy; reject generic argv/shell hooks.
2. Implement `ntnt project env init|up|down|status NAME`; JSON status is schema-versioned and contains no secret values. Effective rendered configuration is parsed in memory, recursively redacted/tainted, bounded, and never emitted raw.
3. Render effective Compose/OCI configuration through the pinned provider and reject undeclared files/services, privileged/host modes, arbitrary mounts/devices/socket forwarding, mutable images where policy requires digests, ambient environment overrides, shell interpreters/operators, and project-owned shell/Python entrypoints or lifecycle hooks.
4. Execute dev/staging lifecycle as typed actions. A durable broker binds and retains each host TCP listener for the environment lifetime and proxies it to an unpublished final container port; Compose never rebinds that host port. Before exposure finalize `{listener ID, container object ID, network endpoint ID, generation, target port, ownership token}`. Every accept/reconnect re-inspects that exact target or uses an authenticated daemon event stream that invalidates fail-closed; service names, aliases, and cached IPs never authorize routing. Before forwarding application/readiness bytes, require a broker-controlled generation-bound target handshake using an opaque sidecar nonce or ephemeral mTLS identity unavailable to peers. Recreation/restart, event loss, or failed target authentication requires a new finalized generation and wrong/stale targets receive zero application bytes. UDP/non-proxy backends must use daemon allocation on the final service object with exact pre-exposure receipt/recovery or be rejected. Under the allocator transaction, create the exact external network, persist/finalize daemon object IDs, ownership token, and receipts, then let Compose consume that network. Continue with validate, build, final service create, start dependencies, landed DD-077 migration, app/workers, authenticated readiness through the broker, and state commit.
5. Use `reserve → create → finalize → expose` for network, service, and listener/proxy objects. On failure/cancellation/crash, retain truthful partial state and recover/clean only exact reserved identities, provider object IDs, and ownership tokens. `down` revalidates root, manifest, provider, generation, receipt, and object identity; names/labels/prefixes cannot nominate deletion. Unsupported crash windows are non-verifying and rejected in protected profiles.
6. Test concurrent worktrees, cross-user/shared-daemon runs, a non-cooperating daemon client, listener/network collisions, container recreation, bridge-IP reuse, alias collision/attachment, stale broker routes, stolen/stale target nonce and mTLS identity rejection, daemon restart, wrong-target zero-application-byte proof, crash before create/after create/before finalization/before exposure, repeated up/down, partial starts, migration failure/rollback, provider drift, occupied allocations, subnet overlap/exhaustion/IPv4/IPv6 mismatch, readiness spoof, stale state, and reboot recovery. The user-state lock is not treated as a daemon-wide mutex; OS binding and daemon object creation are authoritative.
7. Host policy separately grants OCI socket/build/network authority; untrusted PR execution without it is blocked before provider startup.

**GREEN:** Implement one typed OCI/Compose lifecycle provider, durable ingress broker, exact creation receipts/recovery, and the CLI. Dev/staging behavior is data in the project manifest; no placeholder-socket handoff, shell-wrapper provider, or arbitrary command escape exists.

**Larrimon deletion gate:** Replace `scripts/dev-up.sh`, `dev-down.sh`, `staging-up.sh`, and `staging-down.sh` only after same-revision positive, negative, partial-failure, cleanup, and mutation parity. `staging-smoke.sh` remains until its HTTP/browser evidence also migrates.

---

# Track G — Larrimon conversion and proving the product

## Task 15: Larrimon Wave A — Intent truth and native cases

**Repository:** Larrimon only
**Depends on ntnt:** Tasks 1–4

**Files:**

- Modify all seven current `.intent` files with stable scenario/outcome IDs and missing behavioral scenarios.
- Add/convert `.tnt` files under `verification/`.
- Modify `ntnt.toml` profiles.
- Modify CI to run old and new tiers together temporarily.
- Remove obsolete manual assertion/pass wrappers only after parity.

**Required additions:** Include currently under-covered wrong-tenant session, readiness dependency failure, internal dispatch denial, request-path no-probe, persistence rollback, concurrent projection serialization, enqueue failure reconciliation, and audit immutability contracts.

**Gate:** Every audited obligation is verified or explicitly documentation-only with rationale. `@implements` coverage is no longer presented as behavioral verification.

---

## Task 16: Larrimon Waves B–D — HTTP, database/jobs, browser

**Depends on ntnt:** Tasks 5–12B

Execute separate focused Larrimon PRs:

1. HTTP/auth/server conversion.
2. PostgreSQL/migration/RLS conversion.
3. Jobs/eventual/restart conversion.
4. Deterministic concurrency conversion.
5. Browser/reconciliation conversion.

For each PR:

1. Inventory old cases with stable IDs.
2. Add failing `.tnt` equivalents.
3. Run old/new on the same clean database/runtime/image.
4. Compare positive, negative, timing, cleanup, and race behavior.
5. Apply representative semantic mutations/faults and prove both old and new checks fail with equivalent useful detection evidence.
6. Delete only the replaced slice; every non-dual-runnable exception names the exact blocker and reviewer-approved alternative witness.
7. Update Intent bindings and architecture/testing docs.

**Gate:** No project-owned Node/Playwright test files or non-migration SQL-only test case files remain after Wave D. Production JavaScript and SQL migrations remain where required. Migration fixtures/support scripts remain until Slice 16M passes.

### Slice 16M: Larrimon production migration compatibility

**Table dependencies:** DD-077 PR 1C, 8, Task 16 DB conversion

Run the old migration suite and native `ntnt db`/`.tnt` evidence on one immutable revision across fresh install, idempotent rerun, every supported legacy ledger, checksum backfill, pre-package unverifiable rows, unknown-ledger rejection before mutation, malformed/missing manifests, missing/mutated applied files, database checksum enforcement, concurrent migrators/advisory locks, per-migration rollback, dirty recovery, cancellation, role configuration, and each supported old/new application-schema upgrade matrix. Inject failures/mutations for every family and retain paired reports.

**Gate:** Only this slice may authorize removal of `scripts/migrate.sh`, `scripts/migrate-prod.sh`, `scripts/check-migration-checksums.py`, and `tests/migrate_prod_integration.sh`. It owns no SQL-only application test fixture; those exact three files belong to Task 16. A later Task 20 may expand operational matrices, but the supported production matrix cannot be deferred past this deletion.

---

## Task 17: Larrimon Wave E — project policy and one-command CI

**Depends on ntnt:** Tasks 13–14, Slices 14C–14D and 16M, and prior Larrimon waves

**Files:**

- Convert Python/static/provenance checks to `.tnt`.
- Inventory every remaining project-owned `.sh`/`.py` outside the historical test tree; convert operational helpers to ordinary `.tnt` CLI programs or direct typed ntnt/provider commands. Any externally owned non-support artifact exclusion must come from the operator-controlled origin/digest lock; project-generated support cannot be retained.
- Replace suite shell wrappers with `ntnt.toml` profiles.
- Simplify `.github/workflows/ci.yml` to pinned host-installed/setup/verify actions that invoke named ntnt profiles through typed inputs; no inline `run:` script, shell block, or project wrapper.
- Update `README.md`, `ARCHITECTURE.md`, and testing docs.

**Audited replacement ledger:** The immutable path/range/line/blob inventory is [`dd-078-larrimon-baseline.md`](dd-078-larrimon-baseline.md), bound to Larrimon commit `ceadfd992d1435ac27afb054968ff5569d697ce1`. The grouped table below is a destination summary only. Before deletion, regenerate the baseline from the exact migration base and require its digest, protected-contract base, and execution snapshot base to match.

| Current project-owned files | Required native destination |
|---|---|
| `tests/{all,fast,intent,db}.sh` | `ntnt.toml` profiles and pinned typed CI action entries for `ntnt intent check` |
| `tests/integration.sh`, `tests/server-smoke.sh`, `scripts/staging-smoke.sh` | linked `.tnt` HTTP/DB/process/browser cases under the corresponding profile |
| `tests/migrate_prod_integration.sh` | Slice 16M only, after landed DD-077 PRs 1B–1C: native migration-runner compatibility, legacy/upgrade/failure matrix, and same-revision evidence |
| `scripts/{migrate,migrate-prod}.sh` | DD-077 `ntnt db` migration/apply/verify commands plus linked migration evidence |
| `scripts/{dev-up,dev-down,staging-up,staging-down}.sh` | Slices 14C–14D typed `ntnt project env` state/OCI lifecycle; never a generic native command or shell-wrapper provider |
| `scripts/staging-state.py`, `tests/staging_state_cases.py` | Slice 14C typed root-bound state/lock/lease substrate plus Slice 14D OCI allocation and `.tnt` cases |
| `scripts/check-migration-checksums.py` | Task 8 migration observations plus Slice 16M's landed DD-077 compatibility matrix; only 16M authorizes deletion |
| `tests/{architecture_cases,ci_cases,assets_provenance,runtime_provenance,runtime_image_provenance}.py` | `std/test/project`, Git/YAML/OCI/migration facts, and linked `.tnt` constraints |
| `tests/{smtp_mock,redirect_mock,resend_mock}.py` | Slices 7B–7C built-in SMTP and scripted HTTP fixture providers |
| `tests/reconciliation_cases.js`, `tests/staging-browser-smoke.mjs` | `std/test/browser` `.tnt` cases; production `public/larrimon.js` remains product code |
| `tests/{assertions,probe_run_state_fixture,security_definer_tenant_case}.sql` | Task 16 typed PostgreSQL assertions/seed fixtures and role/RLS/security-definer `.tnt` cases; deleted before Slice 16M |

Before deletion, expand this table to invariant-level rows with old line/range, stable invariant ID, replacement obligation/case IDs, resources/environment, positive result, deliberate mutation/fault witness, and retained report digest. The path inventory is a floor, not permission to batch-delete by filename count.

**Final commands:**

```bash
ntnt intent lint .
ntnt intent plan . --profile full --json
ntnt intent check . --profile fast
ntnt intent check . --profile full --report-json verification-report.json
```

Environment-backed gated profiles remain explicit:

```bash
# Operator-installed wrapper chooses fixed profile/policy/contract outside the checkout.
/usr/local/bin/ntnt-protected-verify live-public-network
/usr/local/bin/ntnt-protected-verify ha-game-day
```

**Final deletion gate:**

- zero project-owned `.sh` files; any externally owned artifact exclusion is operator-locked by origin/digest and is not verification/support;
- zero project-owned `.py` files under the same rule; project-generated support is never exempt;
- zero project-local browser/reconciliation test `.js`/`.mjs` files;
- zero SQL-only test case files;
- all suite orchestration represented by ntnt profiles;
- all required obligations current and verified;
- complete old-to-new invariant ledger retained in project docs/history.
- representative mutation/fault witnesses retained for every deleted invariant family.

---

# Track H — Future Larrimon proving fixtures

These are acceptance applications for the runtime, not reasons to add product-specific primitives.

## Task 18P: Streaming and event-source feasibility spike

**Table dependencies:** 7A, 7D
**Artifact:** `plans/dd078-streaming-event-source-spike.md`; no public provider API

Prove bounded cross-platform client/server fixtures for NETCONF framing, HTTP/2/gRPC streaming, TLS syslog/event sources, reconnect ordering, half-close/cancellation, flow-control, backpressure, slow-drip, certificate rotation, and byte/message/retention ceilings. Record library choices, containment/egress class, immutable endpoint identity, and unsupported-platform behavior. A failed spike revises 18A rather than exposing raw sockets or generic commands.

## Task 18A: Typed streaming/event-source fixture providers

**Table dependencies:** 7A, 18P

Land separate typed NETCONF, gNMI/HTTP2-stream, and TLS syslog/event-source adapters over the provider protocol. Each contract has finite scripts, explicit auth/identity, deadlines, flow-control/backpressure, deterministic failure injection, bounded evidence, and cleanup. It grants no arbitrary network destination or raw protocol escape.

## Task 18B: Monitoring protocol and inventory acceptance profiles

**Table dependencies:** 13A, 18A, DD-047 Slice 1C, DD-047 PR 2

Add pure `.tnt` verification for:

- SNMP GET/WALK strict BER, correlation, timeout, packet/result/byte bounds, opaque communities, counter wrap/reset/rate normalization, private/live gates;
- MIB compiler/catalog/profile/inventory expected-hash and rollback behavior;
- device recognition confidence/tie handling;
- finite inventory plans and normalized snapshots;
- NETCONF plus gRPC/HTTP2 gNMI and syslog/event-source auth, ordering, malformed input, reconnect, streaming subscription flow-control/backpressure, and bounded retention;
- gated real-device smoke with host policy and opaque secrets.

## Task 19A: KMS/secret-service and encrypted completion-spool fixtures

**Table dependencies:** 7A, 7E, 10C, 14C

Define typed, finite KMS/secret-service fixtures with opaque handles, key version/rotation/revocation, deny/outage/timeout behavior, nonce/AAD/envelope vectors, purpose/run/node binding, and proof app/agent code never receives KEK authority. Add an encrypted completion-spool fixture whose exact files/claims/idempotency records live in a root-confined workspace, survive supervised restart/failover, and expose bounded replay evidence without secret material.

## Task 19B: Bounded load/backpressure provider

**Table dependencies:** 6B, 7A, 10B, 11A

Define a contained provider with typed workload plans, fixed target handles, deterministic seeds, arrival/concurrency/byte/request ceilings, priority classes, cancellation, load-shed/fault controls, and bounded percentile/queue evidence. It cannot choose arbitrary destinations, execute project commands, or claim hermetic timing. Add exact-limit and limit-plus-one, overload ordering, backpressure, cleanup, and report-redaction tests.

## Task 19C: Multi-agent, AI, alerting, and retention acceptance profiles

**Table dependencies:** 7F, 9, 19A, 19B

Add pure `.tnt` verification for:

- signup/invite/Turnstile, independent identity/IP/purpose rate limits, verified-email membership creation, and transactional side-effect rollback;
- unified IPv4/IPv6 egress policy, metadata denial, DNS rebinding, per-hop redirect/reconnect validation, credential stripping, and private-node scope;
- AES-GCM nonce/AAD/envelope/rewrap vectors, write-only/expiry/purpose/run/node binding, KMS denial, and proof agents never receive KEK/KMS authority;
- multiple supervised application/agent protocol fixture processes, enrollment/signing/nonce/replay/rotation/revocation, wrong-node/cross-tenant denial, and encrypted completion-spool failover replay preserving run/claim/idempotency identity; these fixtures do not execute production tools;
- deterministic 7F AI responses with schema/citation/no-tool/token/plan assertions;
- 9/7B/7C email/webhook retries, signatures, deduplication, and ambiguous outcomes;
- clock-driven partition/retention/legal-hold cases and 19B bounded load/backpressure priorities.

Production agent/tool execution, typed effect transcripts, and tool-using cases are not owned by 19C and are excluded from this release sequence. They remain blocked until DD-065 has a real design artifact, exact owner/contract, implementation merge identities, and a later plan truth-sync adds that dependency.

## Task 20P: Backup/restore and multi-node topology/fault feasibility spike

**Table dependencies:** 14D, 19A, 19B
**Artifact:** `plans/dd078-ha-recovery-provider-spike.md`; no public provider API

Prove provider ownership/containment for disposable multi-node topologies, independent failure-domain representation, network partitions, clock/certificate/DNS/queue/KMS faults, fenced promotion, backup/PITR artifacts, and crash-safe teardown. Establish immutable environment/artifact identity, no production-target default, supported platform/OCI boundaries, measurable RPO/RTO semantics, and a non-hermetic report class. A failed spike blocks 20A/20B.

## Task 20A: Backup/PITR/restore provider and recovery evidence

**Table dependencies:** 7A, 20P

Implement typed backup, restore, point-in-time target, integrity validation, and rollback actions against disposable resources only. Bind source database/image/runtime, backup object digest, encryption/KMS identity, target time, restored schema/data checks, cleanup, and measured recovery to one immutable evidence record. Reject arbitrary paths/buckets/credentials and unsupported recovery claims.

## Task 20B: Multi-node topology, fencing, partition, and outage provider

**Table dependencies:** 7A, 14D, 20P

Implement an exact-owned disposable topology DAG with independent failure-domain labels, one-writer fencing, tenant home-region/data-residency assertions, network/queue/KMS/DNS/certificate/clock fault handles, encrypted completion-spool recovery, and bounded game-day timelines. Cleanup acts only on immutable provider object IDs; protected profiles block unless containment and topology claims are demonstrable.

## Task 20C: Upgrade, restore, HA, and on-prem acceptance profiles

**Table dependencies:** 7F, 19A, 20A, 20B

Use explicit non-hermetic provider-backed profiles for:

- old-to-new migration matrices, expand/migrate/contract compatibility, and rollback constraints across app/worker/agent versions;
- OCI/runtime/source provenance and signed artifacts;
- backup/PITR restore evidence;
- independent-failure-domain topology, fenced one-writer promotion, tenant home-region/data-residency, canonical queue/KMS outage behavior, and encrypted completion-spool recovery;
- overload ordering that preserves heartbeat/completion/reducer/alert work before probes and sheds discovery/AI first;
- measurable clock-skew, DNS, certificate, KMS, queue-loss, and partition recovery;
- private-network/device smoke under customer-controlled policy;
- customer-managed/BYO KMS and 7F private-AI matrices proving no hosted credential or private-evidence fallback;
- measured SLO/RPO/RTO/game-day evidence.

Reports bind exact runtime/provider/environment/artifact identities and label these profiles non-hermetic.

---

## Per-PR validation template

Use the smallest focused commands first, then the full applicable gate. Exact test filters will evolve with modules.

```bash
cargo fmt --check
cargo test <focused-filter>
cargo test --test <focused-integration-test>
cargo clippy --all-targets -- -D warnings
ntnt docs --validate
```

Before each ntnt runtime PR is considered ready:

```bash
env -u NTNT_TYPE_MODE \
    -u NTNT_NETMON_ENABLE \
    -u NTNT_NET_ALLOW_PRIVATE \
    CARGO_TARGET_DIR=target \
    RUST_MIN_STACK=8388608 \
    cargo nextest run

CARGO_TARGET_DIR=target RUST_MIN_STACK=8388608 cargo test --doc
```

If `cargo nextest` is unavailable, record that fact and run the complete `cargo test --all-targets` fallback. Never report a synthetic pass.

Security-sensitive provider/process/network/browser PRs additionally require:

- Linux hosted CI;
- macOS and Windows hosted CI where the capability claims support them;
- exact immutable diff review;
- secret canary scan of stdout, stderr, JSON, JUnit, textual artifacts, screenshots metadata, and failure messages, plus sensitive-artifact handling review for binary/browser artifacts;
- cancellation/timeout/cleanup test evidence;
- dependency/provenance review for new crates or provider binaries.

---

## Release sequencing

Do not put this portfolio into a patch release. Recommended feature sequence:

| Candidate feature boundary | Minimum content |
|---|---|
| v0.6.0 foundation | Landed DD-077 PR 0A plus DD-078 Slices 1A–1B, 2A–2G, 3A–3E: truthful ledger/renderers, canonical project/policy/contract/purity/snapshot/planner, concrete runtime grants, bridge adaptation, metadata, isolated cases, assertions, and fixtures |
| next feature release | Landed DD-077 Design spike 0B and PRs 2C–2E plus DD-078 Slice 4, 5A–5B, 6A–6B: seeded data, shared production/verification HTTP policy, sessions, containment spike, and process supervisor/attach mode |
| following feature release | Landed DD-077 PRs 1B–1C plus DD-078 Slices 7P, 7A–7F, 8–9, 10P, 10A–10C, and 11A: provider protocol/fixtures, PostgreSQL/Redis/migration evidence, eventual/clock/lifecycle, and coordination |
| browser/project feature release | Slices 12P, 12A–12B, 13A–13E, and 14A–14B after their spikes and security review |
| project-environment feature release | Slices 14C–14D after process/provider foundations and DD-077 migration runner |
| Larrimon pure-project deletion | All relevant slices above plus Slices 14C–14D and migration compatibility Slice 16M before Task 17 |
| future monitoring/reliability releases | Slices 18P, 18A–18B, then 19A–19C, then 20P, 20A–20C; 18B also waits for pinned landed DD-047 Slice 1C/PR 2 identities, and every public provider waits for its feasibility spike and exact dependency closure |

Larrimon can begin deletion after each pinned release; it does not need to wait for the entire portfolio. The final pure-project claim waits for Task 17.

---

## Definition of done

DD-078 is implemented when:

1. one evidence ledger truthfully represents every obligation and execution result;
2. project-wide static plan and strict execution are stable public CLI contracts bound to one immutable input snapshot;
3. protected CI enforces an operator-owned obligation/profile/evidence contract and pure-authoring disposition rather than trusting repository scope;
4. native `.tnt` verification covers typed unit, HTTP, database, process, fixture, eventual, concurrency, browser, and project-policy cases;
5. capabilities are externally granted, root-confined, bounded, redacted, and cleaned up through authenticated host-ledger ownership;
6. external providers are versioned, pinned, explicitly sandboxed or trusted-uncontained, and fail closed; protected PR lanes admit only allowed containment classes;
7. Larrimon's current and planned invariant families have executable ntnt paths;
8. Larrimon has removed all project-owned Bash/Python support and orchestration plus the other compensating test-language files identified by the DD; only operator-locked externally owned non-support artifacts may be excluded from the project-owned inventory;
9. fast, full, live-network, and environment-backed profiles state their evidence, claim level, containment, and hermeticity honestly;
10. full ntnt regression, docs, hosted-platform, and independent security/architecture reviews pass against immutable commits;
11. Larrimon counts, paths, ranges, and deletion claims bind the same exact repository commit and canonical inventory digest used by the protected contract and execution snapshot.
