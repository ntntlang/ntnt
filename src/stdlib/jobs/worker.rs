//! Worker loop implementations for all backends.
//!
//! Contains the background worker threads that poll for jobs and execute them.
//! Each backend has its own worker loop due to differences in:
//! - How idle waiting works (memory: sleep, PG: LISTEN/NOTIFY, Redis: XREADGROUP block)
//! - Heartbeat requirements (PG/Redis need heartbeat threads, memory doesn't)
//! - Periodic maintenance (PG: stale release, Redis: scheduled promotion + stale release)
//! - Job execution details (each backend has its own execute function)

use super::memory;
use super::postgres;
use super::redis_backend;
use super::{
    BackendKind, JobStatus, PostgresBackendConfig, QueuedJob, RedisBackendConfig, JOB_REGISTRY,
    MAX_DEAD_JOBS, WORKER_RUNNING, WORKER_STOP,
};
use crate::error::IntentError;
use crate::interpreter::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, IntentError>;

// ============================================================
// Job Execution (shared logic)
// ============================================================

/// Execute a single memory job's perform body
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

// ============================================================
// Memory Worker
// ============================================================

/// Start the in-memory background worker loop
pub(crate) fn start_worker_memory() -> Result<()> {
    if WORKER_RUNNING.load(Ordering::SeqCst) {
        return Ok(());
    }

    WORKER_RUNNING.store(true, Ordering::SeqCst);
    WORKER_STOP.store(false, Ordering::SeqCst);

    std::thread::spawn(move || {
        while !WORKER_STOP.load(Ordering::SeqCst) {
            match memory::claim_next_job() {
                Ok(Some(mut job)) => {
                    // Check if the job's queue is paused
                    if memory::is_queue_paused_memory(&job.queue_name) {
                        // Put the job back
                        if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
                            job.status = JobStatus::Pending;
                            if let Some(q) = backend.queues.get_mut(&job.queue_name) {
                                q.push_front(job);
                            }
                        }
                        std::thread::sleep(Duration::from_millis(200));
                        continue;
                    }

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
                            if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
                                backend.jobs_by_id.insert(job_id.clone(), job.clone());
                                backend.completed_jobs.push((Instant::now(), job));
                            }
                        }
                        Err(error) => {
                            job.error = Some(error.clone());
                            job.attempt_count += 1;

                            if job.attempt_count < job.max_attempts {
                                execute_on_failure(&job.job_type, &error, job.attempt_count);
                                if let Err(e) = memory::requeue_for_retry(&mut job, backoff_base) {
                                    eprintln!("[jobs] Failed to requeue for retry: {}", e);
                                }
                            } else {
                                execute_on_failure(&job.job_type, &error, job.attempt_count);
                                job.status = JobStatus::Dead;
                                if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
                                    backend.jobs_by_id.insert(job_id.clone(), job.clone());
                                    backend.dead_jobs.push(job);

                                    // Dead job cap for memory backend
                                    let max_dead = MAX_DEAD_JOBS.load(Ordering::Relaxed) as usize;
                                    if backend.dead_jobs.len() > max_dead {
                                        let excess = backend.dead_jobs.len() - max_dead;
                                        let removed: Vec<QueuedJob> =
                                            backend.dead_jobs.drain(..excess).collect();
                                        for removed_job in &removed {
                                            backend.jobs_by_id.remove(&removed_job.id);
                                        }
                                    }
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
            if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
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

// ============================================================
// Postgres Worker
// ============================================================

/// Start the postgres-backed worker loop
pub(crate) fn start_worker_postgres(
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

    // Start LISTEN/NOTIFY listener for instant dispatch
    if let Err(e) = postgres::pg_start_listen_notify(&config.connection_url) {
        eprintln!("[jobs/pg] Failed to start LISTEN/NOTIFY: {}", e);
    }

    std::thread::spawn(move || {
        // Release stale jobs on startup
        if let Err(e) = postgres::pg_release_stale_jobs(visibility_timeout) {
            eprintln!("[jobs/pg] Failed to release stale jobs on startup: {}", e);
        }

        // Stale job release interval
        let stale_check_interval = Duration::from_secs(visibility_timeout / 2);
        let mut last_stale_check = Instant::now();

        while !WORKER_STOP.load(Ordering::SeqCst) {
            // Periodically release stale jobs
            if last_stale_check.elapsed() >= stale_check_interval {
                if let Err(e) = postgres::pg_release_stale_jobs(visibility_timeout) {
                    eprintln!("[jobs/pg] Failed to release stale jobs: {}", e);
                }
                last_stale_check = Instant::now();
            }

            // Filter out paused queues
            let active_queues: Vec<String> = queues
                .iter()
                .filter(|q| !postgres::pg_paused_queues().unwrap_or_default().contains(q))
                .cloned()
                .collect();

            if active_queues.is_empty() {
                // All queues are paused, wait briefly
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }

            match postgres::pg_claim_next_job(&active_queues, &worker_id) {
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
                            if let Err(e) = postgres::pg_heartbeat(&hb_job_id) {
                                eprintln!("[jobs/pg] Heartbeat failed for {}: {}", hb_job_id, e);
                                break;
                            }
                        }
                    });

                    // Execute the job
                    let result = postgres::execute_pg_job(&job);

                    // Stop heartbeat
                    hb_stop.store(true, Ordering::SeqCst);
                    let _ = hb_handle.join();

                    match result {
                        Ok(result_value) => {
                            let result_json =
                                Some(crate::stdlib::json::intent_value_to_json(&result_value));
                            if let Err(e) = postgres::pg_complete_job(&job_id, result_json) {
                                eprintln!("[jobs/pg] Failed to complete job {}: {}", job_id, e);
                            }
                        }
                        Err(error) => {
                            execute_on_failure(&job.job_type, &error, job.attempts);
                            if let Err(e) = postgres::pg_fail_job(
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
                    // No jobs ready — wait for LISTEN/NOTIFY signal or 5s timeout
                    let (lock, cvar) = &*postgres::PG_NOTIFY_SIGNAL;
                    if let Ok(mut notified) = lock.lock() {
                        if !*notified {
                            let result = cvar.wait_timeout(notified, Duration::from_secs(5));
                            if let Ok((mut guard, _)) = result {
                                *guard = false;
                            }
                        } else {
                            *notified = false;
                        }
                    }
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

// ============================================================
// Redis Worker
// ============================================================

/// Start the Redis-backed worker loop
pub(crate) fn start_worker_redis(
    config: RedisBackendConfig,
    queue_names: Option<Vec<String>>,
) -> Result<()> {
    if WORKER_RUNNING.load(Ordering::SeqCst) {
        return Ok(());
    }

    WORKER_RUNNING.store(true, Ordering::SeqCst);
    WORKER_STOP.store(false, Ordering::SeqCst);

    let worker_id = uuid::Uuid::new_v4().to_string();
    let group = config.consumer_group.clone();
    let visibility_timeout_ms = config.visibility_timeout_secs * 1000;
    let prune_secs = config.prune_completed_after_secs;

    let queues: Vec<String> = queue_names.unwrap_or_else(|| vec!["default".to_string()]);

    std::thread::spawn(move || {
        let stale_check_interval = Duration::from_secs(config.visibility_timeout_secs / 2);
        let scheduled_check_interval = Duration::from_secs(1);
        let mut last_stale_check = Instant::now();
        let mut last_scheduled_check = Instant::now();

        while !WORKER_STOP.load(Ordering::SeqCst) {
            // Promote scheduled jobs
            if last_scheduled_check.elapsed() >= scheduled_check_interval {
                if let Err(e) = redis_backend::redis_promote_scheduled_jobs() {
                    eprintln!("[jobs/redis] Failed to promote scheduled jobs: {}", e);
                }
                last_scheduled_check = Instant::now();
            }

            // Release stale jobs
            if last_stale_check.elapsed() >= stale_check_interval {
                if let Err(e) =
                    redis_backend::redis_release_stale_jobs(&queues, &group, visibility_timeout_ms)
                {
                    eprintln!("[jobs/redis] Failed to release stale jobs: {}", e);
                }
                last_stale_check = Instant::now();
            }

            // Filter out paused queues
            let active_queues: Vec<String> = queues
                .iter()
                .filter(|q| !redis_backend::redis_is_queue_paused(q).unwrap_or(false))
                .cloned()
                .collect();

            if active_queues.is_empty() {
                std::thread::sleep(Duration::from_millis(500));
                continue;
            }

            match redis_backend::redis_claim_next_job(&active_queues, &worker_id, &group) {
                Ok(Some(job)) => {
                    let job_id = job.id.clone();
                    let job_type = job.job_type.clone();
                    let stream_msg_id = job.stream_message_id.clone();
                    let queue_name = job.queue_name.clone();

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
                    let hb_interval = config.visibility_timeout_secs / 3;

                    let hb_handle = std::thread::spawn(move || {
                        while !hb_stop_clone.load(Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_secs(hb_interval.max(5)));
                            if hb_stop_clone.load(Ordering::SeqCst) {
                                break;
                            }
                            if let Err(e) = redis_backend::redis_heartbeat(&hb_job_id) {
                                eprintln!("[jobs/redis] Heartbeat failed for {}: {}", hb_job_id, e);
                                break;
                            }
                        }
                    });

                    let result = redis_backend::execute_redis_job(&job);

                    hb_stop.store(true, Ordering::SeqCst);
                    let _ = hb_handle.join();

                    match result {
                        Ok(result_value) => {
                            let result_json =
                                Some(crate::stdlib::json::intent_value_to_json(&result_value));
                            if let Err(e) = redis_backend::redis_complete_job(
                                &job_id,
                                &queue_name,
                                &stream_msg_id,
                                result_json,
                                &group,
                                prune_secs,
                            ) {
                                eprintln!("[jobs/redis] Failed to complete job {}: {}", job_id, e);
                            }
                        }
                        Err(error) => {
                            execute_on_failure(&job.job_type, &error, job.attempts);
                            if let Err(e) = redis_backend::redis_fail_job(
                                &job_id,
                                &queue_name,
                                &stream_msg_id,
                                &error,
                                job.attempts,
                                job.max_attempts,
                                backoff_base,
                                &group,
                            ) {
                                eprintln!("[jobs/redis] Failed to fail job {}: {}", job_id, e);
                            }
                        }
                    }
                }
                Ok(None) => {
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    eprintln!("[jobs/redis] Worker error claiming job: {}", e);
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }

        WORKER_RUNNING.store(false, Ordering::SeqCst);
    });

    Ok(())
}

// ============================================================
// Blocking Worker (multi-threaded)
// ============================================================

/// Start a blocking worker (for dedicated worker processes)
pub(crate) fn start_worker_blocking_impl(opts: &HashMap<String, Value>) -> Result<()> {
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

    match super::get_backend()? {
        BackendKind::Memory => {
            if concurrency <= 1 {
                // Single-threaded: use the original async worker and block
                start_worker_memory()?;
                loop {
                    if WORKER_STOP.load(Ordering::SeqCst) {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(1));
                }
            } else {
                // Multi-threaded memory worker
                WORKER_RUNNING.store(true, Ordering::SeqCst);
                WORKER_STOP.store(false, Ordering::SeqCst);

                let mut handles = Vec::new();
                for _ in 0..concurrency {
                    let handle = std::thread::spawn(move || {
                        while !WORKER_STOP.load(Ordering::SeqCst) {
                            match memory::claim_next_job() {
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
                                            if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
                                                backend
                                                    .jobs_by_id
                                                    .insert(job_id.clone(), job.clone());
                                                backend.completed_jobs.push((Instant::now(), job));
                                            }
                                        }
                                        Err(error) => {
                                            job.error = Some(error.clone());
                                            job.attempt_count += 1;

                                            if job.attempt_count < job.max_attempts {
                                                execute_on_failure(
                                                    &job.job_type,
                                                    &error,
                                                    job.attempt_count,
                                                );
                                                if let Err(e) = memory::requeue_for_retry(
                                                    &mut job,
                                                    backoff_base,
                                                ) {
                                                    eprintln!(
                                                        "[jobs] Failed to requeue for retry: {}",
                                                        e
                                                    );
                                                }
                                            } else {
                                                execute_on_failure(
                                                    &job.job_type,
                                                    &error,
                                                    job.attempt_count,
                                                );
                                                job.status = JobStatus::Dead;
                                                if let Ok(mut backend) =
                                                    memory::QUEUE_BACKEND.lock()
                                                {
                                                    backend
                                                        .jobs_by_id
                                                        .insert(job_id.clone(), job.clone());
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
                            if let Ok(mut backend) = memory::QUEUE_BACKEND.lock() {
                                let prune_after =
                                    Duration::from_millis(backend.config.prune_completed_after_ms);
                                let now = Instant::now();
                                backend.completed_jobs.retain(|(completed_at, _)| {
                                    now.duration_since(*completed_at) < prune_after
                                });
                            }
                        }
                    });
                    handles.push(handle);
                }

                for handle in handles {
                    let _ = handle.join();
                }
                WORKER_RUNNING.store(false, Ordering::SeqCst);
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
            if let Err(e) = postgres::pg_release_stale_jobs(visibility_timeout) {
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
                            let _ = postgres::pg_release_stale_jobs(vis_timeout);
                            last_stale_check = Instant::now();
                        }

                        match postgres::pg_claim_next_job(&queues, &worker_id) {
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
                                        if let Err(e) = postgres::pg_heartbeat(&hb_job_id) {
                                            eprintln!(
                                                "[jobs/pg] Heartbeat failed for {}: {}",
                                                hb_job_id, e
                                            );
                                            break;
                                        }
                                    }
                                });

                                let result = postgres::execute_pg_job(&job);

                                hb_stop.store(true, Ordering::SeqCst);
                                let _ = hb_handle.join();

                                match result {
                                    Ok(result_value) => {
                                        let result_json =
                                            Some(crate::stdlib::json::intent_value_to_json(
                                                &result_value,
                                            ));
                                        if let Err(e) =
                                            postgres::pg_complete_job(&job_id, result_json)
                                        {
                                            eprintln!(
                                                "[jobs/pg] Failed to complete job {}: {}",
                                                job_id, e
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        execute_on_failure(&job.job_type, &error, job.attempts);
                                        if let Err(e) = postgres::pg_fail_job(
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
        BackendKind::Redis(config) => {
            // Start N worker threads for Redis
            WORKER_RUNNING.store(true, Ordering::SeqCst);
            WORKER_STOP.store(false, Ordering::SeqCst);

            let worker_id_base = uuid::Uuid::new_v4().to_string();
            let group = config.consumer_group.clone();
            let visibility_timeout_ms = config.visibility_timeout_secs * 1000;
            let prune_secs = config.prune_completed_after_secs;

            let mut handles = Vec::new();

            for i in 0..concurrency {
                let queues = queues.clone();
                let worker_id = format!("{}-{}", worker_id_base, i);
                let group = group.clone();
                let vis_timeout_ms = visibility_timeout_ms;
                let vis_timeout_secs = config.visibility_timeout_secs;

                let handle = std::thread::spawn(move || {
                    let stale_check_interval = Duration::from_secs(vis_timeout_secs / 2);
                    let scheduled_check_interval = Duration::from_secs(1);
                    let mut last_stale_check = Instant::now();
                    let mut last_scheduled_check = Instant::now();

                    while !WORKER_STOP.load(Ordering::SeqCst) {
                        // Only first worker promotes scheduled jobs
                        if i == 0 && last_scheduled_check.elapsed() >= scheduled_check_interval {
                            let _ = redis_backend::redis_promote_scheduled_jobs();
                            last_scheduled_check = Instant::now();
                        }

                        if last_stale_check.elapsed() >= stale_check_interval {
                            let _ = redis_backend::redis_release_stale_jobs(
                                &queues,
                                &group,
                                vis_timeout_ms,
                            );
                            last_stale_check = Instant::now();
                        }

                        match redis_backend::redis_claim_next_job(&queues, &worker_id, &group) {
                            Ok(Some(job)) => {
                                let job_id = job.id.clone();
                                let job_type = job.job_type.clone();
                                let stream_msg_id = job.stream_message_id.clone();
                                let queue_name = job.queue_name.clone();

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
                                let hb_interval = vis_timeout_secs / 3;

                                let hb_handle = std::thread::spawn(move || {
                                    while !hb_stop_clone.load(Ordering::SeqCst) {
                                        std::thread::sleep(Duration::from_secs(hb_interval.max(5)));
                                        if hb_stop_clone.load(Ordering::SeqCst) {
                                            break;
                                        }
                                        if let Err(e) = redis_backend::redis_heartbeat(&hb_job_id) {
                                            eprintln!(
                                                "[jobs/redis] Heartbeat failed for {}: {}",
                                                hb_job_id, e
                                            );
                                            break;
                                        }
                                    }
                                });

                                let result = redis_backend::execute_redis_job(&job);

                                hb_stop.store(true, Ordering::SeqCst);
                                let _ = hb_handle.join();

                                match result {
                                    Ok(result_value) => {
                                        let result_json =
                                            Some(crate::stdlib::json::intent_value_to_json(
                                                &result_value,
                                            ));
                                        if let Err(e) = redis_backend::redis_complete_job(
                                            &job_id,
                                            &queue_name,
                                            &stream_msg_id,
                                            result_json,
                                            &group,
                                            prune_secs,
                                        ) {
                                            eprintln!(
                                                "[jobs/redis] Failed to complete job {}: {}",
                                                job_id, e
                                            );
                                        }
                                    }
                                    Err(error) => {
                                        execute_on_failure(&job.job_type, &error, job.attempts);
                                        if let Err(e) = redis_backend::redis_fail_job(
                                            &job_id,
                                            &queue_name,
                                            &stream_msg_id,
                                            &error,
                                            job.attempts,
                                            job.max_attempts,
                                            backoff_base,
                                            &group,
                                        ) {
                                            eprintln!(
                                                "[jobs/redis] Failed to fail job {}: {}",
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
                                eprintln!("[jobs/redis] Worker {} error: {}", worker_id, e);
                                std::thread::sleep(Duration::from_millis(500));
                            }
                        }
                    }
                });

                handles.push(handle);
            }

            for handle in handles {
                let _ = handle.join();
            }

            WORKER_RUNNING.store(false, Ordering::SeqCst);
            Ok(())
        }
    }
}
