//! JobBackend trait definition for pluggable job queue backends.
//!
//! This trait defines the interface that all job queue backends must implement.
//! Currently serves as the architectural foundation for future trait-based dispatch.

use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::concurrent::SerializedValue;
use std::collections::HashMap;

/// Result type for backend operations
pub type Result<T> = std::result::Result<T, IntentError>;

/// A claimed job — unified representation across all backends.
/// Used by the generic worker loop to execute jobs regardless of backend.
#[derive(Debug, Clone)]
pub struct ClaimedJob {
    pub id: String,
    pub job_type: String,
    pub queue_name: String,
    pub payload: serde_json::Value,
    pub priority: i64,
    pub attempts: i64,
    pub max_attempts: i64,
    /// Redis stream message ID (empty for other backends)
    pub stream_message_id: String,
}

/// Pluggable backend trait for the job queue system.
///
/// Each backend (memory, postgres, redis) implements this trait to provide
/// queue operations through a unified interface.
#[allow(dead_code)]
pub trait JobBackend: Send + Sync {
    /// Enqueue a job for processing
    fn enqueue(
        &self,
        job_type: &str,
        queue: &str,
        args: &HashMap<String, SerializedValue>,
        max_attempts: i64,
        priority: i64,
        scheduled_at_offset_ms: Option<u64>,
        scheduled_at_timestamp_ms: Option<u64>,
        unique_for: Option<u64>,
    ) -> Result<String>;

    /// Claim the next ready job from the given queues
    fn claim_next(&self, queues: &[String], worker_id: &str) -> Result<Option<ClaimedJob>>;

    /// Mark a job as completed
    fn complete(&self, job_id: &str, result: Option<serde_json::Value>) -> Result<()>;

    /// Mark a job as failed (handles retry/dead logic internally)
    fn fail(
        &self,
        job_id: &str,
        error: &str,
        attempts: i64,
        max_attempts: i64,
        backoff_base_ms: u64,
    ) -> Result<()>;

    /// Cancel a job
    fn cancel(&self, job_id: &str) -> Result<bool>;

    /// Retry a dead job (move back to pending)
    fn retry(&self, job_id: &str) -> Result<bool>;

    /// Get global queue status counts by state
    fn status(&self) -> Result<HashMap<String, i64>>;

    /// Get per-queue status breakdowns
    fn per_queue_status(
        &self,
        queue_filter: Option<&str>,
    ) -> Result<HashMap<String, HashMap<String, i64>>>;

    /// List jobs with optional status filter
    fn list(&self, status_filter: Option<&str>) -> Result<Vec<HashMap<String, Value>>>;

    /// Get recent jobs
    fn recent(&self, limit: i64) -> Result<Vec<HashMap<String, Value>>>;

    /// Get dead jobs
    fn dead(&self, limit: i64) -> Result<Vec<HashMap<String, Value>>>;

    /// Pause a queue
    fn pause(&self, queue: &str) -> Result<()>;

    /// Resume a paused queue
    fn resume(&self, queue: &str) -> Result<()>;

    /// Get list of paused queues
    fn paused(&self) -> Result<Vec<String>>;

    /// Check if a queue is paused
    fn is_paused(&self, queue: &str) -> Result<bool>;

    /// Prune dead jobs beyond caps
    fn prune_dead(&self, max_jobs: u64, retention_secs: u64) -> Result<()>;

    /// Send heartbeat for an active job
    fn heartbeat(&self, job_id: &str) -> Result<()>;

    /// Release stale/abandoned jobs
    fn release_stale(&self, timeout_secs: u64) -> Result<u64>;

    /// Backend name ("memory", "postgres", "redis")
    fn name(&self) -> &str;
}
