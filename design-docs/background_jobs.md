# Background Jobs / Job Queuing System

## Overview

A background job system for NTNT that enables async task processing, scheduled jobs, and reliable task queuing. Works out-of-the-box with in-memory storage, with optional persistent backends (PostgreSQL, Redis/Valkey) for production.

## Goals

1. **Zero-config start** - Works immediately with in-memory queue
2. **Production-ready** - Graceful shutdown, heartbeats, resilience built-in
3. **Simple API** - Declarative Job DSL with co-located configuration
4. **Reliable** - At-least-once delivery, retries, dead-letter queues
5. **Observable** - Status, history, metrics
6. **Intent-driven** - Jobs testable in .intent files
7. **Minimal footprint** - Optional backends via feature flags

---

## Job Declaration (DSL)

Jobs use a minimal, readable syntax with sensible defaults:

### Simple Job (Most Common)

```ntnt
/// Sends personalized welcome email to newly registered users
Job SendWelcomeEmail on emails {
    perform(user_id: String) {
        let user = db.find_user(user_id)
        email.send(user.email, "Welcome!", "...")
    }
}
```

That's it. Defaults handle retry (3), timeout (30s), backoff (exponential).

### With Common Options

```ntnt
/// Charges customer credit card for completed orders
Job ProcessPayment on payments (retry: 5, timeout: 120s) {
    perform(order_id: String, amount: Float) {
        let order = db.find(order_id)
        stripe.charge(order.customer_id, amount)
    }
}
```

### Full-Featured Job

```ntnt
/// Syncs user data to external CRM
///
/// Triggers: user.updated, user.created
/// Affects: CRM records, analytics
Job SyncToCRM on integrations (priority: high) {
    retry: 5
    timeout: 60s
    rate: 100/minute
    concurrency: 5
    unique: args for 1h
    expires: 2h
    idempotent: true

    perform(user_id: String) {
        let user = db.find(user_id)
        crm.upsert(user)
    }

    on_failure(error, attempt) {
        alert.notify("CRM sync failed: {error}")
    }
}
```

### Syntax Reference

```
Job <Name> on <queue> [(<options>)] {
    [config]

    perform(<args>) {
        <body>
    }

    [hooks]
}
```

| Element | Required | Example |
|---------|----------|---------|
| `/// description` | Recommended | `/// Sends welcome email` |
| `on <queue>` | Yes | `on emails` |
| `(<options>)` | No | `(retry: 5, timeout: 60s)` |
| Config block | No | `rate: 10/second` |
| `perform()` | Yes | `perform(id: String) { ... }` |
| Hooks | No | `on_failure(e, n) { ... }` |

### Defaults

| Option | Default | Description |
|--------|---------|-------------|
| `retry` | `3` | Max attempts before dead letter |
| `timeout` | `30s` | Kill job if exceeds |
| `backoff` | `exponential` | Retry delay strategy |
| `priority` | `normal` | `low`, `normal`, `high` |
| `rate` | unlimited | Rate limit (e.g., `10/second`) |
| `concurrency` | unlimited | Max parallel executions |
| `unique` | none | Deduplication (`args`, `args for 1h`) |
| `expires` | never | Discard if older than this |
| `idempotent` | `false` | Enable idempotency checks |

### Doc Comment Metadata

Use `///` comments for structured metadata (parsed by tools and agents):

```ntnt
/// Brief description (first line)
///
/// Triggers: user.created, user.updated
/// Affects: external CRM, analytics
/// Side effects: API calls, database writes
Job SyncUser on integrations { ... }
```

| Field | Description |
|-------|-------------|
| First line | Job description (used in logs, dashboards, tests) |
| `Triggers:` | Events that cause this job to run |
| `Affects:` | Systems/data impacted by this job |
| `Side effects:` | External calls made (for simulation mode) |

This metadata is queryable: `ntnt jobs ask "what jobs affect the CRM?"`

### Enqueuing Jobs

```ntnt
// Enqueue immediately
let job_id = SendWelcomeEmail.enqueue(map { "user_id": "123" })

// With options override
SendWelcomeEmail.enqueue(map { "user_id": "123" }, map {
    "priority": "high",
    "delay": 300
})

// Schedule for specific time
SendWelcomeEmail.enqueue_at(tomorrow_9am, map { "user_id": "123" })

// Schedule with delay
SendWelcomeEmail.enqueue_in(3600, map { "user_id": "123" })  // 1 hour

// Check status
let status = Queue.status(job_id)

// Cancel
Queue.cancel(job_id)
```

---

## Queue Configuration

```ntnt
import { Queue } from "std/jobs"

Queue.configure(map {
    // Backend Selection
    "backend": env("JOB_BACKEND", "memory"),  // memory, postgres, redis
    "postgres_url": env("DATABASE_URL"),
    "redis_url": env("REDIS_URL"),

    // Worker Resilience
    "visibility_timeout": 300,     // Re-enqueue job if no heartbeat for 5 min
    "heartbeat_interval": 30,      // Worker heartbeats every 30s
    "shutdown_timeout": 30,        // Max seconds to wait for jobs on shutdown

    // Automatic Cleanup
    "prune_completed_after": 86400,   // Delete completed jobs after 24h
    "prune_failed_after": 604800,     // Keep failed jobs for 7 days (review)
    "prune_cancelled_after": 3600,    // Delete cancelled jobs after 1h

    // Monitoring (optional)
    "status_endpoint": "/jobs/status",  // Adds GET endpoint
    "status_bind": "127.0.0.1"          // Localhost only for security
})
```

---

## Worker Model

### Combined Mode (Simple Apps)

HTTP server and job worker in same process:

```ntnt
import { Queue } from "std/jobs"

// Configure
Queue.configure(map { "backend": "memory" })

// Define jobs...
Job SendEmail { ... }

// Start HTTP server
get("/", home_handler)
listen(8080)

// Process jobs alongside HTTP (non-blocking)
Queue.work_async()
```

### Separate Workers (Production)

Dedicated worker process for reliability:

```ntnt
// worker.tnt
import { Queue } from "std/jobs"

Queue.configure(map {
    "backend": "postgres",
    "postgres_url": env("DATABASE_URL")
})

// Import job definitions
import { SendEmail, ProcessPayment } from "./jobs.tnt"

// Start worker (blocking)
Queue.work(map {
    "queues": ["emails", "payments"],  // or omit for all queues
    "concurrency": 10                   // parallel job processing
})
```

### Queue Priority & Weighted Processing

For critical/high/low priority workloads, use separate queues:

```ntnt
/// Sends urgent system alerts
Job CriticalAlert on critical {
    perform(alert_id: String) { ... }
}

/// Standard background task
Job NormalTask on default {
    perform(task_id: String) { ... }
}

/// Low-priority batch report generation
Job BatchReport on low {
    perform(report_id: String) { ... }
}
```

**Strict priority** - process all critical before any default:

```ntnt
Queue.work(map {
    "queues": ["critical", "default", "low"]  // order matters
})
```

**Weighted priority** - prevents starvation of low-priority jobs:

```ntnt
Queue.work(map {
    "queues": map {
        "critical": 5,   // 5x weight
        "default": 3,    // 3x weight
        "low": 1         // 1x weight
    }
})
// Ratio: for every 5 critical, process 3 default, then 1 low
```

**Dedicated workers** - isolate critical work entirely:

```ntnt
// critical-worker.tnt - dedicated to critical queue only
Queue.work(map { "queues": ["critical"], "concurrency": 5 })

// general-worker.tnt - handles everything else
Queue.work(map { "queues": ["default", "low"], "concurrency": 10 })
```

This ensures critical jobs are never blocked by a backlog of low-priority work.

**When to use which:**

| Approach | Use When |
|----------|----------|
| Job `priority` field | Fine-grained ordering within a queue (e.g., premium vs free users in same email queue) |
| Separate queues | Different SLAs, dedicated workers, or isolation requirements |
| Both | Complex systems (critical queue + priority within each queue)

### Graceful Shutdown

Workers handle shutdown signals (SIGTERM, SIGINT) gracefully:

1. Stop accepting new jobs
2. Wait for in-progress jobs to complete (up to `shutdown_timeout`)
3. Release any incomplete jobs back to queue (for re-processing)
4. Exit cleanly

```ntnt
// Optional: custom shutdown hook
Queue.on_shutdown(fn() {
    print("Worker shutting down...")
    // Close connections, flush logs, etc.
})
```

**What happens on crash (no graceful shutdown):**
- In-progress jobs have no heartbeat
- After `visibility_timeout` seconds, jobs are automatically re-enqueued
- This is why jobs MUST be idempotent

---

## Job Lifecycle

```
                              ┌─────────────┐
                              │  Scheduled  │ (delayed jobs)
                              └──────┬──────┘
                                     │ (time reached)
                                     ▼
┌─────────┐    claim    ┌─────────────────┐    success    ┌───────────┐
│ Pending │────────────▶│     Active      │──────────────▶│ Completed │
└─────────┘             │ (locked+heartbeat)│              └───────────┘
     ▲                  └────────┬────────┘
     │                           │
     │ (retries left)            │ failure
     │                           ▼
     │                  ┌─────────────────┐
     └──────────────────│      Retry      │
                        │ (backoff delay) │
                        └────────┬────────┘
                                 │ (retries exhausted)
                                 ▼
┌───────────┐           ┌─────────────────┐
│ Cancelled │           │   Dead Letter   │
└───────────┘           │   (for review)  │
     ▲                  └─────────────────┘
     │
     │ (manual cancel or TTL expired)
```

### Job States

| State | Description |
|-------|-------------|
| `scheduled` | Delayed job waiting for scheduled time |
| `pending` | Ready to be processed |
| `active` | Claimed by worker, being processed |
| `completed` | Successfully finished |
| `retry` | Failed, waiting for retry (with backoff) |
| `dead` | Failed all retries, in dead-letter queue |
| `cancelled` | Manually cancelled or TTL expired |

---

## Resilience Features

### Heartbeats & Visibility Timeout

Workers send heartbeats while processing jobs. If a worker dies:

```
Worker A claims Job 123
  └─ heartbeat at t=0
  └─ heartbeat at t=30s
  └─ heartbeat at t=60s
  └─ [CRASH - no more heartbeats]

After visibility_timeout (300s) with no heartbeat:
  └─ Job 123 automatically released back to pending
  └─ Worker B can now claim it
```

This prevents jobs from being stuck forever when workers crash.

### Idempotency

**Critical:** Jobs WILL be retried. Design for at-least-once delivery.

```ntnt
/// Charges customer for order
Job ProcessPayment on payments {
    idempotent: true

    perform(order_id: String) {
        // Check if already processed
        if db.exists("SELECT 1 FROM payments WHERE order_id = $1", order_id) {
            return  // Already done, skip
        }

        // Process with idempotency key (Stripe supports this)
        stripe.charge(map {
            "amount": order.amount,
            "idempotency_key": "payment-{order_id}"
        })

        // Record completion
        db.insert("payments", map { "order_id": order_id })
    }
}
```

**Best practices:**
- Use database transactions with unique constraints
- Use external service idempotency keys (Stripe, etc.)
- Check if work already done before doing it
- Make side effects reversible or skippable

### Rate Limiting

Prevent overwhelming external services:

```ntnt
/// Calls rate-limited external API
Job CallExternalAPI on integrations {
    rate: 100/minute
    concurrency: 5

    perform(request_id: String) {
        external_api.call(request_id)
    }
}
```

Rate limits are enforced globally across all workers.

### Job Expiration

Don't process stale jobs:

```ntnt
/// Sends time-sensitive alert (discard if stale)
Job SendTimeSensitiveAlert on alerts {
    expires: 5m

    perform(alert_id: String) {
        // Won't run if queued > 5 minutes
    }
}
```

---

## Workflows & Composition

### Job Chains (Sequential)

Jobs that must run in order:

```ntnt
// Define a chain
Chain ProcessOrder {
    ValidateOrder -> ReserveInventory -> ChargePayment -> SendConfirmation
}

// Start the chain
ProcessOrder.start(map { "order_id": "123" })

// Each job receives the previous job's result
/// Validates order exists and is ready
Job ValidateOrder on orders {
    perform(order_id: String) -> Map {
        let order = db.find(order_id)
        return map { "order_id": order_id, "amount": order.amount }
    }
}

/// Charges the customer
Job ChargePayment on payments {
    perform(order_id: String, amount: Float) -> Map {
        let charge = stripe.charge(amount)
        return map { "charge_id": charge.id }
    }
}
```

### Workflows (DAG Dependencies)

Complex dependencies with fan-out and fan-in:

```ntnt
import { Workflow } from "std/jobs"

Workflow UserOnboarding {
    // Fan-out: CreateAccount triggers two parallel jobs
    CreateAccount -> SendWelcomeEmail
    CreateAccount -> SetupBilling

    // Fan-in: ActivateAccount waits for both to complete
    [SendWelcomeEmail, SetupBilling] -> ActivateAccount
}

// Start workflow
let workflow_id = UserOnboarding.start(map { "user_id": "123" })

// Check workflow status
Workflow.status(workflow_id)
// { "status": "running", "completed": ["CreateAccount"], "pending": ["SendWelcomeEmail", "SetupBilling"] }
```

### Batches (Parallel with Callback)

Process many jobs in parallel, run callback when all complete:

```ntnt
import { Batch } from "std/jobs"

// Create a batch
let batch = Batch.create(map {
    "on_complete": fn(results) {
        // Called when ALL jobs in batch complete
        let total = sum(results, fn(r) { r["count"] })
        db.update_total(total)
    },
    "on_failure": fn(errors) {
        // Called if ANY job fails (after retries)
        alert("Batch failed: {errors}")
    }
})

// Add jobs to batch
for chunk in data_chunks {
    batch.add(ProcessChunk, map { "chunk": chunk })
}

// Start batch processing
batch.run()
```

---

## In-Process Scheduling

Schedule recurring jobs without system cron:

```ntnt
import { every } from "std/concurrent"
import { Queue } from "std/jobs"

// Fetch data every hour
every(3600, fn() {
    FetchDataJob.enqueue(map {})
})

// Health check every 5 minutes
every(300, fn() {
    HealthCheckJob.enqueue(map {})
})

// Run immediately on startup
FetchDataJob.enqueue(map {})

// Start HTTP server and workers
listen(8080)
Queue.work_async()
```

| Aspect | System Cron | In-Process |
|--------|-------------|------------|
| Dependencies | Requires cron daemon | None |
| Deployment | Configure crontab separately | Single deploy |
| Portability | Unix only | Cross-platform |
| Visibility | Separate logs | Same process logs |
| Testing | Hard to test | Easy to test |

---

## Monitoring & Observability

### CLI Output (Always On)

Jobs log to stdout:

```
[2024-01-25 10:00:00] Job SendEmail enqueued (id: abc123)
[2024-01-25 10:00:01] Job SendEmail started (id: abc123, worker: w1)
[2024-01-25 10:00:02] Job SendEmail completed (id: abc123, duration: 1.2s)
[2024-01-25 10:05:00] Job FetchData failed (id: def456, attempt: 1/3, error: "timeout")
[2024-01-25 10:05:30] Job FetchData retrying (id: def456, attempt: 2/3, backoff: 30s)
```

### Programmatic Access

```ntnt
// Queue statistics
let stats = Queue.stats()
// { "pending": 12, "active": 3, "completed": 1547, "failed": 8, "dead": 2 }

// Per-queue breakdown
let queue_stats = Queue.stats("emails")
// { "pending": 5, "active": 1, ... }

// Recent jobs
let recent = Queue.recent(20)
// Array of job info: id, type, status, duration, error, etc.

// Failed jobs (dead letter queue)
let failed = Queue.dead(50)

// Retry a dead job
Queue.retry(job_id)

// Clear dead letter queue
Queue.clear_dead()
```

### Admin Dashboard Endpoint

```ntnt
fn admin_jobs_handler(req) {
    return json(map {
        "stats": Queue.stats(),
        "recent": Queue.recent(10),
        "dead": Queue.dead(10),
        "scheduled": Queue.scheduled(10)
    })
}

// Localhost only for security
get(r"/admin/jobs", admin_jobs_handler, map { "bind": "127.0.0.1" })
```

### CLI Commands

```bash
ntnt jobs status              # Summary of all queues
ntnt jobs list                # List recent jobs
ntnt jobs list --pending      # Filter by status
ntnt jobs list --failed
ntnt jobs list --dead
ntnt jobs inspect <job-id>    # Full job details
ntnt jobs retry <job-id>      # Retry a failed/dead job
ntnt jobs cancel <job-id>     # Cancel a pending job
ntnt jobs clear --dead        # Clear dead letter queue
ntnt jobs clear --completed   # Clear old completed jobs
```

### Security Model

| Access Method | Default | Security |
|--------------|---------|----------|
| CLI/stdout logs | Always on | Process owner only |
| `Queue.stats()` API | Always on | Code access only |
| `/jobs/status` endpoint | Off | Localhost only when enabled |
| `ntnt jobs` CLI | Always on | Requires shell access |

---

## Backend Implementations

### 1. In-Memory (Default)

```ntnt
Queue.configure(map { "backend": "memory" })
```

- Zero dependencies, instant setup
- Jobs lost on restart (fine for dev/transient tasks)
- Single-process only (no distributed workers)
- **Use for:** Development, testing, simple apps

### 2. PostgreSQL

```ntnt
Queue.configure(map {
    "backend": "postgres",
    "postgres_url": env("DATABASE_URL")
})
```

- Reliable, ACID transactions
- Job history preserved
- Multi-worker support with `SELECT FOR UPDATE SKIP LOCKED`
- **Use for:** Production apps already using Postgres

**Schema (auto-created):**

```sql
CREATE TABLE ntnt_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue VARCHAR(255) NOT NULL DEFAULT 'default',
    job_type VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    result JSONB,

    -- Status & Attempts
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    priority INT NOT NULL DEFAULT 0,
    attempts INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    error TEXT,

    -- Scheduling
    scheduled_at TIMESTAMPTZ DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,

    -- Resilience
    locked_by VARCHAR(255),          -- Worker ID
    locked_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,          -- TTL

    -- Deduplication
    idempotency_key VARCHAR(255),
    unique_key VARCHAR(255),
    unique_until TIMESTAMPTZ,

    -- Workflows
    workflow_id UUID,
    depends_on UUID[],

    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Indexes for efficient polling
CREATE INDEX idx_ntnt_jobs_pending ON ntnt_jobs(queue, priority DESC, scheduled_at)
    WHERE status = 'pending';
CREATE INDEX idx_ntnt_jobs_locked ON ntnt_jobs(locked_by, heartbeat_at)
    WHERE status = 'active';
CREATE INDEX idx_ntnt_jobs_workflow ON ntnt_jobs(workflow_id)
    WHERE workflow_id IS NOT NULL;
CREATE INDEX idx_ntnt_jobs_unique ON ntnt_jobs(unique_key)
    WHERE unique_key IS NOT NULL AND unique_until > NOW();
CREATE INDEX idx_ntnt_jobs_prune ON ntnt_jobs(completed_at)
    WHERE status IN ('completed', 'cancelled');
```

### 3. Redis / Valkey

```ntnt
Queue.configure(map {
    "backend": "redis",
    "redis_url": env("REDIS_URL")
})
```

- Very fast (10k+ jobs/second)
- Built-in expiration
- Pub/sub for real-time notifications
- **Use for:** High-throughput, real-time apps

**Note:** Valkey is the Linux Foundation's Redis fork. API-compatible, same driver works.

---

## IDD Integration

Jobs are testable in .intent files:

```intent
Feature: Welcome Email Job
  id: feature.welcome_email_job

  test:
    - job: SendWelcomeEmail
      args: { "user_id": "123" }
      given:
        - mock db.find_user returns { "id": "123", "email": "test@example.com" }
        - mock email.send returns { "sent": true }
      assert:
        - status: completed
        - email.send was called with "test@example.com"

    - job: SendWelcomeEmail
      args: { "user_id": "invalid" }
      given:
        - mock db.find_user throws "User not found"
      assert:
        - status: failed
        - error contains "User not found"
```

The job's `///` doc comment description is automatically used as the test description.

---

## Binary Size & Feature Flags

To avoid bloating the ntnt binary, backends are optional:

```toml
# Cargo.toml
[features]
default = ["jobs-memory"]           # Always included (~50KB)
jobs-memory = []                    # In-memory backend
jobs-postgres = ["dep:sqlx"]        # Adds ~2MB (reuses std/db/postgres)
jobs-redis = ["dep:redis"]          # Adds ~1MB
jobs-full = ["jobs-postgres", "jobs-redis"]
```

**Distribution options:**
- `ntnt` - Base binary with in-memory jobs
- `ntnt-full` - All backends included
- Or: users compile with needed features

The Job DSL parsing is always included (minimal overhead). Heavy code is in backends.

---

## Implementation Plan

### Phase 1: Core + In-Memory
1. Add `Job` declaration syntax to parser
2. Implement in-memory backend
3. Basic job methods (enqueue, perform, status)
4. Worker loop with retry logic
5. Graceful shutdown handling
6. Job cancellation
7. Semantic metadata (description, triggers, affects, side_effects)

### Phase 2: Resilience
1. Heartbeat system
2. Visibility timeout (re-enqueue stale jobs)
3. Rate limiting
4. Concurrency limits
5. Job TTL/expiration
6. Automatic pruning
7. Weighted queue processing

### Phase 3: PostgreSQL Backend
1. PostgreSQL backend implementation
2. Auto-migration for jobs table
3. Distributed locking (`SELECT FOR UPDATE SKIP LOCKED`)
4. Job history queries

### Phase 4: Composition
1. Job chains (sequential)
2. Workflows (DAG dependencies)
3. Batches with callbacks
4. Unique jobs (deduplication)

### Phase 5: Observability CLI
1. `ntnt jobs` CLI commands (list, status, inspect)
2. Live streaming (`ntnt jobs tail`)
3. Job replay (`ntnt jobs replay`)
4. Request tracing (`ntnt jobs trace`)
5. Agent-optimized output (`--format=agent`)

### Phase 6: Agent-First Features
1. Natural language queries (`ntnt jobs ask`)
2. AI-powered diagnosis (`ntnt jobs diagnose`)
3. Auto-generated test suggestions (`ntnt jobs suggest-tests`)
4. Impact analysis (`ntnt jobs impact`)

### Phase 7: Advanced Features
1. Simulation/dry-run mode
2. Job contracts (requires/ensures)
3. Intent verification
4. Idempotency static analysis (`ntnt lint`)
5. Time-travel inspection

### Phase 8: Redis Backend + Dashboard
1. Redis/Valkey backend
2. Optional web dashboard
3. Metrics/telemetry hooks

---

## Feature Checklist

### Core (Must Have)
- [ ] Job DSL parsing
- [ ] Enqueue / enqueue_at / enqueue_in
- [ ] Priority queues (job-level + queue-level)
- [ ] Retry with exponential backoff
- [ ] Timeout enforcement
- [ ] Dead letter queue
- [ ] Graceful shutdown
- [ ] Job cancellation
- [ ] In-memory backend

### Resilience (Must Have for Production)
- [ ] Worker heartbeats
- [ ] Visibility timeout (auto-release stale jobs)
- [ ] Rate limiting (per job type)
- [ ] Concurrency limits
- [ ] Job TTL/expiration
- [ ] Automatic pruning
- [ ] Weighted queue processing

### Composition (High Value)
- [ ] Job chains (sequential)
- [ ] Workflows (DAG dependencies)
- [ ] Batches with callbacks
- [ ] Unique jobs (deduplication)

### Backends
- [ ] In-memory (default)
- [ ] PostgreSQL
- [ ] Redis/Valkey

### Observability
- [ ] CLI commands (`ntnt jobs`)
- [ ] Programmatic stats API
- [ ] Status endpoint (opt-in)
- [ ] IDD integration (job testing)
- [ ] Live job streaming (`ntnt jobs tail`)
- [ ] Request tracing across jobs

### Agent-First (Differentiators)
- [ ] Semantic job metadata (description, triggers, affects)
- [ ] Natural language queries (`ntnt jobs ask`)
- [ ] AI-powered diagnosis (`ntnt jobs diagnose`)
- [ ] Auto-generated test suggestions
- [ ] Impact analysis (`ntnt jobs impact`)
- [ ] Agent-optimized output format

### Advanced (Revolutionary)
- [ ] Job simulation/dry-run mode
- [ ] Intent verification (did job achieve its purpose?)
- [ ] Job contracts (requires/ensures)
- [ ] Side effect declarations
- [ ] Idempotency static analysis
- [ ] Job replay for debugging
- [ ] Time-travel inspection

---

## Agent-First Design

NTNT's job system is designed for AI agents to write, debug, and operate.

### Semantic Job Metadata

Jobs are self-documenting with rich metadata agents can query:

```ntnt
/// Sends personalized welcome email to newly registered users
///
/// Triggers: user.registered, manual.admin_resend
/// Affects: email delivery, user engagement metrics
/// Side effects: sends email, updates user.last_emailed_at
Job SendWelcomeEmail on emails {
    idempotent: true

    perform(user_id: String) { ... }
}
```

Doc comments (`///`) are parsed as structured metadata that agents can query.

### Natural Language Queries

```bash
ntnt jobs ask "why are emails failing?"
ntnt jobs ask "what jobs touch the payments table?"
ntnt jobs ask "show me jobs slower than usual"
ntnt jobs ask "what happens when a user registers?"
```

### AI-Powered Diagnosis

```bash
ntnt jobs diagnose <job-id>

# Output:
# Job SendWelcomeEmail failed after 3 attempts
#
# Root Cause Analysis:
#   - SMTP connection timeout to smtp.example.com:587
#   - Last successful email: 2 hours ago
#   - Similar failures: 47 jobs in last 30 minutes
#
# Suggested Actions:
#   1. Check SMTP server status
#   2. Verify network connectivity
#   3. Review rate limits (10,000 emails sent today)
#
# Related: 47 pending jobs affected
```

### Auto-Generated Test Suggestions

```bash
ntnt jobs suggest-tests SendWelcomeEmail

# Based on code analysis, suggested test cases:
#
# 1. Happy path - valid user
#    assert: status completed, email.send called
#
# 2. Edge case - user not found
#    assert: status failed, error contains "not found"
#
# 3. Edge case - user has no email
#    assert: status failed, error contains "no email"
#
# Add to server.intent? [Y/n]
```

### Impact Analysis

```bash
ntnt jobs impact SendWelcomeEmail

# If SendWelcomeEmail fails:
#
# Direct impact:
#   - New users won't receive welcome email
#   - ~150 users/day affected (based on signup rate)
#
# Downstream jobs blocked:
#   - ActivateTrialSubscription (depends on email sent)
#   - TrackOnboardingMetrics (waits for email open)
#
# Business impact: MEDIUM
#   - User activation rate may decrease
#   - No direct revenue impact
```

### Agent-Optimized Output

```bash
ntnt jobs status --format=agent

# Compact, high-signal output for LLM context windows:
#
# QUEUES: critical(0/0/15) default(23/5/892) low(99/2/234)
#
# ANOMALIES:
# ⚠️ SendEmail: 3.2s avg (4x normal)
# ⚠️ ProcessPayment: 12% failures (normally 0.1%)
#
# ACTIONS:
# 1. Check SendEmail - likely SMTP timeout
# 2. Check ProcessPayment - see 12 unique errors
```

---

## Real-Time Visibility & Troubleshooting

### Live Job Streaming

```bash
# Tail a specific job's output
ntnt jobs tail <job-id>

# Stream all jobs in a queue
ntnt jobs tail --queue=critical

# Stream failures only
ntnt jobs tail --failed
```

### Job Replay (Reproduction)

Re-run a failed job with exact same inputs for debugging:

```bash
# Replay locally (for debugging)
ntnt jobs replay <job-id>

# Replay without side effects (dry-run)
ntnt jobs replay <job-id> --dry-run

# Replay with modified args
ntnt jobs replay <job-id> --args='{"user_id": "different"}'
```

### Time-Travel Inspection

See exactly what a job saw at execution time:

```bash
ntnt jobs inspect <job-id> --snapshot

# Shows:
# - Original args
# - Environment at execution time
# - Database queries and results
# - External API calls and responses
# - Timing breakdown
```

### Request Tracing

Follow a user request through the entire job chain:

```bash
ntnt jobs trace <request-id>

# HTTP POST /api/orders
#   └─ CreateOrder (job-123) ✓ 0.5s
#       └─ ValidateInventory (job-124) ✓ 0.2s
#       └─ ChargePayment (job-125) ✗ FAILED
#           └─ Error: Card declined
#       └─ SendConfirmation (job-126) ⏸ BLOCKED
```

---

## Simulation Mode

Dry-run jobs without side effects:

```bash
ntnt jobs simulate SendWelcomeEmail --args='{"user_id": "123"}'

# SIMULATION: SendWelcomeEmail(user_id: "123")
#
# Would execute:
#   1. db.find_user("123")
#      → { id: "123", email: "alice@example.com", name: "Alice" }
#   2. email.send("alice@example.com", "Welcome!")
#      → SIMULATED (no actual email)
#   3. db.update_user("123", { welcomed: true })
#      → SIMULATED (no db write)
#
# Estimated duration: 0.8s
# Side effects: 1 email, 1 db write
#
# Run for real? [y/N]
```

In code, use `effect` blocks to mark side effects (skipped in simulation):

```ntnt
/// Sends transactional email
/// Side effects: sends email, logs to analytics
Job SendEmail on emails {
    perform(user_id: String) {
        let user = db.find(user_id)

        effect "sends email" {
            email.send(user.email, subject, body)
        }

        effect "logs to analytics" {
            analytics.track("email_sent", user_id)
        }
    }
}
```

---

## Intent Verification

Jobs that verify they achieved their purpose, not just that they ran:

```ntnt
/// Sends welcome email to new users
Job SendWelcomeEmail on emails {
    // How long to wait for verification
    verify_timeout: 1h

    perform(user_id: String) -> Map {
        let user = db.find(user_id)
        let msg = email.send(user.email, "Welcome!", body)
        return map { "message_id": msg.id }
    }

    // Define what "success" really means
    verify(args, result) {
        let status = email.check_delivery(result["message_id"])
        return status == "delivered" || status == "opened"
    }
}
```

Job states now include:
- `completed` - Job ran without errors
- `verified` - Job achieved its intent (email was delivered)
- `unverified` - Job completed but intent not confirmed yet
- `intent_failed` - Job ran but didn't achieve intent (email bounced)

```bash
ntnt jobs list --unverified  # Jobs that ran but intent not confirmed
```

---

## Job Contracts

Pre/post conditions with static analysis:

```ntnt
/// Charges customer credit card
Job ProcessPayment on payments {
    idempotent: true

    // Checked before job runs
    requires(args) {
        args["amount"] > 0 && args["order_id"] != ""
    }

    // Checked after job completes
    ensures(args, result) {
        result["status"] in ["charged", "declined", "pending"]
    }

    perform(order_id: String, amount: Float) -> Map {
        // ...
    }
}
```

Static analysis catches issues:

```bash
ntnt lint server.tnt

# jobs.tnt:45 - ProcessPayment marked idempotent but uses db.insert()
#              Consider: db.upsert() or check-before-insert pattern
#
# jobs.tnt:78 - SendEmail has no timeout for external API call
#              Consider: adding timeout or circuit breaker
```

---

## Open Questions

1. **Progress reporting** - Should long jobs report progress?
   ```ntnt
   fn perform(large_file: String) {
       for i, chunk in enumerate(chunks) {
           process(chunk)
           Job.progress(i / len(chunks))  // 0.0 to 1.0
       }
   }
   ```

2. **Middleware/plugins** - Extensibility for logging, error tracking?
   ```ntnt
   Queue.use(fn(job, next) {
       let start = now()
       let result = next(job)
       metrics.record("job.duration", now() - start)
       return result
   })
   ```

3. **Checkpointing** - For very long jobs, save progress to resume on crash?

4. **Multi-tenant** - Job isolation between tenants in SaaS apps?

5. **AI integration architecture** - How should `ntnt jobs ask` work?
   - Local LLM inference?
   - API call to Claude/OpenAI?
   - Hybrid with local embeddings + cloud reasoning?

6. **Side effect mocking** - How does simulation mode intercept calls?
   - Compile-time rewriting of `side_effect` blocks?
   - Runtime flag that jobs check?
   - Dependency injection pattern?

7. **Intent verification timing** - When to verify async intents?
   - Background verification job?
   - Webhook from external service?
   - Polling with backoff?

---

## Why This System is Different

| Feature | Sidekiq/Bull/Oban | NTNT Jobs |
|---------|-------------------|-----------|
| Job definition | Code + config | Declarative DSL with metadata |
| Testing | Manual test setup | IDD integration, auto-generated tests |
| Debugging | Log diving | AI diagnosis, job replay, simulation |
| Documentation | External docs | Self-documenting (description, triggers, affects) |
| Side effects | Implicit | Explicit declarations |
| Success criteria | "Did it run?" | "Did it achieve intent?" |
| Agent support | None | First-class (queries, impact analysis) |
| Idempotency | Developer discipline | Static analysis + runtime checks |

NTNT's job system treats jobs as **intentional units of work** rather than just functions to execute. This aligns with the IDD philosophy: declare what you want to achieve, verify it was achieved, and make the system understandable to both humans and AI agents.
