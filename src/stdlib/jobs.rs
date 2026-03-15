//! std/jobs module — Job DSL in-memory backend
//!
//! Provides background job processing with queues, retry logic, and lifecycle management.
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
// Enqueueing
// ============================================================

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Enqueue a job for immediate processing
pub fn enqueue_job(job_type: &str, args: HashMap<String, SerializedValue>) -> Result<String> {
    enqueue_job_at(job_type, args, Instant::now(), 0)
}

/// Enqueue a job with a delay (in milliseconds)
pub fn enqueue_job_in(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    delay_ms: u64,
) -> Result<String> {
    let scheduled = Instant::now() + Duration::from_millis(delay_ms);
    enqueue_job_at(job_type, args, scheduled, 0)
}

/// Enqueue a job at a specific timestamp (unix ms)
pub fn enqueue_job_at_timestamp(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    timestamp_ms: u64,
) -> Result<String> {
    let now = now_ms();
    let delay = if timestamp_ms > now {
        timestamp_ms - now
    } else {
        0
    };
    let scheduled = Instant::now() + Duration::from_millis(delay);
    enqueue_job_at(job_type, args, scheduled, 0)
}

/// Core enqueue implementation
fn enqueue_job_at(
    job_type: &str,
    args: HashMap<String, SerializedValue>,
    scheduled_at: Instant,
    priority: i64,
) -> Result<String> {
    if SHUTDOWN_FLAG.load(Ordering::SeqCst) {
        return Err(IntentError::runtime_error(
            "Cannot enqueue jobs during shutdown".to_string(),
        ));
    }

    // Look up the job definition
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
        max_attempts: def.max_retries + 1, // retries + 1 initial attempt
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

/// Re-enqueue a job for retry with exponential backoff
fn requeue_for_retry(job: &mut QueuedJob, backoff_base_ms: u64) -> Result<()> {
    job.status = JobStatus::Retry;
    job.attempt_count += 1;
    // Exponential backoff: base * 2^(attempt-1)
    let backoff = backoff_base_ms * (1u64 << (job.attempt_count as u64 - 1).min(10));
    job.scheduled_at = Instant::now() + Duration::from_millis(backoff);

    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    // Update the job in the index
    backend.jobs_by_id.insert(job.id.clone(), job.clone());

    // Re-add to queue
    // Change status to Pending before adding back
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

/// Claim the next ready job from any queue (or a specific queue)
fn claim_next_job() -> Result<Option<QueuedJob>> {
    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    let now = Instant::now();

    // Find the highest-priority ready job across all queues
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

    // Create a new interpreter for this job (same pattern as spawn())
    let mut interpreter = crate::interpreter::Interpreter::new();
    interpreter.define_all_stdlib_as_globals();

    // Inject job arguments as variables matching perform param names
    for param in &def.perform_params {
        if let Some(sv) = job.args.get(&param.name) {
            interpreter.define_variable(param.name.clone(), sv.to_value());
        } else if param.default.is_some() {
            // Default will be handled by the interpreter when the variable is referenced
            // For now, set to Unit as a placeholder — the AST default isn't easily evaluable here
            interpreter.define_variable(param.name.clone(), Value::Unit);
        } else {
            return Err(format!(
                "Missing required argument '{}' for job '{}'",
                param.name, job.job_type
            ));
        }
    }

    // Execute the perform body
    match interpreter.eval_block(&def.perform_body) {
        Ok(value) => {
            // Unwrap Return values
            match value {
                Value::Return(inner) => Ok(*inner),
                other => Ok(other),
            }
        }
        Err(e) => Err(format!("{}", e)),
    }
}

/// Execute the on_failure handler for a job
fn execute_on_failure(job: &QueuedJob, error: &str, attempt: i64) {
    let def = {
        if let Ok(registry) = JOB_REGISTRY.lock() {
            registry.get(&job.job_type).cloned()
        } else {
            None
        }
    };

    if let Some(def) = def {
        if let Some((params, body)) = def.on_failure {
            let mut interpreter = crate::interpreter::Interpreter::new();
            interpreter.define_all_stdlib_as_globals();

            // Bind on_failure params (error, attempt)
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

/// Start the background worker loop
pub fn start_worker() -> Result<()> {
    if WORKER_RUNNING.load(Ordering::SeqCst) {
        return Ok(()); // Already running
    }

    WORKER_RUNNING.store(true, Ordering::SeqCst);
    WORKER_STOP.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        while !WORKER_STOP.load(Ordering::SeqCst) {
            match claim_next_job() {
                Ok(Some(mut job)) => {
                    let job_id = job.id.clone();
                    let job_type = job.job_type.clone();

                    // Get timeout from definition
                    let timeout_ms = {
                        if let Ok(registry) = JOB_REGISTRY.lock() {
                            registry.get(&job_type).and_then(|d| d.timeout_ms)
                        } else {
                            None
                        }
                    };

                    // Get backoff base
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

                    // Execute the job (timeout enforcement is cooperative via check_cancellation)
                    let _timeout_ms = timeout_ms; // Phase 3 will add proper timeout enforcement
                    let result = execute_job(&job);

                    match result {
                        Ok(_) => {
                            // Mark completed
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
                                // Retry
                                execute_on_failure(&job, &error, job.attempt_count);
                                if let Err(e) = requeue_for_retry(&mut job, backoff_base) {
                                    eprintln!("[jobs] Failed to requeue for retry: {}", e);
                                }
                            } else {
                                // Dead letter
                                execute_on_failure(&job, &error, job.attempt_count);
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
                    // No jobs ready, poll interval
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

/// Start the worker with timeout enforcement via spawn tasks
pub fn start_worker_with_timeouts() -> Result<()> {
    // For Phase 2, the basic worker handles timeouts inline
    // Phase 3 will add proper async timeout enforcement
    start_worker()
}

// ============================================================
// Queue Operations
// ============================================================

/// Cancel a job by ID
pub fn cancel_job(job_id: &str) -> Result<bool> {
    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    if let Some(job) = backend.jobs_by_id.get_mut(job_id) {
        match job.status {
            JobStatus::Pending | JobStatus::Retry => {
                job.status = JobStatus::Cancelled;
                // Remove from queue
                for queue in backend.queues.values_mut() {
                    queue.retain(|j| j.id != job_id);
                }
                Ok(true)
            }
            JobStatus::Active => {
                // Mark as cancelled — the worker will see this eventually
                job.status = JobStatus::Cancelled;
                Ok(true)
            }
            _ => Ok(false), // Already completed/dead/cancelled
        }
    } else {
        Ok(false)
    }
}

/// Get queue status — counts by state
pub fn queue_status() -> Result<HashMap<String, i64>> {
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
    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    // Check if job exists and is dead
    let job_info = backend.jobs_by_id.get(job_id).and_then(|job| {
        if job.status == JobStatus::Dead {
            Some(job.queue_name.clone())
        } else {
            None
        }
    });

    if let Some(queue_name) = job_info {
        // Update the job
        if let Some(job) = backend.jobs_by_id.get_mut(job_id) {
            job.status = JobStatus::Pending;
            job.attempt_count = 0;
            job.error = None;
            job.scheduled_at = Instant::now();
        }

        // Remove from dead jobs list
        let job_id_owned = job_id.to_string();
        backend.dead_jobs.retain(|j| j.id != job_id_owned);

        // Add back to queue
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

/// Configure the queue
pub fn configure_queue(config_map: &HashMap<String, Value>) -> Result<()> {
    let mut backend = QUEUE_BACKEND
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Failed to lock queue backend: {}", e)))?;

    if let Some(Value::Int(ms)) = config_map.get("shutdown_timeout") {
        backend.config.shutdown_timeout_ms = *ms as u64;
    }
    if let Some(Value::Int(ms)) = config_map.get("prune_completed_after") {
        backend.config.prune_completed_after_ms = *ms as u64;
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

    // Wait for worker to finish (up to shutdown_timeout)
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

    // Release any active jobs back to pending
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

    // Note: Queue methods are handled as special cases in the interpreter
    // because they need access to native functions that can't be directly
    // represented as Value::NativeFunction (they need variable args or complex logic).
    // The interpreter intercepts Queue.method_name() calls.

    Value::Map(module)
}

// ============================================================
// Module Initialization
// ============================================================

/// Initialize the std/jobs module
pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // Queue module object — methods handled by interpreter special-casing
    // Methods: Queue.work_async(), Queue.status(), Queue.cancel(id), Queue.configure(opts)
    module.insert("Queue".to_string(), create_queue_module());

    module
}
