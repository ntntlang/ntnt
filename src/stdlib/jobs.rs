//! Job DSL module for NTNT
//!
//! Provides background job definitions, enqueue, status, and cancellation.
//!
//! ## KV Key Layout
//!
//! ```text
//! jobs:pending:<zero-padded-timestamp>:<id>   →  "" (queue ordering key)
//! jobs:data:<id>                               →  full job data map (type, queue, payload, status, etc.)
//! jobs:active:<id>                             →  TTL key for visibility timeout (PR 2b)
//! ```
//!
//! `list(kv, "jobs:pending:")` returns keys in lexicographic order.
//! Zero-padded timestamps sort correctly for FIFO ordering.
//!
//! Example usage:
//! ```ntnt
//! import { configure_queue, enqueue, job_status, cancel_job } from "std/jobs"
//!
//! configure_queue(map { "store": "sqlite:./jobs.db" })
//! let id = enqueue("SendEmail", map { "to": "alice@example.com" })
//! let status = job_status(id)
//! ```

use crate::error::{IntentError, Result};
use crate::interpreter::Value;
use crate::stdlib::kv;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

// ============================================================================
// Job Runtime — global singleton (mirrors ConcurrencyRuntime pattern)
// ============================================================================

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

/// A registered job definition from `Job Name on queue { perform(...) { ... } }`.
#[derive(Debug, Clone)]
pub struct JobDefinition {
    /// Job name (e.g., "SendEmail")
    pub name: String,
    /// Queue name (e.g., "emails")
    pub queue: String,
    /// Options: retry count, timeout, etc. (Send + Sync safe).
    pub options: HashMap<String, JobOptionValue>,
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
}

impl JobRuntime {
    fn new() -> Self {
        JobRuntime {
            job_registry: RwLock::new(HashMap::new()),
            kv_handle_info: Mutex::new(None),
            kv_url: Mutex::new("sqlite:./jobs.db".to_string()),
            test_queue: Mutex::new(None),
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
    fn get_or_init_kv(&self) -> Result<Value> {
        let mut info = self.kv_handle_info.lock().map_err(|e| {
            IntentError::runtime_error(format!("Job KV handle lock poisoned: {}", e))
        })?;

        if let Some(ref h) = *info {
            return Ok(h.to_value());
        }

        // Lazy init with default or configured URL
        let url = self.kv_url.lock().map_err(|e| {
            IntentError::runtime_error(format!("Job KV URL lock poisoned: {}", e))
        })?;

        let kv_handle_value = kv::open_kv(&url)?;

        // Extract info from the returned Value::Map
        let handle_info = extract_kv_handle_info(&kv_handle_value)?;
        *info = Some(handle_info);
        Ok(kv_handle_value)
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
fn timestamp_key() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{:020}", now.as_nanos())
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

                // Validate URL format
                if !store_url.starts_with("sqlite:")
                    && !store_url.starts_with("redis://")
                    && !store_url.starts_with("valkey://")
                {
                    return Err(IntentError::runtime_error(format!(
                        "Invalid store URL '{}'. Expected sqlite:, redis://, or valkey://",
                        store_url
                    )));
                }

                // Update the URL
                {
                    let mut url = JOB_RUNTIME.kv_url.lock().map_err(|e| {
                        IntentError::runtime_error(format!("Lock error: {}", e))
                    })?;
                    *url = store_url.clone();
                }

                // Open the KV connection now
                let kv_handle_value = kv::open_kv(&store_url)?;
                let handle_info = extract_kv_handle_info(&kv_handle_value)?;
                {
                    let mut info = JOB_RUNTIME.kv_handle_info.lock().map_err(|e| {
                        IntentError::runtime_error(format!("Lock error: {}", e))
                    })?;
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

                // Look up job in registry
                let job_def = JOB_RUNTIME.get_job(&job_name)?;
                let job_def = match job_def {
                    Some(def) => def,
                    None => {
                        return Err(IntentError::runtime_error(format!(
                            "Job '{}' is not registered. Define it with: Job {} on <queue> {{ perform(...) {{ ... }} }}",
                            job_name, job_name
                        )));
                    }
                };

                let job_id = Uuid::new_v4().to_string();

                // Check test mode
                {
                    let mut test_queue = JOB_RUNTIME.test_queue.lock().map_err(|e| {
                        IntentError::runtime_error(format!("Lock error: {}", e))
                    })?;
                    if let Some(ref mut queue) = *test_queue {
                        let payload_json = serde_json::to_string(
                            &crate::stdlib::kv::value_to_json_public(&payload),
                        )
                        .unwrap_or_default();
                        queue.push(EnqueuedJob {
                            id: job_id.clone(),
                            job_type: job_name.clone(),
                            queue: job_def.queue.clone(),
                            payload_json,
                        });
                        return Ok(Value::ok(Value::String(job_id)));
                    }
                }

                // Get KV handle (lazy init)
                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;

                let ts = timestamp_key();

                // Build job data map
                let mut job_data = HashMap::new();
                job_data.insert("id".to_string(), Value::String(job_id.clone()));
                job_data.insert("type".to_string(), Value::String(job_name.clone()));
                job_data.insert("queue".to_string(), Value::String(job_def.queue.clone()));
                job_data.insert("payload".to_string(), payload);
                job_data.insert(
                    "status".to_string(),
                    Value::String("pending".to_string()),
                );
                job_data.insert("attempts".to_string(), Value::Int(0));
                job_data.insert("created_at".to_string(), Value::String(ts.clone()));

                // Copy job options (retry, timeout, etc.)
                for (k, v) in &job_def.options {
                    job_data.insert(k.clone(), v.to_value());
                }

                // Write to KV: jobs:data:<id>
                let data_key = format!("jobs:data:{}", job_id);
                kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None)?;

                // Write queue ordering key: jobs:pending:<timestamp>:<id>
                let pending_key = format!("jobs:pending:{}:{}", ts, job_id);
                kv::kv_set(
                    &kv_handle,
                    &pending_key,
                    &Value::String(job_id.clone()),
                    None,
                )?;

                Ok(Value::ok(Value::String(job_id)))
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
    // @signature cancel_job(job_id: String) -> Result<Bool, String>
    // Cancel a pending job by its ID.
    //
    // Sets the job status to "cancelled" and removes it from the pending queue.
    // Returns true if the job was cancelled, false if it was not in a cancellable state.
    // @param job_id The job ID returned by enqueue()
    // @returns Result containing true if cancelled, false if not cancellable
    // @example cancel_job("abc-123") ~ "Cancel a pending job"
    module.insert(
        "cancel_job".to_string(),
        Value::NativeFunction {
            name: "cancel_job".to_string(),
            arity: 1,
            max_arity: 1,
            func: |args| {
                if args.len() != 1 {
                    return Err(IntentError::type_error(
                        "cancel_job() requires 1 argument (job_id)".to_string(),
                    ));
                }

                let job_id = match &args[0] {
                    Value::String(s) => s.clone(),
                    _ => {
                        return Err(IntentError::type_error(
                            "cancel_job() requires a string job ID".to_string(),
                        ))
                    }
                };

                let kv_handle = JOB_RUNTIME.get_or_init_kv()?;
                let data_key = format!("jobs:data:{}", job_id);

                // Read current job data
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

                // Check if cancellable (only pending jobs)
                let status = match job_data.get("status") {
                    Some(Value::String(s)) => s.clone(),
                    _ => "unknown".to_string(),
                };

                if status != "pending" {
                    return Ok(Value::ok(Value::Bool(false)));
                }

                // Update status to cancelled
                job_data.insert(
                    "status".to_string(),
                    Value::String("cancelled".to_string()),
                );
                kv::kv_set(&kv_handle, &data_key, &Value::Map(job_data), None)?;

                // Remove from pending queue — find and delete the pending key
                let pending_keys = kv::kv_list(&kv_handle, Some("jobs:pending:"))?;
                for key in pending_keys {
                    if key.ends_with(&job_id) {
                        kv::kv_del(&kv_handle, &key)?;
                        break;
                    }
                }

                Ok(Value::ok(Value::Bool(true)))
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
        let _guard = TEST_LOCK.lock().unwrap();
        JOB_RUNTIME.reset();
        f();
    }

    #[test]
    fn test_register_job() {
        with_clean_runtime(|| {
            let result = JOB_RUNTIME.register_job(JobDefinition {
                name: "TestJob".to_string(),
                queue: "default".to_string(),
                options: HashMap::new(),
            });
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
                .register_job(JobDefinition {
                    name: "DupJob".to_string(),
                    queue: "default".to_string(),
                    options: HashMap::new(),
                })
                .unwrap();

            let result = JOB_RUNTIME.register_job(JobDefinition {
                name: "DupJob".to_string(),
                queue: "other".to_string(),
                options: HashMap::new(),
            });
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
                .register_job(JobDefinition {
                    name: "OptsJob".to_string(),
                    queue: "default".to_string(),
                    options,
                })
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
                .register_job(JobDefinition {
                    name: "EmailJob".to_string(),
                    queue: "emails".to_string(),
                    options: HashMap::new(),
                })
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
            let result = enqueue_fn(&[
                Value::String("EmailJob".to_string()),
                Value::Map(payload),
            ])
            .unwrap();

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
                            let status_result =
                                status_fn(&[Value::String(id.clone())]).unwrap();
                            match status_result {
                                Value::EnumValue {
                                    variant, values, ..
                                } if variant == "Ok" => {
                                    match &values[0] {
                                        Value::Map(data) => {
                                            assert!(matches!(data.get("status"), Some(Value::String(s)) if s == "pending"));
                                            assert!(matches!(data.get("type"), Some(Value::String(s)) if s == "EmailJob"));
                                            assert!(matches!(data.get("queue"), Some(Value::String(s)) if s == "emails"));
                                        }
                                        _ => panic!("Expected Map in status Ok"),
                                    }
                                }
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
                .register_job(JobDefinition {
                    name: "CancelJob".to_string(),
                    queue: "default".to_string(),
                    options: HashMap::new(),
                })
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
                        assert!(matches!(data.get("status"), Some(Value::String(s)) if s == "cancelled"));
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
            assert!(err.contains("Invalid store URL"));
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
}
