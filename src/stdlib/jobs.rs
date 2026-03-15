//! std/jobs module — Job DSL with pluggable backends (memory + postgres)
//!
//! Provides background job processing with queues, retry logic, and lifecycle management.
//! Supports in-memory (default) and PostgreSQL (persistent, distributed) backends.
//!
//! ```ntnt
//! Job SendEmail on emails (retry: 3, timeout: 60s) {
//!     perform(to: String, subject: String) {
//!         fetch("https://api.email.com/send", map { "method": "POST", "json": map { "to": to, "subject": subject } })
//!     }
//!     on_failure(error, attempt) {
//!         print("Email failed: #{error}, attempt #{attempt}")
//!     }
//! }
//!
//! // Enqueue a job
//! let job_id = SendEmail.enqueue(map { "to": "user@example.com", "subject": "Hello" })
//!
//! // Start processing
//! Queue.work_async()
//! ```

use crate::ast::{Block, Parameter};
use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::concurrent::SerializedValue;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, IntentError>;

// ============================================================
// Job Definition (registered at declaration time)
// ============================================================

/// A registered job type — stores the AST for later execution
#[derive(Debug, Clone)]
pub struct JobDefinition {
    pub name: String,
    pub queue: String,
    pub max_retries: i64,
    pub timeout_ms: Option<u64>,
    pub backoff_base_ms: u64,
    pub perform_params: Vec<Parameter>,
    pub perform_body: Block,
    pub on_failure: Option<(Vec<Parameter>, Block)>,
}

/// Global registry of job definitions (populated when Job declarations are evaluated)
pub static JOB_REGISTRY: std::sync::LazyLock<Mutex<HashMap<String, JobDefinition>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register a job definition
pub fn register_job(def: JobDefinition) -> Result<()> {
    let mut registry = JOB_REGISTRY
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock job registry: {}", e)))?;
    registry.insert(def.name.clone(), def);
    Ok(())
}

/// Get a job definition by name
pub fn get_job_definition(name: &str) -> Result<Option<JobDefinition>> {
    let registry = JOB_REGISTRY
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock job registry: {}", e)))?;
    Ok(registry.get(name).cloned())
}

// ============================================================
// Job States and Queued Jobs
// ============================================================

/// Possible states of a queued job
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Pending,
    Active,
    Completed,
    Retry,
    Dead,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Active => "active",
            JobStatus::Completed => "completed",
            JobStatus::Retry => "retry",
            JobStatus::Dead => "dead",
            JobStatus::Cancelled => "cancelled",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(JobStatus::Pending),
            "active" => Some(JobStatus::Active),
            "completed" => Some(JobStatus::Completed),
            "retry" => Some(JobStatus::Retry),
            "dead" => Some(JobStatus::Dead),
            "cancelled" => Some(JobStatus::Cancelled),
            _ => None,
        }
    }
}

impl std::fmt::Display for JobStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

static JOB_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A job instance that has been enqueued
#[derive(Debug, Clone)]
pub struct QueuedJob {
    pub id: String,
    pub job_type: String,
    pub queue_name: String,
    pub args: HashMap<String, SerializedValue>,
    pub priority: i64,
    pub status: JobStatus,
    pub attempt_count: i64,
    pub max_attempts: i64,
    pub scheduled_at: Instant,
    pub error: Option<String>,
    pub created_at_ms: u64,
}

impl PartialEq for QueuedJob {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

impl Eq for QueuedJob {}

impl PartialOrd for QueuedJob {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for QueuedJob {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first, then earlier scheduled_at
        self.priority
            .cmp(&other.priority)
            .then_with(|| other.scheduled_at.cmp(&self.scheduled_at))
    }
}

// ============================================================
// Backend Abstraction
// ============================================================

/// Which backend is active for the job queue system
#[derive(Debug, Clone)]
pub enum BackendKind {
    Memory,
    Postgres(PostgresBackendConfig),
}

/// Configuration for the PostgreSQL backend
#[derive(Debug, Clone)]
pub struct PostgresBackendConfig {
    pub connection_url: String,
    pub heartbeat_interval_secs: u64,
    pub visibility_timeout_secs: u64,
}

impl Default for PostgresBackendConfig {
    fn default() -> Self {
        PostgresBackendConfig {
            connection_url: String::new(),
            heartbeat_interval_secs: 30,
            visibility_timeout_secs: 300, // 5 minutes
        }
    }
}

/// Global backend selection — defaults to Memory
static ACTIVE_BACKEND: std::sync::LazyLock<Mutex<BackendKind>> =
    std::sync::LazyLock::new(|| Mutex::new(BackendKind::Memory));

/// Global postgres pool for the job backend (separate from std/db/postgres pools)
static JOB_PG_POOL: std::sync::LazyLock<Mutex<Option<deadpool_postgres::Pool>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Get the current backend kind
fn get_backend() -> Result<BackendKind> {
    let backend = ACTIVE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock backend: {}", e)))?;
    Ok(backend.clone())
}

/// Set the active backend
fn set_backend(kind: BackendKind) -> Result<()> {
    let mut backend = ACTIVE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock backend: {}", e)))?;
    *backend = kind;
    Ok(())
}

// ============================================================
// Queue Backend (In-Memory)
// ============================================================

/// Queue configuration
#[derive(Debug, Clone)]
pub struct QueueConfig {
    pub shutdown_timeout_ms: u64,
    pub prune_completed_after_ms: u64,
}

impl Default for QueueConfig {
    fn default() -> Self {
        QueueConfig {
            shutdown_timeout_ms: 30_000,
            prune_completed_after_ms: 3_600_000, // 1 hour
        }
    }
}

/// Global queue state
pub struct QueueBackend {
    /// Jobs organized by queue name, sorted by priority + schedule time
    pub queues: HashMap<String, VecDeque<QueuedJob>>,
    /// All jobs by ID (for lookups)
    pub jobs_by_id: HashMap<String, QueuedJob>,
    /// Dead letter queue (failed jobs that exhausted retries)
    pub dead_jobs: Vec<QueuedJob>,
    /// Completed jobs (for pruning)
    pub completed_jobs: Vec<(Instant, QueuedJob)>,
    /// Configuration
    pub config: QueueConfig,
}

impl QueueBackend {
    pub fn new() -> Self {
        QueueBackend {
            queues: HashMap::new(),
            jobs_by_id: HashMap::new(),
            dead_jobs: Vec::new(),
            completed_jobs: Vec::new(),
            config: QueueConfig::default(),
        }
    }
}

/// Global queue backend
pub static QUEUE_BACKEND: std::sync::LazyLock<Arc<Mutex<QueueBackend>>> =
    std::sync::LazyLock::new(|| Arc::new(Mutex::new(QueueBackend::new())));

/// Flag to stop accepting new jobs during shutdown
static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

/// Flag indicating the worker loop is running
static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Flag to request worker stop
static WORKER_STOP: AtomicBool = AtomicBool::new(false);

// ============================================================
// PostgreSQL Backend
// ============================================================

/// Auto-migration SQL for the ntnt_jobs table
const PG_MIGRATION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS ntnt_jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    queue VARCHAR(255) NOT NULL DEFAULT 'default',
    job_type VARCHAR(255) NOT NULL,
    payload JSONB NOT NULL,
    result JSONB,
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    priority INT NOT NULL DEFAULT 0,
    attempts INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 3,
    error TEXT,
    scheduled_at TIMESTAMPTZ DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    locked_by VARCHAR(255),
    locked_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_ntnt_jobs_pending
    ON ntnt_jobs(queue, priority DESC, scheduled_at)
    WHERE status = 'pending';
CREATE INDEX IF NOT EXISTS idx_ntnt_jobs_locked
    ON ntnt_jobs(locked_by, heartbeat_at)
    WHERE status = 'active';
"#;

/// Initialize the postgres pool for jobs and run migrations
fn pg_init_job_pool(connection_url: &str) -> Result<()> {
    use deadpool_postgres::{Config, ManagerConfig, RecyclingMethod, Runtime};
    use tokio_postgres::NoTls;

    let mut cfg = Config::new();
    cfg.url = Some(connection_url.to_string());
    cfg.manager = Some(ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    });
    let pool_size: usize = std::env::var("NTNT_JOB_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    cfg.pool = Some(deadpool_postgres::PoolConfig {
        max_size: pool_size,
        ..Default::default()
    });

    let pool = cfg
        .create_pool(Some(Runtime::Tokio1), NoTls)
        .map_err(|e| IntentError::runtime_error(format!("Failed to create job pool: {}", e)))?;

    // Run migrations
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;
    db_rt.block_on(async {
        let client = pool.get().await.map_err(|e| {
            IntentError::runtime_error(format!("Failed to get connection for migration: {}", e))
        })?;
        client.batch_execute(PG_MIGRATION_SQL).await.map_err(|e| {
            IntentError::runtime_error(format!("Failed to run job migration: {}", e))
        })?;
        Ok::<(), IntentError>(())
    })?;

    // Store pool
    let mut pool_guard = JOB_PG_POOL
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock job pool: {}", e)))?;
    *pool_guard = Some(pool);

    Ok(())
}

/// Get the job postgres pool
fn get_job_pool() -> Result<deadpool_postgres::Pool> {
    let pool_guard = JOB_PG_POOL
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock job pool: {}", e)))?;
    pool_guard
        .clone()
        .ok_or_else(|| IntentError::runtime_error("Postgres job pool not initialized".to_string()))
}

/// Initialize postgres pool from a connection URL (for CLI usage)
pub fn pg_init_pool_from_url(connection_url: &str) -> Result<()> {
    pg_init_job_pool(connection_url)
}

/// Enqueue a job to postgres
fn pg_enqueue_job(
    job_type: &str,
    queue: &str,
    args: &HashMap<String, SerializedValue>,
    max_attempts: i64,
    priority: i64,
    scheduled_at_offset_ms: Option<u64>,
    scheduled_at_timestamp_ms: Option<u64>,
) -> Result<String> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    // Serialize args to JSON
    let args_map = SerializedValue::Map(args.clone());
    let payload_json = args_map.to_json();

    db_rt.block_on(async {
        let client = pool.get().await.map_err(|e| {
            IntentError::runtime_error(format!("Failed to get connection: {}", e))
        })?;

        let (sql, job_id_result) = if let Some(offset_ms) = scheduled_at_offset_ms {
            let interval = format!("{} milliseconds", offset_ms);
            let row = client
                .query_one(
                    "INSERT INTO ntnt_jobs (job_type, queue, payload, max_attempts, priority, scheduled_at) \
                     VALUES ($1, $2, $3, $4, $5, NOW() + $6::interval) RETURNING id::text",
                    &[
                        &job_type,
                        &queue,
                        &payload_json,
                        &(max_attempts as i32),
                        &(priority as i32),
                        &interval,
                    ],
                )
                .await
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to enqueue job: {}", e))
                })?;
            let id: String = row.get(0);
            (String::new(), id)
        } else if let Some(ts_ms) = scheduled_at_timestamp_ms {
            let secs = (ts_ms / 1000) as i64;
            let nsecs = ((ts_ms % 1000) * 1_000_000) as u32;
            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsecs)
                .unwrap_or_else(chrono::Utc::now);
            let row = client
                .query_one(
                    "INSERT INTO ntnt_jobs (job_type, queue, payload, max_attempts, priority, scheduled_at) \
                     VALUES ($1, $2, $3, $4, $5, $6) RETURNING id::text",
                    &[
                        &job_type,
                        &queue,
                        &payload_json,
                        &(max_attempts as i32),
                        &(priority as i32),
                        &dt,
                    ],
                )
                .await
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to enqueue job: {}", e))
                })?;
            let id: String = row.get(0);
            (String::new(), id)
        } else {
            let row = client
                .query_one(
                    "INSERT INTO ntnt_jobs (job_type, queue, payload, max_attempts, priority) \
                     VALUES ($1, $2, $3, $4, $5) RETURNING id::text",
                    &[
                        &job_type,
                        &queue,
                        &payload_json,
                        &(max_attempts as i32),
                        &(priority as i32),
                    ],
                )
                .await
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to enqueue job: {}", e))
                })?;
            let id: String = row.get(0);
            (String::new(), id)
        };
        let _ = sql; // suppress unused warning

        Ok(job_id_result)
    })
}

/// Claim the next job from postgres using SELECT FOR UPDATE SKIP LOCKED
fn pg_claim_next_job(queues: &[String], worker_id: &str) -> Result<Option<PgJobRow>> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        // Build a transaction for claim
        client
            .execute("BEGIN", &[])
            .await
            .map_err(|e| IntentError::runtime_error(format!("BEGIN failed: {}", e)))?;

        // Try each queue in order
        let mut claimed: Option<PgJobRow> = None;

        for queue in queues {
            let rows = client
                .query(
                    "SELECT id, job_type, payload, priority, attempts, max_attempts, queue \
                     FROM ntnt_jobs \
                     WHERE status = 'pending' AND queue = $1 AND scheduled_at <= NOW() \
                     ORDER BY priority DESC, scheduled_at ASC \
                     LIMIT 1 \
                     FOR UPDATE SKIP LOCKED",
                    &[&queue.as_str()],
                )
                .await
                .map_err(|e| IntentError::runtime_error(format!("Claim query failed: {}", e)))?;

            if let Some(row) = rows.first() {
                let id: uuid::Uuid = row.get(0);
                let job_type: String = row.get(1);
                let payload: serde_json::Value = row.get(2);
                let priority: i32 = row.get(3);
                let attempts: i32 = row.get(4);
                let max_attempts: i32 = row.get(5);
                let queue_name: String = row.get(6);

                // Lock the job
                client
                    .execute(
                        "UPDATE ntnt_jobs SET status = 'active', locked_by = $1, \
                         locked_at = NOW(), started_at = NOW(), heartbeat_at = NOW(), \
                         attempts = attempts + 1 \
                         WHERE id = $2",
                        &[&worker_id, &id],
                    )
                    .await
                    .map_err(|e| {
                        IntentError::runtime_error(format!("Failed to lock job: {}", e))
                    })?;

                claimed = Some(PgJobRow {
                    id: id.to_string(),
                    job_type,
                    payload,
                    priority: priority as i64,
                    attempts: (attempts + 1) as i64, // we just incremented
                    max_attempts: max_attempts as i64,
                    queue_name,
                });
                break;
            }
        }

        client
            .execute("COMMIT", &[])
            .await
            .map_err(|e| IntentError::runtime_error(format!("COMMIT failed: {}", e)))?;

        Ok(claimed)
    })
}

/// A claimed job row from postgres
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PgJobRow {
    id: String,
    job_type: String,
    payload: serde_json::Value,
    priority: i64,
    attempts: i64,
    max_attempts: i64,
    queue_name: String,
}

/// Mark a postgres job as completed
fn pg_complete_job(job_id: &str, result: Option<serde_json::Value>) -> Result<()> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    let id: uuid::Uuid = job_id
        .parse()
        .map_err(|e| IntentError::runtime_error(format!("Invalid job ID: {}", e)))?;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        client
            .execute(
                "UPDATE ntnt_jobs SET status = 'completed', completed_at = NOW(), \
                 result = $2, locked_by = NULL \
                 WHERE id = $1",
                &[&id, &result],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to complete job: {}", e)))?;

        Ok(())
    })
}

/// Mark a postgres job as failed (with retry or dead)
fn pg_fail_job(
    job_id: &str,
    error: &str,
    attempts: i64,
    max_attempts: i64,
    backoff_base_ms: u64,
) -> Result<()> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    let id: uuid::Uuid = job_id
        .parse()
        .map_err(|e| IntentError::runtime_error(format!("Invalid job ID: {}", e)))?;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        if attempts < max_attempts {
            // Retry with exponential backoff
            let backoff_ms =
                backoff_base_ms * (1u64 << (attempts as u64).saturating_sub(1).min(10));
            let interval = format!("{} milliseconds", backoff_ms);
            client
                .execute(
                    "UPDATE ntnt_jobs SET status = 'pending', locked_by = NULL, \
                     locked_at = NULL, error = $2, \
                     scheduled_at = NOW() + $3::interval \
                     WHERE id = $1",
                    &[&id, &error, &interval],
                )
                .await
                .map_err(|e| IntentError::runtime_error(format!("Failed to retry job: {}", e)))?;
        } else {
            // Dead
            client
                .execute(
                    "UPDATE ntnt_jobs SET status = 'dead', error = $2, \
                     completed_at = NOW(), locked_by = NULL \
                     WHERE id = $1",
                    &[&id, &error],
                )
                .await
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to mark job dead: {}", e))
                })?;
        }

        Ok(())
    })
}

/// Send heartbeat for an active postgres job
fn pg_heartbeat(job_id: &str) -> Result<()> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    let id: uuid::Uuid = job_id
        .parse()
        .map_err(|e| IntentError::runtime_error(format!("Invalid job ID: {}", e)))?;

    db_rt.block_on(async {
        let client = pool.get().await.map_err(|e| {
            IntentError::runtime_error(format!("Failed to get connection for heartbeat: {}", e))
        })?;

        client
            .execute(
                "UPDATE ntnt_jobs SET heartbeat_at = NOW() WHERE id = $1 AND status = 'active'",
                &[&id],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Heartbeat failed: {}", e)))?;

        Ok(())
    })
}

/// Release stale jobs (no heartbeat within visibility_timeout)
fn pg_release_stale_jobs(visibility_timeout_secs: u64) -> Result<u64> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let interval = format!("{} seconds", visibility_timeout_secs);
        let count = client
            .execute(
                "UPDATE ntnt_jobs SET status = 'pending', locked_by = NULL, locked_at = NULL \
                 WHERE status = 'active' AND heartbeat_at < NOW() - $1::interval",
                &[&interval],
            )
            .await
            .map_err(|e| {
                IntentError::runtime_error(format!("Failed to release stale jobs: {}", e))
            })?;

        Ok(count)
    })
}

/// Get queue stats from postgres
pub fn pg_queue_status() -> Result<HashMap<String, i64>> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let rows = client
            .query(
                "SELECT status, count(*)::bigint FROM ntnt_jobs GROUP BY status",
                &[],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to query stats: {}", e)))?;

        let mut counts: HashMap<String, i64> = HashMap::new();
        counts.insert("pending".to_string(), 0);
        counts.insert("active".to_string(), 0);
        counts.insert("completed".to_string(), 0);
        counts.insert("retry".to_string(), 0);
        counts.insert("dead".to_string(), 0);
        counts.insert("cancelled".to_string(), 0);

        for row in &rows {
            let status: String = row.get(0);
            let count: i64 = row.get(1);
            counts.insert(status, count);
        }

        Ok(counts)
    })
}

/// Get recent jobs from postgres
pub fn pg_recent_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let rows = client
            .query(
                "SELECT id::text, job_type, queue, status, priority, attempts, max_attempts, \
                 error, payload, created_at, scheduled_at, started_at, completed_at \
                 FROM ntnt_jobs ORDER BY created_at DESC LIMIT $1",
                &[&limit],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to query recent: {}", e)))?;

        Ok(pg_rows_to_job_maps(&rows))
    })
}

/// Get dead jobs from postgres
pub fn pg_dead_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let rows = client
            .query(
                "SELECT id::text, job_type, queue, status, priority, attempts, max_attempts, \
                 error, payload, created_at, scheduled_at, started_at, completed_at \
                 FROM ntnt_jobs WHERE status = 'dead' ORDER BY completed_at DESC LIMIT $1",
                &[&limit],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to query dead jobs: {}", e)))?;

        Ok(pg_rows_to_job_maps(&rows))
    })
}

/// List jobs from postgres with optional status filter
pub fn pg_list_jobs(status_filter: Option<&str>) -> Result<Vec<HashMap<String, Value>>> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let rows = if let Some(status) = status_filter {
            client
                .query(
                    "SELECT id::text, job_type, queue, status, priority, attempts, max_attempts, \
                     error, payload, created_at, scheduled_at, started_at, completed_at \
                     FROM ntnt_jobs WHERE status = $1 ORDER BY created_at DESC LIMIT 100",
                    &[&status],
                )
                .await
                .map_err(|e| IntentError::runtime_error(format!("Failed to query jobs: {}", e)))?
        } else {
            client
                .query(
                    "SELECT id::text, job_type, queue, status, priority, attempts, max_attempts, \
                     error, payload, created_at, scheduled_at, started_at, completed_at \
                     FROM ntnt_jobs ORDER BY created_at DESC LIMIT 100",
                    &[],
                )
                .await
                .map_err(|e| IntentError::runtime_error(format!("Failed to query jobs: {}", e)))?
        };

        Ok(pg_rows_to_job_maps(&rows))
    })
}

/// Retry a dead job in postgres
pub fn pg_retry_dead_job(job_id: &str) -> Result<bool> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    let id: uuid::Uuid = job_id
        .parse()
        .map_err(|e| IntentError::runtime_error(format!("Invalid job ID: {}", e)))?;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let count = client
            .execute(
                "UPDATE ntnt_jobs SET status = 'pending', attempts = 0, error = NULL, \
                 locked_by = NULL, scheduled_at = NOW() \
                 WHERE id = $1 AND status = 'dead'",
                &[&id],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to retry job: {}", e)))?;

        Ok(count > 0)
    })
}

/// Cancel a job in postgres
pub fn pg_cancel_job(job_id: &str) -> Result<bool> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    let id: uuid::Uuid = job_id
        .parse()
        .map_err(|e| IntentError::runtime_error(format!("Invalid job ID: {}", e)))?;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let count = client
            .execute(
                "UPDATE ntnt_jobs SET status = 'cancelled' \
                 WHERE id = $1 AND status IN ('pending', 'active')",
                &[&id],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to cancel job: {}", e)))?;

        Ok(count > 0)
    })
}

/// Convert postgres rows to job map vectors
fn pg_rows_to_job_maps(rows: &[tokio_postgres::Row]) -> Vec<HashMap<String, Value>> {
    let mut result = Vec::new();
    for row in rows {
        let mut map = HashMap::new();
        let id: String = row.get(0);
        let job_type: String = row.get(1);
        let queue: String = row.get(2);
        let status: String = row.get(3);
        let priority: i32 = row.get(4);
        let attempts: i32 = row.get(5);
        let max_attempts: i32 = row.get(6);
        let error: Option<String> = row.get(7);
        let payload: serde_json::Value = row.get(8);
        let created_at: Option<chrono::DateTime<chrono::Utc>> = row.get(9);

        map.insert("id".to_string(), Value::String(id));
        map.insert("type".to_string(), Value::String(job_type));
        map.insert("queue".to_string(), Value::String(queue));
        map.insert("status".to_string(), Value::String(status));
        map.insert("priority".to_string(), Value::Int(priority as i64));
        map.insert("attempt".to_string(), Value::Int(attempts as i64));
        map.insert("max_attempts".to_string(), Value::Int(max_attempts as i64));
        if let Some(ref err) = error {
            map.insert("error".to_string(), Value::String(err.clone()));
        }
        // Convert payload back to ntnt Value
        map.insert(
            "payload".to_string(),
            crate::stdlib::json::json_to_intent_value(&payload),
        );
        if let Some(dt) = created_at {
            map.insert("created_at".to_string(), Value::String(dt.to_rfc3339()));
        }
        result.push(map);
    }
    result
}

// ============================================================
// Enqueueing (dispatches to active backend)
// ============================================================

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Enqueue a job for immediate processing
pub fn enqueue_job(job_type: &str, args: HashMap<String, SerializedValue>) -> Result<String> {
    if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
        return Err(IntentError::runtime_error(
            "Cannot enqueue jobs during shutdown".to_string(),
        ));
    }

    let def = get_job_definition(job_type)?
        .ok_or_else(|| IntentError::runtime_error(format!("Unknown job type: {}", job_type)))?;

    match get_backend()? {
        BackendKind::Memory => enqueue_job_memory(job_type, args, Instant::now(), 0),
        BackendKind::Postgres(_) => pg_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            None,
            None,
        ),
    }
}

/// Enqueue a job with a delay (in milliseconds)
pub fn enqueue_job_in(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    delay_ms: u64,
) -> Result<String> {
    if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
        return Err(IntentError::runtime_error(
            "Cannot enqueue jobs during shutdown".to_string(),
        ));
    }

    let def = get_job_definition(job_type)?
        .ok_or_else(|| IntentError::runtime_error(format!("Unknown job type: {}", job_type)))?;

    match get_backend()? {
        BackendKind::Memory => {
            let scheduled = Instant::now() + Duration::from_millis(delay_ms);
            enqueue_job_memory(job_type, args, scheduled, 0)
        }
        BackendKind::Postgres(_) => pg_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            Some(delay_ms),
            None,
        ),
    }
}

/// Enqueue a job at a specific timestamp (unix ms)
pub fn enqueue_job_at_timestamp(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    timestamp_ms: u64,
) -> Result<String> {
    if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
        return Err(IntentError::runtime_error(
            "Cannot enqueue jobs during shutdown".to_string(),
        ));
    }

    let def = get_job_definition(job_type)?
        .ok_or_else(|| IntentError::runtime_error(format!("Unknown job type: {}", job_type)))?;

    match get_backend()? {
        BackendKind::Memory => {
            let now = now_ms();
            let delay = if timestamp_ms > now {
                timestamp_ms - now
            } else {
                0
            };
            let scheduled = Instant::now() + Duration::from_millis(delay);
            enqueue_job_memory(job_type, args, scheduled, 0)
        }
        BackendKind::Postgres(_) => pg_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            None,
            Some(timestamp_ms),
        ),
    }
}

/// In-memory enqueue implementation
fn enqueue_job_memory(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    scheduled_at: Instant,
    priority: i64,
) -> Result<String> {
    let def = get_job_definition(job_type)?
        .ok_or_else(|| IntentError::runtime_error(format!("Unknown job type: {}", job_type)))?;

    let job_id = format!("job_{}", JOB_ID_COUNTER.fetch_add(1, Ordering::SeqCst));

    let job = QueuedJob {
        id: job_id.clone(),
        job_type: job_type.to_string(),
        queue_name: def.queue.clone(),
        args,
        priority,
        status: JobStatus::Pending,
        attempt_count: 0,
        max_attempts: def.max_retries + 1,
        scheduled_at,
        error: None,
        created_at_ms: now_ms(),
    };

    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    backend
        .queues
        .entry(def.queue.clone())
        .or_insert_with(VecDeque::new)
        .push_back(job.clone());

    backend.jobs_by_id.insert(job_id.clone(), job);

    Ok(job_id)
}

/// Re-enqueue a job for retry with exponential backoff (in-memory)
fn requeue_for_retry(job: &mut QueuedJob, backoff_base_ms: u64) -> Result<()> {
    job.status = JobStatus::Retry;
    job.attempt_count += 1;
    let backoff = backoff_base_ms * (1u64 << (job.attempt_count as u64 - 1).min(10));
    job.scheduled_at = Instant::now() + Duration::from_millis(backoff);

    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    backend.jobs_by_id.insert(job.id.clone(), job.clone());

    let mut retry_job = job.clone();
    retry_job.status = JobStatus::Pending;
    backend
        .queues
        .entry(job.queue_name.clone())
        .or_insert_with(VecDeque::new)
        .push_back(retry_job.clone());

    backend.jobs_by_id.insert(job.id.clone(), retry_job);

    Ok(())
}

// ============================================================
// Worker Loop
// ============================================================

/// Claim the next ready job from any queue (in-memory)
fn claim_next_job() -> Result<Option<QueuedJob>> {
    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    let now = Instant::now();

    let mut best_queue: Option<String> = None;
    let mut best_idx: Option<usize> = None;
    let mut best_priority = i64::MIN;

    for (queue_name, queue) in backend.queues.iter() {
        for (idx, job) in queue.iter().enumerate() {
            if job.status == JobStatus::Pending && job.scheduled_at <= now {
                if job.priority > best_priority
                    || (job.priority == best_priority && best_idx.is_none())
                {
                    best_queue = Some(queue_name.clone());
                    best_idx = Some(idx);
                    best_priority = job.priority;
                }
            }
        }
    }

    if let (Some(queue_name), Some(idx)) = (best_queue, best_idx) {
        if let Some(queue) = backend.queues.get_mut(&queue_name) {
            if let Some(mut job) = queue.remove(idx) {
                job.status = JobStatus::Active;
                backend.jobs_by_id.insert(job.id.clone(), job.clone());
                return Ok(Some(job));
            }
        }
    }

    Ok(None)
}

/// Execute a single job's perform body
fn execute_job(job: &QueuedJob) -> std::result::Result<Value, String> {
    let def = {
        let registry = JOB_REGISTRY
            .lock()
            .map_err(|e| format!("Failed to lock job registry: {}", e))?;
        registry
            .get(&job.job_type)
            .cloned()
            .ok_or_else(|| format!("Job type '{}' not found in registry", job.job_type))?
    };

    let mut interpreter = crate::interpreter::Interpreter::new();
    interpreter.define_all_stdlib_as_globals();

    for param in &def.perform_params {
        if let Some(sv) = job.args.get(&param.name) {
            interpreter.define_variable(param.name.clone(), sv.to_value());
        } else if param.default.is_some() {
            interpreter.define_variable(param.name.clone(), Value::Unit);
        } else {
            return Err(format!(
                "Missing required argument '{}' for job '{}'",
                param.name, job.job_type
            ));
        }
    }

    match interpreter.eval_block(&def.perform_body) {
        Ok(value) => match value {
            Value::Return(inner) => Ok(*inner),
            other => Ok(other),
        },
        Err(e) => Err(format!("{}", e)),
    }
}

/// Execute a postgres job (args from JSONB payload)
fn execute_pg_job(job: &PgJobRow) -> std::result::Result<Value, String> {
    let def = {
        let registry = JOB_REGISTRY
            .lock()
            .map_err(|e| format!("Failed to lock job registry: {}", e))?;
        registry
            .get(&job.job_type)
            .cloned()
            .ok_or_else(|| format!("Job type '{}' not found in registry", job.job_type))?
    };

    let mut interpreter = crate::interpreter::Interpreter::new();
    interpreter.define_all_stdlib_as_globals();

    // Convert JSONB payload back to param values
    let args = SerializedValue::from_json(&job.payload);
    let args_map = match args {
        SerializedValue::Map(m) => m,
        _ => HashMap::new(),
    };

    for param in &def.perform_params {
        if let Some(sv) = args_map.get(&param.name) {
            interpreter.define_variable(param.name.clone(), sv.to_value());
        } else if param.default.is_some() {
            interpreter.define_variable(param.name.clone(), Value::Unit);
        } else {
            return Err(format!(
                "Missing required argument '{}' for job '{}'",
                param.name, job.job_type
            ));
        }
    }

    match interpreter.eval_block(&def.perform_body) {
        Ok(value) => match value {
            Value::Return(inner) => Ok(*inner),
            other => Ok(other),
        },
        Err(e) => Err(format!("{}", e)),
    }
}

/// Execute the on_failure handler for a job
fn execute_on_failure(job_type: &str, error: &str, attempt: i64) {
    let def = {
        if let Ok(registry) = JOB_REGISTRY.lock() {
            registry.get(job_type).cloned()
        } else {
            None
        }
    };

    if let Some(def) = def {
        if let Some((params, body)) = def.on_failure {
            let mut interpreter = crate::interpreter::Interpreter::new();
            interpreter.define_all_stdlib_as_globals();

            if let Some(p) = params.first() {
                interpreter.define_variable(p.name.clone(), Value::String(error.to_string()));
            }
            if let Some(p) = params.get(1) {
                interpreter.define_variable(p.name.clone(), Value::Int(attempt));
            }

            if let Err(e) = interpreter.eval_block(&body) {
                eprintln!("[jobs] on_failure handler error: {}", e);
            }
        }
    }
}

/// Start the background worker loop (dispatches to correct backend)
pub fn start_worker() -> Result<()> {
    match get_backend()? {
        BackendKind::Memory => start_worker_memory(),
        BackendKind::Postgres(config) => start_worker_postgres(config, None),
    }
}

/// Start the in-memory background worker loop
fn start_worker_memory() -> Result<()> {
    if WORKER_RUNNING.load(Ordering::SeqCst) {
        return Ok(());
    }

    WORKER_RUNNING.store(true, Ordering::SeqCst);
    WORKER_STOP.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        while !WORKER_STOP.load(Ordering::SeqCst) {
            match claim_next_job() {
                Ok(Some(mut job)) => {
                    let job_id = job.id.clone();
                    let job_type = job.job_type.clone();

                    let backoff_base = {
                        if let Ok(registry) = JOB_REGISTRY.lock() {
                            registry
                                .get(&job_type)
                                .map(|d| d.backoff_base_ms)
                                .unwrap_or(1000)
                        } else {
                            1000
                        }
                    };

                    let result = execute_job(&job);

                    match result {
                        Ok(_) => {
                            job.status = JobStatus::Completed;
                            if let Ok(mut backend) = QUEUE_BACKEND.lock() {
                                backend.jobs_by_id.insert(job_id.clone(), job.clone());
                                backend.completed_jobs.push((Instant::now(), job));
                            }
                        }
                        Err(error) => {
                            job.error = Some(error.clone());
                            job.attempt_count += 1;

                            if job.attempt_count < job.max_attempts {
                                execute_on_failure(&job.job_type, &error, job.attempt_count);
                                if let Err(e) = requeue_for_retry(&mut job, backoff_base) {
                                    eprintln!("[jobs] Failed to requeue for retry: {}", e);
                                }
                            } else {
                                execute_on_failure(&job.job_type, &error, job.attempt_count);
                                job.status = JobStatus::Dead;
                                if let Ok(mut backend) = QUEUE_BACKEND.lock() {
                                    backend.jobs_by_id.insert(job_id.clone(), job.clone());
                                    backend.dead_jobs.push(job);
                                }
                            }
                        }
                    }
                }
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    eprintln!("[jobs] Worker error claiming job: {}", e);
                    std::thread::sleep(Duration::from_millis(100));
                }
            }

            // Prune completed jobs
            if let Ok(mut backend) = QUEUE_BACKEND.lock() {
                let prune_after = Duration::from_millis(backend.config.prune_completed_after_ms);
                let now = Instant::now();
                backend
                    .completed_jobs
                    .retain(|(completed_at, _)| now.duration_since(*completed_at) < prune_after);
            }
        }

        WORKER_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

/// Start the postgres-backed worker loop
fn start_worker_postgres(
    config: PostgresBackendConfig,
    queue_names: Option<Vec<String>>,
) -> Result<()> {
    if WORKER_RUNNING.load(Ordering::SeqCst) {
        return Ok(());
    }

    WORKER_RUNNING.store(true, Ordering::SeqCst);
    WORKER_STOP.store(false, Ordering::SeqCst);

    let worker_id = uuid::Uuid::new_v4().to_string();
    let heartbeat_interval = config.heartbeat_interval_secs;
    let visibility_timeout = config.visibility_timeout_secs;

    // Determine which queues to poll
    let queues: Vec<String> = queue_names.unwrap_or_else(|| vec!["default".to_string()]);

    std::thread::spawn(move || {
        // Release stale jobs on startup
        if let Err(e) = pg_release_stale_jobs(visibility_timeout) {
            eprintln!("[jobs/pg] Failed to release stale jobs on startup: {}", e);
        }

        // Stale job release interval
        let stale_check_interval = Duration::from_secs(visibility_timeout / 2);
        let mut last_stale_check = Instant::now();

        while !WORKER_STOP.load(Ordering::SeqCst) {
            // Periodically release stale jobs
            if last_stale_check.elapsed() >= stale_check_interval {
                if let Err(e) = pg_release_stale_jobs(visibility_timeout) {
                    eprintln!("[jobs/pg] Failed to release stale jobs: {}", e);
                }
                last_stale_check = Instant::now();
            }

            match pg_claim_next_job(&queues, &worker_id) {
                Ok(Some(job)) => {
                    let job_id = job.id.clone();
                    let job_type = job.job_type.clone();

                    let backoff_base = {
                        if let Ok(registry) = JOB_REGISTRY.lock() {
                            registry
                                .get(&job_type)
                                .map(|d| d.backoff_base_ms)
                                .unwrap_or(1000)
                        } else {
                            1000
                        }
                    };

                    // Start heartbeat thread for this job
                    let hb_job_id = job_id.clone();
                    let hb_stop = Arc::new(AtomicBool::new(false));
                    let hb_stop_clone = hb_stop.clone();
                    let hb_interval = heartbeat_interval;

                    let hb_handle = std::thread::spawn(move || {
                        while !hb_stop_clone.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_secs(hb_interval));
                            if hb_stop_clone.load(Ordering::SeqCst) {
                                break;
                            }
                            if let Err(e) = pg_heartbeat(&hb_job_id) {
                                eprintln!("[jobs/pg] Heartbeat failed for {}: {}", hb_job_id, e);
                                break;
                            }
                        }
                    });

                    // Execute the job
                    let result = execute_pg_job(&job);

                    // Stop heartbeat
                    hb_stop.store(true, Ordering::SeqCst);
                    let _ = hb_handle.join();

                    match result {
                        Ok(result_value) => {
                            let result_json =
                                Some(crate::stdlib::json::intent_value_to_json(&result_value));
                            if let Err(e) = pg_complete_job(&job_id, result_json) {
                                eprintln!("[jobs/pg] Failed to complete job {}: {}", job_id, e);
                            }
                        }
                        Err(error) => {
                            execute_on_failure(&job.job_type, &error, job.attempts);
                            if let Err(e) = pg_fail_job(
                                &job_id,
                                &error,
                                job.attempts,
                                job.max_attempts,
                                backoff_base,
                            ) {
                                eprintln!("[jobs/pg] Failed to fail job {}: {}", job_id, e);
                            }
                        }
                    }
                }
                Ok(None) => {
                    // No jobs ready, poll interval
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    eprintln!("[jobs/pg] Worker error claiming job: {}", e);
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }

        WORKER_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

/// Start the worker with timeout enforcement via spawn tasks
pub fn start_worker_with_timeouts() -> Result<()> {
    start_worker()
}

/// Start a blocking worker (for dedicated worker processes)
pub fn start_worker_blocking(opts: &HashMap<String, Value>) -> Result<()> {
    let queues: Vec<String> = if let Some(Value::Array(arr)) = opts.get("queues") {
        arr.iter()
            .filter_map(|v| {
                if let Value::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect()
    } else {
        vec!["default".to_string()]
    };

    let concurrency: usize = if let Some(Value::Int(n)) = opts.get("concurrency") {
        *n as usize
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
    };

    match get_backend()? {
        BackendKind::Memory => {
            // For memory backend, just start the async worker and block
            start_worker_memory()?;
            // Block main thread
            loop {
                if WORKER_STOP.load(Ordering::SeqCst) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            Ok(())
        }
        BackendKind::Postgres(config) => {
            // Start N worker threads
            WORKER_RUNNING.store(true, Ordering::SeqCst);
            WORKER_STOP.store(false, Ordering::SeqCst);

            let worker_id_base = uuid::Uuid::new_v4().to_string();
            let heartbeat_interval = config.heartbeat_interval_secs;
            let visibility_timeout = config.visibility_timeout_secs;

            // Release stale jobs once on startup
            if let Err(e) = pg_release_stale_jobs(visibility_timeout) {
                eprintln!("[jobs/pg] Failed to release stale jobs on startup: {}", e);
            }

            let mut handles = Vec::new();

            for i in 0..concurrency {
                let queues = queues.clone();
                let worker_id = format!("{}-{}", worker_id_base, i);
                let hb_interval = heartbeat_interval;
                let vis_timeout = visibility_timeout;

                let handle = std::thread::spawn(move || {
                    let stale_check_interval = Duration::from_secs(vis_timeout / 2);
                    let mut last_stale_check = Instant::now();

                    while !WORKER_STOP.load(Ordering::SeqCst) {
                        if last_stale_check.elapsed() >= stale_check_interval {
                            let _ = pg_release_stale_jobs(vis_timeout);
                            last_stale_check = Instant::now();
                        }

                        match pg_claim_next_job(&queues, &worker_id) {
                            Ok(Some(job)) => {
                                let job_id = job.id.clone();
                                let job_type = job.job_type.clone();

                                let backoff_base = {
                                    if let Ok(registry) = JOB_REGISTRY.lock() {
                                        registry
                                            .get(&job_type)
                                            .map(|d| d.backoff_base_ms)
                                            .unwrap_or(1000)
                                    } else {
                                        1000
                                    }
                                };

                                // Heartbeat thread
                                let hb_job_id = job_id.clone();
                                let hb_stop = Arc::new(AtomicBool::new(false));
                                let hb_stop_clone = hb_stop.clone();

                                let hb_handle = std::thread::spawn(move || {
                                    while !hb_stop_clone.load(Ordering::SeqCst) {
                                        std::thread::sleep(Duration::from_secs(hb_interval));
                                        if hb_stop_clone.load(Ordering::SeqCst) {
                                            break;
                                        }
                                        if let Err(e) = pg_heartbeat(&hb_job_id) {
                                            eprintln!(
                                                "[jobs/pg] Heartbeat failed for {}: {}",
                                                hb_job_id, e
                                            );
                                            break;
                                        }
                                    }
                                });

                                let result = execute_pg_job(&job);

                                hb_stop.store(true, Ordering::SeqCst);
                                let _ = hb_handle.join();

                                match result {
                                    Ok(result_value) => {
                                        let result_json =
                                            Some(crate::stdlib::json::intent_value_to_json(
                                                &result_value,
                                            ));
                                        if let Err(e) = pg_complete_job(&job_id, result_json) {
                                            eprintln!(
                                                "[jobs/pg] Failed to complete job {}: {}",
                                                job_id, e
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        execute_on_failure(&job.job_type, &error, job.attempts);
                                        if let Err(e) = pg_fail_job(
                                            &job_id,
                                            &error,
                                            job.attempts,
                                            job.max_attempts,
                                            backoff_base,
                                        ) {
                                            eprintln!(
                                                "[jobs/pg] Failed to fail job {}: {}",
                                                job_id, e
                                            );
                                        }
                                    }
                                }
                            }
                            Ok(None) => {
                                std::thread::sleep(Duration::from_millis(100));
                            }
                            Err(e) => {
                                eprintln!("[jobs/pg] Worker {} error: {}", worker_id, e);
                                std::thread::sleep(Duration::from_millis(500));
                            }
                        }
                    }
                });

                handles.push(handle);
            }

            // Block until shutdown requested
            for handle in handles {
                let _ = handle.join();
            }

            WORKER_RUNNING.store(false, Ordering::SeqCst);
            Ok(())
        }
    }
}

// ============================================================
// Queue Operations (dispatch to active backend)
// ============================================================

/// Cancel a job by ID
pub fn cancel_job(job_id: &str) -> Result<bool> {
    match get_backend()? {
        BackendKind::Postgres(_) => pg_cancel_job(job_id),
        BackendKind::Memory => cancel_job_memory(job_id),
    }
}

fn cancel_job_memory(job_id: &str) -> Result<bool> {
    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    if let Some(job) = backend.jobs_by_id.get_mut(job_id) {
        match job.status {
            JobStatus::Pending | JobStatus::Retry => {
                job.status = JobStatus::Cancelled;
                for queue in backend.queues.values_mut() {
                    queue.retain(|j| j.id != job_id);
                }
                Ok(true)
            }
            JobStatus::Active => {
                job.status = JobStatus::Cancelled;
                Ok(true)
            }
            _ => Ok(false),
        }
    } else {
        Ok(false)
    }
}

/// Get queue status — counts by state
pub fn queue_status() -> Result<HashMap<String, i64>> {
    match get_backend()? {
        BackendKind::Postgres(_) => pg_queue_status(),
        BackendKind::Memory => queue_status_memory(),
    }
}

fn queue_status_memory() -> Result<HashMap<String, i64>> {
    let backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    let mut counts: HashMap<String, i64> = HashMap::new();
    counts.insert("pending".to_string(), 0);
    counts.insert("active".to_string(), 0);
    counts.insert("completed".to_string(), 0);
    counts.insert("retry".to_string(), 0);
    counts.insert("dead".to_string(), 0);
    counts.insert("cancelled".to_string(), 0);

    for job in backend.jobs_by_id.values() {
        let key = job.status.as_str().to_string();
        *counts.entry(key).or_insert(0) += 1;
    }

    Ok(counts)
}

/// Retry a dead job (move back to pending)
pub fn retry_dead_job(job_id: &str) -> Result<bool> {
    match get_backend()? {
        BackendKind::Postgres(_) => pg_retry_dead_job(job_id),
        BackendKind::Memory => retry_dead_job_memory(job_id),
    }
}

fn retry_dead_job_memory(job_id: &str) -> Result<bool> {
    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    let job_info = backend.jobs_by_id.get(job_id).and_then(|job| {
        if job.status == JobStatus::Dead {
            Some(job.queue_name.clone())
        } else {
            None
        }
    });

    if let Some(queue_name) = job_info {
        if let Some(job) = backend.jobs_by_id.get_mut(job_id) {
            job.status = JobStatus::Pending;
            job.attempt_count = 0;
            job.error = None;
            job.scheduled_at = Instant::now();
        }

        let job_id_owned = job_id.to_string();
        backend.dead_jobs.retain(|j| j.id != job_id_owned);

        if let Some(job) = backend.jobs_by_id.get(job_id) {
            let job_clone = job.clone();
            backend
                .queues
                .entry(queue_name)
                .or_insert_with(VecDeque::new)
                .push_back(job_clone);
        }

        Ok(true)
    } else {
        Ok(false)
    }
}

/// List jobs with optional status filter
pub fn list_jobs(status_filter: Option<&str>) -> Result<Vec<HashMap<String, Value>>> {
    match get_backend()? {
        BackendKind::Postgres(_) => pg_list_jobs(status_filter),
        BackendKind::Memory => list_jobs_memory(status_filter),
    }
}

fn list_jobs_memory(status_filter: Option<&str>) -> Result<Vec<HashMap<String, Value>>> {
    let backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    let mut result = Vec::new();
    for job in backend.jobs_by_id.values() {
        if let Some(filter) = status_filter {
            if job.status.as_str() != filter {
                continue;
            }
        }

        let mut map = HashMap::new();
        map.insert("id".to_string(), Value::String(job.id.clone()));
        map.insert("type".to_string(), Value::String(job.job_type.clone()));
        map.insert("queue".to_string(), Value::String(job.queue_name.clone()));
        map.insert(
            "status".to_string(),
            Value::String(job.status.as_str().to_string()),
        );
        map.insert("attempt".to_string(), Value::Int(job.attempt_count));
        map.insert("max_attempts".to_string(), Value::Int(job.max_attempts));
        map.insert(
            "created_at".to_string(),
            Value::Int(job.created_at_ms as i64),
        );
        if let Some(ref error) = job.error {
            map.insert("error".to_string(), Value::String(error.clone()));
        }
        result.push(map);
    }

    Ok(result)
}

/// Get recent jobs
pub fn recent_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    match get_backend()? {
        BackendKind::Postgres(_) => pg_recent_jobs(limit),
        BackendKind::Memory => list_jobs_memory(None), // memory doesn't sort by recent
    }
}

/// Get dead jobs
pub fn dead_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    match get_backend()? {
        BackendKind::Postgres(_) => pg_dead_jobs(limit),
        BackendKind::Memory => list_jobs_memory(Some("dead")),
    }
}

/// Configure the queue
pub fn configure_queue(config_map: &HashMap<String, Value>) -> Result<()> {
    // Check if backend is being configured
    if let Some(Value::String(backend_str)) = config_map.get("backend") {
        match backend_str.as_str() {
            "memory" => {
                set_backend(BackendKind::Memory)?;
            }
            "postgres" => {
                let url = match config_map.get("url") {
                    Some(Value::String(u)) => u.clone(),
                    _ => std::env::var("DATABASE_URL").map_err(|_| {
                        IntentError::runtime_error(
                            "Queue.configure() with postgres backend requires 'url' or DATABASE_URL env var".to_string(),
                        )
                    })?,
                };

                let mut pg_config = PostgresBackendConfig::default();
                pg_config.connection_url = url.clone();

                if let Some(Value::Int(secs)) = config_map.get("heartbeat_interval") {
                    pg_config.heartbeat_interval_secs = *secs as u64;
                }
                if let Some(Value::Int(secs)) = config_map.get("visibility_timeout") {
                    pg_config.visibility_timeout_secs = *secs as u64;
                }

                // Initialize pool and run migrations
                pg_init_job_pool(&url)?;

                set_backend(BackendKind::Postgres(pg_config))?;
            }
            other => {
                return Err(IntentError::runtime_error(format!(
                    "Unknown backend '{}'. Use 'memory' or 'postgres'",
                    other
                )));
            }
        }
    }

    // Also apply memory backend config
    if matches!(get_backend()?, BackendKind::Memory) {
        let mut backend = QUEUE_BACKEND.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock queue backend: {}", e))
        })?;

        if let Some(Value::Int(ms)) = config_map.get("shutdown_timeout") {
            backend.config.shutdown_timeout_ms = *ms as u64;
        }
        if let Some(Value::Int(ms)) = config_map.get("prune_completed_after") {
            backend.config.prune_completed_after_ms = *ms as u64;
        }
    }

    Ok(())
}

// ============================================================
// Graceful Shutdown
// ============================================================

/// Graceful shutdown of the job system
pub fn graceful_shutdown() {
    SHUTDOWN_FLAG.store(true, Ordering::SeqCst);
    WORKER_STOP.store(true, Ordering::SeqCst);

    let timeout = {
        QUEUE_BACKEND
            .lock()
            .ok()
            .map(|b| b.config.shutdown_timeout_ms)
            .unwrap_or(30_000)
    };

    let start = Instant::now();
    while WORKER_RUNNING.load(Ordering::SeqCst) && start.elapsed() < Duration::from_millis(timeout)
    {
        std::thread::sleep(Duration::from_millis(50));
    }

    // Release any active jobs back to pending (memory backend only)
    if matches!(get_backend(), Ok(BackendKind::Memory)) {
        if let Ok(mut backend) = QUEUE_BACKEND.lock() {
            let active_ids: Vec<String> = backend
                .jobs_by_id
                .iter()
                .filter(|(_, j)| j.status == JobStatus::Active)
                .map(|(id, _)| id.clone())
                .collect();

            for id in active_ids {
                if let Some(job) = backend.jobs_by_id.get_mut(&id) {
                    job.status = JobStatus::Pending;
                    let queue_name = job.queue_name.clone();
                    let job_clone = job.clone();
                    backend
                        .queues
                        .entry(queue_name)
                        .or_insert_with(VecDeque::new)
                        .push_back(job_clone);
                }
            }
        }
    }
}

/// Reset the job system (for testing)
pub fn reset_job_system() {
    SHUTDOWN_FLAG.store(false, Ordering::SeqCst);
    WORKER_STOP.store(true, Ordering::SeqCst);

    // Wait for worker to stop
    let start = Instant::now();
    while WORKER_RUNNING.load(Ordering::SeqCst) && start.elapsed() < Duration::from_secs(2) {
        std::thread::sleep(Duration::from_millis(50));
    }

    if let Ok(mut backend) = QUEUE_BACKEND.lock() {
        backend.queues.clear();
        backend.jobs_by_id.clear();
        backend.dead_jobs.clear();
        backend.completed_jobs.clear();
        backend.config = QueueConfig::default();
    }

    if let Ok(mut registry) = JOB_REGISTRY.lock() {
        registry.clear();
    }

    // Reset backend to memory
    if let Ok(mut backend) = ACTIVE_BACKEND.lock() {
        *backend = BackendKind::Memory;
    }

    WORKER_RUNNING.store(false, Ordering::SeqCst);
    WORKER_STOP.store(false, Ordering::SeqCst);
}

// ============================================================
// Queue module object (callable from ntnt as Queue.method())
// ============================================================

/// Create the Queue module value (a Map with method entries)
pub fn create_queue_module() -> Value {
    let mut module = HashMap::new();

    module.insert("type".to_string(), Value::String("Queue".to_string()));

    Value::Map(module)
}

// ============================================================
// Module Initialization
// ============================================================

/// Initialize the std/jobs module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // Queue module object — methods handled by interpreter special-casing
    // Methods: Queue.work_async(), Queue.work(opts), Queue.status(), Queue.cancel(id),
    //          Queue.configure(opts), Queue.recent(n), Queue.dead(n), Queue.retry(id),
    //          Queue.stats()
    module.insert("Queue".to_string(), create_queue_module());

    module
}
