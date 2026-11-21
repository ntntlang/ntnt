# Unreleased (0.6.4)

0.6.4 fixes `ntnt db` problems found while moving real projects onto it, and
reports a runtime error in an imported module against that module's file.

## `ntnt db` on SQLite

- **Time defaults.** Migrations can use SQLite's date and time functions, so
  `DEFAULT CURRENT_TIMESTAMP`, `DEFAULT (datetime('now'))` and an `updated_at`
  trigger plan, migrate and verify. They used to fail `plan` with
  `DB_SQLITE` "code -1" (#414).
- **Refusals name what was refused.** A migration statement that calls a
  function outside the supported set, itself or through a trigger it fires or a
  view it reads, fails with `DB_SQL_UNSUPPORTED`, naming the statement number
  and the function, for example ``Statement 2 uses SQLite function
  `randomblob` ``. Other failing statements are numbered too. A trigger or view
  body that nothing in the migration uses is checked only during inspection and
  reported as `DB_COVERAGE_INCOMPLETE`, and an authored PRAGMA is refused before
  replay with `DB_REPLAY_UNSUPPORTED` (#414).
- **Scratch journal mode.** An empty scratch takes the target's journal mode, so
  a scratch made with `touch` still works after the application has opened the
  target, which `std/db/sqlite` `connect()` switches to WAL. Any other setting
  that differs fails with `DB_SCRATCH_MISMATCH`, naming each setting and both
  values. These cases used to fail with `DB_SQLITE` "code 1" (#413).
- **Partial and expression indexes.** `CREATE INDEX ... WHERE ...`, unique or
  not, and indexes on expressions such as `lower(email)` plan, migrate and
  verify, and a predicate or expression changed outside a migration is reported
  as drift. They used to leave every plan `unknown` with
  `DB_COVERAGE_INCOMPLETE` (#411).
- **Drops that would delete child rows.** A migration that drops a table which
  other tables reference with `ON DELETE CASCADE`, `SET NULL` or `SET DEFAULT`
  is refused with `DB_REBUILD_REQUIRED`, naming those tables, unless it uses
  the `sqlite_rebuild` sidecar. SQLite deletes every row of a dropped table
  first. A rebuild without the sidecar therefore deleted or orphaned child rows
  on the target while plan, migrate and verify all passed (#419).
- **Statement limit.** On a target, a SQLite migration runs under
  `[databases.<name>.limits] statement_seconds` (90 seconds by default) instead
  of a fixed 5 seconds. A statement past the limit fails with
  `DB_STATEMENT_LIMIT`, naming the statement and the limit. Scratch replay keeps
  its fixed 5-second bound. A plan records a declared limit, so a changed limit
  makes the plan stale (#429).
- **Long histories.** Replay inspection reuses the work for objects a migration
  does not change, so `plan`, `status`, `verify` and `migrate` grow about
  linearly with the history: 400 one-table migrations plan in under half a
  second instead of 28 seconds, and 1,024 in about 2 seconds instead of timing
  out. Histories dense in foreign keys benefit too: a 400-table `ON DELETE
  CASCADE` chain plans in half a second. SQLite refuses programs nested more
  than 1,000 trigger or foreign-key-action levels deep, so a table at the end
  of a deeper chain is uncovered. SQLite operations now run on threads with an
  8 MiB stack, enough for the deepest programs SQLite accepts (#421).

## `ntnt db` on PostgreSQL

- **Objects outside the managed schema.** Inspection covers every schema the
  migration history creates, and the objects the history creates in other
  schemas (for example `public.audit_log`), with the same coverage and drift
  rules as the managed schema. An object dropped there out of band is now
  reported as drift, not as a pass. Other objects in those schemas are not
  inspected (#420).
- **Statements outside the managed schema** run as written. A pending statement
  that may act outside the managed scope (a qualified name, an `IN SCHEMA` list,
  `search_path`, or SQL the parser cannot read) is flagged in the plan with the
  hazard `OUTSIDE_MANAGED_SCOPE`, naming the statement and the schema, and the
  plan can still be approved. `DB_OUTSIDE_SCOPE` refuses only changes to the
  `_ntnt` ledger schema and to system schemas. When the history creates objects
  outside the managed scope, `status` and `verify` read the target twice, and
  report `DB_TARGET_CHANGED` instead of comparing if it changed in between. A
  saved plan whose pending migrations act outside the managed scope must be
  planned again (#420).
- **Policies that call outside routines.** A row-level security policy that
  calls a routine outside `pg_catalog` and the managed scope is now uncovered.
  It used to be reported as covered (#434).
- **Long histories (unchanged).** PostgreSQL replay still costs time in
  proportion to the square of the history length; DATABASE_MIGRATIONS.md
  states the current limits. #421 stays open for PostgreSQL.
- **Known limitation.** A pending migration that replaces an object already on
  the target outside the managed schema, for example `DROP TABLE IF EXISTS
  public.legacy` followed by `CREATE TABLE public.legacy`, is reported as drift
  and cannot be planned. Create the replacement under a new name, or drop the
  old object before planning (#439).

## `ntnt db` on every engine

- **Empty migrations.** A migration with no statements, such as the file `ntnt
  db new` creates, fails with `DB_MIGRATION_EMPTY`, naming it, instead of being
  reported as unsupported SQL (#428).
- **Coverage diagnostics name the objects.** `DB_COVERAGE_INCOMPLETE` lists the
  catalog objects outside supported inspection in a new `objects` field, and
  when replaying a migration produced them, `source` names that migration.
  Human output prints a diagnostic's `Source:` and `Objects:` lines (#411).
- **`ntnt db new` takes turns.** While it numbers and creates a migration,
  `ntnt db new` holds the lock on `ntnt.toml` that `ntnt db init` already uses,
  so two runs at the same moment can no longer both create `0001_...`. A run
  that finds the lock held stops at once with `DB_NEW_CONFLICT`, creates
  nothing and can simply be rerun; the lock ends with the command, so there is
  nothing to clean up. `ntnt db init`'s conflict message now names `ntnt db new`
  and the manifest (#388).
  - On Windows, a `status` or `plan` that reads `ntnt.toml` during that moment
    fails with `DB_INPUT_IO` and can be rerun.
  - On NFS, a read-only `ntnt.toml` cannot be locked, so `ntnt db new` fails
    with `DB_SCAFFOLD_IO` naming the file and the error.

## Runtime errors in imported modules

- **The right file and line.** `ntnt run` reports a runtime error raised in a
  function from an imported module against that module's file and line, shows
  that file's source line, and adds a `note: called from main.tnt:3` for each
  call site that led there (`note: imported from ...` when the module's own
  top-level code failed while it was imported). A parse error in an imported
  module names the module, and an error raised while a function's `requires` or
  `ensures` clause is evaluated points at that clause. These errors used to name the entry file with the
  module's line number, beside unrelated or empty source (#412).
- Route handler error logs and the development error page still pair the
  handler's file with the error's line; #257 tracks them.

## Upgrading

- Finish any `migrate` that stopped during a concurrent index build with
  `ntnt db reconcile` before upgrading. A plan saved by 0.6.3 whose pending
  migrations act outside the managed schema no longer validates in 0.6.4, so
  0.6.4 cannot reconcile that attempt (#440).

See [NTNT v0.6.3](v0.6.3.md) for the previous release.
