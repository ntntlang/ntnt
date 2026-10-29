# PostgreSQL migrations, contracts and catalog evidence

`ntnt db` implements the PostgreSQL transactional subset of issue #245: read-only inspection/planning and separately authorized exact-plan execution. It keeps native PostgreSQL SQL and one immutable migration history. It does not implement an ORM or an authored desired-schema DSL.

```sh
ntnt db status --target development --json
ntnt db plan --target development --json --out review.json
ntnt db verify --target development --json
ntnt db verify --target development --against review.json --json
ntnt db migrate --database app --target development --plan review.json --json
ntnt db status --target development --object accounts
```

`status` and `verify` inspect history and compare the observed catalog with replay at the **applied prefix**. `plan` additionally replays pending SQL and captures a non-authorizing review artifact. Pending migrations are not drift. `--object` is a case-sensitive substring filter on displayed catalog keys; it never narrows verification, hides incomplete coverage, or changes the plan binding. Without `--json`, the same result is rendered for humans.

`migrate` requires a reviewed execution-capable plan and independent provider-backed approval. It never plans implicitly, auto-baselines, resets, repairs or retries application SQL after an uncertain commit. The existing `ntnt migrate` source-interpolation conversion command is unchanged. SQLite, baseline adoption, concurrent indexes/non-transactional operations, DML/backfill orchestration and query lint are not implemented. Existing application-owned migration code is not invoked.

## Explicit configuration and secrets

`--manifest` defaults to `ntnt.toml` in the current directory; there is no implicit ancestor/target selection. The migration directory is relative to that manifest, not the caller's working directory.

```toml
[databases.app]
engine = "postgresql"
schema = "app"
migrations_dir = "migrations/app"
approval_secret = "APP_MIGRATION_APPROVAL"

[databases.app.targets.development]
connection_secret = "APP_INSPECTION_DATABASE"
database = "app_development"
role = "app_owner"

[databases.app.scratch]
connection_secret = "APP_REPLAY_DATABASE"
database = "app_replay"
role = "app_replay_owner"
disposable = true

[secrets.APP_INSPECTION_DATABASE]
required = true

[secrets.APP_REPLAY_DATABASE]
required = true

[secrets.APP_MIGRATION_APPROVAL]
required = true
```

- Declare logical stores as `[databases.<name>]`. `--database <name>` selects one; omit it only when exactly one database is declared. Unknown or ambiguous selection fails before connection lookup. Names start with an ASCII letter and contain at most 63 ASCII letters, digits, underscores or hyphens.
- Select a named deployment target with `--target`; it is always required, even for a single database or target. There is no fallback connection, URL argument, `url_env` alias, plaintext manifest credential, or `--force` bypass. Unknown fields in the selected database definition are errors.
- Connections reuse the existing **`std/secrets` providers and declaration validation**. Development's environment provider reads the exact logical name. Production uses the configured Unix-socket provider; the environment provider remains disabled in production. See [Secrets Agent Protocol](secrets-agent-protocol.md). Declarations are read from the captured manifest, not reopened during replay.
- A connection secret must specify exactly one host, the declared `user`/`dbname`, and **`sslmode=disable`**, with no startup `options`. This initial adapter reuses the existing NoTLS driver. Use a trusted local Unix socket or a separately secured local transport; do not send credentials over an untrusted plaintext network. TLS transport is not implemented and there is no silent TLS downgrade.
- Physical database, role and managed-schema names use ASCII letters/digits/underscores, start with a letter, and are at most 63 bytes. SQL object identifiers may be quoted. System schemas, `_ntnt`, and `public` cannot be the managed schema.
- The **target managed schema must already exist**. Read-only commands never create it or `_ntnt`. Authorized `migrate` bootstraps its scoped ledger only inside the first migration transaction, after locked validation of an empty recognized base. A nonempty unrecognized scope is never assumed to have empty history.
- The declared target role is the logical owner for expected owner/grant canonicalization. Scratch uses its separate owner role. Other owners/grantees remain distinct; ownership and grants are never stripped wholesale. Multi-owner histories or additional grants that cannot be reproduced by the supported history will not receive a clean comparison. Target authority/membership facts are also bound into plans.

## Exact-plan authority and execution

`approval_secret` is optional for inspection but **required for every execution**, including additive changes and reruns. It names a separate top-level secret, never a connection secret. Its resolved value must be exactly the reviewed `plan_digest` (no whitespace trimming). The deployment/operator publishes that value through a separately protected existing `std/secrets` provider after reviewing the plan and migration SQL. The digest binds SQL/metadata, policy, logical store/target and physical identity; **knowing the digest does not authorize execution**. Obtaining its exact value through the protected provider does. There is no `--approve`, `--force`, self-authorizing manifest Boolean, signing server or new approval service.

Use separate application and migrator credentials, and restrict schema DDL/ledger ownership to the migrator. Development fixtures may use the environment provider; protected deployments use the existing Unix-socket provider and separately controlled approval delivery. NTNT cannot protect against someone who already controls that provider or direct database-owner credentials. Never publish resolved connection strings or provider values. Backup/PITR, restore testing, old-client compatibility and maintenance authorization remain separately operated responsibilities; a committed destructive change is not made reversible by transactional rollback.

Each invocation uses a dedicated physical connection and a session advisory lock on the database-local `_ntnt` namespace (two-key lock `1853124212, 245`). The reserved namespace deliberately owns **one logical store/managed schema per physical database**; separate stores must use separate databases. This is not a multi-tenant ledger. The lock spans all per-migration commits and is bounded by the native lock timeout.

For each migration, the runner:

1. Begins a `READ COMMITTED READ WRITE` transaction and takes `ACCESS EXCLUSIVE` locks on all existing ordinary managed/ledger tables in deterministic order.
2. Takes fresh identity/history/catalog observations **after acquiring those locks** and validates the exact ordered planned prefix, captured source/policy and mutation-safe ledger.
3. Runs named `requires`, executes the captured full SQL script, then runs `ensures`.
4. Verifies the resulting catalog against that exact prefix fingerprint and inserts the success ledger entry in the same transaction.
5. Commits that migration before starting the next. Failure never rolls back an earlier committed migration.

These intentionally strong native locks protect data checks against ordinary concurrent writers. They can block readers too; this is **not online migration tooling**. Advisory locks alone only coordinate cooperating migrators. No lock protocol promises to stop administrators or actors with direct DDL privileges. Incoming foreign keys from outside the managed schema and all explicit cascading DDL are refused rather than claiming complete dependency/effect mediation.

Every source/plan is captured before I/O; no validated filename is reopened during execution. A new invocation recaptures it. Same-plan reruns after partial/all commits recognize matching immutable history and the expected catalog at each ordered prefix, without reapplying SQL or rerunning already-applied transition checks. Edited SQL/checks, missing/reordered history, catalog drift, other stores/targets, changed policy/authority or extra unsupported extensions fail closed.

`progress` distinguishes `committed`, `already_applied`, `failed`, `unknown` and `not_run`. It retains acknowledged commits even when later work fails. An I/O error while acknowledging COMMIT discards the connection and performs **fresh locked history/catalog reconciliation** on a new physical connection, without retrying SQL. If that inspection cannot establish the outcome, it remains `unknown`; restore connectivity and rerun the same reviewed plan to reconcile before further work. A killed process cannot return a report; its next invocation follows that same reconciliation path. Failed rollback also discards the connection and reports unknown cleanup. No failed/unknown command promises that the whole batch was rolled back.

## Multiple databases in one application

A **logical database** (`app`, `analytics`) owns an engine, managed schema, migration directory, named deployment targets, and optional scratch configuration. A **target** (`development`, `production`) selects an endpoint for that logical database's history. Targets are not separate migration histories.

For example, add this alongside the `app` definition above:

```toml
[databases.analytics]
engine = "postgresql"
schema = "analytics"
migrations_dir = "migrations/analytics"

[databases.analytics.targets.development]
connection_secret = "ANALYTICS_INSPECTION_DATABASE"
database = "analytics_development"
role = "analytics_owner"

[databases.analytics.scratch]
connection_secret = "ANALYTICS_REPLAY_DATABASE"
database = "analytics_replay"
role = "analytics_replay_owner"
disposable = true

[secrets.ANALYTICS_INSPECTION_DATABASE]
required = true
[secrets.ANALYTICS_REPLAY_DATABASE]
required = true
```

```sh
ntnt db plan --database app --target development --json
ntnt db plan --database analytics --target development --json
```

Only the selected store's migration directory and connection secrets are accessed. Other stores may be unprovisioned or use a future adapter; their backend-specific fields are not interpreted by this operation. Registry names/table shapes and the shared secret declaration boundary remain validated. Each invocation operates on one logical database—there is no implicit `--all`, cross-database execution, or distributed transaction.

Reports identify the logical name as `database`, the deployment label as `target.name`, and the actual connected database as `target.identity.database`. Plans bind both logical database and target labels as well as physical identity, so even aliases for the same endpoint cannot exchange plans silently. A single database's explicit and implicit selection produce the same binding. The captured manifest is still bound in full: editing any part of that manifest invalidates an earlier plan; changing an unselected migration file does not.

This named form replaces the unmerged PR prototype's singular `[database]` configuration. Move its tables under `[databases.app]`, `[databases.app.targets.<target>]`, and `[databases.app.scratch]`. Singular or mixed forms are rejected with a configuration error rather than guessed or silently combined. The top-level `[secrets.<name>]` declarations remain unchanged.

## Immutable SQL inventory

```text
migrations/app/
  0001_accounts.sql
  0002_timezone.sql
  0002_timezone.toml
```

```sql
-- 0001_accounts.sql
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE
);
```

```sql
-- 0002_timezone.sql
ALTER TABLE accounts ADD COLUMN timezone TEXT DEFAULT 'UTC';
```

Optional TOML sidecars add traceability and named transition contracts:

```toml
intent_ids = ["account-timezone"]

[requires]
no_null_emails = "SELECT NOT EXISTS (SELECT 1 FROM app.accounts WHERE email IS NULL)"
no_duplicate_emails = "SELECT NOT EXISTS (SELECT email FROM app.accounts GROUP BY email HAVING count(*) > 1)"

[ensures]
filled_timezone = "SELECT NOT EXISTS (SELECT 1 FROM app.accounts WHERE timezone IS NULL)"
```

Checks run in name order within each phase. Each must return **exactly one row, one Boolean column, and true**. False, NULL, no rows, multiple rows/columns, wrong type, SQL error and timeout fail the transaction. Reports include check name, phase, source and outcome, never returned values. Only actually attempted target/rehearsal checks count as executed assertions; zero checks remains `not_run`. A check pass before later rollback is not a committed schema guarantee.

The closed PostgreSQL AST subset supports Boolean expressions, identifiers/literals, comparisons and arithmetic, `IS NULL`/`IS NOT NULL`, `EXISTS`/`NOT EXISTS`, scalar subqueries, one managed heap table per SELECT, ordinary column GROUP BY/HAVING and builtin `count(*)`. Each query must have only these recognized clauses. Tables may be unqualified or explicitly managed-schema qualified; checks use a controlled `pg_catalog`-first search path. The adapter verifies live catalog coverage and builtin base types/indexes, and conservatively rejects database-wide custom implicit/assignment casts before DDL or checks. Builtin operators/functions resolve through the trusted PostgreSQL installation, not a `provolatile` assertion on a user routine.

Not supported: arbitrary functions/routines, casts, aliases/joins, SELECT INTO, CTEs, locking clauses, table functions/sampling, windows, wildcard projections, custom types/operators/access methods, session/transaction changes or arbitrary SQL. A SELECT keyword or READ ONLY transaction is **not a purity proof**. Unsupported syntax is rejected before target I/O, never approved by regex. There are at most 64 named checks per migration, each at most 64 KiB; names use simple ASCII identifiers.

`ensures` are transition checks, not permanent invariants: `verify` does not re-execute historical postconditions. Use database constraints for lasting concurrency/future-write enforcement and ordinary native/Intent tests for business behavior. Intent IDs remain traceability, not proof. Execution/transaction modes and other metadata remain rejected.

Files use `<positive-decimal-u64>_<ASCII-name>.sql`; names contain letters, digits, `_` or `-`. Ordering is numeric; `01` and `0001` are the same ID and cannot coexist. Gaps are allowed. Sidecars must have the exact SQL stem. The directory contains only SQL and matching TOML files. Orphan sidecars, duplicate IDs, invalid UTF-8, unsafe paths, symlinks, hard links, directories in place of files and nonregular inputs are rejected.

Checksum v1 is lowercase SHA-256 over length-framed byte parts. Each part is prefixed by its unsigned 64-bit big-endian byte length. The parts are:

1. `ntnt-migration-v1`
2. exact SQL filename bytes
3. exact SQL file bytes
4. `sidecar` or `absent`
5. exact metadata bytes, or empty bytes when absent

Whitespace, comments, filenames, metadata comments, and even absent versus empty metadata matter. No newline or SQL normalization occurs. Applied ledger entries must be the exact ordered inventory prefix: missing/edited files, inserted historical migrations, reordered IDs and dirty attempts fail before any replay can be mistaken for valid history.

Capture is bounded to 1 MiB per input (including a saved review artifact), 16 MiB of migration files, and 1,024 migrations. Parsing is limited to 4,096 tokens per script (including whitespace tokens) and recursion depth 64. Oversized or unsupported SQL is unknown, not automatically safe. Source files and manifest policy are captured once, then replay consumes those captured bytes even if their paths are subsequently replaced. A new command recaptures current inputs and invalidates an old binding if they changed.

## Disposable replay and its boundary

Provision scratch yourself using existing native test/container facilities. NTNT does not create a database/container, copy target rows, reuse target credentials, or reset an existing database.

Scratch must:

- have `disposable = true` and a separate secret and role;
- resolve to a different **cluster-system-identifier/database-OID pair**, not merely a different URL spelling;
- exactly match the target's server version number, encoding, locale/provider/version, timezone, DateStyle, IntervalStyle and client/string settings;
- be owned by a nonsuperuser role with no memberships, `CREATEDB`, `CREATEROLE`, replication, or RLS-bypass authority;
- contain only the empty stock public namespace and system objects, with no user routines/types/relations, event triggers, extra extensions or default grants.

Identity discovery uses `pg_control_system()` and catalog metadata. Missing permission to establish identity fails closed. Server administrators and the PostgreSQL installation are trusted; this is not protection against a hostile superuser modifying PostgreSQL's own catalogs or concurrent administrators changing scratch after validation. Use an exclusively disposable fixture, preferably network-isolated. Restored/cloned clusters sharing identity can be conservatively rejected.

Replay owns a dedicated physical connection and a **rollback-only outer transaction**, not a pool handle. It creates the managed scratch schema only after isolation/emptiness checks, replays captured scripts through the native execution primitive, captures the applied-prefix/candidate catalogs, and explicitly rolls back. Success and failure tests verify no scratch schema survives. The authorized per-migration executor reuses that exact captured-SQL primitive. Catalog derivation deliberately skips transition contracts: an empty replay cannot establish data-dependent production assertions. Fresh/populated native rehearsal tests invoke the same `migrate` executor on disposable fixtures with representative rows; they are not a second application-level runner or a claim about production data.

### Supported replay subset

The PostgreSQL dialect parser (`sqlparser`) checks a closed AST subset before SQL reaches scratch. PostgreSQL itself determines the resulting catalog. Scripts are **not split on semicolons**; quoted semicolons remain intact.

Supported:

- ordinary `CREATE TABLE` with integer/smallint/bigint, text/varchar, boolean, numeric/decimal, real/double precision, date/timestamp, UUID and bytea built-ins;
- NULL/NOT NULL, literal defaults, primary/unique keys, managed-scope foreign keys and restricted CHECK expressions using identifiers, literals, comparisons, Boolean operators and basic arithmetic;
- `ALTER TABLE` add/drop/rename column, add/drop constraint, set/drop NOT NULL or a literal default, and built-in type changes without `USING`;
- ordinary column-only btree indexes (including uniqueness, ordering, INCLUDE and null-distinctness options);
- non-cascading managed-scope table/index drops. Explicit CASCADE is rejected, including column/constraint drops.

Unqualified names use the managed schema; PostgreSQL's implicit `pg_catalog` lookup precedes it for built-ins. Catalog deparsing uses a fixed `pg_catalog` search path for canonical comparisons. Cross-schema relation references are rejected.

Not accepted as migration scripts: functions/procedures/DO, SELECT or DML, COPY, function/default calls (including `now()` and `nextval()`), serial/identity/generated-column creation, casts in expressions, arbitrary queries, custom types/operators/access methods, views, sequences, triggers, grants/RLS changes, extensions, partitions/inheritance, session/transaction/role changes, or concurrent/non-transactional indexes. Even plausible benign unsupported SQL stays `DB_REPLAY_UNSUPPORTED`. This is intentionally narrower than PostgreSQL's full grammar. **Arbitrary SQL is not a sandbox, and READ ONLY alone does not establish purity.**

## Inspection coverage and drift

The adapter declares capabilities **per operation and observed engine version** for PostgreSQL 16–18. Other engines/versions are rejected. It reads only fixed catalog queries and a structurally validated ledger, in a dedicated `REPEATABLE READ READ ONLY` target transaction. It never prepares/executes application queries, invokes a user's check/routine, reads live application rows, or initializes target state.

Supported catalog fingerprints include:

- schema/table owners, schema/relation/column ACLs with sorted grant entries;
- ordinary heap tables, persistence, storage options and tablespaces;
- column order, precise type/typmod, array dimensions, collation/version, nullability, default/generated-expression identity, identity/generated flags, storage/compression;
- key/check/foreign-key definitions, validation/enforcement and deferrability state, including the enabled state of internal constraint triggers (without unstable generated trigger names/OIDs);
- index definitions **and valid/ready/live flags**, uniqueness, primary/exclusion/immediate state, null-distinctness, clustering and replica identity.

Quoted identifier components are unambiguously quoted in composite object keys. OIDs and ACL array order do not become semantic differences. The declared owner role maps to a typed managed-role marker in target/scratch, while every other role and the PUBLIC grantee remain distinct. Sequence counters are never read or fingerprinted.

Views/materialized views, sequences, routines, user triggers and RLS policies are inventoried but **opaque/unverified**. Partitioning/inheritance (including children outside the managed scope), foreign/custom types, non-btree or expression/partial indexes, custom collations/operator classes, extensions, event triggers, security labels, other namespace objects and external function/FK dependencies expose incomplete coverage. Unreadable objects or unknown ledger shape also block a clean result. This is not complete PostgreSQL semantic coverage, dependency closure, security-policy verification or replication/publication configuration auditing.

Engine-deparsed SQL, defaults, expressions, comments and storage option payloads are represented by **definition hashes**, not printed literals: schema definitions can themselves contain credentials. Object names, structural fields, owner/grant facts and object-level differences remain visible. This avoids echoing connection credentials, SQL error payloads or live row samples. It is not an SQL dump; review source SQL separately through appropriately authorized tooling.

Drift compares `observed` with `expected(applied_prefix)`, not repository HEAD. Differences name the affected object and show expected/observed properties or definition fingerprints. An incomplete comparison is `unknown`, never “no drift.” Unsupported pending SQL can block a candidate plan while a separately verified applied prefix still passes. `verify` does not replay pending SQL.

### Scoped execution ledger and legacy inspection

Execution uses `_ntnt.schema_migrations_v2`. Its first five NOT NULL columns are:

| Column | PostgreSQL type |
|---|---|
| position | bigint (one-based, consecutive) |
| id | text (canonical decimal ID) |
| name | text (SQL filename) |
| checksum | text (checksum v1) |
| state | text (`applied` for a clean prefix) |

v2 appends `logical_database text NOT NULL` and `managed_schema text NOT NULL`, in that order. Every row must bind the same selected logical store/schema. The table has a primary key on position. `_ntnt` and the ledger must be owned by the selected role, with no other grantees, defaults/generated columns, mutating triggers, check/FK constraints or rules before insertion. Unknown objects, RLS, custom/expression indexes, inheritance or unrecognized ledger formats block execution.

Read-only commands still recognize the earlier unscoped five-column `_ntnt.schema_migrations_v1` for portable historical inspection. **Execution refuses v1**, even if its checksums match: no silent rewriting or scope adoption occurs. Only an absent reserved namespace on a verified empty base may be bootstrapped. An empty existing v2 or arbitrary namespace is not trusted as a new base.

The relation must be an ordinary heap table with no inheritance, RLS, rewrite rules, custom/expression indexes, or unrecognized namespace objects. Inspection locks the physical ledger in ACCESS SHARE mode, validates its built-in types/structure before reading rows, and uses `row_security=off` as an additional fail-closed check. A view or RLS policy cannot masquerade as a ledger and invoke user SQL. Malformed ledger row values are not echoed in failure reports.

**Do not create or seed this ledger to adopt an existing database.** Legacy inspection tests seed v1 explicitly; execution/rehearsal tests use the real runner to create v2. Production baseline adoption is not implemented. An absent ledger plus nonempty managed scope requires deliberate later adoption; no checksum rewriting, fabricated history or automatic repair occurs.

## Results and plan identity

JSON has `format_version: 1`, UTC `observed_at`, stable diagnostic codes, source filenames/object keys and next actions. `pass`, `fail`, `unknown`, and `not_run` are distinct. Exit code 0 means the requested inspection/replay result passed within its declared coverage; other outcomes return nonzero. `status`, `plan` and `verify` report **`not_run`, zero executed** assertions; `migrate` separately counts the contracts actually attempted in that invocation. A catalog/history pass is not evidence of SQL contracts, data correctness, business invariants or deployed-client compatibility.

Plans bind:

- logical database and target names, captured manifest, ordered source/checksum identities and policy;
- cluster/database identity, database name, role/authority/memberships, engine and settings;
- managed scope, observed base history/catalog, applied prefix and replayed expected base;
- adapter/canonicalization version, exact pending migration identities and candidate fingerprint;
- explicit `execution.format = "postgresql-transactional-v1"` and ordered catalog fingerprints for every migration prefix, including the empty prefix. Old/incomplete artifacts cannot authorize execution.

`plan_digest` hashes that descriptor. Observation time and display filters are not authority inputs. `--against` checks the saved descriptor's digest, pending descriptors and fresh source/target/base binding. It neither executes nor approves the plan. For `plan --out`, the file contains the compact plan descriptor and result metadata, not duplicate catalog/source display fields; stdout still contains the full report. `status`/`verify --out` save their report. All saved artifacts share the reader's 1 MiB bound: oversized output fails before creating a file rather than producing an unreadable plan. Files are new-only (`--out` cannot replace a source, existing plan or symlink); on Unix, new reports are mode 0600.

Every saved plan has `authorized = false`, `executable = false`, `automatic_safe = false` (the artifact alone grants no capability), and a `requires_review` or `blocked` classification. Known drop/constraint-removal/cast/CASCADE/DML/authorization hazards are reported conservatively. Lock/scan labels are qualitative, not production duration estimates. Missing data checks and still-deployed application compatibility remain explicit. `migrate` validates the execution format and all prefix evidence under its own lock and requires independent provider authority; the digest alone is not an approval token.

## Bounds, cleanup and concurrency

Connect/authentication and every query/cleanup exchange have 5-second client deadlines; PostgreSQL statement and lock limits are 5 and 2 seconds, with a 10-second idle transaction timeout. Database orchestration has a 120-second outer deadline. Existing secrets providers retain their own bounded lookup/retry policy. Dedicated connection tasks are discarded on cancellation/error; sessions are never recycled into application pools. Runtime shutdown waits at most 100 milliseconds for uncancellable OS-resolver work, so cancelling a connect is not followed by an unbounded runtime join. Scratch rollback is checked, and cleanup failure cannot produce a verified candidate.

Inspection transactions are observations, not deployment locks. ACCESS SHARE protects the ledger relation while inspecting it; the target transaction is rolled back and its dedicated session discarded immediately after capturing identity, catalog and history, before scratch replay starts. Long replay cannot retain the target lock or hit its idle-transaction deadline. A repeatable-read snapshot does not exclude arbitrary concurrent administrator DDL, application writes, changes after observation or physical cluster cloning. Execution separately revalidates under migration ownership and native table locks. Empty replay does not prove an upgrade preserves existing rows, that a narrowing cast will succeed, or that a lock will be brief.

## Native regression tests

Ordinary `cargo nextest run --locked` runs portable inventory, parser, redaction, timeout and CLI tests. Real PostgreSQL tests are explicitly ignored unless selected, and **mandatory CI lanes execute them on PostgreSQL 16, 17 and 18**, with fixture-only control databases and separate randomly named target/scratch roles/databases for each test.

For a disposable local PostgreSQL instance (never production), set `NTNT_DB_TEST_ADMIN` to its administrative connection secret and `NTNT_DB_TEST_HOST` to its host or Unix-socket directory, then run:

```sh
cargo test --locked --test db_catalog_postgres -- --ignored --test-threads=2 --nocapture
cargo test --locked --test db_execution_postgres -- --ignored --test-threads=2 --nocapture
cargo test --locked --lib migrations::postgres::tests::target_transaction_is_really_read_only_even_for_superuser -- --ignored --exact --nocapture
```

These fixtures create/drop databases and roles. They test real read-only transactions, no target bootstrap, applied-prefix versus pending drift, invalid indexes, constraint validity/grants, immutable SQL/metadata history, malformed/RLS/view ledgers, unknown coverage, physical scratch collisions, low-privilege/empty/matching scratch, redaction, bounded waits, cleanup, source replacement during a blocked inspection, and target/source/authority-bound plans. Execution tests additionally cover fresh construction, populated upgrade preservation, exact authority, stale plans, partial commits and reruns, false/NULL/shape/type/error/timeout checks, native/advisory contention, snapshots after lock acquisition, process cancellation, unsafe/scoped ledgers, unsupported effects and source replacement. A test-only wire proxy drops connections before COMMIT, after durable COMMIT but before acknowledgement, during rollback, and during reconciliation. It is not linked into release code and exposes no authorization bypass. These executed fixture checks are not evidence about arbitrary application business behavior or real deployment compatibility.
