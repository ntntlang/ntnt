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
file after the highest existing ID, zero-padded to the width of the widest existing
ID, or to four digits in an empty directory: a history numbered `001_` to `034_`
gets `035_`, which `ls` and shell globs list after it. `ntnt db` itself orders
migrations by numeric ID, not by file name. Migration names use lowercase ASCII
letters, digits and underscores, and `new` never overwrites a file. Writing the
same files by hand works just as well.

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
`status` and `plan` reject that history with `DB_DUPLICATE_ID`, naming both files;
renumbering the migration that has not been applied anywhere, with its `.toml`
sidecar renamed to the same new stem, fixes it. A merged
migration numbered below one a target already applied is refused for that target
with `DB_HISTORY_DIVERGENT`, which names it and the ID to renumber it above. An
applied migration renamed with its ID kept is refused the same way, but named by
the name it was applied under, with advice to rename it back, with its `.toml`
sidecar when it has one; an applied migration is never renumbered.

`new` never changes `ntnt.toml`, but like `init` it opens the file for writing to
lock it, because NFS grants the lock only through such a handle. A `ntnt.toml` that
`new` may not write is locked through a read-only handle instead: that works on
local filesystems, and on NFS `new` fails with `DB_SCAFFOLD_IO`. Both commands fail
that way on a filesystem without file locks. On Windows, file locks are mandatory,
so while `init` or `new` holds the lock no other program can read `ntnt.toml`: an
`ntnt db status` or `plan` that reads it at that moment fails with `DB_INPUT_IO`,
naming the manifest and the operating system's error, and must be rerun.

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
at their **absolute** paths. `pwd -P` prints the physical directory: `ntnt db`
refuses a path through a symbolic link, and `$PWD` keeps one when the project was
entered through a link (on macOS, `/var` is a link to `/private/var`):

```sh
mkdir -p data && touch data/app.sqlite data/replay.sqlite
export NTNT_ENV=development NTNT_SECRETS_PROVIDER=env
export APP_DATABASE_FILE="$(pwd -P)/data/app.sqlite"
export APP_REPLAY_FILE="$(pwd -P)/data/replay.sqlite"

ntnt db status --target development
ntnt db plan --target development --out review-0001.json
# Review review-0001.json and the SQL. Approve only a plan that can run
# (outcome pass, with a plan_digest): for a blocked plan the check fails,
# and nothing is approved or migrated.
if jq -e '.outcome == "pass" and (.plan_digest | type) == "string"' review-0001.json > /dev/null; then
  # Approve exactly this plan:
  export APP_MIGRATION_APPROVAL="$(jq -r .plan_digest review-0001.json)"
  ntnt db migrate --target development --plan review-0001.json
fi
ntnt db verify --target development
```

The scratch can stay a `touch`ed empty file after your application has opened
the target: `std/db/sqlite` `connect()` switches a database to WAL mode, and
`ntnt db` gives an empty scratch the target's journal mode before replaying
(and a 0-byte one its encoding, page size and auto-vacuum too). A
plan binds the target's journal mode, so a plan made before the application first
opened the file fails `migrate` with `DB_PLAN_STALE` naming `target setting
journal_mode: plan delete, now wal`; plan and approve again.

`migrate` without `APP_MIGRATION_APPROVAL` fails with `DB_APPROVAL_REQUIRED` and
writes nothing; its `next_action` names `APP_MIGRATION_APPROVAL` and the plan's
digest to set it to, then rerun `migrate` with the same plan. Rerunning the same plan reports `already_applied`. For the next
change, run `ntnt db new <name>`: it creates `migrations/0002_<name>.sql` and prints
`ntnt db plan --target <target> --out review-0002.json`. Write its SQL, then plan
into that new file, approve its digest and migrate it, as above. `--out` never
replaces a file, so each plan gets its own; each new plan has a new digest.

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
too (see [Managed scope](#managed-scope)). Creating a schema needs the `CREATE`
privilege on the database. Each owner role above has it as its database's owner,
and it owns the schemas its migrations create. Do not create those in advance.

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
ntnt db plan --target development --out review-0001.json
if jq -e '.outcome == "pass" and (.plan_digest | type) == "string"' review-0001.json > /dev/null; then
  export APP_MIGRATION_APPROVAL="$(jq -r .plan_digest review-0001.json)"
  ntnt db migrate --target development --plan review-0001.json
fi
ntnt db verify --target development
```

A secret without an explicit `sslmode` (or with `sslmode=prefer`, a different user or
database, or several hosts) fails with `DB_CONNECTION_CONFIG`. A secret that cannot be resolved fails with
`DB_SECRET`, which names the secret and whether it is the target's or the scratch's,
never its value, and says which cause applies, each with its own `next_action`: unset
or empty in the environment, not declared under `[secrets]`, the env provider disabled
because `NTNT_ENV` is `production`, an unsupported `NTNT_SECRETS_PROVIDER`, or a
unix-socket provider that is misconfigured, unavailable or refuses it. A refused login fails with `DB_POSTGRES` and SQLSTATE `28P01` (wrong
password) or `28000` (for example `peer`): check `pg_hba.conf` and the secret.
Add `--json` to any command for the machine-readable report. If the
manifest declares only one database, `--database` is optional; `--target` is
required by every command except `init` and `new`. Target and database names are
case-sensitive manifest keys. A name that is not declared fails, echoing it and
listing the declared ones: `DB_CONFIG` "Target dev is not declared for database
app; declared targets: development." or `DB_DATABASE_UNKNOWN` "Database ap is not
declared; declared databases: app.", and `DB_DATABASE_REQUIRED` lists the
databases to choose from.

## Explicit configuration and secrets

The example below selects PostgreSQL. See [SQLite](#sqlite-native-files-and-rebuilds) for its file-backed configuration and distinct guarantees.

`--manifest` defaults to `ntnt.toml` in the current directory; there is no implicit ancestor/target selection. The migration directory is relative to that manifest, not the caller's working directory. Run `ntnt db` from the project directory or pass `--manifest <path>`: from a subdirectory it fails with `DB_INPUT_IO` "Manifest ntnt.toml: not found in the current directory". Every input file (the manifest, a `--plan` or `--against` file, each migration and sidecar) must be a regular file of at most 1 MiB with a single hard link; a refusal names the file as you gave it and the rule it broke: not found, a symbolic link, not a regular file, larger than 1 MiB, more than one hard link, changed while it was being read, or the operating system's read error.

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

`approval_secret` is optional for inspection but **required for every execution**, including additive changes and reruns. It names a separate top-level secret, never a connection secret. Its resolved value must be exactly the reviewed `plan_digest` (no whitespace trimming). Otherwise `migrate` and `reconcile` fail with `DB_APPROVAL_REQUIRED`, which names the database and its approval secret, never the secret's value: its `next_action` says to set that secret to this plan's `plan_digest` through the secrets provider and rerun with the same plan. For local development with the env provider it adds the command: `export <NAME>=<digest>`, or, for a name a shell cannot export because it holds `.` or `-` (`deploy.approval`, `app-approval`), `env '<name>=<digest>' ntnt db migrate ...` with the same manifest, database, target and plan. A secret the provider cannot read (its configuration is invalid, it is unavailable or denies access, or the value is not Unicode) is refused for that cause instead, with the same advice as a connection secret's. A database that declares no `approval_secret`, or whose secret has no `[secrets.<NAME>]` table, is told to declare it, with the table to add (a dotted name is quoted, as in `[secrets."deploy.approval"]`, since TOML reads `[secrets.deploy.approval]` as nested tables); as the manifest is part of the plan, plan and approve again after that. The deployment/operator publishes that value through a separately protected existing `std/secrets` provider after reviewing the plan and migration SQL. The digest binds SQL/metadata, policy, logical store/target and physical identity; **knowing the digest does not authorize execution**. Obtaining its exact value through the protected provider does. There is no `--approve`, `--force`, self-authorizing manifest Boolean, signing server or new approval service.

For PostgreSQL, use separate application and migrator credentials, and restrict schema DDL/ledger ownership to the migrator. SQLite has no database roles: restrict file/directory access and approval delivery through the operating system and provider, as described below. Development fixtures may use the environment provider; protected deployments use the existing Unix-socket provider and separately controlled approval delivery. NTNT cannot protect against someone who already controls that provider or direct database-owner credentials. Never publish resolved connection strings or provider values. Backup/PITR, restore testing, old-client compatibility and maintenance authorization remain separately operated responsibilities; a committed destructive change is not made reversible by transactional rollback.

Each PostgreSQL invocation uses a dedicated physical connection and a session advisory lock on the database-local `_ntnt` namespace (two-key lock `1853124212, 245`). The advisory lock serializes migrators; a second migrator waits up to `migrator_wait_seconds` (5 seconds by default; see [Time limits](#time-limits)) and then fails with `DB_LOCK_CONFLICT`. The reserved namespace deliberately owns **one logical store/managed schema per physical database**; separate stores must use separate databases. This is not a multi-tenant ledger. The lock spans all per-migration commits.

For each PostgreSQL migration, the runner:

1. Begins a `READ COMMITTED READ WRITE` transaction with `lock_timeout = 1s`. It does not lock tables up front: each statement takes only the locks PostgreSQL requires for it.
2. Takes fresh identity/history/catalog observations and validates the exact ordered planned prefix, captured source/policy and mutation-safe ledger.
3. Locks the tables that the migration's `requires`/`ensures` checks read in `SHARE` mode (writers wait, readers do not), runs `requires`, executes the captured full SQL script within the statement limit (`statement_seconds`, 90 seconds by default), then runs `ensures`.
4. Verifies the resulting catalog against that exact prefix fingerprint, records the relation locks the transaction holds in `progress[].locks`, and inserts the success ledger entry in the same transaction.
5. Commits that migration before starting the next. Failure never rolls back an earlier committed migration.

If a lock is not granted within one second, or the session is chosen as a deadlock victim, the whole migration transaction rolls back and is retried after a backoff (250 ms doubling, with jitter), up to 4 attempts; `progress[].lock_retries` counts them. A short lock timeout keeps application queries from queueing behind a migration that is waiting for a lock. Statements that need strong locks (most `ALTER TABLE`, `DROP`, `CREATE INDEX` without `CONCURRENTLY`) still block other access to their table while the migration transaction is open; this is **not online migration tooling** for long rewrites. Advisory locks alone only coordinate cooperating migrators. No lock protocol promises to stop administrators or actors with direct DDL privileges. Incoming foreign keys from outside the [managed scope](#managed-scope), into its tables or into the [tables the history creates in other schemas](#objects-the-history-creates-in-other-schemas), and all explicit cascading DDL are refused rather than claiming complete dependency/effect mediation.

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
approval_secret = "ANALYTICS_MIGRATION_APPROVAL"

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
[secrets.ANALYTICS_MIGRATION_APPROVAL]
required = true
```

Each database has its own `approval_secret`, so approving a plan for one never
approves a plan for another. `migrate` refuses a database that declares none with
`DB_APPROVAL_REQUIRED`, naming the database and saying to declare one.

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

The closed PostgreSQL AST subset supports Boolean expressions, identifiers/literals, comparisons and arithmetic, `IS NULL`/`IS NOT NULL`, `EXISTS`/`NOT EXISTS`, scalar subqueries, one managed heap table per SELECT, ordinary column GROUP BY/HAVING and builtin `count(*)`. Each query must have only these recognized clauses. Every table reference in a check must be explicitly qualified with the managed schema (for example, `app.accounts`), including nested queries. Unqualified names are rejected rather than resolved through `search_path`, so a managed table named `pg_class` cannot accidentally become a system-catalog read. Checks use `search_path=pg_catalog` for builtin function/operator lookup; this does not change unqualified migration-script names. The adapter verifies live catalog coverage and builtin base types/indexes, and conservatively rejects database-wide custom implicit/assignment casts before DDL or checks, unless they belong to an extension installed in the managed scope. A check may read a managed view, and a view reads what its query names, so these rules hold for everything a check can reach: every relation in the managed schema and, through the views among them, every relation those views read, transitively and in any schema, including objects the history did not create. Tables there must have column types that are builtin or in the managed scope, where inspection has covered them (enums, domains, composite types and an installed extension's types), relations must be tables, indexes, sequences, plain views or composite types, and views may use only builtin non-volatile functions. The check grammar writes no casts or typed literals, so a check never converts a value to a domain by name. Comparing a column with a literal converts the literal to the column's type, and for a domain that is its base type, so a plain domain's constraints never run, whatever they call. Below the top level of a column's type, though, the literal goes through each type's input, domain constraints included: an array's element type, a composite's attributes and a range's subtype, recursively, also under a top-level domain's base type. There a domain's constraints may call only builtin non-volatile functions (a regular expression, say); one whose constraints call a routine the history wrote or a volatile function, directly, through an operator or through a conversion to another domain, fails the checks with `DB_CHECK_UNSUPPORTED`, which names the table, column, type, domain and function (`table app.orders has column emails of type app.email[], and converting a literal to that type runs the constraints of domain app.email, which call app.valid(text) (a routine the history wrote)`). A view's conversions and operators run when a check reads it, so a view that converts values to a type holding such a domain at any level, its top level included, or uses an operator whose function the history wrote or is volatile, is refused the same way. An operator counts with its negator and commutator, which the planner may use in its place. A view that calls a builtin converting values to a type given at run time (`pg_input_is_valid`, `json_populate_record`, a type's input function) records no dependency on that type, so it is refused while any domain's constraints call such a routine. A routine the history wrote is one outside `pg_catalog` that no extension installed in the managed scope provides. A composite as a column's top-level type is counted too, although PostgreSQL rejects a literal compared with it as an anonymous record before any input runs. One disallowed object in that set fails every check of the phase with `DB_CHECK_UNSUPPORTED` before any of them runs: for example, when a managed view reads `audit.v`, which calls a routine that writes. Objects no check can reach, such as a view in `audit` that no managed view reads, do not affect checks. Builtin operators/functions resolve through the trusted PostgreSQL installation, not a `provolatile` assertion on a user routine.

On PostgreSQL a check must also compare values with the operators the application uses. Checks resolve operators with `search_path=pg_catalog`, where no operator outside `pg_catalog` is visible: comparing a `citext` column with a literal there converts the column to `text` and compares case-sensitively, while the application, with the managed schema on its search path, uses citext's case-insensitive `=`. NTNT cannot know the application's search path: it can be set per role or database (`ALTER ROLE ... SET search_path`), per connection or per transaction, and can include schemas the history never names, such as `extensions` or `public`. So a check is refused when a column it compares or computes with (directly, or as the single output of a scalar subquery) has a type, or a type in its domain's base-type chain, that a binary operator of that name takes as either operand, in any schema but `pg_catalog`: the managed scope, a schema the history creates, `public`, or a schema the application never puts on its path. For a type outside `pg_catalog`, an operator that takes a polymorphic `any*` type such as `anyelement` counts too. Builtin types count because the application can find such an operator before `pg_catalog`'s: `pg_catalog` has no `=(varchar,varchar)`, so an application with `app.=(varchar,varchar)` on its path uses it to compare a `varchar` column with a literal, where a check uses `text`'s `=`, and `app.=(integer,integer)` wins for an application whose path lists `app` before `pg_catalog`. For example, `audit.=(app.mood,app.mood)` refuses a check that compares an `app.mood` column with `=`, although an application without `audit` on its path uses `pg_catalog`'s enum `=`. The rule does not emulate PostgreSQL's operator resolution either, so it also refuses some comparisons that resolve the same way under both search paths: a `citext` column compared with a `text` column uses `text`'s `=` under both, and is refused, and so is `id + 1` on an `integer` column while intarray, whose `+(integer[],integer)` takes `integer`, is installed outside `pg_catalog`. Both kinds of refusal fail closed, and the view described below avoids them. Checks write no qualified column names: a column is the one its own query level's table has, else an enclosing level's (when neither has it, every table the check reads that has a column of that name counts). A name that no table at those levels has as a column is, as in PostgreSQL, the whole row of the table it names (`users = users` over `app.users`), with that table's row type. `DB_CHECK_UNSUPPORTED` names the check, the table, the column, its type and one such operator (`app.=(app.citext,app.citext)`), and `plan` lists the check in `plan.check_warnings` as `CHECK_OPERATOR_MISMATCH`. Still allowed: columns of builtin types that no such operator takes; enums, domains and composite types with no such operator (a plain enum compared with a literal uses `pg_catalog`'s `anyenum` operators either way); arrays such as `citext[]`, whose `=` compares elements with the element type's default operator class either way; and citext columns a check reads without comparing them (`IS NULL`, `count(*)`, `GROUP BY`). To compare such a column, do it in a view that a migration creates, which keeps the operator PostgreSQL chose when the migration created it, and check the view: over `CREATE VIEW app.admin_accounts AS SELECT id FROM app.users WHERE email = 'admin@example.com'`, the check `SELECT NOT EXISTS (SELECT 1 FROM app.admin_accounts)` compares as the application does.

A check must not change anything through a view it reads, directly or through other views, either. pg_depend never records a builtin function, so the views' stored queries are read: a call to a builtin that PostgreSQL marks parallel unsafe (`nextval`, `setval`, `set_config`, `txid_current`, the `lo_*` functions and others, read-only ones such as `currval` and `current_schema` included), or to an advisory-lock function or `pg_notify`, which it marks parallel restricted, refuses the check with `DB_CHECK_UNSUPPORTED` naming the check, the view and the function, and `plan` lists it as `CHECK_VIEW_WRITES`; read the underlying tables instead. Volatile builtins that change nothing, such as `random()`, `clock_timestamp()` and `gen_random_uuid()`, stay allowed. Both refusals happen in `migrate` before any check of the phase runs, and the migration rolls back.

Not supported: arbitrary functions/routines, casts, aliases/joins, SELECT INTO, CTEs, locking clauses, table functions/sampling, windows, wildcard projections, custom types/operators/access methods, session/transaction changes or arbitrary SQL. A SELECT keyword or READ ONLY transaction is **not a purity proof**. Unsupported syntax is rejected before target I/O, never approved by regex. There are at most 64 named checks per migration, each at most 64 KiB; names use simple ASCII identifiers.

`ensures` are transition checks, not permanent invariants: `verify` does not re-execute historical postconditions. Use database constraints for lasting concurrency/future-write enforcement and ordinary native/Intent tests for business behavior. Intent IDs remain traceability, not proof. The SQLite-only `sqlite_rebuild` field below is the sole supported adapter-mode metadata. Other execution/transaction modes remain rejected.

### Checks and row-level security

PostgreSQL checks read with `row_security=off`, as inspection does, so that no
row-level security policy can silently filter what a check sees. PostgreSQL then
refuses a read that a policy would affect (SQLSTATE 42501) instead of filtering
it. A check therefore cannot read a table where row-level security applies to the
migration role: one with `FORCE ROW LEVEL SECURITY`, which applies its policies to
its owner too, or one with row-level security that the role does not own. A view
the role owns reads its tables as the role, so the same holds for tables a check
reads through such a view. The unprivileged owner role this guide uses does not
bypass row-level security; a role with `BYPASSRLS`, or a superuser, does.

`plan` looks for such checks at the state each will run in (pending migrations'
`requires` and `ensures`, or the `adoption_checks` of an adoption plan) and lists
them in `plan.check_warnings`, with code `CHECK_ROW_SECURITY`, the migration
(`source`, `manifest` for adoption checks), `phase`, `check` and `relation`. The
human plan prints each under its migration:

```text
Pending "0001_tenants.sql" (...): hazards [...]; unknowns [...]
  Warning "CHECK_ROW_SECURITY" on "app.tenants" ("ensures" check "slugs_present"): "Check slugs_present ([ensures]) reads app.tenants (FORCE ROW LEVEL SECURITY), where row-level security applies to the migration role, ..."
```

The warning does not block the plan. `migrate` tests each check with PostgreSQL's
`row_security_active()` first. A check that names such a table is refused with
`DB_CHECK_ROW_SECURITY`, naming the check, its phase and each table (`objects`
holds their catalog keys); it does not run and is not counted in
`assertions.executed`. A check that reaches such a table only through a view runs,
since a view can name a table without reading it; if PostgreSQL refuses its read,
the failure is the same `DB_CHECK_ROW_SECURITY`, marked `(through a view)`. Either
way the migration's transaction rolls back. To run the check, have a superuser give the migration role
`BYPASSRLS` (`ALTER ROLE app_owner BYPASSRLS`), then plan, review and approve
again: the role's attributes are part of the plan's target binding. The scratch
role may then have `BYPASSRLS` too. Otherwise, change the check so that it reads
no such table. A plan without such checks has no `check_warnings`, so its digest
is the one earlier releases computed.

`plan.check_warnings` holds these entries and the `CHECK_OPERATOR_MISMATCH` and
`CHECK_VIEW_WRITES` ones above. Every entry has `code`, `source`, `phase`, `check`,
`relation` and `message`, and `CHECK_ROW_SECURITY` adds `forced`. One check can
have several entries. They come in the order `migrate` meets them: by migration,
each migration's `requires` before its `ensures`, by check name, and within one
check `CHECK_OPERATOR_MISMATCH` and `CHECK_VIEW_WRITES` before `CHECK_ROW_SECURITY`.
A check with both kinds is refused with `DB_CHECK_UNSUPPORTED` before any check of
its phase runs.

Files use `<positive-decimal-u64>_<ASCII-name>.sql`; names contain letters, digits, `_` or `-`. Ordering is numeric; `01` and `0001` are the same ID and cannot coexist. Gaps are allowed. Sidecars must have the exact SQL stem. The directory contains only SQL and matching TOML files, apart from entries it ignores: names that start with `.` (`.gitkeep`, `.DS_Store`, editor swap files and lock links, macOS `._` files) and `README` or `README.<ext>` with one extension (`README.md`, `readme.txt`) in any letter case. Ignored entries are never read and are not part of a plan's identity. Every other entry, `*.SQL` files and editor backups such as `0001_initial.sql~` included, is refused by name. Orphan sidecars, duplicate IDs, invalid UTF-8, unsafe paths, symlinks, hard links, directories in place of files and nonregular inputs are rejected.

Each refusal names the entry relative to the project, in its message and `source`
(`migrations/0002_more.SQL: the migrations directory holds only <id>_<name>.sql
migrations and their matching .toml sidecars.`), and its `next_action` says what
to do about the directory. A file that is not UTF-8 gives the line and column of
its first other byte. A sidecar that is not valid TOML, or holds an unknown key or
a value of the wrong type, fails with `DB_METADATA_UNSUPPORTED` giving the line
and column, never the file's contents. `DB_CHECK_UNSUPPORTED` names each check
outside the supported subset with its phase (`snmp_oid_unique ([requires])`), and
`DB_DUPLICATE_ID` names both files that share an ID.

Checksum v1 is lowercase SHA-256 over length-framed byte parts. Each part is prefixed by its unsigned 64-bit big-endian byte length. The parts are:

1. `ntnt-migration-v1`
2. exact SQL filename bytes
3. exact SQL file bytes
4. `sidecar` or `absent`
5. exact metadata bytes, or empty bytes when absent

Whitespace, comments, filenames, metadata comments, and even absent versus empty metadata matter. No newline or SQL normalization occurs. Applied ledger entries must be the exact ordered inventory prefix: missing/edited files, inserted historical migrations, reordered IDs and dirty attempts fail before any replay can be mistaken for valid history.

Each of these refusals names the migration in its message and `source`, and its
`next_action` says how to restore a valid history. `DB_HISTORY_EDITED` names the
applied migration whose bytes changed, with the checksum the ledger records and the
checksum its file and sidecar have now (an editor or git `core.autocrlf` that
changes line endings is a common cause); restore the applied bytes and put the
change in a new migration. `DB_HISTORY_MISSING` names the applied migration that is
missing from the migrations directory. `DB_HISTORY_DIVERGENT` names the first
ledger position whose ID or name differs, with both names. A migration this target
has not applied that is numbered below the last ID in its ledger, such as one merged
from another branch, is named with advice to rename it to an unused ID above that
ID, as for `DB_DUPLICATE_ID`, only if no target has applied or adopted it:
adoption records migrations without running them, and a migration is never
renumbered once any target has applied or adopted it. When it has a
sidecar, the advice names both files and one new stem for them. An adopted
checkpoint records one checksum for the whole prefix it covers, so its
`DB_HISTORY_EDITED` names that range of migrations rather than one file. That
checksum also tells a migration added below the checkpoint's last one from the ones
it covers: one added migration is named with the same advice, never a covered one.
When more were added, the advice is to compare the migrations directory with the
adopted release, keep the covered migrations under the names and bytes they were
adopted with, and rename each added one to an unused ID above the last. Checksums
are shown, file contents never; a value read from the ledger is shown only when it
has the shape NTNT writes there (a migration file name, a decimal ID, a SHA-256
digest).

Capture is bounded to 1 MiB per input (including a saved review artifact), 16 MiB of migration files, and 1,024 migrations. On PostgreSQL, each replayed step still inspects the whole scratch catalog (#421), and commits and records its transactions in the marker (#430), so a history that adds a table per migration stays within `plan`'s replay deadline (120 seconds plus `migrator_wait_seconds`) up to about 800 migrations, and its first `migrate` from an empty database within the work deadline up to about 600. A plan that reaches the deadline fails with `DB_TIMEOUT` and cleans scratch up afterwards; if that cleanup cannot finish either, the next plan recovers what the replay committed. A history that changes a stable set of tables goes further. For PostgreSQL, a script may have up to 262,144 tokens; each statement is parsed separately, up to 4,096 tokens and recursion depth 64. A larger statement is reported as `SQL_NOT_PARSED` but still replays. SQLite scripts are parsed whole, up to 4,096 tokens (including whitespace tokens) and recursion depth 64; oversized SQLite SQL is unknown, not automatically safe. Source files and manifest policy are captured once, then replay consumes those captured bytes even if their paths are subsequently replaced. A new command recaptures current inputs and invalidates an old binding if they changed.

## Disposable replay and its boundary

Provision scratch yourself using existing native test/container facilities. NTNT does not create a database/container, copy target rows, reuse target credentials, or reset an existing database.

PostgreSQL scratch must:

- have `disposable = true` and a separate secret and role;
- resolve to a different **cluster-system-identifier/database-OID pair**, not merely a different URL spelling;
- exactly match the target's server version number, encoding, locale/provider/version, timezone, DateStyle, IntervalStyle and client/string settings;
- be owned by a nonsuperuser role with no memberships, `CREATEDB`, `CREATEROLE` or replication. It may have `BYPASSRLS` only when the target role bypasses row-level security (directly or as a superuser), so seed rows in migrations see the same policies in scratch as on the target;
- contain only the empty stock public namespace and system objects, with no user routines/types/relations, event triggers, extra extensions or default grants;
- (since 0.6.5) give the scratch role no privileges on objects it does not own, other than on `public`: no grants on other databases, tablespaces, configuration parameters or system objects. Cleanup uses `DROP OWNED`, which would revoke them; such a scratch is refused with `DB_SCRATCH_UNSAFE`, whose `objects` field names each grant;
- (since 0.6.5) run with `track_counts` on, PostgreSQL's default (otherwise `DB_SCRATCH_UNSAFE`);
- (since 0.6.5) hold nothing else the scratch role owns, such as large objects or publications, which the checks above do not look for. Cleanup's `DROP OWNED` would drop them, so such a scratch is refused with `DB_SCRATCH_NOT_EMPTY`, whose `objects` field names them, and kept as it is.

The schema `_ntnt_replay` in scratch is NTNT's replay marker (below); like `_ntnt`, migrations may not act on it.

Identity discovery uses `pg_control_system()` and catalog metadata. Missing permission to establish identity fails closed. Server administrators and the PostgreSQL installation are trusted; this is not protection against a hostile superuser modifying PostgreSQL's own catalogs or concurrent administrators changing scratch after validation. Use an exclusively disposable fixture, preferably network-isolated. Restored/cloned clusters sharing identity can be conservatively rejected.

Each migration runs with normal row-level security; deferred constraints are checked at its end and attributed to it. Inspection and checks still read with `row_security=off`, so a check cannot read a table whose row-level security applies to the migration role, such as one with `FORCE ROW LEVEL SECURITY`, unless that role has `BYPASSRLS`. `plan` warns about such checks and `migrate` refuses them by name; see [Checks and row-level security](#checks-and-row-level-security).

### How replay commits and cleans up scratch

Replay owns a dedicated physical connection, not a pool handle. Since 0.6.5 it **commits each migration on scratch**, as `migrate` commits each one on the target: every migration runs in a `READ COMMITTED` transaction with the target's settings (the same `SET LOCAL`s; only `synchronous_commit` is off on scratch), is inspected, and commits. A history therefore replays exactly as the target runs it. A value `ALTER TYPE ... ADD VALUE` adds in one migration can be used by the next one on every supported major, PostgreSQL 16 included, and a migration that adds a value and uses it fails at plan, on 17 and 18 too, with the `DB_ENUM_VALUE_UNCOMMITTED` (SQLSTATE 55P04) that `migrate` would hit on the target. (0.6.4 and earlier replayed the whole history in one rollback-only transaction, where PostgreSQL 16 rejected the first case and 17 and 18 accepted the second.) The authorized per-migration executor reuses that exact captured-SQL primitive.

Because migrations commit, NTNT cleans scratch up afterwards, and only drops what it can prove it created:

- **Replay lock.** Before it reads scratch, a replay takes a session advisory lock on the scratch database (two-key lock `1853124212, 430`) and holds it until cleanup ends, so two plans, statuses or verifies on one scratch never interleave. A second one waits up to `migrator_wait_seconds` (5 by default; see [Time limits](#time-limits)) and then fails with `DB_SCRATCH_BUSY`, having changed nothing. A migration that releases the lock (`pg_advisory_unlock_all()`, say) is refused with `DB_REPLAY_UNSUPPORTED` before it commits.
- **Marker.** After the emptiness checks, replay commits the managed schema together with a marker: the schema `_ntnt_replay` and its table `replay_v1`, whose one row holds a replay ID, the start time, the cluster's system identifier (`pg_control_system()`) and the database's OID it was written in, the state of `public` (owner, privileges and comment), the scratch database's privileges, connection limit and comment, and the settings (`pg_db_role_setting`) of the scratch database and of the scratch role. In the same transaction as each migration's commit (and as the marker's own), replay adds to the row the IDs of the transactions, subtransactions of PL/pgSQL exception blocks included, that wrote a catalog row. Catalog inspection and `outside_objects` never include it, and a migration that acts on it (with `UPDATE`, `DELETE`, `DROP`, `TRUNCATE`, `INSERT`, `ALTER` or anything else that changes its shape, row or recorded transactions) is refused with `DB_OUTSIDE_SCOPE`, before it runs when it names it, otherwise before it commits.
- **Cleanup.** After replay, whatever its outcome (success, a failing statement, a limit, a refusal, a coverage failure or the replay deadline), replay drops what the scratch role owns with `DROP OWNED BY CURRENT_USER`, which removes the marker too, gives `public` back its name, owner, privileges and comment (a migration may grant on it, comment on it, take it over, rename it, or drop it and create it again, for `pg_database_owner` say), gives the scratch database back its privileges (compared as sets, so an explicit default counts as the default), connection limit and comment, and the database and the scratch role their settings, and runs the emptiness check again, all in one transaction. Casts have no owner, so the casts of the scratch role's own types are dropped first. Cleanup drops nothing unless its session holds the replay lock: it takes the lock again if a migration released it, and fails if another session took it meanwhile. A cleanup that fails is reported as `DB_SCRATCH_CLEANUP` after the replay's own result, and its results are discarded; the marker then stays. So is a marker the replay committed that another session dropped or changed. `scratch.cleanup` is `pass` when cleanup ran, `not_needed` when the replay committed nothing (a refused prepare, say), and `unknown` when it failed.
- **Cleanup's limits.** Cleanup's statements wait at most 2 seconds for each lock and run at most 5 seconds, the limits of NTNT's catalog exchanges, except `DROP OWNED`, which may run 10 seconds (dropping what an 800-migration history commits takes well under one). Its transaction stops after 15 seconds, its statement cancelled, so that with the rollback and the release of the replay lock cleanup stays within its 30 seconds. When another session holds a lock on what the replay committed (an open transaction that read a replayed table, of the scratch role or of another role), `DROP OWNED` gives up after 2 seconds and cleanup fails with `DB_SCRATCH_CLEANUP`, naming each such session by PID and role; nothing is dropped and the marker stays, so the next plan, status or verify recovers once they are gone (while a session of the scratch role stays connected to scratch, it fails with `DB_SCRATCH_BUSY` instead). A lock wait after which no session holds a lock on scratch's objects any more, as when autovacuum gives way to it (after `deadlock_timeout`), is tried again, up to three times. The same limits apply when a replay cleans up an unfinished one's leftovers.
- **Replay deadline.** Replay (with the target's first inspection and the wait for the replay lock) stops at the 120-second inspection deadline plus `migrator_wait_seconds`, as before; NTNT cancels the statement it was running and reports `DB_TIMEOUT`, and cleanup then has 30 seconds of its own. A statement that outlasts its client deadline is cancelled the same way before cleanup.
- **Unfinished replays.** If a replay never cleaned up (its process was killed between migrations, or cleanup itself failed or ran out of time), the next plan, status or verify finds its marker. It cleans scratch up the same way before replaying, in the same transaction, and reports `scratch.recovered` with the replay ID and start time, only when it can prove that everything `DROP OWNED` would drop is what that replay committed: the marker was written in this database of this cluster (not copied by a dump and restore, a template or a physical copy), scratch holds no `_ntnt` schema (a replay never commits NTNT's ledger there, so the database is, or was, a deployment target), no other role owns anything there (objects of extensions the scratch role installed aside), every catalog row of what is there (`pg_class`, `pg_type`, `pg_proc`, `pg_namespace` and the other catalogs of objects, with their columns, constraints, triggers, policies, rules, defaults and comments) was last written by one of the marker's transactions, and those are recent enough for 32-bit transaction IDs to tell them apart. Freezing and in-place updates (VACUUM, ANALYZE) keep a row's `xmin`; an `ALTER`, `GRANT` or `COMMENT` outside the replay does not. While another session of the scratch role is connected to scratch, that replay may still be running, so it fails with `DB_SCRATCH_BUSY` and changes nothing. If that replay left the timezone, `DateStyle` or `IntervalStyle` of new sessions changed (a migration's `ALTER DATABASE ... SET timezone`), so that they no longer match the target, the next plan recovers first, restoring them, and then fails with `DB_SCRATCH_MISMATCH` asking for a rerun. Otherwise it refuses with `DB_SCRATCH_NOT_EMPTY`, naming why (and the objects it cannot prove, or `schema:_ntnt`), and drops nothing; its `next_action` says to recreate the scratch database if it really is only this configuration's scratch. Without the marker it refuses exactly as before: `DROP OWNED` never runs in a database without NTNT's marker.
- **A target that holds the marker** is, or was, some configuration's scratch database, whose next plan could clean it up, so `status`, `plan`, `verify` and `migrate` refuse it with `DB_TARGET_IS_SCRATCH`, naming `schema:_ntnt_replay`, before reading its history.
- **Changes cleanup cannot undo** are refused before the migration that made them commits, with `DB_REPLAY_UNSUPPORTED` naming the migration: a new password, memberships, the role's attributes or comment, and the settings, privileges or comments of databases other than scratch (all of which the parser gate allows inside `DO` blocks and functions); objects given to another role (`pg_database_owner`, through `SET ROLE`); and a session left acting as another role. On the target such statements would change the deployment role or another database. The scratch role cannot read `pg_authid`, so a password change is seen from PostgreSQL's write counters, which also count a write a subtransaction rolled back (a password changed in an exception block that then raised): that is refused too. What a migration does to the scratch database itself or to its role's settings (`ALTER DATABASE ... SET`, `ALTER ROLE CURRENT_USER [IN DATABASE ...] SET`, a `GRANT`, `REVOKE` or `COMMENT` on the database, a connection limit) commits and is restored by cleanup, so the next plan still matches the target. Two exceptions are refused with `DB_REPLAY_UNSUPPORTED` before they commit: a connection limit of 0, and privileges that leave the scratch role without `CONNECT` on scratch (`REVOKE CONNECT` from the role itself, or from `PUBLIC` when the role connects through `PUBLIC`'s grant; the message names whose grant was revoked). A replay stopped before cleanup would leave the scratch role unable to connect, so no later plan could clean scratch up. Cleanup restores the privileges in its one transaction, on the connection it already holds, so restoring them cannot lock it out.

The scratch requirements above are unchanged except for the three marked 0.6.5. NTNT never creates, resets or drops the scratch database itself. Catalog derivation deliberately skips transition contracts: an empty replay cannot establish data-dependent production assertions. Fresh/populated native rehearsal tests invoke the same `migrate` executor on disposable fixtures with representative rows; they are not a second application-level runner or a claim about production data.

### What PostgreSQL replay runs

PostgreSQL migrations are complete SQL scripts. Scratch replay and authorized execution both send each captured script, unchanged, in its own transaction, which commits (above). That includes `DO` blocks, functions, triggers, policies, grants, seed `INSERT`s, `SET CONSTRAINTS` and other transactional SQL a real application history contains. PostgreSQL itself determines the resulting catalog.

PostgreSQL cannot use an enum value in the transaction that added it with `ALTER TYPE ... ADD VALUE` (SQLSTATE 55P04). Such a migration fails at plan, in replay, as it would in `migrate`, with `DB_ENUM_VALUE_UNCOMMITTED`, which names the migration, the statement when PostgreSQL reports where it failed, and the migration's `ADD VALUE` statements: put `ALTER TYPE ... ADD VALUE` in its own migration and use the value in a later one. Since replay commits each migration as `migrate` does, that fixes `plan` and `migrate` alike, on PostgreSQL 16 too.

Before anything runs, the script is tokenized (quoted and dollar-quoted text stays intact) and each top-level statement is checked. These statements are rejected with `DB_REPLAY_UNSUPPORTED`, because they would end or split the migration transaction, change who the session acts as, or act outside the database:

- transaction control: `BEGIN`, `START TRANSACTION`, `COMMIT`, `END`, `ROLLBACK`, `ABORT`, `SAVEPOINT`, `RELEASE`, `PREPARE TRANSACTION`;
- session identity: `SET [LOCAL|SESSION] ROLE`, `SET SESSION AUTHORIZATION`, `SET session_replication_role`, `RESET ROLE|SESSION|ALL`, `DISCARD`;
- non-transactional work: `VACUUM`, `CONCURRENTLY` operations (except the single-statement index migration described below), `REINDEX DATABASE|SYSTEM`;
- server scope: `COPY`, `LOAD`, `CHECKPOINT`, `ALTER SYSTEM`, and creating/altering/dropping databases, tablespaces and roles;
- user mappings: `CREATE`, `ALTER` and `DROP USER MAPPING` (`USER_MAPPING`), which hold a role's credentials for a foreign server; see [Foreign servers and user mappings](#extensions-a-superuser-installs);
- explicit `CASCADE` drops, whose transitive effects are not bounded. `ON DELETE CASCADE` and `ON UPDATE CASCADE` foreign-key actions are allowed.

Text inside `DO` and function bodies (quoted, or a SQL-standard `BEGIN ATOMIC ... END` body) is not inspected; it runs inside the same transaction with the migration role's privileges. Keep role creation out of migrations and provision roles separately. Database-wide custom implicit/assignment casts still block execution, because ordinary DDL coercion could run them without appearing in the reviewed script. Casts that belong to an extension installed in the managed scope, such as citext's, are the extension's own code, which its version covers, and do not block. Any other is refused before the next migration runs with `DB_CHECK_UNSUPPORTED`, whose `objects` field names each cast (`cast:app.mood AS text`) and whose `source` names that migration: drop the cast, or create it `AS EXPLICIT`.

Each pending migration's `analysis` lists its statements' `operations`, known lock/effect `hazards`, and `unknowns`. `SQL_NOT_PARSED` means a statement (for example a `DO` block) was not parsed. `EFFECTS_NOT_ANALYZED` means it parsed but falls outside the subset NTNT analyzes in detail: ordinary tables with built-in types and literal defaults, simple constraints, column-only btree indexes, and non-cascading table/index drops. Unknowns are review prompts, not blockers: the replayed catalog and drift comparison still apply. **Scratch replay is not a sandbox.** Use a disposable database with a separate unprivileged owner, as required above.

For PostgreSQL, `analysis.locks` predicts the table locks each statement takes (`statement` is its 1-based position in the script, and `through` ends a run of consecutive statements that take exactly the same locks, such as many seed `INSERT`s; `mode` uses the same `pg_locks` names as `progress[].locks` after `migrate`). Predictions come from the SQL and PostgreSQL's documented lock levels, not from replay: replay runs on empty tables, so it cannot show how long a lock would block. Statements NTNT cannot parse, `DO` blocks, `DROP INDEX` and queries (which can lock rows or call functions) add `LOCKS_NOT_PREDICTED` to `unknowns`. Tables created earlier in the same migration are omitted, except after `CREATE TABLE IF NOT EXISTS`, which may find an existing table.

`analysis.lock_warnings` flags locks that block or scan an existing table:

- `BLOCKS_WRITES_WHILE_INDEXING`: `CREATE INDEX` without `CONCURRENTLY`.
- `SCANS_TABLE_UNDER_LOCK`: `SET NOT NULL`, a `CHECK` or foreign key added without `NOT VALID`, or a primary key/unique constraint built without an existing index.
- `REWRITES_TABLE`: a column type change.
- `MAY_REWRITE_TABLE`: a new column with a volatile default, identity or stored generated value.

Warnings are review prompts and do not block a plan. The safer forms are `ADD CONSTRAINT ... NOT VALID` followed by `VALIDATE CONSTRAINT` in a later migration, and a `CHECK (column IS NOT NULL) NOT VALID` validated before `SET NOT NULL`.

For PostgreSQL, `analysis.outside_scope` lists the statements that may create, change, drop, grant or revoke objects in a schema outside the [managed scope](#managed-scope), and the migration gets the hazard `OUTSIDE_MANAGED_SCOPE`. Each entry gives the `statement` number, the `schema`, and `via`: `name` for a schema-qualified name, an `IN SCHEMA` list or an `ALTER ... SET SCHEMA` in a statement NTNT parses; `search_path` for a `SET search_path`, `SET SCHEMA` or `set_config('search_path', ...)` that names the schema, so that later unqualified names in the migration may resolve there; `token` for a schema-qualified name, or a name after `SCHEMA`, in SQL NTNT does not parse or does not read names from (such a name may also be a column of a table alias). The human plan prints each one under its pending migration:

```text
Pending "0002_outside.sql" (...): hazards ["AUTHORIZATION_CHANGE",...,"OUTSIDE_MANAGED_SCOPE"]; unknowns [...]
  Outside the managed scope: statement 1 names schema "public"
  Outside the managed scope: statement 2 puts schema "public" on the search path
  Outside the managed scope: statement 5 names schema "public" in SQL NTNT does not analyze
```

These are review prompts like the other hazards: the plan stays `requires_review`, `migrate` runs the statements as written, and an applied migration is not flagged again. Statements that only read or write rows, unqualified names and text inside `DO` or function bodies (quoted or `BEGIN ATOMIC ... END`) are not flagged; statements on `_ntnt` and system schemas are refused instead. A plan saved by an earlier build whose pending migrations are flagged no longer matches its migrations' analysis; plan it again.

### Extensions

A migration may create an extension, for example
`CREATE EXTENSION IF NOT EXISTS pg_trgm SCHEMA app`. Replay runs it as the scratch
role and `migrate` as the target role, and neither is a superuser. PostgreSQL lets
a role that is not a superuser create only **trusted** extensions (the `trusted`
column of `pg_available_extension_versions`; `pg_trgm`, `citext`, `hstore`,
`pgcrypto`, `uuid-ossp` and `btree_gist` are some), and only with `CREATE`
privilege on the database, which its owner has. An **untrusted** extension, such as
`postgres_fdw`, `dblink`, `file_fdw` or `pg_stat_statements`, needs a superuser.
Creating one fails with `DB_EXTENSION_PRIVILEGE` instead of a bare SQLSTATE 42501.
The diagnostic names each `CREATE EXTENSION` statement and extension that the
refusal explains (an extension created in a `DO` block, or installed by `CASCADE`
as a requirement, is explained without a name), and says what to do. An
extension's objects must also stay within
[inspection coverage](#inspection-coverage-and-drift).

#### Extensions a superuser installs

An untrusted extension, or any extension a DBA installs out of band (monitoring
such as `pg_stat_statements` is common), is declared in the manifest and installed
by a superuser in **both** the target and the scratch database, at the same
version and in the same schema:

```toml
[databases.app]
engine = "postgresql"
schema = "app"
extensions = ["postgres_fdw", "pg_stat_statements"]
```

```sql
-- As a superuser, in the target database and in the scratch database:
CREATE EXTENSION postgres_fdw SCHEMA public;
CREATE EXTENSION pg_stat_statements SCHEMA public;
```

- A declared extension may live in any schema, `public` included, except `_ntnt`
  and PostgreSQL's own schemas. In the managed schema it must be `public`: replay
  creates any other managed schema, so scratch cannot hold it beforehand.
- Migrations may keep `CREATE EXTENSION IF NOT EXISTS postgres_fdw`. With the
  extension installed it does nothing, in replay and on the target.
- Inspection covers a declared extension wherever it is installed, as
  `extension:<name>` with its version and schema. A different version or schema in
  scratch than on the target is drift on that object. Its members outside the
  managed scope are not inspected. When `public` is the managed schema, its
  members there are covered by its version, the relations it installs included,
  and they are not taken for an unrecognized history on a target without a
  ledger. When a declared extension itself differs (a superuser moved it to another
  schema with `ALTER EXTENSION ... SET SCHEMA`, say), drift lists it once, as
  `extension:<name>`, with the differences of its members, which moved with it,
  under that entry's `members` rather than each on its own.
- An extension in scratch that `extensions` does not declare is refused with
  `DB_SCRATCH_NOT_EMPTY` naming it (`extension:hstore`); declare it (and install it
  on the target too, at the same version and in the same schema) or have a
  superuser drop it from the scratch database.
- Scratch must hold each declared extension. One that is missing, or installed in
  `_ntnt`, a system schema or a managed schema other than `public`, is refused with
  `DB_SCRATCH_MISMATCH` naming it. Scratch's `public` and the declared extensions'
  schemas may hold only what those extensions own: their members, and what
  depends on a member internally or automatically, such as array and row types,
  the relations of composite types (tablefunc's and dblink's) and indexes.
  Anything else there is refused with `DB_SCRATCH_NOT_EMPTY`. Replay does not
  count what they own as the history's work.
- A superuser owns the declared extensions and what they install, so replay's
  cleanup (`DROP OWNED`) keeps them, and cleaning up after a replay that never
  finished neither counts them as another role's objects nor asks the marker to
  have recorded their catalog rows: scratch is recovered with them in place.
- A privilege a DBA grants the scratch role itself on what a declared extension
  installed, such as `GRANT USAGE ON FOREIGN DATA WRAPPER postgres_fdw TO
  <scratch role>`, is refused with `DB_SCRATCH_UNSAFE` naming it, before anything
  runs: `DROP OWNED` would revoke it, and only the extension's owner can grant it
  again. Grant it to `PUBLIC` in the scratch database instead
  (`GRANT USAGE ON FOREIGN DATA WRAPPER postgres_fdw TO PUBLIC`), which
  `DROP OWNED` keeps. Privileges on what declared extensions install are not
  compared, so the target's migration role keeps its own grant.
- **Foreign servers and user mappings.** With a declared foreign-data wrapper's
  extension (postgres_fdw), a migration may create a server
  (`CREATE SERVER app_remote FOREIGN DATA WRAPPER postgres_fdw OPTIONS (...)`); the
  target role needs `USAGE` on the wrapper, and in scratch `PUBLIC` does. Servers are
  database-wide. One the history creates is covered as `server:<name>`, by its
  wrapper, type, version, owner, privileges, comment and options; options can hold
  host names, so reports show only their digest. An out-of-band change to any of
  these on the target (`ALTER SERVER ... OPTIONS`, `OWNER TO`) is drift on that
  object. A server no migration creates (a DBA's) is uncovered by name, and so is a
  foreign-data wrapper no extension installed (`other:foreign_data_wrapper.<name>`);
  foreign tables are uncovered as relations. Scratch must hold no server before
  replay. `CREATE`, `ALTER` and `DROP USER MAPPING` are refused in migrations with
  `DB_REPLAY_UNSUPPORTED` (`USER_MAPPING` in `analysis.unknowns`): a user mapping
  holds the credentials a role uses on the server and belongs to that role. Create
  it outside the history, as a DBA step on the target (and in the scratch database
  if replay needs it), with the credentials kept in the secrets provider. NTNT does
  not inspect user mappings, so one created that way is neither drift nor uncovered.
- A declared extension's casts do not block DDL, as an in-scope extension's do
  not.
- The key is part of the plan's policy, so adding or changing it needs a new plan.
  A manifest without it keeps its plan digests.

An extension installed outside the managed scope without a declaration is still
reported in `catalog.uncovered` (`DB_COVERAGE_INCOMPLETE`), and the diagnostic says
to declare it. A declared extension installed in `_ntnt` makes the ledger
unrecognized (`DB_LEDGER_UNKNOWN`), as anything else there does.

Managed objects that use a declared extension's members outside the scope, such as
a column of type `public.hstore` or a view that calls `dblink`, stay outside
inspection coverage. A trusted extension that a migration creates in the managed
schema is covered with its members.

### Concurrent index builds

A migration whose entire script is one named `CREATE [UNIQUE] INDEX CONCURRENTLY` on a managed table runs outside a transaction (`analysis.non_transactional`). It takes only `SHARE UPDATE EXCLUSIVE` on the table, so reads and writes continue while the index builds. The build waits for transactions already running on the table rather than failing on the 1-second lock timeout. The statement limit (`statement_seconds`, 90 seconds by default) covers the whole build, waits and table scan included; a build that exceeds it fails and its invalid index is dropped. For a large table, raise the limit in [`[databases.<name>.limits]`](#time-limits), then plan and approve again. Comments around the statement are allowed; any other statement, an unnamed index or a table outside the managed schema is rejected as before. Such a migration cannot have `requires` or `ensures` checks, because no transaction surrounds the change. Custom implicit/assignment casts block the build as they block other DDL, and the build uses replay's storage settings (`default_tablespace=''`).

Replay builds the same index without `CONCURRENTLY` inside that migration's replay transaction, so the plan's expected catalog is identical. On the target, `migrate`:

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

Plain views, sequences, SQL and PL/pgSQL functions/procedures (full definition, owner and grants), user triggers, RLS policies, declarative partitions in their parent's schema, expression/partial indexes on built-in access methods, `EXCLUDE` constraints, enums (their labels, in order), domains (base type, `NOT NULL`, default, collation and constraints), composite types (their attributes, as columns, and the type's privileges), operators whose function is covered, and extensions installed in the managed scope (by version) are compared like tables: an out-of-band change is reported as drift on the named object, such as `ALTER TYPE ... ADD VALUE` on `type:order_status`. Columns, domains and arrays of these types are covered. An operator's key carries its argument types, as a routine's does (`other:operator.===(integer,integer)`), and an operator class's or family's its access method (`other:operator_class.citext_ops USING btree`).

An extension's version covers its members: the types, functions, aggregates, operators, operator classes and families, and casts it installs. Their owner and grants are the installer's and are not compared. Indexes, columns and constraints that use them are covered, so `CREATE EXTENSION pg_trgm WITH SCHEMA app` with a `gin_trgm_ops` index, or a `citext` column, plans and migrates, and an out-of-band `ALTER EXTENSION ... UPDATE` is drift on `extension:<name>`.

Materialized views, functions in other languages, aggregates and base or range types other than an extension's, non-partition inheritance or partitions in a different schema from their parent, foreign tables, custom collations/access methods, operator classes and families other than an extension's (only a superuser can create one), extensions in other schemas, event triggers, security labels, other namespace objects and references to routines outside `pg_catalog` and the managed scope expose incomplete coverage. Unreadable objects or unknown ledger shape also block a clean result. Comparison is by definition, not behavior: NTNT does not analyze what a function body or policy expression does. This is not dependency closure, security-policy verification or replication/publication configuration auditing.

Engine-deparsed SQL, defaults, expressions, comments, enum labels and storage option payloads are represented by **definition hashes**, not printed literals: schema definitions can themselves contain credentials. Object names, structural fields, owner/grant facts and object-level differences remain visible. This avoids echoing connection credentials, SQL error payloads or live row samples. It is not an SQL dump; review source SQL separately through appropriately authorized tooling.

Drift compares `observed` with `expected(applied_prefix)`, not repository HEAD. Differences name the affected object and show expected/observed properties or definition fingerprints. An incomplete comparison is `unknown`, never “no drift.” Unsupported pending SQL can block a candidate plan while a separately verified applied prefix still passes. `verify` does not replay pending SQL.

`DB_COVERAGE_INCOMPLETE` names what could not be inspected: its `objects` field lists the uncovered catalog keys, for example `index:"contact_requests_pending"` or `table:"t"`. When the objects are on the target, `catalog.uncovered` lists them too. When replaying a migration produced them, they exist only in the replayed intermediate catalog, and `source` names that migration. The human output prints both fields under the diagnostic.

`DB_CATALOG_AMBIGUOUS` means two catalog objects share one key, so neither can be compared: its `objects` field names the key and, when replaying a migration produced them, `source` names that migration. Overloaded operators, and an operator class or family of one name in two access methods, have distinct keys. The one case known to remain is a database-wide default privilege and one in the managed schema for the same role and object type (`ALTER DEFAULT PRIVILEGES` without and with `IN SCHEMA`), which are outside coverage anyway: drop one of them.

### Restored databases

A database restored with `pg_dump` and `pg_restore` verifies like the one it was dumped from (#460). The restore parses the definitions in the dump again, and PostgreSQL then stores two kinds of value differently, though the schema behaves the same. Inspection compares both as a restore leaves them:

- **`AND` and `OR` association.** Parse analysis nests an `AND` (`OR`) inside another when it expands `BETWEEN`, `NOT BETWEEN`, an `IN` list of columns, a row comparison or `IS DISTINCT FROM` between rows. When the grammar parses the deparsed text again, it merges an `AND` (`OR`) that is the first operand of another into it. `CHECK (timeout_ms BETWEEN 1000 AND 15000 AND timeout_ms % 1000 = 0)` is stored as `(((timeout_ms >= 1000) AND (timeout_ms <= 15000)) AND ((timeout_ms % 1000) = 0))` and restored as `((timeout_ms >= 1000) AND (timeout_ms <= 15000) AND ((timeout_ms % 1000) = 0))`. Inspection compares the merged form of CHECK constraints (of tables and domains), `EXCLUDE` predicates, index predicates and expressions, policies' `USING` and `WITH CHECK`, column defaults and generated expressions, domain defaults, views, triggers' `WHEN`, partition keys and SQL-standard routine bodies (`RETURN`, `BEGIN ATOMIC`). Merging drops only those parentheses, and `AND` and `OR` are associative, so the expression means what it did. Nesting anywhere else survives a restore and is compared as stored, and so is a definition NTNT cannot read with certainty, such as one with a `$` or a comment outside quotes.
- **`connoinherit` of PRIMARY KEY, UNIQUE and EXCLUDE constraints.** A partition's constraint cloned from its parent's has it false. pg_dump adds the constraint to the partition with `ALTER TABLE ONLY` and attaches its index, which leaves it true. It does nothing for these constraints, so their `no_inherit` is compared as PostgreSQL sets it when it creates one: true for a constraint of its own, false for one attached to its parent's (`local` false). CHECK and NOT NULL constraints keep their `connoinherit`, which changes what they do.

A real change to one of these values is still drift: on the restored database, a CHECK with another bound is drift on that constraint.

A plan records `canonicalization_version: 2` when one of its catalogs holds such a value, whose fingerprint then differs from the one an earlier version computed, and `1` otherwise, with the fingerprints and `plan_digest` an earlier version computes. `migrate`, `reconcile` and `--against` match a saved plan's fingerprints with the target's catalog in both forms, so a plan saved by an earlier version still migrates. After an upgrade, a new plan for a history with such values has a different `plan_digest` from the one an earlier version made for the same state, and needs approving. No plan carries over to a restored database, whose cluster or database identity differs: `DB_PLAN_STALE` names it, so plan again there. The ledger records no catalog fingerprint, so applied history is not affected.

### Managed scope

PostgreSQL inspects the **managed scope**: the configured `schema` plus every
schema the captured migration history creates, and, one by one, the objects the
history creates in other schemas ([below](#objects-the-history-creates-in-other-schemas)). A schema is created by the history
when a top-level `CREATE SCHEMA <name>` statement in any captured migration names
it (pending migrations included), or when an `ALTER SCHEMA <old> RENAME TO <new>`
renames such a schema. The set comes from the captured SQL, before any database is
read, so `status`, `plan`, `verify`, `migrate` and `reconcile` inspect the same
schemas, and the plan's input digest binds them. Reports list them in
`target.history_schemas`. A `CREATE SCHEMA` inside a `DO` block or function body,
`CREATE SCHEMA AUTHORIZATION <role>` without a schema name, and the reserved names
`pg_*`, `information_schema`, `_ntnt` and `_ntnt_replay` never join the scope.
None of them can be the manifest's `schema` either: `_ntnt` and `_ntnt_replay`
are refused with `DB_CONFIG` naming them as NTNT's ledger and replay marker.

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
`ensures` and `adoption_checks` still name only managed-schema tables and views.
The check-purity rules ([Immutable SQL inventory](#immutable-sql-inventory)) cover
what they can read: the managed schema's relations and, through views, whatever
those read, in these schemas or any other. One disallowed object there fails
every check of the phase; an object no check can reach does not. Lock
predictions, `progress[].locks` and `CREATE INDEX CONCURRENTLY` cover managed-schema
tables only.

Statements outside the managed scope run as written; NTNT does not refuse them. A
migration may create, change, drop, grant, revoke or comment on schemas and objects
elsewhere, for example `REVOKE CREATE ON SCHEMA public FROM PUBLIC`, `DROP TABLE IF
EXISTS public.legacy` or `GRANT USAGE ON SCHEMA public` while the managed schema is
`app`, whether it names them, reaches them through `SET search_path`, a `DO` block or
a function, or uses SQL NTNT does not parse. NTNT does not inspect those schemas
apart from the objects the history creates there.
A plan flags each pending statement that it can tell may act there
([`analysis.outside_scope`](#what-postgresql-replay-runs)), and stays reviewable.
Replay runs the statements in scratch, which holds nothing but an empty `public`: a
statement that needs a schema or object the history does not create there (`CREATE
TABLE shared.t` while only the target has `shared`) fails in replay, and an `IF
EXISTS` form is a no-op in scratch while it acts on the target.

#### Objects the history creates in other schemas

An object a migration creates in a schema the history did not create, such as
`CREATE TABLE public.audit_log (...)` while the managed schema is `app`, is
inspected, and nothing else there is: other objects in that schema, such as another
application's tables, and the schema's own owner and privileges are never inspected
or reported. Replay finds these objects however a migration creates them: with a
qualified name, through `SET search_path`, in a `DO` block or function, or in SQL
NTNT cannot parse. Scratch holds nothing outside the managed scope but an empty
`public`, so whatever appears there during replay is the history's.

Their catalog keys lead with the schema: `public.relation:audit_log`,
`public.column:audit_log.msg`. A relation's columns, constraints, indexes, triggers,
policies and rules follow it, so an index added to `public.audit_log` out of band is
drift, like an out-of-band `DROP TABLE public.audit_log`, `ALTER TABLE ... ADD COLUMN`
or `GRANT` on it. The managed schema's coverage rules apply, and managed objects may
refer to these (a view over `public.audit_log`, foreign keys between them) as they may
to a created schema. A foreign key into one of them from a table the history does not
create, such as another application's table in `public`, blocks every migration with
`DB_EXTERNAL_DEPENDENCY`, as one into the managed scope does: a `DELETE` from
`public.audit_log` could otherwise cascade into rows that nothing inspects. A schema
the history creates without a top-level `CREATE SCHEMA`
(in a `DO` block, say) is one of these objects, `schema:<name>`, with whatever the
history creates in it. Members of an extension follow the extension, which stays
uncovered when it is installed outside the managed scope.

`status`, `verify` and `plan` first read the target in one read-only snapshot. When
replay then finds such objects, they read the whole catalog again in a second
snapshot that includes them; it must show the same target and history, or nothing is
compared (`DB_TARGET_CHANGED`). Reports list the objects in `target.outside_objects`,
and a plan records every object its replay found in `plan.execution.outside_objects`,
so `migrate` and `reconcile` inspect the same objects on the target before and after
each migration. A history that creates nothing outside the managed scope has neither
field and keeps its catalogs and plan digests. A plan saved before these objects were
inspected no longer matches; plan it again.

Two kinds of change stay refused with `DB_OUTSIDE_SCOPE`, because they would break
NTNT's own guarantees: changes to `_ntnt`, the schema that holds NTNT's ledger, and to
PostgreSQL's system schemas (`pg_catalog`, `information_schema`, `pg_toast` and other
`pg_*` names; `pg_temp`, the session's temporary schema, is allowed). Its `source`
names the migration and `objects` names what it would change:

- **Before the script runs**, in replay and on the target: a top-level statement that
  creates, alters, drops, truncates, grants or revokes on an object or schema named
  there, or moves an object there with `ALTER ... SET SCHEMA` (`DROP TABLE IF EXISTS
  _ntnt.schema_migrations_v2`, `GRANT USAGE ON SCHEMA pg_catalog`), a `SET
  search_path`, `SET SCHEMA` or `set_config('search_path', ...)` that puts `_ntnt` on
  the search path, and a `_ntnt.<name>` or `SCHEMA _ntnt` in a statement NTNT does not
  parse or does not read names from (`COMMENT ON TABLE _ntnt.schema_migrations_v2`),
  other than one that reads or writes rows, runs code or changes settings (`INSERT`,
  `DO`, `SET` and the like). The message gives the statement numbers; `objects` lists
  the names as written. Such a statement can be a no-op in scratch, which has no `_ntnt` and whose
  unprivileged role cannot change system schemas, yet act on the target.
- **After each replayed migration**: scratch must not have gained a schema `_ntnt`
  (`objects`: `schema:_ntnt`), as a `DO` block can create one, and NTNT's replay
  marker `_ntnt_replay` must be as replay committed it (`objects`:
  `schema:_ntnt_replay`). The same name checks as for `_ntnt` apply to `_ntnt_replay`
  before the script runs.

A `DO` block or function body that changes `_ntnt` only on the target (for example
behind an `IF` that finds the ledger) is not detected; keep such SQL out of migrations.
Database-wide objects without a schema, such as publications and large objects, remain
outside inspection.

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
only on `plan`, not status, verify, migrate or alongside `--against`. Each
subcommand's `--help` lists only the flags it takes; one it does not take (such as
`--adopt-through` on `status`, or `--against` on `migrate` and `reconcile`, which run
only the plan given with `--plan`) fails with `DB_CONFIG` naming that flag. The selected
ID must exist in captured source. Gaps in IDs remain valid. Supply the expected SQL
history/baseline yourself and review it; the tool does not dump the live target and
then declare that target to be its own reference. The whole selected prefix must
replay within the adapter's supported grammar in the same isolated scratch used
for ordinary planning. Only that prefix is replayed; future pending SQL is not
included in this adoption action. No old migration SQL executes on the target.

A refusal (`DB_ADOPTION_INPUT`) says which requirement failed, with advice for it:
no `[databases.<name>.adoption_checks]` declared; an `--adopt-through` ID that is
not in the migrations directory (`Migration ID 7 is not in migrations; the highest
ID there is 1.`); a target that already has an ntnt ledger, whose history needs
ordinary `plan` and `migrate`; or a legacy v1 ledger that is empty, records
migrations that are not applied, or ends at a different ID than the one requested.
A target that holds tables the selected migrations do not create, such as the
application's own migration ledger, is refused with `DB_DRIFT` instead; see
[Retiring an application's previous migration ledger](#retiring-an-applications-previous-migration-ledger).

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

For SQLite, adoption compares each object's stored `CREATE` text exactly. SQLite keeps a statement as it was written, so the adopted migrations must create every table, index, view and trigger with the text the file holds, formatting and quoting included; `DB_DRIFT` shows only redacted hashes of the definitions that differ. Write baseline migrations from the database's own `sqlite_master.sql` text, for example the output of `SELECT sql || ';' FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite\_%' ESCAPE '\' ORDER BY rowid`, which leaves out SQLite's own tables such as `sqlite_sequence`, rather than from a schema dump that reformats it.

### Retiring an application's previous migration ledger

An application that ran its own migrations usually recorded them in a ledger table
of its own in the managed schema, such as `public.schema_migrations` (PostgreSQL)
or `main.schema_migrations` (SQLite). No migration creates that table, so
`plan --adopt-through <last-id>` refuses the adoption with `DB_DRIFT`: the target
holds an object the history does not create, and such an object is never adopted.
The refusal's `next_action` names each table the history does not create and, in
case one is the old ledger, the steps below for the engine. `ntnt db` never moves
or drops the table itself. Retire the ledger by hand, outside `ntnt db`, then plan
the adoption again:

1. Stop the old migration runner, and remove it from deploy scripts, so it cannot
   recreate the table or apply a file that `ntnt db` now applies. This is the
   "retire old migration writers" step that
   [Recognized legacy PostgreSQL history](#recognized-legacy-postgresql-history)
   asks for.
2. PostgreSQL: move the table out of the managed schema, to a schema that `ntnt db`
   does not inspect. Its indexes, constraints and owned sequences move with it, and
   its rows are kept:

   ```sql
   CREATE SCHEMA IF NOT EXISTS app_legacy;
   ALTER TABLE public.schema_migrations SET SCHEMA app_legacy;
   ```

   `next_action` names `app_legacy`, or, when the managed schema or a schema the
   history creates has that name, the first of `app_legacy_2`, `app_legacy_3` and so
   on that none has: a table moved into the scope would still be found when the
   adoption is planned again. Or archive its rows elsewhere, then
   `DROP TABLE public.schema_migrations;`.
3. SQLite: a database file has no other schema that `ntnt db` leaves alone, so copy
   the table to another file, compare the copy with it, and only then drop it. Open
   the target with `sqlite3 -bail`, followed by its path, in the directory where the
   copy, `legacy-ledger.sqlite`, should be written. `-bail` stops the shell at the
   first error; without it, the shell runs the next statement anyway (in a shell
   that is already open, run `.bail on`). Use a maintenance window: stop other
   users of the target and use a private directory for the archive. Run all steps
   **in the same shell**, checking each result before proceeding.

   ```sql
   PRAGMA main.locking_mode = EXCLUSIVE;
   BEGIN EXCLUSIVE;
   COMMIT;
   ```

   The pragma must print `exclusive`, and both transaction statements must
   succeed. This acquires the target lock now and keeps it until the shell
   closes, including between the archive commit and the drop. If the database
   is busy or any step fails, stop and close the shell. After interruption,
   start again with a fresh archive; never resume at `DROP TABLE`.

   1. List the other tables whose foreign keys refer to the ledger:

      ```sql
      SELECT s.name FROM main.sqlite_schema AS s, pragma_foreign_key_list(s.name) AS f
      WHERE s.type = 'table' AND s.name <> 'schema_migrations' COLLATE NOCASE
        AND f."table" = 'schema_migrations' COLLATE NOCASE;
      ```

      It must list no table. If it lists any, close the shell before repairing them: with foreign-key enforcement
      on, dropping the ledger first deletes its rows, and that deletes or changes
      the rows that refer to them (`ON DELETE CASCADE`, `SET NULL`, `SET DEFAULT`)
      or makes the drop fail. Remove those foreign keys into the ledger with a
      reviewed migration first (SQLite removes a foreign key only by rebuilding
      its table), then restart from the lock step. Never turn foreign-key enforcement off to drop
      the ledger: that leaves their references dangling.

   2. Copy the table, and compare the copy with it:

      ```sql
      ATTACH DATABASE 'legacy-ledger.sqlite' AS legacy;
      CREATE TABLE legacy.schema_migrations AS SELECT * FROM main.schema_migrations;
      SELECT (SELECT count(*) FROM legacy.schema_migrations) = (SELECT count(*) FROM main.schema_migrations)
        AND NOT EXISTS (SELECT * FROM main.schema_migrations EXCEPT SELECT * FROM legacy.schema_migrations);
      ```

      The last query must print `1`: the copy holds as many rows as the ledger,
      and each of its rows. If a statement fails, for example because
      `legacy-ledger.sqlite` already holds a `schema_migrations` table from an
      earlier copy, the shell stops there and the ledger is untouched: move that
      file aside and start again.

   3. Only then drop the ledger in the same shell, then close the shell to release the lock:

      ```sql
      DROP TABLE main.schema_migrations;
      ```

Use the managed schema and table names that `next_action` gives. A table that is
not an old ledger is ordinary drift: reconcile it deliberately before adopting.

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
fail closed; retire old migration writers before adopting/importing a deployment,
and retire an application's own ledger as
[Retiring an application's previous migration ledger](#retiring-an-applications-previous-migration-ledger)
describes.

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

Files must have one hard link and no symlink path components. Filename components cannot contain `:` on any platform (a Windows drive prefix is allowed); NTFS alternate streams have independent contents but share file-level identity and are not supported. Use the native canonical path on macOS (for example `/private/...`, not its `/var` alias). Identity binds native device/inode or Windows volume/file-index plus available creation time, not merely path spelling. Plans also bind the bundled SQLite version, encoding, page size, auto-vacuum and journal mode, with Unix owner/group/mode or Windows readonly metadata. File paths themselves are not disclosed: `DB_FILE_IDENTITY` names the role and the secret, as the manifest names it, and the rule the path broke (`Scratch connection secret APP_REPLAY_FILE names a relative path; ...`): a relative path, a file that does not exist, a symbolic link in the path, not a regular file, more than one hard link, `:` in a file name, or a file replaced or moved while `ntnt db` used it. Its `next_action` says how to fix that rule. Replacement/copying changes identity and requires a new plan. The Unix adapter checks SQLite's open-file movement status without opening and closing a second descriptor that could drop SQLite's POSIX locks; Windows retains a non-delete-sharing guard. Filesystem administrators, local directories and OS ACL administration remain trusted. These checks are **not a filesystem sandbox or full ACL attestation**; use local storage with reliable SQLite locking, not network/shared filesystems. A target or scratch file SQLite cannot write (`SQLITE_READONLY` and its extended codes: a read-only file or directory, or a WAL shared-memory file it cannot write) fails with `DB_SQLITE`, which names the file's role and secret and why, never a statement: `The SQLite scratch file (secret APP_REPLAY_FILE) is not writable (SQLITE_READONLY): ...`. Its `next_action` says to make the file and its directory writable; for a target, plan and approve again afterwards, since the plan binds the file's permissions.

### Transactions, replay and contracts

Support is for the bundled **SQLite 3.54 series**, not an arbitrary separately installed `sqlite3` executable. `status`/`plan`/`verify` open the target readonly and use a deferred snapshot, releasing it before scratch replay. Catalog inspection reads metadata and compiles, but never steps, application programs with a native authorizer to validate their supported effect paths. It does not scan application rows. Replay inspects the scratch catalog after every migration. Each inspection reads every `sqlite_schema` row again, but reads metadata again only for objects whose definitions changed, and compiles a table's or view's programs again only when something they compile may have changed: their own definition, the objects their stored SQL names, the tables their triggers write, or the foreign-key children their actions reach. A change below a table in its foreign-key tree does not compile the table again when the children there check the same thing in their own programs and still compile, and the nesting stays within SQLite's limit, so a history that adds one table per migration to chains, DAGs or hubs of foreign keys compiles each table's programs once. Every inspection reports exactly what a full inspection would. SQLite does not compile a program whose triggers and foreign-key actions nest more than 1,000 levels deep (`SQLITE_LIMIT_TRIGGER_DEPTH`), so a table whose `DELETE` cascades through more than 1,000 `ON DELETE CASCADE` keys, or whose triggers nest that deep, is outside coverage (`DB_COVERAGE_INCOMPLETE` names it). A full inspection, which `status` and `verify` make of the target, compiles every table's programs, and SQLite's work for a chain of cascades grows with the square of its length: a chain of 1,000 tables takes about four seconds of the five-second operation deadline on a release build. Scratch uses a distinct physical file, separate secret and matching settings; `BEGIN IMMEDIATE` owns its rollback-only replay. An empty scratch takes the target's journal mode, because `std/db/sqlite` `connect()` puts application databases in WAL mode and a scratch made with `touch` is not. A scratch with no pages yet (0 bytes, as `touch` makes it) first takes the target's encoding, page size and auto-vacuum too. SQLite fixes the page size and auto-vacuum once the file has a page, and the encoding once it has held a table, so a difference SQLite can no longer change fails with `DB_SCRATCH_MISMATCH`, which names each setting with both values, for example `page_size: target 4096, scratch 1024`; its `next_action` says to replace the scratch with a new empty file. A nonempty scratch file is rejected, never cleared.

Execution opens an existing file only after approval and file-identity checks. It acquires **`BEGIN IMMEDIATE` before inspecting history/catalog for each migration**, revalidating on every acquisition; the writer reservation is not retained between commits. Competing migrators recognize each other's matching committed prefixes instead of applying SQL twice. SQLite's writer lock covers ordinary application writers; it is not an online-DDL guarantee. Dedicated connections enable foreign keys, recursive triggers and CHECK enforcement, `trusted_schema=OFF`, defensive mode, strict quoting, and `synchronous=FULL`. Durable rollback-journal modes and WAL are accepted; journal-off/memory modes are not. The runner does not change the selected persistent journal mode.

The two-second busy timeout, five-second worker/progress deadline, and shared orchestration/reconciliation deadlines bound operations. A target or scratch file that another connection keeps locked (an open write transaction, a reader that a rollback-journal commit must wait for, or `PRAGMA locking_mode=EXCLUSIVE`, whose connection keeps the lock until it closes) fails with `DB_BUSY` once the busy timeout runs out, or at once where SQLite does not wait (where waiting could deadlock, for example). `DB_BUSY` names the file's role and secret and says that nothing was committed; retry once the writer finishes. A lock conflict within ntnt's own connection (`SQLITE_LOCKED`) is also `DB_BUSY`, but names no other connection, since waiting for one cannot clear it. A migration script on the target runs under `statement_seconds` instead (90 seconds by default, see [time limits](#time-limits)); scratch replay keeps the five-second bound. SQLite interrupts an operation at its deadline, and a statement interrupted there fails with `DB_STATEMENT_LIMIT`, naming the statement and the limit. An operation that has not returned five seconds after its deadline is cancelled: its connection is discarded and never returned to a pool. Native I/O still depends on the OS/filesystem. A busy or interrupted COMMIT is reconciled using a fresh connection and writer reservation, without retrying SQL; if the fresh observation cannot establish the prefix, the result stays unknown. Process termination cannot return acknowledgement: rerun the same approved plan to inspect committed truth first. A hot rollback journal may prevent readonly inspection (`DB_SQLITE_RECOVERY_REQUIRED`); wait for an active writer to finish, or rerun `migrate` with that same approved plan so native recovery and locked reconciliation occur through the authorized path. Do not delete the journal or silently make inspection writable.

SQL, named transition checks, the resulting catalog and the success ledger row are one transaction. SQLite contracts use the same closed SELECT expression grammar described above, with every relation qualified as `main.<table>`; the native authorizer additionally denies side-effecting calls and nonmanaged relations. Only **one integer `1`**, in one row/column, passes. Numeric values other than `1`, floating-point `1.0`, text, NULL, incorrect shape, errors and timeouts fail. Historical checks remain `not_run` on verify/rerun.

### Supported SQL and catalog coverage

The SQLite dialect parser accepts ordinary table/index/view/trigger DDL, INSERT/UPDATE/DELETE (also when a `WITH` clause leads them) and `ANALYZE`, while the native authorizer mediates resolved reads, writes and calls. Native SQLite statement boundaries preserve trigger bodies and quoted semicolons; scripts are not split with string operations. Tables, columns/order/type/defaults, PK/UNIQUE/CHECK/FK definitions, indexes (column, expression and partial, unique or not) and builtin collations (`BINARY`, `NOCASE`, `RTRIM`) are fingerprinted. WITHOUT ROWID and STRICT flags are retained. Supported views and table triggers have definition hashes and native compile checks, **not inferred business semantics or complete transitive-effect explanations**. SQL definitions/defaults remain redacted hashes; final catalogs are engine-native, not normalized into PostgreSQL definitions.

An index's expressions and partial-index `WHERE` predicate are part of its definition hash, so changing one out of band, for example dropping and recreating the index with another predicate, is drift on that index. SQLite resolves these expressions, like CHECK constraints, when it loads the schema, without consulting the authorizer, so inspection cannot re-check the functions they call. The function policy below applies when a migration's `CREATE INDEX` is compiled, in scratch replay and on the target; inspection then requires the target's index to match the replayed definition. SQLite itself allows only deterministic functions of the indexed table's own columns there, with no subqueries. Inspection still compiles the table's INSERT/UPDATE/DELETE programs, which evaluate them.

Only concrete managed relations are eligible. `CREATE TABLE ... AS SELECT` is rejected: native authorization of a pending table name must not authorize a same-named virtual-table source. Express copies as ordinary `CREATE TABLE` followed by `INSERT ... SELECT`, with resolved relations refreshed between native statements. Eponymous virtual tables such as `dbstat` or `pragma_*` cannot masquerade as ordinary `main` tables; the JSON table-valued functions `json_each`, `json_tree`, `jsonb_each` and `jsonb_tree`, which read only their arguments, can be read. A read that uses no column of what it names (`INSERT INTO stats SELECT count(*) FROM events`, `EXISTS (SELECT 1 FROM events)`) reaches the authorizer without a database name. It is allowed when the name is a table or view of `main` (ntnt attaches no database and refuses temporary objects, so no other schema holds one) or a common table expression of the statement, and refused for ntnt's ledger, SQLite's internal tables and the other eponymous virtual tables. SQLite reports a read of a common table expression the same way, by its name alone, and a name can mean a common table expression in one part of a statement and the table in another, so a common table expression named like a refused table (`_ntnt...`, `sqlite_...`, `dbstat`, `fts3tokenize`, `fts4aux` or `pragma_...`) is refused too; give it another name. Common table expressions, `WITH RECURSIVE` included, can lead an INSERT, UPDATE or DELETE or appear inside one.

Builtin `count` is available to checks. Migration and definition programs can call every built-in function of the bundled SQLite 3.54 except the two that SQLite itself marks direct-only: `load_extension`, which loads code, and `sqlite_log`, which writes to the process's log. That is the core scalar functions (`abs`, `changes`, `char`, `coalesce`, `concat`, `concat_ws`, `format`, `glob`, `hex`, `if`, `ifnull`, `iif`, `instr`, `last_insert_rowid`, `length`, `like`, `likelihood`, `likely`, `lower`, `ltrim`, `max`, `min`, `nullif`, `octet_length`, `printf`, `quote`, `random`, `randomblob`, `replace`, `round`, `rtrim`, `sign`, `soundex`, `sqlite_compileoption_get`, `sqlite_compileoption_used`, `sqlite_source_id`, `sqlite_version`, `substr`, `substring`, `subtype`, `total_changes`, `trim`, `typeof`, `unhex`, `unicode`, `unistr`, `unistr_quote`, `unlikely`, `upper`, `zeroblob`), the aggregate functions (`avg`, `count`, `group_concat`, `max`, `min`, `string_agg`, `sum`, `total`), the window functions (`cume_dist`, `dense_rank`, `first_value`, `lag`, `last_value`, `lead`, `nth_value`, `ntile`, `percent_rank`, `rank`, `row_number`), the date and time functions (`date`, `time`, `datetime`, `julianday`, `unixepoch`, `strftime`, `timediff`, `CURRENT_DATE`, `CURRENT_TIME`, `CURRENT_TIMESTAMP`) and the JSON functions (`json`, `jsonb`, every `json_*` and `jsonb_*` function, and the `->` and `->>` operators), plus the internal helpers SQLite's own ALTER TABLE statements call. The bundled build has no math functions such as `sqrt` or `ln` (SQLite's `SQLITE_ENABLE_MATH_FUNCTIONS` is off), so no migration can call them. Functions that SQLite's FTS3, FTS5 and R*Tree extensions register (`fts3_tokenizer`, `fts5_source_id`, `snippet`, `bm25`, `rtreenode`, ...) are not built in, and neither is any application function; both stay refused. Defaults such as `DEFAULT CURRENT_TIMESTAMP`, `DEFAULT (datetime('now'))` and `DEFAULT (lower(hex(randomblob(16))))` work in `CREATE TABLE`, and in `ALTER TABLE ... ADD COLUMN` only on a table without rows (on a table with rows SQLite accepts only a constant default there, see below), and the catalog stays deterministic: SQLite stores a default as its expression. SQLite itself refuses a non-deterministic call where determinism is required (generated columns, index expressions and partial-index predicates), and, as ntnt's connections set `trusted_schema=OFF`, a direct-only function in a trigger or view body.

SQLite checks a call when it compiles the statement that makes it, including the body of a trigger that statement fires or a view it reads. A migration statement that calls anything else that way fails with `DB_SQL_UNSUPPORTED`, naming the statement number and what SQLite refused, such as a function (``Statement 3 uses SQLite function `load_extension` ``), a table (``Statement 2 uses table `dbstat` ``) or a temporary object; its `next_action` says what a SQLite migration can use. A function SQLite has no definition of, such as one an application registers on its own connections, is named the same way in any statement, as in a DEFAULT (``Statement 2 uses SQLite function `app_slugify` ``). A trigger or view body that no statement in the migration uses is compiled only during inspection, so a call in it is reported as `DB_COVERAGE_INCOMPLETE`, with the trigger's table or the view in `objects`. Before replay, a parser gate reads each statement, numbered by SQLite's own statement boundaries. A statement it refuses fails with `DB_REPLAY_UNSUPPORTED`, which names the statement and why: a syntax error, with the keyword, identifier or symbol it stopped near (never a literal), as in ``Statement 2 has a syntax error near `CREAT`.``; the 4,096-token limit for one migration file, whitespace and comments included; an authored PRAGMA, by name (migrations cannot run PRAGMAs); or a statement outside the supported subset, by its leading keywords (``Statement 2 (`CREATE VIRTUAL TABLE`) is outside the supported SQLite subset.``). Its `next_action` says how to fix that cause. Any other failing statement is numbered too, with SQLite's error and the name of its extended result code, such as `Statement 3: UNIQUE constraint failed: accounts.email (SQLITE_CONSTRAINT_UNIQUE).` SQLite's own message is shown only where it holds names alone: a missing or duplicate table, column or other object, which UNIQUE, NOT NULL, FOREIGN KEY or named CHECK constraint failed, or a RAISE message that the trigger's SQL spells out as a literal, with ABORT, FAIL or ROLLBACK (including a ROLLBACK that undoes the migration that created the trigger). Any other message, which could hold a value from the failing row, is reduced to the code's name. In replay the `next_action` says to fix that statement; in `migrate` it says that nothing from the migration was committed, and how to continue. A known function name is not permission to load extensions or register application functions.

SQLite's state tables are not schema. `sqlite_sequence`, which holds the AUTOINCREMENT counters, and the planner statistics `sqlite_stat1` and `sqlite_stat4` (and the legacy `sqlite_stat2` and `sqlite_stat3`), which `ANALYZE` and `PRAGMA optimize` write, are left out of coverage and drift, as PostgreSQL's sequence counters and statistics are. An AUTOINCREMENT table is covered like any other table, and its definition hash includes the AUTOINCREMENT keyword. SQLite creates these tables itself and updates them when a statement renames or drops a table or index they describe. A migration may read and write them, for example `UPDATE sqlite_sequence SET seq = 1000 WHERE name = 'orders'` to set where a counter continues, and may drop the statistics tables. A migration may also run `ANALYZE`: it reads the tables of `main`, leaving out ntnt's ledger, and writes only the statistics, so the replayed and target catalogs stay comparable whatever the statistics hold. The application's own `ANALYZE` or `PRAGMA optimize` changes nothing that `status`, `verify` or `plan` compares.

SQLite runs statements of its own for ALTER TABLE, under the same authorizer: the rename helpers and, for `ADD COLUMN` on a STRICT table or with a CHECK constraint, a re-check of the table's existing rows with `glob` and `pragma_quick_check`. Replay starts from an empty database, so the only rows it meets are those the replayed history inserts: when an earlier migration inserted rows that the new column would break, `plan` fails the same way, and the migration must change. Rows added outside the replayed history, such as the application's, meet the new column only in `migrate`. It then fails with `DB_SQLITE`, naming the failure (``Statement 1: CHECK constraint failed (SQLITE_ERROR).``, or `type mismatch on DEFAULT` for a STRICT column), and nothing from the migration is committed.

SQLite's `ADD COLUMN` has rules of its own, and its errors are shown with a `next_action` for each. It cannot add a UNIQUE or PRIMARY KEY column: `ALTER TABLE t ADD COLUMN c TEXT UNIQUE` fails in replay with `Statement 1: Cannot add a UNIQUE column (SQLITE_ERROR).`, so add the column without UNIQUE and then create a unique index on it (`CREATE UNIQUE INDEX t_c_key ON t (c);`), which enforces the same rule; a new PRIMARY KEY needs a rebuild. On a table that has rows, `ADD COLUMN` gives every row the column's default, so the default must be a constant (a literal or NULL, not `CURRENT_TIMESTAMP`, `datetime('now')` or `randomblob()`), and a NOT NULL column needs one. Replay's tables hold only the rows the history inserted, so unless an earlier migration inserted rows into the table, such a statement plans clean and fails only in `migrate`, on rows added outside the replayed history, with `Cannot add a column with non-constant default` or `Cannot add a NOT NULL column with default value NULL`: add the column with a constant default and set the rows with an `UPDATE` in the same migration, or rebuild the table with the [sidecar](#explicit-one-table-rebuild) to keep the expression as the default for new rows.

Virtual/shadow tables, generated columns, `INSTEAD OF` triggers, custom collations/functions, unsafe/unsupported view or trigger programs, other internal objects, bind parameters, ATTACH/DETACH, authored PRAGMAs, transaction/savepoint/session control and extension loading are unsupported. Triggers are covered only on tables: an `INSTEAD OF` trigger, which SQLite allows only on a view, is outside coverage (`DB_COVERAGE_INCOMPLETE`). Unsupported catalog coverage blocks a clean result. DML and trigger effects remain review-required (`SQLITE_EFFECTS_NOT_INFERRED`); metadata never turns an unknown effect into automatic safety. Large transformations belong in separately operated application jobs.

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
- keeps the rebuilt table's AUTOINCREMENT counter: if `sqlite_sequence` held N for it before, it must hold at least N after, in one row, unless the rebuilt table no longer declares AUTOINCREMENT (SQLite reads and updates only the first row for a name, so a second row is refused);
- restores foreign-key enforcement, including on failure; failure rolls back or discards the session.

Explicitly recreate the selected table's named indexes/triggers and any views dropped to allow the rebuild. The selected table's columns/constraints and autoindexes may change. Definitions are compared by the SQL text SQLite stores (`sqlite_master.sql`), so a recreated index, trigger or view must repeat its original `CREATE` statement exactly, whitespace and quoting included; equivalent SQL written differently is rejected. Copy it from the target, for example `SELECT sql FROM sqlite_master WHERE name = 'index_posts_on_user_id'`. A rebuild that breaks a rule fails with `DB_REBUILD_GUARD`, which names each rule and object, such as ``the row count of table `log` changed from 0 to 1``, ``index `index_posts_on_user_id` has another stored SQL text than before the rebuild`` or ``view `titles` is missing``, and lists their catalog keys in `objects`; row values are never shown.

Copying the rows of an AUTOINCREMENT table into its replacement starts the replacement's counter at the highest id copied, so SQLite would give new rows the ids of deleted rows above it. Copy the counter before the drop:

```sql
CREATE TABLE orders_new (id INTEGER PRIMARY KEY AUTOINCREMENT, ref TEXT NOT NULL CHECK (length(ref) > 0));
INSERT INTO orders_new (id, ref) SELECT id, ref FROM orders;
UPDATE sqlite_sequence SET seq = (SELECT max(seq) FROM sqlite_sequence WHERE name IN ('orders', 'orders_new')) WHERE name='orders_new';
DELETE FROM sqlite_sequence WHERE name='orders_new' AND rowid <> (SELECT min(rowid) FROM sqlite_sequence WHERE name='orders_new');
INSERT INTO sqlite_sequence (name, seq) SELECT 'orders_new', max(seq) FROM sqlite_sequence WHERE name='orders' AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name='orders_new') HAVING count(*) > 0;
DROP TABLE orders;
ALTER TABLE orders_new RENAME TO orders;
```

Any statement that writes `orders_new` gives it a counter row, even an `INSERT ... SELECT` that copies no row (the counter is then 0), each `INSERT INTO sqlite_sequence` for it adds another, and a replacement that nothing wrote has none. The `UPDATE` raises every row `orders_new` has to the highest counter of the two tables (copied ids can be higher than the counter of `orders`, for example when the rebuild renumbers them), the `DELETE` keeps only the first of those rows, the one SQLite reads, and the `INSERT` adds the counter of `orders` only when `orders_new` has none. So `orders_new` keeps one row, never a lower counter than either table had, whether it had no row, one or more. SQLite reads and updates only the first row for a name, so the guard refuses a rebuilt table that has more than one (``table `orders` has 2 AUTOINCREMENT counters in sqlite_sequence``), and without the three lines it refuses the rebuild with both values (``the AUTOINCREMENT counter of table `orders` went back from 3 to 2``); the `next_action` of both refusals gives them. Deliberate dependent-object retirement belongs in a separate reviewed migration, not an accidental omission inside this mode. The mode scans row counts **during authorized execution**, not during production planning.

Without this sidecar, a migration that drops a table which other tables reference with `ON DELETE CASCADE`, `SET NULL` or `SET DEFAULT` is refused with `DB_REBUILD_REQUIRED` before the drop runs, naming each child table and its action. Statements before the drop may already have run; the migration is rolled back, so none of their changes are committed. SQLite deletes every row of a dropped table first, so wherever the table has rows (on the target, or in replay when the history inserted them) the drop would run those actions; the refusal comes before the drop runs, whether or not there are rows. A drop that runs no such action stays allowed: one whose children were dropped first, one whose child foreign keys have no `ON DELETE` action, and a table's reference to itself. Allowed does not mean it works on a target with rows. SQLite still deletes the dropped table's rows first, so when other tables reference them through a foreign key without an `ON DELETE` action, the drop fails there (``Statement 3: FOREIGN KEY constraint failed``), or, for a `DEFERRABLE INITIALLY DEFERRED` key such as Django's, the COMMIT does (``SQLite could not commit the migration: FOREIGN KEY constraint failed``), even though the replacement now has the old name and the same rows. Replay meets only the rows the replayed history inserts, so it shows this failure only when an earlier migration inserted rows on both sides of the key; its `next_action` then says that replay found it and that the migration must change. Rows added outside the replayed history, such as the application's, meet it only on the target, so a migration that rebuilds a table other tables reference needs the sidecar on a populated target; the `next_action` of both target failures says so.

The sidecar is for a migration that rebuilds the table: it creates the replacement and copies the rows. A migration that retires the table cannot use it, because the guard requires the same set of tables. To retire a table, first remove the foreign keys that reference it from the tables the refusal names, or drop those tables, earlier in the migration or in an earlier one. SQLite removes a foreign key only by rebuilding its table: create the table again without the key, copy its rows, drop the old table and rename the new one. For example, to retire `accounts` while keeping `posts` and dropping `comments`:

```sql
CREATE TABLE posts_new(id INTEGER PRIMARY KEY, account_id INTEGER NOT NULL);
INSERT INTO posts_new SELECT id, account_id FROM posts;
DROP TABLE posts;
ALTER TABLE posts_new RENAME TO posts;
DROP TABLE comments;
DROP TABLE accounts;
```

Counts and FK checks do not prove row identities, correct value mappings, every uniqueness assumption, old-client compatibility or recovery after a committed destructive change. Use named transition checks and the same executor in representative native upgrade tests to assert exact keys/values, preserved children and working dependencies. `tests/db_sqlite.rs` demonstrates an email-lowercasing rebuild with existing rows, indexes, views, audit trigger and cascading foreign keys, plus negative dependency/row-loss/orphan cases. Empty replay alone cannot provide that evidence. Unknown business effects remain review-required even when these guards pass.

## Results and plan identity

JSON has `format_version: 1`, UTC `observed_at`, stable diagnostic codes, source filenames/object keys and next actions. `pass`, `fail`, `unknown`, and `not_run` are distinct. Exit code 0 means the requested inspection/replay result passed within its declared coverage; other outcomes return nonzero. `status`, `plan` and `verify` report **`not_run`, zero executed** assertions; `migrate` separately counts the contracts actually attempted in that invocation. A catalog/history pass is not evidence of SQL contracts, data correctness, business invariants or deployed-client compatibility.

Plans bind:

- logical database and target names, captured manifest, ordered source/checksum identities and policy;
- native cluster/database or file identity, engine and settings, and adapter-specific role/authority facts;
- managed scope, observed base history/catalog, applied prefix and replayed expected base;
- adapter/canonicalization version, exact pending migration identities and candidate fingerprint;
- explicit `execution.format = "postgresql-transactional-v1"` or `"sqlite-transactional-v1"` and ordered engine-specific catalog fingerprints for every migration prefix, including the empty prefix. Old/incomplete artifacts cannot authorize execution;
- the [time limits](#time-limits) `migrate` will use (`execution.limits`): PostgreSQL's effective limits, and SQLite's `statement_seconds` when the manifest declares a limits table.

`plan_digest` hashes that descriptor. Observation time and display filters are not authority inputs. `--against` checks the saved descriptor's digest, pending descriptors and fresh source/target/base binding. It neither executes nor approves the plan.

A plan that no longer matches fails with `DB_PLAN_STALE`, for `migrate`, `reconcile`
and `--against` alike, and the message names the binding that changed:

- the selection, checked before any I/O: `review.json was planned for database app,
  target development; this run selected database app, target staging.`;
- an input: the pending migration that changed (with both checksums), was added or
  was removed; a migration added below the ones the plan treats as applied (named
  when the plan's ledger tells it from them, with how to run the plan without it or
  renumber it, also when the plan has nothing pending) or an applied one removed
  (`status` names it and how to restore it); other changes below them, such as one
  added and one removed, which `status` names one at a time before you plan again;
  or the manifest itself, also on an adopted target;
- the policy: the engine, checked first because limits differ by engine; the
  limits, as in `[databases.app.limits] in ntnt.toml changed after the plan was
  made: statement_seconds: plan 90, now 600.`; the schema; or another part of the
  database's configuration;
- the target: each identity field that differs, such as `target setting journal_mode:
  plan delete, now wal` when the application opened a new SQLite file between `plan`
  and `migrate`, or a PostgreSQL session setting or transport;
- its history (the ledger covers other migrations than when the plan was made) or its
  catalog (the schema changed outside migrations).

Its `next_action` says how to get a plan that runs. A saved file that is not an
intact plan (edited by hand, a forged digest) is refused as invalid instead, without
naming a target, and a blocked plan says its outcome was not `pass`: `Saved plan is
blocked: its outcome is fail, ...`. That includes a file `plan --out` saved when `plan`
stopped before it could make a plan, which has no plan and no `plan_digest` (so
`jq -r .plan_digest` prints `null`); check the outcome before approving, as the
[quick start](#quick-start) does. A blocked plan keeps the code `DB_PLAN_STALE`, like
every other saved plan `migrate` will not run, so automation that handles one handles
all of them; `DB_PLAN_INVALID` stays reserved for input that is not JSON. An unreadable
`--against` file is reported as `DB_INPUT_IO` naming the file and the cause, and one
that is not JSON as `DB_PLAN_INVALID`.

For `plan --out`, the file contains the compact plan descriptor and result metadata, not duplicate catalog/source display fields; stdout still contains the full report. `status`/`verify --out` save their report. All saved artifacts share the reader's 1 MiB bound: oversized output fails before creating a file rather than producing an unreadable plan. Files are new-only (`--out` cannot replace a source, existing plan or symlink); on Unix, new reports are mode 0600. `--out` is checked before any work runs: a file that already exists, a directory that does not, or a path through a file that is not a directory (`review.json/sub/report.json`), fails with `DB_OUTPUT` naming the path and the cause (`Cannot save the plan to review.json: it already exists, and --out never replaces a file.`), and its `next_action` suggests a free name such as `review-0002.json`. A save that fails later names the operating system's error and removes the incomplete file it created, so the filename can be reused; the human report then shows no plan digest.

A late `DB_OUTPUT` from `migrate --out` means the report could not be saved, not that the database rolled back. Check `progress` on stdout and run `ntnt db status` with the same manifest, database and target before deciding whether to retry. Do not edit the ledger to recreate a missing report.

Every saved plan has `authorized = false`, `executable = false`, `automatic_safe = false` (the artifact alone grants no capability), and a `requires_review` or `blocked` classification. Known drop/constraint-removal/cast/CASCADE/DML/authorization hazards are reported conservatively. Lock/scan labels are qualitative, not production duration estimates. Missing data checks and still-deployed application compatibility remain explicit. `migrate` validates the execution format and all prefix evidence under its own lock and requires independent provider authority; the digest alone is not an approval token.

## Bounds, cleanup and concurrency

Connect/authentication and every query/cleanup exchange have 5-second client deadlines; PostgreSQL statement and lock limits are 5 and 2 seconds (scratch cleanup's `DROP OWNED` may run 10; see [How replay commits and cleans up scratch](#how-replay-commits-and-cleans-up-scratch)), with a 10-second idle transaction timeout. Migration scripts, concurrent index statements and the wait for another migrator have their own [time limits](#time-limits). Database orchestration has a 120-second outer deadline by default; migration work reserves the final 20 seconds (more if `migrator_wait_seconds` is raised) for fresh reconciliation if a work deadline interrupts COMMIT. If reconciliation itself cannot finish, progress remains unknown rather than guessing. Existing secrets providers retain their own bounded lookup/retry policy. Dedicated connection tasks are discarded on cancellation/error; sessions are never recycled into application pools. Runtime shutdown waits at most 100 milliseconds for uncancellable OS-resolver work, so cancelling a connect is not followed by an unbounded runtime join. Scratch rollback is checked, and cleanup failure cannot produce a verified candidate.

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
| `migrator_wait_seconds` | 5 | 1–300 | PostgreSQL only: how long `migrate` and `reconcile` wait for another migrator's advisory lock before failing with `DB_LOCK_CONFLICT`, and how long `status`, `plan` and `verify` wait for another replay of the same scratch database before failing with `DB_SCRATCH_BUSY`. |

Omitted fields keep their defaults; without the table nothing changes. Unknown fields, non-integers and values outside the range fail with `DB_CONFIG`, naming the field. The 2-second minimum for `statement_seconds` keeps it above the 1-second migration `lock_timeout`. PostgreSQL counts the statement limit from the start of the statement, so at 1 second a statement waiting for a brief lock would reach the statement limit first and fail without a [retry](#exact-plan-authority-and-execution). The upper bounds are deliberate: a statement that needs more than a day belongs in a supervised manual build, and a migrator that has held the lock for five minutes is usually stuck or running a long build, so a deployment should fail and retry rather than queue behind it. On SQLite the table takes only `statement_seconds`; `migrator_wait_seconds` fails with `DB_CONFIG`. Scratch replay and SQLite's other operations keep the fixed bounds described under [SQLite](#transactions-replay-and-contracts). On SQLite, a statement past its limit fails with `DB_STATEMENT_LIMIT`, naming the statement and the limit. On PostgreSQL the server cancels the statement, and the migration fails with `DB_POSTGRES` and SQLSTATE 57014.

`migrate` and `reconcile` size their deadlines so that one full migrator wait and one full statement fit:

- the work deadline is `statement_seconds + migrator_wait_seconds + 5` seconds;
- the command deadline adds `migrator_wait_seconds + 15` seconds, so fresh reconciliation after an interrupted COMMIT can still wait for the migrator lock.

A limit set below its default counts at its default here, so lowering a limit never shortens these deadlines below 100 and 120 seconds. A run that reaches its work deadline keeps its committed progress; rerun the same plan to continue (after `ntnt db reconcile` if it interrupted a concurrent index build). The 5-second catalog exchanges, the 1-second migration `lock_timeout` and its retries are not configurable.

`status`, `plan` and `verify` keep the default limit and the 120-second deadline (on PostgreSQL, plus `migrator_wait_seconds` for the replay lock, and then 30 seconds for scratch cleanup, which runs even when replay reached its deadline): they replay on an empty scratch database, where index builds and table rewrites are fast. A script that is slow even on empty tables (for example, one that generates rows) must still replay within 90 seconds.

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

PostgreSQL fixtures create/drop databases and roles. They test schemas the history creates (drift, coverage and grants there, cross-schema foreign keys, renames, `DROP SCHEMA`, adoption and reconciliation), statements outside the managed scope that run as written and are flagged in plans, objects the history creates in other schemas (inspected however they are created, while unrelated objects there are ignored), the `DB_OUTSIDE_SCOPE` refusals that protect `_ntnt`, `_ntnt_replay` and system schemas, per-migration commits on scratch (enum values added in one migration and used in the next, deferred-constraint violations at plan), scratch emptied after every plan whatever its outcome with `public` restored, refused changes that outlive scratch's objects, cleanup of an unfinished replay only with its marker, and plans serialized by the replay lock, single-schema fingerprints recorded before the scope was added, real read-only transactions, no target bootstrap, applied-prefix versus pending drift, invalid indexes, constraint validity/grants, immutable SQL/metadata history, malformed/RLS/view ledgers, unknown coverage, physical scratch collisions, low-privilege/empty/matching scratch, redaction, bounded waits, cleanup, source replacement during a blocked inspection, and target/source/authority-bound plans. Execution tests additionally cover fresh construction, populated upgrade preservation, exact authority, stale plans, partial commits and reruns, false/NULL/shape/type/error/timeout checks, native/advisory contention, configured statement and migrator-wait limits (scaled to seconds), snapshots after lock acquisition, process cancellation, unsafe/scoped ledgers, unsupported effects and source replacement. A test-only wire proxy drops connections before COMMIT, after durable COMMIT but before acknowledgement, during rollback, and during reconciliation. It is not linked into release code and exposes no authorization bypass. These executed fixture checks are not evidence about arbitrary application business behavior or real deployment compatibility.
