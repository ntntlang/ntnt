## 6. Adoption Slice 16M — production migration compatibility

**Consumer dependency only:** landed DD-077 PR 1C, landed DD-078 owner 8, and the Larrimon database-conversion wave.

This slice is intentionally absent from the DD-078 core dependency table and releases.

Run old migration checks and native `ntnt db`/`.tnt` evidence on one immutable Larrimon revision across:

- fresh install and idempotent rerun;
- every supported legacy ledger and application/schema upgrade pair;
- checksum backfill and pre-package unverifiable rows;
- unknown-ledger rejection before mutation;
- malformed or missing manifests;
- missing or mutated applied files;
- database checksum enforcement;
- concurrent migrators and advisory locks;
- per-migration rollback and dirty recovery;
- cancellation and role configuration.

Inject failures/mutations for every family and retain paired reports.

**Exclusive deletion authority:** Only this consumer slice may authorize removal of:

- `scripts/migrate.sh`;
- `scripts/migrate-prod.sh`;
- `scripts/check-migration-checksums.py`;
- `tests/migrate_prod_integration.sh`.

DD-078 owner 8 may provide observations but cannot authorize these deletions. A later operational matrix may expand supported cases, but the currently supported production matrix cannot be deferred past deletion.
