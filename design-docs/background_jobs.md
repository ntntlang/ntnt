# Background Jobs / Job Queuing System

## Overview

Add a background job system to NTNT that enables async task processing, scheduled jobs, and reliable task queuing. The system should work out-of-the-box with in-memory storage, but allow plugging in persistent backends (Redis, PostgreSQL, Valkey) for production use.

## Goals

1. **Zero-config start** - Works immediately with in-memory queue
2. **Pluggable backends** - Swap storage without changing job code
3. **Simple API** - Easy to define and enqueue jobs
4. **Reliable** - Retries, dead-letter queues, job status tracking
5. **Observable** - Job status, history, metrics
6. **Intent-driven** - Align with NTNT's IDD philosophy

---

## Design Options

### Option A: Backend Abstraction Layer

Functional approach - register handlers, enqueue by string name.

```ntnt
import { Queue } from "std/jobs"

// Configure backend (optional - defaults to in-memory)
Queue.configure(map {
    "backend": "memory"  // or "redis", "postgres", "valkey"
})

// Define a job handler
fn send_welcome_email(user_id: String) {
    let user = db.find_user(user_id)
    email.send(user.email, "Welcome!", "...")
}

// Register handler with options
Queue.register("send_welcome_email", send_welcome_email, map {
    "retry": 3,
    "timeout": 60,
    "queue": "emails"
})

// Enqueue by string name
Queue.enqueue("send_welcome_email", map { "user_id": "123" })

// Start worker
Queue.work()
```

**Pros:**
- Familiar pattern from other languages (Celery, Sidekiq, Bull)
- Flexible - can register any function
- Simple to implement

**Cons:**
- Magic strings for job names (typos cause runtime errors)
- Configuration separate from job definition
- Harder to discover all jobs in codebase
- Jobs are just functions, not entities

---

### Option C: Job DSL (Recommended)

Declarative approach - jobs are first-class entities with co-located configuration.

```ntnt
import { Job, Queue } from "std/jobs"

// Jobs are declared entities with configuration
Job SendWelcomeEmail {
    queue: "emails"
    retry: 3
    timeout: 60s
    priority: normal

    fn perform(user_id: String) {
        let user = db.find_user(user_id)
        email.send(user.email, "Welcome!", "...")
    }
}

Job ProcessPayment {
    queue: "payments"
    retry: 5
    timeout: 120s
    priority: high
    unique: true  // prevent duplicate jobs

    fn perform(order_id: String, amount: Float) {
        let order = db.find_order(order_id)
        payment.charge(order.customer_id, amount)
    }
}

// Enqueue with type-safe reference
SendWelcomeEmail.enqueue(map { "user_id": "123" })

// Or with additional options
SendWelcomeEmail.enqueue(map { "user_id": "123" }, map {
    "delay": 300  // override or add options
})

// Schedule for later
ProcessPayment.enqueue_at(tomorrow_9am, map {
    "order_id": "456",
    "amount": 99.99
})

// Start workers
Queue.work()
```

**Backend Configuration (same as Option A):**

```ntnt
// In server.tnt or config
Queue.configure(map {
    "backend": env("JOB_BACKEND", "memory"),
    "redis_url": env("REDIS_URL"),
    "postgres_url": env("DATABASE_URL")
})
```

**Pros:**
- **Self-documenting** - All config co-located with handler
- **Type-safe** - No magic strings, parameters validated at definition
- **Discoverable** - Easy to find all jobs (`grep "Job "`)
- **IDD integration** - Jobs can be tested in .intent files
- **First-class entities** - `SendWelcomeEmail.enqueue()` not `"send_welcome_email"`
- **Aligned with NTNT philosophy** - Declarative, intent-driven

**Cons:**
- New syntax to learn
- Slightly more complex parser implementation

---

## Comparison

| Aspect | Option A (Functional) | Option C (Declarative) |
|--------|----------------------|------------------------|
| Job reference | `"send_welcome_email"` (string) | `SendWelcomeEmail` (entity) |
| Configuration | Separate from handler | Co-located with handler |
| Type safety | Runtime errors | Compile-time validation |
| Discoverability | Search for `register()` calls | Search for `Job ` declarations |
| IDD integration | Harder | Natural fit |
| Learning curve | Familiar | New pattern (but NTNT-like) |
| Implementation | Simpler | More parser work |

---

## IDD Integration (Option C)

Jobs as entities enable intent-driven testing. Here's how it works:

### How Job Testing Works

When you run `ntnt intent check`, the test runner:

1. Starts a test worker in the background
2. Enqueues the job with the specified args
3. Waits for the job to complete (or fail/timeout)
4. Asserts on the job's final state

```
┌─────────────────────────────────────────────────────────────┐
│                    ntnt intent check                         │
│                                                              │
│  1. Parse .intent file                                       │
│  2. Find job tests                                           │
│  3. For each job test:                                       │
│     ┌─────────────────────────────────────────────────────┐ │
│     │ a. Enqueue job with test args                       │ │
│     │ b. Start worker (processes job synchronously)       │ │
│     │ c. Capture: status, duration, attempts, output      │ │
│     │ d. Run assertions                                   │ │
│     │ e. Report pass/fail                                 │ │
│     └─────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### Basic Job Testing

```intent
# server.intent

## Module: Background Jobs

Feature: Welcome Email Job
  id: feature.welcome_email_job
  description: "Sends welcome email to new users"

  test:
    - job: SendWelcomeEmail
      args: { "user_id": "test-user-123" }
      assert:
        - status: completed
        - duration < 5s
```

### Testing with Mocked Dependencies

Jobs often have side effects (send email, charge card, call API). Use `given` to set up mocks:

```intent
Feature: Welcome Email Job
  id: feature.welcome_email_job

  test:
    - job: SendWelcomeEmail
      given:
        - mock email.send returns { "sent": true }
        - mock db.find_user returns { "id": "123", "email": "test@example.com" }
      args: { "user_id": "123" }
      assert:
        - status: completed
        - email.send was called with "test@example.com"
```

The `given` clause sets up test doubles before the job runs. The assertions can verify:
- The job completed successfully
- The mocked function was called with expected arguments

### Testing Failure Scenarios

```intent
Feature: Payment Processing
  id: feature.payment_job

  test:
    # Happy path
    - job: ProcessPayment
      given:
        - mock payment.charge returns { "success": true }
      args: { "order_id": "order-123", "amount": 50.00 }
      assert:
        - status: completed

    # Payment declined - should fail after retries
    - job: ProcessPayment
      given:
        - mock payment.charge throws "Card declined"
      args: { "order_id": "order-456", "amount": 100.00 }
      assert:
        - status: failed
        - attempts: 5
        - error contains "Card declined"

    # Invalid amount - should fail immediately (no retry)
    - job: ProcessPayment
      args: { "order_id": "order-789", "amount": -1 }
      assert:
        - status: failed
        - attempts: 1
        - error contains "Invalid amount"
```

### Testing Retry Behavior

```intent
Feature: Flaky API Integration
  id: feature.flaky_api_job

  test:
    # Succeeds on 3rd attempt
    - job: SyncWithExternalAPI
      given:
        - mock api.sync fails 2 times then returns { "synced": true }
      args: { "account_id": "acc-123" }
      assert:
        - status: completed
        - attempts: 3

    # Exhausts all retries
    - job: SyncWithExternalAPI
      given:
        - mock api.sync always throws "Service unavailable"
      args: { "account_id": "acc-456" }
      assert:
        - status: failed
        - attempts: 5  # max retries
        - error contains "Service unavailable"
```

### Testing Scheduled/Delayed Jobs

```intent
Feature: Reminder Emails
  id: feature.reminder_job

  test:
    # Job should not run immediately when delayed
    - job: SendReminder
      args: { "user_id": "123" }
      options: { "delay": 3600 }  # 1 hour delay
      assert:
        - status: scheduled
        - scheduled_for > now

    # When processed, should complete
    - job: SendReminder
      given:
        - mock email.send returns { "sent": true }
      args: { "user_id": "123" }
      options: { "run_immediately": true }  # bypass delay for testing
      assert:
        - status: completed
```

### Testing Job Output/Results

Jobs can return values that are stored with the job:

```ntnt
Job GenerateReport {
    queue: "reports"

    fn perform(report_type: String) -> Map {
        let data = gather_data(report_type)
        let url = upload_to_s3(data)
        return map { "url": url, "rows": len(data) }
    }
}
```

```intent
Feature: Report Generation
  id: feature.report_job

  test:
    - job: GenerateReport
      given:
        - mock gather_data returns [1, 2, 3, 4, 5]
        - mock upload_to_s3 returns "https://s3.example.com/report.csv"
      args: { "report_type": "sales" }
      assert:
        - status: completed
        - result.url: "https://s3.example.com/report.csv"
        - result.rows: 5
```

### Full Example

```intent
# server.intent

## Module: User Onboarding Jobs

Feature: Send Welcome Email
  id: feature.welcome_email
  description: "Sends personalized welcome email to new users"

  test:
    - job: SendWelcomeEmail
      given:
        - mock db.find_user returns { "id": "u1", "name": "Alice", "email": "alice@test.com" }
        - mock email.send returns { "message_id": "msg-123" }
      args: { "user_id": "u1" }
      assert:
        - status: completed
        - duration < 2s
        - email.send was called with "alice@test.com", "Welcome Alice!"

---

Feature: Create Trial Subscription
  id: feature.trial_subscription
  description: "Sets up 14-day trial for new users"

  test:
    - job: CreateTrialSubscription
      given:
        - mock billing.create_trial returns { "subscription_id": "sub-123" }
      args: { "user_id": "u1", "plan": "pro" }
      assert:
        - status: completed
        - result.subscription_id exists

    - job: CreateTrialSubscription
      given:
        - mock billing.create_trial throws "User already has subscription"
      args: { "user_id": "existing-user", "plan": "pro" }
      assert:
        - status: failed
        - error contains "already has subscription"
        - attempts: 1  # should not retry business logic errors
```

### Running Job Tests

```bash
# Run all tests including job tests
ntnt intent check server.tnt

# Output:
# ✓ feature.welcome_email - Send Welcome Email (0.8s)
#   ✓ Job SendWelcomeEmail completed in 0.3s
# ✓ feature.trial_subscription - Create Trial Subscription (1.2s)
#   ✓ Job CreateTrialSubscription completed in 0.4s
#   ✓ Job CreateTrialSubscription failed as expected
#
# 3 tests passed, 0 failed
```

---

## Backend Implementations

### 1. In-Memory (Default)

```
┌─────────────────────────────────────────────┐
│              In-Memory Queue                │
│  ┌─────────┐  ┌─────────┐  ┌─────────┐     │
│  │ Pending │  │ Active  │  │  Done   │     │
│  │  Jobs   │  │  Jobs   │  │  Jobs   │     │
│  └─────────┘  └─────────┘  └─────────┘     │
└─────────────────────────────────────────────┘
```

- Uses Rust channels or VecDeque
- Jobs lost on restart (fine for dev/simple use)
- Good for: development, testing, simple apps, transient tasks

### 2. PostgreSQL

```sql
CREATE TABLE background_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue VARCHAR(255) NOT NULL DEFAULT 'default',
    job_type VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    priority INT NOT NULL DEFAULT 0,
    attempts INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    scheduled_at TIMESTAMP WITH TIME ZONE,
    started_at TIMESTAMP WITH TIME ZONE,
    completed_at TIMESTAMP WITH TIME ZONE,
    failed_at TIMESTAMP WITH TIME ZONE,
    error TEXT,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW()
);

CREATE INDEX idx_jobs_queue_status ON background_jobs(queue, status, scheduled_at);
```

- Reliable, ACID transactions
- Job history preserved
- Good for: production apps already using Postgres, audit requirements

### 3. Redis / Valkey

```
Keys:
  ntnt:jobs:pending:<queue>     - Sorted set (score = priority/time)
  ntnt:jobs:active:<queue>      - Set of active job IDs
  ntnt:jobs:job:<id>            - Hash with job data
  ntnt:jobs:dead                - Dead letter queue
```

- Very fast
- Built-in expiration
- Pub/sub for real-time notifications
- Good for: high-throughput, real-time apps

**Note on Valkey:** Valkey is the Linux Foundation's open-source Redis fork (after Redis license change). API-compatible with Redis, same driver works for both.

---

## Job Lifecycle

```
┌─────────┐    ┌─────────┐    ┌─────────┐
│ Pending │───▶│ Active  │───▶│Complete │
└─────────┘    └─────────┘    └─────────┘
                    │
                    │ (on failure)
                    ▼
              ┌─────────┐    ┌─────────┐
              │  Retry  │───▶│  Dead   │
              │ (backoff)│    │ Letter  │
              └─────────┘    └─────────┘
                    │
                    │ (if retries left)
                    ▼
              ┌─────────┐
              │ Pending │
              └─────────┘
```

---

## Features

### Core Features
- [x] **Enqueue** - Add job to queue
- [x] **Delay** - Schedule job for later
- [x] **Priority** - High/normal/low priority
- [x] **Retry** - Automatic retry with exponential backoff
- [x] **Timeout** - Kill jobs that run too long
- [x] **Dead Letter** - Failed jobs preserved for inspection

### Advanced Features
- [ ] **Unique Jobs** - Prevent duplicate jobs (by args hash)
- [ ] **Batch** - Enqueue multiple jobs atomically
- [ ] **Cron** - Recurring scheduled jobs
- [ ] **Cancel** - Cancel pending jobs
- [ ] **Pause/Resume** - Pause queue processing
- [ ] **Callbacks** - on_success, on_failure, on_retry hooks

---

## API Design

### Job Declaration

```ntnt
Job JobName {
    // Configuration (all optional with defaults)
    queue: "default"        // which queue to use
    retry: 3                // max retry attempts
    timeout: 30s            // max execution time
    priority: normal        // high, normal, low
    unique: false           // prevent duplicates

    // Required: the job handler
    fn perform(arg1: Type, arg2: Type) {
        // job logic
    }

    // Optional: lifecycle hooks
    fn on_success() { }
    fn on_failure(error: String) { }
    fn on_retry(attempt: Int) { }
}
```

### Job Methods (auto-generated)

```ntnt
// Enqueue immediately
JobName.enqueue(map { "arg1": value, "arg2": value }) -> String  // returns job ID

// Enqueue with options
JobName.enqueue(args, map { "delay": 60, "priority": "high" }) -> String

// Schedule for specific time
JobName.enqueue_at(datetime, args) -> String

// Schedule with delay
JobName.enqueue_in(seconds, args) -> String

// Get job status
JobName.status(job_id) -> Map
```

### Queue Management

```ntnt
import { Queue } from "std/jobs"

// Configuration
Queue.configure(map {
    "backend": "postgres",
    "postgres_url": env("DATABASE_URL")
})

// Start workers
Queue.work()                           // all queues, blocking
Queue.work(["emails", "payments"])     // specific queues
Queue.work_async()                     // non-blocking (with HTTP server)

// Management
Queue.pause("emails")
Queue.resume("emails")
Queue.clear("emails")
Queue.stats() -> Map  // counts by queue/status
```

### Cron/Recurring Jobs

```ntnt
import { Cron } from "std/jobs"

Cron.schedule("daily_report", "0 9 * * *", DailyReportJob)
Cron.schedule("cleanup", "0 0 * * 0", CleanupJob)

// Or inline
Cron.schedule("heartbeat", "*/5 * * * *", fn() {
    health.ping()
})

Cron.start()
```

---

## Worker Model

```ntnt
// Option 1: Combined (simple apps)
// HTTP server + job worker in same process
listen(8080)
Queue.work_async()  // processes jobs between requests

// Option 2: Separate (production)
// worker.tnt - dedicated worker process
Queue.configure(map { "backend": "postgres", ... })
Queue.work()  // blocking, only processes jobs
```

---

## Implementation Plan

### Phase 1: Core + In-Memory
1. Add `Job` declaration syntax to parser
2. Implement in-memory backend
3. Basic job methods (enqueue, perform)
4. Worker loop with retry logic
5. Job status tracking

### Phase 2: PostgreSQL Backend
1. PostgreSQL backend implementation
2. Auto-migration for jobs table
3. Job history queries
4. Distributed locking (SELECT FOR UPDATE)

### Phase 3: Redis/Valkey Backend
1. Redis backend implementation
2. Valkey compatibility testing
3. Sorted sets for priority queues

### Phase 4: Advanced Features
1. Cron/recurring jobs
2. Unique jobs (deduplication)
3. Batch enqueueing
4. IDD integration (job testing in .intent files)

### Phase 5: Observability
1. Job metrics (duration, success rate)
2. Optional web dashboard
3. Webhook notifications

---

## Open Questions

1. **Worker concurrency** - How many jobs to process in parallel? Should be configurable:
   ```ntnt
   Queue.work(map { "concurrency": 5 })
   ```

2. **Job serialization** - Jobs stored as JSON payload. Complex types need serialization strategy.

3. **Distributed locking** - PostgreSQL uses `SELECT FOR UPDATE SKIP LOCKED`. Redis uses `SETNX`.

4. **Web UI** - Should we include a job dashboard? Could be a separate `ntnt jobs` CLI command or web UI.

---

## Recommendation

**Option C (Job DSL)** is recommended because:

1. **Aligned with NTNT's philosophy** - Declarative, intent-driven, self-documenting
2. **First-class jobs** - Type-safe references, no magic strings
3. **IDD integration** - Jobs can be tested like features in .intent files
4. **Better DX** - Configuration co-located with code, easy to discover
5. **Future-proof** - Jobs as entities enable richer tooling (IDE support, documentation generation)

The trade-off is slightly more parser work, but the result is a more NTNT-native experience that treats jobs as first-class citizens rather than just registered functions.
