//! Redis Streams job queue backend.
//!
//! Provides distributed job processing using Redis Streams with:
//! - Consumer groups for multi-worker support
//! - Scheduled job promotion via sorted sets
//! - Heartbeat-based visibility timeout with XCLAIM recovery
//! - Unique job deduplication via SET NX EX

use super::{
    compute_args_hash, now_ms, BackendKind, ACTIVE_BACKEND, DEAD_RETENTION_SECS, JOB_REGISTRY,
    MAX_DEAD_JOBS,
};
use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::concurrent::SerializedValue;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

type Result<T> = std::result::Result<T, IntentError>;

// ============================================================
// Redis Connection
// ============================================================

/// Global Redis connection for the job backend
pub static JOB_REDIS_CONN: std::sync::LazyLock<Mutex<Option<redis::Connection>>> =
    std::sync::LazyLock::new(|| Mutex::new(None));

/// Get the Redis connection (locked)
fn get_redis_conn() -> Result<std::sync::MutexGuard<'static, Option<redis::Connection>>> {
    JOB_REDIS_CONN
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock Redis connection: {}", e)))
}

/// Initialize the Redis connection for jobs and create consumer groups
pub(crate) fn redis_init(url: &str, consumer_group: &str) -> Result<()> {
    // Normalize valkey:// to redis:// (same as kv.rs)
    let normalized_url = if url.starts_with("valkey://") {
        url.replacen("valkey://", "redis://", 1)
    } else {
        url.to_string()
    };

    let client = redis::Client::open(normalized_url.as_str())
        .map_err(|e| IntentError::runtime_error(format!("Failed to create Redis client: {}", e)))?;

    let conn = client
        .get_connection()
        .map_err(|e| IntentError::runtime_error(format!("Failed to connect to Redis: {}", e)))?;

    // Store connection
    let mut conn_guard = get_redis_conn()?;
    *conn_guard = Some(conn);

    // Create consumer group for the default queue (idempotent)
    redis_ensure_consumer_group("default", consumer_group)?;

    Ok(())
}

/// Initialize Redis pool from URL (for CLI usage)
pub fn redis_init_from_url(url: &str) -> Result<()> {
    redis_init(url, "ntnt_workers")
}

/// Ensure a consumer group exists for a queue stream (idempotent)
pub(crate) fn redis_ensure_consumer_group(queue: &str, group: &str) -> Result<()> {
    let stream_key = format!("ntnt:queue:{}", queue);
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    // XGROUP CREATE ... MKSTREAM — idempotent (ignore BUSYGROUP error)
    let result: redis::RedisResult<()> = redis::cmd("XGROUP")
        .arg("CREATE")
        .arg(&stream_key)
        .arg(group)
        .arg("0")
        .arg("MKSTREAM")
        .query(conn);

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            let msg = format!("{}", e);
            if msg.contains("BUSYGROUP") {
                Ok(()) // Already exists — idempotent
            } else {
                Err(IntentError::runtime_error(format!(
                    "Failed to create consumer group: {}",
                    e
                )))
            }
        }
    }
}

// ============================================================
// A claimed job row from Redis
// ============================================================

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct RedisJobRow {
    pub id: String,
    pub stream_message_id: String,
    pub queue_name: String,
    pub job_type: String,
    pub payload: serde_json::Value,
    pub priority: i64,
    pub attempts: i64,
    pub max_attempts: i64,
}

// ============================================================
// Enqueue
// ============================================================

/// Enqueue a job via Redis Streams
pub(crate) fn redis_enqueue_job(
    job_type: &str,
    queue: &str,
    args: &HashMap<String, SerializedValue>,
    max_attempts: i64,
    priority: i64,
    scheduled_at_offset_ms: Option<u64>,
    scheduled_at_timestamp_ms: Option<u64>,
    unique_for_secs: Option<u64>,
) -> Result<String> {
    let job_id = uuid::Uuid::new_v4().to_string();
    let now = now_ms();
    let args_map = SerializedValue::Map(args.clone());
    let payload_json = serde_json::to_string(&args_map.to_json()).unwrap_or_default();

    // Check unique constraint if configured
    if let Some(unique_secs) = unique_for_secs {
        let hash = compute_args_hash(job_type, args);
        let unique_key = format!("ntnt:unique:{}", hash);

        let mut conn_guard = get_redis_conn()?;
        let conn = conn_guard.as_mut().ok_or_else(|| {
            IntentError::runtime_error("Redis job connection not initialized".to_string())
        })?;

        // SET NX EX — only set if key doesn't exist
        let result: redis::RedisResult<bool> = redis::cmd("SET")
            .arg(&unique_key)
            .arg(&job_id)
            .arg("NX")
            .arg("EX")
            .arg(unique_secs)
            .query(conn);

        match result {
            Ok(true) => {} // Key was set, proceed with enqueue
            Ok(false) => {
                // Key already exists — return existing job ID
                let existing_id: redis::RedisResult<String> =
                    redis::cmd("GET").arg(&unique_key).query(conn);
                return Ok(existing_id.unwrap_or(job_id));
            }
            Err(e) => {
                eprintln!(
                    "[jobs/redis] Unique check failed: {}, proceeding with enqueue",
                    e
                );
            }
        }
    }

    // Determine if this is a scheduled (delayed) job
    let scheduled_at_ms: Option<u64> = if let Some(offset_ms) = scheduled_at_offset_ms {
        Some(now + offset_ms)
    } else {
        scheduled_at_timestamp_ms
    };

    // Ensure consumer group exists for this queue
    let group = {
        let backend = ACTIVE_BACKEND
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Failed to lock backend: {}", e)))?;
        match &*backend {
            BackendKind::Redis(config) => config.consumer_group.clone(),
            _ => "ntnt_workers".to_string(),
        }
    };
    redis_ensure_consumer_group(queue, &group)?;

    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    // Store job details in a hash
    let hash_key = format!("ntnt:job:{}", job_id);
    let status = if scheduled_at_ms.is_some() {
        "scheduled"
    } else {
        "pending"
    };

    redis::pipe()
        .cmd("HSET")
        .arg(&hash_key)
        .arg("id")
        .arg(&job_id)
        .arg("type")
        .arg(job_type)
        .arg("queue")
        .arg(queue)
        .arg("payload")
        .arg(&payload_json)
        .arg("status")
        .arg(status)
        .arg("priority")
        .arg(priority)
        .arg("attempts")
        .arg(0i64)
        .arg("max_attempts")
        .arg(max_attempts)
        .arg("created_at")
        .arg(now)
        .arg("error")
        .arg("")
        .query::<()>(conn)
        .map_err(|e| IntentError::runtime_error(format!("Failed to HSET job: {}", e)))?;

    if let Some(sched_ms) = scheduled_at_ms {
        // Scheduled job — add to sorted set, not stream
        redis::cmd("ZADD")
            .arg("ntnt:scheduled")
            .arg(sched_ms as f64)
            .arg(&job_id)
            .query::<()>(conn)
            .map_err(|e| {
                IntentError::runtime_error(format!("Failed to ZADD scheduled job: {}", e))
            })?;

        // Also store the queue name so we know where to move it later
        redis::cmd("HSET")
            .arg(&hash_key)
            .arg("scheduled_at")
            .arg(sched_ms)
            .query::<()>(conn)
            .map_err(|e| {
                IntentError::runtime_error(format!("Failed to set scheduled_at: {}", e))
            })?;
    } else {
        // Immediate job — add to stream
        let stream_key = format!("ntnt:queue:{}", queue);
        redis::cmd("XADD")
            .arg(&stream_key)
            .arg("*")
            .arg("job_id")
            .arg(&job_id)
            .query::<String>(conn)
            .map_err(|e| IntentError::runtime_error(format!("Failed to XADD job: {}", e)))?;
    }

    Ok(job_id)
}

// ============================================================
// Claim
// ============================================================

/// Claim the next job from Redis using XREADGROUP
pub(crate) fn redis_claim_next_job(
    queues: &[String],
    worker_id: &str,
    group: &str,
) -> Result<Option<RedisJobRow>> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    for queue in queues {
        let stream_key = format!("ntnt:queue:{}", queue);

        // XREADGROUP GROUP {group} {worker_id} COUNT 1 BLOCK 100 STREAMS {stream} >
        let result: redis::RedisResult<redis::Value> = redis::cmd("XREADGROUP")
            .arg("GROUP")
            .arg(group)
            .arg(worker_id)
            .arg("COUNT")
            .arg(1)
            .arg("BLOCK")
            .arg(100) // 100ms block
            .arg("STREAMS")
            .arg(&stream_key)
            .arg(">")
            .query(conn);

        match result {
            Ok(redis::Value::Array(streams)) => {
                // Parse the XREADGROUP response
                // Format: [[stream_name, [[message_id, [field, value, ...]]]]]
                if let Some(redis::Value::Array(stream_data)) = streams.first() {
                    if let Some(redis::Value::Array(messages)) = stream_data.get(1) {
                        if let Some(redis::Value::Array(msg)) = messages.first() {
                            let message_id = match msg.first() {
                                Some(redis::Value::BulkString(bytes)) => {
                                    String::from_utf8_lossy(bytes).to_string()
                                }
                                Some(redis::Value::SimpleString(s)) => s.clone(),
                                _ => continue,
                            };

                            // Extract job_id from message fields
                            let job_id = if let Some(redis::Value::Array(fields)) = msg.get(1) {
                                // Fields are [key, value, key, value, ...]
                                let mut id = None;
                                let mut i = 0;
                                while i + 1 < fields.len() {
                                    let key = match &fields[i] {
                                        redis::Value::BulkString(b) => {
                                            String::from_utf8_lossy(b).to_string()
                                        }
                                        redis::Value::SimpleString(s) => s.clone(),
                                        _ => {
                                            i += 2;
                                            continue;
                                        }
                                    };
                                    let val = match &fields[i + 1] {
                                        redis::Value::BulkString(b) => {
                                            String::from_utf8_lossy(b).to_string()
                                        }
                                        redis::Value::SimpleString(s) => s.clone(),
                                        _ => {
                                            i += 2;
                                            continue;
                                        }
                                    };
                                    if key == "job_id" {
                                        id = Some(val);
                                    }
                                    i += 2;
                                }
                                id
                            } else {
                                None
                            };

                            if let Some(job_id) = job_id {
                                // Read job details from hash
                                let hash_key = format!("ntnt:job:{}", job_id);
                                let hash_data: redis::RedisResult<HashMap<String, String>> =
                                    redis::cmd("HGETALL").arg(&hash_key).query(conn);

                                if let Ok(data) = hash_data {
                                    if data.is_empty() {
                                        // Job hash doesn't exist — ACK and skip
                                        let _ = redis::cmd("XACK")
                                            .arg(&stream_key)
                                            .arg(group)
                                            .arg(&message_id)
                                            .query::<i64>(conn);
                                        continue;
                                    }

                                    let job_type = data.get("type").cloned().unwrap_or_default();
                                    let payload_str =
                                        data.get("payload").cloned().unwrap_or_default();
                                    let payload: serde_json::Value = serde_json::from_str(
                                        &payload_str,
                                    )
                                    .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));
                                    let priority: i64 = data
                                        .get("priority")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0);
                                    let attempts: i64 = data
                                        .get("attempts")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(0);
                                    let max_attempts: i64 = data
                                        .get("max_attempts")
                                        .and_then(|s| s.parse().ok())
                                        .unwrap_or(3);

                                    // Update job to active + increment attempts
                                    let now = now_ms();
                                    redis::pipe()
                                        .cmd("HSET")
                                        .arg(&hash_key)
                                        .arg("status")
                                        .arg("active")
                                        .arg("locked_by")
                                        .arg(worker_id)
                                        .arg("started_at")
                                        .arg(now)
                                        .arg("heartbeat_at")
                                        .arg(now)
                                        .arg("attempts")
                                        .arg(attempts + 1)
                                        .query::<()>(conn)
                                        .map_err(|e| {
                                            IntentError::runtime_error(format!(
                                                "Failed to update job to active: {}",
                                                e
                                            ))
                                        })?;

                                    return Ok(Some(RedisJobRow {
                                        id: job_id,
                                        stream_message_id: message_id,
                                        queue_name: queue.clone(),
                                        job_type,
                                        payload,
                                        priority,
                                        attempts: attempts + 1,
                                        max_attempts,
                                    }));
                                }
                            }
                        }
                    }
                }
            }
            Ok(redis::Value::Nil) | Ok(_) => {
                // No messages, try next queue
                continue;
            }
            Err(e) => {
                // Timeout or other error — try next queue
                let msg = format!("{}", e);
                if !msg.contains("timeout") {
                    eprintln!("[jobs/redis] XREADGROUP error on {}: {}", queue, e);
                }
                continue;
            }
        }
    }

    Ok(None)
}

// ============================================================
// Complete / Fail / Heartbeat
// ============================================================

/// Complete a Redis job — XACK + update hash + set expiry
pub(crate) fn redis_complete_job(
    job_id: &str,
    queue: &str,
    stream_message_id: &str,
    result: Option<serde_json::Value>,
    group: &str,
    prune_secs: u64,
) -> Result<()> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let hash_key = format!("ntnt:job:{}", job_id);
    let stream_key = format!("ntnt:queue:{}", queue);
    let now = now_ms();

    let result_json = result
        .map(|v| serde_json::to_string(&v).unwrap_or_default())
        .unwrap_or_default();

    // XACK + update hash + EXPIRE
    redis::pipe()
        .cmd("XACK")
        .arg(&stream_key)
        .arg(group)
        .arg(stream_message_id)
        .cmd("HSET")
        .arg(&hash_key)
        .arg("status")
        .arg("completed")
        .arg("completed_at")
        .arg(now)
        .arg("result")
        .arg(&result_json)
        .arg("locked_by")
        .arg("")
        .cmd("EXPIRE")
        .arg(&hash_key)
        .arg(prune_secs)
        .query::<()>(conn)
        .map_err(|e| IntentError::runtime_error(format!("Failed to complete Redis job: {}", e)))?;

    Ok(())
}

/// Fail a Redis job — update hash, optionally re-add to stream for retry or mark dead
pub(crate) fn redis_fail_job(
    job_id: &str,
    queue: &str,
    stream_message_id: &str,
    error: &str,
    attempts: i64,
    max_attempts: i64,
    backoff_base_ms: u64,
    group: &str,
) -> Result<()> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let hash_key = format!("ntnt:job:{}", job_id);
    let stream_key = format!("ntnt:queue:{}", queue);

    // ACK the current message first
    redis::cmd("XACK")
        .arg(&stream_key)
        .arg(group)
        .arg(stream_message_id)
        .query::<i64>(conn)
        .map_err(|e| IntentError::runtime_error(format!("Failed to XACK failed job: {}", e)))?;

    if attempts < max_attempts {
        // Retry with exponential backoff — schedule it
        let backoff_ms = backoff_base_ms * (1u64 << (attempts as u64).saturating_sub(1).min(10));
        let scheduled_at = now_ms() + backoff_ms;

        redis::pipe()
            .cmd("HSET")
            .arg(&hash_key)
            .arg("status")
            .arg("scheduled")
            .arg("error")
            .arg(error)
            .arg("locked_by")
            .arg("")
            .arg("scheduled_at")
            .arg(scheduled_at)
            .cmd("ZADD")
            .arg("ntnt:scheduled")
            .arg(scheduled_at as f64)
            .arg(job_id)
            .query::<()>(conn)
            .map_err(|e| IntentError::runtime_error(format!("Failed to schedule retry: {}", e)))?;
    } else {
        // Dead
        let now = now_ms();
        redis::cmd("HSET")
            .arg(&hash_key)
            .arg("status")
            .arg("dead")
            .arg("error")
            .arg(error)
            .arg("completed_at")
            .arg(now)
            .arg("locked_by")
            .arg("")
            .query::<()>(conn)
            .map_err(|e| {
                IntentError::runtime_error(format!("Failed to mark Redis job dead: {}", e))
            })?;
    }

    // Drop the connection guard before pruning (which needs its own lock)
    drop(conn_guard);

    // Prune dead jobs if we just marked one dead
    if attempts >= max_attempts {
        let _ = redis_prune_dead_jobs();
    }

    Ok(())
}

/// Send heartbeat for an active Redis job
pub(crate) fn redis_heartbeat(job_id: &str) -> Result<()> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let hash_key = format!("ntnt:job:{}", job_id);
    let now = now_ms();

    redis::cmd("HSET")
        .arg(&hash_key)
        .arg("heartbeat_at")
        .arg(now)
        .query::<()>(conn)
        .map_err(|e| IntentError::runtime_error(format!("Redis heartbeat failed: {}", e)))?;

    Ok(())
}

/// Release stale jobs using XPENDING + XCLAIM
pub(crate) fn redis_release_stale_jobs(
    queues: &[String],
    group: &str,
    visibility_timeout_ms: u64,
) -> Result<u64> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let mut released = 0u64;

    for queue in queues {
        let stream_key = format!("ntnt:queue:{}", queue);

        // XPENDING stream group - + COUNT 100
        let pending: redis::RedisResult<redis::Value> = redis::cmd("XPENDING")
            .arg(&stream_key)
            .arg(group)
            .arg("-")
            .arg("+")
            .arg(100)
            .query(conn);

        if let Ok(redis::Value::Array(entries)) = pending {
            for entry in &entries {
                if let redis::Value::Array(fields) = entry {
                    // [message_id, consumer, idle_time_ms, delivery_count]
                    let message_id = match fields.first() {
                        Some(redis::Value::BulkString(b)) => String::from_utf8_lossy(b).to_string(),
                        Some(redis::Value::SimpleString(s)) => s.clone(),
                        _ => continue,
                    };
                    let idle_ms: u64 = match fields.get(2) {
                        Some(redis::Value::Int(n)) => *n as u64,
                        _ => continue,
                    };

                    if idle_ms > visibility_timeout_ms {
                        // XCLAIM the stale message to a recovery worker
                        let claim_result: redis::RedisResult<redis::Value> = redis::cmd("XCLAIM")
                            .arg(&stream_key)
                            .arg(group)
                            .arg("ntnt_recovery")
                            .arg(visibility_timeout_ms)
                            .arg(&message_id)
                            .query(conn);

                        if claim_result.is_ok() {
                            // Extract job_id from the claimed message and reset it
                            let read_result: redis::RedisResult<redis::Value> =
                                redis::cmd("XRANGE")
                                    .arg(&stream_key)
                                    .arg(&message_id)
                                    .arg(&message_id)
                                    .query(conn);

                            if let Ok(redis::Value::Array(msgs)) = read_result {
                                if let Some(redis::Value::Array(msg)) = msgs.first() {
                                    let job_id =
                                        if let Some(redis::Value::Array(fields)) = msg.get(1) {
                                            let mut id = None;
                                            let mut i = 0;
                                            while i + 1 < fields.len() {
                                                let key = match &fields[i] {
                                                    redis::Value::BulkString(b) => {
                                                        String::from_utf8_lossy(b).to_string()
                                                    }
                                                    redis::Value::SimpleString(s) => s.clone(),
                                                    _ => {
                                                        i += 2;
                                                        continue;
                                                    }
                                                };
                                                let val = match &fields[i + 1] {
                                                    redis::Value::BulkString(b) => {
                                                        String::from_utf8_lossy(b).to_string()
                                                    }
                                                    redis::Value::SimpleString(s) => s.clone(),
                                                    _ => {
                                                        i += 2;
                                                        continue;
                                                    }
                                                };
                                                if key == "job_id" {
                                                    id = Some(val);
                                                }
                                                i += 2;
                                            }
                                            id
                                        } else {
                                            None
                                        };

                                    if let Some(job_id) = job_id {
                                        let hash_key = format!("ntnt:job:{}", job_id);
                                        // ACK old message, reset job to pending, re-add to stream
                                        let _ = redis::cmd("XACK")
                                            .arg(&stream_key)
                                            .arg(group)
                                            .arg(&message_id)
                                            .query::<i64>(conn);

                                        let _ = redis::cmd("HSET")
                                            .arg(&hash_key)
                                            .arg("status")
                                            .arg("pending")
                                            .arg("locked_by")
                                            .arg("")
                                            .query::<()>(conn);

                                        let _ = redis::cmd("XADD")
                                            .arg(&stream_key)
                                            .arg("*")
                                            .arg("job_id")
                                            .arg(&job_id)
                                            .query::<String>(conn);

                                        released += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(released)
}

/// Move scheduled jobs that are ready to their queue streams
pub(crate) fn redis_promote_scheduled_jobs() -> Result<u64> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let now = now_ms();

    // ZRANGEBYSCORE ntnt:scheduled -inf {now}
    let ready: redis::RedisResult<Vec<String>> = redis::cmd("ZRANGEBYSCORE")
        .arg("ntnt:scheduled")
        .arg("-inf")
        .arg(now)
        .query(conn);

    let ready_ids = match ready {
        Ok(ids) => ids,
        Err(_) => return Ok(0),
    };

    let mut promoted = 0u64;

    for job_id in &ready_ids {
        let hash_key = format!("ntnt:job:{}", job_id);

        // Get the queue name from the hash
        let queue: redis::RedisResult<String> =
            redis::cmd("HGET").arg(&hash_key).arg("queue").query(conn);

        if let Ok(queue_name) = queue {
            let stream_key = format!("ntnt:queue:{}", queue_name);

            // Move to stream + update status + remove from scheduled set
            let _ = redis::pipe()
                .cmd("HSET")
                .arg(&hash_key)
                .arg("status")
                .arg("pending")
                .cmd("XADD")
                .arg(&stream_key)
                .arg("*")
                .arg("job_id")
                .arg(job_id)
                .cmd("ZREM")
                .arg("ntnt:scheduled")
                .arg(job_id)
                .query::<()>(conn);

            promoted += 1;
        }
    }

    Ok(promoted)
}

// ============================================================
// Status / List / Recent / Dead
// ============================================================

/// Get queue status from Redis — scan job hashes by status
pub fn redis_queue_status() -> Result<HashMap<String, i64>> {
    redis_queue_status_detailed(false, None)
}

/// Get detailed queue stats from Redis
fn redis_queue_status_detailed(
    _per_queue: bool,
    _queue_filter: Option<&str>,
) -> Result<HashMap<String, i64>> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let mut counts: HashMap<String, i64> = HashMap::new();
    counts.insert("pending".to_string(), 0);
    counts.insert("active".to_string(), 0);
    counts.insert("completed".to_string(), 0);
    counts.insert("retry".to_string(), 0);
    counts.insert("dead".to_string(), 0);
    counts.insert("cancelled".to_string(), 0);
    counts.insert("scheduled".to_string(), 0);

    // SCAN for ntnt:job:* keys
    let mut cursor: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:job:*")
            .arg("COUNT")
            .arg(100)
            .query(conn)
            .map_err(|e| IntentError::runtime_error(format!("Redis SCAN failed: {}", e)))?;

        for key in &keys {
            let status: redis::RedisResult<String> =
                redis::cmd("HGET").arg(key).arg("status").query(conn);

            if let Ok(s) = status {
                *counts.entry(s).or_insert(0) += 1;
            }
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    Ok(counts)
}

/// Get per-queue status from Redis
pub(crate) fn redis_per_queue_status() -> Result<HashMap<String, HashMap<String, i64>>> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
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
        "scheduled",
    ] {
        total.insert(s.to_string(), 0);
    }

    let mut cursor: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:job:*")
            .arg("COUNT")
            .arg(100)
            .query(conn)
            .map_err(|e| IntentError::runtime_error(format!("Redis SCAN failed: {}", e)))?;

        for key in &keys {
            let fields: redis::RedisResult<(String, String)> = redis::cmd("HMGET")
                .arg(key)
                .arg("status")
                .arg("queue")
                .query(conn);

            if let Ok((status, queue)) = fields {
                *total.entry(status.clone()).or_insert(0) += 1;
                let queue_counts = per_queue.entry(queue).or_insert_with(|| {
                    let mut m = HashMap::new();
                    for s in &[
                        "pending",
                        "active",
                        "completed",
                        "retry",
                        "dead",
                        "cancelled",
                        "scheduled",
                    ] {
                        m.insert(s.to_string(), 0);
                    }
                    m
                });
                *queue_counts.entry(status).or_insert(0) += 1;
            }
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    per_queue.insert("total".to_string(), total);
    Ok(per_queue)
}

/// Get recent jobs from Redis
pub(crate) fn redis_recent_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let mut jobs: Vec<(u64, HashMap<String, String>)> = Vec::new();
    let mut cursor: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:job:*")
            .arg("COUNT")
            .arg(100)
            .query(conn)
            .map_err(|e| IntentError::runtime_error(format!("Redis SCAN failed: {}", e)))?;

        for key in &keys {
            let data: redis::RedisResult<HashMap<String, String>> =
                redis::cmd("HGETALL").arg(key).query(conn);
            if let Ok(d) = data {
                let created_at: u64 = d
                    .get("created_at")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                jobs.push((created_at, d));
            }
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    jobs.sort_by(|a, b| b.0.cmp(&a.0));
    jobs.truncate(limit as usize);

    Ok(jobs
        .into_iter()
        .map(|(_, data)| redis_hash_to_job_map(&data))
        .collect())
}

/// Get dead jobs from Redis
pub(crate) fn redis_dead_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let mut jobs: Vec<(u64, HashMap<String, String>)> = Vec::new();
    let mut cursor: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:job:*")
            .arg("COUNT")
            .arg(100)
            .query(conn)
            .map_err(|e| IntentError::runtime_error(format!("Redis SCAN failed: {}", e)))?;

        for key in &keys {
            let data: redis::RedisResult<HashMap<String, String>> =
                redis::cmd("HGETALL").arg(key).query(conn);
            if let Ok(d) = data {
                if d.get("status").map(|s| s.as_str()) == Some("dead") {
                    let created_at: u64 = d
                        .get("created_at")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    jobs.push((created_at, d));
                }
            }
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    jobs.sort_by(|a, b| b.0.cmp(&a.0));
    jobs.truncate(limit as usize);

    Ok(jobs
        .into_iter()
        .map(|(_, data)| redis_hash_to_job_map(&data))
        .collect())
}

/// List jobs from Redis with optional status filter
pub fn redis_list_jobs(status_filter: Option<&str>) -> Result<Vec<HashMap<String, Value>>> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let mut jobs: Vec<(u64, HashMap<String, String>)> = Vec::new();
    let mut cursor: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:job:*")
            .arg("COUNT")
            .arg(100)
            .query(conn)
            .map_err(|e| IntentError::runtime_error(format!("Redis SCAN failed: {}", e)))?;

        for key in &keys {
            let data: redis::RedisResult<HashMap<String, String>> =
                redis::cmd("HGETALL").arg(key).query(conn);
            if let Ok(d) = data {
                if let Some(filter) = status_filter {
                    if d.get("status").map(|s| s.as_str()) != Some(filter) {
                        continue;
                    }
                }
                let created_at: u64 = d
                    .get("created_at")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                jobs.push((created_at, d));
            }
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    jobs.sort_by(|a, b| b.0.cmp(&a.0));
    if jobs.len() > 100 {
        jobs.truncate(100);
    }

    Ok(jobs
        .into_iter()
        .map(|(_, data)| redis_hash_to_job_map(&data))
        .collect())
}

/// Retry a dead Redis job
pub fn redis_retry_dead_job(job_id: &str) -> Result<bool> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let hash_key = format!("ntnt:job:{}", job_id);

    // Check status is dead
    let status: redis::RedisResult<String> =
        redis::cmd("HGET").arg(&hash_key).arg("status").query(conn);

    match status {
        Ok(s) if s == "dead" => {
            let queue: String = redis::cmd("HGET")
                .arg(&hash_key)
                .arg("queue")
                .query(conn)
                .unwrap_or_else(|_| "default".to_string());
            let stream_key = format!("ntnt:queue:{}", queue);

            redis::pipe()
                .cmd("HSET")
                .arg(&hash_key)
                .arg("status")
                .arg("pending")
                .arg("attempts")
                .arg(0i64)
                .arg("error")
                .arg("")
                .arg("locked_by")
                .arg("")
                .cmd("XADD")
                .arg(&stream_key)
                .arg("*")
                .arg("job_id")
                .arg(job_id)
                .query::<()>(conn)
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to retry Redis job: {}", e))
                })?;

            // Remove EXPIRE if any was set
            redis::cmd("PERSIST").arg(&hash_key).query::<()>(conn).ok();

            Ok(true)
        }
        _ => Ok(false),
    }
}

/// Cancel a Redis job
pub fn redis_cancel_job(job_id: &str) -> Result<bool> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let hash_key = format!("ntnt:job:{}", job_id);

    let status: redis::RedisResult<String> =
        redis::cmd("HGET").arg(&hash_key).arg("status").query(conn);

    match status {
        Ok(s) if s == "pending" || s == "active" || s == "scheduled" => {
            redis::cmd("HSET")
                .arg(&hash_key)
                .arg("status")
                .arg("cancelled")
                .arg("locked_by")
                .arg("")
                .query::<()>(conn)
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to cancel Redis job: {}", e))
                })?;

            // If scheduled, remove from scheduled set
            if s == "scheduled" {
                redis::cmd("ZREM")
                    .arg("ntnt:scheduled")
                    .arg(job_id)
                    .query::<()>(conn)
                    .ok();
            }

            Ok(true)
        }
        _ => Ok(false),
    }
}

// ============================================================
// Queue Pause/Resume
// ============================================================

pub(crate) fn redis_pause_queue(queue_name: &str) -> Result<()> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    redis::cmd("SADD")
        .arg("ntnt:paused")
        .arg(queue_name)
        .query::<()>(conn)
        .map_err(|e| IntentError::runtime_error(format!("Failed to pause queue: {}", e)))?;

    Ok(())
}

pub(crate) fn redis_resume_queue(queue_name: &str) -> Result<()> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    redis::cmd("SREM")
        .arg("ntnt:paused")
        .arg(queue_name)
        .query::<()>(conn)
        .map_err(|e| IntentError::runtime_error(format!("Failed to resume queue: {}", e)))?;

    Ok(())
}

pub(crate) fn redis_paused_queues() -> Result<Vec<String>> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let members: Vec<String> = redis::cmd("SMEMBERS")
        .arg("ntnt:paused")
        .query(conn)
        .map_err(|e| IntentError::runtime_error(format!("Failed to get paused queues: {}", e)))?;

    Ok(members)
}

pub(crate) fn redis_is_queue_paused(queue_name: &str) -> Result<bool> {
    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    let is_member: bool = redis::cmd("SISMEMBER")
        .arg("ntnt:paused")
        .arg(queue_name)
        .query(conn)
        .map_err(|e| IntentError::runtime_error(format!("Failed to check paused state: {}", e)))?;

    Ok(is_member)
}

// ============================================================
// Dead Job Pruning
// ============================================================

pub(crate) fn redis_prune_dead_jobs() -> Result<()> {
    let max_dead = MAX_DEAD_JOBS.load(Ordering::Relaxed) as usize;
    let retention_secs = DEAD_RETENTION_SECS.load(Ordering::Relaxed);
    let now = now_ms();
    let cutoff_ms = now.saturating_sub(retention_secs * 1000);

    let mut conn_guard = get_redis_conn()?;
    let conn = conn_guard.as_mut().ok_or_else(|| {
        IntentError::runtime_error("Redis job connection not initialized".to_string())
    })?;

    // Scan for dead jobs
    let mut dead_jobs: Vec<(String, u64)> = Vec::new(); // (job_key, completed_at_ms)
    let mut cursor: u64 = 0;
    loop {
        let (next_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
            .arg(cursor)
            .arg("MATCH")
            .arg("ntnt:job:*")
            .arg("COUNT")
            .arg(100)
            .query(conn)
            .map_err(|e| IntentError::runtime_error(format!("Redis SCAN failed: {}", e)))?;

        for key in &keys {
            let fields: redis::RedisResult<(String, String)> = redis::cmd("HMGET")
                .arg(key)
                .arg("status")
                .arg("completed_at")
                .query(conn);
            if let Ok((status, completed_at_str)) = fields {
                if status == "dead" {
                    let completed_at: u64 = completed_at_str.parse().unwrap_or(0);
                    dead_jobs.push((key.clone(), completed_at));
                }
            }
        }

        cursor = next_cursor;
        if cursor == 0 {
            break;
        }
    }

    // Sort by completed_at descending (newest first)
    dead_jobs.sort_by(|a, b| b.1.cmp(&a.1));

    // Prune by count (keep only max_dead most recent)
    if dead_jobs.len() > max_dead {
        for (key, _) in &dead_jobs[max_dead..] {
            let _ = redis::cmd("DEL").arg(key).query::<()>(conn);
        }
        dead_jobs.truncate(max_dead);
    }

    // Prune by age
    for (key, completed_at) in &dead_jobs {
        if *completed_at > 0 && *completed_at < cutoff_ms {
            let _ = redis::cmd("DEL").arg(key).query::<()>(conn);
        }
    }

    Ok(())
}

// ============================================================
// Helpers
// ============================================================

/// Convert a Redis hash data map to a job Value map
fn redis_hash_to_job_map(data: &HashMap<String, String>) -> HashMap<String, Value> {
    let mut map = HashMap::new();
    if let Some(id) = data.get("id") {
        map.insert("id".to_string(), Value::String(id.clone()));
    }
    if let Some(t) = data.get("type") {
        map.insert("type".to_string(), Value::String(t.clone()));
    }
    if let Some(q) = data.get("queue") {
        map.insert("queue".to_string(), Value::String(q.clone()));
    }
    if let Some(s) = data.get("status") {
        map.insert("status".to_string(), Value::String(s.clone()));
    }
    if let Some(p) = data.get("priority") {
        if let Ok(n) = p.parse::<i64>() {
            map.insert("priority".to_string(), Value::Int(n));
        }
    }
    if let Some(a) = data.get("attempts") {
        if let Ok(n) = a.parse::<i64>() {
            map.insert("attempt".to_string(), Value::Int(n));
        }
    }
    if let Some(m) = data.get("max_attempts") {
        if let Ok(n) = m.parse::<i64>() {
            map.insert("max_attempts".to_string(), Value::Int(n));
        }
    }
    if let Some(err) = data.get("error") {
        if !err.is_empty() {
            map.insert("error".to_string(), Value::String(err.clone()));
        }
    }
    if let Some(payload_str) = data.get("payload") {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(payload_str) {
            map.insert(
                "payload".to_string(),
                crate::stdlib::json::json_to_intent_value(&json),
            );
        }
    }
    if let Some(created) = data.get("created_at") {
        map.insert("created_at".to_string(), Value::String(created.clone()));
    }
    map
}

/// Execute a Redis job (args from JSON payload hash)
pub(crate) fn execute_redis_job(job: &RedisJobRow) -> std::result::Result<Value, String> {
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
