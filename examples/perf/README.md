# DD-061 performance benchmarks

This directory contains the opt-in benchmark fixtures for DD-061. Normal CI does not run these benchmarks; they are for local before/after comparisons on performance PRs.

## Run the quick suite

```bash
python3 scripts/bench/run-benchmarks.py --quick
```

The harness:

1. builds `target/dev-release/ntnt`
2. starts `examples/perf/server.tnt` on `127.0.0.1:18080`
3. runs representative HTTP routes with `wrk` when installed, or a sequential `urllib` fallback when it is not
4. runs an interpreter-only CLI compute fixture
5. writes JSON and Markdown results under `target/perf-bench/`

## Run a fuller local suite

```bash
python3 scripts/bench/run-benchmarks.py --duration 10s --runs 3 --connections 32 --threads 4
```

Durations use `s` or `m` suffixes so wrk and the fallback runner measure the same window.
Use the same command on `main` and on the performance branch, then compare the generated Markdown summaries.

## Optional PostgreSQL routes

DB routes are skipped unless both of these are true:

- `DATABASE_URL` is set
- `--include-db` is passed

Example:

```bash
DATABASE_URL=postgres://ntnt:password@localhost/ntnt \
  python3 scripts/bench/run-benchmarks.py --quick --include-db
```

The DB fixtures use simple `SELECT` queries so they do not require schema setup. They intentionally measure the current full connect-query-close request path rather than pooled query-only latency. Keep DB numbers separate from the default HTTP/template/interpreter results; database latency can easily drown out interpreter changes, as databases enjoy making everything about themselves.

## Benchmarked routes

- `/` — plaintext response
- `/json` — small JSON response
- `/param/{id}` — route params and map reads
- `/compute` — interpreter loop inside an HTTP request
- `/native/calls` — repeated stable native/global calls (`len`, `str`) inside an HTTP request
- `/template/layout` — external template render with layout, partial, and loop
- `/template/rows` — template-heavy 100-row render
- `/db/single` — optional single PostgreSQL query
- `/db/multi` — optional small multi-query PostgreSQL handler
