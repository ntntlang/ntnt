## 25. Acceptance criteria

### Truth and evidence

- [ ] Every obligation has stable identity and source location in strict mode.
- [ ] Implementation, executable, and verified coverage are separate.
- [ ] Zero-executable, unbound, unsupported, stale, blocked, and disallowed-skipped obligations fail strict mode.
- [ ] Unsupported assertions can never pass.
- [ ] JSON, JUnit, human output, and exit code derive from one evidence ledger.
- [ ] `@implements` alone cannot satisfy an obligation.

### Runtime and security

- [ ] Static lint/plan runs no project code or provider.
- [ ] `.intent` cannot request or exercise CLI, filesystem, network, database, process, browser, or secret authority.
- [ ] Intent may supply only bounded data to a preplanned binding; negative tests reject destinations, providers, resources, paths, secret headers, and legacy CLI/file actions.
- [ ] `pure-ntnt` authoring requires `proven` and fails planning for project-owned wrappers/providers, executable shebangs, inline workflow/package/Compose/Docker execution, unpinned actions/images, gitlinks/nested repositories, unclassified generated helpers, non-`.tnt` verification/support, SQL-only/browser harnesses, or untrusted exclusions; violations and exclusions are reported.
- [ ] Project requests are intersected with external host policy.
- [ ] Privileged policy and protected evidence contracts originate outside repository-controlled argv, use the same inherited-handle `TrustedInput` loader, reject unknown/non-canonical/duplicate envelope fields, and use separate frozen domains that sign the exact raw-payload SHA-256 before parsing; immutable base repository/commit/tree and protected inventory remain bound.
- [ ] Effective policy identity is always digest-bound, and hardlink/symlink/writable-ancestor/TOCTOU/malformed-policy attacks fail closed.
- [ ] Plan, execution, and report consume one immutable content-addressed snapshot; launch identity and source drift are checked.
- [ ] Paths are project-confined and symlink-safe.
- [ ] Exact argv execution has no shell expansion.
- [ ] Handles are opaque, generation-bound, unforgeable, and invalid after scope.
- [ ] Semantic `EffectKind` never authorizes an operation; every effectful sink validates an exact run/case/generation/resource/operation `VerificationGrant`.
- [ ] Constructors, provider output, serialization, globals, and concurrent runs cannot widen or cross resource grants.
- [ ] Clean environment, recursive redaction, output bounds, deadlines, cancellation, and cleanup are adversarially tested.
- [ ] Provider crash/hang/malformed/late/duplicate messages fail closed and clean resources.
- [ ] Verification authority cannot be bypassed through direct or transitive imports, aliases, module initializers, or ordinary effectful stdlib calls.
- [ ] Untrusted executable/browser/provider profiles use enforceable OS containment and brokered egress; trusted-uncontained execution is visibly prohibited from protected PR lanes.
- [ ] CPU, memory, PIDs/threads, descriptors, disk, sockets, and descendants are bounded below project code.
- [ ] Stale cleanup uses authenticated exact host-ledger records rather than project labels or prefix scans.
- [ ] Strict resources prove a durable `reserve → create → finalize → expose` broker/backend protocol and crash recovery at every boundary; unsupported backends are non-verifying and blocked from protected profiles.
- [ ] Strict/protected Redis uses the brokered disposable-instance lifecycle and proves zero residual keys/credentials after completed cleanup/reconciliation; pending cleanup cannot pass, and attached ACL mode is non-verifying.
- [ ] Imported strict evidence uses a supervisor invocation record or canonical signed, expiring, replay-resistant envelope.
- [ ] Project-wide execution is strict by default; diagnostic mode is explicitly non-verifying and cannot satisfy obligations.
- [ ] Uncatchable termination limitations are explicit; supervisor-crash and startup orphan-reaper paths are tested against authenticated ledger records.

### Application verification

- [ ] Structured function arguments/results and first-class assertions replace local assertion helpers.
- [ ] Stateful HTTP supports headers, forms, cookies, redirects, captures, multiple clients, and attach mode.
- [ ] PostgreSQL supports isolated committed fixtures, roles/RLS, migration evidence, direct observations, and cleanup.
- [ ] Managed processes support readiness, expected failure, logs, restart, exit, and process-tree cleanup.
- [ ] Local HTTP/SMTP/webhook/TCP/UDP fixtures support strict scripted behavior.
- [ ] Eventual assertions use one bounded deadline and report attempts/final observation.
- [ ] Named actors/barriers reproduce application-defined claim/scheduler/projection races without sleeps in project-neutral fixture applications.
- [ ] Browser cases cover authenticated, HTMX, no-JavaScript, focus/history, and reconciliation behavior from `.tnt`.
- [ ] Project/provider facts replace the audited Python provenance and architecture checks without granting arbitrary shell.

### Adoption portability

- [ ] A project-neutral fixture repository exercises the complete adoption protocol: immutable inventory, exact-once classification, protected contract/snapshot binding, old/new parity, mutation/fault witnesses, and evidence-backed deletion eligibility.
- [ ] The adoption protocol produces reusable machine-readable inputs and reports without project names or paths in public APIs, schemas, defaults, policies, fixture semantics, or privileged modes.
- [ ] A consumer adoption plan can bind its own inventory and migration waves to landed capabilities without joining or changing the DD-078 core DAG, releases, or completion criteria.
- [ ] The Larrimon reference-adoption checklist remains separately reviewable in [`plans/dd-078-larrimon-adoption.md`](../plans/dd-078-larrimon-adoption.md) and is not evidence that the project-neutral runtime itself passed.

---
