# PostgreSQL and SQLite migrations, contracts and catalog evidence

`ntnt db` implements the PostgreSQL and SQLite transactional subsets of issue #245: read-only inspection/planning and separately authorized exact-plan execution. Both native adapters use one inventory, plan/approval protocol, history validation and execution/recovery loop. Keep an engine-native immutable SQL history for each logical store; this is portable tooling, **not portable SQL**, an ORM, or an authored desired-schema DSL.

```sh
ntnt db init --engine postgresql
ntnt db new add_accounts
ntnt db status --target development --json
ntnt db plan --target development --json --out review.json
ntnt db verify --target development --json
ntnt db verify --target development --against review.json --json
ntnt db migrate --database app --target development --plan review.json --json
ntnt db status --target development --object accounts
```

`init` and `new` only write local scaffolding: a manifest section that names secrets, and empty numbered migration files (see [Quick start](#quick-start)). `status` and `verify` inspect history and compare the observed catalog with replay at the **applied prefix**. `plan` additionally replays pending SQL and captures a non-authorizing review artifact. Pending migrations are not drift. `--object` is a case-sensitive substring filter on displayed catalog keys; it never narrows verification, hides incomplete coverage, or changes the plan binding. Without `--json`, the same result is rendered for humans.

`migrate` requires a reviewed execution-capable plan and independent provider-backed approval. It never plans implicitly, auto-baselines, resets, repairs or retries application SQL after an uncertain commit. The existing `ntnt migrate` source-interpolation conversion command is unchanged. Explicit reviewed baseline adoption and recognized PostgreSQL legacy-history import use that same executor; see below. The only non-transactional operation is a lone PostgreSQL `CREATE INDEX CONCURRENTLY` (see [Concurrent index builds](#concurrent-index-builds)); backfill orchestration and query lint are not implemented. PostgreSQL migrations may include transactional DML such as seed rows; the SQLite subset includes bounded native DML for explicit mappings/rebuilds, not a resumable backfill system. Existing application-owned migration code is not invoked.

## Quick start

Both paths use the development environment secrets provider. Run them from the
project directory that holds `ntnt.toml`. They are for local development only.
Production uses the Unix-socket secrets provider and a separately protected
approval; see the rest of this document before deploying.

Two commands write the starting files. Neither reads a secret value or connects
to a database:

```sh
ntnt db init --engine sqlite    # or --engine postgresql
ntnt db new initial             # creates migrations/0001_initial.sql
```

`init` writes the commented `ntnt.toml` configuration shown below for the chosen
engine, declares its secret names (never values) and creates `migrations/`. It
prints the remaining steps; their commands, like the `plan` command `new` prints,
keep any `--manifest` and `--database` selection, quoted for a POSIX shell.
`--database <name>` (lowercase ASCII letters, digits
and underscores, at most 50 characters) replaces `app`; the secret names and the
PostgreSQL schema, role and database names derive from it. If `ntnt.toml` exists,
`init` appends the section and keeps every existing byte. It refuses, changing
nothing, when the manifest already declares that database or one of those secrets,
or when another declared database already uses `migrations/`. `new` numbers the
file after the highest existing ID, zero-padded to four digits (or wider, to match
existing files). Migration names use lowercase ASCII letters, digits and
underscores, and `new` never overwrites a file. Writing the same files by hand
works just as well.

`init` and `new` take turns through an exclusive lock on `ntnt.toml`. `init` holds
it from reading the manifest until it has written it, and `new` from reading the
manifest, before it lists `migrations/`, until the new file exists. Neither waits
for the lock. While another `init` or `new` holds it, `init` refuses with
`DB_INIT_CONFLICT` instead of overwriting another init's section, and `new` refuses
with `DB_NEW_CONFLICT` instead of picking the ID another `new` is about to use. The
refused command changes nothing; rerun it once the other has finished. The lock
ends when the command exits, so it never needs cleaning up. One refusal leaves a
file: when `init` has just created `ntnt.toml` and a `new` locks it first, `init`
refuses with `DB_INIT_CONFLICT` and the new manifest stays empty; rerun `init` to
fill it. The lock belongs to one manifest, so two manifests that share a
migrations directory do not take turns; an ID both of them pick is refused as
`DB_DUPLICATE_ID` before anything runs. Editors do not take
that lock: `init` also refuses when it sees that the manifest changed, but an edit
saved just before `init` replaces the file can be lost, so do not run `init` while
`ntnt.toml` is being edited. No local lock helps across branches: two branches
that each add the next migration still produce two files with the same ID.
`status` and `plan` reject that history with `DB_DUPLICATE_ID`; renumbering the
migration that has not been applied anywhere fixes it.

`new` never changes `ntnt.toml`, but like `init` it opens the file for writing to
lock it, because NFS grants the lock only through such a handle. A `ntnt.toml` that
`new` may not write is locked through a read-only handle instead: that works on
local filesystems, and on NFS `new` fails with `DB_SCAFFOLD_IO`. Both commands fail
that way on a filesystem without file locks. On Windows, file locks are mandatory,
so while `init` or `new` holds the lock no other program can read `ntnt.toml`: an
`ntnt db status` or `plan` that reads it at that moment fails with `DB_INPUT_IO`
and must be rerun.

For a ready-made local setup, [`examples/database-dev`](../examples/database-dev/README.md)
starts PostgreSQL with matching target and scratch databases in one
`docker compose up -d --wait`, and has the SQLite equivalent. Each comes with a
manifest, a `dev.env` and a first migration; `tests/db_dev_recipe.rs` runs both
exactly as documented.

### SQLite

`ntnt db init --engine sqlite` writes this manifest (its comments are omitted here):

```toml
# ntnt.toml
[databases.app]
engine = "sqlite"
schema = "main"
migrations_dir = "migrations"
approval_secret = "APP_MIGRATION_APPROVAL"

[databases.app.targets.development]
connection_secret = "APP_DATABASE_FILE"

[databases.app.scratch]
connection_secret = "APP_REPLAY_FILE"
disposable = true

[secrets.APP_DATABASE_FILE]
[secrets.APP_REPLAY_FILE]
[secrets.APP_MIGRATION_APPROVAL]
```

Write the first migration below the comment header of the file `ntnt db new initial` created. Until it holds a statement, `plan` refuses the file with `DB_MIGRATION_EMPTY`:

```sql
-- migrations/0001_initial.sql
CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    email TEXT NOT NULL UNIQUE
);
```

`ntnt db` never creates database files. Create an empty target file and a separate
empty scratch file (an empty file is a valid SQLite database). Then point the secrets
at their **absolute** paths:

```sh
mkdir -p data && touch data/app.sqlite data/replay.sqlite
export NTNT_ENV=development NTNT_SECRETS_PROVIDER=env
export APP_DATABASE_FILE="$PWD/data/app.sqlite"
export APP_REPLAY_FILE="$PWD/data/replay.sqlite"

ntnt db status --target development
ntnt db plan --target development --out review.json
# Review review.json and the SQL. Then approve exactly this plan:
export APP_MIGRATION_APPROVAL="$(jq -r .plan_digest review.json)"
ntnt db migrate --target development --plan review.json
ntnt db verify --target development
```

The scratch can stay a `touch`ed empty file after your application has opened
the target: `std/db/sqlite` `connect()` switches a database to WAL mode, and
`ntnt db` gives an empty scratch the target's journal mode before replaying.

`migrate` without `APP_MIGRATION_APPROVAL` fails with `DB_APPROVAL_REQUIRED` and
writes nothing. Rerunning the same plan reports `already_applied`. For the next
change, run `ntnt db new <name>` (it creates `migrations/0002_<name>.sql`), write
its SQL, and plan, approve and migrate again. Each new plan has a new digest.

### PostgreSQL

Provision a target database and a separate, disposable scratch database, each owned
by its own unprivileged login role. Run this as a PostgreSQL superuser (for
example `sudo -u postgres psql -f provision.sql` with a distribution package):

```sql
-- provision.sql (local development only)
CREATE ROLE app_owner LOGIN PASSWORD 'dev-only-owner';
CREATE DATABASE app_development OWNER app_owner TEMPLATE template0;
CREATE ROLE app_replay_owner LOGIN PASSWORD 'dev-only-replay';
CREATE DATABASE app_replay OWNER app_replay_owner TEMPLATE template0;
```

The roles log in over loopback (`127.0.0.1`) with these passwords. Logins follow the
server's `pg_hba.conf`: distribution packages usually require passwords on loopback
but authenticate Unix-socket logins with `peer`, which ignores passwords and admits
only an operating-system user with the role's name. To use a Unix-socket directory
as `host` instead, allow password authentication for these roles on local
connections first.

The managed schema must already exist on the target. Create it as the owner role,
for example `PGPASSWORD=dev-only-owner psql -h 127.0.0.1 -U app_owner -d app_development -c 'CREATE SCHEMA app'`.
Skip this if you use `schema = "public"`. Leave the scratch database empty.
Migrations may create further schemas with `CREATE SCHEMA`; NTNT inspects those
too (see [Managed scope](#managed-scope)). Each owner role above owns its database,
which gives it the `CREATE` privilege that needs, and it owns the schemas its
migrations create. Do not create those schemas in advance.

`ntnt db init --engine postgresql` writes this manifest (its comments are omitted here):

```toml
# ntnt.toml
[databases.app]
engine = "postgresql"
schema = "app"
migrations_dir = "migrations"
approval_secret = "APP_MIGRATION_APPROVAL"

[databases.app.targets.development]
connection_secret = "APP_DATABASE"
database = "app_development"
role = "app_owner"

[databases.app.scratch]
connection_secret = "APP_REPLAY_DATABASE"
database = "app_replay"
role = "app_replay_owner"
disposable = true

[secrets.APP_DATABASE]
[secrets.APP_REPLAY_DATABASE]
[secrets.APP_MIGRATION_APPROVAL]
```

```sql
-- migrations/0001_initial.sql
CREATE TABLE app.accounts (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    email TEXT NOT NULL UNIQUE
);
```

Connection secrets are libpq key/value strings (or `postgresql://` URLs) with exactly
one `host`, the declared `user` and `dbname`, an explicit **`sslmode`**, and no
`options`. This local quick start uses `sslmode=disable` over loopback. For anything
that crosses a network, use `sslmode=verify-full` (see [TLS](#tls)). Add `port=` if
the server is not on 5432.

```sh
export NTNT_ENV=development NTNT_SECRETS_PROVIDER=env
export APP_DATABASE="host=127.0.0.1 user=app_owner dbname=app_development password=dev-only-owner sslmode=disable"
export APP_REPLAY_DATABASE="host=127.0.0.1 user=app_replay_owner dbname=app_replay password=dev-only-replay sslmode=disable"

ntnt db status --target development
ntnt db plan --target development --out review.json
export APP_MIGRATION_APPROVAL="$(jq -r .plan_digest review.json)"
ntnt db migrate --target development --plan review.json
ntnt db verify --target development
```

A secret without an explicit `sslmode` (or with `sslmode=prefer`, a different user or
database, or several hosts) fails with `DB_CONNECTION_CONFIG`. An unset or undeclared secret fails with
`DB_SECRET`. A refused login fails with `DB_POSTGRES` and SQLSTATE `28P01` (wrong
password) or `28000` (for example `peer`): check `pg_hba.conf` and the secret.
Add `--json` to any command for the machine-readable report. If the
manifest declares only one database, `--database` is optional; `--target` is
required by every command except `init` and `new`.

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
- Select a named deployment target with `--target`; inspection and execution always require it, even for a single database or target. There is no fallback connection, URL argument, `url_env` alias, plaintext manifest credential, or `--force` bypass. Unknown fields in the selected database definition are errors.
- Connections reuse the existing **`std/secrets` providers and declaration validation**. Development's environment provider reads the exact logical name. Production uses the configured Unix-socket provider; the environment provider remains disabled in production. See [Secrets Agent Protocol](secrets-agent-protocol.md). Declarations are read from the captured manifest, not reopened during replay.
- A connection secret must specify exactly one host, the declared `user`/`dbname`, an explicit `sslmode`, and no startup `options`. [TLS](#tls) describes the transport rules; there is no silent downgrade to plaintext.
- Physical database, role and managed-schema names use ASCII letters/digits/underscores, start with a letter, and are at most 63 bytes. SQL object identifiers may be quoted. System schemas and `_ntnt` cannot be the managed schema; `public` is supported and is where most applications keep their tables.
- The **target managed schema must already exist**. Read-only commands never create it or `_ntnt`. Schemas the migrations create join the [managed scope](#managed-scope); the target role needs the `CREATE` privilege on the database for them (a database owner has it). Authorized `migrate` bootstraps its scoped ledger only inside the first migration transaction, after locked validation of an empty recognized base. A nonempty unrecognized scope is never assumed to have empty history.
- The declared target role is the logical owner for expected owner/grant canonicalization. Scratch uses its separate owner role. Other owners/grantees remain distinct; ownership and grants are never stripped wholesale. Multi-owner histories or additional grants that cannot be reproduced by the supported history will not receive a clean comparison. Target authority/membership facts are also bound into plans.

### TLS

`sslmode` in the PostgreSQL connection secret selects the transport for the target
and scratch sessions. It must be explicit:

- **`verify-full`** (use it for any network path): TLS, a certificate chain to a
  trusted CA, and a certificate valid for `host`. Trusted CAs are the host CA store
  by default (the store NTNT's other TLS clients use; `SSL_CERT_FILE` and
  `SSL_CERT_DIR` override it), or only the CAs in
  `sslrootcert=/absolute/path/ca.pem`. `sslrootcert=system` names the host store
  explicitly. Set `hostaddr=` to connect to an address while `host` stays the name
  the certificate must match.
- **`verify-ca`**: the same chain check without the host-name check, for a server
  reached by a name its certificate does not carry. It needs
  `sslrootcert=/absolute/path/ca.pem` holding the CA that issues your database's
  certificates, and never uses the host CA store: without the name check, any
  certificate a host-store CA issued to anyone, for any name, would pass. Like libpq,
  `ntnt db` rejects `verify-ca` without `sslrootcert` or with `sslrootcert=system`.
- **`require`**: TLS without verifying the server. It protects against passive
  observers only; anyone on the path can impersonate the server. Output and plans
  report it as unverified.
- **`disable`**: plaintext, only for a trusted local transport (loopback, a
  Unix-socket directory or a private container network).

```sh
export APP_DATABASE="host=db.example.com user=app_owner dbname=app_production sslmode=verify-full password=..."
export APP_DATABASE="host=10.0.4.12 user=app_owner dbname=app sslmode=verify-full sslrootcert=/etc/ntnt/db-ca.pem password=..."
```

A missing `sslmode`, `prefer`/`allow` (which can fall back to plaintext), `verify-ca`
without an `sslrootcert` file, `sslrootcert` with `require` or `disable`, and a
relative `sslrootcert` path fail with `DB_CONNECTION_CONFIG`. In key/value secrets a
backslash escapes the next character, so write Windows paths with forward slashes
(`sslrootcert=C:/certs/db-ca.pem`). TLS needs a TCP host: a Unix-socket host must use `disable`.
Client certificates (`sslcert`/`sslkey`), CRLs and `channel_binding=require` are not
supported. SCRAM authentication runs without channel binding, so the certificate
check is what authenticates the server.

Every TLS mode requires TLS. A server that does not offer it fails with `DB_TLS`,
and nothing is sent in plaintext. An untrusted, expired or wrongly named
certificate also fails with `DB_TLS` before any SQL runs. For a managed service whose
CA is not in the host store (Amazon RDS, for example), download the provider's CA
bundle and point `sslrootcert` at it; publicly trusted certificates work with the
default.

The transport is part of each session's identity: `sslmode`, the trust source
(`system`, or `file` with the file's SHA-256), the TLS version this client negotiated
(`tls_version`), and whether PostgreSQL reports its own end of the session as SSL
(`server_ssl`, from `pg_stat_ssl`). Behind a TLS-terminating proxy or pooler,
`server_ssl` is `false` even though the client's connection is verified TLS. Plans bind
the target's identity, so a plan reviewed over `verify-full` fails as stale if
`migrate` connects with `require` or `disable`, and a changed `sslrootcert` file needs
a new plan. Human output prints a `Transport:` line. Plans saved by ntnt 0.6.1 bind no
transport: `migrate` reports them as stale, but `reconcile` accepts one over
`sslmode=disable`, the only transport 0.6.1 used, so an attempt 0.6.1 left
unresolved can still be resolved after upgrading.

## Exact-plan authority and execution

`approval_secret` is optional for inspection but **required for every execution**, including additive changes and reruns. It names a separate top-level secret, never a connection secret. Its resolved value must be exactly the reviewed `plan_digest` (no whitespace trimming). The deployment/operator publishes that value through a separately protected existing `std/secrets` provider after reviewing the plan and migration SQL. The digest binds SQL/metadata, policy, logical store/target and physical identity; **knowing the digest does not authorize execution**. Obtaining its exact value through the protected provider does. There is no `--approve`, `--force`, self-authorizing manifest Boolean, signing server or new approval service.

For PostgreSQL, use separate application and migrator credentials, and restrict schema DDL/ledger ownership to the migrator. SQLite has no database roles: restrict file/directory access and approval delivery through the operating system and provider, as described below. Development fixtures may use the environment provider; protected deployments use the existing Unix-socket provider and separately controlled approval delivery. NTNT cannot protect against someone who already controls that provider or direct database-owner credentials. Never publish resolved connection strings or provider values. Backup/PITR, restore testing, old-client compatibility and maintenance authorization remain separately operated responsibilities; a committed destructive change is not made reversible by transactional rollback.

Each PostgreSQL invocation uses a dedicated physical connection and a session advisory lock on the database-local `_ntnt` namespace (two-key lock `1853124212, 245`). The advisory lock serializes migrators; a second migrator waits up to `migrator_wait_seconds` (5 seconds by default; see [Time limits](#time-limits)) and then fails with `DB_LOCK_CONFLICT`. The reserved namespace deliberately owns **one logical store/managed schema per physical database**; separate stores must use separate databases. This is not a multi-tenant ledger. The lock spans all per-migration commits.

For each PostgreSQL migration, the runner:

1. Begins a `READ COMMITTED READ WRITE` transaction with `lock_timeout = 1s`. It does not lock tables up front: each statement takes only the locks PostgreSQL requires for it.
2. Takes fresh identity/history/catalog observations and validates the exact ordered planned prefix, captured source/policy and mutation-safe ledger.
3. Locks the tables that the migration's `requires`/`ensures` checks read in `SHARE` mode (writers wait, readers do not), runs `requires`, executes the captured full SQL script within the statement limit (`statement_seconds`, 90 seconds by default), then runs `ensures`.
4. Verifies the resulting catalog against that exact prefix fingerprint, records the relation locks the transaction holds in `progress[].locks`, and inserts the success ledger entry in the same transaction.
5. Commits that migration before starting the next. Failure never rolls back an earlier committed migration.

If a lock is not granted within one second, or the session is chosen as a deadlock victim, the whole migration transaction rolls back and is retried after a backoff (250 ms doubling, with jitter), up to 4 attempts; `progress[].lock_retries` counts them. A short lock timeout keeps application queries from queueing behind a migration that is waiting for a lock. Statements that need strong locks (most `ALTER TABLE`, `DROP`, `CREATE INDEX` without `CONCURRENTLY`) still block other access to their table while the migration transaction is open; this is **not online migration tooling** for long rewrites. Advisory locks alone only coordinate cooperating migrators. No lock protocol promises to stop administrators or actors with direct DDL privileges. Incoming foreign keys from outside the [managed scope](#managed-scope) and all explicit cascading DDL are refused rather than claiming complete dependency/effect mediation.

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

`ntnt db init` always uses `migrations/` and refuses when another declared database
already uses that directory; declare such a database by hand, as above.
`ntnt db new <name> --database analytics` numbers files in that database's own
directory.

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

Capture is bounded to 1 MiB per input (including a saved review artifact), 16 MiB of migration files, and 1,024 migrations. For PostgreSQL, a script may have up to 262,144 tokens; each statement is parsed separately, up to 4,096 tokens and recursion depth 64. A larger statement is reported as `SQL_NOT_PARSED` but still replays. SQLite scripts are parsed whole, up to 4,096 tokens (including whitespace tokens) and recursion depth 64; oversized SQLite SQL is unknown, not automatically safe. Source files and manifest policy are captured once, then replay consumes those captured bytes even if their paths are subsequently replaced. A new command recaptures current inputs and invalidates an old binding if they changed.

## Disposable replay and its boundary

Provision scratch yourself using existing native test/container facilities. NTNT does not create a database/container, copy target rows, reuse target credentials, or reset an existing database.

PostgreSQL scratch must:

- have `disposable = true` and a separate secret and role;
- resolve to a different **cluster-system-identifier/database-OID pair**, not merely a different URL spelling;
- exactly match the target's server version number, encoding, locale/provider/version, timezone, DateStyle, IntervalStyle and client/string settings;
- be owned by a nonsuperuser role with no memberships, `CREATEDB`, `CREATEROLE` or replication. It may have `BYPASSRLS` only when the target role bypasses row-level security (directly or as a superuser), so seed rows in migrations see the same policies in scratch as on the target;
- contain only the empty stock public namespace and system objects, with no user routines/types/relations, event triggers, extra extensions or default grants.

Identity discovery uses `pg_control_system()` and catalog metadata. Missing permission to establish identity fails closed. Server administrators and the PostgreSQL installation are trusted; this is not protection against a hostile superuser modifying PostgreSQL's own catalogs or concurrent administrators changing scratch after validation. Use an exclusively disposable fixture, preferably network-isolated. Restored/cloned clusters sharing identity can be conservatively rejected.

Each migration runs with normal row-level security and ends as its own commit would: deferred constraints are checked and attributed to that migration, then `INITIALLY DEFERRED` constraints are restored for the next one. Inspection and checks still read with `row_security=off`.

Replay owns a dedicated physical connection and a **rollback-only outer transaction**, not a pool handle. It creates the managed scratch schema only after isolation/emptiness checks, replays captured scripts through the native execution primitive, captures the applied-prefix/candidate catalogs, and explicitly rolls back. Success and failure tests verify no scratch schema survives. The authorized per-migration executor reuses that exact captured-SQL primitive. Catalog derivation deliberately skips transition contracts: an empty replay cannot establish data-dependent production assertions. Fresh/populated native rehearsal tests invoke the same `migrate` executor on disposable fixtures with representative rows; they are not a second application-level runner or a claim about production data.

### What PostgreSQL replay runs

PostgreSQL migrations are complete SQL scripts. Scratch replay and authorized execution both send the captured script, unchanged, inside one transaction per migration. That includes `DO` blocks, functions, triggers, policies, grants, seed `INSERT`s, `SET CONSTRAINTS` and other transactional SQL a real application history contains. PostgreSQL itself determines the resulting catalog.

Before anything runs, the script is tokenized (quoted and dollar-quoted text stays intact) and each top-level statement is checked. These statements are rejected with `DB_REPLAY_UNSUPPORTED`, because they would end or split the migration transaction, change who the session acts as, or act outside the database:

- transaction control: `BEGIN`, `START TRANSACTION`, `COMMIT`, `END`, `ROLLBACK`, `ABORT`, `SAVEPOINT`, `RELEASE`, `PREPARE TRANSACTION`;
- session identity: `SET [LOCAL|SESSION] ROLE`, `SET SESSION AUTHORIZATION`, `SET session_replication_role`, `RESET ROLE|SESSION|ALL`, `DISCARD`;
- non-transactional work: `VACUUM`, `CONCURRENTLY` operations (except the single-statement index migration described below), `REINDEX DATABASE|SYSTEM`;
- server scope: `COPY`, `LOAD`, `CHECKPOINT`, `ALTER SYSTEM`, and creating/altering/dropping databases, tablespaces and roles;
- explicit `CASCADE` drops, whose transitive effects are not bounded. `ON DELETE CASCADE` and `ON UPDATE CASCADE` foreign-key actions are allowed.

Text inside `DO` and function bodies is not inspected; it runs inside the same transaction with the migration role's privileges. Keep role creation out of migrations and provision roles separately. Database-wide custom implicit/assignment casts still block execution, because ordinary DDL coercion could run them without appearing in the reviewed script.

Each pending migration's `analysis` lists its statements' `operations`, known lock/effect `hazards`, and `unknowns`. `SQL_NOT_PARSED` means a statement (for example a `DO` block) was not parsed. `EFFECTS_NOT_ANALYZED` means it parsed but falls outside the subset NTNT analyzes in detail: ordinary tables with built-in types and literal defaults, simple constraints, column-only btree indexes, and non-cascading table/index drops. Unknowns are review prompts, not blockers: the replayed catalog and drift comparison still apply. **Scratch replay is not a sandbox.** Use a disposable database with a separate unprivileged owner, as required above.

For PostgreSQL, `analysis.locks` predicts the table locks each statement takes (`statement` is its 1-based position in the script, and `through` ends a run of consecutive statements that take exactly the same locks, such as many seed `INSERT`s; `mode` uses the same `pg_locks` names as `progress[].locks` after `migrate`). Predictions come from the SQL and PostgreSQL's documented lock levels, not from replay: replay holds every lock until its single transaction ends and runs on empty tables, so it cannot show which migration took a lock or how long it would block. Statements NTNT cannot parse, `DO` blocks, `DROP INDEX` and queries (which can lock rows or call functions) add `LOCKS_NOT_PREDICTED` to `unknowns`. Tables created earlier in the same migration are omitted, except after `CREATE TABLE IF NOT EXISTS`, which may find an existing table.

`analysis.lock_warnings` flags locks that block or scan an existing table:

- `BLOCKS_WRITES_WHILE_INDEXING`: `CREATE INDEX` without `CONCURRENTLY`.
- `SCANS_TABLE_UNDER_LOCK`: `SET NOT NULL`, a `CHECK` or foreign key added without `NOT VALID`, or a primary key/unique constraint built without an existing index.
- `REWRITES_TABLE`: a column type change.
- `MAY_REWRITE_TABLE`: a new column with a volatile default, identity or stored generated value.

Warnings are review prompts and do not block a plan. The safer forms are `ADD CONSTRAINT ... NOT VALID` followed by `VALIDATE CONSTRAINT` in a later migration, and a `CHECK (column IS NOT NULL) NOT VALID` validated before `SET NOT NULL`.

### Concurrent index builds

A migration whose entire script is one named `CREATE [UNIQUE] INDEX CONCURRENTLY` on a managed table runs outside a transaction (`analysis.non_transactional`). It takes only `SHARE UPDATE EXCLUSIVE` on the table, so reads and writes continue while the index builds. The build waits for transactions already running on the table rather than failing on the 1-second lock timeout. The statement limit (`statement_seconds`, 90 seconds by default) covers the whole build, waits and table scan included; a build that exceeds it fails and its invalid index is dropped. For a large table, raise the limit in [`[databases.<name>.limits]`](#time-limits), then plan and approve again. Comments around the statement are allowed; any other statement, an unnamed index or a table outside the managed schema is rejected as before. Such a migration cannot have `requires` or `ensures` checks, because no transaction surrounds the change. Custom implicit/assignment casts block the build as they block other DDL, and the build uses replay's storage settings (`default_tablespace=''`).

Replay builds the same index without `CONCURRENTLY` inside its rollback-only transaction, so the plan's expected catalog is identical. On the target, `migrate`:

1. validates the planned prefix, then inserts a ledger row in state `running` for the migration and **commits it before the build starts**;
2. builds the index outside any transaction, still holding the migrator advisory lock;
3. in a short transaction, checks that history still ends in that attempt and that the catalog is exactly the plan's state after the migration, then marks the row `applied`.

If the build fails (for example, duplicate values for a unique index), `migrate` drops the invalid index with `DROP INDEX CONCURRENTLY`, deletes the `running` row and reports the migration `failed` with `cleanup: dropped_invalid_index`. Fix the data and rerun the same plan.

If `migrate` stops between steps 1 and 3 (killed, disconnected, timed out), the `running` row stays. `status`, `verify`, `plan` and `migrate` then fail with `DB_ATTEMPT_UNRESOLVED` and run nothing until it is resolved with the same approved plan:

```sh
ntnt db reconcile --database app --target production --plan review.json --json
```

`reconcile` takes the migrator advisory lock, so it waits for (and then refuses to run alongside) a `migrate` that still holds it. It never runs migration SQL. It reports the index it observed (`reconciliation.observed_index`) and one resolution:

- `completed`: the index exists, is valid, and the catalog is exactly the plan's state after the migration. The row becomes `applied`.
- `rolled_back`: the index is invalid, and, treated as finished, it would make the catalog exactly the planned state. It is dropped, the catalog is rechecked against the state before the migration, and the row is deleted.
- `not_started`: no index of that name exists and the catalog is exactly the state before the migration. The row is deleted.
- `none`: there was no attempt to resolve.

Anything else fails with `DB_RECONCILE_MISMATCH` (for example, an index of the planned name with another definition) and changes nothing. `DB_ATTEMPT_ACTIVE` means a session is still building that index; wait for it or cancel that backend, then reconcile again. After `rolled_back` or `not_started`, rerun `migrate` with the same plan.

An index of the planned name that exists without a `running` row is not treated as progress: `migrate` reports a stale plan, because nothing records that this migration built it.

## Inspection coverage and drift

The PostgreSQL adapter declares capabilities **per operation and observed engine version** for PostgreSQL 16–18. SQLite declares its own coverage below; other engines/versions are rejected. It reads only fixed catalog queries and a structurally validated ledger, in a dedicated `REPEATABLE READ READ ONLY` target transaction. It never prepares/executes application queries, invokes a user's check/routine, reads live application rows, or initializes target state.

Supported catalog fingerprints include:

- schema/table owners, schema/relation/column ACLs with sorted grant entries;
- ordinary heap tables, persistence, storage options and tablespaces;
- column order, precise type/typmod, array dimensions, collation/version, nullability, default/generated-expression identity, identity/generated flags, storage/compression;
- key/check/foreign-key definitions, validation/enforcement and deferrability state, including the enabled state of internal constraint triggers (without unstable generated trigger names/OIDs);
- index definitions **and valid/ready/live flags**, uniqueness, primary/exclusion/immediate state, null-distinctness, clustering and replica identity.

Quoted identifier components are unambiguously quoted in composite object keys. OIDs and ACL array order do not become semantic differences. The declared owner role maps to a typed managed-role marker in target/scratch, while every other role and the PUBLIC grantee remain distinct. Sequence counters are never read or fingerprinted.

Plain views, sequences, SQL and PL/pgSQL functions/procedures (full definition, owner and grants), user triggers, RLS policies, declarative partitions in their parent's schema, expression/partial indexes on built-in access methods, and extensions installed in the managed scope (by version) are compared like tables: an out-of-band change is reported as drift on the named object. Materialized views, functions in other languages, non-partition inheritance or partitions in another schema than their parent, foreign/custom types, custom collations/operator classes/access methods, extensions in other schemas, event triggers, security labels, other namespace objects and references to routines outside `pg_catalog` and the managed scope expose incomplete coverage. Unreadable objects or unknown ledger shape also block a clean result. Comparison is by definition, not behavior: NTNT does not analyze what a function body or policy expression does. This is not dependency closure, security-policy verification or replication/publication configuration auditing.

Engine-deparsed SQL, defaults, expressions, comments and storage option payloads are represented by **definition hashes**, not printed literals: schema definitions can themselves contain credentials. Object names, structural fields, owner/grant facts and object-level differences remain visible. This avoids echoing connection credentials, SQL error payloads or live row samples. It is not an SQL dump; review source SQL separately through appropriately authorized tooling.

Drift compares `observed` with `expected(applied_prefix)`, not repository HEAD. Differences name the affected object and show expected/observed properties or definition fingerprints. An incomplete comparison is `unknown`, never “no drift.” Unsupported pending SQL can block a candidate plan while a separately verified applied prefix still passes. `verify` does not replay pending SQL.

`DB_COVERAGE_INCOMPLETE` names what could not be inspected: its `objects` field lists the uncovered catalog keys, for example `index:"contact_requests_pending"` or `table:"t"`. When the objects are on the target, `catalog.uncovered` lists them too. When replaying a migration produced them, they exist only in the replayed intermediate catalog, and `source` names that migration. The human output prints both fields under the diagnostic.

### Managed scope

PostgreSQL inspects the **managed scope**: the configured `schema` plus every
schema the captured migration history creates. A schema is created by the history
when a top-level `CREATE SCHEMA <name>` statement in any captured migration names
it (pending migrations included), or when an `ALTER SCHEMA <old> RENAME TO <new>`
renames such a schema. The set comes from the captured SQL, before any database is
read, so `status`, `plan`, `verify`, `migrate` and `reconcile` inspect the same
schemas, and the plan's input digest binds them. Reports list them in
`target.history_schemas`. A `CREATE SCHEMA` inside a `DO` block or function body,
`CREATE SCHEMA AUTHORIZATION <role>` without a schema name, and the reserved names
`pg_*`, `information_schema` and `_ntnt` never join the scope.

```sql
-- migrations/0001_initial.sql
CREATE TABLE app.accounts (id int PRIMARY KEY);
CREATE SCHEMA audit;
CREATE TABLE audit.log (id int PRIMARY KEY, account int REFERENCES app.accounts (id));
```

Objects in `audit` get the same inspection and coverage rules as the managed
schema's: an out-of-band `DROP TABLE audit.log` or `GRANT SELECT ON audit.log TO
PUBLIC` is drift on `audit.relation:log`, and an unsupported object there, such as
a materialized view, makes the result `unknown`. Catalog keys in those schemas
start with the schema name (quoted where PostgreSQL would quote it):
`audit.relation:log`, `audit.column:log.account`, `audit.index:log_pkey`. A schema
itself is `schema:audit`. Keys in the managed schema stay unqualified
(`relation:accounts`), so a history without `CREATE SCHEMA` keeps exactly the
catalog, plan digests and saved plans it had before the scope was added. A plan
saved by an earlier release for a history with `CREATE SCHEMA` did not inspect those
schemas; `migrate` reports it as `DB_PLAN_STALE`, so plan it again.

Foreign keys, views, triggers, column defaults and policies may refer to any schema
of the scope. A later migration may rename or drop (`DROP SCHEMA audit`, without
`CASCADE`) a schema the history created; replay produces the expected state and a
schema that is recreated out of band is drift. A schema the history will create
belongs to the scope from the start: if the target already has a schema of that
name, its contents are compared with the replayed history (an unexpected schema is
drift; with no ledger, objects there need [adoption](#explicit-adoption-of-existing-databases)).
Schemas the history creates are owned by the migration role on the target and by
the scratch role in replay; both map to the managed-role marker. `requires`,
`ensures` and `adoption_checks` still read only managed-schema tables. Lock
predictions, `progress[].locks` and `CREATE INDEX CONCURRENTLY` cover managed-schema
tables only.

Anything a migration would create, change or drop outside the managed scope would
never be inspected, so it is refused with `DB_OUTSIDE_SCOPE`. Its `source` names
the migration and `objects` names what lies outside the scope:

- **Before the script runs**, in replay and on the target: a top-level statement
  that creates, alters, drops, truncates or grants on an object named with a schema
  outside the scope (`CREATE TABLE public.audit_log`, `DROP TABLE IF EXISTS
  public.legacy`, `CREATE INDEX ... ON shared.t`), or that creates, alters, drops
  or grants on such a schema itself (`DROP SCHEMA IF EXISTS shared`, `GRANT USAGE
  ON SCHEMA public`, `GRANT ... ON ALL TABLES IN SCHEMA public`, `CREATE EXTENSION
  ... WITH SCHEMA public`). The message gives the statement number; `objects` lists
  the names as written (`public.legacy`). `pg_temp` is not outside the scope. These
  statements can be no-ops in empty scratch or fail there for lack of the schema,
  so they are refused by name before replay.
- **After each replayed migration**: scratch must hold nothing outside the scope
  that it was not provisioned with: no other schema, no object in `public` (unless
  `public` is the managed schema or the history creates it), and `public`'s owner
  and privileges unchanged. `objects` lists catalog-style keys such as
  `public.relation:audit_log` or `schema:reporting`. This catches what statement
  analysis cannot see: `DO` blocks and function bodies, `SET search_path`,
  `ALTER ... SET SCHEMA`, and extensions that install objects in `public`.

Keep shared objects, such as extensions installed in `public` for several
applications, out of migrations and provision them separately, or install an
extension in the managed schema. A statement inside a `DO` block or function body
that acts outside the scope only on the target (for example dropping a `public`
table if it exists) is not detected; keep such SQL out of migrations. Database-wide
objects without a schema, such as publications and large objects, remain outside
inspection.

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
ntnt db verify --database app --target development --against adoption.json --json
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
`status`, `verify` or `plan --against adoption.json` revalidate that saved action's
requested baseline read-only, without running its data checks or applying history.
An unchanged pre-adoption artifact remains valid even though standalone inspection
still reports unrecognized history. After adoption changes the base ledger, use
ordinary `verify`; `--against` compares the saved pre-action base, not a deployment
receipt or permission to execute.

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
ledger evidence, not assertions executed in this invocation. Fresh reconciliation
reports use the same separate counts; adopted/imported coverage is never relabeled
as applied migrations after a failed or unacknowledged upgrade. Normal execution
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
requires its own authorization. The only supported non-transactional DDL is a lone
PostgreSQL `CREATE INDEX CONCURRENTLY` (see [Concurrent index builds](#concurrent-index-builds));
other non-transactional DDL and schema-aware query analysis remain later work.

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

Support is for the bundled **SQLite 3.54 series**, not an arbitrary separately installed `sqlite3` executable. `status`/`plan`/`verify` open the target readonly and use a deferred snapshot, releasing it before scratch replay. Catalog inspection reads metadata and compiles, but never steps, application programs with a native authorizer to validate their supported effect paths. It does not scan application rows. Scratch uses a distinct physical file, separate secret and matching settings; `BEGIN IMMEDIATE` owns its rollback-only replay. An empty scratch takes the target's journal mode, because `std/db/sqlite` `connect()` puts application databases in WAL mode and a scratch made with `touch` is not. Any other setting that differs (encoding, page size, auto-vacuum) fails with `DB_SCRATCH_MISMATCH`, which names each setting with both values, for example `page_size: target 4096, scratch 1024`. A nonempty scratch file is rejected, never cleared.

Execution opens an existing file only after approval and file-identity checks. It acquires **`BEGIN IMMEDIATE` before inspecting history/catalog for each migration**, revalidating on every acquisition; the writer reservation is not retained between commits. Competing migrators recognize each other's matching committed prefixes instead of applying SQL twice. SQLite's writer lock covers ordinary application writers; it is not an online-DDL guarantee. Dedicated connections enable foreign keys, recursive triggers and CHECK enforcement, `trusted_schema=OFF`, defensive mode, strict quoting, and `synchronous=FULL`. Durable rollback-journal modes and WAL are accepted; journal-off/memory modes are not. The runner does not change the selected persistent journal mode.

The two-second busy timeout, five-second worker/progress deadline, and shared orchestration/reconciliation deadlines bound operations. A migration script on the target runs under `statement_seconds` instead (90 seconds by default, see [time limits](#time-limits)); scratch replay keeps the five-second bound. SQLite interrupts an operation at its deadline, and a statement interrupted there fails with `DB_STATEMENT_LIMIT`, naming the statement and the limit. An operation that has not returned five seconds after its deadline is cancelled: its connection is discarded and never returned to a pool. Native I/O still depends on the OS/filesystem. A busy or interrupted COMMIT is reconciled using a fresh connection and writer reservation, without retrying SQL; if the fresh observation cannot establish the prefix, the result stays unknown. Process termination cannot return acknowledgement: rerun the same approved plan to inspect committed truth first. A hot rollback journal may prevent readonly inspection (`DB_SQLITE_RECOVERY_REQUIRED`); wait for an active writer to finish, or rerun `migrate` with that same approved plan so native recovery and locked reconciliation occur through the authorized path. Do not delete the journal or silently make inspection writable.

SQL, named transition checks, the resulting catalog and the success ledger row are one transaction. SQLite contracts use the same closed SELECT expression grammar described above, with every relation qualified as `main.<table>`; the native authorizer additionally denies side-effecting calls and nonmanaged relations. Only **one integer `1`**, in one row/column, passes. Numeric values other than `1`, floating-point `1.0`, text, NULL, incorrect shape, errors and timeouts fail. Historical checks remain `not_run` on verify/rerun.

### Supported SQL and catalog coverage

The SQLite dialect parser accepts ordinary table/index/view/trigger DDL and INSERT/UPDATE/DELETE, while the native authorizer mediates resolved reads, writes and calls. Native SQLite statement boundaries preserve trigger bodies and quoted semicolons; scripts are not split with string operations. Tables, columns/order/type/defaults, PK/UNIQUE/CHECK/FK definitions, indexes (column, expression and partial, unique or not) and builtin collations (`BINARY`, `NOCASE`, `RTRIM`) are fingerprinted. WITHOUT ROWID and STRICT flags are retained. Supported views and table triggers have definition hashes and native compile checks, **not inferred business semantics or complete transitive-effect explanations**. SQL definitions/defaults remain redacted hashes; final catalogs are engine-native, not normalized into PostgreSQL definitions.

An index's expressions and partial-index `WHERE` predicate are part of its definition hash, so changing one out of band, for example dropping and recreating the index with another predicate, is drift on that index. SQLite resolves these expressions, like CHECK constraints, when it loads the schema, without consulting the authorizer, so inspection cannot re-check the functions they call. The function allowlist below applies when a migration's `CREATE INDEX` is compiled, in scratch replay and on the target; inspection then requires the target's index to match the replayed definition. SQLite itself allows only deterministic functions of the indexed table's own columns there, with no subqueries. Inspection still compiles the table's INSERT/UPDATE/DELETE programs, which evaluate them.

Only concrete managed relations are eligible. `CREATE TABLE ... AS SELECT` is rejected: native authorization of a pending table name must not authorize a same-named virtual-table source. Express copies as ordinary `CREATE TABLE` followed by `INSERT ... SELECT`, with resolved relations refreshed between native statements. Eponymous virtual/table functions such as `json_each` or `pragma_*` cannot masquerade as ordinary `main` tables. Builtin `count` is available to checks; migration/definition programs additionally allow the native rename helpers and the small builtin set `like`, `substr`, `length`, `printf`, `quote`, `coalesce`, `ifnull`, `lower`, `upper`, `trim`, `abs`, `typeof`, plus the date and time functions `date`, `time`, `datetime`, `julianday`, `unixepoch`, `strftime`, `timediff`, `CURRENT_DATE`, `CURRENT_TIME` and `CURRENT_TIMESTAMP`, so defaults such as `DEFAULT CURRENT_TIMESTAMP` and `DEFAULT (datetime('now'))` work. SQLite itself still refuses non-deterministic calls where determinism is required (CHECK constraints, generated columns, index expressions and predicates). SQLite checks a call when it compiles the statement that makes it, including the body of a trigger that statement fires or a view it reads. A migration statement that calls anything else that way fails with `DB_SQL_UNSUPPORTED`, naming the statement number and what SQLite refused, such as a function (``Statement 3 uses SQLite function `hex` ``) or a table. A trigger or view body that no statement in the migration uses is compiled only during inspection, so a call in it is reported as `DB_COVERAGE_INCOMPLETE`, with the trigger's table or the view in `objects`. Authored PRAGMAs are refused before replay with `DB_REPLAY_UNSUPPORTED`. Any other failing statement is numbered too. A known function name is not permission to load extensions or register application functions.

Virtual/shadow tables, generated columns, AUTOINCREMENT/`sqlite_sequence`, custom collations/functions, unsafe/unsupported view or trigger programs, statistics/internal objects, bind parameters, ATTACH/DETACH, authored PRAGMAs, transaction/savepoint/session control and extension loading are unsupported. Unsupported catalog coverage blocks a clean result. DML and trigger effects remain review-required (`SQLITE_EFFECTS_NOT_INFERRED`); metadata never turns an unknown effect into automatic safety. Large transformations belong in separately operated application jobs.

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
- preserves every existing named index (partial and expression indexes included, predicate and expressions unchanged), view and trigger definition and every other table's definition;
- restores foreign-key enforcement, including on failure; failure rolls back or discards the session.

Explicitly recreate the selected table's named indexes/triggers and any views dropped to allow the rebuild. The selected table's columns/constraints and autoindexes may change. Definition comparisons are conservative: equivalent SQL written differently may still be rejected. Deliberate dependent-object retirement belongs in a separate reviewed migration, not an accidental omission inside this mode. The mode scans row counts **during authorized execution**, not during production planning.

Without this sidecar, a migration that drops a table which other tables reference with `ON DELETE CASCADE`, `SET NULL` or `SET DEFAULT` is refused with `DB_REBUILD_REQUIRED` before the drop runs, naming each child table and its action. SQLite deletes every row of a dropped table first. On a target with data, the drop would run those actions, and scratch replay, which starts empty, could not show it. A drop that runs no such action stays allowed: one whose children were dropped first, one whose child foreign keys have no `ON DELETE` action, and a table's reference to itself.

Counts and FK checks do not prove row identities, correct value mappings, every uniqueness assumption, old-client compatibility or recovery after a committed destructive change. Use named transition checks and the same executor in representative native upgrade tests to assert exact keys/values, preserved children and working dependencies. `tests/db_sqlite.rs` demonstrates an email-lowercasing rebuild with existing rows, indexes, views, audit trigger and cascading foreign keys, plus negative dependency/row-loss/orphan cases. Empty replay alone cannot provide that evidence. Unknown business effects remain review-required even when these guards pass.

## Results and plan identity

JSON has `format_version: 1`, UTC `observed_at`, stable diagnostic codes, source filenames/object keys and next actions. `pass`, `fail`, `unknown`, and `not_run` are distinct. Exit code 0 means the requested inspection/replay result passed within its declared coverage; other outcomes return nonzero. `status`, `plan` and `verify` report **`not_run`, zero executed** assertions; `migrate` separately counts the contracts actually attempted in that invocation. A catalog/history pass is not evidence of SQL contracts, data correctness, business invariants or deployed-client compatibility.

Plans bind:

- logical database and target names, captured manifest, ordered source/checksum identities and policy;
- native cluster/database or file identity, engine and settings, and adapter-specific role/authority facts;
- managed scope, observed base history/catalog, applied prefix and replayed expected base;
- adapter/canonicalization version, exact pending migration identities and candidate fingerprint;
- explicit `execution.format = "postgresql-transactional-v1"` or `"sqlite-transactional-v1"` and ordered engine-specific catalog fingerprints for every migration prefix, including the empty prefix. Old/incomplete artifacts cannot authorize execution;
- for PostgreSQL, the [time limits](#time-limits) `migrate` will use (`execution.limits`).

`plan_digest` hashes that descriptor. Observation time and display filters are not authority inputs. `--against` checks the saved descriptor's digest, pending descriptors and fresh source/target/base binding. It neither executes nor approves the plan. For `plan --out`, the file contains the compact plan descriptor and result metadata, not duplicate catalog/source display fields; stdout still contains the full report. `status`/`verify --out` save their report. All saved artifacts share the reader's 1 MiB bound: oversized output fails before creating a file rather than producing an unreadable plan. Files are new-only (`--out` cannot replace a source, existing plan or symlink); on Unix, new reports are mode 0600.

Every saved plan has `authorized = false`, `executable = false`, `automatic_safe = false` (the artifact alone grants no capability), and a `requires_review` or `blocked` classification. Known drop/constraint-removal/cast/CASCADE/DML/authorization hazards are reported conservatively. Lock/scan labels are qualitative, not production duration estimates. Missing data checks and still-deployed application compatibility remain explicit. `migrate` validates the execution format and all prefix evidence under its own lock and requires independent provider authority; the digest alone is not an approval token.

## Bounds, cleanup and concurrency

Connect/authentication and every query/cleanup exchange have 5-second client deadlines; PostgreSQL statement and lock limits are 5 and 2 seconds, with a 10-second idle transaction timeout. Migration scripts, concurrent index statements and the wait for another migrator have their own [time limits](#time-limits). Database orchestration has a 120-second outer deadline by default; migration work reserves the final 20 seconds (more if `migrator_wait_seconds` is raised) for fresh reconciliation if a work deadline interrupts COMMIT. If reconciliation itself cannot finish, progress remains unknown rather than guessing. Existing secrets providers retain their own bounded lookup/retry policy. Dedicated connection tasks are discarded on cancellation/error; sessions are never recycled into application pools. Runtime shutdown waits at most 100 milliseconds for uncancellable OS-resolver work, so cancelling a connect is not followed by an unbounded runtime join. Scratch rollback is checked, and cleanup failure cannot produce a verified candidate.

Inspection transactions are observations, not deployment locks. ACCESS SHARE protects the ledger relation while inspecting it; the target transaction is rolled back and its dedicated session discarded immediately after capturing identity, catalog and history, before scratch replay starts. Long replay cannot retain the target lock or hit its idle-transaction deadline. A repeatable-read snapshot does not exclude arbitrary concurrent administrator DDL, application writes, changes after observation or physical cluster cloning. Execution separately revalidates under migration ownership and native table locks. Empty replay does not prove an upgrade preserves existing rows, that a narrowing cast will succeed, or that a lock will be brief.

### Time limits

A database can set execution limits, in whole seconds. PostgreSQL takes both; SQLite takes `statement_seconds`:

```toml
[databases.app.limits]
statement_seconds = 3600     # one migration script, or one concurrent index build
migrator_wait_seconds = 60   # wait for a competing migrator's advisory lock
```

| Field | Default | Range | Limits |
|---|---|---|---|
| `statement_seconds` | 90 | 2–86400 | One migration script on the target (PostgreSQL applies it to each statement; NTNT stops waiting for the whole script 5 seconds later; SQLite bounds the whole script, its foreign-key check included), its deferred constraint checks, and each `CREATE INDEX CONCURRENTLY` build or `DROP INDEX CONCURRENTLY` cleanup, waits and table scan included. |
| `migrator_wait_seconds` | 5 | 1–300 | PostgreSQL only: how long `migrate` and `reconcile` wait for another migrator's advisory lock before failing with `DB_LOCK_CONFLICT`. |

Omitted fields keep their defaults; without the table nothing changes. Unknown fields, non-integers and values outside the range fail with `DB_CONFIG`, naming the field. The 2-second minimum for `statement_seconds` keeps it above the 1-second migration `lock_timeout`. PostgreSQL counts the statement limit from the start of the statement, so at 1 second a statement waiting for a brief lock would reach the statement limit first and fail without a [retry](#exact-plan-authority-and-execution). The upper bounds are deliberate: a statement that needs more than a day belongs in a supervised manual build, and a migrator that has held the lock for five minutes is usually stuck or running a long build, so a deployment should fail and retry rather than queue behind it. On SQLite the table takes only `statement_seconds`; `migrator_wait_seconds` fails with `DB_CONFIG`. Scratch replay and SQLite's other operations keep the fixed bounds described under [SQLite](#transactions-replay-and-contracts). A statement past its limit fails with `DB_STATEMENT_LIMIT`, naming the statement and the limit.

`migrate` and `reconcile` size their deadlines so that one full migrator wait and one full statement fit:

- the work deadline is `statement_seconds + migrator_wait_seconds + 5` seconds;
- the command deadline adds `migrator_wait_seconds + 15` seconds, so fresh reconciliation after an interrupted COMMIT can still wait for the migrator lock.

A limit set below its default counts at its default here, so lowering a limit never shortens these deadlines below 100 and 120 seconds. A run that reaches its work deadline keeps its committed progress; rerun the same plan to continue (after `ntnt db reconcile` if it interrupted a concurrent index build). The 5-second catalog exchanges, the 1-second migration `lock_timeout` and its retries are not configurable.

`status`, `plan` and `verify` keep the default limit and the 120-second deadline: they replay on an empty scratch database, where index builds and table rewrites are fast. A script that is slow even on empty tables (for example, one that generates rows) must still replay within 90 seconds.

Each PostgreSQL plan records the limits it was made with in `plan.execution.limits`; a SQLite plan records `statement_seconds` there only when the manifest declares the table, and `migrate` and `reconcile` report the limits and derived deadlines they used in `limits`. Limits are reviewed policy: changing them changes the policy digest, and `migrate` rejects an earlier plan with `DB_PLAN_STALE`, so plan and approve again. A plan saved before NTNT recorded limits has no `execution.limits`; it implies the defaults and stays valid while the manifest declares no limits table.

Limits apply to every migration of the logical database; there is no per-migration override. To give one large index build more time, raise the limit for the release that contains it and lower it again in a later plan.

## Native regression tests

Ordinary `cargo nextest run --locked` runs portable inventory, parser, redaction, timeout and CLI tests, the real-file SQLite lifecycle/rebuild/failure tests, and SQLite shared conformance. These are not ignored and run on Linux, macOS and Windows CI. Real PostgreSQL tests are explicitly ignored unless selected, and **mandatory CI lanes execute them on PostgreSQL 16, 17 and 18**, with fixture-only control databases and separate randomly named target/scratch roles/databases for each test.

For a disposable local PostgreSQL instance (never production), set `NTNT_DB_TEST_ADMIN` to its administrative connection secret and `NTNT_DB_TEST_HOST` to its host or Unix-socket directory, then run:

```sh
cargo test --locked --test db_catalog_postgres -- --ignored --test-threads=2 --nocapture
cargo test --locked --test db_execution_postgres -- --ignored --test-threads=2 --nocapture
cargo test --locked --test db_migration_conformance -- --include-ignored --test-threads=2 --nocapture
cargo test --locked --lib migrations::postgres::tests::target_transaction_is_really_read_only_even_for_superuser -- --ignored --exact --nocapture
```

The shared conformance suite executes the same history/drift, approval/stale/wrong-target, contract failure and committed-prefix recovery scenarios on both native adapters. It also exercises populated baseline adoption, exact multi-file coverage, required check failures, unknown ledgers and later upgrades. PostgreSQL cases import only recognized immutable legacy history and reject unsafe ledger definitions. Adoption commit-fault tests distinguish baseline/import records from SQL execution; SQLite cases cover competing adopters, cancellation/timeout and busy-COMMIT recovery. SQLite-only tests also cover physical file aliases/replacement, read-only nonmutation, WAL, bounded busy waits, competing CLI processes, forced process death before commit and after an acknowledged-to-SQLite prefix, busy-COMMIT fresh reconciliation, timeout/discard, ledger tampering, redaction and populated rebuild preservation. `tests/db_sqlite_indexes.rs` covers partial (including partial unique) and expression indexes through plan, migrate and verify, out-of-band predicate and expression drift, a rebuild that must recreate a partial index unchanged, the function allowlist for predicates and expressions, and coverage diagnostics that name the uncovered objects. Native SQLite fixtures provision disposable local files only; no service is needed.

PostgreSQL fixtures create/drop databases and roles. They test schemas the history creates (drift, coverage and grants there, cross-schema foreign keys, renames, `DROP SCHEMA`, adoption and reconciliation), `DB_OUTSIDE_SCOPE` refusals, single-schema fingerprints recorded before the scope was added, real read-only transactions, no target bootstrap, applied-prefix versus pending drift, invalid indexes, constraint validity/grants, immutable SQL/metadata history, malformed/RLS/view ledgers, unknown coverage, physical scratch collisions, low-privilege/empty/matching scratch, redaction, bounded waits, cleanup, source replacement during a blocked inspection, and target/source/authority-bound plans. Execution tests additionally cover fresh construction, populated upgrade preservation, exact authority, stale plans, partial commits and reruns, false/NULL/shape/type/error/timeout checks, native/advisory contention, configured statement and migrator-wait limits (scaled to seconds), snapshots after lock acquisition, process cancellation, unsafe/scoped ledgers, unsupported effects and source replacement. A test-only wire proxy drops connections before COMMIT, after durable COMMIT but before acknowledgement, during rollback, and during reconciliation. It is not linked into release code and exposes no authorization bypass. These executed fixture checks are not evidence about arbitrary application business behavior or real deployment compatibility.
