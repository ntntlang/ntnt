# NTNT v0.5.4 — proposed, unreleased

This release adds native primitives for raw cryptography, filesystem operations,
TCP listeners, and HTTP listener options.

- `std/crypto`: `sha384`, `sha384_bytes` accept exact UTF-8 or checked integer bytes;
  `base64_encode_bytes` produces standard padded base64. Existing SHA256 and string
  base64 contracts remain unchanged. The SHA256 static signature now includes its
  already-supported byte-array input.
- `std/fs`: `write_bytes`, `write_file_exclusive`, `mkdir_private`, `sync_file`,
  `sync_dir`, `file_permissions`, `chmod`, `chown`, `access`; `std/path.resolve_missing`.
  Unix exclusive creation requests 0600 and directory creation 0700 by default,
  restricted by the inherited umask. No process umask mutation or chmod-after-write
  fallback. Real-ID access is advisory; ownership, ACLs and trusted ancestors remain
  OS/caller concerns. Secure POSIX-only operations return `unsupported:` elsewhere.
- `std/net`: bounded opaque listener/stream APIs, literal loopback default, port 0,
  byte reads, complete writes, addresses, half-close and close. No outgoing socket
  API or outbound-policy bypass. Existing diagnostic `tcp_connect` is unchanged.
- Global `listen(port, options?)`: literal host, post-bind JSON readiness and opt-in
  loopback fixture suppression of Server/Cache-Control through both existing engines.
  Existing one-argument calls and ordinary security defaults remain supported.

Examples in `examples/system-primitives` demonstrate SRI composition, trusted-parent
file publication, a bounded two-client binary DATA fixture, and HTTP redirect,
JSON CAS/version state and no-send email capture. No SMTP/MIME parser or application
migration is included.

Limits: file writes 16 MiB; TCP 128 live/reserved descriptors, 16 MiB transient buffers,
65536-byte operations and 1..60000 ms deadlines (5000 default). Reads distinguish EOF
from timeout. Failed nonempty writes close the stream and report the prefix length.
Sockets cannot transfer through spawn/parallel/channels or public JSON.

Durability means the OS/filesystem sync contract, not guaranteed physical-media
survival. A created file remains after partial-write/sync errors, with truthful error
state. Sync after rename may fail after publication has already happened. Date,
reason phrases and HTTP framing may differ from other fixture servers; 204 has no body.
