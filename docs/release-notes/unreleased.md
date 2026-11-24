# Unreleased

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

See [NTNT v0.6.4](v0.6.4.md) for the latest release announcement and upgrade
guidance.
