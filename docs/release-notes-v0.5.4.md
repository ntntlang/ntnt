# NTNT v0.5.4 — Durable job claims and native system primitives

This release adds crash-recoverable job claims, bounded task-result retention,
TTL-based job history, persistent ICMP probes, and native system primitives.
It also includes database and worker-lifecycle fixes. **Read the upgrade guidance
below before updating workers or applications from v0.5.3.**

## Native system APIs and HTTP behavior

- `std/http`: fetch, cache misses and both download forms now follow redirects by
  default. `redirect: "manual"` preserves terminal 3xx behavior; `"error"` refuses
  redirects. Boolean `follow_redirects` aliases remain supported. Default/error
  requests have a 30-second chain budget, configured response caps, and reject
  explicit Host/userinfo. Charset/BOM text decoding remains compatible. Same-origin
  Secrets may follow; crossing origins strips credentials and rejects body replay.
  Download status errors no longer reflect server bodies. These are intentional
  compatibility changes: see [Fetch migration notes](migration-v0.5.4-fetch.md).
- `std/crypto`: `sha384`, `sha384_bytes` accept exact UTF-8 or checked integer bytes;
  `base64_encode_bytes` produces standard padded base64. Existing SHA256 and string
  base64 valid results remain unchanged. SHA256 now rejects out-of-range integer
  array values that formerly truncated to bytes (an intentional validation correction).
- `std/fs`: `write_bytes`, `write_file_exclusive`, `mkdir_private`, `sync_file`,
  `sync_dir`, `file_permissions`, `chmod`, `chown`, `access`; `std/path.resolve_missing`.
  Unix exclusive creation requests 0600 and directory creation 0700 by default,
  restricted by the inherited umask. No process umask mutation or chmod-after-write
  fallback. Real-ID access is advisory; ownership, ACLs and trusted ancestors remain
  OS/caller concerns. Secure POSIX-only operations return `unsupported:` elsewhere.
- `std/net`: bounded opaque listener/stream APIs, literal loopback default, port 0,
  byte reads, complete writes, addresses, half-close and close. No outgoing socket
  API or outbound-policy bypass. Existing diagnostic `tcp_connect` is unchanged.
- Binary completion: raw standard/URL-safe Base64 decoders, URL-safe byte encoding,
  strict UTF-8 conversion, SHA512 hex/bytes, byte input for SHA256 bytes and HMAC,
  raw HMAC output and RustCrypto-backed `hmac_sha256_verify`.
- Process-local monotonic milliseconds and checked elapsed/deadline/remaining helpers;
  existing wall-clock helpers are unchanged.
- `write_file_atomic` publishes a same-parent temporary inode with optional durability;
  default private Unix creation honors umask. Explicit broader modes apply to staging
  too. Non-Unix supports sync:false without mode; other options fail before mutation.
  New-inode replacement does not preserve owner/ACL metadata and replaces terminal links.
  Atomic-write and temporary-resource path strings reject NUL before mutation; name prefixes reject
  separators, colon and NUL on every platform to prevent drive/stream reinterpretation.
- Opaque `temp_file`/`temp_dir`, `temp_path` and idempotent `temp_close`: 128 live resources,
  Unix 0600/0700 at creation, OS ACL rules elsewhere, terminal cleanup errors retained.
  Trusted paths are required; Drop/shutdown are best-effort, with no crash guarantee.
  Portable no-follow `lstat`, exact `read_link`, and file/dir `symlink` inspection/creation.
- `TcpReader` shares the existing socket Owner and closes all stream aliases on last-reader
  Drop. Exact and delimiter reads preserve buffered bytes on timeout, EOF and oversize,
  enforce whole-call deadlines and exclusive read ownership, and use linear matching.
  Read/both shutdown admits only already-buffered frames. Nested resource task captures
  now fail before spawning instead of warning and starting a task with missing bindings.
- Global `listen(port, options?)`: literal host, post-bind JSON readiness and opt-in
  loopback fixture suppression of Server/Cache-Control through both existing engines.
  Existing one-argument calls and ordinary security defaults remain supported.

Examples in `examples/system-primitives` demonstrate SRI composition, trusted-parent
file publication, a bounded two-client binary DATA fixture, and HTTP redirect,
JSON CAS/version state and no-send email capture. No SMTP/MIME parser or application
migration is included.

Limits: existing write_bytes/write_file_exclusive payloads and new Base64 decoded
payloads cap at 16 MiB (Value-array memory is larger). Atomic writes do not add a
content cap. TCP has 128 live/reserved descriptors, 16 MiB runtime-owned buffer
capacity plus active matcher/output construction (excluding caller-retained arrays),
65536-byte operations and 1..60000 ms deadlines (5000 default). Reads distinguish EOF
from timeout. Failed nonempty writes close the stream and report the prefix length.
Sockets cannot transfer through spawn/parallel/channels or public JSON.

Durability means the OS/filesystem sync contract, not guaranteed physical-media
survival. A created file remains after partial-write/sync errors, with truthful error
state. Sync after rename may fail after publication has already happened. Date,
reason phrases and HTTP framing may differ from other fixture servers; 204 has no body.

`examples/system-io-completeness` adds strict typed imports, three native Intent
scenarios, owned-file cleanup, and a Normal-mode loopback binary framing fixture.
Outbound TCP/UDP/TLS/Unix sockets, general file streams/incremental hashing and HTTP
lifecycle APIs remain deferred.

## Durable jobs: claims, recovery, and terminal-history TTL

- Ready work is claimed atomically with durable owner/attempt identity, execution
  phase, and an indexed renewable lease on SQLite and Redis/Valkey. Healthy leases
  renew; stale workers and late Redis transactions cannot overwrite fenced state.
- Bounded recovery runs at worker startup and between jobs. Claims provably not
  authorized to execute can be requeued. Interrupted authorized execution becomes
  inspectable `outcome_unknown`, not a blind replay. Unknown work has no automatic
  history TTL, uniqueness release, or batch completion accounting.
- Prepared state writes recover from acknowledgement loss without rerunning the
  job body or failure handler. Native/CLI inspection exposes ownership and recovery
  metadata; legacy active records without leases are conservatively shown read-only
  as unknown. Inspection itself does not start recovery.
- Completed/cancelled history defaults to 30 days after finishing;
  dead/failed/expired history defaults to 90 days. `configure_queue` accepts
  `retention: map { "enabled": true, "completed_days": 30, "failed_days": 90 }`.
  Live jobs do not receive terminal-history TTLs.
- Redis/Valkey uses native expiry. SQLite uses bounded generic KV expiry maintenance,
  not cleanup jobs. Physical deletion may lag under load and makes database pages
  reusable; it does not promise to shrink the file.

### Jobs upgrade requirements

1. **Upgrade all writers/workers sharing a queue together.** Do not mix old and new
   claim protocols. Pause producers/schedulers and drain or orderly-stop old workers;
   inspect pending and legacy active work before resuming. Do not discard queue data
   or automatically replay ambiguous external actions as an upgrade shortcut.
2. **Use Redis 6.2+ or compatible Valkey.** Redis authorization expiry requires the
   newer absolute-expiry support. Credentials also need the transaction commands
   documented in the [jobs guide](AI_AGENT_GUIDE.md#background-jobs-stdjobs).
3. **Configure durable, non-evicting storage for recovery guarantees.** Use file-backed
   SQLite or appropriately persisted Redis/Valkey. Recovery cannot reconstruct data
   the storage backend has lost or evicted.
4. The execution lease defaults to 300 seconds with renewal every 30 seconds.
   Optional `lease_seconds` is 10–86400. This is ownership protection, **not an
   application freshness deadline** or rollback of an external request already sent.
5. Use matching retention settings in every writer. New settings affect subsequent
   state writes, not existing TTLs or legacy history without TTL. Disabling retention
   does not remove previously assigned expirations; there is no automatic backfill.
6. **Unknown outcomes require reconciliation.** `retry_job` rejects blind replay of
   unknown work. A general auditable reconciliation/replay API, downstream idempotency
   integration, and remaining semaphore/batch-finalization crash windows are follow-up
   work in [#209](https://github.com/ntntlang/ntnt/issues/209), not an exactly-once
   execution guarantee in this release. Applications with their own recovery layer
   must not independently redispatch the same uncertain logical operation.

## Bounded in-memory task retention

Consumed task results release registry-owned payloads and synchronization state.
Unconsumed public results have independent inactivity/count/estimated-byte limits:
1 hour, 100,000 records, and 128 MiB. Compact history defaults to 24 hours after
retirement, 100,000 records, and 64 MiB. Running tasks are not evicted to satisfy
these limits. `parallel`/`race` children are cleaned up as internally owned work;
late completion cannot resurrect a forgotten result.

`NTNT_TASK_REMOVAL_TTL` now controls compact-history age, defaults to 86400 seconds,
and accepts zero to disable history. Expired/evicted task handles are not permanent
records. The limits do not include caller-owned results, active execution, or
separately owned channel buffers. This history remains process-local; durable
job state is separate. Partial worker scale-up failures no longer publish or
activate an incomplete worker pool.

## Persistent ICMP probes and worker lifecycle

- `std/net` adds owner-local `ping_open`, `ping_probe`, and `ping_close` handles for
  repeated single measurements without reopening the socket each time. Idle expiry,
  bounded ownership, sequence correlation, and finite receive deadlines apply.
  Handles cannot be transferred through tasks/channels or public JSON.
- Persistent receive deadlines remain finite across platform socket behavior.
- HTTP worker request evaluation restores task capabilities without granting unrelated
  capabilities; nested opaque Secret/resource captures fail before task startup.
- Worker control sockets use explicit ownership-safe identities, preserve worker
  state, and tolerate busy control endpoints without treating them as abandoned.
  Platform fixture fixes cover Windows atomic replacement and macOS socket behavior.

## Database and authentication fixes

- PostgreSQL String parameters bind correctly to DATE, TIME, TIMESTAMP, and
  TIMESTAMPTZ through typed encoders. TIMESTAMPTZ requires an explicit RFC 3339
  offset; invalid forms and leap seconds fail without echoing input. Existing
  text-first casts remain available for PostgreSQL-specific textual expressions.
- Shared PostgreSQL pools are bounded by `NTNT_POSTGRES_MAX_SHARED_POOLS` (default
  32, a positive integer read at first connect). Admission includes pending creation;
  unused pools can be evicted, but live handles/operations/transactions retain their
  leases. This is a pool-count cap, not a per-pool connection cap. Applications with
  more simultaneously live database targets must configure it intentionally.
  **Explicitly commit or roll back before close; close is not an implicit rollback
  guarantee.**
- SQLite `connect(path, map { "busy_timeout_ms": 5000 })` and
  `begin(db, map { "mode": "immediate" })` expose lock-wait and transaction-mode
  control. Modes are deferred/immediate/exclusive; existing one-argument defaults
  remain. Explicit setup failures are surfaced before publishing a connection.
  Busy timeout is not an overall request deadline.
- Magic-link `generic_response_floor_ms` now defaults to 0 instead of 1200.
  Set it explicitly to 1200 to retain prior padding. Padding is opt-in, best-effort,
  not constant-time protection; delivery callbacks remain synchronous.

## Scope and verification

This release packages merged work; it does not migrate or deploy consuming apps.
The source baseline passed cross-platform CI, real PostgreSQL/Redis contracts,
formatting, release-build documentation validation/generation, and example lint.
The tag-triggered release workflow separately builds and tests Linux x64, macOS
ARM64, and Windows x64 packages and publishes SHA256 checksums plus the generated
stdlib reference. See the GitHub release and its workflow for artifact status.
