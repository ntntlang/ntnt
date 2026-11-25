# Changelog

## Unreleased

### Fixed

- `ntnt db` (SQLite): migrations can use `IS NOT` in a trigger, `GLOB` in a
  `CHECK`, and `STRICT, WITHOUT ROWID` on the same table (#426).
- `ntnt db` (PostgreSQL): recheck incoming foreign keys after the migration SQL
  runs. A newly visible key from an untracked table rolls back the migration
  instead of letting it commit changes outside the approved scope (#447).

## 0.6.5

See the [versioned announcement and upgrade guidance](docs/release-notes/v0.6.5.md).

### Security

- Update `cookie` to 0.18.3 for RUSTSEC-2026-0594 (#456).

### Fixed

- `ntnt db` (PostgreSQL): a database restored with `pg_dump` and `pg_restore`
  verifies. The restore re-associates `AND` and `OR` expressions that
  `BETWEEN`, `NOT BETWEEN`, `IN` lists and row comparisons nest (in CHECK
  constraints, `EXCLUDE` and index predicates, index expressions, policies,
  defaults, generated columns, views, triggers' `WHEN`, domains, partition keys
  and SQL-standard routine bodies, also those naming an argument by position,
  `$1`), and sets `connoinherit` on partitions'
  PRIMARY KEY and UNIQUE constraints; inspection compares both as a restore
  leaves them, so neither is `DB_DRIFT`. Plans for histories with such values
  record `canonicalization_version` 2 and new fingerprints, so a new plan
  needs approving; other histories keep their fingerprints and plan digests,
  and plans saved by earlier versions still migrate (#460).
- `ntnt db` (PostgreSQL, review follow-ups): a check that compares a value it
  computes (`id + 1 = 2`, `count(*) = 0`) is refused, like a direct column
  comparison, when an operator outside `pg_catalog` could take the value's type;
  unquoted names in checks fold like PostgreSQL's (ASCII letters only in UTF-8),
  so a citext column named `Émail` is screened; checks may read columns of types
  the history created outside the managed scope; a domain whose constraints use
  an uninspected operator outside the scope is uncovered; only PostgreSQL's
  row-security refusal of a check's read is reported as `DB_CHECK_ROW_SECURITY`;
  `DB_EXTENSION_PRIVILEGE` names no statement for `CREATE EXTENSION ... VERSION`;
  declared-extension drift lists a relation under the extension that owns it.
- `ntnt db` (SQLite, review follow-ups): `sqlite_rebuild` also refuses a
  rebuild that lowers or removes another table's AUTOINCREMENT counter, and
  replay checks deferred foreign keys at the end of each migration, so `plan`
  refuses what the target's COMMIT would.
- `ntnt db` (PostgreSQL): enums, domains and composite types plan, migrate and
  verify, in the managed schema and in schemas the history creates; a change
  to one outside a migration is drift (#430).
- `ntnt db` (PostgreSQL): a trusted extension installed in the managed schema,
  such as pg_trgm or citext, covers its types, operators, operator classes and
  casts by its version, so indexes and columns that use them plan and migrate,
  and citext's casts no longer block later migrations; an out-of-band
  `ALTER EXTENSION ... UPDATE` is drift (#430).
- `ntnt db` (PostgreSQL): overloaded operators and operator classes with one
  name in two access methods no longer collide as catalog keys, and
  `DB_CATALOG_AMBIGUOUS` names the shared key and the migration (#430).
- `ntnt db` (PostgreSQL): `EXCLUDE` constraints plan, migrate and verify (#430).
- `ntnt db` (PostgreSQL): replay commits each migration on scratch in a
  transaction set up like the target's, instead of one rollback-only
  transaction, so an enum value added by one migration can be used by the next
  on PostgreSQL 16, and adding and using one in the same migration fails at
  plan with `DB_ENUM_VALUE_UNCOMMITTED` instead of at `migrate` on 17 and 18
  (#430).
- `ntnt db` (PostgreSQL): replay cleans scratch up afterwards, whatever its
  outcome: it commits a marker schema `_ntnt_replay` first, then drops what the
  scratch role owns (`DROP OWNED`), restores `public` and checks scratch is
  empty again (`DB_SCRATCH_CLEANUP` if not). A later plan cleans up the
  leftovers of a killed replay (`scratch.recovered`) only when it can prove
  they are that replay's: the marker was written in this database of this
  cluster, scratch holds no `_ntnt` ledger, only the scratch role owns objects
  there, and every catalog row there was last written by one of the
  transactions the marker records. Otherwise `DB_SCRATCH_NOT_EMPTY`, dropping
  nothing. A target that holds the marker is refused with
  `DB_TARGET_IS_SCRATCH` (#430).
- `ntnt db` (PostgreSQL): replay stops at its own deadline, cancels the
  statement it was running and still cleans scratch up within 30 more seconds;
  `scratch.cleanup` is `not_needed` when replay committed nothing, and cleanup
  drops nothing without the replay lock (#430).
- `ntnt db` (PostgreSQL): scratch cleanup keeps to its documented limits. Each
  of its locks waits at most 2 seconds, `DROP OWNED` runs at most 10 and the
  whole cleanup fits its 30 seconds, so a session holding a lock on a replayed
  table no longer holds a plan up to 90 seconds. Cleanup then fails with
  `DB_SCRATCH_CLEANUP` naming the sessions that hold locks (PID and role),
  the marker stays, and the next plan recovers once they are gone (#430).
- `ntnt db` (PostgreSQL): plans, statuses and verifies on one scratch database
  are serialized by an advisory lock; a second one waits up to
  `migrator_wait_seconds`, then fails with `DB_SCRATCH_BUSY`. A migration that
  releases that lock is refused, and a killed replay's leftovers are not cleaned
  up while another session of the scratch role is connected (`DB_SCRATCH_BUSY`)
  (#430).
- `ntnt db` (PostgreSQL): replay and scratch cleanup, from the 0.6.5 review.
  A migration that swaps the replay lock for a transaction-level lock on the
  same key is refused before it commits, and the lock is checked again after
  each commit. Recovering a killed replay's leftovers also proves who wrote
  the rows of the scratch role's tables: replay records each migration's
  transaction and those of its subtransactions that wrote rows, recovery locks
  those tables before it reads them, and a row another session wrote there
  refuses recovery (`DB_SCRATCH_NOT_EMPTY`, naming the table) and drops
  nothing. A marker whose transaction list holds a NULL or whose settings are
  not a snapshot is not NTNT's and is never cleaned. Refused before they
  commit (`DB_REPLAY_UNSUPPORTED`): a schema another role owns when the
  migration dropped `public` (cleanup could not drop it), a change of the
  scratch database's or `public`'s privileges while they hold a grant another
  role made with a grant option (cleanup grants as the owner, so it could not
  restore it), and `local_preload_libraries` or `session_preload_libraries`
  set for the scratch database or role (a library that does not load refuses
  every connection, so a replay stopped before cleanup could not be
  recovered). Cleanup restores setting values as escape string literals, and
  after a failed cleanup the advice to drop or recreate scratch applies only to
  a database nothing else uses. A failure to release the replay lock after
  cleanup finished is reported beside it, and no longer turns a finished
  cleanup into `unknown` or drops the replay's results; NTNT closes the scratch
  session, which releases the lock.
- `ntnt db` (PostgreSQL): cleanup restores the scratch database's privileges,
  connection limit, comment and settings, and the scratch role's settings, so
  histories that change them in a `DO` block still plan. Replay refuses, before
  they commit, changes cleanup cannot undo: passwords (also one an exception
  block rolled back), memberships, other databases, objects given to another
  role and a session left as another role (`DB_REPLAY_UNSUPPORTED`). So are a
  connection limit of 0 on scratch and a `REVOKE CONNECT` (from the scratch
  role, or from `PUBLIC` when the role connects through it) that leaves the
  scratch role unable to connect: cleanup would restore them, but a replay
  stopped before cleanup would leave scratch unrecoverable. `_ntnt_replay`, like
  `_ntnt`, cannot be the managed schema (`DB_CONFIG` names it as reserved). Scratch now also requires
  that its role hold no privileges on objects it does not own, other than on
  `public`, and `track_counts` on (`DB_SCRATCH_UNSAFE`), and that it own nothing
  in scratch but `public` before replay, such as a large object or a
  publication (`DB_SCRATCH_NOT_EMPTY`) (#430).
- `ntnt db` (PostgreSQL): `capabilities.operations.transactional_script_replay`
  is now `per_migration_commits_on_scratch` (was `rollback_only_scratch`).
  Capabilities feed no digest, so saved plans, plan digests and fingerprints of
  accepted histories are unchanged (#430).
- `ntnt db` (PostgreSQL): `requires`, `ensures` and adoption checks can read
  tables with columns of these types, and run while an extension's casts exist.
  They still run no routine the history wrote: `DB_CHECK_UNSUPPORTED` names a
  column whose type holds a domain below its top level (an array's element, a
  composite's attribute) whose constraint calls one or a volatile function,
  and a view that converts values to such a domain or uses such an operator
  (#430).
- `ntnt db` (PostgreSQL): a check that compares a column with an operator
  the application would resolve differently, such as `email = 'a@b'` on a
  `citext` column (checks resolve operators with `search_path=pg_catalog`, so
  it compared case-sensitive text and could pass where the application's
  comparison fails), is refused with `DB_CHECK_UNSUPPORTED` naming the check,
  table, column, type and operator before any check runs, and `plan` lists it
  in `plan.check_warnings` (#456).
- `ntnt db` (PostgreSQL): a check that reads a view calling a builtin that
  writes or takes locks (`nextval`, `setval`, advisory locks and the other
  builtins PostgreSQL marks parallel unsafe) is refused with
  `DB_CHECK_UNSUPPORTED` naming the check, view and function before it runs;
  `random()`, `clock_timestamp()` and `gen_random_uuid()` stay allowed (#456).
- `ntnt db` (PostgreSQL): a statement that uses an enum value added by
  `ALTER TYPE ... ADD VALUE` in the same transaction fails with
  `DB_ENUM_VALUE_UNCOMMITTED` naming the migration and statements, in replay
  and in `migrate`, instead of an opaque `DB_POSTGRES` SQLSTATE 55P04 (#456).
- `ntnt db` (PostgreSQL): a `requires`, `ensures` or adoption check that reads a
  table whose row-level security applies to the migration role, such as one with
  `FORCE ROW LEVEL SECURITY`, is listed in `plan.check_warnings`
  (`CHECK_ROW_SECURITY`), and `migrate` refuses it with `DB_CHECK_ROW_SECURITY`,
  naming the check, its phase and the table, and saying the role needs
  `BYPASSRLS`. It used to fail with a bare SQLSTATE 42501 after `plan` passed
  (#430).
- `ntnt db` (PostgreSQL): creating an untrusted extension such as `postgres_fdw`
  fails with `DB_EXTENSION_PRIVILEGE`, naming the statement and the extension and
  explaining that roles that are not superusers can create only trusted
  extensions. It used to fail with a bare SQLSTATE 42501 (#430).
- `ntnt db` (SQLite): migrations can call SQLite's built-in functions, the JSON
  functions, `hex` and `randomblob` included; `load_extension`, `sqlite_log`
  and extension or application functions stay refused, and the refusal says
  what a migration can use (#427).
- `ntnt db` (SQLite): AUTOINCREMENT tables plan, migrate and verify; the
  `sqlite_sequence` counters are outside coverage and drift (#427).
- `ntnt db` (SQLite): the planner statistics that `ANALYZE` and `PRAGMA
  optimize` write are outside coverage and drift, and a migration can run
  `ANALYZE` (#427).
- `ntnt db` (SQLite): `ALTER TABLE ... ADD COLUMN` on a STRICT table or with a
  CHECK constraint plans, and reads that name no column (`count(*) FROM t`)
  and `WITH RECURSIVE` are allowed (#427).
- `ntnt db` (SQLite): a `sqlite_rebuild` must keep the rebuilt table's
  AUTOINCREMENT counter in one `sqlite_sequence` row, so deleted ids are
  never reused; the refusal names the table and both values (or the number of
  rows), and gives the `UPDATE`, `DELETE` and `INSERT` lines that fix it
  (#427).
- `ntnt db` (SQLite): `DB_REBUILD_GUARD` names the rule and object a rebuild
  broke (a row count with both values, a missing or changed index, trigger or
  view, a changed table) and lists them in `objects` (#427).
- `ntnt db` (SQLite): an application function in DML is `DB_SQL_UNSUPPORTED`
  naming the function; `ADD COLUMN ... UNIQUE` reports `Cannot add a UNIQUE
  column` and suggests a unique index; ADD COLUMN defaults that fail on rows,
  and drops of referenced tables without the sidecar, get a `next_action`
  that says what to write instead (#427).
- `ntnt db`: `migrate --plan` with a saved plan that holds no plan (outcome
  `fail`, no `plan_digest`) says the plan is blocked instead of edited (#427).

### Added

- `ntnt db` (PostgreSQL): `[databases.<name>] extensions = [...]` declares
  extensions a superuser installs in both the target and the scratch database,
  in any schema but `_ntnt` and PostgreSQL's, `public` included. Each is compared
  by version and schema, scratch may hold it and what it owns, and migrations may
  keep `CREATE EXTENSION IF NOT EXISTS` for it. An undeclared extension outside
  the managed scope is still uncovered, and `DB_COVERAGE_INCOMPLETE` now says to
  declare it. Replay's cleanup and its recovery of an unfinished replay keep the
  declared extensions; a privilege a DBA grants the scratch role on what they
  install is refused by name (`DB_SCRATCH_UNSAFE`), since cleanup would revoke
  it, and the diagnostic says to grant it to `PUBLIC` in scratch instead.
  Manifests without the key keep their plan digests (#430).
- `ntnt db` (PostgreSQL): a foreign server a migration creates is covered as
  `server:<name>` (wrapper, owner, privileges and options, redacted to a
  digest), so an out-of-band `ALTER SERVER ... OPTIONS` or `OWNER TO` on the
  target is drift; a server no migration creates, and a foreign-data wrapper no
  extension installed, are uncovered by name instead of ignored. Histories
  without servers keep their digests (#430).
- `ntnt db` (PostgreSQL): `CREATE`, `ALTER` and `DROP USER MAPPING` are refused
  with their own reason (`USER_MAPPING`: credentials bound to a role) and a
  next action (a DBA step outside the history, credentials in the secrets
  provider) instead of as role DDL; user mappings are not inspected (#430).
- `ntnt db` (PostgreSQL): an undeclared extension in scratch is named in
  `DB_SCRATCH_NOT_EMPTY` (`extension:hstore`) with what to do, and a declared
  extension moved to another schema is one drift entry, its members listed
  under it (#430).

### Changed

- `ntnt db` (PostgreSQL): the `capabilities.inspection` keys of the JSON
  reports changed, with `format_version` still 1:
  `custom_types_external_dependencies_non_sql_routines` (`unknown`) is
  replaced by `enums_domains_composite_types_operators_exclusion_constraints`
  (`definition_fingerprints`) and
  `range_and_base_types_external_dependencies_non_sql_routines` (`unknown`),
  and `managed_schema_extensions` changed from `version` to
  `version_covers_members` (#430).

## 0.6.4

See the [versioned announcement and upgrade guidance](docs/release-notes/v0.6.4.md).

### Security

- Update `url` to 2.5.9 for RUSTSEC-2026-0592 (#450).
- Keep exclusive access to a SQLite target until the verified old ledger is dropped (#451).

### Fixed

- `ntnt db` (SQLite): migrations can use the date and time functions, so timestamp
  defaults plan and migrate; a statement SQLite refuses names its number and the
  function or table it used (#414).
- `ntnt db` (SQLite): an empty scratch takes the target's journal mode, and a
  scratch settings mismatch names each setting with both values (#413).
- `ntnt db` (SQLite): partial and expression indexes plan, migrate and verify; a
  changed predicate or expression is drift (#411).
- `ntnt db`: `DB_COVERAGE_INCOMPLETE` names the uncovered objects (`objects`) and
  the migration whose replay produced them (`source`) (#411).
- `ntnt db new` holds the `ntnt.toml` lock while it numbers a migration, so
  concurrent runs cannot take the same ID; a run that finds the lock held fails
  with the new `DB_NEW_CONFLICT` and creates nothing (#388).
- `ntnt db` (SQLite): a drop that would run ON DELETE actions on child tables is
  refused with DB_REBUILD_REQUIRED unless the migration uses sqlite_rebuild
  (#419).
- `ntnt db` (SQLite): migrations on a target run under `statement_seconds` (90 s
  by default) instead of a fixed 5 s; DB_STATEMENT_LIMIT names the statement
  (#429).
- `ntnt db`: a migration with no statements fails with DB_MIGRATION_EMPTY
  (#428).
- `ntnt db` (PostgreSQL): inspection covers the schemas the history creates and
  the objects it creates in other schemas; statements outside the managed scope
  run as written and are flagged in the plan (OUTSIDE_MANAGED_SCOPE);
  DB_OUTSIDE_SCOPE refuses only changes to `_ntnt` and system schemas;
  DB_TARGET_CHANGED reports a target that changed between verify's two reads
  (#420).
- `ntnt db` (PostgreSQL): a foreign key from an untracked table into a table the
  history creates outside the managed schema blocks execution with
  DB_EXTERNAL_DEPENDENCY (#420).
- `ntnt db` (SQLite): replay inspection reuses unchanged work, so long
  histories, including ones dense in foreign keys, plan in seconds; SQLite
  operations get an 8 MiB stack (#421).
- `ntnt db` (PostgreSQL): contract checks are refused before they run when a view
  they can read reaches a non-builtin or volatile function through views in
  another schema (#420).
- `ntnt db` (PostgreSQL): a policy that calls a routine outside the managed scope
  is uncovered (#434).
- `ntnt db`: refusals name the file, secret, migration or flag they are about and
  the cause, with advice for that cause: `DB_INPUT_IO`, inventory and sidecar
  refusals, `DB_SECRET`, `DB_FILE_IDENTITY`, unknown `--target` and `--database`
  values, and `DB_ADOPTION_INPUT` (#422).
- `ntnt db`: dotfiles and `README` files in the migrations directory are ignored
  (#422).
- `ntnt db`: history refusals name the migration and how to restore it, and
  `DB_PLAN_STALE` names the binding that changed (#423).
- `ntnt db`: `DB_APPROVAL_REQUIRED` says how to approve the plan, or why the
  approval secret cannot be read; `DB_OUTPUT` names the file, an `--out` that
  exists, has no directory or passes through a file is refused before any
  work, and an incomplete report is removed (#425).
- `ntnt db`: each subcommand's `--help` lists only the flags it takes (#432).
- `ntnt db` (SQLite): failures name the result code and, where it holds only
  names, SQLite's message; an unwritable file is named by role and secret; the
  parser gate names the statement and the cause; `DB_BUSY` says who holds the
  lock and that nothing was committed (#424, #425).
- `ntnt db` (SQLite): an empty scratch also takes the target's page size,
  encoding and auto-vacuum (#425).
- `ntnt db`: an adoption plan refused because the target holds tables the history
  does not create names them, and says how to retire an application's previous
  migration ledger (#431).
- `ntnt db new` pads a new migration's ID to the width of the widest existing ID,
  or four digits in an empty directory (#431).
- `ntnt run`: a runtime error raised in an imported module names that module's
  file and line, shows its source, and lists the call sites that led there
  (#412).

## 0.6.3

See the [versioned announcement and upgrade guidance](docs/release-notes/v0.6.3.md).

### Security

- `std/auth`: `require_auth()` and protected-path rules cover application handlers
  under the auth route prefix; only std/auth's own handlers there stay public. Auth
  matches the canonical path the router serves, so repeated slashes no longer
  bypass a rule.
- `std/auth`: protection rules stay in force while other workers start, restart or
  reload; while a server serves, `enable_auth()` adds rules instead of replacing them.
- Process-execution and `fetch()` network policy come from the environment that
  starts `ntnt`, captured before application code runs; `load_env` skips reserved
  policy keys with a warning.
- `fetch()` and `download()` block cloud metadata endpoints in any spelling and after
  DNS resolution, even when private or localhost addresses are allowed.
- `to_html_safe()` drops links and images whose scheme is not http, https or mailto,
  and ignores heading attributes.
- `std/email`: `lettre` 0.11.24 fixes six SMTP advisories (RUSTSEC-2026-0560 to
  RUSTSEC-2026-0565): STARTTLS response injection, SMTP command injection through
  missing dot-stuffing, delivery to a different address for quoted local parts,
  unbounded reply buffering, and two panics.
- `std/db/postgres` and the std/auth store use TLS for PostgreSQL: `sslmode`
  `verify-full`, `verify-ca`, `require` and `disable` are honoured, `prefer` and
  `allow` are rejected, and a connection string without `sslmode` uses verified TLS
  unless every host is local (loopback, a Unix socket, or a single-label name such
  as a Docker service) and it names no CA with `sslrootcert`.
- PostgreSQL TLS handshakes are bounded by `connect_timeout` (10 seconds when
  unset), and a target given only as `hostaddr=` is verified against its IP
  address.
- `std/kv`, `std/jobs` stores and the std/auth session store accept `rediss://` and
  `valkeys://` with verified TLS.
- Every Redis/Valkey connection bounds each wait for the server during the TCP
  connect, TLS handshake and setup commands (10 seconds for `std/kv` `open()`, job
  stores and the auth session store); one auth-store operation's connect no longer
  holds up the others.
- The `${...}` interpolation warning scans each string literal in one bounded pass.
- `install.ps1` verifies the release checksum before extracting and keeps the
  existing `ntnt.exe` until the new one runs; it no longer builds from source when
  the download fails, and never reuses or overwrites `.\ntnt`.

### Fixed

- `ntnt test` waits until the app's server is listening before sending requests,
  so slow-starting apps no longer fail their first requests; `--port 0` picks a free
  port.
- The Windows release's `.sha256` file ends its line with LF, so macOS
  `shasum -a 256 -c` accepts it.

## 0.6.2

See the [versioned announcement and upgrade guidance](docs/release-notes/v0.6.2.md).

### Added

- `ntnt db init` declares a SQLite or PostgreSQL database and its secret names in
  `ntnt.toml` and creates `migrations/`; `ntnt db new <name>` creates the next
  numbered migration file. Neither writes secret values or overwrites a file (#348).
- `examples/database-dev`: tested local PostgreSQL (Docker Compose) and SQLite
  setups for `ntnt db`, with matching scratch databases (#345).
- `ntnt db` PostgreSQL TLS: `sslmode=verify-full` (host-store or `sslrootcert`
  trust), `verify-ca` (`sslrootcert` CA file only) and `require`, no plaintext
  fallback, and the transport bound into plans (#344).
- PostgreSQL `[databases.<name>.limits]` for `ntnt db`: configurable
  `statement_seconds` (scripts and concurrent index builds) and
  `migrator_wait_seconds`, bounded, bound into plans, with deadlines that fit
  them (#346).

### Changed

- `ntnt db` plans bind the PostgreSQL transport, so `migrate` needs a plan made by
  0.6.2: regenerate plans made by 0.6.1 (#344).
- A competing `ntnt db` migrator fails with `DB_LOCK_CONFLICT` instead of a
  statement timeout (#346).

### Fixed

- On macOS, `std/process` `run` and `start` could fail with `failed to configure
  process monitoring: No such process` when the command exited before NTNT began
  watching it. Such a child is now treated as exited (#381).

## 0.6.1

See the [versioned announcement and upgrade guidance](docs/release-notes/v0.6.1.md).

### Added

- Native WebSocket endpoints, explicit connection state, worker callbacks, shared
  broadcast sources, bounded queues, exact-Origin policy and finite lifetimes.
- Exact loaded-source snapshots and realtime startup source parity, with
  different-source restarted workers kept out of service.
- Broader PostgreSQL migration replay/catalog coverage, lock predictions and
  reporting, and guarded concurrent-index execution/reconciliation.

### Fixed

- Restore caller scope after match, invariant and `otherwise` evaluation errors;
  report integer division/remainder overflow and zero divisors without panicking.
- Count imports referenced by `otherwise` recovery blocks as used.
- Reclaim unreachable closure/environment cycles, including imported modules in
  completed tasks and local closures in persistent interpreters (#183). Escaped
  callbacks retain their lexical state until their final reachable owner releases it.
- Avoid lost worker wakeups across HTTP/callback queues; retain the full route
  prelude when reloading a route file edited in place.
- Respect static-mount path boundaries and serve async `.mjs` as JavaScript.
- Release WebSocket broadcast subscriptions before slow close teardown, report
  oversized broadcast delivery with 1009, and preserve complete finite upgrade
  refusals through proxies without disabling ordinary HTTP compression.
- Remove the final migration re-lock pass; allow already-current PostgreSQL auth
  schema startup with runtime DML privileges rather than redundant DDL.

### Native embedding

- `Value::Function::closure` now uses an Rc-compatible `Closure` release-notification
  wrapper. Native Rust constructors can convert their existing environment Rc with
  `.into()`; borrowing and Rc identity checks continue through `Deref`. NTNT source
  syntax and capture semantics are unchanged.

### Security

- Update `pulldown-cmark` to 0.13.6, fixing nested-wikilink parser denial of service
  (RUSTSEC-2026-0433).

## 0.6.0

See the [release announcement and upgrade guidance](docs/release-notes/v0.6.0.md).
The 0.5.7 version was used during development only; the previous published release
is 0.5.6.

### Added

- SQL-first `ntnt db status|plan|migrate|verify` for PostgreSQL and SQLite: immutable
  history, catalog drift, isolated replay, separately approved exact plans, named
  transition checks and committed-prefix/uncertain-commit recovery (#245).
- Explicit existing-database adoption and recognized PostgreSQL v1 history import,
  with distinct adopted/imported evidence rather than fictional applied rows.
- Native SSE through `broadcast`, `publish` and `sse`, shared across HTTP workers
  and in-process tasks, with bounded resources and explicit slow-client disconnects.
- Opt-in automatic HTTP rate limiting, per-route overrides, application-defined
  `check_limit` keys, and trusted `req.client_ip` resolution (#294).
- Optional `serve_static` extension allowlists. Configured mounts reject unlisted,
  extensionless and dotfiles, and re-check resolved symlink targets; default behavior
  is unchanged.

### Changed

- Explicit `./` and `../` imports retain their declaring-file origin through calls,
  callbacks, tasks and jobs. Bare imports remain CWD-relative (#296).
- Context-dependent builtins resolve through ordinary function bindings, including
  aliases and shadowing. Lint tracks imports that hide route functions (#293).
- Realtime listeners wait for initial worker setup before bind/readiness, with
  bounded startup failure and source-path diagnostics. Buffered-only startup is unchanged.

### Fixed

- Bound async graceful shutdown drain by the request timeout before continuing shutdown.
- Allow job enqueue from secondary HTTP-worker request handlers while retaining
  bootstrap suppression.

### Security

- Upgrade `pulldown-cmark` to 0.13.5 for RUSTSEC-2026-0414 and require the patched
  minimum. NTNT did not enable the affected wikilink option; rendering options are unchanged.

## 0.5.6

See the [complete release notes](docs/release-notes-v0.5.6.md) for upgrade guidance. TLS inspection, monitoring HTTPS probes and SMTP TLS now require a usable host/container CA store; update the NTNT binary for the security fixes, and restart after subsequent CA-only updates.

### Added

- `std/auth.totp_qr_svg(uri)` renders a validated TOTP enrollment URI as a local, deterministic QR SVG without sending enrollment secrets to an external service. The SVG remains secret-bearing; show it only during enrollment, never log or cache it (#283).
- `std/concurrent.task_stats()` returns bounded process-local task counts and estimated retained result/history bytes without exposing task contents or changing retention. Includes typed integer fields for lint and no new configuration (#267).
- Task closures passed to `spawn`, `after`, `schedule`, `parallel` and `race` can call your own NTNT functions, from the same file or imported, including helpers those functions call, recursion and mutual recursion (#186). Each function is copied into the task with the values it uses, so changes stay inside the task. Values that can't be copied still fail before the task starts, and the error now names the dependency path, e.g. `task -> outer -> call_a -> handlers`. Struct invariants are now enforced inside tasks when they read only the struct's own fields; a rule that calls a helper or reads a constant fails with a clear error when that struct is built in a task (previously tasks skipped invariants entirely).

### Security

- Updated `ctutils` to 0.4.4 and Tokio to 1.53.3 for RUSTSEC-2026-0385 (non-constant-time masks/comparisons) and RUSTSEC-2026-0386 (Unix `AsyncFd` I/O safety). Tokio's minimum is now 1.53.3 to avoid affected version lines in future resolutions. Rebuild NTNT binaries to obtain the fixes; TLS backends are unchanged.

- Updated Rust dependencies to resolve the currently reported RustSec advisories, including the older HTTP/2 and IDNA dependencies pulled in by reqwest 0.11. Migrated the DNS, JWT, and randomness adapters while retaining their NTNT APIs. JWT uses an explicit AWS-LC crypto provider; HTTP clients retain native/system TLS and the monitoring/SMTP clients retain ring-based Rust TLS. Rebuild NTNT binaries to obtain these dependency fixes.

- Updated the TLS certificate verifier to distrust compromised eMudhra intermediate keys, even when a host still trusts the issuing roots. Rebuild NTNT binaries to obtain the verifier fix for `std/net.tls_info`, `std/http.probe_fetch`, and SMTP TLS. Maintain the operating-system trust store (including container images) separately; root-only changes need a process restart, not a new binary. See the [incident advisory](https://emudhra.we-have-a-problem.com/) for platform-specific patched versions and test hosts.

### Fixed

- A function call that fails now restores its caller's state on every path (#183, part 1). Previously an error in a function body, default argument, destructured parameter, `requires`, `old()` capture or `ensures` left the interpreter in the callee's scope. A caller's `otherwise` handler could then lose its own variables (`Undefined variable`) and see the callee's.
- Each function call has its own `old()` context. Nested calls no longer wipe the caller's snapshot, so a correct `ensures result == old(x) + 1` no longer fails after a nested call. Callees no longer see the caller's snapshot.
- Task threads now have a 16 MiB stack instead of Rust's 2 MiB default. In release builds a task overflowed its stack in fewer than 200 nested calls, below the default recursion limit of 256, and the overflow aborted the whole process. Tasks now have about 950 calls of headroom, so they reach the normal "Maximum recursion depth" error. Debug builds overflow much earlier on every thread, including the main one.
- Copied task scopes are released even when evaluation panics, so a failed helper task cannot keep channel senders alive through its copied closure cycles. Dependency-path storage is linear in the captured names; full paths are formatted only for rejected captures.

### Changed

- `tls_info`, HTTPS `probe_fetch`, and SMTP TLS now use host CA roots instead of roots compiled into NTNT. No bundled fallback remains. Install/maintain the host or container CA store and restart NTNT after CA updates; root-only changes no longer require a new NTNT binary. Rustls/ring, hostname checking, and compromised-key protection are retained. Explicit `SSL_CERT_FILE` / `SSL_CERT_DIR` overrides are supported for these Rustls clients.
- A failing function's `defer` statements run when that function exits, in its own scope, before the caller's `otherwise` handler. Previously they ran later, in the wrong scope, or not at all. This includes errors that escape to the top level.
- `old()` read in a callee's defaults or `requires` now refers to the callee's own values, not the caller's snapshot. A caller reading `old(x)` in its body after a nested call now sees its real starting value.

## 0.5.5

See the [complete release notes](docs/release-notes-v0.5.5.md) for upgrade guidance. The Redis job backend now requires Redis 7.0+ or Valkey 7.2+, and workers sharing a queue must be upgraded together.

### Added

- `int_or(value, fallback)`: integer conversion that returns a fallback instead of a `Result`.
- Optional start deadlines for `ping`, `snmp_get` and `dns_lookup`, and a bounded monitoring `std/http.probe_fetch`. See [probe start deadlines](docs/probe-start-deadlines.md).
- Linux ARMv7 hard-float release archives; the Unix installer accepts `--version` / `NTNT_VERSION`.
- IAL `header {name} exists` and `header {name} equals {value}` resolve and execute in intent checks.
- Several `ntnt worker` processes can now share one project and worker group on a machine. Each process takes its own numbered control endpoint, and `ntnt workers` commands reach every live process in the group. `scale` applies per process, and `status` prints one table per process. An explicit `--control-socket` path is still single-owner. See [worker control](docs/worker-control.md#several-processes-in-one-group).

### Changed

- **Breaking:** the Redis job backend now requires Redis 7.0+ or Valkey 7.2+. Job state changes commit through one Lua script that re-checks the values it read and the credential's permission for every write before writing anything. Workers refuse to start on an older server. Redis credentials need EVAL, EVALSHA and SCRIPT; WATCH, MULTI and EXEC are no longer used. Stop all workers and upgrade them together.
- Redis job throughput: each state change is one read round trip plus one commit, and claims read 32 ready-index entries instead of 256.

### Fixed

- OAuth logins through the built-in auth routes in server mode no longer return 404 on the provider callback. Server mode registers the routes `std/auth` lists, under `route_prefix`; `/auth/health` works, and unknown providers return 404 instead of 500.
- Every `enable_auth` call form accepts the same options. Local-only auth (no OAuth provider), the preset form, and keys such as `cookie_same_site` and `route_prefix` work with one or two arguments.
- Windows DNS lookups retry once on an OS-assigned UDP port when Windows refuses the resolver's random port.
- The Unix-socket secrets provider reads a complete response from an agent that already closed the connection (macOS).
- Parse errors at string literals report the opening quote's column and show the literal as written.
- Redis workers no longer scan every pending key per claim, and scale across processes instead of slowing down.
- A job's lease keeper now retries a renewal that hit momentary store contention after 25ms instead of waiting a full renewal interval (a third of the lease). Before, a few unlucky collisions in a row could let a running job's lease expire.
- SQLite job state changes (claim, renew, recovery) no longer fail with a generic `job state storage operation failed` when another worker slot or lease keeper briefly holds the store, or another connection holds the write lock. These are now reported as retryable contention (`local_busy` / `busy`), which job recovery already retries.
- On macOS, a `std/process` run that timed out could fail with `failed to clean up process descendants: failed to inspect macOS process <pid>` instead of returning a timed-out result. A process that is partway through exiting can briefly be impossible to inspect; the check now retries for up to 250ms before failing.

## 0.5.4

See the [complete release notes](docs/release-notes-v0.5.4.md) for compatibility changes, storage requirements, and coordinated worker-upgrade guidance.

### Added

- Crash-recoverable durable job claims, renewable fenced execution leases, and inspectable `outcome_unknown` states. Only provably unstarted work is automatically requeued; general reconciliation remains follow-up work.
- Terminal job-history expiry through KV TTL: 30 days for completed/cancelled records and 90 days for dead/failed/expired records by default. No cleanup jobs or legacy-history backfill.
- Bounded process-local task-result/history retention and structured-concurrency cleanup.
- Owner-local persistent ICMP probe handles through `ping_open`, `ping_probe`, and `ping_close`.
- Native binary crypto, owned filesystem/temp resources, atomic file publication, monotonic timing, bounded TCP listeners/readers, and HTTP listener fixture options.
- SQLite transaction-mode and busy-timeout options.

### Fixed

- PostgreSQL temporal String parameter encoding and bounded shared-pool lifecycle.
- HTTP worker task capabilities, pre-spawn rejection of nested opaque captures, worker control-socket ownership/state preservation, and finite persistent ICMP receive deadlines.
- Partial job-worker scale-up publication and state-persistence acknowledgement recovery without re-executing job bodies.

### Compatibility

- Safe HTTP redirects are now followed by default. Use `redirect: "manual"` for terminal 3xx behavior; see the [fetch migration guide](docs/migration-v0.5.4-fetch.md).
- Magic-link response padding now defaults to zero; set `generic_response_floor_ms: 1200` to retain the previous floor.
- Job workers require Redis 6.2+ or compatible Valkey for the Redis backend. Upgrade writers/workers sharing each queue together; mixed old/new claim protocols are unsupported. Recovery requires durable, non-evicting storage and does not provide universal exactly-once execution.
- Shared PostgreSQL pools default to a process-local cap of 32; `NTNT_POSTGRES_MAX_SHARED_POOLS` configures the positive bound before first connect.
- `NTNT_TASK_REMOVAL_TTL` now controls compact task-history age and defaults to 86400 seconds. Task handles are not permanent history records.

## 0.5.3

The next published release after v0.5.1; v0.5.2 was an unpublished development version. This entry includes that work. See the [complete release notes](docs/release-notes/v0.5.3.md) for upgrade guidance, capability boundaries, and known limitations.

### Security

- HTTP requests no longer follow redirects by default. Explicit `follow_redirects: true` opts into bounded manual following with per-hop target validation/DNS pinning, credential isolation, downgrade rejection, and conservative body replay rules; `max_redirects` defaults to 5 (1–10). Opaque Secret-bearing requests cannot opt in.
- SSRF-protected HTTP connections use the validated DNS addresses and bypass system proxies to prevent rebinding and proxy-side re-resolution.

### Added

- Native `.intent` tests invoke ordinary `.tnt` functions through glossary `call:` / `source:` bindings without a paired server, with typed results and fail-closed native assertion observation.
- Native CLI test cases run in supervised isolated processes with deadlines and cleanup; this milestone is synchronous and is not a security sandbox or project-wide runner.
- Explicitly imported, capability-gated `std/netmon` supports bounded SNMPv2c numeric GET and GETNEXT subtree walks with opaque community secrets and strict protocol/resource validation.

- Added `std/markdown.parse_blocks` for semantic Markdown blocks with exact UTF-8 byte ranges and source slices.
- Extended `std/http.download` with fetch-compatible request maps, streaming binary writes, safe file options, and atomic promotion.
- Added capability-gated `std/process` APIs for bounded commands and supervised long-running child processes.

### Compatibility

- The legacy `download(url, path)` form still creates parent directories, overwrites an existing destination, and preserves Unix regular-file permissions.
- Process execution is disabled unless `NTNT_PROCESS_ENABLE=1`; `NTNT_PROCESS_ALLOW` can restrict execution to canonical executable paths.
- Active `run` and `start` commands are reaped on runtime shutdown and direct CLI exits, started processes enforce deadlines and capture limits without caller polling, detached descendants cannot hold capture finalization open indefinitely, and completed-result retention is bounded to 64 handles and 64 MiB; Unix launches supervise descendant process groups, while Windows rejects implicit `.bat`/`.cmd` shell execution.
- Unix process-group cleanup now signals descendants before reaping the exited group leader, preventing a reused PID from redirecting cleanup to an unrelated group. Windows capture now uses cancellable overlapped reads so zero buffered bytes cannot hide pipe EOF.
