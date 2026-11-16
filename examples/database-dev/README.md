# Local development databases for `ntnt db`

`ntnt db` never provisions databases. It needs an existing target and a separate,
empty **scratch** database for disposable replay, plus an approval for each plan.
These two setups provide both for local development. They are not for production.

## PostgreSQL (Docker Compose)

```sh
cd examples/database-dev/postgresql
docker compose up -d --wait   # PostgreSQL 17 on 127.0.0.1:55432
. ./dev.env

ntnt db status --target development
ntnt db plan --target development --out review-0001.json
# Review review-0001.json and the SQL. Then approve exactly this plan:
export APP_MIGRATION_APPROVAL="$(jq -r .plan_digest review-0001.json)"
ntnt db migrate --target development --plan review-0001.json
ntnt db verify --target development
```

`initdb/10-ntnt-db.sql` runs once, when the container first starts. It creates
`app_development` and the scratch database `app_replay`, each owned by its own
unprivileged role, from the same template so their settings match. The manifest
manages the `public` schema. `dev.env` holds the throwaway development
credentials and selects the development environment provider.

The databases live in the `data` volume: `docker compose down` and
`docker compose up -d --wait` keep them. `docker compose down -v` deletes
everything, scratch included, and the next start is empty. Set `NTNT_DB_DEV_IMAGE`
to use another PostgreSQL 16–18 image.

## SQLite

```sh
cd examples/database-dev/sqlite
. ./dev.env   # creates empty data/app.sqlite and data/replay.sqlite once

ntnt db status --target development
ntnt db plan --target development --out review-0001.json
export APP_MIGRATION_APPROVAL="$(jq -r .plan_digest review-0001.json)"
ntnt db migrate --target development --plan review-0001.json
ntnt db verify --target development
```

`rm -rf data` resets both files.

## Next changes

Add the next numbered file to `migrations/` (for example `0002_names.sql`), then
plan, review, approve and migrate again. `ntnt db` never overwrites a saved plan,
so give each plan its own file:

```sh
ntnt db plan --target development --out review-0002.json
export APP_MIGRATION_APPROVAL="$(jq -r .plan_digest review-0002.json)"
ntnt db migrate --target development --plan review-0002.json
ntnt db verify --target development
```

Each plan has its own digest, and an approval applies only to the plan it names:
`migrate` without `APP_MIGRATION_APPROVAL`, or with another plan's digest, changes
nothing. Rerunning an applied plan reports `already_applied`.

Production uses its own targets, the Unix-socket secrets provider and a
separately protected approval. See
[Database migrations](../../docs/DATABASE_MIGRATIONS.md) for the full workflow.

`tests/db_dev_recipe.rs` runs both setups end to end, exactly as written here.
