# Unreleased

## Post-deploy verification

Use `ntnt db verify --target production --require-no-pending` after deployment
to catch a skipped `migrate` on PostgreSQL or SQLite. With one migration left,
ordinary `verify` can report `pass (1 pending)`; the same check with the flag
reports `fail (1 pending)` and exits nonzero. Neither command applies the
missing migration (#432).

`status`, `plan`, and `verify` no longer print every table and column by
default. The verdict, migration counts, and any diagnostic and next step come
first. Schema differences and unverified objects are still named. Use
`--object accounts` to inspect matching entries, or `--json` for the full report.

If an applied file such as `0020_b.sql` was renamed to `0025_bee.sql`, `ntnt db
status` can now identify it by checking its contents under the original name.
Restore the original filename and its `.toml` sidecar if it has one; no SQL
needs to run again. The database ledger does not change (#446).

## SQLite migrations

`ntnt db plan` used to report syntax errors for SQL that SQLite itself
accepts, including `WHEN OLD.value IS NOT NEW.value` in a trigger, `GLOB`
in a `CHECK`, and `STRICT, WITHOUT ROWID` on a table. These migrations can
now be planned, applied and verified. The same restrictions on what a
migration may do still apply (#426).

## PostgreSQL migrations

`ntnt db plan` used to pass a concurrent index build on a partitioned parent
because scratch built a plain index instead. PostgreSQL then rejected the real
build during deployment. Planning now blocks that command. Concurrent builds
on leaf partitions and regular index builds on the parent still work.

Script errors for an undefined column, undefined table, or syntax error keep
their SQLSTATE and include a statement number when PostgreSQL supplies a usable
position in the submitted SQL. Server error text is not printed (#424).

`ntnt db plan` now names the migration file and suggests an explicit schema
name for `CREATE SCHEMA AUTHORIZATION CURRENT_USER`. Without a name, scratch
and target create differently named schemas, so the migration used to pass
planning but roll back during deployment. `CREATE SCHEMA audit AUTHORIZATION
CURRENT_USER` works on both (#448).

If another session adds a foreign key from an untracked table into a tracked
one while a migration is running, `ntnt db migrate` checks again after the
SQL runs, while PostgreSQL still holds its write locks. A newly visible key
rolls back that migration without recording it as applied; earlier migrations
stay committed (#447).

`ntnt db plan` does less repeated work when many PostgreSQL tables have the
same shape. In a PostgreSQL 17 test, the default deadline used to expire
before all 1,024 single-table migrations could be planned; the same history
now plans in 84 seconds, with unchanged fingerprints (#421).

`ntnt db migrate` now applies that same history to an empty PostgreSQL 17
schema within its separate 100-second work budget. In a local debug build,
492 migrations committed before the old CLI timed out; the new CLI applied
all 1,024 in 97 seconds using the old saved plan. It still checks the live
schema before and after each migration, so it still detects changes made by
another connection between commits. More complex histories can still time out.

See [NTNT v0.6.5](v0.6.5.md) for the latest release announcement and upgrade
guidance.
