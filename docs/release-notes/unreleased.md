# Unreleased

## SQLite migrations

`ntnt db plan` used to report syntax errors for SQL that SQLite itself
accepts, including `WHEN OLD.value IS NOT NEW.value` in a trigger, `GLOB`
in a `CHECK`, and `STRICT, WITHOUT ROWID` on a table. These migrations can
now be planned, applied and verified. The same restrictions on what a
migration may do still apply (#426).

## PostgreSQL migrations

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
