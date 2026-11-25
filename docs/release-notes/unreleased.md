# Unreleased

## SQLite migrations

`ntnt db plan` used to report syntax errors for SQL that SQLite itself
accepts, including `WHEN OLD.value IS NOT NEW.value` in a trigger, `GLOB`
in a `CHECK`, and `STRICT, WITHOUT ROWID` on a table. These migrations can
now be planned, applied and verified. The same restrictions on what a
migration may do still apply (#426).

See [NTNT v0.6.5](v0.6.5.md) for the latest release announcement and upgrade
guidance.
