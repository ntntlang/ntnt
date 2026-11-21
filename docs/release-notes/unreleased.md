# Unreleased (0.6.4)

0.6.4 fixes `ntnt db` problems found while moving real projects onto it, and
reports a runtime error in an imported module against that module's file.

## `ntnt db` on SQLite

- **Time defaults.** Migrations can use SQLite's date and time functions, so
  `DEFAULT CURRENT_TIMESTAMP`, `DEFAULT (datetime('now'))` and an `updated_at`
  trigger plan, migrate and verify. They used to fail `plan` with
  `DB_SQLITE` "code -1" (#414).
- **Refusals name what was refused.** A statement that uses a function, table or
  PRAGMA outside the supported set fails with `DB_SQL_UNSUPPORTED`, naming the
  statement number and what it used, for example ``Statement 2 uses SQLite
  function `randomblob` ``. Other failing statements are numbered too (#414).
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

## `ntnt db` on every engine

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
  module names the module. These errors used to name the entry file with the
  module's line number, beside unrelated or empty source (#412).
- Route handler error logs and the development error page still pair the
  handler's file with the error's line; #257 tracks them.

See [NTNT v0.6.3](v0.6.3.md) for the previous release.
