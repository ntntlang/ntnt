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

See [NTNT v0.6.1](v0.6.1.md) for the published announcement, upgrade guidance,
known limits and retained implementation measurements.
