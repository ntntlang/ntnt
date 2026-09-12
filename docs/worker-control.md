# Worker control sockets

On Unix, `ntnt worker`, `work_async()` and `work_jobs()` start a private control
socket before starting job workers. Setup failure is an error to the caller.
The newline JSON commands and their responses are unchanged.

```bash
ntnt worker app.tnt --worker-group emails
ntnt workers status --dir /srv/my-app --worker-group emails
ntnt workers scale normal 4 --dir /srv/my-app --worker-group emails

ntnt worker app.tnt --control-socket /run/user/1000/my-app/jobs.sock
ntnt workers status --control-socket /run/user/1000/my-app/jobs.sock
```

All four `workers` commands (`status`, `scale`, `pause`, `resume`) accept
`--control-socket PATH`, `--worker-group NAME`, and `--dir DIR`. The worker
command accepts the first two. `NTNT_CONTROL_SOCKET` and `NTNT_WORKER_GROUP`
provide environment equivalents; explicit CLI options win. The group defaults
to `default`. Empty paths and group names are errors. An explicit socket path
selects the endpoint directly; the group does not modify that path.

Embedded workers accept `"control_socket"` and `"worker_group"` in their options
map, overriding the same environment variables:

```ntnt
import { work_async } from "std/jobs"
let workers = work_async(map { "worker_group": "emails", "concurrency": 4 })
```

## Project identity and discovery

The server canonicalizes the main source file's directory, then uses the nearest
ancestor containing an `ntnt.toml` file. Without a manifest, the source directory
itself is the project identity. The client applies the identical rule to `--dir`,
or its current directory if omitted. Use `--dir` with the source directory when
there is no manifest. Symlink aliases resolve to the same canonical identity.
Native Rust callers without a source context use CWD; embedders can scope an
explicit source with `control_socket::with_source`.

**Relative explicit socket paths resolve from this project identity on both
server and client**, including paths supplied through the environment. They do
not resolve from a different launch directory. The explicit parent must already
exist, be owned by the caller, and be private (`0700`). This also protects the
brief interval between binding and setting socket permissions under a permissive
umask. CLI paths must be UTF-8. Absolute
paths are independent of project identity. Unix endpoints are limited to 103
pathname bytes for portability; failures identify the exact endpoint.

The default filename is the first 20 bytes of SHA-256 over canonical project
path bytes, a NUL separator, and the group name, encoded as hexadecimal plus
`.sock`. It is stored in `$XDG_RUNTIME_DIR/ntnt` when XDG_RUNTIME_DIR is an
absolute, caller-owned private directory and the resulting endpoint fits the
pathname limit. Otherwise it uses `/tmp/ntnt-<effective-uid>`. Runtime directories
are created with `0700`; existing directories are validated, never repaired by
chmod. A hostile `ntnt` child directory is an error. Clients can discover an
endpoint before any server has run: they reserve/validate the runtime directory
but create neither a socket nor a lock. No source-tree `.ntnt.sock` is created,
probed, or used as a fallback. Old deployments must restart their workers and
update custom socket clients to use the new endpoint or an explicit path.

## Ownership and shutdown

A `0600` regular sidecar file, `<socket>.lock`, holds a nonblocking OS advisory
exclusive lock. Ownership is acquired before probing or binding and remains
held until the accept thread has stopped and cleanup has finished. A second
process gets an error without disturbing the first. The lock file is deliberately
never unlinked: deleting it could allow two processes to lock different inodes.
Do not delete lock files while workers could be starting or running.

After a crash the OS releases the lock. The next owner probes any existing
owner-only socket with a nonblocking datagram connection, which avoids consuming
a stream listener's backlog, and removes it only after `ECONNREFUSED` confirms
it is stale. A live stream socket returns `EPROTOTYPE` to this probe even before
`listen()` or when its backlog is full. A stream connection alone is not a safe
stale test: macOS can return `ECONNREFUSED` for a full, live backlog.
Live listeners, full backlogs, other ambiguous failures, symlinks, ordinary files,
unsafe modes, foreign ownership, and hard-linked lock files are refused. This
also protects listeners that do not implement the lock protocol.

Sockets are set to `0600` before accepting commands. Cleanup checks the socket's
device and inode, so a replaced file or socket survives shutdown. Failed
reconfiguration retains the previous in-process listener. Server connection IO
has a five-second total deadline and checks cancellation; a trickling client
cannot extend that deadline indefinitely. Requests remain limited to 64 KiB.
Runtime shutdown closes the control socket, including for embedded workers.
Clients wait for connection completion within a ten-second deadline, including
temporary backlog saturation. In-progress connections use readiness polling and
`SO_ERROR`; a backlog refusal retries only connection establishment, never a
command that has already been sent.
A host with its own Ctrl-C handler can call `jobs::use_host_shutdown_handler()`
after installing a handler that shuts down the runtimes and exits; `ntnt run`
does this automatically.

The safety boundary is the operating-system user and a protected parent
directory. Same-user processes can manage workers; do not expose these endpoints
to untrusted same-user code. A user able to replace parent directories or delete
lock files can invalidate the ownership protocol.

## Windows

Default worker execution continues without a Unix control listener. Explicit
socket/group options, either environment variable, and every `ntnt workers`
command return an unsupported error. There is no named-pipe equivalent.
Unix tests require Linux and macOS CI; Linux testing alone does not establish
macOS or Windows runtime behavior.
