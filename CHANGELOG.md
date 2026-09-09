# Changelog

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
