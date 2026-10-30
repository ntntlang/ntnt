# Adopt an existing customer/order application

This disposable example uses the same `ntnt db` executor as deployment. It is not
an application-level migration runner and does not contact a real deployment.

## Run the complete rehearsal

From the repository root:

```sh
cargo test --locked --test db_sqlite sqlite_documented_application_adopts_upgrades_and_preserves_old_readers -- --exact --nocapture
```

The native fixture provisions separate local target/scratch files, models an
already-running application with customers and orders but no NTNT ledger, then:

1. Runs `app-v1.tnt` through the real NTNT executable and checks its exact rows.
2. Confirms ordinary inspection reports unrecognized history rather than a new DB.
3. Plans adoption through migration **ID 1** (not pending ID 10), with zero data
   assertions executed during planning.
4. Supplies fixture-only independent approval and adopts the matching baseline,
   running the three named checks without replaying `0001_existing.sql` on target.
5. Plans/applies migration 10, which expands customers with a locale.
6. Runs both old and new application readers, proving names, emails, order amounts
   and the added default are preserved for this fixture.
7. Reruns the same upgrade without re-executing SQL/checks and verifies the catalog.

Both files use WAL because the ordinary `std/db/sqlite` application driver selects
WAL. The migration adapter does **not** silently change journal mode to make replay
match. File paths and fake fixture approvals stay in the disposable test process.
The scripts' `EXAMPLE_DATABASE_FILE` is local application configuration, not a
migration approval or production credential. They use the existing application
SQLite driver, not the migration inspector's readonly connection API.

## Operator workflow

Use a copied example/project and explicitly provision an existing target plus a
distinct empty scratch file with matching settings. Supply `DB_TARGET`,
`DB_SCRATCH` and approval through the existing declared secrets provider. Paths
must be existing absolute native file paths; no URI, links, streams or implicit
file creation by `ntnt db`.

```sh
ntnt db status --database app --target test --json
ntnt db plan --database app --target test --adopt-through 1 --out adoption.json --json
# Review the exact baseline, target, checks and plan. Separately provision
# DB_APPROVAL with the reviewed digest through the protected provider.
ntnt db migrate --database app --target test --plan adoption.json --json
ntnt db verify --database app --target test --json

ntnt db plan --database app --target test --out upgrade.json --json
# Review/provision the new exact approval; the adoption digest cannot approve it.
ntnt db migrate --database app --target test --plan upgrade.json --json
ntnt db verify --database app --target test --json
```

Do not copy these fixture steps onto production or seed a ledger manually. A
baseline checkpoint attests to the supported current catalog and checks; it does
not claim the old SQL executed, prove arbitrary mappings, or retire old writers.
Backups/restore, application cutover and production authorization stay separate.
See [Database Migrations](../../docs/DATABASE_MIGRATIONS.md) for PostgreSQL,
recognized legacy history import, authority, coverage and failure recovery.
