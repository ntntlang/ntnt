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

See [NTNT v0.6.1](v0.6.1.md) for the published announcement, upgrade guidance,
known limits and retained implementation measurements.
