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
use crate::interpreter::{FunctionContract, RuntimeCapability, Value};
use crate::stdlib::concurrent::{
    check_task_limit, finalize_task, is_current_task_cancelled, sleep_cancellable, CancelToken,
    CURRENT_CANCEL_TOKEN, RUNTIME,
};
use crate::stdlib::kv;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
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

    /// Ceiling key for kv_claim: upper bound of this band's priority range.
    ///
    /// NOTE: With priority-prefixed keys, a single ceiling cannot gate both priority
    /// AND timestamp (a future job at priority 15 sorts below priority 39 ceiling
    /// regardless of timestamp). The ceiling restricts to the band's priority range
    /// only. Future-scheduled jobs are filtered by the defense-in-depth `scheduled_at`
    /// check in worker_loop (log + re-enqueue + sleep).
    pub fn ceiling_key(&self) -> String {
        format!("jobs:pending:{:02}:~", self.max_priority)
    }
}

/// Rate limit parsed from a `rate: "N/interval"` job option.
#[derive(Debug, Clone)]
pub struct RateLimit {
    /// Maximum number of executions per window.
    pub count: u64,
    /// Window duration in seconds (1 for /second, 60 for /minute, 3600 for /hour).
    pub window_secs: u64,
}

/// Parse `"N/interval"` rate limit strings.
///
/// Supported intervals: `second`, `minute`, `hour`.
/// Returns `None` for invalid format (caller should error at registration time).
pub(crate) fn parse_rate_limit(s: &str) -> Option<RateLimit> {
    let (count_str, interval) = s.trim().split_once('/')?;
    let count: u64 = count_str.trim().parse().ok()?;
    if count == 0 {
        return None;
    }
    let window_secs = match interval.trim() {
        "second" => 1,
        "minute" => 60,
        "hour" => 3600,
        _ => return None,
    };
    Some(RateLimit { count, window_secs })
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
    /// Optional contract (requires/ensures) for the perform block
    pub perform_contract: Option<FunctionContract>,
    /// Body of the perform block — executed by workers in a child scope of the worker interpreter
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
/// **Lock discipline:**
/// When acquiring multiple locks, always use this order:
///   1. `band_worker_task_ids`
///   2. `band_cancel_arcs`
///   3. `active_bands`
/// Prefer acquiring one lock at a time (release before acquiring another).
/// When two locks must be held simultaneously (e.g., scale_workers), follow the order above.
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
    pub band_worker_task_ids: Mutex<HashMap<String, Vec<u64>>>,
    /// Active band configurations (set at work_jobs/work_async startup).
    pub active_bands: Mutex<Vec<BandConfig>>,
    /// Cancel Arcs for active band workers — used by scale_workers to cancel excess threads.
    pub band_cancel_arcs: Mutex<HashMap<String, Vec<Arc<CancelToken>>>>,
    /// Queue filter active at worker startup — so scale_workers uses the same queue filter.
    pub active_queues: Mutex<Option<Vec<String>>>,
    /// Main source file path — set when a job is registered so workers can recreate a
    /// full interpreter (with imports and user functions) for each job execution.
    source_file: Mutex<Option<String>>,
    /// In-memory cache of paused queue names (fast read path — no KV round-trip per poll).
    pub paused_queues: RwLock<HashSet<String>>,
    /// Timestamp of last pause cache refresh from KV. Refreshed lazily every 5 seconds
    /// to pick up pauses from other processes in multi-process deployments.
    paused_cache_updated_at: Mutex<std::time::Instant>,
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
            band_cancel_arcs: Mutex::new(HashMap::new()),
            active_queues: Mutex::new(None),
            source_file: Mutex::new(None),
            paused_queues: RwLock::new(HashSet::new()),
            // Initialize as stale (10s in the past) so first is_queue_paused() call
            // forces a KV refresh — ensures pauses persisted before restart are respected.
            paused_cache_updated_at: Mutex::new(
                std::time::Instant::now() - std::time::Duration::from_secs(10),
            ),
        }
    }

    /// Register a job definition. Idempotent — silently skips if a job with the
    /// same name is already registered (first registration wins). This is intentional:
    /// worker threads re-execute the .tnt file and hit job declarations again.
    pub fn register_job(&self, def: JobDefinition) -> Result<()> {
        let mut registry = self.job_registry.write().map_err(|e| {
            IntentError::runtime_error(format!("Job registry lock poisoned: {}", e))
        })?;
        if registry.contains_key(&def.name) {
            // Idempotent: silently skip re-registration. Workers re-execute
            // the .tnt file, hitting job declarations again. First registration wins.
            return Ok(());
        }
        registry.insert(def.name.clone(), def);
        Ok(())
    }

    /// Register a job definition, overwriting any existing definition with the same name.
    ///
    /// Used by hot-reload (HotReload execution mode) to update perform bodies.
    /// Workers read definitions fresh via `get_job()` on each iteration, so updated
    /// perform blocks take effect on the next job run. Workers always see either the
    /// old or new definition, never a missing one.
    ///
    /// Note: workers cache their interpreter at startup, so new imports or helper
    /// functions in the updated definition won't be available until workers restart.
    pub fn register_job_overwrite(&self, def: JobDefinition) -> Result<()> {
        let mut registry = self.job_registry.write().map_err(|e| {
            IntentError::runtime_error(format!("Job registry lock poisoned: {}", e))
        })?;
        registry.insert(def.name.clone(), def);
        Ok(())
    }

    /// Set the main source file path (called when a job is registered).
    ///
    /// Panics if the mutex is poisoned — this indicates a prior panic in a
    /// critical path and the runtime is in an unrecoverable state.
    pub fn set_source_file(&self, path: String) {
        let mut sf = self
            .source_file
            .lock()
            .expect("Job source_file lock poisoned");
        *sf = Some(path);
    }

    /// Get the main source file path, if set.
    ///
    /// Panics if the mutex is poisoned — workers must not silently fall back
    /// to a bare interpreter when the lock is in an unrecoverable state.
    pub fn get_source_file(&self) -> Option<String> {
        self.source_file
            .lock()
            .expect("Job source_file lock poisoned")
            .clone()
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
        if let Ok(mut a) = self.band_cancel_arcs.lock() {
            a.clear();
        }
        if let Ok(mut aq) = self.active_queues.lock() {
            *aq = None;
        }
        if let Ok(mut sf) = self.source_file.lock() {
            *sf = None;
        }
        if let Ok(mut pq) = self.paused_queues.write() {
            pq.clear();
        }
        if let Ok(mut ts) = self.paused_cache_updated_at.lock() {
            // Reset as stale so next is_queue_paused() refreshes from KV
            *ts = std::time::Instant::now() - std::time::Duration::from_secs(10);
        }
    }
}

pub static JOB_RUNTIME: LazyLock<JobRuntime> = LazyLock::new(JobRuntime::new);

// ============================================================================
// Batch Runtime — in-memory state for open (unsealed) batches
// ============================================================================

#[derive(Clone)]
struct BufferedJob {
    job_type: String,
    /// Payload serialized as JSON — avoids Value's Rc/Send issues (mirrors EnqueuedJob).
    payload_json: String,
    /// Set to true once written to KV during seal. Prevents duplicates on retry.
    flushed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BatchStatus {
    /// Batch is open — enqueues are accepted.
    Open,
    /// Batch is being sealed — no more enqueues, KV flush in progress.
    Sealing,
}

#[allow(dead_code)]
#[derive(Clone)]
struct BatchState {
    id: String,
    name: String,
    /// Names of callbacks that were provided ("on_success", "on_complete", "on_death").
    /// Actual closures not stored — Phase 2 handles serialization.
    callback_names: Vec<String>,
    buffered: Vec<BufferedJob>,
    created_at: String,
    status: BatchStatus,
}

pub(crate) struct BatchRuntime {
    batches: Mutex<HashMap<String, BatchState>>,
}

impl BatchRuntime {
    fn new() -> Self {
        BatchRuntime {
            batches: Mutex::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub(crate) fn reset(&self) {
        match self.batches.lock() {
            Ok(mut b) => b.clear(),
            Err(e) => {
                eprintln!(
                    "[WARN] BatchRuntime::reset(): mutex poisoned, clearing anyway: {}",
                    e
                );
                let mut b = e.into_inner();
                b.clear();
            }
        }
    }
}

pub(crate) static BATCH_RUNTIME: LazyLock<BatchRuntime> = LazyLock::new(BatchRuntime::new);

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

/// Build batch metadata map with common fields used for tracking job batches.
/// Caller is responsible for updating status-specific fields (e.g. fired flags, timestamps).
fn build_batch_meta(
    batch_id: &str,
    name: &str,
    created_at: &str,
    status: &str,
    total: i64,
    pending: i64,
) -> HashMap<String, Value> {
    let mut meta = HashMap::new();
    meta.insert("id".to_string(), Value::String(batch_id.to_string()));
    meta.insert("name".to_string(), Value::String(name.to_string()));
    meta.insert("status".to_string(), Value::String(status.to_string()));
    meta.insert("total".to_string(), Value::Int(total));
    meta.insert("pending".to_string(), Value::Int(pending));
    meta.insert("succeeded".to_string(), Value::Int(0));
    meta.insert("dead".to_string(), Value::Int(0));
    meta.insert("cancelled".to_string(), Value::Int(0));
    meta.insert("fired_success".to_string(), Value::Bool(false));
    meta.insert("fired_complete".to_string(), Value::Bool(false));
    meta.insert("fired_death".to_string(), Value::Bool(false));
    meta.insert(
        "created_at".to_string(),
        Value::String(created_at.to_string()),
    );
    meta.insert("sealed_at".to_string(), Value::none());
    meta.insert("completed_at".to_string(), Value::none());
    meta
}

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

/// Create a fully-initialised interpreter for a job worker.
///
/// If a main source file has been recorded via `JOB_RUNTIME.set_source_file()`,
/// reads, parses, and evaluates it in `Worker` execution mode (so that
/// `work_async()`/`work_jobs()` are no-ops during bootstrap), then switches the
/// interpreter to `Job` mode for actual job execution.  This gives the interpreter
/// access to all imports and user-defined functions from the application, so job
/// perform blocks can call any helper the user has defined.
///
/// If no source file has been set (e.g. in unit tests that call `worker_loop`
/// directly) a bare interpreter is returned — job perform bodies run with no
/// application context, which is sufficient for simple test jobs.
///
/// Panics if a source file path is set but the file cannot be read, parsed, or evaluated.
fn create_job_interpreter() -> crate::interpreter::Interpreter {
    let Some(source_path) = JOB_RUNTIME.get_source_file() else {
        // No source file set — return a bare interpreter (sufficient for tests).
        // In production this means work_async() was called before any job declarations,
        // so there's nothing useful for the worker to execute anyway.
        eprintln!("[ntnt] warning: worker started without source file — jobs will run without app context");
        return crate::interpreter::Interpreter::new();
    };

    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|e| panic!("failed to read '{}' for worker: {}", source_path, e));

    use crate::lexer::Lexer;
    use crate::parser::Parser;
    let tokens: Vec<_> = Lexer::new(&source).collect();
    let ast = Parser::new(tokens)
        .parse()
        .unwrap_or_else(|e| panic!("failed to parse '{}' for worker: {}", source_path, e));

    let mut interp = crate::interpreter::Interpreter::new();
    // Bootstrap in Worker mode (not Job) so that work_async()/work_jobs()/scale_workers()
    // are no-ops during source evaluation — prevents recursive worker spawning.
    interp.set_execution_mode(crate::interpreter::ExecutionMode::Worker);
    interp.set_current_file(&source_path);
    interp.set_main_source_file(&source_path);
    interp
        .eval(&ast)
        .unwrap_or_else(|e| panic!("failed to evaluate '{}' for worker: {}", source_path, e));
    // Switch to Job mode for actual job execution semantics.
    interp.set_execution_mode(crate::interpreter::ExecutionMode::Job);

    interp
}

/// Execute a job's perform block using a pre-initialised interpreter.
///
/// Snapshots the interpreter's environment, pushes a child scope for parameters,
/// evaluates the perform body, then unconditionally restores the snapshot.
/// This is depth-independent: even if the perform body has nested blocks that
/// each push their own scope, a panic at any depth restores correctly.
fn execute_in_worker(
    interp: &mut crate::interpreter::Interpreter,
    def: &JobDefinition,
    payload: &HashMap<String, Value>,
) -> std::result::Result<Value, String> {
    // Snapshot before any scope manipulation — unconditional restore is depth-safe
    let snapshot = interp.snapshot_env();

    // Push a child scope for this job's parameters
    interp.push_scope();

    // Inject perform parameters from the payload
    for param in &def.perform_params {
        let val = payload.get(&param.name).cloned().unwrap_or(Value::Unit);
        interp.define_in_scope(param.name.clone(), val);
    }

    // Evaluate the perform body (with contract checking if present)
    let body = def.perform_body.clone();
    let name = def.name.clone();
    let contract = def.perform_contract.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        interp.eval_block_with_contract(&body, &name, contract.as_ref())
    }));

    // Unconditionally restore to the snapshot — works regardless of how many
    // nested scopes eval_block leaked on panic.
    interp.restore_env(snapshot);

    // Clean up interpreter state that may have accumulated during a panic.
    // call_depth is incremented on function entry and decremented on exit;
    // a Rust panic skips the decrement, leaving the depth permanently > 0.
    // Reset both deferred statements and call depth together so subsequent
    // jobs on this reused interpreter start from a clean state.
    if result.is_err() {
        interp.clear_deferred();
        interp.reset_call_depth();
        interp.clear_contract_state();
    }

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

/// Execute a job's on_failure handler using a pre-initialised interpreter.
///
/// Binds `error` (String) and `attempt` (Int) to the on_failure params
/// (first and second respectively, falling back to those names if the param
/// list is shorter).  Errors are silently discarded — on_failure is
/// fire-and-forget.
fn execute_on_failure_in_worker(
    interp: &mut crate::interpreter::Interpreter,
    def: &JobDefinition,
    error: &str,
    attempt: i64,
) {
    let Some((params, body)) = def.on_failure.as_ref() else {
        return;
    };

    // Snapshot before scope manipulation — depth-safe restore on panic
    let snapshot = interp.snapshot_env();

    interp.push_scope();

    // Bind by position: first param → error string, second param → attempt int.
    // Also bind the conventional names so handlers can use either.
    interp.define_in_scope("error".to_string(), Value::String(error.to_string()));
    interp.define_in_scope("attempt".to_string(), Value::Int(attempt));

    if let Some(p) = params.first() {
        interp.define_in_scope(p.name.clone(), Value::String(error.to_string()));
    }
    if let Some(p) = params.get(1) {
        interp.define_in_scope(p.name.clone(), Value::Int(attempt));
    }

    let body = body.clone();
    let panic_result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| interp.eval_block(&body)));

    // Unconditionally restore — depth-safe regardless of nested scope leaks
    interp.restore_env(snapshot);

    if panic_result.is_err() {
        interp.clear_deferred();
        interp.reset_call_depth();
        interp.clear_contract_state();
    }
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
    batch_id: Option<&str>,
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
        Some(other) => {
            return Err(IntentError::type_error(format!(
                "Priority must be a string (\"critical\", \"high\", \"normal\", \"low\") or integer 0-99, got {:?}",
                other
            )));
        }
        None => 50, // default: "normal"
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

    let in_test_mode = JOB_RUNTIME
        .test_queue
        .lock()
        .map(|tq| tq.is_some())
        .unwrap_or(false);

    // Generate job_id early so we can use it as the atomic dedup claim value.
    let job_id = Uuid::new_v4().to_string();

    // Atomic dedup: use kv_set_nx to claim the dedup key before writing job data.
    //
    // Pre-flight stale check: if the dedup key already exists and references a
    // terminal job (cancelled/dead/expired/failed), delete it so set_nx can succeed.
    // Then atomically claim the slot — only one concurrent enqueue wins.
    if let (Some(ref dk), false) = (&dedup_key, in_test_mode) {
        let kv_handle = JOB_RUNTIME.get_or_init_kv()?;

        // Check for stale dedup key referencing a terminal job.
        if let Ok(Value::String(ref existing_id)) = kv::kv_get(&kv_handle, dk) {
            let data_key = format!("jobs:data:{}", existing_id);
            let is_terminal = match kv::kv_get(&kv_handle, &data_key) {
                Ok(Value::Map(data)) => match data.get("status") {
                    Some(Value::String(s)) => {
                        matches!(s.as_str(), "cancelled" | "dead" | "expired" | "failed")
                    }
                    _ => true,
                },
                Ok(Value::Unit) => false, // data not yet written — treat as live (set_nx arbitrates)
                Ok(_) => false,           // unexpected shape — conservative: treat as live
                Err(_) => {
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
            if is_terminal {
                let _ = kv::kv_del(&kv_handle, dk);
            }
        }

        // Atomically claim the dedup slot with our new job_id.
        let ttl = unique_secs;
        let claimed = kv::kv_set_nx(&kv_handle, dk, &Value::String(job_id.clone()), ttl)
            .unwrap_or_else(|e| {
                emit_job_event(
                    "job.dedup_warning",
                    &[(
                        "reason",
                        Value::String(format!("kv_set_nx error (skipping dedup): {}", e)),
                    )],
                );
                true // fail-open: proceed with enqueue on KV error
            });

        if !claimed {
            // Concurrent enqueue already holds the slot — return existing job_id.
            match kv::kv_get(&kv_handle, dk) {
                Ok(Value::String(existing_id)) => {
                    return Ok(Value::ok(Value::String(existing_id)));
                }
                _ => {
                    // Key vanished between set_nx and get (TTL race).
                    // Re-attempt claiming so subsequent enqueues see a live dedup entry.
                    let _ =
                        kv::kv_set_nx(&kv_handle, dk, &Value::String(job_id.clone()), unique_secs);
                }
            }
        }
    }

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
    if let Some(bid) = batch_id {
        job_data.insert("batch_id".to_string(), Value::String(bid.to_string()));
    }

    // Copy job options (retry, timeout, etc.)
    for (k, v) in &job_def.options {
        job_data.insert(k.clone(), v.to_value());
    }

    // Add priority and band name to job data
    job_data.insert("priority".to_string(), Value::Int(priority as i64));
    // Derive band name from active band config (if workers are running),
    // falling back to default band ranges when no workers are active yet.
    let band_name = {
        let active = JOB_RUNTIME
            .active_bands
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if active.is_empty() {
            // No workers started yet — use default band ranges
            let defaults = default_bands();
            defaults
                .iter()
                .find(|b| priority >= b.min_priority && priority <= b.max_priority)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "unknown".to_string())
        } else {
            active
                .iter()
                .find(|b| priority >= b.min_priority && priority <= b.max_priority)
                .map(|b| b.name.clone())
                .unwrap_or_else(|| "unknown".to_string())
        }
    };
    job_data.insert("band".to_string(), Value::String(band_name));

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

    // Write queue ordering key: jobs:pending:<priority>:<timestamp>:<id>
    kv::kv_set(
        &kv_handle,
        &pending_key,
        &Value::String(job_id.clone()),
        None,
    )?;

    // Note: dedup key was already written atomically via kv_set_nx above (before job_data).
    // No second write needed here.

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

fn reenqueue_job(kv_handle: &Value, job_data: &HashMap<String, Value>, job_id: &str) {
    if let Some(Value::String(pk)) = job_data.get("pending_key") {
        let _ = kv::kv_set(kv_handle, pk, &Value::String(job_id.to_string()), None);
    }
}

/// RAII guard that decrements the concurrency counter on drop.
struct ConcurrencyGuard {
    kv_handle: Value,
    counter_key: Option<String>,
}

impl ConcurrencyGuard {
    /// Manually release the concurrency slot (e.g., before a long sleep).
    /// After calling this, drop() becomes a no-op.
    fn release(&mut self) {
        if let Some(ref ck) = self.counter_key.take() {
            let _ = kv::kv_incr(&self.kv_handle, ck, -1);
        }
    }
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        if let Some(ref ck) = self.counter_key {
            let _ = kv::kv_incr(&self.kv_handle, ck, -1);
        }
    }
}

/// Worker loop — runs until cooperative cancellation is signalled.
///
/// `kv_info`: serializable KV handle info (reconstructed into a `Value` on entry).
/// `band`: the band configuration (determines floor, ceiling, and poll interval).
/// `queues`: if Some, only process jobs whose queue field matches one of
///           these names; if None, process all queues.
/// Sleep for `dur`; returns `true` if the task was cancelled (caller should `break`).
fn sleep_or_break(dur: std::time::Duration) -> bool {
    sleep_cancellable(dur)
}

/// Re-enqueue a job then sleep for `dur`; returns `true` if cancelled (caller should `break`).
fn reenqueue_and_backoff(
    kv_handle: &Value,
    job_data: &HashMap<String, Value>,
    job_id: &str,
    dur: std::time::Duration,
) -> bool {
    reenqueue_job(kv_handle, job_data, job_id);
    sleep_cancellable(dur)
}

fn worker_loop(kv_info: KvHandleInfo, band: BandConfig, queues: Option<Vec<String>>) {
    let kv_handle = kv_info.to_value();
    let poll_duration = std::time::Duration::from_millis(band.poll_interval_ms);
    let band_stats = JOB_RUNTIME.get_or_create_band_stats(&band.name);

    // Build a fully-initialised interpreter once per worker thread so that
    // job perform blocks have access to all imports and user-defined functions.
    let mut interp = create_job_interpreter();

    loop {
        if is_current_task_cancelled() {
            break;
        }

        // Compute floor and ceiling for this band's priority range.
        // ceiling filters out future-scheduled jobs; floor restricts to band's min priority.
        let floor = band.floor_key();
        let ceiling = band.ceiling_key();

        let claimed = match kv::kv_claim(&kv_handle, "jobs:pending:", Some(&floor), Some(&ceiling))
        {
            Ok(Some((_pending_key, value))) => value,
            Ok(None) => {
                // Queue empty — sleep and try again
                if sleep_or_break(poll_duration) {
                    break;
                }
                continue;
            }
            Err(_) => {
                if sleep_or_break(poll_duration) {
                    break;
                }
                continue;
            }
        };

        // The claimed value is the job_id string
        let job_id = match &claimed {
            Value::String(s) => s.clone(),
            _ => {
                if sleep_or_break(poll_duration) {
                    break;
                }
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
                if sleep_or_break(poll_duration) {
                    break;
                }
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
                if sleep_or_break(poll_duration) {
                    break;
                }
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

        // Check if queue is paused before executing.
        let job_queue_for_pause = match job_data.get("queue") {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        if is_queue_paused(&job_queue_for_pause, &kv_handle) {
            reenqueue_job(&kv_handle, &job_data, &job_id);
            emit_job_event(
                "job.queue_paused",
                &[
                    ("job_id", Value::String(job_id.clone())),
                    ("queue", Value::String(job_queue_for_pause)),
                ],
            );
            if sleep_or_break(poll_duration) {
                break;
            }
            continue;
        }

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
                continue;
            }
        };

        // Concurrency limit: atomic counter semaphore via kv_incr.
        let has_concurrency_limit =
            if let Some(JobOptionValue::Int(max_slots)) = def.options.get("concurrency") {
                let max = (*max_slots).max(1) as i64;
                let counter_key = format!("jobs:concurrency:{}", job_type);
                match kv::kv_incr(&kv_handle, &counter_key, 1) {
                    Ok(new_count) => {
                        // Refresh TTL on every acquire so counter self-heals after SIGKILL/OOM.
                        let _ = kv::kv_expire(&kv_handle, &counter_key, 310);
                        if new_count > max {
                            let _ = kv::kv_incr(&kv_handle, &counter_key, -1);
                            reenqueue_job(&kv_handle, &job_data, &job_id);
                            emit_job_event(
                                "job.concurrency_limited",
                                &[
                                    ("job_id", Value::String(job_id.clone())),
                                    ("type", Value::String(job_type.clone())),
                                    ("max", Value::Int(max)),
                                    ("current", Value::Int(new_count)),
                                ],
                            );
                            if sleep_or_break(std::time::Duration::from_millis(500)) {
                                break;
                            }
                            continue;
                        }
                        true
                    }
                    Err(_) => {
                        if reenqueue_and_backoff(
                            &kv_handle,
                            &job_data,
                            &job_id,
                            std::time::Duration::from_millis(500),
                        ) {
                            break;
                        }
                        continue;
                    }
                }
            } else {
                false
            };
        let mut concurrency_guard = ConcurrencyGuard {
            kv_handle: kv_handle.clone(),
            counter_key: if has_concurrency_limit {
                Some(format!("jobs:concurrency:{}", job_type))
            } else {
                None
            },
        };

        // Rate limit: sliding window counter via kv_incr.
        // Uses weighted average of current + previous window to smooth boundary bursts.
        if let Some(JobOptionValue::String(rate_str)) = def.options.get("rate") {
            if let Some(rl) = parse_rate_limit(rate_str) {
                let now_secs = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let ws = rl.window_secs as i64;
                let window_start = now_secs - (now_secs % ws);
                let prev_window_start = window_start - ws;
                let rl_key = format!("jobs:ratelimit:{}:{}", job_type, window_start);
                let prev_key = format!("jobs:ratelimit:{}:{}", job_type, prev_window_start);
                match kv::kv_incr(&kv_handle, &rl_key, 1) {
                    Ok(current_count) => {
                        if current_count == 1 {
                            // Best-effort TTL — if it fails, key may leak but counter stays
                            // correct. Deleting would reset the window and allow overage.
                            let _ = kv::kv_expire(&kv_handle, &rl_key, ws * 2);
                        }
                        // Sliding window: weight previous window by how much of it is still relevant
                        let prev_count = match kv::kv_get(&kv_handle, &prev_key) {
                            Ok(Value::Int(n)) => n,
                            Ok(Value::String(s)) => s.parse::<i64>().unwrap_or(0),
                            _ => 0,
                        };
                        let elapsed_pct = (now_secs % ws) as f64 / ws as f64;
                        let weighted =
                            current_count as f64 + prev_count as f64 * (1.0 - elapsed_pct);
                        if weighted > rl.count as f64 {
                            let _ = kv::kv_incr(&kv_handle, &rl_key, -1);
                            concurrency_guard.release();
                            reenqueue_job(&kv_handle, &job_data, &job_id);
                            let remaining = ws - (now_secs % ws);
                            emit_job_event(
                                "job.rate_limited",
                                &[
                                    ("job_id", Value::String(job_id.clone())),
                                    ("type", Value::String(job_type.clone())),
                                    ("window", Value::String(rate_str.clone())),
                                    ("current", Value::Int(current_count)),
                                    ("weighted", Value::Float(weighted)),
                                    ("retry_after_secs", Value::Int(remaining)),
                                ],
                            );
                            if sleep_or_break(std::time::Duration::from_secs(
                                remaining.max(1) as u64
                            )) {
                                break;
                            }
                            continue;
                        }
                    }
                    Err(e) => {
                        concurrency_guard.release();
                        emit_job_event(
                            "job.rate_limit_error",
                            &[
                                ("job_id", Value::String(job_id.clone())),
                                ("type", Value::String(job_type.clone())),
                                ("error", Value::String(e.to_string())),
                            ],
                        );
                        if reenqueue_and_backoff(&kv_handle, &job_data, &job_id, poll_duration) {
                            break;
                        }
                        continue;
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
            let _ = kv::kv_del(&kv_handle, &active_key);
            continue;
        }

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

        // Track active count for stats
        band_stats
            .active
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

        // Record start time for timeout detection and duration stats
        let start = std::time::Instant::now();

        let exec_result = execute_in_worker(&mut interp, &def, &payload);

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

        // Record elapsed time for band stats
        let elapsed_ms = start.elapsed().as_millis() as u64;

        // Execute result handling
        match exec_result {
            Ok(_) => {
                // Update band stats: decrement active, increment completed, add duration
                band_stats
                    .active
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                band_stats
                    .completed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                band_stats
                    .total_duration_ms
                    .fetch_add(elapsed_ms, std::sync::atomic::Ordering::Relaxed);

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
                // Update band stats: decrement active, increment failed
                band_stats
                    .active
                    .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                band_stats
                    .failed
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

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
                execute_on_failure_in_worker(&mut interp, &def, &err_msg, new_attempts);

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
                    // Priority was validated at enqueue time — default 50 is defensive
                    // for any jobs missing the field (shouldn't happen normally)
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

/// Parse a single BandConfig from a Value::Map.
///
/// Supports two key styles (design doc API takes precedence):
///   - `"range": [min, max]` **or** `"min_priority"` / `"max_priority"` (legacy)
///   - `"poll": ms` **or** `"poll_interval": ms` (legacy)
///   - `"name"`: required String
///   - `"concurrency"`: optional Int >= 1 (default 1)
///
/// Poll minimum is 100ms; concurrency 0 is rejected, > 32 emits a warning.
fn parse_band_config(m: &HashMap<String, Value>) -> Result<BandConfig> {
    let name = match m.get("name") {
        Some(Value::String(s)) => s.clone(),
        _ => {
            return Err(IntentError::type_error(
                "band config must have a 'name' string".to_string(),
            ))
        }
    };

    // "range": [min, max]  OR  "min_priority" + "max_priority"
    let (min_priority, max_priority): (u8, u8) = if let Some(range_val) = m.get("range") {
        match range_val {
            Value::Array(arr) if arr.len() == 2 => {
                let parse_bound = |v: &Value, label: &str| -> Result<u8> {
                    match v {
                        Value::Int(n) if *n >= 0 && *n <= 99 => Ok(*n as u8),
                        Value::Int(n) => Err(IntentError::runtime_error(format!(
                            "Band \"{}\": {} value {} out of range — priority must be 0-99",
                            name, label, n
                        ))),
                        _ => Err(IntentError::type_error(format!(
                            "Band \"{}\": {} must be an integer 0-99",
                            name, label
                        ))),
                    }
                };
                let min = parse_bound(&arr[0], "range[0]")?;
                let max = parse_bound(&arr[1], "range[1]")?;
                (min, max)
            }
            _ => {
                return Err(IntentError::type_error(format!(
                    "Band \"{}\": \"range\" must be an array [min, max]",
                    name
                )))
            }
        }
    } else {
        // Legacy: separate min_priority / max_priority keys
        let min: u8 = match m.get("min_priority") {
            Some(Value::Int(n)) if *n >= 0 && *n <= 99 => *n as u8,
            Some(Value::Int(n)) => {
                return Err(IntentError::runtime_error(format!(
                    "Band \"{}\": min_priority {} out of range 0-99",
                    name, n
                )))
            }
            _ => {
                return Err(IntentError::type_error(format!(
                    "Band \"{}\": missing \"range\" or \"min_priority\" (integer 0-99)",
                    name
                )))
            }
        };
        let max: u8 = match m.get("max_priority") {
            Some(Value::Int(n)) if *n >= 0 && *n <= 99 => *n as u8,
            Some(Value::Int(n)) => {
                return Err(IntentError::runtime_error(format!(
                    "Band \"{}\": max_priority {} out of range 0-99",
                    name, n
                )))
            }
            _ => {
                return Err(IntentError::type_error(format!(
                    "Band \"{}\": missing \"range\" or \"max_priority\" (integer 0-99)",
                    name
                )))
            }
        };
        (min, max)
    };

    if min_priority > max_priority {
        return Err(IntentError::runtime_error(format!(
            "Band \"{}\" has invalid range [{}, {}] — min must be ≤ max",
            name, min_priority, max_priority
        )));
    }

    let concurrency: usize = match m.get("concurrency") {
        Some(Value::Int(n)) if *n >= 1 => *n as usize,
        Some(Value::Int(n)) if *n == 0 => {
            return Err(IntentError::runtime_error(format!(
                "Band \"{}\" has concurrency 0 — must be at least 1",
                name
            )))
        }
        Some(Value::Int(n)) => {
            return Err(IntentError::runtime_error(format!(
                "Band \"{}\" has negative concurrency ({})",
                name, n
            )))
        }
        None => 1,
        _ => {
            return Err(IntentError::type_error(format!(
                "Band \"{}\": concurrency must be an integer",
                name
            )))
        }
    };

    // "poll": ms  OR  "poll_interval": ms (legacy).  Minimum 100ms.
    let poll_key = if m.contains_key("poll") {
        "poll"
    } else {
        "poll_interval"
    };
    let poll_interval_ms: u64 = match m.get(poll_key) {
        Some(Value::Int(n)) if *n >= 100 => *n as u64,
        Some(Value::Int(n)) if *n > 0 => {
            return Err(IntentError::runtime_error(format!(
                "Band \"{}\" has poll interval {}ms — minimum is 100ms",
                name, n
            )))
        }
        Some(Value::Int(_)) => {
            return Err(IntentError::runtime_error(format!(
                "Band \"{}\" has poll interval 0ms — minimum is 100ms",
                name
            )))
        }
        None => 1000,
        _ => {
            return Err(IntentError::type_error(format!(
                "Band \"{}\": poll interval must be an integer (milliseconds)",
                name
            )))
        }
    };

    Ok(BandConfig {
        name,
        min_priority,
        max_priority,
        concurrency,
        poll_interval_ms,
    })
}

/// Validate a slice of BandConfigs.
///
/// Enforces at startup (fail fast, before any threads spawn):
/// - Non-empty list
/// - No overlapping ranges: `band[N].max < band[N+1].min` for adjacent sorted bands
/// - No gaps: bands must collectively span exactly 0-99
///
/// Concurrency > 32 emits a stderr warning (allowed).
fn validate_bands(bands: &[BandConfig]) -> Result<()> {
    if bands.is_empty() {
        return Err(IntentError::runtime_error(
            "Bands configuration cannot be empty".to_string(),
        ));
    }

    // Warn on unusually high concurrency (allowed, not rejected)
    for band in bands {
        if band.concurrency > 32 {
            let stderr = std::io::stderr();
            let mut locked = stderr.lock();
            let _ = std::io::Write::write_all(
                &mut locked,
                format!(
                    "[WARN] Band \"{}\" has concurrency {} — unusually high \
                     (sleeping threads are cheap, but verify this is intentional)\n",
                    band.name, band.concurrency
                )
                .as_bytes(),
            );
        }
    }

    // Check for duplicate band names
    {
        let mut seen = std::collections::HashSet::new();
        for band in bands {
            if !seen.insert(&band.name) {
                return Err(IntentError::runtime_error(format!(
                    "Duplicate band name \"{}\". Band names must be unique.",
                    band.name
                )));
            }
        }
    }

    // Sort a copy by min_priority to check coverage
    let mut sorted: Vec<&BandConfig> = bands.iter().collect();
    sorted.sort_by_key(|b| b.min_priority);

    // Must start at 0
    if sorted[0].min_priority != 0 {
        return Err(IntentError::runtime_error(format!(
            "Bands must cover the full 0-99 range. Missing: 0-{}.",
            sorted[0].min_priority - 1
        )));
    }

    let mut expected_next: u8 = 0;
    for i in 0..sorted.len() {
        let band = sorted[i];
        if band.min_priority < expected_next {
            // Overlap: find the previous band for a helpful message
            let prev = sorted[i - 1];
            return Err(IntentError::runtime_error(format!(
                "Band ranges overlap — \"{}\" ({}-{}) and \"{}\" ({}-{}) both cover priorities {}-{}",
                prev.name,
                prev.min_priority,
                prev.max_priority,
                band.name,
                band.min_priority,
                band.max_priority,
                band.min_priority,
                prev.max_priority,
            )));
        }
        if band.min_priority > expected_next {
            return Err(IntentError::runtime_error(format!(
                "Priority gap — no band covers priorities {}-{}. \
                 Jobs at these priorities would never be processed.",
                expected_next,
                band.min_priority - 1
            )));
        }
        expected_next = band.max_priority.saturating_add(1);
    }

    // Must end at 99
    if expected_next != 100 {
        return Err(IntentError::runtime_error(format!(
            "Bands must cover the full 0-99 range. Missing: {}-99.",
            expected_next
        )));
    }

    Ok(())
}

/// Parse bands and queues from work opts.
///
/// If `opts["bands"]` is present, parse each element as a BandConfig and
/// validate them. Otherwise, fall back to the legacy path: create a single
/// full-range band from `poll_interval` and `concurrency` opts.
///
/// Returns `(bands, queues)`.
fn parse_bands_and_queues(args: &[Value]) -> Result<(Vec<BandConfig>, Option<Vec<String>>)> {
    let (poll_interval, concurrency, queues) = parse_work_opts(args)?;

    let opts_map = args.first().and_then(|v| match v {
        Value::Map(m) => Some(m),
        _ => None,
    });

    if let Some(m) = opts_map {
        if let Some(bands_val) = m.get("bands") {
            let band_arr = match bands_val {
                Value::Array(arr) => arr,
                _ => {
                    return Err(IntentError::type_error(
                        "'bands' must be an array of band config maps".to_string(),
                    ))
                }
            };

            let mut bands = Vec::with_capacity(band_arr.len());
            for item in band_arr {
                match item {
                    Value::Map(bm) => bands.push(parse_band_config(bm)?),
                    _ => {
                        return Err(IntentError::type_error(
                            "each element of 'bands' must be a map".to_string(),
                        ))
                    }
                }
            }

            validate_bands(&bands)?;
            return Ok((bands, queues));
        }
    }

    // Legacy fallback: single full-range band
    Ok((
        vec![BandConfig {
            name: "normal".to_string(),
            min_priority: 0,
            max_priority: 99,
            concurrency,
            poll_interval_ms: poll_interval,
        }],
        queues,
    ))
}

/// Spawn a single worker task registered with the ConcurrencyRuntime.
///
/// Returns `(TaskHandle, cancel_arc)` — the `Arc<AtomicBool>` is the cooperative
/// cancellation flag for this worker, stored in `JOB_RUNTIME.band_cancel_arcs`
/// so `scale_workers` can cancel excess threads without going through RUNTIME.
fn spawn_worker_task(
    kv_handle: Value,
    band: BandConfig,
    queues: Option<Vec<String>>,
) -> Result<(Value, Arc<CancelToken>)> {
    // Extract serializable KvHandleInfo — Value is not Send due to Rc internals.
    let kv_info = extract_kv_handle_info(&kv_handle)?;

    RUNTIME.try_reap_expired_tasks();
    check_task_limit()?;

    let cancelled = Arc::new(CancelToken::new());
    let cancel_clone = Arc::clone(&cancelled);
    let task_id = RUNTIME.register_task(Arc::clone(&cancelled))?;
    RUNTIME.active_tasks.fetch_add(1, AtomicOrdering::Release);
    // Safe: task_id was just returned by register_task(), so it must be in the registry
    let arcs = RUNTIME
        .get_task_arcs(task_id)?
        .expect("task just registered must exist");

    std::thread::spawn(move || {
        CURRENT_CANCEL_TOKEN.with(|cell| {
            *cell.borrow_mut() = Some(cancelled);
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            worker_loop(kv_info, band, queues);
            Ok(Value::Unit)
        }));
        finalize_task(result, &arcs.inner, &arcs.completed_notify);
    });

    Ok((Value::TaskHandle(task_id), cancel_clone))
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
    // Priority was validated at enqueue time — default 50 is defensive
    // for any jobs missing the field (shouldn't happen normally)
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
// Control-socket helpers — called by src/control_socket.rs
// ============================================================================

/// Return a status snapshot of the job worker system.
///
/// Same logic as the `worker_status` NativeFunction; extracted so the control
/// socket can call it directly without going through the stdlib function table.
pub(crate) fn worker_status_impl() -> crate::error::Result<Value> {
    let pending_count = JOB_RUNTIME
        .get_or_init_kv()
        .and_then(|kv| kv::kv_list(&kv, Some("jobs:pending:")))
        .map(|keys| keys.len() as i64)
        .unwrap_or(0);

    let active_bands = JOB_RUNTIME
        .active_bands
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();

    let stats_map_guard = JOB_RUNTIME
        .band_stats
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let task_ids_guard = JOB_RUNTIME.band_worker_task_ids.lock();

    let mut band_entries = Vec::new();
    for band in &active_bands {
        let mut entry = HashMap::new();
        entry.insert("name".to_string(), Value::String(band.name.clone()));
        entry.insert(
            "min_priority".to_string(),
            Value::Int(band.min_priority as i64),
        );
        entry.insert(
            "max_priority".to_string(),
            Value::Int(band.max_priority as i64),
        );
        entry.insert(
            "concurrency".to_string(),
            Value::Int(band.concurrency as i64),
        );
        entry.insert(
            "poll_interval_ms".to_string(),
            Value::Int(band.poll_interval_ms as i64),
        );

        let worker_count = task_ids_guard
            .as_ref()
            .map(|m| m.get(&band.name).map(|v| v.len()).unwrap_or(0))
            .unwrap_or(0);
        entry.insert("workers".to_string(), Value::Int(worker_count as i64));

        if let Some(stats) = stats_map_guard.get(&band.name) {
            entry.insert(
                "completed".to_string(),
                Value::Int(stats.completed.load(std::sync::atomic::Ordering::Relaxed) as i64),
            );
            entry.insert(
                "failed".to_string(),
                Value::Int(stats.failed.load(std::sync::atomic::Ordering::Relaxed) as i64),
            );
            entry.insert(
                "active".to_string(),
                Value::Int(stats.active.load(std::sync::atomic::Ordering::Relaxed) as i64),
            );
            let total_ms = stats
                .total_duration_ms
                .load(std::sync::atomic::Ordering::Relaxed);
            let completed = stats.completed.load(std::sync::atomic::Ordering::Relaxed);
            let avg_ms = if completed > 0 {
                total_ms / completed
            } else {
                0
            };
            entry.insert("avg_duration_ms".to_string(), Value::Int(avg_ms as i64));
        } else {
            entry.insert("completed".to_string(), Value::Int(0));
            entry.insert("failed".to_string(), Value::Int(0));
            entry.insert("active".to_string(), Value::Int(0));
            entry.insert("avg_duration_ms".to_string(), Value::Int(0));
        }

        band_entries.push(Value::Map(entry));
    }

    if let Ok(kv) = JOB_RUNTIME.get_or_init_kv() {
        refresh_paused_cache(&kv, true);
    }
    let paused_queue_names: Vec<Value> =
        read_paused_cache().into_iter().map(Value::String).collect();

    let mut result = HashMap::new();
    result.insert("bands".to_string(), Value::Array(band_entries));
    result.insert("pending".to_string(), Value::Int(pending_count));
    result.insert(
        "paused_queues".to_string(),
        Value::Array(paused_queue_names),
    );
    Ok(Value::Map(result))
}

/// Scale the number of worker threads for a named band up or down.
///
/// Same logic as the `scale_workers` NativeFunction; extracted so the control
/// socket can call it directly without going through the stdlib function table.
pub(crate) fn scale_workers_impl(
    band_name: &str,
    target_count: usize,
) -> crate::error::Result<Value> {
    let band_config = {
        let active = JOB_RUNTIME
            .active_bands
            .lock()
            .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;
        active.iter().find(|b| b.name == band_name).cloned()
    };
    let band_config = match band_config {
        Some(b) => b,
        None => {
            return Err(IntentError::runtime_error(format!(
                "scale_workers(): band '{}' not found. Call work_async() or work_jobs() first.",
                band_name
            )))
        }
    };

    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    let queues = JOB_RUNTIME
        .active_queues
        .lock()
        .map(|q| q.clone())
        .unwrap_or(None);

    // Lock discipline: task_ids before cancel_arcs.
    let mut task_ids_map = JOB_RUNTIME
        .band_worker_task_ids
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;
    let mut cancel_map = JOB_RUNTIME
        .band_cancel_arcs
        .lock()
        .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?;

    let arcs = cancel_map.entry(band_name.to_string()).or_default();
    let ids = task_ids_map.entry(band_name.to_string()).or_default();
    let current_count = arcs.len();

    if target_count > current_count {
        for _ in current_count..target_count {
            match spawn_worker_task(kv_handle.clone(), band_config.clone(), queues.clone()) {
                Ok((Value::TaskHandle(id), cancel_arc)) => {
                    ids.push(id);
                    arcs.push(cancel_arc);
                }
                Ok((_, _)) => {
                    return Err(IntentError::runtime_error(
                        "spawn_worker_task returned unexpected value type".to_string(),
                    ));
                }
                Err(e) => return Err(e),
            }
        }
    } else if target_count < current_count {
        for arc in arcs.drain(target_count..) {
            arc.cancel();
        }
        if ids.len() > target_count {
            ids.drain(target_count..ids.len());
        }
    }

    // Update active_bands concurrency to reflect the new count.
    {
        let mut active = JOB_RUNTIME
            .active_bands
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Some(band) = active.iter_mut().find(|b| b.name == band_name) {
            band.concurrency = target_count;
        }
    }

    Ok(Value::ok(Value::Unit))
}

fn set_queue_paused(queue_name: &str, paused: bool) -> crate::error::Result<Value> {
    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
    if paused {
        let now_ts = now_nanos_str();
        kv::kv_set(
            &kv_handle,
            &format!("jobs:paused:{}", queue_name),
            &Value::String(now_ts.clone()),
            None,
        )?;
        JOB_RUNTIME
            .paused_queues
            .write()
            .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?
            .insert(queue_name.to_string());
        if let Ok(mut ts) = JOB_RUNTIME.paused_cache_updated_at.lock() {
            *ts = std::time::Instant::now();
        }
        emit_job_event(
            "queue.paused",
            &[
                ("queue", Value::String(queue_name.to_string())),
                ("paused_at", Value::String(now_ts)),
            ],
        );
    } else {
        kv::kv_del(&kv_handle, &format!("jobs:paused:{}", queue_name))?;
        JOB_RUNTIME
            .paused_queues
            .write()
            .map_err(|e| IntentError::runtime_error(format!("Lock error: {}", e)))?
            .remove(queue_name);
        if let Ok(mut ts) = JOB_RUNTIME.paused_cache_updated_at.lock() {
            *ts = std::time::Instant::now();
        }
        emit_job_event(
            "queue.resumed",
            &[("queue", Value::String(queue_name.to_string()))],
        );
    }
    Ok(Value::ok(Value::Unit))
}

pub(crate) fn pause_queue_impl(queue_name: &str) -> crate::error::Result<Value> {
    set_queue_paused(queue_name, true)
}

pub(crate) fn resume_queue_impl(queue_name: &str) -> crate::error::Result<Value> {
    set_queue_paused(queue_name, false)
}

const PAUSE_CACHE_STALE_SECS: u64 = 5;

fn mark_pause_cache_stale() {
    if let Ok(mut ts) = JOB_RUNTIME.paused_cache_updated_at.lock() {
        *ts =
            std::time::Instant::now() - std::time::Duration::from_secs(PAUSE_CACHE_STALE_SECS + 1);
    }
}

fn refresh_paused_cache(kv_handle: &Value, force: bool) {
    // Single critical section: check staleness + claim refresh atomically.
    let should_refresh = match JOB_RUNTIME.paused_cache_updated_at.lock() {
        Ok(mut ts) => {
            if !force && ts.elapsed().as_secs() <= PAUSE_CACHE_STALE_SECS {
                false
            } else {
                *ts = std::time::Instant::now(); // claim refresh
                true
            }
        }
        Err(_) => true, // poisoned → refresh
    };
    if !should_refresh {
        return;
    }
    match kv::kv_list(kv_handle, Some("jobs:paused:")) {
        Ok(keys) => {
            let refreshed = keys
                .iter()
                .filter_map(|k| k.strip_prefix("jobs:paused:"))
                .map(str::to_string)
                .collect();
            if let Ok(mut paused) = JOB_RUNTIME.paused_queues.write() {
                *paused = refreshed;
            }
        }
        Err(_) => {
            // Roll back timestamp so next caller retries
            if let Ok(mut ts) = JOB_RUNTIME.paused_cache_updated_at.lock() {
                *ts = std::time::Instant::now()
                    - std::time::Duration::from_secs(PAUSE_CACHE_STALE_SECS + 1);
            }
        }
    }
}

/// Read the cached paused queue names (sorted).
fn read_paused_cache() -> Vec<String> {
    JOB_RUNTIME
        .paused_queues
        .read()
        .map(|p| {
            let mut names: Vec<String> = p.iter().cloned().collect();
            names.sort();
            names
        })
        .unwrap_or_default()
}

pub(crate) fn is_queue_paused(queue_name: &str, kv_handle: &Value) -> bool {
    refresh_paused_cache(kv_handle, false);
    JOB_RUNTIME
        .paused_queues
        .read()
        .map(|p| p.contains(queue_name))
        .unwrap_or(false)
}

/// Return nanosecond epoch as a string (used for paused_at timestamp).
fn now_nanos_str() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
        .to_string()
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
            requires: None,
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
    // @signature enqueue(batch_handle: Map, job_name: String, args: Map) -> Result<Unit, String>
    // Enqueue a background job for processing, or buffer a job into an open batch.
    //
    // Two-arg form: enqueue(job_name, args) — writes job to KV immediately and returns
    // the job ID string. Three-arg form: enqueue(batch_handle, job_name, args) — buffers
    // the job in memory until seal() is called. Does not write to KV; returns Ok(Unit)
    // instead of a job ID (the job has no KV identity until sealed).
    // @param job_name The registered job name (e.g., "SendEmail") — 2-arg form
    // @param args A map of arguments to pass to the job's perform block
    // @param batch_handle The batch handle returned by batch() — 3-arg form (first positional)
    // @returns 2-arg: Result<String, String> containing the job ID; 3-arg: Result<Unit, String>
    // @gotcha The typechecker cannot distinguish the 2-arg and 3-arg overloads at compile time. Passing enqueue("name", string, map) will typecheck but fail at runtime. This is a known gap — the runtime validates argument types strictly.
    // @example enqueue("SendEmail", map { "to": "alice@example.com" }) ~ "Enqueue an email job"
    // @example enqueue(b, "ProcessRow", map { "row_id": row_id }) ~ "Buffer job into a batch"
    module.insert(
        "enqueue".to_string(),
        Value::NativeFunction {
            name: "enqueue".to_string(),
            arity: 2,
            max_arity: 3,
            requires: Some(crate::interpreter::RuntimeCapability::JobEnqueue),
            func: |args| {
                match args.len() {
                    2 => {
                        // Normal enqueue: (job_name, args)
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
                        enqueue_internal(&job_name, payload, &timestamp_key(), None, None)
                    }
                    3 => {
                        // Batch enqueue: (batch_handle, job_name, args)
                        let batch_id = match &args[0] {
                            Value::Map(m) => match m.get("_batch_id") {
                                Some(Value::String(bid)) => bid.clone(),
                                _ => {
                                    return Err(IntentError::type_error(
                                        "enqueue() first argument must be a batch handle (map with _batch_id)".to_string(),
                                    ))
                                }
                            },
                            _ => {
                                return Err(IntentError::type_error(
                                    "enqueue() with 3 arguments requires a batch handle as first argument".to_string(),
                                ))
                            }
                        };
                        let job_name = match &args[1] {
                            Value::String(s) => s.clone(),
                            _ => {
                                return Err(IntentError::type_error(
                                    "enqueue() second argument must be a string job name".to_string(),
                                ))
                            }
                        };
                        let payload = match &args[2] {
                            Value::Map(_) => args[2].clone(),
                            _ => {
                                return Err(IntentError::type_error(
                                    "enqueue() third argument must be a map".to_string(),
                                ))
                            }
                        };
                        let payload_json = serde_json::to_string(
                            &kv::value_to_json_public(&payload),
                        )
                        .map_err(|e| {
                            IntentError::runtime_error(format!(
                                "Failed to serialize batch payload: {}",
                                e
                            ))
                        })?;
                        // Validate job type is registered before buffering (#5)
                        if JOB_RUNTIME.get_job(&job_name)?.is_none() {
                            return Err(IntentError::runtime_error(format!(
                                "Unknown job type '{}' — make sure it is defined with a job block before enqueueing",
                                job_name
                            )));
                        }
                        let mut batches = BATCH_RUNTIME.batches.lock().map_err(|e| {
                            IntentError::runtime_error(format!("Batch lock error: {}", e))
                        })?;
                        match batches.get_mut(&batch_id) {
                            Some(batch) if batch.status == BatchStatus::Sealing => {
                                Err(IntentError::runtime_error(format!(
                                    "Batch '{}' is being sealed — no more enqueues accepted",
                                    batch_id
                                )))
                            }
                            Some(batch) => {
                                batch.buffered.push(BufferedJob {
                                    job_type: job_name,
                                    payload_json,
                                    flushed: false,
                                });
                                // Return Unit — the job has no KV identity until seal() (#4)
                                Ok(Value::ok(Value::Unit))
                            }
                            None => Err(IntentError::runtime_error(format!(
                                "Batch '{}' not found or already sealed",
                                batch_id
                            ))),
                        }
                    }
                    _ => Err(IntentError::type_error(
                        "enqueue() requires 2-3 arguments: (job_name, args) or (batch, job_name, args)".to_string(),
                    )),
                }
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
            requires: None,
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
            requires: None,
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
            requires: Some(crate::interpreter::RuntimeCapability::JobEnqueue),
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
                enqueue_internal(&job_name, payload, &pending_ts, Some(&pending_ts), None)
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
            requires: Some(crate::interpreter::RuntimeCapability::JobEnqueue),
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
                enqueue_internal(&job_name, payload, &pending_ts, Some(&pending_ts), None)
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
            requires: Some(RuntimeCapability::JobWorkers),
            func: |args| {
                let (bands, queues) = parse_bands_and_queues(args)?;
                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                // Store active bands in the runtime for worker_status
                if let Ok(mut active) = JOB_RUNTIME.active_bands.lock() {
                    *active = bands.clone();
                }
                let mut handles = Vec::new();
                let mut band_task_ids: HashMap<String, Vec<u64>> = HashMap::new();
                let mut band_cancel_arcs: HashMap<String, Vec<Arc<CancelToken>>> = HashMap::new();
                // Store active queues so scale_workers can reuse the same filter
                if let Ok(mut aq) = JOB_RUNTIME.active_queues.lock() {
                    *aq = queues.clone();
                }
                for band in &bands {
                    let mut ids = Vec::new();
                    let mut arcs = Vec::new();
                    for _ in 0..band.concurrency {
                        match spawn_worker_task(kv_handle.clone(), band.clone(), queues.clone()) {
                            Ok((Value::TaskHandle(id), cancel_arc)) => {
                                ids.push(id);
                                arcs.push(cancel_arc);
                                handles.push(Value::TaskHandle(id));
                            }
                            Ok((h, cancel_arc)) => {
                                arcs.push(cancel_arc);
                                handles.push(h);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    band_task_ids.insert(band.name.clone(), ids);
                    band_cancel_arcs.insert(band.name.clone(), arcs);
                }
                if let Ok(mut task_ids) = JOB_RUNTIME.band_worker_task_ids.lock() {
                    *task_ids = band_task_ids;
                }
                if let Ok(mut ca) = JOB_RUNTIME.band_cancel_arcs.lock() {
                    *ca = band_cancel_arcs;
                }
                // Start control socket — lives until process exit or stop_control_socket().
                // For work_async (non-blocking), the socket persists with the app process.
                // For work_jobs (blocking), stop_control_socket() is called on Ctrl-C.
                crate::control_socket::start_control_socket();
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
            requires: Some(RuntimeCapability::JobWorkers),
            func: |args| {
                let (bands, queues) = parse_bands_and_queues(args)?;
                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;

                // Store active bands and queues (same as work_async, so scale_workers works)
                if let Ok(mut active) = JOB_RUNTIME.active_bands.lock() {
                    *active = bands.clone();
                }
                if let Ok(mut aq) = JOB_RUNTIME.active_queues.lock() {
                    *aq = queues.clone();
                }

                // Spawn all band workers (background threads), collect cancel arcs
                let mut band_task_ids: HashMap<String, Vec<u64>> = HashMap::new();
                let mut band_cancel_arcs_map: HashMap<String, Vec<Arc<CancelToken>>> =
                    HashMap::new();
                let mut all_cancel_arcs: Vec<Arc<CancelToken>> = Vec::new();

                for band in &bands {
                    let mut ids = Vec::new();
                    let mut arcs = Vec::new();
                    for _ in 0..band.concurrency {
                        match spawn_worker_task(kv_handle.clone(), band.clone(), queues.clone()) {
                            Ok((Value::TaskHandle(id), cancel_arc)) => {
                                ids.push(id);
                                all_cancel_arcs.push(Arc::clone(&cancel_arc));
                                arcs.push(cancel_arc);
                            }
                            Ok((_, cancel_arc)) => {
                                all_cancel_arcs.push(Arc::clone(&cancel_arc));
                                arcs.push(cancel_arc);
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    band_task_ids.insert(band.name.clone(), ids);
                    band_cancel_arcs_map.insert(band.name.clone(), arcs);
                }

                if let Ok(mut task_ids) = JOB_RUNTIME.band_worker_task_ids.lock() {
                    *task_ids = band_task_ids;
                }
                if let Ok(mut ca) = JOB_RUNTIME.band_cancel_arcs.lock() {
                    *ca = band_cancel_arcs_map;
                }

                crate::control_socket::start_control_socket();

                // Set up Ctrl-C handler — sets a shared shutdown flag
                let shutdown = Arc::new(AtomicBool::new(false));
                let shutdown_clone = Arc::clone(&shutdown);
                ctrlc::set_handler(move || {
                    shutdown_clone.store(true, AtomicOrdering::Release);
                })
                .map_err(|e| {
                    IntentError::runtime_error(format!("Failed to set Ctrl-C handler: {}", e))
                })?;

                // Block until Ctrl-C, then cooperatively cancel ALL workers
                // (including any spawned later via scale_workers)
                loop {
                    if shutdown.load(AtomicOrdering::Acquire) {
                        // Signal workers from JOB_RUNTIME (includes scaled workers)
                        if let Ok(ca) = JOB_RUNTIME.band_cancel_arcs.lock() {
                            for arcs in ca.values() {
                                for arc in arcs {
                                    arc.cancel();
                                }
                            }
                        }
                        // Also signal the initial arcs (belt and suspenders)
                        for arc in &all_cancel_arcs {
                            arc.cancel();
                        }
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }

                // Wait briefly for workers to finish current jobs
                std::thread::sleep(std::time::Duration::from_millis(500));

                crate::control_socket::stop_control_socket();
                Ok(Value::Unit)
            },
        },
    );

    // @ntnt scale_workers
    // @module std/jobs
    // @signature scale_workers(band_name: String, count: Int) -> Result<Unit, String>
    // Scale the number of worker threads for a named band up or down.
    //
    // Adds workers (up to count) or cancels excess workers cooperatively.
    // Only takes effect after work_async() has been called to initialise
    // the band pool. Returns Err if the band name is not found.
    // @param band_name The band to scale (e.g. "critical", "high", "normal", "low")
    // @param count Target number of concurrent workers for this band (>= 1)
    // @returns Ok(Unit) on success, Err(String) if band not found or count < 1
    // @see_also work_async, worker_status
    // @example scale_workers("critical", 8) ~ "Scale critical band to 8 workers"
    // @example scale_workers("normal", 1) ~ "Scale normal band down to 1 worker"
    module.insert(
        "scale_workers".to_string(),
        Value::NativeFunction {
            name: "scale_workers".to_string(),
            arity: 2,
            max_arity: 2,
            requires: Some(RuntimeCapability::JobWorkers),
            func: |args| {
                let band_name = match args.first() {
                    Some(Value::String(s)) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "scale_workers() requires a band name string".to_string(),
                        ))
                    }
                };
                let target_count: usize = match args.get(1) {
                    Some(Value::Int(n)) if *n >= 1 && (*n as u64) <= usize::MAX as u64 => {
                        *n as usize
                    }
                    Some(Value::Int(n)) if *n < 1 => {
                        return Err(IntentError::runtime_error(format!(
                            "scale_workers() count must be >= 1, got {}",
                            n
                        )))
                    }
                    Some(Value::Int(n)) => {
                        return Err(IntentError::runtime_error(format!(
                            "scale_workers() count {} exceeds platform maximum",
                            n
                        )))
                    }
                    _ => {
                        return Err(IntentError::type_error(
                            "scale_workers() requires an integer count".to_string(),
                        ))
                    }
                };

                scale_workers_impl(&band_name, target_count)?;
                Ok(Value::ok(Value::Unit))
            },
        },
    );

    // @ntnt worker_status
    // @module std/jobs
    // @signature worker_status() -> Map
    // Return a status snapshot of the job worker system.
    //
    // Returns a map with per-band stats and a system-wide pending count.
    // Requires work_async() to have been called to populate band data.
    // @returns Map with keys: "bands" (Array of per-band stat maps), "pending" (Int total pending jobs)
    // @see_also work_async, scale_workers
    // @example worker_status() ~ "Get current worker and queue stats"
    module.insert(
        "worker_status".to_string(),
        Value::NativeFunction {
            name: "worker_status".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| worker_status_impl(),
        },
    );

    // @ntnt pause_queue
    // @module std/jobs
    // @signature pause_queue(queue: String) -> Result<Unit, String>
    // Pause a queue — workers stop executing jobs from it.
    // @example pause_queue("emails") ~ "Stop processing the emails queue"
    module.insert(
        "pause_queue".to_string(),
        Value::NativeFunction {
            name: "pause_queue".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "pause_queue() requires 1 argument (queue)".to_string(),
                    ));
                }
                let queue = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "pause_queue() requires a string queue name".to_string(),
                        ))
                    }
                };
                pause_queue_impl(&queue)
            },
        },
    );

    // @ntnt resume_queue
    // @module std/jobs
    // @signature resume_queue(queue: String) -> Result<Unit, String>
    // Resume a paused queue — workers resume claiming and executing jobs from it.
    // @example resume_queue("emails") ~ "Resume processing the emails queue"
    module.insert(
        "resume_queue".to_string(),
        Value::NativeFunction {
            name: "resume_queue".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "resume_queue() requires 1 argument (queue)".to_string(),
                    ));
                }
                let queue = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "resume_queue() requires a string queue name".to_string(),
                        ))
                    }
                };
                resume_queue_impl(&queue)
            },
        },
    );

    // @ntnt queue_status
    // @module std/jobs
    // @signature queue_status(queue: String) -> Map
    // Get the current status of a queue, including whether it is paused.
    // @example queue_status("emails") ~ "Check if emails queue is paused"
    module.insert(
        "queue_status".to_string(),
        Value::NativeFunction {
            name: "queue_status".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "queue_status() requires 1 argument (queue)".to_string(),
                    ));
                }
                let queue = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "queue_status() requires a string queue name".to_string(),
                        ))
                    }
                };

                let mut result = HashMap::new();
                result.insert("name".to_string(), Value::String(queue.clone()));

                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                let kv_key = format!("jobs:paused:{}", queue);
                let paused_val = kv::kv_get(&kv_handle, &kv_key)?;

                match paused_val {
                    Value::String(ts) if !ts.is_empty() => {
                        result.insert("paused".to_string(), Value::Bool(true));
                        result.insert("paused_at".to_string(), Value::String(ts));
                    }
                    _ => {
                        result.insert("paused".to_string(), Value::Bool(false));
                    }
                }

                Ok(Value::Map(result))
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
            requires: None,
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
            requires: None,
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
            requires: None,
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

                // drain_jobs() is a test helper. It should not re-evaluate the
                // application source file, because top-level drain_jobs() would recurse
                // through worker bootstrap. Tests register jobs inline, so a bare
                // interpreter is sufficient here.
                let mut drain_interp = crate::interpreter::Interpreter::new();

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

                    match execute_in_worker(&mut drain_interp, &def, &payload) {
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
            requires: None,
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
            requires: None,
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
            requires: None,
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
            requires: None,
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
            requires: Some(crate::interpreter::RuntimeCapability::JobEnqueue),
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
                    let result = enqueue_internal(&job_name, item, &ts, None, None).map_err(|e| {
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

    // @ntnt batch
    // @module std/jobs
    // @signature batch(name: String, opts?: Map) -> Map
    // Create a new job batch that buffers enqueues until sealed.
    //
    // Returns a batch handle (Map with _batch_id field). Jobs enqueued via
    // enqueue(handle, job_name, args) are buffered in memory until seal() is called.
    // Batches are not durable until sealed — a process crash loses buffered jobs.
    // @param name Human-readable name for the batch (for observability)
    // @param opts Optional map with "on_success", "on_complete", "on_death" callback functions
    // @returns Batch handle map with _batch_id field
    // @gotcha Callbacks (on_success, on_complete, on_death) are accepted for API forward-compatibility but are not executed until Phase 2. Providing them now is safe and encouraged for future-proofing.
    // @example batch("csv-import", map { "on_success": fn(s) { print("done") } }) ~ "Create batch with callback"
    // @example batch("daily-report") ~ "Create batch without callbacks"
    // @see_also seal, batch_status, enqueue
    module.insert(
        "batch".to_string(),
        Value::NativeFunction {
            name: "batch".to_string(),
            arity: 1,
            max_arity: 2,
            requires: Some(crate::interpreter::RuntimeCapability::JobEnqueue),
            func: |args| {
                if args.is_empty() || args.len() > 2 {
                    return Err(IntentError::type_error(
                        "batch() requires 1-2 arguments (name, opts?)".to_string(),
                    ));
                }
                let name = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "batch() first argument must be a string name".to_string(),
                        ))
                    }
                };
                let mut callback_names = Vec::new();
                if args.len() == 2 {
                    match &args[1] {
                        Value::Map(opts) => {
                            for key in ["on_success", "on_complete", "on_death"] {
                                if opts.contains_key(key) {
                                    callback_names.push(key.to_string());
                                }
                            }
                        }
                        _ => {
                            return Err(IntentError::type_error(
                                "batch() second argument must be a map".to_string(),
                            ))
                        }
                    }
                }
                let batch_id = Uuid::new_v4().to_string();
                let state = BatchState {
                    id: batch_id.clone(),
                    name,
                    callback_names,
                    buffered: Vec::new(),
                    created_at: timestamp_key(),
                    status: BatchStatus::Open,
                };
                let mut batches = BATCH_RUNTIME
                    .batches
                    .lock()
                    .map_err(|e| IntentError::runtime_error(format!("Batch lock error: {}", e)))?;
                // Lazily prune abandoned Open batches older than 1 hour (#6).
                // Never prune Sealing batches — they're actively being flushed.
                let one_hour_nanos: u128 = 3_600_000_000_000;
                let now_nanos = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos();
                batches.retain(|_, s| {
                    if s.status == BatchStatus::Sealing {
                        return true; // never prune active seals
                    }
                    s.created_at.parse::<u128>().map_or(true, |created| {
                        now_nanos.saturating_sub(created) < one_hour_nanos
                    })
                });
                batches.insert(batch_id.clone(), state);
                let mut handle = HashMap::new();
                handle.insert("_batch_id".to_string(), Value::String(batch_id));
                Ok(Value::Map(handle))
            },
        },
    );

    // @ntnt seal
    // @module std/jobs
    // @signature seal(batch_handle: Map) -> Result<Unit, String>
    // Seal a batch, flushing all buffered jobs to KV in order.
    //
    // After seal(), no more jobs can be buffered. Workers can immediately begin
    // claiming the flushed jobs. Idempotent — sealing an already-sealed batch
    // is a no-op. Empty batches (0 jobs) are immediately marked complete.
    // Note: seal writes metadata then individual jobs sequentially (not transactionally).
    // On failure, already-flushed jobs are tracked and skipped on retry.
    // @param batch_handle The batch handle returned by batch()
    // @returns Result<Unit, String>
    // @gotcha Callbacks (on_success, on_complete, on_death) registered via batch() are accepted for API forward-compatibility but are not executed until Phase 2.
    // @gotcha If enqueue_internal fails mid-write for a single job (e.g., data key written but pending key not), that job's flushed flag stays false and retry will re-enqueue with a new ID, potentially creating a duplicate. This is a known limitation of the non-transactional KV backend.
    // @example seal(b) ~ "Seal a batch after buffering all jobs"
    // @see_also batch, batch_status
    module.insert(
        "seal".to_string(),
        Value::NativeFunction {
            name: "seal".to_string(),
            arity: 1,
            max_arity: 1,
            requires: Some(crate::interpreter::RuntimeCapability::JobEnqueue),
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "seal() requires 1 argument (batch_handle)".to_string(),
                    ));
                }
                let batch_id = match &args[0] {
                    Value::Map(m) => match m.get("_batch_id") {
                        Some(Value::String(bid)) => bid.clone(),
                        _ => {
                            return Err(IntentError::type_error(
                                "seal() requires a batch handle (map with _batch_id)".to_string(),
                            ))
                        }
                    },
                    _ => {
                        return Err(IntentError::type_error(
                            "seal() requires a batch handle".to_string(),
                        ))
                    }
                };

                // Step 1: Under lock — mark batch as "sealing" to block concurrent enqueues.
                // The batch stays in the map with Sealing status so other threads see it.
                let mut batch_state = {
                    let mut batches = BATCH_RUNTIME
                        .batches
                        .lock()
                        .map_err(|e| IntentError::runtime_error(format!("Batch lock error: {}", e)))?;

                    match batches.get_mut(&batch_id) {
                        Some(batch) if batch.status == BatchStatus::Sealing => {
                            // Another thread is already sealing this batch
                            return Err(IntentError::runtime_error(format!(
                                "Batch '{}' is being sealed by another thread",
                                batch_id
                            )));
                        }
                        Some(batch) => {
                            // Mark as sealing (stays in map so concurrent enqueues see it)
                            batch.status = BatchStatus::Sealing;
                            // Clone out the data we need for KV work
                            BatchState {
                                id: batch.id.clone(),
                                name: batch.name.clone(),
                                callback_names: batch.callback_names.clone(),
                                buffered: batch.buffered.clone(),
                                created_at: batch.created_at.clone(),
                                status: BatchStatus::Sealing,
                            }
                        }
                        None => {
                            // Not in memory — check if already sealed in KV (idempotent)
                            drop(batches);
                            let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                            let meta_key = format!("jobs:batch:{}", batch_id);
                            return match kv::kv_get(&kv_handle, &meta_key)? {
                                Value::Map(m) => {
                                    // Only treat as idempotent if status is terminal
                                    let status = m.get("status").and_then(|v| match v {
                                        Value::String(s) => Some(s.as_str()),
                                        _ => None,
                                    });
                                    match status {
                                        Some("sealed") | Some("complete") => Ok(Value::ok(Value::Unit)),
                                        Some("sealing") => Err(IntentError::runtime_error(format!(
                                            "Batch '{}' has incomplete seal (status='sealing'). Previous seal may have crashed mid-flush.",
                                            batch_id
                                        ))),
                                        _ => Err(IntentError::runtime_error(format!(
                                            "Batch '{}' has unexpected status in KV",
                                            batch_id
                                        ))),
                                    }
                                }
                                _ => Err(IntentError::runtime_error(format!(
                                    "Batch '{}' not found",
                                    batch_id
                                ))),
                            };
                        }
                    }
                };
                // Lock is dropped here — KV work proceeds without holding global lock.
                // Batch remains in map with Sealing status, blocking concurrent enqueues.

                // Step 2: All KV work outside the lock.
                // On failure, re-lock and reinsert the batch for retry (#2).
                let kv_result: Result<()> = (|| {
                    let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                    let meta_key = format!("jobs:batch:{}", batch_id);
                    let total = batch_state.buffered.len() as i64;

                    if total == 0 {
                        // Empty batch → immediately complete
                        let mut meta = build_batch_meta(
                            &batch_id, &batch_state.name, &batch_state.created_at,
                            "complete", 0, 0,
                        );
                        meta.insert("fired_success".to_string(), Value::Bool(true));
                        meta.insert("fired_complete".to_string(), Value::Bool(true));
                        let now = timestamp_key();
                        meta.insert("sealed_at".to_string(), Value::String(now.clone()));
                        meta.insert("completed_at".to_string(), Value::String(now));
                        kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), None)?;
                        return Ok(());
                    }

                    // Pre-parse ALL payloads before any KV writes (#7).
                    // Fail immediately on parse error rather than substituting empty maps.
                    let base_nanos = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos();
                    let mut prepared: Vec<(usize, String, Value, String)> =
                        Vec::with_capacity(batch_state.buffered.len());
                    for i in 0..batch_state.buffered.len() {
                        if batch_state.buffered[i].flushed {
                            continue;
                        }
                        let job = &batch_state.buffered[i];
                        let ts = format!("{:020}", base_nanos + i as u128);
                        let payload = serde_json::from_str::<serde_json::Value>(&job.payload_json)
                            .map(|j| kv::json_to_value_public(&j))
                            .map_err(|e| {
                                IntentError::runtime_error(format!(
                                    "Failed to parse buffered payload for job '{}': {}",
                                    job.job_type, e
                                ))
                            })?;
                        prepared.push((i, job.job_type.clone(), payload, ts));
                    }

                    // Write "sealing" metadata BEFORE flushing jobs.
                    // This ensures batch_status() can always find the batch even if
                    // a crash occurs mid-flush (prevents orphaned jobs with batch_id
                    // but no batch metadata).
                    let meta = build_batch_meta(
                        &batch_id, &batch_state.name, &batch_state.created_at,
                        "sealing", total, total,
                    );
                    kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), None)?;

                    // Flush jobs to KV.
                    // Mark each flushed so retries skip already-written jobs.
                    for (idx, job_type, payload, ts) in &prepared {
                        enqueue_internal(job_type, payload.clone(), ts, None, Some(&batch_id))?;
                        batch_state.buffered[*idx].flushed = true;
                    }

                    // Update metadata to "sealed" after all jobs are durable.
                    let sealed_at = timestamp_key();
                    let mut meta = build_batch_meta(
                        &batch_id, &batch_state.name, &batch_state.created_at,
                        "sealed", total, total,
                    );
                    meta.insert("sealed_at".to_string(), Value::String(sealed_at));
                    kv::kv_set(&kv_handle, &meta_key, &Value::Map(meta), None)?;

                    Ok(())
                })();

                match kv_result {
                    Ok(_) => {
                        // Success — remove the batch from the map (no longer needed in memory).
                        // Use poison recovery so a poisoned mutex doesn't leave the batch
                        // permanently stuck in Sealing state after a successful seal.
                        let mut batches = BATCH_RUNTIME
                            .batches
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        batches.remove(&batch_id);
                        Ok(Value::ok(Value::Unit))
                    }
                    Err(e) => {
                        // KV writes failed — clean up both KV and in-memory state for retry.
                        // Use the same kv_handle from the closure scope to avoid re-init failures.
                        let meta_key = format!("jobs:batch:{}", batch_id);

                        // 1. Revert KV metadata: delete the "sealing" record so
                        //    batch_status() doesn't report a phantom seal, and
                        //    a process restart doesn't leave an unrecoverable state.
                        //    If cleanup fails, return that error — don't leave KV and
                        //    memory in divergent states.
                        match JOB_RUNTIME.get_or_init_kv() {
                            Ok(kv) => {
                                if let Err(cleanup_err) = kv::kv_del(&kv, &meta_key) {
                                    // KV cleanup failed — don't reset in-memory state either,
                                    // so KV ("sealing") and memory (Sealing) stay consistent.
                                    return Err(IntentError::runtime_error(format!(
                                        "Seal failed ({}), and KV cleanup also failed ({}). \
                                         Batch '{}' is stuck in 'sealing' state.",
                                        e, cleanup_err, batch_id
                                    )));
                                }
                            }
                            Err(kv_err) => {
                                // Can't acquire KV handle — don't reset in-memory state,
                                // keep both sides in Sealing to avoid divergence.
                                return Err(IntentError::runtime_error(format!(
                                    "Seal failed ({}), and KV handle unavailable for cleanup ({}). \
                                     Batch '{}' is stuck in 'sealing' state.",
                                    e, kv_err, batch_id
                                )));
                            }
                        }

                        // 2. KV metadata cleaned up successfully — now safe to reset
                        //    in-memory batch to Open for retry. Copy flushed flags from
                        //    local state so retries skip already-written jobs.
                        let mut batches = BATCH_RUNTIME
                            .batches
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        if let Some(batch) = batches.get_mut(&batch_id) {
                            batch.status = BatchStatus::Open;
                            for (i, local_job) in batch_state.buffered.iter().enumerate() {
                                if i < batch.buffered.len() && local_job.flushed {
                                    batch.buffered[i].flushed = true;
                                }
                            }
                        }
                        Err(e)
                    }
                }
            },
        },
    );

    // @ntnt batch_status
    // @module std/jobs
    // @signature batch_status(batch_id_or_handle: Any) -> Result<Map, String>
    // Get the current status and counters for a batch.
    //
    // Accepts either a batch ID string or a batch handle map (with _batch_id field).
    // Returns the full batch metadata map from KV. Only available after seal().
    // @param batch_id_or_handle Batch ID string or batch handle map
    // @returns Result containing the batch metadata map or an error
    // @example batch_status(b) ~ "Get status of a sealed batch via handle"
    // @example batch_status("batch-abc-123") ~ "Get status by batch ID string"
    // @see_also batch, seal
    module.insert(
        "batch_status".to_string(),
        Value::NativeFunction {
            name: "batch_status".to_string(),
            arity: 1,
            max_arity: 1,
            requires: None,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "batch_status() requires 1 argument".to_string(),
                    ));
                }
                let batch_id = match &args[0] {
                    Value::String(s) => s.clone(),
                    Value::Map(m) => match m.get("_batch_id") {
                        Some(Value::String(bid)) => bid.clone(),
                        _ => {
                            return Err(IntentError::type_error(
                                "batch_status() argument must be a batch ID or handle".to_string(),
                            ))
                        }
                    },
                    _ => {
                        return Err(IntentError::type_error(
                            "batch_status() argument must be a batch ID string or handle"
                                .to_string(),
                        ))
                    }
                };
                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                let meta_key = format!("jobs:batch:{}", batch_id);
                match kv::kv_get(&kv_handle, &meta_key)? {
                    Value::Unit => Err(IntentError::runtime_error(format!(
                        "Batch '{}' not found",
                        batch_id
                    ))),
                    other => Ok(Value::ok(other)),
                }
            },
        },
    );

    // @ntnt batch_id
    // @module std/jobs
    // @signature batch_id() -> Option<String>
    // Returns the batch ID of the currently-executing job, or None.
    //
    // Available inside a job's perform block when the job belongs to a batch.
    // Use this to dynamically add more jobs to the same batch from within a job.
    // Returns None for jobs not associated with a batch.
    // Phase 1: always returns None. Phase 2 wires up thread-local job context.
    // @returns Option<String> — Some(batch_id) or None
    // @example let bid = batch_id() ~ "Get current job's batch ID"
    // @see_also batch, enqueue
    module.insert(
        "batch_id".to_string(),
        Value::NativeFunction {
            name: "batch_id".to_string(),
            arity: 0,
            max_arity: 0,
            requires: None,
            func: |_args| Ok(Value::none()),
        },
    );

    module
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Serialize tests that use the global JOB_RUNTIME.
    /// Parallel tests that call reset() or configure_queue() will race.
    /// pub(crate) so interpreter.rs tests that touch JOB_RUNTIME can use the same lock.
    pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn with_clean_runtime<F: FnOnce()>(f: F) {
        // unwrap_or_else recovers from poisoned mutex (a previous test panicked
        // while holding the lock — we still need to run subsequent tests)
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        JOB_RUNTIME.reset();
        BATCH_RUNTIME.reset();
        f();
    }

    /// Set up a temp SQLite KV store for the duration of a test, then clean up.
    fn with_temp_kv<F: FnOnce(&Value)>(db_name: &str, f: F) {
        with_clean_runtime(|| {
            let tmp = std::env::temp_dir().join(db_name);
            let url = format!("sqlite:{}", tmp.display());
            if let Ok(mut u) = JOB_RUNTIME.kv_url.lock() {
                *u = url.clone();
            }
            let kv = JOB_RUNTIME.get_or_init_kv().unwrap();
            f(&kv);
            let _ = std::fs::remove_file(&tmp);
        });
    }

    /// Create a minimal JobDefinition for tests (no perform body needed for registry tests).
    fn test_job_def(name: &str, queue: &str) -> JobDefinition {
        JobDefinition {
            name: name.to_string(),
            queue: queue.to_string(),
            options: HashMap::new(),
            perform_params: vec![],
            perform_contract: None,
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
            perform_contract: None,
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
    fn test_duplicate_job_registration_is_idempotent() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("DupJob", "default"))
                .unwrap();

            // Second registration is idempotent — returns Ok, first definition wins
            let result = JOB_RUNTIME.register_job(test_job_def("DupJob", "other"));
            assert!(
                result.is_ok(),
                "Duplicate registration should be idempotent"
            );

            // First registration's queue is preserved
            let job = JOB_RUNTIME.get_job("DupJob").unwrap().unwrap();
            assert_eq!(job.queue, "default", "First registration should win");
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
    fn test_execute_in_worker_empty_body() {
        // A job with an empty perform body should return Unit
        let def = test_job_def("EmptyJob", "default");
        let mut interp = crate::interpreter::Interpreter::new();
        let result = execute_in_worker(&mut interp, &def, &HashMap::new());
        assert!(result.is_ok(), "Empty body should succeed: {:?}", result);
    }

    /// Helper: parse ntnt source containing a job declaration and return the JobDefinition.
    fn parse_job_def(src: &str) -> JobDefinition {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        let tokens: Vec<_> = Lexer::new(src).collect();
        let ast = Parser::new(tokens).parse().expect("parse failed");
        let job_name = src
            .split_whitespace()
            .skip_while(|&t| t != "job")
            .nth(1)
            .expect("no job name after 'job'")
            .to_string();
        let mut interp = crate::interpreter::Interpreter::new();
        interp.eval(&ast).expect("eval failed");
        JOB_RUNTIME
            .get_job(&job_name)
            .expect("get_job failed")
            .expect("job not found")
    }

    #[test]
    fn test_execute_in_worker_scope_isolation() {
        // Locals from one job execution must not leak to parent scope or next job
        with_clean_runtime(|| {
            let def = parse_job_def("job ScopeTest on q { perform() { let leak_test = 999 } }");
            let mut interp = crate::interpreter::Interpreter::new();

            let _ = execute_in_worker(&mut interp, &def, &HashMap::new());
            assert!(
                interp.get_global("leak_test").is_none(),
                "Locals from job execution must not leak to parent scope"
            );

            // Second run — must not see job-1 locals
            let _ = execute_in_worker(&mut interp, &def, &HashMap::new());
            assert!(
                interp.get_global("leak_test").is_none(),
                "Locals from previous job must not leak"
            );
        });
    }

    #[test]
    fn test_execute_in_worker_inherits_parent_scope() {
        // Perform body can read constants defined in the interpreter's parent scope
        with_clean_runtime(|| {
            let def = parse_job_def("job InheritTest on q { perform() { APP_NAME } }");
            let mut interp = crate::interpreter::Interpreter::new();
            interp.define_global(
                "APP_NAME".to_string(),
                crate::interpreter::Value::String("test-app".to_string()),
            );

            let result = execute_in_worker(&mut interp, &def, &HashMap::new());
            match result {
                Ok(crate::interpreter::Value::String(s)) => assert_eq!(s, "test-app"),
                other => panic!("Expected String 'test-app', got {:?}", other),
            }
        });
    }

    #[test]
    fn test_execute_in_worker_params_injected() {
        // Perform parameters from the payload are visible in the body
        with_clean_runtime(|| {
            let def = parse_job_def("job ParamTest on q { perform(user_id) { user_id } }");
            let mut interp = crate::interpreter::Interpreter::new();
            let mut payload = HashMap::new();
            payload.insert(
                "user_id".to_string(),
                crate::interpreter::Value::String("abc-123".to_string()),
            );

            let result = execute_in_worker(&mut interp, &def, &payload);
            match result {
                Ok(crate::interpreter::Value::String(s)) => assert_eq!(s, "abc-123"),
                other => panic!("Expected String 'abc-123', got {:?}", other),
            }
        });
    }

    #[test]
    fn test_execute_in_worker_error_recovery() {
        // An error in the perform body must not corrupt the interpreter
        with_clean_runtime(|| {
            let def = parse_job_def("job ErrorTest on q { perform() { 1 / 0 } }");
            let mut interp = crate::interpreter::Interpreter::new();
            interp.define_global("MARKER".to_string(), crate::interpreter::Value::Int(42));

            let result = execute_in_worker(&mut interp, &def, &HashMap::new());
            assert!(result.is_err(), "Division by zero should fail");

            // Interpreter must remain functional
            assert!(
                interp.get_global("MARKER").is_some(),
                "Interpreter must remain functional after job error"
            );

            // A subsequent job must succeed
            let def2 = parse_job_def("job RecoveryTest on q { perform() { 42 } }");
            let result2 = execute_in_worker(&mut interp, &def2, &HashMap::new());
            assert!(
                result2.is_ok(),
                "Worker should recover and run next job: {:?}",
                result2
            );
        });
    }

    #[test]
    fn test_execute_in_worker_panic_recovery() {
        // A Rust-level panic in the perform body must not corrupt the interpreter.
        // This exercises the catch_unwind + snapshot_env/restore_env path — the error
        // recovery test above only covers ntnt Err (not Rust panic).
        with_clean_runtime(|| {
            // Register a native function that panics
            let mut interp = crate::interpreter::Interpreter::new();
            interp.define_global(
                "trigger_panic".to_string(),
                crate::interpreter::Value::NativeFunction {
                    name: "trigger_panic".to_string(),
                    arity: 0,
                    max_arity: 0,
                    func: |_| panic!("intentional test panic"),
                    requires: None,
                },
            );
            interp.define_global("MARKER".to_string(), crate::interpreter::Value::Int(42));

            let def =
                parse_job_def("job PanicTest on q { perform() { if true { trigger_panic() } } }");

            // Job should fail via panic (caught by catch_unwind)
            let result = execute_in_worker(&mut interp, &def, &HashMap::new());
            assert!(
                result.is_err(),
                "Panic should be caught and returned as Err"
            );
            assert!(
                result.unwrap_err().contains("intentional test panic"),
                "Error message should contain the panic message"
            );

            // Interpreter must remain functional — MARKER accessible, scope at root depth
            assert!(
                interp.get_global("MARKER").is_some(),
                "Interpreter must remain functional after panic"
            );

            // A subsequent normal job must succeed
            let def2 = parse_job_def("job AfterPanic on q { perform() { MARKER } }");
            let result2 = execute_in_worker(&mut interp, &def2, &HashMap::new());
            match result2 {
                Ok(crate::interpreter::Value::Int(42)) => {} // correct
                other => panic!("Expected Int(42) after panic recovery, got {:?}", other),
            }
        });
    }

    #[test]
    fn test_execute_in_worker_panic_does_not_leak_call_depth() {
        // Regression: a Rust panic inside a user function call increments call_depth
        // but skips the decrement. Without reset_call_depth() this accumulates across
        // jobs on a reused interpreter, eventually triggering "Maximum recursion depth
        // exceeded" for unrelated jobs.
        with_clean_runtime(|| {
            let mut interp = crate::interpreter::Interpreter::new();
            // Set a low recursion limit so the leak is detectable quickly
            interp.set_max_recursion_depth(5);

            interp.define_global(
                "trigger_panic".to_string(),
                crate::interpreter::Value::NativeFunction {
                    name: "trigger_panic".to_string(),
                    arity: 0,
                    max_arity: 0,
                    func: |_| panic!("call_depth leak test panic"),
                    requires: None,
                },
            );

            // Job that panics inside a function call — call_depth gets incremented
            // before trigger_panic() and never decremented on panic.
            let panic_def =
                parse_job_def("job DepthLeakPanic on q { perform() { trigger_panic() } }");
            // Job that calls a function successfully — verifies call_depth is still valid
            let normal_def = parse_job_def("job DepthLeakNormal on q { perform() { 1 + 1 } }");

            // Panic 4 times — without the fix this would accumulate depth=4,
            // and the next function call would hit the limit of 5.
            for _ in 0..4 {
                let r = execute_in_worker(&mut interp, &panic_def, &HashMap::new());
                assert!(r.is_err(), "Expected panic job to fail");
            }

            // This must succeed even though we've panicked 4 times.
            // Without reset_call_depth() it would fail with "Maximum recursion depth exceeded".
            let result = execute_in_worker(&mut interp, &normal_def, &HashMap::new());
            assert!(
                result.is_ok(),
                "Job after repeated panics must not hit recursion limit: {:?}",
                result
            );
        });
    }

    #[test]
    fn test_source_file_tracking() {
        with_clean_runtime(|| {
            assert!(JOB_RUNTIME.get_source_file().is_none());
            JOB_RUNTIME.set_source_file("/tmp/test.tnt".to_string());
            assert_eq!(
                JOB_RUNTIME.get_source_file(),
                Some("/tmp/test.tnt".to_string())
            );
        });
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
            let cancel = std::sync::Arc::new(crate::stdlib::concurrent::CancelToken::new());
            let cancel_clone = cancel.clone();
            let handle = std::thread::spawn(move || {
                // Set cancel token so the loop can be stopped
                crate::stdlib::concurrent::CURRENT_CANCEL_TOKEN.with(|cell| {
                    *cell.borrow_mut() = Some(cancel_clone);
                });
                let band = BandConfig {
                    name: "test".to_string(),
                    min_priority: 0,
                    max_priority: 99,
                    concurrency: 1,
                    poll_interval_ms: 50,
                };
                worker_loop(kv_info, band, None);
            });

            // Give the worker time to process
            std::thread::sleep(std::time::Duration::from_millis(300));

            // Cancel the worker
            cancel.cancel();
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
            let cancel = std::sync::Arc::new(crate::stdlib::concurrent::CancelToken::new());
            let cancel_clone = cancel.clone();
            let handle = std::thread::spawn(move || {
                crate::stdlib::concurrent::CURRENT_CANCEL_TOKEN.with(|cell| {
                    *cell.borrow_mut() = Some(cancel_clone);
                });
                let band = BandConfig {
                    name: "test".to_string(),
                    min_priority: 0,
                    max_priority: 99,
                    concurrency: 1,
                    poll_interval_ms: 50,
                };
                worker_loop(kv_info, band, None);
            });

            // Let worker poll a few times
            std::thread::sleep(std::time::Duration::from_millis(200));
            cancel.cancel();
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

    // -----------------------------------------------------------------------
    // DD-037: Priority Queues + Atomic Dedup
    // -----------------------------------------------------------------------

    /// Lower priority numbers produce lexicographically earlier pending keys.
    #[test]
    fn test_priority_key_ordering_lexicographic() {
        let k_critical = format!(
            "jobs:pending:{:02}:{}:{}",
            5u8, "01000000000000000001", "id1"
        );
        let k_high = format!(
            "jobs:pending:{:02}:{}:{}",
            25u8, "01000000000000000001", "id2"
        );
        let k_normal = format!(
            "jobs:pending:{:02}:{}:{}",
            50u8, "01000000000000000001", "id3"
        );
        let k_low = format!(
            "jobs:pending:{:02}:{}:{}",
            85u8, "01000000000000000001", "id4"
        );

        assert!(k_critical < k_high, "critical < high");
        assert!(k_high < k_normal, "high < normal");
        assert!(k_normal < k_low, "normal < low");
    }

    /// Two jobs at the same priority sort FIFO by timestamp.
    #[test]
    fn test_priority_fifo_within_band() {
        let k_first = format!(
            "jobs:pending:{:02}:{}:{}",
            50u8, "01000000000000000001", "id1"
        );
        let k_second = format!(
            "jobs:pending:{:02}:{}:{}",
            50u8, "01000000000000000002", "id2"
        );
        assert!(
            k_first < k_second,
            "earlier timestamp = higher priority within band"
        );
    }

    /// Named priority strings resolve to the expected numeric values.
    #[test]
    fn test_priority_named_values() {
        with_clean_runtime(|| {
            let make_job = |name: &str, priority: &str| -> JobDefinition {
                let mut opts = HashMap::new();
                opts.insert(
                    "priority".to_string(),
                    JobOptionValue::String(priority.to_string()),
                );
                JobDefinition {
                    name: name.to_string(),
                    queue: "default".to_string(),
                    perform_params: vec![],
                    perform_contract: None,
                    perform_body: Block { statements: vec![] },
                    on_failure: None,
                    options: opts,
                }
            };

            let module = init();
            configure_in_memory(&module);

            for (name, priority_str, expected_num) in &[
                ("PnCritical", "critical", 5i64),
                ("PnHigh", "high", 25i64),
                ("PnNormal", "normal", 50i64),
                ("PnLow", "low", 85i64),
            ] {
                JOB_RUNTIME
                    .register_job(make_job(name, priority_str))
                    .unwrap();
                let id = enqueue_job(&module, name, HashMap::new());
                let kv = JOB_RUNTIME.get_or_init_kv().unwrap();
                let data_key = format!("jobs:data:{}", id);
                match kv::kv_get(&kv, &data_key).unwrap() {
                    Value::Map(data) => {
                        assert!(
                            matches!(data.get("priority"), Some(Value::Int(n)) if *n == *expected_num),
                            "priority for {} should be {}, got {:?}",
                            priority_str,
                            expected_num,
                            data.get("priority")
                        );
                    }
                    _ => panic!("Expected Map for job data"),
                }
            }
        });
    }

    /// Numeric priority 0-99 is stored verbatim in job data.
    #[test]
    fn test_priority_numeric_stored_in_job_data() {
        with_clean_runtime(|| {
            let mut opts = HashMap::new();
            opts.insert("priority".to_string(), JobOptionValue::Int(7));
            let def = JobDefinition {
                name: "NumPrioJob".to_string(),
                queue: "default".to_string(),
                perform_params: vec![],
                perform_contract: None,
                perform_body: Block { statements: vec![] },
                on_failure: None,
                options: opts,
            };
            JOB_RUNTIME.register_job(def).unwrap();

            let module = init();
            configure_in_memory(&module);
            let id = enqueue_job(&module, "NumPrioJob", HashMap::new());

            let kv = JOB_RUNTIME.get_or_init_kv().unwrap();
            let data_key = format!("jobs:data:{}", id);
            match kv::kv_get(&kv, &data_key).unwrap() {
                Value::Map(data) => {
                    assert!(
                        matches!(data.get("priority"), Some(Value::Int(7))),
                        "priority should be 7"
                    );
                    assert!(
                        matches!(data.get("band"), Some(Value::String(s)) if s == "critical"),
                        "band should be 'critical'"
                    );
                }
                _ => panic!("Expected Map"),
            }
        });
    }

    /// Priority out of range (> 99) returns an error.
    #[test]
    fn test_priority_out_of_range_error() {
        with_clean_runtime(|| {
            let mut opts = HashMap::new();
            opts.insert("priority".to_string(), JobOptionValue::Int(100));
            let def = JobDefinition {
                name: "BadPrioJob".to_string(),
                queue: "default".to_string(),
                perform_params: vec![],
                perform_contract: None,
                perform_body: Block { statements: vec![] },
                on_failure: None,
                options: opts,
            };
            JOB_RUNTIME.register_job(def).unwrap();
            let module = init();
            configure_in_memory(&module);

            let enqueue_fn = get_fn(&module, "enqueue");
            let result = enqueue_fn(&[
                Value::String("BadPrioJob".to_string()),
                Value::Map(HashMap::new()),
            ]);
            assert!(result.is_err(), "Priority 100 should be rejected");
        });
    }

    /// Unknown named priority returns an error mentioning the name.
    #[test]
    fn test_priority_unknown_named_error() {
        with_clean_runtime(|| {
            let mut opts = HashMap::new();
            opts.insert(
                "priority".to_string(),
                JobOptionValue::String("urgent".to_string()),
            );
            let def = JobDefinition {
                name: "UnknownPrioJob".to_string(),
                queue: "default".to_string(),
                perform_params: vec![],
                perform_contract: None,
                perform_body: Block { statements: vec![] },
                on_failure: None,
                options: opts,
            };
            JOB_RUNTIME.register_job(def).unwrap();
            let module = init();
            configure_in_memory(&module);

            let enqueue_fn = get_fn(&module, "enqueue");
            let result = enqueue_fn(&[
                Value::String("UnknownPrioJob".to_string()),
                Value::Map(HashMap::new()),
            ]);
            assert!(result.is_err(), "Unknown priority name should be rejected");
            let err = format!("{}", result.unwrap_err());
            assert!(
                err.contains("Unknown priority"),
                "Error should mention unknown priority"
            );
        });
    }

    /// Band name in job data should reflect the actual priority range.
    #[test]
    fn test_band_name_derived_from_priority() {
        let cases: &[(u8, &str)] = &[
            (0, "critical"),
            (5, "critical"),
            (9, "critical"),
            (10, "high"),
            (39, "high"),
            (40, "normal"),
            (69, "normal"),
            (70, "low"),
            (99, "low"),
        ];
        for (priority, expected_band) in cases {
            let band_name = if *priority <= 9 {
                "critical"
            } else if *priority <= 39 {
                "high"
            } else if *priority <= 69 {
                "normal"
            } else {
                "low"
            };
            assert_eq!(
                band_name, *expected_band,
                "Priority {} should be in band '{}'",
                priority, expected_band
            );
        }
    }

    /// BandConfig floor_key starts with the min_priority prefix.
    #[test]
    fn test_band_config_floor_key_prefix() {
        let band = BandConfig {
            name: "critical".to_string(),
            min_priority: 0,
            max_priority: 9,
            concurrency: 4,
            poll_interval_ms: 200,
        };
        assert!(
            band.floor_key().starts_with("jobs:pending:00:"),
            "floor should start with 00 prefix"
        );
        assert!(
            band.ceiling_key().starts_with("jobs:pending:09:"),
            "ceiling should start with 09 prefix"
        );
    }

    /// parse_band_config rejects min_priority > max_priority.
    #[test]
    fn test_parse_band_config_min_gt_max() {
        let mut m = HashMap::new();
        m.insert("name".to_string(), Value::String("bad".to_string()));
        m.insert("min_priority".to_string(), Value::Int(50));
        m.insert("max_priority".to_string(), Value::Int(10));
        m.insert("concurrency".to_string(), Value::Int(1));
        let result = parse_band_config(&m);
        assert!(result.is_err(), "min > max should be rejected");
    }

    /// parse_band_config rejects concurrency < 1.
    #[test]
    fn test_parse_band_config_zero_concurrency() {
        let mut m = HashMap::new();
        m.insert("name".to_string(), Value::String("bad".to_string()));
        m.insert("min_priority".to_string(), Value::Int(0));
        m.insert("max_priority".to_string(), Value::Int(99));
        m.insert("concurrency".to_string(), Value::Int(0));
        let result = parse_band_config(&m);
        assert!(result.is_err(), "concurrency 0 should be rejected");
    }

    /// parse_band_config rejects poll_interval < 1.
    #[test]
    fn test_parse_band_config_zero_poll_interval() {
        let mut m = HashMap::new();
        m.insert("name".to_string(), Value::String("bad".to_string()));
        m.insert("min_priority".to_string(), Value::Int(0));
        m.insert("max_priority".to_string(), Value::Int(99));
        m.insert("concurrency".to_string(), Value::Int(1));
        m.insert("poll_interval".to_string(), Value::Int(0));
        let result = parse_band_config(&m);
        assert!(result.is_err(), "poll_interval 0 should be rejected");
    }

    /// validate_bands rejects overlapping priority ranges.
    #[test]
    fn test_validate_bands_overlap() {
        let bands = vec![
            BandConfig {
                name: "a".to_string(),
                min_priority: 0,
                max_priority: 49,
                concurrency: 1,
                poll_interval_ms: 1000,
            },
            BandConfig {
                name: "b".to_string(),
                min_priority: 40,
                max_priority: 99,
                concurrency: 1,
                poll_interval_ms: 1000,
            },
        ];
        let result = validate_bands(&bands);
        assert!(result.is_err(), "Overlapping bands should be rejected");
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("overlap"), "Error should mention overlap");
    }

    /// validate_bands rejects gaps in priority coverage.
    #[test]
    fn test_validate_bands_gap() {
        let bands = vec![
            BandConfig {
                name: "a".to_string(),
                min_priority: 0,
                max_priority: 39,
                concurrency: 1,
                poll_interval_ms: 1000,
            },
            BandConfig {
                name: "b".to_string(),
                min_priority: 50,
                max_priority: 99,
                concurrency: 1,
                poll_interval_ms: 1000,
            },
        ];
        let result = validate_bands(&bands);
        assert!(result.is_err(), "Gaps should be rejected");
        let err = format!("{}", result.unwrap_err());
        assert!(err.contains("gap"), "Error should mention gap");
    }

    /// validate_bands rejects bands not starting at 0.
    #[test]
    fn test_validate_bands_no_zero_start() {
        let bands = vec![BandConfig {
            name: "a".to_string(),
            min_priority: 10,
            max_priority: 99,
            concurrency: 1,
            poll_interval_ms: 1000,
        }];
        assert!(
            validate_bands(&bands).is_err(),
            "Bands not starting at 0 should be rejected"
        );
    }

    /// validate_bands rejects bands not ending at 99.
    #[test]
    fn test_validate_bands_no_99_end() {
        let bands = vec![BandConfig {
            name: "a".to_string(),
            min_priority: 0,
            max_priority: 89,
            concurrency: 1,
            poll_interval_ms: 1000,
        }];
        assert!(
            validate_bands(&bands).is_err(),
            "Bands not ending at 99 should be rejected"
        );
    }

    /// validate_bands accepts the default 4-band configuration.
    #[test]
    fn test_validate_default_bands() {
        let bands = default_bands();
        assert!(
            validate_bands(&bands).is_ok(),
            "Default bands should be valid"
        );
    }

    /// Parser: `job Name on queue` sets the queue field correctly.
    #[test]
    fn test_parser_job_on_queue() {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        let src = "job SendEmail on emails (retry: 3) { perform(to) { print(to) } }";
        let tokens: Vec<_> = Lexer::new(src).collect();
        let mut parser = Parser::new(tokens);
        let prog = parser.parse().expect("Should parse without error");
        // Filter out Located wrappers to find the Job statement
        let job_stmt = prog.statements.iter().find(|s| {
            matches!(s, crate::ast::Statement::Job { .. })
                || matches!(s, crate::ast::Statement::Located { stmt, .. }
                    if matches!(stmt.as_ref(), crate::ast::Statement::Job { .. }))
        });
        assert!(job_stmt.is_some(), "Should have a Job statement");
        let inner = match job_stmt.unwrap() {
            crate::ast::Statement::Job { name, queue, .. } => (name.clone(), queue.clone()),
            crate::ast::Statement::Located { stmt, .. } => match stmt.as_ref() {
                crate::ast::Statement::Job { name, queue, .. } => (name.clone(), queue.clone()),
                _ => panic!("Expected Job"),
            },
            _ => panic!("Expected Job"),
        };
        assert_eq!(inner.0, "SendEmail");
        assert_eq!(inner.1, "emails");
    }

    /// Parser: `job Name (...)` without `on queue` defaults queue to "default".
    #[test]
    fn test_parser_job_default_queue() {
        use crate::lexer::Lexer;
        use crate::parser::Parser;
        let src = "job SendEmail (retry: 3) { perform(to) { print(to) } }";
        let tokens: Vec<_> = Lexer::new(src).collect();
        let mut parser = Parser::new(tokens);
        let prog = parser.parse().expect("Should parse without error");
        let job_stmt = prog.statements.iter().find(|s| {
            matches!(s, crate::ast::Statement::Job { .. })
                || matches!(s, crate::ast::Statement::Located { stmt, .. }
                    if matches!(stmt.as_ref(), crate::ast::Statement::Job { .. }))
        });
        assert!(job_stmt.is_some(), "Should have a Job statement");
        let inner = match job_stmt.unwrap() {
            crate::ast::Statement::Job { name, queue, .. } => (name.clone(), queue.clone()),
            crate::ast::Statement::Located { stmt, .. } => match stmt.as_ref() {
                crate::ast::Statement::Job { name, queue, .. } => (name.clone(), queue.clone()),
                _ => panic!("Expected Job"),
            },
            _ => panic!("Expected Job"),
        };
        assert_eq!(inner.0, "SendEmail");
        assert_eq!(inner.1, "default");
    }

    /// kv_set_nx returns true on first write, false on second write, preserving original value.
    #[test]
    fn test_kv_set_nx_basic() {
        with_clean_runtime(|| {
            let module = init();
            configure_in_memory(&module);
            let kv = JOB_RUNTIME.get_or_init_kv().unwrap();

            let first = kv::kv_set_nx(&kv, "test:nx:key", &Value::String("v1".to_string()), None)
                .expect("set_nx should not error");
            assert!(first, "First set_nx should return true");

            let second = kv::kv_set_nx(&kv, "test:nx:key", &Value::String("v2".to_string()), None)
                .expect("set_nx should not error");
            assert!(!second, "Second set_nx should return false");

            match kv::kv_get(&kv, "test:nx:key").unwrap() {
                Value::String(s) => assert_eq!(s, "v1", "Value should still be v1"),
                _ => panic!("Expected String"),
            }
        });
    }

    /// Atomic dedup: two enqueues with the same payload return the same job_id.
    #[test]
    fn test_atomic_dedup_concurrent_enqueue() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def_with_unique("ConcurDedupJob", "default", 3600))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            let mut payload = HashMap::new();
            payload.insert("user".to_string(), Value::String("alice".to_string()));

            let id1 = enqueue_job(&module, "ConcurDedupJob", payload.clone());
            let id2 = enqueue_job(&module, "ConcurDedupJob", payload.clone());
            assert_eq!(id1, id2, "Duplicate enqueues should return the same job_id");
        });
    }

    /// Atomic dedup: different payloads are not deduped.
    #[test]
    fn test_atomic_dedup_different_payloads() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def_with_unique("DiffPldJob", "default", 3600))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            let mut p1 = HashMap::new();
            p1.insert("user".to_string(), Value::String("alice".to_string()));
            let mut p2 = HashMap::new();
            p2.insert("user".to_string(), Value::String("bob".to_string()));

            let id1 = enqueue_job(&module, "DiffPldJob", p1);
            let id2 = enqueue_job(&module, "DiffPldJob", p2);
            assert_ne!(id1, id2, "Different payloads should not be deduped");
        });
    }

    /// Retry pending key should preserve the original priority prefix.
    #[test]
    fn test_retry_pending_key_preserves_priority() {
        with_clean_runtime(|| {
            let mut opts = HashMap::new();
            opts.insert("priority".to_string(), JobOptionValue::Int(5)); // critical
            opts.insert("retry".to_string(), JobOptionValue::Int(2));
            let def = JobDefinition {
                name: "RetryPrioJob".to_string(),
                queue: "default".to_string(),
                perform_params: vec![],
                perform_contract: None,
                perform_body: Block { statements: vec![] },
                on_failure: None,
                options: opts,
            };
            JOB_RUNTIME.register_job(def).unwrap();
            let module = init();
            configure_in_memory(&module);

            let id = enqueue_job(&module, "RetryPrioJob", HashMap::new());
            let kv = JOB_RUNTIME.get_or_init_kv().unwrap();
            let data_key = format!("jobs:data:{}", id);

            match kv::kv_get(&kv, &data_key).unwrap() {
                Value::Map(data) => {
                    let pk = match data.get("pending_key") {
                        Some(Value::String(s)) => s.clone(),
                        _ => panic!("Expected pending_key string"),
                    };
                    assert!(
                        pk.starts_with("jobs:pending:05:"),
                        "Pending key should start with priority 05, got: {}",
                        pk
                    );
                }
                _ => panic!("Expected Map"),
            }
        });
    }

    /// worker_status() returns a Map with "bands" and "pending" keys.
    #[test]
    fn test_worker_status_returns_map() {
        with_clean_runtime(|| {
            let module = init();
            configure_in_memory(&module);

            let status_fn = get_fn(&module, "worker_status");
            let result = status_fn(&[]).unwrap();
            match result {
                Value::Map(m) => {
                    assert!(m.contains_key("bands"), "worker_status should have 'bands'");
                    assert!(
                        m.contains_key("pending"),
                        "worker_status should have 'pending'"
                    );
                }
                _ => panic!("worker_status should return a Map"),
            }
        });
    }

    /// worker_status() pending count reflects actual enqueued jobs.
    #[test]
    fn test_worker_status_pending_count() {
        with_clean_runtime(|| {
            JOB_RUNTIME
                .register_job(test_job_def("WsPendingJob", "default"))
                .unwrap();
            let module = init();
            configure_in_memory(&module);

            enqueue_job(&module, "WsPendingJob", HashMap::new());
            enqueue_job(&module, "WsPendingJob", HashMap::new());

            let status_fn = get_fn(&module, "worker_status");
            let result = status_fn(&[]).unwrap();
            match result {
                Value::Map(m) => match m.get("pending") {
                    Some(Value::Int(n)) => assert_eq!(*n, 2, "pending count should be 2"),
                    _ => panic!("Expected Int for pending"),
                },
                _ => panic!("Expected Map"),
            }
        });
    }

    /// scale_workers() errors when called before work_async() (no active bands).
    #[test]
    fn test_scale_workers_no_active_bands() {
        with_clean_runtime(|| {
            let module = init();
            configure_in_memory(&module);

            let scale_fn = get_fn(&module, "scale_workers");
            let result = scale_fn(&[Value::String("normal".to_string()), Value::Int(2)]);
            assert!(
                result.is_err(),
                "scale_workers without active bands should error"
            );
        });
    }

    /// scale_workers() errors for count < 1.
    #[test]
    fn test_scale_workers_count_below_one() {
        with_clean_runtime(|| {
            let module = init();
            configure_in_memory(&module);

            let scale_fn = get_fn(&module, "scale_workers");
            let result = scale_fn(&[Value::String("normal".to_string()), Value::Int(0)]);
            assert!(result.is_err(), "scale_workers with count 0 should error");
        });
    }

    /// parse_bands_and_queues parses custom band configs from "bands" key.
    #[test]
    fn test_parse_bands_and_queues_custom_bands() {
        let mut band_map = HashMap::new();
        band_map.insert("name".to_string(), Value::String("fast".to_string()));
        band_map.insert("min_priority".to_string(), Value::Int(0));
        band_map.insert("max_priority".to_string(), Value::Int(99));
        band_map.insert("concurrency".to_string(), Value::Int(2));

        let mut opts = HashMap::new();
        opts.insert(
            "bands".to_string(),
            Value::Array(vec![Value::Map(band_map)]),
        );

        let result = parse_bands_and_queues(&[Value::Map(opts)]);
        assert!(result.is_ok(), "Custom bands should parse OK");
        let (bands, queues) = result.unwrap();
        assert_eq!(bands.len(), 1);
        assert_eq!(bands[0].name, "fast");
        assert_eq!(bands[0].concurrency, 2);
        assert!(queues.is_none());
    }

    /// parse_bands_and_queues without "bands" key falls back to single legacy band.
    #[test]
    fn test_parse_bands_and_queues_legacy_fallback() {
        let mut opts = HashMap::new();
        opts.insert("poll_interval".to_string(), Value::Int(500));
        opts.insert("concurrency".to_string(), Value::Int(3));

        let (bands, _) = parse_bands_and_queues(&[Value::Map(opts)]).unwrap();
        assert_eq!(bands.len(), 1, "Legacy fallback should produce one band");
        assert_eq!(bands[0].name, "normal");
        assert_eq!(bands[0].poll_interval_ms, 500);
        assert_eq!(bands[0].concurrency, 3);
    }

    /// BandStats atomic counters track active, completed, and failed counts correctly.
    #[test]
    fn test_band_stats_increment() {
        let stats = Arc::new(BandStats::new());
        stats
            .active
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        stats
            .completed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        stats
            .failed
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        stats
            .total_duration_ms
            .fetch_add(42, std::sync::atomic::Ordering::Relaxed);

        assert_eq!(stats.active.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            stats.completed.load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        assert_eq!(stats.failed.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(
            stats
                .total_duration_ms
                .load(std::sync::atomic::Ordering::Relaxed),
            42
        );
    }

    /// get_or_create_band_stats returns the same Arc for the same band name.
    #[test]
    fn test_get_or_create_band_stats_idempotent() {
        with_clean_runtime(|| {
            let stats1 = JOB_RUNTIME.get_or_create_band_stats("critical");
            let stats2 = JOB_RUNTIME.get_or_create_band_stats("critical");
            stats1
                .completed
                .fetch_add(5, std::sync::atomic::Ordering::Relaxed);
            assert_eq!(
                stats2.completed.load(std::sync::atomic::Ordering::Relaxed),
                5,
                "Should return the same Arc for the same band name"
            );
        });
    }

    /// default_bands() returns exactly 4 bands covering 0-99 with 10 total workers.
    #[test]
    fn test_default_bands_count_and_coverage() {
        let bands = default_bands();
        assert_eq!(bands.len(), 4, "default_bands() should return 4 bands");
        let total_workers: usize = bands.iter().map(|b| b.concurrency).sum();
        assert_eq!(total_workers, 10, "default bands should total 10 workers");
        assert_eq!(bands[0].min_priority, 0, "first band must start at 0");
        assert_eq!(
            bands[bands.len() - 1].max_priority,
            99,
            "last band must end at 99"
        );
        // Contiguous coverage
        for i in 1..bands.len() {
            assert_eq!(
                bands[i].min_priority,
                bands[i - 1].max_priority + 1,
                "bands must be contiguous at index {}",
                i
            );
        }
    }

    /// A band worker with range 0-9 should NOT claim a job enqueued at priority 50.
    #[test]
    fn test_band_isolation_worker_ignores_outside_range() {
        with_clean_runtime(|| {
            // Register a job with priority 50 (normal band)
            let mut opts = HashMap::new();
            opts.insert("priority".to_string(), JobOptionValue::Int(50));
            let def = JobDefinition {
                name: "TaggedJob".to_string(),
                queue: "default".to_string(),
                perform_params: vec![],
                perform_contract: None,
                perform_body: crate::ast::Block { statements: vec![] },
                on_failure: None,
                options: opts,
            };
            JOB_RUNTIME.register_job(def).unwrap();

            let module = init();
            configure_in_memory(&module);

            // Enqueue the job (priority comes from job definition)
            let enqueue_fn = get_fn(&module, "enqueue");
            let mut payload = HashMap::new();
            payload.insert(
                "tag".to_string(),
                Value::String("normal-priority".to_string()),
            );
            let result =
                enqueue_fn(&[Value::String("TaggedJob".to_string()), Value::Map(payload)]).unwrap();
            let job_id = match result {
                Value::EnumValue { values, .. } => match &values[0] {
                    Value::String(s) => s.clone(),
                    _ => panic!("Expected string ID"),
                },
                _ => panic!("Expected Ok"),
            };

            // Run a critical-band worker (range 0-9) for a short time
            let kv_handle = JOB_RUNTIME.get_or_init_kv().unwrap();
            let kv_info = extract_kv_handle_info(&kv_handle).unwrap();
            let cancel = std::sync::Arc::new(crate::stdlib::concurrent::CancelToken::new());
            let cancel_clone = cancel.clone();
            let handle = std::thread::spawn(move || {
                crate::stdlib::concurrent::CURRENT_CANCEL_TOKEN.with(|cell| {
                    *cell.borrow_mut() = Some(cancel_clone);
                });
                let band = BandConfig {
                    name: "critical".to_string(),
                    min_priority: 0,
                    max_priority: 9,
                    concurrency: 1,
                    poll_interval_ms: 50,
                };
                worker_loop(kv_info, band, None);
            });

            std::thread::sleep(std::time::Duration::from_millis(250));
            cancel.cancel();
            handle.join().unwrap();

            // Job at priority 50 should still be pending — critical worker should not have claimed it
            let status_fn = get_fn(&module, "job_status");
            let status = status_fn(&[Value::String(job_id)]).unwrap();
            match status {
                Value::EnumValue {
                    variant, values, ..
                } if variant == "Ok" => match &values[0] {
                    Value::Map(data) => {
                        let s = match data.get("status") {
                            Some(Value::String(s)) => s.clone(),
                            _ => panic!("Expected status string"),
                        };
                        assert_eq!(
                            s, "pending",
                            "Priority-50 job should remain pending after critical-band worker run"
                        );
                    }
                    _ => panic!("Expected Map in Ok"),
                },
                _ => panic!("Expected Ok from job_status"),
            }
        });
    }

    /// parse_band_config accepts the design-doc format: "range" array + "poll" key.
    #[test]
    fn test_parse_band_config_range_poll_format() {
        let mut m = HashMap::new();
        m.insert("name".to_string(), Value::String("payments".to_string()));
        m.insert(
            "range".to_string(),
            Value::Array(vec![Value::Int(0), Value::Int(15)]),
        );
        m.insert("poll".to_string(), Value::Int(500));
        m.insert("concurrency".to_string(), Value::Int(3));

        let result = parse_band_config(&m);
        assert!(
            result.is_ok(),
            "range+poll format should parse OK: {:?}",
            result
        );
        let band = result.unwrap();
        assert_eq!(band.name, "payments");
        assert_eq!(band.min_priority, 0);
        assert_eq!(band.max_priority, 15);
        assert_eq!(band.poll_interval_ms, 500);
        assert_eq!(band.concurrency, 3);
    }

    /// parse_band_config rejects poll interval below 100ms (e.g., 50ms).
    #[test]
    fn test_parse_band_config_poll_below_100ms() {
        let mut m = HashMap::new();
        m.insert("name".to_string(), Value::String("fast".to_string()));
        m.insert("min_priority".to_string(), Value::Int(0));
        m.insert("max_priority".to_string(), Value::Int(99));
        m.insert("concurrency".to_string(), Value::Int(1));
        m.insert("poll".to_string(), Value::Int(50));
        let result = parse_band_config(&m);
        assert!(result.is_err(), "poll 50ms should be rejected (min 100ms)");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("100ms"),
            "Error should mention 100ms minimum, got: {}",
            msg
        );
    }

    #[test]
    fn test_pause_resume_queue_basic() {
        with_temp_kv("ntnt_pause_test.db", |kv| {
            assert!(
                !is_queue_paused("emails", kv),
                "Queue should not be paused initially"
            );

            pause_queue_impl("emails").unwrap();
            assert!(
                is_queue_paused("emails", kv),
                "Queue should be paused after pause_queue_impl"
            );

            resume_queue_impl("emails").unwrap();
            mark_pause_cache_stale();
            assert!(
                !is_queue_paused("emails", kv),
                "Queue should not be paused after resume_queue_impl"
            );
        });
    }

    #[test]
    fn test_pause_queue_persisted_in_kv() {
        with_temp_kv("ntnt_pause_kv_test.db", |kv| {
            pause_queue_impl("webhooks").unwrap();

            let val = kv::kv_get(kv, "jobs:paused:webhooks").unwrap();
            assert!(
                matches!(val, Value::String(ref s) if !s.is_empty()),
                "jobs:paused:webhooks should be set in KV, got {:?}",
                val
            );

            resume_queue_impl("webhooks").unwrap();

            let val = kv::kv_get(kv, "jobs:paused:webhooks").unwrap();
            assert!(
                matches!(val, Value::Unit),
                "jobs:paused:webhooks should be deleted after resume, got {:?}",
                val
            );
        });
    }

    #[test]
    fn test_is_queue_paused_cache_refresh() {
        with_temp_kv("ntnt_pause_refresh_test.db", |kv| {
            kv::kv_set(
                kv,
                "jobs:paused:billing",
                &Value::String("123456789".to_string()),
                None,
            )
            .unwrap();

            mark_pause_cache_stale();

            assert!(
                is_queue_paused("billing", kv),
                "Cache refresh should detect externally-set pause"
            );
        });
    }

    // ── Rate Limit Tests ─────────────────────────────────────────────────────

    #[test]
    fn test_parse_rate_limit_valid() {
        for (input, count, window) in [
            ("10/second", 10u64, 1u64),
            ("100/minute", 100, 60),
            ("1000/hour", 1000, 3600),
        ] {
            let rl =
                parse_rate_limit(input).unwrap_or_else(|| panic!("expected Some for '{}'", input));
            assert_eq!(rl.count, count, "count for '{}'", input);
            assert_eq!(rl.window_secs, window, "window_secs for '{}'", input);
        }
    }

    #[test]
    fn test_parse_rate_limit_invalid() {
        for (input, reason) in [
            ("0/second", "zero count is invalid"),
            ("10/day", "day not supported"),
            ("abc/second", "non-numeric count"),
            ("10", "missing interval"),
            ("", "empty string"),
        ] {
            assert!(parse_rate_limit(input).is_none(), "{}", reason);
        }
    }

    #[test]
    fn test_rate_limit_kv_counter_increments() {
        with_temp_kv("ntnt_rate_limit_counter_test.db", |kv| {
            let rl_key = "jobs:ratelimit:SendEmail:1000000";
            let c1 = kv::kv_incr(kv, rl_key, 1).unwrap();
            let c2 = kv::kv_incr(kv, rl_key, 1).unwrap();
            let c3 = kv::kv_incr(kv, rl_key, 1).unwrap();
            assert_eq!(c1, 1);
            assert_eq!(c2, 2);
            assert_eq!(c3, 3);
        });
    }

    #[test]
    fn test_rate_limit_window_key_format() {
        // Verify window_start calculation is stable within a window.
        let window_secs = 60u64;
        let now_secs = 1_700_000_100i64; // arbitrary timestamp
        let window_start = now_secs - (now_secs % window_secs as i64);
        let key = format!("jobs:ratelimit:SendEmail:{}", window_start);
        assert!(key.starts_with("jobs:ratelimit:SendEmail:"));
        // window_start should be a multiple of 60
        assert_eq!(window_start % 60, 0);
    }

    // ── Concurrency Limit Tests ───────────────────────────────────────────────

    #[test]
    fn test_concurrency_counter_acquire() {
        with_temp_kv("ntnt_concurrency_counter_test.db", |kv| {
            let counter_key = "jobs:concurrency:SendEmail";
            let c1 = kv::kv_incr(kv, counter_key, 1).unwrap();
            let c2 = kv::kv_incr(kv, counter_key, 1).unwrap();
            let c3 = kv::kv_incr(kv, counter_key, 1).unwrap();
            assert_eq!(c1, 1);
            assert_eq!(c2, 2);
            assert_eq!(c3, 3);

            let c4 = kv::kv_incr(kv, counter_key, 1).unwrap();
            assert_eq!(c4, 4, "Counter increments atomically");
            let c4_rollback = kv::kv_incr(kv, counter_key, -1).unwrap();
            assert_eq!(c4_rollback, 3, "Rollback restores count");
        });
    }

    #[test]
    fn test_concurrency_counter_release() {
        with_temp_kv("ntnt_concurrency_release_test.db", |kv| {
            let counter_key = "jobs:concurrency:ProcessVideo";
            kv::kv_incr(kv, counter_key, 1).unwrap();
            kv::kv_incr(kv, counter_key, 1).unwrap();

            let after = kv::kv_incr(kv, counter_key, -1).unwrap();
            assert_eq!(after, 1, "Release should decrement counter");

            let after2 = kv::kv_incr(kv, counter_key, -1).unwrap();
            assert_eq!(after2, 0, "All slots released");
        });
    }

    #[test]
    fn test_concurrency_counter_per_job_type() {
        with_temp_kv("ntnt_concurrency_pertype_test.db", |kv| {
            let c1 = kv::kv_incr(kv, "jobs:concurrency:SendEmail", 1).unwrap();
            let c2 = kv::kv_incr(kv, "jobs:concurrency:ProcessVideo", 1).unwrap();
            assert_eq!(c1, 1);
            assert_eq!(c2, 1, "Different job type starts at 1, independent");
        });
    }

    // ── Batch Tests ───────────────────────────────────────────────────────────

    #[test]
    fn test_batch_create() {
        with_clean_runtime(|| {
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let result = batch_fn(&[Value::String("csv-import".to_string())]).unwrap();
            match result {
                Value::Map(ref m) => {
                    assert!(
                        matches!(m.get("_batch_id"), Some(Value::String(_))),
                        "batch handle must have _batch_id string field"
                    );
                }
                _ => panic!("batch() must return a Map"),
            }
        });
    }

    #[test]
    fn test_batch_enqueue_buffers() {
        with_temp_kv("ntnt_batch_buffer_test.db", |kv| {
            JOB_RUNTIME
                .register_job(test_job_def("ProcessRow", "imports"))
                .unwrap();
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let enqueue_fn = get_fn(&module, "enqueue");

            let handle = batch_fn(&[Value::String("test-batch".to_string())]).unwrap();

            // Enqueue into batch — should NOT write to KV
            let mut payload = HashMap::new();
            payload.insert("row_id".to_string(), Value::String("r1".to_string()));
            let result = enqueue_fn(&[
                handle.clone(),
                Value::String("ProcessRow".to_string()),
                Value::Map(payload),
            ])
            .unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "enqueue to batch must return Ok"
            );

            // No pending keys in KV
            let pending_keys = kv::kv_list(kv, Some("jobs:pending:")).unwrap_or_default();
            assert!(
                pending_keys.is_empty(),
                "No jobs:pending: keys should exist before seal"
            );
            // No job data in KV
            let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
            assert!(
                data_keys.is_empty(),
                "No jobs:data: keys should exist before seal"
            );
        });
    }

    #[test]
    fn test_batch_seal_writes_jobs() {
        with_temp_kv("ntnt_batch_seal_test.db", |kv| {
            JOB_RUNTIME
                .register_job(test_job_def("ProcessRow", "imports"))
                .unwrap();
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let enqueue_fn = get_fn(&module, "enqueue");
            let seal_fn = get_fn(&module, "seal");

            let handle = batch_fn(&[Value::String("seal-test".to_string())]).unwrap();

            for i in 0..3 {
                let mut payload = HashMap::new();
                payload.insert("row_id".to_string(), Value::String(format!("r{}", i)));
                enqueue_fn(&[
                    handle.clone(),
                    Value::String("ProcessRow".to_string()),
                    Value::Map(payload),
                ])
                .unwrap();
            }

            let seal_result = seal_fn(&[handle.clone()]).unwrap();
            assert!(
                matches!(seal_result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "seal() must return Ok"
            );

            // Batch metadata written to KV
            let bid = match &handle {
                Value::Map(m) => match m.get("_batch_id") {
                    Some(Value::String(s)) => s.clone(),
                    _ => panic!("no _batch_id"),
                },
                _ => panic!("not a map"),
            };
            let meta = kv::kv_get(kv, &format!("jobs:batch:{}", bid)).unwrap();
            assert!(
                matches!(meta, Value::Map(_)),
                "batch metadata must be in KV"
            );
            match meta {
                Value::Map(ref m) => {
                    assert!(
                        matches!(m.get("status"), Some(Value::String(s)) if s == "sealed"),
                        "status must be sealed"
                    );
                    assert!(
                        matches!(m.get("total"), Some(Value::Int(3))),
                        "total must be 3"
                    );
                    assert!(
                        matches!(m.get("pending"), Some(Value::Int(3))),
                        "pending must be 3"
                    );
                }
                _ => unreachable!(),
            }

            // 3 jobs written to KV
            let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
            assert_eq!(
                data_keys.len(),
                3,
                "3 jobs:data: entries expected after seal"
            );
            let pending_keys = kv::kv_list(kv, Some("jobs:pending:")).unwrap_or_default();
            assert_eq!(
                pending_keys.len(),
                3,
                "3 jobs:pending: entries expected after seal"
            );
        });
    }

    #[test]
    fn test_batch_seal_empty() {
        with_temp_kv("ntnt_batch_empty_test.db", |kv| {
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let seal_fn = get_fn(&module, "seal");

            let handle = batch_fn(&[Value::String("empty-batch".to_string())]).unwrap();
            let seal_result = seal_fn(&[handle.clone()]).unwrap();
            assert!(
                matches!(seal_result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "seal() on empty batch must return Ok"
            );

            let bid = match &handle {
                Value::Map(m) => match m.get("_batch_id") {
                    Some(Value::String(s)) => s.clone(),
                    _ => panic!("no _batch_id"),
                },
                _ => panic!("not a map"),
            };
            let meta = kv::kv_get(kv, &format!("jobs:batch:{}", bid)).unwrap();
            match meta {
                Value::Map(ref m) => {
                    assert!(
                        matches!(m.get("status"), Some(Value::String(s)) if s == "complete"),
                        "status must be complete"
                    );
                    assert!(
                        matches!(m.get("total"), Some(Value::Int(0))),
                        "total must be 0"
                    );
                    assert!(
                        matches!(m.get("fired_success"), Some(Value::Bool(true))),
                        "fired_success must be true"
                    );
                    assert!(
                        matches!(m.get("fired_complete"), Some(Value::Bool(true))),
                        "fired_complete must be true"
                    );
                }
                _ => panic!("expected batch metadata map"),
            }
        });
    }

    #[test]
    fn test_batch_status() {
        with_temp_kv("ntnt_batch_status_test.db", |_kv| {
            JOB_RUNTIME
                .register_job(test_job_def("ProcessRow", "imports"))
                .unwrap();
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let enqueue_fn = get_fn(&module, "enqueue");
            let seal_fn = get_fn(&module, "seal");
            let status_fn = get_fn(&module, "batch_status");

            let handle = batch_fn(&[Value::String("status-test".to_string())]).unwrap();
            let mut payload = HashMap::new();
            payload.insert("x".to_string(), Value::Int(1));
            enqueue_fn(&[
                handle.clone(),
                Value::String("ProcessRow".to_string()),
                Value::Map(payload),
            ])
            .unwrap();
            seal_fn(&[handle.clone()]).unwrap();

            // Query via handle
            let result = status_fn(&[handle.clone()]).unwrap();
            let meta = match result {
                Value::EnumValue {
                    ref variant,
                    ref values,
                    ..
                } if variant == "Ok" => &values[0],
                _ => panic!("batch_status must return Ok"),
            };
            match meta {
                Value::Map(m) => {
                    assert!(
                        matches!(m.get("total"), Some(Value::Int(1))),
                        "total must be 1"
                    );
                    assert!(
                        matches!(m.get("pending"), Some(Value::Int(1))),
                        "pending must be 1"
                    );
                    assert!(
                        matches!(m.get("status"), Some(Value::String(s)) if s == "sealed"),
                        "status must be sealed"
                    );
                }
                _ => panic!("expected map in Ok"),
            }

            // Also query by string ID
            let bid = match &handle {
                Value::Map(m) => match m.get("_batch_id") {
                    Some(Value::String(s)) => s.clone(),
                    _ => panic!("no _batch_id"),
                },
                _ => panic!("not a map"),
            };
            let result2 = status_fn(&[Value::String(bid)]).unwrap();
            assert!(
                matches!(result2, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "batch_status by string ID must return Ok"
            );
        });
    }

    #[test]
    fn test_batch_seal_idempotent() {
        with_temp_kv("ntnt_batch_idempotent_test.db", |_kv| {
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let seal_fn = get_fn(&module, "seal");

            let handle = batch_fn(&[Value::String("idempotent-batch".to_string())]).unwrap();
            // First seal
            seal_fn(&[handle.clone()]).unwrap();
            // Second seal — must be no-op (Ok, not an error)
            let result = seal_fn(&[handle.clone()]).unwrap();
            assert!(
                matches!(result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "second seal must return Ok (no-op)"
            );
        });
    }

    #[test]
    fn test_enqueue_after_seal_rejected() {
        with_temp_kv("ntnt_batch_enqueue_after_seal_test.db", |_kv| {
            JOB_RUNTIME
                .register_job(test_job_def("ProcessRow", "imports"))
                .unwrap();
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let enqueue_fn = get_fn(&module, "enqueue");
            let seal_fn = get_fn(&module, "seal");

            let handle = batch_fn(&[Value::String("sealed-batch".to_string())]).unwrap();
            seal_fn(&[handle.clone()]).unwrap();

            // Try to enqueue after seal — must return an error
            let mut payload = HashMap::new();
            payload.insert("x".to_string(), Value::Int(1));
            let result = enqueue_fn(&[
                handle.clone(),
                Value::String("ProcessRow".to_string()),
                Value::Map(payload),
            ]);
            assert!(result.is_err(), "enqueue after seal must return an error");
        });
    }

    #[test]
    fn test_batch_id_propagation_into_job_kv_data() {
        with_temp_kv("ntnt_batch_id_prop_test.db", |kv| {
            JOB_RUNTIME
                .register_job(test_job_def("ProcessRow", "imports"))
                .unwrap();
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let enqueue_fn = get_fn(&module, "enqueue");
            let seal_fn = get_fn(&module, "seal");

            let handle = batch_fn(&[Value::String("id-prop-test".to_string())]).unwrap();
            let bid = match &handle {
                Value::Map(m) => match m.get("_batch_id") {
                    Some(Value::String(s)) => s.clone(),
                    _ => panic!("no _batch_id"),
                },
                _ => panic!("not a map"),
            };

            let mut payload = HashMap::new();
            payload.insert("row_id".to_string(), Value::String("r1".to_string()));
            enqueue_fn(&[
                handle.clone(),
                Value::String("ProcessRow".to_string()),
                Value::Map(payload),
            ])
            .unwrap();
            seal_fn(&[handle.clone()]).unwrap();

            // Verify each job's data map contains batch_id
            let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
            assert_eq!(data_keys.len(), 1, "expected 1 job data entry");
            for key in &data_keys {
                let job_data = kv::kv_get(kv, key).unwrap();
                match job_data {
                    Value::Map(ref m) => {
                        assert!(
                            matches!(m.get("batch_id"), Some(Value::String(ref s)) if s == &bid),
                            "job data must contain batch_id matching the batch: got {:?}",
                            m.get("batch_id")
                        );
                    }
                    _ => panic!("job data must be a Map"),
                }
            }
        });
    }

    #[test]
    fn test_batch_seal_flushed_flag_retry_path() {
        // Tests the partial seal retry scenario: if some jobs were flushed
        // before a failure, re-sealing skips already-flushed jobs.
        with_temp_kv("ntnt_batch_flushed_retry_test.db", |kv| {
            JOB_RUNTIME
                .register_job(test_job_def("ProcessRow", "imports"))
                .unwrap();
            let module = init();
            let batch_fn = get_fn(&module, "batch");
            let enqueue_fn = get_fn(&module, "enqueue");
            let seal_fn = get_fn(&module, "seal");

            let handle = batch_fn(&[Value::String("flushed-retry".to_string())]).unwrap();
            let bid = match &handle {
                Value::Map(m) => match m.get("_batch_id") {
                    Some(Value::String(s)) => s.clone(),
                    _ => panic!("no _batch_id"),
                },
                _ => panic!("not a map"),
            };

            // Enqueue 3 jobs
            for i in 0..3 {
                let mut payload = HashMap::new();
                payload.insert("row_id".to_string(), Value::String(format!("r{}", i)));
                enqueue_fn(&[
                    handle.clone(),
                    Value::String("ProcessRow".to_string()),
                    Value::Map(payload),
                ])
                .unwrap();
            }

            // Simulate partial flush: manually mark the first job as flushed
            // then seal. The seal should only flush the remaining 2 jobs.
            {
                let mut batches = BATCH_RUNTIME.batches.lock().unwrap();
                if let Some(batch) = batches.get_mut(&bid) {
                    // Simulate: first job was already flushed in a prior attempt
                    batch.buffered[0].flushed = true;
                }
            }

            // Manually write the "first" job to KV to simulate partial flush
            let first_job_payload = HashMap::new();
            enqueue_internal(
                "ProcessRow",
                Value::Map(first_job_payload),
                "00000000000000000000",
                None,
                Some(&bid),
            )
            .unwrap();

            // Now seal — should skip the already-flushed first job
            let seal_result = seal_fn(&[handle.clone()]).unwrap();
            assert!(
                matches!(seal_result, Value::EnumValue { ref variant, .. } if variant == "Ok"),
                "seal() must return Ok even with pre-flushed jobs"
            );

            // Total jobs in KV should be 3 (1 pre-flushed + 2 flushed during seal)
            let data_keys = kv::kv_list(kv, Some("jobs:data:")).unwrap_or_default();
            assert_eq!(
                data_keys.len(),
                3,
                "3 jobs:data: entries expected (1 pre-flushed + 2 from seal)"
            );

            // Batch metadata should show total=3, status=sealed
            let meta = kv::kv_get(kv, &format!("jobs:batch:{}", bid)).unwrap();
            match meta {
                Value::Map(ref m) => {
                    assert!(
                        matches!(m.get("status"), Some(Value::String(s)) if s == "sealed"),
                        "status must be sealed"
                    );
                    assert!(
                        matches!(m.get("total"), Some(Value::Int(3))),
                        "total must be 3"
                    );
                }
                _ => panic!("expected batch metadata map"),
            }
        });
    }
}
