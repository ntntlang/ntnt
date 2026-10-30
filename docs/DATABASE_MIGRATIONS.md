# PostgreSQL and SQLite migrations, contracts and catalog evidence

`ntnt db` implements the PostgreSQL and SQLite transactional subsets of issue #245: read-only inspection/planning and separately authorized exact-plan execution. Both native adapters use one inventory, plan/approval protocol, history validation and execution/recovery loop. Keep an engine-native immutable SQL history for each logical store; this is portable tooling, **not portable SQL**, an ORM, or an authored desired-schema DSL.

```sh
ntnt db status --target development --json
ntnt db plan --target development --json --out review.json
ntnt db verify --target development --json
ntnt db verify --target development --against review.json --json
ntnt db migrate --database app --target development --plan review.json --json
ntnt db status --target development --object accounts
```

`status` and `verify` inspect history and compare the observed catalog with replay at the **applied prefix**. `plan` additionally replays pending SQL and captures a non-authorizing review artifact. Pending migrations are not drift. `--object` is a case-sensitive substring filter on displayed catalog keys; it never narrows verification, hides incomplete coverage, or changes the plan binding. Without `--json`, the same result is rendered for humans.

`migrate` requires a reviewed execution-capable plan and independent provider-backed approval. It never plans implicitly, auto-baselines, resets, repairs or retries application SQL after an uncertain commit. The existing `ntnt migrate` source-interpolation conversion command is unchanged. Explicit reviewed baseline adoption and recognized PostgreSQL legacy-history import use that same executor; see below. Concurrent indexes/non-transactional operations, backfill orchestration and query lint are not implemented. PostgreSQL migration DML remains unsupported; the SQLite subset includes bounded native DML for explicit mappings/rebuilds, not a resumable backfill system. Existing application-owned migration code is not invoked.

## Explicit configuration and secrets

The example below selects PostgreSQL. See [SQLite](#sqlite-native-files-and-rebuilds) for its file-backed configuration and distinct guarantees.

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

For PostgreSQL, use separate application and migrator credentials, and restrict schema DDL/ledger ownership to the migrator. SQLite has no database roles: restrict file/directory access and approval delivery through the operating system and provider, as described below. Development fixtures may use the environment provider; protected deployments use the existing Unix-socket provider and separately controlled approval delivery. NTNT cannot protect against someone who already controls that provider or direct database-owner credentials. Never publish resolved connection strings or provider values. Backup/PITR, restore testing, old-client compatibility and maintenance authorization remain separately operated responsibilities; a committed destructive change is not made reversible by transactional rollback.

Each PostgreSQL invocation uses a dedicated physical connection and a session advisory lock on the database-local `_ntnt` namespace (two-key lock `1853124212, 245`). The reserved namespace deliberately owns **one logical store/managed schema per physical database**; separate stores must use separate databases. This is not a multi-tenant ledger. The lock spans all per-migration commits and is bounded by the native lock timeout.

For each PostgreSQL migration, the runner:

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

Checks run in name order within each phase. Each must return **exactly one row, one Boolean column, and true** (SQLite represents this as integer `1`, not text or a floating-point value). False, NULL, no rows, multiple rows/columns, wrong type, SQL error and timeout fail the transaction. Reports include check name, phase, source and outcome, never returned values. Only actually attempted target/rehearsal checks count as executed assertions; zero checks remains `not_run`. A check pass before later rollback is not a committed schema guarantee.

The closed PostgreSQL AST subset supports Boolean expressions, identifiers/literals, comparisons and arithmetic, `IS NULL`/`IS NOT NULL`, `EXISTS`/`NOT EXISTS`, scalar subqueries, one managed heap table per SELECT, ordinary column GROUP BY/HAVING and builtin `count(*)`. Each query must have only these recognized clauses. Every table reference in a check must be explicitly qualified with the managed schema (for example, `app.accounts`), including nested queries. Unqualified names are rejected rather than resolved through `search_path`, so a managed table named `pg_class` cannot accidentally become a system-catalog read. Checks use `search_path=pg_catalog` for builtin function/operator lookup; this does not change unqualified migration-script names. The adapter verifies live catalog coverage and builtin base types/indexes, and conservatively rejects database-wide custom implicit/assignment casts before DDL or checks. Builtin operators/functions resolve through the trusted PostgreSQL installation, not a `provolatile` assertion on a user routine.

Not supported: arbitrary functions/routines, casts, aliases/joins, SELECT INTO, CTEs, locking clauses, table functions/sampling, windows, wildcard projections, custom types/operators/access methods, session/transaction changes or arbitrary SQL. A SELECT keyword or READ ONLY transaction is **not a purity proof**. Unsupported syntax is rejected before target I/O, never approved by regex. There are at most 64 named checks per migration, each at most 64 KiB; names use simple ASCII identifiers.

`ensures` are transition checks, not permanent invariants: `verify` does not re-execute historical postconditions. Use database constraints for lasting concurrency/future-write enforcement and ordinary native/Intent tests for business behavior. Intent IDs remain traceability, not proof. The SQLite-only `sqlite_rebuild` field below is the sole supported adapter-mode metadata. Other execution/transaction modes remain rejected.

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

PostgreSQL scratch must:

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

The PostgreSQL adapter declares capabilities **per operation and observed engine version** for PostgreSQL 16–18. SQLite declares its own coverage below; other engines/versions are rejected. It reads only fixed catalog queries and a structurally validated ledger, in a dedicated `REPEATABLE READ READ ONLY` target transaction. It never prepares/executes application queries, invokes a user's check/routine, reads live application rows, or initializes target state.

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
| position | bigint (one-based source position; an adopted checkpoint covers its prefix) |
| id | text (canonical decimal ID) |
| name | text (SQL filename) |
| checksum | text (checksum v1) |
| state | text (`applied`, explicit `adopted` checkpoint, or recognized `imported` legacy entry) |

v2 appends `logical_database text NOT NULL` and `managed_schema text NOT NULL`, in that order. Every row must bind the same selected logical store/schema. The table must be permanent (logged), with a valid, immediate primary key on position. Execution explicitly enables local synchronous commit. `_ntnt` and the ledger must be owned by the selected role, with no other grantees, defaults/generated columns, mutating triggers, check/FK constraints or rules before insertion. Unknown objects, RLS, custom/expression indexes, inheritance or unrecognized ledger formats block execution.

Read-only commands recognize the earlier unscoped five-column `_ntnt.schema_migrations_v1`. **Ordinary migration execution refuses v1**, even if its checksums match. Only an explicit adoption plan can import the strictly validated owned legacy ledger described below; no silent rewriting or scope adoption occurs. Ordinary execution bootstraps only an absent reserved namespace on a verified empty base. An empty existing v2 or arbitrary namespace is never trusted as a new base.

The relation must be an ordinary heap table with no inheritance, RLS, rewrite rules, custom/expression indexes, or unrecognized namespace objects. Inspection locks the physical ledger in ACCESS SHARE mode, validates its built-in types/structure before reading rows, and uses `row_security=off` as an additional fail-closed check. A view or RLS policy cannot masquerade as a ledger and invoke user SQL. Malformed ledger row values are not echoed in failure reports.

**Do not create or seed this ledger to adopt an existing database.** Legacy inspection tests seed v1 explicitly; execution/adoption rehearsals use the real runner. An absent ledger plus nonempty managed scope still fails ordinary inspection/planning; use the explicit reviewed adoption path below rather than fabricating applied history. No automatic repair or source-checksum rewriting occurs.

## Explicit adoption of existing databases

Use adoption when an existing PostgreSQL managed schema or SQLite file matches a
reviewed, replayable prefix of your immutable SQL inventory but has no NTNT ledger.
This is an **observed checkpoint**, not evidence that NTNT executed those old files.
The existing four-command workflow remains; there is no separate migration runner:

```sh
ntnt db plan --database app --target development --adopt-through 10 --out adoption.json --json
# Review SQL, reference/catalog differences, target identity and named data checks.
# Separately deliver this exact digest via the existing approval_secret provider.
ntnt db migrate --database app --target development --plan adoption.json --json
ntnt db verify --database app --target development --json
# Later changes use ordinary plan/migrate, with their own review and approval.
```

`--adopt-through` is a positive **migration ID**, not a file count. It is accepted
only on `plan`, not status, verify, migrate or alongside `--against`. The selected
ID must exist in captured source. Gaps in IDs remain valid. Supply the expected SQL
history/baseline yourself and review it; the tool does not dump the live target and
then declare that target to be its own reference. The whole selected prefix must
replay within the adapter's supported grammar in the same isolated scratch used
for ordinary planning. Only that prefix is replayed; future pending SQL is not
included in this adoption action. No old migration SQL executes on the target.

Declare **at least one named adoption check** in the selected database's manifest:

```toml
[databases.app.adoption_checks]
no_null_email = "SELECT NOT EXISTS (SELECT 1 FROM app.accounts WHERE email IS NULL)"
no_duplicates = "SELECT NOT EXISTS (SELECT email FROM app.accounts GROUP BY email HAVING count(*) > 1)"
```

Use `main.accounts` for SQLite. Names, query grammar, limits and true/NULL/type/shape
rules are the same as transition contracts (64 bounded checks; SQLite true is
integer `1`). Checks are sorted by name. These are adoption-operation policy,
separate from historical `requires`/`ensures`: changing the manifest invalidates
old plans without editing the legacy source/checksums you are trying to import.
Historical transition checks may be superseded and are never selected implicitly
as adoption checks. Check results live in the execution report; archive the
reviewed plan/report for audit. They are not a new permanent-invariant registry.

Planning remains readonly and performs **zero data assertions**. It binds the
requested prefix checksum, action kind, check names and exact captured manifest,
source/policy, logical selection, physical target and observed ledger/catalog.
It compares observed schema with the replayed requested baseline, not an assumed
empty target. Mismatch or incomplete coverage blocks adoption. The artifact uses
`postgresql-adoption-v1` or `sqlite-adoption-v1`, distinct from ordinary execution
formats, and still has no authority without the separately protected approval.

Execution uses the existing owned session/native locks and rechecks identity,
source, history and catalog after acquisition. It runs the captured adoption checks
against current data, then commits only the validated ledger change in that same
transaction. SQL errors, false checks, drift, wrong targets, unknown ledgers and
failed cleanup cannot become a successful adoption. An adoption plan never also
applies pending migrations; plan those separately afterward.

### Checkpoint and history truth

For a target with **no reserved ledger**, one scoped v2 record has:

- `state = "adopted"`;
- `position` equal to the number of covered source files, with the final file's ID/name;
- a domain-separated `ntnt-baseline-v1` checksum of the entire ordered prefix's
  exact source identities (including each SQL file and sidecar checksum).

It does not insert fictional `applied` rows for each old file. Later real migrations
append consecutive source positions with ordinary per-file checksums and `applied`
states. Keep the whole baseline prefix: edited SQL/metadata, inserted historical
IDs, missing files or a forged checkpoint cannot be treated as valid history.

Reports separate `covered_count`, `applied_count`, `adopted_count` and
`imported_count`. For example, a checkpoint covering three old files followed by
one real migration has four covered, three adopted and one applied. These describe
ledger evidence, not assertions executed in this invocation. Normal execution
progress labels covered old files `baseline_covered`; the adoption action explicitly
reports `migration_sql_executed = false`. Actual named checks still count in
`assertions.executed`. For saved-format compatibility, `binding.applied_count`
retains its historical spelling for the **covered-prefix index**; consumers should
use the separate history counts to distinguish adoption/import from execution.

A same-plan retry recognizes a matching committed checkpoint (`already_adopted`)
without rerunning old SQL or transition checks. New source/policy, later migrations
or an unrelated ledger extension require the appropriate fresh plan, not a reset.
Uncertain COMMIT acknowledgement uses the same fresh locked reconciliation as
normal migrations; unavailable evidence stays unknown and never triggers blind SQL
re-execution. Data-dependent checks run again only if adoption did not commit.

### Recognized legacy PostgreSQL history

The only supported legacy import is the unscoped `_ntnt.schema_migrations_v1`
format already recognized by this adapter. Its whole nonempty history must exactly
match the selected prefix: ordered positions, canonical IDs, filenames, checksums
and clean `applied` states. The requested ID must be that history's final ID.
Adoption still requires matching replayed schema and current named checks.

Under the existing migration/table locks, a separately approved `import_legacy_v1`
action validates the physical ledger as a permanent owned heap with a valid
immediate position primary key and no extra grants, defaults, mutating triggers,
rules, check/FK constraints or unsupported dependencies. It transactionally renames
the same table to v2, adds explicit logical-database/schema scope, and marks those
existing rows `imported`, preserving their source identities/checksums. A normal
migration plan cannot perform this import implicitly. Repeats report
`already_imported`; subsequent ordinary progress uses `history_imported` for those
rows. Import must be a leading scoped prefix, never mixed with an adopted checkpoint.

Unknown formats, empty/dirty/edited/ambiguous ledgers, missing source and unsafe
ownership/physical definitions are refused, not reconstructed, acknowledged or
repaired. There is no arbitrary Flyway/application-ledger importer and no SQLite
legacy-ledger import. Do not change checksums or seed a recognized-looking table to
bypass this boundary. Older NTNT versions do not understand the new states and
fail closed; retire old migration writers before adopting/importing a deployment.

### Rehearsal and operational boundary

The [customer/order example](../examples/database-adoption/README.md) executes the
whole existing-app → inspect → plan/adopt → upgrade → rerun → verify workflow on
disposable SQLite files. Its native test runs old and new `.tnt` application readers
and asserts exact retained values plus the expansion's default. Shared native
fixtures exercise adoption on both adapters, including real PostgreSQL legacy
imports and commit-fault reconciliation.

This completes the transactional foundation's explicit adoption path **within the
published inspection/SQL coverage**. It is not blanket production rollout approval,
automatic baseline generation, full schema coverage, proof of arbitrary business
mappings or a backup/restore system. Archive reviewed artifacts, arrange a tested
recovery path and retire conflicting old writers; actual application deployment
requires its own authorization. Concurrent/nontransactional DDL and schema-aware
query analysis remain later work.

## SQLite native files and rebuilds

```toml
[databases.local]
engine = "sqlite"
schema = "main"
migrations_dir = "migrations/local"
approval_secret = "LOCAL_MIGRATION_APPROVAL"

[databases.local.targets.development]
connection_secret = "LOCAL_DATABASE_FILE"

[databases.local.scratch]
connection_secret = "LOCAL_REPLAY_FILE"
disposable = true

[secrets.LOCAL_DATABASE_FILE]
[secrets.LOCAL_REPLAY_FILE]
[secrets.LOCAL_MIGRATION_APPROVAL]
```

Resolve the connection secrets to **absolute native paths of existing regular files**, not URLs, SQLite URIs, `:memory:`, or connection strings. Provision the empty deployment and scratch files yourself with SQLite first. Do not declare `database`/`role` for SQLite; its managed scope is exactly `main`, the entire file except the reserved ledger. There is no ATTACH, temp namespace, multi-store file, implicit creation or reset. Existing-file adoption is explicit and subject to the same scope/coverage checks.

Files must have one hard link and no symlink path components. Filename components cannot contain `:` on any platform (a Windows drive prefix is allowed); NTFS alternate streams have independent contents but share file-level identity and are not supported. Use the native canonical path on macOS (for example `/private/...`, not its `/var` alias). Identity binds native device/inode or Windows volume/file-index plus available creation time, not merely path spelling. Plans also bind the bundled SQLite version, encoding, page size, auto-vacuum and journal mode, with Unix owner/group/mode or Windows readonly metadata. File paths themselves are not disclosed. Replacement/copying changes identity and requires a new plan. The Unix adapter checks SQLite's open-file movement status without opening and closing a second descriptor that could drop SQLite's POSIX locks; Windows retains a non-delete-sharing guard. Filesystem administrators, local directories and OS ACL administration remain trusted. These checks are **not a filesystem sandbox or full ACL attestation**; use local storage with reliable SQLite locking, not network/shared filesystems.

### Transactions, replay and contracts

Support is for the bundled **SQLite 3.45 series**, not an arbitrary separately installed `sqlite3` executable. `status`/`plan`/`verify` open the target readonly and use a deferred snapshot, releasing it before scratch replay. Catalog inspection reads metadata and compiles, but never steps, application programs with a native authorizer to validate their supported effect paths. It does not scan application rows. Scratch uses a distinct physical file, separate secret and matching settings; `BEGIN IMMEDIATE` owns its rollback-only replay. A nonempty scratch file is rejected, never cleared.

Execution opens an existing file only after approval and file-identity checks. It acquires **`BEGIN IMMEDIATE` before inspecting history/catalog for each migration**, revalidating on every acquisition; the writer reservation is not retained between commits. Competing migrators recognize each other's matching committed prefixes instead of applying SQL twice. SQLite's writer lock covers ordinary application writers; it is not an online-DDL guarantee. Dedicated connections enable foreign keys, recursive triggers and CHECK enforcement, `trusted_schema=OFF`, defensive mode, strict quoting, and `synchronous=FULL`. Durable rollback-journal modes and WAL are accepted; journal-off/memory modes are not. The runner does not change the selected persistent journal mode.

The two-second busy timeout, five-second worker/progress deadline, and shared orchestration/reconciliation deadlines bound operations. Timeout/cancellation interrupts and discards the connection, never returns it to a pool. Native I/O still depends on the OS/filesystem. A busy or interrupted COMMIT is reconciled using a fresh connection and writer reservation, without retrying SQL; if the fresh observation cannot establish the prefix, the result stays unknown. Process termination cannot return acknowledgement: rerun the same approved plan to inspect committed truth first. A hot rollback journal may prevent readonly inspection (`DB_SQLITE_RECOVERY_REQUIRED`); wait for an active writer to finish, or rerun `migrate` with that same approved plan so native recovery and locked reconciliation occur through the authorized path. Do not delete the journal or silently make inspection writable.

SQL, named transition checks, the resulting catalog and the success ledger row are one transaction. SQLite contracts use the same closed SELECT expression grammar described above, with every relation qualified as `main.<table>`; the native authorizer additionally denies side-effecting calls and nonmanaged relations. Only **one integer `1`**, in one row/column, passes. Numeric values other than `1`, floating-point `1.0`, text, NULL, incorrect shape, errors and timeouts fail. Historical checks remain `not_run` on verify/rerun.

### Supported SQL and catalog coverage

The SQLite dialect parser accepts ordinary table/index/view/trigger DDL and INSERT/UPDATE/DELETE, while the native authorizer mediates resolved reads, writes and calls. Native SQLite statement boundaries preserve trigger bodies and quoted semicolons; scripts are not split with string operations. Tables, columns/order/type/defaults, PK/UNIQUE/CHECK/FK definitions, ordinary column indexes and builtin collations (`BINARY`, `NOCASE`, `RTRIM`) are fingerprinted. WITHOUT ROWID and STRICT flags are retained. Supported views and table triggers have definition hashes and native compile checks, **not inferred business semantics or complete transitive-effect explanations**. SQL definitions/defaults remain redacted hashes; final catalogs are engine-native, not normalized into PostgreSQL definitions.

Only concrete managed relations are eligible. `CREATE TABLE ... AS SELECT` is rejected: native authorization of a pending table name must not authorize a same-named virtual-table source. Express copies as ordinary `CREATE TABLE` followed by `INSERT ... SELECT`, with resolved relations refreshed between native statements. Eponymous virtual/table functions such as `json_each` or `pragma_*` cannot masquerade as ordinary `main` tables. Builtin `count` is available to checks; migration/definition programs additionally allow the native rename helpers and the small builtin set `like`, `substr`, `length`, `printf`, `quote`, `coalesce`, `ifnull`, `lower`, `upper`, `trim`, `abs`, `typeof`. A known function name is not permission to load extensions or register application functions.

Virtual/shadow tables, generated columns, AUTOINCREMENT/`sqlite_sequence`, expression/partial indexes, custom collations/functions, unsafe/unsupported view or trigger programs, statistics/internal objects, bind parameters, ATTACH/DETACH, authored PRAGMAs, transaction/savepoint/session control and extension loading are unsupported. Unsupported catalog coverage blocks a clean result. DML and trigger effects remain review-required (`SQLITE_EFFECTS_NOT_INFERRED`); metadata never turns an unknown effect into automatic safety. Large transformations belong in separately operated application jobs.

The reserved `_ntnt*` namespace contains only `_ntnt_schema_migrations_v2`, a STRICT table with the same seven ordered scope/history fields as PostgreSQL (`position INTEGER`, the others TEXT). Only the exact native ledger definition is recognized; extra indexes, triggers, views or columns block mutation. A missing ledger can be bootstrapped only at a verified empty base for ordinary migrations, or by an independently approved matching-baseline adoption. An empty existing ledger, unknown history or nonempty unadopted file is not a new database. Do not seed this table as an adoption shortcut.

### Explicit one-table rebuild

SQLite schema changes often require **create → explicitly map/copy rows → drop old table → rename replacement → recreate dependencies**. Author the native SQL yourself; NTNT never infers a mapping or translates PostgreSQL DDL. For a supported rebuild, place this in its SQL sidecar:

```toml
sqlite_rebuild = "accounts"

[requires]
no_null_email = "SELECT NOT EXISTS (SELECT 1 FROM main.accounts WHERE email IS NULL)"

[ensures]
no_null_email = "SELECT NOT EXISTS (SELECT 1 FROM main.accounts WHERE email IS NULL)"
```

The name must be a simple ordinary table identifier. In the already-owned transaction, this mode snapshots table row counts and supported definitions, temporarily disables foreign-key actions through SQLite's **native connection configuration API** (authored `PRAGMA foreign_keys=OFF` is still rejected), and executes the captured mapping. This prevents implicit `ON DELETE CASCADE` during the drop/rebuild from deleting child rows. Before postconditions/ledger/commit, it:

- checks all foreign keys against the rebuilt catalog/data;
- requires the same set of tables and the same row count in each;
- preserves every existing named index, view and trigger definition and every other table's definition;
- restores foreign-key enforcement, including on failure; failure rolls back or discards the session.

Explicitly recreate the selected table's named indexes/triggers and any views dropped to allow the rebuild. The selected table's columns/constraints and autoindexes may change. Definition comparisons are conservative: equivalent SQL written differently may still be rejected. Deliberate dependent-object retirement belongs in a separate reviewed migration, not an accidental omission inside this mode. The mode scans row counts **during authorized execution**, not during production planning.

Counts and FK checks do not prove row identities, correct value mappings, every uniqueness assumption, old-client compatibility or recovery after a committed destructive change. Use named transition checks and the same executor in representative native upgrade tests to assert exact keys/values, preserved children and working dependencies. `tests/db_sqlite.rs` demonstrates an email-lowercasing rebuild with existing rows, indexes, views, audit trigger and cascading foreign keys, plus negative dependency/row-loss/orphan cases. Empty replay alone cannot provide that evidence. Unknown business effects remain review-required even when these guards pass.

## Results and plan identity

JSON has `format_version: 1`, UTC `observed_at`, stable diagnostic codes, source filenames/object keys and next actions. `pass`, `fail`, `unknown`, and `not_run` are distinct. Exit code 0 means the requested inspection/replay result passed within its declared coverage; other outcomes return nonzero. `status`, `plan` and `verify` report **`not_run`, zero executed** assertions; `migrate` separately counts the contracts actually attempted in that invocation. A catalog/history pass is not evidence of SQL contracts, data correctness, business invariants or deployed-client compatibility.

Plans bind:

- logical database and target names, captured manifest, ordered source/checksum identities and policy;
- native cluster/database or file identity, engine and settings, and adapter-specific role/authority facts;
- managed scope, observed base history/catalog, applied prefix and replayed expected base;
- adapter/canonicalization version, exact pending migration identities and candidate fingerprint;
- explicit `execution.format = "postgresql-transactional-v1"` or `"sqlite-transactional-v1"` and ordered engine-specific catalog fingerprints for every migration prefix, including the empty prefix. Old/incomplete artifacts cannot authorize execution.

`plan_digest` hashes that descriptor. Observation time and display filters are not authority inputs. `--against` checks the saved descriptor's digest, pending descriptors and fresh source/target/base binding. It neither executes nor approves the plan. For `plan --out`, the file contains the compact plan descriptor and result metadata, not duplicate catalog/source display fields; stdout still contains the full report. `status`/`verify --out` save their report. All saved artifacts share the reader's 1 MiB bound: oversized output fails before creating a file rather than producing an unreadable plan. Files are new-only (`--out` cannot replace a source, existing plan or symlink); on Unix, new reports are mode 0600.

Every saved plan has `authorized = false`, `executable = false`, `automatic_safe = false` (the artifact alone grants no capability), and a `requires_review` or `blocked` classification. Known drop/constraint-removal/cast/CASCADE/DML/authorization hazards are reported conservatively. Lock/scan labels are qualitative, not production duration estimates. Missing data checks and still-deployed application compatibility remain explicit. `migrate` validates the execution format and all prefix evidence under its own lock and requires independent provider authority; the digest alone is not an approval token.

## Bounds, cleanup and concurrency

Connect/authentication and every query/cleanup exchange have 5-second client deadlines; PostgreSQL statement and lock limits are 5 and 2 seconds, with a 10-second idle transaction timeout. Database orchestration has a 120-second outer deadline; migration work reserves the final 20 seconds for fresh reconciliation if a work deadline interrupts COMMIT. If reconciliation itself cannot finish, progress remains unknown rather than guessing. Existing secrets providers retain their own bounded lookup/retry policy. Dedicated connection tasks are discarded on cancellation/error; sessions are never recycled into application pools. Runtime shutdown waits at most 100 milliseconds for uncancellable OS-resolver work, so cancelling a connect is not followed by an unbounded runtime join. Scratch rollback is checked, and cleanup failure cannot produce a verified candidate.

Inspection transactions are observations, not deployment locks. ACCESS SHARE protects the ledger relation while inspecting it; the target transaction is rolled back and its dedicated session discarded immediately after capturing identity, catalog and history, before scratch replay starts. Long replay cannot retain the target lock or hit its idle-transaction deadline. A repeatable-read snapshot does not exclude arbitrary concurrent administrator DDL, application writes, changes after observation or physical cluster cloning. Execution separately revalidates under migration ownership and native table locks. Empty replay does not prove an upgrade preserves existing rows, that a narrowing cast will succeed, or that a lock will be brief.

## Native regression tests

Ordinary `cargo nextest run --locked` runs portable inventory, parser, redaction, timeout and CLI tests, the real-file SQLite lifecycle/rebuild/failure tests, and SQLite shared conformance. These are not ignored and run on Linux, macOS and Windows CI. Real PostgreSQL tests are explicitly ignored unless selected, and **mandatory CI lanes execute them on PostgreSQL 16, 17 and 18**, with fixture-only control databases and separate randomly named target/scratch roles/databases for each test.

For a disposable local PostgreSQL instance (never production), set `NTNT_DB_TEST_ADMIN` to its administrative connection secret and `NTNT_DB_TEST_HOST` to its host or Unix-socket directory, then run:

```sh
cargo test --locked --test db_catalog_postgres -- --ignored --test-threads=2 --nocapture
cargo test --locked --test db_execution_postgres -- --ignored --test-threads=2 --nocapture
cargo test --locked --test db_migration_conformance -- --include-ignored --test-threads=2 --nocapture
cargo test --locked --lib migrations::postgres::tests::target_transaction_is_really_read_only_even_for_superuser -- --ignored --exact --nocapture
```

The shared conformance suite executes the same history/drift, approval/stale/wrong-target, contract failure and committed-prefix recovery scenarios on both native adapters. It also exercises populated baseline adoption, exact multi-file coverage, required check failures, unknown ledgers and later upgrades. PostgreSQL cases import only recognized immutable legacy history and reject unsafe ledger definitions. Adoption commit-fault tests distinguish baseline/import records from SQL execution; SQLite cases cover competing adopters, cancellation/timeout and busy-COMMIT recovery. SQLite-only tests also cover physical file aliases/replacement, read-only nonmutation, WAL, bounded busy waits, competing CLI processes, forced process death before commit and after an acknowledged-to-SQLite prefix, busy-COMMIT fresh reconciliation, timeout/discard, ledger tampering, redaction and populated rebuild preservation. Native SQLite fixtures provision disposable local files only; no service is needed.

PostgreSQL fixtures create/drop databases and roles. They test real read-only transactions, no target bootstrap, applied-prefix versus pending drift, invalid indexes, constraint validity/grants, immutable SQL/metadata history, malformed/RLS/view ledgers, unknown coverage, physical scratch collisions, low-privilege/empty/matching scratch, redaction, bounded waits, cleanup, source replacement during a blocked inspection, and target/source/authority-bound plans. Execution tests additionally cover fresh construction, populated upgrade preservation, exact authority, stale plans, partial commits and reruns, false/NULL/shape/type/error/timeout checks, native/advisory contention, snapshots after lock acquisition, process cancellation, unsafe/scoped ledgers, unsupported effects and source replacement. A test-only wire proxy drops connections before COMMIT, after durable COMMIT but before acknowledgement, during rollback, and during reconciliation. It is not linked into release code and exposes no authorization bypass. These executed fixture checks are not evidence about arbitrary application business behavior or real deployment compatibility.
