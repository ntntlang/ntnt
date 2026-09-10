# NTNT v0.5.4 — proposed, unreleased

This release adds native primitives for raw cryptography, filesystem operations,
TCP listeners, and HTTP listener options.

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
lifecycle changes remain deferred. Package version stays 0.5.4, unreleased.
