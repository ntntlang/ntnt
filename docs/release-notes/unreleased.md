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

See [NTNT v0.6.5](v0.6.5.md) for the latest release announcement and upgrade
guidance.
