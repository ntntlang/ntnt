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
  comparison would fail could pass. A check that compares a column whose type
  (or a domain's base type) has a same-named operator outside `pg_catalog`
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

## Security

The lockfile updates `cookie` from 0.18.2 to 0.18.3 for
[RUSTSEC-2026-0594](https://rustsec.org/advisories/RUSTSEC-2026-0594.html)
(#456).

See [NTNT v0.6.4](v0.6.4.md) for the latest release announcement and upgrade
guidance.
