# Unreleased (0.6.4)

0.6.4 fixes `ntnt db` problems found while moving real projects onto it.

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

See [NTNT v0.6.3](v0.6.3.md) for the previous release.
