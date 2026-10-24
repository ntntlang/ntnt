# NTNT v0.5.6 — Task helpers, local TOTP QR codes and host CA trust

This release lets concurrent tasks call your own NTNT functions, adds task-memory
accounting and local TOTP enrollment QR rendering, and fixes caller-state
restoration after function errors. It also patches dependency vulnerabilities and
moves TLS inspection, monitoring HTTPS probes and SMTP TLS to the host CA store.

## Upgrade notes

- **Update the NTNT binary for the security fixes.** This release includes a
  patched TLS verifier and other dependency updates. Updating CA certificates
  alone does not update those libraries. Keep the operating system and container
  trust stores patched separately; see the
  [eMudhra incident advisory](https://emudhra.we-have-a-problem.com/).
- **Provide a usable host/container CA store.** `std/net.tls_info`, HTTPS
  `std/http.probe_fetch` and SMTP TLS no longer use compiled-in public roots, and
  there is no bundled fallback. Linux/container images need a CA package such as
  `ca-certificates`, or an explicitly supplied store. Containers do not
  automatically inherit their physical host's certificates. Windows and macOS
  use their platform certificate stores.
- **Restart NTNT after CA-store changes.** For these Rustls clients, trust
  discovery, including a failed discovery, is cached at first TLS use. Set
  `SSL_CERT_FILE` / `SSL_CERT_DIR` before first use if overriding discovery;
  explicit paths replace native discovery. After installing, repairing or
  changing the store or overrides, restart the process. Subsequent CA-only
  updates do not require rebuilding NTNT; TLS-library fixes still can.
- **Check structs constructed inside tasks.** Field-only invariants now run
  instead of being silently skipped. An invariant that calls a helper or reads
  an outer constant is not yet supported in tasks and fails clearly when the
  struct is constructed there. Construct such structs outside the task.
- **Error cleanup and contracts now use the correct call frame.** A failing
  function's `defer` runs in its own scope before the caller's `otherwise`
  handler, and nested calls no longer overwrite the caller's `old()` snapshot.
  Code depending on the previous leaking state will behave differently.
- **Upgrading from before 0.5.5?** The Redis job backend still requires Redis 7.0+
  or Valkey 7.2+, and writers/workers sharing a queue must be upgraded together.
  See the [0.5.5 upgrade notes](release-notes-v0.5.5.md#upgrade-notes).

## Concurrent tasks

### Call application helpers from task closures

Closures passed to `spawn`, `after`, `schedule`, `parallel` and `race` can now
call user-defined NTNT functions, including imported helpers, transitive calls,
recursion and mutual recursion. Functions and the values they use are copied into
the task's interpreter; mutations remain task-local, not shared with the caller.

Unsupported captures such as opaque secrets, process/TCP handles and closures
stored in maps still fail before the task starts. Errors identify the dependency
path, for example `task -> outer -> call_a -> handlers`. Contract captures respect
the helper's entry scope and callee-local bindings; capture analysis remains
conservative around early returns. See the
[task guide](AI_AGENT_GUIDE.md#tasks-spawnawait) for capture and invariant boundaries.
([#271](https://github.com/ntntlang/ntnt/pull/271), #186)

Task threads now use 16 MiB stacks rather than Rust's 2 MiB default, allowing
release builds to reach the normal recursion-limit error instead of aborting the
process first. Debug builds still have less recursion headroom. Copied task scopes
are torn down on completion and panic, breaking helper-closure cycles and releasing
captured channel senders.

### Inspect task retention without reading task contents

`std/concurrent.task_stats()` returns six nonnegative integer gauges:

- `active_tasks`, `tracked_tasks`
- `retained_results`, `retained_result_bytes`
- `history_records`, `history_bytes`

Inspection is constant-time and process-local. It does not expose IDs, payloads,
errors or captures, consume results, refresh inactivity clocks, or trigger eviction.
Byte gauges estimate retained public results/errors and compact history, **not
process RSS**. The active-task counter is sampled separately from registry fields;
this is not an atomic snapshot of all task transitions. Existing retention defaults
are unchanged, with no new configuration or task-list API.
([#281](https://github.com/ntntlang/ntnt/pull/281), #267)

## Authentication: local TOTP QR SVG

`std/auth.totp_qr_svg(uri: String) -> Result<String, String>` turns the URI from
`begin_totp_enrollment` or `totp_uri` into an SVG locally. It needs no auth
initialization, network requests, files or external commands. Valid TOTP URIs of
up to 2048 UTF-8 bytes are accepted, subject to QR capacity; invalid input returns
secret-free errors.

The renderer encodes the original URI bytes with Q-level error correction and a
four-module quiet zone. Output is SVG geometry, without the URI, label or seed
interpolated into XML text or metadata. **The QR itself still contains the
enrollment secret:** show it only during enrollment; never log, cache or send it
to a third-party QR service. Preserve the quiet zone when resizing. See the
[native example](../examples/totp_qr.tnt).
([#283](https://github.com/ntntlang/ntnt/pull/283))

## Interpreter error recovery and contracts

User-function calls restore the caller's scope and contract context after failures
in default arguments, destructured parameters, `requires`, `old()` capture, the
function body and `ensures`. A caller's `otherwise` handler no longer loses its
own variables or inherits the failed callee's scope.

Each call owns its `old()` context, so nested calls no longer invalidate the
caller's postconditions or expose the caller's snapshot to the callee. Defers run
in the callee's scope on error, including errors escaping to the top level;
successful-call ordering still runs defers before `ensures`. This fixes
user-function exits, not every remaining scope-restoration or cycle-collection
case tracked under #183.
([#270](https://github.com/ntntlang/ntnt/pull/270))

## TLS and dependency security

- TLS inspection, monitoring HTTPS probes and SMTP TLS share a host-root loader.
  Missing or unusable roots produce a local setup error before target TCP contact.
  Rustls/ring, hostname and chain verification, required STARTTLS versus implicit
  TLS, and compromised-intermediate-key protection are retained. Ordinary
  `fetch`/`download` continue using native/system TLS. Importing host roots does
  **not** reproduce every OS/browser revocation or distrust policy; initial local
  trust discovery is synchronous and is not interrupted by network timeouts.
  Plain HTTP, email log transport and explicitly plaintext SMTP need no CA store.
  ([#284](https://github.com/ntntlang/ntnt/pull/284))
- The patched certificate verifier distrusts compromised eMudhra intermediate
  keys even if their old issuer roots remain trusted.
  ([#274](https://github.com/ntntlang/ntnt/pull/274))
- Rust dependency updates address the advisory backlog, including older HTTP/2
  and IDNA dependencies. DNS, JWT and randomness adapters retain their NTNT APIs;
  JWT uses an explicit AWS-LC crypto provider without switching the HTTP clients'
  TLS backends.
  ([#278](https://github.com/ntntlang/ntnt/pull/278))
- `ctutils` 0.4.4 fixes
  [RUSTSEC-2026-0385](https://rustsec.org/advisories/RUSTSEC-2026-0385.html), and
  Tokio 1.53.3 fixes
  [RUSTSEC-2026-0386](https://rustsec.org/advisories/RUSTSEC-2026-0386.html).
  Tokio's minimum version is raised to avoid resolving an affected version again.
  ([#282](https://github.com/ntntlang/ntnt/pull/282))

## Development and CI

- Security-only Cargo Dependabot updates, a RustSec audit on every PR/main push,
  and a daily audit with deduplicated public issues for published advisories.
  Updates are not automatically merged or deployed.
  ([#275](https://github.com/ntntlang/ntnt/pull/275))
- Auth-backend CI pulls official PostgreSQL and Redis service images through the
  public ECR mirror to avoid Docker Hub's anonymous pull limits.
  ([#272](https://github.com/ntntlang/ntnt/pull/272))

The release archive platforms and checksum-verified installation flow are
unchanged. See [release builds and installation](RELEASING.md).

**Full changelog:** https://github.com/ntntlang/ntnt/compare/v0.5.5...v0.5.6
