# NTNT v0.5.5 — Faster Redis jobs, auth fixes and probe deadlines

This release makes the Redis job backend scale across worker processes, fixes
built-in OAuth logins in server mode, makes every `enable_auth` call form accept
the same options, and adds start deadlines for monitoring probes. **Read the
upgrade notes before updating workers: the Redis backend now requires Redis 7.0+
or Valkey 7.2+, and workers sharing a queue must be upgraded together.**

## Upgrade notes

- **Redis 7.0+ / Valkey 7.2+ required for the Redis job backend.** Job state
  changes now commit through one Lua script. Workers refuse to start on an older
  server. Redis credentials need `EVAL`, `EVALSHA` and `SCRIPT`; `WATCH`, `MULTI`
  and `EXEC` are no longer used.
- **Upgrade all writers and workers on a queue together.** The Redis ready index
  and the Lua commit are not compatible with v0.5.4 workers. Stop every worker,
  upgrade, then restart. SQLite-backed queues are unaffected.
- **Built-in auth route changes in server mode.** Auth routes now follow
  `route_prefix`, and the unused `GET /auth/callback` route is gone. OAuth
  callbacks use `{prefix}/{provider}/callback`, which is what `auth_start` always
  sent to providers. An unknown provider under the prefix returns 404 instead of
  500. A custom provider named `health` takes `{prefix}/health`; the built-in
  health route is then not registered and startup prints a warning.

## Authentication

- **OAuth logins through the built-in routes no longer end on a 404.** In server
  mode the interpreter registered its own hardcoded `/auth` routes, so the callback
  URL sent to providers was never registered. The server now registers exactly the
  routes `std/auth` lists, under `route_prefix`, with fixed routes (`health`,
  `logout`) before `{provider}`. `/auth/health` works again instead of returning
  500. (#258)
- **One option parser for every `enable_auth` call form.** One- and two-argument
  calls used an older parser that required an OAuth provider and rejected
  documented keys such as `cookie_same_site`, `cookie_http_only` and
  `route_prefix`, as well as the `enable_auth(providers, "preset")` form. All
  forms now use the same parser, so local-only auth (an empty provider array) works
  as documented. `login_url` and `callback_url` were never options; the guide no
  longer shows them. (#223)

## Background jobs

- Several `ntnt worker` processes can share one project and worker group. Each
  takes its own numbered control endpoint, and `ntnt workers` commands reach every
  live process in the group. See [worker control](worker-control.md#several-processes-in-one-group).
- Redis workers use a `jobs:ready` index instead of scanning all pending keys on
  every claim, give each worker slot its own connection, and no longer fence on
  shared indexes. Each state change is one read round trip plus one commit, and
  claims read 32 ready entries instead of 256. (#219, #220, #228)
- A lease keeper retries a renewal that hit momentary store contention after 25ms
  instead of waiting a full renewal interval, so a few collisions in a row can no
  longer let a running job's lease expire.
- SQLite job state changes report brief store contention as retryable
  (`local_busy` / `busy`) instead of a generic storage failure.

## Language and standard library

- `int_or(value, fallback)` converts like `int()` but returns the integer
  fallback on failure instead of a `Result`. (#128)
- Optional `start_deadline_ms` / `start_monotonic_deadline_ms` for `ping`,
  `snmp_get` and `dns_lookup`, checked just before the first send or connect.
  New `std/http.probe_fetch` performs a bounded monitoring GET with the same
  deadline. See [probe start deadlines](probe-start-deadlines.md).
- `dns_lookup` / `dns_reverse` on Windows retry once on an OS-assigned UDP port
  when Windows refuses the resolver's random port (error 10013).
- `std/secrets` Unix-socket provider reads a complete response from an agent that
  answered and closed before the client read it, instead of reporting it
  unavailable (macOS).
- `std/process` on macOS no longer fails timeout cleanup when a process is briefly
  impossible to inspect while it exits.

## Intent and tooling

- IAL `header {name} exists` and `header {name} equals {value}` resolve in
  `ntnt intent lint` and run in `ntnt intent check`. Header names match
  case-insensitively; values compare exactly. (#224)
- Parse errors at string literals point at the opening quote, and show the
  literal exactly as written. (#261)

## Releases and installation

- Linux ARMv7 hard-float release archives. The Unix installer accepts
  `--version` / `NTNT_VERSION` and verifies checksums and the binary version
  before replacing an installed `ntnt`. See [RELEASING](RELEASING.md).

## Not in this release

Exact column-level runtime error locations (#218) were merged after v0.5.4 and
reverted before this release because of regressions in path resolution,
performance and diagnostics. Runtime errors keep v0.5.4's line-level locations.
A redesign is tracked in #257.

## Verification

- Release CI on Linux, macOS and Windows for the tagged commit.
- New in CI: a multi-file fixture app (templates, partials, `compile()`, file
  routes and `jobs()` called from `lib/`), assertions that interpreter fast
  paths actually execute, and a Linux release-profile timing guard.
