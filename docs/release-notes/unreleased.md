# Unreleased

## `ntnt db` on PostgreSQL

- **Enums, domains and composite types.** A history that uses
  `CREATE TYPE ... AS ENUM`, `CREATE DOMAIN` or a composite type plans,
  migrates and verifies. Each type is compared by its definition (an enum's
  labels in order; a domain's base type, `NOT NULL`, default, collation and
  constraints; a composite type's attributes), in the managed schema and in
  schemas the history creates, and columns of these types are covered. An
  out-of-band `ALTER TYPE ... ADD VALUE` or a changed domain constraint is
  drift on the type. Any of these used to leave every plan `unknown` with
  `DB_COVERAGE_INCOMPLETE` (#430).
- **Trusted extensions in the managed schema.** An extension such as pg_trgm,
  citext, hstore or btree_gist installed in the managed schema is compared by
  its name and version, which cover what it installs: types, functions,
  aggregates, operators, operator classes and families, and casts. A
  `gin_trgm_ops` index or a `citext` column plans and migrates, and citext's
  implicit casts no longer stop every later migration. An out-of-band
  `ALTER EXTENSION ... UPDATE` is drift on the extension. Any other custom
  implicit or assignment cast still stops the next migration, now with
  `DB_CHECK_UNSUPPORTED` naming the cast and the migration (#430).
- **Operators and operator classes.** An operator's catalog key carries its
  argument types (`other:operator.===(integer,integer)`) and an operator
  class's its access method (`other:operator_class.citext_ops USING btree`), so
  overloads, and citext's btree and hash `citext_ops`, no longer collide. A
  user-defined operator whose function is covered is covered. When two objects
  still share a key, `DB_CATALOG_AMBIGUOUS` names it and the migration that
  made them (#430).
- **`EXCLUDE` constraints** plan, migrate and verify, with built-in operator
  classes or an extension's such as btree_gist (#430).
- **Checks on these tables.** `requires`, `ensures` and adoption checks may
  read tables with columns of these types, and run while an extension's casts
  exist. Adopting a history that uses an enum or citext column used to fail at
  `migrate` with `DB_CHECK_UNSUPPORTED`. Checks still run no routine the
  history wrote. Comparing a column with a literal converts the literal to the
  column's type; for a domain that is its base type, so a domain's own
  constraints never run, whatever they call. The elements of an array and the
  attributes of a composite go through their types' input, though, so a check
  that can read `emails app.email[]`, where `app.email`'s constraint calls a
  function the history wrote or a volatile one such as `nextval`, fails with
  `DB_CHECK_UNSUPPORTED` before it runs, naming the table, column, type,
  domain and function. So does a view that converts values to such a domain,
  or that uses an operator whose function, or whose negator's, the history
  wrote, and a view that calls `pg_input_is_valid` or `json_populate_record`
  while such a domain exists. A domain whose
  constraint uses only builtins, such as a regular expression, is allowed at
  any depth (#430).
- **Checks compare as the application does.** Checks resolve operators with
  `search_path=pg_catalog`, so `email = 'admin@example.com'` on a `citext`
  column compared case-sensitive text, and a check that the application's
  comparison would fail could pass. A check that compares a column (or a table's whole
  row) whose type, builtin or not, or a domain's base type, has a
  same-named operator outside `pg_catalog`,
  in any schema, since NTNT cannot know the application's search path,
  is now refused before any check runs, with `DB_CHECK_UNSUPPORTED` naming
  the check, table, column, type and operator (`app.=(app.citext,app.citext)`),
  and `plan` lists it in `plan.check_warnings`. Compare in a view a migration
  creates, which keeps citext's operator, and check the view. Enums, `citext[]`
  arrays and citext columns read with `IS NULL` or `count(*)` stay allowed
  (#456).
- **Checks never write through a view.** A check that reads a view, directly
  or through other views, that calls `nextval`, `setval`, an advisory-lock
  function or another builtin PostgreSQL marks parallel unsafe is refused
  before it runs, naming the check, view and function. `random()`,
  `clock_timestamp()` and `gen_random_uuid()` stay allowed (#456).
- **Enum values used too early are named.** A statement that uses an enum
  value that `ALTER TYPE ... ADD VALUE` added in the same transaction fails
  with `DB_ENUM_VALUE_UNCOMMITTED`, naming the migration and statements, in
  replay and in `migrate`, instead of an opaque SQLSTATE 55P04. Put
  `ALTER TYPE ... ADD VALUE` in its own migration and use the value in a later
  one (#456).
- **Checks on tables with row-level security.** Checks read with
  `row_security=off`, so that no policy can silently filter them, and PostgreSQL
  refuses such a read where row-level security applies to the migration role:
  a table with `FORCE ROW LEVEL SECURITY`, for the unprivileged owner role the
  migration guide uses. `plan` now lists each such `requires`, `ensures` or
  adoption check in `plan.check_warnings` (`CHECK_ROW_SECURITY`), naming the
  check and the table, and `migrate` refuses it with `DB_CHECK_ROW_SECURITY`
  before it runs, saying that the migration role needs `BYPASSRLS`. Such a
  migration used to plan cleanly and then fail with a bare SQLSTATE 42501.
  Plans without such checks keep their digests (#430).
- **Untrusted extensions.** A migration that creates an untrusted extension,
  such as `postgres_fdw`, fails with `DB_EXTENSION_PRIVILEGE`, naming the
  statement and the extension, explaining that roles that are not superusers
  can create only trusted extensions, and saying what to do. It used to fail
  with a bare SQLSTATE 42501.
- **Extensions a superuser installs.** `[databases.<name>] extensions =
  ["postgres_fdw", "pg_stat_statements"]` declares extensions that a superuser
  installs in both the target and the scratch database, at one version and in
  one schema, which may be `public`. Inspection compares each by its version and
  schema, so a mismatch between the two databases is drift; scratch may hold the
  extension and what it owns, and one missing from scratch is refused by name;
  migrations may keep `CREATE EXTENSION IF NOT EXISTS`, which then does nothing.
  An undeclared extension outside the managed scope, such as a DBA's
  `pg_stat_statements` in `public`, still leaves the catalog uncovered, and the
  diagnostic now says to declare it. The key is part of the plan only when
  present (#430).

- **Replay commits each migration on scratch, as `migrate` does on the target.**
  Replay used to run the whole history in one rollback-only transaction. A
  history whose migration adds an enum value with `ALTER TYPE ... ADD VALUE`
  (as Prisma generates) and whose next migration uses it could never plan on
  PostgreSQL 16 (SQLSTATE 55P04); it now plans, migrates and verifies on 16,
  17 and 18. A migration that adds a value and uses it in the same script
  used to plan on 17 and 18 and then fail at `migrate`; it now fails at plan
  with `DB_ENUM_VALUE_UNCOMMITTED`. Each migration runs in a `READ COMMITTED`
  transaction with the target's settings and commits (#430).
- **Scratch is cleaned up, fail-safe.** Replay commits a marker (schema
  `_ntnt_replay`) before the first migration and, whatever the outcome, drops
  what the scratch role owns with `DROP OWNED`, restores `public`'s owner,
  privileges and comment, and checks scratch is empty again. A plan after one
  that was killed between migrations cleans up its leftovers when only the
  scratch role owns them, and reports `scratch.recovered`; without the marker,
  or with another role's objects, scratch is refused with
  `DB_SCRATCH_NOT_EMPTY` as before and left untouched. A failed cleanup is
  `DB_SCRATCH_CLEANUP` (#430).
- **Recovery drops only what it can prove the unfinished replay committed.**
  The marker is bound to the database's OID and the cluster's system
  identifier, and records each transaction of the replay that wrote the
  catalog, in that transaction. A plan recovers only when every catalog row in
  scratch was last written by one of them and scratch holds no `_ntnt` ledger;
  otherwise it refuses with `DB_SCRATCH_NOT_EMPTY`, names why, and drops
  nothing. A database another project uses as its target, reached through a
  misconfigured scratch, is therefore never wiped, and that project's
  `status`, `plan`, `verify` and `migrate` refuse a target holding the marker
  with `DB_TARGET_IS_SCRATCH` (#430).
- **Replay has its own deadline.** It stops at the inspection deadline, cancels
  the running statement, and cleanup still gets 30 seconds; `DB_TIMEOUT` says
  scratch may keep leftovers for the next plan to recover. `scratch.cleanup`
  is `not_needed` when nothing was committed. Cleanup drops nothing unless it
  holds the replay lock, and a committed marker that is gone is
  `DB_SCRATCH_CLEANUP` (#430).
- **Concurrent plans on one scratch database wait for each other** instead of
  interleaving: a session advisory lock covers each replay, a second plan,
  status or verify waits up to `migrator_wait_seconds` and then fails with
  `DB_SCRATCH_BUSY`. A migration that releases the lock
  (`pg_advisory_unlock_all()`) is refused before it commits, and a killed
  replay's leftovers wait until no other session of the scratch role is
  connected (`DB_SCRATCH_BUSY`) (#430).
- **Cleanup restores the scratch database and role.** Its privileges
  (compared as sets), connection limit, comment and settings, and the scratch
  role's settings, are recorded in the marker and restored, so histories that
  run `ALTER DATABASE current_database() SET ...` or `GRANT`/`REVOKE CONNECT`
  in a `DO` block, or recreate `public` for `pg_database_owner`, plan with the
  same digests as before (#430).
- **Changes cleanup cannot undo are refused before they commit**, with
  `DB_REPLAY_UNSUPPORTED` naming the migration: passwords (PostgreSQL's write
  counters also count one an exception block rolled back), memberships, role
  attributes and comments, other databases, objects given to another role, and
  a session left acting as another role (#430).
- **Three new scratch requirements:** the scratch role may hold no privileges
  on objects it does not own other than on `public` (`DROP OWNED` would revoke
  them), and `track_counts` must be on, as it is by default. Otherwise plans
  fail with `DB_SCRATCH_UNSAFE` and change nothing. Before replay the scratch
  role must also own nothing in scratch but `public`, such as a large object
  or a publication, which the emptiness check did not look for: `DROP OWNED`
  would drop it. Such a scratch is refused with `DB_SCRATCH_NOT_EMPTY` and
  kept (#430).
- Same-named constraints with different `INITIALLY DEFERRED` defaults no
  longer stop replay with `DB_REPLAY_UNSUPPORTED` (#430).
- `capabilities.operations.transactional_script_replay` now reads
  `per_migration_commits_on_scratch` instead of `rollback_only_scratch`.
  Capabilities feed no digest: plans saved by earlier 0.6.x builds stay valid,
  and accepted histories keep their plan digests and fingerprints.

Catalogs that use none of these keep their fingerprints, so saved plans and
applied history stay valid.

### Changed: capabilities keys

`capabilities.inspection` in the JSON reports names what PostgreSQL inspection
covers, and its keys changed with `format_version` still 1:

| 0.6.4 | 0.6.5 |
| --- | --- |
| `custom_types_external_dependencies_non_sql_routines`: `unknown` | `enums_domains_composite_types_operators_exclusion_constraints`: `definition_fingerprints` and `range_and_base_types_external_dependencies_non_sql_routines`: `unknown` |
| `managed_schema_extensions`: `version` | `managed_schema_extensions`: `version_covers_members` |

A script that reads these keys must use the new names (#430).

## `ntnt db` on SQLite

- **Built-in functions.** SQLite migrations can call SQLite's built-in
  functions in defaults, CHECK constraints, index expressions, triggers, views
  and backfills: the JSON functions (`json_extract`, `json_set`, `->>`, the
  `json_each` and `json_tree` table-valued functions and the rest), `hex`,
  `randomblob`, `replace`, `instr`, `round`, `nullif`, `iif`,
  `group_concat`, the window functions and the other core scalar and
  aggregate functions. `DEFAULT (lower(hex(randomblob(16))))` now plans,
  migrates and verifies. `load_extension`, `sqlite_log`, and functions an
  extension or the application registers stay refused with
  `DB_SQL_UNSUPPORTED`, which names the function and now says what a
  migration can use (#427).
- **AUTOINCREMENT.** Tables with `INTEGER PRIMARY KEY AUTOINCREMENT`, the
  default primary key in Django and Rails schemas, plan, migrate and verify.
  The counters in `sqlite_sequence` are left out of coverage and drift, like
  PostgreSQL sequence counters; the table's definition, keyword included,
  still counts. These tables used to fail with ``Statement 1 uses table
  `sqlite_sequence` `` (#427).
- **Planner statistics.** After the application runs `ANALYZE` or `PRAGMA
  optimize`, `status`, `verify` and `plan` no longer report `sqlite_stat1` as
  uncovered, which blocked every migration, and a migration can drop a table
  or index the statistics describe. A migration can run `ANALYZE` itself
  (#427).
- **ADD COLUMN.** `ALTER TABLE ... ADD COLUMN` on a STRICT table, or with a
  CHECK constraint, plans like any other ADD COLUMN instead of failing with
  ``uses SQLite function `glob` ``. When existing rows on the target would
  break the new column, `migrate` names the failure, such as `CHECK
  constraint failed`, and commits nothing (#427).
- **Reads and CTEs.** `INSERT INTO stats SELECT count(*) FROM events` and
  other reads that name no column of a table run, and `WITH RECURSIVE` is
  allowed, including a `WITH` clause that leads an INSERT, UPDATE or DELETE
  (#427).
- **Rebuilds keep AUTOINCREMENT counters.** A `sqlite_rebuild` of an
  AUTOINCREMENT table that copies its rows into a replacement used to reset
  the counter to the highest id copied, so the ids of deleted rows were
  given out again. The rebuild guard now refuses that with
  `DB_REBUILD_GUARD`, naming the table and both counter values, and its
  `next_action` gives the `UPDATE`, `DELETE` and `INSERT` to put before the
  DROP (docs/DATABASE_MIGRATIONS.md, Explicit one-table rebuild). They leave
  the replacement one counter row with the highest counter of both tables,
  whether it had none, one or more. SQLite reads only the first
  `sqlite_sequence` row for a name, so the guard also refuses a rebuilt table
  left with two, and gives the same lines (#427).
- **Rebuild guard says what failed.** `DB_REBUILD_GUARD` names each rule a
  rebuild broke and the object: ``the row count of table `log` changed from 0
  to 1``, ``index `index_posts_on_user_id` has another stored SQL text than
  before the rebuild`` (SQLite compares the stored text, so whitespace and
  quoting must match) or ``view `titles` is missing``, with their catalog
  keys in `objects` and a `next_action` for each rule (#427).
- **Clearer refusals.** A function SQLite does not define, such as one the
  application registers, used in an INSERT or UPDATE is now
  ``DB_SQL_UNSUPPORTED: Statement 2 uses SQLite function `app_slugify` ``,
  as in a DEFAULT, instead of `no such function`. `ALTER TABLE ... ADD COLUMN
  c TEXT UNIQUE` reports SQLite's `Cannot add a UNIQUE column` and suggests
  ADD COLUMN followed by CREATE UNIQUE INDEX, instead of naming an internal
  autoindex (#427).
- **Failures that replay cannot show.** On a target with rows, ADD COLUMN
  with a non-constant default (`CURRENT_TIMESTAMP`, `randomblob()`) or NOT
  NULL without a default, and a rebuild without the sidecar of a table that
  other tables reference (immediate keys fail the DROP, deferred keys such as
  Django's fail the COMMIT), now fail with a `next_action` that says what to
  write instead: a constant default and an UPDATE, or the `sqlite_rebuild`
  sidecar. The COMMIT failure also names its migration (#427).
- **Blocked saved plans.** `migrate --plan` with a file that `plan --out`
  saved when `plan` stopped before it could make a plan (outcome `fail`, no
  `plan_digest`) says the plan is blocked instead of calling it edited. The
  quick start checks a plan's outcome before approving its digest (#427).

## Security

The lockfile updates `cookie` from 0.18.2 to 0.18.3 for
[RUSTSEC-2026-0594](https://rustsec.org/advisories/RUSTSEC-2026-0594.html)
(#456).

See [NTNT v0.6.4](v0.6.4.md) for the latest release announcement and upgrade
guidance.
