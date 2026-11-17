# Unreleased (0.6.2)

## Database scaffolding

`ntnt db init --engine sqlite` (or `--engine postgresql`) writes a commented
`[databases.app]` section to `ntnt.toml` with a development target, a disposable
scratch database and an approval secret, declares those secret names (never their
values) and creates `migrations/`. `--database <name>` replaces `app`. If
`ntnt.toml` exists, the section is appended and the existing bytes are kept; init
refuses, changing nothing, when that database or one of its secrets is already
declared. `ntnt db new <name>` creates the next numbered migration file, such as
`migrations/0002_<name>.sql`, without reading secrets or connecting to a database.
Neither command overwrites a file (#348). The manifest can still be written by
hand; see [Quick start](../DATABASE_MIGRATIONS.md#quick-start).

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

See [NTNT v0.6.1](v0.6.1.md) for the published announcement, upgrade guidance,
known limits and retained implementation measurements.
