# Unreleased (0.6.1)

See [NTNT v0.6.0](v0.6.0.md) for the curated announcement, upgrade guidance and
retained implementation measurement notes. The 0.5.7 development cycle is included
in 0.6.0; there was no separately published 0.5.7 release.

## Runtime memory lifetime

The runtime automatically collects unreachable closure/environment cycles. This
addresses memory retained by function-bearing imports in completed scheduled
callbacks, and by local closures in long-running interpreters. Escaped callbacks
keep their captured bindings while they remain reachable. NTNT applications need
no new syntax, cleanup calls or garbage-collector settings.

Native Rust embedders constructing `Value::Function` must convert the environment
`Rc` into the Rc-compatible `Closure` wrapper with `.into()`. Strong capture and
shared-binding semantics are unchanged. This is automatic memory reclamation,
not a new deterministic resource-close API.

## DD-041: WebSockets

Applications can accept WebSockets. Register an endpoint during setup with
`ws_endpoint(name, on_message, options?)`, then return `websocket(endpoint,
map { "state": ... })` from an ordinary GET handler. Middleware and your own
authorization run first; only a successful final response upgrades, so denials,
auth redirects and errors are sent as ordinary HTTP.

Callbacks run on the existing HTTP workers, one message at a time per connection,
with explicit JSON state (16 KiB) passed in and returned. `ws_send` queues a text
or binary message (`Ok` means queued, not delivered); `ws_close` closes after
queued messages. Optional `on_open` and `on_close` hooks are supported.
A callback not finished within `callback_timeout_ms` (default 30 s, counted
from when the message is queued) closes the connection with 1011. A running
callback is not interrupted; `on_close` is skipped if it is still running once
the connection has closed, so the two never overlap. Registration is setup-only in `ntnt run` and `ntnt test`.

Origins are matched exactly: the `origins` option, else the origin of `SITE_URL`,
else localhost in development. Production startup fails without one of them.
Each connection has a bounded send queue charged to the realtime payload budget
and a separately bounded receive backlog (4 messages / 256 KiB). A process-wide
callback queue (64) and, with more than one configured worker, a limit of all but
one worker running callbacks limit how far sockets can delay requests. The limit
uses the configured worker count: while a worker is down and restarting, the
remaining workers can all be running callbacks. With a single worker, a long
callback still delays HTTP. Shutdown closes sockets with 1001.
`ntnt test` returns 501 for `websocket()`.

`websocket(endpoint, map { "source": bus })` forwards each later `publish()` as a
text message with the published JSON value, encoded once and shared with SSE.
A socket that falls behind the bus capacity is closed with 1013.

Connections have a finite lifetime: `max_lifetime_ms` (default 5 minutes, at
most 24 hours, never unlimited) closes them with 1001 so clients reconnect
through middleware and auth again. A development hot reload (including a route
file edited in place) closes open sockets with 1012 so old connection state never
runs against new code: their queued messages and `on_close` are skipped. Existing
HTTP and SSE behavior is unchanged.

## DD-041: exact loaded-source provenance

The runtime records the actual file input parsed by each
interpreter, including main source, ordinary/transitive imports and direct
libs/middleware/routes/jobs loaders. Source identity no longer needs to be guessed
from mtimes or a later read of possibly replaced files. A cache hit retains the
original loaded bytes' digest. Path resolution is shared with the corresponding
read/parser origin rather than resolving a symlink again after reading.

For Rust embedding, `source::SourceFile::read(...).parse()` yields immutable
`ParsedSource` input for `Interpreter::eval_source`. `source_snapshot()` returns
an ordered, length-delimited, platform-local source-only snapshot. Existing
`eval(&Program)`, public opaque block evaluation and AST construction/JSON remain
supported and unchanged;
a filename set independently of an AST does not attest to its input. Source-less
entries or multiple evaluated versions of one identity make attestation
unavailable, without disabling evaluation/reload. Readable virtual/pipe inputs
such as `/dev/stdin` remain executable but are not given regular-file attestation.
Per-file history retains one digest plus a conflict bit, not unbounded versions
or application source text. Host-injected callable values (including nested
bindings/arguments) also make snapshots unavailable. Callable values exported to
Rust through getters, file results or named-call results are treated conservatively
because their public closure environments can be mutated outside the interpreter.
Ordinary host data and tracked file-backed calls remain supported without losing
provenance. No existing embedding operation is rejected by this tracking.

This is a prerequisite, **not** WebSocket support, full program-generation
compatibility, worker quarantine, per-application realtime isolation, or a new
worker-admission check. Imports remain lazy; snapshots describe only inputs
observed so far and are not inherited by captured tasks. No new NTNT syntax,
stdlib API, mandatory configuration, dependency, or deployment is introduced.

## DD-041: realtime workers run the same code

Apps using realtime (a setup `broadcast` bus or an explicit `realtime` map) now
check that every HTTP worker loaded exactly the primary's source, compared where
setup first reaches `listen`. Workers that loaded different files fail startup
before the listener binds. A worker that restarts later with changed files on
disk stays out of service until its source matches the running generation again.
Buffered-only apps are
unchanged. Endpoint configuration and lazy imports after `listen` are not yet
part of the check.

## Database migrations (`ntnt db`) on real PostgreSQL applications

Tested by adopting and migrating Jarri's staging database (33 migrations, about 11 GB).

- **`public` can be the managed schema.** It was previously rejected, although most applications keep their tables there.
- **Complete migration scripts replay and run.** `DO` blocks, functions, triggers, policies, grants, seed rows and `SET CONSTRAINTS` are allowed. Statements that would end the migration transaction, change the session's role, run outside a transaction, act outside the database, or `CASCADE`-drop are still rejected. Statements NTNT cannot analyze in detail are reported in `analysis.unknowns` rather than blocking the plan.
- **More of the schema is compared.** Views, sequences, SQL and PL/pgSQL routines (definition, owner, grants), triggers, RLS policies, declarative partitions, expression/partial indexes and managed-schema extensions are compared by definition. A manual change is reported as drift on the named object instead of incomplete coverage. Extension functions are compared through the extension's version.
- **Replay matches the target.** Scripts run with normal row-level security, and scratch may bypass RLS only when the target role does. Each migration settles deferred constraints at its own boundary.
- **Locking is narrower.** `migrate` no longer takes `ACCESS EXCLUSIVE` locks on every managed table. An advisory lock serializes migrators, and each statement takes only the locks it needs under `lock_timeout = 1s`. Tables read by `requires`/`ensures` checks are `SHARE`-locked. A lock timeout or deadlock rolls back the migration and retries it with backoff, up to 4 attempts. Migration scripts get a 90-second statement limit. Each committed migration reports the relation locks it held and its `lock_retries`. On Jarri staging, the worst concurrent write stall during a small migration fell from 937 ms to 164 ms.
- **Plans predict locks.** Each pending PostgreSQL migration's `analysis.locks` lists the table locks each statement takes, using the same mode names `migrate` reports. `analysis.lock_warnings` flags plain `CREATE INDEX`, `SET NOT NULL`, constraints added without `NOT VALID`, column type changes and rewriting defaults on existing tables. Predictions are checked against the locks PostgreSQL actually holds in the test suite.
- **Concurrent index builds.** A migration that is a single named `CREATE INDEX CONCURRENTLY` runs outside a transaction, so writes continue while the index builds. A failed build's invalid index is dropped. `migrate` commits a `running` ledger entry before the build, and an interrupted build blocks all later `status`, `plan`, `verify` and `migrate` work with `DB_ATTEMPT_UNRESOLVED` until `ntnt db reconcile --plan <same plan>` records it (`completed`) or rolls it back (`rolled_back`, `not_started`), changing nothing it cannot explain.
- **Fixed:** `migrate` no longer re-locks every table after the last migration commits. On a live database, that extra lock pass could deadlock and report failure for a migration that had committed.

## Dependencies

- `pulldown-cmark` 0.13.6 fixes RUSTSEC-2026-0433 (quadratic parsing of nested image wikilinks). NTNT does not enable wikilinks, so `std/markdown` was not exposed.
