//! In-memory job queue backend.
//!
//! Provides the default memory-based queue with priority scheduling,
//! dead letter queue, and completed job pruning.

use super::{compute_args_hash, now_ms, JobStatus, QueuedJob, JOB_ID_COUNTER};
use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::concurrent::SerializedValue;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

// ============================================================
// Queue Configuration and State
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

// ============================================================
// Unique Jobs — in-memory dedup map
// ============================================================

/// In-memory unique job dedup map: hash → (expires_at, job_id)
static UNIQUE_JOBS_MAP: std::sync::LazyLock<Mutex<HashMap<String, (Instant, String)>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Check unique constraint in memory backend. Returns existing job ID if duplicate found.
pub(crate) fn check_unique_memory(
    job_type: &str,
    args: &HashMap<String, SerializedValue>,
    _unique_secs: u64,
) -> Option<String> {
    let hash = compute_args_hash(job_type, args);
    let mut map = UNIQUE_JOBS_MAP.lock().ok()?;

    // Clean expired entries
    let now = Instant::now();
    map.retain(|_, (expires, _)| *expires > now);

    // Check for existing
    if let Some((_, job_id)) = map.get(&hash) {
        return Some(job_id.clone());
    }
    None
}

/// Register a unique job in memory backend
pub(crate) fn register_unique_memory(
    job_type: &str,
    args: &HashMap<String, SerializedValue>,
    unique_secs: u64,
    job_id: &str,
) {
    let hash = compute_args_hash(job_type, args);
    if let Ok(mut map) = UNIQUE_JOBS_MAP.lock() {
        map.insert(
            hash,
            (
                Instant::now() + Duration::from_secs(unique_secs),
                job_id.to_string(),
            ),
        );
    }
}

// ============================================================
// Queue Pause/Resume (in-memory)
// ============================================================

/// In-memory set of paused queue names
static PAUSED_QUEUES: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Pause a queue — workers will skip it
pub(crate) fn memory_pause_queue(queue_name: &str) -> Result<()> {
    let mut paused = PAUSED_QUEUES
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock paused queues: {}", e)))?;
    paused.insert(queue_name.to_string());
    Ok(())
}

/// Resume a paused queue
pub(crate) fn memory_resume_queue(queue_name: &str) -> Result<()> {
    let mut paused = PAUSED_QUEUES
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock paused queues: {}", e)))?;
    paused.remove(queue_name);
    Ok(())
}

/// Get list of paused queues
pub(crate) fn memory_paused_queues() -> Result<Vec<String>> {
    let paused = PAUSED_QUEUES
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock paused queues: {}", e)))?;
    Ok(paused.iter().cloned().collect())
}

/// Check if a queue is paused (in-memory)
pub(crate) fn is_queue_paused_memory(queue_name: &str) -> bool {
    PAUSED_QUEUES
        .lock()
        .map(|p| p.contains(queue_name))
        .unwrap_or(false)
}

// ============================================================
// Enqueue (in-memory)
// ============================================================

/// In-memory enqueue implementation
pub(crate) fn enqueue_job_memory(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    scheduled_at: Instant,
    priority: i64,
) -> Result<String> {
    let def = super::get_job_definition(job_type)?
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
pub(crate) fn requeue_for_retry(job: &mut QueuedJob, backoff_base_ms: u64) -> Result<()> {
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
// Claim (in-memory)
// ============================================================

/// Claim the next ready job from any queue (in-memory)
pub(crate) fn claim_next_job() -> Result<Option<QueuedJob>> {
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

// ============================================================
// Cancel (in-memory)
// ============================================================

pub(crate) fn cancel_job_memory(job_id: &str) -> Result<bool> {
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

// ============================================================
// Status / List (in-memory)
// ============================================================

pub(crate) fn queue_status_memory() -> Result<HashMap<String, i64>> {
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

/// Get per-queue stats from in-memory backend
pub(crate) fn memory_per_queue_status(
    queue_filter: Option<&str>,
) -> Result<HashMap<String, HashMap<String, i64>>> {
    let backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

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

    for job in backend.jobs_by_id.values() {
        if let Some(filter) = queue_filter {
            if job.queue_name != filter {
                continue;
            }
        }
        let status = job.status.as_str().to_string();
        *total.entry(status.clone()).or_insert(0) += 1;
        let queue_counts = per_queue.entry(job.queue_name.clone()).or_insert_with(|| {
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
        *queue_counts.entry(status).or_insert(0) += 1;
    }

    per_queue.insert("total".to_string(), total);
    Ok(per_queue)
}

pub(crate) fn retry_dead_job_memory(job_id: &str) -> Result<bool> {
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

pub(crate) fn list_jobs_memory(status_filter: Option<&str>) -> Result<Vec<HashMap<String, Value>>> {
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
