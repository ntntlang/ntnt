# Unreleased (0.6.1)

See [NTNT v0.6.0](v0.6.0.md) for the curated announcement, upgrade guidance and
retained implementation measurement notes. The 0.5.7 development cycle is included
in 0.6.0; there was no separately published 0.5.7 release.

## Database migrations (`ntnt db`) on real PostgreSQL applications

Tested by adopting and migrating Jarri's staging database (33 migrations, about 11 GB).

- **`public` can be the managed schema.** It was previously rejected, although most applications keep their tables there.
- **Complete migration scripts replay and run.** `DO` blocks, functions, triggers, policies, grants, seed rows and `SET CONSTRAINTS` are allowed. Statements that would end the migration transaction, change the session's role, run outside a transaction, act outside the database, or `CASCADE`-drop are still rejected. Statements NTNT cannot analyze in detail are reported in `analysis.unknowns` rather than blocking the plan.
- **More of the schema is compared.** Views, sequences, SQL and PL/pgSQL routines (definition, owner, grants), triggers, RLS policies, declarative partitions, expression/partial indexes and managed-schema extensions are compared by definition. A manual change is reported as drift on the named object instead of incomplete coverage. Extension functions are compared through the extension's version.
- **Replay matches the target.** Scripts run with normal row-level security, and scratch may bypass RLS only when the target role does. Each migration settles deferred constraints at its own boundary.
- **Locking is narrower.** `migrate` no longer takes `ACCESS EXCLUSIVE` locks on every managed table. An advisory lock serializes migrators, and each statement takes only the locks it needs under `lock_timeout = 1s`. Tables read by `requires`/`ensures` checks are `SHARE`-locked. A lock timeout or deadlock rolls back the migration and retries it with backoff, up to 4 attempts. Migration scripts get a 90-second statement limit. Each committed migration reports the relation locks it held and its `lock_retries`. On Jarri staging, the worst concurrent write stall during a small migration fell from 937 ms to 164 ms.
- **Fixed:** `migrate` no longer re-locks every table after the last migration commits. On a live database, that extra lock pass could deadlock and report failure for a migration that had committed.

## Dependencies

- `pulldown-cmark` 0.13.6 fixes RUSTSEC-2026-0433 (quadratic parsing of nested image wikilinks). NTNT does not enable wikilinks, so `std/markdown` was not exposed.
