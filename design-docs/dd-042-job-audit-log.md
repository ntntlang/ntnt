# DD-042: Job Audit Log & Observability Pipeline

**Status:** Planning
**Author:** Larri
**Created:** 2026-03-18
**Parent:** [DD-037: Concurrency & Job System](dd-037-concurrency-and-jobs.md) — Phase 8
**Depends on:** Phase 2 ✅ (job system core), Phase 6 ✅ (CLI observability)

---

## Table of Contents

1. [Problem Statement](#problem-statement)
2. [Vision](#vision)
3. [Architecture](#architecture)
4. [Configuration API](#configuration-api)
5. [Log Entry Schema](#log-entry-schema)
6. [Sink Backends](#sink-backends)
7. [CLI: `ntnt jobs logs`](#cli-ntnt-jobs-logs)
8. [Programmatic API](#programmatic-api)
9. [Webhooks](#webhooks)
10. [Web Viewer Integration](#web-viewer-integration)
11. [Implementation Plan](#implementation-plan)
12. [Open Questions](#open-questions)

---

## Problem Statement

The job system currently emits structured JSON to stderr — fire-and-forget. This is useful for `2>&1 | grep`, but it has fundamental limitations:

1. **No persistence.** Once the process exits, logs are gone. No post-mortem analysis.
2. **No querying.** Can't ask "show me all failed SendEmail jobs in the last hour."
3. **No payload capture.** You see that a job ran, but not what data it processed or what it returned.
4. **No live tail across workers.** Multiple `ntnt worker` processes each write to their own stderr — no unified view.
5. **No alerting.** When a job dies, nobody knows unless they're watching stderr.
6. **No web viewer.** Building a dashboard requires a queryable log store, not stderr scraping.

### What exists today

| Feature | Current state | Gap |
|---------|--------------|-----|
| Event emission | `emit_job_event()` → stderr JSON | Not persisted, not queryable |
| Event types | `job.enqueued`, `job.started`, `job.completed`, `job.failed`, `job.dead` | No payload/result capture |
| CLI | `ntnt jobs status/list/inspect` | No log viewing, no live tail |
| Programmatic | `list_jobs()`, `job_status()` | No log history |
| Alerting | None | No webhook/notification support |

---

## Vision

A structured, queryable, configurable audit log for every job event. Think: **the job equivalent of HTTP access logs** — always on, always queryable, configurable verbosity, multiple output destinations.

**Design principles:**
- **On by default, zero config.** Jobs log to KV automatically. You don't configure logging — you configure where it goes and how verbose it is.
- **Queryable.** Logs are data, not text. Filter by job type, status, queue, time range.
- **Configurable verbosity.** Summary (event + timing), verbose (+ truncated payload), full (everything).
- **Pluggable sinks.** KV (default, queryable), file (rotatable), stderr (current behavior, piping), webhook (alerting).
- **TTL by default.** 48 hours in KV. Logs don't accumulate forever.
- **Foundation for dashboard.** The web viewer reads from the same log store.
- **Foundation for webhooks.** Event routing is the same pipeline — just a different sink.

---

## Architecture

### Log Pipeline

```
Job Event (worker thread)
    │
    ▼
┌─────────────────────┐
│   Log Collector      │  Formats the log entry (schema below)
│   (in-process)       │  Applies verbosity filter
└──────────┬──────────┘
           │
     ┌─────┴──────┐
     │  Fan-out    │  Sends to all configured sinks
     └─┬──┬──┬──┬─┘
       │  │  │  │
       ▼  ▼  ▼  ▼
     KV  File Stderr Webhook
```

### Key Design Decisions

1. **Logs are separate from job data.** Job data (`jobs:data:<id>`) is the source of truth for job state. Logs (`jobs:log:<timestamp>:<id>`) are an append-only audit trail. Deleting logs doesn't affect job execution.

2. **Logs use the same KV store.** No separate connection. `configure_queue(map { "store": "redis://..." })` configures both job storage and log storage. This means logs benefit from the same backend choice (Redis for prod, SQLite for dev).

3. **TTL is KV-native.** Redis TTL keys auto-expire. SQLite uses a reaper (same pattern as task reaper in Phase 1). Default: 48 hours.

4. **Fan-out is synchronous but non-blocking.** KV write and stderr are synchronous (fast). File append is synchronous (fast). Webhook POSTs are fire-and-forget via `spawn()` — a slow webhook doesn't block job execution.

5. **Verbosity is per-sink.** You might want full payloads in KV (for the web viewer) but summary only in stderr (for terminal readability).

---

## Configuration API

### Basic (zero-config — just works)

```ntnt
configure_queue(map { "store": "redis://localhost:6379" })
// Logging is ON by default:
//   sink: kv (same store)
//   ttl: 172800 (48h)
//   verbosity: "summary"
//   stderr: still emits (backward compat)
```

### Explicit Configuration

```ntnt
configure_queue(map {
  "store": "redis://localhost:6379",
  "log": map {
    "enabled": true,                    // default: true. Set false to disable all logging.
    "ttl": 172800,                      // seconds. Default 48h. Set 0 for no expiry.
    "verbosity": "verbose",             // "summary" | "verbose" | "full"
    "sinks": [
      map { "type": "kv" },            // default, uses same store
      map { "type": "stderr" },         // backward compat (current behavior)
      map {
        "type": "file",
        "path": "/var/log/ntnt/jobs.log",
        "verbosity": "full"             // per-sink verbosity override
      },
      map {
        "type": "webhook",
        "url": "https://hooks.slack.com/services/...",
        "events": ["job.dead"],          // only fire on specific events
        "verbosity": "verbose"
      },
      map {
        "type": "webhook",
        "url": "https://my-app.com/api/job-events",
        "events": ["job.completed", "job.dead"],
        "headers": map { "Authorization": "Bearer ${JOB_WEBHOOK_TOKEN}" }
      }
    ]
  }
})
```

### Disable Logging Entirely

```ntnt
configure_queue(map {
  "store": "redis://localhost:6379",
  "log": map { "enabled": false }
})
```

---

## Log Entry Schema

### Summary (default)

```json
{
  "event": "job.completed",
  "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "type": "SendEmail",
  "queue": "emails",
  "status": "completed",
  "attempt": 1,
  "duration_ms": 142,
  "timestamp": "00001742256000000000000"
}
```

### Verbose (+ truncated payload/result)

```json
{
  "event": "job.completed",
  "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "type": "SendEmail",
  "queue": "emails",
  "status": "completed",
  "attempt": 1,
  "duration_ms": 142,
  "timestamp": "00001742256000000000000",
  "payload_summary": "{\"to\":\"alice@example.com\",\"subject\":\"Welcome...\"}",
  "result_summary": "{\"sent\":true,\"message_id\":\"msg_abc123\"}"
}
```

Payload and result are JSON-serialized and truncated to 500 characters. If truncated, a `"…"` suffix is appended.

### Full (complete payload and result)

```json
{
  "event": "job.completed",
  "job_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "type": "SendEmail",
  "queue": "emails",
  "status": "completed",
  "attempt": 1,
  "duration_ms": 142,
  "timestamp": "00001742256000000000000",
  "payload": {"to": "alice@example.com", "subject": "Welcome to Acme", "body": "...full body..."},
  "result": {"sent": true, "message_id": "msg_abc123", "provider_response": "..."}
}
```

### Error Events (job.failed, job.dead)

All verbosity levels include `error` and `will_retry`:

```json
{
  "event": "job.failed",
  "job_id": "...",
  "type": "ProcessPayment",
  "queue": "payments",
  "status": "retrying",
  "attempt": 2,
  "max_attempts": 5,
  "duration_ms": 3200,
  "error": "Stripe API timeout after 3000ms",
  "will_retry": true,
  "next_retry_at": "00001742259200000000000",
  "timestamp": "..."
}
```

---

## Sink Backends

### KV Sink (default)

**Key layout:**
```
jobs:log:<zero-padded-timestamp>:<job_id>  →  JSON log entry
```

- Timestamp prefix gives chronological ordering via `kv_list("jobs:log:")`
- TTL: 48h default. Redis uses native key TTL. SQLite uses a periodic reaper.
- Queryable via `list_job_logs()` and `ntnt jobs logs`

**Why the same KV store:** No additional infrastructure. If you're running Redis for jobs, your logs go to the same Redis. If you're using SQLite for dev, logs go to the same SQLite. Zero config.

### File Sink

```ntnt
map { "type": "file", "path": "/var/log/ntnt/jobs.log" }
```

- Append-only, one JSON line per event (JSONL format)
- No TTL — use external log rotation (logrotate, etc.)
- Good for: shipping to ELK/Splunk/Datadog via filebeat, or `tail -f | grep`
- File is opened once, kept open. Reopens on SIGHUP (log rotation friendly).

### Stderr Sink

```ntnt
map { "type": "stderr" }
```

- Current behavior, preserved for backward compatibility
- Locked stderr writes (atomic lines across concurrent workers)
- Good for: `ntnt worker server.tnt 2>&1 | grep error`, Docker log drivers
- This is the ONLY sink that's on by default even without explicit config (backward compat)

### Webhook Sink

```ntnt
map {
  "type": "webhook",
  "url": "https://hooks.slack.com/services/T.../B.../xxx",
  "events": ["job.dead"],
  "headers": map { "Content-Type": "application/json" },
  "verbosity": "verbose",
  "timeout": 5                   // seconds, default 5
}
```

- HTTP POST with JSON body (the log entry)
- Fire-and-forget via `spawn()` — slow/failed webhooks don't block job execution
- Configurable event filter — only fire on specific events
- Configurable headers for auth
- Timeout: 5s default. Failed webhooks log a warning to stderr, do not retry.
- Rate limiting: max 1 POST per second per URL (prevent webhook storms from job bursts)

**Use cases:**
- Slack/Discord alert on `job.dead`
- Analytics pipeline on `job.completed`
- Custom monitoring on all events
- Trigger downstream workflows on job completion

---

## CLI: `ntnt jobs logs`

### Commands

```bash
# Show recent log entries (default: last 50)
ntnt jobs logs server.tnt

# Live tail (follows new entries, like tail -f)
ntnt jobs logs server.tnt --follow

# Filter by job type
ntnt jobs logs server.tnt --type=SendEmail

# Filter by event
ntnt jobs logs server.tnt --event=job.dead

# Filter by queue
ntnt jobs logs server.tnt --queue=emails

# Time range
ntnt jobs logs server.tnt --since=1h
ntnt jobs logs server.tnt --since=2026-03-18T00:00:00

# Output format
ntnt jobs logs server.tnt --format=json    # raw JSON (pipe to jq)
ntnt jobs logs server.tnt --format=table   # human-readable table (default)

# Combine filters
ntnt jobs logs server.tnt --type=SendEmail --event=job.dead --since=24h --format=json

# Limit
ntnt jobs logs server.tnt --limit=100

# Verbose: show payload/result
ntnt jobs logs server.tnt --verbose
```

### Table Format (default)

```
TIMESTAMP            EVENT          TYPE              QUEUE     ATTEMPT  DURATION  STATUS
2026-03-18 00:15:02  job.completed  SendEmail         emails    1/3      142ms     completed
2026-03-18 00:14:58  job.started    SendEmail         emails    1/3      —         active
2026-03-18 00:14:55  job.failed     ProcessPayment    payments  2/5      3200ms    retrying
2026-03-18 00:14:50  job.dead       ImportCSV         imports   5/5      45200ms   dead
────────────────────────────────────────────────────────────────────────────────────────────
4 entries (filtered from 127 total)
```

### Piping

```bash
# Grep for errors
ntnt jobs logs server.tnt --format=json | grep '"event":"job.dead"'

# Count failures by type
ntnt jobs logs server.tnt --event=job.dead --format=json | jq '.type' | sort | uniq -c

# Watch for dead jobs in real-time
ntnt jobs logs server.tnt --follow --event=job.dead

# Export last 24h of logs as JSON
ntnt jobs logs server.tnt --since=24h --format=json --limit=10000 > /tmp/job-logs.json
```

---

## Programmatic API

### `list_job_logs(opts?)`

```ntnt
import { list_job_logs } from "std/jobs"

// All recent logs (last 50)
let logs = unwrap(list_job_logs())

// Filter by type and event
let dead_emails = unwrap(list_job_logs(map {
  "type": "SendEmail",
  "event": "job.dead",
  "limit": 10
}))

// Filter by time
let last_hour = unwrap(list_job_logs(map {
  "since_secs": 3600    // last 3600 seconds
}))

// Filter by queue
let payment_logs = unwrap(list_job_logs(map {
  "queue": "payments",
  "limit": 100
}))
```

**Returns:** `Result<Array<Map>, String>` — array of log entry maps.

### `delete_job_logs(opts)`

```ntnt
import { delete_job_logs } from "std/jobs"

// Clear old logs manually (beyond TTL auto-cleanup)
let cleared = unwrap(delete_job_logs(map {
  "older_than_secs": 86400    // older than 24h
}))

// Clear logs for a specific job type
let cleared = unwrap(delete_job_logs(map {
  "type": "SendEmail"
}))
```

### `configure_job_log(opts)`

```ntnt
import { configure_job_log } from "std/jobs"

// Runtime reconfiguration (e.g., increase verbosity for debugging)
configure_job_log(map {
  "verbosity": "full",
  "ttl": 3600    // reduce to 1h while debugging (less KV pressure)
})
```

---

## Webhooks

### Event Routing

Webhooks are sinks with event filters. The pipeline fans out to all configured sinks; each webhook checks its event filter before POSTing.

```
job.completed event
    │
    ├─→ KV sink: always writes
    ├─→ stderr sink: always writes
    ├─→ Slack webhook (events: ["job.dead"]): SKIP (not matching)
    └─→ Analytics webhook (events: ["job.completed"]): POST
```

### Webhook Payload

The POST body is the log entry JSON (same schema as KV), wrapped in a metadata envelope:

```json
{
  "source": "ntnt/jobs",
  "version": "1",
  "event": "job.dead",
  "data": {
    "job_id": "...",
    "type": "ProcessPayment",
    "queue": "payments",
    "error": "Stripe API timeout",
    "attempt": 5,
    "max_attempts": 5,
    "duration_ms": 3200,
    "timestamp": "..."
  }
}
```

### Webhook Reliability

- **Fire-and-forget.** Webhook failures do not affect job execution.
- **No retry.** Failed webhooks log a warning to stderr. If you need guaranteed delivery, use the KV sink and poll from your consumer.
- **Timeout: 5s.** Slow endpoints are abandoned.
- **Rate limit: 1/sec per URL.** Prevents webhook storms from batch enqueue operations. Excess events are buffered in a small queue (100 entries) — overflow drops with a warning.
- **Future: webhook retry queue.** If demand exists, webhook delivery could use the job system itself (dog-fooding). Deferred.

---

## Web Viewer Integration

The job audit log is the data source for Phase 5's dashboard. The web viewer reads from `jobs:log:*` in KV:

```ntnt
// In a route handler
get "/admin/jobs/logs" {
  let logs = unwrap(list_job_logs(map {
    "limit": 100,
    "type": request.query("type"),
    "event": request.query("event")
  }))
  render("admin/job_logs.html", map { "logs": logs })
}

// SSE endpoint for live tail
get "/admin/jobs/logs/stream" {
  // Phase 5: SSE integration with DD-041
  // Poll list_job_logs with increasing since_secs, push new entries
}
```

The web viewer doesn't need its own data pipeline — it uses the same `list_job_logs()` that the CLI and application code use. SSE for live updates comes from DD-041 (Real-Time Streaming).

---

## Implementation Plan

### Sub-phases

```
PR 8a   Log Collector + KV Sink + stderr sink      Core pipeline, replaces emit_job_event
PR 8b   CLI: ntnt jobs logs                         Table/JSON output, filters, --follow
PR 8c   Programmatic API                            list_job_logs(), delete_job_logs()
PR 8d   File Sink + Webhook Sink                    External output destinations
```

### PR 8a: Log Collector + KV Sink (core)

**Estimated effort:** 2-3 days

- [ ] `JobLogCollector` struct in `src/stdlib/jobs.rs` — replaces `emit_job_event()`
- [ ] Log entry schema: `JobLogEntry` struct with event, job_id, type, queue, status, attempt, duration_ms, error, payload, result, timestamp
- [ ] Verbosity filtering: summary/verbose/full
- [ ] KV sink: write to `jobs:log:<timestamp>:<job_id>` with TTL
- [ ] Stderr sink: backward-compatible JSON lines (locked writes)
- [ ] `configure_queue` extended: `"log"` key with enabled, ttl, verbosity, sinks
- [ ] TTL reaper for SQLite (Redis uses native TTL)
- [ ] Duration tracking: record `started_at` on claim, compute `duration_ms` on completion/failure
- [ ] Payload capture: serialize job args at enqueue time (respecting verbosity)
- [ ] Result capture: serialize perform return value on completion (respecting verbosity)
- [ ] Tests: log entries written to KV, TTL expiry, verbosity filtering
- [ ] `@ntnt` doc blocks on all new functions

### PR 8b: CLI — `ntnt jobs logs`

**Estimated effort:** 1-2 days

- [ ] `ntnt jobs logs <FILE>` subcommand with filters
- [ ] `--follow` mode: poll KV for new entries every 500ms, print incremental
- [ ] `--type`, `--event`, `--queue`, `--since`, `--limit` filters
- [ ] `--format=json` and `--format=table` output
- [ ] `--verbose` flag: show payload/result columns in table mode
- [ ] Color-coded event types (completed=green, failed=red, dead=red+bold, started=cyan)

### PR 8c: Programmatic API

**Estimated effort:** 1 day

- [ ] `list_job_logs(opts?)` — query KV log entries with filters
- [ ] `delete_job_logs(opts)` — bulk clear logs by type/age
- [ ] `configure_job_log(opts)` — runtime reconfiguration
- [ ] Typechecker signatures
- [ ] `@ntnt` doc blocks
- [ ] AI_AGENT_GUIDE documentation

### PR 8d: File Sink + Webhook Sink

**Estimated effort:** 2-3 days

- [ ] File sink: JSONL append, SIGHUP reopen
- [ ] Webhook sink: HTTP POST via `spawn()`, event filtering, rate limiting
- [ ] Webhook timeout (5s), error logging to stderr
- [ ] Webhook rate limiter: 1/sec per URL, 100-entry buffer
- [ ] `configure_queue` webhook configuration
- [ ] Tests: file output, webhook mock
- [ ] Documentation: webhook payload schema, configuration examples

### Estimated total: 6-9 days across 4 PRs

---

## Open Questions

| Question | Options | Notes |
|----------|---------|-------|
| Log key ordering | Timestamp-prefix (like pending keys) vs. incrementing ID | Timestamp aligns with job key pattern |
| SQLite reaper interval | 5 min (like task reaper) vs configurable | Start with 5 min, make configurable later |
| Webhook retry | None (fire-and-forget) vs job-system dogfood | Start with none, add retry-via-jobs later |
| Result capture mechanism | Return value from perform body vs explicit `job_result()` call | Return value is cleaner but requires interpreter change |
| Log entry size limit | Per-entry cap (e.g. 64KB) vs unlimited | Cap prevents a single job from filling KV |
| --follow implementation | KV polling vs channel-based | KV polling is simpler, channel is lower latency |
| Per-sink verbosity default | Inherit from global or each sink has own default | Per-sink override with global fallback |

---

## Version History

| Date | Change |
|------|--------|
| 2026-03-18 | Initial design: log collector, 4 sink backends, CLI, programmatic API, webhooks |
