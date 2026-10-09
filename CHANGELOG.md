# Changelog

## Unreleased

### Added

- Task closures passed to `spawn`, `after`, `schedule`, `parallel` and `race` can call your own NTNT functions, from the same file or imported, including helpers those functions call, recursion and mutual recursion (#186). Each function is copied into the task with the values it uses, so changes stay inside the task. Values that can't be copied still fail before the task starts, and the error now names the dependency path, e.g. `task -> outer -> call_a -> handlers`.

### Fixed

- Task threads now have a 16 MiB stack instead of Rust's 2 MiB default. In release builds a task overflowed its stack in fewer than 200 nested calls, below the default recursion limit of 256, and the overflow aborted the whole process. Tasks now have about 950 calls of headroom, so they reach the normal "Maximum recursion depth" error. Debug builds overflow much earlier on every thread, including the main one.

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
