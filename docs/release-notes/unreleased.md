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

The lockfile updates `cookie` from 0.18.2 to 0.18.3 for [RUSTSEC-2026-0594](https://rustsec.org/advisories/RUSTSEC-2026-0594.html).

See [NTNT v0.6.4](v0.6.4.md) for the latest release announcement and upgrade
guidance.
