# DD-078 Larrimon Reference-Adoption Plan

> **Status:** Consumer migration plan. This file does not participate in the DD-078 core dependency DAG, release sequence, or definition of done.

**Consumer repository:** [`larimonious/larrimon`](https://github.com/larimonious/larrimon)

**Immutable audit baseline:** commit `ceadfd992d1435ac27afb054968ff5569d697ce1`, recorded in [`dd-078-larrimon-baseline.md`](dd-078-larrimon-baseline.md)

**Runtime architecture:** [`../design-docs/dd-078-intent-verification-runtime.md`](../design-docs/dd-078-intent-verification-runtime.md)

**Core implementation plan:** [`dd-078-intent-verification-implementation.md`](dd-078-intent-verification-implementation.md)

## 1. Boundary

Larrimon is the first demanding consumer of DD-078's generalized verification runtime. It supplies application-specific inventories, invariant families, migration waves, deletion authority, and pressure cases. It does not define ntnt public APIs, schemas, keywords, defaults, fixture semantics, policies, privileged modes, or core release boundaries.

This plan may begin only from pinned, reviewed ntnt commits. Larrimon migration PRs must not patch ntnt runtime behavior. Missing generalized capability returns to a separately reviewed ntnt slice; it is never implemented as project-owned shell, Python, or compensating JavaScript.

Completion of this plan proves one reference adoption. Failure or delay here does not make the project-neutral DD-078 runtime incomplete, and completion here does not by itself prove the runtime generally correct.

## 2. Adoption protocol

Every deletion requires a checked-in old-to-new invariant ledger containing:

1. old file and exact line/range;
2. stable invariant ID;
3. replacement obligation and case IDs;
4. environment and resources;
5. expected positive result;
6. deliberate violation, mutation, or fault witness;
7. retained versioned evidence digest;
8. exact candidate repository commit and canonical inventory digest.

Deletion additionally requires:

- old and new checks on the same clean immutable revision whenever technically runnable;
- a reviewer-approved alternative witness for every non-dual-runnable invariant;
- equivalent or stronger negative, race, cleanup, security, timing-boundary, and failure-injection coverage;
- no claim derived from annotation count, declaration count, filename count, or user-authored pass data;
- protected contract, execution snapshot, baseline inventory, and evidence bound to the same identity;
- narrowly scoped migration PRs that delete only the proven replacement slice.

## 3. Core-capability consumption gates

These are Larrimon gates against landed DD-078 capabilities. They are not core ntnt release gates.

| Core owner | Larrimon consumer gate |
|---|---|
| 1A–1B | Run current Intent files through the static ledger and record declared, documentation-only, linked, unbound, executable, and verified obligations. Delete nothing. |
| 2A–2G | Add a non-executing draft `[verification]` section on a separate Larrimon branch and prove complete resource/test planning without startup. Merge only after the consuming runtime commit is pinned. |
| 3A–3C | No consumer execution until the authority floor is landed and externally granted. |
| 3D | Convert representative reducer, validation, probe-shape, and application-service files to discovered `.tnt` cases. After all 18 current direct `ntnt run` cases have same-revision parity, remove their manual print/pass conventions and the 18-run loop from `tests/intent.sh`. |
| 3E | Replace repeated scalar seed/setup builders in pure `.tnt` cases. Resource-backed database/auth fixtures wait for PostgreSQL and application-fixture owners. |
| 4 | Move validation matrices and reducer golden streams into typed data fixtures without growing hand-written assertion helpers. |
| 5A–5B | Migrate public health, headers, origin/HTMX, form fallback, auth request/consume, cookie, role, and multiple-identity cases. Keep server lifecycle in the old harness until process resources land. |
| 6A–6B | Move server/config/startup-failure and authenticated HTTP suites to manifest-managed application resources. Remove equivalent process, port, wait, and curl helpers only after parity. |
| 7B | Replace redirect/resend HTTP mocks and webhook receiver programs after mutation parity. |
| 7C | Replace the project-owned SMTP capture program after magic-link and alert-delivery mutation parity. |
| 8 | Migrate schema, migration, checksum, RLS, security-definer, immutability, tenant-isolation, rollback, and role cases. Application/schema SQL test files may be deleted after same-revision positive, negative, race, cleanup, and mutation parity; production migration programs remain until adoption Slice 16M. |
| 9 | Migrate magic-link email, queue wakeup/reconciliation, alert delivery, and resource-cleanup cases. |
| 10A–10C | Replace sleeps and poll loops for queued/running/terminal runs, readiness, session revocation, alert delivery, and scheduler recovery. |
| 11A | Port every background/FIFO/parallel race case. No sleep-based race is deleted until the replacement deterministically forces the intended interleaving. |
| 12A–12B | Rewrite reconciliation and staging browser smoke in `.tnt`; cover desktop/mobile, HTMX/full-page, no-JavaScript, focus, URL, abort/replacement, mutation ambiguity, and authentication. Remove project test `.js`/`.mjs` only after parity. |
| 13A–13D | Port architecture, CI policy, assets, runtime/image provenance, and Compose/project assertions. Dual-run migration-checksum facts but retain the migration-checksum program until Slice 16M. |
| 14A–14B | CI uploads one versioned JSON report and optional JUnit; no shell post-processing infers coverage or status. |
| 14C | Port every state/transition invariant and mutation from `scripts/staging-state.py` and `tests/staging_state_cases.py`; preserve worktree identity, legacy-state upgrade, restrictive permissions, and foreign-state rejection. Deletion also waits for 14D environment parity. |
| 14D | Replace `scripts/dev-up.sh`, `dev-down.sh`, `staging-up.sh`, and `staging-down.sh` only after same-revision positive, negative, partial-failure, cleanup, and mutation parity. `staging-smoke.sh` remains until its HTTP/browser evidence also migrates. |

## 4. Wave A — Intent truth and native cases

**Depends on landed ntnt owners:** 1A–1B, 2A–2G, 3A–3E, 4

1. Add stable scenario/outcome IDs and missing behavioral scenarios to all seven audited `.intent` files.
2. Add or convert `.tnt` cases under the project verification tree.
3. Add pinned profiles to `ntnt.toml`.
4. Run old and new tiers together temporarily.
5. Add the under-covered wrong-tenant session, readiness dependency failure, internal dispatch denial, request-path no-probe, persistence rollback, concurrent projection serialization, enqueue-failure reconciliation, and audit-immutability contracts.
6. Remove manual assertion/pass wrappers only after invariant-level parity.

**Wave gate:** Every audited obligation is verified or explicitly documentation-only, corrected, or superseded with rationale. `@implements` coverage is never behavioral evidence.

## 5. Waves B–D — HTTP, database/jobs, concurrency, and browser

**Depends on landed ntnt owners:** 5A–12B plus the required landed DD-077 transport and migration owners

Use separate focused Larrimon PRs:

1. HTTP/auth/server conversion.
2. PostgreSQL/migration/RLS conversion.
3. Jobs/eventual/restart conversion.
4. Deterministic concurrency conversion.
5. Browser/reconciliation conversion.

For each PR:

1. inventory old cases with stable invariant IDs;
2. add failing `.tnt` equivalents;
3. run old and new checks on the same clean database, runtime, and image;
4. compare positive, negative, timing, cleanup, and race behavior;
5. inject representative semantic faults and prove both checks detect them with useful evidence;
6. delete only the replaced slice;
7. update Intent bindings and architecture/testing documentation.

**Wave gate:** No project-owned Node/Playwright test files or non-migration SQL-only application test files remain after Wave D. Production JavaScript and SQL migrations remain. Production migration helpers remain until Slice 16M passes.

## 6. Adoption Slice 16M — production migration compatibility

**Consumer dependency only:** landed DD-077 PR 1C, landed DD-078 owner 8, and the Larrimon database-conversion wave.

This slice is intentionally absent from the DD-078 core dependency table and releases.

Run old migration checks and native `ntnt db`/`.tnt` evidence on one immutable Larrimon revision across:

- fresh install and idempotent rerun;
- every supported legacy ledger and application/schema upgrade pair;
- checksum backfill and pre-package unverifiable rows;
- unknown-ledger rejection before mutation;
- malformed or missing manifests;
- missing or mutated applied files;
- database checksum enforcement;
- concurrent migrators and advisory locks;
- per-migration rollback and dirty recovery;
- cancellation and role configuration.

Inject failures/mutations for every family and retain paired reports.

**Exclusive deletion authority:** Only this consumer slice may authorize removal of:

- `scripts/migrate.sh`;
- `scripts/migrate-prod.sh`;
- `scripts/check-migration-checksums.py`;
- `tests/migrate_prod_integration.sh`.

DD-078 owner 8 may provide observations but cannot authorize these deletions. A later operational matrix may expand supported cases, but the currently supported production matrix cannot be deferred past deletion.

## 7. Wave E — project policy and one-command CI

**Depends on landed ntnt owners:** 13A–14D, adoption Slice 16M, and prior Larrimon waves

1. Convert Python/static/provenance checks to `.tnt`.
2. Convert project-owned operational support outside the historical test tree to ordinary `.tnt` CLI programs or direct typed ntnt/provider commands.
3. Replace suite shell wrappers with `ntnt.toml` profiles.
4. Reduce CI to pinned setup/install/verify actions invoking named ntnt profiles through typed inputs; no project wrapper or inline script block.
5. Update `README.md`, `ARCHITECTURE.md`, and testing documentation.

The immutable path/range/line/blob inventory remains [`dd-078-larrimon-baseline.md`](dd-078-larrimon-baseline.md). Regenerate it from the exact candidate base before deletion and require its digest, protected-contract base, and execution-snapshot base to match.

## 8. Audited replacement destinations

| Current Larrimon files | Required native destination |
|---|---|
| `tests/{all,fast,intent,db}.sh` | `ntnt.toml` profiles and pinned typed CI action entries for `ntnt intent check` |
| `tests/integration.sh`, `tests/server-smoke.sh`, `scripts/staging-smoke.sh` | linked `.tnt` HTTP/database/process/browser cases under the corresponding profile |
| `tests/migrate_prod_integration.sh` | adoption Slice 16M after the landed DD-077 migration owners |
| `scripts/{migrate,migrate-prod}.sh` | landed DD-077 `ntnt db` migration/apply/verify commands plus linked migration evidence |
| `scripts/{dev-up,dev-down,staging-up,staging-down}.sh` | landed 14C–14D typed `ntnt project env` state and OCI lifecycle |
| `scripts/staging-state.py`, `tests/staging_state_cases.py` | landed 14C state/lock/lease substrate, landed 14D OCI allocation, and `.tnt` cases |
| `scripts/check-migration-checksums.py` | owner 8 observations plus adoption Slice 16M compatibility; only 16M authorizes deletion |
| `tests/{architecture_cases,ci_cases,assets_provenance,runtime_provenance,runtime_image_provenance}.py` | `std/test/project`, Git/YAML/OCI/migration facts, and linked `.tnt` constraints |
| `tests/{smtp_mock,redirect_mock,resend_mock}.py` | landed typed SMTP and scripted HTTP fixture providers |
| `tests/reconciliation_cases.js`, `tests/staging-browser-smoke.mjs` | landed `std/test/browser` `.tnt` cases; production `public/larrimon.js` remains product code |
| `tests/{assertions,probe_run_state_fixture,security_definer_tenant_case}.sql` | typed PostgreSQL assertions/seed fixtures and role/RLS/security-definer `.tnt` cases; deleted before 16M |

## 9. Consumer release sequence

These are Larrimon milestones, not DD-078 releases:

1. **Intent truth:** pinned owners 1A–4 plus Wave A.
2. **HTTP and process migration:** pinned owners 5A–7F plus Wave B.
3. **Database, jobs, and concurrency migration:** pinned owners 8–11A plus Wave C.
4. **Browser migration:** pinned owners 12A–12B plus Wave D.
5. **Project/environment migration:** pinned owners 13A–14D.
6. **Production migration compatibility:** adoption Slice 16M.
7. **Pure-project claim:** Wave E after every required prior milestone.

A milestone starts only from exact landed ntnt commit identities. None of these milestones blocks a core DD-078 release.

## 10. Larrimon definition of done

This consumer adoption is complete when:

- all 27 audited scenarios and 38 assertion/outcome lines are verified, corrected, superseded, or explicitly documentation-only; none vanish silently;
- every shell, Python, JavaScript test, and SQL-only application-test invariant has a destination and same-revision parity evidence;
- representative semantic mutations/faults prove detection before each old file is deleted;
- `tests/intent.sh` and suite wrappers are removed;
- project-owned `.sh` and `.py` support/orchestration files are zero, except operator-locked externally owned non-support artifacts;
- project-local browser/reconciliation test `.js`/`.mjs` and SQL-only application-test files are zero;
- typed project-state and `ntnt project env` replace dev/staging lifecycle programs with allocation, failure, cleanup, and mutation parity;
- the baseline inventory, protected contract, candidate base, execution snapshot, and evidence bind the same exact Larrimon commit and canonical digest;
- fast and full profiles run through ntnt with current verified coverage at the configured threshold;
- specialist external resources remain pinned, capability-scoped, and visible in reports;
- the complete old-to-new invariant ledger and mutation/fault witnesses remain in project history.

Expected end-state commands:

```bash
ntnt intent lint .
ntnt intent plan . --profile full --json
ntnt intent check . --profile fast
ntnt intent check . --profile full --report-json verification-report.json
```

Environment-backed protected profiles remain operator-selected outside the checkout.
