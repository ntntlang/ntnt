# Unreleased (0.6.2)

## Database migration time limits

PostgreSQL databases can raise `ntnt db`'s fixed limits for large tables with an
optional `[databases.<name>.limits]` table. `statement_seconds` (default 90, up
to 86400) bounds each migration script and concurrent index build on the target;
`migrator_wait_seconds` (default 5, up to 300) bounds the wait for a competing
migrator. `migrate` and `reconcile` extend their deadlines to fit, plans record
the limits in `execution.limits`, and changing them requires a new plan.
Manifests without the table keep the old limits and their saved plans stay
valid. A competing migrator now fails with `DB_LOCK_CONFLICT` instead of a
statement timeout. See
[Time limits](../DATABASE_MIGRATIONS.md#time-limits) (#346).

See [NTNT v0.6.1](v0.6.1.md) for the published announcement, upgrade guidance,
known limits and retained implementation measurements.
