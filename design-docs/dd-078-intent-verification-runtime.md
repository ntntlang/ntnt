# DD-078: Intent Verification Runtime and Pure-NTNT Project Testing

**Status:** Broad roadmap retained; active implementation is the [Intent-led native milestone](dd-078-native-milestone.md).

## Active testing direction and ownership

The first useful milestone is `.intent` selecting ordinary `.tnt` functions through one small native runner. Simple IAL/glossary/HTTP scenarios remain first-class; complex executable setup and assertions belong in `.tnt`. Unit, integration and behavioral testing will compose from `.intent`, without hidden order-dependence between cases.

The linked milestone checklist is authoritative for current implementation scope. The broader verification/governance design below is retained for later discussion, not a prerequisite chain: verification-only `std/test`, mandatory opaque grants, exhaustive source/asset/migration inventories, protected CI and signed evidence are deferred. Suite composition belongs at the Intent front door; optional future resource configuration may use `ntnt.toml`. No full-suite, provider, governance or private-consumer migration claim is made by the native milestone.

- [x] First useful native function/assertion runtime (see linked checklist and validation record; full-suite composition is not shipped).
- [ ] Explicit root `.intent` suite composition with mixed native/HTTP cases.
- [ ] Bounded optional integration resources and later browser/external-DB adapters.
- [ ] Separately reviewed operational governance, only when independently needed.
**Authors:** Larri + Josh
**Created:** 2026-07-28
**Origin:** General-purpose application-verification architecture, pressure-tested by the Larrimon audit
**Related:** [IAL v1](INTENT_ASSERTION_LANGUAGE.md), [IAL vision v2](ial_vision_v2.md), [DD-037: Concurrency and Jobs](dd-037-concurrency-and-jobs.md), [DD-062: Secure Compiled Extensions](dd-062-secure-compiled-extension-libraries.md), [DD-063: Language Assessment](dd-063-language-assessment.md), DD-077: Correctness Primitives for Durable Applications

---

## 1. Decision

Ntnt will grow a first-class **Intent Verification Runtime** that makes a production ntnt application testable as a pure ntnt project. This is the v0.6.0-and-later verification track; it is not patch-release work and it may span multiple feature releases.

A pure ntnt project may still contain production assets and migrations in their natural formats, and it may depend on PostgreSQL, Redis/Valkey, Chromium, OCI containers, Git, network devices, and other external systems. The purity claim is narrower and useful:

- application behavior and durable requirements are declared in `.intent`;
- project-owned executable verification is written in `.tnt`;
- resources, profiles, capabilities, and suite composition are declared in `ntnt.toml`;
- one ntnt command plans, executes, reports, and cleans up the verification run;
- project-local Bash, Python, JavaScript, SQL-only test harnesses, and ad hoc CI orchestration are unnecessary;
- specialist engines may remain behind typed, bounded ntnt providers, but their evidence is incorporated without pretending that annotation coverage is behavioral proof.

The target CI command is:

```bash
ntnt intent check . --profile full
```

The command MUST fail closed when required obligations are unbound, unexecutable, blocked, skipped without an allowed reason, stale, or failed. `@implements` is traceability, not evidence.

This DD supersedes the execution roadmap in `ial_vision_v2.md`. That document remains historical context for term rewriting and Studio, but execution trust, resource ownership, and project verification come before additional visual tooling.

---

## 2. Why this is needed

Production applications routinely need verification across HTTP state, databases, migrations, queues, browser behavior, external protocols, project policy, lifecycle, and failure recovery. Ntnt currently has useful pieces, but not a general runtime that can plan those resources, execute typed cases, preserve authority boundaries, and report current behavioral evidence truthfully.

Larrimon is the first reference adoption and a deliberately demanding pressure test: authenticated multi-tenant HTTP, PostgreSQL RLS, immutable evidence, durable jobs, scheduler races, browser reconciliation, migrations, provenance, network protocols, alerts, AI inference, multi-node control planes, retention, and eventual HA/on-prem operation. It validates the generalized mechanisms; it does not define their public names, schemas, semantics, or release boundaries.

The [immutable Larrimon audit baseline](../plans/dd-078-larrimon-baseline.md) binds these findings to repository `https://github.com/larimonious/larrimon.git` at commit `ceadfd992d1435ac27afb054968ff5569d697ce1`. At that commit:

- the application had seven `.intent` files with 20 features, 27 scenarios, and 38 outcome/assertion lines;
- five domain intent files contained 11 features but no scenarios or assertions;
- `.tnt` source had 37 `@implements` annotations and no `@supports` annotations;
- `tests/intent.sh` had 18 direct `ntnt run tests/...` invocations and no `ntnt intent check` invocation;
- annotation coverage could report 100% while every scenario remained unexecuted;
- 27 project-owned shell, Python, and JavaScript/MJS support/test programs totaled 4,149 lines, and 3 SQL-only test inputs added 400 lines, for 30 replacement artifacts/4,549 lines; the audited `tests/` executable/spec set totaled 4,935 lines versus 4,827 lines of non-test production `.tnt`;
- most compensating code was lifecycle, fixture, assertion, polling, concurrency, database, browser, or policy plumbing rather than product-specific reasoning.

The baseline records every path, full-file range, line count, Git blob, retained product-asset classification, and canonical inventory digest. Dirty-worktree bytes were excluded. A changed Larrimon base invalidates the counts and paths and requires a regenerated baseline plus protected contract before migration or deletion. Other projects adopt the same generalized inventory, protected-contract, parity, and deletion protocol with their own repository identities and pressure profiles.

The compensation is rational. Current ntnt can parse and lint intent, resolve glossary terms, run simple HTTP checks, call simple functions, expand tabular data, and trace `@implements`. It cannot yet safely own an application verification run.

Specific current defects reinforce the architectural gap:

- live `intent check` and `src/ial/execute.rs` implement different execution paths;
- IAL request headers are modeled but ignored by one executor;
- live HTTP is hand-parsed, loses duplicate headers, assumes JSON request bodies, and has no cookie jar or capture;
- technical `setup` bindings are parsed but not executed;
- function arguments are reduced to strings/numbers and structured results are stringified;
- unsupported unit/code-quality assertions can pass as “not applicable”;
- `intent check` always starts one server, inherits ambient environment, discards stdout/stderr, and has no resource graph;
- coverage fails only at zero and measures implementation annotations rather than execution;
- existing CLI/file primitives have more authority than an untrusted specification should possess.

The answer is not an unrestricted shell primitive. That would preserve the same portability, authority, cleanup, and observability problems under friendlier prose.

---

## 3. Product outcome

A mature application should be able to organize verification as:

```text
server.intent
lib/auth.intent
jobs/run_probe.intent
verification/
  auth_cases.tnt
  database_cases.tnt
  scheduler_cases.tnt
  browser_cases.tnt
  project_cases.tnt
ntnt.toml
migrations/*.sql          # production artifact, not a test harness
public/*.js               # production asset when the application needs it
```

There should be no requirement for:

```text
tests/*.sh
tests/*.py
tests/*.mjs
tests/*_case.sql
Makefile test orchestration
curl/grep/psql polling loops
handwritten JUnit conversion
```

The runtime should support these general application-verification classes through project-neutral acceptance fixtures. Larrimon supplies a separate external consumer corpus:

- auth, tenant isolation, CSRF/origin, sessions, role changes, and revocation;
- HTTP/HTMX/full-page/no-JavaScript/browser reconciliation behavior;
- PostgreSQL migrations, RLS, roles, security-definer functions, immutability, and checksums;
- durable jobs, idempotency, leases, recovery, restart, queue failure, and deterministic races;
- monitoring probes and local protocol fixtures, including HTTP, DNS, TCP, TLS, ICMP capability behavior, SNMP, and later NETCONF/gNMI;
- immutable observation envelopes, reducers, replay, late data, incidents, suppression, and alert outboxes;
- email/webhook capture, retry, signing, and ambiguous outcomes;
- multi-node enrollment, claim, heartbeat, completion, anti-replay, rotation, and wrong-node/tenant denial;
- deterministic AI-provider fixtures, schema validation, token/plan gates, evidence citations, and no-tool assertions;
- retention, partition pruning, legal holds, backpressure, overload priorities, restore/failover evidence, and upgrade compatibility;
- project architecture, migration inventory, CI/build configuration, OCI/runtime provenance, and deployable-artifact identity.

---

## 4. Goals

1. Make execution status truthful and machine-consumable.
2. Make `.intent` the durable obligation layer without turning natural-language files into scripts.
3. Make `.tnt` the project-owned executable verification language.
4. Share one action, observation, assertion, and evidence model across `ntnt test`, `ntnt intent check`, and Studio.
5. Provide hermetic process, fixture, database, HTTP, browser, local-protocol, and project-inspection capabilities.
6. Support multi-step state, captured values, named sessions, setup/teardown, eventual assertions, restarts, and deterministic coordination.
7. Keep authority explicit, capability-gated, root-confined, bounded, redacted, and reviewable before execution.
8. Preserve external specialist engines behind typed provider boundaries while keeping project test code in ntnt.
9. Produce stable JSON and JUnit evidence with source locations, timings, hashes, and diagnostics.
10. Let any adopting project delete compensating test harnesses incrementally, with each deletion gated by equivalent or stronger evidence; prove the protocol with project-neutral fixtures before any consumer migration.
11. Remain general-purpose: ntnt gains reusable verification mechanics, not application-, monitoring-, or Larrimon-specific syntax.
12. Work on Linux first without making unearned portability claims; define Windows/macOS behavior and explicit unsupported capabilities.

## 5. Non-goals

- No arbitrary shell evaluation from `.intent` or `.tnt`.
- No claim that every external system is deterministic.
- No container orchestrator, CI service, browser engine, database server, or network emulator reimplementation inside ntnt.
- No monitoring-, tenant-, incident-, or Larrimon-specific language keywords.
- No exactly-once network, queue, or alert-delivery claim.
- No automatic proof that implementation annotations are correct.
- No execution of repository code during `ntnt intent lint` or static plan inspection.
- No live production target, private network, cloud account, payment provider, or AI-provider access by default.
- No requirement to remove production SQL migrations, JavaScript assets, HTML, CSS, or other legitimate application artifacts.
- No plugin ABI that loads untrusted native libraries into the ntnt process. DD-062 governs compiled extension trust.

---

## 6. Definitions

### 6.1 Obligation

A stable, source-located claim that must be proven. Scenario outcomes are obligations. A feature description with no outcomes is `unproven` unless it is explicitly and validly marked `verification: documentation-only`; it never passes behavioral coverage by default.

### 6.2 Evidence

A current, source-bound result produced by executing a verification case or approved provider operation. `@implements` and `@supports` are links, not evidence.

### 6.3 Verification case

A `.tnt` function discovered by stable test metadata, executed in `ExecutionMode::Verification`, and linked to one or more obligation IDs.

### 6.4 Resource

A lifecycle-owned capability such as a PostgreSQL database, disposable Redis instance, application process, worker, browser context, local mock server, temporary directory, or OCI container.

### 6.5 Provider

A built-in or separately trusted implementation that creates resources or observations behind a versioned protocol and host policy.

### 6.6 Pure ntnt project

A project whose verification specifications, test logic, suite orchestration, and project-owned development/support helpers are represented by `.intent`, `.tnt`, `ntnt.toml`, or direct ntnt/provider commands, even when the system under test depends on external resources or contains non-ntnt production assets.

---

## 7. Architectural model

```text
.intent files ───────┐
                     ├─> obligation compiler ──┐
@implements links ───┘                         │
                                               ├─> verification planner
.tnt @test/@verifies ─> test discovery ────────┤       │
                                               │       ├─> capability plan
ntnt.toml ───────────> profiles/resources ─────┘       ├─> resource DAG
                                                       ├─> executable cases
host policy ─────────> grants and hard ceilings ───────┘

planner -> resource supervisor -> case interpreters -> typed actions/providers
        -> observations -> assertions -> evidence ledger -> JSON/JUnit/human report
                                      -> guaranteed teardown/reconciliation
```

The layers have deliberately different authority:

1. **Intent parser and obligation compiler:** no execution authority.
2. **Planner:** reads project metadata and requests capabilities; it does not grant them.
3. **Host policy:** grants capability classes and hard ceilings.
4. **Supervisor:** owns processes, resources, deadlines, cancellation, and cleanup.
5. **Case interpreter:** receives only opaque handles for resources assigned to the case.
6. **Providers:** perform one bounded class of external work and return typed observations.
7. **Evidence ledger:** records results and provenance; it cannot manufacture a pass.

---

## 8. Obligation identity and truth model

### 8.1 Stable IDs

Strict mode requires stable feature, scenario, and outcome IDs:

```intent
Feature: Tenant-bound sessions
  id: feature.auth.tenant-session

  Scenario: Disabled operators lose existing access
    id: scenario.auth.disabled-session
    Given an authenticated operator
    When that identity is disabled
    → id: outcome.auth.disabled-session.denied; access is denied
    → id: outcome.auth.disabled-session.isolated; no cross-tenant state is exposed
```

Compatibility mode may derive IDs and warn. Derived IDs are not suitable for long-lived imported evidence because renaming or reordering changes identity.

A descriptive feature that intentionally makes no behavioral claim may declare `verification: documentation-only` plus a rationale. It is excluded from behavioral denominators and remains visibly counted. This marker is invalid on an outcome and cannot be used to turn an unsupported promised behavior into a pass. Future/planned behavior remains unproven unless the selected profile explicitly excludes it by stable tag or ID.

### 8.2 Orthogonal truth dimensions

Do not compress truth into one status string. Every obligation reports:

| Dimension | Values |
|---|---|
| specification | declared, documentation-only |
| implementation | linked, unlinked |
| binding | bound, unbound, ambiguous |
| executability | executable, unsupported, blocked |
| disposition | planned, running, passed, failed, flaky, skipped, cancelled, no-result |
| freshness | current, stale |

An obligation is **verified** only when all required evidence bindings are current and passed. A feature is verified only when every required obligation is verified. A feature with no required obligations is `unproven`, not behaviorally passed, unless explicitly documentation-only.

This is execution evidence, not a theorem prover. Project code can still contain a vacuous or incorrect assertion; review remains necessary. Ntnt records exactly which observation and assertion claimed each obligation, rejects zero-evidence success, and lints obvious tautologies such as literal `expect_true(true)`, but it does not claim to infer semantic correctness from arbitrary test code.

Skipped tests do not satisfy obligations. A profile may allow a named skip reason, but the report must retain the unmet obligation and strict coverage remains below 100%.

Case retries are never hidden. Strict profiles do not retry failed assertions by default. If a profile explicitly requests diagnostic reruns, every attempt is recorded and fail-then-pass is `flaky`, not verified, unless a separately reviewed policy permits that disposition. Provider-level transport retries are bounded action semantics and remain visible in evidence.

### 8.3 Evidence binding

Project-owned complex verification uses comment metadata on ordinary ntnt functions:

```ntnt
// @test: test.auth.disabled-session
// @verifies: outcome.auth.disabled-session.denied
// @verifies: outcome.auth.disabled-session.isolated
// @uses: app, postgres, redis
// @tags: auth, http, full
fn verify_disabled_session(ctx) {
    // std/test APIs consume the opaque verification context.
}
```

The first implementation uses annotations because it requires no new language syntax and matches current traceability. `@uses` assigns the minimum named resources to the case; undeclared resources remain unavailable even when the profile starts them for other cases. `@tags` supplies deterministic profile selection without conferring authority. The parser/discovery path must validate duplicate test IDs, unknown obligation IDs, unknown resources/tags, missing functions, and stale file references before resource startup. Strict profiles reject unlinked verification cases unless they are explicitly marked as diagnostic-only.

Simple scenarios may compile directly into built-in actions. Complex scenarios bind to a test function. `@verifies` declares candidate evidence bindings; returning successfully is not enough to satisfy them. Every obligation must receive at least one current assertion/evidence atom. When a test names one obligation, its unlabelled assertions may default to that obligation. When it names several, each assertion must identify the obligation it proves. A zero-assertion successful function yields `no-result`. Provider assertions follow the same rule.

Verification is profile-relative. For a selected profile, every selected non-advisory binding for an obligation must pass, at least one selected binding must produce a current assertion/evidence atom, and any selected failure fails the run. Known bindings excluded by profile tags remain visible in the report; their prior results do not become a global pass. Advisory/diagnostic bindings never satisfy an obligation. The `full` profile selects every applicable required binding, while narrower profiles can make only a profile-qualified verification claim. Manifest/profile policy, not a test's successful return, defines any required evidence classes.

### 8.4 Imported evidence

Imported JUnit/TAP/JSON may supplement provider-backed execution only when it includes:

- schema and provider identity;
- obligation IDs;
- source, intent, manifest, and plan hashes;
- tool/provider versions;
- timestamps and execution identity;
- a supervisor-issued invocation record or canonical signed envelope.

A strict imported envelope uses RFC 8785 JSON Canonicalization Scheme and Ed25519. The signature input is the domain-separated bytes `ntnt-evidence-v1\0 || JCS(envelope_without_signature)`; only `signature` is excluded, so `schema`, `algorithm`, and `key_id` are signed. `EvidenceEnvelopeV1` has one normative closed field set:

| Signed field | Required binding |
|---|---|
| `schema`, `algorithm`, `key_id` | exact v1 schema, Ed25519 algorithm, authorized key |
| `issuer`, `audience`, `evidence_class` | supervisor/provider identity, intended verifier/workflow audience, allowed evidence class |
| `repository` | forge plus immutable repository ID and canonical owner/name/URL |
| `subject` | full immutable commit SHA plus requested ref kind/name and source-snapshot digest |
| `workflow` | CI system, workflow path/ref, run ID/attempt, and protected environment/runner trust class |
| `protected_scope` | protected-contract raw and canonical semantic digests, trusted base repository ID/ref/full commit/tree OIDs, and protected inventory digest |
| `run_id`, `operation_id`, `profile`, `plan_hash`, `policy_hash` | exact invocation and authority plan |
| `input_hashes` | source, Intent, verifier, fixture, manifest, lockfile, migration, and provider-input closures |
| `runtime_hash`, `provider_hashes`, `environment_hash` | exact ntnt/runtime, provider executables/images, and mutable environment identity |
| `obligations`, `result` | stable obligation/assertion IDs and per-atom dispositions, never one unstructured pass boolean |
| `artifact_digests`, `cleanup` | every retained artifact and resource cleanup disposition |
| `issued_at`, `started_at`, `finished_at`, `expires_at`, `nonce` | bounded freshness and replay identity |

The schema fixture, report importer, key-authorization policy, and implementation tests use this exact field list rather than parallel aliases. A trusted supervisor-issued invocation record carries the same identity/result closure and is accepted only from the current authenticated supervisor channel; it is not a reduced-field bypass around the envelope. Host policy maps trusted keys to issuer/audience/evidence class, immutable repository IDs, allowed ref/workflow identities, protected-contract/base scopes, and validity/revocation windows. Unsigned evidence, legacy schemas missing mandatory fields, mutable-environment results without current environment identity, unknown fields, or user-authored `passed: true` files are display-only; they cannot satisfy strict mode. Import rejects tamper, duplicate claims, replay, key rotation/revocation failures, schema downgrade, and cross-repository, cross-ref/commit, cross-workflow/audience, cross-contract/base-ref, cross-profile/plan/policy, swapped-environment/provider, and swapped-artifact reuse. Import parsers are bounded and non-resolving: JUnit/XML disables DTDs, external entities, XInclude, and network/file resolution; TAP/JSON enforce depth, line, field, and byte limits. Artifact paths are treated as data and never followed outside the approved import bundle/root.

### 8.5 Protected evidence contract

Repository-authored specifications and tests are executable claims, not an adversarially stable requirement baseline. A pull request can otherwise delete outcomes, narrow globs, weaken a profile, or replace a meaningful assertion with a less obvious tautology while keeping every repository hash current.

Protected CI therefore supplies an operator-owned evidence contract outside the repository. It fixes the required profile, obligation IDs or approved base-ref delta policy, minimum evidence classes/resources, deletion/rename rules, and minimum counts. Planning compares the candidate snapshot with the trusted contract and base ref before execution. Repository configuration may tighten this contract but cannot weaken it. The contract proves scope continuity, not arbitrary assertion semantics: changed verifier code still requires code review and the migration mutation/fault witnesses in §22. Without that external contract, the report is labeled `project-authored-claim`; even with it, ntnt reports `protected-contract-execution-claim`, not cryptographic proof that human-authored assertions are meaningful.

Host policy and the protected evidence contract are one `TrustedInput` class. The trusted launcher opens each payload before repository code runs and passes inherited read-only handles, not repository-selected paths. Both require exact raw-byte digest, regular-file identity, trusted non-writable ownership/ancestor/ACL checks, hardlink/symlink rejection, pre/post-open identity validation, and approved signature/canonicalization algorithms where signed. The contract also has a canonical semantic digest. The launcher resolves its base repository/ref to an immutable repository ID plus full commit and tree OIDs; a mutable ref name is never the base identity. The raw contract bytes/digest, semantic digest, base repository/commit/tree, and protected inventory digest enter the snapshot, plan hash, report, replay checks, and strict evidence envelope. Rename swaps, hardlinks, mid-run replacement, base-ref retargeting, and cross-repository/ref/workflow/contract reuse fail closed.

Signed trusted inputs use two closed, unknown-field-rejecting JCS envelopes—`PolicyTrustedInputV1` and `ProtectedContractTrustedInputV1`—whose top-level fields are exactly `schema`, `algorithm`, `key_id`, `issuer`, `audience`, `repository`, `ref`, `workflow`, `not_before`, `expires_at`, `nonce`, `payload_sha256`, and `signature`. `schema` is the corresponding type name, `algorithm` is exactly `Ed25519`, identity/freshness fields are mandatory, and `payload_sha256` is lowercase SHA-256 of the exact already-open payload bytes before parsing. The envelope bytes themselves must be canonical RFC 8785 JCS with duplicate keys rejected. Signature input excludes only `signature` and is:

```text
policy:   "ntnt-policy-trusted-input-v1\0" || JCS(envelope_without_signature)
contract: "ntnt-protected-contract-trusted-input-v1\0" || JCS(envelope_without_signature)
```

Verification authenticates the envelope and key authorization, compares `payload_sha256` to the inherited payload handle, and only then parses the payload and derives its canonical semantic digest. The signature never authenticates semantics while leaving the raw-byte identity unsigned. Cross-type envelopes, unknown/duplicate fields, non-canonical envelope bytes, raw payload mutation, wrong repository/ref/workflow/audience, expired/not-yet-valid/revoked keys, and unsupported algorithms fail closed. Producer/consumer fixtures freeze both schemas and domains.

---

## 9. Project manifest and profiles

`ntnt.toml` gains a versioned `[verification]` section. The syntax below is normative in shape but may receive naming polish during implementation:

```toml
[verification]
schema = 1
authoring = "pure-ntnt"
clean_environment = true
default_profile = "fast"

[verification.files]
application = ["server.tnt", "app/**/*.tnt"]
intent = ["**/*.intent"]
verification = ["verification/**/*.tnt"]
support = ["tools/**/*.tnt"]
product_assets = ["public/**", "views/**"]
migrations = ["migrations/**"]
project_metadata = ["*.md", "*.toml", ".github/**", "compose*.yaml"]

[verification.profiles.fast]
mode = "strict"
include = ["unit", "project"]
required_coverage = 1.0

[verification.profiles.full]
mode = "strict"
include = ["unit", "db", "http", "browser", "project"]
resources = ["postgres", "redis", "mail", "app", "worker", "browser"]
required_coverage = 1.0

[verification.resources.postgres]
provider = "postgres"
mode = "external"
url_from = "TEST_DATABASE_URL"
isolation = "database-per-run"
migrations = "migrations"

[verification.resources.redis]
provider = "redis"
mode = "managed"
isolation = "instance-per-run"

[verification.resources.mail]
provider = "smtp.capture"
mode = "managed"

[verification.resources.app]
provider = "process"
containment = "mediated-ntnt"
argv = ["ntnt", "run", "server.tnt"]
depends_on = ["postgres", "redis", "mail"]
readiness = { ntnt_child_http = "/readyz", status = 200, inherited_listener = true, timeout_ms = 30000 }
pass_environment = ["APP_NAME"]
environment = [
  { name = "DATABASE_URL", resource = "postgres", output = "url" },
  { name = "REDIS_URL", resource = "redis", output = "url" },
  { name = "SMTP_URL", resource = "mail", output = "url" }
]

[verification.resources.browser]
provider = "browser.cdp"
containment = "sandboxed"
depends_on = ["app"]
executable_from_policy = "chromium"

[verification.capabilities]
request = [
  "process:ntnt",
  "network:loopback",
  "database:postgres:test",
  "browser:local",
  "filesystem:project-read",
  "git:project-read"
]
```

Rules:

- secret values never appear in the manifest;
- `*_from` names refer to host-policy-approved inputs and are redacted;
- process environment receives only explicitly passed host values and typed outputs exported by declared dependency resources; arbitrary string interpolation is not performed. Secret resource outputs may be injected by the supervisor but are not made readable to test code or reports merely because the case has the resource handle;
- commands are exact executable plus argv; no shell parsing, interpolation, redirection, or command substitution;
- host policy constrains executable identity, provider/image digest, permitted argument templates, mounts, destinations, and exported outputs; a broad capability label is not permission to choose an arbitrary executable or container;
- `authoring = "pure-ntnt"` is mechanically enforced through an exhaustive project-file and executable-declaration classifier. Every tracked path must match exactly one protected class: ntnt application source, Intent/verification, ntnt support CLI, production asset, migration, or project metadata. Overlap, omission, an unclassified executable/support file, extensionless or renamed wrapper, executable shebang, relevant untracked executable, symlink/hardlink escape, or a project-owned non-ntnt helper fails planning. The classifier also parses every supported executable-bearing metadata format: CI workflow steps/actions, Compose/OCI command and entrypoint declarations, Docker build/run hooks, package/task-runner manifests, and generated-helper declarations. Support/orchestration contexts allow only closed typed ntnt/provider operations with immutable origins; inline shell/Python/Node, YAML block scripts, heredocs, shell operators/substitution, unpinned actions/images, arbitrary container commands, and unknown executable-bearing formats fail `proven`. Git mode `160000`, nested repositories, and generated executable closure fail by default; an operator-owned origin/digest lock must recursively pin and classify every committed object before any exception. Globs and classes are part of the protected contract, so a candidate cannot relabel or omit a helper. Product assets/migrations may contain JavaScript, SQL, templates, or data, but imports, build hooks, metadata declarations, provider origins, generated outputs, and process argv must prove they are not verification/support programs;
- verification/support may use only `.tnt` cases/CLI programs and approved typed built-in/host-installed providers. Planning rejects `Primitive::Cli`, legacy file/CLI actions, direct generic process/shell effects in those roots, Bash/Python/Node wrappers, SQL-only/browser test harnesses, generic command-taking providers, and provider executables/wrappers under the project root. Application source is also ntnt in a pure-ntnt project; a production capability such as DD-065 must use a typed native contract rather than becoming a wrapper loophole;
- third-party/generated exceptions come only from an operator-controlled lock outside the repository containing origin, immutable digest, non-project ownership, and proof the artifact is not verification/support. Project-generated support is never exempt. Violations and every excluded artifact are reported; fast/full require `authoring_purity = proven` before startup;
- project-wide `intent check` is strict by default. A verifying profile cannot weaken required-obligation truth, purity, protected-contract requirements, or host-clamped skip/advisory policy. Diagnostic execution is an explicit non-verifying CLI mode whose report/exit cannot be reused as verification evidence;
- reports include `authoring_purity = proven | not_checked | violated` plus the scanned source/provider closure; successful execution never implies purity;
- all paths are canonicalized under the project root unless host policy grants a named external path;
- profile inheritance is acyclic and deterministic;
- resource dependencies form an acyclic graph;
- a dry plan is available without executing project code;
- the same profile and policy produce a stable plan hash apart from explicitly recorded dynamic allocations.

Privileged policy authority must originate outside repository-controlled argv and environment. A trusted launcher or CI control-plane step outside the checkout opens the policy/contract and then executes ntnt with a fixed profile:

```bash
/usr/local/bin/ntnt-protected-verify full
```

The wrapper is operator-installed, accepts no policy path or capability arguments from the repository, clears untrusted policy environment, opens the fixed policy/contract as `TrustedInput` handles, resolves the contract base to immutable repository/commit/tree identity, and then invokes `ntnt intent check`.

The repository requests authority. The host grants it. Ntnt's built-in default policy is unprivileged. Repository-controlled `--policy`, environment, workflow, symlink, or configuration may only reduce that default or an already provisioned host grant; it cannot create privilege. Privileged policy and protected-contract payloads always authenticate exact already-open bytes through the same `TrustedInput` machinery. Unsigned filesystem inputs require a regular non-hardlinked file, no symlink at any component, trusted owner, non-writable file and ancestor chain, platform ACL checks, and pre/post-open identity validation; same-CI-user ownership alone is insufficient. Signed inputs use the exact closed envelopes and domain strings in §8.5, verify the signed raw payload digest before payload parsing, and bind repository/ref/workflow/audience plus key validity/revocation. The trusted launcher—not repository code—chooses both payload/envelope handle pairs. The plan/report records each non-secret raw/canonical digest and trust class.

### 9.1 Immutable execution snapshot

Planning and execution consume one immutable input closure. After safe discovery, the supervisor copies or opens the exact tracked/project-declared source, Intent, verifier, fixture, migration, manifest, lockfile, policy, and protected-contract raw bytes into a private content-addressed read-only snapshot. It records the contract's raw and canonical semantic digests, immutable base repository/commit/tree, and protected inventory digest. Test interpreters and managed ntnt children execute that snapshot, not the mutable checkout. Provider/browser/runtime executables and OCI images are opened or resolved by immutable digest at launch; a path or mutable tag is insufficient. The report hashes the bytes actually consumed.

Strict mode fails if pre-snapshot discovery races, a required input cannot be captured safely, executable identity changes between validation and launch, or the source checkout drifts during the run. Generated outputs and external environment identity are recorded separately. This closes plan/use/report time-of-check/time-of-use gaps rather than attempting a hopeful post-hoc hash comparison.

---

## 10. Verification runtime and `std/test`

### 10.1 Execution isolation

Each case receives:

- a fresh interpreter by default;
- deterministic case seed;
- opaque `TestContext` bound to run, case, generation, and assigned resources;
- a clean environment containing only allowed values;
- a case deadline and cancellation token;
- a private artifact directory;
- assertion and diagnostic sinks with bounded output.

A forged, serialized, copied across runs, or expired resource handle fails deterministically. Handles are runtime values, not maps containing provider IDs.

DD-077 Design spike 0C/PR 4B's future `EffectKind` is descriptive static metadata; it is never authorization and is not a prerequisite for verification mediation. Runtime authority uses an opaque `VerificationGrant` bound to `{run_id, case_id, generation, resource_id, operation_set, scope, expiry, budget}`. Every authority-bearing sink validates the exact grant/handle at the final operation seam. A broad `database`, `network`, or `filesystem` effect classification cannot enable a constructor, another resource, another endpoint/path, or a wider operation. Grants are supervisor-minted and attenuating; test values, provider output, strings, environment, durable jobs, and imported evidence cannot mint or widen them. Constructors for external authority are unavailable in verification mode unless they consume the assigned resource grant.

Each case uses a fresh interpreter/module environment, but a fresh interpreter alone is not an isolation proof. Verification authority and environment/cwd/args overlays are installed before interpreter initialization or module evaluation can observe host state. Every process-global auth, job, database, HTTP, cache, SQLite/KV, email, secret, time/random, and concurrency registry is inventoried and made run-scoped, reset/namespaced, or unreachable in verification mode. Only after those gates pass may module globals, imported singletons, deferred statements, and mutable values be claimed not to bleed across cases or concurrent runs. Suite fixtures share only serialized/typed fixture values or opaque supervisor handles under declared synchronization; they do not share an application interpreter.

Suite/feature fixture sharing is opt-in. Shared resources must declare reset semantics. Cases are not parallelized across a shared mutable resource unless the profile explicitly allows it.

Project fixtures are ordinary `.tnt` functions with discoverable metadata:

```ntnt
// @fixture: fixture.auth.operator
// @scope: case
// @uses: postgres
// @teardown: cleanup_operator
fn create_operator(ctx) {
    // Return typed fixture values; commit state needed by app processes.
}

fn cleanup_operator(ctx, value) {
    // Runs even when the dependent case fails, before resource teardown.
}

// @test: test.auth.operator-dashboard
// @fixtures: fixture.auth.operator
// @uses: app
fn operator_dashboard(ctx) {
    let operator = fixture(ctx, "fixture.auth.operator")?
    // ...
}
```

Fixture dependencies form an acyclic DAG. Setup failure blocks dependent cases and does not satisfy their obligations. Teardown runs in reverse order after pass, failure, timeout, or cancellation; teardown failure is separately reported and fails strict mode. Case scope is the default. Suite/run sharing requires explicit reset semantics and scheduling constraints. Fixture return values preserve types and secret taint.

### 10.2 Test API shape

`std/test` and focused submodules expose free functions that require the context or an opaque child handle. Test modules are available only in `ExecutionMode::Verification`; importing them from production source or an ordinary `ntnt run` fails during validation. Verification files are loaded from configured test roots and cannot be imported by the application graph.

Verification mode is not a blanket bypass for ordinary effectful stdlib. Direct network, database connection construction, secrets, environment, filesystem, jobs, or other I/O must consume an assigned opaque `VerificationGrant` for the exact resource and operation or be denied. A denied verification grant returns a structured failure; it never uses the current runtime convention of silently returning `Unit`. Until an existing production API can accept such scoped authority safely, it is tested through a managed process/provider boundary rather than enabled ambiently in the test interpreter. Pure production functions remain directly callable.

```ntnt
import {
    expect_equal,
    expect_true,
    expect_error,
    expect_match,
    expect_contains,
    expect_path,
    subcase,
    resource
} from "std/test"

// @test: test.reducer.golden-replay
// @verifies: outcome.reducer.replay-identical
fn reducer_golden_replay(ctx) {
    let fixture = resource(ctx, "golden_observations")?
    let first = reduce(fixture.observations, fixture.eval_time)
    let second = reduce(fixture.observations, fixture.eval_time)
    expect_equal(ctx, first, second, "semantic replay")
}
```

For a verifier bound to several obligations, assertion options carry the exact evidence identity:

```ntnt
expect_equal(ctx, response.status, 403, map {
    "obligation": "outcome.auth.disabled-session.denied",
    "message": "disabled session is rejected"
})
```

Assertions compare typed interpreter values, not debug strings. Required baseline assertions include:

- equality/inequality with structural diffs;
- true/false, nil/some/ok/err and expected error class/message;
- type and shape;
- contains/not-contains for strings, arrays, maps, and sets where supported;
- key/path existence and typed path equality;
- regex, prefix, suffix, range, order, count, uniqueness, and approximate numeric comparison;
- redaction-aware snapshots/golden data;
- explicit failure and bounded diagnostic attachment.

Ordinary verification is read-only with respect to committed golden files. An explicit update command may render a taint-checked candidate and generated patch into a private restrictive artifact directory, binding the source snapshot digest, target path, prior file identity/digest/mode, and proposed digest. Ntnt never replaces the committed target automatically: a human or VCS applies the patch, resolves concurrent edits, and reruns verification against the new immutable snapshot. CI never generates or applies acceptance updates. This deliberately avoids pretending that compare-then-rename is a cross-process compare-and-swap.

Failed expectations are accumulated until the case deadline or a fatal assertion. Runtime errors, unsupported assertions, expired handles, provider failures, cleanup failures, zero-assertion cases, and candidate bindings without assertion evidence cannot pass as “not applicable.”

### 10.3 Table and property cases

Existing Intent test data remains supported. `.tnt` tests can name subcases and deterministic seeds. Property tests require explicit generators, seed recording, bounded case counts, shrinking ceilings, and replay commands. Shrinking and reproducibility claims are limited to pure, case-local observations; resource/network/database/browser effects are rejected in shrinkable properties unless a future provider defines transactional reset semantics. Resource-backed matrices remain ordinary named subcases. “Deterministic” means the same inputs under the same declared runtime observations, not merely two calls made accidentally close together.

---

## 11. One action and observation model

`src/ial/execute.rs`, live `intent check`, Studio, and test commands must converge on one model:

```text
Action + Capability + Deadline -> Observation | StructuredError
Observation + Assertion -> AssertionEvidence
```

Actions are typed and closed. Vocabulary rewrites terms into actions/assertions; it does not acquire authority.

Intent may select only a planner-approved action/binding template and supply bounded non-authoritative data validated by that template. It cannot choose a provider, executable, resource, filesystem path, network destination, secret source/value, database connection, browser target, or capability. Auto-compiled HTTP is restricted to a relative path on an already planned application resource and a closed set of non-secret fields; authenticated or stateful flows bind to `.tnt` cases holding opaque clients. Legacy `Cli`, `ReadFile`/write, arbitrary URL, secret-header, and compatibility bindings are reported unsupported in strict/pure planning and fail before effects.

Observations include:

- provider/action kind and schema version;
- start/end monotonic timing;
- bounded typed values;
- structured error class;
- truncation/redaction metadata;
- resource and process identity without secrets;
- provenance needed to reproduce the check.

Legacy IAL primitives become compatibility constructors over this model. `Primitive::Cli` is deprecated for project Intent and removed from default resolution; approved external programs run only through manifest resources/providers and host policy.

---

## 12. HTTP and session verification

Verification HTTP does not create a fourth transport/security stack. DD-077 Design spike 0B and PRs 2C–2E first land transport feasibility, trusted network configuration, one policy-bound HTTP transport, and `std/net` integration used by `src/stdlib/http.rs` and `src/stdlib/net/policy.rs`; DD-078 cannot start HTTP work before those exact merge commits are pinned. Production `std/http`, `std/net` target classification, IAL compatibility, and verification HTTP share all-address resolution, connect-time approved-address binding, proxy policy, TLS, per-hop redirect/reconnect validation, credential stripping, IPv4-mapped/private/metadata rules, body/time budgets, and error taxonomy. Verification adds session jars, captures, assertions, and evidence above that seam. Raw `TcpStream` test clients are retired rather than extended.

The shared HTTP action uses one maintained client stack and supports:

- arbitrary validated request headers;
- query parameters;
- JSON, form, raw bytes/text, and multipart bodies;
- named cookie jars and multiple simultaneous identities;
- duplicate response headers, especially `Set-Cookie`;
- redirect disabled by default, with bounded explicit policy and per-hop validation;
- response status, headers, body bytes/text, JSON, and timing;
- capture from headers, cookies, regex groups, JSON paths, HTML selectors, and URLs;
- substitution into later actions without logging secret captures;
- capture taint: cookies, authorization values, magic links, tokens, credentials, and values selected by policy become opaque secret values that may flow into approved later actions but cannot be stringified, snapshotted, or emitted;
- attach mode for an already running base URL;
- connection, request, response-size, redirect-count, and total-deadline bounds;
- exact origin, remote destination, and network-capability enforcement.

Example `.tnt` verifier:

```ntnt
import { client, request, expect_cookie_present } from "std/test/http"
import { expect_equal, resource } from "std/test"
import { latest_link } from "std/test/mail"

// @test: test.auth.magic-link-session
// @verifies: outcome.auth.magic-link.single-use
// @uses: app, mail
fn magic_link_session(ctx) {
    let browser = client(ctx, "operator")?
    let issued = request(browser, map {
        "method": "POST",
        "path": "/auth/request",
        "form": map { "email": "operator@example.test" }
    })?
    expect_equal(ctx, issued.status, 200)

    let mail = resource(ctx, "mail")?
    let link = latest_link(mail, "operator@example.test")?
    let signed_in = request(browser, map { "method": "GET", "url": link })?
    expect_equal(ctx, signed_in.status, 303)
    expect_cookie_present(ctx, browser, "__Host-session")
}
```

The API must support HTMX headers, form fallback, reconciliation polling, session revocation, redirects, and wrong-origin cases without shelling out to curl.

---

## 13. Resource and fixture model

### 13.1 Lifecycle

Every resource follows:

```text
declared -> planned -> reserved -> creating -> created -> finalized -> ready -> leased -> stopping -> stopped
                                  \-> failed/recoverable
```

The supervisor starts dependencies in topological order and tears them down in reverse order. Teardown runs after pass, failure, interpreter error, timeout, cancellation, and ordinary signal handling. Cleanup failure is reported separately and fails strict CI only for a backend whose crash-safe ownership class was proven during planning.

No system can guarantee cleanup after host power loss or an uncatchable kill, and a filesystem ledger cannot transact atomically with a process, database, or OCI daemon. Strict lifecycle ownership therefore uses an explicit `reserve → create → finalize → expose` protocol through a durable host supervisor/broker:

1. persist and fsync an authenticated reservation before creation, binding a high-entropy ownership token, deterministic backend-safe creation identity, run/resource/project/policy/provider identities, operation scope, expiry, and cleanup authority;
2. have the durable broker create the resource while retaining lifetime ownership or using a backend idempotency/recovery token;
3. obtain an exact object ID plus provider creation receipt, persist/finalize it atomically in the ledger, and revalidate the created object's token/identity;
4. only after finalization release readiness, credentials, endpoint, listener, or opaque handle to project code.

A crash before creation leaves only an expirable reservation. A crash after creation but before finalization must be recoverable by exact reserved identity/token or by broker-owned lifetime cleanup; broad PID/name/prefix/port/container/database scans remain forbidden. The broker is an ntnt-installed, host-policy-pinned service or dedicated supervisor process started and authenticated outside repository control; project argv/env/config cannot select its endpoint, identity, state root, or cleanup authority. Clients connect over an inherited or mutually authenticated local handle, and protected profiles fail planning if no durable broker class is available. Broker binary/config/state identities enter the plan/report. Broker restart reconciles only its authenticated reservations/receipts. Processes are created suspended or inside a pre-owned cgroup/Job Object, recorded with pidfd/start time/executable/run token, then resumed only after finalization. Providers declare and prove their prepare/create/recover/finalize state machine, including controller and broker crash at every boundary. If the OS/backend cannot retain lifetime ownership or recover exact creation from the reservation token, cleanup is reported `best-effort`/non-verifying and strict/protected profiles reject that resource before startup. Partial/corrupt ledger writes and identity mismatches fail closed; leases have bounded TTLs and cleanup is idempotent.

### 13.2 Fixture scopes

- `case`: fresh for each test; default for mutable state;
- `suite`: shared with an explicit reset operation;
- `run`: shared infrastructure such as one PostgreSQL server, while each case/suite receives an isolated database/schema;
- `external`: lifecycle not owned by ntnt; health and namespace cleanup still apply.

Fixtures return typed values or opaque handles. App-specific setup belongs in named `.tnt` fixtures, while providers own generic infrastructure lifecycle. Inline `setup` strings in `.intent` are deprecated and never interpreted as SQL or shell.

### 13.3 PostgreSQL

The PostgreSQL provider supports:

- external server and managed OCI modes;
- database-per-run or schema-per-run isolation;
- committed canonical seeds visible to app/worker pools;
- explicit owner, migrator, app, worker, and tenant roles;
- migration application and checksum evidence through landed DD-077 PRs 1B–1C;
- bounded query/execute observations from `.tnt` tests;
- transaction and held-lock actors for SQL-only deterministic races;
- RLS context and privilege assertions;
- bounded database/schema cleanup on ordinary termination plus authenticated stale-run reconciliation;
- query, row, byte, statement-timeout, and lock-timeout ceilings;
- no credential or parameter value leakage.

Destructive lifecycle operations are confined to identifiers generated for the current run and recorded in the supervisor ledger. External-server policy pins the approved endpoint identity, generated database/schema prefix, and allowed create/drop operations. A repository cannot point the provider at another server and inherit cleanup authority. SQL identifiers are constructed only by validated provider code, never interpolated from test values.

Transaction-per-case alone is insufficient for app-backed tests because separate pooled connections cannot see uncommitted fixture data. It remains useful for direct SQL checks.

### 13.4 Services and processes

The process provider supports exact argv, clean environment, working directory under policy, readiness observations, expected-startup-failure mode, stdout/stderr ring buffers, process groups/job objects, restart, stop, liveness, exit assertions, and deadlines.

A process group is lifecycle control, not a security sandbox. Every executable resource declares one containment class:

- `mediated-ntnt`: a managed ntnt child executing the immutable snapshot with a run-scoped policy; all native/module-initializer/transitive effects are checked at the final runtime dispatch seam;
- `sandboxed`: OS/container enforcement provides a private writable root/HOME/tmp, constrained read-only inputs, dedicated identity, CPU/memory/PID/file-descriptor/disk limits, descendant containment, and brokered/allowlisted egress;
- `trusted-uncontained`: a pinned operator-approved binary whose direct syscalls are outside ntnt enforcement. This class is prohibited in untrusted-PR and hermetic profiles and receives no repository secret merely because its protocol is valid.

Linux may satisfy `sandboxed` with user/mount/network namespaces or rootless OCI plus cgroups/seccomp; Windows requires Job Object/AppContainer or equivalent policy; macOS requires an approved sandbox boundary. If a platform cannot enforce a profile's declared guarantees, planning blocks it. Verification disables implicit dotenv loading and denies project `.env`/credential files unless named host policy grants them. Bare port probing is not authenticated readiness: ntnt children use inherited/reserved listeners or a run-nonce-bound readiness channel; other processes require provider/process identity evidence.

Application, worker, scheduler, provider stubs, and external-agent processes are ordinary named resources. Tests may hold separate instances with isolated ports and resource namespaces.

### 13.5 Local protocol fixtures

Built-in local-only fixtures should cover common deterministic dependencies:

- HTTP/HTTPS scripted server;
- SMTP/mail capture;
- webhook receiver;
- TCP and UDP scripted peers;
- DNS fixture/resolver where platform support permits;
- Redis disposable instance for verifying profiles; attached ACL mode is diagnostic/non-verifying;
- byte-oriented request/response scripts suitable for SNMP and later protocol fixtures;
- deterministic AI/payment/API stub responses;
- temporary workspace resources with root-confined read/write APIs and optional bounded copies of declared project fixtures.

Fixtures must be finite, bounded, loopback by default, record requests with redaction, and fail on unexpected traffic when strict. Temporary workspaces expose opaque handles or supervisor-injected paths only to declared dependants; test code cannot turn them into arbitrary host filesystem authority.

Redis key prefixes and logical database numbers are organization conventions, not isolation. Strict, hermetic, protected, and cleanup-claiming profiles require a disposable per-run instance whose process/container/volume follows §13.1's brokered lifecycle. Controller/provider crashes must be recoverable by exact reservation/object/token; reports remain cleanup-pending/non-passing until reconciliation completes, and only completed ordinary/recovery cleanup may claim zero residual keys plus revoked credentials. Host power loss retains §13.1's honest limitation. Attached Redis is explicitly trusted, non-hermetic, and non-verifying because Redis does not attribute dynamically created keys to the ACL user that created them, so exact cleanup cannot be proven when the application connects directly. An operator-created per-run ACL user with random key pattern, strict command allowlist, and mandatory bounded TTL may reduce attached-mode risk, but it cannot satisfy protected obligations or claim immediate cleanup. A future enforcing broker may strengthen that class only after it transactionally observes every mutation, records exact keys, enforces TTLs, and proves cleanup. Supervisor credentials never reach app/test code.

---

## 14. Eventual behavior, lifecycle, and deterministic coordination

### 14.1 Eventual assertions

Polling is represented as a repeated observation under one deadline:

```ntnt
import { eventually } from "std/test"

eventually(ctx, map {
    "within_ms": 5000,
    "every_ms": 100,
    "description": "queued run becomes terminal"
}, fn() {
    return load_run_state(db, run_id) == "completed"
})?
```

The final syntax depends on DD-077 PR 0A's reusable native-callback bridge. This feature waits for that bridge; no bounded-provider or verification-only callback fallback is permitted. Reports include attempts, elapsed time, final observation, and whether cancellation interrupted the wait. No unbounded sleep loops.

### 14.2 Expected failures and restart

Tests can assert startup rejection, process exit, provider error class, transaction rollback, readiness loss, and recovery after restart. “Failed to start” is data only when the case explicitly expects it; otherwise it is a failed/blocked resource.

### 14.3 Concurrency

The runtime provides named actors, parallel groups, and barriers:

- actor start/release/join;
- barrier wait/release with participant count;
- held PostgreSQL transaction/lock steps;
- mock-provider response holds;
- bounded cancellation and deadlock diagnostics;
- deterministic release order recorded in evidence.

This controls test-visible interleavings. It does not claim to deterministically schedule arbitrary kernel, database, browser, or interpreter internals. Race tests must place barriers at observable seams.

### 14.4 Time, randomness, and faults

- property/table tests receive a deterministic generator seed; this does not virtualize wall time;
- DD-078 Slice 10P first inventories every direct wall-clock, monotonic-clock, sleep, auth/job expiry, UUID/random, retry, scheduler, and runtime-deadline site and proves one internal per-interpreter observation seam; DD-077 currently owns no such seam;
- only after that seam exists do in-process tests receive a test clock and managed ntnt processes opt into a local, run-token-bound control channel;
- the control channel is disabled outside `ExecutionMode::Verification`, never listens on a non-loopback interface, and cannot affect an unbound process;
- external systems use bounded real time unless their provider supports virtual time;
- dependency failures are injected through providers/proxies or named resources, not unrestricted monkey-patching of production code;
- every injected fault is named and reported.

---

## 15. Browser verification from ntnt

A pure ntnt project must not require a project-owned Playwright/Node test file. `std/test/browser` provides typed browser actions backed by a policy-approved local browser provider, initially Chromium DevTools Protocol:

- launch/connect and isolated browser contexts;
- JavaScript-enabled and no-JavaScript modes;
- viewport, locale, timezone, and reduced-motion controls;
- navigation and redirect history;
- selectors/locators, count, text, HTML, attributes, visibility, focus, and accessibility snapshots;
- click, fill, select, submit, keyboard, and history actions;
- request interception, delay, abort, offline mode, and response observation;
- screenshots, traces, console errors, and failed requests as bounded artifacts;
- explicit script evaluation for behavior that cannot be observed otherwise, with output and deadline bounds;
- cookies, storage, and multiple contexts for separate users;
- deterministic cleanup of pages, contexts, and the browser process.

Chromium remains an external resource. Ntnt owns the project-facing API, capability plan, lifecycle, evidence, and redaction. The provider reports browser executable/version/digest. Host policy pins acceptable executables or OCI images.

CDP interception is observability, not an egress security boundary. Untrusted-PR browser profiles require a sandboxed browser with a private profile/home/filesystem and network enforcement beneath Chromium—an isolated network namespace/container or mandatory broker/proxy that denies undeclared DNS, loopback, private/metadata, WebSocket, WebRTC, service-worker, extension, and download paths. If the host cannot enforce that boundary, the profile blocks. A trusted-uncontained local browser profile is explicitly non-hermetic and cannot receive protected secrets or satisfy the protected CI contract.

Browser navigation and subresource requests use the same network policy as other actions. `file:` URLs, local browser profile reuse, extension loading, arbitrary remote-debug targets, downloads outside the artifact directory, and access to undeclared loopback services are denied by default.

Screenshots and traces can contain secret pixels or page content that generic string redaction cannot repair. They are sensitive artifacts: disabled unless the profile/policy permits them, written with restrictive permissions, masked with configured private selectors where possible, bounded, and labeled with retention/export policy. Ntnt MUST NOT claim arbitrary screenshot pixels are safely redacted.

Consumer adoption plans can then express reconciliation, fragment behavior, responsive rendering, focus transfer, no-JavaScript fallback, and authenticated browser smoke in `.tnt`. The Larrimon plan is the first concrete consumer of that generalized capability.

---

## 16. Project, architecture, migration, and provenance verification

Not every obligation runs against a live app. `std/test/project` exposes typed, read-only facts:

- canonical project file inventory and hashes;
- ntnt AST/import graph, function annotations, routes, effects when available, and ownership locations;
- UTF-8/text queries under the project root;
- parsed JSON, TOML, YAML, and lockfile data through bounded parsers;
- Git tracked/untracked/blob/ref facts without network access;
- migration inventory/checksums/status through landed DD-077 PRs 1B–1C;
- OCI image config, labels, layers, platform, and digest through a read-only provider;
- rendered Compose/project configuration through a pinned provider when required;
- generated-document drift and runtime/source identity.

The default project view contains tracked source plus explicitly configured generated artifacts. VCS-ignored files, `.env*`, credentials, private keys, editor state, and host metadata are excluded unless trusted host policy grants a named path. Facts are read-only, root-confined, bounded, and cacheable by source hash.

First-class Intent `Constraint` support should compile architecture obligations into these facts. Until then, `.tnt` tests linked by `@verifies` provide the executable form.

The goal is not to replace Python regexes with Rust regexes. Ntnt source constraints should use AST/import/effect facts. Generic text checks remain available for artifacts without a stable parser, but reports identify them as textual evidence.

Migration immutability, image provenance, CI policy, deployment shape, and asset integrity can therefore be authored in `.tnt`. Specialist tools may execute behind typed providers, but project-local Python and shell are unnecessary.

### 16.1 Typed project state and environment lifecycle

Pure-ntnt support includes ordinary development/staging operations, not only tests. Slices 14C–14D add a native project-state service plus typed environment lifecycle; they do not expose generic shell or arbitrary OCI commands.

Slice 14C stores versioned state outside the checkout under an OS-appropriate user state root keyed by canonical project-root digest and environment name. Each record binds project/manifest digests, environment/generation, lifecycle state, pre-creation reservation identity/token, exact provider/object identities and creation receipts once available, finalization generation, allocation leases, opaque secret handles, timestamps, and cleanup disposition. Creation and transition use restrictive permissions, exclusive cross-process locking, temporary-file plus file/directory fsync and atomic replace, schema validation, and compare-and-swap generation. Corrupt, foreign-root, stale-generation, symlink/hardlink, writable-parent, or partially written state fails closed. Legacy schema upgrades are explicit and tested. This slice supplies provider-neutral lock/lease/CAS machinery; it neither inspects nor mutates OCI.

Port allocation never probes and releases. Slice 14C gives managed processes inherited listeners and exposes a global allocation transaction. Slice 14D uses one realizable OCI handoff: a durable broker retains each host TCP listener for the environment lifetime and proxies it to an unpublished container port after the final service object is created; Compose does not publish or rebind that host port. Before exposure, the broker finalizes a route object binding `{listener ID, container object ID, network endpoint ID, generation, target port, ownership token}`. On every accept/reconnect it re-inspects and validates that exact object/endpoint/generation through the pinned daemon API, or consumes an authenticated daemon event stream that invalidates the route fail-closed; it never routes by service name, alias, or cached IP alone. Before forwarding any application or readiness byte, the target must also complete a broker-controlled generation-bound authentication handshake (for example an opaque sidecar nonce or ephemeral mTLS identity unavailable to peer containers). Container recreation, endpoint change, daemon restart, failed target authentication, or event-stream loss requires a newly finalized generation before traffic flows, and stale/wrong targets receive zero application bytes. UDP or a backend that cannot provide this lifetime route proof must instead let the daemon allocate the port on the final service object and persist/recover that exact object/port before exposure, or be rejected from strict profiles—there is no placeholder-socket handoff. While holding the allocation transaction, 14D also creates the exact pre-created network/reservation, records immutable daemon object IDs plus ownership token/creation receipt, and only then releases the lock. Compose consumes that external network rather than recreating it by name. The user-state lock coordinates ntnt planners but is not a daemon-wide mutex; OS listener binding and daemon object creation are the authoritative conflict checks against other users and non-cooperating clients. Conflicts cause bounded re-inspection/replan, never overlap. Crash between backend creation and state finalization follows §13.1's exact reserved identity/token recovery. Worktree/project/image identities use the full canonical root digest plus collision handling; ambient environment cannot retarget them. Secret values are generated or obtained as opaque outputs, written only when an approved provider requires a restrictive compatibility file, and never printed or exposed to `.tnt` string APIs.

Slice 14D defines `[project.environments.NAME]` with pinned provider, Compose/OCI manifests, profiles, services, build/create/up ordering, a migration action backed by landed DD-077 PRs 1B–1C, readiness, exported non-secret outputs, state schema, and cleanup policy. The normative shape is typed rather than an action/argv escape:

```toml
[project.environments.staging]
schema = 1
provider = "oci.compose"
manifest_files = ["compose.prod.yaml", "compose.staging.yaml"]
profiles = ["workers"]
allowed_services = ["postgres", "redis", "mail", "migrate", "app", "worker"]
build_services = ["app"]
create_services = ["app", "worker"]
start_dependencies = ["postgres", "redis", "mail"]
migration = "default" # landed DD-077 PRs 1B–1C ntnt db plan identity
start_services = ["app", "worker"]
readiness = { service = "app", path = "/readyz", status = 200, timeout_ms = 60000 }
state_scope = "canonical-worktree"
port_pools = ["staging-app", "staging-postgres", "staging-redis"]
subnet_pools = ["staging-private", "staging-edge", "staging-egress"]
cleanup = "exact-owned-objects"
```

`ntnt project env init|up|down|status NAME` parses and renders effective configuration through a typed OCI provider. Pure mode rejects shell interpreters/operators and project-owned shell/Python entrypoints or lifecycle hooks in that rendered configuration. `down` acts only on exact state-ledger object IDs after root/manifest/generation/ownership revalidation; it never trusts a project name, label prefix, or repository-supplied cleanup target. Crashes, concurrent worktrees, occupied ports/subnets, partial starts, migration failure, cancellation, stale state, and provider drift produce explicit recoverable dispositions. Host policy grants OCI/socket/build/network authority; untrusted PR profiles lacking that grant can plan but cannot execute the environment.

These slices provide the generalized replacement surface for project-owned environment lifecycle and state programs. The standalone Larrimon adoption plan maps its concrete files and pure-project claim to these capabilities without making that migration a DD-078 release gate.

---

## 17. Provider protocol

Built-in providers are preferred for core HTTP, process, PostgreSQL, local fixtures, and project facts. Out-of-process providers, including CDP/OCI adapters when not built in, are allowed only through a versioned protocol and host policy.

Protocol Slice 7P first proves this transport/framing adversarially on supported platforms without creating a public API. Only after that gate does v1 freeze inherited anonymous stdin/stdout pipes; each frame is a four-byte big-endian length followed by bounded UTF-8 JSON in a strict versioned schema. Stdout carries protocol only and stderr carries bounded redacted diagnostics. A later transport needs a protocol revision and conformance suite rather than silent substitution.

Required protocol properties:

- explicit handshake with protocol/provider versions and capability set;
- length-delimited messages with schema validation and message-size ceilings;
- request IDs, deadlines, cancellation, heartbeat, and terminal result exactly once;
- opaque resource handles scoped to run/provider/generation;
- no ambient inherited secrets or environment unless granted;
- structured errors and redaction metadata;
- bounded stdout/stderr separate from protocol transport;
- provider executable/digest provenance;
- crash, hang, malformed message, duplicate result, late result, and cancellation tests;
- an explicit containment class (`sandboxed` or `trusted-uncontained`) and permission manifest; protocol mediation alone is not described as syscall isolation;
- no in-process native plugin loading through this interface;
- denial of provider-requested capability escalation.

A provider cannot declare its own result trusted merely because it emitted valid JSON. Host policy decides which provider identity may satisfy each evidence class. Pinned `trusted-uncontained` providers are fully trusted for their direct OS access and therefore forbidden in untrusted-PR/hermetic profiles; sandboxed providers receive only operation-specific handles and brokered destinations. Project-owned provider wrappers are executable repository code and fail `pure-ntnt` authoring.

---

## 18. Capability and security model

### 18.1 Trust zones

| Input | Default trust | Authority |
|---|---|---|
| `.intent` | untrusted specification | none |
| ntnt source under test | project code | ordinary runtime capabilities |
| verification `.tnt` | executable test code | only assigned opaque test capabilities |
| repository `ntnt.toml` | authority request | cannot grant itself authority |
| host/CI policy | trusted operator configuration | grants capabilities and ceilings |
| provider binary/image | trusted only when pinned/approved | provider-specific |
| imported report | untrusted data by default | none without provenance validation |

### 18.2 Mandatory rules

- Static lint and plan inspection execute no project code.
- Network defaults to loopback and declared resource destinations.
- Public live-network smoke requires an explicit profile, host grant, destination policy, and visible report marker.
- Private/link-local/metadata destinations require stronger explicit grants; production credentials are never implied.
- Destination policy is enforced after resolution and on every redirect/reconnect; DNS rebinding or a later private address cannot inherit approval from an earlier public answer.
- Filesystem access is root-confined and symlink-safe; writes go only to assigned temp/artifact locations unless granted.
- Process execution uses exact argv and executable identity; no shell.
- Environment is empty by default apart from runtime essentials and allowed names.
- Secret inputs remain opaque and are redacted recursively from values, diffs, logs, URLs, headers, SQL diagnostics, artifact metadata, and textual artifacts. Binary/browser artifacts follow the separate sensitive-artifact policy because arbitrary pixels and opaque formats cannot be reliably redacted.
- Each provider/action has time, byte, row, process, request, redirect, and concurrency ceilings clamped by host policy.
- Cleanup authority is retained by the supervisor and cannot be discarded by test code.
- A timeout or cancellation never becomes a pass.
- Unsupported capability, platform, assertion, or evidence schema fails closed in strict profiles.
- JSON output never mixes banners or diagnostics on stdout.
- Providers capable of spend, public mutation, deployment, device configuration, or other irreversible side effects are denied in ordinary verification. A future explicit live-validation profile must expose estimated/hard budget, destination, idempotency/cleanup semantics, and separate host approval; it cannot be enabled merely by repository manifest changes.
- OCI/container authority is separate from ordinary process authority. Managed containers require pinned images and deny privileged mode, host networking, host PID/IPC, device access, arbitrary bind mounts, and Docker-socket mounting unless each is independently granted by host policy.
- CI for untrusted pull requests uses an unprivileged policy with no production secrets, private-network grants, host container socket, or deployment credentials. A repository change to `ntnt.toml` or verification code cannot modify the external policy that grants those capabilities.

### 18.3 Baseline resource limits

Defaults are conservative and host policy may lower them. Project configuration may request increases only up to host maxima.

| Resource | Default | Suggested host maximum |
|---|---:|---:|
| case deadline | 30 s | 10 min |
| suite deadline | 15 min | 2 h |
| eventual wait | 5 s | 5 min |
| action output per stream | 1 MiB | 16 MiB |
| HTTP response body | 1 MiB | 16 MiB |
| database rows | 1,000 | 100,000 |
| database result bytes | 8 MiB | 64 MiB |
| managed processes | 16 | 64 |
| cumulative process launches | 64 | 512 |
| process tree PIDs/threads | 128 | host clamp |
| process CPU/memory | 2 cores / 2 GiB | host clamp |
| open files/sockets per process | 256 / 128 | host clamp |
| scratch + temp disk | 1 GiB | host clamp |
| aggregate network connections | 128 | host clamp |
| aggregate database connections | 64 | host clamp |
| concurrent actors | 32 | 256 |
| browser contexts/pages | 4/8 | 16/32 |
| artifacts per suite | 100 MiB | 1 GiB |

One monotonic whole-run budget includes planning, startup, retries, readiness, actions, decoding, teardown, and report construction; nested budgets do not reset it. Bytes are charged at the read boundary, including malformed frames. CPU, memory, PIDs/threads, file descriptors, disk, descendants, and sockets require OS/container enforcement for any profile claiming containment. If those controls or descendant guarantees are unsupported, that profile blocks rather than merely printing a caveat. Exact numeric limits may be adjusted through implementation review, but unbounded or cooperative-only containment claims are not acceptable.

---

## 19. Reports and CI contract

JSON is a versioned public contract. A run report includes:

- report schema and ntnt version/commit;
- run ID, profile, platform, start/end/duration;
- immutable repository identity, full subject commit/requested ref, and workflow/run/attempt/trust identity;
- claim level (`project-authored-claim` or `protected-contract-execution-claim`), protected-contract raw/canonical digests plus trusted base repository/ref/full commit/tree and protected-inventory identity, and authoring-purity disposition;
- immutable input-snapshot digest plus project root identity, exact consumed source/Intent/verifier/fixture/migration/manifest/lockfile/provider-input hashes, effective host-policy digest and trust class, plan hash, runtime/provider/environment identities, and source-drift result;
- requested, granted, and denied capabilities without secret values;
- resource lifecycle, containment class/guarantees, authenticated ledger IDs, and cleanup results;
- every feature/scenario/outcome obligation with all truth dimensions;
- test/evidence IDs, source locations, provider/tool versions, timings, attempts, seed, and disposition;
- bounded expected/actual structural diffs;
- redaction and truncation markers;
- artifacts by digest/path/media type;
- implementation, executable, and verified coverage as separate metrics.

Exit behavior for strict profiles:

| Condition | Exit |
|---|---:|
| all required obligations current and passed; authoring purity proven when required; cleanup succeeded | 0 |
| failed, flaky, unbound, unsupported, blocked, stale, disallowed skip, no-result, or cleanup failure | 1 |
| invalid spec/manifest/policy/plan, purity violation/not-checked, or protected-contract scope regression | 2 |
| internal ntnt/provider protocol defect | 3 |

JUnit is derived from the same ledger. It must not recompute truth differently. Human output is a rendering of the same report.

`ntnt intent coverage --json` reports at least:

- implementation-linked feature/outcome coverage;
- executable obligation coverage;
- verified current obligation coverage;
- documentation-only counts;
- thresholds and unmet IDs.

---

## 20. CLI surface

Planned commands and compatibility:

```bash
# Safe, static
ntnt intent lint .
ntnt intent plan . --profile full --json
ntnt intent coverage . --json

# Execute
ntnt intent check . --profile full
ntnt intent check . --profile full --report-json report.json
ntnt intent check . --profile http --base-url http://127.0.0.1:8081
ntnt test verification/reducer_cases.tnt

# Reproduce
ntnt intent replay report.json --case test.reducer.golden-replay

# Typed project environments
ntnt project env init staging
ntnt project env up dev
ntnt project env up staging
ntnt project env status staging --json
ntnt project env down staging

# Resource diagnostics
ntnt intent doctor . --profile full
ntnt intent clean . --stale --dry-run
```

Existing `ntnt intent check server.tnt`, direct technical `test:` blocks, and `ntnt test server.tnt --get ...` remain compatibility surfaces during migration. They are implemented through the shared planner/executor and emit deprecation guidance only when a safer project form is available.

Replay treats the report as untrusted selection data. It may recover a case ID, seed, and requested profile, but it re-discovers current project sources, rebuilds the plan, revalidates hashes, and reacquires authority from the current host policy. It never executes provider paths, environment values, or capability grants copied from a report.

`intent clean` consults the supervisor-owned authenticated ledger for the canonical project and current operator policy. It can reconcile only exact recorded objects with valid ownership tokens; project files, report contents, names, prefixes, or glob scans cannot nominate cleanup targets. Dry-run output uses safe resource IDs only.

`--json` prints JSON only. A file argument should be separated from output naming to avoid stdout ambiguity during implementation.

---

## 21. Relationship to DD-077 and other systems

DD-078 owns verification orchestration and evidence. The audited DD-077 candidate is in `https://github.com/ntntlang/ntnt.git` at commit `f0132afcff984bb43305be39122d7e74a6850396`, document `design-docs/dd-077-correctness-primitives-roadmap.md`, Git blob `31a6d82f79e6051a7f00bfb182c979e5e78f2c3f`, on a separate unmerged lineage; it is not an ancestor of this DD. It is design evidence only until its owning document and implementations merge. DD-078 uses that document's literal owning identifiers rather than inventing aliases:

- DD-077 PR 0A owns the callback bridge; Design spike 0B and PRs 2C–2E own shared network policy/transport; PRs 1B–1C own migration lifecycle; Design spike 0C/PR 4B own static effects. DD-077 also owns scoped PostgreSQL transactions, leases/fencing, purity, and production runtime context.
- DD-078 owns isolated test databases/schemas, migration evidence, held test transactions, barriers, and resource cleanup.
- DD-062 owns trust for compiled extension libraries. DD-078's external provider protocol is out-of-process, does not authorize native in-process loading, and claims no sandbox beyond its separately reported containment class.
- DD-037 owns production job semantics. DD-078 owns observation, orchestration, failure injection, and evidence for those semantics.
- DD-047 owns network-monitoring primitives and catalogs. Its audited source is ntnt commit `5a24c0cd1ff2f4d58e77ef263346cf6828cd28d6`, path `design-docs/dd-047-std-netmon.md`, blob `41b644195e2aaa81997f76631daa8bae5e5cb53c`; source identity does not satisfy its unimplemented Slice 1C or PR 2. DD-078 owns bounded local fixtures and application verification around those APIs only after the plan ledger records their exact implementation merges.
- DD-065/`std/harness` would own production agent/tool execution, but this baseline contains no DD-065 design/owner artifact. DD-078 therefore excludes effect-transcript and tool-using production cases; deterministic protocol/AI fixtures and no-tool assertions do not substitute for that missing contract.

DD-078 truth-accounting, project, policy, contract, purity, snapshot, and concrete-grant slices may proceed independently. Any externally dependent slice blocks until the implementation plan's generalized prerequisite ledger names the exact owner/source and records every required implementation merge. DD-077 PR 0A gates callback consumers; Design spike 0B plus PRs 2C–2E gate shared HTTP/network work; PRs 1B–1C gate migration consumers. DD-047 Slice 1C and PR 2 gate catalog/recognition acceptance Slice 18B. DD-078 Slices 10P/10B exclusively own the missing generic internal runtime observation/clock seam. DD-078 ships no temporary public traits, verification-only callback special cases, provider fallback, or duplicate network/migration seam.

---

## 22. General adoption contract

DD-078 defines a project-neutral adoption protocol: pin an immutable project inventory, classify every relevant path exactly once, bind a protected contract and execution snapshot to that identity, dual-run old and new checks, require semantic mutation/fault witnesses, and delete compensating code only after equivalent or stronger evidence passes on clean CI.

The protocol itself is normative and is accepted with project-neutral fixture repositories. No particular application inventory, migration wave, helper deletion, or adoption completion date participates in the DD-078 core DAG, release sequence, or definition of done. Consumer projects maintain separate adoption plans that bind their own invariants to landed DD-078 capabilities.

Larrimon is the first reference consumer. Its immutable baseline, Waves A–E, migration compatibility Slice 16M, deletion authority, and future pressure corpus are tracked in [`plans/dd-078-larrimon-adoption.md`](../plans/dd-078-larrimon-adoption.md). That plan may expose missing generalized capabilities, but cannot change ntnt APIs or block completion of the project-neutral runtime.

---

## 23. Illustrative future-pressure matrix (non-normative)

| Reference-consumer pressure | Generalized DD-078 mechanism |
|---|---|
| pure reducer, replay, backtest | typed values, golden fixtures, deterministic seed/time, property/subcase reports |
| signup/invite/Turnstile | stateful HTTP, independent identity/IP/purpose rate-limit actors, provider stub, verified-email membership fixture, transactional side-effect/rollback evidence |
| unified egress policy | shared production/verification transport policy, IPv4/IPv6 classification, metadata denial, scripted DNS rebinding, per-hop redirect/reconnect checks, credential stripping, private-node scope |
| tenant/RLS/security definer | isolated PostgreSQL roles/databases, direct SQL observations, app HTTP sessions |
| wrapped secrets/KMS release | typed KMS/secret-service fixture, AES-GCM nonce/AAD and envelope/rewrap vectors, purpose/run/node binding, expiry/write-only tests, KMS denial, proof agents never receive KEK/KMS authority |
| durable scheduler/probe workers | multi-process resources, eventual assertions, restart, queue observations |
| race-safe claims/projections/suppression | actors, barriers, held transactions, bounded deadlock diagnostics |
| alerts/email/webhooks | SMTP/webhook capture, stable-event assertions, fault/ambiguous outcome fixtures |
| SNMP/MIB/device inventory | UDP/binary scripted fixtures, counter wrap/reset/rate normalization vectors, gated real-network profile, secret redaction |
| NETCONF/gNMI/device onboarding | typed NETCONF plus gRPC/HTTP2 streaming/event-source providers with auth, ordering, reconnect, subscription flow-control/backpressure, bounded retention, and capability-scoped device smoke |
| syslog/inbound telemetry | TCP/UDP/TLS event-source fixtures, malformed/auth/order/reconnect/backpressure cases, bounded retention and immutable ingestion evidence |
| multi-node control plane | multiple app/agent processes, signed request fixtures, clocks, anti-replay concurrency, encrypted completion-spool failover replay |
| agent/tool harness | DD-065 effect-transcript provider, allow/deny capability assertions, bounded tool/network budgets, deterministic no-tool and tool-using cases; outside DD-078 and blocked because this baseline has no DD-065 design/owner artifact |
| AI hypotheses/discovery | deterministic provider stub, schema/evidence assertions, call/token counters, no-tool/network policy |
| plans/billing/usage | external API/webhook fixture, deterministic clock, idempotency and signature assertions |
| retention/partitions/legal hold | database resources, clock control, large bounded fixture generation |
| load/backpressure/priorities | bounded actor/load provider, resource metrics, starvation/deadline assertions |
| browser/HTMX/no-JS | CDP contexts, interception, focus/history/DOM/accessibility evidence |
| migrations/upgrades/rollback | DD-077 PRs 1B–1C migration integration, matrix resources, image/runtime pinning |
| HA/restore/failover | external topology provider proving independent failure domains, fenced one-writer promotion, tenant home-region/data residency, encrypted completion-spool replay, canonical queue/KMS outage behavior, PITR/RPO/RTO and immutable provenance |
| upgrades/overload/game day | expand/migrate/contract compatibility across app/worker/agent versions, exact heartbeat/completion/reducer/alert priority before probes/discovery/AI, clock/DNS/certificate/KMS failures and measurable recovery evidence |
| on-prem/private networks | host policy, scoped network capabilities, customer-managed/BYO KMS and private AI routing, proof of no hosted credential or evidence fallback, gated device smoke |
| build/OCI/CI provenance | project/Git/YAML/OCI providers and evidence hashes |

The final HA, restore, live-network, and private-device profiles are environment-backed system verification. They can still be authored in `.tnt`, but reports must not call them hermetic. This matrix is not itself an implementation owner: plan Slices 18P/18A own streaming/event sources, 19A owns KMS/spool fixtures, 19B owns bounded load, and 20P/20A/20B own recovery/topology/fault feasibility and providers. Tasks 18B, 19C, and 20C cannot begin until those exact dependencies land. The agent/tool row remains a pressure requirement only and is excluded from those releases until DD-065 gains an immutable source design, exact owner/contract, and landed implementation identities.

---

## 24. Compatibility and migration

1. Existing `.intent` syntax remains parseable.
2. Strict IDs and evidence are opt-in initially, then become the default for project-wide `intent check` at the next feature boundary.
3. Existing coverage remains available as `implementation coverage`; its label changes before thresholds change.
4. Existing simple HTTP/function scenarios are translated into the new action model and must preserve behavior except where old behavior passed unsupported assertions.
5. Unsupported assertions change from pass-with-message to fail/unsupported. This is an intentional correctness fix.
6. Old `setup` technical bindings remain parsed but produce a warning and never gain arbitrary execution semantics.
7. Studio consumes the new report but is not an implementation prerequisite for the runner.
8. Provider/report schemas are versioned. Ntnt supports at least the current and previous report schema for reading/replay; execution uses the current provider protocol.
9. Platform-specific unsupported providers are blocked during planning, not skipped after expensive resource startup.

---

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

## 26. Open implementation questions

These are implementation decisions, not permission to weaken the architecture:

1. Whether project-wide execution remains under `ntnt intent check` alone or gains a future `ntnt verify` alias. The canonical first command is `ntnt intent check .`.
2. Whether a later language release introduces native `test fn` syntax. V1 comment metadata is fixed as `@test`, `@verifies`, `@uses`, `@tags`, `@fixture`, `@scope`, `@teardown`, and `@fixtures`.
3. Whether browser CDP ships in core or as an ntnt-maintained signed provider package. The project-facing API and evidence contract remain the same.
4. Exact resource-limit defaults after performance measurement.
5. How much virtual-time control can be safely exposed to managed app processes without creating a production footgun.
6. Which static project facts belong in core versus optional maintained providers.
7. Provider package discovery and lockfile format, coordinated with DD-062.

---

## 27. Rejected alternatives

### Run the existing scripts from Intent

Rejected. It preserves ambient authority, shell portability problems, hidden setup, weak evidence, and unreliable cleanup.

### Put SQL directly in `.intent`

Rejected. Intent is an obligation layer, not a privileged database script. SQL may remain in migrations or bounded `.tnt` provider calls under explicit database authority.

### Treat every skipped scenario as passing

Rejected. A precondition mismatch is useful diagnostic data, not evidence that the promised behavior holds.

### Build a universal YAML workflow engine

Rejected. Ntnt needs a bounded verification planner and resource graph, not a second general-purpose CI language.

### Keep specialist tests forever and only import JUnit

Rejected as the target for ntnt application projects. Imported evidence is useful, but it does not achieve project-owned pure ntnt verification. Maintained providers should expose the specialist engine through `.tnt` where feasible.

### Bundle a browser engine or database server into the ntnt binary

Rejected. Ntnt owns lifecycle and evidence; external systems retain their own release and security boundaries.

---

## 28. Delivery

Implementation is split into reviewable, test-first PRs in the companion plan:

[DD-078 implementation plan](../plans/dd-078-intent-verification-implementation.md)

The design PR authorizes no production implementation by itself. Each runtime slice needs its own focused PR, security review proportional to new authority, full regression gates, generated-document truth sync, and a project-neutral acceptance corpus. Consumer migrations and their deletion gates land in separate adoption PRs against pinned runtime commits; Larrimon is the first such consumer, not a privileged runtime mode or core completion gate.
