//! std/jobs module — Job DSL with pluggable backends (memory + postgres + redis)
//!
//! Provides background job processing with queues, retry logic, and lifecycle management.
//! Supports in-memory (default), PostgreSQL (persistent, distributed), and Redis Streams backends.
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

pub mod backend;
pub mod memory;
pub mod postgres;
pub mod redis_backend;
pub mod worker;

use crate::ast::{Block, Parameter};
use crate::error::IntentError;
use crate::interpreter::Value;
use crate::stdlib::concurrent::SerializedValue;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
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
    /// Unique job deduplication window in seconds. If set, prevents duplicate jobs
    /// with the same type + args combination within this time window.
    pub unique_for_secs: Option<u64>,
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

pub(crate) static JOB_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

/// A job instance that has been enqueued (in-memory representation)
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
    Redis(RedisBackendConfig),
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

/// Configuration for the Redis Streams backend
#[derive(Debug, Clone)]
pub struct RedisBackendConfig {
    pub redis_url: String,
    pub visibility_timeout_secs: u64,
    pub consumer_group: String,
    pub prune_completed_after_secs: u64,
}

impl Default for RedisBackendConfig {
    fn default() -> Self {
        RedisBackendConfig {
            redis_url: String::new(),
            visibility_timeout_secs: 300,
            consumer_group: "ntnt_workers".to_string(),
            prune_completed_after_secs: 3600,
        }
    }
}

/// Global backend selection — defaults to Memory
static ACTIVE_BACKEND: std::sync::LazyLock<Mutex<BackendKind>> =
    std::sync::LazyLock::new(|| Mutex::new(BackendKind::Memory));

/// Get the current backend kind
pub fn get_backend() -> Result<BackendKind> {
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
// Shared Global State
// ============================================================

/// Flag to stop accepting new jobs during shutdown
pub(crate) static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

/// Flag indicating the worker loop is running
pub(crate) static WORKER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Flag to request worker stop
pub(crate) static WORKER_STOP: AtomicBool = AtomicBool::new(false);

/// Max dead jobs to retain (default: 10000)
pub(crate) static MAX_DEAD_JOBS: AtomicU64 = AtomicU64::new(10000);

/// Dead job retention in seconds (default: 180 days = 15552000)
pub(crate) static DEAD_RETENTION_SECS: AtomicU64 = AtomicU64::new(15552000);

// ============================================================
// Unique Jobs — hash computation
// ============================================================

/// Compute a SHA256 hash of job_type + sorted JSON args for uniqueness
pub(crate) fn compute_args_hash(job_type: &str, args: &HashMap<String, SerializedValue>) -> String {
    use std::collections::BTreeMap;
    use std::hash::{Hash, Hasher};

    // Sort args by key for deterministic hashing
    let sorted: BTreeMap<&String, &SerializedValue> = args.iter().collect();
    let args_json = serde_json::to_string(
        &sorted
            .into_iter()
            .map(|(k, v)| (k.clone(), v.to_json()))
            .collect::<serde_json::Map<String, serde_json::Value>>(),
    )
    .unwrap_or_default();

    // Use a simple FNV-like hash (64-bit) for speed — collision rate acceptable for dedup
    let combined = format!("{}:{}", job_type, args_json);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    combined.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Utility: current time in milliseconds since epoch
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ============================================================
// Enqueueing (dispatches to active backend)
// ============================================================

/// Enqueue a job for immediate processing
pub fn enqueue_job(job_type: &str, args: HashMap<String, SerializedValue>) -> Result<String> {
    if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
        return Err(IntentError::runtime_error(
            "Cannot enqueue jobs during shutdown".to_string(),
        ));
    }

    let def = get_job_definition(job_type)?
        .ok_or_else(|| IntentError::runtime_error(format!("Unknown job type: {}", job_type)))?;

    let unique_for = def.unique_for_secs;
    let result = match get_backend()? {
        BackendKind::Memory => {
            // Check unique constraint for memory backend
            if let Some(unique_secs) = unique_for {
                if let Some(existing_id) = memory::check_unique_memory(job_type, &args, unique_secs)
                {
                    return Ok(existing_id);
                }
            }
            let id = memory::enqueue_job_memory(job_type, args.clone(), Instant::now(), 0)?;
            // Register in unique map
            if let Some(unique_secs) = unique_for {
                memory::register_unique_memory(job_type, &args, unique_secs, &id);
            }
            Ok(id)
        }
        BackendKind::Postgres(_) => postgres::pg_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            None,
            None,
            unique_for,
        ),
        BackendKind::Redis(_) => redis_backend::redis_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            None,
            None,
            unique_for,
        ),
    };

    result
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

    let unique_for = def.unique_for_secs;
    match get_backend()? {
        BackendKind::Memory => {
            let scheduled = Instant::now() + Duration::from_millis(delay_ms);
            memory::enqueue_job_memory(job_type, args, scheduled, 0)
        }
        BackendKind::Postgres(_) => postgres::pg_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            Some(delay_ms),
            None,
            unique_for,
        ),
        BackendKind::Redis(_) => redis_backend::redis_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            Some(delay_ms),
            None,
            unique_for,
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

    let unique_for = def.unique_for_secs;
    match get_backend()? {
        BackendKind::Memory => {
            let now = now_ms();
            let delay = if timestamp_ms > now {
                timestamp_ms - now
            } else {
                0
            };
            let scheduled = Instant::now() + Duration::from_millis(delay);
            memory::enqueue_job_memory(job_type, args, scheduled, 0)
        }
        BackendKind::Postgres(_) => postgres::pg_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            None,
            Some(timestamp_ms),
            unique_for,
        ),
        BackendKind::Redis(_) => redis_backend::redis_enqueue_job(
            job_type,
            &def.queue,
            &args,
            def.max_retries + 1,
            0,
            None,
            Some(timestamp_ms),
            unique_for,
        ),
    }
}

/// Enqueue a job within an existing PostgreSQL transaction.
pub fn enqueue_job_tx(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    tx_handle: &Value,
) -> Result<String> {
    postgres::pg_enqueue_job_tx(job_type, args, tx_handle)
}

// ============================================================
// Queue Pause/Resume
// ============================================================

/// Pause a queue — workers will skip it
pub fn pause_queue(queue_name: &str) -> Result<()> {
    match get_backend()? {
        BackendKind::Memory => memory::memory_pause_queue(queue_name),
        BackendKind::Postgres(_) => postgres::pg_pause_queue(queue_name),
        BackendKind::Redis(_) => redis_backend::redis_pause_queue(queue_name),
    }
}

/// Resume a paused queue
pub fn resume_queue(queue_name: &str) -> Result<()> {
    match get_backend()? {
        BackendKind::Memory => memory::memory_resume_queue(queue_name),
        BackendKind::Postgres(_) => postgres::pg_resume_queue(queue_name),
        BackendKind::Redis(_) => redis_backend::redis_resume_queue(queue_name),
    }
}

/// Get list of paused queues
pub fn paused_queues() -> Result<Vec<String>> {
    match get_backend()? {
        BackendKind::Memory => memory::memory_paused_queues(),
        BackendKind::Postgres(_) => postgres::pg_paused_queues(),
        BackendKind::Redis(_) => redis_backend::redis_paused_queues(),
    }
}

// ============================================================
// Worker
// ============================================================

/// Start the background worker loop (dispatches to correct backend)
pub fn start_worker() -> Result<()> {
    match get_backend()? {
        BackendKind::Memory => worker::start_worker_memory(),
        BackendKind::Postgres(config) => worker::start_worker_postgres(config, None),
        BackendKind::Redis(config) => worker::start_worker_redis(config, None),
    }
}

/// Start the worker with timeout enforcement via spawn tasks
pub fn start_worker_with_timeouts() -> Result<()> {
    start_worker()
}

/// Start a blocking worker (for dedicated worker processes)
pub fn start_worker_blocking(opts: &HashMap<String, Value>) -> Result<()> {
    worker::start_worker_blocking_impl(opts)
}

// ============================================================
// Queue Operations (dispatch to active backend)
// ============================================================

/// Cancel a job by ID
pub fn cancel_job(job_id: &str) -> Result<bool> {
    match get_backend()? {
        BackendKind::Postgres(_) => postgres::pg_cancel_job(job_id),
        BackendKind::Redis(_) => redis_backend::redis_cancel_job(job_id),
        BackendKind::Memory => memory::cancel_job_memory(job_id),
    }
}

/// Get queue status — counts by state (global totals)
pub fn queue_status() -> Result<HashMap<String, i64>> {
    match get_backend()? {
        BackendKind::Postgres(_) => postgres::pg_queue_status(),
        BackendKind::Redis(_) => redis_backend::redis_queue_status(),
        BackendKind::Memory => memory::queue_status_memory(),
    }
}

/// Get per-queue stats — returns nested map with "total" and per-queue breakdowns
pub fn queue_status_per_queue(
    queue_filter: Option<&str>,
) -> Result<HashMap<String, HashMap<String, i64>>> {
    match get_backend()? {
        BackendKind::Postgres(_) => postgres::pg_per_queue_status(),
        BackendKind::Redis(_) => redis_backend::redis_per_queue_status(),
        BackendKind::Memory => memory::memory_per_queue_status(queue_filter),
    }
}

/// Retry a dead job (move back to pending)
pub fn retry_dead_job(job_id: &str) -> Result<bool> {
    match get_backend()? {
        BackendKind::Postgres(_) => postgres::pg_retry_dead_job(job_id),
        BackendKind::Redis(_) => redis_backend::redis_retry_dead_job(job_id),
        BackendKind::Memory => memory::retry_dead_job_memory(job_id),
    }
}

/// List jobs with optional status filter
pub fn list_jobs(status_filter: Option<&str>) -> Result<Vec<HashMap<String, Value>>> {
    match get_backend()? {
        BackendKind::Postgres(_) => postgres::pg_list_jobs(status_filter),
        BackendKind::Redis(_) => redis_backend::redis_list_jobs(status_filter),
        BackendKind::Memory => memory::list_jobs_memory(status_filter),
    }
}

/// Get recent jobs
pub fn recent_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    match get_backend()? {
        BackendKind::Postgres(_) => postgres::pg_recent_jobs(limit),
        BackendKind::Redis(_) => redis_backend::redis_recent_jobs(limit),
        BackendKind::Memory => memory::list_jobs_memory(None), // memory doesn't sort by recent
    }
}

/// Get dead jobs
pub fn dead_jobs(limit: i64) -> Result<Vec<HashMap<String, Value>>> {
    match get_backend()? {
        BackendKind::Postgres(_) => postgres::pg_dead_jobs(limit),
        BackendKind::Redis(_) => redis_backend::redis_dead_jobs(limit),
        BackendKind::Memory => memory::list_jobs_memory(Some("dead")),
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
                postgres::pg_init_job_pool(&url)?;

                set_backend(BackendKind::Postgres(pg_config))?;
            }
            "redis" => {
                let url = match config_map.get("redis_url") {
                    Some(Value::String(u)) => u.clone(),
                    _ => match config_map.get("url") {
                        Some(Value::String(u)) => u.clone(),
                        _ => std::env::var("REDIS_URL").map_err(|_| {
                            IntentError::runtime_error(
                                "Queue.configure() with redis backend requires 'redis_url', 'url', or REDIS_URL env var".to_string(),
                            )
                        })?,
                    },
                };

                let mut redis_config = RedisBackendConfig::default();
                redis_config.redis_url = url.clone();

                if let Some(Value::Int(secs)) = config_map.get("visibility_timeout") {
                    redis_config.visibility_timeout_secs = *secs as u64;
                }
                if let Some(Value::String(group)) = config_map.get("consumer_group") {
                    redis_config.consumer_group = group.clone();
                }
                if let Some(Value::Int(secs)) = config_map.get("prune_completed_after") {
                    redis_config.prune_completed_after_secs = *secs as u64;
                }

                // Initialize connection
                redis_backend::redis_init(&url, &redis_config.consumer_group)?;

                set_backend(BackendKind::Redis(redis_config))?;
            }
            other => {
                return Err(IntentError::runtime_error(format!(
                    "Unknown backend '{}'. Use 'memory', 'postgres', or 'redis'",
                    other
                )));
            }
        }
    }

    // Also apply memory backend config
    if matches!(get_backend()?, BackendKind::Memory) {
        let mut backend = memory::QUEUE_BACKEND.lock().map_err(|e| {
            IntentError::runtime_error(format!("Failed to lock queue backend: {}", e))
        })?;

        if let Some(Value::Int(ms)) = config_map.get("shutdown_timeout") {
            backend.config.shutdown_timeout_ms = *ms as u64;
        }
        if let Some(Value::Int(ms)) = config_map.get("prune_completed_after") {
            backend.config.prune_completed_after_ms = *ms as u64;
        }
    }

    // Dead job caps config (applies to all backends)
    if let Some(Value::Int(n)) = config_map.get("max_dead_jobs") {
        MAX_DEAD_JOBS.store(*n as u64, Ordering::Relaxed);
    }
    if let Some(Value::Int(secs)) = config_map.get("dead_retention_secs") {
        DEAD_RETENTION_SECS.store(*secs as u64, Ordering::Relaxed);
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
        memory::QUEUE_BACKEND
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
        if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
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
                        .or_insert_with(std::collections::VecDeque::new)
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

    if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
        backend.queues.clear();
        backend.jobs_by_id.clear();
        backend.dead_jobs.clear();
        backend.completed_jobs.clear();
        backend.config = memory::QueueConfig::default();
    }

    if let Ok(mut registry) = JOB_REGISTRY.lock() {
        registry.clear();
    }

    // Reset Redis connection
    if let Ok(mut conn) = redis_backend::JOB_REDIS_CONN.lock() {
        *conn = None;
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
    module.insert("Queue".to_string(), create_queue_module());

    module
}

// ============================================================
// Re-exports for external consumers
// ============================================================

// Re-exports for external consumers (main.rs CLI, concurrent.rs)
pub use postgres::{get_job_pool, pg_init_pool_from_url};
pub use postgres::{pg_cancel_job, pg_list_jobs, pg_queue_status, pg_retry_dead_job};
pub use redis_backend::{
    redis_cancel_job, redis_init_from_url, redis_list_jobs, redis_queue_status,
    redis_retry_dead_job,
};
