# Database catalog, history and plans

`ntnt db` is the **read-only-target first increment** of issue #245, not a complete migration runner. It keeps native PostgreSQL SQL and one immutable migration history. It does not implement an ORM or an authored desired-schema DSL.

```sh
ntnt db status --target development --json
ntnt db plan --target development --json --out review.json
ntnt db verify --target development --json
ntnt db verify --target development --against review.json --json
ntnt db status --target development --object accounts
```

`status` and `verify` inspect history and compare the observed catalog with replay at the **applied prefix**. `plan` additionally replays pending SQL and captures a non-authorizing review artifact. Pending migrations are not drift. `--object` is a case-sensitive substring filter on displayed catalog keys; it never narrows verification, hides incomplete coverage, or changes the plan binding. Without `--json`, the same result is rendered for humans.

There is deliberately **no `ntnt db migrate`** yet. The existing `ntnt migrate` source-interpolation conversion command is unchanged. Target execution, SQL contracts, populated-data rehearsal, SQLite, baseline adoption, non-transactional recovery, and query lint are later increments. Existing application-owned migration code is not replaced or invoked by these commands.

## Explicit configuration and secrets

`--manifest` defaults to `ntnt.toml` in the current directory; there is no implicit ancestor/target selection. The migration directory is relative to that manifest, not the caller's working directory.

```toml
[database]
engine = "postgresql"
schema = "app"
migrations_dir = "migrations"

[database.targets.development]
connection_secret = "APP_INSPECTION_DATABASE"
database = "app_development"
role = "app_owner"

[database.scratch]
connection_secret = "APP_REPLAY_DATABASE"
database = "app_replay"
role = "app_replay_owner"
disposable = true

[secrets.APP_INSPECTION_DATABASE]
required = true

[secrets.APP_REPLAY_DATABASE]
required = true
```

- Select a named target with `--target`. There is no fallback connection, URL argument, `url_env` alias, plaintext manifest credential, or `--force` bypass. Unknown database fields are errors.
- Connections reuse the existing **`std/secrets` providers and declaration validation**. Development's environment provider reads the exact logical name. Production uses the configured Unix-socket provider; the environment provider remains disabled in production. See [Secrets Agent Protocol](secrets-agent-protocol.md). Declarations are read from the captured manifest, not reopened during replay.
- A connection secret must specify exactly one host, the declared `user`/`dbname`, and **`sslmode=disable`**, with no startup `options`. This initial adapter reuses the existing NoTLS driver. Use a trusted local Unix socket or a separately secured local transport; do not send credentials over an untrusted plaintext network. TLS transport is not implemented and there is no silent TLS downgrade.
- Database, role and managed-schema names use ASCII letters/digits/underscores, start with a letter, and are at most 63 bytes. SQL object identifiers may be quoted. System schemas, `_ntnt`, and `public` cannot be the managed schema.
- The **target managed schema must already exist**. Commands do not create it or `_ntnt`. Empty, explicitly provisioned managed schemas can be planned without a ledger. A nonempty unrecognized scope is not assumed to have an empty history.
- The declared target role is the logical owner for expected owner/grant canonicalization. Scratch uses its separate owner role. Other owners/grantees remain distinct; ownership and grants are never stripped wholesale. Multi-owner histories or additional grants that cannot be reproduced by the supported history will not receive a clean comparison. Target authority/membership facts are also bound into plans.

## Immutable SQL inventory

```text
migrations/
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

The only optional sidecar metadata currently implemented is traceability:

```toml
intent_ids = ["account-timezone"]
```

`requires`, `ensures`, execution/transaction modes and all other metadata are **rejected**, not ignored or counted as executed assertions. Intent IDs are traceability, not proof.

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

Replay owns a dedicated physical connection and a **rollback-only outer transaction**, not a pool handle. It creates the managed scratch schema only after isolation/emptiness checks, replays captured scripts through the native execution primitive, captures the applied-prefix/candidate catalogs, and explicitly rolls back. Success and failure tests verify no scratch schema survives. This shared primitive is the extension point for the later authorized per-migration executor; scratch is not a second application-level runner. This increment does not claim real per-migration commits, transactional checks/ledger writes, migration locking, or populated-data safety.

### Supported replay subset

The PostgreSQL dialect parser (`sqlparser`) checks a closed AST subset before SQL reaches scratch. PostgreSQL itself determines the resulting catalog. Scripts are **not split on semicolons**; quoted semicolons remain intact.

Supported:

- ordinary `CREATE TABLE` with integer/smallint/bigint, text/varchar, boolean, numeric/decimal, real/double precision, date/timestamp, UUID and bytea built-ins;
- NULL/NOT NULL, literal defaults, primary/unique keys, managed-scope foreign keys and restricted CHECK expressions using identifiers, literals, comparisons, Boolean operators and basic arithmetic;
- `ALTER TABLE` add/drop/rename column, add/drop constraint, set/drop NOT NULL or a literal default, and built-in type changes without `USING`;
- ordinary column-only btree indexes (including uniqueness, ordering, INCLUDE and null-distinctness options);
- managed-scope table/index drops, including explicitly hazardous CASCADE.

Unqualified names use the managed schema; PostgreSQL's implicit `pg_catalog` lookup precedes it for built-ins. Catalog deparsing uses a fixed `pg_catalog` search path for canonical comparisons. Cross-schema relation references are rejected.

Not replayed: functions/procedures/DO, SELECT or DML, COPY, function/default calls (including `now()` and `nextval()`), serial/identity/generated-column creation, casts in expressions, arbitrary queries, custom types/operators/access methods, views, sequences, triggers, grants/RLS changes, extensions, partitions/inheritance, session/transaction/role changes, or concurrent/non-transactional indexes. Even plausible benign unsupported SQL stays `DB_REPLAY_UNSUPPORTED`. This is intentionally narrower than PostgreSQL's full grammar. **Arbitrary SQL is not a sandbox, and READ ONLY alone does not establish purity.**

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

### Ledger protocol, not an adoption recipe

The reserved recognized relation is `_ntnt.schema_migrations_v1`, with exactly these NOT NULL columns in order:

| Column | PostgreSQL type |
|---|---|
| position | bigint (one-based, consecutive) |
| id | text (canonical decimal ID) |
| name | text (SQL filename) |
| checksum | text (checksum v1) |
| state | text (`applied` for a clean prefix) |

The relation must be an ordinary heap table with no inheritance, RLS, rewrite rules, custom/expression indexes, or unrecognized namespace objects. Inspection locks the physical ledger in ACCESS SHARE mode, validates its built-in types/structure before reading rows, and uses `row_security=off` as an additional fail-closed check. A view or RLS policy cannot masquerade as a ledger and invoke user SQL. Malformed ledger row values are not echoed in failure reports.

**Do not create or seed this ledger to adopt an existing database.** The tests seed it explicitly as fixtures for the future executor. Production baseline adoption is not implemented. An absent ledger plus nonempty managed scope requires deliberate later adoption; no checksum rewriting, fabricated history or automatic repair occurs.

## Results and plan identity

JSON has `format_version: 1`, UTC `observed_at`, stable diagnostic codes, source filenames/object keys and next actions. `pass`, `fail`, `unknown`, and `not_run` are distinct. Exit code 0 means the requested inspection/replay result passed within its declared coverage; other outcomes return nonzero. `assertions` always reports **`not_run`, zero executed** in this increment. A catalog/history pass is not evidence of SQL contracts, data correctness, business invariants or deployed-client compatibility.

Plans bind:

- captured manifest, ordered source/checksum identities and policy;
- cluster/database identity, database name, role/authority/memberships, engine and settings;
- managed scope, observed base history/catalog, applied prefix and replayed expected base;
- adapter/canonicalization version, exact pending migration identities and candidate fingerprint.

`plan_digest` hashes that descriptor. Observation time and display filters are not authority inputs. `--against` checks the saved descriptor's digest, pending descriptors and fresh source/target/base binding. It neither executes nor approves the plan. Files are new-only (`--out` cannot replace a source, existing plan or symlink); on Unix, new reports are mode 0600.

Every plan has `authorized = false`, `executable = false`, `automatic_safe = false`, and a `requires_review` or `blocked` classification. Known drop/constraint-removal/cast/CASCADE/DML/authorization hazards are reported conservatively. Lock/scan labels are qualitative, not production duration estimates. Missing data checks and still-deployed application compatibility remain explicit. A later executor must revalidate under its own lock and require external authority; this digest is not an approval token.

## Bounds, cleanup and concurrency

Connect/authentication and every query/cleanup exchange have 5-second client deadlines; PostgreSQL statement and lock limits are 5 and 2 seconds, with a 10-second idle transaction timeout. Database orchestration has a 120-second outer deadline. Existing secrets providers retain their own bounded lookup/retry policy. Dedicated connection tasks are discarded on cancellation/error; sessions are never recycled into application pools. Runtime shutdown waits at most 100 milliseconds for uncancellable OS-resolver work, so cancelling a connect is not followed by an unbounded runtime join. Scratch rollback is checked, and cleanup failure cannot produce a verified candidate.

These are observations, not deployment locks. ACCESS SHARE protects the ledger relation while inspecting it; a repeatable-read snapshot does not exclude arbitrary concurrent administrator DDL, application writes, changes after observation or physical cluster cloning. Future execution must revalidate under migration ownership. Empty replay does not prove an upgrade preserves existing rows, that a narrowing cast will succeed, or that a lock will be brief.

## Native regression tests

Ordinary `cargo nextest run --locked` runs portable inventory, parser, redaction, timeout and CLI tests. Real PostgreSQL tests are explicitly ignored unless selected, and **mandatory CI lanes execute them on PostgreSQL 16, 17 and 18**, with fixture-only control databases and separate randomly named target/scratch roles/databases for each test.

For a disposable local PostgreSQL instance (never production), set `NTNT_DB_TEST_ADMIN` to its administrative connection secret and `NTNT_DB_TEST_HOST` to its host or Unix-socket directory, then run:

```sh
cargo test --locked --test db_catalog_postgres -- --ignored --test-threads=2 --nocapture
cargo test --locked --lib migrations::postgres::tests::target_transaction_is_really_read_only_even_for_superuser -- --ignored --exact --nocapture
```

These fixtures create/drop databases and roles. They test real read-only transactions, no target bootstrap, applied-prefix versus pending drift, invalid indexes, constraint validity/grants, immutable SQL/metadata history, malformed/RLS/view ledgers, unknown coverage, physical scratch collisions, low-privilege/empty/matching scratch, redaction, bounded waits, cleanup, source replacement during a blocked inspection, and target/source/authority-bound plans. Application behavior, business preservation and crash/commit reconciliation belong to the later executor tests, not invented evidence here.
