//! Job DSL module for NTNT
//!
//! Provides background job definitions, enqueue, status, and cancellation.
//!
//! ## KV Key Layout
//!
//! ```text
//! jobs:pending:<priority>:<zero-padded-timestamp>:<id>   →  "" (queue ordering key)
//! jobs:data:<id>                                          →  full job data map (type, queue, payload, status, etc.)
//! jobs:active:<id>                                        →  TTL key for visibility timeout (PR 2b)
//! ```
//!
//! Priority is a 2-digit zero-padded integer (00-99). Lower = higher priority.
//! Named priorities: critical=05, high=25, normal=50 (default), low=85.
//! `list(kv, "jobs:pending:")` returns keys in lexicographic order.
//! Zero-padded timestamps sort correctly for FIFO ordering within a band.
//!
//! Example usage:
//! ```ntnt
//! import { configure_queue, enqueue, job_status, cancel_job } from "std/jobs"
//!
//! configure_queue(map { "store": "sqlite:./jobs.db" })
//! let id = enqueue("SendEmail", map { "to": "alice@example.com" })
//! let status = job_status(id)
//! ```

use crate::ast::{Block, Parameter};
use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use crate::stdlib::concurrent::{
    check_task_limit, finalize_task, is_current_task_cancelled, CURRENT_TASK_CANCELLED, RUNTIME,
};
use crate::stdlib::kv;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ============================================================================
// Job Runtime — global singleton (mirrors ConcurrencyRuntime pattern)
// ============================================================================

/// Configuration for a single worker band.
///
/// Each band scans a contiguous range of priorities and spawns its own thread pool.
#[derive(Debug, Clone)]
pub struct BandConfig {
    /// Band name (e.g., "critical", "high", "normal", "low")
    pub name: String,
    /// Lower bound of priority range (inclusive, 0-99)
    pub min_priority: u8,
    /// Upper bound of priority range (inclusive, 0-99)
    pub max_priority: u8,
    /// Number of worker threads for this band
    pub concurrency: usize,
    /// Milliseconds between polls when queue is empty
    pub poll_interval_ms: u64,
}

impl BandConfig {
    /// Floor key for kv_claim: first key in this band's range.
    pub fn floor_key(&self) -> String {
        format!("jobs:pending:{:02}:", self.min_priority)
    }

    /// Ceiling key for kv_claim: last claimable key in this band (past due only).
    pub fn ceiling_key(&self) -> String {
        format!(
            "jobs:pending:{:02}:{}:~",
            self.max_priority,
            timestamp_key()
        )
    }
}

/// Default worker band configuration (used when work_jobs/work_async gets no "bands" option).
///
/// | Band     | Range | Workers | Poll  |
/// |----------|-------|---------|-------|
/// | critical | 0-9   | 4       | 1s    |
/// | high     | 10-39 | 3       | 2s    |
/// | normal   | 40-69 | 2       | 5s    |
/// | low      | 70-99 | 1       | 20s   |
pub fn default_bands() -> Vec<BandConfig> {
    vec![
        BandConfig {
            name: "critical".to_string(),
            min_priority: 0,
            max_priority: 9,
            concurrency: 4,
            poll_interval_ms: 1_000,
        },
        BandConfig {
            name: "high".to_string(),
            min_priority: 10,
            max_priority: 39,
            concurrency: 3,
            poll_interval_ms: 2_000,
        },
        BandConfig {
            name: "normal".to_string(),
            min_priority: 40,
            max_priority: 69,
            concurrency: 2,
            poll_interval_ms: 5_000,
        },
        BandConfig {
            name: "low".to_string(),
            min_priority: 70,
            max_priority: 99,
            concurrency: 1,
            poll_interval_ms: 20_000,
        },
    ]
}

/// A serializable job option value (Send + Sync safe, no Rc).
#[derive(Debug, Clone)]
pub enum JobOptionValue {
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
}

impl JobOptionValue {
    /// Convert from a Value, returning None for non-serializable types.
    pub fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::Int(i) => Some(JobOptionValue::Int(*i)),
            Value::Float(f) => Some(JobOptionValue::Float(*f)),
            Value::String(s) => Some(JobOptionValue::String(s.clone())),
            Value::Bool(b) => Some(JobOptionValue::Bool(*b)),
            _ => None,
        }
    }

    /// Convert back to a Value.
    pub fn to_value(&self) -> Value {
        match self {
            JobOptionValue::Int(i) => Value::Int(*i),
            JobOptionValue::Float(f) => Value::Float(*f),
            JobOptionValue::String(s) => Value::String(s.clone()),
            JobOptionValue::Bool(b) => Value::Bool(*b),
        }
    }
}

/// A registered job definition from `job Name on queue { perform(...) { ... } }`.
#[derive(Debug, Clone)]
pub struct JobDefinition {
    /// Job name (e.g., "SendEmail")
    pub name: String,
    /// Queue name (e.g., "emails")
    pub queue: String,
    /// Options: retry count, timeout, etc. (Send + Sync safe).
    pub options: HashMap<String, JobOptionValue>,
    /// Parameters for the perform block (e.g., [to, body])
    pub perform_params: Vec<Parameter>,
    /// Body of the perform block — executed by workers in a fresh interpreter
    pub perform_body: Block,
    /// Optional on_failure handler: (params, body)
    pub on_failure: Option<(Vec<Parameter>, Block)>,
}

/// An enqueued job stored in the test queue (Send + Sync safe).
#[derive(Debug, Clone)]
pub struct EnqueuedJob {
    pub id: String,
    pub job_type: String,
    pub queue: String,
    /// Payload serialized as JSON string (avoids Value's Rc/Send issues).
    pub payload_json: String,
}

/// Serializable KV handle info (Send + Sync safe, no Rc).
#[derive(Debug, Clone)]
struct KvHandleInfo {
    backend: String,
    url: String,
    store_id: i64,
}

impl KvHandleInfo {
    /// Reconstruct a Value::Map handle from the stored info.
    fn to_value(&self) -> Value {
        let mut handle = HashMap::new();
        handle.insert("_backend".to_string(), Value::String(self.backend.clone()));
        handle.insert("_url".to_string(), Value::String(self.url.clone()));
        handle.insert("_kv_store_id".to_string(), Value::Int(self.store_id));
        Value::Map(handle)
    }
}

/// Atomic counters for a single worker band.
pub struct BandStats {
    pub completed: std::sync::atomic::AtomicU64,
    pub failed: std::sync::atomic::AtomicU64,
    pub active: std::sync::atomic::AtomicU64,
    pub total_duration_ms: std::sync::atomic::AtomicU64,
}

impl BandStats {
    fn new() -> Self {
        BandStats {
            completed: std::sync::atomic::AtomicU64::new(0),
            failed: std::sync::atomic::AtomicU64::new(0),
            active: std::sync::atomic::AtomicU64::new(0),
            total_duration_ms: std::sync::atomic::AtomicU64::new(0),
        }
    }
}

/// Global job runtime state.
///
/// **Lock discipline (same as ConcurrencyRuntime — NEVER nest):**
/// Acquire one lock at a time, do work, release before acquiring another.
pub struct JobRuntime {
    /// Registered job definitions: name -> definition.
    job_registry: RwLock<HashMap<String, JobDefinition>>,
    /// Lazy-init KV connection info (Send + Sync safe).
    kv_handle_info: Mutex<Option<KvHandleInfo>>,
    /// Default KV store URL (set by configure_queue, default "sqlite:./jobs.db").
    kv_url: Mutex<String>,
    /// Test queue: when Some, enqueue() collects here instead of writing to KV.
    test_queue: Mutex<Option<Vec<EnqueuedJob>>>,
    /// Per-band stats: map from band name to atomic counters.
    pub band_stats: RwLock<HashMap<String, Arc<BandStats>>>,
    /// Running worker task IDs per band (for scale_workers).
    pub band_worker_task_ids: Mutex<HashMap<String, Vec<usize>>>,
    /// Active band configurations (set at work_jobs/work_async startup).
    pub active_bands: Mutex<Vec<BandConfig>>,
}

impl JobRuntime {
    fn new() -> Self {
        JobRuntime {
            job_registry: RwLock::new(HashMap::new()),
            kv_handle_info: Mutex::new(None),
            kv_url: Mutex::new("sqlite:./jobs.db".to_string()),
            test_queue: Mutex::new(None),
            band_stats: RwLock::new(HashMap::new()),
            band_worker_task_ids: Mutex::new(HashMap::new()),
            active_bands: Mutex::new(Vec::new()),
        }
    }

    /// Register a job definition. Errors if a job with the same name already exists.
    pub fn register_job(&self, def: JobDefinition) -> Result<()> {
        let mut registry = self.job_registry.write().map_err(|e| {
            IntentError::runtime_error(format!("Job registry lock poisoned: {}", e))
        })?;
        if registry.contains_key(&def.name) {
            return Err(IntentError::runtime_error(format!(
                "Duplicate job definition: '{}' is already registered",
                def.name
            )));
        }
        registry.insert(def.name.clone(), def);
        Ok(())
    }

    /// Look up a job definition by name.
    pub fn get_job(&self, name: &str) -> Result<Option<JobDefinition>> {
        let registry = self.job_registry.read().map_err(|e| {
            IntentError::runtime_error(format!("Job registry lock poisoned: {}", e))
        })?;
        Ok(registry.get(name).cloned())
    }

    /// Get or lazily initialize the KV handle.
    ///
    /// Uses double-checked locking to avoid holding locks during I/O:
    /// 1. Check kv_handle_info (fast path — already initialized)
    /// 2. Clone URL under kv_url lock, drop it
    /// 3. Open KV connection (slow I/O, no locks held)
    /// 4. Store result under kv_handle_info lock (short critical section)
    pub fn get_or_init_kv(&self) -> Result<Value> {
        // Fast path: already initialized
        {
            let info = self.kv_handle_info.lock().map_err(|e| {
                IntentError::runtime_error(format!("Job KV handle lock poisoned: {}", e))
            })?;
            if let Some(ref h) = *info {
                return Ok(h.to_value());
            }
        } // drop kv_handle_info lock

        // Clone URL under its own lock, then drop
        let url = {
            let url_guard = self.kv_url.lock().map_err(|e| {
                IntentError::runtime_error(format!("Job KV URL lock poisoned: {}", e))
            })?;
            url_guard.clone()
        }; // drop kv_url lock

        // Open KV connection — no locks held during I/O
        let kv_handle_value = kv::open_kv(&url)?;
        let handle_info = extract_kv_handle_info(&kv_handle_value)?;

        // Store result (short critical section)
        {
            let mut info = self.kv_handle_info.lock().map_err(|e| {
                IntentError::runtime_error(format!("Job KV handle lock poisoned: {}", e))
            })?;
            *info = Some(handle_info);
        }

        Ok(kv_handle_value)
    }

    /// Get or create band stats for a given band name.
    pub fn get_or_create_band_stats(&self, band_name: &str) -> Arc<BandStats> {
        // Try read first
        if let Ok(stats_map) = self.band_stats.read() {
            if let Some(stats) = stats_map.get(band_name) {
                return Arc::clone(stats);
            }
        }
        // Create under write lock
        if let Ok(mut stats_map) = self.band_stats.write() {
            stats_map
                .entry(band_name.to_string())
                .or_insert_with(|| Arc::new(BandStats::new()))
                .clone()
        } else {
            // Fallback: create a new untracked stats object
            Arc::new(BandStats::new())
        }
    }

    /// Reset the runtime (for testing).
    #[cfg(test)]
    pub fn reset(&self) {
        if let Ok(mut reg) = self.job_registry.write() {
            reg.clear();
        }
        if let Ok(mut h) = self.kv_handle_info.lock() {
            *h = None;
        }
        if let Ok(mut url) = self.kv_url.lock() {
            *url = "sqlite:./jobs.db".to_string();
        }
        if let Ok(mut tq) = self.test_queue.lock() {
            *tq = None;
        }
        if let Ok(mut s) = self.band_stats.write() {
            s.clear();
        }
        if let Ok(mut ids) = self.band_worker_task_ids.lock() {
            ids.clear();
        }
        if let Ok(mut ab) = self.active_bands.lock() {
            ab.clear();
        }
    }
}

pub static JOB_RUNTIME: LazyLock<JobRuntime> = LazyLock::new(JobRuntime::new);

/// Extract KvHandleInfo from a Value::Map returned by kv::open_kv.
fn extract_kv_handle_info(handle: &Value) -> Result<KvHandleInfo> {
    match handle {
        Value::Map(m) => {
            let backend = match m.get("_backend") {
                Some(Value::String(s)) => s.clone(),
                _ => "unknown".to_string(),
            };
            let url = match m.get("_url") {
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            };
            let store_id = match m.get("_kv_store_id") {
                Some(Value::Int(i)) => *i,
                _ => 0,
            };
            Ok(KvHandleInfo {
                backend,
                url,
                store_id,
            })
        }
        _ => Err(IntentError::runtime_error(
            "Expected KV handle map".to_string(),
        )),
    }
}

// ============================================================================
// Helper: zero-padded timestamp for lexicographic ordering
// ============================================================================

/// Returns a zero-padded nanosecond timestamp string for KV key ordering.
/// Format: 20-digit zero-padded Unix timestamp in nanoseconds.
pub fn timestamp_key() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:020}", now.as_nanos())
}

// ============================================================================
// Worker helpers
// ============================================================================

/// Calculate retry backoff duration in seconds.
///
/// - "exponential" (default): `base * 2^attempt`, capped at 3600s
/// - "linear": `base * attempt`
/// - "constant": `base`
fn calculate_backoff(strategy: &str, attempt: i64, base_secs: i64) -> i64 {
    match strategy {
        "linear" => base_secs * attempt,
        "constant" => base_secs,
        _ => {
            // "exponential" is the default
            let exp = (2i64).saturating_pow(attempt.max(0) as u32);
            base_secs.saturating_mul(exp).min(3600)
        }
    }
}

/// Execute a job's perform block in a fresh interpreter.
///
/// Creates a new `Interpreter`, injects each perform parameter from the payload
/// map, then evaluates the perform body wrapped in `catch_unwind`.
fn execute_job_perform(
    def: &JobDefinition,
    payload: &HashMap<String, Value>,
) -> std::result::Result<Value, String> {
    let mut interp = crate::interpreter::Interpreter::new();

    // Inject perform parameters from the payload map
    for param in &def.perform_params {
        let val = payload.get(&param.name).cloned().unwrap_or(Value::Unit);
        interp.define_global(param.name.clone(), val);
    }

    let body = def.perform_body.clone();
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| interp.eval_block(&body)));

    match result {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{}", e)),
        Err(panic_info) => {
            let msg = if let Some(s) = panic_info.downcast_ref::<String>() {
                s.clone()
            } else if let Some(s) = panic_info.downcast_ref::<&str>() {
                s.to_string()
            } else {
                "job perform panicked".to_string()
            };
            Err(msg)
        }
    }
}

/// Execute a job's on_failure handler, if present.
///
/// Binds `error` (String) and `attempt` (Int) to the on_failure params
/// (first and second respectively, falling back to those names if the param
/// list is shorter).  Errors are silently discarded — on_failure is
/// fire-and-forget.
fn execute_on_failure(def: &JobDefinition, error: &str, attempt: i64) {
    let Some((params, body)) = def.on_failure.as_ref() else {
        return;
    };

    let mut interp = crate::interpreter::Interpreter::new();

    // Bind by position: first param → error string, second param → attempt int.
    // Also bind the conventional names so handlers can use either.
    interp.define_global("error".to_string(), Value::String(error.to_string()));
    interp.define_global("attempt".to_string(), Value::Int(attempt));

    if let Some(p) = params.first() {
        interp.define_global(p.name.clone(), Value::String(error.to_string()));
    }
    if let Some(p) = params.get(1) {
        interp.define_global(p.name.clone(), Value::Int(attempt));
    }

    let body = body.clone();
    // Ignore result — on_failure is best-effort
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| interp.eval_block(&body)));
}

/// Core implementation shared by `enqueue()`, `enqueue_at()`, and `enqueue_in()`.
///
/// `pending_ts` is the timestamp portion used for the pending key (controls
/// ordering / scheduling).  For immediate jobs pass `timestamp_key()`.  For
/// scheduled jobs pass the future nanosecond timestamp as a 20-digit string.
fn enqueue_internal(
    job_name: &str,
    payload: Value,
    pending_ts: &str,
    scheduled_at: Option<&str>,
) -> Result<Value> {
    // Look up job in registry
    let job_def = JOB_RUNTIME.get_job(job_name)?;
    let job_def = match job_def {
        Some(def) => def,
        None => {
            return Err(IntentError::runtime_error(format!(
                "Job '{}' is not registered. Define it with: job {} on <queue> {{ perform(...) {{ ... }} }}",
                job_name, job_name
            )));
        }
    };

    // Resolve numeric priority from job options
    // Named: critical=5, high=25, normal=50 (default), low=85
    // Numeric: 0-99 inclusive
    let priority: u8 = match job_def.options.get("priority") {
        Some(JobOptionValue::String(s)) => match s.as_str() {
            "critical" => 5,
            "high" => 25,
            "normal" => 50,
            "low" => 85,
            other => {
                return Err(IntentError::runtime_error(format!(
                    "Unknown priority '{}'. Use: critical, high, normal, low (or an integer 0-99)",
                    other
                )));
            }
        },
        Some(JobOptionValue::Int(p)) if *p >= 0 && *p <= 99 => *p as u8,
        Some(JobOptionValue::Int(p)) => {
            return Err(IntentError::runtime_error(format!(
                "Priority must be 0-99, got {}",
                p
            )));
        }
        _ => 50, // default: "normal"
    };

    // Dedup: if job has `unique` option, compute deterministic hash and check for existing job.
    // Hash determinism: value_to_json_public converts HashMap to serde_json::Map backed by
    // BTreeMap (no `preserve_order` feature), so keys are sorted automatically.
    // Race condition note: dedup check (kv_get) and write (kv_set) are not atomic.
    // Best-effort dedup under concurrent enqueues, same as Sidekiq's unique_for.
    let unique_secs = match job_def.options.get("unique") {
        Some(JobOptionValue::Int(n)) if *n > 0 => Some(*n),
        _ => None,
    };
    let dedup_key = if unique_secs.is_some() {
        let pjson = serde_json::to_string(&crate::stdlib::kv::value_to_json_public(&payload))
            .map_err(|e| {
                IntentError::runtime_error(format!(
                    "Failed to serialize payload for dedup hash: {}",
                    e
                ))
            })?;
        let mut hasher = Sha256::new();
        hasher.update(format!("{}:{}", job_name, pjson).as_bytes());
        let full_hash = format!("{:x}", hasher.finalize());
        Some(format!("jobs:unique:{}:{}", job_name, &full_hash[..32]))
    } else {
        None
    };

    // Check dedup before generating ID (skip in test mode)
    let in_test_mode = JOB_RUNTIME
        .test_queue
        .lock()
        .map(|tq| tq.is_some())
        .unwrap_or(false);
    if let (Some(ref dk), false) = (&dedup_key, in_test_mode) {
        let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
        if let Value::String(existing_id) = kv::kv_get(&kv_handle, dk)? {
            let data_key = format!("jobs:data:{}", existing_id);
            let is_terminal = match kv::kv_get(&kv_handle, &data_key) {
                Ok(Value::Map(data)) => match data.get("status") {
                    Some(Value::String(s)) => {
                        matches!(s.as_str(), "cancelled" | "dead" | "expired" | "failed")
                    }
                    _ => true,
                },
                Ok(Value::Unit) => true, // job deleted — stale reference
                Ok(_) => true,           // unexpected data shape — treat as stale
                Err(_) => {
                    // KV error — fail-closed: assume job is still live to prevent duplicates.
                    // A transient timeout should not cause re-enqueue.
                    emit_job_event(
                        "job.dedup_warning",
                        &[
                            ("job_id", Value::String(existing_id.clone())),
                            (
                                "reason",
                                Value::String(
                                    "KV error checking existing job status; assuming live"
                                        .to_string(),
                                ),
                            ),
                        ],
                    );
                    false
                }
            };
            if !is_terminal {
                return Ok(Value::ok(Value::String(existing_id)));
            }
            let _ = kv::kv_del(&kv_handle, dk);
        }
    }

    let job_id = Uuid::new_v4().to_string();

    // Check test mode
    {
        let mut test_queue = JOB_RUNTIME
            .test_queue
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;
        if let Some(ref mut queue) = *test_queue {
            let payload_json = serde_json::to_string(&crate::stdlib::kv::value_to_json_public(
                &payload,
            ))
            .map_err(|e| {
                IntentError::runtime_error(format!("Failed to serialize job payload: {}", e))
            })?;
            queue.push(EnqueuedJob {
                id: job_id.clone(),
                job_type: job_name.to_string(),
                queue: job_def.queue.clone(),
                payload_json,
            });
            return Ok(Value::ok(Value::String(job_id)));
        }
    }

    // Get KV handle (lazy init)
    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;

    // Build job data map
    let mut job_data = HashMap::new();
    job_data.insert("id".to_string(), Value::String(job_id.clone()));
    job_data.insert("type".to_string(), Value::String(job_name.to_string()));
    job_data.insert("queue".to_string(), Value::String(job_def.queue.clone()));
    job_data.insert("payload".to_string(), payload);
    job_data.insert("status".to_string(), Value::String("pending".to_string()));
    job_data.insert("attempts".to_string(), Value::Int(0));
    job_data.insert("created_at".to_string(), Value::String(timestamp_key()));

    // Copy job options (retry, timeout, etc.)
    for (k, v) in &job_def.options {
        job_data.insert(k.clone(), v.to_value());
    }

    // Add priority and band name to job data
    job_data.insert("priority".to_string(), Value::Int(priority as i64));
    let band_name = if priority <= 9 {
        "critical"
    } else if priority <= 39 {
        "high"
    } else if priority <= 69 {
        "normal"
    } else {
        "low"
    };
    job_data.insert("band".to_string(), Value::String(band_name.to_string()));

    // Store scheduled_at if provided, and set status to "scheduled" (not "pending")
    // so the CLI can distinguish between ready-to-run and future-dated jobs.
    // The worker claims by scanning jobs:pending:* keys — the status field is for display only.
    if let Some(sat) = scheduled_at {
        job_data.insert("scheduled_at".to_string(), Value::String(sat.to_string()));
        job_data.insert("status".to_string(), Value::String("scheduled".to_string()));
    }

    if let Some(ref dk) = dedup_key {
        job_data.insert("dedup_key".to_string(), Value::String(dk.clone()));
    }

    // Build pending key with priority prefix for band-aware worker claim
    // Format: jobs:pending:<priority_2digit>:<timestamp>:<id>
    let pending_key = format!("jobs:pending:{:02}:{}:{}", priority, pending_ts, job_id);
    job_data.insert(
        "pending_key".to_string(),
        Value::String(pending_key.clone()),
    );

    // Write to KV: jobs:data:<id>
    let data_key = format!("jobs:data:{}", job_id);
    kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None)?;

    // Write queue ordering key: jobs:pending:<timestamp>:<id>
    kv::kv_set(
        &kv_handle,
        &pending_key,
        &Value::String(job_id.clone()),
        None,
    )?;

    if let (Some(ref dk), Some(ttl)) = (&dedup_key, unique_secs) {
        if let Err(e) = kv::kv_set(&kv_handle, dk, &Value::String(job_id.clone()), Some(ttl)) {
            emit_job_event(
                "job.dedup_warning",
                &[
                    ("job_id", Value::String(job_id.clone())),
                    ("dedup_key", Value::String(dk.clone())),
                    (
                        "error",
                        Value::String(format!("Failed to write dedup key: {}", e)),
                    ),
                ],
            );
        }
    }

    emit_job_event(
        "job.enqueued",
        &[
            ("job_id", Value::String(job_id.clone())),
            ("type", Value::String(job_name.to_string())),
            ("queue", Value::String(job_def.queue.clone())),
        ],
    );

    Ok(Value::ok(Value::String(job_id)))
}

/// Worker loop — runs until cooperative cancellation is signalled.
///
/// `kv_info`: serializable KV handle info (reconstructed into a `Value` on entry).
/// `poll_interval_ms`: milliseconds to sleep between empty-queue polls.
/// `queues`: if Some, only process jobs whose queue field matches one of
///           these names; if None, process all queues.
fn worker_loop(kv_info: KvHandleInfo, poll_interval_ms: u64, queues: Option<Vec<String>>) {
    let kv_handle = kv_info.to_value();
    let poll_duration = std::time::Duration::from_millis(poll_interval_ms);

    loop {
        if is_current_task_cancelled() {
            break;
        }

        // Attempt to claim a pending job.
        // Future-scheduled jobs are filtered at the claim level by band workers (via ceiling).
        // For the legacy no-band worker, the defense-in-depth scheduled_at check
        // in the body handles future jobs by re-enqueuing them.
        let claimed = match kv::kv_claim(&kv_handle, "jobs:pending:", None, None) {
            Ok(Some((_pending_key, value))) => value,
            Ok(None) => {
                // Queue empty — sleep and try again
                std::thread::sleep(poll_duration);
                continue;
            }
            Err(_) => {
                std::thread::sleep(poll_duration);
                continue;
            }
        };

        // The claimed value is the job_id string
        let job_id = match &claimed {
            Value::String(s) => s.clone(),
            _ => {
                std::thread::sleep(poll_duration);
                continue;
            }
        };

        // Read full job data
        let data_key = format!("jobs:data:{}", job_id);
        let job_data_val = match kv::kv_get(&kv_handle, &data_key) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let mut job_data = match job_data_val {
            Value::Map(m) => m,
            _ => continue,
        };

        // Skip cancelled jobs (do not re-enqueue)
        let status = match job_data.get("status") {
            Some(Value::String(s)) => s.clone(),
            _ => "pending".to_string(),
        };
        if status == "cancelled" {
            continue;
        }

        // Queue filtering: skip jobs whose queue doesn't match our filter
        if let Some(ref filter) = queues {
            let job_queue = match job_data.get("queue") {
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            };
            if !filter.contains(&job_queue) {
                // Re-enqueue: restore pending key so another worker (or next poll)
                // can pick it up.  Use the original pending_key so ordering is
                // preserved.
                if let Some(Value::String(pk)) = job_data.get("pending_key") {
                    let pk = pk.clone();
                    let _ = kv::kv_set(&kv_handle, &pk, &Value::String(job_id.clone()), None);
                }
                std::thread::sleep(poll_duration);
                continue;
            }
        }

        // Defense-in-depth: the ceiling filter should prevent future jobs from being
        // claimed, but verify at runtime in case the invariant is violated (manual KV
        // edit, clock skew, enqueue bug). Re-enqueue and skip rather than execute early.
        if let Some(Value::String(scheduled_at)) = job_data.get("scheduled_at") {
            let now_ts = timestamp_key();
            if scheduled_at.as_str() > now_ts.as_str() {
                emit_job_event(
                    "job.skipped",
                    &[
                        ("job_id", Value::String(job_id.clone())),
                        (
                            "reason",
                            Value::String(format!(
                                "future scheduled_at {} (now {}); ceiling filter bypassed",
                                scheduled_at, now_ts
                            )),
                        ),
                    ],
                );
                // Re-enqueue: use stored pending_key, or reconstruct from scheduled_at
                let pk = match job_data.get("pending_key") {
                    Some(Value::String(s)) => s.clone(),
                    _ => format!("jobs:pending:{}:{}", scheduled_at, job_id),
                };
                let _ = kv::kv_set(&kv_handle, &pk, &Value::String(job_id.clone()), None);
                std::thread::sleep(poll_duration);
                continue;
            }
        }

        // Expiration check: if job has `expires` option, check if it has exceeded
        // its maximum wait time (time between creation and now). If so, mark as expired
        // and clean up dedup key.
        if let Some(Value::Int(expires_secs)) = job_data.get("expires") {
            let expires_after = *expires_secs;
            if expires_after > 0 {
                let now_nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                match job_data.get("created_at") {
                    Some(Value::String(created_str)) => {
                        match created_str.parse::<u128>() {
                            Ok(created_nanos) => {
                                let age_secs =
                                    (now_nanos.saturating_sub(created_nanos)) / 1_000_000_000;
                                if age_secs >= expires_after as u128 {
                                    // Extract type before mutations
                                    let expired_job_type = match job_data.get("type") {
                                        Some(Value::String(s)) => s.clone(),
                                        _ => String::new(),
                                    };
                                    // Clean up dedup key before expiring
                                    if let Some(Value::String(dk)) = job_data.get("dedup_key") {
                                        let _ = kv::kv_del(&kv_handle, dk);
                                    }
                                    job_data.insert(
                                        "status".to_string(),
                                        Value::String("expired".to_string()),
                                    );
                                    job_data.insert(
                                        "expired_at".to_string(),
                                        Value::String(timestamp_key()),
                                    );
                                    let _ = kv::kv_set(
                                        &kv_handle,
                                        &data_key,
                                        &Value::Map(job_data.clone()),
                                        None,
                                    );
                                    emit_job_event(
                                        "job.expired",
                                        &[
                                            ("job_id", Value::String(job_id.clone())),
                                            ("type", Value::String(expired_job_type)),
                                            ("age_secs", Value::Int(age_secs as i64)),
                                        ],
                                    );
                                    continue;
                                }
                            }
                            Err(_) => {
                                emit_job_event(
                                    "job.expire_warning",
                                    &[
                                        ("job_id", Value::String(job_id.clone())),
                                        (
                                            "error",
                                            Value::String(format!(
                                                "Could not parse created_at '{}' for expiration check",
                                                created_str
                                            )),
                                        ),
                                    ],
                                );
                            }
                        }
                    }
                    _ => {
                        emit_job_event(
                            "job.expire_warning",
                            &[
                                ("job_id", Value::String(job_id.clone())),
                                (
                                    "error",
                                    Value::String(
                                        "Missing created_at field for expiration check".to_string(),
                                    ),
                                ),
                            ],
                        );
                    }
                }
            }
        }

        // Write visibility timeout key: jobs:active:<id> with TTL 300s
        let active_key = format!("jobs:active:{}", job_id);
        let _ = kv::kv_set(
            &kv_handle,
            &active_key,
            &Value::String(job_id.clone()),
            Some(300),
        );

        // Mark status as "active"
        job_data.insert("status".to_string(), Value::String("active".to_string()));
        if kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data.clone()), None).is_err() {
            continue;
        }

        // Look up the job definition
        let job_type = match job_data.get("type") {
            Some(Value::String(s)) => s.clone(),
            _ => continue,
        };

        let def = match JOB_RUNTIME.get_job(&job_type) {
            Ok(Some(d)) => d,
            _ => {
                // Unknown job type — mark as dead
                job_data.insert("status".to_string(), Value::String("dead".to_string()));
                job_data.insert(
                    "error".to_string(),
                    Value::String(format!("No job definition found for '{}'", job_type)),
                );
                job_data.insert("dead_at".to_string(), Value::String(timestamp_key()));
                let _ = kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None);
                let _ = kv::kv_del(&kv_handle, &active_key);
                continue;
            }
        };

        // Extract payload map
        let payload = match job_data.get("payload") {
            Some(Value::Map(m)) => m.clone(),
            _ => HashMap::new(),
        };

        // Extract attempt count
        let attempts = match job_data.get("attempts") {
            Some(Value::Int(n)) => *n,
            _ => 0,
        };

        // Emit job.started event
        emit_job_event(
            "job.started",
            &[
                ("job_id", Value::String(job_id.clone())),
                ("type", Value::String(job_type.clone())),
                (
                    "queue",
                    Value::String(
                        job_data
                            .get("queue")
                            .and_then(|v| {
                                if let Value::String(s) = v {
                                    Some(s.clone())
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default(),
                    ),
                ),
                ("attempt", Value::Int(attempts + 1)),
            ],
        );

        // Record start time for timeout detection
        let start = std::time::Instant::now();

        let exec_result = execute_job_perform(&def, &payload);

        // Check job timeout (non-negative values only; >= for boundary correctness)
        let timed_out = if let Some(Value::Int(timeout_secs)) = job_data.get("timeout") {
            let timeout = (*timeout_secs).max(0) as u64;
            timeout > 0 && start.elapsed().as_secs() >= timeout
        } else {
            false
        };

        // Timeout always wins — whether the job succeeded or errored, a timeout
        // is the root cause and should be the reported failure reason.
        let exec_result = if timed_out {
            Err(format!(
                "Job timed out after {}s",
                start.elapsed().as_secs()
            ))
        } else {
            exec_result
        };

        // Check if job was force-cancelled while we were executing —
        // re-read status from KV and discard our result if cancelled.
        if let Ok(Value::Map(fresh_data)) = kv::kv_get(&kv_handle, &data_key) {
            if let Some(Value::String(current_status)) = fresh_data.get("status") {
                if current_status == "cancelled" {
                    emit_job_event(
                        "job.cancelled",
                        &[
                            ("job_id", Value::String(job_id.clone())),
                            ("type", Value::String(job_type.clone())),
                            (
                                "reason",
                                Value::String("force-cancelled during execution".to_string()),
                            ),
                        ],
                    );
                    continue;
                }
            }
        }

        // Execute result handling
        match exec_result {
            Ok(_) => {
                // Success
                job_data.insert("status".to_string(), Value::String("completed".to_string()));
                job_data.insert("completed_at".to_string(), Value::String(timestamp_key()));
                let _ = kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None);
                let _ = kv::kv_del(&kv_handle, &active_key);
                emit_job_event(
                    "job.completed",
                    &[
                        ("job_id", Value::String(job_id.clone())),
                        ("type", Value::String(job_type.clone())),
                    ],
                );
            }
            Err(err_msg) => {
                let new_attempts = attempts + 1;

                // Determine retry limit (default 3)
                let retry_limit = match job_data.get("retry") {
                    Some(Value::Int(n)) => *n,
                    _ => 3,
                };

                // Record failed_at on every failure
                let fail_ts = timestamp_key();
                job_data.insert("failed_at".to_string(), Value::String(fail_ts));
                job_data.insert("error".to_string(), Value::String(err_msg.clone()));
                job_data.insert("attempts".to_string(), Value::Int(new_attempts));

                // Call on_failure handler (fire-and-forget)
                execute_on_failure(&def, &err_msg, new_attempts);

                if new_attempts < retry_limit {
                    // Re-enqueue with backoff
                    let backoff_strategy = match job_data.get("backoff") {
                        Some(Value::String(s)) => s.clone(),
                        _ => "exponential".to_string(),
                    };
                    let base_secs = match job_data.get("backoff_base") {
                        Some(Value::Int(n)) => *n,
                        _ => 5,
                    };
                    let delay_secs = calculate_backoff(&backoff_strategy, new_attempts, base_secs);

                    // Build new pending key with future timestamp
                    let future_nanos = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                        + (delay_secs as u128) * 1_000_000_000;
                    let future_ts = format!("{:020}", future_nanos);
                    // Preserve original priority in retry pending key
                    let job_priority = match job_data.get("priority") {
                        Some(Value::Int(p)) => *p as u8,
                        _ => 50u8,
                    };
                    let new_pending_key =
                        format!("jobs:pending:{:02}:{}:{}", job_priority, future_ts, job_id);

                    // Use "retrying" status so CLI can distinguish retry-waiting
                    // jobs from permanently failed or ready-to-run ones. "retrying"
                    // means the runtime will auto-retry after backoff — clearing these
                    // would be data loss. The pending key in KV still drives worker
                    // claiming — status is for display/filtering only.
                    job_data.insert("status".to_string(), Value::String("retrying".to_string()));
                    job_data.insert(
                        "pending_key".to_string(),
                        Value::String(new_pending_key.clone()),
                    );
                    job_data.insert("scheduled_at".to_string(), Value::String(future_ts.clone()));

                    let _ = kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None);
                    let _ = kv::kv_set(
                        &kv_handle,
                        &new_pending_key,
                        &Value::String(job_id.clone()),
                        None,
                    );
                    emit_job_event(
                        "job.failed",
                        &[
                            ("job_id", Value::String(job_id.clone())),
                            ("type", Value::String(job_type.clone())),
                            ("error", Value::String(err_msg.clone())),
                            ("attempt", Value::Int(new_attempts)),
                            ("will_retry", Value::Bool(true)),
                        ],
                    );
                } else {
                    // Exhausted retries — mark as dead
                    // Clean up dedup key on death
                    if let Some(Value::String(dk)) = job_data.get("dedup_key") {
                        let _ = kv::kv_del(&kv_handle, dk);
                    }
                    job_data.insert("status".to_string(), Value::String("dead".to_string()));
                    job_data.insert("dead_at".to_string(), Value::String(timestamp_key()));
                    let _ = kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None);
                    emit_job_event(
                        "job.dead",
                        &[
                            ("job_id", Value::String(job_id.clone())),
                            ("type", Value::String(job_type.clone())),
                            ("error", Value::String(err_msg.clone())),
                            ("attempt", Value::Int(new_attempts)),
                        ],
                    );
                }

                let _ = kv::kv_del(&kv_handle, &active_key);
            }
        }
    }
}

/// Parse shared options for work_async() and work_jobs().
///
/// Returns (poll_interval_ms, concurrency, queues).
fn parse_work_opts(args: &[Value]) -> Result<(u64, usize, Option<Vec<String>>)> {
    let mut poll_interval: u64 = 1000;
    let mut concurrency: usize = 1;
    let mut queues: Option<Vec<String>> = None;

    if let Some(opts_val) = args.first() {
        let opts = match opts_val {
            Value::Map(m) => m,
            Value::Unit => return Ok((poll_interval, concurrency, queues)),
            _ => {
                return Err(IntentError::type_error(
                    "work options must be a map".to_string(),
                ))
            }
        };

        if let Some(v) = opts.get("poll_interval") {
            match v {
                Value::Int(n) => poll_interval = (*n).max(1) as u64,
                _ => {
                    return Err(IntentError::type_error(
                        "poll_interval must be an integer (milliseconds)".to_string(),
                    ))
                }
            }
        }

        if let Some(v) = opts.get("concurrency") {
            match v {
                Value::Int(n) => concurrency = (*n).max(1) as usize,
                _ => {
                    return Err(IntentError::type_error(
                        "concurrency must be an integer".to_string(),
                    ))
                }
            }
        }

        if let Some(v) = opts.get("queues") {
            match v {
                Value::Array(arr) => {
                    let mut names = Vec::new();
                    for item in arr {
                        match item {
                            Value::String(s) => names.push(s.clone()),
                            _ => {
                                return Err(IntentError::type_error(
                                    "queues must be an array of strings".to_string(),
                                ))
                            }
                        }
                    }
                    queues = Some(names);
                }
                _ => {
                    return Err(IntentError::type_error(
                        "queues must be an array of strings".to_string(),
                    ))
                }
            }
        }
    }

    Ok((poll_interval, concurrency, queues))
}

/// Spawn a single worker task registered with the ConcurrencyRuntime.
///
/// Returns a `Value::TaskHandle` so callers can cancel the worker via
/// `cancel_task()`.
fn spawn_worker_task(
    kv_handle: Value,
    poll_interval_ms: u64,
    queues: Option<Vec<String>>,
) -> Result<Value> {
    // Extract serializable KvHandleInfo — Value is not Send due to Rc internals.
    let kv_info = extract_kv_handle_info(&kv_handle)?;

    RUNTIME.try_reap_expired_tasks();
    check_task_limit()?;

    let cancelled = Arc::new(AtomicBool::new(false));
    let task_id = RUNTIME.register_task(Arc::clone(&cancelled))?;
    RUNTIME.active_tasks.fetch_add(1, AtomicOrdering::Release);
    // Safe: task_id was just returned by register_task(), so it must be in the registry
    let arcs = RUNTIME
        .get_task_arcs(task_id)?
        .expect("task just registered must exist");

    std::thread::spawn(move || {
        CURRENT_TASK_CANCELLED.with(|cell| {
            *cell.borrow_mut() = Some(cancelled);
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worker_loop(kv_info, poll_interval_ms, queues);
            Ok(Value::Unit)
        }));
        finalize_task(result, &arcs.inner, &arcs.completed_notify);
    });

    Ok(Value::TaskHandle(task_id))
}

// ============================================================================
// Event emission helper
// ============================================================================

/// Emit a structured job event to stderr as JSON.
fn emit_job_event(event: &str, fields: &[(&str, Value)]) {
    let mut map = HashMap::new();
    map.insert("event".to_string(), Value::String(event.to_string()));
    map.insert("timestamp".to_string(), Value::String(timestamp_key()));
    for (k, v) in fields {
        map.insert(k.to_string(), v.clone());
    }
    // Write JSON to stderr — lock for atomic line output across concurrent workers.
    // serde_json::Map uses BTreeMap (no preserve_order feature), so keys are always
    // alphabetically sorted in the output regardless of HashMap insertion order.
    if let Ok(json) =
        serde_json::to_string(&crate::stdlib::kv::value_to_json_public(&Value::Map(map)))
    {
        let stderr = std::io::stderr();
        let mut locked = stderr.lock();
        let _ = std::io::Write::write_all(&mut locked, format!("{}\n", json).as_bytes());
    }
}

// ============================================================================
// Public API — shared logic for CLI and stdlib bindings
// ============================================================================

/// Result of a retry_job operation
pub enum RetryResult {
    /// Job was re-queued. Contains the queue name.
    Requeued(String),
    /// Job status doesn't allow retry (not retrying/failed/dead).
    NotRetryable(String),
}

/// Result of a cancel_job operation
pub enum CancelResult {
    /// Job was cancelled. Bool indicates if it was an active (force) cancel.
    Cancelled { was_active: bool },
    /// Job status doesn't allow cancellation.
    NotCancellable(String),
}

/// Options for listing jobs
pub struct ListJobsOpts {
    pub status: Option<String>,
    pub queue: Option<String>,
    pub limit: usize,
}

/// Options for deleting jobs
pub struct DeleteJobsOpts {
    pub status: String,
    pub older_than_secs: Option<u64>,
}

/// Job status counts for the status summary
pub struct JobStatusCounts {
    pub pending: u64,
    pub scheduled: u64,
    pub active: u64,
    pub completed: u64,
    pub retrying: u64,
    pub dead: u64,
    pub cancelled: u64,
    pub expired: u64,
    pub total: u64,
}

/// Retry a failed/retrying/dead job by ID. Resets attempts, clears errors, re-enqueues.
pub fn retry_job_by_id(job_id: &str) -> Result<RetryResult> {
    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    let data_key = format!("jobs:data:{}", job_id);

    let current = kv::kv_get(&kv_handle, &data_key)?;
    let mut job_data = match current {
        Value::Map(m) => m,
        Value::Unit => {
            return Err(IntentError::runtime_error(format!(
                "Job '{}' not found",
                job_id
            )));
        }
        _ => {
            return Err(IntentError::runtime_error(format!(
                "Corrupt job data for '{}'",
                job_id
            )));
        }
    };

    let status = match job_data.get("status") {
        Some(Value::String(s)) => s.clone(),
        _ => "unknown".to_string(),
    };

    // Accept "failed" for backward compat with pre-v0.4.6 job data
    if status != "retrying" && status != "failed" && status != "dead" {
        return Ok(RetryResult::NotRetryable(status));
    }

    let queue = match job_data.get("queue") {
        Some(Value::String(s)) => s.clone(),
        _ => "default".to_string(),
    };

    // Delete old pending key to prevent double execution
    if let Some(Value::String(old_pk)) = job_data.get("pending_key") {
        let old_pk = old_pk.clone();
        let _ = kv::kv_del(&kv_handle, &old_pk);
    }

    let pending_ts = timestamp_key();
    let job_priority = match job_data.get("priority") {
        Some(Value::Int(p)) => *p as u8,
        _ => 50u8,
    };
    let new_pending_key = format!("jobs:pending:{:02}:{}:{}", job_priority, pending_ts, job_id);

    let retry_manually = match job_data.get("retry_manually") {
        Some(Value::Int(n)) => n + 1,
        _ => 1,
    };

    job_data.insert("status".to_string(), Value::String("pending".to_string()));
    job_data.insert("attempts".to_string(), Value::Int(0));
    job_data.insert(
        "pending_key".to_string(),
        Value::String(new_pending_key.clone()),
    );
    job_data.insert("retry_manually".to_string(), Value::Int(retry_manually));
    job_data.remove("error");
    job_data.remove("failed_at");
    job_data.remove("dead_at");

    kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None)?;
    kv::kv_set(
        &kv_handle,
        &new_pending_key,
        &Value::String(job_id.to_string()),
        None,
    )?;

    Ok(RetryResult::Requeued(queue))
}

/// Cancel a job by ID. Returns NotCancellable if status doesn't allow it.
pub fn cancel_job_by_id(job_id: &str, force: bool) -> Result<CancelResult> {
    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    let data_key = format!("jobs:data:{}", job_id);

    let current = kv::kv_get(&kv_handle, &data_key)?;
    let mut job_data = match current {
        Value::Map(m) => m,
        Value::Unit => {
            return Err(IntentError::runtime_error(format!(
                "Job '{}' not found",
                job_id
            )));
        }
        _ => {
            return Err(IntentError::runtime_error(format!(
                "Corrupt job data for '{}'",
                job_id
            )));
        }
    };

    let status = match job_data.get("status") {
        Some(Value::String(s)) => s.clone(),
        _ => "unknown".to_string(),
    };

    // Without force: only non-active, non-terminal jobs
    if !force
        && status != "pending"
        && status != "scheduled"
        && status != "retrying"
        && status != "failed"
    {
        return Ok(CancelResult::NotCancellable(status));
    }

    // With force: reject terminal states
    if force
        && (status == "completed"
            || status == "dead"
            || status == "cancelled"
            || status == "expired")
    {
        return Ok(CancelResult::NotCancellable(status));
    }

    let was_active = status == "active";

    // Remove pending key
    if let Some(Value::String(pk)) = job_data.get("pending_key") {
        let pk = pk.clone();
        let _ = kv::kv_del(&kv_handle, &pk);
    }

    // If force-cancelling active job, remove visibility timeout key
    if was_active {
        let _ = kv::kv_del(&kv_handle, &format!("jobs:active:{}", job_id));
    }

    // Clean up dedup key on cancellation
    if let Some(Value::String(dk)) = job_data.get("dedup_key") {
        let _ = kv::kv_del(&kv_handle, dk);
    }

    job_data.insert("status".to_string(), Value::String("cancelled".to_string()));
    job_data.insert("cancelled_at".to_string(), Value::String(timestamp_key()));
    kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None)?;

    Ok(CancelResult::Cancelled { was_active })
}

/// List jobs with optional filters, sorted newest-first.
pub fn list_jobs_filtered(opts: ListJobsOpts) -> Result<Vec<HashMap<String, Value>>> {
    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    let data_keys = kv::kv_list(&kv_handle, Some("jobs:data:"))?;

    let mut results = Vec::new();
    for key in &data_keys {
        if let Ok(Value::Map(data)) = kv::kv_get(&kv_handle, key) {
            if let Some(ref sf) = opts.status {
                match data.get("status") {
                    Some(Value::String(s)) if s == sf.as_str() => {}
                    _ => continue,
                }
            }
            if let Some(ref qf) = opts.queue {
                match data.get("queue") {
                    Some(Value::String(q)) if q == qf.as_str() => {}
                    _ => continue,
                }
            }
            results.push(data);
        }
    }

    // Sort newest-first by created_at
    results.sort_by(|a, b| {
        let ts = |m: &HashMap<String, Value>| -> String {
            match m.get("created_at") {
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            }
        };
        ts(b).cmp(&ts(a))
    });
    results.truncate(opts.limit);

    Ok(results)
}

/// Bulk delete jobs by status. Returns count of deleted jobs.
pub fn delete_jobs_filtered(opts: DeleteJobsOpts) -> Result<i64> {
    if opts.status == "active" {
        return Err(IntentError::runtime_error(
            "Cannot delete active jobs — workers are currently processing them. Stop workers first."
                .to_string(),
        ));
    }

    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    let data_keys = kv::kv_list(&kv_handle, Some("jobs:data:"))?;

    let now_nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let mut deleted = 0i64;
    for key in &data_keys {
        if let Ok(Value::Map(data)) = kv::kv_get(&kv_handle, key) {
            match data.get("status") {
                Some(Value::String(s)) if s == &opts.status => {}
                _ => continue,
            }

            if let Some(threshold_secs) = opts.older_than_secs {
                let created_nanos: u128 = match data.get("created_at") {
                    Some(Value::String(s)) => s.parse().unwrap_or(0),
                    _ => 0,
                };
                let age_secs = (now_nanos.saturating_sub(created_nanos)) / 1_000_000_000;
                if age_secs < threshold_secs as u128 {
                    continue;
                }
            }

            // Clean up associated keys
            if let Some(Value::String(pk)) = data.get("pending_key") {
                let _ = kv::kv_del(&kv_handle, pk);
            }
            if let Some(Value::String(id)) = data.get("id") {
                let _ = kv::kv_del(&kv_handle, &format!("jobs:active:{}", id));
            }
            if let Some(Value::String(dk)) = data.get("dedup_key") {
                let _ = kv::kv_del(&kv_handle, dk);
            }
            let _ = kv::kv_del(&kv_handle, key);
            deleted += 1;
        }
    }

    Ok(deleted)
}

/// Get job status counts across all jobs.
pub fn job_status_counts() -> Result<JobStatusCounts> {
    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    let data_keys = kv::kv_list(&kv_handle, Some("jobs:data:"))?;

    let mut counts = JobStatusCounts {
        pending: 0,
        scheduled: 0,
        active: 0,
        completed: 0,
        retrying: 0,
        dead: 0,
        cancelled: 0,
        expired: 0,
        total: data_keys.len() as u64,
    };

    for key in &data_keys {
        if let Ok(Value::Map(data)) = kv::kv_get(&kv_handle, key) {
            if let Some(Value::String(s)) = data.get("status") {
                match s.as_str() {
                    "pending" => counts.pending += 1,
                    "scheduled" => counts.scheduled += 1,
                    "active" => counts.active += 1,
                    "completed" => counts.completed += 1,
                    "retrying" => counts.retrying += 1,
                    "dead" => counts.dead += 1,
                    "cancelled" => counts.cancelled += 1,
                    "expired" => counts.expired += 1,
                    _ => {}
                }
            }
        }
    }

    Ok(counts)
}

// ============================================================================
// Module Export
// ============================================================================

pub fn init() -> HashMap<String, Value> {
    let mut module = HashMap::new();

    // @ntnt configure_queue
    // @module std/jobs
    // @module_description Background job queue with persistent storage
    // @signature configure_queue(opts: Map) -> Result<Unit, String>
    // Configure the job queue storage backend.
    //
    // Pass a map with a "store" key to set the KV backend for job storage.
    // If never called, enqueue() auto-initializes with "sqlite:./jobs.db".
    // @param opts Configuration map with optional "store" key (e.g., "redis://localhost:6379" or "sqlite:./jobs.db")
    // @returns Result indicating success or error
    // @example configure_queue(map { "store": "sqlite:./jobs.db" }) ~ "Use SQLite for job storage"
    // @example configure_queue(map { "store": "redis://localhost:6379" }) ~ "Use Redis for job storage"
    module.insert(
        "configure_queue".to_string(),
        Value::NativeFunction {
            name: "configure_queue".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "configure_queue() requires 1 argument (opts)".to_string(),
                    ));
                }

                let opts = match &args[0] {
                    Value::Map(m) => m,
                    _ => {
                        return Err(IntentError::type_error(
                            "configure_queue() requires a map argument".to_string(),
                        ))
                    }
                };

                // Check for testing mode
                if let Some(Value::String(mode)) = opts.get("mode") {
                    if mode == "testing" {
                        let mut tq = JOB_RUNTIME.test_queue.lock().map_err(|e| {
                            IntentError::runtime_error(format!("Lock error: {}", e))
                        })?;
                        *tq = Some(Vec::new());
                        return Ok(Value::ok(Value::Unit));
                    }
                }

                // Extract store URL
                let store_url = match opts.get("store") {
                    Some(Value::String(s)) => s.clone(),
                    Some(_) => {
                        return Err(IntentError::type_error(
                            "configure_queue() 'store' option must be a string".to_string(),
                        ))
                    }
                    None => "sqlite:./jobs.db".to_string(),
                };

                // Validate URL format — redis/valkey need explicit scheme,
                // everything else is treated as a SQLite path (consistent with std/kv)
                if store_url.contains("://")
                    && !store_url.starts_with("redis://")
                    && !store_url.starts_with("valkey://")
                {
                    return Err(IntentError::runtime_error(format!(
                        "Invalid store URL '{}'. Use a file path for SQLite, or redis:// / valkey:// for Redis",
                        store_url
                    )));
                }

                // Update the URL
                {
                    let mut url = JOB_RUNTIME
                        .kv_url
                        .lock()
                        .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;
                    *url = store_url.clone();
                }

                // Open the KV connection now
                let kv_handle_value = kv::open_kv(&store_url)?;
                let handle_info = extract_kv_handle_info(&kv_handle_value)?;
                {
                    let mut info = JOB_RUNTIME
                        .kv_handle_info
                        .lock()
                        .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;
                    *info = Some(handle_info);
                }

                Ok(Value::ok(Value::Unit))
            },
        },
    );

    // @ntnt enqueue
    // @module std/jobs
    // @signature enqueue(job_name: String, args: Map) -> Result<String, String>
    // Enqueue a background job for processing.
    //
    // Looks up the job name in the registry, generates a unique ID, serializes
    // the job data, and writes it to the configured KV store. Returns the job ID.
    // If configure_queue() hasn't been called, auto-initializes with SQLite.
    // @param job_name The registered job name (e.g., "SendEmail")
    // @param args A map of arguments to pass to the job's perform block
    // @returns Result containing the job ID string or an error
    // @example enqueue("SendEmail", map { "to": "alice@example.com" }) ~ "Enqueue an email job"
    // @example enqueue("ProcessPayment", map { "amount": 100 }) ~ "Enqueue a payment job"
    module.insert(
        "enqueue".to_string(),
        Value::NativeFunction {
            name: "enqueue".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "enqueue() requires 2 arguments (job_name, args)".to_string(),
                    ));
                }

                let job_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue() first argument must be a string job name".to_string(),
                        ))
                    }
                };

                let payload = match &args[1] {
                    Value::Map(_) => args[1].clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue() second argument must be a map".to_string(),
                        ))
                    }
                };

                enqueue_internal(&job_name, payload, &timestamp_key(), None)
            },
        },
    );

    // @ntnt job_status
    // @module std/jobs
    // @signature job_status(job_id: String) -> Result<Map, String>
    // Get the current status and data for a job by its ID.
    //
    // Returns the full job data map including status, type, queue, payload,
    // attempts, and timestamps. Returns an error if the job ID is not found.
    // @param job_id The job ID returned by enqueue()
    // @returns Result containing the job data map or an error
    // @example job_status("abc-123") ~ "Check job status"
    module.insert(
        "job_status".to_string(),
        Value::NativeFunction {
            name: "job_status".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "job_status() requires 1 argument (job_id)".to_string(),
                    ));
                }

                let job_id = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "job_status() requires a string job ID".to_string(),
                        ))
                    }
                };

                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                let data_key = format!("jobs:data:{}", job_id);
                let result = kv::kv_get(&kv_handle, &data_key)?;

                match result {
                    Value::Unit => Err(IntentError::runtime_error(format!(
                        "Job '{}' not found",
                        job_id
                    ))),
                    other => Ok(Value::ok(other)),
                }
            },
        },
    );

    // @ntnt cancel_job
    // @module std/jobs
    // @module_description Background job queue with persistent storage
    // @signature cancel_job(job_id: String, opts?: Map) -> Result<Bool, String>
    // Cancel a job by its ID.
    //
    // By default, only pending, scheduled, retrying, or failed jobs can be cancelled.
    // Pass `map { "force": true }` to cancel an active (running) job — this marks it
    // as cancelled and removes its visibility timeout key. The worker thread may still
    // be executing, but the result will be discarded when it checks the status.
    // Returns true if the job was cancelled, false if it was not in a cancellable state.
    // @param job_id The job ID returned by enqueue()
    // @param opts Optional map. Pass `map { "force": true }` to force-cancel active jobs.
    // @returns Result containing true if cancelled, false if not cancellable
    // @example cancel_job("abc-123") ~ "Cancel a pending job"
    // @example cancel_job("abc-123", map { "force": true }) ~ "Force-cancel a stuck active job"
    // @see_also retry_job, job_status
    module.insert(
        "cancel_job".to_string(),
        Value::NativeFunction {
            name: "cancel_job".to_string(),
            arity: 1,
            max_arity: 2,
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "cancel_job() requires 1-2 arguments (job_id, opts?)".to_string(),
                    ));
                }
                let job_id = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "cancel_job() first argument must be a string job ID".to_string(),
                        ))
                    }
                };
                let force = if args.len() > 1 {
                    match &args[1] {
                        Value::Map(opts) => matches!(opts.get("force"), Some(Value::Bool(true))),
                        _ => false,
                    }
                } else {
                    false
                };
                match cancel_job_by_id(&job_id, force)? {
                    CancelResult::Cancelled { .. } => Ok(Value::ok(Value::Bool(true))),
                    CancelResult::NotCancellable(_) => Ok(Value::ok(Value::Bool(false))),
                }
            },
        },
    );

    // @ntnt enqueue_at
    // @module std/jobs
    // @signature enqueue_at(job_name: String, timestamp: Int, args: Map) -> Result<String, String>
    // Enqueue a job to run at a specific future time.
    //
    // The timestamp is a Unix nanosecond timestamp (Int). The job will not be
    // picked up by workers until the current time reaches that timestamp.
    // @param job_name The registered job name (e.g., "SendEmail")
    // @param timestamp Unix nanosecond timestamp when the job should run
    // @param args A map of arguments to pass to the job's perform block
    // @returns Result containing the job ID string or an error
    // @example enqueue_at("SendEmail", now_nanos + 3600_000_000_000, map { "to": "alice@example.com" }) ~ "Enqueue email in 1 hour"
    // @see_also enqueue_in, enqueue
    module.insert(
        "enqueue_at".to_string(),
        Value::NativeFunction {
            name: "enqueue_at".to_string(),
            arity: 3,
            max_arity: 3,
            func: |args| {
                if args.len() != 3 {
                    return Err(IntentError::type_error(
                        "enqueue_at() requires 3 arguments (job_name, timestamp, args)".to_string(),
                    ));
                }

                let job_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_at() first argument must be a string job name".to_string(),
                        ))
                    }
                };

                let ts_nanos =
                    match &args[1] {
                        Value::Int(n) => *n,
                        _ => return Err(IntentError::type_error(
                            "enqueue_at() second argument must be an integer nanosecond timestamp"
                                .to_string(),
                        )),
                    };

                if ts_nanos < 0 {
                    return Err(IntentError::runtime_error(
                        "enqueue_at() timestamp must be non-negative".to_string(),
                    ));
                }

                let payload = match &args[2] {
                    Value::Map(_) => args[2].clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_at() third argument must be a map".to_string(),
                        ))
                    }
                };

                let pending_ts = format!("{:020}", ts_nanos);
                enqueue_internal(&job_name, payload, &pending_ts, Some(&pending_ts))
            },
        },
    );

    // @ntnt enqueue_in
    // @module std/jobs
    // @signature enqueue_in(job_name: String, delay_secs: Int, args: Map) -> Result<String, String>
    // Enqueue a job to run after a delay in seconds.
    //
    // Calculates the future timestamp as `now + delay_secs` and enqueues the job
    // to run no earlier than that time. Convenience wrapper around enqueue_at().
    // @param job_name The registered job name (e.g., "SendEmail")
    // @param delay_secs Number of seconds to wait before the job becomes eligible
    // @param args A map of arguments to pass to the job's perform block
    // @returns Result containing the job ID string or an error
    // @example enqueue_in("SendEmail", 3600, map { "to": "alice@example.com" }) ~ "Send email in 1 hour"
    // @example enqueue_in("PurgeCache", 300, map {}) ~ "Purge cache in 5 minutes"
    // @see_also enqueue_at, enqueue
    module.insert(
        "enqueue_in".to_string(),
        Value::NativeFunction {
            name: "enqueue_in".to_string(),
            arity: 3,
            max_arity: 3,
            func: |args| {
                if args.len() != 3 {
                    return Err(IntentError::type_error(
                        "enqueue_in() requires 3 arguments (job_name, delay_secs, args)"
                            .to_string(),
                    ));
                }

                let job_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_in() first argument must be a string job name".to_string(),
                        ))
                    }
                };

                let delay_secs = match &args[1] {
                    Value::Int(n) => *n,
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_in() second argument must be an integer (seconds)".to_string(),
                        ))
                    }
                };

                let payload = match &args[2] {
                    Value::Map(_) => args[2].clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_in() third argument must be a map".to_string(),
                        ))
                    }
                };

                let future_nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos()
                    + (delay_secs.max(0) as u128) * 1_000_000_000;
                let pending_ts = format!("{:020}", future_nanos);
                enqueue_internal(&job_name, payload, &pending_ts, Some(&pending_ts))
            },
        },
    );

    // @ntnt work_async
    // @module std/jobs
    // @signature work_async(opts?: Map) -> Array<TaskHandle>
    // Start one or more background worker threads that process jobs from the queue.
    //
    // Always returns an Array of TaskHandles (even for a single worker) that
    // can be used with cancel_task() to stop the workers. Workers run until
    // cancelled. If configure_queue() hasn't been called, auto-initializes
    // with the default SQLite store.
    // @param opts Optional configuration map:
    //   - "poll_interval": poll interval in milliseconds (default 1000)
    //   - "concurrency": number of parallel worker threads (default 1)
    //   - "queues": array of queue names to process (default: all queues)
    // @returns Array of TaskHandles (one per worker)
    // @see_also work_jobs, cancel_task
    // @example work_async() ~ "Start a single background worker"
    // @example work_async(map { "concurrency": 4, "poll_interval": 500 }) ~ "Start 4 workers polling every 500ms"
    // @example work_async(map { "queues": ["emails", "payments"] }) ~ "Process only specific queues"
    module.insert(
        "work_async".to_string(),
        Value::NativeFunction {
            name: "work_async".to_string(),
            arity: 0,
            max_arity: 1,
            func: |args| {
                let (poll_interval, concurrency, queues) = parse_work_opts(args)?;
                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                let mut handles = Vec::new();
                for _ in 0..concurrency {
                    match spawn_worker_task(kv_handle.clone(), poll_interval, queues.clone()) {
                        Ok(handle) => handles.push(handle),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Value::Array(handles))
            },
        },
    );

    // @ntnt work_jobs
    // @module std/jobs
    // @signature work_jobs(opts?: Map) -> Unit
    // Run a blocking worker loop that processes jobs from the queue.
    //
    // Runs on the current thread until interrupted (Ctrl-C) or cancelled via
    // cooperative cancellation. Typically called at the end of a worker script.
    // If configure_queue() hasn't been called, auto-initializes with the
    // default SQLite store.
    // @param opts Optional configuration map:
    //   - "poll_interval": poll interval in milliseconds (default 1000)
    //   - "queues": array of queue names to process (default: all queues)
    // @returns Unit (blocks until cancelled)
    // @see_also work_async, enqueue
    // @example work_jobs() ~ "Run a blocking worker (at end of worker script)"
    // @example work_jobs(map { "poll_interval": 500 }) ~ "Poll every 500ms"
    module.insert(
        "work_jobs".to_string(),
        Value::NativeFunction {
            name: "work_jobs".to_string(),
            arity: 0,
            max_arity: 1,
            func: |args| {
                let (poll_interval, concurrency, queues) = parse_work_opts(args)?;
                if concurrency > 1 {
                    return Err(IntentError::runtime_error(
                        "work_jobs() does not support concurrency > 1. Use work_async() for multiple worker threads.".to_string(),
                    ));
                }
                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                let kv_info = extract_kv_handle_info(&kv_handle)?;

                // Set up Ctrl-C cancellation so worker_loop can exit gracefully
                let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                let cancelled_clone = cancelled.clone();
                ctrlc::set_handler(move || {
                    cancelled_clone.store(true, std::sync::atomic::Ordering::Release);
                })
                .map_err(|e| {
                    IntentError::runtime_error(format!(
                        "Failed to set Ctrl-C handler: {}",
                        e
                    ))
                })?;
                CURRENT_TASK_CANCELLED.with(|cell| {
                    *cell.borrow_mut() = Some(cancelled);
                });

                worker_loop(kv_info, poll_interval, queues);
                Ok(Value::Unit)
            },
        },
    );

    // @ntnt assert_enqueued
    // @module std/jobs
    // @signature assert_enqueued(job_name: String, args?: Map) -> Result<Bool, String>
    // Assert that a job was enqueued in testing mode.
    //
    // Checks the test queue for a job with the given name. If args is provided,
    // performs a partial match — every key in args must match the corresponding
    // key in the job's payload. Returns Ok(true) on success, Err with a
    // descriptive message listing what was actually enqueued on failure.
    // Must call configure_queue(map { "mode": "testing" }) first.
    // @param job_name The job name to look for (e.g., "SendEmail")
    // @param args Optional map of payload keys to match (partial match)
    // @returns Ok(true) if found and matches, Err with details otherwise
    // @see_also assert_not_enqueued, drain_jobs, clear_jobs
    // @example assert_enqueued("SendEmail") ~ "Assert any SendEmail was enqueued"
    // @example assert_enqueued("SendEmail", map { "to": "alice@example.com" }) ~ "Assert SendEmail with specific args"
    module.insert(
        "assert_enqueued".to_string(),
        Value::NativeFunction {
            name: "assert_enqueued".to_string(),
            arity: 1,
            max_arity: 2,
            func: |args| {
                if args.is_empty() {
                    return Err(IntentError::type_error(
                        "assert_enqueued() requires at least 1 argument (job_name)".to_string(),
                    ));
                }
                let job_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "assert_enqueued() first argument must be a string job name"
                                .to_string(),
                        ))
                    }
                };

                let expected_args: Option<&HashMap<String, Value>> = if args.len() >= 2 {
                    match &args[1] {
                        Value::Map(m) => Some(m),
                        _ => {
                            return Err(IntentError::type_error(
                                "assert_enqueued() second argument must be a map".to_string(),
                            ))
                        }
                    }
                } else {
                    None
                };

                let tq = JOB_RUNTIME
                    .test_queue
                    .lock()
                    .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;

                let queue = match tq.as_ref() {
                    Some(q) => q,
                    None => {
                        return Err(IntentError::runtime_error(
                            "assert_enqueued() requires testing mode. Call configure_queue(map { \"mode\": \"testing\" }) first.".to_string(),
                        ))
                    }
                };

                // Collect matching jobs
                let matching: Vec<&EnqueuedJob> = queue
                    .iter()
                    .filter(|j| j.job_type == job_name)
                    .collect();

                if matching.is_empty() {
                    let enqueued_types: Vec<String> =
                        queue.iter().map(|j| j.job_type.clone()).collect();
                    return Err(IntentError::runtime_error(format!(
                        "Expected '{}' to be enqueued, but it was not. Enqueued jobs: [{}]",
                        job_name,
                        enqueued_types.join(", ")
                    )));
                }

                // If args provided, check for partial match
                if let Some(expected) = expected_args {
                    for job in &matching {
                        let payload: serde_json::Value =
                            serde_json::from_str(&job.payload_json).unwrap_or(serde_json::Value::Null);
                        let all_match = expected.iter().all(|(k, v)| {
                            let expected_json =
                                crate::stdlib::kv::value_to_json_public(v);
                            payload.get(k).map(|actual| *actual == expected_json).unwrap_or(false)
                        });
                        if all_match {
                            return Ok(Value::ok(Value::Bool(true)));
                        }
                    }
                    // None matched the args
                    let payloads: Vec<String> =
                        matching.iter().map(|j| j.payload_json.clone()).collect();
                    return Err(IntentError::runtime_error(format!(
                        "Found {} '{}' job(s) enqueued, but none matched the expected args. Payloads: [{}]",
                        matching.len(),
                        job_name,
                        payloads.join(", ")
                    )));
                }

                Ok(Value::ok(Value::Bool(true)))
            },
        },
    );

    // @ntnt assert_not_enqueued
    // @module std/jobs
    // @signature assert_not_enqueued(job_name: String) -> Result<Bool, String>
    // Assert that a job was NOT enqueued in testing mode.
    //
    // Checks the test queue and returns an error if any job with the given name
    // is found. Returns Ok(true) if no such job was enqueued.
    // Must call configure_queue(map { "mode": "testing" }) first.
    // @param job_name The job name to check (e.g., "SendEmail")
    // @returns Ok(true) if not enqueued, Err with details if found
    // @see_also assert_enqueued, drain_jobs, clear_jobs
    // @example assert_not_enqueued("SendEmail") ~ "Assert no SendEmail was enqueued"
    module.insert(
        "assert_not_enqueued".to_string(),
        Value::NativeFunction {
            name: "assert_not_enqueued".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "assert_not_enqueued() requires 1 argument (job_name)".to_string(),
                    ));
                }
                let job_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "assert_not_enqueued() argument must be a string job name".to_string(),
                        ))
                    }
                };

                let tq = JOB_RUNTIME
                    .test_queue
                    .lock()
                    .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;

                let queue = match tq.as_ref() {
                    Some(q) => q,
                    None => {
                        return Err(IntentError::runtime_error(
                            "assert_not_enqueued() requires testing mode. Call configure_queue(map { \"mode\": \"testing\" }) first.".to_string(),
                        ))
                    }
                };

                let found: Vec<&EnqueuedJob> =
                    queue.iter().filter(|j| j.job_type == job_name).collect();

                if !found.is_empty() {
                    let payloads: Vec<String> =
                        found.iter().map(|j| j.payload_json.clone()).collect();
                    return Err(IntentError::runtime_error(format!(
                        "Expected '{}' NOT to be enqueued, but found {} instance(s). Payloads: [{}]",
                        job_name,
                        found.len(),
                        payloads.join(", ")
                    )));
                }

                Ok(Value::ok(Value::Bool(true)))
            },
        },
    );

    // @ntnt drain_jobs
    // @module std/jobs
    // @signature drain_jobs() -> Result<Int, String>
    // Execute all enqueued test jobs synchronously and return the count.
    //
    // Takes all jobs from the test queue and executes each via the job's perform
    // block synchronously in the current thread. Returns the number of jobs
    // executed. Useful for integration tests that need to verify side effects.
    // Must call configure_queue(map { "mode": "testing" }) first.
    // @returns Ok(Int) with count of jobs executed, or Err on failure
    // @see_also assert_enqueued, clear_jobs
    // @example drain_jobs() ~ "Execute all pending test jobs"
    module.insert(
        "drain_jobs".to_string(),
        Value::NativeFunction {
            name: "drain_jobs".to_string(),
            arity: 0,
            max_arity: 0,
            func: |_args| {
                let jobs: Vec<EnqueuedJob> = {
                    let mut tq = JOB_RUNTIME
                        .test_queue
                        .lock()
                        .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;
                    match tq.as_mut() {
                        Some(q) => std::mem::take(q),
                        None => {
                            return Err(IntentError::runtime_error(
                                "drain_jobs() requires testing mode. Call configure_queue(map { \"mode\": \"testing\" }) first.".to_string(),
                            ))
                        }
                    }
                };

                // Run ALL jobs before reporting errors — do not bail early with `?`
                // so that a single failure never silently discards remaining queued jobs.
                let mut executed: i64 = 0;
                let mut errors: Vec<String> = Vec::new();

                for job in jobs {
                    let def = match JOB_RUNTIME.get_job(&job.job_type)? {
                        Some(d) => d,
                        None => {
                            errors.push(format!(
                                "drain_jobs(): no job definition found for '{}'",
                                job.job_type
                            ));
                            continue;
                        }
                    };

                    let payload_json: serde_json::Value =
                        serde_json::from_str(&job.payload_json).unwrap_or(serde_json::Value::Null);

                    // Convert serde_json::Value back to HashMap<String, Value>
                    let payload: HashMap<String, Value> = match payload_json {
                        serde_json::Value::Object(obj) => obj
                            .into_iter()
                            .map(|(k, v)| (k, crate::stdlib::kv::json_to_value_public(&v)))
                            .collect(),
                        _ => HashMap::new(),
                    };

                    match execute_job_perform(&def, &payload) {
                        Ok(_) => executed += 1,
                        Err(e) => errors.push(format!(
                            "drain_jobs(): job '{}' failed: {}",
                            job.job_type, e
                        )),
                    }
                }

                if !errors.is_empty() {
                    return Err(IntentError::runtime_error(errors.join("\n")));
                }

                Ok(Value::ok(Value::Int(executed)))
            },
        },
    );

    // @ntnt clear_jobs
    // @module std/jobs
    // @signature clear_jobs() -> Result<Unit, String>
    // Clear all jobs from the test queue without executing them.
    //
    // Empties the test queue. Useful for resetting state between tests.
    // Must call configure_queue(map { "mode": "testing" }) first.
    // @returns Ok(Unit) on success, Err if not in testing mode
    // @see_also assert_enqueued, drain_jobs
    // @example clear_jobs() ~ "Clear all enqueued test jobs"
    module.insert(
        "clear_jobs".to_string(),
        Value::NativeFunction {
            name: "clear_jobs".to_string(),
            arity: 0,
            max_arity: 0,
            func: |_args| {
                let mut tq = JOB_RUNTIME
                    .test_queue
                    .lock()
                    .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;
                match tq.as_mut() {
                    Some(q) => {
                        q.clear();
                        Ok(Value::ok(Value::Unit))
                    }
                    None => Err(IntentError::runtime_error(
                        "clear_jobs() requires testing mode. Call configure_queue(map { \"mode\": \"testing\" }) first.".to_string(),
                    )),
                }
            },
        },
    );

    // @ntnt retry_job
    // @module std/jobs
    // @module_description Background job queue with persistent storage
    // @signature retry_job(job_id: String) -> Result<Bool, String>
    // Re-queue a failed or dead job for another attempt.
    //
    // Resets the job's status to pending, clears its attempts counter and
    // error fields, and creates a new pending key so the worker picks it up.
    // Returns Ok(true) on success, Ok(false) if the job's current status
    // does not allow retry (only "failed" and "dead" are retryable).
    // @param job_id The ID of the job to retry
    // @returns Ok(true) if the job was re-queued, Ok(false) if the status is not retryable
    // @example retry_job("abc123") ~ "Re-queue a failed job"
    // @see_also cancel_job, job_status
    module.insert(
        "retry_job".to_string(),
        Value::NativeFunction {
            name: "retry_job".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "retry_job() requires 1 argument (job_id)".to_string(),
                    ));
                }
                let job_id = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "retry_job() requires a string job ID".to_string(),
                        ))
                    }
                };
                match retry_job_by_id(&job_id)? {
                    RetryResult::Requeued(_) => Ok(Value::ok(Value::Bool(true))),
                    RetryResult::NotRetryable(_) => Ok(Value::ok(Value::Bool(false))),
                }
            },
        },
    );

    // @ntnt list_jobs
    // @module std/jobs
    // @module_description Background job queue with persistent storage
    // @signature list_jobs(opts?: Map) -> Result<Array<Map>, String>
    // List jobs with optional status and queue filters.
    //
    // Returns an array of job data maps. Pass a map with optional "status"
    // and/or "queue" keys to filter. Pass "limit" to cap the result count
    // (default 100).
    // @param opts Optional filter map with "status", "queue", "limit" keys
    // @returns Ok(Array of job data Maps)
    // @example list_jobs() ~ "List all jobs (up to 100)"
    // @example list_jobs(map { "status": "failed" }) ~ "List failed jobs"
    // @example list_jobs(map { "status": "dead", "limit": 10 }) ~ "List up to 10 dead jobs"
    // @see_also job_status, retry_job
    module.insert(
        "list_jobs".to_string(),
        Value::NativeFunction {
            name: "list_jobs".to_string(),
            arity: 0,
            max_arity: 1,
            func: |args| {
                let (status, queue, limit) = if !args.is_empty() {
                    match &args[0] {
                        Value::Map(opts) => {
                            let sf = match opts.get("status") {
                                Some(Value::String(s)) => Some(s.clone()),
                                _ => None,
                            };
                            let qf = match opts.get("queue") {
                                Some(Value::String(s)) => Some(s.clone()),
                                _ => None,
                            };
                            let lim = match opts.get("limit") {
                                Some(Value::Int(n)) => (*n).max(1) as usize,
                                _ => 100,
                            };
                            (sf, qf, lim)
                        }
                        _ => (None, None, 100),
                    }
                } else {
                    (None, None, 100)
                };
                let results = list_jobs_filtered(ListJobsOpts {
                    status,
                    queue,
                    limit,
                })?;
                Ok(Value::ok(Value::Array(
                    results.into_iter().map(Value::Map).collect(),
                )))
            },
        },
    );

    // @ntnt delete_jobs
    // @module std/jobs
    // @module_description Background job queue with persistent storage
    // @signature delete_jobs(opts: Map) -> Result<Int, String>
    // Bulk delete jobs by status.
    //
    // Requires a "status" key in the options map to prevent accidental
    // deletion of all jobs. Returns the number of jobs deleted.
    // @param opts Map with required "status" key and optional "older_than_secs" (Int)
    // @returns Ok(Int) — count of deleted jobs
    // @example delete_jobs(map { "status": "completed" }) ~ "Delete all completed jobs"
    // @example delete_jobs(map { "status": "dead", "older_than_secs": 604800 }) ~ "Delete dead jobs older than 7 days"
    // @see_also list_jobs, clear_jobs
    module.insert(
        "delete_jobs".to_string(),
        Value::NativeFunction {
            name: "delete_jobs".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "delete_jobs() requires 1 argument (opts map with 'status' key)"
                            .to_string(),
                    ));
                }
                let opts = match &args[0] {
                    Value::Map(m) => m,
                    _ => {
                        return Err(IntentError::type_error(
                            "delete_jobs() argument must be a map".to_string(),
                        ))
                    }
                };
                let status = match opts.get("status") {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        return Err(IntentError::runtime_error(
                            "delete_jobs() requires a 'status' key in the options map".to_string(),
                        ))
                    }
                };
                let older_than_secs = match opts.get("older_than_secs") {
                    Some(Value::Int(n)) => Some(*n as u64),
                    _ => None,
                };
                let deleted = delete_jobs_filtered(DeleteJobsOpts {
                    status,
                    older_than_secs,
                })?;
                Ok(Value::ok(Value::Int(deleted)))
            },
        },
    );

    // @ntnt enqueue_batch
    // @module std/jobs
    // @signature enqueue_batch(job_name: String, args: Array<Map>) -> Result<Array<String>, String>
    // Enqueue multiple jobs of the same type in one call.
    //
    // Validates the job name and all payload types upfront before any writes.
    // Then enqueues each payload map from the array. Respects dedup (unique
    // option) and test mode. Returns an array of job IDs.
    // If the job has `unique` set and two items have identical payloads, the
    // same job ID is returned for both — no duplicate job is created.
    // Note: if a KV error occurs mid-batch, earlier jobs are already enqueued
    // (no rollback). This matches the behavior of calling enqueue() in a loop.
    // @param job_name The registered job name (e.g., "SendEmail")
    // @param args Array of payload maps — one job created per element
    // @returns Result containing array of job IDs, or error
    // @see_also enqueue, enqueue_at, enqueue_in
    // @error TypeError ~ "enqueue_batch() args[N] must be a map" fix: "Ensure every array element is a map"
    // @error RuntimeError ~ "Job 'X' is not registered" fix: "Define the job with: job X on queue { perform(...) { ... } }"
    // @example enqueue_batch("SendEmail", [map { "to": "alice@test.com" }, map { "to": "bob@test.com" }]) ~ "Enqueue 2 email jobs"
    // @example enqueue_batch("ProcessOrder", []) ~ "Empty array returns Ok([])"
    module.insert(
        "enqueue_batch".to_string(),
        Value::NativeFunction {
            name: "enqueue_batch".to_string(),
            arity: 2,
            max_arity: 2,
            func: |args| {
                if args.len() != 2 {
                    return Err(IntentError::type_error(
                        "enqueue_batch() requires 2 arguments (job_name, args_array)".to_string(),
                    ));
                }

                let job_name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_batch() first argument must be a string job name".to_string(),
                        ))
                    }
                };

                let items = match &args[1] {
                    Value::Array(arr) => arr.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "enqueue_batch() second argument must be an array of maps".to_string(),
                        ))
                    }
                };

                // Fast-fail: validate job exists before any KV writes.
                // enqueue_internal also re-validates per call (acquires read lock again).
                let job_def = JOB_RUNTIME.get_job(&job_name)?;
                if job_def.is_none() {
                    return Err(IntentError::runtime_error(format!(
                        "Job '{}' is not registered. Define it with: job {} on <queue> {{ perform(...) {{ ... }} }}",
                        job_name, job_name
                    )));
                }

                // Empty array — return immediately
                if items.is_empty() {
                    return Ok(Value::ok(Value::Array(vec![])));
                }

                // Guard against pathological batch sizes — each item is a KV write
                const MAX_BATCH_SIZE: usize = 10_000;
                if items.len() > MAX_BATCH_SIZE {
                    return Err(IntentError::runtime_error(format!(
                        "enqueue_batch() batch size {} exceeds maximum of {}",
                        items.len(),
                        MAX_BATCH_SIZE
                    )));
                }

                // Validate all items are maps before any writes (intentional double iteration:
                // first pass validates types, second pass writes to KV — ensures no partial
                // writes on type errors)
                for (i, item) in items.iter().enumerate() {
                    if !matches!(item, Value::Map(_)) {
                        return Err(IntentError::type_error(format!(
                            "enqueue_batch() args[{}] must be a map, got {}",
                            i,
                            item.type_name()
                        )));
                    }
                }

                // Enqueue each item using enqueue_internal (handles dedup, test mode, etc.)
                // Use base timestamp + per-item offset to preserve FIFO ordering within
                // the batch — wall-clock resolution is too coarse for tight loops.
                let base_nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                let mut ids = Vec::with_capacity(items.len());
                for (i, item) in items.into_iter().enumerate() {
                    let ts = format!("{:020}", base_nanos + i as u128);
                    // Wrap errors with item index so callers know which item failed
                    let result = enqueue_internal(&job_name, item, &ts, None).map_err(|e| {
                        IntentError::runtime_error(format!(
                            "enqueue_batch: item {} failed: {}",
                            i, e
                        ))
                    })?;
                    // enqueue_internal returns Value::ok(Value::String(job_id))
                    match result {
                        Value::EnumValue {
                            ref variant,
                            ref values,
                            ..
                        } if variant == "Ok" && !values.is_empty() => {
                            ids.push(values[0].clone());
                        }
                        _ => {
                            return Err(IntentError::runtime_error(format!(
                                "enqueue_batch: unexpected return type for item {}",
                                i
                            )));
                        }
                    }
                }

                Ok(Value::ok(Value::Array(ids)))
            },
        },
    );

    module
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that use the global JOB_RUNTIME.
    /// Parallel tests that call reset() or configure_queue() will race.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_runtime<F: FnOnce()>(f: F) {
        // unwrap_or_else recovers from poisoned mutex (a previous test panicked
        // while holding the lock — we still need to run subsequent tests)
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        JOB_RUNTIME.reset();
        f();
    }

    /// Create a minimal JobDefinition for tests (no perform body needed for registry tests).
    fn test_job_def(name: &str, queue: &str) -> JobDefinition {
        JobDefinition {
            name: name.to_string(),
            queue: queue.to_string(),
            options: HashMap::new(),
            perform_params: vec![],
            perform_body: crate::ast::Block { statements: vec![] },
            on_failure: None,
        }
    }

    fn test_job_def_with_opts(
        name: &str,
        queue: &str,
        options: HashMap<String, JobOptionValue>,
    ) -> JobDefinition {
        JobDefinition {
            name: name.to_string(),
            queue: queue.to_string(),
            options,
            perform_params: vec![],
            perform_body: crate::ast::Block { statements: vec![] },
            on_failure: None,
        }
    }

    #[test]
    fn test_register_job() {
        with_clean_runtime(|| {
            let result = JOB_RUNTIME.register_job(test_job_def("TestJob", "default"));
            assert!(result.is_ok());

            let job = JOB_RUNTIME.get_job("TestJob").unwrap();
            assert!(job.is_some());
            let def = job.unwrap();
            assert_eq!(def.name, "TestJob");
            assert_eq!(def.queue, "default");
        });
    }

    #[test]
    fn test_duplicate_job_registration() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("DupJob", "default"))
                .unwrap();

            let result = JOB_RUNTIME.register_job(test_job_def("DupJob", "other"));
            assert!(result.is_err());
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("Duplicate job definition"));
        });
    }

    #[test]
    fn test_get_nonexistent_job() {
        with_clean_runtime(|| {
            let result = JOB_RUNTIME.get_job("NoSuchJob").unwrap();
            assert!(result.is_none());
        });
    }

    #[test]
    fn test_job_options() {
        with_clean_runtime(|| {
            let mut options = HashMap::new();
            options.insert("retry".to_string(), JobOptionValue::Int(5));
            options.insert("timeout".to_string(), JobOptionValue::Int(120));

            JOB_RUNTIME
                .register_job(test_job_def_with_opts("OptsJob", "default", options))
                .unwrap();

            let def = JOB_RUNTIME.get_job("OptsJob").unwrap().unwrap();
            assert!(matches!(
                def.options.get("retry"),
                Some(JobOptionValue::Int(5))
            ));
            assert!(matches!(
                def.options.get("timeout"),
                Some(JobOptionValue::Int(120))
            ));
        });
    }

    #[test]
    fn test_enqueue_unregistered_job() {
        with_clean_runtime(|| {
            let module = init();
            let enqueue_fn = match module.get("enqueue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };

            let result = enqueue_fn(&[
                Value::String("NonexistentJob".to_string()),
                Value::Map(HashMap::new()),
            ]);
            assert!(result.is_err());
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("not registered"));
        });
    }

    #[test]
    fn test_enqueue_to_kv() {
        with_clean_runtime(|| {
            // Register a job
            JOB_RUNTIME
                .register_job(test_job_def("EmailJob", "emails"))
                .unwrap();

            // Configure with in-memory SQLite
            let module = init();
            let configure_fn = match module.get("configure_queue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut opts = HashMap::new();
            opts.insert(
                "store".to_string(),
                Value::String("sqlite::memory:".to_string()),
            );
            configure_fn(&[Value::Map(opts)]).unwrap();

            // Enqueue
            let enqueue_fn = match module.get("enqueue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut payload = HashMap::new();
            payload.insert(
                "to".to_string(),
                Value::String("alice@example.com".to_string()),
            );
            let result =
                enqueue_fn(&[Value::String("EmailJob".to_string()), Value::Map(payload)]).unwrap();

            // Result should be Ok(job_id_string)
            match result {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => {
                    assert_eq!(values.len(), 1);
                    match &values[0] {
                        Value::String(id) => {
                            assert!(!id.is_empty());
                            // Check job_status
                            let status_fn = match module.get("job_status").unwrap() {
                                Value::NativeFunction { func, .. } => func,
                                _ => panic!("Expected NativeFunction"),
                            };
                            let status_result = status_fn(&[Value::String(id.clone())]).unwrap();
                            match status_result {
                                Value::EnumValue {
                                    variant, values, ..
                                } if variant == "Ok" => match &values[0] {
                                    Value::Map(data) => {
                                        assert!(
                                            matches!(data.get("status"), Some(Value::String(s)) if s == "pending")
                                        );
                                        assert!(
                                            matches!(data.get("type"), Some(Value::String(s)) if s == "EmailJob")
                                        );
                                        assert!(
                                            matches!(data.get("queue"), Some(Value::String(s)) if s == "emails")
                                        );
                                    }
                                    _ => panic!("Expected Map in status Ok"),
                                },
                                _ => panic!("Expected Ok from job_status"),
                            }
                        }
                        _ => panic!("Expected String job ID"),
                    }
                }
                _ => panic!("Expected Ok result from enqueue: {:?}", result),
            }
        });
    }

    #[test]
    fn test_cancel_job() {
        with_clean_runtime(|| {
            // Register and configure
            JOB_RUNTIME
                .register_job(test_job_def("CancelJob", "default"))
                .unwrap();

            let module = init();
            let configure_fn = match module.get("configure_queue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut opts = HashMap::new();
            opts.insert(
                "store".to_string(),
                Value::String("sqlite::memory:".to_string()),
            );
            configure_fn(&[Value::Map(opts)]).unwrap();

            // Enqueue
            let enqueue_fn = match module.get("enqueue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let result = enqueue_fn(&[
                Value::String("CancelJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            let job_id = match result {
                Value::EnumValue { values, .. } => match &values[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected string job ID"),
                },
                _ => panic!("Expected Ok"),
            };

            // Cancel
            let cancel_fn = match module.get("cancel_job").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let cancel_result = cancel_fn(&[Value::String(job_id.clone())]).unwrap();
            match cancel_result {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => {
                    assert!(matches!(&values[0], Value::Bool(true)));
                }
                _ => panic!("Expected Ok(true) from cancel"),
            }

            // Verify status is cancelled
            let status_fn = match module.get("job_status").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let status = status_fn(&[Value::String(job_id)]).unwrap();
            match status {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => match &values[0] {
                    Value::Map(data) => {
                        assert!(
                            matches!(data.get("status"), Some(Value::String(s)) if s == "cancelled")
                        );
                    }
                    _ => panic!("Expected Map"),
                },
                _ => panic!("Expected Ok from status"),
            }
        });
    }

    #[test]
    fn test_configure_queue_invalid_url() {
        with_clean_runtime(|| {
            let module = init();
            let configure_fn = match module.get("configure_queue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut opts = HashMap::new();
            opts.insert(
                "store".to_string(),
                Value::String("invalid://something".to_string()),
            );
            let result = configure_fn(&[Value::Map(opts)]);
            assert!(result.is_err());
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("Invalid store URL") || err.contains("Use a file path"));
        });
    }

    #[test]
    fn test_timestamp_key_ordering() {
        let k1 = timestamp_key();
        // Ensure it's 20 chars zero-padded
        assert_eq!(k1.len(), 20);
        assert!(k1.chars().all(|c| c.is_ascii_digit()));

        // Two calls should produce non-decreasing keys
        std::thread::sleep(std::time::Duration::from_millis(1));
        let k2 = timestamp_key();
        assert!(k2 >= k1);
    }

    // -----------------------------------------------------------------------
    // New tests for backoff, execute helpers, and enqueue_at/enqueue_in
    // -----------------------------------------------------------------------

    #[test]
    fn test_calculate_backoff_exponential() {
        // Default: base=5, attempt=0 → 5*1=5
        assert_eq!(calculate_backoff("exponential", 0, 5), 5);
        // attempt=1 → 5*2=10
        assert_eq!(calculate_backoff("exponential", 1, 5), 10);
        // attempt=2 → 5*4=20
        assert_eq!(calculate_backoff("exponential", 2, 5), 20);
        // Capped at 3600
        assert_eq!(calculate_backoff("exponential", 100, 5), 3600);
    }

    #[test]
    fn test_calculate_backoff_linear() {
        assert_eq!(calculate_backoff("linear", 0, 10), 0);
        assert_eq!(calculate_backoff("linear", 1, 10), 10);
        assert_eq!(calculate_backoff("linear", 3, 10), 30);
    }

    #[test]
    fn test_calculate_backoff_constant() {
        assert_eq!(calculate_backoff("constant", 0, 15), 15);
        assert_eq!(calculate_backoff("constant", 10, 15), 15);
    }

    #[test]
    fn test_calculate_backoff_unknown_defaults_to_exponential() {
        // Unknown strategy falls through to exponential
        assert_eq!(calculate_backoff("unknown", 1, 5), 10);
    }

    #[test]
    fn test_execute_job_perform_empty_body() {
        // A job with an empty perform body should return Unit
        let def = test_job_def("EmptyJob", "default");
        let result = execute_job_perform(&def, &HashMap::new());
        assert!(result.is_ok(), "Empty body should succeed: {:?}", result);
    }

    #[test]
    fn test_enqueue_at() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("ScheduledJob", "default"))
                .unwrap();

            let module = init();
            let configure_fn = match module.get("configure_queue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut opts = HashMap::new();
            opts.insert(
                "store".to_string(),
                Value::String("sqlite::memory:".to_string()),
            );
            configure_fn(&[Value::Map(opts)]).unwrap();

            let enqueue_at_fn = match module.get("enqueue_at").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };

            // Use a timestamp far in the future (year ~2096, fits in i64)
            let future_nanos: i64 = 4_000_000_000_000_000_000i64;
            let result = enqueue_at_fn(&[
                Value::String("ScheduledJob".to_string()),
                Value::Int(future_nanos),
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            // Should return Ok(job_id)
            let job_id = match result {
                Value::EnumValue {
                    variant,
                    ref values,
                    ..
                } if variant == "Ok" => match &values[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected String job ID"),
                },
                _ => panic!("Expected Ok from enqueue_at: {:?}", result),
            };

            // Verify scheduled_at is stored in job data
            let status_fn = match module.get("job_status").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let status = status_fn(&[Value::String(job_id)]).unwrap();
            match status {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => match &values[0] {
                    Value::Map(data) => {
                        let scheduled_at = match data.get("scheduled_at") {
                            Some(Value::String(s)) => s.clone(),
                            _ => panic!("Expected scheduled_at in job data"),
                        };
                        let expected_ts = format!("{:020}", future_nanos);
                        assert_eq!(scheduled_at, expected_ts);
                        // Status should be "scheduled" for future-dated jobs
                        assert!(
                            matches!(data.get("status"), Some(Value::String(s)) if s == "scheduled")
                        );
                    }
                    _ => panic!("Expected Map"),
                },
                _ => panic!("Expected Ok from job_status"),
            }
        });
    }

    #[test]
    fn test_enqueue_in() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("DelayedJob", "default"))
                .unwrap();

            let module = init();
            let configure_fn = match module.get("configure_queue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut opts = HashMap::new();
            opts.insert(
                "store".to_string(),
                Value::String("sqlite::memory:".to_string()),
            );
            configure_fn(&[Value::Map(opts)]).unwrap();

            let enqueue_in_fn = match module.get("enqueue_in").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };

            let before_nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();

            let result = enqueue_in_fn(&[
                Value::String("DelayedJob".to_string()),
                Value::Int(60), // 60 seconds
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            let job_id = match result {
                Value::EnumValue {
                    variant,
                    ref values,
                    ..
                } if variant == "Ok" => match &values[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected String job ID"),
                },
                _ => panic!("Expected Ok from enqueue_in: {:?}", result),
            };

            // Verify scheduled_at is at least 60 seconds from before_nanos
            let status_fn = match module.get("job_status").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let status = status_fn(&[Value::String(job_id)]).unwrap();
            match status {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => match &values[0] {
                    Value::Map(data) => {
                        let scheduled_at_str = match data.get("scheduled_at") {
                            Some(Value::String(s)) => s.clone(),
                            _ => panic!("Expected scheduled_at in job data"),
                        };
                        let scheduled_nanos: u128 = scheduled_at_str
                            .parse()
                            .expect("scheduled_at should be a valid number");
                        let min_expected = before_nanos + 60 * 1_000_000_000;
                        assert!(
                            scheduled_nanos >= min_expected,
                            "scheduled_at {} should be >= {}",
                            scheduled_nanos,
                            min_expected
                        );
                    }
                    _ => panic!("Expected Map"),
                },
                _ => panic!("Expected Ok from job_status"),
            }
        });
    }

    #[test]
    fn test_enqueue_at_type_errors() {
        with_clean_runtime(|| {
            let module = init();
            let enqueue_at_fn = match module.get("enqueue_at").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };

            // Wrong arg count
            let r = enqueue_at_fn(&[Value::String("Job".to_string())]);
            assert!(r.is_err());

            // Non-integer timestamp
            let r = enqueue_at_fn(&[
                Value::String("Job".to_string()),
                Value::String("not-a-number".to_string()),
                Value::Map(HashMap::new()),
            ]);
            assert!(r.is_err());
        });
    }

    #[test]
    fn test_enqueue_in_type_errors() {
        with_clean_runtime(|| {
            let module = init();
            let enqueue_in_fn = match module.get("enqueue_in").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };

            // Non-integer delay
            let r = enqueue_in_fn(&[
                Value::String("Job".to_string()),
                Value::Float(1.5),
                Value::Map(HashMap::new()),
            ]);
            assert!(r.is_err());

            // Non-map args
            let r = enqueue_in_fn(&[
                Value::String("Job".to_string()),
                Value::Int(10),
                Value::String("not-a-map".to_string()),
            ]);
            assert!(r.is_err());
        });
    }

    #[test]
    fn test_parse_work_opts_defaults() {
        let (poll, concurrency, queues) = parse_work_opts(&[]).unwrap();
        assert_eq!(poll, 1000);
        assert_eq!(concurrency, 1);
        assert!(queues.is_none());
    }

    #[test]
    fn test_parse_work_opts_custom() {
        let mut opts = HashMap::new();
        opts.insert("poll_interval".to_string(), Value::Int(500));
        opts.insert("concurrency".to_string(), Value::Int(4));
        let mut queues_arr = Vec::new();
        queues_arr.push(Value::String("emails".to_string()));
        queues_arr.push(Value::String("payments".to_string()));
        opts.insert("queues".to_string(), Value::Array(queues_arr));

        let (poll, concurrency, queues) = parse_work_opts(&[Value::Map(opts)]).unwrap();
        assert_eq!(poll, 500);
        assert_eq!(concurrency, 4);
        assert_eq!(
            queues,
            Some(vec!["emails".to_string(), "payments".to_string()])
        );
    }

    #[test]
    fn test_worker_loop_end_to_end() {
        with_clean_runtime(|| {
            // Register a job with an empty perform body (returns Unit on success)
            JOB_RUNTIME
                .register_job(test_job_def("E2EJob", "default"))
                .unwrap();

            // Configure in-memory SQLite
            let module = init();
            let configure_fn = match module.get("configure_queue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut opts = HashMap::new();
            opts.insert(
                "store".to_string(),
                Value::String("sqlite::memory:".to_string()),
            );
            configure_fn(&[Value::Map(opts)]).unwrap();

            // Enqueue a job
            let enqueue_fn = match module.get("enqueue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let result = enqueue_fn(&[
                Value::String("E2EJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();
            let job_id = match result {
                Value::EnumValue { values, .. } => match &values[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected string ID"),
                },
                _ => panic!("Expected Ok"),
            };

            // Run worker_loop in a thread for one iteration, then cancel
            let kv_handle = JOB_RUNTIME.get_or_init_kv().unwrap();
            let kv_info = extract_kv_handle_info(&kv_handle).unwrap();
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancel_clone = cancel.clone();
            let handle = std::thread::spawn(move || {
                // Set cancellation flag so the loop can be stopped
                crate::stdlib::concurrent::CURRENT_TASK_CANCELLED.with(|cell| {
                    *cell.borrow_mut() = Some(cancel_clone);
                });
                worker_loop(kv_info, 50, None);
            });

            // Give the worker time to process
            std::thread::sleep(std::time::Duration::from_millis(300));

            // Cancel the worker
            cancel.store(true, std::sync::atomic::Ordering::Release);
            handle.join().unwrap();

            // Check job status — should be "completed"
            let status_fn = match module.get("job_status").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let status = status_fn(&[Value::String(job_id)]).unwrap();
            match status {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => match &values[0] {
                    Value::Map(data) => {
                        assert!(
                            matches!(data.get("status"), Some(Value::String(s)) if s == "completed"),
                            "Expected 'completed', got {:?}",
                            data.get("status")
                        );
                        assert!(data.get("completed_at").is_some());
                    }
                    _ => panic!("Expected Map"),
                },
                _ => panic!("Expected Ok from job_status"),
            }
        });
    }

    #[test]
    fn test_enqueue_at_rejects_negative_timestamp() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("NegTsJob", "default"))
                .unwrap();
            let module = init();
            let enqueue_at_fn = match module.get("enqueue_at").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let result = enqueue_at_fn(&[
                Value::String("NegTsJob".to_string()),
                Value::Int(-1),
                Value::Map(HashMap::new()),
            ]);
            assert!(result.is_err());
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("non-negative"));
        });
    }

    // -----------------------------------------------------------------------
    // Testing mode / DX helpers
    // -----------------------------------------------------------------------

    fn get_fn<'a>(module: &'a HashMap<String, Value>, name: &str) -> fn(&[Value]) -> Result<Value> {
        match module.get(name).unwrap() {
            Value::NativeFunction { func, .. } => *func,
            _ => panic!("Expected NativeFunction for {}", name),
        }
    }

    fn setup_memory_kv() {
        let module = init();
        let configure_fn = get_fn(&module, "configure_queue");
        let mut opts = HashMap::new();
        opts.insert(
            "store".to_string(),
            Value::String("sqlite::memory:".to_string()),
        );
        configure_fn(&[Value::Map(opts)]).unwrap();
    }

    fn activate_testing_mode(module: &HashMap<String, Value>) {
        let configure_fn = get_fn(module, "configure_queue");
        let mut opts = HashMap::new();
        opts.insert("mode".to_string(), Value::String("testing".to_string()));
        configure_fn(&[Value::Map(opts)]).unwrap();
    }

    #[test]
    fn test_testing_mode_activation() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("TmJob", "default"))
                .unwrap();
            let module = init();
            activate_testing_mode(&module);

            // Enqueue should go to test queue, not KV
            let enqueue_fn = get_fn(&module, "enqueue");
            let result = enqueue_fn(&[
                Value::String("TmJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "Expected Ok from enqueue in testing mode"
            );

            // Verify it ended up in test_queue
            let tq = JOB_RUNTIME.test_queue.lock().unwrap();
            let queue = tq.as_ref().unwrap();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].job_type, "TmJob");
        });
    }

    #[test]
    fn test_assert_enqueued_found() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("AeJob", "default"))
                .unwrap();
            let module = init();
            activate_testing_mode(&module);

            let enqueue_fn = get_fn(&module, "enqueue");
            enqueue_fn(&[
                Value::String("AeJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            let assert_fn = get_fn(&module, "assert_enqueued");
            let result = assert_fn(&[Value::String("AeJob".to_string())]).unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "assert_enqueued should succeed"
            );
        });
    }

    #[test]
    fn test_assert_enqueued_partial_match() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("EmailJob2", "emails"))
                .unwrap();
            let module = init();
            activate_testing_mode(&module);

            let enqueue_fn = get_fn(&module, "enqueue");
            let mut payload = HashMap::new();
            payload.insert(
                "to".to_string(),
                Value::String("alice@example.com".to_string()),
            );
            payload.insert("subject".to_string(), Value::String("Hello".to_string()));
            enqueue_fn(&[Value::String("EmailJob2".to_string()), Value::Map(payload)]).unwrap();

            let assert_fn = get_fn(&module, "assert_enqueued");

            // Partial match: only check "to"
            let mut expected = HashMap::new();
            expected.insert(
                "to".to_string(),
                Value::String("alice@example.com".to_string()),
            );
            let result =
                assert_fn(&[Value::String("EmailJob2".to_string()), Value::Map(expected)]).unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "Partial match should succeed"
            );
        });
    }

    #[test]
    fn test_assert_enqueued_not_found() {
        with_clean_runtime(|| {
            let module = init();
            activate_testing_mode(&module);

            let assert_fn = get_fn(&module, "assert_enqueued");
            let result = assert_fn(&[Value::String("MissingJob".to_string())]);
            assert!(
                result.is_err(),
                "assert_enqueued should fail when not found"
            );
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("MissingJob"), "Error should mention job name");
        });
    }

    #[test]
    fn test_assert_not_enqueued_pass() {
        with_clean_runtime(|| {
            let module = init();
            activate_testing_mode(&module);

            let assert_fn = get_fn(&module, "assert_not_enqueued");
            let result = assert_fn(&[Value::String("NeverEnqueued".to_string())]).unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "assert_not_enqueued should succeed when nothing enqueued"
            );
        });
    }

    #[test]
    fn test_assert_not_enqueued_fail() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("AnJob", "default"))
                .unwrap();
            let module = init();
            activate_testing_mode(&module);

            let enqueue_fn = get_fn(&module, "enqueue");
            enqueue_fn(&[
                Value::String("AnJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            let assert_fn = get_fn(&module, "assert_not_enqueued");
            let result = assert_fn(&[Value::String("AnJob".to_string())]);
            assert!(
                result.is_err(),
                "assert_not_enqueued should fail when job is found"
            );
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("AnJob"));
        });
    }

    #[test]
    fn test_drain_jobs() {
        with_clean_runtime(|| {
            // Register a job with empty perform body
            JOB_RUNTIME
                .register_job(test_job_def("DrainJob", "default"))
                .unwrap();
            let module = init();
            activate_testing_mode(&module);

            let enqueue_fn = get_fn(&module, "enqueue");
            enqueue_fn(&[
                Value::String("DrainJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();
            enqueue_fn(&[
                Value::String("DrainJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            let drain_fn = get_fn(&module, "drain_jobs");
            let result = drain_fn(&[]).unwrap();

            match result {
                Value::EnumValue {
                    ref variant,
                    ref values,
                    ..
                } if variant == "Ok" => {
                    assert!(
                        matches!(values[0], Value::Int(2)),
                        "Should drain 2 jobs, got {:?}",
                        values[0]
                    );
                }
                _ => panic!("Expected Ok(2) from drain_jobs"),
            }

            // Queue should now be empty
            let tq = JOB_RUNTIME.test_queue.lock().unwrap();
            assert!(
                tq.as_ref().unwrap().is_empty(),
                "Queue should be empty after drain"
            );
        });
    }

    #[test]
    fn test_clear_jobs() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("ClearJob", "default"))
                .unwrap();
            let module = init();
            activate_testing_mode(&module);

            let enqueue_fn = get_fn(&module, "enqueue");
            enqueue_fn(&[
                Value::String("ClearJob".to_string()),
                Value::Map(HashMap::new()),
            ])
            .unwrap();

            // Verify something is in the queue
            {
                let tq = JOB_RUNTIME.test_queue.lock().unwrap();
                assert_eq!(tq.as_ref().unwrap().len(), 1);
            }

            let clear_fn = get_fn(&module, "clear_jobs");
            let result = clear_fn(&[]).unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "clear_jobs should return Ok"
            );

            // Queue should be empty
            let tq = JOB_RUNTIME.test_queue.lock().unwrap();
            assert!(
                tq.as_ref().unwrap().is_empty(),
                "Queue should be empty after clear"
            );
        });
    }

    // -----------------------------------------------------------------------
    // Dedup tests
    // -----------------------------------------------------------------------

    fn test_job_def_with_unique(name: &str, queue: &str, unique_secs: i64) -> JobDefinition {
        let mut options = HashMap::new();
        options.insert("unique".to_string(), JobOptionValue::Int(unique_secs));
        test_job_def_with_opts(name, queue, options)
    }

    fn configure_in_memory(module: &HashMap<String, Value>) {
        let configure_fn = get_fn(module, "configure_queue");
        let mut opts = HashMap::new();
        opts.insert(
            "store".to_string(),
            Value::String("sqlite::memory:".to_string()),
        );
        configure_fn(&[Value::Map(opts)]).unwrap();
    }

    fn enqueue_job(
        module: &HashMap<String, Value>,
        job_name: &str,
        payload: HashMap<String, Value>,
    ) -> String {
        let enqueue_fn = get_fn(module, "enqueue");
        let result =
            enqueue_fn(&[Value::String(job_name.to_string()), Value::Map(payload)]).unwrap();
        match result {
            Value::EnumValue {
                ref variant,
                ref values,
                ..
            } if variant == "Ok" => match &values[0] {
                Value::String(s) => s.clone(),
                _ => panic!("Expected string job ID in Ok variant"),
            },
            Value::EnumValue {
                variant, values, ..
            } => {
                panic!("Expected Ok from enqueue, got {}: {:?}", variant, values)
            }
            _ => panic!("Expected EnumValue from enqueue"),
        }
    }

    /// Two enqueues of the same job with the same payload should return the same ID.
    #[test]
    fn test_dedup_same_payload() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def_with_unique("DedupJob", "default", 3600))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            let mut payload = HashMap::new();
            payload.insert("key".to_string(), Value::String("value".to_string()));

            let id1 = enqueue_job(&module, "DedupJob", payload.clone());
            let id2 = enqueue_job(&module, "DedupJob", payload.clone());

            assert_eq!(id1, id2, "Same payload should return same job ID (dedup)");
        });
    }

    /// Two enqueues with different payloads should return different IDs.
    #[test]
    fn test_dedup_different_payload() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def_with_unique("DedupJobDiff", "default", 3600))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            let mut payload1 = HashMap::new();
            payload1.insert("key".to_string(), Value::String("value1".to_string()));
            let mut payload2 = HashMap::new();
            payload2.insert("key".to_string(), Value::String("value2".to_string()));

            let id1 = enqueue_job(&module, "DedupJobDiff", payload1);
            let id2 = enqueue_job(&module, "DedupJobDiff", payload2);

            assert_ne!(
                id1, id2,
                "Different payloads should produce different job IDs"
            );
        });
    }

    /// Jobs without `unique` option should always get new IDs.
    #[test]
    fn test_no_dedup_without_unique() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("NoDedupJob", "default"))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            let id1 = enqueue_job(&module, "NoDedupJob", HashMap::new());
            let id2 = enqueue_job(&module, "NoDedupJob", HashMap::new());

            assert_ne!(
                id1, id2,
                "Without unique option, two enqueues should get different IDs"
            );
        });
    }

    /// Dedup key should be cleared when a job is cancelled, allowing re-enqueue.
    #[test]
    fn test_dedup_cleared_on_cancel() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def_with_unique("DedupCancelJob", "default", 3600))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            let mut payload = HashMap::new();
            payload.insert("task".to_string(), Value::String("send".to_string()));

            let id1 = enqueue_job(&module, "DedupCancelJob", payload.clone());

            // Cancel the job
            let cancel_fn = get_fn(&module, "cancel_job");
            let result = cancel_fn(&[Value::String(id1.clone())]).unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "Cancel should succeed"
            );

            // Re-enqueue — should get a new ID since dedup key was cleared
            let id2 = enqueue_job(&module, "DedupCancelJob", payload.clone());
            assert_ne!(id1, id2, "After cancel, re-enqueue should get a new job ID");
        });
    }

    /// If the existing job under a dedup key is in a terminal state (e.g., from
    /// a previous run with a stale key), re-enqueue should succeed with a new ID.
    #[test]
    fn test_dedup_stale_terminal_job() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def_with_unique(
                    "DedupTerminalJob",
                    "default",
                    3600,
                ))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            let mut payload = HashMap::new();
            payload.insert("msg".to_string(), Value::String("hello".to_string()));

            // Enqueue first
            let id1 = enqueue_job(&module, "DedupTerminalJob", payload.clone());

            // Manually set the job to "dead" status to simulate a terminal job
            let kv_handle = JOB_RUNTIME.get_or_init_kv().unwrap();
            let data_key = format!("jobs:data:{}", id1);
            if let Ok(Value::Map(mut data)) = kv::kv_get(&kv_handle, &data_key) {
                data.insert("status".to_string(), Value::String("dead".to_string()));
                kv::kv_set(&kv_handle, &data_key, &Value::Map(data), None).unwrap();
            }

            // Re-enqueue — the dedup check should see the terminal status and allow a new job
            let id2 = enqueue_job(&module, "DedupTerminalJob", payload.clone());
            assert_ne!(
                id1, id2,
                "Dedup should allow new job when existing job is in terminal state"
            );
        });
    }

    #[test]
    fn test_scheduled_job_not_claimed_by_worker() {
        with_clean_runtime(|| {
            // Register a job
            JOB_RUNTIME
                .register_job(test_job_def("FutureJob", "default"))
                .unwrap();

            // Configure in-memory SQLite
            let module = init();
            let configure_fn = match module.get("configure_queue").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let mut opts = HashMap::new();
            opts.insert(
                "store".to_string(),
                Value::String("sqlite::memory:".to_string()),
            );
            configure_fn(&[Value::Map(opts)]).unwrap();

            // Enqueue a job far in the future (year 2099)
            let enqueue_at_fn = match module.get("enqueue_at").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let future_ts: i64 = 4_102_444_800_000_000_000; // ~2099
            let result = enqueue_at_fn(&[
                Value::String("FutureJob".to_string()),
                Value::Int(future_ts),
                Value::Map(HashMap::new()),
            ])
            .unwrap();
            let job_id = match result {
                Value::EnumValue { values, .. } => match &values[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected string ID"),
                },
                _ => panic!("Expected Ok"),
            };

            // Run worker for a short time — it should NOT claim the future job
            let kv_handle = JOB_RUNTIME.get_or_init_kv().unwrap();
            let kv_info = extract_kv_handle_info(&kv_handle).unwrap();
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let cancel_clone = cancel.clone();
            let handle = std::thread::spawn(move || {
                crate::stdlib::concurrent::CURRENT_TASK_CANCELLED.with(|cell| {
                    *cell.borrow_mut() = Some(cancel_clone);
                });
                worker_loop(kv_info, 50, None);
            });

            // Let worker poll a few times
            std::thread::sleep(std::time::Duration::from_millis(200));
            cancel.store(true, std::sync::atomic::Ordering::Release);
            handle.join().unwrap();

            // Job should still be pending (not claimed, not completed)
            let status_fn = match module.get("job_status").unwrap() {
                Value::NativeFunction { func, .. } => func,
                _ => panic!("Expected NativeFunction"),
            };
            let status = status_fn(&[Value::String(job_id)]).unwrap();
            match status {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => match &values[0] {
                    Value::Map(data) => {
                        // enqueue_at sets status to "scheduled" (not "pending")
                        let job_status = match data.get("status") {
                            Some(Value::String(s)) => s.as_str(),
                            _ => "unknown",
                        };
                        assert!(
                            job_status == "pending" || job_status == "scheduled",
                            "Future job should still be pending/scheduled, got {:?}",
                            data.get("status")
                        );
                    }
                    _ => panic!("Expected Map"),
                },
                _ => panic!("Expected Ok from job_status"),
            }
        });
    }

    #[test]
    fn test_enqueue_batch_basic() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("BatchJob", "default"))
                .unwrap();
            setup_memory_kv();

            let module = init();
            let batch_fn = get_fn(&module, "enqueue_batch");

            let items = vec![
                Value::Map({
                    let mut m = HashMap::new();
                    m.insert(
                        "to".to_string(),
                        Value::String("alice@test.com".to_string()),
                    );
                    m
                }),
                Value::Map({
                    let mut m = HashMap::new();
                    m.insert("to".to_string(), Value::String("bob@test.com".to_string()));
                    m
                }),
                Value::Map({
                    let mut m = HashMap::new();
                    m.insert(
                        "to".to_string(),
                        Value::String("carol@test.com".to_string()),
                    );
                    m
                }),
            ];

            let result =
                batch_fn(&[Value::String("BatchJob".to_string()), Value::Array(items)]).unwrap();

            match result {
                Value::EnumValue {
                    ref variant,
                    ref values,
                    ..
                } if variant == "Ok" => match &values[0] {
                    Value::Array(ids) => {
                        assert_eq!(ids.len(), 3, "Should create 3 jobs");
                        // All IDs should be unique strings
                        let id_strs: Vec<String> = ids
                            .iter()
                            .map(|v| match v {
                                Value::String(s) => s.clone(),
                                _ => panic!("Expected string ID"),
                            })
                            .collect();
                        assert_ne!(id_strs[0], id_strs[1]);
                        assert_ne!(id_strs[1], id_strs[2]);
                        assert_ne!(id_strs[0], id_strs[2]);
                    }
                    _ => panic!("Expected Array in Ok"),
                },
                _ => panic!("Expected Ok from enqueue_batch: {:?}", result),
            }
        });
    }

    #[test]
    fn test_enqueue_batch_empty() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("BatchEmpty", "default"))
                .unwrap();
            setup_memory_kv();

            let module = init();
            let batch_fn = get_fn(&module, "enqueue_batch");

            let result = batch_fn(&[
                Value::String("BatchEmpty".to_string()),
                Value::Array(vec![]),
            ])
            .unwrap();

            match result {
                Value::EnumValue {
                    ref variant,
                    ref values,
                    ..
                } if variant == "Ok" => match &values[0] {
                    Value::Array(ids) => assert!(ids.is_empty(), "Empty input → empty output"),
                    _ => panic!("Expected Array"),
                },
                _ => panic!("Expected Ok"),
            }
        });
    }

    #[test]
    fn test_enqueue_batch_unregistered() {
        with_clean_runtime(|| {
            setup_memory_kv();

            let module = init();
            let batch_fn = get_fn(&module, "enqueue_batch");

            let result = batch_fn(&[
                Value::String("NoSuchJob".to_string()),
                Value::Array(vec![Value::Map(HashMap::new())]),
            ]);
            assert!(result.is_err(), "Unregistered job should error");
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("not registered"));
        });
    }

    #[test]
    fn test_enqueue_batch_bad_item() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("BatchBad", "default"))
                .unwrap();
            setup_memory_kv();

            let module = init();
            let batch_fn = get_fn(&module, "enqueue_batch");

            // Second item is a string, not a map
            let result = batch_fn(&[
                Value::String("BatchBad".to_string()),
                Value::Array(vec![
                    Value::Map(HashMap::new()),
                    Value::String("not a map".to_string()),
                ]),
            ]);
            assert!(result.is_err(), "Non-map item should error");
            let err = format!("{}", result.unwrap_err());
            assert!(err.contains("args[1]"), "Error should identify index");
        });
    }

    #[test]
    fn test_enqueue_batch_test_mode() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("BatchTest", "default"))
                .unwrap();

            // Enable test mode
            let module = init();
            let configure_fn = get_fn(&module, "configure_queue");
            let mut opts = HashMap::new();
            opts.insert("mode".to_string(), Value::String("testing".to_string()));
            configure_fn(&[Value::Map(opts)]).unwrap();

            let batch_fn = get_fn(&module, "enqueue_batch");
            let items = vec![Value::Map(HashMap::new()), Value::Map(HashMap::new())];
            batch_fn(&[Value::String("BatchTest".to_string()), Value::Array(items)]).unwrap();

            // Check test queue has 2 items
            let tq = JOB_RUNTIME.test_queue.lock().unwrap();
            assert_eq!(
                tq.as_ref().unwrap().len(),
                2,
                "Test queue should have 2 items"
            );
        });
    }

    #[test]
    fn test_enqueue_batch_dedup() {
        with_clean_runtime(|| {
            let mut opts = HashMap::new();
            opts.insert("unique".to_string(), JobOptionValue::Int(3600));
            JOB_RUNTIME
                .register_job(test_job_def_with_opts("BatchDedupJob", "default", opts))
                .unwrap();
            setup_memory_kv();

            let module = init();
            let batch_fn = get_fn(&module, "enqueue_batch");

            // Two identical payloads — dedup should return same ID for both
            let same_payload = || {
                let mut m = HashMap::new();
                m.insert(
                    "email".to_string(),
                    Value::String("alice@test.com".to_string()),
                );
                Value::Map(m)
            };

            let result = batch_fn(&[
                Value::String("BatchDedupJob".to_string()),
                Value::Array(vec![same_payload(), same_payload()]),
            ])
            .unwrap();

            match result {
                Value::EnumValue {
                    ref variant,
                    ref values,
                    ..
                } if variant == "Ok" => match &values[0] {
                    Value::Array(ids) => {
                        assert_eq!(ids.len(), 2, "Should return 2 IDs");
                        let id0 = match &ids[0] {
                            Value::String(s) => s.clone(),
                            _ => panic!("Expected string"),
                        };
                        let id1 = match &ids[1] {
                            Value::String(s) => s.clone(),
                            _ => panic!("Expected string"),
                        };
                        assert_eq!(
                            id0, id1,
                            "Identical payloads with unique should return same ID (deduped)"
                        );
                    }
                    _ => panic!("Expected Array"),
                },
                _ => panic!("Expected Ok"),
            }
        });
    }
}
