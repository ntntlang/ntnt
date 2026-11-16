# Unreleased (0.6.2)

## `ntnt db` connects to PostgreSQL over TLS

`ntnt db` can now manage remote and managed PostgreSQL. The connection secret's
`sslmode` accepts libpq's `verify-full`, `verify-ca` and `require`, with `sslrootcert`
for a private CA (#344).

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
- **Upgrade:** the target identity now includes the transport, so plans made by
  0.6.1 must be regenerated. `sslmode=disable` secrets keep working unchanged.

See [NTNT v0.6.1](v0.6.1.md) for the published announcement, upgrade guidance,
known limits and retained implementation measurements.
