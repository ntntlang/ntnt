//! PostgreSQL job queue backend.
//!
//! Provides persistent, distributed job processing using PostgreSQL with:
//! - SELECT FOR UPDATE SKIP LOCKED for job claiming
//! - LISTEN/NOTIFY for instant dispatch
//! - Heartbeat-based visibility timeout
//! - Transactional enqueue support

use super::{
    compute_args_hash, BackendKind, DEAD_RETENTION_SECS, JOB_REGISTRY, MAX_DEAD_JOBS, WORKER_STOP,
};
use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::concurrent::SerializedValue;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

type Result<T> = std::result::Result<T, IntentError>;

// ============================================================
// LISTEN/NOTIFY signaling
// ============================================================

/// Condvar for waking the PG worker when a LISTEN/NOTIFY fires
pub(crate) static PG_NOTIFY_SIGNAL: std::sync::LazyLock<(Mutex<bool>, std::sync::Condvar)> =
    std::sync::LazyLock::new(|| (Mutex::new(false), std::sync::Condvar::new()));

// ============================================================
// PostgreSQL Pool
// ============================================================

/// Global postgres pool for the job backend (separate from std/db/postgres pools)
static JOB_PG_POOL: std::sync::LazyLock<Mutex<Option<deadpool_postgres::Pool>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

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
    args_hash VARCHAR(64),
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
CREATE INDEX IF NOT EXISTS idx_ntnt_jobs_args_hash
    ON ntnt_jobs(args_hash, job_type)
    WHERE args_hash IS NOT NULL;

-- Queue pause/resume state table
CREATE TABLE IF NOT EXISTS ntnt_queue_state (
    queue VARCHAR(255) PRIMARY KEY,
    paused BOOLEAN DEFAULT false
);

-- LISTEN/NOTIFY trigger for instant dispatch
CREATE OR REPLACE FUNCTION ntnt_jobs_notify() RETURNS trigger AS $$
BEGIN
    PERFORM pg_notify('ntnt_jobs_notify', NEW.queue);
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger WHERE tgname = 'ntnt_jobs_insert_trigger'
    ) THEN
        CREATE TRIGGER ntnt_jobs_insert_trigger
            AFTER INSERT ON ntnt_jobs
            FOR EACH ROW
            WHEN (NEW.status = 'pending' AND NEW.scheduled_at <= NOW())
            EXECUTE FUNCTION ntnt_jobs_notify();
    END IF;
END
$$;
"#;

/// Initialize the postgres pool for jobs and run migrations
pub(crate) fn pg_init_job_pool(connection_url: &str) -> Result<()> {
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
pub fn get_job_pool() -> Result<deadpool_postgres::Pool> {
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

// ============================================================
// A claimed job row from postgres
// ============================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct PgJobRow {
    pub id: String,
    pub job_type: String,
    pub payload: serde_json::Value,
    pub priority: i64,
    pub attempts: i64,
    pub max_attempts: i64,
    pub queue_name: String,
}

// ============================================================
// Enqueue
// ============================================================

/// Enqueue a job to postgres
pub(crate) fn pg_enqueue_job(
    job_type: &str,
    queue: &str,
    args: &HashMap<String, SerializedValue>,
    max_attempts: i64,
    priority: i64,
    scheduled_at_offset_ms: Option<u64>,
    scheduled_at_timestamp_ms: Option<u64>,
    unique_for_secs: Option<u64>,
) -> Result<String> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    // Serialize args to JSON
    let args_map = SerializedValue::Map(args.clone());
    let payload_json = args_map.to_json();

    // Compute args hash for unique job support
    let args_hash: Option<String> = if unique_for_secs.is_some() {
        Some(compute_args_hash(job_type, args))
    } else {
        None
    };

    db_rt.block_on(async {
        let client = pool.get().await.map_err(|e| {
            IntentError::runtime_error(format!("Failed to get connection: {}", e))
        })?;

        // Check unique constraint if configured
        if let (Some(ref hash), Some(unique_secs)) = (&args_hash, unique_for_secs) {
            let interval = format!("{} seconds", unique_secs);
            let existing = client
                .query_opt(
                    "SELECT id::text FROM ntnt_jobs \
                     WHERE args_hash = $1 AND job_type = $2 \
                     AND status IN ('pending', 'active', 'retry') \
                     AND created_at > NOW() - $3::interval \
                     LIMIT 1",
                    &[&hash.as_str(), &job_type, &interval],
                )
                .await
                .map_err(|e| IntentError::runtime_error(format!("Unique check failed: {}", e)))?;
            if let Some(row) = existing {
                return Ok(row.get::<_, String>(0));
            }
        }

        let args_hash_ref = args_hash.as_deref();

        let job_id_result = if let Some(offset_ms) = scheduled_at_offset_ms {
            let interval = format!("{} milliseconds", offset_ms);
            let row = client
                .query_one(
                    "INSERT INTO ntnt_jobs (job_type, queue, payload, max_attempts, priority, scheduled_at, args_hash) \
                     VALUES ($1, $2, $3, $4, $5, NOW() + $6::interval, $7) RETURNING id::text",
                    &[
                        &job_type,
                        &queue,
                        &payload_json,
                        &(max_attempts as i32),
                        &(priority as i32),
                        &interval,
                        &args_hash_ref,
                    ],
                )
                .await
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to enqueue job: {}", e))
                })?;
            row.get::<_, String>(0)
        } else if let Some(ts_ms) = scheduled_at_timestamp_ms {
            let secs = (ts_ms / 1000) as i64;
            let nsecs = ((ts_ms % 1000) * 1_000_000) as u32;
            let dt = chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsecs)
                .unwrap_or_else(chrono::Utc::now);
            let row = client
                .query_one(
                    "INSERT INTO ntnt_jobs (job_type, queue, payload, max_attempts, priority, scheduled_at, args_hash) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id::text",
                    &[
                        &job_type,
                        &queue,
                        &payload_json,
                        &(max_attempts as i32),
                        &(priority as i32),
                        &dt,
                        &args_hash_ref,
                    ],
                )
                .await
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to enqueue job: {}", e))
                })?;
            row.get::<_, String>(0)
        } else {
            let row = client
                .query_one(
                    "INSERT INTO ntnt_jobs (job_type, queue, payload, max_attempts, priority, args_hash) \
                     VALUES ($1, $2, $3, $4, $5, $6) RETURNING id::text",
                    &[
                        &job_type,
                        &queue,
                        &payload_json,
                        &(max_attempts as i32),
                        &(priority as i32),
                        &args_hash_ref,
                    ],
                )
                .await
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to enqueue job: {}", e))
                })?;
            row.get::<_, String>(0)
        };

        Ok(job_id_result)
    })
}

/// Enqueue a job within an existing PostgreSQL transaction.
pub(crate) fn pg_enqueue_job_tx(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    tx_handle: &Value,
) -> Result<String> {
    // Only available with PostgreSQL backend
    match super::get_backend()? {
        BackendKind::Postgres(_) => {}
        _ => {
            return Err(IntentError::runtime_error(
                "enqueue_tx() is only available with the PostgreSQL backend".to_string(),
            ));
        }
    }

    let def = super::get_job_definition(job_type)?
        .ok_or_else(|| IntentError::runtime_error(format!("Unknown job type: {}", job_type)))?;

    // Get the connection ID from the transaction handle
    let conn_id = match tx_handle {
        Value::Map(map) => {
            if let Some(Value::Int(id)) = map.get("_pg_connection_id") {
                *id as u64
            } else {
                return Err(IntentError::type_error(
                    "enqueue_tx() requires a database connection handle with an active transaction"
                        .to_string(),
                ));
            }
        }
        _ => {
            return Err(IntentError::type_error(
                "enqueue_tx() requires a database connection handle".to_string(),
            ));
        }
    };

    // Look up the transaction client from TXN_REGISTRY
    let txn_client = {
        let registry = crate::stdlib::postgres::TXN_REGISTRY.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock txn registry: {}", e))
        })?;
        registry.get(&conn_id).cloned().ok_or_else(|| {
            IntentError::runtime_error(
                "No active transaction found. enqueue_tx() must be called within a transaction block.".to_string(),
            )
        })?
    };

    let args_map = SerializedValue::Map(args.clone());
    let payload_json = args_map.to_json();
    let max_attempts = (def.max_retries + 1) as i32;

    // Compute args hash for unique job support
    let args_hash = if def.unique_for_secs.is_some() {
        Some(compute_args_hash(job_type, &args))
    } else {
        None
    };

    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;
    db_rt.block_on(async {
        let client = txn_client.lock().await;

        // Check unique constraint if configured
        if let (Some(ref hash), Some(unique_secs)) = (&args_hash, def.unique_for_secs) {
            let interval = format!("{} seconds", unique_secs);
            let existing = client
                .query_opt(
                    "SELECT id::text FROM ntnt_jobs \
                     WHERE args_hash = $1 AND job_type = $2 \
                     AND status IN ('pending', 'active', 'retry') \
                     AND created_at > NOW() - $3::interval \
                     LIMIT 1",
                    &[&hash.as_str(), &job_type, &interval],
                )
                .await
                .map_err(|e| IntentError::runtime_error(format!("Unique check failed: {}", e)))?;
            if let Some(row) = existing {
                return Ok(row.get::<_, String>(0));
            }
        }

        let row = client
            .query_one(
                "INSERT INTO ntnt_jobs (job_type, queue, payload, max_attempts, priority, args_hash) \
                 VALUES ($1, $2, $3, $4, $5, $6) RETURNING id::text",
                &[
                    &job_type,
                    &def.queue.as_str(),
                    &payload_json,
                    &max_attempts,
                    &0i32,
                    &args_hash.as_deref(),
                ],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to enqueue job in transaction: {}", e)))?;

        Ok(row.get::<_, String>(0))
    })
}

// ============================================================
// Claim
// ============================================================

/// Claim the next job from postgres using SELECT FOR UPDATE SKIP LOCKED
pub(crate) fn pg_claim_next_job(queues: &[String], worker_id: &str) -> Result<Option<PgJobRow>> {
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

// ============================================================
// Complete / Fail / Heartbeat
// ============================================================

/// Mark a postgres job as completed
pub(crate) fn pg_complete_job(job_id: &str, result: Option<serde_json::Value>) -> Result<()> {
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
pub(crate) fn pg_fail_job(
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
    })?;

    // Prune dead jobs if we just marked one dead (async, best-effort)
    if attempts >= max_attempts {
        let _ = pg_prune_dead_jobs();
    }

    Ok(())
}

/// Send heartbeat for an active postgres job
pub(crate) fn pg_heartbeat(job_id: &str) -> Result<()> {
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
pub(crate) fn pg_release_stale_jobs(visibility_timeout_secs: u64) -> Result<u64> {
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

// ============================================================
// Status / List / Recent / Dead
// ============================================================

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
pub(crate) fn pg_recent_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
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
pub(crate) fn pg_dead_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
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

/// Get per-queue stats from PostgreSQL
pub(crate) fn pg_per_queue_status() -> Result<HashMap<String, HashMap<String, i64>>> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;

        let rows = client
            .query(
                "SELECT queue, status, count(*)::bigint FROM ntnt_jobs GROUP BY queue, status",
                &[],
            )
            .await
            .map_err(|e| {
                IntentError::runtime_error(format!("Failed to query per-queue stats: {}", e))
            })?;

        let mut per_queue: HashMap<String, HashMap<String, i64>> = HashMap::new();
        let mut total: HashMap<String, i64> = HashMap::new();
        for s in &[
            "pending",
            "active",
            "completed",
            "retry",
            "dead",
            "cancelled",
        ] {
            total.insert(s.to_string(), 0);
        }

        for row in &rows {
            let queue: String = row.get(0);
            let status: String = row.get(1);
            let count: i64 = row.get(2);

            *total.entry(status.clone()).or_insert(0) += count;

            let queue_counts = per_queue.entry(queue).or_insert_with(|| {
                let mut m = HashMap::new();
                for s in &[
                    "pending",
                    "active",
                    "completed",
                    "retry",
                    "dead",
                    "cancelled",
                ] {
                    m.insert(s.to_string(), 0);
                }
                m
            });
            *queue_counts.entry(status).or_insert(0) += count;
        }

        per_queue.insert("total".to_string(), total);
        Ok(per_queue)
    })
}

// ============================================================
// Queue Pause/Resume
// ============================================================

pub(crate) fn pg_pause_queue(queue_name: &str) -> Result<()> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;
        client
            .execute(
                "INSERT INTO ntnt_queue_state (queue, paused) VALUES ($1, true) \
                 ON CONFLICT (queue) DO UPDATE SET paused = true",
                &[&queue_name],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to pause queue: {}", e)))?;
        Ok(())
    })
}

pub(crate) fn pg_resume_queue(queue_name: &str) -> Result<()> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;
        client
            .execute(
                "INSERT INTO ntnt_queue_state (queue, paused) VALUES ($1, false) \
                 ON CONFLICT (queue) DO UPDATE SET paused = false",
                &[&queue_name],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to resume queue: {}", e)))?;
        Ok(())
    })
}

pub(crate) fn pg_paused_queues() -> Result<Vec<String>> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;

    db_rt.block_on(async {
        let client = pool
            .get()
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to get connection: {}", e)))?;
        let rows = client
            .query(
                "SELECT queue FROM ntnt_queue_state WHERE paused = true",
                &[],
            )
            .await
            .map_err(|e| {
                IntentError::runtime_error(format!("Failed to query paused queues: {}", e))
            })?;
        Ok(rows.iter().map(|r| r.get::<_, String>(0)).collect())
    })
}

// ============================================================
// Dead Job Pruning
// ============================================================

/// Prune dead jobs beyond the configured cap and retention period
pub(crate) fn pg_prune_dead_jobs() -> Result<()> {
    let pool = get_job_pool()?;
    let db_rt = &crate::stdlib::postgres::DB_RUNTIME;
    let max_dead = MAX_DEAD_JOBS.load(Ordering::Relaxed) as i64;
    let retention_secs = DEAD_RETENTION_SECS.load(Ordering::Relaxed);

    db_rt.block_on(async {
        let client = pool.get().await.map_err(|e| {
            IntentError::runtime_error(format!("Failed to get connection: {}", e))
        })?;

        // Prune by count — keep only the most recent N
        client
            .execute(
                "DELETE FROM ntnt_jobs WHERE status = 'dead' AND id NOT IN \
                 (SELECT id FROM ntnt_jobs WHERE status = 'dead' ORDER BY completed_at DESC LIMIT $1)",
                &[&max_dead],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to prune dead jobs by count: {}", e)))?;

        // Prune by age
        let interval = format!("{} seconds", retention_secs);
        client
            .execute(
                "DELETE FROM ntnt_jobs WHERE status = 'dead' AND completed_at < NOW() - $1::interval",
                &[&interval],
            )
            .await
            .map_err(|e| IntentError::runtime_error(format!("Failed to prune dead jobs by age: {}", e)))?;

        Ok(())
    })
}

// ============================================================
// LISTEN/NOTIFY Listener
// ============================================================

/// Start a background thread that listens for PostgreSQL NOTIFY events
pub(crate) fn pg_start_listen_notify(connection_url: &str) -> Result<()> {
    let url = connection_url.to_string();

    std::thread::spawn(move || {
        let db_rt = &crate::stdlib::postgres::DB_RUNTIME;
        let result = db_rt.block_on(async {
            let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
                .await
                .map_err(|e| {
                IntentError::runtime_error(format!("LISTEN/NOTIFY connection failed: {}", e))
            })?;

            // Spawn the connection driver
            let conn_handle = tokio::spawn(async move {
                if let Err(e) = connection.await {
                    eprintln!("[jobs/pg] LISTEN connection error: {}", e);
                }
            });

            // Subscribe to notifications
            client
                .execute("LISTEN ntnt_jobs_notify", &[])
                .await
                .map_err(|e| IntentError::runtime_error(format!("LISTEN failed: {}", e)))?;

            // Wait for notifications in a loop using periodic polling
            loop {
                if WORKER_STOP.load(Ordering::SeqCst) {
                    break;
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;

                // Wake up workers
                let (lock, cvar) = &*PG_NOTIFY_SIGNAL;
                if let Ok(mut notified) = lock.lock() {
                    *notified = true;
                    cvar.notify_all();
                }
            }

            conn_handle.abort();
            Ok::<(), IntentError>(())
        });

        if let Err(e) = result {
            eprintln!("[jobs/pg] LISTEN/NOTIFY thread failed: {}", e);
        }
    });

    Ok(())
}

// ============================================================
// Helpers
// ============================================================

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

/// Execute a postgres job (args from JSONB payload)
pub(crate) fn execute_pg_job(job: &PgJobRow) -> std::result::Result<Value, String> {
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
