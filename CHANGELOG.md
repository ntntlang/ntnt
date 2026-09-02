# Changelog

## 0.5.3

### Added

- Added `std/markdown.parse_blocks` for semantic Markdown blocks with exact UTF-8 byte ranges and source slices.
- Extended `std/http.download` with fetch-compatible request maps, streaming binary writes, safe file options, and atomic promotion.
- Added capability-gated `std/process` APIs for bounded commands and supervised long-running child processes.

### Compatibility

- The legacy `download(url, path)` form still creates parent directories and overwrites an existing destination.
- Process execution is disabled unless `NTNT_PROCESS_ENABLE=1`; `NTNT_PROCESS_ALLOW` can restrict execution to canonical executable paths.
