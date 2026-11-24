# Unreleased

## `ntnt db` on PostgreSQL

- **Checks on tables with row-level security.** Checks read with
  `row_security=off`, so that no policy can silently filter them, and PostgreSQL
  refuses such a read where row-level security applies to the migration role:
  a table with `FORCE ROW LEVEL SECURITY`, for the unprivileged owner role the
  migration guide uses. `plan` now lists each such `requires`, `ensures` or
  adoption check in `plan.check_warnings` (`CHECK_ROW_SECURITY`), naming the
  check and the table, and `migrate` refuses it with `DB_CHECK_ROW_SECURITY`
  before it runs, saying that the migration role needs `BYPASSRLS`. Such a
  migration used to plan cleanly and then fail with a bare SQLSTATE 42501.
  Plans without such checks keep their digests (#430).
- **Untrusted extensions.** A migration that creates an untrusted extension,
  such as `postgres_fdw`, fails with `DB_EXTENSION_PRIVILEGE`, naming the
  statement and the extension, explaining that roles that are not superusers
  can create only trusted extensions, and saying what to do. It used to fail
  with a bare SQLSTATE 42501. Support for extensions that a superuser installs
  remains open in #430.

See [NTNT v0.6.4](v0.6.4.md) for the latest release announcement and upgrade
guidance.
