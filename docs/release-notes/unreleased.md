# Unreleased (0.6.2)

## A tested local development setup for `ntnt db`

`examples/database-dev` provides what local `ntnt db` work needs (#345): a target
database, a separate disposable scratch database and an approval step.

- **PostgreSQL:** one `docker compose up -d --wait` starts PostgreSQL 17 on
  `127.0.0.1:55432`. An init script creates the target and scratch databases, each
  owned by its own unprivileged role, with matching settings. A `dev.env` sets the
  development secrets. The data lives in a named volume: `docker compose down`
  keeps it, and `down -v` resets it.
- **SQLite:** `. ./dev.env` creates the empty target and scratch files once and sets
  their absolute paths.
- **Approval stays explicit.** Development supplies the plan digest through the
  environment provider, and production still requires its separately protected
  provider. `ntnt db` itself still never provisions databases.

`tests/db_dev_recipe.rs` runs both setups as documented, including the next
migration and a rejected approval for the wrong plan.

## `ntnt db` connects to PostgreSQL over TLS

`ntnt db` can now manage remote and managed PostgreSQL. The connection secret's
`sslmode` accepts libpq's `verify-full`, `verify-ca` and `require`, with `sslrootcert`
for a private CA (#344). `verify-ca`, which skips the host-name check, requires an
`sslrootcert` CA file and never trusts the host CA store.

- **Verified by default.** `verify-full` checks the certificate chain and the host
  name against the host CA store, the store NTNT's other TLS clients use, or only
  against the CAs in `sslrootcert`.
- **No plaintext fallback.** A missing `sslmode` and `prefer`/`allow` are rejected.
  Every TLS mode fails with `DB_TLS` when the server does not offer TLS. Certificate
  and host-name failures stop before any SQL runs.
- **Plans bind the transport.** Each session reports its `sslmode`, trust source
  and negotiated TLS version, plus what PostgreSQL reports for its own end
  (`server_ssl`, which is false behind a TLS-terminating proxy or pooler). A plan
  reviewed over `verify-full` is stale over `require` or `disable`. `require`
  (encrypted, unverified) must be chosen explicitly and is reported as unverified.
- **Upgrade:** the target identity now includes the transport, so `migrate` needs a
  plan made by 0.6.2: regenerate plans made by 0.6.1. `sslmode=disable` secrets keep
  working unchanged. If a 0.6.1 `migrate` stopped during `CREATE INDEX CONCURRENTLY`,
  `plan` fails with `DB_ATTEMPT_UNRESOLVED` until that attempt is resolved:
  `ntnt db reconcile --plan <the 0.6.1 plan>` accepts the 0.6.1 plan over
  `sslmode=disable`, the only transport 0.6.1 used. Then plan, approve and migrate
  as usual.

## Database migration time limits

PostgreSQL databases can raise `ntnt db`'s fixed limits for large tables with an
optional `[databases.<name>.limits]` table. `statement_seconds` (default 90, 2
to 86400) bounds each migration script and concurrent index build on the target;
`migrator_wait_seconds` (default 5, 1 to 300) bounds the wait for a competing
migrator. `migrate` and `reconcile` extend their deadlines to fit, plans record
the limits in `execution.limits`, and changing them requires a new plan.
Manifests without the table keep the old limits and their saved plans stay
valid. A competing migrator now fails with `DB_LOCK_CONFLICT` instead of a
statement timeout. See
[Time limits](../DATABASE_MIGRATIONS.md#time-limits) (#346).

## Fixes

- **macOS `std/process`:** starting a command that exits immediately could fail
  with `failed to configure process monitoring: No such process (os error 3)`. The
  child had exited before NTNT began watching it; it is now treated as exited and
  its result is returned as usual (#381).

See [NTNT v0.6.1](v0.6.1.md) for the published announcement, upgrade guidance,
known limits and retained implementation measurements.
