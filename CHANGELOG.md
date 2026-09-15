# Changelog

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
