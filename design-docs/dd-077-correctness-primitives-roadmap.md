# DD-077: Correctness Primitives for Durable Applications

**Status:** Draft / implementation roadmap
**Author:** Larri + Josh
**Created:** 2026-07-24
**Origin:** Larrimon DD-001 architecture review
**Related:** [DD-037: Concurrency and Jobs](dd-037-concurrency-and-jobs.md), [DD-046: `std/net`](dd-046-std-net.md), [DD-051: Job Rate Limiting and Concurrency](dd-051-rate-limiting-concurrency-pause.md), [DD-052: Job System Enterprise Features](dd-052-job-system-enterprise-features.md), [DD-058: Stdlib Gaps](dd-058-stdlib-gaps.md), [DD-060: AI-Native Developer Experience](dd-060-ai-native-developer-experience.md), [DD-062: Secure Compiled Extensions](dd-062-secure-compiled-extension-libraries.md), [DD-063: Language Assessment](dd-063-language-assessment.md)

---

## Summary

Larrimon is a useful pressure test for ntnt because it is not merely a CRUD web application. It combines immutable observations, replayable reducers, durable jobs, multi-tenant PostgreSQL state, external monitoring nodes, outbound-network policy, high-volume ingestion, alert delivery, and production operations.

Ntnt already covers much of the visible application surface:

- safe network probes through `std/net`;
- durable jobs with retries, priorities, deduplication, rate/concurrency limits, pause/resume, cancellation, scheduled/delayed work, and batches;
- PostgreSQL pooling, parameterized queries, and manual transactions;
- auth/session primitives;
- declarative input validation;
- structured logging;
- opaque provider-backed `Secret` values;
- concurrency primitives and worker isolation.

The remaining gaps are not primarily more domain functions. They are **correctness-preserving connective tissue**: abstractions that make combinations of existing features safe under crashes, retries, multiple tenants, multiple replicas, untrusted targets, and large data volumes.

This DD proposes eight focused additions:

1. Scoped PostgreSQL transactions and a transactional outbox.
2. Stronger distributed-work primitives: leases, fencing, keyed limits, leadership, draining, and backpressure.
3. Opaque outbound-network capabilities shared by `std/http` and `std/net`.
4. Strict, nested, versioned data contracts.
5. PostgreSQL cursor/streaming, batch, and bulk-ingestion primitives.
6. Scoped runtime context, metrics, tracing, and health registration.
7. A real raw-SQL database migration runner.
8. `pure fn`, effect-aware checking, static exhaustive matches, and reducer helpers.

This document deliberately does **not** authorize implementation. It defines the product boundary, proposed contracts, dependencies, PR slices, and acceptance gates for later review.

---

## Why These Features Belong in ntnt

A feature is language-, runtime-, or stdlib-worthy when it satisfies most of the following:

1. **It recurs across unrelated applications.** The feature is not merely one product's domain model.
2. **It encodes a difficult correctness or security invariant.** Ordinary application conventions are likely to get it subtly wrong.
3. **The runtime can enforce the invariant better than application code can.** The abstraction removes unsafe paths rather than adding another helper beside them.
4. **It materially reduces agent-written boilerplate and repair loops.** One paved road replaces many nearly-correct local implementations.
5. **Its semantics can remain small, explicit, and testable.** The abstraction does not become a framework-shaped fog bank.
6. **It composes with existing ntnt primitives.** New work strengthens the current language instead of creating parallel systems.
7. **Failure behavior can be specified precisely.** Crashes, retries, timeouts, cancellation, partial results, and ownership loss are part of the contract.

### Placement test

| If the runtime can enforce... | Prefer |
|---|---|
| syntax, type, control-flow, purity, or opaque authority | language/runtime feature |
| a reusable I/O or lifecycle contract | stdlib primitive |
| a product's policy, schema, or domain transitions | application/private library |
| provider-specific integration behind a stable seam | optional extension/provider module |

### Explicit product boundary

This DD does not add monitoring-specific syntax or types. Ntnt should not gain `monitor`, `incident`, `dependency`, `root_cause`, or `tenant` keywords. Larrimon owns those concepts. Ntnt owns the reusable safety boundaries beneath them.

---

## Goals

- Remove common crash windows between canonical database state and asynchronous side effects.
- Make transaction cleanup, transaction-local settings, and nested transaction behavior safe by construction.
- Give distributed workers explicit leases, heartbeat, ownership loss, fencing, fairness, and drain semantics.
- Replace process-global outbound-network trust switches with opaque, deployment-issued authority.
- Validate complex payloads once and reuse the contract across HTTP, jobs, AI, and documentation.
- Process large PostgreSQL datasets with bounded memory and efficient ingestion.
- Propagate safe request/job/run context through logs, traces, jobs, and health diagnostics without leakage.
- Give production ntnt applications a reliable raw-SQL migration lifecycle.
- Let the language prove reducer purity and enum handling earlier than runtime.
- Preserve ntnt's synchronous, batteries-included, free-function-oriented programming model.
- Deliver each capability in small, reviewable PRs with compatibility and failure-matrix tests.

## Non-goals

- No implementation in this DD.
- No ORM or generated application schema.
- No monitoring, incident-management, topology, or carrier-device framework in the default stdlib.
- No exactly-once job or event-delivery claim.
- No general workflow DSL.
- No automatic database partition policy.
- No dynamic tenant-secret management API; deployment secrets remain under `std/secrets` and provider boundaries.
- No autonomous AI tool-loop or structured-inference work in this roadmap.
- No HTTP abuse-rate-limiting middleware in this roadmap; it remains useful follow-up work but is not one of the selected eight items.
- No requirement to complete all eight tracks before an application can begin. Each shipped slice must provide standalone value.

---

## Current Baseline and Confirmed Gaps

| Area | Current baseline | Gap addressed here |
|---|---|---|
| PostgreSQL transactions | Manual `begin()`, `commit()`, `rollback()` on a pinned logical connection | Scoped cleanup, local settings, nested behavior, durable outbox |
| Jobs | Retries, priority, dedup, scheduled/delayed work, rate/concurrency limits, pause, cancellation, batches | Heartbeat, fencing, keyed controls, cluster-safe periodic scheduling/leadership, draining, queue admission |
| HTTP SSRF policy | Initial URL and resolved-address validation with process-global configuration | Opaque policy authority, connection binding, redirect revalidation, shared policy engine |
| Validation | Field-rule maps returning cleaned maps | Nested/strict/versioned contracts, bounded decode, JSON Schema export |
| PostgreSQL queries | Fully materialized query arrays and single-statement execute/query APIs | Cursor batches, `COPY`, batched execution, cancellation and cleanup |
| Operations | Structured logs and request logger | Scoped context, metrics, traces, readiness/liveness registration |
| Migrations | `ntnt migrate` is a source-syntax migration command, not a DB migration system | Checksummed raw-SQL migration lifecycle under `ntnt db ...` |
| Determinism | Runtime contracts plus existing typechecker/runtime exhaustiveness checks for enums, `Option`, and `Result`; guard and duplicate-arm handling remain incomplete | Effect checking, guard-aware exhaustiveness, unreachable/duplicate diagnostics, reducer helpers |

## Release Targeting

As of 2026-07-24, v0.5.1 release-readiness PR #166 is merged into `main`, `Cargo.toml` is already `0.5.1`, all release checks are green, there are no open PRs, and only the tag/release remain pending. **No DD-077 runtime feature should be added to v0.5.1 now.** The patch release has reached its freeze point; reopening it for an interpreter, PostgreSQL, job, network, validation, or health subsystem would invalidate the compatibility pass that just completed.

Work permitted before the v0.5.1 tag is limited to documentation and non-shipping design evidence:

- [x] finish DD-077 review corrections and release slicing;
- [ ] perform Wave 0B's transport-binding spike without production code;
- [ ] perform Wave 0C's effect-classification inventory without syntax/runtime changes;
- [ ] triage a newly proven security defect if one is discovered. Any such fix must be narrowly scoped, tested against v0.5.0 compatibility, and rerun the full v0.5.1 release-readiness matrix. It is not permission to land the complete network-capability track.

Recommended implementation targets:

| DD-077 slice | Earliest sensible target | Rationale |
|---|---|---|
| Wave 0A callback bridge + scoped `Tx` from item 1 | **v0.5.2 primary candidate** | Highest-leverage correctness primitive, additive public API, but it touches interpreter callback and PostgreSQL ownership internals and needs a full feature-release test matrix |
| Item 1 transactional outbox | **v0.5.3+** | Depends on scoped transactions, migration support, and atomic/reconcilable job acceptance |
| Item 2 distributed job hardening | **v0.5.3+** | Cross-backend lease, fencing, leadership, drain, and backpressure semantics are too broad for the pending patch |
| Item 3 network capabilities | **v0.5.3+** | Requires trusted configuration and policy-bound transport; a narrowly proven vulnerability fix may ship earlier without the new capability API |
| Item 4 strict nested schemas | **v0.5.2 candidate** | Stage 1 can be additive; typed decode and purity-proven upcasters remain later slices |
| Item 5 PostgreSQL streaming/COPY | **v0.5.2 candidate after callback/transaction foundations** | Additive and useful, but cursor callbacks and bounded-memory claims depend on settled resource ownership |
| Item 6 context/metrics/tracing/health | **v0.5.3+** | Runtime context propagation and native health isolation cut across HTTP and job workers |
| Item 7 database migrations | **v0.5.2 or v0.5.3 feature slice** | Additive CLI, but cluster locking and dirty non-transactional lifecycle deserve their own release-sized implementation |
| Item 8 match hardening | **v0.5.2 candidate** | Improves an existing checker; guard/unreachable diagnostics can change strict-mode outcomes and therefore do not belong in v0.5.1 |
| Item 8 effect system and `pure fn` | **v0.6.0 candidate** | New syntax and transitive static semantics deserve a language-feature release rather than a patch/minor tail-end addition |

For v0.5.2, choose one primary runtime track rather than packing every candidate into the release. The recommended first track is **Wave 0A plus scoped PostgreSQL transactions**. Strict schemas, PostgreSQL streaming/COPY, migration tooling, and match hardening are independently useful follow-on candidates, not automatic passengers.

## Relationship to Existing DDs

- **DD-037 and DD-052 remain the job-system history for shipped behavior.** DD-077 item 2 and Wave 2F–2H supersede DD-052's unimplemented sections 4 (Rolling Restarts) and 5 (Leader Election), because the newer design adds backend boundaries, renewable leases, fencing, cooperative ownership loss, schedule idempotency, and drain semantics. Merged job PRs must truth-sync both records rather than maintaining two future authorities.
- **DD-046 remains the shipped `std/net` baseline.** DD-077 adds shared authority and transport-binding semantics; it does not reopen the monitoring-protocol scope.
- **DD-058 remains the general stdlib gap inventory.** DD-077 deepens its shipped validation work into durable boundary contracts.
- **DD-060 provides the AI-native product rationale.** DD-077 turns part of that rationale into concrete runtime safety work.
- **DD-063 remains the language assessment.** Its distinction between shipped capabilities and aspirational effects is preserved; `pure fn` stays unshipped until Wave 4 passes its design and coverage gates.

When implementation begins, status and checkboxes must be truth-synced in the owning DD as well as this roadmap. DD-077 is the portfolio-level sequence, not a replacement history for every subsystem.

---

# 1. Scoped Transactions and Transactional Outbox

## Problem

Manual transaction calls require every application to correctly handle:

- rollback on every error and early-return path;
- transaction-local RLS context;
- statement and lock timeouts;
- nested transaction attempts;
- connection-pool cleanup;
- commit uncertainty;
- the crash window between a committed state transition and queue/email/webhook delivery.

A correct application must never publish asynchronous work before the state it references commits, and must not lose the work after commit. Calling `enqueue()` after `commit()` leaves a crash gap. Calling it before `commit()` lets work observe uncommitted or rolled-back state.

## Proposed surface

Initial API should be a stdlib function, not new syntax:

```ntnt
import { with_transaction } from "std/db/postgres"
import { outbox_emit } from "std/outbox"

let incident = with_transaction(db, map {
    "isolation": "serializable",
    "lock_timeout_ms": 1000,
    "statement_timeout_ms": 5000,
    "set_local": map {
        "app.tenant_id": tenant_id
    }
}, fn(tx) {
    let incident = create_incident(tx, input)?

    outbox_emit(tx, "incident.opened", map {
        "incident_id": incident.id,
        "tenant_id": tenant_id
    }, map {
        "idempotency_key": "incident.opened:#{incident.id}"
    })?

    return incident
})?
```

The outbox relay should be explicit process work:

```ntnt
import { configure_outbox, work_outbox } from "std/outbox"

configure_outbox(map {
    "database": db,
    "dispatcher": "jobs",
    "routes": map {
        "incident.opened": "DeliverIncidentAlert"
    },
    "poll_interval_ms": 250,
    "batch_size": 100
})

work_outbox()
```

`dispatcher: "jobs"` maps each outbox topic to one explicitly configured job route. It does not depend on the unshipped `std/events` design. Future dispatchers may be extensions, but v1 should have one narrow path rather than a provider abstraction pageant.

## Required guarantees

### `with_transaction`

- The callback receives a distinct opaque, generation-bound `Tx` handle, not another clone of the current map-shaped base connection handle.
- Query/execute functions accept `Tx`, but ordinary values cannot forge or deserialize it.
- Commit only when the callback returns successfully.
- Roll back on `Err`, `?` propagation, runtime error, contract violation, or callback failure.
- Always release the pinned pool connection.
- Invalidate the `Tx` generation on callback exit; escaped aliases fail deterministically.
- While a scoped transaction is active, operations through any alias of its base connection are rejected instead of silently joining the transaction.
- Manual `commit()` and `rollback()` reject scoped transactions; only `with_transaction()` owns their terminal state.
- Validate isolation level and timeout bounds; never interpolate them into SQL unchecked.
- Apply `set_local` through parameter-safe `set_config(..., true)` or an equivalent safe mechanism.
- Prevent transaction-local values from leaking to the next pooled borrower.
- Define nesting against the scoped handle. Recommended v1: `with_transaction(tx, ...)` creates a savepoint; calling `with_transaction(base_connection, ...)` while that base connection already has an active scoped transaction is rejected. A different base connection is independent.
- Preserve the original failure if rollback also fails, while attaching rollback failure as secondary diagnostic context.
- Report commit failure as indeterminate when PostgreSQL cannot prove whether commit reached the server.

### Outbox

- Insert outbox records in the caller's transaction.
- Delivery is at-least-once.
- Every record has a stable ID and optional unique idempotency key.
- Claims use `FOR UPDATE SKIP LOCKED` or an equivalent race-safe mechanism.
- Claims have lease/attempt metadata and stale-claim recovery.
- Dispatch success and retry/dead state changes are transactional.
- Payloads reject nested `Secret` values.
- Payload and metadata size are bounded.
- Poison messages move to a bounded dead state with operator-visible diagnostics.
- The jobs dispatcher is unavailable until ntnt has an atomic/reconcilable job-acceptance primitive. A deterministic ID alone is insufficient because the current enqueue path writes its dedup key, job record, and pending index separately.
- Job acceptance must atomically commit the idempotency key, job record, and pending index: one SQLite transaction for the SQLite backend and one Redis/Valkey Lua transaction for the Redis backend. It must detect and repair legacy/orphan dedup records rather than reporting a nonexistent job as accepted.
- After atomic acceptance exists, the relay derives a stable downstream enqueue key from the outbox record and retries until acceptance is confirmed.
- No exactly-once claim. Downstream consumers remain idempotent.

## Storage decision

Recommended v1: ntnt owns a versioned internal PostgreSQL schema installed by the migration runner:

```sql
CREATE SCHEMA IF NOT EXISTS _ntnt;

CREATE TABLE _ntnt.outbox_v1 (
    id UUID PRIMARY KEY,
    topic TEXT NOT NULL,
    payload JSONB NOT NULL,
    idempotency_key TEXT,
    state TEXT NOT NULL,
    available_at TIMESTAMPTZ NOT NULL,
    claimed_by TEXT,
    claim_token BIGINT,
    claim_expires_at TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    UNIQUE (idempotency_key)
);
```

The final schema needs a dedicated review. This sample is a starting point, not approved migration SQL.

## Implementation starting points

- Modify `src/stdlib/postgres.rs` for scoped transaction lifecycle, base-handle exclusion, generation checks, and callback plumbing.
- Add an opaque `Tx` runtime value in `src/interpreter.rs`/`src/types.rs`; do not model scoped authority as another cloneable connection map.
- Modify `src/interpreter.rs` to provide one reusable, internal closure-invocation bridge for native stdlib functions instead of adding another one-off special case.
- Create `src/stdlib/outbox.rs`.
- Modify `src/stdlib/mod.rs` and `src/typechecker.rs` for module registration and signatures.
- Add PostgreSQL integration tests under a new `tests/postgres_correctness_tests.rs` or a focused test module with disposable PostgreSQL setup.
- Update `docs/STDLIB_REFERENCE.md` through the generated-doc path and add examples to `docs/AI_AGENT_GUIDE.md`.

## Acceptance criteria

- [ ] Callback success commits exactly once.
- [ ] Every callback failure shape rolls back and returns the original error.
- [ ] Pool reuse after success and failure shows no leaked transaction or local setting.
- [ ] Base-connection aliases cannot join, commit, or roll back an active scoped transaction.
- [ ] Escaped and stale-generation `Tx` handles fail after scope exit.
- [ ] Nested savepoints work only through the active `Tx` handle and preserve outer ownership.
- [ ] Commit-disconnect ambiguity has a distinct error class/message.
- [ ] Outbox insert rolls back with application state.
- [ ] Duplicate idempotency key produces one logical record.
- [ ] Two relays cannot own the same claim token concurrently.
- [ ] Relay crash before dispatch, during dispatch, and after downstream acceptance is tested.
- [ ] Secret-bearing and oversized payloads are rejected without value leakage.

---

# 2. Distributed Work: Leases, Fencing, Keyed Limits, Leadership, Draining, and Backpressure

## Problem

The job system is strong for ordinary background work but lacks several primitives required by multi-replica systems:

- long-running work cannot refresh a first-class visibility lease;
- ownership loss does not produce an application-visible fencing token;
- rate/concurrency limits are definition-oriented rather than keyed by tenant, node, account, or upstream;
- one scheduler per cluster requires leadership rather than one scheduler per process;
- rolling deploys need stop-claiming-and-drain semantics;
- unbounded queue growth has no explicit admission contract.

## Proposed surface

### Generic lease

```ntnt
import { with_lease } from "std/jobs"

with_lease("scheduler:observations", map {
    "ttl_ms": 30000,
    "heartbeat_ms": 10000,
    "fencing": true
}, fn(lease) {
    run_scheduler(lease.fencing_token)
})?
```

### Job heartbeat and ownership

```ntnt
job BuildRollup on maintenance (
    timeout: 900,
    lease: 60,
    heartbeat: 15
) {
    perform(partition_id: String) {
        // Runtime renews while the job is actively executing.
        // job_fencing_token() is available for guarded writes.
        build_rollup(partition_id, job_fencing_token())
    }
}
```

### Keyed limits

```ntnt
job RunProbe on probes (
    concurrency: 20,
    concurrency_by: "node_id",
    rate: "300/minute",
    rate_by: "tenant_id"
) {
    perform(run_id: String, tenant_id: String, node_id: String) {
        execute_probe(run_id)
    }
}
```

### Queue admission and drain

```ntnt
configure_queue(map {
    "max_pending": 100000,
    "admission": "reject",
    "dead_retention": "7 days"
})
```

```bash
ntnt workers drain --timeout 30
```

## Required guarantees

- Lease renewal is bounded and stops on cancellation/shutdown.
- Loss of lease sets cooperative cancellation/ownership-lost state, observed at documented interpreter and stdlib yield points. Ntnt cannot preempt arbitrary blocking native/database/network work already in progress.
- Fencing tokens are monotonically increasing per lease key.
- The runtime does not claim that a fencing token protects arbitrary external effects. A protected sink must atomically compare/store the token with its state change; stale-write rejection is tested separately.
- Key extraction is declarative from validated payload fields, not arbitrary closures.
- Missing or invalid key fields fail before enqueue or execution according to the declared contract.
- Keyed counters are atomic across workers for Redis/Valkey and SQLite backends.
- Leadership is a lease, not a permanent process role.
- Cron/periodic emission occurs only under current leadership and remains idempotent by schedule occurrence key.
- Drain stops new claims, waits for active jobs, then cancels or requeues after a bounded timeout.
- Admission policy is explicit: reject, delay, or drop may not be silently substituted.
- `enqueue()` reports admission failure as data the caller must handle.
- Dead and completed retention are bounded.

## Backend scope

V1 preserves current Redis/Valkey and SQLite support without pretending they provide identical deployment guarantees:

| Guarantee | Redis/Valkey | SQLite |
|---|---|---|
| Atomic job/keyed-limit operations | Supported across clients through server-side atomic operations | Supported for processes sharing the same database file through SQLite transactions |
| Lease renewal and fencing | Supported across hosts using one reachable deployment store | Supported only inside the explicit shared-file/process-host boundary |
| Cluster leadership | Supported across hosts | Rejected unless every participant demonstrably coordinates through the same supported shared store; ordinary per-host SQLite files are not a cluster |
| Multi-host failure domain | External coordinated store | Not claimed; file availability/locking define the boundary |

Backend capability is exposed explicitly. Cluster mode fails configuration when the selected backend cannot establish the requested coordination boundary; it never degrades to one leader per host silently.

A PostgreSQL job backend is deliberately deferred. The transactional outbox is the initial bridge between canonical PostgreSQL state and the existing job system. Add a PostgreSQL backend only after multiple applications prove it is preferable to that bridge.

## Implementation starting points

- Modify `src/stdlib/jobs.rs` for lease metadata, heartbeats, keyed controls, queue admission, drain state, and public APIs.
- Modify `src/stdlib/kv.rs` for atomic compare/renew/fencing operations shared by supported backends.
- Modify `src/control_socket.rs` and `src/main.rs` for worker drain/status commands.
- Modify job option parsing in `src/parser.rs`, `src/ast.rs`, `src/typechecker.rs`, and interpreter job registration where new declarative options are introduced.
- Add Redis-gated and SQLite-default race tests.
- Extend job streaming events with lease-lost, drain-started, drained, admission-rejected, and leadership-changed events.

## Acceptance criteria

- [ ] Long jobs renew leases and survive beyond the original visibility timeout.
- [ ] A stalled worker loses ownership and cannot successfully renew with an old token.
- [ ] Fencing tokens increase across ownership changes.
- [ ] Keyed limits isolate two tenants/nodes while enforcing each key independently.
- [ ] Multi-process periodic scheduling emits one occurrence per schedule key.
- [ ] Drain completes active work and claims no new jobs.
- [ ] Drain timeout requeues or cancels according to documented semantics.
- [ ] Queue-full behavior is deterministic and visible to the caller.
- [ ] Redis and SQLite behavior matrices are documented and tested.

---

# 3. Opaque Outbound-Network Capabilities

## Problem

Outbound networking is an authority boundary. Process-global flags are too broad for applications that perform both ordinary public fetches and deliberately scoped private-network work.

The current HTTP safety path validates the initial URL and its resolved addresses before constructing a normal `reqwest` request. The next design must make the connection itself obey the validated policy and must revalidate redirect targets. Preflight validation followed by an independent resolver/connect path is not a sufficient DNS-rebinding boundary.

## Proposed model

Network authority is requested by application configuration, granted by trusted deployment configuration, and obtained as an opaque runtime value. Request data and application-controlled working directories cannot mint or widen authority.

Conceptual application `ntnt.toml` request metadata:

```toml
[network.requests.public-probes]
policy = "public-probes"

[network.requests.site-private]
policy = "site-private"
```

Conceptual deployment policy, selected explicitly by the operator rather than discovered from the current working directory:

```toml
[network.policies.public-probes]
schemes = ["https", "http"]
allow_public = true
allow_private = false
deny_metadata = true
ports = [80, 443]
max_redirects = 3
max_response_bytes = 1048576

[network.policies.site-private]
schemes = ["https", "http", "tcp"]
allow_public = true
allow_cidrs = ["10.20.0.0/16"]
deny_cidrs = ["10.20.99.0/24"]
deny_metadata = true
ports = [22, 80, 443, 5432]
```

The runtime grants only the intersection of the application's named request and the deployment policy. In production, the policy path is an explicit startup/deployment input, canonicalized once, loaded before untrusted application code, and required to satisfy documented ownership/permission rules. Symlink/path replacement, CLI/environment precedence, reload behavior, and failure mode are part of PR 2C's configuration-security contract. V1 should load once and fail closed rather than hot-reload authority. A development-only self-grant mode may exist only behind an explicit development environment boundary and must never become the production default.

Ntnt surface:

```ntnt
import { require_net_capability } from "std/net"
import { fetch } from "std/http"

let public_probes = require_net_capability("public-probes")?

let result = fetch(target_url, map {
    "policy": public_probes,
    "timeout_ms": 5000,
    "max_redirects": 3,
    "max_response_bytes": 1048576
})?
```

`NetCapability` must be an opaque `Value` variant or equivalent unforgeable handle. A map carrying the same field names is not authority.

## Shared policy engine

Create one internal engine for `std/http` and `std/net`:

- scheme validation;
- host normalization;
- direct-IP classification;
- DNS resolution and maximum-address bounds;
- allow/deny CIDR evaluation;
- loopback, private, link-local, multicast, unspecified, documentation, and metadata classification;
- port policy;
- redirect target revalidation;
- connection binding to an approved address;
- stable audit reason codes;
- timeout, redirect, response-size, port-count, and fan-out clamps.

The existing `src/stdlib/net/policy.rs` is the likely home for the shared target-policy core, but HTTP-specific redirect and body limits should remain in `std/http` transport code.

## Required guarantees

- Application data cannot create, widen, clone into a broader, or deserialize a capability.
- Application manifests may request named authority but cannot grant it; production grants come only from the explicitly selected trusted deployment policy.
- `require_net_capability()` is not exported until at least one policy-bound transport consumes the resulting handle. Possessing a decorative capability that the transport ignores would be worse than no API.
- Capabilities reject JSON, templates, logs, database parameters, caches, and cross-process serialization unless a future explicit delegation protocol is designed.
- Every address the transport may connect to passes policy.
- The transport connects to an approved address rather than re-resolving without policy binding.
- Every redirect repeats scheme, host, port, resolution, and address checks.
- Metadata endpoints are denied regardless of private-network opt-in.
- IPv4-mapped IPv6 and alternate textual forms cannot bypass classification.
- Proxy configuration cannot silently transfer destination enforcement to an untrusted proxy path.
- Secret-bearing requests preserve the existing HTTPS/direct-loopback development policy and do not weaken network policy.
- Existing safe public `fetch()` behavior remains compatible when no named capability is requested.

## Compatibility posture

Do not make every existing app add a policy declaration immediately. The default capability should represent the current secure public-fetch behavior. Named capabilities are required only to widen or narrow authority deliberately.

Legacy environment flags should become compatibility inputs to the default policy and receive a deprecation plan only after the manifest path is proven.

## Implementation starting points

- Refactor `src/stdlib/net/policy.rs` into the shared policy core.
- Modify `src/stdlib/net/mod.rs`, `probe.rs`, `transport.rs`, and `traceroute.rs` to accept opaque policies where applicable.
- Modify `src/stdlib/http.rs` to use a redirect-denying or per-hop-validating client and approved-address binding.
- Add `NetCapability` handling to `src/interpreter.rs`, `src/types.rs`, serialization guards, and typechecker signatures.
- Add a focused configuration-security layer for explicit deployment-policy selection, canonicalization, ownership/permission validation, request/grant intersection, precedence, and fail-closed startup behavior; current `src/config.rs` is not yet a general trusted-manifest loader.
- Add deterministic loopback DNS/HTTP fixtures covering redirect and rebinding behavior.

## Acceptance criteria

- [ ] Public target through default policy remains compatible.
- [ ] Public target redirecting to loopback/private/metadata is denied before connection.
- [ ] A hostname whose approved preflight answer changes cannot redirect the transport to a denied answer.
- [ ] Mixed public/private answer sets follow one documented policy; recommended: deny when any candidate the client may select is denied.
- [ ] Named private capability reaches only declared CIDRs and ports.
- [ ] User-supplied maps cannot impersonate capabilities.
- [ ] Capabilities cannot cross serialization, job, cache, DB, template, or log boundaries.
- [ ] Secret-bearing request tests remain green.

---

# 4. Strict, Nested, Versioned Data Contracts

## Problem

`std/validate` currently excels at form-style field rules and coercion, but durable systems exchange deeper contracts:

- nested objects and arrays;
- discriminated unions;
- strict unknown-field handling;
- payload versions and upcasters;
- bounded recursive data;
- schemas reused by HTTP, jobs, AI output, documentation, and tests.

Returning another untyped `Map` after shallow validation does not give the typechecker enough information to catch downstream drift.

## Proposed staged surface

### Stage 1: stdlib-composed schemas

```ntnt
import {
    object_schema,
    array_of,
    enum_of,
    literal,
    optional,
    string,
    uuid_string,
    datetime_string,
    float_value,
    any_value,
    decode,
    json_schema
} from "std/validate"

let ObservationV1 = object_schema(map {
    "schema_version": literal(1),
    "probe_run_id": uuid_string(),
    "observed_at": datetime_string(),
    "status": enum_of(["healthy", "degraded", "failed", "unknown"]),
    "latency_ms": optional(float_value()),
    "evidence": array_of(object_schema(map {
        "kind": string(),
        "value": any_value()
    }, map { "strict": true }))
}, map {
    "strict": true,
    "max_depth": 8,
    "max_fields": 64
})

let observation = decode(ObservationV1, payload)?
```

### Stage 2: typed decode requires a separate type-level decision

Do not ship the earlier conceptual `decode_as(Observation, schema, payload)` form as an ordinary stdlib function. Ntnt cannot currently pass a type as a runtime value or derive a dependent return type from a function argument. Typed decode requires a focused compiler API decision—such as generic/type-argument syntax or a compiler intrinsic—after Stage 1 proves the runtime contract. Until then, `decode()` returns the canonical validated map and callers may construct a typed struct explicitly.

Do not begin with new `schema` grammar. Add syntax only if the function-composed API proves materially repetitive and the typechecker integration has a clear contract.

## Required capabilities

- nested object schemas;
- arrays with element schemas and length bounds;
- strict or permissive unknown-key policy;
- literals and enums;
- optional/default/nullable distinction;
- discriminated unions;
- stable nested error paths;
- maximum depth, fields, array length, and string/byte size;
- canonical cleaned output;
- schema version dispatch;
- explicit upcasters between adjacent versions; Wave 1 treats determinism as a documented/tested callback contract, and compiler-enforced purity is added only after `pure fn` ships;
- JSON Schema export for the supported subset;
- schema fingerprint/version metadata for jobs and APIs;
- rejection of `Secret` values in ordinary data schemas unless a specific secret-aware sink contract accepts them.

## Error shape

Errors should be structured and machine-legible:

```ntnt
Err([
    map {
        "path": ["evidence", 2, "kind"],
        "code": "unknown_enum_value",
        "message": "Expected one of: dns, tcp, tls",
        "actual_type": "String"
    }
])
```

Human rendering can summarize this, but the underlying value should not be a map of field to one string. Multiple nested errors may exist.

## Versioning

A versioned contract should dispatch explicitly:

```ntnt
let Observation = versioned_schema("observation", map {
    "1": ObservationV1,
    "2": ObservationV2
}, map {
    "current": 2,
    "upcasters": map {
        "1->2": upcast_observation_v1_to_v2
    }
})
```

Decode should preserve original version metadata for audit while returning the current canonical representation.

## Implementation starting points

- Refactor `src/stdlib/validate.rs` from flat rules into explicit schema node variants while retaining the current API.
- Add schema runtime values to `src/interpreter.rs` only if opaque values materially simplify validation and export; avoid another ordinary map convention if it cannot enforce invariants.
- Defer typed decode until a focused compiler/type-level DD selects a sound API; do not add runtime type reification as an incidental validation feature.
- Add focused tests to `tests/validate_tests.rs` and new typechecker cases to `tests/type_checker_tests.rs`.
- Generate documentation from public `@ntnt` blocks and add boundary examples to `docs/AI_AGENT_GUIDE.md`.

## Acceptance criteria

- [ ] Existing `schema()`/`validate()` behavior remains compatible.
- [ ] Nested paths and multiple errors are stable and machine-readable.
- [ ] Strict schemas reject unknown fields; permissive schemas document preservation or dropping.
- [ ] Resource limits reject adversarial deep/wide payloads before unbounded allocation.
- [ ] Version dispatch and adjacent upcasters are deterministic under replay tests; compiler-enforced purity is explicitly deferred until item 8.
- [ ] JSON Schema output matches runtime acceptance for the supported subset, and schemas containing non-exportable custom predicates return an explicit export error.
- [ ] Stage 1 returns canonical validated maps; typed decode remains blocked on a separate type-level API decision.
- [ ] Job/HTTP examples use one shared schema object rather than duplicated validation.

---

# 5. PostgreSQL Streaming, Batching, and Bulk Ingestion

## Problem

`query()` returns a fully materialized array. That is appropriate for ordinary request queries but unsafe for replaying, exporting, backfilling, or rolling up millions of records. Repeated single-row inserts also waste round trips for high-volume ingestion.

## Proposed surface

### Cursor-style bounded batches

```ntnt
import { each_query_batch } from "std/db/postgres"

each_query_batch(db,
    "SELECT * FROM observations WHERE tenant_id = $1 ORDER BY observed_at, id",
    [tenant_id],
    map {
        "batch_size": 1000,
        "max_row_bytes": 1048576,
        "max_batch_bytes": 8388608,
        "statement_timeout_ms": 30000
    },
    fn(rows) {
        reduce_batch(rows)?
    }
)?
```

### Bulk copy

```ntnt
import { copy_rows } from "std/db/postgres"

let inserted = copy_rows(db, "observations", [
    "tenant_id",
    "probe_run_id",
    "observed_at",
    "result"
], rows)?
```

### Batched statements

```ntnt
let results = execute_batch(db, [
    map { "sql": "UPDATE ...", "params": [...] },
    map { "sql": "INSERT ...", "params": [...] }
], map {
    "transaction": true
})?
```

`execute_batch` is for bounded batches, not arbitrary dynamic SQL concatenation.

## Required guarantees

- Bounded delivered row count and batch byte budget. Driver memory is bounded by the configured batch plus the largest single PostgreSQL row already decoded; `max_row_bytes` rejects oversized decoded rows but cannot prevent the driver from allocating that one row first. The DD does not claim a hard transport-level memory cap PostgreSQL cannot enforce.
- Batch size, per-row decoded bytes, total batch bytes, and input row/byte counts are clamped.
- Early return, `Err`, cancellation, or panic closes the cursor and releases transaction resources.
- The implementation fetches a batch, releases every PostgreSQL client mutex/borrow, and only then invokes the ntnt callback. The callback may issue sequential queries through the same active `Tx` without deadlocking; nested concurrent use remains rejected by transaction ownership rules.
- Cursor ordering is caller-defined; the API does not imply deterministic order without SQL `ORDER BY`.
- Cursor transaction/isolation semantics are explicit.
- `copy_rows` validates table/column identifiers against a strict identifier grammar and quotes them safely.
- Row width and type mismatch errors include row and column indexes without leaking `Secret` values.
- V1 `copy_rows` is atomic: any row failure aborts the copy.
- No hidden partial-success mode.
- Batched calls preserve parameterization.
- Cancellation is checked between batches and during long transport operations where the PostgreSQL client supports it.
- After PR 3B provides the metrics registry, a follow-up integration exposes row count, batch count, elapsed time, and failures without high-cardinality SQL text. Metrics are not a blocker for PR 2A/2B.

## Implementation starting points

- PR 2A depends on PR 0A's callback bridge and PR 1A's scoped transaction ownership model.
- Modify `src/stdlib/postgres.rs`.
- Use `tokio-postgres` cursor/portal or transaction query streaming where lifetime ownership can be made explicit.
- Fetch into the bounded batch, release the client guard, invoke the callback, then reacquire for the next fetch; add a regression proving callback queries through the same `Tx` do not deadlock.
- Use `tokio-postgres` COPY APIs rather than constructing multi-value SQL strings.
- Reuse the scoped transaction callback bridge from item 1 for batch callbacks.
- Add PostgreSQL integration tests for bounded batches, early exit, cancellation, type errors, transaction interaction, and pool reuse.

## Acceptance criteria

- [ ] Replay memory remains bounded by configured batch budgets plus the largest decoded row, independent of total result count.
- [ ] Oversized rows/batches fail with stable diagnostics under the documented post-decode limitation.
- [ ] Early callback exit closes cursor state and reuses the pool connection safely.
- [ ] A callback can issue a sequential query through the same active `Tx` without a held client guard or deadlock.
- [ ] `copy_rows` inserts a valid batch atomically.
- [ ] One invalid row aborts the batch with stable row/column diagnostics.
- [ ] Identifier injection attempts are rejected.
- [ ] Secret values are rejected without plaintext in diagnostics.
- [ ] Existing `query()` and `execute()` behavior remains unchanged.

---

# 6. Scoped Context, Metrics, Tracing, and Health

## Problem

Production work crosses HTTP requests, jobs, outbox records, reducers, and outbound calls. Applications repeatedly pass identifiers or lose correlation entirely. Process-global mutable context risks leaking one tenant or request into the next pooled worker.

Ntnt also has structured logging but no first-class Prometheus/OpenMetrics metrics, OpenTelemetry-compatible spans, or registered readiness/liveness aggregation.

## Proposed surface

### Scoped context

```ntnt
import { with_context, context_value } from "std/context"

with_context(map {
    "tenant_id": tenant_id,
    "request_id": request_id,
    "probe_run_id": run_id
}, fn() {
    execute_probe(run_id)
})
```

### Metrics

```ntnt
import { counter_add, histogram_observe } from "std/metrics"

counter_add("probe_runs_total", 1, map {
    "status": status,
    "check_kind": kind
})

histogram_observe("probe_latency_ms", latency_ms, map {
    "check_kind": kind
})
```

### Tracing

```ntnt
import { with_span, span_field } from "std/trace"

with_span("reduce_observation", fn(span) {
    span_field(span, "observation_id", observation.id)
    reduce(observation)
})
```

### Health

```ntnt
import {
    postgres_health_check,
    jobs_health_check,
    register_readiness_check,
    register_readiness_state,
    set_readiness_state,
    enable_health_routes
} from "std/health"

register_readiness_check("postgres", postgres_health_check(db, map {
    "timeout_ms": 500
}))
register_readiness_check("jobs", jobs_health_check(map {
    "timeout_ms": 500
}))
register_readiness_state("scheduler", map { "max_age_ms": 30000 })
set_readiness_state("scheduler", true, map { "summary": "fresh" })

enable_health_routes(map {
    "health": "/health",
    "live": "/livez",
    "ready": "/readyz"
})
```

V1 does not run arbitrary ntnt closures on the health request path. Native check constructors return opaque, deadline-aware `HealthCheck` values. Application-specific work publishes bounded cached readiness state with an expiry; the request path reads that state rather than invoking application code.

## Context contract

- Context is lexically/scopally bound to a callback.
- Previous context is restored on every exit path.
- Request and job workers begin with a fresh root context.
- Child tasks inherit a serialized allowlist of safe scalar fields, not arbitrary values.
- Jobs capture only keys explicitly configured for propagation.
- `Secret`, connection, capability, task, channel, and closure values are rejected.
- Context keys and serialized size are bounded.
- Logging and tracing may include context automatically; business payloads do not.
- Tenant context remains observability/correlation data, not authorization. Database RLS still uses transaction-local canonical tenant state.

## Metrics contract

- Counter, gauge, and histogram primitives.
- OpenMetrics/Prometheus exposition first.
- Metric and label names validated.
- Label count and value length bounded.
- Runtime cardinality guard with operator-visible rejection metrics.
- No raw target, URL, user, tenant, incident, or request IDs in default label examples.
- Worker/job/runtime metrics exported under stable names.
- Application metrics cannot overwrite runtime metric definitions with incompatible types.

## Tracing contract

- Scoped spans with automatic close on all exits.
- W3C trace-context parse/inject helpers.
- Safe propagation through jobs and outbox records.
- OTLP exporter is opt-in and may be a later PR after the in-process span model is stable.
- Trace export failure cannot fail business operations by default.
- No secret or full-body capture.

## Health contract

- `/livez` is a runtime-native fast path outside the interpreter worker queue. It answers whether the process/event loop can serve and is not gated by application callbacks or dependency checks.
- `/readyz` answers whether this instance should receive traffic using cached state from bounded native checks and expiring application-published readiness values.
- `/health` provides bounded internal dependency summary suitable for authenticated/operator use; public detail is configurable and redacted.
- Runtime-native checks use deadline-aware I/O. The request path never waits on arbitrary ntnt code.
- Application-specific readiness is cooperative: application code publishes state with a maximum age. Stale state fails readiness; it does not run or preempt a callback during the probe.
- Health refresh has bounded concurrency. A stuck native refresh is timed out/quarantined without consuming the `/livez` path or spawning unbounded replacements.
- Health endpoints do not expose secret names, values, connection strings, tenant data, or raw exception chains.

## Implementation starting points

- Create `src/stdlib/context.rs`, `metrics.rs`, `trace.rs`, and `health.rs` as separate focused modules.
- Add scoped context storage to interpreter/request/job execution state rather than a mutable process-global map.
- Modify `src/stdlib/log.rs` to merge safe context fields.
- Modify `src/stdlib/jobs.rs` and outbox implementation for explicit propagation.
- Modify `src/stdlib/http_server.rs`, `http_server_async.rs`, and `http_bridge.rs` for root request context, runtime-native `/livez`, and cached readiness/health routes.
- Add native deadline-aware health-check constructors plus expiring application-published readiness state; do not execute arbitrary ntnt closures on probe requests.
- Add a small metrics registry before selecting an exporter dependency.
- Add integration tests proving worker/context cleanup and bounded health behavior.

## Acceptance criteria

- [ ] Sequential requests/jobs cannot observe each other's context.
- [ ] Nested context restores outer values correctly.
- [ ] Error and cancellation paths clear context.
- [ ] Secret/capability/handle values cannot enter context propagation.
- [ ] Runtime and application metrics expose valid OpenMetrics text.
- [ ] Cardinality limits reject abuse without crashing the process.
- [ ] Trace IDs propagate across an HTTP→outbox→job test.
- [ ] Liveness remains available during dependency failure, interpreter saturation, and a stuck readiness refresh while readiness fails.
- [ ] Readiness uses native checks or expiring published state; arbitrary closures cannot block the probe path.
- [ ] Health output is bounded and redacted.

---

# 7. Raw-SQL Database Migration Runner

## Problem

Production ntnt applications need a reliable schema lifecycle. The existing `ntnt migrate` command rewrites legacy string interpolation; it is not a database migration tool.

Larrimon-style schemas require extensions, composite foreign keys, RLS, triggers, partitioned tables, concurrent indexes, and expand–migrate–contract rollouts. An ORM would hide the important details. Ntnt should preserve raw SQL and own the lifecycle around it.

## Proposed layout

```text
migrations/
├── 202607240001_create_core.sql
├── 202607240002_enable_rls.sql
├── 202607240003_create_observations.sql
└── 202607240004_observation_indexes.sql
```

Connection and directory selection should come from committed non-secret manifest metadata plus an environment-variable reference, not a raw connection string in shell history:

```toml
[database]
url_env = "DATABASE_URL"
migrations_dir = "migrations"
```

Optional metadata header:

```sql
-- ntnt:migration
-- transaction: false
-- phase: expand
-- min_app_version: 0.5.1
-- timeout_ms: 600000
```

CLI:

```bash
ntnt db status
ntnt db plan
ntnt db migrate
ntnt db verify
ntnt db migrate --to 202607240003
ntnt db repair 202607240004 --mark-applied --checksum <sha256>
```

Use `ntnt db ...` to avoid colliding with source-code `ntnt migrate`.

## Required guarantees

- Ordered immutable migration IDs.
- SHA-256 checksum recorded after successful application.
- Modified applied migrations fail verification.
- PostgreSQL advisory lock serializes migrators.
- Transactional by default.
- Explicit `transaction: false` for operations such as `CREATE INDEX CONCURRENTLY`.
- Dirty/incomplete non-transactional migrations block progress until resolved explicitly.
- Before non-transactional SQL runs, the runner commits a `running` ledger row. Success changes it to `applied`; failure changes it to `dirty` with a redacted error summary.
- A stale `running` row is treated as dirty after lock acquisition; elapsed wall time alone never marks it applied.
- `ntnt db repair` is an explicit operator acknowledgement after manual inspection/repair. Marking applied requires the expected file checksum and is audit-visible; the runner never guesses that partial SQL is safe.
- Statement and lock timeouts are configurable and bounded.
- Migration table lives in a versioned `_ntnt` schema.
- `plan` is read-only and shows pending/blocked state.
- `verify` checks file/database agreement without applying changes.
- Rollback is not automatically inferred.
- Down migrations are omitted from v1; forward repair is the production-safe default.
- Expand/migrate/contract phase is metadata and validation guidance, not automatic destructive rollout.
- No migration filename or SQL body is built from runtime user input.
- Connection strings and SQL parameter values never appear in routine logs.
- V1 reads the URL from the manifest-declared environment-variable name (or a documented default such as `DATABASE_URL`); it should not encourage a plaintext `--database-url` argument visible in process listings and shell history.

## Proposed internal table

```sql
CREATE TABLE _ntnt.schema_migrations_v1 (
    migration_id TEXT PRIMARY KEY,
    checksum_sha256 TEXT NOT NULL,
    phase TEXT NOT NULL,
    transactional BOOLEAN NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('running', 'applied', 'dirty')),
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    completed_at TIMESTAMPTZ,
    app_version TEXT NOT NULL,
    execution_ms BIGINT,
    error_summary TEXT,
    repaired_at TIMESTAMPTZ,
    repaired_by TEXT
);
```

Final SQL remains subject to implementation review.

## Implementation starting points

- Add `db` command group in `src/main.rs`.
- Create a focused migration module, likely `src/migrations.rs`, rather than placing CLI lifecycle code in `src/stdlib/postgres.rs`.
- Reuse PostgreSQL connection parsing/pooling carefully; migrations may require a dedicated connection and must not share an application transaction handle.
- Use `sha2`, already present, for checksums.
- Add fixture migrations and CLI integration tests to `tests/cli_tests.rs` plus disposable PostgreSQL tests.
- Add `docs/DATABASE_MIGRATIONS.md` and link it from `docs/AI_AGENT_GUIDE.md`.

## Acceptance criteria

- [ ] Two concurrent migrators result in one ordered application.
- [ ] Transactional failure leaves no applied record or partial schema.
- [ ] Non-transactional failure marks migration dirty and blocks later work.
- [ ] Applied-file mutation is detected by `status` and `verify`.
- [ ] `plan` performs no writes.
- [ ] Timeouts and advisory-lock waits produce actionable diagnostics.
- [ ] Connection credentials are redacted.
- [ ] Existing source `ntnt migrate` behavior remains unchanged.

---

# 8. `pure fn`, Effect Checking, Static Exhaustiveness, and Reducer Helpers

## Problem

Replayable derived state depends on deterministic reducers. Today a reducer can accidentally call `now()`, generate a UUID, read the environment, fetch a URL, enqueue work, log, or touch the database. Reviews and conventions are weaker than a language-enforced boundary.

Ntnt already tracks execution-mode capabilities for native functions, but capability availability and semantic effects are different concepts. A function may be available in a mode and still be impure.

## Proposed language surface

```ntnt
pure fn reduce_check(
    state: CheckState,
    event: Observation
) -> CheckState {
    match event.kind {
        ObservationKind::Success(data) => reduce_success(state, data)
        ObservationKind::Failure(data) => reduce_failure(state, data)
        ObservationKind::Unknown(data) => reduce_unknown(state, data)
    }
}
```

`pure fn` is the only new syntax proposed in this DD. It should be withheld until a focused parser/typechecker design review confirms the transitive call rules.

## Effect model

Introduce effect metadata separate from `RuntimeCapability`:

```rust
enum EffectKind {
    Database,
    Network,
    FileSystem,
    Clock,
    Random,
    Environment,
    Process,
    Job,
    Log,
    MutableGlobal,
}
```

A native function declares its effects. User functions infer the transitive union of effects from their call graph.

A `pure fn` must have an empty effect set.

### Unknown and dynamic calls

Recommended rule:

- A statically resolved pure user/native call is allowed.
- A call with unknown/dynamic target is rejected inside `pure fn` in strict lint/typecheck mode.
- Default lint mode warns while the feature is experimental; production verification treats the warning as an error.
- No runtime fallback pretends dynamic code is pure.

### Values and mutation

Purity means no externally observable effect and deterministic output for the same inputs. V1 conservatively rejects assignment, index mutation, mutable captures, and module-state mutation inside `pure fn`; current typechecker scopes do not track enough binding provenance or alias escape to prove a mutation is confined to a fresh local value. Reading immutable constants is allowed. Reading process configuration, current time, randomness, or mutable module state is not. A later DD may permit proven-local mutation after dedicated binding-provenance/capture analysis exists.

## Static match exhaustiveness

The typechecker already checks known enum, `Option`, and `Result` matches, and the runtime retains a defense-in-depth enum check. This roadmap hardens the existing checker rather than introducing a new subsystem:

- missing variants keep producing source-located diagnostics;
- guarded variants do not count as total coverage unless an unguarded arm covers the variant;
- wildcard behavior is explicit and tested;
- duplicate and unreachable arms produce stable diagnostics;
- strict/default severity and source ranges remain compatibility-reviewed.

This hardening is valuable independently of reducers and should ship before or with `pure fn`.

## Reducer helpers

Keep persistence application-owned. `std/reducer` should provide small pure helpers rather than an event-sourcing framework:

```ntnt
import { replay, replay_batches } from "std/reducer"

let state = replay(initial_state, events, reduce_check)?
```

V1 helpers are intentionally limited to:

- `replay(initial, events, reducer)`;
- `replay_batches(initial, batches, reducer)`.

Canonical state hashing, persisted checkpoint formats, and idempotence test helpers each require independent semantic contracts. They remain focused follow-ups rather than being smuggled into the purity PR. Generic reducers are not necessarily idempotent when the same event is applied twice, and a sample-based helper must not claim to prove otherwise.

The reducer module must not own application tables, event schemas, topology, suppression, or alert behavior.

## Required guarantees

- Effect metadata covers every builtin and stdlib native function; generated docs/build checks prevent undocumented additions.
- User-function effects are transitive across imports and ordinary calls.
- Recursive functions converge under call-graph analysis.
- Pure functions cannot call unknown dynamic functions in strict verification.
- Diagnostics show the shortest effect path: `reduce_check → classify → now`.
- Pure functions cannot access `Secret`, connections, network capabilities, task handles, channels, or mutable runtime state; recommended v1 rejects these parameter/result types entirely.
- V1 rejects assignment, index mutation, and mutable captures inside `pure fn` until binding-provenance/alias analysis can prove local mutation safe.
- Reducer helpers do not hide ordering; callers supply ordered events.

## Implementation starting points

- Modify `src/lexer.rs`, `src/parser.rs`, and `src/ast.rs` for the `pure` modifier only after the design spike.
- Add effect metadata to native-function definitions in `src/interpreter.rs`; avoid overloading `RuntimeCapability`.
- Extend module/type information in `src/typechecker.rs` to compute user-function effect sets and static match exhaustiveness.
- Add dedicated diagnostics in `src/error.rs` and machine-readable lint rule IDs.
- Create `src/stdlib/reducer.rs` for pure `replay`/`replay_batches` helpers only.
- Add language tests to `tests/language_features_tests.rs`, type/effect tests to `tests/type_checker_tests.rs`, and diagnostic rendering cases to `tests/diagnostics_tests.rs`.
- Update syntax, stdlib, and agent guides with generated-doc drift checks.

## Acceptance criteria

- [ ] Pure arithmetic/data functions pass.
- [ ] Every direct effect category is rejected from `pure fn`.
- [ ] Transitive and imported effect calls are rejected with a useful call path.
- [ ] Recursive pure functions are accepted when their reachable graph is pure.
- [ ] Unknown dynamic calls follow the documented strict/default behavior.
- [ ] Static match diagnostics cover enums, `Option`, and `Result`.
- [ ] Runtime exhaustiveness remains as defense in depth.
- [ ] Assignment, index mutation, and mutable captures are rejected in v1 pure functions.
- [ ] Reducer replay processes bounded batches without owning persistence.

---

# Architecture Decisions

## AD-1: Prefer closure-scoped APIs before new resource syntax

`with_transaction`, `with_lease`, `with_context`, and `with_span` should prove one reusable native-callback bridge. Do not add `using`, `defer`, or context-manager syntax in this roadmap. Reconsider syntax only after several scoped APIs show repeated friction that functions cannot solve cleanly.

## AD-2: Keep capability availability separate from effect classification

`RuntimeCapability` answers whether an operation may execute in a runtime mode. `EffectKind` answers whether a function is deterministic/pure. Combining them would make both concepts harder to reason about.

## AD-3: At-least-once plus idempotency, never exactly-once theater

Outbox dispatch, jobs, lease recovery, and callbacks are at-least-once. Stable IDs, uniqueness constraints, fencing tokens, and idempotent consumers provide correctness.

## AD-4: Preserve raw SQL

The migration, outbox, streaming, and transaction tracks should not introduce an ORM. SQL remains explicit for advanced PostgreSQL features.

## AD-5: Policies are opaque authority

A network-policy map is configuration, not authority. Runtime-created `NetCapability` values are unforgeable and non-serializable.

## AD-6: Schema functions before schema syntax

Expand `std/validate` first. Add language grammar only after the runtime schema contract is proven; typed decode requires its own compiler/type-level decision rather than being implied by stdlib syntax.

## AD-7: Observability context is not authorization

Context propagation improves correlation. Canonical authorization, tenant ownership, and RLS context still come from authenticated state and transaction-local database settings.

## AD-8: Reducers remain application-defined

Ntnt enforces purity and supplies replay helpers. Applications own event schemas, ordering rules, persistence, checkpoints, and domain transitions.

---

# Focused Implementation Roadmap

The eight items are grouped into four release waves. Each PR must be independently useful and may ship without committing to later waves.

## Wave 0: Design spikes and cross-cutting foundations

### PR 0A — Reusable native callback bridge

**Goal:** Give stdlib functions a single safe way to invoke ntnt callbacks with interpreter context and guaranteed cleanup.

**Files:**

- `src/interpreter.rs`
- focused internal helper module if extraction improves ownership
- callback tests in existing language/integration suites

**Scope:**

- [ ] Inventory existing one-off callback invocation paths (`validate`, auth flows, concurrent helpers).
- [ ] Define callback success/error/non-local-return behavior.
- [ ] Prevent callback references from escaping unsupported thread/process boundaries.
- [ ] Add cleanup-hook support used by scoped primitives.
- [ ] Preserve existing callback APIs.

**Gate:** No public API change; tests prove cleanup after every exit shape.

### Design spike 0B — Outbound transport binding

**Goal:** Prove `reqwest`/resolver mechanics can bind a request to policy-approved addresses and revalidate redirects without weakening TLS hostname verification or proxy behavior.

**Artifacts:**

- short spike note under `plans/`;
- loopback redirect/rebinding fixture;
- dependency/API decision;
- no production API.

**Gate:** Do not begin public `NetCapability` work until the transport boundary is demonstrated.

### Design spike 0C — Effect metadata coverage

**Goal:** Classify current builtins/stdlib functions and identify unresolved dynamic-call/import cases before adding `pure` syntax.

**Gate:** A coverage report shows every native function can receive an effect set without conflating execution capabilities.

---

## Wave 1: Atomic data and boundary contracts

### PR 1A — Scoped PostgreSQL transactions

- [ ] `with_transaction()` callback API with opaque generation-bound `Tx`.
- [ ] Base-handle exclusion and scoped manual commit/rollback rejection.
- [ ] Isolation/timeout/`set_local` validation.
- [ ] rollback/commit ambiguity diagnostics.
- [ ] savepoints only through active `Tx`.
- [ ] Alias, concurrent-use, escaped-handle, and PostgreSQL failure-matrix tests.

### PR 1B — Database migration runner foundation

- [ ] `ntnt db status|plan|migrate|verify`.
- [ ] checksums and advisory lock.
- [ ] transactional migrations.
- [ ] internal `_ntnt` schema bootstrap with `running | applied | dirty` lifecycle fields from the start.
- [ ] CLI and concurrent-migrator tests.

### PR 1C — Non-transactional migration hardening

- [ ] metadata headers.
- [ ] committed `running` row before non-transactional execution.
- [ ] dirty/stale-running detection and progress blocking.
- [ ] explicit checksum-bound `ntnt db repair` acknowledgement.
- [ ] timeout controls and expand/migrate/contract phase metadata.
- [ ] operator recovery documentation.

### PR 1D — Strict nested schemas

- [ ] schema-node representation and explicit primitive constructors.
- [ ] nested objects/arrays/enums/literals.
- [ ] strict unknown-field policy.
- [ ] structured error paths.
- [ ] depth/width/length bounds.
- [ ] backward compatibility for current `validate()`.

### PR 1E — Versioned data contracts

- [ ] version dispatch and adjacent ordinary upcaster callbacks.
- [ ] JSON Schema export for the supported subset with explicit non-exportable-schema errors.
- [ ] schema fingerprints.
- [ ] HTTP/job reuse examples.
- [ ] defer purity enforcement to PR 4C and typed decode to a focused compiler/type-level decision.

### PR 1F — Atomic/reconcilable job acceptance

- [ ] Explicit durable idempotency key API.
- [ ] One SQLite transaction or Redis/Valkey Lua operation atomically commits idempotency key, job record, and pending index.
- [ ] Detect and repair legacy/orphan dedup records instead of reporting nonexistent jobs as accepted.
- [ ] Crash-injection and backend-parity tests.

### PR 1G — Transactional outbox

**Depends on:** PR 1A, migration foundation, and PR 1F atomic job acceptance.

- [ ] `_ntnt.outbox_v1` migration.
- [ ] `outbox_emit()`.
- [ ] claim/lease/retry/dead lifecycle.
- [ ] jobs dispatcher with deterministic key and confirmed atomic acceptance.
- [ ] relay crash matrix and secret rejection.

**Wave 1 exit criteria:** Applications can validate a versioned payload, execute a scoped tenant/RLS transaction, commit canonical state plus an outbox event, and migrate the supporting schema safely.

---

## Wave 2: Scale, network authority, and distributed work

### PR 2A — PostgreSQL cursor batches

**Depends on:** PR 0A callback bridge and PR 1A scoped transaction ownership.

- [ ] `each_query_batch()`.
- [ ] bounded fetch count plus per-row and per-batch decoded-byte limits.
- [ ] release the PostgreSQL client guard before callback invocation; same-`Tx` sequential query regression.
- [ ] callback early-exit cleanup.
- [ ] cancellation and transaction interaction.
- [ ] replay/export examples.

### PR 2B — PostgreSQL COPY and bounded statement batches

- [ ] `copy_rows()`.
- [ ] strict identifier validation.
- [ ] atomic error semantics.
- [ ] `execute_batch()` if the implementation remains materially smaller than repeated app calls.
- [ ] ingestion benchmarks and diagnostics.

### PR 2C — Trusted network configuration and internal capability core

**Depends on:** transport spike 0B.

- [ ] application request versus deployment grant model.
- [ ] explicit policy path, canonicalization, owner/permission checks, precedence, load-once/fail-closed behavior.
- [ ] opaque internal `NetCapability` value/type.
- [ ] shared target classification and reason codes.
- [ ] serialization/log/template/cache/DB rejection.
- [ ] default-policy compatibility tests.
- [ ] keep `require_net_capability()` unexported until PR 2D provides an enforcing transport.

### PR 2D — Policy-bound HTTP transport and public capability API

**Depends on:** PR 2C.

- [ ] approved-address binding.
- [ ] redirect revalidation.
- [ ] proxy behavior contract.
- [ ] response-size and redirect clamps.
- [ ] export `require_net_capability()` only with an end-to-end enforcing sink.
- [ ] secret-bearing request compatibility matrix.

### PR 2E — `std/net` capability integration

- [ ] use shared policy engine across probes.
- [ ] named private-scope capabilities.
- [ ] port/CIDR enforcement.
- [ ] mixed-address and mapped-address tests.

### PR 2F — Job lease heartbeat and fencing

- [ ] renewable active-job leases.
- [ ] cooperative ownership-loss state at documented yield points.
- [ ] fencing-token API plus atomic guarded-sink examples/tests.
- [ ] Redis cross-host operations and SQLite shared-file/process-host operations with explicit backend capability reporting.
- [ ] stale-worker race tests.

### PR 2G — Keyed limits and queue admission

- [ ] `rate_by` and `concurrency_by` declared fields.
- [ ] queue `max_pending` and explicit admission behavior.
- [ ] bounded dead/completed retention.
- [ ] per-key fairness and backend parity tests.

### PR 2H — Leadership and rolling drain

- [ ] leader lease for periodic emission.
- [ ] schedule occurrence idempotency.
- [ ] worker drain control socket/CLI.
- [ ] drain timeout and active-job handoff tests.

**Wave 2 exit criteria:** Applications can ingest and replay at scale, issue outbound requests only through explicit runtime authority, and operate jobs safely across multiple workers and replicas.

---

## Wave 3: Operational context and observability

### PR 3A — Scoped context

- [ ] `with_context()` and read helpers.
- [ ] request/job root cleanup.
- [ ] explicit safe propagation allowlist.
- [ ] log integration.
- [ ] secret/capability/handle rejection.

### PR 3B — Metrics and health

- [ ] bounded in-process metric registry.
- [ ] counter/gauge/histogram APIs.
- [ ] OpenMetrics exposition.
- [ ] cardinality guard.
- [ ] runtime-native `/livez` outside the interpreter queue.
- [ ] native deadline-aware readiness checks plus expiring application-published state; no arbitrary closure execution on probe requests.

### PR 3C — Tracing

- [ ] scoped spans.
- [ ] W3C trace-context parse/inject.
- [ ] HTTP→outbox→job propagation.
- [ ] in-process/log exporter.
- [ ] OTLP decision deferred until the span model is stable.

**Wave 3 exit criteria:** Operators can correlate work across system boundaries, scrape bounded metrics, trace asynchronous paths, and distinguish process liveness from traffic readiness.

---

## Wave 4: Deterministic logic

### PR 4A — Harden existing static match exhaustiveness

- [ ] preserve existing enum, `Option`, and `Result` checks.
- [ ] guard-aware coverage and explicit wildcard semantics.
- [ ] unreachable/duplicate arm diagnostics with stable source locations.
- [ ] strict/default severity compatibility tests.
- [ ] runtime check retained.

### PR 4B — Effect metadata and transitive analysis

**Depends on:** spike 0C.

- [ ] effect set on every native function.
- [ ] generated/build-enforced coverage.
- [ ] user-function call-graph inference.
- [ ] import and recursion handling.
- [ ] machine-readable diagnostics with effect paths.
- [ ] document that binding-provenance/alias analysis is not present in v1; do not claim proven-local mutation.

### PR 4C — `pure fn`

- [ ] lexer/parser/AST modifier.
- [ ] strict/default behavior for unknown calls.
- [ ] opaque value restrictions.
- [ ] conservatively reject assignment, index mutation, and mutable captures.
- [ ] syntax/type/docs tests.
- [ ] no runtime claim stronger than static analysis supports.

### PR 4D — Minimal reducer helpers

- [ ] pure `replay` and bounded `replay_batches` only.
- [ ] deterministic examples without application persistence.
- [ ] defer canonical hashing, checkpoint formats, and idempotence helpers to focused follow-up designs.

**Wave 4 exit criteria:** Ntnt can reject impure reducers before execution, diagnose guarded/missing event variants statically, and provide small reusable replay helpers.

---

# Verification Strategy

Every implementation PR follows the ntnt pre-push gates plus feature-specific tests.

## Standard gates

```bash
cargo fmt
cargo build --profile dev-release
cargo test
./target/dev-release/ntnt docs --generate
git diff --exit-code docs/
./target/dev-release/ntnt validate examples/
./target/dev-release/ntnt lint examples/
```

## Additional infrastructure gates

- PostgreSQL tests run against a supported disposable PostgreSQL container and include concurrency/failure cases.
- Redis-specific lease/limit tests run under the existing opt-in Redis test feature; SQLite remains the default deterministic backend test.
- Network-policy tests use loopback fixtures and a controllable resolver; they do not depend on public Internet or cloud metadata endpoints.
- Metrics/tracing tests use in-memory exporters.
- Effect coverage is build-enforced so a new native function cannot omit classification silently.
- Compatibility tests compare behavior against the previous release for every modified existing API.

## Failure matrices required before release

### Transactions/outbox

- callback success;
- callback `Err`;
- `?` propagation;
- contract/runtime error;
- rollback failure;
- commit disconnect/ambiguity;
- relay crash before claim, after claim, before downstream acceptance, after acceptance, before acknowledgement.

### Leases/jobs

- healthy renewals;
- delayed heartbeat;
- store outage;
- ownership transfer;
- stale owner resumes;
- graceful drain;
- forced drain timeout;
- queue full under concurrent enqueues.

### Network

- direct public/private/loopback/link-local/metadata address;
- hostname with all-public, mixed, and all-denied answers;
- IPv4-mapped IPv6;
- redirect public→public and public→denied;
- DNS answer change between validation and connect;
- HTTP proxy/no proxy;
- secret-bearing HTTPS and direct-loopback development exception.

### Context/observability

- nested context;
- request/job reuse;
- spawn/job propagation;
- cancellation/error cleanup;
- cardinality exhaustion;
- exporter outage;
- dependency-down readiness.

### Purity

- direct native effect;
- transitive user-function effect;
- import effect;
- recursion;
- unknown dynamic call;
- assignment/index/capture rejection;
- opaque value use;
- missing enum variant and guarded arms.

---

# Rollout and Compatibility

- Every new public API begins additive and opt-in.
- Existing `begin`/`commit`/`rollback`, `fetch`, `std/net`, `std/jobs`, and `std/validate` APIs remain available while new paths prove themselves.
- Secure defaults may be tightened only with explicit compatibility tests and release notes.
- Process-global network environment flags remain compatibility inputs until named capabilities are stable.
- Current validation error shapes remain for current `validate()`; new structured errors belong to new decode APIs unless a major-version migration is approved.
- `pure fn` starts behind strict lint/verification behavior if effect coverage is not yet complete; it must not silently accept unknown calls.
- Generated stdlib docs, typechecker signatures, runtime registration, examples, and agent guides land in the same PR as each public API.
- No PR should claim enterprise/carrier readiness based solely on API presence. Race, crash, restart, bounded-resource, and recovery tests are release gates.

---

# Risks

| Risk | Impact | Mitigation |
|---|---|---|
| Roadmap becomes one giant release | High | Four waves, small standalone PRs, no all-or-nothing branch |
| Scoped callback APIs multiply interpreter special cases | High | Land one reusable callback bridge first |
| Outbox claims exactly-once semantics | High | Explicit at-least-once contract and crash matrix |
| Network capabilities become maps in disguise or app manifests self-grant authority | High | Opaque non-serializable runtime type plus explicit trusted deployment grants intersected with app requests |
| Redirect/rebinding fix breaks TLS or proxies | High | Transport spike and inverse compatibility matrix before public API |
| Schema work turns into a second type system | High | Stdlib schema nodes first; typed decode is a later narrow layer |
| Migration runner becomes an ORM | Medium | Raw SQL only; lifecycle metadata and verification are the product |
| Job fencing is treated as universal side-effect protection | High | Require applications to include tokens in guarded state writes |
| Metrics labels cause unbounded cardinality | High | Hard label/cardinality budgets and operator-visible rejection |
| Context becomes authorization | High | Explicitly non-authoritative; canonical auth/RLS remain separate |
| Effect system overreaches | High | Small effect categories, coverage spike, `pure fn` only after transitive analysis |
| Runtime and stdlib docs drift from shipped behavior | Medium | Build-enforced registration/docs/effect coverage and compatibility tests |

---

# Deferred Follow-up Ideas

These remain good ideas but are outside this DD's implementation roadmap:

- distributed HTTP abuse-rate-limiting middleware;
- provider-neutral structured inference;
- secret-aware protocol sinks beyond current HTTP support;
- job/reducer-specific IAL scenarios and virtual time;
- WebSocket/streaming expansion;
- PostgreSQL job backend;
- optional `std/netmon` promotion;
- typed schema decode after a focused compiler/type-level API decision;
- binding-provenance/alias analysis to permit proven-local mutation in `pure fn`;
- canonical reducer state hashing, persisted checkpoint formats, and narrowly named sample-based idempotence test assistance;
- workflow/state-machine DSLs.

They should not be smuggled into the eight selected tracks during implementation review. Scope creep is still scope creep when wearing a correctness badge.

---

# Definition of Done

The DD track is complete when:

- [ ] Wave 0 spikes settle the callback, network transport, and effect-coverage foundations.
- [ ] Scoped transactions, migrations, strict contracts, and transactional outbox are shipped and documented.
- [ ] PostgreSQL streaming/bulk APIs and opaque network capabilities are shipped and validated.
- [ ] Job leases, fencing, keyed limits, backpressure, leadership, and drain behavior are shipped for supported backends.
- [ ] Scoped context, bounded metrics, tracing, and health registration are shipped.
- [ ] Static exhaustiveness, effect metadata, `pure fn`, and reducer helpers are shipped.
- [ ] Every public function has runtime registration, typechecker signatures, generated docs, examples, and compatibility tests.
- [ ] Crash/race/resource-bound matrices pass in CI or documented opt-in integration jobs.
- [ ] Design-document status and roadmap checkboxes are truth-synced to merged PRs.

Until then, this document remains a roadmap. Individual waves may be independently complete without implying that later language work has shipped.
