# Unreleased

## DD-041: exact loaded-source provenance

The 0.6.1 development runtime records the actual file input parsed by each
interpreter, including main source, ordinary/transitive imports and direct
libs/middleware/routes/jobs loaders. Source identity no longer needs to be guessed
from mtimes or a later read of possibly replaced files. A cache hit retains the
original loaded bytes' digest. Path resolution is shared with the corresponding
read/parser origin rather than resolving a symlink again after reading.

For Rust embedding, `source::SourceFile::read(...).parse()` yields immutable
`ParsedSource` input for `Interpreter::eval_source`. `source_snapshot()` returns
an ordered, length-delimited, platform-local source-only snapshot. Existing
`eval(&Program)`, public opaque block evaluation and AST construction/JSON remain
supported and unchanged;
a filename set independently of an AST does not attest to its input. Source-less
entries or multiple evaluated versions of one identity make attestation
unavailable, without disabling evaluation/reload. Readable virtual/pipe inputs
such as `/dev/stdin` remain executable but are not given regular-file attestation.
Per-file history retains one digest plus a conflict bit, not unbounded versions
or application source text.

This is a prerequisite, **not** WebSocket support, full program-generation
compatibility, worker quarantine, per-application realtime isolation, or a new
worker-admission check. Imports remain lazy; snapshots describe only inputs
observed so far and are not inherited by captured tasks. No new NTNT syntax,
stdlib API, mandatory configuration, dependency, or deployment is introduced.

See [NTNT v0.6.0](v0.6.0.md) for the previous release announcement and upgrade
notes. The 0.5.7 development cycle was included in 0.6.0, not released separately.
